// audit_trail.rs
// Kullanıcı ve işlem bazlı detaylı audit trail modülü

use chrono::Utc;
use serde::{Serialize, Deserialize};
use std::sync::{Mutex, OnceLock};
use serde_json::Value;

// Modern Rust: Lazy yerine OnceLock kullanımı standarttır.
static AUDIT_LOG: OnceLock<Mutex<Vec<AuditEntry>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub timestamp: i64,
    pub username: String,
    pub action: String,
    pub resource: String,
    pub ip: String,
    pub result: String,
    pub details: Value,
}

pub struct AuditTrail;

impl AuditTrail {
    /// Global log listesine güvenli erişim sağlayan yardımcı metot
    fn get_log() -> &'static Mutex<Vec<AuditEntry>> {
        AUDIT_LOG.get_or_init(|| Mutex::new(Vec::with_capacity(1000)))
    }

    /// Audit log kaydı ekle - Performans: Gereksiz kopyalamadan kaçınır.
    pub fn log(username: &str, action: &str, resource: &str, ip: &str, result: &str, details: &Value) {
        let entry = AuditEntry {
            timestamp: Utc::now().timestamp(),
            username: username.to_owned(), // to_string() yerine to_owned() daha nettir
            action: action.to_owned(),
            resource: resource.to_owned(),
            ip: ip.to_owned(),
            result: result.to_owned(),
            details: details.clone(),
        };

        if let Ok(mut log) = Self::get_log().lock() {
            log.push(entry);
        }
    }

    /// Audit loglarını filtrele - Performans: Referans döndürerek bellek kopyalamasını (cloning) önler.
    /// Not: Lock tutulduğu sürece veri okunabilir, bu yüzden sonuçları kopyalayıp dönmek (vec collect) güvenlidir.
    pub fn search(user: Option<&str>, action: Option<&str>, result: Option<&str>) -> Vec<AuditEntry> {
        let log = Self::get_log().lock().unwrap();
        
        log.iter()
            .filter(|e| {
                user.map_or(true, |u| e.username == u) &&
                action.map_or(true, |a| e.action == a) &&
                result.map_or(true, |r| e.result == r)
            })
            .cloned() // Sadece filtrelenmiş küçük küme kopyalanır
            .collect()
    }

    /// Tüm audit loglarını getir
    pub fn all() -> Vec<AuditEntry> {
        Self::get_log().lock().unwrap().clone()
    }
}
