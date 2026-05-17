// combined_strategy.rs - Optimize Edilmiş MA + RSI Hibrit Stratejisi

use crate::core::types::{Candle, Signal};
use crate::Result;

pub struct CombinedStrategy;

impl CombinedStrategy {
    /// MA Crossover Sinyali - Performans: Sıfır kopyalama, sadece son pencere hesabı.
    pub fn ma_crossover(candles: &[Candle], fast_p: usize, slow_p: usize) -> Result<Signal> {
        let n = candles.len();
        if n < slow_p + 1 { return Ok(Signal::Hold); }

        // Slicing: Tüm listeyi hesaplamak yerine sadece son iki durumu hesapla
        let fast_curr = Self::get_single_sma(&candles[n - fast_p..n], fast_p);
        let fast_prev = Self::get_single_sma(&candles[n - fast_p - 1..n - 1], fast_p);
        let slow_curr = Self::get_single_sma(&candles[n - slow_p..n], slow_p);
        let slow_prev = Self::get_single_sma(&candles[n - slow_p - 1..n - 1], slow_p);

        let signal = match (fast_prev <= slow_prev, fast_curr > slow_curr) {
            (true, true) => Signal::Buy,   // Golden Cross
            _ if fast_prev >= slow_prev && fast_curr < slow_curr => Signal::Sell, // Death Cross
            _ => Signal::Hold,
        };

        Ok(signal)
    }

    /// RSI Sinyali - Wilder's Smoothing mantığıyla optimize edildi.
    pub fn rsi_signal(candles: &[Candle], period: usize, threshold: f64) -> Result<Signal> {
        if candles.len() < period + 1 { return Ok(Signal::Hold); }

        let rsi_val = Self::get_last_rsi(candles, period);

        let signal = match rsi_val {
            r if r < threshold => Signal::Buy,          // Oversold
            r if r > (100.0 - threshold) => Signal::Sell, // Overbought
            _ => Signal::Hold,
        };

        Ok(signal)
    }

    /// Combined Strateji: MA Trend + RSI Momentum Filtresi
    pub fn combined_signal(
        candles: &[Candle],
        ma_fast: usize,
        ma_slow: usize,
        rsi_period: usize,
        rsi_threshold: f64,
    ) -> Result<Signal> {
        let ma = Self::ma_crossover(candles, ma_fast, ma_slow)?;
        let rsi = Self::rsi_signal(candles, rsi_period, rsi_threshold)?;

        // Karar Matrisi: Sinyallerin birbirini teyit etmesi (Confluence)
        let final_signal = match (ma, rsi) {
            (Signal::Buy, Signal::Buy) => Signal::Buy,
            (Signal::Sell, Signal::Sell) => Signal::Sell,
            // RSI aşırı alım/satım bölgesindeyken gelen MA kırılımlarını önceliklendir
            (Signal::Buy, Signal::Hold) | (Signal::Hold, Signal::Buy) => Signal::Buy,
            (Signal::Sell, Signal::Hold) | (Signal::Hold, Signal::Sell) => Signal::Sell,
            _ => Signal::Hold,
        };

        Ok(final_signal)
    }

    // --- ÖZEL PERFORMANS YARDIMCILARI (INTERNAL) ---

    /// Tek bir SMA değeri hesaplar (O(p) hızında)
    fn get_single_sma(slice: &[Candle], period: usize) -> f64 {
        slice.iter().map(|c| c.close).sum::<f64>() / period as f64
    }

    /// En son RSI değerini kopyalama yapmadan hesaplar
    fn get_last_rsi(candles: &[Candle], period: usize) -> f64 {
        let start = candles.len() - period - 1;
        let mut gains = 0.0;
        let mut losses = 0.0;

        for w in candles[start..].windows(2) {
            let diff = w[1].close - w[0].close;
            if diff > 0.0 { gains += diff; } else { losses += diff.abs(); }
        }

        if losses == 0.0 { return 100.0; }
        100.0 - (100.0 / (1.0 + (gains / losses)))
    }
}
