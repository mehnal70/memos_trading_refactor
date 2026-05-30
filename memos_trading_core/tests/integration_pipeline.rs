/// Tam trading pipeline integration testleri
/// WebSocket -> Validator -> PaperExecutor -> SafetyManager -> Dashboard

#[cfg(test)]
mod integration_tests {
    use memos_trading_core::robot::data_fetcher::parse_kline;
    use memos_trading_core::robot::order_management::paper_executor::PaperTradingExecutor;
    use memos_trading_core::robot::safety::{SafetyManager, SafetyRules, PaperTradingDashboard};
    use memos_trading_core::core::types::Trade;
    use chrono::Utc;

    /// Örnek test: Basit kline'ı parse et
    #[test]
    fn test_websocket_kline_parsing() {
        // WebSocket'ten mock kline oluştur ve parse et
        let kline_json = r#"{
            "e": "kline",
            "E": 1672531200000,
            "s": "BTCUSDT",
            "k": {
                "s": "BTCUSDT",
                "t": 1672531200000,
                "T": 1672531260000,
                "o": "16500.00",
                "h": "16700.00",
                "l": "16400.00",
                "c": "16600.00",
                "v": "100.50"
            }
        }"#;

        if let Ok(kline_update) = serde_json::from_str(kline_json) {
            let result = parse_kline(kline_update);
            assert!(result.is_ok());
            
            let candle = result.unwrap();
            assert_eq!(candle.symbol, "BTCUSDT");
            assert_eq!(candle.close, 16600.0);
            assert_eq!(candle.volume, 100.5);
        }
    }

    /// Tam flow: validator -> paper executor -> safety manager
    #[test]
    fn test_complete_trading_pipeline() {
        // 2. Paper Trading Executor
        let mut executor = PaperTradingExecutor::new(10000.0);
        
        // 3. Safety Manager
        let rules = SafetyRules::default();
        let mut safety = SafetyManager::new(10000.0, rules);
        
        // 4. Dashboard
        let mut dashboard = PaperTradingDashboard::new(10000.0);
        
        // Senaryoda birkaç işlem yap
        // İşlem 1: BTC satın al
        let buy_result = executor.buy("BTC", 50000.0, 0.1);
        assert!(buy_result.is_ok());
        
        // İşlem 2: Pozisyonu kapat (kar)
        let close_result = executor.close_position(55000.0); // %10 kar
        assert!(close_result.is_ok());
        
        if let Ok((trade, _)) = close_result {
            let pnl = trade.pnl.unwrap_or(0.0);
            assert!(pnl > 0.0); // Kar olmalı

            let safety_check = safety.check_trade_safety(&trade);
            assert!(safety_check.is_ok());

            dashboard.add_trade(trade);
        }
        
        // Dashboard metrikleri kontrol et
        assert!(dashboard.metrics().total_trades > 0);
        assert!(dashboard.metrics().win_rate > 0.0);
    }

    /// Loss senaryosu ve safety pause testi
    #[test]
    fn test_consecutive_losses_trigger_safety_pause() {
        let mut executor = PaperTradingExecutor::new(10000.0);
        let mut safety = SafetyManager::new(10000.0, SafetyRules {
            max_consecutive_losses: 2,
            ..Default::default()
        });
        
        // 3 ardışık kayıp oluştur
        for i in 0..3 {
            let _ = executor.buy("BTC", 50000.0, 0.1);
            if let Ok((close, _)) = executor.close_position(49500.0) {
                // Kayıp var
                if i < 2 {
                    // İlk 2 loss'ta safety check geçmeli
                    let check = safety.check_trade_safety(&close);
                    assert!(check.is_ok());
                } else {
                    // 3. loss'ta pause tetiklenmeliydi
                    let check = safety.check_trade_safety(&close);
                    assert!(check.is_ok()); // Ama pause aktifleşti
                    assert!(safety.metrics().is_paused);
                }
            }
        }
    }

    /// Drawdown kontrol testi
    #[test]
    fn test_drawdown_monitoring() {
        let initial_balance = 10000.0;
        let mut safety = SafetyManager::new(initial_balance, SafetyRules {
            max_drawdown_pct: 10.0,
            ..Default::default()
        });
        
        // Dummy trade oluştur
        let dummy_trade = Trade {
            id: None,
            symbol: "BTC".to_string(),
            entry_price: 100.0,
            exit_price: Some(100.0),
            amount: 1.0,
            entry_time: Utc::now(),
            exit_time: Some(Utc::now()),
            pnl: None,
            pnl_pct: None,
            strategy: "test".to_string(),
        };
        
        // %5 kayıp
        safety.update_balance(9500.0);
        assert!(safety.can_trade());
        assert!(safety.metrics().current_drawdown_pct > 0.0);
        
        // %15 kayıp -> circuit breaker tetikle
        safety.update_balance(8500.0);
        let check = safety.check_trade_safety(&dummy_trade);
        assert!(check.is_err()); // Drawdown aşıldı, circuit breaker tetiklendi
        assert!(!safety.can_trade());
        assert!(safety.metrics().circuit_breaker_triggered);
    }

    /// Dashboard JSON serialization testi
    #[test]
    fn test_dashboard_to_json_for_tauri() {
        let mut dashboard = PaperTradingDashboard::new(5000.0);
        
        // Sahte trade ekle
        let trade = Trade {
            id: None,
            symbol: "ETH".to_string(),
            entry_price: 2000.0,
            exit_price: Some(2200.0),
            amount: 1.0,
            entry_time: Utc::now(),
            exit_time: Some(Utc::now()),
            pnl: Some(200.0),
            pnl_pct: Some(10.0),
            strategy: "ma_crossover".to_string(),
        };
        
        dashboard.add_trade(trade);
        
        // JSON'a dönüştür (Tauri için)
        let json_data = dashboard.to_json_data();
        
        // Tauri iletişimi için gerekli alanlar
        assert_eq!(json_data.initial_balance, 5000.0);
        assert_eq!(json_data.total_trades, 1);
        assert_eq!(json_data.winning_trades, 1);
        assert!(json_data.last_trade_symbol.is_some());
        assert!(!json_data.updated_at.is_empty());
    }

    /// End-to-end: WebSocket → Execution → Safety → Dashboard
    #[test]
    fn test_end_to_end_trading_pipeline() {
        // 1. Executor
        let mut executor = PaperTradingExecutor::new(10000.0);
        
        // 2. Safety
        let mut safety = SafetyManager::new(10000.0, SafetyRules::default());
        
        // 3. Dashboard
        let mut dashboard = PaperTradingDashboard::new(10000.0);
        
        // Pipeline senaryosu:
        // - Executor'dan trade al
        // - Safety'den kontrol et
        // - Dashboard'a kaydet
        
        let _ = executor.buy("BTC", 50000.0, 0.1);
        
        if let Ok((buy_trade, _)) = executor.close_position(51000.0) {
            let safety_result = safety.check_trade_safety(&buy_trade);

            assert!(safety_result.is_ok());
            dashboard.add_trade(buy_trade);
        }
        
        // Final check
        assert!(!dashboard.trade_history().is_empty());
        assert!(dashboard.metrics().total_pnl > 0.0);
        assert!(safety.can_trade());
    }

    /// Stress test: Çok sayıda işlem
    #[test]
    fn test_stress_many_trades() {
        let mut executor = PaperTradingExecutor::new(10000.0);
        let mut dashboard = PaperTradingDashboard::new(10000.0);
        
        for i in 0..10 {
            let price = 50000.0 + (i as f64 * 100.0);
            
            let _ = executor.buy("BTC", price, 0.01);
            
            if let Ok((sell, _)) = executor.close_position(price + 500.0) {
                dashboard.add_trade(sell);
            }
        }
        
        assert!(!dashboard.trade_history().is_empty());
        let metrics = dashboard.metrics();
        assert!(metrics.total_trades > 0);
        println!("Stress test: {} trades, P&L: {:.2}%", 
            metrics.total_trades, metrics.total_pnl);
    }

    /// Error recovery testi
    #[test]
    fn test_safety_manager_recovery() {
        let initial_balance = 10000.0;
        let mut safety = SafetyManager::new(initial_balance, SafetyRules {
            max_drawdown_pct: 10.0,
            ..Default::default()
        });
        
        // Dummy trade oluştur
        let dummy_trade = Trade {
            id: None,
            symbol: "BTC".to_string(),
            entry_price: 100.0,
            exit_price: Some(100.0),
            amount: 1.0,
            entry_time: Utc::now(),
            exit_time: Some(Utc::now()),
            pnl: None,
            pnl_pct: None,
            strategy: "test".to_string(),
        };
        
        // Circuit breaker tetikle
        safety.update_balance(8500.0);
        let check = safety.check_trade_safety(&dummy_trade);
        assert!(check.is_err());
        assert!(safety.metrics().circuit_breaker_triggered);
        assert!(!safety.can_trade());
        
        // Reset et
        safety.reset_circuit_breaker();
        assert!(!safety.metrics().circuit_breaker_triggered);
        assert!(safety.can_trade());
    }
}
