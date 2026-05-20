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
}

impl Default for ParameterStore {
    fn default() -> Self {
        Self { edge_thresholds: EdgeThresholds::default() }
    }
}

impl ParameterStore {
    /// Boot anında çağrılır: önce Default, sonra ENV override'ları.
    /// Tanınan env değişkenleri:
    ///   EDGE_THRESHOLD_COLD, EDGE_THRESHOLD_WARM, EDGE_THRESHOLD_HOT
    ///   EDGE_COLD_UNTIL, EDGE_WARM_UNTIL
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
        store
    }

    /// `EdgeThresholds::for_confidence`'in kestirme erişim noktası.
    /// Engine cycle'ları ParameterStore tutarken bu method'a doğrudan ulaşır.
    pub fn edge_threshold(&self, ml_confidence: f64) -> f64 {
        self.edge_thresholds.for_confidence(ml_confidence)
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
}
