use crate::types::{Candle, Signal};
use crate::robot::ml_engine::{FeatureExtractor, FeatureVector, LinearRegressor, Prediction};
use crate::Result;
use crate::MemosTradingError;
use serde::{Deserialize, Serialize};

/// ML-tabanlı sinyal tahmini
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MLSignalPrediction {
    pub signal: Signal,
    pub confidence: f64,      // 0.0 to 1.0
    pub ml_score: f64,        // -1.0 to 1.0
    pub feature_importance: FeatureImportance,
}

/// Feature önem derecesi
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureImportance {
    pub rsi_impact: f64,
    pub macd_impact: f64,
    pub bollinger_impact: f64,
    pub sma_impact: f64,
    pub momentum_impact: f64,
    pub volatility_impact: f64,
}

/// ML-based signal predictor
pub struct MLSignalPredictor {
    model: LinearRegressor,
    confidence_threshold: f64,  // Sinyali vermek için minimum confidence
}

impl MLSignalPredictor {
    /// Yeni predictor oluştur
    pub fn new(confidence_threshold: f64) -> Self {
        Self {
            model: LinearRegressor::with_defaults(),
            confidence_threshold: confidence_threshold.min(1.0).max(0.0),
        }
    }

    /// Mum'lardan sinyal tahmin et
    pub fn predict(&self, candles: &[Candle]) -> Result<MLSignalPrediction> {
        if candles.is_empty() {
            return Err(MemosTradingError::Strategy(
                "Hiç mum verisi sağlanmadı".to_string(),
            ));
        }

        // Özellikleri çıkar
        let features = FeatureExtractor::extract(candles);
        
        // Model tahmini yap
        let prediction = self.model.predict(&features);

        // Feature importance hesapla
        let importance = self.calculate_feature_importance(&features, &prediction);

        // Sinyal oluştur
        let signal = if prediction.score > self.confidence_threshold {
            Signal::Buy
        } else if prediction.score < -self.confidence_threshold {
            Signal::Sell
        } else {
            Signal::Hold
        };

        Ok(MLSignalPrediction {
            signal,
            confidence: prediction.confidence,
            ml_score: prediction.score,
            feature_importance: importance,
        })
    }

    /// Batch tahmin
    pub fn predict_batch(&self, candles_list: &[Vec<Candle>]) -> Result<Vec<MLSignalPrediction>> {
        let mut predictions = Vec::new();
        for candles in candles_list {
            predictions.push(self.predict(candles)?);
        }
        Ok(predictions)
    }

    /// Model'i eğit
    pub fn train(&mut self, training_data: &[(Vec<Candle>, f64)]) -> Result<f64> {
        let mut data = Vec::new();
        
        for (candles, target) in training_data {
            let features = FeatureExtractor::extract(candles);
            data.push((features, *target));
        }

        if data.is_empty() {
            return Err(MemosTradingError::Strategy(
                "Eğitim verisi boş".to_string(),
            ));
        }

        // Training/Test split
        let split = (data.len() as f64 * 0.8) as usize;
        let train_data = &data[..split];
        let test_data = &data[split..];

        // Model eğit
        self.model.train(train_data, 10, 0.01);

        // Accuracy hesapla
        let accuracy = self.model.evaluate(test_data);

        Ok(accuracy)
    }

    /// Feature importance hesapla
    fn calculate_feature_importance(
        &self,
        features: &FeatureVector,
        _prediction: &Prediction,
    ) -> FeatureImportance {
        let normalized = features.normalize();

        // Normalized features'e göre impact hesapla
        let rsi_impact = (normalized.rsi - 0.5).abs() * self.model.weights[0];
        let macd_impact = (normalized.macd - 0.5).abs() * self.model.weights[1];
        let bb_upper_impact = (normalized.bb_upper - 0.5).abs() * self.model.weights[3];
        let bb_lower_impact = (normalized.bb_lower - 0.5).abs() * self.model.weights[4];
        let bollinger_impact = (bb_upper_impact + bb_lower_impact) / 2.0;
        let sma_impact = ((normalized.sma_5 - 0.5).abs() +
                         (normalized.sma_10 - 0.5).abs() +
                         (normalized.sma_20 - 0.5).abs()) / 3.0;
        let momentum_impact = (normalized.momentum - 0.5).abs() * self.model.weights[9];
        let volatility_impact = normalized.volatility * self.model.weights[10];

        FeatureImportance {
            rsi_impact,
            macd_impact,
            bollinger_impact,
            sma_impact,
            momentum_impact,
            volatility_impact,
        }
    }

    /// Threshold'u ayarla
    pub fn set_confidence_threshold(&mut self, threshold: f64) {
        self.confidence_threshold = threshold.min(1.0).max(0.0);
    }

    /// Model'i döndür (immutable reference)
    pub fn get_model(&self) -> &LinearRegressor {
        &self.model
    }

    /// Model'i döndür (mutable reference)
    pub fn get_model_mut(&mut self) -> &mut LinearRegressor {
        &mut self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn create_test_candles() -> Vec<Candle> {
        let mut candles = Vec::new();
        let mut price = 100.0;

        for i in 0..30 {
            candles.push(Candle {
                symbol: "BTC".to_string(),
                interval: "1h".to_string(),
                timestamp: Utc::now() + chrono::Duration::hours(i),
                open: price,
                high: price + 2.0,
                low: price - 1.0,
                close: price + 0.5,
                volume: 1000.0 + (i as f64 * 50.0),
            });

            price += (i as f64 * 0.3) % 2.0 - 0.5;
        }

        candles
    }

    #[test]
    fn test_ml_signal_predictor_creation() {
        let predictor = MLSignalPredictor::new(0.5);
        assert_eq!(predictor.confidence_threshold, 0.5);
    }

    #[test]
    fn test_ml_signal_predict() {
        let predictor = MLSignalPredictor::new(0.3);
        let candles = create_test_candles();
        let result = predictor.predict(&candles);

        assert!(result.is_ok());

        if let Ok(prediction) = result {
            assert!(matches!(prediction.signal, Signal::Buy | Signal::Sell | Signal::Hold));
            assert!(prediction.confidence >= 0.0 && prediction.confidence <= 1.0);
            assert!(prediction.ml_score >= -1.0 && prediction.ml_score <= 1.0);
        }
    }

    #[test]
    fn test_ml_signal_predict_empty_candles() {
        let predictor = MLSignalPredictor::new(0.5);
        let result = predictor.predict(&[]);

        assert!(result.is_err());
    }

    #[test]
    fn test_ml_signal_predict_batch() {
        let predictor = MLSignalPredictor::new(0.3);
        let candles = create_test_candles();
        let candles_list = vec![candles.clone(), candles.clone()];

        let result = predictor.predict_batch(&candles_list);

        assert!(result.is_ok());

        if let Ok(predictions) = result {
            assert_eq!(predictions.len(), 2);
        }
    }

    #[test]
    fn test_ml_signal_threshold_adjustment() {
        let mut predictor = MLSignalPredictor::new(0.5);
        predictor.set_confidence_threshold(0.8);
        assert_eq!(predictor.confidence_threshold, 0.8);

        // Out of bounds should be clamped
        predictor.set_confidence_threshold(1.5);
        assert_eq!(predictor.confidence_threshold, 1.0);
    }

    #[test]
    fn test_feature_importance_calculation() {
        let predictor = MLSignalPredictor::new(0.3);
        let candles = create_test_candles();
        
        if let Ok(prediction) = predictor.predict(&candles) {
            assert!(prediction.feature_importance.rsi_impact >= 0.0);
            assert!(prediction.feature_importance.macd_impact >= 0.0);
        }
    }

    #[test]
    fn test_ml_signal_serialization() {
        let predictor = MLSignalPredictor::new(0.5);
        let candles = create_test_candles();

        if let Ok(prediction) = predictor.predict(&candles) {
            let json = serde_json::to_string(&prediction).unwrap();
            let deserialized: MLSignalPrediction = serde_json::from_str(&json).unwrap();

            assert_eq!(
                format!("{:?}", prediction.signal),
                format!("{:?}", deserialized.signal)
            );
        }
    }
}
