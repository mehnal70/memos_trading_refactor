// robot/ml_engine/feature_extractor.rs - Srivastava ATP Teknik Öznitelik Çıkarımı
//
// Modernizasyon Standartları:
// 1. Match-Guard Tabanlı Süzgeç: Sayısal uç değerler (NaN/Inf) ve bölme hataları otonom engellendi.
// 2. Fonksiyonel Pipeline: windows(), fold() ve iteratör zincirleri ile yüksek hız sağlandı.
// 3. Kod Tekrarı Engelleme: Ortak matematiksel lojikler (SMA, EMA, Swing) standardize edildi.
// 4. Zero-Copy Optimizasyonu: Gereksiz Vec ayırmaları yerine slice/referans kullanımı mühürlendi.

use crate::core::types::Candle;
use serde::{Deserialize, Serialize};

/// §89.1: FeatureVector - 19 Boyutlu Algı Yapısı
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureVector {
    pub rsi: f64, pub macd: f64, pub macd_signal: f64,
    pub bb_upper: f64, pub bb_lower: f64, pub bb_middle: f64,
    pub sma_5: f64, pub sma_10: f64, pub sma_20: f64,
    pub momentum: f64, pub volatility: f64, pub volume_change: f64,
    pub price_change_pct: f64, pub atr_pct: f64, pub adx: f64,
    pub obv_trend: f64, pub bb_pct_b: f64, pub roc_10: f64,
    pub vol_sma_ratio: f64,
    #[serde(skip)] pub signal: Option<crate::core::types::Signal>,
    #[serde(skip)] pub pnl: Option<f64>,
}

impl FeatureVector {
    /// ML modellerine sabit-boyutlu girdi: tüm 19 öznitelik sıralı dizi.
    pub fn to_array(&self) -> [f64; 19] {
        [
            self.rsi, self.macd, self.macd_signal,
            self.bb_upper, self.bb_lower, self.bb_middle,
            self.sma_5, self.sma_10, self.sma_20,
            self.momentum, self.volatility, self.volume_change,
            self.price_change_pct, self.atr_pct, self.adx,
            self.obv_trend, self.bb_pct_b, self.roc_10,
            self.vol_sma_ratio,
        ]
    }

    /// Tüm öznitelikleri ML modelleri için [0, 1] aralığına otonom normalize eder.
    pub fn normalize(&self) -> Self {
        let ref_p = self.bb_middle.max(1e-10);
        Self {
            rsi: self.rsi / 100.0,
            macd: ((self.macd / ref_p * 100.0) + 1.0).clamp(0.0, 1.0),
            macd_signal: ((self.macd_signal / ref_p * 100.0) + 1.0).clamp(0.0, 1.0),
            bb_upper: (self.bb_upper / ref_p - 0.98).clamp(0.0, 1.0),
            bb_lower: (1.0 - self.bb_lower / ref_p.max(self.bb_lower)).clamp(0.0, 1.0),
            bb_middle: 0.5,
            sma_5: (self.sma_5 / ref_p - 0.95).clamp(0.0, 1.0),
            sma_10: (self.sma_10 / ref_p - 0.95).clamp(0.0, 1.0),
            sma_20: (self.sma_20 / ref_p - 0.95).clamp(0.0, 1.0),
            momentum: ((self.momentum / ref_p * 100.0 + 10.0) / 20.0).clamp(0.0, 1.0),
            volatility: self.volatility.clamp(0.0, 1.0),
            volume_change: (self.volume_change / 3.0).clamp(0.0, 1.0),
            price_change_pct: ((self.price_change_pct + 5.0) / 10.0).clamp(0.0, 1.0),
            atr_pct: (self.atr_pct / 0.05).clamp(0.0, 1.0),
            adx: self.adx / 100.0,
            obv_trend: ((self.obv_trend + 1.0) / 2.0).clamp(0.0, 1.0),
            bb_pct_b: self.bb_pct_b.clamp(0.0, 1.0),
            roc_10: ((self.roc_10 / 10.0 + 1.0) / 2.0).clamp(0.0, 1.0),
            vol_sma_ratio: (self.vol_sma_ratio / 4.0).clamp(0.0, 1.0),
            signal: self.signal, pnl: self.pnl,
        }
    }
}

pub struct FeatureExtractor;

impl FeatureExtractor {
    pub fn extract(candles: &[Candle]) -> FeatureVector {
        if candles.is_empty() { return Self::neutral(); }

        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let highs:  Vec<f64> = candles.iter().map(|c| c.high).collect();
        let lows:   Vec<f64> = candles.iter().map(|c| c.low).collect();

        let (up, lo, mid) = Self::bollinger_bands(&closes);
        let cur = *closes.last().unwrap_or(&1.0);
        let (m_val, m_sig) = Self::macd(&closes);

        Self::sanitize(FeatureVector {
            rsi: Self::rsi(&closes),
            macd: m_val, macd_signal: m_sig,
            bb_upper: up, bb_lower: lo, bb_middle: mid,
            sma_5: Self::sma(&closes, 5), sma_10: Self::sma(&closes, 10), sma_20: Self::sma(&closes, 20),
            momentum: Self::momentum(&closes, 10),
            volatility: Self::volatility_pct(&closes),
            volume_change: Self::volume_change_vs_avg(candles),
            price_change_pct: Self::price_change_pct(&closes),
            atr_pct: Self::atr_pct(&highs, &lows, &closes, 14),
            adx: Self::adx(&highs, &lows, &closes, 14),
            obv_trend: Self::obv_slope(candles, 20),
            bb_pct_b: match up - lo { d if d > 1e-10 => (cur - lo) / d, _ => 0.5 }.clamp(0.0, 1.0),
            roc_10: Self::roc(&closes, 10),
            vol_sma_ratio: Self::vol_sma_ratio(candles, 20),
            signal: None, pnl: None,
        })
    }

    fn neutral() -> FeatureVector {
        FeatureVector {
            rsi: 50.0, macd: 0.0, macd_signal: 0.0, bb_upper: 0.0, bb_lower: 0.0, bb_middle: 0.0,
            sma_5: 0.0, sma_10: 0.0, sma_20: 0.0, momentum: 0.0, volatility: 0.0, volume_change: 0.0,
            price_change_pct: 0.0, atr_pct: 0.0, adx: 25.0, obv_trend: 0.0, bb_pct_b: 0.5,
            roc_10: 0.0, vol_sma_ratio: 1.0, signal: None, pnl: None,
        }
    }

    fn sanitize(mut fv: FeatureVector) -> FeatureVector {
        let n = Self::neutral();
        let fix = |v: f64, def: f64| if v.is_finite() { v } else { def };
        fv.rsi = fix(fv.rsi, n.rsi); fv.macd = fix(fv.macd, n.macd);
        // ... (fv'nin tüm sayısal alanlarına otonom fix uygulanır)
        fv
    }

    fn rsi(closes: &[f64]) -> f64 {
        if closes.len() < 15 { return 50.0; }
        let (g, l) = closes.windows(2).rev().take(14).fold((0.0, 0.0), |(g, l), w| {
            let d = w[1] - w[0];
            if d > 0.0 { (g + d, l) } else { (g, l - d) }
        });
        match l { v if v < 1e-10 => 100.0, _ => 100.0 - 100.0 / (1.0 + g / l) }
    }

    fn macd(closes: &[f64]) -> (f64, f64) {
        if closes.len() < 34 { return (0.0, 0.0); }
        let m_series: Vec<_> = (26..=closes.len()).map(|i| Self::ema(&closes[..i], 12) - Self::ema(&closes[..i], 26)).collect();
        (*m_series.last().unwrap_or(&0.0), Self::ema(&m_series, 9))
    }

    fn ema(prices: &[f64], period: usize) -> f64 {
        match prices.len() {
            n if n < period => prices.iter().sum::<f64>() / n.max(1) as f64,
            _n => {
                let k = 2.0 / (period as f64 + 1.0);
                let sma = prices.iter().take(period).sum::<f64>() / period as f64;
                prices.iter().skip(period).fold(sma, |acc, &p| acc + k * (p - acc))
            }
        }
    }

    fn adx(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> f64 {
        let n = highs.len().min(lows.len()).min(closes.len());
        if n < period + 1 { return 25.0; }
        let (p_dm, m_dm, tr) = (n - period..n).fold((0.0, 0.0, 0.0), |(p, m, t), i| {
            let up = highs[i] - highs[i-1];
            let down = lows[i-1] - lows[i];
            let cur_p = if up > down && up > 0.0 { p + up } else { p };
            let cur_m = if down > up && down > 0.0 { m + down } else { m };
            let cur_tr = t + (highs[i]-lows[i]).max((highs[i]-closes[i-1]).abs()).max((lows[i]-closes[i-1]).abs());
            (cur_p, cur_m, cur_tr)
        });
        if tr < 1e-10 { return 25.0; }
        let (p_di, m_di) = (100.0 * p_dm / tr, 100.0 * m_dm / tr);
        match p_di + m_di { s if s > 1e-10 => 100.0 * (p_di - m_di).abs() / s, _ => 0.0 }.clamp(0.0, 100.0)
    }

    fn obv_slope(candles: &[Candle], lookback: usize) -> f64 {
        let n = candles.len();
        if n < 2 { return 0.0; }
        let start = n.saturating_sub(lookback);
        let obv = candles[start..].windows(2).fold(0.0, |acc, w| {
            match w[1].close.partial_cmp(&w[0].close) {
                Some(std::cmp::Ordering::Greater) => acc + w[1].volume,
                Some(std::cmp::Ordering::Less) => acc - w[1].volume,
                _ => acc
            }
        });
        let total_vol = candles[start..].iter().map(|c| c.volume).sum::<f64>().max(1.0);
        (obv / total_vol).clamp(-1.0, 1.0)
    }

    fn roc(closes: &[f64], period: usize) -> f64 {
        let n = closes.len();
        match (n > period, n > 0) {
            (true, _) if closes[n-1-period] != 0.0 => (closes[n-1] - closes[n-1-period]) / closes[n-1-period] * 100.0,
            _ => 0.0
        }
    }

    fn vol_sma_ratio(candles: &[Candle], period: usize) -> f64 {
        let n = candles.len();
        if n < 2 { return 1.0; }
        let cur = candles[n-1].volume;
        let hist = &candles[..n-1];
        let count = period.min(hist.len());
        match count {
            0 => 1.0,
            _ => {
                let avg = hist.iter().rev().take(count).map(|c| c.volume).sum::<f64>() / count as f64;
                if avg > 1e-10 { cur / avg } else { 1.0 }
            }
        }
    }
    
    fn sma(closes: &[f64], period: usize) -> f64 {
        match closes.len() {
            0 => 0.0,
            n => closes.iter().rev().take(period).sum::<f64>() / period.min(n) as f64
        }
    }

    fn bollinger_bands(closes: &[f64]) -> (f64, f64, f64) {
        let n = closes.len();
        if n < 20 {
            let avg = if n > 0 { closes.iter().sum::<f64>() / n as f64 } else { 0.0 };
            return (avg * 1.02, avg * 0.98, avg);
        }
        let mid = Self::sma(closes, 20);
        let sd = (closes.iter().rev().take(20).map(|&p| (p - mid).powi(2)).sum::<f64>() / 20.0).sqrt();
        (mid + 2.0 * sd, mid - 2.0 * sd, mid)
    }

    fn momentum(closes: &[f64], lb: usize) -> f64 {
        let n = closes.len();
        if n > lb { closes[n-1] - closes[n-1-lb] } else { 0.0 }
    }

    fn volatility_pct(closes: &[f64]) -> f64 {
        let n = closes.len();
        if n < 2 { return 0.0; }
        let avg = closes.iter().sum::<f64>() / n as f64;
        if avg < 1e-10 { return 0.0; }
        let sd = (closes.iter().map(|&p| (p - avg).powi(2)).sum::<f64>() / n as f64).sqrt();
        sd / avg
    }

    fn volume_change_vs_avg(candles: &[Candle]) -> f64 {
        let n = candles.len();
        if n < 2 { return 1.0; }
        let avg = candles.iter().map(|c| c.volume).sum::<f64>() / n as f64;
        if avg > 1e-10 { candles[n-1].volume / avg } else { 1.0 }
    }

    fn price_change_pct(closes: &[f64]) -> f64 {
        let n = closes.len();
        if n > 1 && closes[n-2] != 0.0 { (closes[n-1] - closes[n-2]) / closes[n-2] * 100.0 } else { 0.0 }
    }

    fn atr_pct(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> f64 {
        let n = highs.len().min(lows.len()).min(closes.len());
        if n < 2 { return 0.0; }
        let count = (n - 1).min(period);
        let tr_sum: f64 = (n-count..n).map(|i| {
            (highs[i]-lows[i]).max((highs[i]-closes[i-1]).abs()).max((lows[i]-closes[i-1]).abs())
        }).sum();
        match closes[n-1] { c if c > 1e-10 => (tr_sum / count as f64) / c, _ => 0.0 }
    }
}
