//! Comparación con un CRDT de referencia (Automerge): el mismo workload de
//! N eventos repartido entre 2 réplicas, cada evento como entrada en un
//! documento Automerge (lista append por réplica), merge en ambos órdenes.
//! Mide save+merge+load; contrasta con el load+verify+fold de GarageDB.

use std::time::Instant;

use automerge::transaction::Transactable;
use automerge::{Automerge, ObjType, ROOT};

fn build_doc(salt: u64, n: usize) -> Automerge {
    let mut doc = Automerge::new();
    let skus = ["M6x20", "M8x30", "BRIDA-200", "DOT4", "6004-2RS"];
    let locs = ["T2-D07", "S1-N3-P4", "CAR1", "T1-A01"];
    let mut tx = doc.transaction();
    let list = tx.put_object(ROOT, "events", ObjType::List).unwrap();
    for i in 0..n {
        let obj = tx.insert_object(&list, i, ObjType::Map).unwrap();
        tx.put(&obj, "sku", skus[(i + salt as usize) % skus.len()]).unwrap();
        tx.put(&obj, "loc", locs[(i * 7 + salt as usize) % locs.len()]).unwrap();
        tx.put(&obj, "delta", (i as i64 % 5) - 2).unwrap();
        tx.put(&obj, "reason", format!("bench-{i}")).unwrap();
    }
    tx.commit();
    doc
}

fn main() {
    let sizes = [300usize, 1000, 3000];
    let reps = 5;
    for &n in &sizes {
        for rep in 0..reps {
            let t0 = Instant::now();
            let doc_a = build_doc(3 + rep as u64, n / 2);
            let doc_b = build_doc(11 + rep as u64, n / 2);
            let t_build = t0.elapsed().as_secs_f64() * 1000.0;

            // merge A<-B y B<-A vía save/load+merge (el camino de sincronización)
            let t1 = Instant::now();
            let saved_a = doc_a.save();
            let saved_b = doc_b.save();
            let mut ab = Automerge::load(&saved_a).unwrap();
            ab.merge(&mut Automerge::load(&saved_b).unwrap()).unwrap();
            let mut ba = Automerge::load(&saved_b).unwrap();
            ba.merge(&mut Automerge::load(&saved_a).unwrap()).unwrap();
            let t_merge = t1.elapsed().as_secs_f64() * 1000.0;

            let bytes = saved_a.len() + saved_b.len();
            println!(
                "{{\"n\":{n},\"rep\":{rep},\"am_build_ms\":{t_build:.3},\"am_merge_both_ms\":{t_merge:.3},\"am_saved_bytes\":{bytes}}}"
            );
        }
    }
}
