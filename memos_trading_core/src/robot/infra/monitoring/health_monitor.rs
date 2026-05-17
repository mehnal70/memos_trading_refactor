// health_monitor.rs
// Otomatik Sağlık ve Anomali İzleme Modülü

use chrono::{DateTime, Utc};
use std::fmt;

/// Sağlık durumu türleri
#[derive(Debug, Clone, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Warning(String),
    Critical(String),
}

/// Anomali türleri
#[derive(Debug, Clone, PartialEq)]
pub enum AnomalyType {
    LatencySpike(f64),
    DataDelay(u64),
    OrderRejection(String),
    ApiError(String),
    Custom(String),
}

// Display implementasyonu: Loglama ve Dashboard için sıfır maliyetli formatlama
impl fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Healthy => write!(f, "✅ Sağlıklı"),
            Self::Warning(msg) => write!(f, "⚠️ Uyarı: {}", msg),
            Self::Critical(msg) => write!(f, "🚨 KRİTİK: {}", msg),
        }
    }
}

/// Sağlık kontrolü trait'i
pub trait HealthCheck {
    fn check_health(&self) -> HealthStatus;
}

/// Anomali tespit trait'i
pub trait AnomalyDetector {
    fn detect_anomaly(&self) -> Option<AnomalyType>;
}

/// Sağlık ve anomali raporu
#[derive(Debug, Clone)]
pub struct HealthReport {
    pub timestamp: DateTime<Utc>,
    pub status: HealthStatus,
    pub anomalies: Vec<AnomalyType>,
}

impl HealthReport {
    #[inline]
    pub fn new(status: HealthStatus, anomalies: Vec<AnomalyType>) -> Self {
        Self {
            timestamp: Utc::now(),
            status,
            anomalies,
        }
    }
}

// --- OPTİMİZE EDİLMİŞ CHECKER'LAR ---

pub struct LatencyHealthChecker {
    pub latency_ms: f64,
    pub warning_threshold: f64,
    pub critical_threshold: f64,
}

impl HealthCheck for LatencyHealthChecker {
    fn check_health(&self) -> HealthStatus {
        // Modern Rust: if/else yerine aralık tabanlı match (match guards)
        match self.latency_ms {
            l if l > self.critical_threshold => {
                HealthStatus::Critical(format!("Latency kritik: {} ms", l))
            }
            l if l > self.warning_threshold => {
                HealthStatus::Warning(format!("Latency yüksek: {} ms", l))
            }
            _ => HealthStatus::Healthy,
        }
    }
}

impl AnomalyDetector for LatencyHealthChecker {
    fn detect_anomaly(&self) -> Option<AnomalyType> {
        match self.latency_ms {
            l if l > self.critical_threshold => Some(AnomalyType::LatencySpike(l)),
            _ => None,
        }
    }
}

pub struct RobotHealthMonitor {
    pub last_cycle_success: bool,
    pub error_count: usize,
    pub last_error: Option<String>,
}

impl HealthCheck for RobotHealthMonitor {
    fn check_health(&self) -> HealthStatus {
        if !self.last_cycle_success {
            HealthStatus::Warning("Son işlem döngüsü başarısız".to_string())
        } else if self.error_count > 5 {
            HealthStatus::Warning(format!("Yüksek hata oranı: {}", self.error_count))
        } else {
            HealthStatus::Healthy
        }
    }
}

impl AnomalyDetector for RobotHealthMonitor {
    fn detect_anomaly(&self) -> Option<AnomalyType> {
        if !self.last_cycle_success {
            return Some(AnomalyType::Custom("Döngü Kesintisi".to_string()));
        }
        if self.error_count > 10 {
            return Some(AnomalyType::Custom(format!("Kritik Hata Seviyesi: {}", self.error_count)));
        }
        if let Some(err) = &self.last_error {
            if err.contains("RateLimit") || err.contains("429") {
                return Some(AnomalyType::Custom("Borsa Kısıtlaması (Rate Limit)".to_string()));
            }
        }
        None
    }
}
