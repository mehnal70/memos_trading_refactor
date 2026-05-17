// system_health_orchestrator.rs
// Merkezi Sistem Sağlık ve Anomali Orkestratörü

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
    /// Tüm modüllerin durumlarını kopyalama yapmadan (lazy) döndüren yardımcı metod
    /// Performans: Bellekte Vec oluşturmaz, doğrudan bir dizi (array) döndürür.
    fn components(&self) -> [(&'static str, &dyn HealthCheck, &dyn AnomalyDetector); 8] {
        [
            ("RiskManager", self.risk, self.risk),
            ("Engine", self.engine, self.engine),
            ("API", self.api, self.api),
            ("Portfolio", self.portfolio, self.portfolio),
            ("Robot", self.robot, self.robot),
            ("Dashboard", self.dashboard, self.dashboard),
            ("AlertSystem", self.alert, self.alert),
            ("Trending", self.trending, self.trending),
        ]
    }

    /// Tüm modülleri toplu kontrol et - Performans: Gereksiz String kopyalamaları temizlendi.
    pub fn check_all_health(&self) -> Vec<(&'static str, HealthStatus)> {
        self.components()
            .iter()
            .map(|(name, check, _)| (*name, check.check_health()))
            .collect()
    }

    /// Tüm modüllerde anomali tara
    pub fn detect_any_anomaly(&self) -> Vec<(&'static str, AnomalyType)> {
        self.components()
            .iter()
            .filter_map(|(name, _, det)| {
                det.detect_anomaly().map(|a| (*name, a))
            })
            .collect()
    }

    /// Otonom Karar Mekanizması: Kritik durumlarda anında aksiyon alır.
    pub fn auto_action(&self) {
        // Tek bir döngüde hem sağlık hem anomali kontrolü (Performans optimizasyonu)
        for (name, check, det) in self.components() {
            
            // 1. Sağlık Durumu Değerlendirmesi
            match check.check_health() {
                HealthStatus::Critical(msg) => {
                    eprintln!("[🚨 KRİTİK] {}: {}", name, msg);
                    // Acil Durum: Burada Kill-Switch tetiklenebilir
                    self.trigger_emergency_stop(name);
                },
                HealthStatus::Warning(msg) => {
                    println!("[⚠️ UYARI] {}: {}", name, msg);
                },
                HealthStatus::Healthy => (),
            }

            // 2. Anomali Tespiti
            if let Some(anomaly) = det.detect_anomaly() {
                eprintln!("[🔍 ANOMALİ] {}: {:?}", name, anomaly);
                // Otonom aksiyon: self.handle_anomaly(name, anomaly);
            }
        }
    }

    /// Acil durum durdurma mantığı
    fn trigger_emergency_stop(&self, source: &str) {
        // Gerçek implementasyonda portföyü dondurur ve açık pozisyonları yönetir
        println!("[ACTION] {} kaynaklı güvenlik durdurması başlatıldı.", source);
    }
}
