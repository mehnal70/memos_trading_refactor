//! Venue sözleşmesi — memos_venue_api'ye taşındı (Adım 5 önkoşulu).
//!
//! `MarketData`/`OrderExecution`/`VenueAdapter`/`ExchangeCapability`/`TradingProfile`
//! artık contract crate'inde yaşıyor: piyasa crate'leri (memos_market_mt5 vb.) core'a
//! bağımlı olmadan implement edebilsin. Eski yollar bu kabukla AYNEN çalışır.
pub use memos_venue_api::adapter::*;
