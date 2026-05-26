// robot/parameters/mod.rs — Dinamik parametre store'u.
//
// Faz 2 hedefi: sabit eşikleri (örn. `EDGE_THRESHOLD`, slippage limitleri,
// scalp/swing eşiği) tek bir merkezi store'dan beslemek. HyperOpt + IntelligenceHub
// bu store'a yazar, engine her cycle'da okur. Bu sayede:
//   - Sabit değerler runtime'da güncellenebilir (rejim/öğrenme akışıyla).
//   - HyperOpt sonuçları otomatik propagation.
//   - Test edilebilirlik artar (env yerine direct construct).
//
// Bu commit (c1) iskelet: edge_threshold katmanlarını taşıyor. Sonraki commit'lerde
// (c2) daha çok parametre, (c3) HyperOpt yazımı, (c4) rejim-bazlı katmanlama.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub mod adaptive;
pub mod symbol_stats;
pub mod trail_feedback;

pub use symbol_stats::{SymbolStats, compute_symbol_stats};
pub use trail_feedback::{TrailFeedback, PendingTrailObservation};

/// Rejim-bazlı parametre patch'i. Yalnızca override edilmek istenen alanlar `Some`
/// olur; diğerleri base ParameterStore değerlerini korur (sparse override).
/// Key olarak `MarketRegime::as_str()` çıktısı kullanılır
/// ("Ranging", "StrongUptrend", "HighVolatility", ...).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegimePatch {
    #[serde(default)]
    pub edge_thresholds: Option<EdgeThresholds>,
    #[serde(default)]
    pub trade_risk: Option<TradeRiskParams>,
}

impl RegimePatch {
    pub fn empty() -> Self { Self::default() }

    pub fn with_edge(mut self, e: EdgeThresholds) -> Self {
        self.edge_thresholds = Some(e);
        self
    }

    pub fn with_trade_risk(mut self, t: TradeRiskParams) -> Self {
        self.trade_risk = Some(t);
        self
    }

    /// Patch hiçbir alanı override etmiyor mu? Engine boş patch'leri store'a
    /// koymaktan kaçınmak için bunu kontrol eder.
    pub fn is_empty(&self) -> bool {
        self.edge_thresholds.is_none() && self.trade_risk.is_none()
    }
}

/// Rejim-bazlı trade feedback kuyruğu. Faz 3 c2: her kapanış pnl_pct'sini ilgili
/// rejim için kayıt altına alır; düşük win_rate görüldüğünde patch otomatik
/// sıkılaştırılır.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegimeFeedback {
    /// Son N kapanışın pnl_pct değerleri (en yeni sonda).
    #[serde(default)]
    pub recent_pnl: std::collections::VecDeque<f64>,
    /// Toplam kayıt sayısı (kuyruk dışı sayım da dahil — rapor için).
    #[serde(default)]
    pub total_trades: u32,
}

impl RegimeFeedback {
    /// Kuyrukta tutulan son trade sayısı. Çok küçük olursa istatistik gürültülü,
    /// çok büyük olursa eski rejim sinyallerini geç bırakır.
    pub const WINDOW: usize = 10;

    /// Win-rate (0.0..=1.0). Kuyruk boşsa 0 döner.
    pub fn win_rate(&self) -> f64 {
        if self.recent_pnl.is_empty() { return 0.0; }
        let wins = self.recent_pnl.iter().filter(|&&p| p > 0.0).count();
        wins as f64 / self.recent_pnl.len() as f64
    }

    /// Yeni bir trade pnl kaydını kuyruğa ekler; WINDOW'u aşan eski kayıtları atar.
    pub fn record(&mut self, pnl_pct: f64) {
        self.recent_pnl.push_back(pnl_pct);
        while self.recent_pnl.len() > Self::WINDOW {
            self.recent_pnl.pop_front();
        }
        self.total_trades = self.total_trades.saturating_add(1);
    }
}

/// Trade-bazlı risk parametreleri. HyperOpt + ML retrain job'larının çıktısı buraya
/// yazılır; engine pozisyon açılışta bu store'dan okur (best_params HashMap fallback).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TradeRiskParams {
    /// Take-profit yüzdesi (entry'den uzaklık).
    pub take_profit_pct: f64,
    /// Stop-loss yüzdesi.
    pub stop_loss_pct: f64,
    /// Equity'nin tek pozisyona ayrılabilecek maksimum payı (0..1, örn 0.5 = %50).
    pub max_position_size: f64,
}

impl Default for TradeRiskParams {
    fn default() -> Self {
        Self {
            take_profit_pct:   3.0,
            stop_loss_pct:     1.5,
            max_position_size: 0.5,
        }
    }
}

/// Otonom Leverage katmanı parametreleri (futures pozisyon açılışları için).
/// `enabled=false` (default) → davranış legacy: open_paper_position lev=1.0
/// kullanmaya devam eder (spot). True ise `resolve_leverage` çağrısı
/// rejim + ML confidence + win rate + noise floor karışımıyla [1.0, max]
/// arasında bir değer üretir.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LeverageParams {
    /// Master kapı. False → tüm pozisyonlar lev=1.0 (spot davranış).
    pub enabled: bool,
    /// Modülasyon başlangıç noktası. Rejim/conf/win_rate çarpanları bunun
    /// üzerine binip [1.0, max]'a clamp edilir.
    pub base: f64,
    /// Sert üst sınır — risk filter ve clamp burayı geçmez.
    pub max: f64,
    /// ML confidence eşiği — bu değer ve üstünde lev *= 1.2 boost.
    pub conf_boost_threshold: f64,
    /// SymbolStats.noise_floor_pct (median ATR%) bu değerin üstündeyse
    /// lev *= 0.7 (yüksek volatilitede pozisyon küçült).
    pub vol_floor_pct: f64,
}

impl Default for LeverageParams {
    fn default() -> Self {
        Self {
            // Otonom davranış (multi-TF ve ScalpSwing gibi). Formül rejime göre
            // 0.5x-1.5x arası modüle eder; HighVolatility'de küçültür, conf+wr
            // yüksekse büyütür. Risk: kazanç+kayıp lev katı. Kapatmak için
            // `LEVERAGE_ENABLED=0` env. Manuel override: base/max env'leri.
            enabled: true,
            base: 3.0,
            max: 10.0,           // core/model.rs default_leverage_max ile aynı
            conf_boost_threshold: 0.70,
            vol_floor_pct: 1.0,  // %1 median ATR — bunun üstü "yüksek wiggle"
        }
    }
}

/// Multi-TF (Faz B) parametreleri. Engine cycle StrategyEval öncesi
/// `load_htf_candles` çağrısını ve run_download_job HTF fetch'ini kontrol eder.
/// `enabled=false` → davranış legacy single-TF ile aynı (htf_slice=None,
/// htf_trend_filter pass-through).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MultiTfParams {
    /// Master kapı. False ise loader çağrılmaz, generate_signal `None` htf alır.
    pub enabled: bool,
    /// Loader minimum mum eşiği. Daha az gelirse htf=None (filtre pass-through).
    pub min_required: usize,
    /// run_download_job HTF interval'i de indirsin mi.
    /// False ise base interval yeterli sayılır (cycle 1m fallback'a yaslanır).
    pub download_htf: bool,
}

impl Default for MultiTfParams {
    fn default() -> Self {
        Self {
            enabled: true,
            min_required: 30, // htf_trend_filter slow SMA = 30 → bu altında zaten guard
            download_htf: true,
        }
    }
}

/// Partial fill anomali tespiti eşikleri (master.rs::detect_partial_fill_anomalies).
/// Overfill ve cum-tutarsızlık için rounding payı + adverse slipaj limiti.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PartialFillParams {
    /// last_qty > local_qty * (1 + overfill_tolerance) → bot↔borsa qty ayrışması.
    pub overfill_tolerance: f64,
    /// cum_qty > orig_qty * (1 + cum_tolerance) → borsa payload tutarsız.
    pub cum_tolerance: f64,
    /// Bot tarafına göre adverse fiyat sapması yüzdesi; aşılırsa anomaly emit.
    pub max_slippage_pct: f64,
}

impl Default for PartialFillParams {
    fn default() -> Self {
        Self {
            overfill_tolerance: 0.001,
            cum_tolerance:      0.001,
            max_slippage_pct:   1.0,
        }
    }
}

/// Sembol/strateji bazlı edge skor eşikleri. ML modelinin güvenine göre üç katmanlı.
/// `dynamic_edge_threshold` mantığı buradan akıyor.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EdgeThresholds {
    /// ML henüz hazır değil (confidence < cold_until): gevşek eşik, momentum baskın.
    pub cold: f64,
    /// ML kısmen hazır (cold_until <= confidence < warm_until): orta eşik.
    pub warm: f64,
    /// ML yetkin (confidence >= warm_until): katı eşik.
    pub hot: f64,
    /// Cold→Warm geçiş eşiği (ml_confidence).
    pub cold_until: f64,
    /// Warm→Hot geçiş eşiği (ml_confidence).
    pub warm_until: f64,
}

impl Default for EdgeThresholds {
    fn default() -> Self {
        Self {
            cold: 0.20,
            warm: 0.35,
            hot:  0.55,
            cold_until: 0.05,
            warm_until: 0.30,
        }
    }
}

impl EdgeThresholds {
    /// ML confidence'a göre ilgili katmanın eşiğini döner.
    pub fn for_confidence(&self, ml_confidence: f64) -> f64 {
        if ml_confidence < self.cold_until { self.cold }
        else if ml_confidence < self.warm_until { self.warm }
        else { self.hot }
    }
}

/// Tüm dinamik parametrelerin merkezi store'u. Faz 2 boyunca alan eklenecek;
/// her yeni alan `Default` ve gerekirse `from_env` desteği taşımalı.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterStore {
    pub edge_thresholds: EdgeThresholds,
    pub partial_fill:    PartialFillParams,
    pub trade_risk:      TradeRiskParams,
    /// Scalp/Swing ayrımı eşiği (dakika). Holding < bu eşik → SCALP, üstü → SWING.
    pub scalp_swing_threshold_min: i64,
    /// Periyodik S/R updater task'ının yenileme aralığı (saniye).
    pub sr_update_every_secs: u64,
    /// Rejim-bazlı sparse override'lar. Key `MarketRegime::as_str()` döndürüsüdür.
    /// Boş ise tüm rejimler base parametreleri kullanır.
    #[serde(default)]
    pub regime_overrides: HashMap<String, RegimePatch>,
    /// Rejim başına son N kapanış pnl_pct geri beslemesi (Faz 3 c2).
    /// `apply_trade_feedback` bunu okuyup patch'i rafine eder.
    #[serde(default)]
    pub regime_feedback: HashMap<String, RegimeFeedback>,
    /// Faz 3 c3: en son cycle'da gözlenen (confirmed) rejim. `observe_regime`
    /// drift tespiti yapıp değişimde patch'i otomatik sıkılaştırır.
    #[serde(default)]
    pub last_observed_regime: Option<String>,
    /// Hysteresis için aday rejim: anlık sallanmaları drift saymadan önce
    /// `DRIFT_CONFIRMATION_TURNS` kez üst üste görülmesi gerekir.
    /// (regime, ardışık sayım). Confirmed olunca None.
    #[serde(default)]
    pub pending_regime: Option<(String, u32)>,
    /// Cooldown'ı sürdürmek için son confirmed drift'in epoch saniyesi.
    /// 0 = hiç drift yok. Aynı 60 sn içinde tekrar drift tetiklenmez.
    #[serde(default)]
    pub last_drift_at_secs: u64,
    /// Sembol+interval bazlı gürültü/volatilite istatistikleri.
    /// `perform_download` her başarılı sembolden sonra günceller; `resolve_atr_mult`
    /// burayı tüketir. Boş ise tüm çağrılar rejim/base fallback'e düşer.
    /// Key = (symbol, interval). TTL: default 6 saat (`resolve_atr_mult` içinde
    /// `is_fresh` ile kontrol; stale ise fallback).
    #[serde(default)]
    pub symbol_stats: HashMap<(String, String), SymbolStats>,
    /// Strateji bazlı trail target yüzdesi: SUPERTREND trend takipçisi geniş trail,
    /// BB mean-reversion sıkı trail vb. `resolve_atr_mult` strategy_name ile bunu okur.
    /// HyperOpt / feedback loop bu map'i runtime'da güncelleyebilir.
    /// Env override: TARGET_TRAIL_PCT set ise tüm stratejiler için onurlanır
    /// (operatör manuel müdahale veya A/B test).
    #[serde(default = "default_strategy_trail_targets")]
    pub strategy_trail_targets: HashMap<String, f64>,
    /// Per-(sembol, strateji) trailing outcome feedback'i (Phase C).
    /// TRAILING_STOP kapanışları 60sn sonra evalue edilir; early-exit oranı yüksekse
    /// hedef genişler, düşükse daralır. `target_override` Phase B default'unu by-pass eder.
    #[serde(default)]
    pub trail_feedback: HashMap<(String, String), TrailFeedback>,
    /// Multi-TF (Faz B) parametreleri. Engine cycle ve download path bu alanı okur.
    #[serde(default)]
    pub multi_tf: MultiTfParams,
    /// Otonom leverage katmanı (futures). Default disabled → lev=1.0.
    #[serde(default)]
    pub leverage: LeverageParams,
    /// Strateji bazlı en iyi YAPISAL parametreler (indikatör periyot/eşikleri),
    /// backtest job'ın `param_spec` araması ile bulduğu set. Key = kanonik strateji
    /// adı (best_params gibi global; backtest tek sembol→global uygular). Canlı
    /// cycle `resolve_strategy_params` ile okur, yoksa `StrategyParams::default()`.
    #[serde(default)]
    pub strategy_params: HashMap<String, crate::core::types::StrategyParams>,
}

/// Strateji niyetiyle hizalı default trail target'lar (yüzde, entry'den uzaklık).
/// Tablo: registry kanonik isimler + IDLE_PROTECT için ölçeklendirme.
fn default_strategy_trail_targets() -> HashMap<String, f64> {
    let mut m = HashMap::new();
    // Trend takipçileri — geniş trail, küçük dalgalanmalarda çıkmaz, trend süresince ride.
    m.insert("SUPERTREND".to_string(),     1.2);
    m.insert("MA_CROSSOVER".to_string(),   1.5);
    m.insert("EMA".to_string(),            1.2);
    m.insert("MACD".to_string(),           1.5);
    m.insert("DONCHIAN".to_string(),       1.5);
    m.insert("ADX".to_string(),            1.3);
    m.insert("VWAP".to_string(),           1.2);
    // Mean-reversion — sıkı trail, hızlı kâr al/çık (BB mean'e dön mantığı).
    m.insert("BB".to_string(),             0.5);
    // Momentum osilatörleri — orta sıkılık.
    m.insert("RSI".to_string(),            0.7);
    m.insert("STOCH_RSI".to_string(),      0.6);
    m.insert("CCI".to_string(),            0.7);
    // Default fallback için anahtar (resolve sırasında bulunamayan strateji adına).
    m.insert("default".to_string(),        0.7);
    m
}

/// Hysteresis: yeni rejim drift sayılmadan önce kaç ardışık cycle gözlemlenmeli.
pub const DRIFT_CONFIRMATION_TURNS: u32 = 3;
/// Cooldown: iki ardışık drift arasında en az kaç saniye geçmeli (saniye, epoch).
pub const DRIFT_COOLDOWN_SECS: u64 = 60;

impl Default for ParameterStore {
    fn default() -> Self {
        Self {
            edge_thresholds: EdgeThresholds::default(),
            partial_fill:    PartialFillParams::default(),
            trade_risk:      TradeRiskParams::default(),
            scalp_swing_threshold_min: 60,
            sr_update_every_secs:      30,
            regime_overrides: HashMap::new(),
            regime_feedback:  HashMap::new(),
            last_observed_regime: None,
            pending_regime: None,
            last_drift_at_secs: 0,
            symbol_stats: HashMap::new(),
            strategy_trail_targets: default_strategy_trail_targets(),
            trail_feedback: HashMap::new(),
            multi_tf: MultiTfParams::default(),
            leverage: LeverageParams::default(),
            strategy_params: HashMap::new(),
        }
    }
}

impl ParameterStore {
    /// Boot anında çağrılır: önce Default, sonra ENV override'ları.
    /// Tanınan env değişkenleri:
    ///   EDGE_THRESHOLD_{COLD,WARM,HOT}, EDGE_{COLD,WARM}_UNTIL
    ///   PARTIAL_FILL_OVERFILL_TOLERANCE, PARTIAL_FILL_CUM_TOLERANCE,
    ///   PARTIAL_FILL_MAX_SLIPPAGE_PCT
    ///   SCALP_SWING_THRESHOLD_MIN, SR_UPDATE_EVERY_SECS
    pub fn from_env() -> Self {
        let mut store = Self::default();
        if let Some(v) = parse_env_f64("EDGE_THRESHOLD_COLD") {
            store.edge_thresholds.cold = v;
        }
        if let Some(v) = parse_env_f64("EDGE_THRESHOLD_WARM") {
            store.edge_thresholds.warm = v;
        }
        if let Some(v) = parse_env_f64("EDGE_THRESHOLD_HOT") {
            store.edge_thresholds.hot = v;
        }
        if let Some(v) = parse_env_f64("EDGE_COLD_UNTIL") {
            store.edge_thresholds.cold_until = v;
        }
        if let Some(v) = parse_env_f64("EDGE_WARM_UNTIL") {
            store.edge_thresholds.warm_until = v;
        }
        if let Some(v) = parse_env_f64("PARTIAL_FILL_OVERFILL_TOLERANCE") {
            store.partial_fill.overfill_tolerance = v;
        }
        if let Some(v) = parse_env_f64("PARTIAL_FILL_CUM_TOLERANCE") {
            store.partial_fill.cum_tolerance = v;
        }
        if let Some(v) = parse_env_f64("PARTIAL_FILL_MAX_SLIPPAGE_PCT") {
            store.partial_fill.max_slippage_pct = v;
        }
        if let Some(v) = std::env::var("SCALP_SWING_THRESHOLD_MIN").ok()
            .and_then(|v| v.parse::<i64>().ok()) {
            store.scalp_swing_threshold_min = v;
        }
        if let Some(v) = std::env::var("SR_UPDATE_EVERY_SECS").ok()
            .and_then(|v| v.parse::<u64>().ok()) {
            store.sr_update_every_secs = v;
        }
        // Multi-TF (Faz B) env override'ları.
        if let Some(v) = parse_env_bool("MULTI_TF_ENABLED") {
            store.multi_tf.enabled = v;
        }
        if let Some(v) = std::env::var("MULTI_TF_MIN_REQUIRED").ok()
            .and_then(|v| v.parse::<usize>().ok()) {
            store.multi_tf.min_required = v;
        }
        if let Some(v) = parse_env_bool("MULTI_TF_DOWNLOAD") {
            store.multi_tf.download_htf = v;
        }
        // Leverage (otonom katman) env override'ları.
        if let Some(v) = parse_env_bool("LEVERAGE_ENABLED") {
            store.leverage.enabled = v;
        }
        if let Some(v) = parse_env_f64("LEVERAGE_BASE") {
            store.leverage.base = v;
        }
        if let Some(v) = parse_env_f64("LEVERAGE_MAX") {
            store.leverage.max = v;
        }
        if let Some(v) = parse_env_f64("LEVERAGE_CONF_THRESHOLD") {
            store.leverage.conf_boost_threshold = v;
        }
        if let Some(v) = parse_env_f64("LEVERAGE_VOL_FLOOR_PCT") {
            store.leverage.vol_floor_pct = v;
        }
        store
    }

    /// Otonom leverage çözücü. enabled=false ise daima 1.0 döner.
    /// Formül (base'in üzerine çarpan katmanları, sonda [1.0, max] clamp):
    ///   - Rejim: HighVolatility → ×0.5, Strong{Up,Down}trend → ×1.3,
    ///     Ranging/LowVolatility → ×1.0, Weak* / Unknown → ×0.9.
    ///   - ML confidence ≥ conf_boost_threshold → ×1.2.
    ///   - win_rate ≥ 0.6 → ×1.15; ∈ (0.0, 0.4] → ×0.75; aksi (0 veya nötr) → ×1.0.
    ///   - noise_floor_pct > vol_floor_pct → ×0.7 (yüksek wiggle).
    ///
    /// `win_rate=0.0` "henüz veri yok" anlamında nötr sayılır (yeni başlangıç
    /// cezalandırılmaz). `noise_floor_pct=None` (stats yok) → volatilite
    /// faktörü uygulanmaz.
    pub fn resolve_leverage(
        &self,
        regime: &str,
        ml_confidence: f64,
        win_rate: f64,
        noise_floor_pct: Option<f64>,
    ) -> f64 {
        if !self.leverage.enabled {
            return 1.0;
        }
        let mut lev = self.leverage.base;

        // Rejim modülasyonu
        lev *= match regime {
            "HighVolatility" | "Volatile"               => 0.5,
            "StrongUptrend"  | "StrongDowntrend"        => 1.3,
            "Ranging"        | "LowVolatility"          => 1.0,
            // Weak trends / Unknown → temkinli
            _ => 0.9,
        };

        // ML confidence boost
        if ml_confidence >= self.leverage.conf_boost_threshold {
            lev *= 1.2;
        }

        // Win rate feedback (0.0 = "veri yok" → nötr)
        if win_rate >= 0.6 {
            lev *= 1.15;
        } else if win_rate > 0.0 && win_rate <= 0.4 {
            lev *= 0.75;
        }

        // Volatilite tabanı — symbol_stats taze değilse None → atla
        if let Some(noise) = noise_floor_pct {
            if noise > self.leverage.vol_floor_pct {
                lev *= 0.7;
            }
        }

        lev.clamp(1.0, self.leverage.max.max(1.0))
    }

    /// `EdgeThresholds::for_confidence`'in kestirme erişim noktası.
    /// Engine cycle'ları ParameterStore tutarken bu method'a doğrudan ulaşır.
    pub fn edge_threshold(&self, ml_confidence: f64) -> f64 {
        self.edge_thresholds.for_confidence(ml_confidence)
    }

    /// HyperOpt veya ML retrain job'larının ürettiği `OptimizationParameters`'ı
    /// store'un trade_risk alanına yazar. `f64` üçlüsü olarak iletilir ki
    /// modül bağımsızlığı korunsun (ParameterStore başka modüllere bağlı
    /// olmadan kendi başına test edilebilir).
    pub fn apply_optimization(&mut self, take_profit_pct: f64, stop_loss_pct: f64, max_position_size: f64) {
        // Sıfır/negatif değerler kabul edilmez; default'a düş.
        if take_profit_pct > 0.0   { self.trade_risk.take_profit_pct   = take_profit_pct; }
        if stop_loss_pct   > 0.0   { self.trade_risk.stop_loss_pct     = stop_loss_pct; }
        if max_position_size > 0.0 && max_position_size <= 1.0 {
            self.trade_risk.max_position_size = max_position_size;
        }
    }

    // ─── Rejim-bazlı getter'lar ─────────────────────────────────────────
    //
    // Sparse patch semantiği: rejim için patch yoksa veya patch'in ilgili
    // alanı None ise base parametre kullanılır. Engine cycle'ları rejimi
    // her tur sınıflandırıyor (`Engine::classify_regime`); store sorgusunu
    // o rejim string'iyle yapar.

    /// İlgili rejim için (override varsa o, yoksa base) EdgeThresholds.
    pub fn edge_thresholds_for(&self, regime: &str) -> EdgeThresholds {
        self.regime_overrides.get(regime)
            .and_then(|p| p.edge_thresholds)
            .unwrap_or(self.edge_thresholds)
    }

    /// `edge_thresholds_for` üstüne `for_confidence` zinciri — engine direkt çağırır.
    pub fn edge_threshold_for(&self, regime: &str, ml_confidence: f64) -> f64 {
        self.edge_thresholds_for(regime).for_confidence(ml_confidence)
    }

    /// İlgili rejim için (override varsa o, yoksa base) TradeRiskParams.
    pub fn trade_risk_for(&self, regime: &str) -> TradeRiskParams {
        self.regime_overrides.get(regime)
            .and_then(|p| p.trade_risk)
            .unwrap_or(self.trade_risk)
    }

    /// Sembol+interval için noise-floor stats'i günceller (perform_download çağrır).
    /// Zaten varsa üzerine yazar (taze veri eski hesabı geçersiz kılar).
    pub fn update_symbol_stats(&mut self, symbol: &str, interval: &str, stats: SymbolStats) {
        self.symbol_stats.insert((symbol.to_string(), interval.to_string()), stats);
    }

    /// Stale stats temizliği (cold-start / bakım). TTL üstü kayıtları siler.
    /// Engine periyodik çağırmıyor; in-memory store küçük (~100 entry max).
    pub fn purge_stale_symbol_stats(&mut self, ttl_secs: u64) {
        self.symbol_stats.retain(|_, s| symbol_stats::is_fresh(s, ttl_secs));
    }

    /// Strateji ismine göre trail target yüzdesi (Phase B fallback).
    /// Symbol-bağlamsız çağrılırsa per-symbol feedback patch'i atlanır.
    /// Yeni call site'lar `target_trail_pct_for_strategy_and_symbol` kullanmalı.
    pub fn target_trail_pct_for_strategy(&self, strategy_name: &str) -> f64 {
        if let Some(v) = parse_env_f64("TARGET_TRAIL_PCT") {
            return v;
        }
        if let Some(&v) = self.strategy_trail_targets.get(strategy_name) {
            return v;
        }
        self.strategy_trail_targets.get("default").copied().unwrap_or(0.7)
    }

    /// Strateji + sembol bağlamlı trail target — Phase C feedback katmanı dahil.
    /// Precedence:
    ///   1) `TARGET_TRAIL_PCT` env — operatör global override
    ///   2) `trail_feedback[(sym, strategy)].target_override` — runtime feedback patch
    ///   3) `strategy_trail_targets[strategy_name]` — Phase B sensible default
    ///   4) `strategy_trail_targets["default"]` — bilinmeyen strateji fallback
    ///   5) Hard-coded 0.7 — store boşsa
    pub fn target_trail_pct_for_strategy_and_symbol(&self, symbol: &str, strategy_name: &str) -> f64 {
        if let Some(v) = parse_env_f64("TARGET_TRAIL_PCT") {
            return v;
        }
        if let Some(fb) = self.trail_feedback.get(&(symbol.to_string(), strategy_name.to_string())) {
            if let Some(override_pct) = fb.target_override {
                return override_pct;
            }
        }
        if let Some(&v) = self.strategy_trail_targets.get(strategy_name) {
            return v;
        }
        self.strategy_trail_targets.get("default").copied().unwrap_or(0.7)
    }

    /// Trailing-stop outcome'ını sayaçlara yaz; pencere dolmuşsa adjustment dener.
    /// Caller (master.rs observation processor) `was_early` değerini PendingTrailObservation
    /// üzerinden hesaplar. `base_target` = mevcut strateji default'u (adjustment referansı).
    /// Döner: target_override değiştiyse Some(new_target) — logging için.
    pub fn record_trailing_outcome(
        &mut self,
        symbol: &str,
        strategy_name: &str,
        was_early: bool,
    ) -> Option<f64> {
        let base = self.target_trail_pct_for_strategy(strategy_name);
        let fb = self.trail_feedback
            .entry((symbol.to_string(), strategy_name.to_string()))
            .or_insert_with(TrailFeedback::new);
        fb.record_outcome(was_early, base)
    }

    /// ATR-trail multiplier çözümleme — Otonom katman.
    ///
    /// Precedence:
    /// 1) `symbol_stats[(sym, interval)]` fresh + sample_size≥50:
    ///    `mult = strategy_target_pct / noise_floor_pct`, clamp [1.5, 30].
    /// 2) Hiç stats yok / stale / yetersiz örneklem → `default_mult` (genelde best_params veya 2.0).
    ///
    /// `strategy_name` = pozisyonu açan/açacak stratejinin kanonik adı (SUPERTREND, BB, ...).
    /// Target pct strateji niyetine göre değişir; env override (TARGET_TRAIL_PCT) hala onurludur.
    /// Bu strateji için backtest'in `param_spec` araması ile bulduğu en iyi yapısal
    /// parametreler; yoksa `StrategyParams::default()`. Canlı cycle bunu
    /// `generate_signal`'a verir → optimize edilmiş indikatör periyot/eşikleri
    /// canlıya ulaşır (eskiden her zaman default geçiliyordu — kaçak buradan kapanır).
    pub fn resolve_strategy_params(&self, strategy_name: &str) -> crate::core::types::StrategyParams {
        self.strategy_params.get(strategy_name).copied().unwrap_or_default()
    }

    /// Backtest job kazanan strateji için optimize ettiği yapısal parametreleri buraya yazar.
    pub fn set_strategy_params(
        &mut self,
        strategy_name: impl Into<String>,
        params: crate::core::types::StrategyParams,
    ) {
        self.strategy_params.insert(strategy_name.into(), params);
    }

    pub fn resolve_atr_mult(
        &self,
        symbol: &str,
        interval: &str,
        strategy_name: &str,
        default_mult: f64,
    ) -> f64 {
        const TTL_SECS: u64 = 21_600; // 6 saat
        const MIN_MULT: f64 = 1.5;
        const MAX_MULT: f64 = 30.0;
        const MIN_SAMPLE: usize = 50;

        if let Some(s) = self.symbol_stats.get(&(symbol.to_string(), interval.to_string())) {
            if s.sample_size >= MIN_SAMPLE
                && s.noise_floor_pct > 0.0
                && symbol_stats::is_fresh(s, TTL_SECS)
            {
                // Phase C feedback patch'i de dahil (per-sym, strateji override).
                let target = self.target_trail_pct_for_strategy_and_symbol(symbol, strategy_name);
                let mult = target / s.noise_floor_pct;
                return mult.clamp(MIN_MULT, MAX_MULT);
            }
        }
        default_mult
    }

    /// Belirli bir rejim için patch yerleştirir (HyperOpt rejim-aware tuning sonucu).
    pub fn set_regime_patch(&mut self, regime: impl Into<String>, patch: RegimePatch) {
        self.regime_overrides.insert(regime.into(), patch);
    }

    /// Faz 3 c2: bir trade kapanışında rejim+pnl_pct geri beslemesini işler.
    ///
    /// Kuyruğu günceller (`RegimeFeedback::record`); rejim için WINDOW kadar
    /// trade biriktiyse ve win_rate eşik altına düştüyse patch'i sıkılaştırır
    /// (`tighten_regime_patch`).
    ///
    /// Eşik default 0.40 (10 trade'in en az 4'ü kazançlı olmalı); ileride
    /// HyperOpt'tan ayarlanabilir hale getirilir.
    ///
    /// Döner: tighten gerçekten uygulandı mı (true) / koşul oluşmadı (false).
    pub fn apply_trade_feedback(&mut self, regime: &str, pnl_pct: f64) -> bool {
        let fb = self.regime_feedback.entry(regime.to_string())
            .or_insert_with(RegimeFeedback::default);
        fb.record(pnl_pct);

        // Sıkılaştırma için yeterli veri yoksa çık.
        if fb.recent_pnl.len() < RegimeFeedback::WINDOW { return false; }
        let win_rate = fb.win_rate();
        let trigger_threshold = 0.40;
        if win_rate >= trigger_threshold { return false; }

        self.tighten_regime_patch(regime);
        true
    }

    /// Faz 3 c3: rejim değişimi gözlemi (hysteresis + cooldown'lı).
    ///
    /// Engine cycle her tur bu metodu çağırır. Anlık rejim sallanmaları drift
    /// saymaz; bir rejimin "confirmed drift" olması için iki koşul gerekir:
    ///   1. Yeni rejim üst üste `DRIFT_CONFIRMATION_TURNS` cycle aynı kalmalı (hysteresis).
    ///   2. Son confirmed drift'ten en az `DRIFT_COOLDOWN_SECS` geçmiş olmalı (cooldown).
    ///
    /// Confirmed drift'te:
    ///   - `last_observed_regime` güncellenir.
    ///   - `last_drift_at_secs` şimdiki epoch'a sabitlenir.
    ///   - Yeni rejim için patch bir basamak sıkılaştırılır.
    ///   - `true` döner (çağıran taraf push_alert atabilir).
    /// İlk gözlem değişim sayılmaz (cold start için yumuşak).
    pub fn observe_regime(&mut self, regime: &str) -> bool {
        self.observe_regime_with_now(regime, now_epoch_secs())
    }

    /// `observe_regime`'in deterministik çekirdeği — `now_secs` parametre olarak
    /// alındığı için unit testlerde zaman sahteleştirilebilir.
    pub fn observe_regime_with_now(&mut self, regime: &str, now_secs: u64) -> bool {
        // İlk gözlem: cold-start; sadece kaydet ve geçiş bildirme.
        let prev = match self.last_observed_regime.as_deref() {
            Some(p) => p.to_string(),
            None => {
                self.last_observed_regime = Some(regime.to_string());
                self.pending_regime = None;
                return false;
            }
        };

        // Aynı rejim devam ediyor → aday sıfırlanır, drift yok.
        if prev == regime {
            self.pending_regime = None;
            return false;
        }

        // Yeni bir rejim aday olarak görünüyor → ardışık sayımı arttır.
        let count = match self.pending_regime.take() {
            Some((cand, n)) if cand == regime => n.saturating_add(1),
            _ => 1,
        };

        // Hysteresis: yeterince üst üste görüldü mü?
        if count < DRIFT_CONFIRMATION_TURNS {
            self.pending_regime = Some((regime.to_string(), count));
            return false;
        }

        // Cooldown: önceki drift üstünden yeterli süre geçti mi?
        if self.last_drift_at_secs != 0
            && now_secs.saturating_sub(self.last_drift_at_secs) < DRIFT_COOLDOWN_SECS
        {
            // Aday kararlı ama henüz cooldown bitmedi — sayımı tutmaya devam et,
            // ancak tighten/log tetiklemiyoruz. Cooldown sonunda bir sonraki
            // çağrıda confirmed olacak.
            self.pending_regime = Some((regime.to_string(), count));
            return false;
        }

        // Onaylı drift.
        self.last_observed_regime = Some(regime.to_string());
        self.last_drift_at_secs = now_secs;
        self.pending_regime = None;
        self.tighten_regime_patch(regime);
        true
    }

    /// Rejim patch'ini tek bir basamak sıkılaştırır. apply_trade_feedback ve
    /// observe_regime tek bir mantık üstüne çalışsın diye paylaşılmış helper.
    /// Mevcut patch varsa onun üstüne, yoksa base'in üstüne uygulanır.
    /// Katsayılar deneyimsel: edge *1.15, TP/SL *0.85, max_pos *0.70.
    fn tighten_regime_patch(&mut self, regime: &str) {
        let existing = self.regime_overrides.get(regime).cloned().unwrap_or_default();
        let base_edge = existing.edge_thresholds.unwrap_or(self.edge_thresholds);
        let base_risk = existing.trade_risk.unwrap_or(self.trade_risk);

        let tightened_edge = EdgeThresholds {
            cold: (base_edge.cold * 1.15).min(0.95),
            warm: (base_edge.warm * 1.15).min(0.95),
            hot:  (base_edge.hot  * 1.15).min(0.95),
            cold_until: base_edge.cold_until,
            warm_until: base_edge.warm_until,
        };
        let tightened_risk = TradeRiskParams {
            take_profit_pct:   (base_risk.take_profit_pct * 0.85).max(0.5),
            stop_loss_pct:     (base_risk.stop_loss_pct   * 0.85).max(0.3),
            max_position_size: (base_risk.max_position_size * 0.70).max(0.10),
        };
        self.regime_overrides.insert(regime.to_string(),
            RegimePatch::empty()
                .with_edge(tightened_edge)
                .with_trade_risk(tightened_risk));
    }
}

fn parse_env_f64(key: &str) -> Option<f64> {
    std::env::var(key).ok().and_then(|v| v.parse().ok())
}

/// Bool env okuyucu — kabul edilen değerler: "1"/"0", "true"/"false",
/// "yes"/"no", "on"/"off" (case-insensitive). Tanımsız veya tanınmayan
/// değerlerde None döner → çağıran default değeri korur.
fn parse_env_bool(key: &str) -> Option<bool> {
    let v = std::env::var(key).ok()?;
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on"  => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Sistemden epoch saniyesini okur; SystemTime hatasında 0 döner (cooldown
/// kapanır → güvenli taraf: hiç drift atma değil, eski davranışla aynı).
fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SymbolStats fixture: belirli yaşa sahip tazeleyici. Now-offset saniye.
    fn make_stats(noise_pct: f64, sample: usize, age_secs: u64) -> SymbolStats {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        SymbolStats {
            noise_floor_pct: noise_pct,
            p90_range_pct:   noise_pct * 1.5,
            sample_size:     sample,
            last_updated:    now.saturating_sub(age_secs),
        }
    }

    #[test]
    fn resolve_atr_mult_returns_default_when_no_stats() {
        let s = ParameterStore::default();
        let m = s.resolve_atr_mult("BTCUSDT", "1m", "SUPERTREND", 2.0);
        assert!((m - 2.0).abs() < 1e-9, "stats yokken default döner");
    }

    #[test]
    fn resolve_atr_mult_uses_strategy_target_for_trend() {
        let mut s = ParameterStore::default();
        // SUPERTREND target=1.2, noise=0.05 → mult = 24
        s.update_symbol_stats("ETHUSDT", "1m", make_stats(0.05, 100, 60));
        let m = s.resolve_atr_mult("ETHUSDT", "1m", "SUPERTREND", 2.0);
        assert!((m - 24.0).abs() < 1e-9, "mult = 1.2/0.05 = 24, gerçek {}", m);
    }

    #[test]
    fn resolve_atr_mult_uses_strategy_target_for_meanrev() {
        let mut s = ParameterStore::default();
        // BB target=0.5, noise=0.05 → mult = 10
        s.update_symbol_stats("ETHUSDT", "1m", make_stats(0.05, 100, 60));
        let m = s.resolve_atr_mult("ETHUSDT", "1m", "BB", 2.0);
        assert!((m - 10.0).abs() < 1e-9, "BB mean-rev: mult = 0.5/0.05 = 10, gerçek {}", m);
    }

    #[test]
    fn resolve_atr_mult_falls_back_to_default_target_for_unknown_strategy() {
        let mut s = ParameterStore::default();
        // default target=0.7, noise=0.05 → mult = 14
        s.update_symbol_stats("ETHUSDT", "1m", make_stats(0.05, 100, 60));
        let m = s.resolve_atr_mult("ETHUSDT", "1m", "FANCY_UNKNOWN", 2.0);
        assert!((m - 14.0).abs() < 1e-9, "unknown → default target 0.7, gerçek {}", m);
    }

    #[test]
    fn resolve_atr_mult_clamps_at_max_for_extreme_low_noise() {
        let mut s = ParameterStore::default();
        // Noise = %0.001, target SUPERTREND=1.2 → mult = 1200 → clamp 30
        s.update_symbol_stats("BTCUSDT", "1m", make_stats(0.001, 100, 0));
        let m = s.resolve_atr_mult("BTCUSDT", "1m", "SUPERTREND", 2.0);
        assert!((m - 30.0).abs() < 1e-9, "MAX_MULT clamp 30, gerçek {}", m);
    }

    #[test]
    fn resolve_atr_mult_clamps_at_min_for_very_wide_noise() {
        let mut s = ParameterStore::default();
        // Noise = %5 (çok yüksek), target=1.2 → mult = 0.24 → clamp 1.5
        s.update_symbol_stats("XYZUSDT", "1h", make_stats(5.0, 100, 0));
        let m = s.resolve_atr_mult("XYZUSDT", "1h", "SUPERTREND", 2.0);
        assert!((m - 1.5).abs() < 1e-9, "MIN_MULT clamp 1.5, gerçek {}", m);
    }

    #[test]
    fn resolve_atr_mult_falls_back_when_stats_stale() {
        let mut s = ParameterStore::default();
        s.update_symbol_stats("BTCUSDT", "1m", make_stats(0.025, 100, 10 * 3600));
        let m = s.resolve_atr_mult("BTCUSDT", "1m", "SUPERTREND", 2.0);
        assert!((m - 2.0).abs() < 1e-9, "stale → default 2.0, gerçek {}", m);
    }

    #[test]
    fn resolve_atr_mult_falls_back_when_sample_too_small() {
        let mut s = ParameterStore::default();
        s.update_symbol_stats("BTCUSDT", "1m", make_stats(0.025, 30, 60));
        let m = s.resolve_atr_mult("BTCUSDT", "1m", "SUPERTREND", 2.0);
        assert!((m - 2.0).abs() < 1e-9, "low sample → default, gerçek {}", m);
    }

    #[test]
    fn target_trail_pct_picks_strategy_specific_default() {
        let s = ParameterStore::default();
        // Default tablodan
        assert!((s.target_trail_pct_for_strategy("SUPERTREND") - 1.2).abs() < 1e-9);
        assert!((s.target_trail_pct_for_strategy("BB") - 0.5).abs() < 1e-9);
        assert!((s.target_trail_pct_for_strategy("MA_CROSSOVER") - 1.5).abs() < 1e-9);
        // Bilinmeyen → default
        assert!((s.target_trail_pct_for_strategy("FANCY_FOO") - 0.7).abs() < 1e-9);
    }

    #[test]
    fn purge_stale_keeps_fresh_drops_old() {
        let mut s = ParameterStore::default();
        s.update_symbol_stats("FRESH", "1m", make_stats(0.5, 100, 60));
        s.update_symbol_stats("STALE", "1m", make_stats(0.5, 100, 7 * 3600));
        s.purge_stale_symbol_stats(6 * 3600);
        assert!(s.symbol_stats.contains_key(&("FRESH".into(), "1m".into())));
        assert!(!s.symbol_stats.contains_key(&("STALE".into(), "1m".into())));
    }

    #[test]
    fn default_edge_thresholds_match_legacy_constants() {
        // Faz 1 öncesi Engine::dynamic_edge_threshold sabitleri: 0.20 / 0.35 / 0.55.
        // Default ParameterStore aynı değerleri korumalı (geriye uyum).
        let s = ParameterStore::default();
        assert_eq!(s.edge_thresholds.cold, 0.20);
        assert_eq!(s.edge_thresholds.warm, 0.35);
        assert_eq!(s.edge_thresholds.hot,  0.55);
        assert_eq!(s.edge_thresholds.cold_until, 0.05);
        assert_eq!(s.edge_thresholds.warm_until, 0.30);
    }

    #[test]
    fn edge_threshold_picks_correct_tier_for_each_confidence_zone() {
        let s = ParameterStore::default();
        // cold: ml < 0.05
        assert!((s.edge_threshold(0.0)  - 0.20).abs() < 1e-9);
        assert!((s.edge_threshold(0.04) - 0.20).abs() < 1e-9);
        // warm: 0.05 ≤ ml < 0.30
        assert!((s.edge_threshold(0.05) - 0.35).abs() < 1e-9);
        assert!((s.edge_threshold(0.20) - 0.35).abs() < 1e-9);
        assert!((s.edge_threshold(0.29) - 0.35).abs() < 1e-9);
        // hot: ml ≥ 0.30
        assert!((s.edge_threshold(0.30) - 0.55).abs() < 1e-9);
        assert!((s.edge_threshold(0.99) - 0.55).abs() < 1e-9);
    }

    #[test]
    fn from_env_overrides_individual_thresholds() {
        std::env::set_var("EDGE_THRESHOLD_HOT", "0.70");
        std::env::set_var("EDGE_WARM_UNTIL",    "0.50");
        let s = ParameterStore::from_env();
        std::env::remove_var("EDGE_THRESHOLD_HOT");
        std::env::remove_var("EDGE_WARM_UNTIL");
        assert!((s.edge_thresholds.hot - 0.70).abs() < 1e-9);
        assert!((s.edge_thresholds.warm_until - 0.50).abs() < 1e-9);
        // Diğer alanlar default'ta kalmalı.
        assert_eq!(s.edge_thresholds.cold, 0.20);
        assert_eq!(s.edge_thresholds.warm, 0.35);
    }

    #[test]
    fn from_env_with_garbage_falls_back_to_default() {
        std::env::set_var("EDGE_THRESHOLD_COLD", "not_a_number");
        let s = ParameterStore::from_env();
        std::env::remove_var("EDGE_THRESHOLD_COLD");
        assert_eq!(s.edge_thresholds.cold, 0.20);
    }

    // ─── Multi-TF (Faz B) ────────────────────────────────────────────────

    #[test]
    fn multi_tf_default_enabled() {
        let s = ParameterStore::default();
        assert!(s.multi_tf.enabled, "default'ta multi-TF açık olmalı");
        assert_eq!(s.multi_tf.min_required, 30);
        assert!(s.multi_tf.download_htf);
    }

    #[test]
    fn from_env_multi_tf_disabled_via_env() {
        std::env::set_var("MULTI_TF_ENABLED", "false");
        std::env::set_var("MULTI_TF_DOWNLOAD", "0");
        std::env::set_var("MULTI_TF_MIN_REQUIRED", "50");
        let s = ParameterStore::from_env();
        std::env::remove_var("MULTI_TF_ENABLED");
        std::env::remove_var("MULTI_TF_DOWNLOAD");
        std::env::remove_var("MULTI_TF_MIN_REQUIRED");
        assert!(!s.multi_tf.enabled);
        assert!(!s.multi_tf.download_htf);
        assert_eq!(s.multi_tf.min_required, 50);
    }

    #[test]
    fn parse_env_bool_accepts_common_forms() {
        std::env::set_var("MTF_TEST_BOOL", "yes");
        assert_eq!(parse_env_bool("MTF_TEST_BOOL"), Some(true));
        std::env::set_var("MTF_TEST_BOOL", "OFF");
        assert_eq!(parse_env_bool("MTF_TEST_BOOL"), Some(false));
        std::env::set_var("MTF_TEST_BOOL", "garbage");
        assert_eq!(parse_env_bool("MTF_TEST_BOOL"), None);
        std::env::remove_var("MTF_TEST_BOOL");
        assert_eq!(parse_env_bool("MTF_TEST_BOOL"), None);
    }

    // ─── Leverage (Otonom katman) ────────────────────────────────────────

    fn enabled_lev_store() -> ParameterStore {
        let mut s = ParameterStore::default();
        s.leverage.enabled = true;
        s.leverage.base = 3.0;
        s.leverage.max = 10.0;
        s.leverage.conf_boost_threshold = 0.70;
        s.leverage.vol_floor_pct = 1.0;
        s
    }

    #[test]
    fn resolve_leverage_returns_one_when_disabled() {
        // Default artık enabled=true (otonom davranış); kapalıya çekip kontrol.
        let mut s = ParameterStore::default();
        s.leverage.enabled = false;
        assert_eq!(s.resolve_leverage("StrongUptrend", 0.9, 0.7, Some(0.5)), 1.0);
    }

    #[test]
    fn resolve_leverage_default_is_autonomous() {
        let s = ParameterStore::default();
        assert!(s.leverage.enabled, "default'ta otonom leverage açık");
        assert_eq!(s.leverage.base, 3.0);
        assert_eq!(s.leverage.max, 10.0);
        // StrongUptrend + yüksek conf + iyi wr → 3.0 * 1.3 * 1.2 * 1.15 = 5.382
        let lev = s.resolve_leverage("StrongUptrend", 0.85, 0.7, Some(0.3));
        assert!(lev > 1.0 && lev <= 10.0, "dinamik aralık, got {}", lev);
    }

    #[test]
    fn resolve_leverage_uses_regime_factor() {
        let s = enabled_lev_store();
        // base=3.0, ranging=×1.0, ml=0.5 (no boost), wr=0.5 (neutral), no vol → 3.0
        let lev = s.resolve_leverage("Ranging", 0.5, 0.5, Some(0.5));
        assert!((lev - 3.0).abs() < 1e-9, "ranging+neutral → base, got {}", lev);
    }

    #[test]
    fn resolve_leverage_high_vol_halves() {
        let s = enabled_lev_store();
        // base=3.0 × 0.5 = 1.5
        let lev = s.resolve_leverage("HighVolatility", 0.5, 0.5, None);
        assert!((lev - 1.5).abs() < 1e-9, "highvol → ×0.5, got {}", lev);
    }

    #[test]
    fn resolve_leverage_strong_trend_with_conf_and_wins() {
        let s = enabled_lev_store();
        // 3.0 × 1.3 (strong up) × 1.2 (conf>0.70) × 1.15 (wr≥0.6) = 5.382
        let lev = s.resolve_leverage("StrongUptrend", 0.85, 0.7, Some(0.3));
        assert!((lev - 5.382).abs() < 1e-3, "boost stack: 3×1.3×1.2×1.15, got {}", lev);
    }

    #[test]
    fn resolve_leverage_clamps_to_max() {
        let mut s = enabled_lev_store();
        s.leverage.base = 8.0;
        // 8.0 × 1.3 × 1.2 × 1.15 = 14.35 → clamp to max 10.0
        let lev = s.resolve_leverage("StrongDowntrend", 0.9, 0.7, Some(0.3));
        assert!((lev - 10.0).abs() < 1e-9, "clamp to max, got {}", lev);
    }

    #[test]
    fn resolve_leverage_clamps_to_floor_one() {
        let s = enabled_lev_store();
        // 3.0 × 0.5 (highvol) × 0.75 (wr=0.3) × 0.7 (high noise) = 0.7875 → clamp 1.0
        let lev = s.resolve_leverage("HighVolatility", 0.5, 0.3, Some(2.5));
        assert!((lev - 1.0).abs() < 1e-9, "floor 1.0 koruması, got {}", lev);
    }

    #[test]
    fn resolve_leverage_zero_winrate_treated_neutral() {
        let s = enabled_lev_store();
        // wr=0.0 ("veri yok") cezalandırılmamalı: base=3.0 × ranging=1.0 = 3.0
        let lev_zero = s.resolve_leverage("Ranging", 0.5, 0.0, None);
        let lev_neut = s.resolve_leverage("Ranging", 0.5, 0.5, None);
        assert!((lev_zero - lev_neut).abs() < 1e-9, "0.0 nötr olmalı, zero={} neut={}", lev_zero, lev_neut);
    }

    #[test]
    fn resolve_leverage_noise_floor_optional() {
        let s = enabled_lev_store();
        // None → vol faktörü uygulanmaz; Some(altında) → uygulanmaz; Some(üstünde) → ×0.7
        let lev_none  = s.resolve_leverage("Ranging", 0.5, 0.5, None);
        let lev_under = s.resolve_leverage("Ranging", 0.5, 0.5, Some(0.5));
        let lev_over  = s.resolve_leverage("Ranging", 0.5, 0.5, Some(2.0));
        assert!((lev_none - lev_under).abs() < 1e-9);
        assert!(lev_over < lev_none, "yüksek noise → düşük lev");
    }

    #[test]
    fn from_env_leverage_override_chain() {
        std::env::set_var("LEVERAGE_ENABLED",        "true");
        std::env::set_var("LEVERAGE_BASE",           "5.0");
        std::env::set_var("LEVERAGE_MAX",            "20.0");
        std::env::set_var("LEVERAGE_CONF_THRESHOLD", "0.80");
        std::env::set_var("LEVERAGE_VOL_FLOOR_PCT",  "1.5");
        let s = ParameterStore::from_env();
        std::env::remove_var("LEVERAGE_ENABLED");
        std::env::remove_var("LEVERAGE_BASE");
        std::env::remove_var("LEVERAGE_MAX");
        std::env::remove_var("LEVERAGE_CONF_THRESHOLD");
        std::env::remove_var("LEVERAGE_VOL_FLOOR_PCT");
        assert!(s.leverage.enabled);
        assert!((s.leverage.base - 5.0).abs() < 1e-9);
        assert!((s.leverage.max - 20.0).abs() < 1e-9);
        assert!((s.leverage.conf_boost_threshold - 0.80).abs() < 1e-9);
        assert!((s.leverage.vol_floor_pct - 1.5).abs() < 1e-9);
    }

    #[test]
    fn default_partial_fill_matches_legacy_constants() {
        let s = ParameterStore::default();
        assert_eq!(s.partial_fill.overfill_tolerance, 0.001);
        assert_eq!(s.partial_fill.cum_tolerance,      0.001);
        assert_eq!(s.partial_fill.max_slippage_pct,   1.0);
    }

    #[test]
    fn default_scalp_swing_and_sr_update_match_legacy() {
        let s = ParameterStore::default();
        assert_eq!(s.scalp_swing_threshold_min, 60);
        assert_eq!(s.sr_update_every_secs,      30);
    }

    #[test]
    fn default_trade_risk_matches_legacy_fallbacks() {
        let s = ParameterStore::default();
        assert_eq!(s.trade_risk.take_profit_pct,   3.0);
        assert_eq!(s.trade_risk.stop_loss_pct,     1.5);
        assert_eq!(s.trade_risk.max_position_size, 0.5);
    }

    #[test]
    fn apply_optimization_writes_trade_risk_fields() {
        let mut s = ParameterStore::default();
        s.apply_optimization(4.5, 2.0, 0.75);
        assert!((s.trade_risk.take_profit_pct - 4.5).abs() < 1e-9);
        assert!((s.trade_risk.stop_loss_pct   - 2.0).abs() < 1e-9);
        assert!((s.trade_risk.max_position_size - 0.75).abs() < 1e-9);
    }

    #[test]
    fn apply_optimization_rejects_invalid_values() {
        let mut s = ParameterStore::default();
        s.apply_optimization(-1.0, 0.0, 2.0); // hepsi geçersiz
        // Default'ta kalmalı
        assert_eq!(s.trade_risk.take_profit_pct,   3.0);
        assert_eq!(s.trade_risk.stop_loss_pct,     1.5);
        assert_eq!(s.trade_risk.max_position_size, 0.5);
    }

    #[test]
    fn apply_optimization_partial_keeps_unspecified_alone() {
        let mut s = ParameterStore::default();
        // Sadece TP geçerli, SL=0 (skip), max_pos > 1 (skip).
        s.apply_optimization(5.0, 0.0, 1.5);
        assert_eq!(s.trade_risk.take_profit_pct,   5.0);
        assert_eq!(s.trade_risk.stop_loss_pct,     1.5); // default kaldı
        assert_eq!(s.trade_risk.max_position_size, 0.5); // default kaldı
    }

    #[test]
    fn observe_regime_first_call_does_not_report_change() {
        let mut s = ParameterStore::default();
        let changed = s.observe_regime("Ranging");
        assert!(!changed, "ilk gözlem değişim sayılmamalı");
        assert_eq!(s.last_observed_regime.as_deref(), Some("Ranging"));
        // Patch yazılmamış olmalı (ilk gözlem)
        assert!(s.regime_overrides.is_empty());
    }

    #[test]
    fn observe_regime_change_triggers_tighten_and_reports_true() {
        let mut s = ParameterStore::default();
        s.observe_regime_with_now("Ranging", 1000); // ilk gözlem, seed
        // Hysteresis: yeni rejim için ardışık 3 cycle gerek.
        assert!(!s.observe_regime_with_now("HighVolatility", 1001), "1. tur henüz drift sayılmaz");
        assert!(!s.observe_regime_with_now("HighVolatility", 1002), "2. tur henüz drift sayılmaz");
        let changed = s.observe_regime_with_now("HighVolatility", 1003);
        assert!(changed, "3. ardışık görüşte drift confirmed olmalı");
        let patch = s.regime_overrides.get("HighVolatility")
            .expect("HV patch yazılmalı");
        assert!(patch.edge_thresholds.is_some());
        assert!(patch.trade_risk.is_some());
        // Base 0.50 → 0.50 * 0.70 = 0.35
        assert!(patch.trade_risk.unwrap().max_position_size < 0.50);
    }

    #[test]
    fn observe_regime_same_regime_back_to_back_no_tighten() {
        let mut s = ParameterStore::default();
        s.observe_regime_with_now("StrongUptrend", 1000); // seed
        let changed = s.observe_regime_with_now("StrongUptrend", 1001);
        assert!(!changed);
        assert!(s.regime_overrides.is_empty());
    }

    #[test]
    fn observe_regime_oscillation_does_not_drift() {
        // Rejim her tur A↔B arasında salınıyor: hiçbir aday DRIFT_CONFIRMATION_TURNS'a
        // ulaşamaz → drift yok, patch yazılmaz.
        let mut s = ParameterStore::default();
        s.observe_regime_with_now("Ranging", 1000); // seed
        for t in 1..30 {
            let r = if t % 2 == 0 { "HighVolatility" } else { "Ranging" };
            let changed = s.observe_regime_with_now(r, 1000 + t);
            assert!(!changed, "sallanan rejim drift sayılmamalı (t={}): {}", t, r);
        }
        assert!(s.regime_overrides.is_empty(),
            "sallanma sırasında patch yazılmamalı: {:?}", s.regime_overrides);
    }

    #[test]
    fn observe_regime_cooldown_suppresses_back_to_back_drifts() {
        // İlk drift confirmed olsun.
        let mut s = ParameterStore::default();
        s.observe_regime_with_now("Ranging", 1000); // seed
        for t in 1..=3 { s.observe_regime_with_now("HighVolatility", 1000 + t); }
        assert_eq!(s.last_observed_regime.as_deref(), Some("HighVolatility"));
        let first_drift_at = s.last_drift_at_secs;
        assert!(first_drift_at >= 1003);
        let patches_after_first = s.regime_overrides.len();

        // Hemen ardından yeni bir rejime geçiş — cooldown içinde olduğu için drift
        // confirmed olmaz, sayım toplansa bile tighten/log yok.
        for t in 4..=10 {
            let changed = s.observe_regime_with_now("StrongUptrend", 1000 + t);
            assert!(!changed, "cooldown içinde drift bastırılmalı (t={})", t);
        }
        assert_eq!(s.regime_overrides.len(), patches_after_first,
            "cooldown içinde yeni patch yazılmamalı");

        // Cooldown sonrası bir tur daha → confirmed.
        let after_cd = first_drift_at + DRIFT_COOLDOWN_SECS + 1;
        let changed = s.observe_regime_with_now("StrongUptrend", after_cd);
        assert!(changed, "cooldown bittikten sonra aday onaylanmalı");
        assert!(s.regime_overrides.contains_key("StrongUptrend"));
    }

    #[test]
    fn feedback_record_appends_and_bounds_window() {
        let mut fb = RegimeFeedback::default();
        for i in 0..(RegimeFeedback::WINDOW + 5) {
            fb.record(i as f64);
        }
        assert_eq!(fb.recent_pnl.len(), RegimeFeedback::WINDOW);
        assert_eq!(fb.total_trades, (RegimeFeedback::WINDOW + 5) as u32);
        // Kuyruğun en yeni elemanı son record olmalı (push_back)
        assert_eq!(*fb.recent_pnl.back().unwrap(), (RegimeFeedback::WINDOW + 4) as f64);
    }

    #[test]
    fn feedback_win_rate_counts_only_positive_pnl() {
        let mut fb = RegimeFeedback::default();
        for v in [1.0, -2.0, 0.5, -1.0, 0.0] { fb.record(v); }
        // 5 trade'den 2'si > 0 → win_rate 0.4
        assert!((fb.win_rate() - 0.4).abs() < 1e-9);
    }

    #[test]
    fn apply_feedback_holds_off_until_window_fills() {
        let mut s = ParameterStore::default();
        // İlk 9 kayıp — WINDOW=10 dolmadığı için tighten yok.
        for _ in 0..9 {
            assert!(!s.apply_trade_feedback("Ranging", -1.0));
        }
        assert!(s.regime_overrides.is_empty(),
            "WINDOW dolmadan tighten olmamalı");
    }

    #[test]
    fn apply_feedback_tightens_after_low_winrate() {
        let mut s = ParameterStore::default();
        // 10 trade, 8'i kayıp → win_rate 0.2, eşik 0.40 altında → tighten.
        for v in [-1.0, -1.0, -1.0, 0.5, -1.0, -1.0, -1.0, 0.3, -1.0, -1.0] {
            s.apply_trade_feedback("Ranging", v);
        }
        let patch = s.regime_overrides.get("Ranging").expect("tighten patch yazılmalı");
        let e = patch.edge_thresholds.expect("edge tightened");
        let r = patch.trade_risk.expect("risk tightened");
        // Base 0.20 → 0.20*1.15 = 0.23
        assert!(e.cold > 0.20);
        // Base 0.5 → 0.5*0.70 = 0.35
        assert!(r.max_position_size < 0.5);
        // Base 3.0 → 3.0*0.85 = 2.55
        assert!(r.take_profit_pct < 3.0);
    }

    #[test]
    fn apply_feedback_no_tighten_when_winrate_high() {
        let mut s = ParameterStore::default();
        // 10 trade, 7'si kazanç → win_rate 0.7 > 0.40, tighten yok.
        for v in [1.0, 1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, 1.0] {
            s.apply_trade_feedback("Ranging", v);
        }
        assert!(s.regime_overrides.get("Ranging").is_none(),
            "yüksek win_rate'te patch yazılmamalı");
    }

    #[test]
    fn regime_with_no_override_falls_back_to_base() {
        let s = ParameterStore::default();
        // Hiç override yok → base ile aynı
        let er = s.edge_thresholds_for("Ranging");
        assert_eq!(er.cold, s.edge_thresholds.cold);
        assert_eq!(er.hot,  s.edge_thresholds.hot);
        let tr = s.trade_risk_for("StrongUptrend");
        assert_eq!(tr.take_profit_pct,   s.trade_risk.take_profit_pct);
        assert_eq!(tr.max_position_size, s.trade_risk.max_position_size);
    }

    #[test]
    fn regime_override_only_replaces_specified_fields() {
        let mut s = ParameterStore::default();
        // HighVolatility için sadece edge eşiklerini sıkılaştır; trade_risk patch yok.
        let strict_edges = EdgeThresholds { cold: 0.50, warm: 0.65, hot: 0.80,
            cold_until: 0.05, warm_until: 0.30 };
        s.set_regime_patch("HighVolatility",
            RegimePatch::empty().with_edge(strict_edges));

        // HighVolatility için edge override aktif
        assert!((s.edge_threshold_for("HighVolatility", 0.0)  - 0.50).abs() < 1e-9);
        assert!((s.edge_threshold_for("HighVolatility", 0.99) - 0.80).abs() < 1e-9);
        // Ama trade_risk hâlâ base
        let tr = s.trade_risk_for("HighVolatility");
        assert_eq!(tr.take_profit_pct, 3.0);

        // Patch'siz başka bir rejim base'i kullanır
        assert!((s.edge_threshold_for("Ranging", 0.99) - 0.55).abs() < 1e-9);
    }

    #[test]
    fn regime_trade_risk_override_only_when_set() {
        let mut s = ParameterStore::default();
        // Ranging için pos boyutunu kıs, TP daralt.
        let tight = TradeRiskParams { take_profit_pct: 1.5, stop_loss_pct: 0.8, max_position_size: 0.25 };
        s.set_regime_patch("Ranging", RegimePatch::empty().with_trade_risk(tight));

        let r = s.trade_risk_for("Ranging");
        assert_eq!(r.take_profit_pct,   1.5);
        assert_eq!(r.max_position_size, 0.25);
        // Diğer rejimler base'de kalmalı
        let u = s.trade_risk_for("StrongUptrend");
        assert_eq!(u.take_profit_pct,   3.0);
        assert_eq!(u.max_position_size, 0.5);
    }

    #[test]
    fn from_env_overrides_partial_fill_and_scalp_and_sr() {
        std::env::set_var("PARTIAL_FILL_MAX_SLIPPAGE_PCT", "2.5");
        std::env::set_var("SCALP_SWING_THRESHOLD_MIN",     "15");
        std::env::set_var("SR_UPDATE_EVERY_SECS",          "10");
        let s = ParameterStore::from_env();
        std::env::remove_var("PARTIAL_FILL_MAX_SLIPPAGE_PCT");
        std::env::remove_var("SCALP_SWING_THRESHOLD_MIN");
        std::env::remove_var("SR_UPDATE_EVERY_SECS");
        assert!((s.partial_fill.max_slippage_pct - 2.5).abs() < 1e-9);
        assert_eq!(s.scalp_swing_threshold_min, 15);
        assert_eq!(s.sr_update_every_secs,      10);
        // Diğer alanlar default'ta kalmalı
        assert_eq!(s.partial_fill.overfill_tolerance, 0.001);
    }
}
