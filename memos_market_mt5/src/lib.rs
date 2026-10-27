//! MetaTrader 5 venue implementation.

pub mod bridge;
pub mod executor;
pub mod protocol;
pub mod venue;

pub use bridge::{
    normalize_bridge_addr, normalize_executor_addr, Mt5Bridge, MT5_DEFAULT_ADDR,
    MT5_DEFAULT_EXECUTOR_ADDR,
};
pub use executor::Mt5TradeExecutor;
pub use venue::Mt5Venue;

use memos_venue_api::{Result, VenueAdapter, VenueFactory, VenueSpec};
use std::sync::Arc;

/// Factory for constructing MT5 venues without coupling the core crate to MT5 internals.
pub struct Mt5VenueFactory;

impl Default for Mt5VenueFactory {
    fn default() -> Self {
        Self
    }
}

impl VenueFactory for Mt5VenueFactory {
    fn build(&self, spec: VenueSpec) -> Result<Arc<dyn VenueAdapter>> {
        if spec.exchange != memos_venue_api::Exchange::Mt5 {
            return Err(format!(
                "MT5 factory yalnızca mt5 venue kabul eder: {}",
                spec.token()
            )
            .into());
        }
        let bridge_addr =
            std::env::var("MT5_BRIDGE_ADDR").unwrap_or_else(|_| MT5_DEFAULT_ADDR.to_string());
        let bridge = Arc::new(Mt5Bridge::with_defaults(Some(bridge_addr)));
        let executor_addr = std::env::var("MT5_EXECUTOR_ADDR")
            .unwrap_or_else(|_| MT5_DEFAULT_EXECUTOR_ADDR.to_string());
        Ok(Arc::new(Mt5Venue::with_executor(
            spec.market,
            bridge,
            executor_addr,
        )))
    }
}
