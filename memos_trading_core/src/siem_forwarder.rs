// siem_forwarder.rs
// SIEM entegrasyonu: Güvenlik loglarını ve anomali olaylarını kurumsal SIEM sistemine ileten modül
// Splunk, ELK, Wazuh, Graylog vb. ile uyumlu, hem syslog (UDP/TCP) hem HTTP/JSON push desteği
// Türkçe açıklamalar ile

use std::net::UdpSocket;
use std::sync::Mutex;
use reqwest::blocking::Client;
use serde_json::Value;
use chrono::Utc;
use once_cell::sync::Lazy;

static SIEM_CONFIG: Lazy<Mutex<SiemConfig>> = Lazy::new(|| Mutex::new(SiemConfig::default()));

#[derive(Clone, Debug)]
pub struct SiemConfig {
    pub syslog_addr: Option<String>, // "127.0.0.1:514"
    pub http_url: Option<String>,    // "https://siem.example.com/api/logs"
    pub http_token: Option<String>,  // API token
}

impl Default for SiemConfig {
    fn default() -> Self {
        Self {
            syslog_addr: None,
            http_url: None,
            http_token: None,
        }
    }
}

pub struct SiemForwarder;

impl SiemForwarder {
    // SIEM config ayarla
    pub fn set_config(cfg: SiemConfig) {
        *SIEM_CONFIG.lock().unwrap() = cfg;
    }

    // Log'u SIEM'e ilet (hem syslog hem HTTP)
    pub fn forward_log(event_type: &str, details: &Value) {
        let cfg = SIEM_CONFIG.lock().unwrap().clone();
        let timestamp = Utc::now().to_rfc3339();
        let logline = format!(
            "{{\"ts\":\"{}\",\"type\":\"{}\",\"details\":{}}}",
            timestamp, event_type, details
        );
        // Syslog (UDP)
        if let Some(addr) = cfg.syslog_addr {
            let _ = UdpSocket::bind("0.0.0.0:0").and_then(|sock| sock.send_to(logline.as_bytes(), addr));
        }
        // HTTP/JSON
        if let Some(url) = cfg.http_url {
            let client = Client::new();
            let mut req = client.post(url).body(logline.clone()).header("Content-Type", "application/json");
            if let Some(token) = cfg.http_token {
                req = req.header("Authorization", format!("Bearer {}", token));
            }
            let _ = req.send();
        }
    }
}

// Kullanım örneği:
// SiemForwarder::set_config(SiemConfig { syslog_addr: Some("127.0.0.1:514".into()), http_url: None, http_token: None });
// SiemForwarder::forward_log("anomaly", &serde_json::json!({"user": "alice", "desc": "Şüpheli giriş"}));

// Not: Gerçek ortamda log formatı, güvenlik ve hata yönetimi kurumsal gereksinimlere göre özelleştirilmeli.
