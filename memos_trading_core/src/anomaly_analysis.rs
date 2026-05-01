// anomaly_analysis.rs
// Otonom Hata ve Anomali Analizi Modülü
// Fiyat, latency, veri tutarsızlığı, API hatası tespiti ve otomatik aksiyon

use chrono::{DateTime, Utc};

/// Anomali türleri
#[derive(Debug, Clone, PartialEq)]
pub enum AnomalyKind {
    PriceSpike(f64),
    LatencySpike(f64),
    DataInconsistency(String),
    ApiError(String),
    Custom(String),
}

/// Anomali kaydı
#[derive(Debug, Clone)]
pub struct AnomalyRecord {
    pub kind: AnomalyKind,
    pub detected_at: DateTime<Utc>,
    pub resolved: bool,
    pub resolution: Option<String>,
}

/// Anomali analiz trait'i
pub trait AnomalyAnalysis {
    fn detect(&self) -> Vec<AnomalyRecord>;
    fn auto_action(&self, anomaly: &AnomalyRecord);
}

/// Basit örnek: Latency ve fiyat spike tespiti
pub struct SimpleAnomalyAnalyzer {
    pub last_latency_ms: f64,
    pub last_price_change: f64,
    pub latency_threshold: f64,
    pub price_spike_threshold: f64,
}

impl AnomalyAnalysis for SimpleAnomalyAnalyzer {
    fn detect(&self) -> Vec<AnomalyRecord> {
        let mut anomalies = vec![];
        if self.last_latency_ms > self.latency_threshold {
            anomalies.push(AnomalyRecord {
                kind: AnomalyKind::LatencySpike(self.last_latency_ms),
                detected_at: Utc::now(),
                resolved: false,
                resolution: None,
            });
        }
        if self.last_price_change.abs() > self.price_spike_threshold {
            anomalies.push(AnomalyRecord {
                kind: AnomalyKind::PriceSpike(self.last_price_change),
                detected_at: Utc::now(),
                resolved: false,
                resolution: None,
            });
        }
        anomalies
    }
    fn auto_action(&self, anomaly: &AnomalyRecord) {
        match &anomaly.kind {
            AnomalyKind::LatencySpike(val) => println!("[ANOMALİ] Yüksek latency: {} ms", val),
            AnomalyKind::PriceSpike(val) => println!("[ANOMALİ] Fiyat spike: {}", val),
            AnomalyKind::DataInconsistency(msg) => println!("[ANOMALİ] Veri tutarsızlığı: {}", msg),
            AnomalyKind::ApiError(msg) => println!("[ANOMALİ] API hatası: {}", msg),
            AnomalyKind::Custom(msg) => println!("[ANOMALİ] {}", msg),
        }
        // Burada otomatik aksiyon (pozisyon kapama, sistem durdurma, uyarı) tetiklenebilir
    }
}
