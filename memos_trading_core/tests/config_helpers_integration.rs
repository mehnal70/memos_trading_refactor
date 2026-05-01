// Integration test: Config helpers'ın profil ve config üretim işlevselliği
// Tests that config helpers generate correct configs for different profiles

#[cfg(test)]
mod integration_with_config_helpers {
    use memos_trading_core::{
        PositionManagementProfile, SecurityProfile, PositionConfigBuilder,
        TrailingStopConfig, ScaleInConfig,
        UserRole,
    };

    #[test]
    fn test_conservative_profile_configs() {
        // Conservative profil - düşük risk
        let profile = PositionManagementProfile::Conservative;
        
        let trailing = profile.trailing_stop_config();
        let scale_in = profile.scale_in_config();
        let scale_out = profile.scale_out_config();

        // Conservative özellikleri
        assert_eq!(trailing.trailing_pct, 1.5);
        assert_eq!(trailing.min_movement_bps, 25);
        assert!(trailing.enabled);
        
        assert!(!scale_in.enabled); // Conservative scale-in kullanmaz
        assert_eq!(scale_in.max_scalein_count, 1);
        
        assert!(scale_out.enabled);
        assert_eq!(scale_out.max_scaleout_count, 2);
        assert_eq!(scale_out.profit_targets, vec![1.0, 2.0]);
    }

    #[test]
    fn test_aggressive_profile_configs() {
        // Aggressive profil - yüksek risk/getiri
        let profile = PositionManagementProfile::Aggressive;
        
        let trailing = profile.trailing_stop_config();
        let scale_in = profile.scale_in_config();
        let scale_out = profile.scale_out_config();

        // Aggressive özellikleri
        assert_eq!(trailing.trailing_pct, 4.0);
        assert_eq!(trailing.min_movement_bps, 100);
        
        assert!(scale_in.enabled);
        assert_eq!(scale_in.max_scalein_count, 4);
        assert_eq!(scale_in.scale_in_pct, 75.0);
        
        assert_eq!(scale_out.max_scaleout_count, 4);
        assert_eq!(scale_out.profit_targets, vec![3.0, 7.0, 15.0, 30.0]);
    }

    #[test]
    fn test_custom_balanced_profile_with_builder() {
        // Builder pattern ile Balanced profilini özelleştir
        let (trailing, scale_in, scale_out) = PositionConfigBuilder::new(
            PositionManagementProfile::Balanced
        )
        .with_trailing_stop(TrailingStopConfig {
            trailing_pct: 3.0,      // Custom: Daha geniş trailing
            min_movement_bps: 75,
            enabled: true,
        })
        .with_scale_in(ScaleInConfig {
            enabled: true,
            max_scalein_count: 3,   // Custom: Daha fazla scale-in
            scale_in_pct: 60.0,
            trigger_proximity_pct: 0.4,
        })
        .build();

        // Custom ayarlar doğru uygulanmış
        assert_eq!(trailing.trailing_pct, 3.0); // Custom değer
        assert_eq!(scale_in.max_scalein_count, 3); // Custom değer
        assert_eq!(scale_in.scale_in_pct, 60.0);
        
        // Scale-out Balanced default'undan gelir (override edilmedi)
        assert_eq!(scale_out.max_scaleout_count, 3);
        assert_eq!(scale_out.profit_targets, vec![2.0, 5.0, 10.0]);
    }

    #[test]
    fn test_security_profiles_enforcement() {
        // Development profili - limitless
        let dev_profile = SecurityProfile::Development;
        let dev_trade_limit = dev_profile.trade_rate_limit();
        assert_eq!(dev_trade_limit.max_per_second, 100); // Çok yüksek

        // Production profili - strict
        let prod_profile = SecurityProfile::Production;
        let prod_trade_limit = prod_profile.trade_rate_limit();
        assert_eq!(prod_trade_limit.max_per_second, 0); // 10 per minute

        // Emergency stop yetkisi
        let dev_roles = dev_profile.emergency_stop_roles();
        let prod_roles = prod_profile.emergency_stop_roles();
        
        assert!(dev_roles.contains(&UserRole::Admin));
        assert!(dev_roles.contains(&UserRole::Trader));
        assert!(prod_roles.contains(&UserRole::Admin));
        assert!(!prod_roles.contains(&UserRole::Trader)); // Production'da sadece Admin
    }

    #[test]
    fn test_scalper_profile_characteristics() {
        // Scalper profili - çok dar marj, hızlı exit
        let profile = PositionManagementProfile::Scalper;
        
        let trailing = profile.trailing_stop_config();
        let scale_in = profile.scale_in_config();
        let scale_out = profile.scale_out_config();

        // Scalper özellikleri
        assert_eq!(trailing.trailing_pct, 0.5); // Çok dar trailing
        assert_eq!(trailing.min_movement_bps, 10); // Minimal hareket
        
        assert!(!scale_in.enabled); // Scalper scale-in kullanmaz
        
        assert_eq!(scale_out.profit_targets, vec![0.5, 1.0]); // Düşük kar hedefleri
        assert_eq!(scale_out.scaleout_pct, 50.0);
    }

    #[test]
    fn test_swing_trading_profile_characteristics() {
        // SwingTrading profili - uzun dönem, geniş marj
        let profile = PositionManagementProfile::SwingTrading;
        
        let trailing = profile.trailing_stop_config();
        let scale_in = profile.scale_in_config();
        let scale_out = profile.scale_out_config();

        // Swing trading özellikleri
        assert_eq!(trailing.trailing_pct, 5.0); // Geniş trailing
        assert_eq!(trailing.min_movement_bps, 200); // Büyük hareket gerekli
        
        assert!(scale_in.enabled);
        assert_eq!(scale_in.max_scalein_count, 3);
        assert_eq!(scale_in.scale_in_pct, 60.0);
        
        assert_eq!(scale_out.profit_targets, vec![5.0, 10.0, 20.0, 40.0]); // Yüksek kar hedefleri
    }

    #[test]
    fn test_balanced_profile() {
        // Balanced - orta risk/getiri
        let profile = PositionManagementProfile::Balanced;
        
        let trailing = profile.trailing_stop_config();
        let scale_in = profile.scale_in_config();
        let scale_out = profile.scale_out_config();

        assert_eq!(trailing.trailing_pct, 2.5);
        assert_eq!(scale_in.max_scalein_count, 2);
        assert_eq!(scale_out.profit_targets, vec![2.0, 5.0, 10.0]);
    }

    #[test]
    fn test_profile_comparison() {
        // Profillerin karşılaştırmalı özellikleri
        let conservative = PositionManagementProfile::Conservative;
        let aggressive = PositionManagementProfile::Aggressive;
        
        let c_trailing = conservative.trailing_stop_config();
        let a_trailing = aggressive.trailing_stop_config();
        
        // Aggressive'in trailing stop'u daha geniş
        assert!(a_trailing.trailing_pct > c_trailing.trailing_pct);
        
        // Aggressive scale-in kullanır, Conservative kullanmaz
        assert!(!conservative.scale_in_config().enabled);
        assert!(aggressive.scale_in_config().enabled);
        
        // Aggressive daha fazla scale-out target'a sahip
        assert!(aggressive.scale_out_config().profit_targets.len() > 
                conservative.scale_out_config().profit_targets.len());
    }
}
