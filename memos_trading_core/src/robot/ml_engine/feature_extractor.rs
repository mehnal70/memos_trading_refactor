use crate::types::Candle;
use serde::{Deserialize, Serialize};

/// Teknik göstergeler — 19 öznitelik (13 orijinal + 6 yeni)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureVector {
    // ── Orijinal 13 ─────────────────────────────────────────────────────────
    pub rsi: f64,              // Relative Strength Index (0-100)
    pub macd: f64,             // MACD (12-26)
    pub macd_signal: f64,      // MACD Signal Line
    pub bb_upper: f64,         // Bollinger Bands Upper
    pub bb_lower: f64,         // Bollinger Bands Lower
    pub bb_middle: f64,        // Bollinger Bands Middle (SMA20)
    pub sma_5: f64,            // 5-period SMA
    pub sma_10: f64,           // 10-period SMA
    pub sma_20: f64,           // 20-period SMA
    pub momentum: f64,         // Price momentum (current - past 10)
    pub volatility: f64,       // Std dev / mean
    pub volume_change: f64,    // current_vol / history_avg_vol
    pub price_change_pct: f64, // 1-bar % change
    // ── Yeni 6 ──────────────────────────────────────────────────────────────
    pub atr_pct: f64,          // ATR / close  (gerçek aralık ortalaması, % olarak)
    pub adx: f64,              // Average Directional Index — trend gücü (0-100)
    pub obv_trend: f64,        // OBV normalised slope [-1, +1]
    pub bb_pct_b: f64,         // Bollinger %B  0=alt bant, 1=üst bant
    pub roc_10: f64,           // Rate of Change 10-bar (%)
    pub vol_sma_ratio: f64,    // current_vol / sma_vol_20

    #[serde(skip)]
    pub signal: Option<crate::types::Signal>,
    #[serde(skip)]
    pub pnl: Option<f64>,
}

impl FeatureVector {
    /// Tüm öznitelikleri [0, 1] aralığına normalize et
    pub fn normalize(&self) -> Self {
        let ref_price = self.bb_middle.max(1.0);
        Self {
            // ── Orijinal ──────────────────────────────────────────────────
            rsi:              self.rsi / 100.0,
            macd:             ((self.macd / ref_price * 100.0) + 1.0).clamp(0.0, 1.0),
            macd_signal:      ((self.macd_signal / ref_price * 100.0) + 1.0).clamp(0.0, 1.0),
            bb_upper:         (self.bb_upper / ref_price - 0.98).clamp(0.0, 1.0),
            bb_lower:         (1.0 - self.bb_lower / ref_price.max(self.bb_lower)).clamp(0.0, 1.0),
            bb_middle:        0.5,
            sma_5:            (self.sma_5  / ref_price - 0.95).clamp(0.0, 1.0),
            sma_10:           (self.sma_10 / ref_price - 0.95).clamp(0.0, 1.0),
            sma_20:           (self.sma_20 / ref_price - 0.95).clamp(0.0, 1.0),
            momentum:         ((self.momentum / ref_price * 100.0 + 10.0) / 20.0).clamp(0.0, 1.0),
            volatility:       self.volatility.clamp(0.0, 1.0),
            volume_change:    (self.volume_change / 3.0).clamp(0.0, 1.0),
            price_change_pct: ((self.price_change_pct + 5.0) / 10.0).clamp(0.0, 1.0),
            // ── Yeni ──────────────────────────────────────────────────────
            atr_pct:          (self.atr_pct / 0.05).clamp(0.0, 1.0),
            adx:              self.adx / 100.0,
            obv_trend:        ((self.obv_trend + 1.0) / 2.0).clamp(0.0, 1.0),
            bb_pct_b:         self.bb_pct_b.clamp(0.0, 1.0),
            roc_10:           ((self.roc_10 / 10.0 + 1.0) / 2.0).clamp(0.0, 1.0),
            vol_sma_ratio:    (self.vol_sma_ratio / 4.0).clamp(0.0, 1.0),
            signal:           self.signal.clone(),
            pnl:              self.pnl,
        }
    }

    /// 19 elemanlı dizi — normalize() çağrısından sonra kullanılmalı
    pub fn to_array(&self) -> [f64; 19] {
        [
            self.rsi, self.macd, self.macd_signal,
            self.bb_upper, self.bb_lower, self.bb_middle,
            self.sma_5, self.sma_10, self.sma_20,
            self.momentum, self.volatility, self.volume_change, self.price_change_pct,
            self.atr_pct, self.adx, self.obv_trend, self.bb_pct_b, self.roc_10, self.vol_sma_ratio,
        ]
    }
}

/// Teknik göstergeleri mum verilerinden çıkarır
pub struct FeatureExtractor;

impl FeatureExtractor {
    pub fn extract(candles: &[Candle]) -> FeatureVector {
        if candles.is_empty() { return Self::neutral(); }

        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let highs:  Vec<f64> = candles.iter().map(|c| c.high).collect();
        let lows:   Vec<f64> = candles.iter().map(|c| c.low).collect();

        let (bb_upper, bb_lower, bb_middle) = Self::bollinger_bands(&closes);
        let last_close = *closes.last().unwrap_or(&1.0);
        let bb_pct_b = if bb_upper > bb_lower {
            ((last_close - bb_lower) / (bb_upper - bb_lower)).clamp(0.0, 1.0)
        } else { 0.5 };

        let (macd_v, macd_sig) = Self::macd(&closes); // bir kez hesapla
        Self::sanitize(FeatureVector {
            rsi:              Self::rsi(&closes),
            macd:             macd_v,
            macd_signal:      macd_sig,
            bb_upper, bb_lower, bb_middle,
            sma_5:            Self::sma(&closes, 5),
            sma_10:           Self::sma(&closes, 10),
            sma_20:           Self::sma(&closes, 20),
            momentum:         Self::momentum(&closes, 10),
            volatility:       Self::volatility_pct(&closes),
            volume_change:    Self::volume_change_vs_avg(candles),
            price_change_pct: Self::price_change_pct(&closes),
            atr_pct:          Self::atr_pct(&highs, &lows, &closes, 14),
            adx:              Self::adx(&highs, &lows, &closes, 14),
            obv_trend:        Self::obv_slope(candles, 20),
            bb_pct_b,
            roc_10:           Self::roc(&closes, 10),
            vol_sma_ratio:    Self::vol_sma_ratio(candles, 20),
            signal: None, pnl: None,
        })
    }

    fn neutral() -> FeatureVector {
        FeatureVector {
            rsi: 50.0, macd: 0.0, macd_signal: 0.0,
            bb_upper: 0.0, bb_lower: 0.0, bb_middle: 0.0,
            sma_5: 0.0, sma_10: 0.0, sma_20: 0.0,
            momentum: 0.0, volatility: 0.0, volume_change: 0.0, price_change_pct: 0.0,
            atr_pct: 0.0, adx: 25.0, obv_trend: 0.0, bb_pct_b: 0.5,
            roc_10: 0.0, vol_sma_ratio: 1.0, signal: None, pnl: None,
        }
    }

    /// NaN veya Inf içeren alanları nötr değerlerine sıfırlar.
    /// Hesaplama zincirinin herhangi bir noktasında oluşabilecek sayısal anomalilere karşı son bariyer.
    fn sanitize(mut fv: FeatureVector) -> FeatureVector {
        let n = Self::neutral();
        macro_rules! fix {
            ($field:ident, $neutral:expr) => {
                if !fv.$field.is_finite() { fv.$field = $neutral; }
            };
        }
        fix!(rsi, n.rsi); fix!(macd, n.macd); fix!(macd_signal, n.macd_signal);
        fix!(bb_upper, n.bb_upper); fix!(bb_lower, n.bb_lower); fix!(bb_middle, n.bb_middle);
        fix!(sma_5, n.sma_5); fix!(sma_10, n.sma_10); fix!(sma_20, n.sma_20);
        fix!(momentum, n.momentum); fix!(volatility, n.volatility);
        fix!(volume_change, n.volume_change); fix!(price_change_pct, n.price_change_pct);
        fix!(atr_pct, n.atr_pct); fix!(adx, n.adx); fix!(obv_trend, n.obv_trend);
        fix!(bb_pct_b, n.bb_pct_b); fix!(roc_10, n.roc_10); fix!(vol_sma_ratio, n.vol_sma_ratio);
        fv
    }

    fn rsi(closes: &[f64]) -> f64 {
        if closes.len() < 15 { return 50.0; }
        // Son 15 kapanış → 14 fark (14-periyot RSI)
        let start = closes.len() - 15;
        let (mut g, mut l) = (0.0f64, 0.0f64);
        for i in (start + 1)..closes.len() {
            let d = closes[i] - closes[i - 1];
            if d > 0.0 { g += d; } else { l -= d; }
        }
        let al = l / 14.0;
        if al == 0.0 { return 100.0; }
        100.0 - 100.0 / (1.0 + g / 14.0 / al)
    }

    fn macd(closes: &[f64]) -> (f64, f64) {
        // En az 26 (slow) + 9 (signal) - 1 = 34 bar gerekli
        if closes.len() < 34 { return (0.0, 0.0); }
        // Her bar için MACD değeri hesaplayıp seriyi oluştur
        let macd_series: Vec<f64> = (26..=closes.len())
            .map(|i| {
                let slice = &closes[..i];
                Self::ema(slice, 12) - Self::ema(slice, 26)
            })
            .collect();
        let macd_val    = *macd_series.last().unwrap_or(&0.0);
        // Signal line = MACD serisinin 9-periyot EMA'sı
        let signal_val  = Self::ema(&macd_series, 9);
        (macd_val, signal_val)
    }

    fn ema(prices: &[f64], period: usize) -> f64 {
        if prices.len() < period {
            return prices.iter().sum::<f64>() / prices.len().max(1) as f64;
        }
        let k   = 2.0 / (period as f64 + 1.0);
        let sma = prices.iter().take(period).sum::<f64>() / period as f64;
        prices.iter().skip(period).fold(sma, |e, &p| e + k * (p - e))
    }

    fn bollinger_bands(closes: &[f64]) -> (f64, f64, f64) {
        if closes.len() < 20 {
            let avg = closes.iter().sum::<f64>() / closes.len().max(1) as f64;
            return (avg * 1.02, avg * 0.98, avg);
        }
        let mid = Self::sma(closes, 20);
        let var = closes.iter().rev().take(20)
            .map(|p| (p - mid).powi(2)).sum::<f64>() / 20.0;
        let sd  = var.sqrt();
        (mid + 2.0 * sd, mid - 2.0 * sd, mid)
    }

    fn sma(closes: &[f64], period: usize) -> f64 {
        if closes.is_empty() { return 0.0; }
        closes.iter().rev().take(period).sum::<f64>() / period.min(closes.len()) as f64
    }

    fn momentum(closes: &[f64], lookback: usize) -> f64 {
        if closes.len() < lookback + 1 { return 0.0; }
        closes[closes.len() - 1] - closes[closes.len() - 1 - lookback]
    }

    fn volatility_pct(closes: &[f64]) -> f64 {
        if closes.len() < 2 { return 0.0; }
        let avg = closes.iter().sum::<f64>() / closes.len() as f64;
        if avg == 0.0 { return 0.0; }
        let var = closes.iter().map(|p| (p - avg).powi(2)).sum::<f64>() / closes.len() as f64;
        var.sqrt() / avg
    }

    fn volume_change_vs_avg(candles: &[Candle]) -> f64 {
        if candles.len() < 2 { return 1.0; }
        let cur = candles.last().unwrap().volume;
        let avg = candles.iter().map(|c| c.volume).sum::<f64>() / candles.len() as f64;
        if avg == 0.0 { return 1.0; }
        cur / avg
    }

    fn price_change_pct(closes: &[f64]) -> f64 {
        if closes.len() < 2 { return 0.0; }
        let prev = closes[closes.len() - 2];
        if prev == 0.0 { return 0.0; }
        (closes[closes.len() - 1] - prev) / prev * 100.0
    }

    /// ATR / close — gerçek volatilite ölçüsü
    fn atr_pct(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> f64 {
        let n = highs.len().min(lows.len()).min(closes.len());
        if n < 2 { return 0.0; }
        let last_close = closes[n - 1];
        if last_close == 0.0 { return 0.0; }
        let start = if n > period + 1 { n - period - 1 } else { 0 };
        let trs: Vec<f64> = (start + 1..n).map(|i| {
            let hl = highs[i] - lows[i];
            let hc = (highs[i] - closes[i - 1]).abs();
            let lc = (lows[i]  - closes[i - 1]).abs();
            hl.max(hc).max(lc)
        }).collect();
        if trs.is_empty() { return 0.0; }
        trs.iter().sum::<f64>() / trs.len() as f64 / last_close
    }

    /// Basitleştirilmiş ADX — trend gücü (0-100)
    fn adx(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> f64 {
        let n = highs.len().min(lows.len()).min(closes.len());
        if n < period + 1 { return 25.0; }
        let start = n.saturating_sub(period + 1);
        let (mut plus_dm, mut minus_dm, mut tr_sum) = (0.0f64, 0.0f64, 0.0f64);
        for i in (start + 1)..n {
            let up   = highs[i] - highs[i - 1];
            let down = lows[i - 1] - lows[i];
            if up > down && up > 0.0   { plus_dm  += up; }
            if down > up && down > 0.0 { minus_dm += down; }
            let hl = highs[i] - lows[i];
            let hc = (highs[i] - closes[i - 1]).abs();
            let lc = (lows[i]  - closes[i - 1]).abs();
            tr_sum += hl.max(hc).max(lc);
        }
        if tr_sum == 0.0 { return 25.0; }
        let plus_di  = 100.0 * plus_dm  / tr_sum;
        let minus_di = 100.0 * minus_dm / tr_sum;
        let di_sum   = plus_di + minus_di;
        if di_sum == 0.0 { return 0.0; }
        (100.0 * (plus_di - minus_di).abs() / di_sum).clamp(0.0, 100.0)
    }

    /// OBV normalised slope [-1, +1]
    /// +1 = hacim trendi yükselişi destekliyor, -1 = düşüşü destekliyor
    fn obv_slope(candles: &[Candle], lookback: usize) -> f64 {
        let n = candles.len();
        if n < 2 { return 0.0; }
        let start = if n > lookback { n - lookback } else { 0 };
        // OBV pencere başında 0'dan başlar; tüm kümülatif değişim ölçülür.
        let mut obv = 0.0f64;
        for i in (start + 1)..n {
            let vol = candles[i].volume;
            if   candles[i].close > candles[i - 1].close { obv += vol; }
            else if candles[i].close < candles[i - 1].close { obv -= vol; }
        }
        let range = candles[start..n].iter().map(|c| c.volume).sum::<f64>().max(1.0);
        (obv / range).clamp(-1.0, 1.0)
    }

    /// Rate of Change N-bar (%)
    fn roc(closes: &[f64], period: usize) -> f64 {
        if closes.len() < period + 1 { return 0.0; }
        let past = closes[closes.len() - 1 - period];
        if past == 0.0 { return 0.0; }
        (closes[closes.len() - 1] - past) / past * 100.0
    }

    /// current_vol / N-period SMA vol (mevcut mum hariç)
    /// Paydaya mevcut mumu dahil etmek self-referans bias yaratır:
    /// hacim ne kadar yüksekse oran o kadar 1.0'a yaklaşır.
    fn vol_sma_ratio(candles: &[Candle], period: usize) -> f64 {
        if candles.len() < 2 { return 1.0; }
        let cur  = candles.last().unwrap().volume;
        // Mevcut mumu dışarıda bırak → son period kadar önceki mumların ortalaması
        let hist = &candles[..candles.len() - 1];
        let n    = period.min(hist.len());
        if n == 0 { return 1.0; }
        let avg  = hist.iter().rev().take(n).map(|c| c.volume).sum::<f64>() / n as f64;
        if avg == 0.0 { return 1.0; }
        cur / avg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn test_candles(n: usize) -> Vec<Candle> {
        let mut price = 100.0f64;
        (0..n).map(|i| {
            price += (i as f64 * 0.3) % 2.0 - 0.5;
            Candle {
                symbol: "BTC".to_string(), interval: "1h".to_string(),
                timestamp: Utc::now() + chrono::Duration::hours(i as i64),
                open: price, high: price + 2.0, low: price - 1.0, close: price + 0.5,
                volume: 1000.0 + i as f64 * 50.0,
            }
        }).collect()
    }

    #[test]
    fn test_extract_19_features() {
        let fv = FeatureExtractor::extract(&test_candles(40));
        assert_eq!(fv.to_array().len(), 19);
        assert!(fv.rsi >= 0.0 && fv.rsi <= 100.0);
        assert!(fv.adx >= 0.0 && fv.adx <= 100.0);
        assert!(fv.bb_pct_b >= 0.0 && fv.bb_pct_b <= 1.0);
        assert!(fv.obv_trend >= -1.0 && fv.obv_trend <= 1.0);
    }

    #[test]
    fn test_normalize_bounds() {
        let fv = FeatureExtractor::extract(&test_candles(40));
        let n  = fv.normalize();
        for v in n.to_array() {
            assert!(v >= 0.0 && v <= 1.0, "normalize dışı: {}", v);
        }
    }

    #[test]
    fn test_empty() {
        let fv = FeatureExtractor::extract(&[]);
        assert_eq!(fv.rsi, 50.0);
        assert_eq!(fv.to_array().len(), 19);
    }
}
