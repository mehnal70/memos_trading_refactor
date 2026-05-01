// Order Management System - Binance Implementation
// 
// Srivastava mikarisi: Exchange-specific implementation
// OrderManager trait'ini implement ediyor

use super::types::*;
use crate::Result as MemosTradingResult;
use async_trait::async_trait;
use super::base::OrderManager;
use chrono::{TimeZone, Utc};
use hmac::{Hmac, Mac};
use reqwest::{Client, Method};
use serde_json::Value;
use sha2::Sha256;
use std::collections::{HashMap, HashSet};
use std::env;
use tokio::sync::RwLock;
// use crate::secrets; // (Yorumlandı: derleme hatası önleme amaçlı)
// use crate::secrets::load_api_keys_from_encrypted_file; // (Yorumlandı: derleme hatası önleme amaçlı)

type HmacSha256 = Hmac<Sha256>;

/// Binance OMS Implementation
#[allow(dead_code)]
pub struct BinanceOrderManager {
    api_key: String,
    api_secret: String,
    base_url: String,
    client: Client,
    order_symbol_cache: RwLock<HashMap<u64, String>>,
}

impl BinanceOrderManager {
    /// Ortam değişkeni veya şifreli dosyadan anahtar ve endpoint yükle
    pub fn from_env_or_encrypted() -> Self {
        dotenvy::dotenv().ok();
        let env_type = env::var("TRADING_ENV").unwrap_or_else(|_| "live".to_string());
        let base_url = if env_type == "testnet" {
            env::var("BINANCE_TESTNET_URL").unwrap_or_else(|_| "https://testnet.binance.vision".to_string())
        } else {
            env::var("BINANCE_BASE_URL").unwrap_or_else(|_| "https://api.binance.com".to_string())
        };
        // Şifreli dosya yolu ve parola ortamdan alınabilir
        if let (Ok(_enc_path), Ok(_pass)) = (env::var("BINANCE_KEYS_ENC_PATH"), env::var("BINANCE_KEYS_PASSPHRASE")) {
            // Şifreli anahtar yükleyici devre dışı (eksik fonksiyon): doğrudan fallback'a geç
        }
        // Fallback: düz ortam değişkeni
        let api_key = env::var("BINANCE_API_KEY").unwrap_or_default();
        let api_secret = env::var("BINANCE_API_SECRET").unwrap_or_default();
        Self {
            api_key,
            api_secret,
            base_url,
            client: Client::new(),
            order_symbol_cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn new(api_key: String, api_secret: String) -> Self {
        Self {
            api_key,
            api_secret,
            base_url: "https://api.binance.com".to_string(),
            client: Client::new(),
            order_symbol_cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn testnet() -> Self {
        Self {
            api_key: String::new(),
            api_secret: String::new(),
            base_url: "https://testnet.binance.vision".to_string(),
            client: Client::new(),
            order_symbol_cache: RwLock::new(HashMap::new()),
        }
    }

    fn validate_api_keys(&self) -> MemosTradingResult<()> {
        if self.api_key.is_empty() || self.api_secret.is_empty() {
            return Err("BINANCE_API_KEY/BINANCE_API_SECRET tanımlı değil".into());
        }
        Ok(())
    }

    fn format_num(value: f64) -> String {
        let mut s = format!("{:.8}", value);
        while s.contains('.') && s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
        s
    }

    fn to_hex(bytes: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0f) as usize] as char);
        }
        out
    }

    fn sign_query(&self, query: &str) -> MemosTradingResult<String> {
        let mut mac = HmacSha256::new_from_slice(self.api_secret.as_bytes())
            .map_err(|e| format!("HMAC oluşturulamadı: {}", e))?;
        mac.update(query.as_bytes());
        let signature = mac.finalize().into_bytes();
        Ok(Self::to_hex(&signature))
    }

    async fn signed_request(
        &self,
        method: Method,
        path: &str,
        mut params: Vec<(String, String)>,
    ) -> MemosTradingResult<Value> {
        self.validate_api_keys()?;

        params.push(("timestamp".to_string(), Utc::now().timestamp_millis().to_string()));
        params.push(("recvWindow".to_string(), "5000".to_string()));

        let query = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<String>>()
            .join("&");

        let signature = self.sign_query(&query)?;
        let signed = format!("{}&signature={}", query, signature);
        let url = format!("{}{}", self.base_url, path);

        let req = match method {
            Method::GET => self.client.get(format!("{}?{}", url, signed)),
            Method::DELETE => self.client.delete(format!("{}?{}", url, signed)),
            _ => self.client.post(url).body(signed),
        }
        .header("X-MBX-APIKEY", &self.api_key)
        .header("Content-Type", "application/x-www-form-urlencoded");

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Binance istek hatası: {}", e))?;

        let status = resp.status();
        let body = resp.text().await.map_err(|e| format!("Yanıt okunamadı: {}", e))?;

        if !status.is_success() {
            return Err(format!("Binance API hatası ({}): {}", status, body).into());
        }

        serde_json::from_str(&body).map_err(|e| format!("JSON parse hatası: {}", e).into())
    }

    async fn resolve_symbol_for_order(&self, order_id: u64) -> MemosTradingResult<String> {
        let cache = self.order_symbol_cache.read().await;
        cache
            .get(&order_id)
            .cloned()
            .ok_or_else(|| format!("order_id={} için symbol cache bulunamadı", order_id).into())
    }

    fn parse_order_status(status: &str) -> OrderStatus {
        match status {
            "NEW" => OrderStatus::New,
            "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
            "FILLED" => OrderStatus::Filled,
            "CANCELED" => OrderStatus::Canceled,
            "REJECTED" => OrderStatus::Rejected,
            "EXPIRED" => OrderStatus::Expired,
            _ => OrderStatus::Rejected,
        }
    }

    fn parse_order_type(order_type: &str) -> OrderType {
        match order_type {
            "LIMIT" => OrderType::Limit,
            "MARKET" => OrderType::Market,
            "STOP_LOSS" | "STOP_LOSS_LIMIT" => OrderType::StopLoss,
            "TAKE_PROFIT" | "TAKE_PROFIT_LIMIT" => OrderType::TakeProfit,
            _ => OrderType::Market,
        }
    }

    fn parse_order_side(side: &str) -> OrderSide {
        if side == "SELL" {
            OrderSide::Sell
        } else {
            OrderSide::Buy
        }
    }

    fn parse_order_from_json(v: &Value) -> Order {
        let order_id = v.get("orderId").and_then(|x| x.as_u64()).map(OrderId);
        let symbol = v.get("symbol").and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let side = Self::parse_order_side(v.get("side").and_then(|x| x.as_str()).unwrap_or("BUY"));
        let order_type = Self::parse_order_type(v.get("type").and_then(|x| x.as_str()).unwrap_or("MARKET"));
        let quantity = v
            .get("origQty")
            .and_then(|x| x.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let filled_quantity = v
            .get("executedQty")
            .and_then(|x| x.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let price = v
            .get("price")
            .and_then(|x| x.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|p| *p > 0.0);
        let stop_price = v
            .get("stopPrice")
            .and_then(|x| x.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|p| *p > 0.0);
        let status = Self::parse_order_status(v.get("status").and_then(|x| x.as_str()).unwrap_or("NEW"));
        let update_time = v.get("updateTime").and_then(|x| x.as_i64());
        let created_at = update_time.and_then(|ms| Utc.timestamp_millis_opt(ms).single());

        Order {
            id: order_id,
            symbol,
            side,
            order_type,
            quantity,
            price,
            stop_price,
            filled_quantity,
            status,
            average_price: 0.0,
            created_at,
            raw_data: Some(v.to_string()),
        }
    }
}

#[async_trait]
impl OrderManager for BinanceOrderManager {
    async fn place_order(&self, order: &Order) -> MemosTradingResult<OrderId> {
        let mut params = vec![
            ("symbol".to_string(), order.symbol.clone()),
            (
                "side".to_string(),
                match order.side {
                    OrderSide::Buy => "BUY".to_string(),
                    OrderSide::Sell => "SELL".to_string(),
                },
            ),
            ("quantity".to_string(), Self::format_num(order.quantity)),
        ];

        match order.order_type {
            OrderType::Market => {
                params.push(("type".to_string(), "MARKET".to_string()));
            }
            OrderType::Limit => {
                let price = order.price.ok_or("LIMIT emri için price zorunlu")?;
                params.push(("type".to_string(), "LIMIT".to_string()));
                params.push(("timeInForce".to_string(), "GTC".to_string()));
                params.push(("price".to_string(), Self::format_num(price)));
            }
            OrderType::StopLoss => {
                let stop = order.stop_price.ok_or("STOP_LOSS için stop_price zorunlu")?;
                let price = order.price.ok_or("STOP_LOSS_LIMIT için price zorunlu")?;
                params.push(("type".to_string(), "STOP_LOSS_LIMIT".to_string()));
                params.push(("timeInForce".to_string(), "GTC".to_string()));
                params.push(("stopPrice".to_string(), Self::format_num(stop)));
                params.push(("price".to_string(), Self::format_num(price)));
            }
            OrderType::TakeProfit => {
                let stop = order.stop_price.ok_or("TAKE_PROFIT için stop_price zorunlu")?;
                let price = order.price.ok_or("TAKE_PROFIT_LIMIT için price zorunlu")?;
                params.push(("type".to_string(), "TAKE_PROFIT_LIMIT".to_string()));
                params.push(("timeInForce".to_string(), "GTC".to_string()));
                params.push(("stopPrice".to_string(), Self::format_num(stop)));
                params.push(("price".to_string(), Self::format_num(price)));
            }
        }

        let response = self
            .signed_request(Method::POST, "/api/v3/order", params)
            .await?;

        let order_id = response
            .get("orderId")
            .and_then(|v| v.as_u64())
            .ok_or("Binance response içinde orderId yok")?;

        let mut cache = self.order_symbol_cache.write().await;
        cache.insert(order_id, order.symbol.clone());

        Ok(OrderId(order_id))
    }
    
    async fn cancel_order(&self, order_id: OrderId) -> MemosTradingResult<()> {
        let symbol = self.resolve_symbol_for_order(order_id.0).await?;
        let params = vec![
            ("symbol".to_string(), symbol),
            ("orderId".to_string(), order_id.0.to_string()),
        ];
        self
            .signed_request(Method::DELETE, "/api/v3/order", params)
            .await?;
        Ok(())
    }
    
    async fn get_order_status(&self, order_id: OrderId) -> MemosTradingResult<OrderStatus> {
        let symbol = self.resolve_symbol_for_order(order_id.0).await?;
        let params = vec![
            ("symbol".to_string(), symbol),
            ("orderId".to_string(), order_id.0.to_string()),
        ];
        let response = self
            .signed_request(Method::GET, "/api/v3/order", params)
            .await?;
        let status = response
            .get("status")
            .and_then(|v| v.as_str())
            .map(Self::parse_order_status)
            .ok_or("Order status parse edilemedi")?;
        Ok(status)
    }
    
    async fn get_order(&self, order_id: OrderId) -> MemosTradingResult<Order> {
        let symbol = self.resolve_symbol_for_order(order_id.0).await?;
        let params = vec![
            ("symbol".to_string(), symbol),
            ("orderId".to_string(), order_id.0.to_string()),
        ];
        let response = self
            .signed_request(Method::GET, "/api/v3/order", params)
            .await?;
        Ok(Self::parse_order_from_json(&response))
    }
    
    async fn list_active_orders(&self, symbol: Option<&str>) -> MemosTradingResult<Vec<Order>> {
        let mut params = Vec::new();
        if let Some(sym) = symbol {
            params.push(("symbol".to_string(), sym.to_string()));
        }
        let response = self
            .signed_request(Method::GET, "/api/v3/openOrders", params)
            .await?;

        let arr = response.as_array().ok_or("openOrders yanıtı array değil")?;
        let mut orders = Vec::with_capacity(arr.len());
        let mut cache = self.order_symbol_cache.write().await;
        for item in arr {
            let order = Self::parse_order_from_json(item);
            if let Some(id) = order.id {
                cache.insert(id.0, order.symbol.clone());
            }
            orders.push(order);
        }
        Ok(orders)
    }
    
    async fn get_order_history(
        &self,
        symbol: Option<&str>,
        limit: Option<usize>,
    ) -> MemosTradingResult<Vec<Order>> {
        let lim = limit.unwrap_or(100).min(1000);
        let mut symbols_to_query: Vec<String> = Vec::new();

        if let Some(sym) = symbol {
            symbols_to_query.push(sym.to_string());
        } else {
            let cache = self.order_symbol_cache.read().await;
            let mut set = HashSet::new();
            for sym in cache.values() {
                set.insert(sym.clone());
            }
            symbols_to_query.extend(set);
        }

        if symbols_to_query.is_empty() {
            return Ok(vec![]);
        }

        let mut out = Vec::new();
        for sym in symbols_to_query {
            let params = vec![
                ("symbol".to_string(), sym),
                ("limit".to_string(), lim.to_string()),
            ];
            let response = self
                .signed_request(Method::GET, "/api/v3/allOrders", params)
                .await?;
            let arr = response.as_array().ok_or("allOrders yanıtı array değil")?;
            for item in arr {
                out.push(Self::parse_order_from_json(item));
            }
        }

        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        if let Some(l) = limit {
            out.truncate(l);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_binance_manager_creation() {
        let _manager = BinanceOrderManager::new(
            "test-key".to_string(),
            "test-secret".to_string(),
        );
        // Manager başarıyla oluşturulmalı
    }
}
