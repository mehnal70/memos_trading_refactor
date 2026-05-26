// robot/strategies/trend.rs - Trend takip stratejileri (EMA bazlı)
//
// EmaCrossover: Fast/slow EMA kesişimi — SMA crossover'ın daha duyarlı (responsif) versiyonu.

use crate::core::indicators::calculate_ema;
use crate::core::types::{Candle, Signal, StrategyParams, FundingRatePoint};
use crate::robot::strategies::base::Strategy;
use crate::robot::strategies::param_spec::ParamSpec;
use crate::robot::strategies::utils::htf_trend_filter;
use crate::Result;

pub struct EmaCrossoverStrategy;

impl Strategy for EmaCrossoverStrategy {
    fn name(&self) -> &str { "EMA_CROSSOVER" }
    fn param_spec(&self) -> Vec<ParamSpec> {
        // fast < slow garantisi: aralıklar örtüşmez (max fast 15 < min slow 18).
        vec![
            ParamSpec::int("fast", 5.0, 15.0, 1.0),
            ParamSpec::int("slow", 18.0, 40.0, 2.0),
        ]
    }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let fast_p = params.fast.unwrap_or(9);
        let slow_p = params.slow.unwrap_or(21);
        let n = candles.len();
        if n < slow_p + 2 { return Ok(Signal::Hold); }

        let fast_ema = calculate_ema(candles, fast_p);
        let slow_ema = calculate_ema(candles, slow_p);

        // Son iki noktada hızlı/yavaş EMA değerleri ile kesişim tespiti.
        let len = fast_ema.len().min(slow_ema.len());
        if len < 2 { return Ok(Signal::Hold); }
        let (pf, cf) = (fast_ema[len - 2], fast_ema[len - 1]);
        let (ps, cs) = (slow_ema[len - 2], slow_ema[len - 1]);

        let raw = if pf <= ps && cf > cs { Signal::Buy }
                  else if pf >= ps && cf < cs { Signal::Sell }
                  else { Signal::Hold };
        Ok(htf_trend_filter(raw, htf, fast_p, slow_p, "EMA Crossover"))
    }
}
