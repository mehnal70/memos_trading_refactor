// Integration test: DynamicPosition + Config Helpers workflow
// Real-world usage: Profile seçimi → Pozisyon oluşturma → Fiyat update

#[cfg(test)]
mod dynamic_position_profile_integration {
    use memos_trading_core::{
        DynamicPosition, PositionManagementProfile,
        TrailingStopConfig,
    };

    #[test]
    fn test_conservative_position_with_profile() {
        // Conservative profil kullanarak pozisyon oluştur
        let profile = PositionManagementProfile::Conservative;
        
        let position = DynamicPosition::with_configs(
            "BTCUSDT".to_string(),
            45000.0,  // entry_price
            1.0,      // quantity
            1.0,      // long
            Some(44000.0),
            Some(46000.0),
            profile.trailing_stop_config(),
            profile.scale_in_config(),
            profile.scale_out_config(),
        );
        
        // Conservative özellikleri doğru uygulanmış
        assert_eq!(position.trailing_config.trailing_pct, 1.5);
        assert!(!position.scalein_config.enabled);
        assert_eq!(position.scaleout_config.profit_targets, vec![1.0, 2.0]);
    }

    #[test]
    fn test_aggressive_position_with_profile() {
        // Aggressive profil - maksimum scale-in/out
        let profile = PositionManagementProfile::Aggressive;
        
        let position = DynamicPosition::with_configs(
            "ETHUSDT".to_string(),
            3000.0,
            5.0,
            1.0,
            Some(2900.0),
            Some(3500.0),
            profile.trailing_stop_config(),
            profile.scale_in_config(),
            profile.scale_out_config(),
        );
        
        assert_eq!(position.trailing_config.trailing_pct, 4.0);
        assert!(position.scalein_config.enabled);
        assert_eq!(position.scalein_config.max_scalein_count, 4);
        assert_eq!(position.scaleout_config.max_scaleout_count, 4);
    }

    #[test]
    fn test_apply_configs_to_existing_position() {
        // Mevcut pozisyona config uygula (builder pattern)
        let mut position = DynamicPosition::new(
            "BTCUSDT".to_string(),
            45000.0,
            1.0,
            1.0,
            Some(44000.0),
            Some(46000.0),
        );
        
        // Başlangıçta default config'ler
        assert_eq!(position.trailing_config.trailing_pct, 2.5); // Default
        
        // Balanced profili uygula
        let profile = PositionManagementProfile::Balanced;
        position.apply_configs(
            profile.trailing_stop_config(),
            profile.scale_in_config(),
            profile.scale_out_config(),
        );
        
        // Config'ler DEĞİŞMEMİŞ (Balanced da 2.5 kullanıyor)
        assert_eq!(position.trailing_config.trailing_pct, 2.5); // Balanced
        assert_eq!(position.scalein_config.max_scalein_count, 2);
        
        // Aggressive ile değiştir - bu gerçekten değişir
        let aggressive = PositionManagementProfile::Aggressive;
        position.apply_configs(
            aggressive.trailing_stop_config(),
            aggressive.scale_in_config(), 
            aggressive.scale_out_config(),
        );
        
        assert_eq!(position.trailing_config.trailing_pct, 4.0); // Değişti!
        assert_eq!(position.scalein_config.max_scalein_count, 4);
    }

    #[test]
    fn test_scalper_profile_tight_stops() {
        // Scalper - çok dar trailing stop
        let profile = PositionManagementProfile::Scalper;
        
        let mut position = DynamicPosition::with_configs(
            "BTCUSDT".to_string(),
            45000.0,
            1.0,
            1.0,
            None,
            None,
            profile.trailing_stop_config(),
            profile.scale_in_config(),
            profile.scale_out_config(),
        );
        
        // Scalper: %0.5 trailing
        assert_eq!(position.trailing_config.trailing_pct, 0.5);
        assert_eq!(position.trailing_config.min_movement_bps, 10);
        
        // Fiyat artışı simule et
        position.current_price = 45500.0; // %1.1 artış
        let triggered = position.update_trailing_stop();
        
        // Scalper'ın dar stop'u henüz tetiklenmemiş (fiyat hala yükseliyor)
        assert!(!triggered);
    }

    #[test]
    fn test_swing_trading_wide_stops() {
        // SwingTrading - geniş trailing stop
        let profile = PositionManagementProfile::SwingTrading;
        
        let mut position = DynamicPosition::with_configs(
            "BTCUSDT".to_string(),
            45000.0,
            1.0,
            1.0,
            None,
            None,
            profile.trailing_stop_config(),
            profile.scale_in_config(),
            profile.scale_out_config(),
        );
        
        // SwingTrading: %5 trailing
        assert_eq!(position.trailing_config.trailing_pct, 5.0);
        assert_eq!(position.trailing_config.min_movement_bps, 200);
        
        // Büyük volatilite simule et
        position.current_price = 47000.0; // %4.4 artış
        position.update_trailing_stop();
        
        // Trailing stop level yükselmiş olmalı
        assert!(position.current_trailing_sl.is_some());
        let trailing_sl = position.current_trailing_sl.unwrap();
        
        // 47000 * 0.95 = 44650 civarı
        assert!(trailing_sl > 44000.0);
        assert!(trailing_sl < 45000.0);
    }

    #[test]
    fn test_custom_config_override() {
        // Balanced base, custom trailing
        let profile = PositionManagementProfile::Balanced;
        
        let custom_trailing = TrailingStopConfig {
            trailing_pct: 3.5,
            min_movement_bps: 100,
            enabled: true,
        };
        
        let position = DynamicPosition::with_configs(
            "BTCUSDT".to_string(),
            45000.0,
            1.0,
            1.0,
            None,
            None,
            custom_trailing, // Custom
            profile.scale_in_config(), // Balanced default
            profile.scale_out_config(), // Balanced default
        );
        
        // Custom trailing uygulanmış
        assert_eq!(position.trailing_config.trailing_pct, 3.5);
        
        // Scale-in/out Balanced default'tan gelmiş
        assert_eq!(position.scalein_config.max_scalein_count, 2);
        assert_eq!(position.scaleout_config.profit_targets, vec![2.0, 5.0, 10.0]);
    }

    #[test]
    fn test_scale_in_with_balanced_profile() {
        // Balanced profil ile scale-in test
        let profile = PositionManagementProfile::Balanced;
        
        let mut position = DynamicPosition::with_configs(
            "BTCUSDT".to_string(),
            45000.0,
            1.0,
            1.0,
            Some(44000.0),
            Some(50000.0),
            profile.trailing_stop_config(),
            profile.scale_in_config(),
            profile.scale_out_config(),
        );
        
        // Balanced: scale-in enabled, max 2 kez
        assert!(position.scalein_config.enabled);
        assert_eq!(position.scalein_config.max_scalein_count, 2);
        
        // Fiyat düşüşü - scale-in fırsatı
        position.current_price = 44800.0;
        
        if position.can_scalein(44800.0) {
            position.record_scalein(0.5, 44800.0, 0.0).unwrap();
            assert_eq!(position.scalein_count, 1);
            assert_eq!(position.open_quantity, 1.5);
        }
    }

    #[test]
    fn test_scale_out_with_aggressive_profile() {
        // Aggressive profil ile scale-out test
        let profile = PositionManagementProfile::Aggressive;
        
        let mut position = DynamicPosition::with_configs(
            "BTCUSDT".to_string(),
            45000.0,
            1.0,
            1.0,
            Some(44000.0),
            Some(60000.0),
            profile.trailing_stop_config(),
            profile.scale_in_config(),
            profile.scale_out_config(),
        );
        
        // Aggressive: 4 profit target [3%, 7%, 15%, 30%]
        assert_eq!(position.scaleout_config.profit_targets.len(), 4);
        
        // İlk target: 45000 * 1.03 = 46350
        let first_target = 45000.0 * 1.03;
        position.current_price = first_target + 100.0; // Target'ı geç
        
        let active_target = position.active_profit_target();
        assert_eq!(active_target, Some(3.0)); // %3 target aktif
        
        // Scale-out yap
        let qty = position.calculate_scaleout_quantity();
        position.record_scaleout(qty, position.current_price, 0.0).unwrap();
        
        assert_eq!(position.scaleout_count, 1);
        assert!(position.open_quantity < 1.0); // Bir kısmı kapatılmış
    }
}
