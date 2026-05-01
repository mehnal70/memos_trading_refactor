use crate::robot::optimizer::HyperOptimizer;
use crate::robot::advanced::strategy_selector::StrategySelector;
use crate::robot::autonomous_trader::AutonomousTrader;
use crate::evolution::adaptive_brain::AdaptiveBrain;
use crate::evolution::population_manager::PopulationManager;
use crate::robot::autonomous_control::{
    AutonomousConfig as AutonomousControllerConfig,
    AutonomousController,
    AutonomousRecoveryAction,
    RecoverySupervisor,
    RiskDecision,
    RiskGate,
    RiskGatePolicy,
    RiskInput,
};
// Dosya başındaki fazla kapatma parantezleri kaldırıldı
use crate::robot::ml_engine::{MLModel, FeatureVector, FeatureExtractor, LinearRegressor};
#[cfg(not(target_arch = "wasm32"))]
use crate::robot::Portfolio;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    Live,
    Backtest,
}

// robot/robotic_loop.rs - Tam otomatik robotik trade döngüsü
// Bu modül, veri çekme, sinyal üretme, risk hesaplama ve emir gönderme adımlarını sürekli ve otomatik olarak çalıştırır.
// Hata yönetimi ve loglama entegredir.

use crate::robot::{RoboticTradeExecutor, StateManager, UniversalReporter, ErrorLogger, LiveDataFetcher, SimpleRiskAnalyzer, Monitor};
use crate::robot::error_recovery::circuit_breaker::{CircuitBreaker, CircuitBreakerState};
use crate::robot::risk_guardrails::{DrawdownMonitor, DrawdownStatus};
use crate::robot::order_management::paper_executor::ExecutionCostConfig;
use crate::robot::position_manager::OpenPosition;
use crate::robot::signal_evaluator::{TrendBias, average_range_pct, trend_bias, check_sr_filter};
use crate::robot::sr_detector::SrDetector;
use crate::robot::pattern_matcher::{MarketCondition, compute_confidence};
// TradeQualityConfig signal_evaluator'dan re-export (geriye dönük uyumluluk)
pub use crate::robot::signal_evaluator::TradeQualityConfig;
use crate::robot::adaptive_params::AdaptiveTradeParams;
use crate::risk::RiskManager;
use chrono::Utc;
use chrono::Timelike;
use crate::types::{Signal, StrategyParams, Exchange, RiskParams, Candle};
use crate::candle_synth::CandleSynth;
use crate::types::Market;
use serde::{Deserialize, Serialize};
use std::fs;

/// Loop içinden evrim state'ini doğrudan diske kaydet (AppState'e bağımlı olmadan)
fn save_evolution_state_from_loop(controller: &AutonomousController) {
    #[derive(serde::Serialize)]
    struct EvoSnap<'a> {
        brain:      Option<&'a crate::evolution::adaptive_brain::AdaptiveBrain>,
        population: Option<&'a crate::evolution::population_manager::PopulationManager>,
        /// Restart sonrası cycle devamlılığı için cycle_id persist ediliyor
        cycle_id:   u64,
    }
    let snap = EvoSnap {
        brain:      controller.adaptive_brain.as_ref(),
        population: controller.population_manager.as_ref(),
        cycle_id:   controller.cycle_id,
    };
    let candidates = ["config/evolution_state.json", "../config/evolution_state.json"];
    for path in &candidates {
        if let Some(parent) = std::path::Path::new(path).parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&snap) {
            if fs::write(path, json).is_ok() {
                break;
            }
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
#[cfg(not(target_arch = "wasm32"))]
use crate::strategies::{Strategy, MaCrossoverStrategy, RsiStrategy, MacdStrategy, BollingerBandsStrategy, DonchianChannelStrategy};
use crate::robot::strategies::{
    SupertrendStrategy, EmaCrossoverStrategy, StochasticRsiStrategy, CciStrategy,
    StochasticStrategy, WilliamsRStrategy, AdxStrategy, VwapStrategy,
    PriceActionStrategy, IctFvgStrategy, SmcStrategy,
    IctOrderBlockStrategy, IctLiquiditySweepStrategy, IctKillzoneStrategy,
    IctOteStrategy, IctCompositeStrategy,
};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

/// İsimden strateji trait objesi üretir; bilinmeyenlerde MA Crossover döner.
#[cfg(not(target_arch = "wasm32"))]
pub fn make_strategy_pub(name: &str) -> Box<dyn Strategy> { make_strategy(name) }

fn make_strategy(name: &str) -> Box<dyn Strategy> {
    match name {
        "RSI"        => Box::new(RsiStrategy),
        "MACD"       => Box::new(MacdStrategy),
        "BB"         => Box::new(BollingerBandsStrategy),
        "DONCHIAN"   => Box::new(DonchianChannelStrategy),
        "SUPERTREND" => Box::new(SupertrendStrategy),
        "EMA"        => Box::new(EmaCrossoverStrategy),
        "STOCH_RSI"  => Box::new(StochasticRsiStrategy),
        "CCI"        => Box::new(CciStrategy),
        "STOCHASTIC"     => Box::new(StochasticStrategy),
        "WILLIAMS"       => Box::new(WilliamsRStrategy),
        "ADX"            => Box::new(AdxStrategy),
        "VWAP"           => Box::new(VwapStrategy),
        "PRICE_ACTION"   => Box::new(PriceActionStrategy),
        "ICT_FVG"        => Box::new(IctFvgStrategy),
        "SMC"            => Box::new(SmcStrategy),
        "ICT_OB"         => Box::new(IctOrderBlockStrategy),
        "ICT_SWEEP"      => Box::new(IctLiquiditySweepStrategy),
        "ICT_KILLZONE"   => Box::new(IctKillzoneStrategy),
        "ICT_OTE"        => Box::new(IctOteStrategy),
        "ICT_COMPOSITE"  => Box::new(IctCompositeStrategy),
        _                => Box::new(MaCrossoverStrategy),
    }
}

/// Sabit parametre grid — her tick'te yeniden tahsis edilmez.
/// index 0 = HyperOpt optimize edilmiş (config'den gelir, runtime'da değişebilir)
/// index 1-5 = statik grid (LazyLock ile tek seferlik tahsis)
#[cfg(not(target_arch = "wasm32"))]
static STATIC_PARAM_GRID: std::sync::LazyLock<[crate::types::StrategyParams; 5]> =
    std::sync::LazyLock::new(|| [
        crate::types::StrategyParams { fast: Some(5),  slow: Some(20), period: Some(14), overbought: Some(70.0), oversold: Some(30.0), fast_period: Some(12), slow_period: Some(26), signal_period: Some(9),  std_dev: Some(2.0), bb_period: Some(20) },
        crate::types::StrategyParams { fast: Some(9),  slow: Some(21), period: Some(10), overbought: Some(80.0), oversold: Some(20.0), fast_period: Some(12), slow_period: Some(26), signal_period: Some(9),  std_dev: Some(2.5), bb_period: Some(20) },
        crate::types::StrategyParams { fast: Some(8),  slow: Some(25), period: Some(14), overbought: Some(70.0), oversold: Some(30.0), fast_period: Some(8),  slow_period: Some(21), signal_period: Some(5),  std_dev: Some(2.0), bb_period: Some(20) },
        crate::types::StrategyParams { fast: Some(12), slow: Some(26), period: Some(14), overbought: Some(75.0), oversold: Some(25.0), fast_period: Some(12), slow_period: Some(26), signal_period: Some(9),  std_dev: Some(2.0), bb_period: Some(20) },
        crate::types::StrategyParams { fast: Some(5),  slow: Some(15), period: Some(9),  overbought: Some(70.0), oversold: Some(30.0), fast_period: Some(5),  slow_period: Some(13), signal_period: Some(4),  std_dev: Some(1.5), bb_period: Some(15) },
    ]);

/// Canlı fiyat verisi (son mum) — loop yazar, TUI okur.
#[derive(Debug, Clone)]
pub struct LivePriceData {
    pub symbol:     String,
    pub open:       f64,
    pub high:       f64,
    pub low:        f64,
    pub close:      f64,
    pub volume:     f64,
    pub change_pct: f64,   // (close - open) / open * 100
    pub ts:         String, // insan okunabilir zaman damgası
    /// WS/REST son güncelleme zamanı (epoch milisaniye). 0 = hiç güncellenmedi.
    /// Fiyat ağırlıklandırma motoru bu değere göre WS tazeliğini ölçer.
    pub last_updated_ms: u64,
}

impl Default for LivePriceData {
    fn default() -> Self {
        Self { symbol: String::new(), open: 0.0, high: 0.0, low: 0.0, close: 0.0,
               volume: 0.0, change_pct: 0.0, ts: String::new(), last_updated_ms: 0 }
    }
}

/// Açık pozisyon anlık görüntüsü — loop yazar, TUI okur.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LivePositionData {
    /// Evrensel tekil kimlik — dedup ve cross-reference için
    #[serde(default)]
    pub pos_id:      crate::types::PositionId,
    pub symbol:      String,
    #[serde(default)]  // eski snapshot'larda market yoksa Spot varsayılan
    pub market:      crate::types::Market,  // hangi market'te açıldığı — yanlış fiyat seçimini önler
    pub is_long:     bool,
    pub entry_price: f64,
    pub qty:         f64,
    pub static_sl:   f64,    // sabit SL fiyatı
    pub static_tp:   f64,    // sabit TP fiyatı
    pub trailing_sl: Option<f64>,  // aktif trailing SL fiyatı (None = kâra geçmedi)
    pub trailing_pct: Option<f64>, // trailing SL yüzdesi
    pub best_price:  f64,    // en iyi fiyat (trailing tavanı)
    pub current_price: f64,  // son bilinen fiyat
    /// Uygulanan kaldıraç (1.0 = kaldıraçsız). Eski snapshot'lar için serde default.
    #[serde(default = "default_leverage")]
    pub leverage: f64,
    /// Tasfiye fiyatı — kaldıraçlı işlemlerde borsa zorla kapatma noktası.
    #[serde(default)]
    pub liquidation_price: f64,
    /// B1: Breakeven tetiklendi mi (SL giriş fiyatına taşındı)?
    #[serde(default)]
    pub breakeven_triggered: bool,
    /// B3: Kısmi TP gerçekleşti mi?
    #[serde(default)]
    pub partial_tp_triggered: bool,
    /// B2: ATR trailing aktif mi?
    #[serde(default)]
    pub atr_trail_active: bool,
    /// Pozisyon açılış UTC damgası. Eski snapshot'larda boş string.
    #[serde(default)]
    pub opened_at: String,
    /// TP Merdiveni: TP1 seviyesi (None = kapalı)
    #[serde(default)]
    pub tp1_price: Option<f64>,
    /// TP1 zaten gerçekleşti mi?
    #[serde(default)]
    pub tp1_triggered: bool,

    /// Motor türü: Regular | Scalp | Swing
    #[serde(default)]
    pub trade_type: crate::robot::scalp_swing::TradeType,
}

/// HashMap tipi takma adı — "{symbol}-{market:?}" → pozisyon anlık görüntüsü
/// Composite key kullanımı: aynı sembol farklı market'lerde açık olabilir (Spot vs Futures).
/// Sadece sembol key'i kullanmak, başka market'in fiyatının yanlışlıkla okunmasına yol açar.
pub type LivePositionMap = std::collections::HashMap<String, LivePositionData>;

/// Composite map key: "{symbol}-{market:?}"  →  "BTCUSDT-Futures", "BTCUSDT-Spot"
#[inline]
pub fn live_pos_key(symbol: &str, market: &crate::types::Market) -> String {
    format!("{}-{:?}", symbol, market)
}

/// Kapanmış (gerçekleşmiş) işlem kaydı — TUI geçmiş sekmesi için.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ClosedTradeData {
    /// Evrensel tekil kimlik — String hash yerine Uuid ile güvenilir dedup.
    #[serde(default)]
    pub pos_id:      crate::types::PositionId,
    pub symbol:      String,
    pub is_long:     bool,
    pub entry_price: f64,
    pub exit_price:  f64,
    pub qty:         f64,
    pub pnl:         f64,         // gerçekleşmiş kâr/zarar (USDT)
    /// Marjin bazlı yüzde: pnl / (entry×qty/leverage) × 100
    /// Örn: 1% fiyat hareketi × 8x kaldıraç = %8 pnl_pct
    pub pnl_pct:     f64,
    pub exit_reason: String,      // "SL" | "TP" | "trailing_sl" | "sinyal"
    pub closed_at:   String,      // UTC zaman damgası
    /// Uygulanan kaldıraç. Eski snapshot'lar için serde default = 1.0
    #[serde(default = "default_leverage")]
    pub leverage:    f64,
    /// Girişteki statik SL fiyatı. Eski snapshot'larda 0.0
    #[serde(default)]
    pub sl_price:    f64,
    /// Girişteki statik TP fiyatı. Eski snapshot'larda 0.0
    #[serde(default)]
    pub tp_price:    f64,
    /// Pozisyon açılış UTC damgası. Eski kayıtlarda boş string.
    #[serde(default)]
    pub opened_at:   String,
    /// Motor türü: Regular | Scalp | Swing. Eski kayıtlarda Regular.
    #[serde(default)]
    pub trade_type:  crate::robot::scalp_swing::TradeType,
    /// Giriş emrinde ödenen komisyon (USD). Eski snapshot'larda 0.0.
    #[serde(default)]
    pub entry_commission: f64,
    /// Çıkış emrinde ödenen komisyon (USD). Eski snapshot'larda 0.0.
    #[serde(default)]
    pub exit_commission:  f64,
    /// Toplam slippage + spread + market impact (USD). Eski snapshot'larda 0.0.
    #[serde(default)]
    pub slippage_usd:     f64,
    /// Giriş anındaki RSI değeri — TradePatternClassifier eğitimi için. Eski kayıtlarda 0.0.
    #[serde(default)]
    pub entry_rsi:        f64,
    /// Giriş anındaki ATR% değeri — TradePatternClassifier eğitimi için. Eski kayıtlarda 0.0.
    #[serde(default)]
    pub entry_atr_pct:    f64,

    // ── Feature Engineering (Strateji Scorer + RL girdisi) ───────────────
    /// Kapanış anındaki ADX rejimi (0=Ranging,1=Neutral,2=Trending,3=Volatile).
    #[serde(default)]
    pub close_adx_regime: u8,
    /// Kapanış anındaki futures funding rate (0 = spot veya bilinmiyor).
    #[serde(default)]
    pub close_funding_rate: f64,
    /// Kapanış anındaki BTC korelasyon katsayısı (-1.0..+1.0; 0 = hesaplanmadı).
    #[serde(default)]
    pub close_btc_corr: f64,
}

impl ClosedTradeData {
    /// Toplam kesinti (komisyon + slippage) USD cinsinden.
    #[inline]
    pub fn total_fees(&self) -> f64 {
        self.entry_commission + self.exit_commission + self.slippage_usd
    }
}

/// Serde default: eski snapshot'larda leverage alanı yoktu → 1.0 (kaldıraçsız) kabul edilir.
#[inline]
fn default_leverage() -> f64 { 1.0 }

/// Kapalı işlem listesi — loop yazar (max 500 girdi), TUI okur.
pub type ClosedTradeLog = Vec<ClosedTradeData>;

/// Mükerrer kayıt kontrolü: aynı trade_key zaten logda var mı?
pub fn is_duplicate_trade(log: &ClosedTradeLog, pos_id: crate::types::PositionId) -> bool {
    log.iter().any(|t| t.pos_id == pos_id)
}

/// Per-sembol adaptif SL/TP + global MA periyotları.
/// Backtest thread writes, trading loop reads – hot-reload sağlar.
#[derive(Debug, Clone)]
pub struct LiveRiskMap {
    /// Sembol başına backtest'ten hesaplanan (sl_pct, tp_pct)
    pub per_symbol: std::collections::HashMap<String, (f64, f64)>,
    /// Global fallback SL/TP (henüz o sembol backtesti yapılmadıysa)
    pub global_sl:   f64,
    pub global_tp:   f64,
    /// HyperOpt'tan gelen en iyi MA periyotları
    pub global_fast: usize,
    pub global_slow: usize,
    /// RSI HyperOpt'tan gelen en iyi parametreler
    pub global_rsi_period: usize,  // varsayılan 14
    pub global_rsi_ob:     f64,    // overbought % — varsayılan 70.0
    pub global_rsi_os:     f64,    // oversold   % — varsayılan 30.0
    /// BB HyperOpt'tan gelen en iyi parametreler
    pub global_bb_period:  usize,  // varsayılan 20
    pub global_bb_std_dev: f64,    // std_dev çarpanı — varsayılan 2.0
    /// MACD HyperOpt'tan gelen en iyi parametreler
    pub global_macd_fast:   usize, // varsayılan 12
    pub global_macd_slow:   usize, // varsayılan 26
    pub global_macd_signal: usize, // varsayılan 9
    /// HTF (üst zaman dilimi) trend yönü — loop yazar, TUI okur.
    /// +1 = Bullish, -1 = Bearish, 0 = Neutral, None = henüz hesaplanmadı
    pub htf_trend_bias: Option<i8>,
    /// HTF filtresi etkin mi? true → HTF biasına zıt sinyaller engellenir.
    /// TUI Settings idx 11 ile toggle edilir, loop her cycle okur.
    pub htf_filter_enabled: bool,
    /// Dinamik kaldıraç — taban değer (varsayılan: 7.0x).
    /// Backtest skoru, HTF trend ve volatiliteye göre [base, max] aralığında ayarlanır.
    pub base_leverage: f64,
    /// Dinamik kaldıraç — üst sınır (varsayılan: 10.0x).
    pub max_leverage: f64,
    /// Son hesaplanan etkin kaldıraç — loop yazar, TUI okur (gerçek zamanlı).
    pub effective_leverage: f64,
    /// Oturum istatistikleri — export/değerleme için loop yazar.
    pub session_closed:  usize,   // toplam kapanan işlem
    pub session_wins:    usize,   // kazanılan işlem
    pub loss_streak:     usize,   // ardışık zarar sayısı
    pub session_rr:      f64,     // ort. kazanç / ort. kayıp oranı
    /// SL cooldown'daki semboller — (sembol, kalan_saniye)
    pub sl_cooldowns: Vec<(String, u64)>,
    /// Stochastic HyperOpt'tan gelen en iyi parametreler
    pub global_stoch_k:  usize,  // K period (varsayılan 6 — DB kanıtlı)
    pub global_stoch_ob: f64,    // OB seviyesi (varsayılan 70.0)
    pub global_stoch_os: f64,    // OS seviyesi (varsayılan 20.0)
    /// EMA HyperOpt parametreleri
    pub global_ema_fast:         usize,  // varsayılan 5
    pub global_ema_slow:         usize,  // varsayılan 20
    /// DONCHIAN HyperOpt parametresi
    pub global_donchian_period:  usize,  // varsayılan 20
    /// WILLIAMS_R HyperOpt parametresi
    pub global_williams_period:  usize,  // varsayılan 14
    /// CCI HyperOpt parametresi
    pub global_cci_period:       usize,  // varsayılan 20
    /// STOCH_RSI HyperOpt parametresi
    pub global_stoch_rsi_period: usize,  // varsayılan 14
    /// SUPERTREND HyperOpt parametreleri
    pub global_supertrend_period: usize, // ATR periyodu, varsayılan 10
    pub global_supertrend_mult:   f64,   // ATR çarpanı, varsayılan 3.0
    /// ICT_FVG HyperOpt parametresi
    pub global_ict_fvg_lookback: usize,  // varsayılan 5
    /// SMC HyperOpt parametresi
    pub global_smc_swing_lb:     usize,  // swing lookback, varsayılan 10
    /// Seans/saat bazlı işlem filtresi
    pub session_filter_enabled:      bool,
    pub session_allowed_hours:       Vec<u8>,
    pub session_blocked_hours:       Vec<u8>,
    pub session_long_preferred_hours: Vec<u8>,
    /// Pattern gate: backtest pattern eşleşmesi olmadan işlem açılmasın.
    /// false (varsayılan) = gate devre dışı, veri birikene kadar açık.
    pub pattern_gate_enabled: bool,
    /// Son hyperopt/backtest'ten tespit edilen en iyi strateji adı.
    pub best_strategy_name: String,
    /// Son ML/HyperOpt skoru — negatifse strateji parametreleri zararlı demektir.
    /// Robotic loop bu değer negatifken Buy/Sell sinyali üretmez.
    pub hyperopt_score: f64,
    /// Robotic loop'un güncel equity'si — diagnostic worker AppState.equity'yi buradan okur.
    pub current_equity: f64,
    /// Spot piyasasında arka arkaya engellenen SELL sinyali sayısı.
    /// AUTO mod bu sayı eşiği aşınca sembol değiştirmeyi zorlar.
    pub spot_sell_blocks: u32,
    /// ML worker tarafından eğitilen LinearRegressor ağırlıkları.
    /// None = henüz eğitim yok → with_defaults() kullanılır.
    /// [f64; 19] = 19 feature ağırlığı (bias ayrı bias_trained'de)
    pub ml_weights: Option<[f64; 19]>,
    pub ml_bias_trained: f64,
    /// GBT worker'dan gelen son tahmin skoru (-1..+1).
    /// Robotic loop bu skoru üçüncü bir voter olarak kullanır.
    pub gbt_last_score: Option<f64>,
    /// Feature drift skoru (0=yok, 1=tam kayma) — ML voter bu değere göre güvenini ölçekler.
    pub ml_drift_score: f64,
    /// OOS kalite metrikleri — ML worker her eğitimde yazar, TUI okur
    pub oos_win_rate:   f64,   // OOS test setindeki doğru yön tahmin oranı (%)
    pub oos_avg_return: f64,   // OOS ortalama getiri (%)
    pub oos_bar_count:  usize, // OOS'ta kullanılan bar sayısı
    pub oos_fold_scores: [f64; 3], // Her fold'un skoru
    /// Ensemble çeşitlilik skoru — LR ve GBT'nin aynı yönde oy verme oranı (0=farklı, 1=aynı)
    /// 0.5-0.7 ideal; 1.0 → çeşitlilik yok (ensemble faydası azalır)
    pub ensemble_agreement: f64,
    /// ML worker şu an eğitim yapıyor mu? TUI bu flag ile "ML çalışıyor..." gösterir.
    pub ml_running: bool,
    /// Strateji parametresi versiyon sayacı — TUI/ML worker değer yazınca artırır.
    /// reload_strategy_params() son gördüğü değerden farklıysa reload yapar, aynıysa atlar.
    pub strategy_params_version: u64,
    /// adaptive_params versiyon sayacı — TUI save_adaptive_params() her çağrıda artırır.
    /// reload_adaptive_params() bu değer değişince disk'ten yeniden yükler.
    pub adaptive_params_version: u64,

    // ── UCB1 StrategyScorer — robotic_loop her tick günceller ────────────────
    /// Strateji × rejim bandit özeti — `StrategyScorer::summary()` çıktısı.
    pub scorer_summary: String,
    /// Hangi tipler devre dışı: "Scalp❌ Swing✓ Reg✓" formatı.
    pub scorer_disabled: String,
    /// Toplam kayıtlı işlem sayısı (UCB1 kol güncelleme sayacı).
    pub scorer_total_n: u32,

    // ── Gaussian NB TradePatternClassifier ──────────────────────────────────
    /// Classifier eğitildi mi? false = cold-start, filtre bypass.
    pub classifier_trained: bool,
    /// Eğitimde kaç kazanan örnek var.
    pub classifier_n_win: usize,
    /// Eğitimde kaç kaybeden örnek var.
    pub classifier_n_loss: usize,
    /// Eğitim buffer'ındaki örnek sayısı (MIN_TRAIN=20'ye kadar sayar).
    pub classifier_buffer_len: usize,

    // ── Kümülatif Equity (session carry-over) ────────────────────────────────
    /// Tüm session'lar boyunca biriken PnL — restart'ta sıfırlanmaz.
    pub cumulative_pnl: f64,
    /// Tarihi equity peak — DD hesabı için referans.
    pub peak_equity: f64,
}

impl LiveRiskMap {
    pub fn new(sl: f64, tp: f64, fast: usize, slow: usize) -> Self {
        Self {
            per_symbol:        std::collections::HashMap::new(),
            global_sl:         sl,
            global_tp:         tp,
            global_fast:       fast,
            global_slow:       slow,
            global_rsi_period: 14,
            global_rsi_ob:     70.0,
            global_rsi_os:     30.0,
            global_bb_period:  20,
            global_bb_std_dev: 2.0,
            global_macd_fast:   12,
            global_macd_slow:   26,
            global_macd_signal: 9,
            htf_trend_bias:     None,
            htf_filter_enabled: true,
            base_leverage:      7.0,
            max_leverage:       10.0,
            effective_leverage: 7.0,
            session_closed:     0,
            session_wins:       0,
            loss_streak:        0,
            session_rr:         1.0,
            sl_cooldowns:       Vec::new(),
            global_stoch_k:     6,
            global_stoch_ob:    70.0,
            global_stoch_os:    20.0,
            global_ema_fast:         5,
            global_ema_slow:         20,
            global_donchian_period:  20,
            global_williams_period:  14,
            global_cci_period:       20,
            global_stoch_rsi_period: 14,
            global_supertrend_period: 10,
            global_supertrend_mult:   3.0,
            global_ict_fvg_lookback: 5,
            global_smc_swing_lb:     10,
            session_filter_enabled:       false,
            session_allowed_hours:        Vec::new(),
            session_blocked_hours:        Vec::new(),
            session_long_preferred_hours: vec![3, 4, 10, 11, 12],
            pattern_gate_enabled:         false,
            best_strategy_name:           String::new(),
            hyperopt_score:               0.0,
            current_equity:               0.0,
            spot_sell_blocks:             0,
            ml_weights:                   None,
            ml_bias_trained:              0.0,
            gbt_last_score:               None,
            ml_drift_score:               0.0,
            oos_win_rate:                 0.0,
            oos_avg_return:               0.0,
            oos_bar_count:                0,
            oos_fold_scores:              [0.0; 3],
            ensemble_agreement:           0.0,
            ml_running:                   false,
            strategy_params_version:      0,
            adaptive_params_version:      0,
            scorer_summary:               String::new(),
            scorer_disabled:              String::new(),
            scorer_total_n:               0,
            classifier_trained:           false,
            classifier_n_win:             0,
            classifier_n_loss:            0,
            classifier_buffer_len:        0,
            cumulative_pnl:               0.0,
            peak_equity:                  sl, // sl burada capital placeholder, gerçek değer loop'tan gelir
        }
    }
    /// O sembolün SL/TP'sini döndür; yoksa global fallback
    pub fn sl_tp(&self, symbol: &str) -> (f64, f64) {
        self.per_symbol.get(symbol).copied().unwrap_or((self.global_sl, self.global_tp))
    }
}

/// Evrimsel AI canlı durumu — loop yazar, TUI okur.
#[derive(Debug, Clone)]
pub struct LiveEvolutionStatus {
    pub evolution_enabled:      bool,
    pub brain_active:           bool,
    pub pop_active:             bool,
    pub genome_id:              String,
    pub genome_fitness:         f64,
    pub genome_trades:          usize,
    pub genome_win_rate:        f64,
    pub evolve_every_n_cycles:  u64,
    pub cycle_id:               u64,
    pub brain_summary:          String,
    pub pop_summary:            String,
}

impl Default for LiveEvolutionStatus {
    fn default() -> Self {
        Self {
            evolution_enabled:     false,
            brain_active:          false,
            pop_active:            false,
            genome_id:             "G0-I0".to_string(),
            genome_fitness:        0.0,
            genome_trades:         0,
            genome_win_rate:       0.0,
            evolve_every_n_cycles: 50,
            cycle_id:              0,
            brain_summary:         String::new(),
            pop_summary:           String::new(),
        }
    }
}

/// Sinyal denetim sayaçları — loop her karar noktasında günceller, TUI/export okur.
#[derive(Debug, Default, Clone)]
pub struct LiveSignalCounts {
    pub buy:                u64,
    pub sell:               u64,
    pub hold:               u64,
    pub blocked_rr:         u64,   // risk/ödül oranı düşük
    pub blocked_volatility: u64,   // volatilite bant dışı
    pub blocked_trend:      u64,   // trend filtresine takıldı
    pub blocked_risk_gate:  u64,   // autonomous risk gate reddetti
    pub ml_below_threshold: u64,   // ML confidence < min eşiği
    pub last_params:        String, // son seçilen grid parametresi özeti
    pub last_block_reason:  String, // son blok nedeni
}

// OpenPosition → robot/position_manager.rs'e taşındı

pub struct RoboticLoopConfig {
    pub interval_secs: u64,
    pub trade_amount: Option<f64>,
    pub interval: String,
    /// Aktif işlem sembolü (ör: "BTCUSDT") — active_trade_target'tan gelir
    pub symbol: String,
    /// Aktif market (Spot / Futures) — active_trade_target'tan gelir
    pub market: Market,
    pub strategy_params: StrategyParams,
    pub candle_limit: usize,
    pub risk_params: RiskParams,
    pub capital: f64,
    pub mode: RunMode,
    pub autonomous_enabled: bool,
    pub quality: TradeQualityConfig,
    pub trade_quality_config_path: Option<String>,
    /// Profil tabanlı pozisyon yönetimi (None = default configs kullan)
    pub position_profile: Option<String>, // "Conservative" | "Balanced" | "Aggressive" | "Scalper" | "SwingTrading"
    /// Güvenlik profili (None = Development mode)
    pub security_profile: Option<String>, // "Development" | "Staging" | "Production" | "Enterprise"
    /// Futures/CFD gibi short açılabilen piyasalar: true → Bullish trendde SELL sinyali bloklanmaz
    pub allows_short: bool,
    /// Başlangıç risk politikası — adaptif ayarları loop'a taşır (None → varsayılan)
    pub initial_risk_policy: Option<RiskGatePolicy>,
    /// Kaldığı yerden devam: snapshot'tan yüklenen cycle_id (0 = sıfırdan başla)
    pub initial_cycle_id: u64,
    /// Snapshot'tan yüklenen AdaptiveBrain — loop başlarken enable_evolution() yerine kullanılır
    pub initial_brain: Option<AdaptiveBrain>,
    /// Snapshot'tan yüklenen PopulationManager — loop başlarken enable_evolution() yerine kullanılır
    pub initial_population: Option<PopulationManager>,
    /// ML sinyalini trade kararında kullan
    pub use_ml_signal: bool,
    /// Snapshot'tan yüklenen açık pozisyonlar — loop başlarında open_positions map'ine kopyalanır
    pub initial_open_positions: LivePositionMap,
    /// Komisyon oranı — PnL hesabında giriş+çıkış işlemlerinden düşülür.
    /// Binance Spot VIP0: 0.001 (%0.10) | Futures VIP0: 0.0004 (%0.04)
    /// 0.0 = komisyon simülasyonu yok (eski davranış)
    pub commission_pct: f64,
    /// Giriş/çıkış fiyatı ayarlaması — spread + slippage + market impact.
    /// None = fiyat ayarlaması yok (commission_pct hâlâ geçerli).
    /// NOT: commission_pct ile çakışmaz — bu sadece fiyat kayması maliyetidir.
    pub execution_cost_config: Option<ExecutionCostConfig>,
    /// Merkezi canlı durum — TUI ile paylaşılan tüm Arc<RwLock<>> kanalları tek noktada.
    /// None = headless / backtest modu (TUI bağlantısı yok).
    pub live_state: Option<SharedTradingState>,
    /// Destek/Direnç filtresi yapılandırması.
    /// `enabled = false` ise S/R filtresi ve SL/TP ayarı devre dışı.
    pub sr_config: crate::robot::sr_detector::SrDetectorConfig,
    /// SQLite DB yolu — üst zaman dilimi (HTF) candle yüklemek için kullanılır.
    /// `None` ise MTF filtresi devre dışı (HTF candle stratejiye `None` geçilir).
    pub db_path: Option<String>,
    /// SL tetiklenince kaç saniye yeni pozisyon açılmasın. Varsayılan: 600 (10 dk).
    /// 0 = cooldown yok; daha kısa = recovery hareketlerini kaçırmaz ama whipsaw riski artar.
    pub sl_cooldown_secs: Option<u64>,
    // ── Pozisyon yönetimi iyileştirmeleri ────────────────────────────────────
    /// B1: Kâr bu R katına ulaşınca SL giriş fiyatına taşı (None = devre dışı)
    /// Örn. 0.5 → yarı risk mesafesi kadar kârdayken breakeven tetiklenir
    pub breakeven_at_rr: Option<f64>,
    /// B2: ATR trailing stop çarpanı (None = sabit trailing_stop_pct kullanılır)
    /// Örn. 1.5 → trailing mesafesi = 1.5 × ATR%
    pub atr_trail_mult: Option<f64>,
    /// B3: TP'de kapatılacak pozisyon oranı (None = tam kapat)
    /// Örn. 0.5 → TP'de %50 kapat, kalan trailing SL ile devam eder
    pub partial_tp_ratio: Option<f64>,
    /// robotic_profiles.json dosya yolu — hot-reload için (None = devre dışı)
    pub robotic_profiles_path: Option<String>,
    /// Uyarlamalı parametre dosya yolu — her N trade kapandığında otomatik güncellenir.
    /// None = devre dışı.
    pub adaptive_params_path: Option<String>,
    /// Kalıcı sembol engelleme listesi — bu semboller için hiç pozisyon açılmaz.
    /// rtc_config.json'dan "blocked_symbols" alanından gelir.
    pub blocked_symbols: Vec<String>,
    /// Aynı sembolde iki pozisyon açılışı arasında zorunlu minimum bekleme (saniye).
    /// None = sınır yok. Whipsaw koruması: hızlı sinyal değişimlerinde tekrar girişi engeller.
    /// Önerilen: spot=300 (5dk), futures=120 (2dk), scalper=60 (1dk).
    pub min_trade_interval_secs: Option<u64>,

    /// Scalp & Swing kısa-vade fırsat motoru konfigürasyonu.
    /// None = devre dışı (sadece Regular loop stratejileri çalışır).
    pub scalp_swing: Option<crate::robot::scalp_swing::ScalpSwingConfig>,

    /// Portföy genelinde aynı anda açık tutulabilecek maksimum pozisyon sayısı.
    /// None = sınır yok. Sembol başına kısıtlardan bağımsız portföy-düzeyi güvencedir.
    pub max_open_positions: Option<usize>,

    /// Giriş anında bookTicker spread'i bu bps değerini aşarsa giriş engellenir.
    /// None = spread guard devre dışı. Önerilen: 10–15 bps.
    pub max_spread_bps: Option<f64>,

    /// UCB1 strateji scorer durumu — restart'ta sıfırlanmaması için diske kaydedilir.
    /// None = devre dışı (scorer memory her restart'ta temizlenir).
    pub scorer_state_path: Option<String>,

    /// Gaussian NB classifier + eğitim buffer'ı — restart sonrası ML yeniden öğrenmeden çalışsın.
    /// None = devre dışı.
    pub classifier_state_path: Option<String>,
}

/// Tek bir işlem türü (Regular/Scalp/Swing) için kümülatif maliyet.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TypeCosts {
    pub commission:   f64,
    pub spread:       f64,
    pub slippage:     f64,
    pub impact:       f64,
    pub total_usd:    f64,
    pub trade_count:  usize,
}

impl TypeCosts {
    #[inline]
    pub fn avg_per_trade(&self) -> f64 {
        if self.trade_count == 0 { 0.0 } else { self.total_usd / self.trade_count as f64 }
    }
}

/// Kümülatif işlem maliyetleri — TUI'da maliyet özeti paneli için.
/// Regular/Scalp/Swing türleri ayrı izlenir; total_* alanları tüm türlerin toplamını verir.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CumulativeTradingCosts {
    // Per-tür kümülatifler (yeni)
    #[serde(default)]
    pub regular: TypeCosts,
    #[serde(default)]
    pub scalp:   TypeCosts,
    #[serde(default)]
    pub swing:   TypeCosts,

    // Eski düz toplam alanlar — geriye uyumluluk + özet için korundu,
    // yazım sitelerinde per-tür alanla birlikte güncellenmelidir.
    pub total_commission:    f64,
    pub total_spread:        f64,
    pub total_slippage:      f64,
    pub total_impact:        f64,
    pub total_cost_usd:      f64,
    pub trade_count:         usize,
    pub avg_cost_per_trade:  f64,
}

impl CumulativeTradingCosts {
    /// Verilen türün TypeCosts'una mutable referans.
    pub fn by_type_mut(&mut self, tt: crate::robot::scalp_swing::TradeType) -> &mut TypeCosts {
        use crate::robot::scalp_swing::TradeType;
        match tt {
            TradeType::Regular => &mut self.regular,
            TradeType::Scalp   => &mut self.scalp,
            TradeType::Swing   => &mut self.swing,
        }
    }

    /// Tür-tabanlı kayıt: per-tür bucket ve global toplam aynı anda güncellenir.
    pub fn record(&mut self,
        tt:         crate::robot::scalp_swing::TradeType,
        commission: f64,
        spread:     f64,
        slippage:   f64,
        impact:     f64,
    ) {
        let bucket = self.by_type_mut(tt);
        bucket.commission  += commission;
        bucket.spread      += spread;
        bucket.slippage    += slippage;
        bucket.impact      += impact;
        bucket.total_usd   += commission + spread + slippage + impact;
        bucket.trade_count += 1;

        self.total_commission   += commission;
        self.total_spread       += spread;
        self.total_slippage     += slippage;
        self.total_impact       += impact;
        self.total_cost_usd     += commission + spread + slippage + impact;
        self.trade_count        += 1;
        self.avg_cost_per_trade = if self.trade_count == 0 { 0.0 }
                                  else { self.total_cost_usd / self.trade_count as f64 };
    }
}

// TradeQualityConfig → robot/signal_evaluator.rs'e taşındı

// ── Pipeline Sağlık Monitörü ─────────────────────────────────────────────────

/// Tespit edilen anomalinin türü ve ciddiyeti.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnomalyKind {
    StaleCandles,
    EvolutionStuck,
    HighDrift,
    ConsecLosses,
    FsmBlocked,
    FundingStale,
    DbDisconnected,
    LowWinRate,
    /// API circuit breaker uzun süre açık kaldı
    CircuitBreakerOpen,
    /// Bir pozisyon çok uzun süredir açık (SL/TP ihlal edilmemiş ama gecikmiş)
    PositionStuck,
    /// N saattir non-Hold sinyal üretilemiyor (sinyal kuruluğu)
    SignalDrought,
}

/// Pipeline zinciri adım durumu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainStepStatus {
    Ok,      // taze ve başarılı
    Running, // şu an çalışıyor
    Stale,   // aralık doldu / tetiklenecek
    Failed,  // son çalışma başarısız
    Pending, // hiç çalışmadı
}

/// Pipeline zincirindeki tek bir adımın anlık durumu.
#[derive(Debug, Clone)]
pub struct PipelineChainStep {
    pub id:            &'static str,
    pub label:         String,
    pub status:        ChainStepStatus,
    pub last_run_secs: u64,   // saniye önce; 999_999 = hiç çalışmadı
    pub interval_secs: u64,   // 0 = tetikleyici bazlı (sabit aralık yok)
    pub overdue_secs:  i64,   // pozitif = gecikme, negatif = kalan süre
    pub heal_count:    u32,   // bu oturumda otomatik yeniden tetiklenme sayısı
    pub user_hint:     String,// otomatik düzeltilemezse kullanıcıya gösterilecek talimat
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnomSeverity { Warning, Critical }

#[derive(Debug, Clone)]
pub struct PipelineAnomaly {
    pub kind:       AnomalyKind,
    pub severity:   AnomSeverity,
    pub message:    String,
    /// Döngü tarafından otomatik olarak düzeltildi mi?
    pub auto_fixed: bool,
    /// Otomatik düzeltilemiyorsa kullanıcıya gösterilecek ipucu.
    pub fix_hint:   String,
}

/// Tüm pipeline bileşenlerinin anlık sağlık özeti.
/// Loop yazar (her 5 dk + kritik olayda), TUI Tab 8 okur.
#[derive(Debug, Clone)]
pub struct LivePipelineHealth {
    // ── Veri akışı ──────────────────────────────────────────────────
    pub candle_age_secs:   u64,
    pub last_candle_at:    String,
    pub ws_stale:          bool,      // age > 90 sn

    // ── DB ──────────────────────────────────────────────────────────
    pub db_connected:      bool,

    // ── Funding rate (sadece Futures/CoinM) ─────────────────────────
    pub funding_rate:       f64,
    pub funding_age_secs:   u64,
    pub funding_applicable: bool,

    // ── Kelly criterion ──────────────────────────────────────────────
    pub kelly_active:          bool,
    pub kelly_scale:           f64,
    pub kelly_trades_so_far:   usize,
    pub kelly_min_trades:      usize,

    // ── Evrim ────────────────────────────────────────────────────────
    pub evolution_cycle:        u64,
    pub evolution_stuck_count:  u32,
    pub last_evolution_trigger: String,
    pub mini_evol_count:        u32,

    // ── Drift ────────────────────────────────────────────────────────
    pub drift_score:     f64,
    pub drift_threshold: f64,

    // ── Kayıp serisi ─────────────────────────────────────────────────
    pub loss_streak:           usize,
    pub loss_streak_threshold: usize,

    // ── Oturum istatistikleri ────────────────────────────────────────
    pub session_wins:    usize,
    pub session_closed:  usize,

    // ── Aktif anomaliler + onarım günlüğü ────────────────────────────
    pub anomalies:   Vec<PipelineAnomaly>,
    pub repair_log:  std::collections::VecDeque<String>,  // son 20 kayıt

    // ── Tam pipeline zincir adım durumları (chain monitor yazar) ────
    pub chain_steps: Vec<PipelineChainStep>,

    // ── TUI → Loop komut flag'leri ───────────────────────────────────
    pub force_mini_evolution:  bool,
    pub force_funding_refresh: bool,
}

impl Default for LivePipelineHealth {
    fn default() -> Self {
        Self {
            candle_age_secs:        999,
            last_candle_at:         "—".to_string(),
            ws_stale:               false,
            db_connected:           true,
            funding_rate:           0.0,
            funding_age_secs:       0,
            funding_applicable:     false,
            kelly_active:           false,
            kelly_scale:            1.0,
            kelly_trades_so_far:    0,
            kelly_min_trades:       20,
            evolution_cycle:        0,
            evolution_stuck_count:  0,
            last_evolution_trigger: "—".to_string(),
            mini_evol_count:        0,
            drift_score:            0.0,
            drift_threshold:        0.7,
            loss_streak:            0,
            loss_streak_threshold:  3,
            session_wins:           0,
            session_closed:         0,
            anomalies:              Vec::new(),
            repair_log:             std::collections::VecDeque::new(),
            chain_steps:            Vec::new(),
            force_mini_evolution:   false,
            force_funding_refresh:  false,
        }
    }
}

impl LivePipelineHealth {
    /// Onarım günlüğüne zaman damgalı kayıt ekle (maksimum 20 satır tutulur).
    pub fn log_repair(&mut self, msg: &str) {
        let ts = chrono::Local::now().format("%H:%M:%S").to_string();
        self.repair_log.push_back(format!("[{}] {}", ts, msg));
        while self.repair_log.len() > 20 {
            self.repair_log.pop_front();
        }
    }
}

/// Ardışık SL sayısına göre SCP/SWG cooldown süresi (saniye).
/// Her SL'de artar, TP'de sıfırlanır.
fn ss_cooldown_secs(trade_type: crate::robot::scalp_swing::TradeType, consecutive: u32) -> u64 {
    use crate::robot::scalp_swing::TradeType;
    match trade_type {
        TradeType::Scalp => match consecutive {
            1..=2 => 900,    // 15 dk
            3..=4 => 1800,   // 30 dk
            _     => 3600,   // 1 saat
        },
        TradeType::Swing => match consecutive {
            1..=2 => 7200,   // 2 saat
            3..=4 => 14400,  // 4 saat
            _     => 28800,  // 8 saat
        },
        TradeType::Regular => 0,
    }
}

// ── Merkezi Canlı Durum ──────────────────────────────────────────────────────
/// Tüm canlı veri kanallarını tek noktada toplayan yapı.
/// Her alan kendi lock'unu taşır — bağımsız güncellenebilir.
/// `SharedTradingState` = Arc<TradingStateInner>; hem loop hem TUI aynı Arc'ı klonlar.
pub struct TradingStateInner {
    pub live_risk:            std::sync::Arc<std::sync::RwLock<LiveRiskMap>>,
    pub live_price:           std::sync::Arc<std::sync::RwLock<LivePriceData>>,
    pub live_positions:       std::sync::Arc<std::sync::RwLock<LivePositionMap>>,
    pub live_strategy:        std::sync::Arc<std::sync::RwLock<String>>,
    pub live_regime_strategy: std::sync::Arc<std::sync::RwLock<String>>,
    pub live_evolution:       std::sync::Arc<std::sync::RwLock<LiveEvolutionStatus>>,
    pub live_active_symbol:   std::sync::Arc<std::sync::RwLock<String>>,
    pub live_signal_counts:   std::sync::Arc<std::sync::RwLock<LiveSignalCounts>>,
    pub live_trade_count:     std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub live_closed_trades:   std::sync::Arc<std::sync::RwLock<ClosedTradeLog>>,
    pub live_execution_costs: std::sync::Arc<std::sync::RwLock<CumulativeTradingCosts>>,
    /// Sembol → S/R bölgeleri haritası — tüm aktif semboller için TUI görüntüleme.
    pub live_sr_zones: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, Vec<crate::robot::sr_detector::SrZone>>>>,
    /// Pipeline sağlık monitörü — Tab 8 için anomali + onarım verisi.
    pub live_pipeline: std::sync::Arc<std::sync::RwLock<LivePipelineHealth>>,
}

/// Paylaşımlı canlı durum tutacağı — klonlayarak paylaşılır (Arc semantiği).
pub type SharedTradingState = std::sync::Arc<TradingStateInner>;

// ── TradingStateInner yardımcı metodlar ──────────────────────────────────────
// Write/read guard boilerplate yerine closure alır; lock başarısız olursa sessizce geçer.
impl TradingStateInner {
    pub fn update_price(&self, f: impl FnOnce(&mut LivePriceData)) {
        if let Ok(mut g) = self.live_price.write() { f(&mut g); }
    }
    pub fn update_evolution(&self, f: impl FnOnce(&mut LiveEvolutionStatus)) {
        if let Ok(mut g) = self.live_evolution.write() { f(&mut g); }
    }
    pub fn update_signal_counts(&self, f: impl FnOnce(&mut LiveSignalCounts)) {
        if let Ok(mut g) = self.live_signal_counts.write() { f(&mut g); }
    }
    pub fn update_positions(&self, f: impl FnOnce(&mut LivePositionMap)) {
        if let Ok(mut g) = self.live_positions.write() { f(&mut g); }
    }
    pub fn read_positions<T>(&self, f: impl FnOnce(&LivePositionMap) -> T) -> Option<T> {
        self.live_positions.read().ok().map(|g| f(&g))
    }
    pub fn try_read_risk<T>(&self, f: impl FnOnce(&LiveRiskMap) -> T) -> Option<T> {
        self.live_risk.try_read().ok().map(|g| f(&g))
    }
    pub fn try_write_risk(&self, f: impl FnOnce(&mut LiveRiskMap)) {
        if let Ok(mut g) = self.live_risk.try_write() { f(&mut g); }
    }
    pub fn update_costs(&self, f: impl FnOnce(&mut CumulativeTradingCosts)) {
        if let Ok(mut g) = self.live_execution_costs.write() { f(&mut g); }
    }
    pub fn set_regime_strategy(&self, name: &str) {
        if let Ok(mut g) = self.live_regime_strategy.write() { *g = name.to_string(); }
    }
    pub fn update_pipeline(&self, f: impl FnOnce(&mut LivePipelineHealth)) {
        if let Ok(mut g) = self.live_pipeline.try_write() { f(&mut g); }
    }
    pub fn read_pipeline_flags(&self) -> (bool, bool) {
        self.live_pipeline.try_read()
            .map(|g| (g.force_mini_evolution, g.force_funding_refresh))
            .unwrap_or((false, false))
    }
    pub fn set_strategy(&self, name: &str) {
        if let Ok(mut g) = self.live_strategy.write() { *g = name.to_string(); }
    }
    pub fn get_strategy(&self) -> String {
        self.live_strategy.try_read().ok().map(|g| g.clone()).unwrap_or_default()
    }
    pub fn inc_trade_count(&self) {
        self.live_trade_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    /// Kapalı işlem geçmişine ekle (max 500, PositionId dedup).
    pub fn append_closed_trade(&self, trade: ClosedTradeData) {
        if let Ok(mut log) = self.live_closed_trades.write() {
            if !is_duplicate_trade(&log, trade.pos_id) {
                log.push(trade);
                if log.len() > 500 { log.remove(0); }
            }
        }
    }
    /// Belirtilen sembolün S/R bölgelerini güncelle (her loop cycle'ında çağrılır).
    pub fn update_sr_zones(&self, symbol: &str, f: impl FnOnce(&mut Vec<crate::robot::sr_detector::SrZone>)) {
        if let Ok(mut g) = self.live_sr_zones.write() {
            f(g.entry(symbol.to_string()).or_default());
        }
    }

    /// Pozisyonu haritadan kaldır; Some = bu loop kapattı (dedup koruması), None = zaten kaldırılmış.
    pub fn remove_position(&self, key: &str) -> bool {
        self.live_positions.write().ok()
            .map(|mut lm| lm.remove(key).is_some())
            .unwrap_or(true) // live_state yoksa her zaman kayıt yapılır
    }

    /// UUID eşleşmesi kontrolüyle pozisyon kaldır.
    /// Aynı sembol+market anahtarında farklı bir pozisyon varsa (yeni pozisyon açıldı)
    /// silmeden false döner — "UUID mismatch, yeni pozisyon korundu" demek.
    /// Lock başarısız olursa false döner (eskiden true dönüyordu → silent data loss).
    pub fn remove_position_by_id(&self, key: &str, expected_pos_id: crate::types::PositionId) -> bool {
        match self.live_positions.write() {
            Ok(mut lm) => {
                if lm.get(key).map(|p| p.pos_id == expected_pos_id).unwrap_or(false) {
                    lm.remove(key).is_some()
                } else {
                    false // anahtar yok veya farklı UUID → silme
                }
            }
            Err(_) => false, // lock zehirlendi; caller log'lamalı, sessizce "removed" sayma
        }
    }
}

// ── LivePositionData kolaylaştırıcı ──────────────────────────────────────────
impl LivePositionData {
    /// `OpenPosition`'dan TUI anlık görüntüsü oluştur (boilerplate struct init'i kaldırır).
    pub(crate) fn from_pos(
        pos: &crate::robot::position_manager::OpenPosition,
        market: crate::types::Market,
        current_price: f64,
    ) -> Self {
        Self {
            pos_id:               pos.id,
            symbol:               pos.symbol.clone(),
            market,
            is_long:              pos.is_long,
            entry_price:          pos.entry_price,
            qty:                  pos.qty,
            static_sl:            pos.static_sl,
            static_tp:            pos.static_tp,
            trailing_sl:          pos.trailing_sl,
            trailing_pct:         pos.trailing_pct,
            best_price:           pos.best_price,
            current_price,
            leverage:             pos.leverage,
            liquidation_price:    pos.liquidation_price,
            breakeven_triggered:  pos.breakeven_triggered,
            partial_tp_triggered: pos.partial_tp_triggered,
            atr_trail_active:     pos.atr_trail_mult.is_some(),
            opened_at:            pos.opened_at.clone(),
            tp1_price:            pos.tp1_price,
            tp1_triggered:        pos.tp1_triggered,
            trade_type:           pos.trade_type,
        }
    }
}

// ── RoboticLoopConfig erişim kolaylaştırıcıları ───────────────────────────────
impl RoboticLoopConfig {
    /// `live_state` mevcutsa closure çalıştır; yoksa sessizce geç.
    #[inline]
    pub fn with_live(&self, f: impl FnOnce(&TradingStateInner)) {
        if let Some(ref s) = self.live_state { f(s); }
    }
    /// `live_state` mevcutsa `T` döndür, yoksa `None`.
    #[inline]
    pub fn map_live<T>(&self, f: impl FnOnce(&TradingStateInner) -> T) -> Option<T> {
        self.live_state.as_ref().map(|s| f(s))
    }
}

// ── Sinyal izleme olayı ───────────────────────────────────────────────────────
/// `count_signal()` ile tek satırda sinyal kaydı.
pub enum SignalMetric {
    Buy,
    Sell,
    Hold,
    BlockedRr(std::borrow::Cow<'static, str>),
    BlockedVolatility(std::borrow::Cow<'static, str>),
    BlockedTrend(&'static str),
    BlockedRiskGate(String),
    LastParams(String),
}

// ── Fiyat çözümleme ───────────────────────────────────────────────────────────
/// Öncelik: candle_close > live_arc > last_known — ilk > 0 olan kullanılır.
#[inline]
fn resolve_price(candle_close: f64, live_arc: f64, last_known: f64) -> f64 {
    [candle_close, live_arc, last_known]
        .into_iter()
        .find(|&p| p > 0.0)
        .unwrap_or(0.0)
}

/// Giriş veya çıkış fiyatını spread + slippage + market impact ile ayarla.
/// `is_buy = true` → alış (fiyat yukarı kayar), `false` → satış (fiyat aşağı kayar).
#[cfg(not(target_arch = "wasm32"))]
#[inline]
fn adjusted_price(base: f64, qty: f64, is_buy: bool, ec: &ExecutionCostConfig) -> f64 {
    let notional = base * qty;
    let adj = ec.market_impact_pct(notional) / 100.0
            + ec.spread_pct / 200.0   // half-spread
            + ec.slippage_pct / 100.0;
    if is_buy { base * (1.0 + adj) } else { base * (1.0 - adj) }
}

/// `start()` süresince yaşayan tüm değiştirilebilir yerel durum.
/// Metodlara tek bağımsız değişken olarak geçilerek borç denetçisi sorunlarını ortadan kaldırır.
#[cfg(not(target_arch = "wasm32"))]
struct LoopState {
    // ── Sermaye & equity ──────────────────────────────────────────────────────
    capital:            f64,
    current_equity:     f64,
    day_start_equity:   f64,
    peak_equity:        f64,
    cumulative_pnl:     f64,
    // ── Tick başına sıfırlanan sayaçlar ──────────────────────────────────────
    total_pnl:          f64,
    total_trades:       usize,
    win_trades:         usize,
    // ── Oturum geneli kümülatif sayaçlar (asla sıfırlanmaz) ──────────────────
    session_closed:     usize,  // toplam kapanan işlem
    session_wins:       usize,  // karlı kapanan işlem
    loss_streak:        usize,  // ardışık zarar sayısı
    session_profit:     f64,    // karlı işlemlerin toplam kârı  (RR payı)
    session_loss:       f64,    // zararlı işlemlerin toplam zararı abs (RR paydası)
    // ── Akış kontrolü ────────────────────────────────────────────────────────
    stop_loop:          bool,
    error_count:        usize,
    fetch_backoff_secs: u64,
    // ── Adaptif kalite eşikleri ────────────────────────────────────────────
    min_rr:             f64,
    volatility_min_pct: f64,
    volatility_max_pct: f64,
    // ── Açık pozisyon takibi ─────────────────────────────────────────────────
    open_positions:     std::collections::HashMap<crate::types::PositionId, OpenPosition>,
    // ── SL sonrası cooldown (sembol → son SL zamanı) ──────────────────────────
    sl_cooldown_map:    std::collections::HashMap<String, std::time::Instant>,
    sl_cooldown_secs:   u64,  // config'den gelir; varsayılan 600 sn (10 dk)
    // ── Günlük sembol SL sayacı (sembol → (sayı, tarih)) ────────────────────
    daily_sl_map:       std::collections::HashMap<String, (u32, chrono::NaiveDate)>,
    // ── Sembol bazlı ardışık kayıp sayacı (sembol → ardışık zarar) ──────────
    symbol_consec_loss: std::collections::HashMap<String, u32>,
    // ── Sembol bazlı artan cooldown süresi (sembol → saniye) ─────────────────
    // 3+ ardışık zarar → 7200 sn (2 saat), 5+ → 86400 sn (24 saat)
    symbol_cooldown_secs: std::collections::HashMap<String, u64>,
    // ── Global loss streak decay — son kayıp zamanı (kalıcı kilitlenmeyi önler) ─
    // 3 saatte bir loss_streak -= 1 yapılır; streak ≥ eşik ama uzun süredir kayıp
    // yoksa sistem yeniden trade açabilir.
    last_loss_time: Option<std::time::Instant>,
    last_short_loss_time: Option<std::time::Instant>,
    // ── Flip sonrası kısa cooldown (sembol → flip zamanı, sabit 60 sn) ───────
    flip_cooldown_map:  std::collections::HashMap<String, std::time::Instant>,
    // ── TP kazancı sonrası ters yön bloğu (sembol → (kazanılan_yön_long, zaman)) ─
    // TP ile kapanan sembolde 2 saat ters yön açılmaz (ATUSDT LONG→SHORT pattern)
    tp_win_dir_map:     std::collections::HashMap<String, (bool, std::time::Instant)>,
    // ── Startup cooldown ─────────────────────────────────────────────────────
    startup_time:       std::time::Instant,  // döngü başlangıcı
    startup_cooldown_secs: u64,              // ilk N saniye işlem yok
    // ── Kontrolcüler & guardrail'lar ──────────────────────────────────────────
    autonomous_controller: AutonomousController,
    api_circuit_breaker:   CircuitBreaker,
    dd_monitor:            DrawdownMonitor,
    risk_gate:             RiskGate,
    recovery_supervisor:   RecoverySupervisor,
    /// Döngü boyunca açık tutulan DB bağlantısı — her tick'te yeniden açılmaz.
    /// None = db_path yapılandırılmamış veya ilk açılış başarısız.
    db_conn: Option<rusqlite::Connection>,
    /// Son reload'da görülen LiveRiskMap::strategy_params_version.
    /// reload_strategy_params() bu değer değişmemişse lock almadan atlar.
    last_reloaded_params_version: u64,
    /// Son reload'da görülen LiveRiskMap::adaptive_params_version.
    /// reload_adaptive_params() bu değer değişmemişse disk okumaz.
    last_reloaded_adaptive_version: u64,
    /// Feature drift detector — piyasa rejimi kaymasını ölçer
    drift_detector:        crate::robot::ml_engine::DriftDetector,
    /// Per-interval candle önbelleği — REST/DB tekrar çağrısını önler
    candle_cache:          std::sync::Arc<std::sync::Mutex<CandleCache>>,
    /// ranked_cache: son candle ts → strateji sıralaması
    /// rank_strategies_for_interval: 16 strateji × 195 candle = ~3000 generate_signal/tick
    /// Yeni candle kapanana kadar (timestamp değişene kadar) yeniden hesaplanmaz.
    ranked_cache:          Option<(chrono::DateTime<chrono::Utc>, Vec<(String, crate::robot::optimizer::CompositeScore)>)>,
    /// fv_cache: son candle ts → FeatureVector
    /// FeatureExtractor::extract 19 indikatörü her tick yeniden hesaplar.
    /// Yeni candle kapanana kadar önbellekten okunur.
    fv_cache:              Option<(chrono::DateTime<chrono::Utc>, FeatureVector)>,
    /// grid_search_cache: son candle ts → (best_params, best_score, best_raw_score)
    /// HyperOptimizer::simulate_score_htf 6 param × ~195 candle = ~1170 hesap/tick.
    /// Candle timestamp değişmedikçe sonuç sabittir; önbellekten okunur.
    grid_search_cache:     Option<(chrono::DateTime<chrono::Utc>, crate::types::StrategyParams, f64, f64)>,
    /// WAL checkpoint sayacı — her 60 tick'te bir `PRAGMA wal_checkpoint(TRUNCATE)` çalıştırılır.
    wal_tick_counter:      u32,
    /// B1/B2/B3 optimizasyon sayacı — her 240 tick'te (≈4 saat 1h TF) bir çalışır.
    opt_tick_counter:      u32,
    /// Arka planda çalışan optimizasyon görevi tamamlanınca buraya sonuç düşer.
    /// `None` = bekleyen görev yok.
    opt_result_rx:         Option<tokio::sync::oneshot::Receiver<
                               crate::robot::backtester::engine::PosOptResult>>,
    /// Uyarlamalı trade parametreleri — her N trade kapandığında otomatik güncellenir.
    adaptive_params:       AdaptiveTradeParams,
    /// Ardışık SHORT kayıp sayacı — adaptive_params.auto_adjust() için.
    short_loss_streak:     u32,
    /// HOLD log kısıtlayıcısı — aynı mesajın her tick tekrarlanmasını önler (60 sn eşiği).
    hold_log_throttle:     Option<std::time::Instant>,
    /// Açılışta kaydedilen (MarketRegime, strateji) — kapanışta gerçek PnL ile learn_from_trade çağrısı için.
    /// PositionId → (entry rejimi, entry stratejisi); kapanışta remove edilerek kullanılır.
    pending_evo_data:      std::collections::HashMap<crate::types::PositionId, (crate::evolution::MarketRegime, String)>,
    /// Bu session içinde kapatılan pozisyon UUID'leri — aynı UUID'nin iki kez kapanmasını önler.
    /// process_orphans, check_live_sl_tp ve startup scan her kapanışta buraya ekler.
    closed_position_ids:   std::collections::HashSet<crate::types::PositionId>,
    /// Sembol → son pozisyon açılış zamanı — whipsaw önleme (min_trade_interval_secs).
    /// Aynı sembolde iki işlem arasında en az N saniye zorunlu kılınır.
    last_trade_time:       std::collections::HashMap<String, std::time::Instant>,
    /// Futures/CoinM: son çekilen funding rate + fetch zamanı.
    /// 5 dakika geçmedikçe REST'e tekrar gidilmez.
    funding_rate_cache:    Option<(crate::types::FundingRatePoint, std::time::Instant)>,
    /// Son periyodik sağlık kontrolü zamanı — her 5 dakikada bir çalışır.
    last_health_check:     std::time::Instant,
    /// Evrim takıp sayacı — cycle_id aynı kalırsa stale uyarısı verilir.
    health_last_cycle_id:  u64,
    health_same_cycle_count: u32,
    /// FSM art arda kaç sağlık kontrolünde `can_trade()=false` gördü — auto-recover için.
    fsm_blocked_health_count: u32,
    /// Circuit breaker art arda kaç sağlık kontrolünde Open gördü — force-reset için.
    cb_open_health_count: u32,
    /// En son non-Hold (gerçek) sinyal üretilen an — sinyal kuruluğu tespiti için.
    last_nonhold_signal_at: Option<std::time::Instant>,

    // ── Scalp / Swing oturum istatistikleri ──────────────────────────────────
    /// Scalp işlemleri için oturum geneli istatistikler — otonom ayarlama girdisi
    scalp_stats: crate::robot::scalp_swing::ScalpSwingStats,
    /// Swing işlemleri için oturum geneli istatistikler — otonom ayarlama girdisi
    swing_stats: crate::robot::scalp_swing::ScalpSwingStats,
    /// Günlük scalp/swing toplam kaybı takibi — max_daily_loss_pct için (gün başı equity)
    scalp_swing_day_start_equity: f64,
    /// SCP/SWG SL sonrası tip+sembol bazlı cooldown — key: "SYMBOL_SCP" veya "SYMBOL_SWG"
    /// Değer: (cooldown başlangıcı, toplam süre_sn) — ardışık SL'ye göre artar.
    scalp_swing_sl_cooldown: std::collections::HashMap<String, (std::time::Instant, u64)>,
    /// Ardışık SCP/SWG SL sayacı — key: "SYMBOL_SCP" | "SYMBOL_SWG"; TP'de sıfırlanır.
    scalp_swing_consecutive_sl: std::collections::HashMap<String, u32>,
    /// Duplicate-emir koruması için son giriş zamanı — key: "SYMBOL_LONG"|"SYMBOL_SHORT"
    last_entry_at: std::collections::HashMap<String, std::time::Instant>,

    // ── ADX/HMM rejim & cool-off ─────────────────────────────────────────────
    /// ADX tabanlı piyasa rejimi — sadece rejim değişince strateji filtresi güncellenir (overfitting önlemi).
    adx_regime: crate::market_regime::AdxRegime,
    /// 3 ardışık SL → 1 saatlik "Sadece İzleme" modu bitiş zamanı.
    cooloff_until: Option<std::time::Instant>,

    // ── Dinamik Karantina & BTC/ETH Çapası ──────────────────────────────────
    /// Sembol → son 10 işlem PnL geçmişi (kümülatif negatif → 24h karantina).
    symbol_pnl_history: std::collections::HashMap<String, std::collections::VecDeque<f64>>,
    /// Dinamik karantina listesi — sembol → karantina bitiş zamanı.
    dynamic_blacklist: std::collections::HashMap<String, std::time::Instant>,
    /// BTC/ETH korelasyon anchor cache — (candles, son_fetch_zamanı); 5 dk TTL.
    btc_anchor_cache: Option<(Vec<crate::types::Candle>, std::time::Instant)>,
    eth_anchor_cache: Option<(Vec<crate::types::Candle>, std::time::Instant)>,

    // ── Ticaret Örüntüsü Sınıflandırıcısı ───────────────────────────────────
    /// Gaussian Naive Bayes — başarılı geçmiş işlemlerden kazanan örüntüleri öğrenir.
    pattern_classifier: crate::robot::ml_engine::TradePatternClassifier,
    /// Eğitim buffer'ı — (öznitelikler, label) çiftleri; MIN_TRAIN=20 sonrası retrain.
    classifier_buffer: Vec<([f64; 7], f64)>,

    // ── Strateji Scorer (UCB1 bandit + özerk kontrol) ────────────────────
    /// Strateji × rejim UCB1 bandit motoru.
    /// Her kapanan işlemde beslenir; her EVAL_EVERY işlemde özerk karar üretir.
    strategy_scorer: crate::robot::strategy_scorer::StrategyScorer,

    // ── High-Frequency Blacklist ──────────────────────────────────────────
    /// Sembol → son 1 saatteki slippage/TP-gap hatalarının zaman damgaları.
    /// 2+ olay → sembol 4 saatliğine dynamic_blacklist'e eklenir.
    hf_error_log: std::collections::HashMap<String, std::collections::VecDeque<std::time::Instant>>,

    // ── Rolling Slippage Skoru ────────────────────────────────────────────
    /// Sembol → son 20 kapanıştaki fill sapması (bps cinsinden, + = kötü fill).
    /// Ortalama > 15 bps ise uyarı loglanır.
    symbol_slip_bps: std::collections::HashMap<String, std::collections::VecDeque<f64>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl LoopState {
    /// Tick başına sıfırla.
    fn reset_tick_counters(&mut self) {
        self.total_pnl    = 0.0;
        self.total_trades = 0;
        self.win_trades   = 0;
        self.stop_loop    = false;
    }

    /// PnL'yi equity'e yansıt; session win rate ve loss streak güncelle.
    fn record_pnl(&mut self, pnl: f64) {
        self.record_pnl_dir(pnl, true); // yön bilinmiyorsa LONG varsay (short_loss_streak etkilenmez)
    }

    /// Startup SL/TP kapanışı için PnL kaydet — equity/cumulative güncellenir ama
    /// loss_streak ARTMAZ. Önceki session'dan kalan pozisyonlar mevcut session'ın
    /// strateji kararlarını (cooldown, pause) etkilememeli.
    fn record_pnl_startup(&mut self, pnl: f64) {
        self.total_pnl      += pnl;
        self.cumulative_pnl += pnl;
        self.current_equity  = (self.capital + self.cumulative_pnl).max(0.0);
        if self.current_equity > self.peak_equity {
            self.peak_equity = self.current_equity;
        }
        self.session_closed += 1;
        if pnl > 0.0 {
            self.session_wins   += 1;
            self.session_profit += pnl;
        } else {
            self.session_loss += pnl.abs();
            // loss_streak intentionally NOT incremented — bkz. record_pnl_startup doc
        }
    }

    /// HF hata kaydeder (slippage veya TP-gap).
    /// Son 1 saat içindeki olay sayısı ≥ 2 ise sembol 4 saatliğine dynamic_blacklist'e alınır.
    /// `true` döner → yeni yasak eklendi, `false` → sadece kaydedildi.
    fn record_hf_error(&mut self, symbol: &str) -> bool {
        let now     = std::time::Instant::now();
        let window  = std::time::Duration::from_secs(3600);   // 1 saat
        let ban_dur = std::time::Duration::from_secs(14400);  // 4 saat
        let log = self.hf_error_log
            .entry(symbol.to_string())
            .or_insert_with(std::collections::VecDeque::new);
        log.retain(|&t| now.duration_since(t) < window);
        log.push_back(now);
        if log.len() >= 2 {
            self.dynamic_blacklist.insert(symbol.to_string(), now + ban_dur);
            log.clear(); // ban sonrası sayacı sıfırla
            return true;
        }
        false
    }

    /// Fill sapmasını (bps) sembol başına rolling pencereye kaydeder.
    /// `expected` = hedef fiyat (sl_tp_price), `actual` = gerçek fill.
    /// Döndürür: son 20 fill'in ortalama sapması (bps). 15 bps üzeri uyarı gerektirir.
    fn record_slippage_bps(&mut self, symbol: &str, expected: f64, actual: f64) -> f64 {
        if expected <= 0.0 { return 0.0; }
        let bps = (actual - expected).abs() / expected * 10_000.0;
        let hist = self.symbol_slip_bps
            .entry(symbol.to_string())
            .or_insert_with(std::collections::VecDeque::new);
        if hist.len() >= 20 { hist.pop_front(); }
        hist.push_back(bps);
        hist.iter().sum::<f64>() / hist.len() as f64
    }

    /// Kapanan işlemin PnL'ini sembol başına geçmişe ekle.
    /// Son 10 işlemin kümülatif PnL'i negatifse sembol 24 saat karantinaya alınır.
    fn record_symbol_pnl(&mut self, symbol: &str, pnl: f64) {
        let history = self.symbol_pnl_history
            .entry(symbol.to_string())
            .or_insert_with(|| std::collections::VecDeque::with_capacity(11));
        if history.len() >= 10 { history.pop_front(); }
        history.push_back(pnl);
        if history.len() == 10 {
            let cumulative: f64 = history.iter().sum();
            if cumulative < 0.0 {
                let expiry = std::time::Instant::now() + std::time::Duration::from_secs(86400);
                self.dynamic_blacklist.insert(symbol.to_string(), expiry);
            }
        }
    }

    /// Yön bilgisiyle PnL kaydet — SHORT kaybı sayacını doğru günceller.
    fn record_pnl_dir(&mut self, pnl: f64, is_long: bool) {
        self.total_pnl      += pnl;
        self.cumulative_pnl += pnl;
        self.current_equity  = (self.capital + self.cumulative_pnl).max(0.0);
        if self.current_equity > self.peak_equity {
            self.peak_equity = self.current_equity;
        }
        // Kümülatif istatistikler (asla sıfırlanmaz)
        self.session_closed += 1;
        if pnl > 0.0 {
            self.session_wins   += 1;
            self.session_profit += pnl;
            self.loss_streak     = 0;
            // SHORT kazandı → short streak sıfırla
            if !is_long { self.short_loss_streak = 0; }
        } else {
            self.session_loss += pnl.abs();
            self.loss_streak  += 1;
            self.last_loss_time = Some(std::time::Instant::now());
            // SHORT kaybetti → short streak artır
            if !is_long {
                self.short_loss_streak += 1;
                self.last_short_loss_time = Some(std::time::Instant::now());
            }
            // 3 ardışık zarar → 1 saatlik "Sadece İzleme" cool-off
            // Cooloff zaten aktifse yenileme (reset etme, cezayı uzatma).
            if self.loss_streak == 3 && self.cooloff_until.map(|t| t < std::time::Instant::now()).unwrap_or(true) {
                self.cooloff_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(3600));
            }
        }
    }
}

/// Per-interval candle önbelleği — her iterasyonda REST/DB tekrar çağrısını önler.
/// CandleSynth callback'i ve ana döngü yazar; process_symbol okur.
#[cfg(not(target_arch = "wasm32"))]
struct CandleCache {
    data:     std::collections::HashMap<String, std::collections::VecDeque<Candle>>,
    max_size: usize,
}

#[cfg(not(target_arch = "wasm32"))]
impl CandleCache {
    fn new(max_size: usize) -> Self {
        Self { data: std::collections::HashMap::new(), max_size }
    }

    /// Candle ekle. Aynı timestamp varsa son candle'ı güncelle (canlı kapanmamış mum).
    fn push(&mut self, candle: Candle) {
        let entry = self.data.entry(candle.interval.clone())
            .or_insert_with(std::collections::VecDeque::new);
        if entry.back().map(|c| c.timestamp) == Some(candle.timestamp) {
            if let Some(last) = entry.back_mut() { *last = candle; }
            return;
        }
        entry.push_back(candle);
        while entry.len() > self.max_size {
            entry.pop_front();
        }
    }

    /// Son `limit` candle'ı döner (en eski → en yeni). Yeterli yoksa tümünü döner.
    fn get_latest(&self, interval: &str, limit: usize) -> Vec<Candle> {
        self.data.get(interval)
            .map(|deq| {
                let skip = deq.len().saturating_sub(limit);
                deq.iter().skip(skip).cloned().collect()
            })
            .unwrap_or_default()
    }

    fn len(&self, interval: &str) -> usize {
        self.data.get(interval).map(|d| d.len()).unwrap_or(0)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub struct RoboticLoop<'a> {
        pub strategy_selector: Option<StrategySelector>, // Otomatik strateji seçici
    pub executor: &'a RoboticTradeExecutor<'a>,
    pub state: &'a mut dyn StateManager,
    pub reporter: &'a UniversalReporter,
    pub logger: &'a dyn ErrorLogger,
    pub config: RoboticLoopConfig,
    pub fetcher: &'a dyn LiveDataFetcher,
    pub backtest_fetcher: Option<&'a dyn LiveDataFetcher>,
    pub strategy: &'a dyn Strategy,
    pub ml_model: Option<MLModel>,
    pub ml_data: Option<Vec<FeatureVector>>,
    pub monitor: Option<Monitor>,
    pub portfolio: Option<Portfolio>,
    pub autonomous_trader: Option<AutonomousTrader>,
    pub use_ml_signal: bool,
    /// Çift güvenlik katmanı: true → emir Binance'e iletilmez (paper simülasyon).
    /// BinanceTradeExecutor'daki `is_paper` ile tutarlı olmalı.
    pub paper_mode: bool,

    /// Her interval için ayrı cycle_id — [0]=1m [1]=5m [2]=15m [3]=30m [4]=1h [5]=4h [6]=1d
    pub interval_cycle_ids: [u64; 7],
    /// Telegram push bildirici — None = env var tanımlı değil (özellik devre dışı).
    pub telegram: Option<crate::robot::telegram_notifier::TelegramNotifier>,
}

#[cfg(target_arch = "wasm32")]
pub struct RoboticLoop<'a> {
    pub executor: &'a RoboticTradeExecutor<'a>,
    pub state: &'a mut dyn StateManager,
    pub reporter: &'a UniversalReporter,
    pub logger: &'a dyn ErrorLogger,
    pub config: RoboticLoopConfig,
    pub fetcher: &'a dyn LiveDataFetcher,
    pub backtest_fetcher: Option<&'a dyn LiveDataFetcher>,
    pub ml_model: Option<MLModel>,
    pub ml_data: Option<Vec<FeatureVector>>,
    pub monitor: Option<Monitor>,
    pub paper_mode: bool,
}

impl<'a> RoboticLoop<'a> {
    // ── Sinyal sayacı ─────────────────────────────────────────────────────────
    /// Tek çağrıyla sinyal olayını kaydeder; `with_live` + `update_signal_counts` boilerplate'i yok.
    fn count_signal(&self, m: SignalMetric) {
        self.config.with_live(|s| s.update_signal_counts(|c| match m {
            SignalMetric::Buy                  => c.buy += 1,
            SignalMetric::Sell                 => c.sell += 1,
            SignalMetric::Hold                 => c.hold += 1,
            SignalMetric::BlockedRr(r)         => { c.blocked_rr += 1;         c.last_block_reason = r.into_owned(); }
            SignalMetric::BlockedVolatility(r) => { c.blocked_volatility += 1; c.last_block_reason = r.into_owned(); }
            SignalMetric::BlockedTrend(r)      => { c.blocked_trend += 1;      c.last_block_reason = r.to_string(); }
            SignalMetric::BlockedRiskGate(r)   => { c.blocked_risk_gate += 1;  c.last_block_reason = r; }
            SignalMetric::LastParams(p)        => c.last_params = p,
        }));
    }

    // ── Evrimsel mini-evrim tetikleyici ──────────────────────────────────────
    /// `should_evolve` normal döngü koşuluna ek olarak iki erken tetikleyici:
    /// 1. `loss_streak >= 3` — strateji kendini yenilemiyor
    /// 2. `drift_score > threshold` — piyasa rejimi kaydı
    #[cfg(not(target_arch = "wasm32"))]
    fn maybe_evolve(&self, ls: &mut LoopState) {
        if !self.config.autonomous_enabled { return; }
        let mini_trigger = ls.loss_streak >= 3
            || ls.drift_detector.drift_score > ls.drift_detector.threshold;
        if ls.autonomous_controller.should_evolve() || mini_trigger {
            ls.autonomous_controller.evolve_population();
            save_evolution_state_from_loop(&ls.autonomous_controller);
            if mini_trigger {
                self.logger.log_info("evolution", &format!(
                    "⚡ Mini evrim tetiklendi: ardışık_kayıp={} drift={:.2}",
                    ls.loss_streak, ls.drift_detector.drift_score
                ));
            }
            self.logger.log_info("evolution", &ls.autonomous_controller.get_evolution_summary());
        }
    }

    // ── Pozisyon TUI güncelleme ───────────────────────────────────────────────
    /// Açık pozisyonu live_positions haritasına yaz (insert veya update).
    fn upsert_live_position(
        &self,
        pos: &crate::robot::position_manager::OpenPosition,
        market: crate::types::Market,
        current_price: f64,
    ) {
        self.config.with_live(|s| s.update_positions(|lm| {
            lm.insert(
                live_pos_key(&pos.symbol, &market),
                LivePositionData::from_pos(pos, market, current_price),
            );
        }));
    }

    // ── Pozisyon kapatma ─────────────────────────────────────────────────────
    /// Pozisyonu live_positions'dan kaldır ve kapalı işlem geçmişine ekle.
    /// `remove()` → Some: bu loop kapattı → log'a ekle.
    /// `remove()` → None: başka bir loop kapattı → çift kayıt engellendi.
    fn close_position_and_log(
        &self,
        pos: &crate::robot::position_manager::OpenPosition,
        market: crate::types::Market,
        exit_price: f64,
        pnl: f64,
        exit_reason: &str,
        ls: &mut LoopState,
    ) {
        let key = live_pos_key(&pos.symbol, &market);
        // UUID eşleştirmeli kaldırma: aynı sembol/market'te farklı bir pozisyon açıldıysa silme
        let removed = self.config.map_live(|s| s.remove_position_by_id(&key, pos.id)).unwrap_or(true);
        if removed {
            // pnl_pct = marjin bazlı yüzde: pnl / margin × 100
            // margin = entry × qty / leverage
            // → pnl_pct = pnl × leverage / (entry × qty) × 100
            // Örn: 1% fiyat hareketi × 8x = %8 pnl_pct
            let lev = pos.leverage.max(1.0);
            let pnl_pct = if pos.entry_price > 0.0 && pos.qty > 0.0 {
                pnl * lev / (pos.entry_price * pos.qty) * 100.0
            } else { 0.0 };

            // ── Online ML eğitimi — KAPANIŞTA gerçek PnL ile ──────────────────
            // entry_features: pozisyon açılışında kaydedilmiş normalize öznitelik dizisi.
            // Hedef işareti pozisyon yönüne göre düzeltilir:
            //   LONG kâr (+pnl) → model pozitif tahmin öğrenmeli → target = +pnl_pct / 10
            //   SHORT kâr (+pnl) → model negatif tahmin öğrenmeli → target = −pnl_pct / 10
            //   (Model: pozitif = BUY, negatif = SELL; SHORT kâr ancak bearish koşullarda gerçekleşir)
            if let Some(entry_feats) = pos.entry_features {
                let signed_pnl = if pos.is_long { pnl_pct } else { -pnl_pct };
                let ml_target = (signed_pnl / 10.0).clamp(-1.0, 1.0);
                // Tek write lock: read→compute→write race'ini ve sessiz kayıpları engeller.
                let mut ml_trained_ok = false;
                self.config.with_live(|s| {
                    if let Ok(mut r) = s.live_risk.write() {
                        let mut m = LinearRegressor::with_defaults();
                        if let Some(w) = r.ml_weights {
                            m.weights.copy_from_slice(&w);
                            m.bias = r.ml_bias_trained;
                            m.is_trained = true;
                        }
                        m.train_step_raw(&entry_feats, ml_target, 0.0005);
                        r.ml_weights      = Some(m.weights);
                        r.ml_bias_trained = m.bias;
                        ml_trained_ok = true;
                    }
                });
                if ml_trained_ok {
                    self.logger.log_info("online-train", &format!(
                        "Online ML kapanış: {} {} reason={} target={:.3} pnl={:.2}%",
                        pos.symbol, if pos.is_long { "LONG" } else { "SHORT" },
                        exit_reason, ml_target, pnl_pct
                    ));
                }
            }

            // Kesinti bileşenlerini önceden hesapla — ClosedTradeData ve kümülatif özet aynı değerlerle beslenir
            let (entry_comm, exit_comm, slip_total) = if let Some(ref ec) = self.config.execution_cost_config {
                let entry_notional = pos.entry_price * pos.qty;
                let exit_notional  = exit_price * pos.qty;
                let entry_comm = entry_notional * self.config.commission_pct;
                let exit_comm  = exit_notional  * self.config.commission_pct;
                let spread_usd = exit_notional * ec.spread_pct / 100.0;
                let slip_usd   = exit_notional * ec.slippage_pct / 100.0 * 2.0;
                let impact_usd = ec.market_impact_pct(exit_notional) / 100.0 * exit_notional;
                (entry_comm, exit_comm, spread_usd + slip_usd + impact_usd)
            } else {
                // Execution cost config yoksa komisyonu commission_pct ile hesapla, slippage 0
                let entry_comm = pos.entry_price * pos.qty * self.config.commission_pct;
                let exit_comm  = exit_price     * pos.qty * self.config.commission_pct;
                (entry_comm, exit_comm, 0.0)
            };

            // entry_features indeksleri (FeatureVector::to_array sırası):
            //   [0]=rsi/100, [6]=sma5 norm, [8]=sma20 norm, [11]=vol_change/3, [13]=atr_pct/0.05
            let (entry_rsi, entry_atr_pct) = pos.entry_features
                .map(|f| (f[0] * 100.0, f[13] * 5.0))  // rsi_norm→0-100, atr_norm*0.05*100=atr_pct%
                .unwrap_or((0.0, 0.0));
            // Feature engineering: kapanış anındaki rejim, funding rate, BTC korelasyon
            let close_adx_regime = crate::robot::strategy_scorer::regime_idx(ls.adx_regime) as u8;
            let close_funding_rate = ls.funding_rate_cache
                .as_ref()
                .map(|(fr, _)| fr.funding_rate)
                .unwrap_or(0.0);
            let close_btc_corr = ls.btc_anchor_cache.as_ref().map(|(btc_candles, _)| {
                // Pearson korelasyon hesabı (robotic_loop içindeki inline fn ile aynı mantık)
                fn pearson(a: &[crate::types::Candle], b: &[crate::types::Candle]) -> f64 {
                    let n = a.len().min(b.len());
                    if n < 5 { return 0.0; }
                    let a_r: Vec<f64> = a.windows(2).map(|w| if w[0].close>0.0 { (w[1].close-w[0].close)/w[0].close } else { 0.0 }).collect();
                    let b_r: Vec<f64> = b.windows(2).map(|w| if w[0].close>0.0 { (w[1].close-w[0].close)/w[0].close } else { 0.0 }).collect();
                    let n = a_r.len().min(b_r.len());
                    if n < 5 { return 0.0; }
                    let (a_r, b_r) = (&a_r[..n], &b_r[..n]);
                    let ma = a_r.iter().sum::<f64>() / n as f64;
                    let mb = b_r.iter().sum::<f64>() / n as f64;
                    let num: f64 = a_r.iter().zip(b_r).map(|(a,b)| (a-ma)*(b-mb)).sum();
                    let da: f64 = a_r.iter().map(|a| (a-ma).powi(2)).sum::<f64>().sqrt();
                    let db: f64 = b_r.iter().map(|b| (b-mb).powi(2)).sum::<f64>().sqrt();
                    if da * db > 1e-12 { num / (da * db) } else { 0.0 }
                }
                // Son candle verisini candle_cache'den al
                let sym_candles = ls.candle_cache.lock().ok()
                    .and_then(|cc| Some(cc.get_latest(&self.config.interval, 50)));
                sym_candles.map(|sc| pearson(btc_candles, &sc)).unwrap_or(0.0)
            }).unwrap_or(0.0);
            self.config.with_live(|s| s.append_closed_trade(ClosedTradeData {
                pos_id:       pos.id,
                symbol:       pos.symbol.clone(),
                is_long:      pos.is_long,
                entry_price:  pos.entry_price,
                exit_price,
                qty:          pos.qty,
                pnl,
                pnl_pct,
                exit_reason:  exit_reason.to_string(),
                closed_at:    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                leverage:     lev,
                sl_price:     pos.static_sl,
                tp_price:     pos.static_tp,
                opened_at:    pos.opened_at.clone(),
                trade_type:   pos.trade_type,
                entry_commission: entry_comm,
                exit_commission:  exit_comm,
                slippage_usd:     slip_total,
                entry_rsi,
                entry_atr_pct,
                close_adx_regime,
                close_funding_rate,
                close_btc_corr,
            }));
            // Kümülatif işlem maliyetini güncelle (tür bazlı)
            if let Some(ref ec) = self.config.execution_cost_config {
                let exit_notional = exit_price * pos.qty;
                let impact_usd = ec.market_impact_pct(exit_notional) / 100.0 * exit_notional;
                let spread_usd = exit_notional * ec.spread_pct / 100.0;
                let slip_usd   = exit_notional * ec.slippage_pct / 100.0 * 2.0;
                let comm_total = entry_comm + exit_comm;
                let tt = pos.trade_type;
                self.config.with_live(|s| s.update_costs(|costs| {
                    costs.record(tt, comm_total, spread_usd, slip_usd, impact_usd);
                }));
            } else {
                let comm_total = entry_comm + exit_comm;
                let tt = pos.trade_type;
                self.config.with_live(|s| s.update_costs(|costs| {
                    costs.record(tt, comm_total, 0.0, 0.0, 0.0);
                }));
            }

            // ── Scalp/Swing istatistik güncellemesi + otonom ayarlama ─────────
            use crate::robot::scalp_swing::TradeType;
            let ss_cfg_opt = self.config.scalp_swing.as_ref();
            match pos.trade_type {
                TradeType::Scalp => {
                    ls.scalp_stats.record(pnl);
                    if let Some(cfg) = ss_cfg_opt {
                        if cfg.autonomous_tuning {
                            let b = &cfg.scalp_sl_bounds;
                            if b.adjust_every_n > 0
                                && ls.scalp_stats.total_closed > ls.scalp_stats.last_tune_at
                                && (ls.scalp_stats.total_closed - ls.scalp_stats.last_tune_at) >= b.adjust_every_n
                            {
                                // config Clone alarak güvenli şekilde ayarla, sonra geri yaz
                                if let Some(cfg_mut) = self.config.scalp_swing.as_ref() {
                                    let mut new_cfg = cfg_mut.clone();
                                    let msgs = crate::robot::scalp_swing::auto_tune(
                                        &ls.scalp_stats, TradeType::Scalp, &mut new_cfg
                                    );
                                    ls.scalp_stats.last_tune_at = ls.scalp_stats.total_closed;
                                    for m in &msgs {
                                        self.logger.log_info("scalp-tune", m);
                                    }
                                    // Not: RoboticLoopConfig.scalp_swing immutable ref — runtime ayarlama
                                    // live_risk üzerinden broadcast edilir; gelecek döngüde okunur.
                                    // Şimdilik log ile raporla; kalıcı hot-reload sonraki adımda.
                                    if !msgs.is_empty() {
                                        self.logger.log_info("scalp-tune", &format!(
                                            "⚙ SCP otonom ayarlama ({} işlem, WR={:.0}% PF={:.2}): {}",
                                            ls.scalp_stats.total_closed,
                                            ls.scalp_stats.win_rate() * 100.0,
                                            ls.scalp_stats.profit_factor(),
                                            msgs.join(", ")
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
                TradeType::Swing => {
                    ls.swing_stats.record(pnl);
                    if let Some(cfg) = ss_cfg_opt {
                        if cfg.autonomous_tuning {
                            let b = &cfg.swing_sl_bounds;
                            if b.adjust_every_n > 0
                                && ls.swing_stats.total_closed > ls.swing_stats.last_tune_at
                                && (ls.swing_stats.total_closed - ls.swing_stats.last_tune_at) >= b.adjust_every_n
                            {
                                if let Some(cfg_mut) = self.config.scalp_swing.as_ref() {
                                    let mut new_cfg = cfg_mut.clone();
                                    let msgs = crate::robot::scalp_swing::auto_tune(
                                        &ls.swing_stats, TradeType::Swing, &mut new_cfg
                                    );
                                    ls.swing_stats.last_tune_at = ls.swing_stats.total_closed;
                                    for m in &msgs {
                                        self.logger.log_info("swing-tune", m);
                                    }
                                    if !msgs.is_empty() {
                                        self.logger.log_info("swing-tune", &format!(
                                            "⚙ SWG otonom ayarlama ({} işlem, WR={:.0}% PF={:.2}): {}",
                                            ls.swing_stats.total_closed,
                                            ls.swing_stats.win_rate() * 100.0,
                                            ls.swing_stats.profit_factor(),
                                            msgs.join(", ")
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
                TradeType::Regular => {}
            }

            // Sembol performans geçmişini güncelle → 10 işlemde kümülatif negatifse karantina
            ls.record_symbol_pnl(&pos.symbol, pnl);

            // ── Strategy Scorer (UCB1 bandit) ─────────────────────────────────
            // Kapanan işlemi rejim × strateji matrisine kaydet.
            ls.strategy_scorer.record(pos.trade_type, ls.adx_regime, pnl);
            if ls.strategy_scorer.should_evaluate() {
                let atr_for_eval = ls.candle_cache.lock().ok()
                    .and_then(|cc| Some(cc.get_latest(&self.config.interval, 15)))
                    .and_then(|c| crate::robot::signal_evaluator::average_range_pct(&c, 14));
                ls.strategy_scorer.evaluate(atr_for_eval, ls.adx_regime);
                if !ls.strategy_scorer.last_reason.is_empty() {
                    self.logger.log_info("ai-control", &format!(
                        "🤖 AI Strateji Kontrolü [{} işlem]: {} | {}",
                        ls.strategy_scorer.total_n,
                        ls.strategy_scorer.last_reason,
                        ls.strategy_scorer.summary()
                    ));
                    // Özerk devre dışı bırakma mesajları
                    if ls.strategy_scorer.scalp_disabled {
                        self.logger.log_error("ai-control",
                            "🔴 AI: Scalp DEVRE DIŞI — Swing/REG moduna geçiliyor");
                    }
                    if ls.strategy_scorer.swing_disabled {
                        self.logger.log_error("ai-control",
                            "🔴 AI: Swing DEVRE DIŞI — REG/Scalp moduna geçiliyor");
                    }
                    if ls.strategy_scorer.reg_disabled {
                        self.logger.log_error("ai-control",
                            "🔴 AI: Regular DEVRE DIŞI — Scalp/Swing moduna geçiliyor");
                    }
                }
            }

            // TradePatternClassifier: kapanan işlemin giriş koşullarını öğren
            // Saat, RSI, ATR entry_features'tan kurtarılır; vol/trend/rr pozisyon bilgisinden çıkarılır.
            if entry_rsi > 0.0 || entry_atr_pct > 0.0 {
                let hour = chrono::NaiveDateTime::parse_from_str(&pos.opened_at, "%Y-%m-%d %H:%M:%S")
                    .map(|dt| dt.hour())
                    .unwrap_or_else(|_| chrono::Utc::now().hour());
                // entry_features[11]=vol_change/3 → *3 kurtarır; [6]=sma5_n, [8]=sma20_n
                let entry_vol = pos.entry_features.map(|f| f[11] * 3.0).unwrap_or(1.0);
                let entry_trend = pos.entry_features.map(|f| {
                    (f[6] - f[8] + 0.05).clamp(0.0, 0.10) / 0.10
                }).unwrap_or(0.5);
                let entry_rr = {
                    let risk   = (pos.entry_price - pos.static_sl).abs();
                    let reward = (pos.static_tp - pos.entry_price).abs();
                    if risk > 1e-9 { (reward / risk).clamp(0.0, 10.0) } else { 2.0 }
                };
                let clf_inp = crate::robot::ml_engine::ClassifierInput {
                    hour,
                    rsi:        entry_rsi,
                    atr_pct:    entry_atr_pct,
                    vol_ratio:  entry_vol,
                    trend_dir:  entry_trend,
                    body_ratio: 0.5,  // entry_features'ta mum gövdesi yok
                    rr:         entry_rr,
                };
                ls.pattern_classifier.record_and_maybe_retrain(
                    &mut ls.classifier_buffer,
                    &clf_inp,
                    pnl,
                );
                if ls.pattern_classifier.is_trained {
                    self.logger.log_info("pattern-ml", &format!(
                        "🧠 TradeClassifier güncellendi: {} kazanç / {} kayıp | ATR={:.2}% RR={:.2} vol={:.2}",
                        ls.pattern_classifier.n_win, ls.pattern_classifier.n_loss,
                        entry_atr_pct, entry_rr, entry_vol
                    ));
                }
            }

            // ML + scorer durumunu diske kaydet (her 5 kapanışta bir — I/O yükünü sınırla).
            if ls.session_closed % 5 == 0 {
                if let Some(path) = &self.config.classifier_state_path.clone() {
                    crate::robot::ml_engine::TradePatternClassifier::save_snapshot(
                        &ls.pattern_classifier, &ls.classifier_buffer, path,
                    );
                }
                if let Some(path) = &self.config.scorer_state_path.clone() {
                    ls.strategy_scorer.save(path);
                }
            }
        }
    }

    fn interval_to_secs(interval: &str) -> u64 {
        // Basit interval çevirici (örn: "1m", "5m", "1h")
        let trimmed = interval.trim().to_lowercase();
        if let Some(num) = trimmed.strip_suffix('m') {
            num.parse::<u64>().unwrap_or(1) * 60
        } else if let Some(num) = trimmed.strip_suffix('h') {
            num.parse::<u64>().unwrap_or(1) * 60 * 60
        } else if let Some(num) = trimmed.strip_suffix('d') {
            num.parse::<u64>().unwrap_or(1) * 60 * 60 * 24
        } else {
            60 // bilinmeyen format, 1 dakika varsay
        }
    }

    /// Her döngü başında live_risk'ten strateji parametrelerini sıcak yükle.
    /// Değişen tek parametre grubu güncellenir; değişmeyenler dokunulmadan kalır.
    #[cfg(not(target_arch = "wasm32"))]
    fn reload_strategy_params(&mut self, ls: &mut LoopState) {
        // Versiyon değişmemişse lock alıp parametreleri okuma — gereksiz contention önlenir.
        let current_version = self.config.live_state.as_ref()
            .and_then(|s| s.live_risk.try_read().ok())
            .map(|lrm| lrm.strategy_params_version)
            .unwrap_or(ls.last_reloaded_params_version);
        if current_version == ls.last_reloaded_params_version {
            return; // Parametre değişmemiş, atla
        }
        ls.last_reloaded_params_version = current_version;
        if let Some(lrm) = self.config.live_state.as_ref().and_then(|s| s.live_risk.try_read().ok()) {
            // MA
            let (new_fast, new_slow) = (lrm.global_fast, lrm.global_slow);
            if new_fast != 0 && (new_fast != self.config.strategy_params.fast.unwrap_or(0)
                || new_slow != self.config.strategy_params.slow.unwrap_or(0)) {
                self.config.strategy_params.fast = Some(new_fast);
                self.config.strategy_params.slow = Some(new_slow);
                self.logger.log_info("risk_reload",
                    &format!("🔄 MA periyot güncellendi: fast={} slow={}", new_fast, new_slow));
            }
            // RSI
            let (new_period, new_ob, new_os) = (lrm.global_rsi_period, lrm.global_rsi_ob, lrm.global_rsi_os);
            if new_period > 0
                && (new_period != self.config.strategy_params.period.unwrap_or(0)
                    || (new_ob - self.config.strategy_params.overbought.unwrap_or(0.0)).abs() > 0.5
                    || (new_os - self.config.strategy_params.oversold.unwrap_or(0.0)).abs() > 0.5)
            {
                self.config.strategy_params.period     = Some(new_period);
                self.config.strategy_params.overbought = Some(new_ob);
                self.config.strategy_params.oversold   = Some(new_os);
                self.logger.log_info("risk_reload",
                    &format!("🔄 RSI parametresi güncellendi: period={} OB={:.0}% OS={:.0}%",
                        new_period, new_ob, new_os));
            }
            // BB
            let (new_bb_p, new_bb_s) = (lrm.global_bb_period, lrm.global_bb_std_dev);
            if new_bb_p > 0
                && (new_bb_p != self.config.strategy_params.bb_period.unwrap_or(0)
                    || (new_bb_s - self.config.strategy_params.std_dev.unwrap_or(0.0)).abs() > 0.1)
            {
                self.config.strategy_params.bb_period = Some(new_bb_p);
                self.config.strategy_params.std_dev   = Some(new_bb_s);
                self.logger.log_info("risk_reload",
                    &format!("🔄 BB parametresi güncellendi: period={} std_dev={:.1}", new_bb_p, new_bb_s));
            }
            // MACD
            let (new_mf, new_ms, new_msig) = (lrm.global_macd_fast, lrm.global_macd_slow, lrm.global_macd_signal);
            if new_mf > 0
                && (new_mf != self.config.strategy_params.fast_period.unwrap_or(0)
                    || new_ms != self.config.strategy_params.slow_period.unwrap_or(0)
                    || new_msig != self.config.strategy_params.signal_period.unwrap_or(0))
            {
                self.config.strategy_params.fast_period   = Some(new_mf);
                self.config.strategy_params.slow_period   = Some(new_ms);
                self.config.strategy_params.signal_period = Some(new_msig);
                self.logger.log_info("risk_reload",
                    &format!("🔄 MACD parametresi güncellendi: fast={} slow={} signal={}",
                        new_mf, new_ms, new_msig));
            }
        }
    }

    /// TUI'dan kaydedilen adaptive_params'ı sıcak yükle.
    /// adaptive_params_version değişince disk'ten okur, loop içindeki ls.adaptive_params güncellenir.
    #[cfg(not(target_arch = "wasm32"))]
    fn reload_adaptive_params(&self, ls: &mut LoopState) {
        let current_version = self.config.live_state.as_ref()
            .and_then(|s| s.live_risk.try_read().ok())
            .map(|lrm| lrm.adaptive_params_version)
            .unwrap_or(ls.last_reloaded_adaptive_version);
        if current_version == ls.last_reloaded_adaptive_version {
            return;
        }
        ls.last_reloaded_adaptive_version = current_version;
        if let Some(path) = &self.config.adaptive_params_path {
            let fresh = crate::robot::adaptive_params::AdaptiveTradeParams::load(path);
            ls.adaptive_params = fresh;
            self.logger.log_info("adaptive-reload",
                &format!("🔄 adaptive_params sıcak yüklendi (v{}): sl_atr={:.2}x tp_atr={:.1}x tsl={:.1}%",
                    current_version,
                    ls.adaptive_params.sl_atr_multiplier,
                    ls.adaptive_params.tp_atr_multiplier,
                    ls.adaptive_params.trailing_sl_activation_pct,
                ));
        }
    }

    /// Aktif sembol için WS canlı fiyatını kullanarak anlık SL/TP kontrolü.
    ///
    // ── Smart Limit Entry (Dynamic Tick Offset + Re-Quoting) ─────────────────
    //
    // Giriş emirlerini "Best_Bid + 1 tick" seviyesine yerleştirerek maker komisyon
    // avantajı sağlar. Emir dolmazsa maksimum MAX_ATTEMPTS kez yeni fiyatla yeniden
    // gönderilir. Hepsi başarısız olursa "fırsat kaçtı" hatası döner.
    //
    // Kural özeti:
    //   LONG  → limit = best_bid + 1_tick  (bid tahtasının üstü — hızlı dolan maker)
    //   SHORT → limit = best_ask - 1_tick  (ask tahtasının altı — hızlı dolan maker)
    //   Her deneme = 2 sn timeout; toplam maks 3 × 2 sn = 6 sn
    //   Deneme arası: 200 ms + taze WS/bookTicker fiyatı
    //
    // Çıkış/SL emirleri burada değil, execute_basket (MARKET) ile gönderilir.
    #[cfg(not(target_arch = "wasm32"))]
    async fn smart_limit_entry(
        &self,
        signal: Signal,
        symbol: &str,
        qty: f64,
        base_price: f64,
    ) -> Vec<crate::Result<crate::types::Trade>> {
        const MAX_ATTEMPTS: u32 = 3;
        const PER_ATTEMPT_TIMEOUT_MS: u64 = 2000; // 2 sn / deneme
        // 1 tick ≈ 1 bip (0.01% of price) — Binance'te BTC/ETH için $0.01, altlar için proporsiyonel
        // LONG:  bid + tick → bid tahtası en üst → maker pozisyonu garantili, hızlı dolar
        // SHORT: ask - tick → ask tahtası en alt → maker pozisyonu garantili
        const BIP: f64 = 0.0001; // 0.01%

        let is_long = matches!(signal, Signal::Buy);
        let mut current_price = base_price;

        for attempt in 1..=MAX_ATTEMPTS {
            // 1. BookTicker'dan best bid/ask çek; hata/paper modda WS fiyatına fallback
            let (best_bid, best_ask) = self.executor.executor
                .fetch_book_ticker(symbol)
                .unwrap_or((0.0, 0.0));

            let (bid, ask) = if best_bid > 0.0 && best_ask > 0.0 && best_ask > best_bid {
                (best_bid, best_ask)
            } else {
                // Fallback: WS fiyatı ± tahmini spread (0.02% yarı spread)
                let mid = self.config.live_state.as_ref()
                    .and_then(|s| s.live_price.read().ok())
                    .filter(|pd| pd.close > 0.0 && pd.symbol == symbol)
                    .map(|pd| pd.close)
                    .unwrap_or(current_price);
                (mid * (1.0 - 0.0002), mid * (1.0 + 0.0002))
            };

            // ── Spread guard: spread çok genişse bu denemeyi atla ────────────
            // Geniş spread = likidite düşük veya volatilite ani artışı.
            // Maker fill ihtimali azalır, taker olma ve HF blacklist riski artar.
            if let Some(max_bps) = self.config.max_spread_bps {
                let mid = (bid + ask) / 2.0;
                if mid > 0.0 {
                    let spread_bps = (ask - bid) / mid * 10_000.0;
                    if spread_bps > max_bps {
                        self.logger.log_info("spread-guard", &format!(
                            "↩ Deneme {}/{}: spread {:.1} bps > eşik {:.0} bps — bu deneme atlandı [{}]",
                            attempt, MAX_ATTEMPTS, spread_bps, max_bps, symbol
                        ));
                        // 200ms bekle ve bir sonraki denemeye geç (spread daralabilir)
                        std::thread::sleep(std::time::Duration::from_millis(200));
                        continue;
                    }
                }
            }

            // 2. Tick hesapla + limit fiyatı belirle
            let tick = bid.max(ask) * BIP;
            let limit_price = if is_long { bid + tick } else { ask - tick };
            current_price = if is_long { bid } else { ask };

            self.logger.log_info("smart-entry", &format!(
                "⚡ SmartLimit #{}/{}: {} {} qty={:.6} @ {:.4} (bid={:.4} ask={:.4} tick={:.4}) [{}]",
                attempt, MAX_ATTEMPTS,
                if is_long { "BUY" } else { "SELL" },
                symbol, qty, limit_price, bid, ask, tick, symbol
            ));

            // 3. POST_ONLY limit emir — 2 sn timeout
            let result = self.executor.executor.execute_limit(
                signal.clone(), symbol, qty, limit_price, PER_ATTEMPT_TIMEOUT_MS,
            );

            match result {
                Ok(trade) => {
                    self.logger.log_info("smart-entry", &format!(
                        "✅ SmartLimit DOLU: {} @ {:.4} (deneme {}/{}) [{}]",
                        if is_long { "BUY" } else { "SELL" },
                        trade.entry_price, attempt, MAX_ATTEMPTS, symbol
                    ));
                    return vec![Ok(trade)];
                }
                Err(e) => {
                    if attempt >= MAX_ATTEMPTS {
                        self.logger.log_error("smart-entry", &format!(
                            "🚫 SmartLimit: {} deneme başarısız — fırsat kaçtı [{}] (son hata: {})",
                            MAX_ATTEMPTS, symbol, e
                        ));
                        return vec![Err(format!(
                            "SmartLimit {MAX_ATTEMPTS} deneme başarısız — fırsat kaçtı [{symbol}]"
                        ).into())];
                    }
                    // Taze WS fiyatı al, kısa bekleme, re-quote
                    let ws_fresh = self.config.live_state.as_ref()
                        .and_then(|s| s.live_price.read().ok())
                        .filter(|pd| pd.close > 0.0 && pd.symbol == symbol)
                        .map(|pd| pd.close)
                        .unwrap_or(current_price);
                    let drift_pct = (ws_fresh - current_price).abs() / current_price * 100.0;
                    self.logger.log_info("smart-entry", &format!(
                        "⟳ Re-quote #{}/{}: timeout → yeni_fiyat={:.4} kayış={:.3}% [{}]",
                        attempt, MAX_ATTEMPTS, ws_fresh, drift_pct, symbol
                    ));
                    current_price = ws_fresh;
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                }
            }
        }
        // Buraya gelinmez (loop içinde return'lar var), derleyici için
        vec![Err("smart_limit_entry: logic error".into())]
    }

    /// Her saniye çağrılır (interval sleep içinden).
    /// Böylece 1h aralıkta bile SL/TP gecikmesi en fazla ~1 sn olur.
    /// Tetiklenirse `true` döner → sleep döngüsü kırılır, yeni iterasyon başlar.
    #[cfg(not(target_arch = "wasm32"))]
    fn check_live_sl_tp(&self, ls: &mut LoopState) -> bool {
        // WS'nin güncellediği live_price arc'ından anlık fiyatı al
        let ws_price = match self.config.map_live(|s| {
            s.live_price.read().ok()
                .filter(|pd| pd.close > 0.0 && pd.symbol == self.config.symbol)
                .map(|pd| pd.close)
        }).flatten() {
            Some(p) => p,
            None    => return false,
        };

        // Aktif sembol + market'e ait pozisyonu bul
        let pos_id = ls.open_positions.iter()
            .find(|(_, p)| p.symbol == self.config.symbol && p.market == self.config.market)
            .map(|(id, _)| *id);

        if let Some(id) = pos_id {
            // Manuel çıkış gerekiyorsa otomatik SL/TP uygulama
            if ls.open_positions.get(&id).map(|p| p.manual_exit_required).unwrap_or(false) {
                self.logger.log_error("check-sl-tp", &format!(
                    "⛔ {} — MANUAL EXIT REQUIRED pozisyonu otomatik SL/TP'den muaf tutuldu",
                    self.config.symbol
                ));
                return false;
            }

            if let Some(mut pos) = ls.open_positions.remove(&id) {
                // ── B2: ATR trailing — trailing_pct'yi güncel ATR'ye göre güncelle ──────────────
                if pos.atr_trail_mult.is_some() {
                    let candles = {
                        let cc = ls.candle_cache.lock().unwrap();
                        cc.get_latest(&self.config.interval, 20)
                    };
                    if let (Some(mult), Some(atr_pct)) = (pos.atr_trail_mult, average_range_pct(&candles, 14)) {
                        pos.trailing_pct = Some((mult * atr_pct).max(0.1));
                    }
                }

                let was_breakeven = pos.breakeven_triggered;
                if let Some(exit_reason) = pos.update(ws_price) {
                    // ── TP1 Merdiveni — ara hedef, %40 kapat, SL breakeven, kalan trailing ───────
                    if exit_reason == "tp1" {
                        let ratio = pos.tp1_close_ratio.clamp(0.01, 0.99);
                        let close_qty = pos.qty * ratio;
                        let tp1_exit = pos.tp1_price.unwrap_or(ws_price);
                        let effective_exit = self.config.execution_cost_config.as_ref()
                            .map(|ec| adjusted_price(tp1_exit, close_qty, !pos.is_long, ec))
                            .unwrap_or(tp1_exit);
                        let pnl = pos.realized_pnl_with_commission(effective_exit, self.config.commission_pct)
                            * ratio;
                        ls.record_pnl_dir(pnl, pos.is_long);
                        let close_signal = if pos.is_long { Signal::Sell } else { Signal::Buy };
                        self.logger.log_info("tp1-ladder", &format!(
                            "TP1 tetiklendi ({:.0}%) | {} fiyat={:.4} tp1={:.4} pnl={:+.2}",
                            ratio * 100.0, self.config.symbol, ws_price, tp1_exit, pnl
                        ));
                        let _ = self.executor.execute_basket(close_signal, close_qty);
                        pos.qty -= close_qty;
                        pos.tp1_triggered = true;
                        pos.static_sl = pos.entry_price; // SL breakeven
                        if pos.trailing_pct.is_none() {
                            pos.trailing_pct = self.config.risk_params.trailing_stop_pct;
                        }
                        ls.open_positions.insert(id, pos);
                        return true;
                    }

                    // ── B3: Kısmi TP — pozisyonu tam kapatma, sadece oranı kapat ─────────────────
                    if exit_reason == "partial_tp" {
                        let ratio = pos.partial_tp_ratio.unwrap_or(0.5).clamp(0.01, 0.99);
                        let close_qty = pos.qty * ratio;
                        let effective_exit = self.config.execution_cost_config.as_ref()
                            .map(|ec| adjusted_price(pos.static_tp, close_qty, !pos.is_long, ec))
                            .unwrap_or(pos.static_tp);
                        let pnl = pos.realized_pnl_with_commission(effective_exit, self.config.commission_pct)
                            * ratio;
                        ls.record_pnl_dir(pnl, pos.is_long); // SHORT kısmi TP short_loss_streak sıfırlamalı
                        let close_signal = if pos.is_long { Signal::Sell } else { Signal::Buy };
                        self.logger.log_info("partial-tp", &format!(
                            "⚡ Kısmi TP ({:.0}%) | {} fiyat={:.4} çıkış={:.4} qty={:.6} pnl={:+.2}",
                            ratio * 100.0, self.config.symbol, ws_price, effective_exit, close_qty, pnl
                        ));
                        let _ = self.executor.execute_basket(close_signal, close_qty);
                        // Kalan pozisyon: qty küçül, SL breakeven'a çek, trailing aktif et
                        pos.qty -= close_qty;
                        pos.partial_tp_triggered = true;
                        pos.static_sl = pos.entry_price; // breakeven garantisi
                        // Kalan kısım için trailing yoksa config'deki trailing_pct'yi devreye al
                        if pos.trailing_pct.is_none() {
                            pos.trailing_pct = self.config.risk_params.trailing_stop_pct;
                        }
                        ls.open_positions.insert(id, pos);
                        return true;
                    }

                    // Adil çıkış fiyatı: SL/TP seviyesini kullan (WS anlık fiyatını değil).
                    // WS fiyatı SL/TP'yi bir anda aşabilir; gerçek çıkış emir fiyatı
                    // SL/TP seviyesinde verilmiş sayılır.
                    let sl_tp_price = match exit_reason {
                        "static_sl"   => pos.static_sl,
                        "trailing_sl" => pos.trailing_sl.unwrap_or(pos.static_sl),
                        _             => pos.static_tp, // "take_profit"
                    };
                    // Paper fill güvencesi: stop emirleri piyasa fiyatının daha iyi
                    // tarafında doldurulamaz. Fiyat gap yaparsa fill piyasaya kırpılır.
                    let fair_sl_tp = if exit_reason.contains("sl") {
                        if pos.is_long { sl_tp_price.min(ws_price) }   // LONG stop fill ≤ market
                        else           { sl_tp_price.max(ws_price) }   // SHORT stop fill ≥ market
                    } else { sl_tp_price };
                    let effective_exit = self.config.execution_cost_config.as_ref()
                        .map(|ec| adjusted_price(fair_sl_tp, pos.qty, !pos.is_long, ec))
                        .unwrap_or(fair_sl_tp);
                    let pnl = pos.realized_pnl_with_commission(
                        effective_exit, self.config.commission_pct
                    );

                    // ── Rolling slippage kaydı ────────────────────────────────
                    // fair_sl_tp = beklenen çıkış, effective_exit = gerçek fill.
                    // Ortalama sapma > 15 bps ise düşük kaliteli fill uyarısı ver.
                    {
                        let avg_slip = ls.record_slippage_bps(&self.config.symbol, fair_sl_tp, effective_exit);
                        if avg_slip > 15.0 {
                            self.logger.log_info("slip-quality", &format!(
                                "⚠ {} ortalama fill sapması {:.1} bps (son 20 işlem) — likidite düşük olabilir",
                                self.config.symbol, avg_slip
                            ));
                        }
                    }

                    ls.record_pnl_dir(pnl, pos.is_long);
                    // Sembol bazlı ardışık kayıp takibi — seri kayıptan koruma
                    {
                        let counter = ls.symbol_consec_loss
                            .entry(pos.symbol.clone()) // pos.symbol kullan, self.config.symbol değil
                            .or_insert(0);
                        if pnl < 0.0 { *counter += 1; } else { *counter = 0; }
                        let streak = *counter;
                        // Artan cooldown: 3-4 ardışık kayıp → 2 saat, 5+ → 24 saat
                        if streak >= 5 {
                            ls.symbol_cooldown_secs.insert(pos.symbol.clone(), 86400);
                            self.logger.log_error("loss-streak", &format!(
                                "🚨 {} {} ardışık kayıp — 24 saatlik işlem yasağı aktif",
                                pos.symbol, streak
                            ));
                        } else if streak >= 3 {
                            ls.symbol_cooldown_secs.insert(pos.symbol.clone(), 7200);
                            self.logger.log_error("loss-streak", &format!(
                                "⚠ {} {} ardışık kayıp — 2 saatlik cooldown devreye girdi",
                                pos.symbol, streak
                            ));
                        }
                    }
                    // ── TP Gap tespiti (HF blacklist) ────────────────────────────────
                    // WS fiyatı TP seviyesinden >%0.3 uzaksa gap gerçekleşmiş demektir.
                    // Bu sembolün likiditesi düşük ya da hareket çok ani — HF sayacına ekle.
                    if exit_reason == "take_profit" && pos.static_tp > 0.0 {
                        let gap_pct = (ws_price - pos.static_tp).abs() / pos.static_tp;
                        if gap_pct > 0.003 {
                            let newly_banned = ls.record_hf_error(&self.config.symbol);
                            if newly_banned {
                                self.logger.log_error("hf-blacklist", &format!(
                                    "🚫 {} 4 saatlik işlem yasağı (TP Gap {:.2}% > %0.3) — 2+ HF hatası son 1 saatte",
                                    self.config.symbol, gap_pct * 100.0
                                ));
                            } else {
                                self.logger.log_info("hf-blacklist", &format!(
                                    "⚠ {} TP Gap {:.2}% kaydedildi (2. kayıt → 4h ban)",
                                    self.config.symbol, gap_pct * 100.0
                                ));
                            }
                        }
                    }

                    self.logger.log_info("sl-tp-live", &format!(
                        "⚡ SL/TP tetiklendi ({}) | {} WS fiyat={:.4} çıkış={:.4} \
                         entry={:.4} pnl={:+.2}",
                        exit_reason, self.config.symbol,
                        ws_price, effective_exit, pos.entry_price, pnl
                    ));
                    // FIX-A: LIVE modda gerçek kapanış emri gönder.
                    // Paper modda execute_basket dummy trade döndürür — bu satır paper'da da güvenli.
                    let close_signal = if pos.is_long { Signal::Sell } else { Signal::Buy };
                    self.logger.log_info("executor", &format!(
                        "{} CLOSE (SL/TP) → {} {:?} qty={:.6}",
                        if self.paper_mode { "PAPER" } else { "LIVE" },
                        self.config.symbol, close_signal, pos.qty
                    ));
                    let close_trades = self.executor.execute_basket(close_signal, pos.qty);
                    if close_trades.iter().any(|t| t.is_err()) {
                        self.logger.log_error("sl-tp-live", &format!(
                            "⚠ Kapanış emri başarısız: {} — pozisyon exchange'de açık kalabilir",
                            self.config.symbol
                        ));
                        let _ = ls.api_circuit_breaker.record_failure("sl_tp_close");
                    } else {
                        let _ = ls.api_circuit_breaker.record_success();
                    }
                    // SL ile kapandıysa cooldown + günlük SL sayacı başlat
                    if exit_reason.contains("sl") {
                        // TSL kapanışı → daha uzun cooldown (30 dk) çünkü volatilite henüz sakinleşmedi
                        let cd_secs = if exit_reason.contains("trailing") {
                            let tsl_cd = ls.sl_cooldown_secs.max(1800); // min 30 dk
                            ls.symbol_cooldown_secs.insert(self.config.symbol.to_string(), tsl_cd);
                            tsl_cd
                        } else {
                            ls.sl_cooldown_secs
                        };
                        ls.sl_cooldown_map.insert(
                            self.config.symbol.to_string(),
                            std::time::Instant::now(),
                        );
                        self.logger.log_info("cooldown", &format!(
                            "⏸ {} SL cooldown başladı (live, {} dk)",
                            self.config.symbol, cd_secs / 60
                        ));
                        // SCP/SWG tip bazlı ek cooldown — ardışık SL'ye göre artar
                        {
                            let key = format!("{}_{}", self.config.symbol, pos.trade_type.label());
                            let count = ls.scalp_swing_consecutive_sl.entry(key.clone()).or_insert(0);
                            *count += 1;
                            let ss_cd_secs = ss_cooldown_secs(pos.trade_type, *count);
                            if ss_cd_secs > 0 {
                                ls.scalp_swing_sl_cooldown.insert(key.clone(), (std::time::Instant::now(), ss_cd_secs));
                                self.logger.log_info("ss-cooldown", &format!(
                                    "⏸ [{}] {} SL sonrası tip-cooldown {} dk (ardışık={})",
                                    pos.trade_type.label(), self.config.symbol, ss_cd_secs / 60, count
                                ));
                            }
                        }
                        // Günlük sembol SL sayacını artır
                        let today = chrono::Utc::now().date_naive();
                        let sym = self.config.symbol.to_string();
                        let entry = ls.daily_sl_map.entry(sym).or_insert((0, today));
                        if entry.1 == today { entry.0 += 1; } else { *entry = (1, today); }
                    }
                    ls.closed_position_ids.insert(pos.id); // duplicate guard
                    self.close_position_and_log(
                        &pos, self.config.market, effective_exit, pnl, exit_reason, ls
                    );
                    // Evrimsel Learning: gerçek PnL ile kapanışta çağır (WS yolu)
                    if self.config.autonomous_enabled {
                        let lev = pos.leverage.max(1.0);
                        let pnl_pct_evo = if pos.entry_price > 0.0 && pos.qty > 0.0 {
                            pnl * lev / (pos.entry_price * pos.qty) * 100.0
                        } else { 0.0 };
                        if let Some((regime, strategy)) = ls.pending_evo_data.remove(&pos.id) {
                            ls.autonomous_controller.learn_from_trade(pnl_pct_evo, &regime, &strategy);
                            self.maybe_evolve(ls);
                        }
                    }
                    return true; // sleep'i kes, hemen yeni iterasyona geç
                } else {
                    // ── B1: Breakeven tetiklendiyse logla ────────────────────────────────────────
                    if !was_breakeven && pos.breakeven_triggered {
                        self.logger.log_info("breakeven", &format!(
                            "✅ Breakeven tetiklendi: {} SL giriş fiyatına taşındı ({:.4}) | WS={:.4}",
                            self.config.symbol, pos.entry_price, ws_price
                        ));
                    }
                    ls.open_positions.insert(id, pos);
                }
            }
        }
        false
    }

    /// Ana per-symbol döngüsünde işlenemeyen orphan pozisyonların SL/TP kontrolü.
    /// Key: PositionId — sembol/market karışması imkânsız.
    #[cfg(not(target_arch = "wasm32"))]
    fn process_orphans(&self, ls: &mut LoopState) {
        let orphan_items: Vec<(crate::types::PositionId, String)> = ls.open_positions.iter()
            .map(|(&id, p)| (id, live_pos_key(&p.symbol, &p.market)))
            .collect();
        for (orphan_id, orphan_key) in orphan_items {
            if ls.stop_loop { break; }
            let orphan_price = self.config.live_state.as_ref()
                .and_then(|s| s.live_positions.read().ok())
                .and_then(|lm| lm.get(&orphan_key).map(|p| p.current_price))
                .unwrap_or(0.0);
            if orphan_price <= 0.0 { continue; }
            if let Some(mut pos) = ls.open_positions.remove(&orphan_id) {
                // ATR trailing güncelle (orphan pozisyonlar için de)
                if pos.atr_trail_mult.is_some() {
                    let candles = {
                        let cc = ls.candle_cache.lock().unwrap();
                        cc.get_latest(&self.config.interval, 20)
                    };
                    if let (Some(mult), Some(atr_pct)) = (pos.atr_trail_mult, average_range_pct(&candles, 14)) {
                        pos.trailing_pct = Some((mult * atr_pct).max(0.1));
                    }
                }
                if let Some(exit_reason) = pos.update(orphan_price) {
                    // ── TP1 Merdiveni (orphan) ────────────────────────────────────────────────────
                    if exit_reason == "tp1" {
                        let ratio = pos.tp1_close_ratio.clamp(0.01, 0.99);
                        let close_qty = pos.qty * ratio;
                        let tp1_exit = pos.tp1_price.unwrap_or(orphan_price);
                        let pnl = pos.realized_pnl_with_commission(tp1_exit, self.config.commission_pct) * ratio;
                        ls.record_pnl_dir(pnl, pos.is_long);
                        let close_signal = if pos.is_long { Signal::Sell } else { Signal::Buy };
                        let _ = self.executor.execute_basket(close_signal, close_qty);
                        self.logger.log_info("orphan-tp1", &format!(
                            "Orphan TP1 ({:.0}%) | {}/{:?} fiyat={:.4} pnl={:+.2} [id={}]",
                            ratio * 100.0, pos.symbol, pos.market, orphan_price, pnl, pos.id
                        ));
                        pos.qty -= close_qty;
                        pos.tp1_triggered = true;
                        pos.static_sl = pos.entry_price;
                        if pos.trailing_pct.is_none() {
                            pos.trailing_pct = self.config.risk_params.trailing_stop_pct;
                        }
                        ls.open_positions.insert(orphan_id, pos);
                        continue;
                    }

                    // Kısmi TP — oranı kapat, kalan devam et
                    if exit_reason == "partial_tp" {
                        let ratio = pos.partial_tp_ratio.unwrap_or(0.5).clamp(0.01, 0.99);
                        let close_qty = pos.qty * ratio;
                        let pnl = pos.realized_pnl_with_commission(pos.static_tp, self.config.commission_pct) * ratio;
                        ls.record_pnl_dir(pnl, pos.is_long); // orphan SHORT kısmi TP short_loss_streak sıfırlamalı
                        let close_signal = if pos.is_long { Signal::Sell } else { Signal::Buy };
                        let _ = self.executor.execute_basket(close_signal, close_qty);
                        self.logger.log_info("orphan-partial-tp", &format!(
                            "Orphan kısmi TP ({:.0}%) | {}/{:?} fiyat={:.4} pnl={:+.2} [id={}]",
                            ratio * 100.0, pos.symbol, pos.market, orphan_price, pnl, pos.id
                        ));
                        pos.qty -= close_qty;
                        pos.partial_tp_triggered = true;
                        pos.static_sl = pos.entry_price;
                        if pos.trailing_pct.is_none() {
                            pos.trailing_pct = self.config.risk_params.trailing_stop_pct;
                        }
                        ls.open_positions.insert(orphan_id, pos);
                        continue;
                    }

                    // ── Duplicate closure guard ─────────────────────────────────
                    // Aynı UUID'nin iki kez kapanması: WS handler (rtc_cli) race veya
                    // restart sonrası stale snapshot. closed_position_ids seti her
                    // kapanışta güncellenir; burada görülürse sessizce atla.
                    if ls.closed_position_ids.contains(&pos.id) {
                        self.logger.log_info("orphan-sl-tp", &format!(
                            "Orphan pozisyon zaten kapatılmış — duplicate skip | {} [id={}]",
                            pos.symbol, pos.id
                        ));
                        continue; // open_positions'a geri ekleme — tamamen kaldır
                    }

                    let close_signal = if pos.is_long { Signal::Sell } else { Signal::Buy };
                    let trades = self.executor.execute_basket(close_signal, pos.qty);
                    // Uygulama kapalıyken fiyat SL/TP'yi aşmışsa adil çıkış fiyatı olarak
                    // SL veya TP seviyesini kullan — mevcut piyasa fiyatını değil.
                    // "trailing_sl" → aktif trailing seviyesi (static_tp değil!)
                    let exit_price = match exit_reason {
                        "static_sl"   => pos.static_sl,
                        "trailing_sl" => pos.trailing_sl.unwrap_or(pos.static_sl),
                        _             => pos.static_tp, // "take_profit"
                    };
                    let pnl = pos.realized_pnl_with_commission(exit_price, self.config.commission_pct);
                    ls.record_pnl(pnl);
                    self.logger.log_info("orphan-sl-tp", &format!(
                        "Orphan pozisyon kapandı ({}) | {}/{:?} piyasa={:.4} çıkış={:.4} entry={:.4} pnl={:+.2} [id={}]",
                        exit_reason, pos.symbol, pos.market, orphan_price, exit_price, pos.entry_price, pnl, pos.id
                    ));
                    for t in trades {
                        if let Ok(tr) = t {
                            if tr.pnl.unwrap_or(pnl) > 0.0 { ls.win_trades += 1; }
                            ls.total_trades += 1;
                        }
                    }
                    ls.closed_position_ids.insert(pos.id); // duplicate guard: aynı UUID bir daha kapanmaz
                    self.close_position_and_log(&pos, pos.market, exit_price, pnl, exit_reason, ls);
                    // Evrimsel Learning: gerçek PnL ile kapanışta çağır (orphan yolu)
                    if self.config.autonomous_enabled {
                        let lev = pos.leverage.max(1.0);
                        let pnl_pct_evo = if pos.entry_price > 0.0 && pos.qty > 0.0 {
                            pnl * lev / (pos.entry_price * pos.qty) * 100.0
                        } else { 0.0 };
                        if let Some((regime, strategy)) = ls.pending_evo_data.remove(&pos.id) {
                            ls.autonomous_controller.learn_from_trade(pnl_pct_evo, &regime, &strategy);
                            self.maybe_evolve(ls);
                        }
                    }
                } else {
                    ls.open_positions.insert(orphan_id, pos);
                }
            }

            // Orphan kapanışı sonrası DB snapshot güncelle — kapatılan pozisyonlar bir sonraki
            // restart'ta tekrar işlenmesin.
            if let Some(conn) = &ls.db_conn {
                if ls.open_positions.is_empty() {
                    let _ = crate::database_writer::clear_open_positions_snapshot(conn);
                } else {
                    if let Ok(json) = serde_json::to_string(
                        &ls.open_positions.values().collect::<Vec<_>>()
                    ) {
                        let _ = crate::database_writer::save_open_positions_snapshot(conn, &json);
                    }
                }
            }
        }
    }

    /// Tek sembol için sinyal üretme + SL/TP kontrolü + emir gönderme döngüsü.
    #[cfg(not(target_arch = "wasm32"))]
    async fn process_symbol(&self, symbol: &str, market: Market, ls: &mut LoopState) {
        // Kalıcı engelleme listesi — bu semboller için pozisyon açılmaz (işlem kararı alınmaz)
        if self.config.blocked_symbols.iter().any(|b| b.eq_ignore_ascii_case(symbol)) {
            return;
        }

        // ── Dinamik Karantina Kontrolü ────────────────────────────────────────
        // Son 10 işlemde kümülatif negatif PnL → 24h otomatik karantina.
        if let Some(&expiry) = ls.dynamic_blacklist.get(symbol) {
            let now = std::time::Instant::now();
            if expiry > now {
                let secs = expiry.duration_since(now).as_secs();
                self.logger.log_info("blacklist", &format!(
                    "⛔ {} karantinada (son 10 işlem negatif), kalan: {}sa {}dk",
                    symbol, secs / 3600, (secs % 3600) / 60
                ));
                return;
            } else {
                ls.dynamic_blacklist.remove(symbol);
                self.logger.log_info("blacklist", &format!(
                    "✅ {} karantin sona erdi — trading yeniden aktif", symbol
                ));
            }
        }

        let candle_interval = &self.config.interval;
        let candle_limit    = self.config.candle_limit;

        // Per-sembol SL/TP: live_risk içindeki symbol tablosundan oku, yoksa global fallback
        let sym_risk: Option<RiskParams> = self.config.map_live(|s| {
            s.try_read_risk(|lrm| {
                let (sym_sl, sym_tp) = lrm.sl_tp(symbol);
                let changed = (sym_sl - self.config.risk_params.stop_loss_pct).abs() > 0.01
                    || (sym_tp - self.config.risk_params.take_profit_pct).abs() > 0.01;
                if changed { Some(RiskParams {
                    stop_loss_pct:     sym_sl,
                    take_profit_pct:   sym_tp,
                    trailing_stop_pct: if sym_sl > 0.0 { Some(sym_sl) } else { None },
                    ..self.config.risk_params.clone()
                })} else { None }
            })
        }).flatten().flatten();
        let risk_params: &RiskParams = sym_risk.as_ref().unwrap_or(&self.config.risk_params);
        let risk_mgr = RiskManager::new(risk_params.clone());

        // Candle verisi — önbellekten oku (REST çağrısı yok)
        let candles: Vec<Candle> = {
            let cc = ls.candle_cache.lock().unwrap();
            cc.get_latest(candle_interval, candle_limit)
        };
        if candles.is_empty() {
            self.logger.log_error("cache", &format!(
                "CandleCache boş [{}] {} — henüz veri yok, iterasyon atlandı",
                candle_interval, symbol
            ));
            return;
        }

        // ── Funding rate — yalnızca Futures/CoinM, 5 dk TTL önbellekli ────────
        if matches!(market, Market::Futures | Market::Coinm) {
            let stale = ls.funding_rate_cache.as_ref()
                .map(|(_, t)| t.elapsed().as_secs() > 300)
                .unwrap_or(true);
            if stale {
                match self.fetcher.fetch_funding_rate(market, symbol).await {
                    Ok(Some(fr)) => {
                        self.logger.log_info("funding", &format!(
                            "📈 {} funding rate: {:.5}%", symbol, fr.funding_rate * 100.0
                        ));
                        ls.funding_rate_cache = Some((fr, std::time::Instant::now()));
                    }
                    Ok(None) => {}
                    Err(e) => {
                        self.logger.log_error("funding", &format!("Funding rate çekilemedi: {}", e));
                    }
                }
            }
        }

        // ── HTF (üst zaman dilimi) candle'ları önbellekten oku — DB sorgusu yok ─
        // Mapping: 1m/5m → 1h | 15m/30m → 4h | 1h → 4h | 4h/1d → 1d
        let htf_candles: Option<Vec<crate::types::Candle>> = {
            let htf_interval = htf_for_interval(candle_interval);
            if htf_interval == candle_interval {
                None // zaten en üst TF
            } else {
                let cc = ls.candle_cache.lock().unwrap();
                let htf = cc.get_latest(htf_interval, 200);
                if htf.is_empty() { None } else { Some(htf) }
            }
        };

        // ── HTF trend yönünü hesapla ve live_risk'e yaz ───────────────────────
        // LTF'deki trend_bias() ile aynı MA periyotları kullanılır.
        let htf_bias: Option<TrendBias> = htf_candles.as_deref()
            .and_then(|c| trend_bias(c,
                self.config.quality.trend_short_period,
                self.config.quality.trend_long_period,
                self.config.quality.trend_margin_pct));
        {
            let bias_i8: Option<i8> = htf_bias.map(|b| match b {
                TrendBias::Bullish => 1,
                TrendBias::Bearish => -1,
                TrendBias::Neutral => 0,
            });
            self.config.with_live(|s| {
                if let Ok(mut lrm) = s.live_risk.write() {
                    lrm.htf_trend_bias = bias_i8;
                }
            });
        }

        // Veri tazeliği kontrolü
        // max_lag: 1m → 300sn, 5m → 1500sn; 30 dk'ya kadar tavan (API yavaşlamalarına tolerans)
        let last_ts  = candles.last().unwrap().timestamp;
        let now      = Utc::now();
        let max_lag  = (Self::interval_to_secs(candle_interval) * 5).min(1800).max(300);
        if (now - last_ts).num_seconds() as u64 > max_lag {
            self.logger.log_error("stale-data", &format!(
                "Veri geç geldi: {} saniye gecikme (limit: {}sn)", (now - last_ts).num_seconds(), max_lag
            ));
            return;
        }

        // NOT: live_price güncellemesi dış döngüde 1m verisiyle yapılıyor (daha taze).
        // İnterval candle close'u burada yazmak, 1m güncellemesinin üzerine bayat fiyat
        // yazarak birden fazla worker'ın çakışmasına (PnL titremesi) yol açar.

        // Fiyat önceliği: candle_close > live_arc > last_known
        let candle_close    = candles.last().map(|c| c.close).unwrap_or(0.0);
        let live_arc_price  = self.config.live_state.as_ref()
            .and_then(|s| s.live_price.read().ok())
            .filter(|p| p.close > 0.0 && (p.symbol.is_empty() || p.symbol == symbol))
            .map(|p| p.close)
            .unwrap_or(0.0);
        // Composite key kullan: "SYMBOL-Market" — düz sembol araması her zaman None döner
        let last_known_price = self.config.live_state.as_ref()
            .and_then(|s| s.live_positions.read().ok())
            .and_then(|lm| lm.get(&live_pos_key(symbol, &market)).map(|p| p.current_price))
            .unwrap_or(0.0);
        if candle_close <= 0.0 && live_arc_price <= 0.0 && last_known_price > 0.0 {
            self.logger.log_error("price", &format!(
                "⚠ {} fiyat kaynağı yok — son bilinen {:.4} kullanılıyor (SL/TP aktif)",
                symbol, last_known_price
            ));
        }
        let current_price = resolve_price(candle_close, live_arc_price, last_known_price);

        // ── S/R bölgelerini her tick'te TUI'ya aktar ───────────────────────────
        // (Aşağıdaki sinyal filtresi blokundaki S/R hesabı yalnızca Buy/Sell durumunda
        // ve bir dizi gate'ten sonra çalışır. HOLD veya cooldown'da update_sr_zones
        // çağrılmadığı için TUI'da sembol için zone listesi boş kalırdı. Bu kısım
        // her iterasyonda koşulsuz olarak güncel zone'ları yazar — maliyet ~1ms.)
        if self.config.sr_config.enabled && current_price > 0.0 {
            let det_pre = SrDetector::new(self.config.sr_config.clone());
            let ctx_pre = det_pre.context(&candles, current_price);
            let sr_sym = symbol.to_string();
            self.config.with_live(|s| s.update_sr_zones(&sr_sym, |zones| {
                *zones = ctx_pre.all_zones.clone();
            }));
        }

        // Trailing/Statik SL/TP Kontrolü
        let active_pos_id = ls.open_positions.iter()
            .find(|(_, p)| p.symbol == symbol && p.market == self.config.market)
            .map(|(id, _)| *id);
        if let Some(mut pos) = active_pos_id.and_then(|id| ls.open_positions.remove(&id)) {
            if let Some(exit_reason) = pos.update(current_price) {
                let close_signal = if pos.is_long { Signal::Sell } else { Signal::Buy };
                if ls.api_circuit_breaker.state() == CircuitBreakerState::Open {
                    self.logger.log_error("circuit_breaker", &format!(
                        "⚡ Circuit açık — {} kapanış emri atlandı", symbol
                    ));
                    ls.open_positions.insert(pos.id, pos);
                    return;
                }
                let close_trades = self.executor.execute_basket(close_signal, pos.qty);
                if close_trades.iter().any(|t| t.is_err()) {
                    let _ = ls.api_circuit_breaker.record_failure("close_basket");
                } else {
                    let _ = ls.api_circuit_breaker.record_success();
                }

                // Long kapanış = satış (is_buy=false), Short kapanış = alış (is_buy=true)
                // Adil çıkış fiyatı: SL/TP seviyesini kullan (anlık mum kapanış fiyatını değil).
                let sl_tp_price = match exit_reason {
                    "static_sl"   => pos.static_sl,
                    "trailing_sl" => pos.trailing_sl.unwrap_or(pos.static_sl),
                    _             => pos.static_tp, // "take_profit"
                };
                // Paper fill güvencesi: candle close ile gap yaşandıysa stop fill piyasaya kırp
                let fair_sl_tp = if exit_reason.contains("sl") {
                    if pos.is_long { sl_tp_price.min(current_price) }
                    else           { sl_tp_price.max(current_price) }
                } else { sl_tp_price };
                let effective_exit = self.config.execution_cost_config.as_ref()
                    .map(|ec| adjusted_price(fair_sl_tp, pos.qty, !pos.is_long, ec))
                    .unwrap_or(fair_sl_tp);

                let pnl = pos.realized_pnl_with_commission(effective_exit, self.config.commission_pct);
                ls.record_pnl_dir(pnl, pos.is_long);
                // Sembol bazlı ardışık kayıp takibi — seri kayıptan koruma (çok-sembol yolu)
                {
                    let counter = ls.symbol_consec_loss
                        .entry(symbol.to_string())
                        .or_insert(0);
                    if pnl < 0.0 { *counter += 1; } else { *counter = 0; }
                    let streak = *counter;
                    if streak >= 5 {
                        ls.symbol_cooldown_secs.insert(symbol.to_string(), 86400);
                        self.logger.log_error("loss-streak", &format!(
                            "🚨 {} {} ardışık kayıp — 24 saatlik işlem yasağı aktif",
                            symbol, streak
                        ));
                    } else if streak >= 3 {
                        ls.symbol_cooldown_secs.insert(symbol.to_string(), 7200);
                        self.logger.log_error("loss-streak", &format!(
                            "⚠ {} {} ardışık kayıp — 2 saatlik cooldown devreye girdi",
                            symbol, streak
                        ));
                    }
                }
                // Adaptive params auto-adjust: her N trade kapandığında çalıştır
                // N kontrolü auto_adjust içine taşındı (0-güvenli, fallback 20)
                if let Some(ap_path) = &self.config.adaptive_params_path.clone() {
                    let session_rr = if ls.session_wins > 0 && ls.session_closed > ls.session_wins {
                        ls.session_profit / ls.session_loss.max(1e-9)
                    } else { 1.0 };
                    let avg_win_pct = if ls.session_wins > 0 {
                        ls.session_profit / ls.session_wins as f64
                    } else { 0.0 };
                    ls.adaptive_params.auto_adjust(
                        ls.session_closed,
                        ls.session_wins,
                        ls.loss_streak,
                        session_rr,
                        ls.short_loss_streak,
                        avg_win_pct,
                        ap_path,
                    );
                }
                // SL ile kapandıysa cooldown + günlük SL sayacı başlat
                if exit_reason.contains("sl") {
                    // TSL kapanışı → daha uzun cooldown (30 dk) — volatilite henüz yüksek
                    let cd_secs = if exit_reason.contains("trailing") {
                        let tsl_cd = ls.sl_cooldown_secs.max(1800);
                        ls.symbol_cooldown_secs.insert(symbol.to_string(), tsl_cd);
                        tsl_cd
                    } else {
                        ls.sl_cooldown_secs
                    };
                    ls.sl_cooldown_map.insert(symbol.to_string(), std::time::Instant::now());
                    self.logger.log_info("cooldown", &format!(
                        "⏸ {} SL cooldown başladı ({} dk)", symbol, cd_secs / 60
                    ));
                    // SCP/SWG tip bazlı ek cooldown — ardışık SL'ye göre artar
                    {
                        let key = format!("{}_{}", symbol, pos.trade_type.label());
                        let count = ls.scalp_swing_consecutive_sl.entry(key.clone()).or_insert(0);
                        *count += 1;
                        let ss_cd_secs = ss_cooldown_secs(pos.trade_type, *count);
                        if ss_cd_secs > 0 {
                            ls.scalp_swing_sl_cooldown.insert(key.clone(), (std::time::Instant::now(), ss_cd_secs));
                            self.logger.log_info("ss-cooldown", &format!(
                                "⏸ [{}] {} SL sonrası tip-cooldown {} dk (ardışık={})",
                                pos.trade_type.label(), symbol, ss_cd_secs / 60, count
                            ));
                        }
                    }
                    // Günlük sembol SL sayacını artır
                    let today = chrono::Utc::now().date_naive();
                    let entry = ls.daily_sl_map.entry(symbol.to_string()).or_insert((0, today));
                    if entry.1 == today { entry.0 += 1; } else { *entry = (1, today); }
                }
                if let DrawdownStatus::LimitExceeded { current_dd, limit } = ls.dd_monitor.update_equity(ls.current_equity) {
                    self.logger.log_error("drawdown", &format!(
                        "🚨 Max drawdown aşıldı: {:.2}% / {:.2}% — trading durduruluyor", current_dd, limit
                    ));
                    crate::send_alert!(self.telegram,
                        "🚨 <b>MAX DRAWDOWN AŞILDI</b>\nMevcut: {:.2}% / Limit: {:.2}%\nTrading durduruluyor!",
                        current_dd, limit
                    );
                    ls.stop_loop = true;
                }
                let tsl_info = pos.trailing_sl.map(|tsl| format!(" tSL={:.2}", tsl)).unwrap_or_default();
                self.logger.log_info("trailing", &format!(
                    "Pozisyon kapandı ({}) | {} fiyat={:.2}→{:.2} entry={:.2} pnl={:+.2}{}",
                    exit_reason, symbol, current_price, effective_exit, pos.entry_price, pnl, tsl_info
                ));
                // Pozisyon kapanış bildirimi (WS fiyatıyla tetiklenen çıkışlar)
                crate::send_alert!(self.telegram,
                    "{} <b>{} {}</b> kapandı ({})\nGiriş: {:.4} → Çıkış: {:.4}\nPnL: {:+.2} USD",
                    if pnl >= 0.0 { "✅" } else { "❌" },
                    symbol, if pos.is_long { "LONG" } else { "SHORT" },
                    exit_reason, pos.entry_price, effective_exit, pnl
                );
                for t in close_trades {
                    if let Ok(trade) = t {
                        ls.total_trades += 1;
                        self.config.with_live(|s| s.inc_trade_count());
                        if trade.pnl.unwrap_or(pnl) > 0.0 { ls.win_trades += 1; }
                    }
                }
                // Fix D: TP kazancı → 2 saat ters yön bloğu + ardışık SL sayacını sıfırla
                if exit_reason == "take_profit" && pnl > 0.0 {
                    // TP'de consecutive SL sayacını sıfırla
                    {
                        use crate::robot::scalp_swing::TradeType;
                        if pos.trade_type != TradeType::Regular {
                            let key = format!("{}_{}", symbol, pos.trade_type.label());
                            ls.scalp_swing_consecutive_sl.remove(&key);
                        }
                    }
                    ls.tp_win_dir_map.insert(symbol.to_string(), (pos.is_long, std::time::Instant::now()));
                    self.logger.log_info("tp-dir-block", &format!(
                        "✅ {} TP kâr ({}) → 2h ters yön engellendi", symbol,
                        if pos.is_long { "LONG" } else { "SHORT" }
                    ));
                }
                ls.closed_position_ids.insert(pos.id); // duplicate guard
                self.close_position_and_log(&pos, self.config.market, effective_exit, pnl, exit_reason, ls);
                // Evrimsel Learning: gerçek PnL ile kapanışta çağır
                if self.config.autonomous_enabled {
                    let lev = pos.leverage.max(1.0);
                    let pnl_pct_evo = if pos.entry_price > 0.0 && pos.qty > 0.0 {
                        pnl * lev / (pos.entry_price * pos.qty) * 100.0
                    } else { 0.0 };
                    if let Some((regime, strategy)) = ls.pending_evo_data.remove(&pos.id) {
                        ls.autonomous_controller.learn_from_trade(pnl_pct_evo, &regime, &strategy);
                        self.maybe_evolve(ls);
                    }
                }
                return;
            } else {
                let tsl_str = pos.trailing_sl.map(|tsl| format!(" tSL={:.4}", tsl)).unwrap_or_default();
                self.logger.log_info("trailing", &format!(
                    "POS {} | fiyat={:.4} giriş={:.4} best={:.4}{}",
                    symbol, current_price, pos.entry_price, pos.best_price, tsl_str
                ));
                self.upsert_live_position(&pos, self.config.market, current_price);
                // Futures piyasasında flip (ters pozisyon) mümkün olduğundan pozisyonu
                // geçici olarak saklıyoruz — sinyal hesaplandıktan sonra flip karar verilecek.
                // Spot/diğerleri için eski davranış: hemen dön.
                if !matches!(self.config.market, Market::Futures | Market::Coinm) {
                    ls.open_positions.insert(pos.id, pos);
                    return;
                }
                ls.open_positions.insert(pos.id, pos);
                // Futures: sinyal hesabına düş, aşağıda flip kontrolü yapılacak.
            }
        }

        // ── Dinamik strateji sıralaması (composite score: Sharpe+Sortino+WR+PF+Calmar) ──
        use crate::evolution::MarketRegime;
        use crate::robot::optimizer::{rank_strategies_for_interval, interval_category, strategy_group};
        let active_strategy_name: String = self.config.map_live(|s| s.get_strategy())
            .unwrap_or_else(|| "MA".to_string());
        let regime = ls.autonomous_controller.adaptive_brain.as_ref()
            .map(|b| &b.current_regime)
            .unwrap_or(&MarketRegime::Unknown);
        let regime_label: &str = match regime {
            MarketRegime::LowVolatility  => "low_vol",
            MarketRegime::HighVolatility => "high_vol",
            _                            => "normal",
        };

        // Tüm stratejileri interval tümdengeline göre ağırlıklı composite skora göre sırala — Top 5 al
        // Örn: 5m → Momentum grubu (RSI/Stochastic/PriceAction) ön planda
        //      1h → Trend grubu (MACD/EMA/Supertrend) ön planda
        //      4h → Yapısal grup (ICT_FVG/SMC) ön planda
        //
        // OPT: rank_strategies_for_interval 16 strateji × 195 candle pozisyonu = ~3000
        // generate_signal çağrısı yapar. Candle timestamp değişmedikçe sonuç aynıdır;
        // önbellekten okunur, yeni candle kapandığında yeniden hesaplanır.
        let ranked = if ls.ranked_cache.as_ref()
            .map(|(ts, _)| *ts == last_ts)
            .unwrap_or(false)
        {
            ls.ranked_cache.as_ref().unwrap().1.clone()
        } else {
            let r = rank_strategies_for_interval(
                &candles,
                &self.config.strategy_params,
                htf_candles.as_deref(),
                8,
                Some(&self.config.interval),
            );
            ls.ranked_cache = Some((last_ts, r.clone()));
            r
        };

        // ── ADX / HMM Rejim Filtresi ─────────────────────────────────────────
        // Piyasa rejimini ADX + ATR ile tespit et. Rejim değişince log yaz;
        // değişmiyorsa sessiz kal (log gürültüsünü sıfıra indir).
        {
            let new_regime = crate::market_regime::detect_adx_regime(&candles);
            if new_regime != ls.adx_regime {
                let adx_val = crate::market_regime::compute_adx_from_candles(&candles);
                self.logger.log_info("regime", &format!(
                    "📊 ADX rejim: {} → {} (ADX={:.1}) [{}]",
                    ls.adx_regime, new_regime, adx_val, symbol
                ));
                ls.adx_regime = new_regime;
            }
        }
        // Per-sembol Volatile kontrolü — Regular daha toleranslı (10% vs 7% global)
        // Scalp/Swing per-symbol kontrollerine sahip, Regular için yerel ATR hesapla
        if let Some(local_atr) = average_range_pct(&candles, 14) {
            if local_atr > 10.0 {
                self.logger.log_info("regime", &format!(
                    "⛔ {} yerel ATR={:.2}% > 10% — Regular giriş engellendi", symbol, local_atr
                ));
                return;
            }
        }
        // Global Volatile rejim (ADX > 35) da engelle — ama 10% ATR eşiği alt sınırı oluştur
        let atr_from_candles = average_range_pct(&candles, 14).unwrap_or(0.0);
        if ls.adx_regime == crate::market_regime::AdxRegime::Volatile && atr_from_candles > 7.0 {
            self.logger.log_info("regime", &format!(
                "⛔ Volatile rejim (ATR={:.2}%) — yeni giriş engellendi [{symbol}]", atr_from_candles
            ));
            return;
        }
        // Rejim bazlı strateji filtresi KALDIRILDI — top-8 stratejinin tamamı oylama'ya katılır.
        // ADX/ATR filtresi voting'den önce zaten Volatile rejimi bloke ediyor (yukarıda).
        // Ranging/Trending için strateji kısıtlaması voting'i 1-2 stratejiye düşürüp sinyali öldürüyordu.
        let ranked = ranked;

        // Interval grubunu log'a yaz (ilk tur)
        let int_cat = interval_category(&self.config.interval);
        let _ = (int_cat, strategy_group); // suppress unused warning

        let regime_primary_name: String = ranked.first()
            .map(|(n, _)| n.clone())
            .unwrap_or_else(|| active_strategy_name.clone());

        // Parametre grid search (en iyi strateji için en iyi parametre seti)
        // OPT: grid_search_cache — 6 param seti × ~195 candle = ~1170 hesap/tick.
        // Candle timestamp değişmedikçe sonuç aynıdır; önbellekten okunur.
        let (best_params_owned, best_score, best_raw_score) = if ls.grid_search_cache
            .as_ref()
            .map(|(ts, _, _, _)| *ts == last_ts)
            .unwrap_or(false)
        {
            let (_, p, s, r) = ls.grid_search_cache.as_ref().unwrap();
            (p.clone(), *s, *r)
        } else {
            let mut score_best = f64::MIN;
            let mut raw_best   = f64::MIN;
            let mut params_best: StrategyParams = self.config.strategy_params.clone();
            let active_strat = make_strategy(&regime_primary_name);
            for (idx, params) in std::iter::once(&self.config.strategy_params)
                .chain(STATIC_PARAM_GRID.iter())
                .enumerate()
            {
                let raw = HyperOptimizer::simulate_score_htf(active_strat.as_ref(), &candles, params, htf_candles.as_deref());
                let score = if idx == 0 { raw + 0.1 } else { raw };
                if score > score_best {
                    score_best  = score;
                    raw_best    = raw;
                    params_best = params.clone();
                }
            }
            ls.grid_search_cache = Some((last_ts, params_best.clone(), score_best, raw_best));
            (params_best, score_best, raw_best)
        };
        let best_params    = &best_params_owned;

        // ── Yüzde tabanlı konsensüs oylama ───────────────────────────────────
        // Her strateji composite skorunu oy ağırlığı olarak kullanır.
        // BUY% = BUY oyu veren stratejilerin toplam ağırlığı / toplam pozitif ağırlık
        // Sinyal onaylanır ancak ilgili yön %50+ ağırlık toplarsa.
        let conf_params = best_params;
        // MA HyperOpt skoru çok düşükse (< 0.01) MA crossover voting'den çıkar.
        // RSI/BB/STOCH/MACD zaten optimize ediliyor; kör MA oylaması sinyali kirletir.
        let ma_hyp_score = self.config.map_live(|s| {
            s.try_read_risk(|lr| lr.hyperopt_score).unwrap_or(0.0)
        }).unwrap_or(0.0);
        let ma_excluded = ma_hyp_score < 0.01 && ma_hyp_score >= 0.0;
        let mut buy_weight  = 0.0_f64;
        let mut sell_weight = 0.0_f64;
        let mut total_weight = 0.0_f64;
        for (sn, sc) in &ranked {
            if ma_excluded && sn == "MA" { continue; }  // zayıf MA → voting'den çıkar
            // Taban ağırlık 0.1: composite=0 olan stratejiler de oylama'ya eşit oy hakkıyla katılır.
            // Önceki `w < 1e-6 { continue }` voting'i 1-2 stratejiye düşürüp sinyali öldürüyordu.
            let w = sc.composite.max(0.1);
            let s   = make_strategy(sn);
            let fr_slice: Option<&[crate::types::FundingRatePoint]> = ls.funding_rate_cache
                .as_ref().map(|(fr, _)| std::slice::from_ref(fr));
            let sig = s.generate_signal(&candles, conf_params, fr_slice, htf_candles.as_deref()).unwrap_or(Signal::Hold);
            total_weight += w;
            match sig {
                Signal::Buy  => buy_weight  += w,
                Signal::Sell => sell_weight += w,
                _            => {}
            }
        }
        // ── ML modeli oy — 19-öznitelik LinearRegressor'ı oylama sistemine katılır ──
        // Strateji oylamasına ek olarak ML skoru ağırlıklı oy verir.
        // Eğitilmiş ağırlıklar LiveRiskMap'ten okunur; yoksa with_defaults() kullanılır.
        // Güven (confidence) > 0.25 ise anlamlı; max toplam ağırlığın %25'i kadar etki.
        {
            // OPT: fv_cache — yeni candle kapanana kadar FeatureExtractor::extract (19 indikatör)
            // yeniden hesaplanmaz; aynı candle_ts için önbellekten okunur.
            let ml_fv = if ls.fv_cache.as_ref()
                .map(|(ts, _)| *ts == last_ts)
                .unwrap_or(false)
            {
                ls.fv_cache.as_ref().unwrap().1.clone()
            } else {
                let fv = FeatureExtractor::extract(&candles);
                ls.fv_cache = Some((last_ts, fv.clone()));
                fv
            };
            // Drift detector güncelle — her tick'te feature dağılımını izle
            ls.drift_detector.update(&ml_fv);
            // Adaptif eşik: ADX=0 (yatay) → 0.25 daha hassas, ADX=100 (güçlü trend) → 0.45 tolerant
            // Sabit 0.35 tüm rejimlerde aynı hassasiyeti uygulardı; trend döneminde yanlış alarm azaltılır.
            ls.drift_detector.threshold = (0.25 + (ml_fv.adx / 100.0) * 0.20).clamp(0.25, 0.45);
            let drift_scale = ls.drift_detector.confidence_scale();
            // Drift skoru LiveRiskMap'e yaz (TUI okur)
            self.config.map_live(|s| s.try_write_risk(|r| {
                r.ml_drift_score = ls.drift_detector.drift_score;
            }));
            // LiveRiskMap'ten eğitilmiş ağırlıkları oku
            let ml_model = self.config.map_live(|s| {
                s.try_read_risk(|r| {
                    r.ml_weights.map(|w| {
                        let mut m = LinearRegressor::with_defaults();
                        m.weights.copy_from_slice(&w);
                        m.bias = r.ml_bias_trained;
                        m.is_trained = true;
                        m
                    })
                }).flatten()
            }).flatten().unwrap_or_else(LinearRegressor::with_defaults);
            let trained = self.config.map_live(|s| s.try_read_risk(|r| r.ml_weights.is_some()))
                .flatten().unwrap_or(false);
            let ml_pred = ml_model.predict(&ml_fv);
            if ml_pred.confidence > 0.25 && total_weight > 0.0 {
                // Drift varsa ML ağırlığı azalt (kaymalı piyasada eski model yanıltır)
                let ml_w = ml_pred.confidence * total_weight * 0.25 * drift_scale;
                total_weight += ml_w;
                if ml_pred.score > 0.0 {
                    buy_weight  += ml_w;
                } else {
                    sell_weight += ml_w;
                }
                self.logger.log_info("ml-voter", &format!(
                    "ML oy: {} skor={:.3} conf={:.2} ağırlık=+{:.4} model={} drift={:.2}x (ATR={:.3} ADX={:.1} OBV={:.2})",
                    if ml_pred.score > 0.0 { "BUY" } else { "SELL" },
                    ml_pred.score, ml_pred.confidence, ml_w,
                    if trained { "trained" } else { "defaults" },
                    drift_scale,
                    ml_fv.atr_pct * 100.0, ml_fv.adx, ml_fv.obv_trend
                ));
            }
            // ── GBT oyunu — ML worker tarafından eğitilmiş GBT skoru ──────────
            if let Some(gbt_score) = self.config.map_live(|s| s.try_read_risk(|r| r.gbt_last_score)).flatten().flatten() {
                if gbt_score.abs() > 0.15 && total_weight > 0.0 {
                    let gbt_w = gbt_score.abs() * total_weight * 0.15; // max %15 etki
                    total_weight += gbt_w;
                    if gbt_score > 0.0 { buy_weight  += gbt_w; }
                    else               { sell_weight += gbt_w; }
                    self.logger.log_info("gbt-voter", &format!(
                        "GBT oy: {} skor={:.3} ağırlık=+{:.4}",
                        if gbt_score > 0.0 { "BUY" } else { "SELL" },
                        gbt_score, gbt_w
                    ));
                }
            }
        }

        let buy_pct  = if total_weight > 0.0 { buy_weight  / total_weight } else { 0.0 };
        let sell_pct = if total_weight > 0.0 { sell_weight / total_weight } else { 0.0 };
        // §15.9 Adaptif eşik: piyasa rejimine göre konsensüs eşiği ayarlanır
        // Ranging/HighVol → daha katı eşik (yanlış sinyal riski fazla)
        // StrongTrend     → daha gevşek eşik (trend sinyali güçlü olur)
        let top_composite = ranked.first().map(|(_, sc)| sc.composite).unwrap_or(0.0);
        let base_threshold = match regime {
            MarketRegime::Ranging       => 0.45,
            MarketRegime::HighVolatility => 0.50,
            MarketRegime::StrongUptrend | MarketRegime::StrongDowntrend => 0.33,
            _ => 0.38,
        };
        // Yüksek composite skor → eşiği %12 düşür (güçlü strateji sinyali üst oylamayı override etmemeli)
        let threshold = if top_composite > 0.80 {
            (base_threshold * 0.88_f64).max(0.30_f64)
        } else {
            base_threshold
        };

        let best_signal = if buy_pct >= threshold && buy_pct > sell_pct {
            Signal::Buy
        } else if sell_pct >= threshold && sell_pct > buy_pct {
            Signal::Sell
        } else {
            Signal::Hold
        };
        self.config.with_live(|s| s.set_regime_strategy(
            &format!("{} ({}) B={:.0}% S={:.0}% composite={:.3}",
                regime_primary_name, regime_label,
                buy_pct * 100.0, sell_pct * 100.0, top_composite)
        ));
        self.logger.log_info("sinyal", &format!(
            "Yüzde oylama ({} strateji): BUY={:.0}% SELL={:.0}% eşik={:.0}% → {}",
            ranked.len(), buy_pct * 100.0, sell_pct * 100.0, threshold * 100.0,
            match &best_signal { Signal::Buy => "BUY", Signal::Sell => "SELL", _ => "HOLD" }
        ));

        let confirmed_signal = best_signal.clone();
        let confirmed_signal = if !matches!(confirmed_signal, Signal::Hold) {
            confirmed_signal
        } else {
            Signal::Hold
        };

        let params_summary = format!(
            "fast={:?} slow={:?} period={:?} OB={:?} OS={:?} std={:?} mf={:?} ms={:?} msig={:?}",
            best_params.fast, best_params.slow, best_params.period,
            best_params.overbought, best_params.oversold, best_params.std_dev,
            best_params.fast_period, best_params.slow_period, best_params.signal_period
        );
        self.count_signal(SignalMetric::LastParams(
            format!("[HyperOpt] {}", params_summary)
        ));
        self.logger.log_info("strategy", &format!(
            "Seçilen strateji: {} | Rejim: {} | compare_strategies: {} | En iyi parametre: {:?}",
            regime_primary_name, regime_label, active_strategy_name, best_params
        ));

        let signal = if self.use_ml_signal {
            Signal::Hold
        } else {
            confirmed_signal
        };

        if matches!(signal, Signal::Hold) {
            self.count_signal(SignalMetric::Hold);
            // HOLD mesajını 60 saniyede bir kez logla — her tick tekrarı log gürültüsünü artırır.
            let should_log = ls.hold_log_throttle
                .map(|t| t.elapsed().as_secs() >= 60)
                .unwrap_or(true);
            if should_log {
                ls.hold_log_throttle = Some(std::time::Instant::now());
                self.logger.log_info("sinyal", &format!(
                    "HOLD — {} crossover/threshold koşulu sağlanmadı (en iyi skor: {:.4})",
                    active_strategy_name, best_score
                ));
            }
            return;
        }

        // Spot piyasasında SELL sinyali üretilmişse — pozisyon açılmadan hemen önce değil,
        // burada sustur; oylama logunu kirletmez ve filtre döngüleri gereksiz çalışmaz.
        if matches!(signal, Signal::Sell) && self.config.market == Market::Spot {
            self.count_signal(SignalMetric::Hold);
            return;
        }

        // MA/RSI/BB/MACD/DONCHIAN: parametreler (fast/slow) doğrudan HyperOpt ile optimize edilir
        //   → sıkı eşik (0.05) anlamlı.
        // ADX/VWAP/CCI/STOCHASTIC/STOCH_RSI/WILLIAMS/SUPERTREND/ICT/SMC/ELLIOTT/FUNDING:
        //   → fast/slow parametreleri birincil değil, HyperOpt skoru bu stratejiler için
        //     MA-optimizasyon merceğiyle değerlendirilir, yanıltıcı olur.
        //   → daha geniş eşik (-0.15) uygulanır.
        // MA/RSI/BB/MACD/DONCHIAN için eski eşik 0.05 çok agresifti:
        // raw_score=0.006 gibi pozitif ama küçük skoru bloke ediyordu (işlem kaçırma).
        // Negatif skor (-0.01 gate zaten var, bkz. HYPEROPT_BLOCK_THRESHOLD) → bu gate sadece
        // açıkça pozitif ama çok düşük skoru filtreler. 0.01 makul alt sınır.
        let hyperopt_min_score: f64 = match regime_primary_name.as_str() {
            "MA" | "RSI" | "BB" | "MACD" | "DONCHIAN" => 0.01,
            _ => -0.15,
        };
        if best_raw_score < hyperopt_min_score {
            self.count_signal(SignalMetric::BlockedRr(
                format!("HyperOpt raw_score={:.6} < {:.2} — güvensiz parametre", best_raw_score, hyperopt_min_score).into()
            ));
            self.logger.log_error("hyperopt", &format!(
                "HyperOpt skor={:.6} < {:.2} — {} işlem açılmadı",
                best_raw_score, hyperopt_min_score, regime_primary_name
            ));
            return;
        }

        let rr = if risk_params.stop_loss_pct > 0.0 {
            risk_params.take_profit_pct / risk_params.stop_loss_pct
        } else { 0.0 };
        // ── Asimetrik Min RR Guard ────────────────────────────────────────────
        // Hard floor: TP/SL < 2.0 olan trade'ler baştan reddedilir.
        // Config min_rr daha yüksekse o geçerli; 2.0 asla düşürülemez.
        const MIN_RR_GATE: f64 = 2.0;
        let effective_min_rr = ls.min_rr.max(MIN_RR_GATE);
        if rr < effective_min_rr {
            let reason = format!("R/R yetersiz (rr={:.2} < min={:.2})", rr, effective_min_rr);
            self.count_signal(SignalMetric::BlockedRr(reason.clone().into()));
            self.logger.log_error("rr-gate", &format!(
                "🚫 Asimetrik RR Guard: rr={:.2} < {:.2} (hard floor) — [{symbol}] trade reddedildi",
                rr, effective_min_rr
            ));
            return;
        }

        // ── 24h Volatilite Sapma Hesabı ──────────────────────────────────────
        // Son 20 bar return std / son 100 bar return std = vol_ratio
        // > 3.0 → kaotik volatilite, giriş yok; 2.0-3.0 → notional %50 küçültülecek.
        let vol_ratio: f64 = {
            let returns: Vec<f64> = candles.windows(2)
                .map(|w| if w[0].close > 0.0 { (w[1].close - w[0].close) / w[0].close } else { 0.0 })
                .collect();
            fn vol_stddev(v: &[f64]) -> f64 {
                if v.len() < 2 { return 0.0; }
                let mean = v.iter().sum::<f64>() / v.len() as f64;
                (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (v.len() - 1) as f64).sqrt()
            }
            let recent_n = 20usize.min(returns.len().saturating_sub(4).max(4));
            let base_n   = returns.len().min(100);
            if base_n < 5 || recent_n < 3 { 1.0 }
            else {
                let recent_std = vol_stddev(&returns[returns.len() - recent_n..]);
                let base_std   = vol_stddev(&returns[returns.len() - base_n..]);
                if base_std > 1e-9 { (recent_std / base_std).clamp(0.0, 10.0) } else { 1.0 }
            }
        };
        if vol_ratio > 3.0 {
            self.count_signal(SignalMetric::BlockedVolatility(
                format!("Kaotik volatilite {:.2}x normal", vol_ratio).into()
            ));
            self.logger.log_error("vol-guard", &format!(
                "🚨 Volatilite {:.2}x normal (eşik 3.0x) — [{symbol}] giriş engellendi",
                vol_ratio
            ));
            return;
        }

        if let Some(avg_range_pct) = average_range_pct(&candles, 20) {
            // Yüksek zaman dilimlerinde mumların doğal aralığı katlanarak büyür.
            // 1m baz alınarak interval'e göre eşikler ölçeklenir:
            //   1m→1x  5m→2x  15m→3x  30m→4.5x  1h→6x  4h→10x  1d→20x
            let iv_scale: f64 = match candle_interval.as_str() {
                "1m"  => 1.0,
                "3m"  => 1.5,
                "5m"  => 2.0,
                "15m" => 3.0,
                "30m" => 4.5,
                "1h"  => 6.0,
                "2h"  => 7.5,
                "4h"  => 10.0,
                "6h"  => 13.0,
                "12h" => 17.0,
                "1d"  => 20.0,
                _     => 1.0,
            };
            let vol_min = ls.volatility_min_pct * iv_scale;
            let vol_max = ls.volatility_max_pct * iv_scale;
            if avg_range_pct < vol_min || avg_range_pct > vol_max {
                let reason = format!("Volatilite bant dışı ({:.2}% / [{:.2}%,{:.2}%] @{})", avg_range_pct, vol_min, vol_max, candle_interval);
                self.count_signal(SignalMetric::BlockedVolatility(reason.clone().into()));
                self.logger.log_error("volatility", &format!(
                    "Volatilite bant dışı (avg_range={:.2}% / max={:.2}% @{}) - işlem atlandı",
                    avg_range_pct, vol_max, candle_interval
                ));
                return;
            }
        }

        // ── C2: Hacim filtresi — düşük hacimli barlarda sahte sinyal riski yüksek ────────────
        if self.config.quality.volume_filter_enabled {
            let vol_lookback = 20usize;
            let n = candles.len().min(vol_lookback);
            if n >= 2 {
                let recent = &candles[candles.len() - n..];
                let avg_vol: f64 = recent.iter().map(|c| c.volume).sum::<f64>() / n as f64;
                let cur_vol = candles.last().map(|c| c.volume).unwrap_or(0.0);
                if avg_vol > 0.0 && cur_vol < avg_vol * self.config.quality.volume_min_ratio {
                    self.count_signal(SignalMetric::BlockedVolatility(
                        format!("Hacim düşük ({:.0} < %{:.0} ort)", cur_vol, self.config.quality.volume_min_ratio * 100.0).into()
                    ));
                    self.logger.log_error("volume-filter", &format!(
                        "Hacim filtresi: mevcut={:.0} < ort*{:.2}={:.0} — sinyal engellendi",
                        cur_vol, self.config.quality.volume_min_ratio, avg_vol * self.config.quality.volume_min_ratio
                    ));
                    return;
                }
            }
        }

        // ── C3: RSI aşırı bölge filtresi — aşırı alım/satımda yeni giriş riskli ─────────────
        if self.config.quality.rsi_extreme_filter_enabled {
            let prices: Vec<f64> = candles.iter().map(|c| c.close).collect();
            if let Ok(rsi_val) = crate::robot::calculations::indicators::RSI::last(&prices, 14) {
                if matches!(signal, Signal::Buy) && rsi_val > self.config.quality.rsi_extreme_ob {
                    self.count_signal(SignalMetric::BlockedVolatility(
                        format!("RSI aşırı alım ({:.1})", rsi_val).into()
                    ));
                    self.logger.log_error("rsi-extreme", &format!(
                        "RSI aşırı alım ({:.1} > {:.0}) — BUY sinyali engellendi",
                        rsi_val, self.config.quality.rsi_extreme_ob
                    ));
                    return;
                }
                if matches!(signal, Signal::Sell) && rsi_val < self.config.quality.rsi_extreme_os {
                    self.count_signal(SignalMetric::BlockedVolatility(
                        format!("RSI aşırı satım ({:.1})", rsi_val).into()
                    ));
                    self.logger.log_error("rsi-extreme", &format!(
                        "RSI aşırı satım ({:.1} < {:.0}) — SELL sinyali engellendi",
                        rsi_val, self.config.quality.rsi_extreme_os
                    ));
                    return;
                }
            }
        }

        // ── HTF (Üst Zaman Dilimi) Trend Filtresi — büyük resim trende zıt girişleri engelle ─
        let htf_filter_enabled = self.config.map_live(|s| s.try_read_risk(|r| r.htf_filter_enabled))
            .flatten()
            .unwrap_or(true);
        if htf_filter_enabled {
            if let Some(bias) = htf_bias {
                let htf_interval_name = htf_for_interval(candle_interval);
                if matches!(signal, Signal::Buy) && bias == TrendBias::Bearish {
                    self.count_signal(SignalMetric::BlockedTrend("HTF Bearish — BUY engellendi"));
                    self.logger.log_error("htf-filter", &format!(
                        "HTF ({}) aşağı trend — BUY sinyali engellendi", htf_interval_name));
                    return;
                }
                // Futures SHORT politikası: HTF Bullish iken SELL, yerel trend de Bullish ise bloke et.
                // HTF Bullish + yerel Nötr/Bearish → reversal girişi olabilir → geçir.
                // Spot: SHORT mekanik imkânsız → bloke et (ileriki hard-wall zaten dönecek).
                if matches!(signal, Signal::Sell) && bias == TrendBias::Bullish {
                    let is_futures = matches!(self.config.market, Market::Futures | Market::Coinm);
                    if !is_futures {
                        // Spot: HTF Bullish'te SELL her zaman engellenir
                        self.count_signal(SignalMetric::BlockedTrend("HTF Bullish — SELL engellendi (spot)"));
                        self.logger.log_error("htf-filter", &format!(
                            "HTF ({}) yukarı trend — SELL sinyali engellendi (spot)", htf_interval_name));
                        return;
                    }
                    // Futures: yerel trend de Bullish ise engelle; Neutral/Bearish ise geç
                    let local_margin = self.config.quality.trend_margin_pct;
                    let local_bias = trend_bias(
                        &candles,
                        self.config.quality.trend_short_period,
                        self.config.quality.trend_long_period,
                        local_margin,
                    );
                    if local_bias == Some(TrendBias::Bullish) {
                        self.count_signal(SignalMetric::BlockedTrend("HTF+LTF Bullish — SELL engellendi (futures)"));
                        self.logger.log_error("htf-filter", &format!(
                            "HTF ({}) + LTF her ikisi Bullish — SELL engellendi", htf_interval_name));
                        return;
                    }
                    // HTF Bullish ama LTF Neutral/Bearish → geç, reversal girişi mümkün
                    self.logger.log_info("htf-filter", &format!(
                        "HTF ({}) Bullish ama LTF {:?} — futures SHORT geçirildi",
                        htf_interval_name, local_bias));
                }
                // ── C1: HTF hizalama filtresi — Neutral HTF yeterli değil, açık teyit gerekli ──
                if self.config.quality.htf_require_alignment {
                    let aligned = matches!(
                        (&signal, bias),
                        (Signal::Buy,  TrendBias::Bullish) |
                        (Signal::Sell, TrendBias::Bearish)
                    );
                    if !aligned {
                        self.count_signal(SignalMetric::BlockedTrend("HTF hizalama yok — sinyal engellendi"));
                        self.logger.log_error("htf-alignment", &format!(
                            "HTF ({}) teyidi yok ({:?}) — {:?} sinyali engellendi",
                            htf_interval_name, bias, signal
                        ));
                        return;
                    }
                }
            }
        }

        if self.config.quality.trend_filter_enabled {
            let trend_short_period = self.config.quality.trend_short_period;
            let trend_long_period  = self.config.quality.trend_long_period;
            // 1m/5m için gürültü toleransını 3× artır: kısa TF'de SMA(20/50) sinyal TF'den sık sapabilir.
            // Aksi hâlde MA crossover sinyalleri yerel trend filtresiyle sürekli çelişir → 0 trade.
            let trend_margin = {
                let t = candle_interval.trim().to_lowercase();
                let mins = if let Some(n) = t.strip_suffix('m') {
                    n.parse::<u64>().unwrap_or(60)
                } else { u64::MAX };
                if mins <= 5 {
                    self.config.quality.trend_margin_pct * 3.0
                } else {
                    self.config.quality.trend_margin_pct
                }
            };
            if let Some(trend_bias_val) = trend_bias(&candles, trend_short_period, trend_long_period, trend_margin) {
                if matches!(signal, Signal::Buy) && trend_bias_val == TrendBias::Bearish {
                    self.count_signal(SignalMetric::BlockedTrend("Trend Bearish — BUY engellendi"));
                    self.logger.log_error("trend", "Trend aşağı (Bearish) - BUY sinyali atlandı");
                    return;
                }
                // allows_short futures'ta SHORT mekanik imkânını açar, trend filtresini bypass etmez.
                // Bullish trendde SHORT sinyali her zaman engellenir; HTF bearish ise zaten htf_trend_filter geçer.
                if matches!(signal, Signal::Sell) && trend_bias_val == TrendBias::Bullish {
                    self.count_signal(SignalMetric::BlockedTrend("Trend Bullish — SELL engellendi"));
                    self.logger.log_error("trend", "Trend yukarı (Bullish) - SELL sinyali atlandı");
                    return;
                }
            }
        }

        // ── Mean-Reversion Strateji Filtresi ─────────────────────────────────
        // Mean-reversion stratejiler (STOCHASTIC, BB, RSI, STOCH_RSI, WILLIAMS, CCI)
        // yüksek zaman dilimi trende zıt sinyal üretebilir (oversold=BUY in bearish = fake).
        // HTF bias doğrudan bearish/bullish ise bu stratejilerin zıt yönlü sinyali bloke edilir.
        // Trend-following stratejiler (SUPERTREND, EMA, MACD, MA, DONCHIAN, ADX, VWAP) muaf tutulur.
        {
            let is_mean_reversion = matches!(
                active_strategy_name.as_str(),
                "STOCHASTIC" | "BB" | "RSI" | "STOCH_RSI" | "WILLIAMS" | "CCI"
            );
            if is_mean_reversion {
                if matches!(signal, Signal::Buy) && htf_bias == Some(TrendBias::Bearish) {
                    self.count_signal(SignalMetric::BlockedTrend(
                        "MeanRev+HTFBearish — BUY engellendi",
                    ));
                    self.logger.log_error("mean-rev-filter", &format!(
                        "⚠ {} mean-reversion strateji, HTF bearish trendde BUY sinyali üretemez — engellendi",
                        active_strategy_name
                    ));
                    return;
                }
                // Futures SHORT: overbought RSI + HTF Bullish zirve reversal fırsatı olabilir.
                // Spot'ta SELL kâr almak için gerekli olabilir — sadece spot+HTF Bullish kombosunda bloke et.
                if matches!(signal, Signal::Sell) && htf_bias == Some(TrendBias::Bullish) {
                    let is_futures = matches!(self.config.market, Market::Futures | Market::Coinm);
                    if !is_futures {
                        // Spot: mean-reversion SELL + HTF Bullish → engelle (sahte sinyal riski yüksek)
                        self.count_signal(SignalMetric::BlockedTrend(
                            "MeanRev+HTFBullish — SELL engellendi (spot)",
                        ));
                        self.logger.log_error("mean-rev-filter", &format!(
                            "⚠ {} spot, HTF bullish SELL engellendi", active_strategy_name
                        ));
                        return;
                    }
                    // Futures: mean-rev overbought sinyali reversal fırsatı — geçir
                    self.logger.log_info("mean-rev-filter", &format!(
                        "ℹ {} futures, HTF Bullish'e rağmen SELL geçirildi (overbought reversal)",
                        active_strategy_name
                    ));
                }
            }
        }

        // ── Uyarlamalı SHORT Koruma Filtreleri ────────────────────────────────
        // adaptive_params.json'dan yüklenen parametreler — her N trade sonrası otomatik güncellenir.
        if matches!(signal, Signal::Sell) {
            let ap = &ls.adaptive_params;

            // 1) HTF Bullish → SHORT tamamen engelle (htf_filter toggle'ından bağımsız)
            if ap.short_htf_block {
                if let Some(TrendBias::Bullish) = htf_bias {
                    self.count_signal(SignalMetric::BlockedTrend("AdaptiveHTF: Bullish — SHORT engellendi"));
                    self.logger.log_error("adaptive-short", &format!(
                        "🛡 Adaptive: HTF Bullish — {} SHORT engellendi (short_htf_block=true)", symbol
                    ));
                    return;
                }
            }

            // 2) Ardışık SHORT kaybı eşiği aşıldıysa yeni SHORT açma
            if ap.short_loss_streak_pause > 0 && ls.short_loss_streak >= ap.short_loss_streak_pause {
                self.count_signal(SignalMetric::BlockedTrend("AdaptiveLossStreak: SHORT duraklat"));
                self.logger.log_error("adaptive-short", &format!(
                    "🛡 Adaptive: {} ardışık SHORT kaybı ≥ eşik {} — yeni SHORT duraklat",
                    ls.short_loss_streak, ap.short_loss_streak_pause
                ));
                return;
            }

            // 3) Max eşzamanlı SHORT limiti
            if ap.max_concurrent_shorts > 0 {
                let open_shorts = ls.open_positions.values()
                    .filter(|p| !p.is_long && p.symbol == *symbol)
                    .count() as u32;
                // Tüm semboller için global short sayısı
                let total_open_shorts = ls.open_positions.values()
                    .filter(|p| !p.is_long)
                    .count() as u32;
                if total_open_shorts >= ap.max_concurrent_shorts {
                    self.count_signal(SignalMetric::BlockedTrend("AdaptiveMaxShorts: limit aşıldı"));
                    self.logger.log_error("adaptive-short", &format!(
                        "🛡 Adaptive: Açık SHORT sayısı {} ≥ limit {} ({})",
                        total_open_shorts, ap.max_concurrent_shorts, symbol
                    ));
                    let _ = open_shorts; // suppress unused warning
                    return;
                }
            }

            // 4) Futures SHORT için ek GBT skor eşiği (proxy ML confidence)
            // GBT skoru -1..+1; SHORT için negatif beklenir. Yeterince negatif değilse engelle.
            if ap.futures_short_min_conf > 0.0 && self.config.market == crate::types::Market::Futures {
                let gbt_score = self.config.map_live(|s| s.try_read_risk(|r| r.gbt_last_score))
                    .flatten().flatten();
                if let Some(score) = gbt_score {
                    // GBT skoru 0.000 ise model eğitilmemiş — bu gate'i atla
                    let model_trained = score.abs() > 0.01;
                    // SELL için skor negatif olmalı; mutlak değeri eşiğin altındaysa zayıf sinyal
                    if model_trained && (score >= 0.0 || score.abs() < ap.futures_short_min_conf) {
                        self.count_signal(SignalMetric::BlockedTrend("AdaptiveFuturesConf: GBT bearish yetki yok"));
                        self.logger.log_error("adaptive-short", &format!(
                            "🛡 Adaptive: Futures SHORT GBT={:.3} — bearish güven={:.3} < eşik={:.3} — {} engellendi",
                            score, score.abs(), ap.futures_short_min_conf, symbol
                        ));
                        return;
                    }
                }
            }
        }

        // ── Loss streak zaman bazlı decay ────────────────────────────────────────
        // Son kayıptan bu yana 3 saat geçtiyse global streak 1 azalt (minimum 0).
        // Bu mekanizma, gerçek trade olmadan sistemi kalıcı kilitlenme döngüsünden kurtarır.
        // Her 3 saatte bir 1 birim azalır: streak=5 → 3 saat → 4 → 6 saat → 3 ...
        {
            const STREAK_DECAY_SECS: u64 = 3 * 3600; // 3 saat
            if ls.loss_streak > 0 {
                let decay = ls.last_loss_time
                    .map(|t| t.elapsed().as_secs() / STREAK_DECAY_SECS)
                    .unwrap_or(0);
                if decay > 0 {
                    let old = ls.loss_streak;
                    ls.loss_streak = ls.loss_streak.saturating_sub(decay as usize);
                    // Zamanı ilerlet (decay kadar saat eklendi sayılır)
                    ls.last_loss_time = ls.last_loss_time.map(|t| {
                        t.checked_add(std::time::Duration::from_secs(decay * STREAK_DECAY_SECS))
                            .unwrap_or(t)
                    });
                    if ls.loss_streak < old {
                        self.logger.log_info("streak-decay", &format!(
                            "⏱ Global loss streak düştü: {} → {} ({}s geçti)",
                            old, ls.loss_streak, decay * STREAK_DECAY_SECS
                        ));
                    }
                }
            }
            if ls.short_loss_streak > 0 {
                let decay = ls.last_short_loss_time
                    .map(|t| t.elapsed().as_secs() / STREAK_DECAY_SECS)
                    .unwrap_or(0);
                if decay > 0 {
                    ls.short_loss_streak = ls.short_loss_streak.saturating_sub(decay as u32);
                    ls.last_short_loss_time = ls.last_short_loss_time.map(|t| {
                        t.checked_add(std::time::Duration::from_secs(decay * STREAK_DECAY_SECS))
                            .unwrap_or(t)
                    });
                }
            }
        }

        // ── Uyarlamalı LONG Koruma Filtreleri ────────────────────────────────────
        // short_htf_block'un LONG karşılığı — gece bearish piyasada LONG kaybını önler.
        if matches!(signal, Signal::Buy) {
            let ap = &ls.adaptive_params;

            // 0) Max eşzamanlı LONG limiti (korelasyon riski)
            // Sembol bazlı: aynı sembolde zaten LONG varsa blokla (çift pozisyon önleme).
            // Global: farklı semboller toplam max_concurrent_longs'a kadar açılabilir.
            // NOT: SCP/SWG farklı motorlar — Regular kendi pozisyonlarıyla sınırlanır.
            // Aynı sembolde scalp LONG açıkken Regular LONG yine açılabilir (farklı SL/TP).
            if ap.max_concurrent_longs > 0 {
                let symbol_already_long_reg = ls.open_positions.values()
                    .any(|p| p.is_long && p.symbol == symbol
                         && p.trade_type == crate::robot::scalp_swing::TradeType::Regular);
                if symbol_already_long_reg {
                    self.count_signal(SignalMetric::BlockedTrend("AdaptiveMaxLongs: sembol zaten Regular LONG"));
                    self.logger.log_error("adaptive-long", &format!(
                        "🛑 {} sembolünde zaten açık Regular LONG var — çift pozisyon engellendi",
                        symbol
                    ));
                    return;
                }
                let total_open_longs = ls.open_positions.values()
                    .filter(|p| p.is_long
                         && p.trade_type == crate::robot::scalp_swing::TradeType::Regular)
                    .count() as u32;
                if total_open_longs >= ap.max_concurrent_longs {
                    self.count_signal(SignalMetric::BlockedTrend("AdaptiveMaxLongs: global korelasyon limiti"));
                    self.logger.log_error("adaptive-long", &format!(
                        "🛑 Toplam açık LONG ({}) ≥ global limit ({}) — {} engellendi (korelasyon riski)",
                        total_open_longs, ap.max_concurrent_longs, symbol
                    ));
                    return;
                }
            }

            // 0b) Sembol bazlı seri kayıp koruması — aynı sembolde 3+ ardışık zarar → giriş durdur
            {
                let sym_key: &str = symbol;
                let sym_streak = ls.symbol_consec_loss
                    .get(sym_key)
                    .copied()
                    .unwrap_or(0);
                let sym_max = ap.max_concurrent_longs.max(1) + 2; // default: max_concurrent_longs+2 (en az 3)
                if sym_streak >= sym_max {
                    self.count_signal(SignalMetric::BlockedTrend("AdaptiveSymbolStreak: sembol kayıp serisi"));
                    self.logger.log_error("adaptive-long", &format!(
                        "🛑 {} sembolü {} ardışık zarar — yeni LONG engellendi (seri kayıp koruması)",
                        symbol, sym_streak
                    ));
                    return;
                }
            }

            // 1) HTF Bearish → LONG tamamen engelle
            if ap.long_htf_block {
                if let Some(TrendBias::Bearish) = htf_bias {
                    self.count_signal(SignalMetric::BlockedTrend("AdaptiveHTF: Bearish — LONG engellendi"));
                    self.logger.log_error("adaptive-long", &format!(
                        "🛡 Adaptive: HTF Bearish — {} LONG engellendi (long_htf_block=true)", symbol
                    ));
                    return;
                }
            }

            // 2) Global ardışık kayıp eşiği aşıldıysa tüm girişleri duraklat
            if ap.max_consecutive_losses > 0
                && ls.loss_streak >= ap.max_consecutive_losses as usize
            {
                self.count_signal(SignalMetric::BlockedTrend("AdaptiveMaxLoss: global kayıp serisi"));
                self.logger.log_error("adaptive-long", &format!(
                    "🛡 Adaptive: {} ardışık kayıp ≥ eşik {} — LONG duraklat",
                    ls.loss_streak, ap.max_consecutive_losses
                ));
                return;
            }
        }

        // ── SHORT için de global kayıp koruma ──────────────────────────────────
        if matches!(signal, Signal::Sell) {
            let ap = &ls.adaptive_params;
            if ap.max_consecutive_losses > 0
                && ls.loss_streak >= ap.max_consecutive_losses as usize
            {
                self.count_signal(SignalMetric::BlockedTrend("AdaptiveMaxLoss: global kayıp serisi (SHORT)"));
                self.logger.log_error("adaptive-short", &format!(
                    "🛡 Adaptive: {} ardışık kayıp ≥ eşik {} — SHORT duraklat",
                    ls.loss_streak, ap.max_consecutive_losses
                ));
                return;
            }
        }

        // ── Erken çıkış: S/R ve pattern gate öncesi hızlı kontroller ────────────
        // SrDetector::context() (CPU) ve pattern gate DB sorgusu pahalı işlemler.
        // HyperOpt gate, session filtresi ve cooldown kontrolleri onlardan önce
        // yapılarak gereksiz hesaplamalar atlanır.
        {
            // HyperOpt negatif skor — zararlı parametreler
            // Eşik: -0.01 (sıfıra yakın gürültü toleransı — -0.0009 vb. false-block önler)
            let hopt = self.config.map_live(|s| s.try_read_risk(|lr| lr.hyperopt_score))
                .flatten().unwrap_or(0.0);
            const HYPEROPT_BLOCK_THRESHOLD: f64 = -0.01;
            if hopt < HYPEROPT_BLOCK_THRESHOLD {
                self.logger.log_error("hyperopt-gate", &format!(
                    "🚫 HyperOpt skor={:.4} < {:.2} — {} sinyali engellendi (zararlı parametreler)",
                    hopt, HYPEROPT_BLOCK_THRESHOLD,
                    match signal { Signal::Buy => "BUY", Signal::Sell => "SELL", _ => "HOLD" }
                ));
                return;
            }

            // Seans / saat filtresi
            let is_sell = matches!(signal, Signal::Sell);
            let session_ok = self.config.map_live(|s| s.try_read_risk(|lr| {
                if !lr.session_filter_enabled { return true; }
                let hour = chrono::Utc::now().hour() as u8;
                if lr.session_blocked_hours.contains(&hour) { return false; }
                if !lr.session_allowed_hours.is_empty()
                    && !lr.session_allowed_hours.contains(&hour) { return false; }
                if is_sell && lr.session_long_preferred_hours.contains(&hour) { return false; }
                true
            })).flatten().unwrap_or(true);
            if !session_ok {
                self.logger.log_info("session-filter", &format!(
                    "⏰ {} sinyali saat {}:00 UTC'de engellendi (session_filter)",
                    match signal { Signal::Buy => "BUY", Signal::Sell => "SELL", _ => "HOLD" },
                    chrono::Utc::now().hour()
                ));
                return;
            }

            // Startup cooldown
            let startup_elapsed = ls.startup_time.elapsed().as_secs();
            if startup_elapsed < ls.startup_cooldown_secs {
                let remaining = ls.startup_cooldown_secs - startup_elapsed;
                self.logger.log_info("startup-cooldown", &format!(
                    "⏳ {} startup cooldown — {} sn kaldı, sinyal bekleniyor", symbol, remaining
                ));
                return;
            }
            // Min trade interval — aynı sembolde whipsaw koruması
            if let Some(min_secs) = self.config.min_trade_interval_secs {
                if let Some(&last_t) = ls.last_trade_time.get(symbol) {
                    let elapsed = last_t.elapsed().as_secs();
                    if elapsed < min_secs {
                        let remaining = min_secs - elapsed;
                        self.logger.log_info("trade-interval", &format!(
                            "⏳ {} min_trade_interval — {} sn kaldı (son giriş {}sn önce)",
                            symbol, remaining, elapsed
                        ));
                        return;
                    }
                }
            }
            // Flip cooldown (60 sn)
            if let Some(&flip_time) = ls.flip_cooldown_map.get(symbol) {
                let elapsed = flip_time.elapsed().as_secs();
                if elapsed < 60 {
                    let remaining = 60 - elapsed;
                    self.logger.log_info("flip-cooldown", &format!(
                        "⏸ {} flip cooldown aktif — {} sn kaldı, giriş engellendi", symbol, remaining
                    ));
                    return;
                } else {
                    ls.flip_cooldown_map.remove(symbol);
                }
            }
            // Fix D: TP kazancı sonrası 2 saat ters yön bloğu
            // Örnek: ATUSDT LONG TP → 2 saat SHORT engellendi
            if let Some(&(win_long, tp_time)) = ls.tp_win_dir_map.get(symbol) {
                let elapsed = tp_time.elapsed().as_secs();
                let block_secs: u64 = 7200; // 2 saat
                if elapsed < block_secs {
                    let is_opposite = (win_long && matches!(signal, Signal::Sell))
                        || (!win_long && matches!(signal, Signal::Buy));
                    if is_opposite {
                        let remaining = (block_secs - elapsed) / 60;
                        self.logger.log_info("tp-dir-block", &format!(
                            "🚫 {} ters yön engellendi (TP sonrası 2h koruma, {} dk kaldı)",
                            symbol, remaining
                        ));
                        return;
                    }
                } else {
                    ls.tp_win_dir_map.remove(symbol);
                }
            }
            // Günlük sembol SL limiti — aynı sembole bugün çok fazla SL yenildiyse bloke et
            {
                let max_daily = ls.adaptive_params.max_daily_sl_per_symbol;
                if max_daily > 0 {
                    let today = chrono::Utc::now().date_naive();
                    if let Some(&(count, date)) = ls.daily_sl_map.get(symbol) {
                        if date == today && count >= max_daily {
                            self.logger.log_info("daily-sl-block", &format!(
                                "🚫 {} bugün {} SL yedi (limit={}) — gece yarısına bloke",
                                symbol, count, max_daily
                            ));
                            return;
                        }
                    }
                }
            }

            // SL cooldown — sembol bazlı artan cooldown süresi (3+zarar→2h, 5+→24h)
            if let Some(&sl_time) = ls.sl_cooldown_map.get(symbol) {
                let cooldown_dur = ls.symbol_cooldown_secs.get(symbol).copied().unwrap_or(ls.sl_cooldown_secs);
                let elapsed = sl_time.elapsed().as_secs();
                if elapsed < cooldown_dur {
                    let remaining = cooldown_dur - elapsed;
                    self.logger.log_info("cooldown", &format!(
                        "⏸ {} cooldown aktif — {} dk {} sn kaldı, giriş engellendi",
                        symbol, remaining / 60, remaining % 60
                    ));
                    return;
                } else {
                    ls.sl_cooldown_map.remove(symbol);
                    ls.symbol_cooldown_secs.remove(symbol); // artan cooldown override'ı da temizle
                }
            }
        }

        // ── Destek/Direnç Filtresi ────────────────────────────────────────────
        let sr_ctx = if self.config.sr_config.enabled {
            let det = SrDetector::new(self.config.sr_config.clone());
            let ctx = det.context(&candles, current_price);
            // TUI'ya S/R bölgelerini aktar (sembol bazlı haritaya yaz)
            let sr_symbol = symbol.to_string();
            self.config.with_live(|s| s.update_sr_zones(&sr_symbol, |zones| {
                *zones = ctx.all_zones.clone();
            }));
            let is_buy = matches!(signal, Signal::Buy);
            if let Err(crate::robot::signal_evaluator::FilterBlock::SrQualityTooLow { quality, min_quality }) =
                check_sr_filter(
                    is_buy,
                    ctx.buy_quality, ctx.sell_quality,
                    self.config.sr_config.min_buy_quality,
                    self.config.sr_config.min_sell_quality,
                )
            {
                self.count_signal(SignalMetric::BlockedTrend("S/R kalite düşük — sinyal engellendi"));
                self.logger.log_error("sr-filter", &format!(
                    "S/R filtresi: quality={:.2} < min={:.2} — {} engellendi \
                     (destek={}, direnç={})",
                    quality, min_quality,
                    if is_buy { "BUY" } else { "SELL" },
                    ctx.nearest_support.as_ref()
                        .map(|z| format!("{:.2}", z.midpoint))
                        .unwrap_or_else(|| "-".to_string()),
                    ctx.nearest_resistance.as_ref()
                        .map(|z| format!("{:.2}", z.midpoint))
                        .unwrap_or_else(|| "-".to_string()),
                ));
                return;
            }
            Some(ctx)
        } else {
            None
        };

        // ── Futures flip kontrolü ─────────────────────────────────────────────
        // Futures'ta açık pozisyon ters sinyalle karşılaşırsa: kapat + yeni pozisyon aç.
        // Spot'ta flip olmaz (zaten yukarıda return etti).
        if matches!(self.config.market, Market::Futures | Market::Coinm)
            && !matches!(signal, Signal::Hold)
        {
            let flip_id = ls.open_positions.iter()
                .find(|(_, p)| {
                    p.symbol == symbol && p.market == self.config.market
                    && ((p.is_long && matches!(signal, Signal::Sell))
                        || (!p.is_long && matches!(signal, Signal::Buy)))
                })
                .map(|(id, _)| *id);

            if let Some(fid) = flip_id {
                if let Some(pos) = ls.open_positions.remove(&fid) {
                    let close_signal = if pos.is_long { Signal::Sell } else { Signal::Buy };
                    let _ = self.executor.execute_basket(close_signal, pos.qty);
                    let pnl = pos.realized_pnl_with_commission(current_price, self.config.commission_pct);
                    ls.record_pnl_dir(pnl, pos.is_long);
                    self.logger.log_info("flip", &format!(
                        "Flip: {} {} kapandı (sinyal) | fiyat={:.4} entry={:.4} pnl={:+.2} → {} açılıyor",
                        symbol, if pos.is_long { "LONG" } else { "SHORT" },
                        current_price, pos.entry_price, pnl,
                        if matches!(signal, Signal::Buy) { "LONG" } else { "SHORT" }
                    ));
                    ls.closed_position_ids.insert(pos.id); // duplicate guard
                    self.close_position_and_log(&pos, self.config.market, current_price, pnl, "signal_flip", ls);
                    // Evrimsel Learning: gerçek PnL ile kapanışta çağır (flip yolu)
                    if self.config.autonomous_enabled {
                        let lev = pos.leverage.max(1.0);
                        let pnl_pct_evo = if pos.entry_price > 0.0 && pos.qty > 0.0 {
                            pnl * lev / (pos.entry_price * pos.qty) * 100.0
                        } else { 0.0 };
                        if let Some((regime, strategy)) = ls.pending_evo_data.remove(&pos.id) {
                            ls.autonomous_controller.learn_from_trade(pnl_pct_evo, &regime, &strategy);
                            self.maybe_evolve(ls);
                        }
                    }
                    // Flip sonrası cooldown:
                    // - Kâr ile kapandıysa → 60 sn (normal)
                    // - Zararla kapandıysa → 300 sn (5 dk): yön tahminimiz yanlıştı,
                    //   hemen karşı yönde açmak double-loss üretir (ADAUSDT örneği: -5$→-11$)
                    let flip_cooldown_secs = if pnl < 0.0 { 300u64 } else { 60u64 };
                    ls.flip_cooldown_map.insert(symbol.to_string(), std::time::Instant::now());
                    // Zararlı flip: bu tick'te yeni pozisyon açılmasın (cooldown sonraki tick'te devreye girer,
                    // ama aynı tick'teki açılımı engellemek için symbol_cooldown_secs kullanıyoruz).
                    if pnl < 0.0 {
                        ls.symbol_cooldown_secs.insert(symbol.to_string(), flip_cooldown_secs);
                        self.logger.log_info("flip-cooldown", &format!(
                            "⏸ {} zararlı flip sonrası {} dk sembol cooldown (pnl={:+.2})",
                            symbol, flip_cooldown_secs / 60, pnl
                        ));
                        return; // bu tick'te yeni pozisyon açılmaz
                    }
                    self.logger.log_info("flip-cooldown", &format!(
                        "⏸ {} flip sonrası 60 sn cooldown başladı (pnl={:+.2})", symbol, pnl
                    ));
                }
            }
        }

        // HyperOpt gate + session filter + cooldown kontrolleri yukarıda erken çıkış
        // bloğuna taşındı (S/R hesaplamalarından önce çalışır — bkz. Fix-1).

        // ── Aynı yönde çift pozisyon engeli ──────────────────────────────────
        // Futures flip kontrolü yalnızca ters yönü kapatır. Eğer mevcut pozisyon ile
        // sinyal aynı yöndeyse (LONG iken tekrar BUY) yeni pozisyon açılmamalı.
        if !matches!(signal, Signal::Hold) {
            let same_dir = ls.open_positions.values().any(|p| {
                p.symbol == symbol && p.market == self.config.market
                && ((p.is_long  && matches!(signal, Signal::Buy))
                || (!p.is_long  && matches!(signal, Signal::Sell)))
            });
            if same_dir {
                return;
            }
        }

        // ── Pattern Gate ──────────────────────────────────────────────────────
        // DB'deki pattern_library'e bakarak mevcut piyasa koşulunun geçmişte ne kadar
        // karlı olduğunu kontrol eder. Eşiğin altında kalırsa sinyal Hold'a düşer.
        // pattern_gate_enabled = false ise kontrol atlanır (varsayılan: atla).
        let signal = if !matches!(signal, Signal::Hold) {
            let gate_enabled = self.config.map_live(|s| {
                s.try_read_risk(|lr| lr.pattern_gate_enabled)
            }).flatten().unwrap_or(false);

            if gate_enabled {
                let (stk, sos, sob) = self.config.map_live(|s| {
                    s.try_read_risk(|lr| (lr.global_stoch_k, lr.global_stoch_os, lr.global_stoch_ob))
                }).flatten().unwrap_or((6, 20.0, 80.0));

                let cond = MarketCondition::from_candles(&candles, stk, sos, sob);
                let cond_key = cond.key();
                let (t, v, m) = cond.parts();

                let strategy_name = self.config.map_live(|s| {
                    s.try_read_risk(|lr| lr.best_strategy_name.clone())
                }).flatten().unwrap_or_default();

                // ls.db_conn döngü boyunca açık — burada yeniden açılmaz (Fix-3)
                let pattern_ok = ls.db_conn.as_ref()
                    .and_then(|conn| {
                        crate::database_writer::query_best_pattern(
                            conn,
                            if strategy_name.is_empty() { "STOCHASTIC" } else { &strategy_name },
                            &self.config.interval,
                            match self.config.market {
                                Market::Spot => "spot",
                                Market::Futures => "futures",
                                Market::Coinm => "coinm",
                            },
                            &cond_key,
                            0.55,  // min %55 win rate
                            10,    // min 10 trade
                        )
                    });

                match pattern_ok {
                    Some((win_rate, avg_pnl, trade_count, _confidence)) => {
                        let conf = compute_confidence(win_rate, trade_count, avg_pnl);
                        if conf < 0.20 {
                            self.logger.log_info("pattern-gate", &format!(
                                "⚠ Pattern gate: {}|{}|{} → conf={:.2} düşük (wr={:.1}% n={}) — sinyal engellendi",
                                t, v, m, conf, win_rate * 100.0, trade_count
                            ));
                            Signal::Hold
                        } else {
                            self.logger.log_info("pattern-gate", &format!(
                                "✅ Pattern eşleşti: {}|{}|{} → conf={:.2} (wr={:.1}% n={})",
                                t, v, m, conf, win_rate * 100.0, trade_count
                            ));
                            signal
                        }
                    }
                    None => {
                        // Eşleşen pattern yok: veri birikene kadar işleme devam et
                        self.logger.log_info("pattern-gate", &format!(
                            "ℹ Pattern yok: {}|{}|{} — gate atlandı", t, v, m
                        ));
                        signal
                    }
                }
            } else {
                signal
            }
        } else {
            signal
        };

        // ── BTC/ETH Korelasyon Çapası ─────────────────────────────────────────
        // Altcoinlerde yalnızca BTC veya ETH ile yüksek korelasyon (> 0.7) varsa giriş izni.
        // Sembol BTC veya ETH ise kontrol atlanır. Cache süresi 5 dakika.
        // Fetch başarısız veya veri yetersizse filtre devre dışı bırakılır (graceful degradation).
        let is_btc_or_eth = symbol.eq_ignore_ascii_case("BTCUSDT")
            || symbol.eq_ignore_ascii_case("ETHUSDT")
            || symbol.eq_ignore_ascii_case("BTC")
            || symbol.eq_ignore_ascii_case("ETH");
        if !is_btc_or_eth {
            const ANCHOR_TTL_SECS: u64 = 300;
            // Altcoin'ler genelde BTC/ETH ile 0.0–0.5 arası korelasyona sahip;
            // 0.7 eşiği fazla agresifti; 0.4 hala çok kısıtlayıcı (negatif korelasyon
            // yeterli, sıfır veya zayıf pozitif korelasyon normal sayılmalı).
            const ANCHOR_MIN_CORR: f64 = 0.10;
            let anchor_limit = 30usize;

            // Pearson korelasyon (kapanış getirileri)
            fn pearson_corr(a_candles: &[crate::types::Candle], b_candles: &[crate::types::Candle]) -> f64 {
                let n = a_candles.len().min(b_candles.len());
                if n < 5 { return 0.0; }
                let a: Vec<f64> = a_candles[a_candles.len()-n..].windows(2)
                    .map(|w| if w[0].close > 0.0 { (w[1].close - w[0].close) / w[0].close } else { 0.0 })
                    .collect();
                let b: Vec<f64> = b_candles[b_candles.len()-n..].windows(2)
                    .map(|w| if w[0].close > 0.0 { (w[1].close - w[0].close) / w[0].close } else { 0.0 })
                    .collect();
                let nn = a.len().min(b.len());
                if nn < 4 { return 0.0; }
                let ma = a[..nn].iter().sum::<f64>() / nn as f64;
                let mb = b[..nn].iter().sum::<f64>() / nn as f64;
                let num: f64 = a[..nn].iter().zip(b[..nn].iter()).map(|(x, y)| (x-ma)*(y-mb)).sum();
                let da = a[..nn].iter().map(|x| (x-ma).powi(2)).sum::<f64>().sqrt();
                let db = b[..nn].iter().map(|x| (x-mb).powi(2)).sum::<f64>().sqrt();
                if da * db < 1e-12 { 0.0 } else { (num / (da * db)).clamp(-1.0, 1.0) }
            }

            // BTC anchor cache güncelle
            let stale_btc = ls.btc_anchor_cache.as_ref()
                .map(|(_, t)| t.elapsed().as_secs() > ANCHOR_TTL_SECS)
                .unwrap_or(true);
            if stale_btc {
                match self.fetcher.fetch_latest(crate::types::Exchange::Binance, market, "BTCUSDT", &self.config.interval, anchor_limit).await {
                    Ok(c) if !c.is_empty() => { ls.btc_anchor_cache = Some((c, std::time::Instant::now())); }
                    _ => { ls.btc_anchor_cache = None; }
                }
            }

            // ETH anchor cache güncelle
            let stale_eth = ls.eth_anchor_cache.as_ref()
                .map(|(_, t)| t.elapsed().as_secs() > ANCHOR_TTL_SECS)
                .unwrap_or(true);
            if stale_eth {
                match self.fetcher.fetch_latest(crate::types::Exchange::Binance, market, "ETHUSDT", &self.config.interval, anchor_limit).await {
                    Ok(c) if !c.is_empty() => { ls.eth_anchor_cache = Some((c, std::time::Instant::now())); }
                    _ => { ls.eth_anchor_cache = None; }
                }
            }

            // Korelasyon hesabı — her ikisi de düşükse giriş engelle
            let corr_btc = ls.btc_anchor_cache.as_ref()
                .map(|(bc, _)| pearson_corr(bc, &candles))
                .unwrap_or(1.0); // veri yoksa filtremeyi atla
            let corr_eth = ls.eth_anchor_cache.as_ref()
                .map(|(ec, _)| pearson_corr(ec, &candles))
                .unwrap_or(1.0);

            if corr_btc < ANCHOR_MIN_CORR && corr_eth < ANCHOR_MIN_CORR {
                self.count_signal(SignalMetric::BlockedVolatility(
                    format!("BTC/ETH çapa: corr_btc={:.2} corr_eth={:.2} < {:.2}", corr_btc, corr_eth, ANCHOR_MIN_CORR).into()
                ));
                self.logger.log_info("anchor", &format!(
                    "⚓ BTC/ETH çapası: corr_btc={:.2} corr_eth={:.2} < {:.2} — [{symbol}] akıntıya karşı, giriş engellendi",
                    corr_btc, corr_eth, ANCHOR_MIN_CORR
                ));
                return;
            }
        }

        // ── Cool-off: 3 ardışık SL sonrası 1 saatlik "Sadece İzleme" modu ──────
        // Pozisyon yönetimi (SL/TP) bu check'ten önce gerçekleşir; burada sadece YENİ GİRİŞ engellenir.
        if let Some(cooloff_end) = ls.cooloff_until {
            let now = std::time::Instant::now();
            if cooloff_end > now {
                let remaining_secs = cooloff_end.duration_since(now).as_secs();
                self.logger.log_info("cooloff", &format!(
                    "👁 Sadece İzleme — 3 ardışık SL sonrası cooldown, kalan: {}dk {}sn [{}]",
                    remaining_secs / 60, remaining_secs % 60, symbol
                ));
                return;
            } else {
                // Süre doldu → temizle
                ls.cooloff_until = None;
                self.logger.log_info("cooloff", &format!(
                    "✅ Cool-off sona erdi — trading yeniden aktif [{symbol}]"
                ));
            }
        }

        // ── TradePatternClassifier filtresi ─────────────────────────────────────
        if !matches!(signal, Signal::Hold) {
            let fv_snap = ls.fv_cache.as_ref()
                .map(|(_, fv)| fv.clone())
                .unwrap_or_else(|| FeatureExtractor::extract(&candles));

            let body_ratio = candles.last().map(|c| {
                let range = (c.high - c.low).abs();
                if range > 1e-9 { (c.close - c.open).abs() / range } else { 0.5 }
            }).unwrap_or(0.5);

            let trend_dir = if fv_snap.sma_20 > 1e-9 {
                (fv_snap.sma_5 / fv_snap.sma_20 - 1.0 + 0.05).clamp(0.0, 0.10) / 0.10
            } else { 0.5 };

            let rr = risk_params.take_profit_pct / risk_params.stop_loss_pct.max(0.01);

            let clf_inp = crate::robot::ml_engine::ClassifierInput {
                hour:       chrono::Utc::now().hour(),
                rsi:        fv_snap.rsi,
                atr_pct:    fv_snap.atr_pct * 100.0,  // fraction → yüzde
                vol_ratio:  fv_snap.volume_change,
                trend_dir,
                body_ratio,
                rr,
            };

            // Cold-start koruması: < 20 işlem, yüksek ATR veya yetersiz RR → engelle
            if ls.pattern_classifier.cold_start_blocks(clf_inp.atr_pct, rr) {
                self.logger.log_info("pattern-ml", &format!(
                    "🧊 Cold-Start Guard: giriş reddedildi — ATR={:.2}% RR={:.2} (henüz eğitilmedi, ihtiyatlı mod) [{}]",
                    clf_inp.atr_pct, rr, symbol
                ));
                self.count_signal(SignalMetric::BlockedVolatility(
                    format!("ColdStart ATR={:.2}% RR={:.2}", clf_inp.atr_pct, rr).into()
                ));
                return;
            }

            // Eğitilmişse rejim bazlı P(win) filtresi uygula
            if ls.pattern_classifier.is_trained
                && !ls.pattern_classifier.allows_entry_for_regime(&clf_inp, ls.adx_regime)
            {
                let x = crate::robot::ml_engine::TradePatternClassifier::to_features(&clf_inp);
                let prob = ls.pattern_classifier.win_probability(&x);
                let threshold_for_regime: f64 = match ls.adx_regime {
                    crate::market_regime::AdxRegime::Volatile => 0.60,
                    crate::market_regime::AdxRegime::Trending => 0.52,
                    crate::market_regime::AdxRegime::Neutral  => 0.55,
                    crate::market_regime::AdxRegime::Ranging  => 0.50,
                };
                self.logger.log_info("pattern-ml", &format!(
                    "🧠 PatternClassifier: giriş reddedildi — P(kazanç)={:.0}% < {:.0}% ({:?}) | RSI={:.1} ATR={:.2}% vol={:.2} trend={:.2} [{}]",
                    prob * 100.0, threshold_for_regime * 100.0, ls.adx_regime,
                    clf_inp.rsi, clf_inp.atr_pct, clf_inp.vol_ratio, clf_inp.trend_dir, symbol
                ));
                self.count_signal(SignalMetric::BlockedVolatility(
                    format!("PatternML P(win)={:.0}% regime={:?}", prob * 100.0, ls.adx_regime).into()
                ));
                return;
            }
        }

        // AI özerk kontrol: Regular bu rejimde devre dışıysa girişi engelle
        if !matches!(signal, Signal::Hold)
            && ls.strategy_scorer.is_disabled(crate::robot::scalp_swing::TradeType::Regular)
        {
            self.logger.log_info("ai-control", &format!(
                "🤖 Regular bu rejimde devre dışı ({}), giriş engellendi [{}]",
                ls.strategy_scorer.last_reason, symbol
            ));
            return;
        }

        // Trade uygula
        // (Startup / flip / SL cooldown kontrolleri erken çıkış bloğunda zaten yapıldı — bkz. Fix-1)
        if !matches!(signal, Signal::Hold) {
            // ── Giriş fiyatı: VWAP + WS tazeliğine göre REST ağırlıklandırma ─────
            //
            // WS taze (< 5 sn)  → entry_price = vwap (saf VWAP, spike filtresi)
            // WS gecikmeli (5-30 sn) → w*vwap + (1-w)*rest_mid (doğrusal karışım)
            // WS durdu (> 30 sn) → entry_price = rest_mid (REST Order Book Snapshot)
            //
            // rest_mid = bookTicker (best_bid + best_ask) / 2 — gerçek zamanlı emir defteri
            let vwap = {
                let n = candles.len().min(5);
                let tail = &candles[candles.len() - n..];
                let (num, den) = tail.iter().fold((0.0_f64, 0.0_f64), |(acc_n, acc_d), c| {
                    let tp = (c.high + c.low + c.close) / 3.0;
                    (acc_n + tp * c.volume, acc_d + c.volume)
                });
                if den > 0.0 { num / den } else { candles.last().map(|c| c.close).unwrap_or(0.0) }
            };
            // WS yaşını hesapla (milisaniye → saniye)
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let ws_age_secs = self.config.live_state.as_ref()
                .and_then(|s| s.live_price.read().ok())
                .map(|pd| {
                    if pd.last_updated_ms > 0 {
                        (now_ms.saturating_sub(pd.last_updated_ms)) / 1000
                    } else { 0 }
                })
                .unwrap_or(0);
            // Ağırlıklandırma katsayısı: WS taze ise w=1.0 (pure VWAP), stale ise w→0.0
            let ws_weight = if ws_age_secs < 5 {
                1.0_f64
            } else if ws_age_secs < 30 {
                1.0 - (ws_age_secs as f64 - 5.0) / 25.0
            } else {
                0.0_f64
            };
            // REST bookTicker mid-price (sadece weight < 1.0 ise çek; paperde WS ağırlığı 1.0)
            let rest_mid = if ws_weight < 1.0 {
                self.executor.executor
                    .fetch_book_ticker(symbol)
                    .ok()
                    .and_then(|(b, a)| if b > 0.0 && a > b { Some((b + a) / 2.0) } else { None })
                    .unwrap_or(vwap) // fetch başarısız → VWAP ile devam
            } else {
                vwap
            };
            let entry_price = ws_weight * vwap + (1.0 - ws_weight) * rest_mid;
            if ws_age_secs >= 5 {
                self.logger.log_info("price-weight", &format!(
                    "🔄 Fiyat ağırlıklandırma: WS_yaşı={}sn → WS_w={:.0}% VWAP={:.4} REST_mid={:.4} → entry={:.4} [{}]",
                    ws_age_secs, ws_weight * 100.0, vwap, rest_mid, entry_price, symbol
                ));
            }
            // ── Global max_open_positions kapısı ─────────────────────────────
            if let Some(max_pos) = self.config.max_open_positions {
                let current_open = ls.open_positions.len();
                if current_open >= max_pos {
                    self.logger.log_info("pos-cap", &format!(
                        "🛡 Portföy limiti: açık pozisyon sayısı {} / {} — yeni giriş engellendi [{}]",
                        current_open, max_pos, symbol
                    ));
                    self.count_signal(SignalMetric::BlockedVolatility(
                        format!("MaxPos {}/{}", current_open, max_pos).into()
                    ));
                    return;
                }
            }

            if entry_price > 0.0 {
                // ── Dinamik kaldıraç hesabı ───────────────────────────────────
                let (base_lev, max_lev) = self.config.map_live(|s| {
                    s.try_read_risk(|r| (r.base_leverage, r.max_leverage))
                }).flatten().unwrap_or((7.0, 10.0));

                let atr_pct_val = average_range_pct(&candles, 14);
                let dd_pct = if ls.peak_equity > 0.0 {
                    (ls.peak_equity - ls.current_equity) / ls.peak_equity * 100.0
                } else { 0.0 };
                // RR = ort. kazanç / ort. kayıp — spot'ta kaldıraç anlamsız (1x sabit)
                let session_losses = ls.session_closed - ls.session_wins;
                let session_rr = if ls.session_wins > 0 && session_losses > 0 {
                    (ls.session_profit / ls.session_wins as f64)
                        / (ls.session_loss / session_losses as f64)
                } else { 1.0 }; // henüz yeterli veri yok → nötr

                let effective_lev = compute_effective_leverage(
                    base_lev,
                    max_lev,
                    self.config.market,
                    htf_bias,
                    &signal,
                    atr_pct_val,
                    dd_pct,
                    best_raw_score,
                    session_rr,
                    ls.loss_streak,
                    ls.open_positions.len(),
                );
                // TUI/export'a gerçek zamanlı kaldıraç + session stats + cooldown aktar
                let cooldowns: Vec<(String, u64)> = ls.sl_cooldown_map.iter()
                    .filter_map(|(sym, t)| {
                        let elapsed = t.elapsed().as_secs();
                        let dur = ls.symbol_cooldown_secs.get(sym).copied().unwrap_or(ls.sl_cooldown_secs);
                        if elapsed < dur {
                            Some((sym.clone(), dur - elapsed))
                        } else { None }
                    }).collect();
                let s_closed = ls.session_closed;
                let s_wins   = ls.session_wins;
                let s_streak = ls.loss_streak;
                let s_rr     = session_rr;
                let eq_snapshot    = ls.current_equity;
                let peak_snapshot  = ls.peak_equity;
                let cum_pnl_snap   = ls.cumulative_pnl;
                // UCB1 Scorer snapshot — Tab 2 AI Merkezi için
                let scorer_sum   = ls.strategy_scorer.summary();
                let scorer_dis   = format!(
                    "Scalp{}  Swing{}  Reg{}",
                    if ls.strategy_scorer.scalp_disabled { "❌" } else { "✓" },
                    if ls.strategy_scorer.swing_disabled { "❌" } else { "✓" },
                    if ls.strategy_scorer.reg_disabled   { "❌" } else { "✓" },
                );
                let scorer_n     = ls.strategy_scorer.total_n;
                // GNB Classifier snapshot
                let clf_trained  = ls.pattern_classifier.is_trained;
                let clf_n_win    = ls.pattern_classifier.n_win;
                let clf_n_loss   = ls.pattern_classifier.n_loss;
                let clf_buf_len  = ls.classifier_buffer.len();
                self.config.map_live(|s| s.try_write_risk(|r| {
                    r.effective_leverage     = effective_lev;
                    r.session_closed         = s_closed;
                    r.session_wins           = s_wins;
                    r.loss_streak            = s_streak;
                    r.session_rr             = s_rr;
                    r.sl_cooldowns           = cooldowns;
                    r.current_equity         = eq_snapshot;
                    r.peak_equity            = peak_snapshot;
                    r.cumulative_pnl         = cum_pnl_snap;
                    r.scorer_summary         = scorer_sum;
                    r.scorer_disabled        = scorer_dis;
                    r.scorer_total_n         = scorer_n;
                    r.classifier_trained     = clf_trained;
                    r.classifier_n_win       = clf_n_win;
                    r.classifier_n_loss      = clf_n_loss;
                    r.classifier_buffer_len  = clf_buf_len;
                }));

                // ── SL güvenlik klampı ────────────────────────────────────────
                // Tasfiye noktasından önce çıkmak için: max_sl = %80 / leverage
                // Örn: 10x → max %8 SL; 7x → max %11.4 SL
                let max_sl_pct = 80.0 / effective_lev;

                // Auto-sizing: explicit miktar yoksa max_notional'ı aşmayacak şekilde hesapla.
                // base_qty × effective_lev × price ≤ effective_max_not × 0.85 (güvenlik marjı)
                // max_position_size_pct ile notional equity bazlı kısıtlanır (örn. %10 → $1,000/pozisyon)
                let base_qty = if let Some(explicit_amt) = self.config.trade_amount {
                    explicit_amt
                } else {
                    let max_not = ls.risk_gate.policy.max_notional_usd;
                    let effective_max_not = if let Some(pct) = self.config.risk_params.max_position_size_pct {
                        max_not.min(ls.current_equity * pct / 100.0)
                    } else {
                        max_not
                    };
                    if entry_price > 0.0 && effective_lev > 0.0 {
                        (effective_max_not * 0.85 / (effective_lev * entry_price)).max(0.001)
                    } else {
                        risk_mgr.calculate_position_size(ls.current_equity, entry_price, None, None).unwrap_or(0.0)
                    }
                };

                // §15.7b ML confidence → dinamik pozisyon boyutu skalası
                // confidence 0.0-1.0 → pozisyon %75-%125 arasında ölçeklenir
                // GBT ve LR oylarının ağırlıklı ortalaması kullanılır
                let ml_confidence_scale: f64 = {
                    let lr_conf = self.config.map_live(|s| {
                        s.try_read_risk(|r| r.ml_weights.map(|_| {
                            // Eğitilmiş model varsa, son ML voter confidence'ını LiveRiskMap'ten okuyamayız —
                            // ama GBT skoru var; LR ve GBT birleşik sinyal gücü:
                            let gbt_abs = r.gbt_last_score.map(|s| s.abs()).unwrap_or(0.0);
                            // LR ağırlıkların L1 norm'u bir proxy confidence (normalized 0-1)
                            let lr_w_sum: f64 = r.ml_weights.unwrap().iter().map(|w| w.abs()).sum::<f64>();
                            let lr_norm = (lr_w_sum / 19.0).min(1.0);
                            (lr_norm * 0.6 + gbt_abs * 0.4).clamp(0.0, 1.0)
                        }))
                    }).flatten().flatten().unwrap_or(0.5);
                    // 0.0→0.75x, 0.5→1.0x, 1.0→1.25x
                    0.75 + lr_conf * 0.50
                };
                let base_qty = base_qty * ml_confidence_scale;
                if (ml_confidence_scale - 1.0).abs() > 0.05 {
                    self.logger.log_info("ml-sizing", &format!(
                        "ML confidence skalası: {:.2}x → qty={:.6} (ham={:.6})",
                        ml_confidence_scale, base_qty, base_qty / ml_confidence_scale
                    ));
                }

                // §15.8b Kelly criterion — canlı session performansına dayalı boyut skalası
                // En az 20 kapanmış trade varsa ve use_kelly_criterion aktifse devreye girer.
                // Half-Kelly: f* = (b×p−q)/b → yarıya indir → scale = 1.0 + 0.5×f* ∈ [0.5, 1.5]
                // Negatif edge (kazanma < 50% ve düşük RR) → 0.5× (sistem küçülür)
                // Pozitif edge (yüksek win rate + RR) → 1.5× (sistem büyür)
                let base_qty = if self.config.risk_params.use_kelly_criterion
                    && ls.session_closed >= 20
                {
                    let session_losses = ls.session_closed.saturating_sub(ls.session_wins);
                    let wr  = ls.session_wins as f64 / ls.session_closed as f64;
                    let avg_win  = if ls.session_wins > 0 {
                        ls.session_profit / ls.session_wins as f64
                    } else { 0.0 };
                    let avg_loss = if session_losses > 0 {
                        ls.session_loss / session_losses as f64
                    } else { avg_win.max(1.0) };
                    let wlr = if avg_loss > 1e-9 { avg_win / avg_loss } else { 1.0 };
                    let b   = wlr;
                    let q   = 1.0 - wr;
                    let raw_kelly = if b > 1e-9 { ((b * wr) - q) / b } else { 0.0 };
                    let half_kelly = raw_kelly.clamp(-1.0, 1.0) * 0.5;
                    let kelly_scale = (1.0 + half_kelly).clamp(0.5, 1.5);
                    if (kelly_scale - 1.0).abs() > 0.05 {
                        self.logger.log_info("kelly", &format!(
                            "Kelly skalası: {:.2}x (wr={:.0}% wlr={:.2} f*={:.3}) trades={}",
                            kelly_scale, wr * 100.0, wlr, raw_kelly, ls.session_closed
                        ));
                    }
                    base_qty * kelly_scale
                } else {
                    base_qty
                };

                // §15.8a Ardışık kayıp skalası — loss streak arttıkça pozisyon boyutu küçülür
                // 5→%80, 7→%64, 9→%51; kazanç gelince loss_streak sıfırlanır ve boyut eski haline döner
                let base_qty = if ls.loss_streak >= 5 {
                    let scale = 0.80f64.powi(((ls.loss_streak - 4) / 2).max(1) as i32).max(0.25);
                    if scale < 0.99 {
                        self.logger.log_info("watchdog", &format!(
                            "⚠ {} ardışık zarar → lot skalası {:.2}x uygulandı",
                            ls.loss_streak, scale
                        ));
                    }
                    base_qty * scale
                } else { base_qty };

                // §15.8 Kademeli drawdown koruması
                // dd>20% → yeni pozisyon açma; dd>15% → yarı büyüklük
                let current_dd_pct = if ls.dd_monitor.peak_equity > 0.0 {
                    ((ls.dd_monitor.peak_equity - ls.current_equity) / ls.dd_monitor.peak_equity) * 100.0
                } else {
                    0.0
                };
                if current_dd_pct > 20.0 {
                    self.logger.log_error("drawdown", &format!(
                        "⛔ Drawdown {:.1}% > 20% — yeni pozisyon engellendi", current_dd_pct
                    ));
                    return;
                }
                let base_qty = if current_dd_pct > 15.0 {
                    self.logger.log_info("drawdown", &format!(
                        "⚠ Drawdown {:.1}% > 15% — pozisyon boyutu yarıya indirildi", current_dd_pct
                    ));
                    base_qty * 0.5
                } else {
                    base_qty
                };

                // §15.x Volatility-Adjusted Notional (Stddev tabanlı)
                // vol_ratio 2.0-3.0 arası → notional %50 küçült (> 3.0 yukarıda zaten engellendi).
                // Örnek: RAVE volatilitesi normalin 2.5x → lot = base * 0.5
                let base_qty = if vol_ratio > 2.0 {
                    self.logger.log_info("vol-guard", &format!(
                        "⚠ Volatilite {:.2}x normal → notional %50 küçültüldü [{symbol}]",
                        vol_ratio
                    ));
                    base_qty * 0.5
                } else { base_qty };

                // ── Günlük equity kayıp limiti ──────────────────────────────────────
                // Günün başından bu yana kayıp > 3% → bugün yeni pozisyon açma.
                // RiskGate max_daily_loss'tan bağımsız ek bir güvenlik katmanı.
                // SANITY: equity henüz inisiyalize edilmemiş veya kasıtlı küçükse
                // (ör. paper-mode cold-start) yüzde anlamsız olur — guard'ı atla.
                const DAILY_LOSS_MIN_EQUITY: f64 = 10.0;
                let equities_valid = ls.day_start_equity >= DAILY_LOSS_MIN_EQUITY
                    && ls.current_equity >= DAILY_LOSS_MIN_EQUITY;
                let day_loss_pct = if equities_valid {
                    (ls.day_start_equity - ls.current_equity) / ls.day_start_equity * 100.0
                } else { 0.0 };
                if equities_valid && day_loss_pct > 3.0 {
                    self.logger.log_error("daily-loss", &format!(
                        "⛔ Günlük kayıp {:.2}% > 3% (gün başı=${:.0} şimdi=${:.0}) — bugün yeni pozisyon engellendi",
                        day_loss_pct, ls.day_start_equity, ls.current_equity
                    ));
                    return;
                }

                // §15.12 HTF verisi bekleniyor ama henüz DB'de yok → pozisyon açma
                // Yalnızca: db_path konfigüre edilmiş + bu TF'nin bir üstü var + veri henüz birikmemiş
                let htf_expected = self.config.db_path.is_some()
                    && htf_for_interval(candle_interval) != candle_interval;
                if htf_expected && htf_candles.is_none() {
                    self.logger.log_info("htf", &format!(
                        "⏳ {} HTF verisi henüz DB'de yok — pozisyon bekletiliyor", symbol
                    ));
                    return;
                }

                // §15.13 WS warm-up: canlı fiyat tick'i alınana kadar yeni pozisyon açma.
                // DB verisiyle (eski candle close) işleme girmesini önler; WS'ten ilk tick gelince geçer.
                let ws_ready = self.config.live_state.as_ref()
                    .and_then(|s| s.live_price.read().ok())
                    .map(|p| p.close > 0.0 && !p.ts.is_empty())
                    .unwrap_or(false);
                if !ws_ready {
                    self.logger.log_info("warmup", &format!(
                        "⏳ {} WS fiyatı henüz gelmedi — ilk tick bekleniyor", symbol
                    ));
                    return;
                }

                // Kaldıraçlı notional: aynı marjin × leverage kadar daha büyük pozisyon
                let qty = base_qty * effective_lev;
                if qty > 0.0 {
                    if self.config.autonomous_enabled {
                        let requested_notional = qty * entry_price;
                        let risk_input = RiskInput {
                            account_equity:          ls.current_equity,
                            day_start_equity:        ls.day_start_equity,
                            peak_equity:             ls.peak_equity,
                            requested_notional_usd:  requested_notional,
                            model_confidence: if matches!(signal, Signal::Hold) { 0.50 } else { 0.75 },
                        };
                        match ls.risk_gate.evaluate(risk_input) {
                            RiskDecision::Allow => {}
                            RiskDecision::Deny { reasons, enter_safe_mode, halt } => {
                                let reason_text = reasons.join(" | ");
                                self.count_signal(SignalMetric::BlockedRiskGate(
                                    format!("RiskGate: {}", reason_text)
                                ));
                                self.logger.log_error("autonomous-risk", &reason_text);
                                let _ = ls.autonomous_controller.transition_failure(&reason_text);
                                if halt {
                                    self.logger.log_error("autonomous", "Risk gate HALT kararı verdi");
                                    ls.stop_loop = true;
                                    return;
                                }
                                if enter_safe_mode {
                                    // FSM'i anında SafeMode'a al — threshold dolmayı bekleme.
                                    ls.autonomous_controller.force_safe_mode(0);
                                    self.logger.log_error("autonomous",
                                        "Risk gate SAFE MODE → FSM anında SafeMode'a alındı");
                                }
                                return;
                            }
                        }
                    }

                    let is_long_entry = matches!(signal, Signal::Buy);

                    // Spot piyasasında açığa satış (SHORT) yapılamaz
                    if !is_long_entry && self.config.market == Market::Spot {
                        self.config.map_live(|s| s.try_write_risk(|r| {
                            r.spot_sell_blocks = r.spot_sell_blocks.saturating_add(1);
                        }));
                        self.logger.log_error("market", &format!(
                            "⚠ {} Spot piyasasında SHORT açılamaz — SELL sinyali atlandı", symbol
                        ));
                        return;
                    }

                    self.count_signal(if is_long_entry { SignalMetric::Buy } else { SignalMetric::Sell });

                    if ls.api_circuit_breaker.state() == CircuitBreakerState::Open {
                        self.logger.log_error("circuit_breaker", &format!(
                            "⚡ Circuit açık — {} yeni emir atlandı", symbol
                        ));
                        return;
                    }

                    // ── Execution Gate: WS / REST Senkronizasyon Kontrolü ────────────
                    // İki aşamalı kontrol:
                    //   1. WS vs entry_price (VWAP veya ağırlıklı fiyat) sapması > eşik → engelle
                    //   2. WS stale ise → REST bookTicker ortasıyla karşılaştır (daha sıkı eşik)
                    //
                    // HIGHUSDT gibi anlık spike'larda WS %0.7 sapabilir; bu blok o durumu yakalar.
                    let exec_ws_price = self.config.live_state.as_ref()
                        .and_then(|s| s.live_price.read().ok())
                        .map(|pd| pd.close)
                        .unwrap_or(0.0);
                    if exec_ws_price > 0.0 && entry_price > 0.0 {
                        // Normal eşik: %0.5 (kripto hızlı hareket) | WS stale ise %0.3
                        let exec_threshold = if ws_age_secs >= 5 { 0.003 } else { 0.005 };
                        let exec_div = (exec_ws_price - entry_price).abs() / entry_price;
                        if exec_div > exec_threshold {
                            self.count_signal(SignalMetric::BlockedRiskGate(
                                format!("ExecGate: Sapma {:.3}% > {:.2}% eşik", exec_div * 100.0, exec_threshold * 100.0)
                            ));
                            self.logger.log_error("exec-gate", &format!(
                                "🚫 Execution Gate — emir engellendi: WS={:.4} entry={:.4} sapma={:.3}% (eşik={:.2}%) WS_yaş={}sn [{}]",
                                exec_ws_price, entry_price, exec_div * 100.0, exec_threshold * 100.0, ws_age_secs, symbol
                            ));
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            return;
                        }
                        // REST bookTicker çapraz kontrolü (sadece live modda, WS stale değilken)
                        // Hem WS hem REST eşzamanlı ama birbiriyle çelişiyorsa → fiyat karmaşık → dur
                        if ws_age_secs < 5 && !self.paper_mode {
                            if let Ok((bt_bid, bt_ask)) = self.executor.executor.fetch_book_ticker(symbol) {
                                if bt_bid > 0.0 && bt_ask > bt_bid {
                                    let bt_mid = (bt_bid + bt_ask) / 2.0;
                                    let ws_rest_div = (exec_ws_price - bt_mid).abs() / bt_mid;
                                    if ws_rest_div > 0.002 {  // WS vs REST > %0.2 → ciddi sapma
                                        self.count_signal(SignalMetric::BlockedRiskGate(
                                            format!("ExecGate: WS/REST sapma {:.3}%", ws_rest_div * 100.0)
                                        ));
                                        self.logger.log_error("exec-gate", &format!(
                                            "🚫 WS/REST Çapraz Kontrol başarısız: WS={:.4} BookTicker_mid={:.4} sapma={:.3}% > %0.2 [{}]",
                                            exec_ws_price, bt_mid, ws_rest_div * 100.0, symbol
                                        ));
                                        tokio::time::sleep(Duration::from_millis(500)).await;
                                        return;
                                    }
                                }
                            }
                        }
                    }

                    // Emir modu loglanır
                    self.logger.log_info("executor", &format!(
                        "{} LIMIT-MAKER ORDER → {} {:?} qty={:.6} @ VWAP={:.4}",
                        if self.paper_mode { "PAPER" } else { "LIVE" },
                        symbol, signal, qty, entry_price
                    ));
                    // Giriş: SmartLimit (Best_Bid+1tick, maks 3 re-quote, 2 sn/deneme)
                    // Çıkış/SL/TP emirleri execute_basket (MARKET) ile gönderilir.
                    let trades = self.smart_limit_entry(signal, symbol, qty, entry_price).await;
                    if trades.iter().any(|t| t.is_err()) {
                        let _ = ls.api_circuit_breaker.record_failure("open_basket");
                    } else {
                        let _ = ls.api_circuit_breaker.record_success();
                    }

                    let mut position_opened = false;
                    for t in trades {
                        match t {
                            Ok(trade) => {
                                ls.total_trades += 1;
                                if let Some(ref lstate) = self.config.live_state {
                                    lstate.live_trade_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                }
                                ls.record_pnl(trade.pnl.unwrap_or(0.0));
                                if let DrawdownStatus::LimitExceeded { current_dd, limit } = ls.dd_monitor.update_equity(ls.current_equity) {
                                    self.logger.log_error("drawdown", &format!(
                                        "🚨 Max drawdown aşıldı: {:.2}% / {:.2}% — trading durduruluyor", current_dd, limit
                                    ));
                                    ls.stop_loop = true;
                                }
                                if trade.pnl.unwrap_or(0.0) > 0.0 { ls.win_trades += 1; }
                                position_opened = true;
                                // Trade açıldı → spot SELL blok sayacını sıfırla
                                self.config.map_live(|s| s.try_write_risk(|r| {
                                    r.spot_sell_blocks = 0;
                                }));

                                // NOT: learn_from_trade artık kapanışta çağrılır (pending_evo_data).
                                // should_evolve + mini-evrim tetikleyicileri her tick'te kontrol edilir.
                                self.maybe_evolve(ls);

                                let tsl_active = risk_params.trailing_stop_pct.is_some();
                                self.logger.log_info("trade", &format!(
                                    "Emir gönderildi: {} {} qty={:.4} SL={:.1}% TP={:.1}%{}",
                                    trade.symbol, active_strategy_name, trade.amount,
                                    risk_params.stop_loss_pct, risk_params.take_profit_pct,
                                    if tsl_active { format!(" tSL={:.1}%", risk_params.trailing_stop_pct.unwrap()) } else { String::new() }
                                ));
                            }
                            Err(e) => {
                                ls.error_count += 1;
                                let e_str = e.to_string();
                                self.logger.log_error("trade", &format!("Emir hatası: {}", e_str));

                                // HF Blacklist: slippage/execution hatası → sembol sayacına ekle.
                                // "fırsat kaçtı" = tüm limit denemeleri başarısız (piyasa çok hızlı).
                                // GTX/EXPIRED = maker fill olmadı (spread çok geniş).
                                // "timeout" = doldurma süresi aşıldı (düşük likidite).
                                let is_slip = e_str.contains("fırsat kaçtı")
                                    || e_str.contains("GTX")
                                    || e_str.contains("timeout")
                                    || e_str.contains("EXPIRED")
                                    || e_str.contains("REJECTED")
                                    || e_str.contains("taker olurdu");
                                if is_slip {
                                    let newly_banned = ls.record_hf_error(symbol);
                                    if newly_banned {
                                        self.logger.log_error("hf-blacklist", &format!(
                                            "🚫 {} 4 saatlik işlem yasağı (slippage hatası) — 2+ HF hatası son 1 saatte",
                                            symbol
                                        ));
                                    } else {
                                        self.logger.log_info("hf-blacklist", &format!(
                                            "⚠ {} slippage hatası kaydedildi (2. kayıt → 4h ban): {}",
                                            symbol, e_str
                                        ));
                                    }
                                }

                                if self.config.autonomous_enabled {
                                    let action = ls.recovery_supervisor.next_action(ls.error_count);
                                    match action {
                                        AutonomousRecoveryAction::Retry => {
                                            let _ = ls.autonomous_controller.transition_failure("trade-exec-retry");
                                        }
                                        AutonomousRecoveryAction::EnterSafeMode => {
                                            let _ = ls.autonomous_controller.transition_failure("trade-exec-safe-mode");
                                            self.logger.log_error("autonomous", "Recovery supervisor SAFE MODE tetikledi");
                                        }
                                        AutonomousRecoveryAction::Halt => {
                                            let _ = ls.autonomous_controller.transition_failure("trade-exec-halt");
                                            self.logger.log_error("autonomous", "Recovery supervisor HALT tetikledi");
                                            ls.stop_loop = true;
                                            return;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if position_opened {
                        let effective_entry = self.config.execution_cost_config.as_ref()
                            .map(|ec| adjusted_price(entry_price, qty, is_long_entry, ec))
                            .unwrap_or(entry_price);

                        // ── ATR tabanlı dinamik SL/TP ────────────────────────────────────────
                        // sl_atr_multiplier > 0 → SL = ATR × çarpan (sabit % yerine).
                        // Bu phantom SL'i önler: çok sıkışık SL → volatilite ile anında tetiklenir.
                        // ATR mevcut değilse ya da devre dışıysa config değerleri korunur.
                        let atr_risk_owned: crate::types::RiskParams;
                        let risk_params: &crate::types::RiskParams = {
                            let ap = &ls.adaptive_params;
                            if ap.sl_atr_multiplier > 0.0 {
                                if let Some(atr_pct) = average_range_pct(&candles, 14) {
                                    let atr_sl = atr_pct * ap.sl_atr_multiplier;
                                    let atr_tp = atr_pct * ap.tp_atr_multiplier;
                                    // ATR SL en az config SL kadar, en fazla max_sl_pct kadar
                                    let sl_pct = atr_sl.max(risk_params.stop_loss_pct).min(max_sl_pct);
                                    // TP: ATR öncelikli; min R/R × SL garantisi uygulanır.
                                    // Üst sınır: max(4.0%, sl_pct × min_rr) — yüksek ATR varlıklarda
                                    // sabit %4 kapı R/R'yi bozmasın (örn. ATR=%7 → SL=%14 → TP max=%4 → R/R=0.28x)
                                    let tp_raw = atr_tp.max(sl_pct * ls.min_rr);
                                    let tp_cap = (4.0_f64).max(sl_pct * ls.min_rr);
                                    let tp_pct = tp_raw.min(tp_cap);
                                    self.logger.log_info("atr-sl-tp", &format!(
                                        "ATR-bazlı SL/TP: ATR={:.3}% SL={:.2}%→{:.2}% TP={:.2}% ({}×/{:.1}×)",
                                        atr_pct, risk_params.stop_loss_pct, sl_pct,
                                        tp_pct,
                                        ap.sl_atr_multiplier, ap.tp_atr_multiplier
                                    ));
                                    let mut r = risk_params.clone();
                                    r.stop_loss_pct   = sl_pct;
                                    r.take_profit_pct = tp_pct;
                                    atr_risk_owned = r;
                                    &atr_risk_owned
                                } else { risk_params }
                            } else { risk_params }
                        };

                        // S/R tabanlı optimum SL/TP — R/R kısıtını karşılayan bölge seçimi
                        let sr_adjusted: Option<crate::types::RiskParams> =
                            if self.config.sr_config.adjust_sl_tp {
                                sr_ctx.as_ref().map(|ctx| {
                                    let buf    = self.config.sr_config.sl_tp_buffer_pct;
                                    let min_rr = ls.min_rr.max(1.5);
                                    let (sl, tp, rr, note) = ctx.indicator_adjusted_sl_tp(
                                        &candles,
                                        effective_entry,
                                        is_long_entry,
                                        risk_params.stop_loss_pct,
                                        risk_params.take_profit_pct,
                                        buf,
                                        min_rr,
                                    );
                                    let sl_pct_raw = if is_long_entry {
                                        ((effective_entry - sl) / effective_entry * 100.0).max(0.1)
                                    } else {
                                        ((sl - effective_entry) / effective_entry * 100.0).max(0.1)
                                    };
                                    let tp_pct_raw = if is_long_entry {
                                        ((tp - effective_entry) / effective_entry * 100.0).max(0.1)
                                    } else {
                                        ((effective_entry - tp) / effective_entry * 100.0).max(0.1)
                                    };
                                    // max_sl_adjust_pct > 0 → S/R bu %'nin üstüne çıkaramaz
                                    // ATR tabanı: volatil sembollerde (ETHUSDT vb.) cap ATR*0.80'in
                                    // altına inemez — aksi hâlde phantom SL erken tetikler.
                                    let sl_pct = {
                                        let cap = self.config.sr_config.max_sl_adjust_pct;
                                        if cap > 0.0 && sl_pct_raw > cap {
                                            let atr_floor = average_range_pct(&candles, 14)
                                                .map(|a| a * 0.80)
                                                .unwrap_or(0.0);
                                            let effective_cap = cap.max(atr_floor);
                                            if sl_pct_raw > effective_cap {
                                                self.logger.log_info("sr-sl-cap", &format!(
                                                    "S/R SL {:.2}% → {:.2}% kısıtlandı (cap={:.2}% atr_floor={:.2}%)",
                                                    sl_pct_raw, effective_cap, cap, atr_floor
                                                ));
                                                effective_cap
                                            } else { sl_pct_raw }
                                        } else { sl_pct_raw }
                                    };
                                    let tp_pct = {
                                        let cap = self.config.sr_config.max_tp_adjust_pct;
                                        if cap > 0.0 && tp_pct_raw > cap {
                                            self.logger.log_info("sr-tp-cap", &format!(
                                                "S/R TP {:.2}% → {:.2}% kısıtlandı (max_tp_adjust_pct)",
                                                tp_pct_raw, cap
                                            ));
                                            cap
                                        } else { tp_pct_raw }
                                    };
                                    self.logger.log_info("sr-sl-tp", &format!(
                                        "Optimum S/R SL/TP: sl={:.4}({:.2}%) tp={:.4}({:.2}%) R/R={:.2} [{}]",
                                        sl, sl_pct, tp, tp_pct, rr, note
                                    ));
                                    let mut r = risk_params.clone();
                                    r.stop_loss_pct   = sl_pct;
                                    r.take_profit_pct = tp_pct;
                                    r
                                })
                            } else { None };
                        // SL güvenlik klampı — tasfiye fiyatından önce çıkmak zorunlu
                        // + ATR min-SL: SL volatilitenin en az %80'i kadar olmalı (phantom SL önlemi)
                        let effective_risk_owned: crate::types::RiskParams;
                        let effective_risk = {
                            let base_risk = sr_adjusted.as_ref().unwrap_or(risk_params);
                            let atr_min_sl = average_range_pct(&candles, 14)
                                .map(|a| a * 0.80)
                                .unwrap_or(0.0);
                            let needs_sl_lift = base_risk.stop_loss_pct < atr_min_sl && atr_min_sl > 0.0;
                            let needs_sl_cap  = base_risk.stop_loss_pct > max_sl_pct;
                            if needs_sl_lift || needs_sl_cap {
                                let mut r = base_risk.clone();
                                if needs_sl_cap  { r.stop_loss_pct = max_sl_pct; }
                                if needs_sl_lift { r.stop_loss_pct = r.stop_loss_pct.max(atr_min_sl); }
                                // TP R/R oranını koru
                                let rr_ratio = base_risk.take_profit_pct / base_risk.stop_loss_pct.max(0.01);
                                r.take_profit_pct = r.stop_loss_pct * rr_ratio;
                                self.logger.log_info("sl-guard", &format!(
                                    "SL {:.3}% → {:.3}% (ATR_min={:.3}% max={:.2}%) TP={:.3}%",
                                    base_risk.stop_loss_pct, r.stop_loss_pct,
                                    atr_min_sl, max_sl_pct, r.take_profit_pct
                                ));
                                effective_risk_owned = r;
                                &effective_risk_owned
                            } else {
                                base_risk
                            }
                        };

                        // ── Fix B: ATR-aware TSL — trailing mesafesi volatiliteye göre genişletilir ─────
                        // Config TSL (örn. %2.5) kısa vadeli dalgalanmalarda anında tetiklenebilir.
                        // ATR'nin %1.5 katından küçükse trailing_stop_pct yukarı çekilir.
                        let effective_risk_tsl_owned: crate::types::RiskParams;
                        let effective_risk = {
                            if let Some(tsl_pct) = effective_risk.trailing_stop_pct {
                                if let Some(atr_pct) = average_range_pct(&candles, 14) {
                                    let atr_min_tsl = atr_pct * 1.5;
                                    if tsl_pct < atr_min_tsl {
                                        self.logger.log_info("tsl-atr", &format!(
                                            "TSL {:.2}% → {:.2}% (ATR×1.5)", tsl_pct, atr_min_tsl
                                        ));
                                        let mut r = effective_risk.clone();
                                        r.trailing_stop_pct = Some(atr_min_tsl.min(5.0)); // max 5%
                                        effective_risk_tsl_owned = r;
                                        &effective_risk_tsl_owned
                                    } else { effective_risk }
                                } else { effective_risk }
                            } else { effective_risk }
                        };

                        // ── Anlık fiyat SL kontrolü ──────────────────────────────────────────
                        // Fiyat zaten SL seviyesinin ötesindeyse pozisyon açma:
                        // bu 1-2 saniye içinde kapanan "phantom SL" trade'lerin başlıca nedenidir.
                        let sl_price_preview = if is_long_entry {
                            effective_entry * (1.0 - effective_risk.stop_loss_pct / 100.0)
                        } else {
                            effective_entry * (1.0 + effective_risk.stop_loss_pct / 100.0)
                        };
                        let already_stopped = if is_long_entry {
                            current_price <= sl_price_preview
                        } else {
                            current_price >= sl_price_preview
                        };
                        // ── Post-ATR R/R güvenlik kontrolü ──────────────────────────────────
                        // ATR ayarlaması SL'yi büyütmüş olabilir; TP cap iyileştirilmiş olsa da
                        // son kez kontrol et. sl_pct/tp_pct burada hesaplanmış effective_risk'ten gelir.
                        let post_atr_sl = effective_risk.stop_loss_pct;
                        let post_atr_tp = effective_risk.take_profit_pct;
                        let post_atr_rr = if post_atr_sl > 0.0 { post_atr_tp / post_atr_sl } else { 0.0 };
                        if post_atr_rr < ls.min_rr {
                            self.count_signal(SignalMetric::BlockedRr(
                                format!("Post-ATR R/R düşük ({:.2}x < {:.2}x) SL={:.2}% TP={:.2}%",
                                    post_atr_rr, ls.min_rr, post_atr_sl, post_atr_tp).into()
                            ));
                            self.logger.log_error("post-atr-rr",
                                &format!("Post-ATR R/R={:.2}x red (SL={:.2}% TP={:.2}%) — {} işlem atlandı",
                                    post_atr_rr, post_atr_sl, post_atr_tp, symbol));
                        } else if !already_stopped {

                        let mut pos = OpenPosition::new(
                            symbol.to_string(),
                            self.config.market,
                            effective_entry,
                            qty,
                            is_long_entry,
                            effective_risk,
                            effective_lev,
                            self.config.breakeven_at_rr,
                            self.config.atr_trail_mult,
                            self.config.partial_tp_ratio,
                            Some(ls.adaptive_params.trailing_sl_activation_pct),
                        );
                        // ── ML özniteliklerini kaydet: kapanışta online eğitim için ─
                        pos.entry_features = Some(FeatureExtractor::extract(&candles).normalize().to_array());
                        let tsl_info = pos.trailing_pct.map(|p| format!(" tSL={:.1}%", p)).unwrap_or_default();
                        self.logger.log_info("trailing", &format!(
                            "Pozisyon açıldı: {} {} x{:.1} giriş={:.4} liq={:.4} SL={:.4} TP={:.4}{} [id={}]",
                            symbol, if pos.is_long { "LONG" } else { "SHORT" }, effective_lev,
                            entry_price, pos.liquidation_price, pos.static_sl, pos.static_tp, tsl_info, pos.id
                        ));
                        // ── Evrimsel learning: giriş rejimini + strateji adını sakla ─
                        // Kapanışta gerçek PnL ile learn_from_trade çağrısı için kullanılır.
                        if self.config.autonomous_enabled {
                            let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
                            let slice = &closes[closes.len().saturating_sub(20)..];
                            let first = slice.first().copied().unwrap_or(0.0);
                            let last  = slice.last().copied().unwrap_or(0.0);
                            let trend = if first > 0.0 { ((last - first) / first) * 100.0 } else { 0.0 };
                            let entry_regime = if trend > 3.0 { MarketRegime::StrongUptrend }
                                else if trend >  1.0 { MarketRegime::WeakUptrend }
                                else if trend < -3.0 { MarketRegime::StrongDowntrend }
                                else if trend < -1.0 { MarketRegime::WeakDowntrend }
                                else                 { MarketRegime::Ranging };
                            ls.pending_evo_data.insert(pos.id, (entry_regime, active_strategy_name.clone()));
                        }
                        self.upsert_live_position(&pos, self.config.market, entry_price);
                        // Pozisyon açılış bildirimi
                        crate::send_alert!(self.telegram,
                            "📈 <b>{} {}</b> açıldı\nGiriş: {:.4} | SL: {:.4} | TP: {:.4}\nQty: {:.6} | Kaldıraç: {:.0}x",
                            symbol, if is_long_entry { "LONG" } else { "SHORT" },
                            entry_price, pos.static_sl, pos.static_tp, pos.qty, pos.leverage
                        );
                        ls.open_positions.insert(pos.id, pos);
                        // Min trade interval: bu sembol için son giriş zamanını güncelle
                        ls.last_trade_time.insert(symbol.to_string(), std::time::Instant::now());
                        // Sinyal kuruluğu sayacını sıfırla — gerçek işlem açıldı
                        ls.last_nonhold_signal_at = Some(std::time::Instant::now());
                        } else {
                            // SL precheck başarısız — phantom SL trade engellendi
                            self.logger.log_error("sl-precheck", &format!(
                                "⛔ {} {} açılmadı — anlık fiyat ({:.4}) SL bölgesinde ({:.4}, SL={:.2}%)",
                                symbol, if is_long_entry { "LONG" } else { "SHORT" },
                                current_price, sl_price_preview, effective_risk.stop_loss_pct
                            ));
                        }
                    }
                } else {
                    self.logger.log_error("risk", "Pozisyon boyutu 0 - emir atlandı");
                }
            }
        }

        if ls.error_count > 10 {
            self.logger.log_error("fail-safe", "Kritik hata limiti aşıldı, sistem otomatik durduruluyor.");
            ls.stop_loop = true;
        }
    }

    /// Scalp & Swing fırsat motoru — Regular loop'tan bağımsız, çakışmasız
    #[cfg(not(target_arch = "wasm32"))]
    async fn process_scalp_swing(
        &self,
        symbol: &str,
        market: Market,
        current_price: f64,
        candles_main: &[crate::types::Candle], // ana interval (1h gibi)
        ls: &mut LoopState,
    ) {
        use crate::robot::scalp_swing::{
            ScalpEngine, SwingEngine, SlotGuard, ModeSelector,
            slot_guard::OpenSlot, TradeType,
        };

        let cfg = match &self.config.scalp_swing {
            Some(c) => c.clone(),
            None => return, // devre dışı
        };

        // ── Mod seçimi ────────────────────────────────────────────────────────
        let mode = ModeSelector::select(candles_main, cfg.scalp_active_hours);

        // Her tick tanılama: mod + veri boyutu
        self.logger.log_info("scalp-swing", &format!(
            "[{}] mod={} mumlar={}",
            symbol, mode.label(), candles_main.len()
        ));

        if mode == crate::robot::scalp_swing::TradeMode::Neither { return; }

        // ── Mevcut açık pozisyonları SlotGuard formatına çevir ────────────────
        let open_slots: Vec<OpenSlot> = ls.open_positions.values()
            .map(|p| OpenSlot {
                symbol:     p.symbol.clone(),
                trade_type: p.trade_type,
                is_long:    p.is_long,
            })
            .collect();

        // ── 1. SCALP sinyali ─────────────────────────────────────────────────
        let run_scalp = cfg.scalp_enabled
            && matches!(mode,
                crate::robot::scalp_swing::TradeMode::ScalpOnly |
                crate::robot::scalp_swing::TradeMode::Both)
            && !ls.strategy_scorer.is_disabled(TradeType::Scalp);

        if !run_scalp && ls.strategy_scorer.is_disabled(TradeType::Scalp) {
            self.logger.log_info("ai-control", &format!(
                "🤖 Scalp bu rejimde devre dışı ({}), giriş engellendi [{}]",
                ls.strategy_scorer.last_reason, symbol
            ));
        }

        if run_scalp {
            // 3m/5m candle çek
            let scalp_candles = match self.fetcher.fetch_latest(
                Exchange::Binance, market, symbol, &cfg.scalp_interval, 60
            ).await {
                Ok(c) if c.len() >= 30 => c,
                _ => {
                    self.logger.log_info("scalp", &format!(
                        "{} {} candle alınamadı — scalp atlandı", symbol, cfg.scalp_interval
                    ));
                    Vec::new()
                }
            };

            if !scalp_candles.is_empty() {
                // Per-symbol Volatile guard — her sembolün kendi ATR'si ile değerlendirilir.
                let local_regime = crate::market_regime::detect_adx_regime(&scalp_candles);
                let local_atr_pct = crate::robot::signal_evaluator::average_range_pct(&scalp_candles, 14)
                    .unwrap_or(0.0);
                if local_regime == crate::market_regime::AdxRegime::Volatile || local_atr_pct > 2.0 {
                    self.logger.log_info("scalp", &format!(
                        "⛔ {} scalp engellendi — yerel ATR={:.2}% rejim={} (volatile/yüksek ATR)",
                        symbol, local_atr_pct, local_regime
                    ));
                } else {
                // min_score=0.0 ile skor hesapla, sonra eşikle karşılaştır
                match ScalpEngine::evaluate(&scalp_candles, 0.0) {
                    Some(raw) if raw.score >= cfg.scalp_min_score => {
                        let mut opp = raw;
                        // SHORT HTF bloğu — regular SHORT'la aynı kural: bullish trende SHORT açma.
                        // process_scalp_swing'de htf_bias yok; candles_main'den yerel bias hesapla.
                        let local_bias = crate::robot::signal_evaluator::trend_bias(
                            candles_main, 20, 50, 0.5
                        );
                        let htf_short_blocked = !opp.is_long
                            && ls.adaptive_params.short_htf_block
                            && local_bias == Some(crate::robot::signal_evaluator::TrendBias::Bullish);
                        if htf_short_blocked {
                            self.logger.log_info("scalp-sl", &format!(
                                "🛡 {} SCP SHORT engellendi — HTF Bullish + short_htf_block=true", symbol
                            ));
                        } else {
                            // ATR-tabanlı SL/TP: statik %0.40 küçük coin gürültüsünde anında
                            // tetikleniyor. Kural: SL = max(ATR×çarpan, scalp_sl_pct×2) →
                            // doğal gürültü payı her zaman en az 2× statik değer.
                            let atr_mult = {
                                let ap = &ls.adaptive_params;
                                if ap.sl_atr_multiplier > 0.0 { ap.sl_atr_multiplier } else { 2.0 }
                            };
                            let sl_floor = cfg.scalp_sl_pct * 2.0;
                            let atr_sl = if local_atr_pct > 0.0 { local_atr_pct * atr_mult } else { 0.0 };
                            let raw_sl = atr_sl.max(sl_floor).min(cfg.scalp_sl_bounds.max);
                            let rr_floor = ls.min_rr.max(1.2);
                            let raw_tp = (raw_sl * rr_floor).max(cfg.scalp_tp_pct).min(cfg.scalp_tp_bounds.max);
                            self.logger.log_info("scalp-sl", &format!(
                                "{} SL/TP: ATR={:.3}% SL={:.2}%(floor={:.2}%) TP={:.2}% (×{:.1})",
                                symbol, local_atr_pct, raw_sl, sl_floor, raw_tp, atr_mult
                            ));
                            opp.sl_pct = raw_sl;
                            opp.tp_pct = raw_tp;
                            let (allowed, deny_reason) = SlotGuard::can_open(
                                &open_slots, symbol, TradeType::Scalp, opp.is_long,
                                cfg.max_scalp_per_symbol, cfg.max_swing_per_symbol,
                            );
                            if allowed {
                                self.open_scalp_swing_position(symbol, market, current_price, &opp, ls);
                            } else {
                                self.logger.log_info("scalp", &format!(
                                    "{} scalp reddedildi: {} (score={:.2})", symbol, deny_reason, opp.score
                                ));
                            }
                        }
                    }
                    Some(raw) => {
                        self.logger.log_info("scalp", &format!(
                            "{} sinyal yok — skor={:.2} < eşik={:.2} | {}",
                            symbol, raw.score, cfg.scalp_min_score, raw.reason
                        ));
                    }
                    None => {
                        self.logger.log_info("scalp", &format!(
                            "{} sinyal üretilemedi (yetersiz koşul/veri)", symbol
                        ));
                    }
                }
                } // per-symbol volatile guard else
            }
        }

        // ── 2. SWING sinyali ─────────────────────────────────────────────────
        let run_swing = cfg.swing_enabled
            && matches!(mode,
                crate::robot::scalp_swing::TradeMode::SwingOnly |
                crate::robot::scalp_swing::TradeMode::Both)
            && !ls.strategy_scorer.is_disabled(TradeType::Swing);

        if !run_swing && ls.strategy_scorer.is_disabled(TradeType::Swing) {
            self.logger.log_info("ai-control", &format!(
                "🤖 Swing bu rejimde devre dışı ({}), giriş engellendi [{}]",
                ls.strategy_scorer.last_reason, symbol
            ));
        }

        if run_swing {
            // 4h/1D candle çek
            let swing_candles = match self.fetcher.fetch_latest(
                Exchange::Binance, market, symbol, &cfg.swing_interval, 120
            ).await {
                Ok(c) if c.len() >= 60 => c,
                Ok(c) => {
                    self.logger.log_info("swing", &format!(
                        "{} {} yetersiz candle: {} (min 60)", symbol, cfg.swing_interval, c.len()
                    ));
                    Vec::new()
                }
                Err(e) => {
                    self.logger.log_info("swing", &format!(
                        "{} {} candle alınamadı: {}", symbol, cfg.swing_interval, e
                    ));
                    Vec::new()
                }
            };

            if !swing_candles.is_empty() {
                // Per-symbol Volatile guard (Swing daha toleranslı, kripto normu: 7% eşiği)
                let local_regime = crate::market_regime::detect_adx_regime(&swing_candles);
                let local_atr_pct = crate::robot::signal_evaluator::average_range_pct(&swing_candles, 14)
                    .unwrap_or(0.0);
                if local_regime == crate::market_regime::AdxRegime::Volatile || local_atr_pct > 7.0 {
                    self.logger.log_info("swing", &format!(
                        "⛔ {} swing engellendi — yerel ATR={:.2}% rejim={} (volatile/yüksek ATR)",
                        symbol, local_atr_pct, local_regime
                    ));
                    return;
                }
                let swing_min_score = cfg.swing_min_score;
                match SwingEngine::evaluate(&swing_candles, cfg.swing_min_adx, 0.0) {
                    Some(raw) if raw.score >= swing_min_score => {
                        let mut opp = raw;
                        opp.sl_pct = cfg.swing_sl_pct;
                        opp.tp_pct = cfg.swing_tp_pct;
                        let updated_slots: Vec<OpenSlot> = ls.open_positions.values()
                            .map(|p| OpenSlot { symbol: p.symbol.clone(), trade_type: p.trade_type, is_long: p.is_long })
                            .collect();
                        let (allowed, deny_reason) = SlotGuard::can_open(
                            &updated_slots, symbol, TradeType::Swing, opp.is_long,
                            cfg.max_scalp_per_symbol, cfg.max_swing_per_symbol,
                        );
                        if allowed {
                            self.open_scalp_swing_position(symbol, market, current_price, &opp, ls);
                        } else {
                            self.logger.log_info("swing", &format!(
                                "{} swing reddedildi: {} (score={:.2})", symbol, deny_reason, opp.score
                            ));
                        }
                    }
                    Some(raw) => {
                        self.logger.log_info("swing", &format!(
                            "{} sinyal yok — skor={:.2} < eşik={:.2} | {}",
                            symbol, raw.score, swing_min_score, raw.reason
                        ));
                    }
                    None => {
                        self.logger.log_info("swing", &format!(
                            "{} sinyal üretilemedi (trend/ADX yetersiz)", symbol
                        ));
                    }
                }
            }
        }
    }

    /// ScalpSwing pozisyon açma — kaldıraç, spread/slippage/komisyon, risk gate dahil.
    #[cfg(not(target_arch = "wasm32"))]
    fn open_scalp_swing_position(
        &self,
        symbol: &str,
        market: Market,
        current_price: f64,
        opp: &crate::robot::scalp_swing::TradeOpportunity,
        ls: &mut LoopState,
    ) {
        use crate::robot::scalp_swing::TradeType;
        use crate::types::Signal;

        let cfg = match self.config.scalp_swing.as_ref() {
            Some(c) => c,
            None    => return,
        };

        // ── 0a. Duplicate emir koruması — aynı sembolde aynı yönde 60 sn'den
        // kısa aralıklarla yeni pozisyon açılmasını engeller (RAVEUSDT örneği:
        // 26 sn arayla aynı PnL'li iki açılış görüldü).
        {
            const DUP_WINDOW_SECS: u64 = 60;
            let dup_key = format!("{}_{}", symbol, if opp.is_long { "LONG" } else { "SHORT" });
            if let Some(t) = ls.last_entry_at.get(&dup_key) {
                if t.elapsed().as_secs() < DUP_WINDOW_SECS {
                    self.logger.log_info("scalp-swing", &format!(
                        "⛔ {} {} duplicate koruma — son giriş {} sn önce ({} sn pencere)",
                        symbol, if opp.is_long { "LONG" } else { "SHORT" },
                        t.elapsed().as_secs(), DUP_WINDOW_SECS
                    ));
                    return;
                }
            }
            ls.last_entry_at.insert(dup_key, std::time::Instant::now());
        }

        // ── 0. SCP/SWG tip-bazlı SL cooldown kontrolü ───────────────────────
        // Ardışık SL'ye göre artan süre: SWG 2/4/8 saat, SCP 15/30/60 dk
        {
            use crate::robot::scalp_swing::TradeType;
            if opp.trade_type != TradeType::Regular {
                let key = format!("{}_{}", symbol, opp.trade_type.label());
                if let Some(&(cd_start, cd_dur)) = ls.scalp_swing_sl_cooldown.get(&key) {
                    let elapsed = cd_start.elapsed().as_secs();
                    if elapsed < cd_dur {
                        let remaining = cd_dur - elapsed;
                        let consecutive = ls.scalp_swing_consecutive_sl.get(&key).copied().unwrap_or(1);
                        self.logger.log_info("ss-cooldown", &format!(
                            "⏸ [{}] {} SL cooldown aktif — {} dk {} sn kaldı (ardışık={}, toplam={} dk)",
                            opp.trade_type.label(), symbol,
                            remaining / 60, remaining % 60,
                            consecutive, cd_dur / 60
                        ));
                        return;
                    } else {
                        ls.scalp_swing_sl_cooldown.remove(&key);
                    }
                }
            }
        }

        // ── 0b. Spot piyasasında SHORT açılamaz ──────────────────────────────
        if !opp.is_long && market == Market::Spot {
            self.logger.log_error("scalp-swing", &format!(
                "⛔ {} Spot piyasasında SHORT (SCP/SWG) açılamaz — atlandı", symbol
            ));
            return;
        }

        // ── 1. Günlük kayıp limiti (scalp_swing bağımsız havuz) ──────────────
        // SANITY: equity ≈ 0 cold-start senaryosunda yüzde anlamsız — guard'ı atla.
        const SS_MIN_EQUITY: f64 = 10.0;
        if cfg.max_daily_loss_pct > 0.0
            && ls.scalp_swing_day_start_equity >= SS_MIN_EQUITY
            && ls.current_equity >= SS_MIN_EQUITY
        {
            let day_loss_pct = (ls.scalp_swing_day_start_equity - ls.current_equity)
                / ls.scalp_swing_day_start_equity * 100.0;
            if day_loss_pct > cfg.max_daily_loss_pct {
                self.logger.log_error("scalp-swing", &format!(
                    "⛔ {} günlük kayıp {:.2}% > {:.2}% — işlem engellendi",
                    opp.trade_type.label(), day_loss_pct, cfg.max_daily_loss_pct
                ));
                return;
            }
        }

        // ── 2. Circuit breaker ────────────────────────────────────────────────
        if ls.api_circuit_breaker.state() == crate::robot::error_recovery::circuit_breaker::CircuitBreakerState::Open {
            self.logger.log_error("scalp-swing", "Circuit breaker açık — emir atlandı");
            return;
        }

        // ── 3. Kaldıraç (Spot = 1.0x, Futures = settings min/max'tan türetilir) ──
        // Settings panelindeki base/max leverage scalp/swing'e de yansır:
        //   scalp_leverage = base_leverage   (alt sınır → güvenli)
        //   swing_leverage = (base+max)/2    (orta nokta → orta risk)
        // Settings'ten okunamazsa eski cfg değerlerine düşer (geri uyumlu).
        let (ui_base, ui_max) = self.config.map_live(|s| {
            s.try_read_risk(|r| (r.base_leverage, r.max_leverage))
        }).flatten().unwrap_or((cfg.scalp_leverage, cfg.swing_leverage));
        let derived_scalp_lev = ui_base.max(1.0);
        let derived_swing_lev = ((ui_base + ui_max) * 0.5).max(1.0);
        let leverage = match market {
            Market::Spot => 1.0,
            _ => match opp.trade_type {
                TradeType::Scalp => derived_scalp_lev,
                TradeType::Swing => derived_swing_lev,
                TradeType::Regular => 1.0,
            },
        };
        // Likidasyon güvenliği: max SL = %80 / leverage
        let max_sl_pct = 80.0 / leverage;
        if opp.sl_pct > max_sl_pct {
            self.logger.log_error("scalp-swing", &format!(
                "⛔ {} SL={:.2}% > max={:.2}% ({:.0}x kaldıraç) — likidasyon riski, atlandı",
                opp.trade_type.label(), opp.sl_pct, max_sl_pct, leverage
            ));
            return;
        }

        // ── 4. Pozisyon boyutu: bütçe + kaldıraç ─────────────────────────────
        // Bütçe: config'de ayrılmış yüzde varsa kullan, yoksa trade_amount/capital*2%
        let base_notional = match opp.trade_type {
            TradeType::Scalp => cfg.scalp_budget_pct
                .map(|p| ls.current_equity * p)
                .unwrap_or_else(|| self.config.trade_amount.unwrap_or(self.config.capital * 0.02) * 0.6),
            TradeType::Swing => cfg.swing_budget_pct
                .map(|p| ls.current_equity * p)
                .unwrap_or_else(|| self.config.trade_amount.unwrap_or(self.config.capital * 0.02)),
            TradeType::Regular => self.config.trade_amount.unwrap_or(self.config.capital * 0.02),
        };
        // Kaldıraçlı notional = marjin × leverage
        let notional = base_notional * leverage;
        // max_notional_usd sınırı
        let notional = if let Some(max_not) = cfg.max_notional_usd {
            notional.min(max_not)
        } else { notional };
        let qty = if current_price > 0.0 { notional / current_price } else { return; };
        if qty <= 0.0 { return; }

        // ── 5. Giriş fiyatı: spread + slippage düzeltmesi ────────────────────
        // Öncelik: ScalpSwingConfig > loop ExecutionCostConfig > ham fiyat
        let effective_entry = {
            let spread   = cfg.spread_pct
                .or_else(|| self.config.execution_cost_config.as_ref().map(|ec| ec.spread_pct))
                .unwrap_or(0.0);
            let slippage = cfg.slippage_pct
                .or_else(|| self.config.execution_cost_config.as_ref().map(|ec| ec.slippage_pct))
                .unwrap_or(0.0);
            let adj_pct = spread + slippage;
            if opp.is_long {
                current_price * (1.0 + adj_pct / 100.0)
            } else {
                current_price * (1.0 - adj_pct / 100.0)
            }
        };

        // ── 6. SL / TP fiyatları (effective_entry bazlı) ─────────────────────
        let (sl_price, tp_price) = if opp.is_long {
            (
                effective_entry * (1.0 - opp.sl_pct / 100.0),
                effective_entry * (1.0 + opp.tp_pct / 100.0),
            )
        } else {
            (
                effective_entry * (1.0 + opp.sl_pct / 100.0),
                effective_entry * (1.0 - opp.tp_pct / 100.0),
            )
        };

        // ── 7. Komisyon maliyeti tahmini ─────────────────────────────────────
        let comm_pct = cfg.commission_pct
            .unwrap_or(self.config.commission_pct);
        let entry_comm = notional * comm_pct;  // giriş tarafı
        // Net minimum TP: komisyon giriş+çıkış > TP getirisi olmamalı
        let min_net_tp_pct = (entry_comm * 2.0 / notional) * 100.0 * 1.5; // 1.5x güvenlik marjı
        if opp.tp_pct < min_net_tp_pct {
            self.logger.log_error("scalp-swing", &format!(
                "⛔ {} TP={:.2}% < komisyon eşiği {:.2}% — kârsız işlem atlandı",
                opp.trade_type.label(), opp.tp_pct, min_net_tp_pct
            ));
            return;
        }

        // ── 8. Emir gönder ───────────────────────────────────────────────────
        let sig = if opp.is_long { Signal::Buy } else { Signal::Sell };
        let trades = self.executor.execute_basket(sig, qty);
        let fill_ok = trades.iter().any(|t| t.as_ref().map(|tr| tr.amount > 0.0).unwrap_or(false));
        if !fill_ok {
            self.logger.log_error("scalp-swing", &format!(
                "{} {} emir doldurma başarısız", symbol,
                if opp.is_long { "LONG" } else { "SHORT" }
            ));
            let _ = ls.api_circuit_breaker.record_failure("scalp_swing_open");
            return;
        }
        let _ = ls.api_circuit_breaker.record_success();

        // ── 9. OpenPosition oluştur ───────────────────────────────────────────
        let risk = crate::types::RiskParams {
            stop_loss_pct:         opp.sl_pct,
            take_profit_pct:       opp.tp_pct,
            max_position_size_pct: None,
            max_portfolio_risk_pct: None,
            use_kelly_criterion:   false,
            trailing_stop_pct:     None,
        };
        let mut pos = crate::robot::position_manager::OpenPosition::new(
            symbol.to_string(),
            market,
            effective_entry,
            qty,
            opp.is_long,
            &risk,
            leverage,
            None,  // breakeven
            None,  // atr trail
            None,  // partial tp
            None,  // trailing activation
        );
        pos.trade_type = opp.trade_type;

        // ── 10. Log & bildirim ───────────────────────────────────────────────
        let type_label = opp.trade_type.label();
        self.logger.log_info("scalp-swing", &format!(
            "✅ [{type_label}] {} {} açıldı | giriş={:.4}(adj) SL={:.4} TP={:.4} \
             qty={:.6} lev={:.1}x notional={:.2}$ comm={:.4}$ skor={:.2} | {}",
            symbol, if opp.is_long { "LONG" } else { "SHORT" },
            effective_entry, sl_price, tp_price,
            qty, leverage, notional, entry_comm, opp.score, opp.reason
        ));
        crate::send_alert!(self.telegram,
            "⚡ <b>[{type_label}] {} {}</b> açıldı\nGiriş: {:.4} | SL: {:.4} | TP: {:.4}\n\
             Kaldıraç: {:.0}x | Notional: {:.0}$ | Skor: {:.0}%",
            symbol, if opp.is_long { "LONG" } else { "SHORT" },
            effective_entry, sl_price, tp_price,
            leverage, notional, opp.score * 100.0
        );

        // TUI live_positions haritasına yaz — Tab 4 ve Tab 9'da görünsün
        self.upsert_live_position(&pos, market, effective_entry);
        ls.open_positions.insert(pos.id, pos);
        ls.last_trade_time.insert(symbol.to_string(), std::time::Instant::now());
        ls.last_nonhold_signal_at = Some(std::time::Instant::now());
    }

    pub async fn start(&mut self) {
        // 1m candle her 60 saniyede bir çekilir; sinyal değerlendirme
        // interval_secs beklenmeden yapılandırılan interval kapandığında tetiklenir.
        const POLL_INTERVAL_SECS: u64 = 60;
        let candle_limit = self.config.candle_limit;
        let capital = self.config.capital;
        let _risk = SimpleRiskAnalyzer;

        // Telegram bildirici — env var yoksa None (özellik devre dışı, hata yok)
        if self.telegram.is_none() {
            self.telegram = crate::robot::telegram_notifier::TelegramNotifier::from_env();
        }
        crate::send_alert!(self.telegram,
            "🤖 <b>Memos Trading başladı</b>\nSembol: {} | Market: {} | Mod: {}",
            self.config.symbol,
            self.config.market.as_str(),
            if self.paper_mode { "Paper" } else { "LIVE" }
        );

        // ── Otonom kontrolcü başlat ───────────────────────────────────────────
        let mut autonomous_controller = AutonomousController::new(AutonomousControllerConfig::default());
        if self.config.initial_cycle_id > 0 {
            autonomous_controller.cycle_id = self.config.initial_cycle_id;
        }
        if self.config.autonomous_enabled {
            autonomous_controller.enable_evolution("MA".to_string());
            if let Some(brain) = self.config.initial_brain.clone() {
                autonomous_controller.adaptive_brain = Some(brain);
                self.logger.log_info("evolution", "🧠 AdaptiveBrain snapshot'tan restore edildi");
            }
            if let Some(pop) = self.config.initial_population.clone() {
                let best = pop.get_best_strategy().cloned();
                autonomous_controller.population_manager = Some(pop);
                autonomous_controller.current_strategy_genome = best;
                self.logger.log_info("evolution", "🧬 PopulationManager snapshot'tan restore edildi");
            } else {
                self.logger.log_info("evolution", "🧬 Evrimsel AI aktifleştirildi - Sistem kendi kendine öğrenecek ve gelişecek");
            }
        }

        // ── Açık pozisyonları snapshot'tan yükle ─────────────────────────────
        let mut open_positions: std::collections::HashMap<crate::types::PositionId, OpenPosition> =
            self.config.initial_open_positions
                .iter()
                .map(|(_, lpd)| {
                    let id = lpd.pos_id;
                    let pos = OpenPosition {
                        id,
                        symbol:            lpd.symbol.clone(),
                        market:            lpd.market,
                        entry_price:       lpd.entry_price,
                        qty:               lpd.qty,
                        is_long:           lpd.is_long,
                        static_sl:         lpd.static_sl,
                        static_tp:         lpd.static_tp,
                        best_price:        lpd.best_price,
                        // trailing_sl sanitizasyonu: TP'ye eşit veya üzerindeyse (önceki bug kalıntısı)
                        // ya da LONG için fiyatın çok üzerindeyse sıfırla — ilk tickte phantom çıkış engeli.
                        trailing_sl: {
                            let tsl = lpd.trailing_sl;
                            let sane = tsl.map_or(true, |v| {
                                if lpd.is_long { v < lpd.static_tp && v < lpd.best_price }
                                else           { v > lpd.static_tp && v > lpd.best_price }
                            });
                            if sane { tsl } else { None }
                        },
                        trailing_pct:               lpd.trailing_pct,
                        trailing_activation_pct:    None, // snapshot'ta yok — varsayılan (adaptive_params'tan runtime'da set edilir)
                        leverage:          lpd.leverage,
                        liquidation_price: lpd.liquidation_price,
                        // Snapshot'tan geri yüklenen pozisyonlar: risk_distance hesaplanır,
                        // B1/B2/B3 config değerleri yeniden atanır.
                        risk_distance:         (lpd.entry_price - lpd.static_sl).abs().max(f64::EPSILON),
                        breakeven_at_rr:       self.config.breakeven_at_rr,
                        breakeven_triggered:   false, // snapshot'ta bilgi yok — yeniden hesaplanacak
                        atr_trail_mult:        self.config.atr_trail_mult,
                        partial_tp_ratio:      self.config.partial_tp_ratio,
                        partial_tp_triggered:  false,
                        opened_at:             lpd.opened_at.clone(),
                        entry_features:        None, // snapshot restore — öznitelikler yeniden hesaplanamaz
                        tp1_price:             lpd.tp1_price,
                        tp1_close_ratio:       0.40,
                        tp1_triggered:         lpd.tp1_triggered,
                        trade_type:            lpd.trade_type,
                        manual_exit_required:  false, // session-sync tarafından aşağıda set edilebilir
                    };
                    (id, pos)
                })
                .collect();

        // FIX-3: initial_open_positions boşsa DB snapshot'ından geri yükle
        if open_positions.is_empty() {
            if let Some(db_path) = &self.config.db_path {
                if let Ok(conn) = rusqlite::Connection::open(db_path) {
                    if let Some(json) = crate::database_writer::load_open_positions_snapshot(&conn) {
                        match serde_json::from_str::<Vec<OpenPosition>>(&json) {
                            Ok(persisted) if !persisted.is_empty() => {
                                let count = persisted.len();
                                for mut pos in persisted {
                                    // trailing_sl sanitizasyonu: önceki oturumdan kalan bozuk değer
                                    // (örn. static_tp ile eşit) ilk tickte phantom çıkışa yol açar.
                                    if let Some(tsl) = pos.trailing_sl {
                                        let sane = if pos.is_long {
                                            tsl < pos.static_tp && tsl < pos.best_price
                                        } else {
                                            tsl > pos.static_tp && tsl > pos.best_price
                                        };
                                        if !sane { pos.trailing_sl = None; }
                                    }
                                    open_positions.insert(pos.id, pos);
                                }
                                self.logger.log_info("startup", &format!(
                                    "DB snapshot'tan {} açık pozisyon geri yüklendi", count
                                ));
                            }
                            Ok(_) => {} // boş snapshot
                            Err(e) => self.logger.log_error("startup", &format!(
                                "Pozisyon snapshot parse hatası: {}", e
                            )),
                        }
                    }
                }
            }
        }

        // ── Exchange sync + SessionID / Stale Pozisyon Tespiti ───────────────
        {
            let exchange_syms = self.executor.executor.fetch_open_symbols();
            // Yeni session başlangıcında DB snapshot pozisyonlarını exchange ile karşılaştır.
            // Paper modda fetch_open_symbols() boş vec döner — bu durumda flag koymayız.
            if !exchange_syms.is_empty() {
                for pos in open_positions.values_mut() {
                    if !exchange_syms.contains(&pos.symbol) {
                        pos.manual_exit_required = true;
                        self.logger.log_error("session-sync", &format!(
                            "⚠ MANUAL EXIT REQUIRED: {} — DB snapshot'ta açık ama exchange'de YOK. Otomatik SL/TP devre dışı.",
                            pos.symbol
                        ));
                    } else if !pos.opened_at.is_empty() {
                        // Stale kontrol: >48 saat önce açılmışsa ve güncel fiyat bilinmiyorsa işaretle
                        if let Ok(opened) = chrono::NaiveDateTime::parse_from_str(&pos.opened_at, "%Y-%m-%d %H:%M:%S") {
                            let age_hours = (chrono::Utc::now().naive_utc() - opened).num_hours();
                            if age_hours > 48 {
                                pos.manual_exit_required = true;
                                self.logger.log_error("session-sync", &format!(
                                    "⚠ MANUAL EXIT REQUIRED: {} — pozisyon {}sa önce açıldı (>48sa stale). Manuel kontrol gerekli.",
                                    pos.symbol, age_hours
                                ));
                            }
                        }
                    }
                }
            }
            let local_syms: std::collections::HashSet<&str> =
                open_positions.values().map(|p| p.symbol.as_str()).collect();
            for sym in &exchange_syms {
                if !local_syms.contains(sym.as_str()) {
                    self.logger.log_error("sync", &format!(
                        "⚠ Pozisyon tutarsızlığı: {} exchange'de açık ama yerel snapshot'ta YOK — orphan order",
                        sym
                    ));
                }
            }
            let manual_count = open_positions.values().filter(|p| p.manual_exit_required).count();
            if !exchange_syms.is_empty() {
                self.logger.log_info("sync", &format!(
                    "✓ Exchange sync tamamlandı: {} açık pozisyon doğrulandı{}",
                    exchange_syms.len(),
                    if manual_count > 0 { format!(", {} MANUEL ÇIKIŞ GEREKTİRİYOR", manual_count) } else { String::new() }
                ));
            }
        }

        // ── LoopState oluştur ─────────────────────────────────────────────────
        let risk_gate          = RiskGate::new(self.config.initial_risk_policy.unwrap_or_default());
        let recovery_supervisor = RecoverySupervisor::default();
        let api_circuit_breaker = CircuitBreaker::default();
        let dd_monitor          = DrawdownMonitor::new(capital, risk_gate.policy.max_drawdown_pct);
        // Candle önbelleği — interval başına max 500 candle tutar (REST/DB çağrısını ortadan kaldırır)
        let candle_cache = std::sync::Arc::new(std::sync::Mutex::new(CandleCache::new(500)));

        // ML durumlarını başlatmadan önce bir kez yükle — struct init'te iki kez okumayı önler.
        let classifier_snapshot = self.config.classifier_state_path.as_deref()
            .and_then(crate::robot::ml_engine::TradePatternClassifier::load_snapshot);

        // Equity snapshot: önceki session'daki cumulative_pnl + peak_equity kurtarılır.
        // Yalnızca aynı capital değerindeyse geçerli — capital değiştiyse discardedilir.
        let (saved_cumulative_pnl, saved_peak_equity) = load_equity_snapshot(capital);

        let mut ls = LoopState {
            capital,
            current_equity:     capital + saved_cumulative_pnl,
            day_start_equity:   capital + saved_cumulative_pnl,
            peak_equity:        saved_peak_equity,
            cumulative_pnl:     saved_cumulative_pnl,
            total_pnl:          0.0,
            total_trades:       0,
            win_trades:         0,
            session_closed:     0,
            session_wins:       0,
            loss_streak:        0,
            session_profit:     0.0,
            session_loss:       0.0,
            stop_loop:          false,
            error_count:        0,
            fetch_backoff_secs: 5,
            min_rr:             self.config.quality.min_rr,
            volatility_min_pct: self.config.quality.volatility_min_pct,
            volatility_max_pct: self.config.quality.volatility_max_pct,
            open_positions,
            sl_cooldown_map:    std::collections::HashMap::new(),
            sl_cooldown_secs:   self.config.sl_cooldown_secs.unwrap_or(600), // varsayılan 10 dk
            daily_sl_map:       std::collections::HashMap::new(),
            symbol_consec_loss:   std::collections::HashMap::new(),
            symbol_cooldown_secs:   std::collections::HashMap::new(),
            last_loss_time:         None,
            last_short_loss_time:   None,
            flip_cooldown_map:      std::collections::HashMap::new(),
            tp_win_dir_map:         std::collections::HashMap::new(),
            startup_time:       std::time::Instant::now(),
            startup_cooldown_secs: 90, // ilk 90 saniye pozisyon açma
            autonomous_controller,
            api_circuit_breaker,
            dd_monitor,
            risk_gate,
            recovery_supervisor,
            drift_detector: crate::robot::ml_engine::DriftDetector::default(),
            // DB bağlantısı döngü boyunca açık kalır — tick başına yeniden açılmaz.
            db_conn: self.config.db_path.as_deref()
                .and_then(|p| rusqlite::Connection::open(p).ok()),
            last_reloaded_params_version: 0,
            last_reloaded_adaptive_version: 0,
            candle_cache:   candle_cache.clone(),
            ranked_cache:        None,
            fv_cache:            None,
            grid_search_cache:   None,
            wal_tick_counter:    0,
            opt_tick_counter:    0,
            opt_result_rx:       None,
            adaptive_params: self.config.adaptive_params_path.as_deref()
                .map(AdaptiveTradeParams::load)
                .unwrap_or_default(),
            short_loss_streak: 0,
            hold_log_throttle: None,
            pending_evo_data:  std::collections::HashMap::new(),
            // Başlangıçta live_closed_trades'den doldur: önceki session kapanmışlarını da içerir
            closed_position_ids: self.config.live_state.as_ref()
                .and_then(|s| s.live_closed_trades.read().ok())
                .map(|log| log.iter().map(|t| t.pos_id).collect())
                .unwrap_or_default(),
            last_trade_time: std::collections::HashMap::new(),
            funding_rate_cache: None,
            last_health_check: std::time::Instant::now(),
            health_last_cycle_id: 0,
            health_same_cycle_count: 0,
            fsm_blocked_health_count: 0,
            cb_open_health_count: 0,
            last_nonhold_signal_at: None,
            scalp_stats: crate::robot::scalp_swing::ScalpSwingStats::default(),
            swing_stats:  crate::robot::scalp_swing::ScalpSwingStats::default(),
            scalp_swing_day_start_equity: capital,
            scalp_swing_sl_cooldown: std::collections::HashMap::new(),
            scalp_swing_consecutive_sl: std::collections::HashMap::new(),
            last_entry_at: std::collections::HashMap::new(),
            adx_regime:    crate::market_regime::AdxRegime::Neutral,
            cooloff_until: None,
            symbol_pnl_history: std::collections::HashMap::new(),
            dynamic_blacklist:  std::collections::HashMap::new(),
            btc_anchor_cache:   None,
            eth_anchor_cache:   None,
            // Classifier + buffer: disk'te snapshot varsa yükle, yoksa sıfırdan başla.
            // Restart'ta ML min_train=20 beklemeden devam eder.
            pattern_classifier: classifier_snapshot.as_ref()
                .map(|s| s.classifier.clone())
                .unwrap_or_default(),
            classifier_buffer: classifier_snapshot
                .map(|s| s.buffer)
                .unwrap_or_default(),
            // UCB1 scorer: disk'te durum varsa yükle — strateji disable/enable kararları korunur.
            strategy_scorer: self.config.scorer_state_path.as_deref()
                .map(crate::robot::strategy_scorer::StrategyScorer::load)
                .unwrap_or_default(),
            hf_error_log:       std::collections::HashMap::new(),
            symbol_slip_bps:    std::collections::HashMap::new(),
        };

        // ── Startup SL/TP taraması: REST API'den taze fiyatla SL/TP ihlallerini kapat ──────
        // ESKİ PROBLEM: snap_price = snapshot'taki bayat fiyat (ör. BTC=66869 ama gerçek=72641)
        // → Yanlış SL tetiklemesi: SL=70745 > snap=66869 → "SL hit" sanılıyor, pozisyon hatalı kapanıyor.
        // YENİ YAKLAŞIM:
        //   1. Her açık pozisyon için REST API'den son 1m mum çek (close = güncel fiyat)
        //   2. Fetch başarısız olursa pozisyonu ATLA — orphan handler ilk WS tick'inde halleder
        //   3. Taze fiyatla SL/TP kontrolü yap; exit_price SL/TP seviyesi (piyasa değil)
        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::collections::HashMap as FreshMap;

            // 1. Tüm açık pozisyonlar için unique (symbol, market) çift listesi
            let pos_targets: Vec<(crate::types::PositionId, String, crate::types::Market)> =
                ls.open_positions.iter()
                    .map(|(&id, pos)| (id, pos.symbol.clone(), pos.market))
                    .collect();

            // 2. Her (symbol, market) için REST'ten son 1m mum kapat fiyatı çek
            let mut fresh_prices: FreshMap<(String, &'static str), f64> = FreshMap::new();
            for (_id, sym, mkt) in &pos_targets {
                let map_key = (sym.clone(), mkt.as_str());
                if fresh_prices.contains_key(&map_key) { continue; }
                match self.fetcher.fetch_latest(
                    crate::types::Exchange::Binance, *mkt, sym, "1m", 1
                ).await {
                    Ok(candles) if !candles.is_empty() => {
                        let price = candles.last().unwrap().close;
                        fresh_prices.insert(map_key, price);
                    }
                    _ => {
                        // Fetch başarısız: bu sembol startup'ta kontrol edilmeyecek.
                        // Orphan handler ilk WS tick'inde taze fiyatla zaten kontrol eder.
                        self.logger.log_info("startup-sl-tp", &format!(
                            "⚠ {} ({}) taze fiyat alınamadı — orphan handler bekleyecek",
                            sym, mkt.as_str()
                        ));
                    }
                }
            }

            // 3. Taze fiyatla SL/TP kontrolü — fetch başarısız olanlar atlanır
            let triggered_ids: Vec<crate::types::PositionId> = ls.open_positions.iter()
                .filter_map(|(&id, pos)| {
                    let map_key = (pos.symbol.clone(), pos.market.as_str());
                    let &fresh_price = fresh_prices.get(&map_key)?; // fetch olmadıysa None → atla
                    if fresh_price <= 0.0 { return None; }
                    // static_sl/tp = 0.0 → henüz set edilmemiş seviye; kontrol etme
                    // (0.0 ile karşılaştırma: LONG tp_hit = fresh >= 0 her zaman true olurdu)
                    let sl_hit = pos.static_sl > 0.0
                        && (( pos.is_long && fresh_price <= pos.static_sl)
                          || (!pos.is_long && fresh_price >= pos.static_sl));
                    let tp_hit = pos.static_tp > 0.0
                        && (( pos.is_long && fresh_price >= pos.static_tp)
                          || (!pos.is_long && fresh_price <= pos.static_tp));
                    // Uygulama kapalıyken trailing SL ihlali — restart'ta yakala
                    let tsl_hit = pos.trailing_sl.map_or(false, |tsl| {
                        ( pos.is_long && fresh_price <= tsl) ||
                        (!pos.is_long && fresh_price >= tsl)
                    });
                    if sl_hit || tp_hit || tsl_hit { Some(id) } else { None }
                })
                .collect();

            // 4. Tetiklenen pozisyonları kapat
            for id in triggered_ids {
                if let Some(pos) = ls.open_positions.remove(&id) {
                    // Çoklu orchestrator worker aynı pozisyonu bağımsız olarak tetikleyebilir.
                    // Paylaşımlı live_closed_trades üzerinde erken dedup: bu pos_id zaten
                    // başka bir worker tarafından kapatıldıysa tekrar işleme.
                    let already_closed = self.config.live_state.as_ref()
                        .and_then(|s| s.live_closed_trades.read().ok())
                        .map(|log| is_duplicate_trade(&log, pos.id))
                        .unwrap_or(false);
                    if already_closed {
                        ls.closed_position_ids.insert(pos.id);
                        self.logger.log_info("startup-sl-tp", &format!(
                            "⏭ {} {} (id={}) startup taramasında zaten kapatılmış — atlanıyor",
                            pos.symbol, if pos.is_long { "LONG" } else { "SHORT" }, pos.id
                        ));
                        continue;
                    }

                    let map_key = (pos.symbol.clone(), pos.market.as_str());
                    let fresh_price = fresh_prices.get(&map_key).copied().unwrap_or(pos.static_sl);
                    // Öncelik: TP > trailing_sl > static_sl (position_manager ile tutarlı)
                    let exit_reason = if pos.static_tp > 0.0
                        && (( pos.is_long && fresh_price >= pos.static_tp)
                          || (!pos.is_long && fresh_price <= pos.static_tp))
                    {
                        "take_profit"
                    } else if pos.trailing_sl.map_or(false, |tsl|
                        ( pos.is_long && fresh_price <= tsl) ||
                        (!pos.is_long && fresh_price >= tsl))
                    {
                        "trailing_sl"
                    } else {
                        "static_sl"
                    };
                    // Adil çıkış: tetikleyen seviyeyi kullan (anlık piyasa fiyatı değil).
                    let exit_price = match exit_reason {
                        "trailing_sl" => pos.trailing_sl.unwrap_or(pos.static_sl),
                        "static_sl"   => pos.static_sl,
                        _             => pos.static_tp,
                    };
                    let pnl = pos.realized_pnl_with_commission(exit_price, self.config.commission_pct);
                    ls.record_pnl_startup(pnl); // loss_streak etkilenmez — önceki session kaybı
                    self.logger.log_info("startup-sl-tp", &format!(
                        "⚡ Startup SL/TP: {} {} kapandı ({}) | taze={:.4} çıkış={:.4} giriş={:.4} pnl={:+.2}",
                        pos.symbol, if pos.is_long { "LONG" } else { "SHORT" },
                        exit_reason, fresh_price, exit_price, pos.entry_price, pnl
                    ));
                    self.close_position_and_log(&pos, pos.market, exit_price, pnl, exit_reason, &mut ls);
                    crate::send_alert!(self.telegram,
                        "⚡ <b>Startup {}: {} {}</b> kapandı\nGiriş: {:.4} → Çıkış: {:.4}\nPnL: {:+.2} USD",
                        exit_reason.to_uppercase(),
                        pos.symbol, if pos.is_long { "LONG" } else { "SHORT" },
                        pos.entry_price, exit_price, pnl
                    );
                    ls.closed_position_ids.insert(pos.id);
                    // SL ile kapanan pozisyon zombie döngüsüne girmemesin:
                    // aynı sembol session içinde hemen yeniden açılmasın.
                    // Derin drawdown (fiyat entry'den >3% uzakta) → 4 saatlik cooldown (güçlü trend).
                    // Normal SL hit → 30 dakika cooldown.
                    // TP ile kapananlar cooldown'a alınmaz — kazanan sembol bloklanmaz.
                    if exit_reason == "static_sl" || exit_reason == "trailing_sl" {
                        let drawdown_pct = if pos.entry_price > 0.0 {
                            ((pos.entry_price - exit_price) / pos.entry_price * 100.0).abs()
                        } else { 0.0 };
                        let cooldown_secs: u64 = if drawdown_pct > 3.0 { 14400 } else { 1800 };
                        ls.symbol_cooldown_secs.insert(pos.symbol.clone(), cooldown_secs);
                        self.logger.log_info("startup-sl-tp", &format!(
                            "⏸ {} startup-{} sonrası cooldown {}dk (drawdown={:.1}%)",
                            pos.symbol, exit_reason, cooldown_secs / 60, drawdown_pct
                        ));
                        // SCP/SWG tip-bazlı startup cooldown
                        {
                            {
                                let key = format!("{}_{}", pos.symbol, pos.trade_type.label());
                                let count = ls.scalp_swing_consecutive_sl.entry(key.clone()).or_insert(0);
                                *count += 1;
                                let ss_cd = ss_cooldown_secs(pos.trade_type, *count);
                                if ss_cd > 0 {
                                    ls.scalp_swing_sl_cooldown.insert(key, (std::time::Instant::now(), ss_cd));
                                }
                            }
                        }
                    }
                }
            }

            // ── Startup Stale Orphan: fiyat sapmış ama SL/TP tetiklenmemiş pozisyonlar ──
            // Fiyat entry'den >%2 uzaklaştıysa pozisyon "stale" sayılır.
            // Limit emir ile temiz çıkış denenr; başarısız → manual_exit_required = true.
            {
                const STALE_DRIFT_PCT: f64 = 0.02;
                const BIP: f64 = 0.0001;
                const STALE_TIMEOUT_MS: u64 = 10_000;

                // Kapatılmamış, drift'i aşmış pozisyonları topla (borrow checker için önce collect)
                let stale_targets: Vec<(crate::types::PositionId, f64, bool, f64, String, crate::types::Market)> =
                    ls.open_positions.iter()
                        .filter_map(|(&id, pos)| {
                            if pos.manual_exit_required { return None; }
                            let map_key = (pos.symbol.clone(), pos.market.as_str());
                            let fresh = fresh_prices.get(&map_key).copied().filter(|&p| p > 0.0)?;
                            let drift = (fresh - pos.entry_price).abs() / pos.entry_price;
                            if drift > STALE_DRIFT_PCT {
                                Some((id, fresh, pos.is_long, pos.qty, pos.symbol.clone(), pos.market))
                            } else {
                                None
                            }
                        })
                        .collect();

                for (id, fresh_price, is_long, qty, sym, mkt) in stale_targets {
                    // Drift > %2 → Limit emir ile çıkış dene
                    let tick = fresh_price * BIP;
                    let limit_price = if is_long {
                        // LONG kapat → SELL limit, best_bid'in 1 tick altında
                        fresh_price - tick
                    } else {
                        // SHORT kapat → BUY limit, best_ask'ın 1 tick üstünde
                        fresh_price + tick
                    };
                    let close_signal = if is_long { Signal::Sell } else { Signal::Buy };
                    let drift_pct = (fresh_price - ls.open_positions.get(&id)
                        .map(|p| p.entry_price).unwrap_or(fresh_price)).abs()
                        / ls.open_positions.get(&id).map(|p| p.entry_price).unwrap_or(fresh_price) * 100.0;

                    self.logger.log_info("startup-stale", &format!(
                        "⚠ {} {} fiyat sapması={:.2}% >%2 — limit çıkış deneniyor @ {:.4}",
                        sym, if is_long { "LONG" } else { "SHORT" }, drift_pct, limit_price
                    ));

                    match self.executor.executor.execute_limit(close_signal, &sym, qty, limit_price, STALE_TIMEOUT_MS) {
                        Ok(trade) => {
                            // DummyTradeExecutor sahte sentinel döner (örn. 100.0). Yalnızca
                            // taze fiyatın ±%30'u içindeki fill'i kabul et; aksi halde limit'e düş.
                            let fill_price = if trade.entry_price > fresh_price * 0.7
                                && trade.entry_price < fresh_price * 1.3
                            {
                                trade.entry_price
                            } else {
                                if trade.entry_price > 0.0 {
                                    self.logger.log_info("startup-stale", &format!(
                                        "⚠ {} executor fill={:.4} taze fiyattan ({:.4}) çok uzakta — limit_price kullanılıyor",
                                        sym, trade.entry_price, fresh_price
                                    ));
                                }
                                limit_price
                            };
                            if let Some(pos) = ls.open_positions.remove(&id) {
                                let pnl = pos.realized_pnl_with_commission(fill_price, self.config.commission_pct);
                                ls.record_pnl_startup(pnl);
                                self.logger.log_info("startup-stale", &format!(
                                    "✅ Stale limit çıkış: {} @ {:.4} pnl={:+.2}", sym, fill_price, pnl
                                ));
                                self.close_position_and_log(&pos, mkt, fill_price, pnl, "stale_drift_limit", &mut ls);
                                crate::send_alert!(self.telegram,
                                    "⚠ <b>Startup Stale: {} {}</b> limit çıkış\nDrift: {:.2}% | PnL: {:+.2} USD",
                                    sym, if is_long { "LONG" } else { "SHORT" }, drift_pct, pnl
                                );
                                ls.closed_position_ids.insert(pos.id);
                            }
                        }
                        Err(e) => {
                            self.logger.log_error("startup-stale", &format!(
                                "❌ {} stale limit çıkış başarısız — MANUAL EXIT REQUIRED: {}", sym, e
                            ));
                            if let Some(pos) = ls.open_positions.get_mut(&id) {
                                pos.manual_exit_required = true;
                            }
                        }
                    }
                }
            }

            // Startup taraması sonrası: kapanan pozisyonları DB snapshot'tan temizle.
            // Aksi hâlde bir sonraki restart'ta aynı pozisyonlar tekrar işlenir ("phantom trade").
            if let Some(conn) = &ls.db_conn {
                if ls.open_positions.is_empty() {
                    let _ = crate::database_writer::clear_open_positions_snapshot(conn);
                } else {
                    if let Ok(json) = serde_json::to_string(
                        &ls.open_positions.values().collect::<Vec<_>>()
                    ) {
                        let _ = crate::database_writer::save_open_positions_snapshot(conn, &json);
                    }
                }
            }
        }

        let ml_model = self.ml_model.take();
        let ml_data  = self.ml_data.take().unwrap_or_default();
        #[cfg(not(target_arch = "wasm32"))]
        let portfolio = Portfolio::new(capital, None);
        let mut monitor = self.monitor.take();

        // ── CandleCache başlangıç verisi: interval + HTF'yi bir kez çek ──────
        // Bu, ilk iterasyonda process_symbol'ün REST/DB'ye gitmesini önler.
        {
            let seed_symbol   = self.config.symbol.clone();
            let seed_market   = self.config.market;
            let seed_interval = self.config.interval.clone();
            let htf_interval  = htf_for_interval(&seed_interval);

            // Ana interval geçmiş verisi
            match self.fetcher.fetch_latest(Exchange::Binance, seed_market, &seed_symbol, &seed_interval, candle_limit).await {
                Ok(hist) if !hist.is_empty() => {
                    let mut cc = candle_cache.lock().unwrap();
                    for c in hist { cc.push(c); }
                    self.logger.log_info("cache", &format!(
                        "CandleCache seed: {} {} candle [{}]",
                        seed_symbol, cc.len(&seed_interval), seed_interval
                    ));
                }
                Ok(_) => self.logger.log_error("cache", "Seed: boş interval verisi"),
                Err(e) => self.logger.log_error("cache", &format!("Seed fetch hatası: {}", e)),
            }

            // HTF geçmiş verisi (farklı interval ise)
            if htf_interval != seed_interval.as_str() {
                let htf_ok = match self.fetcher.fetch_latest(Exchange::Binance, seed_market, &seed_symbol, htf_interval, 200).await {
                    Ok(htf) if !htf.is_empty() => {
                        let mut cc = candle_cache.lock().unwrap();
                        for c in htf { cc.push(c); }
                        true
                    }
                    _ => false,
                };
                // REST boşsa DB'den yükle
                if !htf_ok {
                    if let Some(db_path) = &self.config.db_path {
                        let market_str = format!("{:?}", seed_market).to_lowercase();
                        if let Ok(db_candles) = load_htf_candles_from_db(db_path, &seed_symbol, &market_str, htf_interval, 200) {
                            let mut cc = candle_cache.lock().unwrap();
                            for c in db_candles { cc.push(c); }
                        }
                    }
                }
                self.logger.log_info("cache", &format!(
                    "CandleCache seed: {} {} candle [HTF={}]",
                    seed_symbol,
                    candle_cache.lock().unwrap().len(htf_interval),
                    htf_interval
                ));
            }
        }

        // ── CandleSynth kurulumu ──────────────────────────────────────────────
        #[inline]
        fn interval_to_idx(s: &str) -> usize {
            match s { "1m"=>0,"5m"=>1,"15m"=>2,"30m"=>3,"1h"=>4,"4h"=>5,"1d"=>6, _=>0 }
        }
        let interval_cycle_ids_ptr = std::sync::Arc::new(std::sync::Mutex::new([0u64; 7]));
        let controller_ptr = std::sync::Arc::new(std::sync::Mutex::new(ls.autonomous_controller.clone()));
        let logger_ptr     = std::sync::Arc::new(self.logger);
        // [0]=1m [1]=5m [2]=15m [3]=30m [4]=1h [5]=4h [6]=1d
        let synth_counts   = std::sync::Arc::new(std::sync::Mutex::new([0u64; 7]));
        let synth_callback = {
            let interval_cycle_ids_ptr = interval_cycle_ids_ptr.clone();
            let controller_ptr         = controller_ptr.clone();
            let logger_ptr             = logger_ptr.clone();
            let synth_counts           = synth_counts.clone();
            let candle_cache_synth     = candle_cache.clone();
            move |candle: &Candle| {
                // Sentezlenen candle'ı önbelleğe yaz (process_symbol REST'e gitmesin)
                if let Ok(mut cc) = candle_cache_synth.lock() {
                    cc.push(candle.clone());
                }
                let id = {
                    let mut ids = interval_cycle_ids_ptr.lock().unwrap();
                    let idx = interval_to_idx(&candle.interval);
                    ids[idx] += 1;
                    ids[idx]
                };
                {
                    let mut sc = synth_counts.lock().unwrap();
                    sc[interval_to_idx(&candle.interval)] += 1;
                    if candle.interval == "1h" {
                        let c1h = sc[4];
                        if c1h % 200 == 0 {
                            let c5m  = sc[1];
                            let c15m = sc[2];
                            let c30m = sc[3];
                            let c4h  = sc[5];
                            logger_ptr.log_info(
                                "candle_synth",
                                &format!("🕯 Sentez özeti — 5m:{} 15m:{} 30m:{} 1h:{} 4h:{} | C:{:.2} (cycle={})",
                                    c5m, c15m, c30m, c1h, c4h, candle.close, id),
                            );
                        }
                    }
                }
                if candle.interval == "1h" {
                    let mut ctrl = controller_ptr.lock().unwrap();
                    // cycle_id = id YAPILMIYOR: restart'ta sıfırlama olmasın.
                    // begin_cycle() mevcut cycle_id'yi arttırır; initial_cycle_id'den devam eder.
                    ctrl.begin_cycle();
                    if ctrl.evolution_enabled && ctrl.should_evolve() {
                        ctrl.evolve_population();
                        save_evolution_state_from_loop(&ctrl);
                        logger_ptr.log_info("evolution",
                            &format!("🧬 [CandleSynth] 1h cycle_id={} — evrim tetiklendi", ctrl.cycle_id));
                    }
                }
            }
        };
        let symbol   = self.config.symbol.clone();
        let _market  = self.config.market;
        let mut synth = CandleSynth::new(&symbol, Box::new(synth_callback));

        // ── WebSocket gerçek zamanlı fiyat akışı ─────────────────────────────
        let ws_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let _ws_price_handle = self.config.live_state.as_ref().map(|state| {
            crate::robot::price_feed::RealtimePriceFeed::new(
                self.config.symbol.clone(),
                self.config.market,
                std::sync::Arc::clone(state),
                std::sync::Arc::clone(&ws_stop),
            ).spawn()
        });

        // İlk iterasyonda seeded cache ile process_symbol çalışsın; sonraki
        // iterasyonlarda yalnızca yapılandırılan interval kapandığında çalışır.
        let mut evaluated_at_least_once = false;

        // ── Ana döngü ─────────────────────────────────────────────────────────
        loop {
            let symbol = self.config.symbol.clone();
            let market = self.config.market;
            let candles_1m = match self.fetcher.fetch_latest(Exchange::Binance, market, &symbol, "1m", candle_limit).await {
                Ok(c) => {
                    ls.fetch_backoff_secs = 5;
                    c
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("durduruldu") { break; }
                    self.logger.log_error("fetcher", &format!("1m veri çekme hatası: {} (symbol={}, market={:?})", e, symbol, market));
                    tokio::time::sleep(Duration::from_secs(ls.fetch_backoff_secs)).await;
                    ls.fetch_backoff_secs = (ls.fetch_backoff_secs * 2).min(60);
                    continue;
                }
            };
            // 1m mumları sentezleyiciden geçir; kapanan üst interval'ları yakala
            let mut newly_emitted: Vec<crate::types::Candle> = Vec::new();
            for candle in &candles_1m {
                newly_emitted.extend(synth.push_1m(candle.clone()));
            }

            // 1m candle'ları önbelleğe yaz (sadece yeni/güncellenmiş olanlar deduplicate edilir)
            {
                let mut cc = ls.candle_cache.lock().unwrap();
                for c in &candles_1m { cc.push(c.clone()); }
            }

            // ── DB kalıcılığı: 1m + sentezlenen HTF mumları kaydet ──────────────
            // ls.db_conn döngü boyunca açık (Fix-3) — tick başına yeniden açılmaz.
            if let Some(conn) = &ls.db_conn {
                let market_str = match market {
                    crate::types::Market::Spot    => "spot",
                    crate::types::Market::Futures => "futures",
                    crate::types::Market::Coinm   => "coinm",
                };
                // 1m mumları toplu kaydet (tek transaction — INSERT OR IGNORE, duplicate'ler atlanır)
                let _ = crate::database_writer::save_candles_bulk(conn, "binance", market_str, &candles_1m);
                // Sentezlenen HTF mumları teker teker kaydet (sayıca az)
                for c in &newly_emitted {
                    let _ = crate::database_writer::save_candle(conn, "binance", market_str, c);
                }
            }

            // Canlı fiyatı 1m verisinden güncelle
            if let Some(last_1m) = candles_1m.last() {
                let is_active = self.config.live_state.as_ref()
                    .and_then(|s| s.live_active_symbol.read().ok())
                    .map(|a| *a == symbol)
                    .unwrap_or(symbol == self.config.symbol);
                if is_active {
                    // ── WS vs REST fiyat sapma kontrolü ──────────────────────────────
                    // WS fiyatı (RealtimePriceFeed) ile REST kapanış fiyatı arasındaki
                    // fark %0.5'i geçerse uyarı yazılır. Daha büyük sapmalar veri
                    // bütünlüğü sorununa işaret edebilir (stale WS, ağ hatası vs.).
                    let ws_price = self.config.live_state.as_ref()
                        .and_then(|s| s.live_price.read().ok())
                        .map(|pd| pd.close)
                        .unwrap_or(0.0);
                    if ws_price > 0.0 && last_1m.close > 0.0 {
                        let divergence = (ws_price - last_1m.close).abs() / last_1m.close;
                        if divergence > 0.005 {
                            // %0.5 üzeri sapma — olası veri tutarsızlığı
                            self.logger.log_error("price-divergence", &format!(
                                "⚠ WS/REST fiyat sapması: WS={:.4} REST={:.4} fark={:.3}% [{symbol}]",
                                ws_price, last_1m.close, divergence * 100.0
                            ));
                        } else if divergence > 0.002 {
                            // %0.2-0.5 arası — hafif gecikme, bilgi amaçlı
                            self.logger.log_info("price-divergence", &format!(
                                "WS/REST fiyat farkı: {:.3}% (WS={:.4} REST={:.4}) [{symbol}]",
                                divergence * 100.0, ws_price, last_1m.close
                            ));
                        }
                    }

                    self.config.with_live(|s| s.update_price(|pd| {
                        pd.symbol     = symbol.clone();
                        pd.open       = last_1m.open;
                        pd.high       = last_1m.high;
                        pd.low        = last_1m.low;
                        pd.close      = last_1m.close;
                        pd.volume     = last_1m.volume;
                        pd.change_pct = if last_1m.open > 0.0 { (last_1m.close - last_1m.open) / last_1m.open * 100.0 } else { 0.0 };
                        pd.ts         = last_1m.timestamp.with_timezone(&chrono::Local).format("%H:%M:%S").to_string();
                        pd.last_updated_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;
                    }));
                }
            }

            // Controller'ı synth callback'ten senkronize et
            if let Ok(ctrl) = controller_ptr.lock() {
                ls.autonomous_controller.cycle_id = ctrl.cycle_id;
                self.config.with_live(|s| s.update_evolution(|evo| {
                    evo.evolution_enabled     = ctrl.evolution_enabled;
                    evo.brain_active          = ctrl.adaptive_brain.is_some();
                    evo.pop_active            = ctrl.population_manager.is_some();
                    evo.evolve_every_n_cycles = ctrl.evolve_every_n_cycles;
                    evo.cycle_id              = ctrl.cycle_id;
                    if let Some(g) = &ctrl.current_strategy_genome {
                        evo.genome_id       = g.id.clone();
                        evo.genome_fitness  = g.fitness;
                        evo.genome_trades   = g.trade_count;
                        evo.genome_win_rate = g.win_rate;
                    }
                    evo.brain_summary = ctrl.adaptive_brain.as_ref()
                        .map(|b| b.get_summary()).unwrap_or_default();
                    evo.pop_summary   = ctrl.population_manager.as_ref()
                        .map(|p| p.get_summary()).unwrap_or_default();
                }));
            }

            // Otonom kontrol: trade yapılabilir mi?
            if self.config.autonomous_enabled && !ls.autonomous_controller.can_trade() {
                self.logger.log_error("autonomous", &format!(
                    "Otonom kontrol trade'i engelledi: state={} cycle={}",
                    ls.autonomous_controller.state,
                    ls.autonomous_controller.cycle_id
                ));
                tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
                continue;
            }

            if Utc::now().hour() == 0 && Utc::now().minute() < 1 {
                ls.day_start_equity = ls.current_equity;
                ls.scalp_swing_day_start_equity = ls.current_equity;
            }

            // ── Force-flag'leri TUI'den oku (Tab 8 → loop komutları) ────────────
            let (force_evol, force_funding) = self.config.live_state.as_ref()
                .map(|s| s.read_pipeline_flags())
                .unwrap_or((false, false));
            if force_evol {
                self.logger.log_info("pipeline", "⚡ TUI: Mini evrim zorla tetiklendi");
                self.maybe_evolve(&mut ls);
                self.config.live_state.as_ref().map(|s| s.update_pipeline(|p| {
                    p.force_mini_evolution = false;
                    p.mini_evol_count += 1;
                    p.last_evolution_trigger = format!("TUI isteği — {}", chrono::Local::now().format("%H:%M:%S"));
                    p.log_repair("⚡ TUI: Mini evrim zorla çalıştırıldı");
                }));
            }
            if force_funding {
                ls.funding_rate_cache = None; // TTL sıfırla → sonraki tick'te fetch edilir
                self.config.live_state.as_ref().map(|s| s.update_pipeline(|p| {
                    p.force_funding_refresh = false;
                    p.log_repair("🔄 TUI: Funding rate önbelleği temizlendi, yeniden çekilecek");
                }));
            }

            // ── Her tick: candle tazeliğini live_pipeline'a yaz (TUI donmasın) ──
            {
                let candle_age_secs_live = candles_1m.last()
                    .map(|c| (Utc::now() - c.timestamp).num_seconds().max(0) as u64)
                    .unwrap_or(999);
                let last_candle_at_live = candles_1m.last()
                    .map(|c| c.timestamp.with_timezone(&chrono::Local).format("%H:%M:%S").to_string())
                    .unwrap_or_else(|| "—".to_string());
                // 1m mum tazeliği ölçülüyor; trading interval ne olursa olsun
                // 1m WS feed 5 dakikadan uzun süredir gelmiyorsa stale kabul et.
                let interval_secs_cfg: u64 = parse_interval_secs(&self.config.interval);
                let stale_threshold = (interval_secs_cfg * 4 + 60).min(600);
                let ws_stale_live = candle_age_secs_live > stale_threshold;
                self.config.live_state.as_ref().map(|s| s.update_pipeline(|p| {
                    p.candle_age_secs = candle_age_secs_live;
                    p.last_candle_at  = last_candle_at_live.clone();
                    p.ws_stale        = ws_stale_live;
                }));
            }

            // ── Periyodik sağlık kontrolü + pipeline yazımı (her 5 dk) ──────────
            if ls.last_health_check.elapsed().as_secs() >= 300 {
                ls.last_health_check = std::time::Instant::now();

                // 1) Candle tazeliği (health log için — pipeline zaten her tick güncelleniyor)
                let candle_age_secs = candles_1m.last()
                    .map(|c| (Utc::now() - c.timestamp).num_seconds().max(0) as u64)
                    .unwrap_or(999);
                let interval_secs_cfg: u64 = parse_interval_secs(&self.config.interval);
                let stale_threshold = (interval_secs_cfg * 4 + 60).min(600);
                let ws_stale = candle_age_secs > stale_threshold;
                if ws_stale {
                    self.logger.log_error("health", &format!(
                        "⚠ Veri akışı yavaş: son 1m mum {}sn önce (>{}sn)", candle_age_secs, stale_threshold
                    ));
                    crate::send_alert!(self.telegram,
                        "⚠️ <b>Veri akışı yavaş</b>\n{} son mum {}sn önce", symbol, candle_age_secs
                    );
                }

                // 2) Evrim takıp
                let cur_cycle = ls.autonomous_controller.cycle_id;
                if self.config.autonomous_enabled {
                    if cur_cycle == ls.health_last_cycle_id {
                        ls.health_same_cycle_count += 1;
                        if ls.health_same_cycle_count >= 10 {
                            self.logger.log_error("health", &format!(
                                "⚠ Evrim takıp: cycle_id={} {}x kontrolde değişmedi",
                                cur_cycle, ls.health_same_cycle_count
                            ));
                        }
                    } else {
                        ls.health_last_cycle_id    = cur_cycle;
                        ls.health_same_cycle_count = 0;
                    }
                }

                // 3) DB sağlığı
                let db_ok = ls.db_conn.as_ref().map(|conn| {
                    conn.query_row("SELECT 1", [], |_| Ok(())).is_ok()
                }).unwrap_or(true);
                if !db_ok {
                    self.logger.log_error("health", "⚠ DB bağlantı sağlık kontrolü başarısız");
                }

                // 4) Funding rate tazeliği
                let funding_age = ls.funding_rate_cache.as_ref()
                    .map(|(_, t)| t.elapsed().as_secs())
                    .unwrap_or(0);
                let funding_rate = ls.funding_rate_cache.as_ref()
                    .map(|(fr, _)| fr.funding_rate)
                    .unwrap_or(0.0);
                let funding_stale = matches!(self.config.market, Market::Futures | Market::Coinm)
                    && ls.funding_rate_cache.is_some() && funding_age > 600;

                // 5) Anomali listesi oluştur
                let wr_pct = if ls.session_closed > 0 {
                    ls.session_wins as f64 / ls.session_closed as f64 * 100.0
                } else { 0.0 };
                let mut anomalies: Vec<PipelineAnomaly> = Vec::new();

                if ws_stale {
                    anomalies.push(PipelineAnomaly {
                        kind:       AnomalyKind::StaleCandles,
                        severity:   AnomSeverity::Critical,
                        message:    format!("Son mum {}sn önce — veri akışı kesik olabilir", candle_age_secs),
                        auto_fixed: false,
                        fix_hint:   "Ctrl+C → yeniden başlat veya ağ bağlantısını kontrol et".to_string(),
                    });
                }
                if !db_ok {
                    anomalies.push(PipelineAnomaly {
                        kind:       AnomalyKind::DbDisconnected,
                        severity:   AnomSeverity::Critical,
                        message:    "DB bağlantısı yanıt vermiyor".to_string(),
                        auto_fixed: false,
                        fix_hint:   "data/ dizinini ve disk alanını kontrol et".to_string(),
                    });
                }
                if self.config.autonomous_enabled && ls.health_same_cycle_count >= 10 {
                    let auto_fixed = ls.health_same_cycle_count == 10; // ilk tespitte tetikle
                    if auto_fixed { self.maybe_evolve(&mut ls); }
                    anomalies.push(PipelineAnomaly {
                        kind:       AnomalyKind::EvolutionStuck,
                        severity:   AnomSeverity::Warning,
                        message:    format!("Evrim cycle_id={} {}dk değişmedi", cur_cycle, ls.health_same_cycle_count * 5),
                        auto_fixed,
                        fix_hint:   "Tab 8 → [E] Mini Evrim Zorla".to_string(),
                    });
                }
                if ls.drift_detector.drift_score > ls.drift_detector.threshold {
                    anomalies.push(PipelineAnomaly {
                        kind:       AnomalyKind::HighDrift,
                        severity:   AnomSeverity::Warning,
                        message:    format!("Drift {:.2} > eşik {:.2} — piyasa rejimi kaydı", ls.drift_detector.drift_score, ls.drift_detector.threshold),
                        auto_fixed: true,
                        fix_hint:   "Mini evrim otomatik tetiklendi".to_string(),
                    });
                }
                if ls.loss_streak >= 3 {
                    anomalies.push(PipelineAnomaly {
                        kind:       AnomalyKind::ConsecLosses,
                        severity:   if ls.loss_streak >= 5 { AnomSeverity::Critical } else { AnomSeverity::Warning },
                        message:    format!("{} ardışık zarar — mini evrim tetiklendi", ls.loss_streak),
                        auto_fixed: true,
                        fix_hint:   "SL cooldown aktif — pozisyon boyutunu düşür".to_string(),
                    });
                }
                // ── FSM takılı: 3 ardışık sağlık kontrolünde can_trade()=false ─────────────
                if self.config.autonomous_enabled && !ls.autonomous_controller.can_trade() {
                    ls.fsm_blocked_health_count += 1;
                    // SafeMode kendi cooldown mekanizmasıyla çıkar; Halted ise kalıcı.
                    // 3 kontrolde (15 dk) hâlâ SafeMode → cooldown'u sıfırla ve Observe'e döndür.
                    // Halted → 5 kontrolde (25 dk) reset (çok agresif başarısızlık geçmişi olsa da kurtarılır).
                    let auto_fsm_fixed = if ls.autonomous_controller.state == crate::robot::autonomous_control::AutonomousState::Halted
                        && ls.fsm_blocked_health_count >= 5
                    {
                        ls.autonomous_controller.force_observe();
                        ls.fsm_blocked_health_count = 0;
                        self.logger.log_info("watchdog", "🔄 FSM Halted 25dk geçti — Observe'e zorla sıfırlandı");
                        true
                    } else if ls.autonomous_controller.state == crate::robot::autonomous_control::AutonomousState::SafeMode
                        && ls.fsm_blocked_health_count >= 3
                    {
                        ls.autonomous_controller.force_observe();
                        ls.fsm_blocked_health_count = 0;
                        self.logger.log_info("watchdog", "🔄 FSM SafeMode 15dk geçti — Observe'e zorla kurtarıldı");
                        true
                    } else { false };
                    anomalies.push(PipelineAnomaly {
                        kind:       AnomalyKind::FsmBlocked,
                        severity:   AnomSeverity::Warning,
                        message:    format!("FSM engelledi — can_trade()=false, state={} ({}. kontrol)",
                            ls.autonomous_controller.state, ls.fsm_blocked_health_count),
                        auto_fixed: auto_fsm_fixed,
                        fix_hint:   "[P] ile duraklat/devam et; çok hata varsa yeniden başlat".to_string(),
                    });
                } else {
                    ls.fsm_blocked_health_count = 0; // düzeldi → sayacı sıfırla
                }

                // ── Circuit Breaker uzun süredir açık ─────────────────────────────────────
                if ls.api_circuit_breaker.state() == CircuitBreakerState::Open {
                    ls.cb_open_health_count += 1;
                    // 6 kontrolde (30 dk) CB hâlâ Open → zorla kapat
                    let auto_cb_fixed = if ls.cb_open_health_count >= 6 {
                        ls.api_circuit_breaker.reset();
                        ls.cb_open_health_count = 0;
                        self.logger.log_info("watchdog", "🔄 Circuit Breaker 30dk açık kaldı — zorla kapatıldı (reset)");
                        true
                    } else { false };
                    anomalies.push(PipelineAnomaly {
                        kind:       AnomalyKind::CircuitBreakerOpen,
                        severity:   if ls.cb_open_health_count >= 3 { AnomSeverity::Critical } else { AnomSeverity::Warning },
                        message:    format!("API circuit breaker açık — emir gönderimi durdu ({}. kontrol)", ls.cb_open_health_count),
                        auto_fixed: auto_cb_fixed,
                        fix_hint:   "Binance API bağlantısını kontrol et; Tab 8 → onarım logunu gözlemle".to_string(),
                    });
                } else {
                    ls.cb_open_health_count = 0;
                }

                // ── Pozisyon takılı (çok uzun süredir açık) ──────────────────────────────
                {
                    let max_pos_age_secs: u64 = 48 * 3600; // 48 saat
                    for pos in ls.open_positions.values() {
                        if pos.opened_at.is_empty() { continue; }
                        // RFC3339 veya "YYYY-MM-DD HH:MM:SS" formatını dene
                        let opened_utc: Option<chrono::DateTime<chrono::Utc>> =
                            chrono::DateTime::parse_from_rfc3339(&pos.opened_at)
                                .map(|dt| dt.with_timezone(&chrono::Utc))
                                .ok()
                                .or_else(|| {
                                    chrono::NaiveDateTime::parse_from_str(&pos.opened_at, "%Y-%m-%d %H:%M:%S")
                                        .ok()
                                        .map(|ndt| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(ndt, chrono::Utc))
                                });
                        let age_secs: u64 = opened_utc
                            .map(|dt| (chrono::Utc::now() - dt).num_seconds().max(0) as u64)
                            .unwrap_or(0);
                        if age_secs > max_pos_age_secs {
                            anomalies.push(PipelineAnomaly {
                                kind:       AnomalyKind::PositionStuck,
                                severity:   AnomSeverity::Warning,
                                message:    format!("{} {} {}sa {:.0}dk açık — SL/TP ihlal edilmedi; olağandışı",
                                    pos.symbol, if pos.is_long { "LONG" } else { "SHORT" },
                                    age_secs / 3600, (age_secs % 3600) as f64 / 60.0),
                                auto_fixed: false,
                                fix_hint:   format!("Tab 4 Pozisyonlar → {} manuel kapat veya SL seviyesini düşür", pos.symbol),
                            });
                            self.logger.log_info("watchdog", &format!(
                                "⚠ Pozisyon takılı: {} {} {}sa açık (giriş={:.4})",
                                pos.symbol, if pos.is_long {"LONG"} else {"SHORT"}, age_secs/3600, pos.entry_price
                            ));
                        }
                    }
                }

                // ── Sinyal kuruluğu: 12 saattir non-Hold sinyal yok ──────────────────────
                // None iken u64::MAX dönüp overflow'a sebep olmasın; ilk sinyal gelmeden watchdog atlar.
                if self.config.autonomous_enabled {
                    let drought_secs: u64 = 12 * 3600;
                    if let Some(signal_age) = ls.last_nonhold_signal_at.map(|t| t.elapsed().as_secs()) {
                        if signal_age > drought_secs {
                            // Mini evrim tetikle — strateji/parametreleri yenile
                            self.maybe_evolve(&mut ls);
                            anomalies.push(PipelineAnomaly {
                                kind:       AnomalyKind::SignalDrought,
                                severity:   AnomSeverity::Warning,
                                message:    format!("{}sa {:.0}dk süredir non-Hold sinyal yok — mini evrim tetiklendi",
                                    signal_age / 3600, (signal_age % 3600) as f64 / 60.0),
                                auto_fixed: true,
                                fix_hint:   "Backtest yeniden çalıştır; strateji eşiklerini kontrol et".to_string(),
                            });
                            self.logger.log_info("watchdog", &format!(
                                "🔄 Sinyal kuruluğu {}sa → mini evrim tetiklendi",
                                signal_age / 3600
                            ));
                            // Kuruluğu resetle: bir sonraki kontrol tekrar beklemeye başlasın
                            ls.last_nonhold_signal_at = Some(std::time::Instant::now());
                        }
                    }
                }

                if funding_stale {
                    anomalies.push(PipelineAnomaly {
                        kind:       AnomalyKind::FundingStale,
                        severity:   AnomSeverity::Warning,
                        message:    format!("Funding rate {}dk önce çekildi (>10dk)", funding_age / 60),
                        auto_fixed: false,
                        fix_hint:   "Tab 8 → [F] Funding Yenile".to_string(),
                    });
                }
                if ls.session_closed >= 20 && wr_pct < 30.0 {
                    anomalies.push(PipelineAnomaly {
                        kind:       AnomalyKind::LowWinRate,
                        severity:   AnomSeverity::Warning,
                        message:    format!("Win rate {:.0}% < %30 ({} işlemde)", wr_pct, ls.session_closed),
                        auto_fixed: false,
                        fix_hint:   "Strateji parametrelerini gözden geçir; backtest yeniden çalıştır".to_string(),
                    });
                }

                // 6) Pipeline state'i yaz
                let kelly_trades = ls.session_closed;
                let drift = ls.drift_detector.drift_score;
                let drift_thr = ls.drift_detector.threshold;
                let loss_streak = ls.loss_streak;
                let sw = ls.session_wins;
                let sc = ls.session_closed;
                let stuck = ls.health_same_cycle_count;
                let is_futures = matches!(self.config.market, Market::Futures | Market::Coinm);
                self.config.live_state.as_ref().map(|s| s.update_pipeline(|p| {
                    // candle_age_secs / last_candle_at / ws_stale → her tick güncelleniyor (yukarıda)
                    p.db_connected           = db_ok;
                    p.funding_rate           = funding_rate;
                    p.funding_age_secs       = funding_age;
                    p.funding_applicable     = is_futures;
                    p.kelly_trades_so_far    = kelly_trades;
                    p.evolution_cycle        = cur_cycle;
                    p.evolution_stuck_count  = stuck;
                    p.drift_score            = drift;
                    p.drift_threshold        = drift_thr;
                    p.loss_streak            = loss_streak;
                    p.session_wins           = sw;
                    p.session_closed         = sc;
                    p.anomalies              = anomalies.clone();
                    // Auto-fixed anomalileri onarım günlüğüne yaz
                    for a in &anomalies {
                        if a.auto_fixed {
                            p.log_repair(&format!("✅ Otomatik düzeltildi: {}", a.message));
                        }
                    }
                }));

                // 7) Genel özet log
                let anom_str = if anomalies.is_empty() {
                    "✅ Anomali yok".to_string()
                } else {
                    format!("⚠ {} anomali", anomalies.len())
                };
                self.logger.log_info("health", &format!(
                    "Sağlık: equity={:.2} WR={:.0}% cycle={} candle={}sn drift={:.2} {}",
                    ls.current_equity, wr_pct, cur_cycle, candle_age_secs, drift, anom_str
                ));
            }

            // Trade quality hot-reload
            if let Some(path) = &self.config.trade_quality_config_path.clone() {
                if let Some(reloaded) = load_trade_quality_config_from_file(&path) {
                    if reloaded != self.config.quality {
                        self.config.quality = reloaded;
                        self.logger.log_info("quality", "Trade quality config hot-reload uygulandı");
                    }
                }
            }

            // Robotic profiles hot-reload — B1/B2/B3 parametreleri ve SL cooldown
            if let Some(path) = &self.config.robotic_profiles_path.clone() {
                if let Some((sl_cd, be_rr, atr_m, ptp)) = load_robotic_profiles(path) {
                    let changed =
                        sl_cd.map(|v| v as f64) != self.config.sl_cooldown_secs.map(|v| v as f64) ||
                        be_rr != self.config.breakeven_at_rr ||
                        atr_m != self.config.atr_trail_mult  ||
                        ptp   != self.config.partial_tp_ratio;
                    if changed {
                        if let Some(v) = sl_cd { self.config.sl_cooldown_secs = Some(v); ls.sl_cooldown_secs = v; }
                        self.config.breakeven_at_rr  = be_rr;
                        self.config.atr_trail_mult   = atr_m;
                        self.config.partial_tp_ratio = ptp;
                        self.logger.log_info("profiles", &format!(
                            "robotic_profiles hot-reload: sl_cd={:?} be={:?} atr={:?} ptp={:?}",
                            sl_cd, be_rr, atr_m, ptp
                        ));
                    }
                }
            }

            // Strateji parametrelerini sıcak yükle — versiyon değişmemişse atlanır (Fix-4)
            self.reload_strategy_params(&mut ls);
            // Adaptive params sıcak yükle — TUI'dan [e]/←→ sonrası anında geçerli olur
            self.reload_adaptive_params(&mut ls);

            // Per-tick sayaçları sıfırla
            ls.reset_tick_counters();

            let (fetcher, _fetcher_name): (&dyn LiveDataFetcher, &str) = match self.config.mode {
                RunMode::Live     => (self.fetcher, self.fetcher.source_name()),
                RunMode::Backtest => {
                    let f = self.backtest_fetcher.expect("Backtest fetcher tanımlı olmalı");
                    (f, f.source_name())
                }
            };
            let markets = fetcher.supported_markets();
            let _model_ensemble: Vec<MLModel> = vec![];

            // ── Sinyal değerlendirme: yalnızca yapılandırılan interval kapandığında ──
            // 1m config: her tick değerlendir (CandleSynth 1m emit etmez, REST'ten gelir).
            // Diğer intervallar: newly_emitted içinde o interval kapandıysa değerlendir.
            // İlk iterasyonda seeded cache ile bir kez değerlendir (startup sinyali).
            let cfg_interval = self.config.interval.clone();
            let interval_just_closed = if cfg_interval == "1m" {
                !candles_1m.is_empty() // 1m: her yeni 1m mum değerlendirme tetikler
            } else {
                newly_emitted.iter().any(|c| c.interval == cfg_interval)
            };
            let should_evaluate = interval_just_closed || !evaluated_at_least_once;

            if should_evaluate {
                evaluated_at_least_once = true;
                for market in markets {
                    let symbols = fetcher.supported_symbols(market);
                    for symbol in symbols {
                        self.process_symbol(&symbol, market, &mut ls).await;

                        // ── Scalp & Swing motoru: Regular loop sinyalinden bağımsız ─────
                        if self.config.scalp_swing.is_some() && !ls.stop_loop {
                            let (current_price, main_candles) = {
                                let cc = candle_cache.lock().unwrap();
                                let interval = &self.config.interval;
                                let candles = cc.get_latest(interval, 200);
                                let price = candles.last().map(|c| c.close).unwrap_or(0.0);
                                (price, candles)
                            };
                            if current_price > 0.0 && !main_candles.is_empty() {
                                self.process_scalp_swing(
                                    &symbol, market, current_price,
                                    &main_candles, &mut ls,
                                ).await;
                            }
                        }

                        if ls.stop_loop { break; }
                    }
                    if ls.stop_loop { break; }
                }
            }

            // Orphan SL/TP kontrolü
            self.process_orphans(&mut ls);

            // FIX-3 + FIX-4: Pozisyon snapshot ve WAL checkpoint
            // ls.db_conn döngü boyunca açık — her tick'te yeniden açılmaz (Fix-3).
            if let Some(conn) = &ls.db_conn {
                // FIX-3: Açık pozisyonları her tick sonunda kaydet
                let positions_vec: Vec<&OpenPosition> = ls.open_positions.values().collect();
                if let Ok(json) = serde_json::to_string(&positions_vec) {
                    let _ = crate::database_writer::save_open_positions_snapshot(conn, &json);
                }
                // FIX-4: WAL checkpoint — her 60 tick'te WAL dosyasını ana DB'ye yaz
                // PASSIVE: okuyucuları bloklamaz, meşgulse sessizce atlar.
                // Kapanışta rtc_cli.rs TRUNCATE ile tam flush yapılıyor.
                ls.wal_tick_counter += 1;
                if ls.wal_tick_counter >= 60 {
                    let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
                    ls.wal_tick_counter = 0;
                    // Her 60 tick'te equity snapshot'ı güncelle — restart'ta peak_equity doğru başlasın.
                    save_equity_snapshot(ls.capital, ls.cumulative_pnl, ls.peak_equity);
                }
            }

            // B1/B2/B3 otomatik optimizasyon
            // ─ Her 240 tick'te bir arka plan görevi başlatılır (spawn_blocking).
            // ─ Görev tamamlanınca oneshot kanalı üzerinden sonuç alınır.
            // ─ Bir önceki görev bitmeden yeni görev başlatılmaz (örtüşme yok).
            {
                // Tamamlanan görevi yokla (non-blocking try_recv)
                let opt_done = ls.opt_result_rx.as_mut().and_then(|rx| rx.try_recv().ok());
                if let Some(best) = opt_done {
                    ls.opt_result_rx = None;
                    self.logger.log_info("opt", &format!(
                        "Pos-mgmt opt tamamlandı: be={:?} atr={:?} ptp={:?} score={:.2}",
                        best.breakeven_at_rr, best.atr_trail_mult, best.partial_tp_ratio, best.score
                    ));
                    if let Some(profile_path) = &self.config.robotic_profiles_path.clone() {
                        let _ = write_pos_opt_to_profile(profile_path, &best);
                    }
                }

                // Yeni görev zamanı mı?
                ls.opt_tick_counter += 1;
                if ls.opt_tick_counter >= 240 && ls.opt_result_rx.is_none() {
                    ls.opt_tick_counter = 0;
                    if let Some(_profile_path) = &self.config.robotic_profiles_path {
                        let candles_snap = {
                            let cc = ls.candle_cache.lock().unwrap();
                            cc.get_latest(&self.config.interval, 500)
                        };
                        if candles_snap.len() >= 50 {
                            let base_cfg = crate::robot::backtester::engine::BacktestConfig {
                                symbol:            self.config.symbol.clone(),
                                interval:          self.config.interval.clone(),
                                initial_balance:   10_000.0,
                                max_position_size: 0.01,
                                take_profit_pct:   self.config.risk_params.take_profit_pct,
                                stop_loss_pct:     self.config.risk_params.stop_loss_pct,
                                strategy_name:     "MA_Crossover".to_string(),
                                position_profile:  None,
                                security_profile:  None,
                                commission_pct:    0.001,
                                strategy_params:   None,
                                breakeven_at_rr:   None,
                                atr_trail_mult:    None,
                                partial_tp_ratio:  None,
                            };
                            let (tx, rx) = tokio::sync::oneshot::channel();
                            ls.opt_result_rx = Some(rx);
                            tokio::task::spawn_blocking(move || {
                                if let Some(best) = crate::robot::backtester::engine::Backtester
                                    ::optimize_position_management(&base_cfg, &candles_snap)
                                {
                                    let _ = tx.send(best);
                                }
                            });
                            self.logger.log_info("opt", "Pos-mgmt optimizasyon arka planda başlatıldı");
                        }
                    }
                }
            }

            // Metrikler
            let win_rate = if ls.total_trades > 0 {
                100.0 * ls.win_trades as f64 / ls.total_trades as f64
            } else { 0.0 };
            if ls.total_trades > 0 {
                self.logger.log_info("metrics", &format!(
                    "TotalPnL: {:.2}, WinRate: {:.2}%, MaxDrawdown: {:.2}%",
                    ls.total_pnl, win_rate, 0.0_f64 * 100.0
                ));
            }

            if self.config.autonomous_enabled {
                if ls.stop_loop {
                    let _ = ls.autonomous_controller.transition_failure("loop-stop");
                } else {
                    let _ = ls.autonomous_controller.transition_success();
                }
                self.logger.log_info("autonomous", &format!(
                    "state={} cycle={} failures={} equity={:.2}",
                    ls.autonomous_controller.state,
                    ls.autonomous_controller.cycle_id,
                    ls.autonomous_controller.consecutive_failures,
                    ls.current_equity
                ));
            }

            // Adaptif kalite filtresi
            // NOT: 0 kapanan işlemle tight moda geçmek anlamsız; en az 5 işlem gerekli.
            if self.config.quality.adaptive_enabled {
                let enough_trades = ls.total_trades >= 5;
                if enough_trades && win_rate < self.config.quality.win_rate_low {
                    ls.min_rr             = self.config.quality.min_rr_tight;
                    ls.volatility_max_pct = self.config.quality.volatility_max_tight;
                } else if enough_trades && win_rate > self.config.quality.win_rate_high {
                    ls.min_rr             = self.config.quality.min_rr_loose;
                    ls.volatility_max_pct = self.config.quality.volatility_max_loose;
                } else {
                    ls.min_rr             = self.config.quality.min_rr;
                    ls.volatility_max_pct = self.config.quality.volatility_max_pct;
                }
                ls.volatility_min_pct = self.config.quality.volatility_min_pct;
                self.logger.log_info("quality", &format!(
                    "Kalite esikleri guncel: rr>={:.2}, vol=[{:.2}%, {:.2}%]",
                    ls.min_rr, ls.volatility_min_pct, ls.volatility_max_pct
                ));
            }

            let _ml_anomaly_score = if win_rate < 30.0 { Some(1.0_f64) } else { Some(0.0_f64) };
            if let Some(_monitor_ref) = monitor.as_mut() {
                #[cfg(target_arch = "wasm32")]
                {
                    let action = monitor_ref.check(_ml_anomaly_score);
                    match action {
                        MonitorAction::Continue => {},
                        MonitorAction::Pause => {
                            self.logger.log_error("monitor", "Monitor: PAUSE - Sistem geçici olarak duraklatıldı.");
                            return;
                        },
                        MonitorAction::Restart => {
                            self.logger.log_error("monitor", "Monitor: RESTART - Sistem yeniden başlatılıyor.");
                            return;
                        },
                        MonitorAction::Stop => {
                            self.logger.log_error("monitor", "Monitor: STOP - Sistem tamamen durduruldu.");
                            return;
                        },
                    }
                }
            }

            self.ml_model = ml_model.clone();
            self.ml_data  = Some(ml_data.clone());
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.portfolio = Some(portfolio.clone());
                // Aralıklı sleep — her saniye WS fiyatıyla SL/TP kontrolü.
                // Ana döngü artık her 60 saniyede bir 1m candle çeker;
                // bu sürede SL/TP tetiklenirse sleep hemen kesilir.
                let mut rem = POLL_INTERVAL_SECS;
                while rem > 0 {
                    tokio::time::sleep(Duration::from_secs(1.min(rem))).await;
                    rem = rem.saturating_sub(1);
                    // Aktif sembol SL/TP (WS canlı fiyatı) — tetiklenirse sleep'i kes
                    if self.check_live_sl_tp(&mut ls) { break; }
                    // Orphan pozisyon SL/TP
                    self.process_orphans(&mut ls);
                    if ls.stop_loop { break; }
                }
            }
            self.monitor = monitor.clone();

            if ls.stop_loop { break; }
        }

        // ── WS akışını durdur ─────────────────────────────────────────────────
        ws_stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = _ws_price_handle { h.abort(); }
    }

    // ============ PROFILE MANAGEMENT ============
    
    /// Runtime'da pozisyon profili değiştir
    pub fn set_position_profile(&mut self, profile_name: &str) {
        self.config.position_profile = Some(profile_name.to_string());
        self.logger.log_info(
            "config", 
            &format!("Position profile changed to: {}", profile_name)
        );
    }
    
    /// Runtime'da güvenlik profili değiştir
    pub fn set_security_profile(&mut self, profile_name: &str) {
        self.config.security_profile = Some(profile_name.to_string());
        self.logger.log_info(
            "config",
            &format!("Security profile changed to: {}", profile_name)
        );
    }
    
    /// Mevcut profil ayarlarını al
    pub fn get_profiles(&self) -> (Option<String>, Option<String>) {
        (
            self.config.position_profile.clone(),
            self.config.security_profile.clone()
        )
    }
    
    /// Profil bazlı pozisyon config'lerini parse et
    /// Returns: (TrailingStopConfig, ScaleInConfig, ScaleOutConfig) veya None
    #[cfg(not(target_arch = "wasm32"))]
    pub fn parse_position_profile(&self) -> Option<(
        crate::robot::portfolio_manager::TrailingStopConfig,
        crate::robot::portfolio_manager::ScaleInConfig,
        crate::robot::portfolio_manager::ScaleOutConfig,
    )> {
        use crate::robot::config_helpers::PositionManagementProfile;
        
        let profile_str = self.config.position_profile.as_ref()?;
        let profile = match profile_str.as_str() {
            "Conservative" => PositionManagementProfile::Conservative,
            "Balanced" => PositionManagementProfile::Balanced,
            "Aggressive" => PositionManagementProfile::Aggressive,
            "Scalper" => PositionManagementProfile::Scalper,
            "SwingTrading" => PositionManagementProfile::SwingTrading,
            _ => {
                self.logger.log_error("config", &format!("Unknown position profile: {}", profile_str));
                return None;
            }
        };
        
        Some((
            profile.trailing_stop_config(),
            profile.scale_in_config(),
            profile.scale_out_config(),
        ))
    }
}

// Trade quality ayarlarini diskten oku (varsa)
fn load_trade_quality_config_from_file(path: &str) -> Option<TradeQualityConfig> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// robotic_profiles.json'dan pozisyon yönetimi parametrelerini oku.
/// Döndürür: (sl_cooldown_secs, breakeven_at_rr, atr_trail_mult, partial_tp_ratio)
fn load_robotic_profiles(path: &str) -> Option<(Option<u64>, Option<f64>, Option<f64>, Option<f64>)> {
    #[derive(serde::Deserialize)]
    struct P {
        #[serde(default)] sl_cooldown_secs: Option<u64>,
        #[serde(default)] breakeven_at_rr:  Option<f64>,
        #[serde(default)] atr_trail_mult:   Option<f64>,
        #[serde(default)] partial_tp_ratio: Option<f64>,
    }
    let content = fs::read_to_string(path).ok()?;
    let p: P = serde_json::from_str(&content).ok()?;
    Some((p.sl_cooldown_secs, p.breakeven_at_rr, p.atr_trail_mult, p.partial_tp_ratio))
}

/// Optimizasyon sonucunu robotic_profiles.json'a yaz.
/// Mevcut dosyayı okur, yalnızca B1/B2/B3 alanlarını günceller, diğer alanları korur.
fn write_pos_opt_to_profile(
    path: &str,
    best: &crate::robot::backtester::engine::PosOptResult,
) -> std::io::Result<()> {
    // Mevcut JSON'u oku (yoksa boş obje)
    let content = fs::read_to_string(path).unwrap_or_else(|_| "{}".to_string());
    let mut map: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&content).unwrap_or_default();
    // B1/B2/B3 alanlarını güncelle
    map.insert("breakeven_at_rr".to_string(),
        best.breakeven_at_rr.map(serde_json::Value::from).unwrap_or(serde_json::Value::Null));
    map.insert("atr_trail_mult".to_string(),
        best.atr_trail_mult.map(serde_json::Value::from).unwrap_or(serde_json::Value::Null));
    map.insert("partial_tp_ratio".to_string(),
        best.partial_tp_ratio.map(serde_json::Value::from).unwrap_or(serde_json::Value::Null));
    let json = serde_json::to_string_pretty(&serde_json::Value::Object(map))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    fs::write(path, json)
}

// TrendBias, trend_bias(), average_range_pct() → robot/signal_evaluator.rs'e taşındı

// ── MTF (Multi-Timeframe) yardımcı fonksiyonlar ──────────────────────────────

/// İşlem aralığından bir üst zaman dilimine eşleme.
///
/// | İşlem TF | HTF    |
/// |----------|--------|
/// | 1m / 5m  | 1h     |
/// | 15m / 30m| 4h     |
/// | 1h       | 4h     |
/// | 4h / 1d  | 1d     |
///
/// Zaten en üst TF ise kendi değerini döndürür (caller None geçer).
pub fn htf_for_interval(interval: &str) -> &'static str {
    match interval {
        "1m" | "5m"        => "1h",
        "15m" | "30m"      => "4h",
        "1h"               => "4h",
        "4h" | "1d" | _    => "1d",
    }
}

/// SQLite DB'den HTF candle'ları yükle.
///
/// `market` → "spot" | "futures" | "coinm"
/// "1m" → 60, "5m" → 300, "1h" → 3600, "4h" → 14400, "1d" → 86400 gibi interval string'ini saniyeye çevirir.
pub fn parse_interval_secs(interval: &str) -> u64 {
    let s = interval.trim().to_lowercase();
    if let Some(n) = s.strip_suffix('m') {
        n.parse::<u64>().unwrap_or(1) * 60
    } else if let Some(n) = s.strip_suffix('h') {
        n.parse::<u64>().unwrap_or(1) * 3600
    } else if let Some(n) = s.strip_suffix('d') {
        n.parse::<u64>().unwrap_or(1) * 86400
    } else {
        60
    }
}

/// Candle'lar timestamp ASC sıralı döner (en eski → en yeni).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_htf_candles_from_db(
    db_path:  &str,
    symbol:   &str,
    market:   &str,
    interval: &str,
    limit:    usize,
) -> crate::Result<Vec<crate::types::Candle>> {
    load_htf_candles_from_db_with_exchange(db_path, symbol, "binance", market, interval, limit)
}

/// Exchange-specific tabloyu (candles_{exchange}_{market}) önce dener;
/// yetersiz veri varsa legacy `candles` tablosuna düşer.
pub fn load_htf_candles_from_db_with_exchange(
    db_path:  &str,
    symbol:   &str,
    exchange: &str,
    market:   &str,
    interval: &str,
    limit:    usize,
) -> crate::Result<Vec<crate::types::Candle>> {
    use rusqlite::Connection;

    // Whitelist — SQL injection koruması (tablo adı doğrudan sorguya gömülür)
    const VALID_EXCHANGES: &[&str] = &["binance", "bist", "bybit", "kucoin", "coinbase"];
    const VALID_MARKETS:   &[&str] = &["spot", "futures", "coinm", "margin"];

    let conn = Connection::open(db_path)?;

    /// Tek bir satırı Candle'a çevirir; timestamp saniyeyse ms'e normalize eder.
    fn parse_row(row: &rusqlite::Row<'_>, symbol: &str, interval: &str) -> rusqlite::Result<crate::types::Candle> {
        use chrono::{TimeZone, Utc};
        let ts_ms: i64 = row.get(0)?;
        let ts_ms_norm = if ts_ms < 9_999_999_999 { ts_ms * 1000 } else { ts_ms };
        Ok(crate::types::Candle {
            timestamp: Utc.timestamp_millis_opt(ts_ms_norm).single()
                .unwrap_or_else(|| Utc::now()),
            open:     row.get(1)?,
            high:     row.get(2)?,
            low:      row.get(3)?,
            close:    row.get(4)?,
            volume:   row.get(5)?,
            symbol:   symbol.to_string(),
            interval: interval.to_string(),
        })
    }

    // ── 1. Adım: exchange-specific tablo (candles_binance_spot vb.) ─────────
    if VALID_EXCHANGES.contains(&exchange) && VALID_MARKETS.contains(&market) {
        let table = format!("candles_{}_{}", exchange, market);
        let sql = format!(
            "SELECT timestamp, open, high, low, close, volume \
             FROM {} WHERE symbol=?1 AND interval=?2 \
             ORDER BY timestamp DESC LIMIT ?3",
            table
        );
        if let Ok(mut stmt) = conn.prepare(&sql) {
            let sym = symbol.to_string();
            let intv = interval.to_string();
            let rows: Vec<crate::types::Candle> = stmt
                .query_map(rusqlite::params![symbol, interval, limit as i64], |row| {
                    parse_row(row, &sym, &intv)
                })?
                .filter_map(|r| r.ok())
                .collect();
            if rows.len() >= limit / 2 {
                let mut out = rows;
                out.reverse(); // DESC → ASC
                return Ok(out);
            }
            log::warn!(
                "load_htf_candles: {}.{} {} için {} satır (limit={}) — legacy candles'a düşülüyor",
                exchange, market, interval, rows.len(), limit
            );
        }
    }

    // ── 2. Adım: legacy `candles` tablosu (fallback) ────────────────────────
    // Legacy tablo market sütunu barındırır; filtrele (sütun yoksa hata sessizce yutulur).
    let sym = symbol.to_string();
    let intv = interval.to_string();
    let mut stmt = conn.prepare(
        "SELECT timestamp, open, high, low, close, volume \
         FROM candles \
         WHERE symbol=?1 AND interval=?2 AND (market=?3 OR market IS NULL) \
         ORDER BY timestamp DESC LIMIT ?4",
    )?;

    let mut rows: Vec<crate::types::Candle> = stmt.query_map(
        rusqlite::params![symbol, interval, market, limit as i64],
        |row| parse_row(row, &sym, &intv),
    )?
    .filter_map(|r| r.ok())
    .collect();

    rows.reverse(); // DESC → ASC (en eski ilk)
    Ok(rows)
}

// ── Dinamik Kaldıraç Hesabı ───────────────────────────────────────────────────
/// 7x–10x aralığında otonom kaldıraç belirler.
///
/// Artırma koşulları:
///   - HTF trend sinyalle aynı yönde → +1.0x (güçlü onay)
///   - HyperOpt skoru > 0.70 → +0.5x (kaliteli sinyal)
///
/// Azaltma koşulları:
///   - ATR% > 2.5 → -2.0x (yüksek volatilite riski)
///   - ATR% > 1.5 → -1.0x (orta volatilite)
///   - Drawdown > %10 → taban değerine sıfırla (koruma modu)
///   - Drawdown > %5  → -1.0x (dikkatli mod)
///
/// Sonuç her zaman [base, max] aralığına klamp edilir.
#[cfg(not(target_arch = "wasm32"))]
pub fn compute_effective_leverage(
    base:           f64,
    max:            f64,
    market:         Market,    // Spot'ta kaldıraç her zaman 1x
    htf_bias:       Option<TrendBias>,
    signal:         &Signal,
    atr_pct:        Option<f64>,
    dd_pct:         f64,
    hyperopt_score: f64,
    session_rr:     f64,   // ort_kazanç / ort_kayıp; 1.0 = nötr/yeterli veri yok
    loss_streak:    usize, // ardışık zarar sayısı
    open_count:     usize, // anlık açık pozisyon sayısı
) -> f64 {
    // Spot piyasada kaldıraç uygulanamaz
    if matches!(market, Market::Spot) {
        return 1.0;
    }

    let mut lev = base;

    // HTF trend hizalaması — teyit var → güven artar
    let htf_boost = match (htf_bias, signal) {
        (Some(TrendBias::Bullish), Signal::Buy)  => 1.0,
        (Some(TrendBias::Bearish), Signal::Sell) => 1.0,
        _ => 0.0,
    };
    lev += htf_boost;

    // Yüksek volatilite — risk büyür, kaldıraç düşer
    if let Some(atr) = atr_pct {
        if      atr > 2.5 { lev -= 2.0; }
        else if atr > 1.5 { lev -= 1.0; }
    }

    // Drawdown koruma — kayıp büyüdükçe kaldıraç azalır
    if      dd_pct > 10.0 { lev = base; }  // koruma moduna gir
    else if dd_pct >  5.0 { lev -= 1.0; }

    // Güçlü sinyal — küçük artış
    if hyperopt_score > 0.70 { lev += 0.5; }

    // Risk/Reward oranı — RR > 2.0 iyi strateji, < 1.0 kötü strateji
    // session_rr == 1.0 ise henüz yeterli veri yok → nötr kal
    if session_rr > 2.0      { lev += 0.5; }  // çok iyi RR → güven artar
    else if session_rr < 0.8 { lev -= 1.0; }  // kötü RR → riski azalt

    // Ardışık zarar koruması — peş peşe zararlar sisteme güveni düşürür
    if      loss_streak >= 5 { lev = base; }   // ciddi koruma moduna geç
    else if loss_streak >= 3 { lev -= 1.5; }
    else if loss_streak >= 2 { lev -= 0.5; }

    // Çok fazla açık pozisyon — risk konsantrasyonu artar, kaldıraç düşür
    if      open_count >= 5 { lev -= 2.0; }
    else if open_count >= 3 { lev -= 1.0; }

    lev.clamp(base, max)
}

// ── Equity Snapshot ──────────────────────────────────────────────────────────
// Restart sonrası peak_equity ve cumulative_pnl'nin korunması için minimal JSON.
// Aynı capital değerindeyse yüklenir; farklı capital → cold start (discard edilir).

const EQUITY_SNAPSHOT_PATH: &str = "config/equity_snapshot.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct EquitySnapshot {
    capital:         f64,
    cumulative_pnl:  f64,
    peak_equity:     f64,
}

fn save_equity_snapshot(capital: f64, cumulative_pnl: f64, peak_equity: f64) {
    let snap = EquitySnapshot { capital, cumulative_pnl, peak_equity };
    if let Ok(json) = serde_json::to_string_pretty(&snap) {
        let _ = std::fs::create_dir_all("config");
        let _ = std::fs::write(EQUITY_SNAPSHOT_PATH, json);
    }
}

/// Önceki session'dan kaydedilen `(cumulative_pnl, peak_equity)` döner.
/// Capital farklıysa veya dosya yoksa `(0.0, capital)` döner.
fn load_equity_snapshot(capital: f64) -> (f64, f64) {
    let content = match std::fs::read_to_string(EQUITY_SNAPSHOT_PATH) {
        Ok(c) => c,
        Err(_) => return (0.0, capital),
    };
    match serde_json::from_str::<EquitySnapshot>(&content) {
        Ok(snap) if (snap.capital - capital).abs() < 0.01 => {
            (snap.cumulative_pnl, snap.peak_equity.max(capital))
        }
        _ => (0.0, capital),
    }
}
