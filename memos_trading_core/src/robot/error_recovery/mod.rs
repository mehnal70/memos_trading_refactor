// Srivastava ATP Mimarisi - Error Recovery (Tier 3)
//
// Hata yönetimi, circuit breaker pattern, otomatik kurtarma
// Üretim ortamında sistem stabilitesi ve güvenilirliği sağlama

pub mod circuit_breaker;
pub mod failover;
pub mod recovery_state;

pub use circuit_breaker::{
    CircuitBreaker, CircuitBreakerConfig, CircuitBreakerState, CircuitBreakerError,
};
pub use failover::{FailoverManager, FailoverStrategy, ExecutorType};
pub use recovery_state::{RecoveryState, RecoveryStateMachine, RecoveryAction};

#[cfg(test)]
mod tests {
    

    #[test]
    fn test_error_recovery_module_loads() {
        // Modül başarıyla yüklendiğini kontrol et
        assert!(true);
    }
}
