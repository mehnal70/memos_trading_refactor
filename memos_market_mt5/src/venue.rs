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

use crate::executor::Mt5TradeExecutor;
use std::sync::Arc;

use async_trait::async_trait;

use crate::bridge::{Mt5Bridge, MT5_DEFAULT_EXECUTOR_ADDR};
use crate::protocol;
use memos_venue_api::AccountSnapshot;
use memos_venue_api::Result;
use memos_venue_api::Signal;
use memos_venue_api::SymbolFilters;
use memos_venue_api::Trade;
use memos_venue_api::TradeExecutor;
use memos_venue_api::{Candle, Exchange, Market};
use memos_venue_api::{MarketData, OrderExecution, TradingProfile, VenueAdapter};
use memos_venue_api::{OrderReceipt, OrderRequest};

pub struct Mt5Venue {
    pub market: Market,
    pub bridge: Arc<Mt5Bridge>,
    pub mt5_executor: Arc<dyn TradeExecutor>,
}

impl Mt5Venue {
    /// Verilen köprü üzerinden MT5 venue (market = futures/spot; CFD'ler çoğunlukla "spot"
    /// muamelesi görür, short ayrımı için Futures verilebilir).
    pub fn new(market: Market, bridge: Arc<Mt5Bridge>) -> Self {
        return Self {
            market,
            bridge,
            mt5_executor: Arc::new(Mt5TradeExecutor::new(MT5_DEFAULT_EXECUTOR_ADDR)),
        };
    }

    pub fn with_executor(market: Market, bridge: Arc<Mt5Bridge>, executor_addr: String) -> Self {
        Self {
            market,
            bridge,
            mt5_executor: Arc::new(Mt5TradeExecutor::new(&executor_addr)),
        }
    }

    /// Köprüye erişim (paylaşımlı; aynı EA bağlantısını tüm semboller kullanır).
    pub fn bridge(&self) -> &Arc<Mt5Bridge> {
        return &self.bridge;
    }
}

#[async_trait]
impl MarketData for Mt5Venue {
    async fn fetch_candles(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Candle>> {
        let id = self.bridge.next_id();
        let line = protocol::req_candles(id, symbol, interval, limit.clamp(1, 5000));
        let resp = self.bridge.request(&line).await?;
        return protocol::parse_candles(symbol, interval, &resp);
    }

    async fn book_ticker(&self, symbol: &str) -> Result<(f64, f64)> {
        let id = self.bridge.next_id();
        let line = protocol::req_tick(id, symbol);
        let resp = self.bridge.request(&line).await?;
        return protocol::parse_tick(symbol, &resp);
    }

    async fn symbol_filters(&self, symbol: &str) -> Result<SymbolFilters> {
        let id = self.bridge.next_id();
        let line = protocol::req_filters(id, symbol);
        let resp = self.bridge.request(&line).await?;
        return protocol::parse_filters(symbol, &resp);
    }
}

#[async_trait]
impl OrderExecution for Mt5Venue {
    async fn submit_order(&self, req: &OrderRequest) -> Result<OrderReceipt> {
        let id = self.bridge.next_id();
        let line = protocol::req_order(id, req);
        let resp = self.bridge.request(&line).await?;
        return protocol::parse_order(&req.symbol, &resp);
    }

    async fn place_protection_orders(
        &self,
        symbol: &str,
        _is_long: bool,
        _qty: f64,
        stop_loss: f64,
        take_profit: f64,
    ) -> (Result<OrderReceipt>, Result<OrderReceipt>) {
        let id = self.bridge.next_id();
        let bridge_symbol = symbol.strip_suffix("@mt5").unwrap_or(symbol);
        let line = protocol::req_protection(id, bridge_symbol, stop_loss, take_profit);
        match self.bridge.request(&line).await {
            Ok(resp) => match protocol::parse_order(symbol, &resp) {
                Ok(receipt) => {
                    let verify_line = format!(
                        "{{\"id\":{},\"cmd\":\"account_positions\",\"symbol\":\"{}\"}}",
                        self.bridge.next_id(),
                        symbol
                    );
                    let verification = self
                        .bridge
                        .request(&verify_line)
                        .await
                        .ok()
                        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
                        .and_then(|body| body.get("account_positions").cloned())
                        .and_then(|positions| positions.as_array().cloned())
                        .and_then(|positions| {
                            positions.into_iter().find(|position| {
                                position
                                    .get("symbol")
                                    .and_then(|value| value.as_str())
                                            .map(|value| value.eq_ignore_ascii_case(bridge_symbol))
                                    .unwrap_or(false)
                            })
                        });
                    let verified = verification
                        .as_ref()
                        .and_then(|position| {
                            Some((
                                position.get("sl")?.as_f64()?,
                                position.get("tp")?.as_f64()?,
                            ))
                        })
                        .map(|(actual_sl, actual_tp)| {
                            (stop_loss <= 0.0 || (actual_sl - stop_loss).abs() < 1e-8)
                                && (take_profit <= 0.0 || (actual_tp - take_profit).abs() < 1e-8)
                        })
                        .unwrap_or(false);
                    if !verified {
                        let message = format!(
                            "MT5 protection ACK doğrulanamadı: {} sl={} tp={}",
                            symbol, stop_loss, take_profit
                        );
                        return (
                            Err(memos_venue_api::VenueError::Unknown(message.clone())),
                            Err(memos_venue_api::VenueError::Unknown(message)),
                        );
                    }
                    let sl = if stop_loss > 0.0 {
                        Ok(receipt.clone())
                    } else {
                        Err(memos_venue_api::VenueError::Unsupported(
                            "MT5 stop-loss seviyesi yok".to_string(),
                        ))
                    };
                    let tp = if take_profit > 0.0 {
                        Ok(receipt)
                    } else {
                        Err(memos_venue_api::VenueError::Unsupported(
                            "MT5 take-profit seviyesi yok".to_string(),
                        ))
                    };
                    (sl, tp)
                }
                Err(error) => {
                    let message = error.to_string();
                    (
                        Err(memos_venue_api::VenueError::Unknown(message.clone())),
                        Err(memos_venue_api::VenueError::Unknown(message)),
                    )
                }
            },
            Err(error) => (
                Err(memos_venue_api::VenueError::Network(error.clone())),
                Err(memos_venue_api::VenueError::Network(error)),
            ),
        }
    }

    async fn cancel_all(&self, symbol: &str) -> Result<()> {
        let id = self.bridge.next_id();
        let line = protocol::req_cancel_all(id, symbol);
        //let resp = self.bridge.request(&line).await?;
        return self
            .bridge
            .request(&line)
            .await
            .map(|_| ())
            .map_err(Into::into);
    }

    async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<()> {
        let id = self.bridge.next_id();
        let line = protocol::req_set_leverage(id, symbol, leverage);
        //let resp = self.bridge.request(&line).await?;
        return self
            .bridge
            .request(&line)
            .await
            .map(|_| ())
            .map_err(Into::into);
    }

    async fn balance(&self) -> Result<f64> {
        let id = self.bridge.next_id();
        let line = protocol::req_balance(id);
        let resp = self.bridge.request(&line).await?;
        return protocol::parse_balance(&resp);
    }

    async fn account_snapshot(&self) -> Result<AccountSnapshot> {
        let id = self.bridge.next_id();
        let resp = self.bridge.request(&protocol::req_balance(id)).await?;
        protocol::parse_account_snapshot(&resp)
    }

    // ─── 🛰️ OTONOM EMİR SORGULAMA KANCASI YERLEŞTİRMESİ ──────────────────────────────────
    // GEREKÇE: İki argümanlı mutlak Result şemasıyla nesne güvenliği (dyn compatibility) mühürlenmiştir.
    // MT5 köprüsünden asenkron REQ/REP hattı üzerinden bekleyen CFD/Forex emirlerini çeker ve normalleştirir.
    // mt5/venue.rs içerisindeki fetch_open_orders fonksiyonunu şu şekilde tazeleyin:
    async fn fetch_open_orders(&self) -> core::result::Result<Vec<serde_json::Value>, String> {
        let id = self.bridge.next_id();
        //let line = format!("id={}|action=req_open_orders", id);
        // GEREKÇE: MQL5 el yapımı parser'ının okuyabileceği katı JSON şablonu (cmd) fırlatılır.
        let line = format!("{{\"id\":{},\"cmd\":\"open_orders\",\"symbol\":\"\"}}", id);

        match self.bridge.request(&line).await {
            Ok(resp) => {
                if let Ok(parsed_json) = serde_json::from_str::<serde_json::Value>(&resp) {
                    let positions = parsed_json.as_array().or_else(|| {
                        parsed_json
                            .get("account_positions")
                            .and_then(|value| value.as_array())
                    });
                    if let Some(arr) = positions {
                        // KESİN ÇÖZÜM: Üst katman string eşleşme (mismatch) barajını yıkıyoruz.
                        // Borsadan gelen ham emir dizisini geziyoruz ve çıplak sembollerin sonuna otonom `@mt5` mühürünü basıyoruz.
                        let mut tagged_orders = Vec::with_capacity(arr.len());

                        for order in arr {
                            let mut order_clone = order.clone();
                            if let Some(obj) = order_clone.as_object_mut() {
                                if let Some(bare_sym) = obj.get("symbol").and_then(|v| v.as_str()) {
                                    // Eğer gelen sembol zaten etiketli değilse sonuna dinamik olarak mühür vur
                                    if !bare_sym.contains('@') {
                                        let tagged_sym = format!("{}@mt5", bare_sym);
                                        let _ = obj.insert(
                                            "symbol".to_string(),
                                            serde_json::Value::String(tagged_sym),
                                        );
                                    }
                                }
                            }
                            tagged_orders.push(order_clone);
                        }
                        return Ok(tagged_orders);
                    }
                }
                return Ok(vec![]);
            }
            Err(e) => return Err(format!("MT5 bridge fetch_open_orders hatası: {:?}", e)),
        }
    }

    // mt5/venue.rs içerisindeki fetch_account_positions fonksiyonunu şu şekilde güncelleyin:
    /// Kapanmış işlemler — venue'nun KENDİ paylaşımlı köprüsü üzerinden (ikinci bir
    /// listener kurulmaz; EA'nın bağlı olduğu hat kullanılır).
    async fn fetch_closed_trades(&self) -> core::result::Result<Vec<serde_json::Value>, String> {
        let id = self.bridge.next_id();
        let line = format!(
            "{{\"id\":{},\"cmd\":\"closed_trades\",\"symbol\":\"\"}}",
            id
        );
        let resp = self
            .bridge
            .request(&line)
            .await
            .map_err(|e| format!("MT5 closed_trades isteği: {e}"))?;
        let v: serde_json::Value = serde_json::from_str(resp.trim())
            .map_err(|e| format!("MT5 closed_trades parse: {e}"))?;
        if v.get("ok").and_then(|b| b.as_bool()) != Some(true) {
            let err = v
                .get("error")
                .and_then(|m| m.as_str())
                .unwrap_or("ok=false");
            return Err(format!("MT5 closed_trades: {err}"));
        }
        Ok(v.get("closed_trades")
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default())
    }

    async fn fetch_account_positions(
        &self,
    ) -> core::result::Result<Vec<serde_json::Value>, String> {
        let id = self.bridge.next_id();
        //let line = format!("id={}|action=req_account_positions", id);

        // mt5/venue.rs içerisindeki fetch_account_positions fonksiyon istek alanını şu şekilde tazeleyin:
        let line = format!(
            "{{\"id\":{},\"cmd\":\"account_positions\",\"symbol\":\"\"}}",
            id
        );

        match self.bridge.request(&line).await {
            Ok(resp) => {
                if let Ok(parsed_json) = serde_json::from_str::<serde_json::Value>(&resp) {
                    // ⚠️ DÜZELTME: EA yanıtı {"id":..,"ok":true,"account_positions":[...]} OBJESİ —
                    // eski kod yanıtın kendisini array sanıyordu (`parsed_json.as_array()`), bu
                    // asla tutmadığı için fonksiyon HER ZAMAN boş dönüyordu. Sonuç: mutabakat
                    // MT5 pozisyonlarını hiç göremiyor, hepsini "borsada kapanmış" (stale)
                    // sayıp local'den düşürüyordu → pozisyon borsada YÖNETİMSİZ kalıyordu.
                    if let Some(arr) = parsed_json
                        .get("account_positions")
                        .and_then(|v| v.as_array())
                    {
                        // KESİN ÇÖZÜM: Taşınan aktif pozisyonların string uyuşmazlık barajını yıkıyoruz.
                        // MT5 EA'dan gelen ham pozisyon dizisini gezip pariteleri otonom olarak mühürlüyoruz.
                        let mut tagged_positions = Vec::with_capacity(arr.len());

                        for pos in arr {
                            let mut pos_clone = pos.clone();
                            if let Some(obj) = pos_clone.as_object_mut() {
                                if let Some(bare_sym) = obj.get("symbol").and_then(|v| v.as_str()) {
                                    // Eğer gelen pozisyon sembolü çıplaksa sonuna "@mt5" mühürünü vur
                                    if !bare_sym.contains('@') {
                                        let tagged_sym = format!("{}@mt5", bare_sym);
                                        let _ = obj.insert(
                                            "symbol".to_string(),
                                            serde_json::Value::String(tagged_sym),
                                        );
                                    }
                                }
                            }
                            tagged_positions.push(pos_clone);
                        }
                        return Ok(tagged_positions);
                    }
                }
                return Ok(vec![]);
            }
            Err(e) => {
                return Err(format!(
                    "MT5 bridge fetch_account_positions hatası: {:?}",
                    e
                ))
            }
        }
    }
}

impl VenueAdapter for Mt5Venue {
    /// Forex/altın CFD: seanslı, spread hassas → 3 kademe, ATR%2.5 tavan.
    fn trading_profile(&self) -> TradingProfile {
        return TradingProfile {
            max_pyramid_tranches: 3,
            max_atr_pct: 2.5,
        };
    }

    fn exchange(&self) -> Exchange {
        return Exchange::Mt5;
    }
    fn market(&self) -> Market {
        return self.market;
    }

    fn is_delisting_error(&self, error_msg: &str) -> bool {
        let e = error_msg.to_lowercase();
        return e.contains("unknown symbol")
            || e.contains("invalid symbol")
            || e.contains("trade disabled");
    }

    fn execution_engine(&self) -> Option<Arc<dyn TradeExecutor>> {
        let enabled = std::env::var("MT5_EXECUTION_ENABLED")
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        if enabled {
            Some(Arc::clone(&self.mt5_executor))
        } else {
            None
        }
    }
}

impl TradeExecutor for Mt5Bridge {
    fn execute(&self, _signal: Signal, _symbol: &str, _amount: f64) -> Result<Trade> {
        Err("MT5 legacy TradeExecutor desteklenmiyor; VenueAdapter::submit_order kullanın".into())
    }

    fn cancel_all(&self, _symbol: &str) -> Result<()> {
        Err("MT5 legacy TradeExecutor desteklenmiyor; VenueAdapter::cancel_all kullanın".into())
    }

    fn fetch_open_symbols(&self) -> Vec<String> {
        return vec![];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memos_venue_api::AssetClass;
    use memos_venue_api::{OrderSide, OrderStatus};
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
            r.get_mut()
                .write_all(format!("{resp}\n").as_bytes())
                .await
                .unwrap();
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

    /// Kapalı-işlem mutabakat beslemesi: venue PAYLAŞIMLI köprüden çeker (ikinci
    /// listener kurulmaz) ve EA'nın yeni ticket/time/price alanları parse edilir.
    /// (Eski gövde her çağrıda sıfırdan Mt5Bridge kuruyordu → port çekişmesi + EA
    /// yanlış hatta → fonksiyon hiç çalışmıyordu.)
    #[tokio::test]
    async fn fetch_closed_trades_round_trips_shared_bridge() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::TcpStream;

        let bridge = std::sync::Arc::new(Mt5Bridge::new(
            "127.0.0.1:0".into(),
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(2),
        ));
        let addr = bridge.ensure_bound().await.expect("bind");
        let venue = Mt5Venue::new(Market::Cfd, std::sync::Arc::clone(&bridge));

        let ea = tokio::spawn(async move {
            let stream = TcpStream::connect(addr).await.expect("EA connect");
            let mut r = BufReader::new(stream);
            let mut req = String::new();
            let _ = r.read_line(&mut req).await.expect("EA read");
            assert!(req.contains("\"cmd\":\"closed_trades\""), "istek: {req}");
            r.get_mut().write_all(
                b"{\"id\":1,\"ok\":true,\"closed_trades\":[{\"symbol\":\"EURUSD\",\"pnl\":-3.20,\"commission\":0.12,\"qty\":0.1,\"side\":\"buy_close\",\"ticket\":987654321,\"time\":1756700000,\"price\":1.08542}]}\n",
            ).await.expect("EA write");
            r.get_mut().flush().await.unwrap();
        });

        let trades = venue.fetch_closed_trades().await.expect("closed_trades");
        assert_eq!(trades.len(), 1);
        let t = &trades[0];
        assert_eq!(t.get("symbol").and_then(|v| v.as_str()), Some("EURUSD"));
        assert_eq!(t.get("ticket").and_then(|v| v.as_u64()), Some(987654321));
        assert_eq!(t.get("time").and_then(|v| v.as_i64()), Some(1756700000));
        assert!((t.get("price").and_then(|v| v.as_f64()).unwrap() - 1.08542).abs() < 1e-9);
        ea.await.unwrap();
    }
}
