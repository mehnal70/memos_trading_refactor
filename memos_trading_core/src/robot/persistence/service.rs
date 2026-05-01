use crate::robot::{TradeRepository, CandleRepository};
use crate::types::{Trade, Candle};
use crate::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Veritabanı işlemleri için servis katmanı
pub struct PersistenceService {
    pub trade_repo: TradeRepository,
    pub candle_repo: CandleRepository,
}

impl PersistenceService {
    pub fn new(db_path: &str) -> Result<Self> {
        Ok(Self {
            trade_repo: TradeRepository::new(db_path)?,
            candle_repo: CandleRepository::new(db_path)?,
        })
    }

    /// Tüm trade'leri al
    pub fn get_all_trades(&self) -> Result<Vec<TradeResponse>> {
        let trades = self.trade_repo.get_all_trades()?;
        Ok(trades.into_iter().map(TradeResponse::from).collect())
    }

    /// Symbol'e göre trade'leri al
    pub fn get_trades_by_symbol(&self, symbol: &str) -> Result<Vec<TradeResponse>> {
        let trades = self.trade_repo.get_trades_by_symbol(symbol)?;
        Ok(trades.into_iter().map(TradeResponse::from).collect())
    }

    /// Strateji'ye göre trade'leri al
    pub fn get_trades_by_strategy(&self, strategy: &str) -> Result<Vec<TradeResponse>> {
        let trades = self.trade_repo.get_trades_by_strategy(strategy)?;
        Ok(trades.into_iter().map(TradeResponse::from).collect())
    }

    /// Kapalı trade'leri al
    pub fn get_closed_trades(&self) -> Result<Vec<TradeResponse>> {
        let trades = self.trade_repo.get_closed_trades()?;
        Ok(trades.into_iter().map(TradeResponse::from).collect())
    }

    /// Trade sayısı
    pub fn count_trades(&self) -> Result<usize> {
        self.trade_repo.count_trades()
    }

    /// Trade ekle
    pub fn insert_trade(&self, trade: &Trade) -> Result<()> {
        self.trade_repo.insert_trade(trade)
    }

    /// Mum verilerini al
    pub fn get_candles(&self, symbol: &str, interval: &str) -> Result<Vec<CandleResponse>> {
        let candles = self.candle_repo.get_candles(symbol, interval)?;
        Ok(candles.into_iter().map(CandleResponse::from).collect())
    }

    /// Son N mum verisini al
    pub fn get_last_candles(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<CandleResponse>> {
        let candles = self.candle_repo.get_last_candles(symbol, interval, limit)?;
        Ok(candles.into_iter().map(CandleResponse::from).collect())
    }

    /// Mum sayısı
    pub fn count_candles(&self, symbol: &str, interval: &str) -> Result<usize> {
        self.candle_repo.count_candles(symbol, interval)
    }

    /// Mum ekle
    pub fn insert_candle(&self, candle: &Candle) -> Result<()> {
        self.candle_repo.insert_candle(candle)
    }

    /// Mum'ları toplu ekle
    pub fn insert_candles(&self, candles: &[Candle]) -> Result<()> {
        self.candle_repo.insert_candles(candles)
    }

    /// Tarih aralığında mum'ları al
    pub fn get_candles_in_range(
        &self,
        symbol: &str,
        interval: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<CandleResponse>> {
        let candles = self.candle_repo.get_candles_in_range(symbol, interval, start, end)?;
        Ok(candles.into_iter().map(CandleResponse::from).collect())
    }
}

/// Trade yanıt formatı (Tauri için serialize edilebilir)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeResponse {
    pub id: Option<u64>,
    pub symbol: String,
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    pub amount: f64,
    pub entry_time: String,
    pub exit_time: Option<String>,
    pub pnl: Option<f64>,
    pub pnl_pct: Option<f64>,
    pub strategy: String,
}

impl From<Trade> for TradeResponse {
    fn from(trade: Trade) -> Self {
        Self {
            id: trade.id,
            symbol: trade.symbol,
            entry_price: trade.entry_price,
            exit_price: trade.exit_price,
            amount: trade.amount,
            entry_time: trade.entry_time.to_rfc3339(),
            exit_time: trade.exit_time.map(|t| t.to_rfc3339()),
            pnl: trade.pnl,
            pnl_pct: trade.pnl_pct,
            strategy: trade.strategy,
        }
    }
}

/// Mum verisinin yanıt formatı
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandleResponse {
    pub symbol: String,
    pub interval: String,
    pub timestamp: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl From<Candle> for CandleResponse {
    fn from(candle: Candle) -> Self {
        Self {
            symbol: candle.symbol,
            interval: candle.interval,
            timestamp: candle.timestamp.to_rfc3339(),
            open: candle.open,
            high: candle.high,
            low: candle.low,
            close: candle.close,
            volume: candle.volume,
        }
    }
}

/// Basit bir istatistik cevabı
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsResponse {
    pub total_trades: usize,
    pub closed_trades: usize,
    pub total_pnl: f64,
    pub win_rate: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trade_response_conversion() {
        let trade = Trade {
            id: Some(1),
            symbol: "BTC".to_string(),
            entry_price: 100.0,
            exit_price: Some(110.0),
            amount: 1.0,
            entry_time: Utc::now(),
            exit_time: Some(Utc::now()),
            pnl: Some(10.0),
            pnl_pct: Some(10.0),
            strategy: "test".to_string(),
        };

        let response = TradeResponse::from(trade);
        assert_eq!(response.symbol, "BTC");
        assert_eq!(response.entry_price, 100.0);
    }

    #[test]
    fn test_candle_response_conversion() {
        let candle = Candle {
            symbol: "BTC".to_string(),
            interval: "1m".to_string(),
            timestamp: Utc::now(),
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 105.0,
            volume: 1000.0,
        };

        let response = CandleResponse::from(candle);
        assert_eq!(response.symbol, "BTC");
        assert_eq!(response.close, 105.0);
    }

    #[test]
    fn test_persistence_service_creation() {
        let service = PersistenceService::new(":memory:");
        assert!(service.is_ok());
    }
}
