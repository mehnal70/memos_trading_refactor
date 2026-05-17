// robot/strategies/mod.rs - Tek source-of-truth strateji modülü
//
// Tüm stratejiler tek `Strategy` trait'i (base.rs) altında toplanır.
// Indicator hesaplamaları core::indicators'a delege edilir.

pub mod base;
pub mod utils;
pub mod standard;
pub mod volatility;
pub mod funding;
pub mod trend;
pub mod oscillator;
pub mod ensemble;
pub mod strategy_selector;

// Ortak sözleşme
pub use base::Strategy;

// Yardımcılar (HTF filter, optimizer)
pub use utils::{htf_trend_filter, grid_search_optimization};

// Standart bank: trend + osilatör + price action + SMC ailesi
pub use standard::{
    RsiStrategy, MacdStrategy, SupertrendStrategy, PriceActionStrategy,
    IctFvgStrategy, SmcStrategy, IctOrderBlockStrategy, IctCompositeStrategy,
    MaCrossoverStrategy,
};

// Volatilite & kanal stratejileri
pub use volatility::{BollingerBandsStrategy, DonchianChannelStrategy};

// Funding rate kontrar (perpetual'lar için)
pub use funding::FundingRateContrarianStrategy;

// Trend (EMA bazlı)
pub use trend::EmaCrossoverStrategy;

// Osilatörler (Stochastic RSI, CCI)
pub use oscillator::{StochasticRsiStrategy, CciStrategy};

// Konsensüs / ensemble motoru — birden çok stratejiyi oy birliğiyle birleştirir
pub use ensemble::{StrategyEnsemble, StrategyResult};
