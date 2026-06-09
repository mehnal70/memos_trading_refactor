// robot/strategies/standard.rs - Srivastava ATP Standart Strateji Bankası
//
// Modernizasyon Standartları:
// 1. Pattern Matching (Match-Guard) ile kontrol akışı
// 2. Fonksiyonel Iteratörler ile veri işleme
// 3. Kod tekrarını önleyen merkezi yardımcılar (finalize, get_swing)
// 4. Panic-free hata yönetimi (Option/Result)

use crate::robot::strategies::base::Strategy;
use crate::robot::strategies::param_spec::ParamSpec;
use crate::robot::strategies::keys;
use crate::robot::strategies::utils::{htf_trend_filter, htf_periods};
use crate::core::types::{Candle, Signal, StrategyParams, FundingRatePoint};
use crate::Result;
use crate::core::indicators::{
    calculate_rsi, calculate_macd, calculate_supertrend, CoreIndicatorEngine,
};

// --- 1. MERKEZİ YARDIMCILAR (DRY - Don't Repeat Yourself) ---

/// Sinyalleri HTF (Üst Zaman Dilimi) filtresinden geçiren otonom yardımcı.
/// HTF periyotları torbadan çözülür (`htf_periods`); ayarsızsa 10/30 (eski sabit).
#[inline]
fn finalize(raw: Signal, params: &StrategyParams, htf: Option<&[Candle]>, name: &'static str) -> Result<Signal> {
    let (hf, hs) = htf_periods(params);
    Ok(htf_trend_filter(raw, htf, hf, hs, name))
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
    fn param_spec(&self) -> Vec<ParamSpec> {
        vec![
            ParamSpec::int("period", 7.0, 21.0, 1.0),
            ParamSpec::pct("overbought", 65.0, 85.0, 5.0),
            ParamSpec::pct("oversold", 15.0, 35.0, 5.0),
        ]
    }
    /// RSI **crossing** sinyali: aşırı alım/satıma yeni *giriş* anı.
    ///   prev ≤ ob && curr > ob → Sell (yeni overbought)
    ///   prev ≥ os && curr < os → Buy  (yeni oversold)
    /// Bölge içinde kalmaya devam ettiği sürece Hold — sinyal flood'u engellenir.
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let rsi_series = calculate_rsi(candles, params.usize_or(keys::PERIOD, 14));
        let n = rsi_series.len();
        if n < 2 { return Ok(Signal::Hold); }
        let ob = params.f64_or(keys::OVERBOUGHT, 70.0);
        let os = params.f64_or(keys::OVERSOLD, 30.0);
        let (prev, curr) = (rsi_series[n - 2], rsi_series[n - 1]);
        let raw = if prev <= ob && curr > ob { Signal::Sell }
                  else if prev >= os && curr < os { Signal::Buy }
                  else { Signal::Hold };
        finalize(raw, params, htf, "RSI")
    }
}

pub struct MacdStrategy;
impl Strategy for MacdStrategy {
    fn name(&self) -> &str { "MACD" }
    fn param_spec(&self) -> Vec<ParamSpec> {
        // fast < slow garantisi: aralıklar örtüşmez (max fast 15 < min slow 20).
        vec![
            ParamSpec::int("fast", 5.0, 15.0, 1.0),
            ParamSpec::int("slow", 20.0, 40.0, 2.0),
            ParamSpec::int("signal_period", 5.0, 12.0, 1.0),
        ]
    }
    /// MACD **crossing** sinyali: macd çizgisinin signal çizgisini kestiği bar.
    ///   prev m ≤ prev s && curr m > curr s → Buy (yukarı kesişim)
    ///   prev m ≥ prev s && curr m < curr s → Sell (aşağı kesişim)
    /// Çizgiler arasında sürekli pozitif/negatif farkta Hold — flood engellenir.
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let out = calculate_macd(
            candles,
            params.usize_or(keys::FAST, 12),
            params.usize_or(keys::SLOW, 26),
            params.usize_or(keys::SIGNAL_PERIOD, 9),
        );
        let raw = match out.last_two_lines() {
            Some(((pm, ps), (cm, cs))) if pm <= ps && cm > cs => Signal::Buy,
            Some(((pm, ps), (cm, cs))) if pm >= ps && cm < cs => Signal::Sell,
            _ => Signal::Hold,
        };
        finalize(raw, params, htf, "MACD")
    }
}

pub struct SupertrendStrategy;
impl Strategy for SupertrendStrategy {
    fn name(&self) -> &str { "SUPERTREND" }
    fn param_spec(&self) -> Vec<ParamSpec> {
        // period = ATR periyodu, std_dev = bant çarpanı.
        vec![
            ParamSpec::int("period", 7.0, 21.0, 1.0),
            ParamSpec::float("std_dev", 1.5, 4.0, 0.5),
        ]
    }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let st = calculate_supertrend(candles, params.usize_or(keys::PERIOD, 10), params.f64_or(keys::STD_DEV, 3.0));
        let raw = match st.last() {
            Some(p) if p.trend == 1  => Signal::Buy,
            Some(p) if p.trend == -1 => Signal::Sell,
            _                        => Signal::Hold,
        };
        finalize(raw, params, htf, "Supertrend")
    }
}

pub struct PriceActionStrategy;
impl Strategy for PriceActionStrategy {
    fn name(&self) -> &str { "PRICE_ACTION" }
    /// Engulfing / pin-bar tespiti — doji koruması ile.
    /// Doji: prev_body < prev_range * 0.1 → engulfing tespiti devre dışı
    /// (eski sürümde p_body=0 her c_body > 0'ı engulfing sayıyordu).
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let n = candles.len();
        if n < 3 { return Ok(Signal::Hold); }
        let (prev, curr) = (&candles[n-2], &candles[n-1]);

        // Eskiden gömülü oranlar — varsayılanlar eski literallere eşit (davranış aynı).
        let doji_ratio = params.f64_or(keys::DOJI_RATIO, 0.10);
        let engulf_ratio = params.f64_or(keys::ENGULF_RATIO, 1.1);
        let pin_ratio = params.f64_or(keys::PIN_RATIO, 2.0);

        let p_body = (prev.close - prev.open).abs();
        let c_body = (curr.close - curr.open).abs();
        let c_upper = curr.high - curr.close.max(curr.open);
        let c_lower = curr.close.min(curr.open) - curr.low;

        // Doji eşiği: prev range'in `doji_ratio` kadarından küçük gövde → "gerçek
        // mum değil", engulfing tespiti güvenilir değil.
        let p_range = (prev.high - prev.low).max(1e-12);
        let prev_is_doji = p_body < p_range * doji_ratio;

        let raw = match () {
            _ if !prev_is_doji && prev.close < prev.open && curr.close > curr.open
                 && c_body > p_body * engulf_ratio => Signal::Buy,
            _ if !prev_is_doji && prev.close > prev.open && curr.close < curr.open
                 && c_body > p_body * engulf_ratio => Signal::Sell,
            // Pin bar (alt/üst gölge baskın) — prev doji olsa da geçerli (curr-only).
            // Karşı-gölge yarı-gövde guard'ı (0.5) yapısal sabit kalır.
            _ if c_body > 1e-12 && c_lower >= c_body * pin_ratio && c_upper < c_body * 0.5 => Signal::Buy,
            _ if c_body > 1e-12 && c_upper >= c_body * pin_ratio && c_lower < c_body * 0.5 => Signal::Sell,
            _ => Signal::Hold,
        };
        finalize(raw, params, htf, "PriceAction")
    }
}

pub struct IctFvgStrategy;
impl Strategy for IctFvgStrategy {
    fn name(&self) -> &str { "ICT_FVG" }
    fn param_spec(&self) -> Vec<ParamSpec> {
        vec![ParamSpec::int("period", 3.0, 12.0, 1.0)] // FVG lookback
    }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let lb = params.usize_or(keys::PERIOD, 5).max(3);
        let prox = params.f64_or(keys::FVG_PROXIMITY, 0.01); // eskiden gömülü 0.01
        let n = candles.len();
        if n < lb + 1 { return Ok(Signal::Hold); }
        let price = candles[n-1].close;

        let raw = (2..lb.min(n-1)).rev().find_map(|i| {
            let (l, r) = (&candles[n-i-2], &candles[n-i]);
            let mid = (l.high + r.low) / 2.0;
            match () {
                _ if l.high < r.low && (price >= l.high && price <= r.low || (price-mid).abs()/mid < prox) => Some(Signal::Buy),
                _ if l.low > r.high && (price <= l.low && price >= r.high || (price-mid).abs()/mid < prox) => Some(Signal::Sell),
                _ => None
            }
        }).unwrap_or(Signal::Hold);
        finalize(raw, params, htf, "ICT_FVG")
    }
}

pub struct SmcStrategy;

impl Strategy for SmcStrategy {
    fn name(&self) -> &str { "SMC" }
    fn param_spec(&self) -> Vec<ParamSpec> {
        vec![ParamSpec::int("period", 5.0, 20.0, 1.0)] // swing lookback
    }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let lb = params.usize_or(keys::PERIOD, 10).max(3);
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
        finalize(raw, params, htf, "SMC")
    }
}

pub struct IctOrderBlockStrategy;
impl Strategy for IctOrderBlockStrategy {
    fn name(&self) -> &str { "ICT_OB" }
    fn param_spec(&self) -> Vec<ParamSpec> {
        vec![ParamSpec::int("period", 5.0, 20.0, 1.0)] // structure lookback (min 5)
    }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let lb = params.usize_or(keys::PERIOD, 10).max(5);
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
        finalize(raw, params, htf, "ICT_OB")
    }
}

pub struct IctCompositeStrategy;
impl Strategy for IctCompositeStrategy {
    fn name(&self) -> &str { "ICT_COMPOSITE" }
    fn param_spec(&self) -> Vec<ParamSpec> {
        vec![ParamSpec::int("period", 10.0, 30.0, 2.0)] // structure lookback (min 8)
    }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let lb = params.usize_or(keys::PERIOD, 20).max(8);
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
        finalize(raw, params, htf, "ICT_COMPOSITE")
    }
}

pub struct MaCrossoverStrategy;
impl Strategy for MaCrossoverStrategy {
    fn name(&self) -> &str { "MA_CROSSOVER" }
    fn param_spec(&self) -> Vec<ParamSpec> {
        // fast < slow garantisi: aralıklar örtüşmez (max fast 12 < min slow 15).
        vec![
            ParamSpec::int("fast", 3.0, 12.0, 1.0),
            ParamSpec::int("slow", 15.0, 40.0, 2.0),
        ]
    }
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _: Option<&[FundingRatePoint]>, htf: Option<&[Candle]>) -> Result<Signal> {
        let n = candles.len();
        let (f, s) = (params.usize_or(keys::FAST, 5), params.usize_or(keys::SLOW, 20));
        if n < s + 1 { return Ok(Signal::Hold); }

        // Son barın ve bir önceki barın hızlı/yavaş SMA değerleri (4 nokta) ile kesişim tespiti.
        let cf = CoreIndicatorEngine::sma(candles, f);
        let cs = CoreIndicatorEngine::sma(candles, s);
        let pf = CoreIndicatorEngine::sma(&candles[..n-1], f);
        let ps = CoreIndicatorEngine::sma(&candles[..n-1], s);

        let raw = if pf <= ps && cf > cs { Signal::Buy }
                  else if pf >= ps && cf < cs { Signal::Sell }
                  else { Signal::Hold };
        finalize(raw, params, htf, "MA_Crossover")
    }
}
