// Portfolio Manager - Tip Tanımları
//
// Srivastava mimarisi: Portföy varlıklarını track etmek için
// Multi-position, PnL, drawdown, correlation

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};

/// Açık pozisyon
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    /// Trading pair (örnek: "BTCUSDT")
    pub symbol: String,
    
    /// Açılış fiyatı
    pub entry_price: f64,
    
    /// Pozisyon büyüklüğü
    pub quantity: f64,
    
    /// Long (1.0) veya Short (-1.0)
    pub direction: f64,
    
    /// Pozisyon açılış zamanı
    pub entry_time: DateTime<Utc>,
    
    /// Şimdiki fiyat (güncellenecek)
    pub current_price: f64,
    
    /// Stop-loss seviyesi (optional)
    pub stop_loss: Option<f64>,
    
    /// Take-profit seviyesi (optional)
    pub take_profit: Option<f64>,
}

impl Position {
    /// Unrealized PnL hesapla
    pub fn unrealized_pnl(&self) -> f64 {
        (self.current_price - self.entry_price) * self.quantity * self.direction
    }
    
    /// Unrealized PnL yüzdesi
    pub fn unrealized_pnl_pct(&self) -> f64 {
        if self.entry_price == 0.0 {
            0.0
        } else {
            (self.unrealized_pnl() / (self.entry_price * self.quantity)) * 100.0
        }
    }
    
    /// Pozisyon Value (notional)
    pub fn position_value(&self) -> f64 {
        self.current_price * self.quantity
    }
    
    /// Pozisyon yaşı (saniye cinsinden)
    pub fn age_seconds(&self) -> i64 {
        (Utc::now() - self.entry_time).num_seconds()
    }
}

/// Kapalı işlem
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosedTrade {
    /// Trading pair
    pub symbol: String,
    
    /// Açılış fiyatı
    pub entry_price: f64,
    
    /// Kapanış fiyatı
    pub exit_price: f64,
    
    /// Pozisyon büyüklüğü
    pub quantity: f64,
    
    /// Yön (1.0 = long, -1.0 = short)
    pub direction: f64,
    
    /// Açılış zamanı
    pub entry_time: DateTime<Utc>,
    
    /// Kapanış zamanı
    pub exit_time: DateTime<Utc>,
    
    /// Realized PnL
    pub realized_pnl: f64,
    
    /// Realized PnL %
    pub realized_pnl_pct: f64,
}

impl ClosedTrade {
    /// Win mı loss mı?
    pub fn is_win(&self) -> bool {
        self.realized_pnl > 0.0
    }
    
    /// Trade süresi (saniye)
    pub fn duration_seconds(&self) -> i64 {
        (self.exit_time - self.entry_time).num_seconds()
    }
}

/// Portföy metrikleri
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PortfolioMetrics {
    /// Toplam sermaye
    pub total_capital: f64,
    
    /// Mevcut cash
    pub available_cash: f64,
    
    /// Açık pozisyonlardaki toplam value
    pub open_positions_value: f64,
    
    /// Unrealized PnL (açık pozisyonlardan)
    pub unrealized_pnl: f64,
    
    /// Realized PnL (kapalı işlemlerden)
    pub realized_pnl: f64,
    
    /// Toplam PnL
    pub total_pnl: f64,
    
    /// Return % (toplam)
    pub total_return_pct: f64,
    
    /// Açık pozisyon sayısı
    pub open_positions_count: usize,
    
    /// Kapalı işlem sayısı
    pub closed_trades_count: usize,
    
    /// Win rate
    pub win_rate: f64,
    
    /// Average win
    pub avg_win: f64,
    
    /// Average loss
    pub avg_loss: f64,
    
    /// Profit factor (total wins / total losses)
    pub profit_factor: f64,
    
    /// Max drawdown
    pub max_drawdown: f64,
    
    /// Max drawdown %
    pub max_drawdown_pct: f64,
    
    /// Sharpe ratio (if available)
    pub sharpe_ratio: Option<f64>,
    
    /// Sortino ratio (if available)
    pub sortino_ratio: Option<f64>,
}

impl PortfolioMetrics {
    pub fn new(total_capital: f64) -> Self {
        Self {
            total_capital,
            available_cash: total_capital,
            ..Default::default()
        }
    }
}

/// Position Correlation Info (risk yönetimi için)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionCorrelation {
    pub symbol_pair: (String, String),
    pub correlation: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CorrelationMatrix {
    pub pairs: Vec<PositionCorrelation>,
}

impl CorrelationMatrix {
    /// Maksimum correlation bul
    pub fn max_correlation(&self) -> Option<f64> {
        self.pairs.iter().map(|p| p.correlation.abs()).max_by(|a, b| a.partial_cmp(b).unwrap())
    }
    
    /// Aynı sembol pairler için correlation kontrol et
    pub fn get_correlation(&self, symbol1: &str, symbol2: &str) -> Option<f64> {
        self.pairs
            .iter()
            .find(|p| {
                (p.symbol_pair.0 == symbol1 && p.symbol_pair.1 == symbol2)
                    || (p.symbol_pair.0 == symbol2 && p.symbol_pair.1 == symbol1)
            })
            .map(|p| p.correlation)
    }
}
