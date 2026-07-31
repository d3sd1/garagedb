//! Propiedad de convergencia (Teorema 1): el estado es función del CONJUNTO
//! de eventos, no del orden de llegada ni del orden de fusión.

use garagedb_core::event::{
    total_sort, CountSource, CountStatus, Event, EventBody, EVENT_SCHEMA_VERSION,
};
use garagedb_core::fold::{fold, state_canonical_json};
use garagedb_core::hlc::Hlc;
use garagedb_core::ids::{EventId, LocationId, ReplicaId, Sku};
use garagedb_core::quantity::Quantity;
use proptest::prelude::*;

const SKUS: [&str; 5] = ["M6x20", "M8x30", "BRIDA-200", "DOT4", "6004-2RS"];
const LOCS: [&str; 3] = ["T2-D07", "S1-N3-P4", "CAR1"];

fn make_event(replica: &str, seq: u64, hlc_ts: u64, hlc_counter: u32, pick: u8, a: u8, b: u8) -> Event {
    let sku = Sku::new(SKUS[(a as usize) % SKUS.len()]);
    let loc = LocationId::new(LOCS[(b as usize) % LOCS.len()]);
    let body = match pick % 4 {
        0 => EventBody::Ingest {
            sku,
            name: "item".into(),
            category: "cat".into(),
            unit: "ud".into(),
            loc,
            qty: Quantity::Exact { n: (a as u64) % 20 },
        },
        1 => EventBody::Move {
            sku,
            loc,
            delta: (a as i64 % 7) - 3,
            reason: "prop".into(),
            mission: None,
        },
        2 => EventBody::Count {
            sku,
            loc,
            qty: Quantity::Exact { n: (b as u64) % 50 },
            source: CountSource::Human,
            status: CountStatus::Confirmed,
        },
        _ => EventBody::Count {
            sku,
            loc,
            qty: Quantity::Exact { n: 999 },
            source: CountSource::AiVision,
            status: CountStatus::Proposed,
        },
    };
    Event {
        v: EVENT_SCHEMA_VERSION,
        id: EventId::new(format!("{hlc_ts}-{replica}-{seq}")),
        hlc: Hlc { ts_ms: hlc_ts, counter: hlc_counter },
        wall: "2026-07-31T10:00:00Z".into(),
        replica: ReplicaId::new(replica),
        seq,
        actor: "prop".into(),
        body,
        sig: String::new(), // fold no verifica firmas; eso es del store
    }
}

fn gen_replica(name: &str, seed: u64, n: usize) -> Vec<Event> {
    // PRNG determinista simple (xorshift) para no depender de rand en tests
    let mut s = seed.wrapping_add(0x9E3779B97F4A7C15);
    let mut next = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    (0..n)
        .map(|i| {
            let r = next();
            make_event(
                name,
                i as u64 + 1,
                1000 + (r % 500),      // HLCs entrelazados entre réplicas
                (r >> 9) as u32 % 4,
                (r >> 16) as u8,
                (r >> 24) as u8,
                (r >> 32) as u8,
            )
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn convergence_any_merge_order(seed in any::<u64>()) {
        let evs_a = gen_replica("ra", seed, 60);
        let evs_b = gen_replica("rb", seed.wrapping_mul(31), 60);

        let mut ab = [evs_a.clone(), evs_b.clone()].concat();
        let mut ba = [evs_b, evs_a].concat();
        total_sort(&mut ab);
        total_sort(&mut ba);

        prop_assert_eq!(
            state_canonical_json(&fold(&ab)).unwrap(),
            state_canonical_json(&fold(&ba)).unwrap()
        );
    }

    #[test]
    fn idempotence_duplicates_change_nothing(seed in any::<u64>()) {
        let evs = gen_replica("ra", seed, 40);
        let mut once = evs.clone();
        total_sort(&mut once);

        // duplicar y dedup por id (lo que hace store::load_all)
        let mut twice = [evs.clone(), evs].concat();
        let mut seen = std::collections::BTreeSet::new();
        twice.retain(|e| seen.insert(e.id.clone()));
        total_sort(&mut twice);

        prop_assert_eq!(
            state_canonical_json(&fold(&once)).unwrap(),
            state_canonical_json(&fold(&twice)).unwrap()
        );
    }
}
