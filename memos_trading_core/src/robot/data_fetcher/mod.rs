pub mod finnhub;
pub use finnhub::FinnhubFetcher;
// robot/data_fetcher/mod.rs - Data fetcher modules

pub mod websocket;
pub use websocket::{BinanceKlineUpdate, BinanceKline, parse_kline, validate_ohlcv};

pub mod binance;
pub use binance::BinanceFetcher;

pub mod hybrid;
pub use hybrid::{HybridBinanceFetcher, FetchMode};

pub mod market_fetcher;
pub use market_fetcher::MarketFetcher;

pub mod bist_fetcher;
pub use bist_fetcher::BistFetcher;

pub mod live_adapter;
pub use live_adapter::BinanceLiveAdapter;
