// Güvenlik Modülü - API anahtar yalıtımı, denetim izleri, oran limitlemesi, rol tabanlı erişim
// Security Module: API key isolation, audit trails, rate limiting, RBAC

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use crate::MemosTradingError;
use crate::robot::error::ErrorLogger;

type Result<T> = std::result::Result<T, MemosTradingError>;

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
    pub api_key_hash: String, // Asla plain text
    pub created_at: DateTime<Utc>,
    pub last_login: Option<DateTime<Utc>>,
    pub is_active: bool,
}

/// Denetim Olayı (Audit Event)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Olayın türü: "trade", "login", "config_change", "api_key_rotation", "emergency_stop", etc.
    pub event_type: String,
    
    /// Kim yaptı?
    pub user_id: String,
    
    /// Ne yapıldı? (JSON formatında detay)
    pub action: String,
    
    /// Sonuç: "success" | "failure"
    pub result: String,
    
    /// Detay (hata mesajı varsa)
    pub details: Option<String>,
    
    /// IP adresi (eğer biliniyorsa)
    pub ip_address: Option<String>,
    
    /// Zamanı
    pub timestamp: DateTime<Utc>,
    
    /// Trade referansı (varsa)
    pub trade_id: Option<String>,
    
    /// Pozisyon/sembol referansı
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

/// Rate Limit Kuralı
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitRule {
    /// Ne limitliyoruz? ("trades_per_minute", "api_calls_per_second", etc.)
    pub limit_type: String,
    
    /// Limit (saniye başına kaç?)
    pub max_per_second: u32,
    
    /// Bu kural kime uygulanır? (rol veya user_id)
    pub applies_to: String,
}

impl Default for RateLimitRule {
    fn default() -> Self {
        Self {
            limit_type: "trades_per_minute".to_string(),
            max_per_second: 10 / 60, // 10 per minute = ~0.17 per second
            applies_to: "all".to_string(),
        }
    }
}

/// Rate Limiter Tracker
#[derive(Debug)]
pub struct RateLimiterTracker {
    /// {key: (count, reset_time)}
    counters: HashMap<String, (u32, DateTime<Utc>)>,
}

impl RateLimiterTracker {
    pub fn new() -> Self {
        Self {
            counters: HashMap::new(),
        }
    }
    
    /// Check if action is allowed
    pub fn check_limit(&mut self, key: &str, max_per_second: u32) -> bool {
        let now = Utc::now();
        
        match self.counters.get_mut(key) {
            Some((count, reset_time)) => {
                if now >= *reset_time {
                    // Reset period
                    *count = 1;
                    *reset_time = now + chrono::Duration::seconds(1);
                    true
                } else if *count < max_per_second {
                    *count += 1;
                    true
                } else {
                    false
                }
            }
            None => {
                let reset_time = now + chrono::Duration::seconds(1);
                self.counters.insert(key.to_string(), (1, reset_time));
                true
            }
        }
    }
}

/// API Key Manager (çevresel değişkenlerden oku, asla hardcode etme)
pub struct ApiKeyManager {
    keys: HashMap<String, String>, // exchange -> masked_key for logging
}

impl ApiKeyManager {
    /// Çevresel değişkenlerden API key'leri yükle
    /// Convention: EXCHANGE_API_KEY ve EXCHANGE_API_SECRET
    pub fn from_env(exchanges: &[&str]) -> Result<Self> {
        let mut keys = HashMap::new();
        
        for exchange in exchanges {
            let key_var = format!("{}_API_KEY", exchange.to_uppercase());
            let secret_var = format!("{}_API_SECRET", exchange.to_uppercase());
            
            let api_key = std::env::var(&key_var)
                .map_err(|_| MemosTradingError::Unknown(
                    format!("Missing environment variable: {}", key_var)
                ))?;
            
            let api_secret = std::env::var(&secret_var)
                .map_err(|_| MemosTradingError::Unknown(
                    format!("Missing environment variable: {}", secret_var)
                ))?;
            
            if api_key.is_empty() || api_secret.is_empty() {
                return Err(MemosTradingError::Unknown(
                    format!("{}: API key ve secret boş olamaz", exchange)
                ));
            }
            
            // Log için mask et (sadece son 4 karakter)
            let masked = format!("{}...{}", &api_key[0..4.min(api_key.len())], 
                                &api_key[api_key.len().saturating_sub(4)..]);
            keys.insert(exchange.to_lowercase(), masked);
        }
        
        Ok(Self { keys })
    }
    
    /// Get masked key for logging (asla full key döndürme!)
    pub fn get_masked(&self, exchange: &str) -> Option<String> {
        self.keys.get(&exchange.to_lowercase()).cloned()
    }
    
    /// Validate that keys were loaded
    pub fn is_initialized(&self) -> bool {
        !self.keys.is_empty()
    }
}

/// Security Manager - Tüm güvenlik operasyonlarını yönet
/// Merkezi güvenlik yöneticisi - kullanıcı, denetim, oran limitlemesi
pub struct SecurityManager {
    /// Tüm kullanıcılar
    users: HashMap<String, User>,
    /// Denetim olayları (immutable log)
    audit_log: Vec<AuditEvent>,
    /// Oran limitlemesi tracker'ı
    rate_limiter: RateLimiterTracker,
    /// Oran limit kuralları
    rate_limits: HashMap<String, RateLimitRule>,
    /// API key manager (env-based)
    api_key_manager: Option<ApiKeyManager>,
    /// Logger reference (Tauri/file/stdout)
    logger: Option<std::sync::Arc<dyn ErrorLogger>>,
}

impl SecurityManager {
    /// Yeni güvenlik yöneticisi oluştur (logger'sız)
    pub fn new() -> Self {
        Self {
            users: HashMap::new(),
            audit_log: vec![],
            rate_limiter: RateLimiterTracker::new(),
            rate_limits: HashMap::new(),
            api_key_manager: None,
            logger: None,
        }
    }
    
    /// Logger'ı set et (optional)
    pub fn with_logger(&mut self, logger: std::sync::Arc<dyn ErrorLogger>) -> &mut Self {
        self.logger = Some(logger);
        self
    }
    
    /// İç log helper
    fn log(&self, msg: &str, is_error: bool) {
        if let Some(ref logger) = self.logger {
            if is_error {
                logger.log_error("security", msg);
            } else {
                logger.log_info("security", msg);
            }
        }
    }
    
    /// For testing: directly insert a user without auth checks
    #[cfg(test)]
    pub fn add_test_user(&mut self, user: User) {
        self.users.insert(user.id.clone(), user);
    }
    
    /// Çevresel değişkenlerden API key'lerini başlat
    pub fn init_api_keys(&mut self, exchanges: &[&str]) -> Result<()> {
        self.api_key_manager = Some(ApiKeyManager::from_env(exchanges)?);
        self.log("API key'ler başarıyla yüklendi", false);
        self.log_audit(AuditEvent::new(
            "system",
            "system",
            "api_keys_initialized",
            "success",
        ));
        Ok(())
    }
    
    // ============ USER MANAGEMENT ============
    
    /// Kullanıcı ekle (sadece Admin)
    pub fn add_user(&mut self, current_user_id: &str, new_user: User) -> Result<()> {
        let caller = self.users.get(current_user_id)
            .ok_or_else(|| MemosTradingError::Unknown("Caller not found".to_string()))?;
        
        if !caller.role.can_modify_settings() {
            let mut event = AuditEvent::new("user_creation", current_user_id, 
                format!("user_id: {}", new_user.id), "failure");
            event.details = Some("Unauthorized: only Admin can create users".to_string());
            self.log_audit(event);
            return Err(MemosTradingError::Unknown("Unauthorized".to_string()));
        }
        
        self.users.insert(new_user.id.clone(), new_user.clone());
        self.log_audit(AuditEvent::new(
            "user_creation",
            current_user_id,
            format!("user_id: {}, role: {:?}", new_user.id, new_user.role),
            "success",
        ));
        Ok(())
    }
    
    // ============ RATE LIMITING ============
    
    /// Set rate limit for a rule
    pub fn set_rate_limit(&mut self, rule: RateLimitRule) {
        let limit_type = rule.limit_type.clone();
        self.rate_limits.insert(limit_type, rule);
    }
    
    /// Check if action allowed by rate limit
    pub fn check_rate_limit(&mut self, user_id: &str, action_type: &str) -> bool {
        let rule = self.rate_limits
            .get(action_type)
            .cloned()
            .unwrap_or_default();
        
        let key = format!("{}:{}", user_id, action_type);
        self.rate_limiter.check_limit(&key, rule.max_per_second)
    }
    
    // ============ AUDIT TRAIL ============
    
    /// Denetim olayını kaydet
    pub fn log_audit(&mut self, mut event: AuditEvent) {
        event.timestamp = Utc::now();
        // Kısa bilgi log'u
        if event.result == "failure" {
            self.log(
                &format!("[{}] {}: {} - BAŞARISIZ", event.event_type, event.user_id, event.action),
                true
            );
        }
        self.audit_log.push(event);
    }
    
    /// Get audit logs filtered by criteria
    pub fn get_audit_logs(
        &self,
        event_type: Option<&str>,
        user_id: Option<&str>,
        limit: usize,
    ) -> Vec<AuditEvent> {
        self.audit_log
            .iter()
            .filter(|e| {
                (event_type.is_none() || Some(e.event_type.as_str()) == event_type)
                    && (user_id.is_none() || Some(e.user_id.as_str()) == user_id)
            })
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }
    
    /// Export audit log (for compliance/external auditors)
    pub fn export_audit_log(&self, query_type: &str) -> String {
        // CSV formatında export et
        let mut csv = String::from("timestamp,event_type,user_id,action,result,symbol,trade_id\n");
        
        let filtered: Vec<_> = match query_type {
            "trades" => self.audit_log.iter()
                .filter(|e| e.event_type == "trade" && e.result == "success")
                .collect(),
            "all" => self.audit_log.iter().collect(),
            _ => self.audit_log.iter().collect(),
        };
        
        for event in filtered {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{}\n",
                event.timestamp.format("%Y-%m-%d %H:%M:%S"),
                event.event_type,
                event.user_id,
                event.action,
                event.result,
                event.symbol.as_deref().unwrap_or(""),
                event.trade_id.as_deref().unwrap_or("")
            ));
        }
        
        csv
    }
    
    // ============ TRADE AUTHORIZATION ============
    
    /// Kullanıcı trade yapabilir mi?
    pub fn can_execute_trade(&mut self, user_id: &str, symbol: &str, size: f64) -> Result<()> {
        // 1. User exists and is active?
        let user = self.users.get(user_id)
            .ok_or_else(|| MemosTradingError::Unknown("User not found".to_string()))?;
        
        if !user.is_active {
            let mut event = AuditEvent::new("trade_authorization", user_id,
                format!("symbol: {}, size: {}", symbol, size), "failure");
            event.details = Some("User is inactive".to_string());
            event.symbol = Some(symbol.to_string());
            self.log_audit(event);
            return Err(MemosTradingError::Unknown("User is inactive".to_string()));
        }
        
        // 2. Has trade permission?
        if !user.role.can_trade() {
            let mut event = AuditEvent::new("trade_authorization", user_id,
                format!("symbol: {}, size: {}", symbol, size), "failure");
            event.details = Some(format!("Role {:?} cannot trade", user.role));
            event.symbol = Some(symbol.to_string());
            self.log_audit(event);
            return Err(MemosTradingError::Unknown("User role cannot trade".to_string()));
        }
        
        // 3. Rate limit check
        if !self.check_rate_limit(user_id, "trades_per_minute") {
            let mut event = AuditEvent::new("trade_authorization", user_id,
                format!("symbol: {}, size: {}", symbol, size), "failure");
            event.details = Some("Rate limit exceeded".to_string());
            event.symbol = Some(symbol.to_string());
            self.log_audit(event);
            return Err(MemosTradingError::Unknown("Rate limit exceeded".to_string()));
        }
        
        // 4. Log the successful authorization
        let mut event = AuditEvent::new("trade_authorization", user_id,
            format!("symbol: {}, size: {}", symbol, size), "success");
        event.symbol = Some(symbol.to_string());
        self.log_audit(event);
        
        Ok(())
    }
    
    /// Log trade execution
    pub fn log_trade(&mut self, user_id: &str, trade_id: &str, symbol: &str, 
                     side: &str, size: f64, price: f64) {
        let mut event = AuditEvent::new(
            "trade",
            user_id,
            format!("{} {} @ {}", side, size, price),
            "success",
        );
        event.trade_id = Some(trade_id.to_string());
        event.symbol = Some(symbol.to_string());
        self.log_audit(event);
    }
    
    // ============ EMERGENCY STOP ============
    
    /// Acil durdurma'yı dene
    pub fn emergency_stop(&mut self, user_id: &str) -> Result<()> {
        let user = self.users.get(user_id)
            .ok_or_else(|| MemosTradingError::Unknown("User not found".to_string()))?;
        
        if !user.role.can_emergency_stop() {
            let event = AuditEvent::new("emergency_stop", user_id, "triggered", "failure");
            self.log_audit(event);
            return Err(MemosTradingError::Unknown("User cannot trigger emergency stop".to_string()));
        }
        
        let mut event = AuditEvent::new("emergency_stop", user_id, "all_trading_halted", "success");
        event.details = Some("Emergency stop activated".to_string());
        self.log_audit(event);
        
        Ok(())
    }
    
    /// Get statistics
    /// Güvenlik istatistikleri
    pub fn stats(&self) -> HashMap<String, usize> {
        let mut stats = HashMap::new();
        stats.insert("total_users".to_string(), self.users.len());
        stats.insert("audit_events".to_string(), self.audit_log.len());
        stats.insert("rate_limits".to_string(), self.rate_limits.len());
        
        let successful_trades = self.audit_log
            .iter()
            .filter(|e| e.event_type == "trade" && e.result == "success")
            .count();
        stats.insert("successful_trades".to_string(), successful_trades);
        
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_creation() {
        let mut security = SecurityManager::new();
        let admin = User {
            id: "admin1".to_string(),
            username: "admin".to_string(),
            role: UserRole::Admin,
            api_key_hash: "hash123".to_string(),
            created_at: Utc::now(),
            last_login: None,
            is_active: true,
        };
        
        security.users.insert("admin1".to_string(), admin.clone());
        
        let trader = User {
            id: "trader1".to_string(),
            username: "trader".to_string(),
            role: UserRole::Trader,
            api_key_hash: "hash456".to_string(),
            created_at: Utc::now(),
            last_login: None,
            is_active: true,
        };
        
        assert!(security.add_user("admin1", trader).is_ok());
    }

    #[test]
    fn test_trade_authorization() {
        let mut security = SecurityManager::new();
        
        let user = User {
            id: "trader1".to_string(),
            username: "trader".to_string(),
            role: UserRole::Trader,
            api_key_hash: "hash".to_string(),
            created_at: Utc::now(),
            last_login: None,
            is_active: true,
        };
        
        security.users.insert("trader1".to_string(), user);
        security.set_rate_limit(RateLimitRule {
            limit_type: "trades_per_minute".to_string(),
            max_per_second: 1,
            applies_to: "all".to_string(),
        });
        
        assert!(security.can_execute_trade("trader1", "BTCUSDT", 1.0).is_ok());
    }

    #[test]
    fn test_audit_log() {
        let mut security = SecurityManager::new();
        
        let event = AuditEvent::new("test_event", "user1", "test_action", "success");
        security.log_audit(event);
        
        let logs = security.get_audit_logs(Some("test_event"), None, 10);
        assert_eq!(logs.len(), 1);
    }

    #[test]
    fn test_rate_limit() {
        let mut limiter = RateLimiterTracker::new();
        
        // Should allow 2 requests per second
        assert!(limiter.check_limit("key1", 2));
        assert!(limiter.check_limit("key1", 2));
        assert!(!limiter.check_limit("key1", 2)); // Third should fail
    }
}
