use std::sync::mpsc::{self, Sender, Receiver};
use std::thread;

/// DB'ye yazma mesajı
pub enum DBWriteMsg {
    Candle {
        exchange: String,
        market: String,
        candle: Candle,
    },
    // Gerekirse başka mesaj türleri eklenebilir
}

/// DBWriter: işçi thread ve kanal ile async DB yazıcı
pub struct DBWriter {
    sender: Sender<DBWriteMsg>,
}

impl DBWriter {
    /// Yeni bir DBWriter başlatır, işçi thread'i açar
    pub fn new(conn: Connection) -> Self {
        let (tx, rx): (Sender<DBWriteMsg>, Receiver<DBWriteMsg>) = mpsc::channel();
        thread::spawn(move || {
            DBWriter::worker_loop(conn, rx);
        });
        DBWriter { sender: tx }
    }

    /// Candle yazma isteği gönderir
    pub fn write_candle(&self, exchange: &str, market: &str, candle: Candle) {
        let msg = DBWriteMsg::Candle {
            exchange: exchange.to_string(),
            market: market.to_string(),
            candle,
        };
        let _ = self.sender.send(msg);
    }

    /// İşçi thread loop'u
    fn worker_loop(conn: Connection, rx: Receiver<DBWriteMsg>) {
        for msg in rx {
            match msg {
                DBWriteMsg::Candle { exchange, market, candle } => {
                    let _ = save_candle(&conn, &exchange, &market, &candle);
                }
            }
        }
    }
}
use rusqlite::{Connection, params};
use crate::types::Candle;
use crate::Result;

/// İzin verilen exchange ve market değerleri — SQL injection koruması için whitelist.
const VALID_EXCHANGES: &[&str] = &["binance", "bist", "bybit", "kucoin", "coinbase"];
const VALID_MARKETS:   &[&str] = &["spot", "futures", "coinm", "margin"];

/// Tablo adını exchange ve market'e göre oluştur.
/// exchange/market whitelist dışındaysa hata logu yazılır ve "candles_unknown_unknown" döner
/// (SQL injection koruması — tablo adı doğrudan sorguya gömülür).
pub fn get_table_name(exchange: &str, market: &str) -> String {
    let safe_exchange = if VALID_EXCHANGES.contains(&exchange) {
        exchange
    } else {
        log::error!("get_table_name: geçersiz exchange='{}', whitelist: {:?}", exchange, VALID_EXCHANGES);
        "unknown"
    };
    let safe_market = if VALID_MARKETS.contains(&market) {
        market
    } else {
        log::error!("get_table_name: geçersiz market='{}', whitelist: {:?}", market, VALID_MARKETS);
        "unknown"
    };
    format!("candles_{}_{}", safe_exchange, safe_market)
}

/// Veritabanı bağlantısı aç
pub fn open_connection(db_path: &str) -> Result<Connection> {
    let conn = Connection::open(db_path)?;
    // WAL modu ve performans ayarları
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA cache_size = 1000000000;
         PRAGMA locking_mode = NORMAL;
         PRAGMA temp_store = MEMORY;",
    )?;
    Ok(conn)
}

/// Exchange/Market tablosunu oluştur
pub fn ensure_table(conn: &Connection, exchange: &str, market: &str) -> Result<()> {
    let table_name = get_table_name(exchange, market);
    let create_sql = format!(
        "CREATE TABLE IF NOT EXISTS {} (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            symbol TEXT NOT NULL,
            interval TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            open REAL NOT NULL,
            high REAL NOT NULL,
            low REAL NOT NULL,
            close REAL NOT NULL,
            volume REAL NOT NULL,
            UNIQUE(symbol, interval, timestamp)
        )",
        table_name
    );
    conn.execute(&create_sql, [])?;
    
    // Index oluştur
    let index_sql = format!(
        "CREATE INDEX IF NOT EXISTS idx_{}_unique ON {} (symbol, interval, timestamp)",
        table_name, table_name
    );
    conn.execute(&index_sql, [])?;
    
    Ok(())
}

/// Candle kaydı kaydet.
/// Returns: true if inserted, false if duplicate (UNIQUE constraint).
///
/// Değişiklikler:
/// - `timestamp_millis()` kullanılır: saniye hassasiyeti 1000ms'lik çakışmalara yol açıyordu.
/// - `INSERT OR IGNORE`: önceki SELECT COUNT+INSERT çiftinin race condition'ını ve
///   ekstra round-trip'ini ortadan kaldırır. UNIQUE(symbol, interval, timestamp) kısıtı
///   DB katmanında garantilenir.
pub fn save_candle(
    conn: &Connection,
    exchange: &str,
    market: &str,
    candle: &Candle,
) -> Result<bool> {
    let table_name = get_table_name(exchange, market);

    ensure_table(conn, exchange, market)?;

    // Milisaniye hassasiyeti — saniye kesimi 1000ms içindeki çakışmalara neden oluyordu
    let ts_ms = candle.timestamp.timestamp_millis();

    let insert_query = format!(
        "INSERT OR IGNORE INTO {} (symbol, interval, timestamp, open, high, low, close, volume)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        table_name
    );

    let affected = conn.execute(
        &insert_query,
        params![
            &candle.symbol,
            &candle.interval,
            ts_ms,
            candle.open,
            candle.high,
            candle.low,
            candle.close,
            candle.volume
        ],
    )?;

    Ok(affected > 0) // 0 = IGNORE (duplicate), 1 = inserted
}

/// Toplu candle kaydetme (transaction ile)
pub fn save_candles_bulk(
    conn: &Connection,
    exchange: &str,
    market: &str,
    candles: &[Candle],
) -> Result<(usize, usize)> {
    ensure_table(conn, exchange, market)?;
    
    let tx = conn.unchecked_transaction()?;
    let mut inserted = 0;
    let mut skipped = 0;
    
    for candle in candles {
        match save_candle(&tx, exchange, market, candle) {
            Ok(true) => inserted += 1,
            Ok(false) => skipped += 1,
            Err(_) => skipped += 1, // Hata durumunda skip
        }
    }
    
    tx.commit()?;
    Ok((inserted, skipped))
}

/// Backtest sonuçları tablosu oluştur
pub fn ensure_backtest_results_table(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS backtest_results (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            strategy TEXT NOT NULL,
            exchange TEXT NOT NULL,
            market TEXT NOT NULL,
            symbol TEXT NOT NULL,
            interval TEXT NOT NULL,
            ma_fast INTEGER,
            ma_slow INTEGER,
            rsi_period INTEGER,
            rsi_overbought REAL,
            rsi_oversold REAL,
            macd_fast INTEGER,
            macd_slow INTEGER,
            macd_signal INTEGER,
            bb_period INTEGER,
            bb_std_dev REAL,
            stochastic_k INTEGER,
            stochastic_oversold REAL,
            stochastic_overbought REAL,
            williams_period INTEGER,
            williams_oversold REAL,
            williams_overbought REAL,
            adx_period INTEGER,
            adx_threshold REAL,
            vwap_period INTEGER,
            vwap_std_dev REAL,
            initial_balance REAL NOT NULL,
            total_trades INTEGER NOT NULL,
            winning_trades INTEGER NOT NULL,
            losing_trades INTEGER NOT NULL,
            win_rate REAL NOT NULL,
            total_pnl REAL NOT NULL,
            final_balance REAL NOT NULL,
            created_at TEXT NOT NULL,
            notes TEXT
        )",
        [],
    )?;
    
    // Index for fast lookup
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_backtest_strategy_date ON backtest_results (strategy, created_at)",
        [],
    )?;
    // Mevcut DB'ye eksik kolonları ekle — hata (zaten var) sessizce göz ardı edilir
    let _ = conn.execute("ALTER TABLE backtest_results ADD COLUMN profit_factor REAL DEFAULT 0.0", []);
    let _ = conn.execute("ALTER TABLE backtest_results ADD COLUMN sharpe_ratio REAL DEFAULT 0.0", []);
    let _ = conn.execute("ALTER TABLE backtest_results ADD COLUMN max_drawdown_pct REAL DEFAULT 0.0", []);
    Ok(())
}

/// Backtest sonucunu kaydet
pub fn save_backtest_result(
    conn: &Connection,
    strategy: &str,
    exchange: &str,
    market: &str,
    symbol: &str,
    interval: &str,
    ma_fast: Option<i32>,
    ma_slow: Option<i32>,
    rsi_period: Option<i32>,
    rsi_overbought: Option<f64>,
    rsi_oversold: Option<f64>,
    macd_fast: Option<i32>,
    macd_slow: Option<i32>,
    macd_signal: Option<i32>,
    bb_period: Option<i32>,
    bb_std_dev: Option<f64>,
    stochastic_k: Option<i32>,
    stochastic_oversold: Option<f64>,
    stochastic_overbought: Option<f64>,
    williams_period: Option<i32>,
    williams_oversold: Option<f64>,
    williams_overbought: Option<f64>,
    adx_period: Option<i32>,
    adx_threshold: Option<f64>,
    vwap_period: Option<i32>,
    vwap_std_dev: Option<f64>,
    initial_balance: f64,
    total_trades: i32,
    winning_trades: i32,
    losing_trades: i32,
    win_rate: f64,
    total_pnl: f64,
    final_balance: f64,
    profit_factor: f64,
    sharpe_ratio: f64,
    max_drawdown_pct: f64,
    notes: Option<&str>,
) -> Result<i64> {
    ensure_backtest_results_table(conn)?;

    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO backtest_results
         (strategy, exchange, market, symbol, interval, ma_fast, ma_slow, rsi_period, rsi_overbought, rsi_oversold,
          macd_fast, macd_slow, macd_signal, bb_period, bb_std_dev, stochastic_k, stochastic_oversold, stochastic_overbought,
          williams_period, williams_oversold, williams_overbought, adx_period, adx_threshold, vwap_period, vwap_std_dev,
          initial_balance, total_trades, winning_trades, losing_trades, win_rate, total_pnl, final_balance,
          profit_factor, sharpe_ratio, max_drawdown_pct, created_at, notes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36, ?37)",
        params![
            strategy, exchange, market, symbol, interval, ma_fast, ma_slow, rsi_period,
            rsi_overbought, rsi_oversold, macd_fast, macd_slow, macd_signal, bb_period, bb_std_dev,
            stochastic_k, stochastic_oversold, stochastic_overbought, williams_period, williams_oversold, williams_overbought,
            adx_period, adx_threshold, vwap_period, vwap_std_dev,
            initial_balance, total_trades, winning_trades, losing_trades, win_rate, total_pnl, final_balance,
            profit_factor, sharpe_ratio, max_drawdown_pct, now, notes
        ],
    )?;

    Ok(conn.last_insert_rowid())
}

/// Piyasa koşullarına göre en iyi strateji+parametre kombinasyonunu saklar.
/// Her (strategy, interval, market, condition_key) için tek kayıt tutulur (UPSERT).
pub fn ensure_pattern_library_table(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS pattern_library (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            strategy        TEXT NOT NULL,
            params_json     TEXT NOT NULL,
            interval        TEXT NOT NULL,
            exchange        TEXT NOT NULL,
            market          TEXT NOT NULL,
            symbol          TEXT,
            trend           TEXT NOT NULL,
            volatility      TEXT NOT NULL,
            momentum        TEXT NOT NULL,
            condition_key   TEXT NOT NULL,
            win_rate        REAL NOT NULL,
            avg_pnl         REAL NOT NULL,
            trade_count     INTEGER NOT NULL,
            confidence      REAL NOT NULL,
            last_updated    TEXT NOT NULL,
            UNIQUE(strategy, interval, market, condition_key) ON CONFLICT REPLACE
        )",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_pattern_lookup \
         ON pattern_library (strategy, interval, market, condition_key)",
        [],
    )?;
    Ok(())
}

/// Pattern library'e kayıt ekle veya güncelle
pub fn save_pattern(
    conn: &Connection,
    strategy: &str,
    params_json: &str,
    interval: &str,
    exchange: &str,
    market: &str,
    symbol: Option<&str>,
    trend: &str,
    volatility: &str,
    momentum: &str,
    win_rate: f64,
    avg_pnl: f64,
    trade_count: i64,
    confidence: f64,
) -> Result<()> {
    ensure_pattern_library_table(conn)?;
    let condition_key = format!("{}|{}|{}", trend, volatility, momentum);
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR REPLACE INTO pattern_library
         (strategy, params_json, interval, exchange, market, symbol,
          trend, volatility, momentum, condition_key,
          win_rate, avg_pnl, trade_count, confidence, last_updated)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        rusqlite::params![
            strategy, params_json, interval, exchange, market, symbol,
            trend, volatility, momentum, condition_key,
            win_rate, avg_pnl, trade_count, confidence, now
        ],
    )?;
    Ok(())
}

/// Mevcut koşullara en uygun pattern'ı döner.
/// min_win_rate ve min_trades eşiği altındaki kayıtlar yok sayılır.
pub fn query_best_pattern(
    conn: &Connection,
    strategy: &str,
    interval: &str,
    market: &str,
    condition_key: &str,
    min_win_rate: f64,
    min_trades: i64,
) -> Option<(f64, f64, i64, f64)> { // (win_rate, avg_pnl, trade_count, confidence)
    let _ = ensure_pattern_library_table(conn);
    let sql = "SELECT win_rate, avg_pnl, trade_count, confidence \
               FROM pattern_library \
               WHERE strategy=?1 AND interval=?2 AND market=?3 \
                 AND condition_key=?4 AND win_rate>=?5 AND trade_count>=?6 \
               ORDER BY confidence DESC LIMIT 1";
    conn.query_row(
        sql,
        rusqlite::params![strategy, interval, market, condition_key, min_win_rate, min_trades],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).ok()
}

/// Binance kline array'ini Candle'a parse et
pub fn parse_binance_kline(data: &serde_json::Value, symbol: &str, interval: &str) -> Option<Candle> {
    let arr = data.as_array()?;
    if arr.len() < 8 {
        return None;
    }
    
    let ts_ms = arr[0].as_i64()?;
    let open   = arr[1].as_str()?.parse::<f64>().ok()?;
    let high   = arr[2].as_str()?.parse::<f64>().ok()?;
    let low    = arr[3].as_str()?.parse::<f64>().ok()?;
    let close  = arr[4].as_str()?.parse::<f64>().ok()?;
    let volume = arr[5].as_str()?.parse::<f64>().ok()?;

    // OHLCV temel bütünlük kontrolü — fiziksel tutarsızlık (high=0, high<open vb.)
    // borsa veri akışında bazen görülür; bu mum aşağı akışa hiç gitmemeli.
    if high <= 0.0 || low <= 0.0 || open <= 0.0 || close <= 0.0 || volume < 0.0 {
        return None;
    }
    if high < low || high < open.max(close) || low > open.min(close) {
        return None;
    }

    Some(Candle {
        timestamp: chrono::DateTime::from_timestamp_millis(ts_ms)?,
        open,
        high,
        low,
        close,
        volume,
        symbol: symbol.to_string(),
        interval: interval.to_string(),
    })
}

// ── Açık Pozisyon Snapshot Persistence ───────────────────────────────────────

/// Açık pozisyonlar için tek satırlık JSON snapshot tablosu.
/// Sistem crash veya yeniden başlatma sonrasında pozisyonlar geri yüklenir.
pub fn ensure_open_positions_snapshot_table(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS open_positions_snapshot (
            id          INTEGER PRIMARY KEY CHECK (id = 1), -- tek satır
            positions   TEXT NOT NULL,                       -- JSON array
            updated_at  TEXT NOT NULL
        )",
        [],
    )?;
    Ok(())
}

/// Mevcut açık pozisyonları JSON olarak kaydeder (upsert).
/// `positions_json`: `serde_json::to_string(&open_positions.values().collect::<Vec<_>>())`
pub fn save_open_positions_snapshot(conn: &Connection, positions_json: &str) -> Result<()> {
    ensure_open_positions_snapshot_table(conn)?;
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO open_positions_snapshot (id, positions, updated_at) VALUES (1, ?1, ?2)
         ON CONFLICT(id) DO UPDATE SET positions=excluded.positions, updated_at=excluded.updated_at",
        params![positions_json, now],
    )?;
    Ok(())
}

/// Son kaydedilen pozisyon JSON snapshot'ını döner.
/// Yoksa `None` döner.
pub fn load_open_positions_snapshot(conn: &Connection) -> Option<String> {
    let _ = ensure_open_positions_snapshot_table(conn);
    conn.query_row(
        "SELECT positions FROM open_positions_snapshot WHERE id=1",
        [],
        |row| row.get::<_, String>(0),
    ).ok()
}

/// [z] reset sırasında DB pozisyon snapshot'ını sil.
/// Sıfırlanmadan sonra yeni loop DB'den eski pozisyonları geri yüklemez.
pub fn clear_open_positions_snapshot(conn: &Connection) -> Result<()> {
    let _ = ensure_open_positions_snapshot_table(conn);
    conn.execute("DELETE FROM open_positions_snapshot WHERE id=1", [])?;
    Ok(())
}
