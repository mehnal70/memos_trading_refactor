/// Scalp & Swing Trade Engine
///
/// Mevcut robotic loop'a dokunmadan çakışmasız çalışan kısa-vade fırsat motoru.
///
/// - ScalpEngine  : 3m mumlar + EMA(5/13) + RSI(7) + Bollinger sıkışma; SL ~%0.4, TP ~%0.8
/// - SwingEngine  : 4h mumlar + EMA(21/55) + MACD + ADX; SL ~%2.5, TP ~%5.0
/// - SlotGuard    : sembol başına max 1 scalp + 1 swing pozisyon; çakışma engeli
/// - ModeSelector : ADX + volatiliteye göre otomatik mod seçimi

// robot/scalp_swing/mod.rs - Kısa Vade Fırsat Motoru ve Otonom Konfigürasyon


// robot/scalp_swing/mod.rs - Çok Kanallı Taktik İnfaz Merkezi (Srivastava ATP)

pub mod scalp_engine;
pub mod swing_engine;
pub mod slot_guard;
pub mod mode_selector;

pub use scalp_engine::ScalpEngine;
pub use swing_engine::SwingEngine;
pub use slot_guard::{SlotGuard, OpenSlot};
pub use mode_selector::{TradeMode, ModeSelector};

use serde::{Deserialize, Serialize};

// --- 1. TEMEL TİPLER VE ENUMLAR ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TradeType { Regular, Scalp, Swing }

impl Default for TradeType { fn default() -> Self { Self::Regular } }

impl TradeType {
    pub fn label(&self) -> &'static str {
        match self {
            TradeType::Regular => "REG",
            TradeType::Scalp   => "SCP",
            TradeType::Swing   => "SWG",
        }
    }
}

// --- 2. KONFİGÜRASYON VE SINIRLAR ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamBounds {
    pub min: f64,
    pub max: f64,
    pub adjust_every_n: usize,
}

impl ParamBounds {
    pub fn clamp(&self, v: f64) -> f64 { v.clamp(self.min, self.max) }
    pub fn enabled(&self) -> bool { self.adjust_every_n > 0 }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScalpSwingConfig {
    pub scalp_enabled: bool, pub swing_enabled: bool,
    pub scalp_interval: String, pub swing_interval: String,
    pub scalp_sl_pct: f64, pub scalp_tp_pct: f64,
    pub swing_sl_pct: f64, pub swing_tp_pct: f64,
    pub scalp_leverage: f64, pub swing_leverage: f64,
    pub scalp_budget_pct: Option<f64>, pub swing_budget_pct: Option<f64>,
    pub commission_pct: Option<f64>, pub spread_pct: Option<f64>,
    pub slippage_pct: Option<f64>, pub max_daily_loss_pct: f64,
    pub max_notional_usd: Option<f64>, pub scalp_min_score: f64,
    pub swing_min_adx: f64, pub swing_min_score: f64,
    pub max_scalp_per_symbol: usize, pub max_swing_per_symbol: usize,
    pub scalp_active_hours: [u32; 2], pub autonomous_tuning: bool,
    pub scalp_sl_bounds: ParamBounds, pub scalp_tp_bounds: ParamBounds,
    pub swing_sl_bounds: ParamBounds, pub swing_tp_bounds: ParamBounds,
    pub scalp_lev_bounds: ParamBounds, pub swing_lev_bounds: ParamBounds,
    pub scalp_score_bounds: ParamBounds, pub swing_score_bounds: ParamBounds,
}

// --- 3. İSTATİSTİK VE OTONOM AYARLAMA ---

#[derive(Debug, Clone, Default)]
pub struct ScalpSwingStats {
    pub total_closed: usize, pub wins: usize,
    pub total_pnl: f64, pub total_win_pnl: f64, pub total_loss_pnl: f64,
    pub loss_streak: usize, pub max_loss_streak: usize, pub last_tune_at: usize,
}

impl ScalpSwingStats {
    pub fn win_rate(&self) -> f64 { if self.total_closed == 0 { 0.5 } else { self.wins as f64 / self.total_closed as f64 } }
    pub fn profit_factor(&self) -> f64 {
        let loss = self.total_loss_pnl.abs();
        if loss < f64::EPSILON { 3.0 } else { self.total_win_pnl / loss }
    }
}

/// Otonom konfig ayarlayıcısı. `stats` son N kapanışın özetini taşır
/// (win_rate + profit_factor + loss_streak). Karşılığında SL/TP/leverage
/// değerlerini ilgili `ParamBounds` aralığında yumuşakça kaydırır.
///
/// Kural seti (Scalp ve Swing aynı omurga, eşikler farklı):
///   - win_rate düşük (<0.40) → SL daralt (×0.90), leverage daralt (×0.85).
///     "Hatalı sinyaller iniyor; pozisyon büyüklüğü ile zararı sınırla."
///   - win_rate yüksek (>0.60) + profit_factor>1.3 → TP genişlet (×1.08),
///     leverage genişlet (×1.10). "Edge tutuyor; kâr alanını uzat."
///   - profit_factor < 0.80 (sert kayıp) → leverage agresif daralt (×0.70).
///   - loss_streak >= 3 → leverage agresif daralt (×0.80) (overlap ok).
///
/// Tüm değişiklikler `clamp(bounds)` ile sınırlı; pratik değişim < 0.01
/// veya < 1e-3 ise log kaydı eklenmez (gereksiz spam).
pub fn auto_tune(stats: &ScalpSwingStats, trade_type: TradeType, cfg: &mut ScalpSwingConfig) -> Vec<String> {
    let mut changes = Vec::new();
    let wr = stats.win_rate();
    let pf = stats.profit_factor();
    match trade_type {
        TradeType::Scalp => {
            let b_sl  = cfg.scalp_sl_bounds.clone();
            let b_tp  = cfg.scalp_tp_bounds.clone();
            let b_lev = cfg.scalp_lev_bounds.clone();
            if wr < 0.40 {
                let new_sl = b_sl.clamp(cfg.scalp_sl_pct * 0.90);
                if (new_sl - cfg.scalp_sl_pct).abs() > 0.01 {
                    cfg.scalp_sl_pct = new_sl;
                    changes.push("SCP SL Tightened".to_string());
                }
                let new_lev = b_lev.clamp(cfg.scalp_leverage * 0.85);
                if (new_lev - cfg.scalp_leverage).abs() > 1e-3 {
                    cfg.scalp_leverage = new_lev;
                    changes.push("SCP Leverage Reduced".to_string());
                }
            } else if wr > 0.60 && pf > 1.3 {
                let new_tp = b_tp.clamp(cfg.scalp_tp_pct * 1.08);
                if (new_tp - cfg.scalp_tp_pct).abs() > 0.01 {
                    cfg.scalp_tp_pct = new_tp;
                    changes.push("SCP TP Widened".to_string());
                }
                let new_lev = b_lev.clamp(cfg.scalp_leverage * 1.10);
                if (new_lev - cfg.scalp_leverage).abs() > 1e-3 {
                    cfg.scalp_leverage = new_lev;
                    changes.push("SCP Leverage Raised".to_string());
                }
            }
            // Profit factor / loss streak ortak agresif daralma kuralı
            if pf < 0.80 || stats.loss_streak >= 3 {
                let new_lev = b_lev.clamp(cfg.scalp_leverage * 0.70);
                if (new_lev - cfg.scalp_leverage).abs() > 1e-3 {
                    cfg.scalp_leverage = new_lev;
                    changes.push("SCP Leverage Cut (PF/Streak)".to_string());
                }
            }
        }
        TradeType::Swing => {
            let b_sl  = cfg.swing_sl_bounds.clone();
            let b_tp  = cfg.swing_tp_bounds.clone();
            let b_lev = cfg.swing_lev_bounds.clone();
            if wr < 0.40 {
                let new_sl = b_sl.clamp(cfg.swing_sl_pct * 0.90);
                if (new_sl - cfg.swing_sl_pct).abs() > 0.01 {
                    cfg.swing_sl_pct = new_sl;
                    changes.push("SWG SL Tightened".to_string());
                }
                let new_lev = b_lev.clamp(cfg.swing_leverage * 0.85);
                if (new_lev - cfg.swing_leverage).abs() > 1e-3 {
                    cfg.swing_leverage = new_lev;
                    changes.push("SWG Leverage Reduced".to_string());
                }
            } else if wr > 0.60 && pf > 1.3 {
                let new_tp = b_tp.clamp(cfg.swing_tp_pct * 1.08);
                if (new_tp - cfg.swing_tp_pct).abs() > 0.01 {
                    cfg.swing_tp_pct = new_tp;
                    changes.push("SWG TP Widened".to_string());
                }
                let new_lev = b_lev.clamp(cfg.swing_leverage * 1.10);
                if (new_lev - cfg.swing_leverage).abs() > 1e-3 {
                    cfg.swing_leverage = new_lev;
                    changes.push("SWG Leverage Raised".to_string());
                }
            }
            if pf < 0.80 || stats.loss_streak >= 3 {
                let new_lev = b_lev.clamp(cfg.swing_leverage * 0.70);
                if (new_lev - cfg.swing_leverage).abs() > 1e-3 {
                    cfg.swing_leverage = new_lev;
                    changes.push("SWG Leverage Cut (PF/Streak)".to_string());
                }
            }
        }
        _ => {}
    }
    changes
}

// --- 4. ÇIKTI YAPISI ---

#[derive(Debug, Clone)]
pub struct TradeOpportunity {
    pub trade_type: TradeType, pub is_long: bool, pub score: f64,
    pub sl_pct: f64, pub tp_pct: f64, pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test fixture: 2.0 leverage, geniş bounds [1.0, 10.0] ile başlangıç config.
    fn make_cfg() -> ScalpSwingConfig {
        let bounds = ParamBounds { min: 0.1, max: 10.0, adjust_every_n: 1 };
        let lev_bounds = ParamBounds { min: 1.0, max: 10.0, adjust_every_n: 1 };
        ScalpSwingConfig {
            scalp_enabled: true, swing_enabled: true,
            scalp_interval: "3m".into(), swing_interval: "4h".into(),
            scalp_sl_pct: 0.40, scalp_tp_pct: 0.80,
            swing_sl_pct: 2.50, swing_tp_pct: 5.00,
            scalp_leverage: 2.0, swing_leverage: 2.0,
            scalp_budget_pct: None, swing_budget_pct: None,
            commission_pct: None, spread_pct: None, slippage_pct: None,
            max_daily_loss_pct: 5.0, max_notional_usd: None,
            scalp_min_score: 0.5, swing_min_adx: 25.0, swing_min_score: 0.5,
            max_scalp_per_symbol: 1, max_swing_per_symbol: 1,
            scalp_active_hours: [0, 23], autonomous_tuning: true,
            scalp_sl_bounds: bounds.clone(), scalp_tp_bounds: bounds.clone(),
            swing_sl_bounds: bounds.clone(), swing_tp_bounds: bounds.clone(),
            scalp_lev_bounds: lev_bounds.clone(), swing_lev_bounds: lev_bounds.clone(),
            scalp_score_bounds: bounds.clone(), swing_score_bounds: bounds,
        }
    }

    fn stats(wins: usize, losses: usize, total_win_pnl: f64, total_loss_pnl: f64, streak: usize) -> ScalpSwingStats {
        ScalpSwingStats {
            total_closed: wins + losses, wins,
            total_pnl: total_win_pnl - total_loss_pnl,
            total_win_pnl, total_loss_pnl,
            loss_streak: streak, max_loss_streak: streak, last_tune_at: 0,
        }
    }

    #[test]
    fn scalp_low_winrate_tightens_sl_and_cuts_leverage() {
        let mut cfg = make_cfg();
        let s = stats(3, 7, 3.0, 7.0, 0); // wr=0.30, pf=0.43
        let changes = auto_tune(&s, TradeType::Scalp, &mut cfg);
        assert!(cfg.scalp_sl_pct < 0.40);
        assert!(cfg.scalp_leverage < 2.0);
        assert!(changes.iter().any(|c| c.contains("SL Tightened")));
        assert!(changes.iter().any(|c| c.contains("Leverage")));
    }

    #[test]
    fn scalp_high_winrate_widens_tp_and_raises_leverage() {
        let mut cfg = make_cfg();
        // wr=0.70, pf=2.0 → boost path
        let s = stats(7, 3, 8.0, 4.0, 0);
        let changes = auto_tune(&s, TradeType::Scalp, &mut cfg);
        assert!(cfg.scalp_tp_pct > 0.80, "TP genişlemeli, got {}", cfg.scalp_tp_pct);
        assert!(cfg.scalp_leverage > 2.0, "Lev artmalı, got {}", cfg.scalp_leverage);
        assert!(changes.iter().any(|c| c.contains("TP Widened")));
        assert!(changes.iter().any(|c| c.contains("Leverage Raised")));
    }

    #[test]
    fn loss_streak_triggers_aggressive_leverage_cut() {
        let mut cfg = make_cfg();
        // wr=0.50 (nötr), pf=1.0, ama streak=3 → agresif kesinti
        let s = stats(5, 5, 5.0, 5.0, 3);
        let changes = auto_tune(&s, TradeType::Swing, &mut cfg);
        assert!(cfg.swing_leverage < 2.0, "Streak ile lev kesilmeli, got {}", cfg.swing_leverage);
        assert!(changes.iter().any(|c| c.contains("Leverage Cut")));
    }

    #[test]
    fn leverage_clamps_at_bounds_max() {
        let mut cfg = make_cfg();
        cfg.scalp_leverage = 9.5;
        // Boost path: 9.5 × 1.10 = 10.45 → clamp 10.0
        let s = stats(7, 3, 8.0, 4.0, 0);
        auto_tune(&s, TradeType::Scalp, &mut cfg);
        assert!((cfg.scalp_leverage - 10.0).abs() < 1e-9, "max clamp, got {}", cfg.scalp_leverage);
    }

    #[test]
    fn leverage_clamps_at_bounds_min() {
        let mut cfg = make_cfg();
        cfg.swing_leverage = 1.2;
        // Cut: 1.2 × 0.70 = 0.84 → clamp 1.0
        let s = stats(2, 8, 2.0, 8.0, 4);
        auto_tune(&s, TradeType::Swing, &mut cfg);
        assert!((cfg.swing_leverage - 1.0).abs() < 1e-9, "min clamp, got {}", cfg.swing_leverage);
    }

    #[test]
    fn neutral_winrate_no_change_unless_streak() {
        let mut cfg = make_cfg();
        // wr=0.50, pf=1.0, streak=0 → herhangi bir kural tetiklenmemeli
        let s = stats(5, 5, 5.0, 5.0, 0);
        let before_lev = cfg.scalp_leverage;
        let before_sl  = cfg.scalp_sl_pct;
        let before_tp  = cfg.scalp_tp_pct;
        let changes = auto_tune(&s, TradeType::Scalp, &mut cfg);
        assert!((cfg.scalp_leverage - before_lev).abs() < 1e-9);
        assert!((cfg.scalp_sl_pct - before_sl).abs()   < 1e-9);
        assert!((cfg.scalp_tp_pct - before_tp).abs()   < 1e-9);
        assert!(changes.is_empty());
    }

    #[test]
    fn regular_trade_type_is_noop() {
        let mut cfg = make_cfg();
        let s = stats(2, 8, 2.0, 8.0, 5);
        let changes = auto_tune(&s, TradeType::Regular, &mut cfg);
        assert!(changes.is_empty());
        assert_eq!(cfg.scalp_leverage, 2.0);
        assert_eq!(cfg.swing_leverage, 2.0);
    }
}
