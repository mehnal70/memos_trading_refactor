// robot/data_pipeline/normalizer.rs - Veri normalizasyonu

use crate::{types::Candle, Result};
use super::FetchParams;

/// Veri normalizasyon işlemleri
pub struct DataNormalizer;

impl DataNormalizer {
    pub fn new() -> Self {
        Self
    }
    
    /// Ana normalizasyon işlemi
    pub fn normalize(&self, mut candles: Vec<Candle>, params: &FetchParams) -> Result<Vec<Candle>> {
        // 1. Symbol standardı
        for candle in &mut candles {
            candle.symbol = Self::normalize_symbol(&params.symbol);
        }
        
        // 2. Interval standardı
        let interval_seconds = Self::parse_interval(&params.interval)?;
        for candle in &mut candles {
            // Interval saniye olarak ayarla (isimlendirme için)
            candle.interval = format!("{}s", interval_seconds);
        }
        
        // 3. Zaman dilimi standardı (UTC)
        // TODO: Implement timezone normalization
        
        // 4. Veri aralığı kontrolü
        if let (Some(_start), Some(_end)) = (&params.start_time, &params.end_time) {
            // TODO: Implement date range filtering
        }
        
        // 5. Sıralama (chronological)
        candles.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        
        // 6. Duplicate'i kaldır (aynı timestamp'e sahip olanlar)
        candles.dedup_by(|a, b| a.timestamp == b.timestamp);
        
        Ok(candles)
    }
    
    /// Symbol normalizasyon
    pub fn normalize_symbol(symbol: &str) -> String {
        symbol
            .to_uppercase()
            .replace("-", "")
            .replace("_", "")
            .replace("/", "")
    }
    
    /// Interval'i saniyeye çevir
    pub fn parse_interval(interval: &str) -> Result<u64> {
        let interval = interval.to_lowercase();
        
        match interval.as_str() {
            "1m" => Ok(60),
            "5m" => Ok(300),
            "15m" => Ok(900),
            "30m" => Ok(1800),
            "1h" => Ok(3600),
            "4h" => Ok(14400),
            "1d" | "24h" => Ok(86400),
            "1w" => Ok(604800),
            "1mo" => Ok(2592000),
            _ => {
                // Sayısal olarak saniye cinsinden mi?
                if let Ok(seconds) = interval.parse::<u64>() {
                    Ok(seconds)
                } else {
                    return Err(crate::MemosTradingError::Config(format!("Bilinmeyen interval: {}", interval)).into());
                }
            }
        }
    }
    
    /// Interval'i string'e çevir
    pub fn format_interval(seconds: u64) -> String {
        match seconds {
            60 => "1m".to_string(),
            300 => "5m".to_string(),
            900 => "15m".to_string(),
            1800 => "30m".to_string(),
            3600 => "1h".to_string(),
            14400 => "4h".to_string(),
            86400 => "1d".to_string(),
            604800 => "1w".to_string(),
            2592000 => "1mo".to_string(),
            _ => format!("{}s", seconds),
        }
    }
    
    /// Fiyatları normalize et (belirli ondalak basamağa)
    pub fn normalize_price(price: f64, decimals: usize) -> f64 {
        let multiplier = 10_f64.powi(decimals as i32);
        (price * multiplier).round() / multiplier
    }
    
    /// Hacmi normalize et
    pub fn normalize_volume(volume: f64) -> f64 {
        if volume < 0.0 {
            0.0
        } else {
            volume.round()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_normalize_symbol() {
        assert_eq!(DataNormalizer::normalize_symbol("btc-usd"), "BTCUSD");
        assert_eq!(DataNormalizer::normalize_symbol("BTC/USD"), "BTCUSD");
    }
    
    #[test]
    fn test_parse_interval() {
        assert_eq!(DataNormalizer::parse_interval("1h").unwrap(), 3600);
        assert_eq!(DataNormalizer::parse_interval("1d").unwrap(), 86400);
        assert_eq!(DataNormalizer::parse_interval("3600").unwrap(), 3600);
    }
    
    #[test]
    fn test_format_interval() {
        assert_eq!(DataNormalizer::format_interval(3600), "1h");
        assert_eq!(DataNormalizer::format_interval(86400), "1d");
        assert_eq!(DataNormalizer::format_interval(1234), "1234s");
    }
    
    #[test]
    fn test_normalize_price() {
        assert_eq!(DataNormalizer::normalize_price(100.123456, 2), 100.12);
        assert_eq!(DataNormalizer::normalize_price(100.5, 0), 101.0);
    }
}
