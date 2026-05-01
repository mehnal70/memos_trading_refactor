// SQLite veritabanından veri okuma modülü
use crate::types::Candle;
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolInfo {
    pub exchange: String,
    pub market: String,
    pub symbol: String,
    pub interval: String,
    pub first_timestamp: Option<i64>,
    pub last_timestamp: Option<i64>,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperTradingResult {
    pub id: i32,
    pub exchange: String,
    pub market: String,
    pub symbol: String,
    pub interval: String,
    pub strategy_name: String,
    pub total_trades: i32,
    pub win_rate: f64,
    pub profit_loss_pct: f64,
    pub sharpe_ratio: Option<f64>,
    pub max_drawdown_pct: Option<f64>,
    pub tested_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioPosition {
    pub id: i32,
    pub exchange: String,
    pub market: String,
    pub symbol: String,
    pub position_type: String, // "LONG" veya "SHORT"
    pub entry_price: f64,
    pub quantity: f64,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub current_pnl_pct: Option<f64>,
    pub opened_at: Option<DateTime<Utc>>,
}

/// Veritabanındaki exchange/market kombinasyonlarını listele
pub fn list_available_tables(db_path: &str) -> Result<Vec<(String, String)>, String> {
    use rusqlite::Connection;
    
    let conn = Connection::open(db_path).map_err(|e| format!("DB açılamadı: {}", e))?;
    
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'candles_%' AND name NOT IN ('candles_backup')"
    ).map_err(|e| format!("SQL hatası: {}", e))?;
    
    let mut tables = Vec::new();
    let rows = stmt.query_map([], |row| {
        let table_name: String = row.get(0)?;
        Ok(table_name)
    }).map_err(|e| format!("Query hatası: {}", e))?;
    
    for row_result in rows {
        if let Ok(table_name) = row_result {
            // candles_binance_spot -> (binance, spot)
            if let Some(rest) = table_name.strip_prefix("candles_") {
                let parts: Vec<&str> = rest.splitn(2, '_').collect();
                if parts.len() == 2 {
                    tables.push((parts[0].to_string(), parts[1].to_string()));
                }
            }
        }
    }
    
    Ok(tables)
}

/// Belirli bir exchange/market için mevcut sembolleri listele
pub fn list_symbols(
    db_path: &str,
    exchange: &str,
    market: &str,
) -> Result<Vec<SymbolInfo>, String> {
    use rusqlite::Connection;
    
    let conn = Connection::open(db_path).map_err(|e| format!("DB açılamadı: {}", e))?;
    let table_name = format!("candles_{}_{}", exchange, market);
    
    // Tablo var mı kontrol et
    let table_exists: bool = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [&table_name],
        |row| {
            let count: i32 = row.get(0)?;
            Ok(count > 0)
        }
    ).map_err(|e| format!("Tablo kontrolü hatası: {}", e))?;
    
    if !table_exists {
        return Ok(Vec::new());
    }
    
    let query = format!(
        "SELECT symbol, MIN(interval) as interval, MIN(timestamp) as first_ts, MAX(timestamp) as last_ts, COUNT(*) as cnt
         FROM {}
         GROUP BY symbol
         ORDER BY symbol",
        table_name
    );
    
    let mut stmt = conn.prepare(&query).map_err(|e| format!("SQL hatası: {}", e))?;
    
    let rows = stmt.query_map([], |row| {
        Ok(SymbolInfo {
            exchange: exchange.to_string(),
            market: market.to_string(),
            symbol: row.get(0)?,
            interval: row.get(1)?,
            first_timestamp: row.get(2).ok(),
            last_timestamp: row.get(3).ok(),
            count: row.get::<_, i64>(4)? as usize,
        })
    }).map_err(|e| format!("Query hatası: {}", e))?;
    
    let mut symbols = Vec::new();
    for row_result in rows {
        if let Ok(info) = row_result {
            symbols.push(info);
        }
    }
    
    Ok(symbols)
}

/// Belirli bir sembol için candle verilerini oku
pub fn read_candles(
    db_path: &str,
    exchange: &str,
    market: &str,
    symbol: &str,
    interval: &str,
    limit: Option<usize>,
) -> Result<Vec<Candle>, String> {
    use rusqlite::Connection;
    
    let conn = Connection::open(db_path).map_err(|e| format!("DB açılamadı: {}", e))?;
    let table_name = format!("candles_{}_{}", exchange, market);
    
    let query = if let Some(lim) = limit {
        format!(
            "SELECT timestamp, open, high, low, close, volume
             FROM {}
             WHERE symbol = ?1 AND interval = ?2
             ORDER BY timestamp DESC
             LIMIT {}",
            table_name, lim
        )
    } else {
        format!(
            "SELECT timestamp, open, high, low, close, volume
             FROM {}
             WHERE symbol = ?1 AND interval = ?2
             ORDER BY timestamp DESC",
            table_name
        )
    };
    
    let mut stmt = conn.prepare(&query).map_err(|e| format!("SQL hatası: {}", e))?;
    
    let rows = stmt.query_map([symbol, interval], |row| {
        let timestamp_ms: i64 = row.get(0)?;
        let timestamp = DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
            .unwrap_or(Utc::now());
        
        Ok(Candle {
            timestamp,
            symbol: symbol.to_string(),
            interval: interval.to_string(),
            open: row.get(1)?,
            high: row.get(2)?,
            low: row.get(3)?,
            close: row.get(4)?,
            volume: row.get(5)?,
        })
    }).map_err(|e| format!("Query hatası: {}", e))?;
    
    let mut candles = Vec::new();
    for row_result in rows {
        if let Ok(candle) = row_result {
            candles.push(candle);
        }
    }
    
    // Zamana göre sırala (eskiden yeniye)
    candles.reverse();
    
    Ok(candles)
}

/// Paper trading sonuçlarını oku
pub fn read_paper_trading_results(
    db_path: &str,
    limit: Option<usize>,
) -> Result<Vec<PaperTradingResult>, String> {
    use rusqlite::Connection;
    
    let conn = Connection::open(db_path).map_err(|e| format!("DB açılamadı: {}", e))?;
    
    // Tablo var mı kontrol et
    let table_exists: bool = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='paper_trading_results'",
        [],
        |row| {
            let count: i32 = row.get(0)?;
            Ok(count > 0)
        }
    ).map_err(|e| format!("Tablo kontrolü hatası: {}", e))?;
    
    if !table_exists {
        return Ok(Vec::new());
    }
    
    let query = if let Some(lim) = limit {
        format!(
            "SELECT id, exchange, market, symbol, interval, strategies, 
                    total_trades, win_rate, total_pnl_pct, sharpe_ratio, 
                    max_drawdown_pct, created_at
             FROM paper_trading_results
             ORDER BY total_pnl_pct DESC
             LIMIT {}",
            lim
        )
    } else {
        "SELECT id, exchange, market, symbol, interval, strategies, 
                total_trades, win_rate, total_pnl_pct, sharpe_ratio, 
                max_drawdown_pct, created_at
         FROM paper_trading_results
         ORDER BY total_pnl_pct DESC".to_string()
    };
    
    let mut stmt = conn.prepare(&query).map_err(|e| format!("SQL hatası: {}", e))?;
    
    let rows = stmt.query_map([], |row| {
        let created_at_ms: Option<i64> = row.get(11).ok();
        let tested_at = created_at_ms.and_then(|ms| {
            DateTime::<Utc>::from_timestamp_millis(ms)
        });
        
        Ok(PaperTradingResult {
            id: row.get(0)?,
            exchange: row.get(1)?,
            market: row.get(2)?,
            symbol: row.get(3)?,
            interval: row.get(4)?,
            strategy_name: row.get(5)?,
            total_trades: row.get(6)?,
            win_rate: row.get(7)?,
            profit_loss_pct: row.get(8)?,
            sharpe_ratio: row.get(9).ok(),
            max_drawdown_pct: row.get(10).ok(),
            tested_at,
        })
    }).map_err(|e| format!("Query hatası: {}", e))?;
    
    let mut results = Vec::new();
    for row_result in rows {
        if let Ok(result) = row_result {
            results.push(result);
        }
    }
    
    Ok(results)
}

/// Portföy pozisyonlarını oku
pub fn read_portfolio(db_path: &str) -> Result<Vec<PortfolioPosition>, String> {
    use rusqlite::Connection;
    
    let conn = Connection::open(db_path).map_err(|e| format!("DB açılamadı: {}", e))?;
    
    // Tablo var mı kontrol et
    let table_exists: bool = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='portfolio'",
        [],
        |row| {
            let count: i32 = row.get(0)?;
            Ok(count > 0)
        }
    ).map_err(|e| format!("Tablo kontrolü hatası: {}", e))?;
    
    if !table_exists {
        return Ok(Vec::new());
    }
    
    let query = "SELECT id, exchange, market, symbol, position_type, entry_price, 
                        quantity, stop_loss, take_profit, current_pnl_pct, opened_at
                 FROM portfolio
                 WHERE status = 'OPEN'
                 ORDER BY opened_at DESC";
    
    let mut stmt = conn.prepare(query).map_err(|e| format!("SQL hatası: {}", e))?;
    
    let rows = stmt.query_map([], |row| {
        let opened_at_str: Option<String> = row.get(10).ok();
        let opened_at = opened_at_str.and_then(|s| {
            DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))
        });
        
        Ok(PortfolioPosition {
            id: row.get(0)?,
            exchange: row.get(1)?,
            market: row.get(2)?,
            symbol: row.get(3)?,
            position_type: row.get(4)?,
            entry_price: row.get(5)?,
            quantity: row.get(6)?,
            stop_loss: row.get(7).ok(),
            take_profit: row.get(8).ok(),
            current_pnl_pct: row.get(9).ok(),
            opened_at,
        })
    }).map_err(|e| format!("Query hatası: {}", e))?;
    
    let mut positions = Vec::new();
    for row_result in rows {
        if let Ok(pos) = row_result {
            positions.push(pos);
        }
    }
    
    Ok(positions)
}
