// LiveOrderRefs / live_orders Map Testleri
//
// AppState.finance.live_orders'in mevcut olduğunu, init'te boş başladığını,
// LiveOrderRefs'in serde default'larla geri uyumlu olduğunu doğrular.

use memos_trading_core::core::model::{LiveOrderRefs, RoboticLoopConfig};
use memos_trading_core::robot::robotic_loop::AppState;

#[test]
fn fresh_state_has_empty_live_orders_map() {
    let state = AppState::new(RoboticLoopConfig::default());
    let map = state.finance.live_orders.read().expect("live_orders kilidi");
    assert!(map.is_empty(), "init'te live_orders map boş olmalı, len={}", map.len());
}

#[test]
fn live_order_refs_default_is_serde_compatible() {
    let refs = LiveOrderRefs::default();
    assert!(refs.entry_order_id.is_none());
    assert!(refs.sl_order_id.is_none());
    assert!(refs.tp_order_id.is_none());
    assert!(refs.placed_at.is_empty());

    // JSON round-trip — Android client'ın okuyabileceği format
    let json = serde_json::to_string(&refs).expect("serialize");
    let back: LiveOrderRefs = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(refs.entry_order_id, back.entry_order_id);
}

#[test]
fn live_order_refs_partial_json_decodes_with_serde_default() {
    // Eski snapshot (alanlar yok) — yeni LiveOrderRefs serde default ile parse etmeli
    let partial = r#"{}"#;
    let refs: LiveOrderRefs = serde_json::from_str(partial).expect("partial parse");
    assert!(refs.sl_order_id.is_none());
    assert!(refs.placed_at.is_empty());
}

#[test]
fn live_order_refs_full_round_trip() {
    let refs = LiveOrderRefs {
        entry_order_id: Some("1234567890".into()),
        sl_order_id:    Some("1234567891".into()),
        tp_order_id:    Some("1234567892".into()),
        placed_at:      "2026-05-18T12:00:00Z".into(),
    };
    let json = serde_json::to_string(&refs).unwrap();
    let back: LiveOrderRefs = serde_json::from_str(&json).unwrap();
    assert_eq!(refs.entry_order_id, back.entry_order_id);
    assert_eq!(refs.sl_order_id,    back.sl_order_id);
    assert_eq!(refs.tp_order_id,    back.tp_order_id);
    assert_eq!(refs.placed_at,      back.placed_at);
}

#[test]
fn live_orders_map_supports_concurrent_insert_remove() {
    let state = AppState::new(RoboticLoopConfig::default());

    // Yaz
    {
        let mut map = state.finance.live_orders.write().unwrap();
        map.insert("BTCUSDT".into(), LiveOrderRefs {
            entry_order_id: Some("100".into()),
            sl_order_id:    Some("101".into()),
            tp_order_id:    Some("102".into()),
            placed_at:      "2026-05-18T12:00:00Z".into(),
        });
        map.insert("ETHUSDT".into(), LiveOrderRefs::default());
    }
    // Oku
    {
        let map = state.finance.live_orders.read().unwrap();
        assert_eq!(map.len(), 2);
        let btc = map.get("BTCUSDT").expect("BTC kayıt yok");
        assert_eq!(btc.sl_order_id.as_deref(), Some("101"));
    }
    // Sil
    {
        let mut map = state.finance.live_orders.write().unwrap();
        map.remove("BTCUSDT");
    }
    {
        let map = state.finance.live_orders.read().unwrap();
        assert_eq!(map.len(), 1);
        assert!(map.get("BTCUSDT").is_none());
    }
}
