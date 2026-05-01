// audit_trail.rs
// Kullanıcı ve işlem bazlı detaylı audit trail modülü (immutable, arama yapılabilir)
// Türkçe açıklamalar ile

use chrono::Utc;
use serde::{Serialize, Deserialize};
use std::sync::Mutex;
use once_cell::sync::Lazy;
use serde_json::Value;

static AUDIT_LOG: Lazy<Mutex<Vec<AuditEntry>>> = Lazy::new(|| Mutex::new(Vec::new()));

#[derive(Clone, Serialize, Deserialize)]
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
    // Audit log kaydı ekle
    pub fn log(username: &str, action: &str, resource: &str, ip: &str, result: &str, details: &Value) {
        let entry = AuditEntry {
            timestamp: Utc::now().timestamp(),
            username: username.to_string(),
            action: action.to_string(),
            resource: resource.to_string(),
            ip: ip.to_string(),
            result: result.to_string(),
            details: details.clone(),
        };
        AUDIT_LOG.lock().unwrap().push(entry);
    }

    // Audit loglarını filtrele (kullanıcı, işlem, tarih, sonuç)
    pub fn search(user: Option<&str>, action: Option<&str>, result: Option<&str>) -> Vec<AuditEntry> {
        AUDIT_LOG.lock().unwrap().iter().cloned().filter(|e|
            user.map_or(true, |u| e.username == u) &&
            action.map_or(true, |a| e.action == a) &&
            result.map_or(true, |r| e.result == r)
        ).collect()
    }

    // Tüm audit loglarını getir
    pub fn all() -> Vec<AuditEntry> {
        AUDIT_LOG.lock().unwrap().clone()
    }
}

// Not: Gerçek ortamda loglar immutable storage'a (ör. append-only dosya, S3, DB) yazılmalı ve SIEM forwarder ile merkezi olarak iletilmeli.
