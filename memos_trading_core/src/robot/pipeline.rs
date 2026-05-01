// robot/pipeline.rs - Standart Data Processing Pipeline (modüler, interface tabanlı)

use crate::robot::interfaces::DataFetcher;
use crate::types::Candle;
use crate::Result;

pub struct PipelineStepResult<T> {
    pub data: T,
    pub step: &'static str,
}

pub struct DataPipelineModular<'a> {
    pub fetcher: &'a dyn DataFetcher,
    // Gerekirse: validator, normalizer, cacher interface'leri de eklenebilir
}

impl<'a> DataPipelineModular<'a> {
    pub fn new(fetcher: &'a dyn DataFetcher) -> Self {
        Self { fetcher }
    }

    /// Standart pipeline: fetch -> validate -> normalize -> cache -> use
    pub fn run(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>> {
        // 1. Fetch
        let candles = self.fetcher.fetch(symbol, interval, limit)?;
        // 2. Validate (örnek: boş veri kontrolü)
        if candles.is_empty() {
            return Err(crate::MemosTradingError::Config("No data fetched".to_string()));
        }
        // 3. Normalize (örnek: fiyatları yuvarla)
        let normalized: Vec<Candle> = candles
            .into_iter()
            .map(|mut c| { c.close = (c.close * 100.0).round() / 100.0; c })
            .collect();
        // 4. Cache (şimdilik dummy, gerçek cache interface ile genişletilebilir)
        // 5. Use
        Ok(normalized)
    }
}
