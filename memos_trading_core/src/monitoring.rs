// monitoring.rs - Otomatik İzleme, Uyarı ve Kurtarma Sistemi
// Self-healing, watchdog, crash recovery ve gerçek zamanlı uyarı altyapısı
// Türkçe açıklamalar ile

use chrono::Utc;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

#[derive(Debug, Clone)]
pub enum AlertLevel {
    Info,
    Warning,
    Error,
    Critical,
}

pub struct Monitor {
    pub last_heartbeat: Arc<Mutex<chrono::DateTime<chrono::Utc>>>,
}

impl Monitor {
    pub fn new() -> Self {
        Self {
            last_heartbeat: Arc::new(Mutex::new(Utc::now())),
        }
    }

    /// Watchdog: sistemin canlılığını izler, belirli süre heartbeat alınmazsa uyarı üretir
    pub async fn watchdog(&self, timeout_secs: u64) {
        loop {
            sleep(Duration::from_secs(timeout_secs)).await;
            let last = *self.last_heartbeat.lock().await;
            let now = Utc::now();
            if (now - last).num_seconds() > timeout_secs as i64 {
                Self::send_alert(AlertLevel::Critical, "Watchdog: Sistem yanıt vermiyor!");
                // Burada otomatik kurtarma (restart, failover) tetiklenebilir
            }
        }
    }

    /// Heartbeat: sistemin canlı olduğunu bildirir
    pub async fn heartbeat(&self) {
        let mut last = self.last_heartbeat.lock().await;
        *last = Utc::now();
    }

    /// Uyarı/olay bildirimi (log dosyasına ve konsola)
    pub fn send_alert(level: AlertLevel, msg: &str) {
        let now = Utc::now().to_rfc3339();
        let mut file = OpenOptions::new().create(true).append(true).open("logs/alerts.log").unwrap();
        writeln!(file, "[{}][{:?}] {}", now, level, msg).ok();
        println!("[ALERT][{:?}] {}", level, msg);
        // Burada e-posta, Telegram, webhook entegrasyonu eklenebilir
    }
}

// Kullanım örneği (main fonksiyonunda async olarak):
// let monitor = Monitor::new();
// tokio::spawn(monitor.watchdog(60));
// loop { monitor.heartbeat().await; ... }
