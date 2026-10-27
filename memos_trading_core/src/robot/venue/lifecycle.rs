//! Canlı venue yaşam döngüsü: bakiye/hesap senkronu ve venue'ya özgü stream yeteneği.

use std::sync::Arc;

use crate::core::model::TradingMode;
use crate::robot::engines::binance_executor::BinanceFuturesExecutor;
use crate::robot::venue::adapter::VenueAdapter;
use memos_venue_api::Result;

/// Live lifecycle'ın başlatılabileceğini belirleyen güvenli seçim kapısı.
pub fn should_start_live_lifecycle(mode: &TradingMode, dry_run: bool, has_venue: bool) -> bool {
    matches!(mode, TradingMode::Live) && !dry_run && has_venue
}

/// Ortak hesap yaşam döngüsü. Emir gönderme içermez; yalnızca okuma/senkron yeteneklerini
/// venue adaptörüne bırakır. Binance user-data stream'i bilinçli olarak ayrı tutulur.
pub struct VenueLifecycleService {
    venue: Arc<dyn VenueAdapter>,
    binance_user_data: Option<Arc<BinanceFuturesExecutor>>,
}

impl VenueLifecycleService {
    pub fn new(
        venue: Arc<dyn VenueAdapter>,
        binance_user_data: Option<Arc<BinanceFuturesExecutor>>,
    ) -> Self {
        Self {
            venue,
            binance_user_data,
        }
    }

    pub fn venue(&self) -> &Arc<dyn VenueAdapter> {
        &self.venue
    }

    pub async fn balance(&self) -> Result<f64> {
        self.venue.balance().await
    }

    pub fn binance_user_data(&self) -> Option<Arc<BinanceFuturesExecutor>> {
        self.binance_user_data.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_and_dry_run_are_dormant() {
        assert!(!should_start_live_lifecycle(
            &TradingMode::Paper,
            false,
            true
        ));
        assert!(!should_start_live_lifecycle(&TradingMode::Live, true, true));
    }

    #[test]
    fn live_requires_a_selected_venue() {
        assert!(!should_start_live_lifecycle(
            &TradingMode::Live,
            false,
            false
        ));
        assert!(should_start_live_lifecycle(&TradingMode::Live, false, true));
    }
}
