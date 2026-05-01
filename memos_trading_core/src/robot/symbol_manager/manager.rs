use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use crate::Result;
use crate::MemosTradingError;
use serde::{Deserialize, Serialize};

/// Bir symbol'ün ticaret durumunu temsil eder
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolState {
    pub symbol: String,
    pub enabled: bool,
    pub max_position_size: f64,
    pub position_amount: f64,
    pub entry_price: Option<f64>,
    pub current_price: f64,
    pub pnl: f64,
    pub pnl_pct: f64,
    pub last_signal: Option<String>, // "BUY" | "SELL" | "HOLD"
    pub consecutive_losses: usize,
    pub max_consecutive_losses: usize,
}

impl SymbolState {
    pub fn new(symbol: &str, max_position_size: f64) -> Self {
        Self {
            symbol: symbol.to_string(),
            enabled: true,
            max_position_size,
            position_amount: 0.0,
            entry_price: None,
            current_price: 0.0,
            pnl: 0.0,
            pnl_pct: 0.0,
            last_signal: None,
            consecutive_losses: 0,
            max_consecutive_losses: 3,
        }
    }

    /// Pozisyon açılırsa çağır
    pub fn open_position(&mut self, entry_price: f64, amount: f64) {
        self.entry_price = Some(entry_price);
        self.position_amount = amount;
        self.current_price = entry_price;
        self.pnl = 0.0;
        self.pnl_pct = 0.0;
    }

    /// Pozisyon kapanırsa çağır
    pub fn close_position(&mut self, exit_price: f64) {
        if let Some(entry) = self.entry_price {
            self.pnl = (exit_price - entry) * self.position_amount;
            self.pnl_pct = ((exit_price - entry) / entry) * 100.0;
            
            if self.pnl < 0.0 {
                self.consecutive_losses += 1;
            } else {
                self.consecutive_losses = 0;
            }
        }
        
        self.entry_price = None;
        self.position_amount = 0.0;
        self.current_price = exit_price;
    }

    /// Mevcut fiyatı güncelle (unrealized PnL hesapla)
    pub fn update_price(&mut self, price: f64) {
        self.current_price = price;
        
        if let Some(entry) = self.entry_price {
            if self.position_amount > 0.0 {
                self.pnl = (price - entry) * self.position_amount;
                self.pnl_pct = ((price - entry) / entry) * 100.0;
            }
        }
    }

    /// Circuit breaker kontrolü
    pub fn should_disable(&self) -> bool {
        self.consecutive_losses >= self.max_consecutive_losses
    }
}

/// Multi-symbol yöneticisi
pub struct SymbolManager {
    symbols: Arc<Mutex<HashMap<String, SymbolState>>>,
}

impl Clone for SymbolManager {
    fn clone(&self) -> Self {
        Self {
            symbols: Arc::clone(&self.symbols),
        }
    }
}

impl SymbolManager {
    pub fn new() -> Self {
        Self {
            symbols: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Symbol ekle
    pub fn add_symbol(&self, symbol: &str, max_position_size: f64) -> Result<()> {
        let mut symbols = self.symbols.lock().unwrap();
        symbols.insert(
            symbol.to_string(),
            SymbolState::new(symbol, max_position_size),
        );
        Ok(())
    }

    /// Birden fazla symbol ekle
    pub fn add_symbols(&self, symbol_configs: Vec<(&str, f64)>) -> Result<()> {
        for (symbol, max_size) in symbol_configs {
            self.add_symbol(symbol, max_size)?;
        }
        Ok(())
    }

    /// Symbol durumunu al
    pub fn get_symbol(&self, symbol: &str) -> Result<Option<SymbolState>> {
        let symbols = self.symbols.lock().unwrap();
        Ok(symbols.get(symbol).cloned())
    }

    /// Tüm symbol'leri al
    pub fn get_all_symbols(&self) -> Result<Vec<SymbolState>> {
        let symbols = self.symbols.lock().unwrap();
        Ok(symbols.values().cloned().collect())
    }

    /// Etkin symbol'leri al
    pub fn get_enabled_symbols(&self) -> Result<Vec<String>> {
        let symbols = self.symbols.lock().unwrap();
        Ok(symbols
            .values()
            .filter(|s| s.enabled)
            .map(|s| s.symbol.clone())
            .collect())
    }

    /// Symbol durumunu güncelle
    pub fn update_symbol_price(&self, symbol: &str, price: f64) -> Result<()> {
        let mut symbols = self.symbols.lock().unwrap();
        if let Some(state) = symbols.get_mut(symbol) {
            state.update_price(price);
            Ok(())
        } else {
            Err(MemosTradingError::Strategy(
                format!("Symbol {} bulunamadı", symbol)
            ))
        }
    }

    /// Pozisyon aç
    pub fn open_position(&self, symbol: &str, entry_price: f64, amount: f64) -> Result<()> {
        let mut symbols = self.symbols.lock().unwrap();
        if let Some(state) = symbols.get_mut(symbol) {
            if amount > state.max_position_size {
                return Err(MemosTradingError::Risk(
                    format!(
                        "Pozisyon boyutu {} > maksimum {}",
                        amount, state.max_position_size
                    )
                ));
            }
            state.open_position(entry_price, amount);
            Ok(())
        } else {
            Err(MemosTradingError::Strategy(
                format!("Symbol {} bulunamadı", symbol)
            ))
        }
    }

    /// Pozisyon kapat
    pub fn close_position(&self, symbol: &str, exit_price: f64) -> Result<()> {
        let mut symbols = self.symbols.lock().unwrap();
        if let Some(state) = symbols.get_mut(symbol) {
            state.close_position(exit_price);
            Ok(())
        } else {
            Err(MemosTradingError::Strategy(
                format!("Symbol {} bulunamadı", symbol)
            ))
        }
    }

    /// Symbol'ü devre dışı bırak (circuit breaker)
    pub fn disable_symbol(&self, symbol: &str) -> Result<()> {
        let mut symbols = self.symbols.lock().unwrap();
        if let Some(state) = symbols.get_mut(symbol) {
            state.enabled = false;
            Ok(())
        } else {
            Err(MemosTradingError::Strategy(
                format!("Symbol {} bulunamadı", symbol)
            ))
        }
    }

    /// Symbol'ü etkinleştir
    pub fn enable_symbol(&self, symbol: &str) -> Result<()> {
        let mut symbols = self.symbols.lock().unwrap();
        if let Some(state) = symbols.get_mut(symbol) {
            state.enabled = true;
            state.consecutive_losses = 0;
            Ok(())
        } else {
            Err(MemosTradingError::Strategy(
                format!("Symbol {} bulunamadı", symbol)
            ))
        }
    }

    /// Symbol sayısı
    pub fn count_symbols(&self) -> usize {
        let symbols = self.symbols.lock().unwrap();
        symbols.len()
    }

    /// Etkin symbol sayısı
    pub fn count_enabled_symbols(&self) -> usize {
        let symbols = self.symbols.lock().unwrap();
        symbols.values().filter(|s| s.enabled).count()
    }

    /// Portfolio istatistikleri
    pub fn get_portfolio_stats(&self) -> Result<PortfolioStats> {
        let symbols = self.symbols.lock().unwrap();
        let total_pnl: f64 = symbols.values().map(|s| s.pnl).sum();
        let active_positions = symbols.values().filter(|s| s.position_amount > 0.0).count();

        Ok(PortfolioStats {
            total_symbols: symbols.len(),
            enabled_symbols: symbols.values().filter(|s| s.enabled).count(),
            active_positions,
            total_pnl,
            disabled_by_circuit_breaker: symbols.values().filter(|s| !s.enabled).count(),
        })
    }
}

/// Portfolio istatistikleri
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioStats {
    pub total_symbols: usize,
    pub enabled_symbols: usize,
    pub active_positions: usize,
    pub total_pnl: f64,
    pub disabled_by_circuit_breaker: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_state_creation() {
        let state = SymbolState::new("BTC", 10.0);
        assert_eq!(state.symbol, "BTC");
        assert!(state.enabled);
        assert_eq!(state.max_position_size, 10.0);
    }

    #[test]
    fn test_symbol_state_open_close_position() {
        let mut state = SymbolState::new("BTC", 10.0);
        
        // Pozisyon aç
        state.open_position(100.0, 1.0);
        assert_eq!(state.entry_price, Some(100.0));
        assert_eq!(state.position_amount, 1.0);
        
        // Pozisyon kapat (kar)
        state.close_position(110.0);
        assert_eq!(state.pnl, 10.0);
        assert_eq!(state.pnl_pct, 10.0);
        assert_eq!(state.consecutive_losses, 0);
    }

    #[test]
    fn test_symbol_state_loss_tracking() {
        let mut state = SymbolState::new("BTC", 10.0);
        
        // Zararlı kapatış
        state.open_position(100.0, 1.0);
        state.close_position(90.0);
        assert_eq!(state.consecutive_losses, 1);
        
        // İkinci zarar
        state.open_position(90.0, 1.0);
        state.close_position(85.0);
        assert_eq!(state.consecutive_losses, 2);
        
        // Circuit breaker kontrolü
        assert!(!state.should_disable());
        state.consecutive_losses = 3;
        assert!(state.should_disable());
    }

    #[test]
    fn test_symbol_manager_creation() {
        let manager = SymbolManager::new();
        assert_eq!(manager.count_symbols(), 0);
    }

    #[test]
    fn test_symbol_manager_add_symbols() {
        let manager = SymbolManager::new();
        manager.add_symbols(vec![
            ("BTC", 10.0),
            ("ETH", 20.0),
            ("XRP", 50.0),
        ]).unwrap();
        
        assert_eq!(manager.count_symbols(), 3);
    }

    #[test]
    fn test_symbol_manager_get_symbol() {
        let manager = SymbolManager::new();
        manager.add_symbol("BTC", 10.0).unwrap();
        
        let symbol = manager.get_symbol("BTC").unwrap();
        assert!(symbol.is_some());
        assert_eq!(symbol.unwrap().symbol, "BTC");
    }

    #[test]
    fn test_symbol_manager_enable_disable() {
        let manager = SymbolManager::new();
        manager.add_symbol("BTC", 10.0).unwrap();
        
        assert_eq!(manager.count_enabled_symbols(), 1);
        
        manager.disable_symbol("BTC").unwrap();
        assert_eq!(manager.count_enabled_symbols(), 0);
        
        manager.enable_symbol("BTC").unwrap();
        assert_eq!(manager.count_enabled_symbols(), 1);
    }

    #[test]
    fn test_symbol_manager_position_tracking() {
        let manager = SymbolManager::new();
        manager.add_symbols(vec![
            ("BTC", 10.0),
            ("ETH", 20.0),
        ]).unwrap();
        
        manager.open_position("BTC", 100.0, 1.0).unwrap();
        manager.open_position("ETH", 50.0, 2.0).unwrap();
        
        let stats = manager.get_portfolio_stats().unwrap();
        assert_eq!(stats.active_positions, 2);
        assert_eq!(stats.total_symbols, 2);
    }

    #[test]
    fn test_symbol_manager_portfolio_stats() {
        let manager = SymbolManager::new();
        manager.add_symbols(vec![
            ("BTC", 10.0),
            ("ETH", 20.0),
            ("XRP", 50.0),
        ]).unwrap();
        
        manager.open_position("BTC", 100.0, 1.0).unwrap();
        manager.close_position("BTC", 110.0).unwrap();
        
        let stats = manager.get_portfolio_stats().unwrap();
        assert_eq!(stats.total_symbols, 3);
        assert_eq!(stats.total_pnl, 10.0);
    }
}
