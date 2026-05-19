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

/// ATR Serisi — **Wilder Smoothed Moving Average (SMMA)** ile.
/// TradingView/standart formül: ATR_0 = mean(TR[0..N]); ATR_i = (ATR_(i-1)*(N-1) + TR_i) / N.
/// Çıktı uzunluğu candles.len() - period (her bar bir önceki kapanışa bağlı olduğundan TR=candles-1,
/// SMMA seed periodun ilk N TR'sini yer; sonuç =`TR.len() - N + 1`).
pub fn calculate_atr(candles: &[Candle], period: usize) -> Vec<f64> {
    if candles.len() < period + 1 || period == 0 { return Vec::new(); }

    // 1) True Range serisi (bar #0 hariç; ilk TR bar #1'in TR'si)
    let mut trs = Vec::with_capacity(candles.len() - 1);
    for i in 1..candles.len() {
        trs.push(calculate_true_range(candles[i].high, candles[i].low, candles[i - 1].close));
    }

    // 2) Wilder SMMA seed = ilk N TR'nin aritmetik ortalaması
    let mut atr = trs.iter().take(period).sum::<f64>() / period as f64;
    let mut out = Vec::with_capacity(trs.len() - period + 1);
    out.push(atr);

    // 3) Wilder smoothing: ATR_i = (ATR_(i-1) * (N-1) + TR_i) / N
    let n_minus_1 = (period - 1) as f64;
    let n = period as f64;
    for &tr in trs.iter().skip(period) {
        atr = (atr * n_minus_1 + tr) / n;
        out.push(atr);
    }
    out
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

/// Parabolic SAR Serisi — Wilder'ın klasik algoritması.
///
/// Persistent trend rejimi (uptrend/downtrend). Her bar:
/// 1. SAR_i = SAR_(i-1) + AF * (EP - SAR_(i-1))
/// 2. EP yeni extreme'e ulaşırsa AF arttırılır (`+step`, `max` ile sınır).
/// 3. Uptrend: SAR son 2 barın low'larını geçemez (clamp); aksi halde anlık reversal.
/// 4. Downtrend: SAR son 2 barın high'larını altına inemez (clamp).
/// 5. Trend reversal: uptrend'de SAR > current low → trend ⇒ downtrend, SAR ← EP,
///    yeni EP ← current low, AF ← step. Downtrend'de simetrik.
///
/// `step` tipik 0.02, `max` 0.20.
pub fn calculate_parabolic_sar(candles: &[Candle], step: f64, max: f64) -> Vec<f64> {
    let n = candles.len();
    if n < 2 { return Vec::new(); }

    let mut out = Vec::with_capacity(n);

    // İlk barın trend yönünü ikinci bar ile karşılaştırarak seç (sıradan başlangıç heuristiği).
    let mut uptrend = candles[1].close >= candles[0].close;
    let (mut sar, mut ep) = if uptrend {
        (candles[0].low, candles[0].high)
    } else {
        (candles[0].high, candles[0].low)
    };
    let mut af = step;
    out.push(sar);

    for i in 1..n {
        // 1) Tentatif yeni SAR
        let mut new_sar = sar + af * (ep - sar);

        // 3-4) Son 2 barın aşırı uçlarını geçmesini engelle (clamp)
        if uptrend {
            let cap = if i >= 2 { candles[i - 1].low.min(candles[i - 2].low) } else { candles[i - 1].low };
            new_sar = new_sar.min(cap);
        } else {
            let cap = if i >= 2 { candles[i - 1].high.max(candles[i - 2].high) } else { candles[i - 1].high };
            new_sar = new_sar.max(cap);
        }

        // 5) Trend reversal kontrolü
        let reverse = if uptrend { candles[i].low <= new_sar } else { candles[i].high >= new_sar };
        if reverse {
            uptrend = !uptrend;
            new_sar = ep;
            ep = if uptrend { candles[i].high } else { candles[i].low };
            af = step;
        } else {
            // 2) EP güncellemesi + AF arttırımı (sadece yeni extreme görüldüğünde)
            if uptrend && candles[i].high > ep {
                ep = candles[i].high;
                af = (af + step).min(max);
            } else if !uptrend && candles[i].low < ep {
                ep = candles[i].low;
                af = (af + step).min(max);
            }
        }

        sar = new_sar;
        out.push(sar);
    }
    out
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

/// ADX Serisi — Klasik Wilder algoritması (TradingView uyumlu).
///
/// 1. Her bar için TR, +DM, -DM hesaplanır.
/// 2. Wilder SMMA(TR, period) = ATR; aynı şekilde +DM ve -DM smooth edilir.
/// 3. +DI = 100 * +DM_smooth / ATR;  -DI = 100 * -DM_smooth / ATR.
/// 4. DX = 100 * |+DI - -DI| / (+DI + -DI).
/// 5. **ADX = Wilder SMMA(DX, period)** — son adımdaki bu smoothing,
///    önceki sürümde eksikti (DX serisi ADX adıyla dönüyordu).
///
/// Çıktı uzunluğu yaklaşık `candles.len() - 2*period`; başlangıçta hem DI
/// hem ADX seed'i için 2 ardışık SMMA penceresi tüketilir.
pub fn calculate_adx(candles: &[Candle], period: usize) -> Vec<f64> {
    let n = candles.len();
    if n < 2 * period + 1 || period == 0 { return Vec::new(); }

    // 1) TR, +DM, -DM serileri (bar #0 hariç)
    let len = n - 1;
    let mut tr  = Vec::with_capacity(len);
    let mut p_dm = Vec::with_capacity(len);
    let mut m_dm = Vec::with_capacity(len);
    for i in 1..n {
        tr.push(calculate_true_range(candles[i].high, candles[i].low, candles[i - 1].close));
        let up   = candles[i].high - candles[i - 1].high;
        let down = candles[i - 1].low - candles[i].low;
        p_dm.push(if up > down && up > 0.0 { up } else { 0.0 });
        m_dm.push(if down > up && down > 0.0 { down } else { 0.0 });
    }

    // 2) Wilder SMMA seed = ilk period elemanın ortalaması
    let seed = |v: &[f64]| v.iter().take(period).sum::<f64>() / period as f64;
    let mut s_tr  = seed(&tr);
    let mut s_pdm = seed(&p_dm);
    let mut s_mdm = seed(&m_dm);

    // 3) Tüm bar'lar için smooth + DX hesabı
    let n_minus_1 = (period - 1) as f64;
    let n_p = period as f64;
    let mut dx_series = Vec::with_capacity(len - period + 1);

    // İlk seed barına karşılık gelen DX
    let push_dx = |s_pdm: f64, s_mdm: f64, s_tr: f64, out: &mut Vec<f64>| {
        let p_di = if s_tr > f64::EPSILON { 100.0 * s_pdm / s_tr } else { 0.0 };
        let m_di = if s_tr > f64::EPSILON { 100.0 * s_mdm / s_tr } else { 0.0 };
        let denom = (p_di + m_di).max(f64::EPSILON);
        out.push(100.0 * (p_di - m_di).abs() / denom);
    };
    push_dx(s_pdm, s_mdm, s_tr, &mut dx_series);

    // Sonraki barlar için Wilder smoothing devam
    for i in period..len {
        s_tr  = (s_tr  * n_minus_1 + tr[i])   / n_p;
        s_pdm = (s_pdm * n_minus_1 + p_dm[i]) / n_p;
        s_mdm = (s_mdm * n_minus_1 + m_dm[i]) / n_p;
        push_dx(s_pdm, s_mdm, s_tr, &mut dx_series);
    }

    // 4) ADX = Wilder SMMA(DX, period) — son adım.
    if dx_series.len() < period { return Vec::new(); }
    let mut adx = dx_series.iter().take(period).sum::<f64>() / period as f64;
    let mut adx_series = Vec::with_capacity(dx_series.len() - period + 1);
    adx_series.push(adx);
    for &dx in dx_series.iter().skip(period) {
        adx = (adx * n_minus_1 + dx) / n_p;
        adx_series.push(adx);
    }
    adx_series
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

/// Supertrend — ATR bazlı dinamik trend takip indikatörü (klasik path-dependent algoritma).
///
/// Her bar için iki temel bant:
///   basic_upper = hl2 + multiplier * ATR
///   basic_lower = hl2 - multiplier * ATR
///
/// Final bantlar path-dependent şekilde "carry over" yapar (yalnızca trend güçlenirse
/// daralır, yoksa önceki seviyeyi korur):
///   final_upper_i = if basic_upper_i < final_upper_(i-1) OR close_(i-1) > final_upper_(i-1)
///                     then basic_upper_i else final_upper_(i-1)
///   final_lower_i = if basic_lower_i > final_lower_(i-1) OR close_(i-1) < final_lower_(i-1)
///                     then basic_lower_i else final_lower_(i-1)
///
/// Supertrend trend yön kararı: önceki barın supertrend bandı (upper mu lower mu) ve
/// mevcut close'a göre flip eder. Çıktı: her bar için (trend: +1 yukarı / -1 aşağı, value: aktif bant).
pub fn calculate_supertrend(candles: &[Candle], period: usize, multiplier: f64) -> Vec<SupertrendPoint> {
    let n = candles.len();
    if n < period + 1 { return Vec::new(); }

    let atr = calculate_atr(candles, period);
    if atr.is_empty() { return Vec::new(); }

    // ATR `period+1` barda başlar (TR uzunluğu n-1, smooth seed period eler).
    let offset = n - atr.len();
    let mut out: Vec<SupertrendPoint> = Vec::with_capacity(atr.len());

    // İlk barda final bantları basic bantlara eşitle; trend close-vs-hl2 ile başlangıç.
    let c0 = &candles[offset];
    let hl2_0 = (c0.high + c0.low) / 2.0;
    let mut final_upper = hl2_0 + multiplier * atr[0];
    let mut final_lower = hl2_0 - multiplier * atr[0];
    let mut prev_trend: i8 = if c0.close >= hl2_0 { 1 } else { -1 };
    let mut prev_super = if prev_trend == 1 { final_lower } else { final_upper };
    out.push(SupertrendPoint { trend: prev_trend, value: prev_super });

    for k in 1..atr.len() {
        let c     = &candles[offset + k];
        let c_prev = &candles[offset + k - 1];
        let hl2 = (c.high + c.low) / 2.0;
        let basic_upper = hl2 + multiplier * atr[k];
        let basic_lower = hl2 - multiplier * atr[k];

        // Final bant carry-over: yalnızca daralma yönünde güncelle.
        final_upper = if basic_upper < final_upper || c_prev.close > final_upper {
            basic_upper
        } else { final_upper };
        final_lower = if basic_lower > final_lower || c_prev.close < final_lower {
            basic_lower
        } else { final_lower };

        // Trend kararı: önceki bandın hangi taraf olduğuna ve close'un ona göre konumuna bağlı.
        let (trend, value) = if prev_trend == 1 {
            // Önceden uptrend (alt bant aktif); close alt bandın altına düşerse downtrend'e flip.
            if c.close < final_lower { (-1_i8, final_upper) }
            else                     { (1_i8,  final_lower) }
        } else {
            // Önceden downtrend (üst bant aktif); close üst bandın üstüne çıkarsa uptrend'e flip.
            if c.close > final_upper { (1_i8,  final_lower) }
            else                     { (-1_i8, final_upper) }
        };

        prev_trend = trend;
        prev_super = value;
        out.push(SupertrendPoint { trend, value: prev_super });
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

    /// Son barın RSI değerini Wilder SMMA ile döner — `calculate_rsi(...).last()` ile
    /// **bit-bit aynı** sonucu üretir (series/fast-path tutarlılığı şart).
    ///
    /// Eski sürüm sadece son `period+1` bar üzerinden düz toplama yapıyordu; bu hem
    /// Wilder smoothing'i atlıyordu hem de series-path ile farklı değer veriyordu.
    pub fn rsi(candles: &[Candle], period: usize) -> f64 {
        let n = candles.len();
        if n <= period || period == 0 { return 50.0; }

        // 1) İlk `period` bar için Wilder SMMA seed (gain/loss aritmetik ortalaması).
        let mut gains = 0.0;
        let mut losses = 0.0;
        for i in 1..=period {
            let diff = candles[i].close - candles[i - 1].close;
            if diff > 0.0 { gains += diff; } else { losses -= diff; }
        }
        let mut avg_gain = gains / period as f64;
        let mut avg_loss = losses / period as f64;

        // 2) Sonraki barlar için Wilder smoothing:
        //    avg = (prev_avg * (period - 1) + new) / period
        let n_minus_1 = (period - 1) as f64;
        let n_p = period as f64;
        for i in (period + 1)..n {
            let diff = candles[i].close - candles[i - 1].close;
            let (g, l) = if diff > 0.0 { (diff, 0.0) } else { (0.0, -diff) };
            avg_gain = (avg_gain * n_minus_1 + g) / n_p;
            avg_loss = (avg_loss * n_minus_1 + l) / n_p;
        }

        if avg_loss == 0.0 { return 100.0; }
        let rs = avg_gain / avg_loss;
        100.0 - 100.0 / (1.0 + rs)
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
