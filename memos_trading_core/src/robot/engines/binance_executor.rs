// robot/binance_executor.rs - Optimize Edilmiş Tam Sürüm

use std::time::{SystemTime, UNIX_EPOCH};
use reqwest::{Client, Method};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use serde_json::Value;
use crate::Result;
use crate::MemosTradingError;

type HmacSha256 = Hmac<Sha256>;

pub struct BinanceFuturesExecutor {
    pub api_key: String,
    pub api_secret: String,
    pub client: Client,
    pub is_paper: bool,
    pub is_spot: bool,
    pub base_url: String,
}

impl BinanceFuturesExecutor {
    pub fn new_for_market(api_key: String, api_secret: String, is_paper: bool, market: &str) -> Self {
        let is_spot = market == "spot";
        let base_url = match (is_spot, is_paper) {
            (true, _) => "https://binance.com",
            (false, true) => "https://binancefuture.com",
            (false, false) => "https://binance.com",
        }.to_owned();

        Self { api_key, api_secret, client: Client::new(), is_paper, is_spot, base_url }
    }

    // --- MERKEZİ İŞLEMCİLER (Bloat'u temizleyen kısım burası) ---

    fn sign(&self, data: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(self.api_secret.as_bytes()).expect("HMAC Error");
        mac.update(data.as_bytes());
        format!("{:x}", mac.finalize().into_bytes())
    }

    fn format_f64(&self, val: f64) -> String {
        format!("{:.8}", val).trim_end_matches('0').trim_end_matches('.').to_owned()
    }

    async fn signed_request(&self, method: Method, path: &str, mut params: Vec<String>) -> Result<Value> {
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
        params.push(format!("timestamp={}", ts));
        params.push("recvWindow=5000".to_owned());
        params.sort();

        let query = params.join("&");
        let sig = self.sign(&query);
        let url = format!("{}{}?{}&signature={}", self.base_url, path, query, sig);

        let resp = self.client.request(method, &url).header("X-MBX-APIKEY", &self.api_key).send().await?;
        if !resp.status().is_success() {
            return Err(MemosTradingError::Api(format!("Binance Error: {}", resp.text().await?)));
        }
        Ok(resp.json().await?)
    }

    // --- TÜM FONKSİYONLARIN GÜNCEL HALİ ---

    pub async fn place_market_order(&self, symbol: &str, side: &str, qty: f64) -> Result<Value> {
        let path = if self.is_spot { "/api/v3/order" } else { "/fapi/v1/order" };
        let params = vec![format!("symbol={}", symbol), format!("side={}", side), "type=MARKET".to_owned(), format!("quantity={}", self.format_f64(qty))];
        self.signed_request(Method::POST, path, params).await
    }

    pub async fn place_post_only_limit_order(&self, symbol: &str, side: &str, qty: f64, price: f64) -> Result<Value> {
        let path = if self.is_spot { "/api/v3/order" } else { "/fapi/v1/order" };
        let mut params = vec![format!("symbol={}", symbol), format!("side={}", side), format!("quantity={}", self.format_f64(qty)), format!("price={}", self.format_f64(price))];
        if self.is_spot { params.push("type=LIMIT_MAKER".to_owned()); }
        else { params.push("type=LIMIT".to_owned()); params.push("timeInForce=GTX".to_owned()); }
        self.signed_request(Method::POST, path, params).await
    }

    /// 🛡️ STOP-LOSS emri (pozisyonu trigger fiyatında kapatır).
    /// `side` pozisyonun KAPATMA yönü (long pozisyon için "SELL", short için "BUY").
    /// Futures: STOP_MARKET + reduceOnly. Spot: STOP_LOSS (market stop).
    pub async fn place_stop_loss_order(&self, symbol: &str, side: &str, qty: f64, stop_price: f64) -> Result<Value> {
        let path = if self.is_spot { "/api/v3/order" } else { "/fapi/v1/order" };
        let mut params = vec![
            format!("symbol={}", symbol),
            format!("side={}", side),
            format!("quantity={}", self.format_f64(qty)),
            format!("stopPrice={}", self.format_f64(stop_price)),
        ];
        if self.is_spot {
            // Spot: STOP_LOSS (market trigger) — stopPrice tetiklendiğinde market emir oluşur.
            params.push("type=STOP_LOSS".to_owned());
        } else {
            // Futures: STOP_MARKET + reduceOnly → mevcut pozisyonu kapatır, yeni pozisyon açmaz.
            params.push("type=STOP_MARKET".to_owned());
            params.push("reduceOnly=true".to_owned());
            params.push("timeInForce=GTC".to_owned());
        }
        self.signed_request(Method::POST, path, params).await
    }

    /// 🎯 TAKE-PROFIT emri (kâr seviyesinde kapatır).
    /// `side` pozisyonun KAPATMA yönü.
    /// Futures: TAKE_PROFIT_MARKET + reduceOnly. Spot: TAKE_PROFIT (market trigger).
    pub async fn place_take_profit_order(&self, symbol: &str, side: &str, qty: f64, tp_price: f64) -> Result<Value> {
        let path = if self.is_spot { "/api/v3/order" } else { "/fapi/v1/order" };
        let mut params = vec![
            format!("symbol={}", symbol),
            format!("side={}", side),
            format!("quantity={}", self.format_f64(qty)),
            format!("stopPrice={}", self.format_f64(tp_price)),
        ];
        if self.is_spot {
            params.push("type=TAKE_PROFIT".to_owned());
        } else {
            params.push("type=TAKE_PROFIT_MARKET".to_owned());
            params.push("reduceOnly=true".to_owned());
            params.push("timeInForce=GTC".to_owned());
        }
        self.signed_request(Method::POST, path, params).await
    }

    /// Pozisyon için hem SL hem TP emrini sırayla yerleştirir.
    /// Hata varsa Vec içinde toplar; herhangi biri başarısızsa caller emergency_close çağırmalı.
    /// Dönüş: (sl_order_id, tp_order_id) — emir verilemezse None.
    pub async fn place_protection_orders(
        &self,
        symbol: &str,
        is_long: bool,
        qty: f64,
        stop_loss: f64,
        take_profit: f64,
    ) -> (Result<Value>, Result<Value>) {
        // Long pozisyonu kapatma yönü SELL; short pozisyonu BUY ile kapatılır.
        let close_side = if is_long { "SELL" } else { "BUY" };
        let sl_res = self.place_stop_loss_order(symbol, close_side, qty, stop_loss).await;
        let tp_res = self.place_take_profit_order(symbol, close_side, qty, take_profit).await;
        (sl_res, tp_res)
    }

    pub async fn get_order_status(&self, symbol: &str, order_id: u64) -> Result<Value> {
        let path = if self.is_spot { "/api/v3/order" } else { "/fapi/v1/order" };
        self.signed_request(Method::GET, path, vec![format!("symbol={}", symbol), format!("orderId={}", order_id)]).await
    }

    pub async fn cancel_order(&self, symbol: &str, order_id: u64) -> Result<Value> {
        let path = if self.is_spot { "/api/v3/order" } else { "/fapi/v1/order" };
        self.signed_request(Method::DELETE, path, vec![format!("symbol={}", symbol), format!("orderId={}", order_id)]).await
    }

    pub async fn cancel_all_orders(&self, symbol: &str) -> Result<Value> {
        let path = if self.is_spot { "/api/v3/openOrders" } else { "/fapi/v1/allOpenOrders" };
        self.signed_request(Method::DELETE, path, vec![format!("symbol={}", symbol)]).await
    }

    /// Sembolün borsadaki açık emirlerini listele. Protection sync task bunu
    /// kullanarak SL veya TP'nin tetiklendiğini (emir kaybolması) yakalar.
    pub async fn get_open_orders(&self, symbol: &str) -> Result<Vec<Value>> {
        let path = if self.is_spot { "/api/v3/openOrders" } else { "/fapi/v1/openOrders" };
        let resp = self.signed_request(Method::GET, path, vec![format!("symbol={}", symbol)]).await?;
        Ok(resp.as_array().cloned().unwrap_or_default())
    }

    pub async fn get_balance(&self) -> Result<f64> {
        let path = if self.is_spot { "/api/v3/account" } else { "/fapi/v2/account" };
        let resp = self.signed_request(Method::GET, path, vec![]).await?;
        let val = if self.is_spot {
            resp["balances"].as_array().and_then(|l| l.iter().find(|a| a["asset"] == "USDT")).and_then(|u| u["free"].as_str())
        } else { resp["totalWalletBalance"].as_str() };
        Ok(val.and_then(|s| s.parse().ok()).unwrap_or(0.0))
    }

    pub async fn get_positions(&self, symbol: &str) -> Result<Vec<Value>> {
        if self.is_spot { return Ok(vec![]); }
        let resp = self.signed_request(Method::GET, "/fapi/v2/positionRisk", vec![format!("symbol={}", symbol)]).await?;
        Ok(resp.as_array().cloned().unwrap_or_default())
    }

    pub async fn close_position(&self, symbol: &str) -> Result<Value> {
        let pos = self.get_positions(symbol).await?;
        let qty = pos.get(0).and_then(|p| p["positionAmt"].as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        if qty.abs() < f64::EPSILON { return Err(MemosTradingError::Api("Pozisyon kapalı".to_owned())); }
        self.place_market_order(symbol, if qty > 0.0 { "SELL" } else { "BUY" }, qty.abs()).await
    }

    pub async fn fetch_book_ticker(&self, symbol: &str) -> Result<(f64, f64)> {
        if self.is_paper { return Ok((0.0, 0.0)); }
        let path = if self.is_spot { format!("/api/v3/ticker/bookTicker?symbol={}", symbol) }
                   else { format!("/fapi/v1/ticker/bookTicker?symbol={}", symbol) };
        let v: Value = self.client.get(format!("{}{}", self.base_url, path)).send().await?.json().await?;
        let get_f = |k: &str| v[k].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        Ok((get_f("bidPrice"), get_f("askPrice")))
    }

    pub fn log_order(&self, symbol: &str, side: &str, qty: f64, price: f64) -> String {
        format!("[{}] Order: {} {} qty={} @ {}", if self.is_paper { "PAPER" } else { "LIVE" }, side, symbol, qty, price)
    }
}
