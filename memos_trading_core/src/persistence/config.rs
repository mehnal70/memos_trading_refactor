//! Config facade — rtc_cli'a özel tüm konfigürasyon struct'ları.
//!
//! ## İçerik
//!
//! - [`OptimizedParamsCache`] — Hyperopt/backtest sonuçlarının kalıcı önbelleği
//! - [`SessionFilterConfig`] — Saat bazlı işlem filtresi (long/short eğilim)
//! - [`OtoConfig`] — Ana otonom yapılandırma (rtc_config.json)
//! - [`ProfileConfig`] — Pozisyon yönetimi profil parametreleri (robotic_profiles.json)
//!
//! Tüm struct'lar serde uyumlu; load/save fonksiyonları da burada toplandı.
//! `default_*` helper'ları serde `#[serde(default = "...")]` için kullanılır.

use crate::robot::robotic_loop::AppState;

// ─── Optimize Edilmiş Parametre Önbelleği ────────────────────────────────────
//
// Hyperopt/backtest'in bulduğu en iyi parametreler — JSON'a persist edilir,
// restart'ta kalır. Her ML döngüsü güncelleyebilir.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OptimizedParamsCache {
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

// ─── Saat Bazlı İşlem Filtresi ───────────────────────────────────────────────
//
// DB analizine göre: 10-12 arası long için en verimli, 08:00 kısa-yön eğilimli,
// 17-18 yüksek vol ama yönsüz.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionFilterConfig {
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

// ─── Ana Otonom Yapılandırma ─────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OtoConfig {
    pub exchange:               String,
    pub market:                 String,
    pub symbol:                 String,
    pub interval:               String,
    pub db_path:                String,
    pub capital:                f64,
    pub backtest_enabled:       bool,
    pub backtest_every_mins:    u64,
    pub backtest_candle_limit:  usize,
    pub trade_amount:           f64,
    // Otonom veri indirme
    #[serde(default = "default_true")]
    pub download_enabled:       bool,
    #[serde(default = "default_download_mins")]
    pub download_every_mins:    u64,    // kaç dakikada bir indir
    #[serde(default = "default_download_limit")]
    pub download_candle_limit:  usize,  // her indirmede kaç mum
    #[serde(default = "default_download_top_n")]
    pub download_top_n:         usize,  // En iyi N sembol için de indir
    #[serde(default = "default_export_mins")]
    pub auto_export_every_mins: u64,    // otomatik export aralığı (0 = devre dışı)
    #[serde(default = "default_export_keep")]
    pub auto_export_keep:       usize,  // saklanacak maksimum export dosyası sayısı
    // ── Yol yapılandırması ────────────────────────────────────────────────────
    #[serde(default = "default_trade_quality_path")]
    pub trade_quality_config_path: String,
    #[serde(default = "default_adaptive_params_path")]
    pub adaptive_params_path:      String,
    #[serde(default = "default_robotic_profiles_path")]
    pub robotic_profiles_path:     String,
    #[serde(default = "default_evolution_state_path")]
    pub evolution_state_path:      String,
    #[serde(default = "default_fsm_state_path")]
    pub fsm_state_path:            String,
    #[serde(default = "default_app_snapshot_path")]
    pub app_snapshot_path:         String,
    // ── Kaldıraç aralığı ─────────────────────────────────────────────────────
    #[serde(default = "default_leverage_base")]
    pub leverage_base:             f64,   // Minimum kaldıraç (varsayılan: 7x)
    #[serde(default = "default_leverage_max")]
    pub leverage_max:              f64,   // Maksimum kaldıraç (varsayılan: 10x)
    // ── Optimize edilmiş parametre önbelleği (persist) ────────────────────────
    #[serde(default)]
    pub optimized_params:          OptimizedParamsCache,
    // ── Seans/saat filtresi ───────────────────────────────────────────────────
    #[serde(default)]
    pub session_filter:            SessionFilterConfig,
    // ── Kalıcı sembol engelleme listesi ──────────────────────────────────────
    // Bu listeki semboller kesinlikle işlem açılmaz (ör: sürekli zararlı semboller).
    // Örn: ["ETHUSDT", "XRPUSDT"]
    #[serde(default)]
    pub blocked_symbols:           Vec<String>,
    // ── Sabitlenmiş (pinned) sembol listesi ──────────────────────────────────
    // Bu semboller skor/filtre sonucundan bağımsız olarak:
    //   • MTF scanner'a her zaman dahil edilir
    //   • Orchestrator worker top-N listesine her zaman eklenir (capacity varsa)
    //   • blocked_symbols içinde olmadıkları sürece her zaman izlenir
    // Örn: ["BTCUSDT", "ETHUSDT"]
    #[serde(default)]
    pub pinned_symbols:            Vec<String>,
    // ── Otonom Pipeline (D→B→ML→P5) ─────────────────────────────────────────
    #[serde(default = "default_true")]
    pub pipeline_enabled:          bool,     // false = tamamen devre dışı
    #[serde(default = "default_pipeline_mins")]
    pub pipeline_every_mins:       u64,      // periyodik tekrar aralığı (dk)
    #[serde(default = "default_pipeline_p5_top_n")]
    pub pipeline_p5_top_n:         usize,    // kaç sembol için p5 analizi çalıştır
    // ── Interval / HTF Filtre kalıcılığı ─────────────────────────────────────
    #[serde(default)]
    pub auto_interval:             bool,     // otomatik interval geçişi — settings item 10
}

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

// ─── Pozisyon Profili Yapılandırması ─────────────────────────────────────────
//
// robotic_profiles.json'dan okunan pozisyon yönetimi parametreleri.
// Tüm alanlar opsiyonel — dosyada bulunmayan alanlar varsayılan None alır.

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
pub struct ProfileConfig {
    #[serde(default)]
    pub position_profile: String,
    #[serde(default)]
    pub security_profile: String,
    #[serde(default)]
    pub sl_cooldown_secs: Option<u64>,
    #[serde(default)]
    pub breakeven_at_rr: Option<f64>,
    #[serde(default)]
    pub atr_trail_mult: Option<f64>,
    #[serde(default)]
    pub partial_tp_ratio: Option<f64>,
}

pub fn load_profile_config(path: &str) -> ProfileConfig {
    std::fs::read_to_string(path).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_profile_config(path: &str, st: &AppState) {
    // Pozisyon yönetimi parametreleri brain.best_params HashMap'inden okunur.
    let bp = &st.brain.best_params;
    let prof = ProfileConfig {
        position_profile: String::new(),
        security_profile: String::new(),
        sl_cooldown_secs: bp.get("pos_sl_cooldown").map(|v| *v as u64),
        breakeven_at_rr:  bp.get("pos_breakeven_at_rr").copied(),
        atr_trail_mult:   bp.get("pos_atr_trail_mult").copied(),
        partial_tp_ratio: bp.get("pos_partial_tp_ratio").copied(),
    };
    if let Ok(s) = serde_json::to_string_pretty(&prof) {
        let _ = std::fs::write(path, s);
    }
}

// ─── Serde Default Helper Fonksiyonları ──────────────────────────────────────
//
// Bu `pub fn`'ler `#[serde(default = "default_xxx")]` attribute'larından çağrılır.
// `pub` olmaları sadece serde macro'su için gerekli; dış API değil.

pub fn default_true()                  -> bool   { true }
pub fn default_download_mins()         -> u64    { 15 }
pub fn default_download_limit()        -> usize  { 500 }
pub fn default_download_top_n()        -> usize  { 3 }
pub fn default_export_mins()           -> u64    { 30 }
pub fn default_export_keep()           -> usize  { 24 }
pub fn default_trade_quality_path()    -> String { "config/trade_quality.json".into() }
pub fn default_adaptive_params_path()  -> String { "config/adaptive_params.json".into() }
pub fn default_robotic_profiles_path() -> String { "config/robotic_profiles.json".into() }
pub fn default_evolution_state_path()  -> String { "config/evolution_state.json".into() }
pub fn default_fsm_state_path()        -> String { "config/fsm_state.json".into() }
pub fn default_app_snapshot_path()     -> String { "config/app_snapshot.json".into() }
pub fn default_leverage_base()         -> f64    { 7.0 }
pub fn default_leverage_max()          -> f64    { 10.0 }
pub fn default_screener_min_vol()      -> f64    { 5.0 }
pub fn default_screener_min_chg()      -> f64    { 2.0 }
pub fn default_screener_max_new()      -> usize  { 8 }
pub fn default_screener_interval_hours() -> f64 { 4.0 }
pub fn default_pipeline_mins()           -> u64  { 120 }  // 2 saatte bir
pub fn default_pipeline_p5_top_n()       -> usize { 3 }   // en iyi 3 sembol
