    pub fn get_test_results() -> serde_json::Value {
        use std::fs;
        use std::path::Path;
        let test_results_path = "../test_results.json";
        if Path::new(test_results_path).exists() {
            let data = fs::read_to_string(test_results_path).unwrap_or_else(|_| "[]".to_string());
            serde_json::from_str(&data).unwrap_or_else(|_| serde_json::json!([]))
        } else {
            serde_json::json!([])
        }
    }
// robot/api.rs - REST/WS API/Servis Katmanı
// Trade, portföy, sinyal, metrik, model ve kullanıcı işlemleri için endpointler

use crate::types::{Trade, Signal};
#[cfg(not(target_arch = "wasm32"))]
use crate::robot::PortfolioMetrics;

pub struct ApiService;

impl ApiService {
    pub fn get_trade(&self, _trade_id: u64) -> Option<Trade> {
        // Dummy: None
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn get_portfolio_metrics(&self) -> PortfolioMetrics {
        // Dummy: Default metrik
        PortfolioMetrics::default()
    }
    #[cfg(target_arch = "wasm32")]
    pub fn get_portfolio_metrics(&self) {
        // WASM için dummy
    }
    pub fn post_signal(&self, symbol: &str, signal: Signal) {
        println!("[API] Signal for {}: {:?}", symbol, signal);
    }
    pub fn get_user_profile(&self, user_id: &str) {
        println!("[API] Get user profile: {}", user_id);
    }
    // Gerçek uygulamada: actix-web, warp, axum, ws ile endpointler
}
