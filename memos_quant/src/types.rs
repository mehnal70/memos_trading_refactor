//! Quant katmanının tip cephesi: contract tipleri + strateji parametreleri tek yerden.
//!
//! Taşınan algoritma dosyaları eskiden `crate::core::types::*` kullanıyordu; bu cephe
//! sayesinde tek bir yol değişimiyle (`core::types` → `crate::types`) çalışır kaldılar.
pub use crate::params::StrategyParams;
pub use memos_venue_api::types::{
    AssetClass, Candle, Exchange, FundingRatePoint, Market, Signal, SymbolFilters, Trade, VenueSpec,
};
