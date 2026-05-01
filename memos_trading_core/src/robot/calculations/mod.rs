use crate::robot::interfaces::Calculator;
/// CalculationEngine'i Calculator trait'ine adapte eden struct
pub struct CalculationEngineAdapter {
    pub engine: CalculationEngine,
}

impl CalculationEngineAdapter {
    pub fn new(engine: CalculationEngine) -> Self {
        Self { engine }
    }
}

impl Calculator for CalculationEngineAdapter {
    fn sma(&self, values: &[f64], period: usize) -> crate::Result<f64> {
        self.engine.math().sma(values, period)
    }
    fn rsi(&self, values: &[f64], period: usize) -> crate::Result<f64> {
        crate::robot::calculations::indicators::RSI::last(values, period)
    }
}
// robot/calculations/mod.rs - Tüm teknik göstergeler ve hesaplamalar

pub mod indicators;
pub mod math;

pub use indicators::{
    IndicatorEngine, SMA, RSI, MACD, BollingerBands, ATR, ADX, 
    Stochastic, CCI, VolumeWeightedAverage, KeltnerChannel,
    SuperTrend, DonchianChannel, TEMA, StochasticRSI, VWAP, Ichimoku,
};
pub use math::{Math, StandardDeviation, MovingAverage, PercentageChange};

/// Calculation engine tüm hesaplamaları koordine eder
#[derive(Default)]
pub struct CalculationEngine {
    indicators: IndicatorEngine,
    math: Math,
}

impl CalculationEngine {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn indicators(&self) -> &IndicatorEngine {
        &self.indicators
    }
    
    pub fn math(&self) -> &Math {
        &self.math
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_calculation_engine_initialization() {
        let engine = CalculationEngine::new();
        assert_eq!(engine.indicators().version(), "1.0.0");
    }
}
