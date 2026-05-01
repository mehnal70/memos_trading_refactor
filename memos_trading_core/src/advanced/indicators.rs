//! Gelişmiş teknik göstergeler
//! Tüm göstergeler pure math - veritabanı veya UI bağımlılığı yok

/// OHLCV Candle veri yapısı
#[derive(Clone, Copy, Debug)]
pub struct Candle {
    pub ts: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Basit Hareketli Ortalama (SMA)
/// `values.len()` kadar `Option<f64>` döner; yeterli veri gelene kadar `None`.
pub fn sma(values: &[f64], period: usize) -> Vec<Option<f64>> {
    if period == 0 {
        return vec![None; values.len()];
    }
    let mut out = Vec::with_capacity(values.len());
    let mut sum = 0.0;
    for (i, &v) in values.iter().enumerate() {
        sum += v;
        if i >= period {
            sum -= values[i - period];
        }
        if i + 1 >= period {
            out.push(Some(sum / period as f64));
        } else {
            out.push(None);
        }
    }
    out
}

/// Üssel Hareketli Ortalama (EMA)
pub fn ema(values: &[f64], period: usize) -> Vec<Option<f64>> {
    if period == 0 || values.is_empty() {
        return vec![None; values.len()];
    }

    let k = 2.0 / (period as f64 + 1.0);
    let mut out = Vec::with_capacity(values.len());
    let mut prev_ema = 0.0;

    for (i, &v) in values.iter().enumerate() {
        if i + 1 < period {
            out.push(None);
            prev_ema += v;
        } else if i + 1 == period {
            prev_ema = prev_ema / period as f64;
            out.push(Some(prev_ema));
        } else {
            prev_ema = (v - prev_ema) * k + prev_ema;
            out.push(Some(prev_ema));
        }
    }

    out
}

/// RSI (Relative Strength Index)
pub fn rsi(values: &[f64], period: usize) -> Vec<Option<f64>> {
    if period == 0 || values.len() < period + 1 {
        return vec![None; values.len()];
    }

    let mut rsis = vec![None; values.len()];
    let mut gains = 0.0;
    let mut losses = 0.0;

    // İlk period için başlangıç ortalamaları
    for i in 1..=period {
        let diff = values[i] - values[i - 1];
        if diff > 0.0 {
            gains += diff;
        } else {
            losses -= diff;
        }
    }
    let mut avg_gain = gains / period as f64;
    let mut avg_loss = losses / period as f64;

    let mut rsi_val = if avg_loss == 0.0 {
        100.0
    } else {
        let rs = avg_gain / avg_loss;
        100.0 - (100.0 / (1.0 + rs))
    };
    rsis[period] = Some(rsi_val);

    // Devam eden değerler
    for i in (period + 1)..values.len() {
        let diff = values[i] - values[i - 1];
        let gain = if diff > 0.0 { diff } else { 0.0 };
        let loss = if diff < 0.0 { -diff } else { 0.0 };

        avg_gain = (avg_gain * (period as f64 - 1.0) + gain) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + loss) / period as f64;

        rsi_val = if avg_loss == 0.0 {
            100.0
        } else {
            let rs = avg_gain / avg_loss;
            100.0 - (100.0 / (1.0 + rs))
        };
        rsis[i] = Some(rsi_val);
    }

    rsis
}

/// MACD (Moving Average Convergence Divergence)
/// Dönüş: (macd_line, signal_line, histogram)
pub fn macd(
    values: &[f64],
    fast_period: usize,
    slow_period: usize,
    signal_period: usize,
) -> (Vec<Option<f64>>, Vec<Option<f64>>, Vec<Option<f64>>) {
    let fast = ema(values, fast_period);
    let slow = ema(values, slow_period);

    let mut macd_line = Vec::with_capacity(values.len());
    for i in 0..values.len() {
        match (fast[i], slow[i]) {
            (Some(f), Some(s)) => macd_line.push(Some(f - s)),
            _ => macd_line.push(None),
        }
    }

    // signal line: ema(MACD)
    let macd_values: Vec<f64> = macd_line
        .iter()
        .map(|v| v.unwrap_or(0.0))
        .collect();
    let signal_line = ema(&macd_values, signal_period);

    let mut hist = Vec::with_capacity(values.len());
    for i in 0..values.len() {
        match (macd_line[i], signal_line[i]) {
            (Some(m), Some(s)) => hist.push(Some(m - s)),
            _ => hist.push(None),
        }
    }

    (macd_line, signal_line, hist)
}

/// Bollinger Bantları
/// Dönüş: (alt_band, orta_sma, üst_band)
pub fn bollinger_bands(
    values: &[f64],
    period: usize,
    std_dev_multiplier: f64,
) -> (Vec<Option<f64>>, Vec<Option<f64>>, Vec<Option<f64>>) {
    if period == 0 || values.len() < period {
        return (
            vec![None; values.len()],
            vec![None; values.len()],
            vec![None; values.len()],
        );
    }

    let ma = sma(values, period);
    let mut lower = Vec::with_capacity(values.len());
    let mut upper = Vec::with_capacity(values.len());

    for i in 0..values.len() {
        if i + 1 < period {
            lower.push(None);
            upper.push(None);
            continue;
        }
        let start = i + 1 - period;
        let slice = &values[start..=i];
        let mean = ma[i].unwrap_or(0.0);
        let var = slice
            .iter()
            .map(|v| {
                let d = v - mean;
                d * d
            })
            .sum::<f64>()
            / period as f64;
        let std = var.sqrt();
        lower.push(Some(mean - std_dev_multiplier * std));
        upper.push(Some(mean + std_dev_multiplier * std));
    }

    (lower, ma, upper)
}

/// Ortalama Gerçek Aralık (ATR)
pub fn atr(candles: &[Candle], period: usize) -> Vec<Option<f64>> {
    if period == 0 || candles.len() < period + 1 {
        return vec![None; candles.len()];
    }

    let mut trs = Vec::with_capacity(candles.len());
    trs.push(None); // ilk bar için TR yok

    for i in 1..candles.len() {
        let c = candles[i];
        let prev_close = candles[i - 1].close;
        let tr1 = c.high - c.low;
        let tr2 = (c.high - prev_close).abs();
        let tr3 = (c.low - prev_close).abs();
        let tr = tr1.max(tr2).max(tr3);
        trs.push(Some(tr));
    }

    // Wilder ATR (EMA benzeri)
    let mut atr_vals = vec![None; candles.len()];
    let mut sum = 0.0;
    for i in 1..=period {
        sum += trs[i].unwrap_or(0.0);
    }
    let mut prev_atr = sum / period as f64;
    atr_vals[period] = Some(prev_atr);

    for i in (period + 1)..candles.len() {
        let tr = trs[i].unwrap_or(0.0);
        prev_atr = (prev_atr * (period as f64 - 1.0) + tr) / period as f64;
        atr_vals[i] = Some(prev_atr);
    }

    atr_vals
}

/// Son SMA değerini döner (None ise 0.0)
pub fn last_sma(values: &[f64], period: usize) -> f64 {
    sma(values, period)
        .last()
        .and_then(|&v| v)
        .unwrap_or(0.0)
}

/// Son RSI değerini döner (None ise 0.0)
pub fn last_rsi(values: &[f64], period: usize) -> f64 {
    rsi(values, period)
        .last()
        .and_then(|&v| v)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sma() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let result = sma(&values, 3);
        assert_eq!(result[0], None);
        assert_eq!(result[1], None);
        assert_eq!(result[2], Some(2.0)); // (1+2+3)/3
        assert_eq!(result[3], Some(3.0)); // (2+3+4)/3
        assert_eq!(result[4], Some(4.0)); // (3+4+5)/3
    }

    #[test]
    fn test_rsi() {
        let values = vec![
            44.0, 44.34, 44.09, 43.61, 44.33, 44.83, 45.10, 45.42, 45.84, 46.08,
            45.89, 46.03, 45.61, 46.28, 46.00, 46.00, 46.00, 46.00, 46.00, 46.00,
        ];
        let result = rsi(&values, 14);
        // RSI değerleri test edilebilir
        assert!(result[14].is_some());
    }
}
