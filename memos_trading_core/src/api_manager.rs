// api_manager.rs
// API Entegrasyonu ve Dış Servis Yönetimi Modülü
// API bağlantı yönetimi, hata izleme, rate limit, servis durumu

use chrono::{DateTime, Utc};

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
    fn all_connections(&self) -> &Vec<ApiConnection>;
}

pub struct SimpleApiManager {
    pub connections: Vec<ApiConnection>,
}

impl ApiManager for SimpleApiManager {
    fn register_connection(&mut self, conn: ApiConnection) {
        self.connections.push(conn);
    }
    fn update_status(&mut self, name: &str, status: ApiStatus, rate_limit: Option<u32>) {
        if let Some(conn) = self.connections.iter_mut().find(|c| c.name == name) {
            conn.status = status;
            conn.last_checked = Utc::now();
            conn.rate_limit_remaining = rate_limit;
        }
    }
    fn get_status(&self, name: &str) -> Option<&ApiConnection> {
        self.connections.iter().find(|c| c.name == name)
    }
    fn all_connections(&self) -> &Vec<ApiConnection> {
        &self.connections
    }
}
