// robot/parameters/symbol_stats.rs — Sembol×interval bazlı gürültü tabanı
//
// Sabit `pos_atr_trail_mult = 2.0` her sembolde aynı ATR çarpanını uyguluyor;
// 1m ETH'de %0.04 trail (1bp dalgada tetik) → 1h SOL'de %3.3 trail. Otonom
// sistem her sembol+interval için kendi noise floor'unu hesaplayıp trail
// mult'unu hedef yüzdeye (TARGET_TRAIL_PCT, default 0.5%) göre çözmeli.
//
// Hesaplama: son N candle (≥50) üzerinde ATR(14)/close oranlarının medyanı.
// Medyan p50 outlier-robust; p90 paralel saklanır (sonraki fazda hızlı reaksiyon).

use crate::core::types::Candle;
use serde::{Deserialize, Serialize};

/// Bir (symbol, interval) için gürültü/volatilite istatistikleri.
/// `last_updated` = epoch saniye; TTL (default 6 saat) sonrası stale sayılır.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolStats {
    /// Medyan(ATR(14)/close) × 100. Tipik wiggle yüzdesi.
    pub noise_floor_pct: f64,
    /// 90'ıncı percentile aynı oran × 100. Şok dalga göstergesi (UI/HyperOpt).
    pub p90_range_pct: f64,
    /// Hesaba giren ATR-ratio örnek sayısı.
    pub sample_size: usize,
    /// Son hesap epoch saniyesi. Stale kontrolü için.
    pub last_updated: u64,
}

/// Yeterli candle yoksa veya hesap bozuksa None döner; caller fallback'e düşer.
/// Minimum örneklem: 50 candle (yeterli ATR-ratio için). Kesinlik için median
/// hızlı select yerine sort + indeks; N=200 tipik → <100µs.
pub fn compute_symbol_stats(candles: &[Candle]) -> Option<SymbolStats> {
    const ATR_PERIOD: usize = 14;
    const MIN_SAMPLE: usize = 50;

    if candles.len() < ATR_PERIOD + MIN_SAMPLE { return None; }

    let mut ratios: Vec<f64> = Vec::with_capacity(candles.len() - ATR_PERIOD);
    for i in ATR_PERIOD..candles.len() {
        let window = &candles[i - ATR_PERIOD..=i];
        let atr = atr_for_window(window);
        let close = candles[i].close;
        if close > 0.0 && atr > 0.0 && atr.is_finite() {
            ratios.push(atr / close);
        }
    }
    if ratios.len() < MIN_SAMPLE { return None; }

    ratios.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_idx = ratios.len() / 2;
    let p90_idx = (ratios.len() * 9 / 10).min(ratios.len() - 1);
    let median = ratios[median_idx];
    let p90 = ratios[p90_idx];

    let now = crate::core::time::now_epoch_secs();

    Some(SymbolStats {
        noise_floor_pct: median * 100.0,
        p90_range_pct:   p90    * 100.0,
        sample_size:     ratios.len(),
        last_updated:    now,
    })
}

/// ATR Wilder benzeri ortalama: son `period` mum üzerinden true range mean.
/// Master.rs::calc_atr ile davranış eşdeğer — period+1 mum gerekir, sonuç period kadar TR'nin ortalaması.
fn atr_for_window(window: &[Candle]) -> f64 {
    let n = window.len();
    if n < 2 { return 0.0; }
    let mut sum = 0.0;
    for w in window.windows(2) {
        let prev = &w[0];
        let cur  = &w[1];
        let h_l  = cur.high - cur.low;
        let h_pc = (cur.high - prev.close).abs();
        let l_pc = (cur.low  - prev.close).abs();
        sum += h_l.max(h_pc).max(l_pc);
    }
    sum / (n - 1) as f64
}

/// Statin tazelik testi: TTL (saniye) ile karşılaştırır. None ise stale.
pub fn is_fresh(stats: &SymbolStats, ttl_secs: u64) -> bool {
    let now = crate::core::time::now_epoch_secs();
    now.saturating_sub(stats.last_updated) < ttl_secs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::Candle;

    /// Deterministik candle üretici: close fiyatı sabit, range belirgin yüzde.
    /// Tek-saniyeli timestamp uniqueness yeter (DB constraint için değil; test izolasyonu için).
    fn synth_candles(n: usize, base_close: f64, range_pct: f64) -> Vec<Candle> {
        use chrono::{DateTime, Utc, TimeZone};
        let range = base_close * range_pct / 100.0;
        (0..n).map(|i| Candle {
            timestamp: Utc.timestamp_opt(1_700_000_000 + i as i64, 0).single()
                .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap()),
            open:      base_close,
            high:      base_close + range / 2.0,
            low:       base_close - range / 2.0,
            close:     base_close,
            volume:    1.0,
            symbol:    "TEST".to_string(),
            interval:  "1m".to_string(),
        }).collect()
    }

    #[test]
    fn returns_none_for_small_sample() {
        let c = synth_candles(40, 100.0, 0.5);
        assert!(compute_symbol_stats(&c).is_none(), "<50+14 candle → None");
    }

    #[test]
    fn returns_some_with_correct_median_for_uniform_range() {
        // Tüm mumların range'i %0.5 → TR sabit = 0.005×close → ATR/close ≈ 0.005
        let c = synth_candles(200, 100.0, 0.5);
        let s = compute_symbol_stats(&c).expect("200 candle yeter");
        // noise_floor %0.5 civarı (1-2 bps tolerans, TR ilk-mum farklılığı için)
        assert!((s.noise_floor_pct - 0.5).abs() < 0.05,
            "uniform %0.5 range → noise ~0.5, gerçek {}", s.noise_floor_pct);
        assert!(s.sample_size >= 100, "min sample yeterli");
        assert!(s.last_updated > 0, "epoch dolu");
        assert!(s.p90_range_pct >= s.noise_floor_pct, "p90 ≥ median");
    }

    #[test]
    fn handles_higher_volatility() {
        // Range %2 → noise ~2.0
        let c = synth_candles(200, 50.0, 2.0);
        let s = compute_symbol_stats(&c).expect("200 candle yeter");
        assert!((s.noise_floor_pct - 2.0).abs() < 0.1,
            "uniform %2.0 → noise ~2.0, gerçek {}", s.noise_floor_pct);
    }

    #[test]
    fn is_fresh_within_ttl_then_stale() {
        let mut s = SymbolStats {
            noise_floor_pct: 0.5, p90_range_pct: 0.7,
            sample_size: 100, last_updated: 0,
        };
        let now = crate::core::time::now_epoch_secs();
        s.last_updated = now - 100; // 100sn önce
        assert!(is_fresh(&s, 3600), "1 saat TTL: 100sn fresh");
        s.last_updated = now - 10_000; // 10K sn önce
        assert!(!is_fresh(&s, 3600), "1 saat TTL: 10000sn stale");
    }

    #[test]
    fn returns_none_when_all_closes_zero() {
        use chrono::{DateTime, Utc, TimeZone};
        let c: Vec<Candle> = (0..100).map(|i| Candle {
            timestamp: Utc.timestamp_opt(1_700_000_000 + i as i64, 0).single()
                .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap()),
            open: 0.0, high: 0.0, low: 0.0, close: 0.0, volume: 0.0,
            symbol: "TEST".to_string(), interval: "1m".to_string(),
        }).collect();
        assert!(compute_symbol_stats(&c).is_none(), "tüm close=0 → None");
    }
}
