// notification_logger.rs
// Bildirim ve Loglama Modülü

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone)]
pub struct NotificationLog {
    pub log_id: String,
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub message: String,
    pub notified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
    Critical,
}

// Logların terminale veya dosyalara güzel basılması için Display trait'i
impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let emoji = match self {
            Self::Info => "ℹ️",
            Self::Warning => "⚠️",
            Self::Error => "❌",
            Self::Critical => "🚨",
        };
        write!(f, "{} {:?}", emoji, self)
    }
}

pub trait NotificationLogger {
    fn log_event(&mut self, log: NotificationLog);
    fn notify(&mut self, log_id: &str);
    fn get_log(&self, log_id: &str) -> Option<&NotificationLog>;
    fn all_logs(&self) -> Vec<&NotificationLog>;
}

pub struct SimpleNotificationLogger {
    // ID bazlı O(1) erişim için HashMap
    pub logs: HashMap<String, NotificationLog>,
    // Kritik logları hızlı filtrelemek için bir sayaç veya index eklenebilir
}

impl SimpleNotificationLogger {
    pub fn new() -> Self {
        Self {
            logs: HashMap::with_capacity(500),
        }
    }

    /// Belirli bir seviyenin üzerindeki tüm bildirilmemiş (notified=false) logları getirir
    pub fn get_pending_alerts(&self, min_level: LogLevel) -> Vec<&NotificationLog> {
        self.logs
            .values()
            .filter(|l| l.level >= min_level && !l.notified)
            .collect()
    }
}

impl NotificationLogger for SimpleNotificationLogger {
    fn log_event(&mut self, log: NotificationLog) {
        // Log seviyesine göre terminale anında çıktı verilebilir
        if log.level >= LogLevel::Error {
            eprintln!("[{}] {} | {}", log.timestamp.format("%H:%M:%S"), log.level, log.message);
        }
        
        self.logs.insert(log.log_id.clone(), log);
    }

    fn notify(&mut self, log_id: &str) {
        if let Some(l) = self.logs.get_mut(log_id) {
            l.notified = true;
            // Buraya gerçek zamanlı Telegram/Email entegrasyonu tetiklenebilir
        }
    }

    #[inline]
    fn get_log(&self, log_id: &str) -> Option<&NotificationLog> {
        self.logs.get(log_id)
    }

    fn all_logs(&self) -> Vec<&NotificationLog> {
        // Pipeline standartlarına uygun referans listesi dönüşü
        self.logs.values().collect()
    }
}
