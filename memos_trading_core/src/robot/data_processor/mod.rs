// Srivastava ATP Mimarisi - Data Processor
//
// Ham veriyi temizlemek, normalize etmek ve validation yapmak için
// Data Processing katmanı. Teknik analiz için hazır hale getiriyor.

pub mod normalizer;
pub mod validator;
pub mod cleaner;

pub use normalizer::*;
pub use validator::*;
pub use cleaner::*;

use crate::types::Candle;
use crate::Result as MemosTradingResult;

/// Data Processor - Ana koordinatör
pub struct DataProcessor;

impl DataProcessor {
    /// Tam pipeline: Temizle → Normalize et → Valide et
    pub fn process_candles(mut candles: Vec<Candle>) -> MemosTradingResult<Vec<Candle>> {
        // 1. Temizle
        let cleaned = DataCleaner::clean(&mut candles)?;
        
        // 2. Normalize et
        let normalized = DataNormalizer::normalize(&cleaned)?;
        
        // 3. Valide et
        for candle in &normalized {
            DataValidator::validate_ohlc(candle)?;
        }
        
        Ok(normalized)
    }
    
    /// Sadece temizleme ve validation
    pub fn validate_and_clean(candles: &mut Vec<Candle>) -> MemosTradingResult<()> {
        DataCleaner::clean(candles)?;
        
        for candle in candles.iter() {
            DataValidator::validate_ohlc(candle)?;
        }
        
        Ok(())
    }
    
    /// Sadece normalizasyon
    pub fn normalize_only(candles: &[Candle]) -> MemosTradingResult<Vec<Candle>> {
        DataNormalizer::normalize(candles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    
    #[test]
    fn test_full_data_processing_pipeline() {
        let candles = vec![
            Candle {
                timestamp: Utc::now(),
                open: 100.0,
                high: 105.0,
                low: 95.0,
                close: 102.0,
                volume: 1000.0,
                symbol: "BTCUSDT".to_string(),
                interval: "1h".to_string(),
            }
        ];
        
        let result = DataProcessor::process_candles(candles);
        assert!(result.is_ok());
    }
}
