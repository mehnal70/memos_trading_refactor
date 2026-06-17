//! `VenueRegistry` — aktif venue'ların kayıt defteri + sembol→venue yönlendirme.
//!
//! Profiller hangi venue'ların aktif olduğunu seçer; motor sembol başına doğru adaptörü
//! buradan ister (`Exchange::classify` tek-kaynak). Kayıtlı olmayan sembol varsayılan
//! borsaya düşer (geriye-uyum: tek-Binance kurulumunda her şey Binance'e gider).

use std::collections::HashMap;
use std::sync::Arc;

use crate::core::types::{Exchange, VenueSpec};
use crate::robot::engines::binance_executor::BinanceFuturesExecutor;
use crate::robot::venue::adapter::VenueAdapter;
use crate::robot::venue::binance::BinanceVenue;
use crate::robot::venue::bybit::BybitVenue;

pub struct VenueRegistry {
    venues: HashMap<Exchange, Arc<dyn VenueAdapter>>,
    default_exchange: Exchange,
}

impl VenueRegistry {
    /// Sembolü hiçbir kayıtlı venue karşılamazsa düşülecek varsayılan borsa ile kur.
    pub fn new(default_exchange: Exchange) -> Self {
        Self { venues: HashMap::new(), default_exchange }
    }

    /// Config venue-spec'lerinden registry kur (operatör seçimi → çalışan registry).
    /// Bilinen borsa (şu an yalnız Binance) için adaptör kurulur; henüz desteklenmeyen borsa
    /// loglanıp atlanır (Faz 1+ eklendikçe açılır). `binance_executor` verilirse Binance venue
    /// auth'lu (veri+yürütme), yoksa data-only (yalnız public veri). default_exchange = ilk spec.
    ///
    /// NOT: registry borsa-anahtarlı → aynı borsanın birden çok market'i (binance:spot +
    /// binance:futures) verilirse SON spec'in market'i kazanır (tek venue/borsa). Çoklu-market/
    /// borsa anahtarı Faz 1+ işi.
    pub fn from_specs(specs: &[VenueSpec], binance_executor: Option<Arc<BinanceFuturesExecutor>>) -> Self {
        let default_ex = specs.first().map(|s| s.exchange).unwrap_or(Exchange::Binance);
        let mut reg = Self::new(default_ex);
        for spec in specs {
            match spec.exchange {
                Exchange::Binance => {
                    let venue = match binance_executor.clone() {
                        Some(exec) => BinanceVenue::with_executor(spec.market, exec),
                        None => BinanceVenue::data_only(spec.market),
                    };
                    reg.register(Arc::new(venue));
                }
                Exchange::Bybit => {
                    // Bybit veri venue'su (auth gerekmez; yürütme Faz 1+ → açık hata).
                    reg.register(Arc::new(BybitVenue::new(spec.market)));
                }
                _ => {
                    log::warn!(
                        target: "VENUE",
                        "venue '{}' henüz desteklenmiyor — atlandı (Faz 1+ adaptörü eklenecek)",
                        spec.token(),
                    );
                }
            }
        }
        reg
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

    #[test]
    fn venue_spec_token_roundtrip() {
        let s = VenueSpec::parse_token("binance:futures").unwrap();
        assert_eq!(s.exchange, Exchange::Binance);
        assert_eq!(s.market, Market::Futures);
        assert_eq!(s.token(), "binance:futures");
        // Market'siz token → Spot; bilinmeyen borsa → None.
        assert_eq!(VenueSpec::parse_token("binance").unwrap().market, Market::Spot);
        assert!(VenueSpec::parse_token("nasdaq:spot").is_none());
        assert!(VenueSpec::parse_token("  ").is_none());
    }

    #[test]
    fn from_specs_builds_known_skips_unsupported() {
        // binance kurulur; coinbase (henüz adaptörsüz) atlanır. default = ilk spec (binance).
        let specs = vec![
            VenueSpec::new(Exchange::Binance, Market::Futures),
            VenueSpec::new(Exchange::Coinbase, Market::Spot),
        ];
        let reg = VenueRegistry::from_specs(&specs, None);
        assert_eq!(reg.len(), 1, "yalnız Binance kurulmalı");
        let v = reg.for_symbol("BTCUSDT").expect("Binance venue var");
        assert_eq!(v.exchange(), Exchange::Binance);
        assert_eq!(v.market(), Market::Futures);
    }

    #[test]
    fn from_specs_builds_binance_and_bybit() {
        // Bybit gerçek adaptör → kurulur. İkisi de kayıtlı; sembol-şekli aynı (BTCUSDT) olduğu
        // için for_symbol default'a (ilk spec=Binance) düşer — Bybit explicit get ile erişilir.
        let specs = vec![
            VenueSpec::new(Exchange::Binance, Market::Futures),
            VenueSpec::new(Exchange::Bybit, Market::Futures),
        ];
        let reg = VenueRegistry::from_specs(&specs, None);
        assert_eq!(reg.len(), 2, "Binance + Bybit kurulmalı");
        assert_eq!(reg.get(Exchange::Bybit).expect("bybit var").exchange(), Exchange::Bybit);
        // for_symbol kripto sembolünü default borsaya (Binance) yönlendirir (şekil ayırt etmez).
        assert_eq!(reg.for_symbol("BTCUSDT").unwrap().exchange(), Exchange::Binance);
    }
}
