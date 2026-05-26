// robot/data_pipeline/htf_loader.rs - Multi-TF (Faz B) HTF mum yükleyici
//
// Görev: strateji cycle'ı için üst zaman dilimi (HTF) mumlarını sağlamak.
// İki kanal:
//   1) DB önceliği — `read_candles(db_path, symbol, htf_interval, n)` ile
//      doğrudan oku. run_download_job (jobs.rs) multi_tf.download_htf açıkken
//      HTF interval'i de indirip aynı `candles` tablosuna yazar → burası dolar.
//   2) Fallback — base interval "1m" ise, eldeki 1m geçmişini CandleSynth ile
//      sentezleyerek HTF üret. 5m/15m/30m baz interval'lerinde fallback yok;
//      DB boşsa boş döner (htf_trend_filter None'da sinyal pass-through eder).
//
// Hedef HTF eşlemesi `DataPipeline::get_htf_interval` tek-noktasından gelir.

use crate::core::types::Candle;
use crate::persistence::reader::read_candles;
use super::orchestrator::DataPipeline;
use super::synth::CandleSynth;

/// Minimum HTF mum sayısı — htf_trend_filter `slow=30` SMA istediği için
/// 30'un altında çağrı boşa gider; loader bunun altını "yetersiz" sayar.
pub const HTF_MIN_REQUIRED: usize = 30;

/// HTF mumlarını yükle.
/// `min_required` az ise DB sonucu da yetersiz sayılır → fallback denenir.
/// Hiçbiri yetmezse boş Vec döner — çağıran `Some(&v)` yerine `None` geçirebilir.
pub fn load_htf_candles(
    db_path: &str,
    symbol: &str,
    base_interval: &str,
    min_required: usize,
) -> Vec<Candle> {
    let htf = DataPipeline::get_htf_interval(base_interval);
    if htf == base_interval {
        return Vec::new();
    }

    let need = min_required.max(HTF_MIN_REQUIRED);

    // 1) DB önceliği
    if let Ok(c) = read_candles(db_path, symbol, htf, need.max(50)) {
        if c.len() >= need {
            return c;
        }
    }

    // 2) Fallback: sadece base="1m" için CandleSynth ile in-memory aggregate.
    if base_interval == "1m" {
        let target_mins = interval_minutes(htf);
        if target_mins == 0 {
            return Vec::new();
        }
        let pull = (need * target_mins) + target_mins; // bir tampon
        if let Ok(base_1m) = read_candles(db_path, symbol, base_interval, pull) {
            let agg = aggregate_1m_to(&base_1m, htf, symbol);
            if agg.len() >= need {
                return agg;
            }
            // Yetersiz olsa bile hiç olmamasından iyidir; htf_trend_filter
            // kendi `len() < slow` guard'ı ile pass-through yapar.
            return agg;
        }
    }

    Vec::new()
}

/// 1 dakikalık mum dizisini CandleSynth üzerinden hedef HTF'e sentezler.
/// Synth tüm üst-TF'leri eş zamanlı üretir; biz sadece hedefi filtreliyoruz.
pub fn aggregate_1m_to(candles_1m: &[Candle], target_interval: &str, symbol: &str) -> Vec<Candle> {
    if candles_1m.is_empty() {
        return Vec::new();
    }
    let mut synth = CandleSynth::new(symbol, Box::new(|_c: &Candle| {}));
    let mut out: Vec<Candle> = Vec::new();
    for c in candles_1m {
        for emitted in synth.push_1m(c) {
            if emitted.interval == target_interval {
                out.push(emitted);
            }
        }
    }
    out
}

#[inline]
fn interval_minutes(intv: &str) -> usize {
    match intv {
        "1m" => 1,
        "5m" => 5,
        "15m" => 15,
        "30m" => 30,
        "1h" => 60,
        "4h" => 240,
        "1d" => 1440,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};

    fn mk_1m(start_min: i64, count: usize, base_close: f64) -> Vec<Candle> {
        let start = Utc.timestamp_opt(0, 0).single().unwrap();
        (0..count)
            .map(|i| {
                let ts = start + Duration::minutes(start_min + i as i64);
                let close = base_close + (i as f64) * 0.1;
                Candle {
                    timestamp: ts,
                    open: close - 0.05,
                    high: close + 0.2,
                    low: close - 0.2,
                    close,
                    volume: 10.0,
                    symbol: "TEST".to_string(),
                    interval: "1m".to_string(),
                }
            })
            .collect()
    }

    #[test]
    fn aggregate_1m_to_5m_emits_correct_count() {
        let base = mk_1m(0, 100, 100.0); // 100 dk → 20 adet 5m
        let agg = aggregate_1m_to(&base, "5m", "TEST");
        assert_eq!(agg.len(), 20);
        for c in &agg {
            assert_eq!(c.interval, "5m");
            assert!(c.high >= c.low);
        }
    }

    #[test]
    fn aggregate_1m_to_1h_needs_60_bars() {
        let base = mk_1m(0, 59, 100.0);
        let agg = aggregate_1m_to(&base, "1h", "TEST");
        assert!(agg.is_empty(), "59 dk 1h üretmemeli");

        let base2 = mk_1m(0, 120, 100.0); // 2 saat
        let agg2 = aggregate_1m_to(&base2, "1h", "TEST");
        assert_eq!(agg2.len(), 2);
    }

    #[test]
    fn aggregate_filters_other_intervals() {
        let base = mk_1m(0, 60, 100.0);
        let agg = aggregate_1m_to(&base, "15m", "TEST");
        // 60 dakika → 4 tane 15m mum, hiçbiri başka interval'da değil.
        assert_eq!(agg.len(), 4);
        assert!(agg.iter().all(|c| c.interval == "15m"));
    }

    #[test]
    fn aggregate_empty_input_returns_empty() {
        let agg = aggregate_1m_to(&[], "5m", "TEST");
        assert!(agg.is_empty());
    }

    #[test]
    fn aggregate_unsupported_target_returns_empty() {
        let base = mk_1m(0, 60, 100.0);
        let agg = aggregate_1m_to(&base, "7m", "TEST");
        assert!(agg.is_empty());
    }

    #[test]
    fn interval_minutes_known_values() {
        assert_eq!(interval_minutes("1m"), 1);
        assert_eq!(interval_minutes("5m"), 5);
        assert_eq!(interval_minutes("1h"), 60);
        assert_eq!(interval_minutes("4h"), 240);
        assert_eq!(interval_minutes("1d"), 1440);
        assert_eq!(interval_minutes("garbage"), 0);
    }
}
