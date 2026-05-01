// Alert System - Email ve Slack Bildirimleri
//
// Kritik olayları tespit et: Max DD aşıldı, Sistem hatası, Kazanç milestone
// Email ve Slack aracılığıyla uyarılar gönder

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use crate::health_monitor::{HealthCheck, AnomalyDetector, HealthStatus, AnomalyType};

/// Alert Seviyeleri
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertLevel {
    /// Bilgi (Info)
    Info,
    /// Uyarı (Warning)
    Warning,
    /// Hata (Error)
    Error,
    /// Kritik (Critical)
    Critical,
}

/// Alert Kanalları
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertChannel {
    /// Email
    Email,
    /// Slack
    Slack,
    /// SMS
    SMS,
    /// Log dosyası
    Log,
}

/// Tek Alert Mesajı
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    /// Alert ID
    pub id: String,
    
    /// Oluşturulma zamanı
    pub timestamp: DateTime<Utc>,
    
    /// Alert seviyesi
    pub level: AlertLevel,
    
    /// Alert başlığı
    pub title: String,
    
    /// Alert mesajı
    pub message: String,
    
    /// Hedef kanallar
    pub channels: Vec<AlertChannel>,
    
    /// Gönderildi mi?
    pub sent: bool,
    
    /// Gönderme zamanı
    pub sent_at: Option<DateTime<Utc>>,
}

/// Alert Konfigürasyonu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertConfig {
    /// Max DD threshold (%)
    pub max_dd_threshold: f64,
    
    /// Minimum kazanç milestone ($)
    pub profit_milestone: f64,
    
    /// Email adresleri
    pub email_recipients: Vec<String>,
    
    /// Slack webhook URL
    pub slack_webhook: Option<String>,
    
    /// SMS nummaraları
    pub sms_recipients: Vec<String>,
    
    /// Alert gizleme süresi (saniye)
    pub alert_cooldown_secs: u64,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            max_dd_threshold: 20.0,
            profit_milestone: 10000.0,
            email_recipients: vec![],
            slack_webhook: None,
            sms_recipients: vec![],
            alert_cooldown_secs: 300,
        }
    }
}

/// Alert Sistemi
pub struct AlertSystem {
    /// Alert geçmişi
    alerts_history: VecDeque<Alert>,
    
    /// Konfigürasyon
    config: AlertConfig,
    
    /// Son alert zamanı (cooldown için)
    last_alert_time: Option<DateTime<Utc>>,
    
    /// Alert sayacı
    alert_counter: usize,
    
    /// Sistem aktif mi?
    is_active: bool,
}

impl AlertSystem {
    /// Yeni Alert Sistemi oluştur
    pub fn new(config: AlertConfig) -> Self {
        Self {
            alerts_history: VecDeque::new(),
            config,
            last_alert_time: None,
            alert_counter: 0,
            is_active: false,
        }
    }
}

// AlertSystem için HealthCheck ve AnomalyDetector trait implementasyonları
impl HealthCheck for AlertSystem {
    fn check_health(&self) -> HealthStatus {
        let (_, warning, error, critical) = self.get_stats();
        if critical > 0 {
            HealthStatus::Critical(format!("{} kritik alert!", critical))
        } else if error > 0 {
            HealthStatus::Warning(format!("{} hata alerti!", error))
        } else if warning > 0 {
            HealthStatus::Warning(format!("{} uyarı alerti!", warning))
        } else {
            HealthStatus::Healthy
        }
    }
}

impl AnomalyDetector for AlertSystem {
    fn detect_anomaly(&self) -> Option<AnomalyType> {
        let (_, _, _, critical) = self.get_stats();
        if critical > 0 {
            return Some(AnomalyType::Custom(format!("Kritik alert sayısı: {}", critical)));
        }
        None
    }
}

impl AlertSystem {
    /// Alert sistemini başlat
    pub fn start(&mut self) -> Result<(), String> {
        if self.is_active {
            return Err("Alert system already active".to_string());
        }
        
        self.is_active = true;
        println!("✓ Alert System başlatıldı");
        Ok(())
    }

    /// Alert sistemini durdur
    pub fn stop(&mut self) -> Result<(), String> {
        if !self.is_active {
            return Err("Alert system not active".to_string());
        }
        
        self.is_active = false;
        println!("✓ Alert System durduruldu");
        Ok(())
    }

    /// Max DD uyarısını tetikle
    pub fn trigger_max_dd_alert(&mut self, current_dd: f64) -> Result<Option<Alert>, String> {
        if !self.is_active {
            return Err("Alert system not active".to_string());
        }

        if current_dd > self.config.max_dd_threshold {
            self.create_alert(
                AlertLevel::Critical,
                "Max Drawdown Aşıldı".to_string(),
                format!(
                    "Maksimum çekilme threshold'u aşıldı! Mevcut: {:.2}%, Eşik: {:.2}%",
                    current_dd, self.config.max_dd_threshold
                ),
                vec![AlertChannel::Email, AlertChannel::Slack],
            )
        } else {
            Ok(None)
        }
    }

    /// Kazanç milestone uyarısını tetikle
    pub fn trigger_profit_milestone_alert(&mut self, current_profit: f64, milestone: Option<f64>) -> Result<Option<Alert>, String> {
        if !self.is_active {
            return Err("Alert system not active".to_string());
        }

        let threshold = milestone.unwrap_or(self.config.profit_milestone);

        if current_profit >= threshold && current_profit < threshold + 1000.0 {
            self.create_alert(
                AlertLevel::Info,
                "Kazanç Milestone Ulaşıldı".to_string(),
                format!("Tebrikler! ${:.2} kazanç milestone'ı ulaşıldı.", threshold),
                vec![AlertChannel::Email, AlertChannel::Slack],
            )
        } else {
            Ok(None)
        }
    }

    /// Sistem hatası uyarısı
    pub fn trigger_system_error_alert(&mut self, error_message: String) -> Result<Option<Alert>, String> {
        if !self.is_active {
            return Err("Alert system not active".to_string());
        }

        self.create_alert(
            AlertLevel::Error,
            "Sistem Hatası".to_string(),
            format!("Sistem hatası oluştu: {}", error_message),
            vec![AlertChannel::Email, AlertChannel::Log],
        )
    }

    /// Özel alert oluştur
    pub fn create_custom_alert(
        &mut self,
        level: AlertLevel,
        title: String,
        message: String,
        channels: Vec<AlertChannel>,
    ) -> Result<Option<Alert>, String> {
        if !self.is_active {
            return Err("Alert system not active".to_string());
        }

        self.create_alert(level, title, message, channels)
    }

    /// Alert oluştur (iç fonksiyon)
    fn create_alert(
        &mut self,
        level: AlertLevel,
        title: String,
        message: String,
        channels: Vec<AlertChannel>,
    ) -> Result<Option<Alert>, String> {
        // Cooldown kontrolü
        if let Some(last_time) = self.last_alert_time {
            let elapsed = (Utc::now() - last_time).num_seconds() as u64;
            if elapsed < self.config.alert_cooldown_secs {
                return Ok(None);  // Cooldown'da
            }
        }

        self.alert_counter += 1;
        let alert = Alert {
            id: format!("alert_{}", self.alert_counter),
            timestamp: Utc::now(),
            level,
            title,
            message: message.clone(),
            channels: channels.clone(),
            sent: false,
            sent_at: None,
        };

        println!("🔔 Alert [{:?}]: {}", level, alert.title);

        // Kanallara gönder
        self.send_alert(&alert)?;

        self.last_alert_time = Some(Utc::now());
        self.alerts_history.push_back(alert.clone());

        // Maksimum 1000 alert sakla
        if self.alerts_history.len() > 1000 {
            self.alerts_history.pop_front();
        }

        Ok(Some(alert))
    }

    /// Alert gönder
    fn send_alert(&mut self, alert: &Alert) -> Result<(), String> {
        for channel in &alert.channels {
            match channel {
                AlertChannel::Email => {
                    println!("📧 Email gönderiliyor: {:?}", self.config.email_recipients);
                },
                AlertChannel::Slack => {
                    println!("💬 Slack gönderiliyor: {}", self.config.slack_webhook.as_ref().unwrap_or(&"".to_string()));
                },
                AlertChannel::SMS => {
                    println!("📱 SMS gönderiliyor: {:?}", self.config.sms_recipients);
                },
                AlertChannel::Log => {
                    println!("📝 Log dosyasına yazılıyor");
                },
            }
        }

        Ok(())
    }

    /// Alert geçmişini al
    pub fn get_alerts(&self, limit: usize) -> Vec<Alert> {
        self.alerts_history
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    /// Alert istatistikleri
    pub fn get_stats(&self) -> (usize, usize, usize, usize) {
        let info = self.alerts_history.iter().filter(|a| a.level == AlertLevel::Info).count();
        let warning = self.alerts_history.iter().filter(|a| a.level == AlertLevel::Warning).count();
        let error = self.alerts_history.iter().filter(|a| a.level == AlertLevel::Error).count();
        let critical = self.alerts_history.iter().filter(|a| a.level == AlertLevel::Critical).count();

        (info, warning, error, critical)
    }

    /// Sistem aktif mi?
    pub fn is_active(&self) -> bool {
        self.is_active
    }
}

impl Default for AlertSystem {
    fn default() -> Self {
        Self::new(AlertConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alert_system_creation() {
        let config = AlertConfig::default();
        let system = AlertSystem::new(config);
        
        assert!(!system.is_active());
    }

    #[test]
    fn test_alert_system_start_stop() {
        let config = AlertConfig::default();
        let mut system = AlertSystem::new(config);
        
        assert!(system.start().is_ok());
        assert!(system.is_active());
        
        assert!(system.stop().is_ok());
        assert!(!system.is_active());
    }

    #[test]
    fn test_cannot_create_alert_while_inactive() {
        let config = AlertConfig::default();
        let mut system = AlertSystem::new(config);
        
        let result = system.create_custom_alert(
            AlertLevel::Warning,
            "Test".to_string(),
            "Test message".to_string(),
            vec![AlertChannel::Email],
        );
        
        assert!(result.is_err());
    }

    #[test]
    fn test_max_dd_alert() {
        let config = AlertConfig {
            max_dd_threshold: 20.0,
            ..Default::default()
        };
        let mut system = AlertSystem::new(config);
        system.start().unwrap();
        
        let result = system.trigger_max_dd_alert(25.0);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_profit_milestone_alert() {
        let config = AlertConfig {
            profit_milestone: 10000.0,
            ..Default::default()
        };
        let mut system = AlertSystem::new(config);
        system.start().unwrap();
        
        let result = system.trigger_profit_milestone_alert(10500.0, None);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_alert_cooldown() {
        let config = AlertConfig {
            alert_cooldown_secs: 60,
            ..Default::default()
        };
        let mut system = AlertSystem::new(config);
        system.start().unwrap();
        
        let _ = system.trigger_max_dd_alert(25.0);
        let result = system.trigger_max_dd_alert(26.0);
        
        // İkinci alert cooldown'da
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_alert_stats() {
        let config = AlertConfig {
            alert_cooldown_secs: 0,
            ..Default::default()
        };
        let mut system = AlertSystem::new(config);
        system.start().unwrap();
        
        let _ = system.create_custom_alert(AlertLevel::Info, "Info".to_string(), "msg".to_string(), vec![]);
        let _ = system.create_custom_alert(AlertLevel::Warning, "Warn".to_string(), "msg".to_string(), vec![]);
        let _ = system.create_custom_alert(AlertLevel::Critical, "Crit".to_string(), "msg".to_string(), vec![]);
        
        let (info, warning, error, critical) = system.get_stats();
        assert_eq!(info, 1);
        assert_eq!(warning, 1);
        assert_eq!(error, 0);
        assert_eq!(critical, 1);
    }
}
