// ml_anomaly.rs
// ML tabanlı gelişmiş anomali tespiti modülü (istatistiksel, saf Rust)
// Z-score tabanlı gerçek zamanlı anomali tespiti — ONNX gerektirmez.
// Türkçe açıklamalar ile

use serde_json::Value;

/// |z-score| bu eşiği aşarsa anomali sayılır
const ZSCORE_THRESHOLD: f64 = 3.0;
/// Güvenilir istatistik için minimum örnek sayısı
const MIN_SAMPLES: usize = 5;

pub struct MlAnomalyDetector;

impl MlAnomalyDetector {
    /// `input` penceresi üzerinde z-score hesaplar, son değeri değerlendirir.
    /// Dönüş: (anomali mi, z-score değeri)
    pub fn predict_with_score(input: &[f64]) -> (bool, f64) {
        if input.len() < MIN_SAMPLES {
            return (false, 0.0);
        }
        // Baseline: son eleman hariç tüm pencere — spike kendisini mean'e dahil etmesin
        let baseline = &input[..input.len() - 1];
        let n = baseline.len() as f64;
        let mean = baseline.iter().sum::<f64>() / n;
        let variance = baseline.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
        let std_dev = variance.sqrt();
        if std_dev < f64::EPSILON {
            return (false, 0.0);
        }
        let last = input[input.len() - 1];
        let z = (last - mean).abs() / std_dev;
        (z > ZSCORE_THRESHOLD, z)
    }

    /// Girdi vektörünün son değeri anomali mi?
    pub fn predict(input: Vec<f64>) -> bool {
        Self::predict_with_score(&input).0
    }

    /// JSON event'ten feature çıkar, z-score ile anomali tespiti yap.
    pub fn analyze_event(event: &Value) -> bool {
        let features = extract_features(event);
        Self::predict(features)
    }
}

fn extract_features(event: &Value) -> Vec<f64> {
    let amount = event["amount"].as_f64().unwrap_or(0.0);
    let freq   = event["freq"].as_f64().unwrap_or(0.0);
    vec![amount, freq]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_values_no_anomaly() {
        let data = vec![100.0, 101.0, 99.5, 100.2, 100.8];
        assert!(!MlAnomalyDetector::predict(data));
    }

    #[test]
    fn spike_detected() {
        let data = vec![100.0, 101.0, 99.5, 100.2, 100.8, 500.0];
        assert!(MlAnomalyDetector::predict(data));
    }
}
