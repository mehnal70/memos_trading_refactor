// robot/reporting.rs - Universal Reporter & Logging System (JSON, CSV, Table, Console)

use crate::robot::interfaces::Reporter;
use crate::types::{Trade, Signal};
use crate::Result;

pub struct UniversalReporter;

impl Reporter for UniversalReporter {
    fn report_trade(&self, _trade: &Trade) -> Result<()> {
        // TUI raw mode'da println! ekranı bozar.
        // Trade bilgisi SharedLogger → AppState.log üzerinden zaten akıyor.
        Ok(())
    }
    fn report_strategy(&self, _name: &str, _signal: &Signal) -> Result<()> {
        Ok(())
    }
    fn export_json(&self, _data: &serde_json::Value) -> Result<()> {
        Ok(())
    }
    fn export_csv(&self, _data: &str) -> Result<()> {
        Ok(())
    }
}

impl UniversalReporter {
    /// Tablo formatında trade listesi yazdır
    pub fn report_trades_table(&self, trades: &[Trade]) {
        println!("|   ID   |  Symbol  | Entry  | Exit   | Amount | PnL   | Strategy   |");
        for t in trades {
            println!("| {:6?} | {:8} | {:6.2} | {:6.2?} | {:6.2} | {:6.2?} | {:10} |",
                t.id, t.symbol, t.entry_price, t.exit_price, t.amount, t.pnl, t.strategy);
        }
    }
}
