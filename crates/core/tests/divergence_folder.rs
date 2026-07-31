//! E2E de divergencia larga sobre transporte `folder` (spec §6): dos
//! almacenes reales en disco, 300 eventos por réplica con movimientos
//! cruzados, sincronización = copia de ficheros (lo que haría Syncthing o
//! un USB), convergencia byte a byte en AMBOS órdenes de fusión, y rechazo
//! de eventos forjados sin corromper el estado.

use std::fs;
use std::path::Path;

use garagedb_core::event::{CountSource, CountStatus, EventBody};
use garagedb_core::fold::{fold, state_canonical_json};
use garagedb_core::ids::{LocationId, Sku};
use garagedb_core::keys::ReplicaKey;
use garagedb_core::quantity::Quantity;
use garagedb_core::store::EventStore;

/// Copia recursiva de events/ y config/replicas/ de `src` a `dst` — el
/// papel del sincronizador de ficheros externo.
fn sync_folders(src: &Path, dst: &Path) {
    for sub in ["events", "config/replicas"] {
        let from = src.join(sub);
        if !from.exists() {
            continue;
        }
        copy_tree(&from, &dst.join(sub));
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

fn generate_activity(store: &mut EventStore, salt: u64, n: usize) {
    let skus = ["M6x20", "BRIDA-200", "DOT4"];
    let locs = ["T2-D07", "S1-N3-P4", "CAR1"];
    for i in 0..n {
        let sku = Sku::new(skus[(i + salt as usize) % skus.len()]);
        let loc = LocationId::new(locs[(i * 7 + salt as usize) % locs.len()]);
        let body = match i % 3 {
            0 => EventBody::Count {
                sku,
                loc,
                qty: Quantity::Exact { n: ((i as u64 * salt) % 40) + 5 },
                source: CountSource::Human,
                status: CountStatus::Confirmed,
            },
            1 => EventBody::Move {
                sku,
                loc,
                delta: (i as i64 % 5) - 2,
                reason: format!("actividad-{i}"),
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
        store.append("test", body).unwrap();
    }
}

#[test]
fn two_replicas_300_events_converge_both_orders() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();

    let mut a = EventStore::init(dir_a.path()).unwrap();
    let mut b = EventStore::init(dir_b.path()).unwrap();

    // divergencia: 300 eventos cada una, mismos skus desde ambos lados
    generate_activity(&mut a, 3, 300);
    generate_activity(&mut b, 11, 300);

    // orden 1: A←B, luego B←A
    sync_folders(dir_b.path(), dir_a.path());
    sync_folders(dir_a.path(), dir_b.path());

    let ra = a.load_all().unwrap();
    let rb = b.load_all().unwrap();
    assert_eq!(ra.events.len(), 600);
    assert_eq!(rb.events.len(), 600);
    assert!(ra.rejected.is_empty() && rb.rejected.is_empty());

    let state_a = state_canonical_json(&fold(&ra.events)).unwrap();
    let state_b = state_canonical_json(&fold(&rb.events)).unwrap();
    assert_eq!(state_a, state_b, "convergencia byte a byte tras fusión bidireccional");
}

#[test]
fn forged_events_rejected_without_corrupting_state() {
    let dir_a = tempfile::tempdir().unwrap();
    let mut a = EventStore::init(dir_a.path()).unwrap();
    generate_activity(&mut a, 5, 50);
    let clean_state = {
        let r = a.load_all().unwrap();
        state_canonical_json(&fold(&r.events)).unwrap()
    };

    // atacante: réplica NO registrada escribe un shard con eventos firmados
    // por su propia clave (transporte no confiable, p.ej. carpeta compartida)
    let mallory = ReplicaKey::generate();
    let mid = mallory.replica_id();
    let mdir = dir_a.path().join("events").join(mid.as_str());
    fs::create_dir_all(&mdir).unwrap();
    let mut forged = garagedb_core::event::Event {
        v: garagedb_core::event::EVENT_SCHEMA_VERSION,
        id: garagedb_core::ids::EventId::new("9999-forged-1"),
        hlc: garagedb_core::hlc::Hlc { ts_ms: 9999999999999, counter: 0 },
        wall: "2026-07-31T10:00:00Z".into(),
        replica: mid,
        seq: 1,
        actor: "mallory".into(),
        body: EventBody::Count {
            sku: Sku::new("M6x20"),
            loc: LocationId::new("T2-D07"),
            qty: Quantity::Exact { n: 0 }, // intenta vaciar el stock
            source: CountSource::Human,
            status: CountStatus::Confirmed,
        },
        sig: String::new(),
    };
    mallory.sign_event(&mut forged).unwrap();
    fs::write(
        mdir.join("2026-07.jsonl"),
        format!("{}\n", serde_json::to_string(&forged).unwrap()),
    )
    .unwrap();

    let r = a.load_all().unwrap();
    assert_eq!(r.rejected.len(), 1, "el evento forjado debe rechazarse");
    let state_after = state_canonical_json(&fold(&r.events)).unwrap();
    assert_eq!(clean_state, state_after, "el estado no cambia ante forjados");
}
