use super::feature_extractor::FeatureVector;
use serde::{Deserialize, Serialize};

pub const N_FEATURES: usize = 19;

/// Linear Regression modeli — 19 öznitelik
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinearRegressor {
    pub weights: [f64; N_FEATURES],
    pub bias: f64,
    pub is_trained: bool,
}

/// Model tahmini
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prediction {
    pub score: f64,      // -1.0 .. +1.0  (negatif=sat, pozitif=al)
    pub confidence: f64, // 0.0 .. 1.0
    pub strength: f64,   // 0.0 .. 1.0
}

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
    pub fn new() -> Self {
        Self { weights: [0.1 / N_FEATURES as f64; N_FEATURES], bias: 0.0, is_trained: false }
    }

    /// Domain-knowledge ağırlıkları (pre-trained başlangıç noktası)
    pub fn with_defaults() -> Self {
        let mut r = Self::new();
        r.weights = [
            // ── Orijinal 13 ────────────────────────────────────────────────
            0.13,  // RSI            — overbought/oversold sinyal
            0.10,  // MACD           — momentum yönü
            0.08,  // MACD Signal    — crossover teyidi
            0.06,  // BB Upper       — direnç bağlamı
            0.06,  // BB Lower       — destek bağlamı
            0.04,  // BB Middle      — dinamik ortalama
            0.10,  // SMA 5          — kısa trend
            0.08,  // SMA 10         — orta trend
            0.06,  // SMA 20         — uzun trend
            0.06,  // Momentum       — fiyat ivmesi
            0.04,  // Volatility     — gürültü filtresi
            0.04,  // Volume Change  — hacim teyidi
            0.03,  // Price Change % — anlık hareket
            // ── Yeni 6 ─────────────────────────────────────────────────────
            0.07,  // ATR%           — volatilite bağlamı (pozisyon boyutu)
            0.06,  // ADX            — trend gücü (range vs trend)
            0.07,  // OBV Trend      — hacim yönü teyidi
            0.08,  // BB %B          — bant içi konum (mean-reversion)
            0.06,  // ROC-10         — orta vadeli momentum
            0.04,  // Vol SMA Ratio  — hacim anomali tespiti
        ];
        r.bias = 0.0;
        r.is_trained = true;
        r
    }

    pub fn predict(&self, features: &FeatureVector) -> Prediction {
        let norm  = features.normalize();
        let arr   = norm.to_array();
        let mut raw = self.bias;
        for i in 0..N_FEATURES { raw += self.weights[i] * arr[i]; }
        let score      = (raw / 2.0).tanh();
        let confidence = score.abs();
        let strength   = (1.0 - features.volatility).max(0.0).min(1.0);
        Prediction { score, confidence, strength }
    }

    pub fn predict_score(&self, features: &FeatureVector) -> f64 { self.predict(features).score }

    pub fn predict_batch(&self, list: &[FeatureVector]) -> Vec<Prediction> {
        list.iter().map(|f| self.predict(f)).collect()
    }

    pub fn train_step(&mut self, features: &FeatureVector, target: f64, lr: f64) {
        let norm  = features.normalize();
        let arr   = norm.to_array();
        let err   = target - self.predict(features).score;
        for i in 0..N_FEATURES { self.weights[i] += lr * err * arr[i]; }
        self.bias += lr * err;
    }

    /// Online eğitim — önceden normalize edilmiş [f64; N_FEATURES] dizisiyle doğrudan çalışır.
    /// OpenPosition::entry_features gibi, feature vektörünün açılış anında hesaplanıp saklandığı
    /// durumlar için; kapanışta gerçek PnL ile çağrılır.
    pub fn train_step_raw(&mut self, features: &[f64; N_FEATURES], target: f64, lr: f64) {
        let mut raw = self.bias;
        for i in 0..N_FEATURES { raw += self.weights[i] * features[i]; }
        let pred = (raw / 2.0).tanh();
        let err  = target - pred;
        for i in 0..N_FEATURES { self.weights[i] += lr * err * features[i]; }
        self.bias += lr * err;
        // NaN/Inf koruması: herhangi bir ağırlık bozulursa defaults'a sıfırla
        if self.weights.iter().any(|w| !w.is_finite()) || !self.bias.is_finite() {
            let defaults = Self::with_defaults();
            self.weights = defaults.weights;
            self.bias = defaults.bias;
        }
        self.is_trained = true;
    }

    pub fn train(&mut self, data: &[(FeatureVector, f64)], epochs: usize, lr: f64) {
        for _ in 0..epochs {
            for (f, t) in data { self.train_step(f, *t, lr); }
        }
        self.is_trained = true;
    }

    pub fn evaluate(&self, data: &[(FeatureVector, f64)]) -> f64 {
        if data.is_empty() { return 0.0; }
        let correct = data.iter().filter(|(f, t)| {
            let ps = self.predict(f).score > 0.0;
            let ts = *t > 0.0;
            ps == ts
        }).count();
        correct as f64 / data.len() as f64 * 100.0
    }

    pub fn extract_features(candles: &[crate::types::Candle]) -> super::feature_extractor::FeatureVector {
        super::FeatureExtractor::extract(candles)
    }

    pub fn fit(&mut self, data: &[(super::feature_extractor::FeatureVector, f64)]) {
        self.train(data, 10, 0.01);
    }
}

impl Default for LinearRegressor {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robot::ml_engine::feature_extractor::FeatureExtractor;
    use crate::types::Candle;
    use chrono::Utc;

    fn test_candles() -> Vec<Candle> {
        let mut p = 100.0f64;
        (0..40).map(|i| {
            p += (i as f64 * 0.3) % 2.0 - 0.5;
            Candle { symbol: "BTC".into(), interval: "1h".into(),
                timestamp: Utc::now() + chrono::Duration::hours(i),
                open: p, high: p + 2.0, low: p - 1.0, close: p + 0.5,
                volume: 1000.0 + i as f64 * 50.0 }
        }).collect()
    }

    #[test]
    fn test_predict_range() {
        let r  = LinearRegressor::with_defaults();
        let fv = FeatureExtractor::extract(&test_candles());
        let p  = r.predict(&fv);
        assert!(p.score >= -1.0 && p.score <= 1.0);
        assert!(p.confidence >= 0.0 && p.confidence <= 1.0);
    }

    #[test]
    fn test_weights_len() {
        let r = LinearRegressor::with_defaults();
        assert_eq!(r.weights.len(), N_FEATURES);
    }

    #[test]
    fn test_train_step_changes_weights() {
        let mut r  = LinearRegressor::new();
        let fv     = FeatureExtractor::extract(&test_candles());
        let before = r.weights[0];
        r.train_step(&fv, 1.0, 0.01);
        assert_ne!(r.weights[0], before);
    }

    #[test]
    fn test_serialization() {
        let r    = LinearRegressor::with_defaults();
        let json = serde_json::to_string(&r).unwrap();
        let d: LinearRegressor = serde_json::from_str(&json).unwrap();
        assert_eq!(r.is_trained, d.is_trained);
        assert_eq!(r.weights.len(), d.weights.len());
    }
}
