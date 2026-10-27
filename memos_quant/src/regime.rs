//! Piyasa rejimi kimliği — rejim-farkında her katmanın (strateji uyumu, kaldıraç,
//! edge hunisi, parametre override) tek-kaynağı.
//!
//! `memos_trading_core/src/evolution/adaptive_brain.rs`'ten BİREBİR taşındı; çekirdek
//! `evolution::MarketRegime` yolunu `pub use` ile korur → çağrı yerleri değişmedi.
//! `AdaptiveBrain` (öğrenme durumu) bilinçli olarak çekirdekte KALDI: o bir algoritma
//! değil, durum makinesidir.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MarketRegime {
    StrongUptrend,
    WeakUptrend,
    Ranging,
    WeakDowntrend,
    StrongDowntrend,
    HighVolatility,
    LowVolatility,
    Unknown,
}

impl MarketRegime {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::StrongUptrend => "StrongUptrend",
            Self::WeakUptrend => "WeakUptrend",
            Self::Ranging => "Ranging",
            Self::WeakDowntrend => "WeakDowntrend",
            Self::StrongDowntrend => "StrongDowntrend",
            Self::HighVolatility => "HighVolatility",
            Self::LowVolatility => "LowVolatility",
            Self::Unknown => "Unknown",
        }
    }
}
