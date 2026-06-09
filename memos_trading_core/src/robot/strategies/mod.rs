// robot/strategies/mod.rs - Tek source-of-truth strateji modülü
//
// Tüm stratejiler tek `Strategy` trait'i (base.rs) altında toplanır.
// Indicator hesaplamaları core::indicators'a delege edilir.

pub mod base;
pub mod keys;
pub mod param_spec;
pub mod utils;
pub mod standard;
pub mod volatility;
pub mod funding;
pub mod trend;
pub mod oscillator;
pub mod ensemble;
pub mod registry;
pub mod strategy_selector;

// Ortak sözleşme
pub use base::Strategy;

// Parametre uzayı (modüler optimizasyon temeli)
pub use param_spec::{ParamSpec, ParamKind, apply_param, build_params};

// Plug-in registry (Faz 4 c2)
pub use registry::{default_registry, StrategyFactory, StrategyRegistry};

// Yardımcılar (HTF filter, optimizer)
pub use utils::{htf_trend_filter, htf_periods, grid_search_optimization};

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
