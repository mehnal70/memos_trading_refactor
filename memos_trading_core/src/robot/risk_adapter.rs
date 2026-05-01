// robot/risk.rs - RiskAnalyzer örnek şablon

use crate::robot::interfaces::RiskAnalyzer;
use crate::types::Trade;
use crate::Result;

pub struct SimpleRiskAnalyzer;

impl RiskAnalyzer for SimpleRiskAnalyzer {
    fn analyze(&self, trade: &Trade) -> Result<f64> {
        // Basit risk: realized PnL (örnek)
        Ok(trade.pnl.unwrap_or(0.0))
    }
    fn max_position_size(&self, capital: f64, price: f64) -> Result<f64> {
        // Basit pozisyon boyutu: %10 sermaye
        Ok((capital * 0.10) / price)
    }
}
