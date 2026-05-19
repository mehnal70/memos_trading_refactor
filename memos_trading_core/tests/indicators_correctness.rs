// Indicator doğruluk birim testleri — K1-K5 audit bulguları için.
//
// Kapsam:
//   - ATR: Wilder SMMA mı? (TradingView-uyumlu seed + recurrence)
//   - ADX: Final smoothing var mı? (DX değil ADX)
//   - PSAR: Persistent trend rejimi (bar-by-bar flip yok)
//   - Supertrend: Path-dependent bant + close-crossing flip
//   - RSI fast-path ≡ series-path (Wilder)

use memos_trading_core::core::indicators::{
    calculate_adx, calculate_atr, calculate_parabolic_sar, calculate_rsi,
    calculate_supertrend, CoreIndicatorEngine, SupertrendPoint,
};
use memos_trading_core::core::types::Candle;

fn bar(open: f64, high: f64, low: f64, close: f64) -> Candle {
    Candle {
        open, high, low, close, volume: 100.0,
        ..Default::default()
    }
}

// Sabit-aralıklı (high-low = 1.0) ardışık fiyatlar; TR_i = max(1.0, |close_i - close_(i-1)|, ...) = 1.0
// olduğundan ATR sabit 1.0'a yakınsamalı.
fn flat_range_series(n: usize) -> Vec<Candle> {
    (0..n).map(|i| {
        let c = 100.0 + i as f64 * 0.5; // close hafifçe artar; high/low ±0.5 ile çerçeveler.
        bar(c, c + 0.5, c - 0.5, c)
    }).collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// K1: ATR Wilder smoothing
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn atr_constant_range_converges_to_true_range() {
    let candles = flat_range_series(30);
    let atr = calculate_atr(&candles, 14);
    assert!(!atr.is_empty(), "ATR boş döndü");
    // TR her bar 1.0 (sabit) → ATR de 1.0'a çok yakın olmalı (Wilder smoothing değişmez sinyalde sabit kalır).
    let last = *atr.last().unwrap();
    assert!((last - 1.0).abs() < 1e-9, "Sabit TR=1.0 için ATR ≈ 1.0 olmalı: {}", last);
}

#[test]
fn atr_wilder_recurrence_matches_formula() {
    // Wilder formülünü saf olarak hesapla, calculate_atr ile birebir eşleşmeli.
    // Sentetik veri: TR_i = i (1, 2, 3, ...) — varyantlı TR, smoothing'in etkisi görünür.
    // high-low = i; prev_close = 100, close = 100 → |high-prev|, |low-prev| TR'yi etkilemez bu setup'ta.
    let mut candles = vec![bar(100.0, 100.5, 99.5, 100.0)];
    for i in 1..20 {
        let span = i as f64;
        // close sabit 100 → high-low TR olur
        candles.push(bar(100.0, 100.0 + span/2.0, 100.0 - span/2.0, 100.0));
    }
    let period = 5;
    let atr = calculate_atr(&candles, period);

    // Referans Wilder hesabı (TR = high-low çünkü close değişmiyor):
    // TR serisi (bar #0 hariç): 1, 2, 3, ..., 19
    let trs: Vec<f64> = (1..20).map(|i| i as f64).collect();
    let mut expected = trs.iter().take(period).sum::<f64>() / period as f64;
    let mut exp_vec = vec![expected];
    let n_minus_1 = (period - 1) as f64;
    let n_p = period as f64;
    for &tr in trs.iter().skip(period) {
        expected = (expected * n_minus_1 + tr) / n_p;
        exp_vec.push(expected);
    }

    assert_eq!(atr.len(), exp_vec.len(), "Uzunluk eşleşmeli");
    for (i, (a, e)) in atr.iter().zip(exp_vec.iter()).enumerate() {
        assert!((a - e).abs() < 1e-9, "Bar {}: ATR={} beklenen={}", i, a, e);
    }
}

#[test]
fn atr_returns_empty_when_not_enough_data() {
    let c = flat_range_series(5);
    assert!(calculate_atr(&c, 14).is_empty(), "5 bar 14 period için boş dönmeli");
}

// ─────────────────────────────────────────────────────────────────────────────
// K2: ADX final Wilder smoothing
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn adx_returns_empty_when_too_few_bars() {
    let c = flat_range_series(10);
    // period=14 için en az 2*14+1=29 bar gerekir
    assert!(calculate_adx(&c, 14).is_empty());
}

#[test]
fn adx_strong_uptrend_produces_high_value() {
    // Saf yukarı trend: her bar +1, high/low de monoton artıyor.
    let candles: Vec<Candle> = (0..40).map(|i| {
        let c = 100.0 + i as f64 * 1.0;
        bar(c - 0.5, c + 0.5, c - 0.5, c)
    }).collect();
    let adx = calculate_adx(&candles, 14);
    assert!(!adx.is_empty(), "ADX serisi üretilmeli");
    let last = *adx.last().unwrap();
    // Saf uptrend'de ADX 50+ kolayca aşılır (trend gücü yüksek).
    assert!(last > 50.0, "Saf uptrend ADX'i 50+ olmalı: {}", last);
}

#[test]
fn adx_is_smoothed_not_raw_dx() {
    // Bir tek tepe (spike) ekleyip ADX'in DX'e göre daha pürüzsüz olduğunu doğrula:
    // Sıkı bir trend serisinde son barda DX yön değişimi → smoothed ADX'in tepkisi gecikmeli olmalı.
    let mut candles: Vec<Candle> = (0..35).map(|i| {
        let c = 100.0 + i as f64 * 1.0;
        bar(c, c + 0.5, c - 0.5, c)
    }).collect();
    // Son barda yön değişimi yarat (close düşer)
    let last_c = 100.0 + 35.0;
    candles.push(bar(last_c, last_c + 0.5, last_c - 5.0, last_c - 4.0));
    let adx = calculate_adx(&candles, 14);
    // ADX hâlâ yüksek değerde olmalı (smoothing tek-barlık spike'a hızlı tepki vermez)
    let last = *adx.last().unwrap();
    assert!(last > 30.0, "Smoothed ADX tek-spike'a anında çökmemeli: {}", last);
}

// ─────────────────────────────────────────────────────────────────────────────
// K3: Parabolic SAR persistent trend
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn psar_persistent_uptrend_sar_below_lows() {
    // Saf yukarı trend — PSAR her zaman barın low'unun altında kalmalı.
    let candles: Vec<Candle> = (0..30).map(|i| {
        let c = 100.0 + i as f64 * 1.0;
        bar(c - 0.5, c + 0.5, c - 0.5, c)
    }).collect();
    let psar = calculate_parabolic_sar(&candles, 0.02, 0.2);
    assert_eq!(psar.len(), candles.len());

    // İlk barı atla (initial seed), kalan tüm barlarda PSAR ≤ bar.low (uptrend).
    for i in 1..candles.len() {
        assert!(psar[i] <= candles[i].low + 1e-9,
            "Bar {} uptrend'de PSAR low'un üstüne çıkmamalı: psar={} low={}",
            i, psar[i], candles[i].low);
    }
}

#[test]
fn psar_persistent_downtrend_sar_above_highs() {
    let candles: Vec<Candle> = (0..30).map(|i| {
        let c = 100.0 - i as f64 * 1.0;
        bar(c + 0.5, c + 0.5, c - 0.5, c)
    }).collect();
    let psar = calculate_parabolic_sar(&candles, 0.02, 0.2);
    for i in 1..candles.len() {
        assert!(psar[i] >= candles[i].high - 1e-9,
            "Bar {} downtrend'de PSAR high'ın altına düşmemeli: psar={} high={}",
            i, psar[i], candles[i].high);
    }
}

#[test]
fn psar_flips_only_on_extreme_violation() {
    // Karışık veri ama trend belirgin başlıyor — bar-by-bar her durumda flip
    // olmamalı (eski yanlış implementasyon her bar flip ediyordu).
    // 10 bar uptrend + 1 minik düşüş + 9 bar daha uptrend.
    let mut candles: Vec<Candle> = (0..10).map(|i| {
        let c = 100.0 + i as f64;
        bar(c, c + 0.4, c - 0.4, c)
    }).collect();
    // Minik dip: close hafifçe düşüyor ama low PSAR'ın çok altına inmiyor
    let c = 109.5;
    candles.push(bar(c, c + 0.4, c - 0.4, c));
    for i in 0..9 {
        let c = 110.0 + i as f64;
        candles.push(bar(c, c + 0.4, c - 0.4, c));
    }
    let psar = calculate_parabolic_sar(&candles, 0.02, 0.2);

    // Son bar'da hâlâ uptrend olduğunu doğrula: PSAR son barın low'unun altında.
    let last = psar.len() - 1;
    assert!(psar[last] < candles[last].low,
        "Trend uptrend olarak kalmalıydı (eski sürüm her bar flip ediyordu): psar={} low={}",
        psar[last], candles[last].low);
}

// ─────────────────────────────────────────────────────────────────────────────
// K4: Supertrend path-dependent
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn supertrend_uptrend_keeps_trend_positive() {
    let candles: Vec<Candle> = (0..40).map(|i| {
        let c = 100.0 + i as f64 * 1.0;
        bar(c - 0.3, c + 0.5, c - 0.5, c)
    }).collect();
    let st = calculate_supertrend(&candles, 10, 3.0);
    assert!(!st.is_empty());
    // Son birkaç bar uptrend (+1) olmalı.
    let last_few: Vec<i8> = st.iter().rev().take(5).map(|p| p.trend).collect();
    assert!(last_few.iter().all(|&t| t == 1), "Saf uptrend'de son barlar +1 olmalı: {:?}", last_few);
}

#[test]
fn supertrend_flips_on_strong_reversal() {
    // 20 bar uptrend + 20 bar sert downtrend; supertrend mutlaka -1'e flip etmeli.
    let mut candles: Vec<Candle> = (0..20).map(|i| {
        let c = 100.0 + i as f64 * 1.0;
        bar(c - 0.3, c + 0.5, c - 0.5, c)
    }).collect();
    for i in 0..20 {
        // Reversal: 120 → 80 doğrultusunda düşüş
        let c = 120.0 - i as f64 * 2.0;
        candles.push(bar(c + 0.3, c + 0.5, c - 0.5, c));
    }
    let st = calculate_supertrend(&candles, 10, 3.0);
    let last = st.last().unwrap();
    assert_eq!(last.trend, -1, "Sert downtrend sonunda trend -1 olmalı: {:?}", last);
}

#[test]
fn supertrend_value_is_active_band() {
    // Uptrend: value = lower band (close'un altında); Downtrend: value = upper band (close'un üstünde).
    let candles: Vec<Candle> = (0..40).map(|i| {
        let c = 100.0 + i as f64 * 1.0;
        bar(c - 0.3, c + 0.5, c - 0.5, c)
    }).collect();
    let st = calculate_supertrend(&candles, 10, 3.0);
    let last_idx = st.len() - 1;
    let candle_offset = candles.len() - st.len();
    let last_close = candles[candle_offset + last_idx].close;
    let SupertrendPoint { trend, value } = st[last_idx];
    if trend == 1 {
        assert!(value < last_close,
            "Uptrend value (lower band) close'un altında olmalı: value={} close={}",
            value, last_close);
    } else {
        assert!(value > last_close,
            "Downtrend value (upper band) close'un üstünde olmalı: value={} close={}",
            value, last_close);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// K5: RSI fast-path ≡ series-path
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn rsi_fast_path_matches_series_path_last_value() {
    // Çeşitli desenler için son barda fast ve series sonuçları bit-bit eşit olmalı.
    let datasets: Vec<Vec<Candle>> = vec![
        // (1) Monoton artış
        (0..40).map(|i| bar(0.0, 0.0, 0.0, 100.0 + i as f64)).collect(),
        // (2) Monoton azalış
        (0..40).map(|i| bar(0.0, 0.0, 0.0, 200.0 - i as f64)).collect(),
        // (3) Zikzak (her bar yön değişir)
        (0..40).map(|i| bar(0.0, 0.0, 0.0, 100.0 + if i % 2 == 0 { 1.0 } else { -0.5 })).collect(),
        // (4) Sabit fiyat (zero division riski)
        (0..40).map(|_| bar(0.0, 0.0, 0.0, 100.0)).collect(),
    ];

    for (idx, candles) in datasets.iter().enumerate() {
        let series = calculate_rsi(candles, 14);
        let fast   = CoreIndicatorEngine::rsi(candles, 14);
        let series_last = *series.last().expect("series boş");
        assert!((fast - series_last).abs() < 1e-9,
            "Dataset {}: fast={} series_last={}", idx, fast, series_last);
    }
}

#[test]
fn rsi_fast_path_returns_50_when_not_enough_data() {
    // Wilder seed için n > period gerekir; eşit/azsa nötr 50.
    let c = (0..10).map(|i| bar(0.0, 0.0, 0.0, 100.0 + i as f64)).collect::<Vec<_>>();
    assert_eq!(CoreIndicatorEngine::rsi(&c, 14), 50.0);
}

#[test]
fn rsi_fast_path_returns_100_when_no_losses() {
    // Sıfır kayıp serisinde RSI = 100.
    let c: Vec<Candle> = (0..40).map(|i| bar(0.0, 0.0, 0.0, 100.0 + i as f64)).collect();
    assert_eq!(CoreIndicatorEngine::rsi(&c, 14), 100.0);
}
