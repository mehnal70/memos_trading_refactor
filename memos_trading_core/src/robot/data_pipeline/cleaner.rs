// robot/data_pipeline/cleaner.rs - Akıllı Veri Temizleme ve İnterpolasyon Ünitesi
//
// Görev: Mükerrerleri siler, outlier (anomali) fiyatları törpüler ve eksik zaman dilimlerini
// lineer interpolasyon yöntemiyle otonom olarak doldurur.

use crate::core::types::Candle;
use crate::Result;
use std::collections::HashSet;

pub struct DataCleaner;

impl DataCleaner {
    /// Otonom Temizlik Döngüsü: 
    /// Veriyi in-place temizler, sıralar ve volume/fiyat anomalilerini düzeltir.
    pub fn clean(candles: &mut Vec<Candle>) -> Result<()> {
        if candles.is_empty() { return Ok(()); }

        // 1. Sıralama ve Tekilleştirme
        candles.sort_by_key(|c| c.timestamp);
        let mut seen = HashSet::new();
        candles.retain(|c| seen.insert((c.timestamp, c.symbol.clone())));

        // 2. Outlier ve Hacim Düzeltme
        Self::handle_outliers_and_volume(candles);

        Ok(())
    }

    /// Fiyat ve Hacim Anomalilerini Düzeltir
    fn handle_outliers_and_volume(candles: &mut [Candle]) {
        let mut last_valid_vol = 0.001f64;
        
        for i in 0..candles.len() {
            // Hacim Forward Fill
            if candles[i].volume > 0.0 {
                last_valid_vol = candles[i].volume;
            } else {
                candles[i].volume = last_valid_vol;
            }

            // Fiyat Outlier Kontrolü (%50+ anlık sapma ve düşük hacim)
            if i > 0 {
                let prev_close = candles[i - 1].close;
                let current = &mut candles[i];
                if prev_close > 0.0 {
                    let change_pct = ((current.close - prev_close) / prev_close).abs() * 100.0;
                    if change_pct > 50.0 && current.volume < 100.0 {
                        current.close = prev_close;
                        current.high = current.high.max(prev_close);
                        current.low = current.low.min(prev_close);
                    }
                }
            }
        }
    }

    /// Eksik Mumları Tespit Eder ve Lineer İnterpolasyon ile Doldurur
    pub fn interpolate_missing_candles(
        candles: &mut Vec<Candle>,
        interval_secs: i64,
    ) -> Result<()> {
        if candles.len() < 2 { return Ok(()); }
        
        let mut interpolated_vec = Vec::with_capacity(candles.len());
        
        for i in 0..candles.len() - 1 {
            interpolated_vec.push(candles[i].clone());
            
            let curr = &candles[i];
            let next = &candles[i + 1];
            let time_diff = (next.timestamp - curr.timestamp).num_seconds();
            
            if time_diff > interval_secs {
                let missing_count = (time_diff / interval_secs) as usize - 1;
                
                for j in 1..=missing_count {
                    let ratio = j as f64 / (missing_count + 1) as f64;
                    
                    // Matematiksel İnterpolasyon: Fiyatı zamanla orantılı dağıt
                    let interp_close = curr.close + (next.close - curr.close) * ratio;
                    let timestamp = curr.timestamp + chrono::Duration::seconds(interval_secs * j as i64);
                    
                    interpolated_vec.push(Candle {
                        timestamp,
                        open: curr.close, // Geçiş mumu olduğu için önceki close baz alınır
                        high: curr.high.max(next.high),
                        low: curr.low.min(next.low),
                        close: interp_close,
                        volume: 0.001, // 0 yerine minimal hacim (indikatör çökmelerini engeller)
                        symbol: curr.symbol.clone(),
                        interval: curr.interval.clone(),
                    });
                }
            }
        }
        
        // Son mumu ekle
        if let Some(last) = candles.last() {
            interpolated_vec.push(last.clone());
        }
        
        *candles = interpolated_vec;
        Ok(())
    }
}
