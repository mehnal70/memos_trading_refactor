// robot/indicators.rs - Gelişmiş teknik göstergeler, modüler ve extensible yapı
// Türkçe inline açıklamalar, robotik trade sistemine tam uyumlu

use crate::core::types::Candle;

// Hareketli Ortalama (SMA)
pub fn calculate_sma(candles: &[Candle], period: usize) -> Option<f64> {
    if candles.len() < period { return None; }
    let sum: f64 = candles[candles.len()-period..].iter().map(|c| c.close).sum();
    Some(sum / period as f64)
}

// Üssel Hareketli Ortalama (EMA)
// Seed: ilk N barın SMA'sı; N+1. bardan itibaren EMA formülü uygulanır.
// Önceki implementasyon seed ile iterasyonu aynı pencereye uyguluyordu (çift sayım).
pub fn calculate_ema(candles: &[Candle], period: usize) -> Option<f64> {
    if candles.len() < period { return None; }
    let k = 2.0 / (period as f64 + 1.0);
    let mut ema = candles[..period].iter().map(|c| c.close).sum::<f64>() / period as f64;
    for c in &candles[period..] {
        ema = c.close * k + ema * (1.0 - k);
    }
    Some(ema)
}

// RSI — Wilder's Smoothed Moving Average (SMMA)
// Seed: ilk period değişiminin basit ortalaması.
// Devam: SMMA(n) = (prev × (n-1) + current) / n  →  standart platform değerleriyle uyumlu.
// Önceki implementasyon yalnızca son N barın basit ortalamasını kullanıyordu.
pub fn calculate_rsi(candles: &[Candle], period: usize) -> Option<f64> {
    if candles.len() < period + 1 { return None; }
    // Seed aşaması: ilk period adım
    let mut avg_gain = 0.0_f64;
    let mut avg_loss = 0.0_f64;
    for i in 1..=period {
        let diff = candles[i].close - candles[i - 1].close;
        if diff > 0.0 { avg_gain += diff; } else { avg_loss += diff.abs(); }
    }
    avg_gain /= period as f64;
    avg_loss /= period as f64;
    // Wilder smoothing: geri kalan barlar
    for i in (period + 1)..candles.len() {
        let diff = candles[i].close - candles[i - 1].close;
        let gain = if diff > 0.0 { diff } else { 0.0 };
        let loss = if diff < 0.0 { diff.abs() } else { 0.0 };
        avg_gain = (avg_gain * (period - 1) as f64 + gain) / period as f64;
        avg_loss = (avg_loss * (period - 1) as f64 + loss) / period as f64;
    }
    let rs = if avg_loss == 0.0 { 100.0 } else { avg_gain / avg_loss };
    Some(100.0 - (100.0 / (1.0 + rs)))
}

// MACD — signal line, MACD serisinin EMA'sıdır (fiyat EMA'sı değil)
pub fn calculate_macd(candles: &[Candle], fast: usize, slow: usize, signal: usize) -> Option<(f64, f64, f64)> {
    if candles.len() < slow + signal { return None; }
    let k_fast   = 2.0 / (fast   as f64 + 1.0);
    let k_slow   = 2.0 / (slow   as f64 + 1.0);
    let k_signal = 2.0 / (signal as f64 + 1.0);
    // MACD serisi: her zaman adımı için EMA_fast - EMA_slow
    let mut ema_f = candles[0].close;
    let mut ema_s = candles[0].close;
    let mut macd_series: Vec<f64> = Vec::with_capacity(candles.len());
    for c in candles {
        ema_f = c.close * k_fast   + ema_f * (1.0 - k_fast);
        ema_s = c.close * k_slow   + ema_s * (1.0 - k_slow);
        macd_series.push(ema_f - ema_s);
    }
    // Signal line: MACD serisinin EMA'sı — ilk `signal` değerin SMA'sı seed olarak kullanılır
    if macd_series.len() < signal { return None; }
    let mut sig = macd_series[..signal].iter().sum::<f64>() / signal as f64;
    for &m in &macd_series[signal..] {
        sig = m * k_signal + sig * (1.0 - k_signal);
    }
    let macd_val = *macd_series.last()?;
    Some((macd_val, sig, macd_val - sig))
}

// Bollinger Bands
pub fn calculate_bollinger(candles: &[Candle], period: usize, std_dev: f64) -> Option<(f64, f64, f64)> {
    if candles.len() < period { return None; }
    let sma = calculate_sma(candles, period)?;
    let closes: Vec<f64> = candles[candles.len()-period..].iter().map(|c| c.close).collect();
    let mean = sma;
    let variance = closes.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / period as f64;
    let std = variance.sqrt();
    Some((sma + std_dev * std, sma, sma - std_dev * std))
}

// Stochastic Oscillator
pub fn calculate_stochastic(candles: &[Candle], period: usize) -> Option<f64> {
    if candles.len() < period { return None; }
    let highest = candles[candles.len()-period..].iter().map(|c| c.high).fold(f64::MIN, f64::max);
    let lowest = candles[candles.len()-period..].iter().map(|c| c.low).fold(f64::MAX, f64::min);
    let close = candles.last()?.close;
    Some((close - lowest) / (highest - lowest) * 100.0)
}

// ATR
pub fn calculate_atr(candles: &[Candle], period: usize) -> Option<f64> {
    if candles.len() < period + 1 { return None; }
    let mut trs = vec![];
    for i in (candles.len()-period)..candles.len() {
        let high = candles[i].high;
        let low = candles[i].low;
        let prev_close = if i == 0 { candles[i].close } else { candles[i-1].close };
        let tr = (high - low).max((high - prev_close).abs()).max((low - prev_close).abs());
        trs.push(tr);
    }
    Some(trs.iter().sum::<f64>() / period as f64)
}

// Parabolic SAR (basit versiyon)
pub fn calculate_parabolic_sar(candles: &[Candle], step: f64, max: f64) -> Option<f64> {
    if candles.len() < 2 { return None; }
    let mut sar = candles[candles.len()-2].low;
    let mut ep = candles[candles.len()-2].high;
    let mut af = step;
    let uptrend = candles.last()?.close > candles[candles.len()-2].close;
    for i in (candles.len()-2)..candles.len() {
        if uptrend {
            sar += af * (ep - sar);
            if candles[i].high > ep {
                ep = candles[i].high;
                af = (af + step).min(max);
            }
        } else {
            sar -= af * (sar - ep);
            if candles[i].low < ep {
                ep = candles[i].low;
                af = (af + step).min(max);
            }
        }
    }
    Some(sar)
}

// Yeni göstergeler eklemek için sadece yeni fonksiyon ekle, strateji modülleri otomatik kullanabilir.

// Williams %R (-100 .. 0)
pub fn calculate_williams_r(candles: &[Candle], period: usize) -> Option<f64> {
    if candles.len() < period { return None; }
    let slice = &candles[candles.len() - period..];
    let highest = slice.iter().map(|c| c.high).fold(f64::MIN, f64::max);
    let lowest = slice.iter().map(|c| c.low).fold(f64::MAX, f64::min);
    let close = candles.last()?.close;
    let range = (highest - lowest).max(f64::EPSILON);
    let wr = (highest - close) / range * -100.0;
    Some(wr)
}

// ADX (+DI, -DI dahil)
pub fn calculate_adx(candles: &[Candle], period: usize) -> Option<(f64, f64, f64)> {
    if candles.len() < period + 1 { return None; }
    let len = candles.len();
    // Diziler
    let mut tr_vec = Vec::with_capacity(period);
    let mut plus_dm_vec = Vec::with_capacity(period);
    let mut minus_dm_vec = Vec::with_capacity(period);
    for i in len - period..len {
        let (prev, cur) = if i == 0 { (i, i) } else { (i - 1, i) };
        let high = candles[cur].high;
        let low = candles[cur].low;
        let prev_close = candles[prev].close;
        let tr = (high - low)
            .max((high - prev_close).abs())
            .max((low - prev_close).abs());
        tr_vec.push(tr);
        let up_move = high - candles[prev].high;
        let down_move = candles[prev].low - low;
        plus_dm_vec.push(if up_move > down_move && up_move > 0.0 { up_move } else { 0.0 });
        minus_dm_vec.push(if down_move > up_move && down_move > 0.0 { down_move } else { 0.0 });
    }
    let tr_sum: f64 = tr_vec.iter().sum();
    if tr_sum == 0.0 { return None; }
    let plus_di = 100.0 * (plus_dm_vec.iter().sum::<f64>() / tr_sum);
    let minus_di = 100.0 * (minus_dm_vec.iter().sum::<f64>() / tr_sum);
    let dx = 100.0 * (plus_di - minus_di).abs() / (plus_di + minus_di).max(f64::EPSILON);
    // Basitçe son periyodun DX'ini ADX olarak al (RMA yerine basit yaklaşım)
    Some((dx, plus_di, minus_di))
}

// VWAP (son N bar için)
pub fn calculate_vwap(candles: &[Candle], period: usize) -> Option<f64> {
    if candles.len() < period { return None; }
    let slice = &candles[candles.len() - period..];
    let mut pv_sum = 0.0;
    let mut v_sum = 0.0;
    for c in slice {
        let typical = (c.high + c.low + c.close) / 3.0;
        pv_sum += typical * c.volume;
        v_sum += c.volume;
    }
    if v_sum <= 0.0 { return None; }
    Some(pv_sum / v_sum)
}

/// EMA serisi: tüm barların EMA dizisini döndürür (trend yönü için gerekli).
/// Supertrend ve EMA-crossover stratejilerinde kullanılır.
pub fn calculate_ema_series(candles: &[Candle], period: usize) -> Option<Vec<f64>> {
    if candles.len() < period { return None; }
    let k = 2.0 / (period as f64 + 1.0);
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let mut ema = closes[..period].iter().sum::<f64>() / period as f64;
    let mut series = vec![ema];
    for &close in &closes[period..] {
        ema = close * k + ema * (1.0 - k);
        series.push(ema);
    }
    Some(series)
}

/// Supertrend (ATR tabanlı trend takip göstergesi).
/// Döndürür: (trend_yonu: 1=yukarı / -1=aşağı, supertrend_değeri)
/// period = ATR periyodu (genellikle 10), multiplier = ATR çarpanı (genellikle 3.0)
pub fn calculate_supertrend(
    candles: &[Candle],
    period: usize,
    multiplier: f64,
) -> Option<(i8, f64)> {
    let n = candles.len();
    if n < period + 1 { return None; }

    // Her bar için ATR hesapla (EMA tabanlı ATR — daha pürüzsüz)
    let mut atr_series = vec![0.0_f64; n];
    {
        let mut sum = 0.0_f64;
        for i in 1..=period {
            let tr = (candles[i].high - candles[i].low)
                .max((candles[i].high - candles[i - 1].close).abs())
                .max((candles[i].low  - candles[i - 1].close).abs());
            sum += tr;
        }
        atr_series[period] = sum / period as f64;
        let k = 1.0 / period as f64; // RMA (Wilder's smoothing) faktörü
        for i in (period + 1)..n {
            let tr = (candles[i].high - candles[i].low)
                .max((candles[i].high - candles[i - 1].close).abs())
                .max((candles[i].low  - candles[i - 1].close).abs());
            atr_series[i] = atr_series[i - 1] * (1.0 - k) + tr * k;
        }
    }

    // Supertrend bandlarını hesapla
    let mut upper = vec![0.0_f64; n];
    let mut lower = vec![0.0_f64; n];
    let mut trend = vec![1_i8; n]; // 1=up, -1=down
    let mut st_val = vec![0.0_f64; n];

    for i in period..n {
        let hl2 = (candles[i].high + candles[i].low) / 2.0;
        let atr = atr_series[i];
        upper[i] = hl2 + multiplier * atr;
        lower[i] = hl2 - multiplier * atr;

        // Bant daraltma mantığı (önceki değerlerle karşılaştır)
        if i > period {
            lower[i] = lower[i].max(if candles[i - 1].close > lower[i - 1] { lower[i - 1] } else { lower[i] });
            upper[i] = upper[i].min(if candles[i - 1].close < upper[i - 1] { upper[i - 1] } else { upper[i] });
        }

        // Trend yönünü belirle
        let close = candles[i].close;
        trend[i] = if i == period {
            1
        } else if trend[i - 1] == 1 {
            if close < lower[i] { -1 } else { 1 }
        } else {
            if close > upper[i] { 1 } else { -1 }
        };
        st_val[i] = if trend[i] == 1 { lower[i] } else { upper[i] };
    }

    let last_idx = n - 1;
    if last_idx < period { return None; }
    Some((trend[last_idx], st_val[last_idx]))
}

/// StochasticRSI: RSI serisine Stochastic uygular.
/// Döndürür: (k_pct, d_pct) — 0..100 aralığında
pub fn calculate_stochastic_rsi(
    candles: &[Candle],
    rsi_period: usize,
    stoch_period: usize,
    smooth_k: usize,
    smooth_d: usize,
) -> Option<(f64, f64)> {
    let needed = rsi_period + stoch_period + smooth_k + smooth_d + 5;
    if candles.len() < needed { return None; }

    // RSI serisini oluştur
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let mut rsi_series: Vec<f64> = Vec::new();
    let mut avg_gain = 0.0_f64;
    let mut avg_loss = 0.0_f64;
    for i in 1..=rsi_period {
        let diff = closes[i] - closes[i - 1];
        if diff > 0.0 { avg_gain += diff; } else { avg_loss += diff.abs(); }
    }
    avg_gain /= rsi_period as f64;
    avg_loss /= rsi_period as f64;
    let rs = if avg_loss == 0.0 { 100.0 } else { avg_gain / avg_loss };
    rsi_series.push(100.0 - 100.0 / (1.0 + rs));
    for i in (rsi_period + 1)..closes.len() {
        let diff = closes[i] - closes[i - 1];
        let gain = if diff > 0.0 { diff } else { 0.0 };
        let loss = if diff < 0.0 { diff.abs() } else { 0.0 };
        avg_gain = (avg_gain * (rsi_period - 1) as f64 + gain) / rsi_period as f64;
        avg_loss = (avg_loss * (rsi_period - 1) as f64 + loss) / rsi_period as f64;
        let rs2 = if avg_loss == 0.0 { 100.0 } else { avg_gain / avg_loss };
        rsi_series.push(100.0 - 100.0 / (1.0 + rs2));
    }

    if rsi_series.len() < stoch_period + smooth_k + smooth_d { return None; }

    // Stochastic K serisini RSI üzerine uygula
    let mut raw_k: Vec<f64> = Vec::new();
    for i in stoch_period..=rsi_series.len() {
        let slice = &rsi_series[i - stoch_period..i];
        let max = slice.iter().cloned().fold(f64::MIN, f64::max);
        let min = slice.iter().cloned().fold(f64::MAX, f64::min);
        let range = (max - min).max(f64::EPSILON);
        raw_k.push(100.0 * (rsi_series[i - 1] - min) / range);
    }

    // K smooth (SMA)
    let smooth_k_series: Vec<f64> = raw_k.windows(smooth_k)
        .map(|w| w.iter().sum::<f64>() / smooth_k as f64).collect();
    if smooth_k_series.len() < smooth_d { return None; }

    // D smooth (SMA of K)
    let d_series: Vec<f64> = smooth_k_series.windows(smooth_d)
        .map(|w| w.iter().sum::<f64>() / smooth_d as f64).collect();

    let k = *smooth_k_series.last()?;
    let d = *d_series.last()?;
    Some((k, d))
}

/// CCI (Commodity Channel Index) — döngüsel dönüm noktaları için.
/// Standart: period=20. Overbought >+100, Oversold <-100.
pub fn calculate_cci(candles: &[Candle], period: usize) -> Option<f64> {
    if candles.len() < period { return None; }
    let slice = &candles[candles.len() - period..];
    let tp: Vec<f64> = slice.iter().map(|c| (c.high + c.low + c.close) / 3.0).collect();
    let mean = tp.iter().sum::<f64>() / period as f64;
    let mad = tp.iter().map(|x| (x - mean).abs()).sum::<f64>() / period as f64;
    if mad == 0.0 { return None; }
    Some((tp.last()? - mean) / (0.015 * mad))
}
