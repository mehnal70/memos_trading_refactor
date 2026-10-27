//! `TradeExecutor` — legacy yürütme arayüzü (sinyal → piyasa emri).
//!
//! `memos_trading_core/src/robot/infra/interfaces.rs`'ten taşındı: `Mt5Venue` ve
//! diğer venue adaptörleri `execution_engine()` üzerinden bunu döndürür; piyasa
//! crate'lerinin core'a bağımlı olmaması için sözleşmenin parçası olmak zorunda.
//! Hata tipi `VenueError`'a çevrildi (core `From<VenueError>` köprüsüyle tüketir).
use crate::error::Result;
use crate::types::{Signal, Trade};

/// TradeExecutor: Otomatik trade işlemlerini yönetir
pub trait TradeExecutor: Send + Sync {
    fn execute(&self, signal: Signal, symbol: &str, amount: f64) -> Result<Trade>;
    fn cancel_all(&self, symbol: &str) -> Result<()>;
    /// Exchange'deki gerçek açık pozisyon sembollerini döndürür.
    /// Paper mod veya desteklenmeyen executor'larda boş vec döner.
    fn fetch_open_symbols(&self) -> Vec<String> {
        return vec![];
    }
}
