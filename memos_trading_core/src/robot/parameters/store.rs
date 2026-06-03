//! Merkezi ParameterStore + dinamik parametre çözümleme (edge / risk / leverage /
//! rejim-patch / feedback / trail / interval). parameters/mod.rs'ten ayrıldı
//! (Faz 2 modülerleştirme; davranış birebir korundu).

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use super::types::*;
use super::symbol_stats::SymbolStats;
use super::trail_feedback::{TrailFeedback, PendingTrailObservation};
use super::{parse_env_f64, parse_env_bool, now_epoch_secs};

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
    /// Per-sembol otonom trading interval (TF). Key = sembol, value = "15m"/"1h"...
    /// Değerlendirme job'ı (jobs.rs) aday TF'ler arası walk-forward A/B ile doldurur;
    /// cycle dispatch + download per-sembol bu TF'i kullanır. Boş ise tüm semboller
    /// `config.interval`'e düşer (sıfır regresyon). [[project_adaptive_regime]].
    #[serde(default)]
    pub symbol_interval: HashMap<String, String>,
    /// Per-sembol otonom STRATEJİ. Key = sembol, value = kanonik strateji adı
    /// ("ICT_COMPOSITE"/"MA_CROSSOVER"...). edge_scan (offline survey, robust-filtreli seed)
    /// + backtest job per-symbol WF/pooled-PF A/B'si doldurur. Canlı cycle `strategy_for` ile
    /// per-symbol okur (precedence: symbol_strategy > global live_strategy > auto select_best).
    /// Boş → mevcut davranış (global/auto), sıfır regresyon. [[project_edge_scan]].
    #[serde(default)]
    pub symbol_strategy: HashMap<String, String>,
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
            symbol_interval: HashMap::new(),
            symbol_strategy: HashMap::new(),
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
        // edge_scan SEED (Part 3): EDGE_SEED_REPORT bir edge_sweep JSON'una işaret ederse,
        // robustluk barını (EDGE_SEED_MIN_TRADES/EDGE_SEED_MIN_PF) geçen sembol→strateji
        // adayları symbol_strategy'ye PRIOR olarak yüklenir. Online backtest job sonra doğrular/
        // üzerine yazar. Boş/yok → no-op (sıfır regresyon). [[project_edge_scan]].
        if let Some(path) = std::env::var("EDGE_SEED_REPORT").ok().filter(|s| !s.trim().is_empty()) {
            let r = crate::robot::backtester::SeedRobustness {
                min_trades: std::env::var("EDGE_SEED_MIN_TRADES").ok()
                    .and_then(|v| v.parse().ok()).unwrap_or(30),
                min_pf: parse_env_f64("EDGE_SEED_MIN_PF").unwrap_or(1.2),
                // ÜST sanity cap: PF>max_pf fluke (illikit-alt fat-tail) → elenir. Default 25.0;
                // EDGE_SEED_MAX_PF ile gevşet (devre dışı için çok büyük değer).
                max_pf: parse_env_f64("EDGE_SEED_MAX_PF").unwrap_or(25.0),
                // Default: yalnız WF-onaylı seed (EDGE_SEED_REQUIRE_WF=0 ile gevşet).
                require_wf_robust: !matches!(
                    std::env::var("EDGE_SEED_REQUIRE_WF").ok().as_deref(),
                    Some("0") | Some("false") | Some("off")),
                // MAJÖR (likidite) tabanı: günlük quote-volume bunun altındaki illikit-alt seri
                // seed'lenmez (canlı feed'de purge edilen MYX/SIREN tipi). Default 0.0=kapalı;
                // EDGE_SEED_MIN_QVOL (USDT/gün) + taze rapor (avg_daily_quote_volume'lı) ile aktive.
                min_daily_quote_volume: parse_env_f64("EDGE_SEED_MIN_QVOL").unwrap_or(0.0),
            };
            // Fix A: seed (TF, strateji) ÇİFTİni taşır → strateji DOĞRU TF'de koşar (BB'yi 1m'de
            // değil 1d'de). symbol_interval + symbol_strategy birlikte yüklenir. [[project_edge_scan]].
            let seed = crate::robot::backtester::seed_symbol_plan_from_file(&path, r);
            // Item #1: MARKET-UYUMU. config.market = env_or("TRADE_MARKET","spot") (tek-kaynak);
            // engine yalnız o markette işlem açar → başka marketin edge'i (spot ALPACAUSDT'yi
            // futures engine'e) seed edilmemeli (sembol orada işlem görmeyebilir/farklı enstrüman).
            // EDGE_SEED_IGNORE_MARKET=1 ile devre dışı (operatör çapraz-market seed isterse).
            let engine_market = crate::core::env::env_or("TRADE_MARKET", "spot");
            let ignore_market = matches!(
                std::env::var("EDGE_SEED_IGNORE_MARKET").ok().as_deref(),
                Some("1") | Some("true") | Some("on"));
            let mut loaded = 0usize;
            let mut skipped_market = 0usize;
            for (sym, entry) in &seed {
                if !ignore_market && !entry.market.eq_ignore_ascii_case(&engine_market) {
                    skipped_market += 1;
                    continue;
                }
                store.symbol_interval.insert(sym.clone(), entry.interval.clone());
                store.symbol_strategy.insert(sym.clone(), entry.strategy.clone());
                loaded += 1;
            }
            if loaded > 0 || skipped_market > 0 {
                log::info!(
                    "🌱 edge seed: {} sembol (market={}, interval+strateji) yüklendi, {} market-uyumsuz elendi ({})",
                    loaded, engine_market, skipped_market, path);
            }
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

    /// Bu rejimde rejim-yön disiplini uygulanmalı mı? Otonom per-rejim policy varsa onu
    /// (değerlendirme job'ı backtest A/B ile doldurur), yoksa `fallback` (RuntimeTuning
    /// env'i). Sparse: policy yoksa/None ise fallback → sıfır regresyon.
    pub fn regime_directional_for(&self, regime: &str, fallback: bool) -> bool {
        self.regime_overrides.get(regime)
            .and_then(|p| p.policy)
            .and_then(|pol| pol.regime_directional)
            .unwrap_or(fallback)
    }

    /// Bu sembol için otonom seçilmiş trading interval; yoksa `fallback` (config.interval).
    /// `symbol_interval` map'i değerlendirme job'ı doldurur; boş → tüm semboller fallback
    /// (sıfır regresyon). Cycle dispatch + download tek-nokta bunu çağırır.
    pub fn interval_for(&self, symbol: &str, fallback: &str) -> String {
        self.symbol_interval.get(symbol).cloned().unwrap_or_else(|| fallback.to_string())
    }

    /// Bu sembol için otonom seçilmiş STRATEJİ; yoksa `None` (çağıran global/auto'ya düşer).
    /// `symbol_strategy` map'i edge_scan seed + backtest job WF/PF A/B'si doldurur. Boş değer
    /// (whitespace) güvenli şekilde None sayılır. Canlı cycle precedence'in 1. basamağı.
    pub fn strategy_for(&self, symbol: &str) -> Option<String> {
        self.symbol_strategy.get(symbol)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
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
        self.symbol_stats.retain(|_, s| super::symbol_stats::is_fresh(s, ttl_secs));
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
    /// Rejim-agnostik thin delege (geriye-uyum); rejim-farkında call site'lar
    /// `target_trail_pct_resolved`'i kullanmalı.
    pub fn target_trail_pct_for_strategy_and_symbol(&self, symbol: &str, strategy_name: &str) -> f64 {
        self.target_trail_pct_resolved(symbol, strategy_name, None)
    }

    /// İlgili rejim için otonom backtest A/B'siyle yazılmış trailing hedefi (varsa).
    /// `regime_overrides[regime].target_trail_pct`; yoksa None → çözümleme strateji
    /// default'una düşer (sıfır regresyon).
    pub fn regime_trail_target_for(&self, regime: &str) -> Option<f64> {
        self.regime_overrides.get(regime).and_then(|p| p.target_trail_pct)
    }

    /// Trail target çözümleme — tam precedence (rejim-farkında).
    ///   1) `TARGET_TRAIL_PCT` env — operatör global override
    ///   2) `trail_feedback[(sym, strategy)].target_override` — runtime online feedback
    ///   3) `regime_overrides[regime].target_trail_pct` — per-rejim backtest A/B (Option B)
    ///   4) `strategy_trail_targets[strategy_name]` — Phase B sensible default
    ///   5) `strategy_trail_targets["default"]` — bilinmeyen strateji fallback
    ///   6) Hard-coded 0.7 — store boşsa
    ///
    /// Online feedback (2) rejim A/B'sinin (3) üstünde: canlı gözlem, offline-ölçülmüş
    /// rejim taban-çizgisini ezer. Rejim A/B static strateji default'unun (4) üstünde.
    pub fn target_trail_pct_resolved(
        &self, symbol: &str, strategy_name: &str, regime: Option<&str>,
    ) -> f64 {
        if let Some(v) = parse_env_f64("TARGET_TRAIL_PCT") {
            return v;
        }
        if let Some(fb) = self.trail_feedback.get(&(symbol.to_string(), strategy_name.to_string())) {
            if let Some(override_pct) = fb.target_override {
                return override_pct;
            }
        }
        if let Some(r) = regime {
            if let Some(t) = self.regime_trail_target_for(r) {
                return t;
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
            .or_default();
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

    /// Rejim-agnostik thin delege (geriye-uyum). Yeni call site'lar rejim bağlamını
    /// geçen `resolve_atr_mult_for_regime`'i kullanmalı.
    pub fn resolve_atr_mult(
        &self,
        symbol: &str,
        interval: &str,
        strategy_name: &str,
        default_mult: f64,
    ) -> f64 {
        self.resolve_atr_mult_for_regime(symbol, interval, strategy_name, default_mult, None)
    }

    /// ATR-trail multiplier çözümleme — rejim-farkında. Taze symbol_stats varsa
    /// `mult = target_trail_pct / noise_floor_pct` (per-sembol mikro-yapı korunur);
    /// `target_trail_pct` precedence rejim A/B'sini içerir (`target_trail_pct_resolved`).
    /// Stats yok/stale/yetersiz → `default_mult`.
    pub fn resolve_atr_mult_for_regime(
        &self,
        symbol: &str,
        interval: &str,
        strategy_name: &str,
        default_mult: f64,
        regime: Option<&str>,
    ) -> f64 {
        const TTL_SECS: u64 = 21_600; // 6 saat
        const MIN_MULT: f64 = 1.5;
        const MAX_MULT: f64 = 30.0;
        const MIN_SAMPLE: usize = 50;

        if let Some(s) = self.symbol_stats.get(&(symbol.to_string(), interval.to_string())) {
            if s.sample_size >= MIN_SAMPLE
                && s.noise_floor_pct > 0.0
                && super::symbol_stats::is_fresh(s, TTL_SECS)
            {
                // Phase C feedback patch'i + per-rejim A/B hedefi dahil.
                let target = self.target_trail_pct_resolved(symbol, strategy_name, regime);
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
            .or_default();
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
