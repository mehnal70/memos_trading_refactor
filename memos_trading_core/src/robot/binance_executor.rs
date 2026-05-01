// robot/binance_executor.rs - Binance Spot + Futures REST client + HMAC imzalama
// Paper ve live mod desteği, market-aware endpoint yönetimi

use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(not(target_arch = "wasm32"))]
use reqwest::Client;

#[cfg(not(target_arch = "wasm32"))]
use hmac::{Hmac, Mac};
#[cfg(not(target_arch = "wasm32"))]
use sha2::Sha256;

#[cfg(not(target_arch = "wasm32"))]
type HmacSha256 = Hmac<Sha256>;

/// Binance executor - Spot ve Futures market desteği, paper ve live mod
#[cfg(not(target_arch = "wasm32"))]
pub struct BinanceFuturesExecutor {
    pub api_key: String,
    pub api_secret: String,
    pub client: Client,
    pub is_paper: bool,  // true = paper/test, false = live
    pub is_spot: bool,   // true = spot (api.binance.com), false = futures (fapi.binance.com)
    pub base_url: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl BinanceFuturesExecutor {
    /// Futures executor (geriye dönük uyumluluk)
    pub fn new(api_key: String, api_secret: String, is_paper: bool) -> Self {
        Self::new_for_market(api_key, api_secret, is_paper, "futures")
    }

    /// Market-aware constructor: market = "spot" | "futures" | "coinm"
    /// Spot ve Futures ayrı API hesabı gerektirir (farklı key kullanmak için
    /// BINANCE_SPOT_API_KEY / BINANCE_SPOT_API_SECRET env var'larını set edin;
    /// yoksa BINANCE_API_KEY / BINANCE_API_SECRET her iki market için de kullanılır).
    pub fn new_for_market(api_key: String, api_secret: String, is_paper: bool, market: &str) -> Self {
        let is_spot = market == "spot";
        let base_url = if is_spot {
            "https://api.binance.com".to_string()
        } else if is_paper {
            "https://testnet.binancefuture.com".to_string()
        } else {
            "https://fapi.binance.com".to_string()
        };
        Self {
            api_key,
            api_secret,
            client: Client::new(),
            is_paper,
            is_spot,
            base_url,
        }
    }

    /// HMAC-SHA256 imza hesapla
    fn sign_request(&self, query_string: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(self.api_secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(query_string.as_bytes());
        // Output'u hex string'e çevir
        let result = mac.finalize();
        format!("{:x}", result.into_bytes())
    }

    /// Nonce (timestamp ms)
    fn nonce(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Order endpoint path: market'e göre seç
    fn order_path(&self) -> &'static str {
        if self.is_spot { "/api/v3/order" } else { "/fapi/v1/order" }
    }

    /// Market emir gönder (BUY/SELL)
    pub async fn place_market_order(
        &self,
        symbol: &str,
        side: &str,       // "BUY" | "SELL"
        quantity: f64,
    ) -> crate::Result<serde_json::Value> {
        let timestamp = self.nonce();
        let mut params = vec![
            format!("symbol={}", symbol),
            format!("side={}", side),
            format!("type=MARKET"),
            format!("quantity={}", quantity),
            format!("timestamp={}", timestamp),
            "recvWindow=5000".to_string(),
        ];

        params.sort();
        let query_string = params.join("&");
        let signature = self.sign_request(&query_string);

        let url = format!(
            "{}{}?{}&signature={}",
            self.base_url, self.order_path(), query_string, signature
        );

        let response = self
            .client
            .post(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(crate::MemosTradingError::Api(format!("Binance API error: {}", error_text)));
        }

        let result = response.json::<serde_json::Value>().await?;
        Ok(result)
    }

    /// Pozisyonu kapat (BUY pozisyonu varsa SELL, SELL pozisyonu varsa BUY)
    pub async fn close_position(&self, symbol: &str) -> crate::Result<serde_json::Value> {
        // Mevcut pozisyonu al
        let positions = self.get_positions(symbol).await?;
        if positions.is_empty() {
            return Err(crate::MemosTradingError::Api("No open position to close".to_string()));
        }

        let position = &positions[0];
        let current_qty = position
            .get("positionAmt")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        if current_qty == 0.0 {
            return Err(crate::MemosTradingError::Api("Position already closed".to_string()));
        }

        // Pozisyonu kapat (ters yönde emir)
        let close_side = if current_qty > 0.0 { "SELL" } else { "BUY" };
        let close_qty = current_qty.abs();

        self.place_market_order(symbol, close_side, close_qty).await
    }

    /// Açık emrimleri iptal et
    pub async fn cancel_all_orders(&self, symbol: &str) -> crate::Result<()> {
        let timestamp = self.nonce();
        let mut params = vec![
            format!("symbol={}", symbol),
            format!("timestamp={}", timestamp),
            "recvWindow=5000".to_string(),
        ];

        params.sort();
        let query_string = params.join("&");
        let signature = self.sign_request(&query_string);

        let cancel_path = if self.is_spot { "/api/v3/openOrders" } else { "/fapi/v1/allOpenOrders" };
        let url = format!(
            "{}{}?{}&signature={}",
            self.base_url, cancel_path, query_string, signature
        );

        let response = self
            .client
            .delete(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(crate::MemosTradingError::Api(format!("Cancel orders error: {}", error_text)));
        }

        Ok(())
    }

    /// Bakiye al
    pub async fn get_balance(&self) -> crate::Result<f64> {
        let timestamp = self.nonce();
        let mut params = vec![
            format!("timestamp={}", timestamp),
            "recvWindow=5000".to_string(),
        ];

        params.sort();
        let query_string = params.join("&");
        let signature = self.sign_request(&query_string);

        let balance_path = if self.is_spot { "/api/v3/account" } else { "/fapi/v2/account" };
        let url = format!(
            "{}{}?{}&signature={}",
            self.base_url, balance_path, query_string, signature
        );

        let response = self
            .client
            .get(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(crate::MemosTradingError::Api(format!("Get balance error: {}", error_text)));
        }

        let account = response.json::<serde_json::Value>().await?;
        // Futures: totalWalletBalance (string)
        // Spot: balances array → USDT freeBalance
        let total_wallet = if self.is_spot {
            account.get("balances")
                .and_then(|b| b.as_array())
                .and_then(|arr| arr.iter().find(|e| e.get("asset").and_then(|a| a.as_str()) == Some("USDT")))
                .and_then(|e| e.get("free").and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()))
                .unwrap_or(0.0)
        } else {
            account
                .get("totalWalletBalance")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0)
        };

        Ok(total_wallet)
    }

    /// Açık pozisyonları al
    pub async fn get_positions(&self, symbol: &str) -> crate::Result<Vec<serde_json::Value>> {
        let timestamp = self.nonce();
        let mut params = vec![
            format!("symbol={}", symbol),
            format!("timestamp={}", timestamp),
            "recvWindow=5000".to_string(),
        ];

        params.sort();
        let query_string = params.join("&");
        let signature = self.sign_request(&query_string);

        // Spot'ta futures pozisyon kavramı yok → boş döner
        if self.is_spot {
            return Ok(vec![]);
        }

        let url = format!(
            "{}/fapi/v2/positionRisk?{}&signature={}",
            self.base_url, query_string, signature
        );

        let response = self
            .client
            .get(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(crate::MemosTradingError::Api(format!("Get positions error: {}", error_text)));
        }

        let positions = response.json::<Vec<serde_json::Value>>().await?;
        Ok(positions)
    }

    /// POST_ONLY / Maker-Only limit emir
    /// Futures: timeInForce=GTX — hemen dolacaksa borsa EXPIRED döndürür (taker olmaz)
    /// Spot:    type=LIMIT_MAKER — taker olacaksa borsa anında reddeder
    pub async fn place_post_only_limit_order(
        &self,
        symbol: &str,
        side: &str,
        quantity: f64,
        price: f64,
    ) -> crate::Result<serde_json::Value> {
        let timestamp = self.nonce();
        let price_str = {
            let mut s = format!("{:.8}", price);
            while s.contains('.') && s.ends_with('0') { s.pop(); }
            if s.ends_with('.') { s.pop(); }
            s
        };
        let qty_str = {
            let mut s = format!("{:.8}", quantity);
            while s.contains('.') && s.ends_with('0') { s.pop(); }
            if s.ends_with('.') { s.pop(); }
            s
        };
        let mut params = if self.is_spot {
            vec![
                format!("symbol={}", symbol),
                format!("side={}", side),
                "type=LIMIT_MAKER".to_string(),
                format!("quantity={}", qty_str),
                format!("price={}", price_str),
                format!("timestamp={}", timestamp),
                "recvWindow=5000".to_string(),
            ]
        } else {
            vec![
                format!("symbol={}", symbol),
                format!("side={}", side),
                "type=LIMIT".to_string(),
                "timeInForce=GTX".to_string(),
                format!("quantity={}", qty_str),
                format!("price={}", price_str),
                format!("timestamp={}", timestamp),
                "recvWindow=5000".to_string(),
            ]
        };
        params.sort();
        let query_string = params.join("&");
        let signature = self.sign_request(&query_string);
        let url = format!(
            "{}{}?{}&signature={}",
            self.base_url, self.order_path(), query_string, signature
        );
        let response = self.client.post(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send().await?;
        if !response.status().is_success() {
            let err = response.text().await?;
            return Err(crate::MemosTradingError::Api(format!("PostOnly limit hatası: {}", err)));
        }
        Ok(response.json::<serde_json::Value>().await?)
    }

    /// Emir durumunu sorgula (orderId ile)
    pub async fn get_order_status(
        &self,
        symbol: &str,
        order_id: u64,
    ) -> crate::Result<serde_json::Value> {
        let timestamp = self.nonce();
        let mut params = vec![
            format!("symbol={}", symbol),
            format!("orderId={}", order_id),
            format!("timestamp={}", timestamp),
            "recvWindow=5000".to_string(),
        ];
        params.sort();
        let query_string = params.join("&");
        let signature = self.sign_request(&query_string);
        let url = format!(
            "{}{}?{}&signature={}",
            self.base_url, self.order_path(), query_string, signature
        );
        let response = self.client.get(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send().await?;
        if !response.status().is_success() {
            let err = response.text().await?;
            return Err(crate::MemosTradingError::Api(format!("get_order_status hatası: {}", err)));
        }
        Ok(response.json::<serde_json::Value>().await?)
    }

    /// Tek emri orderId ile iptal et
    pub async fn cancel_order(
        &self,
        symbol: &str,
        order_id: u64,
    ) -> crate::Result<serde_json::Value> {
        let timestamp = self.nonce();
        let mut params = vec![
            format!("symbol={}", symbol),
            format!("orderId={}", order_id),
            format!("timestamp={}", timestamp),
            "recvWindow=5000".to_string(),
        ];
        params.sort();
        let query_string = params.join("&");
        let signature = self.sign_request(&query_string);
        let url = format!(
            "{}{}?{}&signature={}",
            self.base_url, self.order_path(), query_string, signature
        );
        let response = self.client.delete(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send().await?;
        if !response.status().is_success() {
            let err = response.text().await?;
            return Err(crate::MemosTradingError::Api(format!("cancel_order hatası: {}", err)));
        }
        Ok(response.json::<serde_json::Value>().await?)
    }

    /// Anlık best bid/ask fiyatı (public endpoint, imzasız).
    /// Paper modda WS price fallback için (0.0, 0.0) döner.
    pub async fn fetch_book_ticker(&self, symbol: &str) -> crate::Result<(f64, f64)> {
        if self.is_paper {
            return Ok((0.0, 0.0));
        }
        let path = if self.is_spot {
            format!("/api/v3/ticker/bookTicker?symbol={symbol}")
        } else {
            format!("/fapi/v1/ticker/bookTicker?symbol={symbol}")
        };
        let url = format!("{}{}", self.base_url, path);
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            let e = resp.text().await.unwrap_or_default();
            return Err(crate::MemosTradingError::Api(format!("bookTicker hatası: {e}")));
        }
        let v: serde_json::Value = resp.json().await?;
        let bid = v["bidPrice"].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        let ask = v["askPrice"].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
        Ok((bid, ask))
    }

    /// Paper mod veya live - log'a yazıp devam et
    pub fn log_order(
        &self,
        symbol: &str,
        side: &str,
        quantity: f64,
        entry_price: f64,
    ) -> String {
        let mode = if self.is_paper { "PAPER" } else { "LIVE" };
        format!(
            "[{}] Order: {} {} qty={:.4} @ {:.2}",
            mode, side, symbol, quantity, entry_price
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_executor_new_paper() {
        let exec = BinanceFuturesExecutor::new(
            "test_key".to_string(),
            "test_secret".to_string(),
            true,
        );
        assert!(exec.is_paper);
        assert!(!exec.is_spot);
        assert_eq!(exec.base_url, "https://testnet.binancefuture.com");
    }

    #[test]
    fn test_executor_new_live() {
        let exec = BinanceFuturesExecutor::new(
            "test_key".to_string(),
            "test_secret".to_string(),
            false,
        );
        assert!(!exec.is_paper);
        assert!(!exec.is_spot);
        assert_eq!(exec.base_url, "https://fapi.binance.com");
    }

    #[test]
    fn test_executor_spot() {
        let exec = BinanceFuturesExecutor::new_for_market(
            "test_key".to_string(),
            "test_secret".to_string(),
            false,
            "spot",
        );
        assert!(exec.is_spot);
        assert_eq!(exec.base_url, "https://api.binance.com");
        assert_eq!(exec.order_path(), "/api/v3/order");
    }

    #[test]
    fn test_sign_request() {
        let exec = BinanceFuturesExecutor::new(
            "key".to_string(),
            "secret".to_string(),
            true,
        );
        let sig = exec.sign_request("symbol=BTCUSDT&timestamp=1000");
        assert!(!sig.is_empty());
    }

    #[test]
    fn test_log_order() {
        let exec = BinanceFuturesExecutor::new(
            "key".to_string(),
            "secret".to_string(),
            true,
        );
        let log = exec.log_order("BTCUSDT", "BUY", 0.1, 45000.0);
        assert!(log.contains("PAPER"));
        assert!(log.contains("BUY"));
    }
}
