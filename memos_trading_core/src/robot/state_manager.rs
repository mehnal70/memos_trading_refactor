// robot/state_manager.rs - Merkezi State & Account Store interface ve örnek in-memory adapter

use crate::types::{Trade};
use crate::Result;

/// Hesap, bakiye, sembol, pozisyon ve trade yönetimi için standart arayüz
pub trait StateManager: Send + Sync {
    fn get_symbols(&self) -> Result<Vec<String>>;
    fn set_symbols(&mut self, symbols: Vec<String>) -> Result<()>;
    fn get_balance(&self) -> Result<f64>;
    fn set_balance(&mut self, balance: f64) -> Result<()>;
    fn get_trades(&self) -> Result<Vec<Trade>>;
    fn add_trade(&mut self, trade: Trade) -> Result<()>;
}

/// Basit in-memory StateManager (örnek, test amaçlı)
pub struct InMemoryStateManager {
    symbols: Vec<String>,
    balance: f64,
    trades: Vec<Trade>,
}

impl InMemoryStateManager {
    pub fn new() -> Self {
        Self { symbols: vec![], balance: 0.0, trades: vec![] }
    }
}

impl StateManager for InMemoryStateManager {
    fn get_symbols(&self) -> Result<Vec<String>> {
        Ok(self.symbols.clone())
    }
    fn set_symbols(&mut self, symbols: Vec<String>) -> Result<()> {
        self.symbols = symbols;
        Ok(())
    }
    fn get_balance(&self) -> Result<f64> {
        Ok(self.balance)
    }
    fn set_balance(&mut self, balance: f64) -> Result<()> {
        self.balance = balance;
        Ok(())
    }
    fn get_trades(&self) -> Result<Vec<Trade>> {
        Ok(self.trades.clone())
    }
    fn add_trade(&mut self, trade: Trade) -> Result<()> {
        self.trades.push(trade);
        Ok(())
    }
}
