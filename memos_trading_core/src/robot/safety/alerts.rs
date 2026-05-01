use chrono::{DateTime, Utc};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TradingAlertLevel {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone)]
pub struct TradingAlert {
    pub level: TradingAlertLevel,
    pub message: String,
    pub timestamp: DateTime<Utc>,
    pub code: AlertCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertCode {
    HighDrawdown,
    CircuitBreakerTriggered,
    ConsecutiveLosses,
    PositionTooLarge,
    InsufficientBalance,
    OrderFailed,
    HighSlippage,
    ConnectionLost,
    StrategySignalGenerated,
    PriceAlert,
}

pub struct AlertManager {
    alerts: VecDeque<TradingAlert>,
    max_alerts: usize,
}

impl AlertManager {
    pub fn new(max_alerts: usize) -> Self {
        Self {
            alerts: VecDeque::new(),
            max_alerts,
        }
    }

    pub fn send_alert(&mut self, level: TradingAlertLevel, message: String, code: AlertCode) {
        let alert = TradingAlert {
            level,
            message,
            timestamp: Utc::now(),
            code,
        };

        self.alerts.push_back(alert);

        while self.alerts.len() > self.max_alerts {
            self.alerts.pop_front();
        }
    }

    pub fn drawdown_alert(&mut self, current_drawdown: f64, threshold: f64) {
        if current_drawdown > threshold {
            self.send_alert(
                TradingAlertLevel::Critical,
                format!("Drawdown aşıldı: {:.2}% / {:.2}%", current_drawdown, threshold),
                AlertCode::HighDrawdown,
            );
        }
    }

    pub fn consecutive_losses_alert(&mut self, count: usize, max_allowed: usize) {
        if count > max_allowed {
            self.send_alert(
                TradingAlertLevel::Warning,
                format!("Ardışık kayıp sayısı aşıldı: {} / {}", count, max_allowed),
                AlertCode::ConsecutiveLosses,
            );
        }
    }

    pub fn position_size_alert(&mut self, current_size: f64, max_size: f64) {
        if current_size > max_size {
            self.send_alert(
                TradingAlertLevel::Warning,
                format!("Pozisyon boyutu çok büyük: {:.2} / {:.2}", current_size, max_size),
                AlertCode::PositionTooLarge,
            );
        }
    }

    pub fn strategy_signal_alert(&mut self, symbol: &str, signal: &str, price: f64) {
        self.send_alert(
            TradingAlertLevel::Info,
            format!("{}: {} sinyali @ {:.2}", symbol, signal, price),
            AlertCode::StrategySignalGenerated,
        );
    }

    pub fn recent_alerts(&self, n: usize) -> Vec<TradingAlert> {
        self.alerts
            .iter()
            .rev()
            .take(n)
            .cloned()
            .collect()
    }

    pub fn alert_count_by_level(&self, level: TradingAlertLevel) -> usize {
        self.alerts.iter().filter(|a| a.level == level).count()
    }

    pub fn clear_alerts(&mut self) {
        self.alerts.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alert_manager_creation() {
        let manager = AlertManager::new(100);
        assert_eq!(manager.alerts.len(), 0);
    }

    #[test]
    fn test_send_alert() {
        let mut manager = AlertManager::new(100);
        manager.send_alert(TradingAlertLevel::Critical, "Test".to_string(), AlertCode::CircuitBreakerTriggered);
        assert_eq!(manager.alerts.len(), 1);
    }

    #[test]
    fn test_alert_levels() {
        let mut manager = AlertManager::new(100);
        manager.send_alert(TradingAlertLevel::Critical, "C".to_string(), AlertCode::CircuitBreakerTriggered);
        manager.send_alert(TradingAlertLevel::Warning, "W".to_string(), AlertCode::ConsecutiveLosses);
        manager.send_alert(TradingAlertLevel::Info, "I".to_string(), AlertCode::StrategySignalGenerated);
        
        assert_eq!(manager.alert_count_by_level(TradingAlertLevel::Critical), 1);
        assert_eq!(manager.alert_count_by_level(TradingAlertLevel::Warning), 1);
        assert_eq!(manager.alert_count_by_level(TradingAlertLevel::Info), 1);
    }
}
