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

use serde::{Deserialize, Serialize};

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
}

impl Default for ParameterStore {
    fn default() -> Self {
        Self {
            edge_thresholds: EdgeThresholds::default(),
            partial_fill:    PartialFillParams::default(),
            trade_risk:      TradeRiskParams::default(),
            scalp_swing_threshold_min: 60,
            sr_update_every_secs:      30,
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
