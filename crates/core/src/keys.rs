//! Identidad de réplica: par de claves ed25519. `replica_id` = primeros 16
//! hex del SHA-256 de la clave pública. La privada vive en `.local/` (fuera
//! de sincronización); las públicas se comparten en `config/replicas/`.

use std::fs;
use std::path::Path;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::canonical::canonical_bytes_of;
use crate::event::Event;
use crate::ids::ReplicaId;

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("clave inválida: {0}")]
    Invalid(String),
    #[error("canonical: {0}")]
    Canonical(#[from] crate::canonical::CanonicalError),
}

pub struct ReplicaKey {
    pub signing: SigningKey,
}

pub fn replica_id_of(vk: &VerifyingKey) -> ReplicaId {
    let h = Sha256::digest(vk.to_bytes());
    ReplicaId::new(&hex::encode(h)[..16])
}

impl ReplicaKey {
    pub fn generate() -> Self {
        let mut rng = rand::rngs::OsRng;
        Self { signing: SigningKey::generate(&mut rng) }
    }

    pub fn replica_id(&self) -> ReplicaId {
        replica_id_of(&self.signing.verifying_key())
    }

    pub fn save(&self, path: &Path) -> Result<(), KeyError> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, self.signing.to_bytes())?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, KeyError> {
        let bytes = fs::read(path)?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| KeyError::Invalid("clave privada debe ser 32 bytes".into()))?;
        Ok(Self { signing: SigningKey::from_bytes(&arr) })
    }

    pub fn public_hex(&self) -> String {
        hex::encode(self.signing.verifying_key().to_bytes())
    }

    /// Firma un evento: hex(ed25519) sobre bytes canónicos con `sig:""`.
    pub fn sign_event(&self, ev: &mut Event) -> Result<(), KeyError> {
        ev.sig = String::new();
        let bytes = canonical_bytes_of(ev)?;
        let sig: Signature = self.signing.sign(&bytes);
        ev.sig = hex::encode(sig.to_bytes());
        Ok(())
    }
}

pub fn verifying_key_from_hex(s: &str) -> Result<VerifyingKey, KeyError> {
    let bytes = hex::decode(s.trim())
        .map_err(|e| KeyError::Invalid(format!("hex: {e}")))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| KeyError::Invalid("clave pública debe ser 32 bytes".into()))?;
    VerifyingKey::from_bytes(&arr).map_err(|e| KeyError::Invalid(e.to_string()))
}

/// Verifica firma de un evento contra la clave pública de su réplica.
pub fn verify_event(ev: &Event, vk: &VerifyingKey) -> bool {
    let mut unsigned = ev.clone();
    unsigned.sig = String::new();
    let Ok(bytes) = canonical_bytes_of(&unsigned) else {
        return false;
    };
    let Ok(sig_bytes) = hex::decode(&ev.sig) else {
        return false;
    };
    let Ok(arr) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else {
        return false;
    };
    vk.verify(&bytes, &Signature::from_bytes(&arr)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventBody, EVENT_SCHEMA_VERSION};
    use crate::hlc::Hlc;
    use crate::ids::{EventId, LocationId, Sku};

    fn sample_event(key: &ReplicaKey) -> Event {
        let mut ev = Event {
            v: EVENT_SCHEMA_VERSION,
            id: EventId::new("1000-x-1"),
            hlc: Hlc { ts_ms: 1000, counter: 0 },
            wall: "2026-07-31T10:00:00Z".into(),
            replica: key.replica_id(),
            seq: 1,
            actor: "andrei".into(),
            body: EventBody::Move {
                sku: Sku::new("M6x20"),
                loc: LocationId::new("T2-D07"),
                delta: -4,
                reason: "montaje".into(),
                mission: None,
            },
            sig: String::new(),
        };
        key.sign_event(&mut ev).unwrap();
        ev
    }

    #[test]
    fn sign_verify_roundtrip_and_tamper() {
        let key = ReplicaKey::generate();
        let ev = sample_event(&key);
        let vk = key.signing.verifying_key();
        assert!(verify_event(&ev, &vk));
        let mut tampered = ev.clone();
        tampered.actor = "mallory".into();
        assert!(!verify_event(&tampered, &vk));
    }

    #[test]
    fn save_load_stable_id(){
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".local/replica.key");
        let key = ReplicaKey::generate();
        key.save(&p).unwrap();
        let loaded = ReplicaKey::load(&p).unwrap();
        assert_eq!(key.replica_id(), loaded.replica_id());
    }

    #[test]
    fn public_hex_roundtrip() {
        let key = ReplicaKey::generate();
        let vk = verifying_key_from_hex(&key.public_hex()).unwrap();
        assert_eq!(replica_id_of(&vk), key.replica_id());
    }
}
