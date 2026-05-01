// security_manager.rs
// Güvenlik ve Erişim Kontrol Modülü
// API anahtar yönetimi, erişim loglama, yetkilendirme, şifreleme

use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct ApiKey {
    pub key_id: String,
    pub created_at: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
    pub permissions: Vec<String>,
    pub active: bool,
}

pub trait SecurityManager {
    fn add_api_key(&mut self, key: ApiKey);
    fn revoke_api_key(&mut self, key_id: &str) -> bool;
    fn log_access(&mut self, key_id: &str, action: &str);
    fn is_authorized(&self, key_id: &str, permission: &str) -> bool;
}

pub struct SimpleSecurityManager {
    pub keys: Vec<ApiKey>,
    pub access_logs: Vec<AccessLog>,
}

#[derive(Debug, Clone)]
pub struct AccessLog {
    pub key_id: String,
    pub action: String,
    pub timestamp: DateTime<Utc>,
}

impl SecurityManager for SimpleSecurityManager {
    fn add_api_key(&mut self, key: ApiKey) {
        self.keys.push(key);
    }
    fn revoke_api_key(&mut self, key_id: &str) -> bool {
        if let Some(key) = self.keys.iter_mut().find(|k| k.key_id == key_id) {
            key.active = false;
            true
        } else {
            false
        }
    }
    fn log_access(&mut self, key_id: &str, action: &str) {
        let log = AccessLog {
            key_id: key_id.to_string(),
            action: action.to_string(),
            timestamp: Utc::now(),
        };
        self.access_logs.push(log);
    }
    fn is_authorized(&self, key_id: &str, permission: &str) -> bool {
        self.keys.iter().any(|k| k.key_id == key_id && k.active && k.permissions.contains(&permission.to_string()))
    }
}
