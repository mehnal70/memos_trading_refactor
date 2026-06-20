// robot/interfaces.rs - Modüler mimari için ana trait ve interface şablonları

use crate::core::types::{Candle, Signal, StrategyParams, Trade};
use crate::Result;

/// DataFetcher: Her türlü veri kaynağından veri çeker
pub trait DataFetcher: Send + Sync {
    fn fetch(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>>;
    fn source_type(&self) -> &str;
}

// NOT: `LiveDataFetcher` trait'i + (BinanceLiveAdapter/HybridBinanceFetcher) implementasyonları
// kaldırıldı (çoklu-piyasa Faz 0-C). Ölü-kümeydi: canlı tüketici yoktu, URL'leri bozuktu
// (binance.com), funding `data_fetcher::binance::fetch_funding_history`'den geliyor. Canlı veri/
// fiyat artık `venue::VenueAdapter` (MarketData) üzerinden tek-kaynaklı. [[venue]]

/// StrategyEngine: Strateji yönetimi ve sinyal üretimi
pub trait StrategyEngine: Send + Sync {
    fn name(&self) -> &str;
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams) -> Result<Signal>;
}

/// Calculator: Teknik göstergeler ve matematiksel hesaplamalar
pub trait Calculator: Send + Sync {
    fn sma(&self, values: &[f64], period: usize) -> Result<f64>;
    fn rsi(&self, values: &[f64], period: usize) -> Result<f64>;
    // Diğer göstergeler eklenebilir
}

/// Reporter: Sonuçları, işlemleri ve riskleri raporlar
pub trait Reporter: Send + Sync {
    fn report_trade(&self, trade: &Trade) -> Result<()>;
    fn report_strategy(&self, name: &str, signal: &Signal) -> Result<()>;
    fn export_json(&self, data: &serde_json::Value) -> Result<()>;
    fn export_csv(&self, data: &str) -> Result<()>;
}

/// RiskAnalyzer: Pozisyon ve portföy riskini analiz eder
pub trait RiskAnalyzer: Send + Sync {
    fn analyze(&self, trade: &Trade) -> Result<f64>;
    fn max_position_size(&self, capital: f64, price: f64) -> Result<f64>;
}

/// TradeExecutor: Otomatik trade işlemlerini yönetir
pub trait TradeExecutor: Send + Sync {
    fn execute(&self, signal: Signal, symbol: &str, amount: f64) -> Result<Trade>;
    fn cancel_all(&self, symbol: &str) -> Result<()>;
    /// Exchange'deki gerçek açık pozisyon sembollerini döndürür.
    /// Paper mod veya desteklenmeyen executor'larda boş vec döner.
    fn fetch_open_symbols(&self) -> Vec<String> { vec![] }
}
