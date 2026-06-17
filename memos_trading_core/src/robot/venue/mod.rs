//! Venue katmanı — borsa/piyasa soyutlamasının tek çatısı (çoklu-piyasa Faz 0).
//!
//! Hedef: yeni bir borsa/varlık-sınıfı (başka kripto borsası, BIST, emtia, forex) eklemek =
//! [`VenueAdapter`]'ın bir implementasyonu + `Exchange`/`AssetClass`'a bir kol. Otonom çekirdek
//! (rejim/screener/edge/risk/icra) `Candle` + sembol üzerinden çalıştığı için dokunulmaz.
//!
//! * [`VenueAdapter`] — kimlik (exchange/market/asset_class) + veri + yürütme tek arayüzü.
//! * [`MarketData`] / [`OrderExecution`] — ayrılabilir yetenekler (veri-only venue mümkün).
//! * [`BinanceVenue`] — mevcut `BinanceFetcher`+`BinanceFuturesExecutor`'ı saran ilk impl.
//! * [`VenueRegistry`] — aktif venue'lar + sembol→venue yönlendirme.
//!
//! Faz 0 kapsamı: temel + Binance adaptörü + testler (canlı cycle wiring = Faz 1).

pub mod adapter;
pub mod binance;
pub mod bybit;
pub mod registry;
pub mod types;

pub use adapter::{MarketData, OrderExecution, VenueAdapter};
pub use binance::BinanceVenue;
pub use bybit::BybitVenue;
pub use registry::VenueRegistry;
pub use types::{OrderKind, OrderReceipt, OrderRequest, OrderSide, OrderStatus};
