use memos_trading_core::robot::StateManager;
// Entegrasyon Testi: RoboticLoop
use memos_trading_core::robot::robotic_loop::{RoboticLoop, RoboticLoopConfig, RunMode};
use memos_trading_core::robot::{RoboticTradeExecutor, InMemoryStateManager, UniversalReporter, FileLogger};
use memos_trading_core::types::{StrategyParams, RiskParams};

// ── Yardımcı: DummyFetcher ve DummyStrategy ──────────────────────────────────

struct DummyFetcher;
impl memos_trading_core::robot::LiveDataFetcher for DummyFetcher {
    fn supported_markets(&self) -> Vec<memos_trading_core::types::Market> { vec![] }
    fn supported_symbols(&self, _market: memos_trading_core::types::Market) -> Vec<String> { vec![] }
    fn source_name(&self) -> &str { "dummy" }
    fn fetch_latest<'life0, 'life1, 'life2, 'async_trait>(
        &'life0 self,
        _exchange: memos_trading_core::types::Exchange,
        _market: memos_trading_core::types::Market,
        _symbol: &'life1 str,
        _interval: &'life2 str,
        _limit: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<memos_trading_core::types::Candle>, memos_trading_core::MemosTradingError>> + Send + 'async_trait>>
    where 'life0: 'async_trait, 'life1: 'async_trait, 'life2: 'async_trait, Self: 'async_trait
    {
        Box::pin(async { Ok(vec![]) })
    }
}

struct DummyStrategy;
impl memos_trading_core::robot::Strategy for DummyStrategy {
    fn name(&self) -> &str { "dummy" }
    fn generate_signal(
        &self,
        _candles: &[memos_trading_core::types::Candle],
        _params: &StrategyParams,
        _funding: Option<&[memos_trading_core::types::FundingRatePoint]>,
        _htf_candles: Option<&[memos_trading_core::types::Candle]>,
    ) -> Result<memos_trading_core::types::Signal, memos_trading_core::MemosTradingError> {
        Ok(memos_trading_core::types::Signal::Hold)
    }
}

/// Geçerli `RoboticLoopConfig` döndüren yardımcı — tüm zorunlu alanları doldurur.
fn make_config(position_profile: &str) -> RoboticLoopConfig {
    RoboticLoopConfig {
        interval_secs: 1,
        trade_amount: Some(0.1),
        interval: "1m".to_string(),
        symbol: "BTCUSDT".to_string(),
        market: memos_trading_core::types::Market::Spot,
        strategy_params: StrategyParams::default(),
        candle_limit: 10,
        risk_params: RiskParams {
            stop_loss_pct: 2.0,
            take_profit_pct: 4.0,
            max_position_size_pct: Some(10.0),
            max_portfolio_risk_pct: Some(20.0),
            use_kelly_criterion: false,
            trailing_stop_pct: None,
        },
        capital: 10000.0,
        mode: RunMode::Live,
        autonomous_enabled: false,
        quality: memos_trading_core::robot::robotic_loop::TradeQualityConfig {
            min_rr: 1.5,
            volatility_min_pct: 0.1,
            volatility_max_pct: 5.0,
            trend_short_period: 5,
            trend_long_period: 20,
            trend_filter_enabled: true,
            trend_margin_pct: 0.5,
            adaptive_enabled: true,
            min_rr_tight: 2.0,
            min_rr_loose: 1.2,
            volatility_max_tight: 3.0,
            volatility_max_loose: 8.0,
            win_rate_low: 40.0,
            win_rate_high: 55.0,
            volume_filter_enabled: false,
            volume_min_ratio: 0.7,
            rsi_extreme_filter_enabled: false,
            rsi_extreme_ob: 80.0,
            rsi_extreme_os: 20.0,
            htf_require_alignment: false,
        },
        trade_quality_config_path: None,
        position_profile: Some(position_profile.to_string()),
        security_profile: Some("Development".to_string()),
        allows_short: false,
        initial_risk_policy: None,
        initial_cycle_id: 0,
        initial_brain: None,
        initial_population: None,
        use_ml_signal: false,
        initial_open_positions: std::collections::HashMap::new(),
        commission_pct: 0.0,
        execution_cost_config: None,
        live_state: None,
        sr_config: Default::default(),
        db_path: None,
        sl_cooldown_secs: None,
        breakeven_at_rr: None,
        atr_trail_mult: None,
        partial_tp_ratio: None,
        robotic_profiles_path: None,
        adaptive_params_path:  None,
        blocked_symbols:        Vec::new(),
        min_trade_interval_secs: None,
        scalp_swing:            None,
        max_open_positions:     None,
        max_spread_bps:         None,
        scorer_state_path:      None,
        classifier_state_path:  None,
    }
}

#[tokio::test]
async fn test_robotic_loop_basic() {
    let mut state = InMemoryStateManager::new();
    StateManager::set_symbols(&mut state, vec!["BTCUSDT".to_string()]).unwrap();
    StateManager::set_balance(&mut state, 10000.0).unwrap();
    let executor_state = InMemoryStateManager::new();
    let executor = RoboticTradeExecutor::with_state(
        &memos_trading_core::robot::DummyTradeExecutor,
        &executor_state,
        Some((0, 24)),
    );
    let reporter = UniversalReporter;
    let logger   = FileLogger::new("robotic_loop_test.log");

    let fetcher   = DummyFetcher;
    let strategy  = DummyStrategy;
    let mut loop_engine = RoboticLoop {
        executor:          &executor,
        state:             &mut state,
        reporter:          &reporter,
        logger:            &logger,
        config:            make_config("Balanced"),
        fetcher:           &fetcher,
        backtest_fetcher:  None,
        strategy:          &strategy,
        strategy_selector: None,
        ml_model:          None,
        ml_data:           None,
        portfolio:         None,
        autonomous_trader: None,
        monitor:           None,
        use_ml_signal:     false,
        paper_mode:        true,
        interval_cycle_ids: [0u64; 7],
        telegram:          None,
    };

    // Test sadece loop'un başlayıp çalıştığını doğrular
    // Sonsuz loop olduğu için timeout ile sınırlandırılır
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), loop_engine.start()).await;
    // Timeout olması beklenir (loop sonsuz), panic olmaması başarı sayılır
}

#[tokio::test]
async fn test_robotic_loop_profile_switching() {
    let mut state = InMemoryStateManager::new();
    StateManager::set_symbols(&mut state, vec!["BTCUSDT".to_string()]).unwrap();
    StateManager::set_balance(&mut state, 10000.0).unwrap();
    let executor_state = InMemoryStateManager::new();
    let executor = RoboticTradeExecutor::with_state(
        &memos_trading_core::robot::DummyTradeExecutor,
        &executor_state,
        Some((0, 24)),
    );
    let reporter = UniversalReporter;
    let logger   = FileLogger::new("profile_test.log");
    let fetcher   = DummyFetcher;
    let strategy  = DummyStrategy;

    let mut loop_engine = RoboticLoop {
        executor:          &executor,
        state:             &mut state,
        reporter:          &reporter,
        logger:            &logger,
        config:            make_config("Conservative"),
        fetcher:           &fetcher,
        backtest_fetcher:  None,
        strategy:          &strategy,
        strategy_selector: None,
        ml_model:          None,
        ml_data:           None,
        portfolio:         None,
        autonomous_trader: None,
        monitor:           None,
        use_ml_signal:     false,
        paper_mode:        true,
        interval_cycle_ids: [0u64; 7],
        telegram:          None,
    };

    // Başlangıç profilleri
    let (pos, sec) = loop_engine.get_profiles();
    assert_eq!(pos, Some("Conservative".to_string()));
    assert_eq!(sec, Some("Development".to_string()));

    // Profil değiştir
    loop_engine.set_position_profile("Aggressive");
    loop_engine.set_security_profile("Production");

    let (pos2, sec2) = loop_engine.get_profiles();
    assert_eq!(pos2, Some("Aggressive".to_string()));
    assert_eq!(sec2, Some("Production".to_string()));

    // Profile parse test
    let configs = loop_engine.parse_position_profile();
    assert!(configs.is_some());

    let (trailing, scale_in, scale_out) = configs.unwrap();
    assert_eq!(trailing.trailing_pct, 4.0);
    assert_eq!(scale_in.max_scalein_count, 4);
    assert_eq!(scale_out.profit_targets, vec![3.0, 7.0, 15.0, 30.0]);
}

// ── ML Pipeline Integration Tests ────────────────────────────────────────────

/// FeatureExtractor → normalize → LinearRegressor::predict uçtan uca zinciri.
/// Hiçbir adımda NaN/Inf üretilmemeli; score ve confidence sınırlar içinde olmalı.
#[test]
fn test_ml_pipeline_no_nan() {
    use memos_trading_core::robot::ml_engine::{FeatureExtractor, LinearRegressor};
    use memos_trading_core::types::Candle;
    use chrono::Utc;

    let mut price = 30_000.0f64;
    let candles: Vec<Candle> = (0..60).map(|i| {
        price += (i as f64 * 0.7) % 5.0 - 2.0;
        Candle {
            symbol: "BTCUSDT".into(), interval: "1h".into(),
            timestamp: Utc::now() + chrono::Duration::hours(i),
            open: price, high: price + 150.0, low: price - 80.0,
            close: price + 30.0, volume: 500.0 + i as f64 * 10.0,
        }
    }).collect();

    let fv   = FeatureExtractor::extract(&candles);
    let norm = fv.normalize();

    for (i, v) in norm.to_array().iter().enumerate() {
        assert!(v.is_finite(),             "feature[{}] NaN/Inf", i);
        assert!(*v >= 0.0 && *v <= 1.0,   "feature[{}]={} normalize dışı", i, v);
    }

    let model = LinearRegressor::with_defaults();
    let pred  = model.predict(&fv);
    assert!(pred.score.is_finite(),        "score NaN/Inf");
    assert!(pred.score >= -1.0 && pred.score <= 1.0, "score={} sınır dışı", pred.score);
    assert!(pred.confidence >= 0.0 && pred.confidence <= 1.0);
}

/// Düz (sıfır volatilite) piyasada NaN üretilmemeli.
#[test]
fn test_ml_pipeline_flat_market_no_nan() {
    use memos_trading_core::robot::ml_engine::{FeatureExtractor, LinearRegressor};
    use memos_trading_core::types::Candle;
    use chrono::Utc;

    let candles: Vec<Candle> = (0..50).map(|i| Candle {
        symbol: "TEST".into(), interval: "1h".into(),
        timestamp: Utc::now() + chrono::Duration::hours(i),
        open: 100.0, high: 100.0, low: 100.0, close: 100.0, volume: 1000.0,
    }).collect();

    let fv   = FeatureExtractor::extract(&candles);
    let norm = fv.normalize();
    for (i, v) in norm.to_array().iter().enumerate() {
        assert!(v.is_finite(), "düz piyasada feature[{}] NaN/Inf", i);
    }
    let pred = LinearRegressor::with_defaults().predict(&fv);
    assert!(pred.score.is_finite(), "düz piyasada score NaN/Inf");
}

// ── Backtester Integration Tests ─────────────────────────────────────────────

/// RSI stratejisi ile yükselen piyasada backtest:
/// en az 1 trade açılmalı, PnL sonlu olmalı.
#[test]
fn test_backtest_rsi_bull_market_pnl() {
    use memos_trading_core::robot::backtester::engine::{Backtester, BacktestConfig};
    use memos_trading_core::types::Candle;
    use chrono::Utc;

    let mut price = 100.0f64;
    let candles: Vec<Candle> = (0..100).map(|i| {
        price += if i < 20 { -0.5 } else { 2.0 };
        Candle {
            symbol: "BTC".into(), interval: "1h".into(),
            timestamp: Utc::now() + chrono::Duration::hours(i),
            open: price, high: price + 1.5, low: price - 0.5,
            close: price, volume: 1000.0,
        }
    }).collect();

    let cfg = BacktestConfig {
        symbol: "BTC".into(), interval: "1h".into(),
        initial_balance: 10_000.0, max_position_size: 1.0,
        take_profit_pct: 5.0, stop_loss_pct: 3.0,
        strategy_name: "RSI".into(),
        position_profile: None, security_profile: None,
        commission_pct: 0.001, strategy_params: None,
        breakeven_at_rr: None, atr_trail_mult: None, partial_tp_ratio: None,
    };

    let result = Backtester::new(cfg).run(&candles).expect("backtest hata vermemeli");
    assert!(result.total_trades > 0,      "en az 1 trade bekleniyor");
    assert!(result.total_pnl.is_finite(), "PnL NaN/Inf olmamalı");
}

/// optimize_position_management 100 kombinasyonu tarar;
/// en az 3 trade olan bir kombinasyon varsa sonuç döner ve skor mantıklı aralıkta.
#[test]
fn test_pos_opt_returns_valid_result() {
    use memos_trading_core::robot::backtester::engine::{Backtester, BacktestConfig};
    use memos_trading_core::types::Candle;
    use chrono::Utc;

    let mut price = 100.0f64;
    let candles: Vec<Candle> = (0..200).map(|i| {
        price += (i as f64 * 0.4) % 3.0 - 1.0;
        Candle {
            symbol: "BTC".into(), interval: "1h".into(),
            timestamp: Utc::now() + chrono::Duration::hours(i),
            open: price, high: price + 2.0, low: price - 1.0,
            close: price + 0.5, volume: 1000.0 + i as f64 * 5.0,
        }
    }).collect();

    let base = BacktestConfig {
        symbol: "BTC".into(), interval: "1h".into(),
        initial_balance: 10_000.0, max_position_size: 1.0,
        take_profit_pct: 5.0, stop_loss_pct: 2.0,
        strategy_name: "RSI".into(),
        position_profile: None, security_profile: None,
        commission_pct: 0.001, strategy_params: None,
        breakeven_at_rr: None, atr_trail_mult: None, partial_tp_ratio: None,
    };

    if let Some(best) = Backtester::optimize_position_management(&base, &candles) {
        assert!(best.score.is_finite(),    "skor NaN/Inf olmamalı");
        assert!(best.total_trades >= 3,    "en az 3 trade bekleniyor");
        assert!(best.win_rate >= 0.0 && best.win_rate <= 100.0);
        assert!(best.profit_factor >= 0.0, "profit_factor negatif olamaz");
    }
    // None dönmesi de kabul edilir (tüm kombinasyonlarda < 3 trade)
}
