use crate::types::Trade;
use crate::robot::safety::{SafetyMetrics, TradingMetrics};
use chrono::{DateTime, Utc};

/// Gerçek zamanlı dashboard durumu
#[derive(Debug, Clone)]
pub struct DashboardState {
    /// Cari account balance
    pub current_balance: f64,
    /// İlk yatırılan miktar
    pub initial_balance: f64,
    /// Toplam P&L
    pub total_pnl: f64,
    /// Toplam P&L yüzdesi
    pub total_pnl_pct: f64,
    /// Açık pozisyon
    pub open_position: Option<OpenPosition>,
    /// Son işlem
    pub last_trade: Option<Trade>,
    /// Dashboard güncellenme tarihi
    pub updated_at: DateTime<Utc>,
}

/// Açık pozisyon bilgisi
#[derive(Debug, Clone)]
pub struct OpenPosition {
    pub symbol: String,
    pub entry_price: f64,
    pub current_price: f64,
    pub amount: f64,
    pub unrealized_pnl: f64,
    pub unrealized_pnl_pct: f64,
    pub entry_time: DateTime<Utc>,
}

impl OpenPosition {
    /// Pozisyon fiyatını güncelle ve unrealized P&L hesapla
    pub fn update_price(&mut self, current_price: f64) {
        self.current_price = current_price;
        self.unrealized_pnl = (current_price - self.entry_price) * self.amount;
        self.unrealized_pnl_pct = 
            ((current_price - self.entry_price) / self.entry_price) * 100.0;
    }
}

/// Tauri için JSON formatında dashboard verisi
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DashboardData {
    pub balance: f64,
    pub initial_balance: f64,
    pub total_pnl: f64,
    pub total_pnl_pct: f64,
    pub win_rate: f64,
    pub consecutive_losses: usize,
    pub current_drawdown_pct: f64,
    pub is_paused: bool,
    pub circuit_breaker_triggered: bool,
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub profit_factor: f64,
    pub last_trade_symbol: Option<String>,
    pub last_trade_pnl: Option<f64>,
    pub updated_at: String,
}

/// Paper Trading Dashboard
#[derive(Debug, Clone)]
pub struct PaperTradingDashboard {
    state: DashboardState,
    trading_metrics: TradingMetrics,
    safety_metrics: SafetyMetrics,
    trade_history: Vec<Trade>,
}

impl PaperTradingDashboard {
    /// Yeni dashboard oluştur
    pub fn new(initial_balance: f64) -> Self {
        Self {
            state: DashboardState {
                current_balance: initial_balance,
                initial_balance,
                total_pnl: 0.0,
                total_pnl_pct: 0.0,
                open_position: None,
                last_trade: None,
                updated_at: Utc::now(),
            },
            trading_metrics: TradingMetrics::new(),
            safety_metrics: Default::default(),
            trade_history: Vec::new(),
        }
    }

    /// Trade'i dashboard'a ekle
    pub fn add_trade(&mut self, trade: Trade) {
        self.state.last_trade = Some(trade.clone());
        self.trade_history.push(trade.clone());
        
        // Trading metrics'i güncelle
        self.trading_metrics = TradingMetrics::from_trades(&self.trade_history);
        
        // Balance güncelle
        if let Some(pnl) = trade.pnl {
            self.state.total_pnl += pnl;
        }
        
        self.recalculate_metrics();
    }

    /// Safety metrics'i güncelle
    pub fn update_safety_metrics(&mut self, metrics: SafetyMetrics) {
        self.safety_metrics = metrics.clone();
        self.state.current_balance = metrics.current_balance;
        self.recalculate_metrics();
    }

    /// Dashboard verilerini Tauri'ye gönderilecek formata çevir
    pub fn to_json_data(&self) -> DashboardData {
        DashboardData {
            balance: self.state.current_balance,
            initial_balance: self.state.initial_balance,
            total_pnl: self.state.total_pnl,
            total_pnl_pct: self.state.total_pnl_pct,
            win_rate: self.trading_metrics.win_rate,
            consecutive_losses: self.trading_metrics.consecutive_losses,
            current_drawdown_pct: self.safety_metrics.current_drawdown_pct,
            is_paused: self.safety_metrics.is_paused,
            circuit_breaker_triggered: self.safety_metrics.circuit_breaker_triggered,
            total_trades: self.trading_metrics.total_trades,
            winning_trades: self.trading_metrics.winning_trades,
            losing_trades: self.trading_metrics.losing_trades,
            profit_factor: self.trading_metrics.profit_factor,
            last_trade_symbol: self.state.last_trade.as_ref().map(|t| t.symbol.clone()),
            last_trade_pnl: self.state.last_trade.as_ref().and_then(|t| t.pnl),
            updated_at: self.state.updated_at.to_rfc3339(),
        }
    }

    /// Açık pozisyon ekle
    pub fn set_open_position(&mut self, position: Option<OpenPosition>) {
        self.state.open_position = position;
        self.state.updated_at = Utc::now();
    }

    /// Sonraki trade için mevcut balance bilgisi getir
    pub fn available_balance(&self) -> f64 {
        self.state.current_balance
    }

    /// Trade history getir
    pub fn trade_history(&self) -> &[Trade] {
        &self.trade_history
    }

    /// Trading metrics getir
    pub fn metrics(&self) -> &TradingMetrics {
        &self.trading_metrics
    }

    /// Safety metrics getir
    pub fn safety_metrics(&self) -> &SafetyMetrics {
        &self.safety_metrics
    }

    /// Dashboard state getir
    pub fn state(&self) -> &DashboardState {
        &self.state
    }

    // Özel helpers
    fn recalculate_metrics(&mut self) {
        if self.state.initial_balance > 0.0 {
            self.state.total_pnl_pct = 
                (self.state.total_pnl / self.state.initial_balance) * 100.0;
        }
        self.state.updated_at = Utc::now();
    }
}

// Default implementation for SafetyMetrics
impl Default for SafetyMetrics {
    fn default() -> Self {
        SafetyMetrics {
            current_drawdown_pct: 0.0,
            consecutive_losses: 0,
            is_paused: false,
            circuit_breaker_triggered: false,
            peak_balance: 0.0,
            current_balance: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_trade(pnl: Option<f64>) -> Trade {
        Trade {
            id: None,
            symbol: "BTC".to_string(),
            entry_price: 100.0,
            exit_price: Some(100.0),
            amount: 1.0,
            entry_time: Utc::now(),
            exit_time: Some(Utc::now()),
            pnl,
            pnl_pct: pnl.map(|p| (p / 100.0) * 100.0),
            strategy: "test".to_string(),
        }
    }

    #[test]
    fn test_dashboard_creation() {
        let dashboard = PaperTradingDashboard::new(10000.0);
        assert_eq!(dashboard.state.initial_balance, 10000.0);
        assert_eq!(dashboard.state.current_balance, 10000.0);
        assert_eq!(dashboard.state.total_pnl, 0.0);
    }

    #[test]
    fn test_add_trade() {
        let mut dashboard = PaperTradingDashboard::new(10000.0);
        let trade = create_test_trade(Some(500.0));
        
        dashboard.add_trade(trade);
        assert_eq!(dashboard.trade_history.len(), 1);
        assert_eq!(dashboard.state.total_pnl, 500.0);
        assert!(dashboard.state.total_pnl_pct > 0.0);
    }

    #[test]
    fn test_dashboard_json_conversion() {
        let mut dashboard = PaperTradingDashboard::new(10000.0);
        dashboard.add_trade(create_test_trade(Some(200.0)));
        
        let json_data = dashboard.to_json_data();
        assert_eq!(json_data.initial_balance, 10000.0);
        assert_eq!(json_data.total_trades, 1);
        // Balance initial + trade P&L
        assert_eq!(json_data.total_pnl, 200.0);
    }

    #[test]
    fn test_open_position_update() {
        let mut position = OpenPosition {
            symbol: "BTC".to_string(),
            entry_price: 100.0,
            current_price: 100.0,
            amount: 1.0,
            unrealized_pnl: 0.0,
            unrealized_pnl_pct: 0.0,
            entry_time: Utc::now(),
        };

        position.update_price(110.0);
        assert_eq!(position.unrealized_pnl, 10.0);
        assert!(position.unrealized_pnl_pct > 9.9 && position.unrealized_pnl_pct < 10.1);
    }

    #[test]
    fn test_available_balance() {
        let dashboard = PaperTradingDashboard::new(5000.0);
        assert_eq!(dashboard.available_balance(), 5000.0);
    }
}
