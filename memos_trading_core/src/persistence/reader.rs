// src/persistence/reader.rs - Srivastava ATP Adli Okuma Birimi

// ... Mevcut importlar ve struct'lar (SymbolInfo, PaperTradingResult vb.) aynı kalsın ...
use crate::core::model::{PaperTradingResult, PositionModel};
use rusqlite::{params, Connection};
use std::fs;

pub fn load_uncertain_orders_for_profile(
    db_path: &str,
    profile: &str,
) -> Result<Vec<crate::core::model::UncertainOrderState>, String> {
    let conn = crate::persistence::open_db(db_path).map_err(|e| e.to_string())?;
    let mut stmt = match conn.prepare(
        "SELECT symbol, side, requested_quantity, filled_quantity, broker_order_id,
                status, reason, created_at, last_checked_at, resolved_at
                , healthy_reconcile_count
         FROM uncertain_orders
         WHERE profile=?1 AND resolved_at IS NULL
         ORDER BY created_at",
    ) {
        Ok(stmt) => stmt,
        Err(error) if error.to_string().contains("no such table") => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let rows = stmt
        .query_map(params![profile], |row| {
            Ok(crate::core::model::UncertainOrderState {
                symbol: row.get(0)?,
                side: row.get(1)?,
                requested_quantity: row.get(2)?,
                filled_quantity: row.get(3)?,
                broker_order_id: row.get(4)?,
                status: row.get(5)?,
                reason: row.get(6)?,
                created_at: row.get(7)?,
                last_checked_at: row.get(8)?,
                resolved_at: row.get(9)?,
                healthy_reconcile_count: row.get::<_, i64>(10)? as u32,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("uncertain_orders recovery hatası: {e}"))
}

pub fn load_pending_closes_for_profile(
    db_path: &str,
    profile: &str,
) -> Result<Vec<(memos_venue_api::CanonicalPositionKey, String)>, String> {
    let conn = crate::persistence::open_db(db_path).map_err(|e| e.to_string())?;
    let mut stmt = match conn.prepare(
        "SELECT position_key, broker_order_id FROM pending_closes WHERE profile=?1",
    ) {
        Ok(stmt) => stmt,
        Err(error) if error.to_string().contains("no such table") => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let rows = stmt
        .query_map(params![profile], |row| {
            let raw_key: String = row.get(0)?;
            let key = serde_json::from_str(&raw_key).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok((key, row.get(1)?))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("pending_closes recovery hatası: {e}"))
}

// --- 3. SRIVASTAVA MODERNİZE OKUMA METODLARI ---

/// Srivastava ATP - Akıllı Konfigürasyon Yükleyici
/// Dosya yoksa veya bozuksa 'Default' dönerek robotun 'Kritik Panik' yapmasını engeller.
pub fn load_config_with_fallback<T: serde::de::DeserializeOwned + Default>(path: &str) -> T {
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
            eprintln!(
                "⚠️ [Reader] JSON parse hatası [{}]: {}. Varsayılanlar yükleniyor.",
                path, e
            );
            T::default()
        }),
        Err(_) => {
            eprintln!(
                "ℹ️ [Reader] {} bulunamadı. Temiz profil oluşturuluyor.",
                path
            );
            T::default()
        }
    }
}

/// Veritabanından en son 'Adli Durum Snapshot'ını canlandırır.
/// Bu fonksiyon robotun 'ben kimim?' sorusuna yanıtıdır.
pub fn recover_open_positions(db_path: &str) -> Result<Vec<PositionModel>, String> {
    recover_open_positions_for_profile(db_path, "default")
}

pub fn recover_open_positions_for_profile(
    db_path: &str,
    profile: &str,
) -> Result<Vec<PositionModel>, String> {
    let conn = crate::persistence::open_db(db_path).map_err(|e| e.to_string())?;

    let json_str: Option<String> = conn
        .query_row(
            "SELECT positions FROM open_positions_snapshot WHERE profile = ?1",
            params![profile],
            |row| row.get(0),
        )
        .ok();

    match json_str {
        Some(s) => serde_json::from_str(&s).map_err(|e| format!("Recovery hatası: {}", e)),
        None => Ok(vec![]),
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

#[derive(Debug, Clone, Default)]
pub struct ClosedTradesSnapshotRecord {
    pub trades: Vec<crate::core::model::ClosedTradeModel>,
    pub equity_history: Vec<f64>,
}

pub fn load_closed_trades_snapshot_for_profile(
    db_path: &str,
    profile: &str,
) -> Result<Option<ClosedTradesSnapshotRecord>, String> {
    let conn = crate::persistence::open_db(db_path).map_err(|e| e.to_string())?;
    let row = conn.query_row(
        "SELECT trades, equity_history FROM closed_trades_snapshot WHERE profile = ?1",
        params![profile],
        |r| {
            let trades_json: String = r.get(0)?;
            let equity_json: String = r.get(1)?;
            Ok((trades_json, equity_json))
        },
    );
    match row {
        Ok((trades_json, equity_json)) => {
            let trades = serde_json::from_str(&trades_json)
                .map_err(|e| format!("Kapalı işlem recovery hatası: {}", e))?;
            let equity_history = serde_json::from_str(&equity_json)
                .map_err(|e| format!("Equity history recovery hatası: {}", e))?;
            Ok(Some(ClosedTradesSnapshotRecord {
                trades,
                equity_history,
            }))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) if e.to_string().contains("no such table") => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Boot'ta önceki run'un `account_state` satırını okur.
/// - Tablo yoksa veya kayıt yoksa: `Ok(None)` (cold-start).
/// - DB açılamaz/parse hatası: `Err(String)`; çağıran cold-start'a düşer.
pub fn load_account_state(db_path: &str) -> Result<Option<AccountStateRecord>, String> {
    load_account_state_for_profile(db_path, "default")
}

pub fn load_account_state_for_profile(
    db_path: &str,
    profile: &str,
) -> Result<Option<AccountStateRecord>, String> {
    let conn = crate::persistence::open_db(db_path).map_err(|e| e.to_string())?;
    let row = conn.query_row(
        "SELECT equity, peak_equity, starting_capital, closed_trades_count, updated_at \
         FROM account_state WHERE profile = ?1",
        params![profile],
        |r| {
            Ok(AccountStateRecord {
                equity: r.get(0)?,
                peak_equity: r.get(1)?,
                starting_capital: r.get(2)?,
                closed_trades_count: r.get::<_, i64>(3)? as usize,
                updated_at: r.get(4)?,
            })
        },
    );
    match row {
        Ok(rec) => Ok(Some(rec)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => {
            // Tablo henüz oluşturulmamış olabilir (eski DB). Bunu cold-start kabul et.
            let msg = e.to_string();
            if msg.contains("no such table") {
                Ok(None)
            } else {
                Err(msg)
            }
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
        return vec![];
    }
}

/// 🔬 ADLİ VERİ HASADI: Veritabanından en taze sanal işlem/backtest sonuçlarını okur
pub fn read_paper_trading_results(
    db_path: &str,
    limit: Option<usize>,
) -> Result<Vec<PaperTradingResult>, crate::MemosTradingError> {
    // 1. Veritabanı Bağlantısı Aç
    let conn = crate::persistence::open_db(db_path)
        .map_err(|e| crate::MemosTradingError::Database(format!("DB Açılamadı: {}", e)))?;

    let max_rows = limit.unwrap_or(50);

    // 2. Adli Sorguyu Hazırla
    let mut stmt = conn
        .prepare(
            "SELECT symbol, interval, total_trades, win_trades, loss_trades, win_rate, \
         profit_factor, total_pnl_usd, max_drawdown_pct, sharpe_ratio, tested_at \
         FROM paper_trading_results \
         ORDER BY total_pnl_usd DESC \
         LIMIT ?",
        )
        .map_err(|e| crate::MemosTradingError::Database(format!("Sorgu Hazırlanamadı: {}", e)))?;

    // 3. Verileri Hasat Et ve Modelle
    let rows = stmt
        .query_map(params![max_rows], |row| {
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
        })
        .map_err(|e| crate::MemosTradingError::Database(format!("Veri Okuma Hatası: {}", e)))?;

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
    let conn = crate::persistence::open_db(db_path)
        .map_err(|e| crate::MemosTradingError::Database(format!("DB bağlantı hatası: {}", e)))?;

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .map_err(|e| {
            crate::MemosTradingError::Database(format!("Sorgu hazırlama hatası: {}", e))
        })?;

    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| {
            crate::MemosTradingError::Database(format!("Tablo listesi okuma hatası: {}", e))
        })?;

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

/// `candles` tablosundaki bir kanonik seri referansı: (exchange, market, symbol, interval)
/// + bar sayısı. `list_series` döndürür; edge-tarama gibi "DB'de ne var" gezen araçlar için.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandleSeriesRef {
    pub exchange: String,
    pub market: String,
    pub symbol: String,
    pub interval: String,
    pub rows: usize,
}

/// 🗂️ `candles` tablosundaki TÜM benzersiz (exchange, market, symbol, interval) serilerini
/// bar sayısıyla listeler — DB'de fiilen ne olduğunu tek sorguyla çıkarır (edge-tarama,
/// envanter). `market_filter = Some("futures")` → yalnız o market. Bar sayısına göre AZALAN
/// sıralı (en zengin seri başta). Market-SAF (exchange+market kolonlu kanonik şema, Faz 0).
pub fn list_series(
    db_path: &str,
    market_filter: Option<&str>,
) -> Result<Vec<CandleSeriesRef>, crate::MemosTradingError> {
    let conn = crate::persistence::open_db(db_path)
        .map_err(|e| crate::MemosTradingError::Database(format!("DB bağlantı hatası: {}", e)))?;
    let (sql, has_filter) = match market_filter {
        Some(_) => (
            "SELECT exchange, market, symbol, interval, COUNT(*) AS n FROM candles \
             WHERE market = ?1 GROUP BY exchange, market, symbol, interval ORDER BY n DESC",
            true,
        ),
        None => (
            "SELECT exchange, market, symbol, interval, COUNT(*) AS n FROM candles \
             GROUP BY exchange, market, symbol, interval ORDER BY n DESC",
            false,
        ),
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| crate::MemosTradingError::Database(format!("list_series hazırlık: {}", e)))?;
    let map_row = |row: &rusqlite::Row| -> rusqlite::Result<CandleSeriesRef> {
        Ok(CandleSeriesRef {
            exchange: row.get(0)?,
            market: row.get(1)?,
            symbol: row.get(2)?,
            interval: row.get(3)?,
            rows: row.get::<_, i64>(4)?.max(0) as usize,
        })
    };
    let rows = if has_filter {
        stmt.query_map(rusqlite::params![market_filter.unwrap()], map_row)
    } else {
        stmt.query_map([], map_row)
    }
    .map_err(|e| crate::MemosTradingError::Database(format!("list_series sorgu: {}", e)))?;
    Ok(rows.flatten().collect())
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
    let conn = crate::persistence::open_db(db_path)
        .map_err(|e| crate::MemosTradingError::Database(format!("DB bağlantı hatası: {}", e)))?;

    // KESİN ÇÖZÜM: Match içinde closure çalıştırmıyoruz, sadece SQL şablonunu seçiyoruz.
    // Böylece closure tipi tek bir noktada doğar ve uyuşmazlık tamamen biter.
    let mut symbols = Vec::new();

    match (market, interval) {
        (Some(m), Some(i)) => {
            let sql = "SELECT DISTINCT symbol FROM candles WHERE market = ? AND interval = ? ORDER BY symbol";
            let mut stmt = conn.prepare(sql).map_err(|e| {
                crate::MemosTradingError::Database(format!("Sorgu hazırlama hatası: {}", e))
            })?;
            let rows = stmt
                .query_map(rusqlite::params![m, i], |row| row.get::<_, String>(0))
                .map_err(|e| {
                    crate::MemosTradingError::Database(format!(
                        "Sembol listesi okuma hatası: {}",
                        e
                    ))
                })?;
            for sym in rows.flatten() {
                symbols.push(sym);
            }
        }
        (Some(m), None) => {
            let sql = "SELECT DISTINCT symbol FROM candles WHERE market = ? ORDER BY symbol";
            let mut stmt = conn.prepare(sql).map_err(|e| {
                crate::MemosTradingError::Database(format!("Sorgu hazırlama hatası: {}", e))
            })?;
            let rows = stmt
                .query_map(rusqlite::params![m], |row| row.get::<_, String>(0))
                .map_err(|e| {
                    crate::MemosTradingError::Database(format!(
                        "Sembol listesi okuma hatası: {}",
                        e
                    ))
                })?;
            for sym in rows.flatten() {
                symbols.push(sym);
            }
        }
        (None, Some(i)) => {
            let sql = "SELECT DISTINCT symbol FROM candles WHERE interval = ? ORDER BY symbol";
            let mut stmt = conn.prepare(sql).map_err(|e| {
                crate::MemosTradingError::Database(format!("Sorgu hazırlama hatası: {}", e))
            })?;
            let rows = stmt
                .query_map(rusqlite::params![i], |row| row.get::<_, String>(0))
                .map_err(|e| {
                    crate::MemosTradingError::Database(format!(
                        "Sembol listesi okuma hatası: {}",
                        e
                    ))
                })?;
            for sym in rows.flatten() {
                symbols.push(sym);
            }
        }
        (None, None) => {
            let sql = "SELECT DISTINCT symbol FROM candles ORDER BY symbol";
            let mut stmt = conn.prepare(sql).map_err(|e| {
                crate::MemosTradingError::Database(format!("Sorgu hazırlama hatası: {}", e))
            })?;
            let rows = stmt
                .query_map(rusqlite::params![], |row| row.get::<_, String>(0))
                .map_err(|e| {
                    crate::MemosTradingError::Database(format!(
                        "Sembol listesi okuma hatası: {}",
                        e
                    ))
                })?;
            for sym in rows.flatten() {
                symbols.push(sym);
            }
        }
    };

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
    read_candles_filtered(db_path, symbol, interval, None, limit)
}

/// `read_candles`'ın market-farkında sürümü: yalnız verilen `market` (örn. "spot",
/// "futures") satırlarını okur. `candles` tablosunda spot+futures tek (symbol,interval)
/// serisine karıştığından (dedup index market içermez), temiz/saf seri gereken yerler
/// (interval-tarama, market-özel backtest) bunu kullanmalı. `read_candles` market'i
/// yok sayar (geriye-uyum). [[project_candle_schema_mismatch]]
pub fn read_candles_market(
    db_path: &str,
    symbol: &str,
    interval: &str,
    market: &str,
    limit: usize,
) -> Result<Vec<crate::core::types::Candle>, crate::MemosTradingError> {
    read_candles_filtered(db_path, symbol, interval, Some(market), limit)
}

/// 🗄️ NAMESPACE-farkında mum okuma: `(exchange, market)` çiftinin İKİSİNİ de filtreler.
///
/// NEDEN GEREKLİ: `read_candles_market` yalnız `market`'i süzer. Ama `mt5:cfd` ve
/// `ibkr:cfd` aynı market etiketini paylaşıyor (ikisi de `Market::Cfd`), dolayısıyla
/// çıplak sembolle (`AUDUSD`) market-only okuma İKİ BORSANIN serisini birbirine
/// karıştırır — aynı timestamp'te iki farklı fiyat satırı. Tablonun UNIQUE index'i
/// `(exchange, market, symbol, interval, timestamp)` olduğuna göre doğru okuma anahtarı
/// da bu çifttir. Canlı döngü `VenueAdapter::storage_namespace()` ile bunu besler.
pub fn read_candles_ns(
    db_path: &str,
    symbol: &str,
    interval: &str,
    exchange: &str,
    market: &str,
    limit: usize,
) -> Result<Vec<crate::core::types::Candle>, crate::MemosTradingError> {
    read_candles_ns_for_profile(
        db_path, symbol, interval, exchange, market, "default", limit,
    )
}

pub fn read_candles_ns_for_profile(
    db_path: &str,
    symbol: &str,
    interval: &str,
    exchange: &str,
    market: &str,
    profile: &str,
    limit: usize,
) -> Result<Vec<crate::core::types::Candle>, crate::MemosTradingError> {
    use chrono::{DateTime, TimeZone, Utc};
    use rusqlite::types::ValueRef;

    let conn = crate::persistence::open_db(db_path)
        .map_err(|e| crate::MemosTradingError::Database(format!("DB bağlantı hatası: {}", e)))?;

    let mut stmt = conn
        .prepare(
            "SELECT timestamp, open, high, low, close, volume, symbol, interval \
         FROM candles \
         WHERE symbol = ?1 AND interval = ?2 AND exchange = ?3 AND market = ?4 AND profile = ?5 \
         ORDER BY timestamp DESC LIMIT ?6",
        )
        .map_err(|e| {
            crate::MemosTradingError::Database(format!("candles(ns) sorgusu hazırlanamadı: {}", e))
        })?;

    let rows = stmt
        .query_map(
            params![symbol, interval, exchange, market, profile, limit as i64],
            |row: &rusqlite::Row| {
                let ts: DateTime<Utc> = match row.get_ref(0)? {
                    ValueRef::Integer(ms) => Utc
                        .timestamp_millis_opt(ms)
                        .single()
                        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap()),
                    ValueRef::Text(b) => {
                        let t = std::str::from_utf8(b).unwrap_or("");
                        DateTime::parse_from_rfc3339(t)
                            .map(|d| d.with_timezone(&Utc))
                            .unwrap_or_else(|_| Utc.timestamp_opt(0, 0).single().unwrap())
                    }
                    _ => Utc.timestamp_opt(0, 0).single().unwrap(),
                };
                Ok(crate::core::types::Candle {
                    timestamp: ts,
                    open: row.get(1)?,
                    high: row.get(2)?,
                    low: row.get(3)?,
                    close: row.get(4)?,
                    volume: row.get(5)?,
                    symbol: row.get(6)?,
                    interval: row.get(7)?,
                })
            },
        )
        .map_err(|e| crate::MemosTradingError::Database(format!("Mum hasat hatası (ns): {}", e)))?;

    let mut candles = Vec::with_capacity(limit.min(4096));
    for row in rows {
        match row {
            Ok(c) => candles.push(c),
            Err(e) => log::warn!("read_candles_ns: bozuk satır atlandı ({}): {}", symbol, e),
        }
    }
    candles.reverse(); // DESC → kronolojik
    Ok(candles)
}

/// `exchange+market` kanonik namespace için okuma denemesi. İlk önce kanonik alanı
/// dener; boş/eksik sonuçta yalnızca market filtresiyle geriye-uyum okuma yapılır.
/// Bu, aktif runtime'ı bozmaz; yalnız uyumluluk ve dedup açısından en güvenli akışı sağlar.
pub fn read_candles_for_venue(
    db_path: &str,
    symbol: &str,
    interval: &str,
    exchange: &str,
    market: &str,
    limit: usize,
) -> Result<Vec<crate::core::types::Candle>, crate::MemosTradingError> {
    read_candles_for_venue_profile(
        db_path, symbol, interval, exchange, market, "default", limit,
    )
}

pub fn read_candles_for_venue_profile(
    db_path: &str,
    symbol: &str,
    interval: &str,
    exchange: &str,
    market: &str,
    profile: &str,
    limit: usize,
) -> Result<Vec<crate::core::types::Candle>, crate::MemosTradingError> {
    let canonical =
        read_candles_ns_for_profile(db_path, symbol, interval, exchange, market, profile, limit);
    match canonical {
        Ok(candles) if !candles.is_empty() => {
            validate_venue_cadence(exchange, symbol, interval, &candles)?;
            Ok(candles)
        }
        _ if profile == "default" => {
            let candles = read_candles_market(db_path, symbol, interval, market, limit)?;
            validate_venue_cadence(exchange, symbol, interval, &candles)?;
            Ok(candles)
        }
        Ok(_) => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn validate_venue_cadence(
    exchange: &str,
    symbol: &str,
    interval: &str,
    candles: &[crate::core::types::Candle],
) -> Result<(), crate::MemosTradingError> {
    if !matches!(exchange, "ibkr" | "mt5") {
        return Ok(());
    }
    let expected = match interval {
        "1m" => 60_000,
        "5m" => 300_000,
        "15m" => 900_000,
        "30m" => 1_800_000,
        "1h" => 3_600_000,
        "4h" => 14_400_000,
        "1d" => 86_400_000,
        _ => return Ok(()),
    };
    let mut counts = std::collections::BTreeMap::<i64, usize>::new();
    for pair in candles.windows(2) {
        let delta = (pair[1].timestamp - pair[0].timestamp).num_milliseconds();
        if delta > 0 && delta < expected {
            return Err(crate::MemosTradingError::Database(format!(
                "Veri uygun değil: {exchange} {symbol} {interval} alt-interval delta={delta}ms, beklenen={expected}ms"
            )));
        }
        if delta > 0 {
            *counts.entry(delta).or_insert(0) += 1;
        }
    }
    let Some((observed, _)) = counts.into_iter().max_by_key(|(_, count)| *count) else {
        return Ok(());
    };
    if observed != expected {
        return Err(crate::MemosTradingError::Database(format!(
            "Veri uygun değil: {exchange} {symbol} {interval} cadence={observed}ms, beklenen={expected}ms"
        )));
    }
    Ok(())
}

fn read_candles_filtered(
    db_path: &str,
    symbol: &str,
    interval: &str,
    market: Option<&str>,
    limit: usize,
) -> Result<Vec<crate::core::types::Candle>, crate::MemosTradingError> {
    use chrono::{DateTime, TimeZone, Utc};
    use rusqlite::types::ValueRef;

    let conn = crate::persistence::open_db(db_path)
        .map_err(|e| crate::MemosTradingError::Database(format!("DB bağlantı hatası: {}", e)))?;

    // Market filtresi opsiyonel: None → eski davranış (market-agnostik).
    let query = match market {
        Some(_) => {
            "SELECT timestamp, open, high, low, close, volume, symbol, interval \
                    FROM candles \
                    WHERE symbol = ?1 AND interval = ?2 AND market = ?4 \
                    ORDER BY timestamp DESC LIMIT ?3"
        }
        None => {
            "SELECT timestamp, open, high, low, close, volume, symbol, interval \
                 FROM candles \
                 WHERE symbol = ?1 AND interval = ?2 \
                 ORDER BY timestamp DESC LIMIT ?3"
        }
    };

    let mut stmt = conn.prepare(query).map_err(|e| {
        crate::MemosTradingError::Database(format!("candles sorgusu hazırlanamadı: {}", e))
    })?;

    let map_row = |row: &rusqlite::Row| {
        // timestamp INTEGER (ms) ya da TEXT (RFC3339) olabilir → her ikisini de destekle.
        let ts: DateTime<Utc> = match row.get_ref(0)? {
            ValueRef::Integer(ms) => Utc
                .timestamp_millis_opt(ms)
                .single()
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
            open: row.get(1)?,
            high: row.get(2)?,
            low: row.get(3)?,
            close: row.get(4)?,
            volume: row.get(5)?,
            symbol: row.get(6)?,
            interval: row.get(7)?,
        })
    };

    let rows = match market {
        Some(m) => stmt.query_map(params![symbol, interval, limit as i64, m], map_row),
        None => stmt.query_map(params![symbol, interval, limit as i64], map_row),
    }
    .map_err(|e| crate::MemosTradingError::Database(format!("Mum hasat hatası: {}", e)))?;

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

/// Bir (symbol, interval, market) serisinin en yeni mum timestamp'i (epoch ms) — gap-farkında
/// download için (Faz 2). Kayıt yoksa None. MAX(timestamp) tek-satır, hızlı (idx_candles_pk).
pub fn last_candle_ts(db_path: &str, symbol: &str, interval: &str, market: &str) -> Option<i64> {
    last_candle_ts_for_profile(db_path, symbol, interval, market, "default")
}

pub fn last_candle_ts_for_profile(
    db_path: &str,
    symbol: &str,
    interval: &str,
    market: &str,
    profile: &str,
) -> Option<i64> {
    let conn = crate::persistence::open_db(db_path).ok()?;
    conn.query_row(
        "SELECT MAX(timestamp) FROM candles WHERE symbol=?1 AND interval=?2 AND market=?3 AND profile=?4",
        params![symbol, interval, market, profile],
        |row| row.get::<_, Option<i64>>(0),
    )
    .ok()
    .flatten()
}

/// 💰 funding geçmişini KRONOLOJİK (ASC) okur — son `limit` kayıt. Tablo yoksa boş (henüz indirilmemiş).
/// Market-saf (funding yalnız futures'ta anlamlı). funding_carry harness bunu hizalar.
pub fn read_funding_market(
    db_path: &str,
    symbol: &str,
    market: &str,
    limit: usize,
) -> Result<Vec<(i64, f64)>, crate::MemosTradingError> {
    let conn = crate::persistence::open_db(db_path)
        .map_err(|e| crate::MemosTradingError::Database(format!("DB bağlantı hatası: {}", e)))?;
    let mut stmt = match conn.prepare(
        "SELECT funding_time, rate FROM funding_rates WHERE symbol=?1 AND market=?2 \
         ORDER BY funding_time DESC LIMIT ?3",
    ) {
        Ok(s) => s,
        Err(_) => return Ok(Vec::new()), // tablo yok → boş
    };
    let rows = stmt
        .query_map(params![symbol, market, limit as i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
        })
        .map_err(|e| crate::MemosTradingError::Database(format!("funding okuma: {}", e)))?;
    let mut v: Vec<(i64, f64)> = rows.filter_map(|r| r.ok()).collect();
    v.reverse(); // DESC → ASC (kronolojik)
    Ok(v)
}

/// 💰 Borsa-FARKINDA funding okuma. `read_funding_market` yalnız (symbol,market) ile sorgular →
/// aynı sembolün çok-borsalı funding'i KARIŞIR; cross-exchange analiz için exchange'i de filtreler.
pub fn read_funding_exchange(
    db_path: &str,
    exchange: &str,
    symbol: &str,
    market: &str,
    limit: usize,
) -> Result<Vec<(i64, f64)>, crate::MemosTradingError> {
    let conn = crate::persistence::open_db(db_path)
        .map_err(|e| crate::MemosTradingError::Database(format!("DB bağlantı hatası: {}", e)))?;
    let mut stmt = match conn.prepare(
        "SELECT funding_time, rate FROM funding_rates WHERE exchange=?1 AND symbol=?2 AND market=?3 \
         ORDER BY funding_time DESC LIMIT ?4",
    ) {
        Ok(s) => s,
        Err(_) => return Ok(Vec::new()), // tablo yok → boş
    };
    let rows = stmt
        .query_map(params![exchange, symbol, market, limit as i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
        })
        .map_err(|e| crate::MemosTradingError::Database(format!("funding okuma: {}", e)))?;
    let mut v: Vec<(i64, f64)> = rows.filter_map(|r| r.ok()).collect();
    v.reverse(); // DESC → ASC (kronolojik)
    Ok(v)
}

/// 💰 Son funding_time (gap-farkında indirme için). Tablo/kayıt yoksa None.
pub fn last_funding_ts(db_path: &str, symbol: &str, market: &str) -> Option<i64> {
    let conn = crate::persistence::open_db(db_path).ok()?;
    conn.query_row(
        "SELECT MAX(funding_time) FROM funding_rates WHERE symbol=?1 AND market=?2",
        params![symbol, market],
        |row| row.get::<_, Option<i64>>(0),
    )
    .ok()
    .flatten()
}

/// 💰 Borsa-FARKINDA son funding_time (çok-borsalı gap-farkında indirme için).
pub fn last_funding_ts_exchange(
    db_path: &str,
    exchange: &str,
    symbol: &str,
    market: &str,
) -> Option<i64> {
    let conn = crate::persistence::open_db(db_path).ok()?;
    conn.query_row(
        "SELECT MAX(funding_time) FROM funding_rates WHERE exchange=?1 AND symbol=?2 AND market=?3",
        params![exchange, symbol, market],
        |row| row.get::<_, Option<i64>>(0),
    )
    .ok()
    .flatten()
}

/// 🗂️ Sembol-statü registry'sini DB'den okur (boot hydrate). Tablo henüz yoksa boş
/// döner (ilk çalıştırmada refresh job doldurana kadar). Dönen (symbol, status) listesi
/// `set_symbol_statuses` ile cache'e yüklenir.
pub fn load_symbol_statuses(db_path: &str) -> crate::Result<Vec<(String, String)>> {
    let conn = crate::persistence::open_db(db_path)
        .map_err(|e| crate::MemosTradingError::Database(format!("DB bağlantı hatası: {}", e)))?;
    let mut stmt = match conn.prepare("SELECT symbol, status FROM symbol_status") {
        Ok(s) => s,
        Err(_) => return Ok(Vec::new()), // tablo yok → boş
    };
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| crate::MemosTradingError::Database(format!("symbol_status sorgu: {}", e)))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}
