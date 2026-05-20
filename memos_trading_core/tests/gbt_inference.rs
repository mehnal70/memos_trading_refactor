// GBT inference uçtan uca testi:
// build_training_set → gbt_grid_search → GradientBoostedTrees.train →
// IntelligenceHub.predict_confidence(fv, signal).
//
// Engine cycle entegrasyonu state-heavy olduğundan burada hub API üzerinden
// inference yolu doğrulanır; cycle wiring (master.rs) ml_confidence fallback'ini
// koruduğu için bu pipeline sızıntı yapmaz.

use memos_trading_core::core::types::{Candle, Signal};
use memos_trading_core::evolution::{AutonomousController, AutonomousControllerConfig};
use memos_trading_core::robot::ml_engine::{
    build_training_set, gbt_grid_search, FeatureExtractor, GradientBoostedTrees,
};
use memos_trading_core::robot::ml_engine::intelligence_hub::IntelligenceHub;

fn cs(closes: &[f64]) -> Vec<Candle> {
    closes.iter().map(|&c| Candle {
        open: c, high: c + 0.5, low: c - 0.5, close: c, volume: 100.0,
        ..Default::default()
    }).collect()
}

fn fresh_hub() -> IntelligenceHub {
    IntelligenceHub::new(AutonomousController::new(AutonomousControllerConfig::default()))
}

#[test]
fn full_pipeline_trains_and_predicts_on_uptrend() {
    let candles = cs(&(0..200).map(|i| 100.0 + i as f64 * 0.5).collect::<Vec<_>>());
    let ds = build_training_set(&candles, 20, 5);
    assert!(ds.len() >= 30, "yeterli training örneği bekleniyor: {}", ds.len());

    let mut hub = fresh_hub();
    let tune = gbt_grid_search(&ds).expect("grid search hyperparam üretmeli");
    let mut gbt = GradientBoostedTrees::new(tune.n_estimators, tune.learning_rate, tune.max_depth);
    gbt.train(&ds);
    assert!(gbt.is_ready());
    hub.gbt = gbt;

    let fv = FeatureExtractor::extract(&candles[150..]);
    let conf_buy = hub.predict_confidence(&fv, &Signal::Buy).expect("hazır model → Some");
    let conf_sell = hub.predict_confidence(&fv, &Signal::Sell).expect("hazır model → Some");
    let conf_hold = hub.predict_confidence(&fv, &Signal::Hold).expect("hazır model → Some");
    assert!(conf_buy >= 0.5,
        "Buy + tek yön yukarı → conf ≥ 0.5: {conf_buy}");
    // Simetri: Buy + Sell ≈ 1
    assert!((conf_buy + conf_sell - 1.0).abs() < 1e-9);
    assert!((conf_hold - 0.5).abs() < 1e-9);
    // Aralık
    assert!(conf_buy >= 0.0 && conf_buy <= 1.0);
    assert!(conf_sell >= 0.0 && conf_sell <= 1.0);
}

#[test]
fn untrained_hub_returns_none_for_inference() {
    let hub = fresh_hub();
    let candles = cs(&(0..40).map(|i| 100.0 + i as f64).collect::<Vec<_>>());
    let fv = FeatureExtractor::extract(&candles);
    assert!(hub.predict_confidence(&fv, &Signal::Buy).is_none(),
        "GBT eğitilmemişken None bekleniyor");
}

#[test]
fn grid_search_skips_with_too_few_samples() {
    // 15 örnek < 20 → gbt_grid_search None döner; çağıran sessizce default
    // hyperparam'a düşmeli (cycle dışında).
    let small_data: Vec<([f64; 19], f64)> = (0..15)
        .map(|i| {
            let mut x = [0.0f64; 19];
            x[0] = i as f64;
            (x, if i % 2 == 0 { 1.0 } else { -1.0 })
        })
        .collect();
    assert!(gbt_grid_search(&small_data).is_none());
}

#[test]
fn build_set_then_grid_search_returns_some_for_clean_signal() {
    let candles = cs(&(0..200).map(|i| 100.0 + i as f64).collect::<Vec<_>>());
    let ds = build_training_set(&candles, 20, 5);
    let r = gbt_grid_search(&ds).expect("temiz veri → grid search bir sonuç vermeli");
    assert!(r.oos_accuracy >= 0.0 && r.oos_accuracy <= 100.0);
    assert!(matches!(r.n_estimators, 3 | 5 | 8));
    assert!(matches!(r.max_depth, 2 | 3));
}
