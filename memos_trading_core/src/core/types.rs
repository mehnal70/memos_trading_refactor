// types.rs - Merkezi Veri Tipleri ve Tanımlamalar

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Her pozisyon için evrensel tekil kimlik (UUID tabanlı)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PositionId(uuid::Uuid);

impl PositionId {
    #[inline]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
    /// Var olan UUID'den inşa et — track_trade ↔ learn_from_exit eşlemesi için.
    #[inline]
    pub fn from_uuid(u: uuid::Uuid) -> Self {
        Self(u)
    }
    /// String'ten parse et; hata sessizce yeni bir UUID üretir (test/serde uyumu).
    #[inline]
    pub fn from_str_or_new(s: &str) -> Self {
        uuid::Uuid::parse_str(s)
            .map(Self)
            .unwrap_or_else(|_| Self::new())
    }
}

impl Default for PositionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PositionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        return write!(f, "{}", self.0);
    }
}

/// Risk parametreleri - Copy eklendi
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct RiskParams {
    pub stop_loss_pct: f64,
    pub take_profit_pct: f64,
    pub max_position_size_pct: Option<f64>,
    pub max_portfolio_risk_pct: Option<f64>,
    pub use_kelly_criterion: bool,
    pub trailing_stop_pct: Option<f64>,
}

// StrategyParams + ParamName memos_quant'a taşındı (Adım 3). Eski yol korunur:
// `crate::core::types::StrategyParams` -> `memos_quant::params::StrategyParams`.
pub use memos_quant::params::StrategyParams;

// =============================================================================
// VENUE CONTRACT'A TAŞINAN TİPLER — eski yollardan yeniden yayın
// =============================================================================
// Candle/Signal/Trade/FundingRatePoint/Exchange/AssetClass/VenueSpec/Market ve
// bist_symbol_shape artık `memos_venue_api::types` içinde yaşıyor (dikey-dilim
// refactor'ü, Adım 2). Bu `pub use` sayesinde `crate::core::types::Candle` gibi
// 400+ mevcut çağrı yeri AYNEN çalışmaya devam eder → davranış nötr.
pub use memos_venue_api::types::{
    bist_symbol_shape, AssetClass, Candle, Exchange, FundingRatePoint, Market, Signal, Trade,
    VenueProfile, VenueSpec,
};
