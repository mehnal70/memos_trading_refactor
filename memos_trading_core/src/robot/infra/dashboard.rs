// robot/dashboard.rs - Gelişmiş Görselleştirme ve Dashboard Entegrasyonu
// Web tabanlı, REST/WS API ile canlı portföy, sinyal, metrik ve model görselleştirme

use crate::core::types::{Trade, Signal};
#[cfg(not(target_arch = "wasm32"))]
use crate::robot::logic::portfolio::PortfolioMetrics;

pub struct Dashboard;

impl Dashboard {
    pub fn show_test_results(results: &serde_json::Value) {
        println!("[DASHBOARD][TEST_RESULTS] {}", results);
    }
    pub fn show_trade(trade: &Trade) {
        // Dummy: Konsola yaz
        println!("[DASHBOARD][TRADE] {} {} {}", trade.symbol, trade.entry_price, trade.pnl.unwrap_or(0.0));
    }
    pub fn show_signal(symbol: &str, signal: &Signal) {
        println!("[DASHBOARD][SIGNAL] {}: {:?}", symbol, signal);
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn show_metrics(metrics: &PortfolioMetrics) {
        println!("[DASHBOARD][METRICS] PnL: {:.2}, WinRate: {:.2}%", metrics.total_pnl, metrics.win_rate * 100.0);
    }
    #[cfg(target_arch = "wasm32")]
    pub fn show_metrics() {
        println!("[DASHBOARD][METRICS] WASM mode - metrics not available");
    }
    // Gerçek uygulamada: WebSocket/REST API ile frontend'e veri gönder
}
