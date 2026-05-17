// exchange_connector.rs (MODERNİZE EDİLMİŞ HALİ)

use async_trait::async_trait;
use crate::core::types::{Exchange, Trade, Candle, Signal};
use crate::Result;

#[async_trait]
pub trait ExchangeConnector: Send + Sync {
    fn exchange(&self) -> Exchange;
    fn exchange_name(&self) -> &'static str;

    // --- VERİ ÇEKME ---
    async fn fetch_candles(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>>;
    async fn fetch_portfolio(&self) -> Result<Vec<(String, f64)>>;
    async fn get_balance(&self, asset: &str) -> Result<f64>;

    // --- İŞLEM YAPMA ---
    async fn place_order(&self, symbol: &str, side: &str, qty: f64, price: Option<f64>) -> Result<Trade>;

    // --- CANLI AKIŞLAR (STREAM) ---
    // Callback yerine kanal (Sender) kullanacak şekilde finalize edilmeli
    async fn start_signal_stream(&self) -> Result<()>;
    async fn start_position_stream(&self) -> Result<()>;

    // --- GÜVENLİK VE YÖNETİM ---
    fn check_rate_limit(&self, endpoint: &str) -> bool;
    async fn handle_reconnect(&mut self);
}
