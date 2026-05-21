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
    for row in rows {
        if let Ok(res) = row {
            results.push(res);
        }
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
    for row in rows {
        if let Ok(table) = row {
            tables.push(table);
        }
    }
    Ok(tables)
}

/// 📊 SEMBOL ARŞİVİ: `candles` tablosunda mum verisi indirilmiş tüm benzersiz
/// sembolleri döndürür. Otonom tarayıcı (Screener) ve pipeline süreçleri için
/// hedef havuzu oluşturur.
///
/// Önceki davranış (paper_trading_results) yumurta-tavuk problemine sokuyordu:
/// işlem hiç yapılmadıkça tablo boş → screener havuz bulamıyor → yeni işlem
/// olmuyor. `candles` tablosu indirilmiş ham veri havuzu olduğu için doğru
/// kaynak. Sıralama deterministik (alfabetik).
pub fn list_symbols(db_path: &str) -> Result<Vec<String>, crate::MemosTradingError> {
    let conn = Connection::open(db_path)
        .map_err(|e| crate::MemosTradingError::Database(format!("DB bağlantı hatası: {}", e)))?;

    let mut stmt = conn.prepare("SELECT DISTINCT symbol FROM candles ORDER BY symbol")
        .map_err(|e| crate::MemosTradingError::Database(format!("Sorgu hazırlama hatası: {}", e)))?;

    let rows = stmt.query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| crate::MemosTradingError::Database(format!("Sembol listesi okuma hatası: {}", e)))?;

    let mut symbols = Vec::new();
    for row in rows {
        if let Ok(sym) = row {
            symbols.push(sym);
        }
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