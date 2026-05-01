// market_regime.rs
// Gerçek Zamanlı Piyasa Adaptasyonu ve Strateji Switching Modülü
// ML tabanlı piyasa rejimi tespiti ve otomatik strateji geçişi

use crate::types::Candle;

/// Piyasa rejimi türleri
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketRegime {
    Trending,
    Ranging,
    Volatile,
    LowVolatility,
    Unknown,
}

/// Piyasa rejimi tespit trait'i
pub trait MarketRegimeDetector {
    fn detect_regime(&self, candles: &[Candle]) -> MarketRegime;
}

/// Basit örnek: SMA ve volatilite ile rejim tespiti
pub struct SimpleRegimeDetector;

impl MarketRegimeDetector for SimpleRegimeDetector {
    fn detect_regime(&self, candles: &[Candle]) -> MarketRegime {
        if candles.len() < 20 {
            return MarketRegime::Unknown;
        }
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let mean = closes.iter().sum::<f64>() / closes.len() as f64;
        let stddev = (closes.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / closes.len() as f64).sqrt();
        if stddev > mean * 0.05 {
            MarketRegime::Volatile
        } else if closes.last().unwrap() > &mean {
            MarketRegime::Trending
        } else {
            MarketRegime::Ranging
        }
    }
}

/// Strateji switching trait'i
pub trait StrategySwitcher {
    fn select_strategy(&self, regime: MarketRegime, available: &[String]) -> Option<String>;
}

/// Basit örnek: Rejime göre strateji seçici
pub struct SimpleStrategySwitcher;

impl StrategySwitcher for SimpleStrategySwitcher {
    fn select_strategy(&self, regime: MarketRegime, available: &[String]) -> Option<String> {
        match regime {
            MarketRegime::Trending => available.iter().find(|s| s.contains("trend")).cloned(),
            MarketRegime::Ranging => available.iter().find(|s| s.contains("range")).cloned(),
            MarketRegime::Volatile => available.iter().find(|s| s.contains("vol")).cloned(),
            MarketRegime::LowVolatility => available.iter().find(|s| s.contains("mean")).cloned(),
            MarketRegime::Unknown => None,
        }
    }
}

// Gelişmiş ML tabanlı tespit için Python/Rust FFI veya WASM entegrasyonu eklenebilir.

// ─── ADX / HMM Rejim Algılama ───────────────────────────────────────────────

/// HMM tabanlı üç-rejim modeli; strateji değişikliği basit kayıp sayısına değil
/// piyasanın yapısal durumuna bağlanır.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdxRegime {
    /// ADX < 20: düz/yatay piyasa → Mean Reversion stratejileri
    Ranging,
    /// ADX 20-25: geçiş/belirsizlik → mevcut composite ranking geçerli
    Neutral,
    /// ADX > 25: trend → Trend takipçi stratejiler
    Trending,
    /// ATR% > %3 override: volatilite/kaos → yeni giriş yok
    Volatile,
}

impl std::fmt::Display for AdxRegime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdxRegime::Ranging  => write!(f, "Ranging(BB/RSI)"),
            AdxRegime::Neutral  => write!(f, "Nötr"),
            AdxRegime::Trending => write!(f, "Trending(EMA/ST)"),
            AdxRegime::Volatile => write!(f, "Volatile/Kaos"),
        }
    }
}

/// Wilder basitleştirilmiş ADX — `feature_extractor::adx` ile özdeş mantık.
pub fn compute_adx_from_candles(candles: &[Candle]) -> f64 {
    let n = candles.len();
    if n < 15 { return 25.0; }
    let period = 14usize;
    let start = n.saturating_sub(period + 1);
    let (mut plus_dm, mut minus_dm, mut tr_sum) = (0.0_f64, 0.0_f64, 0.0_f64);
    for i in (start + 1)..n {
        let c = &candles[i];
        let p = &candles[i - 1];
        let up   = c.high - p.high;
        let down = p.low  - c.low;
        if up > down && up > 0.0   { plus_dm  += up;   }
        if down > up && down > 0.0 { minus_dm += down; }
        let hl = c.high - c.low;
        let hc = (c.high - p.close).abs();
        let lc = (c.low  - p.close).abs();
        tr_sum += hl.max(hc).max(lc);
    }
    if tr_sum == 0.0 { return 25.0; }
    let plus_di  = 100.0 * plus_dm  / tr_sum;
    let minus_di = 100.0 * minus_dm / tr_sum;
    let di_sum   = plus_di + minus_di;
    if di_sum == 0.0 { return 0.0; }
    (100.0 * (plus_di - minus_di).abs() / di_sum).clamp(0.0, 100.0)
}

fn compute_atr_pct(candles: &[Candle]) -> f64 {
    let n = candles.len();
    if n < 2 { return 0.0; }
    let period = 14.min(n - 1);
    let start  = n - period - 1;
    let atr: f64 = candles[(start + 1)..n].iter().enumerate().map(|(i, c)| {
        let p = &candles[start + i];
        let hl = c.high - c.low;
        let hc = (c.high - p.close).abs();
        let lc = (c.low  - p.close).abs();
        hl.max(hc).max(lc)
    }).sum::<f64>() / period as f64;
    let last_close = candles.last().map(|c| c.close).unwrap_or(1.0);
    if last_close > 0.0 { atr / last_close * 100.0 } else { 0.0 }
}

/// ADX + ATR'ye dayalı piyasa rejimi tespiti.
pub fn detect_adx_regime(candles: &[Candle]) -> AdxRegime {
    let adx     = compute_adx_from_candles(candles);
    let atr_pct = compute_atr_pct(candles);
    // Kripto için 3% ATR çok yaygın; 7%+ gerçek "kaos" rejimidir
    if atr_pct > 7.0      { return AdxRegime::Volatile; }
    if adx < 20.0         { AdxRegime::Ranging  }
    else if adx > 25.0    { AdxRegime::Trending }
    else                  { AdxRegime::Neutral  }
}

/// Rejime göre izin verilen strateji kümesi.
/// Boş dilim = filtre yok (Neutral); Volatile = tüm girişler engellenir.
pub fn strategies_for_adx_regime(regime: AdxRegime) -> &'static [&'static str] {
    match regime {
        AdxRegime::Ranging  => &["RSI", "BB", "CCI", "WILLIAMS", "STOCHASTIC", "STOCH_RSI", "PRICE_ACTION", "SMC"],
        AdxRegime::Trending => &["SUPERTREND", "EMA", "MACD", "ICT_SWEEP", "ICT_OTE", "ICT_COMPOSITE", "ICT_OB", "ADX"],
        AdxRegime::Neutral  => &[],
        AdxRegime::Volatile => &[],
    }
}
