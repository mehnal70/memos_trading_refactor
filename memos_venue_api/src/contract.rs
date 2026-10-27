//! Venue'ler arası kanonik durum sözleşmesi.
//!
//! Adapter'lar kendi API cevaplarını bu tiplere çevirir; core katmanı venue'ya
//! özgü JSON alanlarını bilmez.

use serde::{Deserialize, Serialize};

use crate::types::{Exchange, Market, VenueProfile};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VenueKey {
    pub exchange: Exchange,
    pub market: Market,
    pub profile: VenueProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CanonicalPositionKey {
    pub venue: VenueKey,
    pub symbol: String,
}

impl CanonicalPositionKey {
    pub fn new(venue: VenueKey, symbol: impl Into<String>) -> Self {
        Self {
            venue,
            symbol: symbol.into(),
        }
    }

    pub fn token(&self) -> String {
        format!("{}:{}", self.venue.token(), self.symbol)
    }
}

impl VenueKey {
    pub const fn new(exchange: Exchange, market: Market, profile: VenueProfile) -> Self {
        Self {
            exchange,
            market,
            profile,
        }
    }

    pub fn token(&self) -> String {
        crate::types::VenueSpec {
            exchange: self.exchange,
            market: self.market,
            profile: self.profile,
        }
        .token()
    }
}

impl From<crate::types::VenueSpec> for VenueKey {
    fn from(spec: crate::types::VenueSpec) -> Self {
        Self::new(spec.exchange, spec.market, spec.profile)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalPosition {
    pub venue: VenueKey,
    pub symbol: String,
    pub quantity: f64,
    pub average_price: Option<f64>,
    pub mark_price: Option<f64>,
    pub unrealized_pnl: Option<f64>,
    pub broker_reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalOrder {
    pub venue: VenueKey,
    pub symbol: String,
    pub broker_order_id: Option<String>,
    pub status: String,
    pub side: String,
    pub quantity: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalFill {
    pub venue: VenueKey,
    pub symbol: String,
    pub broker_fill_id: Option<String>,
    pub broker_order_id: Option<String>,
    pub quantity: f64,
    pub price: f64,
    pub realized_pnl: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotQuality {
    Fresh,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VenueReconcileResult {
    pub venue: VenueKey,
    pub quality: SnapshotQuality,
    pub positions: Vec<CanonicalPosition>,
    pub orders: Vec<CanonicalOrder>,
    pub fills: Vec<CanonicalFill>,
    pub error: Option<String>,
}

impl VenueReconcileResult {
    pub fn unavailable(venue: VenueKey, error: impl Into<String>) -> Self {
        Self {
            venue,
            quality: SnapshotQuality::Unavailable,
            positions: Vec::new(),
            orders: Vec::new(),
            fills: Vec::new(),
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VenueCapabilities {
    pub market_data: bool,
    pub order_execution: bool,
    pub open_positions: bool,
    pub open_orders: bool,
    pub closed_fills: bool,
    pub native_protection_orders: bool,
    pub supports_short: bool,
}

impl VenueCapabilities {
    pub const fn data_only() -> Self {
        Self {
            market_data: true,
            order_execution: false,
            open_positions: false,
            open_orders: false,
            closed_fills: false,
            native_protection_orders: false,
            supports_short: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CanonicalPositionKey, VenueKey};
    use crate::types::{Exchange, Market, VenueProfile, VenueSpec};

    #[test]
    fn venue_key_round_trips_without_symbol_collisions() {
        let key = VenueKey::from(VenueSpec::with_profile(
            Exchange::Ibkr,
            Market::Cfd,
            VenueProfile::Paper,
        ));
        assert_eq!(key.token(), "ibkr:cfd:paper");
        assert_ne!(
            key,
            VenueKey::new(Exchange::Mt5, Market::Cfd, VenueProfile::Paper)
        );
        assert_eq!(
            CanonicalPositionKey::new(key, "XLE").token(),
            "ibkr:cfd:paper:XLE"
        );
    }
}
