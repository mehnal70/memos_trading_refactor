// Data Normalizer - Farklı kaynaklardan gelen veriyi normalize et
//
// Makale Gereksinimi: Data standardization
// - Farklı exchange'lerden gelen veriler standardize formata döndür
// - Decimal precision kontrol et
// - Unit normalization (crypto: satoshi, etc)

use crate::types::Candle;
use crate::Result as MemosTradingResult;

pub struct DataNormalizer;

impl DataNormalizer {
    /// Mumları normalize et
    pub fn normalize(candles: &[Candle]) -> MemosTradingResult<Vec<Candle>> {
        let mut normalized = candles.to_vec();
        
        for candle in &mut normalized {
            // Fiyatları round et (6 decimal place)
            candle.open = Self::round_price(candle.open);
            candle.high = Self::round_price(candle.high);
            candle.low = Self::round_price(candle.low);
            candle.close = Self::round_price(candle.close);
            
            // Volume'ü round et (2 decimal place)
            candle.volume = Self::round_volume(candle.volume);
            
            // Symbol'ü uppercase yap
            candle.symbol = candle.symbol.to_uppercase();
            
            // Interval'i lowercase yap
            candle.interval = candle.interval.to_lowercase();
        }
        
        Ok(normalized)
    }
    
    /// Fiyatı 6 decimal place'e round et
    fn round_price(price: f64) -> f64 {
        (price * 1_000_000.0).round() / 1_000_000.0
    }
    
    /// Volume'ü 2 decimal place'e round et
    fn round_volume(volume: f64) -> f64 {
        (volume * 100.0).round() / 100.0
    }
    
    /// Satoshi'den Bitcoin'e dönüştür (crypto için)
    pub fn satoshi_to_btc(satoshi: f64) -> f64 {
        satoshi / 100_000_000.0
    }
    
    /// Bitcoin'den Satoshi'ye dönüştür
    pub fn btc_to_satoshi(btc: f64) -> f64 {
        btc * 100_000_000.0
    }
    
    /// Wei'den Ether'e dönüştür
    pub fn wei_to_eth(wei: f64) -> f64 {
        wei / 1_000_000_000_000_000_000.0
    }
    
    /// Ether'den Wei'ye dönüştür
    pub fn eth_to_wei(eth: f64) -> f64 {
        eth * 1_000_000_000_000_000_000.0
    }
    
    /// Merkez eksik veriler (NaN/Inf) olup olmadığını kontrol et
    pub fn has_invalid_data(candles: &[Candle]) -> bool {
        candles.iter().any(|c| {
            c.open.is_nan() || c.open.is_infinite()
                || c.high.is_nan() || c.high.is_infinite()
                || c.low.is_nan() || c.low.is_infinite()
                || c.close.is_nan() || c.close.is_infinite()
                || c.volume.is_nan() || c.volume.is_infinite()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    
    #[test]
    fn test_normalize_prices() {
        let candles = vec![
            Candle {
                timestamp: Utc::now(),
                open: 100.123456789,
                high: 105.987654321,
                low: 95.555555555,
                close: 102.123456789,
                volume: 1000.12345,
                symbol: "btcusdt".to_string(),
                interval: "1H".to_string(),
            }
        ];
        
        let normalized = DataNormalizer::normalize(&candles).unwrap();
        
        // Fiyatlar 6 decimal place'e rounded olmalı
        assert_eq!(normalized[0].open, 100.123457);
        assert_eq!(normalized[0].high, 105.987654);
        
        // Volume 2 decimal place'e rounded olmalı
        assert_eq!(normalized[0].volume, 1000.12);
        
        // Symbol uppercase olmalı
        assert_eq!(normalized[0].symbol, "BTCUSDT");
        
        // Interval lowercase olmalı
        assert_eq!(normalized[0].interval, "1h");
    }
    
    #[test]
    fn test_satoshi_conversion() {
        let btc = 1.0;
        let satoshi = DataNormalizer::btc_to_satoshi(btc);
        assert_eq!(satoshi, 100_000_000.0);
        
        let converted_back = DataNormalizer::satoshi_to_btc(satoshi);
        assert_eq!(converted_back, 1.0);
    }
    
    #[test]
    fn test_invalid_data_detection() {
        let candles = vec![
            Candle {
                timestamp: Utc::now(),
                open: f64::NAN,
                high: 105.0,
                low: 95.0,
                close: 102.0,
                volume: 1000.0,
                symbol: "BTCUSDT".to_string(),
                interval: "1h".to_string(),
            }
        ];
        
        assert!(DataNormalizer::has_invalid_data(&candles));
    }
}
