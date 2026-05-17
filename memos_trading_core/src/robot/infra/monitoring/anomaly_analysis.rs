// anomaly_analysis.rs
// Otonom Hata ve Anomali Analizi Modülü

use chrono::{DateTime, Utc};
use std::fmt;

/// Anomali türleri - Bellek dostu ve esnek
#[derive(Debug, Clone, PartialEq)]
pub enum AnomalyKind {
    PriceSpike(f64),
    LatencySpike(f64),
    DataInconsistency(String),
    ApiError(String),
    Custom(String),
}

// Display trait'ini implement ederek loglama performansını artırıyoruz (Allocationsız)
impl fmt::Display for AnomalyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PriceSpike(val) => write!(f, "Fiyat Spike: {:.2}", val),
            Self::LatencySpike(val) => write!(f, "Yüksek Latency: {} ms", val),
            Self::DataInconsistency(msg) => write!(f, "Veri Tutarsızlığı: {}", msg),
            Self::ApiError(msg) => write!(f, "API Hatası: {}", msg),
            Self::Custom(msg) => write!(f, "Özel: {}", msg),
        }
    }
}

/// Anomali kaydı - Küçük ve Verimli
#[derive(Debug, Clone)]
pub struct AnomalyRecord {
    pub kind: AnomalyKind,
    pub detected_at: DateTime<Utc>,
    pub resolved: bool,
    pub resolution: Option<String>,
}

impl AnomalyRecord {
    #[inline] // Performans için küçük fonksiyonu inline ediyoruz
    pub fn new(kind: AnomalyKind) -> Self {
        Self {
            kind,
            detected_at: Utc::now(),
            resolved: false,
            resolution: None,
        }
    }
}

/// Anomali analiz trait'i
pub trait AnomalyAnalysis {
    fn detect(&self) -> Vec<AnomalyRecord>;
    fn auto_action(&self, anomaly: &AnomalyRecord);
}

pub struct SimpleAnomalyAnalyzer {
    pub last_latency_ms: f64,
    pub last_price_change: f64,
    pub latency_threshold: f64,
    pub price_spike_threshold: f64,
}

impl AnomalyAnalysis for SimpleAnomalyAnalyzer {
    /// Detect fonksiyonunu iteratör mantığıyla optimize ediyoruz
    fn detect(&self) -> Vec<AnomalyRecord> {
        let mut anomalies = Vec::with_capacity(2); // Önceden bellek ayırarak (allocation) hızı artırıyoruz

        if self.last_latency_ms > self.latency_threshold {
            anomalies.push(AnomalyRecord::new(AnomalyKind::LatencySpike(self.last_latency_ms)));
        }

        if self.last_price_change.abs() > self.price_spike_threshold {
            anomalies.push(AnomalyRecord::new(AnomalyKind::PriceSpike(self.last_price_change)));
        }

        anomalies
    }

    fn auto_action(&self, anomaly: &AnomalyRecord) {
        // Display trait kullanımı sayesinde formatlama maliyetini düşürdük
        println!(
            "[{}] [ANOMALİ] {}", 
            anomaly.detected_at.format("%H:%M:%S"), 
            anomaly.kind 
        );

        // Kritik aksiyonlar için Match Guards (Modern Kontrol)
        match &anomaly.kind {
            AnomalyKind::PriceSpike(val) if val.abs() > 10.0 => {
                self.emergency_stop("Aşırı fiyat hareketi!");
            }
            AnomalyKind::ApiError(_) => {
                self.reconnect_service();
            }
            _ => {}
        }
    }
}

impl SimpleAnomalyAnalyzer {
    fn emergency_stop(&self, reason: &str) {
        // Zero-copy: string kopyalamadan referansla logluyoruz
        eprintln!("!!! ACİL DURUM STOP: {}", reason);
    }

    fn reconnect_service(&self) {
        println!("Servis yeniden bağlanıyor...");
    }
}
