// robot/data_pipeline/health.rs — Faz 3: candle veri-sağlık kapısı
//
// Otonom işler (screener pool, backtest) eşik-altı (bayat/sparse/gappy) sembol×interval
// üzerinde koşmasın → kirli veriden çıkan yanıltıcı verdiktler engellenir. Metrikler
// ZATEN YÜKLÜ mumlardan hesaplanır (ek IO yok); screener/backtest candle'ı zaten okuyor.
//
// Mevcut veri gappy olduğundan (1h bile ~%22-49 boşluk, Faz 2 backfill öncesi) kapı
// ÖNCELİKLE bayatlık + min-satır üzerinedir; gap eşiği gevşek default'lu (operatör
// Faz 2 sonrası sıkabilir). Eşikler RuntimeTuning'den (env-ayarlı, [[feedback_config_externalization]]).

use crate::core::types::Candle;
use super::normalizer::DataNormalizer;

/// Bir (sembol,interval,market) serisinin sağlık metrikleri.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CandleHealth {
    /// Yüklü bar sayısı.
    pub rows: usize,
    /// Son pencerede beklenen bara göre eksik yüzdesi (0..100).
    pub gap_pct: f64,
    /// Son barın yaşı (saniye). Boş seri → i64::MAX.
    pub stale_secs: i64,
}

/// Operatör-ayarı sağlık eşikleri (RuntimeTuning env'lerinden türetilir).
#[derive(Debug, Clone, Copy)]
pub struct HealthThresholds {
    /// Asgari bar sayısı (altı → sağlıksız).
    pub min_rows: usize,
    /// İzin verilen maksimum gap yüzdesi (0..100).
    pub max_gap_pct: f64,
    /// Son bar en fazla bu kadar BAR-yaşında olabilir (interval-farkında: ×interval_secs).
    pub max_stale_bars: u64,
}

impl CandleHealth {
    /// Kronolojik (ASC) mum diliminden hesaplar. Boş → rows=0, gap=100, stale=MAX.
    pub fn from_candles(candles: &[Candle], interval: &str) -> Self {
        let rows = candles.len();
        if rows == 0 {
            return Self { rows: 0, gap_pct: 100.0, stale_secs: i64::MAX };
        }
        let sec = DataNormalizer::parse_interval(interval).max(1) as i64;
        let first = candles.first().map(|c| c.timestamp.timestamp()).unwrap_or(0);
        let last = candles.last().map(|c| c.timestamp.timestamp()).unwrap_or(0);
        let span = (last - first).max(0);
        let expected = (span / sec) + 1;
        let gap_pct = if expected > 0 {
            (1.0 - rows as f64 / expected as f64).clamp(0.0, 1.0) * 100.0
        } else { 0.0 };
        let now = crate::core::time::now_epoch_secs() as i64;
        let stale_secs = (now - last).max(0);
        Self { rows, gap_pct, stale_secs }
    }

    /// Kapı verdiği: tüm eşikleri geçiyor mu (rows ≥ min, gap ≤ max, stale ≤ N×interval).
    pub fn is_healthy(&self, th: &HealthThresholds, interval: &str) -> bool {
        let sec = DataNormalizer::parse_interval(interval).max(1) as i64;
        let max_stale = (th.max_stale_bars as i64).saturating_mul(sec);
        self.rows >= th.min_rows
            && self.gap_pct <= th.max_gap_pct
            && self.stale_secs <= max_stale
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn series(n: usize, interval_secs: i64, last_age_secs: i64) -> Vec<Candle> {
        let now = crate::core::time::now_epoch_secs() as i64;
        let last = now - last_age_secs;
        (0..n).map(|i| {
            let ts = last - (n as i64 - 1 - i as i64) * interval_secs;
            Candle {
                timestamp: Utc.timestamp_opt(ts, 0).single().unwrap(),
                open: 1.0, high: 1.0, low: 1.0, close: 1.0, volume: 1.0,
                symbol: "T".into(), interval: "1h".into(),
            }
        }).collect()
    }

    #[test]
    fn fresh_contiguous_series_is_healthy() {
        let c = series(200, 3600, 0); // 200 bar, 1h, taze, gapsiz
        let h = CandleHealth::from_candles(&c, "1h");
        assert_eq!(h.rows, 200);
        assert!(h.gap_pct < 1.0, "gapsiz → ~%0, gerçek {}", h.gap_pct);
        let th = HealthThresholds { min_rows: 100, max_gap_pct: 90.0, max_stale_bars: 10 };
        assert!(h.is_healthy(&th, "1h"));
    }

    #[test]
    fn stale_series_rejected() {
        // Son bar 50 bar-yaşında (50h) → max_stale_bars=10 eşiğini geçemez.
        let c = series(200, 3600, 50 * 3600);
        let h = CandleHealth::from_candles(&c, "1h");
        let th = HealthThresholds { min_rows: 100, max_gap_pct: 90.0, max_stale_bars: 10 };
        assert!(!h.is_healthy(&th, "1h"), "bayat seri reddedilmeli (stale={}s)", h.stale_secs);
    }

    #[test]
    fn sparse_series_rejected() {
        let c = series(30, 3600, 0); // 30 bar < min 100
        let h = CandleHealth::from_candles(&c, "1h");
        let th = HealthThresholds { min_rows: 100, max_gap_pct: 90.0, max_stale_bars: 10 };
        assert!(!h.is_healthy(&th, "1h"));
    }

    #[test]
    fn empty_series_is_unhealthy() {
        let h = CandleHealth::from_candles(&[], "1h");
        assert_eq!(h.rows, 0);
        assert_eq!(h.stale_secs, i64::MAX);
        let th = HealthThresholds { min_rows: 1, max_gap_pct: 100.0, max_stale_bars: u64::MAX };
        assert!(!h.is_healthy(&th, "1h"));
    }
}
