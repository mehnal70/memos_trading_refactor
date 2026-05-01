// robot/pattern_matcher.rs — Piyasa koşulu parmak izi (MarketCondition) ve pattern eşleştirme
//
// Kullanım:
//   1. Backtest sonrası: MarketCondition::from_candles() ile koşulu hesapla,
//      database_writer::save_pattern() ile kaydet.
//   2. Canlı işlem öncesi: aynı fonksiyon ile mevcut koşulu hesapla,
//      database_writer::query_best_pattern() ile geçmiş başarıyı sorgula.

use crate::types::Candle;

// ─── Trend ─────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, PartialEq)]
pub enum TrendState {
    Up,
    Down,
    Sideways,
}

impl TrendState {
    pub fn as_str(&self) -> &'static str {
        match self { TrendState::Up => "up", TrendState::Down => "dn", TrendState::Sideways => "sw" }
    }
}

// ─── Volatilite ────────────────────────────────────────────────────────────
#[derive(Debug, Clone, PartialEq)]
pub enum VolatilityState {
    Low,
    Med,
    High,
}

impl VolatilityState {
    pub fn as_str(&self) -> &'static str {
        match self { VolatilityState::Low => "lo", VolatilityState::Med => "md", VolatilityState::High => "hi" }
    }
}

// ─── Momentum ──────────────────────────────────────────────────────────────
#[derive(Debug, Clone, PartialEq)]
pub enum MomentumState {
    Oversold,
    Neutral,
    Overbought,
}

impl MomentumState {
    pub fn as_str(&self) -> &'static str {
        match self { MomentumState::Oversold => "os", MomentumState::Neutral => "ne", MomentumState::Overbought => "ob" }
    }
}

// ─── MarketCondition ───────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub struct MarketCondition {
    pub trend:      TrendState,
    pub volatility: VolatilityState,
    pub momentum:   MomentumState,
}

impl MarketCondition {
    /// Son `candles` diliminden piyasa koşulunu türetir.
    /// `stoch_k_period`: Stochastic K periyodu (genellikle 6–14)
    /// `stoch_os` / `stoch_ob`: aşırı satış / alış eşikleri
    pub fn from_candles(candles: &[Candle], stoch_k_period: usize, stoch_os: f64, stoch_ob: f64) -> Self {
        Self {
            trend:      compute_trend(candles),
            volatility: compute_volatility(candles),
            momentum:   compute_momentum(candles, stoch_k_period, stoch_os, stoch_ob),
        }
    }

    /// "up|lo|os" formatında tek string anahtar
    pub fn key(&self) -> String {
        format!("{}|{}|{}", self.trend.as_str(), self.volatility.as_str(), self.momentum.as_str())
    }

    /// (trend_str, volatility_str, momentum_str) — DB'ye ayrı sütunlar için
    pub fn parts(&self) -> (&'static str, &'static str, &'static str) {
        (self.trend.as_str(), self.volatility.as_str(), self.momentum.as_str())
    }
}

// ─── Hesaplama fonksiyonları ───────────────────────────────────────────────

/// Trend: son kapanış ile 50-mum SMA karşılaştırması
fn compute_trend(candles: &[Candle]) -> TrendState {
    let n = candles.len().min(50);
    if n < 5 { return TrendState::Sideways; }
    // candles[0] en eski, candles[last] en yeni varsayımı (veya tersi — ikisini de dene)
    let closes: Vec<f64> = candles.iter().rev().take(n).map(|c| c.close).collect();
    let sma = closes.iter().sum::<f64>() / closes.len() as f64;
    let last = closes[0]; // en yeni mum
    if last > sma * 1.005 { TrendState::Up }
    else if last < sma * 0.995 { TrendState::Down }
    else { TrendState::Sideways }
}

/// Volatilite: BB genişliği (BB band/orta) tarihsel ortalamaya göre normalize
fn compute_volatility(candles: &[Candle]) -> VolatilityState {
    const PERIOD: usize = 20;
    const STD_MULT: f64 = 2.0;

    let n = candles.len();
    if n < PERIOD { return VolatilityState::Med; }

    let closes: Vec<f64> = candles.iter().rev().map(|c| c.close).collect();

    let bb_width = |slice: &[f64]| -> f64 {
        let mean = slice.iter().sum::<f64>() / slice.len() as f64;
        let std = (slice.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / slice.len() as f64).sqrt();
        if mean > 0.0 { STD_MULT * std / mean } else { 0.0 }
    };

    let cur = bb_width(&closes[..PERIOD]);

    // Karşılaştırma: sonraki 20 mum penceresi (varsa)
    if n < PERIOD * 2 {
        return if cur < 0.015 { VolatilityState::Low }
               else if cur > 0.04 { VolatilityState::High }
               else { VolatilityState::Med };
    }
    let hist = bb_width(&closes[PERIOD..PERIOD * 2]);
    if hist < 1e-10 { return VolatilityState::Med; }
    let ratio = cur / hist;
    if ratio < 0.80 { VolatilityState::Low }
    else if ratio > 1.35 { VolatilityState::High }
    else { VolatilityState::Med }
}

/// Momentum: Stochastic %K değeri
fn compute_momentum(candles: &[Candle], k_period: usize, os: f64, ob: f64) -> MomentumState {
    let k = k_period.max(3);
    let window: Vec<&Candle> = candles.iter().rev().take(k).collect();
    if window.is_empty() { return MomentumState::Neutral; }
    let high_max = window.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
    let low_min  = window.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    let close    = window[0].close;
    let stoch_k  = if (high_max - low_min).abs() < 1e-10 { 50.0 }
                   else { (close - low_min) / (high_max - low_min) * 100.0 };
    if stoch_k < os { MomentumState::Oversold }
    else if stoch_k > ob { MomentumState::Overbought }
    else { MomentumState::Neutral }
}

/// Backtest sonucundan composite confidence skoru hesapla (0.0–1.0)
/// Hem win_rate hem trade_count'u ağırlıklandırır.
pub fn compute_confidence(win_rate: f64, trade_count: i64, avg_pnl: f64) -> f64 {
    if trade_count < 5 { return 0.0; }
    // Win rate faktörü: 50%=0, 75%=0.5, 100%=1.0
    let wr_factor = ((win_rate - 0.5) / 0.5).clamp(0.0, 1.0);
    // Trade count faktörü: 5=0.2, 20=0.67, 50+=1.0
    let tc_factor = ((trade_count as f64 - 5.0) / 45.0).clamp(0.0, 1.0);
    // Avg PnL faktörü: 0%=0, 0.5%=0.5, 1%+=1.0
    let pnl_factor = (avg_pnl / 1.0).clamp(0.0, 1.0);
    (wr_factor * 0.5 + tc_factor * 0.25 + pnl_factor * 0.25).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_candles(closes: &[f64]) -> Vec<Candle> {
        closes.iter().enumerate().map(|(_i, &c)| Candle {
            timestamp: Utc::now(),
            open: c, high: c * 1.01, low: c * 0.99, close: c,
            volume: 1.0,
            symbol: "TEST".to_string(),
            interval: "1m".to_string(),
        }).collect()
    }

    #[test]
    fn test_trend_up() {
        let closes: Vec<f64> = (0..50).map(|i| 100.0 + i as f64 * 0.5).collect();
        // son mum SMA'dan %1 yukarıda
        let candles = make_candles(&closes);
        let cond = MarketCondition::from_candles(&candles, 6, 20.0, 80.0);
        assert_eq!(cond.trend, TrendState::Up);
    }

    #[test]
    fn test_momentum_oversold() {
        // Son 6 mumda kapanış minimum yakın
        let closes: Vec<f64> = vec![100.0, 99.0, 98.0, 97.0, 96.5, 96.0];
        let candles = make_candles(&closes);
        let cond = MarketCondition::from_candles(&candles, 6, 20.0, 80.0);
        assert_eq!(cond.momentum, MomentumState::Oversold);
    }

    #[test]
    fn test_confidence_low_trades() {
        let c = compute_confidence(0.80, 3, 0.5);
        assert_eq!(c, 0.0); // min 5 trade gerekli
    }

    #[test]
    fn test_condition_key_format() {
        let cond = MarketCondition {
            trend: TrendState::Up,
            volatility: VolatilityState::Low,
            momentum: MomentumState::Oversold,
        };
        assert_eq!(cond.key(), "up|lo|os");
    }
}
