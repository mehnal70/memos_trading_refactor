//! Risk katmanı — memos_quant'a taşındı (dikey-dilim refactor'ü, Adım 3b).
//!
//! Gövde `memos_quant::risk` içinde BİREBİR yaşıyor. Burada iki şey var:
//!   1) Yeniden-yayın kabuğu → `crate::robot::risk::{RiskManager, risk_gate::*, filter::*}`
//!      gibi mevcut yollar AYNEN çalışır.
//!   2) `RiskSnapshot` köprüsü → risk zinciri artık `MissionControl` tipini tanımıyor;
//!      okuduğu BEŞ değeri bu impl sağlar. Okunan alanlar birebir eskisiyle aynı.
pub use memos_quant::risk::*;

/// Motor snapshot'ını piyasa-agnostik risk sözleşmesine bağlar.
///
/// ⚠️ Bu impl'deki alan eşlemeleri risk kararlarını (DD / günlük zarar / VaR / Kelly
/// edge) doğrudan belirler. Değiştirmek canlı risk davranışını değiştirir.
impl memos_quant::risk::RiskSnapshot for crate::core::model::MissionControl {
    fn total_equity(&self) -> f64 {
        self.finance.total_equity
    }
    fn starting_capital(&self) -> f64 {
        self.finance.starting_capital
    }
    fn peak_equity(&self) -> f64 {
        self.charts.peak_equity
    }
    fn trade_pnls(&self) -> Vec<f64> {
        self.trade_history.iter().map(|t| t.pnl).collect()
    }
    fn trade_returns(&self) -> Vec<f64> {
        self.trade_history
            .iter()
            .map(|t| t.pnl_pct / 100.0)
            .collect()
    }
}

#[cfg(test)]
mod snapshot_bridge_tests {
    use super::*;
    use crate::core::model::{ClosedTradeModel, RoboticLoopConfig};
    use memos_quant::risk::RiskSnapshot;

    fn trade(pnl: f64, pnl_pct: f64) -> ClosedTradeModel {
        ClosedTradeModel {
            symbol: "BTCUSDT".into(),
            is_long: true,
            pnl,
            pnl_pct,
            exit_reason: "TAKE_PROFIT".into(),
            closed_at: "2026-05-18T01:00:00Z".into(),
            opened_at: "2026-05-18T00:00:00Z".into(),
            leverage: 1.0,
            net_pnl: pnl,
            net_pnl_pct: pnl_pct,
            commission: 0.0,
            entry_price: 100.0,
            exit_price: 101.0,
            strategy: "RSI".into(),
            venue: "binance".into(),
            pyramid_tranches: 1,
            exchange_ref: String::new(),
        }
    }

    /// `AppState::new` MT5 venue'sü listede olduğunda `registry.rs`'te TCP portunu
    /// ERKEN bağlar (`block_in_place` + `Handle::current()`), bu yüzden senkron ya da
    /// tek-thread'li testte panikler. `VENUES=binance` ile MT5 kaydı devre dışı kalır;
    /// multi_thread bayrağı env yarışına karşı ikinci emniyet.
    fn snapshot_for_test() -> crate::core::model::MissionControl {
        std::env::set_var("VENUES", "binance");
        let state = crate::robot::robotic_loop::AppState::new(RoboticLoopConfig::default());
        crate::core::bridge::get_snapshot(&state)
    }

    /// Köprünün BEŞ alan eşlemesi doğru olmalı. Her alana FARKLI değer verilir —
    /// varsayılan snapshot'ta equity/starting/peak aynı olabildiği için eşit
    /// değerlerle test edilse yanlış eşleme (örn. peak↔starting takası) sessiz kalırdı.
    #[tokio::test(flavor = "multi_thread")]
    async fn bridge_maps_each_field_to_the_right_source() {
        let mut snap = snapshot_for_test();
        snap.finance.total_equity = 111.0;
        snap.finance.starting_capital = 222.0;
        snap.charts.peak_equity = 333.0;
        snap.trade_history = vec![trade(10.0, 1.0), trade(-20.0, -2.0)];

        assert_eq!(snap.total_equity(), 111.0, "total_equity yanlış alandan");
        assert_eq!(
            snap.starting_capital(),
            222.0,
            "starting_capital yanlış alandan"
        );
        assert_eq!(snap.peak_equity(), 333.0, "peak_equity yanlış alandan");
        assert_eq!(snap.trade_pnls(), vec![10.0, -20.0], "pnl serisi");
        assert_eq!(
            snap.trade_returns(),
            vec![0.01, -0.02],
            "pnl_pct/100 serisi"
        );
    }

    /// Zincir MissionControl üzerinden uçtan uca çalışmalı (dyn coercion + karar).
    #[tokio::test(flavor = "multi_thread")]
    async fn chain_denies_on_drawdown_through_mission_control() {
        let mut snap = snapshot_for_test();
        let m = RiskManager::new();
        assert_eq!(m.filters.len(), 3, "default zincir 3 filtre");

        // Zirveden %20 düşüş → risk_gate max_drawdown_pct (10.0) aşılır → Deny + halt.
        snap.finance.starting_capital = 10_000.0;
        snap.charts.peak_equity = 10_000.0;
        snap.finance.total_equity = 8_000.0;
        let d = m.authorize(&memos_venue_api::Signal::Buy, &snap, 0.9, 100.0);
        match d {
            memos_quant::risk::risk_gate::RiskDecision::Deny {
                halt, ref reasons, ..
            } => {
                assert!(halt, "kritik DD halt üretmeli");
                assert!(
                    reasons[0].starts_with("[risk_gate]"),
                    "filtre adı etiketlenmeli: {reasons:?}"
                );
            }
            other => panic!("DD aşımında Deny beklenirdi, got {other:?}"),
        }
    }
}
