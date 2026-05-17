// robot/scalp_swing/scalp_engine.rs - Mikro Zaman Dilimi (3m/5m) Fırsat Avcısı

use crate::core::types::Candle;
use crate::core::indicators::{
    calculate_ema, calculate_rsi, calculate_bollinger, calculate_atr
};
use super::{TradeOpportunity, TradeType};

pub struct ScalpEngine;

impl ScalpEngine {
    /// 3m/5m mumlarından scalp sinyali üretir ve otonom puanlar.
    pub fn evaluate(candles: &[Candle], min_score: f64) -> Option<TradeOpportunity> {
        if candles.len() < 30 { return None; }

        let last = candles.last()?;
        let current_price = last.close;

        // --- 1. TEKNİK ANALİZ BİRİMİ (yeni Vec<f64>/struct API) ---
        let ema5  = *calculate_ema(candles, 5).last()?;
        let ema13 = *calculate_ema(candles, 13).last()?;
        let rsi7  = *calculate_rsi(candles, 7).last()?;
        let bb    = calculate_bollinger(candles, 20, 2.0);
        let bb_upper = *bb.upper.last()?;
        let bb_mid   = *bb.middle.last()?;
        let bb_lower = *bb.lower.last()?;
        let atr   = calculate_atr(candles, 14).last().copied().unwrap_or(current_price * 0.005);

        // --- 2. HACİM VE MOMENTUM DENETİMİ ---
        let n = candles.len().min(20);
        let vol_avg = candles[candles.len() - n..].iter().map(|c| c.volume).sum::<f64>() / n as f64;
        let vol_spike = vol_avg > 0.0 && last.volume > vol_avg * 1.4;

        let bars = &candles[candles.len().saturating_sub(3)..];
        let green_momentum = bars.iter().filter(|c| c.close > c.open).count() >= 2;
        let red_momentum   = bars.iter().filter(|c| c.close < c.open).count() >= 2;

        // --- 3. REJİM VE SIKIŞMA ANALİZİ ---
        let bb_width = if bb_mid > 0.0 { (bb_upper - bb_lower) / bb_mid } else { 0.0 };
        let prev_close = candles.get(candles.len().saturating_sub(2)).map(|c| c.close).unwrap_or(current_price);

        let bb_buy_break  = prev_close < bb_lower && current_price >= bb_lower;
        let bb_sell_break = prev_close > bb_upper && current_price <= bb_upper;

        let bullish_ribbon = ema5 > ema13;
        let bearish_ribbon = ema5 < ema13;

        let min_atr_pct = (atr / current_price) * 100.0;
        if min_atr_pct < 0.05 { return None; } // "Ölü Piyasa" Koruması

        // --- 4. OTONOM SKORLAMA ---
        let buy_score = Self::compute_score(
            candles, bullish_ribbon, rsi7 < 60.0, rsi7 > 20.0,
            bb_buy_break || (current_price > bb_lower && current_price < bb_mid && bullish_ribbon),
            vol_spike, green_momentum, bb_width,
        );

        let sell_score = Self::compute_score(
            candles, bearish_ribbon, rsi7 > 40.0, rsi7 < 80.0,
            bb_sell_break || (current_price < bb_upper && current_price > bb_mid && bearish_ribbon),
            vol_spike, red_momentum, bb_width,
        );

        let (is_long, score) = if buy_score >= sell_score { (true, buy_score) } else { (false, sell_score) };

        if score < min_score { return None; }

        Some(TradeOpportunity {
            trade_type: TradeType::Scalp,
            is_long,
            score,
            sl_pct: 0.0,
            tp_pct: 0.0,
            reason: format!("Scalp {} | RSI={:.1} BB_Width={:.3} score={:.2}",
                if is_long { "BUY" } else { "SELL" }, rsi7, bb_width, score),
        })
    }

    fn compute_score(
        _candles: &[Candle], ribbon: bool, rsi_ok: bool, rsi_range: bool, bb_signal: bool,
        vol_spike: bool, momentum: bool, bb_width: f64
    ) -> f64 {
        if !ribbon { return 0.0; }

        let mut score: f64 = 0.30;
        if rsi_ok     { score += 0.20; }
        if rsi_range  { score += 0.10; }
        if bb_signal  { score += 0.20; }
        if vol_spike  { score += 0.15; }
        if momentum   { score += 0.10; }
        if bb_width < 0.02 { score += 0.05; }

        score.clamp(0.0, 1.0)
    }
}
