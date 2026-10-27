use crate::error::Result;
use crate::types::Candle;

/// Venue transportu kullanılamadığında tarihsel veriyi sağlayan core-dışı port.
pub trait CandleFallback: Send + Sync {
    fn read_candles(
        &self,
        symbol: &str,
        interval: &str,
        exchange: &str,
        market: &str,
        limit: usize,
    ) -> Result<Vec<Candle>>;

    fn read_candles_for_profile(
        &self,
        symbol: &str,
        interval: &str,
        exchange: &str,
        market: &str,
        profile: &str,
        limit: usize,
    ) -> Result<Vec<Candle>> {
        let _ = profile;
        self.read_candles(symbol, interval, exchange, market, limit)
    }
}
