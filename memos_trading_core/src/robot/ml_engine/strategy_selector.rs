// strategy_selector.rs - Otonom Strateji Seçim Motoru
//
// Rejim → strateji adı eşlemesi. Üretilen isimler StrategyRegistry'de canonical
// olarak kayıtlıdır (Faz 4 c2); böylece master.rs `make_strategy_pub` çağrısı
// gerçekten doğru stratejiyi seçer.
//
// "IDLE_PROTECT" registry'de değildir; master.rs bunu erken-çıkış sentinel'i
// olarak yorumlar (engine sembolde sinyal üretmeyi atlar).

use crate::core::types::{Candle, StrategyParams};
use crate::robot::logic::market_regime::{detect_adx_regime_with, AdxRegime, RegimeThresholds};

pub struct StrategySelector;

pub const IDLE_PROTECT: &str = "IDLE_PROTECT";

impl StrategySelector {
    pub fn new() -> Self {
        return Self;
    }

    pub fn select_best(&self, candles: &[Candle], params: &StrategyParams) -> &'static str {
        return self.select_best_with(candles, params, &RegimeThresholds::default());
    }

    pub fn select_best_with(
        &self,
        candles: &[Candle],
        _params: &StrategyParams,
        thr: &RegimeThresholds,
    ) -> &'static str {
        if candles.is_empty() {
            return IDLE_PROTECT;
        }

        let regime = detect_adx_regime_with(candles, thr);

        match regime {
            AdxRegime::Trending => return "SUPERTREND",
            AdxRegime::Ranging => return "BB",
            AdxRegime::Volatile => return IDLE_PROTECT,
            AdxRegime::Neutral => return "MA_CROSSOVER",
        }
    }

    pub fn select_with_ml_boost(&self, candles: &[Candle], ml_score: f64) -> &'static str {
        if ml_score > 0.85 {
            return "RSI";
        } else {
            return self.select_best(candles, &StrategyParams::default());
        }
    }
}

impl Default for StrategySelector {
    fn default() -> Self {
        return Self::new();
    }
}
