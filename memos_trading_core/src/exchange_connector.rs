// exchange_connector.rs
// Çoklu borsa canlı veri ve işlem entegrasyonu için ana trait/interface
// Her borsa (Binance, Bybit, Kucoin, Coinbase, vs.) için adapter bu trait'i implement eder
// Spot/futures, websocket/REST, fiyat, sinyal, pozisyon, risk, emir, portföy veri tiplerini kapsar

use async_trait::async_trait;
use crate::types::{Exchange, Trade, Signal};

#[async_trait]
pub trait ExchangeConnector: Send + Sync {
    /// Borsa adı (ör: Binance, Bybit)
    fn exchange(&self) -> Exchange;

    /// Websocket ile canlı fiyat akışını başlat
    async fn start_price_stream(&mut self, symbol: &str, on_price: Box<dyn Fn(f64) + Send + Sync>);

    /// Websocket ile canlı sinyal akışını başlat
    async fn start_signal_stream(&mut self, symbol: &str, on_signal: Box<dyn Fn(Signal) + Send + Sync>);

    /// Websocket ile pozisyon/portföy güncelleme akışını başlat
    async fn start_position_stream(&mut self, account_id: &str, on_position: Box<dyn Fn(Trade) + Send + Sync>);

    /// REST ile anlık fiyat al
    async fn fetch_price(&self, symbol: &str) -> Result<f64, String>;

    /// REST ile portföy/bakiye al
    async fn fetch_portfolio(&self, account_id: &str) -> Result<Vec<Trade>, String>;

    /// Emir gönder (REST)
    async fn send_order(&self, order: &Trade) -> Result<String, String>;

    /// Rate limit ve ban koruması (her istekte otomatik uygulanır)
    fn check_rate_limit(&self, endpoint: &str) -> bool;

    /// Otomatik reconnect, proxy, IP rotasyonu gibi koruma mekanizmaları
    async fn handle_reconnect(&mut self);
}
