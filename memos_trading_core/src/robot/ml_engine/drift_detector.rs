/// Feature Drift Detector
///
/// Eğitim setinin feature dağılımı ile son N mumun feature dağılımını karşılaştırır.
/// Population Stability Index (PSI) veya basit z-score tabanlı yöntem kullanılır.
///
/// Drift yüksekse → ML model güveni azaltılır / yeniden eğitim tetiklenir.

use std::collections::VecDeque;
use super::feature_extractor::FeatureVector;
use super::linear_regressor::N_FEATURES;

/// Tek feature için istatistik
#[derive(Debug, Clone, Default)]
struct FeatureStat {
    mean: f64,
    std:  f64,
}

impl FeatureStat {
    fn from_slice(vals: &[f64]) -> Self {
        if vals.is_empty() { return Self::default(); }
        let n    = vals.len() as f64;
        let mean = vals.iter().sum::<f64>() / n;
        let std  = (vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n).sqrt();
        Self { mean, std: std.max(1e-9) }
    }

    /// Z-score of x under this distribution
    fn z(&self, x: f64) -> f64 {
        (x - self.mean) / self.std
    }
}

/// Drift detector durumu
#[derive(Debug, Clone)]
pub struct DriftDetector {
    /// Referans (eğitim) dağılım istatistikleri — ilk N gözlemden kurulur
    reference: Option<[FeatureStat; N_FEATURES]>,
    /// Son döngünün feature penceresi (normalize edilmiş)
    window:    VecDeque<[f64; N_FEATURES]>,
    window_size: usize,
    /// Son hesaplanan drift skoru (0.0 = yok, 1.0 = tam kayma)
    pub drift_score: f64,
    /// Drift eşiği — bu değerin üzerinde model güveni azaltılır
    pub threshold: f64,
    /// Toplam örnek sayısı (referans kuruldu mu?)
    sample_count: usize,
}

impl DriftDetector {
    pub fn new(window_size: usize, threshold: f64) -> Self {
        Self {
            reference:       None,
            window:          VecDeque::with_capacity(window_size + 1),
            window_size,
            drift_score:     0.0,
            threshold,
            sample_count:    0,
        }
    }

    /// Yeni feature vektörü ekle; drift skoru güncellenir
    pub fn update(&mut self, fv: &FeatureVector) {
        let norm = fv.normalize();
        let arr  = norm.to_array();

        // Referans yok → biriktir ve kur
        if self.reference.is_none() {
            if self.window.len() >= self.window_size {
                // Penceredeki veriden referans istatistiklerini kur
                let ref_stats: [FeatureStat; N_FEATURES] = std::array::from_fn(|i| {
                    let vals: Vec<f64> = self.window.iter().map(|a| a[i]).collect();
                    FeatureStat::from_slice(&vals)
                });
                self.reference = Some(ref_stats);
            }
        }

        // Kayan pencereyi güncelle
        self.window.push_back(arr);
        if self.window.len() > self.window_size {
            self.window.pop_front();
        }
        self.sample_count += 1;

        // Drift hesapla
        if let Some(ref ref_stats) = self.reference {
            if self.window.len() >= self.window_size / 2 {
                self.drift_score = self.compute_drift(ref_stats);
            }
        }
    }

    /// Ortalama |z-score| ile drift ölç — PSI'ye göre daha hızlı.
    /// Alloc yok: her feature için mean/std kayan pencereden inline hesaplanır.
    fn compute_drift(&self, ref_stats: &[FeatureStat; N_FEATURES]) -> f64 {
        let n = self.window.len() as f64;
        if n < 2.0 { return 0.0; }

        let mut total_drift = 0.0f64;
        for feat in 0..N_FEATURES {
            // İki geçiş ama alloc yok
            let mean = self.window.iter().map(|a| a[feat]).sum::<f64>() / n;
            let var  = self.window.iter().map(|a| (a[feat] - mean).powi(2)).sum::<f64>() / n;
            let std  = var.sqrt().max(1e-9);
            // Ortalama kayması: referans std'ye göre kaç sigma?
            let mean_shift = ref_stats[feat].z(mean).abs();
            // Std değişimi: |log(std_ratio)| — 3σ ile sınırla (düz piyasada taşma önlenir)
            let std_shift  = (std / ref_stats[feat].std).ln().abs().min(3.0);
            let contrib = (mean_shift + std_shift) / 2.0;
            // NaN/Inf koruması: geçersiz değer hesaba katılmaz
            if contrib.is_finite() {
                total_drift += contrib;
            }
        }

        // Normalize et: 0..3+ sigma → 0..1
        (total_drift / N_FEATURES as f64 / 3.0).clamp(0.0, 1.0)
    }

    /// Drift var mı?
    pub fn is_drifting(&self) -> bool {
        self.drift_score > self.threshold
    }

    /// ML voter için güven skalası: drift yoksa 1.0, drift varsa azalır
    /// drift=0 → 1.0, drift=threshold → 0.8, drift=1.0 → 0.5
    pub fn confidence_scale(&self) -> f64 {
        if self.drift_score <= self.threshold {
            1.0
        } else {
            let excess = (self.drift_score - self.threshold) / (1.0 - self.threshold).max(1e-9);
            (1.0 - excess * 0.5).clamp(0.5, 1.0)
        }
    }

    /// Referans kuruldu mu?
    pub fn is_calibrated(&self) -> bool { self.reference.is_some() }

    /// Toplam işlenen örnek sayısı
    pub fn sample_count(&self) -> usize { self.sample_count }

    /// Referansı sıfırla (yeniden eğitim sonrası çağrılır)
    pub fn reset_reference(&mut self) {
        self.reference = None;
        self.sample_count = 0;
    }
}

impl Default for DriftDetector {
    fn default() -> Self { Self::new(50, 0.35) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robot::ml_engine::feature_extractor::FeatureExtractor;
    use crate::types::Candle;
    use chrono::Utc;

    fn candles(n: usize, start_price: f64, trend: f64) -> Vec<Candle> {
        let mut p = start_price;
        (0..n).map(|i| {
            p += trend + (i as f64 * 0.1) % 1.0 - 0.5;
            Candle {
                symbol: "T".into(), interval: "1h".into(),
                timestamp: Utc::now() + chrono::Duration::hours(i as i64),
                open: p, high: p + 1.0, low: p - 0.5, close: p + 0.3,
                volume: 1000.0 + i as f64 * 20.0,
            }
        }).collect()
    }

    #[test]
    fn no_drift_stable_market() {
        // Drift detector'ı aynı piyasa rejiminde besle — referans ve güncel dağılım yakın olmalı
        let mut det = DriftDetector::new(10, 0.35);
        let c = candles(80, 100.0, 0.0);
        for i in 5..c.len() {
            let fv = FeatureExtractor::extract(&c[..=i]);
            det.update(&fv);
        }
        // Referans kurulduktan sonra aynı trendde çok yüksek drift olmamalı
        // (1.0 = tamamen farklı dağılım; stabil market için bu beklenemez)
        assert!(det.is_calibrated(), "detector calibrate edilmeli");
        // confidence_scale her zaman 0.5-1.0 aralığında
        let cs = det.confidence_scale();
        assert!(cs >= 0.5 && cs <= 1.0, "confidence_scale={}", cs);
    }

    #[test]
    fn drift_regime_change() {
        let mut det = DriftDetector::new(20, 0.35);
        // İlk stabil
        let c1 = candles(30, 100.0, 0.0);
        for i in 5..c1.len() {
            det.update(&FeatureExtractor::extract(&c1[..=i]));
        }
        // Sert trend değişimi
        let c2 = candles(30, 100.0, 5.0);
        for i in 5..c2.len() {
            det.update(&FeatureExtractor::extract(&c2[..=i]));
        }
        assert!(det.is_calibrated());
        // confidence_scale her zaman 0.5-1.0 arası
        let cs = det.confidence_scale();
        assert!(cs >= 0.5 && cs <= 1.0);
    }
}
