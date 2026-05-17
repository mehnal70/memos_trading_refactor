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

/// 📥 ADLİ MUM KAYDI: Chrono DateTime<Utc> uyumlu, SQL Injection korumalı mühürleyici.
pub fn save_candle(
    conn: &Connection,
    _exchange: &str,
    _market: &str,
    candle: &Candle,
) -> Result<()> {
    // Tablo adını dinamik yapılandır (Örn: candles_btcusdt_1m)
    let tbl_name = format!(
        "candles_{}_{}",
        candle.symbol.to_lowercase(),
        candle.interval.to_lowercase()
    );

    // SQL Injection engellemek için adli karakter kontrolü (Sadece harf, rakam ve alt tire)
    if !tbl_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(crate::MemosTradingError::Database(
            "🚨 Adli Güvenlik İhlali: Tablo isminde geçersiz karakter!".to_string(),
        ));
    }

    // Tablo yoksa otonom olarak şemayı ayağa kaldır
    let create_query = format!(
        "CREATE TABLE IF NOT EXISTS {} (
            open_time INTEGER PRIMARY KEY,
            open REAL NOT NULL,
            high REAL NOT NULL,
            low REAL NOT NULL,
            close REAL NOT NULL,
            volume REAL NOT NULL
        )",
        tbl_name
    );
    conn.execute(&create_query, [])
        .map_err(|e| crate::MemosTradingError::Database(format!("Tablo oluşturulamadı: {}", e)))?;

    // Veriyi mühürle (Chrono DateTime<Utc> nesnesi i64 milisaniyeye indirgeniyor)
    let insert_query = format!(
        "INSERT OR REPLACE INTO {} (open_time, open, high, low, close, volume) \
         VALUES (?, ?, ?, ?, ?, ?)",
        tbl_name
    );

    let raw_ms = candle.timestamp.timestamp_millis();

    conn.execute(
        &insert_query,
        params![
            raw_ms,
            candle.open,
            candle.high,
            candle.low,
            candle.close,
            candle.volume
        ],
    )
    .map_err(|e| crate::MemosTradingError::Database(format!("Mum yazma hatası: {}", e)))?;

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