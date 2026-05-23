// robot/logger.rs - Trade logging sistemi (dosya + JSON)

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use chrono::Utc;
use serde::{Serialize, Deserialize};
use crate::core::types::{Signal, Trade};
use crate::MemosTradingError;

/// SIGNAL throttle: aynı (symbol, signal) için bu süre içinde gelen tekrar
/// log'ları yutulur. Varsayılan 60 sn, env: MEMOS_SIGNAL_THROTTLE_SECS.
/// 0 verilirse throttle kapalı (her sinyal yazılır).
const DEFAULT_SIGNAL_THROTTLE_SECS: u64 = 60;

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
    /// `leverage` = pozisyonun çözülmüş kaldıraç değeri (1.0 = spot, >1 = futures).
    pub fn trade_open(
        symbol: &str,
        strategy: &str,
        is_long: bool,
        price: f64,
        qty: f64,
        equity: f64,
        leverage: f64,
    ) -> Self {
        let side = if is_long { "LONG" } else { "SHORT" };
        let strat_label = crate::core::model::normalize_strategy_label(strategy);
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
                "OPEN {} {} qty={:.6} @ ${:.4} | Lev={:.1}x | Strat={} | Equity: ${:.2}",
                side, symbol, qty, price, leverage, strat_label, equity,
            ),
        }
    }

    /// Pozisyon KAPANIŞ eventi (TRADE_CLOSE).
    /// `leverage` = pozisyonun çözülmüş kaldıraç değeri (PositionModel.leverage'tan).
    #[allow(clippy::too_many_arguments)]
    pub fn trade_close(
        symbol: &str,
        strategy: &str,
        is_long: bool,
        exit_price: f64,
        qty: f64,
        pnl: f64,
        equity: f64,
        reason: &str,
        leverage: f64,
    ) -> Self {
        let side = if is_long { "LONG" } else { "SHORT" };
        let strat_label = crate::core::model::normalize_strategy_label(strategy);
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
                "CLOSE {} {} qty={:.6} @ ${:.4} | Lev={:.1}x | Reason={} | PnL: ${:.4} | Strat={} | Equity: ${:.2}",
                side, symbol, qty, exit_price, leverage, reason, pnl, strat_label, equity,
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
    /// SIGNAL idempotency cache: (symbol, signal_label) → son yazılan an.
    /// Aynı kombinasyon throttle_window içinde tekrar gelirse atılır.
    last_signal: Mutex<HashMap<(String, String), Instant>>,
    /// Throttle penceresi; 0 ise throttle kapalı.
    signal_throttle: Duration,
}

impl TradingLogger {
    /// Yeni logger oluştur. SIGNAL throttle penceresi env'den okunur
    /// (`MEMOS_SIGNAL_THROTTLE_SECS`, default 60 sn).
    pub fn new(log_file: &str, json_file: &str) -> Result<Self, MemosTradingError> {
        let throttle_secs = std::env::var("MEMOS_SIGNAL_THROTTLE_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_SIGNAL_THROTTLE_SECS);
        Self::new_with_throttle(log_file, json_file, throttle_secs)
    }

    /// Throttle saniyesini doğrudan belirleyen varyant (test ve özel durumlar).
    /// `throttle_secs = 0` → throttle kapalı, her SIGNAL yazılır.
    pub fn new_with_throttle(
        log_file: &str,
        json_file: &str,
        throttle_secs: u64,
    ) -> Result<Self, MemosTradingError> {
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
            last_signal: Mutex::new(HashMap::new()),
            signal_throttle: Duration::from_secs(throttle_secs),
        })
    }

    /// Aynı (event_type, symbol, signal) throttle penceresi içinde tekrar
    /// gelirse `true` döner. İlk geçişte cache güncellenir ve `false` döner.
    /// SIGNAL ve RISK_BLOCK için kullanılır; TRADE_OPEN/TRADE_CLOSE/ERROR
    /// throttle dışıdır (zaten seyrek olaylardır).
    fn should_throttle(&self, event_type: &str, symbol: &str, signal_label: &str) -> bool {
        if self.signal_throttle.is_zero() {
            return false;
        }
        let key = (
            format!("{}|{}", event_type, symbol),
            signal_label.to_string(),
        );
        let now = Instant::now();
        let mut map = match self.last_signal.lock() {
            Ok(g) => g,
            Err(_) => return false, // poisoned ise log'a izin ver
        };
        if let Some(prev) = map.get(&key) {
            if now.duration_since(*prev) < self.signal_throttle {
                return true;
            }
        }
        map.insert(key, now);
        false
    }

    /// Trade eventini logla (hem text hem JSON)
    pub fn log_event(&self, event: &TradeEvent) -> Result<(), MemosTradingError> {
        // Throttle: SIGNAL + RISK_BLOCK aynı koşullarda her cycle'da tekrarlanabilir,
        // 60sn pencerede tekilleştir → 12 MB trades.jsonl spam'i çözülür.
        let throttled = matches!(event.event_type.as_str(), "SIGNAL" | "RISK_BLOCK");
        if throttled
            && self.should_throttle(&event.event_type, &event.symbol, &event.signal)
        {
            return Ok(());
        }

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
            "BTCUSDT", "MA_CROSSOVER", true, 50_000.0, 0.01, 10_000.0, 3.0,
        );
        assert_eq!(ev.event_type, "TRADE_OPEN");
        assert_eq!(ev.signal, "LONG");
        assert_eq!(ev.price, 50_000.0);
        assert_eq!(ev.quantity, 0.01);
        assert_eq!(ev.pnl, 0.0);
        assert_eq!(ev.equity, 10_000.0);
        assert!(ev.message.contains("OPEN LONG BTCUSDT"));
        assert!(ev.message.contains("MA_CROSSOVER"));
        assert!(ev.message.contains("Lev=3.0x"));
    }

    #[test]
    fn test_trade_open_normalizes_auto_strategy() {
        // "AUTO" sentinel → "Otonom (rejime göre)" olarak yansımalı.
        let ev = TradeEvent::trade_open(
            "BTCUSDT", "AUTO", true, 50_000.0, 0.01, 10_000.0, 1.0,
        );
        assert!(ev.message.contains("Strat=Otonom (rejime göre)"),
            "AUTO normalize edilmedi: {}", ev.message);
        assert!(!ev.message.contains("Strat=AUTO"),
            "raw AUTO görünüyor: {}", ev.message);

        // "Default" da aynı normalize edilmeli.
        let ev2 = TradeEvent::trade_open(
            "ETHUSDT", "Default", false, 3_000.0, 0.5, 9_500.0, 1.0,
        );
        assert!(ev2.message.contains("Strat=Otonom (rejime göre)"));
    }

    #[test]
    fn test_trade_close_normalizes_auto_strategy() {
        let ev = TradeEvent::trade_close(
            "BTCUSDT", "AUTO", true, 51_000.0, 0.01, 10.0, 10_010.0, "TAKE_PROFIT", 1.0,
        );
        assert!(ev.message.contains("Strat=Otonom (rejime göre)"),
            "AUTO normalize edilmedi: {}", ev.message);
    }

    #[test]
    fn test_trade_open_event_short() {
        let ev = TradeEvent::trade_open(
            "ETHUSDT", "BOLLINGER", false, 3_000.0, 0.5, 9_500.0, 1.0,
        );
        assert_eq!(ev.event_type, "TRADE_OPEN");
        assert_eq!(ev.signal, "SHORT");
        assert!(ev.message.contains("OPEN SHORT ETHUSDT"));
    }

    #[test]
    fn test_trade_close_event_profit() {
        let ev = TradeEvent::trade_close(
            "BTCUSDT", "MA_CROSSOVER", true, 51_500.0, 0.01, 15.0, 10_015.0, "TAKE_PROFIT", 3.0,
        );
        assert_eq!(ev.event_type, "TRADE_CLOSE");
        assert_eq!(ev.signal, "LONG");
        assert_eq!(ev.price, 51_500.0);
        assert_eq!(ev.pnl, 15.0);
        assert_eq!(ev.equity, 10_015.0);
        assert!(ev.message.contains("CLOSE LONG BTCUSDT"));
        assert!(ev.message.contains("TAKE_PROFIT"));
        assert!(ev.message.contains("MA_CROSSOVER"));
        assert!(ev.message.contains("Lev=3.0x"));
    }

    #[test]
    fn test_trade_close_event_loss() {
        let ev = TradeEvent::trade_close(
            "ETHUSDT", "BOLLINGER", false, 3_050.0, 0.5, -25.0, 9_475.0, "STOP_LOSS", 1.0,
        );
        assert_eq!(ev.event_type, "TRADE_CLOSE");
        assert_eq!(ev.signal, "SHORT");
        assert_eq!(ev.pnl, -25.0);
        assert!(ev.message.contains("STOP_LOSS"));
    }

    fn count_lines(path: &str) -> usize {
        std::fs::read_to_string(path)
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    #[test]
    fn test_signal_throttle_dedup_same_symbol_signal() {
        let dir = std::env::temp_dir().join(format!(
            "memos_logger_throttle_{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let log = dir.join("a.log");
        let json = dir.join("a.jsonl");
        let log_s = log.to_string_lossy().to_string();
        let json_s = json.to_string_lossy().to_string();
        let logger = TradingLogger::new_with_throttle(&log_s, &json_s, 60).unwrap();
        logger.clear_logs().ok();

        let ev = TradeEvent::signal("ADGYO", Signal::Buy, 44.86);
        // Üst üste üç çağrı (gerçek cycle senaryosu) — sadece ilki yazılmalı.
        logger.log_event(&ev).unwrap();
        logger.log_event(&ev).unwrap();
        logger.log_event(&ev).unwrap();

        assert_eq!(count_lines(&json_s), 1, "throttle aynı sinyali yutmadı");
        logger.clear_logs().ok();
    }

    #[test]
    fn test_signal_throttle_distinct_keys_pass() {
        let dir = std::env::temp_dir().join(format!(
            "memos_logger_distinct_{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let log = dir.join("b.log");
        let json = dir.join("b.jsonl");
        let log_s = log.to_string_lossy().to_string();
        let json_s = json.to_string_lossy().to_string();
        let logger = TradingLogger::new_with_throttle(&log_s, &json_s, 60).unwrap();
        logger.clear_logs().ok();

        logger.log_event(&TradeEvent::signal("ADGYO", Signal::Buy, 44.86)).unwrap();
        logger.log_event(&TradeEvent::signal("ADGYO", Signal::Sell, 44.86)).unwrap(); // farklı sinyal
        logger.log_event(&TradeEvent::signal("THYAO", Signal::Buy, 100.0)).unwrap();   // farklı sembol

        assert_eq!(count_lines(&json_s), 3, "farklı anahtarlar yutuldu");
        logger.clear_logs().ok();
    }

    #[test]
    fn test_signal_throttle_zero_disables() {
        let dir = std::env::temp_dir().join(format!(
            "memos_logger_zero_{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let log = dir.join("c.log");
        let json = dir.join("c.jsonl");
        let log_s = log.to_string_lossy().to_string();
        let json_s = json.to_string_lossy().to_string();
        let logger = TradingLogger::new_with_throttle(&log_s, &json_s, 0).unwrap();
        logger.clear_logs().ok();

        let ev = TradeEvent::signal("ADGYO", Signal::Buy, 44.86);
        for _ in 0..5 {
            logger.log_event(&ev).unwrap();
        }
        assert_eq!(count_lines(&json_s), 5, "throttle=0 olduğunda tümü yazılmalı");
        logger.clear_logs().ok();
    }

    #[test]
    fn test_throttle_only_signal_and_risk_block() {
        let dir = std::env::temp_dir().join(format!(
            "memos_logger_other_{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let log = dir.join("d.log");
        let json = dir.join("d.jsonl");
        let log_s = log.to_string_lossy().to_string();
        let json_s = json.to_string_lossy().to_string();
        let logger = TradingLogger::new_with_throttle(&log_s, &json_s, 60).unwrap();
        logger.clear_logs().ok();

        // SIGNAL ve RISK_BLOCK throttle hedefi → ikinci çağrı yutulur.
        logger.log_event(&TradeEvent::signal("BTC", Signal::Buy, 50_000.0)).unwrap();
        logger.log_event(&TradeEvent::signal("BTC", Signal::Buy, 50_000.0)).unwrap();    // yutulur
        logger.log_event(&TradeEvent::risk_block("test", "BTC")).unwrap();
        logger.log_event(&TradeEvent::risk_block("test", "BTC")).unwrap();               // yutulur
        // TRADE_OPEN / TRADE_CLOSE / ERROR throttle dışı.
        logger.log_event(&TradeEvent::trade_open("BTC", "S", true, 50_000.0, 0.01, 10_000.0, 1.0)).unwrap();
        logger.log_event(&TradeEvent::trade_close("BTC", "S", true, 51_000.0, 0.01, 10.0, 10_010.0, "TP", 1.0)).unwrap();
        logger.log_event(&TradeEvent::error("e")).unwrap();

        // 1 SIGNAL + 1 RISK_BLOCK + 1 OPEN + 1 CLOSE + 1 ERROR = 5
        assert_eq!(count_lines(&json_s), 5);
        logger.clear_logs().ok();
    }

    #[test]
    fn test_risk_block_throttle_dedup() {
        let dir = std::env::temp_dir().join(format!(
            "memos_logger_rb_{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let log = dir.join("e.log");
        let json = dir.join("e.jsonl");
        let log_s = log.to_string_lossy().to_string();
        let json_s = json.to_string_lossy().to_string();
        let logger = TradingLogger::new_with_throttle(&log_s, &json_s, 60).unwrap();
        logger.clear_logs().ok();

        // Aynı sembolde art arda 3 RISK_BLOCK → sadece 1 yazılmalı.
        for _ in 0..3 {
            logger.log_event(&TradeEvent::risk_block("guardrails", "BTCUSDT")).unwrap();
        }
        // Farklı sembol throttle'dan bağımsız geçer.
        logger.log_event(&TradeEvent::risk_block("guardrails", "ETHUSDT")).unwrap();

        assert_eq!(count_lines(&json_s), 2);
        logger.clear_logs().ok();
    }
}
