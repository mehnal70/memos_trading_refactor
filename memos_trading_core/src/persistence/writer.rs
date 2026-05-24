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

/// 📥 ADLİ MUM KAYDI: Ana `candles` tablosuna timestamp=INTEGER (ms) formatında yazar.
/// Eski per-symbol `candles_{symbol}_{interval}` şeması terkedildi; tüm okuyucu/yazıcılar
/// ana tabloda hizalı. Repository init_schema şemasıyla birebir aynı kolonlar kullanılır;
/// `exchange`/`market` parametreleri şu an yalnız çağrı kaynağını izlemeye yarar ve tabloya
/// yazılmaz (legacy şemada bu kolonlar yok, repository ile uyumlu kalmak için).
///
/// Şema: candles(id, symbol, interval, timestamp, open, high, low, close, volume, created_at).
/// Aynı (symbol, interval, timestamp) varsa UPDATE; yoksa INSERT.
pub fn save_candle(
    conn: &Connection,
    _exchange: &str,
    _market: &str,
    candle: &Candle,
) -> Result<()> {
    // CandleRepository::new yoluyla gelmemiş bir bağlantı olabilir (örn. download job
    // doğrudan rusqlite::Connection::open ile açıyor). Defensive olarak tabloyu yarat.
    ensure_candles_table(conn)?;

    let raw_ms = candle.timestamp.timestamp_millis();

    // Önce UPDATE — varsa OHLCV revize edilir (kapanmamış mumun düzeltilmesi).
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

/// Ana `candles` tablosunu ve gerekli indeksleri defensive olarak yaratır.
/// Repository::init_schema ile aynı tanım — iki yer ayrışmamalı.
/// Boot zincirinden de çağrılır → ML retrain ve scheduler'lar cold-start'ta
/// "no such table: candles" hatasına çarpmasın.
pub fn ensure_candles_table(conn: &Connection) -> Result<()> {
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
    ).map_err(|e| crate::MemosTradingError::Database(format!("candles tablo init hatası: {}", e)))?;
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
        .unwrap_or_else(|| Utc::now());

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