pub mod finnhub;
pub use finnhub::FinnhubFetcher;
// robot/data_fetcher/mod.rs - Data fetcher modules

pub mod websocket;
pub use websocket::{BinanceKlineUpdate, BinanceKline, parse_kline, validate_ohlcv};

pub mod binance;
pub use binance::BinanceFetcher;

pub mod market_fetcher;
pub use market_fetcher::MarketFetcher;

pub mod bist_fetcher;
pub use bist_fetcher::BistFetcher;

// NOT: hybrid (HybridBinanceFetcher) + live_adapter (BinanceLiveAdapter) kaldırıldı
// (çoklu-piyasa Faz 0-C): ölü LiveDataFetcher kümesi, bozuk URL'ler. Canlı veri venue::VenueAdapter
// (MarketData) üzerinden; WS feed deseni data_pipeline::price_feed'de korunuyor.
