use memos_trading_core::strategy_lifecycle::StrategyLifecycleManager;
use memos_trading_core::auto_trading_engine::AutoTrading;
// auto_trading_engine_tests.rs
// AutoTradingEngine ve modüllerinin entegrasyon testi


use memos_trading_core::market_regime::SimpleRegimeDetector;
use memos_trading_core::strategy_lifecycle::SimpleStrategyLifecycleManager;
use memos_trading_core::risk_limits::SimpleRiskLimitManager;
use memos_trading_core::anomaly_analysis::SimpleAnomalyAnalyzer;
use memos_trading_core::health_dashboard::HealthDashboard;
use memos_trading_core::portfolio::Portfolio;
use memos_trading_core::types::StrategyParams;
use memos_trading_core::sim_data::generate_sample_candles;
use memos_trading_core::auto_trading_engine::AutoTradingEngine;

#[test]
fn test_auto_trading_engine_tick_with_sim_data() {
    let regime_detector = Box::new(SimpleRegimeDetector);
    let mut strategy_manager = Box::new(SimpleStrategyLifecycleManager {
        strategies: vec![],
        performances: vec![],
        min_win_rate: 0.5,
    });
    strategy_manager.register_strategy("trend_sma".to_string(), StrategyParams::default());
    let risk_manager = Box::new(SimpleRiskLimitManager {
        max_drawdown_pct: 10.0,
        max_loss_per_day: 1000.0,
        circuit_breaker_enabled: true,
        last_triggered: None,
    });
    let anomaly_analyzer = Box::new(SimpleAnomalyAnalyzer {
        last_latency_ms: 10.0,
        last_price_change: 0.5,
        latency_threshold: 100.0,
        price_spike_threshold: 5.0,
    });
    let health_dashboard = HealthDashboard::new();
    let portfolio = Portfolio::new(10000.0, None);
    let mut engine = AutoTradingEngine {
        last_tick: None,
        regime_detector,
        strategy_manager,
        risk_manager,
        anomaly_analyzer,
        health_dashboard,
        portfolio,
    };
    // Simülasyon verisi ile modülleri besle
    let candles = generate_sample_candles("BTCUSDT", "1m", 50);
    // Örnek: regime tespiti
    let _regime = engine.regime_detector.detect_regime(&candles);
    // Diğer modüller de benzer şekilde simülasyon verisiyle test edilebilir
    engine.tick(&candles);
    assert!(engine.last_tick.is_some());
}
