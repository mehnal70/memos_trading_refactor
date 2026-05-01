//! Advanced Trading Module
//! 
//! trading_cli'den uyarlanmış, modüler ve platform-bağımsız trading işlevleri.
//! İçerir: göstergeler, stratejiler, risk yönetimi, motor

pub mod indicators;
pub mod risk;
pub mod strategies;

// Re-export ana types
pub use indicators::Candle;
pub use risk::{RiskParams, TradeAction, TradeSignal, RiskManager};
pub use strategies::{StrategyResult, StrategyEngine};
