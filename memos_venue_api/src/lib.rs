//! `memos_venue_api` — piyasa/borsa katmanının CONTRACT crate'i.
//!
//! Hedef: her piyasa kendi crate'inde (dikey dilim) yaşarken ortak sözleşmeyi burada
//! paylaşsın. Bu crate KASTEN hafiftir (rusqlite/reqwest yok) — bkz. `error.rs`.
//!
//! Katman kuralı: bu crate hiçbir şeye bağımlı değildir. `memos_quant` (algoritmalar)
//! ve `memos_market_*` (dikey dilimler) buna bağımlıdır; tersi ASLA olmaz.

pub mod adapter;
pub mod contract;
pub mod error;
pub mod executor;
pub mod factory;
pub mod fallback;
pub mod order;
pub mod types;

pub use adapter::{ExchangeCapability, MarketData, OrderExecution, TradingProfile, VenueAdapter};
pub use contract::{
    CanonicalFill, CanonicalOrder, CanonicalPosition, CanonicalPositionKey, SnapshotQuality,
    VenueCapabilities, VenueKey, VenueReconcileResult,
};
pub use error::{Result, VenueError};
pub use executor::TradeExecutor;
pub use factory::VenueFactory;
pub use fallback::CandleFallback;
pub use order::{OrderKind, OrderReceipt, OrderRequest, OrderSide, OrderStatus};
pub use types::{
    bist_symbol_shape, AccountSnapshot, AccountSnapshotQuality, AccountSnapshotStatus, AssetClass,
    Candle, Exchange, FundingRatePoint, Market, Signal, SymbolFilters, Trade, VenueProfile,
    VenueSpec,
};
