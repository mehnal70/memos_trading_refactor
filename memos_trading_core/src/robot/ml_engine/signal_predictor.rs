// robot/ml_engine/signal_predictor.rs - Otonom Sinyal Tahmin ve Karar Motoru

use crate::core::types::{Candle, Signal};
use crate::robot::ml_engine::{FeatureExtractor, FeatureVector, LinearRegressor, Prediction};
use crate::Result;
use crate::MemosTradingError;
use serde::{Deserialize, Serialize};

/// ML-tabanlı otonom sinyal tahmini ve güven analizi.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MLSignalPrediction {
    pub signal: Signal,
    pub confidence: f64,      // 0.0 ile 1.0 arası (Modelin kararlılığı)
    pub ml_score: f64,        // -1.0 ile 1.0 arası (Ham karar skoru)
    pub feature_importance: FeatureImportance,
}

/// Özniteliklerin (Features) tahmin üzerindeki otonom etki ağırlıkları.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureImportance {
    pub rsi_impact: f64,
    pub macd_impact: f64,
    pub bollinger_impact: f64,
    pub sma_impact: f64,
    pub momentum_impact: f64,
    pub volatility_impact: f64,
}

/// MLSignalPredictor: Modelleri orkestra eder ve nihai işlem kararını verir.
pub struct MLSignalPredictor {
    model: LinearRegressor,
    confidence_threshold: f64, 
}

impl MLSignalPredictor {
    /// Yeni bir tahminci oluşturur. 
    /// confidence_threshold: Sinyal üretmek için gereken minimum model güveni.
    pub fn new(confidence_threshold: f64) -> Self {
        Self {
            model: LinearRegressor::with_defaults(),
            confidence_threshold: confidence_threshold.clamp(0.0, 1.0),
        }
    }

    /// Mum verilerinden otonom sinyal tahmini yapar.
    /// robotic_loop içindeki 'sinyal oylama' mantığının yeni yuvasıdır.
    pub fn predict(&self, candles: &[Candle]) -> Result<MLSignalPrediction> {
        if candles.is_empty() {
            return Err(MemosTradingError::Unknown("Veri yok — tahmin yapılamaz".into()));
        }

        // 1. Otonom Öznitelik Çıkarımı (Feature Extraction)
        let features = FeatureExtractor::extract(candles);
        
        // 2. Model Tahmini (Inference)
        let prediction = self.model.predict(&features);

        // 3. Etki Analizi (Feature Importance)
        let importance = self.calculate_feature_importance(&features);

        // 4. Sinyal Karar Mekanizması (Eşik Denetimi)
        // PnL beklentisi ve güven skoru eşiği geçemezse 'Hold' dönülür.
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

    /// Toplu (Batch) tahmin işleme — Geçmiş veriler üzerinde hızlı tarama sağlar.
    pub fn predict_batch(&self, candles_list: &[Vec<Candle>]) -> Result<Vec<MLSignalPrediction>> {
        candles_list.iter()
            .map(|c| self.predict(c))
            .collect()
    }

    /// Modeli otonom eğitir ve başarı oranını (Accuracy) döner.
    pub fn train(&mut self, training_data: &[(Vec<Candle>, f64)]) -> Result<f64> {
        if training_data.is_empty() {
            return Err(MemosTradingError::Unknown("Eğitim verisi boş".into()));
        }

        let mut data = Vec::with_capacity(training_data.len());
        for (candles, target) in training_data {
            let fv = FeatureExtractor::extract(candles);
            data.push((fv, *target));
        }

        // %80 Eğitim / %20 Test ayırımı
        let split = (data.len() as f64 * 0.8) as usize;
        let (train_data, test_data) = data.split_at(split);

        // Otonom Eğitim Döngüsü
        self.model.train(train_data, 10, 0.01);

        // Doğruluk Analizi
        Ok(self.model.evaluate(test_data))
    }

    /// Mevcut tahminin hangi indikatöre dayandığını otonom analiz eder.
    fn calculate_feature_importance(&self, features: &FeatureVector) -> FeatureImportance {
        let norm = features.normalize();

        // Her bir özniteliğin (normalize değer - nötr nokta) * model ağırlığı
        let rsi_impact = (norm.rsi - 0.5).abs() * self.model.weights[0];
        let macd_impact = (norm.macd - 0.5).abs() * self.model.weights[1];
        
        let bb_impact = ((norm.bb_upper - 0.5).abs() * self.model.weights[3] + 
                        (norm.bb_lower - 0.5).abs() * self.model.weights[4]) / 2.0;

        let sma_impact = ((norm.sma_5 - 0.5).abs() * self.model.weights[6] + 
                         (norm.sma_10 - 0.5).abs() * self.model.weights[7] + 
                         (norm.sma_20 - 0.5).abs() * self.model.weights[8]) / 3.0;

        let momentum_impact = (norm.momentum - 0.5).abs() * self.model.weights[9];
        let volatility_impact = norm.volatility * self.model.weights[10];

        FeatureImportance {
            rsi_impact,
            macd_impact,
            bollinger_impact: bb_impact,
            sma_impact,
            momentum_impact,
            volatility_impact,
        }
    }

    pub fn set_confidence_threshold(&mut self, threshold: f64) {
        self.confidence_threshold = threshold.clamp(0.0, 1.0);
    }

    pub fn get_model(&self) -> &LinearRegressor { &self.model }
    pub fn get_model_mut(&mut self) -> &mut LinearRegressor { &mut self.model }
}
