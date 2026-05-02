// rtc_cli.rs — Gerçek Otonom Sistem TUI CLI
// RoboticLoop::start() + BinanceLiveAdapter + MaCrossoverStrategy + SharedLogger
// FSM durumu SharedLogger aracılığıyla TUI'ya yansıtılır; stop/pause AtomicBool ile kontrol edilir.

// ─── Alt Modüller ────────────────────────────────────────────────────────────
// rtc_cli bin'i `src/bin/rtc_cli/main.rs` formatında çoklu dosyaya bölünmüştür.
// pipeline: D→B→ML→P5 zincirleme pipeline orchestration worker'ı.
mod pipeline;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use memos_trading_core::{
    robot::{
        AutonomousController, AutonomousControllerConfig as AutonomousConfig, AutonomousState,
        RiskGate, RiskGatePolicy, RecoverySupervisor,
        BinanceLiveAdapter,
        BinanceTradeExecutor,
        DummyTradeExecutor,
        ErrorLogger,
        InMemoryStateManager,
        RoboticTradeExecutor,
        StateManager,
        UniversalReporter,
        Backtester, BacktestConfig,
        robotic_loop::{RoboticLoop, RoboticLoopConfig, RunMode, TradeQualityConfig, LiveRiskMap, LivePriceData, LivePositionMap, LiveEvolutionStatus, LiveSignalCounts, live_pos_key},
        strategies::MaCrossoverStrategy,
        strategies::{
            RsiStrategy, MacdStrategy, BollingerStrategy, SupertrendStrategy,
            EmaCrossoverStrategy, StochasticRsiStrategy, CciStrategy,
            PriceActionStrategy, IctFvgStrategy, SmcStrategy,
            IctOrderBlockStrategy, IctLiquiditySweepStrategy, IctKillzoneStrategy,
            IctOteStrategy, IctCompositeStrategy,
        },
        optimizer::{HyperOptimizer, rank_strategies_for_interval},
        ml_engine::{MLSignalPredictor, FeatureExtractor, LinearRegressor, GradientBoostedTrees, gbt_grid_search},
        HyperOpt,
        symbol_orchestrator::{SymbolOrchestrator, pos_pnl, pos_pnl_pct},
        advanced_risk::MonteCarloSimulator,
        backtester::walk_forward::{WalkForwardTester, WalkForwardConfig},
    },
    types::{Exchange, Market, RiskParams, StrategyParams, Candle},
    evolution::{AdaptiveBrain, PopulationManager},
    database_reader, database_writer,
};

// ── Strateji Doğrulama Sonuçları ─────────────────────────────────────────────
/// ML worker her eğitimde Monte Carlo + Walk-Forward çalıştırır;
/// sonuçlar AppState'e yazılır, Tab 3 ve Tab 2 köprüsü okur.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ValidationResult {
    // ── Monte Carlo (2000 simülasyon) ─────────────────────────────────────
    pub mc_n_sims:        usize,
    pub mc_n_trades:      usize,
    pub mc_ruin_pct:      f64,    // 0–100: %50+ kayıp riski
    pub mc_p5_balance:    f64,    // En kötü %5 senaryo bakiyesi
    pub mc_p50_balance:   f64,    // Medyan senaryo
    pub mc_p95_balance:   f64,    // En iyi %5 senaryo
    pub mc_max_dd_p50:    f64,    // Medyan max drawdown (%)
    pub mc_max_dd_p95:    f64,    // Kötü senaryo max drawdown (%)
    pub mc_positive_pct:  f64,    // Kârlı biten simülasyon oranı (%)
    pub mc_expected_ret:  f64,    // Medyan beklenen getiri (%)

    // ── Walk-Forward (kayan pencere OOS) ──────────────────────────────────
    pub wf_windows:       usize,
    pub wf_profitable:    usize,
    pub wf_consistency:   f64,    // 0–1: karlı pencere oranı
    pub wf_avg_oos_wr:    f64,    // Ortalama OOS kazanma oranı (%)
    pub wf_avg_oos_pnl:   f64,    // Ortalama OOS PnL (%)
    pub wf_avg_oos_pf:    f64,    // Ortalama OOS profit factor
    pub wf_avg_oos_dd:    f64,    // Ortalama OOS max drawdown (%)
    pub wf_avg_oos_sharpe:f64,    // Ortalama OOS Sharpe

    // ── Bileşik skor ve zaman damgası ─────────────────────────────────────
    pub composite_score:  f64,    // 0–100: genel strateji sağlığı
    pub risk_level:       RiskLevel,
    pub computed_at:      String, // "HH:MM" formatı
    pub strategy_name:    String, // hangi strateji üzerinde çalışıldı
    #[allow(dead_code)]
    pub symbol:           String, // "BTCUSDT/1h" formatı — log ve export için
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
enum RiskLevel {
    #[default]
    Unknown,
    Low,
    Moderate,
    High,
    Critical,
}

// ── p5_crypto Python analizi sonuçları ───────────────────────────────────────
/// Full analiz sonuçları — `status.json` + `*_results.json` birleşimi.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
struct P5TopStrategy {
    pub name:      String,
    pub direction: String,
    pub wr:        f64,
    pub pf:        f64,
    pub dd:        f64,
    pub wf_pass:   u32,
    pub edge:      String,
    pub tp_mult:   f64,
    pub sl_mult:   f64,
    pub p_value:   f64,
}

#[derive(Debug, Clone, Default)]
struct P5Status {
    // Durum (status.json'dan)
    pub state:              String,   // "running"|"done"|"error"|"scanning"
    pub msg:                String,
    pub ts:                 String,
    pub symbol:             String,
    pub interval:           String,
    // Özet (full results JSON'dan)
    pub strategies_found:   u32,
    pub edge_confirmed:     u32,
    pub wf_consistency:     f64,
    pub best_name:          String,
    pub best_wr:            f64,
    pub best_pf:            f64,
    pub best_dd:            f64,
    pub best_tp_mult:       f64,
    pub best_sl_mult:       f64,
    pub best_edge:          String,
    pub best_p_value:       f64,
    pub mc_prob_profit:     f64,
    pub ruin_pct:           f64,
    pub active_signals:     u32,
    pub active_dir:         String,  // "long"|"short"|""
    pub active_atr:         f64,
    // Scanning ilerleme
    pub tested:             u32,
    pub found_so_far:       u32,
    // Top stratejiler (max 3)
    pub top_strategies:     Vec<P5TopStrategy>,
}

// ── Otonom Pipeline ──────────────────────────────────────────────────────────
/// D → B → ML → P5 hattının anlık aşaması.
#[derive(Debug, Clone, PartialEq)]
enum PipelinePhase {
    Idle,
    Download,
    Backtest,
    MLTrain,
    P5Analysis,
    Done,
}

impl Default for PipelinePhase {
    fn default() -> Self { PipelinePhase::Idle }
}

impl PipelinePhase {
    fn label(&self) -> &'static str {
        match self {
            PipelinePhase::Idle       => "Bekliyor",
            PipelinePhase::Download   => "İndirme",
            PipelinePhase::Backtest   => "Backtest",
            PipelinePhase::MLTrain    => "ML Eğitim",
            PipelinePhase::P5Analysis => "P5 Analiz",
            PipelinePhase::Done       => "Tamamlandı",
        }
    }
    fn icon(&self) -> &'static str {
        match self {
            PipelinePhase::Idle       => "⏸",
            PipelinePhase::Download   => "⬇",
            PipelinePhase::Backtest   => "🔬",
            PipelinePhase::MLTrain    => "🧠",
            PipelinePhase::P5Analysis => "🐍",
            PipelinePhase::Done       => "✓",
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PipelineStatus {
    pub phase:            PipelinePhase,
    pub phase_started_at: Option<std::time::Instant>,
    pub last_run_at:      Option<String>,    // insan okunur zaman damgası
    pub runs_completed:   u32,
    pub trigger:          Arc<AtomicBool>,   // [P] tuşu / programatik
    pub enabled:          bool,
    pub every_mins:       u64,               // tekrar aralığı
    pub p5_top_n:         usize,             // kaç sembol için p5 çalıştır
    pub p5_sym_idx:       usize,             // şu an işlenen sembol sırası
    pub p5_symbols:       Vec<(String,String,String,String)>, // (exch,mkt,sym,intv)
    pub next_run_at:      std::time::Instant,
    pub last_error:       Option<String>,
}

impl Default for PipelineStatus {
    fn default() -> Self {
        Self {
            phase:            PipelinePhase::Idle,
            phase_started_at: None,
            last_run_at:      None,
            runs_completed:   0,
            trigger:          Arc::new(AtomicBool::new(false)),
            enabled:          true,
            every_mins:       120,
            p5_top_n:         3,
            p5_sym_idx:       0,
            p5_symbols:       Vec::new(),
            next_run_at:      std::time::Instant::now() + std::time::Duration::from_secs(15),
            last_error:       None,
        }
    }
}

// ─── MTF Fırsat Tarayıcısı ───────────────────────────────────────────────────
/// Arka planda tüm sembol × interval kombinasyonlarını tarayan scanner'ın
/// bulduğu yüksek güvenilirlikli fırsat kaydı.
#[derive(Debug, Clone)]
struct MtfOpportunity {
    /// Sembol (örn. "BTCUSDT")
    symbol:    String,
    /// Tarama yapılan interval (örn. "5m", "1h")
    interval:  String,
    /// En yüksek skoru alan strateji adı
    strategy:  String,
    /// Composite score (0.0 – 1.0+)
    score:     f64,
    /// Kazanma oranı (0.0 – 1.0)
    win_rate:  f64,
    /// İşlem yönü tahmini: "LONG" | "SHORT" | "-"
    direction: String,
    /// Bulunma zamanı (HH:MM)
    found_at:  String,
    /// Canlı sinyal: "BUY" | "SELL" | "-" (price monitor worker günceller)
    live_signal: String,
    /// Sinyal fiyatı
    signal_price: f64,
    /// Sinyal zamanı (HH:MM:SS)
    signal_at: Option<String>,
}

impl ValidationResult {
    /// MC + WF metriklerinden bileşik sağlık skoru hesapla (0–100)
    fn compute_composite(&mut self) {
        if self.mc_n_trades == 0 { self.composite_score = 0.0; return; }

        // Bileşenler (her biri 0–1 aralığında normalize)
        let ruin_score     = (1.0 - self.mc_ruin_pct / 100.0).max(0.0);          // Düşük ruin = iyi
        let positive_score = self.mc_positive_pct / 100.0;                        // Yüksek kârlı sim = iyi
        let consistency    = self.wf_consistency;                                  // Yüksek tutarlılık = iyi
        let oos_wr_score   = (self.wf_avg_oos_wr / 60.0).min(1.0).max(0.0);      // WinRate 60%+ = tam puan
        let dd_score       = (1.0 - self.mc_max_dd_p50 / 30.0).max(0.0);         // DD 30%+ = sıfır puan

        // Ağırlıklı ortalama
        let score = ruin_score * 0.30
            + positive_score * 0.20
            + consistency    * 0.25
            + oos_wr_score   * 0.15
            + dd_score       * 0.10;
        self.composite_score = (score * 100.0).clamp(0.0, 100.0);

        self.risk_level = if self.mc_ruin_pct >= 20.0 || self.composite_score < 30.0 {
            RiskLevel::Critical
        } else if self.mc_ruin_pct >= 10.0 || self.composite_score < 45.0 {
            RiskLevel::High
        } else if self.mc_ruin_pct >= 5.0 || self.composite_score < 60.0 {
            RiskLevel::Moderate
        } else {
            RiskLevel::Low
        };
    }
}
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph, Tabs},
    Terminal,
};
use std::{
    collections::VecDeque,
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        {Arc, Mutex},
    },
    time::{Duration, Instant},
};

// ─── Otonom Konfigürasyon ───────────────────────────────────────────────────

/// Hyperopt/backtest'in bulduğu en iyi parametreler — JSON'a persist edilir,
/// restart'ta kalır. Her ML döngüsü güncelleyebilir.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct OptimizedParamsCache {
    pub ma_fast:        usize,
    pub ma_slow:        usize,
    pub rsi_period:     usize,
    pub rsi_ob:         f64,
    pub rsi_os:         f64,
    pub bb_period:      usize,
    pub bb_std_dev:     f64,
    pub macd_fast:      usize,
    pub macd_slow:      usize,
    pub macd_signal:    usize,
    pub stoch_k:        usize,
    pub stoch_ob:       f64,
    pub stoch_os:       f64,
    #[serde(default)]
    pub ema_fast:          usize,
    #[serde(default)]
    pub ema_slow:          usize,
    #[serde(default)]
    pub donchian_period:   usize,
    #[serde(default)]
    pub williams_period:   usize,
    #[serde(default)]
    pub cci_period:        usize,
    #[serde(default)]
    pub stoch_rsi_period:  usize,
    #[serde(default)]
    pub supertrend_period: usize,
    #[serde(default)]
    pub supertrend_mult:   f64,
    #[serde(default)]
    pub ict_fvg_lookback:  usize,
    #[serde(default)]
    pub smc_swing_lb:      usize,
    /// Backtest sıralamasında birinci çıkan strateji adı
    pub best_strategy:  Option<String>,
    pub last_updated:   Option<String>,
}

impl Default for OptimizedParamsCache {
    fn default() -> Self {
        Self {
            ma_fast: 5, ma_slow: 20,
            rsi_period: 14, rsi_ob: 70.0, rsi_os: 30.0,
            bb_period: 20, bb_std_dev: 2.0,
            macd_fast: 12, macd_slow: 26, macd_signal: 9,
            stoch_k: 14, stoch_ob: 80.0, stoch_os: 20.0,
            ema_fast: 5, ema_slow: 20,
            donchian_period: 20, williams_period: 14, cci_period: 20,
            stoch_rsi_period: 14, supertrend_period: 10, supertrend_mult: 3.0,
            ict_fvg_lookback: 5, smc_swing_lb: 10,
            best_strategy: None, last_updated: None,
        }
    }
}

/// Saat bazlı işlem filtresi — hangi saat dilimlerinde işlem açılabilir.
/// DB analizine göre: 10-12 arası long için en verimli, 08:00 kısa-yön eğilimli, 17-18 yüksek vol ama yönsüz.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SessionFilterConfig {
    /// Filtre etkin mi? (false = tüm saatler açık)
    #[serde(default)]
    pub enabled: bool,
    /// İzin verilen saatler (UTC). Boş = tüm saatler. Örn: [8,9,10,11,12,13,14,15,16]
    #[serde(default)]
    pub allowed_hours: Vec<u8>,
    /// Bu saatlerde işlem açma. Örn: [8] — Avrupa açılış, kısa yönlü baskı
    #[serde(default)]
    pub blocked_hours: Vec<u8>,
    /// Bu saatlerde yalnızca BUY sinyali işleme (long tercihli saatler). Örn: [3,4,10,11,12]
    #[serde(default)]
    pub long_preferred_hours: Vec<u8>,
}

impl Default for SessionFilterConfig {
    fn default() -> Self {
        // DB analizinden türetilmiş varsayılanlar:
        // 10-12 UTC: %65 yukarı, uzun pozisyon dostu
        // 03-04 UTC: %59-65 yukarı, Asya kapanış güçlü
        // 08 UTC: Avrupa açılışı %30 yukarı → short yönlü
        // 17-18 UTC: US açılış, yüksek vol ama %43-44 yön → riskli
        Self {
            enabled: false, // varsayılan kapalı, config'de açılabilir
            allowed_hours: vec![],
            blocked_hours: vec![],
            long_preferred_hours: vec![3, 4, 10, 11, 12],
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct OtoConfig {
    exchange:               String,
    market:                 String,
    symbol:                 String,
    interval:               String,
    db_path:                String,
    capital:                f64,
    backtest_enabled:       bool,
    backtest_every_mins:    u64,
    backtest_candle_limit:  usize,
    trade_amount:           f64,
    // Otonom veri indirme
    #[serde(default = "default_true")]
    download_enabled:       bool,
    #[serde(default = "default_download_mins")]
    download_every_mins:    u64,    // kaç dakikada bir indir
    #[serde(default = "default_download_limit")]
    download_candle_limit:  usize,  // her indirmede kaç mum
    #[serde(default = "default_download_top_n")]
    download_top_n:         usize,  // En iyi N sembol için de indir
    #[serde(default = "default_export_mins")]
    auto_export_every_mins: u64,    // otomatik export aralığı (0 = devre dışı)
    #[serde(default = "default_export_keep")]
    auto_export_keep:       usize,  // saklanacak maksimum export dosyası sayısı
    // ── Yol yapılandırması ────────────────────────────────────────────────────
    #[serde(default = "default_trade_quality_path")]
    trade_quality_config_path: String,
    #[serde(default = "default_adaptive_params_path")]
    adaptive_params_path:      String,
    #[serde(default = "default_robotic_profiles_path")]
    robotic_profiles_path:     String,
    #[serde(default = "default_evolution_state_path")]
    evolution_state_path:      String,
    #[serde(default = "default_fsm_state_path")]
    fsm_state_path:            String,
    #[serde(default = "default_app_snapshot_path")]
    app_snapshot_path:         String,
    // ── Kaldıraç aralığı ─────────────────────────────────────────────────────
    #[serde(default = "default_leverage_base")]
    leverage_base:             f64,   // Minimum kaldıraç (varsayılan: 7x)
    #[serde(default = "default_leverage_max")]
    leverage_max:              f64,   // Maksimum kaldıraç (varsayılan: 10x)
    // ── Optimize edilmiş parametre önbelleği (persist) ────────────────────────
    #[serde(default)]
    optimized_params:          OptimizedParamsCache,
    // ── Seans/saat filtresi ───────────────────────────────────────────────────
    #[serde(default)]
    session_filter:            SessionFilterConfig,
    // ── Kalıcı sembol engelleme listesi ──────────────────────────────────────
    // Bu listeki semboller kesinlikle işlem açılmaz (ör: sürekli zararlı semboller).
    // Örn: ["ETHUSDT", "XRPUSDT"]
    #[serde(default)]
    blocked_symbols:           Vec<String>,
    // ── Sabitlenmiş (pinned) sembol listesi ──────────────────────────────────
    // Bu semboller skor/filtre sonucundan bağımsız olarak:
    //   • MTF scanner'a her zaman dahil edilir
    //   • Orchestrator worker top-N listesine her zaman eklenir (capacity varsa)
    //   • blocked_symbols içinde olmadıkları sürece her zaman izlenir
    // Örn: ["BTCUSDT", "ETHUSDT"]
    #[serde(default)]
    pinned_symbols:            Vec<String>,
    // ── Otonom Pipeline (D→B→ML→P5) ─────────────────────────────────────────
    #[serde(default = "default_true")]
    pipeline_enabled:          bool,     // false = tamamen devre dışı
    #[serde(default = "default_pipeline_mins")]
    pipeline_every_mins:       u64,      // periyodik tekrar aralığı (dk)
    #[serde(default = "default_pipeline_p5_top_n")]
    pipeline_p5_top_n:         usize,    // kaç sembol için p5 analizi çalıştır
    // ── Interval / HTF Filtre kalıcılığı ─────────────────────────────────────
    #[serde(default)]
    auto_interval:             bool,     // otomatik interval geçişi — settings item 10
}

/// robotic_profiles.json'dan okunan pozisyon yönetimi parametreleri.
/// Tüm alanlar opsiyonel — dosyada bulunmayan alanlar varsayılan None alır.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct ProfileConfig {
    #[serde(default)]
    position_profile: String,
    #[serde(default)]
    security_profile: String,
    #[serde(default)]
    sl_cooldown_secs: Option<u64>,
    #[serde(default)]
    breakeven_at_rr: Option<f64>,
    #[serde(default)]
    atr_trail_mult: Option<f64>,
    #[serde(default)]
    partial_tp_ratio: Option<f64>,
}

fn load_profile_config(path: &str) -> ProfileConfig {
    std::fs::read_to_string(path).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_profile_config(path: &str, st: &AppState) {
    let prof = ProfileConfig {
        position_profile: String::new(),
        security_profile: String::new(),
        sl_cooldown_secs: Some(st.pos_sl_cooldown),
        breakeven_at_rr:  st.pos_breakeven_at_rr,
        atr_trail_mult:   st.pos_atr_trail_mult,
        partial_tp_ratio: st.pos_partial_tp_ratio,
    };
    if let Ok(s) = serde_json::to_string_pretty(&prof) {
        let _ = std::fs::write(path, s);
    }
}

fn default_true()                  -> bool   { true }
fn default_download_mins()         -> u64    { 15 }
fn default_download_limit()        -> usize  { 500 }
fn default_download_top_n()        -> usize  { 3 }
fn default_export_mins()           -> u64    { 30 }
fn default_export_keep()           -> usize  { 24 }
fn default_trade_quality_path()    -> String { "config/trade_quality.json".into() }
fn default_adaptive_params_path()  -> String { "config/adaptive_params.json".into() }
fn default_robotic_profiles_path() -> String { "config/robotic_profiles.json".into() }
fn default_evolution_state_path()  -> String { "config/evolution_state.json".into() }
fn default_fsm_state_path()        -> String { "config/fsm_state.json".into() }
fn default_app_snapshot_path()     -> String { "config/app_snapshot.json".into() }
fn default_leverage_base()         -> f64    { 7.0 }
fn default_leverage_max()          -> f64    { 10.0 }
fn default_screener_min_vol()      -> f64    { 5.0 }
fn default_screener_min_chg()      -> f64    { 2.0 }
fn default_screener_max_new()      -> usize  { 8 }
fn default_screener_interval_hours() -> f64 { 4.0 }
fn default_pipeline_mins()           -> u64  { 120 }  // 2 saatte bir
fn default_pipeline_p5_top_n()       -> usize { 3 }   // en iyi 3 sembol

impl Default for OtoConfig {
    fn default() -> Self {
        Self {
            exchange:              "binance".into(),
            market:               "futures".into(),
            symbol:               "BTCUSDT".into(),
            interval:             "1m".into(),
            db_path:              "data/trader.db".into(),
            capital:              10_000.0,
            backtest_enabled:     true,
            backtest_every_mins:  60,
            backtest_candle_limit: 1000,
            trade_amount:         0.01,
            download_enabled:     true,
            download_every_mins:  15,
            download_candle_limit: 500,
            download_top_n:       3,
            auto_export_every_mins: 30,
            auto_export_keep:       24,
            trade_quality_config_path: default_trade_quality_path(),
            adaptive_params_path:      default_adaptive_params_path(),
            robotic_profiles_path:     default_robotic_profiles_path(),
            evolution_state_path:      default_evolution_state_path(),
            fsm_state_path:            default_fsm_state_path(),
            app_snapshot_path:         default_app_snapshot_path(),
            leverage_base:             default_leverage_base(),
            leverage_max:              default_leverage_max(),
            optimized_params:          OptimizedParamsCache::default(),
            session_filter:            SessionFilterConfig::default(),
            blocked_symbols:           Vec::new(),
            pinned_symbols:            Vec::new(),
            pipeline_enabled:          true,
            pipeline_every_mins:       120,
            pipeline_p5_top_n:         3,
            auto_interval:             false,
        }
    }
}

/// Optimize edilmiş parametreleri rtc_config.json'a yazar — restart'ta kaybolmaz.
/// Sadece `optimized_params` alanını günceller, diğer alanları korur.
fn persist_optimized_params(cache: OptimizedParamsCache) {
    let paths = ["config/rtc_config.json", "../config/rtc_config.json"];
    for path in &paths {
        if let Ok(txt) = std::fs::read_to_string(path) {
            if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&txt) {
                if let Ok(cache_val) = serde_json::to_value(&cache) {
                    val["optimized_params"] = cache_val;
                    if let Ok(out) = serde_json::to_string_pretty(&val) {
                        let _ = std::fs::write(path, out);
                    }
                }
                return;
            }
        }
    }
}

/// Göreceli db_path'i exe konumundan workspace köküne göre mutlak yola çevirir.
/// Örn: binary = target/debug/rtc_cli → ancestors().nth(4) = workspace kökü
fn resolve_db_path(mut cfg: OtoConfig) -> OtoConfig {
    let p = std::path::Path::new(&cfg.db_path);
    if p.is_absolute() && p.exists() { return cfg; }
    // Önce mevcut dizinden dene (workspace kökünden çalışılıyorsa)
    if p.exists() { return cfg; }
    // Exe konumundan workspace köküne git: target/debug/rtc_cli → 4 üst
    if let Ok(exe) = std::env::current_exe() {
        for n in 3..=6 {
            if let Some(root) = exe.ancestors().nth(n) {
                let candidate = root.join(&cfg.db_path);
                if candidate.exists() {
                    cfg.db_path = candidate.to_string_lossy().into_owned();
                    return cfg;
                }
            }
        }
    }
    cfg
}

fn load_oto_config() -> OtoConfig {
    let paths = ["config/rtc_config.json", "../config/rtc_config.json"];
    for path in &paths {
        if let Ok(txt) = std::fs::read_to_string(path) {
            if let Ok(cfg) = serde_json::from_str::<OtoConfig>(&txt) {
                return resolve_db_path(cfg);
            }
        }
    }
    resolve_db_path(OtoConfig::default())
}

fn save_oto_config(cfg: &OtoConfig) {
    let paths = ["config/rtc_config.json", "../config/rtc_config.json"];
    // Mevcut config dosyasının nerede olduğunu bul
    let target = paths.iter()
        .find(|p| std::path::Path::new(p).exists())
        .copied()
        .unwrap_or("config/rtc_config.json");
    if let Ok(json) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(target, json);
    }
}

/// Tüm stratejileri iki boyutlu skorla + interval tümdengeline göre karşılaştırır:
///   - Tarihi backtest (500 mum): uzun vadeli tutarlılık
///   - HyperOpt mini-backtest (son 60 mum): mevcut piyasa koşullarına uyum
///   - Interval ağırlığı: 5m'de Momentum grubu, 1h'de Trend grubu, 4h'de Yapısal grup öne çıkar
/// Kombinasyon: (bt_score × 0.4 + hyperopt_score × 0.6) × interval_weight
fn compare_strategies(candles: &[Candle], capital: f64, trade_amount: f64, sl: f64, tp: f64, interval: &str) -> String {
    use memos_trading_core::strategies::Strategy;

    // (kısa_ad, backtest_adı, strateji_nesnesi, default_params)
    let default_p = StrategyParams {
        period: Some(14), overbought: Some(70.0), oversold: Some(30.0),
        fast: Some(9), slow: Some(21), fast_period: Some(12),
        slow_period: Some(26), signal_period: Some(9),
        std_dev: Some(2.0), bb_period: Some(20),
    };
    let strats: Vec<(&str, &str, Box<dyn Strategy>)> = vec![
        ("RSI",       "RSI",        Box::new(RsiStrategy)),
        ("MACD",      "MACD",       Box::new(MacdStrategy)),
        ("BB",        "BOLLINGER",  Box::new(BollingerStrategy)),
        ("SUPERTREND","SUPERTREND", Box::new(SupertrendStrategy)),
        ("EMA",       "EMA",        Box::new(EmaCrossoverStrategy)),
        ("STOCH_RSI",    "STOCH_RSI",    Box::new(StochasticRsiStrategy)),
        ("CCI",          "CCI",          Box::new(CciStrategy)),
        ("MA",           "MA_CROSSOVER", Box::new(memos_trading_core::robot::strategies::MaCrossoverStrategy)),
        ("PRICE_ACTION", "PRICE_ACTION", Box::new(PriceActionStrategy)),
        ("ICT_FVG",        "ICT_FVG",        Box::new(IctFvgStrategy)),
        ("SMC",            "SMC",            Box::new(SmcStrategy)),
        ("ICT_OB",         "ICT_OB",         Box::new(IctOrderBlockStrategy)),
        ("ICT_SWEEP",      "ICT_SWEEP",      Box::new(IctLiquiditySweepStrategy)),
        ("ICT_KILLZONE",   "ICT_KILLZONE",   Box::new(IctKillzoneStrategy)),
        ("ICT_OTE",        "ICT_OTE",        Box::new(IctOteStrategy)),
        ("ICT_COMPOSITE",  "ICT_COMPOSITE",  Box::new(IctCompositeStrategy)),
    ];

    let mut best_name  = String::new();
    let mut best_score = f64::MIN;

    for (short_name, bt_name, strat_obj) in &strats {
        // ── 1. Tarihi backtest skoru (uzun vade) ─────────────────
        let bt_score = {
            let cfg = BacktestConfig {
                symbol:            "CMP".to_string(),
                interval:          "1m".to_string(),
                initial_balance:   capital,
                max_position_size: trade_amount,
                take_profit_pct:   tp,
                stop_loss_pct:     sl,
                strategy_name:     bt_name.to_string(),
                position_profile:  Some("Balanced".to_string()),
                security_profile:  Some("Development".to_string()),
                strategy_params:   None,
                commission_pct:    0.001,
                breakeven_at_rr:   None,
                atr_trail_mult:    None,
                partial_tp_ratio:  None,
            };
            Backtester::new(cfg).run(candles).ok()
                .filter(|r| r.total_trades >= 3)
                .map(|r| score_backtest_result(r.win_rate, r.profit_factor, r.sharpe_ratio, r.max_drawdown_pct))
                .unwrap_or(0.0)
        };

        // ── 2. HyperOpt mini-backtest (son 60 mum) — güncel koşullar ──
        let hyper_score = HyperOptimizer::simulate_score_dyn(strat_obj.as_ref(), candles, &default_p);

        // ── 3. Interval tümdengeli çarpanı ───────────────────────
        use memos_trading_core::robot::optimizer::{strategy_group, interval_category, interval_weight};
        let grp = strategy_group(bt_name);
        let cat = interval_category(interval);
        let int_w = interval_weight(grp, cat);

        // Kombinasyon: (güncel 60% + tarih 40%) × interval_weight
        let combined = (bt_score * 0.40 + hyper_score * 0.60) * int_w;

        if combined > best_score {
            best_score = combined;
            best_name  = short_name.to_string();
        }
    }

    // En az bir strateji seçilmiş olmalı; yoksa interval'e göre uygun fallback
    if best_name.is_empty() {
        use memos_trading_core::robot::optimizer::interval_category;
        match interval_category(interval) {
            memos_trading_core::robot::optimizer::IntervalCategory::Scalp => "STOCHASTIC",
            memos_trading_core::robot::optimizer::IntervalCategory::Intra => "SUPERTREND",
            memos_trading_core::robot::optimizer::IntervalCategory::Swing => "SMC",
        }.to_string()
    } else {
        best_name
    }
}

// ─── Sembol Skoru ────────────────────────────────────────────────────────────
// DB'deki her (exchange/market/symbol/interval) kombinasyonu için
// backtest ve veri bolluğu bazlı ağırlıklı skor.

/// Download loop'un anlık ilerleme durumu — loading ekranında gösterilir.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct DownloadProgress {
    current_idx:          usize,        // kaçıncı hedef (1-bazlı)
    total_targets:        usize,        // toplam hedef sayısı
    symbol:               String,
    market:               String,
    interval:             String,
    gap_start_ms:         Option<i64>,  // gap-fill: şu anki pencere başlangıcı (ms)
    gap_end_ms:           i64,          // gap-fill bitiş (≈ şimdiki zaman)
    gap_interval_ms:      i64,          // interval uzunluğu (ms)
    gap_initial_candles:  Option<i64>,  // gap-fill: ilk seferde ölçülen toplam boşluk
    dl_limit:             usize,        // her HTTP isteğinde max kaç mum
    inserted_session:     usize,        // bu seansta eklenen toplam mum
    derived_session:      usize,        // bu seansta türetilen HTF mum sayısı (1m→5m/1h/4h/1d)
    batch_no:             u32,          // aktif hedef için kaçıncı HTTP isteği
    session_start_ms:     i64,          // indirme başlangıç zamanı (epoch ms)
    /// Her hedefin kısa durumu: "⏳ Bekliyor" / "⬇ İndiriliyor" / "✓ +500" / "✗ Hata"
    target_labels:        Vec<(String, String, String, String, i64, usize)>, // (mkt, sym, intv, durum, gap_initial, inserted)
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct SymbolScore {
    exchange:     String,
    market:       String,
    symbol:       String,
    interval:     String,
    candle_count: usize,
    win_rate:     f64,   // backtest_results'tan; 0.0 = veri yok
    total_pnl:   f64,   // $
    total_trades: usize, // istatistiksel güvenilirlik için
    score:          f64,   // ağırlıklı kompozit skor
    last_price:     f64,   // son bilinen close fiyatı
    last_candle_ts: i64,   // son mumun Unix timestamp (sn) — 0 = bilinmiyor
    // ── Gelişmiş backtest metrikleri ─────────────────────────────────────────
    #[serde(default)]
    best_strategy:    String,  // En yüksek skor veren strateji adı
    #[serde(default)]
    profit_factor:    f64,     // Gross kâr / Gross zarar (>1.0 = kârlı)
    #[serde(default)]
    sharpe_ratio:     f64,     // Risk-ayarlı getiri (annualized)
    #[serde(default)]
    max_drawdown_pct: f64,     // Maksimum düşüş (%)
}

/// Tüm dosya yollarını tek yerde tutar — rtc_config.json'dan veya varsayılanlardan gelir.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ConfigPaths {
    db_path:                String,
    trade_quality:          String,
    robotic_profiles:       String,
    evolution_state:        String,
    fsm_state:              String,
    app_snapshot:           String,
}

impl ConfigPaths {
    fn from_oto(cfg: &OtoConfig) -> Self {
        Self {
            db_path:          cfg.db_path.clone(),
            trade_quality:    cfg.trade_quality_config_path.clone(),
            robotic_profiles: cfg.robotic_profiles_path.clone(),
            evolution_state:  cfg.evolution_state_path.clone(),
            fsm_state:        cfg.fsm_state_path.clone(),
            app_snapshot:     cfg.app_snapshot_path.clone(),
        }
    }
    fn resolve(path: &str) -> std::path::PathBuf {
        let p = std::path::Path::new(path);
        if p.exists() { return p.to_path_buf(); }
        // ../path denemesi (memos_trading_core/ içinden çalışılıyorsa)
        let up = std::path::Path::new("..").join(p);
        if up.exists() { return up; }
        p.to_path_buf()
    }
    fn evolution_state_path(&self) -> std::path::PathBuf { Self::resolve(&self.evolution_state) }
    fn fsm_state_path(&self)       -> std::path::PathBuf { Self::resolve(&self.fsm_state) }
    fn app_snapshot_path(&self)    -> std::path::PathBuf { Self::resolve(&self.app_snapshot) }
}

/// Onay bekleyen tehlikeli işlem türü
#[derive(Debug, Clone, PartialEq)]
enum ConfirmAction {
    /// [z] Paper bakiye + loop sıfırla
    PaperReset,
    /// [r] Tam sistem sıfırla (snapshot sil + engine yeniden başlat)
    FullReset,
}

impl ConfirmAction {
    fn prompt(&self) -> &'static str {
        match self {
            ConfirmAction::PaperReset => "⚠  [z] Paper bakiye SIFIRLANCAK — tüm açık pozisyonlar ve işlem geçmişi silinir. Devam? (E/H)",
            ConfirmAction::FullReset  => "⚠  [r] TAM SİFIRLAMA — snapshot dosyaları SİLİNECEK, engine yeniden başlayacak. Devam? (E/H)",
        }
    }
}

/// Binance 24hr ticker taramasında bulunan yeni sembol adayının durumu
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
enum ScreenerStatus {
    /// Tarayıcı tarafından bulundu, henüz indirilmedi
    New,
    /// İndirme kuyruğuna alındı
    Queued,
    /// Mum verisi indirildi, backtest bekleniyor
    Downloaded,
    /// Backtest tamamlandı, symbol_candidates'a eklendi
    Scored,
    /// Backtest skoru yetersiz — trade listesine alınmadı
    Rejected,
}

impl ScreenerStatus {
    fn label(&self) -> &'static str {
        match self {
            ScreenerStatus::New        => "🔎 Yeni",
            ScreenerStatus::Queued     => "⬇ Kuyrukta",
            ScreenerStatus::Downloaded => "✓ İndirildi",
            ScreenerStatus::Scored     => "🏆 Puanlandı",
            ScreenerStatus::Rejected   => "✗ Zayıf",
        }
    }
    fn color(&self) -> Color {
        match self {
            ScreenerStatus::New        => Color::White,
            ScreenerStatus::Queued     => Color::Yellow,
            ScreenerStatus::Downloaded => Color::Cyan,
            ScreenerStatus::Scored     => Color::LightGreen,
            ScreenerStatus::Rejected   => Color::Blue,
        }
    }
}

/// Binance REST API'den senkronize edilen tek bir emir/işlem satırı.
/// Hem açık emirler (status=NEW/PARTIALLY_FILLED) hem de geçmiş işlemler (FILLED) için kullanılır.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
struct ExchangeOrderRow {
    order_id:   u64,
    symbol:     String,
    side:       String,    // "BUY" | "SELL"
    status:     String,    // "NEW" | "FILLED" | "CANCELED" | "PARTIALLY_FILLED" | "POSITION"
    qty:        f64,
    filled_qty: f64,
    price:      f64,       // limit fiyatı; market = 0
    avg_price:  f64,       // ortalama dolum fiyatı
    pnl:        f64,       // sadece futures pozisyon satırları için (unrealized)
    is_active:  bool,      // true = açık emir, false = geçmiş işlem / kapalı
    created_at: String,    // yerel saat "HH:MM:SS"
    source:     String,    // "spot-open" | "spot-trade" | "fut-open" | "fut-pos" | "fut-trade"
}

/// Binance 24hr ticker taramasında bulunan sembol adayı
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ScreenerCandidate {
    symbol:            String,
    quote_volume_24h:  f64,    // 24 saatlik işlem hacmi (USDT)
    price_change_pct:  f64,    // 24 saatlik fiyat değişimi (%)
    trade_count_24h:   u64,    // 24 saatlik işlem adedi
    last_price:        f64,    // son fiyat
    status:            ScreenerStatus,
    found_at:          String, // bulunduğu saat (HH:MM:SS)
}

struct AppState {
    // FSM aynası (SharedLogger üzerinden log parse ile güncellenir)
    controller: AutonomousController,
    risk_gate: RiskGate,
    #[allow(dead_code)]
    recovery: RecoverySupervisor,

    // Bağlantı & mod
    live_mode: bool,
    api_key_set: bool,
    paper_mode: bool,

    // Başlangıç sermayesi
    equity: f64,

    // Trade sayacı — live_trade_count (AtomicU64) loop tarafından doğrudan artırılır
    total_trades: u32,
    live_trade_count: Arc<std::sync::atomic::AtomicU64>,
    // Kapanmış işlem geçmişi — loop pozisyon kapatırken ekler, Tab 4 okur
    live_closed_trades: Arc<std::sync::RwLock<memos_trading_core::robot::robotic_loop::ClosedTradeLog>>,

    // PnL anlık görüntü geçmişi: (saat, açık_pnl, sembol:fiyat_kaynağı özeti)
    // Her 30 saniyede bir diagnostic worker tarafından eklenir, son 120 kayıt saklanır (~60dk)
    pnl_snapshots: std::collections::VecDeque<(String, f64, String)>,

    // Olay günlüğü (son 300 satır)
    log: VecDeque<String>,

    // Kontrol bayrakları
    paused: bool,
    #[allow(dead_code)]
    evolution_enabled: bool,
    // Uygulama ömrü boyunca geçerli global durdurma sinyali.
    // Yalnızca kullanıcı 'q' tuşuna bastığında true olur; loop geçişlerinden etkilenmez.
    // Diagnostic worker, price poller, WS besleyici gibi singleton worker'lar bunu kullanır.
    app_stop_signal: Arc<AtomicBool>,
    // Atomik döngü kontrolleri (BinanceLiveAdapter ile paylaşılır)
    stop_signal: Arc<AtomicBool>,
    pause_signal: Arc<AtomicBool>,

    // Yapılandırma (config/rtc_config.json'dan yüklenir)
    config: OtoConfig,

    // Son backtest özeti
    last_backtest: Option<String>,

    // ML/AI durum izleme
    ml_signal:      String,      // "BUY" | "SELL" | "HOLD"
    ml_confidence:  f64,         // 0.0 – 1.0
    ml_score:       f64,         // -1.0 – 1.0 (raw regressor çıktısı)
    ml_train_count: u64,         // toplam eğitim adımı
    best_fast:      usize,       // HyperOpt'un bulduğu en iyi fast period
    best_slow:      usize,       // HyperOpt'un bulduğu en iyi slow period
    hyperopt_score: f64,         // en iyi hiperopt skoru (MA)
    best_rsi_period: usize,      // RSI HyperOpt: en iyi period
    best_rsi_ob:     f64,        // RSI HyperOpt: en iyi overbought %
    best_rsi_os:     f64,        // RSI HyperOpt: en iyi oversold %
    best_bb_period:  usize,      // BB HyperOpt: en iyi period
    best_bb_std_dev: f64,        // BB HyperOpt: en iyi std_dev çarpanı
    best_macd_fast:  usize,      // MACD HyperOpt: fast period
    best_macd_slow:  usize,      // MACD HyperOpt: slow period
    best_macd_signal: usize,     // MACD HyperOpt: signal period
    best_stoch_k:    usize,      // Stochastic/StochRSI HyperOpt: K period
    best_stoch_ob:   f64,        // Stochastic OB seviyesi
    best_stoch_os:   f64,        // Stochastic OS seviyesi
    best_ema_fast:          usize,  // EMA HyperOpt: fast period
    best_ema_slow:          usize,  // EMA HyperOpt: slow period
    best_donchian_period:   usize,  // DONCHIAN HyperOpt: period
    best_williams_period:   usize,  // WILLIAMS_R HyperOpt: period
    best_cci_period:        usize,  // CCI HyperOpt: period
    best_stoch_rsi_period:  usize,  // STOCH_RSI HyperOpt: rsi period
    best_supertrend_period: usize,  // SUPERTREND HyperOpt: ATR period
    best_supertrend_mult:   f64,    // SUPERTREND HyperOpt: ATR multiplier
    best_ict_fvg_lookback:  usize,  // ICT_FVG HyperOpt: lookback bars
    best_smc_swing_lb:      usize,  // SMC HyperOpt: swing lookback
    best_strategy_name: Option<String>, // Backtest sıralamasında 1. çıkan strateji
    best_sl:        f64,         // Backtest'ten türetilen otonom stop-loss %
    best_tp:        f64,         // Backtest'ten türetilen otonom take-profit %
    /// Paylaşılan per-sembol SL/TP + global MA — backtest günceller, loop anında okur
    live_risk: Arc<std::sync::RwLock<LiveRiskMap>>,
    /// Canlı OHLCV — loop her tick'te yazar, dashboard okur
    live_price: Arc<std::sync::RwLock<LivePriceData>>,
    /// Config'in orijinal primary sembolü için WS fiyat arşivi (örn. BTCUSDT).
    /// Primary geçişi sonrası active_sym farklılaşınca live_price'ı kirletmemek için ayrı tutuluyor.
    config_symbol_price: Arc<std::sync::RwLock<LivePriceData>>,
    /// Açık pozisyon snapshot — loop yazar (trailing SL/TP), dashboard okur
    live_positions: Arc<std::sync::RwLock<LivePositionMap>>,
    /// Aktif strateji adı — backtest karşılaştırması yazar (hot-reload), loop okur, dashboard gösterir
    live_strategy: Arc<std::sync::RwLock<String>>,
    /// Rejim bazlı aktif strateji — loop her tick yazar (örn: "RSI (low_vol)"), dashboard gösterir
    live_regime_strategy: Arc<std::sync::RwLock<String>>,
    /// Evrimsel AI canlı durumu — loop yazar, Evrim tab okur
    live_evolution: Arc<std::sync::RwLock<LiveEvolutionStatus>>,
    /// Aktif sembol adı — TUI yazar (AUTO mod), loop live_price filtrelemesi için okur
    live_active_symbol: Arc<std::sync::RwLock<String>>,
    last_ml_train:    Option<String>, // son eğitim özet metni
    last_ml_train_at: Option<String>, // son eğitim "YYYY-MM-DD HH:MM:SS" zaman damgası
    last_backtest_at: Option<String>, // son backtest skor güncellemesi — "donmuş skor" tanısı için
    ml_next_run_at:  Instant,        // sonraki ML çalışma zamanı (geri sayım için)
    ml_trigger:       Arc<AtomicBool>,  // [m] tuşu → anlık ML eğitim
    backtest_trigger: Arc<AtomicBool>, // [b] tuşu → anlık backtest
    backtest_running: Arc<AtomicBool>, // eşzamanlı çift backtest önleme kilidi
    ml_worker_running: Arc<AtomicBool>, // eşzamanlı çift ML worker önleme kilidi
    loop_restart_trigger: Arc<AtomicBool>, // aktif sembol/interval değişince loop restart
    /// [z] tetikleyince true → restart handler eski loop tamamen durduktan SONRA costs/trades sıfırlar.
    /// Sembol değişiminde false kalır (costs birikimli tutulur).
    pending_paper_reset: bool,
    loop_active_since:    Instant,         // mevcut sembolde ne zamandan beri çalışıyor (B kuralı)
    strategy_locked_until: Instant,       // strateji min-stay: bu zamandan önce değişim yok

    // Otonom Sembol Seçici
    auto_symbol:       bool,                // true = sistem en iyiyi seçer
    active_symbol:     SymbolScore,         // şu an işlem yapılan hedef
    symbol_candidates: Vec<SymbolScore>,    // tüm puanlı adaylar (en iyi önce)
    symbol_trigger:    Arc<AtomicBool>,     // [s] ile anlık yeniden tarama

    // ── Binance Sembol Tarayıcı (Screener) ───────────────────────────────────
    /// Binance 24hr ticker'dan gelen yeni sembol adayları (henüz backtest yapılmamış)
    screener_candidates:    Vec<ScreenerCandidate>,
    /// Son tarama zamanı
    screener_last_run:      Option<String>,
    /// [T] ile manuel tetik veya periyodik otomatik tetik
    screener_trigger:       Arc<AtomicBool>,
    /// Tarayıcı aktif mi (false = devre dışı)
    screener_enabled:       bool,
    /// Minimum 24h hacim filtresi (milyon USDT, default: 5.0)
    screener_min_volume_m:  f64,
    /// Minimum mutlak fiyat değişimi filtresi (%, default: 2.0)
    screener_min_change_pct: f64,
    /// Bir taramada keşfedilecek maksimum yeni sembol sayısı (default: 8)
    screener_max_new:       usize,
    /// Tarama periyodu (saat, default: 4.0)
    screener_interval_hours: f64,

    // Otonom Interval Optimizasyonu
    /// Her backtest sonrası doldurulur: (interval, score, win_rate, total_pnl)
    interval_scores:    Vec<(String, f64, f64, f64)>,
    /// En iyi bulunan interval tavsiyesi — dashboard + settings'de gösterilir
    best_interval_rec:  Option<(String, f64, f64, f64)>,
    /// true = en iyi interval bulunduğunda otomatik geçiş yapar (açık pozisyon yoksa)
    auto_interval:      bool,

    // Otonom veri indirme durumu
    last_download:      Option<String>,      // son indirme özet metni
    last_download_at:   Option<String>,      // son indirme "YYYY-MM-DD HH:MM:SS" zaman damgası
    download_count:     u64,                 // toplam indirilen mum sayısı
    download_active:    bool,                // şu an indirme yapılıyor mu?
    download_progress:  Option<DownloadProgress>, // anlık ilerleme (loading ekranı için)
    download_next_at:   Instant,             // sonraki indirme zamanı (geri sayım için)
    download_trigger:   Arc<AtomicBool>,     // [d] tuşu → anlık indirme
    init_complete:      Arc<AtomicBool>,     // ilk indirme+backtest bitti → dashboard göster
    /// D→B→ML→P5 otonom pipeline durumu
    pipeline:           PipelineStatus,
    signal_trigger:     Arc<AtomicBool>,     // [t] tuşu → anlık sinyal değerlendirmesi
    /// Sinyal denetim sayaçları — loop yazar, TUI/export/t-key okur
    live_signal_counts: Arc<std::sync::RwLock<LiveSignalCounts>>,
    /// Kümülatif işlem maliyetleri — loop yazar, TUI okur
    live_execution_costs: Arc<std::sync::RwLock<memos_trading_core::robot::robotic_loop::CumulativeTradingCosts>>,

    // ── Canlı Tanılama (Diagnostic) ─────────────────────────────────────────
    diag_alerts:     VecDeque<String>,  // son 20 tanılama uyarısı
    diag_warn_count: u32,               // aktif uyarı sayısı (dashboard göstergesi)
    #[allow(dead_code)]
    last_cycle_seen: u64,               // döngü donma tespiti için önceki cycle_id
    #[allow(dead_code)]
    last_cycle_at:   Instant,           // son cycle değişimi zamanı

    // ── Secondary worker log dedup ───────────────────────────────────────────
    /// Son 2 saniyede secondary worker'lardan gelen tekrarlayan mesajları filtreler.
    /// Key = context+msg hash, Value = son görülme zamanı.
    secondary_log_dedup: std::collections::HashMap<u64, Instant>,

    // ── Çoklu Sembol Orkestratörü ────────────────────────────────────────────
    /// Her sembol worker'ını (stop/pause signal + live_price) yönetir.
    /// max 5 eş zamanlı sembol destekler.
    orchestrator: SymbolOrchestrator,

    // ── Destek / Direnç bölgeleri ────────────────────────────────────────────
    /// Sembol → S/R bölgeleri — tüm aktif loop'lar yazar, TUI okur.
    live_sr_zones: Arc<std::sync::RwLock<std::collections::HashMap<String, Vec<memos_trading_core::robot::sr_detector::SrZone>>>>,
    /// Pipeline sağlık monitörü — Tab 8 için
    live_pipeline: Arc<std::sync::RwLock<memos_trading_core::robot::robotic_loop::LivePipelineHealth>>,
    /// Tab 6'da seçili sembol indeksi (← → ile gezinme)
    sr_tab_sym_idx: usize,
    config_paths: ConfigPaths,

    // ── HTF (üst zaman dilimi) mum sayacı ────────────────────────────────────
    /// sembol → interval → (mum_sayısı, son_timestamp_str).
    /// aggregate_from_1m tamamlanınca güncellenir; Tab 6 ve Tab 7'de gösterilir.
    htf_candle_counts: std::collections::HashMap<String, std::collections::HashMap<String, (usize, String)>>,

    // ── Pozisyon yönetimi (robotic_profiles.json ile senkron) ───────────────
    pos_sl_cooldown:      u64,          // SL cooldown saniyesi (varsayılan 600)
    pos_breakeven_at_rr:  Option<f64>,  // Breakeven R çarpanı (None = kapalı)
    pos_atr_trail_mult:   Option<f64>,  // ATR trailing çarpanı (None = kapalı)
    pos_partial_tp_ratio: Option<f64>,  // Kısmi TP oranı (None = kapalı)
    max_daily_trades:     u32,          // Günlük maksimum işlem limiti (0 = sınırsız)
    // ── Uyarlamalı koruma parametreleri (adaptive_params.json ile senkron) ──
    adaptive_params:      memos_trading_core::robot::adaptive_params::AdaptiveTradeParams,
    /// Auto-tune tetiklemek için son tune sırasındaki kapanmış işlem sayısı.
    /// Her `adaptive_params.adjust_every_n_trades` işlemde bir otomatik tune çalışır.
    last_auto_tune_trade_count: u32,

    // ── Strateji Doğrulama (Monte Carlo + Walk-Forward) ──────────────────────
    /// ML worker her eğitimde günceller; Tab 3 ve Tab 2 köprüsü okur.
    validation_result: Option<ValidationResult>,

    // ── p5_crypto analiz durumu (Python ML worker çıktısı) ─────────────────────
    /// `data/p5_results/status.json` + results JSON her ML döngüsünden sonra okunur.
    p5_last_status: Option<P5Status>,
    /// [p] tuşu veya ML döngüsü sonrası p5_crypto.py başlatma tetikleyicisi.
    p5_trigger:     Arc<AtomicBool>,

    // ── Tehlikeli işlem onay sistemi ──────────────────────────────────────────
    /// None = onay bekleniyor. Some(action) = kullanıcıdan E/H bekleniyor.
    confirm_pending: Option<ConfirmAction>,

    // ── Borsa Emir Senkronizasyonu ────────────────────────────────────────────
    /// Binance'dan çekilen aktif + geçmiş emirler — yalnızca live modda dolar.
    exchange_orders:      Vec<ExchangeOrderRow>,
    exchange_orders_sync: String,           // son sync zamanı "HH:MM:SS" veya "—"
    exchange_orders_trigger: Arc<AtomicBool>, // 'o' tuşu → anlık yenileme

    // ── MTF Fırsat Tarayıcısı ─────────────────────────────────────────────────
    /// Her 90 saniyede bir güncellenen çok-zaman-dilimli fırsat listesi.
    /// Tüm sembol adayları × {1m,5m,15m,1h,4h} taranır; yüksek güvenilirlikli
    /// sinyaller bu listede gösterilir.
    mtf_opportunities: Vec<MtfOpportunity>,
    /// Son MTF tarama zamanı (HH:MM:SS)
    mtf_last_scan:     Option<String>,
    /// [m] tuşu ile anında tarama tetikler
    mtf_scan_trigger:  Arc<AtomicBool>,
    /// Yüksek-güven MTF sinyali köprüsü: ML worker'ın HOLD döndürdüğü durumlarda
    /// bu sinyal ml_signal'ı geçici olarak override eder.
    /// Tuple: (symbol, interval, "BUY"|"SELL", score, win_rate)
    mtf_signal_inject: Option<(String, String, String, f64, f64)>,
}

impl AppState {
    fn new() -> Self {
        // Config'i en başta yükle — diğer her şey buradan türer
        let mut config = load_oto_config();
        let cfg_auto_interval = config.auto_interval; // move öncesinde sakla
        let cfg_paths = ConfigPaths::from_oto(&config);
        let init_profile = load_profile_config(&cfg_paths.robotic_profiles);

        let mut controller = AutonomousController::new(AutonomousConfig::default());
        controller.enable_evolution("MA".to_string());
        // Evolution snapshot'ı yükle: Q-table ve evrimleşmiş popülasyon kaldığı yerden devam eder
        if let Some((brain, population, saved_cycle_id)) = load_evolution_snapshot(&cfg_paths) {
            controller.adaptive_brain = Some(brain);
            controller.population_manager = Some(population);
            if saved_cycle_id > 0 {
                controller.cycle_id = saved_cycle_id;
            }
        }

        let api_key_set = std::env::var("BINANCE_API_KEY")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let is_testnet = std::env::var("TRADING_ENV")
            .map(|v| v.to_lowercase() == "testnet")
            .unwrap_or(false);
        let paper_mode = !api_key_set
            || is_testnet
            || std::env::var("BINANCE_PAPER_MODE")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(true);
        // equity daha sonra snapshot'tan restore edilir; şimdilik config ile başlat
        let equity = config.capital;
        // active_symbol için config değerlerini önceden klonla (config moved olacağından)
        let init_exchange = config.exchange.clone();
        let init_market   = config.market.clone();
        let init_symbol   = config.symbol.clone();
        let init_interval = config.interval.clone();
        let init_db_path  = config.db_path.clone();
        // Pipeline config değerlerini config move'dan önce al
        let pipeline_enabled    = config.pipeline_enabled;
        let pipeline_every_mins = config.pipeline_every_mins;
        let pipeline_p5_top_n   = config.pipeline_p5_top_n;
        // Kapasite — interval belli olduktan hemen sonra hesapla
        let auto_max_workers = compute_max_workers(&init_interval);

        // FSM snapshot'ını yükle → cycle_id + state kaldığı yerden devam eder
        let fsm_snap = load_fsm_snapshot(&cfg_paths);
        controller.cycle_id = fsm_snap.cycle_id;
        controller.consecutive_failures = fsm_snap.consecutive_failures;
        if let Some(s) = parse_autonomous_state(&format!("state={}", fsm_snap.state)) {
            controller.state = s;
        }

        // Uygulama snapshot'ını yükle → ML, risk, sembol, trade sayacı kaldığı yerden devam eder
        let mut app = load_app_snapshot(&cfg_paths);
        let risk_policy = if app.risk_max_notional_usd > 0.0 {
            RiskGatePolicy {
                max_notional_usd:     app.risk_max_notional_usd,
                max_daily_loss_pct:   app.risk_max_daily_loss_pct,
                max_drawdown_pct:     app.risk_max_drawdown_pct,
                min_model_confidence: app.risk_min_model_confidence,
            }
        } else { RiskGatePolicy::default() };
        // AUTO modda her zaman config default ile başla — scanner birkaç dakikada en iyiyi seçer
        // Manuel modda snapshot'tan geri yükle
        // Kullanıcının son seçtiği interval — snapshot'ta varsa config.interval'ı ezer.
        // 'i' tuşu veya ayarlar paneli değişikliği sonraki başlatmada da korunur.
        let effective_interval = if !app.active_interval.is_empty() {
            // Snapshot'taki interval config.interval'ı da ezer — ikisi senkron kalsın
            config.interval = app.active_interval.clone();
            app.active_interval.clone()
        } else {
            init_interval.clone()
        };
        let init_active = if app.auto_symbol || app.active_symbol.symbol.is_empty() {
            SymbolScore {
                exchange: init_exchange,
                market:   init_market,
                symbol:   init_symbol,
                interval: effective_interval,
                ..Default::default()
            }
        } else {
            // Manuel sembol seçiminde interval da snapshot'tan gelir; active_interval öncelikli
            let mut sym = app.active_symbol.clone();
            if !app.active_interval.is_empty() {
                sym.interval = app.active_interval.clone();
            }
            sym
        };

        // live_positions Arc'ını önceden oluştur — hem struct alanı hem orchestrator paylaşır
        // Paper veya live fark etmeksizin önceki oturumun açık pozisyonları geri yüklenir.
        // ASCII olmayan sembol adlı pozisyonları dışla (örn. Çince karakterli tokenlar)
        app.saved_positions.retain(|_, p| p.symbol.bytes().all(|b| b.is_ascii_alphanumeric()));
        let restore_positions = !app.saved_positions.is_empty();
        let live_positions_arc: Arc<std::sync::RwLock<LivePositionMap>> = Arc::new(
            std::sync::RwLock::new(
                if restore_positions { app.saved_positions.clone() } else { LivePositionMap::new() }
            )
        );

        // Yeniden başlatma bildirimi
        if restore_positions {
            let pos_list: Vec<String> = app.saved_positions.values()
                .map(|p| format!("{} {} @{:.4}", p.symbol, if p.is_long {"LONG"} else {"SHORT"}, p.entry_price))
                .collect();
            eprintln!("[RESTORE] {} açık pozisyon geri yüklendi: {}", app.saved_positions.len(), pos_list.join(", "));
            eprintln!("[RESTORE] MTF monitör aktif olunca fiyat kontrolü yapılacak.");
        }

        // LiveEvolutionStatus başlangıç değerleri için yardımcı değişkenler
        let controller_ref_brain = controller.adaptive_brain.is_some();
        let controller_ref_pop   = controller.population_manager.is_some();
        let genome_id_init       = controller.current_strategy_genome
            .as_ref().map(|g| g.id.clone()).unwrap_or_else(|| "G0-I0".to_string());

        // Paper modda güven eşiğini düşür → model öğrenene kadar trade izni ver
        let risk_policy = if paper_mode && risk_policy.min_model_confidence > 0.35 {
            RiskGatePolicy { min_model_confidence: 0.35, ..risk_policy }
        } else {
            risk_policy
        };

        // Config'den kaldıraç değerlerini struct move'undan önce çıkar
        let init_leverage_base  = config.leverage_base;
        let init_leverage_max   = config.leverage_max;
        let init_session_filter = config.session_filter.clone();

        // Snapshot'ta sıfır kalan alanları, config'deki persist edilmiş
        // optimized_params ile tamamla — restart sonrası parametre kaybı önlenir.
        let op = &config.optimized_params;
        if app.best_fast == 0     && op.ma_fast > 0      { app.best_fast      = op.ma_fast; }
        if app.best_slow == 0     && op.ma_slow > 0      { app.best_slow      = op.ma_slow; }
        if app.best_rsi_period==0 && op.rsi_period > 0   { app.best_rsi_period= op.rsi_period; }
        if app.best_rsi_ob==0.0   && op.rsi_ob > 0.0    { app.best_rsi_ob    = op.rsi_ob; }
        if app.best_rsi_os==0.0   && op.rsi_os > 0.0    { app.best_rsi_os    = op.rsi_os; }
        if app.best_bb_period==0  && op.bb_period > 0    { app.best_bb_period = op.bb_period; }
        if app.best_bb_std_dev==0.0&& op.bb_std_dev>0.0 { app.best_bb_std_dev= op.bb_std_dev; }
        if app.best_macd_fast==0  && op.macd_fast > 0   { app.best_macd_fast  = op.macd_fast; }
        if app.best_macd_slow==0  && op.macd_slow > 0   { app.best_macd_slow  = op.macd_slow; }
        if app.best_macd_signal==0&& op.macd_signal > 0 { app.best_macd_signal= op.macd_signal; }
        if app.best_stoch_k==0    && op.stoch_k > 0     { app.best_stoch_k   = op.stoch_k; }
        if app.best_stoch_ob==0.0 && op.stoch_ob > 0.0  { app.best_stoch_ob  = op.stoch_ob; }
        if app.best_stoch_os==0.0 && op.stoch_os > 0.0  { app.best_stoch_os  = op.stoch_os; }
        if app.best_strategy_name.is_none() { app.best_strategy_name = op.best_strategy.clone(); }

        Self {
            controller,
            risk_gate: RiskGate::new(risk_policy),
            recovery: RecoverySupervisor::default(),
            live_mode: false,
            api_key_set,
            paper_mode,
            // Snapshot'tan geri yüklenen özkaynak; 0 ise config.capital kullan (ilk başlatma)
            equity: if app.equity > 0.0 { app.equity } else { equity },
            total_trades: app.total_trades,
            live_trade_count: Arc::new(std::sync::atomic::AtomicU64::new(app.total_trades as u64)),
            live_closed_trades: Arc::new(std::sync::RwLock::new(app.saved_closed_trades.clone())),
            pnl_snapshots: std::collections::VecDeque::with_capacity(120),
            log: VecDeque::with_capacity(300),
            paused: false,
            evolution_enabled: true,
            app_stop_signal: Arc::new(AtomicBool::new(false)),
            stop_signal: Arc::new(AtomicBool::new(false)),
            pause_signal: Arc::new(AtomicBool::new(false)),
            config,
            last_backtest:     None,
            ml_signal:         "HOLD".to_string(), // Startup'ta her zaman HOLD; ML worker ilk çalışınca günceller
            ml_confidence:     app.ml_confidence,
            ml_score:          app.ml_score,
            ml_train_count:    app.ml_train_count,
            best_fast:         if app.best_fast > 0 { app.best_fast } else { 5 },
            best_slow:         if app.best_slow > 0 { app.best_slow } else { 20 },
            hyperopt_score:    app.hyperopt_score,
            best_rsi_period:   if app.best_rsi_period > 0 { app.best_rsi_period } else { 14 },
            best_rsi_ob:       if app.best_rsi_ob > 0.0 { app.best_rsi_ob } else { 70.0 },
            best_rsi_os:       if app.best_rsi_os > 0.0 { app.best_rsi_os } else { 30.0 },
            best_bb_period:    if app.best_bb_period  > 0   { app.best_bb_period  } else { 20 },
            best_bb_std_dev:   if app.best_bb_std_dev > 0.0 { app.best_bb_std_dev } else { 2.0 },
            best_macd_fast:    if app.best_macd_fast   > 0  { app.best_macd_fast   } else { 12 },
            best_macd_slow:    if app.best_macd_slow   > 0  { app.best_macd_slow   } else { 26 },
            best_macd_signal:  if app.best_macd_signal > 0  { app.best_macd_signal } else { 9  },
            best_stoch_k:      if app.best_stoch_k  > 0   { app.best_stoch_k  } else { 6  },
            best_stoch_ob:     if app.best_stoch_ob > 0.0 { app.best_stoch_ob } else { 70.0 },
            best_stoch_os:     if app.best_stoch_os > 0.0 { app.best_stoch_os } else { 20.0 },
            best_ema_fast:          if app.best_ema_fast          > 0   { app.best_ema_fast          } else { 5   },
            best_ema_slow:          if app.best_ema_slow          > 0   { app.best_ema_slow          } else { 20  },
            best_donchian_period:   if app.best_donchian_period   > 0   { app.best_donchian_period   } else { 20  },
            best_williams_period:   if app.best_williams_period   > 0   { app.best_williams_period   } else { 14  },
            best_cci_period:        if app.best_cci_period        > 0   { app.best_cci_period        } else { 20  },
            best_stoch_rsi_period:  if app.best_stoch_rsi_period  > 0   { app.best_stoch_rsi_period  } else { 14  },
            best_supertrend_period: if app.best_supertrend_period > 0   { app.best_supertrend_period } else { 10  },
            best_supertrend_mult:   if app.best_supertrend_mult   > 0.0 { app.best_supertrend_mult   } else { 3.0 },
            best_ict_fvg_lookback:  if app.best_ict_fvg_lookback  > 0   { app.best_ict_fvg_lookback  } else { 5   },
            best_smc_swing_lb:      if app.best_smc_swing_lb      > 0   { app.best_smc_swing_lb      } else { 10  },
            best_strategy_name: app.best_strategy_name.clone(),
            best_sl:           if app.best_sl  > 0.0 { app.best_sl  } else { 2.0 },
            best_tp:           if app.best_tp  > 0.0 { app.best_tp  } else { 4.0 },
            live_risk: Arc::new(std::sync::RwLock::new({
                let mut lrm = LiveRiskMap::new(
                    if app.best_sl  > 0.0 { app.best_sl  } else { 2.0 },
                    if app.best_tp  > 0.0 { app.best_tp  } else { 4.0 },
                    if app.best_fast > 0  { app.best_fast } else { 5   },
                    if app.best_slow > 0  { app.best_slow } else { 20  },
                );
                // Config'den kaldıraç aralığını yükle (kalıcı)
                lrm.base_leverage = init_leverage_base;
                lrm.max_leverage  = init_leverage_max;
                // Stochastic en iyi parametreleri
                lrm.global_stoch_k  = if app.best_stoch_k  > 0   { app.best_stoch_k  } else { 6    };
                lrm.global_stoch_ob = if app.best_stoch_ob > 0.0 { app.best_stoch_ob } else { 70.0 };
                lrm.global_stoch_os = if app.best_stoch_os > 0.0 { app.best_stoch_os } else { 20.0 };
                // Session filter config'den yükle
                lrm.session_filter_enabled       = init_session_filter.enabled;
                lrm.session_allowed_hours        = init_session_filter.allowed_hours.clone();
                lrm.session_blocked_hours        = init_session_filter.blocked_hours.clone();
                lrm.session_long_preferred_hours = init_session_filter.long_preferred_hours.clone();
                // Best strategy: snapshot'tan varsa yükle
                if let Some(ref sname) = app.best_strategy_name {
                    lrm.best_strategy_name = sname.clone();
                }
                // ML model ağırlıkları — restart sonrası hemen trained modeli kullan
                if let Some(ref w) = app.ml_weights_trained {
                    if w.len() == memos_trading_core::robot::ml_engine::linear_regressor::N_FEATURES {
                        let mut arr = [0.0f64; memos_trading_core::robot::ml_engine::linear_regressor::N_FEATURES];
                        arr.copy_from_slice(w);
                        lrm.ml_weights      = Some(arr);
                        lrm.ml_bias_trained = app.ml_bias_trained;
                    }
                }
                lrm.gbt_last_score  = app.gbt_last_score;
                // OOS metrikleri — restart sonrası Tab 2 ML Worker paneli veri gösterir
                lrm.oos_win_rate    = app.oos_win_rate;
                lrm.oos_avg_return  = app.oos_avg_return;
                lrm.oos_bar_count   = app.oos_bar_count;
                lrm.oos_fold_scores = app.oos_fold_scores;
                lrm
            })),
            live_price: Arc::new(std::sync::RwLock::new(LivePriceData::default())),
            config_symbol_price: Arc::new(std::sync::RwLock::new(LivePriceData::default())),
            live_positions: Arc::clone(&live_positions_arc),
            live_strategy:  Arc::new(std::sync::RwLock::new(
                if !app.live_strategy.is_empty() { app.live_strategy.clone() } else { "MA".to_string() }
            )),
            live_regime_strategy: Arc::new(std::sync::RwLock::new("—".to_string())),
            live_evolution: Arc::new(std::sync::RwLock::new(LiveEvolutionStatus {
                evolution_enabled:     true,
                brain_active:          controller_ref_brain,
                pop_active:            controller_ref_pop,
                evolve_every_n_cycles: 50,
                cycle_id:              fsm_snap.cycle_id,
                genome_id:             genome_id_init,
                ..LiveEvolutionStatus::default()
            })),
            live_active_symbol: Arc::new(std::sync::RwLock::new(init_active.symbol.clone())),
            last_ml_train:     None,
            last_ml_train_at:  None,
            last_backtest_at:  None,
            ml_next_run_at:    Instant::now() + Duration::from_secs(45),
            ml_trigger:            Arc::new(AtomicBool::new(false)),
            backtest_trigger:      Arc::new(AtomicBool::new(false)),
            backtest_running:      Arc::new(AtomicBool::new(false)),
            ml_worker_running:     Arc::new(AtomicBool::new(false)),
            loop_restart_trigger:  Arc::new(AtomicBool::new(false)),
            pending_paper_reset:   false,
            loop_active_since:     Instant::now(),
            strategy_locked_until: if app.strategy_lock_remaining_secs > 0 {
                Instant::now() + Duration::from_secs(app.strategy_lock_remaining_secs)
            } else {
                Instant::now()
            },
            auto_symbol:       true, // her başlangıçta AUTO — snapshot'tan gelmez
            active_symbol:     init_active,
            // Snapshot'tan gelen eski skorları filtreden geçir:
            // yetersiz trade (<20) veya break-even altı WR → skor sıfırla
            symbol_candidates: {
                let be_wr = app.best_sl / (app.best_sl + app.best_tp) * 100.0;
                let mut cands = app.symbol_candidates;
                // ASCII olmayan semboller dışla (örn. Çince karakterli tokenlar)
                cands.retain(|c| c.symbol.bytes().all(|b| b.is_ascii_alphanumeric()));
                for c in &mut cands {
                    if c.total_trades < 20 || c.win_rate < be_wr {
                        c.score = 0.0;
                    }
                }
                cands
            },
            symbol_trigger:          Arc::new(AtomicBool::new(false)),
            screener_candidates:     {
                let mut sc = app.screener_candidates;
                sc.retain(|c| c.symbol.bytes().all(|b| b.is_ascii_alphanumeric()));
                sc
            },
            screener_last_run:       app.screener_last_run,
            screener_trigger:        Arc::new(AtomicBool::new(false)),
            screener_enabled:        app.screener_enabled,
            screener_min_volume_m:   app.screener_min_volume_m,
            screener_min_change_pct: app.screener_min_change_pct,
            screener_max_new:        app.screener_max_new,
            screener_interval_hours: app.screener_interval_hours,
            interval_scores:   Vec::new(),
            best_interval_rec: None,
            auto_interval:     cfg_auto_interval, // rtc_config.json'dan yükle
            last_download:     None,
            last_download_at:  None,
            download_count:    app.download_count,
            download_active:   false,
            download_progress: None,
            download_next_at:  Instant::now() + Duration::from_secs(5), // worker 5sn sonra başlar, hemen indirir
            download_trigger:  Arc::new(AtomicBool::new(false)),
            // DB'de önceki oturumdan kalan candle verisi varsa loading ekranını atla.
            // İlk kurulumda (boş DB) false → download tamamlanınca true olur.
            init_complete: Arc::new(AtomicBool::new({
                rusqlite::Connection::open(&init_db_path)
                    .ok()
                    .and_then(|conn| {
                        conn.query_row(
                            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE 'candles%'",
                            [],
                            |row| row.get::<_, i64>(0),
                        ).ok()
                    })
                    .map(|n| n > 0)
                    .unwrap_or(false)
            })),
            signal_trigger:    Arc::new(AtomicBool::new(false)),
            live_signal_counts: Arc::new(std::sync::RwLock::new(LiveSignalCounts::default())),
            // Snapshot'tan geri yüklenen maliyet sayaçları — restart sonrası sıfırlanmaz
            live_execution_costs: Arc::new(std::sync::RwLock::new(app.cumulative_costs.clone())),
            diag_alerts:       VecDeque::with_capacity(20),
            diag_warn_count:   0,
            last_cycle_seen:   fsm_snap.cycle_id,
            last_cycle_at:     Instant::now(),
            secondary_log_dedup: std::collections::HashMap::new(),
            orchestrator: SymbolOrchestrator::new(
                auto_max_workers,
                Arc::clone(&live_positions_arc),
            ),
            live_sr_zones: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
            live_pipeline: Arc::new(std::sync::RwLock::new(
                memos_trading_core::robot::robotic_loop::LivePipelineHealth::default()
            )),
            sr_tab_sym_idx: 0,
            config_paths: cfg_paths,
            htf_candle_counts: std::collections::HashMap::new(),
            pos_sl_cooldown:      init_profile.sl_cooldown_secs.unwrap_or(600),
            pos_breakeven_at_rr:  init_profile.breakeven_at_rr,
            pos_atr_trail_mult:   init_profile.atr_trail_mult,
            pos_partial_tp_ratio: init_profile.partial_tp_ratio,
            max_daily_trades:     0, // 0 = sınırsız
            adaptive_params:      memos_trading_core::robot::adaptive_params::AdaptiveTradeParams::load(
                &default_adaptive_params_path()
            ),
            last_auto_tune_trade_count: 0,
            validation_result:    app.validation_result.clone(),
            p5_last_status:       None,
            p5_trigger:           Arc::new(AtomicBool::new(false)),
            pipeline: PipelineStatus {
                enabled:     pipeline_enabled,
                every_mins:  pipeline_every_mins,
                p5_top_n:    pipeline_p5_top_n,
                // İlk çalıştırma: 15 saniye sonra (init tamamlandıktan sonra başlar)
                next_run_at: Instant::now() + Duration::from_secs(15),
                ..PipelineStatus::default()
            },
            confirm_pending:      None,
            exchange_orders:          Vec::new(),
            exchange_orders_sync:     "—".to_string(),
            exchange_orders_trigger:  Arc::new(AtomicBool::new(false)),
            mtf_opportunities:        Vec::new(),
            mtf_last_scan:            None,
            mtf_scan_trigger:         Arc::new(AtomicBool::new(false)),
            mtf_signal_inject:        None,
        }
    }

    /// Uyarlamalı parametreleri diske kaydet.
    fn save_adaptive_params(&self) {
        let path = &self.config.adaptive_params_path;
        self.adaptive_params.save(path);
        // Versiyon sayacını artır — loop'taki reload_adaptive_params() bunu görür ve anında yükler
        if let Ok(mut lrm) = self.live_risk.write() {
            lrm.adaptive_params_version = lrm.adaptive_params_version.wrapping_add(1);
        }
    }

    // Aktif işlem hedefini döndür: auto_symbol=true ise sistem seçer,
    // false ise config.json'daki manuel değer kullanılır.
    fn active_trade_target(&self) -> (String, String, String, String) {
        if self.auto_symbol && !self.active_symbol.symbol.is_empty() {
            (
                self.active_symbol.exchange.clone(),
                self.active_symbol.market.clone(),
                self.active_symbol.symbol.clone(),
                self.active_symbol.interval.clone(),
            )
        } else {
            (
                self.config.exchange.clone(),
                self.config.market.clone(),
                self.config.symbol.clone(),
                self.config.interval.clone(),
            )
        }
    }

    fn push_log(&mut self, msg: String) {
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let line = format!("[{}] {}", ts, msg);
        self.log.push_back(line.clone());
        if self.log.len() > 300 {
            self.log.pop_front();
        }
        // TUI tuş basımı ve manuel tetikleme mesajlarını da log dosyasına yaz
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true).append(true).open("logs/rtc_cli.log")
        {
            use std::io::Write;
            let _ = writeln!(f, "{}", line);
        }
    }

    /// Equity'yi güvenli şekilde güncelle — tüm yazma noktaları bu metodu kullanır.
    /// Hesap: başlangıç_sermayesi + realize_pnl (kapanmış işlemler toplamı).
    /// Açık pozisyon unrealized PnL dahil değil — display katmanı ayrıca ekler.
    ///
    /// NOT: live_risk.current_equity KULLANILMAZ — çoklu sembol worker'ları (orchestrator)
    /// aynı Arc'a sırayla yazar ve sonuncu yazı öncekini siler. Bunun yerine tüm
    /// worker'ların paylaştığı live_closed_trades toplamı doğru kaynaktır.
    fn recalculate_equity(&mut self) {
        let realized_pnl: f64 = self.live_closed_trades.read().ok()
            .map(|ct| ct.iter().map(|t| t.pnl).sum())
            .unwrap_or(0.0);
        self.equity = (self.config.capital + realized_pnl).max(0.0);
    }

    /// Sermaye ayarlandığında equity'yi de senkronize et.
    fn set_capital(&mut self, new_capital: f64) {
        self.config.capital = new_capital.max(100.0);
        self.recalculate_equity();
    }

    /// Hard reset: tüm işlem geçmişini temizle, equity'yi sermayeye sıfırla.
    fn reset_pnl(&mut self) {
        if let Ok(mut log) = self.live_closed_trades.write() {
            log.clear();
        }
        self.equity = self.config.capital;
        self.total_trades = 0;
        // ph istatistiklerini anında sıfırla — stale loss_streak/session_closed görünmesin
        if let Ok(mut ph) = self.live_pipeline.write() {
            ph.loss_streak    = 0;
            ph.session_wins   = 0;
            ph.session_closed = 0;
        }
    }

    /// Paper modda çalışırken bakiyeyi + kapalı işlemleri sıfırlar.
    /// Açık pozisyonlar da temizlenir (sanal — fiyat kaydı geçersiz kılar).
    /// Engine durdurulmaz, strateji/ML/evrim korunur.
    fn reset_paper_balance(&mut self) {
        if !self.paper_mode { return; } // live modda çağrılırsa no-op

        // ── Bakiye ve trade istatistikleri ────────────────────────────────────
        self.equity       = self.config.capital;
        self.total_trades = 0;
        self.pnl_snapshots.clear();
        self.live_trade_count.store(0, std::sync::atomic::Ordering::Relaxed);

        // ── Açık/kapalı pozisyonlar ───────────────────────────────────────────
        if let Ok(mut log) = self.live_closed_trades.write() { log.clear(); }
        if let Ok(mut pos) = self.live_positions.write()     { pos.clear(); }

        // ── Komisyon / spread / slippage / market impact ──────────────────────
        // Bunlar uygulama yeniden başlatılana kadar birikmekte, [z] ile sıfırlanmıyordu.
        if let Ok(mut costs) = self.live_execution_costs.write() {
            *costs = memos_trading_core::robot::robotic_loop::CumulativeTradingCosts::default();
        }

        // ── Sinyal sayaçları (BUY/SELL/HOLD istatistikleri) ──────────────────
        if let Ok(mut sc) = self.live_signal_counts.write() {
            *sc = LiveSignalCounts::default();
        }
    }
}

// ─── SharedLogger ─────────────────────────────────────────────────────────────
// RoboticLoop'un ErrorLogger çıktısını AppState.log'a yönlendirir.
// FSM durum değişikliklerini ve cycle ID'yi log satırlarından parse eder.

struct SharedLogger {
    state:      Arc<Mutex<AppState>>,
    /// true = birincil loop (FSM state + cycle_id parse eder, snapshot kaydeder)
    /// false = orchestrator worker (sadece log satırı ekler, FSM'e dokunmaz)
    is_primary: bool,
    /// Dosyaya yazılacak log yolu (None = sadece bellek)
    log_path:   Option<String>,
}

impl SharedLogger {
    fn new(state: Arc<Mutex<AppState>>, log_path: Option<String>) -> Self {
        Self { state, is_primary: true, log_path }
    }
    fn new_secondary(state: Arc<Mutex<AppState>>, log_path: Option<String>) -> Self {
        Self { state, is_primary: false, log_path }
    }
}

impl ErrorLogger for SharedLogger {
    fn log_error(&self, context: &str, msg: &str) {
        self.dispatch("[ERR]", context, msg);
    }
    fn log_info(&self, context: &str, msg: &str) {
        self.dispatch("[INF]", context, msg);
    }
}

impl SharedLogger {
    fn dispatch(&self, tag: &str, context: &str, msg: &str) {
        // Disk I/O (save_fsm_snapshot) lock Dışında yapılacak.
        // Lock içinde sadece bellek işlemleri: state parse, log push, sayaç.
        let fsm_save: Option<(u64, AutonomousState, usize, ConfigPaths)>;
        let log_line: String;

        {
            let Ok(mut st) = self.state.lock() else { return; };

            // Secondary worker log dedup: aynı context+msg kombinasyonu son 2 saniye içinde
            // başka bir secondary worker tarafından yazıldıysa tekrarı atla.
            if !self.is_primary {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                context.hash(&mut h);
                msg.hash(&mut h);
                let key = h.finish();
                let now = Instant::now();
                let recent = st.secondary_log_dedup.get(&key)
                    .map(|t| now.duration_since(*t).as_millis() < 2000)
                    .unwrap_or(false);
                if recent {
                    return;
                }
                st.secondary_log_dedup.insert(key, now);
                if st.secondary_log_dedup.len() > 100 {
                    st.secondary_log_dedup.retain(|_, t| now.duration_since(*t).as_millis() < 5000);
                }
            }

            // FSM state + cycle_id parse (sadece primary worker)
            fsm_save = if self.is_primary {
                let mut dirty = false;
                if let Some(s) = parse_autonomous_state(msg) {
                    st.controller.state = s;
                    dirty = true;
                }
                if let Some(c) = parse_cycle_id(msg) {
                    st.controller.cycle_id = c;
                    dirty = true;
                }
                if dirty {
                    Some((
                        st.controller.cycle_id,
                        st.controller.state.clone(),
                        st.controller.consecutive_failures,
                        st.config_paths.clone(),
                    ))
                } else {
                    None
                }
            } else {
                None
            };

            // Trade sayacı
            if context == "trade"
                || msg.contains("BUY executed")
                || msg.contains("SELL executed")
                || msg.contains("Trade #")
            {
                st.total_trades += 1;
            }
            let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            log_line = format!("[{}][{}][{}] {}", ts, tag, context, msg);
            st.log.push_back(log_line.clone());
            if st.log.len() > 300 {
                st.log.pop_front();
            }
        } // ← lock burada serbest bırakılır

        // Disk I/O: lock dışında
        if let Some(ref path) = self.log_path {
            use std::fs::OpenOptions;
            use std::io::Write as _;
            if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "{}", log_line);
            }
        }
        if let Some((cycle_id, ref state, failures, ref paths)) = fsm_save {
            save_fsm_snapshot(cycle_id, state, failures, paths);
        }
    }
}

fn parse_autonomous_state(msg: &str) -> Option<AutonomousState> {
    let idx = msg.find("state=")?;
    let rest = &msg[idx + 6..];
    let word: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
    match word.as_str() {
        "Observe"  => Some(AutonomousState::Observe),
        "Decide"   => Some(AutonomousState::Decide),
        "Validate" => Some(AutonomousState::Validate),
        "Execute"  => Some(AutonomousState::Execute),
        "Verify"   => Some(AutonomousState::Verify),
        "Adapt"    => Some(AutonomousState::Adapt),
        "SafeMode" => Some(AutonomousState::SafeMode),
        "Halted"   => Some(AutonomousState::Halted),
        _          => None,
    }
}

fn parse_cycle_id(msg: &str) -> Option<u64> {
    let idx = msg.find("cycle=")?;
    let rest = &msg[idx + 6..];
    rest.chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

// ─── FSM Durum Kalıcılığı ──────────────────────────────────────────────────────
// Yeniden başlatmalar arasında cycle_id + FSM state korunur.
// Dosya: config/fsm_state.json

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct FsmSnapshot {
    cycle_id:             u64,
    state:                String,   // "Observe", "Execute" vb.
    consecutive_failures: usize,
}


/// Startup'ta bir kez çağrılır — config dizininin var olduğunu garanti eder.
fn ensure_config_dir() {
    let candidates = ["config", "../config"];
    for dir in &candidates {
        if std::path::Path::new(dir).exists() {
            return; // zaten mevcut
        }
    }
    let _ = std::fs::create_dir_all("config");
}

fn save_fsm_snapshot(cycle_id: u64, state: &AutonomousState, failures: usize, paths: &ConfigPaths) {
    let snap = FsmSnapshot {
        cycle_id,
        state: format!("{:?}", state),
        consecutive_failures: failures,
    };
    if let Ok(json) = serde_json::to_string_pretty(&snap) {
        let _ = std::fs::write(paths.fsm_state_path(), json);
    }
}

fn load_fsm_snapshot(paths: &ConfigPaths) -> FsmSnapshot {
    let path = paths.fsm_state_path();
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(snap) = serde_json::from_str::<FsmSnapshot>(&text) {
            return snap;
        }
    }
    FsmSnapshot::default()
}

// ─── Uygulama Durum Kalıcılığı ───────────────────────────────────────────────
// Yeniden başlatmalar arasında korunacak alanlar:
//   ML sonuçları, HyperOpt parametreleri, aktif sembol, sembol adayları,
//   risk politikası, toplam trade sayısı, indirme sayısı.
// Dosya: config/app_snapshot.json

fn load_evolution_snapshot(paths: &ConfigPaths) -> Option<(AdaptiveBrain, PopulationManager, u64)> {
    #[derive(serde::Deserialize)]
    struct EvSnap {
        brain: Option<AdaptiveBrain>,
        population: Option<PopulationManager>,
        #[serde(default)]
        cycle_id: u64,
    }
    let path = paths.evolution_state_path();
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(snap) = serde_json::from_str::<EvSnap>(&text) {
            if let (Some(b), Some(pm)) = (snap.brain, snap.population) {
                return Some((b, pm, snap.cycle_id));
            }
        }
    }
    None
}

/// Snapshot şema sürümü — bu sabit artırılırsa eski snapshot otomatik atılır.
/// Sembol skorlama formülü veya SymbolScore alanları değiştiğinde artır.
const APP_SNAPSHOT_VERSION: u32 = 2; // v2: multi-strateji + profit_factor/sharpe/drawdown eklendi

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct AppSnapshot {
    #[serde(default)]
    schema_version:            u32,
    total_trades:              u32,
    ml_signal:                 String,
    ml_confidence:             f64,
    ml_score:                  f64,
    ml_train_count:            u64,
    best_fast:                 usize,
    best_slow:                 usize,
    hyperopt_score:            f64,
    #[serde(default)]
    best_rsi_period:           usize,
    #[serde(default)]
    best_rsi_ob:               f64,
    #[serde(default)]
    best_rsi_os:               f64,
    #[serde(default)]
    best_bb_period:            usize,
    #[serde(default)]
    best_bb_std_dev:           f64,
    #[serde(default)]
    best_macd_fast:            usize,
    #[serde(default)]
    best_macd_slow:            usize,
    #[serde(default)]
    best_macd_signal:          usize,
    #[serde(default)]
    best_stoch_k:              usize,
    #[serde(default)]
    best_stoch_ob:             f64,
    #[serde(default)]
    best_stoch_os:             f64,
    #[serde(default)]
    best_ema_fast:             usize,
    #[serde(default)]
    best_ema_slow:             usize,
    #[serde(default)]
    best_donchian_period:      usize,
    #[serde(default)]
    best_williams_period:      usize,
    #[serde(default)]
    best_cci_period:           usize,
    #[serde(default)]
    best_stoch_rsi_period:     usize,
    #[serde(default)]
    best_supertrend_period:    usize,
    #[serde(default)]
    best_supertrend_mult:      f64,
    #[serde(default)]
    best_ict_fvg_lookback:     usize,
    #[serde(default)]
    best_smc_swing_lb:         usize,
    #[serde(default)]
    best_strategy_name:        Option<String>,
    best_sl:                   f64,
    best_tp:                   f64,
    #[serde(default = "default_true")]
    auto_symbol:               bool,
    active_symbol:             SymbolScore,
    symbol_candidates:         Vec<SymbolScore>,
    download_count:            u64,
    risk_max_notional_usd:     f64,
    risk_max_daily_loss_pct:   f64,
    risk_max_drawdown_pct:     f64,
    risk_min_model_confidence: f64,
    live_strategy:             String,
    #[serde(default)]
    strategy_lock_remaining_secs: u64, // kilitte kalan süre (sn) — Instant serialize edilemez
    #[serde(default)]
    saved_positions: std::collections::HashMap<String, memos_trading_core::robot::robotic_loop::LivePositionData>,
    #[serde(default)]
    saved_closed_trades: Vec<memos_trading_core::robot::robotic_loop::ClosedTradeData>,
    /// Eğitilmiş LinearRegressor ağırlıkları — restart sonrası with_defaults() yerine bunlar kullanılır
    #[serde(default)]
    ml_weights_trained: Option<Vec<f64>>,
    #[serde(default)]
    ml_bias_trained: f64,
    /// GBT'nin son tahmin skoru — restart sonrası voter hemen devreye girer
    #[serde(default)]
    gbt_last_score: Option<f64>,
    /// ML OOS kalite metrikleri — restart sonrası Tab 2 ML Worker paneli veri gösterir
    #[serde(default)]
    oos_win_rate: f64,
    #[serde(default)]
    oos_avg_return: f64,
    #[serde(default)]
    oos_bar_count: usize,
    #[serde(default)]
    oos_fold_scores: [f64; 3],
    /// MC + Walk-Forward doğrulama sonucu — restart sonrası Tab 3 ve Tab 2 köprüsü veri gösterir
    #[serde(default)]
    validation_result: Option<ValidationResult>,
    /// Screener adayları — restart sonrası durum korunur
    #[serde(default)]
    screener_candidates: Vec<ScreenerCandidate>,
    #[serde(default)]
    screener_last_run: Option<String>,
    #[serde(default = "default_true")]
    screener_enabled: bool,
    #[serde(default = "default_screener_min_vol")]
    screener_min_volume_m: f64,
    #[serde(default = "default_screener_min_chg")]
    screener_min_change_pct: f64,
    #[serde(default = "default_screener_max_new")]
    screener_max_new: usize,
    #[serde(default = "default_screener_interval_hours")]
    screener_interval_hours: f64,
    /// Kullanıcının 'i' tuşu veya ayarlar paneli ile seçtiği interval — restart sonrası korunur.
    /// Boşsa config.interval kullanılır.
    #[serde(default)]
    active_interval: String,
    /// Gerçekleşmiş özkaynak — restart sonrası balance korunur.
    /// 0.0 ise config.capital kullanılır (geriye dönük uyumluluk).
    #[serde(default)]
    equity: f64,
    /// Kümülatif işlem maliyetleri — restart sonrası komisyon/slippage sayaçları korunur.
    #[serde(default)]
    cumulative_costs: memos_trading_core::robot::robotic_loop::CumulativeTradingCosts,
}

/// Snapshot için gereken veriler: JSON string + yazılacak yollar.
/// Disk I/O içermez — AppState lock'u tutarken güvenle çağrılabilir.
struct SnapshotPayload {
    app_json:  Option<String>,
    app_path:  std::path::PathBuf,
    evo_json:  Option<String>,
    evo_path:  std::path::PathBuf,
}

/// Adım 1: AppState'den veri topla (bellek, lock güvenli).
fn collect_snapshot_payload(st: &AppState) -> SnapshotPayload {
    let snap = AppSnapshot {
        schema_version:            APP_SNAPSHOT_VERSION,
        total_trades:              st.total_trades,
        ml_signal:                 st.ml_signal.clone(),
        ml_confidence:             st.ml_confidence,
        ml_score:                  st.ml_score,
        ml_train_count:            st.ml_train_count,
        best_fast:                 st.best_fast,
        best_slow:                 st.best_slow,
        hyperopt_score:            st.hyperopt_score,
        best_rsi_period:           st.best_rsi_period,
        best_rsi_ob:               st.best_rsi_ob,
        best_rsi_os:               st.best_rsi_os,
        best_bb_period:            st.best_bb_period,
        best_bb_std_dev:           st.best_bb_std_dev,
        best_macd_fast:            st.best_macd_fast,
        best_macd_slow:            st.best_macd_slow,
        best_macd_signal:          st.best_macd_signal,
        best_stoch_k:              st.best_stoch_k,
        best_stoch_ob:             st.best_stoch_ob,
        best_stoch_os:             st.best_stoch_os,
        best_ema_fast:             st.best_ema_fast,
        best_ema_slow:             st.best_ema_slow,
        best_donchian_period:      st.best_donchian_period,
        best_williams_period:      st.best_williams_period,
        best_cci_period:           st.best_cci_period,
        best_stoch_rsi_period:     st.best_stoch_rsi_period,
        best_supertrend_period:    st.best_supertrend_period,
        best_supertrend_mult:      st.best_supertrend_mult,
        best_ict_fvg_lookback:     st.best_ict_fvg_lookback,
        best_smc_swing_lb:         st.best_smc_swing_lb,
        best_strategy_name:        st.best_strategy_name.clone(),
        best_sl:                   st.best_sl,
        best_tp:                   st.best_tp,
        auto_symbol:               st.auto_symbol,
        active_symbol:             st.active_symbol.clone(),
        symbol_candidates:         st.symbol_candidates.clone(),
        download_count:            st.download_count,
        risk_max_notional_usd:     st.risk_gate.policy.max_notional_usd,
        risk_max_daily_loss_pct:   st.risk_gate.policy.max_daily_loss_pct,
        risk_max_drawdown_pct:     st.risk_gate.policy.max_drawdown_pct,
        risk_min_model_confidence: st.risk_gate.policy.min_model_confidence,
        live_strategy:             st.live_strategy.read().ok().map(|g| g.clone()).unwrap_or_default(),
        strategy_lock_remaining_secs: st.strategy_locked_until
            .saturating_duration_since(Instant::now()).as_secs(),
        saved_positions:           st.live_positions.read().ok()
            .map(|p| p.clone())
            .unwrap_or_default(),
        saved_closed_trades:       st.live_closed_trades.read().ok()
            .map(|v| v.clone())
            .unwrap_or_default(),
        ml_weights_trained: st.live_risk.read().ok()
            .and_then(|r| r.ml_weights.map(|w| w.to_vec())),
        ml_bias_trained: st.live_risk.read().ok()
            .map(|r| r.ml_bias_trained).unwrap_or(0.0),
        gbt_last_score: st.live_risk.read().ok()
            .and_then(|r| r.gbt_last_score),
        oos_win_rate:    st.live_risk.read().ok().map(|r| r.oos_win_rate).unwrap_or(0.0),
        oos_avg_return:  st.live_risk.read().ok().map(|r| r.oos_avg_return).unwrap_or(0.0),
        oos_bar_count:   st.live_risk.read().ok().map(|r| r.oos_bar_count).unwrap_or(0),
        oos_fold_scores: st.live_risk.read().ok().map(|r| r.oos_fold_scores).unwrap_or([0.0; 3]),
        validation_result: st.validation_result.clone(),
        screener_candidates:      st.screener_candidates.clone(),
        screener_last_run:        st.screener_last_run.clone(),
        screener_enabled:         st.screener_enabled,
        screener_min_volume_m:    st.screener_min_volume_m,
        screener_min_change_pct:  st.screener_min_change_pct,
        screener_max_new:         st.screener_max_new,
        screener_interval_hours:  st.screener_interval_hours,
        // Kullanıcının seçtiği interval — restart sonrası config.interval'ı ezer
        active_interval:          st.active_symbol.interval.clone(),
        // Özkaynak ve işlem maliyetleri — restart sonrası balance + maliyet sayaçları korunur
        equity:                   st.equity,
        cumulative_costs:         st.live_execution_costs.read().ok()
            .map(|c| c.clone())
            .unwrap_or_default(),
    };

    #[derive(serde::Serialize)]
    struct EvSnap<'a> {
        brain:      Option<&'a memos_trading_core::evolution::AdaptiveBrain>,
        population: Option<&'a memos_trading_core::evolution::PopulationManager>,
    }
    let ev = EvSnap {
        brain:      st.controller.adaptive_brain.as_ref(),
        population: st.controller.population_manager.as_ref(),
    };

    SnapshotPayload {
        app_json: serde_json::to_string_pretty(&snap).ok(),
        app_path: st.config_paths.app_snapshot_path(),
        evo_json: serde_json::to_string_pretty(&ev).ok(),
        evo_path: st.config_paths.evolution_state_path(),
    }
}

/// Adım 2: Payload'ı diske yaz (lock gerekmez, zaten serbest).
fn flush_snapshot_payload(payload: SnapshotPayload) {
    if let Some(json) = payload.app_json {
        let _ = std::fs::write(payload.app_path, json);
    }
    if let Some(json) = payload.evo_json {
        let _ = std::fs::write(payload.evo_path, json);
    }
}

/// Geriye dönük uyumluluk: lock tutarken çağrılabilir (background worker'lar için).
/// Lock dışında çağrılacak yerlerde collect_snapshot_payload + flush_snapshot_payload kullan.
fn save_app_snapshot(st: &AppState) {
    flush_snapshot_payload(collect_snapshot_payload(st));
}

fn load_app_snapshot(paths: &ConfigPaths) -> AppSnapshot {
    let path = paths.app_snapshot_path();
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(snap) = serde_json::from_str::<AppSnapshot>(&text) {
            // Eski şema sürümünü algıla — sembol adayları ve skor alanları geçersiz olabilir
            if snap.schema_version < APP_SNAPSHOT_VERSION {
                eprintln!(
                    "[snapshot] Eski şema sürümü {} tespit edildi (beklenen: {}). \
                     symbol_candidates sıfırlanıyor.",
                    snap.schema_version, APP_SNAPSHOT_VERSION
                );
                // Trade sayacı, risk politikası ve ML verileri korunur; skor bağımlı alanlar sıfırlanır
                return AppSnapshot {
                    schema_version: APP_SNAPSHOT_VERSION,
                    total_trades:   snap.total_trades,
                    download_count: snap.download_count,
                    risk_max_notional_usd:     snap.risk_max_notional_usd,
                    risk_max_daily_loss_pct:   snap.risk_max_daily_loss_pct,
                    risk_max_drawdown_pct:     snap.risk_max_drawdown_pct,
                    risk_min_model_confidence: snap.risk_min_model_confidence,
                    saved_positions:           snap.saved_positions,
                    saved_closed_trades:       snap.saved_closed_trades,
                    // symbol_candidates, active_symbol, best_strategy_name → Default (boş)
                    ..AppSnapshot::default()
                };
            }
            return snap;
        }
    }
    AppSnapshot::default()
}

fn clear_all_snapshots(paths: &ConfigPaths) {
    let _ = std::fs::remove_file(paths.app_snapshot_path());
    let _ = std::fs::remove_file(paths.fsm_state_path());
}

// ─── DB Cache Adapter ────────────────────────────────────────────────────────
// BinanceLiveAdapter'ı sarar: her candle fetch sonrası DB'ye yazar.
// supported_markets/symbols config'den gelir — hardcode kalmaz.

struct DbCachingLiveAdapter {
    inner:       BinanceLiveAdapter,
    db_path:     String,
    exchange:    String,
    market_str:  String,   // "spot" | "futures"
    market_enum: Market,
    symbols:     Vec<String>,
}

#[async_trait::async_trait]
impl memos_trading_core::robot::interfaces::LiveDataFetcher for DbCachingLiveAdapter {
    fn source_name(&self) -> &str { "binance-db-cache" }

    fn supported_markets(&self) -> Vec<Market> { vec![self.market_enum] }

    fn supported_symbols(&self, _market: Market) -> Vec<String> { self.symbols.clone() }

    async fn fetch_latest(
        &self,
        exchange: Exchange,
        market: Market,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> memos_trading_core::Result<Vec<memos_trading_core::types::Candle>> {
        let candles = self.inner.fetch_latest(exchange, market, symbol, interval, limit).await?;

        // Yeni mum verilerini DB'ye yaz (spawn_blocking ile async bloke olmaz)
        let db_path    = self.db_path.clone();
        let exch       = self.exchange.clone();
        let mkt        = self.market_str.clone();
        let to_write   = candles.clone();
        tokio::task::spawn_blocking(move || {
            if let Ok(conn) = database_writer::open_connection(&db_path) {
                let (ins, _skip) = database_writer::save_candles_bulk(&conn, &exch, &mkt, &to_write)
                    .unwrap_or((0, 0));
                if ins > 0 {
                    // sessiz — log spam önlenir
                }
            }
        });

        Ok(candles)
    }
}

// ─── Otonom Veri İndirme Worker ───────────────────────────────────────────────
// ─── Adaptif Risk Politikası Hesaplayıcı ────────────────────────────────────
// Backtest sonuçlarından otomatik olarak optimum risk sınırlarını türetir:
//   max_drawdown_pct   = gözlenen_dd × 1.5        → [5%, 15%]
//   max_daily_loss_pct = max_drawdown / 3          → [1%,  5%]
//   max_notional_usd   = sermaye × win_rate        → [%20, %100] sermaye
//   min_model_confidence = 0.80 - win_rate × 0.30 → [0.50, 0.75]
fn compute_adaptive_policy(capital: f64, win_rate: f64, max_drawdown_pct: f64) -> RiskGatePolicy {
    // Drawdown 0.0 ise (hiç kayıp olmadı) konservatif varsayılanı tut
    let dd       = if max_drawdown_pct > 0.0 { max_drawdown_pct } else { 10.0 };
    let new_dd   = (dd * 1.5).clamp(5.0, 15.0);
    let new_day  = (new_dd / 3.0).clamp(1.0, 5.0);
    let wr       = win_rate / 100.0;
    let new_not  = (capital * wr).clamp(capital * 0.20, capital * 1.0);
    let new_conf = (0.80 - wr * 0.30).clamp(0.50, 0.75);
    RiskGatePolicy {
        max_notional_usd:     new_not,
        max_daily_loss_pct:   new_day,
        max_drawdown_pct:     new_dd,
        min_model_confidence: new_conf,
    }
}

/// Backtest sonuçlarından otonom SL/TP yüzdesi türetir.
/// Win rate düşükse risk/ödül oranı artırılır (daha büyük TP).
/// Drawdown yüksekse SL sıkılaştırılır.
/// Döndürür: (stop_loss_pct, take_profit_pct)
fn compute_adaptive_sl_tp(win_rate: f64, max_drawdown_pct: f64) -> (f64, f64) {
    let wr = win_rate / 100.0; // 0.0–1.0
    let dd = if max_drawdown_pct > 0.0 { max_drawdown_pct } else { 10.0 };
    // Stop-loss: drawdown'un ~%25'i, [0.5%, 4.0%] arasında sınırlı
    let sl = (dd / 4.0).clamp(0.5, 4.0);
    // Risk/ödül oranı: win rate düşükse daha büyük ödül gerekir
    //   wr ≥ 0.60 → rr = 1.5   (kazanma yüksek, küçük ödül yeter)
    //   wr ≈ 0.50 → rr = 2.0   (denge noktası)
    //   wr ≤ 0.40 → rr = 3.0   (kazanma düşük, büyük ödül şart)
    let rr = if wr >= 0.60 { 1.5 }
             else if wr >= 0.50 { 2.0 }
             else if wr >= 0.40 { 2.5 }
             else { 3.0 };
    let tp = (sl * rr).clamp(1.0, 8.0);
    (sl, tp)
}

// Her N dakikada bir (veya [d] ile anında) Binance REST API'den mum indirir:
// ─── 1m → Üst Zaman Dilimleri Türetici ──────────────────────────────────────
/// İndirilen 1m mumlardan 5m / 15m / 30m / 1h / 4h / 1d mumlarını türetir
/// ve doğrudan ana `candles` tablosuna yazar (symbol selector ve backtest kullanır).
///
/// Algoritma: BTreeMap ile bucket gruplama
///   open  = ilk gelen mum (BTreeMap sıralı → insert ile gelir)
///   high  = MAX(high)
///   low   = MIN(low)
///   close = son gelen mum (and_modify ile güncellenir)
///   volume = SUM(volume)
///

/// Emoji ve Unicode sembol karakterlerini ASCII karşılıklarıyla değiştirir.
/// Böylece export dosyası her editörde/viewer'da doğru görünür.
/// Stablecoin-stablecoin çiftleri backtest'te anlamsız sonuç verir (BTCUSDC, ETHUSDC, USDCUSDT vb.)
/// Base veya quote'un her ikisi de stablecoin ise true döner.
/// $0.05 altı nano-cap tokenlar filtrele (ADA/TRX/DOGE gibi meşru coinler etkilenmez)
fn is_low_price(last_price: f64) -> bool {
    last_price > 0.0 && last_price < 0.05
}

/// Son mum 30 günden eskiyse veri bayat → filtrele (0 = bilinmiyor, geçilir)
fn is_stale_data(last_candle_ts: i64) -> bool {
    if last_candle_ts == 0 { return false; }
    let now = chrono::Utc::now().timestamp();
    now - last_candle_ts > 30 * 24 * 3600
}

/// Screener için piyasa string'ini normalleştirir (config.market → download hedefi için)
fn market_str_for_screener(market: &str) -> String {
    match market.to_lowercase().as_str() {
        "futures" | "usdm" => "futures".to_string(),
        "coinm"            => "coinm".to_string(),
        _                  => "spot".to_string(),
    }
}

fn is_stablecoin_pair(symbol: &str) -> bool {
    const STABLES: &[&str] = &["USDC", "BUSD", "FDUSD", "TUSD", "USDP", "DAI"];
    let sym = symbol.to_uppercase();
    // İkisi de stablecoin mi? (örn. USDCUSDT, BTCUSDC, ETHUSDC)
    // Quote para birimi zaten USDT — base stablecoin ise filtrele
    STABLES.iter().any(|s| sym.starts_with(s) || sym.ends_with(s))
        && STABLES.iter().any(|s| sym.len() > s.len() && (sym.starts_with(s) || sym.ends_with(s)))
}

fn strip_emoji(s: &str) -> String {
    s.chars().filter(|c| {
        let u = *c as u32;
        // ASCII ve temel Latin genişletmesi (Türkçe dahil) tut
        // Emoji blokları, Dingbats, özel semboller çıkar
        u < 0x2500 || (u >= 0x2500 && u <= 0x257F) // box-drawing kalsın (─═)
    }).collect()
}

/// `"BTCUSDT-Futures"` gibi bileşik anahtar dizelerinden saf sembolü çıkarır.
/// Tire yoksa orijinal string'i döner.
fn bare_symbol(key: &str) -> &str {
    key.splitn(2, '-').next().unwrap_or(key)
}

/// `app_state` kilidini kısa süreliğine alıp `live_risk` Arc'ını klonlar.
/// ML worker gibi uzun yaşayan thread'lerde mutex'i uzun süre tutmamak için kullanılır.
fn clone_live_risk(app_state: &std::sync::Arc<std::sync::Mutex<AppState>>)
    -> std::sync::Arc<std::sync::RwLock<LiveRiskMap>>
{
    let st = app_state.lock().unwrap();
    std::sync::Arc::clone(&st.live_risk)
}

/// 30 saniyelik timeout ile ortak HTTP client oluşturur.
/// İki veya daha fazla worker aynı bloğu tekrar kullanmak yerine bunu çağırır.
fn build_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP client hatası: {e}"))
}

/// Standart işlem interval'leri — lokalde tekrar tekrar tanımlanmak yerine buradan kullanılır.
const STANDARD_INTERVALS: &[&str] = &["1m", "5m", "15m", "30m", "1h", "4h", "1d"];

/// Interval string'ini milisaniyeye çevirir.
/// "1m" → 60_000, "1h" → 3_600_000 vb.
/// Tanınmayan interval için 3_600_000 (1 saat) döner.
fn interval_to_ms(interval: &str) -> i64 {
    match interval {
        "1m"  =>      60_000,
        "3m"  =>     180_000,
        "5m"  =>     300_000,
        "15m" =>     900_000,
        "30m" =>   1_800_000,
        "1h"  =>   3_600_000,
        "2h"  =>   7_200_000,
        "4h"  =>  14_400_000,
        "6h"  =>  21_600_000,
        "1d"  =>  86_400_000,
        _     =>   3_600_000,
    }
}

/// Standart strateji değerlendirme skoru: win_rate + profit_factor + sharpe + drawdown.
/// compare_strategies ve ML/HyperOpt döngülerinde tutarlı hesaplama için.
fn score_backtest_result(
    win_rate: f64,
    profit_factor: f64,
    sharpe_ratio: f64,
    max_drawdown_pct: f64,
) -> f64 {
    (win_rate / 100.0) * 0.35
        + (profit_factor / 3.0).clamp(0.0, 1.0) * 0.30
        + (sharpe_ratio / 2.0).clamp(0.0, 1.0) * 0.20
        + (1.0 - max_drawdown_pct / 20.0).clamp(0.0, 1.0) * 0.15
}

/// Sembol verisinin işleme alınabilir kalitede olup olmadığını denetler.
/// compute_symbol_score ile tutarlı; tüm "0.0 döndür" dallarını kapsar.
fn is_valid_symbol_data(
    candle_count: usize,
    symbol: &str,
    last_price: f64,
    last_candle_ts: i64,
) -> bool {
    candle_count >= 30
        && !is_stablecoin_pair(symbol)
        && !is_low_price(last_price)
        && last_price > 0.0
        && !is_stale_data(last_candle_ts)
}

///// UTC zaman damgasından bu ana kadar geçen süreyi döndürür.
/// Desteklenen formatlar: "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%SZ", "%Y-%m-%dT%H:%M:%S"
/// Boş string veya parse hatası durumunda "—" döner.
fn format_duration_since(opened_at: &str) -> String {
    if opened_at.is_empty() { return "—".to_string(); }
    let Some(dt) = parse_ts(opened_at) else {
        return "—".to_string();
    };
    let secs = (chrono::Utc::now() - dt).num_seconds().max(0) as u64;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 { format!("{}sa {}dk", h, m) } else { format!("{}dk {}sn", m, s) }
}

/// "%Y-%m-%d %H:%M:%S" formatındaki UTC string'i DateTime'a çevirir.
/// Eski snapshot uyumluluğu için ISO-8601 ve saat-sadece formatlarına da fallback yapar.
fn parse_ts(ts: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%SZ"))
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S"))
        .ok()
        .map(|ndt| ndt.and_utc())
}

/// İki UTC zaman damgası ("%Y-%m-%d %H:%M:%S") arasındaki süreyi döndürür.
fn format_duration_between(open_ts: &str, close_ts: &str) -> String {
    let (Some(odt), Some(cdt)) = (parse_ts(open_ts), parse_ts(close_ts)) else {
        return "—".to_string();
    };
    let secs = (cdt - odt).num_seconds().abs() as u64;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 { format!("{}sa {}dk", h, m) }
    else if m > 0 { format!("{}dk {}sn", m, s) }
    else { format!("{}sn", s) }
}

/// Otonom Optimize — mevcut session istatistiklerine göre adaptive_params'ı hesaplar ve uygular.
/// Ayarlar panelinde item 27 "→" tuşuyla tetiklenir.
/// Her parametre değişikliği log'a yazılır; sonunda disk'e kaydedilir.
fn auto_tune_adaptive_params(st: &mut AppState) {
    let mut changes: Vec<String> = Vec::new();

    // ── İstatistikleri hesapla ──────────────────────────────────────────────
    let (win_rate, session_rr, session_closed) = {
        match st.live_risk.read() {
            Ok(lr) => {
                let wr = if lr.session_closed > 0 {
                    lr.session_wins as f64 / lr.session_closed as f64 * 100.0
                } else { 0.0 };
                (wr, lr.session_rr, lr.session_closed)
            }
            Err(_) => (0.0, 0.0, 0),
        }
    };

    let (tsl_ratio, sl_ratio, tp_ratio, short_loss_ratio, instant_tsl_ratio, avg_trade_dur_secs) = {
        match st.live_closed_trades.read() {
            Ok(closed) => {
                let n = closed.len();
                if n == 0 {
                    (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0u64)
                } else {
                    let tsl_n = closed.iter().filter(|t| t.exit_reason.contains("trailing")).count();
                    let sl_n  = closed.iter().filter(|t| {
                        let r = &t.exit_reason;
                        (r.contains("sl") || r.contains("SL") || r == "static_sl")
                            && !r.contains("trailing")
                    }).count();
                    let tp_n  = closed.iter().filter(|t|
                        t.exit_reason.contains("take_profit")).count();
                    let short_total = closed.iter().filter(|t| !t.is_long).count();
                    let short_loss  = closed.iter().filter(|t| !t.is_long && t.pnl < 0.0).count();

                    // Anlık TSL: açılıp 60 sn içinde kapanan
                    let tsl_trades: Vec<_> = closed.iter().filter(|t|
                        t.exit_reason.contains("trailing")).collect();
                    let instant_tsl = tsl_trades.iter().filter(|t| {
                        let dur = parse_ts(&t.opened_at).and_then(|o|
                            parse_ts(&t.closed_at).map(|c| (c - o).num_seconds().abs()));
                        dur.map(|d| d < 60).unwrap_or(false)
                    }).count();

                    // Ortalama trade süresi
                    let dur_secs: Vec<u64> = closed.iter().filter_map(|t| {
                        let o = parse_ts(&t.opened_at)?;
                        let c = parse_ts(&t.closed_at)?;
                        Some((c - o).num_seconds().abs() as u64)
                    }).collect();
                    let avg_dur = if dur_secs.is_empty() { 0u64 }
                        else { dur_secs.iter().sum::<u64>() / dur_secs.len() as u64 };

                    (
                        tsl_n as f64 / n as f64,
                        sl_n  as f64 / n as f64,
                        tp_n  as f64 / n as f64,
                        if short_total > 0 { short_loss as f64 / short_total as f64 } else { 0.0 },
                        if tsl_n > 0 { instant_tsl as f64 / tsl_n as f64 } else { 0.0 },
                        avg_dur,
                    )
                }
            }
            Err(_) => (0.0, 0.0, 0.0, 0.0, 0.0, 0),
        }
    };

    let enough_data = session_closed >= 3; // 3 trade'den itibaren otonom ayar başlar

    // ── 0. Acil: Sıfır win rate erken müdahale ────────────────────────────
    // 2+ trade hepsi kaybedilmişse → TSL aktivasyon ve SL ATR hemen ayarla
    if session_closed >= 2 && win_rate == 0.0 {
        // TSL aktivasyonu artır
        let old_tsl = st.adaptive_params.trailing_sl_activation_pct;
        let new_tsl = (old_tsl + 1.0).min(5.0);
        if (new_tsl - old_tsl).abs() > 1e-9 {
            st.adaptive_params.trailing_sl_activation_pct = new_tsl;
            changes.push(format!("🚨 0% WR acil: TSL aktivasyon {:.1}% → {:.1}%", old_tsl, new_tsl));
        }
        // SL ATR çarpanını artır (SL'yi uzat — daha az erken çıkış)
        let old_sl = st.adaptive_params.sl_atr_multiplier;
        let new_sl = (old_sl + 0.25).min(2.5);
        if (new_sl - old_sl).abs() > 1e-9 {
            st.adaptive_params.sl_atr_multiplier = new_sl;
            changes.push(format!("🚨 0% WR acil: SL ATR çarpanı {:.2}x → {:.2}x", old_sl, new_sl));
        }
        // Max ardışık kayıp koruması
        if st.adaptive_params.max_consecutive_losses == 0 {
            st.adaptive_params.max_consecutive_losses = 4;
            changes.push("🚨 0% WR acil: Max ardışık kayıp sınırı 4 olarak açıldı".to_string());
        }
    }

    // ── 1. TP ATR Çarpanı ──────────────────────────────────────────────────
    // Kural: TP oranı < %20 (nadiren TP'ye ulaşılıyor) → çarpanı küçült
    //        TP oranı > %60 (her trade TP'ye gidiyor) → çarpanı biraz artır
    if enough_data {
        let old = st.adaptive_params.tp_atr_multiplier;
        let new_val = if tp_ratio < 0.20 {
            // TP hiç tetiklenmiyor → hedef çok uzak
            (old - 0.3).max(0.8)
        } else if tp_ratio > 0.60 && win_rate > 55.0 {
            // Sık TP + yüksek win → biraz açılabilir
            (old + 0.1).min(2.0)
        } else if win_rate < 35.0 {
            // Genel win rate düşük → TP'yi küçült
            (old - 0.2).max(0.8)
        } else {
            old
        };
        if (new_val - old).abs() > 1e-9 {
            st.adaptive_params.tp_atr_multiplier = new_val;
            changes.push(format!("TP ATR çarpanı {:.2}x → {:.2}x (tp_oran={:.0}% wr={:.0}%)",
                old, new_val, tp_ratio * 100.0, win_rate));
        }
    }

    // ── 2. TSL Aktivasyon % ────────────────────────────────────────────────
    // Kural: TSL anlık tetiklenme yüksekse → aktivasyonu artır (daha geç devreye girsin)
    //        TSL kapanış oranı > %50 ama kâr yok → aktivasyonu artır
    if enough_data {
        let old = st.adaptive_params.trailing_sl_activation_pct;
        let new_val = if instant_tsl_ratio > 0.3 {
            // Anlık TSL %30'dan fazla → aktivasyon eşiği yetersiz
            (old + 1.0).min(5.0)
        } else if tsl_ratio > 0.50 && win_rate < 45.0 {
            // TSL ağırlıklı & win düşük → aktivasyon artır
            (old + 0.5).min(4.0)
        } else if tsl_ratio < 0.10 && avg_trade_dur_secs > 3600 {
            // TSL hiç tetiklenmiyor & trade uzun sürüyor → aktivasyonu düşür (kârı koru)
            (old - 0.2).max(0.5)
        } else {
            old
        };
        if (new_val - old).abs() > 1e-9 {
            st.adaptive_params.trailing_sl_activation_pct = new_val;
            changes.push(format!("TSL aktivasyon {:.1}% → {:.1}% (anlık_tsl={:.0}% tsl_oran={:.0}%)",
                old, new_val, instant_tsl_ratio * 100.0, tsl_ratio * 100.0));
        }
    }

    // ── 3. SHORT HTF Blok ──────────────────────────────────────────────────
    // Kural: SHORT kayıp oranı > %60 (2+ SHORT yeterli) → SHORT HTF blok aç
    if session_closed >= 2 && short_loss_ratio > 0.60 && !st.adaptive_params.short_htf_block {
        st.adaptive_params.short_htf_block = true;
        changes.push(format!("SHORT HTF Blok AÇILDI (short kayıp oranı={:.0}%)", short_loss_ratio * 100.0));
    }

    // ── 4. Günlük SL Limiti ────────────────────────────────────────────────
    // Kural: SL oranı > %50 & günlük limit kapalı → 3/gün aç
    if enough_data && sl_ratio > 0.50 && st.adaptive_params.max_daily_sl_per_symbol == 0 {
        st.adaptive_params.max_daily_sl_per_symbol = 3;
        changes.push(format!("Günlük SL Limiti AÇILDI → 3/gün (sl_oran={:.0}%)", sl_ratio * 100.0));
    }

    // ── 5. Max Ardışık Kayıp ───────────────────────────────────────────────
    // Kural: RR < 0.8 & otonom mod kapalı → max_consecutive_losses = 4 devreye al
    if enough_data && session_rr < 0.8 && st.adaptive_params.max_consecutive_losses == 0 {
        st.adaptive_params.max_consecutive_losses = 4;
        changes.push(format!("Max Ardışık Kayıp AÇILDI → 4 (rr={:.2})", session_rr));
    }

    // ── 6. Otonom Mod (adjust_every_n_trades) ─────────────────────────────
    // Kural: 7+ trade & otonom kapalı → her 5 işlemde bir aç
    if session_closed >= 7 && st.adaptive_params.adjust_every_n_trades == 0 {
        st.adaptive_params.adjust_every_n_trades = 5;
        changes.push("Otonom Mod AÇILDI → her 5 işlem (yeterli veri birikti)".to_string());
    }

    // ── 7. SL ATR Çarpanı ─────────────────────────────────────────────────
    // Kural: SL oranı > %50 & ortalama trade kısa (<5dk) → SL ATR çarpanını artır
    if enough_data && sl_ratio > 0.50 && avg_trade_dur_secs < 300 {
        let old = st.adaptive_params.sl_atr_multiplier;
        let new_val = (old + 0.25).min(2.5);
        if (new_val - old).abs() > 1e-9 {
            st.adaptive_params.sl_atr_multiplier = new_val;
            changes.push(format!("SL ATR çarpanı {:.2}x → {:.2}x (sl_oran={:.0}%, ort_süre={}sn)",
                old, new_val, sl_ratio * 100.0, avg_trade_dur_secs));
        }
    }

    // ── 8. UCB1 Scorer — disk'ten oku, strateji disable durumu yansıt ────────
    // Scorer restart sonrası korunduğu için burada da okunabilir.
    if let Ok(content) = std::fs::read_to_string("config/strategy_scorer_state.json") {
        #[derive(serde::Deserialize)]
        struct ScorerSnap {
            scalp_disabled: bool,
            swing_disabled: bool,
            reg_disabled:   bool,
            total_n:        u32,
            last_reason:    String,
        }
        if let Ok(snap) = serde_json::from_str::<ScorerSnap>(&content) {
            // UCB1 birden fazla stratejiyi kapattıysa → SL ATR'i koru (volatilite yüksek)
            let n_disabled = [snap.scalp_disabled, snap.swing_disabled, snap.reg_disabled]
                .iter().filter(|&&x| x).count();
            if n_disabled >= 2 && enough_data {
                let old = st.adaptive_params.sl_atr_multiplier;
                let new_val = (old + 0.25).min(2.5);
                if (new_val - old).abs() > 1e-9 {
                    st.adaptive_params.sl_atr_multiplier = new_val;
                    changes.push(format!(
                        "UCB1: {} strateji devre dışı ({}) → SL ATR {:.2}x → {:.2}x",
                        n_disabled, snap.last_reason, old, new_val
                    ));
                }
            }
            // Scalp kapalıyken TP_ATR genişletme (swing/regular daha fazla bekleme)
            if snap.scalp_disabled && !snap.swing_disabled && enough_data {
                let old = st.adaptive_params.tp_atr_multiplier;
                if old < 2.5 {
                    let new_val = (old + 0.2).min(2.5);
                    st.adaptive_params.tp_atr_multiplier = new_val;
                    changes.push(format!(
                        "UCB1: Scalp kapalı, Swing aktif → TP ATR {:.1}x → {:.1}x (daha uzun tutuş)",
                        old, new_val
                    ));
                }
            }
            if snap.total_n > 0 {
                st.push_log(format!("   📊 UCB1 Scorer: {} işlem analiz edildi (scalp={} swing={} reg={})",
                    snap.total_n,
                    if snap.scalp_disabled { "❌" } else { "✓" },
                    if snap.swing_disabled { "❌" } else { "✓" },
                    if snap.reg_disabled   { "❌" } else { "✓" },
                ));
            }
        }
    }

    // ── 9. ML Classifier — cold-start mi, eğitim ilerledi mi? ──────────────
    {
        #[derive(serde::Deserialize)]
        struct ClassifierSnap {
            classifier: ClassifierInfo,
            buffer: Vec<serde_json::Value>,
        }
        #[derive(serde::Deserialize)]
        struct ClassifierInfo {
            is_trained: bool,
            n_win:      usize,
            n_loss:     usize,
        }
        if let Ok(content) = std::fs::read_to_string("config/classifier_state.json") {
            if let Ok(snap) = serde_json::from_str::<ClassifierSnap>(&content) {
                let n_total  = snap.buffer.len();
                let clf      = &snap.classifier;
                if !clf.is_trained {
                    // Cold-start: min_rr eşiğini koru
                    st.push_log(format!(
                        "   🧠 ML Classifier: cold-start ({}/{} örnek) — min_rr korunacak",
                        n_total, 20
                    ));
                    // ML henüz eğitilmemişken SHORT eşiğini kaldır (cold-start bloğu zaten var)
                    if st.adaptive_params.futures_short_min_conf > 0.55 {
                        let old = st.adaptive_params.futures_short_min_conf;
                        st.adaptive_params.futures_short_min_conf = 0.50;
                        changes.push(format!(
                            "ML cold-start: SHORT eşiği {:.2} → 0.50 (classifier henüz eğitilmedi)",
                            old
                        ));
                    }
                } else {
                    // Eğitilmiş: win/loss denge durumu
                    let total = (clf.n_win + clf.n_loss) as f64;
                    let ml_wr = if total > 0.0 { clf.n_win as f64 / total * 100.0 } else { 0.0 };
                    st.push_log(format!(
                        "   🧠 ML Classifier: aktif — kazanç={} kayıp={} WR={:.0}%",
                        clf.n_win, clf.n_loss, ml_wr
                    ));
                    // ML win rate < 40% ama session da düşükse → SHORT ML eşiğini artır
                    if ml_wr < 40.0 && win_rate < 45.0 && enough_data {
                        let old = st.adaptive_params.futures_short_min_conf;
                        let new_val = (old + 0.05).min(0.70);
                        if (new_val - old).abs() > 1e-9 {
                            st.adaptive_params.futures_short_min_conf = new_val;
                            changes.push(format!(
                                "ML WR={:.0}%+session WR={:.0}% → SHORT eşiği {:.2} → {:.2}",
                                ml_wr, win_rate, old, new_val
                            ));
                        }
                    }
                    // ML güçlüyse (WR>60%) → SHORT eşiğini biraz gevşet (false positive azalt)
                    if ml_wr > 60.0 && win_rate > 55.0 {
                        let old = st.adaptive_params.futures_short_min_conf;
                        let new_val = (old - 0.05).max(0.35);
                        if (new_val - old).abs() > 1e-9 {
                            st.adaptive_params.futures_short_min_conf = new_val;
                            changes.push(format!(
                                "ML güçlü (ML WR={:.0}%) → SHORT eşiği {:.2} → {:.2}",
                                ml_wr, old, new_val
                            ));
                        }
                    }
                }
            }
        }
    }

    // ── 10. Equity Snapshot — session DD durumu yansıt ──────────────────────
    if let Ok(content) = std::fs::read_to_string("config/equity_snapshot.json") {
        #[derive(serde::Deserialize)]
        struct EqSnap { capital: f64, cumulative_pnl: f64, peak_equity: f64 }
        if let Ok(eq) = serde_json::from_str::<EqSnap>(&content) {
            let real_dd_pct = if eq.peak_equity > 0.0 {
                ((eq.peak_equity - (eq.capital + eq.cumulative_pnl)) / eq.peak_equity * 100.0).max(0.0)
            } else { 0.0 };
            st.push_log(format!(
                "   📈 Equity: capital=${:.0} cumPnL={:+.2} peak=${:.0} realDD={:.1}%",
                eq.capital, eq.cumulative_pnl, eq.peak_equity, real_dd_pct
            ));
            // Gerçek DD > %15 → max ardışık kayıp ve günlük SL limitini zorla
            if real_dd_pct > 15.0 {
                if st.adaptive_params.max_consecutive_losses == 0
                    || st.adaptive_params.max_consecutive_losses > 3
                {
                    st.adaptive_params.max_consecutive_losses = 3;
                    changes.push(format!(
                        "Gerçek DD {:.1}%>%15 → Max Ardışık Kayıp 3 olarak kısıtlandı",
                        real_dd_pct
                    ));
                }
                if st.adaptive_params.max_daily_sl_per_symbol == 0
                    || st.adaptive_params.max_daily_sl_per_symbol > 2
                {
                    st.adaptive_params.max_daily_sl_per_symbol = 2;
                    changes.push(format!(
                        "Gerçek DD {:.1}%>%15 → Günlük SL limiti 2/gün/sembol kısıtlandı",
                        real_dd_pct
                    ));
                }
            }
        }
    }

    // ── Sonuç ──────────────────────────────────────────────────────────────
    if changes.is_empty() {
        st.push_log(format!(
            "✅ Otonom Optimize: mevcut parametreler istatistiksel olarak makul \
             (WR={:.0}% RR={:.2} n={})",
            win_rate, session_rr, session_closed
        ));
    } else {
        st.push_log(format!(
            "🤖 Otonom Optimize uygulandı ({} değişiklik — WR={:.0}% RR={:.2} n={}):",
            changes.len(), win_rate, session_rr, session_closed
        ));
        for c in &changes {
            st.push_log(format!("   • {}", c));
        }
        st.save_adaptive_params();
        st.push_log("   💾 adaptive_params.json kaydedildi".to_string());
    }
}

fn build_export_report(st: &AppState) -> String {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let sep = "═".repeat(60);
    let mut out = String::with_capacity(8192);

    // UTF-8 BOM: editörler/viewer'lar encoding'i otomatik tanır
    out.push('\u{FEFF}');

    // ── Başlık ────────────────────────────────────────────────
    out.push_str(&format!("{}\n", sep));
    out.push_str(&format!("  MEMOS RTC -- DURUM RAPORU  [{}]\n", now));
    out.push_str(&format!("{}\n\n", sep));

    // ── DASHBOARD ─────────────────────────────────────────────
    out.push_str(&format!("── DASHBOARD {}\n", "─".repeat(48)));
    let mod_str = if st.paper_mode { "Kağıt (Paper)" } else { "CANLI" };
    out.push_str(&format!("RoboticLoop : Çalışıyor | Mod: {} | API: {}\n",
        mod_str, if st.api_key_set { "Yüklendi" } else { "EKSİK" }));

    let (exch, mkt, sym, intv) = st.active_trade_target();
    out.push_str(&format!("Aktif Hedef : {}/{} {} | {}\n", exch, mkt, sym, intv));

    let strat = st.live_strategy.read().ok()
        .map(|s| s.clone()).unwrap_or_else(|| "?".into());
    let regime_strat = st.live_regime_strategy.read().ok()
        .map(|s| s.clone()).unwrap_or_else(|| "—".into());
    out.push_str(&format!("Strateji    : {} | Aktif: {} | SL={:.1}% TP={:.1}% (rr={:.1}x)\n",
        strat, regime_strat, st.best_sl, st.best_tp,
        if st.best_sl > 0.0 { st.best_tp / st.best_sl } else { 0.0 }));
    let live_trades = st.live_trade_count.load(std::sync::atomic::Ordering::Relaxed);
    let realized_pnl = st.equity - st.config.capital;
    out.push_str(&format!("Sermaye     : ${:.2} (başlangıç=${:.2} realize={:+.2}$) | Toplam İşlem: {} (canlı loop: {})\n",
        st.equity, st.config.capital, realized_pnl, st.total_trades, live_trades));
    out.push_str(&format!("Otonom Mod  : {} | Aday Sayısı: {}\n",
        if st.auto_symbol { "AUTO" } else { "Manuel" }, st.symbol_candidates.len()));
    if let Some(ref bt) = st.last_backtest {
        out.push_str(&format!("Son Backtest: {}\n", bt));
    }
    if let Some(ref dl) = st.last_download {
        out.push_str(&format!("Son İndirme : {}\n", dl));
    }
    // Risk politikası
    let rp = &st.risk_gate.policy;
    let eff_conf = if st.paper_mode { rp.min_model_confidence.min(0.35) } else { rp.min_model_confidence };
    let conf_note = if st.paper_mode && rp.min_model_confidence > 0.35 { " (paper→0.35 efektif)" } else { "" };
    out.push_str(&format!("Risk        : MaxNotional=${:.0} DayLoss={:.1}% DD={:.1}% MinConf={:.2}{} (politika={:.2})\n",
        rp.max_notional_usd, rp.max_daily_loss_pct,
        rp.max_drawdown_pct, eff_conf, conf_note, rp.min_model_confidence));
    // Canlı fiyat (primary) — sadece aktif sembol eşleşiyorsa göster
    let primary_sym = st.active_symbol.symbol.clone();
    if let Ok(pd) = st.live_price.read() {
        if pd.close > 0.0 && pd.symbol == primary_sym {
            out.push_str(&format!("Canlı Fiyat : {} {:.4} @ {} (Yerel)\n",
                pd.symbol, pd.close, pd.ts));
        }
    }
    // Orkestratör özeti
    let wc = st.orchestrator.worker_count();
    out.push_str(&format!("Orkestratör : {} / {} aktif worker | Açık PnL: {:+.3} USDT\n",
        wc, st.orchestrator.max_workers,
        st.orchestrator.total_open_pnl(Some(&*st.live_price))));
    out.push('\n');

    // ── ÇOKLU SEMBOL / WORKER TABLOSU ─────────────────────────
    out.push_str(&format!("── ÇOKLU SEMBOL {}\n", "─".repeat(45)));
    if wc == 0 {
        out.push_str("  (Aktif multi-sembol worker yok)\n");
    } else {
        out.push_str(&format!("  {:<12} {:<9} {:<5} {:>12} {:>9} {:>10}  {}\n",
            "Sembol", "Market", "Int.", "Fiyat", "Değişim%", "Uptime", "Durum"));
        out.push_str(&format!("  {}\n", "─".repeat(70)));
        // Primary sembol — st.live_price boşsa orchestrator arc'ına düş (sembol geçişi sonrası)
        let (_, pm, ps, pi) = st.active_trade_target();
        {
            // st.live_price önce dene; sembol eşleşmiyorsa veya close=0 ise orchestrator arc'ına düş
            let (close, chg) = st.live_price.read().ok()
                .filter(|pd| pd.close > 0.0 && pd.symbol == ps)
                .map(|pd| (pd.close, pd.change_pct))
                .or_else(|| {
                    st.orchestrator.live_price_for(&ps)
                        .and_then(|arc| arc.read().ok().filter(|pd| pd.close > 0.0)
                            .map(|pd| (pd.close, pd.change_pct)))
                })
                .unwrap_or((0.0, 0.0));
            let price_s = if close > 0.0 { format!("{:>12.4}", close) } else { format!("{:>12}", "—") };
            let chg_s   = if close > 0.0 {
                let sym = if chg >= 0.0 { "▲" } else { "▼" };
                format!("{}{:.2}%", sym, chg.abs())
            } else { "—".to_string() };
            let uptime = st.loop_active_since.elapsed().as_secs();
            let h = uptime / 3600; let m = (uptime % 3600) / 60; let s = uptime % 60;
            out.push_str(&format!("  {:<12} {:<9} {:<5} {}  {:>8}  {:02}:{:02}:{:02}  {} (birincil)\n",
                ps, pm, pi, price_s, chg_s, h, m, s,
                if st.paused { "Duraklatıldı" } else { "Çalışıyor" }));
        }
        // Orchestrator workers
        for status in st.orchestrator.worker_status() {
            if status.symbol == ps { continue; }
            let (price_s, chg_s) = if let Some(arc) = st.orchestrator.live_price_for(&status.symbol) {
                arc.read().ok().map(|pd| {
                    let ps = if pd.close > 0.0 { format!("{:>12.4}", pd.close) } else { format!("{:>12}", "—") };
                    let cs = if pd.close > 0.0 {
                        let sym = if pd.change_pct >= 0.0 { "▲" } else { "▼" };
                        format!("{}{:.2}%", sym, pd.change_pct.abs())
                    } else { "—".to_string() };
                    (ps, cs)
                }).unwrap_or_else(|| (format!("{:>12}", "—"), "—".to_string()))
            } else { (format!("{:>12}", "—"), "—".to_string()) };
            let h = status.uptime_secs / 3600;
            let m = (status.uptime_secs % 3600) / 60;
            let s = status.uptime_secs % 60;
            out.push_str(&format!("  {:<12} {:<9} {:<5} {}  {:>8}  {:02}:{:02}:{:02}  {}\n",
                status.symbol, status.market, status.interval,
                price_s, chg_s, h, m, s,
                if status.paused { "Duraklatıldı" } else { "Çalışıyor" }));
        }
        // Config'in orijinal primary sembolü (örn. BTCUSDT) — orchestrator worker değil ama WS takip ediyor.
        // Birincil geçişten sonra listeden düşmemesi için ayrıca gösterilir.
        let cfg_sym = st.config.symbol.clone();
        let cfg_mkt = st.config.market.clone();
        if !cfg_sym.is_empty() && cfg_sym != ps && !st.orchestrator.workers.contains_key(&cfg_sym) {
            if let Some((cfg_close, cfg_chg)) = st.config_symbol_price.read().ok()
                .filter(|pd| pd.close > 0.0)
                .map(|pd| (pd.close, pd.change_pct))
            {
                let price_s = format!("{:>12.4}", cfg_close);
                let sym_c   = if cfg_chg >= 0.0 { "▲" } else { "▼" };
                let chg_s   = format!("{}{:.2}%", sym_c, cfg_chg.abs());
                out.push_str(&format!("  {:<12} {:<9} {:<5} {}  {:>8}  {:>10}  {}\n",
                    cfg_sym, cfg_mkt, "—", price_s, chg_s, "—", "İzleniyor"));
            }
        }
    }
    out.push('\n');

    // ── ML / AI ───────────────────────────────────────────────
    out.push_str(&format!("── ML/AI {}\n", "─".repeat(52)));
    out.push_str(&format!("ML Sinyal   : {} | Güven: {:.3} | Ham Skor: {:.4}\n",
        st.ml_signal, st.ml_confidence, st.ml_score));
    out.push_str(&format!("Eğitim      : {} veri noktası\n", st.ml_train_count));
    out.push_str(&format!("HyperOpt    : fast={} slow={} | Skor={:.6}\n",
        st.best_fast, st.best_slow, st.hyperopt_score));
    out.push_str(&format!("RSI HyperOpt: period={} OB={:.0}% OS={:.0}%\n",
        st.best_rsi_period, st.best_rsi_ob, st.best_rsi_os));
    out.push_str(&format!("BB  HyperOpt: period={} σ={:.1}\n",
        st.best_bb_period, st.best_bb_std_dev));
    out.push_str(&format!("MACD HyperOpt: fast={} slow={} signal={}\n",
        st.best_macd_fast, st.best_macd_slow, st.best_macd_signal));
    // Aktif strateji parametreleri (loop'ta gerçekten kullanılan)
    if let Ok(lr) = st.live_risk.read() {
        let strat_now = st.live_strategy.read().ok()
            .map(|s| s.clone()).unwrap_or_else(|| "?".into());
        let params_str = match strat_now.to_uppercase().as_str() {
            s if s.contains("RSI") =>
                format!("period={} OB={:.0}% OS={:.0}%",
                    lr.global_rsi_period, lr.global_rsi_ob, lr.global_rsi_os),
            s if s.contains("BOLLINGER") | s.contains("BB") =>
                format!("period={} σ={:.1}",
                    lr.global_bb_period, lr.global_bb_std_dev),
            s if s.contains("MACD") =>
                format!("fast={} slow={} signal={}",
                    lr.global_macd_fast, lr.global_macd_slow, lr.global_macd_signal),
            _ =>
                format!("fast={} slow={}", lr.global_fast, lr.global_slow),
        };
        out.push_str(&format!("Aktif Params: [{}] {}\n", strat_now, params_str));
    }
    if let Some(ref mt) = st.last_ml_train {
        out.push_str(&format!("Son Eğitim  : {}\n", mt));
    }
    if let Some(ref bt) = st.last_backtest_at {
        out.push_str(&format!("Son Backtest: {} (aynı mum verisi = skor stabil, bu normaldir)\n", bt));
    }
    // OOS + Ensemble metrikleri (ML kalite göstergeleri)
    if let Ok(lr) = st.live_risk.read() {
        let gbt_s = lr.gbt_last_score.map(|s| format!("{:+.4}", s)).unwrap_or_else(|| "—".into());
        let folds_s = format!("[{:.2}, {:.2}, {:.2}]",
            lr.oos_fold_scores[0], lr.oos_fold_scores[1], lr.oos_fold_scores[2]);
        out.push_str(&format!(
            "OOS Metriks : WinRate={:.1}% AvgReturn={:+.3}% BarCount={} | Folds={}\n",
            lr.oos_win_rate, lr.oos_avg_return, lr.oos_bar_count, folds_s));
        out.push_str(&format!(
            "Ensemble    : Agreement={:.1}% GBT={} | ML çalışıyor={}\n",
            lr.ensemble_agreement * 100.0, gbt_s,
            if lr.ml_running { "EVET" } else { "Hayır" }));
    }
    out.push('\n');

    // ── P5 CRYPTO ANALİZİ ─────────────────────────────────────
    out.push_str(&format!("── P5 CRYPTO ANALİZİ {}\n", "─".repeat(40)));
    if let Some(ref p5) = st.p5_last_status {
        let state_icon = match p5.state.as_str() {
            "done"    => "✓",
            "running" => "⟳",
            "error"   => "✗",
            _         => "—",
        };
        out.push_str(&format!("Durum       : {} {} | {} {}\n",
            state_icon, p5.state, p5.symbol, p5.interval));
        out.push_str(&format!("Zaman       : {}\n", p5.ts));
        if !p5.msg.is_empty() {
            out.push_str(&format!("Mesaj       : {}\n", p5.msg));
        }
        if p5.state == "done" || p5.strategies_found > 0 {
            out.push_str(&format!("Stratejiler : {}/{} geçerli edge | WF tutarlılık: {:.0}%\n",
                p5.edge_confirmed, p5.strategies_found, p5.wf_consistency * 100.0));
            if !p5.best_name.is_empty() {
                out.push_str(&format!("En İyi Strat: {} | WR={:.1}% PF={:.2} DD={:.1}% TP={}x SL={}x p={:.4}\n",
                    p5.best_name, p5.best_wr * 100.0, p5.best_pf, p5.best_dd * 100.0,
                    p5.best_tp_mult, p5.best_sl_mult, p5.best_p_value));
                out.push_str(&format!("Edge        : {}\n", p5.best_edge));
            }
            if p5.mc_prob_profit > 0.0 || p5.ruin_pct > 0.0 {
                out.push_str(&format!("Monte Carlo : Kâr Olas.={:.1}% | Ruin={:.1}%\n",
                    p5.mc_prob_profit * 100.0, p5.ruin_pct * 100.0));
            }
            // Aktif sinyal
            if !p5.active_dir.is_empty() {
                let dir_arrow = if p5.active_dir.to_uppercase().contains("LONG") { "▲" } else { "▼" };
                out.push_str(&format!("Aktif Sinyal: {} {} | TP={}x SL={}x ATR | ATR={:.4} | p={:.4}\n",
                    dir_arrow, p5.active_dir,
                    p5.best_tp_mult, p5.best_sl_mult,
                    p5.active_atr, p5.best_p_value));
            } else {
                out.push_str("Aktif Sinyal: Yok\n");
            }
            // Top 3 stratejiler
            if !p5.top_strategies.is_empty() {
                out.push_str(&format!("  {:<30} {:>5}  {:>6}  {:>5}  {:>5}  {:>3}  {:<16}  {}\n",
                    "Strateji", "WR%", "PF", "DD%", "p-val", "WF", "Edge", "Yön"));
                out.push_str(&format!("  {}\n", "─".repeat(90)));
                for (i, s) in p5.top_strategies.iter().enumerate() {
                    out.push_str(&format!("  #{} {:<28} {:>5.1}%  {:>6.2}  {:>5.1}%  {:>5.4}  {}/3  {:<16}  {}\n",
                        i + 1, s.name,
                        s.wr * 100.0, s.pf, s.dd * 100.0,
                        s.p_value, s.wf_pass,
                        s.edge, s.direction));
                }
            }
        }
        if p5.tested > 0 {
            out.push_str(&format!("Test İlerle : {}/{} kombinasyon test edildi\n",
                p5.found_so_far, p5.tested));
        }
    } else {
        out.push_str("Henüz analiz çalıştırılmadı. [y] tuşu ile TUI'dan başlatabilirsiniz.\n");
    }
    out.push('\n');

    // ── MTF Fırsat Tarayıcısı ─────────────────────────────────────────────────
    out.push_str("══════ MTF FIRSAT TARAYICI ══════════════════════════════════════════════════\n");
    out.push_str(&format!("Son Tarama   : {}\n", st.mtf_last_scan.as_deref().unwrap_or("—")));
    if st.mtf_opportunities.is_empty() {
        out.push_str("Fırsat       : Henüz bulunamadı (skor > 0.15 gerekli, 90s aralık)\n");
    } else {
        out.push_str(&format!("Toplam       : {} fırsat\n\n", st.mtf_opportunities.len()));
        out.push_str(&format!("  {:<10}  {:<4}  {:<16}  {:>6}  {:>5}  {:<5}  {}\n",
            "Sembol", "Int.", "Strateji", "Skor", "WR%", "Yön", "Zaman"));
        out.push_str(&format!("  {}\n", "─".repeat(60)));
        for opp in &st.mtf_opportunities {
            out.push_str(&format!("  {:<10}  {:<4}  {:<16}  {:>6.2}  {:>5.1}%  {:<5}  {}\n",
                opp.symbol, opp.interval, &opp.strategy[..opp.strategy.len().min(16)],
                opp.score, opp.win_rate * 100.0, opp.direction, opp.found_at));
        }
    }
    out.push('\n');

    // Taze per-sembol fiyat haritası — aday tablosu ve pozisyonlar için ortak
    let export_prices = {
        let mut m = st.orchestrator.build_price_map(Some(&st.live_price));
        // Orphan pozisyon fiyatları: worker arc'ı olmayan semboller için pos.current_price ekle
        if let Ok(positions) = st.live_positions.read() {
            for (key, pos) in positions.iter() {
                let sym = bare_symbol(key).to_string();
                if !m.contains_key(&sym) && pos.current_price > 0.0 {
                    m.insert(sym, pos.current_price);
                }
            }
        }
        m
    };

    // Sembol adayları tablosu — score>0 olanlar tam gösterilir, score=0 olanlar kısaltılır
    out.push_str(&format!("  {:<10} {:<8} {:<6}  {:>11}  {:>8}  {:>5}  {:>5}  {:>4}  {:>6}  {:>8}  {:<14}  {}\n",
        "Sembol", "Market", "Int", "Fiyat", "WinRate", "PF", "DD%", "Trd", "Sharpe", "Skor", "Strateji", "Durum"));
    out.push_str(&format!("  {}\n", "─".repeat(107)));
    let active_key = (st.active_symbol.exchange.clone(), st.active_symbol.market.clone(), st.active_symbol.symbol.clone(), st.active_symbol.interval.clone());
    let (scored, zero_score): (Vec<_>, Vec<_>) = st.symbol_candidates.iter()
        .partition(|c| c.score > 0.0 || (c.exchange.clone(), c.market.clone(), c.symbol.clone(), c.interval.clone()) == active_key);
    for cand in &scored {
        let durum = if (cand.exchange.clone(), cand.market.clone(), cand.symbol.clone(), cand.interval.clone()) == active_key { "AKTİF" } else { "" };
        let (display_price, live_tag) = if let Some(&lp) = export_prices.get(&cand.symbol) {
            (lp, "*")
        } else {
            (cand.last_price, " ")
        };
        let price_str = if display_price >= 1000.0 { format!("{:>10.1}", display_price) }
            else if display_price >= 1.0 { format!("{:>10.4}", display_price) }
            else if display_price > 0.0  { format!("{:>10.6}", display_price) }
            else { format!("{:>10}", "-") };
        let strat = if cand.best_strategy.is_empty() { "—" } else { &cand.best_strategy };
        // Freshness: son mum ne kadar eski (saat cinsinden)
        let age_str = if cand.last_candle_ts > 0 {
            let age_h = (chrono::Utc::now().timestamp() - cand.last_candle_ts) / 3600;
            if age_h < 24 { format!("{}sa", age_h) } else { format!("{}g", age_h / 24) }
        } else { "?".to_string() };
        out.push_str(&format!("  {:<10} {:<8} {:<6}  {}{} {:>7.1}%  {:>5.2}  {:>5.1}  {:>4}  {:>6.2}  {:>8.4}  {:<14}  {} [{}mum,{}]\n",
            cand.symbol, cand.market, cand.interval,
            price_str, live_tag, cand.win_rate, cand.profit_factor, cand.max_drawdown_pct,
            cand.total_trades, cand.sharpe_ratio, cand.score, strat, durum,
            cand.candle_count, age_str));
    }
    // Score=0 olan adayları özetle (ilk 10 göster, kalanlar için sayı)
    if !zero_score.is_empty() {
        let show_n = zero_score.len().min(10);
        out.push_str(&format!("  ── Score=0 adaylar ({} toplam, ilk {} gösteriliyor) ──\n", zero_score.len(), show_n));
        for cand in zero_score.iter().take(show_n) {
            let (display_price, live_tag) = if let Some(&lp) = export_prices.get(&cand.symbol) {
                (lp, "*")
            } else {
                (cand.last_price, " ")
            };
            let price_str = if display_price >= 1000.0 { format!("{:>10.1}", display_price) }
                else if display_price >= 1.0 { format!("{:>10.4}", display_price) }
                else if display_price > 0.0  { format!("{:>10.6}", display_price) }
                else { format!("{:>10}", "-") };
            let strat = if cand.best_strategy.is_empty() { "—" } else { &cand.best_strategy };
            out.push_str(&format!("  {:<10} {:<8} {:<6}  {}{} {:>7.1}%  {:>5.2}  {:>5.1}  {:>4}  {:>6.2}  {:>8.4}  {:<14}  [{}mum]\n",
                cand.symbol, cand.market, cand.interval,
                price_str, live_tag, cand.win_rate, cand.profit_factor, cand.max_drawdown_pct,
                cand.total_trades, cand.sharpe_ratio, cand.score, strat, cand.candle_count));
        }
        if zero_score.len() > show_n {
            out.push_str(&format!("  ... ({} sembol daha atlandı)\n", zero_score.len() - show_n));
        }
    }
    out.push('\n');

    // ── SCREENER ADAYLARI (Binance 24hr Ticker) ───────────────────────────────
    if !st.screener_candidates.is_empty() {
        out.push_str(&format!("── SCREENER ADAYLARI ({} sembol) {}\n",
            st.screener_candidates.len(), "─".repeat(29)));
        out.push_str(&format!("  {:<12}  {:>14}  {:>9}  {:>10}  {:<10}  {}\n",
            "Sembol", "Hacim24h(USDT)", "Değişim%", "SonFiyat", "Durum", "Bulunma"));
        out.push_str(&format!("  {}\n", "─".repeat(75)));
        for sc in &st.screener_candidates {
            let status_str = format!("{:?}", sc.status);
            out.push_str(&format!("  {:<12}  {:>14.0}  {:>+9.2}%  {:>10.4}  {:<10}  {}\n",
                sc.symbol, sc.quote_volume_24h, sc.price_change_pct,
                sc.last_price, status_str, sc.found_at));
        }
        out.push('\n');
    }

    // ── SEMBOL GEÇİŞ ANALİZİ ─────────────────────────────────────────────────
    // Aktif sembolün mevcut geçiş engelini ve top-5 adayın uygunluğunu gösterir.
    if st.auto_symbol {
        out.push_str(&format!("── SEMBOL GEÇİŞ ANALİZİ {}\n", "─".repeat(36)));
        let cur = &st.active_symbol;
        let elapsed_secs = st.loop_active_since.elapsed().as_secs();

        // Aktif sembol için min_stay hesapla (scanner ile aynı mantık)
        let intv_lower = cur.interval.trim().to_lowercase();
        let isecs: u64 = if let Some(n) = intv_lower.strip_suffix('m') {
            n.parse::<u64>().unwrap_or(1) * 60
        } else if let Some(n) = intv_lower.strip_suffix('h') {
            n.parse::<u64>().unwrap_or(1) * 3600
        } else if let Some(n) = intv_lower.strip_suffix('d') {
            n.parse::<u64>().unwrap_or(1) * 86400
        } else { 60 };
        let min_stay_base: u64 = isecs.clamp(60, 3_600);
        let min_stay: u64 = if cur.score < 0.15 {
            (min_stay_base as f64 * 0.30) as u64
        } else { min_stay_base };
        let time_ok = elapsed_secs >= min_stay || cur.score < 0.001;
        let remaining = if time_ok { 0u64 } else { min_stay - elapsed_secs };

        let breakeven_wr = if st.best_sl > 0.0 {
            st.best_sl / (st.best_sl + st.best_tp) * 100.0
        } else { 33.3 };

        out.push_str(&format!(
            "  Aktif    : {}/{}/{}   skor={:.4}   loop={}sn\n",
            cur.symbol, cur.market, cur.interval, cur.score, elapsed_secs
        ));
        out.push_str(&format!(
            "  Min bekle: {}sn ({}int, {}score)  {}  kalan={}sn\n",
            min_stay, &cur.interval,
            if cur.score < 0.15 { "düşük→%30" } else { "normal" },
            if time_ok { "✓ Süre tamam" } else { "⏳ Beklemede" },
            remaining
        ));
        out.push_str(&format!(
            "  Breakeven WR eşiği: {:.1}%  (SL={:.1}% TP={:.1}%)\n",
            breakeven_wr, st.best_sl, st.best_tp
        ));
        out.push_str(&format!("  {}\n", "─".repeat(70)));

        let top_candidates: Vec<_> = st.symbol_candidates.iter()
            .filter(|c| {
                let is_active = c.symbol == cur.symbol
                    && c.market == cur.market
                    && c.interval == cur.interval;
                !is_active && c.score > 0.0
            })
            .take(5)
            .collect();

        if top_candidates.is_empty() {
            out.push_str("  (Aktif dışında uygun aday yok — tüm adaylar score=0 veya aktif sembol)\n");
        } else {
            for (i, cand) in top_candidates.iter().enumerate() {
                let score_gap_pct = if cur.score > 0.001 {
                    (cand.score / cur.score - 1.0) * 100.0
                } else { 999.0 };
                let score_ok = cur.score < 0.001
                    || (cur.score <= 0.0 && cand.score > 0.01)
                    || cand.score >= cur.score * 1.08;
                // WinRate ve profit_factor kontrolü (sync_orchestrator ile aynı)
                let wr_ok  = cand.win_rate >= breakeven_wr;
                let pf_ok  = cand.profit_factor >= 1.0 || cand.profit_factor == 0.0;
                let dd_ok  = cand.max_drawdown_pct < 40.0;
                let cnt_ok = cand.candle_count >= 50;

                let identity_changed = cand.symbol   != cur.symbol
                    || cand.interval != cur.interval
                    || cand.market   != cur.market;

                let a_str = if !identity_changed { "= (aynı sembol)" }
                    else if score_ok { "✓" } else { "✗" };
                let b_str = if time_ok { "✓" } else { &format!("✗ ({}sn kaldı)", remaining) };
                let filters = if !wr_ok { format!("WR={:.1}%<{:.1}% ", cand.win_rate, breakeven_wr) }
                    else if !pf_ok { format!("PF={:.2}<1.0 ", cand.profit_factor) }
                    else if !dd_ok { format!("DD={:.1}%>40% ", cand.max_drawdown_pct) }
                    else if !cnt_ok { format!("cnt={}<50 ", cand.candle_count) }
                    else { String::new() };

                let verdict = if !identity_changed { "— (aynı sembol, skor güncellenir)" }
                    else if !filters.is_empty() { "✗ FİLTRE" }
                    else if !score_ok { "✗ SKOR FARKI YETERSİZ" }
                    else if !time_ok  { "✗ SÜRE BEKLENİYOR" }
                    else              { "✓ GEÇİŞ KRİTERLERİ SAĞLANIYOR" };

                out.push_str(&format!(
                    "  [{}/5] {}/{}/{}  skor={:.4}  fark={:+.1}%\n",
                    i + 1, cand.symbol, cand.market, cand.interval,
                    cand.score, score_gap_pct
                ));
                out.push_str(&format!(
                    "         KuralA(≥8%)={} KuralB(süre)={}  {}{}\n",
                    a_str, b_str, filters, verdict
                ));
            }
        }
        out.push('\n');
    }

    // ── AÇIK POZİSYONLAR ──────────────────────────────────────
    out.push_str(&format!("── POZİSYONLAR {}\n", "─".repeat(46)));
    if let Ok(positions) = st.live_positions.read() {
        if positions.is_empty() {
            out.push_str("  (Açık pozisyon yok)\n");
        } else {
            let mut total_pnl = 0.0_f64;
            out.push_str(&format!("  {:<10} {:<5} {:>5} {:>8} {:>10} {:>12} {:>10} {:>10} {:>10} {:>10} {:>8} {:>7} {:>12}\n",
                "Sembol", "Yön", "Lev", "Qty", "Giriş", "Güncel", "SL", "TSL", "TP", "Liq", "PnL%", "Açık-RR", "Süre"));
            out.push_str(&format!("  {}\n", "─".repeat(128)));
            for (sym, pos) in positions.iter() {
                // composite key "BTCUSDT-Futures" → "BTCUSDT" çıkar (export_prices sembol adına göre)
                let bare_sym = bare_symbol(sym);
                let arc_price = export_prices.get(bare_sym).copied().filter(|&v| v > 0.0);
                let cur = arc_price.unwrap_or(pos.current_price);
                // Fiyat kaynağını belirt: live=WS/orchestrator arc, db=current_price (MTF/DB güncel)
                let src = if arc_price.is_some() { "live" } else { "db" };
                // Fiyat tutarsızlık uyarısı: arc ile stale arasında >2% fark varsa işaretle
                let price_warn = if arc_price.is_some() && pos.current_price > 0.0 {
                    let diff_pct = (arc_price.unwrap() - pos.current_price).abs() / pos.current_price * 100.0;
                    if diff_pct > 2.0 { format!(" ⚠UYARSIZ({:.1}%)", diff_pct) } else { String::new() }
                } else { String::new() };
                // Marjin bazlı PnL% = fiyat farkı × leverage / margin
                let gross_pnl = pos_pnl(cur, pos.entry_price, pos.qty, pos.is_long);
                let lev = pos.leverage.max(1.0);
                let margin = pos.entry_price * pos.qty / lev;
                let pnl_pct = if margin > 0.0 { gross_pnl / margin * 100.0 } else { 0.0 };
                let pnl_usdt = gross_pnl;
                total_pnl += pnl_usdt;
                let yon = if pos.is_long { "LONG" } else { "SHORT" };
                let tsl = pos.trailing_sl.map(|t| format!("{:.4}", t)).unwrap_or_else(|| "-".to_string());
                let liq_str = if lev > 1.0 { format!("{:.4}", pos.liquidation_price) } else { "-".to_string() };
                // B1/B2/B3 durum etiketleri
                let mut flags = String::new();
                if pos.breakeven_triggered  { flags.push_str(" BE");  }
                if pos.partial_tp_triggered { flags.push_str(" P-TP"); }
                if pos.atr_trail_active     { flags.push_str(" ATR-T"); }
                // Gerçekleşmemiş R/R: mevcut kâr / başlangıç risk mesafesi
                let risk_dist = (pos.entry_price - pos.static_sl).abs();
                let unrealized_rr = if risk_dist > 0.0 {
                    let raw_move = if pos.is_long { cur - pos.entry_price } else { pos.entry_price - cur };
                    format!("{:+.2}R", raw_move / risk_dist)
                } else { "—".to_string() };
                // Pozisyon süresi
                let duration = format_duration_since(&pos.opened_at);
                out.push_str(&format!("  {:<10} {:<5} {:>4.1}x {:>8.4} {:>10.4} {:>12.4} {:>10.4} {:>10} {:>10.4} {:>10} {:>+7.2}% {:>7} {:>12}  [{}{}]{}\n",
                    sym, yon, lev, pos.qty, pos.entry_price, cur,
                    pos.static_sl, tsl, pos.static_tp, liq_str,
                    pnl_pct, unrealized_rr, duration, src, price_warn, flags));
                // Fiyat kaynakları karşılaştırması (sorun tespiti için)
                if let Some(live) = arc_price {
                    out.push_str(&format!("    live={:.4}  db={:.4}  entry={:.4}  ({:+.3}$)\n",
                        live, pos.current_price, pos.entry_price, pnl_usdt));
                } else {
                    out.push_str(&format!("    db={:.4}  entry={:.4}  ({:+.3}$)  (WS yok — orphan WS başlatılıyor)\n",
                        pos.current_price, pos.entry_price, pnl_usdt));
                }
            }
            out.push_str(&format!("  {}\n", "─".repeat(100)));
            out.push_str(&format!("  Toplam Açık PnL: {:+.3} USDT\n", total_pnl));
        }
    } else {
        out.push_str("  (Pozisyon verisi alınamadı)\n");
    }
    out.push('\n');

    // ── PnL SNAPSHOT GEÇMİŞİ ──────────────────────────────────
    out.push_str(&format!("── PnL GEÇMİŞİ (son {}dk) {}\n", st.pnl_snapshots.len() / 2, "─".repeat(35)));
    if st.pnl_snapshots.is_empty() {
        out.push_str("  (henüz snapshot yok — 30s sonra başlar)\n");
    } else {
        out.push_str(&format!("  {:<10} {:>10}  {}\n", "Saat", "Açık PnL", "Pozisyon Detayı"));
        out.push_str(&format!("  {}\n", "─".repeat(80)));
        // Son 30 snapshot göster (yaklaşık 15 dakika)
        for (ts, pnl, detail) in st.pnl_snapshots.iter().rev().take(30).collect::<Vec<_>>().iter().rev() {
            let pnl_sign = if *pnl >= 0.0 { "+" } else { "" };
            out.push_str(&format!("  {:<10} {:>+9.2}$  {}\n", ts, pnl, detail));
            let _ = pnl_sign;
        }
    }
    out.push('\n');

    // ── RISK / PER-SEMBOL SL/TP ───────────────────────────────
    out.push_str(&format!("── RİSK {}\n", "─".repeat(53)));
    if let Ok(lr) = st.live_risk.read() {
        let max_sl_at_base = 80.0 / lr.base_leverage.max(1.0);
        let max_sl_at_max  = 80.0 / lr.max_leverage.max(1.0);
        out.push_str(&format!(
            "⚡ Kaldıraç  : {:.1}x–{:.1}x (aralık)  anlık={:.1}x  |  SL klamp: {:.1}%–{:.1}%\n",
            lr.base_leverage, lr.max_leverage, lr.effective_leverage,
            max_sl_at_max, max_sl_at_base));
        out.push_str(&format!("Global SL/TP: {:.1}% / {:.1}%  |  MA: fast={} slow={}\n",
            lr.global_sl, lr.global_tp, lr.global_fast, lr.global_slow));
        out.push_str(&format!("RSI aktif   : period={} OB={:.0}% OS={:.0}%\n",
            lr.global_rsi_period, lr.global_rsi_ob, lr.global_rsi_os));
        out.push_str(&format!("BB  aktif   : period={} σ={:.1}  |  MACD aktif: {}/{}/{}\n",
            lr.global_bb_period, lr.global_bb_std_dev,
            lr.global_macd_fast, lr.global_macd_slow, lr.global_macd_signal));
        if !lr.per_symbol.is_empty() {
            out.push_str(&format!("  {:<12} {:>8} {:>8}\n", "Sembol", "SL%", "TP%"));
            out.push_str(&format!("  {}\n", "─".repeat(30)));
            let mut sym_list: Vec<_> = lr.per_symbol.iter().collect();
            sym_list.sort_by_key(|(k, _)| k.as_str());
            for (sym, (sl, tp)) in sym_list {
                out.push_str(&format!("  {:<12} {:>7.1}% {:>7.1}%\n", sym, sl, tp));
            }
        }
    }
    out.push('\n');

    // ── OTURUM İSTATİSTİKLERİ & COOLDOWN ─────────────────────
    out.push_str(&format!("── OTURUM & COOLDOWN {}\n", "─".repeat(40)));
    if let Ok(lr) = st.live_risk.read() {
        let session_losses = lr.session_closed.saturating_sub(lr.session_wins);
        let win_rate = if lr.session_closed > 0 {
            lr.session_wins as f64 / lr.session_closed as f64 * 100.0
        } else { 0.0 };
        out.push_str(&format!(
            "İşlem       : {}/{} kapalı  |  Kazanma: {} ({:.1}%)  |  Loss Streak: {}\n",
            lr.session_wins, lr.session_closed, lr.session_wins, win_rate, lr.loss_streak
        ));
        out.push_str(&format!(
            "RR Oranı    : {:.2}  (ort. kazanç / ort. kayıp)  |  Zarar: {} işlem\n",
            lr.session_rr, session_losses
        ));
        if lr.sl_cooldowns.is_empty() {
            out.push_str("SL Cooldown : (aktif cooldown yok)\n");
        } else {
            out.push_str("SL Cooldown : Aşağıdaki semboller SL sonrası bekleme modunda:\n");
            for (sym, secs) in &lr.sl_cooldowns {
                out.push_str(&format!("  ⏸ {}  —  {} dk {} sn kaldı\n",
                    sym, secs / 60, secs % 60));
            }
        }
    }
    out.push('\n');

    // ── EVRİM ─────────────────────────────────────────────────
    out.push_str(&format!("── EVRİM {}\n", "─".repeat(52)));
    if let Ok(evo) = st.live_evolution.read() {
        out.push_str(&format!("Durum       : {} | Brain: {} | Pop: {}\n",
            if evo.evolution_enabled { "Aktif" } else { "Pasif" },
            if evo.brain_active { "Yüklendi" } else { "Yok" },
            if evo.pop_active   { "Yüklendi" } else { "Yok" }));
        out.push_str(&format!("Genom       : {} | fitness={:.4} | trades={} | win={:.1}%\n",
            evo.genome_id, evo.genome_fitness,
            evo.genome_trades, evo.genome_win_rate));
        out.push_str(&format!("Her N Cycle : {} | Şimdiki Cycle: {}\n",
            evo.evolve_every_n_cycles, evo.cycle_id));
        out.push_str(&format!("Brain Özet  : {}\n", evo.brain_summary));
        out.push_str(&format!("Pop Özet    : {}\n", evo.pop_summary));
    }
    out.push('\n');

    // ── DENETİM / SİNYAL SAYAÇLARI ───────────────────────────
    out.push_str(&format!("── DENETİM {}\n", "─".repeat(50)));
    if let Ok(sc) = st.live_signal_counts.read() {
        let total_signals = sc.buy + sc.sell + sc.hold;
        let total_blocked = sc.blocked_rr + sc.blocked_volatility + sc.blocked_trend + sc.blocked_risk_gate;
        out.push_str(&format!(
            "Sinyaller   : BUY={} SELL={} HOLD={} | Toplam={}\n",
            sc.buy, sc.sell, sc.hold, total_signals
        ));
        out.push_str(&format!(
            "Bloklar     : R/R={} Volatilite={} Trend={} RiskGate={} | Toplam={}\n",
            sc.blocked_rr, sc.blocked_volatility, sc.blocked_trend, sc.blocked_risk_gate, total_blocked
        ));
        out.push_str(&format!(
            "ML Eşik Altı: {} kez (conf < min_conf)\n",
            sc.ml_below_threshold
        ));
        if !sc.last_params.is_empty() {
            out.push_str(&format!("Son Params  : {}\n", sc.last_params));
        }
        if !sc.last_block_reason.is_empty() {
            out.push_str(&format!("Son Blok    : {}\n", sc.last_block_reason));
        }
        // Trade kaçırma analizi
        if total_signals > 0 {
            let hold_pct = sc.hold as f64 / total_signals as f64 * 100.0;
            let block_pct = if (sc.buy + sc.sell + total_blocked) > 0 {
                total_blocked as f64 / (sc.buy + sc.sell + total_blocked) as f64 * 100.0
            } else { 0.0 };
            out.push_str(&format!(
                "Analiz      : HOLD oranı={:.1}% | Blok oranı={:.1}% (BUY+SELL sonrası)\n",
                hold_pct, block_pct
            ));
        }
    }
    out.push('\n');

    // ── GÜNLÜK ────────────────────────────────────────────────
    out.push_str(&format!("── GÜNLÜK (son {} kayıt) {}\n", st.log.len(), "─".repeat(36)));
    for line in st.log.iter().rev() {
        out.push_str(&strip_emoji(line));
        out.push('\n');
    }

    // ── KAPALI İŞLEM ÖZETİ ───────────────────────────────────
    out.push_str(&format!("── KAPALI İŞLEM ÖZETİ {}\n", "─".repeat(39)));
    if let Ok(closed) = st.live_closed_trades.read() {
        if closed.is_empty() {
            out.push_str("  (kapanmış işlem yok)\n");
        } else {
            let total = closed.len();
            let wins:  Vec<_> = closed.iter().filter(|t| t.pnl > 0.0).collect();
            let losses:Vec<_> = closed.iter().filter(|t| t.pnl <= 0.0).collect();
            let total_pnl: f64 = closed.iter().map(|t| t.pnl).sum();
            let avg_pnl = total_pnl / total as f64;
            let avg_win  = if wins.is_empty()   { 0.0 } else { wins.iter().map(|t| t.pnl).sum::<f64>() / wins.len() as f64 };
            let avg_loss = if losses.is_empty()  { 0.0 } else { losses.iter().map(|t| t.pnl).sum::<f64>() / losses.len() as f64 };
            let max_win  = wins.iter().map(|t| t.pnl).fold(f64::NEG_INFINITY, f64::max);
            let max_loss = losses.iter().map(|t| t.pnl).fold(f64::INFINITY, f64::min);
            let win_pct  = wins.len() as f64 / total as f64 * 100.0;
            // Çıkış sebebi dağılımı
            // Önce TSL'yi kesin eşleştir, sonra SL'yi TSL olmayan olarak say
            let tsl_count = closed.iter().filter(|t| t.exit_reason == "trailing_sl"
                || t.exit_reason.contains("trailing")).count();
            let sl_count  = closed.iter().filter(|t| {
                let r = &t.exit_reason;
                (r.contains("sl") || r.contains("SL") || r == "static_sl")
                    && !r.contains("trailing")
            }).count();
            let tp_count  = closed.iter().filter(|t| {
                let r = &t.exit_reason;
                r == "take_profit" || r.contains("TP") || r.contains("tp")
            }).count();
            let sig_count = total.saturating_sub(sl_count + tp_count + tsl_count);
            out.push_str(&format!(
                "  Toplam={} | Kâr={} ({:.1}%) Zarar={} | Net PnL={:+.3}$ Ort={:+.3}$/işlem\n",
                total, wins.len(), win_pct, losses.len(), total_pnl, avg_pnl));
            out.push_str(&format!(
                "  Ort Kazanç={:+.3}$ | Ort Kayıp={:+.3}$ | Max Kazanç={:+.3}$ | Max Kayıp={:+.3}$\n",
                avg_win, avg_loss,
                if max_win  == f64::NEG_INFINITY { 0.0 } else { max_win },
                if max_loss == f64::INFINITY     { 0.0 } else { max_loss }));
            out.push_str(&format!(
                "  Çıkış: SL={} TP={} TSL={} Sinyal={}\n",
                sl_count, tp_count, tsl_count, sig_count));
            // Gerçekleşen R/R ortalaması (SL fiyatı bilinenlerde)
            let rr_vals: Vec<f64> = closed.iter().filter_map(|t| {
                let risk_dist = (t.entry_price - t.sl_price).abs();
                if risk_dist > 0.0 && t.sl_price > 0.0 {
                    let raw_move = if t.is_long { t.exit_price - t.entry_price } else { t.entry_price - t.exit_price };
                    Some(raw_move / risk_dist)
                } else { None }
            }).collect();
            if !rr_vals.is_empty() {
                let avg_rr = rr_vals.iter().sum::<f64>() / rr_vals.len() as f64;
                let pos_rr: Vec<_> = rr_vals.iter().filter(|&&v| v > 0.0).collect();
                let neg_rr: Vec<_> = rr_vals.iter().filter(|&&v| v <= 0.0).collect();
                let avg_pos_rr = if pos_rr.is_empty() { 0.0 } else { pos_rr.iter().copied().sum::<f64>() / pos_rr.len() as f64 };
                let avg_neg_rr = if neg_rr.is_empty() { 0.0 } else { neg_rr.iter().copied().sum::<f64>() / neg_rr.len() as f64 };
                out.push_str(&format!(
                    "  R/R       : Ort={:+.2}R | Kâr Ort={:+.2}R | Zarar Ort={:+.2}R ({}/{} trade R/R hesaplandı)\n",
                    avg_rr, avg_pos_rr, avg_neg_rr, rr_vals.len(), total));
            }
            // İşlem süresi istatistikleri (opened_at bilinenlerde)
            let dur_secs: Vec<u64> = closed.iter().filter_map(|t| {
                let odt = parse_ts(&t.opened_at)?;
                let cdt = parse_ts(&t.closed_at)?;
                Some((cdt - odt).num_seconds().abs() as u64)
            }).collect();
            if !dur_secs.is_empty() {
                let avg_dur = dur_secs.iter().sum::<u64>() / dur_secs.len() as u64;
                let min_dur = *dur_secs.iter().min().unwrap_or(&0);
                let max_dur = *dur_secs.iter().max().unwrap_or(&0);
                let fmt_dur = |s: u64| -> String {
                    let h = s / 3600; let m = (s % 3600) / 60; let sec = s % 60;
                    if h > 0 { format!("{}sa{}dk", h, m) }
                    else if m > 0 { format!("{}dk{}sn", m, sec) }
                    else { format!("{}sn", sec) }
                };
                out.push_str(&format!(
                    "  Süre      : Ort={} | Min={} | Max={} ({}/{} trade)\n",
                    fmt_dur(avg_dur), fmt_dur(min_dur), fmt_dur(max_dur), dur_secs.len(), total));
            }
            // Detay tablosu — en son 20 işlem
            out.push('\n');
            out.push_str(&format!("── KAPALI İŞLEM DETAY (son {}) {}\n",
                total.min(20), "─".repeat(30)));
            out.push_str(&format!("  {:<10} {:<5} {:>10} {:>10} {:>8} {:>8} {:>6}  {:<12}  {:<8}  {}\n",
                "Sembol","Yön","Giriş","Çıkış","PnL$","PnL%","R/R","Kapanış","Süre","Sebep"));
            out.push_str(&format!("  {}\n", "─".repeat(105)));
            for t in closed.iter().rev().take(20) {
                let yon = if t.is_long { "LONG" } else { "SHORT" };
                let risk_dist = (t.entry_price - t.sl_price).abs();
                let rr_str = if risk_dist > 0.0 && t.sl_price > 0.0 {
                    let raw_move = if t.is_long { t.exit_price - t.entry_price } else { t.entry_price - t.exit_price };
                    format!("{:+.2}R", raw_move / risk_dist)
                } else { "—".to_string() };
                let dur = format_duration_between(&t.opened_at, &t.closed_at);
                out.push_str(&format!("  {:<10} {:<5} {:>10.4} {:>10.4} {:>+8.3} {:>+7.2}% {:>6}  {:<12}  {:<8}  {}\n",
                    t.symbol, yon, t.entry_price, t.exit_price,
                    t.pnl, t.pnl_pct, rr_str, t.closed_at, dur, t.exit_reason));
            }
        }
    }
    out.push('\n');

    // ── SCALP / SWING ÖZET ────────────────────────────────────
    out.push_str(&format!("── SCALP / SWING {}\n", "─".repeat(44)));
    if let Ok(closed) = st.live_closed_trades.read() {
        use memos_trading_core::robot::scalp_swing::TradeType;
        let scp: Vec<_> = closed.iter().filter(|t| t.trade_type == TradeType::Scalp).collect();
        let swg: Vec<_> = closed.iter().filter(|t| t.trade_type == TradeType::Swing).collect();
        // Açık pozisyonlar
        let open_scp = st.live_positions.read().ok()
            .map(|m| m.values().filter(|p| p.trade_type == TradeType::Scalp).count()).unwrap_or(0);
        let open_swg = st.live_positions.read().ok()
            .map(|m| m.values().filter(|p| p.trade_type == TradeType::Swing).count()).unwrap_or(0);

        if scp.is_empty() && swg.is_empty() && open_scp == 0 && open_swg == 0 {
            out.push_str("  (Scalp/Swing işlem yok)\n");
        } else {
            let fmt_group = |label: &str, trades: &[&memos_trading_core::robot::robotic_loop::ClosedTradeData], open: usize, out: &mut String| {
                let total = trades.len();
                if total == 0 && open == 0 { return; }
                let wins: Vec<_> = trades.iter().filter(|t| t.pnl > 0.0).collect();
                let net_pnl: f64 = trades.iter().map(|t| t.pnl).sum();
                let win_pct = if total > 0 { wins.len() as f64 / total as f64 * 100.0 } else { 0.0 };
                let sl_c = trades.iter().filter(|t| t.exit_reason.to_lowercase().contains("sl")).count();
                let tp_c = trades.iter().filter(|t| t.exit_reason.to_lowercase().contains("tp")).count();
                // Ortalama süre
                let dur_secs: Vec<u64> = trades.iter().filter_map(|t| {
                    let o = parse_ts(&t.opened_at)?;
                    let c = parse_ts(&t.closed_at)?;
                    Some((c - o).num_seconds().abs() as u64)
                }).collect();
                let avg_dur_str = if dur_secs.is_empty() { "—".to_string() } else {
                    let avg = dur_secs.iter().sum::<u64>() / dur_secs.len() as u64;
                    if avg < 60 { format!("{}sn", avg) }
                    else if avg < 3600 { format!("{}dk", avg / 60) }
                    else { format!("{}sa{}dk", avg / 3600, (avg % 3600) / 60) }
                };
                out.push_str(&format!(
                    "  [{label}] Açık={open} | Kapalı={total} | WinRate={:.1}% | Net={:+.3}$ | SL={sl_c} TP={tp_c} | OrtSüre={avg_dur_str}\n",
                    win_pct, net_pnl,
                ));
                // Son 10 kapalı işlem
                if !trades.is_empty() {
                    out.push_str(&format!("    {:<10} {:<5} {:>10} {:>10} {:>+8} {:>+7}%  {:<12}  {:<7}  {}\n",
                        "Sembol","Yön","Giriş","Çıkış","PnL$","PnL%","Kapanış","Süre","Sebep"));
                    out.push_str(&format!("    {}\n", "─".repeat(90)));
                    for t in trades.iter().rev().take(10) {
                        let yon = if t.is_long { "LONG" } else { "SHORT" };
                        let dur = format_duration_between(&t.opened_at, &t.closed_at);
                        out.push_str(&format!("    {:<10} {:<5} {:>10.4} {:>10.4} {:>+8.3} {:>+7.2}%  {:<12}  {:<7}  {}\n",
                            t.symbol, yon, t.entry_price, t.exit_price,
                            t.pnl, t.pnl_pct, t.closed_at, dur, t.exit_reason));
                    }
                }
            };
            fmt_group("SCP", &scp, open_scp, &mut out);
            fmt_group("SWG", &swg, open_swg, &mut out);
        }
    } else {
        out.push_str("  (Veri alınamadı)\n");
    }
    out.push('\n');

    // ── KONFİGÜRASYON ÖZETİ ──────────────────────────────────
    out.push_str(&format!("── KONFİGÜRASYON {}\n", "─".repeat(43)));
    {
        let cfg = &st.config;
        let mod_str = if st.paper_mode { "Kağıt (Paper)" } else { "CANLI" };
        out.push_str(&format!("Borsa/Market : {}/{} | Mod: {}\n", cfg.exchange, cfg.market, mod_str));
        out.push_str(&format!("Sembol/İnt.  : {} | {}\n", cfg.symbol, cfg.interval));
        out.push_str(&format!("Sermaye      : ${:.2} | İşlem Miktarı: {:.4}\n", cfg.capital, cfg.trade_amount));
        out.push_str(&format!("Kaldıraç     : {:.0}x – {:.0}x\n", cfg.leverage_base, cfg.leverage_max));
        out.push_str(&format!("Backtest     : {} | Her {} dk | {} mum\n",
            if cfg.backtest_enabled { "Aktif" } else { "Devre Dışı" },
            cfg.backtest_every_mins, cfg.backtest_candle_limit));
        out.push_str(&format!("İndirme      : {} | Her {} dk | {} mum | Top-N={}\n",
            if cfg.download_enabled { "Aktif" } else { "Devre Dışı" },
            cfg.download_every_mins, cfg.download_candle_limit, cfg.download_top_n));
        out.push_str(&format!("Auto-Export  : {} | Maks. saklanan: {}\n",
            if cfg.auto_export_every_mins > 0 { format!("Her {} dk", cfg.auto_export_every_mins) } else { "Devre Dışı".into() },
            cfg.auto_export_keep));
        out.push_str(&format!("API Anahtarı : {}\n",
            if st.api_key_set { "Yüklendi" } else { "EKSİK – sadece paper mod" }));
    }
    out.push('\n');

    // ── MATEMATİK DENETİM ────────────────────────────────────
    // Her kapanmış işlem için bağımsız math doğrulaması.
    // Kayıtlı pnl/pnl_pct ile entry/exit/qty'den yeniden hesaplanan değerleri karşılaştırır.
    out.push_str(&format!("── MATEMATİK DENETİM {}\n", "─".repeat(40)));
    if let Ok(closed) = st.live_closed_trades.read() {
        if closed.is_empty() {
            out.push_str("  (kapanmış işlem yok)\n");
        } else {
            let mut math_errors = 0usize;
            let mut price_leaks = 0usize;
            let mut direction_errors = 0usize;
            out.push_str(&format!(
                "  {:<10} {:<5} {:>10} {:>10} {:>10} {:>8} {:>8}  {}\n",
                "Sembol","Yön","Giriş","Çıkış","PnL(kay)","PnL%(kay)","PnL%(hes)","Durum"
            ));
            out.push_str(&format!("  {}\n", "─".repeat(95)));
            for t in closed.iter() {
                let yon = if t.is_long { "LONG" } else { "SHORT" };

                // 1) PnL math: (exit-entry)*qty veya (entry-exit)*qty
                //    t.exit_price = adjusted (slippage dahil), t.entry_price = adjusted.
                //    t.pnl = gross - commission; gross = (exit-entry)*qty.
                //    Tolerans: komisyon (typically 0.2% × notional) + küçük yuvarlama.
                let computed_pnl = if t.is_long {
                    (t.exit_price - t.entry_price) * t.qty
                } else {
                    (t.entry_price - t.exit_price) * t.qty
                };
                let notional = t.entry_price * t.qty;
                let pnl_tol = (notional * 0.003).max(0.10); // komisyon %0.2 × 2 yüz + buffer
                let pnl_math_ok = (computed_pnl - t.pnl).abs() < pnl_tol;

                // 2) Yüzde doğrulaması: margin bazlı (pnl × leverage / notional × 100)
                //    pnl_pct = pnl / margin × 100,  margin = notional / leverage
                //    → computed_pct = pnl × leverage / notional × 100
                let lev = t.leverage.max(1.0);
                let computed_pct = if t.entry_price > 0.0 && t.qty > 0.0 {
                    computed_pnl * lev / notional * 100.0
                } else { 0.0 };
                // Tolerans kaldıraçla büyür (%.0.5 × leverage)
                let pct_ok = (computed_pct - t.pnl_pct).abs() < (0.5 * lev).max(0.5);

                // 3) Yön tutarlılığı: take_profit + LONG → exit > entry
                //    trailing_sl: fiyat önce yükselip sonra geri dönebilir → kârlı çıkış normaldir.
                //    Sadece static_sl için yön kontrolü yapılır; trailing_sl atlanır.
                //    NOT: entry==exit (tick çözünürlüğü) durumunda yön hatası değil — komisyon kaybı.
                let price_eps = t.entry_price * 1e-5; // 0.001% tolerans
                let direction_ok = match t.exit_reason.as_str() {
                    r if r.contains("take_profit") => {
                        if t.is_long { t.exit_price > t.entry_price - price_eps }
                        else         { t.exit_price < t.entry_price + price_eps }
                    }
                    r if (r.contains("sl") || r.contains("SL")) && !r.contains("trailing") => {
                        // entry==exit kabul edilir (komisyon kaybı)
                        if t.is_long { t.exit_price <= t.entry_price + price_eps }
                        else         { t.exit_price >= t.entry_price - price_eps }
                    }
                    _ => true,
                };

                // 4) Fiyat sızıntısı: TP/SL çıkışında yüzde beklenmedik şekilde büyük.
                //    Kaldıraçlı sistemde pnl_pct marjin bazlıdır, bu yüzden eşik leverage ile ölçeklenir.
                //    Normal TP 10x → %10-30% normaldir; >100% ise anomali.
                let lev_thresh = (lev * 15.0).max(15.0); // 1x → 15%, 10x → 150%
                let tp_slip = t.exit_reason.contains("take_profit") && t.pnl_pct.abs() > lev_thresh;
                let sl_slip = (t.exit_reason.contains("static_sl") || t.exit_reason.contains("SL"))
                    && t.pnl_pct.abs() > lev_thresh
                    && !t.exit_reason.contains("trailing");
                let price_leak = tp_slip || sl_slip;

                // Durum etiketi
                let mut flags: Vec<&str> = Vec::new();
                if !pnl_math_ok    { flags.push("MATH_HATASI"); math_errors += 1; }
                if !pct_ok         { flags.push("PCT_HATASI"); }
                if !direction_ok   { flags.push("YON_HATASI"); direction_errors += 1; }
                if price_leak      { flags.push("FIYAT_SIZINTISI"); price_leaks += 1; }
                let durum = if flags.is_empty() { "OK".to_string() } else { format!("[!!] {}", flags.join(" ")) };

                out.push_str(&format!(
                    "  {:<10} {:<5} {:>10.4} {:>10.4} {:>10.3} {:>7.2}% {:>7.2}%  {}\n",
                    t.symbol, yon,
                    t.entry_price, t.exit_price,
                    t.pnl, t.pnl_pct, computed_pct,
                    durum
                ));
            }
            out.push_str(&format!("  {}\n", "─".repeat(95)));
            let total = closed.len();
            out.push_str(&format!(
                "  Toplam: {} işlem | Math hataları: {} | Fiyat sızıntısı: {} | Yön hataları: {}\n",
                total, math_errors, price_leaks, direction_errors
            ));
            if math_errors + price_leaks + direction_errors == 0 {
                out.push_str("  TUM ISLEMLER MATEMATIKSEL OLARAK TUTARLI\n");
            } else {
                out.push_str("  UYARI: Yukaridaki [!!] satirlarinda tutarsizlik tespit edildi!\n");
            }
        }
    }
    out.push('\n');

    // ── TİMELİNE ─────────────────────────────────────────────
    // Mevcut veri kaynaklarından (closed_trades + log + pnl_snapshots) derlenir.
    // Her olay: [saat] [kategori] mesaj  +  anomali bayrakları
    out.push_str(&format!("\n── TİMELİNE {}\n", "─".repeat(48)));
    out.push_str("  Kategori: TRD=İşlem  LOOP=Loop/Strateji  RISK=Risk  FIYAT=Fiyat  EQT=Sermaye\n");
    out.push_str(&format!("  {}\n", "─".repeat(88)));

    // --- Olayları topla: (saat_str, kategori, mesaj, anomali_flag)
    let mut events: Vec<(String, &'static str, String, bool)> = Vec::new();

    // 1) Kapanmış işlemler
    if let Ok(closed) = st.live_closed_trades.read() {
        let mut cumulative = 0.0f64;
        let mut win_count = 0usize;
        let mut total_count = 0usize;
        // Önce sırala (kapatılma saatine göre)
        let mut sorted = closed.clone();
        sorted.sort_by(|a, b| a.closed_at.cmp(&b.closed_at));
        for t in &sorted {
            total_count += 1;
            cumulative += t.pnl;
            let won = t.pnl > 0.0;
            if won { win_count += 1; }
            let yon = if t.is_long { "LONG" } else { "SHORT" };
            // Anomali: SL'den büyük kayıp veya TP'den büyük kazanç (fiyat atlaması)
            let expected_max_loss = t.entry_price * 0.025 * t.qty; // >2.5× SL = anomali
            let anomaly = !won && t.pnl.abs() > expected_max_loss;
            let win_str = if won { "KAR " } else { "KAYIP" };
            let lev_str = format!(" ×{:.1}", t.leverage);
            let msg = format!(
                "{} {}{} {} | giriş={:.4} çıkış={:.4} | pnl={:+.2}$ ({:+.2}%) | neden={} | kümülatif={:+.2}$ | win={}/{}",
                t.symbol, yon, lev_str, win_str,
                t.entry_price, t.exit_price,
                t.pnl, t.pnl_pct,
                t.exit_reason,
                cumulative,
                win_count, total_count,
            );
            events.push((t.closed_at.clone(), "TRD", msg, anomaly));
        }
    }

    // 2) Log'dan seçili olayları çıkar (loop, strateji, risk, fiyat anomali)
    let log_patterns: &[(&str, &'static str)] = &[
        ("loop", "LOOP"),
        ("restart", "LOOP"),
        ("yeniden başla", "LOOP"),      // "engine yeniden başlatılıyor"
        ("Sembol →", "LOOP"),           // "🔄 Sembol → X (ayarlar, restart)"
        ("Primary geçiş", "LOOP"),      // "🔄 Primary geçiş: X durduruldu"
        ("Strateji otomatik", "LOOP"),  // "🦅 Strateji otomatik değişti"
        ("Strateji geçişi", "LOOP"),    // "🔒 Strateji geçişi ertelendi"
        ("Strateji secilen", "LOOP"),   // eski format geriye dönük uyumluluk
        ("Rejim:", "LOOP"),
        ("durduruldu", "LOOP"),         // worker/sembol durduruldu
        ("veri çekme hatası", "LOOP"),  // fetcher REST hatası
        ("Monitor: PAUSE", "LOOP"),
        ("Monitor: STOP", "LOOP"),
        ("circuit", "RISK"),
        ("drawdown", "RISK"),
        ("Max drawdown", "RISK"),
        ("RiskGate", "RISK"),
        ("UYARSIZ", "FIYAT"),
        ("stale", "FIYAT"),
        ("anomali", "FIYAT"),
        ("Pozisyon tutarsizligi", "FIYAT"),
        ("orphan", "FIYAT"),
    ];
    for line in st.log.iter() {
        // Log formatı: [YYYY-MM-DD HH:MM:SS] ... veya eski [HH:MM:SS] ...
        let ts = if line.starts_with('[') {
            // Tam tarih-zaman (19 karakter): "2026-04-03 18:28:11"
            if line.len() > 20 && line.as_bytes().get(11) == Some(&b' ') {
                line.get(1..20).unwrap_or("").to_string()
            } else {
                // Eski format HH:MM:SS → bugünün tarihi ile genişlet
                let hms = line.get(1..9).unwrap_or("");
                if hms.len() == 8 {
                    format!("{} {}", chrono::Local::now().format("%Y-%m-%d"), hms)
                } else { continue }
            }
        } else { continue };
        if ts.len() < 10 { continue; }
        // [metrics] logları sadece istatistik; "drawdown" gibi RISK keyword içerebilir ama uyarı değil
        if line.contains("[metrics]") { continue; }
        let lower = line.to_lowercase();
        for (pat, cat) in log_patterns {
            if lower.contains(&pat.to_lowercase()) {
                let clean = strip_emoji(line);
                let anomaly = *cat == "RISK" || *cat == "FIYAT";
                events.push((ts.clone(), cat, clean, anomaly));
                break;
            }
        }
    }

    // 3) PnL snapshot'lardan büyük anlık değişimleri işaretle (>50$ sıçrama = anomali)
    {
        let snaps: Vec<_> = st.pnl_snapshots.iter().collect();
        for w in snaps.windows(2) {
            let (ts0, pnl0, _) = w[0];
            let (ts1, pnl1, detail) = w[1];
            let delta = pnl1 - pnl0;
            if delta.abs() > 50.0 {
                let msg = format!(
                    "PnL anlık degisim: {}{:.2}$ ({} -> {}) | {}",
                    if delta > 0.0 { "+" } else { "" }, delta, ts0, ts1,
                    detail.chars().take(80).collect::<String>()
                );
                events.push((ts1.clone(), "FIYAT", msg, true));
            }
        }
    }

    // Sırala (zaman damgasına göre)
    events.sort_by(|a, b| a.0.cmp(&b.0));

    if events.is_empty() {
        out.push_str("  (henüz kayıt yok)\n");
    } else {
        // Özet istatistikler
        let trd_events: Vec<_> = events.iter().filter(|e| e.1 == "TRD").collect();
        let anomaly_count = events.iter().filter(|e| e.3).count();
        let risk_count = events.iter().filter(|e| e.1 == "RISK").count();
        out.push_str(&format!(
            "  Toplam: {} olay | {} işlem | {} anomali | {} risk uyarısı\n",
            events.len(), trd_events.len(), anomaly_count, risk_count
        ));
        out.push_str(&format!("  {}\n", "─".repeat(88)));

        for (ts, cat, msg, anomaly) in &events {
            let flag = if *anomaly { " [!!]" } else { "" };
            out.push_str(&format!("  [{}] [{:<4}]{} {}\n", ts, cat, flag, msg));
        }
    }

    // ── HTF INTERVAL TÜREVLERİ ───────────────────────────────
    out.push_str(&format!("\n── HTF INTERVAL TÜREVLERİ {}\n", "─".repeat(34)));
    if st.htf_candle_counts.is_empty() {
        out.push_str("  (HTF verisi henüz yüklenmedi)\n");
    } else {
        out.push_str(&format!("  {:<12} {:<8} {:>8}  {}\n", "Sembol", "Interval", "Mum", "Son Zaman"));
        out.push_str(&format!("  {}\n", "─".repeat(50)));
        let mut syms: Vec<_> = st.htf_candle_counts.keys().collect();
        syms.sort();
        for sym in syms {
            let intervals = &st.htf_candle_counts[sym];
            let mut ivs: Vec<_> = intervals.iter().collect();
            ivs.sort_by_key(|(iv, _)| iv.as_str());
            for (iv, (count, last_ts)) in ivs {
                out.push_str(&format!("  {:<12} {:<8} {:>8}  {}\n",
                    sym, iv, count, last_ts));
            }
        }
    }
    out.push('\n');

    // ── S/R BÖLGELERİ ─────────────────────────────────────────
    out.push_str(&format!("── S/R BÖLGELERİ {}\n", "─".repeat(44)));
    if let Ok(zones_map) = st.live_sr_zones.read() {
        if zones_map.is_empty() {
            out.push_str("  (S/R bölge verisi henüz yüklenmedi)\n");
        } else {
            let mut syms: Vec<_> = zones_map.keys().collect();
            syms.sort();
            for sym in syms {
                let zones = &zones_map[sym];
                out.push_str(&format!("  {} — {} bölge\n", sym, zones.len()));
                out.push_str(&format!("    {:<8} {:>12} {:>12} {:>12} {:>8} {:>6}\n",
                    "Tip", "Alt", "Orta", "Üst", "Güç", "Touch"));
                let mut sorted = zones.clone();
                sorted.sort_by(|a, b| b.strength.partial_cmp(&a.strength).unwrap_or(std::cmp::Ordering::Equal));
                for z in sorted.iter().take(10) {
                    let tip = match z.zone_type {
                        memos_trading_core::robot::sr_detector::ZoneType::Support    => "Destek",
                        memos_trading_core::robot::sr_detector::ZoneType::Resistance => "Direnç",
                    };
                    out.push_str(&format!("    {:<8} {:>12.4} {:>12.4} {:>12.4} {:>8.3} {:>6}\n",
                        tip, z.price_low, z.midpoint, z.price_high, z.strength, z.touch_count));
                }
                if zones.len() > 10 {
                    out.push_str(&format!("    ... ({} bölge daha)\n", zones.len() - 10));
                }
            }
        }
    } else {
        out.push_str("  (S/R verisi alınamadı)\n");
    }
    out.push('\n');

    // ── STRATEJİ SIRALAMASI ────────────────────────────────────
    out.push_str(&format!("── STRATEJİ SIRALAMASI {}\n", "─".repeat(38)));
    let regime_now = st.live_regime_strategy.read().ok()
        .map(|s| s.clone()).unwrap_or_default();
    if !regime_now.is_empty() && regime_now != "—" {
        out.push_str(&format!("  Rejim Stratejisi: {}\n", regime_now));
    }
    if st.symbol_candidates.is_empty() {
        out.push_str("  (Henüz strateji puanlaması yok)\n");
    } else {
        // Aynı sembol × strateji için skorlara bakıyoruz — symbol_candidates skora göre sırala
        let mut sorted_cands = st.symbol_candidates.clone();
        sorted_cands.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        out.push_str(&format!("  {:<12} {:<8} {:>8}  {:>8}  {:>7}  {:>8}  {}\n",
            "Sembol", "Market", "Skor", "WinRate", "PnL$", "İşlem", "Interval"));
        out.push_str(&format!("  {}\n", "─".repeat(68)));
        for cand in sorted_cands.iter().take(20) {
            let aktif = if (cand.exchange.clone(), cand.market.clone(), cand.symbol.clone(), cand.interval.clone())
                == (st.active_symbol.exchange.clone(), st.active_symbol.market.clone(),
                    st.active_symbol.symbol.clone(), st.active_symbol.interval.clone())
            { " ← AKTİF" } else { "" };
            out.push_str(&format!("  {:<12} {:<8} {:>8.4}  {:>7.1}%  {:>+7.1}  {:>8}  {}{}\n",
                cand.symbol, cand.market, cand.score,
                cand.win_rate, cand.total_pnl, cand.total_trades,
                cand.interval, aktif));
        }
    }
    out.push('\n');

    // ── KÜMÜLATİF İŞLEM MALİYETİ ─────────────────────────────
    out.push_str(&format!("── KÜMÜLATİF İŞLEM MALİYETİ {}\n", "─".repeat(32)));
    if let Ok(costs) = st.live_execution_costs.read() {
        if costs.trade_count == 0 {
            out.push_str("  (Henüz işlem maliyeti kaydı yok)\n");
        } else {
            let total_notional_est = if costs.avg_cost_per_trade > 0.0 {
                // Oran tahmini: avg_cost ≈ %0.1 notional → notional ≈ avg_cost / 0.001
                costs.avg_cost_per_trade / 0.001 * costs.trade_count as f64
            } else { 0.0 };
            out.push_str(&format!("  İşlem Sayısı    : {}\n", costs.trade_count));
            out.push_str(&format!("  Toplam Komisyon : {:>10.4} $\n", costs.total_commission));
            out.push_str(&format!("  Toplam Spread   : {:>10.4} $\n", costs.total_spread));
            out.push_str(&format!("  Toplam Slippage : {:>10.4} $\n", costs.total_slippage));
            out.push_str(&format!("  Toplam Etki     : {:>10.4} $\n", costs.total_impact));
            out.push_str(&format!("  {}\n", "─".repeat(38)));
            out.push_str(&format!("  TOPLAM MALİYET  : {:>10.4} $\n", costs.total_cost_usd));
            out.push_str(&format!("  Ort. İşlem Baş. : {:>10.4} $\n", costs.avg_cost_per_trade));
            // Tür kırılımı (REG/SCP/SWG) — her tür için işlem sayısı, toplam maliyet ve ortalama
            out.push_str(&format!("  {}\n", "─".repeat(38)));
            out.push_str("  Tür kırılımı:\n");
            out.push_str(&format!("    REG : {:>3} işlem  toplam {:>9.4}$  ort {:>7.4}$  (kom {:.4}$ slip {:.4}$)\n",
                costs.regular.trade_count, costs.regular.total_usd, costs.regular.avg_per_trade(),
                costs.regular.commission, costs.regular.slippage));
            out.push_str(&format!("    SCP : {:>3} işlem  toplam {:>9.4}$  ort {:>7.4}$  (kom {:.4}$ slip {:.4}$)\n",
                costs.scalp.trade_count, costs.scalp.total_usd, costs.scalp.avg_per_trade(),
                costs.scalp.commission, costs.scalp.slippage));
            out.push_str(&format!("    SWG : {:>3} işlem  toplam {:>9.4}$  ort {:>7.4}$  (kom {:.4}$ slip {:.4}$)\n",
                costs.swing.trade_count, costs.swing.total_usd, costs.swing.avg_per_trade(),
                costs.swing.commission, costs.swing.slippage));
            if total_notional_est > 0.0 {
                let cost_pct = costs.total_cost_usd / total_notional_est * 100.0;
                out.push_str(&format!("  Tahmini Maliyet%: {:>10.4}%  (toplam ciro tahminine göre)\n", cost_pct));
            }
            // Maliyet dağılımı
            if costs.total_cost_usd > 0.0 {
                let comm_pct    = costs.total_commission / costs.total_cost_usd * 100.0;
                let spread_pct  = costs.total_spread     / costs.total_cost_usd * 100.0;
                let slip_pct    = costs.total_slippage   / costs.total_cost_usd * 100.0;
                let impact_pct  = costs.total_impact     / costs.total_cost_usd * 100.0;
                out.push_str(&format!("  Dağılım: Komis={:.1}%  Spread={:.1}%  Slip={:.1}%  Etki={:.1}%\n",
                    comm_pct, spread_pct, slip_pct, impact_pct));
            }
        }
    } else {
        out.push_str("  (Maliyet verisi alınamadı)\n");
    }
    out.push('\n');

    // ── PRE-LIVE KONTROL LİSTESİ ──────────────────────────────
    out.push_str(&format!("── PRE-LIVE KONTROL LİSTESİ {}\n", "─".repeat(33)));
    out.push_str("  Her export'ta otomatik güncellenir. ✅=geçti  ❌=geçmedi  ⚠=dikkat\n\n");

    let closed_guard = st.live_closed_trades.read().ok();
    let closed_ref: &[_] = closed_guard.as_deref().map(|v: &Vec<_>| v.as_slice()).unwrap_or(&[]);
    let costs_ok  = st.live_execution_costs.read().ok();
    let costs_ref = costs_ok.as_deref();

    // [1] Paper mode
    {
        let (icon, note) = if st.paper_mode {
            ("✅", "Paper mode aktif — live geçmeden önce kapat")
        } else {
            ("⚠ ", "LIVE mod aktif — gerçek emirler gönderiliyor!")
        };
        out.push_str(&format!("  [1] {} Paper mode           : {}\n", icon, note));
    }

    // [2] API key set
    {
        let (icon, note) = if st.api_key_set {
            ("✅", "API key yüklendi")
        } else {
            ("⚠ ", "API key YOK — DummyExecutor çalışıyor")
        };
        out.push_str(&format!("  [2] {} API Key               : {}\n", icon, note));
    }

    // [3] En az 1 kapalı trade (sistem çalıştı)
    {
        let n = closed_ref.len();
        let (icon, note) = if n > 0 {
            ("✅", format!("{} trade tamamlandı", n))
        } else {
            ("❌", "Henüz hiç trade kapanmadı — sistemi test et".to_string())
        };
        out.push_str(&format!("  [3] {} En az 1 trade         : {}\n", icon, note));
    }

    // [4] Komisyon sayacı — session bazlı takip (geçmiş snapshot dahil değil)
    {
        let cost_n   = costs_ref.map(|c| c.trade_count).unwrap_or(0);
        let total_comm = costs_ref.map(|c| c.total_commission).unwrap_or(0.0);
        let (icon, note) = if cost_n == 0 {
            ("⚠ ", "Bu oturumda henüz trade yok — komisyon takibi başlamadı".to_string())
        } else {
            let avg = if cost_n > 0 { total_comm / cost_n as f64 } else { 0.0 };
            ("✅", format!("Oturum: {} trade, toplam komisyon: ${:.4}, ortalama: ${:.4}/trade", cost_n, total_comm, avg))
        };
        out.push_str(&format!("  [4] {} Komisyon takibi       : {}\n", icon, note));
    }

    // [5] Sıfır-süre trade yok (duration bug)
    {
        let zero_dur = closed_ref.iter().filter(|t| {
            let ok = parse_ts(&t.opened_at).zip(parse_ts(&t.closed_at))
                .map(|(o, c)| (c - o).num_seconds().abs() > 0)
                .unwrap_or(false);
            !ok
        }).count();
        let (icon, note) = if zero_dur == 0 {
            ("✅", "Tüm trade'lerde geçerli süre var".to_string())
        } else {
            ("❌", format!("{} trade sıfır/geçersiz süreye sahip", zero_dur))
        };
        out.push_str(&format!("  [5] {} Trade süreleri        : {}\n", icon, note));
    }

    // [6] Kaldıraç gösterimi — tüm trade'lerde leverage alanı dolu
    {
        let no_lev = closed_ref.iter().filter(|t| t.leverage <= 0.0).count();
        let (icon, note) = if no_lev == 0 && !closed_ref.is_empty() {
            ("✅", "Tüm trade'lerde kaldıraç kaydı var".to_string())
        } else if closed_ref.is_empty() {
            ("⚠ ", "Trade yok".to_string())
        } else {
            ("❌", format!("{} trade'de leverage=0 (kayıt eksik)", no_lev))
        };
        out.push_str(&format!("  [6] {} Kaldıraç kaydı        : {}\n", icon, note));
    }

    // [7] Açık pozisyon boyutu — marjin bazlı %20 sınır (notional değil, marjin)
    // Kontrol: açık pozisyonlarda marjin = entry × qty / leverage > sermaye × %20 mi?
    // Notional (kaldıraçlı toplam) %20'yi kolayca aşar; önemli olan kullanılan marjin.
    {
        let capital = st.equity.max(1.0);
        let open_pos_ref = st.live_positions.try_read().ok();
        let oversized: Vec<String> = if let Some(positions) = open_pos_ref.as_ref() {
            positions.iter().filter_map(|(key, pos)| {
                if pos.entry_price <= 0.0 || pos.qty <= 0.0 { return None; }
                let lev = pos.leverage.max(1.0);
                let margin = pos.entry_price * pos.qty / lev;  // kullanılan marjin
                if margin > capital * 0.20 {
                    Some(format!("{} ({:.0}%)", key, margin / capital * 100.0))
                } else { None }
            }).collect()
        } else { vec![] };
        let (icon, note) = if oversized.is_empty() {
            ("✅", "Açık pozisyonlarda marjin sermayenin %20'sini aşmıyor".to_string())
        } else {
            let syms = oversized[..oversized.len().min(3)].join(", ");
            ("⚠ ", format!("Yüksek marjin pozisyon: {} — pozisyon boyutunu kontrol et", syms))
        };
        out.push_str(&format!("  [7] {} Pozisyon boyutu       : {}\n", icon, note));
    }

    // [8] DIAG uyarısı sayısı
    {
        let diag_n = st.diag_warn_count;
        let (icon, note) = if diag_n == 0 {
            ("✅", "Aktif DIAG uyarısı yok".to_string())
        } else {
            ("⚠ ", format!("{} aktif DIAG uyarısı var — Tab 2 veya rapordaki DIAG bölümünü kontrol et", diag_n))
        };
        out.push_str(&format!("  [8] {} DIAG uyarıları        : {}\n", icon, note));
    }

    // [9] Risk gate aktif mi
    {
        let p = &st.risk_gate.policy;
        let (icon, note) = if p.max_daily_loss_pct > 0.0 || p.max_drawdown_pct > 0.0 {
            ("✅", format!("RiskGate aktif — max_daily_loss={:.1}%  max_dd={:.1}%",
                p.max_daily_loss_pct, p.max_drawdown_pct))
        } else {
            ("⚠ ", "RiskGate limitleri sıfır — loss limiti tanımsız".to_string())
        };
        out.push_str(&format!("  [9] {} Risk gate             : {}\n", icon, note));
    }

    // [10] En az 1 LOOP olayı zaman damgası içeriyor (timestamp fix doğrulaması)
    {
        let has_dated_log = st.log.iter().any(|l| {
            l.starts_with('[') && l.len() > 11 && l.as_bytes().get(5) == Some(&b'-')
        });
        let (icon, note) = if has_dated_log {
            ("✅", "Log zaman damgaları tam tarih formatında (YYYY-MM-DD)")
        } else {
            ("⚠ ", "Log zaman damgaları eski formatta — bu export'tan itibaren düzeltildi")
        };
        out.push_str(&format!("  [10] {} Log zaman formatı   : {}\n", icon, note));
    }

    out.push_str("\n  NOT: Bu bölüm her 'R' (export) tuşunda yeniden hesaplanır.\n");
    out.push_str("       Tüm maddeler ✅ olduğunda live geçişe hazırsın.\n");
    out.push('\n');

    // ── MC + WALK-FORWARD DOĞRULAMA ───────────────────────────
    out.push_str(&format!("── MC + WALK-FORWARD DOĞRULAMA {}\n", "─".repeat(30)));
    if let Some(ref vr) = st.validation_result {
        let risk_str = match vr.risk_level {
            RiskLevel::Low      => "DUSUK",
            RiskLevel::Moderate => "ORTA",
            RiskLevel::High     => "YUKSEK",
            RiskLevel::Critical => "KRİTİK",
            RiskLevel::Unknown  => "BILINMIYOR",
        };
        out.push_str(&format!("  Strateji    : {}  |  Sembol: {}  |  Hesaplandi: {}\n",
            vr.strategy_name, vr.symbol, vr.computed_at));
        out.push_str(&format!("  Bilesik Skor: {:.1}/100  |  Risk Seviyesi: {}\n",
            vr.composite_score, risk_str));
        out.push('\n');
        out.push_str(&format!("  Monte Carlo ({} simülasyon, {} trade):\n",
            vr.mc_n_sims, vr.mc_n_trades));
        if vr.mc_n_trades > 0 {
            out.push_str(&format!("    Iflaslar        : {:.1}%  (50%+ kayıp riski)\n", vr.mc_ruin_pct));
            out.push_str(&format!("    Kârli Sim'ler   : {:.1}%  |  Beklenen Getiri: {:+.2}%\n",
                vr.mc_positive_pct, vr.mc_expected_ret));
            out.push_str(&format!("    Bakiye P5/P50/P95: ${:.0} / ${:.0} / ${:.0}\n",
                vr.mc_p5_balance, vr.mc_p50_balance, vr.mc_p95_balance));
            out.push_str(&format!("    Max Drawdown P50: {:.1}%  |  P95: {:.1}%\n",
                vr.mc_max_dd_p50, vr.mc_max_dd_p95));
        } else {
            out.push_str("    (Yeterli trade verisi yok — en az 1 trade gerekli)\n");
        }
        out.push('\n');
        out.push_str(&format!("  Walk-Forward ({} pencere, {} kârli):\n",
            vr.wf_windows, vr.wf_profitable));
        if vr.wf_windows > 0 {
            out.push_str(&format!("    Tutarlılık      : {:.1}%  (kârli pencere oranı)\n",
                vr.wf_consistency * 100.0));
            out.push_str(&format!("    OOS WinRate     : {:.1}%  |  OOS PnL: {:+.2}%\n",
                vr.wf_avg_oos_wr, vr.wf_avg_oos_pnl));
            out.push_str(&format!("    OOS Profit Fakt.: {:.2}  |  OOS Sharpe: {:.2}  |  OOS MaxDD: {:.1}%\n",
                vr.wf_avg_oos_pf, vr.wf_avg_oos_sharpe, vr.wf_avg_oos_dd));
        } else {
            out.push_str("    (Walk-Forward için yetersiz mum — en az 40 mum gerekli)\n");
        }
        out.push('\n');
        // Öneri
        let recommendation = if vr.composite_score >= 70.0 {
            "ONAY: Strateji güvenilir görünüyor — live geçiş değerlendirilebilir."
        } else if vr.composite_score >= 45.0 {
            "DİKKAT: Orta güven — küçük pozisyon büyüklüğü ile devam et, izle."
        } else if vr.composite_score > 0.0 {
            "UYARI: Düşük güven — parametreleri yeniden optimize et, sermayeyi koru."
        } else {
            "BİLİNMİYOR: Doğrulama henüz çalıştırılmadı — 'm' tuşuna basarak ML/backtest başlat."
        };
        out.push_str(&format!("  Öneri: {}\n", recommendation));
    } else {
        out.push_str("  (Doğrulama henüz çalıştırılmadı — 'm' tuşuna basarak ML eğitimini tetikle)\n");
    }
    out.push('\n');

    // ── KONSANTRASYON ANALİZİ ─────────────────────────────────
    out.push_str(&format!("── KONSANTRASYON ANALİZİ {}\n", "─".repeat(36)));
    if let Ok(closed) = st.live_closed_trades.read() {
        if closed.len() >= 3 {
            // Sembol başına PnL topla
            let mut sym_pnl: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
            for t in closed.iter() {
                *sym_pnl.entry(t.symbol.clone()).or_insert(0.0) += t.pnl;
            }
            let total_gross_wins: f64 = closed.iter().filter(|t| t.pnl > 0.0).map(|t| t.pnl).sum();
            let total_net: f64 = closed.iter().map(|t| t.pnl).sum();
            // Kazanç dağılımı (sadece kârlı semboller)
            let mut pnl_list: Vec<(&String, f64)> = sym_pnl.iter()
                .filter(|(_, pnl)| **pnl > 0.0)
                .map(|(k, v)| (k, *v))
                .collect();
            pnl_list.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            if !pnl_list.is_empty() && total_gross_wins > 0.0 {
                out.push_str(&format!("  {:<12} {:>10} {:>10}  {}\n",
                    "Sembol", "Net PnL $", "Kazanç %", "Uyarı"));
                out.push_str(&format!("  {}\n", "─".repeat(50)));
                for (sym, pnl) in &pnl_list {
                    let pct = pnl / total_gross_wins * 100.0;
                    let warn = if pct > 60.0 { "<-- KONSANTRASYON RISKI" }
                               else if pct > 40.0 { "<-- Yüksek oran" }
                               else { "" };
                    out.push_str(&format!("  {:<12} {:>+9.2}$ {:>9.1}%  {}\n",
                        sym, pnl, pct, warn));
                }
                // Net zarar eden semboller
                let loss_syms: Vec<(&String, f64)> = sym_pnl.iter()
                    .filter(|(_, pnl)| **pnl < 0.0)
                    .map(|(k, v)| (k, *v))
                    .collect();
                if !loss_syms.is_empty() {
                    out.push_str(&format!("  {}\n", "─".repeat(50)));
                    for (sym, pnl) in &loss_syms {
                        out.push_str(&format!("  {:<12} {:>+9.2}$  (net zarar)\n", sym, pnl));
                    }
                }
                out.push_str(&format!("  {}\n", "─".repeat(50)));
                out.push_str(&format!("  Genel Net PnL: {:+.2}$  |  Toplam Kazanç: ${:.2}\n",
                    total_net, total_gross_wins));
                // Konsantrasyon uyarısı
                if let Some((top_sym, top_pnl)) = pnl_list.first() {
                    let top_pct = top_pnl / total_gross_wins * 100.0;
                    if top_pct > 60.0 {
                        out.push_str(&format!(
                            "\n  [!] KONSANTRASYON RISKI: {} toplam kazancin {:.0}%'ini uretiyor.\n",
                            top_sym, top_pct));
                        out.push_str("      Diger semboller kaldirilirsa sistem performansi dramatik duzer.\n");
                        out.push_str("      Oneri: Sembol havuzunu genislet, per-sembol max marjin %15 ile sinirla.\n");
                    }
                }
            } else {
                out.push_str("  (Kârli kapalı işlem yok — analiz yapılamadı)\n");
            }
        } else {
            out.push_str("  (Yeterli kapanmış işlem yok — en az 3 trade gerekli)\n");
        }
    }
    out.push('\n');

    // ── ADAPTİF PARAMETRE DURUMU ──────────────────────────────
    out.push_str(&format!("── ADAPTİF PARAMETRELER {}\n", "─".repeat(37)));
    {
        let ap = &st.adaptive_params;
        out.push_str(&format!(
            "  SL ATR Çarpanı    : {:.2}x  (SL = ATR × çarpan, 0=sabit %)\n",
            ap.sl_atr_multiplier
        ));
        out.push_str(&format!(
            "  TP ATR Çarpanı    : {:.2}x  (TP = ATR × çarpan, max 4.0%)\n",
            ap.tp_atr_multiplier
        ));
        out.push_str(&format!(
            "  TSL Aktivasyon    : {:.1}%  (bu kâr olmadan trailing SL devreye girmez)\n",
            ap.trailing_sl_activation_pct
        ));
        out.push_str(&format!(
            "  SHORT HTF Blok    : {}  |  LONG HTF Blok : {}\n",
            if ap.short_htf_block { "AÇIK ✓" } else { "KAPALI" },
            if ap.long_htf_block  { "AÇIK ✓" } else { "KAPALI" }
        ));
        out.push_str(&format!(
            "  Futures SHORT Eşik: {:.2} conf  (min ML güveni SHORT açmak için)\n",
            ap.futures_short_min_conf
        ));
        out.push_str(&format!(
            "  Günlük SL Limit   : {}  |  Max Ardışık Kayıp: {}\n",
            if ap.max_daily_sl_per_symbol == 0 { "kapalı".to_string() }
            else { format!("{}/sembol/gün", ap.max_daily_sl_per_symbol) },
            if ap.max_consecutive_losses == 0 { "kapalı".to_string() }
            else { format!("{} kayıp → dur", ap.max_consecutive_losses) }
        ));
        out.push_str(&format!(
            "  Otonom Ayar Periy : {}\n",
            if ap.adjust_every_n_trades == 0 { "kapalı".to_string() }
            else { format!("her {} işlem", ap.adjust_every_n_trades) }
        ));
    }
    out.push('\n');

    // ── TSL / TP KORUMA ANALİZİ ──────────────────────────────
    out.push_str(&format!("── TSL / TP KORUMA ANALİZİ {}\n", "─".repeat(34)));
    if let Ok(closed) = st.live_closed_trades.read() {
        let total = closed.len();
        if total == 0 {
            out.push_str("  (kapanmış işlem yok)\n");
        } else {
            // TSL kapanış analizi
            let tsl_trades: Vec<_> = closed.iter().filter(|t|
                t.exit_reason.contains("trailing")).collect();
            let tsl_n = tsl_trades.len();
            let tsl_pct = tsl_n as f64 / total as f64 * 100.0;

            // Anlık TSL: açılıp 60 sn içinde TSL ile kapanan
            let instant_tsl = tsl_trades.iter().filter(|t| {
                let dur = parse_ts(&t.opened_at).and_then(|o|
                    parse_ts(&t.closed_at).map(|c| (c - o).num_seconds().abs()));
                dur.map(|d| d < 60).unwrap_or(false)
            }).count();

            // Kâr ile kapanan TSL (pozitif PnL)
            let tsl_wins = tsl_trades.iter().filter(|t| t.pnl > 0.0).count();

            // TSL kapanış ortalama süresi
            let tsl_durations: Vec<u64> = tsl_trades.iter().filter_map(|t| {
                let o = parse_ts(&t.opened_at)?;
                let c = parse_ts(&t.closed_at)?;
                Some((c - o).num_seconds().abs() as u64)
            }).collect();
            let tsl_avg_dur = if tsl_durations.is_empty() { 0u64 }
                else { tsl_durations.iter().sum::<u64>() / tsl_durations.len() as u64 };

            out.push_str(&format!(
                "  TSL Kapanış       : {}/{} işlem ({:.1}%) | Kârlı={} | Zararlı={}\n",
                tsl_n, total, tsl_pct, tsl_wins, tsl_n.saturating_sub(tsl_wins)
            ));
            if tsl_n > 0 {
                let fmt_dur = |s: u64| -> String {
                    let m = s / 60; let sec = s % 60;
                    if m > 0 { format!("{}dk{}sn", m, sec) } else { format!("{}sn", sec) }
                };
                out.push_str(&format!(
                    "  TSL Ort. Süre     : {}  |  Anlık TSL (<60sn): {}/{}\n",
                    fmt_dur(tsl_avg_dur), instant_tsl, tsl_n
                ));
                // Uyarı: anlık TSL çok fazlaysa TSL activation veya ATR çok dar
                if instant_tsl > tsl_n / 2 && tsl_n >= 3 {
                    out.push_str(&format!(
                        "  ⚠ Uyarı: TSL kapanışlarının %{:.0}'i anlık (<60sn) — TSL aktivasyon % artırılmalı!\n",
                        instant_tsl as f64 / tsl_n as f64 * 100.0
                    ));
                }
            }

            // Per-sembol tekrar kapanış analizi (aynı sembolde 3+ TSL)
            let mut tsl_by_sym: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
            for t in &tsl_trades {
                *tsl_by_sym.entry(t.symbol.as_str()).or_insert(0) += 1;
            }
            let mut repeat_syms: Vec<_> = tsl_by_sym.iter().filter(|(_, &c)| c >= 3).collect();
            repeat_syms.sort_by(|a, b| b.1.cmp(a.1));
            if !repeat_syms.is_empty() {
                out.push_str("  Tekrar TSL Sembol : ");
                let parts: Vec<String> = repeat_syms.iter().map(|(s, c)| format!("{}({}x)", s, c)).collect();
                out.push_str(&parts.join(", "));
                out.push('\n');
            }

            // TP Yön Bloğu — ters yön kapanmış işlem analizi
            // Kaç kez TP sonrası aynı sembol ters yönde açılıp zarar etmiş?
            let tp_then_opposite_loss = {
                let mut sorted = closed.clone();
                sorted.sort_by(|a, b| a.closed_at.cmp(&b.closed_at));
                let mut count = 0usize;
                for i in 1..sorted.len() {
                    let prev = &sorted[i - 1];
                    let curr = &sorted[i];
                    if prev.symbol == curr.symbol
                        && prev.exit_reason.contains("take_profit")
                        && prev.pnl > 0.0
                        && curr.is_long != prev.is_long
                        && curr.pnl < 0.0
                    {
                        // Süre kontrolü: 2 saat içinde mi açılmış?
                        let gap = parse_ts(&prev.closed_at).and_then(|pc|
                            parse_ts(&curr.opened_at).map(|co| (co - pc).num_seconds().abs()));
                        if gap.map(|g| g < 7200).unwrap_or(false) {
                            count += 1;
                        }
                    }
                }
                count
            };
            if tp_then_opposite_loss > 0 {
                out.push_str(&format!(
                    "  TP→Ters Yön Kayıp: {} kez tespit edildi (Fix D koruması şimdi aktif)\n",
                    tp_then_opposite_loss
                ));
            } else {
                out.push_str("  TP→Ters Yön Kayıp: Tespit edilmedi ✓\n");
            }
        }
    } else {
        out.push_str("  (veri alınamadı)\n");
    }
    out.push('\n');

    // ── OTONOM TAVSİYELER ────────────────────────────────────
    out.push_str(&format!("── OTONOM TAVSİYELER {}\n", "─".repeat(40)));
    out.push_str("  (Mevcut trade istatistiklerine göre otomatik üretilmiştir)\n");
    {
        let mut advice: Vec<String> = Vec::new();
        let ap = &st.adaptive_params;

        if let Ok(lr) = st.live_risk.read() {
            let win_rate = if lr.session_closed > 0 {
                lr.session_wins as f64 / lr.session_closed as f64 * 100.0
            } else { 0.0 };
            let session_rr = lr.session_rr;

            // Erken uyarılar: 1+ trade yeterli
            if lr.session_closed >= 1 && win_rate == 0.0 {
                advice.push(format!(
                    "🚨 SIFIR KAZANÇ: {}/{} trade hepsini kaybetti (WR=%0) — aşağıdaki ayarlar gözden geçirilmeli",
                    lr.session_closed - lr.session_wins, lr.session_closed
                ));
            }

            // Win rate çok düşük → TP'yi küçült (daha ulaşılabilir hedef) — 3+ trade'den itibaren
            if lr.session_closed >= 3 && win_rate < 35.0 {
                advice.push(format!(
                    "⚠ Win rate {:.0}% < 35% — TP ATR çarpanı {:.1}x → {:.1}x küçültün (ayarlar #21)",
                    win_rate, ap.tp_atr_multiplier,
                    (ap.tp_atr_multiplier - 0.2).max(0.8)
                ));
            }
            // RR çok düşük → SL biraz büyüt veya TP büyüt — 3+ trade'den itibaren
            if lr.session_closed >= 3 && session_rr < 1.0 {
                advice.push(format!(
                    "⚠ R/R {:.2} < 1.0 — SHORT HTF blok açın (#19) ve TSL aktivasyon %{:.1} → %{:.1} artırın",
                    session_rr, ap.trailing_sl_activation_pct,
                    (ap.trailing_sl_activation_pct + 0.5).min(4.0)
                ));
            }
        }

        if let Ok(closed) = st.live_closed_trades.read() {
            let tsl_n = closed.iter().filter(|t| t.exit_reason.contains("trailing")).count();
            let total = closed.len();

            // Anlık TSL uyarısı: 1+ TSL kapatma yeterli
            {
                let tsl_trades: Vec<_> = closed.iter().filter(|t|
                    t.exit_reason.contains("trailing")).collect();
                let instant_tsl = tsl_trades.iter().filter(|t| {
                    let dur = parse_ts(&t.opened_at).and_then(|o|
                        parse_ts(&t.closed_at).map(|c| (c - o).num_seconds().abs()));
                    dur.map(|d| d < 300).unwrap_or(false) // 5 dk'dan kısa
                }).count();
                if instant_tsl > 0 {
                    advice.push(format!(
                        "🚨 {} anlık TSL kapanışı (<5dk) — Fix B (ATR-aware TSL) aktif mi? TSL aktivasyon %{:.1} → %{:.1} artırın",
                        instant_tsl, ap.trailing_sl_activation_pct,
                        (ap.trailing_sl_activation_pct + 1.0).min(5.0)
                    ));
                }
            }

            // Büyük tek kayıp uyarısı
            {
                let capital = st.config.capital;
                let big_losses: Vec<_> = closed.iter()
                    .filter(|t| t.pnl < -(capital * 0.01)) // sermayenin %1'inden büyük kayıp
                    .collect();
                if !big_losses.is_empty() {
                    let worst = big_losses.iter().min_by(|a,b| a.pnl.partial_cmp(&b.pnl).unwrap()).unwrap();
                    advice.push(format!(
                        "💰 En büyük tek kayıp: {} {:.2}$ (sermayenin {:.1}%) — Pozisyon boyutu fazla olabilir, trade_amount azaltın",
                        worst.symbol, worst.pnl, (worst.pnl.abs() / capital) * 100.0
                    ));
                }
            }

            if total >= 3 {
                let tsl_pct = tsl_n as f64 / total as f64 * 100.0;
                // TSL ağırlıklı kapanış → TSL aktivasyon eşiği çok düşük
                if tsl_pct > 50.0 && tsl_n > 0 {
                    // Karlı TSL var mı kontrol et
                    let profitable_tsl = closed.iter().filter(|t|
                        t.exit_reason.contains("trailing") && t.pnl > 0.0).count();
                    if profitable_tsl == 0 {
                        advice.push(format!(
                            "⚠ İşlemlerin %{:.0}'i TSL ile kapanıyor & hepsi zararda — TSL aktivasyon %{:.1} → %{:.1} artırın (ayarlar #22)",
                            tsl_pct, ap.trailing_sl_activation_pct,
                            (ap.trailing_sl_activation_pct + 1.0).min(5.0)
                        ));
                    } else {
                        advice.push(format!(
                            "⚠ İşlemlerin %{:.0}'i TSL ile kapanıyor — TSL aktivasyon %{:.1} → %{:.1} artırın (ayarlar #22)",
                            tsl_pct, ap.trailing_sl_activation_pct,
                            (ap.trailing_sl_activation_pct + 0.5).min(4.0)
                        ));
                    }
                }
                // Çok fazla SL kapanışı → SL ATR çarpanı yüksek, dar SL
                let sl_n = closed.iter().filter(|t| {
                    let r = &t.exit_reason;
                    (r.contains("sl") || r.contains("SL")) && !r.contains("trailing")
                }).count();
                if sl_n as f64 / total as f64 > 0.5 {
                    advice.push(format!(
                        "⚠ İşlemlerin %{:.0}'i SL ile kapanıyor — SL ATR çarpanı {:.2}x → {:.2}x artırın (ayarlar #22)",
                        sl_n as f64 / total as f64 * 100.0,
                        ap.sl_atr_multiplier,
                        (ap.sl_atr_multiplier + 0.25).min(2.5)
                    ));
                }
                // Günlük SL limiti kapalıyken çok kayıp
                if ap.max_daily_sl_per_symbol == 0 && sl_n + tsl_n > total * 2 / 3 {
                    advice.push(
                        "💡 Günlük SL limiti kapalı — Gün SL Limiti=3 ile tekrarlı zararı sınırlayın (ayarlar #23)".to_string()
                    );
                }
            }
            // SHORT işlem kayıpları analizi — 2+ SHORT yeterli
            let short_losses: Vec<_> = closed.iter().filter(|t| !t.is_long && t.pnl < 0.0).collect();
            let short_total: Vec<_> = closed.iter().filter(|t| !t.is_long).collect();
            if short_total.len() >= 2 {
                let short_loss_pct = short_losses.len() as f64 / short_total.len() as f64 * 100.0;
                if short_loss_pct > 60.0 && !ap.short_htf_block {
                    advice.push(format!(
                        "⚠ SHORT işlemlerinin %{:.0}'i zararda ({}/{}) — SHORT HTF Blok açın (ayarlar #19)",
                        short_loss_pct, short_losses.len(), short_total.len()
                    ));
                }
            }
        }

        // Ardışık kayıp & otonom mod uyarısı — 1+ kayıp serisinde
        if let Ok(lr) = st.live_risk.read() {
            if lr.loss_streak >= 2 && ap.adjust_every_n_trades == 0 {
                advice.push(format!(
                    "💡 {} ardışık kayıp & Otonom Mod kapalı — Otonom Mod=5 açmayı düşünün (ayarlar #26)",
                    lr.loss_streak
                ));
            }
            // Özel: 4+ ardışık kayıp → daha sert uyarı
            if lr.loss_streak >= 4 {
                advice.push(format!(
                    "🛑 {} ardışık kayıp — Trading'i geçici durdurun, strateji parametrelerini gözden geçirin!",
                    lr.loss_streak
                ));
            }
        }

        if advice.is_empty() {
            out.push_str("  ✅ Mevcut parametreler istatistiksel olarak makul görünüyor.\n");
        } else {
            for (i, a) in advice.iter().enumerate() {
                out.push_str(&format!("  [{}] {}\n", i + 1, a));
            }
        }
    }
    out.push('\n');

    // ── DIAG UYARILARI ────────────────────────────────────────
    if !st.diag_alerts.is_empty() {
        out.push_str(&format!("── DIAG UYARILARI ({} adet) {}\n", st.diag_alerts.len(), "─".repeat(33)));
        for (i, alert) in st.diag_alerts.iter().enumerate() {
            out.push_str(&format!("  [{}] {}\n", i + 1, alert));
        }
        out.push('\n');
    }

    // ── PIPELINE SAĞLIK MONİTÖRÜ ────────────────────────────────
    out.push_str(&format!("── PIPELINE SAĞLIK MONİTÖRÜ {}\n", "─".repeat(32)));
    if let Ok(ph) = st.live_pipeline.read() {
        // Veri akışı
        let ws_tag = if ph.ws_stale { " ⚠STALE" } else { " ✓" };
        out.push_str(&format!(
            "  Veri Akışı   : son_mum_yaşı={}sn{}  |  son={}\n",
            ph.candle_age_secs, ws_tag, ph.last_candle_at
        ));

        // DB
        out.push_str(&format!(
            "  DB           : {}\n",
            if ph.db_connected { "✓ Bağlı" } else { "✗ BAĞLANTI YOK" }
        ));

        // Funding rate
        if ph.funding_applicable {
            let fr_tag = if ph.funding_age_secs > 360 { " ⚠ESKI" } else { " ✓" };
            out.push_str(&format!(
                "  Funding Rate : {:.4}%{}  yaş={}sn\n",
                ph.funding_rate * 100.0, fr_tag, ph.funding_age_secs
            ));
        } else {
            out.push_str("  Funding Rate : (Spot piyasası — uygulanmaz)\n");
        }

        // Kelly
        out.push_str(&format!(
            "  Kelly        : aktif={}  scale={:.3}  trades={}/{}\n",
            if ph.kelly_active { "EVET" } else { "Hayır" },
            ph.kelly_scale, ph.kelly_trades_so_far, ph.kelly_min_trades
        ));

        // Evrim
        out.push_str(&format!(
            "  Evrim        : cycle={}  stuck={}  mini_evol={}  son_tetik={}\n",
            ph.evolution_cycle, ph.evolution_stuck_count,
            ph.mini_evol_count, ph.last_evolution_trigger
        ));

        // Drift & loss streak
        let drift_tag = if ph.drift_score > ph.drift_threshold { " ⚠YÜKSEK" } else { " ✓" };
        out.push_str(&format!(
            "  Drift        : {:.4}{}  (eşik={:.4})\n",
            ph.drift_score, drift_tag, ph.drift_threshold
        ));
        let streak_tag = if ph.loss_streak >= ph.loss_streak_threshold { " ⚠STREAK" } else { " ✓" };
        out.push_str(&format!(
            "  Kayıp Serisi : {}/{}{}  |  Oturum={}/{} (win/kapalı)\n",
            ph.loss_streak, ph.loss_streak_threshold, streak_tag,
            ph.session_wins, ph.session_closed
        ));

        // Anomaliler
        out.push_str(&format!("\n  Aktif Anomaliler ({} adet):\n", ph.anomalies.len()));
        if ph.anomalies.is_empty() {
            out.push_str("    ✅ Anomali yok\n");
        } else {
            for (i, a) in ph.anomalies.iter().enumerate() {
                let sev_str = match a.severity {
                    memos_trading_core::robot::robotic_loop::AnomSeverity::Warning  => "UYARI",
                    memos_trading_core::robot::robotic_loop::AnomSeverity::Critical => "KRİTİK",
                };
                let fix_str = if a.auto_fixed {
                    "  [OTOMATİK DÜZELTİLDİ]".to_string()
                } else if !a.fix_hint.is_empty() {
                    format!("  → {}", a.fix_hint)
                } else {
                    String::new()
                };
                out.push_str(&format!(
                    "    [{}] [{}] {}{}\n",
                    i + 1, sev_str, a.message, fix_str
                ));
            }
        }

        // Onarım günlüğü
        out.push_str(&format!("\n  Onarım Günlüğü (son {} kayıt):\n", ph.repair_log.len()));
        if ph.repair_log.is_empty() {
            out.push_str("    (Henüz onarım kaydı yok)\n");
        } else {
            for entry in &ph.repair_log {
                out.push_str(&format!("    {}\n", entry));
            }
        }
    } else {
        out.push_str("  (Pipeline verisi henüz oluşturulmadı — robotic_loop başlatıldıktan sonra görünür)\n");
    }
    out.push('\n');

    out.push_str(&format!("\n{}\n", sep));
    out
}

// Döndürür: toplam eklenen türetilmiş mum sayısı
fn aggregate_from_1m(
    conn: &rusqlite::Connection,
    exchange: &str,
    market: &str,
    symbol: &str,
    candles_1m: &[memos_trading_core::types::Candle],
) -> usize {
    use std::collections::BTreeMap;

    // (interval_adı, bucket_ms): timestamp'ı ms olarak hizalar
    const TARGETS: &[(&str, i64)] = &[
        ("5m",    5  * 60 * 1_000),
        ("15m",   15 * 60 * 1_000),
        ("30m",   30 * 60 * 1_000),
        ("1h",    60 * 60 * 1_000),
        ("4h",  4 * 60 * 60 * 1_000),
        ("1d", 24 * 60 * 60 * 1_000),
    ];

    let mut total = 0usize;

    for &(intv, bucket_ms) in TARGETS {
        // bucket_ts_ms → (open, high, low, close, volume)
        // BTreeMap sıralı olduğundan en küçük ts ilk insert edilir → open doğru
        let mut buckets: BTreeMap<i64, (f64, f64, f64, f64, f64)> = BTreeMap::new();

        for c in candles_1m {
            let ts_ms   = c.timestamp.timestamp_millis();
            let bucket  = (ts_ms / bucket_ms) * bucket_ms;
            buckets.entry(bucket)
                .and_modify(|e| {
                    if c.high > e.1 { e.1 = c.high; }  // high
                    if c.low  < e.2 { e.2 = c.low;  }  // low
                    e.3 = c.close;                      // close (son)
                    e.4 += c.volume;                    // volume
                })
                .or_insert((c.open, c.high, c.low, c.close, c.volume));
        }

        for (ts_ms, (open, high, low, close, volume)) in &buckets {
            let r = conn.execute(
                "INSERT OR REPLACE INTO candles \
                 (exchange, market, symbol, interval, timestamp, open, high, low, close, volume) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                rusqlite::params![
                    exchange, market, symbol, intv,
                    ts_ms, open, high, low, close, volume
                ],
            );
            if r.map(|n| n > 0).unwrap_or(false) {
                total += 1;
            }
        }
    }
    total
}

/// aggregate_from_1m sonrasında sembol/market için her HTF interval'deki
/// (mum_sayısı, son_timestamp) çiftini DB'den okuyup `htf_counts` haritasına yazar.
fn update_htf_candle_counts(
    conn:       &rusqlite::Connection,
    htf_counts: &mut std::collections::HashMap<String, std::collections::HashMap<String, (usize, String)>>,
    symbol:     &str,
    market:     &str,
) {
    let entry = htf_counts.entry(symbol.to_string()).or_default();
    for &intv in STANDARD_INTERVALS {
        let count: usize = conn.query_row(
            "SELECT COUNT(*) FROM candles WHERE symbol=?1 AND market=?2 AND interval=?3",
            rusqlite::params![symbol, market, intv],
            |row| row.get::<_, i64>(0),
        ).unwrap_or(0) as usize;
        // Son mum zamanı: ms → "YYYY-MM-DD HH:MM"
        let last_ts: String = conn.query_row(
            "SELECT MAX(timestamp) FROM candles WHERE symbol=?1 AND market=?2 AND interval=?3",
            rusqlite::params![symbol, market, intv],
            |row| row.get::<_, Option<i64>>(0),
        ).ok().flatten()
        .map(|ms| {
            use chrono::{TimeZone, Local};
            Local.timestamp_millis_opt(ms).single()
                .map(|dt| dt.format("%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "-".to_string())
        })
        .unwrap_or_else(|| "-".to_string());
        entry.insert(intv.to_string(), (count, last_ts));
    }
}

/// Başlangıç senkron gap-fill: aktif sembol/market için DB'deki son 1m mumdan şimdiye
/// kadar eksik olanları tek bir Binance REST isteğiyle çeker (max 500 mum ≈ 8.3 saat).
/// Trading loop başlamadan çağrılır; 15s timeout ile hızlı başarısızlık garantisi var.
/// Asenkron download worker, bu fonksiyondan bağımsız devam eder (daha geniş geçmiş).
fn startup_sync_gap_fill(db_path: &str, exchange: &str, market: &str, symbol: &str) {
    use chrono::Utc;
    use memos_trading_core::types::Candle;

    let interval    = "1m";
    let interval_ms = 60_000i64;
    let batch       = 500usize;
    let now_ms      = Utc::now().timestamp_millis();

    let raw_table = database_writer::get_table_name(exchange, market);

    // DB'deki son 1m timestamp'i bul
    let last_ts_ms: Option<i64> = database_writer::open_connection(db_path).ok()
        .and_then(|c| {
            let sql = format!(
                "SELECT MAX(CASE WHEN timestamp > 1000000000000 \
                               THEN timestamp/1000*1000 \
                               ELSE timestamp*1000 END) \
                 FROM {} WHERE symbol=?1 AND interval=?2",
                raw_table
            );
            c.query_row(&sql, rusqlite::params![symbol, interval],
                |row| row.get::<_, Option<i64>>(0)).ok().flatten()
        });

    let start_ms = match last_ts_ms {
        Some(last) => {
            let gap_candles = (now_ms - last) / interval_ms;
            if gap_candles <= 2 { return; } // zaten güncel
            last + interval_ms
        }
        None => now_ms - batch as i64 * interval_ms,
    };

    let base_url = if market == "futures" {
        "https://fapi.binance.com/fapi/v1/klines"
    } else {
        "https://api.binance.com/api/v3/klines"
    };

    let url = format!(
        "{}?symbol={}&interval={}&startTime={}&endTime={}&limit={}",
        base_url, symbol, interval, start_ms, now_ms, batch
    );

    let client = match reqwest::blocking::ClientBuilder::new()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    let data: Vec<Vec<serde_json::Value>> = match client.get(&url).send()
        .and_then(|r| r.json())
    {
        Ok(d) => d,
        Err(e) => {
            log::warn!("startup-gap-fill: HTTP hatası ({}/{}): {}", symbol, market, e);
            return;
        }
    };

    if data.is_empty() { return; }

    let candles: Vec<Candle> = data.iter().filter_map(|k| {
        let ts_ms  = k.get(0)?.as_i64()?;
        let open:  f64 = k.get(1)?.as_str()?.parse().ok()?;
        let high:  f64 = k.get(2)?.as_str()?.parse().ok()?;
        let low:   f64 = k.get(3)?.as_str()?.parse().ok()?;
        let close: f64 = k.get(4)?.as_str()?.parse().ok()?;
        let vol:   f64 = k.get(7)?.as_str()?.parse().ok()?;
        let ts = chrono::DateTime::from_timestamp(ts_ms / 1000, 0)?;
        Some(Candle {
            timestamp: ts.with_timezone(&chrono::Utc),
            open, high, low, close, volume: vol,
            symbol: symbol.to_string(),
            interval: interval.to_string(),
        })
    }).collect();

    if let Ok(conn) = database_writer::open_connection(db_path) {
        if let Ok((ins, _)) = database_writer::save_candles_bulk(&conn, exchange, market, &candles) {
            if ins > 0 {
                log::info!("startup-gap-fill: {} mum eklendi ({} {})", ins, symbol, market);
            }
        }
    }
}

//  1. Aktif sembol + en iyi N aday için paralel indirme
//  2. Spot → api.binance.com/api/v3/klines
//     Futures → fapi.binance.com/fapi/v1/klines
//  3. Yeni mumları DB'ye yazar (save_candles_bulk — duplicate'ları atlar)
//  4. İlerlemeyi AppState.last_download'a yazar, log'a basar

fn run_download_worker(
    app_state:    Arc<Mutex<AppState>>,
    stop_signal:  Arc<AtomicBool>,
    config:       OtoConfig,
) {
    std::thread::spawn(move || {
        use std::sync::atomic::Ordering;

        if !config.download_enabled {
            return;
        }

        std::thread::sleep(Duration::from_secs(5)); // hızlı başla

        let download_every_secs = config.download_every_mins * 60;
        let mut elapsed = download_every_secs; // ilk döngüde hemen indir
        let tick = 1u64;
        // Bayt veri tazele sayacı: her cycle'da skor=0 ama verisi olan 1 sembol döngüsel tazelenir
        let mut stale_refresh_idx = 0usize;
        // Gap-fill ilerleme: sembol başına ilk boşluk boyutu — döngüler arası korunur
        let mut gap_initial_map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        // Live price ticker: aktif sembol interval≠1m iken her 10s'de 2 adet 1m mum çek → live_price güncelle
        let live_price_every_secs = 10u64;
        let mut live_price_elapsed = live_price_every_secs; // ilk tick'te hemen çalış

        // Tokio runtime — async HTTP çağrıları için
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("download worker runtime");

        loop {
            if stop_signal.load(Ordering::Relaxed) { break; }

            let triggered = {
                if let Ok(st) = app_state.lock() {
                    st.download_trigger.swap(false, Ordering::Relaxed)
                } else { false }
            };

            if triggered || elapsed >= download_every_secs {
                elapsed = 0;

                // ── İndirilecek hedefleri belirle ────────────────────────
                // Tüm hedefler için interval="1m" — aggregate_from_1m zinciri
                // 5m/15m/30m/1h/4h/1d'yi otomatik türetir. Symbol scanner
                // tüm kombinasyonları backtest eder, AUTO en iyisini seçer.
                let targets: Vec<(String, String, String, String)> = {
                    if let Ok(mut st) = app_state.lock() {
                        let (e, m, s, _) = st.active_trade_target();
                        // Aktif sembol: 1m + HTF (1h, 4h) — HTF trend filtresi için taze veri
                        let mut list: Vec<(String, String, String, String)> = vec![
                            (e.clone(), m.clone(), s.clone(), "1m".to_string()),
                            (e.clone(), m.clone(), s.clone(), "1h".to_string()),
                            (e.clone(), m.clone(), s.clone(), "4h".to_string()),
                        ];
                        // En iyi N aday — interval fark etmez, hepsi 1m indirilir
                        for cand in st.symbol_candidates.iter().take(config.download_top_n + 1) {
                            let key = (cand.exchange.clone(), cand.market.clone(),
                                       cand.symbol.clone(), "1m".to_string());
                            if !list.contains(&key) {
                                list.push(key);
                                if list.len() > config.download_top_n + 1 { break; }
                            }
                        }
                        // Aktif worker'ların sembollerini 1h olarak ekle
                        // (1m primary dışındaki worker'lar stale kalmasın)
                        for ws in st.orchestrator.worker_status() {
                            if ws.symbol == st.active_symbol.symbol { continue; }
                            let wkey = (config.exchange.clone(), ws.market.clone(),
                                        ws.symbol.clone(), "1h".to_string());
                            if !list.contains(&wkey) {
                                list.push(wkey);
                            }
                        }

                        // ── Screener adayları — yeni keşfedilmiş, henüz indirilmemiş semboller
                        // Kuyrukta bekleyenler (New/Queued) indir; indirme başlar başlamaz
                        // Queued olarak işaretle (download sonrası symbol_trigger tetikler → otomatik puanlama)
                        let screener_new: Vec<(String, String)> = st.screener_candidates.iter()
                            .filter(|c| c.status == ScreenerStatus::New || c.status == ScreenerStatus::Queued)
                            .map(|c| (config.exchange.clone(), c.symbol.clone()))
                            .collect();
                        for (se, ss) in &screener_new {
                            // 1h interval ile indir — symbol_selector_worker backtest için yeterli
                            let key_1h = (se.clone(), market_str_for_screener(&config.market), ss.clone(), "1h".to_string());
                            if !list.contains(&key_1h) { list.push(key_1h); }
                            let key_4h = (se.clone(), market_str_for_screener(&config.market), ss.clone(), "4h".to_string());
                            if !list.contains(&key_4h) { list.push(key_4h); }
                        }
                        // Screener adaylarını Queued olarak işaretle
                        for c in &mut st.screener_candidates {
                            if c.status == ScreenerStatus::New { c.status = ScreenerStatus::Queued; }
                        }

                        // Kısır döngü kırıcı: skor=0 ama DB'de verisi olan sembollerden
                        // döngüsel olarak 1 tanesini her cycle'da tazele.
                        // Stablecoin ve is_low_price filtrelerine takılanlar atlanır.
                        let stale_pool: Vec<(String, String, String)> = st.symbol_candidates.iter()
                            .filter(|c| c.score == 0.0
                                && c.candle_count >= 30
                                && !is_stablecoin_pair(&c.symbol)
                                && !is_low_price(c.last_price))
                            .map(|c| (c.exchange.clone(), c.market.clone(), c.symbol.clone()))
                            .collect();
                        if !stale_pool.is_empty() {
                            // Her döngüde 2 stale sembol tazele (30+ stale sembol → tek tek çok yavaş)
                            for offset in 0..2usize {
                                let idx = (stale_refresh_idx + offset) % stale_pool.len();
                                let (se, sm, ss) = &stale_pool[idx];
                                // 1h indir: 500 1h mum ≈ 20 gün → backtest yeterli,
                                // aggregate_from_1m çağrılmaz (interval != "1m")
                                let stale_key = (se.clone(), sm.clone(), ss.clone(), "1h".to_string());
                                if !list.contains(&stale_key) {
                                    list.push(stale_key);
                                }
                            }
                            stale_refresh_idx = stale_refresh_idx.wrapping_add(2);
                        }
                        list
                    } else {
                        vec![(config.exchange.clone(), config.market.clone(),
                              config.symbol.clone(), "1m".to_string())]
                    }
                };

                // Başladığını işaretle
                let session_start_ms = chrono::Utc::now().timestamp_millis();
                if let Ok(mut st) = app_state.lock() {
                    st.download_active = true;
                    st.download_progress = None;
                    st.push_log(format!(
                        "⬇ İndirme başladı: {} hedef × {} mum",
                        targets.len(), config.download_candle_limit
                    ));
                }

                let mut total_inserted = 0usize;
                let mut total_skipped  = 0usize;
                let mut total_derived  = 0usize;
                let mut errors         = 0usize;

                // Hedef durum listesi — tüm targetlar için başlangıçta "⏳ Bekliyor"
                let mut target_labels: Vec<(String, String, String, String, i64, usize)> = targets.iter()
                    .map(|(_, mkt, sym, intv)| (mkt.clone(), sym.clone(), intv.clone(), "⏳ Bekliyor".to_string(), 0i64, 0usize))
                    .collect();

                for (target_idx, (exchange, market, symbol, interval)) in targets.iter().enumerate() {
                    if stop_signal.load(Ordering::Relaxed) { break; }

                    // interval_ms: boşluk kontrolü için mum süresi
                    let interval_ms: i64 = interval_to_ms(interval.as_str());
                    let dl_limit = config.download_candle_limit as i64;
                    let map_key  = format!("{}/{}/{}", market, symbol, interval);

                    // İlk boşluk ölçümü — DB sorgusu yalnızca bir kez yapılır
                    // save_candles_bulk → candles_{exchange}_{market} tablosuna yazar;
                    // gap sorgusu da aynı tablodan okunmalı (aksi halde her açılışta sıfırlanır)
                    let now_init = chrono::Utc::now().timestamp_millis();
                    let raw_table = database_writer::get_table_name(exchange, market);
                    let raw_max_ts_secs: Option<i64> =
                        database_writer::open_connection(&config.db_path).ok()
                        .and_then(|c| {
                            // Karma format normalize: ms (>1_000_000_000_000) → saniyeye çevir,
                            // ardından MAX al. Böylece eski ms-format kayıtlar yeni sn-format
                            // kayıtları gölgelemez.
                            let sql = format!(
                                "SELECT MAX(CASE WHEN timestamp > 1000000000000 \
                                               THEN timestamp/1000 \
                                               ELSE timestamp END) \
                                 FROM {} WHERE symbol=?1 AND interval=?2",
                                raw_table
                            );
                            c.query_row(&sql,
                                rusqlite::params![symbol, interval],
                                |row: &rusqlite::Row| row.get::<_, Option<i64>>(0),
                            ).ok().flatten()
                        });
                    let init_start: Option<i64> = match raw_max_ts_secs {
                        Some(raw_ts) => {
                            // Tabloda karma timestamp var: saniye (~10 basamak) veya ms (~13 basamak)
                            let ts_ms = if raw_ts > 1_000_000_000_000 {
                                raw_ts          // zaten ms
                            } else {
                                raw_ts * 1000   // saniye → ms
                            };
                            let next = ts_ms + interval_ms;
                            let gap  = (now_init - next) / interval_ms;
                            // gap > 0: en az 1 eksik mum var → indir.
                            // (Eski hata: gap > dl_limit — 1500'den az boşluk "taze" sayılıyordu)
                            if gap > 0 { Some(next) } else { None }
                        }
                        None => {
                            // Tablo yok veya sembol için hiç veri yok → dl_limit+1 kadar geriye başla
                            Some(now_init - (dl_limit + 1) * interval_ms)
                        }
                    };

                    // Gap yoksa bu hedefi taze olarak işaretle ve geç
                    let Some(mut fetch_start) = init_start else {
                        gap_initial_map.remove(&map_key);
                        if let Some(lbl) = target_labels.get_mut(target_idx) {
                            lbl.3 = "✓ taze".to_string();
                        }
                        if let Ok(mut st) = app_state.try_lock() {
                            if let Some(ref mut dp) = st.download_progress {
                                dp.target_labels = target_labels.clone();
                            }
                        }
                        // Taze olsa bile HTF sayılarını DB'den güncelle — Tab 7 boş kalmasın
                        if interval == "1m" {
                            if let Ok(conn) = database_writer::open_connection(&config.db_path) {
                                if let Ok(mut st) = app_state.lock() {
                                    update_htf_candle_counts(&conn, &mut st.htf_candle_counts, symbol, market);
                                }
                            }
                        }
                        std::thread::sleep(Duration::from_millis(150));
                        continue;
                    };

                    // gap_initial: ilk ölçülen boşluk (döngüler arası korunur)
                    let gap_initial = {
                        let current_gap = (now_init - fetch_start) / interval_ms.max(1);
                        let initial = gap_initial_map.entry(map_key.clone()).or_insert(current_gap);
                        if current_gap > *initial { *initial = current_gap; }
                        *initial
                    };
                    if let Some(lbl) = target_labels.get_mut(target_idx) {
                        lbl.3 = "⬇ İndiriliyor".to_string();
                        lbl.4 = gap_initial;
                        lbl.5 = 0;
                    }

                    let mut target_inserted = 0usize;
                    let mut target_skipped  = 0usize;
                    let mut batch_no        = 0u32;
                    let mut had_error       = false;
                    let mut last_candles: Vec<memos_trading_core::types::Candle> = Vec::new();

                    // ── Çok-turlu indirme döngüsü: fetch_start Binance yanıtından ilerler ─
                    'batch: loop {
                        if stop_signal.load(Ordering::Relaxed) { break; }

                        let now_ms = chrono::Utc::now().timestamp_millis();

                        // fetch_start DB sorgusu YOK — önceki batch'in son mum ts'inden gelir
                        let remaining = (now_ms - fetch_start) / interval_ms;
                        if remaining <= 0 { break 'batch; }

                        batch_no += 1;

                        // gap_initial_candles: ilk ölçümü koru
                        let gap_initial_candles = Some(
                            *gap_initial_map.get(&map_key).unwrap_or(&gap_initial)
                        );

                        // DownloadProgress güncelle (her batch başında)
                        if let Ok(mut st) = app_state.try_lock() {
                            st.download_progress = Some(DownloadProgress {
                                current_idx:         target_idx + 1,
                                total_targets:       targets.len(),
                                symbol:              symbol.clone(),
                                market:              market.clone(),
                                interval:            interval.clone(),
                                gap_start_ms:        Some(fetch_start),
                                gap_end_ms:          now_ms,
                                gap_interval_ms:     interval_ms,
                                gap_initial_candles,
                                dl_limit:            config.download_candle_limit,
                                inserted_session:    total_inserted,
                                derived_session:     total_derived,
                                batch_no,
                                session_start_ms,
                                target_labels:       target_labels.clone(),
                            });
                        }

                        // Binance URL — her batch'te fetch_start kullanılır
                        let url = if market == "futures" {
                            format!(
                                "https://fapi.binance.com/fapi/v1/klines?symbol={}&interval={}&limit={}&startTime={}",
                                symbol, interval, config.download_candle_limit, fetch_start
                            )
                        } else {
                            format!(
                                "https://api.binance.com/api/v3/klines?symbol={}&interval={}&limit={}&startTime={}",
                                symbol, interval, config.download_candle_limit, fetch_start
                            )
                        };

                        // Async HTTP → sync blok (30s timeout — sonsuz beklemeyi önler)
                        let candles_result: Result<Vec<memos_trading_core::types::Candle>, String> =
                            rt.block_on(async {
                                let client = build_http_client()?;
                                let resp = client.get(&url)
                                    .send()
                                    .await
                                    .map_err(|e| format!("HTTP: {e}"))?
                                    .json::<Vec<Vec<serde_json::Value>>>()
                                    .await
                                    .map_err(|e| format!("JSON: {e}"))?;

                                let mut candles = Vec::with_capacity(resp.len());
                                for k in resp {
                                    let ts_ms = k.get(0).and_then(|v| v.as_i64()).unwrap_or(0);
                                    let open  = k.get(1).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                                    let high  = k.get(2).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                                    let low   = k.get(3).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                                    let close = k.get(4).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                                    let volume= k.get(7).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                                    if let Some(ts) = chrono::DateTime::<chrono::Utc>::from_timestamp_secs(ts_ms / 1000) {
                                        candles.push(memos_trading_core::types::Candle {
                                            timestamp: ts,
                                            open, high, low, close, volume,
                                            symbol: symbol.clone(),
                                            interval: interval.clone(),
                                        });
                                    }
                                }
                                Ok(candles)
                            });

                        match candles_result {
                            Ok(candles) if !candles.is_empty() => {
                                // DB'ye yaz
                                if let Ok(conn) = database_writer::open_connection(&config.db_path) {
                                    match database_writer::save_candles_bulk(&conn, exchange, market, &candles) {
                                        Ok((ins, skip)) => {
                                            total_inserted  += ins;
                                            total_skipped   += skip;
                                            target_inserted += ins;
                                            target_skipped  += skip;
                                            // Batch sonrası label: kaçıncı parti ve toplam eklenen
                                            if let Some(t) = target_labels.get_mut(target_idx) {
                                                t.3 = format!("⬇ B{} +{}mum", batch_no, target_inserted);
                                                t.5 = target_inserted;
                                            }
                                            if let Ok(mut st) = app_state.try_lock() {
                                                if let Some(ref mut dp) = st.download_progress {
                                                    dp.inserted_session = total_inserted;
                                                    dp.target_labels    = target_labels.clone();
                                                }
                                            }
                                            if ins > 0 {
                                                if let Ok(mut st) = app_state.try_lock() {
                                                    st.download_count += ins as u64;
                                                    st.push_log(format!(
                                                        "⬇ {}/{}/{} B{} → +{} mum ({} atl)",
                                                        market, symbol, interval, batch_no, ins, skip
                                                    ));
                                                }
                                            }
                                            // ── 1m'den üst zaman dilimleri türet ────────────
                                            if interval == "1m" {
                                                let derived = aggregate_from_1m(&conn, exchange, market, symbol, &candles);
                                                if derived > 0 {
                                                    total_derived += derived;
                                                    if let Ok(mut st) = app_state.try_lock() {
                                                        st.push_log(format!(
                                                            "🕯 {}/{} → 1m'den 5m/15m/30m/1h/4h/1d türetildi: +{} mum",
                                                            market, symbol, derived
                                                        ));
                                                        // HTF sayılarını DB'den güncelle
                                                        update_htf_candle_counts(&conn, &mut st.htf_candle_counts, symbol, market);
                                                }
                                            }
                                        } else {
                                            // Stale refresh (1h vb.) → candles_binance_* yanı sıra
                                            // ana `candles` tablosuna da yaz; scanner buradan okur.
                                            let bucket_ms = if interval == "1m" { 0i64 }
                                                else { interval_to_ms(interval.as_str()) };
                                            if bucket_ms > 0 {
                                                // Sadece mevcut max timestamp'den yeni olanları yaz
                                                let max_ts: i64 = conn.query_row(
                                                    "SELECT COALESCE(MAX(timestamp), 0) FROM candles \
                                                     WHERE exchange=?1 AND market=?2 AND symbol=?3 AND interval=?4",
                                                    rusqlite::params![exchange, market, symbol, interval],
                                                    |row| row.get(0),
                                                ).unwrap_or(0i64);
                                                let mut written = 0usize;
                                                for c in &candles {
                                                    let ts_ms = c.timestamp.timestamp_millis();
                                                    let aligned = (ts_ms / bucket_ms) * bucket_ms;
                                                    if aligned <= max_ts { continue; }
                                                    let r = conn.execute(
                                                        "INSERT OR REPLACE INTO candles \
                                                         (exchange, market, symbol, interval, timestamp, open, high, low, close, volume) \
                                                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                                                        rusqlite::params![
                                                            exchange, market, symbol, interval,
                                                            aligned, c.open, c.high, c.low, c.close, c.volume
                                                        ],
                                                    );
                                                    if r.map(|n| n > 0).unwrap_or(false) { written += 1; }
                                                }
                                                if written > 0 {
                                                    if let Ok(mut st) = app_state.lock() {
                                                        st.push_log(format!(
                                                            "📥 {}/{}/{} → candles tablosuna +{} mum eklendi",
                                                            market, symbol, interval, written
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                            // fetch_start'ı Binance yanıtındaki son mum'dan ilerlet
                                            // (DB sorgusu YOK — tablo bağımsız, her zaman doğru)
                                            if let Some(last) = candles.last() {
                                                fetch_start = last.timestamp.timestamp_millis() + interval_ms;
                                            }
                                            last_candles = candles;
                                        }
                                        Err(e) => {
                                            errors    += 1;
                                            had_error  = true;
                                            if let Ok(mut st) = app_state.lock() {
                                                st.push_log(format!("⚠️  DB yaz ({}/{}): {}", symbol, interval, e));
                                            }
                                            break 'batch;
                                        }
                                    }
                                }
                            }
                            Ok(_) => {
                                // Boş yanıt: Binance'te bu aralıkta veri yok → gap atla
                                // Bir batch boyutu kadar ilerleyerek devam et
                                fetch_start += dl_limit * interval_ms;
                                let remaining_after = (chrono::Utc::now().timestamp_millis() - fetch_start) / interval_ms;
                                if remaining_after <= 0 { break 'batch; }
                            }
                            Err(e) => {
                                errors    += 1;
                                had_error  = true;
                                if let Ok(mut st) = app_state.lock() {
                                    st.push_log(format!("⚠️  İndirme ({}/{}): {}", symbol, interval, e));
                                }
                                break 'batch;
                            }
                        }

                        // Gap kapandıysa (fetch_start artık yeterince yakın) çık
                        let now_check = chrono::Utc::now().timestamp_millis();
                        if (now_check - fetch_start) / interval_ms <= dl_limit {
                            break 'batch;
                        }

                        // Batch'ler arası rate-limit koruması
                        std::thread::sleep(Duration::from_millis(300));
                    } // 'batch loop sonu
                    gap_initial_map.remove(&map_key); // gap kapandı veya hata, temizle

                    // ── Hedef tamamlandı: final label ────────────────────────────────────
                    let final_lbl = if had_error && target_inserted == 0 {
                        "✗ Hata".to_string()
                    } else if target_inserted > 0 {
                        format!("✓ +{} mum ({} parti)", target_inserted, batch_no)
                    } else {
                        format!("✓ taze ({}atl)", target_skipped)
                    };
                    if let Some(t) = target_labels.get_mut(target_idx) {
                        t.3 = final_lbl;
                        t.5 = target_inserted;
                    }
                    if let Ok(mut st) = app_state.try_lock() {
                        if let Some(ref mut dp) = st.download_progress {
                            dp.inserted_session = total_inserted;
                            dp.target_labels    = target_labels.clone();
                        }
                    }

                    // ── Backtest — tüm batch'ler bittikten sonra TEK KEZ ─────────────────
                    // 4 strateji test edilir, en iyi kompozit skoru veren seçilir.
                    if last_candles.is_empty() {
                        // 0 mum indirildiyse sembol/interval kombinasyonu geçersiz — kandidat listesine ekleme
                        if let Ok(mut st) = app_state.lock() {
                            st.push_log(format!(
                                "⚠️  {}/{}: 0 mum indirildi — skor hesaplanamadı, kandidat listesi güncellenmedi",
                                symbol, interval
                            ));
                        }
                    } else {
                        let (cur_sl, cur_tp) = if let Ok(st) = app_state.lock() {
                            (st.best_sl, st.best_tp)
                        } else { (2.0_f64, 4.0_f64) };
                        if let Some((result, best_strat)) = best_strategy_backtest(
                            &last_candles, &symbol, &interval,
                            cur_sl, cur_tp, config.capital, config.trade_amount,
                        ) {
                            let last_close = last_candles.last().map(|c| c.close).unwrap_or(0.0);
                            let last_ts    = last_candles.last().map(|c| c.timestamp.timestamp()).unwrap_or(0);
                            let updated = SymbolScore {
                                exchange:         exchange.clone(),
                                market:           market.clone(),
                                symbol:           symbol.clone(),
                                interval:         interval.clone(),
                                candle_count:     last_candles.len(),
                                win_rate:         result.win_rate,
                                total_pnl:        result.total_pnl,
                                total_trades:     result.total_trades,
                                score:            0.0,
                                last_price:       last_close,
                                last_candle_ts:   last_ts,
                                best_strategy:    best_strat.clone(),
                                profit_factor:    result.profit_factor,
                                sharpe_ratio:     result.sharpe_ratio,
                                max_drawdown_pct: result.max_drawdown_pct,
                            };
                            if let Ok(mut st) = app_state.lock() {
                                if let Some(ex) = st.symbol_candidates.iter_mut()
                                    .find(|c| c.exchange == updated.exchange && c.market == updated.market
                                           && c.symbol == updated.symbol && c.interval == updated.interval) {
                                    *ex = updated;
                                } else {
                                    st.symbol_candidates.push(updated);
                                }
                                let be_wr = st.best_sl / (st.best_sl + st.best_tp) * 100.0;
                                for c in &mut st.symbol_candidates {
                                    let sym_clone = c.symbol.clone();
                                    c.score = compute_symbol_score(
                                        c.win_rate, c.profit_factor, c.sharpe_ratio,
                                        c.max_drawdown_pct, c.total_trades, c.total_pnl,
                                        c.candle_count, c.last_price, c.last_candle_ts,
                                        &sym_clone, be_wr,
                                    );
                                }
                                st.symbol_candidates.sort_by(|a, b|
                                    b.score.partial_cmp(&a.score)
                                        .unwrap_or(std::cmp::Ordering::Equal));
                                let cur_score = st.symbol_candidates.iter()
                                    .find(|c| c.symbol.as_str() == symbol.as_str())
                                    .map(|c| c.score).unwrap_or(0.0);
                                st.push_log(format!(
                                    "📊 {}/{} strateji={} win={:.1}% pf={:.2} dd={:.1}% skor={:.4}",
                                    symbol, interval, best_strat,
                                    result.win_rate, result.profit_factor, result.max_drawdown_pct,
                                    cur_score,
                                ));
                                // ── Otonom SL/TP Güncelle ─────────────────────────────────────
                                let (new_sl, new_tp) = compute_adaptive_sl_tp(
                                    result.win_rate, result.max_drawdown_pct,
                                );
                                st.best_sl = new_sl;
                                st.best_tp = new_tp;
                                if let Ok(mut lr) = st.live_risk.write() {
                                    lr.per_symbol.insert(symbol.clone(), (new_sl, new_tp));
                                    lr.global_sl   = new_sl;
                                    lr.global_tp   = new_tp;
                                    lr.global_fast = st.best_fast;
                                    lr.global_slow = st.best_slow;
                                }
                                st.push_log(format!(
                                    "⚙ SL/TP ({}/{}): SL={:.1}% TP={:.1}% (rr={:.1}x)",
                                    symbol, interval, new_sl, new_tp, new_tp / new_sl,
                                ));
                                // ── Adaptif Risk Politikası Güncelle ──────────────────────────
                                // Sadece aktif sembol global politikayı güncellemeli.
                                // Secondary semboller SL/TP'yi per_symbol'e zaten yazdı;
                                // global policy'yi ezerlerse birincil sembolün kalibre ettiği
                                // risk parametreleri bozulur.
                                let is_active_sym = symbol.as_str() == st.active_symbol.symbol.as_str()
                                    && market.as_str() == st.active_symbol.market.as_str();
                                if is_active_sym {
                                    let new_policy = compute_adaptive_policy(
                                        config.capital, result.win_rate, result.max_drawdown_pct,
                                    );
                                    st.risk_gate.policy = new_policy;
                                    st.push_log(format!(
                                        "🛡 Risk güncellendi ({}/{}): dd={:.1}% day={:.1}% not=${:.0} conf={:.2}",
                                        symbol, interval,
                                        new_policy.max_drawdown_pct, new_policy.max_daily_loss_pct,
                                        new_policy.max_notional_usd, new_policy.min_model_confidence,
                                    ));
                                }
                                // ── AdaptiveBrain: backtest PnL → reward ──────────────────────
                                {
                                    let strategy_name = st.live_strategy.read().ok()
                                        .map(|s| s.clone()).unwrap_or_else(|| "MA".to_string());
                                    let pnl_pct = result.total_pnl / config.capital * 100.0;
                                    if let Some(brain) = st.controller.adaptive_brain.as_mut() {
                                        let closes:  Vec<f64> = last_candles.iter().map(|c| c.close).collect();
                                        let volumes: Vec<f64> = last_candles.iter().map(|c| c.volume).collect();
                                        let regime = brain.detect_market_regime(&closes, &volumes);
                                        brain.learn_from_trade(&regime, &strategy_name, pnl_pct);
                                        if pnl_pct.abs() > 0.01 {
                                            st.push_log(format!(
                                                "🧠 Brain reward: {}/{} → {}{:.2}% (strateji={})",
                                                symbol, interval,
                                                if pnl_pct >= 0.0 { "+" } else { "" },
                                                pnl_pct, strategy_name,
                                            ));
                                        }
                                    }
                                }
                                // ── Strateji Karşılaştırması: SADECE aktif sembol için ────────
                                let (_, active_mkt, active_sym, active_intv) = st.active_trade_target();
                                if symbol.as_str() == active_sym && interval.as_str() == active_intv
                                    && market.as_str() == active_mkt
                                {
                                    let best_strat = compare_strategies(
                                        &last_candles, config.capital, config.trade_amount, new_sl, new_tp,
                                        &interval,
                                    );
                                    let old_strat = st.live_strategy.read().ok()
                                        .map(|s| s.clone()).unwrap_or_default();
                                    if old_strat != best_strat {
                                        // Açık pozisyon varsa da strateji değiştir — mevcut
                                        // pozisyonların SL/TP'si sabit, sadece YENİ girişler
                                        // yeni stratejiyi kullanır.
                                        if st.strategy_locked_until <= Instant::now() {
                                            let has_open_pos = st.live_positions.read().ok()
                                                .map(|p| !p.is_empty()).unwrap_or(false);
                                            let note = if has_open_pos { " (açık pos var — yeni girişler için)" } else { "" };
                                            st.push_log(format!(
                                                "🦅 Strateji otomatik değişti: {} → {}{}", old_strat, best_strat, note
                                            ));
                                            if let Ok(mut ls) = st.live_strategy.write() {
                                                *ls = best_strat;
                                            }
                                            st.strategy_locked_until = Instant::now() + Duration::from_secs(600);
                                        } else {
                                            let rem = st.strategy_locked_until
                                                .saturating_duration_since(Instant::now()).as_secs();
                                            st.push_log(format!("⏳ Strateji kilidi: {} → {} ({}sn sonra)", old_strat, best_strat, rem));
                                        }
                                    }
                                }
                                save_app_snapshot(&st);
                            }
                        }
                    }

                    // Her target tamamlandığında last_download_at güncelle
                    // → chain monitor uzun download boyunca stale saymaz
                    if let Ok(mut st) = app_state.try_lock() {
                        st.last_download_at = Some(
                            chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
                        );
                    }

                    // Hedefler arası 300ms
                    std::thread::sleep(Duration::from_millis(300));
                }

                // Özet
                let summary = format!(
                    "+{} yeni / {} mevcut / {} hata | {} hedef | {}",
                    total_inserted, total_skipped, errors,
                    targets.len(),
                    chrono::Local::now().format("%H:%M:%S"),
                );
                // WAL checkpoint: indirilen tüm mumları ana DB dosyasına flush et
                // Böylece uygulama kapanınca WAL'daki veri kaybolmaz
                if let Ok(conn) = database_writer::open_connection(&config.db_path) {
                    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
                }

                if let Ok(mut st) = app_state.lock() {
                    st.last_download    = Some(summary.clone());
                    st.last_download_at = Some(chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string());
                    st.download_active  = false;
                    st.download_progress = None;
                    st.init_complete.store(true, Ordering::Relaxed); // ilk indirme bitti
                    st.download_next_at = Instant::now() + Duration::from_secs(download_every_secs);
                    // Pipeline'ın next_run_at'ini ileriye it: startup indirmesi pipeline Download
                    // fazını zaten doyurdu; hemen ikinci indirme başlatılmasın.
                    // Pipeline bir sonraki döngüde every_mins sonra çalışır.
                    {
                        let every_secs = st.pipeline.every_mins * 60;
                        st.pipeline.next_run_at =
                            std::time::Instant::now() + std::time::Duration::from_secs(every_secs);
                    }
                    st.push_log(format!("✅ İndirme tamamlandı: {}", summary));
                    // Yeni candle'lar DB'ye yazıldı → scan + backtest + ML hemen tetikle
                    st.symbol_trigger.store(true, Ordering::Relaxed);
                    st.backtest_trigger.store(true, Ordering::Relaxed);
                    st.ml_trigger.store(true, Ordering::Relaxed);
                    // Screener: Queued → Downloaded; symbol_trigger puanlama yapacak
                    for c in &mut st.screener_candidates {
                        if c.status == ScreenerStatus::Queued {
                            c.status = ScreenerStatus::Downloaded;
                        }
                    }
                }
            }

            // ── Live Price Ticker ────────────────────────────────────────────
            // Aktif interval 1m değilse (örn. 1h): her 60s'de 2 adet 1m mum çek,
            // DB'ye yaz, live_price güncelle. Böylece 1h loop'un 14:00 @ 14:33 sorunu çözülür.
            live_price_elapsed += tick;
            if live_price_elapsed >= live_price_every_secs {
                live_price_elapsed = 0;
                let (lp_exchange, lp_market, lp_symbol, lp_interval, lp_arc) = {
                    if let Ok(st) = app_state.lock() {
                        let (e, m, s, i) = st.active_trade_target();
                        (e, m, s, i, Arc::clone(&st.live_price))
                    } else { continue; }
                };
                // Sadece interval≠1m ise aktif sembol için çalıştır; 1m loop zaten kendi günceller
                if lp_interval != "1m" {
                    let lp_url = if lp_market == "futures" {
                        format!("https://fapi.binance.com/fapi/v1/klines?symbol={}&interval=1m&limit=2", lp_symbol)
                    } else {
                        format!("https://api.binance.com/api/v3/klines?symbol={}&interval=1m&limit=2", lp_symbol)
                    };
                    let fetch_result: Result<Vec<memos_trading_core::types::Candle>, String> =
                        rt.block_on(async {
                            let resp = reqwest::get(&lp_url).await
                                .map_err(|e| format!("LP HTTP: {e}"))?
                                .json::<Vec<Vec<serde_json::Value>>>().await
                                .map_err(|e| format!("LP JSON: {e}"))?;
                            let mut out = Vec::new();
                            for k in resp {
                                let ts_ms = k.get(0).and_then(|v| v.as_i64()).unwrap_or(0);
                                let open  = k.get(1).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0f64);
                                let high  = k.get(2).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0f64);
                                let low   = k.get(3).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0f64);
                                let close = k.get(4).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0f64);
                                let vol   = k.get(7).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0f64);
                                if let Some(ts) = chrono::DateTime::<chrono::Utc>::from_timestamp_secs(ts_ms / 1000) {
                                    out.push(memos_trading_core::types::Candle {
                                        timestamp: ts, open, high, low, close, volume: vol,
                                        symbol: lp_symbol.clone(), interval: "1m".to_string(),
                                    });
                                }
                            }
                            Ok(out)
                        });
                    match fetch_result {
                        Err(e) => {
                            // REST fiyat ticker hatası — log'a yaz; WS arc varsa fallback olarak kullan
                            eprintln!("[live_price] {} REST çekme hatası: {}", lp_symbol, e);
                            // Orchestrator arc'ında veri varsa st.live_price'a kopyala
                            if let Ok(st) = app_state.lock() {
                                if let Some(orch_arc) = st.orchestrator.live_price_for(&lp_symbol) {
                                    if let (Ok(src), Ok(mut dst)) = (orch_arc.read(), lp_arc.write()) {
                                        if src.close > 0.0 { *dst = src.clone(); }
                                    }
                                }
                            }
                        }
                        Ok(candles) => {
                        // DB'ye yaz (INSERT OR REPLACE — yeni mum varsa ekle)
                        if let Ok(conn) = database_writer::open_connection(&config.db_path) {
                            for c in &candles {
                                let ts_ms = c.timestamp.timestamp_millis();
                                let _ = conn.execute(
                                    "INSERT OR REPLACE INTO candles \
                                     (exchange,market,symbol,interval,timestamp,open,high,low,close,volume) \
                                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                                    rusqlite::params![
                                        lp_exchange, lp_market, lp_symbol, "1m",
                                        ts_ms, c.open, c.high, c.low, c.close, c.volume
                                    ],
                                );
                            }
                        }
                        // live_price güncelle: en son mum
                        // change_pct: mevcut close ile önceki close karşılaştır (birikimli)
                        if let Some(last) = candles.last() {
                            if last.close > 0.0 {
                                if let Ok(mut pd) = lp_arc.write() {
                                    let prev_close = if pd.close > 0.0 { pd.close } else { last.open };
                                    pd.symbol     = lp_symbol.clone();
                                    pd.open       = if pd.open > 0.0 { pd.open } else { last.open }; // ilk fiyatı koru
                                    pd.high       = last.high;
                                    pd.low        = last.low;
                                    pd.close      = last.close;
                                    pd.volume     = last.volume;
                                    // change_pct: session başından bu yana değişim (ilk fiyata göre)
                                    pd.change_pct = if pd.open > 0.0 { (last.close - pd.open) / pd.open * 100.0 } else { (last.close - prev_close) / prev_close * 100.0 };
                                    pd.ts = last.timestamp.with_timezone(&chrono::Local).format("%H:%M:%S").to_string();
                                    pd.last_updated_ms = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default().as_millis() as u64;
                                }
                            }
                        }
                        }
                    }
                }

                // ── Orchestrator worker'ları için canlı fiyat güncelle (DIAG K2 fix) ──
                // Açık pozisyonu olan semboller her 10s (yüksek öncelik),
                // diğerleri sadece her 30s trigger'ında (live_price_every_secs=10, ancak 30s'de bir).
                // Bu şekilde Binance weight kullanımı minimum tutulur (açık pos: N×1w/10s).
                // Composite key yerine gerçek sembol adlarını topla → h.symbol ile eşleşsin
                let open_syms: std::collections::HashSet<String> = {
                    if let Ok(st) = app_state.lock() {
                        if let Ok(lp) = st.live_positions.read() {
                            lp.values().map(|v| v.symbol.clone()).collect()
                        } else { std::collections::HashSet::new() }
                    } else { std::collections::HashSet::new() }
                };
                let orch_targets: Vec<(String, String, Arc<std::sync::RwLock<LivePriceData>>)> = {
                    if let Ok(st) = app_state.lock() {
                        let (_, _, active_sym, _) = st.active_trade_target();
                        st.orchestrator.workers.values()
                            .filter(|h| h.symbol != active_sym)
                            // Açık pozisyon varsa her zaman güncelle; yoksa sadece 30s trigger'ı (3. döngü)
                            .filter(|h| open_syms.contains(&h.symbol) || live_price_elapsed == 0)
                            .filter_map(|h| {
                                st.orchestrator.live_price_for(&h.symbol)
                                    .map(|arc| (h.market.clone(), h.symbol.clone(), arc))
                            })
                            .collect()
                    } else { vec![] }
                };
                for (mkt, sym, price_arc) in orch_targets {
                    if stop_signal.load(Ordering::Relaxed) { break; }
                    let orch_url = if mkt == "futures" {
                        format!("https://fapi.binance.com/fapi/v1/klines?symbol={}&interval=1m&limit=2", sym)
                    } else {
                        format!("https://api.binance.com/api/v3/klines?symbol={}&interval=1m&limit=2", sym)
                    };
                    let fetch_res: Result<Vec<memos_trading_core::types::Candle>, String> =
                        rt.block_on(async {
                            let client = build_http_client()?;
                            let resp = client.get(&orch_url).send().await
                                .map_err(|e| format!("OLP HTTP: {e}"))?
                                .json::<Vec<Vec<serde_json::Value>>>().await
                                .map_err(|e| format!("OLP JSON: {e}"))?;
                            let mut out = Vec::new();
                            for k in resp {
                                let ts_ms = k.get(0).and_then(|v| v.as_i64()).unwrap_or(0);
                                let open  = k.get(1).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0f64);
                                let high  = k.get(2).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0f64);
                                let low   = k.get(3).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0f64);
                                let close = k.get(4).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0f64);
                                let vol   = k.get(7).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0f64);
                                if let Some(ts) = chrono::DateTime::<chrono::Utc>::from_timestamp_secs(ts_ms / 1000) {
                                    out.push(memos_trading_core::types::Candle {
                                        timestamp: ts, open, high, low, close, volume: vol,
                                        symbol: sym.clone(), interval: "1m".to_string(),
                                    });
                                }
                            }
                            Ok(out)
                        });
                    if let Ok(candles) = fetch_res {
                        if let Some(last) = candles.last() {
                            if last.close > 0.0 {
                                if let Ok(mut pd) = price_arc.write() {
                                    // change_pct: session başından bu yana değişim (ilk fiyatı koru)
                                    let session_open = if pd.open > 0.0 { pd.open } else { last.open };
                                    pd.symbol     = sym.clone();
                                    pd.open       = session_open;
                                    pd.high       = last.high;
                                    pd.low        = last.low;
                                    pd.close      = last.close;
                                    pd.volume     = last.volume;
                                    pd.change_pct = if session_open > 0.0 {
                                        (last.close - session_open) / session_open * 100.0
                                    } else { 0.0 };
                                    pd.ts = last.timestamp.with_timezone(&chrono::Local)
                                        .format("%H:%M:%S").to_string();
                                }
                            }
                        }
                    }
                    // Rate-limit koruma: worker'lar arası 200ms
                    std::thread::sleep(Duration::from_millis(200));
                }
            }

            std::thread::sleep(Duration::from_secs(tick));
            elapsed += tick;
        }
    });
}

// ─── Otonom Sembol Seçici Worker ─────────────────────────────────────────────
// ─── Kapasite Hesaplayıcı ────────────────────────────────────────────────────
// Sistemin kaldırabileceği eş zamanlı sembol sayısını otomatik belirler.
//
// Kısıtlayıcı faktörler:
//   1. CPU çekirdek sayısı  — her worker ağırlıklı uyur ama işletim sistemi
//      thread scheduling yükü göz önünde bulundurulur.
//   2. İşlem aralığı (interval) — kısa aralık (1m) → dakikada N candle çekimi,
//      API istek bütçesini daha hızlı tüketir.
//   3. Binance REST ağırlık limiti — 1200 req/dk.
//      Her worker: live_price (30s=2 req/dk) + candle fetch (interval başına 1).
//      Toplam: ~(2 + 60/interval_mins) req/dk per worker.
//   4. Pratik tavan — tek exchange hesabında >20 eş zamanlı sembol anlamsız.
//
// Formül:
//   api_budget   = floor(1200 / req_per_worker_per_min)   → API'den gelen tavan
//   cpu_budget   = max(2, cpu_cores × 2)                  → thread sayısı tavanı
//   max_workers  = min(api_budget, cpu_budget, HARD_CAP=20)

fn compute_max_workers(interval: &str) -> usize {
    const HARD_CAP: usize = 20;
    const BINANCE_WEIGHT_PER_MIN: f64 = 1200.0;

    // Mantıksal CPU çekirdek sayısı (std — harici crate gerektirmez)
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);

    // İnterval → dakika cinsinden
    let intv = interval.trim().to_lowercase();
    let interval_mins: f64 = if let Some(n) = intv.strip_suffix('m') {
        n.parse::<f64>().unwrap_or(1.0)
    } else if let Some(n) = intv.strip_suffix('h') {
        n.parse::<f64>().unwrap_or(1.0) * 60.0
    } else if let Some(n) = intv.strip_suffix('d') {
        n.parse::<f64>().unwrap_or(1.0) * 1440.0
    } else {
        1.0
    };

    // Her worker'ın dakika başına Binance API isteği:
    //   live_price: 30s periyot → 2.0 req/dk (ağırlık=1 her biri)
    //   candle fetch: interval başına 1 istek (ağırlık=1)
    let candle_req_per_min = (1.0 / interval_mins).min(1.0); // max 1/dk candle çekimi
    let req_per_worker = 2.0 + candle_req_per_min;

    let api_budget  = (BINANCE_WEIGHT_PER_MIN / req_per_worker).floor() as usize;
    let cpu_budget  = (cpu_cores * 2).max(2); // hyperthread'li sistemlerde 2× çekirdek

    // Üç kısıtın minimuму + sabit tavan
    api_budget.min(cpu_budget).min(HARD_CAP).max(2)
}

// ─── Çok-Sembol Orkestrasyon Eşleştirici ─────────────────────────────────────
// symbol_candidates sıralamasına göre orchestrator'ı günceller:
//   • Sıralamaya girenleri (score>0, candle_count≥50) otomatik spawn eder.
//   • Sıralamadan çıkanları (veya kapasiteyi aşanları) durdurur.
//   • auto_symbol=false ise işlem yapmaz (manuel mod).
// Kilid dışında çağrılmalıdır — içeride kendi kilidi alır/bırakır.

fn sync_orchestrator_workers(app_state: Arc<Mutex<AppState>>) {
    use std::collections::HashSet;

    // Faz 1: Hesapla + kaydet (kısa lock)
    let (capital, start_cfgs) = {
        let mut st = match app_state.lock() {
            Ok(g)  => g,
            Err(_) => return,
        };
        if !st.auto_symbol { return; }

        let capital      = st.equity;
        let max_workers  = st.orchestrator.max_workers;

        // Break-even win rate: SL / (SL + TP)
        // Bu eşiğin altındaki semboller sistematik zarar üretir — trade edilmez.
        let breakeven_wr = st.best_sl / (st.best_sl + st.best_tp) * 100.0;
        // Engellenen semboller orchestrator seviyesinde de filtrelenir — thread bile spawn edilmez
        let blocked: HashSet<String> = st.config.blocked_symbols.iter().cloned().collect();
        // Pinned semboller: filtrelerden geçmese bile her zaman worker'a alınır
        let pinned: HashSet<String> = st.config.pinned_symbols.iter()
            .filter(|s| !blocked.contains(*s))
            .cloned()
            .collect();
        // Gerçek verisi olan, break-even üstünde ve risk metrikleri sağlıklı adaylar
        let scored_top: Vec<SymbolScore> = st.symbol_candidates.iter()
            .filter(|c| {
                !blocked.contains(&c.symbol)
                    && !pinned.contains(&c.symbol) // pinned ayrı iş görür; burada tekrarlama
                    && c.score > 0.0
                    && c.candle_count >= 50
                    && c.total_trades > 5
                    && c.win_rate >= breakeven_wr
                    && (c.profit_factor >= 1.0 || c.profit_factor == 0.0) // 0.0 = henüz hesaplanmadı → geç
                    && c.max_drawdown_pct < 40.0
            })
            .cloned()
            .collect();
        // Pinned sembolleri her zaman başa ekle (puanlamaya/filtrelere bakmaz)
        let pinned_entries: Vec<SymbolScore> = st.config.pinned_symbols.iter()
            .filter(|s| !blocked.contains(*s))
            .map(|sym| {
                // Mevcut aday kaydı varsa onu kullan (skor bilgisi korunur)
                st.symbol_candidates.iter()
                    .find(|c| &c.symbol == sym)
                    .cloned()
                    .unwrap_or_else(|| SymbolScore {
                        exchange:         st.config.exchange.clone(),
                        market:           st.config.market.clone(),
                        symbol:           sym.clone(),
                        interval:         st.config.interval.clone(),
                        candle_count:     0,
                        win_rate:         0.0,
                        total_pnl:        0.0,
                        total_trades:     0,
                        score:            0.0,
                        last_price:       0.0,
                        last_candle_ts:   0,
                        best_strategy:    String::new(),
                        profit_factor:    0.0,
                        sharpe_ratio:     0.0,
                        max_drawdown_pct: 0.0,
                    })
            })
            .collect();
        let mut top_n: Vec<SymbolScore> = pinned_entries;
        for c in scored_top.into_iter() {
            if top_n.len() >= max_workers { break; }
            top_n.push(c);
        }
        top_n.truncate(max_workers);

        let top_syms: HashSet<String> = top_n.iter().map(|c| c.symbol.clone()).collect();
        let running:  HashSet<String> = st.orchestrator.active_symbols().into_iter().collect();

        // Birincil (primary) sembol: restart trigger tarafından yönetilir.
        // sync_orchestrator_workers buraya dokunmaz — aksi hâlde primary loop ile
        // orchestrator loop arasında yarış oluşur ve N×spawn gerçekleşir.
        let primary_sym = st.active_symbol.symbol.clone();

        // Durdurulacaklar: çalışıyor ama top-N'de yok (primary hariç)
        let mut stop_logs: Vec<String> = vec![];
        for sym in running.difference(&top_syms) {
            if sym == &primary_sym { continue; } // primary loop restart trigger yönetir
            // Worker market'ini durdurmadan önce al → composite key oluştur
            let worker_market_str = st.orchestrator.workers.get(sym.as_str())
                .map(|h| h.market.clone())
                .unwrap_or_else(|| "spot".to_string());
            if st.orchestrator.stop_symbol(sym) {
                let pos_key = {
                    let m = match worker_market_str.to_lowercase().as_str() {
                        "futures" => memos_trading_core::types::Market::Futures,
                        "coinm"   => memos_trading_core::types::Market::Coinm,
                        _         => memos_trading_core::types::Market::Spot,
                    };
                    live_pos_key(sym, &m)
                };
                let pnl_str = if let Ok(mut lm) = st.live_positions.write() {
                    lm.remove(&pos_key)
                        .map(|p| {
                            let pnl = pos_pnl(p.current_price, p.entry_price, p.qty, p.is_long);
                            format!("{:+.3} USDT", pnl)
                        })
                        .unwrap_or_else(|| "pozisyon yok".to_string())
                } else { "?".to_string() };
                stop_logs.push(format!(
                    "⛔ Çok-sembol: {} sıralamadan düştü, durduruldu ({})", sym, pnl_str
                ));
            }
        }
        for msg in stop_logs { st.push_log(msg); }

        // Başlatılacaklar: top-N'de var ama henüz çalışmıyor (primary hariç)
        let mut start_cfgs: Vec<(SymbolScore, Arc<AtomicBool>, Arc<AtomicBool>)> = vec![];
        for cand in top_n.iter().filter(|c| !running.contains(&c.symbol) && c.symbol != primary_sym) {
            if let Some((stop, pause, _)) = st.orchestrator
                .register(&cand.symbol, &cand.market, &cand.interval)
            {
                st.push_log(format!(
                    "🚀 Çok-sembol: {} ({}/{}) eklendi | skor={:.4}",
                    cand.symbol, cand.market, cand.interval, cand.score
                ));
                start_cfgs.push((cand.clone(), stop, pause));
            }
        }

        (capital, start_cfgs)
    }; // kilit bırakıldı

    // Faz 2: Thread'leri başlat (kilit dışında)
    for (cand, stop_sig, pause_sig) in start_cfgs {
        let exch = if cand.exchange.is_empty() {
            "binance".to_string()
        } else {
            cand.exchange.clone()
        };
        real_robotic_loop(
            Arc::clone(&app_state),
            stop_sig,
            pause_sig,
            capital,
            Some((exch, cand.market.clone(), cand.symbol.clone(), cand.interval.clone())),
        );
    }
}

// ─── MTF Fiyat Monitörü ──────────────────────────────────────────────────────
// Her 30 saniyede bir, açık pozisyonu olan sembollerin en güncel 1m kapanış fiyatını
// DB'den okur ve ilgili live_price arc'ını günceller.
// Amaç: 1h stratejisinde SL/TP kontrolü mum kapanışına değil, 1m veriye dayanır
// → mum-kaydı (candle slippage) azalır, trailing SL daha erken tetiklenir.
fn run_mtf_price_monitor_worker(
    app_state:   Arc<Mutex<AppState>>,
    stop_signal: Arc<AtomicBool>,
    db_path:     String,
) {
    std::thread::spawn(move || {
        use std::sync::atomic::Ordering;
        use rusqlite::Connection;

        // İlk çalışmayı geciktirme — açık pozisyon varsa hemen kontrol et
        let has_positions = app_state.lock().ok()
            .and_then(|st| st.live_positions.read().ok().map(|p| !p.is_empty()))
            .unwrap_or(false);
        if !has_positions {
            std::thread::sleep(Duration::from_secs(15));
        }

        loop {
            if stop_signal.load(Ordering::Relaxed) { break; }

            // Açık pozisyon sembollerini topla — (composite_key, real_sym, market_str)
            let open_symbols: Vec<(String, String, String)> = {
                if let Ok(st) = app_state.lock() {
                    st.live_positions.read().ok()
                        .map(|p| p.iter().map(|(k, v)| {
                            let mkt = format!("{:?}", v.market).to_lowercase();
                            (k.clone(), v.symbol.clone(), mkt)
                        }).collect())
                        .unwrap_or_default()
                } else { vec![] }
            };

            if !open_symbols.is_empty() {
                if let Ok(conn) = Connection::open(&db_path) {
                    for (composite_key, sym, pos_market) in &open_symbols {
                        // Composite key'den gelen market bilgisini kullan —
                        // fallback spot sorgusunu önler ve yanlış fiyat karışmasını engeller
                        let worker_market: Option<String> = app_state.lock().ok()
                            .and_then(|st| st.orchestrator.workers.get(sym.as_str())
                                .map(|h| h.market.clone()))
                            .or_else(|| Some(pos_market.clone()));

                        // Spot veya futures — pozisyonun market'ine ait son 1m kapanışı + timestamp al
                        let price_1m: Option<(f64, i64)> = if let Some(ref mkt) = worker_market {
                            conn.query_row(
                                "SELECT close, timestamp FROM candles \
                                 WHERE symbol=?1 AND market=?2 AND interval='1m' \
                                 ORDER BY timestamp DESC LIMIT 1",
                                rusqlite::params![sym, mkt],
                                |row| Ok((row.get::<_, f64>(0)?, row.get::<_, i64>(1)?)),
                            ).ok()
                        } else {
                            // Worker bulunamadıysa spot'u dene
                            conn.query_row(
                                "SELECT close, timestamp FROM candles \
                                 WHERE symbol=?1 AND market='spot' AND interval='1m' \
                                 ORDER BY timestamp DESC LIMIT 1",
                                rusqlite::params![sym],
                                |row| Ok((row.get::<_, f64>(0)?, row.get::<_, i64>(1)?)),
                            ).ok()
                        };

                        if let Some((price, candle_ts)) = price_1m {
                            if price <= 0.0 { continue; }

                            // DB mumu 30 dakikadan eskiyse kullanma — yanlış fiyat yazılmasını önler.
                            // Orphan semboller için 1m mumu günlerce güncellenmeyebilir.
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as i64;
                            let candle_age_secs = (now_ms - candle_ts) / 1000;
                            let db_price_usable = candle_age_secs < 1800; // 30 dakika

                            // WS fiyatını DB fiyatıyla override etme — WS daha taze.
                            // Orchestrator worker'ının live_price'ında mevcut WS fiyatı varsa koru;
                            // yoksa (0.0 veya sembol uyuşmuyorsa) DB fiyatıyla doldur.
                            let ws_price_fresh: Option<f64> = app_state.lock().ok()
                                .and_then(|st| st.orchestrator.workers.get(sym.as_str())
                                    .and_then(|h| h.live_price.read().ok()
                                        .filter(|pd| pd.close > 0.0 && pd.symbol == *sym)
                                        .map(|pd| pd.close)));

                            // WS taze fiyat yoksa ve DB mumı tazeyse worker live_price'ına yaz
                            if ws_price_fresh.is_none() && db_price_usable {
                                let updated = app_state.lock().ok()
                                    .and_then(|st| st.orchestrator.workers.get(sym.as_str())
                                        .and_then(|h| h.live_price.write().ok()
                                            .map(|mut pd| { pd.close = price; pd.symbol = sym.clone(); true })))
                                    .unwrap_or(false);
                                if !updated {
                                    if let Ok(st) = app_state.lock() {
                                        if let Ok(mut pd) = st.live_price.write() {
                                            if pd.symbol == *sym { pd.close = price; }
                                        }
                                    }
                                }
                            }

                            // live_positions.current_price → WS fiyatı öncelikli,
                            // DB mumı tazeyse fallback, aksi hâlde güncelleme.
                            let effective_price = ws_price_fresh
                                .or_else(|| if db_price_usable { Some(price) } else { None });
                            if let Some(ep) = effective_price {
                                if let Ok(st) = app_state.lock() {
                                    if let Ok(mut lm) = st.live_positions.write() {
                                        if let Some(pos) = lm.get_mut(composite_key.as_str()) {
                                            pos.current_price = ep;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ── MTF Canlı Sinyal Kontrolü ──────────────────────────────────────────
            // Her 30s döngüsünde MTF fırsatlarındaki stratejileri canlı candle'larla
            // yeniden çalıştır. Buy/Sell sinyali çıkarsa live_signal güncelle + log bas.
            {
                use memos_trading_core::robot::robotic_loop::{make_strategy_pub, load_htf_candles_from_db};
                use memos_trading_core::types::{Signal, StrategyParams};

                let opportunities: Vec<MtfOpportunity> = app_state.lock().ok()
                    .map(|st| st.mtf_opportunities.clone())
                    .unwrap_or_default();

                if !opportunities.is_empty() {
                    if let Ok(conn) = rusqlite::Connection::open(&db_path) {
                        let params = StrategyParams::default();
                        let mut updated = opportunities.clone();
                        let mut new_alerts: Vec<String> = Vec::new();

                        for opp in &mut updated {
                            // market: candles tablosundan bul
                            let market_str: String = conn.query_row(
                                "SELECT market FROM candles WHERE symbol=?1 AND interval=?2 LIMIT 1",
                                rusqlite::params![opp.symbol, opp.interval],
                                |r| r.get::<_, String>(0),
                            ).unwrap_or_else(|_| "spot".to_string());

                            // Son 100 candle yükle
                            let candles = load_htf_candles_from_db(
                                &db_path, &opp.symbol, &market_str,
                                &opp.interval, 100
                            ).unwrap_or_default();

                            if candles.len() < 20 { continue; }

                            let strategy = make_strategy_pub(&opp.strategy);
                            let signal = strategy.generate_signal(&candles, &params, None, None)
                                .unwrap_or(Signal::Hold);

                            let sig_str = match signal {
                                Signal::Buy  => "BUY",
                                Signal::Sell => "SELL",
                                Signal::Hold => "-",
                            };

                            let price = candles.last().map(|c| c.close).unwrap_or(0.0);
                            let now_hms = chrono::Local::now().format("%H:%M:%S").to_string();

                            // Yeni sinyal tetiklendiyse (önceden Hold/farklıysa) alert oluştur
                            let is_new_signal = sig_str != "-" && opp.live_signal != sig_str;
                            if is_new_signal {
                                new_alerts.push(format!(
                                    "🚨 MTF SİNYAL: {}/{} {} {} fiyat={:.4} skor={:.3} wr={:.0}%",
                                    opp.symbol, opp.interval, sig_str, opp.strategy,
                                    price, opp.score, opp.win_rate * 100.0
                                ));
                            }

                            opp.live_signal  = sig_str.to_string();
                            opp.signal_price = price;
                            if sig_str != "-" {
                                opp.signal_at = Some(now_hms);
                            }
                        }

                        // AppState'e yaz + yüksek-skor sinyal inject
                        if let Ok(mut st) = app_state.try_lock() {
                            // Mevcut live_signal'ları koru (tarayıcı sıfırlamasın)
                            for u in &updated {
                                if let Some(ex) = st.mtf_opportunities.iter_mut()
                                    .find(|o| o.symbol == u.symbol && o.interval == u.interval)
                                {
                                    ex.live_signal  = u.live_signal.clone();
                                    ex.signal_price = u.signal_price;
                                    ex.signal_at    = u.signal_at.clone();
                                }
                            }
                            // Alert logları
                            for alert in &new_alerts {
                                st.push_log(alert.clone());
                            }
                            // ── Yüksek-güven MTF sinyal köprüsü ────────────────────
                            // Aktif sembol/interval için skor≥0.80 ve wr≥0.75 olan
                            // YENİ bir sinyal gelince ml_signal'ı override et.
                            // ML worker'ı HOLD döndürdüğünde bu sinyal devreye girer.
                            const MTF_INJECT_MIN_SCORE: f64 = 0.80;
                            const MTF_INJECT_MIN_WR:    f64 = 0.75;
                            let (_, act_mkt, act_sym, act_intv) = st.active_trade_target();
                            for u in &updated {
                                if u.live_signal == "BUY" || u.live_signal == "SELL" {
                                    if u.symbol == act_sym && u.interval == act_intv
                                        && u.score >= MTF_INJECT_MIN_SCORE
                                        && u.win_rate >= MTF_INJECT_MIN_WR
                                        && st.ml_signal == "HOLD"
                                    {
                                        st.mtf_signal_inject = Some((
                                            u.symbol.clone(), u.interval.clone(),
                                            u.live_signal.clone(), u.score, u.win_rate,
                                        ));
                                        st.ml_signal = u.live_signal.clone();
                                        st.push_log(format!(
                                            "🚀 MTF AUTO: {}/{} {} {} skor={:.3} wr={:.0}% → ml_signal override",
                                            u.symbol, u.interval, u.live_signal, u.strategy,
                                            u.score, u.win_rate * 100.0
                                        ));
                                    }
                                }
                            }
                            let _ = act_mkt; // suppress unused
                        }
                    }
                }
            }

            std::thread::sleep(Duration::from_secs(30));
        }
    });
}

// ─── MTF Fırsat Tarayıcısı Worker ────────────────────────────────────────────
// Her 90 saniyede bir, aktif + aday semboller için birden fazla interval üzerinde
// strateji skorlarını hesaplar. Yüksek kompozit skora sahip sinyaller
// AppState.mtf_opportunities listesine yazılır; TUI Dashboard panelinde gösterilir.
//
// Tarama mantığı:
//   1. Tüm semboller = {active_symbol} ∪ {symbol_candidates} ∪ {screener_candidates}
//   2. Her sembol için intervals = ["1m", "5m", "15m", "1h", "4h"]
//   3. DB'den son 200 candle yükle → rank_strategies_for_interval ile top-3 strateji al
//   4. Composite score > 0.15 olan (symbol, interval, strategy) üçlüleri kaydet
//   5. Sonuçlar score'a göre sıralı, son 30 kayıt tutulur
fn run_mtf_opportunity_scanner(
    app_state:   Arc<Mutex<AppState>>,
    stop_signal: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        use std::sync::atomic::Ordering;
        use memos_trading_core::robot::robotic_loop::load_htf_candles_from_db;

        const SCAN_INTERVALS: &[&str] = &["1m", "5m", "15m", "1h", "4h"];
        const MIN_SCORE:       f64    = 0.15;
        const MAX_OPPS:        usize  = 30;
        const SCAN_SECS:       u64    = 90;

        // Trigger Arc'ını al — ilk lock denemesi başarısız olursa kısa süre tekrar dene
        let scan_trigger = {
            let mut trigger_arc = None;
            for _ in 0..10 {
                match app_state.try_lock() {
                    Ok(st) => { trigger_arc = Some(Arc::clone(&st.mtf_scan_trigger)); break; }
                    Err(_) => std::thread::sleep(Duration::from_millis(200)),
                }
            }
            trigger_arc.unwrap_or_else(|| Arc::new(AtomicBool::new(false)))
        };

        // Thread başladı — hemen log yaz
        if let Ok(mut st) = app_state.lock() {
            st.push_log("🔭 MTF scanner thread başladı (30s bekleniyor veya [u] ile tetikle)".to_string());
        }

        // İlk taramayı geciktir — ama trigger gelirse hemen başla
        let mut waited = 0u64;
        while waited < 30 && !stop_signal.load(Ordering::Relaxed) {
            if scan_trigger.load(Ordering::Relaxed) { break; }
            std::thread::sleep(Duration::from_secs(1));
            waited += 1;
        }

        loop {
            if stop_signal.load(Ordering::Relaxed) { break; }
            scan_trigger.store(false, Ordering::Relaxed);

            // Taranacak sembolleri ve ayarları topla — try_lock + retry ile blokajı önle
            let config_result: Option<(String, String, StrategyParams)> = {
                let mut result = None;
                for _ in 0..20 {
                    match app_state.try_lock() {
                        Ok(st) => {
                            let op = &st.config.optimized_params;
                            let sp = StrategyParams {
                                period:        Some(op.rsi_period.max(1)),
                                overbought:    Some(op.rsi_ob),
                                oversold:      Some(op.rsi_os),
                                fast:          Some(op.ma_fast.max(1)),
                                slow:          Some(op.ma_slow.max(1)),
                                fast_period:   Some(op.macd_fast.max(1)),
                                slow_period:   Some(op.macd_slow.max(1)),
                                signal_period: Some(op.macd_signal.max(1)),
                                std_dev:       Some(op.bb_std_dev),
                                bb_period:     Some(op.bb_period.max(1)),
                            };
                            result = Some((st.config.db_path.clone(), st.config.market.clone(), sp));
                            break;
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(100)),
                    }
                }
                result
            };
            let (db_path, fallback_market, strategy_params) = match config_result {
                Some(v) => v,
                None => {
                    if let Ok(mut st) = app_state.lock() {
                        st.push_log("🔭 MTF tarama: config lock alınamadı, 90s bekleniyor".to_string());
                    }
                    std::thread::sleep(Duration::from_secs(SCAN_SECS));
                    continue;
                }
            };

            // (symbol, market, score) — SymbolScore puanına göre sıralı, en fazla 50 sembol
            // try_lock + retry: render loop veya download worker mutex'i tutuyorsa bloke olmaz
            let symbols: Vec<(String, String)> = {
                let mut result = vec![];
                for _ in 0..20 {
                    match app_state.try_lock() {
                        Ok(st) => {
                            // Önce aktif sembolü ekle
                            let mut ordered: Vec<(String, String, f64)> = Vec::new();
                            if !st.active_symbol.symbol.is_empty() {
                                ordered.push((
                                    st.active_symbol.symbol.clone(),
                                    st.active_symbol.market.clone(),
                                    f64::MAX, // aktif sembol her zaman başta
                                ));
                            }
                            // Pinned semboller: aktif sembolden hemen sonra, skor/filtre baypas
                            for sym in &st.config.pinned_symbols {
                                if !sym.is_empty()
                                    && !ordered.iter().any(|(s, _, _)| s == sym)
                                {
                                    let mkt = st.symbol_candidates.iter()
                                        .find(|c| &c.symbol == sym)
                                        .map(|c| c.market.clone())
                                        .unwrap_or_else(|| fallback_market.clone());
                                    // f64::MAX - 1.0: aktif sembolden sonra, skorlu adaylardan önce
                                    ordered.push((sym.clone(), mkt, f64::MAX - 1.0));
                                }
                            }
                            // Puanlı adaylar — skora göre eklenir
                            for c in &st.symbol_candidates {
                                if !c.symbol.is_empty()
                                    && !ordered.iter().any(|(s, _, _)| s == &c.symbol)
                                {
                                    ordered.push((c.symbol.clone(), c.market.clone(), c.score));
                                }
                            }
                            // Screener adayları — puan bilinmiyor, sona ekle
                            for c in &st.screener_candidates {
                                if !c.symbol.is_empty()
                                    && !ordered.iter().any(|(s, _, _)| s == &c.symbol)
                                {
                                    ordered.push((c.symbol.clone(), fallback_market.clone(), 0.0));
                                }
                            }
                            // Skora göre sırala (büyükten küçüğe), ilk 50 al
                            ordered.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
                            ordered.truncate(50);
                            result = ordered.into_iter().map(|(s, m, _)| (s, m)).collect();
                            break;
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(100)),
                    }
                }
                result
            };

            if symbols.is_empty() {
                // try_lock + retry: render loop mutex'ini beklemeden log yaz
                for _ in 0..20 {
                    if let Ok(mut st) = app_state.try_lock() {
                        st.push_log("🔭 MTF tarama: aday sembol yok, bekleniyor...".to_string());
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                let mut w = 0u64;
                while w < SCAN_SECS && !stop_signal.load(Ordering::Relaxed) {
                    if scan_trigger.load(Ordering::Relaxed) { break; }
                    std::thread::sleep(Duration::from_secs(1));
                    w += 1;
                }
                continue;
            }

            let mut found: Vec<MtfOpportunity> = Vec::new();
            let now_str = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
            let total_syms = symbols.len();

            // Tarama başladı logu — try_lock + retry
            for _ in 0..20 {
                if let Ok(mut st) = app_state.try_lock() {
                    st.push_log(format!(
                        "🔭 MTF tarama başladı: {} sembol × {} interval (min_score={})",
                        total_syms, SCAN_INTERVALS.len(), MIN_SCORE
                    ));
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }

            let mut total_candle_miss = 0usize;
            let mut total_scored = 0usize;

            for (sym_idx, (sym, sym_market)) in symbols.iter().enumerate() {
                if stop_signal.load(Ordering::Relaxed) { break; }

                // Progress: her 5 sembolde AppState'e yaz (UI hemen güncellenir)
                if sym_idx % 5 == 0 {
                    for _ in 0..5 {
                        if let Ok(mut st) = app_state.try_lock() {
                            let mut partial = found.clone();
                            partial.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
                            partial.truncate(MAX_OPPS);
                            let progress_str = format!("{} ({}/{})", now_str, sym_idx, total_syms);
                            st.mtf_opportunities = partial;
                            st.mtf_last_scan = Some(progress_str);
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }

                // Her 10 sembolde log yaz
                if sym_idx > 0 && sym_idx % 10 == 0 {
                    for _ in 0..5 {
                        if let Ok(mut st) = app_state.try_lock() {
                            st.push_log(format!(
                                "🔭 MTF taranıyor {}/{} — {} fırsat bulundu şu ana kadar",
                                sym_idx, total_syms, found.len()
                            ));
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }

                for &intv in SCAN_INTERVALS {
                    if stop_signal.load(Ordering::Relaxed) { break; }

                    // DB'den candle'ları yükle — her sembolün kendi market'i kullanılır
                    let candles = match load_htf_candles_from_db(
                        &db_path, sym, sym_market, intv, 250
                    ) {
                        Ok(c) if c.len() >= 20 => c,
                        Ok(c) => {
                            total_candle_miss += 1;
                            if c.is_empty() {
                                // çok fazla olabilir, sadece ilk sembolde logla
                                if sym_idx == 0 {
                                    if let Ok(mut st) = app_state.lock() {
                                        st.push_log(format!(
                                            "🔭 MTF candle yok: {}/{} ({}) — DB boş olabilir",
                                            sym, intv, sym_market
                                        ));
                                    }
                                }
                            }
                            continue;
                        }
                        Err(e) => {
                            total_candle_miss += 1;
                            if sym_idx == 0 {
                                if let Ok(mut st) = app_state.lock() {
                                    st.push_log(format!(
                                        "🔭 MTF DB hata: {}/{} — {}",
                                        sym, intv, e
                                    ));
                                }
                            }
                            continue;
                        }
                    };

                    // HTF candles (bir üst interval)
                    let htf_intv = match intv {
                        "1m" | "5m"  => "1h",
                        "15m"| "30m" => "4h",
                        "1h"         => "4h",
                        _            => "1d",
                    };
                    let htf_candles = load_htf_candles_from_db(
                        &db_path, sym, sym_market, htf_intv, 100
                    ).ok();

                    // Strateji sıralama
                    let ranked = rank_strategies_for_interval(
                        &candles,
                        &strategy_params,
                        htf_candles.as_deref(),
                        3,
                        Some(intv),
                    );

                    if let Some((strat_name, score)) = ranked.first() {
                        total_scored += 1;
                        if score.composite < MIN_SCORE { continue; }

                        // Son candledan yön tahmini
                        let direction = {
                            let last = candles.last().map(|c| c.close).unwrap_or(0.0);
                            let prev = candles.get(candles.len().saturating_sub(3))
                                .map(|c| c.close).unwrap_or(last);
                            if last > prev { "LONG" } else if last < prev { "SHORT" } else { "-" }
                        };

                        found.push(MtfOpportunity {
                            symbol:   sym.clone(),
                            interval: intv.to_string(),
                            strategy: strat_name.clone(),
                            score:    score.composite,
                            win_rate: score.win_rate,
                            direction: direction.to_string(),
                            found_at: now_str.clone(),
                            live_signal:  "-".to_string(),
                            signal_price: 0.0,
                            signal_at:    None,
                        });
                    }
                }
            }

            // Skora göre sırala, en iyi MAX_OPPS al
            found.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
            found.truncate(MAX_OPPS);

            let best_score = found.first().map(|o| o.score).unwrap_or(0.0);

            // Nihai sonuçları yaz + tamamlanma logu — try_lock + retry
            {
                let found_len = found.len();
                let found_clone = found.clone();
                let now_str_clone = now_str.clone();
                let log_msg = format!(
                    "🔭 MTF tarama tamamlandı: {} fırsat | {} skor edildi | {} candle eksik | en iyi skor: {:.3}",
                    found_len, total_scored, total_candle_miss, best_score
                );
                for _ in 0..30 {
                    if let Ok(mut st) = app_state.try_lock() {
                        st.push_log(log_msg.clone());
                        st.mtf_opportunities = found_clone;
                        st.mtf_last_scan = Some(now_str_clone);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }

            // 90 saniye bekle ama her saniye trigger kontrol et
            let mut elapsed_wait = 0u64;
            while elapsed_wait < SCAN_SECS && !stop_signal.load(Ordering::Relaxed) {
                if scan_trigger.load(Ordering::Relaxed) { break; }
                std::thread::sleep(Duration::from_secs(1));
                elapsed_wait += 1;
            }
        }
    });
}

// ─── Orphan Pozisyon WS Fiyat Besleyici ──────────────────────────────────────
// Aktif RoboticLoop worker'ı olmayan pozisyon semboller için Binance miniTicker
// Orphan pozisyon WS besleyici + anlık SL/TP tetikleyici.
//
// Her sembol için bağımsız miniTicker WS task'i başlatır.
// Her fiyat tick'inde:
//   1. live_positions[key].current_price güncellenir
//   2. Trailing best_price ve trailing_sl hesaplanır
//   3. static_sl / static_tp / trailing_sl koşulları kontrol edilir
//   4. Tetiklenirse pozisyon live_positions'dan çıkarılır ve live_closed_trades'e eklenir
//
// WS Ping/Pong: split edilmez — ws.send(Pong) ile bağlantı canlı tutulur.
async fn run_orphan_position_ws_feeds(
    app_state:   Arc<Mutex<AppState>>,
    stop_signal: Arc<std::sync::atomic::AtomicBool>,
) {
    use futures_util::{SinkExt, StreamExt};
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;
    use memos_trading_core::types::Market;
    use memos_trading_core::robot::robotic_loop::{ClosedTradeData, is_duplicate_trade};
    use tokio_tungstenite::tungstenite::Message;

    // (composite_key → JoinHandle)
    let mut handles: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();

    loop {
        if stop_signal.load(Ordering::Relaxed) { break; }

        // Orphan sembolleri bul: pozisyon var ama orchestrator worker yok
        let orphans: Vec<(String, String, Market)> = {
            if let Ok(st) = app_state.try_lock() {
                let workers: std::collections::HashSet<String> =
                    st.orchestrator.workers.keys().cloned().collect();
                st.live_positions.read().ok()
                    .map(|p| p.iter()
                        .filter(|(_, v)| !workers.contains(&v.symbol))
                        .map(|(k, v)| (k.clone(), v.symbol.clone(), v.market))
                        .collect())
                    .unwrap_or_default()
            } else { vec![] }
        };

        for (composite_key, sym, market) in &orphans {
            if handles.contains_key(composite_key.as_str()) { continue; }
            let key   = composite_key.clone();
            let sym   = sym.clone();
            let mkt   = *market;
            let state = Arc::clone(&app_state);
            let stop  = Arc::clone(&stop_signal);

            let handle = tokio::spawn(async move {
                // Doğru WS endpoint: Spot, USDT-Futures, CoinM ayrı ayrı
                let base = match mkt {
                    Market::Coinm   => "wss://dstream.binance.com/ws",  // CoinM: dstream
                    Market::Futures => "wss://fstream.binance.com/ws",  // USDT-Futures: fstream
                    _               => "wss://stream.binance.com:9443/ws", // Spot
                };
                // CoinM sembolü için perp suffix ekle (BTCUSD → btcusd_perp)
                let ws_sym = match mkt {
                    Market::Coinm => format!("{}_perp", sym.to_lowercase()),
                    _             => sym.to_lowercase(),
                };
                let url = format!("{}/{}@miniTicker", base, ws_sym);
                let mut backoff = 1u64;
                // Son geçerli fiyat: spike filtresi için referans
                let mut last_valid_price: f64 = 0.0;

                'reconnect: loop {
                    if stop.load(Ordering::Relaxed) { break; }

                    let mut ws = match tokio_tungstenite::connect_async(&url).await {
                        Ok((ws, _)) => { backoff = 1; ws }
                        Err(_) => {
                            // DNS kurtarma: max 15s cap — eski 60s cap 22dk kesintiye yol açıyordu
                            tokio::time::sleep(tokio::time::Duration::from_secs(backoff)).await;
                            backoff = (backoff * 2).min(15);
                            continue 'reconnect;
                        }
                    };

                    while let Some(msg_result) = ws.next().await {
                        if stop.load(Ordering::Relaxed) { break 'reconnect; }
                        let msg = match msg_result { Ok(m) => m, Err(_) => break };

                        match msg {
                            // ── Ping → Pong (bağlantı canlı tutma) ──────────────
                            Message::Ping(data) => {
                                let _ = ws.send(Message::Pong(data)).await;
                            }
                            Message::Close(_) => break,
                            Message::Text(text) => {
                                #[derive(serde::Deserialize)]
                                struct Tick {
                                    #[serde(rename="c")] c: String,
                                }
                                let price = match serde_json::from_str::<Tick>(&text)
                                    .ok()
                                    .and_then(|t| t.c.parse::<f64>().ok())
                                    .filter(|&p| p > 0.0)
                                {
                                    Some(p) => p,
                                    None    => continue,
                                };

                                // Spike filtresi: önceki geçerli fiyattan >15% sapma → sahte tick
                                if last_valid_price > 0.0 {
                                    let change_pct = (price - last_valid_price).abs() / last_valid_price * 100.0;
                                    if change_pct > 15.0 {
                                        continue; // Phantom/spike tick — yoksay
                                    }
                                }
                                last_valid_price = price;

                                // ── Tek write-lock'ta: fiyat güncelle + SL/TP kontrol ──
                                // try_lock kullan — async task içinde blocking lock Tokio'yu dondurur
                                let (close_info, snap_payload): (Option<(ClosedTradeData, String)>, Option<SnapshotPayload>) = {
                                    if let Ok(mut st) = state.try_lock() {
                                        let mut info = None;
                                        if let Ok(mut lm) = st.live_positions.write() {
                                            if let Some(pos) = lm.get_mut(key.as_str()) {
                                                // Fiyat ve trailing güncelle
                                                pos.current_price = price;
                                                if pos.is_long && price > pos.best_price {
                                                    pos.best_price = price;
                                                } else if !pos.is_long && price < pos.best_price {
                                                    pos.best_price = price;
                                                }
                                                if let Some(tpct) = pos.trailing_pct {
                                                    let in_profit = (pos.is_long && price > pos.entry_price)
                                                        || (!pos.is_long && price < pos.entry_price);
                                                    if in_profit {
                                                        let tsl = if pos.is_long {
                                                            pos.best_price * (1.0 - tpct / 100.0)
                                                        } else {
                                                            pos.best_price * (1.0 + tpct / 100.0)
                                                        };
                                                        pos.trailing_sl = Some(tsl);
                                                    }
                                                }
                                                // TP1 merdiveni: ara hedef tetiklendi → SL breakeven yap, devam et
                                                if !pos.tp1_triggered {
                                                    if let Some(tp1) = pos.tp1_price {
                                                        let tp1_hit = (pos.is_long && price >= tp1)
                                                            || (!pos.is_long && price <= tp1);
                                                        if tp1_hit {
                                                            pos.tp1_triggered = true;
                                                            pos.static_sl = pos.entry_price; // breakeven
                                                            // trailing yoksa varsayılan %1.5 yüklenir
                                                            if pos.trailing_pct.is_none() {
                                                                pos.trailing_pct = Some(1.5);
                                                            }
                                                        }
                                                    }
                                                }

                                                // SL/TP koşulları
                                                let exit_reason: Option<&'static str> =
                                                    if let Some(tsl) = pos.trailing_sl {
                                                        if (pos.is_long && price <= tsl)
                                                            || (!pos.is_long && price >= tsl)
                                                        { Some("trailing_sl") } else { None }
                                                    } else { None }
                                                    .or_else(|| {
                                                        if (pos.is_long && price <= pos.static_sl)
                                                            || (!pos.is_long && price >= pos.static_sl)
                                                        { Some("static_sl") }
                                                        else if (pos.is_long && price >= pos.static_tp)
                                                            || (!pos.is_long && price <= pos.static_tp)
                                                        { Some("take_profit") }
                                                        else { None }
                                                    });

                                                if let Some(reason) = exit_reason {
                                                    // Pozisyonu al ve kaldır
                                                    let snap = pos.clone();
                                                    lm.remove(key.as_str());
                                                    // Adil çıkış fiyatı: SL/TP seviyesini kullan,
                                                    // WS tick fiyatını değil. Tek bir anlık spike
                                                    // PnL'yi şişirmemeli.
                                                    let fair_exit = match reason {
                                                        "static_sl"   => snap.static_sl,
                                                        "trailing_sl" => snap.trailing_sl.unwrap_or(snap.static_sl),
                                                        _             => snap.static_tp, // "take_profit"
                                                    };
                                                    let pnl = pos_pnl(fair_exit, snap.entry_price, snap.qty, snap.is_long);
                                                    let lev_snap = snap.leverage.max(1.0);
                                                    let pnl_pct = if snap.entry_price > 0.0 && snap.qty > 0.0 {
                                                        pnl * lev_snap / (snap.entry_price * snap.qty) * 100.0
                                                    } else { 0.0 };
                                                    // Kesinti: ORPHAN yolu execution_cost_config'e erişmez —
                                                    // commission_pct=0.001 (spot taker) varsayılanı kullanılır, slippage 0
                                                    const ORPHAN_COMM_PCT: f64 = 0.001;
                                                    let entry_comm_w = snap.entry_price * snap.qty * ORPHAN_COMM_PCT;
                                                    let exit_comm_w  = fair_exit        * snap.qty * ORPHAN_COMM_PCT;
                                                    let trade = ClosedTradeData {
                                                        pos_id:      snap.pos_id,
                                                        symbol:      snap.symbol.clone(),
                                                        is_long:     snap.is_long,
                                                        entry_price: snap.entry_price,
                                                        exit_price:  fair_exit,
                                                        qty:         snap.qty,
                                                        pnl,
                                                        pnl_pct,
                                                        exit_reason: reason.to_string(),
                                                        closed_at:   chrono::Utc::now()
                                                            .format("%Y-%m-%d %H:%M:%S")
                                                            .to_string(),
                                                        leverage:    lev_snap,
                                                        sl_price:    snap.static_sl,
                                                        tp_price:    snap.static_tp,
                                                        opened_at:   snap.opened_at.clone(),
                                                        trade_type:  snap.trade_type,
                                                        entry_commission: entry_comm_w,
                                                        exit_commission:  exit_comm_w,
                                                        slippage_usd:     0.0,
                                                        entry_rsi:        0.0,
                                                        entry_atr_pct:    0.0,
                                                        close_adx_regime: 0,
                                                        close_funding_rate: 0.0,
                                                        close_btc_corr:   0.0,
                                                    };
                                                    let log_msg = format!(
                                                        "⛔ [ORPHAN] {} {} kapandı ({}) | \
                                                         çıkış={:.4} tick={:.4} giriş={:.4} PnL={:+.2} ({:+.1}%)",
                                                        snap.symbol,
                                                        if snap.is_long { "LONG" } else { "SHORT" },
                                                        reason,
                                                        fair_exit, price, snap.entry_price, pnl, pnl_pct
                                                    );
                                                    info = Some((trade, log_msg));
                                                }
                                            }
                                        }
                                        // Kapalı işlemi kaydet ve logla (lock dışında)
                                        if let Some((ref trade, ref msg)) = info {
                                            let dup = st.live_closed_trades.read().ok()
                                                .map(|cl| is_duplicate_trade(&cl, trade.pos_id))
                                                .unwrap_or(false);
                                            if !dup {
                                                if let Ok(mut cl) = st.live_closed_trades.write() {
                                                    if cl.len() >= 500 { cl.remove(0); }
                                                    cl.push(trade.clone());
                                                }
                                                // Kümülatif maliyet: trade türüne göre bucket
                                                let comm_usd = trade.entry_commission + trade.exit_commission;
                                                let tt = trade.trade_type;
                                                if let Ok(mut costs) = st.live_execution_costs.write() {
                                                    costs.record(tt, comm_usd, 0.0, trade.slippage_usd, 0.0);
                                                }
                                            }
                                            st.push_log(msg.clone());
                                        }
                                        // Cross-session duplicate fix: pozisyon kapandıysa
                                        // snapshot'ı hemen kaydet — crash/kill sonrası yeni
                                        // session saved_closed_trades'de UUID'yi bulur ve
                                        // tekrar kapatmaz.
                                        let snap = if info.is_some() {
                                            Some(collect_snapshot_payload(&st))
                                        } else { None };
                                        (info, snap)
                                    } else { (None, None) }
                                };
                                // Snapshot diske yaz (lock dışında — async task bloke etme)
                                if let Some(payload) = snap_payload {
                                    let _ = tokio::task::spawn_blocking(|| {
                                        flush_snapshot_payload(payload);
                                    });
                                }

                                // Pozisyon kapandıysa bu WS task'ini sonlandır
                                if close_info.is_some() {
                                    break 'reconnect;
                                }
                            }
                            _ => {}
                        }
                    }
                    // Bağlantı koptu — hemen yeniden bağlan (temiz kapanış)
                }
            });
            handles.insert(composite_key.clone(), handle);
        }

        // Artık pozisyon olmayan sembollerin handle'larını iptal et
        let active_keys: std::collections::HashSet<String> = orphans.iter()
            .map(|(k, _, _)| k.clone()).collect();
        handles.retain(|k, h| {
            if !active_keys.contains(k) { h.abort(); false } else { true }
        });

        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
    }

    // Temizlik
    for (_, h) in handles { h.abort(); }
}

// Tüm aktif semboller (aktif sembol + orchestrator worker'ları) için
// Binance miniTicker WebSocket feed'i açar.
// Her fiyat tick'inde AppState üzerinden GÜNCEL arc alınır → stale arc sorunu yok.
// Sembol listesi her 5sn'de yeniden kontrol edilir; yeni semboller için task spawn edilir.
async fn run_live_price_ws_feeds(
    app_state:   Arc<Mutex<AppState>>,
    stop_signal: Arc<std::sync::atomic::AtomicBool>,
) {
    use futures_util::{SinkExt, StreamExt};
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;
    use tokio_tungstenite::tungstenite::Message;

    // sym → JoinHandle
    let mut handles: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();

    loop {
        if stop_signal.load(Ordering::Relaxed) { break; }

        // Tüm izlenecek sembolleri topla: aktif sembol + orchestrator worker'ları
        // + config'in orijinal primary sembolü (örn. BTCUSDT) her zaman dahil.
        // BTCUSDT piyasa yön göstergesi ve HTF filtresi için kritik —
        // primary geçiş sonrası orchestrator'dan düşse de WS takibi sürer.
        let targets: Vec<(String, String)> = {
            if let Ok(st) = app_state.try_lock() {
                let mut list: Vec<(String, String)> = vec![];
                let (_, active_mkt, active_sym, _) = st.active_trade_target();
                list.push((active_sym.clone(), active_mkt.clone()));
                for handle in st.orchestrator.workers.values() {
                    if handle.symbol == active_sym { continue; }
                    list.push((handle.symbol.clone(), handle.market.clone()));
                }
                // Config'in orijinal primary sembolü: primary geçişten sonra da fiyatını takip et
                let cfg_sym = st.config.symbol.clone();
                let cfg_mkt = st.config.market.clone();
                if !cfg_sym.is_empty() && !list.iter().any(|(s, _)| s == &cfg_sym) {
                    list.push((cfg_sym, cfg_mkt));
                }
                list
            } else { vec![] }
        };

        // Yeni semboller için WS task başlat
        for (sym, mkt) in targets {
            if handles.contains_key(&sym) { continue; }
            let sym_c    = sym.clone();
            let stop_c   = Arc::clone(&stop_signal);
            let state_c  = Arc::clone(&app_state);
            let handle = tokio::spawn(async move {
                let base = if mkt == "futures" {
                    "wss://fstream.binance.com/ws"
                } else {
                    "wss://stream.binance.com:9443/ws"
                };
                let url = format!("{}/{}@miniTicker", base, sym_c.to_lowercase());
                let mut backoff = 1u64;

                'reconnect: loop {
                    if stop_c.load(Ordering::Relaxed) { break; }
                    let mut ws = match tokio_tungstenite::connect_async(&url).await {
                        Ok((ws, _)) => { backoff = 1; ws }
                        Err(e) => {
                            if backoff >= 8 {
                                // Uzun süreli bağlantı hatalarını logla (ilk birkaç retry sessiz)
                                if let Ok(mut st) = state_c.try_lock() {
                                    st.push_log(format!(
                                        "⚠️  WS bağlantı hatası [{}]: {} — {}s sonra yeniden denenecek",
                                        sym_c, e, backoff
                                    ));
                                }
                            }
                            tokio::time::sleep(tokio::time::Duration::from_secs(backoff)).await;
                            backoff = (backoff * 2).min(15); // DNS kurtarma: max 15s cap
                            continue 'reconnect;
                        }
                    };

                    while let Some(msg_result) = ws.next().await {
                        if stop_c.load(Ordering::Relaxed) { break 'reconnect; }
                        let msg = match msg_result { Ok(m) => m, Err(_) => break };
                        match msg {
                            Message::Ping(data) => { let _ = ws.send(Message::Pong(data)).await; }
                            Message::Close(_) => break,
                            Message::Text(text) => {
                                #[derive(serde::Deserialize)]
                                struct MiniTick {
                                    #[serde(rename="c")] c: String,
                                    #[serde(rename="o")] o: String,
                                    #[serde(rename="h")] h: String,
                                    #[serde(rename="l")] l: String,
                                    #[serde(rename="v")] v: String,
                                }
                                if let Ok(t) = serde_json::from_str::<MiniTick>(&text) {
                                    let close: f64 = t.c.parse().unwrap_or(0.0);
                                    if close <= 0.0 { continue; }
                                    // AppState'ten GÜNCEL arc'ı al — try_lock ile bloklamadan
                                    let arc_opt = if let Ok(st) = state_c.try_lock() {
                                        let (_, _, active_sym, _) = st.active_trade_target();
                                        let a = st.orchestrator.live_price_for(&sym_c)
                                            .unwrap_or_else(|| {
                                                // Config'in orijinal primary sembolü (örn. BTCUSDT) aktif
                                                // primary'den farklılaşınca ayrı arc'a yaz; live_price'ı kirletme.
                                                if sym_c != active_sym && sym_c == st.config.symbol {
                                                    Arc::clone(&st.config_symbol_price)
                                                } else {
                                                    Arc::clone(&st.live_price)
                                                }
                                            });
                                        Some(a)
                                        // st (guard) burada drop olur → Mutex anında serbest
                                    } else {
                                        None // TUI renderlıyor, bu tick'i atla
                                    };
                                    if let Some(arc) = arc_opt {
                                        let open: f64 = t.o.parse().unwrap_or(0.0);
                                        let high: f64 = t.h.parse().unwrap_or(0.0);
                                        let low:  f64 = t.l.parse().unwrap_or(0.0);
                                        let vol:  f64 = t.v.parse().unwrap_or(0.0);
                                        if let Ok(mut pd) = arc.write() {
                                            let session_open = if pd.open > 0.0 { pd.open } else { open };
                                            pd.symbol     = sym_c.clone();
                                            pd.open       = session_open;
                                            pd.high       = high;
                                            pd.low        = low;
                                            pd.close      = close;
                                            pd.volume     = vol;
                                            pd.change_pct = if session_open > 0.0 {
                                                (close - session_open) / session_open * 100.0
                                            } else { 0.0 };
                                            pd.ts = chrono::Local::now().format("%H:%M:%S").to_string();
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    // Bağlantı koptu — yeniden dene (max 15s cap: DNS kurtarma hızlandırması)
                    tokio::time::sleep(tokio::time::Duration::from_secs(backoff)).await;
                    backoff = (backoff * 2).min(15);
                }
            });
            handles.insert(sym, handle);
        }

        // Biten task'leri temizle
        handles.retain(|_, h| !h.is_finished());

        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }

    for (_, h) in handles { h.abort(); }
}

// ── Binance Sembol Tarayıcı Worker ───────────────────────────────────────────
// Periyodik olarak Binance 24hr ticker'ı çeker; hacim + volatilite filtresi
// uygular; DB'de henüz mum verisi olmayan yeni sembolleri screener_candidates'e
// ekler ve download_trigger'ı ateşler. Download worker yeni sembolleri indirir,
// ardından run_symbol_selector_worker bunları otomatik puanlar.
fn run_symbol_screener_worker(
    app_state:   Arc<Mutex<AppState>>,
    stop_signal: Arc<AtomicBool>,
    db_path:     String,
) {
    std::thread::spawn(move || {
        use std::sync::atomic::Ordering;
        use rusqlite::Connection;

        // Uygulama başlarken screener_enabled kontrol et; devre dışıysa çık
        let enabled = app_state.lock().ok().map(|s| s.screener_enabled).unwrap_or(true);
        if !enabled { return; }

        // İlk taramadan önce sistemin hazırlanmasını bekle
        std::thread::sleep(Duration::from_secs(45));

        // Tarama periyodunu config'den al
        let scan_every_secs = {
            app_state.lock().ok()
                .map(|s| (s.screener_interval_hours * 3600.0) as u64)
                .unwrap_or(4 * 3600)
        }.max(60);

        let mut elapsed = scan_every_secs; // ilk taramayı hemen başlat
        let tick = 5u64;

        // Tokio runtime — async HTTP için
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("screener runtime");

        loop {
            if stop_signal.load(Ordering::Relaxed) { break; }

            let triggered = app_state.lock().ok()
                .map(|s| s.screener_trigger.swap(false, Ordering::Relaxed))
                .unwrap_or(false);

            if triggered || elapsed >= scan_every_secs {
                elapsed = 0;

                // Parametreleri al
                let (min_vol_m, min_chg_pct, max_new, market_str, _exchange_str) = {
                    match app_state.lock() {
                        Ok(s) => (
                            s.screener_min_volume_m,
                            s.screener_min_change_pct,
                            s.screener_max_new,
                            s.config.market.clone(),
                            s.config.exchange.clone(),
                        ),
                        Err(_) => {
                            std::thread::sleep(Duration::from_secs(tick));
                            elapsed += tick;
                            continue;
                        }
                    }
                };

                let is_futures = market_str.to_lowercase().contains("futures")
                    || market_str.to_lowercase() == "coinm";
                let ticker_url = if is_futures {
                    "https://fapi.binance.com/fapi/v1/ticker/24hr"
                } else {
                    "https://api.binance.com/api/v3/ticker/24hr"
                };

                if let Ok(mut st) = app_state.lock() {
                    st.push_log(format!("🔍 Sembol tarayıcı başladı ({})", ticker_url));
                }

                // Binance 24hr ticker verisi çek
                let raw_json = rt.block_on(async {
                    let client = build_http_client().unwrap_or_default();
                    match client.get(ticker_url).send().await {
                        Ok(r) => r.text().await.unwrap_or_default(),
                        Err(e) => { eprintln!("[screener] ticker hatası: {}", e); String::new() }
                    }
                });

                if raw_json.is_empty() {
                    if let Ok(mut st) = app_state.lock() {
                        st.push_log("⚠️ Tarayıcı: Binance ticker alınamadı".to_string());
                    }
                    std::thread::sleep(Duration::from_secs(tick));
                    elapsed += tick;
                    continue;
                }

                let tickers: Vec<serde_json::Value> =
                    serde_json::from_str(&raw_json).unwrap_or_default();

                let total_tickers = tickers.len();

                // Stablecoin baz listesi (USDCUSDT, BUSDUSDT gibi çiftleri dışla)
                const STABLECOINS: &[&str] = &[
                    "USDC","BUSD","TUSD","DAI","USDP","FDUSD","USDD","GUSD","FRAX","USDJ",
                ];
                // Bilinen düşük kaliteli / manipüle edilebilir tokenlar
                const BLOCKLIST: &[&str] = &["LUNC", "LUNA", "FTT", "CELR"];

                let min_vol = min_vol_m * 1_000_000.0;

                let mut candidates: Vec<ScreenerCandidate> = tickers.iter()
                    .filter_map(|t| {
                        let sym = t["symbol"].as_str()?;
                        // ASCII olmayan karakter içeren sembolleri dışla (örn. Çince karakterler)
                        if !sym.bytes().all(|b| b.is_ascii_alphanumeric()) { return None; }
                        if !sym.ends_with("USDT") { return None; }
                        let base = sym.trim_end_matches("USDT");
                        if STABLECOINS.contains(&base) || BLOCKLIST.contains(&base) {
                            return None;
                        }
                        let vol: f64   = t["quoteVolume"].as_str()?.parse().ok()?;
                        let chg: f64   = t["priceChangePercent"].as_str()?.parse().ok()?;
                        let price: f64 = t["lastPrice"].as_str()?.parse().ok()?;
                        let count: u64 = t["count"].as_i64()
                            .map(|v| v as u64)
                            .or_else(|| t["count"].as_u64())
                            .unwrap_or(0);

                        if vol < min_vol            { return None; }
                        if chg.abs() < min_chg_pct  { return None; }
                        if price < 0.000_01          { return None; } // aşırı düşük fiyat dışla

                        Some(ScreenerCandidate {
                            symbol:           sym.to_string(),
                            quote_volume_24h: vol,
                            price_change_pct: chg,
                            trade_count_24h:  count,
                            last_price:       price,
                            status:           ScreenerStatus::New,
                            found_at:         chrono::Utc::now()
                                                .format("%H:%M:%S").to_string(),
                        })
                    })
                    .collect();

                // Hacme göre büyükten küçüğe sırala
                candidates.sort_by(|a, b|
                    b.quote_volume_24h.partial_cmp(&a.quote_volume_24h)
                        .unwrap_or(std::cmp::Ordering::Equal)
                );

                // DB'de zaten mum verisi olan semboller — bunları atla
                let known: std::collections::HashSet<String> = match Connection::open(&db_path) {
                    Ok(conn) => conn
                        .prepare("SELECT DISTINCT symbol FROM candles")
                        .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))
                            .map(|rows| rows.filter_map(|r| r.ok()).collect()))
                        .unwrap_or_default(),
                    Err(_) => std::collections::HashSet::new(),
                };

                // Mevcut screener adayları — zaten takip edilenleri tekrar ekleme
                let already_tracked: std::collections::HashSet<String> = app_state.lock().ok()
                    .map(|s| s.screener_candidates.iter().map(|c| c.symbol.clone()).collect())
                    .unwrap_or_default();

                let new_candidates: Vec<ScreenerCandidate> = candidates.into_iter()
                    .filter(|c| !known.contains(&c.symbol) && !already_tracked.contains(&c.symbol))
                    .take(max_new)
                    .collect();

                let found = new_candidates.len();
                let now_str = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

                if let Ok(mut st) = app_state.lock() {
                    // Puanlanmış/Reddedilmiş adayları temizle; yeni adaylar ekle
                    st.screener_candidates.retain(|c|
                        c.status != ScreenerStatus::Rejected &&
                        c.status != ScreenerStatus::Scored
                    );
                    st.screener_candidates.extend(new_candidates);
                    st.screener_last_run = Some(now_str.clone());

                    if found > 0 {
                        st.push_log(format!(
                            "🔍 Tarayıcı: {} yeni sembol bulundu ({} toplam incelendi) — indirme başlatıldı",
                            found, total_tickers
                        ));
                        // Yeni semboller için download tetikle
                        st.download_trigger.store(true, Ordering::Relaxed);
                    } else {
                        st.push_log(format!(
                            "🔍 Tarayıcı: Yeni keşif yok ({} sembol incelendi, filtre: ≥{:.0}M USDT hacim, ≥{:.1}% değişim)",
                            total_tickers, min_vol_m, min_chg_pct
                        ));
                    }
                }

                // Kısa bir bekle: ticker API'ye saygılı ol
                std::thread::sleep(Duration::from_secs(2));
            }

            std::thread::sleep(Duration::from_secs(tick));
            elapsed += tick;
        }
    });
}

// DB'deki (exchange/market/symbol/interval) kombinasyonlarını puanlar:
//   Mum sayısı eşiği: < 30 mum olan sembol atlanır (istatistiksel veri yetersiz)
//   skor = win_rate×0.35 + profit_factor×0.30 + sharpe×0.20 + (1-drawdown)×0.15
//   trade_conf penaltısı: trade sayısı < 10 ise skor * (trade_sayısı / 10)
// En yüksek puan alan kombinasyonu AppState.active_symbol olarak atar.
// auto_symbol=false ise sadece symbol_candidates listesini günceller (tavsiye).

fn run_symbol_selector_worker(
    app_state:    Arc<Mutex<AppState>>,
    stop_signal:  Arc<AtomicBool>,
    db_path:      String,
) {
    std::thread::spawn(move || {
        use std::sync::atomic::Ordering;
        use rusqlite::Connection;

        // İlk tarama: uygulama hazır olsun, sonra hemen tara
        std::thread::sleep(Duration::from_secs(10));

        let scan_every_secs = 300u64; // her 5 dakikada bir tara
        let mut elapsed = scan_every_secs; // ilk taramayı hemen başlat
        let tick = 1u64;

        loop {
            if stop_signal.load(Ordering::Relaxed) { break; }

            let triggered = {
                if let Ok(st) = app_state.lock() {
                    st.symbol_trigger.swap(false, Ordering::Relaxed)
                } else { false }
            };

            if triggered || elapsed >= scan_every_secs {
                elapsed = 0;

                // Aktif sembol snapshot — step 3'de bu sembolü backtest dışında tut.
                // Backtest worker aktif sembolü ayrıca işler; scanner'ın step 3'ü
                // backtest worker'ın sonucunu iyi sonuçla ezerse geçiş kalıcı bloke olur.
                let (active_sym_snap, active_mkt_snap, active_intv_snap) = {
                    if let Ok(st) = app_state.lock() {
                        (st.active_symbol.symbol.clone(),
                         st.active_symbol.market.clone(),
                         st.active_symbol.interval.clone())
                    } else { (String::new(), String::new(), String::new()) }
                };

                // ── 1. Candle envanterini çek ─────────────────────────────
                let conn = match Connection::open(&db_path) {
                    Ok(c) => c,
                    Err(e) => {
                        if let Ok(mut st) = app_state.lock() {
                            st.push_log(format!("⚠️  SembolSeçici DB: {}", e));
                        }
                        std::thread::sleep(Duration::from_secs(tick));
                        elapsed += tick;
                        continue;
                    }
                };

                // Candle sayısını al, minimum 50 mum şartı
                // MAX(timestamp) filtresi: son mumu 30 günden eski olan kombinasyonlar dışlanır
                // (Aralık ayından kalma stale 1m verileri 200 limitini boşa harcamasın)
                let stale_ms_cutoff = (chrono::Utc::now().timestamp() - 30 * 24 * 3600) * 1000;
                let rows: Vec<(String, String, String, String, usize)> =
                    match conn.prepare(
                        "SELECT exchange, market, symbol, interval, COUNT(*) as cnt \
                         FROM candles \
                         GROUP BY exchange, market, symbol, interval \
                         HAVING cnt >= 50 AND MAX(timestamp) >= ?1 \
                         ORDER BY cnt DESC \
                         LIMIT 200"
                    ) {
                        Err(_) => Vec::new(),
                        Ok(mut stmt) => match stmt.query_map(rusqlite::params![stale_ms_cutoff], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, i64>(4)? as usize,
                            ))
                        }) {
                            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
                            Err(_) => Vec::new(),
                        },
                    };

                if rows.is_empty() {
                    std::thread::sleep(Duration::from_secs(tick));
                    elapsed += tick;
                    continue;
                }

                // ── 2. Her kombinasyon için backtest skoru al ─────────────
                let _max_candles = rows.iter().map(|r| r.4).max().unwrap_or(1) as f64;

                let mut candidates: Vec<SymbolScore> = rows.iter().map(|(exch, mkt, sym, intv, cnt)| {
                    // backtest_results tablosundan en güncel kayıt — profit_factor / sharpe / drawdown dahil
                    let (win_rate, total_pnl, bt_trades, profit_factor, sharpe_ratio, max_drawdown_pct) = conn.query_row(
                        "SELECT win_rate, total_pnl, total_trades, \
                                COALESCE(profit_factor,0.0), COALESCE(sharpe_ratio,0.0), COALESCE(max_drawdown_pct,0.0) \
                         FROM backtest_results \
                         WHERE exchange=?1 AND market=?2 AND symbol=?3 AND interval=?4 \
                         ORDER BY id DESC LIMIT 1",
                        rusqlite::params![exch, mkt, sym, intv],
                        |row| Ok((
                            row.get::<_, f64>(0).unwrap_or(0.0),
                            row.get::<_, f64>(1).unwrap_or(0.0),
                            row.get::<_, i64>(2).unwrap_or(0),
                            row.get::<_, f64>(3).unwrap_or(0.0),
                            row.get::<_, f64>(4).unwrap_or(0.0),
                            row.get::<_, f64>(5).unwrap_or(0.0),
                        )),
                    ).unwrap_or((0.0, 0.0, 0, 0.0, 0.0, 0.0));

                    // Son mumun fiyatı ve zaman damgasını DB'den çek
                    let (last_price, last_candle_ts) = conn.query_row(
                        "SELECT close, timestamp FROM candles \
                         WHERE exchange=?1 AND market=?2 AND symbol=?3 AND interval=?4 \
                         ORDER BY timestamp DESC LIMIT 1",
                        rusqlite::params![exch, mkt, sym, intv],
                        |row| Ok((row.get::<_, f64>(0)?, row.get::<_, i64>(1)? / 1000)),
                    ).unwrap_or((0.0, 0));

                    // Sabit break-even: DB'den best_sl/best_tp alınamadığı için 33.3% kullan
                    let be_wr = 33.3_f64;
                    let score = compute_symbol_score(
                        win_rate, profit_factor, sharpe_ratio, max_drawdown_pct,
                        bt_trades as usize, total_pnl, *cnt, last_price, last_candle_ts, sym, be_wr,
                    );

                    SymbolScore {
                        exchange:         exch.clone(),
                        market:           mkt.clone(),
                        symbol:           sym.clone(),
                        interval:         intv.clone(),
                        candle_count:     *cnt,
                        win_rate,
                        total_pnl,
                        total_trades:     bt_trades as usize,
                        score,
                        last_price,
                        last_candle_ts,
                        best_strategy:    String::new(), // re-backtest'te doldurulur
                        profit_factor,
                        sharpe_ratio,
                        max_drawdown_pct,
                    }
                }).collect();

                // ── 3. Backtest edilmemiş VEYA 7 günden eski sonucu olan sembolleri yeniden test et
                // (her scan döngüsünde en fazla 8 sembol — tüm adaylar ~3 turda tamamlanır)
                // Bayat backtest: 7 gün sonra yeni candle verisiyle yeniden hesaplanır.
                let stale_cutoff_secs = chrono::Utc::now().timestamp() - 3 * 24 * 3600; // 3 gün: adaylar daha sık yeniden skorlanır
                let unscored: Vec<_> = candidates.iter()
                    .filter(|c| c.last_candle_ts > 0
                        && !is_stale_data(c.last_candle_ts)
                        && c.candle_count >= 50
                        // Aktif sembolü backtest etme — backtest worker hallediyor.
                        // Scanner'ın inline backtesti backtest worker'ın kötü sonucunu
                        // iyi sonuçla ezerse sembol geçişi kalıcı olarak bloke olur.
                        && !(c.symbol == active_sym_snap
                             && c.market == active_mkt_snap
                             && c.interval == active_intv_snap)
                        && {
                            // DB'den bu sembolün son backtest zamanını kontrol et
                            // (0 trade sonuçlar da stale check'e tabi — sürekli yeniden test edilmelerini engeller)
                            let last_bt: i64 = conn.query_row(
                                "SELECT strftime('%s', MAX(created_at)) FROM backtest_results \
                                 WHERE exchange=?1 AND market=?2 AND symbol=?3 AND interval=?4",
                                rusqlite::params![&c.exchange, &c.market, &c.symbol, &c.interval],
                                |row| row.get::<_, Option<String>>(0),
                            ).ok().flatten()
                             .and_then(|s| s.parse::<i64>().ok())
                             .unwrap_or(0);
                            last_bt < stale_cutoff_secs
                        })
                    .take(15)
                    .map(|c| (c.exchange.clone(), c.market.clone(), c.symbol.clone(), c.interval.clone()))
                    .collect();
                for (exch, mkt, sym, intv) in &unscored {
                    // DB'den candle çek
                    let candles_db: Vec<memos_trading_core::types::Candle> = {
                        // 1h ve üzeri interval için daha fazla mum — daha güvenilir backtest
                        let candle_limit = if intv.ends_with('h') || intv.ends_with('d') { 1500 } else { 500 };
                        let mut stmt = match conn.prepare(&format!(
                            "SELECT timestamp, open, high, low, close, volume FROM candles \
                             WHERE exchange=?1 AND market=?2 AND symbol=?3 AND interval=?4 \
                             ORDER BY timestamp DESC LIMIT {} ",
                            candle_limit
                        )) { Ok(s) => s, Err(_) => continue };
                        stmt.query_map(rusqlite::params![exch, mkt, sym, intv], |row| {
                            let ts_ms: i64 = row.get(0)?;
                            let ts = chrono::DateTime::from_timestamp(ts_ms / 1000, 0)
                                .map(|t| t.with_timezone(&chrono::Utc))
                                .ok_or_else(|| rusqlite::Error::InvalidColumnType(0, "ts".into(), rusqlite::types::Type::Integer))?;
                            Ok(memos_trading_core::types::Candle {
                                timestamp: ts, open: row.get(1)?, high: row.get(2)?,
                                low: row.get(3)?, close: row.get(4)?, volume: row.get(5)?,
                                symbol: sym.clone(), interval: intv.clone(),
                            })
                        }).ok().map(|r| r.filter_map(|x| x.ok()).collect())
                        .unwrap_or_default()
                    };
                    if candles_db.len() < 50 { continue; }
                    let (cap, sl, tp) = {
                        if let Ok(st) = app_state.lock() {
                            (st.config.capital, st.best_sl, st.best_tp)
                        } else { (10000.0, 0.5, 1.5) }
                    };
                    // 4 stratejiyi test et, en iyisini seç
                    if let Some((result, best_strat)) = best_strategy_backtest(
                        &candles_db, sym, intv, sl, tp, cap, 0.01,
                    ) {
                        // backtest_results tablosuna yaz
                        if let Ok(bconn) = database_writer::open_connection(&db_path) {
                            let _ = database_writer::save_backtest_result(
                                &bconn, &best_strat, exch, mkt, sym, intv,
                                None, None, None, None, None, None, None, None, None, None,
                                None, None, None, None, None, None, None, None, None, None,
                                cap, result.total_trades as i32, result.winning_trades as i32,
                                result.losing_trades as i32, result.win_rate, result.total_pnl,
                                cap + result.total_pnl,
                                result.profit_factor, result.sharpe_ratio, result.max_drawdown_pct,
                                Some("auto-scorer"),
                            );
                        }
                        // in-memory güncelle
                        if let Some(c) = candidates.iter_mut().find(|c| c.symbol == *sym && c.interval == *intv) {
                            c.win_rate         = result.win_rate;
                            c.total_pnl        = result.total_pnl;
                            c.total_trades     = result.total_trades;
                            c.profit_factor    = result.profit_factor;
                            c.sharpe_ratio     = result.sharpe_ratio;
                            c.max_drawdown_pct = result.max_drawdown_pct;
                            c.best_strategy    = best_strat.clone();
                            c.last_price       = candles_db.last().map(|x| x.close).unwrap_or(0.0);
                            c.last_candle_ts   = candles_db.last().map(|x| x.timestamp.timestamp()).unwrap_or(0);
                            let be_wr = {
                                if let Ok(st) = app_state.lock() { st.best_sl / (st.best_sl + st.best_tp) * 100.0 } else { 33.3 }
                            };
                            let sym_clone = c.symbol.clone();
                            c.score = compute_symbol_score(
                                c.win_rate, c.profit_factor, c.sharpe_ratio, c.max_drawdown_pct,
                                c.total_trades, c.total_pnl, c.candle_count,
                                c.last_price, c.last_candle_ts, &sym_clone, be_wr,
                            );
                        }
                        if let Ok(mut st) = app_state.lock() {
                            st.push_log(format!(
                                "📊 Otomatik skor: {}/{} strat={} win={:.1}% pf={:.2} dd={:.1}%",
                                sym, intv, best_strat, result.win_rate,
                                result.profit_factor, result.max_drawdown_pct,
                            ));
                        }
                    }
                }

                // ── 4. MTF bonus — aktif sinyal olan sembollere skor katkısı ──
                // mtf_opportunities listesindeki sembol başına max skor alınır.
                // Backtest skoruna ağırlıklı bonus eklenir: bonus = mtf_skor × 0.10, max +0.15.
                // Bu sayede backtest kalitesi korunurken o an güçlü sinyali olan semboller öne çıkar.
                {
                    let mut mtf_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
                    for _ in 0..5 {
                        if let Ok(st) = app_state.try_lock() {
                            for opp in &st.mtf_opportunities {
                                let e = mtf_map.entry(opp.symbol.clone()).or_insert(0.0_f64);
                                *e = e.max(opp.score);
                            }
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    let mut bonus_log: Vec<String> = Vec::new();
                    for c in &mut candidates {
                        if let Some(&mtf_s) = mtf_map.get(&c.symbol) {
                            let bonus = (mtf_s * 0.10_f64).min(0.15);
                            if bonus > 0.005 {
                                bonus_log.push(format!("{}+{:.3}", c.symbol, bonus));
                                c.score += bonus;
                            }
                        }
                    }
                    if !bonus_log.is_empty() {
                        if let Ok(mut st) = app_state.try_lock() {
                            st.push_log(format!("📡 MTF bonus uygulandı: {}", bonus_log.join(", ")));
                        }
                    }
                }

                // ── 5. Skorlara göre sırala (en iyi başta) ───────────────
                candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

                // ── 6. AppState güncelle ──────────────────────────────────
                if let Ok(mut st) = app_state.lock() {
                    st.last_backtest_at = Some(chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string());
                    let auto = st.auto_symbol;
                    let _prev_sym = st.active_symbol.symbol.clone();

                    if auto {
                        if let Some(best) = candidates.first() {
                            // cur'u klonla — sonraki mutable borrow'larla çakışmasın
                            let mut cur = st.active_symbol.clone();
                            // Snapshot'tan gelen eski/stale skoru tazele:
                            // candidates listesinde aktif sembol varsa oradan güncel skoru al.
                            // Aksi halde switch mantığı eski skora göre karar verir.
                            if let Some(fresh) = candidates.iter().find(|c|
                                c.symbol   == cur.symbol
                                && c.market   == cur.market
                                && c.interval == cur.interval
                            ) {
                                cur.score = fresh.score;
                                // Snapshot'a da yaz — bir sonraki restart'ta stale skor kalmasın
                                st.active_symbol.score = fresh.score;
                            }
                            let identity_changed = best.symbol   != cur.symbol
                                                || best.interval != cur.interval
                                                || best.market   != cur.market;

                            if identity_changed {
                                // ── Kural A: Yeni sembol en az %8 daha iyi olmalı ──────
                                // Aktif sembol score=0 VE en iyi adayın skoru pozitifse → acil geçiş
                                // (BEATUSDT score=0,PF=0.70 gibi zararlı sembollerde takılmayı önler)
                                let cur_is_loser = cur.score <= 0.0 && best.score > 0.01;
                                let score_ok = cur.score < 0.001  // ilk çalışma / sıfır skor
                                    || cur_is_loser               // aktif zararlı, acil geçiş
                                    || best.score >= cur.score * 1.08;

                                // ── Kural B: Mevcut sembolde minimum süre geçmeli ──────
                                // interval_secs × 1 cycle (min 1dk, max 1sa)
                                // Düşük skorlu sembol (<0.15) için min_stay %30'a indirilir —
                                // kötü performanslı sembolde 30 dakika beklemenin anlamı yok.
                                let intv = cur.interval.trim().to_lowercase();
                                let isecs: u64 = if let Some(n) = intv.strip_suffix('m') {
                                    n.parse::<u64>().unwrap_or(1) * 60
                                } else if let Some(n) = intv.strip_suffix('h') {
                                    n.parse::<u64>().unwrap_or(1) * 3600
                                } else if let Some(n) = intv.strip_suffix('d') {
                                    n.parse::<u64>().unwrap_or(1) * 86400
                                } else { 60 };
                                let min_stay_base = isecs.clamp(60, 3_600); // 1 mum–1sa (maks 1sa)
                                // Kötü performans → süreyi kısalt
                                let min_stay = if cur.score < 0.15 {
                                    (min_stay_base as f64 * 0.30) as u64
                                } else {
                                    min_stay_base
                                };
                                let elapsed  = st.loop_active_since.elapsed().as_secs();
                                // Skor=0 veya çok düşük ise süre şartı aranmaz
                                let time_ok  = elapsed >= min_stay || cur.score < 0.001;

                                // ── Kural C: Spot+Bearish kilidi — spot SELL N kez bloklandıysa geçişi zorla ──
                                let spot_blocks = st.live_risk.read().ok()
                                    .map(|lr| lr.spot_sell_blocks)
                                    .unwrap_or(0);
                                let force_switch = spot_blocks >= 8 && cur.market == "spot";

                                if (score_ok && time_ok) || force_switch {
                                    if force_switch {
                                        st.push_log(format!(
                                            "⚡ Spot SELL kilidi ({} kez) → {} geçiş zorlandı",
                                            spot_blocks, best.symbol
                                        ));
                                        if let Ok(mut lr) = st.live_risk.write() {
                                            lr.spot_sell_blocks = 0;
                                        }
                                    }
                                    st.push_log(format!(
                                        "🎯 Geçiş: {}/{} (skor={:.4}) → {}/{} (skor={:.4}) | +{:.1}% fark | {}sn beklendi",
                                        cur.symbol, cur.interval, cur.score,
                                        best.symbol, best.interval, best.score,
                                        (best.score / cur.score.max(0.001) - 1.0) * 100.0,
                                        elapsed,
                                    ));
                                    st.loop_active_since = Instant::now();
                                    st.loop_restart_trigger.store(true, Ordering::Relaxed);
                                    // Yeni sembol için ML worker'ı anında tetikle —
                                    // eski sembolün negatif HyperOpt skoru yeni sembolü engellemesein
                                    st.ml_trigger.store(true, Ordering::Relaxed);
                                    st.ml_next_run_at = Instant::now();
                                    // HyperOpt skorunu sıfırla — stale skor sinyali bloke etmesin
                                    st.hyperopt_score = 0.0;
                                    // live_risk.hyperopt_score'u da sıfırla —
                                    // robotic loop live_risk'ten okur, ML çalışana kadar bloke etmemeli
                                    if let Ok(mut lr) = st.live_risk.write() {
                                        lr.hyperopt_score = 0.0;
                                    }
                                    if let Ok(mut las) = st.live_active_symbol.write() {
                                        *las = best.symbol.clone();
                                    }
                                    // auto_interval=false ise kullanıcının seçtiği interval korunur
                                    let mut new_sym = best.clone();
                                    if !st.auto_interval {
                                        new_sym.interval = st.config.interval.clone();
                                    }
                                    st.active_symbol = new_sym;
                                } else {
                                    // Geçiş engellendi — sebebini logla
                                    let reason = if !score_ok && !time_ok {
                                        format!("skor farkı yetersiz ({:.1}%) VE süre yetersiz ({}s/{}s) | cur={:.4} best={:.4}",
                                            (best.score / cur.score.max(0.001) - 1.0) * 100.0, elapsed, min_stay,
                                            cur.score, best.score)
                                    } else if !score_ok {
                                        format!("skor farkı yetersiz ({:.1}% < %8) | cur={:.4} best={:.4}",
                                            (best.score / cur.score.max(0.001) - 1.0) * 100.0,
                                            cur.score, best.score)
                                    } else {
                                        format!("bekleme süresi dolmadı ({}s/{}s) | cur_score={:.4}",
                                            elapsed, min_stay, cur.score)
                                    };
                                    st.push_log(format!(
                                        "⏸ Geçiş bekleniyor: {}/{} → {}/{} | {}",
                                        cur.symbol, cur.interval, best.symbol, best.interval, reason
                                    ));
                                }
                            } else {
                                // Aynı sembol, skor güncelle
                                let mut upd = best.clone();
                                if !st.auto_interval {
                                    upd.interval = st.config.interval.clone();
                                }
                                st.active_symbol = upd;
                            }
                        }
                    }
                    st.symbol_candidates = candidates;
                    // Screener adaylarının durumunu güncelle:
                    // Downloaded → Scored (symbol_candidates'ta score>0 varsa) veya Rejected
                    // Önce scored semboller toplanır (immutable borrow), sonra screener güncellenir
                    let scored_syms: std::collections::HashSet<String> = st.symbol_candidates
                        .iter()
                        .filter(|c| c.score > 0.0)
                        .map(|c| c.symbol.clone())
                        .collect();
                    for sc in &mut st.screener_candidates {
                        if sc.status == ScreenerStatus::Downloaded {
                            if scored_syms.contains(&sc.symbol) {
                                sc.status = ScreenerStatus::Scored;
                            } else {
                                sc.status = ScreenerStatus::Rejected;
                            }
                        }
                    }
                    // Sembol seçimi değişti → snapshot kaydet
                    save_app_snapshot(&st);
                }
            }

            // Kilit bırakıldıktan sonra çok-sembol worker havuzunu senkronize et
            sync_orchestrator_workers(Arc::clone(&app_state));

            std::thread::sleep(Duration::from_secs(tick));
            elapsed += tick;
        }
    });
}

// ─── Periyodik Backtest Worker ────────────────────────────────────────────────
// Kendi std::thread'inde çalışır, her N dakikada bir DB'den mum yükler,
// Backtester::run() ile analiz eder ve sonucu AppState.last_backtest'e yazar.

fn run_backtest_worker(
    app_state:        Arc<Mutex<AppState>>,
    stop_signal:      Arc<AtomicBool>,
    backtest_running: Arc<AtomicBool>,
    config:           OtoConfig,
) {
    std::thread::spawn(move || {
        use std::sync::atomic::Ordering;

        if !config.backtest_enabled {
            return;
        }

        // İlk çalışmadan önce kısa bekleme — engine hazırlansın
        std::thread::sleep(std::time::Duration::from_secs(30));

        let interval_secs = config.backtest_every_mins * 60;
        let mut elapsed = 0u64;
        let tick = 1u64;
        // Trigger spam önleme: aynı saniyede birden fazla tetiklenmeyi engeller
        let mut last_run_at: Option<Instant> = None;

        loop {
            if stop_signal.load(Ordering::Relaxed) {
                break;
            }

            // backtest_trigger veya zaman doldu mu?
            let triggered = {
                if let Ok(st) = app_state.lock() {
                    st.backtest_trigger.swap(false, Ordering::Relaxed)
                } else { false }
            };
            let time_up = elapsed >= interval_secs;

            if triggered || time_up {
                // Trigger spam önleme: son çalışmadan 30s geçmeden tekrar tetikleme
                if triggered && !time_up {
                    if let Some(last) = last_run_at {
                        if last.elapsed() < Duration::from_secs(30) {
                            let remaining = 30u64.saturating_sub(last.elapsed().as_secs());
                            if let Ok(mut st) = app_state.lock() {
                                st.push_log(format!(
                                    "⏳ Backtest spam koruması — {}s sonra tekrar denenebilir",
                                    remaining
                                ));
                            }
                            std::thread::sleep(std::time::Duration::from_secs(tick));
                            elapsed += tick;
                            continue;
                        }
                    }
                }
                last_run_at = Some(Instant::now());
                elapsed = 0;

                // Stop sinyali geldiyse backtest başlatma (eski worker çift çalışmasın)
                if stop_signal.load(Ordering::Relaxed) { break; }

                // Eşzamanlı çift backtest önleme: başka bir worker zaten çalışıyorsa atla
                if backtest_running.compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed).is_err() {
                    // Başka bir backtest_worker zaten çalışıyor
                    if let Ok(mut st) = app_state.lock() {
                        st.push_log("⏳ Backtest zaten çalışıyor — önceki tamamlanınca yenisi başlar".to_string());
                    }
                    std::thread::sleep(std::time::Duration::from_secs(tick));
                    elapsed += tick;
                    continue;
                }

                // Aktif hedefi al (auto_symbol modu veya manuel config)
                let (act_exch, act_mkt, act_sym, act_intv) = {
                    if let Ok(st) = app_state.lock() {
                        st.active_trade_target()
                    } else {
                        (config.exchange.clone(), config.market.clone(),
                         config.symbol.clone(), config.interval.clone())
                    }
                };

                // DB'den mumları yükle
                let candles = database_reader::read_candles(
                    &config.db_path,
                    &act_exch,
                    &act_mkt,
                    &act_sym,
                    &act_intv,
                    Some(config.backtest_candle_limit),
                );

            match candles {
                Ok(c) if c.len() >= 30 => {
                    // Otonom SL/TP: snapshot'tan al
                    let (cur_sl, cur_tp) = if let Ok(st) = app_state.lock() {
                        (st.best_sl, st.best_tp)
                    } else { (2.0_f64, 4.0_f64) };
                    let bt_cfg = BacktestConfig {
                        symbol:            act_sym.clone(),
                        interval:          act_intv.clone(),
                        initial_balance:   config.capital,
                        max_position_size: config.trade_amount,
                        take_profit_pct:   cur_tp,
                        stop_loss_pct:     cur_sl,
                        strategy_name:     "MA_CROSSOVER".to_string(),
                        position_profile:  Some("Balanced".to_string()),
                        security_profile:  Some("Development".to_string()),
                        strategy_params:   None,
                        commission_pct:    0.001,
                        breakeven_at_rr:   None,
                        atr_trail_mult:    None,
                        partial_tp_ratio:  None,
                    };

                    let mut bt = Backtester::new(bt_cfg);
                    match bt.run(&c) {
                        Ok(result) => {
                            // DB'ye kaydet
                            if let Ok(conn) = database_writer::open_connection(&config.db_path) {
                                let _ = database_writer::save_backtest_result(
                                    &conn,
                                    "MA_CROSSOVER",
                                    &act_exch, &act_mkt,
                                    &act_sym, &act_intv,
                                    Some(5), Some(20),
                                    None, None, None,
                                    None, None, None, None, None,
                                    None, None, None,
                                    None, None, None,
                                    None, None, None, None,
                                    config.capital,
                                    result.total_trades as i32,
                                    result.winning_trades as i32,
                                    result.losing_trades as i32,
                                    result.win_rate,
                                    result.total_pnl,
                                    config.capital + result.total_pnl,
                                    result.profit_factor,
                                    result.sharpe_ratio,
                                    result.max_drawdown_pct,
                                    Some("oto-backtest"),
                                );
                            }

                            // AppState'e yaz
                            let summary = format!(
                                "[{}/{}] trades={} win={:.1}% pnl={:+.2}% dd={:.1}% PF={:.2} Sharpe={:.2} | {}",
                                act_sym, act_intv,
                                result.total_trades,
                                result.win_rate,
                                result.total_pnl_pct,
                                result.max_drawdown_pct,
                                result.profit_factor,
                                result.sharpe_ratio,
                                chrono::Local::now().format("%H:%M"),
                            );
                            if let Ok(mut st) = app_state.lock() {
                                st.last_backtest = Some(summary.clone());
                                st.push_log(format!("🔬 Otonom Backtest: {}", summary));
                                // ── Sembol skoru backtest sonucuyla güncelle ───────────
                                if let Some(cand) = st.symbol_candidates.iter_mut()
                                    .find(|c| c.exchange == act_exch && c.market == act_mkt
                                           && c.symbol == act_sym && c.interval == act_intv)
                                {
                                    cand.win_rate       = result.win_rate;
                                    cand.total_pnl      = result.total_pnl;
                                    cand.total_trades   = result.total_trades;
                                    cand.candle_count   = cand.candle_count.max(result.total_trades + 30);
                                    cand.last_price     = c.last().map(|x| x.close).unwrap_or(cand.last_price);
                                    cand.last_candle_ts = c.last().map(|x| x.timestamp.timestamp()).unwrap_or(cand.last_candle_ts);
                                    // compute_symbol_score ile aynı disqualifier'lar — tutarsız skor divergansını önler
                                    cand.score = if !is_valid_symbol_data(cand.candle_count, &cand.symbol, cand.last_price, cand.last_candle_ts)
                                        || result.profit_factor < 1.0
                                        || result.win_rate < 33.3
                                        || result.max_drawdown_pct > 40.0
                                    {
                                        0.0
                                    } else {
                                        let wn = result.win_rate / 100.0;
                                        let pn = ((result.total_pnl + 500.0) / 1500.0).clamp(0.0, 1.0);
                                        let tc = (result.total_trades as f64 / 20.0).clamp(0.05, 1.0);
                                        (wn * 0.60 + pn * 0.40) * tc
                                    };
                                }
                                st.symbol_candidates.sort_by(|a, b|
                                    b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

                                // ── active_symbol.score'u senkronize et ───────────────
                                // Backtest tamamlandı → symbol_candidates güncel.
                                // active_symbol snapshot'tan stale skor taşıyabilir; düzelt.
                                // Symbol scanner "cur.score" için bu değeri kullanır.
                                if let Some(fresh) = st.symbol_candidates.iter()
                                    .find(|c| c.symbol   == st.active_symbol.symbol
                                           && c.market   == st.active_symbol.market
                                           && c.interval == st.active_symbol.interval)
                                {
                                    st.active_symbol.score = fresh.score;
                                }
                                // Backtest yeni sonuçları DB'ye yazdı → scanner'ı yeniden tetikle.
                                // Download'ın tetiklediği scanner backtest'ten önce çalışmış olabilir;
                                // bu tetikle doğru (güncel) veriyle yeniden çalışmasını garantiler.
                                st.symbol_trigger.store(true, std::sync::atomic::Ordering::Relaxed);

                                // ── AdaptiveBrain: backtest PnL → simülasyon reward ────
                                // Gerçek trade beklemeden evrim öğrenmesini başlat
                                let bt_pnl_pct = result.total_pnl_pct;
                                let sname = st.live_strategy.read().ok()
                                    .map(|s| s.clone()).unwrap_or_else(|| "MA".to_string());
                                if let Some(ref mut brain) = st.controller.adaptive_brain {
                                    let n = c.len().saturating_sub(20);
                                    let closes:  Vec<f64> = c[n..].iter().map(|x| x.close).collect();
                                    let volumes: Vec<f64> = c[n..].iter().map(|x| x.volume).collect();
                                    let regime = brain.detect_market_regime(&closes, &volumes);
                                    brain.learn_from_trade(&regime, &sname, bt_pnl_pct);
                                    st.push_log(format!(
                                        "🧬 Brain simülasyon reward: pnl={:+.2}% strateji={} rejim={:?}",
                                        bt_pnl_pct, sname, regime
                                    ));
                                }
                                // ── Genome fitness: backtest PnL → sıfır gerçek trade bekleme ────
                                // trade_count=0 olan genome'lara backtest sonucunu besle.
                                // Böylece evrim donmaz; gerçek trade olmasa da fitness hesaplanır.
                                if result.total_trades >= 3 {
                                    // Genome fitness: backtest verisiyle besle, sıfır gerçek trade bekleme
                                    let mut genome_log: Option<String> = None;
                                    if let Some(ref mut genome) = st.controller.current_strategy_genome {
                                        if genome.trade_count == 0 {
                                            genome.trade_count    = result.total_trades as usize;
                                            genome.total_pnl_pct  = result.total_pnl_pct;
                                            genome.win_rate       = result.win_rate;
                                            genome.max_drawdown_pct = result.max_drawdown_pct;
                                            genome.sharpe_ratio   = 0.0;
                                            genome.calculate_fitness();
                                            genome_log = Some(format!(
                                                "🧬 Genome fitness backtest'ten: {} → {:.2} (trades={} win={:.1}%)",
                                                genome.id, genome.fitness,
                                                genome.trade_count, genome.win_rate
                                            ));
                                        }
                                    }
                                    if let Some(msg) = genome_log { st.push_log(msg); }
                                    // PopulationManager içindeki genome'ları da güncelle
                                    if let Some(ref mut pop) = st.controller.population_manager {
                                        for genome in pop.current_population.iter_mut() {
                                            if genome.trade_count == 0 {
                                                genome.trade_count    = result.total_trades as usize;
                                                genome.total_pnl_pct  = result.total_pnl_pct;
                                                genome.win_rate       = result.win_rate;
                                                genome.max_drawdown_pct = result.max_drawdown_pct;
                                                genome.calculate_fitness();
                                            }
                                        }
                                        pop.update_population_fitness();
                                    }
                                }
                                // NOT: total_trades backtest sayacından değil, SharedLogger trade parse'dan gelir
                                // ── Otonom SL/TP Güncelle ─────────────────────────────────────
                                let (new_sl, new_tp) = compute_adaptive_sl_tp(
                                    result.win_rate, result.max_drawdown_pct,
                                );
                                st.best_sl = new_sl;
                                st.best_tp = new_tp;
                                // 🔄 live_risk güncelle → loop anında yeni SL/TP alır
                                if let Ok(mut lr) = st.live_risk.write() {
                                    // Per-sembol kayıt (download + backtest için aktif sembol)
                                    lr.per_symbol.insert(act_sym.clone(), (new_sl, new_tp));
                                    // Global fallback güncelle
                                    lr.global_sl = new_sl;
                                    lr.global_tp = new_tp;
                                    lr.global_fast = st.best_fast;
                                    lr.global_slow = st.best_slow;
                                }
                                st.push_log(format!(
                                    "⚙ SL/TP adaptif: SL={:.1}% TP={:.1}% (rr={:.1}x)",
                                    new_sl, new_tp, new_tp / new_sl
                                ));
                                // ── Adaptif Risk Politikası Güncelle ──────────────────
                                let new_policy = compute_adaptive_policy(
                                    config.capital,
                                    result.win_rate,
                                    result.max_drawdown_pct,
                                );
                                let old_policy = st.risk_gate.policy;
                                st.risk_gate.policy = new_policy;
                                st.push_log(format!(
                                    "🛡 Risk adaptif: dd={:.1}% day={:.1}% not=${:.0} conf={:.2}",
                                    new_policy.max_drawdown_pct,
                                    new_policy.max_daily_loss_pct,
                                    new_policy.max_notional_usd,
                                    new_policy.min_model_confidence,
                                ));
                                let _ = old_policy; // eski degeri loglamak istersen kullan
                                // ── Strateji karşılaştırması ──
                                let best_strat = compare_strategies(
                                    &c, config.capital, config.trade_amount,
                                    new_sl, new_tp,
                                    &act_intv,
                                );
                                let old_strat = st.live_strategy.read().ok()
                                    .map(|s| s.clone()).unwrap_or_default();
                                if old_strat != best_strat {
                                    if st.strategy_locked_until <= Instant::now() {
                                        let has_open_pos = st.live_positions.read().ok()
                                            .map(|p| !p.is_empty()).unwrap_or(false);
                                        let note = if has_open_pos { " (açık pos var — yeni girişler için)" } else { "" };
                                        st.push_log(format!(
                                            "🦥 Strateji otomatik değişti: {} → {}{}", old_strat, best_strat, note
                                        ));
                                        if let Ok(mut ls) = st.live_strategy.write() {
                                            *ls = best_strat;
                                        }
                                        st.strategy_locked_until = Instant::now() + Duration::from_secs(600);
                                    } else {
                                        let rem = st.strategy_locked_until.saturating_duration_since(Instant::now()).as_secs();
                                        st.push_log(format!("⏳ Strateji kilidi: {} → {} ({}sn sonra)", old_strat, best_strat, rem));
                                    }
                                }
                                // ── Interval Karşılaştırması ──────────────────────────────────────
                                // Mevcut sembol için tüm interval'leri hızlı backtest et,
                                // en iyi skoru bulan interval'i öner veya otomatik geçiş yap.
                                let (auto_intv, cur_intv_snap) = (st.auto_interval, act_intv.clone());
                                drop(st); // lock bırak — uzun işlem başlıyor

                                const INTV_TEST: &[&str] = &["1m","5m","15m","30m","1h","4h"];
                                let mut intv_scores: Vec<(String, f64, f64, f64)> = vec![];
                                for &test_intv in INTV_TEST {
                                    if stop_signal.load(Ordering::Relaxed) { break; }
                                    let Ok(ic) = database_reader::read_candles(
                                        &config.db_path, &act_exch, &act_mkt, &act_sym,
                                        test_intv, Some(config.backtest_candle_limit),
                                    ) else { continue };
                                    if ic.len() < 30 { continue; }
                                    let ibt_cfg = BacktestConfig {
                                        symbol:            act_sym.clone(),
                                        interval:          test_intv.to_string(),
                                        initial_balance:   config.capital,
                                        max_position_size: config.trade_amount,
                                        take_profit_pct:   new_tp,
                                        stop_loss_pct:     new_sl,
                                        strategy_name:     "MA_CROSSOVER".to_string(),
                                        position_profile:  Some("Balanced".to_string()),
                                        security_profile:  Some("Development".to_string()),
                                        strategy_params:   None,
                                        commission_pct:    0.001,
                                        breakeven_at_rr:   None,
                                        atr_trail_mult:    None,
                                        partial_tp_ratio:  None,
                                    };
                                    let mut ibt = Backtester::new(ibt_cfg);
                                    if let Ok(ir) = ibt.run(&ic) {
                                        if ir.total_trades < 5 { continue; }
                                        // PF < 1.0 interval'i eleme — kötü interval öneri olmasın
                                        let score = if ir.profit_factor < 1.0 || ir.win_rate < 33.3 {
                                            0.0
                                        } else {
                                            let wn = ir.win_rate / 100.0;
                                            let pn = ((ir.total_pnl + 500.0) / 1500.0).clamp(0.0, 1.0);
                                            let tc = (ir.total_trades as f64 / 10.0).clamp(0.1, 1.0);
                                            (wn * 0.60 + pn * 0.40) * tc
                                        };
                                        intv_scores.push((test_intv.to_string(), score, ir.win_rate, ir.total_pnl));
                                    }
                                }

                                // En yüksek skorlu interval
                                intv_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                                let best_intv_opt = intv_scores.first().cloned();

                                if let Ok(mut st) = app_state.lock() {
                                    st.interval_scores = intv_scores;
                                    if let Some((ref bi, bscore, bwr, bpnl)) = best_intv_opt {
                                        let is_better = bi != &cur_intv_snap && bscore > 0.05;
                                        if is_better {
                                            st.push_log(format!(
                                                "💡 Interval önerisi: {} → {} (skor={:.3} win={:.1}% pnl={:+.2}$)",
                                                cur_intv_snap, bi, bscore, bwr, bpnl
                                            ));
                                        }
                                        st.best_interval_rec = Some((bi.clone(), bscore, bwr, bpnl));

                                        // Oto-geçiş: açık pozisyon yoksa ve kullanıcı etkinleştirdiyse
                                        if auto_intv && is_better {
                                            let has_pos = st.live_positions.read().ok()
                                                .map(|p| !p.is_empty()).unwrap_or(false);
                                            if !has_pos {
                                                st.config.interval = bi.clone();
                                                st.active_symbol.interval = bi.clone();
                                                st.loop_restart_trigger.store(true, Ordering::Relaxed);
                                                // Interval değişiminde ML worker'ı anında tetikle
                                                st.ml_trigger.store(true, Ordering::Relaxed);
                                                st.ml_next_run_at = Instant::now();
                                                st.hyperopt_score = 0.0;
                                                if let Ok(mut lr) = st.live_risk.write() {
                                                    lr.hyperopt_score = 0.0;
                                                }
                                                // Interval değişimini diske kaydet (yeniden başlatmada korunsun)
                                                save_oto_config(&st.config);
                                                st.push_log(format!(
                                                    "🔄 Interval oto-geçiş: {} → {} (skor {:.3})",
                                                    cur_intv_snap, bi, bscore
                                                ));
                                            } else {
                                                st.push_log(format!(
                                                    "⏳ Interval geçişi bekleniyor (açık pozisyon): {} → {}",
                                                    cur_intv_snap, bi
                                                ));
                                            }
                                        }
                                    }
                                    // Backtest + SL/TP + risk güncellendi
                                    save_app_snapshot(&st);
                                }
                            }
                        }
                        Err(e) => {
                            if let Ok(mut st) = app_state.lock() {
                                st.push_log(format!("⚠️  Backtest hatası: {}", e));
                            }
                        }
                    }
                }
                Ok(c) => {
                    if let Ok(mut st) = app_state.lock() {
                        st.push_log(format!(
                            "⚠️  Backtest: yetersiz veri ({}/{} {} mum < 30)",
                            act_sym, act_intv, c.len()
                        ));
                    }
                }
                Err(e) => {
                    if let Ok(mut st) = app_state.lock() {
                        st.push_log(format!("⚠️  Backtest DB okuma: {}", e));
                    }
                }
            }
                // Backtest tamamlandı — kilidi serbest bırak
                backtest_running.store(false, Ordering::SeqCst);
            } // if triggered || time_up

            std::thread::sleep(std::time::Duration::from_secs(tick));
            elapsed += tick;
        }
    });
}

// ─── Periyodik ML/AI Worker ───────────────────────────────────────────────────
// Kendi std::thread'inde çalışır:
//  1. DB'den mum yükler → FeatureExtractor ile özellik vektörleri üretir
//  2. LinearRegressor'ı online eğitir (son candle etiket: sonraki fiyat değişimi)
//  3. HyperOpt::random_search() ile en iyi strateji parametrelerini arar
//  4. Sonuçları AppState'e yazar (ML tab'ında gösterilir)
//  5. `ml_trigger` veya `backtest_trigger` AtomicBool'lar anlık çalışmayı tetikler

fn run_ml_worker(
    app_state:        Arc<Mutex<AppState>>,
    stop_signal:      Arc<AtomicBool>,
    ml_worker_running: Arc<AtomicBool>,
    config:           OtoConfig,
) {
    std::thread::spawn(move || {
        use std::sync::atomic::Ordering;

        // İlk çalışmadan önce bekle; ml_trigger set ise (önbellek yok/ilk çalışma) hemen başla
        {
            let immediate = app_state.lock().unwrap().ml_trigger.load(Ordering::Relaxed);
            if !immediate {
                std::thread::sleep(Duration::from_secs(45));
            }
            // Kısa sync fırsatı: startup_sync_gap_fill'ın DB'ye yazması için 2s bekle
            else {
                std::thread::sleep(Duration::from_secs(2));
            }
        }

        let train_every_secs = config.backtest_every_mins * 60 + 30; // backtest'ten 30s sonra

        // İlk çalışmayı hemen başlat (elapsed = train_every_secs → ilk iterasyonda çalışır)
        let mut elapsed = train_every_secs;
        let tick = 1u64;

        loop {
            if stop_signal.load(Ordering::Relaxed) {
                break;
            }

            // ml_trigger veya zaman doldu mu?
            let triggered = {
                let st = app_state.lock().unwrap();
                st.ml_trigger.swap(false, Ordering::Relaxed)
            };
            let time_up = elapsed >= train_every_secs;

            if triggered || time_up {
                elapsed = 0;

                // Stop sinyali geldiyse ML başlatma (eski worker çift çalışmasın)
                if stop_signal.load(Ordering::Relaxed) { break; }

                // Çift ML worker önleme — backtest_running ile aynı pattern
                // compare_exchange: false → true başarısızsa başka worker zaten çalışıyor, atla
                if ml_worker_running.compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed).is_err() {
                    if triggered {
                        if let Ok(mut st) = app_state.lock() {
                            st.push_log("⏳ ML worker zaten çalışıyor — tamamlanınca sonuç görünür".to_string());
                        }
                    }
                    std::thread::sleep(Duration::from_secs(tick));
                    elapsed += tick;
                    continue;
                }

                // Aktif hedefi al
                let (act_exch, act_mkt, act_sym, act_intv) = {
                    if let Ok(st) = app_state.lock() {
                        st.active_trade_target()
                    } else {
                        (config.exchange.clone(), config.market.clone(),
                         config.symbol.clone(), config.interval.clone())
                    }
                };

                // DB'den mum yükle
                let candles = database_reader::read_candles(
                    &config.db_path,
                    &act_exch,
                    &act_mkt,
                    &act_sym,
                    &act_intv,
                    Some(config.backtest_candle_limit.max(200)),
                );

                let ml_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    candles
                }));
                let candles = match ml_result {
                    Ok(c) => c,
                    Err(_) => {
                        if let Ok(mut st) = app_state.lock() {
                            st.push_log("⚠️  ML Worker: beklenmedik hata (panic), bir sonraki çevrimde yeniden denenecek".to_string());
                        }
                        std::thread::sleep(Duration::from_secs(tick));
                        elapsed += tick;
                        continue;
                    }
                };
                match candles {
                    Ok(c) if c.len() >= 30 => {
                        // ML çalıştığını TUI'ye bildir
                        {
                            let lr_arc = clone_live_risk(&app_state);
                            if let Ok(mut lr) = lr_arc.write() { lr.ml_running = true; };
                        }
                        // ── 1. ML Tahmini ──────────────────────────────────
                        let predictor = MLSignalPredictor::new(0.25);
                        let prediction = predictor.predict(&c).unwrap_or_else(|_| {
                            use memos_trading_core::robot::ml_engine::MLSignalPrediction;
                            use memos_trading_core::robot::ml_engine::FeatureImportance;
                            use memos_trading_core::types::Signal;
                            MLSignalPrediction {
                                signal: Signal::Hold,
                                confidence: 0.0,
                                ml_score: 0.0,
                                feature_importance: FeatureImportance {
                                    rsi_impact: 0.0, macd_impact: 0.0,
                                    bollinger_impact: 0.0, sma_impact: 0.0,
                                    momentum_impact: 0.0, volatility_impact: 0.0,
                                },
                            }
                        });

                        // ── 2. Eğitim: (feature, hedef) çiftleri oluştur ──
                        // LOOKAHEAD bar toplam getirisi kullanılır (ortalamayla bölmeden).
                        // LR: sürekli getiri hedefi [-1,+1] — regression için uygun.
                        // GBT: işaret tabanlı sınıflandırma hedefi {-1, 0, +1} — ağaç modeli için ideal.
                        //      Eşik altındaki nötr barlar atlanır → net yönsel sinyal, sıfıra yakın çıkış engellenir.
                        const LOOKAHEAD: usize = 3;
                        // GBT sınıflandırma eşiği: mutlak getiri bu değerin altındaysa veri noktası atlanır.
                        // Kripto için tipik: 0.3% (gürültüden arındırır, net trend barlarını korur).
                        const GBT_THRESHOLD: f64 = 0.003;
                        use memos_trading_core::robot::ml_engine::linear_regressor::N_FEATURES;
                        let mut regressor = LinearRegressor::with_defaults();
                        let mut train_data    = Vec::new();
                        let mut gbt_cls_data: Vec<([f64; N_FEATURES], f64)> = Vec::new();
                        for i in 0..c.len().saturating_sub(LOOKAHEAD) {
                            let fv = FeatureExtractor::extract(&c[..=i]);
                            let forward_return = (c[i + LOOKAHEAD].close - c[i].close)
                                / c[i].close; // toplam getiri (LOOKAHEAD barı)
                            // LR hedefi: sürekli normalize getiri
                            let lr_target = (forward_return / LOOKAHEAD as f64).clamp(-1.0, 1.0);
                            train_data.push((fv.clone(), lr_target));
                            // GBT hedefi: yalnızca güçlü yönlü barları al → sıfır çıkış sorunu çözülür
                            if forward_return.abs() > GBT_THRESHOLD {
                                let gbt_target = if forward_return > 0.0 { 1.0_f64 } else { -1.0_f64 };
                                gbt_cls_data.push((fv.normalize().to_array(), gbt_target));
                            }
                        }
                        let epochs = 3;
                        let lr = 0.001;
                        regressor.train(&train_data, epochs, lr);

                        // ── 2c. GBT Auto-Tune + Eğitim + Ensemble Diversity ─
                        // gbt_cls_data: sınıflandırma hedefleri ({-1, +1}) ile GBT eğitilir.
                        // raw_data: LR hedefleriyle normalize edilmiş veri; diversity hesabı için kullanılır.
                        // Grid search + train paniklerse catch_unwind yakalar, worker ölmez.
                        let raw_data: Vec<([f64; N_FEATURES], f64)> =
                            train_data.iter()
                                .map(|(fv, t)| (fv.normalize().to_array(), *t))
                                .collect();

                        let (gbt_score, ensemble_agreement) = {
                            let gbt_ref = &gbt_cls_data;
                            let raw_ref = &raw_data;
                            let reg_ref = &regressor;
                            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                // Grid search — en iyi parametreyi bul (18 kombinasyon)
                                let best_params = gbt_grid_search(gbt_ref);
                                let mut gbt = if let Some(ref bp) = best_params {
                                    GradientBoostedTrees::new(bp.n_estimators, bp.learning_rate, bp.max_depth)
                                } else {
                                    GradientBoostedTrees::with_defaults()
                                };
                                gbt.train(gbt_ref);

                                let score = if gbt.is_ready() && !gbt_ref.is_empty() {
                                    // Son raw_data örneği veya gbt_ref son örneği — raw_data her zaman dolu
                                    Some(gbt.predict_raw(&raw_ref[raw_ref.len() - 1].0))
                                } else {
                                    None
                                };

                                // Ensemble diversity — aynı (tuned) GBT kullanılır, voter ile tutarlı
                                let diversity = if gbt.is_ready() && !raw_ref.is_empty() {
                                    let agree = raw_ref.iter().filter(|(x, _)| {
                                        let mut lr_raw = reg_ref.bias;
                                        for (i, w) in reg_ref.weights.iter().enumerate() { lr_raw += w * x[i]; }
                                        let lr_sign  = lr_raw > 0.0;
                                        let gbt_sign = gbt.predict_raw(x) > 0.0;
                                        lr_sign == gbt_sign
                                    }).count();
                                    agree as f64 / raw_ref.len() as f64
                                } else {
                                    0.0
                                };

                                (score, diversity)
                            }));
                            match result {
                                Ok(v) => v,
                                Err(_) => {
                                    if let Ok(mut st) = app_state.lock() {
                                        st.push_log("⚠️  ML Worker: GBT grid search panikledi, varsayılan değerler kullanılıyor".to_string());
                                    }
                                    (None, 0.0)
                                }
                            }
                        };

                        // ── 2d. OOS Kalite Raporu ──────────────────────────
                        // Walk-forward 3 fold: her fold'da eğitim penceresi büyür, OOS sabit.
                        // fold_sz = n/5 → min_tr = 2×fold_sz başlangıç penceresi (n'in %40'ı).
                        // (+2 kasıtlı: 3 fold + 2 = 5 parça; 2 parça başlangıç train, 3 parça OOS)
                        // catch_unwind: OOS paniklerse tüm ML worker ölmez; sıfır değerlerle devam.
                        let train_data_snapshot: Vec<(memos_trading_core::robot::ml_engine::feature_extractor::FeatureVector, f64)> =
                            train_data.iter().cloned().collect();
                        let regressor_snapshot = regressor.clone();
                        let (oos_wr, oos_ar, oos_bc, oos_folds) = {
                            let oos_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                let n       = train_data_snapshot.len();
                                let n_folds = 3usize;
                                let fold_sz = (n / (n_folds + 2)).max(1);
                                let min_tr  = fold_sz * 2;
                                let mut wins = 0usize; let mut total = 0usize;
                                let mut sum_ret = 0.0f64;
                                let mut fold_scores_arr = [0.0f64; 3];
                                for fold in 0..n_folds {
                                    let tr_end = (min_tr + fold * fold_sz).min(n.saturating_sub(fold_sz));
                                    let oos_end = (tr_end + fold_sz).min(n);
                                    if oos_end <= tr_end { continue; }
                                    let mut m = regressor_snapshot.clone();
                                    m.train(&train_data_snapshot[..tr_end], 3, 0.001);
                                    let mut fold_win = 0usize; let mut fold_tot = 0usize;
                                    let mut fold_ret = 0.0f64;
                                    for (fv, target) in &train_data_snapshot[tr_end..oos_end] {
                                        let pred = m.predict_score(fv);
                                        if (pred > 0.0) == (*target > 0.0) { fold_win += 1; }
                                        fold_ret += *target;
                                        fold_tot += 1;
                                    }
                                    if fold_tot > 0 {
                                        fold_scores_arr[fold] = fold_win as f64 / fold_tot as f64 * 100.0;
                                        wins  += fold_win;
                                        total += fold_tot;
                                        sum_ret += fold_ret;
                                    }
                                }
                                let wr = if total > 0 { wins as f64 / total as f64 * 100.0 } else { 0.0 };
                                let ar = if total > 0 { sum_ret / total as f64 * 100.0 } else { 0.0 };
                                (wr, ar, total, fold_scores_arr)
                            }));
                            match oos_result {
                                Ok(v) => v,
                                Err(_) => {
                                    if let Ok(mut st) = app_state.lock() {
                                        st.push_log("⚠️  ML Worker: OOS hesaplama panikledi, sıfır değerler kullanılıyor".to_string());
                                    }
                                    (0.0, 0.0, 0, [0.0f64; 3])
                                }
                            }
                        };
                        // OOS tanı logu — oos_bc=0 ise uyarı bas
                        {
                            let bc_msg = if oos_bc == 0 {
                                format!("⚠️  ML OOS: BarCount=0 (eğitim={} örnek) — yetersiz veri veya atlanan fold",
                                    train_data.len())
                            } else {
                                format!("✅ ML OOS: BarCount={} WinRate={:.1}% (eğitim={} örnek)",
                                    oos_bc, oos_wr, train_data.len())
                            };
                            if let Ok(mut st) = app_state.lock() { st.push_log(bc_msg); }
                        }
                        // Tek write — LR + GBT + OOS tümü birlikte yazılır.
                        // Robotic loop asla yarım ensemble durumu görmez.
                        {
                            let lr_arc2 = clone_live_risk(&app_state);
                            if let Ok(mut lrm) = lr_arc2.write() {
                                lrm.ml_weights      = Some(regressor.weights);
                                lrm.ml_bias_trained = regressor.bias;
                                lrm.gbt_last_score      = gbt_score;
                                lrm.oos_win_rate        = oos_wr;
                                lrm.oos_avg_return      = oos_ar;
                                lrm.oos_bar_count       = oos_bc;
                                lrm.oos_fold_scores     = oos_folds;
                                lrm.ensemble_agreement  = ensemble_agreement;
                                lrm.ml_running          = false; // eğitim bitti
                            };
                        }

                        // ── 3. HyperOpt — MA Crossover fast/slow backtester ile optimize ──
                        let param_grid: Vec<StrategyParams> = (5usize..=20).step_by(2)
                            .flat_map(|fast| (fast + 5..=50).step_by(5).map(move |slow| StrategyParams {
                                fast: Some(fast),
                                slow: Some(slow),
                                period: Some(14),
                                overbought: Some(70.0),
                                oversold: Some(30.0),
                                ..Default::default()
                            }))
                            .collect();

                        // MA grid base config (mevcut best_sl/best_tp kullan)
                        let hopt_base_cfg = {
                            let st = app_state.lock().unwrap();
                            BacktestConfig {
                                symbol:            format!("{}_HOPT", act_sym),
                                interval:          act_intv.clone(),
                                initial_balance:   st.equity,
                                max_position_size: if config.trade_amount > 0.0 { config.trade_amount } else { 0.01 },
                                take_profit_pct:   st.best_tp.max(1.0),
                                stop_loss_pct:     st.best_sl.max(0.5),
                                strategy_name:     "MA_CROSSOVER".to_string(),
                                position_profile:  None,
                                security_profile:  None,
                                strategy_params:   None,
                                commission_pct:    0.001,
                                breakeven_at_rr:   None,
                                atr_trail_mult:    None,
                                partial_tp_ratio:  None,
                            }
                        };
                        // HyperOpt → Option<HyperOptResult>; None durumunda sıfır değerli fallback
                        let hopt_opt = HyperOpt::grid_search(&c, &param_grid, &hopt_base_cfg);
                        // Borrow hatası olmaması için alanları kopyala
                        let (hopt_best_fast, hopt_best_slow, hopt_best_score_raw) = hopt_opt.as_ref()
                            .map(|r| (r.best_params.fast.unwrap_or(5),
                                      r.best_params.slow.unwrap_or(20),
                                      r.best_score))
                            .unwrap_or((5, 20, 0.0));
                        // Sentinel yerine gerçek Option türetilmiş struct — backward compat shim
                        struct _HoptShim { fast: usize, slow: usize, score: f64 }
                        let hopt_result = _HoptShim { fast: hopt_best_fast, slow: hopt_best_slow, score: hopt_best_score_raw };

                        // ── 3b. Tüm Strateji HyperOpt (backtest tabanlı) ──────────────
                        // Strateji ne olursa olsun tüm stratejiler her seferinde optimize edilir.
                        // Strateji değiştiğinde hazır parametreler hemen devreye girer.
                        let (opt_capital, opt_amount, opt_sl, opt_tp) = {
                            let st = app_state.lock().unwrap();
                            (st.equity, config.trade_amount, st.best_sl.max(0.5), st.best_tp.max(1.0))
                        };

                        // Hassas kompozit skor: win_rate + profit_factor + sharpe + max_drawdown
                        let bt_score = |r: &memos_trading_core::robot::backtester::BacktestResult| -> f64 {
                            score_backtest_result(r.win_rate, r.profit_factor, r.sharpe_ratio, r.max_drawdown_pct)
                        };
                        let make_cfg = |name: &str, params: Option<StrategyParams>| -> BacktestConfig {
                            BacktestConfig {
                                symbol:            format!("{}_OPT", name),
                                interval:          act_intv.clone(),
                                initial_balance:   opt_capital,
                                max_position_size: opt_amount,
                                take_profit_pct:   opt_tp,
                                stop_loss_pct:     opt_sl,
                                strategy_name:     name.to_string(),
                                position_profile:  None,
                                security_profile:  None,
                                strategy_params:   params,
                                commission_pct:    0.001, // %0.1 giriş + %0.1 çıkış
                                breakeven_at_rr:   None,
                                atr_trail_mult:    None,
                                partial_tp_ratio:  None,
                            }
                        };

                        // RSI: 4 period × 4 OB × 4 OS = 64 kombinasyon
                        let rsi_opt: Option<(usize, f64, f64, f64)> = {
                            let mut best = f64::MIN;
                            let (mut bp, mut bo, mut bos) = (14usize, 70.0f64, 30.0f64);
                            for period in [7usize, 10, 14, 21] {
                                for ob in [60.0f64, 65.0, 70.0, 75.0] {
                                    for os in [20.0f64, 25.0, 30.0, 35.0] {
                                        if os >= ob { continue; }
                                        let cfg = make_cfg("RSI", Some(StrategyParams {
                                            period: Some(period),
                                            overbought: Some(ob),
                                            oversold: Some(os),
                                            ..Default::default()
                                        }));
                                        if let Ok(r) = Backtester::new(cfg).run(&c) {
                                            if r.total_trades >= 3 {
                                                let sc = bt_score(&r);
                                                if sc > best { best = sc; bp = period; bo = ob; bos = os; }
                                            }
                                        }
                                    }
                                }
                            }
                            Some((bp, bo, bos, best))
                        };

                        // BB: 4 period × 3 std_dev = 12 kombinasyon
                        let bb_opt: Option<(usize, f64, f64)> = {
                            let mut best = f64::MIN;
                            let (mut bp, mut bs) = (20usize, 2.0f64);
                            for period in [10usize, 15, 20, 25] {
                                for std_dev in [1.5f64, 2.0, 2.5] {
                                    let cfg = make_cfg("BOLLINGER", Some(StrategyParams {
                                        bb_period: Some(period),
                                        std_dev: Some(std_dev),
                                        ..Default::default()
                                    }));
                                    if let Ok(r) = Backtester::new(cfg).run(&c) {
                                        if r.total_trades >= 3 {
                                            let sc = bt_score(&r);
                                            if sc > best { best = sc; bp = period; bs = std_dev; }
                                        }
                                    }
                                }
                            }
                            Some((bp, bs, best))
                        };

                        // STOCHASTIC: K=6 en iyi (DB kanıtlı), OB/OS grid = 9 kombinasyon
                        let stoch_opt: Option<(usize, f64, f64, f64)> = {
                            let mut best = f64::MIN;
                            let (mut bk, mut bob, mut bos) = (6usize, 70.0f64, 20.0f64);
                            for k in [6usize, 9, 14] {
                                for ob in [65.0f64, 70.0, 80.0] {
                                    for os in [15.0f64, 20.0, 25.0] {
                                        if os >= ob { continue; }
                                        let cfg = make_cfg("STOCHASTIC", Some(StrategyParams {
                                            period: Some(k),
                                            overbought: Some(ob),
                                            oversold: Some(os),
                                            ..Default::default()
                                        }));
                                        if let Ok(r) = Backtester::new(cfg).run(&c) {
                                            if r.total_trades >= 3 {
                                                let sc = bt_score(&r);
                                                if sc > best { best = sc; bk = k; bob = ob; bos = os; }
                                            }
                                        }
                                    }
                                }
                            }
                            Some((bk, bob, bos, best))
                        };

                        // MACD: 2 fast × 2 slow × 2 signal = 8 kombinasyon
                        let macd_opt: Option<(usize, usize, usize, f64)> = {
                            let mut best = f64::MIN;
                            let (mut bf, mut bsl, mut bsig) = (12usize, 26usize, 9usize);
                            for fast in [8usize, 12] {
                                for slow in [21usize, 26] {
                                    if fast >= slow { continue; }
                                    for signal in [7usize, 9] {
                                        let cfg = make_cfg("MACD", Some(StrategyParams {
                                            fast_period: Some(fast),
                                            slow_period: Some(slow),
                                            signal_period: Some(signal),
                                            ..Default::default()
                                        }));
                                        if let Ok(r) = Backtester::new(cfg).run(&c) {
                                            if r.total_trades >= 3 {
                                                let sc = bt_score(&r);
                                                if sc > best { best = sc; bf = fast; bsl = slow; bsig = signal; }
                                            }
                                        }
                                    }
                                }
                            }
                            Some((bf, bsl, bsig, best))
                        };

                        // EMA: 4 fast × 4 slow = 12 kombinasyon
                        let ema_opt: Option<(usize, usize, f64)> = {
                            let mut best = f64::MIN;
                            let (mut bf, mut bs) = (5usize, 20usize);
                            for fast in [5usize, 8, 10, 13] {
                                for slow in [20usize, 26, 34, 50] {
                                    if fast >= slow { continue; }
                                    let cfg = make_cfg("EMA", Some(StrategyParams {
                                        fast: Some(fast), slow: Some(slow), ..Default::default()
                                    }));
                                    if let Ok(r) = Backtester::new(cfg).run(&c) {
                                        if r.total_trades >= 3 {
                                            let sc = bt_score(&r);
                                            if sc > best { best = sc; bf = fast; bs = slow; }
                                        }
                                    }
                                }
                            }
                            Some((bf, bs, best))
                        };

                        // DONCHIAN: 5 period kombinasyonu
                        let donchian_opt: Option<(usize, f64)> = {
                            let mut best = f64::MIN;
                            let mut bp = 20usize;
                            for period in [10usize, 15, 20, 30, 50] {
                                let cfg = make_cfg("DONCHIAN", Some(StrategyParams {
                                    period: Some(period), ..Default::default()
                                }));
                                if let Ok(r) = Backtester::new(cfg).run(&c) {
                                    if r.total_trades >= 3 {
                                        let sc = bt_score(&r);
                                        if sc > best { best = sc; bp = period; }
                                    }
                                }
                            }
                            Some((bp, best))
                        };

                        // WILLIAMS_R: 4 period kombinasyonu
                        let williams_opt: Option<(usize, f64)> = {
                            let mut best = f64::MIN;
                            let mut bp = 14usize;
                            for period in [7usize, 10, 14, 21] {
                                let cfg = make_cfg("WILLIAMS", Some(StrategyParams {
                                    period: Some(period), ..Default::default()
                                }));
                                if let Ok(r) = Backtester::new(cfg).run(&c) {
                                    if r.total_trades >= 3 {
                                        let sc = bt_score(&r);
                                        if sc > best { best = sc; bp = period; }
                                    }
                                }
                            }
                            Some((bp, best))
                        };

                        // CCI: 4 period kombinasyonu
                        let cci_opt: Option<(usize, f64)> = {
                            let mut best = f64::MIN;
                            let mut bp = 20usize;
                            for period in [10usize, 14, 20, 30] {
                                let cfg = make_cfg("CCI", Some(StrategyParams {
                                    period: Some(period), ..Default::default()
                                }));
                                if let Ok(r) = Backtester::new(cfg).run(&c) {
                                    if r.total_trades >= 3 {
                                        let sc = bt_score(&r);
                                        if sc > best { best = sc; bp = period; }
                                    }
                                }
                            }
                            Some((bp, best))
                        };

                        // STOCH_RSI: 4 period kombinasyonu
                        let stoch_rsi_opt: Option<(usize, f64)> = {
                            let mut best = f64::MIN;
                            let mut bp = 14usize;
                            for period in [7usize, 10, 14, 21] {
                                let cfg = make_cfg("STOCH_RSI", Some(StrategyParams {
                                    period: Some(period), ..Default::default()
                                }));
                                if let Ok(r) = Backtester::new(cfg).run(&c) {
                                    if r.total_trades >= 3 {
                                        let sc = bt_score(&r);
                                        if sc > best { best = sc; bp = period; }
                                    }
                                }
                            }
                            Some((bp, best))
                        };

                        // SUPERTREND: 3 period × 3 mult = 9 kombinasyon
                        let supertrend_opt: Option<(usize, f64, f64)> = {
                            let mut best = f64::MIN;
                            let (mut bp, mut bm) = (10usize, 3.0f64);
                            for period in [7usize, 10, 14] {
                                for mult in [2.0f64, 3.0, 4.0] {
                                    let cfg = make_cfg("SUPERTREND", Some(StrategyParams {
                                        period: Some(period), std_dev: Some(mult), ..Default::default()
                                    }));
                                    if let Ok(r) = Backtester::new(cfg).run(&c) {
                                        if r.total_trades >= 3 {
                                            let sc = bt_score(&r);
                                            if sc > best { best = sc; bp = period; bm = mult; }
                                        }
                                    }
                                }
                            }
                            Some((bp, bm, best))
                        };

                        // ICT_FVG: 4 lookback kombinasyonu
                        let ict_fvg_opt: Option<(usize, f64)> = {
                            let mut best = f64::MIN;
                            let mut bl = 5usize;
                            for lookback in [3usize, 5, 8, 15] {
                                let cfg = make_cfg("ICT_FVG", Some(StrategyParams {
                                    period: Some(lookback), ..Default::default()
                                }));
                                if let Ok(r) = Backtester::new(cfg).run(&c) {
                                    if r.total_trades >= 3 {
                                        let sc = bt_score(&r);
                                        if sc > best { best = sc; bl = lookback; }
                                    }
                                }
                            }
                            Some((bl, best))
                        };

                        // SMC: 4 swing lookback kombinasyonu
                        let smc_opt: Option<(usize, f64)> = {
                            let mut best = f64::MIN;
                            let mut bl = 10usize;
                            for swing_lb in [5usize, 8, 10, 20] {
                                let cfg = make_cfg("SMC", Some(StrategyParams {
                                    period: Some(swing_lb), ..Default::default()
                                }));
                                if let Ok(r) = Backtester::new(cfg).run(&c) {
                                    if r.total_trades >= 3 {
                                        let sc = bt_score(&r);
                                        if sc > best { best = sc; bl = swing_lb; }
                                    }
                                }
                            }
                            Some((bl, best))
                        };

                        // PRICE_ACTION: parametre yok, tek çalıştır
                        let price_action_score: f64 = {
                            let cfg = make_cfg("PRICE_ACTION", None);
                            Backtester::new(cfg).run(&c)
                                .map(|r| if r.total_trades >= 3 { bt_score(&r) } else { f64::MIN })
                                .unwrap_or(f64::MIN)
                        };

                        // ICT_OB: Order Block — 4 swing lookback kombinasyonu
                        let ict_ob_opt: Option<(usize, f64)> = {
                            let mut best = f64::MIN;
                            let mut bl = 10usize;
                            for swing_lb in [5usize, 8, 10, 15] {
                                let cfg = make_cfg("ICT_OB", Some(StrategyParams {
                                    period: Some(swing_lb), ..Default::default()
                                }));
                                if let Ok(r) = Backtester::new(cfg).run(&c) {
                                    if r.total_trades >= 3 {
                                        let sc = bt_score(&r);
                                        if sc > best { best = sc; bl = swing_lb; }
                                    }
                                }
                            }
                            Some((bl, best))
                        };

                        // ICT_SWEEP: Liquidity Sweep — 4 lookback kombinasyonu
                        let ict_sweep_opt: Option<(usize, f64)> = {
                            let mut best = f64::MIN;
                            let mut bl = 20usize;
                            for lb in [10usize, 15, 20, 30] {
                                let cfg = make_cfg("ICT_SWEEP", Some(StrategyParams {
                                    period: Some(lb), ..Default::default()
                                }));
                                if let Ok(r) = Backtester::new(cfg).run(&c) {
                                    if r.total_trades >= 3 {
                                        let sc = bt_score(&r);
                                        if sc > best { best = sc; bl = lb; }
                                    }
                                }
                            }
                            Some((bl, best))
                        };

                        // ICT_OTE: Optimal Trade Entry — 4 swing lookback kombinasyonu
                        let ict_ote_opt: Option<(usize, f64)> = {
                            let mut best = f64::MIN;
                            let mut bl = 15usize;
                            for swing_lb in [10usize, 15, 20, 25] {
                                let cfg = make_cfg("ICT_OTE", Some(StrategyParams {
                                    period: Some(swing_lb), ..Default::default()
                                }));
                                if let Ok(r) = Backtester::new(cfg).run(&c) {
                                    if r.total_trades >= 3 {
                                        let sc = bt_score(&r);
                                        if sc > best { best = sc; bl = swing_lb; }
                                    }
                                }
                            }
                            Some((bl, best))
                        };

                        // ICT_KILLZONE + ICT_COMPOSITE: parametre yok, tek çalıştır
                        let ict_killzone_score: f64 = {
                            let cfg = make_cfg("ICT_KILLZONE", None);
                            Backtester::new(cfg).run(&c)
                                .map(|r| if r.total_trades >= 3 { bt_score(&r) } else { f64::MIN })
                                .unwrap_or(f64::MIN)
                        };
                        let ict_composite_score: f64 = {
                            let cfg = make_cfg("ICT_COMPOSITE", None);
                            Backtester::new(cfg).run(&c)
                                .map(|r| if r.total_trades >= 3 { bt_score(&r) } else { f64::MIN })
                                .unwrap_or(f64::MIN)
                        };

                        // MA_CROSSOVER: tek çalıştır (HyperOpt fast/slow zaten var)
                        let ma_score: f64 = {
                            let cfg = make_cfg("MA_CROSSOVER", Some(StrategyParams {
                                fast: Some(hopt_result.fast),
                                slow: Some(hopt_result.slow),
                                ..Default::default()
                            }));
                            Backtester::new(cfg).run(&c)
                                .map(|r| if r.total_trades >= 3 { bt_score(&r) } else { f64::MIN })
                                .unwrap_or(f64::MIN)
                        };

                        // ── 4. AppState güncelle ───────────────────────────
                        // Debug format "Hold"/"Buy"/"Sell" → normalize "HOLD"/"BUY"/"SELL"
                        let signal_str = format!("{:?}", prediction.signal).to_uppercase();
                        let train_count_delta = train_data.len() as u64;
                        let rsi_summary = rsi_opt.map(|(p, ob, os, sc)| {
                            let sc = if sc > f64::MIN / 2.0 { sc } else { 0.0 };
                            format!(" | RSI period={} OB={:.0}% OS={:.0}% sc={:.4}", p, ob, os, sc)
                        }).unwrap_or_default();
                        let bb_summary = bb_opt.map(|(p, s, sc)| {
                            let sc = if sc > f64::MIN / 2.0 { sc } else { 0.0 };
                            format!(" | BB period={} σ={:.1} sc={:.4}", p, s, sc)
                        }).unwrap_or_default();
                        let macd_summary = macd_opt.map(|(f, sl, sig, sc)| {
                            let sc = if sc > f64::MIN / 2.0 { sc } else { 0.0 };
                            format!(" | MACD {}/{}/{} sc={:.4}", f, sl, sig, sc)
                        }).unwrap_or_default();
                        let gbt_score_str = gbt_score.map(|s| format!(" GBT={:+.3}", s)).unwrap_or_default();
                        let hopt_best_score = if hopt_result.score > f64::MIN / 2.0 { hopt_result.score } else { 0.0 };
                        let summary = format!(
                            "[{}/{}] sinyal={} conf={:.2} skor={:.3}{} OOS={:.0}% | MA fast={} slow={} sc={:.4} | eğitim={} veri | {}{}{}{}",
                            act_sym, act_intv,
                            signal_str,
                            prediction.confidence,
                            prediction.ml_score,
                            gbt_score_str,
                            oos_wr,
                            hopt_result.fast,
                            hopt_result.slow,
                            hopt_best_score,
                            train_data.len(),
                            chrono::Local::now().format("%H:%M"),
                            rsi_summary, bb_summary, macd_summary,
                        );

                        if let Ok(mut st) = app_state.lock() {
                            // HyperOpt ve train istatistikleri her zaman güncellenir
                            st.ml_train_count += train_count_delta;
                            st.best_fast      = hopt_result.fast;
                            st.best_slow      = hopt_result.slow;
                            st.hyperopt_score = hopt_best_score;
                            st.last_ml_train    = Some(summary.clone());
                            st.last_ml_train_at = Some(chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string());
                            st.ml_next_run_at = Instant::now() + Duration::from_secs(train_every_secs);
                            // ML → p5Ana otomatik zincir: ML biter bitmez p5 analizi tetikle
                            st.p5_trigger.store(true, Ordering::Relaxed);
                            // Tüm strateji HyperOpt sonuçlarını AppState'e yaz
                            if let Some((p, ob, os, _)) = rsi_opt {
                                st.best_rsi_period = p;
                                st.best_rsi_ob     = ob;
                                st.best_rsi_os     = os;
                            }
                            if let Some((p, s, _)) = bb_opt {
                                st.best_bb_period  = p;
                                st.best_bb_std_dev = s;
                            }
                            if let Some((f, sl, sig, _)) = macd_opt {
                                st.best_macd_fast   = f;
                                st.best_macd_slow   = sl;
                                st.best_macd_signal = sig;
                            }
                            if let Some((k, ob, os, _)) = stoch_opt {
                                st.best_stoch_k  = k;
                                st.best_stoch_ob = ob;
                                st.best_stoch_os = os;
                            }
                            if let Some((f, s, _)) = ema_opt {
                                st.best_ema_fast = f;
                                st.best_ema_slow = s;
                            }
                            if let Some((p, _)) = donchian_opt  { st.best_donchian_period  = p; }
                            if let Some((p, _)) = williams_opt  { st.best_williams_period  = p; }
                            if let Some((p, _)) = cci_opt       { st.best_cci_period       = p; }
                            if let Some((p, _)) = stoch_rsi_opt { st.best_stoch_rsi_period = p; }
                            if let Some((p, m, _)) = supertrend_opt {
                                st.best_supertrend_period = p;
                                st.best_supertrend_mult   = m;
                            }
                            if let Some((l, _)) = ict_fvg_opt { st.best_ict_fvg_lookback = l; }
                            if let Some((l, _)) = smc_opt     { st.best_smc_swing_lb     = l; }

                            // En iyi strateji: tüm 13 stratejinin backtest skorunu karşılaştır
                            {
                                let scores: &[(&str, f64)] = &[
                                    ("MA_CROSSOVER",  ma_score),
                                    ("RSI",           rsi_opt.map(|x| x.3).unwrap_or(f64::MIN)),
                                    ("BOLLINGER",     bb_opt.map(|x| x.2).unwrap_or(f64::MIN)),
                                    ("MACD",          macd_opt.map(|x| x.3).unwrap_or(f64::MIN)),
                                    ("STOCHASTIC",    stoch_opt.map(|x| x.3).unwrap_or(f64::MIN)),
                                    ("EMA",           ema_opt.map(|x| x.2).unwrap_or(f64::MIN)),
                                    ("DONCHIAN",      donchian_opt.map(|x| x.1).unwrap_or(f64::MIN)),
                                    ("WILLIAMS",      williams_opt.map(|x| x.1).unwrap_or(f64::MIN)),
                                    ("CCI",           cci_opt.map(|x| x.1).unwrap_or(f64::MIN)),
                                    ("STOCH_RSI",     stoch_rsi_opt.map(|x| x.1).unwrap_or(f64::MIN)),
                                    ("SUPERTREND",    supertrend_opt.map(|x| x.2).unwrap_or(f64::MIN)),
                                    ("ICT_FVG",       ict_fvg_opt.map(|x| x.1).unwrap_or(f64::MIN)),
                                    ("SMC",           smc_opt.map(|x| x.1).unwrap_or(f64::MIN)),
                                    ("PRICE_ACTION",  price_action_score),
                                    ("ICT_OB",        ict_ob_opt.map(|x| x.1).unwrap_or(f64::MIN)),
                                    ("ICT_SWEEP",     ict_sweep_opt.map(|x| x.1).unwrap_or(f64::MIN)),
                                    ("ICT_OTE",       ict_ote_opt.map(|x| x.1).unwrap_or(f64::MIN)),
                                    ("ICT_KILLZONE",  ict_killzone_score),
                                    ("ICT_COMPOSITE", ict_composite_score),
                                ];
                                if let Some((name, _)) = scores.iter()
                                    .filter(|(_, sc)| *sc > f64::MIN)
                                    .max_by(|a,b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                                {
                                    st.best_strategy_name = Some(name.to_string());
                                }
                            }
                            // 🔄 live_risk güncelle → loop tüm strateji parametrelerini anında alır
                            // Skor ≤ 0 ise MA parametrelerini yazmıyoruz — negatif/sıfır skor geçersiz optimizasyona işaret eder
                            let (fast, slow, rp, rob, ros, bbp, bbs, mf, ms, msig) = (
                                st.best_fast, st.best_slow,
                                st.best_rsi_period, st.best_rsi_ob, st.best_rsi_os,
                                st.best_bb_period, st.best_bb_std_dev,
                                st.best_macd_fast, st.best_macd_slow, st.best_macd_signal,
                            );
                            if let Ok(mut lr) = st.live_risk.write() {
                                // MA parametreleri yalnızca negatif skor değilse güncellenir —
                                // skor=0 nötr (MA etkisiz ama zararlı değil), skor<0 geçersiz.
                                if st.hyperopt_score >= 0.0 {
                                    lr.global_fast = fast;
                                    lr.global_slow = slow;
                                }
                                lr.global_rsi_period  = rp;
                                lr.global_rsi_ob      = rob;
                                lr.global_rsi_os      = ros;
                                lr.global_bb_period   = bbp;
                                lr.global_bb_std_dev  = bbs;
                                lr.global_macd_fast   = mf;
                                lr.global_macd_slow   = ms;
                                lr.global_macd_signal = msig;
                                lr.hyperopt_score     = st.hyperopt_score;
                                // Yeni strateji parametrelerini de güncelle
                                lr.global_ema_fast         = st.best_ema_fast;
                                lr.global_ema_slow         = st.best_ema_slow;
                                lr.global_donchian_period  = st.best_donchian_period;
                                lr.global_williams_period  = st.best_williams_period;
                                lr.global_cci_period       = st.best_cci_period;
                                lr.global_stoch_rsi_period = st.best_stoch_rsi_period;
                                lr.global_supertrend_period = st.best_supertrend_period;
                                lr.global_supertrend_mult  = st.best_supertrend_mult;
                                lr.global_ict_fvg_lookback = st.best_ict_fvg_lookback;
                                lr.global_smc_swing_lb     = st.best_smc_swing_lb;
                                // Versiyon sayacını artır — reload_strategy_params() bunu görür ve reload eder (Fix-4)
                                lr.strategy_params_version = lr.strategy_params_version.wrapping_add(1);
                            }
                            // ── Parametreleri config dosyasına persist et ──────────────
                            {
                                let cache = OptimizedParamsCache {
                                    ma_fast:     fast,
                                    ma_slow:     slow,
                                    rsi_period:  rp,
                                    rsi_ob:      rob,
                                    rsi_os:      ros,
                                    bb_period:   bbp,
                                    bb_std_dev:  bbs,
                                    macd_fast:   mf,
                                    macd_slow:   ms,
                                    macd_signal: msig,
                                    stoch_k:     st.best_stoch_k,
                                    stoch_ob:    st.best_stoch_ob,
                                    stoch_os:    st.best_stoch_os,
                                    ema_fast:          st.best_ema_fast,
                                    ema_slow:          st.best_ema_slow,
                                    donchian_period:   st.best_donchian_period,
                                    williams_period:   st.best_williams_period,
                                    cci_period:        st.best_cci_period,
                                    stoch_rsi_period:  st.best_stoch_rsi_period,
                                    supertrend_period: st.best_supertrend_period,
                                    supertrend_mult:   st.best_supertrend_mult,
                                    ict_fvg_lookback:  st.best_ict_fvg_lookback,
                                    smc_swing_lb:      st.best_smc_swing_lb,
                                    best_strategy: st.best_strategy_name.clone(),
                                    last_updated: Some(chrono::Utc::now().to_rfc3339()),
                                };
                                persist_optimized_params(cache);
                            }
                            // ── Pattern Library: iyi sonuçları piyasa koşuluyla birlikte kaydet ──
                            // Her hyperopt döngüsünde en iyi strateji için MarketCondition hesapla
                            // ve pattern_library'ye kaydet. Canlı işlemde pattern gate kullanır.
                            // NOT: Bu blok zaten app_state.lock() guard'ı (st) içinde — iç lock çağrısı
                            //      deadlock'a yol açar. st doğrudan kullanılır.
                            {
                                use memos_trading_core::robot::pattern_matcher::{MarketCondition, compute_confidence};
                                // st zaten scope'ta — app_state.lock() yeniden çağrılmaz
                                let (stk, sos, sob) = (st.best_stoch_k.max(3), st.best_stoch_os, st.best_stoch_ob);
                                let cond = MarketCondition::from_candles(&c, stk, sos, sob);
                                let (trend_s, vol_s, mom_s) = cond.parts();

                                // Kazanan strateji varsa kaydet
                                let best_strat = st.best_strategy_name.clone()
                                    .unwrap_or_else(|| "STOCHASTIC".to_string());
                                let (win_rate, avg_pnl, trade_count) = stoch_opt
                                    .map(|(k, ob, os, sc)| {
                                        let _ = (k, ob, os);
                                        (sc.clamp(0.0, 1.0), sc * 2.0, 20i64)
                                    })
                                    .unwrap_or((0.0, 0.0, 0));

                                let confidence = compute_confidence(win_rate, trade_count, avg_pnl);
                                if confidence > 0.0 {
                                    let params_json = format!(
                                        r#"{{"k":{},"ob":{},"os":{}}}"#,
                                        stk, sob, sos
                                    );
                                    if let Ok(conn) = rusqlite::Connection::open(&config.db_path) {
                                        let _ = database_writer::save_pattern(
                                            &conn,
                                            &best_strat,
                                            &params_json,
                                            &act_intv,
                                            &act_exch,
                                            &act_mkt,
                                            Some(&act_sym),
                                            trend_s, vol_s, mom_s,
                                            win_rate,
                                            avg_pnl,
                                            trade_count,
                                            confidence,
                                        );
                                    }
                                }
                                // live_risk'e best_strategy_name yaz — st zaten elimizde
                                if let Ok(mut lr) = st.live_risk.write() {
                                    if let Some(ref sname) = st.best_strategy_name.clone() {
                                        lr.best_strategy_name = sname.clone();
                                    }
                                    lr.global_stoch_k  = stk;
                                    lr.global_stoch_os = sos;
                                    lr.global_stoch_ob = sob;
                                }
                            }
                            // Sinyal sadece hâlâ aynı sembol üzerindeyse geçerli
                            let (cur_sym, cur_intv) = {
                                let t = st.active_trade_target();
                                (t.2, t.3)
                            };
                            if act_sym == cur_sym && act_intv == cur_intv {
                                // Güven eşiği: paper modda 0.35, live modda tam politika değeri
                                let base_conf = st.risk_gate.policy.min_model_confidence;
                                let min_conf  = if st.paper_mode { base_conf.min(0.35) } else { base_conf };
                                // hyperopt_score yalnızca MA_CROSSOVER stratejisine aittir.
                                // Aktif strateji MA değilse bu skoru HOLD filtresi olarak kullanma.
                                let active_strat = st.live_strategy.read().ok()
                                    .map(|s| s.clone()).unwrap_or_default();
                                let hopt_score = if active_strat.to_uppercase().contains("MA") {
                                    st.hyperopt_score
                                } else {
                                    0.0 // MA dışı stratejiler için MA skoru anlamsız → bypass
                                };
                                let forced_signal = if prediction.confidence < min_conf {
                                    "HOLD".to_string()
                                } else if hopt_score < 0.0 {
                                    // HyperOpt negatif skor = mevcut parametreler zararlı, işlem açma
                                    "HOLD".to_string()
                                } else {
                                    signal_str.clone()
                                };
                                if forced_signal != signal_str {
                                    // ml_below_threshold sayacını artır
                                    if let Ok(mut sc) = st.live_signal_counts.write() {
                                        sc.ml_below_threshold += 1;
                                    }
                                    if hopt_score < 0.0 {
                                        st.push_log(format!(
                                            "🚫 ML sinyal HOLD'a zorlandı: HyperOpt skor={:.4} negatif (orijinal={})",
                                            hopt_score, signal_str
                                        ));
                                    } else {
                                        st.push_log(format!(
                                            "🧠 ML sinyal HOLD'a zorlandı: conf={:.3} < min={:.2} (orijinal={})",
                                            prediction.confidence, min_conf, signal_str
                                        ));
                                    }
                                }
                                st.ml_signal     = forced_signal;
                                st.ml_confidence = prediction.confidence;
                                st.ml_score      = prediction.ml_score;
                            } else {
                                st.push_log(format!(
                                    "⚠️ ML sinyal atlandı: eğitim={}/{} ≠ aktif={}/{}",
                                    act_sym, act_intv, cur_sym, cur_intv
                                ));
                            }
                            st.push_log(format!("🧠 ML/AI Worker: {}", summary));
                            // ML eğitimi tamamlandı → snapshot kaydet
                            save_app_snapshot(&st);
                        }

                        // ── Monte Carlo + Walk-Forward Doğrulama ─────────────────────
                        // catch_unwind: backtester/WF paniklenirse ML worker ölmez.
                        let mc_wf_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            // Poisoned mutex'ten recover et: into_inner() guard'ı kurtarır
                            let (equity, best_strat, opt_sl, opt_tp) = {
                                let st = app_state.lock()
                                    .unwrap_or_else(|e| e.into_inner());
                                (st.equity,
                                 st.best_strategy_name.clone().unwrap_or_else(|| "RSI".to_string()),
                                 st.best_sl.max(0.5),
                                 st.best_tp.max(1.0))
                            };

                            // 1. En iyi stratejiyle quick backtest → trade PnL listesi
                            let trade_pnls: Vec<f64> = {
                                let bt_cfg = BacktestConfig {
                                    symbol:            format!("{}_VAL", act_sym),
                                    interval:          act_intv.clone(),
                                    initial_balance:   equity,
                                    max_position_size: 0.01,
                                    take_profit_pct:   opt_tp,
                                    stop_loss_pct:     opt_sl,
                                    strategy_name:     best_strat.clone(),
                                    position_profile:  None,
                                    security_profile:  None,
                                    strategy_params:   None,
                                    commission_pct:    0.001,
                                    breakeven_at_rr:   None,
                                    atr_trail_mult:    None,
                                    partial_tp_ratio:  None,
                                };
                                Backtester::new(bt_cfg).run(&c)
                                    .map(|r| r.trades.iter()
                                        .map(|t| t.pnl)
                                        .filter(|p| p.abs() > 0.001) // PnL=0 trade'ler bootstrap'ta anlamsız
                                        .collect())
                                    .unwrap_or_default()
                            };

                            // 2. Monte Carlo (1000 simülasyon) — en az 5 anlamlı trade gerekli
                            // PnL=0 trade'ler filtrelendi; P5/P50/P95 artık farklı değer verecek
                            let mc_result = if trade_pnls.len() >= 5 {
                                MonteCarloSimulator {
                                    n_simulations: 1000,
                                    ruin_threshold: 0.50,
                                    seed: None,
                                }.run(&trade_pnls, equity)
                            } else { None };

                            // 3. Walk-Forward — uyarlamalı pencere; minimum 40 mum yeterli
                            let wf_result = if c.len() >= 40 {
                                let n = c.len();
                                let oos_bars = (n / 5).max(8);       // %20 OOS, min 8 bar
                                let is_bars  = (n - oos_bars).max(16); // %80 IS, min 16 bar
                                let wf_cfg = WalkForwardConfig {
                                    in_sample_bars:     is_bars,
                                    out_of_sample_bars: oos_bars,
                                    step_bars:          oos_bars,
                                    initial_balance:    equity,
                                    strategy_name:      best_strat.clone(),
                                    symbol:             act_sym.clone(),
                                    interval:           act_intv.clone(),
                                    commission_pct:     0.001,
                                };
                                WalkForwardTester::new(wf_cfg).run(&c)
                            } else { None };

                            // 4. ValidationResult oluştur ve bileşik skor hesapla
                            let mut vr = ValidationResult {
                                strategy_name: best_strat,
                                symbol:        format!("{}/{}", act_sym, act_intv),
                                computed_at:   chrono::Local::now().format("%H:%M").to_string(),
                                ..Default::default()
                            };
                            if let Some(ref mc) = mc_result {
                                vr.mc_n_sims       = mc.n_simulations;
                                vr.mc_n_trades     = mc.n_trades;
                                vr.mc_ruin_pct     = mc.ruin_probability * 100.0;
                                vr.mc_p5_balance   = mc.final_balance_p5;
                                vr.mc_p50_balance  = mc.final_balance_p50;
                                vr.mc_p95_balance  = mc.final_balance_p95;
                                vr.mc_max_dd_p50   = mc.max_dd_p50;
                                vr.mc_max_dd_p95   = mc.max_dd_p95;
                                vr.mc_positive_pct = mc.positive_scenario_pct;
                                vr.mc_expected_ret = mc.expected_return_pct;
                            }
                            if let Some(ref wf) = wf_result {
                                vr.wf_windows        = wf.total_windows;
                                vr.wf_profitable     = wf.profitable_windows;
                                vr.wf_consistency    = wf.consistency_score;
                                vr.wf_avg_oos_wr     = wf.avg_oos_win_rate;
                                vr.wf_avg_oos_pnl    = wf.avg_oos_pnl_pct;
                                vr.wf_avg_oos_pf     = wf.avg_oos_profit_factor;
                                vr.wf_avg_oos_dd     = wf.avg_oos_max_dd_pct;
                                vr.wf_avg_oos_sharpe = wf.avg_oos_sharpe;
                            }
                            vr.compute_composite();

                            // 5. AppState'e yaz + log (poisoned mutex'ten de recover et)
                            let mc_status = if mc_result.is_some() {
                                format!("MC-Ruin={:.1}% ({} trade)", vr.mc_ruin_pct, vr.mc_n_trades)
                            } else {
                                format!("MC=-- (backtest={} trade)", trade_pnls.len())
                            };
                            let wf_status = if wf_result.is_some() {
                                format!("WF={:.0}% ({} pencere)", vr.wf_consistency * 100.0, vr.wf_windows)
                            } else {
                                format!("WF=-- ({} mum)", c.len())
                            };
                            let log_msg = format!(
                                "Dogrulama: Skor={:.0}/100 {}  {}  [{}]",
                                vr.composite_score, mc_status, wf_status, vr.strategy_name,
                            );
                            {
                                let mut st = app_state.lock()
                                    .unwrap_or_else(|e| e.into_inner());
                                st.push_log(format!("📊 {}", log_msg));
                                st.validation_result = Some(vr);
                            }
                        }));
                        // Panic olduysa da log'a yaz — sessiz hata kalmasın
                        if let Err(_) = mc_wf_result {
                            let mut st = app_state.lock()
                                .unwrap_or_else(|e| e.into_inner());
                            st.push_log("⚠️ MC/WF hesaplama panikle sonlandı — validation_result güncellenmedi".to_string());
                        }
                    }
                    Ok(c) => {
                        if let Ok(mut st) = app_state.lock() {
                            st.push_log(format!("⚠️  ML Worker: yetersiz veri ({} < 30)", c.len()));
                        }
                    }
                    Err(e) => {
                        if let Ok(mut st) = app_state.lock() {
                            st.push_log(format!("⚠️  ML Worker DB: {}", e));
                        }
                    }
                }
                // Her durumda (başarı/hata/yetersiz veri) ml_running ve worker kilidi sıfırla
                {
                    let lr_arc = clone_live_risk(&app_state);
                    if let Ok(mut lr) = lr_arc.write() { lr.ml_running = false; };
                }
                ml_worker_running.store(false, Ordering::SeqCst);

                // ── p5_crypto.py arka plan analizi ─────────────────────────────
                {
                    let (p5_sym, p5_mkt, p5_exch, p5_intv, p5_db) = {
                        if let Ok(st) = app_state.lock() {
                            let tgt = st.active_trade_target();
                            (tgt.2, tgt.1, tgt.0, tgt.3, config.db_path.clone())
                        } else {
                            (config.symbol.clone(), config.market.clone(),
                             config.exchange.clone(), config.interval.clone(),
                             config.db_path.clone())
                        }
                    };
                    spawn_p5_analysis(&app_state, &p5_sym, &p5_mkt, &p5_exch, &p5_intv, &p5_db);
                }
            }

            std::thread::sleep(Duration::from_secs(tick));
            elapsed += tick;
        }
    });
}

// ─── p5_crypto Spawn Yardımcısı ──────────────────────────────────────────────
/// `python3 p5_crypto.py` sürecini arka planda başlatır.
/// ML worker döngüsünden sonra ve [p] tuşuyla çağrılır.
fn spawn_p5_analysis(
    app_state: &Arc<Mutex<AppState>>,
    symbol:    &str,
    market:    &str,
    exchange:  &str,
    interval:  &str,
    db_path:   &str,
) {
    let sym = symbol.to_string();
    let intv = interval.to_string();
    match std::process::Command::new("python3")
        .arg("p5_crypto.py")
        .arg("--db").arg(db_path)
        .arg("--symbol").arg(symbol)
        .arg("--market").arg(market)
        .arg("--exchange").arg(exchange)
        .arg("--interval").arg(interval)
        .arg("--out-dir").arg("data/p5_results")
        .arg("--n-sim").arg("3000")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_child) => {
            if let Ok(mut st) = app_state.lock() {
                st.push_log(format!("🐍 p5_crypto başlatıldı — {} {} (arka plan)", sym, intv));
                st.p5_last_status = Some(P5Status {
                    state:    "running".to_string(),
                    msg:      "Başlatıldı".to_string(),
                    symbol:   sym.clone(),
                    interval: intv.clone(),
                    ..Default::default()
                });
            }
        }
        Err(e) => {
            if let Ok(mut st) = app_state.lock() {
                st.push_log(format!("⚠️  p5_crypto başlatılamadı: {}", e));
                st.push_log("   → python3 kurulu mu? p5_crypto.py aynı dizinde mi?".to_string());
            }
        }
    }
}

// ─── Otonom Pipeline Worker ──────────────────────────────────────────────────

// ─── Anlık Sinyal Değerlendirme Worker ───────────────────────────────────────
// [t] tuşu tetikler: DB'den mevcut mumları çeker, RSI/BB/MACD hesaplar, sinyali loglar.
// Bekleme süresi olmaksızın anlık durum raporu.

fn run_signal_check_worker(
    app_state:   Arc<Mutex<AppState>>,
    stop_signal: Arc<AtomicBool>,
    config:      OtoConfig,
) {
    use memos_trading_core::robot::indicators::{calculate_rsi, calculate_bollinger, calculate_macd};

    std::thread::spawn(move || {
        loop {
            if stop_signal.load(Ordering::Relaxed) { break; }

            let triggered = {
                let st = app_state.lock().unwrap();
                st.signal_trigger.swap(false, Ordering::Relaxed)
            };

            if triggered {
                // Tüm hedefleri topla: aktif sembol + orchestrator workers
                let (targets, common_params, sc_snapshot) = {
                    let st = app_state.lock().unwrap();
                    let (e, m, s, i) = st.active_trade_target();
                    let lrm = st.live_risk.read().ok()
                        .map(|r| (r.global_rsi_period, r.global_rsi_ob, r.global_rsi_os,
                                  r.global_bb_period, r.global_bb_std_dev,
                                  r.global_macd_fast, r.global_macd_slow, r.global_macd_signal))
                        .unwrap_or((14, 70.0, 30.0, 20, 2.0, 12, 26, 9));
                    let sname = st.live_strategy.read().ok()
                        .map(|s| s.clone())
                        .unwrap_or_else(|| "MA".to_string());
                    // Birincil hedef
                    let mut list: Vec<(String, String, String, String, String)> =
                        vec![(e.clone(), m.clone(), s.clone(), i.clone(), sname.clone())];
                    // Orchestrator worker'ları
                    for h in st.orchestrator.workers.values() {
                        if h.symbol != s {
                            list.push((
                                if e.is_empty() { "binance".to_string() } else { e.clone() },
                                h.market.clone(),
                                h.symbol.clone(),
                                h.interval.clone(),
                                sname.clone(),
                            ));
                        }
                    }
                    let sc = st.live_signal_counts.read().ok().map(|g| g.clone());
                    (list, lrm, sc)
                };

                let total_syms = targets.len();
                {
                    let mut st = app_state.lock().unwrap();
                    st.push_log(format!(
                        "⚡ [t] {} sembol için sinyal raporu başlatıldı — {}",
                        total_syms, chrono::Local::now().format("%H:%M:%S")
                    ));
                }

                let (rsi_period, rsi_ob, rsi_os,
                     bb_period, bb_std,
                     macd_fast, macd_slow, macd_sig) = common_params;

                for (act_exch, act_mkt, act_sym, act_intv, strat_name) in targets {
                    // Her sembol için DB'den mum yükle
                    let candles = database_reader::read_candles(
                        &config.db_path,
                        &act_exch,
                        &act_mkt,
                        &act_sym,
                        &act_intv,
                        Some(config.backtest_candle_limit.max(200)),
                    );

                    match candles {
                        Ok(c) if c.len() >= 30 => {
                            let last_close = c.last().map(|x| x.close).unwrap_or(0.0);

                            // RSI
                            let rsi_val = calculate_rsi(&c, rsi_period);
                            let rsi_signal = rsi_val.map(|v| {
                                if v < rsi_os { "BUY" } else if v > rsi_ob { "SELL" } else { "HOLD" }
                            }).unwrap_or("N/A");

                            // Bollinger
                            let bb_result = calculate_bollinger(&c, bb_period, bb_std);
                            let bb_signal = bb_result.map(|(upper, _mid, lower)| {
                                if last_close <= lower { "BUY" }
                                else if last_close >= upper { "SELL" }
                                else { "HOLD" }
                            }).unwrap_or("N/A");

                            // MACD crossover
                            let macd_signal_str = if c.len() >= macd_slow + macd_sig + 2 {
                                let prev = &c[..c.len()-1];
                                let prev_m = calculate_macd(prev, macd_fast, macd_slow, macd_sig);
                                let curr_m = calculate_macd(&c, macd_fast, macd_slow, macd_sig);
                                match (prev_m, curr_m) {
                                    (Some((pm, ps, _)), Some((cm, cs, _))) => {
                                        if pm <= ps && cm > cs { "BUY" }
                                        else if pm >= ps && cm < cs { "SELL" }
                                        else { "HOLD" }
                                    }
                                    _ => "N/A",
                                }
                            } else { "N/A" };

                            let mut msgs = Vec::new();
                            msgs.push(format!(
                                "⚡ [{}/{}] {} | strateji={}  fiyat={:.4}  mumlar={}",
                                act_sym, act_intv, chrono::Local::now().format("%H:%M:%S"),
                                strat_name, last_close, c.len()
                            ));
                            msgs.push(format!(
                                "   RSI({})={:.1} → {}  BB({},σ{:.1}) → {}  MACD({}/{}/{}) → {}",
                                rsi_period, rsi_val.unwrap_or(0.0), rsi_signal,
                                bb_period, bb_std, bb_signal,
                                macd_fast, macd_slow, macd_sig, macd_signal_str
                            ));
                            if let Some((upper, mid, lower)) = bb_result {
                                msgs.push(format!(
                                    "   BB detay: upper={:.4} mid={:.4} lower={:.4}",
                                    upper, mid, lower
                                ));
                            }
                            if let Some((macd_v, sig_v, hist)) = calculate_macd(&c, macd_fast, macd_slow, macd_sig) {
                                msgs.push(format!(
                                    "   MACD detay: macd={:.4} sig={:.4} hist={:.4}",
                                    macd_v, sig_v, hist
                                ));
                            }
                            {
                                let mut st = app_state.lock().unwrap();
                                for m in msgs { st.push_log(m); }
                            }
                        }
                        Ok(_) => {
                            let mut st = app_state.lock().unwrap();
                            st.push_log(format!("⚡ [{}] Yeterli mum yok — [d] ile indir", act_sym));
                        }
                        Err(e) => {
                            let mut st = app_state.lock().unwrap();
                            st.push_log(format!("⚡ [{}] DB okuma: {}", act_sym, e));
                        }
                    }
                }

                // Genel sinyal sayaçları (birincil sembol için — son olarak göster)
                if let Some(sc) = sc_snapshot {
                    let total_blocked = sc.blocked_rr + sc.blocked_volatility
                        + sc.blocked_trend + sc.blocked_risk_gate;
                    let mut st = app_state.lock().unwrap();
                    st.push_log(format!(
                        "   Sayaçlar: BUY={} SELL={} HOLD={} | Blok: R/R={} Vol={} Trend={} Gate={} | ML↓={}",
                        sc.buy, sc.sell, sc.hold,
                        sc.blocked_rr, sc.blocked_volatility,
                        sc.blocked_trend, sc.blocked_risk_gate,
                        sc.ml_below_threshold
                    ));
                    if !sc.last_block_reason.is_empty() {
                        st.push_log(format!("   Son blok: {}", sc.last_block_reason));
                    }
                    if total_blocked > 0 || sc.hold > 0 {
                        let total = sc.buy + sc.sell + sc.hold + total_blocked;
                        if total > 0 {
                            st.push_log(format!(
                                "   Trade kaçırma: {:.1}% blok + {:.1}% HOLD (/{} toplam karar)",
                                total_blocked as f64 / total as f64 * 100.0,
                                sc.hold as f64 / total as f64 * 100.0,
                                total
                            ));
                        }
                    }
                }
            }

            std::thread::sleep(Duration::from_millis(200));
        }
    });
}

// ─── Otomatik Export Worker ───────────────────────────────────────────────────
// Her `every_mins` dakikada bir export raporu yazar.
// Son `keep` dosyayı tutar, eskilerini siler (disk dolmasını önler).

fn run_auto_export_worker(
    app_state:   Arc<Mutex<AppState>>,
    stop_signal: Arc<AtomicBool>,
    every_mins:  u64,
    keep:        usize,
) {
    if every_mins == 0 { return; } // devre dışı

    std::thread::spawn(move || {
        // İlk çalışmayı biraz geciktir (sistem oturmasın)
        std::thread::sleep(Duration::from_secs(every_mins * 60));

        loop {
            if stop_signal.load(Ordering::Relaxed) { break; }

            // logs/ klasörünü oluştur
            let _ = std::fs::create_dir_all("logs");

            // Export içeriğini oluştur
            let (content, path) = {
                let st = app_state.lock().unwrap();
                let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
                let p = format!("logs/rtc_export_{}.txt", ts);
                (build_export_report(&st), p)
            };

            let file_ok = std::fs::write(&path, &content).is_ok();

            // Eski dosyaları temizle (en eski önce sil)
            if keep > 0 {
                if let Ok(mut entries) = std::fs::read_dir("logs") {
                    let mut exports: Vec<_> = std::fs::read_dir("logs")
                        .unwrap_or_else(|_| std::fs::read_dir(".").unwrap())
                        .flatten()
                        .filter(|e| {
                            e.file_name().to_string_lossy().starts_with("rtc_export_")
                                && e.file_name().to_string_lossy().ends_with(".txt")
                        })
                        .collect();
                    exports.sort_by_key(|e| e.file_name());
                    if exports.len() > keep {
                        let to_delete = exports.len() - keep;
                        for entry in exports.iter().take(to_delete) {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                    let _ = entries.next(); // suppress unused warning
                }
            }

            {
                let mut st = app_state.lock().unwrap();
                if file_ok {
                    st.push_log(format!("📊 Otomatik export → {}", path));
                } else {
                    st.push_log("❌ Otomatik export başarısız (logs/ yazılamadı)".to_string());
                }
            }

            std::thread::sleep(Duration::from_secs(every_mins * 60));
        }
    });
}

// ─── Canlı Tanılama Worker ────────────────────────────────────────────────────
// 4 katmanlı tutarlılık kontrolü: 10 saniyede bir çalışır.
// Bulgular AppState.diag_alerts'e ve olay günlüğüne [DIAG] önekiyle yazılır.
// Dashboard'da diag_warn_count ile gösterilir.

/// Orchestrator worker'ları için bağımsız fiyat sorgulayıcı.
/// Download loop tıkalı olsa bile her 10s'de non-primary sembollerin fiyatını günceller.
fn run_price_poller(
    app_state:   Arc<Mutex<AppState>>,
    stop_signal: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("price_poller tokio runtime");

        loop {
            if stop_signal.load(Ordering::Relaxed) { break; }
            std::thread::sleep(Duration::from_secs(10));
            if stop_signal.load(Ordering::Relaxed) { break; }

            // Aktif sembol dahil TÜM worker'lar için REST güncelleme.
            // WS kopuk veya loop yeniden başlarken fiyatın sıfırlanmaması için
            // primary sembolü de dahil ediyoruz.
            let targets: Vec<(String, String, Arc<std::sync::RwLock<LivePriceData>>)> = {
                match app_state.lock() {
                    Ok(st) => {
                        let (_, active_mkt, active_sym, _) = st.active_trade_target();
                        let mut list: Vec<(String, String, Arc<std::sync::RwLock<LivePriceData>>)> =
                            st.orchestrator.workers.values()
                                .filter_map(|h| {
                                    st.orchestrator.live_price_for(&h.symbol)
                                        .map(|arc| (h.market.clone(), h.symbol.clone(), arc))
                                })
                                .collect();
                        // Primary sembol orchestrator'da yoksa st.live_price'ı da güncelle
                        if !list.iter().any(|(_, s, _)| s == &active_sym) {
                            list.push((active_mkt, active_sym, Arc::clone(&st.live_price)));
                        }
                        list
                    }
                    Err(_) => continue,
                }
            };

            for (mkt, sym, price_arc) in targets {
                if stop_signal.load(Ordering::Relaxed) { break; }
                let url = if mkt == "futures" {
                    format!("https://fapi.binance.com/fapi/v1/klines?symbol={}&interval=1m&limit=2", sym)
                } else {
                    format!("https://api.binance.com/api/v3/klines?symbol={}&interval=1m&limit=2", sym)
                };
                let res: Result<Vec<Vec<serde_json::Value>>, _> = rt.block_on(async {
                    reqwest::get(&url).await?.json().await
                });
                if let Ok(rows) = res {
                    if let Some(k) = rows.last() {
                        let parse = |i: usize| -> f64 {
                            k.get(i).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0)
                        };
                        let (open, close, vol) = (parse(1), parse(4), parse(7));
                        let ts_ms = k.get(0).and_then(|v| v.as_i64()).unwrap_or(0);
                        if close > 0.0 {
                            if let Ok(mut pd) = price_arc.write() {
                                pd.symbol     = sym.clone();
                                pd.open       = open;
                                pd.close      = close;
                                pd.volume     = vol;
                                pd.change_pct = if open > 0.0 { (close - open) / open * 100.0 } else { 0.0 };
                                pd.ts = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts_ms)
                                    .map(|dt| dt.with_timezone(&chrono::Local).format("%H:%M:%S").to_string())
                                    .unwrap_or_default();
                            }
                        }
                    }
                }
                // Binance rate-limit koruması
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    });
}

fn run_diagnostic_worker(
    app_state:   Arc<Mutex<AppState>>,
    stop_signal: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        // Yerel durum — önceki değerlerle karşılaştırma için
        let mut prev_close: f64   = 0.0;
        let mut prev_close_at     = Instant::now();
        let mut last_cycle_local: u64 = 0;
        let mut last_cycle_local_at   = Instant::now();
        // K2/K3/K4 debounce: aynı uyarı mesajı max 1 kez/5dk (K4: 1 saat) push_log'a yazılır
        let mut k2_last_logged: std::collections::HashMap<String, Instant> = std::collections::HashMap::new();
        let mut k3_last_logged: std::collections::HashMap<String, Instant> = std::collections::HashMap::new();
        // K4 OHLCV fiziksel ihlalleri borsadan gelen ham veri kalite sorunlarıdır — 1 saat sustur
        let mut k4_last_logged: std::collections::HashMap<String, Instant> = std::collections::HashMap::new();
        // Kapanan işlem takibi — yeni işlem tespit edilince bildirim gönderilir
        let mut last_trade_notify_count: usize = 0;

        loop {
            if stop_signal.load(Ordering::Relaxed) { break; }
            std::thread::sleep(Duration::from_secs(10));

            let now = Instant::now();
            let mut alerts: Vec<String> = Vec::new();

            // ── Snapshot al (lock süresi kısa tutulsun) ──────────────────────
            let (
                live_mode, paused, can_trade,
                consecutive_failures, cycle_id, fsm_state,
                ml_signal, ml_confidence, min_confid,
                best_fast, best_slow, best_sl, best_tp,
                auto_symbol, active_sym, candidates,
                _price_sym, price_open, price_high, price_low,
                price_close, price_change,
                hyperopt_score,
                interval_secs,
            ) = {
                let st = match app_state.lock() { Ok(s) => s, Err(_) => continue };
                let (_, _, act_sym_diag, _) = st.active_trade_target();
                let price = st.orchestrator.live_price_for(&act_sym_diag)
                    .and_then(|arc| arc.read().ok().map(|p| (p.symbol.clone(), p.open, p.high, p.low, p.close, p.change_pct)))
                    .filter(|p| p.4 > 0.0)
                    .or_else(|| st.live_price.read().ok()
                        .map(|p| (p.symbol.clone(), p.open, p.high, p.low, p.close, p.change_pct)))
                    .unwrap_or_default();
                // Aktif interval'ı saniyeye çevir: eşik hesabında kullanılır
                let intv = st.active_symbol.interval.trim().to_lowercase();
                let isecs: u64 = if let Some(n) = intv.strip_suffix('m') {
                    n.parse::<u64>().unwrap_or(1) * 60
                } else if let Some(n) = intv.strip_suffix('h') {
                    n.parse::<u64>().unwrap_or(1) * 3600
                } else if let Some(n) = intv.strip_suffix('d') {
                    n.parse::<u64>().unwrap_or(1) * 86400
                } else { 60 };
                (
                    st.live_mode, st.paused, st.controller.can_trade(),
                    st.controller.consecutive_failures,
                    st.controller.cycle_id,
                    format!("{}", st.controller.state),
                    st.ml_signal.clone(), st.ml_confidence,
                    // paper modda gerçek eşik 0.35 — K3 bunu bilmeli
                    if st.paper_mode {
                        st.risk_gate.policy.min_model_confidence.min(0.35)
                    } else {
                        st.risk_gate.policy.min_model_confidence
                    },
                    st.best_fast, st.best_slow, st.best_sl, st.best_tp,
                    st.auto_symbol, st.active_symbol.clone(), st.symbol_candidates.clone(),
                    price.0, price.1, price.2, price.3, price.4, price.5,
                    st.hyperopt_score,
                    isecs,
                )
            };

            // Eşik: interval süresinin 2 katı (min 5dk, max 2 saat)
            // Örnek: 1m → 2dk, 1h → 2sa, 4h → 2sa (kısıtlı)
            let stale_secs = (interval_secs * 2).clamp(300, 7200);

            // ══ KATMAN 1: Sağlık Kontrolü ════════════════════════════════════

            // can_trade=false ama sıfır hata — risk limiti aşılmış olabilir
            if live_mode && !can_trade && consecutive_failures == 0 && !paused {
                alerts.push("K1⚠ can_trade()=false, hata=0 — risk limiti aşıldı?".to_string());
            }

            // Duraklatılmış ama FSM hâlâ Halted değil
            if paused && fsm_state != "Halted" {
                alerts.push(format!("K1⚠ Duraklatıldı ama FSM={} (Halted bekleniyor)", fsm_state));
            }

            // ══ KATMAN 2: Veri Akışı / Döngü Donma Tespiti ═══════════════════

            // Cycle ID değişimini takip et
            if cycle_id != last_cycle_local {
                last_cycle_local    = cycle_id;
                last_cycle_local_at = now;
            }
            // cycle_id sadece 1h candle geldiğinde artar — eşik sabit 2 saat (7200 sn).
            // interval×2 (5 dk) kullanmak yanlış: 1m interval'da bile cycle saatte bir artar.
            let cycle_stale_secs = 7200u64; // 2 saat
            if live_mode && now.duration_since(last_cycle_local_at) > Duration::from_secs(cycle_stale_secs) {
                alerts.push(format!(
                    "K2⚠ Cycle ID {} son 2 saatte değişmedi — 1h veri akışı durmuş olabilir",
                    last_cycle_local
                ));
            }

            // Fiyat verisi bayatlık: interval×2'yi aşarsa gerçek sorun
            if price_close != 0.0 {
                if price_close != prev_close {
                    prev_close    = price_close;
                    prev_close_at = now;
                }
                if live_mode && now.duration_since(prev_close_at) > Duration::from_secs(stale_secs) {
                    let mins = stale_secs / 60;
                    alerts.push(format!(
                        "K2⚠ Canlı fiyat {:.4} son {} dk değişmedi — veri beslemesi durmuş olabilir",
                        prev_close, mins
                    ));
                }
            } else if live_mode {
                alerts.push("K2⚠ Canlı fiyat verisi henüz yok (close=0)".to_string());
            }

            // ══ KATMAN 3: Tab-Arası Tutarlılık ═══════════════════════════════

            // ML confidence < eşik ama sinyal HOLD değil
            if ml_confidence < min_confid && ml_signal != "HOLD" {
                alerts.push(format!(
                    "K3⚠ ML conf={:.3} < min_confid={:.2} ama sinyal={} (HOLD olmalı)",
                    ml_confidence, min_confid, ml_signal
                ));
            }

            // HyperOpt parametreleri: fast >= slow geçersiz MA çifti
            if best_fast >= best_slow {
                alerts.push(format!(
                    "K3⚠ HyperOpt: fast={} >= slow={} — geçersiz MA çifti",
                    best_fast, best_slow
                ));
            }

            // Risk parametreleri: SL >= TP → RR oranı bozuk
            if best_sl >= best_tp {
                alerts.push(format!(
                    "K3⚠ Risk: SL={:.2}% >= TP={:.2}% — RR oranı bozuk (TP > SL olmalı)",
                    best_sl, best_tp
                ));
            }

            // AUTO modda active_symbol, candidates listesinde yer almalı
            if auto_symbol && !active_sym.symbol.is_empty() {
                let found = candidates.iter().any(|c|
                    c.symbol == active_sym.symbol && c.interval == active_sym.interval
                );
                if !found {
                    alerts.push(format!(
                        "K3⚠ active_symbol ({} {}) sembol adaylarında yok — seçici güncellenmedi",
                        active_sym.symbol, active_sym.interval
                    ));
                }
            }

            // HyperOpt skoru negatif + sinyal BUY/SELL → gerçekten zararlı işlem riski
            // HOLD ise zaten işlem yok, uyarı gereksiz
            if hyperopt_score < 0.0 && (ml_signal == "BUY" || ml_signal == "SELL") {
                alerts.push(format!(
                    "K3⚠ HyperOpt skor={:.6} negatif ama sinyal={} — zararlı parametrelerle işlem açılabilir!",
                    hyperopt_score, ml_signal
                ));
            }

            // ══ KATMAN 4: Matematik Doğrulaması ══════════════════════════════

            if price_close != 0.0 {
                // OHLCV fiziksel tutarlılık — Binance live tick float farkları için %0.1 tolerans
                // High==0 ise mum henüz oluşmamış (WS ilk mesajı) — ihlal kontrolü atla.
                let eps = price_close * 0.001;
                if price_high > 0.0 && (price_low > price_open + eps || price_low > price_close + eps) {
                    alerts.push(format!(
                        "K4⚠ OHLCV: Low({:.4}) > Open({:.4}) veya Close({:.4}) — fiziksel ihlal",
                        price_low, price_open, price_close
                    ));
                }
                if price_high > 0.0 && (price_high < price_open - eps || price_high < price_close - eps) {
                    alerts.push(format!(
                        "K4⚠ OHLCV: High({:.4}) < Open({:.4}) veya Close({:.4}) — fiziksel ihlal",
                        price_high, price_open, price_close
                    ));
                }

                // Değişim yüzdesi matematiksel tutarlılık
                if price_open != 0.0 {
                    let expected_change = (price_close - price_open) / price_open * 100.0;
                    if (expected_change - price_change).abs() > 0.1 {
                        alerts.push(format!(
                            "K4⚠ Değişim% tutarsız: hesaplanan={:.3}% gösterilen={:.3}%",
                            expected_change, price_change
                        ));
                    }
                }
            }

            // ── Sonuçları AppState'e yaz ──────────────────────────────────────
            let alert_count = alerts.len() as u32;
            if let Ok(mut st) = app_state.lock() {
                // Eski uyarıları temizle (her 10s'de yeniden değerlendirme)
                st.diag_alerts.clear();
                st.diag_warn_count = alert_count;

                // ── Realize edilmiş PnL → st.equity güncelle ─────────────────
                // robotic_loop içindeki current_equity yerel değişken, AppState'e yazılmıyor.
                // Bu yüzden equity'yi her 10s'de kapanan işlemlerden yeniden hesapla.
                {
                    st.recalculate_equity();
                }

                // ── PnL Snapshot (her 30s'de bir) ────────────────────────────────────
                // Her açık pozisyon için fiyat kaynağını tespit et ve PnL kaydını al.
                // Bu kayıtlar export'ta "PnL Geçmişi" bölümünde gösterilir (sorun tespiti için).
                {
                    // Per-sembol fiyat haritası — orchestrator önce (WS buraya yazar), fallback st.live_price
                    let mut price_map: std::collections::HashMap<String, (f64, &'static str)> = std::collections::HashMap::new();
                    for handle in st.orchestrator.workers.values() {
                        if let Ok(pd) = handle.live_price.read() {
                            if pd.close > 0.0 { price_map.insert(handle.symbol.clone(), (pd.close, "arc")); }
                        }
                    }
                    // Fallback: orchestrator'da bulunmayan semboller için (eski st.live_price)
                    if let Ok(pd) = st.live_price.read() {
                        if pd.close > 0.0 { price_map.entry(pd.symbol.clone()).or_insert((pd.close, "arc")); }
                    }

                    let mut total_open_pnl = 0.0f64;
                    let mut pos_summary = String::new();
                    if let Ok(lm) = st.live_positions.read() {
                        for pos in lm.values() {
                            // composite key yerine pos.symbol ile price_map'e bak
                            let (cur, src) = price_map.get(&pos.symbol)
                                .copied()
                                .filter(|(v, _)| *v > 0.0)
                                .unwrap_or((pos.current_price, "db"));
                            let pnl = pos_pnl(cur, pos.entry_price, pos.qty, pos.is_long);
                            total_open_pnl += pnl;
                            pos_summary.push_str(&format!("{}:{:.4}[{}]({:+.2}$) ",
                                pos.symbol, cur, src, pnl));
                        }
                    }
                    let ts = chrono::Local::now().format("%H:%M:%S").to_string();
                    if st.pnl_snapshots.len() >= 120 { st.pnl_snapshots.pop_front(); }
                    st.pnl_snapshots.push_back((ts, total_open_pnl, pos_summary));
                }

                for msg in &alerts {
                    // Olay günlüğüne K2/K3 debounce: cycle stale → 2 saat, diğerleri 5 dk
                    let should_log = if msg.starts_with("K2") {
                        let debounce = if msg.contains("Cycle ID") { 7200u64 } else { 300u64 };
                        let last = k2_last_logged.get(msg).copied();
                        match last {
                            Some(t) if t.elapsed().as_secs() < debounce => false,
                            _ => { k2_last_logged.insert(msg.clone(), now); true }
                        }
                    } else if msg.starts_with("K3") {
                        let last = k3_last_logged.get(msg).copied();
                        match last {
                            Some(t) if t.elapsed().as_secs() < 300 => false,
                            _ => { k3_last_logged.insert(msg.clone(), now); true }
                        }
                    } else if msg.starts_with("K4") {
                        // K4 OHLCV ihlalleri borsa kaynaklı veri kalite sorunlarıdır — 1 saat debounce
                        let last = k4_last_logged.get(msg).copied();
                        match last {
                            Some(t) if t.elapsed().as_secs() < 3600 => false,
                            _ => { k4_last_logged.insert(msg.clone(), now); true }
                        }
                    } else {
                        true
                    };
                    if should_log {
                        st.push_log(format!("[DIAG] {}", msg));
                    }
                    if st.diag_alerts.len() >= 20 { st.diag_alerts.pop_front(); }
                    st.diag_alerts.push_back(msg.clone());
                }

                // ── Kapanan işlem bildirimi — log'a yaz ──────────────────────
                let new_trade_msgs: Vec<String> = {
                    if let Ok(trades) = st.live_closed_trades.read() {
                        let count = trades.len();
                        if count > last_trade_notify_count {
                            let msgs = trades.iter().skip(last_trade_notify_count).map(|trade| {
                                use memos_trading_core::robot::scalp_swing::TradeType;
                                let dir  = if trade.is_long { "LONG" } else { "SHORT" };
                                let sign = if trade.pnl >= 0.0 { "+" } else { "" };
                                let icon = if trade.pnl >= 0.0 { "✅" } else { "❌" };
                                let type_tag = match trade.trade_type {
                                    TradeType::Scalp   => "[SCP] ",
                                    TradeType::Swing   => "[SWG] ",
                                    TradeType::Regular => "",
                                };
                                format!(
                                    "{} {}{} {} | {:.4}→{:.4} pnl={}{:.2}$ ({})",
                                    icon, type_tag, trade.symbol, dir,
                                    trade.entry_price, trade.exit_price,
                                    sign, trade.pnl, trade.exit_reason,
                                )
                            }).collect();
                            last_trade_notify_count = count;
                            msgs
                        } else { vec![] }
                    } else { vec![] }
                };
                for msg in new_trade_msgs {
                    st.push_log(msg);
                }
            }
        }
    });
}

// ─── Borsa Emir Senkronizasyon Worker ────────────────────────────────────────
// Binance REST API'den açık emirler + geçmiş işlemler + futures pozisyonlarını çeker.
// Yalnızca live modda (paper=false, api_key set) çalışır; 60s'de bir yeniler.
// 'o' tuşuyla anında tetiklenebilir.
fn run_exchange_orders_worker(
    app_state:   Arc<Mutex<AppState>>,
    stop_signal: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("exchange_orders tokio rt");
        rt.block_on(async move {
            let client = reqwest::Client::new();

            // HMAC-SHA256 imzalama yardımcısı
            // İmzalı GET isteği: base_url + path?params&signature
            async fn signed_get(
                client: &reqwest::Client,
                api_key: &str,
                secret: &str,
                base: &str,
                path: &str,
                params: &str,
            ) -> Option<serde_json::Value> {
                let ts   = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                let query = if params.is_empty() {
                    format!("timestamp={}&recvWindow=5000", ts)
                } else {
                    format!("{}&timestamp={}&recvWindow=5000", params, ts)
                };
                // sign fonksiyonu burada tekrar tanımlanır (closure capture için)
                use hmac::{Hmac, Mac};
                use sha2::Sha256;
                type H = Hmac<Sha256>;
                let sig = {
                    let mut mac = H::new_from_slice(secret.as_bytes()).ok()?;
                    mac.update(query.as_bytes());
                    format!("{:x}", mac.finalize().into_bytes())
                };
                let url = format!("{}{}?{}&signature={}", base, path, query, sig);
                let resp = client.get(&url)
                    .header("X-MBX-APIKEY", api_key)
                    .send().await.ok()?;
                if !resp.status().is_success() { return None; }
                resp.json::<serde_json::Value>().await.ok()
            }

            let mut last_poll = std::time::Instant::now()
                .checked_sub(Duration::from_secs(120))
                .unwrap_or(std::time::Instant::now());

            loop {
                if stop_signal.load(Ordering::Relaxed) { break; }
                tokio::time::sleep(Duration::from_secs(5)).await;

                // trigger kontrolü: 'o' tuşu veya 60s periyodik
                let (triggered, paper_mode, api_key, api_secret, symbol, market) = {
                    let st = app_state.lock().unwrap();
                    let trig = st.exchange_orders_trigger.load(Ordering::Relaxed);
                    if trig { st.exchange_orders_trigger.store(false, Ordering::Relaxed); }
                    (
                        trig,
                        st.paper_mode,
                        std::env::var("BINANCE_API_KEY").unwrap_or_default(),
                        std::env::var("BINANCE_API_SECRET").unwrap_or_default(),
                        st.active_symbol.symbol.clone(),
                        st.active_symbol.market.clone(),
                    )
                };

                let should_poll = triggered || last_poll.elapsed() >= Duration::from_secs(60);
                if !should_poll { continue; }
                if paper_mode || api_key.is_empty() || api_secret.is_empty() {
                    // Paper modda borsa verisi olmaz — açık bilgi notu
                    if triggered {
                        if let Ok(mut st) = app_state.try_lock() {
                            st.push_log(
                                "ℹ️ [o] Borsa emir sync: paper mod — Binance'dan veri çekilmez".to_string()
                            );
                        }
                    }
                    last_poll = std::time::Instant::now();
                    continue;
                }
                last_poll = std::time::Instant::now();

                let is_futures = market == "futures" || market == "coinm";
                let spot_base  = "https://api.binance.com";
                let fut_base   = "https://fapi.binance.com";
                let sym_param  = format!("symbol={}", symbol);

                let mut rows: Vec<ExchangeOrderRow> = Vec::new();

                // ── 1. Açık emirler ──────────────────────────────────────────
                if is_futures {
                    // Futures açık emirler: /fapi/v1/openOrders
                    if let Some(arr) = signed_get(
                        &client, &api_key, &api_secret,
                        fut_base, "/fapi/v1/openOrders", &sym_param
                    ).await {
                        for item in arr.as_array().unwrap_or(&vec![]) {
                            rows.push(parse_binance_order(item, "fut-open", true));
                        }
                    }
                    // Futures açık pozisyonlar: /fapi/v2/positionRisk
                    if let Some(arr) = signed_get(
                        &client, &api_key, &api_secret,
                        fut_base, "/fapi/v2/positionRisk", &sym_param
                    ).await {
                        for item in arr.as_array().unwrap_or(&vec![]) {
                            let amt = item.get("positionAmt")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(0.0);
                            if amt.abs() < 1e-8 { continue; }
                            let entry = item.get("entryPrice").and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                            let unreal = item.get("unRealizedProfit").and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                            let sym_name = item.get("symbol").and_then(|v| v.as_str()).unwrap_or(&symbol);
                            rows.push(ExchangeOrderRow {
                                order_id:   0,
                                symbol:     sym_name.to_string(),
                                side:       if amt > 0.0 { "LONG".to_string() } else { "SHORT".to_string() },
                                status:     "POSITION".to_string(),
                                qty:        amt.abs(),
                                filled_qty: amt.abs(),
                                price:      entry,
                                avg_price:  entry,
                                pnl:        unreal,
                                is_active:  true,
                                created_at: "—".to_string(),
                                source:     "fut-pos".to_string(),
                            });
                        }
                    }
                    // Futures son 50 işlem: /fapi/v1/userTrades
                    let trade_param = format!("{}&limit=50", sym_param);
                    if let Some(arr) = signed_get(
                        &client, &api_key, &api_secret,
                        fut_base, "/fapi/v1/userTrades", &trade_param
                    ).await {
                        for item in arr.as_array().unwrap_or(&vec![]).iter().rev().take(20) {
                            rows.push(parse_binance_trade(item, "fut-trade"));
                        }
                    }
                } else {
                    // Spot açık emirler: /api/v3/openOrders
                    if let Some(arr) = signed_get(
                        &client, &api_key, &api_secret,
                        spot_base, "/api/v3/openOrders", &sym_param
                    ).await {
                        for item in arr.as_array().unwrap_or(&vec![]) {
                            rows.push(parse_binance_order(item, "spot-open", true));
                        }
                    }
                    // Spot son 20 işlem: /api/v3/myTrades
                    let trade_param = format!("{}&limit=20", sym_param);
                    if let Some(arr) = signed_get(
                        &client, &api_key, &api_secret,
                        spot_base, "/api/v3/myTrades", &trade_param
                    ).await {
                        for item in arr.as_array().unwrap_or(&vec![]).iter().rev().take(20) {
                            rows.push(parse_binance_trade(item, "spot-trade"));
                        }
                    }
                }

                // Aktif emirler üstte, geçmiş altta; her grup kendi içinde en yenisi önce
                rows.sort_by(|a, b| b.is_active.cmp(&a.is_active)
                    .then(b.created_at.cmp(&a.created_at)));

                let sync_time = chrono::Local::now().format("%H:%M:%S").to_string();
                if let Ok(mut st) = app_state.try_lock() {
                    let n_active = rows.iter().filter(|r| r.is_active).count();
                    let n_hist   = rows.len() - n_active;
                    st.exchange_orders      = rows;
                    st.exchange_orders_sync = sync_time.clone();
                    if triggered {
                        st.push_log(format!(
                            "🔄 [o] Borsa sync: {} açık emir, {} geçmiş işlem — {}",
                            n_active, n_hist, sync_time
                        ));
                    }
                }
            }
        });
    });
}

/// Binance order JSON'ını ExchangeOrderRow'a çevirir.
fn parse_binance_order(v: &serde_json::Value, source: &str, is_active: bool) -> ExchangeOrderRow {
    let ts = v.get("time").or_else(|| v.get("updateTime"))
        .and_then(|x| x.as_i64()).unwrap_or(0);
    let dt = if ts > 0 {
        chrono::DateTime::from_timestamp_millis(ts)
            .map(|d| chrono::DateTime::<chrono::Local>::from(d).format("%H:%M:%S").to_string())
            .unwrap_or_else(|| "—".to_string())
    } else { "—".to_string() };

    ExchangeOrderRow {
        order_id:   v.get("orderId").and_then(|x| x.as_u64()).unwrap_or(0),
        symbol:     v.get("symbol").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        side:       v.get("side").and_then(|x| x.as_str()).unwrap_or("?").to_string(),
        status:     v.get("status").and_then(|x| x.as_str()).unwrap_or("?").to_string(),
        qty:        v.get("origQty").and_then(|x| x.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0),
        filled_qty: v.get("executedQty").and_then(|x| x.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0),
        price:      v.get("price").and_then(|x| x.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0),
        avg_price:  v.get("avgPrice").or_else(|| v.get("cummulativeQuoteQty"))
            .and_then(|x| x.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0),
        pnl:        0.0,
        is_active,
        created_at: dt,
        source:     source.to_string(),
    }
}

/// Binance trade JSON'ını ExchangeOrderRow'a çevirir.
fn parse_binance_trade(v: &serde_json::Value, source: &str) -> ExchangeOrderRow {
    let ts = v.get("time").and_then(|x| x.as_i64()).unwrap_or(0);
    let dt = if ts > 0 {
        chrono::DateTime::from_timestamp_millis(ts)
            .map(|d| chrono::DateTime::<chrono::Local>::from(d).format("%H:%M:%S").to_string())
            .unwrap_or_else(|| "—".to_string())
    } else { "—".to_string() };
    let qty    = v.get("qty").and_then(|x| x.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let price  = v.get("price").and_then(|x| x.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let pnl    = v.get("realizedPnl").and_then(|x| x.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let side   = if v.get("isBuyer").and_then(|x| x.as_bool()).unwrap_or(true) { "BUY" } else { "SELL" };
    ExchangeOrderRow {
        order_id:   v.get("orderId").and_then(|x| x.as_u64()).unwrap_or(0),
        symbol:     v.get("symbol").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        side:       side.to_string(),
        status:     "FILLED".to_string(),
        qty,
        filled_qty: qty,
        price,
        avg_price:  price,
        pnl,
        is_active:  false,
        created_at: dt,
        source:     source.to_string(),
    }
}

// ─── Gerçek RoboticLoop ───────────────────────────────────────────────────────
// std::thread içinde kendi Tokio runtime'ını başlatır,
// Ana TUI thread'i ile Arc<Mutex<AppState>> üzerinden iletişim kurar.

/// `symbol_override` = Some((exchange, market, symbol, interval)) → bu sembol için çalış.
/// None ise `st.active_trade_target()` kullanılır (tekil-sembol geriye dönük uyumluluk).
fn real_robotic_loop(
    app_state: Arc<Mutex<AppState>>,
    stop_signal: Arc<AtomicBool>,
    pause_signal: Arc<AtomicBool>,
    capital: f64,
    symbol_override: Option<(String, String, String, String)>, // (exchange, market, symbol, interval)
) {
    std::thread::spawn(move || {
        let api_key    = std::env::var("BINANCE_API_KEY").unwrap_or_default();
        let api_secret = std::env::var("BINANCE_API_SECRET").unwrap_or_default();
        let is_testnet = std::env::var("TRADING_ENV")
            .map(|v| v.to_lowercase() == "testnet")
            .unwrap_or(false);
        let paper_mode = api_key.is_empty()
            || api_secret.is_empty()
            || is_testnet
            || std::env::var("BINANCE_PAPER_MODE")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(true);

        {
            let mut st = app_state.lock().unwrap();
            st.live_mode   = true;
            st.paper_mode  = paper_mode;
            st.api_key_set = !api_key.is_empty() && !api_secret.is_empty();
            st.push_log(format!(
                "🚀 RoboticLoop başlatıldı | paper={} | api_key={}",
                paper_mode,
                if api_key.is_empty() { "YOK" } else { "SET" }
            ));
            let cfg_backtest_enabled = st.config.backtest_enabled;
            let cfg_backtest_mins = st.config.backtest_every_mins;
            // symbol_override varsa o sembol loglanır (secondary worker); yoksa active_trade_target
            let (cfg_exchange, cfg_market, cfg_symbol, cfg_interval) =
                symbol_override.clone().unwrap_or_else(|| st.active_trade_target());
            st.push_log(format!(
                "📋 Konfig: {}/{}/{} interval={} | {}",
                cfg_exchange, cfg_market, cfg_symbol, cfg_interval,
                if symbol_override.is_some() { "secondary" } else { "primary" }
            ));
            if !api_key.is_empty() {
                st.push_log(format!(
                    "🔴 Executor: BinanceTradeExecutor | is_paper={}",
                    paper_mode
                ));
            } else {
                st.push_log("🧪 Executor: DummyTradeExecutor (API key yok — kâğıt mod)".to_string());
            }
            st.push_log("🧬 autonomous_enabled=true | RoboticLoop içinde enable_evolution() → AdaptiveBrain + PopulationManager aktif".to_string());
            if cfg_backtest_enabled {
                st.push_log(format!(
                    "🔬 Otonom Backtest worker aktif: her {} dk'da bir çalışacak",
                    cfg_backtest_mins
                ));
            }
        }

        // Config değerlerini al (lock bırakıldıktan sonra)
        // symbol_override varsa onu kullan (çoklu sembol modu), yoksa active_trade_target().
        let (oto_cfg, db_path_clone, loop_exchange, loop_market, loop_symbol, loop_interval) = {
            let st = app_state.lock().unwrap();
            let (ae, am, asym, aint) = symbol_override.clone().unwrap_or_else(|| st.active_trade_target());
            (st.config.clone(), st.config.db_path.clone(), ae, am, asym, aint)
        };

        // Pozisyon yönetimi parametrelerini robotic_profiles.json'dan yükle
        let profile_cfg = load_profile_config(&oto_cfg.robotic_profiles_path);
        // Interval'dan saniye hesapla (interval_secs loop uyku süresi için)
        let loop_interval_secs: u64 = {
            let t = loop_interval.trim().to_lowercase();
            if let Some(n) = t.strip_suffix('m')      { n.parse::<u64>().unwrap_or(1) * 60 }
            else if let Some(n) = t.strip_suffix('h') { n.parse::<u64>().unwrap_or(1) * 3600 }
            else if let Some(n) = t.strip_suffix('d') { n.parse::<u64>().unwrap_or(1) * 86400 }
            else { 60 }
        };

        // Singleton worker'lar SADECE birincil loop tarafından başlatılır.
        // Orchestrator worker'ları (symbol_override.is_some()) bu worker'ları atlar;
        // aksi halde N sembol = N×7 gereksiz thread + AppState yarış koşulu oluşur.
        let is_primary_loop = symbol_override.is_none();
        if is_primary_loop {
            // Canlı Tanılama worker'ı — uygulama ömrü boyunca çalışır.
            // Loop geçişlerinde stop_signal true olur ama app_stop_signal yalnızca 'q'de true olur.
            let app_stop_for_diag = {
                let st = app_state.lock().unwrap();
                Arc::clone(&st.app_stop_signal)
            };
            run_diagnostic_worker(
                Arc::clone(&app_state),
                app_stop_for_diag,
            );

            // Orchestrator fiyat güncelleyici — download loop'tan bağımsız, her 10s (REST fallback)
            let app_stop_for_poller = {
                let st = app_state.lock().unwrap();
                Arc::clone(&st.app_stop_signal)
            };
            run_price_poller(
                Arc::clone(&app_state),
                app_stop_for_poller,
            );

            // Canlı fiyat WS besleyici — tüm semboller için miniTicker, bağımsız thread+runtime
            // app_stop_signal kullanır: primary loop geçişlerinde ölmez, yalnızca 'q'de durur.
            {
                let state_ws = Arc::clone(&app_state);
                let stop_ws  = {
                    let st = app_state.lock().unwrap();
                    Arc::clone(&st.app_stop_signal)
                };
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("price_ws tokio runtime");
                    rt.block_on(run_live_price_ws_feeds(state_ws, stop_ws));
                });
            }

            // Otomatik export worker'ı
            run_auto_export_worker(
                Arc::clone(&app_state),
                Arc::clone(&stop_signal),
                oto_cfg.auto_export_every_mins,
                oto_cfg.auto_export_keep,
            );

            // Backtest worker'ı
            let backtest_running_arc = {
                let st = app_state.lock().unwrap();
                Arc::clone(&st.backtest_running)
            };
            run_backtest_worker(
                Arc::clone(&app_state),
                Arc::clone(&stop_signal),
                backtest_running_arc,
                oto_cfg.clone(),
            );

            // ML/AI worker'ı
            let ml_worker_running_arc = {
                let st = app_state.lock().unwrap();
                Arc::clone(&st.ml_worker_running)
            };
            run_ml_worker(
                Arc::clone(&app_state),
                Arc::clone(&stop_signal),
                ml_worker_running_arc,
                oto_cfg.clone(),
            );

            // Otonom Sembol Seçici worker'ı
            run_symbol_selector_worker(
                Arc::clone(&app_state),
                Arc::clone(&stop_signal),
                oto_cfg.db_path.clone(),
            );

            // Binance Sembol Tarayıcı: 24hr ticker → yeni sembol keşfi
            run_symbol_screener_worker(
                Arc::clone(&app_state),
                Arc::clone(&stop_signal),
                oto_cfg.db_path.clone(),
            );

            // Otonom Veri İndirme worker'ı
            run_download_worker(
                Arc::clone(&app_state),
                Arc::clone(&stop_signal),
                oto_cfg.clone(),
            );

            // [t] tuşu: anlık sinyal değerlendirme worker'ı
            run_signal_check_worker(
                Arc::clone(&app_state),
                Arc::clone(&stop_signal),
                oto_cfg.clone(),
            );

            // MTF Fiyat Monitörü: açık pozisyonlar için 30s'de bir 1m fiyatı günceller
            // SL/TP 1h mum kapanışı yerine 1m veriye dayanır → mum-kaydı azalır
            run_mtf_price_monitor_worker(
                Arc::clone(&app_state),
                Arc::clone(&stop_signal),
                oto_cfg.db_path.clone(),
            );

            // MTF Fırsat Tarayıcısı: 90s'de bir tüm sembol×interval kombinasyonlarını tarar
            // Yüksek kompozit skora sahip sinyaller dashboard MTF panelinde gösterilir
            run_mtf_opportunity_scanner(
                Arc::clone(&app_state),
                Arc::clone(&stop_signal),
            );

            // Borsa Emir Senkronizasyonu — live modda 60s'de bir Binance'dan order/trade çeker
            // 'o' tuşuyla anında tetiklenebilir; paper modda no-op
            run_exchange_orders_worker(
                Arc::clone(&app_state),
                Arc::clone(&stop_signal),
            );

            // ── Otonom Pipeline worker (D→B→ML→P5) ──────────────────────────
            // Başlangıçta 15s sonra çalışır; ardından pipeline_every_mins periyodik tekrar.
            if oto_cfg.pipeline_enabled {
                pipeline::run_pipeline_worker(
                    Arc::clone(&app_state),
                    Arc::clone(&stop_signal),
                    oto_cfg.clone(),
                );
            }

            // ── p5_crypto poller + trigger worker ────────────────────────────
            // status.json'ı 10s'de bir izler; "done" görünce full results JSON'ı da okur.
            // p5_trigger set ise anında p5 başlatır ([p] tuşu veya otomatik tetik).
            {
                let app_state_p5 = Arc::clone(&app_state);
                let stop_p5      = Arc::clone(&stop_signal);
                let oto_cfg_p5   = oto_cfg.clone();
                std::thread::spawn(move || {
                    use std::sync::atomic::Ordering;
                    let status_path  = "data/p5_results/status.json";
                    let mut last_ts  = String::new();
                    let mut tick_cnt = 0u64;
                    loop {
                        if stop_p5.load(Ordering::Relaxed) { break; }
                        std::thread::sleep(std::time::Duration::from_secs(10));
                        if stop_p5.load(Ordering::Relaxed) { break; }
                        tick_cnt += 1;

                        // p5_trigger tetiklendiyse anında başlat
                        let triggered = {
                            app_state_p5.lock().ok()
                                .map(|st| st.p5_trigger.swap(false, Ordering::Relaxed))
                                .unwrap_or(false)
                        };
                        if triggered {
                            let (sym, mkt, exch, intv, db) = {
                                if let Ok(st) = app_state_p5.lock() {
                                    let tgt = st.active_trade_target();
                                    (tgt.2, tgt.1, tgt.0, tgt.3, oto_cfg_p5.db_path.clone())
                                } else {
                                    (oto_cfg_p5.symbol.clone(), oto_cfg_p5.market.clone(),
                                     oto_cfg_p5.exchange.clone(), oto_cfg_p5.interval.clone(),
                                     oto_cfg_p5.db_path.clone())
                                }
                            };
                            spawn_p5_analysis(&app_state_p5, &sym, &mkt, &exch, &intv, &db);
                        }

                        // Her 3 tick'te bir (30s) status.json oku
                        if tick_cnt % 3 != 0 { continue; }

                        let content = match std::fs::read_to_string(status_path) {
                            Ok(c) => c,
                            Err(_) => continue,
                        };
                        let v: serde_json::Value = match serde_json::from_str(&content) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        let ts = v["ts"].as_str().unwrap_or("").to_string();
                        if ts == last_ts { continue; }
                        last_ts = ts.clone();

                        let st_val = v["state"].as_str().unwrap_or("").to_string();
                        let msg    = v["msg"].as_str().unwrap_or("").to_string();
                        let sym    = v["symbol"].as_str().unwrap_or("").to_string();
                        let intv   = v["interval"].as_str().unwrap_or("").to_string();
                        let tested = v["tested"].as_u64().unwrap_or(0) as u32;
                        let found2 = v["found"].as_u64().unwrap_or(0) as u32;

                        // Hızlı güncelleme: scanning/running durumu
                        if st_val != "done" {
                            if let Ok(mut st) = app_state_p5.lock() {
                                let prev = st.p5_last_status.clone().unwrap_or_default();
                                st.p5_last_status = Some(P5Status {
                                    state: st_val.clone(), msg: msg.clone(),
                                    ts: ts.clone(), symbol: sym.clone(), interval: intv.clone(),
                                    tested, found_so_far: found2,
                                    // geri kalanı koru
                                    strategies_found: prev.strategies_found,
                                    edge_confirmed:   prev.edge_confirmed,
                                    wf_consistency:   prev.wf_consistency,
                                    best_name:        prev.best_name,
                                    best_wr:          prev.best_wr,
                                    best_pf:          prev.best_pf,
                                    best_dd:          prev.best_dd,
                                    best_tp_mult:     prev.best_tp_mult,
                                    best_sl_mult:     prev.best_sl_mult,
                                    best_edge:        prev.best_edge,
                                    best_p_value:     prev.best_p_value,
                                    mc_prob_profit:   prev.mc_prob_profit,
                                    ruin_pct:         prev.ruin_pct,
                                    active_signals:   prev.active_signals,
                                    active_dir:       prev.active_dir,
                                    active_atr:       prev.active_atr,
                                    top_strategies:   prev.top_strategies,
                                });
                            }
                            continue;
                        }

                        // "done" — full results JSON oku
                        let found_n   = v["strategies_found"].as_u64().unwrap_or(0) as u32;
                        let best_n    = v["best_name"].as_str().unwrap_or("").to_string();
                        let best_wr   = v["best_wr"].as_f64().unwrap_or(0.0);
                        let best_pf   = v["best_pf"].as_f64().unwrap_or(0.0);
                        let mc_pp     = v["mc_prob_profit"].as_f64().unwrap_or(0.0);
                        let ruin      = v["ruin_pct"].as_f64().unwrap_or(0.0);
                        let sigs_n    = v["active_signals"].as_u64().unwrap_or(0) as u32;

                        // Full results JSON — mevcut sembol/interval için doğru dosya adı dene
                        let exch_str = v["exchange"].as_str().unwrap_or("binance");
                        let mkt_str  = v["market"].as_str().unwrap_or("spot");
                        let results_path2 = format!(
                            "data/p5_results/{}_{}_{}_{}_results.json",
                            sym, intv, exch_str, mkt_str
                        );

                        let (edge_cnt, wf_cons, best_dd, best_tp, best_sl, best_edge,
                             best_pval, active_dir, active_atr, top_strats) =
                        if let Ok(rc) = std::fs::read_to_string(&results_path2) {
                            if let Ok(rv) = serde_json::from_str::<serde_json::Value>(&rc) {
                                let summ = &rv["summary"];
                                let ec   = summ["edge_confirmed"].as_u64().unwrap_or(0) as u32;
                                let wfc  = summ["wf_consistency"].as_f64().unwrap_or(0.0);
                                let bdd  = summ["best_dd"].as_f64().unwrap_or(0.0);
                                let bpval= summ["best_p_value"].as_f64().unwrap_or(1.0);
                                let bedge= summ["best_edge"].as_str().unwrap_or("").to_string();

                                // En iyi stratejinin TP/SL mult'unu al
                                let (btp, bsl) = rv["top_strategies"].as_array()
                                    .and_then(|a| a.first())
                                    .map(|s| (
                                        s["tp_mult"].as_f64().unwrap_or(2.0),
                                        s["sl_mult"].as_f64().unwrap_or(1.0),
                                    ))
                                    .unwrap_or((2.0, 1.0));

                                // Aktif sinyal yönü
                                let (adir, aatr) = rv["current_signals"].as_array()
                                    .and_then(|a| a.first())
                                    .map(|s| (
                                        s["direction"].as_str().unwrap_or("").to_string(),
                                        s["atr"].as_f64().unwrap_or(0.0),
                                    ))
                                    .unwrap_or_default();

                                // Top 3 strateji
                                let top3: Vec<P5TopStrategy> = rv["top_strategies"].as_array()
                                    .map(|arr| arr.iter().take(3).map(|s| P5TopStrategy {
                                        name:      s["name"].as_str().unwrap_or("").to_string(),
                                        direction: s["direction"].as_str().unwrap_or("").to_string(),
                                        wr:        s["win_rate"].as_f64().unwrap_or(0.0),
                                        pf:        s["profit_factor"].as_f64().unwrap_or(0.0),
                                        dd:        s["max_dd"].as_f64().unwrap_or(0.0),
                                        wf_pass:   s["wf_pass"].as_u64().unwrap_or(0) as u32,
                                        edge:      s.get("edge_label").and_then(|e| e.as_str()).unwrap_or("").to_string(),
                                        tp_mult:   s["tp_mult"].as_f64().unwrap_or(2.0),
                                        sl_mult:   s["sl_mult"].as_f64().unwrap_or(1.0),
                                        p_value:   s.get("p_value").and_then(|p| p.as_f64()).unwrap_or(1.0),
                                    }).collect())
                                    .unwrap_or_default();

                                (ec, wfc, bdd, btp, bsl, bedge, bpval, adir, aatr, top3)
                            } else { (0, 0.0, 0.0, 2.0, 1.0, String::new(), 1.0, String::new(), 0.0, vec![]) }
                        } else { (0, 0.0, 0.0, 2.0, 1.0, String::new(), 1.0, String::new(), 0.0, vec![]) };

                        let p5s = P5Status {
                            state: st_val.clone(), msg: msg.clone(), ts,
                            symbol: sym.clone(), interval: intv.clone(),
                            strategies_found: found_n, edge_confirmed: edge_cnt,
                            wf_consistency: wf_cons,
                            best_name: best_n.clone(), best_wr, best_pf,
                            best_dd, best_tp_mult: best_tp, best_sl_mult: best_sl,
                            best_edge: best_edge.clone(), best_p_value: best_pval,
                            mc_prob_profit: mc_pp, ruin_pct: ruin,
                            active_signals: sigs_n, active_dir: active_dir.clone(),
                            active_atr,
                            tested, found_so_far: found2,
                            top_strategies: top_strats,
                        };

                        if let Ok(mut st) = app_state_p5.lock() {
                            if found_n > 0 {
                                st.push_log(format!(
                                    "🐍 p5_crypto ✓ — {} edge'li strateji | En iyi: {} WR={:.1}% PF={:.2} edge={}",
                                    edge_cnt, best_n, best_wr * 100.0, best_pf, best_edge
                                ));
                                if !active_dir.is_empty() {
                                    st.push_log(format!(
                                        "   ⚡ Şu an aktif sinyal: {} — TP={:.1}x SL={:.1}x ATR={:.4}",
                                        active_dir.to_uppercase(), best_tp, best_sl, active_atr
                                    ));
                                }
                                // Otonom: p5 güçlü bir LONG/SHORT sinyali varsa → log + best_strategy'yi güncelle önerisi
                                if !active_dir.is_empty() && best_pval < 0.05 {
                                    let dir_upper = active_dir.to_uppercase();
                                    st.push_log(format!(
                                        "   💡 p5 önerisi: {} gir — p={:.4} WF={:.0}% | [y] ile yenile",
                                        dir_upper, best_pval, wf_cons * 100.0
                                    ));
                                }
                            } else {
                                st.push_log(format!(
                                    "🐍 p5_crypto tamamlandı — {} için uygun strateji bulunamadı",
                                    sym
                                ));
                            }
                            st.p5_last_status = Some(p5s);
                        }
                    }
                });
            }

        }
        // NOT: run_orphan_position_ws_feeds rt.block_on() içinden spawn edilecek
        //      (tokio runtime henüz oluşturulmadı — burada spawn yapılamaz)

        // ── Başlangıç senkron gap-fill + optimizasyon tetikleyici ─────────────
        // Sadece birincil loop çalıştırır; ikincil worker'lar (orchestrator) atlar.
        if is_primary_loop {
            // 1. Senkron gap-fill: son 500 1m mumun eksiklerini REST'ten tamamla
            //    Async download worker daha geniş geçmişi arka planda doldurur.
            startup_sync_gap_fill(
                &oto_cfg.db_path,
                &oto_cfg.exchange,
                &oto_cfg.market,
                &loop_symbol,
            );

            // 2. Optimizasyon önbelleği yoksa ML worker'ı hemen tetikle
            //    (ML worker spawn sonrası ml_trigger'ı okur, 45s beklemeyi atlar)
            let needs_optimization = {
                let st = app_state.lock().unwrap();
                st.hyperopt_score == 0.0 && st.best_strategy_name.is_none()
            };
            if needs_optimization {
                if let Ok(st) = app_state.lock() {
                    st.ml_trigger.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                log::info!("startup: optimizasyon önbelleği yok — ML worker hemen tetiklendi");
            }
        }

        // ── Pipeline Zincir Monitörü ────────────────────────────────────────────
        // Her 60s'de tüm adımların tazeliğini kontrol eder; gecikenleri otomatik
        // yeniden tetikler veya kullanıcıya talimat gösterir.
        if is_primary_loop {
            let app_chain  = Arc::clone(&app_state);
            let stop_chain = {
                let st = app_state.lock().unwrap();
                Arc::clone(&st.app_stop_signal)
            };
            let chain_download_every = oto_cfg.download_every_mins * 60;
            let chain_backtest_every = oto_cfg.backtest_every_mins * 60;
            let chain_ml_every       = oto_cfg.backtest_every_mins * 60 + 30;
            let chain_screener_every = app_state.lock().ok()
                .map(|st| (st.screener_interval_hours * 3600.0) as u64)
                .unwrap_or(14400);
            std::thread::spawn(move || {
                use std::sync::atomic::Ordering;
                use memos_trading_core::robot::robotic_loop::{
                    PipelineChainStep, ChainStepStatus,
                };
                // Her adım için otomatik iyileştirme sayacı ve son tetik zamanı
                let mut heal_counts = [0u32; 7]; // [download, backtest, ml, p5, screener, mtf, signal]
                let mut heal_last   = [std::time::Instant::now(); 7]; // son auto-trigger zamanı
                const HEAL_COOLDOWN_SECS: u64 = 300; // aynı adım 5 dk'da bir en fazla 1x tetiklenebilir
                loop {
                    if stop_chain.load(Ordering::Relaxed) { break; }

                    let (
                        last_download, last_backtest, last_ml_train,
                        p5_state, p5_ts,
                        screener_last, mtf_last,
                        dl_trig, bt_trig, ml_trig, scr_trig, mtf_trig, p5_trig,
                        live_pipeline,
                        download_every, backtest_every, ml_every, screener_every,
                        download_active, backtest_active, ml_active,
                    ) = {
                        let Ok(st) = app_chain.lock() else { continue };
                        let bt_running = st.backtest_running.load(Ordering::Relaxed);
                        let ml_running = st.live_risk.read().ok()
                            .map(|r| r.ml_running).unwrap_or(false);
                        (
                            st.last_download_at.clone(),
                            st.last_backtest_at.clone(),
                            st.last_ml_train_at.clone(),
                            st.p5_last_status.as_ref().map(|p| p.state.clone()).unwrap_or_default(),
                            st.p5_last_status.as_ref().map(|p| p.ts.clone()).unwrap_or_default(),
                            st.screener_last_run.clone(),
                            st.mtf_last_scan.clone(),
                            Arc::clone(&st.download_trigger),
                            Arc::clone(&st.backtest_trigger),
                            Arc::clone(&st.ml_trigger),
                            Arc::clone(&st.screener_trigger),
                            Arc::clone(&st.mtf_scan_trigger),
                            Arc::clone(&st.p5_trigger),
                            st.live_pipeline.clone(),
                            chain_download_every,
                            chain_backtest_every,
                            chain_ml_every,
                            chain_screener_every,
                            st.download_active,
                            bt_running,
                            ml_running,
                        )
                    };

                    // Zaman damgasından saniye yaşını hesapla
                    let age_secs = |ts: &Option<String>| -> u64 {
                        ts.as_deref()
                            .and_then(|s| {
                                // "YYYY-MM-DD HH:MM:SS" formatını parse et (ilk 19 karakter)
                                let s = if s.len() > 19 { &s[..19] } else { s };
                                chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()
                            })
                            .map(|dt| {
                                let utc = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc);
                                (chrono::Utc::now() - utc).num_seconds().max(0) as u64
                            })
                            .unwrap_or(999_999)
                    };

                    let dl_age  = age_secs(&last_download);
                    let bt_age  = age_secs(&last_backtest);
                    let ml_age  = age_secs(&last_ml_train);
                    let scr_age = age_secs(&screener_last);
                    let mtf_age = age_secs(&{
                        mtf_last.as_ref().map(|s| {
                            // mtf_last_scan bazen "HH:MM (X/Y)" formatında — tarih yok
                            // sadece tarih formatındaki değerleri kabul et
                            if s.len() >= 19 && s.as_bytes().get(4) == Some(&b'-') {
                                s.clone()
                            } else {
                                String::new()
                            }
                        })
                    });
                    // p5: son çalışma zamanı p5_ts string'inden
                    let p5_age  = age_secs(&if p5_ts.is_empty() { None } else { Some(p5_ts.clone()) });
                    let p5_running = p5_state == "running";

                    // ── Adım durumlarını hesapla ve gerekirse tetikle ──────────────
                    let mut steps: Vec<PipelineChainStep> = Vec::with_capacity(7);
                    let mut repairs: Vec<String> = Vec::new();

                    // Yardımcı: adım oluştur
                    let make_step = |id: &'static str, label: &str, age: u64, interval: u64,
                                     running: bool, heal: u32, hint: &str| -> PipelineChainStep {
                        let status = if running {
                            ChainStepStatus::Running
                        } else if age == 999_999 {
                            ChainStepStatus::Pending
                        } else if interval > 0 && age > interval + 120 {
                            ChainStepStatus::Stale
                        } else {
                            ChainStepStatus::Ok
                        };
                        let overdue = if interval > 0 && age != 999_999 {
                            age as i64 - interval as i64
                        } else { 0 };
                        PipelineChainStep {
                            id, label: label.to_string(), status,
                            last_run_secs: age, interval_secs: interval,
                            overdue_secs: overdue, heal_count: heal,
                            user_hint: hint.to_string(),
                        }
                    };

                    // Yardımcı: hiç çalışmamış (Pending) veya süresi geçmiş adımı tetikle
                    macro_rules! maybe_trigger {
                        ($age:expr, $interval:expr, $trig:expr, $idx:expr, $label:expr) => {{
                            let never_run = $age == 999_999;
                            let overdue   = !never_run && $age > $interval + 120;
                            let cooled    = heal_last[$idx].elapsed().as_secs() >= HEAL_COOLDOWN_SECS;
                            if (never_run || overdue) && !$trig.load(Ordering::Relaxed) && cooled {
                                heal_last[$idx] = std::time::Instant::now();
                                $trig.store(true, Ordering::Relaxed);
                                heal_counts[$idx] += 1;
                                let reason = if never_run { "hiç çalışmadı".to_string() }
                                             else { format!("{}s gecikme", $age.saturating_sub($interval)) };
                                repairs.push(format!("🔄 {}: {} → otomatik tetiklendi (#{})", $label, reason, heal_counts[$idx]));
                            }
                        }};
                    }

                    // [0] Download
                    {
                        // Download aktif çalışıyorken tetikleme yapma ve Running göster
                        if !download_active {
                            maybe_trigger!(dl_age, download_every, dl_trig, 0, "İndir");
                        }
                        steps.push(make_step("download", "İndir", dl_age, download_every, download_active, heal_counts[0], "[d] tuşu ile manuel başlat"));
                    }
                    // [1] Backtest — download tamamlandıktan sonra tetikle
                    {
                        let dl_done = dl_age != 999_999;
                        if dl_done && !backtest_active {
                            maybe_trigger!(bt_age, backtest_every, bt_trig, 1, "BTest");
                        }
                        steps.push(make_step("backtest", "BTest", bt_age, backtest_every, backtest_active, heal_counts[1], "[b] tuşu ile manuel başlat"));
                    }
                    // [2] ML — backtest tamamlandıktan sonra tetikle
                    {
                        let bt_done = bt_age != 999_999;
                        if bt_done && !ml_active {
                            maybe_trigger!(ml_age, ml_every, ml_trig, 2, "ML");
                        }
                        steps.push(make_step("ml", "ML Eğit", ml_age, ml_every, ml_active, heal_counts[2], "[m] tuşu ile manuel başlat"));
                    }
                    // [3] p5Ana — ML tamamlandıktan sonra tetikle
                    {
                        let ml_done = ml_age != 999_999;
                        let p5_behind = ml_done && (p5_age == 999_999 || p5_age > ml_age + 120);
                        let p5_cooled = heal_last[3].elapsed().as_secs() >= HEAL_COOLDOWN_SECS;
                        if p5_behind && !p5_running && !p5_trig.load(Ordering::Relaxed) && p5_cooled {
                            heal_last[3] = std::time::Instant::now();
                            p5_trig.store(true, Ordering::Relaxed);
                            heal_counts[3] += 1;
                            let reason = if p5_age == 999_999 { "hiç çalışmadı".to_string() }
                                         else { format!("ML'den {}s geride", p5_age.saturating_sub(ml_age)) };
                            repairs.push(format!("🔄 p5Ana: {} → otomatik tetiklendi (#{})", reason, heal_counts[3]));
                        }
                        steps.push(make_step("p5", "p5Ana", p5_age, 0, p5_running, heal_counts[3],
                            "[y] ile manuel başlat | p5_crypto.py + python3 gerekli"));
                    }
                    // [4] Screener
                    {
                        maybe_trigger!(scr_age, screener_every, scr_trig, 4, "Tarayıcı");
                        steps.push(make_step("screener", "Tarayıcı", scr_age, screener_every, false, heal_counts[4], "[f] tuşu ile manuel başlat"));
                    }
                    // [5] MTF — screener sonrası tetiklenir; çok eskiyse yenile
                    {
                        let mtf_interval = 7200u64; // 2 saat
                        maybe_trigger!(mtf_age, mtf_interval, mtf_trig, 5, "MTF");
                        steps.push(make_step("mtf", "MTF Tarama", mtf_age, mtf_interval, false, heal_counts[5], "[u] tuşu ile manuel başlat"));
                    }
                    // [6] Sinyal — sürekli çalışır; sadece ML/BTest sonrası freshness göster
                    {
                        // Sinyal stale = ML veya BTest tamamlandı ama sinyal ML'den daha eski
                        let sig_age  = ml_age.min(bt_age);
                        steps.push(make_step("signal", "Sinyal", sig_age, 0, false, heal_counts[6],
                            "[t] tuşu ile manuel sinyal testi"));
                    }

                    // ── Onarım loglarını live_pipeline'a yaz ──────────────────────
                    {
                        let write_result = live_pipeline.write();
                        if let Ok(mut ph) = write_result {
                            ph.chain_steps = steps;
                            for r in &repairs {
                                ph.log_repair(r);
                            }
                        }
                    }

                    // Çalışmadan sonra 60s bekle (ilk iterasyonda anında çalışır)
                    for _ in 0..60 {
                        if stop_chain.load(Ordering::Relaxed) { break; }
                        std::thread::sleep(std::time::Duration::from_secs(1));
                    }
                }
            });
        }

        // Market enum'ını belirle — active_trade_target'ın market'ını kullan
        let market_enum = if loop_market == "futures" {
            Market::Futures
        } else {
            Market::Spot
        };

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Tokio runtime oluşturulamadı");

        rt.block_on(async {
            // Orphan pozisyon WS besleyici: tokio runtime içinde spawn et
            // (primary loop ise singleton — secondary worker'lar atlar)
            if is_primary_loop {
                tokio::spawn(run_orphan_position_ws_feeds(
                    Arc::clone(&app_state),
                    Arc::clone(&stop_signal),
                ));
            }

            let mut loop_state = InMemoryStateManager::new();
            loop_state.set_balance(capital).ok();
            loop_state.set_symbols(vec![oto_cfg.symbol.clone()]).ok();

            let mut exec_state = InMemoryStateManager::new();
            exec_state.set_symbols(vec![loop_symbol.clone()]).ok(); // execute_basket'in sembol bulabilmesi için
            let binance_exec = BinanceTradeExecutor::new_for_market(
                api_key.clone(),
                api_secret.clone(),
                paper_mode,
                &loop_market,  // "spot" | "futures" — market-aware endpoint
            );
            let dummy_exec   = DummyTradeExecutor;
            let robotic_exec = if !api_key.is_empty() && !api_secret.is_empty() {
                RoboticTradeExecutor::with_state(&binance_exec, &exec_state, None)
            } else {
                RoboticTradeExecutor::with_state(&dummy_exec, &exec_state, None)
            };
            let reporter  = UniversalReporter;
            // Log dosyası: logs/rtc_cli.log (sabit isim, her çalışmada append)
            // Sadece primary worker yazar; secondary worker'lar None alır (dedup zaten var)
            let shared_log_path: Option<String> = if is_primary_loop {
                let _ = std::fs::create_dir_all("logs");
                Some("logs/rtc_cli.log".to_string())
            } else {
                None
            };
            // Orchestrator worker'ları FSM state/cycle_id parse etmez → secondary logger
            let logger = if is_primary_loop {
                SharedLogger::new(Arc::clone(&app_state), shared_log_path)
            } else {
                SharedLogger::new_secondary(Arc::clone(&app_state), None)
            };
            // DB cache adapter: market/symbol/interval config'den gelir, mumlar DB'ye yazılır
            let fetcher   = DbCachingLiveAdapter {
                inner: BinanceLiveAdapter::new(
                    Arc::clone(&stop_signal),
                    Arc::clone(&pause_signal),
                ),
                db_path:    db_path_clone.clone(),
                exchange:   loop_exchange.clone(),
                market_str: loop_market.clone(),
                market_enum,
                symbols:    vec![loop_symbol.clone()],
            };
            let strategy  = MaCrossoverStrategy;

            // HyperOpt'un bulduğu en iyi parametreleri ve otonom SL/TP oku
            let (cur_fast, cur_slow, cur_sl, cur_tp,
                 cur_rsi_period, cur_rsi_ob, cur_rsi_os,
                 cur_bb_period, cur_bb_std,
                 cur_macd_fast, cur_macd_slow, cur_macd_sig) = {
                let st = app_state.lock().unwrap();
                (st.best_fast, st.best_slow, st.best_sl, st.best_tp,
                 st.best_rsi_period, st.best_rsi_ob, st.best_rsi_os,
                 st.best_bb_period,  st.best_bb_std_dev,
                 st.best_macd_fast,  st.best_macd_slow, st.best_macd_signal)
            };

            let config = RoboticLoopConfig {
                interval_secs: loop_interval_secs,
                trade_amount: None, // risk_mgr notional-based sizing kullanılır (eski 0.01 BTC-centric'ti)
                interval: loop_interval.clone(),
                symbol: loop_symbol.clone(),
                market: market_enum,
                strategy_params: StrategyParams {
                    fast:          Some(cur_fast),
                    slow:          Some(cur_slow),
                    period:        Some(if cur_rsi_period > 0 { cur_rsi_period } else { 14 }),
                    overbought:    Some(if cur_rsi_ob   > 0.0 { cur_rsi_ob   } else { 70.0 }),
                    oversold:      Some(if cur_rsi_os   > 0.0 { cur_rsi_os   } else { 30.0 }),
                    fast_period:   Some(if cur_macd_fast  > 0 { cur_macd_fast  } else { 12 }),
                    slow_period:   Some(if cur_macd_slow  > 0 { cur_macd_slow  } else { 26 }),
                    signal_period: Some(if cur_macd_sig   > 0 { cur_macd_sig   } else { 9  }),
                    std_dev:       Some(if cur_bb_std > 0.0   { cur_bb_std     } else { 2.0 }),
                    bb_period:     Some(if cur_bb_period > 0  { cur_bb_period  } else { 20 }),
                },
                candle_limit: 500, // MA crossover için yeterli sinyal gerekir (eski 100 → her zaman skor=0 veriyordu)
                risk_params: RiskParams {
                    stop_loss_pct:          cur_sl,
                    take_profit_pct:        cur_tp,
                    max_position_size_pct:  Some(10.0), // Anlamlı pozisyon boyutu: sermayenin %10'u
                    max_portfolio_risk_pct: Some(20.0),
                    use_kelly_criterion:    true, // Kelly kriteri aktif: dinamik pozisyon boyutu
                    // Takipli SL: static SL ile aynı yüzdede başlar, kâr bölgesine girdikten sonra
                    // fiyatı yukarıdan takip eder (long için)
                    trailing_stop_pct:      if cur_sl > 0.0 { Some(cur_sl) } else { None },
                },
                capital: oto_cfg.capital,
                mode: RunMode::Live,
                autonomous_enabled: true, // 🧬 FSM + Evrimsel AI
                quality: TradeQualityConfig {
                    min_rr:                1.2,
                    volatility_min_pct:    0.01,  // 1m verisi için gevşetildi (BTC tipik 0.02-0.05%)
                    volatility_max_pct:    3.0,
                    trend_short_period:    20,
                    trend_long_period:     50,
                    trend_filter_enabled:  true,
                    trend_margin_pct:      2.0,   // %2.0 nötr bölge — SMA(20)/SMA(50) farkı %2 altındaysa her iki yön serbest
                    adaptive_enabled:      true,
                    min_rr_tight:          1.5,
                    min_rr_loose:          1.1,
                    volatility_max_tight:  2.0,
                    volatility_max_loose:  3.5,
                    win_rate_low:          40.0,
                    win_rate_high:         55.0,
                    volume_filter_enabled:       false,
                    volume_min_ratio:            0.7,
                    rsi_extreme_filter_enabled:  false,
                    rsi_extreme_ob:              80.0,
                    rsi_extreme_os:              20.0,
                    htf_require_alignment:       false,
                },
                trade_quality_config_path: Some(oto_cfg.trade_quality_config_path.clone()),
                position_profile:   Some("Balanced".to_string()),
                security_profile:   Some("Development".to_string()),
                allows_short:       oto_cfg.market == "futures", // futures → short açılabilir
                // Adaptif risk politikasını AppState'den taşı (Bug-1 fix)
                initial_risk_policy: Some({
                    let st = app_state.lock().unwrap();
                    st.risk_gate.policy
                }),
                // Snapshot'tan cycle_id — uygulama yeniden başlatıldığında saydığı yerden devam eder
                initial_cycle_id: {
                    let st = app_state.lock().unwrap();
                    st.controller.cycle_id
                },
                // Evrimsel AI: sadece birincil loop snapshot'tan devam eder.
                // Orchestrator worker'ları evolution almaz — her biri izole kopyada
                // ilerlese de AppState'e geri yazamaz; birincil loop yönetir.
                initial_brain: if is_primary_loop {
                    let st = app_state.lock().unwrap();
                    st.controller.adaptive_brain.clone()
                } else { None },
                initial_population: if is_primary_loop {
                    let st = app_state.lock().unwrap();
                    st.controller.population_manager.clone()
                } else { None },
                use_ml_signal: false, // TODO: TUI veya config'den kontrol edilecek
                // Binance komisyon oranı: Futures %0.04 taker, Spot %0.10 taker (VIP0, BNB ödemesiz)
                commission_pct: match market_enum {
                    Market::Futures => 0.0004, // %0.04 taker (VIP0)
                    _               => 0.001,  // %0.10 spot taker (VIP0)
                },
                // Spread + slippage + market impact — gerçekçi fiyat ayarlaması
                execution_cost_config: Some(match market_enum {
                    Market::Futures => memos_trading_core::robot::order_management::paper_executor::ExecutionCostConfig::binance_futures(),
                    _               => memos_trading_core::robot::order_management::paper_executor::ExecutionCostConfig::binance_spot(),
                }),
                // Merkezi canlı durum — tüm TUI paylaşımlı Arc kanalları tek noktada.
                // Evrim arc'ı: birincil loop AppState ile paylaşır; worker'lar izole kopya alır.
                live_state: {
                    let st = app_state.lock().unwrap();
                    // Çoklu sembol: orchestrator'dan per-sembol arc; tekil: genel live_price
                    let price_arc = st.orchestrator
                        .live_price_for(&loop_symbol)
                        .unwrap_or_else(|| Arc::clone(&st.live_price));
                    let evolution_arc = if is_primary_loop {
                        Arc::clone(&st.live_evolution)
                    } else {
                        Arc::new(std::sync::RwLock::new(
                            memos_trading_core::robot::robotic_loop::LiveEvolutionStatus::default()
                        ))
                    };
                    Some(Arc::new(memos_trading_core::robot::robotic_loop::TradingStateInner {
                        live_risk:            Arc::clone(&st.live_risk),
                        live_price:           price_arc,
                        live_positions:       Arc::clone(&st.live_positions),
                        live_strategy:        Arc::clone(&st.live_strategy),
                        live_regime_strategy: Arc::clone(&st.live_regime_strategy),
                        live_evolution:       evolution_arc,
                        live_active_symbol:   Arc::clone(&st.live_active_symbol),
                        live_signal_counts:   Arc::clone(&st.live_signal_counts),
                        live_trade_count:     Arc::clone(&st.live_trade_count),
                        live_closed_trades:   Arc::clone(&st.live_closed_trades),
                        live_execution_costs: Arc::clone(&st.live_execution_costs),
                        live_sr_zones:        Arc::clone(&st.live_sr_zones),
                        live_pipeline:        Arc::clone(&st.live_pipeline),
                    }))
                },
                // S/R filtresi — varsayılan olarak etkin, 5 swing lookback
                sr_config: memos_trading_core::robot::sr_detector::SrDetectorConfig::default(),
                // MTF filtresi için DB yolu — HTF candle'lar buradan okunur
                db_path: Some(db_path_clone.clone()),
                // Pozisyon yönetimi — robotic_profiles.json'dan gelir
                sl_cooldown_secs:      profile_cfg.sl_cooldown_secs,
                breakeven_at_rr:       profile_cfg.breakeven_at_rr,
                atr_trail_mult:        profile_cfg.atr_trail_mult,
                partial_tp_ratio:      profile_cfg.partial_tp_ratio,
                robotic_profiles_path:  Some(oto_cfg.robotic_profiles_path.clone()),
                adaptive_params_path:   Some(oto_cfg.adaptive_params_path.clone()),
                blocked_symbols:         oto_cfg.blocked_symbols.clone(),
                min_trade_interval_secs: None,
                max_open_positions: None,   // rtc_cli'de config'den okunabilir; şimdilik sınırsız
                max_spread_bps: None,       // rtc_cli'de config'den okunabilir; şimdilik guard kapalı
                scorer_state_path:     Some("config/strategy_scorer_state.json".to_string()),
                classifier_state_path: Some("config/classifier_state.json".to_string()),
                scalp_swing: Some(memos_trading_core::robot::scalp_swing::ScalpSwingConfig::default()),
                initial_open_positions: {
                    // Tüm pozisyonlar loop'a alınır — SL ihlal edilmişse saniye bazlı
                    // check_live_sl_tp / process_orphans gerçek zamanlı WS fiyatıyla kapatır.
                    let st = app_state.lock().unwrap();
                    st.live_positions.read().ok()
                        .map(|p| p.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<LivePositionMap>())
                        .unwrap_or_default()
                },
            };

            let mut loop_engine = RoboticLoop {
                executor:          &robotic_exec,
                state:             &mut loop_state,
                reporter:          &reporter,
                logger:            &logger,
                config,
                fetcher:           &fetcher,
                backtest_fetcher:  None,
                strategy:          &strategy,
                strategy_selector: None,
                ml_model:          None,
                ml_data:           None,
                portfolio:         None,
                autonomous_trader: None,
                monitor:           None,
                use_ml_signal: false, // TODO: TUI veya config'den kontrol edilecek (oto_cfg üzerinden: oto_cfg.use_ml_signal)
                paper_mode,
                interval_cycle_ids: [0u64; 7],
                telegram: None, // start() içinde from_env() ile yüklenir
            };

            // ↓ Gerçek trading döngüsü — Binance'e bağlanır, sinyal üretir
            loop_engine.start().await;
        });

        if let Ok(mut st) = app_state.lock() {
            st.push_log("♻ RoboticLoop döngüsü tamamlandı — otomatik yeniden başlatılabilir".to_string());
            // Loop bittikten sonra diskten brain yenile — döngü içinde öğrenilen
            // exploration_rate / Q-table kayıpları; AppState.controller stale kalır.
            // Sonraki loop başlatılacağında initial_brain buradan okunur.
            if is_primary_loop {
                if let Some((brain, pop, cycle)) = load_evolution_snapshot(&st.config_paths) {
                    st.controller.adaptive_brain = Some(brain);
                    st.controller.population_manager = Some(pop);
                    if cycle > 0 { st.controller.cycle_id = cycle; }
                }
            }
            // Yeni bir loop başlatıldıysa live_mode'u false yapma:
            // stop_signal (bu loop'a ait, true=durduruldu) ≠ st.stop_signal (yeni loop'un sinyal Arc'ı)
            if Arc::ptr_eq(&stop_signal, &st.stop_signal) {
                st.live_mode = false;
            }
        }
    });
}

// ─── TUI Renk Yardımcısı ─────────────────────────────────────────────────────

fn state_color(s: AutonomousState) -> Color {
    match s {
        AutonomousState::Observe => Color::Cyan,
        AutonomousState::Decide => Color::Blue,
        AutonomousState::Validate => Color::Yellow,
        AutonomousState::Execute => Color::Green,
        AutonomousState::Verify => Color::Magenta,
        AutonomousState::Adapt => Color::LightGreen,
        AutonomousState::SafeMode => Color::LightYellow,
        AutonomousState::Halted => Color::Red,
    }
}

fn draw_dashboard(f: &mut ratatui::Frame, area: Rect, st: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(12), // Bağlantı + FSM (10 satır + 2 border)
            Constraint::Length(11), // Risk politikası + SL/TP + Takipli SL
            Constraint::Min(16),    // Konfigürasyon (14 satır + 2 border) + OHLCV + Gauge
            Constraint::Length(5),  // PnL Equity Curve (Sparkline)
        ])
        .split(area);

    // ── Bağlantı + FSM ──────────────────────────────────────────────────────
    let fsm_color  = state_color(st.controller.state);
    let conn_color = if st.live_mode { Color::Green } else { Color::Red };

    let fsm_lines = vec![
        Line::from(vec![
            Span::styled("  RoboticLoop  : ", Style::default().fg(Color::White)),
            Span::styled(
                if st.live_mode { "✅ Çalışıyor" } else { "⭕ Başlatılıyor..." },
                Style::default().fg(conn_color),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Mod           : ", Style::default().fg(Color::White)),
            Span::styled(
                if st.paper_mode { "🧪 Kağıt (Paper)" } else { "🔴 CANLI (Live)" },
                Style::default().fg(if st.paper_mode { Color::Yellow } else { Color::Red }),
            ),
        ]),
        Line::from(vec![
            Span::styled("  API Key       : ", Style::default().fg(Color::White)),
            Span::styled(
                if st.api_key_set { "✅ Yüklendi" } else { "⚠️  YOK (kağıt mod)" },
                Style::default().fg(if st.api_key_set { Color::Green } else { Color::Yellow }),
            ),
        ]),
        Line::from(vec![
            Span::styled("  FSM Durumu   : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{}", st.controller.state),
                Style::default().fg(fsm_color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Cycle ID     : ", Style::default().fg(Color::White)),
            Span::styled(
                {
                    let elapsed = st.loop_active_since.elapsed().as_secs();
                    if elapsed < 120 {
                        format!("{} (yeni başladı, {}sn önce)", st.controller.cycle_id, elapsed)
                    } else {
                        let mins = elapsed / 60;
                        format!("{} ({}dk önce başlatıldı)", st.controller.cycle_id, mins)
                    }
                },
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Art. Hatalar : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{}", st.controller.consecutive_failures),
                Style::default().fg(if st.controller.consecutive_failures > 0 {
                    Color::LightYellow
                } else {
                    Color::White
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled("  can_trade()  : ", Style::default().fg(Color::White)),
            Span::styled(
                if st.controller.can_trade() { "✅ true" } else { "🚫 false" },
                Style::default().fg(if st.controller.can_trade() {
                    Color::Green
                } else {
                    Color::Red
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Duraklatıldı : ", Style::default().fg(Color::White)),
            Span::styled(
                if st.paused { "⏸ Evet" } else { "▶ Hayır" },
                Style::default().fg(if st.paused { Color::Yellow } else { Color::Green }),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Tanılama     : ", Style::default().fg(Color::White)),
            Span::styled(
                if st.diag_warn_count == 0 {
                    "✅ Sorun yok".to_string()
                } else {
                    format!("⚠ {} uyarı — Günlük'e bak [DIAG]", st.diag_warn_count)
                },
                Style::default().fg(if st.diag_warn_count == 0 {
                    Color::Green
                } else if st.diag_warn_count <= 2 {
                    Color::Yellow
                } else {
                    Color::LightRed
                }),
            ),
        ]),
        // ── Pipeline satırı ──────────────────────────────────────────────────
        {
            let pl = &st.pipeline;
            let phase_icon  = pl.phase.icon();
            let phase_label = pl.phase.label();
            let (phase_col, detail) = match &pl.phase {
                PipelinePhase::Idle => {
                    let secs = pl.next_run_at.saturating_duration_since(std::time::Instant::now()).as_secs();
                    if secs < 5 {
                        (Color::Yellow, "başlıyor...".to_string())
                    } else {
                        (Color::Blue, format!("{}dk sonra", secs / 60 + 1))
                    }
                }
                PipelinePhase::Download | PipelinePhase::Backtest | PipelinePhase::MLTrain => {
                    let elapsed = pl.phase_started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                    (Color::LightCyan, format!("{}sn...", elapsed))
                }
                PipelinePhase::P5Analysis => {
                    let total = pl.p5_symbols.len();
                    let idx   = pl.p5_sym_idx + 1;
                    let sym   = pl.p5_symbols.get(pl.p5_sym_idx)
                        .map(|s| s.2.clone()).unwrap_or_default();
                    let elapsed = pl.phase_started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                    (Color::LightMagenta, format!("{}/{} {} {}sn", idx, total, sym, elapsed))
                }
                PipelinePhase::Done => {
                    let secs = pl.next_run_at.saturating_duration_since(std::time::Instant::now()).as_secs();
                    let mins = secs / 60;
                    let last = pl.last_run_at.as_deref().unwrap_or("—");
                    (Color::Green, format!("#{} @ {}  sonraki {}dk", pl.runs_completed, last, mins))
                }
            };
            Line::from(vec![
                Span::styled("  Pipeline     : ", Style::default().fg(Color::White)),
                Span::styled(
                    format!("{} {}  ", phase_icon, phase_label),
                    Style::default().fg(phase_col).add_modifier(Modifier::BOLD),
                ),
                Span::styled(detail, Style::default().fg(Color::LightBlue)),
                Span::styled(
                    if pl.enabled { "  [w]=tetikle" } else { "  DEVRE DIŞI" },
                    Style::default().fg(Color::Blue),
                ),
            ])
        },
    ];

    f.render_widget(
        Paragraph::new(fsm_lines).block(
            Block::default()
                .title(" 🤖 RoboticLoop — FSM (log parse) ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(fsm_color)),
        ),
        chunks[0],
    );

    // ── Risk Politikası ──────────────────────────────────────────────────────
    let risk_lines = vec![
        Line::from(vec![
            Span::styled("  Max Notional : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("${:.0}", st.risk_gate.policy.max_notional_usd),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Max Day Loss : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{:.1}%", st.risk_gate.policy.max_daily_loss_pct),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Max Drawdown : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{:.1}%", st.risk_gate.policy.max_drawdown_pct),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Min Confid.  : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{:.2}", st.risk_gate.policy.min_model_confidence),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Stop Loss    : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{:.1}%", st.best_sl),
                Style::default().fg(if st.best_sl <= 2.0 { Color::Green } else { Color::Yellow }),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Take Profit  : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{:.1}% (rr={:.1}x)", st.best_tp, if st.best_sl > 0.0 { st.best_tp / st.best_sl } else { 0.0 }),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::styled("  MA Periyot   : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("fast={} / slow={}", st.best_fast, st.best_slow),
                Style::default().fg(Color::Magenta),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Toplam Trade : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{}", st.total_trades),
                Style::default().fg(Color::Cyan),
            ),
        ]),
    ];

    f.render_widget(
        Paragraph::new(risk_lines).block(
            Block::default()
                .title(" 🛡 Risk Politikası ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        ),
        chunks[1],
    );

    // ── Konfigürasyon + Gauge ────────────────────────────────────────────────
    let stat_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[2]);

    let stat_lines = vec![
        Line::from(vec![
            Span::styled("  Otonom Mod   : ", Style::default().fg(Color::White)),
            Span::styled(
                if st.auto_symbol { "✅ AUTO (sistem seçiyor)" } else { "📌 MANUEL (config.json)" },
                Style::default().fg(if st.auto_symbol { Color::LightGreen } else { Color::Yellow }),
            ),
        ]),
        {
            let best_candidate = st.symbol_candidates.first();
            let active_sym = &st.active_symbol.symbol;
            let active_int = &st.active_symbol.interval;
            let pending_switch = st.auto_symbol
                && best_candidate.map(|c| c.symbol.as_str() != active_sym.as_str()).unwrap_or(false);
            if pending_switch {
                let best = best_candidate.unwrap();
                Line::from(vec![
                    Span::styled("  Aktif Hedef  : ", Style::default().fg(Color::White)),
                    Span::styled(
                        format!("{} | {}", active_sym, active_int),
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  →  {} (geçiş bekl.)", best.symbol),
                        Style::default().fg(Color::Yellow),
                    ),
                ])
            } else {
                let score_hint = if st.active_symbol.score == 0.0
                    && !st.active_symbol.symbol.is_empty()
                    && st.active_symbol.total_trades < 10
                {
                    "  ⚠ veri az — backtest bekliyor"
                } else { "" };
                Line::from(vec![
                    Span::styled("  Aktif Hedef  : ", Style::default().fg(Color::White)),
                    Span::styled(
                        format!("{} | {}", active_sym, active_int),
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        score_hint,
                        Style::default().fg(Color::Yellow),
                    ),
                ])
            }
        },
        Line::from(vec![
            Span::styled("  Aday Sayısı  : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{} kombinasyon puanlandı", st.symbol_candidates.len()),
                Style::default().fg(Color::White),
            ),
        ]),
        {
            let wc  = st.orchestrator.worker_count();
            let max = st.orchestrator.max_workers;
            let pnl = st.orchestrator.total_open_pnl(Some(&*st.live_price));
            let realize = st.equity - st.config.capital;
            let (wc_color, wc_label) = if wc == 0 {
                (Color::LightBlue, format!("0 / {} — bekliyor", max))
            } else {
                let pnl_str = format!(" | Açık: {:+.2}$ | Realize: {:+.2}$", pnl, realize);
                (Color::LightMagenta, format!("{} / {} aktif{}", wc, max, pnl_str))
            };
            Line::from(vec![
                Span::styled("  Worker       : ", Style::default().fg(Color::White)),
                Span::styled(wc_label, Style::default().fg(wc_color).add_modifier(Modifier::BOLD)),
                Span::styled("  [6]", Style::default().fg(Color::White)),
            ])
        },
        Line::from(vec![
            Span::styled("  Exchange     : ", Style::default().fg(Color::White)),
            Span::styled(
                { let (ae, am, _, _) = st.active_trade_target(); format!("{}/{}", ae, am) },
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Konfig Int.  : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("({} — ayarlar)", st.config.interval),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Strateji     : ", Style::default().fg(Color::White)),
            Span::styled(
                {
                    let sname = st.live_strategy.read().ok()
                        .map(|s| s.clone())
                        .unwrap_or_else(|| "MA".to_string());
                    match sname.as_str() {
                        "MA" | "MA_CROSSOVER" =>
                            format!("MA_CROSSOVER (fast={}, slow={})", st.best_fast, st.best_slow),
                        "RSI"        => format!("RSI (period={}, ob={}, os={})", st.best_rsi_period, st.best_rsi_ob as u32, st.best_rsi_os as u32),
                        "MACD"       => format!("MACD ({}/{}/{})", st.best_macd_fast, st.best_macd_slow, st.best_macd_signal),
                        "BB"         => format!("Bollinger ({}, {:.1}σ)", st.best_bb_period, st.best_bb_std_dev),
                        "SUPERTREND" => format!("Supertrend (atr={}, mult={:.1})", st.best_rsi_period.max(10), st.best_bb_std_dev.max(2.0).min(4.0)),
                        "EMA"        => format!("EMA Cross (fast={}, slow={})", st.best_fast, st.best_slow),
                        "STOCH_RSI"  => format!("StochRSI (rsi={}, ob={}, os={})", st.best_rsi_period, st.best_rsi_ob as u32, st.best_rsi_os as u32),
                        "CCI"        => "CCI (period=20, ±100)".to_string(),
                        "DONCHIAN"   => "Donchian Channel".to_string(),
                        other        => other.to_string(),
                    }
                },
                Style::default().fg(Color::Cyan),
            ),
        ]),
        // Rejim bazlı aktif strateji + onay stratejileri
        Line::from(vec![
            Span::styled("  Rejim/Aktif  : ", Style::default().fg(Color::White)),
            Span::styled(
                {
                    let regime_strat = st.live_regime_strategy.read().ok()
                        .map(|s| s.clone())
                        .unwrap_or_else(|| "—".to_string());
                    let confirms = if regime_strat.contains("low_vol") {
                        "onay: RSI+BB+StochRSI"
                    } else if regime_strat.contains("high_vol") {
                        "onay: Supertrend+MACD+EMA"
                    } else {
                        "onay: Supertrend+MACD"
                    };
                    format!("{} | {}", regime_strat, confirms)
                },
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Otonom AI    : ", Style::default().fg(Color::White)),
            Span::styled(
                "🧬 AdaptiveBrain + PopulationManager",
                Style::default().fg(Color::LightGreen),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Executor     : ", Style::default().fg(Color::White)),
            Span::styled(
                if st.api_key_set {
                    if st.paper_mode { "BinanceTrade (paper)" } else { "🔴 BinanceTrade (CANLI)" }
                } else {
                    "🧪 DummyExecutor (geliştirme)"
                },
                Style::default().fg(if st.api_key_set && !st.paper_mode {
                    Color::Red
                } else if st.api_key_set {
                    Color::Yellow
                } else {
                    Color::LightBlue
                }),
            ),
        ]),
        {
            let realize = st.equity - st.config.capital;
            let realize_col = if realize >= 0.0 { Color::LightGreen } else { Color::LightRed };
            Line::from(vec![
                Span::styled("  Sermaye      : ", Style::default().fg(Color::White)),
                Span::styled(format!("${:.2}", st.equity),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" (başl=${:.2} ", st.config.capital),
                    Style::default().fg(Color::DarkGray)),
                Span::styled(format!("realize={:+.2}$", realize),
                    Style::default().fg(realize_col)),
                Span::styled(")", Style::default().fg(Color::DarkGray)),
            ])
        },
        Line::from(vec![
            Span::styled("  🔬 Son Backtest : ", Style::default().fg(Color::White)),
            Span::styled(
                st.last_backtest.as_deref().unwrap_or("Henüz çalışmadı"),
                Style::default().fg(if st.last_backtest.is_some() { Color::LightGreen } else { Color::LightBlue }),
            ),
        ]),
        {
            // Interval öneri satırı — sadece veri varsa ve mevcut interval farklıysa göster
            let (_, _, _cur_sym_d, cur_intv_d) = st.active_trade_target();
            if let Some((ref bi, bscore, bwr, bpnl)) = st.best_interval_rec {
                let is_diff = bi != &cur_intv_d;
                let (label, color) = if is_diff {
                    (format!("💡 {} → {} önerisi (skor={:.2} win={:.0}% pnl={:+.0}$){}",
                        cur_intv_d, bi, bscore, bwr, bpnl,
                        if st.auto_interval { " [OTO]" } else { " [e→Ayarlar]" }),
                     Color::LightYellow)
                } else {
                    (format!("✅ {} en iyi interval (skor={:.2} win={:.0}%)", bi, bscore, bwr),
                     Color::LightGreen)
                };
                Line::from(vec![
                    Span::styled("  📊 Interval    : ", Style::default().fg(Color::White)),
                    Span::styled(label, Style::default().fg(color)),
                ])
            } else {
                Line::from(vec![
                    Span::styled("  📊 Interval    : ", Style::default().fg(Color::White)),
                    Span::styled("analiz bekleniyor...", Style::default().fg(Color::White)),
                ])
            }
        },
        {
            // HTF trend yönü + filtre durumu
            let (htf_bias_str, htf_color, htf_enabled) = st.live_risk.read().ok()
                .map(|lrm| {
                    let (s, c) = match lrm.htf_trend_bias {
                        Some(1)  => ("Bullish ↑".to_string(), Color::LightGreen),
                        Some(-1) => ("Bearish ↓".to_string(), Color::LightRed),
                        Some(0)  => ("Neutral →".to_string(), Color::Yellow),
                        _        => ("bekleniyor".to_string(), Color::LightBlue),
                    };
                    (s, c, lrm.htf_filter_enabled)
                })
                .unwrap_or(("bekleniyor".to_string(), Color::LightBlue, true));
            let htf_intv = {
                let intv = st.config.interval.as_str();
                match intv {
                    "1m" | "5m"   => "1h",
                    "15m" | "30m" => "4h",
                    "1h"          => "4h",
                    _             => "1d",
                }
            };
            let filter_badge = if htf_enabled { "[filtre: AÇIK]" } else { "[filtre: KAPALI]" };
            Line::from(vec![
                Span::styled(format!("  🌐 HTF ({htf_intv})   : "), Style::default().fg(Color::White)),
                Span::styled(format!("{htf_bias_str}  {filter_badge}"), Style::default().fg(htf_color)),
            ])
        },
        {
            // Dinamik kaldıraç durum satırı — effective_leverage gerçek zamanlı
            let (base_lev, max_lev, eff_lev) = st.live_risk.read().ok()
                .map(|lrm| (lrm.base_leverage, lrm.max_leverage, lrm.effective_leverage))
                .unwrap_or((7.0, 10.0, 7.0));
            let lev_color = if eff_lev >= 9.0 { Color::LightRed }
                else if eff_lev >= 7.5 { Color::LightYellow }
                else { Color::LightGreen };
            Line::from(vec![
                Span::styled("  ⚡ Kaldıraç    : ", Style::default().fg(Color::White)),
                Span::styled(
                    format!("{:.1}x–{:.1}x  anlık={:.1}x", base_lev, max_lev, eff_lev),
                    Style::default().fg(lev_color),
                ),
            ])
        },
        Line::from(vec![
            Span::styled("  ⏬ Son İndirme :  ", Style::default().fg(Color::White)),
            Span::styled(
                {
                    let dl_text = if st.download_active {
                        "⏳ İndiriliyor...".to_string()
                    } else if let Some(ref last) = st.last_download {
                        let secs_left = st.download_next_at.saturating_duration_since(Instant::now()).as_secs();
                        if secs_left > 0 {
                            format!("{}  (sonraki: {}dk {}sn)", last, secs_left / 60, secs_left % 60)
                        } else {
                            last.clone()
                        }
                    } else {
                        let secs_left = st.download_next_at.saturating_duration_since(Instant::now()).as_secs();
                        if secs_left > 0 {
                            format!("İlk indirme {}sn içinde başlayacak...", secs_left)
                        } else {
                            "İndirme hazırlanıyor...".to_string()
                        }
                    };
                    dl_text
                },
                Style::default().fg(if st.download_active { Color::Yellow }
                    else if st.last_download.is_some() { Color::LightGreen }
                    else { Color::LightBlue }),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Toplam İndir. : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{} mum", st.download_count),
                Style::default().fg(Color::White),
            ),
        ]),
    ];

    // wrap kaldırıldı: uzun satırlar widget sınırında kırpılır, taşma olmaz
    f.render_widget(
        Paragraph::new(stat_lines).block(
            Block::default()
                .title(" ⚙️  Konfigürasyon ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        ),
        stat_chunks[0],
    );

    // ── Sağ panel: OHLCV fiyat + Gauge ──────────────────────────────────────
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(stat_chunks[1]);

    // ── Sağ panel: fiyat kutusu ──────────────────────────────────────────────
    // Tek worker → detaylı OHLCV  |  Çoklu worker → kompakt satır listesi
    let worker_count_dash = st.orchestrator.worker_count();
    let (price_lines, price_title, price_border) = if worker_count_dash <= 1 {
        // ── Tek sembol: tam OHLCV ────────────────────────────────────────────
        // Orchestrator arc'ından oku (WS buraya yazar); yoksa st.live_price fallback
        let (_, _, active_sym_dash, _) = st.active_trade_target();
        let pd_cloned: Option<LivePriceData> =
            st.orchestrator.live_price_for(&active_sym_dash)
                .and_then(|arc| arc.read().ok().map(|p| p.clone()))
                .filter(|p| p.close > 0.0)
                .or_else(|| st.live_price.read().ok()
                    .filter(|p| p.close > 0.0).map(|p| p.clone()));
        let lines: Vec<Line> = if let Some(ref p) = pd_cloned {
            if p.close > 0.0 {
                let chg_color = if p.change_pct >= 0.0 { Color::LightGreen } else { Color::LightRed };
                let chg_sym   = if p.change_pct >= 0.0 { "▲" } else { "▼" };
                vec![
                    Line::from(vec![
                        Span::styled("  Sembol  : ", Style::default().fg(Color::White)),
                        Span::styled(p.symbol.clone(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("  @ {} (Yerel)", p.ts), Style::default().fg(Color::White)),
                    ]),
                    Line::from(vec![
                        Span::styled("  Fiyat   : ", Style::default().fg(Color::White)),
                        Span::styled(
                            format!("{:.4}", p.close),
                            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                        ),
                        Span::raw("  "),
                        Span::styled(
                            format!("{}{:.2}%", chg_sym, p.change_pct.abs()),
                            chg_color,
                        ).add_modifier(Modifier::BOLD),
                    ]),
                    Line::from(vec![
                        Span::styled("  Open    : ", Style::default().fg(Color::White)),
                        Span::styled(format!("{:.4}", p.open), Style::default().fg(Color::White)),
                    ]),
                    Line::from(vec![
                        Span::styled("  High    : ", Style::default().fg(Color::White)),
                        Span::styled(format!("{:.4}", p.high), Style::default().fg(Color::LightGreen)),
                    ]),
                    Line::from(vec![
                        Span::styled("  Low     : ", Style::default().fg(Color::White)),
                        Span::styled(format!("{:.4}", p.low), Style::default().fg(Color::LightRed)),
                    ]),
                    Line::from(vec![
                        Span::styled("  Close   : ", Style::default().fg(Color::White)),
                        Span::styled(format!("{:.4}", p.close), Style::default().fg(chg_color)),
                    ]),
                    Line::from(vec![
                        Span::styled("  Volume  : ", Style::default().fg(Color::White)),
                        Span::styled(
                            if p.volume >= 1_000_000.0 { format!("{:.2}M", p.volume / 1_000_000.0) }
                            else if p.volume >= 1_000.0 { format!("{:.2}K", p.volume / 1_000.0) }
                            else { format!("{:.4}", p.volume) },
                            Style::default().fg(Color::White),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("  ML Sinyal: ", Style::default().fg(Color::White)),
                        Span::styled(
                            format!("{} (conf={:.3})", st.ml_signal, st.ml_confidence),
                            Style::default().fg(match st.ml_signal.as_str() {
                                "BUY"  => Color::LightGreen,
                                "SELL" => Color::LightRed,
                                _      => Color::LightBlue,
                            }).add_modifier(Modifier::BOLD),
                        ),
                    ]),
                ]
            } else {
                vec![Line::from(Span::styled("  Fiyat bekleniyor...", Style::default().fg(Color::White)))]
            }
        } else {
            vec![Line::from(Span::styled("  Veri yok", Style::default().fg(Color::White)))]
        };
        (lines, " 📈 Canlı Fiyat ".to_string(), Color::LightGreen)
    } else {
        // ── Çoklu sembol: kompakt satır listesi ─────────────────────────────
        // Her worker için bir satır: "  SEM  fiyat  ▲/▼ chg%  vol"
        let mut lines: Vec<Line> = vec![
            Line::from(vec![
                Span::styled(
                    format!("  {:<10}  {:>12}  {:>8}  {:>9}  {}",
                        "Sembol", "Fiyat", "Değ%", "Hacim", ""),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
            ]),
        ];

        // Önce primary sembol — orchestrator arc > st.live_price (WS her zaman orchestrator'a yazar)
        let (_, _, active_sym_d, _) = st.active_trade_target();
        let primary_pd = st.orchestrator.live_price_for(&active_sym_d)
            .and_then(|arc| arc.read().ok().map(|p| (p.symbol.clone(), p.close, p.change_pct, p.volume)))
            .filter(|p| p.1 > 0.0)
            .or_else(|| st.live_price.read().ok()
                .filter(|p| p.close > 0.0)
                .map(|p| (p.symbol.clone(), p.close, p.change_pct, p.volume)));
        if let Some((sym_d, close_d, chg_d, vol_d)) = primary_pd {
            let chg_color = if chg_d >= 0.0 { Color::LightGreen } else { Color::LightRed };
            let chg_sym   = if chg_d >= 0.0 { "▲" } else { "▼" };
            let vol_s = if vol_d >= 1_000_000.0 { format!("{:.1}M", vol_d / 1_000_000.0) }
                        else if vol_d >= 1_000.0 { format!("{:.1}K", vol_d / 1_000.0) }
                        else { format!("{:.2}", vol_d) };
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<10} ", sym_d), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{:>12.4}  ", close_d), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{}{:.2}%  ", chg_sym, chg_d.abs()), Style::default().fg(chg_color).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{:>9}", vol_s), Style::default().fg(Color::White)),
                Span::styled("  ◀", Style::default().fg(Color::Cyan)),
            ]));
        }

        // Orchestrator workers
        for status in st.orchestrator.worker_status() {
            if status.symbol == active_sym_d { continue; }
            let (close, chg, vol) = if let Some(arc) = st.orchestrator.live_price_for(&status.symbol) {
                arc.read().ok()
                    .map(|p| (p.close, p.change_pct, p.volume))
                    .unwrap_or_default()
            } else { (0.0, 0.0, 0.0) };

            if close > 0.0 {
                let chg_color = if chg >= 0.0 { Color::LightGreen } else { Color::LightRed };
                let chg_sym   = if chg >= 0.0 { "▲" } else { "▼" };
                let vol_s = if vol >= 1_000_000.0 { format!("{:.1}M", vol / 1_000_000.0) }
                            else if vol >= 1_000.0 { format!("{:.1}K", vol / 1_000.0) }
                            else { format!("{:.2}", vol) };
                let state_mark = if status.paused { " ⏸" } else { "" };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<10} ", status.symbol), Style::default().fg(Color::White)),
                    Span::styled(format!("{:>12.4}  ", close), Style::default().fg(Color::White)),
                    Span::styled(format!("{}{:.2}%  ", chg_sym, chg.abs()), Style::default().fg(chg_color)),
                    Span::styled(format!("{:>9}", vol_s), Style::default().fg(Color::White)),
                    Span::styled(state_mark.to_string(), Style::default().fg(Color::Yellow)),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {:<10} ", status.symbol), Style::default().fg(Color::White)),
                    Span::styled("  fiyat bekleniyor...", Style::default().fg(Color::White)),
                ]));
            }
        }

        // ML sinyali kompakt
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  ML: ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{} ({:.2})", st.ml_signal, st.ml_confidence),
                Style::default().fg(match st.ml_signal.as_str() {
                    "BUY"  => Color::LightGreen,
                    "SELL" => Color::LightRed,
                    _      => Color::LightBlue,
                }).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  → detay: [6]", Style::default().fg(Color::White)),
        ]));

        let title = format!(" 🌐 Çoklu Fiyat ({} sembol) ", worker_count_dash + 1);
        (lines, title, Color::Magenta)
    };

    f.render_widget(
        Paragraph::new(price_lines).block(
            Block::default()
                .title(price_title.as_str())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(price_border)),
        ),
        right_chunks[0],
    );

    // Açık pozisyonların toplam anlık PnL'ini hesapla
    // Fiyat kaynağı: orchestrator per-sembol live_price arc → pos.current_price (fallback)
    let mut open_pnl = 0.0;
    {
        let price_map = st.orchestrator.build_price_map(Some(&st.live_price));
        if let Ok(lm) = st.live_positions.read() {
            for pos in lm.values() {
                let cur = price_map.get(&pos.symbol).copied()
                    .filter(|&v| v > 0.0)
                    .unwrap_or(pos.current_price);
                open_pnl += pos_pnl(cur, pos.entry_price, pos.qty, pos.is_long);
            }
        }
    }
    // st.equity = başlangıç_sermayesi + realize_pnl (diagnostic worker 10s'de günceller)
    // total_equity = realize + açık pozisyon PnL
    let total_equity = st.equity + open_pnl;

    // Kümülatif işlem maliyeti özeti
    let costs = st.live_execution_costs.read().ok()
        .map(|c| c.clone())
        .unwrap_or_default();

    // Layout: Sermaye gauge + maliyet satırı
    let sermaye_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(right_chunks[1]);

    f.render_widget(
        Gauge::default()
            .block(
                Block::default()
                    .title(" 💰 Sermaye ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .gauge_style(Style::default().fg(Color::Green).bg(Color::DarkGray))
            .percent(100)
            .label(format!("${:.2}  [Realize: {:+.2}  Açık: {:+.2}]",
                total_equity,
                st.equity - st.config.capital,
                open_pnl)),
        sermaye_chunks[0],
    );

    // Maliyet özeti satırı — REG/SCP/SWG ayrımlı
    let cost_line = if costs.trade_count > 0 {
        format!(
            " Maliyet: REG ${:.2}/{} • SCP ${:.2}/{} • SWG ${:.2}/{} | Kom=${:.2} Slip=${:.2} Toplam=${:.2} (ort=${:.2})",
            costs.regular.total_usd, costs.regular.trade_count,
            costs.scalp.total_usd,   costs.scalp.trade_count,
            costs.swing.total_usd,   costs.swing.trade_count,
            costs.total_commission,
            costs.total_slippage,
            costs.total_cost_usd,
            costs.avg_cost_per_trade,
        )
    } else {
        " Maliyet: — (henüz işlem yok)".to_string()
    };
    f.render_widget(
        Paragraph::new(cost_line)
            .style(Style::default().fg(Color::White)),
        sermaye_chunks[1],
    );

    // ── PnL Equity Curve (Sparkline) ─────────────────────────────────────────
    draw_pnl_sparkline(f, chunks[3], st, total_equity);
}

fn draw_mtf_opportunities(f: &mut ratatui::Frame, area: Rect, st: &AppState) {
    let last_scan = st.mtf_last_scan.as_deref().unwrap_or("—");
    let opps = &st.mtf_opportunities;
    // Progress string "(X/Y)" içeriyorsa tarama devam ediyor demektir
    let in_progress = last_scan.contains('/');

    let active_signals = opps.iter().filter(|o| o.live_signal != "-").count();
    let alert_badge = if active_signals > 0 {
        format!(" 🚨{}SİNYAL ", active_signals)
    } else {
        String::new()
    };
    let title = format!(" 🔭 MTF Fırsatlar ({}){} — {} ",
        opps.len(),
        alert_badge,
        if in_progress {
            format!("taranıyor {}...", last_scan)
        } else {
            format!("son: {}  [u]=yenile", last_scan)
        });

    if opps.is_empty() {
        let cand_count = st.symbol_candidates.len();
        let msg = if st.mtf_last_scan.is_none() {
            if cand_count == 0 {
                "  Sembol adayı yok — önce [d] veri indir, [w] pipeline çalıştır".to_string()
            } else {
                format!("  {} aday hazır — [m] ile taramayı başlat", cand_count)
            }
        } else {
            format!("  {} aday tarandı, skor > 0.15 eşiğini geçen bulunamadı — [m] yenile", cand_count)
        };
        let p = Paragraph::new(msg.as_str())
            .style(Style::default().fg(Color::Blue))
            .block(Block::default().borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(Color::Blue)));
        f.render_widget(p, area);
        return;
    }

    // Her fırsat için tek satır: Sembol | Interval | Strateji | Skor | WR | Yön | Zaman
    let display_n = ((area.height as usize).saturating_sub(2)).min(opps.len());
    let mut lines: Vec<Line> = Vec::with_capacity(display_n);
    for opp in opps.iter().take(display_n) {
        let dir_color = match opp.direction.as_str() {
            "LONG"  => Color::LightGreen,
            "SHORT" => Color::LightRed,
            _       => Color::LightBlue,
        };
        let score_color = if opp.score >= 0.45 {
            Color::LightGreen
        } else if opp.score >= 0.30 {
            Color::Cyan
        } else {
            Color::Yellow
        };
        let (sig_label, sig_color) = match opp.live_signal.as_str() {
            "BUY"  => ("▲BUY ", Color::LightGreen),
            "SELL" => ("▼SELL", Color::LightRed),
            _      => ("  —  ", Color::DarkGray),
        };
        let sig_time = opp.signal_at.as_deref().unwrap_or("");
        lines.push(Line::from(vec![
            Span::styled(format!("  {:10}", opp.symbol),   Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {:4}",   opp.interval), Style::default().fg(Color::LightBlue)),
            Span::styled(format!(" {:14}",  &opp.strategy[..opp.strategy.len().min(14)]), Style::default().fg(Color::Cyan)),
            Span::styled(format!(" skor={:.2}",  opp.score),  Style::default().fg(score_color)),
            Span::styled(format!(" wr={:.0}%",   opp.win_rate * 100.0), Style::default().fg(Color::LightBlue)),
            Span::styled(format!(" {:5}",  opp.direction), Style::default().fg(dir_color).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" [{}]", sig_label), Style::default().fg(sig_color).add_modifier(if opp.live_signal != "-" { Modifier::BOLD } else { Modifier::empty() })),
            Span::styled(
                if sig_time.is_empty() { String::new() } else { format!(" {}", sig_time) },
                Style::default().fg(Color::Yellow),
            ),
        ]));
    }
    if opps.len() > display_n {
        lines.push(Line::from(Span::styled(
            format!("  ... +{} daha", opps.len() - display_n),
            Style::default().fg(Color::Blue),
        )));
    }

    let block_color = if active_signals > 0 { Color::LightYellow }
                      else if !opps.is_empty() { Color::LightGreen }
                      else { Color::Blue };
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(block_color)),
        ),
        area,
    );
}

fn draw_pnl_sparkline(f: &mut ratatui::Frame, area: Rect, st: &AppState, current_equity: f64) {
    use ratatui::widgets::Sparkline;

    if st.pnl_snapshots.is_empty() {
        let p = Paragraph::new("  PnL geçmişi yükleniyor... (30s'de bir güncellenir)")
            .style(Style::default().fg(Color::White))
            .block(Block::default().borders(Borders::ALL)
                .title(" 📊 PnL Equity Curve ")
                .border_style(Style::default().fg(Color::White)));
        f.render_widget(p, area);
        return;
    }

    // PnL değerlerini al, normalize et (0..=u64::MAX aralığına)
    let pnl_vals: Vec<f64> = st.pnl_snapshots.iter().map(|(_, pnl, _)| *pnl).collect();
    let min_pnl = pnl_vals.iter().cloned().fold(f64::MAX, f64::min);
    let max_pnl = pnl_vals.iter().cloned().fold(f64::MIN, f64::max);
    let range = (max_pnl - min_pnl).max(1.0);

    // Sparkline u64 istiyor — [0, 100] aralığına normalize
    let data: Vec<u64> = pnl_vals.iter()
        .map(|&v| ((v - min_pnl) / range * 100.0) as u64)
        .collect();

    // Son değer pozitif mi negatif mi
    let last_pnl = pnl_vals.last().copied().unwrap_or(0.0);
    let spark_color = if last_pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };

    let initial = st.config.capital;
    let realize_pnl = current_equity - initial;
    let title = format!(
        " 📊 PnL Equity Curve  Sermaye: ${:.2}  Realize: {:+.2}$  ({} nokta, {:.0}dk) ",
        current_equity,
        realize_pnl,
        pnl_vals.len(),
        pnl_vals.len() as f64 * 0.5, // 30s'de bir → dakika
    );

    let sparkline = Sparkline::default()
        .block(Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if realize_pnl >= 0.0 { Color::Green } else { Color::Red })))
        .data(&data)
        .style(Style::default().fg(spark_color));

    f.render_widget(sparkline, area);
}

#[allow(dead_code)]
fn draw_evolution(f: &mut ratatui::Frame, area: Rect, st: &AppState) {
    let evo = st.live_evolution.read().ok()
        .map(|g| g.clone())
        .unwrap_or_default();

    // ── ML köprüsü: live_risk'ten GBT + OOS oku ─────────────────────────────
    let (ml_running, gbt_score, oos_wr, oos_bc) =
        st.live_risk.read().ok()
            .map(|r| (r.ml_running, r.gbt_last_score, r.oos_win_rate, r.oos_bar_count))
            .unwrap_or((false, None, 0.0, 0));
    let ml_last_train = st.last_ml_train.clone();
    // Sonraki ML çalışmasına kalan süre (sn)
    let ml_next_secs = st.ml_next_run_at.saturating_duration_since(std::time::Instant::now()).as_secs();

    // ── Yardımcı: "k: v" şeklindeki özet string'inden değer çek ─────────────
    let extract = |s: &str, key: &str| -> String {
        s.split(',')
            .find_map(|part| {
                let part = part.trim();
                if let Some(rest) = part.strip_prefix(key) {
                    Some(rest.trim_start_matches(':').trim().to_string())
                } else { None }
            })
            .unwrap_or_else(|| "—".to_string())
    };

    // ── Brain alanları ────────────────────────────────────────────────────────
    let b = &evo.brain_summary;
    let regime      = extract(b, "Rejim");
    let qtable      = extract(b, "Q-table boyutu");
    let steps       = extract(b, "Öğrenme adımı");
    let exploration = extract(b, "Exploration");   // "18.3%"
    let avg_reward  = extract(b, "Ortalama reward (son 100)");

    // Keşif oranını sayıya çevir (bar için)
    let expl_f: f64 = exploration.trim_end_matches('%').parse().unwrap_or(0.0);

    // ── Popülasyon alanları ───────────────────────────────────────────────────
    let p = &evo.pop_summary;
    let generation  = extract(p, "Nesil");
    let pop_size    = extract(p, "Popülasyon");
    let best_fit    = extract(p, "En İyi Fitness");
    let avg_fit     = extract(p, "Ortalama");
    let hof         = extract(p, "Hall of Fame");

    // ── Progress bar yardımcısı (Unicode blok dolumu) ─────────────────────────
    let bar = |ratio: f64, width: usize| -> String {
        let filled = ((ratio.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
        format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
    };

    // ── Döngü ilerlemesi ──────────────────────────────────────────────────────
    let (cycles_done, cycles_left, cycle_bar) = if evo.evolve_every_n_cycles > 0 {
        let done = evo.cycle_id % evo.evolve_every_n_cycles;
        let left = evo.evolve_every_n_cycles - done;
        let ratio = done as f64 / evo.evolve_every_n_cycles as f64;
        (done, left, bar(ratio, 20))
    } else {
        (0, 0, "░".repeat(20))
    };

    // ── Fitness rengi ─────────────────────────────────────────────────────────
    let fit_color = if evo.genome_fitness >= 0.5 { Color::Green }
        else if evo.genome_fitness >= 0.2 { Color::Yellow }
        else { Color::Red };

    let win_color = if evo.genome_win_rate >= 50.0 { Color::Green }
        else if evo.genome_win_rate >= 35.0 { Color::Yellow }
        else { Color::Red };

    let reward_color = if avg_reward.trim_start_matches('+').parse::<f64>().unwrap_or(0.0) > 0.0 {
        Color::Green
    } else { Color::Red };

    let divider = "  ─────────────────────────────────────────────────────────────────────";

    let lines: Vec<Line> = vec![
        // ── Başlık ────────────────────────────────────────────────────────────
        Line::from(""),
        Line::from(vec![
            Span::styled("  ┌── SİSTEM DURUMU ", Style::default().fg(Color::Blue)),
            Span::styled(
                if evo.evolution_enabled { "● EVRİM AKTİF" } else { "○ EVRİM PASİF" },
                Style::default().fg(if evo.evolution_enabled { Color::LightGreen } else { Color::Red })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  │  Brain: ", Style::default().fg(Color::Blue)),
            Span::styled(
                if evo.brain_active { "●" } else { "○" },
                Style::default().fg(if evo.brain_active { Color::Green } else { Color::LightBlue }),
            ),
            Span::styled("  Pop: ", Style::default().fg(Color::Blue)),
            Span::styled(
                if evo.pop_active { "●" } else { "○" },
                Style::default().fg(if evo.pop_active { Color::Green } else { Color::LightBlue }),
            ),
        ]),
        Line::from(""),

        // ── Aktif Genom ───────────────────────────────────────────────────────
        Line::from(Span::styled(
            "  ┌── AKTİF GENOM ─────────────────────────────────────────────",
            Style::default().fg(Color::Blue),
        )),
        Line::from(vec![
            Span::styled("  │  ID        ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{:<10}", evo.genome_id), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("    Döngü   ", Style::default().fg(Color::Blue)),
            Span::styled(
                format!("{}/{}", cycles_done, evo.evolve_every_n_cycles),
                Style::default().fg(Color::White),
            ),
            Span::styled("    Sonraki evrim: ", Style::default().fg(Color::Blue)),
            Span::styled(
                format!("{} cycle", cycles_left),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(vec![
            Span::styled("  │  Fitness   ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{:>6.3}", evo.genome_fitness), Style::default().fg(fit_color).add_modifier(Modifier::BOLD)),
            Span::styled("  ", Style::default()),
            Span::styled(bar(evo.genome_fitness.clamp(0.0, 1.0), 16), Style::default().fg(fit_color)),
            Span::styled("  hedef ≥ 0.50", Style::default().fg(Color::Blue)),
        ]),
        Line::from(vec![
            Span::styled("  │  İşlemler  ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{:>4}", evo.genome_trades), Style::default().fg(Color::White)),
            Span::styled("    Kazanma   ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{:.1}%", evo.genome_win_rate), Style::default().fg(win_color).add_modifier(Modifier::BOLD)),
            Span::styled("    İlerleme  ", Style::default().fg(Color::Blue)),
            Span::styled(format!("[{}]", cycle_bar), Style::default().fg(Color::Blue)),
        ]),
        Line::from(""),

        // ── İki sütun: Brain | Popülasyon ─────────────────────────────────────
        Line::from(vec![
            Span::styled(
                "  ┌── ADAPTİF BEYİN ──────────────────────────",
                Style::default().fg(Color::Blue),
            ),
            Span::styled(
                "    ┌── POPÜLASYON ───────────────────────",
                Style::default().fg(Color::Blue),
            ),
        ]),
        Line::from(vec![
            Span::styled("  │  Rejim     ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{:<18}", regime), Style::default().fg(Color::LightCyan)),
            Span::styled("    │  Nesil      ", Style::default().fg(Color::Blue)),
            Span::styled(generation, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  │  Q-Table   ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{:<6} durum", qtable), Style::default().fg(Color::White)),
            Span::styled("       │  Popülasyon  ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{} birey", pop_size), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  │  Adım      ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{:<18}", steps), Style::default().fg(Color::White)),
            Span::styled("    │  En İyi     ", Style::default().fg(Color::Blue)),
            Span::styled(
                best_fit.clone(),
                Style::default().fg(
                    if best_fit.parse::<f64>().unwrap_or(0.0) >= 0.5 { Color::Green }
                    else { Color::Yellow }
                ),
            ),
        ]),
        Line::from(vec![
            Span::styled("  │  Keşif     ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{:>6}", exploration), Style::default().fg(Color::Yellow)),
            Span::styled(format!("  {} ", bar(expl_f / 100.0, 8)), Style::default().fg(Color::Yellow)),
            Span::styled("   │  Ortalama   ", Style::default().fg(Color::Blue)),
            Span::styled(avg_fit, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  │  Ödül/100  ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{:<18}", avg_reward), Style::default().fg(reward_color)),
            Span::styled("    │  Hall of Fame ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{} birey", hof), Style::default().fg(Color::LightYellow)),
        ]),
        Line::from(""),
        Line::from(Span::styled(divider, Style::default().fg(Color::Blue))),
        Line::from(vec![
            Span::styled("  Cycle: ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{}", evo.cycle_id), Style::default().fg(Color::White)),
            Span::styled("   Her ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{}", evo.evolve_every_n_cycles), Style::default().fg(Color::White)),
            Span::styled(" cycle'da bir evrim tetiklenir", Style::default().fg(Color::Blue)),
        ]),
        Line::from(""),
        // ── ML Köprüsü ────────────────────────────────────────────────────────
        // 'm' tuşu ML worker'ı tetikler → burası Tab 3'teki eğitim durumunu özetler.
        // GBT skoru dolaylı olarak genome fitness'ı etkiler (daha iyi sinyal → daha iyi trade).
        Line::from(vec![
            Span::styled(
                "  ┌── ML KÖPRÜSÜ ",
                Style::default().fg(Color::Blue),
            ),
            Span::styled(
                "[m] ile tetikle",
                Style::default().fg(Color::Blue).add_modifier(Modifier::DIM),
            ),
            Span::styled(
                " ─────────────────────────────────────────",
                Style::default().fg(Color::Blue),
            ),
        ]),
        Line::from(vec![
            Span::styled("  │  Durum   ", Style::default().fg(Color::Blue)),
            if ml_running {
                Span::styled(
                    "⏳ Eğitim devam ediyor...  ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(
                    "● Hazır  ",
                    Style::default().fg(Color::Green),
                )
            },
            Span::styled("  Sonraki çalışma: ", Style::default().fg(Color::Blue)),
            {
                let (label, color) = if ml_running {
                    ("eğitim devam ediyor".to_string(), Color::Yellow)
                } else if ml_next_secs == 0 {
                    ("tetiklenmeyi bekliyor".to_string(), Color::LightYellow)
                } else if ml_next_secs >= 60 {
                    (format!("{} dk {} sn", ml_next_secs / 60, ml_next_secs % 60), Color::White)
                } else {
                    (format!("{} sn", ml_next_secs), Color::LightYellow)
                };
                Span::styled(label, Style::default().fg(color))
            },
        ]),
        Line::from(vec![
            Span::styled("  │  GBT     ", Style::default().fg(Color::Blue)),
            {
                let (label, color) = match gbt_score {
                    Some(s) if s >= 0.1  => (format!("{:+.4}  ↑ Bullish", s), Color::Green),
                    Some(s) if s <= -0.1 => (format!("{:+.4}  ↓ Bearish", s), Color::Red),
                    Some(s)              => (format!("{:+.4}  → Nötr",     s), Color::Yellow),
                    None                 => ("—  (snapshot yok)".to_string(),  Color::Blue),
                };
                Span::styled(format!("{:<26}", label), Style::default().fg(color).add_modifier(Modifier::BOLD))
            },
            Span::styled("  OOS: ", Style::default().fg(Color::Blue)),
            Span::styled(
                if oos_bc > 0 { format!("{:.1}%  / {} bar", oos_wr, oos_bc) }
                else { "—  bekleniyor".to_string() },
                Style::default().fg(
                    if oos_bc == 0    { Color::Blue }
                    else if oos_wr >= 52.0 { Color::Green }
                    else if oos_wr >= 45.0 { Color::Yellow }
                    else               { Color::Red }),
            ),
        ]),
        Line::from(vec![
            Span::styled("  │  Eğitim  ", Style::default().fg(Color::Blue)),
            match &ml_last_train {
                Some(s) => Span::styled(s.as_str(), Style::default().fg(Color::White)),
                None    => Span::styled("—  henüz eğitilmedi  (m = tetikle)", Style::default().fg(Color::Blue)),
            },
        ]),
        Line::from(vec![
            Span::styled("  │  Doğrul. ", Style::default().fg(Color::Blue)),
            match &st.validation_result {
                None => Span::styled("—  ML tamamlanınca otomatik hesaplanır  (Tab 3'te ayrıntı)", Style::default().fg(Color::Blue)),
                Some(vr) => {
                    let (risk_color, risk_str) = match vr.risk_level {
                        RiskLevel::Low      => (Color::Green,  "Düşük"),
                        RiskLevel::Moderate => (Color::Yellow, "Orta"),
                        RiskLevel::High     => (Color::Red,    "Yüksek"),
                        RiskLevel::Critical => (Color::Red,    "KRİTİK"),
                        RiskLevel::Unknown  => (Color::LightBlue,   "—"),
                    };
                    let bar_w = 15usize;
                    let filled = ((vr.composite_score / 100.0) * bar_w as f64).round() as usize;
                    let sbar = format!("{}{}", "█".repeat(filled.min(bar_w)), "░".repeat(bar_w.saturating_sub(filled)));
                    Span::styled(
                        format!("Skor {:.0}/100 [{}] Risk: {}  MC-Ruin: {:.1}%  WF: {:.0}%  [{}]",
                            vr.composite_score, sbar, risk_str,
                            vr.mc_ruin_pct, vr.wf_consistency * 100.0, vr.computed_at),
                        Style::default().fg(risk_color),
                    )
                }
            },
        ]),
    ];

    let block = Paragraph::new(lines)
        .block(
            Block::default()
                .title(" 🧬 Evrimsel AI ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        );
    f.render_widget(block, area);
}

// ── Birleşik AI Görünümü (Tab 2) ─────────────────────────────────────────────
// Satır 1: Evrimsel AI özeti (Genome · Brain · Population · ML bağlantısı)
// Satır 2 (sol/sağ): ML Model Durumu  |  MC + Walk-Forward Doğrulama
// Satır 3: Sinyal Tanılaması (BUY/SELL/HOLD sayaçları + engellenme istatistikleri)
// Satır 4: Sembol Adayları (sol) | Binance Tarayıcı (sağ)
fn draw_ai_combined(f: &mut ratatui::Frame, area: Rect, st: &AppState) {
    // Terminal yüksekliğine göre adaptif layout:
    // Minimum 35 satır varsayılır; küçük terminallerde blok yükseklikleri kısalır.
    let h = area.height as usize;
    // Yükseklik eşiklerine göre üst blokları küçülterek BLOK 4'e (Sembol Adayları)
    // yeterli alan bırakıyoruz. Min(8) → en az 8 satır garantisi.
    let blk1_h: u16 = if h >= 50 { 12 } else if h >= 44 { 10 } else { 8 };
    let blk2_h: u16 = if h >= 46 { 12 } else if h >= 40 { 10 } else { 8 };
    let blk3_h: u16 = if h >= 46 { 11 } else if h >= 40 { 9 }  else { 7 };
    let main = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(blk1_h), // Evrim özeti
            Constraint::Length(blk2_h), // ML Model (sol) | MC+WF (sağ)
            Constraint::Length(blk3_h), // Sinyal tanılaması
            Constraint::Min(8),         // Sembol adayları + Tarayıcı (en az 8 satır)
        ])
        .split(area);

    // ════════════════════════════════════════════════════════════════════════
    // BLOK 1 — Evrimsel AI Özeti
    // ════════════════════════════════════════════════════════════════════════
    {
        let evo = st.live_evolution.read().ok().map(|g| g.clone()).unwrap_or_default();
        let (ml_running, gbt_score, oos_wr, oos_bc, ml_trained,
             scorer_sum, scorer_dis, scorer_n,
             clf_trained, clf_n_win, clf_n_loss, clf_buf,
             cum_pnl, peak_eq) = st.live_risk.read().ok()
            .map(|r| (
                r.ml_running, r.gbt_last_score, r.oos_win_rate, r.oos_bar_count, r.ml_weights.is_some(),
                r.scorer_summary.clone(), r.scorer_disabled.clone(), r.scorer_total_n,
                r.classifier_trained, r.classifier_n_win, r.classifier_n_loss, r.classifier_buffer_len,
                r.cumulative_pnl, r.peak_equity,
            ))
            .unwrap_or((false, None, 0.0, 0, false,
                        String::new(), String::new(), 0,
                        false, 0, 0, 0, 0.0, 0.0));
        let ml_next_secs = st.ml_next_run_at
            .saturating_duration_since(std::time::Instant::now()).as_secs();

        // Metin özetlerini satır bazlı parse et (key: value formatı)
        let extract_kv = |s: &str, key: &str| -> Option<String> {
            for part in s.split(',') {
                let part = part.trim();
                if let Some(rest) = part.strip_prefix(key) {
                    return Some(rest.trim_start_matches(':').trim().to_string());
                }
            }
            None
        };
        let regime      = extract_kv(&evo.brain_summary, "Rejim").unwrap_or_else(|| "—".into());
        let exploration = extract_kv(&evo.brain_summary, "Exploration").unwrap_or_else(|| "—".into());
        let avg_reward  = extract_kv(&evo.brain_summary, "Ortalama reward (son 100)").unwrap_or_else(|| "—".into());
        let generation  = extract_kv(&evo.pop_summary, "Nesil").unwrap_or_else(|| "—".into());
        let pop_size    = extract_kv(&evo.pop_summary, "Popülasyon").unwrap_or_else(|| "—".into());
        let best_fit_s  = extract_kv(&evo.pop_summary, "En İyi Fitness").unwrap_or_else(|| "—".into());
        let hof         = extract_kv(&evo.pop_summary, "Hall of Fame").unwrap_or_else(|| "—".into());

        let bar = |ratio: f64, w: usize| -> String {
            let f = ((ratio.clamp(0.0,1.0)*w as f64).round() as usize).min(w);
            format!("{}{}", "█".repeat(f), "░".repeat(w-f))
        };
        let (cycles_done, cycles_left, cycle_bar) = if evo.evolve_every_n_cycles > 0 {
            let d = evo.cycle_id % evo.evolve_every_n_cycles;
            let l = evo.evolve_every_n_cycles - d;
            (d, l, bar(d as f64 / evo.evolve_every_n_cycles as f64, 20))
        } else { (0, 0, "░".repeat(20)) };
        let expl_f: f64 = exploration.trim_end_matches('%').parse().unwrap_or(0.0);
        let rew_f:  f64 = avg_reward.trim_start_matches('+').parse().unwrap_or(0.0);
        let bf_f:   f64 = best_fit_s.parse().unwrap_or(0.0);

        // ── Renk eşlikleri ──────────────────────────────────────────────────
        let fit_c   = if evo.genome_fitness >= 0.5 { Color::Green } else if evo.genome_fitness >= 0.2 { Color::Yellow } else { Color::Red };
        let win_c   = if evo.genome_win_rate >= 50.0 { Color::Green } else if evo.genome_win_rate >= 35.0 { Color::Yellow } else { Color::Red };
        let rew_c   = if rew_f > 0.1 { Color::Green } else if rew_f > -0.1 { Color::Yellow } else { Color::Red };
        let bf_c    = if bf_f >= 0.5 { Color::Green } else if bf_f >= 0.2 { Color::Yellow } else { Color::LightBlue };
        let evo_st  = if evo.evolution_enabled { ("● AKTİF", Color::LightGreen) } else { ("○ PASİF", Color::Red) };
        let brain_c = if evo.brain_active { Color::Green } else { Color::Blue };
        let pop_c   = if evo.pop_active   { Color::Green } else { Color::Blue };
        let ml_c    = if ml_running { Color::Yellow } else if ml_trained { Color::Green } else { Color::LightBlue };
        let ml_lbl  = if ml_running { "⏳ Eğitim..." } else if ml_trained { "✓ Trained" } else { "○ Default" };
        let next_ml_lbl = if ml_running { "çalışıyor".into() }
                          else if ml_next_secs == 0 { "bekliyor".into() }
                          else if ml_next_secs >= 60 { format!("{}d{}s", ml_next_secs/60, ml_next_secs%60) }
                          else { format!("{}s", ml_next_secs) };
        let gbt_span = match gbt_score {
            Some(s) if s >= 0.1  => (format!("{:+.3} ↑ BUY",  s), Color::Green),
            Some(s) if s <= -0.1 => (format!("{:+.3} ↓ SELL", s), Color::Red),
            Some(s)              => (format!("{:+.3} → NÖTR", s), Color::Yellow),
            None                 => ("— (henüz yok)".into(),       Color::Blue),
        };
        let oos_lbl = if oos_bc > 0 { format!("{:.1}% / {}bar", oos_wr, oos_bc) } else { "— (hesaplanmadı)".into() };
        let oos_c   = if oos_bc == 0 { Color::Blue } else if oos_wr >= 52.0 { Color::Green } else if oos_wr >= 45.0 { Color::Yellow } else { Color::Red };

        let sep = Span::styled("  │  ", Style::default().fg(Color::Blue));
        let lbl = |s: &str| Span::styled(s.to_string(), Style::default().fg(Color::Blue));
        let val = |s: String, c: Color| Span::styled(s, Style::default().fg(c));
        let bold_val = |s: String, c: Color| Span::styled(s, Style::default().fg(c).add_modifier(Modifier::BOLD));

        let evo_lines: Vec<Line> = vec![
            // ── Satır 1: Evrim/Brain/Pop durumu + Genome ────────────────────
            Line::from(vec![
                bold_val(format!("  {} EVRİM", evo_st.0), evo_st.1),
                sep.clone(),
                lbl("Brain:"), val(if evo.brain_active { "●".into() } else { "○".into() }, brain_c),
                lbl("  Pop:"),  val(if evo.pop_active   { "●".into() } else { "○".into() }, pop_c),
                sep.clone(),
                lbl("Genome: "), bold_val(evo.genome_id.clone(), Color::Cyan),
                lbl("  Fitness: "),
                val(format!("{:.3} [{}]", evo.genome_fitness, bar(evo.genome_fitness.clamp(0.0,1.0), 10)), fit_c),
                lbl("  WR: "), bold_val(format!("{:.1}%", evo.genome_win_rate), win_c),
                lbl("  Trades: "), val(format!("{}", evo.genome_trades), Color::White),
            ]),
            // ── Satır 2: Döngü / Nesil / Popülasyon / HoF ──────────────────
            Line::from(vec![
                lbl("  Döngü: "),
                val(format!("{}/{}", cycles_done, evo.evolve_every_n_cycles), Color::White),
                val(format!(" [{}]", cycle_bar), Color::Blue),
                val(format!(" +{} evrime kalan", cycles_left), Color::Yellow),
                sep.clone(),
                lbl("Nesil: "), val(format!("{:<4}", generation), Color::White),
                lbl("  Pop: "),  val(format!("{:<4}", pop_size), Color::White),
                lbl("  En İyi: "), val(format!("{}", best_fit_s), bf_c),
                lbl("  HoF: "),  val(format!("{}", hof), Color::LightYellow),
            ]),
            // ── Satır 3: Brain özeti (Rejim / Keşif / Ödül) ────────────────
            Line::from(vec![
                lbl("  Brain  "),
                lbl("Rejim: "), val(format!("{:<18}", regime), Color::LightCyan),
                lbl("  Keşif: "),
                val(format!("{:>5} [{}]", exploration, bar(expl_f/100.0, 8)), Color::Yellow),
                lbl("  Ödül(100): "), val(format!("{}", avg_reward), rew_c),
            ]),
            // ── Ayırıcı ─────────────────────────────────────────────────────
            Line::from(Span::styled(
                "  ─────────────────────────────────────────────────────────────────────────────",
                Style::default().fg(Color::Blue),
            )),
            // ── Satır 5: ML bağlantısı ──────────────────────────────────────
            Line::from(vec![
                lbl("  ML     "),
                val(format!("{:<14}", ml_lbl), ml_c),
                lbl("  Sonraki: "), val(format!("{:<10}", next_ml_lbl), Color::White),
                sep.clone(),
                lbl("GBT: "), bold_val(format!("{:<22}", gbt_span.0), gbt_span.1),
                sep.clone(),
                lbl("OOS: "), val(oos_lbl, oos_c),
            ]),
            // ── Satır 6: UCB1 StrategyScorer ────────────────────────────────
            Line::from(vec![
                lbl("  UCB1   "),
                {
                    let c = if scorer_n == 0 { Color::Blue }
                            else if scorer_dis.contains("❌") { Color::Yellow }
                            else { Color::LightGreen };
                    if scorer_n == 0 {
                        val("— (henüz işlem yok)".into(), Color::Blue)
                    } else {
                        val(format!("n={:>3}  {}   {}",
                            scorer_n, scorer_dis,
                            if scorer_sum.len() > 50 { format!("{}…", &scorer_sum[..50]) }
                            else if scorer_sum.is_empty() { "veri birikme sürecinde".into() }
                            else { scorer_sum.clone() }
                        ), c)
                    }
                },
            ]),
            // ── Satır 7: GNB Classifier ─────────────────────────────────────
            Line::from(vec![
                lbl("  GNB    "),
                {
                    let total_clf = clf_n_win + clf_n_loss;
                    let clf_wr = if total_clf > 0 { clf_n_win as f64 / total_clf as f64 * 100.0 } else { 0.0 };
                    if clf_trained {
                        let c = if clf_wr >= 50.0 { Color::Green } else if clf_wr >= 38.0 { Color::Yellow } else { Color::Red };
                        val(format!("✓ Aktif   kazanç={} kayıp={} WR={:.0}%  (buf={})",
                            clf_n_win, clf_n_loss, clf_wr, clf_buf), c)
                    } else {
                        let c = if clf_buf >= 15 { Color::Yellow } else { Color::Blue };
                        val(format!("○ Cold-start  buf={}/20  (henüz filtre bypass modunda)",
                            clf_buf), c)
                    }
                },
                sep.clone(),
                {
                    // Equity carry-over özeti — cumulative PnL + gerçek DD (peak'ten)
                    let cur_eq  = peak_eq + cum_pnl;
                    let real_dd = if peak_eq > 0.0 && cur_eq < peak_eq {
                        (peak_eq - cur_eq) / peak_eq * 100.0
                    } else { 0.0 };
                    let pnl_c = if cum_pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };
                    let dd_c  = if real_dd > 15.0 { Color::Red } else if real_dd > 7.0 { Color::Yellow } else { Color::Green };
                    Span::styled(
                        format!("cumPnL={:+.2}$  peak={:.0}$  DD={:.1}%",
                            cum_pnl, peak_eq, real_dd),
                        Style::default().fg(if real_dd > 7.0 { dd_c } else { pnl_c }),
                    )
                },
            ]),
            // ── Satır 8: MC+WF doğrulama özeti ──────────────────────────────
            Line::from(vec![
                lbl("  MC+WF  "),
                match &st.validation_result {
                    None => val("— (hesaplanmadı — [m] ile ML tetikle)".into(), Color::Blue),
                    Some(vr) => {
                        let (rc, rs) = match vr.risk_level {
                            RiskLevel::Low      => (Color::Green,  "● Düşük"),
                            RiskLevel::Moderate => (Color::Yellow, "◑ Orta"),
                            RiskLevel::High     => (Color::Red,    "▲ Yüksek"),
                            RiskLevel::Critical => (Color::Red,    "✖ KRİTİK"),
                            RiskLevel::Unknown  => (Color::LightBlue,   "? —"),
                        };
                        let bw = 12usize;
                        let bf = ((vr.composite_score/100.0)*bw as f64).round() as usize;
                        let sb = format!("{}{}", "█".repeat(bf.min(bw)), "░".repeat(bw.saturating_sub(bf)));
                        val(format!("Skor {:.0}/100 [{}] {}  Ruin:{:.1}%  WF:{:.0}%  [{} @ {}]",
                            vr.composite_score, sb, rs,
                            vr.mc_ruin_pct, vr.wf_consistency*100.0,
                            vr.strategy_name, vr.computed_at), rc)
                    }
                },
            ]),
        ];
        let evo_title = format!(" 🧬 Evrimsel AI — Genome · Brain · Population  [Döngü {}] ",
            evo.cycle_id);
        let border_col = if evo.evolution_enabled && (evo.brain_active || evo.pop_active) {
            Color::Green
        } else if evo.evolution_enabled {
            Color::Yellow
        } else {
            Color::Blue
        };
        f.render_widget(
            Paragraph::new(evo_lines).block(
                Block::default()
                    .title(evo_title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_col)),
            ),
            main[0],
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // BLOK 2 — ML Model (sol) | Strateji Doğrulama (sağ) — yan yana
    // ════════════════════════════════════════════════════════════════════════
    {
        let two_col = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main[1]);

        // ── Sol: ML Model Durumu ─────────────────────────────────────────────
        let (lr_trained, gbt_score, drift, oos_wr, oos_ar, oos_bc, oos_folds, ens, ml_run) =
            st.live_risk.read().ok().map(|r| (
                r.ml_weights.is_some(), r.gbt_last_score, r.ml_drift_score,
                r.oos_win_rate, r.oos_avg_return, r.oos_bar_count, r.oos_fold_scores,
                r.ensemble_agreement, r.ml_running,
            )).unwrap_or((false, None, 0.0, 0.0, 0.0, 0, [0.0;3], 0.0, false));

        // ── Durum etiketleri ────────────────────────────────────────────────
        let sig_c   = match st.ml_signal.as_str() { "BUY" => Color::Green, "SELL" => Color::Red, _ => Color::LightBlue };
        let conf_c  = if st.ml_confidence >= 0.6 { Color::Green } else if st.ml_confidence >= 0.35 { Color::Yellow } else { Color::LightBlue };
        let lr_c    = if ml_run { Color::Cyan } else if lr_trained { Color::Green } else { Color::Yellow };
        let lr_lbl  = if ml_run { "⏳ Eğitim..." } else if lr_trained { "✓ Trained" } else { "○ Default" };
        let gbt_lbl = match gbt_score {
            Some(s) if s >  0.1 => format!("▲ BUY  {:.4}", s),
            Some(s) if s < -0.1 => format!("▼ SELL {:.4}", s),
            Some(s)             => format!("→ NÖTR {:.4}", s),
            None                => "— (henüz hesaplanmadı)".into(),
        };
        let gbt_c = match gbt_score {
            Some(s) if s > 0.1 => Color::Green, Some(s) if s < -0.1 => Color::Red, _ => Color::LightBlue
        };
        let oos_wr_c = if oos_bc == 0 { Color::Blue } else if oos_wr >= 55.0 { Color::Green }
                       else if oos_wr >= 45.0 { Color::Yellow } else { Color::Red };
        let drift_lbl = if drift > 0.35 { "▲ Kayma" } else if drift > 0.15 { "~ Hafif" } else { "✓ Stabil" };
        let drift_c   = if drift > 0.35 { Color::Red } else if drift > 0.15 { Color::Yellow } else { Color::Green };
        let ens_lbl   = if ens > 0.85 { "⚠ Düşük çeş." } else if ens > 0.65 { "✓ Normal" } else { "★ Yüksek çeş." };
        let ens_c     = if ens > 0.85 { Color::Yellow } else if ens > 0.0 { Color::Green } else { Color::Blue };

        let lm = |s: &str| Span::styled(s.to_string(), Style::default().fg(Color::Blue));
        let vm = |s: String, c: Color| Span::styled(s, Style::default().fg(c));
        let bm = |s: String, c: Color| Span::styled(s, Style::default().fg(c).add_modifier(Modifier::BOLD));

        let ml_lines: Vec<Line> = vec![
            // Satır 1: Model durumu + sinyal
            Line::from(vec![
                lm("  Durum  "),
                Span::styled(format!("{:<14}", lr_lbl), Style::default().fg(lr_c)),
                lm("  #"),
                vm(format!("{}", st.ml_train_count), Color::White),
                lm("  eğitim  "),
                lm("│  Sinyal: "),
                bm(format!("▶ {:<4}", st.ml_signal), sig_c),
                lm("  Güven: "),
                vm(format!("{:.3}", st.ml_confidence), conf_c),
                lm("  Skor: "),
                vm(format!("{:+.4}", st.ml_score),
                    if st.ml_score > 0.0 { Color::Green } else if st.ml_score < 0.0 { Color::Red } else { Color::LightBlue }),
            ]),
            // Satır 2: GBT tahmini
            Line::from(vec![
                lm("  GBT     "),
                bm(format!("{:<32}", gbt_lbl), gbt_c),
                lm("  │  Ensemble: "),
                vm(if ens > 0.0 { format!("{:.0}% uyum  {}", ens*100.0, ens_lbl) } else { "— ".into() }, ens_c),
            ]),
            // Satır 3: OOS kalitesi
            Line::from(vec![
                lm("  OOS WR  "),
                vm(if oos_bc > 0 { format!("{:.1}%", oos_wr) } else { "—".into() }, oos_wr_c),
                lm("  Ret: "),
                vm(if oos_bc > 0 { format!("{:+.3}%", oos_ar) } else { "—".into() },
                    if oos_ar >= 0.0 { Color::Green } else { Color::Red }),
                lm("  Barlar: "),
                vm(if oos_bc > 0 { format!("{}", oos_bc) } else { "—".into() }, Color::White),
                lm("  Folds: "),
                vm(if oos_bc > 0 {
                    format!("[{:.0}%/{:.0}%/{:.0}%]", oos_folds[0], oos_folds[1], oos_folds[2])
                } else { "[—/—/—]".into() }, Color::Cyan),
            ]),
            // Satır 4: Feature drift
            Line::from(vec![
                lm("  Drift    "),
                vm(format!("{:.3}  {}", drift, drift_lbl), drift_c),
                lm("  │  LR+GBT Ensemble  19 feature  "),
            ]),
        ];
        let ml_border = if ml_run { Color::Cyan } else if lr_trained { Color::LightGreen } else { Color::Yellow };
        f.render_widget(
            Paragraph::new(ml_lines).block(
                Block::default()
                    .title(" 🧠 ML Model — LR + GBT Ensemble ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ml_border)),
            ),
            two_col[0],
        );

        // ── Sağ: Strateji Doğrulama ──────────────────────────────────────────
        // Her durumda başlık satırları gösterilir; veri yoksa "—" ile doldurulur.
        let lv = |s: &str| Span::styled(s.to_string(), Style::default().fg(Color::Blue));
        let vv = |s: String, c: Color| Span::styled(s, Style::default().fg(c));
        let bv = |s: String, c: Color| Span::styled(s, Style::default().fg(c).add_modifier(Modifier::BOLD));

        let (vr_opt, border_col_val) = match &st.validation_result {
            None => (None, Color::Blue),
            Some(vr) => {
                let bc = match vr.risk_level {
                    RiskLevel::Low      => Color::Green,
                    RiskLevel::Moderate => Color::Yellow,
                    RiskLevel::High | RiskLevel::Critical => Color::Red,
                    RiskLevel::Unknown  => Color::Blue,
                };
                (Some(vr), bc)
            }
        };

        // Yardımcı: Option<&ValidationResult> + alan varlığı → değer veya "—"
        let mc_ok = vr_opt.map(|v| v.mc_n_sims > 0).unwrap_or(false);
        let wf_ok = vr_opt.map(|v| v.wf_windows > 0).unwrap_or(false);

        let skor_line = {
            match vr_opt {
                None => Line::from(vec![
                    lv("  Sağlık  "), vv("—  (henüz hesaplanmadı)".into(), Color::Blue),
                    vv("  [m] ile tetikle".into(), Color::Blue),
                ]),
                Some(vr) => {
                    let (rc, rl) = match vr.risk_level {
                        RiskLevel::Low      => (Color::Green,  "● Düşük"),
                        RiskLevel::Moderate => (Color::Yellow, "◑ Orta"),
                        RiskLevel::High     => (Color::Red,    "▲ Yüksek"),
                        RiskLevel::Critical => (Color::Red,    "✖ KRİTİK"),
                        RiskLevel::Unknown  => (Color::LightBlue,   "? —"),
                    };
                    let bw = 14usize;
                    let bf = ((vr.composite_score/100.0)*bw as f64).round() as usize;
                    let sb = format!("{}{}", "█".repeat(bf.min(bw)), "░".repeat(bw.saturating_sub(bf)));
                    let sc2 = if vr.composite_score >= 65.0 { Color::Green }
                              else if vr.composite_score >= 40.0 { Color::Yellow } else { Color::Red };
                    Line::from(vec![
                        lv("  Sağlık  "),
                        vv(format!("{:.0}/100 [{}] ", vr.composite_score, sb), sc2),
                        bv(format!("{:<10}", rl), rc),
                        vv(format!("@ {}", vr.computed_at), Color::Blue),
                    ])
                }
            }
        };

        let mc_ruin_line = {
            if !mc_ok {
                let why = if vr_opt.is_none() { "—" } else { "— (< 5 trade)" };
                Line::from(vec![ lv("  MC Ruin "), vv(why.into(), Color::Blue) ])
            } else {
                let vr = vr_opt.unwrap();
                let rc = if vr.mc_ruin_pct < 5.0 { Color::Green }
                         else if vr.mc_ruin_pct < 15.0 { Color::Yellow } else { Color::Red };
                Line::from(vec![
                    lv("  MC Ruin "),
                    bv(format!("{:.1}%  ", vr.mc_ruin_pct), rc),
                    vv(format!("P5={:.0}  P50={:.0}  P95={:.0}",
                        vr.mc_p5_balance, vr.mc_p50_balance, vr.mc_p95_balance), Color::LightBlue),
                ])
            }
        };

        let mc_karl_line = {
            if !mc_ok {
                let why = if vr_opt.is_none() { "—" } else { "— (< 5 trade)" };
                Line::from(vec![ lv("  Kârlı   "), vv(why.into(), Color::Blue) ])
            } else {
                let vr = vr_opt.unwrap();
                let mcp = ((vr.mc_positive_pct/100.0)*10.0).round() as usize;
                let mcbar = format!("{}{}", "█".repeat(mcp.min(10)), "░".repeat(10usize.saturating_sub(mcp)));
                let kc = if vr.mc_positive_pct >= 60.0 { Color::Green }
                         else if vr.mc_positive_pct >= 40.0 { Color::Yellow } else { Color::Red };
                Line::from(vec![
                    lv("  Kârlı   "),
                    bv(format!("{:.0}% [{}] ", vr.mc_positive_pct, mcbar), kc),
                    vv(format!("Bkl:{:+.2}%  DD:{:.1}/{:.1}%",
                        vr.mc_expected_ret, vr.mc_max_dd_p50, vr.mc_max_dd_p95), Color::LightBlue),
                ])
            }
        };

        let wf_tut_line = {
            if !wf_ok {
                let why = if vr_opt.is_none() { "—" } else { "— (< 40 mum)" };
                Line::from(vec![ lv("  WF Tut  "), vv(why.into(), Color::Blue) ])
            } else {
                let vr = vr_opt.unwrap();
                let wfp = (vr.wf_consistency*10.0).round() as usize;
                let wfbar = format!("{}{}", "█".repeat(wfp.min(10)), "░".repeat(10usize.saturating_sub(wfp)));
                let wc = if vr.wf_consistency >= 0.6 { Color::Green }
                         else if vr.wf_consistency >= 0.4 { Color::Yellow } else { Color::Red };
                Line::from(vec![
                    lv("  WF Tut  "),
                    bv(format!("{:.0}% [{}] ", vr.wf_consistency*100.0, wfbar), wc),
                    vv(format!("{}/{} pencere", vr.wf_profitable, vr.wf_windows), Color::LightBlue),
                ])
            }
        };

        let wf_oos_line = {
            if !wf_ok {
                let why = if vr_opt.is_none() { "—" } else { "— (< 40 mum)" };
                Line::from(vec![ lv("  WF OOS  "), vv(why.into(), Color::Blue) ])
            } else {
                let vr = vr_opt.unwrap();
                let wrc = if vr.wf_avg_oos_wr >= 55.0 { Color::Green }
                          else if vr.wf_avg_oos_wr >= 45.0 { Color::Yellow } else { Color::LightBlue };
                Line::from(vec![
                    lv("  WF OOS  "),
                    vv(format!("WR={:.0}%  PnL={:+.1}%  PF={:.2}  DD={:.1}%  S={:.2}",
                        vr.wf_avg_oos_wr, vr.wf_avg_oos_pnl, vr.wf_avg_oos_pf,
                        vr.wf_avg_oos_dd, vr.wf_avg_oos_sharpe), wrc),
                ])
            }
        };

        let strat_line = {
            match vr_opt {
                None => Line::from(vec![
                    lv("  Strateji "), vv("—".into(), Color::Blue),
                ]),
                Some(vr) => Line::from(vec![
                    lv("  Strateji "),
                    vv(format!("{}", vr.strategy_name), Color::Cyan),
                    vv(format!("  {} sim  {} trade", vr.mc_n_sims, vr.mc_n_trades), Color::Blue),
                ]),
            }
        };

        let val_title = match vr_opt {
            None    => " 🎲 MC + Walk-Forward  [m = hesapla] ".to_string(),
            Some(vr) => format!(" 🎲 MC + Walk-Forward  [{} sim · {} pnc] ",
                vr.mc_n_sims, vr.wf_windows),
        };

        let val_lines = vec![skor_line, mc_ruin_line, mc_karl_line, wf_tut_line, wf_oos_line, strat_line];
        f.render_widget(
            Paragraph::new(val_lines).block(
                Block::default()
                    .title(val_title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_col_val)),
            ),
            two_col[1],
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // BLOK 3 — Sinyal Tanılaması + Son Eğitim/HyperOpt özeti
    // ════════════════════════════════════════════════════════════════════════
    {
        // Sinyal sayaçlarını oku
        let sc = st.live_signal_counts.read().ok().map(|s| s.clone())
            .unwrap_or_default();
        let total_sig = sc.buy + sc.sell + sc.hold;
        let total_blk = sc.blocked_rr + sc.blocked_volatility + sc.blocked_trend
                      + sc.blocked_risk_gate + sc.ml_below_threshold;
        let buy_pct  = if total_sig > 0 { sc.buy  as f64 / total_sig as f64 * 100.0 } else { 0.0 };
        let sell_pct = if total_sig > 0 { sc.sell as f64 / total_sig as f64 * 100.0 } else { 0.0 };
        let hold_pct = if total_sig > 0 { sc.hold as f64 / total_sig as f64 * 100.0 } else { 0.0 };

        // Son eğitim + strateji
        let remaining_secs = st.ml_next_run_at.saturating_duration_since(std::time::Instant::now()).as_secs();
        let train_status = match &st.last_ml_train {
            Some(s) => format!("{}", s),
            None    => if remaining_secs > 0 {
                format!("Henüz çalışmadı — {}s sonra başlar ([m] ile hemen)", remaining_secs)
            } else { "ML çalışıyor...".into() },
        };
        let best_strat_lbl = match &st.best_strategy_name {
            Some(n) => {
                let p = match n.as_str() {
                    "RSI"             => format!("RSI(p={} OB={:.0} OS={:.0})", st.best_rsi_period, st.best_rsi_ob, st.best_rsi_os),
                    "BOLLINGER"       => format!("BB(p={} σ={:.1})", st.best_bb_period, st.best_bb_std_dev),
                    "MACD"            => format!("MACD({}/{}/{})", st.best_macd_fast, st.best_macd_slow, st.best_macd_signal),
                    "MA_CROSSOVER"|"MA" => format!("MA(f={} s={})", st.best_fast, st.best_slow),
                    _                 => n.clone(),
                };
                format!("{} score={:.4} SL={:.1}% TP={:.1}%", p, st.hyperopt_score, st.best_sl, st.best_tp)
            }
            None => format!("— score={:.4} SL={:.1}% TP={:.1}%", st.hyperopt_score, st.best_sl, st.best_tp),
        };
        let last_block = if sc.last_block_reason.is_empty() { "—".to_string() } else { sc.last_block_reason.clone() };

        let sep2 = Span::styled("  │  ", Style::default().fg(Color::Blue));
        let lbl2 = |s: &str| Span::styled(s.to_string(), Style::default().fg(Color::Blue));
        let val2 = |s: String, c: Color| Span::styled(s, Style::default().fg(c));

        let diag_lines = vec![
            // Satır 1: Sinyal dağılımı
            Line::from(vec![
                lbl2("  Sinyaller "),
                Span::styled(format!("BUY {}", sc.buy),  Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                val2(format!("({:.0}%)", buy_pct), Color::Green),
                lbl2("  SELL "),
                Span::styled(format!("{}", sc.sell), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                val2(format!("({:.0}%)", sell_pct), Color::Red),
                lbl2("  HOLD "),
                val2(format!("{}({:.0}%)", sc.hold, hold_pct), Color::Yellow),
                lbl2("  Toplam: "),
                val2(format!("{}", total_sig), Color::White),
                sep2.clone(),
                lbl2("Bloklar: "),
                val2(format!("{}", total_blk),
                    if total_blk > total_sig / 2 { Color::Red } else { Color::Yellow }),
            ]),
            // Satır 2: Engellenme detayı
            Line::from(vec![
                lbl2("  Engellenme"),
                lbl2(" R/R:"),   val2(format!("{}", sc.blocked_rr),         if sc.blocked_rr > 5   { Color::Red } else { Color::LightBlue }),
                lbl2("  Vol:"),  val2(format!("{}", sc.blocked_volatility),  if sc.blocked_volatility > 5 { Color::Red } else { Color::LightBlue }),
                lbl2("  Trend:"),val2(format!("{}", sc.blocked_trend),       if sc.blocked_trend > 5 { Color::Yellow } else { Color::LightBlue }),
                lbl2("  Risk:"), val2(format!("{}", sc.blocked_risk_gate),   if sc.blocked_risk_gate > 5 { Color::Red } else { Color::LightBlue }),
                lbl2("  ML-cnf:"),val2(format!("{}", sc.ml_below_threshold), if sc.ml_below_threshold > 5 { Color::Yellow } else { Color::LightBlue }),
                sep2.clone(),
                lbl2("Son neden: "), val2(format!("{}", last_block), Color::LightYellow),
            ]),
            // Satır 3: Son eğitim + strateji
            Line::from(vec![
                lbl2("  Eğitim   "),
                val2(train_status, if st.last_ml_train.is_some() { Color::LightGreen } else { Color::LightBlue }),
            ]),
            // ── p5_crypto bölümü (satır 4-8) ──────────────────────────────
            {
                // Başlık çizgisi: ayraç
                Line::from(vec![
                    Span::styled("  ─── 🐍 p5_crypto Analizi ", Style::default().fg(Color::Blue)),
                    {
                        match &st.p5_last_status {
                            None => Span::styled(
                                "○ çalışmadı  [y]=başlat  [m] sonrası otonom ─────".to_string(),
                                Style::default().fg(Color::Blue)),
                            Some(p5) => {
                                let (icon, col) = match p5.state.as_str() {
                                    "done"    => ("✓", Color::LightGreen),
                                    "running" => ("⏳", Color::Yellow),
                                    "scanning"=> ("🔍", Color::Cyan),
                                    "error"   => ("✗", Color::Red),
                                    _         => ("?", Color::LightBlue),
                                };
                                Span::styled(
                                    format!("{} {} {}  edge={}/{}  WF={:.0}%  MC={:.0}%  Ruin={:.0}% ─",
                                        icon, p5.symbol, p5.interval,
                                        p5.edge_confirmed, p5.strategies_found,
                                        p5.wf_consistency * 100.0,
                                        p5.mc_prob_profit * 100.0,
                                        p5.ruin_pct * 100.0),
                                    Style::default().fg(col))
                            }
                        }
                    },
                ])
            },
            // Satır 5-7: top 3 strateji
            {
                let no_strats = st.p5_last_status.as_ref()
                    .map(|p| p.top_strategies.is_empty())
                    .unwrap_or(true);
                if no_strats {
                    let scanning_info = st.p5_last_status.as_ref().map(|p5| {
                        if p5.state == "scanning" {
                            format!("  Tarıyor... {} kombinasyon test edildi, {} aday",
                                p5.tested, p5.found_so_far)
                        } else if p5.state == "running" {
                            "  Başlatıldı, tarama başlamadı...".to_string()
                        } else if p5.state == "error" {
                            format!("  Hata: {}", p5.msg)
                        } else {
                            "  Strateji bulunamadı veya veri yetersiz.".to_string()
                        }
                    }).unwrap_or_else(|| "  [y] tuşuyla analizi başlatın veya ML döngüsü bekleyin.".to_string());
                    Line::from(val2(scanning_info, Color::LightBlue))
                } else {
                    let top = st.p5_last_status.as_ref()
                        .map(|p5| p5.top_strategies.clone())
                        .unwrap_or_default();
                    let parts: Vec<Span> = top.iter().enumerate().map(|(i, s)| {
                        let short_name = s.name.splitn(3, '_').skip(1)
                            .collect::<Vec<_>>().join("+");
                        let short_name = short_name.chars().take(28).collect::<String>();
                        Span::styled(
                            format!("  #{} [{}] {:<28} WR={:.0}% PF={:.1} WF={}/3 {}",
                                i+1,
                                if s.direction == "long" { "L" } else { "S" },
                                short_name,
                                s.wr * 100.0, s.pf, s.wf_pass,
                                if s.edge == "GUCLU_EDGE" { "★" } else { " " }),
                            Style::default().fg(if i == 0 {
                                if s.direction == "long" { Color::LightGreen } else { Color::LightRed }
                            } else { Color::White })
                        )
                    }).collect();
                    if parts.is_empty() {
                        Line::from(val2("  —".to_string(), Color::LightBlue))
                    } else {
                        Line::from(parts)
                    }
                }
            },
            // Satır 8: aktif sinyal + otonom öneri
            {
                match &st.p5_last_status {
                    Some(p5) if !p5.active_dir.is_empty() => {
                        let dir_c = if p5.active_dir == "long" { Color::LightGreen } else { Color::LightRed };
                        let dir_sym = if p5.active_dir == "long" { "▲ LONG" } else { "▼ SHORT" };
                        Line::from(vec![
                            Span::styled("  ⚡ Aktif Sinyal: ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                            Span::styled(format!("{}", dir_sym), Style::default().fg(dir_c).add_modifier(Modifier::BOLD)),
                            Span::styled(
                                format!("  TP={:.1}x SL={:.1}x ATR  ATR={:.4}  p={:.4}  Edge={}",
                                    p5.best_tp_mult, p5.best_sl_mult, p5.active_atr,
                                    p5.best_p_value, p5.best_edge),
                                Style::default().fg(Color::Yellow)),
                        ])
                    },
                    Some(p5) if p5.state == "done" => Line::from(vec![
                        lbl2("  Aktif sinyal "),
                        val2("yok".to_string(), Color::LightBlue),
                        lbl2("  — Son analiz: "),
                        val2(p5.ts.chars().take(19).collect::<String>(), Color::Blue),
                        lbl2("  [y]=yenile"),
                    ]),
                    _ => Line::from(val2(String::new(), Color::LightBlue)),
                }
            },
        ];
        f.render_widget(
            Paragraph::new(diag_lines)
                .block(Block::default()
                    .title(format!(" 🚦 Sinyal Tanılaması  •  En İyi: {} ", best_strat_lbl))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(
                        if total_blk > 0 && total_sig > 0 && total_blk * 2 > total_sig {
                            Color::Yellow
                        } else {
                            Color::Cyan
                        }
                    )))
                .wrap(ratatui::widgets::Wrap { trim: false }),
            main[2],
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // BLOK 4 — Sembol Adayları (sol) | Binance Tarayıcı (sağ) yan yana
    // ════════════════════════════════════════════════════════════════════════
    {
        let b4 = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(main[3]);

        // ── Sol: Sembol Adayları Tablosu ─────────────────────────────────────
        {
            let header = format!(
                "  {:<10} {:<8} {:<6}  {:>10}  {:>7}  {:>5}  {:>5}  {:>4}  {:>8}  {}",
                "Sembol","Market","Int","Fiyat","WinRate","PF","DD%","Trd","Skor","Strateji/Durum"
            );
            let mut cand_lines: Vec<ListItem> = vec![
                ListItem::new(Line::from(Span::styled(header, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))),
            ];
            let active_key = (st.active_symbol.exchange.clone(), st.active_symbol.market.clone(),
                              st.active_symbol.symbol.clone(), st.active_symbol.interval.clone());
            for (i, c) in st.symbol_candidates.iter().take(8).enumerate() {
                let is_active = (c.exchange.clone(), c.market.clone(), c.symbol.clone(), c.interval.clone()) == active_key;
                let status = if is_active { "▶ AKTİF" } else if i == 0 && st.auto_symbol { "▶ #1" } else { "" };
                let price_str = if c.last_price > 0.0 {
                    if c.last_price >= 1000.0 { format!("{:>10.1}", c.last_price) }
                    else if c.last_price >= 1.0 { format!("{:>10.4}", c.last_price) }
                    else { format!("{:>10.6}", c.last_price) }
                } else { format!("{:>10}", "-") };
                let strat = if !c.best_strategy.is_empty() { &c.best_strategy } else { "—" };
                let dur = if !status.is_empty() { format!("{} [{}]", status, strat) } else { strat.to_string() };
                let row = format!(
                    "  {:<10} {:<8} {:<6}  {}  {:>7.1}%  {:>5.2}  {:>5.1}  {:>4}  {:>8.4}  {}",
                    &c.symbol, &c.market, &c.interval, price_str, c.win_rate,
                    c.profit_factor, c.max_drawdown_pct, c.total_trades, c.score, dur
                );
                let color = if is_active { Color::LightGreen }
                    else if c.win_rate >= 60.0 { Color::Green }
                    else if c.win_rate >= 40.0 { Color::White }
                    else { Color::LightBlue };
                cand_lines.push(ListItem::new(Line::from(Span::styled(row, Style::default().fg(color)))));
            }
            if st.symbol_candidates.is_empty() {
                cand_lines.push(ListItem::new(Line::from(Span::styled(
                    "  Tarama bekleniyor ([s] anlık tara)...",
                    Style::default().fg(Color::White),
                ))));
            }
            let mode_lbl = if st.auto_symbol {
                " 🎯 Sembol Adayları — AUTO MOD "
            } else {
                " 📌 Sembol Adayları — MANUEL MOD "
            };
            f.render_widget(
                List::new(cand_lines).block(
                    Block::default().title(mode_lbl).borders(Borders::ALL)
                        .border_style(Style::default().fg(if st.auto_symbol { Color::LightGreen } else { Color::Yellow })),
                ),
                b4[0],
            );
        }

        // ── Sağ: Binance Sembol Tarayıcı ─────────────────────────────────────
        {
            let last_run = st.screener_last_run.as_deref().unwrap_or("Henüz çalışmadı");
            let sc = &st.screener_candidates;
            let next_secs_hint = if st.screener_enabled {
                format!("her {:.0}s'de", st.screener_interval_hours * 3600.0)
            } else {
                "DEVRE DIŞI".to_string()
            };

            // Başlık: son tarama zamanı + filtre parametreleri
            let title = format!(
                " 🔍 Binance Tarayıcı [T]  {}  Son: {} ",
                next_secs_hint, last_run
            );

            let mut scr_lines: Vec<ListItem> = vec![
                ListItem::new(Line::from(vec![
                    Span::styled("  Sembol     ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::styled("    Hacim(M)  ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::styled(" Chg%  ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::styled("Durum", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                ])),
            ];

            if sc.is_empty() {
                scr_lines.push(ListItem::new(Line::from(Span::styled(
                    "  (Henüz tarama yapılmadı — [T] ile başlat)",
                    Style::default().fg(Color::Blue),
                ))));
                scr_lines.push(ListItem::new(Line::from(Span::styled(
                    format!("  Filtre: ≥{:.0}M USDT hacim, ≥{:.1}% değişim",
                        st.screener_min_volume_m, st.screener_min_change_pct),
                    Style::default().fg(Color::Blue),
                ))));
            } else {
                for c in sc.iter().take(10) {
                    let chg_col = if c.price_change_pct >= 0.0 { Color::LightGreen } else { Color::LightRed };
                    let vol_m = c.quote_volume_24h / 1_000_000.0;
                    let short = if c.symbol.len() > 9 { &c.symbol[..9] } else { c.symbol.as_str() };
                    scr_lines.push(ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("  {:<9}  ", short),
                            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("{:>7.1}M  ", vol_m),
                            Style::default().fg(Color::Cyan),
                        ),
                        Span::styled(
                            format!("{:>+6.1}%  ", c.price_change_pct),
                            Style::default().fg(chg_col),
                        ),
                        Span::styled(
                            c.status.label(),
                            Style::default().fg(c.status.color()),
                        ),
                    ])));
                }
                // Alt bilgi: toplam / filtre özeti
                let scored_n  = sc.iter().filter(|c| c.status == ScreenerStatus::Scored).count();
                let queued_n  = sc.iter().filter(|c| matches!(c.status, ScreenerStatus::Queued | ScreenerStatus::Downloaded)).count();
                scr_lines.push(ListItem::new(Line::from(Span::styled(
                    format!("  — {} aday: {} indiriliyor, {} puanlandı",
                        sc.len(), queued_n, scored_n),
                    Style::default().fg(Color::Blue),
                ))));
            }

            f.render_widget(
                List::new(scr_lines).block(
                    Block::default().title(title).borders(Borders::ALL)
                        .border_style(Style::default().fg(
                            if st.screener_enabled { Color::LightCyan } else { Color::Blue }
                        )),
                ),
                b4[1],
            );
        }
    }
}

#[allow(dead_code)]
fn draw_ml_brain(f: &mut ratatui::Frame, area: Rect, st: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9),  // ML tahmini + model durumu
            Constraint::Length(8),  // HyperOpt sonuçları
            Constraint::Length(5),  // Son eğitim özeti
            Constraint::Length(10), // MC + Walk-Forward doğrulama paneli
            Constraint::Min(4),     // Sembol adayları tablosu
        ])
        .split(area);

    // ── ML Model Durumu ────────────────────────────────────────────────────
    let signal_color = match st.ml_signal.as_str() {
        "BUY"  => Color::Green,
        "SELL" => Color::Red,
        _      => Color::LightBlue,
    };
    let conf_color = if st.ml_confidence >= 0.6 { Color::Green }
        else if st.ml_confidence >= 0.35 { Color::Yellow }
        else { Color::LightBlue };

    // LiveRiskMap'ten ML model durumunu oku
    let (lr_is_trained, gbt_score, drift_score, oos_wr, oos_ar, oos_bc, oos_folds, ensemble_agr, ml_running) =
        st.live_risk.read().ok().map(|r| {
            (r.ml_weights.is_some(), r.gbt_last_score, r.ml_drift_score,
             r.oos_win_rate, r.oos_avg_return, r.oos_bar_count, r.oos_fold_scores,
             r.ensemble_agreement, r.ml_running)
        }).unwrap_or((false, None, 0.0, 0.0, 0.0, 0, [0.0; 3], 0.0, false));
    let lr_model_label = if ml_running {
        "⏳ ML eğitim devam ediyor..."
    } else if lr_is_trained {
        "LR trained ✓"
    } else {
        "LR defaults"
    };
    let lr_model_color = if ml_running { Color::Cyan } else if lr_is_trained { Color::Green } else { Color::Yellow };
    let gbt_label = match gbt_score {
        Some(s) if s > 0.1  => format!("GBT: BUY  {:.3}", s),
        Some(s) if s < -0.1 => format!("GBT: SELL {:.3}", s),
        Some(s)              => format!("GBT: NÖTR {:.3}", s),
        None                 => "GBT: henüz eğitilmedi".to_string(),
    };
    let gbt_color = match gbt_score {
        Some(s) if s > 0.1  => Color::Green,
        Some(s) if s < -0.1 => Color::Red,
        _                   => Color::LightBlue,
    };

    let model_lines = vec![
        Line::from(vec![
            Span::styled("  ML Sinyal     : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("▶ {}", st.ml_signal),
                Style::default().fg(signal_color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Güven (conf)  : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{:.3}", st.ml_confidence),
                Style::default().fg(conf_color),
            ),
        ]),
        Line::from(vec![
            Span::styled("  LR Ham Skor  : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{:+.4}", st.ml_score),
                Style::default().fg(if st.ml_score > 0.0 { Color::Green } else if st.ml_score < 0.0 { Color::Red } else { Color::LightBlue }),
            ),
        ]),
        Line::from(vec![
            Span::styled("  LR Durum     : ", Style::default().fg(Color::White)),
            Span::styled(lr_model_label, Style::default().fg(lr_model_color)),
        ]),
        Line::from(vec![
            Span::styled("  GBT Durum    : ", Style::default().fg(Color::White)),
            Span::styled(gbt_label, Style::default().fg(gbt_color)),
        ]),
        Line::from(vec![
            Span::styled("  Eğitim Adımı  : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{} batch + online (her işlem)", st.ml_train_count),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Model Tipi    : ", Style::default().fg(Color::White)),
            Span::styled(
                "LR (gradient) + GBT (5 ağaç, depth=3) ensemble",
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Feature Sayısı: ", Style::default().fg(Color::White)),
            Span::styled(
                "19 (RSI,MACD,BB,SMA×3,mom,vol,ATR%,ADX,OBV,BB%B,ROC10,VolRatio)",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Drift Skoru  : ", Style::default().fg(Color::White)),
            Span::styled(
                format!("{:.3}  {}", drift_score,
                    if drift_score > 0.35 { "⚠ Kayma var — ML ağırlığı azaltıldı" }
                    else if drift_score > 0.15 { "~ Hafif kayma" }
                    else { "✓ Stabil" }
                ),
                Style::default().fg(
                    if drift_score > 0.35 { Color::Red }
                    else if drift_score > 0.15 { Color::Yellow }
                    else { Color::Green }
                ),
            ),
        ]),
        Line::from(vec![
            Span::styled("  OOS Win Rate : ", Style::default().fg(Color::White)),
            Span::styled(
                if oos_bc > 0 {
                    format!("{:.1}%  ({} bar)", oos_wr, oos_bc)
                } else { "—  (henüz hesaplanmadı)".to_string() },
                Style::default().fg(
                    if oos_wr >= 55.0 { Color::Green }
                    else if oos_wr >= 45.0 { Color::Yellow }
                    else if oos_bc > 0 { Color::Red }
                    else { Color::LightBlue }
                ),
            ),
        ]),
        Line::from(vec![
            Span::styled("  OOS Avg Ret  : ", Style::default().fg(Color::White)),
            Span::styled(
                if oos_bc > 0 { format!("{:+.3}%", oos_ar) }
                else { "—".to_string() },
                Style::default().fg(if oos_ar >= 0.0 { Color::Green } else { Color::Red }),
            ),
        ]),
        Line::from(vec![
            Span::styled("  OOS Fold Skr : ", Style::default().fg(Color::White)),
            Span::styled(
                if oos_bc > 0 {
                    format!("[{:.0}% / {:.0}% / {:.0}%]",
                        oos_folds[0], oos_folds[1], oos_folds[2])
                } else { "—".to_string() },
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Ensemble Çeş : ", Style::default().fg(Color::White)),
            Span::styled(
                if ensemble_agr > 0.0 {
                    format!("{:.0}% uyum  {}",
                        ensemble_agr * 100.0,
                        if ensemble_agr > 0.85 { "⚠ LR≈GBT (düşük çeşitlilik)" }
                        else if ensemble_agr > 0.65 { "✓ Normal" }
                        else { "★ Yüksek çeşitlilik" }
                    )
                } else { "—  (henüz hesaplanmadı)".to_string() },
                Style::default().fg(
                    if ensemble_agr > 0.85 { Color::Yellow }
                    else if ensemble_agr > 0.0 { Color::Green }
                    else { Color::LightBlue }
                ),
            ),
        ]),
    ];
    f.render_widget(
        Paragraph::new(model_lines).block(
            Block::default()
                .title(" 🧠 ML Model — LR + GBT Ensemble (19 feature, OOS) ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::LightGreen)),
        ),
        chunks[0],
    );

    // ── HyperOpt Sonuçları ─────────────────────────────────────────────────
    let active_strat_name = st.live_strategy.read().ok().map(|s| s.clone()).unwrap_or_else(|| "MA".to_string());
    let hopt_lines = {
        let ma_active = matches!(active_strat_name.as_str(), "MA" | "MA_CROSSOVER");
        let param_label: String = match active_strat_name.as_str() {
            "RSI"      => "RSI (period=14, OB=70, OS=30) — MA HyperOpt geçersiz".to_string(),
            "MACD"     => "MACD (12/26/9) — MA HyperOpt geçersiz".to_string(),
            "BB"       => "Bollinger Bands (20, 2σ) — MA HyperOpt geçersiz".to_string(),
            "DONCHIAN" => "Donchian Channel (20) — MA HyperOpt geçersiz".to_string(),
            "MA" | "MA_CROSSOVER" => String::new(),  // ma_active branch'i kullanır
            other      => format!("{} — MA HyperOpt geçersiz", other),
        };
        let mut lines = vec![];
        if ma_active {
            lines.push(Line::from(vec![
                Span::styled("  Aktif Strateji: ", Style::default().fg(Color::White)),
                Span::styled("MA Crossover (HyperOpt aktif)", Style::default().fg(Color::Magenta)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  En İyi fast   : ", Style::default().fg(Color::White)),
                Span::styled(format!("MA period = {}", st.best_fast), Style::default().fg(Color::Yellow)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  En İyi slow   : ", Style::default().fg(Color::White)),
                Span::styled(format!("MA period = {}", st.best_slow), Style::default().fg(Color::Yellow)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  HyperOpt Skor : ", Style::default().fg(Color::White)),
                Span::styled(
                    format!("{:.6}", st.hyperopt_score),
                    Style::default().fg(if st.hyperopt_score > 0.0 { Color::Green } else { Color::LightBlue }),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  Yöntem        : ", Style::default().fg(Color::White)),
                Span::styled(
                    "Grid Search (fast 5..20 step 2  ×  slow +5..50 step 5)",
                    Style::default().fg(Color::Cyan),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  Kullanım      : ", Style::default().fg(Color::White)),
                Span::styled(
                    "[r] Sıfırla → yeni loop bu parametrelerle başlar",
                    Style::default().fg(Color::White),
                ),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled("  Aktif Strateji: ", Style::default().fg(Color::White)),
                Span::styled(param_label, Style::default().fg(Color::Magenta)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  HyperOpt      : ", Style::default().fg(Color::White)),
                Span::styled(
                    "Bu strateji için HyperOpt uygulanmıyor",
                    Style::default().fg(Color::White),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  MA HyperOpt   : ", Style::default().fg(Color::White)),
                Span::styled(
                    format!("fast={} slow={} (MA moda geçince aktif olur)", st.best_fast, st.best_slow),
                    Style::default().fg(Color::White),
                ),
            ]));
        }
        lines
    };
    f.render_widget(
        Paragraph::new(hopt_lines).block(
            Block::default()
                .title(" 🔍 HyperOpt — En İyi Strateji Parametreleri ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        ),
        chunks[1],
    );

    // ── Son Eğitim Özeti ───────────────────────────────────────────────────
    let remaining_secs = st.ml_next_run_at.saturating_duration_since(Instant::now()).as_secs();
    let summary_text: String = match &st.last_ml_train {
        Some(s) => {
            if remaining_secs > 0 {
                let (m, s2) = (remaining_secs / 60, remaining_secs % 60);
                format!("{} | ⏱ sonraki: {}:{:02}", s, m, s2)
            } else {
                format!("{} | ⏱ çalışıyor...", s)
            }
        }
        None => {
            if remaining_secs > 0 {
                format!("ML worker henüz çalışmadı — ilk çalışma: {} sn sonra", remaining_secs)
            } else {
                "ML worker çalışıyor...".to_string()
            }
        }
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(summary_text.as_str(), Style::default().fg(
                if st.last_ml_train.is_some() { Color::LightGreen } else { Color::LightBlue }
            )),
        ])).block(
            Block::default()
                .title(" 📊 Son ML Eğitim + HyperOpt Özeti ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        ).wrap(ratatui::widgets::Wrap { trim: false }),
        chunks[2],
    );

    // ── Monte Carlo + Walk-Forward Doğrulama Paneli ───────────────────────────
    {
        let val_lines: Vec<Line> = match &st.validation_result {
            None => vec![
                Line::from(vec![
                    Span::styled("  Bekleniyor... ", Style::default().fg(Color::LightBlue)),
                    Span::styled("[m] ile ML worker'ı tetikle → otomatik çalışır", Style::default().fg(Color::Blue)),
                ]),
            ],
            Some(vr) => {
                // Risk rengi
                let (risk_color, risk_label) = match vr.risk_level {
                    RiskLevel::Low      => (Color::Green,  "● Düşük"),
                    RiskLevel::Moderate => (Color::Yellow, "◑ Orta"),
                    RiskLevel::High     => (Color::Red,    "▲ Yüksek"),
                    RiskLevel::Critical => (Color::Red,    "✖ KRİTİK"),
                    RiskLevel::Unknown  => (Color::LightBlue,   "? —"),
                };
                // Skor bar (20 karakter genişlik)
                let bar_w = 20usize;
                let filled = ((vr.composite_score / 100.0) * bar_w as f64).round() as usize;
                let score_bar = format!("{}{}", "█".repeat(filled.min(bar_w)), "░".repeat(bar_w.saturating_sub(filled)));
                let score_color = if vr.composite_score >= 65.0 { Color::Green }
                    else if vr.composite_score >= 40.0 { Color::Yellow }
                    else { Color::Red };

                // MC pozitif bar
                let mc_pos_filled = ((vr.mc_positive_pct / 100.0) * 10.0).round() as usize;
                let mc_pos_bar = format!("{}{}", "█".repeat(mc_pos_filled.min(10)), "░".repeat(10usize.saturating_sub(mc_pos_filled)));

                // WF tutarlılık bar
                let wf_filled = (vr.wf_consistency * 10.0).round() as usize;
                let wf_bar = format!("{}{}", "█".repeat(wf_filled.min(10)), "░".repeat(10usize.saturating_sub(wf_filled)));

                vec![
                    Line::from(vec![
                        Span::styled("  Strateji Sağlığı : ", Style::default().fg(Color::White)),
                        Span::styled(format!("{:.0}/100 [{}] ", vr.composite_score, score_bar), Style::default().fg(score_color)),
                        Span::styled(risk_label, Style::default().fg(risk_color).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("  ({}  {})", vr.strategy_name, vr.computed_at), Style::default().fg(Color::Blue)),
                    ]),
                    Line::from(vec![
                        Span::styled("  MC Ruin Riski    : ", Style::default().fg(Color::White)),
                        Span::styled(
                            format!("{:.1}%  ", vr.mc_ruin_pct),
                            Style::default().fg(if vr.mc_ruin_pct < 5.0 { Color::Green } else if vr.mc_ruin_pct < 15.0 { Color::Yellow } else { Color::Red }),
                        ),
                        Span::styled(
                            format!("P5={:.0}$  P50={:.0}$  P95={:.0}$  ({} sim)",
                                vr.mc_p5_balance, vr.mc_p50_balance, vr.mc_p95_balance, vr.mc_n_sims),
                            Style::default().fg(Color::LightBlue),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("  MC Kârlı Sim     : ", Style::default().fg(Color::White)),
                        Span::styled(
                            format!("{:.0}% [{}]  ", vr.mc_positive_pct, mc_pos_bar),
                            Style::default().fg(if vr.mc_positive_pct >= 60.0 { Color::Green } else if vr.mc_positive_pct >= 40.0 { Color::Yellow } else { Color::Red }),
                        ),
                        Span::styled(
                            format!("Bkl. Getiri: {:+.2}%  MaxDD P50/P95: {:.1}/{:.1}%",
                                vr.mc_expected_ret, vr.mc_max_dd_p50, vr.mc_max_dd_p95),
                            Style::default().fg(Color::LightBlue),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("  WF Tutarlılık    : ", Style::default().fg(Color::White)),
                        Span::styled(
                            format!("{:.0}% [{}]  ", vr.wf_consistency * 100.0, wf_bar),
                            Style::default().fg(if vr.wf_consistency >= 0.6 { Color::Green } else if vr.wf_consistency >= 0.4 { Color::Yellow } else { Color::Red }),
                        ),
                        Span::styled(
                            format!("{}/{} kârlı pencere", vr.wf_profitable, vr.wf_windows),
                            Style::default().fg(Color::LightBlue),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("  WF OOS Metrikleri: ", Style::default().fg(Color::White)),
                        Span::styled(
                            format!("WR={:.0}%  PnL={:+.2}%  PF={:.2}  DD={:.1}%  Sharpe={:.2}",
                                vr.wf_avg_oos_wr, vr.wf_avg_oos_pnl, vr.wf_avg_oos_pf,
                                vr.wf_avg_oos_dd, vr.wf_avg_oos_sharpe),
                            Style::default().fg(if vr.wf_avg_oos_wr >= 55.0 { Color::Green } else if vr.wf_avg_oos_wr >= 45.0 { Color::Yellow } else { Color::LightBlue }),
                        ),
                    ]),
                ]
            }
        };
        f.render_widget(
            Paragraph::new(val_lines).block(
                Block::default()
                    .title(" 🎲 Strateji Doğrulama — Monte Carlo (2000 sim) + Walk-Forward ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(
                        match st.validation_result.as_ref().map(|v| &v.risk_level) {
                            Some(RiskLevel::Low)      => Color::Green,
                            Some(RiskLevel::Moderate) => Color::Yellow,
                            Some(RiskLevel::High) | Some(RiskLevel::Critical) => Color::Red,
                            _ => Color::Blue,
                        }
                    )),
            ),
            chunks[3],
        );
    }

    // ── Sembol Adayları Puan Tablosu ─────────────────────────────────
    let header = format!(
        "  {:<10} {:<8} {:<6}  {:>10}  {:>7}  {:>5}  {:>5}  {:>4}  {:>8}  {}",
        "Sembol", "Market", "Int", "Fiyat", "WinRate", "PF", "DD%", "Trd", "Skor", "Strateji/Durum"
    );
    let mut candidate_lines: Vec<ListItem> = vec![
        ListItem::new(Line::from(Span::styled(
            header,
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ))),
    ];

    let active_key = (
        st.active_symbol.exchange.clone(),
        st.active_symbol.market.clone(),
        st.active_symbol.symbol.clone(),
        st.active_symbol.interval.clone(),
    );
    for (i, cand) in st.symbol_candidates.iter().take(12).enumerate() {
        let is_active = (cand.exchange.clone(), cand.market.clone(), cand.symbol.clone(), cand.interval.clone()) == active_key;
        let status = if is_active { "▶ AKTİF" } else if i == 0 && st.auto_symbol { "▶ #1 bekl." } else { "" };
        let price_str = if cand.last_price > 0.0 {
            if cand.last_price >= 1000.0 {
                format!("{:>10.1}", cand.last_price)
            } else if cand.last_price >= 1.0 {
                format!("{:>10.4}", cand.last_price)
            } else {
                format!("{:>10.6}", cand.last_price)
            }
        } else {
            format!("{:>10}", "-")
        };
        let strat_label = if !cand.best_strategy.is_empty() { &cand.best_strategy } else { "—" };
        let durum_str = if !status.is_empty() {
            format!("{} [{}]", status, strat_label)
        } else {
            strat_label.to_string()
        };
        let row = format!(
            "  {:<10} {:<8} {:<6}  {}  {:>7.1}%  {:>5.2}  {:>5.1}  {:>4}  {:>8.4}  {}",
            &cand.symbol, &cand.market, &cand.interval,
            price_str, cand.win_rate, cand.profit_factor, cand.max_drawdown_pct,
            cand.total_trades, cand.score, durum_str
        );
        let color = if is_active {
            Color::LightGreen
        } else if cand.win_rate >= 60.0 {
            Color::Green
        } else if cand.win_rate >= 40.0 {
            Color::White
        } else {
            Color::LightBlue
        };
        candidate_lines.push(ListItem::new(Line::from(Span::styled(row, Style::default().fg(color)))));
    }

    if st.symbol_candidates.is_empty() {
        candidate_lines.push(ListItem::new(Line::from(Span::styled(
            "  Tarama bekleniyor (başlatıldıktan 15 sn sonra...) | [s] ile anlık tara",
            Style::default().fg(Color::White),
        ))));
    }

    let mode_label = if st.auto_symbol {
        " 🎯 Sembol Adayları — AUTO MOD (▶ sistem seçer) "
    } else {
        " 📌 Sembol Adayları — MANUEL MOD ([s] ile AUTO aç) "
    };
    f.render_widget(
        List::new(candidate_lines).block(
            Block::default()
                .title(mode_label)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(
                    if st.auto_symbol { Color::LightGreen } else { Color::Yellow }
                )),
        ),
        chunks[4],
    );
}

fn draw_positions(f: &mut ratatui::Frame, area: Rect, st: &AppState, trades_scroll: usize) {
    use ratatui::widgets::Table;
    use ratatui::widgets::Row;
    use ratatui::layout::Constraint as C;

    let positions: Vec<_> = st.live_positions.read()
        .map(|m| m.values().cloned().collect())
        .unwrap_or_default();

    // Orchestrator worker arc'ları + primary live_price'tan taze fiyat haritası
    let per_sym_prices = st.orchestrator.build_price_map(Some(&st.live_price));

    // Her pozisyon için taze fiyat: orchestrator veya primary live_price, yoksa son bilinen
    let fresh_price = |p: &memos_trading_core::robot::robotic_loop::LivePositionData| -> f64 {
        per_sym_prices.get(&p.symbol).copied()
            .filter(|&v| v > 0.0)
            .unwrap_or(p.current_price)
    };
    let is_live_price = |sym: &str| -> bool {
        per_sym_prices.get(sym).map_or(false, |&v| v > 0.0)
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),   // özet satırı
            Constraint::Percentage(37), // açık pozisyonlar
            Constraint::Percentage(33), // kapanmış işlemler
            Constraint::Percentage(30), // borsa emirleri (Binance live sync)
        ])
        .split(area);

    // Üst özet satırı
    let (total_pnl, total_notional) = {
        let mut pnl = 0.0_f64;
        let mut notional = 0.0_f64;
        for p in &positions {
            let cur = fresh_price(p);
            pnl += pos_pnl(cur, p.entry_price, p.qty, p.is_long);
            notional += p.entry_price * p.qty;
        }
        (pnl, notional)
    };

    let pnl_color = if total_pnl >= 0.0 { Color::Green } else { Color::Red };
    let summary_line = if positions.is_empty() {
        Line::from(Span::styled(
            "  Açık pozisyon yok",
            Style::default().fg(Color::White),
        ))
    } else {
        Line::from(vec![
            Span::raw(format!("  {} açık pozisyon  |  ", positions.len())),
            Span::styled(
                format!("Açık PnL: {:+.2} USDT", total_pnl),
                Style::default().fg(pnl_color).add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  |  Notional: {:.2} USDT", total_notional)),
        ])
    };

    let any_live = !per_sym_prices.is_empty();
    let price_freshness = if any_live { "canlı fiyat" } else { "~bayat fiyat" };
    f.render_widget(
        Paragraph::new(summary_line).block(
            Block::default()
                .title(format!(" Açık Pozisyonlar — Özet  [{}] ", price_freshness))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(
                    if any_live { Color::Cyan } else { Color::Yellow }
                )),
        ),
        chunks[0],
    );

    if positions.is_empty() {
        let empty = Paragraph::new("  Henüz açık pozisyon yok. Trade döngüsü çalışırken burası otomatik güncellenir.")
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .title(" Açık Pozisyonlar ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::White)),
            );
        f.render_widget(empty, chunks[1]);
        draw_closed_trades(f, chunks[2], st, trades_scroll);
        draw_exchange_orders(f, chunks[3], st);
        return;
    }

    use ratatui::widgets::Cell;

    // Header — koyu arka plan, sarı bold metin
    let header = Row::new(vec![
        Cell::from("Sembol").style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Cell::from("Tür").style(Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        Cell::from("Yön").style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Cell::from("Giriş").style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Cell::from("Şu an").style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Cell::from("Qty").style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Cell::from("SL").style(Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD)),
        Cell::from("TP1").style(Style::default().fg(Color::LightYellow).add_modifier(Modifier::BOLD)),
        Cell::from("TP").style(Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD)),
        Cell::from("TSL").style(Style::default().fg(Color::LightBlue).add_modifier(Modifier::BOLD)),
        Cell::from("PnL%").style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Cell::from("Durum").style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    ])
    .height(1);

    let rows: Vec<Row> = positions.iter().map(|p| {
        let cur      = fresh_price(p);
        let is_live  = is_live_price(&p.symbol);
        let pnl_raw  = pos_pnl(cur, p.entry_price, p.qty, p.is_long);
        let pnl_pct  = if p.entry_price > 0.0 { pos_pnl_pct(cur, p.entry_price, p.is_long) } else { 0.0 };
        let pnl_pos  = pnl_raw >= 0.0;

        let dir_color  = if p.is_long  { Color::LightGreen  } else { Color::LightRed };
        let pnl_color  = if pnl_pos    { Color::LightGreen  } else { Color::LightRed };
        let cur_color  = if is_live    { Color::White        } else { Color::Blue };

        let tsl_str = p.trailing_sl
            .map(|v| format!("{:.4}", v))
            .unwrap_or_else(|| "-".to_string());
        let cur_str = if is_live {
            format!("{:.4}", cur)
        } else {
            format!("~{:.4}", cur)  // ~ = son bilinen (mum kapanışı), taze değil
        };

        // TP1 sütunu: tetiklendiyse "✓ BE" (breakeven'a taşındı), tetiklenmediyse fiyat veya "-"
        let (tp1_str, tp1_color) = match (p.tp1_price, p.tp1_triggered) {
            (_, true)        => ("✓ BE".to_string(), Color::Cyan),
            (Some(v), false) => (format!("{:.4}", v), Color::LightYellow),
            (None,    false) => ("-".to_string(),      Color::Blue),
        };

        // Durum bayrakları — her biri kendi rengiyle
        let mut flag_cells: Vec<ratatui::text::Span> = Vec::new();
        if p.tp1_triggered {
            flag_cells.push(ratatui::text::Span::styled("TP1✓ ", Style::default().fg(Color::Cyan)));
        }
        if p.breakeven_triggered {
            flag_cells.push(ratatui::text::Span::styled("BE ", Style::default().fg(Color::Cyan)));
        }
        if p.partial_tp_triggered {
            flag_cells.push(ratatui::text::Span::styled("P-TP ", Style::default().fg(Color::LightGreen)));
        }
        if p.atr_trail_active {
            flag_cells.push(ratatui::text::Span::styled("ATR-T", Style::default().fg(Color::LightBlue)));
        }
        let flag_line = ratatui::text::Line::from(flag_cells);

        let type_color = match p.trade_type {
            memos_trading_core::robot::scalp_swing::TradeType::Scalp   => Color::Magenta,
            memos_trading_core::robot::scalp_swing::TradeType::Swing   => Color::Cyan,
            memos_trading_core::robot::scalp_swing::TradeType::Regular => Color::White,
        };
        Row::new(vec![
            Cell::from(p.symbol.clone())
                .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Cell::from(p.trade_type.label())
                .style(Style::default().fg(type_color).add_modifier(Modifier::BOLD)),
            Cell::from(if p.is_long { "▲ LONG" } else { "▼ SHORT" })
                .style(Style::default().fg(dir_color).add_modifier(Modifier::BOLD)),
            Cell::from(format!("{:.4}", p.entry_price))
                .style(Style::default().fg(Color::LightBlue)),
            Cell::from(cur_str)
                .style(Style::default().fg(cur_color)),
            Cell::from(format!("{:.4}", p.qty))
                .style(Style::default().fg(Color::LightBlue)),
            Cell::from(format!("{:.4}", p.static_sl))
                .style(Style::default().fg(Color::LightRed)),
            Cell::from(tp1_str)
                .style(Style::default().fg(tp1_color)),
            Cell::from(format!("{:.4}", p.static_tp))
                .style(Style::default().fg(Color::LightGreen)),
            Cell::from(tsl_str)
                .style(Style::default().fg(Color::LightBlue)),
            Cell::from(format!("{:+.2}%", pnl_pct))
                .style(Style::default().fg(pnl_color).add_modifier(Modifier::BOLD)),
            Cell::from(flag_line),
        ])
        .height(1)
    }).collect();

    let widths = [
        C::Length(10), // Sembol
        C::Length(5),  // Tür (SCP/SWG/REG)
        C::Length(8),  // Yön  (▲/▼)
        C::Length(10), // Giriş
        C::Length(10), // Şu an
        C::Length(8),  // Qty
        C::Length(10), // SL
        C::Length(10), // TP1 (yeni)
        C::Length(10), // TP
        C::Length(10), // TSL
        C::Length(8),  // PnL%
        C::Min(12),    // Durum (TP1✓/BE/P-TP/ATR-T)
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(" Pozisyon Detayları  [SL=❌ | TP1=🪜Merdiven | TP=✅ | TSL=📍Trailing] ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::LightBlue)),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED))
        .column_spacing(1);

    f.render_widget(table, chunks[1]);

    draw_closed_trades(f, chunks[2], st, trades_scroll);
    draw_exchange_orders(f, chunks[3], st);
}

// ── Borsa Emirleri Paneli (Tab 4 alt bölüm) ──────────────────────────────────
fn draw_exchange_orders(f: &mut ratatui::Frame, area: Rect, st: &AppState) {
    use ratatui::widgets::{Table, Row};
    use ratatui::layout::Constraint as C;

    let orders = &st.exchange_orders;
    let sync   = &st.exchange_orders_sync;

    // Paper mod veya henüz veri yok
    if orders.is_empty() {
        let msg = if st.paper_mode {
            "  Paper mod — Binance'dan emir çekilmez. [o] live modda aktif olur."
        } else {
            "  Live borsa verisi bekleniyor ([o] ile anında yenile, 60s'de bir otomatik)"
        };
        f.render_widget(
            Paragraph::new(msg)
                .style(Style::default().fg(Color::Blue))
                .block(Block::default()
                    .title(format!(" 🌐 Borsa Emirleri — [o] yenile  [son: {}] ", sync))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Blue))),
            area,
        );
        return;
    }

    let header = Row::new(vec!["Kaynak","Sembol","Yön","Durum","Qty","Dolum","Fiyat","Ort.Fiyat","Gerç.PnL","Saat"])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .height(1);

    let rows: Vec<Row> = orders.iter().map(|o| {
        let status_color = match o.status.as_str() {
            "NEW" | "POSITION"        => Color::Cyan,
            "PARTIALLY_FILLED"        => Color::Yellow,
            "FILLED"                  => Color::LightGreen,
            "CANCELED" | "EXPIRED"    => Color::Blue,
            _                         => Color::White,
        };
        let side_color = match o.side.as_str() {
            "BUY"  | "LONG"  => Color::LightGreen,
            "SELL" | "SHORT" => Color::LightRed,
            _                => Color::White,
        };
        let pnl_color = if o.pnl > 0.0 { Color::LightGreen } else if o.pnl < 0.0 { Color::LightRed } else { Color::Blue };
        let pnl_str = if o.pnl.abs() > 0.001 { format!("{:+.2}", o.pnl) } else { "—".to_string() };
        let price_str  = if o.price     > 0.0 { format!("{:.4}", o.price)     } else { "MKT".to_string() };
        let avg_str    = if o.avg_price > 0.0 { format!("{:.4}", o.avg_price) } else { "—".to_string()   };

        use ratatui::widgets::Cell;
        Row::new(vec![
            Cell::from(o.source.clone()).style(Style::default().fg(Color::LightBlue)),
            Cell::from(o.symbol.clone()),
            Cell::from(o.side.clone()).style(Style::default().fg(side_color)),
            Cell::from(o.status.clone()).style(Style::default().fg(status_color)),
            Cell::from(format!("{:.4}", o.qty)),
            Cell::from(format!("{:.4}", o.filled_qty)),
            Cell::from(price_str),
            Cell::from(avg_str),
            Cell::from(pnl_str).style(Style::default().fg(pnl_color)),
            Cell::from(o.created_at.clone()).style(Style::default().fg(Color::LightBlue)),
        ])
    }).collect();

    let widths = [
        C::Length(10), // kaynak
        C::Length(10), // sembol
        C::Length(6),  // yön
        C::Length(16), // durum
        C::Length(10), // qty
        C::Length(10), // dolum
        C::Length(10), // fiyat
        C::Length(10), // ort fiyat
        C::Length(10), // gerç pnl
        C::Length(8),  // saat
    ];

    let n_active = orders.iter().filter(|o| o.is_active).count();
    let n_hist   = orders.len() - n_active;
    let title = format!(
        " 🌐 Borsa Emirleri — {} açık  {} geçmiş  [o] yenile  [son: {}] ",
        n_active, n_hist, sync
    );

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(
                if st.paper_mode { Color::Blue } else { Color::Cyan }
            )))
        .column_spacing(1);

    f.render_widget(table, area);
}

// ── Multi-strateji Backtest Yardımcısı ───────────────────────────────────────
// MA_CROSSOVER, RSI, MACD ve BOLLINGER stratejilerini aynı mumlarla test eder.
// Kompozit skoru en yüksek stratejiyi (BacktestResult, strateji_adı) olarak döner.
// Hiçbiri çalışmazsa None döner.
fn best_strategy_backtest(
    candles: &[memos_trading_core::types::Candle],
    symbol: &str,
    interval: &str,
    sl_pct: f64,
    tp_pct: f64,
    capital: f64,
    trade_amount: f64,
) -> Option<(memos_trading_core::robot::backtester::engine::BacktestResult, String)> {
    use memos_trading_core::robot::backtester::engine::{Backtester, BacktestConfig};
    use memos_trading_core::robot::optimizer::{strategy_group, interval_category, interval_weight};

    // Tüm desteklenen stratejiler — engine artık OHLCV verilerini tam kullanıyor
    const STRATEGIES: &[&str] = &[
        "MA_CROSSOVER",    // Trend grubu
        "RSI",             // Momentum grubu
        "MACD",            // Trend grubu
        "BOLLINGER",       // Trend grubu
        "EMA",             // Trend grubu
        "DONCHIAN",        // Trend grubu
        "WILLIAMS",        // Momentum grubu
        "CCI",             // Momentum grubu
        "STOCH_RSI",       // Momentum grubu
        "SUPERTREND",      // Trend grubu (ATR tabanlı)
        "PRICE_ACTION",    // Yapısal (engulfing + pin bar)
        "ICT_FVG",         // ICT — Fair Value Gap
        "SMC",             // ICT — Break of Structure / CHoCH
        "ICT_OB",          // ICT — Order Block (displacement sonrası kurumsal bölge)
        "ICT_SWEEP",       // ICT — Liquidity Sweep / Stop Hunt reversal
        "ICT_KILLZONE",    // ICT — Londra/NY Açılışı seans filtreli FVG
        "ICT_OTE",         // ICT — Optimal Trade Entry (%62-79 Fibonacci retrace)
        "ICT_COMPOSITE",   // ICT — Kompozit: MS + Premium/Discount + FVG/OB
    ];

    let int_cat = interval_category(interval);

    let weighted_score = |r: &memos_trading_core::robot::backtester::engine::BacktestResult, sname: &str| -> f64 {
        if r.profit_factor < 1.0 || r.max_drawdown_pct > 40.0 || r.total_trades == 0 {
            return 0.0;
        }
        let win_n  = r.win_rate / 100.0;
        let pf_n   = (r.profit_factor - 1.0).clamp(0.0, 3.0) / 3.0;
        let sr_n   = r.sharpe_ratio.clamp(0.0, 3.0) / 3.0;
        let dd_pen = 1.0 - (r.max_drawdown_pct / 40.0).clamp(0.0, 1.0);
        let tc     = (r.total_trades as f64 / 30.0).clamp(0.10, 1.0);
        let base   = (win_n * 0.25 + pf_n * 0.40 + sr_n * 0.25 + dd_pen * 0.10) * tc;
        // Interval uyumu çarpanı: momentum stratejileri scalp'ta, trend stratejileri intra/swing'de daha değerli
        let grp    = strategy_group(sname);
        let int_w  = interval_weight(grp, int_cat);
        base * int_w
    };

    STRATEGIES.iter().filter_map(|&sname| {
        let cfg = BacktestConfig {
            symbol:           symbol.to_string(),
            interval:         interval.to_string(),
            initial_balance:  capital,
            max_position_size: trade_amount,
            take_profit_pct:  tp_pct,
            stop_loss_pct:    sl_pct,
            strategy_name:    sname.to_string(),
            position_profile: Some("Balanced".to_string()),
            security_profile: Some("Development".to_string()),
            strategy_params:  None,
            commission_pct:   0.001,
            breakeven_at_rr:  None,
            atr_trail_mult:   None,
            partial_tp_ratio: None,
        };
        Backtester::new(cfg).run(candles).ok().map(|r| (r, sname.to_string()))
    })
    .max_by(|(a, sa), (b, sb)| {
        weighted_score(a, sa).partial_cmp(&weighted_score(b, sb))
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// Kompozit skor hesabı — SymbolScore.score alanı için kullanılır.
/// Hem download worker hem auto-scorer aynı formülü kullanır.
fn compute_symbol_score(
    win_rate: f64,
    profit_factor: f64,
    sharpe_ratio: f64,
    max_drawdown_pct: f64,
    total_trades: usize,
    total_pnl: f64,
    candle_count: usize,
    last_price: f64,
    last_candle_ts: i64,
    symbol: &str,
    breakeven_wr: f64,
) -> f64 {
    if !is_valid_symbol_data(candle_count, symbol, last_price, last_candle_ts) {
        return 0.0;
    }
    // Aktif olarak zararlı semboller → negatif skor (tabloda alttan sıralanır, seçilmez)
    // PF < 0.5 ve belirgin negatif PnL → her 100$ zarar için -0.001 ceza
    if profit_factor < 0.5 && total_pnl < -50.0 {
        return (total_pnl / 100_000.0).clamp(-0.01, -0.001);
    }
    if (total_trades <= 5 && total_pnl <= 0.0)
        || profit_factor < 1.0
        || max_drawdown_pct > 40.0
        || win_rate < breakeven_wr
    {
        return 0.0;
    }
    let win_n  = win_rate / 100.0;
    let pf_n   = (profit_factor - 1.0).clamp(0.0, 3.0) / 3.0;
    let sr_n   = sharpe_ratio.clamp(0.0, 3.0) / 3.0;
    let dd_pen = 1.0 - (max_drawdown_pct / 40.0).clamp(0.0, 1.0);
    let tc     = (total_trades as f64 / 30.0).clamp(0.10, 1.0);
    (win_n * 0.25 + pf_n * 0.40 + sr_n * 0.25 + dd_pen * 0.10) * tc
}

fn draw_closed_trades(f: &mut ratatui::Frame, area: Rect, st: &AppState, scroll: usize) {
    use ratatui::widgets::{Table, Row};
    use ratatui::layout::Constraint as C;

    let closed: Vec<_> = st.live_closed_trades.read()
        .map(|log| log.iter().rev().cloned().collect())
        .unwrap_or_default();

    if closed.is_empty() {
        let empty = Paragraph::new("  Henüz kapatılmış işlem yok.")
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .title(" Geçmiş İşlemler ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::White)),
            );
        f.render_widget(empty, area);
        return;
    }

    let total = closed.len();
    // Görünür satır sayısı = alan yüksekliği − başlık(1) − kenarlık(2)
    let visible = (area.height as usize).saturating_sub(3);
    // scroll=0 → offset=0 → en yeni işlemler (listenin başı = rev() ile geldiği için)
    // scroll artar → offset artar → daha eski işlemlere kaydır
    let max_offset = total.saturating_sub(visible);
    let offset = scroll.min(max_offset);

    let header = Row::new(vec!["Sembol", "Yön", "Kaldıraç", "Giriş", "Çıkış", "Qty", "PnL", "PnL%", "SL", "TP", "Neden", "Tarih/Saat"])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .height(1);

    let rows: Vec<Row> = closed.iter().skip(offset).take(visible).map(|t| {
        let pnl_color = if t.pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };
        let sl_str = if t.sl_price > 0.0 && t.entry_price > 0.0 {
            let pct = (t.sl_price - t.entry_price) / t.entry_price * 100.0;
            format!("{:.2} ({:+.2}%)", t.sl_price, pct)
        } else { "-".to_string() };
        let tp_str = if t.tp_price > 0.0 && t.entry_price > 0.0 {
            let pct = (t.tp_price - t.entry_price) / t.entry_price * 100.0;
            format!("{:.2} ({:+.2}%)", t.tp_price, pct)
        } else { "-".to_string() };
        let lev_str = if t.leverage > 1.05 { format!("×{:.1}", t.leverage) } else { "×1.0".to_string() };
        Row::new(vec![
            t.symbol.clone(),
            if t.is_long { "LONG".to_string() } else { "SHORT".to_string() },
            lev_str,
            format!("{:.4}", t.entry_price),
            format!("{:.4}", t.exit_price),
            format!("{:.4}", t.qty),
            format!("{:+.2}", t.pnl),
            format!("{:+.2}%", t.pnl_pct),
            sl_str,
            tp_str,
            t.exit_reason.clone(),
            t.closed_at.clone(),
        ])
        .style(Style::default().fg(pnl_color))
        .height(1)
    }).collect();

    let widths = [
        C::Length(10), // Sembol
        C::Length(6),  // Yön
        C::Length(8),  // Kaldıraç
        C::Length(10), // Giriş
        C::Length(10), // Çıkış
        C::Length(8),  // Qty
        C::Length(10), // PnL
        C::Length(8),  // PnL%
        C::Length(18), // SL (fiyat + %)
        C::Length(18), // TP (fiyat + %)
        C::Length(12), // Neden
        C::Length(19), // Tarih/Saat (YYYY-MM-DD HH:MM:SS)
    ];

    let total_pnl: f64 = closed.iter().map(|t| t.pnl).sum();
    let wins = closed.iter().filter(|t| t.pnl > 0.0).count();
    let pnl_color = if total_pnl >= 0.0 { Color::Green } else { Color::Red };
    // Başlıkta toplam işlem sayısı + scroll konumu göster
    let scroll_hint = if total > visible {
        format!("  ↑↓ [{}-{}/{}]", offset + 1, (offset + visible).min(total), total)
    } else {
        String::new()
    };
    let title = format!(" Geçmiş İşlemler ({}){}  Toplam: ", total, scroll_hint);

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(Line::from(vec![
                    Span::raw(title),
                    Span::styled(format!("{:+.2} USDT", total_pnl), Style::default().fg(pnl_color).add_modifier(Modifier::BOLD)),
                    Span::raw(format!("  Kazanma: {}/{}", wins, total)),
                ]))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::LightMagenta)),
        )
        .column_spacing(1);

    f.render_widget(table, area);
}

// ── Grafik Sekmesi (Tab 7) ────────────────────────────────────────────────────
// Kapalı işlemler için görsel analiz: donut pie chart + yatay PnL barları + özet tablo.
fn draw_charts(f: &mut ratatui::Frame, area: Rect, st: &AppState) {
    use ratatui::widgets::canvas::{Canvas, Points};
    use ratatui::widgets::{Table, Row};
    use ratatui::layout::Constraint as C;
    use ratatui::symbols::Marker;
    use std::f64::consts::PI;

    const SLICE_COLORS: [Color; 8] = [
        Color::Cyan,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightMagenta,
        Color::LightRed,
        Color::LightBlue,
        Color::White,
        Color::LightBlue,
    ];

    // ── Veri ─────────────────────────────────────────────────────────────────
    let closed: Vec<_> = st.live_closed_trades.read()
        .map(|log| log.iter().cloned().collect())
        .unwrap_or_default();

    if closed.is_empty() {
        f.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  Henüz kapatılmış işlem yok — grafikler işlemler kapandıkça otomatik oluşacak.",
                    Style::default().fg(Color::Blue),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Grafik: Donut pasta grafik (sembol dağılımı) + Yatay PnL barları + Özet tablo",
                    Style::default().fg(Color::Blue),
                )),
            ])
            .block(Block::default()
                .title(" 📈 İşlem Grafikleri ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::White))),
            area,
        );
        return;
    }

    // Sembol bazlı istatistik toplanıyor
    let mut sym_map: std::collections::HashMap<String, (f64, u32, u32)> =
        std::collections::HashMap::new();
    for t in &closed {
        let e = sym_map.entry(t.symbol.clone()).or_insert((0.0, 0, 0));
        e.0 += t.pnl;
        e.1 += 1;
        if t.pnl > 0.0 { e.2 += 1; }
    }
    let mut sym_stats: Vec<(String, f64, u32, u32)> = sym_map.into_iter()
        .map(|(s, (p, c, w))| (s, p, c, w))
        .collect();
    // İşlem sayısına göre büyükten küçüğe, eşitlerde alfabetik sırala
    sym_stats.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));

    let total_trades  = closed.len();
    let total_wins    = closed.iter().filter(|t| t.pnl > 0.0).count();
    let total_pnl: f64 = closed.iter().map(|t| t.pnl).sum();
    let total_count   = sym_stats.iter().map(|(_, _, c, _)| *c).sum::<u32>().max(1);
    let slice_n       = sym_stats.len().min(8);

    // ── Ana Layout: özet çubuğu + üst (pasta+bar) + alt (tablo) ─────────────
    let main = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Percentage(55),
            Constraint::Percentage(45),
        ])
        .split(area);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(main[1]);

    // ── Bakiye / PnL / Maliyet hesapları ─────────────────────────────────────
    let open_pnl    = st.orchestrator.total_open_pnl(Some(&*st.live_price));
    let realize_pnl = st.equity - st.config.capital;
    let total_equity = st.equity + open_pnl;
    let costs_snap = st.live_execution_costs.read().ok();
    let (fees_total, reg_pnl, scp_pnl, swg_pnl) = {
        let mut reg = 0.0; let mut scp = 0.0; let mut swg = 0.0;
        use memos_trading_core::robot::scalp_swing::TradeType;
        for t in &closed {
            match t.trade_type {
                TradeType::Regular => reg += t.pnl,
                TradeType::Scalp   => scp += t.pnl,
                TradeType::Swing   => swg += t.pnl,
            }
        }
        let fees = costs_snap.as_ref().map(|c| c.total_cost_usd).unwrap_or(0.0);
        (fees, reg, scp, swg)
    };

    // ── Özet Çubuğu (2 satır) ────────────────────────────────────────────────
    let win_pct = total_wins as f64 / total_trades as f64 * 100.0;
    let pnl_col    = if total_pnl  >= 0.0 { Color::LightGreen } else { Color::LightRed };
    let realize_col= if realize_pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };
    let open_col   = if open_pnl   >= 0.0 { Color::LightGreen } else { Color::LightRed };
    let reg_col    = if reg_pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };
    let scp_col    = if scp_pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };
    let swg_col    = if swg_pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };

    let line1 = Line::from(vec![
        Span::raw("  Sermaye: "),
        Span::styled(format!("${:.2}", total_equity),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw("  │  Realize: "),
        Span::styled(format!("{:+.2}", realize_pnl), Style::default().fg(realize_col)),
        Span::raw("  │  Açık: "),
        Span::styled(format!("{:+.2}", open_pnl), Style::default().fg(open_col)),
        Span::raw("  │  Kesinti: "),
        Span::styled(format!("-{:.2}", fees_total), Style::default().fg(Color::LightRed)),
        Span::raw("  │  Net PnL: "),
        Span::styled(format!("{:+.2}", total_pnl),
            Style::default().fg(pnl_col).add_modifier(Modifier::BOLD)),
    ]);
    let line2 = Line::from(vec![
        Span::raw("  İşlem: "),
        Span::styled(format!("{}", total_trades),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw("  │  Kazanma: "),
        Span::styled(format!("{}/{} ({:.1}%)", total_wins, total_trades, win_pct),
            Style::default().fg(Color::LightGreen)),
        Span::raw("  │  REG: "),
        Span::styled(format!("{:+.2}", reg_pnl), Style::default().fg(reg_col)),
        Span::raw("  SCP: "),
        Span::styled(format!("{:+.2}", scp_pnl), Style::default().fg(scp_col)),
        Span::raw("  SWG: "),
        Span::styled(format!("{:+.2}", swg_pnl), Style::default().fg(swg_col)),
        Span::raw(format!("  │  {} sembol", sym_stats.len())),
    ]);
    f.render_widget(
        Paragraph::new(vec![line1, line2])
        .block(Block::default()
            .title(" 📈 İşlem Grafikleri — Özet ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))),
        main[0],
    );

    // ── Sol: Donut Pasta Grafik (Canvas) + Legend ────────────────────────────
    {
        let pie_split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(top[0]);

        // Saat yönlü kümülatif açılar [0, 2π], saat-12'den başlıyor.
        // Dönüşüm: a_cw = (-atan2(y,x) + π/2).rem_euclid(2π)
        let mut cum_angles = vec![0.0f64];
        for (_, _, count, _) in sym_stats.iter().take(slice_n) {
            let frac = *count as f64 / total_count as f64;
            let last = *cum_angles.last().unwrap();
            cum_angles.push(last + frac * 2.0 * PI);
        }

        // Noktaları önceden hesapla (150×150 ızgara, halka şeklinde)
        let outer_r = 4.2_f64;
        let inner_r = 1.7_f64;
        let steps   = 150_usize;
        let mut slice_pts: Vec<Vec<(f64, f64)>> = vec![Vec::new(); slice_n.max(1)];

        for xi in 0..steps {
            for yi in 0..steps {
                let x = -outer_r + 2.0 * outer_r * xi as f64 / (steps - 1) as f64;
                let y = -outer_r + 2.0 * outer_r * yi as f64 / (steps - 1) as f64;
                let r = (x * x + y * y).sqrt();
                if r < inner_r || r > outer_r { continue; }
                // Saat yönlü açı: saat 12 = 0, saat yönü artar
                let a = y.atan2(x);
                let a_cw = (-a + PI / 2.0).rem_euclid(2.0 * PI);
                for si in 0..slice_n {
                    let in_slice = if si + 1 < cum_angles.len() {
                        a_cw >= cum_angles[si] && a_cw < cum_angles[si + 1]
                    } else {
                        a_cw >= cum_angles[si]
                    };
                    if in_slice {
                        slice_pts[si].push((x, y));
                        break;
                    }
                }
            }
        }

        // Canvas render
        let canvas = Canvas::default()
            .block(Block::default()
                .title(" Sembol Dağılımı ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::LightMagenta)))
            .marker(Marker::Braille)
            .x_bounds([-5.0, 5.0])
            .y_bounds([-5.0, 5.0])
            .paint(move |ctx| {
                for (si, pts) in slice_pts.iter().enumerate() {
                    if pts.is_empty() { continue; }
                    let color = SLICE_COLORS[si % SLICE_COLORS.len()];
                    ctx.draw(&Points { coords: pts, color });
                }
            });
        f.render_widget(canvas, pie_split[0]);

        // Legend: renk + sembol + % pay + PnL + kazanma oranı
        let legend_lines: Vec<Line> = sym_stats.iter().enumerate().take(8)
            .map(|(i, (sym, pnl, cnt, wins))| {
                let color = SLICE_COLORS[i % SLICE_COLORS.len()];
                let pct = *cnt as f64 / total_count as f64 * 100.0;
                let wr  = if *cnt > 0 { *wins as f64 / *cnt as f64 * 100.0 } else { 0.0 };
                let pc  = if *pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };
                let short = if sym.len() > 9 { &sym[..9] } else { sym.as_str() };
                Line::from(vec![
                    Span::styled("█ ", Style::default().fg(color)),
                    Span::styled(
                        format!("{:<9}", short),
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(format!(" {:.0}%  ", pct)),
                    Span::styled(format!("{:+.2}", pnl), Style::default().fg(pc)),
                    Span::raw(format!("  K:{:.0}%", wr)),
                ])
            }).collect();
        f.render_widget(
            Paragraph::new(legend_lines)
                .block(Block::default()
                    .title(" Semboller ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::LightMagenta))),
            pie_split[1],
        );
    }

    // ── Sağ: Yatay PnL Bar Chart ──────────────────────────────────────────────
    {
        let max_abs: f64 = sym_stats.iter()
            .map(|(_, p, _, _)| p.abs())
            .fold(0.01_f64, f64::max);
        // Sabit ön metin: "█ "(2) + sembol(10) + " pnl(10)" + "  "(2) = 24 karakter
        // Sonraki W/L:   "  W/L"(2+3+1+3+1+4) ≈ 14 karakter (maks "  99/99 (100%)")
        // Bar için kalan: genişlik - 24 - 14 = genişlik - 38; maks 20 ile sınırla
        let bar_max = (top[1].width as usize).saturating_sub(42).min(20).max(4);

        let mut lines: Vec<Line> = vec![
            Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    "Sembol      PnL (USDT)  Grafik",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "       W/L",
                    Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
        ];

        for (i, (sym, pnl, cnt, wins)) in sym_stats.iter().enumerate().take(12) {
            let color   = SLICE_COLORS[i % SLICE_COLORS.len()];
            let pc      = if *pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };
            let filled  = ((pnl.abs() / max_abs) * bar_max as f64) as usize;
            let bar_ch  = if *pnl >= 0.0 { "▓" } else { "░" };
            // Çubuğu sabit genişlikte göster: filled ▓/░ + boşluk dolgu
            let bar_filled = bar_ch.repeat(filled);
            let bar_empty  = " ".repeat(bar_max.saturating_sub(filled));
            let wr      = if *cnt > 0 { *wins as f64 / *cnt as f64 * 100.0 } else { 0.0 };
            let short   = if sym.len() > 10 { &sym[..10] } else { sym.as_str() };
            lines.push(Line::from(vec![
                Span::styled("█ ", Style::default().fg(color)),
                Span::styled(
                    format!("{:<10}", short),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {:>9.2}", pnl),
                    Style::default().fg(pc).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(bar_filled, Style::default().fg(pc)),
                Span::raw(bar_empty),
                Span::raw(format!("  {}/{} ({:.0}%)", wins, cnt, wr)),
            ]));
        }

        f.render_widget(
            Paragraph::new(lines)
                .block(Block::default()
                    .title(" Sembol PnL Karşılaştırması ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::LightBlue))),
            top[1],
        );
    }

    // ── Alt: Giriş/Çıkış/Kâr Özet Tablosu ────────────────────────────────────
    {
        let header = Row::new(vec![
            "Sembol", "İşlem", "Kazan", "Kaybet",
            "Ort.Giriş", "Ort.Çıkış", "Ort.PnL%",
            "MaxKâr", "MaxZarar",
        ])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .height(1);

        let rows: Vec<Row> = sym_stats.iter().map(|(sym, sym_pnl, cnt, wins)| {
            let losses = cnt.saturating_sub(*wins);
            let sym_trades: Vec<_> = closed.iter()
                .filter(|t| t.symbol == *sym)
                .collect();
            let n = sym_trades.len() as f64;
            let avg_entry = sym_trades.iter().map(|t| t.entry_price).sum::<f64>() / n;
            let avg_exit  = sym_trades.iter().map(|t| t.exit_price).sum::<f64>()  / n;
            let avg_pct   = sym_trades.iter().map(|t| t.pnl_pct).sum::<f64>()     / n;
            let max_win   = sym_trades.iter().map(|t| t.pnl).fold(0.0_f64, f64::max);
            let max_loss  = sym_trades.iter().map(|t| t.pnl).fold(0.0_f64, f64::min);
            let pc        = if *sym_pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };
            Row::new(vec![
                sym.clone(),
                cnt.to_string(),
                wins.to_string(),
                losses.to_string(),
                format!("{:.4}", avg_entry),
                format!("{:.4}", avg_exit),
                format!("{:+.2}%", avg_pct),
                format!("{:+.2}", max_win),
                format!("{:+.2}", max_loss),
            ])
            .style(Style::default().fg(pc))
            .height(1)
        }).collect();

        let widths = [
            C::Length(11), C::Length(6), C::Length(6), C::Length(7),
            C::Length(11), C::Length(11), C::Length(9),
            C::Length(9),  C::Min(9),
        ];

        let tot_col = if total_pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };
        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default()
                .title(Line::from(vec![
                    Span::raw(" Sembol İstatistikleri — Giriş/Çıkış/Kâr Analizi  "),
                    Span::styled(
                        format!("Toplam: {:+.2} USDT", total_pnl),
                        Style::default().fg(tot_col).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                ]))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::LightCyan)))
            .column_spacing(1);
        f.render_widget(table, main[2]);
    }
}

fn draw_live_prices(f: &mut ratatui::Frame, area: Rect, st: &AppState) {
    use ratatui::widgets::{Table, Row};
    use ratatui::layout::Constraint as C;
    use memos_trading_core::robot::sr_detector::ZoneType;

    let worker_count = st.orchestrator.worker_count();
    let total_pnl    = st.orchestrator.total_open_pnl(Some(&*st.live_price));
    let pnl_color    = if total_pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };

    let summary = if worker_count == 0 {
        Line::from(Span::styled(
            "  Aktif multi-sembol worker yok — veri yüklendikten sonra otomatik başlar",
            Style::default().fg(Color::White),
        ))
    } else {
        Line::from(vec![
            Span::raw(format!("  {} aktif sembol  |  ", worker_count)),
            Span::styled(
                format!("Açık PnL: {:+.2} USDT", total_pnl),
                Style::default().fg(pnl_color).add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  |  Kapasite: {}/{}", worker_count, st.orchestrator.max_workers)),
            Span::raw("  |  [←→] S/R sembol seç"),
        ])
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    f.render_widget(
        Paragraph::new(summary).block(
            Block::default()
                .title(" 🌐 Çok-Sembol Canlı Fiyatlar — S/R Dahil ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta)),
        ),
        chunks[0],
    );

    if worker_count == 0 {
        let hint = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  AUTO mod açıkken semboller puanlandıktan sonra buraya otomatik eklenir.",
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                "  [d] İndir → [s] Tarama → En iyi N sembol spawn edilir",
                Style::default().fg(Color::White),
            )),
        ])
        .block(Block::default().title(" Detay ").borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White)));
        f.render_widget(hint, chunks[1]);
        return;
    }

    // S/R zone haritasını bir kez oku
    let zones_map = st.live_sr_zones.read().ok()
        .map(|g| g.clone())
        .unwrap_or_default();

    // En yakın destek/direnç mesafesini hesapla (% cinsinden)
    let sr_info = |sym: &str, price: f64| -> (String, String, bool) {
        // (destek_str, direnc_str, bölgedeyiz_mi)
        if price <= 0.0 { return ("—".into(), "—".into(), false); }
        let zones = match zones_map.get(sym) { Some(z) => z, None => return ("?".into(), "?".into(), false) };

        let in_zone = zones.iter().any(|z| z.contains(price));

        // En yakın destek: price altındaki en yüksek destek midpoint
        let nearest_sup = zones.iter()
            .filter(|z| z.zone_type == ZoneType::Support && z.midpoint < price)
            .max_by(|a, b| a.midpoint.partial_cmp(&b.midpoint).unwrap_or(std::cmp::Ordering::Equal));

        // En yakın direnç: price üstündeki en düşük direnç midpoint
        let nearest_res = zones.iter()
            .filter(|z| z.zone_type == ZoneType::Resistance && z.midpoint > price)
            .min_by(|a, b| a.midpoint.partial_cmp(&b.midpoint).unwrap_or(std::cmp::Ordering::Equal));

        let sup_str = nearest_sup.map(|z| {
            let pct = (price - z.midpoint) / price * 100.0;
            format!("{:.4}(-{:.1}%)", z.midpoint, pct)
        }).unwrap_or_else(|| "—".into());

        let res_str = nearest_res.map(|z| {
            let pct = (z.midpoint - price) / price * 100.0;
            format!("{:.4}(+{:.1}%)", z.midpoint, pct)
        }).unwrap_or_else(|| "—".into());

        (sup_str, res_str, in_zone)
    };

    let (_, active_mkt, active_sym, active_intv) = st.active_trade_target();

    // Açık pozisyonları bir kez oku
    let pos_map = st.orchestrator.live_positions.read().ok()
        .map(|m| m.clone())
        .unwrap_or_default();

    // Satır verilerini topla
    struct PriceRow {
        sym: String, mkt: String, interval: String,
        close: f64, chg: f64,
        uptime: u64, paused: bool,
        // Açık pozisyon
        has_pos: bool, pos_long: bool, pnl: f64,
        // Backtest kalitesi
        score: f64, win_rate: f64, total_trades: usize,
    }

    // Per-sembol PnL hesabı: "{sym}-Spot" / "{sym}-Futures" key kontrolü
    let compute_pos = |sym: &str, mkt: &str, cur_close: f64| -> (bool, bool, f64) {
        let mkt_key = if mkt == "futures" { "Futures" } else { "Spot" };
        let key = format!("{}-{}", sym, mkt_key);
        if let Some(pos) = pos_map.get(&key) {
            let price = if cur_close > 0.0 { cur_close } else { pos.current_price };
            let pnl = if pos.is_long {
                (price - pos.entry_price) * pos.qty
            } else {
                (pos.entry_price - price) * pos.qty
            };
            (true, pos.is_long, pnl)
        } else {
            (false, false, 0.0)
        }
    };

    // Per-sembol skor/wr/trade lookup (symbol_candidates'tan)
    let candidate_score = |sym: &str| -> (f64, f64, usize) {
        st.symbol_candidates.iter()
            .find(|c| c.symbol == sym)
            .map(|c| (c.score, c.win_rate, c.total_trades))
            .unwrap_or((0.0, 0.0, 0))
    };

    let mut price_rows: Vec<PriceRow> = vec![];

    let primary_close = st.orchestrator.live_price_for(&active_sym)
        .and_then(|arc| arc.read().ok().map(|p| (p.symbol.clone(), p.close, p.change_pct)))
        .filter(|p| p.1 > 0.0)
        .or_else(|| st.live_price.read().ok()
            .filter(|p| p.close > 0.0)
            .map(|p| (p.symbol.clone(), p.close, p.change_pct)));

    if let Some(ref pp) = primary_close.filter(|p| p.0 == active_sym) {
        let (has_pos, pos_long, pnl) = compute_pos(&active_sym, &active_mkt, pp.1);
        let (score, win_rate, total_trades) = candidate_score(&active_sym);
        price_rows.push(PriceRow {
            sym: active_sym.clone(), mkt: active_mkt.clone(),
            interval: active_intv.clone(),
            close: pp.1, chg: pp.2,
            uptime: st.loop_active_since.elapsed().as_secs(), paused: st.paused,
            has_pos, pos_long, pnl, score, win_rate, total_trades,
        });
    }
    for status in st.orchestrator.worker_status() {
        if status.symbol == active_sym { continue; }
        let (close, chg) = if let Some(arc) = st.orchestrator.live_price_for(&status.symbol) {
            arc.read().ok().map(|p| (p.close, p.change_pct)).unwrap_or_default()
        } else { (0.0, 0.0) };
        let (has_pos, pos_long, pnl) = compute_pos(&status.symbol, &status.market, close);
        let (score, win_rate, total_trades) = candidate_score(&status.symbol);
        price_rows.push(PriceRow {
            sym: status.symbol, mkt: status.market,
            interval: status.interval,
            close, chg,
            uptime: status.uptime_secs, paused: status.paused,
            has_pos, pos_long, pnl, score, win_rate, total_trades,
        });
    }

    // Orphan pozisyon sembolleri — aktif worker'ı olmayan ama açık pozisyonu olan semboller
    // st.live_positions: orphan WS besleyici tarafından güncellenen pozisyonlar (futures vb.)
    {
        let listed_syms: std::collections::HashSet<String> =
            price_rows.iter().map(|r| r.sym.clone()).collect();
        // orchestrator.live_positions + st.live_positions'ı birleştir
        let orphan_positions: Vec<(String, f64, bool, f64, f64)> = {
            let mut entries: Vec<(String, f64, bool, f64, f64)> = vec![];
            // 1. orchestrator pozisyonları
            for (key, pos) in &pos_map {
                let parts: Vec<&str> = key.splitn(2, '-').collect();
                if parts.len() != 2 { continue; }
                let sym = parts[0];
                if listed_syms.contains(sym) || sym == active_sym { continue; }
                let mkt_raw = parts[1];
                let mkt_str = if mkt_raw == "Futures" { "futures" } else { "spot" };
                entries.push((format!("{}\x00{}", sym, mkt_str), pos.current_price, pos.is_long, pos.entry_price, pos.qty));
            }
            // 2. st.live_positions (orphan WS tarafından güncellenen)
            if let Ok(live_pos) = st.live_positions.read() {
                for (key, pos) in live_pos.iter() {
                    let parts: Vec<&str> = key.splitn(2, '-').collect();
                    if parts.len() != 2 { continue; }
                    let sym = parts[0];
                    if listed_syms.contains(sym) || sym == active_sym { continue; }
                    // orchestrator'dan zaten eklenmiş mi?
                    if entries.iter().any(|(k, ..)| k.starts_with(&format!("{}\x00", sym))) { continue; }
                    let mkt_raw = parts[1];
                    let mkt_str = if mkt_raw == "Futures" { "futures" } else { "spot" };
                    entries.push((format!("{}\x00{}", sym, mkt_str), pos.current_price, pos.is_long, pos.entry_price, pos.qty));
                }
            }
            entries
        };
        for (key, close, is_long, entry_price, qty) in orphan_positions {
            let mut parts = key.splitn(2, '\x00');
            let sym = parts.next().unwrap_or("");
            let mkt = parts.next().unwrap_or("futures");
            let pnl = if is_long { (close - entry_price) * qty } else { (entry_price - close) * qty };
            let (score, win_rate, total_trades) = candidate_score(sym);
            price_rows.push(PriceRow {
                sym: sym.to_string(), mkt: mkt.to_string(),
                interval: "pos".to_string(),
                close, chg: 0.0,
                uptime: 0, paused: false,
                has_pos: true, pos_long: is_long, pnl, score, win_rate, total_trades,
            });
        }
    }

    // symbol_candidates'ta olan ama listede görünmeyen semboller — worker yoksa da göster
    // (sembol değişiminden sonra eski primary kaybolmasın; "◌ Aday" olarak listelenir)
    {
        let listed_syms: std::collections::HashSet<String> =
            price_rows.iter().map(|r| r.sym.clone()).collect();
        for cand in &st.symbol_candidates {
            if listed_syms.contains(&cand.symbol) { continue; }
            // Son bilinen fiyat: orchestrator price arc veya sıfır
            let (close, chg) = st.orchestrator.live_price_for(&cand.symbol)
                .and_then(|arc| arc.read().ok().map(|p| (p.close, p.change_pct)))
                .filter(|p| p.0 > 0.0)
                .unwrap_or((0.0, 0.0));
            let (has_pos, pos_long, pnl) = compute_pos(&cand.symbol, &cand.market, close);
            price_rows.push(PriceRow {
                sym: cand.symbol.clone(), mkt: cand.market.clone(),
                interval: cand.interval.clone(),
                close, chg,
                uptime: 0, paused: true, // paused=true → "◌ Aday" görünümü
                has_pos, pos_long, pnl,
                score: cand.score, win_rate: cand.win_rate,
                total_trades: cand.total_trades,
            });
        }
    }

    // Terminal genişliğine göre dinamik kolon ayarı
    // Temel kolonlar: sym(10) mkt(5) intv(4) fiyat(11) chg(8) bölge(6) durum(9) poz(11) skor(6) uptime(9) = 79
    let avail_w  = chunks[1].width as usize;
    let sr_col_w = ((avail_w.saturating_sub(86)) / 2).clamp(12, 22) as u16;
    let show_sr  = avail_w >= 100; // S/R kolonları: geniş terminal
    let show_ext = avail_w >= 80;  // Poz/Skor kolonları: orta terminal

    let header_cells: Vec<&str> = if show_sr {
        vec!["Sembol","Mkt","İntv","Fiyat","Değişim%","Bölge","Durum","Poz/PnL","Skor(WR%)","Uptime","Destek(-%)","Direnç(+%)"]
    } else if show_ext {
        vec!["Sembol","Mkt","İntv","Fiyat","Değişim%","Bölge","Durum","Poz/PnL","Skor(WR%)","Uptime"]
    } else {
        vec!["Sembol","Mkt","İntv","Fiyat","Değişim%","Bölge","Uptime"]
    };
    let header = Row::new(header_cells)
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .height(1);

    let rows: Vec<Row> = price_rows.iter().map(|r| {
        let chg_color = if r.chg >= 0.0 { Color::LightGreen } else { Color::LightRed };
        let chg_sym   = if r.chg >= 0.0 { "▲" } else { "▼" };
        let price_str = if r.close > 0.0 { format!("{:.4}", r.close) } else { "—".into() };
        let chg_str   = if r.close > 0.0 { format!("{}{:.2}%", chg_sym, r.chg.abs()) } else { "—".into() };
        let mkt_short = if r.mkt == "futures" { "fut" } else { "spot" };
        let h = r.uptime/3600; let m = (r.uptime%3600)/60; let s = r.uptime%60;
        let uptime_str = format!("{:02}:{:02}:{:02}", h, m, s);

        // Durum: çalışıyor / duraklatıldı / aday (worker yok, uptime=0)
        let is_candidate = r.paused && r.uptime == 0;
        let durum_str = if is_candidate   { "◌ Aday".to_string() }
                        else if r.paused  { "⏸ Duraklat".to_string() }
                        else              { "● Çalışıyor".to_string() };
        let durum_color = if is_candidate { Color::Blue }
                          else if r.paused { Color::LightBlue }
                          else             { Color::LightCyan };

        // Poz/PnL: LONG▲+12.3$ / SHORT▼-1.1$ / —
        let (poz_str, poz_color) = if !r.has_pos {
            ("—".to_string(), Color::LightBlue)
        } else if r.pos_long {
            (format!("▲LONG{:+.2}$", r.pnl), if r.pnl >= 0.0 { Color::LightGreen } else { Color::LightRed })
        } else {
            (format!("▼SHRT{:+.2}$", r.pnl), if r.pnl >= 0.0 { Color::LightGreen } else { Color::LightRed })
        };

        // Skor + Win Rate + Trade sayısı
        let (skor_str, skor_color) = if r.score <= 0.0 {
            ("—".to_string(), Color::LightBlue)
        } else {
            let s = if r.total_trades > 0 {
                format!("{:.2}({:.0}%/{}t)", r.score, r.win_rate, r.total_trades)
            } else {
                format!("{:.2}({:.0}%)", r.score, r.win_rate)
            };
            let c = if r.score >= 0.65 { Color::LightGreen }
                    else if r.score >= 0.40 { Color::LightYellow }
                    else { Color::LightRed };
            (s, c)
        };

        // S/R bölge durumu ve yakın seviyeler
        let (sup_str, res_str, in_zone) = sr_info(&r.sym, r.close);

        // Bölge göstergesi + satır rengi
        let (zone_label, row_color) = if r.close <= 0.0 {
            ("—".to_string(), Color::LightBlue)
        } else if zones_map.get(&r.sym).map(|z| z.is_empty()).unwrap_or(true) {
            ("?".to_string(), chg_color)
        } else {
            let zones = zones_map.get(&r.sym).unwrap();
            let in_sup = zones.iter().any(|z| z.zone_type == ZoneType::Support    && z.contains(r.close));
            let in_res = zones.iter().any(|z| z.zone_type == ZoneType::Resistance && z.contains(r.close));
            if in_sup        { ("●DES".into(), Color::LightGreen) }
            else if in_res   { ("●DİR".into(), Color::LightRed)   }
            else if in_zone  { ("●".into(),    Color::LightYellow) }
            else             { ("–".into(),    chg_color)          }
        };

        use ratatui::text::Span as S;
        if show_sr {
            Row::new(vec![
                ratatui::text::Text::from(r.sym.clone()),
                ratatui::text::Text::from(mkt_short),
                ratatui::text::Text::from(r.interval.clone()),
                ratatui::text::Text::from(price_str),
                ratatui::text::Text::from(chg_str),
                ratatui::text::Text::from(zone_label),
                ratatui::text::Text::from(Line::from(S::styled(durum_str, Style::default().fg(durum_color)))),
                ratatui::text::Text::from(Line::from(S::styled(poz_str, Style::default().fg(poz_color)))),
                ratatui::text::Text::from(Line::from(S::styled(skor_str, Style::default().fg(skor_color)))),
                ratatui::text::Text::from(uptime_str),
                ratatui::text::Text::from(sup_str),
                ratatui::text::Text::from(res_str),
            ]).style(Style::default().fg(row_color)).height(1)
        } else if show_ext {
            Row::new(vec![
                ratatui::text::Text::from(r.sym.clone()),
                ratatui::text::Text::from(mkt_short),
                ratatui::text::Text::from(r.interval.clone()),
                ratatui::text::Text::from(price_str),
                ratatui::text::Text::from(chg_str),
                ratatui::text::Text::from(zone_label),
                ratatui::text::Text::from(Line::from(S::styled(durum_str, Style::default().fg(durum_color)))),
                ratatui::text::Text::from(Line::from(S::styled(poz_str, Style::default().fg(poz_color)))),
                ratatui::text::Text::from(Line::from(S::styled(skor_str, Style::default().fg(skor_color)))),
                ratatui::text::Text::from(uptime_str),
            ]).style(Style::default().fg(row_color)).height(1)
        } else {
            Row::new(vec![
                ratatui::text::Text::from(r.sym.clone()),
                ratatui::text::Text::from(mkt_short),
                ratatui::text::Text::from(r.interval.clone()),
                ratatui::text::Text::from(price_str),
                ratatui::text::Text::from(chg_str),
                ratatui::text::Text::from(zone_label),
                ratatui::text::Text::from(uptime_str),
            ]).style(Style::default().fg(row_color)).height(1)
        }
    }).collect();

    let widths: Vec<C> = if show_sr {
        vec![
            C::Length(10), C::Length(5), C::Length(4),
            C::Length(11), C::Length(8), C::Length(6),
            C::Length(10), C::Length(11), C::Length(10), C::Length(9),
            C::Length(sr_col_w), C::Length(sr_col_w),
        ]
    } else if show_ext {
        vec![
            C::Length(10), C::Length(5), C::Length(4),
            C::Length(11), C::Length(8), C::Length(6),
            C::Length(10), C::Length(11), C::Length(10), C::Length(9),
        ]
    } else {
        vec![
            C::Length(10), C::Length(5), C::Length(4),
            C::Length(11), C::Length(8), C::Length(6), C::Length(9),
        ]
    };

    let paused_count = price_rows.iter().filter(|r| r.paused).count();
    let pos_count    = price_rows.iter().filter(|r| r.has_pos).count();
    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(format!(
                    " Worker Tablosu — {}/{} aktif{}{}",
                    worker_count, st.orchestrator.max_workers,
                    if paused_count > 0 { format!(" | ⏸{} duraklattı", paused_count) } else { String::new() },
                    if pos_count    > 0 { format!(" | {}$ açık poz", pos_count) }        else { String::new() },
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::LightMagenta)),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .column_spacing(1);

    // Tablo tam alanı kaplar; seçilen sembol S/R detayı alt panelde
    let main_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(chunks[1]);

    f.render_widget(table, main_split[0]);
    draw_sr_zones_panel(f, main_split[1], st);
}

/// Tüm aktif semboller için S/R bölgelerini gösteren panel (← → ile sembol gezinme).
fn draw_sr_zones_panel(f: &mut ratatui::Frame, area: Rect, st: &AppState) {
    use ratatui::widgets::List;
    use memos_trading_core::robot::sr_detector::ZoneType;

    let sr_area = area;

    // Mevcut haritayı oku; sembol listesini sırala
    let zones_map = st.live_sr_zones.read().ok()
        .map(|g| g.clone())
        .unwrap_or_default();

    let mut syms: Vec<String> = zones_map.keys().cloned().collect();
    syms.sort();

    // Seçili sembol indeksini sınırla
    let idx = st.sr_tab_sym_idx.min(syms.len().saturating_sub(1));
    let selected_sym = syms.get(idx).cloned().unwrap_or_default();
    let zones = zones_map.get(&selected_sym).cloned().unwrap_or_default();

    // Seçili sembolün canlı fiyatı
    let current_price = st.orchestrator.live_price_for(&selected_sym)
        .and_then(|arc| arc.read().ok().map(|p| p.close))
        .filter(|&p| p > 0.0)
        .unwrap_or(0.0);

    // Sembol nav başlığı: "◄ BTCUSDT (2/11) ►"
    let sym_nav = if syms.is_empty() {
        "  Henüz S/R hesaplanmadı — loop çalışmalı (min ~11 mum)".to_string()
    } else {
        format!(
            " {} ◄ {} ({}/{}) ► | ← → ile sembol seç ",
            if current_price > 0.0 { format!("{:.4}", current_price) } else { "-".to_string() },
            selected_sym, idx + 1, syms.len()
        )
    };

    let items: Vec<ListItem> = if zones.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  Veri bekleniyor — robot loop S/R hesaplar (min ~11 mum gerekli)",
            Style::default().fg(Color::White),
        )))]
    } else {
        let mut items = Vec::new();
        // Direnç bölgeleri (yukarıdan aşağıya)
        let mut res: Vec<_> = zones.iter().filter(|z| z.zone_type == ZoneType::Resistance).collect();
        res.sort_by(|a, b| b.midpoint.partial_cmp(&a.midpoint).unwrap_or(std::cmp::Ordering::Equal));
        for z in &res {
            let dist = if current_price > 0.0 {
                format!(" (+{:.2}%)", (z.midpoint - current_price) / current_price * 100.0)
            } else { String::new() };
            let in_zone = current_price > 0.0 && z.contains(current_price);
            let label = format!(
                "  ▲ DİRENÇ  {:.4}–{:.4}  güç={:.1} [×{}]{}{}",
                z.price_low, z.price_high, z.strength, z.touch_count,
                dist, if in_zone { " ← ŞU AN" } else { "" },
            );
            items.push(ListItem::new(Line::from(Span::styled(
                label,
                if in_zone { Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD) }
                else { Style::default().fg(Color::LightRed) },
            ))));
        }
        if current_price > 0.0 {
            items.push(ListItem::new(Line::from(Span::styled(
                format!("  ── {:.4} (şu an) ──", current_price),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ))));
        }
        // Destek bölgeleri (yukarıdan aşağıya)
        let mut sup: Vec<_> = zones.iter().filter(|z| z.zone_type == ZoneType::Support).collect();
        sup.sort_by(|a, b| b.midpoint.partial_cmp(&a.midpoint).unwrap_or(std::cmp::Ordering::Equal));
        for z in &sup {
            let dist = if current_price > 0.0 {
                format!(" (-{:.2}%)", (current_price - z.midpoint) / current_price * 100.0)
            } else { String::new() };
            let in_zone = current_price > 0.0 && z.contains(current_price);
            let label = format!(
                "  ▼ DESTEK  {:.4}–{:.4}  güç={:.1} [×{}]{}{}",
                z.price_low, z.price_high, z.strength, z.touch_count,
                dist, if in_zone { " ← ŞU AN" } else { "" },
            );
            items.push(ListItem::new(Line::from(Span::styled(
                label,
                if in_zone { Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD) }
                else { Style::default().fg(Color::LightGreen) },
            ))));
        }
        items
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(
                    " Destek/Direnç Bölgeleri — {}{} ",
                    selected_sym,
                    if zones.is_empty() { String::new() } else { format!(" ({} bölge)", zones.len()) },
                ))
                .title_bottom(Line::from(Span::styled(
                    sym_nav,
                    Style::default().fg(Color::Cyan),
                )))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta)),
        );
    f.render_widget(list, sr_area);

}

/// Log satırını ayrıştırıp önemli değerlere renk/sembol ekler.
fn colorize_log_line(line: &str) -> Line<'static> {
    // Satır seviyesi renk
    let base_color = if line.contains("[DIAG]") {
        Color::Magenta
    } else if line.contains("FSM:") {
        Color::Cyan
    } else if line.contains("🧬") {
        Color::LightGreen
    } else if line.contains("RiskGate DENY") || line.contains("🛑") || line.contains("HALT") {
        Color::Red
    } else if line.contains("engellendi") || line.contains("ENGEL") || line.contains("Onay yetersiz") {
        Color::Red
    } else if line.contains("⚠") || line.contains("SafeMode") {
        Color::Yellow
    } else if line.contains("💹") {
        if line.contains('▲') { Color::Green } else { Color::LightRed }
    } else {
        Color::LightBlue
    };

    fn rr_color(v: f64) -> Color {
        if v >= 2.0 { Color::LightGreen } else if v >= 1.5 { Color::Yellow } else { Color::LightRed }
    }
    fn pct_color(v: f64) -> Color {
        if v >= 50.0 { Color::LightGreen } else if v >= 30.0 { Color::Yellow } else { Color::LightBlue }
    }

    let line_owned = line.to_string();

    macro_rules! find_keyword {
        ($kw:expr, $from:expr) => {
            line_owned[$from..].find($kw).map(|p| p + $from)
        };
    }

    // Tüm eşleşmeleri (start, end, renk, metin) topla, sırala, aralarını base_color ile doldur
    struct Seg { start: usize, end: usize, color: Color, bold: bool }
    let mut segs: Vec<Seg> = Vec::new();

    // Yardımcı: sayısal değeri bul
    fn parse_num_after(s: &str, pos: usize) -> Option<(f64, usize)> {
        let sub = &s[pos..];
        let end = sub.find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-').unwrap_or(sub.len());
        sub[..end].parse::<f64>().ok().map(|v| (v, pos + end))
    }

    // BUY / SELL / HOLD
    for (kw, color, _pfx) in &[
        ("BUY",  Color::LightGreen, "▲"),
        ("SELL", Color::LightRed,   "▼"),
        ("HOLD", Color::LightBlue,   "–"),
        ("LONG", Color::LightGreen, ""),
        ("SHORT",Color::LightRed,   ""),
    ] {
        let mut from = 0usize;
        while let Some(pos) = find_keyword!(kw, from) {
            // Sınır kontrolü: önceki/sonraki karakter harf değil mi?
            let before_ok = pos == 0 || !line_owned.as_bytes()[pos-1].is_ascii_alphabetic();
            let after_ok  = pos + kw.len() >= line_owned.len() || !line_owned.as_bytes()[pos + kw.len()].is_ascii_alphabetic();
            if before_ok && after_ok {
                segs.push(Seg { start: pos, end: pos + kw.len(), color: *color, bold: true });
            }
            from = pos + 1;
        }
    }

    // R/R=<num>
    {
        let mut from = 0usize;
        while let Some(pos) = find_keyword!("R/R=", from) {
            let val_start = pos + 4;
            if let Some((v, val_end)) = parse_num_after(&line_owned, val_start) {
                segs.push(Seg { start: pos, end: val_end, color: rr_color(v), bold: false });
            }
            from = pos + 1;
        }
    }
    // composite=<num>
    {
        let mut from = 0usize;
        while let Some(pos) = find_keyword!("composite=", from) {
            let val_start = pos + 10;
            if let Some((v, val_end)) = parse_num_after(&line_owned, val_start) {
                let c = if v >= 0.5 { Color::LightGreen } else if v >= 0.2 { Color::Yellow } else { Color::LightRed };
                segs.push(Seg { start: pos, end: val_end, color: c, bold: false });
            }
            from = pos + 1;
        }
    }
    // B=<num>% (buy yüzdesi)
    {
        let mut from = 0usize;
        while let Some(pos) = find_keyword!("B=", from) {
            let val_start = pos + 2;
            if let Some((v, val_end)) = parse_num_after(&line_owned, val_start) {
                segs.push(Seg { start: pos, end: val_end + (line_owned[val_end..].starts_with('%') as usize), color: pct_color(v), bold: false });
            }
            from = pos + 1;
        }
    }
    // S=<num>% (sell yüzdesi)
    {
        let mut from = 0usize;
        while let Some(pos) = find_keyword!("S=", from) {
            let val_start = pos + 2;
            if let Some((v, val_end)) = parse_num_after(&line_owned, val_start) {
                segs.push(Seg { start: pos, end: val_end + (line_owned[val_end..].starts_with('%') as usize), color: pct_color(v), bold: false });
            }
            from = pos + 1;
        }
    }
    // adx=<num>
    {
        let mut from = 0usize;
        while let Some(pos) = find_keyword!("adx=", from) {
            let val_start = pos + 4;
            if let Some((v, val_end)) = parse_num_after(&line_owned, val_start) {
                let c = if v > 25.0 { Color::LightGreen } else if v > 20.0 { Color::Yellow } else { Color::LightBlue };
                segs.push(Seg { start: pos, end: val_end, color: c, bold: false });
            }
            from = pos + 1;
        }
    }
    // atr_sl_floor / atr_sl_ceil / bb_sl_buf / adx_tp_ext / adx_tp_cap / rr_floor
    for (kw, color) in &[
        ("atr_sl_floor", Color::Yellow),
        ("atr_sl_ceil",  Color::Yellow),
        ("bb_sl_buf",    Color::Cyan),
        ("adx_tp_ext",   Color::LightGreen),
        ("adx_tp_cap",   Color::LightRed),
        ("rr_floor",     Color::Yellow),
        ("sr_optimal",   Color::LightBlue),
        ("sr_sl+pct_tp", Color::Blue),
        ("fallback",     Color::LightBlue),
    ] {
        let mut from = 0usize;
        while let Some(pos) = find_keyword!(kw, from) {
            let end = line_owned[pos..].find(|c: char| c == ')' || c == '|' || c == ' ')
                .map(|p| pos + p + if line_owned.as_bytes().get(pos + p) == Some(&b')') { 1 } else { 0 })
                .unwrap_or(line_owned.len());
            segs.push(Seg { start: pos, end: end.min(line_owned.len()), color: *color, bold: false });
            from = pos + 1;
        }
    }
    // SL= ve TP= fiyat etiketleri
    for kw in &["SL=", "TP=", "sl=", "tp="] {
        let mut from = 0usize;
        while let Some(pos) = find_keyword!(kw, from) {
            let val_start = pos + kw.len();
            if let Some((_v, val_end)) = parse_num_after(&line_owned, val_start) {
                segs.push(Seg { start: pos, end: val_end, color: Color::Cyan, bold: false });
            }
            from = pos + 1;
        }
    }

    // Segs çakışma gider: start'a göre sırala, örtüşenleri at
    segs.sort_by_key(|s| s.start);
    let mut merged: Vec<Seg> = Vec::new();
    for seg in segs {
        if let Some(last) = merged.last() {
            if seg.start < last.end { continue; } // örtüşüyor, atla
        }
        merged.push(seg);
    }

    // Span listesi oluştur
    let mut result: Vec<Span<'static>> = Vec::new();
    let mut cur = 0usize;
    for seg in &merged {
        if cur < seg.start {
            result.push(Span::styled(line_owned[cur..seg.start].to_string(), Style::default().fg(base_color)));
        }
        let style = if seg.bold {
            Style::default().fg(seg.color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(seg.color)
        };
        result.push(Span::styled(line_owned[seg.start..seg.end].to_string(), style));
        cur = seg.end;
    }
    if cur < line_owned.len() {
        result.push(Span::styled(line_owned[cur..].to_string(), Style::default().fg(base_color)));
    }
    if result.is_empty() {
        result.push(Span::styled(line_owned, Style::default().fg(base_color)));
    }

    Line::from(result)
}

fn draw_logs(f: &mut ratatui::Frame, area: Rect, st: &AppState, log_scroll: usize) {
    let visible = (area.height as usize).saturating_sub(2);
    let total   = st.log.len();
    let scroll  = log_scroll.min(total.saturating_sub(1));

    let log_items: Vec<ListItem> = st
        .log
        .iter()
        .rev()
        .skip(scroll)
        .take(visible)
        .map(|line| ListItem::new(colorize_log_line(line)))
        .collect();

    // Başlık: kaydırma konumu + toplam satır + kılavuz
    let title = if scroll == 0 {
        format!(" 📋 Olay Günlüğü  [toplam: {}]  ↑↓ kaydır | PgUp/PgDn | End=en eski ", total)
    } else {
        format!(
            " 📋 Olay Günlüğü  [{}/{}]  ↑↓ kaydır | Home=en yeni | End=en eski ",
            scroll + 1, total
        )
    };

    let border_color = if scroll == 0 { Color::LightBlue } else { Color::Yellow };

    let list = List::new(log_items).block(
        Block::default()
            .title(title.as_str())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color)),
    );
    f.render_widget(list, area);
}

fn draw_htf_intervals(f: &mut ratatui::Frame, area: Rect, st: &AppState, scroll: usize) {
    use ratatui::widgets::Table;
    use ratatui::widgets::Row as TRow;

    // HTF tablosu üstte, MTF fırsat paneli altta
    let mtf_h = if st.mtf_opportunities.is_empty() { 3u16 } else { 7u16 };
    let htf_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),
            Constraint::Length(mtf_h),
        ])
        .split(area);

    // MTF Fırsat panelini render et
    draw_mtf_opportunities(f, htf_chunks[1], st);

    // HTF tablo alanı
    let area = htf_chunks[0];

    const HTF_ROLE: &[&str] = &["base", "→1h", "→4h", "→4h", "→4h", "→1d", "top"];

    // Semboller: htf_candle_counts + orchestrator workers
    let syms: Vec<String> = {
        let mut s: std::collections::HashSet<String> = st.htf_candle_counts.keys().cloned().collect();
        for sym in st.orchestrator.active_symbols() { s.insert(sym); }
        let mut v: Vec<String> = s.into_iter().collect();
        v.sort();
        v
    };

    if syms.is_empty() {
        let p = Paragraph::new("[d] ile veri indirin — 1m mumlar indikten sonra türev interval'ler otomatik üretilir")
            .style(Style::default().fg(Color::White))
            .block(Block::default().borders(Borders::ALL).title(" Interval Türevleri "));
        f.render_widget(p, area);
        return;
    }

    // Aktif (symbol, market, interval) → (win_rate, total_pnl, trades, score) hızlı lookup
    let bt_lookup: std::collections::HashMap<(String, String), (f64, f64, usize, f64)> = {
        let mut m = std::collections::HashMap::new();
        for c in &st.symbol_candidates {
            m.insert(
                (c.symbol.clone(), c.interval.clone()),
                (c.win_rate, c.total_pnl, c.total_trades, c.score),
            );
        }
        m
    };

    // Aktif işlem hedefinin (symbol, interval) çifti — "en iyi" olarak işaretlemek için
    let active_sym_intv = {
        let (_, _, s, i) = st.active_trade_target();
        (s, i)
    };

    // Tek düz tablo: sembol × interval satırları. Semboller arası ayırıcı satır.
    let header = TRow::new(vec!["Sembol", "Intv", "Rol", "Mum", "Son Zaman", "WinRate", "PnL$", "Trd", "Skor", "Durum"])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .height(1);

    let mut rows: Vec<TRow> = Vec::new();

    for sym in &syms {
        let counts = st.htf_candle_counts.get(sym.as_str());
        for (ii, (&intv, &role)) in INTERVALS.iter().zip(HTF_ROLE.iter()).enumerate() {
            let (count, last_ts) = counts
                .and_then(|m| m.get(intv))
                .cloned()
                .unwrap_or((0, "-".to_string()));

            // Durum: mum indirilmediyse "○ İndirilmedi [d]", az ise "⚠ Az mum",
            // 50+ mum var ama backtest sonucu yoksa "◌ Analiz bekliyor", skor hazırsa "✓ Hazır"
            let has_bt = bt_lookup.contains_key(&(sym.clone(), intv.to_string()));
            let (durum, durum_color) = if count == 0 {
                ("○ İndirilmedi [d]", Color::DarkGray)
            } else if count < 50 {
                ("⚠ Az mum", Color::Yellow)
            } else if !has_bt {
                ("◌ Analiz bekliyor", Color::LightBlue)
            } else {
                ("✓ Hazır", Color::LightGreen)
            };

            let intv_color = match intv {
                "1m"               => Color::White,
                "5m"|"15m"|"30m"  => Color::Cyan,
                "1h"               => Color::LightBlue,
                "4h"               => Color::LightMagenta,
                "1d"               => Color::LightYellow,
                _                  => Color::White,
            };

            // Backtest sonuçları (varsa)
            let (wr_s, pnl_s, trd_s, score_s, score_color) =
                if let Some(&(wr, pnl, trd, sc)) = bt_lookup.get(&(sym.clone(), intv.to_string())) {
                    let wr_str  = format!("{:.1}%", wr);
                    let pnl_str = format!("{:+.0}", pnl);
                    let trd_str = trd.to_string();
                    let sc_str  = format!("{:.3}", sc);
                    let sc_col  = if sc >= 0.6 { Color::LightGreen }
                                  else if sc >= 0.35 { Color::Yellow }
                                  else { Color::LightBlue };
                    (wr_str, pnl_str, trd_str, sc_str, sc_col)
                } else {
                    ("—".into(), "—".into(), "—".into(), "—".into(), Color::LightBlue)
                };

            // Aktif trading hedefiyse satırı vurgula
            let is_active = active_sym_intv.0 == *sym && active_sym_intv.1 == intv;

            // İlk satırda sembol adı, geri kalan satırlarda boş
            let sym_cell = if ii == 0 {
                let style = if is_active {
                    Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                };
                ratatui::text::Text::from(Span::styled(sym.clone(), style))
            } else {
                ratatui::text::Text::from("")
            };

            let intv_style = if is_active {
                Style::default().fg(intv_color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(intv_color)
            };

            rows.push(TRow::new(vec![
                sym_cell,
                ratatui::text::Text::from(Span::styled(intv, intv_style)),
                ratatui::text::Text::from(Span::styled(role, Style::default().fg(Color::White))),
                ratatui::text::Text::from(Span::styled(
                    if count == 0 { "—".to_string() } else { count.to_string() },
                    Style::default().fg(if count == 0 { Color::Red } else { Color::LightBlue }),
                )),
                ratatui::text::Text::from(Span::styled(last_ts, Style::default().fg(Color::White))),
                ratatui::text::Text::from(Span::styled(wr_s.clone(), Style::default().fg(
                    if wr_s == "—" { Color::LightBlue }
                    else if wr_s.trim_end_matches('%').parse::<f64>().unwrap_or(0.0) >= 55.0 { Color::LightGreen }
                    else { Color::Yellow }
                ))),
                ratatui::text::Text::from(Span::styled(pnl_s.clone(), Style::default().fg(
                    if pnl_s == "—" { Color::LightBlue }
                    else if pnl_s.starts_with('+') { Color::LightGreen }
                    else { Color::LightRed }
                ))),
                ratatui::text::Text::from(Span::styled(trd_s, Style::default().fg(Color::White))),
                ratatui::text::Text::from(Span::styled(score_s, Style::default().fg(score_color))),
                ratatui::text::Text::from(Span::styled(durum, Style::default().fg(durum_color))),
            ]));
        }
        // Semboller arası boş ayırıcı
        rows.push(TRow::new(vec![""; 10]).height(1));
    }

    let total_rows = rows.len();
    let visible = area.height.saturating_sub(3) as usize; // border + header
    let max_scroll = total_rows.saturating_sub(visible);
    let offset = scroll.min(max_scroll);

    let scroll_info = format!(
        " Interval Türevleri — [d] indir | 1m → 5m/15m/30m/1h/4h/1d otomatik  ↑↓ {}/{} ",
        offset + 1, total_rows.max(1)
    );

    // Manuel slice: sadece görünür satırları al — TableState::offset_mut() güvenilmez
    let visible_rows: Vec<TRow> = rows.into_iter().skip(offset).take(visible).collect();

    let table = Table::new(visible_rows, [
        Constraint::Length(10),  // Sembol
        Constraint::Length(5),   // Interval
        Constraint::Length(5),   // Rol
        Constraint::Length(8),   // Mum
        Constraint::Length(14),  // Son Zaman
        Constraint::Length(8),   // WinRate
        Constraint::Length(8),   // PnL$
        Constraint::Length(5),   // Trd
        Constraint::Length(7),   // Skor
        Constraint::Min(7),      // Durum
    ])
    .header(header)
    .block(
        Block::default()
            .title(scroll_info)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    )
    .column_spacing(1);

    f.render_widget(table, area);
}

// ── Scalp & Swing Pozisyonlar (Tab 9) ────────────────────────────────────────
fn draw_scalp_swing_tab(f: &mut ratatui::Frame, area: Rect, st: &AppState) {
    use ratatui::widgets::{Block, Borders, Table, Row, Cell, Paragraph};
    use ratatui::layout::{Constraint as C, Layout, Direction};
    use ratatui::text::{Line, Span};
    use memos_trading_core::robot::scalp_swing::TradeType;

    let per_sym_prices = st.orchestrator.build_price_map(Some(&st.live_price));

    // ── Açık SCP/SWG pozisyonlar ──────────────────────────────────────────────
    let open_pos: Vec<_> = st.live_positions.read()
        .map(|m| m.values().cloned()
            .filter(|p| p.trade_type != TradeType::Regular)
            .collect())
        .unwrap_or_default();

    // ── Kapalı SCP/SWG işlemler (son 50) ─────────────────────────────────────
    let closed_scp: Vec<_> = st.live_closed_trades.read()
        .map(|v| v.iter().rev()
            .filter(|t| t.trade_type != TradeType::Regular)
            .take(50)
            .cloned()
            .collect())
        .unwrap_or_default();

    // ── PnL hesapları ─────────────────────────────────────────────────────────
    let open_pnl: f64 = open_pos.iter().map(|p| {
        let cp = per_sym_prices.get(&p.symbol).copied().filter(|&v| v > 0.0).unwrap_or(p.current_price);
        if p.is_long { (cp - p.entry_price) * p.qty } else { (p.entry_price - cp) * p.qty }
    }).sum();
    let closed_pnl: f64 = closed_scp.iter().map(|t| t.pnl).sum();
    let total_pnl = open_pnl + closed_pnl;
    // Dashboard ile tutarlı: toplam sermaye = realize (st.equity) + açık PnL
    let total_equity = st.equity + open_pnl;
    let realize_pnl  = st.equity - st.config.capital;

    // ── Per-type performans istatistikleri ────────────────────────────────────
    let scp_trades: Vec<_> = closed_scp.iter().filter(|t| t.trade_type == TradeType::Scalp).collect();
    let swg_trades: Vec<_> = closed_scp.iter().filter(|t| t.trade_type == TradeType::Swing).collect();

    let compute_stats = |trades: &[&memos_trading_core::robot::robotic_loop::ClosedTradeData]| -> (f64, f64, f64, f64, i32) {
        if trades.is_empty() { return (0.0, 0.0, 0.0, 0.0, 0); }
        let wins: Vec<f64>   = trades.iter().filter(|t| t.pnl > 0.0).map(|t| t.pnl).collect();
        let losses: Vec<f64> = trades.iter().filter(|t| t.pnl <= 0.0).map(|t| t.pnl.abs()).collect();
        let win_rate = wins.len() as f64 / trades.len() as f64 * 100.0;
        let gross_win:  f64 = wins.iter().sum();
        let gross_loss: f64 = losses.iter().sum();
        let pf = if gross_loss > 0.0 { gross_win / gross_loss } else { f64::INFINITY };
        let avg_win  = if wins.is_empty()   { 0.0 } else { gross_win  / wins.len()   as f64 };
        let avg_loss = if losses.is_empty() { 0.0 } else { gross_loss / losses.len() as f64 };
        // Güncel streak
        let mut streak: i32 = 0;
        for t in trades.iter().rev() {
            if t.pnl > 0.0 {
                if streak >= 0 { streak += 1; } else { break; }
            } else {
                if streak <= 0 { streak -= 1; } else { break; }
            }
        }
        (win_rate, pf, avg_win, avg_loss, streak)
    };

    let (scp_wr, scp_pf, scp_aw, scp_al, scp_streak) = compute_stats(&scp_trades);
    let (swg_wr, swg_pf, swg_aw, swg_al, swg_streak) = compute_stats(&swg_trades);

    // ── Layout ────────────────────────────────────────────────────────────────
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            C::Length(7),          // üst bilgi paneli
            C::Percentage(40),     // açık pozisyonlar
            C::Length(1),          // ayraç
            C::Min(5),             // kapalı işlemler
        ])
        .split(area);

    // ── Üst bilgi paneli ──────────────────────────────────────────────────────
    let info_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([C::Percentage(34), C::Percentage(33), C::Percentage(33)])
        .split(outer[0]);

    // Sol: Bakiye & genel PnL
    let equity_color = |v: f64| if v >= 0.0 { Color::LightGreen } else { Color::LightRed };
    let equity_lines = vec![
        Line::from(vec![Span::styled(" BAKİYE / PNL", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))]),
        Line::from(vec![
            Span::styled(" Sermaye : ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:.2}$", total_equity), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(" Realize : ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:+.2}$", realize_pnl), Style::default().fg(equity_color(realize_pnl))),
        ]),
        Line::from(vec![
            Span::styled(" Açık PnL: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:+.2}$", open_pnl), Style::default().fg(equity_color(open_pnl))),
        ]),
        Line::from(vec![
            Span::styled(" SCP+SWG: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:+.2}$", total_pnl), Style::default().fg(equity_color(total_pnl)).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(format!(" Açık:{} Kapalı:{}", open_pos.len(), closed_scp.len()), Style::default().fg(Color::Yellow)),
        ]),
    ];
    f.render_widget(
        Paragraph::new(equity_lines).block(Block::default().borders(Borders::ALL)
            .title(Span::styled(" Bakiye ", Style::default().fg(Color::Cyan)))
            .border_style(Style::default().fg(Color::Cyan))),
        info_cols[0],
    );

    // Orta: Scalp istatistikleri
    let streak_color = |s: i32| if s > 0 { Color::LightGreen } else if s < 0 { Color::LightRed } else { Color::DarkGray };
    let scp_lines = vec![
        Line::from(vec![Span::styled(" SCALP İSTATİSTİKLERİ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))]),
        Line::from(vec![
            Span::styled(" Win Rate: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:.1}%", scp_wr), Style::default().fg(if scp_wr >= 50.0 { Color::LightGreen } else { Color::LightRed })),
            Span::styled(format!("  ({} işlem)", scp_trades.len()), Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled(" Kâr Faktörü: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                if scp_pf == f64::INFINITY { "∞".to_string() } else { format!("{:.2}", scp_pf) },
                Style::default().fg(if scp_pf >= 1.5 { Color::LightGreen } else if scp_pf >= 1.0 { Color::Yellow } else { Color::LightRed }),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Ort.Kazanç: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:+.2}$", scp_aw), Style::default().fg(Color::LightGreen)),
            Span::styled("  Ort.Kayıp: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("-{:.2}$", scp_al), Style::default().fg(Color::LightRed)),
        ]),
        Line::from(vec![
            Span::styled(" Streak: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:+}", scp_streak), Style::default().fg(streak_color(scp_streak)).add_modifier(Modifier::BOLD)),
        ]),
    ];
    f.render_widget(
        Paragraph::new(scp_lines).block(Block::default().borders(Borders::ALL)
            .title(Span::styled(" SCP ", Style::default().fg(Color::Magenta)))
            .border_style(Style::default().fg(Color::Magenta))),
        info_cols[1],
    );

    // Sağ: Swing istatistikleri
    let swg_lines = vec![
        Line::from(vec![Span::styled(" SWING İSTATİSTİKLERİ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))]),
        Line::from(vec![
            Span::styled(" Win Rate: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:.1}%", swg_wr), Style::default().fg(if swg_wr >= 50.0 { Color::LightGreen } else { Color::LightRed })),
            Span::styled(format!("  ({} işlem)", swg_trades.len()), Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled(" Kâr Faktörü: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                if swg_pf == f64::INFINITY { "∞".to_string() } else { format!("{:.2}", swg_pf) },
                Style::default().fg(if swg_pf >= 1.5 { Color::LightGreen } else if swg_pf >= 1.0 { Color::Yellow } else { Color::LightRed }),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Ort.Kazanç: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:+.2}$", swg_aw), Style::default().fg(Color::LightGreen)),
            Span::styled("  Ort.Kayıp: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("-{:.2}$", swg_al), Style::default().fg(Color::LightRed)),
        ]),
        Line::from(vec![
            Span::styled(" Streak: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:+}", swg_streak), Style::default().fg(streak_color(swg_streak)).add_modifier(Modifier::BOLD)),
        ]),
    ];
    f.render_widget(
        Paragraph::new(swg_lines).block(Block::default().borders(Borders::ALL)
            .title(Span::styled(" SWG ", Style::default().fg(Color::Cyan)))
            .border_style(Style::default().fg(Color::Cyan))),
        info_cols[2],
    );

    // ── Açık pozisyonlar tablosu ─────────────────────────────────────────────
    // Sütunlar: Tür | Sembol | Yön | Giriş | Güncel | PnL$ | PnL% | Lev | Notional | SL | TP | Süre
    let open_header = Row::new(vec!["Tür","Sembol","Yön","Giriş","Güncel","PnL$","PnL%(M)","Lev","Notional","SL","TP","Süre"])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    let open_rows: Vec<Row> = open_pos.iter().map(|p| {
        let cp = per_sym_prices.get(&p.symbol).copied().filter(|&v| v > 0.0).unwrap_or(p.current_price);
        let pnl = if p.is_long { (cp - p.entry_price) * p.qty } else { (p.entry_price - cp) * p.qty };
        let lev = p.leverage.max(1.0);
        let notional = p.entry_price * p.qty;
        let margin = notional / lev;
        let pnl_pct_margin = if margin > 0.0 { pnl / margin * 100.0 } else { 0.0 };
        let pnl_color = if pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };
        let type_color = match p.trade_type { TradeType::Scalp => Color::Magenta, _ => Color::Cyan };
        let type_label = p.trade_type.label();
        let dir_label  = if p.is_long { "LONG" } else { "SHORT" };
        let dir_color  = if p.is_long { Color::LightGreen } else { Color::LightRed };
        let lev_color  = if lev >= 5.0 { Color::LightRed } else if lev >= 2.0 { Color::Yellow } else { Color::White };

        let duration = if p.opened_at.is_empty() { "—".to_string() } else {
            chrono::DateTime::parse_from_str(&format!("{} +0000", p.opened_at), "%Y-%m-%d %H:%M:%S %z")
                .ok()
                .map(|t| {
                    let secs = (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds().max(0);
                    if secs < 60 { format!("{}s", secs) }
                    else if secs < 3600 { format!("{}m", secs / 60) }
                    else { format!("{}h{}m", secs / 3600, (secs % 3600) / 60) }
                })
                .unwrap_or("—".to_string())
        };

        Row::new(vec![
            Cell::from(type_label).style(Style::default().fg(type_color).add_modifier(Modifier::BOLD)),
            Cell::from(p.symbol.clone()).style(Style::default().fg(Color::White)),
            Cell::from(dir_label).style(Style::default().fg(dir_color)),
            Cell::from(format!("{:.4}", p.entry_price)),
            Cell::from(format!("{:.4}", cp)),
            Cell::from(format!("{:+.2}", pnl)).style(Style::default().fg(pnl_color)),
            Cell::from(format!("{:+.1}%", pnl_pct_margin)).style(Style::default().fg(pnl_color)),
            Cell::from(format!("{:.1}x", lev)).style(Style::default().fg(lev_color)),
            Cell::from(format!("{:.1}", notional)),
            Cell::from(format!("{:.4}", p.static_sl)).style(Style::default().fg(Color::Red)),
            Cell::from(format!("{:.4}", p.static_tp)).style(Style::default().fg(Color::Green)),
            Cell::from(duration),
        ])
    }).collect();

    let open_table = Table::new(open_rows,
        [C::Length(5),C::Length(10),C::Length(6),C::Length(10),C::Length(10),C::Length(9),C::Length(9),C::Length(6),C::Length(10),C::Length(10),C::Length(10),C::Length(6)])
        .header(open_header)
        .block(Block::default().borders(Borders::ALL)
            .title(Span::styled(" ⚡ Açık Pozisyonlar ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
            .border_style(Style::default().fg(Color::Yellow)));
    f.render_widget(open_table, outer[1]);

    // ── Ayraç ────────────────────────────────────────────────────────────────
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("─── Kapalı İşlemler (son 50) ", Style::default().fg(Color::DarkGray)),
        ])),
        outer[2],
    );

    // ── Kapalı işlemler tablosu ───────────────────────────────────────────────
    // Sütunlar: Tür | Sembol | Yön | Giriş | Çıkış | PnL$ | PnL% | Lev | Kom$ | Neden | Kapanış | Süre
    let closed_header = Row::new(vec!["Tür","Sembol","Yön","Giriş","Çıkış","PnL$","PnL%","Lev","Kom$","Neden","Kapanış","Süre"])
        .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD));
    let closed_rows: Vec<Row> = closed_scp.iter().map(|t| {
        let pnl_color = if t.pnl >= 0.0 { Color::LightGreen } else { Color::LightRed };
        let type_color = match t.trade_type { TradeType::Scalp => Color::Magenta, _ => Color::Cyan };
        let type_label = t.trade_type.label();
        let dir_label = if t.is_long { "LONG" } else { "SHORT" };
        let dir_color = if t.is_long { Color::LightGreen } else { Color::LightRed };
        let reason_color = match t.exit_reason.as_str() {
            "SL" | "trailing_sl" => Color::LightRed,
            "TP" | "TP1"         => Color::LightGreen,
            _                    => Color::Yellow,
        };
        let lev = t.leverage.max(1.0);
        let lev_color = if lev >= 5.0 { Color::LightRed } else if lev >= 2.0 { Color::Yellow } else { Color::White };

        // Süre hesabı (açılış→kapanış)
        let duration = if t.opened_at.is_empty() || t.closed_at.is_empty() { "—".to_string() } else {
            let fmt = "%Y-%m-%d %H:%M:%S";
            let open  = chrono::NaiveDateTime::parse_from_str(&t.opened_at, fmt).ok();
            let close = chrono::NaiveDateTime::parse_from_str(&t.closed_at, fmt).ok();
            match (open, close) {
                (Some(o), Some(c)) => {
                    let secs = (c - o).num_seconds().max(0);
                    if secs < 60 { format!("{}s", secs) }
                    else if secs < 3600 { format!("{}m", secs / 60) }
                    else { format!("{}h{}m", secs / 3600, (secs % 3600) / 60) }
                }
                _ => "—".to_string(),
            }
        };

        let total_fees = t.total_fees();
        Row::new(vec![
            Cell::from(type_label).style(Style::default().fg(type_color).add_modifier(Modifier::BOLD)),
            Cell::from(t.symbol.clone()).style(Style::default().fg(Color::White)),
            Cell::from(dir_label).style(Style::default().fg(dir_color)),
            Cell::from(format!("{:.4}", t.entry_price)),
            Cell::from(format!("{:.4}", t.exit_price)),
            Cell::from(format!("{:+.2}", t.pnl)).style(Style::default().fg(pnl_color)),
            Cell::from(format!("{:+.1}%", t.pnl_pct)).style(Style::default().fg(pnl_color)),
            Cell::from(format!("{:.1}x", lev)).style(Style::default().fg(lev_color)),
            Cell::from(format!("{:.3}", total_fees)).style(Style::default().fg(Color::DarkGray)),
            Cell::from(t.exit_reason.clone()).style(Style::default().fg(reason_color)),
            Cell::from(t.closed_at.chars().skip(11).take(8).collect::<String>()),
            Cell::from(duration),
        ])
    }).collect();

    let closed_table = Table::new(closed_rows,
        [C::Length(5),C::Length(10),C::Length(6),C::Length(10),C::Length(10),C::Length(9),C::Length(8),C::Length(6),C::Length(7),C::Length(12),C::Length(9),C::Length(8)])
        .header(closed_header)
        .block(Block::default().borders(Borders::ALL)
            .title(Span::styled(" 📋 Kapalı İşlemler ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)))
            .border_style(Style::default().fg(Color::DarkGray)));
    f.render_widget(closed_table, outer[3]);
}

fn draw_pipeline(f: &mut ratatui::Frame, area: Rect, st: &AppState) {
    use memos_trading_core::robot::robotic_loop::{AnomSeverity, AnomalyKind};
    use ratatui::widgets::{Block, Borders, Paragraph, Table, Row, Cell};
    use ratatui::layout::{Layout, Direction, Constraint};

    let ph = match st.live_pipeline.read() {
        Ok(g) => g.clone(),
        Err(_) => memos_trading_core::robot::robotic_loop::LivePipelineHealth::default(),
    };
    let evo = st.live_evolution.read().ok();

    // ── Layout: Canlı Zincir | Analiz Zinciri | Bileşenler+Anomaliler | Onarım ──
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),   // canlı trading zinciri
            Constraint::Length(8),   // analiz pipeline zinciri (chain_steps)
            Constraint::Min(10),     // bileşenler + anomaliler
            Constraint::Length(7),   // onarım günlüğü
        ])
        .split(area);

    // ── 1) Canlı Trading Zinciri ─────────────────────────────────────────────
    let chain_ok  = |ok: bool| if ok { "✅" } else { "🚨" };
    let feed_ok   = !ph.ws_stale;
    let db_ok     = ph.db_connected;
    let kelly_ok  = ph.kelly_trades_so_far >= ph.kelly_min_trades;
    let evo_ok    = ph.evolution_stuck_count < 10;
    let drift_ok  = ph.drift_score <= ph.drift_threshold;
    let fsm_ok    = st.controller.can_trade();

    let chain_lines = vec![
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(format!("{} Candle", chain_ok(feed_ok)), Style::default().fg(if feed_ok { Color::LightGreen } else { Color::Red })),
            Span::styled(" → ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{} Cache", chain_ok(true)), Style::default().fg(Color::LightGreen)),
            Span::styled(" → ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{} DB", chain_ok(db_ok)), Style::default().fg(if db_ok { Color::LightGreen } else { Color::Red })),
            Span::styled(" → ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{} Kelly", chain_ok(kelly_ok)), Style::default().fg(if kelly_ok { Color::LightGreen } else { Color::Yellow })),
            Span::styled(" → ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{} Evrim", chain_ok(evo_ok)), Style::default().fg(if evo_ok { Color::LightGreen } else { Color::Yellow })),
            Span::styled(" → ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{} Drift", chain_ok(drift_ok)), Style::default().fg(if drift_ok { Color::LightGreen } else { Color::Yellow })),
            Span::styled(" → ", Style::default().fg(Color::Blue)),
            Span::styled(format!("{} FSM", chain_ok(fsm_ok)), Style::default().fg(if fsm_ok { Color::LightGreen } else { Color::Red })),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [E] Mini Evrim Zorla  ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("[F] Funding Yenile  ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("[R] Candle Cache Temizle", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
    ];
    let chain_block = Block::default()
        .title(" 🔗 Canlı Trading Zinciri ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    f.render_widget(
        Paragraph::new(chain_lines).block(chain_block),
        outer[0],
    );

    // ── 2) Analiz Pipeline Zinciri (chain_steps) ─────────────────────────────
    {
        use memos_trading_core::robot::robotic_loop::ChainStepStatus;
        let fmt_age = |secs: u64| -> String {
            if secs >= 999_999 { return "—".to_string(); }
            if secs < 60       { return format!("{}s", secs); }
            if secs < 3600     { return format!("{}dk", secs / 60); }
            format!("{}sa", secs / 3600)
        };
        let step_color = |status: &ChainStepStatus| -> Color {
            match status {
                ChainStepStatus::Ok      => Color::LightGreen,
                ChainStepStatus::Running => Color::Cyan,
                ChainStepStatus::Stale   => Color::Yellow,
                ChainStepStatus::Failed  => Color::Red,
                ChainStepStatus::Pending => Color::LightBlue,
            }
        };
        let step_icon = |status: &ChainStepStatus| -> &'static str {
            match status {
                ChainStepStatus::Ok      => "✅",
                ChainStepStatus::Running => "⟳",
                ChainStepStatus::Stale   => "⚠",
                ChainStepStatus::Failed  => "🚨",
                ChainStepStatus::Pending => "⏳",
            }
        };

        // Zincir akış satırı
        let chain_row: Vec<Span> = if ph.chain_steps.is_empty() {
            vec![Span::styled("  (chain monitor henüz başlamadı — 60s sonra güncellenir)", Style::default().fg(Color::LightBlue))]
        } else {
            let mut spans = vec![Span::raw("  ")];
            for (i, step) in ph.chain_steps.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::styled(" → ", Style::default().fg(Color::Blue)));
                }
                spans.push(Span::styled(
                    format!("{} {}", step_icon(&step.status), step.label),
                    Style::default().fg(step_color(&step.status)),
                ));
            }
            spans
        };

        // Canlı running durumları — chain monitor 60s gecikmesini atlamak için doğrudan oku
        let live_ml_running = st.live_risk.read().ok()
            .map(|r| r.ml_running).unwrap_or(false);
        let live_bt_running = st.backtest_running.load(std::sync::atomic::Ordering::Relaxed);
        let live_dl_running = st.download_active;

        // Canlı age hesaplama — chain_steps'in 60s önbelleğini atlamak için AppState'den oku
        let live_age_secs = |ts: &Option<String>| -> u64 {
            ts.as_deref()
                .and_then(|s| {
                    let s = if s.len() > 19 { &s[..19] } else { s };
                    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()
                })
                .map(|dt| {
                    let utc = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc);
                    (chrono::Utc::now() - utc).num_seconds().max(0) as u64
                })
                .unwrap_or(999_999)
        };
        let dl_live_age = live_age_secs(&st.last_download_at);
        let bt_live_age = live_age_secs(&st.last_backtest_at);
        let ml_live_age = live_age_secs(&st.last_ml_train_at);
        let dl_interval = st.config.download_every_mins * 60;
        let bt_interval = st.config.backtest_every_mins * 60;
        let ml_interval = st.config.backtest_every_mins * 60 + 30;

        // Detay satırları: son çalışma + gecikme + otomatik iyileştirme
        let mut detail_lines: Vec<Line> = vec![Line::from(chain_row), Line::from("")];
        for step in &ph.chain_steps {
            // chain_steps 60s gecikir — running durumunu doğrudan kaynaklardan al
            let is_running = match step.id {
                "ml"       => live_ml_running,
                "backtest" => live_bt_running,
                "download" => live_dl_running,
                _          => matches!(step.status, ChainStepStatus::Running),
            };

            // Son çalışma zamanını ve interval'i canlı kaynaklardan al (download/backtest/ml için)
            let (live_age, live_interval) = match step.id {
                "download" => (dl_live_age, dl_interval),
                "backtest" => (bt_live_age, bt_interval),
                "ml"       => (ml_live_age, ml_interval),
                _          => (step.last_run_secs, step.interval_secs),
            };
            let live_overdue = if live_interval > 0 && live_age != 999_999 {
                live_age as i64 - live_interval as i64
            } else { step.overdue_secs };

            let effective_status = if is_running {
                ChainStepStatus::Running
            } else if live_age == 999_999 {
                ChainStepStatus::Pending
            } else if live_interval > 0 && live_age > live_interval + 120 {
                ChainStepStatus::Stale
            } else {
                ChainStepStatus::Ok
            };

            let age_str  = fmt_age(live_age);
            let due_str  = if is_running {
                " ⏳ çalışıyor...".to_string()
            } else if live_interval > 0 && live_overdue > 60 {
                format!(" ⚠+{}s gecikme", live_overdue)
            } else if live_interval > 0 && live_overdue < -30 {
                format!(" ({}s kaldı)", (-live_overdue))
            } else { String::new() };
            let heal_str = if step.heal_count > 0 { format!(" [{}x otomatik]", step.heal_count) } else { String::new() };
            let hint_str = if !is_running && matches!(effective_status, ChainStepStatus::Stale | ChainStepStatus::Failed | ChainStepStatus::Pending)
                && !step.user_hint.is_empty() {
                format!("  → {}", step.user_hint)
            } else { String::new() };

            let color = step_color(&effective_status);
            detail_lines.push(Line::from(vec![
                Span::styled(format!("  {:<10}", step.label), Style::default().fg(Color::White)),
                Span::styled(format!("son:{:<6}", age_str), Style::default().fg(color)),
                Span::styled(due_str, Style::default().fg(if is_running { Color::LightYellow } else { Color::Yellow })),
                Span::styled(heal_str, Style::default().fg(Color::Cyan)),
                Span::styled(hint_str, Style::default().fg(Color::LightBlue)),
            ]));
        }

        let analysis_block = Block::default()
            .title(" 🔬 Analiz Pipeline Zinciri (İndir→BTest→ML→p5Ana  /  Tarayıcı→MTF→Sinyal) ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta));
        f.render_widget(Paragraph::new(detail_lines).block(analysis_block), outer[1]);
    }

    // ── 3) Bileşenler (sol) + Anomaliler (sağ) ───────────────────────────────
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(outer[2]);

    // Bileşen tablosu
    let wr_pct = if ph.session_closed > 0 {
        ph.session_wins as f64 / ph.session_closed as f64 * 100.0
    } else { 0.0 };
    let kelly_str = if ph.kelly_active {
        format!("{:.2}x ({}/{})", ph.kelly_scale, ph.kelly_trades_so_far, ph.kelly_min_trades)
    } else {
        format!("bekleme ({}/{})", ph.kelly_trades_so_far, ph.kelly_min_trades)
    };
    let funding_str = if ph.funding_applicable {
        format!("{:.4}% ({}dk)", ph.funding_rate * 100.0, ph.funding_age_secs / 60)
    } else {
        "Spot — yok".to_string()
    };
    let evo_cycle_str = evo.as_ref()
        .map(|e| format!("#{} ({})", e.cycle_id, e.genome_id.chars().take(8).collect::<String>()))
        .unwrap_or_else(|| format!("#{}", ph.evolution_cycle));
    let drift_str = format!("{:.3} / {:.2}", ph.drift_score, ph.drift_threshold);

    let comp_rows = vec![
        Row::new(vec![
            Cell::from("Candle Feed").style(Style::default().fg(Color::White)),
            Cell::from(format!("{}sn önce", ph.candle_age_secs))
                .style(Style::default().fg(if !ph.ws_stale { Color::LightGreen } else { Color::Red })),
        ]),
        Row::new(vec![
            Cell::from("DB").style(Style::default().fg(Color::White)),
            Cell::from(if ph.db_connected { "✅ OK" } else { "🚨 HATA" })
                .style(Style::default().fg(if ph.db_connected { Color::LightGreen } else { Color::Red })),
        ]),
        Row::new(vec![
            Cell::from("Kelly Skalası").style(Style::default().fg(Color::White)),
            Cell::from(kelly_str)
                .style(Style::default().fg(if kelly_ok { Color::LightGreen } else { Color::Yellow })),
        ]),
        Row::new(vec![
            Cell::from("Evrim Cycle").style(Style::default().fg(Color::White)),
            Cell::from(evo_cycle_str)
                .style(Style::default().fg(if evo_ok { Color::LightGreen } else { Color::Yellow })),
        ]),
        Row::new(vec![
            Cell::from("Drift Skoru").style(Style::default().fg(Color::White)),
            Cell::from(drift_str)
                .style(Style::default().fg(if drift_ok { Color::LightGreen } else { Color::Yellow })),
        ]),
        Row::new(vec![
            Cell::from("Funding Rate").style(Style::default().fg(Color::White)),
            Cell::from(funding_str)
                .style(Style::default().fg(Color::LightBlue)),
        ]),
        Row::new(vec![
            Cell::from("FSM Durumu").style(Style::default().fg(Color::White)),
            Cell::from(format!("{}", st.controller.state))
                .style(Style::default().fg(if fsm_ok { Color::LightGreen } else { Color::Yellow })),
        ]),
        Row::new(vec![
            Cell::from("Kayıp Serisi").style(Style::default().fg(Color::White)),
            Cell::from(format!("{} / eşik {}", ph.loss_streak, ph.loss_streak_threshold))
                .style(Style::default().fg(
                    if ph.loss_streak == 0 { Color::LightGreen }
                    else if ph.loss_streak < ph.loss_streak_threshold { Color::Yellow }
                    else { Color::Red }
                )),
        ]),
        Row::new(vec![
            Cell::from("Win Rate").style(Style::default().fg(Color::White)),
            Cell::from(format!("{:.0}% ({}/{})", wr_pct, ph.session_wins, ph.session_closed))
                .style(Style::default().fg(
                    if ph.session_closed < 10 { Color::LightBlue }
                    else if wr_pct >= 50.0 { Color::LightGreen }
                    else if wr_pct >= 35.0 { Color::Yellow }
                    else { Color::Red }
                )),
        ]),
    ];
    let comp_table = Table::new(comp_rows, [Constraint::Percentage(45), Constraint::Percentage(55)])
        .block(Block::default()
            .title(" Bileşen Durumu ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)));
    f.render_widget(comp_table, mid[0]);

    // Anomali paneli
    let anom_lines: Vec<Line> = if ph.anomalies.is_empty() {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "  ✅ Tüm sistemler normal",
                Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD),
            )),
        ]
    } else {
        ph.anomalies.iter().map(|a| {
            let icon = match (&a.severity, a.auto_fixed) {
                (_, true)                    => "✅",
                (AnomSeverity::Critical, _)  => "🚨",
                (AnomSeverity::Warning, _)   => "⚠️ ",
            };
            let color = match (&a.severity, a.auto_fixed) {
                (_, true)                    => Color::LightGreen,
                (AnomSeverity::Critical, _)  => Color::Red,
                (AnomSeverity::Warning, _)   => Color::Yellow,
            };
            let tag = match a.kind {
                AnomalyKind::StaleCandles       => "[VERİ]",
                AnomalyKind::EvolutionStuck     => "[EVRİM]",
                AnomalyKind::HighDrift          => "[DRIFT]",
                AnomalyKind::ConsecLosses       => "[KAYIP]",
                AnomalyKind::FsmBlocked         => "[FSM]",
                AnomalyKind::FundingStale       => "[FUND]",
                AnomalyKind::DbDisconnected     => "[DB]",
                AnomalyKind::LowWinRate         => "[WR]",
                AnomalyKind::CircuitBreakerOpen => "[CB]",
                AnomalyKind::PositionStuck      => "[POS]",
                AnomalyKind::SignalDrought      => "[SIG]",
            };
            Line::from(vec![
                Span::styled(format!(" {} {} ", icon, tag), Style::default().fg(color).add_modifier(Modifier::BOLD)),
                Span::styled(a.message.clone(), Style::default().fg(color)),
            ])
        }).chain(
            ph.anomalies.iter().filter(|a| !a.auto_fixed && !a.fix_hint.is_empty()).map(|a| {
                Line::from(vec![
                    Span::styled("        → ", Style::default().fg(Color::Blue)),
                    Span::styled(a.fix_hint.clone(), Style::default().fg(Color::LightBlue)),
                ])
            })
        ).collect()
    };
    let anom_block = Block::default()
        .title(format!(" Anomaliler ({}) ", ph.anomalies.len()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(
            if ph.anomalies.is_empty() { Color::LightGreen }
            else if ph.anomalies.iter().any(|a| a.severity == AnomSeverity::Critical) { Color::Red }
            else { Color::Yellow }
        ));
    f.render_widget(Paragraph::new(anom_lines).block(anom_block), mid[1]);

    // ── 3) Onarım Günlüğü ────────────────────────────────────────────────────
    let repair_lines: Vec<Line> = if ph.repair_log.is_empty() {
        vec![Line::from(Span::styled("  Henüz kayıt yok", Style::default().fg(Color::LightBlue)))]
    } else {
        ph.repair_log.iter().rev().take(5).map(|s| {
            let color = if s.contains("✅") || s.contains("Otomatik") { Color::LightGreen }
                        else if s.contains("⚡") { Color::Cyan }
                        else { Color::LightBlue };
            Line::from(Span::styled(format!("  {}", s), Style::default().fg(color)))
        }).collect()
    };
    let repair_block = Block::default()
        .title(" Onarım Günlüğü (son 5) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));
    f.render_widget(Paragraph::new(repair_lines).block(repair_block), outer[3]);
}

fn draw_help(f: &mut ratatui::Frame, area: Rect) {
    // Renk paleti: tuş=[Cyan bold], açıklama=[Gray], ayraç=[DarkGray]
    let key  = |k: &'static str| Span::styled(k, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
    let desc = |d: &'static str| Span::styled(d, Style::default().fg(Color::LightBlue));
    let sep  = || Span::styled(" │ ", Style::default().fg(Color::Blue));

    // Satır 1: Tab seçimi + scroll
    let line1 = Line::from(vec![
        key("[1]"), desc("Dashboard"),   sep(),
        key("[2]"), desc("AI Merkezi"), sep(),
        key("[3]"), desc("Günlük"),     sep(),
        key("[4]"), desc("Pozisyon"),   sep(),
        key("[5]"), desc("Fiyatlar"),   sep(),
        key("[6]"), desc("HTF"),        sep(),
        key("[7]"), desc("Grafikler"),  sep(),
        Span::styled("↑↓ jk PgUp/Dn g/G", Style::default().fg(Color::Blue)),
        Span::styled(" scroll", Style::default().fg(Color::LightBlue)),
    ]);

    // Satır 2: Aksiyon tuşları — renkler işlev grubuna göre
    let action = |k: &'static str| Span::styled(k, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    let danger  = |k: &'static str| Span::styled(k, Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD));
    let line2 = Line::from(vec![
        action("[e]"), desc("Ayarlar"), sep(),
        action("[l]"), desc("Durum"),   sep(),
        action("[s]"), desc("AUTO"),    sep(),
        action("[d]"), desc("İndir"),   sep(),
        action("[b]"), desc("BTest"),   sep(),
        action("[m]"), desc("ML"),      sep(),
        action("[u]"), desc("MTF"),     sep(),
        action("[y]"), desc("p5Ana"),   sep(),
        action("[w]"), desc("Pipeline"),sep(),
        action("[t]"), desc("Sinyal"),  sep(),
        action("[f]"), desc("Tarayıcı"),sep(),
        action("[o]"), desc("BorsaSync"),sep(),
        action("[i]"), desc("Interval"),sep(),
        action("[z]"), desc("PaperSfr"),sep(),
        action("[p]"), desc("Duraklat"),sep(),
        action("[r]"), desc("Sıfırla"), sep(),
        action("[x]"), desc("Export"),  sep(),
        danger("[q]"), desc("Çıkış"),
    ]);

    let help = Paragraph::new(vec![line1, line2])
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::Blue)),
        );
    f.render_widget(help, area);
}

// ─── Ayarlar Overlay ──────────────────────────────────────────────────────────

const SETTINGS_LABELS: &[&str] = &[
    "Trade Amount",     // 0
    "Capital ($)",      // 1
    "Interval",         // 2  → loop restart
    "Backtest (dk)",    // 3
    "İndirme (dk)",     // 4
    "Manuel SL %",      // 5
    "Manuel TP %",      // 6
    "Sembol",           // 7  → loop restart + auto_symbol off
    "Market",           // 8  → loop restart
    "Top N İndir",      // 9
    "Oto-Interval",     // 10 → bool toggle: otomatik interval geçişi
    "HTF Filtre",       // 11 → bool toggle: üst TF trend yönüne zıt girişleri engelle
    "Base Kaldıraç",    // 12 → dinamik kaldıraç taban değeri (7.0x varsayılan)
    "Max Kaldıraç",     // 13 → dinamik kaldıraç üst sınırı (10.0x varsayılan)
    "SL Cooldown",      // 14 → SL sonrası bekleme süresi (sn)
    "Breakeven R",      // 15 → kâr bu R katında SL entry'e çekil (None = kapalı)
    "ATR Trail",        // 16 → ATR trailing çarpanı (None = kapalı)
    "Kısmi TP",         // 17 → TP'de bu oranda kapat (None = kapalı)
    "Günlük Limit",    // 18 → günlük maksimum işlem sayısı (0 = sınırsız)
    // ── Adaptif Korumalar (adaptive_params.json) ──────────────────────────────
    "SHORT HTF Blok",  // 19 → HTF Bullish'te SHORT engelle (bool)
    "LONG HTF Blok",   // 20 → HTF Bearish'te LONG engelle (bool)
    "TP ATR Çarpanı",  // 21 → TP mesafesi = ATR × bu çarpan
    "SL ATR Çarpanı",  // 22 → SL mesafesi = ATR × bu çarpan (↑ = daha geniş SL)
    "TSL Aktiv.%",     // 23 → TSL bu kâr % olmadan aktif olmaz
    "Gün SL Limiti",   // 24 → sembol başına günlük max SL (0=kapalı)
    "Max Kayıp Serisi",// 25 → global ardışık kayıp duraksama (0=kapalı)
    "SHORT ML Eşiği",  // 26 → Futures SHORT için min GBT confidence
    "Otonom Mod",      // 27 → auto_adjust her N trade (0=kapalı)
    "Otonom Optimize", // 28 → → tuşu: istatistiklere göre parametreleri hesapla & uygula
];
const INTERVALS: &[&str] = &["1m","3m","5m","15m","30m","1h","4h","1d"];

fn centered_rect(pct_x: u16, pct_y: u16, r: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(vert[1])[1]
}

fn draw_settings_overlay(f: &mut ratatui::Frame, area: Rect, idx: usize, st: &AppState) {
    // Popup yüksekliğini içerik sayısına göre ayarla (en az 14 satır + border)
    let n = SETTINGS_LABELS.len();
    let needed_pct = ((n + 4) as u16 * 100) / area.height.max(1);
    let popup_pct_y = needed_pct.clamp(60, 95);
    let popup = centered_rect(62, popup_pct_y, area);
    f.render_widget(Clear, popup);

    let items: Vec<ListItem> = SETTINGS_LABELS.iter().enumerate().map(|(i, label)| {
        let rr = if st.best_sl > 0.0 { st.best_tp / st.best_sl } else { 0.0 };
        let value = match i {
            0 => format!("{:.4}", st.config.trade_amount),
            1 => format!("{:.0}", st.config.capital),
            2 => st.config.interval.clone(),
            3 => format!("{}", st.config.backtest_every_mins),
            4 => format!("{}", st.config.download_every_mins),
            5 => format!("{:.1}%", st.best_sl),
            6 => {
                let rr_warn = if rr < 1.5 { " ⚠<1.5x" } else { "" };
                format!("{:.1}%  (rr={:.1}x{})", st.best_tp, rr, rr_warn)
            }
            7 => format!("{} [{}]", st.config.symbol, st.config.market),
            8 => st.config.market.clone(),
            9 => format!("{}", st.config.download_top_n),
            10 => {
                let rec = st.best_interval_rec.as_ref()
                    .map(|(bi, sc, wr, _)| format!("öneri:{bi} s={sc:.2} w={wr:.0}%"))
                    .unwrap_or_else(|| "analiz bekleniyor".to_string());
                format!("{}  ({})", if st.auto_interval { "AÇIK ✓" } else { "KAPALI" }, rec)
            }
            11 => {
                let htf_bias_str = st.live_risk.read().ok()
                    .and_then(|lrm| lrm.htf_trend_bias.map(|b| match b {
                        1  => "Bullish ↑".to_string(),
                        -1 => "Bearish ↓".to_string(),
                        _  => "Neutral →".to_string(),
                    }))
                    .unwrap_or_else(|| "bekleniyor".to_string());
                let enabled = st.live_risk.read().ok().map(|lrm| lrm.htf_filter_enabled).unwrap_or(true);
                format!("{}  (HTF: {})", if enabled { "AÇIK ✓" } else { "KAPALI" }, htf_bias_str)
            }
            12 => {
                let base_lev = st.live_risk.read().ok().map(|lrm| lrm.base_leverage).unwrap_or(7.0);
                format!("{:.1}x  (min)", base_lev)
            }
            13 => {
                let max_lev = st.live_risk.read().ok().map(|lrm| lrm.max_leverage).unwrap_or(10.0);
                let base_lev = st.live_risk.read().ok().map(|lrm| lrm.base_leverage).unwrap_or(7.0);
                format!("{:.1}x  (aktif aralık: {:.1}x–{:.1}x)", max_lev, base_lev, max_lev)
            }
            14 => format!("{} sn", st.pos_sl_cooldown),
            15 => st.pos_breakeven_at_rr.map(|v| format!("{:.1}R", v)).unwrap_or_else(|| "KAPALI".into()),
            16 => st.pos_atr_trail_mult.map(|v| format!("{:.1}x ATR", v)).unwrap_or_else(|| "KAPALI".into()),
            17 => st.pos_partial_tp_ratio.map(|v| format!("%{:.0}", v * 100.0)).unwrap_or_else(|| "KAPALI".into()),
            18 => if st.max_daily_trades == 0 { "SINIRSIS".into() } else { format!("{} işlem/gün", st.max_daily_trades) },
            // ── Adaptif Korumalar ──────────────────────────────────────────────
            19 => if st.adaptive_params.short_htf_block { "AÇIK ✓".into() } else { "KAPALI".into() },
            20 => if st.adaptive_params.long_htf_block  { "AÇIK ✓".into() } else { "KAPALI".into() },
            21 => format!("{:.1}x ATR", st.adaptive_params.tp_atr_multiplier),
            22 => format!("{:.2}x ATR", st.adaptive_params.sl_atr_multiplier),
            23 => format!("{:.1}% kâr", st.adaptive_params.trailing_sl_activation_pct),
            24 => if st.adaptive_params.max_daily_sl_per_symbol == 0 {
                "KAPALI".into()
            } else {
                format!("{}/gün/sembol", st.adaptive_params.max_daily_sl_per_symbol)
            },
            25 => if st.adaptive_params.max_consecutive_losses == 0 {
                "KAPALI".into()
            } else {
                format!("{} kayıp dur", st.adaptive_params.max_consecutive_losses)
            },
            26 => format!("{:.2} conf", st.adaptive_params.futures_short_min_conf),
            27 => if st.adaptive_params.adjust_every_n_trades == 0 {
                "KAPALI".into()
            } else {
                format!("her {} işlem", st.adaptive_params.adjust_every_n_trades)
            },
            28 => {
                // Anlık istatistikleri özetle
                let (wr, rr, tsl_pct_val) = if let Ok(lr) = st.live_risk.read() {
                    let wr = if lr.session_closed > 0 {
                        lr.session_wins as f64 / lr.session_closed as f64 * 100.0
                    } else { 0.0 };
                    let rr = lr.session_rr;
                    let tsl_pct_v = st.live_closed_trades.read().ok().map(|c| {
                        if c.is_empty() { 0.0 } else {
                            c.iter().filter(|t| t.exit_reason.contains("trailing")).count() as f64
                                / c.len() as f64 * 100.0
                        }
                    }).unwrap_or(0.0);
                    (wr, rr, tsl_pct_v)
                } else { (0.0, 0.0, 0.0) };
                format!("WR={:.0}% RR={:.2} TSL={:.0}%", wr, rr, tsl_pct_val)
            },
            _ => String::new(),
        };
        let hint = match i {
            0 => " ← → ±0.001",
            1 => " ← → ±100",
            2 => " ← → geç  [restart!]",
            3 => " ← → ±1 dk",
            4 => " ← → ±1 dk",
            5 => " ← → ±0.1%  [otonom]",
            6 => " ← → ±0.1%  [otonom]",
            7 => " ← → semboller  [restart!]",
            8 => " ← → spot/futures  [restart!]",
            9 => " ← → ±1",
            10 => " ← → aç/kapat",
            11 => " ← → aç/kapat",
            12 => " ← → ±0.5x  (1.0–9.5)",
            13 => " ← → ±0.5x  (base+0.5–20.0)",
            14 => " ← → ±60sn  (0=kapalı)",
            15 => " ← → ±0.1R  (→ aç, ←min'de kapat)",
            16 => " ← → ±0.5x  (→ aç, ←min'de kapat)",
            17 => " ← → ±10%  (→ aç, ←min'de kapat)",
            18 => " ← → ±1  (0=sınırsız, → artır, ←0'da kapat)",
            19 => " ← → aç/kapat  [disk'e kaydeder]",
            20 => " ← → aç/kapat  [disk'e kaydeder]",
            21 => " ← → ±0.1x  (0.5–6.0)  [disk'e kaydeder]",
            22 => " ← → ±0.25x  (0.5–3.0)  [disk'e kaydeder]",
            23 => " ← → ±0.1%  (0.3–5.0%)  [disk'e kaydeder]",
            24 => " ← → ±1  (0=kapalı, 1-10)  [disk'e kaydeder]",
            25 => " ← → ±1  (0=kapalı, 1-20)  [disk'e kaydeder]",
            26 => " ← → ±0.05  (0.0–0.90)  [disk'e kaydeder]",
            27 => " ← → ±5 işlem  (0=otonom kapalı)  [disk'e kaydeder]",
            28 => " [e] veya [→] Uygula  (UCB1+ML+Equity ile tüm parametreleri otomatik ayarlar)",
            _ => "",
        };
        let style = if i == idx {
            Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        ListItem::new(Line::from(vec![
            Span::styled(format!(" {:15}", label), style),
            Span::styled(format!(" {:>18}", value), style.fg(if i == idx { Color::Black } else { Color::LightGreen })),
            Span::styled(hint.to_string(), Style::default().fg(Color::White)),
        ]))
    }).collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(
                    " ⚙  Ayarlar [{}/{}]  [↑↓] Seç  [←→] Değiştir  [e] Optimize  [Esc] Kapat ",
                    idx + 1, SETTINGS_LABELS.len()
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::LightYellow))
        )
        .highlight_symbol("▶ ")
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD));

    let inner_h = popup.height.saturating_sub(2) as usize; // border hariç
    let offset = if idx >= inner_h { idx - inner_h + 1 } else { 0 };
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(idx));
    // offset manuel — ratatui'nin ListState::offset() API'si sürüme göre değişir
    *list_state.offset_mut() = offset;
    f.render_stateful_widget(list, popup, &mut list_state);
}

// ─── Ana TUI Döngüsü ──────────────────────────────────────────────────────────

/// true döndürürse main process'i yeniden başlatır (F5 restart)
fn run_tui(state: Arc<Mutex<AppState>>) -> io::Result<bool> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut active_tab: usize = 0;
    let mut settings_open: bool = false;
    let mut settings_idx: usize = 0;
    let mut htf_scroll:     usize = 0;
    let mut log_scroll:     usize = 0;
    let mut trades_scroll:  usize = 0;
    let mut loading_scroll:     usize = 0;
    let mut loading_max_scroll: usize = 0;
    let mut show_progress: bool = false; // [p] tuşuyla açılan indirme/backtest overlay
    let tick_rate = Duration::from_millis(150);
    let mut last_tick = Instant::now();
    let mut loading_frame: u64 = 0; // spinner için frame sayacı
    let mut last_worker_info: (usize, usize) = (0, 16); // (aktif, toplam)
    let mut last_symbol: String = "—".to_string();
    let mut last_auto_export = Instant::now(); // 60dk periyodik export
    let mut restart_requested = false; // F5 ile process restart
    // Dashboard bir kez açıldıktan sonra tekrar loading ekranı gösterme.
    // Loop restart, worker sıfırlama vb. geçici durumlardan etkilenmez.
    let mut ever_ready: bool = false;
    // Loading ekranı için detay cache'i
    let mut last_workers: Vec<(String, String, String, bool, u64)> = vec![]; // (sym, mkt, intv, paused, uptime_secs)
    let mut last_download_active = false;
    let mut last_download_progress: Option<DownloadProgress> = None;
    let mut last_download_summary: String = String::new();
    let mut last_backtest_done = false; // en az bir kez tamamlandı mı
    let mut last_log_tail: Vec<String> = vec![];

    // Ardışık restart debounce — 2sn içinde birden fazla restart tetiklenirse yalnızca ilki geçer
    let mut last_restart_at = Instant::now() - Duration::from_secs(10);

    // Arc'ları önceden klonla — loop içinde lock almadan atomik oku
    let restart_trigger  = state.lock().unwrap().loop_restart_trigger.clone();
    let backtest_running = state.lock().unwrap().backtest_running.clone();
    // app_stop_signal: loop geçişlerinde değişmez, yalnızca 'q' ile true olur.
    // stop_signal: loop'a özgü, geçişlerde değişir — quit için kullanılamaz.
    let quit_signal      = state.lock().unwrap().app_stop_signal.clone();
    let init_complete    = state.lock().unwrap().init_complete.clone();

    loop {
        // Lock'u draw öncesinde al — en fazla 20ms bekle, yoksa bu frame'i atla.
        // Bu sayede robotic loop / worker lock'u tutarken TUI donmaz.
        let guard = {
            let mut g = None;
            let deadline = Instant::now() + Duration::from_millis(20);
            loop {
                match state.try_lock() {
                    Ok(guard) => { g = Some(guard); break; }
                    Err(_) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(_) => break, // zaman aşımı — bu frame'i atla
                }
            }
            g
        };

        // Tab başlıkları her frame'de render edilir (lock bağımsız)
        let mtf_alert_cnt = guard.as_ref()
            .map(|st: &std::sync::MutexGuard<AppState>| {
                st.mtf_opportunities.iter().filter(|o| o.live_signal != "-").count()
            })
            .unwrap_or(0);
        let render_tabs = |f: &mut ratatui::Frame, active: usize| {
            let full = f.size();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(3),
                ])
                .split(full);
            // Her sekme: ikon + kısa ad. Aktif sekme REVERSED ile vurgulanır.
        let htf_tab_label = if mtf_alert_cnt > 0 {
            format!(" [6]⏱HTF🚨{} ", mtf_alert_cnt)
        } else {
            " [6]⏱HTF ".to_string()
        };
        let titles: Vec<Line> = vec![
                Line::from(" [1]📊Dash "),
                Line::from(" [2]🧬AI "),
                Line::from(" [3]📋Log "),
                Line::from(" [4]💼Poz "),
                Line::from(" [5]💹Fiy "),
                Line::from(htf_tab_label),
                Line::from(" [7]📈Graf "),
                Line::from(" [8]🔗Pipe "),
                Line::from(" [9]⚡SCP/SWG "),
            ];
            let tabs = Tabs::new(titles)
                .select(active)
                .block(
                    Block::default()
                        .title(Line::from(vec![
                            ratatui::text::Span::styled(
                                " ⚡ Memos RTC ",
                                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                            ),
                            ratatui::text::Span::styled(
                                "— Otonom Trading Konsolu ",
                                Style::default().fg(Color::LightBlue),
                            ),
                        ]))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .highlight_style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
                .divider(ratatui::text::Span::styled(
                    "│",
                    Style::default().fg(Color::Blue),
                ));
            f.render_widget(tabs, chunks[0]);
            chunks
        };

        let mut did_render = false;
        if let Some(st) = guard {
        // Lock alındığında cache'i her zaman güncelle (loading ekranı da kullanır)
        last_worker_info = (st.orchestrator.worker_count(), st.orchestrator.max_workers);
        // Worker'lar aktifse init_complete'i kalıcı olarak işaretle
        if last_worker_info.0 > 0 {
            init_complete.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        // ready: init_complete VEYA worker aktif. ever_ready bir kez true olduktan sonra asla false olmaz.
        let ready = init_complete.load(std::sync::atomic::Ordering::Relaxed) || last_worker_info.0 > 0;
        if ready { ever_ready = true; }
        last_symbol = format!("{}/{} {}",
            st.active_symbol.symbol,
            st.active_symbol.market,
            st.active_symbol.interval,
        );
        last_workers = st.orchestrator.worker_status().into_iter()
            .map(|w| (w.symbol, w.market, w.interval, w.paused, w.uptime_secs))
            .collect();
        last_download_active = st.download_active;
        if st.last_backtest.is_some() { last_backtest_done = true; }
        last_download_progress = st.download_progress.clone();
        if let Some(ref dl) = st.last_download { last_download_summary = dl.clone(); }
        last_log_tail = st.log.iter().rev().take(5).cloned().collect::<Vec<_>>();
        last_log_tail.reverse();

        if ready {
        terminal.draw(|f| {
            // Global zemin: siyah bg + parlak beyaz fg — tüm widget'lar bunu miras alır
            f.render_widget(
                ratatui::widgets::Block::default()
                    .style(Style::default().fg(Color::White).bg(Color::Black).add_modifier(Modifier::BOLD)),
                f.size(),
            );
            let chunks = render_tabs(f, active_tab);
            let full = f.size();

            // Seçili tab içeriği
            match active_tab {
                0 => draw_dashboard(f, chunks[1], &st),
                1 => draw_ai_combined(f, chunks[1], &st),
                2 => draw_logs(f, chunks[1], &st, log_scroll),
                3 => draw_positions(f, chunks[1], &st, trades_scroll),
                4 => draw_live_prices(f, chunks[1], &st),
                5 => draw_htf_intervals(f, chunks[1], &st, htf_scroll),
                6 => draw_charts(f, chunks[1], &st),
                7 => draw_pipeline(f, chunks[1], &st),
                8 => draw_scalp_swing_tab(f, chunks[1], &st),
                _ => {}
            }

            // Yardım çubuğu
            draw_help(f, chunks[2]);

            // Ayarlar overlay
            if settings_open {
                draw_settings_overlay(f, full, settings_idx, &st);
            }

            // İndirme / Backtest durumu overlay — [p] ile aç/kapat
            if show_progress {
                let spinner_s = ["⠋","⠙","⠹","⠸","⠼","⠴","⠦","⠧","⠇","⠏"][
                    (loading_frame as usize) % 10
                ];
                let bt_run = backtest_running.load(Ordering::Relaxed);
                let (aktif, toplam) = last_worker_info;
                let tab_name = ["Dashboard","AI Merkezi","Günlük","Pozisyonlar","Canlı Fiyatlar","Interval Türevleri","Grafikler","Pipeline","Scalp/Swing"]
                    .get(active_tab).copied().unwrap_or("");
                let init_msg = if last_download_active {
                    format!(" {} İndirme devam ediyor", spinner_s)
                } else if bt_run {
                    format!(" {} Backtest / skor hesaplama devam ediyor", spinner_s)
                } else if last_backtest_done {
                    format!(" ✓ İndirme & Backtest tamamlandı")
                } else {
                    format!(" ⏳ İndirme tamamlandı — Backtest henüz çalışmadı")
                };
                let mut plines: Vec<Line<'static>> = vec![
                    Line::from(Span::styled(init_msg, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("  Worker  : ", Style::default().fg(Color::White)),
                        Span::styled(format!("{}/{} aktif", aktif, toplam), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                        Span::raw("   "),
                        Span::styled("Sembol: ", Style::default().fg(Color::White)),
                        Span::styled(last_symbol.clone(), Style::default().fg(Color::LightGreen)),
                        Span::raw("   Tab: "),
                        Span::styled(tab_name.to_string(), Style::default().fg(Color::White)),
                    ]),
                    Line::from(vec![
                        Span::styled("  İndirme : ", Style::default().fg(Color::White)),
                        if last_download_active {
                            Span::styled("⬇ Aktif", Style::default().fg(Color::LightBlue).add_modifier(Modifier::BOLD))
                        } else {
                            Span::styled("✓ Tamamlandı", Style::default().fg(Color::LightGreen))
                        },
                        Span::raw("   "),
                        Span::styled("Backtest: ", Style::default().fg(Color::White)),
                        if bt_run {
                            Span::styled("🔄 Çalışıyor", Style::default().fg(Color::LightYellow).add_modifier(Modifier::BOLD))
                        } else if last_backtest_done {
                            Span::styled("✓ Tamamlandı", Style::default().fg(Color::LightGreen))
                        } else {
                            Span::styled("⏳ Bekliyor", Style::default().fg(Color::White))
                        },
                    ]),
                ];
                // Download detayı — aktif indirme varsa adım+liste, tamamlandıysa özet göster
                if last_download_progress.is_none() && !last_download_summary.is_empty() {
                    plines.push(Line::from(""));
                    plines.push(Line::from(vec![
                        Span::styled("  Özet : ", Style::default().fg(Color::White)),
                        Span::styled(last_download_summary.clone(), Style::default().fg(Color::LightGreen)),
                    ]));
                }
                if let Some(ref dp) = last_download_progress {
                    let now_ms_p = chrono::Utc::now().timestamp_millis();
                    let elapsed  = (now_ms_p - dp.session_start_ms) / 1000;
                    let el_str   = if elapsed < 60 { format!("{}sn", elapsed) } else { format!("{}dk {}sn", elapsed/60, elapsed%60) };
                    let bar_total = 18usize;
                    let bar_done  = if dp.total_targets > 0 {
                        (dp.current_idx.saturating_sub(1) * bar_total / dp.total_targets).min(bar_total)
                    } else { 0 };
                    let bar = format!("[{}{}]", "█".repeat(bar_done), "░".repeat(bar_total - bar_done));
                    plines.push(Line::from(""));
                    plines.push(Line::from(vec![
                        Span::styled("  Adım : ", Style::default().fg(Color::White)),
                        Span::styled(format!("{}/{}", dp.current_idx, dp.total_targets), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                        Span::raw("  "),
                        Span::styled(bar, Style::default().fg(Color::LightBlue)),
                        Span::styled(format!("  Geçen: {}", el_str), Style::default().fg(Color::White)),
                    ]));
                    for (i, (mkt, sym, intv, durum, _, _)) in dp.target_labels.iter().enumerate() {
                        let is_cur = i + 1 == dp.current_idx;
                        let mkt_s = if mkt == "futures" { "fut " } else { "spot" };
                        let (pfx, sc, dc) = if durum.starts_with('✓') {
                            ("  ", Color::LightBlue, Color::LightGreen)
                        } else if durum.starts_with('⬇') {
                            ("▶ ", Color::White, Color::LightBlue)
                        } else if durum.starts_with('✗') {
                            ("  ", Color::LightBlue, Color::LightRed)
                        } else {
                            ("  ", Color::LightBlue, Color::LightBlue)
                        };
                        let _ = is_cur;
                        plines.push(Line::from(vec![
                            Span::styled(format!("  {}{}", pfx, mkt_s), Style::default().fg(sc)),
                            Span::styled(format!("{:<10}", sym), Style::default().fg(sc)),
                            Span::styled(format!(" {:3}  ", intv), Style::default().fg(Color::White)),
                            Span::styled(durum.clone(), Style::default().fg(dc)),
                        ]));
                    }
                }
                // Worker listesi
                plines.push(Line::from(""));
                plines.push(Line::from(Span::styled("  Aktif Worker'lar:", Style::default().fg(Color::Yellow))));
                if last_workers.is_empty() {
                    plines.push(Line::from(Span::styled("  (henüz başlatılmadı)", Style::default().fg(Color::White))));
                } else {
                    for (sym, mkt, intv, paused, uptime) in &last_workers {
                        let h = uptime/3600; let m = (uptime%3600)/60; let s = uptime%60;
                        let mkt_s = if mkt == "futures" { "fut " } else { "spot" };
                        let (icon, durum, color) = if *paused {
                            ("⏸", "Duraklattı", Color::LightBlue)
                        } else {
                            ("●", "Çalışıyor ", Color::LightGreen)
                        };
                        plines.push(Line::from(vec![
                            Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                            Span::styled(format!("{:<10}", sym), Style::default().fg(Color::White)),
                            Span::styled(format!(" {} ", mkt_s), Style::default().fg(Color::White)),
                            Span::styled(format!("{:<3}", intv), Style::default().fg(Color::Cyan)),
                            Span::styled(format!("  {:02}:{:02}:{:02}  ", h, m, s), Style::default().fg(Color::White)),
                            Span::styled(durum.to_string(), Style::default().fg(color)),
                        ]));
                    }
                }
                // Son loglar
                if !last_log_tail.is_empty() {
                    plines.push(Line::from(Span::styled("  Son loglar:", Style::default().fg(Color::Yellow))));
                    for log in &last_log_tail {
                        let trimmed: String = log.chars().take(105).collect();
                        plines.push(Line::from(Span::styled(format!("   {}", trimmed), Style::default().fg(Color::White))));
                    }
                }
                loading_max_scroll = plines.len().saturating_sub(1);
                loading_scroll = loading_scroll.min(loading_max_scroll);
                let popup = centered_rect(82, 88, full);
                f.render_widget(Clear, popup);
                let para = Paragraph::new(plines)
                    .scroll((loading_scroll as u16, 0))
                    .block(Block::default().borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::LightBlue))
                        .title(" 📊 İndirme & Backtest Durumu ")
                        .title_bottom(Line::from(Span::styled(
                            " [l/Esc] Kapat  ↑↓ Kaydır ",
                            Style::default().fg(Color::White),
                        ))));
                f.render_widget(para, popup);
            }
        })?;
        did_render = true;
        loading_frame = loading_frame.wrapping_add(1); // overlay spinner için
        } // if ready
        drop(st);
        }
        if !did_render {
            // Download aktifken loading ekranı her frame'de yenilenir (Geçen sayacı donar değil).
            // Download bitmişse ever_ready=true ise eski içerik 150ms kalabilir.
            if ever_ready && !last_download_active {
                // bu frame'i atla
            } else {
            // Henüz hiç dashboard açılmadı — loading ekranını göster
            loading_frame = loading_frame.wrapping_add(1);
            let spinner = ["⠋","⠙","⠹","⠸","⠼","⠴","⠦","⠧","⠇","⠏"][
                (loading_frame as usize) % 10
            ];
            let (aktif, toplam) = last_worker_info;
            let tab_name = ["Dashboard","AI Merkezi","Günlük","Pozisyonlar","Canlı Fiyatlar","Interval Türevleri","Grafikler","Pipeline","Scalp/Swing"]
                .get(active_tab).unwrap_or(&"");
            let bt_running = backtest_running.load(Ordering::Relaxed);

            let init_msg = if last_download_active {
                format!(" {} İndirme devam ediyor — tamamlanınca dashboard açılacak...", spinner)
            } else if backtest_running.load(Ordering::Relaxed) {
                format!(" {} Backtest / skor hesaplama devam ediyor...", spinner)
            } else {
                format!(" {} Başlangıç işlemleri tamamlanıyor...", spinner)
            };

            let mut lines: Vec<Line> = vec![
                Line::from(Span::styled(
                    init_msg,
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                // ── Genel durum ──
                Line::from(vec![
                    Span::styled("  Worker    : ", Style::default().fg(Color::White)),
                    Span::styled(format!("{} / {} aktif", aktif, toplam), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::raw("   "),
                    Span::styled("Sembol: ", Style::default().fg(Color::White)),
                    Span::styled(last_symbol.clone(), Style::default().fg(Color::LightGreen)),
                    Span::raw("   "),
                    Span::styled("Tab: ", Style::default().fg(Color::White)),
                    Span::styled(tab_name.to_string(), Style::default().fg(Color::White)),
                ]),
                Line::from(vec![
                    Span::styled("  İndirme   : ", Style::default().fg(Color::White)),
                    if last_download_active {
                        Span::styled("⬇ Aktif", Style::default().fg(Color::LightBlue).add_modifier(Modifier::BOLD))
                    } else if last_download_progress.is_some() {
                        Span::styled("✓ Tamamlandı", Style::default().fg(Color::LightGreen))
                    } else {
                        Span::styled("⏳ Bekliyor", Style::default().fg(Color::White))
                    },
                    Span::raw("   "),
                    Span::styled("Backtest   : ", Style::default().fg(Color::White)),
                    if bt_running {
                        Span::styled("🔄 Çalışıyor", Style::default().fg(Color::LightYellow).add_modifier(Modifier::BOLD))
                    } else if last_backtest_done {
                        Span::styled("✓ Tamamlandı", Style::default().fg(Color::LightGreen))
                    } else {
                        Span::styled("⏳ Bekliyor", Style::default().fg(Color::White))
                    },
                ]),
            ];

            // ── Download ilerleme detayı ─────────────────────────────────────────
            if let Some(ref dp) = last_download_progress {
                let now_ms_disp  = chrono::Utc::now().timestamp_millis();
                let elapsed_secs = ((now_ms_disp - dp.session_start_ms) / 1000).max(1);
                let elapsed_str  = if elapsed_secs < 60 {
                    format!("{}sn", elapsed_secs)
                } else {
                    format!("{}dk {}sn", elapsed_secs / 60, elapsed_secs % 60)
                };

                // Seans başlangıç saati
                let start_local = chrono::DateTime::from_timestamp_millis(dp.session_start_ms)
                    .map(|d: chrono::DateTime<chrono::Utc>| {
                        d.with_timezone(&chrono::Local).format("%H:%M:%S").to_string()
                    })
                    .unwrap_or_else(|| "?".to_string());

                // Özet sayaçlar
                let done_cnt    = dp.target_labels.iter().filter(|l| l.3.starts_with('✓')).count();
                let err_cnt     = dp.target_labels.iter().filter(|l| l.3.starts_with('✗')).count();
                let pending_cnt = dp.target_labels.iter().filter(|l| l.3.starts_with('⏳')).count();
                let active_cnt  = dp.target_labels.iter().filter(|l| l.3.starts_with('⬇')).count();

                // Hız: mum/dk
                let speed_min = dp.inserted_session as f64 / (elapsed_secs as f64 / 60.0);
                let speed_str = if speed_min >= 1.0 {
                    format!("{:.0} mum/dk", speed_min)
                } else {
                    format!("{:.1} mum/sn", dp.inserted_session as f64 / elapsed_secs as f64)
                };

                // Seans geneli ETA: mevcut hıza göre kalan hedefler
                let remaining_targets = dp.total_targets.saturating_sub(done_cnt + err_cnt);
                let session_eta_str = if done_cnt > 0 && elapsed_secs > 0 {
                    let secs_per_target = elapsed_secs as f64 / done_cnt.max(1) as f64;
                    let eta = (remaining_targets as f64 * secs_per_target) as i64;
                    if eta < 60 { format!("~{}sn", eta) }
                    else if eta < 3600 { format!("~{}dk", eta / 60) }
                    else { format!("~{}sa {}dk", eta/3600, (eta%3600)/60) }
                } else { "hesaplanıyor".to_string() };

                // Genel ilerleme çubuğu
                let bar_total = 20usize;
                let bar_done  = if dp.total_targets > 0 {
                    (done_cnt * bar_total / dp.total_targets).min(bar_total)
                } else { 0 };
                let bar = format!("[{}{}]", "█".repeat(bar_done), "░".repeat(bar_total - bar_done));

                lines.push(Line::from(vec![
                    Span::styled("  ─── İndirme Detayı ", Style::default().fg(Color::LightBlue)),
                    Span::styled("─────────────────────────────────────────────────────────", Style::default().fg(Color::White)),
                ]));

                // Satır 1: adım + genel progress bar + ETA
                lines.push(Line::from(vec![
                    Span::styled("  Adım   : ", Style::default().fg(Color::White)),
                    Span::styled(
                        format!("{}/{}", dp.current_idx, dp.total_targets),
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(bar, Style::default().fg(Color::LightBlue)),
                    Span::styled(format!("  Kalan: {}", session_eta_str), Style::default().fg(Color::Cyan)),
                ]));

                // Satır 2: özet sayaçlar + hız + geçen süre
                lines.push(Line::from(vec![
                    Span::styled("  Durum  : ", Style::default().fg(Color::White)),
                    Span::styled(format!("✓ {} tamam", done_cnt), Style::default().fg(Color::LightGreen)),
                    Span::styled("  |  ", Style::default().fg(Color::White)),
                    Span::styled(format!("⬇ {} aktif", active_cnt), Style::default().fg(Color::LightBlue)),
                    Span::styled("  |  ", Style::default().fg(Color::White)),
                    Span::styled(format!("⏳ {} bekliyor", pending_cnt), Style::default().fg(Color::White)),
                    if err_cnt > 0 {
                        Span::styled(format!("  |  ✗ {} hata", err_cnt), Style::default().fg(Color::LightRed))
                    } else {
                        Span::raw("")
                    },
                ]));

                // Satır 3: hız + toplam eklenen + türetilen + başlangıç
                lines.push(Line::from(vec![
                    Span::styled("  Hız    : ", Style::default().fg(Color::White)),
                    Span::styled(speed_str, Style::default().fg(Color::Yellow)),
                    Span::styled("   Eklenen: ", Style::default().fg(Color::White)),
                    Span::styled(
                        format!("+{}", dp.inserted_session),
                        Style::default().fg(if dp.inserted_session > 0 { Color::LightGreen } else { Color::LightBlue }),
                    ),
                    if dp.derived_session > 0 {
                        Span::styled(format!("  HTF+{}", dp.derived_session), Style::default().fg(Color::Magenta))
                    } else {
                        Span::raw("")
                    },
                    Span::styled(format!("   Geçen: {}  Başlangıç: {}", elapsed_str, start_local), Style::default().fg(Color::White)),
                ]));

                lines.push(Line::from(Span::styled(
                    "  ─────────────────────────────────────────────────────────────────────",
                    Style::default().fg(Color::White),
                )));

                // Hedef listesi
                for (i, (mkt, sym, intv, durum, gap_initial, inserted)) in dp.target_labels.iter().enumerate() {
                    let is_current = i + 1 == dp.current_idx;
                    let mkt_s = if mkt == "futures" { "fut" } else { "spt" };
                    let (prefix, sym_color, dur_color, row_mod) = if durum.starts_with('✓') {
                        ("  ", Color::LightBlue, Color::LightGreen, Modifier::empty())
                    } else if durum.starts_with('⬇') {
                        ("▶ ", Color::White, Color::LightBlue, Modifier::BOLD)
                    } else if durum.starts_with('✗') {
                        ("  ", Color::LightRed, Color::LightRed, Modifier::empty())
                    } else {
                        ("  ", Color::LightBlue, Color::LightBlue, Modifier::empty())
                    };

                    // Per-row detay
                    let detail: Vec<Span> = if is_current && dp.gap_start_ms.is_some() {
                        let iv         = dp.gap_interval_ms.max(1);
                        let remaining  = ((dp.gap_end_ms - dp.gap_start_ms.unwrap_or(0)) / iv).max(0);
                        let initial    = dp.gap_initial_candles.unwrap_or(remaining).max(1);
                        let downloaded = (initial - remaining).max(0);
                        let pct        = (downloaded * 100 / initial).min(100);
                        let bt         = 10usize;
                        let bd         = (pct as usize * bt / 100).min(bt);
                        let b          = format!("[{}{}]", "█".repeat(bd), "░".repeat(bt - bd));
                        // Aktif batch ETA (bu hedef için)
                        let tgt_eta = if downloaded > 0 && elapsed_secs > 0 {
                            let r = downloaded as f64 / elapsed_secs as f64;
                            if r > 0.0 {
                                let e = (remaining as f64 / r) as i64;
                                if e < 60 { format!(" ~{}sn", e) }
                                else { format!(" ~{}dk", e/60) }
                            } else { String::new() }
                        } else { String::new() };
                        vec![
                            Span::styled(format!(" {} {}%", b, pct), Style::default().fg(Color::LightYellow)),
                            Span::styled(format!(" {}/{} mum", downloaded, initial), Style::default().fg(Color::White)),
                            Span::styled(format!(" istek#{}", dp.batch_no), Style::default().fg(Color::White)),
                            Span::styled(tgt_eta, Style::default().fg(Color::Cyan)),
                        ]
                    } else if durum.starts_with('✓') && *inserted > 0 {
                        let pct_str = if *gap_initial > 0 {
                            format!(" (%{})", ((*inserted as i64) * 100 / (*gap_initial).max(1)).min(100).max(0))
                        } else { String::new() };
                        vec![
                            Span::styled(format!(" +{} mum{}", inserted, pct_str), Style::default().fg(Color::LightGreen)),
                        ]
                    } else { vec![] };

                    let mut spans = vec![
                        Span::styled(format!("  {}", prefix), Style::default().fg(Color::White)),
                        Span::styled(format!("{:>2}. ", i + 1), Style::default().fg(Color::White)),
                        Span::styled(format!("{} ", mkt_s), Style::default().fg(Color::White)),
                        Span::styled(format!("{:<10}", sym), Style::default().fg(sym_color).add_modifier(row_mod)),
                        Span::styled(format!(" {:<4}", intv), Style::default().fg(Color::White)),
                        Span::styled(durum.clone(), Style::default().fg(dur_color).add_modifier(row_mod)),
                    ];
                    spans.extend(detail);
                    lines.push(Line::from(spans));
                }

                // Gap-fill detay (aktif hedef)
                if let Some(gap_start) = dp.gap_start_ms {
                    let iv             = dp.gap_interval_ms.max(1);
                    let gap_remaining  = ((dp.gap_end_ms - gap_start) / iv).max(0);
                    let gap_initial    = dp.gap_initial_candles.unwrap_or(gap_remaining).max(1);
                    let gap_downloaded = (gap_initial - gap_remaining).max(0);
                    let gap_pct        = (gap_downloaded * 100 / gap_initial).min(100);
                    let gbt = 20usize;
                    let gbd = (gap_pct as usize * gbt / 100).min(gbt);
                    let gbar = format!("[{}{}] {}%", "█".repeat(gbd), "░".repeat(gbt - gbd), gap_pct);

                    let eta_str = if gap_downloaded > 0 && elapsed_secs > 0 {
                        let rate = gap_downloaded as f64 / elapsed_secs as f64;
                        if rate > 0.0 {
                            let e = (gap_remaining as f64 / rate) as i64;
                            if e < 60 { format!("~{}sn kaldı", e) }
                            else if e < 3600 { format!("~{}dk kaldı", e / 60) }
                            else { format!("~{}sa {}dk kaldı", e/3600, (e%3600)/60) }
                        } else { "hesaplanıyor...".to_string() }
                    } else { "hesaplanıyor...".to_string() };

                    let start_dt = chrono::DateTime::from_timestamp_millis(gap_start)
                        .map(|d: chrono::DateTime<chrono::Utc>| d.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let end_dt = chrono::DateTime::from_timestamp_millis(dp.gap_end_ms)
                        .map(|d: chrono::DateTime<chrono::Utc>| d.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "?".to_string());

                    lines.push(Line::from(vec![
                        Span::styled("  Gap    : ", Style::default().fg(Color::White)),
                        Span::styled("⚡ Boşluk doldurma  ", Style::default().fg(Color::LightYellow).add_modifier(Modifier::BOLD)),
                        Span::styled(gbar, Style::default().fg(Color::LightYellow)),
                        Span::styled(format!("  {}", eta_str), Style::default().fg(Color::Cyan)),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled("         ", Style::default().fg(Color::White)),
                        Span::styled(format!("İndirilen: {:>7} mum", gap_downloaded), Style::default().fg(Color::LightGreen)),
                        Span::styled("  |  ", Style::default().fg(Color::White)),
                        Span::styled(format!("Kalan: {:>7} mum", gap_remaining), Style::default().fg(Color::LightRed)),
                        Span::styled(format!("  |  {}/istek", dp.dl_limit), Style::default().fg(Color::White)),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled("         ", Style::default().fg(Color::White)),
                        Span::styled(start_dt, Style::default().fg(Color::White)),
                        Span::styled(" → ", Style::default().fg(Color::White)),
                        Span::styled(end_dt, Style::default().fg(Color::White)),
                        Span::styled(format!("  ({})", dp.interval), Style::default().fg(Color::Cyan)),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled("  Mod    : ", Style::default().fg(Color::White)),
                        Span::styled(format!("Normal — son {} mum/hedef", dp.dl_limit), Style::default().fg(Color::White)),
                    ]));
                }
            }

            lines.push(Line::from(Span::styled(
                "  ─────────────────────────────────────────────────────────────────────",
                Style::default().fg(Color::White),
            )));
            lines.push(Line::from(Span::styled(
                "  Aktif Worker'lar:",
                Style::default().fg(Color::Yellow),
            )));

            // Worker satırları
            if last_workers.is_empty() {
                lines.push(Line::from(Span::styled(
                    "    (henüz worker bilgisi yok — ilk lock bekleniyor)",
                    Style::default().fg(Color::White),
                )));
            } else {
                for (sym, mkt, intv, paused, uptime) in &last_workers {
                    let h = uptime/3600; let m = (uptime%3600)/60; let s = uptime%60;
                    let uptime_str = format!("{:02}:{:02}:{:02}", h, m, s);
                    let mkt_s = if mkt == "futures" { "fut " } else { "spot" };
                    let (icon, durum, color) = if *paused {
                        ("⏸", "Duraklattı", Color::LightBlue)
                    } else {
                        ("●", "Çalışıyor ", Color::LightGreen)
                    };
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                        Span::styled(format!("{:<10}", sym),  Style::default().fg(Color::White)),
                        Span::styled(format!(" {} ", mkt_s),  Style::default().fg(Color::White)),
                        Span::styled(format!("{:<3}", intv),   Style::default().fg(Color::Cyan)),
                        Span::styled(format!("  {}  ", uptime_str), Style::default().fg(Color::White)),
                        Span::styled(durum.to_string(), Style::default().fg(color)),
                    ]));
                }
            }

            // Son log satırları
            if !last_log_tail.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  ─────────────────────────────────────────────────────────────────────",
                    Style::default().fg(Color::White),
                )));
                lines.push(Line::from(Span::styled(
                    "  Son loglar:",
                    Style::default().fg(Color::Yellow),
                )));
                for log in &last_log_tail {
                    // Log satırını kısalt — char sınırında kes (UTF-8 güvenli)
                    let trimmed: String = log.chars().take(110).collect();
                    lines.push(Line::from(Span::styled(
                        format!("   {}", trimmed),
                        Style::default().fg(Color::White),
                    )));
                }
            }
            loading_max_scroll = lines.len().saturating_sub(1);
            loading_scroll = loading_scroll.min(loading_max_scroll);
            terminal.draw(|f| {
                f.render_widget(
                    ratatui::widgets::Block::default()
                        .style(Style::default().fg(Color::White).bg(Color::Black).add_modifier(Modifier::BOLD)),
                    f.size(),
                );
                let chunks = render_tabs(f, active_tab);
                let msg = Paragraph::new(lines)
                    .scroll((loading_scroll as u16, 0))
                    .block(Block::default().borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Yellow))
                        .title(" Yükleniyor ")
                        .title_bottom(Line::from(Span::styled(
                            " ↑↓ kaydır ",
                            Style::default().fg(Color::White),
                        ))));
                f.render_widget(msg, chunks[1]);
                draw_help(f, chunks[2]);
            })?;
            } // else (init tamamlanmamış)
        } // if !did_render

        // Olayları işle
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_default();

        if event::poll(timeout)? {
            match event::read()? {
            // ── Mouse scroll → progress overlay, loading ekranı veya Günlük ──
            Event::Mouse(me) if show_progress || !did_render => {
                match me.kind {
                    MouseEventKind::ScrollUp   => { loading_scroll = loading_scroll.saturating_add(3).min(loading_max_scroll); }
                    MouseEventKind::ScrollDown => { loading_scroll = loading_scroll.saturating_sub(3); }
                    _ => {}
                }
            }
            Event::Mouse(me) if active_tab == 2 => {
                match me.kind {
                    MouseEventKind::ScrollUp => {
                        let max = state.try_lock().map(|s| s.log.len().saturating_sub(1)).unwrap_or(usize::MAX);
                        log_scroll = (log_scroll + 3).min(max);
                    }
                    MouseEventKind::ScrollDown => {
                        log_scroll = log_scroll.saturating_sub(3);
                    }
                    _ => {}
                }
            }
            Event::Mouse(me) if active_tab == 3 => {
                match me.kind {
                    MouseEventKind::ScrollUp => {
                        let max = state.try_lock().map(|s| s.live_closed_trades.read().ok().map(|l| l.len()).unwrap_or(0)).unwrap_or(0);
                        trades_scroll = (trades_scroll + 3).min(max.saturating_sub(1));
                    }
                    MouseEventKind::ScrollDown => {
                        trades_scroll = trades_scroll.saturating_sub(3);
                    }
                    _ => {}
                }
            }
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    // ── Loading ekranı aktifken ok tuşları scroll yapar ──────
                    KeyCode::Up | KeyCode::Char('k') if !did_render => {
                        loading_scroll = loading_scroll.saturating_add(1).min(loading_max_scroll);
                    }
                    KeyCode::Down | KeyCode::Char('j') if !did_render => {
                        loading_scroll = loading_scroll.saturating_sub(1);
                    }
                    KeyCode::PageUp if !did_render => {
                        loading_scroll = loading_scroll.saturating_add(10).min(loading_max_scroll);
                    }
                    KeyCode::PageDown if !did_render => {
                        loading_scroll = loading_scroll.saturating_sub(10);
                    }
                    // ── Progress overlay açıkken ↑↓ scroll ve Esc/p kapat ────
                    _ if show_progress => {
                        match key.code {
                            KeyCode::Up   | KeyCode::Char('k') => {
                                loading_scroll = loading_scroll.saturating_add(1).min(loading_max_scroll);
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                loading_scroll = loading_scroll.saturating_sub(1);
                            }
                            KeyCode::PageUp => {
                                loading_scroll = (loading_scroll + 10).min(loading_max_scroll);
                            }
                            KeyCode::PageDown => {
                                loading_scroll = loading_scroll.saturating_sub(10);
                            }
                            KeyCode::Char('l') | KeyCode::Char('L') | KeyCode::Esc => {
                                show_progress = false;
                            }
                            _ => {}
                        }
                    }
                    // ── Ayarlar modu aktifken özel tuş yönetimi ──────────────
                    _ if settings_open => {
                        match key.code {
                            KeyCode::Up   | KeyCode::Char('k') => {
                                if settings_idx > 0 { settings_idx -= 1; }
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                if settings_idx < SETTINGS_LABELS.len() - 1 { settings_idx += 1; }
                            }
                            KeyCode::Right | KeyCode::Char('+') => {
                                let (cfg_clone, payload, do_restart) = {
                                    let mut st = state.lock().unwrap();
                                    let mut do_restart = false;
                                    match settings_idx {
                                        0 => { st.config.trade_amount = (st.config.trade_amount + 0.001).max(0.001); }
                                        1 => { let c = st.config.capital + 100.0; st.set_capital(c); }
                                        2 => {
                                            let pos = INTERVALS.iter().position(|&x| x == st.config.interval).unwrap_or(0);
                                            let new_intv = INTERVALS[(pos + 1) % INTERVALS.len()].to_string();
                                            st.config.interval = new_intv.clone();
                                            st.active_symbol.interval = new_intv;
                                            do_restart = true;
                                        }
                                        3 => { st.config.backtest_every_mins = st.config.backtest_every_mins.saturating_add(1).max(1); }
                                        4 => { st.config.download_every_mins = st.config.download_every_mins.saturating_add(1).max(1); }
                                        5 => {
                                            st.best_sl = (st.best_sl + 0.1).clamp(0.1, 10.0);
                                            let new_sl = st.best_sl;
                                            if let Ok(mut lr) = st.live_risk.write() { lr.global_sl = new_sl; }
                                        }
                                        6 => {
                                            st.best_tp = (st.best_tp + 0.1).clamp(0.1, 20.0);
                                            let new_tp = st.best_tp;
                                            if let Ok(mut lr) = st.live_risk.write() { lr.global_tp = new_tp; }
                                        }
                                        7 => {
                                            // Sembol: symbol_candidates listesinde ileri
                                            let syms: Vec<(String, String)> = st.symbol_candidates.iter()
                                                .map(|c| (c.symbol.clone(), c.market.clone()))
                                                .collect();
                                            if !syms.is_empty() {
                                                let cur_pos = syms.iter().position(|(s, _)| s == &st.config.symbol).unwrap_or(0);
                                                let next_pos = (cur_pos + 1) % syms.len();
                                                let (sym, mkt) = syms[next_pos].clone();
                                                st.config.symbol = sym.clone();
                                                st.config.market = mkt.clone();
                                                st.active_symbol.symbol = sym.clone();
                                                st.active_symbol.market = mkt;
                                                st.auto_symbol = false; // manuel seçim
                                                st.push_log(format!("🔄 Sembol → {} (ayarlar, restart)", sym));
                                                do_restart = true;
                                            }
                                        }
                                        8 => {
                                            // Market: spot ↔ futures
                                            let new_mkt = if st.config.market == "futures" { "spot" } else { "futures" };
                                            st.config.market = new_mkt.to_string();
                                            st.active_symbol.market = new_mkt.to_string();
                                            st.push_log(format!("🔄 Market → {} (restart)", new_mkt));
                                            do_restart = true;
                                        }
                                        9 => { st.config.download_top_n = (st.config.download_top_n + 1).min(20); }
                                        10 => {
                                            st.auto_interval = !st.auto_interval;
                                            st.config.auto_interval = st.auto_interval;
                                            let msg = if st.auto_interval {
                                                "✅ Oto-Interval AÇIK — backtest sonrası en iyi interval otomatik seçilir"
                                            } else {
                                                "⏹ Oto-Interval KAPALI — interval manuel"
                                            };
                                            st.push_log(msg.to_string());
                                        }
                                        11 => {
                                            let (_new_state, msg) = if let Ok(mut lrm) = st.live_risk.write() {
                                                lrm.htf_filter_enabled = !lrm.htf_filter_enabled;
                                                (lrm.htf_filter_enabled,
                                                 if lrm.htf_filter_enabled {
                                                     "✅ HTF Filtre AÇIK — büyük resim trende zıt girişler engellenir"
                                                 } else {
                                                     "⏹ HTF Filtre KAPALI — LTF sinyalleri doğrudan geçer"
                                                 })
                                            } else { (true, "") };
                                            if !msg.is_empty() { st.push_log(msg.to_string()); }
                                        }
                                        12 => {
                                            let msg12 = if let Ok(mut lrm) = st.live_risk.write() {
                                                lrm.base_leverage = (lrm.base_leverage + 0.5).min(lrm.max_leverage - 0.5).min(9.5);
                                                format!("⚡ Base Kaldıraç → {:.1}x", lrm.base_leverage)
                                            } else { String::new() };
                                            let new_base = st.live_risk.read().ok().map(|lrm| lrm.base_leverage);
                                            if let Some(v) = new_base { st.config.leverage_base = v; }
                                            if !msg12.is_empty() { st.push_log(msg12); }
                                        }
                                        13 => {
                                            let msg13 = if let Ok(mut lrm) = st.live_risk.write() {
                                                lrm.max_leverage = (lrm.max_leverage + 0.5).min(20.0);
                                                format!("⚡ Max Kaldıraç → {:.1}x", lrm.max_leverage)
                                            } else { String::new() };
                                            let new_max = st.live_risk.read().ok().map(|lrm| lrm.max_leverage);
                                            if let Some(v) = new_max { st.config.leverage_max = v; }
                                            if !msg13.is_empty() { st.push_log(msg13); }
                                        }
                                        14 => {
                                            st.pos_sl_cooldown = (st.pos_sl_cooldown + 60).min(3600);
                                            let v14 = st.pos_sl_cooldown;
                                            st.push_log(format!("⏸ SL Cooldown → {} sn", v14));
                                        }
                                        15 => {
                                            let v = st.pos_breakeven_at_rr.unwrap_or(0.4);
                                            let nv = (v + 0.1).min(2.0);
                                            st.pos_breakeven_at_rr = Some(nv);
                                            st.push_log(format!("✅ Breakeven → {:.1}R", nv));
                                        }
                                        16 => {
                                            let v = st.pos_atr_trail_mult.unwrap_or(0.5);
                                            let nv = (v + 0.5).min(5.0);
                                            st.pos_atr_trail_mult = Some(nv);
                                            st.push_log(format!("📈 ATR Trail → {:.1}x", nv));
                                        }
                                        17 => {
                                            let v = st.pos_partial_tp_ratio.unwrap_or(0.4);
                                            let nv = (v + 0.1).min(0.9);
                                            st.pos_partial_tp_ratio = Some(nv);
                                            st.push_log(format!("🎯 Kısmi TP → %{:.0}", nv * 100.0));
                                        }
                                        18 => {
                                            st.max_daily_trades = st.max_daily_trades.saturating_add(1).min(200);
                                            let msg = if st.max_daily_trades == 0 { "♾ Günlük limit SINIRSIS".into() } else { format!("📊 Günlük limit → {} işlem", st.max_daily_trades) };
                                            st.push_log(msg);
                                        }
                                        // ── Adaptif Korumalar (→ = artır/aç) ──────────────────────
                                        19 => {
                                            st.adaptive_params.short_htf_block = !st.adaptive_params.short_htf_block;
                                            let v = st.adaptive_params.short_htf_block;
                                            st.push_log(format!("🛡 SHORT HTF Blok → {}", if v { "AÇIK" } else { "KAPALI" }));
                                            st.save_adaptive_params();
                                        }
                                        20 => {
                                            st.adaptive_params.long_htf_block = !st.adaptive_params.long_htf_block;
                                            let v = st.adaptive_params.long_htf_block;
                                            st.push_log(format!("🛡 LONG HTF Blok → {}", if v { "AÇIK" } else { "KAPALI" }));
                                            st.save_adaptive_params();
                                        }
                                        21 => {
                                            let nv = (st.adaptive_params.tp_atr_multiplier + 0.1).min(6.0);
                                            st.adaptive_params.tp_atr_multiplier = nv;
                                            st.push_log(format!("🎯 TP ATR Çarpanı → {:.1}x", nv));
                                            st.save_adaptive_params();
                                        }
                                        22 => {
                                            let nv = (st.adaptive_params.sl_atr_multiplier + 0.25).min(3.0);
                                            st.adaptive_params.sl_atr_multiplier = (nv * 100.0).round() / 100.0;
                                            st.push_log(format!("🛑 SL ATR Çarpanı → {:.2}x", nv));
                                            st.save_adaptive_params();
                                        }
                                        23 => {
                                            let nv = (st.adaptive_params.trailing_sl_activation_pct + 0.1).min(5.0);
                                            st.adaptive_params.trailing_sl_activation_pct = nv;
                                            st.push_log(format!("📈 TSL Aktivasyon → {:.1}%", nv));
                                            st.save_adaptive_params();
                                        }
                                        24 => {
                                            let nv = (st.adaptive_params.max_daily_sl_per_symbol + 1).min(10);
                                            st.adaptive_params.max_daily_sl_per_symbol = nv;
                                            st.push_log(format!("🚫 Gün SL Limiti → {}/gün/sembol", nv));
                                            st.save_adaptive_params();
                                        }
                                        25 => {
                                            let nv = (st.adaptive_params.max_consecutive_losses + 1).min(20);
                                            st.adaptive_params.max_consecutive_losses = nv;
                                            st.push_log(format!("🚫 Max Kayıp Serisi → {}", nv));
                                            st.save_adaptive_params();
                                        }
                                        26 => {
                                            let nv = (st.adaptive_params.futures_short_min_conf + 0.05).min(0.90);
                                            st.adaptive_params.futures_short_min_conf = (nv * 100.0).round() / 100.0;
                                            st.push_log(format!("🤖 SHORT ML Eşiği → {:.2}", nv));
                                            st.save_adaptive_params();
                                        }
                                        27 => {
                                            let nv = (st.adaptive_params.adjust_every_n_trades + 5).min(100);
                                            st.adaptive_params.adjust_every_n_trades = nv;
                                            let msg = if nv == 0 { "⏹ Otonom Mod KAPALI".into() } else { format!("🤖 Otonom Mod → her {} işlem", nv) };
                                            st.push_log(msg);
                                            st.save_adaptive_params();
                                        }
                                        28 => {
                                            // ── Otonom Optimize: istatistiklere göre parametreleri hesapla & uygula ──
                                            auto_tune_adaptive_params(&mut st);
                                        }
                                        _ => {}
                                    }
                                    if do_restart {
                                        st.loop_restart_trigger.store(true, Ordering::Relaxed);
                                    }
                                    let profile_path = st.config_paths.robotic_profiles.clone();
                                    save_profile_config(&profile_path, &st);
                                    (st.config.clone(), collect_snapshot_payload(&st), do_restart)
                                }; // lock serbest
                                save_oto_config(&cfg_clone);
                                flush_snapshot_payload(payload);
                                let _ = do_restart;
                            }
                            KeyCode::Left | KeyCode::Char('-') => {
                                let (cfg_clone, payload, do_restart) = {
                                    let mut st = state.lock().unwrap();
                                    let mut do_restart = false;
                                    match settings_idx {
                                        0 => { st.config.trade_amount = (st.config.trade_amount - 0.001).max(0.001); }
                                        1 => { let c = st.config.capital - 100.0; st.set_capital(c); }
                                        2 => {
                                            let pos = INTERVALS.iter().position(|&x| x == st.config.interval).unwrap_or(0);
                                            let new_intv = INTERVALS[(pos + INTERVALS.len() - 1) % INTERVALS.len()].to_string();
                                            st.config.interval = new_intv.clone();
                                            st.active_symbol.interval = new_intv;
                                            do_restart = true;
                                        }
                                        3 => { st.config.backtest_every_mins = st.config.backtest_every_mins.saturating_sub(1).max(1); }
                                        4 => { st.config.download_every_mins = st.config.download_every_mins.saturating_sub(1).max(1); }
                                        5 => {
                                            st.best_sl = (st.best_sl - 0.1).clamp(0.1, 10.0);
                                            let new_sl = st.best_sl;
                                            if let Ok(mut lr) = st.live_risk.write() { lr.global_sl = new_sl; }
                                        }
                                        6 => {
                                            st.best_tp = (st.best_tp - 0.1).clamp(0.1, 20.0);
                                            let new_tp = st.best_tp;
                                            if let Ok(mut lr) = st.live_risk.write() { lr.global_tp = new_tp; }
                                        }
                                        7 => {
                                            let syms: Vec<(String, String)> = st.symbol_candidates.iter()
                                                .map(|c| (c.symbol.clone(), c.market.clone()))
                                                .collect();
                                            if !syms.is_empty() {
                                                let cur_pos = syms.iter().position(|(s, _)| s == &st.config.symbol).unwrap_or(0);
                                                let prev_pos = (cur_pos + syms.len() - 1) % syms.len();
                                                let (sym, mkt) = syms[prev_pos].clone();
                                                st.config.symbol = sym.clone();
                                                st.config.market = mkt.clone();
                                                st.active_symbol.symbol = sym.clone();
                                                st.active_symbol.market = mkt;
                                                st.auto_symbol = false;
                                                st.push_log(format!("🔄 Sembol → {} (ayarlar, restart)", sym));
                                                do_restart = true;
                                            }
                                        }
                                        8 => {
                                            let new_mkt = if st.config.market == "futures" { "spot" } else { "futures" };
                                            st.config.market = new_mkt.to_string();
                                            st.active_symbol.market = new_mkt.to_string();
                                            st.push_log(format!("🔄 Market → {} (restart)", new_mkt));
                                            do_restart = true;
                                        }
                                        9 => { st.config.download_top_n = st.config.download_top_n.saturating_sub(1).max(1); }
                                        10 => {
                                            st.auto_interval = !st.auto_interval;
                                            st.config.auto_interval = st.auto_interval;
                                            let msg = if st.auto_interval { "✅ Oto-Interval AÇIK".to_string() } else { "⏹ Oto-Interval KAPALI".to_string() };
                                            st.push_log(msg);
                                        }
                                        11 => {
                                            let (_, msg) = if let Ok(mut lrm) = st.live_risk.write() {
                                                lrm.htf_filter_enabled = !lrm.htf_filter_enabled;
                                                (lrm.htf_filter_enabled,
                                                 if lrm.htf_filter_enabled {
                                                     "✅ HTF Filtre AÇIK".to_string()
                                                 } else {
                                                     "⏹ HTF Filtre KAPALI".to_string()
                                                 })
                                            } else { (true, String::new()) };
                                            if !msg.is_empty() { st.push_log(msg); }
                                        }
                                        12 => {
                                            let msg12b = if let Ok(mut lrm) = st.live_risk.write() {
                                                lrm.base_leverage = (lrm.base_leverage - 0.5).max(1.0);
                                                format!("⚡ Base Kaldıraç → {:.1}x", lrm.base_leverage)
                                            } else { String::new() };
                                            let new_base = st.live_risk.read().ok().map(|lrm| lrm.base_leverage);
                                            if let Some(v) = new_base { st.config.leverage_base = v; }
                                            if !msg12b.is_empty() { st.push_log(msg12b); }
                                        }
                                        13 => {
                                            let msg13b = if let Ok(mut lrm) = st.live_risk.write() {
                                                lrm.max_leverage = (lrm.max_leverage - 0.5).max(lrm.base_leverage + 0.5).max(1.5);
                                                format!("⚡ Max Kaldıraç → {:.1}x", lrm.max_leverage)
                                            } else { String::new() };
                                            let new_max = st.live_risk.read().ok().map(|lrm| lrm.max_leverage);
                                            if let Some(v) = new_max { st.config.leverage_max = v; }
                                            if !msg13b.is_empty() { st.push_log(msg13b); }
                                        }
                                        14 => {
                                            st.pos_sl_cooldown = st.pos_sl_cooldown.saturating_sub(60);
                                            let v14b = st.pos_sl_cooldown;
                                            st.push_log(format!("⏸ SL Cooldown → {} sn", v14b));
                                        }
                                        15 => {
                                            let msg = match st.pos_breakeven_at_rr {
                                                None => String::new(),
                                                Some(v) if v <= 0.11 => {
                                                    st.pos_breakeven_at_rr = None;
                                                    "⏹ Breakeven KAPALI".into()
                                                }
                                                Some(v) => {
                                                    let nv = (v - 0.1).max(0.1);
                                                    st.pos_breakeven_at_rr = Some(nv);
                                                    format!("✅ Breakeven → {:.1}R", nv)
                                                }
                                            };
                                            if !msg.is_empty() { st.push_log(msg); }
                                        }
                                        16 => {
                                            let msg = match st.pos_atr_trail_mult {
                                                None => String::new(),
                                                Some(v) if v <= 0.51 => {
                                                    st.pos_atr_trail_mult = None;
                                                    "⏹ ATR Trail KAPALI".into()
                                                }
                                                Some(v) => {
                                                    let nv = (v - 0.5).max(0.5);
                                                    st.pos_atr_trail_mult = Some(nv);
                                                    format!("📈 ATR Trail → {:.1}x", nv)
                                                }
                                            };
                                            if !msg.is_empty() { st.push_log(msg); }
                                        }
                                        17 => {
                                            let msg = match st.pos_partial_tp_ratio {
                                                None => String::new(),
                                                Some(v) if v <= 0.11 => {
                                                    st.pos_partial_tp_ratio = None;
                                                    "⏹ Kısmi TP KAPALI".into()
                                                }
                                                Some(v) => {
                                                    let nv = (v - 0.1).max(0.1);
                                                    st.pos_partial_tp_ratio = Some(nv);
                                                    format!("🎯 Kısmi TP → %{:.0}", nv * 100.0)
                                                }
                                            };
                                            if !msg.is_empty() { st.push_log(msg); }
                                        }
                                        18 => {
                                            let msg = if st.max_daily_trades == 0 {
                                                String::new()
                                            } else {
                                                st.max_daily_trades = st.max_daily_trades.saturating_sub(1);
                                                if st.max_daily_trades == 0 {
                                                    "♾ Günlük limit SINIRSIS".into()
                                                } else {
                                                    format!("📊 Günlük limit → {} işlem", st.max_daily_trades)
                                                }
                                            };
                                            if !msg.is_empty() { st.push_log(msg); }
                                        }
                                        // ── Adaptif Korumalar (← = azalt/kapat) ──────────────────
                                        19 => {
                                            st.adaptive_params.short_htf_block = !st.adaptive_params.short_htf_block;
                                            let v = st.adaptive_params.short_htf_block;
                                            st.push_log(format!("🛡 SHORT HTF Blok → {}", if v { "AÇIK" } else { "KAPALI" }));
                                            st.save_adaptive_params();
                                        }
                                        20 => {
                                            st.adaptive_params.long_htf_block = !st.adaptive_params.long_htf_block;
                                            let v = st.adaptive_params.long_htf_block;
                                            st.push_log(format!("🛡 LONG HTF Blok → {}", if v { "AÇIK" } else { "KAPALI" }));
                                            st.save_adaptive_params();
                                        }
                                        21 => {
                                            let nv = (st.adaptive_params.tp_atr_multiplier - 0.1).max(0.5);
                                            st.adaptive_params.tp_atr_multiplier = (nv * 10.0).round() / 10.0;
                                            st.push_log(format!("🎯 TP ATR Çarpanı → {:.1}x", nv));
                                            st.save_adaptive_params();
                                        }
                                        22 => {
                                            let nv = (st.adaptive_params.sl_atr_multiplier - 0.25).max(0.5);
                                            st.adaptive_params.sl_atr_multiplier = (nv * 100.0).round() / 100.0;
                                            st.push_log(format!("🛑 SL ATR Çarpanı → {:.2}x", nv));
                                            st.save_adaptive_params();
                                        }
                                        23 => {
                                            let nv = (st.adaptive_params.trailing_sl_activation_pct - 0.1).max(0.3);
                                            st.adaptive_params.trailing_sl_activation_pct = (nv * 10.0).round() / 10.0;
                                            st.push_log(format!("📈 TSL Aktivasyon → {:.1}%", nv));
                                            st.save_adaptive_params();
                                        }
                                        24 => {
                                            let nv = st.adaptive_params.max_daily_sl_per_symbol.saturating_sub(1);
                                            st.adaptive_params.max_daily_sl_per_symbol = nv;
                                            let msg = if nv == 0 { "⏹ Gün SL Limiti KAPALI".into() } else { format!("🚫 Gün SL Limiti → {}/gün/sembol", nv) };
                                            st.push_log(msg);
                                            st.save_adaptive_params();
                                        }
                                        25 => {
                                            let nv = st.adaptive_params.max_consecutive_losses.saturating_sub(1);
                                            st.adaptive_params.max_consecutive_losses = nv;
                                            let msg = if nv == 0 { "⏹ Max Kayıp Serisi KAPALI".into() } else { format!("🚫 Max Kayıp Serisi → {}", nv) };
                                            st.push_log(msg);
                                            st.save_adaptive_params();
                                        }
                                        26 => {
                                            let nv = (st.adaptive_params.futures_short_min_conf - 0.05).max(0.0);
                                            st.adaptive_params.futures_short_min_conf = (nv * 100.0).round() / 100.0;
                                            let msg = if nv == 0.0 { "⏹ SHORT ML Eşiği KAPALI".into() } else { format!("🤖 SHORT ML Eşiği → {:.2}", nv) };
                                            st.push_log(msg);
                                            st.save_adaptive_params();
                                        }
                                        27 => {
                                            let nv = st.adaptive_params.adjust_every_n_trades.saturating_sub(5);
                                            st.adaptive_params.adjust_every_n_trades = nv;
                                            let msg = if nv == 0 { "⏹ Otonom Mod KAPALI".into() } else { format!("🤖 Otonom Mod → her {} işlem", nv) };
                                            st.push_log(msg);
                                            st.save_adaptive_params();
                                        }
                                        _ => {}
                                    }
                                    if do_restart {
                                        st.loop_restart_trigger.store(true, Ordering::Relaxed);
                                    }
                                    let profile_path = st.config_paths.robotic_profiles.clone();
                                    save_profile_config(&profile_path, &st);
                                    (st.config.clone(), collect_snapshot_payload(&st), do_restart)
                                }; // lock serbest
                                save_oto_config(&cfg_clone);
                                flush_snapshot_payload(payload);
                                let _ = do_restart;
                            }
                            KeyCode::Char('e') | KeyCode::Char('E') => {
                                // [e] settings_open içinde → otonom optimize tetikle (kapanmaz)
                                let mut st = state.lock().unwrap();
                                auto_tune_adaptive_params(&mut st);
                                // Optimize sonrası otomatik olarak item 28'e atla (sonuçları göster)
                                settings_idx = SETTINGS_LABELS.len() - 1;
                            }
                            KeyCode::Esc => {
                                settings_open = false;
                            }
                            _ => {}
                        }
                    }
                    // ── Günlük kaydırma (Tab 3 aktifken) ─────────────────────
                    KeyCode::Up | KeyCode::Char('k') if active_tab == 2 && !settings_open => {
                        let max = state.try_lock().map(|s| s.log.len().saturating_sub(1)).unwrap_or(usize::MAX);
                        log_scroll = (log_scroll + 3).min(max);
                    }
                    KeyCode::Down | KeyCode::Char('j') if active_tab == 2 && !settings_open => {
                        log_scroll = log_scroll.saturating_sub(3);
                    }
                    KeyCode::PageUp if active_tab == 2 => {
                        let max = state.try_lock().map(|s| s.log.len().saturating_sub(1)).unwrap_or(usize::MAX);
                        log_scroll = (log_scroll + 20).min(max);
                    }
                    KeyCode::PageDown if active_tab == 2 => {
                        log_scroll = log_scroll.saturating_sub(20);
                    }
                    KeyCode::Home | KeyCode::Char('g') if active_tab == 2 && !settings_open => {
                        log_scroll = 0;
                    }
                    KeyCode::End | KeyCode::Char('G') if active_tab == 2 && !settings_open => {
                        log_scroll = state.try_lock().map(|s| s.log.len().saturating_sub(1)).unwrap_or(log_scroll);
                    }
                    // ── Geçmiş işlem kaydırma (Tab 4 / Pozisyonlar aktifken) ──
                    KeyCode::Up | KeyCode::Char('k') if active_tab == 3 && !settings_open => {
                        let max = state.try_lock().map(|s| s.live_closed_trades.read().ok().map(|l| l.len()).unwrap_or(0)).unwrap_or(0);
                        trades_scroll = (trades_scroll + 3).min(max.saturating_sub(1));
                    }
                    KeyCode::Down | KeyCode::Char('j') if active_tab == 3 && !settings_open => {
                        trades_scroll = trades_scroll.saturating_sub(3);
                    }
                    KeyCode::PageUp if active_tab == 3 && !settings_open => {
                        let max = state.try_lock().map(|s| s.live_closed_trades.read().ok().map(|l| l.len()).unwrap_or(0)).unwrap_or(0);
                        trades_scroll = (trades_scroll + 20).min(max.saturating_sub(1));
                    }
                    KeyCode::PageDown if active_tab == 3 && !settings_open => {
                        trades_scroll = trades_scroll.saturating_sub(20);
                    }
                    KeyCode::Home if active_tab == 3 && !settings_open => {
                        trades_scroll = 0;
                    }
                    KeyCode::End if active_tab == 3 && !settings_open => {
                        let max = state.try_lock().map(|s| s.live_closed_trades.read().ok().map(|l| l.len()).unwrap_or(0)).unwrap_or(0);
                        trades_scroll = max.saturating_sub(1);
                    }
                    // ── Normal mod tuşları ────────────────────────────────────
                    KeyCode::Char('x') | KeyCode::Char('X') => {
                        let st = state.lock().unwrap();
                        let content = build_export_report(&st);
                        drop(st);

                        // Dosyaya yaz
                        let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
                        let path = format!("logs/rtc_export_{}.txt", ts);
                        let file_ok = std::fs::write(&path, &content).is_ok();

                        // Clipboard dene: wl-copy → xclip → xsel
                        let clip_ok = ["wl-copy", "xclip -selection clipboard", "xsel --clipboard --input"]
                            .iter()
                            .any(|cmd| {
                                let mut parts = cmd.split_whitespace();
                                let prog = parts.next().unwrap_or("");
                                let args: Vec<&str> = parts.collect();
                                std::process::Command::new(prog)
                                    .args(&args)
                                    .stdin(std::process::Stdio::piped())
                                    .spawn()
                                    .ok()
                                    .and_then(|mut c| {
                                        use std::io::Write;
                                        c.stdin.as_mut()?.write_all(content.as_bytes()).ok()?;
                                        c.wait().ok()
                                    })
                                    .map(|s| s.success())
                                    .unwrap_or(false)
                            });

                        let mut st = state.lock().unwrap();
                        let msg = match (file_ok, clip_ok) {
                            (true,  true)  => format!("📋 Günlük dışa aktarıldı → {} + clipboard", path),
                            (true,  false) => format!("📋 Günlük dışa aktarıldı → {} (clipboard aracı yok: apt install xclip)", path),
                            (false, true)  => "📋 Günlük clipboard'a kopyalandı (dosya yazılamadı)".to_string(),
                            (false, false) => "❌ Dışa aktarma başarısız (logs/ klasörü veya clipboard aracı yok)".to_string(),
                        };
                        st.push_log(msg);
                    }
                    // ── F5: Snapshot kaydedip process'i yeniden başlat ───────────────
                    KeyCode::F(5) => {
                        let payload = {
                            let st = state.lock().unwrap();
                            collect_snapshot_payload(&st)
                        };
                        flush_snapshot_payload(payload);
                        if let Ok(st) = state.try_lock() {
                            save_app_snapshot(&st);
                        }
                        state.lock().unwrap().push_log(
                            "🔄 F5 Restart — snapshot kaydedildi, yeniden başlatılıyor...".to_string()
                        );
                        quit_signal.store(true, Ordering::Relaxed);
                        restart_requested = true;
                        break;
                    }
                    KeyCode::Char('q') | KeyCode::Char('Q') => {
                        // Onay bekliyorsa Q → iptal
                        let mut st = state.lock().unwrap();
                        if st.confirm_pending.is_some() {
                            st.confirm_pending = None;
                            st.push_log("✖ İşlem iptal edildi.".to_string());
                        } else {
                            drop(st);
                            quit_signal.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                    // ── E tuşu: onay varsa evet yanıtı, yoksa ayarlar panelini aç ─────────
                    KeyCode::Char('r') | KeyCode::Char('R') if active_tab == 7 => {
                        // [R] Tab 8 Pipeline: Candle cache'i temizle (loop önbelleği sıfırlar)
                        let mut st = state.lock().unwrap();
                        if let Ok(mut p) = st.live_pipeline.try_write() {
                            p.log_repair("🔄 TUI: Candle cache temizleme istendi — loop yeniden seed'ler");
                        }
                        st.push_log("🔄 [R] Tab 8: Pipeline cache temizlendi — loop sonraki tick'te candle'ları yeniden çeker".to_string());
                        drop(st);
                    }
                    KeyCode::Char('e') | KeyCode::Char('E') => {
                        // Tab 8 Pipeline: Mini evrim zorla
                        if active_tab == 7 {
                            let st = state.lock().unwrap();
                            if let Ok(mut p) = st.live_pipeline.try_write() {
                                p.force_mini_evolution = true;
                                p.log_repair("⚡ TUI: Mini evrim zorla istendi");
                            }
                            drop(st);
                        } else {
                        let action = {
                            let st = state.lock().unwrap();
                            st.confirm_pending.clone()
                        };
                        if let Some(act) = action {
                            match act {
                                ConfirmAction::PaperReset => {
                                    let payload = {
                                        let mut st = state.lock().unwrap();
                                        st.confirm_pending = None;
                                        st.reset_paper_balance();
                                        st.pending_paper_reset = true;
                                        st.loop_restart_trigger.store(true, Ordering::Relaxed);
                                        let capital = st.config.capital;
                                        st.push_log(format!(
                                            "🔄 [z] Paper bakiye + loop sıfırlandı — sermaye: ${:.2}", capital
                                        ));
                                        collect_snapshot_payload(&st)
                                    };
                                    flush_snapshot_payload(payload);
                                }
                                ConfirmAction::FullReset => {
                                    let (stop_new, pause_new, capital) = {
                                        let mut st = state.lock().unwrap();
                                        st.confirm_pending = None;
                                        st.stop_signal.store(true, Ordering::Relaxed);
                                        let sn = Arc::new(AtomicBool::new(false));
                                        let pn = Arc::new(AtomicBool::new(false));
                                        st.stop_signal  = Arc::clone(&sn);
                                        st.pause_signal = Arc::clone(&pn);
                                        st.controller   = AutonomousController::new(AutonomousConfig::default());
                                        st.reset_pnl();
                                        st.paused    = false;
                                        st.live_mode = false;
                                        st.push_log("🔄 Sistem sıfırlandı — engine yeniden başlatılıyor".to_string());
                                        clear_all_snapshots(&st.config_paths.clone());
                                        (sn, pn, st.equity)
                                    };
                                    real_robotic_loop(Arc::clone(&state), stop_new, pause_new, capital, None);
                                }
                            }
                        } else {
                            // Onay bekleyen işlem yok → ayarlar panelini aç
                            settings_open = true;
                            settings_idx  = 0;
                        }
                        } // close else (active_tab != 7)
                    }
                    KeyCode::Char('8') => { active_tab = 7; }
                    KeyCode::Char('9') => { active_tab = 8; }
                    KeyCode::Char('h') | KeyCode::Char('H') => {
                        let mut st = state.lock().unwrap();
                        if st.confirm_pending.is_some() {
                            st.confirm_pending = None;
                            st.push_log("✖ İşlem iptal edildi.".to_string());
                        }
                    }
                    KeyCode::Char('1') => { active_tab = 0; }
                    KeyCode::Char('2') => { active_tab = 1; }
                    KeyCode::Char('3') => { active_tab = 2; }
                    KeyCode::Char('4') => { active_tab = 3; }
                    KeyCode::Char('5') => { active_tab = 4; }
                    KeyCode::Char('6') => { active_tab = 5; }
                    KeyCode::Char('7') => { active_tab = 6; }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        // auto_symbol toggle: AUTO ↔ MANUEL
                        let payload = {
                            let mut st = state.lock().unwrap();
                            st.auto_symbol = !st.auto_symbol;
                            let mode = if st.auto_symbol { "AUTO (otonom seçici)" } else { "MANUEL (config.json)" };
                            st.push_log(format!("🎯 Sembol modu: {}", mode));
                            st.symbol_trigger.store(true, Ordering::Relaxed);
                            collect_snapshot_payload(&st)
                        }; // lock serbest
                        flush_snapshot_payload(payload);
                    }
                    KeyCode::Char('b') | KeyCode::Char('B') => {
                        // Anlık backtest tetikle
                        let mut st = state.lock().unwrap();
                        st.backtest_trigger.store(true, Ordering::Relaxed);
                        st.push_log("🔬 [b] Manuel backtest tetiklendi — sonuç birkaç saniye içinde görünür...".to_string());
                        drop(st);
                    }
                    KeyCode::Char('f') | KeyCode::Char('F') => {
                        if active_tab == 7 {
                            // [F] Tab 8 Pipeline: Funding rate önbelleğini temizle
                            let st = state.lock().unwrap();
                            if let Ok(mut p) = st.live_pipeline.try_write() {
                                p.force_funding_refresh = true;
                                p.log_repair("🔄 TUI: Funding rate yenileme istendi");
                            }
                            drop(st);
                        } else {
                            // [F] Anlık Binance sembol tarayıcı çalıştır
                            let mut st = state.lock().unwrap();
                            st.screener_trigger.store(true, Ordering::Relaxed);
                            st.push_log("🔍 [f] Sembol tarayıcı tetiklendi — Binance 24hr ticker taranıyor...".to_string());
                        }
                    }
                    KeyCode::Char('o') | KeyCode::Char('O') => {
                        // [O] Binance'dan açık emir + geçmiş işlemleri anında senkronize et
                        let mut st = state.lock().unwrap();
                        st.exchange_orders_trigger.store(true, Ordering::Relaxed);
                        if st.paper_mode {
                            st.push_log("ℹ️ [o] Paper modda borsa sync yok — live moda geçince kullanılabilir".to_string());
                        } else {
                            st.push_log("🔄 [o] Borsa emir sync tetiklendi — Binance açık emirler + işlemler yükleniyor...".to_string());
                        }
                    }
                    KeyCode::Char('z') | KeyCode::Char('Z') => {
                        // [z] Onay iste — paper bakiye sıfırla
                        let mut st = state.lock().unwrap();
                        if st.paper_mode {
                            st.confirm_pending = Some(ConfirmAction::PaperReset);
                            st.push_log(ConfirmAction::PaperReset.prompt().to_string());
                        } else {
                            st.push_log("⚠ [z] Sadece paper modda kullanılabilir.".to_string());
                        }
                    }
                    KeyCode::Char('m') | KeyCode::Char('M') => {
                        // Anlık ML eğitim + hyperopt tetikle
                        let mut st = state.lock().unwrap();
                        st.ml_trigger.store(true, Ordering::Relaxed);
                        st.push_log("⚡ [m] ML eğitim + HyperOpt tetiklendi...".to_string());
                    }
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        // Anlık p5_crypto Python analizi tetikle
                        let mut st = state.lock().unwrap();
                        st.p5_trigger.store(true, Ordering::Relaxed);
                        let (sym, intv) = {
                            let tgt = st.active_trade_target();
                            (tgt.2.clone(), tgt.3.clone())
                        };
                        st.push_log(format!("🐍 [y] p5_crypto analizi tetiklendi — {} {}", sym, intv));
                    }
                    KeyCode::Char('w') | KeyCode::Char('W') => {
                        // Pipeline'ı manuel olarak baştan tetikle (D→B→ML→P5)
                        let mut st = state.lock().unwrap();
                        st.pipeline.trigger.store(true, Ordering::Relaxed);
                        // Idle'a döndür (Done aşamasındaysa beklemesini bırak)
                        if st.pipeline.phase != PipelinePhase::Idle {
                            st.pipeline.phase    = PipelinePhase::Done;
                            st.pipeline.next_run_at = std::time::Instant::now();
                        }
                        st.push_log("🔄 [w] Otonom Pipeline manuel tetiklendi (D→B→ML→P5)".to_string());
                    }
                    KeyCode::Char('u') | KeyCode::Char('U') => {
                        // MTF fırsat tarayıcısını anında tetikle
                        let mut st = state.lock().unwrap();
                        st.mtf_scan_trigger.store(true, Ordering::Relaxed);
                        st.push_log("🔭 [u] MTF fırsat taraması tetiklendi...".to_string());
                    }
                    KeyCode::Char('d') | KeyCode::Char('D') => {
                        // Anlık veri indirme tetikle
                        let mut st = state.lock().unwrap();
                        st.download_trigger.store(true, Ordering::Relaxed);
                        st.push_log("⏬ [d] Anlık veri indirme tetiklendi...".to_string());
                    }
                    KeyCode::Char('t') | KeyCode::Char('T') => {
                        // Anlık sinyal değerlendirmesi tetikle
                        let mut st = state.lock().unwrap();
                        st.signal_trigger.store(true, Ordering::Relaxed);
                        st.push_log("⚡ [t] Anlık sinyal değerlendirmesi tetiklendi...".to_string());
                    }
                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        let mut st = state.lock().unwrap();
                        st.paused = !st.paused;
                        st.pause_signal.store(st.paused, Ordering::Relaxed);
                        let msg = if st.paused {
                            "⏸ Döngü duraklatıldı".to_string()
                        } else {
                            "▶ Döngü devam ediyor".to_string()
                        };
                        st.push_log(msg);
                    }
                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        // [r] Onay iste — tam sistem sıfırla
                        let mut st = state.lock().unwrap();
                        st.confirm_pending = Some(ConfirmAction::FullReset);
                        st.push_log(ConfirmAction::FullReset.prompt().to_string());
                    }
                    // ── Tab 6 HTF scroll: ↑↓ / j k / PgUp PgDn / g G ──────────────────────
                    KeyCode::Up | KeyCode::Char('k') if active_tab == 5 && !settings_open => {
                        htf_scroll = htf_scroll.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') if active_tab == 5 && !settings_open => {
                        htf_scroll = htf_scroll.saturating_add(1);
                    }
                    KeyCode::PageUp if active_tab == 5 && !settings_open => {
                        htf_scroll = htf_scroll.saturating_sub(10);
                    }
                    KeyCode::PageDown if active_tab == 5 && !settings_open => {
                        htf_scroll = htf_scroll.saturating_add(10);
                    }
                    KeyCode::Home | KeyCode::Char('g') if active_tab == 5 && !settings_open => {
                        htf_scroll = 0;
                    }
                    KeyCode::End | KeyCode::Char('G') if active_tab == 5 && !settings_open => {
                        htf_scroll = usize::MAX; // draw fonksiyonu sınırlayacak
                    }
                    // ── Tab 5 (S/R) sembol gezinme: ← → ile sembol değiştir ──────────────
                    KeyCode::Left if active_tab == 4 && !settings_open => {
                        let mut st = state.lock().unwrap();
                        let n = st.live_sr_zones.read().ok().map(|g| g.len()).unwrap_or(0);
                        if n > 0 {
                            st.sr_tab_sym_idx = if st.sr_tab_sym_idx == 0 { n - 1 } else { st.sr_tab_sym_idx - 1 };
                        }
                    }
                    KeyCode::Right if active_tab == 4 && !settings_open => {
                        let mut st = state.lock().unwrap();
                        let n = st.live_sr_zones.read().ok().map(|g| g.len()).unwrap_or(0);
                        if n > 0 {
                            st.sr_tab_sym_idx = (st.sr_tab_sym_idx + 1) % n;
                        }
                    }
                    // ── [C] Açık pozisyonları zorla kapat (Tab 5 = Pozisyonlar) ──────────
                    // SL ihlal eden TÜM pozisyonları anlık fiyatla kapatır.
                    // Yeniden başlatma gerekmeden manuel müdahale imkânı sağlar.
                    KeyCode::Char('c') | KeyCode::Char('C') if active_tab == 3 => {
                        use memos_trading_core::robot::robotic_loop::{ClosedTradeData, is_duplicate_trade};
                        let mut st = state.lock().unwrap();
                        let to_close: Vec<(String, memos_trading_core::robot::robotic_loop::LivePositionData)> =
                            if let Ok(lm) = st.live_positions.read() {
                                lm.iter().filter_map(|(k, pos)| {
                                    if pos.current_price <= 0.0 || pos.static_sl <= 0.0 { return None; }
                                    let sl_hit = (pos.is_long  && pos.current_price <= pos.static_sl)
                                              || (!pos.is_long && pos.current_price >= pos.static_sl);
                                    if sl_hit { Some((k.clone(), pos.clone())) } else { None }
                                }).collect()
                            } else { vec![] };
                        for (key, snap) in &to_close {
                            if let Ok(mut lm) = st.live_positions.write() { lm.remove(key.as_str()); }
                            let price = snap.current_price;
                            let pnl = if snap.is_long {
                                (price - snap.entry_price) * snap.qty
                            } else {
                                (snap.entry_price - price) * snap.qty
                            };
                            let lev_snap2 = snap.leverage.max(1.0);
                            let pnl_pct = if snap.entry_price > 0.0 && snap.qty > 0.0 {
                                pnl * lev_snap2 / (snap.entry_price * snap.qty) * 100.0
                            } else { 0.0 };
                            const FORCE_COMM_PCT: f64 = 0.001;
                            let entry_comm_fc = snap.entry_price * snap.qty * FORCE_COMM_PCT;
                            let exit_comm_fc  = price            * snap.qty * FORCE_COMM_PCT;
                            let trade = ClosedTradeData {
                                pos_id:      snap.pos_id,
                                symbol:      snap.symbol.clone(),
                                is_long:     snap.is_long,
                                entry_price: snap.entry_price,
                                exit_price:  price,
                                qty:         snap.qty,
                                pnl,
                                pnl_pct,
                                exit_reason: "force_close_sl".to_string(),
                                closed_at:   chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                                leverage:    lev_snap2,
                                sl_price:    snap.static_sl,
                                tp_price:    snap.static_tp,
                                opened_at:   snap.opened_at.clone(),
                                trade_type:  snap.trade_type,
                                entry_commission: entry_comm_fc,
                                exit_commission:  exit_comm_fc,
                                slippage_usd:     0.0,
                                entry_rsi:        0.0,
                                entry_atr_pct:    0.0,
                                close_adx_regime: 0,
                                close_funding_rate: 0.0,
                                close_btc_corr:   0.0,
                            };
                            let dup = st.live_closed_trades.read().ok()
                                .map(|cl| is_duplicate_trade(&cl, trade.pos_id))
                                .unwrap_or(false);
                            if !dup {
                                if let Ok(mut cl) = st.live_closed_trades.write() {
                                    if cl.len() >= 500 { cl.remove(0); }
                                    cl.push(trade.clone());
                                }
                                let comm_usd = trade.entry_commission + trade.exit_commission;
                                let tt = trade.trade_type;
                                if let Ok(mut costs) = st.live_execution_costs.write() {
                                    costs.record(tt, comm_usd, 0.0, 0.0, 0.0);
                                }
                            }
                            st.push_log(format!(
                                "🔴 [C] Force-close: {} {} | fiyat={:.4} giriş={:.4} pnl={:+.2} ({:+.1}%)",
                                snap.symbol, if snap.is_long { "LONG" } else { "SHORT" },
                                price, snap.entry_price, pnl, pnl_pct
                            ));
                        }
                        if to_close.is_empty() {
                            st.push_log("ℹ [C] Kapatılacak SL ihlali olan pozisyon yok.".to_string());
                        }
                        let payload = collect_snapshot_payload(&st);
                        drop(st); // lock serbest
                        flush_snapshot_payload(payload);
                    }
                    KeyCode::Char('l') | KeyCode::Char('L') => {
                        show_progress = !show_progress;
                        loading_scroll = 0; // her açılışta en üstten başla
                    }
                    KeyCode::Char('i') | KeyCode::Char('I') => {
                        let (cfg_clone, profile_path, payload) = {
                            let mut st = state.lock().unwrap();
                            let cur = st.active_symbol.interval.clone();
                            let next = STANDARD_INTERVALS.iter()
                                .position(|&x| x == cur.as_str())
                                .map(|pos| STANDARD_INTERVALS[(pos + 1) % STANDARD_INTERVALS.len()])
                                .unwrap_or("1m");
                            st.active_symbol.interval = next.to_string();
                            st.config.interval = next.to_string();
                            st.loop_restart_trigger.store(true, Ordering::Relaxed);
                            st.backtest_trigger.store(true, Ordering::Relaxed);
                            st.push_log(format!("⏱ Interval değişti → {} — loop+backtest yeniden başlıyor", next));
                            let pp = st.config_paths.robotic_profiles.clone();
                            (st.config.clone(), pp, collect_snapshot_payload(&st))
                        }; // lock serbest
                        save_oto_config(&cfg_clone);
                        save_profile_config(&profile_path, {
                            // profile için state tekrar lock al
                            &state.lock().unwrap()
                        });
                        flush_snapshot_payload(payload);
                    }
                    _ => {}
                }  // match key.code
            }      // Event::Key arm
            _ => {}// Event::Resize, FocusGained vb. — yoksay
            }      // match event::read()?
            }      // if event::poll

        // ── AUTO sembol/interval değişince loop'u yeniden başlat ────────────
        // Arc<AtomicBool> — lock almadan atomik oku, donma riski yok
        let restart_needed = restart_trigger.swap(false, Ordering::Relaxed);
        // Debounce: 2sn içinde birden fazla restart tetiklenmesini engelle
        let restart_needed = restart_needed && last_restart_at.elapsed() >= Duration::from_secs(2);
        if restart_needed {
            last_restart_at = Instant::now();
            let (stop_new, pause_new, capital, new_exchange, new_sym, new_mkt, new_intv) = {
                let mut st = state.lock().unwrap();

                let new_symbol   = st.active_symbol.symbol.clone();
                let new_market   = st.active_symbol.market.clone();
                let new_interval = st.active_symbol.interval.clone();

                // Çok-sembol modunda: sadece yeni sembole karşılık gelmeyen ESKİ primary'yi durdur.
                // Diğer top-N worker'ları (2-5) çalışmaya devam eder — sync_orchestrator_workers halleder.
                // Eski primary hangi sembol? Çalışan worker'lardan yeni_sembol olmayan tek kayıt.
                let old_primary: Option<String> = st.orchestrator.active_symbols()
                    .into_iter()
                    .find(|s| s != &new_symbol);
                if let Some(ref old_sym) = old_primary {
                    st.orchestrator.stop_symbol(old_sym);
                    st.push_log(format!("🔄 Primary geçiş: {} durduruldu → {}", old_sym, new_symbol));
                }

                // Yeni worker sinyallerini orchestrator üzerinden kaydet
                let (sn, pn, _price) = st.orchestrator
                    .register(&new_symbol, &new_market, &new_interval)
                    .unwrap_or_else(|| {
                        // Kapasite doluysa direkt yeni arc yarat (fallback)
                        (Arc::new(AtomicBool::new(false)),
                         Arc::new(AtomicBool::new(false)),
                         Arc::new(std::sync::RwLock::new(LivePriceData::default())))
                    });

                // Eski primary loop'u durdur: mevcut stop_signal Arc'ını true yap.
                // (Arc::clone(&sn) ile değiştirmeden önce yapılmazsa eski thread çalışmaya devam eder)
                st.stop_signal.store(true, Ordering::Relaxed);

                // AppState'in genel stop/pause sinyalini de güncelle (pause tuşu uyumu)
                st.stop_signal   = Arc::clone(&sn);
                st.pause_signal  = Arc::clone(&pn);
                st.paused        = false;
                st.loop_active_since = Instant::now();

                // live_price aktif sembolün arc'ına güncelle
                if let Some(price_arc) = st.orchestrator.live_price_for(&new_symbol) {
                    *st.live_price.write().unwrap() = LivePriceData::default();
                    // Not: AppState.live_price'ı yeni arc ile değiştiremeyiz (Arc<RwLock<_>> clone),
                    // bunun yerine dashboard draw_live_price zaten orchestrator üzerinden okur.
                    let _ = price_arc; // Arc sahibi orchestrator'da, loop da oradan alır
                }
                let new_exchange = {
                    let e = st.active_symbol.exchange.clone();
                    if e.is_empty() { if st.config.exchange.is_empty() { "binance".to_string() } else { st.config.exchange.clone() } } else { e }
                };
                (sn, pn, st.equity, new_exchange, new_symbol, new_market, new_interval)
            };
            // Kısa bekleme — eski loop'un durması için (200ms yeterli: loop tick ~1s ama
            // stop_signal kontrolü her await noktasında çalışır → genellikle <100ms)
            std::thread::sleep(Duration::from_millis(300));
            // Yeni sembol için hemen backtest + ML eğitim tetikle
            {
                let mut st = state.lock().unwrap();
                st.backtest_trigger.store(true, Ordering::Relaxed);
                st.ml_trigger.store(true, Ordering::Relaxed);
                st.push_log(format!(
                    "🔄 Sembol değişimi → {} | backtest + ML anlık tetiklendi", new_sym
                ));

                // ── [z] reset: eski loop durduktan sonra garantili ikinci temizlik ──────
                // Race condition: eski loop [z] ile reset arasında pozisyon kapatıp
                // costs/trades Arc'ına yazabilir. 300ms bekleme sonrası eski loop
                // durmuş olmalı → şimdi tüm oturum verisi kesin olarak sıfırlanır.
                if st.pending_paper_reset {
                    st.pending_paper_reset = false;
                    if st.paper_mode {
                        // Tüm oturum verisi: equity, trades, positions, costs, signals
                        st.equity       = st.config.capital;
                        st.total_trades = 0;
                        st.pnl_snapshots.clear();
                        st.live_trade_count.store(0, std::sync::atomic::Ordering::Relaxed);
                        if let Ok(mut log) = st.live_closed_trades.write()  { log.clear(); }
                        if let Ok(mut pos) = st.live_positions.write()       { pos.clear(); }
                        if let Ok(mut costs) = st.live_execution_costs.write() {
                            *costs = memos_trading_core::robot::robotic_loop::CumulativeTradingCosts::default();
                        }
                        if let Ok(mut sc) = st.live_signal_counts.write() {
                            *sc = LiveSignalCounts::default();
                        }
                        // ph istatistiklerini de anında sıfırla — stale loss_streak görünmesin
                        if let Ok(mut ph) = st.live_pipeline.write() {
                            ph.loss_streak    = 0;
                            ph.session_wins   = 0;
                            ph.session_closed = 0;
                            ph.anomalies.clear();
                        }
                        // DB pozisyon snapshot'ını temizle — yeni loop DB'den eski pozisyon geri yüklemesin.
                        // [z] olmadan restart'ta (ör. sembol değişimi) DB'den restore DEVAM eder.
                        if let Ok(conn) = rusqlite::Connection::open(&st.config.db_path) {
                            let _ = memos_trading_core::database_writer::clear_open_positions_snapshot(&conn);
                        }
                        let reset_capital = st.config.capital;
                        st.push_log(format!(
                            "✅ [z] Tam sıfırlama tamamlandı — pozisyonlar+DB+maliyetler sıfır, sermaye: ${:.2}",
                            reset_capital
                        ));
                    }
                }
            }
            real_robotic_loop(
                Arc::clone(&state), stop_new, pause_new, capital,
                Some((new_exchange, new_mkt, new_sym, new_intv)),
            );
            // Sembol değişiminden hemen sonra sync çalıştır — eski primary'yi
            // ikincil worker olarak geri ekle (5 dakika beklemeye gerek kalmaz).
            sync_orchestrator_workers(Arc::clone(&state));
        }

        // Ctrl+C veya harici sinyal — q tuşuyla aynı temiz çıkış yolunu izle
        if quit_signal.load(Ordering::Relaxed) {
            break;
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }

        // ── Otonom adaptive params auto-tune: her N kapanmış işlemde bir ───────
        // N = adaptive_params.adjust_every_n_trades (varsayılan 20, min 3)
        if let Ok(mut st) = state.try_lock() {
            let closed_count = {
                st.live_closed_trades.read().ok()
                    .map(|log| log.len() as u32)
                    .unwrap_or(0)
            };
            let n = st.adaptive_params.adjust_every_n_trades.max(3);
            let since_last = closed_count.saturating_sub(st.last_auto_tune_trade_count);
            if since_last >= n && closed_count >= 3 {
                st.last_auto_tune_trade_count = closed_count;
                auto_tune_adaptive_params(&mut st);
                st.push_log(format!(
                    "⚙ Auto-tune: {} kapanmış işlem → parametreler güncellendi", closed_count
                ));
            }
        }

        // ── 60dk periyodik otomatik export ──────────────────────────────────
        if last_auto_export.elapsed() >= Duration::from_secs(3600) {
            last_auto_export = Instant::now();
            if let Ok(st) = state.try_lock() {
                let content = build_export_report(&st);
                drop(st);
                let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
                let path = format!("logs/rtc_export_{}.txt", ts);
                if std::fs::write(&path, &content).is_ok() {
                    if let Ok(mut st) = state.try_lock() {
                        st.push_log(format!("📋 [Otomatik] Export kaydedildi → {}", path));
                    }
                }
            }
        }
    }

    // Çıkıştan önce uygulama snapshot'ını kaydet (açık pozisyonlar dahil)
    if let Ok(st) = state.try_lock() {
        save_app_snapshot(&st);
    }

    // WAL checkpoint: kapanmadan önce tüm veriyi ana DB dosyasına yaz
    if let Ok(st) = state.try_lock() {
        let db = st.config.db_path.clone();
        drop(st);
        if let Ok(conn) = database_writer::open_connection(&db) {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
    }

    // Terminal'i eski haline getir
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(restart_requested)
}

// ─── Giriş Noktası ───────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // Config dizininin varlığını garanti et — path resolution'dan önce, bir kez çalışır
    ensure_config_dir();

    // .env dosyalarını yükle — binary konumundan workspace kökünü bularak
    // from_path_override: sonraki dosya öncekinin üzerine yazar → desktop/.env kazanır
    if let Ok(exe) = std::env::current_exe() {
        // binary: .../memos_trading_core/target/debug/rtc_cli  → 4 yukarı = workspace
        if let Some(workspace) = exe.ancestors().nth(4) {
            let _ = dotenvy::from_path(workspace.join("memos_trading_core/.env"));
            // desktop key'leri varsa onlar kazansın
            let _ = dotenvy::from_path_override(workspace.join("memos_trading_desktop/.env"));
        }
    }
    // Yukarıdaki başarısız olursa fallback: cwd'den dene (desktop key'leri kazanır)
    let _ = dotenvy::from_path("../memos_trading_desktop/.env");
    let _ = dotenvy::from_path_override(".env"); // core/.env son olarak — artık dolu

    let state = Arc::new(Mutex::new(AppState::new()));

    // Başlangıç mesajları
    {
        let mut st = state.lock().unwrap();
        st.push_log("🚀 Memos RTC CLI başlatıldı".to_string());
        // .env yükleme sonucu
        let loaded_key = std::env::var("BINANCE_API_KEY").unwrap_or_default();
        st.push_log(format!(
            "🔑 .env yükleme: BINANCE_API_KEY={} ({} karakter)",
            if loaded_key.is_empty() { "BOŞ" } else { "SET" },
            loaded_key.len(),
        ));
        st.push_log("🧬 AutonomousController + AdaptiveBrain + PopulationManager aktif".to_string());
        st.push_log("🛡 RiskGate politikası yüklendi (max draw %10, day loss %3)".to_string());
        st.push_log("⚙️ RecoverySupervisor hazır (retry=2, safe=3, halt=7)".to_string());
        // Kapasite bilgisi
        let cpu_cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
        let max_w    = st.orchestrator.max_workers;
        let intv_str = st.config.interval.clone();
        st.push_log(format!(
            "📊 Çok-sembol kapasite: max={} worker | CPU={} çekirdek | interval={}",
            max_w, cpu_cores, intv_str,
        ));
    }

    // Gerçek RoboticLoop'u arkaplan thread'inde başlat
    // Başlangıç sembolünü orchestrator'a kaydet — per-sembol live_price Arc'ı oluşturulur
    let (stop_sig, pause_sig, capital) = {
        let mut st = state.lock().unwrap();
        let sym  = st.active_symbol.symbol.clone();
        let mkt  = st.active_symbol.market.clone();
        let intv = st.active_symbol.interval.clone();
        if sym.is_empty() {
            // Fallback: config'den al
            let s = st.config.symbol.clone();
            let m = st.config.market.clone();
            let i = st.config.interval.clone();
            let _ = st.orchestrator.register(&s, &m, &i);
        } else {
            let _ = st.orchestrator.register(&sym, &mkt, &intv);
        }
        (Arc::clone(&st.stop_signal), Arc::clone(&st.pause_signal), st.equity)
    };
    real_robotic_loop(Arc::clone(&state), stop_sig, pause_sig, capital, None);

    // Panic hook: TUI açıkken panic olursa terminal'i düzgün kapat
    // (tmux dahil her ortamda raw mode + alternate screen'i geri alır)
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Terminal'i restore et — hatalar yok sayılır (zaten panikte)
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
        );
        // Orijinal panik mesajını göster
        default_hook(info);
    }));

    // Ctrl+C → q tuşuyla aynı temiz çıkış yolunu tetikle (WAL checkpoint dahil)
    {
        let ctrlc_quit = Arc::clone(&state.lock().unwrap().app_stop_signal);
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            ctrlc_quit.store(true, Ordering::Relaxed);
        });
    }

    // TUI döngüsünü ana thread'de çalıştır
    match run_tui(state) {
        Ok(true) => {
            // F5 restart: mevcut process'i aynı argümanlarla kendisi üzerine exec et
            // exec() mevcut process'i değiştirir — fork etmez, yeni PID yaratmaz.
            // tmux/terminal oturumu bozulmaz; snapshot sayesinde state korunur.
            //
            // Linux'ta cargo rebuild binary'yi atomik rename ile değiştirir.
            // Çalışan process'in /proc/self/exe'si "(deleted)" olarak işaretlenir.
            // current_exe() bu path'i döndürünce exec() ENOENT verir.
            // Çözüm: "(deleted)" suffix'i sil → yeni binary aynı path'te zaten var.
            use std::os::unix::process::CommandExt;
            let exe = std::env::current_exe().unwrap_or_else(|_| {
                std::env::args_os().next()
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::path::PathBuf::from("./target/release/rtc_cli"))
            });
            // " (deleted)" suffix'i temizle (cargo rebuild sonrası Linux davranışı)
            let exe = {
                let s = exe.to_string_lossy();
                if s.ends_with(" (deleted)") {
                    std::path::PathBuf::from(s.trim_end_matches(" (deleted)"))
                } else {
                    exe
                }
            };
            let args: Vec<_> = std::env::args_os().skip(1).collect();
            let err = std::process::Command::new(&exe)
                .args(&args)
                .exec(); // Bu satırdan sonrası sadece exec() başarısız olursa çalışır
            // exec() başarısız: terminal'i temizle, sonra hata bas
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = crossterm::execute!(
                io::stdout(),
                crossterm::terminal::LeaveAlternateScreen,
                crossterm::event::DisableMouseCapture,
            );
            eprintln!("Restart başarısız: {} (binary: {})", err, exe.display());
            std::process::exit(1);
        }
        Ok(false) => {} // Normal çıkış
        Err(e) => {
            let _ = crossterm::terminal::disable_raw_mode();
            eprintln!("TUI hatası: {}", e);
            std::process::exit(1);
        }
    }
}
