// robot/data_pipeline/mod.rs - Merkezi veri işlem pipeline

pub mod sources;
pub mod normalizer;
pub mod cache;

pub use sources::DataSource;
pub use normalizer::DataNormalizer;
pub use cache::DataCache;

use crate::{types::Candle, Result};

/// Veri getirme parametreleri
#[derive(Debug, Clone)]
pub struct FetchParams {
    pub symbol: String,
    pub interval: String,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub limit: Option<usize>,
}

/// Birleşik veri pipeline
pub struct DataPipeline {
    /// Veri kaynağı
    source: Box<dyn DataSource>,
    /// Normalizasyon
    normalizer: DataNormalizer,
    /// Caching
    cache: DataCache,
}

impl DataPipeline {
    pub fn new(source: Box<dyn DataSource>) -> Self {
        Self {
            source,
            normalizer: DataNormalizer::new(),
            cache: DataCache::new(),
        }
    }
    
    /// Ana pipeline işlemi:
    /// 1. Cache'e bak
    /// 2. Fetch (kaynaktan)
    /// 3. Validate
    /// 4. Normalize
    /// 5. Enrich
    /// 6. Cache'e kaydet
    pub async fn process(
        &self,
        params: FetchParams,
    ) -> Result<Vec<Candle>> {
        // 1. Cache'i kontrol et
        if let Some(cached) = self.cache.get(&params.symbol) {
            return Ok(cached);
        }
        
        // 2. Kaynaktan getir
        let mut candles = self.source.fetch(&params).await?;
        
        // 3. Validasyon
        self.validate(&candles)?;
        
        // 4. Normalizasyon
        candles = self.normalizer.normalize(candles, &params)?;
        
        // 5. Zenginleştirme (metadata ekle)
        for candle in &mut candles {
            candle.symbol = params.symbol.clone();
        }
        
        // 6. Cache'e kaydet
        self.cache.set(&params.symbol, candles.clone());
        
        Ok(candles)
    }
    
    fn validate(&self, candles: &[Candle]) -> Result<()> {
        for candle in candles {
            if candle.close <= 0.0 || candle.open <= 0.0 {
                return Err(crate::MemosTradingError::Config("Geçersiz fiyat verisi".to_string()).into());
            }
            if candle.volume < 0.0 {
                return Err(crate::MemosTradingError::Config("Hacim negatif olamaz".to_string()).into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_fetch_params() {
        let params = FetchParams {
            symbol: "AKBNK".to_string(),
            interval: "1h".to_string(),
            start_time: None,
            end_time: None,
            limit: Some(100),
        };
        
        assert_eq!(params.symbol, "AKBNK");
    }
}
