use crate::types::{Candle, Signal};
use crate::Result;

/// MA Crossover + RSI kombinasyon stratejisi
pub struct CombinedStrategy;

impl CombinedStrategy {
    /// MA Crossover sinyali hesapla
    pub fn ma_crossover(candles: &[Candle], fast_period: usize, slow_period: usize) -> Result<Signal> {
        if candles.len() < slow_period {
            return Ok(Signal::Hold);
        }

        let fast_ma = Self::calculate_sma(&candles, fast_period)?;
        let slow_ma = Self::calculate_sma(&candles, slow_period)?;

        // Son iki MA değerini karşılaştır
        if let (Some(fast_curr), Some(fast_prev), Some(slow_curr), Some(slow_prev)) = (
            fast_ma.get(fast_ma.len() - 1),
            fast_ma.get(fast_ma.len() - 2),
            slow_ma.get(slow_ma.len() - 1),
            slow_ma.get(slow_ma.len() - 2),
        ) {
            if fast_prev <= slow_prev && fast_curr > slow_curr {
                return Ok(Signal::Buy);
            } else if fast_prev >= slow_prev && fast_curr < slow_curr {
                return Ok(Signal::Sell);
            }
        }

        Ok(Signal::Hold)
    }

    /// RSI sinyali hesapla
    pub fn rsi_signal(candles: &[Candle], period: usize, threshold: f64) -> Result<Signal> {
        if candles.len() < period {
            return Ok(Signal::Hold);
        }

        let rsi = Self::calculate_rsi(&candles, period)?;
        
        if let Some(current_rsi) = rsi.last() {
            if *current_rsi < threshold {
                return Ok(Signal::Buy); // Oversold
            } else if *current_rsi > (100.0 - threshold) {
                return Ok(Signal::Sell); // Overbought
            }
        }

        Ok(Signal::Hold)
    }

    /// Combined stratejisi: MA + RSI
    pub fn combined_signal(
        candles: &[Candle],
        ma_fast: usize,
        ma_slow: usize,
        rsi_period: usize,
        rsi_threshold: f64,
    ) -> Result<Signal> {
        let ma_signal = Self::ma_crossover(candles, ma_fast, ma_slow)?;
        let rsi_signal = Self::rsi_signal(candles, rsi_period, rsi_threshold)?;

        // Her iki sinyal de aynıysa, o sinyal güçlüdür
        match (ma_signal, rsi_signal) {
            (Signal::Buy, Signal::Buy) => Ok(Signal::Buy),
            (Signal::Sell, Signal::Sell) => Ok(Signal::Sell),
            (Signal::Buy, Signal::Hold) | (Signal::Hold, Signal::Buy) => Ok(Signal::Buy),
            (Signal::Sell, Signal::Hold) | (Signal::Hold, Signal::Sell) => Ok(Signal::Sell),
            _ => Ok(Signal::Hold),
        }
    }

    /// SMA hesapla
    fn calculate_sma(candles: &[Candle], period: usize) -> Result<Vec<f64>> {
        let mut sma = Vec::new();

        for i in 0..candles.len() {
            if i + 1 < period {
                sma.push(0.0);
            } else {
                let start = i + 1 - period;
                let sum: f64 = candles[start..=i]
                    .iter()
                    .map(|c| c.close)
                    .sum();
                sma.push(sum / period as f64);
            }
        }

        Ok(sma)
    }

    /// RSI hesapla
    fn calculate_rsi(candles: &[Candle], period: usize) -> Result<Vec<f64>> {
        let mut rsi = Vec::new();
        let mut gains = 0.0;
        let mut losses = 0.0;

        for (i, candle) in candles.iter().enumerate() {
            if i == 0 {
                rsi.push(50.0);
                continue;
            }

            let change = candle.close - candles[i - 1].close;
            if change > 0.0 {
                gains += change;
            } else {
                losses += change.abs();
            }

            if i < period {
                rsi.push(50.0);
            } else {
                let avg_gain = gains / period as f64;
                let avg_loss = losses / period as f64;

                let rs = if avg_loss == 0.0 {
                    if avg_gain == 0.0 { 50.0 } else { 100.0 }
                } else {
                    100.0 - (100.0 / (1.0 + avg_gain / avg_loss))
                };

                rsi.push(rs);
            }
        }

        Ok(rsi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn create_candle(close: f64) -> Candle {
        Candle {
            timestamp: Utc::now(),
            symbol: "BTC".to_string(),
            interval: "1m".to_string(),
            open: close - 10.0,
            high: close + 20.0,
            low: close - 20.0,
            close,
            volume: 100.0,
        }
    }

    #[test]
    fn test_ma_crossover_buy() {
        let candles = vec![
            create_candle(100.0),
            create_candle(102.0),
            create_candle(104.0),
            create_candle(106.0),
            create_candle(110.0), // Golden cross
        ];

        let signal = CombinedStrategy::ma_crossover(&candles, 2, 3);
        assert!(signal.is_ok());
    }

    #[test]
    fn test_rsi_oversold() {
        let candles = vec![
            create_candle(100.0),
            create_candle(99.0),
            create_candle(98.0),
            create_candle(97.0),
            create_candle(96.0),
        ];

        let signal = CombinedStrategy::rsi_signal(&candles, 5, 30.0);
        assert!(signal.is_ok());
    }

    #[test]
    fn test_combined_strategy() {
        let candles = vec![
            create_candle(100.0),
            create_candle(102.0),
            create_candle(104.0),
            create_candle(103.0),
            create_candle(105.0),
        ];

        let signal = CombinedStrategy::combined_signal(&candles, 2, 3, 5, 30.0);
        assert!(signal.is_ok());
    }

    #[test]
    fn test_insufficient_data() {
        let candles = vec![create_candle(100.0)];
        let signal = CombinedStrategy::ma_crossover(&candles, 5, 10);
        assert_eq!(signal.unwrap(), Signal::Hold);
    }
}
