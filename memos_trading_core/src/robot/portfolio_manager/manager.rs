// Portfolio Manager - Ana Implementasyon
//
// Srivastava mimarisi: Multi-position tracking ve PnL yönetimi

use super::types::*;
use crate::Result as MemosTradingResult;
use crate::MemosTradingError;
use chrono::Utc;
use std::collections::HashMap;

/// Ana Portfolio Manager
#[allow(dead_code)]
pub struct PortfolioManager {
    /// Toplam sermaye
    initial_capital: f64,
    
    /// Mevcut cash
    available_cash: f64,
    
    /// Açık pozisyonlar (symbol → Position)
    positions: HashMap<String, Position>,
    
    /// Kapalı işlemler
    closed_trades: Vec<ClosedTrade>,
    
    /// En yüksek portfolio value (drawdown hesabı için)
    peak_value: f64,
    
    /// Equity history (hesaplanan)
    equity_history: Vec<(chrono::DateTime<Utc>, f64)>,
}

impl PortfolioManager {
    pub fn new(initial_capital: f64) -> Self {
        Self {
            initial_capital,
            available_cash: initial_capital,
            positions: HashMap::new(),
            closed_trades: vec![],
            peak_value: initial_capital,
            equity_history: vec![(Utc::now(), initial_capital)],
        }
    }
    
    // ============ Position Management ============
    
    /// Yeni pozisyon aç
    pub fn open_position(
        &mut self,
        symbol: String,
        entry_price: f64,
        quantity: f64,
        direction: f64, // 1.0 = long, -1.0 = short
        stop_loss: Option<f64>,
        take_profit: Option<f64>,
    ) -> MemosTradingResult<()> {
        // Kontrol: Yeterli cash var mı?
        let required = entry_price * quantity;
        if self.available_cash < required {
            return Err(MemosTradingError::Unknown(
                format!(
                    "Insufficient cash: need {}, have {}",
                    required, self.available_cash
                )
            ));
        }
        
        // Kontrol: Aynı sembol için sadece bir pozisyon
        if self.positions.contains_key(&symbol) {
            return Err(MemosTradingError::Unknown(
                format!("Position already open for {}", symbol)
            ));
        }
        
        let position = Position {
            symbol: symbol.clone(),
            entry_price,
            quantity,
            direction,
            entry_time: Utc::now(),
            current_price: entry_price,
            stop_loss,
            take_profit,
        };
        
        self.positions.insert(symbol.clone(), position);
        self.available_cash -= required;
        
        println!("✓ Pozisyon açıldı: {} x {:.2} @ {:.2}", 
                 quantity, entry_price, symbol);
        
        Ok(())
    }
    
    /// Pozisyonu kapat
    pub fn close_position(&mut self, symbol: &str, exit_price: f64) -> MemosTradingResult<()> {
        let position = self.positions
            .remove(symbol)
            .ok_or_else(|| MemosTradingError::Unknown(
                format!("Position not found: {}", symbol)
            ))?;
        
        let pnl = (exit_price - position.entry_price) * position.quantity * position.direction;
        let pnl_pct = if position.entry_price > 0.0 {
            (pnl / (position.entry_price * position.quantity)) * 100.0
        } else {
            0.0
        };
        
        let closed = ClosedTrade {
            symbol: position.symbol.clone(),
            entry_price: position.entry_price,
            exit_price,
            quantity: position.quantity,
            direction: position.direction,
            entry_time: position.entry_time,
            exit_time: Utc::now(),
            realized_pnl: pnl,
            realized_pnl_pct: pnl_pct,
        };
        
        self.closed_trades.push(closed);
        self.available_cash += exit_price * position.quantity;
        
        println!("✓ Pozisyon kapatıldı: {} | PnL: {:.2} ({:.2}%)", 
                 symbol, pnl, pnl_pct);
        
        Ok(())
    }
    
    /// Pozisyon fiyatlarını güncelle
    pub fn update_prices(&mut self, prices: HashMap<String, f64>) {
        for (symbol, price) in prices {
            if let Some(pos) = self.positions.get_mut(&symbol) {
                pos.current_price = price;
            }
        }
    }
    
    // ============ Queries ============
    
    pub fn total_capital(&self) -> f64 {
        self.initial_capital
    }
    
    pub fn available_cash(&self) -> f64 {
        self.available_cash
    }
    
    pub fn positions_count(&self) -> usize {
        self.positions.len()
    }
    
    pub fn get_position(&self, symbol: &str) -> Option<&Position> {
        self.positions.get(symbol)
    }
    
    pub fn all_positions(&self) -> Vec<&Position> {
        self.positions.values().collect()
    }
    
    pub fn closed_trades_count(&self) -> usize {
        self.closed_trades.len()
    }
    
    pub fn get_closed_trades(&self) -> &[ClosedTrade] {
        &self.closed_trades
    }
    
    // ============ Metrics Calculation ============
    
    /// Unrealized PnL (tüm açık pozisyonlar)
    pub fn unrealized_pnl(&self) -> f64 {
        self.positions
            .values()
            .map(|p| p.unrealized_pnl())
            .sum()
    }
    
    /// Realized PnL (tüm kapalı işlemler)
    pub fn realized_pnl(&self) -> f64 {
        self.closed_trades
            .iter()
            .map(|t| t.realized_pnl)
            .sum()
    }
    
    /// Portfolio equity value
    pub fn equity_value(&self) -> f64 {
        self.initial_capital + self.unrealized_pnl() + self.realized_pnl()
    }
    
    /// Open positions'ların toplam value
    pub fn open_positions_value(&self) -> f64 {
        self.positions
            .values()
            .map(|p| p.position_value())
            .sum()
    }
    
    /// Win rate
    pub fn win_rate(&self) -> f64 {
        if self.closed_trades.is_empty() {
            return 0.0;
        }
        let wins = self.closed_trades.iter().filter(|t| t.is_win()).count();
        wins as f64 / self.closed_trades.len() as f64
    }
    
    /// Average win size
    pub fn avg_win(&self) -> f64 {
        let wins: Vec<f64> = self.closed_trades
            .iter()
            .filter(|t| t.is_win())
            .map(|t| t.realized_pnl)
            .collect();
        
        if wins.is_empty() {
            0.0
        } else {
            wins.iter().sum::<f64>() / wins.len() as f64
        }
    }
    
    /// Average loss size
    pub fn avg_loss(&self) -> f64 {
        let losses: Vec<f64> = self.closed_trades
            .iter()
            .filter(|t| !t.is_win())
            .map(|t| t.realized_pnl)
            .collect();
        
        if losses.is_empty() {
            0.0
        } else {
            losses.iter().sum::<f64>() / losses.len() as f64
        }
    }
    
    /// Profit factor
    pub fn profit_factor(&self) -> f64 {
        let total_wins: f64 = self.closed_trades
            .iter()
            .filter(|t| t.is_win())
            .map(|t| t.realized_pnl)
            .sum();
        
        let total_losses: f64 = self.closed_trades
            .iter()
            .filter(|t| !t.is_win())
            .map(|t| -t.realized_pnl)
            .sum();
        
        if total_losses == 0.0 {
            if total_wins > 0.0 { f64::INFINITY } else { 0.0 }
        } else {
            total_wins / total_losses
        }
    }
    
    /// Max drawdown
    pub fn max_drawdown(&self) -> (f64, f64) {
        // Türkçe: Maksimum equity düşüşü
        let mut max_dd = 0.0;
        let mut max_dd_pct = 0.0;
        let mut peak = self.initial_capital;
        
        let mut equity = self.initial_capital;
        
        // History simulate et
        for trade in &self.closed_trades {
            equity += trade.realized_pnl;
            
            let dd = peak - equity;
            let dd_pct = if peak > 0.0 { (dd / peak) * 100.0 } else { 0.0 };
            
            if dd > max_dd {
                max_dd = dd;
                max_dd_pct = dd_pct;
            }
            
            if equity > peak {
                peak = equity;
            }
        }
        
        (max_dd, max_dd_pct)
    }
    
    /// Comprehensive metrics
    pub fn calculate_metrics(&self) -> PortfolioMetrics {
        let (max_dd, max_dd_pct) = self.max_drawdown();
        
        PortfolioMetrics {
            total_capital: self.initial_capital,
            available_cash: self.available_cash,
            open_positions_value: self.open_positions_value(),
            unrealized_pnl: self.unrealized_pnl(),
            realized_pnl: self.realized_pnl(),
            total_pnl: self.unrealized_pnl() + self.realized_pnl(),
            total_return_pct: if self.initial_capital > 0.0 {
                ((self.equity_value() - self.initial_capital) / self.initial_capital) * 100.0
            } else {
                0.0
            },
            open_positions_count: self.positions.len(),
            closed_trades_count: self.closed_trades.len(),
            win_rate: self.win_rate(),
            avg_win: self.avg_win(),
            avg_loss: self.avg_loss(),
            profit_factor: self.profit_factor(),
            max_drawdown: max_dd,
            max_drawdown_pct: max_dd_pct,
            sharpe_ratio: None, // Advanced metrics modülünde hesaplanacak
            sortino_ratio: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_open_and_close_position() {
        let mut pm = PortfolioManager::new(10000.0);
        
        // Pozisyon aç
        pm.open_position("BTCUSDT".into(), 45000.0, 0.1, 1.0, None, None).unwrap();
        assert_eq!(pm.positions_count(), 1);
        
        // Pozisyon kapat (+100 PnL)
        pm.close_position("BTCUSDT", 46000.0).unwrap();
        assert_eq!(pm.positions_count(), 0);
        assert!(pm.realized_pnl() > 0.0);
    }
    
    #[test]
    fn test_profit_factor() {
        let mut pm = PortfolioManager::new(10000.0);
        
        // Win
        pm.open_position("BTC".into(), 100.0, 1.0, 1.0, None, None).unwrap();
        pm.close_position("BTC", 110.0).unwrap();
        
        // Loss
        pm.open_position("ETH".into(), 2000.0, 0.1, 1.0, None, None).unwrap();
        pm.close_position("ETH", 1900.0).unwrap();
        
        let pf = pm.profit_factor();
        assert!(pf > 0.0);
    }
}
