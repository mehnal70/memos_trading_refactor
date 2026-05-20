use crate::types::{Trade, Candle};
use crate::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

/// Trade Repository - SQLite'den trade geçmişi yönet
pub struct TradeRepository {
    connection: Arc<Mutex<Connection>>,
}

impl Clone for TradeRepository {
    fn clone(&self) -> Self {
        Self {
            connection: Arc::clone(&self.connection),
        }
    }
}

impl TradeRepository {
    /// Yeni repository oluştur veya var olan veritabanını aç
    pub fn new(db_path: &str) -> Result<Self> {
        let connection = Connection::open(db_path)?;
        
        let repo = Self { 
            connection: Arc::new(Mutex::new(connection))
        };
        repo.init_schema()?;
        
        Ok(repo)
    }

    /// Veritabanı şeması oluştur
    fn init_schema(&self) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS trades (
                id INTEGER PRIMARY KEY,
                trade_id INTEGER UNIQUE,
                symbol TEXT NOT NULL,
                entry_price REAL NOT NULL,
                exit_price REAL,
                amount REAL NOT NULL,
                entry_time TEXT NOT NULL,
                exit_time TEXT,
                pnl REAL,
                pnl_pct REAL,
                strategy TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            
            CREATE INDEX IF NOT EXISTS idx_symbol ON trades(symbol);
            CREATE INDEX IF NOT EXISTS idx_entry_time ON trades(entry_time);
            CREATE INDEX IF NOT EXISTS idx_strategy ON trades(strategy);"
        )?;
        
        Ok(())
    }

    /// Trade'i kaydet
    pub fn insert_trade(&self, trade: &Trade) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO trades 
            (trade_id, symbol, entry_price, exit_price, amount, entry_time, exit_time, pnl, pnl_pct, strategy, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                trade.id,
                &trade.symbol,
                trade.entry_price,
                trade.exit_price,
                trade.amount,
                trade.entry_time.to_rfc3339(),
                trade.exit_time.map(|t| t.to_rfc3339()),
                trade.pnl,
                trade.pnl_pct,
                &trade.strategy,
                Utc::now().to_rfc3339(),
            ],
        )?;
        
        Ok(())
    }

    /// Tüm trade'leri getir
    pub fn get_all_trades(&self) -> Result<Vec<Trade>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT trade_id, symbol, entry_price, exit_price, amount, entry_time, exit_time, pnl, pnl_pct, strategy 
             FROM trades ORDER BY entry_time DESC"
        )?;

        let trades = stmt.query_map([], |row| {
            Ok(Trade {
                id: row.get(0)?,
                symbol: row.get(1)?,
                entry_price: row.get(2)?,
                exit_price: row.get(3)?,
                amount: row.get(4)?,
                entry_time: parse_datetime(&row.get::<_, String>(5)?),
                exit_time: row.get::<_, Option<String>>(6)?.map(|s| parse_datetime(&s)),
                pnl: row.get(7)?,
                pnl_pct: row.get(8)?,
                strategy: row.get(9)?,
            })
        })?;

        let mut result = Vec::new();
        for trade in trades {
            result.push(trade?);
        }

        Ok(result)
    }

    /// Symbol'e göre trade'leri getir
    pub fn get_trades_by_symbol(&self, symbol: &str) -> Result<Vec<Trade>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT trade_id, symbol, entry_price, exit_price, amount, entry_time, exit_time, pnl, pnl_pct, strategy 
             FROM trades WHERE symbol = ? ORDER BY entry_time DESC"
        )?;

        let trades = stmt.query_map(params![symbol], |row| {
            Ok(Trade {
                id: row.get(0)?,
                symbol: row.get(1)?,
                entry_price: row.get(2)?,
                exit_price: row.get(3)?,
                amount: row.get(4)?,
                entry_time: parse_datetime(&row.get::<_, String>(5)?),
                exit_time: row.get::<_, Option<String>>(6)?.map(|s| parse_datetime(&s)),
                pnl: row.get(7)?,
                pnl_pct: row.get(8)?,
                strategy: row.get(9)?,
            })
        })?;

        let mut result = Vec::new();
        for trade in trades {
            result.push(trade?);
        }

        Ok(result)
    }

    /// Strateji'ye göre trade'leri getir
    pub fn get_trades_by_strategy(&self, strategy: &str) -> Result<Vec<Trade>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT trade_id, symbol, entry_price, exit_price, amount, entry_time, exit_time, pnl, pnl_pct, strategy 
             FROM trades WHERE strategy = ? ORDER BY entry_time DESC"
        )?;

        let trades = stmt.query_map(params![strategy], |row| {
            Ok(Trade {
                id: row.get(0)?,
                symbol: row.get(1)?,
                entry_price: row.get(2)?,
                exit_price: row.get(3)?,
                amount: row.get(4)?,
                entry_time: parse_datetime(&row.get::<_, String>(5)?),
                exit_time: row.get::<_, Option<String>>(6)?.map(|s| parse_datetime(&s)),
                pnl: row.get(7)?,
                pnl_pct: row.get(8)?,
                strategy: row.get(9)?,
            })
        })?;

        let mut result = Vec::new();
        for trade in trades {
            result.push(trade?);
        }

        Ok(result)
    }

    /// Kapalı trade'leri getir (exit_price bazlı)
    pub fn get_closed_trades(&self) -> Result<Vec<Trade>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT trade_id, symbol, entry_price, exit_price, amount, entry_time, exit_time, pnl, pnl_pct, strategy 
             FROM trades WHERE exit_price IS NOT NULL ORDER BY exit_time DESC"
        )?;

        let trades = stmt.query_map([], |row| {
            Ok(Trade {
                id: row.get(0)?,
                symbol: row.get(1)?,
                entry_price: row.get(2)?,
                exit_price: row.get(3)?,
                amount: row.get(4)?,
                entry_time: parse_datetime(&row.get::<_, String>(5)?),
                exit_time: row.get::<_, Option<String>>(6)?.map(|s| parse_datetime(&s)),
                pnl: row.get(7)?,
                pnl_pct: row.get(8)?,
                strategy: row.get(9)?,
            })
        })?;

        let mut result = Vec::new();
        for trade in trades {
            result.push(trade?);
        }

        Ok(result)
    }

    /// Toplam trade sayısı
    pub fn count_trades(&self) -> Result<usize> {
        let conn = self.connection.lock().unwrap();
        let count: usize = conn.query_row(
            "SELECT COUNT(*) FROM trades",
            [],
            |row| row.get(0),
        )?;

        Ok(count)
    }

    /// Trade sil
    pub fn delete_trade(&self, trade_id: u64) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "DELETE FROM trades WHERE trade_id = ?",
            params![trade_id],
        )?;

        Ok(())
    }
}

/// Account State Repository - Hesap snapshot'larını kaydet
pub struct AccountStateRepository {
    connection: Arc<Mutex<Connection>>,
}

impl Clone for AccountStateRepository {
    fn clone(&self) -> Self {
        Self {
            connection: Arc::clone(&self.connection),
        }
    }
}

impl AccountStateRepository {
    pub fn new(connection: Connection) -> Result<Self> {
        let repo = Self { 
            connection: Arc::new(Mutex::new(connection))
        };
        repo.init_schema()?;
        Ok(repo)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS account_states (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                balance REAL NOT NULL,
                unrealized_pnl REAL,
                realized_pnl REAL,
                total_pnl REAL,
                drawdown_pct REAL,
                win_rate REAL,
                created_at TEXT NOT NULL
            );
            
            CREATE INDEX IF NOT EXISTS idx_timestamp ON account_states(timestamp);"
        )?;

        Ok(())
    }

    /// Account state'i kaydet
    pub fn insert_state(
        &self,
        balance: f64,
        unrealized_pnl: Option<f64>,
        realized_pnl: f64,
        total_pnl: f64,
        drawdown_pct: f64,
        win_rate: f64,
    ) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute(
            "INSERT INTO account_states (timestamp, balance, unrealized_pnl, realized_pnl, total_pnl, drawdown_pct, win_rate, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                Utc::now().to_rfc3339(),
                balance,
                unrealized_pnl,
                realized_pnl,
                total_pnl,
                drawdown_pct,
                win_rate,
                Utc::now().to_rfc3339(),
            ],
        )?;

        Ok(())
    }

    /// Son N state'i getir
    pub fn get_recent_states(&self, limit: usize) -> Result<Vec<(DateTime<Utc>, f64, f64, f64)>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT timestamp, balance, total_pnl, drawdown_pct FROM account_states ORDER BY timestamp DESC LIMIT ?"
        )?;

        let states = stmt.query_map(params![limit as i32], |row| {
            Ok((
                parse_datetime(&row.get::<_, String>(0)?),
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
            ))
        })?;

        let mut result = Vec::new();
        for state in states {
            result.push(state?);
        }

        Ok(result)
    }
}

/// Candle Repository - Mum verilerini kaydet (backtesting için)
pub struct CandleRepository {
    connection: Arc<Mutex<Connection>>,
}

impl Clone for CandleRepository {
    fn clone(&self) -> Self {
        Self {
            connection: Arc::clone(&self.connection),
        }
    }
}

impl CandleRepository {
    pub fn new(db_path: &str) -> Result<Self> {
        let connection = Connection::open(db_path)?;
        
        let repo = Self { 
            connection: Arc::new(Mutex::new(connection))
        };
        repo.init_schema()?;
        
        Ok(repo)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS candles (
                id INTEGER PRIMARY KEY,
                symbol TEXT NOT NULL,
                interval TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                open REAL NOT NULL,
                high REAL NOT NULL,
                low REAL NOT NULL,
                close REAL NOT NULL,
                volume REAL NOT NULL,
                created_at TEXT NOT NULL
            );
            
            CREATE UNIQUE INDEX IF NOT EXISTS idx_symbol_interval_timestamp ON candles(symbol, interval, timestamp);
            CREATE INDEX IF NOT EXISTS idx_symbol ON candles(symbol);
            CREATE INDEX IF NOT EXISTS idx_timestamp ON candles(timestamp);"
        )?;

        Ok(())
    }

    /// Mum verisini kaydet — timestamp INTEGER (ms), `read_candles` ile aynı şema.
    pub fn insert_candle(&self, candle: &Candle) -> Result<()> {
        let conn = self.connection.lock().unwrap();
        let raw_ms = candle.timestamp.timestamp_millis();
        let updated = conn.execute(
            "UPDATE candles SET open=?1, high=?2, low=?3, close=?4, volume=?5 \
             WHERE symbol=?6 AND interval=?7 AND timestamp=?8",
            params![
                candle.open, candle.high, candle.low, candle.close, candle.volume,
                &candle.symbol, &candle.interval, raw_ms,
            ],
        )?;
        if updated == 0 {
            conn.execute(
                "INSERT INTO candles (symbol, interval, timestamp, open, high, low, close, volume, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    &candle.symbol, &candle.interval, raw_ms,
                    candle.open, candle.high, candle.low, candle.close, candle.volume,
                    Utc::now().to_rfc3339(),
                ],
            )?;
        }
        Ok(())
    }

    /// Toplu mum verisini kaydet — INSERT OR IGNORE (varsa atla) ile hızlı tx.
    pub fn insert_candles(&self, candles: &[Candle]) -> Result<()> {
        let mut conn = self.connection.lock().unwrap();
        let tx = conn.transaction()?;

        for candle in candles {
            let raw_ms = candle.timestamp.timestamp_millis();
            let updated = tx.execute(
                "UPDATE candles SET open=?1, high=?2, low=?3, close=?4, volume=?5 \
                 WHERE symbol=?6 AND interval=?7 AND timestamp=?8",
                params![
                    candle.open, candle.high, candle.low, candle.close, candle.volume,
                    &candle.symbol, &candle.interval, raw_ms,
                ],
            )?;
            if updated == 0 {
                tx.execute(
                    "INSERT INTO candles (symbol, interval, timestamp, open, high, low, close, volume, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        &candle.symbol, &candle.interval, raw_ms,
                        candle.open, candle.high, candle.low, candle.close, candle.volume,
                        Utc::now().to_rfc3339(),
                    ],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Symbol'ü ve aralığı için mum verilerini getir
    pub fn get_candles(&self, symbol: &str, interval: &str) -> Result<Vec<Candle>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT symbol, interval, timestamp, open, high, low, close, volume 
             FROM candles WHERE symbol = ? AND interval = ? ORDER BY timestamp ASC"
        )?;

        let candles = stmt.query_map(params![symbol, interval], |row| {
            Ok(Candle {
                symbol: row.get(0)?,
                interval: row.get(1)?,
                timestamp: read_timestamp_col(row, 2)?,
                open: row.get(3)?,
                high: row.get(4)?,
                low: row.get(5)?,
                close: row.get(6)?,
                volume: row.get(7)?,
            })
        })?;

        let mut result = Vec::new();
        for candle in candles {
            result.push(candle?);
        }

        Ok(result)
    }

    /// Son N mum verisini getir
    pub fn get_last_candles(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT symbol, interval, timestamp, open, high, low, close, volume 
             FROM candles WHERE symbol = ? AND interval = ? ORDER BY timestamp DESC LIMIT ?"
        )?;

        let candles = stmt.query_map(params![symbol, interval, limit as i32], |row| {
            Ok(Candle {
                symbol: row.get(0)?,
                interval: row.get(1)?,
                timestamp: read_timestamp_col(row, 2)?,
                open: row.get(3)?,
                high: row.get(4)?,
                low: row.get(5)?,
                close: row.get(6)?,
                volume: row.get(7)?,
            })
        })?;

        let mut result = Vec::new();
        for candle in candles {
            result.push(candle?);
        }
        
        result.reverse(); // Chronological order
        Ok(result)
    }

    /// Mum sayısı
    pub fn count_candles(&self, symbol: &str, interval: &str) -> Result<usize> {
        let conn = self.connection.lock().unwrap();
        let count: usize = conn.query_row(
            "SELECT COUNT(*) FROM candles WHERE symbol = ? AND interval = ?",
            params![symbol, interval],
            |row| row.get(0),
        )?;

        Ok(count)
    }

    /// Tarih aralığında mum verilerini getir
    pub fn get_candles_in_range(&self, symbol: &str, interval: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Vec<Candle>> {
        let conn = self.connection.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT symbol, interval, timestamp, open, high, low, close, volume 
             FROM candles WHERE symbol = ? AND interval = ? AND timestamp BETWEEN ? AND ? ORDER BY timestamp ASC"
        )?;

        let candles = stmt.query_map(params![symbol, interval, start.timestamp_millis(), end.timestamp_millis()], |row| {
            Ok(Candle {
                symbol: row.get(0)?,
                interval: row.get(1)?,
                timestamp: read_timestamp_col(row, 2)?,
                open: row.get(3)?,
                high: row.get(4)?,
                low: row.get(5)?,
                close: row.get(6)?,
                volume: row.get(7)?,
            })
        })?;

        let mut result = Vec::new();
        for candle in candles {
            result.push(candle?);
        }

        Ok(result)
    }
}

// Helper: String'i DateTime'a çevir
fn parse_datetime(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

/// SQLite'taki `timestamp` kolonunu hem INTEGER (ms) hem TEXT (RFC3339) formatından okur.
/// `save_candle`/`insert_candle` ms yazıyor; legacy satırlar string olabilir.
fn read_timestamp_col(row: &rusqlite::Row, idx: usize) -> rusqlite::Result<DateTime<Utc>> {
    use chrono::TimeZone;
    use rusqlite::types::ValueRef;
    match row.get_ref(idx)? {
        ValueRef::Integer(ms) => Ok(Utc.timestamp_millis_opt(ms).single()
            .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap())),
        ValueRef::Text(b) => {
            let s = std::str::from_utf8(b).unwrap_or("");
            Ok(parse_datetime(s))
        }
        _ => Ok(Utc.timestamp_opt(0, 0).single().unwrap()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_trade() -> Trade {
        Trade {
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
        }
    }

    #[test]
    fn test_trade_repository_creation() {
        let repo = TradeRepository::new(":memory:");
        assert!(repo.is_ok());
    }

    #[test]
    fn test_insert_and_retrieve_trade() {
        let repo = TradeRepository::new(":memory:").unwrap();
        let trade = create_test_trade();

        assert!(repo.insert_trade(&trade).is_ok());
        let trades = repo.get_all_trades().unwrap();
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].symbol, "BTC");
    }

    #[test]
    fn test_get_trades_by_symbol() {
        let repo = TradeRepository::new(":memory:").unwrap();
        let trade = create_test_trade();

        repo.insert_trade(&trade).unwrap();
        let trades = repo.get_trades_by_symbol("BTC").unwrap();
        assert_eq!(trades.len(), 1);
    }

    #[test]
    fn test_count_trades() {
        let repo = TradeRepository::new(":memory:").unwrap();
        let trade = create_test_trade();

        repo.insert_trade(&trade).unwrap();
        let count = repo.count_trades().unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_candle_repository_creation() {
        let repo = CandleRepository::new(":memory:");
        assert!(repo.is_ok());
    }

    #[test]
    fn test_insert_and_retrieve_candles() {
        let repo = CandleRepository::new(":memory:").unwrap();
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

        assert!(repo.insert_candle(&candle).is_ok());
        let candles = repo.get_candles("BTC", "1m").unwrap();
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].close, 105.0);
    }

    #[test]
    fn test_get_last_candles() {
        let repo = CandleRepository::new(":memory:").unwrap();
        let mut candles = Vec::new();
        for i in 0..5 {
            candles.push(Candle {
                symbol: "BTC".to_string(),
                interval: "1m".to_string(),
                timestamp: Utc::now() - chrono::Duration::minutes(5 - i),
                open: 100.0 + i as f64,
                high: 110.0,
                low: 90.0,
                close: 105.0,
                volume: 1000.0,
            });
        }

        repo.insert_candles(&candles).unwrap();
        let last_3 = repo.get_last_candles("BTC", "1m", 3).unwrap();
        assert_eq!(last_3.len(), 3);
    }
}
