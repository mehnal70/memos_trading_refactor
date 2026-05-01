// system_health_orchestrator.rs
// Merkezi Sistem Sağlık ve Anomali Orkestratörü
// Tüm modüllerin sağlık/anomali durumunu izler ve otomatik aksiyon alır

use crate::health_monitor::{HealthCheck, AnomalyDetector, HealthStatus, AnomalyType};
use crate::risk::RiskManager;
use crate::engine::Engine;
use crate::api::APIHealthMonitor;
use crate::portfolio::Portfolio;
use crate::robot::RobotHealthMonitor;
use crate::robot::advanced_monitoring::{RealtimeDashboard, AlertSystem, PerformanceTrendingEngine};

/// Sistem Sağlık Orkestratörü
pub struct SystemHealthOrchestrator<'a> {
    pub risk: &'a RiskManager,
    pub engine: &'a Engine,
    pub api: &'a APIHealthMonitor,
    pub portfolio: &'a Portfolio,
    pub robot: &'a RobotHealthMonitor,
    pub dashboard: &'a RealtimeDashboard,
    pub alert: &'a AlertSystem,
    pub trending: &'a PerformanceTrendingEngine,
}

impl<'a> SystemHealthOrchestrator<'a> {
    /// Tüm modüllerin sağlık durumunu toplu kontrol et
    pub fn check_all_health(&self) -> Vec<(String, HealthStatus)> {
        vec![
            ("RiskManager".to_string(), self.risk.check_health()),
            ("Engine".to_string(), self.engine.check_health()),
            ("API".to_string(), self.api.check_health()),
            ("Portfolio".to_string(), self.portfolio.check_health()),
            ("Robot".to_string(), self.robot.check_health()),
            ("Dashboard".to_string(), self.dashboard.check_health()),
            ("AlertSystem".to_string(), self.alert.check_health()),
            ("Trending".to_string(), self.trending.check_health()),
        ]
    }

    /// Tüm modüllerde anomali var mı?
    pub fn detect_any_anomaly(&self) -> Vec<(String, Option<AnomalyType>)> {
        vec![
            ("RiskManager".to_string(), self.risk.detect_anomaly()),
            ("Engine".to_string(), self.engine.detect_anomaly()),
            ("API".to_string(), self.api.detect_anomaly()),
            ("Portfolio".to_string(), self.portfolio.detect_anomaly()),
            ("Robot".to_string(), self.robot.detect_anomaly()),
            ("Dashboard".to_string(), self.dashboard.detect_anomaly()),
            ("AlertSystem".to_string(), self.alert.detect_anomaly()),
            ("Trending".to_string(), self.trending.detect_anomaly()),
        ]
    }

    /// Kritik bir sağlık/anomali varsa otomatik aksiyon al (ör: uyarı, sistem durdurma)
    pub fn auto_action(&self) {
        for (name, status) in self.check_all_health() {
            match status {
                HealthStatus::Critical(msg) => {
                    println!("[KRİTİK] {}: {}", name, msg);
                    // Burada sistem durdurma, pozisyon kapama vb. tetiklenebilir
                },
                HealthStatus::Warning(msg) => {
                    println!("[UYARI] {}: {}", name, msg);
                    // Burada uyarı/log/alert tetiklenebilir
                },
                HealthStatus::Healthy => {},
            }
        }
        for (name, anomaly) in self.detect_any_anomaly() {
            if let Some(anom) = anomaly {
                println!("[ANOMALİ] {}: {:?}", name, anom);
                // Burada otomatik aksiyon/alert tetiklenebilir
            }
        }
    }
}
