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

pub fn auto_tune(stats: &ScalpSwingStats, trade_type: TradeType, cfg: &mut ScalpSwingConfig) -> Vec<String> {
    let mut changes = Vec::new();
    let wr = stats.win_rate();
    let pf = stats.profit_factor();
    match trade_type {
        TradeType::Scalp => { /* ... Scalp lojiği ... */ }
        TradeType::Swing => {
            let b_sl = cfg.swing_sl_bounds.clone();
            if wr < 0.40 {
                let new_sl = b_sl.clamp(cfg.swing_sl_pct * 0.90);
                if (new_sl - cfg.swing_sl_pct).abs() > 0.01 { cfg.swing_sl_pct = new_sl; changes.push("SWG SL Adjusted".to_string()); }
            }
            // ... (Diğer Swing lojikleri) ...
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
