// robot/data_pipeline/normalizer.rs - Birleştirilmiş Veri Normalizasyon Ünitesi
// 
// Görev: Ham veriyi temizler, spike'ları stabilize eder ve decimal hassasiyetini standartlaştırır.

use crate::core::types::Candle;

pub struct DataNormalizer;

impl DataNormalizer {
    /// Otonom Temizlik, Stabilizasyon ve Standartlaştırma:
    /// 1. Kronolojik sıralama ve mükerrer silme yapar.
    /// 2. %20+ fiyat spike'larını törpüler.
    /// 3. Decimal hassasiyeti (6 hane) ve NaN/Inf denetimi yapar.
    pub fn process_and_standardize(mut candles: Vec<Candle>) -> Vec<Candle> {
        if candles.is_empty() { return candles; }

        // 1. Sıralama ve Tekilleştirme
        candles.sort_by_key(|a| a.timestamp);
        candles.dedup_by(|a, b| a.timestamp == b.timestamp);

        let mut cleaned: Vec<crate::core::types::Candle> = Vec::with_capacity(candles.len());
        for i in 0..candles.len() {
            let mut current = candles[i].clone();

            // 2. Geçersiz Veri (NaN/Inf) Guard
            if Self::is_invalid_float(current.close) || current.close <= 0.0 {
                if i > 0 {
                    current.close = cleaned[i-1].close;
                    current.open = current.close;
                    current.high = current.close;
                    current.low = current.close;
                } else { continue; } // İlk mum bozuksa atla
            }

            // 3. Spike (Sıçrama) Kontrolü
            if i > 0 {
                let prev_close = cleaned[i-1].close;
                let change = (current.close - prev_close).abs() / prev_close;
                
                if change > 0.20 { // %20 sapma eşiği
                    let limit = if current.close > prev_close { 1.20 } else { 0.80 };
                    current.close = prev_close * limit;
                    current.high = current.high.max(current.close);
                    current.low = current.low.min(current.close);
                }
            }

            // 4. Standartlaştırma (Format & Hassasiyet)
            current.symbol = current.symbol.to_uppercase().replace(&['-', '_', '/', ':'][..], "");
            current.interval = current.interval.to_lowercase();
            
            current.open  = Self::round_price(current.open);
            current.high  = Self::round_price(current.high);
            current.low   = Self::round_price(current.low);
            current.close = Self::round_price(current.close);
            current.volume = Self::round_volume(current.volume);

            cleaned.push(current);
        }
        cleaned
    }

    /// Fiyatı 6 hane hassasiyete mühürler
    pub fn round_price(price: f64) -> f64 {
        (price * 1_000_000.0).round() / 1_000_000.0
    }

    /// Hacmi 2 hane hassasiyete mühürler
    pub fn round_volume(volume: f64) -> f64 {
        (volume * 100.0).round() / (100.0_f64).max(0.0)
    }

    /// Birim Dönüşümleri (Crypto specific)
    pub fn satoshi_to_btc(s: f64) -> f64 { s / 100_000_000.0 }
    pub fn wei_to_eth(w: f64) -> f64 { w / 1_000_000_000_000_000_000.0 }

    /// Interval'ı saniyeye otonom çevirir
    pub fn parse_interval(interval: &str) -> u64 {
        match interval.to_lowercase().as_str() {
            "1m" => 60, "5m" => 300, "15m" => 900, "30m" => 1800,
            "1h" => 3600, "4h" => 14400, "1d" => 86400,
            _ => interval.parse::<u64>().unwrap_or(60),
        }
    }

    #[inline]
    fn is_invalid_float(f: f64) -> bool { f.is_nan() || f.is_infinite() }
}
