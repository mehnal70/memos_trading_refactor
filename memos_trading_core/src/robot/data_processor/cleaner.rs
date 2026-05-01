// Data Cleaner - Ham veriyi temizle ve düzelt
//
// Makale Gereksinimi: Data cleaning
// - Eksik veriyi doldur (forward/backward fill)
// - Duplikat satırları kaldır
// - Sıra dışı değerleri (outliers) kontrol et
// - Boş alanları işle

use crate::types::Candle;
use crate::Result as MemosTradingResult;

pub struct DataCleaner;

impl DataCleaner {
    /// Veriyi temizle (in-place)
    pub fn clean(candles: &mut Vec<Candle>) -> MemosTradingResult<Vec<Candle>> {
        // 1. Duplikatları kaldır
        Self::remove_duplicates(candles);
        
        // 2. Sıra dışı değerleri düzelt
        Self::handle_outliers(candles)?;
        
        // 3. Eksik veriyi doldur (forward fill)
        Self::fill_missing_data(candles)?;
        
        // 4. Sırt et
        candles.sort_by_key(|c| c.timestamp);
        
        Ok(candles.clone())
    }
    
    /// Duplikat mumları kaldır (aynı timestamp)
    fn remove_duplicates(candles: &mut Vec<Candle>) {
        let mut seen = std::collections::HashSet::new();
        candles.retain(|c| {
            // Timestamp + symbol kombinasyonunu unique key olarak kullan
            let key = (c.timestamp, c.symbol.clone());
            seen.insert(key)
        });
    }
    
    /// Sıra dışı fiyat hareketlerini düzelt
    fn handle_outliers(candles: &mut [Candle]) -> MemosTradingResult<()> {
        if candles.len() < 2 {
            return Ok(());
        }
        
        for i in 1..candles.len() {
            let prev_close = candles[i - 1].close;
            let curr = &mut candles[i];
            
            // Fiyat değişimi %50'den fazlaysa ve volume çok düşükse
            let price_change_pct = ((curr.close - prev_close) / prev_close).abs() * 100.0;
            
            if price_change_pct > 50.0 && curr.volume < 100.0 {
                // Muhtemelen hata, önceki close fiyatını kullan
                println!(
                    "⚠️  Outlier düzeltildi: {}: {:?} → {} ({:.2}% değişim)",
                    curr.symbol, curr.timestamp, prev_close, price_change_pct
                );
                
                curr.close = prev_close;
                curr.high = prev_close.max(curr.high);
                curr.low = prev_close.min(curr.low);
            }
        }
        
        Ok(())
    }
    
    /// Eksik veriyi doldur (forward fill + linear interpolation)
    fn fill_missing_data(candles: &mut Vec<Candle>) -> MemosTradingResult<()> {
        // Volume düzeltmesi tek mum için de gerekli — erken çıkış kaldırıldı.
        // Forward fill: bir önceki mümkün geçerli volume kullanılır.
        let mut last_valid_vol = 0.001f64;
        for candle in candles.iter_mut() {
            if candle.volume > 0.0 {
                last_valid_vol = candle.volume;
            } else {
                candle.volume = last_valid_vol;
                println!("📌 Zero volume düzeltildi: {}", candle.timestamp);
            }
        }

        Ok(())
    }
    
    /// Eksik mumları interpolasyon ile oluştur
    pub fn interpolate_missing_candles(
        candles: &mut Vec<Candle>,
        expected_interval_seconds: i64,
    ) -> MemosTradingResult<()> {
        if candles.len() < 2 {
            return Ok(());
        }
        
        let mut result = vec![];
        
        for i in 0..candles.len() - 1 {
            result.push(candles[i].clone());
            
            let curr = &candles[i];
            let next = &candles[i + 1];
            
            let time_diff = (next.timestamp - curr.timestamp).num_seconds();
            let expected_diff = expected_interval_seconds;
            
            // Eğer zaman farkı beklendikten fazlaysa, eksik mumlar var demektir
            if time_diff > expected_diff {
                let missing_count = (time_diff / expected_diff) as usize - 1;
                
                for j in 1..=missing_count {
                    let ratio = j as f64 / (missing_count + 1) as f64;
                    
                    // Linear interpolation
                    let interp_open = curr.open + (next.open - curr.open) * ratio;
                    let interp_close = curr.close + (next.close - curr.close) * ratio;
                    let interp_high = curr.high.max(next.high);
                    let interp_low = curr.low.min(next.low);
                    
                    let timestamp = curr.timestamp
                        + chrono::Duration::seconds(expected_diff * j as i64);
                    
                    result.push(Candle {
                        timestamp,
                        open: interp_open,
                        high: interp_high,
                        low: interp_low,
                        close: interp_close,
                        volume: 0.0, // Filler mumlar için 0 volume
                        symbol: curr.symbol.clone(),
                        interval: curr.interval.clone(),
                    });
                    
                    println!(
                        "📝 Eksik mum interpoled: {} ({})",
                        timestamp, curr.symbol
                    );
                }
            }
        }
        
        // Son mumı ekle
        result.push(candles[candles.len() - 1].clone());
        
        *candles = result;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    
    #[test]
    fn test_remove_duplicates() {
        let now = Utc::now();
        let mut candles = vec![
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
                timestamp: now, // Duplicate
                open: 100.0,
                high: 105.0,
                low: 95.0,
                close: 102.0,
                volume: 1000.0,
                symbol: "BTCUSDT".to_string(),
                interval: "1h".to_string(),
            }
        ];
        
        DataCleaner::remove_duplicates(&mut candles);
        assert_eq!(candles.len(), 1);
    }
    
    #[test]
    fn test_fill_zero_volume() {
        let now = Utc::now();
        let mut candles = vec![
            Candle {
                timestamp: now,
                open: 100.0,
                high: 105.0,
                low: 95.0,
                close: 102.0,
                volume: 0.0, // Zero volume
                symbol: "BTCUSDT".to_string(),
                interval: "1h".to_string(),
            }
        ];
        
        let cleaned = DataCleaner::clean(&mut candles).unwrap();
        assert!(cleaned[0].volume > 0.0);
    }
}
