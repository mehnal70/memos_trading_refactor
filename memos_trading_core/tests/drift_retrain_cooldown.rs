// Drift → otomatik ML retrain cooldown davranışının kütüphane-dışı doğrulaması.
//
// tick_intelligence_hub (master.rs) `IntelligenceHub.drift_retrain_armed`
// üzerinden bu kuralı uygular; entegrasyon noktası state-heavy olduğu için
// burada doğrudan hub API'ı test edilir (master.rs cycle uçtan uca live mode
// testlerinde dolaylı doğrulanır).

use memos_trading_core::evolution::{AutonomousController, AutonomousControllerConfig};
use memos_trading_core::robot::ml_engine::intelligence_hub::IntelligenceHub;
use std::time::{Duration, Instant};

fn fresh_hub() -> IntelligenceHub {
    IntelligenceHub::new(AutonomousController::new(AutonomousControllerConfig::default()))
}

#[test]
fn first_fire_is_always_armed() {
    let hub = fresh_hub();
    assert!(hub.drift_retrain_armed(600),
        "ilk fire'da timestamp henüz yok → armed");
}

#[test]
fn second_fire_inside_cooldown_is_blocked() {
    let mut hub = fresh_hub();
    let t0 = Instant::now();
    hub.mark_drift_retrain_fired_at(t0);
    // 5 dk içinde tekrar tetik istendiğinde block.
    let t1 = t0 + Duration::from_secs(300);
    assert!(!hub.drift_retrain_armed_at(t1, 600),
        "cooldown 600s, geçen 300s → armed olmamalı");
}

#[test]
fn rearm_happens_at_or_after_cooldown_boundary() {
    let mut hub = fresh_hub();
    let t0 = Instant::now();
    hub.mark_drift_retrain_fired_at(t0);
    // Tam sınırda (saturating_duration_since(>=cooldown)) → armed
    assert!(hub.drift_retrain_armed_at(t0 + Duration::from_secs(600), 600));
    // Sınırın bir saniye altında → değil
    assert!(!hub.drift_retrain_armed_at(t0 + Duration::from_secs(599), 600));
}

#[test]
fn zero_cooldown_disables_throttle() {
    // ML_DRIFT_COOLDOWN_SECS=0 testleme modunda her tick fire edebilmeli.
    let mut hub = fresh_hub();
    let t0 = Instant::now();
    hub.mark_drift_retrain_fired_at(t0);
    assert!(hub.drift_retrain_armed_at(t0, 0));
    assert!(hub.drift_retrain_armed_at(t0 + Duration::from_millis(1), 0));
}

#[test]
fn mark_after_old_fire_resets_window_to_new_fire() {
    let mut hub = fresh_hub();
    let t0 = Instant::now();
    hub.mark_drift_retrain_fired_at(t0);
    // Bir süre sonra ikinci kez fire — pencere ileri sarmalı.
    let t1 = t0 + Duration::from_secs(700); // cooldown geçti
    assert!(hub.drift_retrain_armed_at(t1, 600));
    hub.mark_drift_retrain_fired_at(t1);
    // Şimdi t1'den sonra yeni cooldown başlar.
    assert!(!hub.drift_retrain_armed_at(t1 + Duration::from_secs(599), 600));
    assert!( hub.drift_retrain_armed_at(t1 + Duration::from_secs(600), 600));
}
