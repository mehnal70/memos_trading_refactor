// gdpr.rs
// GDPR ve veri gizliliği uyumluluğu için üretim seviyesinde modül
// Maskeleme, silme (right to be forgotten), erişim loglama, veri erişim kontrolü
// Türkçe açıklamalar ile

use std::collections::HashMap;
use std::sync::Mutex;
use chrono::Utc;
use once_cell::sync::Lazy;
use serde::{Serialize, Deserialize};

static ACCESS_LOG: Lazy<Mutex<Vec<AccessLogEntry>>> = Lazy::new(|| Mutex::new(Vec::new()));
static USER_DATA: Lazy<Mutex<HashMap<String, UserData>>> = Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Serialize, Deserialize)]
pub struct UserData {
    pub username: String,
    pub email: String,
    pub phone: String,
    pub address: String,
    pub created_at: i64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AccessLogEntry {
    pub username: String,
    pub field: String,
    pub timestamp: i64,
    pub action: String, // "read", "mask", "delete"
}

pub struct GdprManager;

impl GdprManager {
    // Kullanıcı verisini maskele (örn: email -> e***@d***.com)
    pub fn mask_data(data: &UserData) -> UserData {
        UserData {
            username: data.username.clone(),
            email: mask_email(&data.email),
            phone: mask_phone(&data.phone),
            address: "***MASKED***".to_string(),
            created_at: data.created_at,
        }
    }

    // Kullanıcı verisini sil (right to be forgotten)
    pub fn delete_user(username: &str) {
        USER_DATA.lock().unwrap().remove(username);
        ACCESS_LOG.lock().unwrap().push(AccessLogEntry {
            username: username.to_string(),
            field: "ALL".to_string(),
            timestamp: Utc::now().timestamp(),
            action: "delete".to_string(),
        });
    }

    // Kullanıcı verisine erişim logla
    pub fn log_access(username: &str, field: &str, action: &str) {
        ACCESS_LOG.lock().unwrap().push(AccessLogEntry {
            username: username.to_string(),
            field: field.to_string(),
            timestamp: Utc::now().timestamp(),
            action: action.to_string(),
        });
    }

    // Kullanıcı verisini getir (maskeleme opsiyonlu)
    pub fn get_user(username: &str, masked: bool) -> Option<UserData> {
        let data = USER_DATA.lock().unwrap().get(username).cloned();
        if data.is_some() {
            GdprManager::log_access(username, "ALL", if masked { "mask" } else { "read" });
        }
        if masked {
            data.map(|d| GdprManager::mask_data(&d))
        } else {
            data
        }
    }

    // Kullanıcı verisi ekle/güncelle
    pub fn upsert_user(data: UserData) {
        USER_DATA.lock().unwrap().insert(data.username.clone(), data);
    }

    // Erişim loglarını getir
    pub fn get_access_logs() -> Vec<AccessLogEntry> {
        ACCESS_LOG.lock().unwrap().clone()
    }
}

fn mask_email(email: &str) -> String {
    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() != 2 { return "***MASKED***".to_string(); }
    let (user, domain) = (parts[0], parts[1]);
    let user_masked = if user.len() > 1 { format!("{}***", &user[0..1]) } else { "*".to_string() };
    let domain_masked = if domain.len() > 1 { format!("{}***", &domain[0..1]) } else { "*".to_string() };
    format!("{}@{}.com", user_masked, domain_masked)
}

fn mask_phone(phone: &str) -> String {
    if phone.len() < 4 { return "***MASKED***".to_string(); }
    let len = phone.len();
    format!("***{}", &phone[len-2..])
}

// Not: Gerçek ortamda veriler DB'de şifreli tutulmalı, loglar immutable olmalı, erişim kontrolleri zorunlu.
