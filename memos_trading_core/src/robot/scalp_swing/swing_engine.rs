/// SwingEngine — 4h/1D mumlarla orta-vade fırsat tespiti
///
/// Sinyal koşulları (AND):
///   BUY : EMA21 > EMA55 (uptrend) AND MACD histogram pozitife döndü
///         AND ADX > min_adx (güçlü trend) AND fiyat EMA21 üstünde
///         AND son kapanış önceki direnci kırdı (higher-high yapısı)
///   SELL: EMA21 < EMA55 AND MACD histogram negatife döndü
///         AND ADX > min_adx AND fiyat EMA21 altında
///         AND son kapanış önceki desteği kırdı (lower-low yapısı)
///
/// Skor [0.0–1.0]: ne kadar çok koşul sağlanıyorsa o kadar yüksek güven.

use crate::types::Candle;
use crate::robot::indicators::{
    calculate_ema, calculate_macd, calculate_adx, calculate_atr
};
use super::{TradeOpportunity, TradeType};

pub struct SwingEngine;

impl SwingEngine {
    /// 4h/1D mumlardan swing sinyali üret.
    pub fn evaluate(candles: &[Candle], min_adx: f64, min_score: f64) -> Option<TradeOpportunity> {
        if candles.len() < 60 { return None; }

        let last  = candles.last()?;
        let _prev = candles.get(candles.len().saturating_sub(2))?;
        let current_price = last.close;

        // ── İndikatörler ──────────────────────────────────────────────────────
        let ema21 = calculate_ema(candles, 21)?;
        let ema55 = calculate_ema(candles, 55)?;
        // MACD(12,26,9)
        let (macd_line, _signal, histogram) = calculate_macd(candles, 12, 26, 9)?;
        // ADX(14): (adx, plus_di, minus_di)
        let (adx, plus_di, minus_di) = calculate_adx(candles, 14)?;
        let atr = calculate_atr(candles, 14).unwrap_or(current_price * 0.02);

        // ── MACD histogram yönü değişimi ──────────────────────────────────────
        // Son iki bar histogram: negatiften pozitife → BUY flip
        let n = candles.len();
        let prev_hist = Self::prev_macd_histogram(candles);
        let macd_buy_flip  = prev_hist < 0.0 && histogram > 0.0;
        let macd_sell_flip = prev_hist > 0.0 && histogram < 0.0;
        // Flip olmasa da pozitif bölgede yükseliş varsa bonus
        let macd_buy_rising  = histogram > 0.0 && histogram > prev_hist;
        let macd_sell_falling= histogram < 0.0 && histogram < prev_hist;

        // ── Yapısal break (Higher-High / Lower-Low) ───────────────────────────
        let lookback = candles.len().min(20);
        let recent = &candles[n.saturating_sub(lookback)..n.saturating_sub(1)];
        let swing_high = recent.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let swing_low  = recent.iter().map(|c| c.low ).fold(f64::INFINITY,     f64::min);
        let higher_high = last.close > swing_high;
        let lower_low   = last.close < swing_low;

        // ── Trend hizası ──────────────────────────────────────────────────────
        let uptrend   = ema21 > ema55 && current_price > ema21;
        let downtrend = ema21 < ema55 && current_price < ema21;

        // ── DI farkı (yön gücü) ───────────────────────────────────────────────
        let di_bull = plus_di > minus_di;
        let di_bear = minus_di > plus_di;

        // ── ATR tabanlı minimum hareket: swing için en az %0.5 ATR gerekli ───
        let atr_pct = (atr / current_price) * 100.0;
        if atr_pct < 0.3 { return None; }

        // ── BUY skoru ────────────────────────────────────────────────────────
        let buy_score = Self::compute_score(
            uptrend,
            adx >= min_adx,
            di_bull,
            macd_buy_flip || macd_buy_rising,
            higher_high,
            macd_line > 0.0,
        );

        // ── SELL skoru ───────────────────────────────────────────────────────
        let sell_score = Self::compute_score(
            downtrend,
            adx >= min_adx,
            di_bear,
            macd_sell_flip || macd_sell_falling,
            lower_low,
            macd_line < 0.0,
        );

        let (is_long, score) = if buy_score >= sell_score {
            (true, buy_score)
        } else {
            (false, sell_score)
        };

        if score < min_score { return None; }

        let direction = if is_long { "BUY" } else { "SELL" };
        Some(TradeOpportunity {
            trade_type: TradeType::Swing,
            is_long,
            score,
            sl_pct: 0.0,
            tp_pct: 0.0,
            reason: format!(
                "Swing {direction} | EMA21={:.4} EMA55={:.4} ADX={:.1} MACD_hist={:.5} di+={:.1} di-={:.1} score={:.2}",
                ema21, ema55, adx, histogram, plus_di, minus_di, score
            ),
        })
    }

    fn prev_macd_histogram(candles: &[Candle]) -> f64 {
        if candles.len() < 35 { return 0.0; }
        let prev_slice = &candles[..candles.len().saturating_sub(1)];
        calculate_macd(prev_slice, 12, 26, 9)
            .map(|(_, _, h)| h)
            .unwrap_or(0.0)
    }

    fn compute_score(
        trend_aligned: bool,
        adx_ok:        bool,
        di_ok:         bool,
        macd_signal:   bool,
        structure:     bool,
        macd_side:     bool,
    ) -> f64 {
        if !trend_aligned { return 0.0; }

        let mut score: f64 = 0.25; // EMA ribbon temel puanı
        if adx_ok     { score += 0.25; } // ADX güçlü trend zorunlu
        if di_ok      { score += 0.15; }
        if macd_signal{ score += 0.20; }
        if structure  { score += 0.10; }
        if macd_side  { score += 0.05; }

        score.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_candle(open: f64, close: f64, high: f64, low: f64) -> Candle {
        Candle { timestamp: Utc::now(), open, high, low, close, volume: 2000.0, symbol: "TEST".into(), interval: "4h".into() }
    }

    /// Yetersiz veri → None
    #[test]
    fn test_insufficient_data_returns_none() {
        let candles: Vec<Candle> = (0..50).map(|i| {
            let p = 100.0 + i as f64;
            make_candle(p - 0.5, p, p + 0.5, p - 1.0)
        }).collect();
        assert!(SwingEngine::evaluate(&candles, 20.0, 0.0).is_none());
    }

    /// compute_score: trend_aligned=false → sıfır
    #[test]
    fn test_compute_score_no_trend_is_zero() {
        let score = SwingEngine::compute_score(false, true, true, true, true, true);
        assert_eq!(score, 0.0);
    }

    /// compute_score: tüm koşullar → yüksek skor (≥ 0.9)
    #[test]
    fn test_compute_score_all_true_high() {
        let score = SwingEngine::compute_score(true, true, true, true, true, true);
        assert!(score >= 0.9, "Beklenen ≥ 0.9, alınan: {score}");
    }

    /// compute_score: sadece trend_aligned → temel puan = 0.25
    #[test]
    fn test_compute_score_only_trend() {
        let score = SwingEngine::compute_score(true, false, false, false, false, false);
        assert!((score - 0.25).abs() < 1e-9);
    }

    /// min_score=1.0 ile herhangi bir veri → None (1.0 ulaşılamaz eşik)
    #[test]
    fn test_min_score_filter_blocks_all() {
        let candles: Vec<Candle> = (0..80).map(|i| {
            let p = 100.0 + i as f64 * 0.3;
            make_candle(p - 0.1, p, p + 0.2, p - 0.3)
        }).collect();
        assert!(SwingEngine::evaluate(&candles, 20.0, 1.0).is_none());
    }

    /// Uptrend konfigürasyonunda üretilen sinyal is_long=true olmalı
    #[test]
    fn test_uptrend_signal_is_long() {
        // EMA21 > EMA55 için uzun süreli yükseliş trendi
        let mut candles: Vec<Candle> = (0..70).map(|i| {
            let p = 100.0 + i as f64 * 1.5;
            make_candle(p - 0.5, p, p + 1.0, p - 1.0)
        }).collect();
        // Son barı güçlü yukarı kır
        let last_p = 205.0;
        candles.push(make_candle(last_p - 1.0, last_p + 5.0, last_p + 6.0, last_p - 1.5));

        if let Some(opp) = SwingEngine::evaluate(&candles, 0.0, 0.0) {
            assert!(opp.is_long, "Uptrend → long sinyal bekleniyor");
            assert_eq!(opp.trade_type, TradeType::Swing);
            assert!(opp.score >= 0.0 && opp.score <= 1.0);
        }
    }

    /// Downtrend konfigürasyonunda üretilen sinyal is_long=false olmalı
    #[test]
    fn test_downtrend_signal_is_short() {
        let mut candles: Vec<Candle> = (0..70).map(|i| {
            let p = 300.0 - i as f64 * 1.5;
            make_candle(p + 0.5, p, p + 1.0, p - 1.0)
        }).collect();
        let last_p = 100.0;
        candles.push(make_candle(last_p + 1.0, last_p - 5.0, last_p + 1.5, last_p - 6.0));

        if let Some(opp) = SwingEngine::evaluate(&candles, 0.0, 0.0) {
            assert!(!opp.is_long, "Downtrend → short sinyal bekleniyor");
        }
    }
}
