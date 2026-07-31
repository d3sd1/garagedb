//! Proyección determinista: fold puro sobre eventos en orden total.
//! Teorema 1: mismo conjunto de eventos ⇒ mismo estado, independiente del
//! orden de fusión (unión conmutativa/asociativa/idempotente + fold sobre
//! orden total). Verificado por proptest en tests/convergence.rs.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::canonical::{canonical_json, CanonicalError};
use crate::event::{
    CountStatus, Criticality, Disposition, Event, EventBody, KitLine, MissionState,
};
use crate::ids::{KitId, LocationId, MissionId, Sku};
use crate::quantity::Quantity;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StockCell {
    pub qty: Quantity,
    pub last_verified: Option<String>,
    pub stale: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ItemMeta {
    pub name: String,
    pub category: String,
    pub unit: String,
    pub crit: Criticality,
    pub stock_min: Option<u64>,
    pub lead_time_days: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LocMeta {
    pub parent: Option<LocationId>,
    pub zone: String,
    pub ctype: String,
    pub mobile: bool,
    pub aliases: Vec<String>,
    /// Para nodos móviles: dónde está AHORA (LocationMove lo actualiza).
    pub current_parent: Option<LocationId>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KitMeta {
    pub name: String,
    pub lines: Vec<KitLine>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MissionMeta {
    pub date: String,
    pub circuit: String,
    pub kit: KitId,
    pub vehicle: String,
    pub state: MissionState,
    /// sku → unidades cargadas al carro.
    pub loaded: BTreeMap<Sku, u64>,
    /// sku → devoluciones (n, disposición).
    pub returned: BTreeMap<Sku, Vec<(u64, Disposition)>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct State {
    pub stock: BTreeMap<(Sku, LocationId), StockCell>,
    pub items: BTreeMap<Sku, ItemMeta>,
    pub locations: BTreeMap<LocationId, LocMeta>,
    pub kits: BTreeMap<KitId, KitMeta>,
    pub missions: BTreeMap<MissionId, MissionMeta>,
    /// COUNT proposed pendientes de confirmación humana/política.
    #[serde(skip)]
    pub proposals: Vec<Event>,
    pub anomalies: Vec<String>,
}

impl State {
    /// Ubicación efectiva actual resolviendo contenedores móviles.
    pub fn effective_zone_path(&self, loc: &LocationId) -> Vec<LocationId> {
        let mut path = vec![loc.clone()];
        let mut cur = loc.clone();
        // límite de profundidad defensivo contra ciclos en datos externos
        for _ in 0..32 {
            let Some(meta) = self.locations.get(&cur) else { break };
            let up = meta.current_parent.clone().or_else(|| meta.parent.clone());
            match up {
                Some(p) => {
                    path.push(p.clone());
                    cur = p;
                }
                None => break,
            }
        }
        path
    }
}

fn cell_mut<'a>(
    state: &'a mut State,
    sku: &Sku,
    loc: &LocationId,
) -> &'a mut StockCell {
    state
        .stock
        .entry((sku.clone(), loc.clone()))
        .or_insert(StockCell { qty: Quantity::Exact { n: 0 }, last_verified: None, stale: false })
}

/// Fold puro. `events` DEBE venir en orden total (store::load_all lo garantiza).
pub fn fold(events: &[Event]) -> State {
    let mut s = State::default();
    for ev in events {
        apply(&mut s, ev);
    }
    s
}

fn apply(s: &mut State, ev: &Event) {
    match &ev.body {
        EventBody::Ingest { sku, name, category, unit, loc, qty } => {
            s.items.entry(sku.clone()).or_insert(ItemMeta {
                name: name.clone(),
                category: category.clone(),
                unit: unit.clone(),
                crit: Criticality::Optional,
                stock_min: None,
                lead_time_days: None,
            });
            let cell = cell_mut(s, sku, loc);
            cell.qty = merge_ingest(cell.qty, *qty);
            cell.last_verified = Some(ev.wall.clone());
        }
        EventBody::Move { sku, loc, delta, .. } => {
            let mut anomaly = None;
            {
                let cell = cell_mut(s, sku, loc);
                match cell.qty {
                    Quantity::Exact { n } => {
                        let next = n as i64 + delta;
                        if next < 0 {
                            anomaly = Some(format!(
                                "clamp a 0: {} en {} ({} {:+})",
                                sku, loc, n, delta
                            ));
                        }
                        cell.qty = Quantity::Exact { n: next.max(0) as u64 };
                    }
                    Quantity::Estimated { n, lo, hi, conf_pct } => {
                        let next = (n as i64 + delta).max(0) as u64;
                        cell.qty = Quantity::Estimated {
                            n: next,
                            lo: (lo as i64 + delta).max(0) as u64,
                            hi: (hi as i64 + delta).max(0) as u64,
                            conf_pct,
                        };
                    }
                    Quantity::Presence { .. } => {
                        anomaly = Some(format!(
                            "move sobre presence ignorado: {} en {} ({:+})",
                            sku, loc, delta
                        ));
                    }
                }
            }
            if let Some(a) = anomaly {
                s.anomalies.push(a);
            }
        }
        EventBody::Relocate { sku, from, to, n } => {
            let take = {
                let cell = cell_mut(s, sku, from);
                match cell.qty {
                    Quantity::Exact { n: have } => {
                        let take = (*n).min(have);
                        cell.qty = Quantity::Exact { n: have - take };
                        take
                    }
                    _ => 0,
                }
            };
            if take < *n {
                s.anomalies.push(format!(
                    "relocate parcial: {} de {} a {} pedía {} y había {}",
                    sku, from, to, n, take
                ));
            }
            let dest = cell_mut(s, sku, to);
            if let Quantity::Exact { n: have } = dest.qty {
                dest.qty = Quantity::Exact { n: have + take };
            }
        }
        EventBody::Count { sku, loc, qty, status, .. } => match status {
            CountStatus::Confirmed => {
                let cell = cell_mut(s, sku, loc);
                cell.qty = *qty;
                cell.last_verified = Some(ev.wall.clone());
                cell.stale = false;
            }
            CountStatus::Proposed => {
                s.proposals.push(ev.clone());
            }
        },
        EventBody::Correct { target, note } => {
            s.anomalies.push(format!("correct sobre {}: {}", target, note));
        }
        EventBody::Retire { sku, loc } => {
            s.stock.remove(&(sku.clone(), loc.clone()));
        }
        EventBody::LocationUpsert { id, parent, zone, ctype, mobile, aliases } => {
            let current = s
                .locations
                .get(id)
                .and_then(|l| l.current_parent.clone())
                .or_else(|| parent.clone());
            s.locations.insert(
                id.clone(),
                LocMeta {
                    parent: parent.clone(),
                    zone: zone.clone(),
                    ctype: ctype.clone(),
                    mobile: *mobile,
                    aliases: aliases.clone(),
                    current_parent: current,
                },
            );
        }
        EventBody::LocationMove { id, new_parent } => {
            if let Some(meta) = s.locations.get_mut(id) {
                meta.current_parent = Some(new_parent.clone());
            } else {
                s.anomalies.push(format!("location_move de ubicación desconocida: {}", id));
            }
        }
        EventBody::ItemUpsert { sku, name, category, unit, crit, stock_min, lead_time_days } => {
            s.items.insert(
                sku.clone(),
                ItemMeta {
                    name: name.clone(),
                    category: category.clone(),
                    unit: unit.clone(),
                    crit: *crit,
                    stock_min: *stock_min,
                    lead_time_days: *lead_time_days,
                },
            );
        }
        EventBody::KitUpsert { id, name, lines } => {
            s.kits.insert(id.clone(), KitMeta { name: name.clone(), lines: lines.clone() });
        }
        EventBody::MissionCreate { id, date, circuit, kit, vehicle } => {
            s.missions.entry(id.clone()).or_insert(MissionMeta {
                date: date.clone(),
                circuit: circuit.clone(),
                kit: kit.clone(),
                vehicle: vehicle.clone(),
                state: MissionState::Preparing,
                loaded: BTreeMap::new(),
                returned: BTreeMap::new(),
            });
        }
        EventBody::MissionState { id, state } => {
            if let Some(m) = s.missions.get_mut(id) {
                m.state = *state;
            } else {
                s.anomalies.push(format!("mission_state de misión desconocida: {}", id));
            }
        }
        EventBody::KitLoad { mission, sku, n, from, to } => {
            // mover stock físico del origen al carro
            apply_move_between(s, sku, from, to, *n);
            if let Some(m) = s.missions.get_mut(mission) {
                *m.loaded.entry(sku.clone()).or_insert(0) += n;
            }
        }
        EventBody::KitReturn { mission, sku, n, to, disposition } => {
            if let Some(m) = s.missions.get_mut(mission) {
                m.returned.entry(sku.clone()).or_default().push((*n, *disposition));
            }
            // solo lo devuelto físicamente vuelve a stock
            if matches!(disposition, Disposition::Returned | Disposition::Misplaced) {
                let dest = cell_mut(s, sku, to);
                if let Quantity::Exact { n: have } = dest.qty {
                    dest.qty = Quantity::Exact { n: have + n };
                }
            }
        }
        EventBody::Touch { loc } => {
            // D8: reservado. Semántica ya definida: decaimiento epistémico.
            let keys: Vec<_> = s
                .stock
                .keys()
                .filter(|(_, l)| l == loc)
                .cloned()
                .collect();
            for k in keys {
                s.stock.get_mut(&k).unwrap().stale = true;
            }
        }
    }
}

fn merge_ingest(prev: Quantity, add: Quantity) -> Quantity {
    match (prev, add) {
        (Quantity::Exact { n: a }, Quantity::Exact { n: b }) => Quantity::Exact { n: a + b },
        // ingesta sobre celda vacía o cambio de clase: manda la nueva observación
        (_, q) => q,
    }
}

fn apply_move_between(s: &mut State, sku: &Sku, from: &LocationId, to: &LocationId, n: u64) {
    let take = {
        let cell = cell_mut(s, sku, from);
        match cell.qty {
            Quantity::Exact { n: have } => {
                let take = n.min(have);
                cell.qty = Quantity::Exact { n: have - take };
                take
            }
            _ => 0,
        }
    };
    if take < n {
        s.anomalies
            .push(format!("kit_load parcial: {} de {} pedía {} y había {}", sku, from, n, take));
    }
    let dest = cell_mut(s, sku, to);
    if let Quantity::Exact { n: have } = dest.qty {
        dest.qty = Quantity::Exact { n: have + take };
    }
}

/// Estado en JSON canónico (D3). `BTreeMap` + canonical_json ⇒ determinista.
/// La clave compuesta (Sku, LocationId) se aplana a "sku|loc" para JSON.
pub fn state_canonical_json(s: &State) -> Result<String, CanonicalError> {
    #[derive(Serialize)]
    struct FlatState<'a> {
        stock: BTreeMap<String, &'a StockCell>,
        items: &'a BTreeMap<Sku, ItemMeta>,
        locations: &'a BTreeMap<LocationId, LocMeta>,
        kits: &'a BTreeMap<KitId, KitMeta>,
        missions: &'a BTreeMap<MissionId, MissionMeta>,
        anomalies: &'a Vec<String>,
        n_proposals: usize,
    }
    let flat = FlatState {
        stock: s
            .stock
            .iter()
            .map(|((sku, loc), cell)| (format!("{}|{}", sku, loc), cell))
            .collect(),
        items: &s.items,
        locations: &s.locations,
        kits: &s.kits,
        missions: &s.missions,
        anomalies: &s.anomalies,
        n_proposals: s.proposals.len(),
    };
    let v = serde_json::to_value(&flat)?;
    canonical_json(&v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{CountSource, EVENT_SCHEMA_VERSION};
    use crate::hlc::Hlc;
    use crate::ids::EventId;

    fn ev(ts: u64, seq: u64, body: EventBody) -> Event {
        Event {
            v: EVENT_SCHEMA_VERSION,
            id: EventId::new(format!("{ts}-r-{seq}")),
            hlc: Hlc { ts_ms: ts, counter: 0 },
            wall: "2026-07-31T10:00:00Z".into(),
            replica: crate::ids::ReplicaId::new("r"),
            seq,
            actor: "test".into(),
            body,
            sig: String::new(),
        }
    }

    fn sku() -> Sku {
        Sku::new("M6x20")
    }
    fn loc() -> LocationId {
        LocationId::new("T2-D07")
    }

    fn count(ts: u64, seq: u64, n: u64, status: CountStatus) -> Event {
        ev(
            ts,
            seq,
            EventBody::Count {
                sku: sku(),
                loc: loc(),
                qty: Quantity::Exact { n },
                source: CountSource::Human,
                status,
            },
        )
    }

    fn mv(ts: u64, seq: u64, delta: i64) -> Event {
        ev(
            ts,
            seq,
            EventBody::Move { sku: sku(), loc: loc(), delta, reason: "t".into(), mission: None },
        )
    }

    #[test]
    fn count_anchor_then_move() {
        let s = fold(&[count(1, 1, 10, CountStatus::Confirmed), mv(2, 2, -3)]);
        assert_eq!(s.stock[&(sku(), loc())].qty, Quantity::Exact { n: 7 });
    }

    #[test]
    fn proposed_count_never_touches_stock() {
        let s = fold(&[
            count(1, 1, 10, CountStatus::Confirmed),
            mv(2, 2, -3),
            count(3, 3, 99, CountStatus::Proposed),
        ]);
        assert_eq!(s.stock[&(sku(), loc())].qty, Quantity::Exact { n: 7 });
        assert_eq!(s.proposals.len(), 1);
    }

    #[test]
    fn clamp_to_zero_records_anomaly() {
        let s = fold(&[count(1, 1, 2, CountStatus::Confirmed), mv(2, 2, -5)]);
        assert_eq!(s.stock[&(sku(), loc())].qty, Quantity::Exact { n: 0 });
        assert_eq!(s.anomalies.len(), 1);
    }

    #[test]
    fn location_move_does_not_touch_stock_keys() {
        let cart = LocationId::new("CAR1");
        let s = fold(&[
            ev(
                1,
                1,
                EventBody::LocationUpsert {
                    id: cart.clone(),
                    parent: Some(LocationId::new("G")),
                    zone: "G".into(),
                    ctype: "mobile_cart".into(),
                    mobile: true,
                    aliases: vec![],
                },
            ),
            ev(
                2,
                2,
                EventBody::Count {
                    sku: sku(),
                    loc: cart.clone(),
                    qty: Quantity::Exact { n: 4 },
                    source: CountSource::Human,
                    status: CountStatus::Confirmed,
                },
            ),
            ev(3, 3, EventBody::LocationMove { id: cart.clone(), new_parent: LocationId::new("V-FURGO") }),
        ]);
        assert_eq!(s.stock[&(sku(), cart.clone())].qty, Quantity::Exact { n: 4 });
        assert_eq!(
            s.locations[&cart].current_parent,
            Some(LocationId::new("V-FURGO"))
        );
        let path = s.effective_zone_path(&cart);
        assert!(path.contains(&LocationId::new("V-FURGO")));
    }

    #[test]
    fn touch_marks_stale() {
        let s = fold(&[
            count(1, 1, 10, CountStatus::Confirmed),
            ev(2, 2, EventBody::Touch { loc: loc() }),
        ]);
        assert!(s.stock[&(sku(), loc())].stale);
    }

    #[test]
    fn canonical_state_stable() {
        let s = fold(&[count(1, 1, 10, CountStatus::Confirmed)]);
        let a = state_canonical_json(&s).unwrap();
        let b = state_canonical_json(&s).unwrap();
        assert_eq!(a, b);
        assert!(a.contains("M6x20|T2-D07"));
    }
}
