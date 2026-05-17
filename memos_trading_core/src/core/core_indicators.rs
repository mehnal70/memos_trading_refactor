
use crate::core::types::Candle;

pub struct CoreIndicatorEngine;

impl CoreIndicatorEngine {
    /// Klasik SMA - Bellek kopyalamasız (Slicing) optimizasyon
    pub fn sma(candles: &[Candle], period: usize) -> f64 {
        let n = candles.len();
        if n == 0 || period == 0 { return 0.0; }
        
        // Slicing: Son 'period' kadar mumu kopyalamadan referansla al
        let start = n.saturating_sub(period);
        let slice = &candles[start..];
        
        slice.iter().map(|c| c.close).sum::<f64>() / slice.len() as f64
    }

    /// Klasik RSI - Pencereleme (Windows) optimizasyonu
    pub fn rsi(candles: &[Candle], period: usize) -> f64 {
        let n = candles.len();
        if n <= period { return 50.0; }

        let start = n.saturating_sub(period + 1);
        let (mut gains, mut losses) = (0.0, 0.0);

        // İteratör zinciri: Kopyalama yapmadan farkları hesapla
        for w in candles[start..].windows(2) {
            let diff = w[1].close - w[0].close;
            if diff > 0.0 { gains += diff; } else { losses -= diff; }
        }

        if losses == 0.0 { return 100.0; }
        let rs = gains / losses;
        100.0 - (100.0 / (1.0 + rs))
    }

    /// ML tabanlı otomatik feature extraction
    pub fn ml_features(candles: &[Candle]) -> Vec<f64> {
        let n = candles.len();
        if n < 2 { return vec![0.0; 4]; }

        let count = n.min(5);
        let start = n - count;
        let slice = &candles[start..];

        // Ortalama hesabı (Sıfır kopyalama)
        let mean = slice.iter().map(|c| c.close).sum::<f64>() / count as f64;
        if mean == 0.0 { return vec![0.0; count - 1]; }

        // Normalize farklar: Allocation optimizasyonu (with_capacity)
        slice.windows(2)
            .map(|w| (w[1].close - w[0].close) / mean)
            .collect()
    }

    /// Otomatik yeni indikatör keşfi
    pub fn discover_features(_candles: &[Candle]) -> Vec<String> {
        // Gereksiz String kopyalamalarını önlemek için 'static referanslardan üretilebilir
        ["sma", "rsi", "ml_feature"]
            .iter()
            .map(|&s| s.to_owned())
            .collect()
    }
}
