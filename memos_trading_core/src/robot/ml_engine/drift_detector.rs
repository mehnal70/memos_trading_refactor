/// Feature Drift Detector
///
/// Eğitim setinin feature dağılımı ile son N mumun feature dağılımını karşılaştırır.
/// Population Stability Index (PSI) veya basit z-score tabanlı yöntem kullanılır.
///
/// Drift yüksekse → ML model güveni azaltılır / yeniden eğitim tetiklenir.
// robot/ml_engine/drift_detector.rs - Otonom Piyasa Kayması (Drift) Denetçisi

use std::collections::VecDeque;
use super::feature_extractor::FeatureVector;
use super::linear_regressor::N_FEATURES;

/// Tek bir öznitelik (feature) için tarihsel istatistik deposu
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

    /// Mevcut değerin tarihsel dağılıma göre Z-skorunu (sapma miktarı) hesaplar
    fn z(&self, x: f64) -> f64 {
        (x - self.mean) / self.std
    }
}

/// DriftDetector: Piyasa rejimindeki istatistiksel kaymaları milisaniyeler içinde yakalar.
#[derive(Debug, Clone)]
pub struct DriftDetector {
    /// Eğitim anındaki referans istatistikler (Piyasanın "normal" hali)
    reference: Option<[FeatureStat; N_FEATURES]>,
    /// Canlı akıştaki son pencere verileri
    window:    VecDeque<[f64; N_FEATURES]>,
    window_size: usize,
    /// Otonom Drift Skoru (0.0: Kararlı, 1.0: Kaotik/Bilinmez)
    pub drift_score: f64,
    /// Müdahale eşiği (Default: 0.35)
    pub threshold: f64,
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

    /// Her yeni mumda piyasa karakteristiğini otonom günceller
    pub fn update(&mut self, fv: &FeatureVector) {
        let arr = fv.normalize().to_array();

        // 1. Referans Noktası Oluşturma (Initial Calibration)
        if self.reference.is_none() && self.window.len() >= self.window_size {
            let ref_stats: [FeatureStat; N_FEATURES] = std::array::from_fn(|i| {
                let vals: Vec<f64> = self.window.iter().map(|a| a[i]).collect();
                FeatureStat::from_slice(&vals)
            });
            self.reference = Some(ref_stats);
        }

        // 2. Kayan Pencere Güncelleme (Bellek Dostu O(1))
        self.window.push_back(arr);
        if self.window.len() > self.window_size {
            self.window.pop_front();
        }
        self.sample_count += 1;

        // 3. Otonom Drift Analizi
        if let Some(ref ref_stats) = self.reference {
            if self.window.len() >= self.window_size / 2 {
                self.drift_score = self.compute_drift(ref_stats);
            }
        }
    }

    /// Bellek tahsisatı yapmadan (Zero-Alloc) inline drift hesabı
    fn compute_drift(&self, ref_stats: &[FeatureStat; N_FEATURES]) -> f64 {
        let n = self.window.len() as f64;
        if n < 2.0 { return 0.0; }

        let mut total_drift = 0.0f64;
        for feat in 0..N_FEATURES {
            let mean = self.window.iter().map(|a| a[feat]).sum::<f64>() / n;
            let var  = self.window.iter().map(|a| (a[feat] - mean).powi(2)).sum::<f64>() / n;
            let std  = var.sqrt().max(1e-9);

            // Ortalama kayması (Sigma) + Oynaklık değişimi (Log-Ratio)
            let mean_shift = ref_stats[feat].z(mean).abs();
            let std_shift  = (std / ref_stats[feat].std).ln().abs().min(3.0);
            
            let contrib = (mean_shift + std_shift) / 2.0;
            if contrib.is_finite() {
                total_drift += contrib;
            }
        }

        // Normalizasyon: 0..3+ sigma -> 0..1 aralığına otonom sıkıştırma
        (total_drift / N_FEATURES as f64 / 3.0).clamp(0.0, 1.0)
    }

    pub fn is_drifting(&self) -> bool { self.drift_score > self.threshold }

    /// ML modellerine ne kadar güvenilmesi gerektiğini söyler.
    /// Drift arttıkça güven otonom azalır.
    pub fn confidence_scale(&self) -> f64 {
        if self.drift_score <= self.threshold { 1.0 } 
        else {
            let excess = (self.drift_score - self.threshold) / (1.0 - self.threshold).max(1e-9);
            (1.0 - excess * 0.5).clamp(0.5, 1.0)
        }
    }

    pub fn is_calibrated(&self) -> bool { self.reference.is_some() }
    pub fn sample_count(&self) -> usize { self.sample_count }
    pub fn reset_reference(&mut self) { self.reference = None; self.sample_count = 0; }
}

impl Default for DriftDetector {
    fn default() -> Self { Self::new(50, 0.35) }
}

