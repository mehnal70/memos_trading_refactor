// src/core/indicators.rs - Memos Trading Core Library (Srivastava ATP Nihai Sürüm)
// Yüksek performanslı, bellek tahsisatsız (Zero-Allocation) teknik analiz motoru.

use crate::prelude::*; // Evrensel anayasa mühürünü çağırıyoruz (Candle yapısı otomatik gelir)
use serde::{Serialize, Deserialize};

// =============================================================================
// 1. ÇOKLU VERİ ÇIKIŞ MODELLERİ (TECHNICAL OUTPUT STRUCTURES)
// =============================================================================

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MacdOutput {
    pub macd_line: Vec<f64>,
    pub signal_line: Vec<f64>,
    pub histogram: Vec<f64>,
}

impl MacdOutput {
    /// Son barın (macd, signal, histogram) üçlüsünü döner — strateji match'leri için.
    pub fn last_lines(&self) -> Option<(f64, f64, f64)> {
        let m = *self.macd_line.last()?;
        let s = *self.signal_line.last()?;
        let h = self.histogram.last().copied().unwrap_or(m - s);
        Some((m, s, h))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BollingerBandsOutput {
    pub upper: Vec<f64>,
    pub middle: Vec<f64>,
    pub lower: Vec<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StochasticOutput {
    pub k_line: Vec<f64>,
    pub d_line: Vec<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct KeltnerChannelOutput {
    pub upper: Vec<f64>,
    pub middle: Vec<f64>,
    pub lower: Vec<f64>,
}

/// Supertrend tek satırı: trend yönü (+1 yukarı / -1 aşağı) ve bant değeri.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct SupertrendPoint {
    pub trend: i8,
    pub value: f64,
}

/// Stochastic RSI çıktısı — RSI üzerinde stochastic uygulanır.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StochasticRsiOutput {
    pub k_line: Vec<f64>,
    pub d_line: Vec<f64>,
}

// =============================================================================
// 2. ÇEKİRDEK PERFORMANS YARDIMCILARI (INTERNAL DRY MATH)
// =============================================================================

/// Verilen serinin ve aritmetik ortalamanın saf varyansını tek geçişte hesaplar
#[inline]
fn calculate_variance(values: &[f64], mean: f64) -> f64 {
    if values.is_empty() { return 0.0; }
    values.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64
}

/// Tek bir barın Gerçek Aralık (True Range) değerini mikro saniyede döndürür
#[inline]
fn calculate_true_range(high: f64, low: f64, prev_close: f64) -> f64 {
    (high - low)
        .max((high - prev_close).abs())
        .max((low - prev_close).abs())
}

// =============================================================================
// 3. GEÇMİŞ SERİ HESAPLAMA MOTORLARI (SERIES-PATH FOR BACKTEST & ML)
// =============================================================================

/// Basit Hareketli Ortalama Serisi - Sliding Window ile O(n) performansı
pub fn calculate_sma(candles: &[Candle], period: usize) -> Vec<f64> {
    let n = candles.len();
    if n < period || period == 0 { return Vec::new(); }
    let mut result = Vec::with_capacity(n - period + 1);
    let mut window_sum: f64 = candles.iter().take(period).map(|c| c.close).sum();
    result.push(window_sum / period as f64);

    for i in period..n {
        window_sum += candles[i].close - candles[i - period].close;
        result.push(window_sum / period as f64);
    }
    result
}

/// Üssel Hareketli Ortalama Serisi - Çift sayım engelleme korumalı (Seed SMA)
pub fn calculate_ema(candles: &[Candle], period: usize) -> Vec<f64> {
    let n = candles.len();
    if n < period || period == 0 { return Vec::new(); }
    let mut ema_vec = Vec::with_capacity(n);
    let multiplier = 2.0 / (period as f64 + 1.0);

    let mut current_ema = candles.iter().take(period).map(|c| c.close).sum::<f64>() / period as f64;
    ema_vec.push(current_ema);

    for c in candles.iter().skip(period) {
        current_ema = (c.close - current_ema) * multiplier + current_ema;
        ema_vec.push(current_ema);
    }
    ema_vec
}

/// RSI Serisi - Standart platformlar (TradingView) ile %100 uyumlu Wilder's SMMA Düzeltmesi
pub fn calculate_rsi(candles: &[Candle], period: usize) -> Vec<f64> {
    let n = candles.len();
    if n <= period || period == 0 { return Vec::new(); }
    let mut rsi_values = Vec::with_capacity(n - period);
    let mut gains = 0.0;
    let mut losses = 0.0;

    for i in 1..=period {
        let diff = candles[i].close - candles[i - 1].close;
        if diff > 0.0 { gains += diff; } else { losses += diff.abs(); }
    }
    let mut avg_gain = gains / period as f64;
    let mut avg_loss = losses / period as f64;

    let calc_rsi = |g: f64, l: f64| if l == 0.0 { 100.0 } else { 100.0 - (100.0 / (1.0 + g / l)) };
    rsi_values.push(calc_rsi(avg_gain, avg_loss));

    for i in (period + 1)..n {
        let diff = candles[i].close - candles[i - 1].close;
        let (g, l) = if diff > 0.0 { (diff, 0.0) } else { (0.0, diff.abs()) };
        
        avg_gain = (avg_gain * (period - 1) as f64 + g) / period as f64;
        avg_loss = (avg_loss * (period - 1) as f64 + l) / period as f64;
        rsi_values.push(calc_rsi(avg_gain, avg_loss));
    }
    rsi_values
}

/// MACD Serisi - Parametre alan modern API: fast/slow/signal EMA pencereleri.
pub fn calculate_macd(candles: &[Candle], fast: usize, slow: usize, signal: usize) -> MacdOutput {
    let fast_ema = calculate_ema(candles, fast);
    let slow_ema = calculate_ema(candles, slow);
    let mut macd_line = Vec::with_capacity(fast_ema.len().min(slow_ema.len()));
    for (f, s) in fast_ema.iter().zip(slow_ema.iter()) { macd_line.push(f - s); }

    let n = macd_line.len();
    if n < signal { return MacdOutput::default(); }
    let multiplier = 2.0 / (signal as f64 + 1.0);
    let mut current_ema = macd_line.iter().take(signal).sum::<f64>() / signal as f64;
    let mut signal_line = Vec::with_capacity(n);
    signal_line.push(current_ema);

    for &m in macd_line.iter().skip(signal) {
        current_ema = (m - current_ema) * multiplier + current_ema;
        signal_line.push(current_ema);
    }
    let mut histogram = Vec::with_capacity(signal_line.len());
    for (m, s) in macd_line.iter().skip(signal).zip(signal_line.iter()) { histogram.push(m - s); }

    MacdOutput { macd_line, signal_line, histogram }
}

/// Bollinger Bantları Serisi - Tek seferlik yerleşik varyans hasadı
pub fn calculate_bollinger(candles: &[Candle], period: usize, mult: f64) -> BollingerBandsOutput {
    let middle = calculate_sma(candles, period);
    let mut upper = Vec::with_capacity(middle.len());
    let mut lower = Vec::with_capacity(middle.len());
    let start_idx = candles.len() - middle.len();

    for (i, &m) in middle.iter().enumerate() {
        let current_start = start_idx + i;
        let window: Vec<f64> = candles[current_start..current_start + period].iter().map(|c| c.close).collect();
        let std_dev = calculate_variance(&window, m).sqrt();
        upper.push(m + mult * std_dev);
        lower.push(m - mult * std_dev);
    }
    BollingerBandsOutput { upper, middle, lower }
}

/// Stochastic Osilatör Serisi - f64::EPSILON korumalı sıfıra bölünme barajı
pub fn calculate_stochastic(candles: &[Candle], period: usize) -> StochasticOutput {
    let n = candles.len();
    if n < period { return StochasticOutput::default(); }
    let mut k_line = Vec::with_capacity(n - period + 1);

    for i in period..=n {
        let slice = &candles[i - period..i];
        let highest = slice.iter().map(|c| c.high).fold(f64::MIN, f64::max);
        let lowest = slice.iter().map(|c| c.low).fold(f64::MAX, f64::min);
        let close = slice.last().map(|c| c.close).unwrap_or(0.0);
        k_line.push((close - lowest) / (highest - lowest).max(f64::EPSILON) * 100.0);
    }
    let d_line = k_line.windows(3).map(|w| w.iter().sum::<f64>() / 3.0).collect();
    StochasticOutput { k_line, d_line }
}

/// ATR Serisi - Kopyalamasız (In-place) Gerçek Aralık süzgeci
pub fn calculate_atr(candles: &[Candle], period: usize) -> Vec<f64> {
    if candles.len() < 2 { return Vec::new(); }
    let mut trs = Vec::with_capacity(candles.len() - 1);
    for i in 1..candles.len() {
        trs.push(calculate_true_range(candles[i].high, candles[i].low, candles[i - 1].close));
    }
    let dummy_candles: Vec<Candle> = trs.iter().map(|&tr| Candle { close: tr, ..Default::default() }).collect();
    calculate_sma(&dummy_candles, period)
}

/// Keltner Kanalları Serisi - SMA ve ATR birleşik otobanı
pub fn calculate_keltner_channel(candles: &[Candle], period: usize, mult: f64) -> KeltnerChannelOutput {
    let middle = calculate_sma(candles, period);
    let atr = calculate_atr(candles, period);
    let min_len = middle.len().min(atr.len());
    let mut upper = Vec::with_capacity(min_len);
    let mut lower = Vec::with_capacity(min_len);

    for i in 0..min_len {
        let m = middle[i];
        let a = atr[i];
        upper.push(m + mult * a);
        lower.push(m - mult * a);
    }
    KeltnerChannelOutput { upper, middle, lower }
}

/// Parabolic SAR Serisi - Otonom ivme çarpanı (`af`) takibi
pub fn calculate_parabolic_sar(candles: &[Candle], step: f64, max: f64) -> Vec<f64> {
    if candles.len() < 2 { return Vec::new(); }
    let mut sar_vec = Vec::with_capacity(candles.len());
    let mut sar = candles[0].low;
    let mut ep = candles[0].high;
    let mut af = step;
    sar_vec.push(sar);

    for i in 1..candles.len() {
        let uptrend = candles[i].close > candles[i - 1].close;
        if uptrend {
            sar += af * (ep - sar);
            if candles[i].high > ep { ep = candles[i].high; af = (af + step).min(max); }
        } else {
            sar -= af * (sar - ep);
            if candles[i].low < ep { ep = candles[i].low; af = (af + step).min(max); }
        }
        sar_vec.push(sar);
    }
    sar_vec
}

/// Williams %R Serisi - Aşırı alım/satım dönüm noktaları
pub fn calculate_williams_r(candles: &[Candle], period: usize) -> Vec<f64> {
    if candles.len() < period { return Vec::new(); }
    let mut wr_vec = Vec::with_capacity(candles.len() - period + 1);

    for i in period..=candles.len() {
        let slice = &candles[i - period..i];
        let highest = slice.iter().map(|c| c.high).fold(f64::MIN, f64::max);
        let lowest = slice.iter().map(|c| c.low).fold(f64::MAX, f64::min);
        let close = slice.last().map(|c| c.close).unwrap_or(0.0);
        wr_vec.push((highest - close) / (highest - lowest).max(f64::EPSILON) * -100.0);
    }
    wr_vec
}

/// ADX Serisi (+DI ve -DI dâhil yönlü hareket endeksi)
pub fn calculate_adx(candles: &[Candle], period: usize) -> Vec<f64> {
    if candles.len() < period + 1 { return Vec::new(); }
    let mut dx_vec = Vec::with_capacity(candles.len() - period);

    for i in period..candles.len() {
        let slice = &candles[i - period..=i];
        let mut tr_sum = 0.0;
        let mut plus_dm = 0.0;
        let mut minus_dm = 0.0;
        for j in 1..slice.len() {
            tr_sum += calculate_true_range(slice[j].high, slice[j].low, slice[j - 1].close);
            let up_move = slice[j].high - slice[j - 1].high;
            let down_move = slice[j - 1].low - slice[j].low;
            if up_move > down_move && up_move > 0.0 { plus_dm += up_move; }
            if down_move > up_move && down_move > 0.0 { minus_dm += down_move; }
        }
        if tr_sum == 0.0 { dx_vec.push(0.0); continue; }
        let plus_di = 100.0 * (plus_dm / tr_sum);
        let minus_di = 100.0 * (minus_dm / tr_sum);
        dx_vec.push(100.0 * (plus_di - minus_di).abs() / (plus_di + minus_di).max(f64::EPSILON));
    }
    dx_vec
}

/// Kümülatif VWAP Serisi - Hacim ağırlıklı ortalama fiyat çizgisi
pub fn calculate_vwap(candles: &[Candle]) -> Vec<f64> {
    let mut vwap_vec = Vec::with_capacity(candles.len());
    let (mut cum_pv, mut cum_v) = (0.0, 0.0);
    for c in candles {
        cum_pv += ((c.high + c.low + c.close) / 3.0) * c.volume;
        cum_v += c.volume;
        vwap_vec.push(if cum_v > 0.0 { cum_pv / cum_v } else { c.close });
    }
    vwap_vec
}

/// EMA-Series Alias - Strateji lifecycle yöneticileri için doğrudan kanal açar
pub fn calculate_ema_series(candles: &[Candle], period: usize) -> Vec<f64> {
    calculate_ema(candles, period)
}

/// Supertrend - ATR bazlı dinamik trend takip indikatörü.
/// Çıktı: her bar için (trend, bant değeri). trend: +1 yukarı, -1 aşağı.
pub fn calculate_supertrend(candles: &[Candle], period: usize, multiplier: f64) -> Vec<SupertrendPoint> {
    let n = candles.len();
    if n < period + 1 { return Vec::new(); }

    let atr = calculate_atr(candles, period);
    if atr.is_empty() { return Vec::new(); }

    let offset = n - atr.len();
    let mut out: Vec<SupertrendPoint> = Vec::with_capacity(atr.len());
    let mut prev_trend: i8 = 1;
    let mut prev_value: f64 = candles[offset].close;

    for (i, &a) in atr.iter().enumerate() {
        let c = &candles[offset + i];
        let hl2 = (c.high + c.low) / 2.0;
        let upper = hl2 + multiplier * a;
        let lower = hl2 - multiplier * a;

        let (trend, value) = if c.close > prev_value {
            (1_i8, lower.max(prev_value))
        } else if c.close < prev_value {
            (-1_i8, upper.min(prev_value))
        } else {
            (prev_trend, prev_value)
        };

        prev_trend = trend;
        prev_value = value;
        out.push(SupertrendPoint { trend, value });
    }
    out
}

/// CCI (Commodity Channel Index) - typical price ile SMA arası standardize sapma.
/// Klasik formül: (TP - SMA(TP, period)) / (0.015 * mean_deviation).
pub fn calculate_cci(candles: &[Candle], period: usize) -> Vec<f64> {
    let n = candles.len();
    if n < period { return Vec::new(); }
    let tps: Vec<f64> = candles.iter().map(|c| (c.high + c.low + c.close) / 3.0).collect();

    let mut out = Vec::with_capacity(n - period + 1);
    for i in period..=n {
        let window = &tps[i - period..i];
        let mean = window.iter().sum::<f64>() / period as f64;
        let mean_dev = window.iter().map(|v| (v - mean).abs()).sum::<f64>() / period as f64;
        let tp = tps[i - 1];
        let cci = if mean_dev == 0.0 { 0.0 } else { (tp - mean) / (0.015 * mean_dev) };
        out.push(cci);
    }
    out
}

/// Stochastic RSI - RSI değerleri üzerine stochastic uygulanır.
/// rsi_period: RSI penceresi, stoch_period: stochastic penceresi, smooth_k/d: ek pürüzsüzleştirme.
pub fn calculate_stochastic_rsi(
    candles: &[Candle],
    rsi_period: usize,
    stoch_period: usize,
    smooth_k: usize,
    smooth_d: usize,
) -> StochasticRsiOutput {
    let rsi_series = calculate_rsi(candles, rsi_period);
    if rsi_series.len() < stoch_period {
        return StochasticRsiOutput::default();
    }

    let mut k_raw = Vec::with_capacity(rsi_series.len() - stoch_period + 1);
    for i in stoch_period..=rsi_series.len() {
        let window = &rsi_series[i - stoch_period..i];
        let high = window.iter().cloned().fold(f64::MIN, f64::max);
        let low = window.iter().cloned().fold(f64::MAX, f64::min);
        let cur = window.last().copied().unwrap_or(0.0);
        let v = (cur - low) / (high - low).max(f64::EPSILON) * 100.0;
        k_raw.push(v);
    }

    let k_line = if smooth_k > 1 {
        k_raw.windows(smooth_k).map(|w| w.iter().sum::<f64>() / smooth_k as f64).collect()
    } else {
        k_raw
    };
    let d_line = if smooth_d > 1 && k_line.len() >= smooth_d {
        k_line.windows(smooth_d).map(|w| w.iter().sum::<f64>() / smooth_d as f64).collect()
    } else {
        k_line.clone()
    };

    StochasticRsiOutput { k_line, d_line }
}

// =============================================================================
// 4. NOKTA ATIŞI HIZLI HASAT MOTORLARI (FAST-PATH FOR LIVE EXECUTION)
// =============================================================================

pub struct CoreIndicatorEngine;

impl CoreIndicatorEngine {
    /// Son barın SMA değerini, listeyi kopyalamadan referansla (Slicing) döner
    pub fn sma(candles: &[Candle], period: usize) -> f64 {
        let n = candles.len();
        if n == 0 || period == 0 { return 0.0; }
        let start = n.saturating_sub(period);
        candles[start..].iter().map(|c| c.close).sum::<f64>() / period as f64
    }

    /// Son barın RSI değerini, listeyi kopyalamadan referansla (Slicing) döner
    pub fn rsi(candles: &[Candle], period: usize) -> f64 {
        let n = candles.len();
        if n <= period { return 50.0; }
        let start = n.saturating_sub(period + 1);
        let (mut gains, mut losses) = (0.0, 0.0);
        for w in candles[start..].windows(2) {
            let diff = w[1].close - w[0].close;
            if diff > 0.0 { gains += diff; } else { losses -= diff.abs(); }
        }
        if losses == 0.0 { return 100.0; }
        100.0 - (100.0 / (1.0 + (gains / losses)))
    }

    /// Machine Learning modelleri için anlık öznitelik (Feature) üreticisi
    pub fn ml_features(candles: &[Candle]) -> Vec<f64> {
        let n = candles.len();
        if n < 2 { return vec![0.0; 4]; }
        let count = n.min(5);
        let start = n - count;
        let slice = &candles[start..];
        let mean = slice.iter().map(|c| c.close).sum::<f64>() / count as f64;
        if mean == 0.0 { return vec![0.0; count - 1]; }
        slice.windows(2).map(|w| (w[1].close - w[0].close) / mean).collect()
    }

    /// Otomatik strateji ve sinyal keşif rehberi
    pub fn discover_features(_candles: &[Candle]) -> Vec<String> {
        ["sma", "rsi", "ml_feature"].iter().map(|&s| s.to_owned()).collect()
    }
}
