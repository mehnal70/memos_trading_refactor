// src/robot/config_engine.rs — Tam otonom merkezi konfigürasyon motoru.
// 9 farklı statik profil dosyasını kaldırıp borsa kabiliyetlerine göre çalışma anında karar verir.
use crate::core::types::{Exchange, Market};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OtonomMode {
    Live,
    Paper,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecisionPolicy {
    pub regime_interval: String,
    pub setup_interval: String,
    pub trigger_interval: String,
}

impl Default for DecisionPolicy {
    fn default() -> Self {
        Self {
            regime_interval: "1h".to_string(),
            setup_interval: "15m".to_string(),
            trigger_interval: "5m".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CentralConfig {
    pub mode: OtonomMode,
    pub build_mode: String,
    pub target: String,
    pub db_path: String,
    pub interval: String,
    pub download_candle_limit: usize,
    pub auto_interval_candidates: Vec<String>,
    #[serde(default)]
    pub decision_policy: DecisionPolicy,
    pub regime_directional: bool,
    pub regime_adaptive_pctl: f64,
    pub telegram_cooldown_secs: u64,
    pub scheduler_logscan_enabled: bool,
    pub scheduler_logscan_every_mins: u64,
    pub scheduler_logscan_recovery_enable: bool,
    pub screener_min_score: f64,
    pub screener_min_volume: f64,
    pub pinned_symbols: Vec<String>,
}

impl Default for CentralConfig {
    fn default() -> Self {
        return Self {
            mode: OtonomMode::Paper,
            build_mode: "release".to_string(),
            target: "tui".to_string(),
            db_path: "data/trader.db".to_string(),
            interval: "15m".to_string(),
            download_candle_limit: 1000,
            auto_interval_candidates: vec!["1h".to_string(), "4h".to_string(), "1d".to_string()],
            decision_policy: DecisionPolicy::default(),
            regime_directional: true,
            regime_adaptive_pctl: 0.90,
            telegram_cooldown_secs: 60,
            scheduler_logscan_enabled: true,
            scheduler_logscan_every_mins: 60,
            scheduler_logscan_recovery_enable: true,
            screener_min_score: 0.35,
            screener_min_volume: 350000.0,
            pinned_symbols: vec![],
        };
    }
}
pub struct OtonomConfigEngine;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_policy_has_explicit_multi_timeframe_defaults() {
        let policy = DecisionPolicy::default();
        assert_eq!(policy.regime_interval, "1h");
        assert_eq!(policy.setup_interval, "15m");
        assert_eq!(policy.trigger_interval, "5m");
    }

    #[test]
    fn central_config_keeps_decision_policy_in_one_place() {
        let config = CentralConfig::default();
        assert_eq!(config.decision_policy, DecisionPolicy::default());
    }
}

impl OtonomConfigEngine {
    fn parse_symbol_csv(raw: &str) -> Vec<String> {
        raw.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    fn fallback_pinned_symbols() -> Vec<String> {
        let mut out = Vec::new();
        for key in [
            "AUTO_BINANCE_SYMBOLS",
            "AUTO_MT5_SYMBOLS",
            "AUTO_IBKR_SYMBOLS",
            "AUTO_BIST_SYMBOLS",
        ] {
            if let Ok(raw) = std::env::var(key) {
                out.extend(Self::parse_symbol_csv(&raw));
            }
        }
        out
    }

    pub fn symbol_allowed(symbol: &str) -> bool {
        let Ok(raw) = std::env::var("MT5_SYMBOL_ALLOWLIST") else {
            return true;
        };
        let allowlist: Vec<String> = Self::parse_symbol_csv(&raw)
            .into_iter()
            .map(|item| item.to_ascii_uppercase())
            .collect();
        if allowlist.is_empty() {
            return true;
        }

        let normalized = symbol.trim().to_ascii_uppercase();
        if !normalized.ends_with("@MT5") {
            return allowlist
                .iter()
                .any(|allowed| allowed == &normalized || allowed == &format!("{normalized}@MT5"));
        }
        allowlist.iter().any(|allowed| allowed == &normalized)
    }

    /// Sistem başlangıcında tek bir ana dosyadan veya env'den temel ayarları yükler.
    pub fn load_central_config() -> CentralConfig {
        let mut cfg = CentralConfig::default();

        if let Ok(m) = std::env::var("TRADING_MODE") {
            if m.to_lowercase() == "live" {
                cfg.mode = OtonomMode::Live;
            }
        }
        if let Ok(p) = std::env::var("DB_PATH") {
            cfg.db_path = p;
        }
        if let Ok(i) = std::env::var("TRADE_INTERVAL") {
            cfg.interval = i;
        }
        if let Ok(i) = std::env::var("REGIME_INTERVAL") {
            cfg.decision_policy.regime_interval = i;
        }
        if let Ok(i) = std::env::var("SETUP_INTERVAL") {
            cfg.decision_policy.setup_interval = i;
        }
        if let Ok(i) = std::env::var("TRIGGER_INTERVAL") {
            cfg.decision_policy.trigger_interval = i;
        }

        if let Ok(syms) = std::env::var("PINNED_SYMBOLS") {
            cfg.pinned_symbols = Self::parse_symbol_csv(&syms);
        } else {
            cfg.pinned_symbols = Self::fallback_pinned_symbols();
            if cfg.pinned_symbols.is_empty() {
                // Hiçbir şey verilmezse ortak korumalı Forex paritelerini tohumla.
                cfg.pinned_symbols = vec![
                    "EURUSD@mt5".to_string(),
                    "GBPUSD@mt5".to_string(),
                    "USDJPY@mt5".to_string(),
                    "XAUUSD@mt5".to_string(),
                ];
            }
        }
        cfg.pinned_symbols
            .retain(|symbol| Self::symbol_allowed(symbol));

        return cfg;
    }
    /// ARTIK TAM OTONOM: Stratejik kitap kısıtlamalarını tamamen yıkarak,
    /// Registry'deki borsa yetenek matrisine (`capabilities()`) göre çalışma anında refleks üretir.
    pub fn evaluate_market_reflexes(
        venue: &std::sync::Arc<dyn crate::robot::venue::adapter::VenueAdapter>,
    ) -> BTreeMap<String, String> {
        let mut reflexes = BTreeMap::new();
        let caps = venue.capabilities();

        // 1. Fonlama ve Carry Arbitraj Kabiliyeti Keşfi
        if caps.contains(&crate::robot::venue::adapter::ExchangeCapability::FundingHistory) {
            reflexes.insert("CARRY_LIVE_ENABLED".to_string(), "1".to_string());
            reflexes.insert("CARRY_LIVE_MARKET".to_string(), "futures".to_string());
            reflexes.insert("USE_LIMIT_ENTRY".to_string(), "1".to_string()); // Maker icra avantajı
        } else if caps.contains(&crate::robot::venue::adapter::ExchangeCapability::SwapRates) {
            reflexes.insert("CARRY_LIVE_ENABLED".to_string(), "1".to_string());
            reflexes.insert("CARRY_LIVE_MARKET".to_string(), "cfd".to_string());
            reflexes.insert("USE_LIMIT_ENTRY".to_string(), "0".to_string()); // Forex'te doğrudan Taker icra
        } else {
            reflexes.insert("CARRY_LIVE_ENABLED".to_string(), "0".to_string());
        }

        // 2. Kesitsel Momentum (XS) ve Portföy Harmanlama (Blend) Keşfi
        if caps.contains(&crate::robot::venue::adapter::ExchangeCapability::Level2OrderBook) {
            reflexes.insert("XS_LIVE_ENABLED".to_string(), "1".to_string());
            reflexes.insert("BLEND_LIVE_ENABLED".to_string(), "1".to_string());
            reflexes.insert("SYMBOLIC_GATING_ENABLE".to_string(), "1".to_string());
        } else {
            reflexes.insert("XS_LIVE_ENABLED".to_string(), "0".to_string());
            reflexes.insert("BLEND_LIVE_ENABLED".to_string(), "0".to_string());
            reflexes.insert("SYMBOLIC_GATING_ENABLE".to_string(), "0".to_string());
        }

        // 3. Piyasa Türüne Göre Hacim Filtre (Screener) Muafiyetleri
        if venue.market() == Market::Cfd {
            reflexes.insert("SCREENER_TOP_N".to_string(), "0".to_string()); // Forex pinned sembollerle çalışır
            reflexes.insert("SCREENER_MIN_VOLUME".to_string(), "0".to_string()); // tick_volume birim hatası elenir
            reflexes.insert("AUTO_SESSION_HANDOFF".to_string(), "1".to_string());
        // Coğrafi seans geçişleri aktif
        } else {
            reflexes.insert("SCREENER_TOP_N".to_string(), "8".to_string());
            reflexes.insert("SCREENER_MIN_VOLUME".to_string(), "350000".to_string());
            reflexes.insert("AUTO_SESSION_HANDOFF".to_string(), "0".to_string());
        }

        return reflexes;
    }
    /// UTC saat dilimine göre Asya, Avrupa ve ABD seans geçişlerini borsa bazında otonom yönetir.
    pub fn get_active_session_symbols(current_utc_hour: u32) -> Vec<String> {
        return match current_utc_hour {
            // Asya Seansı (00:00 - 08:00 UTC)
            0..=7 => vec![
                "USDJPY@mt5".to_string(),
                "AUDUSD@mt5".to_string(),
                "XAUUSD@mt5".to_string(),
            ],
            // Asya - Avrupa Çakışması (08:00 - 10:00 UTC)
            8..=9 => vec![
                "USDJPY@mt5".to_string(),
                "EURUSD@mt5".to_string(),
                "GBPUSD@mt5".to_string(),
            ],
            // Avrupa Seansı (10:00 - 13:00 UTC)
            10..=12 => vec![
                "EURUSD@mt5".to_string(),
                "GBPUSD@mt5".to_string(),
                "XAUUSD@mt5".to_string(),
            ],
            // Avrupa - ABD Çakışması (13:00 - 17:00 UTC)
            13..=16 => vec![
                "EURUSD@mt5".to_string(),
                "GBPUSD@mt5".to_string(),
                "USDCAD@mt5".to_string(),
            ],
            // ABD Seansı (17:00 - 22:00 UTC)
            17..=21 => vec![
                "USDCAD@mt5".to_string(),
                "EURUSD@mt5".to_string(),
                "USDCHF@mt5".to_string(),
            ],
            // Seans Dışı / Kripto Odaklı Saatler (22:00 - 24:00 UTC)
            _ => vec![
                "BTCUSDT@binance".to_string(),
                "ETHUSDT@binance".to_string(),
                "SOLUSDT@binance".to_string(),
            ],
        };
    }
}
