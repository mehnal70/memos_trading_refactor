/// Donchian Channel Stratejisi
pub struct DonchianChannelStrategy;

impl Strategy for DonchianChannelStrategy {
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        let period = params.period.unwrap_or(20);
        if candles.len() < period + 1 {
            return Ok(Signal::Hold);
        }
        let recent       = &candles[candles.len()-period..];
        let highest_high = recent.iter().map(|c| c.high).fold(f64::MIN, f64::max);
        let lowest_low   = recent.iter().map(|c| c.low ).fold(f64::MAX, f64::min);
        let last_close   = candles.last().unwrap().close;
        let raw = if last_close > highest_high {
            Signal::Buy
        } else if last_close < lowest_low {
            Signal::Sell
        } else {
            Signal::Hold
        };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "Donchian"))
    }
    fn name(&self) -> &str { "Donchian Channel" }
}
/// Funding Rate Contrarian Stratejisi
pub struct FundingRateContrarianStrategy {
    /// Pozitif funding rate için short, negatif için long açma eşiği
    pub threshold: f64,
}

impl Strategy for FundingRateContrarianStrategy {
    fn generate_signal(&self, _candles: &[Candle], _params: &StrategyParams, funding_rates: Option<&[FundingRatePoint]>, _htf_candles: Option<&[Candle]>) -> Result<Signal> {
        // Funding rate verisi yoksa işlem yapma
        let rates = match funding_rates {
            Some(r) if !r.is_empty() => r,
            _ => return Ok(Signal::Hold),
        };
        // Son funding rate değerini al, hata kontrolü
        match rates.last() {
            Some(last) => {
                if last.funding_rate >= self.threshold {
                    Ok(Signal::Sell)
                } else if last.funding_rate <= -self.threshold {
                    Ok(Signal::Buy)
                } else {
                    Ok(Signal::Hold)
                }
            }
            None => {
                log::warn!("FundingRateContrarian: rates.last() None, veri yok.");
                Ok(Signal::Hold)
            }
        }
    }

    fn name(&self) -> &str {
        "FundingRateContrarian"
    }
}
use crate::types::{Candle, Signal, StrategyParams};
use log;
use crate::Result;

/// Strateji trait'i
use crate::types::FundingRatePoint;

pub trait Strategy: Send + Sync {
    /// Belirtilen parametrelerle sinyal üret.
    ///
    /// * `candles`      — işlem aralığı (ör: 1m, 5m) mum verileri
    /// * `funding_rates`— futures funding oranları (opsiyonel)
    /// * `htf_candles`  — üst zaman dilimi (ör: 1h, 4h) mum verileri — MTF trend filtresi için.
    ///   `None` gelirse strateji yalnızca `candles` ile çalışır (geriye uyumlu).
    fn generate_signal(
        &self,
        candles:       &[Candle],
        params:        &StrategyParams,
        funding_rates: Option<&[FundingRatePoint]>,
        htf_candles:   Option<&[Candle]>,
    ) -> Result<Signal>;

    /// Strateji adı
    fn name(&self) -> &str;

    /// Varsayılan parametreleri döndür (isteğe bağlı override)
    fn default_params(&self) -> StrategyParams {
        StrategyParams::default()
    }

    /// Parametre aralığı (ör: optimizasyon için)
    fn param_ranges(&self) -> Option<Vec<(String, Vec<f64>)>> {
        None
    }
}
/// Basit grid search ile parametre optimizasyonu
/// Her kombinasyon için evaluate_fn çağrılır, en iyi skor döndürülür
pub async fn grid_search_optimization<F>(
    _strategy: &dyn Strategy,
    candles: &[Candle],
    param_grid: Vec<(String, Vec<f64>)>,
    evaluate_fn: F,
) -> Option<(StrategyParams, f64)>
where
    F: Fn(&[Candle], &StrategyParams) -> f64,
{
    use itertools::Itertools;
    // Tüm kombinasyonları oluştur
    let keys: Vec<_> = param_grid.iter().map(|(k, _)| k.clone()).collect();
    let value_lists: Vec<_> = param_grid.iter().map(|(_, v)| v.clone()).collect();
    let mut best_score = f64::MIN;
    let mut best_params = None;
    for combo in value_lists.into_iter().multi_cartesian_product() {
        let mut params = StrategyParams::default();
        for (i, val) in combo.iter().enumerate() {
            match keys[i].as_str() {
                "fast" => params.fast = Some(*val as usize),
                "slow" => params.slow = Some(*val as usize),
                "period" => params.period = Some(*val as usize),
                "overbought" => params.overbought = Some(*val),
                "oversold" => params.oversold = Some(*val),
                "std_dev" => params.std_dev = Some(*val),
                _ => {},
            }
        }
        let score = evaluate_fn(candles, &params);
        if score > best_score {
            best_score = score;
            best_params = Some(params.clone());
        }
    }
    best_params.map(|p| (p, best_score))
}

/// MA Crossover stratejisi
pub struct MaCrossoverStrategy;

impl Strategy for MaCrossoverStrategy {
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        use std::time::Instant;
        let start = Instant::now();
        let fast_period = params.fast.unwrap_or(10);
        let slow_period = params.slow.unwrap_or(30);
        if fast_period == 0 || slow_period == 0 {
            log::error!("MA Crossover: Parametreler geçersiz (fast={}, slow={})", fast_period, slow_period);
            return Err(crate::MemosTradingError::Strategy("Geçersiz MA parametreleri".to_string()));
        }
        if candles.len() < slow_period + 1 {
            log::warn!("MA Crossover: Yeterli veri yok ({} < {})", candles.len(), slow_period + 1);
            return Ok(Signal::Hold);
        }
        let fast_ma_prev = calculate_sma(&candles[..candles.len()-1], fast_period)?;
        let slow_ma_prev = calculate_sma(&candles[..candles.len()-1], slow_period)?;
        let fast_ma = calculate_sma(candles, fast_period)?;
        let slow_ma = calculate_sma(candles, slow_period)?;
        let raw_signal = if fast_ma_prev <= slow_ma_prev && fast_ma > slow_ma {
            Signal::Buy
        } else if fast_ma_prev >= slow_ma_prev && fast_ma < slow_ma {
            Signal::Sell
        } else {
            Signal::Hold
        };

        let signal = htf_trend_filter(raw_signal, htf_candles, params.fast.unwrap_or(10), params.slow.unwrap_or(30), "MA Crossover");
        if !matches!(signal, Signal::Hold) {
            log::info!("MA Crossover: {:?} sinyali üretildi.", signal);
        } else {
            log::info!("MA Crossover: Sinyal yok, Hold.");
        }
        log::debug!("MA Crossover sinyal hesaplama süresi: {:?}", start.elapsed());
        Ok(signal)
    }
    
    fn name(&self) -> &str {
        "MA Crossover"
    }
}

/// RSI stratejisi
pub struct RsiStrategy;

impl Strategy for RsiStrategy {
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        if candles.is_empty() {
            return Ok(Signal::Hold);
        }
        let period     = params.period.unwrap_or(14);
        let overbought = params.overbought.unwrap_or(70.0);
        let oversold   = params.oversold.unwrap_or(30.0);
        if candles.len() < period {
            return Ok(Signal::Hold);
        }
        let rsi = calculate_rsi(&candles, period)?;
        let raw = if rsi > overbought {
            Signal::Sell
        } else if rsi < oversold {
            Signal::Buy
        } else {
            Signal::Hold
        };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "RSI"))
    }
    
    fn name(&self) -> &str {
        "RSI"
    }
}

/// MACD stratejisi
pub struct MacdStrategy;

impl Strategy for MacdStrategy {
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        if candles.len() < 35 {
            return Ok(Signal::Hold);
        }
        let fast_period   = params.fast_period.unwrap_or(12);
        let slow_period   = params.slow_period.unwrap_or(26);
        let signal_period = params.signal_period.unwrap_or(9);
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let fast_ema  = calculate_ema(&closes, fast_period)?;
        let slow_ema  = calculate_ema(&closes, slow_period)?;
        let macd_line: Vec<f64> = fast_ema.iter().zip(&slow_ema).map(|(f, s)| f - s).collect();
        let signal_line = calculate_ema(&macd_line, signal_period)?;
        if macd_line.len() < 2 {
            return Ok(Signal::Hold);
        }
        let last_macd   = macd_line[macd_line.len() - 1];
        let last_sig    = signal_line[signal_line.len() - 1];
        let prev_macd   = macd_line[macd_line.len() - 2];
        let prev_sig    = signal_line[signal_line.len() - 2];
        let raw = if prev_macd <= prev_sig && last_macd > last_sig {
            Signal::Buy
        } else if prev_macd >= prev_sig && last_macd < last_sig {
            Signal::Sell
        } else {
            Signal::Hold
        };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "MACD"))
    }
    
    fn name(&self) -> &str {
        "MACD"
    }
}

/// Bollinger Bands stratejisi
pub struct BollingerBandsStrategy;

impl Strategy for BollingerBandsStrategy {
    fn generate_signal(&self, candles: &[Candle], params: &StrategyParams, _funding_rates: Option<&[FundingRatePoint]>, htf_candles: Option<&[Candle]>) -> Result<Signal> {
        let period  = params.period.unwrap_or(20);
        let std_dev = params.std_dev.unwrap_or(2.0);
        if candles.len() < period {
            return Ok(Signal::Hold);
        }
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let sma = calculate_sma(candles, period)?;
        let recent = &closes[closes.len() - period..];
        let mean  = recent.iter().sum::<f64>() / period as f64;
        let std   = (recent.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / period as f64).sqrt();
        let upper = sma + std_dev * std;
        let lower = sma - std_dev * std;
        let price = closes[closes.len() - 1];
        let raw = if price < lower {
            Signal::Buy
        } else if price > upper {
            Signal::Sell
        } else {
            Signal::Hold
        };
        Ok(htf_trend_filter(raw, htf_candles, 10, 30, "Bollinger Bands"))
    }
    
    fn name(&self) -> &str {
        "Bollinger Bands"
    }
}

// ── HTF Trend Filtresi ────────────────────────────────────────────────────────
/// HTF candle'ların fast/slow SMA'sına bakarak sinyali filtreler.
///
/// * BUY  → HTF bearish (fast < slow) ise Hold döner
/// * SELL → HTF bullish (fast > slow) ise Hold döner
/// * Hold veya HTF verisi yoksa / yetersizse sinyal aynen döner
///
/// `fast_period` ve `slow_period` HTF MA periyotlarıdır (30/100 tipik değer).
pub fn htf_trend_filter(
    signal:      Signal,
    htf_candles: Option<&[Candle]>,
    fast_period: usize,
    slow_period: usize,
    name:        &str,
) -> Signal {
    if matches!(signal, Signal::Hold) { return signal; }
    let htf = match htf_candles {
        Some(h) if h.len() >= slow_period => h,
        _ => return signal, // HTF yoksa veya yeterli mum yok — filtresiz geç
    };
    let htf_fast = calculate_sma(htf, fast_period).unwrap_or(0.0);
    let htf_slow = calculate_sma(htf, slow_period).unwrap_or(0.0);
    if htf_fast == 0.0 || htf_slow == 0.0 { return signal; }
    let htf_bullish = htf_fast > htf_slow;
    let htf_bearish = htf_fast < htf_slow;
    match signal {
        Signal::Buy  if htf_bearish => {
            log::info!("{} MTF: BUY üretildi fakat HTF Bearish — Hold.", name);
            Signal::Hold
        }
        Signal::Sell if htf_bullish => {
            log::info!("{} MTF: SELL üretildi fakat HTF Bullish — Hold.", name);
            Signal::Hold
        }
        other => {
            log::info!("{} MTF: Sinyal HTF trendi ile uyumlu ({:?}).", name, other);
            other
        }
    }
}

// EMA hesaplama fonksiyonu
fn calculate_ema(values: &[f64], period: usize) -> Result<Vec<f64>> {
    if values.is_empty() {
        return Ok(Vec::new());
    }
    
    let mut ema = vec![0.0; values.len()];
    let multiplier = 2.0 / (period as f64 + 1.0);
    
    // İlk SMA'yı hesapla
    let sum: f64 = values[..period.min(values.len())].iter().sum();
    ema[period.min(values.len()) - 1] = sum / period as f64;
    
    // EMA'yı hesapla
    for i in period..values.len() {
        ema[i] = values[i] * multiplier + ema[i - 1] * (1.0 - multiplier);
    }
    
    Ok(ema)
}

/// Basit Hareketli Ortalama hesapla
pub fn calculate_sma(candles: &[Candle], period: usize) -> Result<f64> {
    if candles.len() < period {
        return Err(crate::MemosTradingError::Strategy(
            format!("Not enough data for SMA calculation: {} < {}", candles.len(), period)
        ));
    }
    // Son period kadarını al
    let sum: f64 = candles[candles.len()-period..].iter().map(|c| c.close).sum();
    Ok(sum / period as f64)
}

/// RSI hesapla
pub fn calculate_rsi(candles: &[Candle], period: usize) -> Result<f64> {
    if candles.len() < period + 1 {
        return Err(crate::MemosTradingError::Strategy(
            format!("Not enough data for RSI calculation")
        ));
    }
    
    let mut gains = 0.0;
    let mut losses = 0.0;
    
    for i in 0..period {
        let change = candles[i].close - candles[i + 1].close;
        if change > 0.0 {
            gains += change;
        } else {
            losses += -change;
        }
    }
    
    let avg_gain = gains / period as f64;
    let avg_loss = losses / period as f64;
    
    if avg_loss == 0.0 {
        Ok(100.0)
    } else {
        let rs = avg_gain / avg_loss;
        Ok(100.0 - (100.0 / (1.0 + rs)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_ma_crossover() {
        // 5 barlık veri, fast=2, slow=3 ile crossover oluşacak şekilde
        let candles = vec![
            Candle { timestamp: Utc::now(), open: 10.0, high: 10.0, low: 10.0, close: 10.0, volume: 1000.0, symbol: "BTC".to_string(), interval: "1h".to_string() },
            Candle { timestamp: Utc::now(), open: 10.0, high: 10.0, low: 10.0, close: 10.0, volume: 1000.0, symbol: "BTC".to_string(), interval: "1h".to_string() },
            Candle { timestamp: Utc::now(), open: 10.0, high: 10.0, low: 10.0, close: 10.0, volume: 1000.0, symbol: "BTC".to_string(), interval: "1h".to_string() },
            Candle { timestamp: Utc::now(), open: 10.0, high: 10.0, low: 10.0, close: 10.0, volume: 1000.0, symbol: "BTC".to_string(), interval: "1h".to_string() },
            Candle { timestamp: Utc::now(), open: 100.0, high: 100.0, low: 100.0, close: 100.0, volume: 1000.0, symbol: "BTC".to_string(), interval: "1h".to_string() },
        ];

        let strategy = MaCrossoverStrategy;
        let params = StrategyParams {
            fast: Some(2),
            slow: Some(4),
            period: None,
            overbought: None,
            oversold: None,
            fast_period: None,
            slow_period: None,
            signal_period: None,
            std_dev: None,
            bb_period: None,
        };

        let result = strategy.generate_signal(&candles, &params, None, None);
        assert!(result.is_ok());
        let signal = match result {
            Ok(sig) => sig,
            Err(e) => {
                log::warn!("MA crossover test sinyali hata: {:?}", e);
                Signal::Hold
            }
        };
        println!("MA crossover test sinyali: {:?}", signal);
        assert_eq!(signal, Signal::Buy);
    }
}
