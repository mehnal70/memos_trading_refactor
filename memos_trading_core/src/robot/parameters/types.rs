//! Dinamik parametre tip tanımları — RegimePatch / RegimePolicy / RegimeFeedback
//! ve TradeRisk / Leverage / MultiTf / PartialFill / EdgeThresholds param
//! struct'ları. parameters/mod.rs'ten ayrıldı (Faz 2 modülerleştirme; davranış
//! birebir korundu, dış erişim mod.rs `pub use types::*` ile sürüyor).

use serde::{Deserialize, Serialize};

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
    /// Otonom işlem politikası (yön disiplini vb.) — değerlendirme job'ı backtest
    /// A/B sonucuna göre doldurur, canlı cycle bu rejimde okur. None → env/operatör.
    #[serde(default)]
    pub policy: Option<RegimePolicy>,
    /// Bu rejim için otonom seçilmiş trailing-stop hedef yüzdesi (R/R lever'ı).
    /// Değerlendirme job'ı her rejimin OOS pencerelerinde target_trail_pct A/B'si
    /// koşup kazananı buraya yazar; canlı `target_trail_pct_for_strategy_and_symbol`
    /// noise-floor formülünün numerator'ında okur (per-sembol mikro-yapı korunur).
    /// None → strateji default'una düş (sıfır regresyon). [[project_runtime_observations]].
    #[serde(default)]
    pub target_trail_pct: Option<f64>,
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

    pub fn with_policy(mut self, p: RegimePolicy) -> Self {
        self.policy = Some(p);
        self
    }

    pub fn with_trail_target(mut self, pct: f64) -> Self {
        self.target_trail_pct = Some(pct);
        self
    }

    /// Patch hiçbir alanı override etmiyor mu? Engine boş patch'leri store'a
    /// koymaktan kaçınmak için bunu kontrol eder.
    pub fn is_empty(&self) -> bool {
        self.edge_thresholds.is_none() && self.trade_risk.is_none()
            && self.policy.is_none() && self.target_trail_pct.is_none()
    }
}

/// Per-rejim otonom işlem politikası. Değerlendirme job'ı (jobs.rs) her rejim için
/// backtest A/B (LongOnly vs RegimeDirectional, OOS) koşup kazananı buraya yazar;
/// canlı cycle o rejimdeyken okur. Tüm alanlar Option → None = env/operatör davranışına
/// düş (geriye-uyum, sıfır regresyon). [[project_adaptive_regime]] [[feedback_autonomy_first]].
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct RegimePolicy {
    /// Bu rejimde rejim-yön disiplini uygulansın mı? (long yalnız non-downtrend, short
    /// yalnız non-uptrend). A/B OOS kazancına göre otonom seçilir. None → env fallback.
    #[serde(default)]
    pub regime_directional: Option<bool>,
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

/// Kesitsel (cross-sectional) relatif-güç ADANMIŞ MOD parametreleri ([[project_xs_momentum]]).
/// `enabled=true` iken sepet sembolleri SADECE kesitsel kitapla (market-nötr long/short) yönetilir;
/// ScalpSwing/seed yalnız sepet-DIŞI sembollerde çalışır → tek-pozisyon/sembol invariantı temiz kalır.
/// Backtest+WF-OOS+Newey-West doğrulamasından gelen edge'in canlı ifadesi. Default DISABLED (opt-in).
/// Skorlama backtest çekirdeğiyle BİT-AYNI (`xs_target_book` → `select_books`, DRY).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XsLiveParams {
    /// Master kapı. False → kesitsel mod kapalı, hiçbir sembol XS-yönetimine girmez (sıfır regresyon).
    pub enabled: bool,
    /// Kesitsel sepet (majör semboller). En az 2*top_k gerekir. Boş → mod fiilen pasif.
    /// Doğrulanmış: 15-majör ya da derin+likit-25 ([[project_xs_momentum]]).
    pub symbols: Vec<String>,
    /// Sinyal TF'i (bar). Doğrulanmış edge 1d → default "1d" (global config.interval'den bağımsız).
    pub interval: String,
    /// Momentum geriye-bakış (bar). WF-OOS'ta 14-30 en iyi; default 14.
    pub lookback: usize,
    /// Sepet kenarı (long k / short k). Doğrulanmış 3-5; default 3.
    pub top_k: usize,
    /// No-trade band (rank-histerezisi): pozisyonu top_k+exit_buffer dışına düşene dek tut. Default 1.
    pub exit_buffer: usize,
    /// true = momentum (en güçlü long); false = reversal. WF 9/9 pencere momentum seçti → default true.
    pub momentum: bool,
}

impl Default for XsLiveParams {
    fn default() -> Self {
        Self {
            enabled: false, // opt-in: XS_LIVE_ENABLED=1 + sepet ile aktive
            symbols: Vec::new(),
            interval: "1d".to_string(),
            lookback: 14,
            top_k: 3,
            exit_buffer: 1,
            momentum: true,
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
