// Order Management System - Tip Tanımları
// 
// Srivastava mimarisi: OMS katmanında standardize tip tanımları
// Forex/Crypto/Stocks hepsi aynı interface'i kullanacak


// robot/order_management/types.rs - Otonom Emir ve İnfaz Veri Modeli

use serde::{Serialize, Deserialize};
use crate::core::model::{OrderId};
//use crate::core::model::Order;

// --- 4. SLIPPAGE VE RETRY POLİTİKALARI ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlippageLevel { Low, Medium, High, Critical }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlippageInfo {
    pub expected_price: f64,
    pub actual_price: f64,
    pub slippage_pct: f64,
    pub level: SlippageLevel,
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub initial_delay_ms: u64,
    pub backoff_multiplier: f64,
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_retries: 3, initial_delay_ms: 100, backoff_multiplier: 2.0, max_delay_ms: 5000 }
    }
}

impl RetryPolicy {
    pub fn get_delay_for_attempt(&self, attempt: u32) -> u64 {
        let delay = (self.initial_delay_ms as f64 * self.backoff_multiplier.powi(attempt as i32)) as u64;
        delay.min(self.max_delay_ms)
    }
}

/// Kısmi Dolum Bilgisi - Emir tamamen dolana kadar her parça için mühürlenir
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialFillInfo {
    pub order_id: OrderId,
    pub filled_quantity: f64,
    pub fill_price: f64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl PartialFillInfo {
    pub fn new(order_id: OrderId, qty: f64, price: f64) -> Self {
        Self {
            order_id,
            filled_quantity: qty,
            fill_price: price,
            timestamp: chrono::Utc::now(),
        }
    }

    /// Bu dolumun toplam dolar hacmini (notional) verir
    pub fn notional(&self) -> f64 {
        self.filled_quantity * self.fill_price
    }
}

