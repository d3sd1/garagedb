//! Eventos del log: inmutables, firmados ed25519, con orden total HLC.
//! El tipo `Touch` está reservado (decisión D8): existe en el esquema pero
//! ningún productor lo emite en v1.

use serde::{Deserialize, Serialize};

use crate::hlc::Hlc;
use crate::ids::{EventId, KitId, LocationId, MissionId, ReplicaId, Sku};
use crate::quantity::Quantity;

pub const EVENT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountSource {
    Human,
    Barcode,
    Scale,
    AiVision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountStatus {
    Proposed,
    Confirmed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Criticality {
    /// Sin esto no se rueda. Bloquea `Mission::Ready`.
    Blocking,
    /// Se improvisa en paddock, pero cuesta caro. Avisa, no bloquea.
    Important,
    Optional,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Consumed,
    Broken,
    Lost,
    Lent,
    Misplaced,
    Returned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionState {
    Preparing,
    Ready,
    EnRoute,
    AtTrack,
    Returning,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KitLine {
    pub sku: Sku,
    pub n: u64,
    pub crit: Criticality,
    /// Posición fija dentro del carro (spec §7): la carga es reproducible.
    pub slot: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EventBody {
    Ingest {
        sku: Sku,
        name: String,
        category: String,
        unit: String,
        loc: LocationId,
        qty: Quantity,
    },
    Move {
        sku: Sku,
        loc: LocationId,
        delta: i64,
        reason: String,
        mission: Option<MissionId>,
    },
    Relocate {
        sku: Sku,
        from: LocationId,
        to: LocationId,
        n: u64,
    },
    Count {
        sku: Sku,
        loc: LocationId,
        qty: Quantity,
        source: CountSource,
        status: CountStatus,
    },
    Correct {
        target: EventId,
        note: String,
    },
    Retire {
        sku: Sku,
        loc: LocationId,
    },
    LocationUpsert {
        id: LocationId,
        parent: Option<LocationId>,
        zone: String,
        ctype: String,
        mobile: bool,
        aliases: Vec<String>,
    },
    /// Contenedor móvil cambia de padre: UN evento, el contenido viaja
    /// resolviendo el árbol en consulta (spec §4).
    LocationMove {
        id: LocationId,
        new_parent: LocationId,
    },
    ItemUpsert {
        sku: Sku,
        name: String,
        category: String,
        unit: String,
        crit: Criticality,
        stock_min: Option<u64>,
        lead_time_days: Option<u32>,
    },
    KitUpsert {
        id: KitId,
        name: String,
        lines: Vec<KitLine>,
    },
    MissionCreate {
        id: MissionId,
        date: String,
        circuit: String,
        kit: KitId,
        vehicle: String,
    },
    MissionState {
        id: MissionId,
        state: MissionState,
    },
    KitLoad {
        mission: MissionId,
        sku: Sku,
        n: u64,
        from: LocationId,
        to: LocationId,
    },
    KitReturn {
        mission: MissionId,
        sku: Sku,
        n: u64,
        to: LocationId,
        disposition: Disposition,
    },
    /// RESERVADO (D8, future work cámaras): marca stock de una ubicación
    /// como `stale`. Ningún productor en v1.
    Touch { loc: LocationId },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub v: u32,
    pub id: EventId,
    pub hlc: Hlc,
    /// RFC3339, informativo. El orden usa SOLO el HLC.
    pub wall: String,
    pub replica: ReplicaId,
    pub seq: u64,
    pub actor: String,
    /// Anidado a propósito: `flatten` colisionaría el `id` del evento con
    /// los `id` de MissionCreate/KitUpsert/LocationUpsert.
    pub body: EventBody,
    /// hex(ed25519) sobre los bytes canónicos del evento con `sig:""`.
    pub sig: String,
}

impl Event {
    /// Clave de orden total (decisión D2/D5).
    pub fn total_order_key(&self) -> (u64, u32, &str, u64) {
        (self.hlc.ts_ms, self.hlc.counter, self.replica.as_str(), self.seq)
    }
}

/// Orden total in-place.
pub fn total_sort(events: &mut [Event]) {
    events.sort_by(|a, b| a.total_order_key().cmp(&b.total_order_key()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quantity::FillLevel;

    #[test]
    fn body_roundtrip_tagged() {
        let b = EventBody::Count {
            sku: Sku::new("M6x20"),
            loc: LocationId::new("T2-D07"),
            qty: Quantity::Presence { level: FillLevel::Half },
            source: CountSource::AiVision,
            status: CountStatus::Proposed,
        };
        let s = serde_json::to_string(&b).unwrap();
        assert!(s.contains(r#""op":"count""#));
        assert!(s.contains(r#""status":"proposed""#));
        let back: EventBody = serde_json::from_str(&s).unwrap();
        assert_eq!(b, back);
    }

    #[test]
    fn criticality_orders_blocking_first() {
        assert!(Criticality::Blocking < Criticality::Important);
        assert!(Criticality::Important < Criticality::Optional);
    }
}
