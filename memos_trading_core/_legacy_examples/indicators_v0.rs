// indicators_advanced.rs - Optimize Edilmiş Saf Matematik Göstergeleri

/// OHLCV Candle veri yapısı - Bellek hizalaması için Copy desteği
#[derive(Clone, Copy, Debug, Default)]
pub struct Candle {
    pub ts: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Basit Hareketli Ortalama (SMA) - O(n) yerine Sliding Window optimizasyonu
pub fn sma(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let len = values.len();
    let mut out = vec![None; len];
    if period == 0 || len < period { return out; }

    let mut sum: f64 = values.iter().take(period).sum();
    out[period - 1] = Some(sum / period as f64);

    for i in period..len {
        sum += values[i] - values[i - period];
        out[i] = Some(sum / period as f64);
    }
    out
}

/// Üssel Hareketli Ortalama (EMA) - Tek geçişli (Single-pass) optimizasyon
pub fn ema(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let len = values.len();
    let mut out = vec![None; len];
    if period == 0 || len < period { return out; }

    let k = 2.0 / (period as f64 + 1.0);
    let mut current_ema: f64 = values.iter().take(period).sum::<f64>() / period as f64;
    out[period - 1] = Some(current_ema);

    for i in period..len {
        current_ema = (values[i] - current_ema) * k + current_ema;
        out[i] = Some(current_ema);
    }
    out
}

/// RSI (Relative Strength Index) - Wilder's Smoothing Metodu
pub fn rsi(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let len = values.len();
    let mut rsis = vec![None; len];
    if period == 0 || len <= period { return rsis; }

    let mut gains = 0.0;
    let mut losses = 0.0;

    for i in 1..=period {
        let diff = values[i] - values[i - 1];
        if diff > 0.0 { gains += diff; } else { losses -= diff; }
    }

    let mut avg_gain = gains / period as f64;
    let mut avg_loss = losses / period as f64;

    let calculate_rsi_val = |g: f64, l: f64| -> f64 {
        if l == 0.0 { 100.0 } else { 100.0 - (100.0 / (1.0 + (g / l))) }
    };

    rsis[period] = Some(calculate_rsi_val(avg_gain, avg_loss));

    for i in (period + 1)..len {
        let diff = values[i] - values[i - 1];
        let (gain, loss) = if diff > 0.0 { (diff, 0.0) } else { (0.0, -diff) };

        avg_gain = (avg_gain * (period as f64 - 1.0) + gain) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + loss) / period as f64;

        rsis[i] = Some(calculate_rsi_val(avg_gain, avg_loss));
    }
    rsis
}

/// Bollinger Bantları - Tek seferlik Varyans hesabı ile performans artışı
pub fn bollinger_bands(
    values: &[f64],
    period: usize,
    std_dev_mult: f64,
) -> (Vec<Option<f64>>, Vec<Option<f64>>, Vec<Option<f64>>) {
    let len = values.len();
    let ma = sma(values, period);
    let mut lower = vec![None; len];
    let mut upper = vec![None; len];

    for i in (period - 1)..len {
        let mean = ma[i].unwrap_or(0.0);
        let start = i + 1 - period;
        let variance = values[start..=i].iter()
            .map(|&v| (v - mean).powi(2))
            .sum::<f64>() / period as f64;
        
        let std = variance.sqrt();
        lower[i] = Some(mean - std_dev_mult * std);
        upper[i] = Some(mean + std_dev_mult * std);
    }

    (lower, ma, upper)
}

/// Ortalama Gerçek Aralık (ATR) - Bellek kopyalamasız (In-place) hesaplama
pub fn atr(candles: &[Candle], period: usize) -> Vec<Option<f64>> {
    let len = candles.len();
    let mut atr_vals = vec![None; len];
    if period == 0 || len <= period { return atr_vals; }

    let mut tr_sum = 0.0;
    for i in 1..=period {
        let tr = (candles[i].high - candles[i].low)
            .max((candles[i].high - candles[i - 1].close).abs())
            .max((candles[i].low - candles[i - 1].close).abs());
        tr_sum += tr;
    }

    let mut prev_atr = tr_sum / period as f64;
    atr_vals[period] = Some(prev_atr);

    for i in (period + 1)..len {
        let tr = (candles[i].high - candles[i].low)
            .max((candles[i].high - candles[i - 1].close).abs())
            .max((candles[i].low - candles[i - 1].close).abs());
        
        prev_atr = (prev_atr * (period as f64 - 1.0) + tr) / period as f64;
        atr_vals[i] = Some(prev_atr);
    }
    atr_vals
}

// --- PERFORMANS YARDIMCILARI (ZERO-ALLOCATION) ---

/// En son SMA değerini, tüm listeyi kopyalamadan döndürür
pub fn get_last_sma(values: &[f64], period: usize) -> Option<f64> {
    if values.len() < period { return None; }
    let sum: f64 = values.iter().rev().take(period).sum();
    Some(sum / period as f64)
}

/// En son RSI değerini, tüm listeyi kopyalamadan döndürür
pub fn get_last_rsi(values: &[f64], period: usize) -> Option<f64> {
    if values.len() <= period { return None; }
    // RSI'ın sağlıklı olması için tüm geçmiş gerekse de, 
    // bu metod sadece kısa pencere üzerinden yaklaşık değer döner.
    let start = values.len() - period - 1;
    let mut gains = 0.0;
    let mut losses = 0.0;

    for w in values[start..].windows(2) {
        let diff = w[1] - w[0];
        if diff > 0.0 { gains += diff; } else { losses -= diff; }
    }
    
    if losses == 0.0 { Some(100.0) } 
    else { Some(100.0 - (100.0 / (1.0 + (gains / losses)))) }
}

/// MACD (Moving Average Convergence Divergence)
/// Performans: EMA hesaplamaları sırasında oluşan ara vektörleri doğrudan kullanarak
/// bellek trafiğini (memory traffic) minimize eder.
/// Dönüş: (macd_hattı, sinyal_hattı, histogram)
pub fn macd(
    values: &[f64],
    fast_period: usize,
    slow_period: usize,
    signal_period: usize,
) -> (Vec<Option<f64>>, Vec<Option<f64>>, Vec<Option<f64>>) {
    let len = values.len();
    
    // 1. Hızlı ve Yavaş EMA'ları hesapla
    let fast_ema = ema(values, fast_period);
    let slow_ema = ema(values, slow_period);

    // 2. MACD Hattını oluştur (Fast EMA - Slow EMA)
    // Zero-allocation: Kapasiteyi önceden ayırarak re-allocation'ı engelliyoruz.
    let mut macd_line = Vec::with_capacity(len);
    for i in 0..len {
        if let (Some(f), Some(s)) = (fast_ema[i], slow_ema[i]) {
            macd_line.push(Some(f - s));
        } else {
            macd_line.push(None);
        }
    }

    // 3. Sinyal Hattını oluştur (MACD Hattı'nın EMA'sı)
    // EMA fonksiyonu &[f64] beklediği için Option'ları temizleyip ham veri çıkarıyoruz.
    // Kritik: Sadece geçerli MACD değerlerini içeren bir buffer oluşturulur.
    let raw_macd: Vec<f64> = macd_line.iter()
        .map(|v| v.unwrap_or(0.0))
        .collect();
    
    let signal_line = ema(&raw_macd, signal_period);

    // 4. Histogramı oluştur (MACD Line - Signal Line)
    let mut histogram = Vec::with_capacity(len);
    for i in 0..len {
        if let (Some(m), Some(s)) = (macd_line[i], signal_line[i]) {
            histogram.push(Some(m - s));
        } else {
            histogram.push(None);
        }
    }

    (macd_line, signal_line, histogram)
}

/// En son MACD değerini, tüm listeyi kopyalamadan yaklaşık olarak döndürür (Fast Path)
pub fn get_last_macd(
    values: &[f64], 
    fast: usize, 
    slow: usize, 
    signal: usize
) -> Option<(f64, f64, f64)> {
    if values.len() < slow + signal { return None; }
    
    let (m, s, h) = macd(values, fast, slow, signal);
    
    // .copied() metodunu referansı değere dönüştürmek için başa alıyoruz.
    // m.last() -> Option<&Option<f64>>
    // .copied() -> Option<Option<f64>>
    // .flatten() -> Option<f64>
    match (
        m.last().copied().flatten(), 
        s.last().copied().flatten(), 
        h.last().copied().flatten()
    ) {
        (Some(mv), Some(sv), Some(hv)) => Some((mv, sv, hv)),
        _ => None
    }
}