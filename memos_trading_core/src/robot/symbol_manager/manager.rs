use crate::MemosTradingError;
use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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
    pub last_signal: Option<String>,
    pub consecutive_losses: usize,
    pub max_consecutive_losses: usize,
}

impl SymbolState {
    pub fn new(symbol: &str, max_position_size: f64) -> Self {
        return Self {
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
        };
    }

    pub fn open_position(&mut self, entry_price: f64, amount: f64) {
        self.entry_price = Some(entry_price);
        self.position_amount = amount;
        self.current_price = entry_price;
        self.pnl = 0.0;
        self.pnl_pct = 0.0;
    }

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

    pub fn update_price(&mut self, price: f64) {
        self.current_price = price;

        if let Some(entry) = self.entry_price {
            if self.position_amount > 0.0 {
                self.pnl = (price - entry) * self.position_amount;
                self.pnl_pct = ((price - entry) / entry) * 100.0;
            }
        }
    }

    pub fn should_disable(&self) -> bool {
        return self.consecutive_losses >= self.max_consecutive_losses;
    }
}

pub struct SymbolManager {
    symbols: Arc<Mutex<HashMap<String, SymbolState>>>,
}

impl Clone for SymbolManager {
    fn clone(&self) -> Self {
        return Self {
            symbols: Arc::clone(&self.symbols),
        };
    }
}

impl Default for SymbolManager {
    fn default() -> Self {
        return Self::new();
    }
}
impl SymbolManager {
    fn lock_symbols(&self) -> std::sync::MutexGuard<'_, HashMap<String, SymbolState>> {
        match self.symbols.lock() {
            Ok(guard) => return guard,
            Err(poison) => return poison.into_inner(),
        }
    }

    pub fn new() -> Self {
        return Self {
            symbols: Arc::new(Mutex::new(HashMap::new())),
        };
    }

    pub fn add_symbol(&self, symbol: &str, max_position_size: f64) -> Result<()> {
        let mut symbols = self.lock_symbols();
        let _ = symbols.insert(
            symbol.to_string(),
            SymbolState::new(symbol, max_position_size),
        );
        return Ok(());
    }

    pub fn add_symbols(&self, symbol_configs: Vec<(&str, f64)>) -> Result<()> {
        for (symbol, max_size) in symbol_configs {
            self.add_symbol(symbol, max_size)?;
        }
        return Ok(());
    }

    pub fn get_symbol(&self, symbol: &str) -> Result<Option<SymbolState>> {
        let symbols = self.lock_symbols();
        return Ok(symbols.get(symbol).cloned());
    }

    pub fn get_all_symbols(&self) -> Result<Vec<SymbolState>> {
        let symbols = self.lock_symbols();
        return Ok(symbols.values().cloned().collect());
    }

    pub fn get_enabled_symbols(&self) -> Result<Vec<String>> {
        let symbols = self.lock_symbols();
        return Ok(symbols
            .values()
            .filter(|s| s.enabled)
            .map(|s| s.symbol.clone())
            .collect());
    }

    pub fn update_symbol_price(&self, symbol: &str, price: f64) -> Result<()> {
        let mut symbols = self.lock_symbols();
        if let Some(state) = symbols.get_mut(symbol) {
            state.update_price(price);
            return Ok(());
        } else {
            return Err(MemosTradingError::Strategy(format!(
                "Symbol {} bulunamadı",
                symbol
            )));
        }
    }

    pub fn open_position(&self, symbol: &str, entry_price: f64, amount: f64) -> Result<()> {
        let mut symbols = self.lock_symbols();
        if let Some(state) = symbols.get_mut(symbol) {
            if amount > state.max_position_size {
                return Err(MemosTradingError::Risk(format!(
                    "Pozisyon boyutu {} > maksimum {}",
                    amount, state.max_position_size
                )));
            }
            state.open_position(entry_price, amount);
            return Ok(());
        } else {
            return Err(MemosTradingError::Strategy(format!(
                "Symbol {} bulunamadı",
                symbol
            )));
        }
    }

    pub fn close_position(&self, symbol: &str, exit_price: f64) -> Result<()> {
        let mut symbols = self.lock_symbols();
        if let Some(state) = symbols.get_mut(symbol) {
            state.close_position(exit_price);
            return Ok(());
        } else {
            return Err(MemosTradingError::Strategy(format!(
                "Symbol {} bulunamadı",
                symbol
            )));
        }
    }

    pub fn disable_symbol(&self, symbol: &str) -> Result<()> {
        let mut symbols = self.lock_symbols();
        if let Some(state) = symbols.get_mut(symbol) {
            state.enabled = false;
            return Ok(());
        } else {
            return Err(MemosTradingError::Strategy(format!(
                "Symbol {} bulunamadı",
                symbol
            )));
        }
    }
    pub fn enable_symbol(&self, symbol: &str) -> Result<()> {
        let mut symbols = self.lock_symbols();
        if let Some(state) = symbols.get_mut(symbol) {
            state.enabled = true;
            state.consecutive_losses = 0;
            return Ok(());
        } else {
            return Err(MemosTradingError::Strategy(format!(
                "Symbol {} bulunamadı",
                symbol
            )));
        }
    }

    pub fn count_symbols(&self) -> usize {
        let symbols = self.lock_symbols();
        return symbols.len();
    }

    pub fn count_enabled_symbols(&self) -> usize {
        let symbols = self.lock_symbols();
        return symbols.values().filter(|s| s.enabled).count();
    }

    pub fn get_portfolio_stats(&self) -> Result<PortfolioStats> {
        let symbols = self.lock_symbols();
        let total_pnl: f64 = symbols.values().map(|s| s.pnl).sum();
        let active_positions = symbols.values().filter(|s| s.position_amount > 0.0).count();

        return Ok(PortfolioStats {
            total_symbols: symbols.len(),
            enabled_symbols: symbols.values().filter(|s| s.enabled).count(),
            active_positions,
            total_pnl,
            disabled_by_circuit_breaker: symbols.values().filter(|s| !s.enabled).count(),
        });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioStats {
    pub total_symbols: usize,
    pub enabled_symbols: usize,
    pub active_positions: usize,
    pub total_pnl: f64,
    pub disabled_by_circuit_breaker: usize,
}
