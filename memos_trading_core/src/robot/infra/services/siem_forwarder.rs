// siem_forwarder.rs
// SIEM Entegrasyonu: Güvenlik ve Anomali Loglarını İleten Modül

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock, RwLock};
use reqwest::Client;
use serde_json::{json, Value};
use chrono::Utc;
use tokio::net::UdpSocket;

// Modern Rust: Lazy yerine yerleşik OnceLock, yazma nadir olduğu için RwLock.
static SIEM_SETTINGS: OnceLock<RwLock<SiemConfig>> = OnceLock::new();

#[derive(Clone, Debug, Default)]
pub struct SiemConfig {
    pub syslog_addr: Option<SocketAddr>, // String yerine tür güvenli SocketAddr
    pub http_url: Option<String>,
    pub http_token: Option<String>,
}

pub struct SiemForwarder;

impl SiemForwarder {
    /// Global konfigürasyona güvenli erişim sağlayan yardımcı
    fn get_config() -> &'static RwLock<SiemConfig> {
        SIEM_SETTINGS.get_or_init(|| RwLock::new(SiemConfig::default()))
    }

    /// SIEM ayarlarını günceller
    pub fn set_config(cfg: SiemConfig) {
        if let Ok(mut guard) = Self::get_config().write() {
            *guard = cfg;
        }
    }

    /// Log'u SIEM'e asenkron olarak iletir (Non-blocking)
    pub async fn forward_log(event_type: &str, details: &Value) {
        // Okuma kilidini al (Shared lock)
        let cfg = match Self::get_config().read() {
            Ok(c) => c.clone(),
            Err(_) => return,
        };

        // Log verisini tek seferde (Zero-copy dostu) oluştur
        let log_payload = json!({
            "ts": Utc::now().to_rfc3339(),
            "type": event_type,
            "details": details
        });
        
        let log_string = log_payload.to_string();

        // 1. Syslog (UDP) - Asenkron gönderim
        if let Some(addr) = cfg.syslog_addr {
            tokio::spawn(async move {
                // Her gönderimde bind yerine socket pool kullanılabilir, 
                // ancak anomali logları düşük frekanslıdır.
                if let Ok(socket) = UdpSocket::bind("0.0.0.0:0").await {
                    let _ = socket.send_to(log_string.as_bytes(), addr).await;
                }
            });
        }

        // 2. HTTP Push - Asenkron gönderim
        if let Some(url) = cfg.http_url {
            let token = cfg.http_token.clone();
            let log_json = log_payload.clone();
            
            tokio::spawn(async move {
                let client = Client::new();
                let mut req = client.post(url).json(&log_json);
                
                if let Some(t) = token {
                    req = req.header("Authorization", format!("Bearer {}", t));
                }
                
                let _ = req.send().await;
            });
        }
    }
}
