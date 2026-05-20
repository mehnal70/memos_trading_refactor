// Screener saf API'sinin kütüphane sınırı dışından doğrulanması.
//
// run_screener_job uçtan uca testi state-heavy + SQLite gerektirdiği için
// burada saf yardımcılar test edilir. Cycle entegrasyonu master.rs içinde,
// trigger handler "screener" case'inde devreye girer.

use memos_trading_core::core::types::Candle;
use memos_trading_core::robot::screener::{
    score_symbol, select_top_n_diff, ScreenerScore, SelectionDiff,
};

fn synthetic_wave(n: usize, vol: f64) -> Vec<Candle> {
    (0..n)
        .map(|i| {
            let phase = (i as f64) * 0.10;
            let close = 100.0 + 10.0 * phase.sin() + (i as f64) * 0.05;
            Candle {
                open: close - 0.3, high: close + 0.6, low: close - 0.6, close,
                volume: vol, ..Default::default()
            }
        })
        .collect()
}

#[test]
fn score_symbol_runs_end_to_end_on_synthetic_wave() {
    let c = synthetic_wave(300, 1_000.0);
    let s = score_symbol(&c, "MA_CROSSOVER", 4.0, 2.0, 0.3, 10_000.0);
    assert_eq!(s.avg_volume, 1_000.0);
    assert!(s.atr_pct >= 0.0);
    // Sentetik veri trade üretebilir veya üretmeyebilir; ama hepsi sayısal
    // alanlar finite olmalı.
    assert!(s.composite.is_finite());
    assert!(s.sharpe.is_finite());
    assert!(s.win_rate >= 0.0 && s.win_rate <= 100.0);
    assert!(s.max_dd_pct >= 0.0);
}

#[test]
fn score_changes_with_volume_proxy_only() {
    let mut a = synthetic_wave(200, 100.0);
    let mut b = synthetic_wave(200, 5_000.0);
    // Aynı fiyat patikası ama volume farklı.
    for c in a.iter_mut() { c.volume = 100.0; }
    for c in b.iter_mut() { c.volume = 5_000.0; }
    let sa = score_symbol(&a, "MA_CROSSOVER", 4.0, 2.0, 0.3, 10_000.0);
    let sb = score_symbol(&b, "MA_CROSSOVER", 4.0, 2.0, 0.3, 10_000.0);
    assert!(sb.avg_volume > sa.avg_volume);
    // Composite skor fiyat patikası aynı olduğu için aynı olmalı (volume
    // skora doğrudan girmiyor, sadece likitite proxy).
    assert!((sa.composite - sb.composite).abs() < 1e-9,
        "composite volume'dan bağımsız olmalı: sa={} sb={}", sa.composite, sb.composite);
}

// ─────────────────────────────────────────────────────────────────────────────
// select_top_n_diff entegrasyon davranışları
// ─────────────────────────────────────────────────────────────────────────────

fn scored(named_scores: &[(&str, f64)]) -> Vec<(String, ScreenerScore)> {
    named_scores.iter().map(|(n, c)| {
        let mut s = ScreenerScore::empty(0.0, 0.0);
        s.composite = *c;
        ((*n).to_string(), s)
    }).collect()
}

#[test]
fn empty_pool_returns_empty_diff() {
    let d: SelectionDiff = select_top_n_diff(&[], &[], &[], 8, 16);
    assert!(d.selected.is_empty());
    assert!(d.to_add.is_empty());
    assert!(d.to_remove.is_empty());
}

#[test]
fn pinned_only_keeps_pinned_no_churn() {
    let pinned = vec!["BTCUSDT".to_string()];
    let current = vec!["BTCUSDT".to_string()];
    let d = select_top_n_diff(&current, &pinned, &[], 8, 16);
    assert_eq!(d.selected, vec!["BTCUSDT".to_string()]);
    assert!(d.to_add.is_empty());
    assert!(d.to_remove.is_empty());
}

#[test]
fn promotion_and_demotion_in_single_cycle() {
    let current = vec!["BTCUSDT".to_string(), "OLDCOIN".to_string()];
    let pinned  = vec!["BTCUSDT".to_string()];
    let s = scored(&[("ETHUSDT", 1.2), ("AVAXUSDT", 0.9), ("OLDCOIN", 0.1)]);
    let d = select_top_n_diff(&current, &pinned, &s, 3, 16);
    // Sıra: BTC (pinned), ETH, AVAX → 3 slot dolu; OLDCOIN düşer.
    assert_eq!(d.selected, vec!["BTCUSDT".to_string(), "ETHUSDT".into(), "AVAXUSDT".into()]);
    assert!(d.to_add.iter().any(|s| s == "ETHUSDT"));
    assert!(d.to_add.iter().any(|s| s == "AVAXUSDT"));
    assert_eq!(d.to_remove, vec!["OLDCOIN".to_string()]);
}

#[test]
fn top_n_zero_with_pinned_still_does_not_drop_pinned() {
    // top_n=0 patolojik: kapasitenin 0 olmaması beklenir ama pinned korunmalı.
    let pinned = vec!["BTC".to_string()];
    let current = vec!["BTC".to_string(), "ETH".to_string()];
    let d = select_top_n_diff(&current, &pinned, &[], 0, 16);
    // top_n=0 → cap=0 → selected boş; ama pinned'i removal listesine almıyoruz
    // (semantik: pinned hiçbir koşulda düşürülmez).
    assert!(d.selected.is_empty());
    assert_eq!(d.to_remove, vec!["ETH".to_string()],
        "pinned düşmemeli, sadece ETH atılmalı: {d:?}");
}

#[test]
fn max_workers_cap_applies_after_top_n() {
    let pinned = vec!["BTC".to_string()];
    let s = scored(&[("ETH", 3.0), ("SOL", 2.0), ("AVAX", 1.0), ("BNB", 0.5)]);
    let d = select_top_n_diff(&[], &pinned, &s, 10, 3); // max_workers=3
    assert_eq!(d.selected.len(), 3);
    assert_eq!(d.selected, vec!["BTC".to_string(), "ETH".into(), "SOL".into()]);
}
