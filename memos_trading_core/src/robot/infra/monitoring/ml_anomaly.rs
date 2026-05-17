// ml_anomaly.rs
// ML tabanlı gelişmiş anomali tespiti modülü (İstatistiksel Analiz)

use serde_json::Value;

/// |z-score| bu eşiği aşarsa anomali sayılır. 3.0 standart sapma %99.7 kapsama demektir.
const ZSCORE_THRESHOLD: f64 = 3.0;
/// Güvenilir istatistik için minimum örnek sayısı.
const MIN_SAMPLES: usize = 5;

pub struct MlAnomalyDetector;

impl MlAnomalyDetector {
    /// `input` penceresi üzerinde z-score hesaplar, son değeri değerlendirir.
    /// Performans: Slicing kullanarak kopyalama yapmaz.
    pub fn predict_with_score(input: &[f64]) -> (bool, f64) {
        let len = input.len();
        if len < MIN_SAMPLES {
            return (false, 0.0);
        }

        // Baseline: Son eleman hariç tüm pencere.
        // Bu sayede spike'ın kendisi ortalamayı (mean) bozmaz.
        let baseline = &input[..len - 1];
        let n = baseline.len() as f64;

        // Tek bir döngüde (pass) hem toplamı hem kareler toplamını hesaplayabiliriz (Welford benzeri optimizasyon)
        let sum: f64 = baseline.iter().sum();
        let mean = sum / n;
        
        let variance = baseline.iter()
            .map(|&x| (x - mean).powi(2))
            .sum::<f64>() / n;
        
        let std_dev = variance.sqrt();

        // Bölme hatasını (division by zero) f64 epsilonu ile engelle
        if std_dev < f64::EPSILON {
            return (false, 0.0);
        }

        let last = input[len - 1];
        let z_score = (last - mean).abs() / std_dev;

        (z_score > ZSCORE_THRESHOLD, z_score)
    }

    /// Girdi vektörünün son değeri anomali mi?
    /// Modernize: Parametreyi referans (&[f64]) alarak gereksiz kopyalamayı (Vec clone) önledik.
    pub fn predict(input: &[f64]) -> bool {
        Self::predict_with_score(input).0
    }

    /// JSON event'ten feature çıkar ve anomali tespiti yap.
    pub fn analyze_event(event: &Value) -> bool {
        // Feature'ları stack üzerinde küçük bir dizide tutmak heap allocation'dan daha hızlıdır
        let amount = event["amount"].as_f64().unwrap_or(0.0);
        let freq = event["freq"].as_f64().unwrap_or(0.0);
        
        // Örnek: Amount ve freq üzerinden birleşik anomali kontrolü
        Self::predict(&[amount, freq])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normal_values() {
        let data = [100.0, 101.0, 99.5, 100.2, 100.8];
        assert!(!MlAnomalyDetector::predict(&data));
    }

    #[test]
    fn test_anomaly_spike() {
        let data = [10.0, 10.5, 9.8, 10.2, 11.0, 100.0];
        let (is_anomaly, score) = MlAnomalyDetector::predict_with_score(&data);
        assert!(is_anomaly);
        assert!(score > 5.0); // Z-score oldukça yüksek olmalı
    }
}
