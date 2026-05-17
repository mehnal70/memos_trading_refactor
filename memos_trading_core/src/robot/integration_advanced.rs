// Entegrasyon: Dinamik Pozisyon + Güvenlik + Risk Kontrol
// Integration: Complete trading workflow combining DynamicPosition + SecurityManager

use crate::robot::portfolio_manager::DynamicPosition;
use crate::robot::security::{SecurityManager, AuditEvent};
use crate::MemosTradingError;
use crate::robot::infra::error::ErrorLogger;
use std::collections::HashMap;
use std::sync::Arc;

type Result<T> = std::result::Result<T, MemosTradingError>;

/// İleri Robotic Trader - Dinamik pozisyon + güvenlik + loglama ile tamamlanmış
pub struct AdvancedRoboticTrader {
    /// Güvenlik yönetimi ve denetim izleri
    pub security: SecurityManager,
    /// Açık pozisyonlar
    pub positions: HashMap<String, DynamicPosition>,
    /// İşlem yapan kullanıcı ID
    pub user_id: String,
    /// Logger reference
    logger: Option<Arc<dyn ErrorLogger>>,
}

impl AdvancedRoboticTrader {
    /// Yeni Advanced Robotic Trader oluştur
    pub fn new(security: SecurityManager, user_id: String) -> Self {
        Self {
            security,
            positions: HashMap::new(),
            user_id,
            logger: None,
        }
    }
    
    /// Logger'ı set et (optional)
    pub fn with_logger(&mut self, logger: Arc<dyn ErrorLogger>) -> &mut Self {
        self.logger = Some(logger);
        self
    }
    
    #[allow(dead_code)]
    /// İç log helper
    fn log(&self, msg: &str, is_error: bool) {
        if let Some(ref logger) = self.logger {
            if is_error {
                logger.log_error("trader", msg);
            } else {
                logger.log_info("trader", msg);
            }
        }
    }
    
    // ============ GİRİŞ YÖNETİMİ ============
    
    /// Yeni trade başlat (güvenlik kontrolleri + hata loglama)
    pub fn open_trade(
        &mut self,
        symbol: &str,
        quantity: f64,
        entry_price: f64,
        stop_loss: Option<f64>,
        take_profit: Option<f64>,
    ) -> Result<()> {
        // 1. Güvenlik kontrolü: Kullanıcı trade yapabilir mi?
        self.security.can_execute_trade(&self.user_id, symbol, quantity)?;
        
        // 2. Pozisyon zaten açık mı?
        if self.positions.contains_key(symbol) {
            return Err(crate::MemosTradingError::Unknown(
                format!("Position already open for {}", symbol)
            ));
        }
        
        // 3. Pozisyonu aç
        let position = DynamicPosition::new(
            symbol.to_string(),
            entry_price,
            quantity,
            1.0, // long
            stop_loss,
            take_profit,
        );
        
        self.positions.insert(symbol.to_string(), position);
        
        // 4. Denetim lojuna log et
        let mut event = AuditEvent::new(
            "trade_open",
            &self.user_id,
            format!("LONG {} @ {} (qty: {})", symbol, entry_price, quantity),
            "success",
        );
        event.symbol = Some(symbol.to_string());
        self.security.log_audit(event);
        
        println!("✓ Trade opened: {} x {} @ {}", quantity, symbol, entry_price);
        Ok(())
    }
    
    // ============ KADEMELI GİRİŞ ============
    
    /// Trend güçlenirse ek pozisyon aç
    pub fn scale_in(&mut self, symbol: &str, current_price: f64) -> Result<()> {
        let position = self.positions.get_mut(symbol)
            .ok_or_else(|| crate::MemosTradingError::Unknown(
                format!("No position for {}", symbol)
            ))?;
        
        // Can we scale-in?
        if !position.can_scalein(current_price) {
            return Err(crate::MemosTradingError::Unknown(
                "Scale-in conditions not met".to_string()
            ));
        }
        
        // Güvenlik: Rate limit check
        if !self.security.check_rate_limit(&self.user_id, "trades_per_minute") {
            return Err(crate::MemosTradingError::Unknown(
                "Rate limit exceeded".to_string()
            ));
        }
        
        let scalein_qty = position.calculate_scalein_quantity();
        position.record_scalein(scalein_qty, current_price, 0.0)?;
        
        // Denetim logu
        let mut event = AuditEvent::new(
            "scale_in",
            &self.user_id,
            format!("Added {} to {} @ {}", scalein_qty, symbol, current_price),
            "success",
        );
        event.symbol = Some(symbol.to_string());
        self.security.log_audit(event);
        
        println!("✓ Scale-in: {} x {} @ {}", scalein_qty, symbol, current_price);
        Ok(())
    }
    
    // ============ FİYAT GÜNCELLEMESİ ============
    
    /// Market fiyatını güncelle - trailing stop + scale-out trigger kontrol
    pub fn update_market_price(&mut self, symbol: &str, new_price: f64) -> Result<()> {
        let position = self.positions.get_mut(symbol)
            .ok_or_else(|| crate::MemosTradingError::Unknown(
                format!("No position for {}", symbol)
            ))?;
        
        position.current_price = new_price;
        
        // 1. Trailing stop check
        if position.update_trailing_stop() {
            println!("⚠️ Trailing stop triggered for {}", symbol);
            // Otomatik olarak pozisyonu kapat
            return self.close_position(symbol, new_price);
        }
        
        // 2. Scale-out check (profit target'a ulaştık mı?)
        if let Some(_target) = position.active_profit_target() {
            if position.scaleout_count < position.scaleout_config.max_scaleout_count {
                println!("💰 Profit target reached for {} - attempting scale-out", symbol);
                
                // Güvenlik kontrol
                if !self.security.check_rate_limit(&self.user_id, "trades_per_minute") {
                    println!("⚠️ Rate limit hit, skipping scale-out");
                    return Ok(());
                }
                
                let scaleout_qty = position.calculate_scaleout_quantity();
                position.record_scaleout(scaleout_qty, new_price, 0.0)?;
                
                // Denetim logu
                let mut event = AuditEvent::new(
                    "scale_out",
                    &self.user_id,
                    format!("Closed {} from {} @ {}", scaleout_qty, symbol, new_price),
                    "success",
                );
                event.symbol = Some(symbol.to_string());
                self.security.log_audit(event);
            }
        }
        
        Ok(())
    }
    
    // ============ ÇIKIŞ YÖNETİMİ ============
    
    /// Pozisyonu tamamen kapat ve denetim log'una kaydet
    pub fn close_position(&mut self, symbol: &str, exit_price: f64) -> Result<()> {
        let position = self.positions.remove(symbol)
            .ok_or_else(|| crate::MemosTradingError::Unknown(
                format!("No position for {}", symbol)
            ))?;
        
        let pnl = position.total_pnl();
        let pnl_pct = position.unrealized_pnl_pct();
        
        // Denetim logu
        let mut event = AuditEvent::new(
            "trade_close",
            &self.user_id,
            format!("Closed {} @ {} | PnL: {} ({:.2}%)", 
                symbol, exit_price, pnl, pnl_pct),
            "success",
        );
        event.symbol = Some(symbol.to_string());
        self.security.log_audit(event);
        
        println!("✓ Position closed: {} | Total PnL: {} ({:.2}%)", 
                 symbol, pnl, pnl_pct);
        Ok(())
    }
    
    // ============ RAPORLAMA ============
    
    /// Tüm açık pozisyonların durum özeti
    pub fn positions_summary(&self) -> String {
        let mut summary = String::from("=== AÇIK POZİSYONLAR ===\n");
        
        for (symbol, pos) in &self.positions {
            summary.push_str(&format!(
                "{}: {} @ {:.2} | Current: {:.2} | PnL: {} ({:.2}%) | Trailing SL: {:?}\n",
                symbol,
                pos.quantity,
                pos.entry_price,
                pos.current_price,
                pos.total_pnl() as i64,
                pos.unrealized_pnl_pct(),
                pos.current_trailing_sl
            ));
        }
        
        summary
    }
    
    /// Security dashboard (yöneticiler için)
    pub fn security_report(&self) -> String {
        let stats = self.security.stats();
        let mut report = String::from("=== SECURITY REPORT ===\n");
        
        report.push_str(&format!("Active Users: {}\n", stats.get("total_users").unwrap_or(&0)));
        report.push_str(&format!("Audit Events: {}\n", stats.get("audit_events").unwrap_or(&0)));
        report.push_str(&format!("Successful Trades: {}\n", stats.get("successful_trades").unwrap_or(&0)));
        
        let recent_logs = self.security.get_audit_logs(None, None, 10);
        report.push_str("\nRecent Activity:\n");
        for log in recent_logs.iter().rev().take(5) {
            report.push_str(&format!(
                "  [{}] {} - {} ({})\n",
                log.timestamp.format("%H:%M:%S"),
                log.event_type,
                log.action,
                log.result
            ));
        }
        
        report
    }
    
    /// Acil durdurma - tüm pozisyonları hemen kapat
    pub fn emergency_stop(&mut self, exit_price: f64) -> Result<()> {
        self.security.emergency_stop(&self.user_id)?;
        
        let symbols: Vec<_> = self.positions.keys().cloned().collect();
        for symbol in symbols {
            self.close_position(&symbol, exit_price)?;
        }
        
        println!("🛑 EMERGENCY STOP: All positions closed!");
        Ok(())
    }
}

// ============ ÖRNEK KULLANIM ============
#[cfg(test)]
mod examples {
    use super::*;
    use crate::robot::security::{User, UserRole, RateLimitRule};
    use chrono::Utc;

    #[test]
    fn test_advanced_robotic_flow() {
        let mut security = SecurityManager::new();
        
        // Setup: Admin user ekle (test helper kullan)
        let admin = User {
            id: "admin1".to_string(),
            username: "admin".to_string(),
            role: UserRole::Admin,
            api_key_hash: "admin_hash".to_string(),
            created_at: Utc::now(),
            last_login: None,
            is_active: true,
        };
        security.add_test_user(admin);
        
        // Setup: Trader user ekle (test helper kullan)
        let trader = User {
            id: "trader1".to_string(),
            username: "trader".to_string(),
            role: UserRole::Trader,
            api_key_hash: "trader_hash".to_string(),
            created_at: Utc::now(),
            last_login: None,
            is_active: true,
        };
        security.add_test_user(trader);
        
        // Set high rate limit for testing (100 trades per second)
        security.set_rate_limit(RateLimitRule {
            limit_type: "trades_per_minute".to_string(),
            max_per_second: 100,
            applies_to: "all".to_string(),
        });
        
        let mut trader_manager = AdvancedRoboticTrader::new(security, "trader1".to_string());
        
        // 1. Trade aç
        assert!(trader_manager.open_trade("BTCUSDT", 1.0, 45000.0, Some(44000.0), Some(50000.0)).is_ok());
        
        // 2. Fiyat güncellemesi
        assert!(trader_manager.update_market_price("BTCUSDT", 45500.0).is_ok());
        
        // 3. Scale-in (güncellenmiş fiyatla)
        assert!(trader_manager.scale_in("BTCUSDT", 45500.0).is_ok());
        
        // 4. Summary
        println!("{}", trader_manager.positions_summary());
        
        // 5. Security report
        println!("{}", trader_manager.security_report());
    }
}
