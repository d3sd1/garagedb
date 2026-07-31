//! Canonicalización JSON (decisión D3): claves ordenadas recursivamente,
//! serialización compacta, floats prohibidos. Garantiza `state/` y firmas
//! byte-idénticas entre máquinas.

use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Debug, thiserror::Error)]
pub enum CanonicalError {
    #[error("float no permitido en JSON canónico: {0}")]
    Float(String),
    #[error("serialización: {0}")]
    Serde(#[from] serde_json::Error),
}

fn normalize(v: &Value) -> Result<Value, CanonicalError> {
    match v {
        Value::Object(m) => {
            let mut out = Map::new();
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            for k in keys {
                out.insert(k.clone(), normalize(&m[k])?);
            }
            Ok(Value::Object(out))
        }
        Value::Array(a) => Ok(Value::Array(
            a.iter().map(normalize).collect::<Result<Vec<_>, _>>()?,
        )),
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                Ok(v.clone())
            } else {
                Err(CanonicalError::Float(n.to_string()))
            }
        }
        _ => Ok(v.clone()),
    }
}

/// JSON canónico de un `Value`. `serde_json` preserva el orden de inserción
/// (feature por defecto), por lo que el objeto reordenado serializa ordenado.
pub fn canonical_json(v: &Value) -> Result<String, CanonicalError> {
    Ok(serde_json::to_string(&normalize(v)?)?)
}

pub fn canonical_bytes_of<T: Serialize>(t: &T) -> Result<Vec<u8>, CanonicalError> {
    let v = serde_json::to_value(t)?;
    Ok(canonical_json(&v)?.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_keys_recursively() {
        let v = serde_json::json!({"b":1,"a":{"z":1,"y":[{"k":2,"a":1}]}});
        assert_eq!(
            canonical_json(&v).unwrap(),
            r#"{"a":{"y":[{"a":1,"k":2}],"z":1},"b":1}"#
        );
    }

    #[test]
    fn rejects_floats() {
        assert!(canonical_json(&serde_json::json!({"x": 1.5})).is_err());
    }

    #[test]
    fn accepts_integer_valued_numbers() {
        assert_eq!(
            canonical_json(&serde_json::json!({"x": 3u64})).unwrap(),
            r#"{"x":3}"#
        );
    }
}
