//! Microbenchmark de replicación para el paper (sección Evaluation).
//! Mide, para N eventos divergentes entre 2 réplicas:
//!   - t_fold_verify_ms: load_all (verificación ed25519 incluida) + fold + estado canónico
//!   - t_parse_only_ms:  mismo recorrido sin verificación de firmas (aísla el overhead)
//!   - store_bytes:      tamaño en disco de events/
//! Salida: JSON lines por (n, rep) → run_all.py agrega.

use std::fs;
use std::path::Path;
use std::time::Instant;

use garagedb_core::event::{CountSource, CountStatus, Event, EventBody, total_sort};
use garagedb_core::fold::{fold, state_canonical_json};
use garagedb_core::ids::{LocationId, Sku};
use garagedb_core::quantity::Quantity;
use garagedb_core::store::EventStore;

fn generate(store: &mut EventStore, salt: u64, n: usize) {
    let skus = ["M6x20", "M8x30", "BRIDA-200", "DOT4", "6004-2RS"];
    let locs = ["T2-D07", "S1-N3-P4", "CAR1", "T1-A01"];
    for i in 0..n {
        let sku = Sku::new(skus[(i + salt as usize) % skus.len()]);
        let loc = LocationId::new(locs[(i * 7 + salt as usize) % locs.len()]);
        let body = match i % 3 {
            0 => EventBody::Count {
                sku,
                loc,
                qty: Quantity::Exact { n: ((i as u64 * (salt + 3)) % 40) + 5 },
                source: CountSource::Human,
                status: CountStatus::Confirmed,
            },
            1 => EventBody::Move {
                sku,
                loc,
                delta: (i as i64 % 5) - 2,
                reason: format!("bench-{i}"),
                mission: None,
            },
            _ => EventBody::Ingest {
                sku,
                name: "item".into(),
                category: "cat".into(),
                unit: "ud".into(),
                loc,
                qty: Quantity::Exact { n: 3 },
            },
        };
        store.append("bench", body).unwrap();
    }
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), &dest).unwrap();
        }
    }
}

fn dir_bytes(dir: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            total += if p.is_dir() { dir_bytes(&p) } else { p.metadata().map(|m| m.len()).unwrap_or(0) };
        }
    }
    total
}

/// Parse sin verificación de firmas (para aislar el coste ed25519).
fn parse_only(root: &Path) -> Vec<Event> {
    let mut out = Vec::new();
    let events = root.join("events");
    for rdir in fs::read_dir(&events).unwrap().flatten() {
        if !rdir.path().is_dir() { continue; }
        for shard in fs::read_dir(rdir.path()).unwrap().flatten() {
            let content = fs::read_to_string(shard.path()).unwrap();
            for line in content.lines() {
                if let Ok(ev) = serde_json::from_str::<Event>(line) {
                    out.push(ev);
                }
            }
        }
    }
    total_sort(&mut out);
    out
}

fn main() {
    let sizes = [50usize, 100, 300, 1000, 3000];
    let reps = 5;
    for &n in &sizes {
        for rep in 0..reps {
            let dir_a = tempfile::tempdir().unwrap();
            let dir_b = tempfile::tempdir().unwrap();
            let mut a = EventStore::init(dir_a.path()).unwrap();
            let mut b = EventStore::init(dir_b.path()).unwrap();
            generate(&mut a, 3 + rep, n / 2);
            generate(&mut b, 11 + rep, n / 2);

            // transporte folder: copiar shards + claves de B a A
            for sub in ["events", "config/replicas"] {
                copy_tree(&dir_b.path().join(sub), &dir_a.path().join(sub));
            }

            // fold con verificación completa
            let t0 = Instant::now();
            let report = a.load_all().unwrap();
            let state = fold(&report.events);
            let canon = state_canonical_json(&state).unwrap();
            let t_verify = t0.elapsed().as_secs_f64() * 1000.0;

            // parse sin firmas
            let t1 = Instant::now();
            let evs = parse_only(dir_a.path());
            let state2 = fold(&evs);
            let canon2 = state_canonical_json(&state2).unwrap();
            let t_parse = t1.elapsed().as_secs_f64() * 1000.0;

            assert_eq!(report.events.len(), n / 2 * 2);
            assert_eq!(canon, canon2, "verificación no debe cambiar el estado");

            let bytes = dir_bytes(&dir_a.path().join("events"));
            println!(
                "{{\"n\":{n},\"rep\":{rep},\"t_fold_verify_ms\":{t_verify:.3},\"t_parse_only_ms\":{t_parse:.3},\"store_bytes\":{bytes}}}"
            );
        }
    }
}
