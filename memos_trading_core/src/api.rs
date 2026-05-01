// API Sağlık ve Anomali İzleme Modülü
// Türkçe açıklamalar ile temel HealthCheck ve AnomalyDetector örneği

use crate::health_monitor::{HealthCheck, AnomalyDetector, HealthStatus, AnomalyType};

pub struct APIHealthMonitor {
	pub last_status_code: Option<u16>,
	pub last_error: Option<String>,
}

impl HealthCheck for APIHealthMonitor {
	fn check_health(&self) -> HealthStatus {
		if let Some(code) = self.last_status_code {
			if code >= 500 {
				HealthStatus::Critical(format!("API sunucu hatası: {}", code))
			} else if code >= 400 {
				HealthStatus::Warning(format!("API istemci hatası: {}", code))
			} else {
				HealthStatus::Healthy
			}
		} else {
			HealthStatus::Warning("API yanıtı yok".to_string())
		}
	}
}

impl AnomalyDetector for APIHealthMonitor {
	fn detect_anomaly(&self) -> Option<AnomalyType> {
		if let Some(code) = self.last_status_code {
			if code >= 500 {
				return Some(AnomalyType::ApiError(format!("Sunucu hatası: {}", code)));
			} else if code >= 400 {
				return Some(AnomalyType::ApiError(format!("İstemci hatası: {}", code)));
			}
		}
		if let Some(err) = &self.last_error {
			return Some(AnomalyType::ApiError(err.clone()));
		}
		None
	}
}
