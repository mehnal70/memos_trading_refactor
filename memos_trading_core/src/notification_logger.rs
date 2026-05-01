// notification_logger.rs
// Bildirim ve Loglama Modülü
// Olay bildirimi, sistem logları, hata ve uyarı kaydı

use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct NotificationLog {
    pub log_id: String,
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub message: String,
    pub notified: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
    Critical,
}

pub trait NotificationLogger {
    fn log_event(&mut self, log: NotificationLog);
    fn notify(&mut self, log_id: &str);
    fn get_log(&self, log_id: &str) -> Option<&NotificationLog>;
    fn all_logs(&self) -> &Vec<NotificationLog>;
}

pub struct SimpleNotificationLogger {
    pub logs: Vec<NotificationLog>,
}

impl NotificationLogger for SimpleNotificationLogger {
    fn log_event(&mut self, log: NotificationLog) {
        self.logs.push(log);
    }
    fn notify(&mut self, log_id: &str) {
        if let Some(l) = self.logs.iter_mut().find(|l| l.log_id == log_id) {
            l.notified = true;
        }
    }
    fn get_log(&self, log_id: &str) -> Option<&NotificationLog> {
        self.logs.iter().find(|l| l.log_id == log_id)
    }
    fn all_logs(&self) -> &Vec<NotificationLog> {
        &self.logs
    }
}
