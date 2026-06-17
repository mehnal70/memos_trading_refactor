//! `VenueRegistry` — aktif venue'ların kayıt defteri + sembol→venue yönlendirme.
//!
//! Profiller hangi venue'ların aktif olduğunu seçer; motor sembol başına doğru adaptörü
//! buradan ister (`Exchange::classify` tek-kaynak). Kayıtlı olmayan sembol varsayılan
//! borsaya düşer (geriye-uyum: tek-Binance kurulumunda her şey Binance'e gider).

use std::collections::HashMap;
use std::sync::Arc;

use crate::core::types::Exchange;
use crate::robot::venue::adapter::VenueAdapter;

pub struct VenueRegistry {
    venues: HashMap<Exchange, Arc<dyn VenueAdapter>>,
    default_exchange: Exchange,
}

impl VenueRegistry {
    /// Sembolü hiçbir kayıtlı venue karşılamazsa düşülecek varsayılan borsa ile kur.
    pub fn new(default_exchange: Exchange) -> Self {
        Self { venues: HashMap::new(), default_exchange }
    }

    /// Bir venue'yu kaydet (borsa-anahtarlı; aynı borsa tekrar kaydedilirse üzerine yazar).
    pub fn register(&mut self, venue: Arc<dyn VenueAdapter>) -> &mut Self {
        self.venues.insert(venue.exchange(), venue);
        self
    }

    /// Borsaya göre venue (kayıtlı değilse None).
    pub fn get(&self, exchange: Exchange) -> Option<&Arc<dyn VenueAdapter>> {
        self.venues.get(&exchange)
    }

    /// Sembolü venue'suna yönlendir: `Exchange::classify(symbol)` → kayıtlıysa o, değilse
    /// varsayılan borsa. Hiçbiri yoksa None (boş registry).
    pub fn for_symbol(&self, symbol: &str) -> Option<&Arc<dyn VenueAdapter>> {
        let ex = Exchange::classify(symbol);
        self.venues.get(&ex).or_else(|| self.venues.get(&self.default_exchange))
    }

    /// Kayıtlı borsalar.
    pub fn exchanges(&self) -> impl Iterator<Item = Exchange> + '_ {
        self.venues.keys().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.venues.is_empty()
    }

    pub fn len(&self) -> usize {
        self.venues.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::Market;
    use crate::robot::engines::binance_executor::BinanceFuturesExecutor;
    use crate::robot::venue::binance::BinanceVenue;

    fn binance_venue() -> Arc<dyn VenueAdapter> {
        let exec = Arc::new(BinanceFuturesExecutor::new_for_market(
            String::new(),
            String::new(),
            true,
            "futures",
        ));
        Arc::new(BinanceVenue::with_executor(Market::Futures, exec))
    }

    #[test]
    fn routes_crypto_symbol_to_binance() {
        let mut reg = VenueRegistry::new(Exchange::Binance);
        reg.register(binance_venue());
        assert_eq!(reg.len(), 1);
        let v = reg.for_symbol("BTCUSDT").expect("kripto sembolü Binance'e gitmeli");
        assert_eq!(v.exchange(), Exchange::Binance);
    }

    #[test]
    fn unregistered_symbol_falls_back_to_default() {
        let mut reg = VenueRegistry::new(Exchange::Binance);
        reg.register(binance_venue());
        // "THYAO" BIST olarak sınıflanır → kayıtlı değil → varsayılan (Binance) venue'ya düşer.
        let v = reg.for_symbol("THYAO").expect("kayıtsız sembol varsayılana düşmeli");
        assert_eq!(v.exchange(), Exchange::Binance);
    }

    #[test]
    fn empty_registry_returns_none() {
        let reg = VenueRegistry::new(Exchange::Binance);
        assert!(reg.is_empty());
        assert!(reg.for_symbol("BTCUSDT").is_none());
    }
}
