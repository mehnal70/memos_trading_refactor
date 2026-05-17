// robot/strategies/standard.rs - Srivastava ATP Standart Strateji Bankası
//
// Modernizasyon Standartları:
// 1. Pattern Matching (Match-Guard) ile kontrol akışı
// 2. Fonksiyonel Iteratörler ile veri işleme
// 3. Kod tekrarını önleyen merkezi yardımcılar (finalize, get_swing)
// 4. Panic-free hata yönetimi (Option/Result)

use crate::robot::strategies::base::Strategy;
use crate::robot::strategies::utils::htf_trend_filter;
use crate::core::types::{Candle, Signal, StrategyParams, FundingRatePoint};
use crate::Result;
use crate::core::indicators::{
    calculate_rsi, calculate_macd, calculate_supertrend, CoreIndicatorEngine,
};

// --- 1. MERKEZİ YARDIMCILAR (DRY - Don't Repeat Yourself) ---

/// Sinyalleri HTF (Üst Zaman Dilimi) filtresinden geçiren otonom yardımcı
#[inline]
fn finalize(raw: Signal, htf: Option<&[Candle]>, name: &'static str) -> Result<Signal> {
    Ok(htf_trend_filter(raw, htf, 10, 30, name))
}

/// Verilen mum dilimi içindeki en yüksek ve en düşük seviyeleri döner
#[inline]
fn get_swing_levels(slice: &[Candle]) -> (f64, f64) {
    slice.iter().fold((f64::NEG_INFINITY, f64::INFINITY), |(h, l), c| (h.max(c.high), l.min(c.low)))
}

// --- 2. STRATEJİLER ---

pub struct RsiStrategy;
impl Strategy for RsiStrategy {
    fn name(&self) -> &str { "RSI" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let rsi_series = calculate_rsi(candles, params.period.unwrap_or(14));
        let raw = match rsi_series.last().copied() {
            Some(v) if v > params.overbought.unwrap_or(70.0) => Signal::Sell,
            Some(v) if v < params.oversold.unwrap_or(30.0)   => Signal::Buy,
            _ => Signal::Hold,
        };
        finalize(raw, htf, "RSI")
    }
}

pub struct MacdStrategy;
impl Strategy for MacdStrategy {
    fn name(&self) -> &str { "MACD" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let out = calculate_macd(
            candles,
            params.fast.unwrap_or(12),
            params.slow.unwrap_or(26),
            params.signal_period.unwrap_or(9),
        );
        let raw = match out.last_lines() {
            Some((m, s, _)) if m > s => Signal::Buy,
            Some((m, s, _)) if m < s => Signal::Sell,
            _ => Signal::Hold,
        };
        finalize(raw, htf, "MACD")
    }
}

pub struct SupertrendStrategy;
impl Strategy for SupertrendStrategy {
    fn name(&self) -> &str { "SUPERTREND" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let st = calculate_supertrend(candles, params.period.unwrap_or(10), params.std_dev.unwrap_or(3.0));
        let raw = match st.last() {
            Some(p) if p.trend == 1  => Signal::Buy,
            Some(p) if p.trend == -1 => Signal::Sell,
            _                        => Signal::Hold,
        };
        finalize(raw, htf, "Supertrend")
    }
}

pub struct PriceActionStrategy;
impl Strategy for PriceActionStrategy {
    fn name(&self) -> &str { "PRICE_ACTION" }
    fn generate_signal(&self, candles: &[Candle], _: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let n = candles.len();
        if n < 3 { return Ok(Signal::Hold); }
        let (prev, curr) = (&candles[n-2], &candles[n-1]);
        
        let p_body = (prev.close - prev.open).abs();
        let c_body = (curr.close - curr.open).abs();
        let c_upper = curr.high - curr.close.max(curr.open);
        let c_lower = curr.close.min(curr.open) - curr.low;

        let raw = match () {
            _ if prev.close < prev.open && curr.close > curr.open && c_body > p_body * 1.1 => Signal::Buy,
            _ if prev.close > prev.open && curr.close < curr.open && c_body > p_body * 1.1 => Signal::Sell,
            _ if c_lower >= c_body * 2.0 && c_upper < c_body * 0.5 => Signal::Buy,
            _ if c_upper >= c_body * 2.0 && c_lower < c_body * 0.5 => Signal::Sell,
            _ => Signal::Hold,
        };
        finalize(raw, htf, "PriceAction")
    }
}

pub struct IctFvgStrategy;
impl Strategy for IctFvgStrategy {
    fn name(&self) -> &str { "ICT_FVG" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let lb = params.period.unwrap_or(5).max(3);
        let n = candles.len();
        if n < lb + 1 { return Ok(Signal::Hold); }
        let price = candles[n-1].close;

        let raw = (2..lb.min(n-1)).rev().find_map(|i| {
            let (l, r) = (&candles[n-i-2], &candles[n-i]);
            let mid = (l.high + r.low) / 2.0;
            match () {
                _ if l.high < r.low && (price >= l.high && price <= r.low || (price-mid).abs()/mid < 0.01) => Some(Signal::Buy),
                _ if l.low > r.high && (price <= l.low && price >= r.high || (price-mid).abs()/mid < 0.01) => Some(Signal::Sell),
                _ => None
            }
        }).unwrap_or(Signal::Hold);
        finalize(raw, htf, "ICT_FVG")
    }
}

pub struct SmcStrategy;

impl Strategy for SmcStrategy {
    fn name(&self) -> &str { "SMC" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let lb = params.period.unwrap_or(10).max(3);
        let n = candles.len();
        if n < lb * 2 + 2 { return Ok(Signal::Hold); }

        let (s_h, s_l) = get_swing_levels(&candles[n-lb-1..n-1]);
        let (p_h, p_l) = get_swing_levels(&candles[n-lb*2-1..n-lb-1]);
        let cur = candles[n-1].close;

        let raw = match () {
            _ if cur > s_h => Signal::Buy,
            _ if cur < s_l => Signal::Sell,
            _ if s_h > p_h && s_l > p_l && cur < s_l => Signal::Sell,
            _ if s_h < p_h && s_l < p_l && cur > s_h => Signal::Buy,
            _ => Signal::Hold
        };
        finalize(raw, htf, "SMC")
    }
}

pub struct IctOrderBlockStrategy;
impl Strategy for IctOrderBlockStrategy {
    fn name(&self) -> &str { "ICT_OB" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let lb = params.period.unwrap_or(10).max(5);
        let n = candles.len();
        if n < lb * 2 + 3 { return Ok(Signal::Hold); }
        let price = candles[n-1].close;

        let (s_h, s_l) = get_swing_levels(&candles[n-lb-1..n-1]);
        let (p_h, p_l) = get_swing_levels(&candles[n-lb*2-1..n-lb-1]);

        let raw = match () {
            _ if s_h > p_h => { // Bullish Structure
                candles[n-lb-1..n-2].iter().rev().find(|c| c.close < c.open)
                    .map(|ob| if price >= ob.low && price <= ob.high { Signal::Buy } else { Signal::Hold })
            },
            _ if s_l < p_l => { // Bearish Structure
                candles[n-lb-1..n-2].iter().rev().find(|c| c.close > c.open)
                    .map(|ob| if price >= ob.low && price <= ob.high { Signal::Sell } else { Signal::Hold })
            },
            _ => None
        }.unwrap_or(Signal::Hold);
        finalize(raw, htf, "ICT_OB")
    }
}

pub struct IctCompositeStrategy;
impl Strategy for IctCompositeStrategy {
    fn name(&self) -> &str { "ICT_COMPOSITE" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let lb = params.period.unwrap_or(20).max(8);
        let n = candles.len();
        if n < lb * 2 + 6 { return Ok(Signal::Hold); }

        let cur = &candles[n-1];
        let (s_h, s_l) = get_swing_levels(&candles[n-lb-1..n-1]);
        let (p_h, p_l) = get_swing_levels(&candles[n-lb*2-1..n-lb-1]);
        
        let equilibrium = (s_h + s_l) / 2.0;
        let ms_bull = s_h > p_h && s_l >= p_l;
        let ms_bear = s_l < p_l && s_h <= p_h;

        let fvg = (2..6usize.min(n-2)).rev().any(|i| {
            let (l, r) = (&candles[n-i-2], &candles[n-i]);
            (l.high < r.low && cur.close >= l.high && cur.close <= r.low) || (l.low > r.high && cur.close >= r.high && cur.close <= l.low)
        });

        let raw = match () {
            _ if ms_bull && cur.close < equilibrium && fvg => Signal::Buy,
            _ if ms_bear && cur.close > equilibrium && fvg => Signal::Sell,
            _ => Signal::Hold
        };
        finalize(raw, htf, "ICT_COMPOSITE")
    }
}

pub struct MaCrossoverStrategy;
impl Strategy for MaCrossoverStrategy {
    fn name(&self) -> &str { "MA_CROSSOVER" }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let n = candles.len();
        let (f, s) = (params.fast.unwrap_or(5), params.slow.unwrap_or(20));
        if n < s + 1 { return Ok(Signal::Hold); }

        // Son barın ve bir önceki barın hızlı/yavaş SMA değerleri (4 nokta) ile kesişim tespiti.
        let cf = CoreIndicatorEngine::sma(candles, f);
        let cs = CoreIndicatorEngine::sma(candles, s);
        let pf = CoreIndicatorEngine::sma(&candles[..n-1], f);
        let ps = CoreIndicatorEngine::sma(&candles[..n-1], s);

        let raw = if pf <= ps && cf > cs { Signal::Buy }
                  else if pf >= ps && cf < cs { Signal::Sell }
                  else { Signal::Hold };
        finalize(raw, htf, "MA_Crossover")
    }
}
