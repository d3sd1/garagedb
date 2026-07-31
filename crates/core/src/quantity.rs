//! Cantidades con honestidad epistémica (spec §4). Enteros solo: sin floats
//! en el estado canónico (decisión D3).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillLevel {
    Full,
    Half,
    Low,
    Empty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Quantity {
    /// Contado, pesado o leído de etiqueta de fábrica.
    Exact { n: u64 },
    /// Estimación con intervalo y confianza en % entero.
    Estimated { n: u64, lo: u64, hi: u64, conf_pct: u8 },
    /// Granel/nivel: nunca se finge precisión que la física no da.
    Presence { level: FillLevel },
}

impl Quantity {
    /// Unidades contables para agregados de misión. `Presence` no cuenta:
    /// los agregados jamás mezclan clases (spec §4).
    pub fn countable(&self) -> Option<u64> {
        match self {
            Quantity::Exact { n } => Some(*n),
            Quantity::Estimated { n, .. } => Some(*n),
            Quantity::Presence { .. } => None,
        }
    }
}
