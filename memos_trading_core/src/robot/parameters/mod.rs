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
}

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
        store
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

    /// Belirli bir rejim için patch yerleştirir (HyperOpt rejim-aware tuning sonucu).
    pub fn set_regime_patch(&mut self, regime: impl Into<String>, patch: RegimePatch) {
        self.regime_overrides.insert(regime.into(), patch);
    }

    /// Faz 3 c2: bir trade kapanışında rejim+pnl_pct geri beslemesini işler.
    ///
    /// Kuyruğu günceller (`RegimeFeedback::record`); rejim için WINDOW kadar
    /// trade biriktiyse ve win_rate eşik altına düştüyse patch'i sıkılaştırır:
    ///   - edge_thresholds her katmanı *1.15 (eşiği yükselt → daha az sinyal)
    ///   - trade_risk.max_position_size *0.7 (pozisyonu küçült)
    ///   - take_profit_pct *0.85, stop_loss_pct *0.85 (kısa hedef + dar stop)
    ///
    /// Eşik default 0.40 (10 trade'in en az 4'ü kazançlı olmalı); ileride
    /// HyperOpt'tan ayarlanabilir hale getirilir.
    ///
    /// Patch zaten o rejim için varsa override edilen alanları sıkılaştırır;
    /// yoksa base'in üstüne yeni bir patch yapılır (adaptive heuristic'i
    /// bypass etmez — rejim daha önce ensure_regime_patch ile dolmuş olabilir).
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

        // Sıkılaştırma uygula. Mevcut patch varsa onu temel al; yoksa base'i.
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
        true
    }
}

fn parse_env_f64(key: &str) -> Option<f64> {
    std::env::var(key).ok().and_then(|v| v.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

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
