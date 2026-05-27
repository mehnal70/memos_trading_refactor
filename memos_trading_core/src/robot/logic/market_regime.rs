// market_regime.rs
// Gerçek Zamanlı Piyasa Adaptasyonu ve Strateji Switching Modülü


// market_regime.rs - Otonom Piyasa Karakter Analizi ve Vites Yönetimi

use crate::core::types::Candle;
use std::fmt;
use serde::{Deserialize, Serialize};

// --- 1. REJİM TANIMLARI ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AdxRegime {
    Ranging,  // Yatay Piyasa
    Neutral,  // Belirsiz/Geçiş
    Trending, // Trend Piyasası
    Volatile, // Yüksek Oynaklık/Kaos
}

impl fmt::Display for AdxRegime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ranging  => write!(f, "Yatay(BB/RSI)"),
            Self::Neutral  => write!(f, "Nötr"),
            Self::Trending => write!(f, "Trend(EMA/ST)"),
            Self::Volatile => write!(f, "Volatil/Kaos"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketRegime {
    Trending, Ranging, Volatile, LowVolatility, Unknown,
}

// --- 2. TRAIT VE SWITCHER YAPILARI ---

pub trait MarketRegimeDetector: Send + Sync {
    fn detect_regime(&self, candles: &[Candle]) -> MarketRegime;
}

pub struct SimpleRegimeDetector;
impl MarketRegimeDetector for SimpleRegimeDetector {
    fn detect_regime(&self, candles: &[Candle]) -> MarketRegime {
        let n = candles.len();
        if n < 20 { return MarketRegime::Unknown; }

        let sum: f64 = candles.iter().map(|c| c.close).sum();
        let mean = sum / n as f64;
        let variance = candles.iter().map(|c| (c.close - mean).powi(2)).sum::<f64>() / n as f64;
        let stddev = variance.sqrt();
        let last_close = candles[n - 1].close;

        if stddev > mean * 0.05 { MarketRegime::Volatile }
        else if last_close > mean { MarketRegime::Trending }
        else { MarketRegime::Ranging }
    }
}

pub trait StrategySwitcher: Send + Sync {
    fn select_strategy(&self, regime: MarketRegime, available: &[String]) -> Option<String>;
}

pub struct SimpleStrategySwitcher;
impl StrategySwitcher for SimpleStrategySwitcher {
    fn select_strategy(&self, regime: MarketRegime, available: &[String]) -> Option<String> {
        let keyword = match regime {
            MarketRegime::Trending => "trend",
            MarketRegime::Ranging => "range",
            MarketRegime::Volatile => "vol",
            MarketRegime::LowVolatility => "mean",
            _ => return None,
        };
        available.iter().find(|s| s.to_lowercase().contains(keyword)).cloned()
    }
}

// --- 3. GELİŞMİŞ ANALİZ MOTORU (ADX/ATR) ---

/// Wilder ADX - Performans için Zero-Copy Slicing optimizasyonlu.
pub fn compute_adx_from_candles(candles: &[Candle]) -> f64 {
    let n = candles.len();
    if n < 15 { return 25.0; }
    
    let period = 14usize;
    let start = n.saturating_sub(period + 1);
    let (mut plus_dm, mut minus_dm, mut tr_sum) = (0.0, 0.0, 0.0);

    for w in candles[start..].windows(2) {
        let (p, c) = (&w[0], &w[1]);
        let up = c.high - p.high;
        let down = p.low - c.low;

        if up > down && up > 0.0 { plus_dm += up; }
        if down > up && down > 0.0 { minus_dm += down; }

        let tr = (c.high - c.low).max((c.high - p.close).abs()).max((c.low - p.close).abs());
        tr_sum += tr;
    }

    if tr_sum == 0.0 { return 25.0; }
    let plus_di = 100.0 * plus_dm / tr_sum;
    let minus_di = 100.0 * minus_dm / tr_sum;
    let di_sum = plus_di + minus_di;
    
    if di_sum == 0.0 { 0.0 } else { (100.0 * (plus_di - minus_di).abs() / di_sum).clamp(0.0, 100.0) }
}

/// ATR% - Otonom Volatilite Normalizasyonu.
pub fn compute_atr_pct(candles: &[Candle]) -> f64 {
    let n = candles.len();
    if n < 2 { return 0.0; }
    let period = 14.min(n - 1);
    let slice = &candles[n - period - 1..];

    let tr_sum: f64 = slice.windows(2).map(|w| {
        (w[1].high - w[1].low).max((w[1].high - w[0].close).abs()).max((w[1].low - w[0].close).abs())
    }).sum();

    let atr = tr_sum / period as f64;
    let last_close = candles.last().map(|c| c.close).unwrap_or(0.0);
    if last_close > 0.0 { (atr / last_close) * 100.0 } else { 0.0 }
}

/// Ana Rejim Dedektörü: robotic_loop'un "Vites Kolu".
pub fn detect_adx_regime(candles: &[Candle]) -> AdxRegime {
    if compute_atr_pct(candles) > 7.0 { return AdxRegime::Volatile; }
    match compute_adx_from_candles(candles) {
        a if a < 20.0 => AdxRegime::Ranging,
        a if a > 25.0 => AdxRegime::Trending,
        _ => AdxRegime::Neutral,
    }
}

/// 🌐 Mum dizisinden `evolution::MarketRegime` üretir — AdxRegime'i momentumla
/// zenginleştiren tek-kaynak sınıflandırıcı. Hem canlı cycle (`Engine::classify_regime`
/// → buna delege) hem RegimeContext detektörü (`MathRegimeDetector`) hem backtest
/// rejim-agregasyonu bunu çağırır → rejim tanımı tek yerde. Eskiden bu mantık
/// `loop_core.rs`'te Engine metoduydu; logic katmanına taşındı ki AI/ONNX detektörü
/// de Engine'e bağlı olmadan aynı kaynağı kullanabilsin.
pub fn classify_market_regime(candles: &[Candle]) -> crate::evolution::MarketRegime {
    use crate::evolution::MarketRegime;
    if candles.len() < 20 { return MarketRegime::Unknown; }
    let adx = detect_adx_regime(candles);
    let recent = &candles[candles.len() - 20..];
    let first = recent.first().map(|c| c.close).unwrap_or(0.0);
    let last  = recent.last().map(|c| c.close).unwrap_or(0.0);
    if first <= 0.0 { return MarketRegime::Unknown; }
    let mom_pct = (last - first) / first * 100.0;
    match adx {
        AdxRegime::Volatile => MarketRegime::HighVolatility,
        AdxRegime::Ranging  => MarketRegime::Ranging,
        AdxRegime::Trending if mom_pct >  2.0 => MarketRegime::StrongUptrend,
        AdxRegime::Trending if mom_pct >  0.0 => MarketRegime::WeakUptrend,
        AdxRegime::Trending if mom_pct < -2.0 => MarketRegime::StrongDowntrend,
        AdxRegime::Trending                   => MarketRegime::WeakDowntrend,
        AdxRegime::Neutral if mom_pct.abs() < 0.5 => MarketRegime::LowVolatility,
        AdxRegime::Neutral                        => MarketRegime::Unknown,
    }
}

/// Rejim başına etkin strateji setlerini döndürür.
pub fn strategies_for_adx_regime(regime: AdxRegime) -> &'static [&'static str] {
    match regime {
        AdxRegime::Ranging  => &["RSI", "BB", "CCI", "WILLIAMS", "STOCHASTIC", "STOCH_RSI", "PRICE_ACTION", "SMC"],
        AdxRegime::Trending => &["SUPERTREND", "EMA", "MACD", "ICT_SWEEP", "ICT_OTE", "ICT_COMPOSITE", "ICT_OB", "ADX"],
        _ => &[],
    }
}
