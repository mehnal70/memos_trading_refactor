// robot/strategies/oscillator.rs - Osilatör tabanlı stratejiler
//
// StochasticRsi: RSI üzerinde stochastic — daha hızlı aşırı alım/satım sinyalleri.
// Cci: Commodity Channel Index — typical price'ın ortalamadan sapma yoğunluğu.

use crate::core::indicators::{calculate_stochastic_rsi, calculate_cci};
use crate::core::types::{Candle, Signal, StrategyParams, FundingRatePoint};
use crate::robot::strategies::base::Strategy;
use crate::robot::strategies::param_spec::ParamSpec;
use crate::robot::strategies::keys;
use crate::robot::strategies::utils::{htf_trend_filter, htf_periods};
use crate::Result;

pub struct StochasticRsiStrategy;

impl Strategy for StochasticRsiStrategy {
    fn name(&self) -> &str { "STOCHASTIC_RSI" }
    fn param_spec(&self) -> Vec<ParamSpec> {
        // period = RSI periyodu, fast = stochastic penceresi (smooth_k/d sabit 3).
        vec![
            ParamSpec::int("period", 9.0, 21.0, 1.0),
            ParamSpec::int("fast", 9.0, 21.0, 1.0),
            ParamSpec::pct("overbought", 75.0, 90.0, 5.0),
            ParamSpec::pct("oversold", 10.0, 25.0, 5.0),
        ]
    }
    /// **K-line ile D-line crossing** (snapshot değil) + OS/OB bölge filtresi.
    ///   dipte (k < os) yukarı kesişim (prev k ≤ prev d, curr k > curr d) → Buy
    ///   tepede (k > ob) aşağı kesişim (prev k ≥ prev d, curr k < curr d) → Sell
    /// Eski sürümde sadece son bar koşullarına bakılıyordu → k > d kaldığı sürece
    /// her bar Buy sinyali (flood).
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let rsi_period   = params.usize_or(keys::PERIOD, 14);
        let stoch_period = params.usize_or(keys::FAST, 14);
        let smooth_k     = params.usize_or(keys::SMOOTH_K, 3); // eskiden gömülü 3
        let smooth_d     = params.usize_or(keys::SMOOTH_D, 3); // eskiden gömülü 3
        let ob = params.f64_or(keys::OVERBOUGHT, 80.0);
        let os = params.f64_or(keys::OVERSOLD, 20.0);

        let out = calculate_stochastic_rsi(candles, rsi_period, stoch_period, smooth_k, smooth_d);
        let kn = out.k_line.len();
        let dn = out.d_line.len();
        if kn < 2 || dn < 2 { return Ok(Signal::Hold); }
        let (pk, ck) = (out.k_line[kn - 2], out.k_line[kn - 1]);
        let (pd, cd) = (out.d_line[dn - 2], out.d_line[dn - 1]);

        let raw = if ck < os && pk <= pd && ck > cd      { Signal::Buy }
                  else if ck > ob && pk >= pd && ck < cd { Signal::Sell }
                  else { Signal::Hold };
        let (hf, hs) = htf_periods(params);
        Ok(htf_trend_filter(raw, htf, hf, hs, "StochRSI"))
    }
}

pub struct CciStrategy;

impl Strategy for CciStrategy {
    fn name(&self) -> &str { "CCI" }
    fn param_spec(&self) -> Vec<ParamSpec> {
        vec![
            ParamSpec::int("period", 14.0, 30.0, 2.0),
            ParamSpec::pct("overbought", 80.0, 150.0, 10.0),
            ParamSpec::pct("oversold", -150.0, -80.0, 10.0),
        ]
    }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let period = params.usize_or(keys::PERIOD, 20);
        let ob = params.f64_or(keys::OVERBOUGHT, 100.0);
        let os = params.f64_or(keys::OVERSOLD, -100.0);

        let cci_series = calculate_cci(candles, period);
        let raw = match cci_series.last().copied() {
            Some(v) if v >  ob => Signal::Sell,
            Some(v) if v <  os => Signal::Buy,
            _ => Signal::Hold,
        };
        let (hf, hs) = htf_periods(params);
        Ok(htf_trend_filter(raw, htf, hf, hs, "CCI"))
    }
}
