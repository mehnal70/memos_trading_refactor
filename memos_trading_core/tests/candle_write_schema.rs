// save_candle şema-uyumu — regresyon testi (network'süz).
//
// Bug: üretim DB'si dış migrasyonla `candles(exchange/market NOT NULL, created_at YOK)`
// şemasına geçmişti; save_candle ise koşulsuz `created_at`'e INSERT ediyordu →
// "no such column: created_at" ile her yazım sessizce patlıyor, download "✓" dese de
// veri günlerce donuyordu. Mevcut download_job testi FRESH şema (created_at) kullandığı
// için bug'ı kaçırıyordu. Bu test her iki şemayı da kapsar.

use memos_trading_core::core::types::Candle;
use memos_trading_core::persistence::reader::read_candles;
use memos_trading_core::persistence::writer::{ensure_candles_table, save_candle};

fn mk_candle(symbol: &str, ts_ms: i64, close: f64) -> Candle {
    Candle {
        timestamp: chrono::DateTime::from_timestamp_millis(ts_ms).unwrap(),
        open: close, high: close + 1.0, low: close - 1.0, close, volume: 10.0,
        symbol: symbol.to_string(),
        interval: "1m".to_string(),
    }
}

fn tmp_db(tag: &str) -> String {
    format!("/tmp/memos_candle_schema_{}_{}.db", tag, std::process::id())
}

#[test]
fn save_candle_persists_on_production_schema_exchange_market() {
    let db = tmp_db("prod");
    let _ = std::fs::remove_file(&db);
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        // ÜRETİM şeması: exchange/market NOT NULL, timestamp INTEGER, created_at YOK.
        conn.execute_batch(
            "CREATE TABLE candles (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                exchange TEXT NOT NULL, market TEXT NOT NULL,
                symbol TEXT NOT NULL, interval TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                open REAL NOT NULL, high REAL NOT NULL, low REAL NOT NULL,
                close REAL NOT NULL, volume REAL NOT NULL);
             CREATE UNIQUE INDEX idx_candles_dedup ON candles(symbol, interval, timestamp);",
        ).unwrap();

        let c = mk_candle("BTCUSDT", 1_700_000_000_000, 50_000.0);
        // Bug öncesi: 'no such column: created_at' ile patlardı.
        save_candle(&conn, "binance", "spot", &c)
            .expect("üretim şemasına (exchange/market) yazım başarısız");

        // Upsert idempotent: aynı (symbol,interval,timestamp) → ON CONFLICT UPDATE.
        let mut c2 = c.clone();
        c2.close = 51_000.0;
        save_candle(&conn, "binance", "spot", &c2)
            .expect("üretim şemasında upsert güncelleme başarısız");
    }

    let candles = read_candles(&db, "BTCUSDT", "1m", 10).expect("okuma başarısız");
    assert_eq!(candles.len(), 1, "dedup: tek mum olmalı (upsert)");
    assert!((candles[0].close - 51_000.0).abs() < 1e-9, "upsert close güncellemeli");
    let _ = std::fs::remove_file(&db);
}

#[test]
fn save_candle_persists_on_fresh_schema_created_at() {
    let db = tmp_db("fresh");
    let _ = std::fs::remove_file(&db);
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        // Kod-üretimi (fresh) şema: ensure_candles_table created_at'li tablo yaratır.
        ensure_candles_table(&conn).unwrap();
        let c = mk_candle("ETHUSDT", 1_700_000_060_000, 3_000.0);
        save_candle(&conn, "binance", "spot", &c)
            .expect("fresh şemaya (created_at) yazım başarısız");
    }
    let candles = read_candles(&db, "ETHUSDT", "1m", 10).expect("okuma başarısız");
    assert_eq!(candles.len(), 1, "fresh şemada mum yazılmalı");
    assert!((candles[0].close - 3_000.0).abs() < 1e-9);
    let _ = std::fs::remove_file(&db);
}
