// Faz 4 c4 — PluginRegistry snapshot kütüphane sınırı dışından doğrulama.
//
// Unit testler `src/robot/diagnostics.rs` içinde; buradakiler crate dışından
// snapshot çağrısının çalıştığını ve raporun beklenen şekilleri içerdiğini
// doğrular. Ayrıca Faz 4 c2/c3 plug-in listelerini regresyona karşı sabitler.

use memos_trading_core::robot::diagnostics::PluginRegistry;

#[test]
fn snapshot_from_external_crate_is_healthy() {
    let snap = PluginRegistry::snapshot();
    assert!(snap.is_healthy(),
        "default plug-in seti sağlıklı olmalı: {snap:?}");
}

#[test]
fn snapshot_pins_default_axis_sizes() {
    // Regresyon sabiti — bu sayılar değişirse default chain'ler bilinçli olarak
    // güncellenmiş demektir; testin güncellenmesi de zorunlu olur. Test bunu
    // göze çarpan bir failure ile zorlar.
    let snap = PluginRegistry::snapshot();
    assert_eq!(snap.risk_filters.len(), 3,
        "Faz 4 c1 default risk chain üç filter içermeli");
    assert_eq!(snap.execution_policies.len(), 3,
        "Faz 4 c3 default execution chain üç policy içermeli");
    assert!(snap.strategies.len() >= 15,
        "Faz 4 c2 default registry en az 15 strateji (alias dahil) içermeli");
}

#[test]
fn report_listing_includes_pinned_canonical_names() {
    let snap = PluginRegistry::snapshot();
    let r = snap.report();
    for needle in &[
        "risk_gate", "kelly_edge", "value_at_risk",
        "market_hours", "idle_strategy", "basket_empty",
        "RSI", "MACD", "SUPERTREND", "MA_CROSSOVER",
    ] {
        assert!(r.contains(needle),
            "raporda kanonik ad eksik: {needle}\n--- rapor ---\n{r}");
    }
}

#[test]
fn idle_strategy_policy_is_canon_for_idle_check() {
    // Master.rs cycle'da IDLE check artık IdleStrategyPolicy'ye delege ediliyor;
    // bu test policy davranışının sözleşmesini sabitler — IDLE_PROTECT ve
    // IDLE prefix variantları veto edilmeli, gerçek stratejiler geçmeli.
    use memos_trading_core::robot::execution::IdleStrategyPolicy;
    let p = IdleStrategyPolicy;
    assert!(!p.evaluate_name(Some("IDLE_PROTECT")).is_allow());
    assert!(!p.evaluate_name(Some("idle_anything")).is_allow());
    assert!(p.evaluate_name(Some("SUPERTREND")).is_allow());
    assert!(p.evaluate_name(Some("MA_CROSSOVER")).is_allow());
    assert!(p.evaluate_name(None).is_allow());
}
