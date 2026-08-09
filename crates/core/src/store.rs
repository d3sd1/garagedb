//! Almacén de eventos en disco: `events/<replica>/<YYYY-MM>.jsonl`,
//! append-only, particionado por réplica (dos réplicas jamás escriben el
//! mismo fichero → la fusión es unión de ficheros y cualquier sincronizador
//! de ficheros es transporte válido).

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use ed25519_dalek::VerifyingKey;

use crate::event::{total_sort, Event, EventBody, EVENT_SCHEMA_VERSION};
use crate::hlc::{wall_now_ms, Hlc, HlcClock};
use crate::ids::{EventId, ReplicaId};
use crate::keys::{replica_id_of, verify_event, verifying_key_from_hex, KeyError, ReplicaKey};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("clave: {0}")]
    Key(#[from] KeyError),
    #[error("serialización: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("canonical: {0}")]
    Canonical(#[from] crate::canonical::CanonicalError),
    #[error("almacén inválido: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct RejectedEvent {
    pub file: PathBuf,
    pub line_no: usize,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct LoadReport {
    /// Eventos válidos, YA en orden total y deduplicados por id.
    pub events: Vec<Event>,
    pub rejected: Vec<RejectedEvent>,
    pub malformed_lines: u32,
}

pub struct EventStore {
    pub root: PathBuf,
    pub key: ReplicaKey,
    pub replica: ReplicaId,
    clock: HlcClock,
    seq: u64,
}

const LOCAL_DIR: &str = ".local";
const KEY_FILE: &str = ".local/replica.key";
const EVENTS_DIR: &str = "events";
const REPLICAS_DIR: &str = "config/replicas";
const STATE_DIR: &str = "state";

impl EventStore {
    /// Crea el layout completo de un almacén nuevo (o adopta uno existente
    /// sin clave local) y publica la clave pública de esta réplica.
    pub fn init(root: &Path) -> Result<Self, StoreError> {
        fs::create_dir_all(root.join(EVENTS_DIR))?;
        fs::create_dir_all(root.join(REPLICAS_DIR))?;
        fs::create_dir_all(root.join(STATE_DIR))?;
        fs::create_dir_all(root.join("media"))?;
        fs::create_dir_all(root.join(LOCAL_DIR))?;
        // derivados y clave privada fuera de sincronización
        let gitignore = root.join(".gitignore");
        if !gitignore.exists() {
            fs::write(&gitignore, ".local/\nstate/\n")?;
        }
        let stignore = root.join(".stignore");
        if !stignore.exists() {
            fs::write(&stignore, ".local\nstate\n")?;
        }
        Self::open(root)
    }

    /// Abre un almacén; genera clave si no existe; reconstruye seq y HLC
    /// desde el shard propio.
    pub fn open(root: &Path) -> Result<Self, StoreError> {
        let key_path = root.join(KEY_FILE);
        let key = if key_path.exists() {
            ReplicaKey::load(&key_path)?
        } else {
            let k = ReplicaKey::generate();
            k.save(&key_path)?;
            k
        };
        let replica = key.replica_id();
        // publicar clave pública
        let pub_path = root.join(REPLICAS_DIR).join(format!("{replica}.pub"));
        if !pub_path.exists() {
            fs::create_dir_all(pub_path.parent().unwrap())?;
            fs::write(&pub_path, key.public_hex())?;
        }
        // reconstruir seq/HLC del shard propio
        let mut seq = 0u64;
        let mut last_hlc = Hlc { ts_ms: 0, counter: 0 };
        let own_dir = root.join(EVENTS_DIR).join(replica.as_str());
        if own_dir.exists() {
            for entry in read_sorted(&own_dir)? {
                let content = fs::read_to_string(&entry)?;
                for line in content.lines() {
                    if let Ok(ev) = serde_json::from_str::<Event>(line) {
                        seq = seq.max(ev.seq);
                        last_hlc = last_hlc.max(ev.hlc);
                    }
                }
            }
        }
        Ok(Self {
            root: root.to_path_buf(),
            replica,
            key,
            clock: HlcClock::from_last(last_hlc),
            seq,
        })
    }

    /// Firma y persiste un evento nuevo en el shard propio del mes en curso.
    pub fn append(&mut self, actor: &str, body: EventBody) -> Result<Event, StoreError> {
        let wall_ms = wall_now_ms();
        let hlc = self.clock.tick(wall_ms);
        self.seq += 1;
        let wall = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let mut ev = Event {
            v: EVENT_SCHEMA_VERSION,
            id: EventId::new(format!("{}-{}-{}", hlc.ts_ms, self.replica, self.seq)),
            hlc,
            wall,
            replica: self.replica.clone(),
            seq: self.seq,
            actor: actor.to_string(),
            body,
            sig: String::new(),
        };
        self.key.sign_event(&mut ev)?;

        let month = chrono::Utc::now().format("%Y-%m").to_string();
        let dir = self.root.join(EVENTS_DIR).join(self.replica.as_str());
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{month}.jsonl"));
        let mut line = serde_json::to_string(&ev)?;
        line.push('\n');
        let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
        f.write_all(line.as_bytes())?;
        f.sync_data()?;
        Ok(ev)
    }

    /// Registro de claves públicas conocidas.
    fn known_replicas(&self) -> Result<BTreeMap<ReplicaId, VerifyingKey>, StoreError> {
        let mut out = BTreeMap::new();
        let dir = self.root.join(REPLICAS_DIR);
        if dir.exists() {
            for entry in read_sorted(&dir)? {
                if entry.extension().and_then(|e| e.to_str()) != Some("pub") {
                    continue;
                }
                let id = ReplicaId::new(
                    entry.file_stem().and_then(|s| s.to_str()).unwrap_or_default(),
                );
                let hexstr = fs::read_to_string(&entry)?;
                match verifying_key_from_hex(&hexstr) {
                    Ok(vk) if replica_id_of(&vk) == id => {
                        out.insert(id, vk);
                    }
                    _ => { /* .pub corrupta: las firmas de esa réplica caerán en rejected */ }
                }
            }
        }
        Ok(out)
    }

    /// Carga TODOS los shards de todas las réplicas, verifica, deduplica y
    /// ordena. Nunca panic por datos externos.
    pub fn load_all(&self) -> Result<LoadReport, StoreError> {
        let mut report = LoadReport::default();
        let keys = self.known_replicas()?;
        let events_root = self.root.join(EVENTS_DIR);
        let mut seen = std::collections::BTreeSet::new();
        // detección de fork: dos eventos distintos no pueden compartir (replica, seq)
        let mut seq_owner: BTreeMap<(ReplicaId, u64), EventId> = BTreeMap::new();

        if events_root.exists() {
            for replica_dir in read_sorted(&events_root)? {
                if !replica_dir.is_dir() {
                    continue;
                }
                let dir_replica = ReplicaId::new(
                    replica_dir.file_name().and_then(|s| s.to_str()).unwrap_or_default(),
                );
                for shard in read_sorted(&replica_dir)? {
                    if shard.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                        continue;
                    }
                    let content = fs::read_to_string(&shard)?;
                    for (i, line) in content.lines().enumerate() {
                        if line.trim().is_empty() {
                            continue;
                        }
                        let Ok(ev) = serde_json::from_str::<Event>(line) else {
                            // línea malformada (p. ej. crash a mitad de escritura)
                            report.malformed_lines += 1;
                            continue;
                        };
                        let reject = |reason: &str| RejectedEvent {
                            file: shard.clone(),
                            line_no: i + 1,
                            reason: reason.to_string(),
                        };
                        if ev.v > EVENT_SCHEMA_VERSION {
                            // A4: nunca proyectar eventos de un esquema más nuevo
                            report.rejected.push(reject("versión de esquema superior a la de este binario"));
                            continue;
                        }
                        if ev.replica != dir_replica {
                            report.rejected.push(reject("réplica del evento no coincide con su shard"));
                            continue;
                        }
                        let Some(vk) = keys.get(&ev.replica) else {
                            report.rejected.push(reject("réplica sin clave pública registrada"));
                            continue;
                        };
                        if !verify_event(&ev, vk) {
                            report.rejected.push(reject("firma inválida"));
                            continue;
                        }
                        match seq_owner.get(&(ev.replica.clone(), ev.seq)) {
                            Some(existing) if *existing != ev.id => {
                                // shard bifurcado (backup restaurado / réplica clonada):
                                // nunca fusionar silenciosamente
                                report.rejected.push(reject(
                                    "fork de réplica: (replica, seq) duplicado con id distinto",
                                ));
                                continue;
                            }
                            _ => {
                                seq_owner
                                    .insert((ev.replica.clone(), ev.seq), ev.id.clone());
                            }
                        }
                        if seen.insert(ev.id.clone()) {
                            report.events.push(ev);
                        }
                    }
                }
            }
        }
        total_sort(&mut report.events);
        Ok(report)
    }

    /// Escribe la proyección canónica derivada en `state/`.
    pub fn write_state(&self, canonical: &str) -> Result<(), StoreError> {
        let dir = self.root.join(STATE_DIR);
        fs::create_dir_all(&dir)?;
        let tmp = dir.join("stock.json.tmp");
        fs::write(&tmp, canonical)?;
        fs::rename(&tmp, dir.join("stock.json"))?;
        Ok(())
    }
}

/// Lectura de directorio con orden determinista (por nombre).
fn read_sorted(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{LocationId, Sku};
    use crate::quantity::Quantity;

    fn mv(delta: i64) -> EventBody {
        EventBody::Move {
            sku: Sku::new("M6x20"),
            loc: LocationId::new("T2-D07"),
            delta,
            reason: "test".into(),
            mission: None,
        }
    }

    #[test]
    fn init_creates_layout() {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::init(dir.path()).unwrap();
        assert!(dir.path().join(".local/replica.key").exists());
        assert!(dir
            .path()
            .join(format!("config/replicas/{}.pub", store.replica))
            .exists());
        assert!(dir.path().join(".gitignore").exists());
    }

    #[test]
    fn append_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = EventStore::init(dir.path()).unwrap();
        store.append("andrei", mv(5)).unwrap();
        store.append("andrei", mv(-2)).unwrap();
        let report = store.load_all().unwrap();
        assert_eq!(report.events.len(), 2);
        assert!(report.rejected.is_empty());
        assert_eq!(report.malformed_lines, 0);
        // reabrir conserva seq
        let mut store2 = EventStore::open(dir.path()).unwrap();
        let ev = store2.append("andrei", mv(1)).unwrap();
        assert_eq!(ev.seq, 3);
    }

    #[test]
    fn malformed_trailing_line_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = EventStore::init(dir.path()).unwrap();
        store.append("andrei", mv(5)).unwrap();
        // simular crash: media línea al final del shard
        let shard_dir = dir.path().join("events").join(store.replica.as_str());
        let shard = read_sorted(&shard_dir).unwrap().pop().unwrap();
        let mut f = OpenOptions::new().append(true).open(&shard).unwrap();
        f.write_all(b"{\"v\":1,\"trunca").unwrap();
        let report = store.load_all().unwrap();
        assert_eq!(report.events.len(), 1);
        assert_eq!(report.malformed_lines, 1);
    }

    #[test]
    fn corrupted_signature_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = EventStore::init(dir.path()).unwrap();
        let ev = store.append("andrei", mv(5)).unwrap();
        let shard_dir = dir.path().join("events").join(store.replica.as_str());
        let shard = read_sorted(&shard_dir).unwrap().pop().unwrap();
        // reescribir el shard con la firma corrupta
        let mut tampered = ev.clone();
        tampered.actor = "mallory".into(); // firma ya no cuadra
        fs::write(&shard, format!("{}\n", serde_json::to_string(&tampered).unwrap())).unwrap();
        let report = store.load_all().unwrap();
        assert!(report.events.is_empty());
        assert_eq!(report.rejected.len(), 1);
        assert!(report.rejected[0].reason.contains("firma"));
    }

    #[test]
    fn unknown_replica_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let store = EventStore::init(dir.path()).unwrap();
        // shard de una réplica cuya .pub no está registrada
        let foreign = ReplicaKey::generate();
        let fid = foreign.replica_id();
        let fdir = dir.path().join("events").join(fid.as_str());
        fs::create_dir_all(&fdir).unwrap();
        let mut ev = Event {
            v: EVENT_SCHEMA_VERSION,
            id: EventId::new("1-x-1"),
            hlc: Hlc { ts_ms: 1, counter: 0 },
            wall: "2026-07-31T10:00:00Z".into(),
            replica: fid,
            seq: 1,
            actor: "ghost".into(),
            body: mv(99),
            sig: String::new(),
        };
        foreign.sign_event(&mut ev).unwrap();
        fs::write(
            fdir.join("2026-07.jsonl"),
            format!("{}\n", serde_json::to_string(&ev).unwrap()),
        )
        .unwrap();
        let report = store.load_all().unwrap();
        assert!(report.events.is_empty());
        assert_eq!(report.rejected.len(), 1);
        assert!(report.rejected[0].reason.contains("sin clave"));
    }

    #[test]
    fn quantity_in_event_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = EventStore::init(dir.path()).unwrap();
        store
            .append(
                "andrei",
                EventBody::Ingest {
                    sku: Sku::new("DOT4"),
                    name: "Líquido de frenos DOT4".into(),
                    category: "liquids".into(),
                    unit: "ml".into(),
                    loc: LocationId::new("A1-N1-P1"),
                    qty: Quantity::Exact { n: 500 },
                },
            )
            .unwrap();
        let report = store.load_all().unwrap();
        assert_eq!(report.events.len(), 1);
    }
}
