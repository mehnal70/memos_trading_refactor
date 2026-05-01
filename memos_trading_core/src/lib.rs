pub mod candle_synth;
#[allow(dead_code, unused_imports, unused_variables, unused_mut, non_snake_case, non_camel_case_types, non_upper_case_globals, clippy::all)]
pub mod auto_test_and_logging;
pub mod pipeline_supervisor;
pub mod symbol_watch_manager;
#[allow(dead_code, unused_imports, unused_variables, unused_mut, non_snake_case, non_camel_case_types, non_upper_case_globals, clippy::all)]
// Yukarıdaki attribute ile tüm uyarılar (dead_code, unused, vs.) crate genelinde bastırılır.
pub mod secure_store;
pub mod bybit_connector;
pub mod kucoin_connector;
pub mod coinbase_connector;
pub mod binance_connector;
pub mod exchange_connector;
impl From<String> for MemosTradingError {
    fn from(s: String) -> Self {
        MemosTradingError::Unknown(s)
    }
}
/// Audit trail modülü (tüm ortamlarda aktif)
pub mod audit_trail;
/// ML tabanlı anomali tespit modülü
pub mod ml_anomaly;
/// GDPR ve veri gizliliği modülü
pub mod gdpr;

// ── Enterprise modülleri (varsayılan olarak kapalı) ─────────────────────────
// Etkinleştirmek için: cargo build --features enterprise
/// RBAC ve SSO/LDAP modülleri
#[cfg(feature = "enterprise")]
pub mod rbac;
#[cfg(feature = "enterprise")]
pub mod sso_ldap;
/// SLA ve uptime modülü
#[cfg(feature = "enterprise")]
pub mod sla;
/// Prometheus metrics modülü
#[cfg(feature = "enterprise")]
pub mod metrics;
/// Mobil push notification modülü
#[cfg(feature = "enterprise")]
pub mod fcm_push;
/// SIEM entegrasyonu
#[cfg(feature = "enterprise")]
pub mod siem_forwarder;
/// HSM (Donanım Güvenlik Modülü) entegrasyonu — pkcs11 feature gerektirir
#[cfg(feature = "enterprise")]
pub mod hsm;
pub mod batch_config;
pub mod auto_trading_engine;
pub mod market_regime;
pub mod strategy_lifecycle;
pub mod risk_limits;
pub mod anomaly_analysis;
pub mod health_dashboard;
pub mod sim_data;
pub mod bist;
// Error conversion: From<&str> and From<reqwest::Error> for MemosTradingError
impl From<&str> for MemosTradingError {
    fn from(s: &str) -> Self {
        MemosTradingError::Unknown(s.to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod exchanges;
impl From<reqwest::Error> for MemosTradingError {
    fn from(err: reqwest::Error) -> Self {
        MemosTradingError::Api(err.to_string())
    }
}

/// Memos Trading Core Library
/// 
/// Bu kütüphane, ticaret stratejileri, risk yönetimi, veri analizi
/// ve backtesting fonksiyonlarını sağlayan modüler bir yapı sunmaktadır.
///
/// MFA (Çok Faktörlü Kimlik Doğrulama) modülü entegre edilmiştir.
pub mod mfa;

pub mod config;
pub mod types;

#[cfg(not(target_arch = "wasm32"))]
pub mod database;
#[cfg(not(target_arch = "wasm32"))]
pub mod database_reader;
#[cfg(not(target_arch = "wasm32"))]
pub mod database_writer;
pub mod strategies;
#[cfg(not(target_arch = "wasm32"))]
pub mod risk;
#[cfg(not(target_arch = "wasm32"))]
pub mod engine;
#[cfg(not(target_arch = "wasm32"))]
pub mod api;
pub mod indicators;
#[cfg(not(target_arch = "wasm32"))]
pub mod portfolio;
pub mod advanced;

// YENİ: Robotik Trading Sistemi (v2.0)
pub mod robot;

// YENİ: Evrimsel AI - Self-Evolving Trading System
pub mod evolution;

// Otomatik Sağlık ve Anomali İzleme Modülü
pub mod health_monitor;

// Re-export ana tipler
pub use config::{Config, TradingMode};
pub use robot::{
    data_pipeline::DataPipeline,
    calculations::CalculationEngine,
    config::{RobotConfig, ConfigManager},
    state::{RobotState, StateManager},
    test_orchestrator::{StrategyTestOrchestrator, PipelineConfig, PipelineResult, StageResult, StageStatus},
    // Yeni: Dinamik Pozisyon Yönetimi ve Güvenlik Modülleri
    portfolio_manager::{DynamicPosition, TrailingStopConfig, ScaleInConfig, ScaleOutConfig, PartialFill},
    security::{SecurityManager, User, UserRole, AuditEvent, RateLimitRule, ApiKeyManager},
    integration_advanced::AdvancedRoboticTrader,
    config_helpers::{PositionManagementProfile, SecurityProfile, PositionConfigBuilder},
};
pub use types::{Trade, Signal, Candle, RiskParams, PositionId};
#[cfg(not(target_arch = "wasm32"))]
pub use database::DatabaseError;
pub use advanced::{
    indicators::Candle as AdvancedCandle,
    risk::{RiskParams as AdvancedRiskParams, TradeAction, TradeSignal},
    strategies::{StrategyEngine, StrategyResult},
};

// Sağlık ve anomali modülünü dışa aktar
pub use health_monitor::{
    HealthStatus, AnomalyType, HealthCheck, AnomalyDetector, HealthReport, LatencyHealthChecker
};

/// Kütüphane sürümü
pub const VERSION: &str = "0.1.0";

/// Hata tipi
#[derive(Debug, thiserror::Error)]
pub enum MemosTradingError {

    #[cfg(not(target_arch = "wasm32"))]
    #[error("Database error: {0}")]
    Database(#[from] DatabaseError),

    #[cfg(not(target_arch = "wasm32"))]
    #[error("Rusqlite error: {0}")]
    RusqliteError(rusqlite::Error),

    #[error("Serde error: {0}")]
    SerdeError(serde_json::Error),

    #[error("IO error: {0}")]
    IoError(std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Strategy error: {0}")]
    Strategy(String),

    #[error("Risk management error: {0}")]
    Risk(String),

    #[error("API error: {0}")]
    Api(String),

    #[error("Unknown error: {0}")]
    Unknown(String),

    #[cfg(target_arch = "wasm32")]
    #[error("Other error: {0}")]
    Other(&'static str),
}

#[cfg(not(target_arch = "wasm32"))]
impl From<rusqlite::Error> for MemosTradingError {
    fn from(err: rusqlite::Error) -> Self {
        MemosTradingError::RusqliteError(err)
    }
}

impl From<serde_json::Error> for MemosTradingError {
    fn from(err: serde_json::Error) -> Self {
        MemosTradingError::SerdeError(err)
    }
}

impl From<std::io::Error> for MemosTradingError {
    fn from(err: std::io::Error) -> Self {
        MemosTradingError::IoError(err)
    }
}

pub type Result<T> = std::result::Result<T, MemosTradingError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert_eq!(VERSION, "0.1.0");
    }
}
