// robot/signal_evaluator.rs
//
// Sinyal kalite filtreleri ve yardımcı analiz fonksiyonları.
// Tüm fonksiyonlar pure — dışarıdan veri alır, yan etki üretmez.
// robotic_loop.rs monolitinden ayrıştırıldı; bağımsız test edilebilir.
//
// Dışa bağımlılıklar: crate::types::Candle, crate::strategies::calculate_sma

use crate::core::types::Candle;
use serde::{Deserialize, Serialize};

// ── Yapılar ─────────────────────────────────────────────────────────────────

/// Ticaret kalitesi parametreleri — hot-reload ile disk'ten okunabilir.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradeQualityConfig {
    /// Minimum risk/ödül oranı
    pub min_rr: f64,
    /// Minimum volatilite yüzdesi (çok durağan piyasada işlem yapma)
    pub volatility_min_pct: f64,
    /// Maksimum volatilite yüzdesi (çok gürültülü piyasada işlem yapma)
    pub volatility_max_pct: f64,
    /// Trend filtresi kısa SMA periyodu
    pub trend_short_period: usize,
    /// Trend filtresi uzun SMA periyodu
    pub trend_long_period: usize,
    /// Trend filtresi aktif mi? false → trend filtresi devre dışı (tüm sinyaller geçer)
    #[serde(default = "default_trend_filter_enabled")]
    pub trend_filter_enabled: bool,
    /// Nötr bölge eşiği — SMA farkı bu %'nin altındaysa Neutral döner, sinyal engellenmez.
    /// Örn. 0.5 → %0.5'ten az ayrışmada trend filtresi uygulanmaz.
    /// 0.0 = eski ikili (Bullish/Bearish) davranış.
    #[serde(default = "default_trend_margin")]
    pub trend_margin_pct: f64,
    /// Uyarlamalı eşik ayarı etkin mi?
    pub adaptive_enabled: bool,
    // Uyarlamalı eşikler — düşük win rate'de daha sıkı, yüksek win rate'de daha geniş
    pub min_rr_tight: f64,
    pub min_rr_loose: f64,
    pub volatility_max_tight: f64,
    pub volatility_max_loose: f64,
    /// Bu win rate altındaysa sıkı mod
    pub win_rate_low: f64,
    /// Bu win rate üstündeyse geniş mod
    pub win_rate_high: f64,
    // ── Hacim filtresi ────────────────────────────────────────────────────────
    /// Hacim filtresi aktif mi?
    #[serde(default)]
    pub volume_filter_enabled: bool,
    /// Son 20 mumun ortalama hacmine oranla minimum hacim eşiği (örn. 0.7 = %70)
    #[serde(default = "default_volume_min_ratio")]
    pub volume_min_ratio: f64,
    // ── RSI aşırı bölge filtresi ──────────────────────────────────────────────
    /// RSI aşırı bölge filtresi aktif mi?
    #[serde(default)]
    pub rsi_extreme_filter_enabled: bool,
    /// BUY'u engelleyen üst RSI eşiği (aşırı alım, örn. 80)
    #[serde(default = "default_rsi_extreme_ob")]
    pub rsi_extreme_ob: f64,
    /// SELL'i engelleyen alt RSI eşiği (aşırı satım, örn. 20)
    #[serde(default = "default_rsi_extreme_os")]
    pub rsi_extreme_os: f64,
    // ── HTF hizalama filtresi ─────────────────────────────────────────────────
    /// true → HTF Neutral değil, sinyal yönüyle açıkça hizalı olmalı
    #[serde(default)]
    pub htf_require_alignment: bool,
}

fn default_trend_filter_enabled() -> bool { true }
fn default_trend_margin() -> f64 { 0.5 }
fn default_volume_min_ratio() -> f64 { 0.7 }
fn default_rsi_extreme_ob() -> f64 { 80.0 }
fn default_rsi_extreme_os() -> f64 { 20.0 }

/// Trend yönü
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrendBias {
    Bullish,
    Bearish,
    Neutral,
}

/// Kalite filtresi bloğunun nedeni — log ve sayaç güncelleme için
#[derive(Debug, Clone)]
pub enum FilterBlock {
    RrTooLow    { rr: f64, min_rr: f64 },
    Volatility  { avg_range_pct: f64 },
    TrendBearish,
    TrendBullish,
    /// Destek/Direnç bağlamı min_quality puanının altında
    SrQualityTooLow { quality: f64, min_quality: f64 },
}

// ── Fonksiyonlar ─────────────────────────────────────────────────────────────

/// Son `lookback` mum için ortalama fiyat aralığı yüzdesi.
/// Volatilite filtresi için kullanılır.
///
/// `(high - low) / close` ortalaması → %cinsinden.
pub fn average_range_pct(candles: &[Candle], lookback: usize) -> Option<f64> {
    let n = candles.len().min(lookback);
    if n == 0 { return None; }
    let recent = &candles[candles.len() - n..];
    let total: f64 = recent.iter()
        .filter(|c| c.close > 0.0)
        .map(|c| (c.high - c.low).abs() / c.close * 100.0)
        .sum();
    Some(total / n as f64)
}

/// Kısa SMA ile uzun SMA'yı karşılaştırarak trend yönünü belirle.
///
/// `margin_pct`: iki SMA arasındaki fark bu yüzdenin altındaysa `Neutral` döner.
/// Örn. 0.5 → SMAs %0.5'ten az ayrışıyorsa sinyal engellenmez.
/// 0.0 kullanılırsa eski ikili (Bullish/Bearish) davranış korunur.
///
/// Yetersiz veri varsa `None` döner.
pub fn trend_bias(
    candles: &[Candle],
    short_period: usize,
    long_period: usize,
    margin_pct: f64,
) -> Option<TrendBias> {
    use crate::core::indicators::CoreIndicatorEngine;
    if candles.len() < long_period + 1 { return None; }
    let short_sma = CoreIndicatorEngine::sma(candles, short_period);
    let long_sma  = CoreIndicatorEngine::sma(candles, long_period);
    if long_sma <= 0.0 { return None; }
    let diff_pct = (short_sma - long_sma).abs() / long_sma * 100.0;
    if diff_pct < margin_pct {
        return Some(TrendBias::Neutral);
    }
    Some(if short_sma > long_sma {
        TrendBias::Bullish
    } else {
        TrendBias::Bearish
    })
}

/// Win rate'e göre kalite eşiklerini uyarlamalı ayarla.
///
/// Döndürür: `(min_rr, volatility_max_pct)`
pub fn adjust_quality_thresholds(
    win_rate: f64,
    quality: &TradeQualityConfig,
) -> (f64, f64) {
    if !quality.adaptive_enabled {
        return (quality.min_rr, quality.volatility_max_pct);
    }
    if win_rate < quality.win_rate_low {
        // Düşük başarı → daha sıkı filtre
        (quality.min_rr_tight, quality.volatility_max_tight)
    } else if win_rate > quality.win_rate_high {
        // Yüksek başarı → biraz gevşet
        (quality.min_rr_loose, quality.volatility_max_loose)
    } else {
        (quality.min_rr, quality.volatility_max_pct)
    }
}

/// R/R ve volatilite için kalite filtresini çalıştır.
///
/// `Ok(())` → sinyal geçti, işleme devam.
/// `Err(FilterBlock)` → sinyal engellendi, nedeni döner.
///
/// Trend filtresi ayrı bir fonksiyonda (`check_trend_filter`) tutulur
/// çünkü `allows_short` ayarına bağlıdır.
pub fn check_quality_filters(
    rr: f64,
    min_rr: f64,
    candles: &[Candle],
    volatility_min_pct: f64,
    volatility_max_pct: f64,
) -> Result<(), FilterBlock> {
    if rr < min_rr {
        return Err(FilterBlock::RrTooLow { rr, min_rr });
    }
    if let Some(avg_range_pct) = average_range_pct(candles, 20) {
        if avg_range_pct < volatility_min_pct || avg_range_pct > volatility_max_pct {
            return Err(FilterBlock::Volatility { avg_range_pct });
        }
    }
    Ok(())
}

/// Destek/Direnç filtresi — `SrContext.buy_quality` / `sell_quality` puanını eşikle karşılaştırır.
///
/// `Ok(())` → entry kalitesi yeterli, devam.
/// `Err(FilterBlock::SrQualityTooLow)` → kötü entry noktası, engellendi.
pub fn check_sr_filter(
    is_buy:           bool,
    buy_quality:      f64,
    sell_quality:     f64,
    min_buy_quality:  f64,
    min_sell_quality: f64,
) -> Result<(), FilterBlock> {
    let (quality, min_quality) = if is_buy {
        (buy_quality,  min_buy_quality)
    } else {
        (sell_quality, min_sell_quality)
    };
    if quality < min_quality {
        return Err(FilterBlock::SrQualityTooLow { quality, min_quality });
    }
    Ok(())
}

/// Trend filtresini uygula.
///
/// `Ok(())` → sinyal trende uygun, devam.
/// `Err(FilterBlock)` → trende karşı sinyal, engellendi.
pub fn check_trend_filter(
    is_buy: bool,
    allows_short: bool,
    candles: &[Candle],
    short_period: usize,
    long_period: usize,
    margin_pct: f64,
) -> Result<(), FilterBlock> {
    let Some(bias) = trend_bias(candles, short_period, long_period, margin_pct) else {
        return Ok(()); // veri yetersiz — engelleme yok
    };
    if is_buy && bias == TrendBias::Bearish {
        return Err(FilterBlock::TrendBearish);
    }
    // allows_short futures'ta mekanik imkânı açar, trend filtresini bypass etmez.
    if !is_buy && bias == TrendBias::Bullish {
        return Err(FilterBlock::TrendBullish);
    }
    let _ = allows_short; // artık filtre kararını etkilemiyor
    Ok(())
}

// ── Testler ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::Candle;
    use chrono::Utc;

    fn make_candle(close: f64, high: f64, low: f64) -> Candle {
        Candle { timestamp: Utc::now(), open: close, high, low, close, volume: 1.0,
            symbol: "TEST".into(), interval: "1h".into() }
    }

    #[test]
    fn average_range_basic() {
        let candles = vec![
            make_candle(100.0, 102.0, 98.0),  // range = 4/100 = 4%
            make_candle(100.0, 103.0, 97.0),  // range = 6/100 = 6%
        ];
        let r = average_range_pct(&candles, 10).unwrap();
        assert!((r - 5.0).abs() < 0.001);
    }

    #[test]
    fn adjust_thresholds_low_winrate() {
        let q = TradeQualityConfig {
            min_rr: 1.2, volatility_min_pct: 0.05, volatility_max_pct: 3.0,
            trend_short_period: 20, trend_long_period: 50,
            trend_filter_enabled: true, trend_margin_pct: 0.5,
            adaptive_enabled: true,
            min_rr_tight: 1.5, min_rr_loose: 1.1,
            volatility_max_tight: 2.0, volatility_max_loose: 3.5,
            win_rate_low: 40.0, win_rate_high: 55.0,
            volume_filter_enabled: false, volume_min_ratio: 0.7,
            rsi_extreme_filter_enabled: false, rsi_extreme_ob: 80.0, rsi_extreme_os: 20.0,
            htf_require_alignment: false,
        };
        let (rr, vol) = adjust_quality_thresholds(35.0, &q);
        assert_eq!(rr, 1.5);
        assert_eq!(vol, 2.0);
    }

    #[test]
    fn rr_filter_blocks_low_rr() {
        let candles: Vec<Candle> = (0..20)
            .map(|_| make_candle(100.0, 101.0, 99.0))
            .collect();
        let result = check_quality_filters(0.9, 1.2, &candles, 0.5, 5.0);
        assert!(matches!(result, Err(FilterBlock::RrTooLow { .. })));
    }

    #[test]
    fn rr_filter_passes() {
        let candles: Vec<Candle> = (0..20)
            .map(|_| make_candle(100.0, 101.5, 98.5))
            .collect();
        let result = check_quality_filters(2.0, 1.2, &candles, 0.5, 5.0);
        assert!(result.is_ok());
    }
}
