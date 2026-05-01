// sim_data.rs
// Basit simülasyon veri sağlayıcı modülü
// Gerçek zamanlıya yakın test için örnek Candle verisi üretir

use chrono::{Utc, Duration};
use crate::types::Candle;

pub fn generate_sample_candles(symbol: &str, interval: &str, count: usize) -> Vec<Candle> {
    let mut candles = Vec::with_capacity(count);
    let now = Utc::now();
    for i in 0..count {
        let ts = now - Duration::minutes((count - i) as i64);
        let base = 100.0 + (i as f64 * 0.1);
        candles.push(Candle {
            timestamp: ts,
            open: base,
            high: base + 1.0,
            low: base - 1.0,
            close: base + ((i % 3) as f64 - 1.0),
            volume: 10.0 + (i as f64),
            symbol: symbol.to_string(),
            interval: interval.to_string(),
        });
    }
    candles
}
