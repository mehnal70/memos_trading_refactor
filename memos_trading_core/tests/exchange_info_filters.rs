// ExchangeInfo / LOT_SIZE / MIN_NOTIONAL Filtre Testleri
//
// SymbolFilters'ın stepSize yuvarlama, minQty / minNotional sınırları ve
// tickSize fiyat hizalama mantığını doğrular. Gerçek HTTP yok — saf matematik.

use memos_trading_core::core::model::SymbolFilters;
use memos_trading_core::robot::engines::binance_executor::BinanceFuturesExecutor;

fn btc_like() -> SymbolFilters {
    // BTCUSDT futures için tipik değerler.
    SymbolFilters {
        step_size: 0.001,
        min_qty: 0.001,
        tick_size: 0.10,
        min_notional: 5.0,
    }
}

#[test]
fn step_size_qty_yuvarlama_asagi_yapilir() {
    let f = btc_like();
    // 0.12345 → 0.123 (stepSize 0.001 → 3 ondalık)
    assert!((f.round_qty_down(0.12345) - 0.123).abs() < 1e-9);
    // Tam katsa olduğu gibi kalır
    assert!((f.round_qty_down(0.500) - 0.500).abs() < 1e-9);
    // Çok küçükse 0'a düşer
    assert!((f.round_qty_down(0.0005) - 0.0).abs() < 1e-9);
}

#[test]
fn step_size_sifirsa_qty_aynen_doner() {
    let f = SymbolFilters { step_size: 0.0, min_qty: 0.0, tick_size: 0.0, min_notional: 0.0 };
    assert!((f.round_qty_down(0.12345) - 0.12345).abs() < 1e-9);
}

#[test]
fn tick_size_fiyati_en_yakina_yuvarlar() {
    let f = btc_like();
    // tickSize 0.10 → 67890.234 → 67890.20
    assert!((f.round_price(67890.234) - 67890.2).abs() < 1e-6);
    // 67890.260 → 67890.30
    assert!((f.round_price(67890.26) - 67890.3).abs() < 1e-6);
}

#[test]
fn validate_minqty_altinda_reddeder() {
    let f = btc_like();
    // 0.0005 < stepSize 0.001 → yuvarlandığında 0 olur → red
    // (stepSize sonrası qty=0 hatası, minQty kontrolünden önce yakalanır)
    let res = f.validate(0.0005, 30000.0);
    assert!(res.is_err(), "stepSize altı kabul edilmemeli");
    let err = res.unwrap_err();
    assert!(err.contains("qty=0") || err.contains("minQty"),
        "hata sebebi qty=0 veya minQty olmalı: {}", err);
}

#[test]
fn validate_minnotional_altinda_reddeder() {
    let f = btc_like();
    // qty 0.001 × price 1000 = $1 < minNotional $5 → red
    let res = f.validate(0.001, 1000.0);
    assert!(res.is_err(), "minNotional altı kabul edilmemeli");
    let err = res.unwrap_err();
    assert!(err.contains("minNotional"), "hata sebebi minNotional içermeli: {}", err);
}

#[test]
fn validate_basarili_durumda_yuvarlanmis_qty_doner() {
    let f = btc_like();
    // qty 0.12345 → 0.123, $0.123 × $30000 = $3690 > minNotional → OK
    let res = f.validate(0.12345, 30000.0);
    assert!(res.is_ok(), "filtre geçmeli: {:?}", res);
    let q = res.unwrap();
    assert!((q - 0.123).abs() < 1e-9, "stepSize'a yuvarlanmış qty döner: {}", q);
}

#[test]
fn validate_stepsize_sonrasi_sifir_olursa_reddeder() {
    let f = btc_like();
    // 0.0009 → stepSize'a yuvarlanınca 0 → red
    let res = f.validate(0.0009, 30000.0);
    assert!(res.is_err(), "stepSize sonrası 0 olursa red");
}

#[test]
fn executor_filtre_cachei_bos_baslar() {
    let exec = BinanceFuturesExecutor::new_for_market(
        "k".into(), "s".into(), false, "futures",
    );
    let cache = exec.filters.read().unwrap();
    assert!(cache.is_empty(), "yeni executor'un filtre cache'i boş olmalı");
}

#[test]
fn cache_yazma_okuma_calisir() {
    let exec = BinanceFuturesExecutor::new_for_market(
        "k".into(), "s".into(), false, "futures",
    );
    // Cache'e elden yaz (gerçek HTTP yerine simülasyon)
    {
        let mut w = exec.filters.write().unwrap();
        w.insert("BTCUSDT".into(), btc_like());
    }
    // Oku
    let f = exec.filters.read().unwrap();
    let bt = f.get("BTCUSDT").expect("BTCUSDT cache'de olmalı");
    assert!((bt.step_size - 0.001).abs() < 1e-9);
    assert!((bt.min_notional - 5.0).abs() < 1e-9);
}
