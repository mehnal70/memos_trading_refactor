// src/robot/engines/base.rs - Motor Baz Sözleşmeleri

use crate::prelude::*;

/// Evrensel Motor Arayüzü (Trait)
pub trait TradingEngine {
    fn name(&self) -> &str;
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub initial_balance: f64,
    pub strategy_params: StrategyParams,
    pub ml_enabled: bool,
    pub monitor_enabled: bool,
}
