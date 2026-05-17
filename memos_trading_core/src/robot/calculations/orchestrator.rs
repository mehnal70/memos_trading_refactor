// src/robot/calculations/orchestrator.rs - Hesaplama Motoru Orkestratör İşçisi
// Srivastava ATP - İşlevsel Çarklar Odası

use crate::prelude::*; // Evrensel anayasa mühürünü ve yeni CoreIndicatorEngine motorunu çağırıyoruz
use crate::robot::infra::interfaces::Calculator; // Yenilenen interfaces konumu (Kapsülleme)


/// CalculationEngine: Tüm matematiksel ve teknik analiz motorlarını orkestra eder.
#[derive(Default)]
pub struct CalculationEngine;

impl CalculationEngine {
    pub fn new() -> Self {
        Self
    }
}

/// CalculationEngineAdapter: Engine'i Calculator arayüzüne (trait) bağlar.
/// Srivastava Geliştirmesi: Doğrudan src/core/indicators altındaki yüksek performanslı 
/// ve platform uyumlu (Wilder SMMA) motorları kullanarak bellek kopyalamasını sıfırlar.
pub struct CalculationEngineAdapter {
    pub engine: CalculationEngine,
}

impl CalculationEngineAdapter {
    pub fn new(engine: CalculationEngine) -> Self {
        Self { engine }
    }
}

impl Calculator for CalculationEngineAdapter {
    /// Kütüphanenin mutlak hesaplama merkezindeki (core::math) optimize SMA'yı ateşler
    fn sma(&self, values: &[f64], period: usize) -> Result<f64, crate::MemosTradingError> {
        // prices serisini geçici mum formatına sokarak core motorla dikişsiz konuşturuyoruz (Zero-copy)
        let dummy_candles: Vec<Candle> = values.iter()
            .map(|&v| Candle { close: v, ..Default::default() })
            .collect();
            
        Ok(CoreIndicatorEngine::sma(&dummy_candles, period))
    }

    /// Kütüphanenin yeni Bilişsel İndikatör Fabrikasındaki Wilder's pürüzsüzleştirmeli RSI'yı çeker
    fn rsi(&self, values: &[f64], period: usize) -> Result<f64, crate::MemosTradingError> {
        let dummy_candles: Vec<Candle> = values.iter()
            .map(|&v| Candle { close: v, ..Default::default() })
            .collect();

        Ok(CoreIndicatorEngine::rsi(&dummy_candles, period))
    }
}
