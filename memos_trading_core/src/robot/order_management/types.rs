// Order Management System - Tip Tanımları
// 
// Srivastava mimarisi: OMS katmanında standardize tip tanımları
// Forex/Crypto/Stocks hepsi aynı interface'i kullanacak

use serde::{Serialize, Deserialize};
use std::fmt;

/// Emir ID tipi (unique identifier)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrderId(pub u64);

impl fmt::Display for OrderId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Emir tarafı
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderSide {
    #[serde(rename = "BUY")]
    Buy,
    #[serde(rename = "SELL")]
    Sell,
}

impl fmt::Display for OrderSide {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            OrderSide::Buy => write!(f, "BUY"),
            OrderSide::Sell => write!(f, "SELL"),
        }
    }
}

/// Emir türü
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    #[serde(rename = "LIMIT")]
    Limit,
    #[serde(rename = "MARKET")]
    Market,
    #[serde(rename = "STOP_LOSS")]
    StopLoss,
    #[serde(rename = "TAKE_PROFIT")]
    TakeProfit,
}

impl fmt::Display for OrderType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            OrderType::Limit => write!(f, "LIMIT"),
            OrderType::Market => write!(f, "MARKET"),
            OrderType::StopLoss => write!(f, "STOP_LOSS"),
            OrderType::TakeProfit => write!(f, "TAKE_PROFIT"),
        }
    }
}

/// Emir durumu
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    #[serde(rename = "NEW")]
    New,
    #[serde(rename = "PARTIALLY_FILLED")]
    PartiallyFilled,
    #[serde(rename = "FILLED")]
    Filled,
    #[serde(rename = "CANCELED")]
    Canceled,
    #[serde(rename = "REJECTED")]
    Rejected,
    #[serde(rename = "EXPIRED")]
    Expired,
}

impl fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            OrderStatus::New => write!(f, "NEW"),
            OrderStatus::PartiallyFilled => write!(f, "PARTIALLY_FILLED"),
            OrderStatus::Filled => write!(f, "FILLED"),
            OrderStatus::Canceled => write!(f, "CANCELED"),
            OrderStatus::Rejected => write!(f, "REJECTED"),
            OrderStatus::Expired => write!(f, "EXPIRED"),
        }
    }
}

/// Tam bir emir tanımı
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    /// Emir ID (bir kez ayarlandığında sabit)
    pub id: Option<OrderId>,
    
    /// Trading pair (örnek: "BTCUSDT")
    pub symbol: String,
    
    /// Al/Sat tarafı
    pub side: OrderSide,
    
    /// Emir türü
    pub order_type: OrderType,
    
    /// Miktar (base currency)
    pub quantity: f64,
    
    /// Fiyat (limit emirleri için gerekli)
    pub price: Option<f64>,
    
    /// Stop price (stop-loss/take-profit için)
    pub stop_price: Option<f64>,
    
    /// İndirilmiş miktar (partial fill durumunda)
    pub filled_quantity: f64,
    
    /// Emir durumu
    pub status: OrderStatus,
    
    /// Ortalama doldurulma fiyatı
    pub average_price: f64,
    
    /// Timestamp
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    
    /// Exchange tarafından dönen extra veriler
    pub raw_data: Option<String>,
}

impl Default for Order {
    fn default() -> Self {
        Self {
            id: None,
            symbol: String::new(),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            quantity: 0.0,
            price: None,
            stop_price: None,
            filled_quantity: 0.0,
            status: OrderStatus::New,
            average_price: 0.0,
            created_at: None,
            raw_data: None,
        }
    }
}

impl Order {
    /// Yeni bir market order oluştur
    pub fn market(symbol: String, side: OrderSide, quantity: f64) -> Self {
        Self {
            symbol,
            side,
            order_type: OrderType::Market,
            quantity,
            filled_quantity: 0.0,
            ..Default::default()
        }
    }
    
    /// Yeni bir limit order oluştur
    pub fn limit(symbol: String, side: OrderSide, quantity: f64, price: f64) -> Self {
        Self {
            symbol,
            side,
            order_type: OrderType::Limit,
            quantity,
            price: Some(price),
            filled_quantity: 0.0,
            ..Default::default()
        }
    }
    
    /// Stop-loss order oluştur
    pub fn stop_loss(symbol: String, quantity: f64, stop_price: f64) -> Self {
        Self {
            symbol,
            side: OrderSide::Sell,
            order_type: OrderType::StopLoss,
            quantity,
            stop_price: Some(stop_price),
            filled_quantity: 0.0,
            ..Default::default()
        }
    }
    
    /// Take-profit order oluştur
    pub fn take_profit(symbol: String, quantity: f64, price: f64) -> Self {
        Self {
            symbol,
            side: OrderSide::Sell,
            order_type: OrderType::TakeProfit,
            quantity,
            price: Some(price),
            filled_quantity: 0.0,
            ..Default::default()
        }
    }
    
    /// Kalan doldurulacak miktar
    pub fn remaining_quantity(&self) -> f64 {
        self.quantity - self.filled_quantity
    }
    
    /// Doldurulma yüzdesi
    pub fn fill_percentage(&self) -> f64 {
        if self.quantity == 0.0 {
            0.0
        } else {
            (self.filled_quantity / self.quantity) * 100.0
        }
    }
    
    /// Emir tamamen dolduruldu mu?
    pub fn is_fully_filled(&self) -> bool {
        (self.quantity - self.filled_quantity).abs() < f64::EPSILON
    }
}

/// Slippage Tespiti Sonucu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlippageInfo {
    /// Beklenen fiyat
    pub expected_price: f64,
    
    /// Fiili ortalama doldurulma fiyatı
    pub actual_price: f64,
    
    /// Slippage yüzdesi
    pub slippage_pct: f64,
    
    /// Slippage düzeyi (Low/High/Critical)
    pub level: SlippageLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlippageLevel {
    Low,    // < 0.1%
    Medium, // 0.1% - 0.5%
    High,   // 0.5% - 1.0%
    Critical, // > 1.0%
}

/// Kısmi Fill Bilgisi
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialFillInfo {
    pub order_id: OrderId,
    pub filled_quantity: f64,
    pub fill_price: f64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Retry Politikası
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maksimum retry sayısı
    pub max_retries: u32,
    
    /// İlk retry delay (millisecond)
    pub initial_delay_ms: u64,
    
    /// Exponential backoff factor
    pub backoff_multiplier: f64,
    
    /// Maksimum delay (millisecond)
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 100,
            backoff_multiplier: 2.0,
            max_delay_ms: 5000,
        }
    }
}

impl RetryPolicy {
    /// N'inci retry için delay hesapla
    pub fn get_delay_for_attempt(&self, attempt: u32) -> u64 {
        let delay = (self.initial_delay_ms as f64 * self.backoff_multiplier.powi(attempt as i32)) as u64;
        delay.min(self.max_delay_ms)
    }
}
