
// src/robot/config_helpers.rs - Konfigürasyon Yardımcıları ve Otonom Profiller
// Srivastava ATP Mimarisi: Dinamik pozisyon ve rejim bazlı güvenlik preset'leri.

use serde::{Serialize, Deserialize};
use crate::prelude::*; // Kütüphanenin elit prelude odasını çağırıyoruz

// =============================================================================
// 1. ATOMİK PROBİL KONTRAATLARI (YAPI TAŞLARI)
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TrailingStopConfig {
    pub trailing_pct: f64,
    pub min_movement_bps: u32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScaleInConfig {
    pub enabled: bool,
    pub max_scalein_count: u32,
    pub scale_in_pct: f64,
    pub trigger_proximity_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleOutConfig {
    pub enabled: bool,
    pub max_scaleout_count: u32,
    pub scaleout_pct: f64,
    pub profit_targets: Vec<f64>,
}

// =============================================================================
// 2. DİNAMİK POZİSYON YÖNETİM PROFİLLERİ (VİTES KUTUSU)
// =============================================================================

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
    /// Profile'a göre trailing stop config oluşturur
    pub fn trailing_stop_config(&self) -> TrailingStopConfig {
        match self {
            Self::Conservative => TrailingStopConfig { trailing_pct: 1.5, min_movement_bps: 25, enabled: true },
            Self::Balanced => TrailingStopConfig { trailing_pct: 2.5, min_movement_bps: 50, enabled: true },
            Self::Aggressive => TrailingStopConfig { trailing_pct: 4.0, min_movement_bps: 100, enabled: true },
            Self::Scalper => TrailingStopConfig { trailing_pct: 0.5, min_movement_bps: 10, enabled: true },
            Self::SwingTrading => TrailingStopConfig { trailing_pct: 5.0, min_movement_bps: 200, enabled: true },
        }
    }
    
    /// Profile'a göre scale-in (Kademeli Alım) config oluşturur
    pub fn scale_in_config(&self) -> ScaleInConfig {
        match self {
            Self::Conservative => ScaleInConfig { enabled: false, max_scalein_count: 1, scale_in_pct: 25.0, trigger_proximity_pct: 1.0 },
            Self::Balanced => ScaleInConfig { enabled: true, max_scalein_count: 2, scale_in_pct: 50.0, trigger_proximity_pct: 0.5 },
            Self::Aggressive => ScaleInConfig { enabled: true, max_scalein_count: 4, scale_in_pct: 75.0, trigger_proximity_pct: 0.3 },
            Self::Scalper => ScaleInConfig { enabled: false, max_scalein_count: 0, scale_in_pct: 0.0, trigger_proximity_pct: 0.0 },
            Self::SwingTrading => ScaleInConfig { enabled: true, max_scalein_count: 3, scale_in_pct: 60.0, trigger_proximity_pct: 2.0 },
        }
    }
    
    /// Profile'a göre scale-out (Kademeli Satım) config oluşturur
    pub fn scale_out_config(&self) -> ScaleOutConfig {
        match self {
            Self::Conservative => ScaleOutConfig { enabled: true, max_scaleout_count: 2, scaleout_pct: 50.0, profit_targets: vec![1.0, 2.0] },
            Self::Balanced => ScaleOutConfig { enabled: true, max_scaleout_count: 3, scaleout_pct: 33.33, profit_targets: vec![2.0, 5.0, 10.0] },
            Self::Aggressive => ScaleOutConfig { enabled: true, max_scaleout_count: 4, scaleout_pct: 25.0, profit_targets: vec![3.0, 7.0, 15.0, 30.0] },
            Self::Scalper => ScaleOutConfig { enabled: true, max_scaleout_count: 2, scaleout_pct: 50.0, profit_targets: vec![0.5, 1.0] },
            Self::SwingTrading => ScaleOutConfig { enabled: true, max_scaleout_count: 4, scaleout_pct: 25.0, profit_targets: vec![5.0, 10.0, 20.0, 40.0] },
        }
    }
}

// =============================================================================
// 3. REJİM BAZLI GÜVENLİK PROFİLLERİ (SECURITY PROFILES)
// =============================================================================

/// Önceden tanımlı güvenlik ve rate limit profilleri
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecurityProfile {
    /// Test ortamı: Limitsiz
    Development,
    /// Stage ortamı: Orta limitler
    Staging,
    /// Production: Sıkı limitler
    Production,
    /// Enterprise: Çok sıkı limitler + tam adli denetim (Full Audit)
    Enterprise,
}

impl SecurityProfile {
    /// Parçaladığımız yeni 'security' garnizonuna bağlı trade akış limitini döndürür
    pub fn trade_rate_limit(&self) -> crate::robot::security::tracker::RateLimitRule {
        use crate::robot::security::tracker::RateLimitRule;
        match self {
            Self::Development => RateLimitRule { limit_type: "trades_per_minute".to_string(), max_per_second: 100, applies_to: "all".to_string() },
            Self::Staging => RateLimitRule { limit_type: "trades_per_minute".to_string(), max_per_second: 1, applies_to: "all".to_string() },
            Self::Production => RateLimitRule { limit_type: "trades_per_minute".to_string(), max_per_second: 0, applies_to: "all".to_string() },
            Self::Enterprise => RateLimitRule { limit_type: "trades_per_minute".to_string(), max_per_second: 0, applies_to: "all".to_string() },
        }
    }
    
    /// API çağrı akış limitini döndürür
    pub fn api_rate_limit(&self) -> crate::robot::security::tracker::RateLimitRule {
        use crate::robot::security::tracker::RateLimitRule;
        match self {
            Self::Development => RateLimitRule { limit_type: "api_calls_per_second".to_string(), max_per_second: 100, applies_to: "all".to_string() },
            Self::Staging => RateLimitRule { limit_type: "api_calls_per_second".to_string(), max_per_second: 20, applies_to: "all".to_string() },
            Self::Production => RateLimitRule { limit_type: "api_calls_per_second".to_string(), max_per_second: 10, applies_to: "all".to_string() },
            Self::Enterprise => RateLimitRule { limit_type: "api_calls_per_second".to_string(), max_per_second: 5, applies_to: "all".to_string() },
        }
    }
    
    /// Audit log export sıklığı (dakika)
    pub fn audit_export_interval_minutes(&self) -> u32 {
        match self {
            Self::Development => 0,  // Pasif
            Self::Staging => 60,     // Her saat
            Self::Production => 30,  // Her 30 dakika
            Self::Enterprise => 15,  // Her 15 dakika
        }
    }
    
    /// Acil durdurma yetkisine sahip yetkili rolleri süzgeçten geçirir
    pub fn emergency_stop_roles(&self) -> Vec<crate::robot::security::types::UserRole> {
        use crate::robot::security::types::UserRole;
        match self {
            Self::Development => vec![UserRole::Admin, UserRole::Trader, UserRole::Monitor],
            Self::Staging => vec![UserRole::Admin, UserRole::Trader],
            Self::Production => vec![UserRole::Admin],
            Self::Enterprise => vec![UserRole::Admin],
        }
    }
}

// =============================================================================
// 4. AKICI KURULUM MOTORU (POSITION CONFIG BUILDER)
// =============================================================================

/// Kullanım örneği fluent builder API kalıbı
pub struct PositionConfigBuilder {
    profile: PositionManagementProfile,
    trailing_stop: Option<TrailingStopConfig>,
    scale_in: Option<ScaleInConfig>,
    scale_out: Option<ScaleOutConfig>,
}

impl PositionConfigBuilder {
    /// Yeni akıllı builder oluşturur
    pub fn new(profile: PositionManagementProfile) -> Self {
        Self {
            profile,
            trailing_stop: None,
            scale_in: None,
            scale_out: None,
        }
    }
    
    /// Dinamik Trailing Stop geçersiz kılma (Override) refleks kapısı
    pub fn with_trailing_stop(mut self, config: TrailingStopConfig) -> Self {
        self.trailing_stop = Some(config);
        self
    }
    
    /// Dinamik Scale-In geçersiz kılma (Override) refleks kapısı
    pub fn with_scale_in(mut self, config: ScaleInConfig) -> Self {
        self.scale_in = Some(config);
        self
    }
    
    /// Dinamik Scale-Out geçersiz kılma (Override) refleks kapısı
    pub fn with_scale_out(mut self, config: ScaleOutConfig) -> Self {
        self.scale_out = Some(config);
        self
    }
    
    /// Tüm profilleri birleştirerek kilit pariteleri infaz döngüsüne teslim eder
    pub fn build(self) -> (TrailingStopConfig, ScaleInConfig, ScaleOutConfig) {
        (
            self.trailing_stop.unwrap_or_else(|| self.profile.trailing_stop_config()),
            self.scale_in.unwrap_or_else(|| self.profile.scale_in_config()),
            self.scale_out.unwrap_or_else(|| self.profile.scale_out_config()),
        )
    }
}
