use crate::types::Candle;
use crate::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinanceKlineUpdate {
    #[serde(rename = "e")]
    pub event_type: String,

    #[serde(rename = "k")]
    pub kline: BinanceKline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinanceKline {
    /// Kline açılış zamanı (ms) — Binance WS "t" alanı
    #[serde(rename = "t")]
    pub open_time_ms: i64,

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

    #[serde(rename = "x")]
    pub is_closed: bool,
}

/// OHLCV bütünlük kontrolü: geçersiz değerlerde hata döner.
/// Sıfır veya negatif fiyat, yanlış H/L ilişkisi, negatif hacim reddedilir.
/// `live_adapter.rs` ve bu dosya tarafından paylaşılır.
pub fn validate_ohlcv(open: f64, high: f64, low: f64, close: f64, volume: f64) -> Result<()> {
    if open <= 0.0 || high <= 0.0 || low <= 0.0 || close <= 0.0 {
        return Err(format!("Geçersiz OHLCV: sıfır/negatif fiyat O={open} H={high} L={low} C={close}").into());
    }
    if high < low {
        return Err(format!("Geçersiz OHLCV: high({high}) < low({low})").into());
    }
    if high < open || high < close {
        return Err(format!("Geçersiz OHLCV: high({high}) < open({open}) veya close({close})").into());
    }
    if low > open || low > close {
        return Err(format!("Geçersiz OHLCV: low({low}) > open({open}) veya close({close})").into());
    }
    if volume < 0.0 {
        return Err(format!("Geçersiz OHLCV: negatif hacim {volume}").into());
    }
    Ok(())
}

pub fn parse_kline(update: BinanceKlineUpdate) -> Result<Candle> {
    let k = update.kline;

    // String → f64: parse hatası açıkça bildirilir
    let open:   f64 = k.open.parse().map_err(|_| format!("WS open parse hatası: '{}'", k.open))?;
    let high:   f64 = k.high.parse().map_err(|_| format!("WS high parse hatası: '{}'", k.high))?;
    let low:    f64 = k.low.parse().map_err(|_| format!("WS low parse hatası: '{}'", k.low))?;
    let close:  f64 = k.close.parse().map_err(|_| format!("WS close parse hatası: '{}'", k.close))?;
    let volume: f64 = k.volume.parse().map_err(|_| format!("WS volume parse hatası: '{}'", k.volume))?;

    validate_ohlcv(open, high, low, close, volume)?;

    // Gerçek candle açılış zamanı — daha önce Utc::now() kullanılıyordu (YANLIŞ).
    // Binance WS "t" alanı kline başlangıç zamanını ms olarak verir.
    let timestamp = chrono::DateTime::from_timestamp(k.open_time_ms / 1000, 0)
        .map(|dt| dt.with_timezone(&Utc))
        .ok_or_else(|| format!("WS geçersiz timestamp: {}", k.open_time_ms))?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kline_parsing() {
        // "t": kline açılış zamanı (ms) — Binance WS zorunlu alanı
        let json = r#"{
            "e": "kline",
            "k": {
                "t": 1700000000000,
                "s": "BTCUSDT",
                "i": "1m",
                "o": "45000.0",
                "h": "46000.0",
                "l": "44000.0",
                "c": "45500.0",
                "v": "100.5",
                "x": true
            }
        }"#;

        let update: BinanceKlineUpdate = serde_json::from_str(json).unwrap();
        let candle = parse_kline(update).unwrap();

        assert_eq!(candle.open, 45000.0);
        assert_eq!(candle.symbol, "BTCUSDT");
        // Timestamp Utc::now() değil, "t" alanından gelmeli
        assert_eq!(candle.timestamp.timestamp(), 1700000000000_i64 / 1000);
    }
}
