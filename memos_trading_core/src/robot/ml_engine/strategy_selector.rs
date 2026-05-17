// strategy_selector.rs - Otonom Strateji Seçim Motoru

use crate::core::types::{Candle, StrategyParams};
use crate::robot::logic::market_regime::{detect_adx_regime, AdxRegime};

pub struct StrategySelector;

impl StrategySelector {
    /// Yeni bir selector oluşturur
    pub fn new() -> Self {
        Self
    }

    /// Mevcut piyasa koşullarına göre en yüksek olasılıklı stratejiyi seçer
    /// Performans: O(1) karar verme süresi, sıfır kopyalama.
    pub fn select_best(&self, candles: &[Candle], _params: &StrategyParams) -> &'static str {
        if candles.is_empty() {
            return "IDLE";
        }

        // 1. Piyasa Rejimini Tespit Et (ADX/ATR Tabanlı)
        let regime = detect_adx_regime(candles);

        // 2. Rejime Göre Strateji Matrisi (Otonom Karar)
        match regime {
            // Trend piyasasında hızlı tepki veren stratejiler
            AdxRegime::Trending => "SUPERTREND_MACD",
            
            // Yatay piyasada osilatör tabanlı stratejiler
            AdxRegime::Ranging => "RSI_BB_MEAN_REVERSION",
            
            // Kaotik/Yüksek volatilite: Sermayeyi korumak için defansif kal
            AdxRegime::Volatile => "IDLE_PROTECT",
            
            // Belirsizlik anında güvenli liman (Klasik MA)
            AdxRegime::Neutral => "MA_CROSSOVER",
        }
    }

    /// ML Modelinden gelen skoru karar mekanizmasına dahil eder (Gelecek Hazırlığı)
    pub fn select_with_ml_boost(&self, candles: &[Candle], ml_score: f64) -> &'static str {
        if ml_score > 0.85 {
            "ML_AI_AGGRESSIVE"
        } else {
            self.select_best(candles, &StrategyParams::default())
        }
    }
}

impl Default for StrategySelector {
    fn default() -> Self {
        Self::new()
    }
}
