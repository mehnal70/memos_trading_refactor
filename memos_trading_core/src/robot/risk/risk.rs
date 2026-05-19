// risk_advanced.rs - Gelişmiş Risk ve Sermaye Koruma Modülü

use serde::{Serialize, Deserialize};

/// Risk parametreleri - Hafıza verimliliği için Copy eklendi
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RiskParams {
    pub stop_loss_pct: f64,
    pub take_profit_pct: f64,
    pub max_position_size_pct: Option<f64>,
    pub max_portfolio_risk_pct: Option<f64>,
}

impl Default for RiskParams {
    fn default() -> Self {
        Self {
            stop_loss_pct: 2.0,
            take_profit_pct: 5.0,
            max_position_size_pct: Some(10.0),
            max_portfolio_risk_pct: Some(2.0),
        }
    }
}

/// Trading aksiyonu - CPU registers düzeyinde taşınması için Copy eklendi
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TradeAction {
    Buy,   // AL
    Sell,  // SAT
    Hold,  // BEKLE
}

impl TradeAction {
    #[inline]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Buy => "AL",
            Self::Sell => "SAT",
            Self::Hold => "BEKLE",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "AL" | "BUY" => Self::Buy,
            "SAT" | "SELL" => Self::Sell,
            _ => Self::Hold,
        }
    }
}

/// Rafine edilmiş Trade Sinyali
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeSignal {
    pub action: TradeAction,
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub timestamp: i64,
}

impl TradeSignal {
    /// Yeni sinyal oluştur — action'a göre yönlü SL/TP.
    ///
    /// - `Buy`  (LONG):  SL = entry * (1 - sl%);  TP = entry * (1 + tp%)
    /// - `Sell` (SHORT): SL = entry * (1 + sl%);  TP = entry * (1 - tp%)
    /// - `Hold`:         SL/TP entry'ye eşitlenir (anlamsız ama panic'siz).
    pub fn new(
        entry_price: f64,
        timestamp: i64,
        risk: &RiskParams,
        action: TradeAction,
    ) -> Self {
        // Matematiksel kontroller: Sıfır fiyat durumunda çökme önlenir
        let entry = entry_price.max(f64::EPSILON);
        let sl_factor = risk.stop_loss_pct / 100.0;
        let tp_factor = risk.take_profit_pct / 100.0;

        let (stop_loss, take_profit) = match action {
            TradeAction::Buy  => (entry * (1.0 - sl_factor), entry * (1.0 + tp_factor)),
            TradeAction::Sell => (entry * (1.0 + sl_factor), entry * (1.0 - tp_factor)),
            TradeAction::Hold => (entry, entry),
        };

        TradeSignal {
            action,
            entry_price: entry,
            stop_loss,
            take_profit,
            timestamp,
        }
    }

    /// Pozisyon boyutu hesabı - Pipeline dostu
    pub fn calculate_position_size(&self, capital: f64, risk: &RiskParams) -> f64 {
        let max_allowed_capital = risk.max_position_size_pct
            .map_or(capital, |pct| capital * (pct / 100.0));

        max_allowed_capital / self.entry_price
    }

    /// Net Kâr/Zarar (PnL) — action yönünü dikkate alır.
    /// LONG: (exit - entry) * qty;  SHORT: (entry - exit) * qty.
    #[inline]
    pub fn calculate_pnl(&self, exit_price: f64, quantity: f64) -> f64 {
        let diff = match self.action {
            TradeAction::Buy  => exit_price - self.entry_price,
            TradeAction::Sell => self.entry_price - exit_price,
            TradeAction::Hold => 0.0,
        };
        diff * quantity
    }

    /// Yüzdesel PnL — action yönünü dikkate alır.
    /// LONG: (exit/entry - 1) * 100;  SHORT: (entry/exit - 1) * 100.
    #[inline]
    pub fn calculate_pnl_pct(&self, exit_price: f64) -> f64 {
        let entry = self.entry_price.max(f64::EPSILON);
        let exit  = exit_price.max(f64::EPSILON);
        match self.action {
            TradeAction::Buy  => ((exit / entry) - 1.0) * 100.0,
            TradeAction::Sell => ((entry / exit) - 1.0) * 100.0,
            TradeAction::Hold => 0.0,
        }
    }
}

/// Merkezi Risk Yöneticisi
pub struct RiskManager {
    params: RiskParams,
}

impl RiskManager {
    pub fn new(params: RiskParams) -> Self {
        Self { params }
    }

    #[inline]
    pub fn calculate_stop_loss(&self, entry: f64) -> f64 {
        entry * (1.0 - self.params.stop_loss_pct / 100.0)
    }

    #[inline]
    pub fn calculate_take_profit(&self, entry: f64) -> f64 {
        entry * (1.0 + self.params.take_profit_pct / 100.0)
    }

    /// R/R (Risk/Ödül) Oranı hesabı
    pub fn calculate_risk_reward_ratio(&self, entry: f64) -> f64 {
        let risk = (entry - self.calculate_stop_loss(entry)).abs();
        let reward = (self.calculate_take_profit(entry) - entry).abs();
        
        if risk < f64::EPSILON { 0.0 } else { reward / risk }
    }

    /// Portföy riskine göre dinamik miktar belirleme (Kelly-like)
    pub fn dynamic_amount(&self, capital: f64, entry: f64) -> f64 {
        let risk_pool = self.params.max_portfolio_risk_pct
            .map_or(capital, |pct| capital * (pct / 100.0));
        
        let position_limit = self.params.max_position_size_pct
            .map_or(risk_pool, |pct| capital * (pct / 100.0));

        position_limit.min(risk_pool) / entry
    }
}
