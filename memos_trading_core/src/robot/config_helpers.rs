// Konfigürasyon Yardımcıları - Dynamik Pozisyon ve Güvenlik için preset'ler
// Configuration Helpers: Common presets for dynamic position management and security

use crate::robot::portfolio_manager::{TrailingStopConfig, ScaleInConfig, ScaleOutConfig};
use crate::robot::security::{UserRole, RateLimitRule};
use serde::{Serialize, Deserialize};

/// Önceden tanımlı dinamik pozisyon yönetim profilleri
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PositionManagementProfile {
    /// Muhafazakar: Dar trailing stop, minimal scale-in/out
    Conservative,
    /// Dengeli: Orta seviye agresiflik
    Balanced,
    /// Agresif: Gevşek trailing stop, maksimum scale-in/out
    Aggressive,
    /// Scalper: Hızlı giriş-çıkış, dar kar hedefleri
    Scalper,
    /// Swing: Uzun dönem, geniş kar hedefleri
    SwingTrading,
}

impl PositionManagementProfile {
    /// Profile'a göre trailing stop config oluştur
    pub fn trailing_stop_config(&self) -> TrailingStopConfig {
        match self {
            Self::Conservative => TrailingStopConfig {
                trailing_pct: 1.5,
                min_movement_bps: 25,
                enabled: true,
            },
            Self::Balanced => TrailingStopConfig {
                trailing_pct: 2.5,
                min_movement_bps: 50,
                enabled: true,
            },
            Self::Aggressive => TrailingStopConfig {
                trailing_pct: 4.0,
                min_movement_bps: 100,
                enabled: true,
            },
            Self::Scalper => TrailingStopConfig {
                trailing_pct: 0.5,
                min_movement_bps: 10,
                enabled: true,
            },
            Self::SwingTrading => TrailingStopConfig {
                trailing_pct: 5.0,
                min_movement_bps: 200,
                enabled: true,
            },
        }
    }
    
    /// Profile'a göre scale-in config oluştur
    pub fn scale_in_config(&self) -> ScaleInConfig {
        match self {
            Self::Conservative => ScaleInConfig {
                enabled: false,
                max_scalein_count: 1,
                scale_in_pct: 25.0,
                trigger_proximity_pct: 1.0,
            },
            Self::Balanced => ScaleInConfig {
                enabled: true,
                max_scalein_count: 2,
                scale_in_pct: 50.0,
                trigger_proximity_pct: 0.5,
            },
            Self::Aggressive => ScaleInConfig {
                enabled: true,
                max_scalein_count: 4,
                scale_in_pct: 75.0,
                trigger_proximity_pct: 0.3,
            },
            Self::Scalper => ScaleInConfig {
                enabled: false,
                max_scalein_count: 0,
                scale_in_pct: 0.0,
                trigger_proximity_pct: 0.0,
            },
            Self::SwingTrading => ScaleInConfig {
                enabled: true,
                max_scalein_count: 3,
                scale_in_pct: 60.0,
                trigger_proximity_pct: 2.0,
            },
        }
    }
    
    /// Profile'a göre scale-out config oluştur
    pub fn scale_out_config(&self) -> ScaleOutConfig {
        match self {
            Self::Conservative => ScaleOutConfig {
                enabled: true,
                max_scaleout_count: 2,
                scaleout_pct: 50.0,
                profit_targets: vec![1.0, 2.0],
            },
            Self::Balanced => ScaleOutConfig {
                enabled: true,
                max_scaleout_count: 3,
                scaleout_pct: 33.33,
                profit_targets: vec![2.0, 5.0, 10.0],
            },
            Self::Aggressive => ScaleOutConfig {
                enabled: true,
                max_scaleout_count: 4,
                scaleout_pct: 25.0,
                profit_targets: vec![3.0, 7.0, 15.0, 30.0],
            },
            Self::Scalper => ScaleOutConfig {
                enabled: true,
                max_scaleout_count: 2,
                scaleout_pct: 50.0,
                profit_targets: vec![0.5, 1.0],
            },
            Self::SwingTrading => ScaleOutConfig {
                enabled: true,
                max_scaleout_count: 4,
                scaleout_pct: 25.0,
                profit_targets: vec![5.0, 10.0, 20.0, 40.0],
            },
        }
    }
}

/// Önceden tanımlı güvenlik ve rate limit profilleri
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecurityProfile {
    /// Test ortamı: Limitsiz
    Development,
    /// Stage ortamı: Orta limitler
    Staging,
    /// Production: Sıkı limitler
    Production,
    /// Enterprise: Çok sıkı limitler + full audit
    Enterprise,
}

impl SecurityProfile {
    /// Trade rate limit oluştur
    pub fn trade_rate_limit(&self) -> RateLimitRule {
        match self {
            Self::Development => RateLimitRule {
                limit_type: "trades_per_minute".to_string(),
                max_per_second: 100, // Test için çok yüksek
                applies_to: "all".to_string(),
            },
            Self::Staging => RateLimitRule {
                limit_type: "trades_per_minute".to_string(),
                max_per_second: 1, // 60 per minute
                applies_to: "all".to_string(),
            },
            Self::Production => RateLimitRule {
                limit_type: "trades_per_minute".to_string(),
                max_per_second: 0, // 10 per minute (0.17/sec)
                applies_to: "all".to_string(),
            },
            Self::Enterprise => RateLimitRule {
                limit_type: "trades_per_minute".to_string(),
                max_per_second: 0, // 5 per minute (0.08/sec)
                applies_to: "all".to_string(),
            },
        }
    }
    
    /// API call rate limit
    pub fn api_rate_limit(&self) -> RateLimitRule {
        match self {
            Self::Development => RateLimitRule {
                limit_type: "api_calls_per_second".to_string(),
                max_per_second: 100,
                applies_to: "all".to_string(),
            },
            Self::Staging => RateLimitRule {
                limit_type: "api_calls_per_second".to_string(),
                max_per_second: 20,
                applies_to: "all".to_string(),
            },
            Self::Production => RateLimitRule {
                limit_type: "api_calls_per_second".to_string(),
                max_per_second: 10,
                applies_to: "all".to_string(),
            },
            Self::Enterprise => RateLimitRule {
                limit_type: "api_calls_per_second".to_string(),
                max_per_second: 5,
                applies_to: "all".to_string(),
            },
        }
    }
    
    /// Audit log export sıklığı (dakika)
    pub fn audit_export_interval_minutes(&self) -> u32 {
        match self {
            Self::Development => 0,  // Disable
            Self::Staging => 60,     // Her saat
            Self::Production => 30,  // Her 30 dakika
            Self::Enterprise => 15,  // Her 15 dakika
        }
    }
    
    /// Emergency stop yetkisi gerektiren roller
    pub fn emergency_stop_roles(&self) -> Vec<UserRole> {
        match self {
            Self::Development => vec![UserRole::Admin, UserRole::Trader, UserRole::Monitor],
            Self::Staging => vec![UserRole::Admin, UserRole::Trader],
            Self::Production => vec![UserRole::Admin],
            Self::Enterprise => vec![UserRole::Admin],
        }
    }
}

/// Kullanım örneği builder
pub struct PositionConfigBuilder {
    profile: PositionManagementProfile,
    trailing_stop: Option<TrailingStopConfig>,
    scale_in: Option<ScaleInConfig>,
    scale_out: Option<ScaleOutConfig>,
}

impl PositionConfigBuilder {
    /// Yeni builder oluştur
    pub fn new(profile: PositionManagementProfile) -> Self {
        Self {
            profile,
            trailing_stop: None,
            scale_in: None,
            scale_out: None,
        }
    }
    
    /// Custom trailing stop override
    pub fn with_trailing_stop(mut self, config: TrailingStopConfig) -> Self {
        self.trailing_stop = Some(config);
        self
    }
    
    /// Custom scale-in override
    pub fn with_scale_in(mut self, config: ScaleInConfig) -> Self {
        self.scale_in = Some(config);
        self
    }
    
    /// Custom scale-out override
    pub fn with_scale_out(mut self, config: ScaleOutConfig) -> Self {
        self.scale_out = Some(config);
        self
    }
    
    /// Build all configs
    pub fn build(self) -> (TrailingStopConfig, ScaleInConfig, ScaleOutConfig) {
        (
            self.trailing_stop.unwrap_or_else(|| self.profile.trailing_stop_config()),
            self.scale_in.unwrap_or_else(|| self.profile.scale_in_config()),
            self.scale_out.unwrap_or_else(|| self.profile.scale_out_config()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conservative_profile() {
        let profile = PositionManagementProfile::Conservative;
        let ts = profile.trailing_stop_config();
        let si = profile.scale_in_config();
        let so = profile.scale_out_config();
        
        assert_eq!(ts.trailing_pct, 1.5);
        assert!(!si.enabled);
        assert_eq!(so.profit_targets.len(), 2);
    }

    #[test]
    fn test_aggressive_profile() {
        let profile = PositionManagementProfile::Aggressive;
        let ts = profile.trailing_stop_config();
        let si = profile.scale_in_config();
        
        assert_eq!(ts.trailing_pct, 4.0);
        assert!(si.enabled);
        assert_eq!(si.max_scalein_count, 4);
    }

    #[test]
    fn test_security_profiles() {
        let dev = SecurityProfile::Development;
        let prod = SecurityProfile::Production;
        
        assert!(dev.trade_rate_limit().max_per_second > prod.trade_rate_limit().max_per_second);
        assert!(dev.emergency_stop_roles().len() >= prod.emergency_stop_roles().len());
    }

    #[test]
    fn test_builder_pattern() {
        let (ts, si, _so) = PositionConfigBuilder::new(PositionManagementProfile::Balanced)
            .with_trailing_stop(TrailingStopConfig {
                trailing_pct: 3.0,
                min_movement_bps: 75,
                enabled: true,
            })
            .build();
        
        assert_eq!(ts.trailing_pct, 3.0); // Custom override
        assert_eq!(si.max_scalein_count, 2); // Default dari Balanced
    }
}
