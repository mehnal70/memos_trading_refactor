// health_dashboard.rs
// Sürekli Performans ve Sağlık İzleme Modülü
// Latency, fill rate, slippage, connectivity, order rejection, Prometheus/Grafana entegrasyonu

use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct HealthMetric {
    pub timestamp: DateTime<Utc>,
    pub latency_ms: f64,
    pub fill_rate: f64,
    pub slippage: f64,
    pub connectivity: f64,
    pub order_rejection_rate: f64,
}

#[derive(Debug, Clone, Default)]
pub struct HealthDashboard {
    pub metrics: Vec<HealthMetric>,
}

impl HealthDashboard {
    pub fn new() -> Self {
        Self { metrics: vec![] }
    }
    pub fn record_metric(&mut self, metric: HealthMetric) {
        self.metrics.push(metric);
        if self.metrics.len() > 1440 {
            self.metrics.remove(0);
        }
    }
    pub fn latest(&self) -> Option<&HealthMetric> {
        self.metrics.last()
    }
    pub fn average_latency(&self) -> f64 {
        let n = self.metrics.len();
        if n == 0 { return 0.0; }
        self.metrics.iter().map(|m| m.latency_ms).sum::<f64>() / n as f64
    }
    pub fn average_fill_rate(&self) -> f64 {
        let n = self.metrics.len();
        if n == 0 { return 0.0; }
        self.metrics.iter().map(|m| m.fill_rate).sum::<f64>() / n as f64
    }
    // Diğer metrikler için benzer fonksiyonlar eklenebilir
}

// Prometheus/Grafana entegrasyonu için exporter fonksiyonları eklenebilir.
