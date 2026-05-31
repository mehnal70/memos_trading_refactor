// memos_trading_core/benches/indicators.rs — Faz 3 perf ölçüm altyapısı.
//
// Bağımlılıksız mikro-benchmark (criterion YOK → offline çalışır, harness=false).
// Her cycle (~500ms) sembol başına çağrılan saf indikatör fonksiyonlarının
// ns/op maliyetini ölçer. Amaç: optimizasyonu TAHMİNLE değil ölçümle yönlendirmek
// ([[project_modernization_roadmap]] Faz 3). Çalıştır: `cargo bench -p memos_trading_core`.
//
// Not: Mutlak-doğruluk testi değil, GÖRECELI profil. En pahalı indikatörleri (sıcak
// yol adayları) ortaya çıkarır; sonraki adım yalnız onları ölçüye dayalı optimize etmek.

use std::hint::black_box;
use std::time::Instant;

use chrono::Utc;
use memos_trading_core::core::indicators as ind;
use memos_trading_core::core::types::Candle;

/// Deterministik sahte mum serisi (sinüs + lineer drift). Rastgelelik yok →
/// tekrar çalıştırmalar karşılaştırılabilir. timestamp indikatör matematiğini
/// etkilemediği için sabit bir `Utc::now()` yeterli.
fn make_candles(n: usize) -> Vec<Candle> {
    let ts = Utc::now();
    let mut v = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64;
        let base = 100.0 + t * 0.05 + (t * 0.15).sin() * 8.0;
        let open = base + (t * 0.3).cos() * 1.5;
        let close = base + (t * 0.21).sin() * 1.5;
        let high = open.max(close) + 1.2 + (t * 0.07).sin().abs();
        let low = open.min(close) - 1.2 - (t * 0.11).cos().abs();
        let volume = 1_000.0 + (t * 0.05).sin().abs() * 500.0;
        v.push(Candle {
            timestamp: ts,
            open,
            high,
            low,
            close,
            volume,
            symbol: "BENCH".to_string(),
            interval: "1m".to_string(),
        });
    }
    v
}

/// `f`'i `iters` kez çalıştırır, op başına ortalama nanosaniyeyi döner.
/// Girdi+çıktı black_box ile sabitlenir (derleyici elemesin).
fn bench<T>(iters: u64, mut f: impl FnMut() -> T) -> f64 {
    for _ in 0..(iters / 10).max(1) {
        black_box(f());
    }
    let start = Instant::now();
    for _ in 0..iters {
        black_box(f());
    }
    start.elapsed().as_nanos() as f64 / iters as f64
}

fn main() {
    // Tipik canlı pencere: ~500 mum (cycle_load_candles limiti mertebesinde).
    let candles = make_candles(500);
    let c = &candles;
    let p = 14usize;

    let mut rows: Vec<(&str, f64)> = Vec::new();
    macro_rules! b {
        ($name:expr, $iters:expr, $body:expr) => {
            rows.push(($name, bench($iters, || $body)));
        };
    }

    b!("calculate_sma",            500_000, ind::calculate_sma(black_box(c), p));
    b!("calculate_ema",            200_000, ind::calculate_ema(black_box(c), p));
    b!("calculate_ema_series",     200_000, ind::calculate_ema_series(black_box(c), p));
    b!("calculate_rsi",            200_000, ind::calculate_rsi(black_box(c), p));
    b!("calculate_atr",            200_000, ind::calculate_atr(black_box(c), p));
    b!("calculate_vwap",           200_000, ind::calculate_vwap(black_box(c)));
    b!("calculate_williams_r",     200_000, ind::calculate_williams_r(black_box(c), p));
    b!("calculate_cci",            100_000, ind::calculate_cci(black_box(c), p));
    b!("calculate_adx",            100_000, ind::calculate_adx(black_box(c), p));
    b!("calculate_macd",           100_000, ind::calculate_macd(black_box(c), 12, 26, 9));
    b!("calculate_bollinger",      100_000, ind::calculate_bollinger(black_box(c), 20, 2.0));
    b!("calculate_keltner_channel", 100_000, ind::calculate_keltner_channel(black_box(c), 20, 2.0));
    b!("calculate_stochastic",     100_000, ind::calculate_stochastic(black_box(c), p));
    b!("calculate_supertrend",      50_000, ind::calculate_supertrend(black_box(c), 10, 3.0));
    b!("calculate_parabolic_sar",   50_000, ind::calculate_parabolic_sar(black_box(c), 0.02, 0.2));
    b!("calculate_stochastic_rsi",  50_000, ind::calculate_stochastic_rsi(black_box(c), p, p, 3, 3));

    // En pahalıdan ucuza sırala — sıcak yol adayları üstte.
    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    println!("\n=== indikatör mikro-benchmark (500 mum, ns/op, pahalı→ucuz) ===");
    for (name, ns) in &rows {
        println!("{:>32}  {:>10.1} ns", name, ns);
    }
    println!("=== bitti ({} fonksiyon) ===\n", rows.len());
}
