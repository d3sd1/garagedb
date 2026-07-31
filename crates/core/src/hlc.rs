//! Reloj híbrido lógico (HLC). Orden total de eventos por
//! `(hlc.ts_ms, hlc.counter, replica_id, seq)` — decisión D5.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Hlc {
    pub ts_ms: u64,
    pub counter: u32,
}

pub struct HlcClock {
    last: Hlc,
}

impl HlcClock {
    pub fn new() -> Self {
        Self { last: Hlc { ts_ms: 0, counter: 0 } }
    }

    /// Reconstruye el reloj a partir del último HLC persistido (al abrir el store).
    pub fn from_last(last: Hlc) -> Self {
        Self { last }
    }

    /// Tick local. Nunca retrocede aunque el reloj de pared lo haga.
    pub fn tick(&mut self, wall_ms: u64) -> Hlc {
        self.last = if wall_ms > self.last.ts_ms {
            Hlc { ts_ms: wall_ms, counter: 0 }
        } else {
            Hlc { ts_ms: self.last.ts_ms, counter: self.last.counter + 1 }
        };
        self.last
    }

    /// Al importar eventos remotos: avanza por encima del máximo observado.
    pub fn observe(&mut self, remote: Hlc, wall_ms: u64) -> Hlc {
        let base = self.last.max(remote);
        self.last = if wall_ms > base.ts_ms {
            Hlc { ts_ms: wall_ms, counter: 0 }
        } else {
            Hlc { ts_ms: base.ts_ms, counter: base.counter + 1 }
        };
        self.last
    }
}

impl Default for HlcClock {
    fn default() -> Self {
        Self::new()
    }
}

pub fn wall_now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hlc_monotonic_even_if_wall_goes_back() {
        let mut c = HlcClock::new();
        let a = c.tick(1000);
        let b = c.tick(1000); // mismo ms → counter sube
        let d = c.tick(500); // reloj hacia atrás → NO retrocede
        assert!(b > a && d > b);
        assert_eq!(d.ts_ms, 1000);
    }

    #[test]
    fn observe_jumps_over_remote() {
        let mut c = HlcClock::new();
        c.tick(1000);
        let h = c.observe(Hlc { ts_ms: 5000, counter: 7 }, 1000);
        assert!(h > Hlc { ts_ms: 5000, counter: 7 });
        assert_eq!(h.ts_ms, 5000);
    }
}
