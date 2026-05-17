// api_manager.rs
// API Entegrasyonu ve Dış Servis Yönetimi Modülü
// API bağlantı yönetimi, hata izleme, rate limit, servis durumu

use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ApiConnection {
    pub name: String,
    pub last_checked: DateTime<Utc>,
    pub status: ApiStatus,
    pub rate_limit_remaining: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApiStatus {
    Connected,
    Disconnected,
    Error(String),
    RateLimited,
}

pub trait ApiManager {
    fn register_connection(&mut self, conn: ApiConnection);
    fn update_status(&mut self, name: &str, status: ApiStatus, rate_limit: Option<u32>);
    fn get_status(&self, name: &str) -> Option<&ApiConnection>;
    fn all_connections(&self) -> Vec<&ApiConnection>;
}

pub struct SimpleApiManager {
    pub connections: HashMap<String, ApiConnection>,
}

impl ApiStatus {
    // Sadece bağlantı var mı?
    pub fn is_connected(&self) -> bool {
        matches!(self, ApiStatus::Connected)
    }

    // Herhangi bir hata durumu var mı?
    pub fn is_error(&self) -> bool {
        matches!(self, ApiStatus::Error(_) | ApiStatus::Disconnected)
    }

    // Rate limit'e takıldık mı?
    pub fn is_limited(&self) -> bool {
        matches!(self, ApiStatus::RateLimited)
    }

    pub fn message(&self) -> &str {
        match self {
            ApiStatus::Error(msg) => msg,
            ApiStatus::Connected => "Bağlantı sağlıklı",
            ApiStatus::Disconnected => "Bağlantı kesildi",
            ApiStatus::RateLimited => "İstek sınırı aşıldı",
        }
    }
}

impl ApiManager for SimpleApiManager {
    fn register_connection(&mut self, conn: ApiConnection) {
        self.connections.insert(conn.name.clone(), conn);
    }
    fn update_status(&mut self, name: &str, status: ApiStatus, rate_limit: Option<u32>) {
        if let Some(conn) = self.connections.get_mut(name) {
            conn.status = status;
            conn.last_checked = Utc::now();
            conn.rate_limit_remaining = rate_limit;
        }
    }
    fn get_status(&self, name: &str) -> Option<&ApiConnection> {
        self.connections.get(name)
    }
    fn all_connections(&self) -> Vec<&ApiConnection> {
        self.connections.values().collect()
    }
}
