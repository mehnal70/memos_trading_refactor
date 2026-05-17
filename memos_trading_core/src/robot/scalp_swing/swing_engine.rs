// robot/scalp_swing/swing_engine.rs - Orta Zaman Dilimi (4h/1D) Trend Avcısı

use crate::core::types::Candle;
use crate::core::indicators::{
    calculate_ema, calculate_macd, calculate_adx, calculate_atr
};
use super::{TradeOpportunity, TradeType};

pub struct SwingEngine;

impl SwingEngine {
    /// 4h/1D mum verilerinden otonom swing sinyali üretir ve puanlar.
    pub fn evaluate(candles: &[Candle], min_adx: f64, min_score: f64) -> Option<TradeOpportunity> {
        if candles.len() < 60 { return None; }

        let last = candles.last()?;
        let current_price = last.close;

        // --- 1. TEKNİK ANALİZ BİRİMİ (yeni Vec<f64>/MacdOutput API) ---
        let ema21 = *calculate_ema(candles, 21).last()?;
        let ema55 = *calculate_ema(candles, 55).last()?;
        let macd_out = calculate_macd(candles, 12, 26, 9);
        let (macd_line, _signal, histogram) = macd_out.last_lines()?;
        let adx = *calculate_adx(candles, 14).last()?;
        let atr = calculate_atr(candles, 14).last().copied().unwrap_or(current_price * 0.02);

        // --- 2. MOMENTUM VE YAPI ANALİZİ ---
        let prev_hist = Self::prev_macd_histogram(candles);
        let macd_buy_flip  = prev_hist < 0.0 && histogram > 0.0;
        let macd_sell_flip = prev_hist > 0.0 && histogram < 0.0;

        // Market Structure (Zirve/Dip Analizi)
        let n = candles.len();
        let lookback = 20.min(n - 1);
        let recent = &candles[n.saturating_sub(lookback)..n.saturating_sub(1)];
        let swing_high = recent.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let swing_low  = recent.iter().map(|c| c.low ).fold(f64::INFINITY, f64::min);

        let higher_high = last.close > swing_high;
        let lower_low   = last.close < swing_low;

        // --- 3. TREND VE VOLATİLİTE FİLTRELERİ ---
        let uptrend   = ema21 > ema55 && current_price > ema21;
        let downtrend = ema21 < ema55 && current_price < ema21;

        let atr_pct = (atr / current_price) * 100.0;
        if atr_pct < 0.3 { return None; } // "Düşük Oynaklık" Guard

        // --- 4. OTONOM SKORLAMA ---
        let buy_score = Self::compute_score(
            uptrend, adx >= min_adx,
            macd_buy_flip || (histogram > 0.0 && histogram > prev_hist),
            higher_high, macd_line > 0.0,
        );

        let sell_score = Self::compute_score(
            downtrend, adx >= min_adx,
            macd_sell_flip || (histogram < 0.0 && histogram < prev_hist),
            lower_low, macd_line < 0.0,
        );

        let (is_long, score) = if buy_score >= sell_score { (true, buy_score) } else { (false, sell_score) };

        if score < min_score { return None; }

        Some(TradeOpportunity {
            trade_type: TradeType::Swing,
            is_long,
            score,
            sl_pct: 0.0,
            tp_pct: 0.0,
            reason: format!("Swing {} | ADX={:.1} Score={:.2} HH={}",
                if is_long { "BUY" } else { "SELL" }, adx, score, higher_high),
        })
    }

    fn prev_macd_histogram(candles: &[Candle]) -> f64 {
        if candles.len() < 35 { return 0.0; }
        let prev_slice = &candles[..candles.len().saturating_sub(1)];
        calculate_macd(prev_slice, 12, 26, 9)
            .last_lines()
            .map(|(_, _, h)| h)
            .unwrap_or(0.0)
    }

    /// Skor: trend ana koşul; ADX, MACD sinyali, yapı (HH/LL), MACD tarafı katkı sağlar.
    fn compute_score(
        trend: bool, adx_ok: bool,
        macd_signal: bool, structure: bool, macd_side: bool
    ) -> f64 {
        if !trend { return 0.0; }

        let mut score: f64 = 0.25;
        if adx_ok      { score += 0.25; }
        if macd_signal { score += 0.25; } // di_ok kaldırıldı, ağırlığı buraya verildi
        if structure   { score += 0.15; }
        if macd_side   { score += 0.10; }

        score.clamp(0.0, 1.0)
    }
}
