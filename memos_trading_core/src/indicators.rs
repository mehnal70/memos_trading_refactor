
// indicators.rs - ML/AI destekli teknik analiz ve feature extraction modülü
// Klasik ve ML tabanlı göstergeler, otomatik yeni feature keşfi

use crate::types::Candle;

pub struct IndicatorEngine;

impl IndicatorEngine {
	/// Klasik SMA
	pub fn sma(candles: &[Candle], period: usize) -> f64 {
		let n = candles.len().min(period);
		if n == 0 { return 0.0; }
		candles[candles.len()-n..].iter().map(|c| c.close).sum::<f64>() / n as f64
	}

	/// Klasik RSI
	pub fn rsi(candles: &[Candle], period: usize) -> f64 {
		if candles.len() < period+1 { return 50.0; }
		let mut gains = 0.0;
		let mut losses = 0.0;
		for w in candles.windows(2).rev().take(period) {
			let diff = w[1].close - w[0].close;
			if diff > 0.0 { gains += diff; } else { losses -= diff; }
		}
		let rs = if losses == 0.0 { 100.0 } else { gains / losses };
		100.0 - (100.0 / (1.0 + rs))
	}

	/// ML tabanlı otomatik feature extraction (örnek: PCA, autoencoder, dummy)
	pub fn ml_features(_candles: &[Candle]) -> Vec<f64> {
		// Gerçek uygulamada: PCA, autoencoder, embedding, vs. ile otomatik feature çıkarımı
		// Dummy: Son 5 kapanışın normalize farkları
		let n = _candles.len().min(5);
		if n < 2 { return vec![0.0; 4]; }
		let closes: Vec<f64> = _candles[_candles.len()-n..].iter().map(|c| c.close).collect();
		let mean = closes.iter().sum::<f64>() / closes.len() as f64;
		closes.windows(2).map(|w| (w[1] - w[0]) / mean).collect()
	}

	/// Otomatik yeni indikatör keşfi (ML ile feature importance)
	pub fn discover_features(_candles: &[Candle]) -> Vec<String> {
		// Gerçek uygulamada: ML ile en anlamlı feature'ları seç
		// Dummy: SMA, RSI, ML_Feature
		vec!["sma".to_string(), "rsi".to_string(), "ml_feature".to_string()]
	}
}
