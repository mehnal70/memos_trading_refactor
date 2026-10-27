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

    /// Borsa bizi SİSTEMİK reddetti: bölge/legal bloğu (HTTP 451), WAF/Cloudflare (403),
    /// geçersiz key-IP-izin (-2015), IP auto-ban/rate-limit (-1003/429/418). Normal emir
    /// reddinden (bakiye, lot/notional, -4120 endpoint) AYRI — devre-kesici tetikleyicisi.
    #[error("Borsa Bloğu (bölge/IP/izin): {0}")]
    ApiBlocked(String),

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

impl MemosTradingError {
    /// Borsa bizi SİSTEMİK reddetti mi (bölge/IP/izin/IP-ban)? `true` → canlı emir
    /// denemesi cooldown boyunca durdurulmalı (API'yi dövme = IP-ban tırmanışı).
    /// Normal emir reddi (`Api`) `false` döner.
    pub fn is_exchange_block(&self) -> bool {
        matches!(self, Self::ApiBlocked(_))
    }
}

pub type Result<T> = std::result::Result<T, MemosTradingError>;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// --- 8. CONVERSIONS (Hata Tip Dönüşümleri) ---
// --- VENUE CONTRACT KÖPRÜSÜ (Adım 2) ---
// `memos_venue_api::VenueError` yalın tiptir (rusqlite/reqwest taşımaz) — piyasa
// crate'leri onu kullanır. Burada çekirdeğin zengin tipine köprülenir.
//
// ⚠️ KRİTİK: `ApiBlocked → ApiBlocked` eşlemesi CANLI DEVRE-KESİCİYİ ayakta tutar
// (`positions.rs` → `e.is_exchange_block()` → `trip_live_block`). Bu kolu değiştirmek
// borsa bloğunda emir dövmeye = IP-ban tırmanışına yol açar. Testle mühürlendi.
impl From<memos_venue_api::VenueError> for MemosTradingError {
    fn from(e: memos_venue_api::VenueError) -> Self {
        use memos_venue_api::VenueError as V;
        match e {
            V::Api(s) => Self::Api(s),
            V::ApiBlocked(s) => Self::ApiBlocked(s), // devre-kesici: KORUNMAK ZORUNDA
            V::Serde(e) => Self::Serde(e),
            V::Io(e) => Self::Io(e),
            V::Network(s) => Self::Api(s),
            V::Config(s) => Self::Config(s),
            V::Unsupported(s) => Self::Unknown(s),
            V::Unknown(s) => Self::Unknown(s),
        }
    }
}

#[cfg(test)]
mod venue_bridge_tests {
    use super::*;

    /// Köprü devre-kesici sınıflandırmasını korumalı: ApiBlocked geçişte kaybolursa
    /// canlı sistem borsa bloğunda emir denemeye devam eder (IP-ban riski).
    #[test]
    fn bridge_preserves_exchange_block_classification() {
        let v = memos_venue_api::VenueError::ApiBlocked("HTTP 451".into());
        assert!(v.is_exchange_block(), "kaynak taraf");
        let core: MemosTradingError = v.into();
        assert!(core.is_exchange_block(), "köprü sonrası KORUNMALI");
    }

    /// TERS köprü de sınıflandırmayı korumalı: Binance executor'ın ApiBlocked'ı
    /// venue sınırından geçerken devre-kesici bilgisini kaybetmemeli.
    #[test]
    fn reverse_bridge_preserves_exchange_block() {
        let core = MemosTradingError::ApiBlocked("HTTP 418 IP ban".into());
        let v: memos_venue_api::VenueError = core.into();
        assert!(v.is_exchange_block(), "ters köprü ApiBlocked'ı korumalı");
        let back: MemosTradingError = v.into();
        assert!(
            back.is_exchange_block(),
            "çift geçiş (roundtrip) da korumalı"
        );

        let ordinary: memos_venue_api::VenueError = MemosTradingError::Api("bakiye".into()).into();
        assert!(
            !ordinary.is_exchange_block(),
            "sıradan hata blok üretmemeli"
        );
    }

    #[test]
    fn bridge_does_not_invent_exchange_block() {
        for v in [
            memos_venue_api::VenueError::Api("bakiye".into()),
            memos_venue_api::VenueError::Network("timeout".into()),
            memos_venue_api::VenueError::Unsupported("veri-only".into()),
            memos_venue_api::VenueError::Config("eksik".into()),
        ] {
            let core: MemosTradingError = v.into();
            assert!(!core.is_exchange_block(), "yanlış pozitif devre-kesici");
        }
    }
}

/// TERS köprü: core hatasını venue sözleşmesi hatasına indir. Venue impl'leri
/// (BinanceFetcher vb. core-hatalı alt katmanları sararken) `?` ile kullanır.
/// ⚠️ `ApiBlocked → ApiBlocked` burada da KORUNUR — iki yönde de kaybolursa canlı
/// devre-kesici (trip_live_block) borsa bloğunu göremez.
impl From<MemosTradingError> for memos_venue_api::VenueError {
    fn from(e: MemosTradingError) -> Self {
        use memos_venue_api::VenueError as V;
        match e {
            MemosTradingError::ApiBlocked(s) => V::ApiBlocked(s), // devre-kesici
            MemosTradingError::Api(s) => V::Api(s),
            MemosTradingError::Serde(e) => V::Serde(e),
            MemosTradingError::Io(e) => V::Io(e),
            MemosTradingError::Reqwest(e) => V::Network(e.to_string()),
            MemosTradingError::Config(s) => V::Config(s),
            other => V::Unknown(other.to_string()),
        }
    }
}

impl From<String> for MemosTradingError {
    fn from(s: String) -> Self {
        Self::Unknown(s)
    }
}

impl From<&str> for MemosTradingError {
    fn from(s: &str) -> Self {
        Self::Unknown(s.to_string())
    }
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
    pub use crate::core::commands::RobotCommand;
    pub use crate::core::metrics::PerformanceScorecard;
    pub use crate::core::model::{
        ClosedTradeModel, MissionControl, Order, PositionModel, RoboticLoopConfig,
    };
    pub use crate::core::security;
    pub use crate::core::types::{Candle, Market, RiskParams, Signal, StrategyParams};
    pub use crate::robot::engines::Engine;
    pub use crate::robot::infra::monitoring::health_monitor;
    pub use crate::robot::ml_engine::MLModel;
    pub use crate::robot::robotic_loop::AdaptiveThresholds;
    pub use crate::robot::robotic_loop::{
        AppState, BrainBox, FinanceVault, FleetCommand, GuardianShield,
    };
    // Geriye uyumluluk ve hızlı erişim alias'ları
    pub mod database_reader {
        pub use crate::persistence::reader::*;
    }
    pub mod database_writer {
        pub use crate::persistence::writer::*;
    }

    // YENİ EKLEME: Hata yönetimi topyekün kütüphane içi araçlara sızıyor
    pub use crate::robot::logic::config_helpers::{
        PositionConfigBuilder, PositionManagementProfile, SecurityProfile,
    };
    pub use crate::{MemosTradingError, Result as AtpResult};
    // Tüm stratejiler robot::strategies altında tek source-of-truth ile organize.
    pub use crate::core::indicators::*;
    pub use crate::robot::strategies::{
        grid_search_optimization,
        // yardımcılar
        htf_trend_filter,
        // volatilite kanalları
        BollingerBandsStrategy,
        CciStrategy,
        DonchianChannelStrategy,
        // trend (EMA bazlı)
        EmaCrossoverStrategy,
        // funding rate (perpetual)
        FundingRateContrarianStrategy,
        IctCompositeStrategy,
        IctFvgStrategy,
        IctOrderBlockStrategy,
        MaCrossoverStrategy,
        MacdStrategy,
        PriceActionStrategy,
        // standart bank (trend + osilatör + price action + SMC ailesi)
        RsiStrategy,
        SmcStrategy,
        // osilatörler
        StochasticRsiStrategy,
        Strategy,
        // konsensüs motoru
        StrategyEnsemble,
        StrategyResult,
        SupertrendStrategy,
    };
}
