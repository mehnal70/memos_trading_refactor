// robot/data_pipeline/cache.rs - Yüksek Performanslı Mum Önbellek Sistemi
use crate::core::indicators::CoreIndicatorEngine;
use crate::core::types::Candle;
use crate::prelude::*;
use chrono::{DateTime, Utc};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock}; // Mutex yerine RwLock (Çoklu okuma desteği için)

/// Sembol-bazlı 15m rolling hafıza üzerinden özet fırsat görünümü.
/// Amaç: ham akışı doğrudan işlemek yerine, küçük ama karar destekli bir pencere
/// üretmek. Bu yapı execution kararı vermez; trade filtresine girdi sağlar.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryOpportunitySnapshot {
    pub symbol: String,
    pub interval: String,
    pub bars: usize,
    pub span_minutes: usize,
    pub first_ts: Option<DateTime<Utc>>,
    pub last_ts: Option<DateTime<Utc>>,
    pub rsi_14: f64,
    pub sma_5: f64,
    pub sma_20: f64,
    pub momentum_pct: f64,
    pub volatility_pct: f64,
    pub trend_bias: f64,
    pub scalp_bias: f64,
    pub swing_bias: f64,
    pub phase: String,
}

fn rolling_volatility_pct(candles: &[Candle]) -> f64 {
    if candles.len() < 2 {
        return 0.0;
    }
    let returns: Vec<f64> = candles
        .windows(2)
        .filter_map(|w| {
            let prev = w[0].close;
            let cur = w[1].close;
            (prev > 0.0).then_some((cur - prev) / prev)
        })
        .collect();
    if returns.is_empty() {
        return 0.0;
    }
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / returns.len() as f64;
    var.sqrt() * 100.0
}

impl MemoryOpportunitySnapshot {
    pub fn from_candles(symbol: &str, interval: &str, candles: &[Candle]) -> Option<Self> {
        if candles.len() < 8 {
            return None;
        }

        let bars = candles.len();
        let first_ts = candles.first().map(|c| c.timestamp);
        let last_ts = candles.last().map(|c| c.timestamp);
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let rsi_14 = CoreIndicatorEngine::rsi(candles, 14);
        let sma_5 = CoreIndicatorEngine::sma(candles, 5);
        let sma_20 = CoreIndicatorEngine::sma(candles, 20.min(bars));
        let first_close = closes.first().copied().unwrap_or(0.0);
        let last_close = closes.last().copied().unwrap_or(0.0);
        let momentum_pct = if first_close > 0.0 {
            (last_close - first_close) / first_close * 100.0
        } else {
            0.0
        };
        let volatility_pct = rolling_volatility_pct(candles);
        let trend_bias = Self::trend_bias(rsi_14, sma_5, sma_20, momentum_pct);
        let scalp_bias = Self::scalp_bias(rsi_14, momentum_pct, volatility_pct);
        let swing_bias = Self::swing_bias(sma_5, sma_20, momentum_pct, volatility_pct);
        let phase = Self::phase_label(trend_bias, scalp_bias, swing_bias);

        Some(Self {
            symbol: symbol.to_string(),
            interval: interval.to_string(),
            bars,
            span_minutes: bars.saturating_mul(Self::interval_minutes(interval)),
            first_ts,
            last_ts,
            rsi_14,
            sma_5,
            sma_20,
            momentum_pct,
            volatility_pct,
            trend_bias,
            scalp_bias,
            swing_bias,
            phase,
        })
    }

    fn interval_minutes(interval: &str) -> usize {
        match interval {
            "1m" => 1,
            "5m" => 5,
            "15m" => 15,
            "30m" => 30,
            "1h" => 60,
            "4h" => 240,
            "1d" => 1440,
            _ => 15,
        }
    }

    fn trend_bias(rsi_14: f64, sma_5: f64, sma_20: f64, momentum_pct: f64) -> f64 {
        let sma_spread = if sma_20 > 0.0 {
            (sma_5 - sma_20) / sma_20 * 100.0
        } else {
            0.0
        };
        let rsi_term = ((rsi_14 - 50.0) / 50.0).clamp(-1.0, 1.0);
        let momentum_term = (momentum_pct / 5.0).clamp(-1.0, 1.0);
        (0.45 * sma_spread.tanh() + 0.35 * momentum_term + 0.20 * rsi_term).clamp(-1.0, 1.0)
    }

    fn scalp_bias(rsi_14: f64, momentum_pct: f64, volatility_pct: f64) -> f64 {
        let rsi_reversal = if rsi_14 <= 30.0 {
            (30.0 - rsi_14) / 30.0
        } else if rsi_14 >= 70.0 {
            -((rsi_14 - 70.0) / 30.0)
        } else {
            0.0
        };
        let momentum_term = (-momentum_pct / 3.0).clamp(-1.0, 1.0);
        let vol_term = (volatility_pct / 2.0).clamp(0.0, 1.0);
        (0.5 * rsi_reversal + 0.3 * momentum_term + 0.2 * vol_term).clamp(-1.0, 1.0)
    }

    fn swing_bias(sma_5: f64, sma_20: f64, momentum_pct: f64, volatility_pct: f64) -> f64 {
        let trend = if sma_20 > 0.0 {
            (sma_5 - sma_20) / sma_20 * 100.0
        } else {
            0.0
        };
        let momentum_term = (momentum_pct / 8.0).clamp(-1.0, 1.0);
        let vol_penalty = (volatility_pct / 4.0).clamp(0.0, 1.0);
        (0.55 * trend.tanh() + 0.35 * momentum_term - 0.10 * vol_penalty).clamp(-1.0, 1.0)
    }

    fn phase_label(trend_bias: f64, scalp_bias: f64, swing_bias: f64) -> String {
        let trend_abs = trend_bias.abs();
        let scalp_abs = scalp_bias.abs();
        let swing_abs = swing_bias.abs();

        let (label, score) = if scalp_abs >= swing_abs && scalp_abs >= trend_abs {
            if scalp_bias > 0.35 {
                ("scalp-long", scalp_bias)
            } else if scalp_bias < -0.35 {
                ("scalp-short", scalp_bias.abs())
            } else {
                ("scalp-watch", scalp_abs)
            }
        } else if swing_abs >= trend_abs {
            if swing_bias > 0.35 {
                ("swing-long", swing_bias)
            } else if swing_bias < -0.35 {
                ("swing-short", swing_bias.abs())
            } else {
                ("swing-watch", swing_abs)
            }
        } else if trend_bias > 0.35 {
            ("trend-start-up", trend_bias)
        } else if trend_bias < -0.35 {
            ("trend-start-down", trend_bias.abs())
        } else {
            ("neutral", trend_abs)
        };

        format!("{}:{:.2}", label, score.abs())
    }
}

/// Mum Önbelleği: robotic_loop içindeki REST/DB yükünü sıfıra indirir.
pub struct CandleCache {
    /// Key: "symbol_interval" (Örn: "BTCUSDT_1h")
    data: Arc<RwLock<HashMap<String, VecDeque<Candle>>>>,
    max_size: usize,
}

impl CandleCache {
    fn read_data(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, VecDeque<Candle>>> {
        match self.data.read() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        }
    }

    fn write_data(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, VecDeque<Candle>>> {
        match self.data.write() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        }
    }

    /// Yeni bir önbellek oluşturur. limit: Per-interval tutulacak maks mum sayısı.
    pub fn new(max_size: usize) -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
            max_size,
        }
    }

    /// Yeni mumları önbelleğe ekler.
    /// Eğer aynı timestamp varsa o mumu günceller (Canlı/Kapanmamış mum desteği).
    pub fn push_bulk(&self, candles: Vec<Candle>) {
        if candles.is_empty() {
            return;
        }
        let mut storage = self.write_data();

        for c in candles {
            let key = format!("{}_{}", c.symbol, c.interval);
            let entry = storage
                .entry(key)
                .or_insert_with(|| VecDeque::with_capacity(self.max_size));

            // Son mumu kontrol et: Eğer TS aynıysa güncelle, değilse yeni ekle
            if let Some(last) = entry.back_mut() {
                if last.timestamp == c.timestamp {
                    *last = c;
                    continue;
                }
            }

            entry.push_back(c);
            if entry.len() > self.max_size {
                entry.pop_front();
            }
        }
    }

    /// Son gelen 15m sentez mumlarını da aynı sembol/interval hafızasına yazar.
    /// Böylece cache yalnız ham akışı değil, karar için kullanılacak üst zaman
    /// dilimi hafızasını da tutar.
    pub fn push_synthesized(&self, candles: Vec<Candle>) {
        self.push_bulk(candles);
    }

    /// Belirli bir sembol ve aralık için son 'limit' mumu döner.
    /// robotic_loop içindeki indikatör hesaplamaları için veriyi hazır eder.
    pub fn get_latest(&self, symbol: &str, interval: &str, limit: usize) -> Vec<Candle> {
        let storage = self.read_data();
        let key = format!("{}_{}", symbol, interval);

        storage
            .get(&key)
            .map(|deq| {
                let skip = deq.len().saturating_sub(limit);
                deq.iter().skip(skip).cloned().collect()
            })
            .unwrap_or_default()
    }

    /// Veri tazeliği kontrolü: Son mum kaç saniye önce geldi?
    pub fn last_update_age(&self, symbol: &str, interval: &str) -> u64 {
        let storage = self.read_data();
        let key = format!("{}_{}", symbol, interval);

        storage
            .get(&key)
            .and_then(|deq| deq.back())
            .map(|c| (Utc::now() - c.timestamp).num_seconds().max(0) as u64)
            .unwrap_or(999_999)
    }

    pub fn clear(&self) {
        self.write_data().clear();
    }

    /// Belirtilen anahtar için kaç mum olduğunu döner.
    pub fn len(&self, symbol: &str, interval: &str) -> usize {
        let key = format!("{}_{}", symbol, interval);
        self.read_data().get(&key).map(|d| d.len()).unwrap_or(0)
    }

    /// Rolling hafızadaki son mumlardan fırsat özeti üretir.
    ///
    /// Kullanım niyeti:
    /// - scalp: RSI aşırı satım/aşırı alım + momentum dönüşü
    /// - swing: SMA5/SMA20 eğim ve momentum uyumu
    /// - trend start: momentum + kısa/uzun SMA ayrışması + ATR benzeri oynaklık
    ///
    /// Bu fonksiyon trade açmaz; yalnızca filtrelenebilir, yorumlanabilir bir
    /// ön-analiz döndürür.
    pub fn opportunity_snapshot(
        &self,
        symbol: &str,
        interval: &str,
    ) -> Option<MemoryOpportunitySnapshot> {
        let candles = self.get_latest(symbol, interval, 30);
        MemoryOpportunitySnapshot::from_candles(symbol, interval, &candles)
    }
}

impl Default for CandleCache {
    fn default() -> Self {
        Self::new(500) // Endüstri standardı 500 bar
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(symbol: &str, interval: &str, ts: i64, close: f64, volume: f64) -> Candle {
        Candle {
            timestamp: chrono::DateTime::from_timestamp(ts, 0).unwrap(),
            open: close,
            high: close * 1.002,
            low: close * 0.998,
            close,
            volume,
            symbol: symbol.to_string(),
            interval: interval.to_string(),
        }
    }

    #[test]
    fn push_synthesized_retains_15m_window() {
        let cache = CandleCache::new(50);
        let emitted = vec![candle("BTCUSDT", "15m", 1_700_000_000, 100.0, 10.0)];
        cache.push_synthesized(emitted);

        assert_eq!(cache.len("BTCUSDT", "15m"), 1);
        let latest = cache.get_latest("BTCUSDT", "15m", 5);
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].close, 100.0);
    }

    #[test]
    fn opportunity_snapshot_detects_positive_trend_bias() {
        let cache = CandleCache::new(100);
        let series: Vec<Candle> = (0..30)
            .map(|i| {
                candle(
                    "ETHUSDT",
                    "15m",
                    1_700_000_000 + i as i64 * 900,
                    100.0 + i as f64,
                    1_000.0 + i as f64,
                )
            })
            .collect();
        cache.push_bulk(series);

        let snap = cache
            .opportunity_snapshot("ETHUSDT", "15m")
            .expect("snapshot");
        assert_eq!(snap.bars, 30);
        assert_eq!(snap.span_minutes, 450);
        assert!(snap.momentum_pct > 0.0);
        assert!(snap.trend_bias > 0.0);
        assert!(snap.swing_bias > 0.0);
        assert!(snap.phase.starts_with("swing-") || snap.phase.starts_with("trend-start-"));
    }
}
