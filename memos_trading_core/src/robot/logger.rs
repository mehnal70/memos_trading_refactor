// robot/logger.rs - Trade logging sistemi (dosya + JSON)

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use chrono::Utc;
use serde::{Serialize, Deserialize};
use crate::{Signal, Trade, MemosTradingError};

/// Trade eventi JSON formatı
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeEvent {
    pub timestamp: String,
    pub event_type: String, // "SIGNAL", "TRADE", "RISK_BLOCK", "ERROR"
    pub symbol: String,
    pub signal: String, // "BUY", "SELL", "HOLD"
    pub price: f64,
    pub quantity: f64,
    pub pnl: f64,
    pub equity: f64,
    pub message: String,
}

impl TradeEvent {
    /// Signal eventi oluştur
    pub fn signal(symbol: &str, signal: Signal, price: f64) -> Self {
        Self {
            timestamp: Utc::now().to_rfc3339(),
            event_type: "SIGNAL".to_string(),
            symbol: symbol.to_string(),
            signal: format!("{:?}", signal),
            price,
            quantity: 0.0,
            pnl: 0.0,
            equity: 0.0,
            message: format!("{:?} signal @ ${:.2}", signal, price),
        }
    }

    /// Trade eventi oluştur
    pub fn trade(trade: &Trade, pnl: f64, equity: f64) -> Self {
        Self {
            timestamp: Utc::now().to_rfc3339(),
            event_type: "TRADE".to_string(),
            symbol: trade.symbol.clone(),
            signal: trade.strategy.clone(),
            price: trade.entry_price,
            quantity: trade.amount,
            pnl,
            equity,
            message: format!(
                "{} {} @ ${:.2} | PnL: ${:.2} | Equity: ${:.2}",
                trade.strategy, trade.amount, trade.entry_price, pnl, equity
            ),
        }
    }

    /// Risk block eventi oluştur
    pub fn risk_block(reason: &str, symbol: &str) -> Self {
        Self {
            timestamp: Utc::now().to_rfc3339(),
            event_type: "RISK_BLOCK".to_string(),
            symbol: symbol.to_string(),
            signal: "HOLD".to_string(),
            price: 0.0,
            quantity: 0.0,
            pnl: 0.0,
            equity: 0.0,
            message: format!("RISK BLOCKED: {}", reason),
        }
    }

    /// Error eventi oluştur
    pub fn error(error: &str) -> Self {
        Self {
            timestamp: Utc::now().to_rfc3339(),
            event_type: "ERROR".to_string(),
            symbol: "UNKNOWN".to_string(),
            signal: "HOLD".to_string(),
            price: 0.0,
            quantity: 0.0,
            pnl: 0.0,
            equity: 0.0,
            message: format!("ERROR: {}", error),
        }
    }
}

/// Trading logger - hem human-readable log hem JSON history yazar
pub struct TradingLogger {
    log_file: String,
    json_file: String,
}

impl TradingLogger {
    /// Yeni logger oluştur
    pub fn new(log_file: &str, json_file: &str) -> Result<Self, MemosTradingError> {
        // Dizinleri oluştur
        if let Some(parent) = Path::new(log_file).parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| MemosTradingError::IoError(e))?;
        }
        if let Some(parent) = Path::new(json_file).parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| MemosTradingError::IoError(e))?;
        }

        Ok(Self {
            log_file: log_file.to_string(),
            json_file: json_file.to_string(),
        })
    }

    /// Trade eventini logla (hem text hem JSON)
    pub fn log_event(&self, event: &TradeEvent) -> Result<(), MemosTradingError> {
        // 1. Human-readable log dosyasına yaz
        self.write_text_log(event)?;

        // 2. JSON dosyasına append et (JSONL formatı - her satır bir JSON)
        self.write_json_log(event)?;

        Ok(())
    }

    /// Text log dosyasına yaz (append mode)
    fn write_text_log(&self, event: &TradeEvent) -> Result<(), MemosTradingError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_file)
            .map_err(|e| MemosTradingError::IoError(e))?;

        let log_line = format!(
            "[{}] {:10} {:8} {} - {}\n",
            event.timestamp,
            event.event_type,
            event.symbol,
            event.signal,
            event.message
        );

        file.write_all(log_line.as_bytes())
            .map_err(|e| MemosTradingError::IoError(e))?;

        Ok(())
    }

    /// JSON log dosyasına yaz (JSONL - newline-delimited JSON)
    fn write_json_log(&self, event: &TradeEvent) -> Result<(), MemosTradingError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.json_file)
            .map_err(|e| MemosTradingError::IoError(e))?;

        let json_line = serde_json::to_string(event)
            .map_err(|e| MemosTradingError::SerdeError(e))?;

        writeln!(file, "{}", json_line)
            .map_err(|e| MemosTradingError::IoError(e))?;

        Ok(())
    }

    /// Log dosyalarını temizle (yeni session için)
    pub fn clear_logs(&self) -> Result<(), MemosTradingError> {
        // Log dosyasını sil
        if Path::new(&self.log_file).exists() {
            std::fs::remove_file(&self.log_file)
                .map_err(|e| MemosTradingError::IoError(e))?;
        }

        // JSON dosyasını sil
        if Path::new(&self.json_file).exists() {
            std::fs::remove_file(&self.json_file)
                .map_err(|e| MemosTradingError::IoError(e))?;
        }

        // Boş dosyaları yeniden oluştur ki monitoring script'i dosya bulunamadı demesin
        File::create(&self.log_file)
            .map_err(|e| MemosTradingError::IoError(e))?;
        File::create(&self.json_file)
            .map_err(|e| MemosTradingError::IoError(e))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Signal;

    #[test]
    fn test_trade_event_creation() {
        let event = TradeEvent::signal("BTCUSDT", Signal::Buy, 50000.0);
        assert_eq!(event.event_type, "SIGNAL");
        assert_eq!(event.signal, "Buy");
        assert_eq!(event.price, 50000.0);
    }

    #[test]
    fn test_logger_writes() {
        let logger = TradingLogger::new(
            "logs/test_robotic.log",
            "logs/test_trades.jsonl"
        ).unwrap();

        // Clear önce
        logger.clear_logs().ok();

        let event = TradeEvent::signal("BTCUSDT", Signal::Buy, 50000.0);
        logger.log_event(&event).unwrap();

        // Dosya oluştu mu kontrol et
        assert!(Path::new("logs/test_robotic.log").exists());
        assert!(Path::new("logs/test_trades.jsonl").exists());

        // Cleanup
        logger.clear_logs().ok();
    }

    #[test]
    fn test_risk_block_event() {
        let event = TradeEvent::risk_block("Max drawdown exceeded", "ETHUSDT");
        assert_eq!(event.event_type, "RISK_BLOCK");
        assert!(event.message.contains("RISK BLOCKED"));
    }
}
