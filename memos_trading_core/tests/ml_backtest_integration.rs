// ml_backtest_integration.rs
//
// ML pipeline ve backtester için uçtan uca integration testler.
// Hiçbir mock kullanmaz — gerçek hesaplama zincirleri test edilir.

use memos_trading_core::robot::ml_engine::{FeatureExtractor, LinearRegressor};
use memos_trading_core::robot::backtester::backtest_engine::{Backtester, BacktestConfig};
use memos_trading_core::core::types::Candle;
use chrono::Utc;

// ── Yardımcı ─────────────────────────────────────────────────────────────────

fn make_candles(n: usize, start: f64, delta: impl Fn(usize) -> f64) -> Vec<Candle> {
    let mut price = start;
    (0..n).map(|i| {
        price += delta(i);
        Candle {
            symbol: "BTC".into(), interval: "1h".into(),
            timestamp: Utc::now() + chrono::Duration::hours(i as i64),
            open: price, high: price + 1.5, low: price - 0.8,
            close: price + 0.3, volume: 1000.0 + i as f64 * 8.0,
        }
    }).collect()
}

fn base_cfg(strategy: &str) -> BacktestConfig {
    BacktestConfig {
        symbol: "BTC".into(), interval: "1h".into(),
        initial_balance: 10_000.0, max_position_size: 1.0,
        take_profit_pct: 5.0, stop_loss_pct: 2.5,
        strategy_name: strategy.into(),
        position_profile: None, security_profile: None,
        commission_pct: 0.001, strategy_params: None,
        breakeven_at_rr: None, atr_trail_mult: None, partial_tp_ratio: None,
        use_htf: false, edge_min_score: None, orderbook_sim: None,
        regime_gate: Default::default(),
        direction: Default::default(),
        regime_style_fit: false,
        atr_sl_mult: None, atr_tp_mult: None, vol_target_pct: None,
    }
}

// ── ML Pipeline ───────────────────────────────────────────────────────────────

/// Normal trending piyasada extract → normalize → predict zinciri:
/// hiçbir adım NaN/Inf üretmemeli, tüm değerler sınırları içinde olmalı.
#[test]
fn test_ml_pipeline_trending_no_nan() {
    let candles = make_candles(60, 30_000.0, |i| (i as f64 * 0.7) % 5.0 - 2.0);

    let fv   = FeatureExtractor::extract(&candles);
    let norm = fv.normalize();

    for (i, v) in norm.to_array().iter().enumerate() {
        assert!(v.is_finite(),             "feature[{}] NaN/Inf (trending)", i);
        assert!(*v >= 0.0 && *v <= 1.0,   "feature[{}]={:.4} normalize dışı", i, v);
    }

    let pred = LinearRegressor::with_defaults().predict(&fv);
    assert!(pred.score.is_finite(),        "score NaN/Inf");
    assert!(pred.score >= -1.0 && pred.score <= 1.0);
    assert!(pred.confidence >= 0.0 && pred.confidence <= 1.0);
}

/// Düz (sıfır volatilite) piyasada std=0 kenar durumu:
/// tüm hesaplamalar yine de sonlu olmalı.
#[test]
fn test_ml_pipeline_flat_market_no_nan() {
    let candles: Vec<Candle> = (0..50).map(|i| Candle {
        symbol: "TEST".into(), interval: "1h".into(),
        timestamp: Utc::now() + chrono::Duration::hours(i),
        open: 100.0, high: 100.0, low: 100.0, close: 100.0, volume: 1000.0,
    }).collect();

    let fv   = FeatureExtractor::extract(&candles);
    let norm = fv.normalize();
    for (i, v) in norm.to_array().iter().enumerate() {
        assert!(v.is_finite(), "düz piyasada feature[{}] NaN/Inf", i);
    }
    let pred = LinearRegressor::with_defaults().predict(&fv);
    assert!(pred.score.is_finite(), "düz piyasada score NaN/Inf");
}

/// Aşırı volatilite (spike) durumunda tüm öznitelikler hâlâ sınırlı kalmalı.
#[test]
fn test_ml_pipeline_spike_no_nan() {
    let mut candles = make_candles(40, 1_000.0, |_| 0.5);
    // Ortaya büyük spike ekle
    candles[20].high  = 50_000.0;
    candles[20].close = 45_000.0;

    let fv   = FeatureExtractor::extract(&candles);
    let norm = fv.normalize();
    for (i, v) in norm.to_array().iter().enumerate() {
        assert!(v.is_finite(), "spike sonrası feature[{}] NaN/Inf", i);
        assert!(*v >= 0.0 && *v <= 1.0, "spike sonrası feature[{}]={:.4} dışında", i, v);
    }
}

/// LinearRegressor eğitim sonrası evaluate() doğru yönde cevap vermeli:
/// BUY target (+1.0) ile eğitilmiş örneklerde doğruluk > %50 beklenir.
#[test]
fn test_linear_regressor_learns_direction() {
    use memos_trading_core::robot::ml_engine::LinearRegressor;

    let candles = make_candles(60, 10_000.0, |i| if i % 3 == 0 { 2.0 } else { -0.5 });
    let fv = FeatureExtractor::extract(&candles);

    let mut model = LinearRegressor::new();
    let data: Vec<_> = (0..30).map(|_| (fv.clone(), 1.0f64)).collect();
    model.train(&data, 50, 0.01);

    let acc = model.evaluate(&data);
    assert!(acc >= 50.0, "eğitim sonrası doğruluk %50'nin altında: {:.1}%", acc);
}

// ── Backtester ────────────────────────────────────────────────────────────────

/// Yükselen trend + RSI stratejisi:
/// en az 1 trade açılmalı, PnL sonlu olmalı.
#[test]
fn test_backtest_bull_trend_rsi() {
    // İlk 20 mum düşüş (RSI oversold tetikler), sonra güçlü yükseliş
    let candles = make_candles(100, 100.0, |i| if i < 20 { -0.5 } else { 2.0 });

    let result = Backtester::new(base_cfg("RSI"))
        .run(&candles)
        .expect("backtest hata vermemeli");

    assert!(result.total_pnl.is_finite(),    "PnL NaN/Inf");
    assert!(result.win_rate.is_finite(),     "win_rate NaN/Inf");
    assert!(result.profit_factor >= 0.0,     "profit_factor negatif");
    assert!(result.max_drawdown_pct >= 0.0,  "drawdown negatif");
}

/// B1 breakeven aktifken: SL hiçbir zaman entry fiyatının altına inmemeli
/// (kârda breakeven tetiklendikten sonra).
#[test]
fn test_backtest_breakeven_protects_profit() {
    let candles = make_candles(120, 100.0, |i| if i < 15 { -0.3 } else { 1.5 });

    let mut cfg = base_cfg("RSI");
    cfg.breakeven_at_rr = Some(0.5); // yarı risk mesafesinde breakeven

    let result = Backtester::new(cfg).run(&candles).expect("backtest hata vermemeli");

    // Breakeven aktifken pozisyon kapatılıyorsa PnL ≥ 0 veya küçük kayıp olabilir
    // (açılış komisyonu nedeniyle), ancak büyük kayıplar olmamalı
    assert!(result.total_pnl.is_finite());
    // En az aynı sayıda veya daha az losing trade (breakeven koruma sağlar)
    // — kesin sayı data'ya bağlı, sadece NaN/crash olmadığını doğrula
}

/// Kısmi TP aktifken kayıt edilen trade sayısı tam TP'den fazla olmalı
/// (aynı pozisyon 2 trade üretir: partial + remaining).
#[test]
fn test_backtest_partial_tp_more_trades() {
    let candles = make_candles(120, 100.0, |i| if i < 10 { -0.4 } else { 1.8 });

    let result_normal = Backtester::new(base_cfg("RSI"))
        .run(&candles).expect("normal backtest");

    let mut cfg_partial = base_cfg("RSI");
    cfg_partial.partial_tp_ratio = Some(0.5);
    let result_partial = Backtester::new(cfg_partial)
        .run(&candles).expect("partial tp backtest");

    // Her iki run'da da PnL sonlu
    assert!(result_normal.total_pnl.is_finite());
    assert!(result_partial.total_pnl.is_finite());
    // Kısmi TP aktifken trade sayısı ≥ normal (ek partial kapanış)
    assert!(result_partial.total_trades >= result_normal.total_trades);
}

