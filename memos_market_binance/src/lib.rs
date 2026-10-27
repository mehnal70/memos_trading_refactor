//! Binance market vertical slice.

pub mod executor;
pub mod fetcher;
pub mod venue;

pub use executor::{BinanceFuturesExecutor, BinanceTradeExecutor};
pub use fetcher::{fetch_error_is_delisting, BinanceFetcher, DELISTING_ERR_PREFIX};
pub use venue::BinanceVenue;

use memos_venue_api::{Result, VenueAdapter, VenueFactory, VenueSpec};
use std::sync::Arc;

/// Constructs Binance venues without coupling the core crate to Binance internals.
#[derive(Clone, Default)]
pub struct BinanceVenueFactory {
    pub executor: Option<Arc<BinanceFuturesExecutor>>,
}

impl BinanceVenueFactory {
    pub fn new(executor: Option<Arc<BinanceFuturesExecutor>>) -> Self {
        Self { executor }
    }
}

impl VenueFactory for BinanceVenueFactory {
    fn build(&self, spec: VenueSpec) -> Result<Arc<dyn VenueAdapter>> {
        if spec.exchange != memos_venue_api::Exchange::Binance {
            return Err(format!(
                "Binance factory yalnızca binance venue kabul eder: {}",
                spec.token()
            )
            .into());
        }
        let venue = match &self.executor {
            Some(executor) => BinanceVenue::with_executor(spec.market, Arc::clone(executor)),
            None => BinanceVenue::data_only(spec.market),
        };
        Ok(Arc::new(venue))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memos_venue_api::{Exchange, Market};

    #[test]
    fn factory_builds_data_only_binance_by_default() {
        let factory = BinanceVenueFactory::default();
        let venue = factory
            .build(VenueSpec::new(Exchange::Binance, Market::Futures))
            .expect("binance venue");
        assert_eq!(venue.exchange(), Exchange::Binance);
        assert_eq!(venue.market(), Market::Futures);
        assert!(venue.execution_engine().is_none());
    }

    #[test]
    fn factory_rejects_other_exchanges() {
        match BinanceVenueFactory::default().build(VenueSpec::new(Exchange::Mt5, Market::Cfd)) {
            Ok(_) => panic!("wrong venue must be rejected"),
            Err(err) => assert!(err.to_string().contains("binance")),
        }
    }
}
