// robot/interfaces.rs - Modüler mimari için ana trait ve interface şablonları

use crate::core::types::{Candle, FundingRatePoint, Signal, StrategyParams, Trade, Exchange, Market};
use crate::Result;
use async_trait::async_trait;

/// DataFetcher: Her türlü veri kaynağından veri çeker
pub trait DataFetcher: Send + Sync {
    fn fetch(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>>;
    fn source_type(&self) -> &str;
}

/// LiveDataFetcher: Async live veri çekme trait'i
#[async_trait]
pub trait LiveDataFetcher: Send + Sync {
    async fn fetch_latest(
        &self,
        exchange: Exchange,
        market: Market,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Candle>>;
    
    /// Futures/CoinM için anlık funding rate — diğer piyasalarda `Ok(None)` döner.
    async fn fetch_funding_rate(
        &self,
        _market: Market,
        _symbol: &str,
    ) -> Result<Option<FundingRatePoint>> {
        Ok(None)
    }

    fn source_name(&self) -> &str;
    fn supported_markets(&self) -> Vec<Market>;
    fn supported_symbols(&self, market: Market) -> Vec<String>;
}

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
