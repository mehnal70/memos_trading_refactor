// robot/data_pipeline/validator.rs - Veri Kalite Denetim Ünitesi
//
// Görev: OHLC ilişkilerini, fiyatların mantıksallığını, zaman sürekliliğini
// ve sembol tutarlılığını otonom doğrular.

use crate::core::types::Candle;
use crate::Result;
use log::{warn, info, error}; 

pub struct DataValidator;

impl DataValidator {
    /// OHLC mum verisinin fiziksel geçerliliğini kontrol eder.
    pub fn validate_ohlc(candle: &Candle) -> Result<()> {
        // 1. Mantıksal Hiyerarşi (High en büyük, Low en küçük olmalı)
        if candle.high < candle.open || candle.high < candle.close || candle.high < candle.low {
            return Err(format!("Geçersiz OHLC ({}): High ({:.4}) seviyesi Open/Close/Low'dan küçük.", candle.symbol, candle.high).into());
        }
        
        if candle.low > candle.open || candle.low > candle.close || candle.low > candle.high {
            return Err(format!("Geçersiz OHLC ({}): Low ({:.4}) seviyesi Open/Close/High'dan büyük.", candle.symbol, candle.low).into());
        }
        
        // 2. Negatif Değer Guard
        if candle.open < 0.0 || candle.high < 0.0 || candle.low < 0.0 || candle.close < 0.0 {
            return Err("Geçersiz OHLC: Negatif fiyatlara izin verilmez.".into());
        }
        
        // 3. Hacim Denetimi
        if candle.volume < 0.0 {
            return Err("Geçersiz OHLC: Hacim negatif olamaz.".into());
        }

        // 4. Intra-bar Spike (Bar içi sıçrama) Tespiti
        // Fiyat range'i ortalama fiyatın %50'sini aşıyorsa uyarır (Hata döndürmez, loglar).
        let mid = (candle.high + candle.low) / 2.0;
        if mid > 0.0 {
            let range_pct = ((candle.high - candle.low) / mid) * 100.0;
            if range_pct > 50.0 {
                warn!("⚠ Anormal bar içi hareket ({}): {:.2}%", candle.symbol, range_pct);
            }
        }
        
        Ok(())
    }
    
    /// Mum dizisinin kronolojik ve yapısal tutarlılığını doğrular.
    pub fn validate_sequence(candles: &[Candle]) -> Result<()> {
        if candles.is_empty() {
            return Err("Doğrulanacak mum verisi yok (Boş dizi).".into());
        }
        
        let first = &candles[0];
        let base_symbol = &first.symbol;
        let base_interval = &first.interval;

        for i in 0..candles.len() {
            let curr = &candles[i];

            // 1. OHLC Yapısı
            Self::validate_ohlc(curr)?;

            // 2. Sembol ve Interval Tutarlılığı (Karışık veri girişi engeli)
            if curr.symbol != *base_symbol || curr.interval != *base_interval {
                return Err(format!("Tutarsız veri seti: {}/{} içinde {}/{} tespit edildi.", 
                    base_symbol, base_interval, curr.symbol, curr.interval).into());
            }

            // 3. Zaman Sürekliliği (Sıralama kontrolü)
            if i > 0 {
                let prev = &candles[i - 1];
                if curr.timestamp <= prev.timestamp {
                    return Err(format!("Zaman akışı bozuk: {} mumu {} mumundan sonra geliyor.", 
                        curr.timestamp, prev.timestamp).into());
                }
            }
        }
        
        Ok(())
    }
}
