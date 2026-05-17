// health_dashboard.rs
// Sürekli Performans ve Sağlık İzleme Modülü

use chrono::{DateTime, Utc};
use std::collections::VecDeque;

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
    // Performans: Vec yerine VecDeque kullanarak listenin başından silme (O(1)) maliyetini düşürdük
    pub metrics: VecDeque<HealthMetric>,
    // Optimizasyon: Her seferinde tüm listeyi taramamak için kümülatif toplamlar
    total_latency: f64,
    total_fill_rate: f64,
}

impl HealthDashboard {
    pub fn new() -> Self {
        Self {
            metrics: VecDeque::with_capacity(1440),
            total_latency: 0.0,
            total_fill_rate: 0.0,
        }
    }

    /// Yeni bir metrik kaydeder ve pencere boyutunu korur (Sliding Window)
    pub fn record_metric(&mut self, metric: HealthMetric) {
        // Yeni değerleri toplama ekle
        self.total_latency += metric.latency_ms;
        self.total_fill_rate += metric.fill_rate;

        self.metrics.push_back(metric);

        // 1440 kaydı (son 24 saat/dakika vb.) aşınca en eskiyi çıkar
        if self.metrics.len() > 1440 {
            if let Some(old) = self.metrics.pop_front() {
                self.total_latency -= old.latency_ms;
                self.total_fill_rate -= old.fill_rate;
            }
        }
    }

    /// En son kaydedilen metriği döndürür
    #[inline]
    pub fn latest(&self) -> Option<&HealthMetric> {
        self.metrics.back()
    }

    /// Ortalama gecikme (O(1) performans)
    pub fn average_latency(&self) -> f64 {
        let n = self.metrics.len();
        if n == 0 { 0.0 } else { self.total_latency / n as f64 }
    }

    /// Ortalama dolum oranı (O(1) performans)
    pub fn average_fill_rate(&self) -> f64 {
        let n = self.metrics.len();
        if n == 0 { 0.0 } else { self.total_fill_rate / n as f64 }
    }

    /// Dashboard genel durum raporu
    pub fn status(&self) -> String {
        format!(
            "Sağlık Raporu | Örneklem: {} | Avg Latency: {:.2}ms | Avg Fill: {:.2}%",
            self.metrics.len(),
            self.average_latency(),
            self.average_fill_rate() * 100.0
        )
    }
}
