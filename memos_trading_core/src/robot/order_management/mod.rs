// Srivastava ATP Mimarisi - Order Management System (OMS)
// 
// Merkezi emir yönetim sistemi: Emirleri göndermek, takip etmek, 
// kısmi fill işlemek, retry ve slippage detection yapacak.
//
// Modüler tasarım: Her exchange için farklı implementation yapılabilir

pub mod types;
pub mod base;
pub mod binance;
pub mod validator;
pub use validator::{OrderValidator, ValidationRules};

pub mod paper_executor;
pub use paper_executor::PaperTradingExecutor;
pub mod orderbook_sim;
pub use orderbook_sim::{OrderBook, BookLevel, OrderBookSimulator, SyntheticBookConfig, FillResult, build_synthetic_book};
pub mod mock;

pub use types::*;
pub use base::*;
pub use binance::*;
pub use mock::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_oms_factory() {
        let _oms = OrderManagementSystem::binance("test-key", "test-secret");
        // OMS başarıyla oluşturulmalı
    }
}
