// src/robot/security/manager.rs - Güvenlik İnfaz Yöneticisi
// Srivastava ATP - İşlevsel Güvenlik Çekirdeği

use crate::prelude::*;
use super::types::{User, AuditEvent};
use super::tracker::{RateLimitRule, RateLimiterTracker};
use super::vault::ApiKeyManager;

use std::collections::HashMap;
use chrono::Utc;

pub struct SecurityManager {
    pub users: HashMap<String, User>,
    pub audit_log: Vec<AuditEvent>,
    pub rate_limiter: RateLimiterTracker,
    pub rate_limits: HashMap<String, RateLimitRule>,
    pub api_key_manager: Option<ApiKeyManager>,
}

impl Default for SecurityManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SecurityManager {
    pub fn new() -> Self {
        Self {
            users: HashMap::new(),
            audit_log: vec![],
            rate_limiter: RateLimiterTracker::new(),
            rate_limits: HashMap::new(),
            api_key_manager: None,
        }
    }
    
    pub fn init_api_keys(&mut self, exchanges: &[&str]) -> Result<(), crate::MemosTradingError> {
        self.api_key_manager = Some(ApiKeyManager::from_env(exchanges)?);
        crate::robot::infra::reporting::reporting::ErrorLogger::log_repair("SECURITY", "API anahtarları güvenli şekilde yüklendi.");
        
        self.log_audit(AuditEvent::new("system", "system", "api_keys_initialized", "success"));
        Ok(())
    }
    
    pub fn add_user(&mut self, current_user_id: &str, new_user: User) -> Result<(), crate::MemosTradingError> {
        let caller = self.users.get(current_user_id)
            .ok_or_else(|| crate::MemosTradingError::Config("Arayan kullanıcı bulunamadı".to_string()))?;
        
        if !caller.role.can_modify_settings() {
            let mut event = AuditEvent::new("user_creation", current_user_id, format!("user_id: {}", new_user.id), "failure");
            event.details = Some("Yetki Reddedildi: Sadece Admin kullanıcı ekleyebilir".to_string());
            self.log_audit(event);
            return Err(crate::MemosTradingError::Config("Yetkisiz İşlem".to_string()));
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

    /// Test/dev ortamı için kullanıcıyı yetki kontrolü atlanarak ekler.
    /// Üretimde `add_user(admin_id, ...)` çağrılmalı.
    pub fn add_test_user(&mut self, new_user: User) {
        self.users.insert(new_user.id.clone(), new_user.clone());
        self.log_audit(AuditEvent::new(
            "user_creation_test",
            "test_harness",
            format!("user_id: {}, role: {:?}", new_user.id, new_user.role),
            "success",
        ));
    }

    pub fn set_rate_limit(&mut self, rule: RateLimitRule) {
        let limit_type = rule.limit_type.clone();
        self.rate_limits.insert(limit_type, rule);
    }
    
    pub fn check_rate_limit(&mut self, user_id: &str, action_type: &str) -> bool {
        let rule = self.rate_limits.get(action_type).cloned().unwrap_or_default();
        let key = format!("{}:{}", user_id, action_type);
        self.rate_limiter.check_limit(&key, rule.max_per_second)
    }
    
    pub fn log_audit(&mut self, mut event: AuditEvent) {
        event.timestamp = Utc::now();
        if event.result == "failure" {
            crate::robot::infra::reporting::reporting::ErrorLogger::log_error(
                "SECURITY_AUDIT", 
                &format!("{}: {} - BAŞARISIZ", event.user_id, event.action)
            );
        }
        self.audit_log.push(event);
    }
    
    pub fn get_audit_logs(&self, event_type: Option<&str>, user_id: Option<&str>, limit: usize) -> Vec<AuditEvent> {
        self.audit_log.iter()
            .filter(|e| {
                (event_type.is_none() || Some(e.event_type.as_str()) == event_type)
                    && (user_id.is_none() || Some(e.user_id.as_str()) == user_id)
            })
            .rev().take(limit).cloned().collect()
    }
    
    pub fn export_audit_log(&self, query_type: &str) -> String {
        let mut csv = String::from("timestamp,event_type,user_id,action,result,symbol,trade_id\n");
        let filtered: Vec<_> = match query_type {
            "trades" => self.audit_log.iter().filter(|e| e.event_type == "trade" && e.result == "success").collect(),
            _ => self.audit_log.iter().collect(),
        };
        
        for event in filtered {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{}\n",
                event.timestamp.format("%Y-%m-%d %H:%M:%S"),
                event.event_type, event.user_id, event.action, event.result,
                event.symbol.as_deref().unwrap_or(""), event.trade_id.as_deref().unwrap_or("")
            ));
        }
        csv
    }
    
    pub fn can_execute_trade(&mut self, user_id: &str, symbol: &str, size: f64) -> Result<(), crate::MemosTradingError> {
        let user = self.users.get(user_id)
            .ok_or_else(|| crate::MemosTradingError::Config("Kullanıcı bulunamadı".to_string()))?;
        
        if !user.is_active {
            let mut event = AuditEvent::new("trade_authorization", user_id, format!("symbol: {}, size: {}", symbol, size), "failure");
            event.details = Some("Kullanıcı pasif durumda".to_string());
            event.symbol = Some(symbol.to_string());
            self.log_audit(event);
            return Err(crate::MemosTradingError::Config("Kullanıcı aktif değil".to_string()));
        }
        
        if !user.role.can_trade() {
            let mut event = AuditEvent::new("trade_authorization", user_id, format!("symbol: {}, size: {}", symbol, size), "failure");
            event.details = Some(format!("Bu rol ({:?}) işlem açamaz", user.role));
            event.symbol = Some(symbol.to_string());
            self.log_audit(event);
            return Err(crate::MemosTradingError::Config("Rol yetkisi yetersiz".to_string()));
        }
        
        if !self.check_rate_limit(user_id, "trades_per_minute") {
            let mut event = AuditEvent::new("trade_authorization", user_id, format!("symbol: {}, size: {}", symbol, size), "failure");
            event.details = Some("Dakikalık işlem limiti aşıldı (Rate limit)".to_string());
            event.symbol = Some(symbol.to_string());
            self.log_audit(event);
            return Err(crate::MemosTradingError::Config("Akış limiti aşıldı".to_string()));
        }
        
        let mut event = AuditEvent::new("trade_authorization", user_id, format!("symbol: {}, size: {}", symbol, size), "success");
        event.symbol = Some(symbol.to_string());
        self.log_audit(event);
        Ok(())
    }

    // ============ 5. KISIM EKLEMELERİ (MÜHÜRLÜ) ============

    /// Gerçekleşen işlemleri adli log kaydına işler
    pub fn log_trade(&mut self, user_id: &str, trade_id: &str, symbol: &str, side: &str, size: f64, price: f64) {
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
    
    /// 🚨 ACİL DURDURMA (CIRCUIT BREAKER KÖPRÜSÜ)
    pub fn emergency_stop(&mut self, user_id: &str) -> Result<(), crate::MemosTradingError> {
        let user = self.users.get(user_id)
            .ok_or_else(|| crate::MemosTradingError::Config("Kullanıcı bulunamadı".to_string()))?;
        
        if !user.role.can_emergency_stop() {
            let event = AuditEvent::new("emergency_stop", user_id, "triggered", "failure");
            self.log_audit(event);
            return Err(crate::MemosTradingError::Config("Bu kullanıcı acil durdurma tetikleyemez".to_string()));
        }
        
        let mut event = AuditEvent::new("emergency_stop", user_id, "all_trading_halted", "success");
        event.details = Some("Acil durdurma otonom olarak aktif edildi".to_string());
        self.log_audit(event);
        
        Ok(())
    }
    
    /// Güvenlik garnizonu canlı istatistiklerini hasat eder
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

// =============================================================================
// 4. ADLİ GÜVENLİK BİRİM TESTLERİ (AUTOMATED AUDIT TESTS)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::UserRole;

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
        assert!(limiter.check_limit("key1", 2));
        assert!(limiter.check_limit("key1", 2));
        assert!(!limiter.check_limit("key1", 2)); 
    }
}
