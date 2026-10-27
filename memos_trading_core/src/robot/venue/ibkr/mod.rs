//! IBKR venue katmanı — Interactive Brokers köprüsü için hazırlık iskeleti.

pub mod bridge {
    pub use memos_market_ibkr::{IbkrBridge, IBKR_DEFAULT_ADDR};
}

pub mod protocol {
    pub use memos_market_ibkr::protocol::*;
}

pub use memos_market_ibkr::IbkrVenue;
pub use memos_market_ibkr::{IbkrBridge, IBKR_DEFAULT_ADDR};
