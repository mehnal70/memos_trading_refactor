// Pozisyon Çıkış Koşulları Testleri (SL / TP / Trailing / Breakeven)
//
// `Engine::check_exit_conditions` saf bir fonksiyondur — engine veya tokio gerektirmez,
// borsaya bağlanmaz. Mock pozisyonla farklı fiyat senaryolarını oynatıp dönen
// ExitReason'ı doğrularız.

use memos_trading_core::core::model::PositionModel;
use memos_trading_core::robot::engines::master::{Engine, ExitReason};

fn long_position(entry: f64, sl: f64, tp: f64, trail: f64) -> PositionModel {
    PositionModel {
        pos_id: String::new(),
        symbol: "BTCUSDT".into(),
        entry_price: entry,
        current_price: entry,
        qty: 0.1,
        leverage: 1.0, market: "spot".into(), interval: "1m".into(),
        is_long: true,
        trade_type: "LONG".into(),
        opened_at: "2026-05-18T00:00:00Z".into(),
        stop_loss: sl,
        take_profit: tp,
        trailing_stop: trail,
        max_favorable_price: entry,
        breakeven_activated: false,
        kind: None,
        entry_commission: 0.0,
    }
}

fn short_position(entry: f64, sl: f64, tp: f64, trail: f64) -> PositionModel {
    PositionModel {
        pos_id: String::new(),
        symbol: "BTCUSDT".into(),
        entry_price: entry,
        current_price: entry,
        qty: 0.1,
        leverage: 1.0, market: "spot".into(), interval: "1m".into(),
        is_long: false,
        trade_type: "SHORT".into(),
        opened_at: "2026-05-18T00:00:00Z".into(),
        stop_loss: sl,
        take_profit: tp,
        trailing_stop: trail,
        max_favorable_price: entry,
        breakeven_activated: false,
        kind: None,
        entry_commission: 0.0,
    }
}

#[test]
fn long_stop_loss_triggers_when_price_drops_below_sl() {
    let mut pos = long_position(100.0, 98.0, 105.0, 0.0);
    // Trailing kapalı (atr=0). Fiyat 97.5 → SL 98 altına düştü.
    let reason = Engine::check_exit_conditions(&mut pos, 97.5, 0.0, 0.0, 1.0);
    assert_eq!(reason, Some(ExitReason::StopLoss));
}

#[test]
fn long_take_profit_triggers_when_price_rises_to_tp() {
    let mut pos = long_position(100.0, 98.0, 105.0, 0.0);
    let reason = Engine::check_exit_conditions(&mut pos, 105.5, 0.0, 0.0, 1.0);
    assert_eq!(reason, Some(ExitReason::TakeProfit));
}

#[test]
fn short_stop_loss_triggers_when_price_rises_above_sl() {
    let mut pos = short_position(100.0, 102.0, 95.0, 0.0);
    let reason = Engine::check_exit_conditions(&mut pos, 102.5, 0.0, 0.0, 1.0);
    assert_eq!(reason, Some(ExitReason::StopLoss));
}

#[test]
fn short_take_profit_triggers_when_price_drops_to_tp() {
    let mut pos = short_position(100.0, 102.0, 95.0, 0.0);
    let reason = Engine::check_exit_conditions(&mut pos, 94.5, 0.0, 0.0, 1.0);
    assert_eq!(reason, Some(ExitReason::TakeProfit));
}

#[test]
fn breakeven_activates_after_one_rr_gain_then_protects_entry() {
    // Long entry=100, SL=98 → risk=2.0. breakeven_rr=1.0 ⇒ kazanç 2.0'e ulaşınca SL = entry.
    let mut pos = long_position(100.0, 98.0, 110.0, 0.0);
    // Fiyat 102 → kazanç 2.0 == 1×risk → breakeven aktive
    let r = Engine::check_exit_conditions(&mut pos, 102.0, 0.0, 0.0, 1.0);
    assert!(r.is_none(), "breakeven aktivasyonu erken kapanışa neden olmamalı: {:?}", r);
    assert!(pos.breakeven_activated, "breakeven_activated true olmalı");
    assert_eq!(pos.stop_loss, 100.0, "SL entry'e taşınmalı, mevcut: {}", pos.stop_loss);

    // Şimdi fiyat 99.5'e düştü → entry (100) altına ⇒ Breakeven exit (kar değil, koruma).
    let r2 = Engine::check_exit_conditions(&mut pos, 99.5, 0.0, 0.0, 1.0);
    assert_eq!(r2, Some(ExitReason::Breakeven),
        "entry altına düştüğünde Breakeven dönmeli, döndü: {:?}", r2);
}

#[test]
fn trailing_stop_locks_in_profits_for_long() {
    // Long entry=100, SL=95, TP=200 (uzak), trailing baş=0 → ilk fiyat hareketinde set olur.
    let mut pos = long_position(100.0, 95.0, 200.0, 0.0);
    let atr = 1.0;        // ATR
    let mult = 2.0;       // 2×ATR uzak trailing
    // Fiyat 110.0 → max_favorable=110, trailing=108. Henüz çıkış yok.
    let r1 = Engine::check_exit_conditions(&mut pos, 110.0, atr, mult, 0.0); // breakeven kapalı
    assert!(r1.is_none(), "yükselişte trailing tetiklenmemeli: {:?}", r1);
    assert!(pos.trailing_stop >= 108.0 - 0.001 && pos.trailing_stop <= 108.0 + 0.001,
        "trailing ≈ 108 olmalı, gerçek: {}", pos.trailing_stop);
    assert_eq!(pos.max_favorable_price, 110.0);

    // Fiyat 115.0 → max_favorable=115, trailing=113.
    let r2 = Engine::check_exit_conditions(&mut pos, 115.0, atr, mult, 0.0);
    assert!(r2.is_none());
    assert!((pos.trailing_stop - 113.0).abs() < 0.001);

    // Fiyat 112.8 → trailing 113 altına düştü → TrailingStop.
    let r3 = Engine::check_exit_conditions(&mut pos, 112.8, atr, mult, 0.0);
    assert_eq!(r3, Some(ExitReason::TrailingStop));
}

#[test]
fn trailing_stop_locks_in_profits_for_short() {
    // Short entry=100, SL=105, TP=0 (uzak), trailing=0
    let mut pos = short_position(100.0, 105.0, 50.0, 0.0);
    let atr = 1.0;
    let mult = 2.0;
    // Fiyat 90 → max_favorable=90 (en düşük), trailing=92
    let r1 = Engine::check_exit_conditions(&mut pos, 90.0, atr, mult, 0.0);
    assert!(r1.is_none(), "short düşüşünde trailing tetiklenmemeli: {:?}", r1);
    assert!((pos.trailing_stop - 92.0).abs() < 0.001,
        "short trailing ≈ 92 olmalı, gerçek: {}", pos.trailing_stop);

    // Fiyat geri 92.5'e çıktı → trailing üstüne çıktı → çıkış
    let r2 = Engine::check_exit_conditions(&mut pos, 92.5, atr, mult, 0.0);
    assert_eq!(r2, Some(ExitReason::TrailingStop));
}

#[test]
fn no_exit_when_price_inside_band() {
    let mut pos = long_position(100.0, 95.0, 110.0, 0.0);
    let r = Engine::check_exit_conditions(&mut pos, 102.0, 0.0, 0.0, 1.0);
    // 1×RR (=5) kazanç değil, BE aktivasyonu yok; SL/TP da tetiklenmemeli.
    // Not: r None olabilir veya BE aktivasyonu olmadan None olmalı. risk=5, gain=2 < 5 → None
    assert!(r.is_none(), "fiyat bant içinde olduğu için çıkış olmamalı: {:?}", r);
    assert!(!pos.breakeven_activated);
}
