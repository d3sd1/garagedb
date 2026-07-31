//! Misiones: check con ruta física, gate tipado de criticidad y cierre con
//! reconciliación. `try_mark_ready` es la ÚNICA vía de construir
//! `MissionState{Ready}` — el gate vive en el sistema de tipos del módulo,
//! no en un `if` de la UI (spec §7).

use std::collections::BTreeMap;

use serde::Serialize;

use crate::event::{Criticality, Disposition, EventBody, MissionState};
use crate::fold::State;
use crate::ids::{LocationId, MissionId, Sku};
use crate::quantity::Quantity;

#[derive(Debug, thiserror::Error)]
pub enum MissionError {
    #[error("misión desconocida: {0}")]
    UnknownMission(MissionId),
    #[error("kit desconocido: {0}")]
    UnknownKit(String),
}

#[derive(Clone, Debug, Serialize)]
pub struct MissingLine {
    pub sku: Sku,
    pub need: u64,
    pub have: u64,
    pub crit: Criticality,
    /// Dónde hay stock estático disponible: (ubicación, unidades).
    pub sources: Vec<(LocationId, u64)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CheckReport {
    pub mission: MissionId,
    pub missing: Vec<MissingLine>,
    /// Faltantes agrupados por ubicación de origen en orden de recorrido
    /// físico (el direccionamiento matricial ordena por mueble/fila/columna).
    pub route: Vec<(LocationId, Vec<(Sku, u64)>)>,
    pub ready: bool,
}

/// Stock disponible de un sku fuera del carro de la misión, por ubicación.
fn static_sources(state: &State, sku: &Sku, cart: &LocationId) -> Vec<(LocationId, u64)> {
    let mut out: Vec<(LocationId, u64)> = state
        .stock
        .iter()
        .filter(|((s, l), _)| s == sku && l != cart)
        .filter_map(|((_, l), cell)| match cell.qty {
            Quantity::Exact { n } if n > 0 => Some((l.clone(), n)),
            Quantity::Estimated { n, .. } if n > 0 => Some((l.clone(), n)),
            _ => None,
        })
        .collect();
    out.sort();
    out
}

/// El carro de una misión es, por convención v1, la ubicación `CAR1` salvo
/// que el vehículo de la misión nombre otra (campo `vehicle` = LocationId
/// del carro si existe como ubicación).
pub fn cart_location(state: &State, id: &MissionId) -> Result<LocationId, MissionError> {
    let m = state.missions.get(id).ok_or_else(|| MissionError::UnknownMission(id.clone()))?;
    let candidate = LocationId::new(m.vehicle.clone());
    if state.locations.contains_key(&candidate) {
        Ok(candidate)
    } else {
        Ok(LocationId::new("CAR1"))
    }
}

pub fn mission_check(state: &State, id: &MissionId) -> Result<CheckReport, MissionError> {
    let m = state.missions.get(id).ok_or_else(|| MissionError::UnknownMission(id.clone()))?;
    let kit = state
        .kits
        .get(&m.kit)
        .ok_or_else(|| MissionError::UnknownKit(m.kit.to_string()))?;
    let cart = cart_location(state, id)?;

    let mut missing: Vec<MissingLine> = Vec::new();
    for line in &kit.lines {
        let have = state
            .stock
            .get(&(line.sku.clone(), cart.clone()))
            .and_then(|c| c.qty.countable())
            .unwrap_or(0);
        if have < line.n {
            missing.push(MissingLine {
                sku: line.sku.clone(),
                need: line.n - have,
                have,
                crit: line.crit,
                sources: static_sources(state, &line.sku, &cart),
            });
        }
    }
    // bloqueantes primero, luego por sku (Criticality deriva Ord con Blocking<Important<Optional)
    missing.sort_by(|a, b| (a.crit, &a.sku).cmp(&(b.crit, &b.sku)));

    // ruta física: agrupar por primera fuente disponible, ordenada por LocationId
    let mut route_map: BTreeMap<LocationId, Vec<(Sku, u64)>> = BTreeMap::new();
    for line in &missing {
        if let Some((loc, _)) = line.sources.first() {
            route_map.entry(loc.clone()).or_default().push((line.sku.clone(), line.need));
        }
    }
    let ready = !missing.iter().any(|l| l.crit == Criticality::Blocking);
    Ok(CheckReport {
        mission: id.clone(),
        missing,
        route: route_map.into_iter().collect(),
        ready,
    })
}

/// Única vía de obtener un `MissionState{Ready}`. Err = bloqueantes ausentes.
pub fn try_mark_ready(
    state: &State,
    id: &MissionId,
) -> Result<EventBody, Vec<MissingLine>> {
    let report = match mission_check(state, id) {
        Ok(r) => r,
        Err(_) => return Err(vec![]),
    };
    if report.ready {
        Ok(EventBody::MissionState { id: id.clone(), state: MissionState::Ready })
    } else {
        Err(report
            .missing
            .into_iter()
            .filter(|l| l.crit == Criticality::Blocking)
            .collect())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CloseLine {
    pub sku: Sku,
    pub n: u64,
    pub disposition: Disposition,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct CloseReport {
    pub consumed: Vec<(Sku, u64)>,
    pub lost: Vec<(Sku, u64)>,
    /// Reposición sugerida desde inventario estático ANTES de comprar:
    /// (sku, unidades necesarias, fuentes estáticas).
    pub restock_suggestions: Vec<(Sku, u64, Vec<(LocationId, u64)>)>,
}

pub fn mission_close_report(
    state: &State,
    id: &MissionId,
    lines: &[CloseLine],
) -> CloseReport {
    let mut report = CloseReport::default();
    let cart = match cart_location(state, id) {
        Ok(c) => c,
        Err(_) => return report,
    };
    for l in lines {
        match l.disposition {
            Disposition::Consumed => {
                report.consumed.push((l.sku.clone(), l.n));
                report.restock_suggestions.push((
                    l.sku.clone(),
                    l.n,
                    static_sources(state, &l.sku, &cart),
                ));
            }
            Disposition::Lost | Disposition::Broken => {
                report.lost.push((l.sku.clone(), l.n));
                report.restock_suggestions.push((
                    l.sku.clone(),
                    l.n,
                    static_sources(state, &l.sku, &cart),
                ));
            }
            _ => {}
        }
    }
    report
}

/// Eventos de cierre: un KitReturn por línea + MissionState{Closed}.
/// El destino de lo devuelto es el propio carro (queda en el carro hasta
/// descarga explícita con Relocate).
pub fn mission_close_events(
    state: &State,
    id: &MissionId,
    lines: &[CloseLine],
) -> Result<Vec<EventBody>, MissionError> {
    if !state.missions.contains_key(id) {
        return Err(MissionError::UnknownMission(id.clone()));
    }
    let cart = cart_location(state, id)?;
    let mut out: Vec<EventBody> = lines
        .iter()
        .map(|l| EventBody::KitReturn {
            mission: id.clone(),
            sku: l.sku.clone(),
            n: l.n,
            to: cart.clone(),
            disposition: l.disposition,
        })
        .collect();
    out.push(EventBody::MissionState { id: id.clone(), state: MissionState::Closed });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{CountSource, CountStatus, Event, KitLine, EVENT_SCHEMA_VERSION};
    use crate::fold::fold;
    use crate::hlc::Hlc;
    use crate::ids::{EventId, KitId, ReplicaId};

    fn ev(ts: u64, seq: u64, body: EventBody) -> Event {
        Event {
            v: EVENT_SCHEMA_VERSION,
            id: EventId::new(format!("{ts}-r-{seq}")),
            hlc: Hlc { ts_ms: ts, counter: 0 },
            wall: "2026-07-31T10:00:00Z".into(),
            replica: ReplicaId::new("r"),
            seq,
            actor: "test".into(),
            body,
            sig: String::new(),
        }
    }

    /// Escenario: kit con transponder (Blocking, 1) y bridas (Important, 8).
    /// Carro CAR1 vacío; transponder en T1-A01 (1 ud), bridas en T1-B02 (100).
    fn scenario(with_transponder_in_cart: bool) -> State {
        let mut evs = vec![
            ev(1, 1, EventBody::LocationUpsert {
                id: LocationId::new("CAR1"),
                parent: Some(LocationId::new("G")),
                zone: "G".into(),
                ctype: "mobile_cart".into(),
                mobile: true,
                aliases: vec![],
            }),
            ev(2, 2, EventBody::Count {
                sku: Sku::new("TRANSPONDER"),
                loc: LocationId::new("T1-A01"),
                qty: Quantity::Exact { n: 1 },
                source: CountSource::Human,
                status: CountStatus::Confirmed,
            }),
            ev(3, 3, EventBody::Count {
                sku: Sku::new("BRIDA-200"),
                loc: LocationId::new("T1-B02"),
                qty: Quantity::Exact { n: 100 },
                source: CountSource::Human,
                status: CountStatus::Confirmed,
            }),
            ev(4, 4, EventBody::KitUpsert {
                id: KitId::new("kit-sprint"),
                name: "Sprint".into(),
                lines: vec![
                    KitLine { sku: Sku::new("TRANSPONDER"), n: 1, crit: Criticality::Blocking, slot: Some("B1.01".into()) },
                    KitLine { sku: Sku::new("BRIDA-200"), n: 8, crit: Criticality::Important, slot: None },
                ],
            }),
            ev(5, 5, EventBody::MissionCreate {
                id: MissionId::new("jarama-0802"),
                date: "2026-08-02".into(),
                circuit: "Jarama".into(),
                kit: KitId::new("kit-sprint"),
                vehicle: "CAR1".into(),
            }),
        ];
        if with_transponder_in_cart {
            evs.push(ev(6, 6, EventBody::KitLoad {
                mission: MissionId::new("jarama-0802"),
                sku: Sku::new("TRANSPONDER"),
                n: 1,
                from: LocationId::new("T1-A01"),
                to: LocationId::new("CAR1"),
            }));
        }
        fold(&evs)
    }

    #[test]
    fn blocking_missing_blocks_ready() {
        let s = scenario(false);
        let id = MissionId::new("jarama-0802");
        let r = mission_check(&s, &id).unwrap();
        assert!(!r.ready);
        assert_eq!(r.missing.len(), 2);
        // bloqueante primero
        assert_eq!(r.missing[0].crit, Criticality::Blocking);
        assert!(matches!(try_mark_ready(&s, &id), Err(blockers) if blockers.len() == 1));
    }

    #[test]
    fn important_missing_warns_but_ready() {
        let s = scenario(true);
        let id = MissionId::new("jarama-0802");
        let r = mission_check(&s, &id).unwrap();
        assert!(r.ready); // solo falta Important
        assert_eq!(r.missing.len(), 1);
        assert_eq!(r.missing[0].sku, Sku::new("BRIDA-200"));
        assert!(matches!(
            try_mark_ready(&s, &id),
            Ok(EventBody::MissionState { state: MissionState::Ready, .. })
        ));
    }

    #[test]
    fn route_groups_by_source_location() {
        let s = scenario(false);
        let r = mission_check(&s, &MissionId::new("jarama-0802")).unwrap();
        let locs: Vec<&str> = r.route.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(locs, vec!["T1-A01", "T1-B02"]); // orden matricial = ruta
    }

    #[test]
    fn close_consumed_suggests_static_restock() {
        let s = scenario(true);
        let id = MissionId::new("jarama-0802");
        let lines = vec![CloseLine {
            sku: Sku::new("BRIDA-200"),
            n: 6,
            disposition: Disposition::Consumed,
        }];
        let report = mission_close_report(&s, &id, &lines);
        assert_eq!(report.consumed, vec![(Sku::new("BRIDA-200"), 6)]);
        assert_eq!(report.restock_suggestions[0].2[0].0, LocationId::new("T1-B02"));
        let evs = mission_close_events(&s, &id, &lines).unwrap();
        assert_eq!(evs.len(), 2); // KitReturn + Closed
    }
}
