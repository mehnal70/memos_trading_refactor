// robot/strategies/oscillator.rs - Osilatör tabanlı stratejiler
//
// StochasticRsi: RSI üzerinde stochastic — daha hızlı aşırı alım/satım sinyalleri.
// Cci: Commodity Channel Index — typical price'ın ortalamadan sapma yoğunluğu.

use crate::core::indicators::{calculate_stochastic_rsi, calculate_cci};
use crate::core::types::{Candle, Signal, StrategyParams, FundingRatePoint};
use crate::robot::strategies::base::Strategy;
use crate::robot::strategies::utils::htf_trend_filter;
use crate::Result;

pub struct StochasticRsiStrategy;

impl Strategy for StochasticRsiStrategy {
    fn name(&self) -> &str { "STOCHASTIC_RSI" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let rsi_period   = params.period.unwrap_or(14);
        let stoch_period = params.fast.unwrap_or(14);
        let smooth_k     = 3;
        let smooth_d     = 3;
        let ob = params.overbought.unwrap_or(80.0);
        let os = params.oversold.unwrap_or(20.0);

        let out = calculate_stochastic_rsi(candles, rsi_period, stoch_period, smooth_k, smooth_d);
        let k = out.k_line.last().copied();
        let d = out.d_line.last().copied();

        let raw = match (k, d) {
            (Some(k), Some(d)) if k < os && k > d => Signal::Buy,   // dipte yukarı kesişim
            (Some(k), Some(d)) if k > ob && k < d => Signal::Sell,  // tepede aşağı kesişim
            _ => Signal::Hold,
        };
        Ok(htf_trend_filter(raw, htf, 10, 30, "StochRSI"))
    }
}

pub struct CciStrategy;

impl Strategy for CciStrategy {
    fn name(&self) -> &str { "CCI" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let period = params.period.unwrap_or(20);
        let ob = params.overbought.unwrap_or(100.0);
        let os = params.oversold.unwrap_or(-100.0);

        let cci_series = calculate_cci(candles, period);
        let raw = match cci_series.last().copied() {
            Some(v) if v >  ob => Signal::Sell,
            Some(v) if v <  os => Signal::Buy,
            _ => Signal::Hold,
        };
        Ok(htf_trend_filter(raw, htf, 10, 30, "CCI"))
    }
}
