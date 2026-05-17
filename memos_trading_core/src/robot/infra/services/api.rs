use crate::robot::infra::monitoring::health_monitor::{HealthCheck, AnomalyDetector, HealthStatus, AnomalyType};

pub struct APIHealthMonitor {
    pub last_status_code: Option<u16>,
    pub last_error: Option<String>,
}

impl HealthCheck for APIHealthMonitor {
    fn check_health(&self) -> HealthStatus {
        match (self.last_status_code, &self.last_error) {
            // 5xx Hataları: Kritik
            (Some(code @ 500..=599), _) => {
                HealthStatus::Critical(format!("API sunucu hatası: {}", code))
            }
            // 4xx Hataları veya Yanıt Yokken Gelen Hata Mesajı: Uyarı
            (Some(code @ 400..=499), _) => {
                HealthStatus::Warning(format!("API istemci hatası: {}", code))
            }
            (None, _) => {
                HealthStatus::Warning("API yanıtı yok".to_string())
            }
            // Diğer durumlar (2xx, 3xx vb.)
            _ => HealthStatus::Healthy,
        }
    }
}

impl AnomalyDetector for APIHealthMonitor {
    fn detect_anomaly(&self) -> Option<AnomalyType> {
        match (self.last_status_code, &self.last_error) {
            // Durum kodu hatası varsa öncelikli olarak onu dön
            (Some(code @ 400..=599), _) => {
                let prefix = if code >= 500 { "Sunucu" } else { "İstemci" };
                Some(AnomalyType::ApiError(format!("{} hatası: {}", prefix, code)))
            }
            // Durum kodu hatası yok ama spesifik bir hata metni varsa onu dön
            (_, Some(err)) => {
                Some(AnomalyType::ApiError(err.clone()))
            }
            // Her şey normal
            _ => None,
        }
    }
}
