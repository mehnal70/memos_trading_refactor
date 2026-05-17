// gdpr.rs - GDPR Uyumluluk ve Veri Gizliliği Modülü

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, RwLock};
use chrono::Utc;
use serde::{Serialize, Deserialize};

// Modern Rust: Lazy yerine OnceLock, Mutex yerine okuma performansı için RwLock.
static ACCESS_LOG: OnceLock<Mutex<Vec<AccessLogEntry>>> = OnceLock::new();
static USER_DATA: OnceLock<RwLock<HashMap<String, UserData>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserData {
    pub username: String,
    pub email: String,
    pub phone: String,
    pub address: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessLogEntry {
    pub username: String,
    pub field: String,
    pub timestamp: i64,
    pub action: String, // "read", "mask", "delete"
}

pub struct GdprManager;

impl GdprManager {
    // Statik yapılara güvenli erişim sağlayan yardımcı metodlar
    fn users() -> &'static RwLock<HashMap<String, UserData>> {
        USER_DATA.get_or_init(|| RwLock::new(HashMap::with_capacity(100)))
    }

    fn logs() -> &'static Mutex<Vec<AccessLogEntry>> {
        ACCESS_LOG.get_or_init(|| Mutex::new(Vec::with_capacity(500)))
    }

    /// Kullanıcı verisini maskele (Bellek dostu ve hızlı string manipülasyonu)
    pub fn mask_data(data: &UserData) -> UserData {
        UserData {
            username: data.username.to_owned(),
            email: mask_email(&data.email),
            phone: mask_phone(&data.phone),
            address: "***MASKED***".to_owned(),
            created_at: data.created_at,
        }
    }

    /// Kullanıcı verisini sil (Right to be forgotten)
    pub fn delete_user(username: &str) {
        if let Ok(mut users) = Self::users().write() {
            if users.remove(username).is_some() {
                Self::log_access(username, "ALL", "delete");
            }
        }
    }

    /// Kullanıcı verisine erişim logla
    pub fn log_access(username: &str, field: &str, action: &str) {
        if let Ok(mut logs) = Self::logs().lock() {
            logs.push(AccessLogEntry {
                username: username.to_owned(),
                field: field.to_owned(),
                timestamp: Utc::now().timestamp(),
                action: action.to_owned(),
            });
        }
    }

    /// Kullanıcı verisini getir (RwLock sayesinde çoklu okuma desteği)
    pub fn get_user(username: &str, masked: bool) -> Option<UserData> {
        let users = Self::users().read().ok()?;
        let data = users.get(username)?;
        
        Self::log_access(username, "ALL", if masked { "mask" } else { "read" });

        Some(if masked {
            Self::mask_data(data)
        } else {
            data.clone()
        })
    }

    /// Kullanıcı verisi ekle/güncelle
    pub fn upsert_user(data: UserData) {
        if let Ok(mut users) = Self::users().write() {
            users.insert(data.username.clone(), data);
        }
    }

    /// Erişim loglarını getir (Referansları kopyalayarak güvenli dönüş)
    pub fn get_access_logs() -> Vec<AccessLogEntry> {
        Self::logs().lock().map(|l| l.clone()).unwrap_or_default()
    }
}

// --- YARDIMCI MASKELENME FONKSİYONLARI ---

fn mask_email(email: &str) -> String {
    let Some((user, domain)) = email.split_once('@') else {
        return "***MASKED***".to_owned();
    };
    
    let user_masked = if !user.is_empty() { 
        format!("{}***", &user[..1]) 
    } else { 
        "*".to_owned() 
    };
    
    format!("{}@{}", user_masked, domain)
}

fn mask_phone(phone: &str) -> String {
    if phone.len() < 4 { 
        return "***MASKED***".to_owned(); 
    }
    format!("***{}", &phone[phone.len() - 2..])
}
