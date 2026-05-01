//! Risk yönetimi - Trading sinyalleri ve risk parametreleri

/// Risk parametreleri
#[derive(Debug, Clone)]
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

/// Trading sinyali
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeAction {
    Buy,   // AL
    Sell,  // SAT
    Hold,  // BEKLE
}

impl TradeAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            TradeAction::Buy => "AL",
            TradeAction::Sell => "SAT",
            TradeAction::Hold => "BEKLE",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "AL" => Some(TradeAction::Buy),
            "SAT" => Some(TradeAction::Sell),
            "BEKLE" => Some(TradeAction::Hold),
            _ => None,
        }
    }
}

/// Trade sinyali ile hesaplanan stop loss ve take profit
#[derive(Debug, Clone)]
pub struct TradeSignal {
    pub action: TradeAction,
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub timestamp: i64,
}

impl TradeSignal {
    /// Yeni trade sinyali oluştur
    pub fn new(
        entry_price: f64,
        timestamp: i64,
        risk: &RiskParams,
        action_str: &str,
    ) -> Self {
        let action = TradeAction::from_str(action_str)
            .unwrap_or(TradeAction::Hold);

        let stop_loss = entry_price * (1.0 - risk.stop_loss_pct / 100.0);
        let take_profit = entry_price * (1.0 + risk.take_profit_pct / 100.0);

        TradeSignal {
            action,
            entry_price,
            stop_loss,
            take_profit,
            timestamp,
        }
    }

    /// Pozisyon boyutunu hesapla
    pub fn calculate_position_size(&self, capital: f64, risk: &RiskParams) -> f64 {
        if let Some(max_pct) = risk.max_position_size_pct {
            let max_amount = capital * (max_pct / 100.0);
            max_amount / self.entry_price
        } else {
            capital / self.entry_price
        }
    }

    /// Kar/zarar hesapla
    pub fn calculate_pnl(&self, exit_price: f64, quantity: f64) -> f64 {
        (exit_price - self.entry_price) * quantity
    }

    /// Kar/zarar yüzdesini hesapla
    pub fn calculate_pnl_pct(&self, exit_price: f64) -> f64 {
        ((exit_price - self.entry_price) / self.entry_price) * 100.0
    }
}

/// Risk yöneticisi
pub struct RiskManager {
    params: RiskParams,
}

impl RiskManager {
    pub fn new(params: RiskParams) -> Self {
        Self { params }
    }

    /// Stop loss seviyesini hesapla
    pub fn calculate_stop_loss(&self, entry_price: f64) -> f64 {
        entry_price * (1.0 - self.params.stop_loss_pct / 100.0)
    }

    /// Take profit seviyesini hesapla
    pub fn calculate_take_profit(&self, entry_price: f64) -> f64 {
        entry_price * (1.0 + self.params.take_profit_pct / 100.0)
    }

    /// Pozisyon boyutunu hesapla
    pub fn calculate_position_size(&self, capital: f64, entry_price: f64) -> f64 {
        if let Some(max_pct) = self.params.max_position_size_pct {
            let max_amount = capital * (max_pct / 100.0);
            max_amount / entry_price
        } else {
            capital / entry_price
        }
    }

    /// Risk/Reward oranını hesapla
    pub fn calculate_risk_reward_ratio(&self, entry: f64) -> f64 {
        let risk = self.calculate_stop_loss(entry);
        let reward = self.calculate_take_profit(entry);
        (reward - entry) / (entry - risk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trade_signal() {
        let risk = RiskParams::default();
        let signal = TradeSignal::new(100.0, 1000, &risk, "AL");

        assert_eq!(signal.action, TradeAction::Buy);
        assert_eq!(signal.entry_price, 100.0);
        assert_eq!(signal.stop_loss, 98.0); // 100 * (1 - 0.02)
        assert_eq!(signal.take_profit, 105.0); // 100 * (1 + 0.05)
    }

    #[test]
    fn test_pnl_calculation() {
        let risk = RiskParams::default();
        let signal = TradeSignal::new(100.0, 1000, &risk, "AL");

        let pnl = signal.calculate_pnl(110.0, 10.0);
        assert_eq!(pnl, 100.0); // (110 - 100) * 10

        let pnl_pct = signal.calculate_pnl_pct(110.0);
        assert_eq!(pnl_pct, 10.0); // ((110 - 100) / 100) * 100
    }
}
