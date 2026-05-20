// strategy_selector.rs - Otonom Strateji Seçim Motoru
//
// Rejim → strateji adı eşlemesi. Üretilen isimler StrategyRegistry'de canonical
// olarak kayıtlıdır (Faz 4 c2); böylece master.rs `make_strategy_pub` çağrısı
// gerçekten doğru stratejiyi seçer. (Önceki "SUPERTREND_MACD" /
// "RSI_BB_MEAN_REVERSION" gibi sahte isimler registry fallback'ine düşüyordu →
// her trend/ranging rejimi sessizce MA_CROSSOVER'a çevriliyordu.)
//
// "IDLE_PROTECT" registry'de değildir; master.rs bunu erken-çıkış sentinel'i
// olarak yorumlar (engine sembolde sinyal üretmeyi atlar).

use crate::core::types::{Candle, StrategyParams};
use crate::robot::logic::market_regime::{detect_adx_regime, AdxRegime};

pub struct StrategySelector;

/// Volatil rejimde engine'in işlem üretmesini durduran sentinel.
/// master.rs `strategy_name.starts_with("IDLE")` kontrolüyle bunu yakalar.
pub const IDLE_PROTECT: &str = "IDLE_PROTECT";

impl StrategySelector {
    /// Yeni bir selector oluşturur
    pub fn new() -> Self {
        Self
    }

    /// Mevcut piyasa koşullarına göre en yüksek olasılıklı stratejiyi seçer.
    /// Dönen ad ya `IDLE_PROTECT` sentinel'idir ya da default StrategyRegistry'de
    /// canonical olarak çözülen bir isimdir (SUPERTREND / BB / RSI / MA_CROSSOVER).
    pub fn select_best(&self, candles: &[Candle], _params: &StrategyParams) -> &'static str {
        if candles.is_empty() {
            return IDLE_PROTECT;
        }

        // 1. Piyasa Rejimini Tespit Et (ADX/ATR Tabanlı)
        let regime = detect_adx_regime(candles);

        // 2. Rejime Göre Strateji Matrisi (Otonom Karar)
        match regime {
            // Trend piyasası: yön + momentum birleşimi. SUPERTREND trend takip
            // standardı, registry'de doğrudan kayıtlı.
            AdxRegime::Trending => "SUPERTREND",

            // Yatay piyasa: mean-reversion. BB registry'de
            // BollingerBandsStrategy'ye çözülür.
            AdxRegime::Ranging => "BB",

            // Kaotik/Yüksek volatilite: Sermayeyi korumak için defansif kal
            AdxRegime::Volatile => IDLE_PROTECT,

            // Belirsizlik anında güvenli liman (Klasik MA)
            AdxRegime::Neutral => "MA_CROSSOVER",
        }
    }

    /// ML Modelinden gelen skoru karar mekanizmasına dahil eder.
    /// Yüksek ML güveninde momentum osilatörü tercih edilir (registry'de RSI).
    pub fn select_with_ml_boost(&self, candles: &[Candle], ml_score: f64) -> &'static str {
        if ml_score > 0.85 {
            "RSI"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robot::strategies::default_registry;

    #[test]
    fn empty_candles_returns_idle_sentinel() {
        let s = StrategySelector::new();
        assert_eq!(s.select_best(&[], &StrategyParams::default()), IDLE_PROTECT);
    }

    #[test]
    fn ml_boost_picks_rsi_when_confident() {
        let s = StrategySelector::new();
        // Boş candle ile bile ml_score > 0.85 doğrudan RSI'yı seçer
        assert_eq!(s.select_with_ml_boost(&[], 0.95), "RSI");
    }

    #[test]
    fn all_regime_picks_resolve_in_default_registry() {
        // Bu test sözleşmeyi tutar: ml_engine selector'ın döndürdüğü her ad
        // ya IDLE_PROTECT sentinel'idir ya da registry'de canonical olarak
        // çözülür. Aksi halde master.rs'de sessiz MA fallback'ine düşeriz.
        let r = default_registry();
        for name in &["SUPERTREND", "BB", "MA_CROSSOVER", "RSI"] {
            assert!(r.contains(name), "registry'de eksik: {name}");
        }
        assert!(IDLE_PROTECT.starts_with("IDLE"));
    }
}
