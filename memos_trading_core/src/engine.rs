use crate::strategies::Strategy;
/// Basit grid search ile otomatik strateji optimizasyonu
pub fn optimize_strategy_grid_search(
	candles: &[Candle],
	strategy: &dyn Strategy,
	param_grid: &[StrategyParams],
) -> Option<(StrategyParams, f64)> {
	let mut best_params = None;
	let mut best_pnl = f64::MIN;
	for params in param_grid {
		// Sinyal üret
		let mut signals = vec![];
		for i in 0..candles.len() {
			let slice = &candles[..=i];
			let signal = strategy.generate_signal(slice, params, None, None).unwrap_or(crate::types::Signal::Hold);
			signals.push(signal);
		}
		// Backtest
		let mut engine = Engine::new(EngineConfig {
			initial_balance: 10000.0,
			strategy_params: params.clone(),
			ml_enabled: false,
			monitor_enabled: false,
		});
		let portfolio = engine.run_backtest(candles, &signals);
		let pnl = portfolio.update_metrics().total_pnl;
		if pnl > best_pnl {
			best_pnl = pnl;
			best_params = Some(params.clone());
		}
	}
	best_params.map(|p| (p, best_pnl))
}
use crate::bist::{get_bist100_symbols, batch_fetch_bist_klines};
use crate::batch_config::BatchFetchConfig;


// engine.rs - ML/AI destekli orchestrator ve backtest-runner modülü
// Strateji, portföy ve ML modelini otomatik seçip, backtest ve canlı modda çalıştırır
// engine.rs - ML/AI destekli orchestrator ve backtest-runner modülü
// Strateji, portföy ve ML modelini otomatik seçip, backtest ve canlı modda çalıştırır

use crate::robot::{Portfolio, MLModel, Monitor};
use crate::types::{Candle, Signal, StrategyParams};
use crate::health_monitor::{HealthCheck, AnomalyDetector, HealthStatus, AnomalyType};

pub struct EngineConfig {
	pub initial_balance: f64,
	pub strategy_params: StrategyParams,
	pub ml_enabled: bool,
	pub monitor_enabled: bool,
}

pub struct Engine {
	pub config: EngineConfig,
	pub ml_model: Option<MLModel>,
	pub monitor: Option<Monitor>,
}

// Engine için HealthCheck ve AnomalyDetector trait implementasyonları
impl HealthCheck for Engine {
	fn check_health(&self) -> HealthStatus {
		// ML model ve monitor var mı, portföy sağlığı örneği
		if self.ml_model.is_none() && self.config.ml_enabled {
			HealthStatus::Warning("ML modeli etkin ama yüklenmemiş".to_string())
		} else if self.monitor.is_none() && self.config.monitor_enabled {
			HealthStatus::Warning("Monitor etkin ama yüklenmemiş".to_string())
		} else {
			HealthStatus::Healthy
		}
	}
}

impl AnomalyDetector for Engine {
	fn detect_anomaly(&self) -> Option<AnomalyType> {
		// Örnek: ML model yoksa ve etkinse anomali
		if self.ml_model.is_none() && self.config.ml_enabled {
			return Some(AnomalyType::Custom("ML modeli eksik".to_string()));
		}
		if self.monitor.is_none() && self.config.monitor_enabled {
			return Some(AnomalyType::Custom("Monitor eksik".to_string()));
		}
		None
	}
}

impl Engine {
	pub fn new(config: EngineConfig) -> Self {
		Self { config, ml_model: None, monitor: None }
	}

	/// ML/AI destekli backtest runner
	pub fn run_backtest(&mut self, candles: &[Candle], signals: &[Signal]) -> Portfolio {
		let mut portfolio = Portfolio::new(self.config.initial_balance, None);
		let mut ml_data = vec![];
		for (i, candle) in candles.iter().enumerate() {
			let features = MLModel::extract_features(&candles[..=i]);
			let ml_signal = if self.config.ml_enabled {
				if let Some(model) = &self.ml_model {
					let pred = model.predict(&features);
					if pred > 0.5 { Signal::Buy } else if pred < -0.5 { Signal::Sell } else { Signal::Hold }
				} else { Signal::Hold }
			} else { Signal::Hold };
			let signal = if ml_signal != Signal::Hold { ml_signal } else { signals[i].clone() };
			match signal {
				Signal::Buy | Signal::Sell => {
					if !portfolio.positions.iter().any(|p| p.symbol == candle.symbol) {
						portfolio.open_position(&candle.symbol, candle.close, 1.0, signal.clone(), "default");
					}
				},
				Signal::Hold => {
					if portfolio.positions.iter().any(|p| p.symbol == candle.symbol) {
						portfolio.close_position(&candle.symbol, candle.close);
					}
				}
			}
			// ML veri biriktir
			let mut fv = features;
			fv.signal = Some(signal);
			fv.pnl = portfolio.trade_history.last().and_then(|t| t.pnl);
			ml_data.push(fv);
		}
		// ML modeli güncelle (örnek)
		if self.config.ml_enabled {
			if let Some(_model) = &mut self.ml_model {
				// Note: ml_data is Vec<FeatureVector>, fit expects &[(FeatureVector, f64)]
				// Convert if needed: let data: Vec<_> = ml_data.iter().map(|f| (f.clone(), f.pnl.unwrap_or(0.0))).collect();
				// model.fit(&data);
			}
		}
		// Monitor ile sonuç analizi (örnek)
		#[cfg(target_arch = "wasm32")]
		if self.config.monitor_enabled {
			if let Some(monitor) = &mut self.monitor {
				let anomaly = if metrics.win_rate < 0.3 { Some(1.0) } else { Some(0.0) };
				let action = monitor.check(anomaly);
				if action != MonitorAction::Continue {
					// Uyarı veya otomatik aksiyon alınabilir
				}
			}
		}
		portfolio
	}

	/// ML/AI destekli canlı mod runner (örnek: BIST sembollerini limitli ve kademeli çek)
	pub async fn run_live(&mut self) {
		// Parametreler örnek, ihtiyaca göre ayarlanabilir
		let symbols = get_bist100_symbols();
		let interval = "1d";
		let now = chrono::Utc::now().timestamp_millis();
		let one_year_ago = now - 365 * 24 * 60 * 60 * 1000;
		let limit = 200;
		let config = BatchFetchConfig {
			concurrency_limit: 10, // Aynı anda 10 sembol çek
			wait_ms: 2000,         // Her deneme arası 2 saniye bekle
			max_retries: 5,        // Maksimum 5 deneme
		};

		let results = batch_fetch_bist_klines(
			&symbols,
			interval,
			one_year_ago,
			now,
			limit,
			&config,
		).await;

		for (symbol, res) in results.into_iter() {
			match res {
				Ok(klines) => println!("✅ {} için {} veri çekildi", symbol, klines.len()),
				Err(e) => println!("❌ {} için veri çekilemedi: {}", symbol, e),
			}
		}
		// Burada pipeline devam ettirilebilir (ML, portföy, sinyal vs.)
	}
}
