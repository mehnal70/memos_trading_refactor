// src/robot/security/types.rs - Güvenlik Kontrat Modelleri
// Srivastava ATP - Saf Veri Yapıları Katmanı

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};

/// Rol Tanımları (Role-Based Access Control)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserRole {
    Admin,           // Tam kontrol
    Trader,          // İşlem yetkilisi
    Monitor,         // Sadece görüntüleme
    Auditor,         // Denetim ve raporlama
}

impl UserRole {
    pub fn can_trade(&self) -> bool {
        matches!(self, Self::Admin | Self::Trader)
    }
    
    pub fn can_view_audit(&self) -> bool {
        matches!(self, Self::Admin | Self::Auditor | Self::Monitor)
    }
    
    pub fn can_modify_settings(&self) -> bool {
        matches!(self, Self::Admin)
    }
    
    pub fn can_emergency_stop(&self) -> bool {
        matches!(self, Self::Admin | Self::Trader)
    }
}

/// Kullanıcı Bilgileri
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub role: UserRole,
    pub api_key_hash: String, // Asla plain text tutulmaz
    pub created_at: DateTime<Utc>,
    pub last_login: Option<DateTime<Utc>>,
    pub is_active: bool,
}

/// Denetim Olayı (Audit Event)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_type: String,
    pub user_id: String,
    pub action: String,
    pub result: String,
    pub details: Option<String>,
    pub ip_address: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub trade_id: Option<String>,
    pub symbol: Option<String>,
}

impl AuditEvent {
    pub fn new(
        event_type: impl Into<String>,
        user_id: impl Into<String>,
        action: impl Into<String>,
        result: impl Into<String>,
    ) -> Self {
        Self {
            event_type: event_type.into(),
            user_id: user_id.into(),
            action: action.into(),
            result: result.into(),
            details: None,
            ip_address: None,
            timestamp: Utc::now(),
            trade_id: None,
            symbol: None,
        }
    }
}
