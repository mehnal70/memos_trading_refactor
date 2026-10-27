// robot/strategies/mod.rs - Tek source-of-truth strateji modülü
//
// Tüm stratejiler tek `Strategy` trait'i (base.rs) altında toplanır.
// Indicator hesaplamaları core::indicators'a delege edilir.

pub mod base;
pub mod ensemble;
pub mod funding;
pub mod keys;
pub mod oscillator;
pub mod param_spec;
pub mod registry;
pub mod standard;
pub mod strategy_selector;
pub mod trend;
pub mod utils;
pub mod volatility;

// Ortak sözleşme
pub use base::Strategy;

// Parametre uzayı (modüler optimizasyon temeli)
pub use param_spec::{apply_param, build_params, ParamKind, ParamSpec};

// Plug-in registry (Faz 4 c2)
pub use registry::{default_registry, StrategyFactory, StrategyRegistry};

// Yardımcılar (HTF filter, optimizer)
pub use utils::{grid_search_optimization, htf_periods, htf_trend_filter};

// Standart bank: trend + osilatör + price action + SMC ailesi
pub use standard::{
    IctCompositeStrategy, IctFvgStrategy, IctOrderBlockStrategy, MaCrossoverStrategy, MacdStrategy,
    PriceActionStrategy, RsiStrategy, SmcStrategy, SupertrendStrategy,
};

// Volatilite & kanal stratejileri
pub use volatility::{BollingerBandsStrategy, DonchianChannelStrategy};

// Funding rate kontrar (perpetual'lar için)
pub use funding::FundingRateContrarianStrategy;

// Trend (EMA bazlı)
pub use trend::EmaCrossoverStrategy;

// Osilatörler (Stochastic RSI, CCI)
pub use oscillator::{CciStrategy, StochasticRsiStrategy};

// Konsensüs / ensemble motoru — birden çok stratejiyi oy birliğiyle birleştirir
pub use ensemble::{StrategyEnsemble, StrategyResult};
