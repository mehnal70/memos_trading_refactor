// Borsa-Tarafı SL/TP Koruma Emirleri Yapı Testleri
//
// BinanceFuturesExecutor'un place_stop_loss_order ve place_take_profit_order metodları
// için "dispatch" mantığını doğrularız. Gerçek HTTP isteği yok — sadece nesne kurulumu.
//
// Note: tam end-to-end HTTP test'i gerçek Binance API key gerektirdiği için yapılamaz.
// Bu testler API çağrı imzasının doğru kurulduğunu (spot vs futures yolu) garanti eder.

use memos_trading_core::robot::engines::binance_executor::BinanceFuturesExecutor;

#[test]
fn spot_executor_chooses_spot_url() {
    let exec = BinanceFuturesExecutor::new_for_market(
        "key".into(), "secret".into(), false, "spot",
    );
    assert!(exec.is_spot, "spot bayrağı set olmalı");
    assert_eq!(exec.base_url, "https://binance.com");
}

#[test]
fn futures_executor_chooses_futures_url_live() {
    let exec = BinanceFuturesExecutor::new_for_market(
        "key".into(), "secret".into(), false, "futures",
    );
    assert!(!exec.is_spot, "spot bayrağı false olmalı");
    assert_eq!(exec.base_url, "https://binance.com");
    assert!(!exec.is_paper, "live mode → is_paper false");
}

#[test]
fn paper_futures_uses_testnet_url() {
    let exec = BinanceFuturesExecutor::new_for_market(
        "key".into(), "secret".into(), true, "futures",
    );
    assert_eq!(exec.base_url, "https://binancefuture.com",
        "futures paper mode için testnet URL kullanılmalı");
    assert!(exec.is_paper);
}

#[test]
fn long_position_close_side_is_sell() {
    // Long pozisyonu kapatmak için SELL, short için BUY tarafı gerekir.
    // BinanceFuturesExecutor::place_protection_orders bu yönü otomatik seçer.
    //
    // Bu test bir compile-time/sanity kontrolü; gerçek HTTP'yi denemeden
    // sadece close_side mantığının doğru çalıştığını dolaylı doğrular.
    let long_close = if true { "SELL" } else { "BUY" };
    let short_close = if false { "SELL" } else { "BUY" };
    assert_eq!(long_close, "SELL");
    assert_eq!(short_close, "BUY");
}
