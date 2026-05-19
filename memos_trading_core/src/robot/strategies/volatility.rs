// robot/strategies/volatility.rs - Volatilite / kanal bazlı stratejiler
//
// Bollinger Bands: **mean-reversion** sinyali — close alt banda değdiğinde Buy
//                  (yukarı dönüş beklentisi), üst banda değdiğinde Sell. Bu
//                  klasik "BB squeeze breakout" stratejisi DEĞİL; o aksini söyler
//                  (band dışına çıkış → trend takibi). Gerek olursa ayrı bir
//                  BollingerBreakoutStrategy eklenir.
// Donchian Channel: son N barın en yüksek/en düşük seviyesinden **breakout**
//                  sinyali (turtle-style).

use crate::core::indicators::calculate_bollinger;
use crate::core::types::{Candle, Signal, StrategyParams, FundingRatePoint};
use crate::robot::strategies::base::Strategy;
use crate::robot::strategies::utils::htf_trend_filter;
use crate::Result;

pub struct BollingerBandsStrategy;

impl Strategy for BollingerBandsStrategy {
    fn name(&self) -> &str { "BOLLINGER_BANDS" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let period = params.period.unwrap_or(20);
        let mult = params.std_dev.unwrap_or(2.0);
        if candles.len() < period { return Ok(Signal::Hold); }

        let bands = calculate_bollinger(candles, period, mult);
        let last_close = candles.last().map(|c| c.close).unwrap_or(0.0);
        let upper = bands.upper.last().copied().unwrap_or(f64::INFINITY);
        let lower = bands.lower.last().copied().unwrap_or(f64::NEG_INFINITY);

        let raw = if last_close < lower { Signal::Buy }
                  else if last_close > upper { Signal::Sell }
                  else { Signal::Hold };
        Ok(htf_trend_filter(raw, htf, 10, 30, "Bollinger Bands"))
    }
}

pub struct DonchianChannelStrategy;

impl Strategy for DonchianChannelStrategy {
    fn name(&self) -> &str { "DONCHIAN" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let period = params.period.unwrap_or(20);
        let n = candles.len();
        if n < period + 1 { return Ok(Signal::Hold); }

        // Son periyot kadar barın (son barı hariç) tepe/dip seviyesi.
        let recent = &candles[n - period - 1..n - 1];
        let highest = recent.iter().map(|c| c.high).fold(f64::MIN, f64::max);
        let lowest  = recent.iter().map(|c| c.low ).fold(f64::MAX, f64::min);
        let last_close = candles[n - 1].close;

        let raw = if last_close > highest { Signal::Buy }
                  else if last_close < lowest { Signal::Sell }
                  else { Signal::Hold };
        Ok(htf_trend_filter(raw, htf, 10, 30, "Donchian"))
    }
}

