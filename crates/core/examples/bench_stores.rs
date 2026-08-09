//! Comparación de coste de convergencia contra un almacén convencional:
//! el MISMO workload de N eventos aplicado a
//!   (a) GarageDB: load + verify Ed25519 + fold + serialización canónica
//!   (b) SQLite (WAL, transacción única): INSERT de los N eventos como filas
//!       + agregado de stock por SELECT (la operación equivalente al fold)
//! Salida: JSONL por (n, rep) → agrega el orquestador Python.

use std::path::Path;
use std::time::Instant;

use garagedb_core::event::{CountSource, CountStatus, EventBody};
use garagedb_core::fold::{fold, state_canonical_json};
use garagedb_core::ids::{LocationId, Sku};
use garagedb_core::quantity::Quantity;
use garagedb_core::store::EventStore;
use rusqlite::Connection;

fn generate(store: &mut EventStore, salt: u64, n: usize) -> Vec<(String, String, i64, bool, u64)> {
    // devuelve el workload plano (sku, loc, delta, is_anchor, qty) para SQLite
    let skus = ["M6x20", "M8x30", "BRIDA-200", "DOT4", "6004-2RS"];
    let locs = ["T2-D07", "S1-N3-P4", "CAR1", "T1-A01"];
    let mut flat = Vec::with_capacity(n);
    for i in 0..n {
        let sku = skus[(i + salt as usize) % skus.len()];
        let loc = locs[(i * 7 + salt as usize) % locs.len()];
        let (body, row) = match i % 3 {
            0 => {
                let q = ((i as u64 * (salt + 3)) % 40) + 5;
                (
                    EventBody::Count {
                        sku: Sku::new(sku),
                        loc: LocationId::new(loc),
                        qty: Quantity::Exact { n: q },
                        source: CountSource::Human,
                        status: CountStatus::Confirmed,
                    },
                    (sku.to_string(), loc.to_string(), 0i64, true, q),
                )
            }
            1 => {
                let d = (i as i64 % 5) - 2;
                (
                    EventBody::Move {
                        sku: Sku::new(sku),
                        loc: LocationId::new(loc),
                        delta: d,
                        reason: format!("bench-{i}"),
                        mission: None,
                    },
                    (sku.to_string(), loc.to_string(), d, false, 0),
                )
            }
            _ => (
                EventBody::Ingest {
                    sku: Sku::new(sku),
                    name: "item".into(),
                    category: "cat".into(),
                    unit: "ud".into(),
                    loc: LocationId::new(loc),
                    qty: Quantity::Exact { n: 3 },
                },
                (sku.to_string(), loc.to_string(), 3, false, 0),
            ),
        };
        store.append("bench", body).unwrap();
        flat.push(row);
    }
    flat
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), &dest).unwrap();
        }
    }
}

/// SQLite: aplicar el workload como filas y computar el stock agregado.
/// Emula el patrón "BBDD local convencional": una transacción de inserts +
/// una consulta de agregación equivalente al fold (anchor = última fila
/// anchor por celda; deltas posteriores encima).
fn sqlite_apply(db_path: &Path, workload: &[(String, String, i64, bool, u64)]) -> (f64, f64) {
    let conn = Connection::open(db_path).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.execute_batch(
        "CREATE TABLE ev (id INTEGER PRIMARY KEY, sku TEXT, loc TEXT, delta INTEGER, anchor INTEGER, qty INTEGER);",
    )
    .unwrap();

    let t0 = Instant::now();
    {
        let tx = conn.unchecked_transaction().unwrap();
        let mut stmt = tx
            .prepare("INSERT INTO ev (sku, loc, delta, anchor, qty) VALUES (?1, ?2, ?3, ?4, ?5)")
            .unwrap();
        for (sku, loc, delta, anchor, qty) in workload {
            stmt.execute(rusqlite::params![sku, loc, delta, *anchor as i64, *qty as i64])
                .unwrap();
        }
        drop(stmt);
        tx.commit().unwrap();
    }
    let t_insert = t0.elapsed().as_secs_f64() * 1000.0;

    let t1 = Instant::now();
    // agregado equivalente al fold: por celda, qty del último anchor + suma de deltas posteriores
    let mut stmt = conn
        .prepare(
            "SELECT e.sku, e.loc,
                    COALESCE((SELECT a.qty FROM ev a WHERE a.sku=e.sku AND a.loc=e.loc AND a.anchor=1
                              ORDER BY a.id DESC LIMIT 1), 0)
                  + COALESCE((SELECT SUM(d.delta) FROM ev d WHERE d.sku=e.sku AND d.loc=e.loc AND d.anchor=0
                              AND d.id > COALESCE((SELECT MAX(a2.id) FROM ev a2 WHERE a2.sku=e.sku AND a2.loc=e.loc AND a2.anchor=1), 0)), 0)
             FROM ev e GROUP BY e.sku, e.loc ORDER BY e.sku, e.loc",
        )
        .unwrap();
    let rows: Vec<(String, String, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let t_query = t1.elapsed().as_secs_f64() * 1000.0;
    assert!(!rows.is_empty());
    (t_insert, t_query)
}

fn main() {
    let sizes = [300usize, 1000, 3000];
    let reps = 5;
    for &n in &sizes {
        for rep in 0..reps {
            let dir_a = tempfile::tempdir().unwrap();
            let dir_b = tempfile::tempdir().unwrap();
            let mut a = EventStore::init(dir_a.path()).unwrap();
            let mut b = EventStore::init(dir_b.path()).unwrap();
            let mut wl = generate(&mut a, 3 + rep, n / 2);
            wl.extend(generate(&mut b, 11 + rep, n / 2));

            for sub in ["events", "config/replicas"] {
                copy_tree(&dir_b.path().join(sub), &dir_a.path().join(sub));
            }

            // GarageDB
            let t0 = Instant::now();
            let report = a.load_all().unwrap();
            let state = fold(&report.events);
            let _canon = state_canonical_json(&state).unwrap();
            let t_gdb = t0.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(report.events.len(), n);

            // SQLite sobre el mismo workload
            let dbdir = tempfile::tempdir().unwrap();
            let (t_ins, t_q) = sqlite_apply(&dbdir.path().join("inv.db"), &wl);

            println!(
                "{{\"n\":{n},\"rep\":{rep},\"garagedb_ms\":{t_gdb:.3},\"sqlite_insert_ms\":{t_ins:.3},\"sqlite_fold_ms\":{t_q:.3}}}"
            );
        }
    }
}
