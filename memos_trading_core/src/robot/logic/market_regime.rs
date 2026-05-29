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

/// Rejim eşikleri — Volatile sınırı + ADX bandları. `Default` tarihsel sabitleri
/// birebir taşır (legacy davranış: ATR%>7 → Volatil, ADX<20 → Yatay, ADX>25 → Trend).
/// Adaptif modda yalnız `atr_volatile_pct` sembolün KENDİ ATR% dağılımının bir
/// persentilinden türetilir ("bu sembol kendisi için olağandışı volatil mi" — cross-
/// symbol sabit yerine relatif). ADX bandları sabit kalır: ADX zaten 0-100 normalize,
/// sembolden bağımsız kıyaslanabilir → adaptif yapmak gereksiz serbestlik (overfit).
/// [[project_autonomy_backlog]] #1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegimeThresholds {
    pub atr_volatile_pct: f64,
    pub adx_ranging: f64,
    pub adx_trending: f64,
}

impl Default for RegimeThresholds {
    fn default() -> Self {
        Self { atr_volatile_pct: 7.0, adx_ranging: 20.0, adx_trending: 25.0 }
    }
}

/// Bir mum dilimi boyunca kayan-pencere ATR% serisi üretir (her bitiş indeksinde
/// 14-pencere ATR%). Adaptif Volatile sınırının (sembolün kendi dağılımının
/// persentili) girdisidir. n<16 → boş (yeterli örnek yok).
fn rolling_atr_pct_series(candles: &[Candle]) -> Vec<f64> {
    let n = candles.len();
    if n < 16 { return Vec::new(); }
    // İlk 15 mum ATR penceresini doldurur; sonraki her bitişte ATR% örneği üret.
    (15..=n).map(|end| compute_atr_pct(&candles[..end])).filter(|v| v.is_finite() && *v > 0.0).collect()
}

/// `sorted`'a göre değil; kopyalayıp sıralar. `pctl` ∈ [0,1]. Boş → None.
/// Doğrusal-enterpolasyonsuz "nearest-rank" (basit, kararlı).
fn percentile(values: &[f64], pctl: f64) -> Option<f64> {
    if values.is_empty() { return None; }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p = pctl.clamp(0.0, 1.0);
    let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
    v.get(idx).copied()
}

/// Sembolün kendi son ATR% dağılımından adaptif Volatile sınırını türetir.
/// `pctl` örn. 0.80 → "ATR% kendi son dağılımının üst %20'sine girince Volatil".
/// Yeterli örnek yoksa absolute default'a düşer (kısa seri / cold-start güvenli).
pub fn adaptive_thresholds(candles: &[Candle], pctl: f64) -> RegimeThresholds {
    let base = RegimeThresholds::default();
    match percentile(&rolling_atr_pct_series(candles), pctl) {
        Some(p) if p.is_finite() && p > 0.0 => RegimeThresholds { atr_volatile_pct: p, ..base },
        _ => base,
    }
}

/// Ana Rejim Dedektörü: robotic_loop'un "Vites Kolu". Tarihsel sabit eşiklerle
/// (`RegimeThresholds::default()`) delege — davranış birebir korunur.
pub fn detect_adx_regime(candles: &[Candle]) -> AdxRegime {
    detect_adx_regime_with(candles, &RegimeThresholds::default())
}

/// Eşik-parametreli rejim dedektörü (tek-kaynak). Absolute (Default) ya da adaptif
/// (`adaptive_thresholds`) eşik geçilebilir; mantık aynı.
pub fn detect_adx_regime_with(candles: &[Candle], thr: &RegimeThresholds) -> AdxRegime {
    if compute_atr_pct(candles) > thr.atr_volatile_pct { return AdxRegime::Volatile; }
    match compute_adx_from_candles(candles) {
        a if a < thr.adx_ranging => AdxRegime::Ranging,
        a if a > thr.adx_trending => AdxRegime::Trending,
        _ => AdxRegime::Neutral,
    }
}

/// 🌐 Mum dizisinden `evolution::MarketRegime` üretir — AdxRegime (yapı/volatilite)
/// + yön sınıflandırması. Tek-kaynak: canlı cycle (`Engine::classify_regime` → delege),
/// RegimeContext detektörü, backtest agregasyonu hep buradan geçer.
///
/// `dir_score` (Adım 1 / AI): `Some(s)` ise Trending rejimin YÖNÜ bu skordan ([-1,1],
/// poz=boğa/neg=ayı) belirlenir — GBT/ONNX gibi bir model rejim yönünü besler. `None`
/// ise yön fiyat momentumundan (eski/saf-matematik davranış). Yapı (Ranging/Volatile/
/// Neutral) her durumda matematik (ADX/ATR) — AI yalnız Trending yönünü zenginleştirir.
pub fn classify_market_regime_with_score(
    candles: &[Candle], dir_score: Option<f64>,
) -> crate::evolution::MarketRegime {
    classify_market_regime_with(candles, dir_score, &RegimeThresholds::default())
}

/// `classify_market_regime_with_score`'un eşik-parametreli hali (tek-kaynak). Yalnız
/// ADX/ATR yapı kararı `thr`'den etkilenir (adaptif Volatile sınırı); yön bandları
/// (momentum ±2%/±0.5%) sabit — sembol-ölçekli adaptasyon Volatile gate'inde test
/// ediliyor, yön persentilleri ayrı bir adım (telemetri/bucketing etkisi, A/B'siz).
pub fn classify_market_regime_with(
    candles: &[Candle], dir_score: Option<f64>, thr: &RegimeThresholds,
) -> crate::evolution::MarketRegime {
    use crate::evolution::MarketRegime;
    if candles.len() < 20 { return MarketRegime::Unknown; }
    let adx = detect_adx_regime_with(candles, thr);
    let recent = &candles[candles.len() - 20..];
    let first = recent.first().map(|c| c.close).unwrap_or(0.0);
    let last  = recent.last().map(|c| c.close).unwrap_or(0.0);
    if first <= 0.0 { return MarketRegime::Unknown; }
    let mom_pct = (last - first) / first * 100.0;
    match adx {
        AdxRegime::Volatile => MarketRegime::HighVolatility,
        AdxRegime::Ranging  => MarketRegime::Ranging,
        AdxRegime::Trending => match dir_score {
            // AI yön skoru [-1,1]: |s|>0.5 güçlü, işaret yön.
            Some(s) if s >  0.5 => MarketRegime::StrongUptrend,
            Some(s) if s >  0.0 => MarketRegime::WeakUptrend,
            Some(s) if s < -0.5 => MarketRegime::StrongDowntrend,
            Some(_)             => MarketRegime::WeakDowntrend,
            // Skor yok → fiyat momentumu (eski davranış birebir).
            None if mom_pct >  2.0 => MarketRegime::StrongUptrend,
            None if mom_pct >  0.0 => MarketRegime::WeakUptrend,
            None if mom_pct < -2.0 => MarketRegime::StrongDowntrend,
            None                   => MarketRegime::WeakDowntrend,
        },
        AdxRegime::Neutral if mom_pct.abs() < 0.5 => MarketRegime::LowVolatility,
        AdxRegime::Neutral                        => MarketRegime::Unknown,
    }
}

/// `classify_market_regime_with_score(candles, None)` — saf matematik (momentum yönü).
pub fn classify_market_regime(candles: &[Candle]) -> crate::evolution::MarketRegime {
    classify_market_regime_with_score(candles, None)
}

/// Rejim yönü verilen işlem yönünü teyit ediyor mu? long → non-downtrend, short →
/// non-uptrend. Yön-belirsiz rejimler (Ranging/HighVolatility/LowVolatility/Unknown)
/// her iki yönü de teyit eder — filtre yalnız AÇIK ters-trend girişini eler, nötr/yatay
/// rejimde stratejinin kararına güvenir. Tek-kaynak: backtest `RegimeDirectional` modu
/// ve canlı `regime_directional` kapısı bunu kullanır. [[project_adaptive_regime]].
pub fn regime_confirms_direction(regime: crate::evolution::MarketRegime, is_long: bool) -> bool {
    use crate::evolution::MarketRegime as R;
    if is_long {
        !matches!(regime, R::StrongDowntrend | R::WeakDowntrend)
    } else {
        !matches!(regime, R::StrongUptrend | R::WeakUptrend)
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

#[cfg(test)]
mod regime_classify_tests {
    use super::*;
    use crate::evolution::MarketRegime;
    use chrono::{TimeZone, Utc};

    /// Sıkı bantlı, kararlı yükseliş → ADX yüksek (Trending), ATR% düşük → yön branch'i.
    fn trending_up(n: usize) -> Vec<Candle> {
        (0..n).map(|i| {
            let c = 100.0 + i as f64;
            Candle {
                timestamp: Utc.timestamp_opt(1_700_000_000 + i as i64 * 60, 0).unwrap(),
                open: c, high: c + 0.5, low: c - 0.5, close: c,
                volume: 1.0, symbol: "T".into(), interval: "1m".into(),
            }
        }).collect()
    }

    #[test]
    fn none_score_equals_classify_market_regime() {
        // Tek-kaynak: with_score(None) ≡ classify_market_regime (parity/regresyon).
        let cs = trending_up(60);
        assert_eq!(
            classify_market_regime_with_score(&cs, None),
            classify_market_regime(&cs),
        );
    }

    #[test]
    fn trending_series_is_detected_as_trending() {
        // Test serisi gerçekten Trending olmalı (yön branch'i çalışsın).
        assert_eq!(detect_adx_regime(&trending_up(60)), AdxRegime::Trending);
    }

    /// Geniş bantlı (yüksek ATR%) seri — adaptif sınır testi için.
    fn jittery(n: usize, amp: f64) -> Vec<Candle> {
        (0..n).map(|i| {
            let base = 100.0;
            let c = base + (i as f64 * 0.7).sin() * amp;
            Candle {
                timestamp: Utc.timestamp_opt(1_700_000_000 + i as i64 * 60, 0).unwrap(),
                open: c, high: c + amp, low: c - amp, close: c,
                volume: 1.0, symbol: "J".into(), interval: "1m".into(),
            }
        }).collect()
    }

    #[test]
    fn default_thresholds_match_legacy_constants() {
        // RegimeThresholds::default() tarihsel sabitleri birebir taşır.
        let t = RegimeThresholds::default();
        assert_eq!((t.atr_volatile_pct, t.adx_ranging, t.adx_trending), (7.0, 20.0, 25.0));
    }

    #[test]
    fn detect_with_default_equals_legacy_detect() {
        // _with(Default) ≡ detect_adx_regime (parity/regresyon, tek-kaynak).
        for cs in [trending_up(60), jittery(60, 3.0), jittery(60, 0.1)] {
            assert_eq!(detect_adx_regime(&cs), detect_adx_regime_with(&cs, &RegimeThresholds::default()));
        }
    }

    #[test]
    fn percentile_nearest_rank() {
        let v = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&v, 0.0), Some(1.0));
        assert_eq!(percentile(&v, 1.0), Some(5.0));
        assert_eq!(percentile(&v, 0.5), Some(3.0));
        assert_eq!(percentile(&[], 0.5), None);
    }

    #[test]
    fn adaptive_threshold_falls_back_on_short_series() {
        // Yetersiz örnek → absolute default (cold-start güvenli).
        assert_eq!(adaptive_thresholds(&trending_up(5), 0.8), RegimeThresholds::default());
    }

    #[test]
    fn adaptive_threshold_is_symbol_relative() {
        // Sıkı bantlı sembolde adaptif Volatile sınırı absolute 7.0'ın ÇOK altına
        // iner → sembol kendi ölçeğinde "olağandışı volatil"i daha erken yakalar.
        let calm = trending_up(80); // ATR% ~ %1 civarı
        let thr = adaptive_thresholds(&calm, 0.8);
        assert!(thr.atr_volatile_pct < 7.0, "adaptif sınır absolute'ın altında olmalı: {}", thr.atr_volatile_pct);
        assert!(thr.atr_volatile_pct > 0.0);
        // ADX bandları değişmez (sabit kalır).
        assert_eq!((thr.adx_ranging, thr.adx_trending), (20.0, 25.0));
    }

    #[test]
    fn regime_direction_confirmation() {
        use crate::evolution::MarketRegime as R;
        // long: downtrend'de RED, diğerlerinde OK.
        assert!(!regime_confirms_direction(R::StrongDowntrend, true));
        assert!(!regime_confirms_direction(R::WeakDowntrend, true));
        assert!(regime_confirms_direction(R::StrongUptrend, true));
        assert!(regime_confirms_direction(R::Ranging, true), "nötr rejim long'u teyit eder");
        // short: uptrend'de RED, diğerlerinde OK.
        assert!(!regime_confirms_direction(R::StrongUptrend, false));
        assert!(!regime_confirms_direction(R::WeakUptrend, false));
        assert!(regime_confirms_direction(R::StrongDowntrend, false));
        assert!(regime_confirms_direction(R::HighVolatility, false), "nötr rejim short'u teyit eder");
    }

    #[test]
    fn gbt_score_drives_trend_direction() {
        // AYNI yükseliş serisi; yön YALNIZ dir_score'dan gelir (momentum boğa olsa bile).
        let cs = trending_up(60);
        assert_eq!(classify_market_regime_with_score(&cs, Some(0.8)),  MarketRegime::StrongUptrend);
        assert_eq!(classify_market_regime_with_score(&cs, Some(0.3)),  MarketRegime::WeakUptrend);
        assert_eq!(classify_market_regime_with_score(&cs, Some(-0.3)), MarketRegime::WeakDowntrend);
        assert_eq!(classify_market_regime_with_score(&cs, Some(-0.8)), MarketRegime::StrongDowntrend);
    }
}
