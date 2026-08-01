//! Búsqueda con ranking: relevancia textual difusa × prior de uso
//! (frecuencia con decaimiento exponencial + afinidad por día de semana).
//! Todo se computa del propio ledger; sin modelos externos.

use crate::fold::State;
use crate::ids::{LocationId, Sku};
use serde::Serialize;

/// Vida media del decaimiento de uso: 30 días.
const HALF_LIFE_DAYS: f64 = 30.0;
const MS_PER_DAY: f64 = 86_400_000.0;

#[derive(Clone, Debug, Serialize)]
pub struct SearchHit {
    pub sku: Sku,
    pub loc: LocationId,
    pub score: f64,
}

/// Normaliza: minúsculas + sin acentos comunes en español.
fn norm(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

/// ¿`needle` es subsecuencia de `hay`? ("trn" ⊂ "tornillo")
fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// Relevancia de un campo ya normalizado frente a la query normalizada.
fn field_score(q: &str, field: &str) -> f64 {
    if field.is_empty() || q.is_empty() {
        return 0.0;
    }
    if field == q {
        100.0
    } else if field.starts_with(q) {
        60.0
    } else if field.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(q)) {
        45.0
    } else if field.contains(q) {
        30.0
    } else if q.len() >= 3 && is_subsequence(q, field) {
        12.0
    } else {
        0.0
    }
}

/// Relevancia textual de una celda de stock: máximo ponderado sobre campos.
fn text_score(state: &State, sku: &Sku, loc: &LocationId, q: &str) -> f64 {
    let item = state.items.get(sku);
    let lmeta = state.locations.get(loc);
    let mut best: f64 = 0.0;
    let mut consider = |raw: &str, weight: f64| {
        let s = field_score(q, &norm(raw)) * weight;
        if s > best {
            best = s;
        }
    };
    consider(sku.as_str(), 1.0);
    if let Some(i) = item {
        consider(&i.name, 0.95);
        consider(&i.category, 0.5);
    }
    consider(loc.as_str(), 0.7);
    if let Some(l) = lmeta {
        for a in &l.aliases {
            consider(a, 0.8);
        }
    }
    best
}

/// Prior de uso: Σ exp(-ln2·edad/semivida) sobre los eventos del sku,
/// más afinidad por día de semana actual (patrón temporal).
fn usage_prior(state: &State, sku: &Sku, now_ms: u64) -> f64 {
    let Some(ts) = state.usage.get(sku) else { return 0.0 };
    if ts.is_empty() {
        return 0.0;
    }
    let ln2 = std::f64::consts::LN_2;
    let mut decayed = 0.0;
    let mut same_weekday = 0u32;
    let now_wd = weekday(now_ms);
    for &t in ts {
        let age_days = (now_ms.saturating_sub(t)) as f64 / MS_PER_DAY;
        decayed += (-ln2 * age_days / HALF_LIFE_DAYS).exp();
        if weekday(t) == now_wd {
            same_weekday += 1;
        }
    }
    let weekday_boost = same_weekday as f64 / ts.len() as f64 * 0.3;
    (1.0 + decayed).ln() * 0.35 + weekday_boost
}

/// Día de semana 0..6 desde epoch ms (1970-01-01 fue jueves = 3).
fn weekday(ts_ms: u64) -> u8 {
    (((ts_ms / MS_PER_DAY as u64) + 3) % 7) as u8
}

/// Búsqueda rankeada. Query vacía → ranking puro por uso (lo más tocado
/// arriba). Devuelve hasta `limit` resultados con score > 0.
pub fn search(state: &State, query: &str, now_ms: u64, limit: usize) -> Vec<SearchHit> {
    let q = norm(query.trim());
    let mut hits: Vec<SearchHit> = state
        .stock
        .keys()
        .filter_map(|(sku, loc)| {
            let text = if q.is_empty() { 1.0 } else { text_score(state, sku, loc, &q) };
            if text <= 0.0 {
                return None;
            }
            let score = text * (1.0 + usage_prior(state, sku, now_ms));
            Some(SearchHit { sku: sku.clone(), loc: loc.clone(), score })
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.sku.cmp(&b.sku))
            .then_with(|| a.loc.cmp(&b.loc))
    });
    hits.truncate(limit);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{CountSource, CountStatus, Event, EventBody, EVENT_SCHEMA_VERSION};
    use crate::fold::fold;
    use crate::hlc::Hlc;
    use crate::ids::{EventId, ReplicaId};
    use crate::quantity::Quantity;

    fn ev(ts: u64, seq: u64, body: EventBody) -> Event {
        Event {
            v: EVENT_SCHEMA_VERSION,
            id: EventId::new(format!("{ts}-r-{seq}")),
            hlc: Hlc { ts_ms: ts, counter: 0 },
            wall: "2026-08-01T10:00:00Z".into(),
            replica: ReplicaId::new("r"),
            seq,
            actor: "t".into(),
            body,
            sig: String::new(),
        }
    }

    fn ingest(ts: u64, seq: u64, sku: &str, name: &str, loc: &str) -> Event {
        ev(ts, seq, EventBody::Ingest {
            sku: Sku::new(sku),
            name: name.into(),
            category: "cat".into(),
            unit: "ud".into(),
            loc: LocationId::new(loc),
            qty: Quantity::Exact { n: 10 },
        })
    }

    fn mv(ts: u64, seq: u64, sku: &str, loc: &str) -> Event {
        ev(ts, seq, EventBody::Move {
            sku: Sku::new(sku),
            loc: LocationId::new(loc),
            delta: -1,
            reason: "uso".into(),
            mission: None,
        })
    }

    #[test]
    fn prefix_beats_substring_and_fuzzy_matches() {
        let s = fold(&[
            ingest(1, 1, "M6x20-DIN912", "Tornillo M6x20 DIN912 A2", "T2-D07"),
            ingest(2, 2, "AR-M6", "Arandela ancha M6", "T2-D08"),
        ]);
        let hits = search(&s, "m6", 1000, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].sku, Sku::new("M6x20-DIN912")); // prefijo de sku > word-prefix en nombre
        // difuso: subsecuencia con typo parcial
        let hits = search(&s, "trnillo", 1000, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].sku, Sku::new("M6x20-DIN912"));
        // sin acentos
        let s2 = fold(&[ingest(1, 1, "DOT4", "Líquido frenos DOT4", "A1")]);
        assert_eq!(search(&s2, "liquido", 1000, 10).len(), 1);
    }

    #[test]
    fn usage_recency_ranks_first_and_empty_query_ranks_by_usage() {
        let day = 86_400_000u64;
        let now = 100 * day;
        let s = fold(&[
            ingest(1, 1, "BRIDA-200", "Brida 200", "T1"),
            ingest(2, 2, "BRIDA-300", "Brida 300", "T1"),
            // BRIDA-300 muy usada recientemente
            mv(now - day, 3, "BRIDA-300", "T1"),
            mv(now - day / 2, 4, "BRIDA-300", "T1"),
            mv(now - day / 4, 5, "BRIDA-300", "T1"),
            // BRIDA-200 usada hace mucho
            mv(10 * day, 6, "BRIDA-200", "T1"),
        ]);
        let hits = search(&s, "brida", now, 10);
        assert_eq!(hits[0].sku, Sku::new("BRIDA-300"));
        let hits = search(&s, "", now, 10);
        assert_eq!(hits[0].sku, Sku::new("BRIDA-300"));
    }
}
