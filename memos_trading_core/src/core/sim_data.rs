// sim_data.rs
// Simülasyon Veri Sağlayıcı Modülü - Performans Optimize Edilmiş

use chrono::{Utc, Duration};
use crate::core::types::Candle;

/// Gerçek zamanlıya yakın test için örnek mum verisi üretir.
/// Modernize: allocation maliyetini düşürmek için string sahipliği optimize edildi.
pub fn generate_sample_candles(symbol: &str, interval: &str, count: usize) -> Vec<Candle> {
    let now = Utc::now();
    
    // Iteratör ve Collect kullanarak daha fonksiyonel ve hızlı bir inşa süreci
    (0..count)
        .map(|i| {
            // Zaman damgasını (timestamp) i64 olarak milisaniye cinsinden hesapla
            let ts = now - Duration::minutes((count - i) as i64);
            let base = 100.0 + (i as f64 * 0.1);
            
            Candle {
                timestamp: ts,
                open: base,
                high: base + 1.0,
                low: base - 1.0,
                close: base + ((i % 3) as f64 - 1.0),
                volume: 10.0 + (i as f64),
                symbol: symbol.to_owned(),   // to_string() yerine to_owned() tercih edildi
                interval: interval.to_owned(),
            }
        })
        .collect() // Vec::with_capacity() otomatik olarak içeride yönetilir
}

/// Pipeline testleri için anlık (real-time) tek bir mum simüle eder
pub fn generate_realtime_tick(symbol: &str, interval: &str, last_price: f64) -> Candle {
    Candle {
        timestamp: Utc::now(),
        open: last_price,
        high: last_price + 0.5,
        low: last_price - 0.5,
        close: last_price + 0.1,
        volume: 1.5,
        symbol: symbol.to_owned(),
        interval: interval.to_owned(),
    }
}
