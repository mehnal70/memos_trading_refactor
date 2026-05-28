// Delisted-skip → eligibility gate doğrulaması.
//
// Delisted tespit edilen sembol (price_poll/download ardışık fetch hatası → purge)
// symbol_eligible_for_live tarafından reddedilmeli → price_poll/cycle/download/hydrate
// (hepsi bu gate'i kullanır) artık yoklamaz → ApiError storm + "Recovering" sticky biter.

use memos_trading_core::robot::engines::master::{
    is_delisted_skipped, mark_delisted_skip, RuntimeTuning,
};

#[test]
fn delisted_symbol_becomes_ineligible() {
    let t = RuntimeTuning::default();
    // Benzersiz sembol — global skip set process-genelinde; başka teste sızmasın.
    let sym = "ZZDELISTEDGATETESTUSDT";

    assert!(!is_delisted_skipped(sym), "başlangıçta skip listesinde olmamalı");
    assert!(t.symbol_eligible_for_live(sym), "binance USDT → başlangıçta uygun");

    mark_delisted_skip(sym);

    assert!(is_delisted_skipped(sym), "mark sonrası skip listesinde olmalı");
    assert!(!t.symbol_eligible_for_live(sym), "delisted işaretlenince uygun OLMAMALI");
}
