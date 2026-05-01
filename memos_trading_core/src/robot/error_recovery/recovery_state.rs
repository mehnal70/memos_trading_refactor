// Recovery State Machine - Durum Yönetimi ve Otomatik Kurtarma
//
// Hata durumlarında sistemi farklı durumlar arasında geçir
// Diagnostic'ler çalıştır ve sistem sağlığını iyileştir

use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};

/// Kurtarma Durumları
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryState {
    /// Normal çalışma
    Healthy,
    /// Hafif hata - uyarı yapılmalı
    Degraded,
    /// Ciddi hata - recovery çalış
    Critical,
    /// Sistem durduruldu - manuel müdahale gerekli
    Shutdown,
}

impl std::fmt::Display for RecoveryState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecoveryState::Healthy => write!(f, "Healthy"),
            RecoveryState::Degraded => write!(f, "Degraded"),
            RecoveryState::Critical => write!(f, "Critical"),
            RecoveryState::Shutdown => write!(f, "Shutdown"),
        }
    }
}

/// Kurtarma Aksiyonları
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecoveryAction {
    /// Hiçbir aksiyon - sistem sağlıklı
    None,
    
    /// Uyarı ver - operatörü bilgilendir
    Warn {
        message: String,
        severity: u8,  // 1-10, 10 = en ciddi
    },
    
    /// Sistem kaynakları temizle
    CleanupResources {
        reason: String,
    },
    
    /// Executor'u sıfırla
    ResetExecutor {
        executor_type: String,
    },
    
    /// Failover'ı tetikle
    TriggerFailover {
        from: String,
        to: String,
    },
    
    /// Sistem durdur
    Shutdown {
        reason: String,
    },
}

/// Health Metric
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthMetric {
    /// Metriklerin adı
    pub name: String,
    /// Mevcut değer (0.0 - 1.0)
    pub value: f64,
    /// Eşik değer
    pub threshold: f64,
    /// Son güncelleme zamanı
    pub last_updated: DateTime<Utc>,
}

impl HealthMetric {
    /// Yeni health metric oluştur
    pub fn new(name: String, threshold: f64) -> Self {
        Self {
            name,
            value: 1.0,  // Sağlıklı olarak başla
            threshold,
            last_updated: Utc::now(),
        }
    }

    /// Metrik sağlıklı mı?
    pub fn is_healthy(&self) -> bool {
        self.value >= self.threshold
    }

    /// Metriği güncelle
    pub fn update(&mut self, value: f64) {
        self.value = value.clamp(0.0, 1.0);
        self.last_updated = Utc::now();
    }
}

/// Recovery State Machine
pub struct RecoveryStateMachine {
    current_state: RecoveryState,
    health_metrics: Vec<HealthMetric>,
    last_state_change: DateTime<Utc>,
    consecutive_healthy_checks: u32,
    consecutive_critical_checks: u32,
    max_checks_to_recover: u32,
    max_checks_to_critical: u32,
}

impl RecoveryStateMachine {
    /// Yeni Recovery State Machine oluştur
    pub fn new() -> Self {
        let mut machine = Self {
            current_state: RecoveryState::Healthy,
            health_metrics: Vec::new(),
            last_state_change: Utc::now(),
            consecutive_healthy_checks: 0,
            consecutive_critical_checks: 0,
            max_checks_to_recover: 3,     // 3 healthy check sonra recover
            max_checks_to_critical: 2,    // 2 critical check sonra shutdown
        };

        // Varsayılan metrikleri ekle
        machine.add_metric(HealthMetric::new("executor_health".to_string(), 0.8));
        machine.add_metric(HealthMetric::new("api_response_time".to_string(), 0.7));
        machine.add_metric(HealthMetric::new("error_rate".to_string(), 0.8));
        machine.add_metric(HealthMetric::new("memory_usage".to_string(), 0.7));

        machine
    }

    /// Metric ekle
    pub fn add_metric(&mut self, metric: HealthMetric) {
        self.health_metrics.push(metric);
    }

    /// Metrik güncelle
    pub fn update_metric(&mut self, name: &str, value: f64) -> Result<(), String> {
        let metric = self
            .health_metrics
            .iter_mut()
            .find(|m| m.name == name)
            .ok_or_else(|| format!("Metric not found: {}", name))?;

        metric.update(value);
        Ok(())
    }

    /// Mevcut durumu döndür
    pub fn state(&self) -> RecoveryState {
        self.current_state
    }

    /// Sistem sağlığını kontrol et ve durumu güncelle
    pub fn check_health(&mut self) -> RecoveryAction {
        // Tüm metriklerin ortalama sağlığını hesapla
        if self.health_metrics.is_empty() {
            return RecoveryAction::None;
        }

        let avg_health: f64 = self.health_metrics.iter().map(|m| m.value).sum::<f64>()
            / self.health_metrics.len() as f64;

        let unhealthy_metrics = self
            .health_metrics
            .iter()
            .filter(|m| !m.is_healthy())
            .count();

        // Durum makinesini güncelle
        match self.current_state {
            RecoveryState::Healthy => {
                if unhealthy_metrics > 0 || avg_health < 0.7 {
                    // Degraded durumuna geç
                    self.current_state = RecoveryState::Degraded;
                    self.last_state_change = Utc::now();
                    self.consecutive_critical_checks = 0;

                    return RecoveryAction::Warn {
                        message: format!(
                            "System degraded: {} metrics unhealthy, avg health: {:.2}",
                            unhealthy_metrics, avg_health
                        ),
                        severity: 5,
                    };
                }
                self.consecutive_healthy_checks += 1;
            }

            RecoveryState::Degraded => {
                if unhealthy_metrics > 2 || avg_health < 0.5 {
                    // Critical durumuna geç
                    self.current_state = RecoveryState::Critical;
                    self.last_state_change = Utc::now();
                    self.consecutive_critical_checks += 1;

                    return RecoveryAction::Warn {
                        message: format!("System critical: {} metrics unhealthy", unhealthy_metrics),
                        severity: 9,
                    };
                } else if unhealthy_metrics == 0 && avg_health >= 0.8 {
                    // Healthy'e geri dön
                    self.consecutive_healthy_checks += 1;

                    if self.consecutive_healthy_checks >= self.max_checks_to_recover {
                        self.current_state = RecoveryState::Healthy;
                        self.last_state_change = Utc::now();
                        self.consecutive_healthy_checks = 0;

                        return RecoveryAction::Warn {
                            message: "System recovered to healthy state".to_string(),
                            severity: 1,
                        };
                    }
                } else {
                    self.consecutive_healthy_checks = 0;
                }
            }

            RecoveryState::Critical => {
                self.consecutive_critical_checks += 1;

                if unhealthy_metrics == 0 && avg_health >= 0.8 {
                    // Degraded'e geri dön
                    self.current_state = RecoveryState::Degraded;
                    self.last_state_change = Utc::now();
                    self.consecutive_critical_checks = 0;

                    return RecoveryAction::CleanupResources {
                        reason: "Exiting critical state".to_string(),
                    };
                } else if self.consecutive_critical_checks >= self.max_checks_to_critical {
                    // Shutdown'a geç
                    self.current_state = RecoveryState::Shutdown;
                    self.last_state_change = Utc::now();

                    return RecoveryAction::Shutdown {
                        reason: format!(
                            "Critical state persisted too long. {} metrics unhealthy",
                            unhealthy_metrics
                        ),
                    };
                }

                return RecoveryAction::CleanupResources {
                    reason: "System in critical state, attempting recovery".to_string(),
                };
            }

            RecoveryState::Shutdown => {
                // Shutdown durumundan çıkamaz (manuel müdahale gerekli)
                return RecoveryAction::None;
            }
        }

        RecoveryAction::None
    }

    /// State machine'i sıfırla
    pub fn reset(&mut self) {
        self.current_state = RecoveryState::Healthy;
        self.last_state_change = Utc::now();
        self.consecutive_healthy_checks = 0;
        self.consecutive_critical_checks = 0;

        for metric in &mut self.health_metrics {
            metric.value = 1.0;
            metric.last_updated = Utc::now();
        }
    }

    /// Sistem sağlığının özeti
    pub fn get_summary(&self) -> (RecoveryState, f64, Vec<String>) {
        let avg_health = if self.health_metrics.is_empty() {
            1.0
        } else {
            self.health_metrics.iter().map(|m| m.value).sum::<f64>()
                / self.health_metrics.len() as f64
        };

        let unhealthy = self
            .health_metrics
            .iter()
            .filter(|m| !m.is_healthy())
            .map(|m| m.name.clone())
            .collect();

        (self.current_state, avg_health, unhealthy)
    }
}

impl Default for RecoveryStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recovery_state_machine_creation() {
        let machine = RecoveryStateMachine::new();
        assert_eq!(machine.state(), RecoveryState::Healthy);
        assert!(!machine.health_metrics.is_empty());
    }

    #[test]
    fn test_health_metric_update() {
        let mut metric = HealthMetric::new("test".to_string(), 0.5);
        assert!(metric.is_healthy());

        metric.update(0.3);
        assert!(!metric.is_healthy());
    }

    #[test]
    fn test_transition_healthy_to_degraded() {
        let mut machine = RecoveryStateMachine::new();

        // Bir metriği kötü yap
        let _ = machine.update_metric("executor_health", 0.3);

        let action = machine.check_health();
        assert_eq!(machine.state(), RecoveryState::Degraded);
        
        match action {
            RecoveryAction::Warn { severity, .. } => assert_eq!(severity, 5),
            _ => panic!("Expected Warn action"),
        }
    }

    #[test]
    fn test_transition_degraded_to_critical() {
        let mut machine = RecoveryStateMachine::new();

        // Çoklu metrikleri kötü yap
        let _ = machine.update_metric("executor_health", 0.2);
        let _ = machine.update_metric("api_response_time", 0.3);
        let _ = machine.update_metric("error_rate", 0.4);

        machine.check_health();
        let action = machine.check_health();
        
        assert_eq!(machine.state(), RecoveryState::Critical);
        match action {
            RecoveryAction::Warn { severity, .. } => assert_eq!(severity, 9),
            _ => panic!("Expected Warn action"),
        }
    }

    #[test]
    fn test_recovery_to_healthy() {
        let mut machine = RecoveryStateMachine::new();

        // Critical'e git
        let _ = machine.update_metric("executor_health", 0.2);
        let _ = machine.update_metric("api_response_time", 0.3);
        let _ = machine.update_metric("error_rate", 0.4);
        machine.check_health();

        // Tüm metrikleri iyi yap
        let _ = machine.update_metric("executor_health", 0.95);
        let _ = machine.update_metric("api_response_time", 0.95);
        let _ = machine.update_metric("error_rate", 0.95);
        let _ = machine.update_metric("memory_usage", 0.95);

        // Recovery checks sayısı kadar kontrol et
        for _ in 0..machine.max_checks_to_recover + 1 {
            machine.check_health();
        }

        assert_eq!(machine.state(), RecoveryState::Healthy);
    }

    #[test]
    fn test_state_summary() {
        let mut machine = RecoveryStateMachine::new();
        let _ = machine.update_metric("executor_health", 0.3);

        let (state, _avg_health, unhealthy) = machine.get_summary();
        
        assert_eq!(state, RecoveryState::Healthy);  // Hala healthy ama metric kötü
        assert!(!unhealthy.is_empty());
    }

    #[test]
    fn test_reset() {
        let mut machine = RecoveryStateMachine::new();
        let _ = machine.update_metric("executor_health", 0.2);

        machine.reset();

        assert_eq!(machine.state(), RecoveryState::Healthy);
        for metric in &machine.health_metrics {
            assert_eq!(metric.value, 1.0);
        }
    }
}
