// alert_system.rs - Akıllı Bildirim ve Uyarı Yönetim Modülü

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use crate::health_monitor::{HealthCheck, AnomalyDetector, HealthStatus, AnomalyType};

// --- 1. VERİ MODELLERİ ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertLevel {
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertChannel {
    Email,
    Slack,
    SMS,
    Log,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub level: AlertLevel,
    pub title: String,
    pub message: String,
    pub channels: Vec<AlertChannel>,
    pub sent: bool,
    pub sent_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertConfig {
    pub max_dd_threshold: f64,
    pub profit_milestone: f64,
    pub email_recipients: Vec<String>,
    pub slack_webhook: Option<String>,
    pub sms_recipients: Vec<String>,
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

// --- 2. ANA SİSTEM ---

pub struct AlertSystem {
    alerts_history: VecDeque<Alert>,
    config: AlertConfig,
    last_alert_time: Option<DateTime<Utc>>,
    alert_counter: usize,
    is_active: bool,
}

impl AlertSystem {
    pub fn new(config: AlertConfig) -> Self {
        Self {
            alerts_history: VecDeque::with_capacity(1000),
            config,
            last_alert_time: None,
            alert_counter: 0,
            is_active: false,
        }
    }

    /// Alert sistemini asenkron hazırlıkla başlatır
    pub fn start(&mut self) -> Result<(), String> {
        if self.is_active { return Err("Alert sistemi zaten aktif".to_owned()); }
        self.is_active = true;
        println!("✓ Alert System otonom modda başlatıldı");
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        if !self.is_active { return Err("Alert sistemi aktif değil".to_owned()); }
        self.is_active = false;
        Ok(())
    }

    /// İç metod: Alert oluşturur, cooldown kontrolü yapar ve kanallara iletir.
    fn create_alert(
        &mut self,
        level: AlertLevel,
        title: String,
        message: String,
        channels: Vec<AlertChannel>,
    ) -> Result<Option<Alert>, String> {
        // 1. Cooldown Kontrolü
        if let Some(last) = self.last_alert_time {
            if (Utc::now() - last).num_seconds() < self.config.alert_cooldown_secs as i64 {
                return Ok(None);
            }
        }

        self.alert_counter += 1;
        let mut alert = Alert {
            id: format!("ALRT_{}", self.alert_counter),
            timestamp: Utc::now(),
            level,
            title,
            message,
            channels,
            sent: false,
            sent_at: None,
        };

        // 2. Kanallara Gönder (Dispatch)
        self.send_alert_to_channels(&alert)?;
        
        alert.sent = true;
        alert.sent_at = Some(Utc::now());

        // 3. Geçmişe Kaydet ve Pencereyi Koru
        self.last_alert_time = Some(Utc::now());
        self.alerts_history.push_back(alert.clone());
        if self.alerts_history.len() > 1000 { self.alerts_history.pop_front(); }

        Ok(Some(alert))
    }

    fn send_alert_to_channels(&self, alert: &Alert) -> Result<(), String> {
        for channel in &alert.channels {
            match channel {
                AlertChannel::Email => println!("📧 Email Dispatch: {:?}", self.config.email_recipients),
                AlertChannel::Slack => println!("💬 Slack Push: {:?}", self.config.slack_webhook),
                AlertChannel::Log => println!("📝 System Audit Log: {}", alert.title),
                AlertChannel::SMS => println!("📱 SMS Alert: {:?}", self.config.sms_recipients),
            }
        }
        Ok(())
    }

    // --- TETİKLEYİCİLER ---

    pub fn trigger_max_dd_alert(&mut self, current_dd: f64) -> Result<Option<Alert>, String> {
        if !self.is_active { return Err("Sistem inaktif".to_owned()); }
        if current_dd > self.config.max_dd_threshold {
            self.create_alert(
                AlertLevel::Critical,
                "🚨 Max Drawdown İhlali".to_owned(),
                format!("DD Eşiği Aşıldı: {:.2}% (Limit: {:.2}%)", current_dd, self.config.max_dd_threshold),
                vec![AlertChannel::Email, AlertChannel::Slack],
            )
        } else { Ok(None) }
    }

    pub fn trigger_profit_milestone_alert(&mut self, profit: f64, milestone: Option<f64>) -> Result<Option<Alert>, String> {
        if !self.is_active { return Err("Sistem inaktif".to_owned()); }
        let threshold = milestone.unwrap_or(self.config.profit_milestone);
        if profit >= threshold {
            self.create_alert(
                AlertLevel::Info,
                "💰 Hedef Kâr Milestone".to_owned(),
                format!("Milestone Ulaşıldı: ${:.2}", threshold),
                vec![AlertChannel::Slack],
            )
        } else { Ok(None) }
    }

    pub fn trigger_system_error_alert(&mut self, err: &str) -> Result<Option<Alert>, String> {
        if !self.is_active { return Err("Sistem inaktif".to_owned()); }
        self.create_alert(
            AlertLevel::Error,
            "⚠️ Kritik Sistem Hatası".to_owned(),
            err.to_owned(),
            vec![AlertChannel::Log, AlertChannel::Email],
        )
    }

    pub fn create_custom_alert(&mut self, level: AlertLevel, title: String, msg: String, channels: Vec<AlertChannel>) -> Result<Option<Alert>, String> {
        if !self.is_active { return Err("Sistem inaktif".to_owned()); }
        self.create_alert(level, title, msg, channels)
    }

    // --- İSTATİSTİKLER ---

    /// Metrikleri tek geçişte (O(n)) hesaplar.
    pub fn get_stats(&self) -> (usize, usize, usize, usize) {
        let (mut i, mut w, mut e, mut c) = (0, 0, 0, 0);
        for alert in &self.alerts_history {
            match alert.level {
                AlertLevel::Info => i += 1,
                AlertLevel::Warning => w += 1,
                AlertLevel::Error => e += 1,
                AlertLevel::Critical => c += 1,
            }
        }
        (i, w, e, c)
    }
}

// --- TRAIT ENTEGRASYONLARI ---

impl HealthCheck for AlertSystem {
    fn check_health(&self) -> HealthStatus {
        let (_, _, _, critical) = self.get_stats();
        if critical > 0 {
            HealthStatus::Critical(format!("Son 1000 olayda {} kritik uyarı mevcut!", critical))
        } else {
            HealthStatus::Healthy
        }
    }
}

impl AnomalyDetector for AlertSystem {
    fn detect_anomaly(&self) -> Option<AnomalyType> {
        let (_, _, error, critical) = self.get_stats();
        if critical + error > 50 {
            return Some(AnomalyType::Custom("Aşırı hata yoğunluğu (Log Flooding)".to_owned()));
        }
        None
    }
}
