// risk_limits.rs
// Tam Otomatik Risk ve Limit Yönetimi Modülü
// Dynamic risk limits, drawdown protection, circuit breaker

use crate::portfolio::Portfolio;
use chrono::{DateTime, Utc};

/// Risk limiti türleri
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RiskLimitType {
    MaxDrawdown(f64),
    MaxPositionSize(f64),
    MaxLossPerDay(f64),
    CircuitBreaker(bool),
}

/// Risk limiti yöneticisi trait'i
pub trait RiskLimitManager {
    fn check_limits(&self, portfolio: &Portfolio) -> Vec<RiskLimitType>;
    fn should_trigger_circuit_breaker(&self, portfolio: &Portfolio) -> bool;
}

/// Basit örnek: Maksimum çekilme ve günlük zarar limiti
pub struct SimpleRiskLimitManager {
    pub max_drawdown_pct: f64,
    pub max_loss_per_day: f64,
    pub circuit_breaker_enabled: bool,
    pub last_triggered: Option<DateTime<Utc>>,
}

impl RiskLimitManager for SimpleRiskLimitManager {
    fn check_limits(&self, portfolio: &Portfolio) -> Vec<RiskLimitType> {
        let mut limits = vec![];
        let metrics = portfolio.update_metrics();
        if metrics.total_pnl < -self.max_loss_per_day {
            limits.push(RiskLimitType::MaxLossPerDay(self.max_loss_per_day));
        }
        // Çekilme kontrolü (örnek: toplam zarar/ilk bakiye)
        // Burada daha gelişmiş bir çekilme hesabı eklenebilir
        if metrics.total_pnl < -portfolio.balance * self.max_drawdown_pct / 100.0 {
            limits.push(RiskLimitType::MaxDrawdown(self.max_drawdown_pct));
        }
        if self.circuit_breaker_enabled && self.should_trigger_circuit_breaker(portfolio) {
            limits.push(RiskLimitType::CircuitBreaker(true));
        }
        limits
    }
    fn should_trigger_circuit_breaker(&self, portfolio: &Portfolio) -> bool {
        let metrics = portfolio.update_metrics();
        metrics.total_pnl < -self.max_loss_per_day * 2.0
    }
}
