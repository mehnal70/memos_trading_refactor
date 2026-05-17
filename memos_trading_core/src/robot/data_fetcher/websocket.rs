// robot/data_fetcher/websocket.rs - Gelişmiş WebSocket Kline İşleyici

use crate::core::types::Candle;
use crate::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Binance WebSocket Kline Güncelleme Yapısı
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinanceKlineUpdate {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: i64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "k")]
    pub kline: BinanceKline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinanceKline {
    #[serde(rename = "t")]
    pub open_time_ms: i64,
    #[serde(rename = "T")]
    pub close_time_ms: i64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "i")]
    pub interval: String,
    #[serde(rename = "o")]
    pub open: String,
    #[serde(rename = "h")]
    pub high: String,
    #[serde(rename = "l")]
    pub low: String,
    #[serde(rename = "c")]
    pub close: String,
    #[serde(rename = "v")]
    pub volume: String,
    #[serde(rename = "n")]
    pub trade_count: u64,
    #[serde(rename = "x")]
    pub is_closed: bool,
    #[serde(rename = "q")]
    pub quote_volume: String,
}

/// OHLCV Bütünlük Kontrolü (Otonom Veri Doğrulama Yasası)
pub fn validate_ohlcv(open: f64, high: f64, low: f64, close: f64, volume: f64) -> Result<()> {
    if open <= 0.0 || high <= 0.0 || low <= 0.0 || close <= 0.0 {
        return Err(format!("Hatalı Veri (Sıfır/Negatif): O={open} H={high} L={low} C={close}").into());
    }
    // Matematiksel Tutarlılık Denetimi: High her şeyden büyük, Low her şeyden küçük olmalı.
    if high < low || high < open || high < close || low > open || low > close {
        return Err(format!("Hatalı Veri (Mantıksız H/L): H={high} L={low} O={open} C={close}").into());
    }
    if volume < 0.0 {
        return Err(format!("Hatalı Veri (Negatif Hacim): {volume}").into());
    }
    Ok(())
}

/// WebSocket verisini otonom `Candle` tipine dönüştürür ve doğrular.
pub fn parse_kline(update: BinanceKlineUpdate) -> Result<Candle> {
    let k = update.kline;

    let parse = |s: &str, field: &str| -> Result<f64> {
        s.parse::<f64>().map_err(|_| format!("WS {field} parse hatası: '{s}'").into())
    };

    let (open, high, low, close, volume) = (
        parse(&k.open, "open")?,
        parse(&k.high, "high")?,
        parse(&k.low, "low")?,
        parse(&k.close, "close")?,
        parse(&k.volume, "volume")?
    );

    // Otonom Doğrulama
    validate_ohlcv(open, high, low, close, volume)?;

    let timestamp = DateTime::from_timestamp_millis(k.open_time_ms)
        .map(|dt| dt.with_timezone(&Utc))
        .ok_or_else(|| format!("Geçersiz Zaman Damgası: {}", k.open_time_ms))?;

    Ok(Candle {
        timestamp,
        open,
        high,
        low,
        close,
        volume,
        symbol: k.symbol,
        interval: k.interval,
    })
}
