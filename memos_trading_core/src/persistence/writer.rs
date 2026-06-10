// src/persistence/writer.rs - Srivastava ATP Adli Mühürleme Merkezi

use rusqlite::{params, Connection};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::fs;
use std::path::Path;
use serde::Serialize;
use crate::prelude::*; // Elit ve agnostik veri tipleri tek satırda bağlandı (Candle, PositionModel, MissionControl)
use crate::Result;
use chrono::{DateTime, Utc};

// =============================================================================
// 1. ASENKRON YAZICI (GELİŞTİRİLMİŞ WORKER FLIGHT-DECK)
// =============================================================================

pub enum DBWriteMsg {
    Candle { exchange: String, market: String, candle: Candle },
    /// Pozisyonları SQLite snapshot tablosuna mühürler
    PositionSnapshot(Vec<PositionModel>),
    /// Tüm zihni (MissionControl) JSON olarak diske mühürler
    DisasterRecovery(MissionControl, String),
}

pub struct DBWriter {
    sender: Sender<DBWriteMsg>,
}

impl DBWriter {
    pub fn new(conn: Connection) -> Self {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            Self::worker_loop(conn, rx);
        });
        Self { sender: tx }
    }

    pub fn write_candle(&self, exchange: &str, market: &str, candle: Candle) {
        let _ = self.sender.send(DBWriteMsg::Candle {
            exchange: exchange.to_owned(),
            market: market.to_owned(),
            candle,
        });
    }

    /// Pozisyonları asenkron kuyruğa gönderir
    pub fn snapshot_positions(&self, positions: Vec<PositionModel>) {
        let _ = self.sender.send(DBWriteMsg::PositionSnapshot(positions));
    }

    /// Tüm sistem yedeğini asenkron kuyruğa gönderir
    pub fn backup_system(&self, snap: MissionControl, path: String) {
        let _ = self.sender.send(DBWriteMsg::DisasterRecovery(snap, path));
    }

    fn worker_loop(conn: Connection, rx: Receiver<DBWriteMsg>) {
        for msg in rx {
            match msg {
                DBWriteMsg::Candle { exchange, market, candle } => {
                    if let Err(e) = save_candle(&conn, &exchange, &market, &candle) {
                        eprintln!("🚀 Srivastava Mum Hatası: {}", e);
                    }
                }
                DBWriteMsg::PositionSnapshot(positions) => {
                    if let Err(e) = save_open_positions_snapshot(&conn, &positions) {
                        eprintln!("🚀 Srivastava Pozisyon Snapshot Hatası: {}", e);
                    }
                }
                DBWriteMsg::DisasterRecovery(snap, path) => {
                    let _ = seal_config_to_disk(&path, &snap);
                }
            }
        }
    }
}

// =============================================================================
// 2. GENERIC JSON MÜHÜRLEYİCİ (ATOMİK DISASTER RECOVERY)
// =============================================================================

pub fn seal_config_to_disk<T: Serialize>(path: &str, data: &T) -> Result<()> {
    // Klasör emniyeti
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(data)
        .map_err(|e| crate::MemosTradingError::Unknown(format!("Serialization hatası: {}", e)))?;
    
    // Atomik Yazma: Önce geçici dosya sonra rename (Data corruption ve elektrik kesintisi koruması)
    let temp_path = format!("{}.tmp", path);
    fs::write(&temp_path, json)?;
    fs::rename(temp_path, path)?;
    Ok(())
}

// =============================================================================
// 3. POZİSYON RECOVERY VE EMİR MÜHÜRLERİ
// =============================================================================

pub fn save_open_positions_snapshot(conn: &Connection, positions: &[PositionModel]) -> Result<()> {
    // Tablonun varlığından emin ol (Hata yönetimli)
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS open_positions_snapshot (id INTEGER PRIMARY KEY CHECK (id = 1), positions TEXT NOT NULL, updated_at TEXT NOT NULL)",
        [],
    );

    let positions_json = serde_json::to_string(positions)
        .map_err(|e| crate::MemosTradingError::Unknown(format!("Snapshot serialization hatası: {}", e)))?;

    conn.execute(
        "INSERT INTO open_positions_snapshot (id, positions, updated_at)
         VALUES (1, ?1, ?2)
         ON CONFLICT(id) DO UPDATE SET positions=excluded.positions, updated_at=excluded.updated_at",
        params![positions_json, Utc::now().to_rfc3339()]
    )?;
    Ok(())
}

/// `account_state` tablosunu (singleton, id=1) idempotent yaratır.
pub fn ensure_account_state_table(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS account_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            equity REAL NOT NULL,
            peak_equity REAL NOT NULL,
            starting_capital REAL NOT NULL,
            closed_trades_count INTEGER NOT NULL,
            updated_at TEXT NOT NULL
        )",
        [],
    ).map_err(|e| crate::MemosTradingError::Database(format!("account_state tablo hatası: {}", e)))?;
    Ok(())
}

/// Account state'i (equity, peak, starting capital, closed_trades_count) tek-satır
/// singleton kayıt olarak DB'ye mühürler. Restart sonrası `load_account_state`
/// bunu okuyup FinanceVault'u hidrate eder → equity ve PnL geçmişi kaybolmaz.
pub fn save_account_state(
    conn: &Connection,
    equity: f64,
    peak_equity: f64,
    starting_capital: f64,
    closed_trades_count: usize,
) -> Result<()> {
    ensure_account_state_table(conn)?;
    conn.execute(
        "INSERT INTO account_state (id, equity, peak_equity, starting_capital, closed_trades_count, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET
            equity=excluded.equity,
            peak_equity=excluded.peak_equity,
            starting_capital=excluded.starting_capital,
            closed_trades_count=excluded.closed_trades_count,
            updated_at=excluded.updated_at",
        params![equity, peak_equity, starting_capital, closed_trades_count as i64, Utc::now().to_rfc3339()],
    ).map_err(|e| crate::MemosTradingError::Database(format!("account_state yazma hatası: {}", e)))?;
    Ok(())
}

/// 🗂️ Binance exchangeInfo sembol-statü registry'sini DB'ye persist eder (bulk upsert).
/// `symbol_status(symbol PK, status, updated_at)`. Restart'ta `load_symbol_statuses`
/// okuyup cache'i hidrate eder → ilk exchangeInfo fetch'ini beklemeden BREAK/delisted
/// semboller dışlanır. Tek tx ile ~2000 satır hızlı yazılır.
pub fn save_symbol_statuses(conn: &Connection, entries: &[(String, String)]) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS symbol_status (
            symbol TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );"
    ).map_err(|e| crate::MemosTradingError::Database(format!("symbol_status tablo: {}", e)))?;

    let now = Utc::now().to_rfc3339();
    let tx = conn.unchecked_transaction()
        .map_err(|e| crate::MemosTradingError::Database(format!("symbol_status tx: {}", e)))?;
    for (sym, status) in entries {
        tx.execute(
            "INSERT INTO symbol_status (symbol, status, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(symbol) DO UPDATE SET status=excluded.status, updated_at=excluded.updated_at",
            params![sym, status, now],
        ).map_err(|e| crate::MemosTradingError::Database(format!("symbol_status upsert: {}", e)))?;
    }
    tx.commit().map_err(|e| crate::MemosTradingError::Database(format!("symbol_status commit: {}", e)))?;
    Ok(())
}

/// 📥 ADLİ MUM KAYDI: Ana `candles` tablosuna timestamp=INTEGER (ms) formatında yazar.
/// Eski per-symbol `candles_{symbol}_{interval}` şeması terkedildi; tüm okuyucu/yazıcılar
/// ana tabloda hizalı. Repository init_schema şemasıyla birebir aynı kolonlar kullanılır;
/// `exchange`/`market` parametreleri şu an yalnız çağrı kaynağını izlemeye yarar ve tabloya
/// yazılmaz (legacy şemada bu kolonlar yok, repository ile uyumlu kalmak için).
///
/// Aynı (symbol, interval, timestamp) varsa UPDATE; yoksa INSERT.
///
/// ⚠️ ŞEMA-UYARLI: Üretim DB'si dış migrasyonla `exchange`/`market` NOT NULL + `created_at`
/// YOK şemasına geçmiş olabilir; kod-üretimi (fresh) tablo ise tersine `created_at` içerir,
/// `exchange`/`market` içermez. Eskiden INSERT koşulsuz `created_at`'e yazıyordu → üretim
/// tablosunda her yazım "no such column: created_at" ile sessizce patlıyordu (download
/// `let _ = save_candle` ile yutuyordu → veri donuyordu). Artık mevcut kolonlara göre uyumlu
/// yazım yapılır (4M satırlık tabloyu migrate etmeden iki şemada da çalışır).
pub fn save_candle(
    conn: &Connection,
    exchange: &str,
    market: &str,
    candle: &Candle,
) -> Result<()> {
    // CandleRepository::new yoluyla gelmemiş bir bağlantı olabilir (örn. download job
    // doğrudan rusqlite::Connection::open ile açıyor). Defensive olarak tabloyu yarat.
    ensure_candles_table(conn)?;

    let raw_ms = candle.timestamp.timestamp_millis();

    // Üretim şeması (exchange/market NOT NULL, created_at yok): tek-atımlık upsert.
    // exchange/market parametreleri burada gerçekten kullanılır (eskiden yok sayılıyordu).
    if column_exists(conn, "candles", "exchange")? {
        conn.execute(
            "INSERT INTO candles (exchange, market, symbol, interval, timestamp, open, high, low, close, volume) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
             ON CONFLICT(exchange, market, symbol, interval, timestamp) DO UPDATE SET \
               open=excluded.open, high=excluded.high, low=excluded.low, \
               close=excluded.close, volume=excluded.volume",
            params![
                exchange, market, &candle.symbol, &candle.interval, raw_ms,
                candle.open, candle.high, candle.low, candle.close, candle.volume,
            ],
        ).map_err(|e| crate::MemosTradingError::Database(format!("Mum upsert hatası: {}", e)))?;
        return Ok(());
    }

    // Fresh/kod-üretimi şema (created_at var, exchange/market yok): UPDATE→INSERT.
    let updated = conn.execute(
        "UPDATE candles SET open=?1, high=?2, low=?3, close=?4, volume=?5 \
         WHERE symbol=?6 AND interval=?7 AND timestamp=?8",
        params![
            candle.open, candle.high, candle.low, candle.close, candle.volume,
            &candle.symbol, &candle.interval, raw_ms,
        ],
    ).map_err(|e| crate::MemosTradingError::Database(format!("Mum update hatası: {}", e)))?;

    if updated == 0 {
        conn.execute(
            "INSERT INTO candles (symbol, interval, timestamp, open, high, low, close, volume, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                &candle.symbol, &candle.interval, raw_ms,
                candle.open, candle.high, candle.low, candle.close, candle.volume,
                Utc::now().to_rfc3339(),
            ],
        ).map_err(|e| crate::MemosTradingError::Database(format!("Mum insert hatası: {}", e)))?;
    }

    Ok(())
}

/// 💰 `funding_rates` tablosunu defensive yaratır — kanonik key (exchange,market,symbol,funding_time),
/// `candles` ile aynı market-saf felsefe. Funding 8 saatte bir → rate ondalık (ör. 0.0001 = %0.01).
pub fn ensure_funding_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS funding_rates ( \
            exchange TEXT NOT NULL, \
            market TEXT NOT NULL, \
            symbol TEXT NOT NULL, \
            funding_time INTEGER NOT NULL, \
            rate REAL NOT NULL, \
            PRIMARY KEY (exchange, market, symbol, funding_time) \
        );",
    ).map_err(|e| crate::MemosTradingError::Database(format!("funding tablo yaratma: {}", e)))?;
    Ok(())
}

/// 💰 Tek funding kaydını upsert eder (save_candle ile aynı kalıp; tekrar-güvenli).
pub fn save_funding(
    conn: &Connection, exchange: &str, market: &str, symbol: &str, funding_time_ms: i64, rate: f64,
) -> Result<()> {
    ensure_funding_table(conn)?;
    conn.execute(
        "INSERT INTO funding_rates (exchange, market, symbol, funding_time, rate) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(exchange, market, symbol, funding_time) DO UPDATE SET rate=excluded.rate",
        params![exchange, market, symbol, funding_time_ms, rate],
    ).map_err(|e| crate::MemosTradingError::Database(format!("funding upsert: {}", e)))?;
    Ok(())
}

/// Tabloda belirtilen kolon var mı — şema-uyarlı yazım için (PRAGMA table_info).
fn column_exists(conn: &Connection, table: &str, col: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))
        .map_err(|e| crate::MemosTradingError::Database(format!("table_info hazırlık: {}", e)))?;
    let found = stmt.query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| crate::MemosTradingError::Database(format!("table_info sorgu: {}", e)))?
        .filter_map(|r| r.ok())
        .any(|name| name == col);
    Ok(found)
}

/// Ana `candles` tablosunu ve gerekli indeksleri defensive olarak yaratır.
/// Repository::init_schema ile aynı tanım — iki yer ayrışmamalı.
/// Boot zincirinden de çağrılır → ML retrain ve scheduler'lar cold-start'ta
/// "no such table: candles" hatasına çarpmasın.
pub fn ensure_candles_table(conn: &Connection) -> Result<()> {
    // KANONİK ŞEMA (Faz 0): market kimliğin parçası → unique key
    // (exchange,market,symbol,interval,timestamp). Eskiden key market içermiyordu →
    // spot+futures aynı (symbol,interval,ts)'de çarpışıp tek seriye karışıyordu (basis
    // sıçraması). timestamp INTEGER (epoch ms). Mevcut DB'ler `migrate_candle_schema`
    // ile dar→geniş index'e taşınır (tablo zaten exchange/market kolonlu).
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS candles (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            exchange TEXT NOT NULL,
            market TEXT NOT NULL,
            symbol TEXT NOT NULL,
            interval TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            open REAL NOT NULL,
            high REAL NOT NULL,
            low REAL NOT NULL,
            close REAL NOT NULL,
            volume REAL NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_candles_pk ON candles(exchange, market, symbol, interval, timestamp);
        CREATE INDEX IF NOT EXISTS idx_candles_smi ON candles(symbol, interval, timestamp);
        CREATE INDEX IF NOT EXISTS idx_timestamp ON candles(timestamp);"
    ).map_err(|e| crate::MemosTradingError::Database(format!("candles tablo init hatası: {}", e)))?;
    Ok(())
}

/// Kanonik candle şema sürümü (PRAGMA user_version ile izlenir).
/// v2 = market-farkında geniş unique key (Faz 0). v3 = ölü/legacy tablo temizliği (Faz 4).
pub const CANDLE_SCHEMA_VERSION: i64 = 3;

/// 🧹 Faz 4: koddan ARTIK OKUNMAYAN/YAZILMAYAN legacy candle tabloları. Canlı kaynak
/// tek-nokta `candles` (kanonik, market-ayrık). Bunlar ya bayat snapshot ya da bozuk/
/// tekrarlı (ör. candles_binance_spot BTCUSDT: 2 günde 236k satır = dup çöp) ya da
/// per-symbol eski şema kalıntısı. Hiçbiri kodda referanslı değil (yalnız test yorumu).
/// DB-boyutunun ~%75'ini (~2GB) bunlar tutuyor → DROP + VACUUM ile geri kazanılır.
/// Tam DB yedeği arşiv politikasıdır (canlı DB'den düşürülür, yedek dosyada kalır).
const LEGACY_DEAD_TABLES: &[&str] = &[
    "candles_backup",
    "candles_binance_spot",
    "candles_binance_futures",
    "candles_binance_coinm",
    "candles_binance_spot_bak",
    "candles_binance_futures_bak",
    "candles_bist_bist100",
    "candles_bist_stocks",
    "candles_btcusdt_1m",
    "candles_ethusdt_1m",
];

/// Verilen tablonun DB'de var olup olmadığını döndürür (sqlite_master sorgusu).
fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        params![table], |r| r.get(0),
    ).map_err(|e| crate::MemosTradingError::Database(format!("sqlite_master sorgu: {}", e)))?;
    Ok(n > 0)
}

/// 🧹 Faz 4 adımı: [`LEGACY_DEAD_TABLES`] içindeki ölü tabloları (varsa) düşürür.
/// Düşürülen tablo sayısını döndürür. VACUUM'u çağıran üstlenir (transaction dışı koşmalı).
fn prune_legacy_tables(conn: &Connection) -> Result<usize> {
    let mut dropped = 0usize;
    for &t in LEGACY_DEAD_TABLES {
        if table_exists(conn, t)? {
            // İsim sabit-liste (kullanıcı girdisi değil) → format! güvenli.
            conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{t}\";"))
                .map_err(|e| crate::MemosTradingError::Database(
                    format!("ölü tablo düşürme ({t}) hatası: {e}")))?;
            dropped += 1;
        }
    }
    Ok(dropped)
}

/// 🔧 Faz 0 migration: mevcut `candles` tablosunu dar unique key
/// `(symbol,interval,timestamp)` → kanonik geniş key
/// `(exchange,market,symbol,interval,timestamp)`'e taşır. TABLO YENİDEN YAZILMAZ —
/// yalnız index'ler değişir (kolonlar zaten yerinde): dar unique'ler düşürülür, geniş
/// unique + (symbol,interval,ts) yardımcı index kurulur. Eski dar key zaten tekil
/// olduğundan geniş key de kesin tekil → UNIQUE oluşturma garantili başarılı (kayıpsız).
///
/// Idempotent: `user_version >= CANDLE_SCHEMA_VERSION` ise no-op. `exchange` kolonu
/// yoksa (kod-üretimi taze şema zaten kanonik) yalnız sürüm damgalanır.
pub fn migrate_candle_schema(conn: &Connection) -> Result<()> {
    let uv: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap_or(0);
    if uv >= CANDLE_SCHEMA_VERSION { return Ok(()); }

    // Tablo yoksa kanonik yarat (taze DB). Varsa ve exchange kolonluysa index swap.
    ensure_candles_table(conn)?;

    // ── v2: market-farkında geniş unique key (Faz 0) — yalnız henüz taşınmamışsa. ──
    if uv < 2 && column_exists(conn, "candles", "exchange")? {
        conn.execute_batch(
            "DROP INDEX IF EXISTS idx_candles_dedup;
             DROP INDEX IF EXISTS idx_symbol_interval_timestamp;
             CREATE UNIQUE INDEX IF NOT EXISTS idx_candles_pk ON candles(exchange, market, symbol, interval, timestamp);
             CREATE INDEX IF NOT EXISTS idx_candles_smi ON candles(symbol, interval, timestamp);
             CREATE INDEX IF NOT EXISTS idx_timestamp ON candles(timestamp);"
        ).map_err(|e| crate::MemosTradingError::Database(
            format!("candle şema migration (index swap) hatası: {}", e)))?;
        log::info!("🔧 candle şema migration: market-farkında unique key kuruldu (v2)");
    }

    // ── v3: ölü/legacy candle tablolarını düşür + alanı geri kazan (Faz 4). ──
    let pruned = if uv < 3 { prune_legacy_tables(conn)? } else { 0 };

    // Sürüm damgası VACUUM'dan ÖNCE: DROP'lar zaten commit'li; VACUUM yarıda kesilse
    // bile tablolar düşürülmüş kalır (correctness korunur), yalnız alan geri kazanılmaz
    // → sonraki boot uv>=3 görüp tekrar denemez (idempotent, gereksiz iş yok).
    conn.pragma_update(None, "user_version", CANDLE_SCHEMA_VERSION)
        .map_err(|e| crate::MemosTradingError::Database(format!("user_version damga hatası: {}", e)))?;

    // VACUUM transaction-dışı koşmalı (rusqlite execute_batch tek-statement → açık tx yok).
    // Yalnız gerçekten tablo düşürdüysek çalıştır (taze DB'de boşa I/O yok). Best-effort:
    // başarısızlık migration'ı bozmaz (tablolar zaten gitti, alan sonra elle geri alınabilir).
    if pruned > 0 {
        match conn.execute_batch("VACUUM;") {
            Ok(_)  => log::info!("🧹 Faz 4: {pruned} ölü tablo düşürüldü + VACUUM tamam (v3, alan geri kazanıldı)"),
            Err(e) => log::warn!("🧹 Faz 4: {pruned} ölü tablo düşürüldü; VACUUM atlandı ({e}) — alan elle geri alınabilir"),
        }
    }
    Ok(())
}

// =============================================================================
// 4. ADLİ HAREKÂT İHRACATI (REPORTERS)
// =============================================================================

/// Adli Harekât İhracatı (Kritik Raporlama Birimi)
pub fn build_export_report(snap: &MissionControl) -> String {
    let mut r = String::new();
    r.push_str("--- SRIVASTAVA ATP OPERASYONEL ANALİZ ---\n");
    r.push_str(&format!("Harekât Zamanı: {}\n", Utc::now()));
    r.push_str(&format!("Net PnL: {:.2} USDT\n", snap.finance.net_pnl()));
    r.push_str("----------------------------------------\n");
    for p in &snap.positions {
        r.push_str(&format!("Sembol: {} | Giriş: {:.4} | ROE: {:.1}%\n", p.symbol, p.entry_price, p.roe()));
    }
    r
}


/// 📊 ADLİ TERCÜME: Binance WebSocket veya REST API'den gelen ham kline (mum) dizisini
/// robotun Chrono DateTime<Utc> ve symbol/interval kalkanı taşıyan yeni Candle yapısına dönüştürür.
pub fn parse_binance_kline(
    raw_kline: &[serde_json::Value],
    symbol: &str,
    interval: &str,
) -> Option<crate::core::types::Candle> {
    // Binance kline formatı en az 6 temel parametre barındırmak zorundadır (OpenTime, O, H, L, C, V)
    if raw_kline.len() < 6 { return None; }

    // 1. Ham milisaniye zaman damgasını oku ve Chrono nesnesine matrisle
    let raw_ms = raw_kline[0].as_i64()?;
    let dt = chrono::DateTime::<Utc>::from_timestamp_millis(raw_ms)
        .unwrap_or_else(Utc::now);

    // 2. String tabanlı finansal verileri f64 rasyolarına güvenle dönüştür (Fail-Safe)
    let open: f64  = raw_kline[1].as_str()?.parse().ok()?;
    let high: f64  = raw_kline[2].as_str()?.parse().ok()?;
    let low: f64   = raw_kline[3].as_str()?.parse().ok()?;
    let close: f64 = raw_kline[4].as_str()?.parse().ok()?;
    let volume: f64 = raw_kline[5].as_str()?.parse().ok()?;

    // 3. Yeni nesil kimlikli Candle yapısını inşa et ve teslim et
    Some(crate::core::types::Candle {
        timestamp: dt,
        open,
        high,
        low,
        close,
        volume,
        symbol: symbol.to_string(),
        interval: interval.to_string(),
    })
}
#[cfg(test)]
mod funding_tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn funding_upsert_roundtrip_and_dedup() {
        let conn = Connection::open_in_memory().unwrap();
        // Yeni kayıtlar.
        save_funding(&conn, "binance", "futures", "BTCUSDT", 1_700_000_000_000, 0.0001).unwrap();
        save_funding(&conn, "binance", "futures", "BTCUSDT", 1_700_028_800_000, -0.00005).unwrap();
        // Aynı funding_time → upsert (kopya değil, rate güncellenir).
        save_funding(&conn, "binance", "futures", "BTCUSDT", 1_700_000_000_000, 0.0002).unwrap();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM funding_rates WHERE symbol='BTCUSDT'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2, "upsert kopya üretmez");
        let r: f64 = conn.query_row(
            "SELECT rate FROM funding_rates WHERE funding_time=1700000000000", [], |r| r.get(0)).unwrap();
        assert!((r - 0.0002).abs() < 1e-12, "rate güncellendi");
    }
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    use rusqlite::Connection;

    // Eski (dar key) üretim şemasını birebir kurar: exchange/market kolonlu ama
    // unique index market içermeyen → spot+futures çarpışır.
    fn setup_old_schema() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE candles (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                exchange TEXT NOT NULL, market TEXT NOT NULL,
                symbol TEXT NOT NULL, interval TEXT NOT NULL, timestamp INTEGER NOT NULL,
                open REAL NOT NULL, high REAL NOT NULL, low REAL NOT NULL,
                close REAL NOT NULL, volume REAL NOT NULL
            );
            CREATE UNIQUE INDEX idx_candles_dedup ON candles(symbol, interval, timestamp);
            CREATE UNIQUE INDEX idx_symbol_interval_timestamp ON candles(symbol, interval, timestamp);"
        ).unwrap();
        conn
    }

    fn ins(conn: &Connection, market: &str, ts: i64, close: f64) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO candles (exchange, market, symbol, interval, timestamp, open, high, low, close, volume) \
             VALUES ('binance', ?1, 'BTCUSDT', '1h', ?2, ?3, ?3, ?3, ?3, 1.0) \
             ON CONFLICT(exchange, market, symbol, interval, timestamp) DO UPDATE SET close=excluded.close",
            params![market, ts, close],
        )
    }

    #[test]
    fn migration_enables_cross_market_coexistence() {
        let conn = setup_old_schema();
        // Migrasyon ÖNCESİ: dar unique → aynı ts'de spot yazılır, futures'ı eklemek için
        // önce migrasyon gerekir (yoksa 5-kolon ON CONFLICT eşleşen index bulamaz).
        super::migrate_candle_schema(&conn).unwrap();

        // Migrasyon SONRASI: geniş unique key → spot ve futures AYNI (symbol,interval,ts)'de
        // birlikte yaşar (basis sıçraması = artık ayrık seriler).
        ins(&conn, "spot", 1000, 100.0).unwrap();
        ins(&conn, "futures", 1000, 101.0).unwrap();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM candles WHERE symbol='BTCUSDT' AND interval='1h' AND timestamp=1000",
            [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2, "spot+futures aynı ts'de ayrı satır olmalı (market key'de)");

        // Market-saf okuma doğru fiyatı verir.
        let spot_close: f64 = conn.query_row(
            "SELECT close FROM candles WHERE market='spot' AND timestamp=1000", [], |r| r.get(0)).unwrap();
        let fut_close: f64 = conn.query_row(
            "SELECT close FROM candles WHERE market='futures' AND timestamp=1000", [], |r| r.get(0)).unwrap();
        assert_eq!(spot_close, 100.0);
        assert_eq!(fut_close, 101.0);

        // user_version damgalandı.
        let uv: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(uv, CANDLE_SCHEMA_VERSION);
    }

    #[test]
    fn migration_is_idempotent() {
        let conn = setup_old_schema();
        super::migrate_candle_schema(&conn).unwrap();
        // İkinci çağrı no-op (uv>=2) — panik/hata yok.
        super::migrate_candle_schema(&conn).unwrap();
        let uv: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(uv, CANDLE_SCHEMA_VERSION);
    }

    #[test]
    fn migration_preserves_existing_rows() {
        let conn = setup_old_schema();
        // Migrasyon ÖNCESİ düz INSERT (geniş-key ON CONFLICT index'i henüz yok).
        let ins_plain = |ts: i64, close: f64| conn.execute(
            "INSERT INTO candles (exchange, market, symbol, interval, timestamp, open, high, low, close, volume) \
             VALUES ('binance','spot','BTCUSDT','1h', ?1, ?2, ?2, ?2, ?2, 1.0)",
            params![ts, close]).unwrap();
        ins_plain(2000, 50.0);
        ins_plain(3000, 51.0);
        super::migrate_candle_schema(&conn).unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM candles", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2, "mevcut satırlar korunmalı (index swap kayıpsız)");
    }

    #[test]
    fn v3_prunes_dead_tables_and_keeps_canonical() {
        let conn = setup_old_schema();
        // Kanonik `candles`'a bir satır + birkaç ölü legacy tablo (veri dolu) kur.
        conn.execute(
            "INSERT INTO candles (exchange, market, symbol, interval, timestamp, open, high, low, close, volume) \
             VALUES ('binance','spot','BTCUSDT','1h', 1000, 1.0,1.0,1.0,1.0,1.0)", []).unwrap();
        conn.execute_batch(
            "CREATE TABLE candles_backup (x INTEGER); INSERT INTO candles_backup VALUES (1);
             CREATE TABLE candles_binance_spot (x INTEGER); INSERT INTO candles_binance_spot VALUES (1);
             CREATE TABLE candles_btcusdt_1m (x INTEGER); INSERT INTO candles_btcusdt_1m VALUES (1);
             CREATE TABLE keep_me (x INTEGER); INSERT INTO keep_me VALUES (1);"
        ).unwrap();

        super::migrate_candle_schema(&conn).unwrap();

        // Ölü tablolar gitti.
        for t in ["candles_backup", "candles_binance_spot", "candles_btcusdt_1m"] {
            assert!(!super::table_exists(&conn, t).unwrap(), "{t} düşürülmeliydi");
        }
        // Kanonik veri + listede olmayan tablo korundu.
        assert!(super::table_exists(&conn, "keep_me").unwrap(), "listede olmayan tablo korunmalı");
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM candles", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "kanonik candles satırı korunmalı");
        let uv: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(uv, 3, "v3 damgalanmalı");
    }

    #[test]
    fn v3_prune_idempotent_no_dead_tables() {
        // uv=2 (zaten market-migrated) DB'de ölü tablo yok → prune no-op, uv=3 olur, hata yok.
        let conn = setup_old_schema();
        conn.pragma_update(None, "user_version", 2i64).unwrap();
        super::migrate_candle_schema(&conn).unwrap();
        let uv: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(uv, 3);
        // İkinci çağrı tam no-op.
        super::migrate_candle_schema(&conn).unwrap();
    }
}
