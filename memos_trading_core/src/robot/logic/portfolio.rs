// portfolio.rs - Portföy ve Durum Yönetimi Modülü

use crate::core::types::{Trade, Signal, RiskParams};
use crate::robot::infra::monitoring::health_monitor::{HealthCheck, AnomalyDetector, HealthStatus, AnomalyType};
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Default)]
pub struct Position {
    pub symbol: String,
    pub entry_price: f64,
    pub amount: f64,
    pub entry_time: DateTime<Utc>,
    pub signal: Signal,
    pub strategy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PortfolioMetrics {
    pub total_pnl: f64,
    pub win_rate: f64,
    pub loss_rate: f64,
    pub trade_count: usize,
    pub closed_trades_count: usize,
    pub open_positions_count: usize,
    pub max_drawdown: f64,
}

#[derive(Debug, Clone, Default)]
pub struct Portfolio {
    pub balance: f64,
    pub positions: Vec<Position>,
    pub trade_history: Vec<Trade>,
    pub risk_params: Option<RiskParams>,
}

impl Portfolio {
    pub fn new(balance: f64, risk_params: Option<RiskParams>) -> Self {
        Self { 
            balance, 
            positions: Vec::with_capacity(10), // Allocation optimizasyonu
            trade_history: Vec::with_capacity(100), 
            risk_params 
        }
    }

    /// Pozisyon aç - Performans: to_owned() ile bellek yönetimi
    pub fn open_position(&mut self, symbol: &str, entry_price: f64, amount: f64, signal: Signal, strategy: &str) {
        let pos = Position {
            symbol: symbol.to_owned(),
            entry_price,
            amount,
            entry_time: Utc::now(),
            signal,
            strategy: strategy.to_owned(),
        };
        self.positions.push(pos);
        self.balance -= entry_price * amount;
    }

    /// Pozisyon kapat - Modern Pattern Matching ile PnL hesabı
    pub fn close_position(&mut self, symbol: &str, exit_price: f64) {
        if let Some(idx) = self.positions.iter().position(|p| p.symbol == symbol) {
            let pos = self.positions.remove(idx);
            
            let pnl_multiplier = match pos.signal {
                Signal::Buy => 1.0,
                Signal::Sell => -1.0,
                _ => 0.0,
            };

            let pnl = (exit_price - pos.entry_price) * pos.amount * pnl_multiplier;
            self.balance += (exit_price * pos.amount) + pnl;

            self.trade_history.push(Trade {
                id: None,
                symbol: pos.symbol,
                entry_price: pos.entry_price,
                exit_price: Some(exit_price),
                amount: pos.amount,
                entry_time: pos.entry_time,
                exit_time: Some(Utc::now()),
                pnl: Some(pnl),
                pnl_pct: Some(pnl / (pos.entry_price * pos.amount)),
                strategy: pos.strategy,
            });
        }
    }

    /// Toplam riskli varlık (Exposure) hesabı
    pub fn total_risk_exposure(&self) -> f64 {
        self.positions.iter().map(|p| p.amount * p.entry_price).sum()
    }

    /// Risk limiti kontrolü - Early return mantığı
    pub fn is_risk_limit_exceeded(&self) -> bool {
        let Some(risk) = &self.risk_params else { return false };
        let Some(max_risk_pct) = risk.max_portfolio_risk_pct else { return false };

        let exposure = self.total_risk_exposure();
        let total_value = self.balance + exposure;
        exposure > (total_value * (max_risk_pct / 100.0))
    }

    /// Metrikleri tek bir geçişte (single-pass) hesapla - O(n) optimizasyonu
    pub fn update_metrics(&self) -> PortfolioMetrics {
        let total = self.trade_history.len();
        if total == 0 { return PortfolioMetrics::default(); }

        let mut total_pnl = 0.0;
        let mut win_count = 0;
        let mut loss_count = 0;

        for trade in &self.trade_history {
            let pnl = trade.pnl.unwrap_or(0.0);
            total_pnl += pnl;
            if pnl > 0.0 { win_count += 1; }
            else if pnl < 0.0 { loss_count += 1; }
        }

        PortfolioMetrics {
            total_pnl,
            win_rate: win_count as f64 / total as f64,
            loss_rate: loss_count as f64 / total as f64,
            trade_count: total,
            closed_trades_count: total,
            open_positions_count: self.positions.len(),
            max_drawdown: 0.0, // Drawdown serisi takip edilirse buradan beslenir.
        }
    }
}

// --- SAĞLIK VE ANOMALİ İMPLEMENTASYONLARI ---

impl HealthCheck for Portfolio {
    fn check_health(&self) -> HealthStatus {
        let m = self.update_metrics();
        match m {
            _ if m.total_pnl < 0.0 => HealthStatus::Warning(format!("Negatif PnL: {:.2}", m.total_pnl)),
            _ if m.trade_count > 10 && m.win_rate < 0.2 => HealthStatus::Warning("Düşük Win-Rate".to_owned()),
            _ => HealthStatus::Healthy,
        }
    }
}

impl AnomalyDetector for Portfolio {
    fn detect_anomaly(&self) -> Option<AnomalyType> {
        let m = self.update_metrics();
        if m.total_pnl < -1000.0 {
            return Some(AnomalyType::Custom(format!("Aşırı Zarar: {:.2}", m.total_pnl)));
        }
        if m.trade_count > 5 && m.win_rate == 0.0 {
            return Some(AnomalyType::Custom("Sıfır Kazanç Anomalisi".to_owned()));
        }
        None
    }
}
