// Data Validator - OHLC verilerinin geçerliliğini kontrol et
//
// Makale Gereksinimi: Data quality checks
// - Tutarlı OHLC ilişkisi
// - Anormal fiyat hareketleri (spike detection)
// - Volume validation
// - Timestamp süreklilik

use crate::types::Candle;
use crate::Result as MemosTradingResult;

pub struct DataValidator;

impl DataValidator {
    /// OHLC mum verisinin geçerliliğini kontrol et
    pub fn validate_ohlc(candle: &Candle) -> MemosTradingResult<()> {
        // 1. High >= Open, Close, Low
        if candle.high < candle.open || candle.high < candle.close || candle.high < candle.low {
            return Err("Invalid OHLC: High must be >= Open, Close, Low".into());
        }
        
        // 2. Low <= Open, Close, High
        if candle.low > candle.open || candle.low > candle.close || candle.low > candle.high {
            return Err("Invalid OHLC: Low must be <= Open, Close, High".into());
        }
        
        // 3. Negatif fiyat yok
        if candle.open < 0.0 || candle.high < 0.0 || candle.low < 0.0 || candle.close < 0.0 {
            return Err("Invalid OHLC: Negative prices not allowed".into());
        }
        
        // 4. Volume >= 0
        if candle.volume < 0.0 {
            return Err("Invalid OHLC: Volume cannot be negative".into());
        }
        
        // 5. Aşırı büyük fiyat hareketini tespit et (spike detection)
        if candle.high > 0.0 && candle.low > 0.0 {
            let price_range = candle.high - candle.low;
            let mid_price = (candle.high + candle.low) / 2.0;
            let spike_pct = (price_range / mid_price) * 100.0;
            
            // Eğer mum içindeki range %50'den fazlaysa uyar
            if spike_pct > 50.0 {
                // Bu uyarı log edilebilir ama hata değil
                println!("⚠️  Anormal fiyat hareketi tespit: {:.2}%", spike_pct);
            }
        }
        
        Ok(())
    }
    
    /// Ardışık mumlar arasında zaman sürekliliğini kontrol et
    pub fn validate_timestamp_continuity(candles: &[Candle]) -> MemosTradingResult<()> {
        if candles.len() < 2 {
            return Ok(());
        }
        
        for i in 1..candles.len() {
            let prev = &candles[i - 1];
            let curr = &candles[i];
            
            if curr.timestamp <= prev.timestamp {
                return Err("Invalid: Timestamps are not in ascending order".into());
            }
        }
        
        Ok(())
    }
    
    /// Bütün mumlar aynı sembol ve interval'de mi?
    pub fn validate_consistency(candles: &[Candle]) -> MemosTradingResult<()> {
        if candles.is_empty() {
            return Ok(());
        }
        
        let first = &candles[0];
        let symbol = &first.symbol;
        let interval = &first.interval;
        
        for candle in candles {
            if candle.symbol != *symbol {
                return Err("Invalid: Mixed symbols in candles".into());
            }
            if candle.interval != *interval {
                return Err("Invalid: Mixed intervals in candles".into());
            }
        }
        
        Ok(())
    }
    
    /// Bütün kontrolleri yapan comprehensive validator
    pub fn validate_comprehensive(candles: &[Candle]) -> MemosTradingResult<()> {
        // Boş veri
        if candles.is_empty() {
            return Err("No candles to validate".into());
        }
        
        // Süreklilik kontrol et
        Self::validate_consistency(candles)?;
        
        // Timestamp sürekliliği
        Self::validate_timestamp_continuity(candles)?;
        
        // Her bir mumun geçerliliği
        for candle in candles {
            Self::validate_ohlc(candle)?;
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    
    #[test]
    fn test_valid_candle() {
        let candle = Candle {
            timestamp: Utc::now(),
            open: 100.0,
            high: 105.0,
            low: 95.0,
            close: 102.0,
            volume: 1000.0,
            symbol: "BTCUSDT".to_string(),
            interval: "1h".to_string(),
        };
        
        assert!(DataValidator::validate_ohlc(&candle).is_ok());
    }
    
    #[test]
    fn test_invalid_high() {
        let candle = Candle {
            timestamp: Utc::now(),
            open: 100.0,
            high: 90.0, // High < Low - INVALID
            low: 95.0,
            close: 102.0,
            volume: 1000.0,
            symbol: "BTCUSDT".to_string(),
            interval: "1h".to_string(),
        };
        
        assert!(DataValidator::validate_ohlc(&candle).is_err());
    }
    
    #[test]
    fn test_negative_price() {
        let candle = Candle {
            timestamp: Utc::now(),
            open: -100.0, // Negative - INVALID
            high: 105.0,
            low: 95.0,
            close: 102.0,
            volume: 1000.0,
            symbol: "BTCUSDT".to_string(),
            interval: "1h".to_string(),
        };
        
        assert!(DataValidator::validate_ohlc(&candle).is_err());
    }
    
    #[test]
    fn test_timestamp_continuity() {
        let now = Utc::now();
        let candles = vec![
            Candle {
                timestamp: now,
                open: 100.0,
                high: 105.0,
                low: 95.0,
                close: 102.0,
                volume: 1000.0,
                symbol: "BTCUSDT".to_string(),
                interval: "1h".to_string(),
            },
            Candle {
                timestamp: now - chrono::Duration::hours(1), // INVALID: goes backward
                open: 102.0,
                high: 107.0,
                low: 97.0,
                close: 104.0,
                volume: 1000.0,
                symbol: "BTCUSDT".to_string(),
                interval: "1h".to_string(),
            },
        ];
        
        assert!(DataValidator::validate_timestamp_continuity(&candles).is_err());
    }
}
