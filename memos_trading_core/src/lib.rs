// lib.rs - Memos Trading Core Library (Srivastava ATP Nihai Sürüm)
// Merkezi modül yönetimi, agnostik veri hatları ve adli hata kontrol katmanı.

// --- 1. GLOBAL ATTRIBUTES ---
#![allow(dead_code, unused_imports)]

// --- 2. CORE & MATH (Agnostik Zeka Birimi) ---
// Bu modüller hiçbir UI bağımlılığı taşımaz, Android/Web için doğrudan taşınabilirdir.
pub mod core;
pub mod evolution;
// handlers/ TUI binary'sine taşındı (interfaces/rtc_tui/src/handlers/).

// --- 3. PERSISTENCE (Adli Hafıza Katmanı) ---
#[cfg(not(target_arch = "wasm32"))]
pub mod persistence;

// --- 4. ROBOTİK & ENGINE (Harekât Merkezi) ---
pub mod robot;

// --- 6. HATA YÖNETİMİ (Adli Süzgeç) ---
#[derive(Debug, thiserror::Error)]
pub enum MemosTradingError {
    #[error("Veritabanı Hatası: {0}")]
    Database(String),

    #[error("Rusqlite Hatası: {0}")]
    Rusqlite(#[from] rusqlite::Error),

    #[error("API Bağlantı Hatası: {0}")]
    Api(String),

    #[error("Veri Çözümleme (Serde) Hatası: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("Giriş/Çıkış (IO) Hatası: {0}")]
    Io(#[from] std::io::Error),

    #[error("Ağ (Reqwest) Hatası: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("Konfigürasyon Hatası: {0}")]
    Config(String),

    #[error("Strateji/Backtest Hatası: {0}")]
    Strategy(String),

    #[error("Risk Yönetimi Hatası: {0}")]
    Risk(String),

    #[error("Bilinmeyen Adli Hata: {0}")]
    Unknown(String),
}

pub type Result<T> = std::result::Result<T, MemosTradingError>;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// --- 8. CONVERSIONS (Hata Tip Dönüşümleri) ---
impl From<String> for MemosTradingError {
    fn from(s: String) -> Self { Self::Unknown(s) }
}

impl From<&str> for MemosTradingError {
    fn from(s: &str) -> Self { Self::Unknown(s.to_string()) }
}

impl From<crate::persistence::memory::DatabaseError> for MemosTradingError {
    fn from(e: crate::persistence::memory::DatabaseError) -> Self {
        Self::Database(e.to_string())
    }
}

// =============================================================================
// 9. EVRENSEL REAKTİF PRELUDE (Zeki Adresleme Odası)
// =============================================================================
/// 🧬 Srivastava ATP - İç Giriş Köprüsü (Prelude)
/// Yalnızca kütüphane içi (crate) görünürlüğe sahiptir. Şişmeyi derleme zamanında engeller.
/// Harekât hattındaki ağır dosyalar (engine, loop vb.) en üstte 'use crate::prelude::*;' ile her şeye erişir.
pub(crate) mod prelude {
    pub use crate::core::model::{MissionControl, PositionModel, Order, ClosedTradeModel, RoboticLoopConfig};
    pub use crate::core::metrics::PerformanceScorecard;
    pub use crate::core::types::{Candle, Signal, StrategyParams, Market, RiskParams};
    pub use crate::robot::robotic_loop::{AppState, BrainBox, FinanceVault, FleetCommand, GuardianShield};
    pub use crate::robot::engines::Engine;
    pub use crate::robot::ml_engine::MLModel;
    pub use crate::robot::infra::monitoring::health_monitor;
    pub use crate::core::security;
    pub use crate::core::commands::RobotCommand;
    pub use crate::robot::robotic_loop::AdaptiveThresholds;
    // Geriye uyumluluk ve hızlı erişim alias'ları
    pub mod database_reader { pub use crate::persistence::reader::*; }
    pub mod database_writer { pub use crate::persistence::writer::*; }

     // YENİ EKLEME: Hata yönetimi topyekün kütüphane içi araçlara sızıyor
    pub use crate::{MemosTradingError, Result as AtpResult}; 
    pub use crate::robot::logic::config_helpers::{PositionManagementProfile, SecurityProfile, PositionConfigBuilder};
    // Tüm stratejiler robot::strategies altında tek source-of-truth ile organize.
    pub use crate::robot::strategies::{
        Strategy,
        // standart bank (trend + osilatör + price action + SMC ailesi)
        RsiStrategy, MacdStrategy, SupertrendStrategy, PriceActionStrategy,
        IctFvgStrategy, SmcStrategy, IctOrderBlockStrategy, IctCompositeStrategy,
        MaCrossoverStrategy,
        // volatilite kanalları
        BollingerBandsStrategy, DonchianChannelStrategy,
        // funding rate (perpetual)
        FundingRateContrarianStrategy,
        // trend (EMA bazlı)
        EmaCrossoverStrategy,
        // osilatörler
        StochasticRsiStrategy, CciStrategy,
        // konsensüs motoru
        StrategyEnsemble, StrategyResult,
        // yardımcılar
        htf_trend_filter, grid_search_optimization,
    };
    pub use crate::core::indicators::*;


}
