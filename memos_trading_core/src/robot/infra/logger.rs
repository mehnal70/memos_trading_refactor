// robot/logger.rs - Trade logging sistemi (dosya + JSON)

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use chrono::Utc;
use serde::{Serialize, Deserialize};
use crate::core::types::{Signal, Trade};
use crate::MemosTradingError;

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

    /// Trade eventi oluştur (eski API — Trade tipi üzerinden)
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

    /// Pozisyon AÇILIŞ eventi (TRADE_OPEN). PnL açılışta 0.
    pub fn trade_open(
        symbol: &str,
        strategy: &str,
        is_long: bool,
        price: f64,
        qty: f64,
        equity: f64,
    ) -> Self {
        let side = if is_long { "LONG" } else { "SHORT" };
        Self {
            timestamp: Utc::now().to_rfc3339(),
            event_type: "TRADE_OPEN".to_string(),
            symbol: symbol.to_string(),
            signal: side.to_string(),
            price,
            quantity: qty,
            pnl: 0.0,
            equity,
            message: format!(
                "OPEN {} {} qty={:.6} @ ${:.4} | Strat={} | Equity: ${:.2}",
                side, symbol, qty, price, strategy, equity,
            ),
        }
    }

    /// Pozisyon KAPANIŞ eventi (TRADE_CLOSE).
    pub fn trade_close(
        symbol: &str,
        strategy: &str,
        is_long: bool,
        exit_price: f64,
        qty: f64,
        pnl: f64,
        equity: f64,
        reason: &str,
    ) -> Self {
        let side = if is_long { "LONG" } else { "SHORT" };
        Self {
            timestamp: Utc::now().to_rfc3339(),
            event_type: "TRADE_CLOSE".to_string(),
            symbol: symbol.to_string(),
            signal: side.to_string(),
            price: exit_price,
            quantity: qty,
            pnl,
            equity,
            message: format!(
                "CLOSE {} {} qty={:.6} @ ${:.4} | Reason={} | PnL: ${:.4} | Strat={} | Equity: ${:.2}",
                side, symbol, qty, exit_price, reason, pnl, strategy, equity,
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
                .map_err(|e| MemosTradingError::Io(e))?;
        }
        if let Some(parent) = Path::new(json_file).parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| MemosTradingError::Io(e))?;
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
            .map_err(|e| MemosTradingError::Io(e))?;

        let log_line = format!(
            "[{}] {:10} {:8} {} - {}\n",
            event.timestamp,
            event.event_type,
            event.symbol,
            event.signal,
            event.message
        );

        file.write_all(log_line.as_bytes())
            .map_err(|e| MemosTradingError::Io(e))?;

        Ok(())
    }

    /// JSON log dosyasına yaz (JSONL - newline-delimited JSON)
    fn write_json_log(&self, event: &TradeEvent) -> Result<(), MemosTradingError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.json_file)
            .map_err(|e| MemosTradingError::Io(e))?;

        let json_line = serde_json::to_string(event)
            .map_err(|e| MemosTradingError::Serde(e))?;

        writeln!(file, "{}", json_line)
            .map_err(|e| MemosTradingError::Io(e))?;

        Ok(())
    }

    /// Log dosyalarını temizle (yeni session için)
    pub fn clear_logs(&self) -> Result<(), MemosTradingError> {
        // Log dosyasını sil
        if Path::new(&self.log_file).exists() {
            std::fs::remove_file(&self.log_file)
                .map_err(|e| MemosTradingError::Io(e))?;
        }

        // JSON dosyasını sil
        if Path::new(&self.json_file).exists() {
            std::fs::remove_file(&self.json_file)
                .map_err(|e| MemosTradingError::Io(e))?;
        }

        // Boş dosyaları yeniden oluştur ki monitoring script'i dosya bulunamadı demesin
        File::create(&self.log_file)
            .map_err(|e| MemosTradingError::Io(e))?;
        File::create(&self.json_file)
            .map_err(|e| MemosTradingError::Io(e))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::Signal;

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

    #[test]
    fn test_trade_open_event_long() {
        let ev = TradeEvent::trade_open(
            "BTCUSDT", "MA_CROSSOVER", true, 50_000.0, 0.01, 10_000.0,
        );
        assert_eq!(ev.event_type, "TRADE_OPEN");
        assert_eq!(ev.signal, "LONG");
        assert_eq!(ev.price, 50_000.0);
        assert_eq!(ev.quantity, 0.01);
        assert_eq!(ev.pnl, 0.0);
        assert_eq!(ev.equity, 10_000.0);
        assert!(ev.message.contains("OPEN LONG BTCUSDT"));
        assert!(ev.message.contains("MA_CROSSOVER"));
    }

    #[test]
    fn test_trade_open_event_short() {
        let ev = TradeEvent::trade_open(
            "ETHUSDT", "BOLLINGER", false, 3_000.0, 0.5, 9_500.0,
        );
        assert_eq!(ev.event_type, "TRADE_OPEN");
        assert_eq!(ev.signal, "SHORT");
        assert!(ev.message.contains("OPEN SHORT ETHUSDT"));
    }

    #[test]
    fn test_trade_close_event_profit() {
        let ev = TradeEvent::trade_close(
            "BTCUSDT", "MA_CROSSOVER", true, 51_500.0, 0.01, 15.0, 10_015.0, "TAKE_PROFIT",
        );
        assert_eq!(ev.event_type, "TRADE_CLOSE");
        assert_eq!(ev.signal, "LONG");
        assert_eq!(ev.price, 51_500.0);
        assert_eq!(ev.pnl, 15.0);
        assert_eq!(ev.equity, 10_015.0);
        assert!(ev.message.contains("CLOSE LONG BTCUSDT"));
        assert!(ev.message.contains("TAKE_PROFIT"));
        assert!(ev.message.contains("MA_CROSSOVER"));
    }

    #[test]
    fn test_trade_close_event_loss() {
        let ev = TradeEvent::trade_close(
            "ETHUSDT", "BOLLINGER", false, 3_050.0, 0.5, -25.0, 9_475.0, "STOP_LOSS",
        );
        assert_eq!(ev.event_type, "TRADE_CLOSE");
        assert_eq!(ev.signal, "SHORT");
        assert_eq!(ev.pnl, -25.0);
        assert!(ev.message.contains("STOP_LOSS"));
    }
}
