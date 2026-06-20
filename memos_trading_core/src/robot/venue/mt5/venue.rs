//! `Mt5Venue` — `VenueAdapter`'ın MetaTrader 5 implementasyonu (forex/emtia/endeks CFD).
//!
//! **Faz 1 = veri venue'su:** `fetch_candles`/`book_ticker`/`symbol_filters` yerel MT5 EA
//! köprüsüne ([`Mt5Bridge`]) gider — yeni piyasada önce izole edge ölçümü duvarı
//! ([[project_world_markets]]).
//!
//! **Faz 2 = yürütme:** `submit_order`/`cancel_all`/`set_leverage`/`balance` de aynı köprüye
//! gider (protokol `req_*`/`parse_*` ile). Gerçek emir gönderimi EA tarafında `InpEnableExec`
//! ile kapılıdır; canlı MT5 yönlendirmesi ayrıca edge doğrulamasına bağlıdır. Sahte başarı
//! DÖNMEZ — EA `ok:false`/timeout verirse açık `Err` döner.
//!
//! Sembol biçimi (EURUSD/XAUUSD) BIST equity heuristic'iyle çakışır → MT5 sembolleri
//! `Exchange::classify` ile OTO-sınıflanmaz; explicit routing (`SYM@mt5`) ile kullanılır.

use std::sync::Arc;

use async_trait::async_trait;

use crate::core::model::SymbolFilters;
use crate::core::types::{Candle, Exchange, Market};
use crate::robot::venue::adapter::{MarketData, OrderExecution, VenueAdapter};
use crate::robot::venue::mt5::bridge::Mt5Bridge;
use crate::robot::venue::mt5::protocol;
use crate::robot::venue::types::{OrderReceipt, OrderRequest};
use crate::Result;

pub struct Mt5Venue {
    market: Market,
    bridge: Arc<Mt5Bridge>,
}

impl Mt5Venue {
    /// Verilen köprü üzerinden MT5 venue (market = futures/spot; CFD'ler çoğunlukla "spot"
    /// muamelesi görür, short ayrımı için Futures verilebilir).
    pub fn new(market: Market, bridge: Arc<Mt5Bridge>) -> Self {
        Self { market, bridge }
    }

    /// Köprüye erişim (paylaşımlı; aynı EA bağlantısını tüm semboller kullanır).
    pub fn bridge(&self) -> &Arc<Mt5Bridge> {
        &self.bridge
    }
}

#[async_trait]
impl MarketData for Mt5Venue {
    async fn fetch_candles(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>> {
        let id = self.bridge.next_id();
        let line = protocol::req_candles(id, symbol, interval, limit.clamp(1, 5000));
        let resp = self.bridge.request(&line).await?;
        protocol::parse_candles(symbol, interval, &resp)
    }

    async fn book_ticker(&self, symbol: &str) -> Result<(f64, f64)> {
        let id = self.bridge.next_id();
        let line = protocol::req_tick(id, symbol);
        let resp = self.bridge.request(&line).await?;
        protocol::parse_tick(symbol, &resp)
    }

    async fn symbol_filters(&self, symbol: &str) -> Result<SymbolFilters> {
        let id = self.bridge.next_id();
        let line = protocol::req_filters(id, symbol);
        let resp = self.bridge.request(&line).await?;
        protocol::parse_filters(symbol, &resp)
    }
}

#[async_trait]
impl OrderExecution for Mt5Venue {
    async fn submit_order(&self, req: &OrderRequest) -> Result<OrderReceipt> {
        let id = self.bridge.next_id();
        let line = protocol::req_order(id, req);
        let resp = self.bridge.request(&line).await?;
        protocol::parse_order(&req.symbol, &resp)
    }
    async fn cancel_all(&self, symbol: &str) -> Result<()> {
        let id = self.bridge.next_id();
        let line = protocol::req_cancel_all(id, symbol);
        let resp = self.bridge.request(&line).await?;
        protocol::parse_ack(symbol, &resp)
    }
    async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<()> {
        let id = self.bridge.next_id();
        let line = protocol::req_set_leverage(id, symbol, leverage);
        let resp = self.bridge.request(&line).await?;
        protocol::parse_ack(symbol, &resp)
    }
    async fn balance(&self) -> Result<f64> {
        let id = self.bridge.next_id();
        let line = protocol::req_balance(id);
        let resp = self.bridge.request(&line).await?;
        protocol::parse_balance(&resp)
    }
}

impl VenueAdapter for Mt5Venue {
    fn exchange(&self) -> Exchange {
        Exchange::Mt5
    }
    fn market(&self) -> Market {
        self.market
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::AssetClass;
    use crate::robot::venue::types::{OrderSide, OrderStatus};
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpStream;

    #[test]
    fn identity_is_mt5_forex() {
        let v = Mt5Venue::new(Market::Spot, Arc::new(Mt5Bridge::with_defaults(None)));
        assert_eq!(v.exchange(), Exchange::Mt5);
        assert_eq!(v.asset_class(), AssetClass::Forex);
        assert!(v.has_live_feed());
        assert_eq!(v.name(), "mt5:spot");
    }

    /// Sahte EA: `addr`'a bağlanır, `count` istek satırı okur ve her birinin `cmd`'sine göre
    /// gerçek EA gibi yanıt yazar (Faz 2 köprü kablolamasını ağ-stub'sız doğrular).
    async fn spawn_fake_ea(addr: std::net::SocketAddr, count: usize) {
        let stream = TcpStream::connect(addr).await.expect("EA connect");
        let mut r = BufReader::new(stream);
        for _ in 0..count {
            let mut req = String::new();
            if r.read_line(&mut req).await.unwrap_or(0) == 0 {
                break;
            }
            let resp = if req.contains("\"cmd\":\"order\"") {
                r#"{"ok":true,"order_id":"777","status":"filled","filled_qty":0.1,"avg_price":1.0825}"#
            } else if req.contains("\"cmd\":\"balance\"") {
                r#"{"ok":true,"balance":9876.5}"#
            } else if req.contains("\"cmd\":\"cancel_all\"") {
                r#"{"ok":true,"canceled":2}"#
            } else if req.contains("\"cmd\":\"set_leverage\"") {
                r#"{"ok":true}"#
            } else {
                r#"{"ok":false,"error":"beklenmeyen cmd"}"#
            };
            r.get_mut().write_all(format!("{resp}\n").as_bytes()).await.unwrap();
            r.get_mut().flush().await.unwrap();
        }
    }

    #[tokio::test]
    async fn execution_round_trips_through_bridge() {
        let bridge = Arc::new(Mt5Bridge::new(
            "127.0.0.1:0".into(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        ));
        let addr = bridge.ensure_bound().await.expect("bind");
        let ea = tokio::spawn(spawn_fake_ea(addr, 4));

        let v = Mt5Venue::new(Market::Futures, bridge);

        let receipt = v
            .submit_order(&OrderRequest::market("EURUSD", OrderSide::Buy, 0.1))
            .await
            .expect("submit_order");
        assert_eq!(receipt.status, OrderStatus::Filled);
        assert_eq!(receipt.filled_qty, 0.1);
        assert_eq!(receipt.avg_price, 1.0825);
        assert_eq!(receipt.venue_order_id.as_deref(), Some("777"));

        assert_eq!(v.balance().await.expect("balance"), 9876.5);
        v.cancel_all("EURUSD").await.expect("cancel_all");
        v.set_leverage("EURUSD", 10).await.expect("set_leverage");

        ea.await.unwrap();
    }

    #[tokio::test]
    async fn ok_false_from_ea_surfaces_error() {
        let bridge = Arc::new(Mt5Bridge::new(
            "127.0.0.1:0".into(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        ));
        let addr = bridge.ensure_bound().await.expect("bind");
        let ea = tokio::spawn(async move {
            let stream = TcpStream::connect(addr).await.expect("EA connect");
            let mut r = BufReader::new(stream);
            let mut req = String::new();
            r.read_line(&mut req).await.unwrap();
            r.get_mut()
                .write_all(b"{\"ok\":false,\"error\":\"trade disabled\"}\n")
                .await
                .unwrap();
            r.get_mut().flush().await.unwrap();
        });

        let v = Mt5Venue::new(Market::Futures, bridge);
        let e = v
            .submit_order(&OrderRequest::market("EURUSD", OrderSide::Buy, 0.1))
            .await;
        assert!(e.is_err());
        assert!(format!("{}", e.unwrap_err()).contains("trade disabled"));
        ea.await.unwrap();
    }
}
