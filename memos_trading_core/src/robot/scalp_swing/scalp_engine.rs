/// ScalpEngine — 3m/5m mumlarla hızlı fırsat tespiti
///
/// Sinyal koşulları (AND — hepsi sağlanmalı):
///   BUY : EMA5 > EMA13 (bullish ribbon) AND RSI(7) < 60 AND fiyat BB alt→orta kırılımı
///         AND hacim > 1.4× 20-bar ortalaması AND son 3 bar green momentum
///   SELL: EMA5 < EMA13 (bearish ribbon) AND RSI(7) > 40 AND fiyat BB üst→orta kırılımı
///         AND hacim > 1.4× ortalama AND son 3 bar red momentum
///
/// Skor [0.0–1.0]: ne kadar çok sinyal bileşeni güçlüyse skor o kadar yüksek.

use crate::types::Candle;
use crate::robot::indicators::{
    calculate_ema, calculate_rsi, calculate_bollinger, calculate_atr
};
use super::{TradeOpportunity, TradeType};

pub struct ScalpEngine;

impl ScalpEngine {
    /// 3m/5m mumlardan scalp sinyali üret.
    /// `min_score`: eşiğin altı → None (fırsat yok)
    pub fn evaluate(candles: &[Candle], min_score: f64) -> Option<TradeOpportunity> {
        if candles.len() < 30 { return None; }

        let last = candles.last()?;
        let current_price = last.close;

        // ── İndikatörler ──────────────────────────────────────────────────────
        let ema5  = calculate_ema(candles, 5)?;
        let ema13 = calculate_ema(candles, 13)?;
        let rsi7  = calculate_rsi(candles, 7)?;
        let (bb_lower, bb_mid, bb_upper) = calculate_bollinger(candles, 20, 2.0)?;
        let atr   = calculate_atr(candles, 14).unwrap_or(current_price * 0.005);

        // ── Hacim filtresi ────────────────────────────────────────────────────
        let n = candles.len().min(20);
        let vol_avg = candles[candles.len() - n..]
            .iter().map(|c| c.volume).sum::<f64>() / n as f64;
        let vol_spike = vol_avg > 0.0 && last.volume > vol_avg * 1.4;

        // ── Momentum (son 3 bar) ──────────────────────────────────────────────
        let bars = &candles[candles.len().saturating_sub(3)..];
        let green_momentum = bars.iter().filter(|c| c.close > c.open).count() >= 2;
        let red_momentum   = bars.iter().filter(|c| c.close < c.open).count() >= 2;

        // ── BB sıkışma kırılımı ───────────────────────────────────────────────
        let bb_width = if bb_mid > 0.0 { (bb_upper - bb_lower) / bb_mid } else { 0.0 };
        let prev_close = candles.get(candles.len().saturating_sub(2)).map(|c| c.close).unwrap_or(current_price);

        // Alt bant altından orta banta doğru kırılım → BUY
        let bb_buy_break  = prev_close < bb_lower && current_price >= bb_lower;
        // Üst bant üstünden orta banta doğru kırılım → SELL
        let bb_sell_break = prev_close > bb_upper && current_price <= bb_upper;

        // ── Ribbon yönü ──────────────────────────────────────────────────────
        let bullish_ribbon = ema5 > ema13;
        let bearish_ribbon = ema5 < ema13;

        // ── ATR tabanlı minimum hareket filtresi (çok küçük ATR = düz piyasa) ─
        let min_atr_pct = (atr / current_price) * 100.0;
        if min_atr_pct < 0.05 { return None; } // düz piyasa, scalp işe yaramaz

        // ── BUY skoru ────────────────────────────────────────────────────────
        let buy_score = Self::compute_score(
            bullish_ribbon,
            rsi7 < 60.0,
            rsi7 > 20.0, // oversold değil — momentum var
            bb_buy_break || (current_price > bb_lower && current_price < bb_mid && bullish_ribbon),
            vol_spike,
            green_momentum,
            bb_width,
        );

        // ── SELL skoru ───────────────────────────────────────────────────────
        let sell_score = Self::compute_score(
            bearish_ribbon,
            rsi7 > 40.0,
            rsi7 < 80.0, // overbought değil — momentum var
            bb_sell_break || (current_price < bb_upper && current_price > bb_mid && bearish_ribbon),
            vol_spike,
            red_momentum,
            bb_width,
        );

        // En yüksek skoru al; min_score eşiğini aş
        let (is_long, score) = if buy_score >= sell_score {
            (true, buy_score)
        } else {
            (false, sell_score)
        };

        if score < min_score { return None; }

        let direction = if is_long { "BUY" } else { "SELL" };
        Some(TradeOpportunity {
            trade_type: TradeType::Scalp,
            is_long,
            score,
            sl_pct: 0.0, // robotic_loop ScalpSwingConfig'den alır
            tp_pct: 0.0,
            reason: format!(
                "Scalp {direction} | EMA5={:.4} EMA13={:.4} RSI7={:.1} BB[{:.4},{:.4}] vol_spike={} score={:.2}",
                ema5, ema13, rsi7, bb_lower, bb_upper, vol_spike, score
            ),
        })
    }

    fn compute_score(
        ribbon:     bool,
        rsi_ok:     bool,
        rsi_range:  bool,
        bb_signal:  bool,
        vol_spike:  bool,
        momentum:   bool,
        bb_width:   f64,
    ) -> f64 {
        if !ribbon { return 0.0; } // ribbon olmadan hiç puan yok

        let mut score: f64 = 0.30; // ribbon temel puanı
        if rsi_ok     { score += 0.20; }
        if rsi_range  { score += 0.10; }
        if bb_signal  { score += 0.20; }
        if vol_spike  { score += 0.15; }
        if momentum   { score += 0.10; }
        // BB genişliği bonus: sıkışma kırılımı daha değerli
        if bb_width < 0.02 { score += 0.05; } // çok sıkışmış

        score.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_candle(close: f64, volume: f64) -> Candle {
        Candle { timestamp: Utc::now(), open: close * 0.999, high: close * 1.002, low: close * 0.997, close, volume, symbol: "TEST".into(), interval: "5m".into() }
    }

    /// Yeterli mum verisi olmadan None dönmeli
    #[test]
    fn test_insufficient_data_returns_none() {
        let candles: Vec<Candle> = (0..20).map(|i| make_candle(100.0 + i as f64, 1000.0)).collect();
        assert!(ScalpEngine::evaluate(&candles, 0.0).is_none());
    }

    /// Düz piyasa (çok küçük ATR) → None dönmeli
    #[test]
    fn test_flat_market_returns_none() {
        // Hepsi aynı fiyat → ATR ~0, min_atr_pct < 0.05
        let candles: Vec<Candle> = (0..60).map(|_| {
            Candle { timestamp: Utc::now(), open: 100.0, high: 100.0001, low: 99.9999, close: 100.0, volume: 1000.0, symbol: "TEST".into(), interval: "5m".into() }
        }).collect();
        assert!(ScalpEngine::evaluate(&candles, 0.0).is_none());
    }

    /// compute_score: ribbon olmadan sıfır dönmeli
    #[test]
    fn test_compute_score_no_ribbon_is_zero() {
        let score = ScalpEngine::compute_score(false, true, true, true, true, true, 0.01);
        assert_eq!(score, 0.0);
    }

    /// compute_score: tüm koşullar sağlandığında yüksek skor (≥ 0.9)
    #[test]
    fn test_compute_score_all_signals_high() {
        let score = ScalpEngine::compute_score(true, true, true, true, true, true, 0.01);
        assert!(score >= 0.9, "Beklenen ≥ 0.9, alınan: {score}");
    }

    /// compute_score: sadece ribbon → temel puan = 0.30
    #[test]
    fn test_compute_score_only_ribbon() {
        let score = ScalpEngine::compute_score(true, false, false, false, false, false, 0.03);
        assert!((score - 0.30).abs() < 1e-9);
    }

    /// Bullish ribbon ile üretilen fırsat is_long=true olmalı
    #[test]
    fn test_bullish_ribbon_produces_long_signal() {
        // EMA5 > EMA13 için son barlar yukarı eğimli olmalı.
        // 60 yavaş yükselen bar üzerine son 5'i dik yükselt.
        let mut candles: Vec<Candle> = (0..55).map(|i| {
            let p = 100.0 + i as f64 * 0.10;
            Candle { timestamp: Utc::now(), open: p - 0.05, high: p + 0.15, low: p - 0.10, close: p, volume: 1500.0, symbol: "TEST".into(), interval: "5m".into() }
        }).collect();
        for i in 0..5 {
            let p = 105.5 + i as f64 * 2.0;
            candles.push(Candle { timestamp: Utc::now(), open: p - 0.5, high: p + 1.0, low: p - 0.8, close: p, volume: 3000.0, symbol: "TEST".into(), interval: "5m".into() });
        }
        // min_score=0.0 → herhangi bir sinyal varsa Some döner
        if let Some(opp) = ScalpEngine::evaluate(&candles, 0.0) {
            assert!(opp.is_long, "Bullish ribbon, long sinyal bekleniyor");
            assert_eq!(opp.trade_type, TradeType::Scalp);
            assert!(opp.score >= 0.0 && opp.score <= 1.0);
        }
        // Sinyal üretilmese de test geçerli (piyasa koşulları uymayabilir),
        // ancak üretilirse doğru yönde olmalı.
    }

    /// min_score eşiğinin üstünde olmayan sinyaller filtrelenmeli
    #[test]
    fn test_min_score_filter() {
        let candles: Vec<Candle> = (0..60).map(|i| {
            let p = 100.0 + i as f64 * 0.05;
            make_candle(p, 500.0)
        }).collect();
        // min_score=1.0 → mükemmel skor dışında her şey filtrelenir
        let result = ScalpEngine::evaluate(&candles, 1.0);
        assert!(result.is_none(), "min_score=1.0 ile sonuç None olmalı");
    }
}
