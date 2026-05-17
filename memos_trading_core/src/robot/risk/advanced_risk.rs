// robot/risk/advanced_risk.rs - Otonom Risk ve Kaldıraç Yönetim Merkezi

use crate::types::{Signal, Market, RiskParams};
use crate::robot::signal_evaluator::TrendBias;

pub struct AdvancedRiskManager;

impl AdvancedRiskManager {
    /// §86.1: Dinamik Kaldıraç Hesaplama Motoru
    /// HTF Onayı, Volatilite ve Performans metriklerini harmanlayarak otonom kaldıraç belirler.
    pub fn compute_dynamic_leverage(
        base: f64,
        max: f64,
        market: Market,
        htf_bias: Option<TrendBias>,
        signal: &Signal,
        atr_pct: Option<f64>,
        dd_pct: f64,
        hyperopt_score: f64,
        session_rr: f64,
        loss_streak: usize,
        open_count: usize,
    ) -> f64 {
        if matches!(market, Market::Spot) { return 1.0; }
        
        let mut lev = base;

        // 1. Stratejik Onay Boost (+1.0x)
        let aligned = match (htf_bias, signal) {
            (Some(TrendBias::Bullish), Signal::Buy) => true,
            (Some(TrendBias::Bearish), Signal::Sell) => true,
            _ => false,
        };
        if aligned { lev += 1.0; }

        // 2. Volatilite Freni (-1.0x / -2.0x)
        if let Some(atr) = atr_pct {
            if      atr > 2.5 { lev -= 2.0; }
            else if atr > 1.5 { lev -= 1.0; }
        }

        // 3. Drawdown ve Performans Koruması
        if dd_pct > 10.0 { lev = base; } // Ciddi koruma modu
        else if dd_pct > 5.0 { lev -= 1.0; }

        if hyperopt_score > 0.70 { lev += 0.5; }
        if session_rr > 2.0 { lev += 0.5; }

        // 4. Seri Kayıp ve Konsantrasyon Koruması
        if      loss_streak >= 5 { lev = base; }
        else if loss_streak >= 3 { lev -= 1.5; }

        if open_count >= 5 { lev -= 2.0; }

        lev.clamp(base, max)
    }

    /// §52.1: Kelly Kriteri ve ML Güven Ölçekleme
    /// İstatistiksel avantajı (Edge) pozisyon büyüklüğüne dönüştürür.
    pub fn calculate_kelly_multiplier(
        session_closed: usize,
        session_wins: usize,
        session_profit: f64,
        session_loss: f64,
        ml_confidence: f64, // 0.0 - 1.0
    ) -> f64 {
        let mut multiplier = 1.0;

        // ML Confidence Sizing (%75 - %125 arası ölçekleme)
        multiplier *= 0.75 + (ml_confidence * 0.50);

        // Half-Kelly Otonomisi (Min 20 işlem tecrübesi gerekir)
        if session_closed >= 20 {
            let wr = session_wins as f64 / session_closed as f64;
            let losses = session_closed.saturating_sub(session_wins);
            
            let avg_win = if session_wins > 0 { session_profit / session_wins as f64 } else { 0.0 };
            let avg_loss = if losses > 0 { session_loss / losses as f64 } else { 1.0 };
            
            let wlr = if avg_loss > 1e-9 { avg_win / avg_loss } else { 1.0 };
            let q = 1.0 - wr;
            
            if wlr > 1e-9 {
                let raw_f = ((wlr * wr) - q) / wlr;
                let half_kelly = raw_f.clamp(-1.0, 1.0) * 0.5;
                multiplier *= (1.0 + half_kelly).clamp(0.5, 1.5);
            }
        }

        multiplier
    }

    /// §53.1: Savunma Hattı (Defensive Scaling)
    /// Drawdown ve Kayıp serisi durumunda pozisyonu logaritmik küçültür.
    pub fn apply_defensive_scaling(
        base_qty: f64,
        loss_streak: usize,
        current_dd_pct: f64,
        vol_ratio: f64,
    ) -> f64 {
        let mut qty = base_qty;

        // 1. Loss Streak Freni (0.80^n)
        if loss_streak >= 5 {
            let exponent = ((loss_streak - 4) / 2).max(1) as i32;
            qty *= 0.80_f64.powi(exponent).max(0.25);
        }

        // 2. Kademeli Drawdown Bariyeri
        if      current_dd_pct > 20.0 { return 0.0; } // HALT: Yeni işleme izin verme
        else if current_dd_pct > 15.0 { qty *= 0.5; }

        // 3. Kaotik Volatilite Koruması
        if vol_ratio > 2.0 { qty *= 0.5; }

        qty
    }

    /// §57.1: ATR Tabanlı Dinamik SL/TP
    /// Sabit yüzdeleri piyasa oynaklığına göre otonom esnetir.
    pub fn calculate_atr_stops(
        entry_price: f64,
        is_long: bool,
        atr_pct: f64,
        sl_mult: f64,
        tp_mult: f64,
        min_rr: f64,
        max_sl_pct: f64,
    ) -> (f64, f64) {
        let sl_pct = (atr_pct * sl_mult).min(max_sl_pct).max(0.1);
        let tp_pct = (atr_pct * tp_mult).max(sl_pct * min_rr).min(20.0);

        if is_long {
            (entry_price * (1.0 - sl_pct / 100.0), entry_price * (1.0 + tp_pct / 100.0))
        } else {
            (entry_price * (1.0 + sl_pct / 100.0), entry_price * (1.0 - tp_pct / 100.0))
        }
    }
}
