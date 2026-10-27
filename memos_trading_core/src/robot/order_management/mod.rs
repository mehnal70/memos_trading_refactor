// Srivastava ATP Mimarisi - Order Management System (OMS)
//
// Merkezi emir yönetim sistemi: Emirleri göndermek, takip etmek,
// kısmi fill işlemek, retry ve slippage detection yapacak.
//
// Modüler tasarım: Her exchange için farklı implementation yapılabilir

pub mod base;
pub mod binance;
pub mod types;
pub mod validator;
pub use validator::{OrderValidator, ValidationRules};

pub mod paper_executor;
pub use paper_executor::PaperTradingExecutor;
pub mod orderbook_sim;
pub use orderbook_sim::{
    build_synthetic_book, BookLevel, FillResult, OrderBook, OrderBookSimulator, SyntheticBookConfig,
};
pub mod mock;

pub use base::*;
pub use binance::*;
pub use mock::*;
pub use types::*;
