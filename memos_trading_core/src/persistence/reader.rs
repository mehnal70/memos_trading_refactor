// src/persistence/reader.rs - Srivastava ATP Adli Okuma Birimi

// ... Mevcut importlar ve struct'lar (SymbolInfo, PaperTradingResult vb.) aynı kalsın ...
use std::fs;
use rusqlite::{Connection,params};
use crate::core::model::{PositionModel,PaperTradingResult};

// --- 3. SRIVASTAVA MODERNİZE OKUMA METODLARI ---

/// Srivastava ATP - Akıllı Konfigürasyon Yükleyici
/// Dosya yoksa veya bozuksa 'Default' dönerek robotun 'Kritik Panik' yapmasını engeller.
pub fn load_config_with_fallback<T: serde::de::DeserializeOwned + Default>(path: &str) -> T {
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
            eprintln!("⚠️ [Reader] JSON parse hatası [{}]: {}. Varsayılanlar yükleniyor.", path, e);
            T::default()
        }),
        Err(_) => {
            eprintln!("ℹ️ [Reader] {} bulunamadı. Temiz profil oluşturuluyor.", path);
            T::default()
        }
    }
}



/// Veritabanından en son 'Adli Durum Snapshot'ını canlandırır.
/// Bu fonksiyon robotun 'ben kimim?' sorusuna yanıtıdır.
pub fn recover_open_positions(db_path: &str) -> Result<Vec<PositionModel>, String> {
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    let json_str: Option<String> = conn.query_row(
        "SELECT positions FROM open_positions_snapshot WHERE id = 1",
        [],

        |row| row.get(0)
    ).ok();

    match json_str {
        Some(s) => serde_json::from_str(&s).map_err(|e| format!("Recovery hatası: {}", e)),
        None => Ok(vec![])
    }
}

/// Persist edilmiş hesap durumu (writer::save_account_state'in ürettiği).
#[derive(Debug, Clone)]
pub struct AccountStateRecord {
    pub equity: f64,
    pub peak_equity: f64,
    pub starting_capital: f64,
    pub closed_trades_count: usize,
    pub updated_at: String,
}

/// Boot'ta önceki run'un `account_state` satırını okur.
/// - Tablo yoksa veya kayıt yoksa: `Ok(None)` (cold-start).
/// - DB açılamaz/parse hatası: `Err(String)`; çağıran cold-start'a düşer.
pub fn load_account_state(db_path: &str) -> Result<Option<AccountStateRecord>, String> {
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    let row = conn.query_row(
        "SELECT equity, peak_equity, starting_capital, closed_trades_count, updated_at \
         FROM account_state WHERE id = 1",
        [],
        |r| Ok(AccountStateRecord {
            equity: r.get(0)?,
            peak_equity: r.get(1)?,
            starting_capital: r.get(2)?,
            closed_trades_count: r.get::<_, i64>(3)? as usize,
            updated_at: r.get(4)?,
        }),
    );
    match row {
        Ok(rec) => Ok(Some(rec)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => {
            // Tablo henüz oluşturulmamış olabilir (eski DB). Bunu cold-start kabul et.
            let msg = e.to_string();
            if msg.contains("no such table") { Ok(None) } else { Err(msg) }
        }
    }
}

// --- 4. GELİŞMİŞ PERFORMANS ANALİTİĞİ (main.rs Tahliyesi İçin) ---

/// Robotun geçmişindeki en başarılı sembolleri süzerek 'Elite Fleet' oluşturur.
pub fn get_top_performing_symbols(db_path: &str, limit: usize) -> Vec<(String, f64)> {
    if let Ok(results) = read_paper_trading_results(db_path, Some(100)) {
        let mut stats: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        for r in results {
            let entry = stats.entry(r.symbol).or_insert(0.0);
            *entry += r.total_pnl_usd;
        }
        let mut sorted: Vec<_> = stats.into_iter().collect();
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        sorted.into_iter().take(limit).collect()
    } else {
        vec![]
    }
}



/// 🔬 ADLİ VERİ HASADI: Veritabanından en taze sanal işlem/backtest sonuçlarını okur
pub fn read_paper_trading_results(db_path: &str, limit: Option<usize>) -> Result<Vec<PaperTradingResult>, crate::MemosTradingError> {
    // 1. Veritabanı Bağlantısı Aç
    let conn = Connection::open(db_path)
        .map_err(|e| crate::MemosTradingError::Database(format!("DB Açılamadı: {}", e)))?;

    let max_rows = limit.unwrap_or(50);

    // 2. Adli Sorguyu Hazırla
    let mut stmt = conn.prepare(
        "SELECT symbol, interval, total_trades, win_trades, loss_trades, win_rate, \
         profit_factor, total_pnl_usd, max_drawdown_pct, sharpe_ratio, tested_at \
         FROM paper_trading_results \
         ORDER BY total_pnl_usd DESC \
         LIMIT ?"
    ).map_err(|e| crate::MemosTradingError::Database(format!("Sorgu Hazırlanamadı: {}", e)))?;

    // 3. Verileri Hasat Et ve Modelle
    let rows = stmt.query_map(params![max_rows], |row| {
        Ok(PaperTradingResult {
            symbol: row.get(0)?,
            interval: row.get(1)?,
            total_trades: row.get(2)?,
            win_trades: row.get(3)?,
            loss_trades: row.get(4)?,
            win_rate: row.get(5)?,
            profit_factor: row.get(6)?,
            total_pnl_usd: row.get(7)?,
            max_drawdown_pct: row.get(8)?,
            sharpe_ratio: row.get(9)?,
            tested_at: row.get(10)?,
        })
    }).map_err(|e| crate::MemosTradingError::Database(format!("Veri Okuma Hatası: {}", e)))?;

    let mut results = Vec::new();
    for res in rows.flatten() {
        results.push(res);
    }

    Ok(results)
}

// ... list_available_tables, list_symbols, read_candles metodları aynı kalabilir ...
/// 🔍 VERİTABANI KEŞFİ: SQLite içindeki tüm mevcut tabloların listesini döndürür.
/// Sistem teşhisi ve otomatik şema doğrulaması için hayati önem taşır.
pub fn list_available_tables(db_path: &str) -> Result<Vec<String>, crate::MemosTradingError> {
    let conn = Connection::open(db_path)
        .map_err(|e| crate::MemosTradingError::Database(format!("DB bağlantı hatası: {}", e)))?;

    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .map_err(|e| crate::MemosTradingError::Database(format!("Sorgu hazırlama hatası: {}", e)))?;

    let rows = stmt.query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| crate::MemosTradingError::Database(format!("Tablo listesi okuma hatası: {}", e)))?;

    let mut tables = Vec::new();
    for table in rows.flatten() {
        tables.push(table);
    }
    Ok(tables)
}

/// 📊 SEMBOL ARŞİVİ: `candles` tablosunda mum verisi indirilmiş tüm benzersiz
/// sembolleri döndürür (market/interval filtresi yok). Otonom tarayıcı
/// (Screener) ve pipeline süreçleri için ham havuz.
///
/// Önceki davranış (paper_trading_results) yumurta-tavuk problemine sokuyordu:
/// işlem hiç yapılmadıkça tablo boş → screener havuz bulamıyor → yeni işlem
/// olmuyor. `candles` tablosu indirilmiş ham veri havuzu olduğu için doğru
/// kaynak. Sıralama deterministik (alfabetik).
///
/// Market/interval'a göre segmentli istek için `list_symbols_for_market`.
pub fn list_symbols(db_path: &str) -> Result<Vec<String>, crate::MemosTradingError> {
    list_symbols_for_market(db_path, None, None)
}

/// 📊 SEGMENTLİ SEMBOL ARŞİVİ: `candles` tablosunda **belirli market ve/veya
/// interval'a** uyan benzersiz sembolleri döndürür. Screener gibi çağrıcılar
/// crypto + BIST karışık havuzu (`candles` ana tablosu) yerine kendi
/// pazarlarına uygun alt-küme isteyebilir.
///
/// - `market = Some("futures")` ve `interval = Some("1m")` → futures-1m
///   sembolleri (config.market + config.interval ile uyumlu)
/// - `market = None, interval = None` → list_symbols ile eşdeğer (tüm havuz)
/// - Parametreler `?` placeholder ile bind, SQL injection güvenli.
pub fn list_symbols_for_market(
    db_path: &str,
    market: Option<&str>,
    interval: Option<&str>,
) -> Result<Vec<String>, crate::MemosTradingError> {
    let conn = Connection::open(db_path)
        .map_err(|e| crate::MemosTradingError::Database(format!("DB bağlantı hatası: {}", e)))?;

    let mut where_clauses: Vec<&str> = Vec::new();
    let mut params: Vec<&str> = Vec::new();
    if let Some(m) = market   { where_clauses.push("market = ?");   params.push(m); }
    if let Some(i) = interval { where_clauses.push("interval = ?"); params.push(i); }

    let sql = if where_clauses.is_empty() {
        "SELECT DISTINCT symbol FROM candles ORDER BY symbol".to_string()
    } else {
        format!(
            "SELECT DISTINCT symbol FROM candles WHERE {} ORDER BY symbol",
            where_clauses.join(" AND "),
        )
    };

    let mut stmt = conn.prepare(&sql)
        .map_err(|e| crate::MemosTradingError::Database(format!("Sorgu hazırlama hatası: {}", e)))?;

    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| row.get::<_, String>(0))
        .map_err(|e| crate::MemosTradingError::Database(format!("Sembol listesi okuma hatası: {}", e)))?;

    let mut symbols = Vec::new();
    for sym in rows.flatten() {
        symbols.push(sym);
    }
    Ok(symbols)
}

/// 🕯️ KRİTİK MUM HASADI: Belirli bir sembol ve interval için geçmiş mum verilerini (`Candle`) RAM'e çeker.
/// Stratejilerin sinyal üretebilmesi için gereken ana yakıttır.
///
/// Ana `candles` tablosunu (symbol, interval, timestamp, open, high, low, close, volume)
/// symbol+interval filtresi ile sorgular. Timestamp DB'de INTEGER milisaniye olarak saklanıyor;
/// geriye dönük uyumluluk için RFC3339 string formatı da kabul edilir.
pub fn read_candles(
    db_path: &str,
    symbol: &str,
    interval: &str,
    limit: usize,
) -> Result<Vec<crate::core::types::Candle>, crate::MemosTradingError> {
    use chrono::{DateTime, TimeZone, Utc};
    use rusqlite::types::ValueRef;

    let conn = Connection::open(db_path)
        .map_err(|e| crate::MemosTradingError::Database(format!("DB bağlantı hatası: {}", e)))?;

    let query = "SELECT timestamp, open, high, low, close, volume, symbol, interval \
                 FROM candles \
                 WHERE symbol = ?1 AND interval = ?2 \
                 ORDER BY timestamp DESC LIMIT ?3";

    let mut stmt = conn.prepare(query).map_err(|e| {
        crate::MemosTradingError::Database(format!("candles sorgusu hazırlanamadı: {}", e))
    })?;

    let rows = stmt.query_map(params![symbol, interval, limit as i64], |row| {
        // timestamp INTEGER (ms) ya da TEXT (RFC3339) olabilir → her ikisini de destekle.
        let ts: DateTime<Utc> = match row.get_ref(0)? {
            ValueRef::Integer(ms) => Utc.timestamp_millis_opt(ms).single()
                .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap()),
            ValueRef::Text(b) => {
                let s = std::str::from_utf8(b).unwrap_or("");
                DateTime::parse_from_rfc3339(s)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc.timestamp_opt(0, 0).single().unwrap())
            }
            _ => Utc.timestamp_opt(0, 0).single().unwrap(),
        };
        Ok(crate::core::types::Candle {
            timestamp: ts,
            open:   row.get(1)?,
            high:   row.get(2)?,
            low:    row.get(3)?,
            close:  row.get(4)?,
            volume: row.get(5)?,
            symbol: row.get(6)?,
            interval: row.get(7)?,
        })
    }).map_err(|e| crate::MemosTradingError::Database(format!("Mum hasat hatası: {}", e)))?;

    let mut candles = Vec::with_capacity(limit.min(4096));
    for row in rows {
        match row {
            Ok(candle) => candles.push(candle),
            Err(e) => {
                log::warn!("read_candles: bozuk satır atlandı ({}): {}", symbol, e);
            }
        }
    }

    // SQL'den en yeniden eskiye geldi (DESC); indikatörlerin doğru çalışması için kronolojik sıraya (ASC) çeviriyoruz
    candles.reverse();
    Ok(candles)
}

/// 🗂️ Sembol-statü registry'sini DB'den okur (boot hydrate). Tablo henüz yoksa boş
/// döner (ilk çalıştırmada refresh job doldurana kadar). Dönen (symbol, status) listesi
/// `set_symbol_statuses` ile cache'e yüklenir.
pub fn load_symbol_statuses(db_path: &str) -> crate::Result<Vec<(String, String)>> {
    let conn = Connection::open(db_path)
        .map_err(|e| crate::MemosTradingError::Database(format!("DB bağlantı hatası: {}", e)))?;
    let mut stmt = match conn.prepare("SELECT symbol, status FROM symbol_status") {
        Ok(s) => s,
        Err(_) => return Ok(Vec::new()), // tablo yok → boş
    };
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| crate::MemosTradingError::Database(format!("symbol_status sorgu: {}", e)))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}