// advanced_risk/leverage_v2.rs - Otonom Kaldıraç Yönetimi
use crate::prelude::*;

use crate::core::types::{Market, Signal};
use crate::robot::signal_evaluator::TrendBias;

pub struct LeverageEngine;

impl LeverageEngine {
    /// 7x–10x aralığında otonom kaldıraç belirler.
    /// robotic_loop içindeki tüm otonom çarpan mantığını buraya topladık.
    pub fn calculate(
        base: f64,
        max: f64,
        market: Market,
        htf_bias: Option<TrendBias>,
        signal: &Signal,
        atr_pct: Option<f64>,
        dd_pct: f64,
        session_rr: f64,
        loss_streak: usize,
        open_count: usize,
    ) -> f64 {
        if matches!(market, Market::Spot) { return 1.0; }

        let mut lev = base;

        // 1. HTF Onayı (+1.0x)
        match (htf_bias, signal) {
            (Some(TrendBias::Bullish), Signal::Buy) | (Some(TrendBias::Bearish), Signal::Sell) => lev += 1.0,
            _ => {}
        }

        // 2. Volatilite Cezası (ATR)
        if let Some(atr) = atr_pct {
            if atr > 2.5 { lev -= 2.0; }
            else if atr > 1.5 { lev -= 1.0; }
        }

        // 3. Finansal Sağlık (Drawdown)
        if dd_pct > 10.0 { return base; } // Koruma modu
        else if dd_pct > 5.0 { lev -= 1.0; }

        // 4. Performans (RR & Streak)
        if session_rr > 2.0 { lev += 0.5; }
        if loss_streak >= 3 { lev -= 1.5; }

        // 5. Risk Yoğunluğu (Açık Pozisyon Sayısı)
        if open_count >= 3 { lev -= 1.0; }

        lev.clamp(base, max)
    }
}
