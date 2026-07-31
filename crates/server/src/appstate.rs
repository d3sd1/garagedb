//! Estado compartido del servidor: el `EventStore` (escritor) y la
//! proyección en memoria, siempre regenerable desde disco (`refold`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use serde::Serialize;

use garagedb_core::event::EventBody;
use garagedb_core::fold::{fold, state_canonical_json, State};
use garagedb_core::store::{EventStore, StoreError};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("lock envenenado")]
    Poisoned,
}

#[derive(Clone, Debug, Serialize)]
pub struct FoldSummary {
    pub replica: String,
    pub n_events: usize,
    pub n_rejected: usize,
    pub malformed_lines: u32,
    pub n_proposals: usize,
    pub n_anomalies: usize,
    pub last_fold: String,
}

pub struct AppState {
    pub root: PathBuf,
    store: Mutex<EventStore>,
    state: RwLock<State>,
    last_summary: RwLock<FoldSummary>,
}

impl AppState {
    pub fn open(root: &Path) -> Result<Arc<Self>, AppError> {
        let store = EventStore::init(root)?;
        let app = Arc::new(Self {
            root: root.to_path_buf(),
            store: Mutex::new(store),
            state: RwLock::new(State::default()),
            last_summary: RwLock::new(FoldSummary {
                replica: String::new(),
                n_events: 0,
                n_rejected: 0,
                malformed_lines: 0,
                n_proposals: 0,
                n_anomalies: 0,
                last_fold: String::new(),
            }),
        });
        app.refold()?;
        Ok(app)
    }

    /// Rescan de disco + fold + persistencia de `state/` canónico.
    /// Es la operación de "sincronización" del transporte folder: tras un
    /// `git pull`/Syncthing/USB, un refold incorpora lo nuevo.
    pub fn refold(&self) -> Result<FoldSummary, AppError> {
        let store = self.store.lock().map_err(|_| AppError::Poisoned)?;
        let report = store.load_all()?;
        let new_state = fold(&report.events);
        let canonical = state_canonical_json(&new_state)
            .map_err(StoreError::Canonical)?;
        store.write_state(&canonical)?;
        let summary = FoldSummary {
            replica: store.replica.to_string(),
            n_events: report.events.len(),
            n_rejected: report.rejected.len(),
            malformed_lines: report.malformed_lines,
            n_proposals: new_state.proposals.len(),
            n_anomalies: new_state.anomalies.len(),
            last_fold: chrono_now(),
        };
        *self.state.write().map_err(|_| AppError::Poisoned)? = new_state;
        *self.last_summary.write().map_err(|_| AppError::Poisoned)? = summary.clone();
        Ok(summary)
    }

    /// Append + refold. Toda mutación pasa por aquí.
    pub fn append(&self, actor: &str, body: EventBody) -> Result<FoldSummary, AppError> {
        {
            let mut store = self.store.lock().map_err(|_| AppError::Poisoned)?;
            store.append(actor, body)?;
        }
        self.refold()
    }

    pub fn append_many(&self, actor: &str, bodies: Vec<EventBody>) -> Result<FoldSummary, AppError> {
        {
            let mut store = self.store.lock().map_err(|_| AppError::Poisoned)?;
            for body in bodies {
                store.append(actor, body)?;
            }
        }
        self.refold()
    }

    pub fn with_state<T>(&self, f: impl FnOnce(&State) -> T) -> Result<T, AppError> {
        let guard = self.state.read().map_err(|_| AppError::Poisoned)?;
        Ok(f(&guard))
    }

    pub fn summary(&self) -> Result<FoldSummary, AppError> {
        Ok(self.last_summary.read().map_err(|_| AppError::Poisoned)?.clone())
    }
}

fn chrono_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
