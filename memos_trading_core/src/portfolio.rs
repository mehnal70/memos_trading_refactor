use crate::types::RiskParams;
// portfolio.rs - Portföy ve durum yönetimi modülü
// Açık pozisyonlar, bakiye, geçmiş işlemler ve portföy durumunun güncellenmesi

use crate::types::{Trade, Signal};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Default)]
pub struct Position {
	pub symbol: String,
	pub entry_price: f64,
	pub amount: f64,
	pub entry_time: DateTime<Utc>,
	pub signal: Signal,
	pub strategy: String, // Çoklu strateji desteği
}

#[derive(Debug, Clone, Default)]
pub struct Portfolio {
	pub balance: f64,
	pub positions: Vec<Position>,
	pub trade_history: Vec<Trade>,
	pub risk_params: Option<RiskParams>, // Portföy seviyesinde risk limiti
}

impl Portfolio {
	pub fn new(balance: f64, risk_params: Option<RiskParams>) -> Self {
		Self { balance, positions: vec![], trade_history: vec![], risk_params }
	}
	/// Pozisyon aç (çoklu strateji desteği)
	pub fn open_position(&mut self, symbol: &str, entry_price: f64, amount: f64, signal: Signal, strategy: &str) {
		let pos = Position {
			symbol: symbol.to_string(),
			entry_price,
			amount,
			entry_time: Utc::now(),
			signal,
			strategy: strategy.to_string(),
		};
		self.positions.push(pos);
		self.balance -= entry_price * amount;
	}
	/// Pozisyon kapat
	pub fn close_position(&mut self, symbol: &str, exit_price: f64) {
		if let Some(idx) = self.positions.iter().position(|p| p.symbol == symbol) {
			let pos = self.positions.remove(idx);
			let pnl = (exit_price - pos.entry_price) * pos.amount * match pos.signal {
				Signal::Buy => 1.0,
				Signal::Sell => -1.0,
				Signal::Hold => 0.0,
			};
			self.balance += exit_price * pos.amount + pnl;
			let trade = Trade {
				id: None,
				symbol: pos.symbol,
				entry_price: pos.entry_price,
				exit_price: Some(exit_price),
				amount: pos.amount,
				entry_time: pos.entry_time,
				exit_time: Some(Utc::now()),
				pnl: Some(pnl),
				pnl_pct: Some(pnl / (pos.entry_price * pos.amount)),
				strategy: pos.strategy,
			};
			self.trade_history.push(trade);
		}
	}

	/// Portföydeki toplam riskli varlık miktarını hesapla
	pub fn total_risk_exposure(&self) -> f64 {
		self.positions.iter().map(|p| p.amount * p.entry_price).sum::<f64>()
	}

	/// Portföy risk limiti aşıldı mı?
	pub fn is_risk_limit_exceeded(&self) -> bool {
		if let Some(risk) = &self.risk_params {
			if let Some(max_risk) = risk.max_portfolio_risk_pct {
				let total = self.total_risk_exposure();
				let limit = (self.balance + total) * (max_risk / 100.0);
				return total > limit;
			}
		}
		false
	}

	/// Otomatik rebalans (örnek: eşit ağırlık)
	pub fn rebalance_equal_weight(&mut self) {
		let n = self.positions.len();
		if n == 0 { return; }
		let total_value = self.balance + self.total_risk_exposure();
		let target = total_value / n as f64;
		for pos in &mut self.positions {
			pos.amount = target / pos.entry_price;
		}
	}



	    /// Portföy durumunu güncelle (örnek: metrikler)
	    pub fn update_metrics(&self) -> PortfolioMetrics {
	        let total_pnl: f64 = self.trade_history.iter().map(|t| t.pnl.unwrap_or(0.0)).sum();
	        let win_count = self.trade_history.iter().filter(|t| t.pnl.unwrap_or(0.0) > 0.0).count();
	        let loss_count = self.trade_history.iter().filter(|t| t.pnl.unwrap_or(0.0) < 0.0).count();
	        let total = self.trade_history.len();
	        PortfolioMetrics {
	            total_pnl,
	            win_rate: if total > 0 { win_count as f64 / total as f64 } else { 0.0 },
	            loss_rate: if total > 0 { loss_count as f64 / total as f64 } else { 0.0 },
	            trade_count: total,
	        }
	    }
	}

	// --- Trait implementasyonları dosyanın en dışına taşındı ---

	impl crate::health_monitor::HealthCheck for Portfolio {
	    fn check_health(&self) -> crate::health_monitor::HealthStatus {
	        let metrics = self.update_metrics();
	        if metrics.total_pnl < 0.0 {
	            crate::health_monitor::HealthStatus::Warning(format!("Toplam PnL negatif: {}", metrics.total_pnl))
	        } else if metrics.win_rate < 0.2 && metrics.trade_count > 10 {
	            crate::health_monitor::HealthStatus::Warning(format!("Kazanç oranı çok düşük: {:.2}", metrics.win_rate))
	        } else {
	            crate::health_monitor::HealthStatus::Healthy
	        }
	    }
	}

	impl crate::health_monitor::AnomalyDetector for Portfolio {
	    fn detect_anomaly(&self) -> Option<crate::health_monitor::AnomalyType> {
	        let metrics = self.update_metrics();
	        if metrics.total_pnl < -1000.0 {
	            return Some(crate::health_monitor::AnomalyType::Custom(format!("Aşırı zarar: {}", metrics.total_pnl)));
	        }
	        if metrics.trade_count > 0 && metrics.win_rate == 0.0 {
	            return Some(crate::health_monitor::AnomalyType::Custom("Hiç kazançlı işlem yok".to_string()));
	        }
	        None
	    }
	}

use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PortfolioMetrics {
	pub total_pnl: f64,
	pub win_rate: f64,
	pub loss_rate: f64,
	pub trade_count: usize,
}
