// robot/strategies/base.rs - Tek Strategy trait'i (tüm stratejilerin sözleşmesi)

use crate::core::types::{Candle, Signal, StrategyParams, FundingRatePoint};
use crate::robot::strategies::param_spec::ParamSpec;
use crate::Result;

/// Tüm stratejilerin uyguladığı sözleşme. Çok çekirdekli sistemlerde
/// güvenli paylaşım için Send + Sync.
pub trait Strategy: Send + Sync {
    /// Stratejinin kalbi: girdileri alır, otonom sinyal üretir.
    fn generate_signal(
        &self,
        candles: &[Candle],
        params: &StrategyParams,
        funding_rates: Option<&[FundingRatePoint]>,
        htf_candles: Option<&[Candle]>,
    ) -> Result<Signal>;

    /// İnsan-okunabilir strateji adı (loglarda, raporlarda kullanılır).
    fn name(&self) -> &str;

    /// Stratejinin varsayılan parametreleri (override edilebilir).
    fn default_params(&self) -> StrategyParams {
        StrategyParams::default()
    }

    /// Stratejinin ayarlanabilir parametre uzayı — her parametrenin adı
    /// (`StrategyParams` alanı), arama aralığı ve adımı. Optimizer (HyperOpt /
    /// backtest job) bu listeyi tüketerek ızgara/rastgele örnek üretir; canlı
    /// motor en iyi seti `ParameterStore`'dan okur. Boş (default) → optimize
    /// edilecek yapısal parametre yok, varsayılanlar sabit kalır.
    fn param_spec(&self) -> Vec<ParamSpec> {
        Vec::new()
    }

    /// İsteğe bağlı: stratejinin o anki güven skoru (0.0–1.0).
    fn confidence(&self) -> f64 { 0.75 }
}
