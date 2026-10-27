//! Piyasa-agnostik risk katmanı: matematik + plug-in zinciri.
//!
//! `RiskSnapshot` sayesinde motor durumundan bağımsız (bkz. `snapshot.rs`).
pub mod filter;
pub mod guardrails;
pub mod kelly;
pub mod manager;
pub mod metrics;
pub mod risk_gate;
pub mod snapshot;
pub mod var;

pub use filter::{KellyEdgeFilter, RiskContext, RiskFilter, RiskGateFilter, VarFilter};
pub use guardrails::Guardrails;
pub use manager::RiskManager;
pub use risk_gate::RiskGate;
pub use snapshot::RiskSnapshot;
