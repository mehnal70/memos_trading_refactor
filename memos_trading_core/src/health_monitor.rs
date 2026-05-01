// health_monitor.rs
// Otomatik Sağlık ve Anomali İzleme Modülü
// Türkçe açıklamalar ile profesyonel auto trading altyapısı için temel trait ve yapı

use chrono::{DateTime, Utc};

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
    pub fn new(status: HealthStatus, anomalies: Vec<AnomalyType>) -> Self {
        Self {
            timestamp: Utc::now(),
            status,
            anomalies,
        }
    }
}

// Örnek: LatencyHealthChecker
pub struct LatencyHealthChecker {
    pub latency_ms: f64,
    pub warning_threshold: f64,
    pub critical_threshold: f64,
}

impl HealthCheck for LatencyHealthChecker {
    fn check_health(&self) -> HealthStatus {
        if self.latency_ms > self.critical_threshold {
            HealthStatus::Critical(format!("Latency kritik: {} ms", self.latency_ms))
        } else if self.latency_ms > self.warning_threshold {
            HealthStatus::Warning(format!("Latency yüksek: {} ms", self.latency_ms))
        } else {
            HealthStatus::Healthy
        }
    }
}

impl AnomalyDetector for LatencyHealthChecker {
    fn detect_anomaly(&self) -> Option<AnomalyType> {
        if self.latency_ms > self.critical_threshold {
            Some(AnomalyType::LatencySpike(self.latency_ms))
        } else {
            None
        }
    }
}

// Diğer metrikler için benzer checker'lar eklenebilir.
