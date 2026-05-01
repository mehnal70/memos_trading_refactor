// main_robotic.rs - Tam otomatik robotik trade başlatıcı
// BinanceLiveAdapter + BinanceTradeExecutor + MaCrossoverStrategy
// Paper mod için BINANCE_PAPER_MODE=true, canlı için false

use memos_trading_core::robot::{
    RoboticTradeExecutor, InMemoryStateManager, UniversalReporter, FileLogger,
    BinanceLiveAdapter, BinanceTradeExecutor,
    TradingLogger, StateManager,
    robotic_loop::{RoboticLoop, RoboticLoopConfig, RunMode, TradeQualityConfig},
};
use memos_trading_core::robot::strategies::MaCrossoverStrategy;
use memos_trading_core::types::{Market, RiskParams, StrategyParams};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::env;
use std::fs;
use serde::{Deserialize, Serialize};

const TRADE_QUALITY_CONFIG_PATH: &str = "config/trade_quality.json";
const PROFILE_CONFIG_PATH: &str = "config/robotic_profiles.json";

#[derive(Serialize, Deserialize, Clone)]
struct ProfileConfig {
    position_profile: String,
    security_profile: String,
    /// SL cooldown saniyesi — None → varsayılan 600 sn
    #[serde(default)]
    sl_cooldown_secs: Option<u64>,
    /// Breakeven R çarpanı — None → devre dışı
    #[serde(default)]
    breakeven_at_rr: Option<f64>,
    /// ATR trailing çarpanı — None → sabit trailing_pct kullanılır
    #[serde(default)]
    atr_trail_mult: Option<f64>,
    /// Kısmi TP oranı — None → tam kapat
    #[serde(default)]
    partial_tp_ratio: Option<f64>,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            position_profile: "Balanced".to_string(),
            security_profile: "Development".to_string(),
            sl_cooldown_secs: None,
            breakeven_at_rr:  None,
            atr_trail_mult:   None,
            partial_tp_ratio: None,
        }
    }
}

fn load_trade_quality_config() -> Option<TradeQualityConfig> {
    let content = fs::read_to_string(TRADE_QUALITY_CONFIG_PATH).ok()?;
    serde_json::from_str(&content).ok()
}

fn load_profile_config() -> ProfileConfig {
    let content = fs::read_to_string(PROFILE_CONFIG_PATH).ok();
    match content {
        Some(c) => serde_json::from_str(&c).unwrap_or_default(),
        None => ProfileConfig::default(),
    }
}

fn main() {
    let api_key    = env::var("BINANCE_API_KEY").unwrap_or_else(|_| "test_key".to_string());
    let api_secret = env::var("BINANCE_API_SECRET").unwrap_or_else(|_| "test_secret".to_string());
    let is_paper   = env::var("BINANCE_PAPER_MODE")
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(true);

    let symbol = env::var("TRADE_SYMBOL").unwrap_or_else(|_| "BTCUSDT".to_string());
    let market_str = env::var("TRADE_MARKET").unwrap_or_else(|_| "spot".to_string());
    let market = match market_str.to_lowercase().as_str() {
        "futures" => Market::Futures,
        "coinm"   => Market::Coinm,
        _         => Market::Spot,
    };
    let autonomous_enabled = env::var("AUTONOMOUS_ENABLED")
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(false);
    let use_ml_signal = env::var("USE_ML_SIGNAL")
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(false);
    let capital: f64 = env::var("TRADE_CAPITAL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000.0);
    let trade_amount: f64 = env::var("TRADE_AMOUNT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.01);
    let db_path = env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".to_string());

    println!(
        "[INIT] Robotic Trading — symbol={} market={:?} paper={} capital={} autonomous={} ml={}",
        symbol, market, is_paper, capital, autonomous_enabled, use_ml_signal
    );

    let trading_logger = TradingLogger::new(
        "logs/robotic_trader.log",
        "logs/trade_history.jsonl",
    ).expect("Logger oluşturulamadı");
    trading_logger.clear_logs().ok();

    let stop_signal  = Arc::new(AtomicBool::new(false));
    let pause_signal = Arc::new(AtomicBool::new(false));

    let mut state = InMemoryStateManager::new();
    state.set_symbols(vec![symbol.clone()]).unwrap();
    state.set_balance(capital).unwrap();

    let executor_state    = InMemoryStateManager::new();
    let binance_executor  = BinanceTradeExecutor::new(api_key, api_secret, is_paper);
    let robotic_executor  = RoboticTradeExecutor::with_state(&binance_executor, &executor_state, Some((0, 24)));

    let reporter = UniversalReporter;
    let logger   = FileLogger::new("robotic_trading.log");

    let quality_config = load_trade_quality_config().unwrap_or(TradeQualityConfig {
        min_rr: 1.2,
        volatility_min_pct: 0.05,
        volatility_max_pct: 3.0,
        trend_short_period: 20,
        trend_long_period: 50,
        trend_filter_enabled: true,
        trend_margin_pct: 0.5,
        adaptive_enabled: true,
        min_rr_tight: 1.5,
        min_rr_loose: 1.1,
        volatility_max_tight: 2.0,
        volatility_max_loose: 3.5,
        win_rate_low: 40.0,
        win_rate_high: 55.0,
        volume_filter_enabled: false,
        volume_min_ratio: 0.7,
        rsi_extreme_filter_enabled: false,
        rsi_extreme_ob: 80.0,
        rsi_extreme_os: 20.0,
        htf_require_alignment: false,
    });

    let profile_config = load_profile_config();
    println!("[CONFIG] Position Profile : {}", profile_config.position_profile);
    println!("[CONFIG] Security Profile : {}", profile_config.security_profile);

    let allows_short = matches!(market, Market::Futures | Market::Coinm);

    let config = RoboticLoopConfig {
        interval_secs: 10,
        trade_amount: Some(trade_amount),
        interval: "1m".to_string(),
        symbol: symbol.clone(),
        market,
        strategy_params: StrategyParams::default(),
        candle_limit: 100,
        risk_params: RiskParams {
            stop_loss_pct: 2.0,
            take_profit_pct: 4.0,
            max_position_size_pct: Some(10.0),
            max_portfolio_risk_pct: Some(20.0),
            use_kelly_criterion: false,
            trailing_stop_pct: None,
        },
        capital,
        mode: RunMode::Live,
        autonomous_enabled,
        quality: quality_config,
        trade_quality_config_path: Some(TRADE_QUALITY_CONFIG_PATH.to_string()),
        position_profile: Some(profile_config.position_profile),
        security_profile: Some(profile_config.security_profile),
        allows_short,
        initial_risk_policy: None,
        initial_cycle_id: 0,
        initial_brain: None,
        initial_population: None,
        initial_open_positions: Default::default(),
        use_ml_signal,
        commission_pct: match market {
            Market::Futures | Market::Coinm => 0.0004, // %0.04 taker
            _                               => 0.001,  // %0.10 spot taker
        },
        execution_cost_config: Some(match market {
            Market::Futures | Market::Coinm =>
                memos_trading_core::robot::order_management::paper_executor::ExecutionCostConfig::binance_futures(),
            _ =>
                memos_trading_core::robot::order_management::paper_executor::ExecutionCostConfig::binance_spot(),
        }),
        live_state: None, // headless mod — TUI bağlantısı yok
        sr_config: memos_trading_core::robot::sr_detector::SrDetectorConfig::default(),
        db_path: Some(db_path),
        sl_cooldown_secs:      profile_config.sl_cooldown_secs,
        breakeven_at_rr:       profile_config.breakeven_at_rr,
        atr_trail_mult:        profile_config.atr_trail_mult,
        partial_tp_ratio:      profile_config.partial_tp_ratio,
        robotic_profiles_path:    Some(PROFILE_CONFIG_PATH.to_string()),
        adaptive_params_path:     Some("config/adaptive_params.json".to_string()),
        blocked_symbols:          vec![],
        min_trade_interval_secs:  None,
        max_open_positions:       std::env::var("MAX_OPEN_POSITIONS").ok().and_then(|v| v.parse().ok()),
        max_spread_bps:           std::env::var("MAX_SPREAD_BPS").ok().and_then(|v| v.parse().ok()),
        scorer_state_path:        Some("config/strategy_scorer_state.json".to_string()),
        classifier_state_path:    Some("config/classifier_state.json".to_string()),
        scalp_swing: {
            let mut sc = memos_trading_core::robot::scalp_swing::ScalpSwingConfig::default();
            // SCALP_PAPER_MODE=true → scalp'i devre dışı bırak, yalnızca Swing ve REG çalışsın.
            // Sermayeyi korumak için akıllıca geçiş: sadece scalp kapatılır, diğerleri çalışmaya devam eder.
            if std::env::var("SCALP_PAPER_MODE").map(|v| v == "true").unwrap_or(false) {
                sc.scalp_enabled = false;
                println!("[CONFIG] SCALP_PAPER_MODE=true → Scalp motoru devre dışı. Sadece Swing + REG aktif.");
            }
            Some(sc)
        },
    };

    let live_fetcher = BinanceLiveAdapter::new(stop_signal.clone(), pause_signal.clone());
    let strategy     = MaCrossoverStrategy;

    let mut loop_engine = RoboticLoop {
        executor: &robotic_executor,
        state: &mut state,
        reporter: &reporter,
        logger: &logger,
        config,
        fetcher: &live_fetcher,
        backtest_fetcher: None,
        strategy: &strategy,
        strategy_selector: None,
        ml_model: None,
        ml_data: None,
        portfolio: None,
        monitor: None,
        autonomous_trader: None,
        use_ml_signal,
        paper_mode: is_paper,
        interval_cycle_ids: [0u64; 7],
        telegram: None,
    };

    println!("[START] Trading loop başlatılıyor — Ctrl+C ile durdur");

    let rt = tokio::runtime::Runtime::new().expect("Tokio runtime oluşturulamadı");
    rt.block_on(async {
        // Ctrl+C gelince stop_signal'ı set et
        let stop = stop_signal.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                println!("\n[STOP] Ctrl+C alındı, döngü durduruluyor...");
                stop.store(true, Ordering::Relaxed);
            }
        });

        loop_engine.start().await;
    });

    println!("[EXIT] Trading loop sonlandı.");
}
