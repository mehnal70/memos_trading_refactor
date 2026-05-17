use crate::core::types::Trade;
use chrono::{DateTime, Utc, Duration};
use std::collections::VecDeque;

/// Maksimum drawdown izlemi için yapı
#[derive(Debug, Clone)]
pub struct SafetyDrawdownMonitor {
    peak_balance: f64,
    current_balance: f64,
    max_drawdown_pct: f64,
    history: VecDeque<(DateTime<Utc>, f64)>, // (timestamp, balance)
}

impl SafetyDrawdownMonitor {
    /// Yeni DrawdownMonitor oluştur
    pub fn new(initial_balance: f64) -> Self {
        let mut history = VecDeque::new();
        history.push_back((Utc::now(), initial_balance));
        
        Self {
            peak_balance: initial_balance,
            current_balance: initial_balance,
            max_drawdown_pct: 0.0,
            history,
        }
    }

    /// Balance güncelle ve drawdown hesapla
    pub fn update_balance(&mut self, new_balance: f64) {
        self.current_balance = new_balance;
        
        // Peak'i güncelle
        if new_balance > self.peak_balance {
            self.peak_balance = new_balance;
        }
        
        // Mevcut drawdown hesapla
        if self.peak_balance > 0.0 {
            self.max_drawdown_pct = 
                ((self.peak_balance - new_balance) / self.peak_balance) * 100.0;
        }
        
        // History'e ekle
        self.history.push_back((Utc::now(), new_balance));
        
        // Son 100 kaydı tut (bellek yönetimi)
        while self.history.len() > 100 {
            self.history.pop_front();
        }
    }

    pub fn current_drawdown_pct(&self) -> f64 {
        self.max_drawdown_pct
    }

    pub fn peak_balance(&self) -> f64 {
        self.peak_balance
    }

    pub fn current_balance(&self) -> f64 {
        self.current_balance
    }
}

/// Safety yönetim kuralları
#[derive(Debug, Clone)]
pub struct SafetyRules {
    pub max_drawdown_pct: f64,              // Maksimum %15 drawdown
    pub max_consecutive_losses: usize,     // Maksimum 5 ardışık kayıp
    pub circuit_breaker_threshold: f64,    // %10'dan fazla loss → durdur
    pub daily_loss_limit_pct: f64,         // Günlük maksimum %2 loss
    pub pause_duration_minutes: u64,       // Hatadan sonra pause süresi
}

impl Default for SafetyRules {
    fn default() -> Self {
        Self {
            max_drawdown_pct: 15.0,
            max_consecutive_losses: 5,
            circuit_breaker_threshold: 10.0,
            daily_loss_limit_pct: 2.0,
            pause_duration_minutes: 5,
        }
    }
}

/// Ana Safety Manager yapı
#[derive(Debug, Clone)]
pub struct SafetyManager {
    drawdown_monitor: SafetyDrawdownMonitor,
    rules: SafetyRules,
    consecutive_losses: usize,
    is_paused: bool,
    pause_until: Option<DateTime<Utc>>,
    circuit_breaker_triggered: bool,
}

impl SafetyManager {
    /// Yeni Safety Manager oluştur
    pub fn new(initial_balance: f64, rules: SafetyRules) -> Self {
        Self {
            drawdown_monitor: SafetyDrawdownMonitor::new(initial_balance),
            rules,
            consecutive_losses: 0,
            is_paused: false,
            pause_until: None,
            circuit_breaker_triggered: false,
        }
    }

    /// Mevcut balance'ı güncelle
    pub fn update_balance(&mut self, new_balance: f64) {
        self.drawdown_monitor.update_balance(new_balance);
    }

    /// Trade sonrası safety kontrolleri
    pub fn check_trade_safety(&mut self, trade: &Trade) -> Result<SafetyStatus, String> {
        // Pause kontrolü
        if self.is_paused {
            if let Some(until) = self.pause_until {
                if Utc::now() < until {
                    return Ok(SafetyStatus::Paused);
                } else {
                    self.is_paused = false;
                    self.pause_until = None;
                }
            }
        }

        // Circuit breaker kontrolü
        if self.circuit_breaker_triggered {
            return Err("Circuit breaker aktif - trading durdu".to_string());
        }

        // Drawdown kontrolü
        let drawdown = self.drawdown_monitor.current_drawdown_pct();
        if drawdown > self.rules.max_drawdown_pct {
            self.circuit_breaker_triggered = true;
            return Err(format!(
                "Drawdown limiti aşıldı: {:.2}% / {:.2}%",
                drawdown, self.rules.max_drawdown_pct
            ));
        }

        // Ardışık kayıp kontrolü
        if let Some(pnl) = trade.pnl {
            if pnl < 0.0 {
                self.consecutive_losses += 1;
            } else {
                self.consecutive_losses = 0;
            }

            if self.consecutive_losses > self.rules.max_consecutive_losses {
                self.trigger_pause();
                return Ok(SafetyStatus::PausedDueToConsecutiveLosses);
            }
        }

        Ok(SafetyStatus::Safe)
    }

    /// Hata durumunda pause tetikle
    fn trigger_pause(&mut self) {
        self.is_paused = true;
        self.pause_until = Some(
            Utc::now() + Duration::minutes(self.rules.pause_duration_minutes as i64)
        );
    }

    /// Safety durum kontrolü - trading yapılabilir mi?
    pub fn can_trade(&self) -> bool {
        !self.is_paused && !self.circuit_breaker_triggered
    }

    /// Mevcut metrics
    pub fn metrics(&self) -> SafetyMetrics {
        SafetyMetrics {
            current_drawdown_pct: self.drawdown_monitor.current_drawdown_pct(),
            consecutive_losses: self.consecutive_losses,
            is_paused: self.is_paused,
            circuit_breaker_triggered: self.circuit_breaker_triggered,
            peak_balance: self.drawdown_monitor.peak_balance(),
            current_balance: self.drawdown_monitor.current_balance(),
        }
    }

    /// Pause'u manuel sıfırla
    pub fn reset_pause(&mut self) {
        self.is_paused = false;
        self.pause_until = None;
    }

    /// Circuit breaker'ı sıfırla (reset için)
    pub fn reset_circuit_breaker(&mut self) {
        self.circuit_breaker_triggered = false;
        self.consecutive_losses = 0;
    }
}

/// Safety durumu
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyStatus {
    Safe,
    Paused,
    PausedDueToConsecutiveLosses,
}

/// Gerçek zamanlı Safety metrics
#[derive(Debug, Clone)]
pub struct SafetyMetrics {
    pub current_drawdown_pct: f64,
    pub consecutive_losses: usize,
    pub is_paused: bool,
    pub circuit_breaker_triggered: bool,
    pub peak_balance: f64,
    pub current_balance: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_drawdown_monitor_creation() {
        let monitor = SafetyDrawdownMonitor::new(10000.0);
        assert_eq!(monitor.peak_balance(), 10000.0);
        assert_eq!(monitor.current_balance(), 10000.0);
        assert_eq!(monitor.current_drawdown_pct(), 0.0);
    }

    #[test]
    fn test_drawdown_calculation() {
        let mut monitor = SafetyDrawdownMonitor::new(10000.0);
        monitor.update_balance(9000.0); // %10 loss
        assert_eq!(monitor.current_drawdown_pct(), 10.0);
        
        monitor.update_balance(8000.0); // %20 total loss
        assert_eq!(monitor.current_drawdown_pct(), 20.0);
        
        // Eski peak'ten recovery
        monitor.update_balance(9500.0);
        assert_eq!(monitor.current_drawdown_pct(), 5.0);
    }

    #[test]
    fn test_safety_manager_creation() {
        let manager = SafetyManager::new(10000.0, SafetyRules::default());
        assert!(!manager.is_paused);
        assert!(!manager.circuit_breaker_triggered);
        assert_eq!(manager.consecutive_losses, 0);
    }

    #[test]
    fn test_circuit_breaker_trigger() {
        let mut manager = SafetyManager::new(10000.0, SafetyRules {
            max_drawdown_pct: 10.0,
            ..Default::default()
        });
        
        manager.update_balance(8500.0); // %15 drawdown
        let result = manager.check_trade_safety(&Trade {
            id: None,
            symbol: "BTC".to_string(),
            entry_price: 100.0,
            exit_price: None,
            amount: 1.0,
            entry_time: Utc::now(),
            exit_time: None,
            pnl: None,
            pnl_pct: None,
            strategy: "test".to_string(),
        });
        
        assert!(result.is_err());
        assert!(manager.circuit_breaker_triggered);
    }

    #[test]
    fn test_consecutive_losses_pause() {
        let mut manager = SafetyManager::new(10000.0, SafetyRules {
            max_consecutive_losses: 2,
            ..Default::default()
        });
        
        // 2 ardışık loss
        for _ in 0..3 {
            let trade = Trade {
                id: None,
                symbol: "BTC".to_string(),
                entry_price: 100.0,
                exit_price: Some(99.0),
                amount: 1.0,
                entry_time: Utc::now(),
                exit_time: Some(Utc::now()),
                pnl: Some(-100.0),
                pnl_pct: Some(-1.0),
                strategy: "test".to_string(),
            };
            
            let _ = manager.check_trade_safety(&trade);
        }
        
        assert!(manager.is_paused);
    }

    #[test]
    fn test_can_trade() {
        let manager = SafetyManager::new(10000.0, SafetyRules::default());
        assert!(manager.can_trade());
    }
}
