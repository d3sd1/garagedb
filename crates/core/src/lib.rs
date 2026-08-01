#![forbid(unsafe_code)]
//! GarageDB core: dominio, log de eventos firmado, fold determinista.
//!
//! Diseño: docs/superpowers/specs/2026-07-31-garagedb-design.md (repo clarividence).
//! Convergencia = unión de conjuntos de eventos + fold determinista sobre orden
//! total (hlc, replica, seq). Sin floats en el estado canónico.

pub mod canonical;
pub mod event;
pub mod fold;
pub mod hlc;
pub mod ids;
pub mod keys;
pub mod mission;
pub mod quantity;
pub mod search;
pub mod store;
