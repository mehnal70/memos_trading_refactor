// security_manager.rs
// Güvenlik ve Erişim Kontrol Modülü

use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct ApiKey {
    pub key_id: String,
    pub created_at: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
    // Performans: İzin kontrolü için Vec yerine HashSet (O(1))
    pub permissions: HashSet<String>,
    pub active: bool,
}

#[derive(Debug, Clone)]
pub struct AccessLog {
    pub key_id: String,
    pub action: String,
    pub timestamp: DateTime<Utc>,
}

pub trait SecurityManager {
    fn add_api_key(&mut self, key: ApiKey);
    fn revoke_api_key(&mut self, key_id: &str) -> bool;
    fn log_access(&mut self, key_id: &str, action: &str);
    fn is_authorized(&self, key_id: &str, permission: &str) -> bool;
}

pub struct SimpleSecurityManager {
    // ID bazlı anında erişim için HashMap
    pub keys: HashMap<String, ApiKey>,
    // Loglar kronolojik olduğu için Vec kalabilir
    pub access_logs: Vec<AccessLog>,
}

impl SimpleSecurityManager {
    pub fn new() -> Self {
        Self {
            keys: HashMap::with_capacity(50),
            access_logs: Vec::with_capacity(500),
        }
    }
}

impl SecurityManager for SimpleSecurityManager {
    fn add_api_key(&mut self, key: ApiKey) {
        self.keys.insert(key.key_id.clone(), key);
    }

    fn revoke_api_key(&mut self, key_id: &str) -> bool {
        if let Some(key) = self.keys.get_mut(key_id) {
            key.active = false;
            return true;
        }
        false
    }

    fn log_access(&mut self, key_id: &str, action: &str) {
        let log = AccessLog {
            key_id: key_id.to_owned(),
            action: action.to_owned(),
            timestamp: Utc::now(),
        };
        
        // Anahtarın son kullanım zamanını da güncelle (Audit için kritik)
        if let Some(key) = self.keys.get_mut(key_id) {
            key.last_used = Some(log.timestamp);
        }

        self.access_logs.push(log);
    }

    fn is_authorized(&self, key_id: &str, permission: &str) -> bool {
        self.keys.get(key_id).map_or(false, |k| {
            k.active && k.permissions.contains(permission)
        })
    }
}
