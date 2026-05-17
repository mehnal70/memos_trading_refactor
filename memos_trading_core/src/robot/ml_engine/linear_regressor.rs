// robot/ml_engine/linear_regressor.rs - Otonom Online Regresyon ve Tahmin Motoru

use super::feature_extractor::FeatureVector;
use serde::{Deserialize, Serialize};

pub const N_FEATURES: usize = 19;

/// LinearRegressor: 19 özniteliği kullanarak piyasa yönü tahmini yapar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinearRegressor {
    pub weights: [f64; N_FEATURES],
    pub bias: f64,
    pub is_trained: bool,
}

/// Model çıktısı: Karar, Güven ve Sinyal Şiddeti bir arada.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prediction {
    pub score: f64,      // -1.0 .. +1.0  (Negatif=Sat, Pozitif=Al)
    pub confidence: f64, // 0.0 .. 1.0    (Tahmin Tutarlılığı)
    pub strength: f64,   // 0.0 .. 1.0    (Piyasa Volatilite Filtresi)
}

// Tahminler üzerinde matematiksel karşılaştırma ve kümülatif toplama desteği
impl PartialEq<f64> for Prediction {
    fn eq(&self, other: &f64) -> bool { (self.score - other).abs() < f64::EPSILON }
}
impl PartialOrd<f64> for Prediction {
    fn partial_cmp(&self, other: &f64) -> Option<std::cmp::Ordering> { self.score.partial_cmp(other) }
    fn lt(&self, other: &f64) -> bool { self.score < *other }
    fn gt(&self, other: &f64) -> bool { self.score > *other }
}
impl std::iter::Sum for Prediction {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        Prediction { score: iter.map(|p| p.score).sum(), confidence: 0.0, strength: 0.0 }
    }
}

impl LinearRegressor {
    /// Sıfırdan nötr ağırlıklı model oluşturur.
    pub fn new() -> Self {
        Self { weights: [0.1 / N_FEATURES as f64; N_FEATURES], bias: 0.0, is_trained: false }
    }

    /// Otonom Başlangıç: Önceden tanımlanmış piyasa uzmanlığı ağırlıkları.
    pub fn with_defaults() -> Self {
        let mut r = Self::new();
        r.weights = [
            0.13, 0.10, 0.08, 0.06, 0.06, 0.04, 0.10, 0.08, 0.06, 0.06, 0.04, 0.04, 0.03, // Orijinal 13
            0.07, 0.06, 0.07, 0.08, 0.06, 0.04, // Yeni 6 (ATR, ADX, OBV, %B, ROC, VolRatio)
        ];
        r.bias = 0.0;
        r.is_trained = true;
        r
    }

    /// FeatureVector'ı alır ve Tanh aktivasyonu ile tahmin üretir.
    pub fn predict(&self, features: &FeatureVector) -> Prediction {
        let arr = features.normalize().to_array();
        let mut raw = self.bias;
        for i in 0..N_FEATURES { raw += self.weights[i] * arr[i]; }
        
        let score      = (raw / 2.0).tanh();
        let confidence = score.abs();
        // Volatilite arttıkça sinyal gücünü törpüle (Otonom Güvenlik)
        let strength   = (1.0 - features.volatility).clamp(0.0, 1.0);
        
        Prediction { score, confidence, strength }
    }

    pub fn predict_score(&self, features: &FeatureVector) -> f64 { self.predict(features).score }

    /// Online Eğitim (Tek Adım): Hata payına göre ağırlıkları günceller.
    pub fn train_step_raw(&mut self, features: &[f64; N_FEATURES], target: f64, lr: f64) {
        let mut raw = self.bias;
        for i in 0..N_FEATURES { raw += self.weights[i] * features[i]; }
        let pred = (raw / 2.0).tanh();
        let err  = target - pred;

        // Gradyan İnişi (Gradient Descent)
        for i in 0..N_FEATURES { self.weights[i] += lr * err * features[i]; }
        self.bias += lr * err;

        // Otonom Öz-İyileştirme: Sayısal kararlılık bozulursa resetle
        if self.weights.iter().any(|w| !w.is_finite()) || !self.bias.is_finite() {
            let defaults = Self::with_defaults();
            self.weights = defaults.weights;
            self.bias = defaults.bias;
        }
        self.is_trained = true;
    }

    /// Batch Eğitim: Geçmiş veriler üzerinden modeli optimize eder.
    pub fn train(&mut self, data: &[(FeatureVector, f64)], epochs: usize, lr: f64) {
        for _ in 0..epochs {
            for (f, t) in data {
                let arr = f.normalize().to_array();
                self.train_step_raw(&arr, *t, lr);
            }
        }
        self.is_trained = true;
    }

    /// Modelin doğruluğunu (tahmin yönü bazlı) otonom ölçer.
    pub fn evaluate(&self, data: &[(FeatureVector, f64)]) -> f64 {
        if data.is_empty() { return 0.0; }
        let correct = data.iter().filter(|(f, t)| {
            (self.predict(f).score > 0.0) == (*t > 0.0)
        }).count();
        (correct as f64 / data.len() as f64) * 100.0
    }
}

impl Default for LinearRegressor {
    fn default() -> Self { Self::with_defaults() }
}
