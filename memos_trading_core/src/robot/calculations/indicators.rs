// ========== SuperTrend ==========
pub struct SuperTrend;
impl SuperTrend {
    /// SuperTrend göstergesi (temel şablon)
    pub fn calculate(_high: &[f64], _low: &[f64], _close: &[f64], _period: usize, _multiplier: f64) -> crate::Result<Vec<f64>> {
        // TODO: SuperTrend algoritması
        Ok(Vec::new())
    }
}

// ========== Donchian Channel ==========
pub struct DonchianChannel;
impl DonchianChannel {
    /// Donchian Channel üst ve alt bantları
    pub fn calculate(_high: &[f64], _low: &[f64], _period: usize) -> crate::Result<(Vec<f64>, Vec<f64>)> {
        // TODO: Donchian Channel algoritması
        Ok((Vec::new(), Vec::new()))
    }
}

// ========== TEMA (Triple Exponential Moving Average) ==========
pub struct TEMA;
impl TEMA {
    /// TEMA göstergesi
    pub fn calculate(_values: &[f64], _period: usize) -> crate::Result<Vec<f64>> {
        // TODO: TEMA algoritması
        Ok(Vec::new())
    }
}

// ========== Stochastic RSI ==========
pub struct StochasticRSI;
impl StochasticRSI {
    /// Stochastic RSI göstergesi
    pub fn calculate(_values: &[f64], _period: usize) -> crate::Result<Vec<f64>> {
        // TODO: Stochastic RSI algoritması
        Ok(Vec::new())
    }
}

// ========== VWAP (Volume Weighted Average Price) ==========
pub struct VWAP;
impl VWAP {
    /// VWAP göstergesi
    pub fn calculate(_close: &[f64], _volume: &[f64]) -> crate::Result<Vec<f64>> {
        // TODO: VWAP algoritması
        Ok(Vec::new())
    }
}

// ========== Ichimoku Kinko Hyo ==========
pub struct Ichimoku;
impl Ichimoku {
    /// Ichimoku göstergesi (Tenkan, Kijun, Senkou Span A/B, Chikou)
    pub fn calculate(_high: &[f64], _low: &[f64], _close: &[f64], _tenkan: usize, _kijun: usize, _senkou: usize) -> crate::Result<(Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>)> {
        // TODO: Ichimoku algoritması
        Ok((Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()))
    }
}
// robot/calculations/indicators.rs - Tüm teknik göstergeler (centralized)

use crate::Result;

/// Tüm teknik göstergeler merkezi Engine
pub struct IndicatorEngine {
    version: String,
}

impl Default for IndicatorEngine {
    fn default() -> Self {
        Self {
            version: "1.0.0".to_string(),
        }
    }
}

impl IndicatorEngine {
    pub fn new() -> Self {
        Self {
            version: "1.0.0".to_string(),
        }
    }
    
    pub fn version(&self) -> &str {
        &self.version
    }
}

// ========== SMA (Simple Moving Average) ==========
pub struct SMA;
impl SMA {
    /// Basit hareketli ortalama
    pub fn calculate(prices: &[f64], period: usize) -> Result<Vec<f64>> {
        if prices.is_empty() || period == 0 || period > prices.len() {
            return Ok(Vec::new());
        }
        
        let mut result = Vec::with_capacity(prices.len() - period + 1);
        for i in period..=prices.len() {
            let sum: f64 = prices[i - period..i].iter().sum();
            result.push(sum / period as f64);
        }
        Ok(result)
    }
    
    /// Son SMA değerini döndür
    pub fn last(prices: &[f64], period: usize) -> Result<f64> {
        let smas = Self::calculate(prices, period)?;
        Ok(*smas.last().unwrap_or(&0.0))
    }
}

// ========== RSI (Relative Strength Index) ==========
pub struct RSI;
impl RSI {
    /// RSI göstergesi
    pub fn calculate(prices: &[f64], period: usize) -> Result<Vec<f64>> {
        if prices.is_empty() || period == 0 || prices.len() < period + 1 {
            return Ok(Vec::new());
        }
        
        let mut gains = Vec::new();
        let mut losses = Vec::new();
        
        // Kazançlar ve kayıpları hesapla
        for i in 1..prices.len() {
            let change = prices[i] - prices[i - 1];
            if change > 0.0 {
                gains.push(change);
                losses.push(0.0);
            } else {
                gains.push(0.0);
                losses.push(-change);
            }
        }
        
        // Ortalama kazançlar ve kayıpları hesapla
        let mut rsi_values = Vec::new();
        // En az period kadar veri gerekli
        if gains.is_empty() || period == 0 || gains.len() < period {
            return Ok(rsi_values);
        }
        
        for i in (period - 1)..gains.len() {
            let start_idx = i + 1 - period;
            let avg_gain: f64 = gains[start_idx..=i].iter().sum::<f64>() / period as f64;
            let avg_loss: f64 = losses[start_idx..=i].iter().sum::<f64>() / period as f64;
            
            let rs = if avg_loss == 0.0 { 100.0 } else { avg_gain / avg_loss };
            let rsi = 100.0 - (100.0 / (1.0 + rs));
            rsi_values.push(rsi);
        }
        
        Ok(rsi_values)
    }
    
    /// Son RSI değerini döndür
    pub fn last(prices: &[f64], period: usize) -> Result<f64> {
        let rsis = Self::calculate(prices, period)?;
        Ok(*rsis.last().unwrap_or(&50.0))
    }
}

// ========== MACD ==========
pub struct MACD;

#[derive(Clone, Debug)]
pub struct MacdOutput {
    pub macd_line: Vec<f64>,
    pub signal_line: Vec<f64>,
    pub histogram: Vec<f64>,
}

impl MACD {
    /// MACD göstergesi
    pub fn calculate(prices: &[f64]) -> Result<MacdOutput> {
        let fast_ema = Self::ema(prices, 12)?;
        let slow_ema = Self::ema(prices, 26)?;
        
        // MACD Line
        let min_len = fast_ema.len().min(slow_ema.len());
        let macd_line: Vec<f64> = (0..min_len)
            .map(|i| fast_ema[i] - slow_ema[i])
            .collect();
        
        // Signal Line (MACD'nin 9-period EMA'sı)
        let signal_line = Self::ema(&macd_line, 9)?;
        
        // Histogram
        let min_len = macd_line.len().min(signal_line.len());
        let histogram: Vec<f64> = (0..min_len)
            .map(|i| macd_line[i] - signal_line[i])
            .collect();
        
        Ok(MacdOutput { macd_line, signal_line, histogram })
    }
    
    fn ema(prices: &[f64], period: usize) -> Result<Vec<f64>> {
        if prices.is_empty() || period == 0 {
            return Ok(Vec::new());
        }
        
        let mut ema = Vec::with_capacity(prices.len());
        let multiplier = 2.0 / (period as f64 + 1.0);
        
        // İlk SMA
        if prices.len() < period {
            return Ok(Vec::new());
        }
        
        let first_sma: f64 = prices[..period].iter().sum::<f64>() / period as f64;
        ema.push(first_sma);
        
        // Kalanlar
        for i in period..prices.len() {
            let new_ema = prices[i] * multiplier + ema[i - period] * (1.0 - multiplier);
            ema.push(new_ema);
        }
        
        Ok(ema)
    }
}

// ========== Bollinger Bands ==========
pub struct BollingerBands;

#[derive(Clone, Debug)]
pub struct BollingerBandsOutput {
    pub upper: Vec<f64>,
    pub middle: Vec<f64>,
    pub lower: Vec<f64>,
}

impl BollingerBands {
    /// Bollinger Bands göstergesi
    pub fn calculate(prices: &[f64], period: usize, std_dev_mult: f64) -> Result<BollingerBandsOutput> {
        if prices.is_empty() || period == 0 {
            return Ok(BollingerBandsOutput {
                upper: Vec::new(),
                middle: Vec::new(),
                lower: Vec::new(),
            });
        }
        
        let smas = SMA::calculate(prices, period)?;
        let mut upper = Vec::new();
        let mut lower = Vec::new();
        
        for i in period - 1..prices.len() {
            let subset = &prices[i - period + 1..=i];
            let mean = subset.iter().sum::<f64>() / period as f64;
            let variance = subset.iter()
                .map(|&x| (x - mean).powi(2))
                .sum::<f64>() / period as f64;
            let std_dev = variance.sqrt();
            
            upper.push(mean + std_dev_mult * std_dev);
            lower.push(mean - std_dev_mult * std_dev);
        }
        
        Ok(BollingerBandsOutput {
            upper,
            middle: smas,
            lower,
        })
    }
}

// ========== ATR (Average True Range) ==========
pub struct ATR;
impl ATR {
    /// ATR hesaplama
    pub fn calculate(high: &[f64], low: &[f64], close: &[f64], period: usize) -> Result<Vec<f64>> {
        if high.len() < 2 || period == 0 {
            return Ok(Vec::new());
        }
        
        let mut tr_values = Vec::new();
        
        for i in 1..high.len() {
            let tr1 = high[i] - low[i];
            let tr2 = (high[i] - close[i - 1]).abs();
            let tr3 = (low[i] - close[i - 1]).abs();
            tr_values.push(tr1.max(tr2).max(tr3));
        }
        
        SMA::calculate(&tr_values, period)
    }
}

// ========== ADX (Average Directional Index) ==========
pub struct ADX;
impl ADX {
    /// ADX hesaplama
    pub fn calculate(high: &[f64], low: &[f64], period: usize) -> Result<Vec<f64>> {
        if high.len() < 2 || period == 0 {
            return Ok(Vec::new());
        }
        
        // Basitleştirilmiş ADX (tam implementasyon daha kompleks)
        let mut adx_values = Vec::new();
        
        for i in period..high.len() {
            let max_high = high[i - period..=i].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let min_low = low[i - period..=i].iter().cloned().fold(f64::INFINITY, f64::min);
            let range = max_high - min_low;
            
            if range > 0.0 {
                let adx = ((high[i] - low[i]) / range) * 100.0;
                adx_values.push(adx);
            }
        }
        
        Ok(adx_values)
    }
}

// ========== Stochastic ==========
pub struct Stochastic;
#[derive(Clone, Debug)]
pub struct StochasticOutput {
    pub k_line: Vec<f64>,
    pub d_line: Vec<f64>,
}

impl Stochastic {
    /// Stochastic Oscillator
    pub fn calculate(high: &[f64], low: &[f64], close: &[f64], period: usize) -> Result<StochasticOutput> {
        if high.len() < period {
            return Ok(StochasticOutput {
                k_line: Vec::new(),
                d_line: Vec::new(),
            });
        }
        
        let mut k_values = Vec::new();
        
        for i in period - 1..close.len() {
            let max = high[i - period + 1..=i].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let min = low[i - period + 1..=i].iter().cloned().fold(f64::INFINITY, f64::min);
            
            if (max - min).abs() > 0.0 {
                let k = ((close[i] - min) / (max - min)) * 100.0;
                k_values.push(k);
            }
        }
        
        let d_line = SMA::calculate(&k_values, 3).unwrap_or_default();
        
        Ok(StochasticOutput {
            k_line: k_values,
            d_line,
        })
    }
}

// ========== CCI (Commodity Channel Index) ==========
pub struct CCI;
impl CCI {
    /// CCI hesaplama
    pub fn calculate(high: &[f64], low: &[f64], close: &[f64], period: usize) -> Result<Vec<f64>> {
        if close.len() < period {
            return Ok(Vec::new());
        }
        
        let mut cci_values = Vec::new();
        
        for i in period - 1..close.len() {
            let tp: f64 = (high[i] + low[i] + close[i]) / 3.0;
            let stp: f64 = (0..period)
                .map(|j| (high[i - j] + low[i - j] + close[i - j]) / 3.0)
                .sum::<f64>() / period as f64;
            
            let mad: f64 = (0..period)
                .map(|j| {
                    let tp_val = (high[i - j] + low[i - j] + close[i - j]) / 3.0;
                    (tp_val - stp).abs()
                })
                .sum::<f64>() / period as f64;
            
            if mad != 0.0 {
                let cci = (tp - stp) / (0.015 * mad);
                cci_values.push(cci);
            }
        }
        
        Ok(cci_values)
    }
}

// ========== Volume Weighted Average ==========
pub struct VolumeWeightedAverage;
impl VolumeWeightedAverage {
    /// VWAP hesaplama
    pub fn calculate(prices: &[f64], volumes: &[f64]) -> Result<Vec<f64>> {
        if prices.len() != volumes.len() || prices.is_empty() {
            return Ok(Vec::new());
        }
        
        let mut vwap_values = Vec::new();
        let mut cum_pv = 0.0;
        let mut cum_v = 0.0;
        
        for i in 0..prices.len() {
            cum_pv += prices[i] * volumes[i];
            cum_v += volumes[i];
            
            if cum_v > 0.0 {
                vwap_values.push(cum_pv / cum_v);
            }
        }
        
        Ok(vwap_values)
    }
}

// ========== Keltner Channel ==========
pub struct KeltnerChannel;
#[derive(Clone, Debug)]
pub struct KeltnerChannelOutput {
    pub upper: Vec<f64>,
    pub middle: Vec<f64>,
    pub lower: Vec<f64>,
}

impl KeltnerChannel {
    /// Keltner Channel
    pub fn calculate(high: &[f64], low: &[f64], close: &[f64], period: usize, atr_mult: f64) -> Result<KeltnerChannelOutput> {
        if close.len() < period {
            return Ok(KeltnerChannelOutput {
                upper: Vec::new(),
                middle: Vec::new(),
                lower: Vec::new(),
            });
        }
        
        let middle = SMA::calculate(close, period)?;
        let atr = ATR::calculate(high, low, close, period)?;
        
        let mut upper = Vec::new();
        let mut lower = Vec::new();
        
        for i in 0..middle.len().min(atr.len()) {
            upper.push(middle[i] + atr_mult * atr[i]);
            lower.push(middle[i] - atr_mult * atr[i]);
        }
        
        Ok(KeltnerChannelOutput {
            upper,
            middle,
            lower,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn test_prices() -> Vec<f64> {
        vec![100.0, 101.5, 102.0, 101.0, 103.0, 102.5, 104.0, 103.5, 105.0,
             106.0, 105.5, 107.0, 106.5, 108.0, 107.5, 109.0, 108.5, 110.0,
             109.5, 111.0, 110.5, 112.0, 111.5, 113.0, 112.5, 114.0, 113.5]
    }
    
    #[test]
    fn test_sma() {
        let prices = test_prices();
        let result = SMA::calculate(&prices, 3).unwrap();
        assert!(!result.is_empty());
        assert_eq!(result.len(), 25); // 27 - 3 + 1
    }
    
    #[test]
    fn test_rsi() {
        let prices = test_prices();
        let result = RSI::calculate(&prices, 3).unwrap();
        assert!(!result.is_empty());
    }
    
    #[test]
    fn test_macd() {
        let prices = test_prices();
        let result = MACD::calculate(&prices).unwrap();
        assert!(!result.macd_line.is_empty());
    }
}
