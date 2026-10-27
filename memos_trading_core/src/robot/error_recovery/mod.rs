// Srivastava ATP Mimarisi - Error Recovery (Tier 3)
//
// Hata yönetimi, circuit breaker pattern, otomatik kurtarma
// Üretim ortamında sistem stabilitesi ve güvenilirliği sağlama

pub mod circuit_breaker;
pub mod failover;
pub mod recovery_state;

pub use circuit_breaker::{
    CircuitBreaker, CircuitBreakerConfig, CircuitBreakerError, CircuitBreakerState,
};
pub use failover::{ExecutorType, FailoverManager, FailoverStrategy};
pub use recovery_state::{RecoveryAction, RecoveryState, RecoveryStateMachine};

#[cfg(test)]
mod tests {

    #[test]
    fn test_error_recovery_module_loads() {
        // Modül başarıyla yüklendiğini kontrol et
        assert!(true);
    }
}
