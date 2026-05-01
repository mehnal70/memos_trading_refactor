// Failover System - Otomatik Yedek Executor'a Geçiş
//
// Hata durumunda Binance executor'dan Mock executor'a otomatik geçiş
// Graceful degradation - sistem çalışmaya devam eder

use serde::{Serialize, Deserialize};

/// Executor Türü
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutorType {
    /// Binance Futures API
    Binance,
    /// Mock executor (test/failover)
    Mock,
}

impl std::fmt::Display for ExecutorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutorType::Binance => write!(f, "Binance"),
            ExecutorType::Mock => write!(f, "Mock"),
        }
    }
}

/// Failover Stratejisi
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FailoverStrategy {
    /// İlk başta Binance, hata durumunda Mock'a geç
    PrimaryWithFallback {
        /// Primary executor
        primary: ExecutorType,
        /// Fallback executor
        fallback: ExecutorType,
    },
    
    /// Sadece Mock executor kullan (test modu)
    MockOnly,
    
    /// Sadece Binance executor kullan (strict)
    BinanceOnly,
}

/// Failover State
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailoverState {
    /// Primary çalışıyor
    Primary,
    /// Fallback'a geçti
    OnFallback,
    /// Hata - her iki executor da başarısız
    Failed,
}

/// Failover Manager
pub struct FailoverManager {
    strategy: FailoverStrategy,
    current_executor: ExecutorType,
    state: FailoverState,
    failure_count: u32,
    max_consecutive_failures: u32,
}

impl FailoverManager {
    /// Yeni Failover Manager oluştur
    pub fn new(strategy: FailoverStrategy) -> Self {
        let current = match strategy {
            FailoverStrategy::PrimaryWithFallback { primary, .. } => primary,
            FailoverStrategy::MockOnly => ExecutorType::Mock,
            FailoverStrategy::BinanceOnly => ExecutorType::Binance,
        };

        Self {
            strategy,
            current_executor: current,
            state: FailoverState::Primary,
            failure_count: 0,
            max_consecutive_failures: 3,
        }
    }

    /// Mevcut executor'u döndür
    pub fn current_executor(&self) -> ExecutorType {
        self.current_executor
    }

    /// Mevcut durumu döndür
    pub fn state(&self) -> FailoverState {
        self.state
    }

    /// Başarılı işlem kaydı
    pub fn record_success(&mut self) {
        self.failure_count = 0;
        
        // Eğer fallback'ta ise, primary'ye geri dön
        if self.state == FailoverState::OnFallback {
            if let FailoverStrategy::PrimaryWithFallback { primary, .. } = self.strategy {
                self.current_executor = primary;
                self.state = FailoverState::Primary;
                println!("✓ Failover: {} executor'a geri döndü", primary);
            }
        }
    }

    /// Başarısız işlem kaydı
    pub fn record_failure(&mut self, error: &str) {
        self.failure_count += 1;

        match self.strategy {
            FailoverStrategy::PrimaryWithFallback {
                primary,
                fallback,
            } => {
                if self.current_executor == primary
                    && self.failure_count >= self.max_consecutive_failures
                {
                    // Fallback'a geç
                    self.current_executor = fallback;
                    self.state = FailoverState::OnFallback;
                    println!(
                        "⚠ Failover: {} executor'dan {} executor'a geçti. Hata: {}",
                        primary, fallback, error
                    );
                    self.failure_count = 0;
                } else if self.current_executor == fallback
                    && self.failure_count >= self.max_consecutive_failures
                {
                    // Her iki executor da başarısız
                    self.state = FailoverState::Failed;
                    println!("✗ Failover: Her iki executor da başarısız! Hata: {}", error);
                }
            }
            FailoverStrategy::MockOnly => {
                // Mock-only modda failover yok
                if self.failure_count >= self.max_consecutive_failures {
                    self.state = FailoverState::Failed;
                }
            }
            FailoverStrategy::BinanceOnly => {
                // Binance-only modda failover yok
                if self.failure_count >= self.max_consecutive_failures {
                    self.state = FailoverState::Failed;
                }
            }
        }
    }

    /// Failover manager'ı sıfırla
    pub fn reset(&mut self) {
        self.failure_count = 0;
        self.state = FailoverState::Primary;
        self.current_executor = match self.strategy {
            FailoverStrategy::PrimaryWithFallback { primary, .. } => primary,
            FailoverStrategy::MockOnly => ExecutorType::Mock,
            FailoverStrategy::BinanceOnly => ExecutorType::Binance,
        };
    }

    /// Failover durumunu almak
    pub fn get_status(&self) -> (ExecutorType, FailoverState, u32) {
        (self.current_executor, self.state, self.failure_count)
    }

    /// Max consecutive failures ayarla
    pub fn set_max_failures(&mut self, max: u32) {
        self.max_consecutive_failures = max;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_failover_manager_creation() {
        let manager = FailoverManager::new(FailoverStrategy::PrimaryWithFallback {
            primary: ExecutorType::Binance,
            fallback: ExecutorType::Mock,
        });

        assert_eq!(manager.current_executor(), ExecutorType::Binance);
        assert_eq!(manager.state(), FailoverState::Primary);
    }

    #[test]
    fn test_failover_to_mock() {
        let mut manager = FailoverManager::new(FailoverStrategy::PrimaryWithFallback {
            primary: ExecutorType::Binance,
            fallback: ExecutorType::Mock,
        });

        manager.set_max_failures(2);

        // 2 hata kaydı
        manager.record_failure("API error");
        manager.record_failure("Timeout");

        // Mock'a geçti
        assert_eq!(manager.current_executor(), ExecutorType::Mock);
        assert_eq!(manager.state(), FailoverState::OnFallback);
    }

    #[test]
    fn test_failover_back_to_primary() {
        let mut manager = FailoverManager::new(FailoverStrategy::PrimaryWithFallback {
            primary: ExecutorType::Binance,
            fallback: ExecutorType::Mock,
        });

        manager.set_max_failures(2);

        // Mock'a geç
        manager.record_failure("API error");
        manager.record_failure("Timeout");
        assert_eq!(manager.current_executor(), ExecutorType::Mock);

        // Başarılı işlem kaydı
        manager.record_success();

        // Binance'e geri döndü
        assert_eq!(manager.current_executor(), ExecutorType::Binance);
        assert_eq!(manager.state(), FailoverState::Primary);
    }

    #[test]
    fn test_failover_both_fail() {
        let mut manager = FailoverManager::new(FailoverStrategy::PrimaryWithFallback {
            primary: ExecutorType::Binance,
            fallback: ExecutorType::Mock,
        });

        manager.set_max_failures(2);

        // Binance fail
        manager.record_failure("API error");
        manager.record_failure("Timeout");
        assert_eq!(manager.state(), FailoverState::OnFallback);

        // Mock da fail
        manager.record_failure("Mock error");
        manager.record_failure("Mock timeout");
        assert_eq!(manager.state(), FailoverState::Failed);
    }

    #[test]
    fn test_failover_reset() {
        let mut manager = FailoverManager::new(FailoverStrategy::PrimaryWithFallback {
            primary: ExecutorType::Binance,
            fallback: ExecutorType::Mock,
        });

        manager.set_max_failures(2);
        manager.record_failure("error");
        manager.record_failure("error");

        manager.reset();

        assert_eq!(manager.current_executor(), ExecutorType::Binance);
        assert_eq!(manager.state(), FailoverState::Primary);
    }

    #[test]
    fn test_mock_only_strategy() {
        let manager = FailoverManager::new(FailoverStrategy::MockOnly);
        assert_eq!(manager.current_executor(), ExecutorType::Mock);
    }

    #[test]
    fn test_executor_type_display() {
        assert_eq!(ExecutorType::Binance.to_string(), "Binance");
        assert_eq!(ExecutorType::Mock.to_string(), "Mock");
    }
}
