// robot/binance_executor.rs - Optimize Edilmiş Tam Sürüm

use hmac::{Hmac, Mac};
use memos_venue_api::{Result, Signal, SymbolFilters, Trade, TradeExecutor, VenueError};
use reqwest::{Client, Method};
use serde_json::Value;
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

pub struct BinanceFuturesExecutor {
    pub api_key: String,
    pub api_secret: String,
    pub client: Client,
    pub is_paper: bool,
    pub is_spot: bool,
    pub base_url: String,
    /// 🧮 Sembol bazlı emir filtreleri (stepSize/minQty/minNotional/tickSize).
    /// İlk kullanımda exchangeInfo'dan çekilir, sonra burada cache'lenir. Live mode'da
    /// `apply_filters` öncesi otomatik doldurulur. Paper modda boş kalır.
    pub filters: RwLock<HashMap<String, SymbolFilters>>,
    /// Exchange serverTime ile local saat arasındaki ofset (ms). Signed endpoint
    /// çağrılarında local timestamp'e eklenir.
    pub time_offset_ms: AtomicI64,
    next_client_order_id: AtomicU64,
}

impl BinanceFuturesExecutor {
    pub fn new_for_market(
        api_key: String,
        api_secret: String,
        is_paper: bool,
        market: &str,
    ) -> Self {
        let is_spot = market == "spot";
        // Doğru Binance API host'ları (eski sürüm hatalı `binance.com`/`binancefuture.com`
        // kullanıyordu → gerçek emir yanlış adrese giderdi). is_paper=true → TESTNET.
        let base_url = match (is_spot, is_paper) {
            (true, false) => "https://api.binance.com", // spot canlı
            (true, true) => "https://testnet.binance.vision", // spot testnet
            (false, false) => "https://fapi.binance.com", // futures canlı
            (false, true) => "https://testnet.binancefuture.com", // futures testnet
        }
        .to_owned();

        Self {
            api_key,
            api_secret,
            client: Client::new(),
            is_paper,
            is_spot,
            base_url,
            filters: RwLock::new(HashMap::new()),
            time_offset_ms: AtomicI64::new(0),
            next_client_order_id: AtomicU64::new(1),
        }
    }

    // --- MERKEZİ İŞLEMCİLER (Bloat'u temizleyen kısım burası) ---

    fn sign(&self, data: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(self.api_secret.as_bytes()).expect("HMAC Error");
        mac.update(data.as_bytes());
        return format!("{:x}", mac.finalize().into_bytes());
    }

    fn format_f64(&self, val: f64) -> String {
        format!("{:.8}", val)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }

    fn local_epoch_millis() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn signed_timestamp_millis(&self) -> i64 {
        Self::local_epoch_millis().saturating_add(self.time_offset_ms.load(Ordering::Relaxed))
    }

    fn next_client_order_id(&self, symbol: &str) -> String {
        let seq = self.next_client_order_id.fetch_add(1, Ordering::Relaxed);
        let symbol = symbol
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(8)
            .collect::<String>();
        format!("memos_{symbol}_{seq}")
    }

    fn server_time_path(&self) -> &'static str {
        if self.is_spot {
            "/api/v3/time"
        } else {
            "/fapi/v1/time"
        }
    }

    pub async fn sync_server_time_offset(&self) -> Result<i64> {
        let url = format!("{}{}", self.base_url, self.server_time_path());
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| VenueError::Network(e.to_string()))?;
        let json: Value = resp
            .json()
            .await
            .map_err(|e| VenueError::Network(e.to_string()))?;
        let server_ms = json
            .get("serverTime")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                VenueError::Api("Binance time endpoint geçersiz yanıt döndü".to_string())
            })?;
        let local_ms = Self::local_epoch_millis();
        let offset = server_ms.saturating_sub(local_ms);
        self.time_offset_ms.store(offset, Ordering::Relaxed);
        Ok(offset)
    }

    async fn signed_request(
        &self,
        method: Method,
        path: &str,
        params: Vec<String>,
    ) -> Result<Value> {
        let mut retried_after_time_sync = false;

        loop {
            let mut req_params = params.clone();
            req_params.push(format!(
                "timestamp={}",
                self.signed_timestamp_millis().max(0)
            ));
            req_params.push("recvWindow=5000".to_owned());
            req_params.sort();

            let query = req_params.join("&");
            let sig = self.sign(&query);
            let url = format!("{}{}?{}&signature={}", self.base_url, path, query, sig);

            let resp = self
                .client
                .request(method.clone(), &url)
                .header("X-MBX-APIKEY", &self.api_key)
                .send()
                .await
                .map_err(|e| VenueError::Network(e.to_string()))?;
            if resp.status().is_success() {
                return Ok(resp
                    .json()
                    .await
                    .map_err(|e| VenueError::Network(e.to_string()))?);
            }

            let status = resp.status().as_u16();
            let body = resp
                .text()
                .await
                .map_err(|e| VenueError::Network(e.to_string()))?;
            if !retried_after_time_sync && body.contains("-1021") {
                if let Ok(offset) = self.sync_server_time_offset().await {
                    log::warn!("Binance timestamp drift tespit edildi, server offset={}ms ile tekrar denenecek", offset);
                }
                retried_after_time_sync = true;
                continue;
            }

            // 🌍 Sistemik blok (bölge/IP/izin) mi normal emir reddi mi? Devre-kesici
            // yalnız sistemik blokta tripler — ApiBlocked olarak ayrı sınıfla.
            if Self::is_exchange_block(status, &body) {
                return Err(VenueError::ApiBlocked(format!(
                    "HTTP {} · {}",
                    status, body
                )));
            }
            return Err(VenueError::Api(format!(
                "Binance Error: HTTP {} · {}",
                status, body
            )));
        }
    }

    /// Borsa-tarafı SİSTEMİK red (bölge/IP/izin/IP-ban) imzası mı? Normal emir reddinden
    /// (yetersiz bakiye, lot/notional/-4120 endpoint) ayırır → devre-kesici yalnız bunda
    /// tripler. HTTP 451 (bölge/legal), 403 (WAF/Cloudflare), 401 (kimlik), 418/429
    /// (IP auto-ban/rate-limit); body kodu -2015 (geçersiz key-IP-izin), -1003 (WAF/IP-ban).
    /// Saf; I/O yok → birim test edilebilir.
    pub fn is_exchange_block(status: u16, body: &str) -> bool {
        matches!(status, 401 | 403 | 418 | 429 | 451)
            || body.contains("-2015")
            || body.contains("-1003")
    }

    // --- TÜM FONKSİYONLARIN GÜNCEL HALİ ---

    pub async fn place_market_order(&self, symbol: &str, side: &str, qty: f64) -> Result<Value> {
        let path = if self.is_spot {
            "/api/v3/order"
        } else {
            "/fapi/v1/order"
        };
        let params = vec![
            format!("symbol={}", symbol),
            format!("side={}", side),
            "type=MARKET".to_owned(),
            format!("quantity={}", self.format_f64(qty)),
            format!("newClientOrderId={}", self.next_client_order_id(symbol)),
        ];
        self.signed_request(Method::POST, path, params).await
    }

    /// 🎚️ Futures sembol kaldıracını borsada ayarlar (POST /fapi/v1/leverage). Pozisyon
    /// AÇMADAN ÖNCE çağrılmalı — yoksa borsa hesap-default kaldıracını kullanır ve sizing
    /// varsayımıyla (notional = teminat × kaldıraç) uyuşmaz. İdempotent: aynı değeri tekrar
    /// set etmek zararsız (Binance mevcut kaldıracı döndürür). `leverage` 1-125 arası tamsayı.
    /// SPOT'ta kaldıraç kavramı yok → no-op (`Value::Null`). is_paper=true ise testnet'e gider.
    pub async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<Value> {
        if self.is_spot {
            return Ok(Value::Null);
        }
        let lev = leverage.clamp(1, 125);
        let params = vec![format!("symbol={}", symbol), format!("leverage={}", lev)];
        self.signed_request(Method::POST, "/fapi/v1/leverage", params)
            .await
    }

    pub async fn place_post_only_limit_order(
        &self,
        symbol: &str,
        side: &str,
        qty: f64,
        price: f64,
    ) -> Result<Value> {
        let path = if self.is_spot {
            "/api/v3/order"
        } else {
            "/fapi/v1/order"
        };
        let mut params = vec![
            format!("symbol={}", symbol),
            format!("side={}", side),
            format!("quantity={}", self.format_f64(qty)),
            format!("price={}", self.format_f64(price)),
            format!("newClientOrderId={}", self.next_client_order_id(symbol)),
        ];
        if self.is_spot {
            params.push("type=LIMIT_MAKER".to_owned());
        } else {
            params.push("type=LIMIT".to_owned());
            params.push("timeInForce=GTX".to_owned());
        }
        self.signed_request(Method::POST, path, params).await
    }

    /// Maker limit fiyatı: long → best_bid'e katıl, short → best_ask'a katıl
    /// (touch'a yerleş = POST_ONLY garanti maker; spread'e girmez). Saf; I/O yok.
    pub fn maker_limit_price(is_long: bool, best_bid: f64, best_ask: f64) -> f64 {
        if is_long {
            best_bid
        } else {
            best_ask
        }
    }

    /// Spread (bps) = (ask-bid)/mid*10_000. Geçersiz kotada (mid≤0 ya da ask≤bid) 0
    /// döner → guard tetiklenmez (kota yoksa caller akışı zaten reddeder). Saf.
    pub fn spread_bps(best_bid: f64, best_ask: f64) -> f64 {
        let mid = (best_bid + best_ask) / 2.0;
        if mid <= 0.0 || best_ask <= best_bid {
            return 0.0;
        }
        (best_ask - best_bid) / mid * 10_000.0
    }

    /// 💱 Maker giriş orkestrasyonu: best_bid/ask'e POST_ONLY limit koyar, dolana
    /// kadar en çok `max_attempts` kez re-quote eder. Spread `max_spread_bps`'i
    /// (0 → guard kapalı) aşarsa o deneme atlanır. Her denemede `timeout_ms` içinde
    /// fill beklenir, dolmazsa emir iptal edilip yeniden kote edilir.
    /// Dönüş: FILLED emir Value'su (orderId + avgPrice içerir). Hata: deneme tükendi
    /// / kota alınamadı / spread sürekli geniş.
    /// `place_post_only_limit_order` zaten futures GTX / spot LIMIT_MAKER yönlendirir
    /// → maker garantisi tek kaynakta.
    pub async fn place_smart_limit_entry(
        &self,
        symbol: &str,
        side: &str, // "BUY" | "SELL"
        qty: f64,
        timeout_ms: u64,
        max_attempts: u32,
        max_spread_bps: f64,
    ) -> Result<Value> {
        let is_long = side.eq_ignore_ascii_case("BUY");
        let attempts = max_attempts.max(1);
        let backoff = std::time::Duration::from_millis(200);
        let poll = std::time::Duration::from_millis(500);

        for attempt in 1..=attempts {
            let last = attempt >= attempts;
            let (bid, ask) = self.fetch_book_ticker(symbol).await.unwrap_or((0.0, 0.0));

            // Geçerli kota yoksa fiyatlandıramayız.
            if bid <= 0.0 || ask <= 0.0 || ask <= bid {
                if !last {
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                return Err(VenueError::Api(format!(
                    "maker giriş: geçerli kota yok [{}]",
                    symbol
                )));
            }

            // Spread guard.
            let spr = Self::spread_bps(bid, ask);
            if max_spread_bps > 0.0 && spr > max_spread_bps {
                if !last {
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                return Err(VenueError::Api(format!(
                    "maker giriş: spread {:.1}bps > {:.1}bps [{}]",
                    spr, max_spread_bps, symbol
                )));
            }

            // Fiyatı tickSize'a yuvarla (cache'teki filtre; yoksa ham fiyat).
            let raw_price = Self::maker_limit_price(is_long, bid, ask);
            let price = self
                .filters
                .read()
                .ok()
                .and_then(|m| m.get(symbol).map(|f| f.round_price(raw_price)))
                .filter(|&p| p > 0.0)
                .unwrap_or(raw_price);

            let resp = match self
                .place_post_only_limit_order(symbol, side, qty, price)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    if !last {
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                    return Err(e);
                }
            };

            match resp.get("status").and_then(|v| v.as_str()).unwrap_or("") {
                "FILLED" => return Ok(resp),
                // GTX/LIMIT_MAKER anında iptal → taker olurdu, re-quote.
                "EXPIRED" | "REJECTED" => {
                    if !last {
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                    return Err(VenueError::Api(format!(
                        "maker giriş: GTX anında iptal (taker olurdu) [{}] @ {}",
                        symbol, price
                    )));
                }
                _ => {}
            }

            // NEW / PARTIALLY_FILLED → fill polling.
            let order_id = match resp.get("orderId").and_then(|v| v.as_u64()) {
                Some(id) => id,
                None => {
                    if !last {
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                    return Err(VenueError::Api(format!(
                        "maker giriş: orderId yok [{}]",
                        symbol
                    )));
                }
            };
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
            let mut filled: Option<Value> = None;
            loop {
                tokio::time::sleep(poll).await;
                let st = self.get_order_status(symbol, order_id).await?;
                match st
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("UNKNOWN")
                {
                    "FILLED" => {
                        filled = Some(st);
                        break;
                    }
                    "CANCELED" | "EXPIRED" | "REJECTED" => break,
                    _ => {
                        if std::time::Instant::now() >= deadline {
                            let _ = self.cancel_order(symbol, order_id).await;
                            break;
                        }
                    }
                }
            }
            if let Some(f) = filled {
                return Ok(f);
            }
            if last {
                return Err(VenueError::Api(format!(
                    "maker giriş: {} deneme sonunda fill yok [{}]",
                    attempts, symbol
                )));
            }
            tokio::time::sleep(backoff).await;
        }
        Err(VenueError::Api(format!(
            "maker giriş: beklenmedik akış [{}]",
            symbol
        )))
    }

    /// 🛡️ STOP-LOSS emri (pozisyonu trigger fiyatında kapatır).
    /// `side` pozisyonun KAPATMA yönü (long pozisyon için "SELL", short için "BUY").
    /// Futures: STOP_MARKET + reduceOnly. Spot: STOP_LOSS (market stop).
    pub async fn place_stop_loss_order(
        &self,
        symbol: &str,
        side: &str,
        qty: f64,
        stop_price: f64,
    ) -> Result<Value> {
        let path = if self.is_spot { "/api/v3/order" } else { "/fapi/v1/algoOrder" };
        let mut params = vec![
            format!("symbol={}", symbol),
            format!("side={}", side),
            format!("quantity={}", self.format_f64(qty)),
        ];
        if self.is_spot {
            // Spot: STOP_LOSS (market trigger) — stopPrice tetiklendiğinde market emir oluşur.
            params.push(format!("stopPrice={}", self.format_f64(stop_price)));
            params.push("type=STOP_LOSS".to_owned());
        } else {
            // Futures Algo API: koşullu emir tetiklenince mevcut pozisyonu kapatır.
            params.push("algoType=CONDITIONAL".to_owned());
            params.push("type=STOP_MARKET".to_owned());
            params.push(format!("triggerPrice={}", self.format_f64(stop_price)));
            params.push("reduceOnly=true".to_owned());
            params.push("workingType=MARK_PRICE".to_owned());
        }
        self.signed_request(Method::POST, path, params).await
    }

    /// 🎯 TAKE-PROFIT emri (kâr seviyesinde kapatır).
    /// `side` pozisyonun KAPATMA yönü.
    /// Futures: TAKE_PROFIT_MARKET + reduceOnly. Spot: TAKE_PROFIT (market trigger).
    pub async fn place_take_profit_order(
        &self,
        symbol: &str,
        side: &str,
        qty: f64,
        tp_price: f64,
    ) -> Result<Value> {
        let path = if self.is_spot { "/api/v3/order" } else { "/fapi/v1/algoOrder" };
        let mut params = vec![
            format!("symbol={}", symbol),
            format!("side={}", side),
            format!("quantity={}", self.format_f64(qty)),
        ];
        if self.is_spot {
            params.push(format!("stopPrice={}", self.format_f64(tp_price)));
            params.push("type=TAKE_PROFIT".to_owned());
        } else {
            params.push("algoType=CONDITIONAL".to_owned());
            params.push("type=TAKE_PROFIT_MARKET".to_owned());
            params.push(format!("triggerPrice={}", self.format_f64(tp_price)));
            params.push("reduceOnly=true".to_owned());
            params.push("workingType=MARK_PRICE".to_owned());
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
        let sl_res = self
            .place_stop_loss_order(symbol, close_side, qty, stop_loss)
            .await;
        let tp_res = self
            .place_take_profit_order(symbol, close_side, qty, take_profit)
            .await;
        (sl_res, tp_res)
    }

    pub async fn get_order_status(&self, symbol: &str, order_id: u64) -> Result<Value> {
        let path = if self.is_spot {
            "/api/v3/order"
        } else {
            "/fapi/v1/order"
        };
        self.signed_request(
            Method::GET,
            path,
            vec![
                format!("symbol={}", symbol),
                format!("orderId={}", order_id),
            ],
        )
        .await
    }

    pub async fn cancel_order(&self, symbol: &str, order_id: u64) -> Result<Value> {
        let path = if self.is_spot {
            "/api/v3/order"
        } else {
            "/fapi/v1/order"
        };
        self.signed_request(
            Method::DELETE,
            path,
            vec![
                format!("symbol={}", symbol),
                format!("orderId={}", order_id),
            ],
        )
        .await
    }

    pub async fn cancel_all_orders(&self, symbol: &str) -> Result<Value> {
        if self.is_spot {
            return self
                .signed_request(
                    Method::DELETE,
                    "/api/v3/openOrders",
                    vec![format!("symbol={}", symbol)],
                )
                .await;
        }
        self.signed_request(
            Method::DELETE,
            "/fapi/v1/allOpenOrders",
            vec![format!("symbol={}", symbol)],
        )
        .await?;
        self.signed_request(
            Method::DELETE,
            "/fapi/v1/algoOpenOrders",
            vec![format!("symbol={}", symbol)],
        )
        .await
    }

    /// Sembolün borsadaki açık emirlerini listele. Protection sync task bunu
    /// kullanarak SL veya TP'nin tetiklendiğini (emir kaybolması) yakalar.
    pub async fn get_open_orders(&self, symbol: &str) -> Result<Vec<Value>> {
        let path = if self.is_spot {
            "/api/v3/openOrders"
        } else {
            "/fapi/v1/openOrders"
        };
        let mut orders = self
            .signed_request(Method::GET, path, vec![format!("symbol={}", symbol)])
            .await
            .map_err(|e| VenueError::Network(e.to_string()))?
            .as_array()
            .cloned()
            .unwrap_or_default();
        if !self.is_spot {
            let algo_orders = self
                .signed_request(
                    Method::GET,
                    "/fapi/v1/openAlgoOrders",
                    vec![format!("symbol={}", symbol)],
                )
                .await
                .map_err(|e| VenueError::Network(e.to_string()))?;
            orders.extend(algo_orders.as_array().cloned().unwrap_or_default());
        }
        Ok(orders)
    }

    pub async fn get_account_positions(&self) -> Result<Vec<Value>> {
        let path = if self.is_spot {
            "/api/v3/account"
        } else {
            "/fapi/v2/positionRisk"
        };
        let resp = self.signed_request(Method::GET, path, vec![]).await?;
        Ok(resp.as_array().cloned().unwrap_or_default())
    }

    // === User Data Stream (WebSocket fill event'leri için) ===

    /// listenKey al — userDataStream WS endpoint'inin anahtarı.
    /// İmzalı değil; sadece X-MBX-APIKEY header'ı gerektirir. Anahtar 60 dk sonra
    /// expire olur, keepalive_listen_key ile yenilenmeli.
    pub async fn create_listen_key(&self) -> Result<String> {
        let path = if self.is_spot {
            "/api/v3/userDataStream"
        } else {
            "/fapi/v1/listenKey"
        };
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .client
            .post(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| VenueError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(VenueError::Api(format!(
                "listenKey hatası: {}",
                resp.text()
                    .await
                    .map_err(|e| VenueError::Network(e.to_string()))?
            )));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| VenueError::Network(e.to_string()))?;
        v.get("listenKey")
            .and_then(|k| k.as_str())
            .map(|s| s.to_owned())
            .ok_or_else(|| VenueError::Api("listenKey alanı yok".into()))
    }

    /// listenKey'i 60 dk daha uzat. WS task'ı bunu her 30 dk'da çağırmalı.
    pub async fn keepalive_listen_key(&self, listen_key: &str) -> Result<()> {
        let (path, body) = if self.is_spot {
            (
                format!("/api/v3/userDataStream?listenKey={}", listen_key),
                None,
            )
        } else {
            // Futures keepalive POST ile listen_key'siz çalışır; aktif anahtarı yeniler.
            ("/fapi/v1/listenKey".to_string(), Some(()))
        };
        let _ = body;
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .client
            .put(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| VenueError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(VenueError::Api(format!(
                "listenKey keepalive: {}",
                resp.text()
                    .await
                    .map_err(|e| VenueError::Network(e.to_string()))?
            )));
        }
        Ok(())
    }

    /// WebSocket URL'si — listenKey ile.
    pub fn user_data_stream_url(&self, listen_key: &str) -> String {
        if self.is_spot {
            return format!("wss://stream.binance.com:9443/ws/{}", listen_key);
        } else {
            // Futures testnet: wss://stream.binancefuture.com/ws/{listenKey}
            // Production:     wss://fstream.binance.com/ws/{listenKey}
            if self.is_paper {
                return format!("wss://stream.binancefuture.com/ws/{}", listen_key);
            } else {
                return format!("wss://fstream.binance.com/ws/{}", listen_key);
            }
        }
    }

    pub async fn get_balance(&self) -> Result<f64> {
        let path = if self.is_spot {
            "/api/v3/account"
        } else {
            "/fapi/v2/account"
        };
        let resp = self.signed_request(Method::GET, path, vec![]).await?;
        let val = if self.is_spot {
            resp["balances"]
                .as_array()
                .and_then(|l| l.iter().find(|a| a["asset"] == "USDT"))
                .and_then(|u| u["free"].as_str())
        } else {
            resp["totalWalletBalance"].as_str()
        };
        Ok(val.and_then(|s| s.parse().ok()).unwrap_or(0.0))
    }

    /// Hesap bakiyesi, equity ve kullanılabilir teminatı tek REST cevabından döndürür.
    pub async fn get_account_snapshot(&self) -> Result<Value> {
        let path = if self.is_spot {
            "/api/v3/account"
        } else {
            "/fapi/v2/account"
        };
        self.signed_request(Method::GET, path, vec![]).await
    }

    pub async fn get_positions(&self, symbol: &str) -> Result<Vec<Value>> {
        if self.is_spot {
            return Ok(vec![]);
        }
        let resp = self
            .signed_request(
                Method::GET,
                "/fapi/v2/positionRisk",
                vec![format!("symbol={}", symbol)],
            )
            .await?;
        Ok(resp.as_array().cloned().unwrap_or_default())
    }

    /// Borsadaki TÜM açık futures pozisyonlarını döndürür (yalnız positionAmt != 0).
    /// Restart reconciliation için — borsa otoritedir. Spot → boş (spot'ta "pozisyon" =
    /// bakiye, ayrı bir model; futures-only reconcile).
    pub async fn get_all_positions(&self) -> Result<Vec<Value>> {
        if self.is_spot {
            return Ok(vec![]);
        }
        let resp = self
            .signed_request(Method::GET, "/fapi/v2/positionRisk", vec![])
            .await?;
        Ok(resp
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| {
                p.get("positionAmt")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|a| a.abs() > 0.0)
                    .unwrap_or(false)
            })
            .collect())
    }

    pub async fn close_position(&self, symbol: &str) -> Result<Value> {
        let pos = self.get_positions(symbol).await?;
        let qty = pos
            .first()
            .and_then(|p| p["positionAmt"].as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        if qty.abs() < f64::EPSILON {
            return Err(VenueError::Api("Pozisyon kapalı".to_owned()));
        }
        self.place_market_order(symbol, if qty > 0.0 { "SELL" } else { "BUY" }, qty.abs())
            .await
    }

    pub async fn fetch_book_ticker(&self, symbol: &str) -> Result<(f64, f64)> {
        if self.is_paper {
            return Ok((0.0, 0.0));
        }
        let path = if self.is_spot {
            format!("/api/v3/ticker/bookTicker?symbol={}", symbol)
        } else {
            format!("/fapi/v1/ticker/bookTicker?symbol={}", symbol)
        };
        let v: Value = self
            .client
            .get(format!("{}{}", self.base_url, path))
            .send()
            .await
            .map_err(|e| VenueError::Network(e.to_string()))?
            .json()
            .await
            .map_err(|e| VenueError::Network(e.to_string()))?;
        let get_f = |k: &str| v[k].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        Ok((get_f("bidPrice"), get_f("askPrice")))
    }

    pub fn log_order(&self, symbol: &str, side: &str, qty: f64, price: f64) -> String {
        return format!(
            "[{}] Order: {} {} qty={} @ {}",
            if self.is_paper { "PAPER" } else { "LIVE" },
            side,
            symbol,
            qty,
            price
        );
    }

    // === ExchangeInfo / LotSize / MinNotional filtreleri ===

    /// 🧮 Binance exchangeInfo'dan tek sembolün filtrelerini çeker (LOT_SIZE,
    /// PRICE_FILTER, MIN_NOTIONAL / NOTIONAL). İmza gerektirmez (public endpoint).
    /// Çağıran genelde `apply_filters` üzerinden çağırır; bu metod cache'i atlatır.
    pub async fn fetch_symbol_filters(&self, symbol: &str) -> Result<SymbolFilters> {
        let path = if self.is_spot {
            "/api/v3/exchangeInfo"
        } else {
            "/fapi/v1/exchangeInfo"
        };
        let url = format!("{}{}?symbol={}", self.base_url, path, symbol);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| VenueError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(VenueError::Api(format!(
                "exchangeInfo {}: {}",
                symbol,
                resp.text()
                    .await
                    .map_err(|e| VenueError::Network(e.to_string()))?
            )));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| VenueError::Network(e.to_string()))?;
        let arr = v.get("symbols").and_then(|s| s.as_array()).ok_or_else(|| {
            VenueError::Api(format!("exchangeInfo: symbols dizisi yok ({})", symbol))
        })?;
        let sym = arr
            .iter()
            .find(|s| s.get("symbol").and_then(|x| x.as_str()) == Some(symbol))
            .ok_or_else(|| VenueError::Api(format!("exchangeInfo: {} sembolü dönmedi", symbol)))?;

        let filters = sym
            .get("filters")
            .and_then(|f| f.as_array())
            .ok_or_else(|| VenueError::Api(format!("exchangeInfo: filters yok ({})", symbol)))?;

        let mut out = SymbolFilters::default();
        for f in filters {
            let kind = f.get("filterType").and_then(|x| x.as_str()).unwrap_or("");
            let parse = |k: &str| -> f64 {
                f.get(k)
                    .and_then(|x| x.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.0)
            };
            match kind {
                "LOT_SIZE" | "MARKET_LOT_SIZE" => {
                    // MARKET_LOT_SIZE futures'ta MARKET emirler için ayrı; varsa onu önceleriz.
                    let step = parse("stepSize");
                    let minq = parse("minQty");
                    if kind == "MARKET_LOT_SIZE" || out.step_size <= 0.0 {
                        out.step_size = step;
                        out.min_qty = minq;
                    }
                }
                "PRICE_FILTER" => {
                    out.tick_size = parse("tickSize");
                }
                "MIN_NOTIONAL" => {
                    out.min_notional = parse("minNotional");
                }
                "NOTIONAL" => {
                    // Futures: NOTIONAL.notional veya minNotional alanı olabilir.
                    let n = parse("notional");
                    let mn = parse("minNotional");
                    out.min_notional = if n > 0.0 { n } else { mn };
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// Cache yardımcısı: sembolün filtreleri yoksa çeker ve mühürler.
    /// Live mode'da `apply_filters`'ın önyüzü.
    pub async fn ensure_filters(&self, symbol: &str) -> Result<SymbolFilters> {
        if let Ok(map) = self.filters.read() {
            if let Some(f) = map.get(symbol) {
                return Ok(f.clone());
            }
        }
        let f = self.fetch_symbol_filters(symbol).await?;
        if let Ok(mut map) = self.filters.write() {
            map.insert(symbol.to_string(), f.clone());
        }
        Ok(f)
    }

    /// 🛡️ Emir öncesi qty'yi borsa filtrelerinden geçirir.
    /// Dönüş: yuvarlanmış qty (Ok) veya red sebebi (Err string).
    /// Cache'te kayıt yoksa exchangeInfo'dan çekilir. Hata olursa filtreler atlanır
    /// ve qty olduğu gibi döner (Binance reddederse de WS REJECTED event'ine düşer).
    pub async fn apply_filters(
        &self,
        symbol: &str,
        qty: f64,
        price: f64,
    ) -> std::result::Result<f64, String> {
        let filters = match self.ensure_filters(symbol).await {
            Ok(f) => f,
            Err(e) => return Err(format!("exchangeInfo çekilemedi ({}): {:?}", symbol, e)),
        };
        filters.validate(qty, price)
    }
}

use std::sync::Arc;

pub struct BinanceTradeExecutor {
    // Projenizin halihazırda kullandığı ham Binance sipariş motorunu içeri alıyoruz
    pub raw_executor: Arc<BinanceFuturesExecutor>,
}

impl BinanceTradeExecutor {
    pub fn new(executor: Arc<BinanceFuturesExecutor>) -> Self {
        Self {
            raw_executor: executor,
        }
    }
}

impl TradeExecutor for BinanceTradeExecutor {
    fn execute(
        &self,
        _signal: Signal,
        symbol: &str,
        amount: f64,
    ) -> memos_venue_api::Result<Trade> {
        // NOT: Burası ileride BinanceFuturesExecutor'ın kendi iç fonksiyonunu çağıracak.
        // Şimdilik derleyicinin hatasız ayağa kalkması için Trade şemanıza uygun boş veri dönüyoruz.
        Ok(Trade {
            id: Some(0), // Canlıda borsadan dönen orderId yazılacak
            symbol: symbol.to_string(),
            entry_price: 0.0,
            exit_price: None,
            amount: amount.abs(),
            entry_time: chrono::Utc::now(),
            exit_time: None,
            pnl: None,
            pnl_pct: None,
            strategy: "Binance_Futures".to_string(),
        })
    }

    fn cancel_all(&self, _symbol: &str) -> memos_venue_api::Result<()> {
        // İleride raw_executor üzerinden emir iptalleri tetiklenecek
        Ok(())
    }

    fn fetch_open_symbols(&self) -> Vec<String> {
        // İleride raw_executor üzerinden Binance'teki açık pozisyonlar listelenecek
        return vec![];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_order_ids_are_unique_and_bounded() {
        let executor = BinanceFuturesExecutor::new_for_market(
            "key".into(),
            "secret".into(),
            true,
            "futures",
        );
        let first = executor.next_client_order_id("BTC/USDT");
        let second = executor.next_client_order_id("BTC/USDT");
        assert_ne!(first, second);
        assert!(first.len() <= 36);
        assert!(first.starts_with("memos_BTCUSDT_"));
    }

    #[test]
    fn maker_price_joins_touch() {
        // Long best_bid'e, short best_ask'a katılır (spread'e girmez = maker).
        assert_eq!(
            BinanceFuturesExecutor::maker_limit_price(true, 100.0, 100.5),
            100.0
        );
        assert_eq!(
            BinanceFuturesExecutor::maker_limit_price(false, 100.0, 100.5),
            100.5
        );
    }

    #[test]
    fn spread_bps_basic() {
        // bid 100, ask 100.1 → mid 100.05 → 0.1/100.05*1e4 ≈ 9.995
        let s = BinanceFuturesExecutor::spread_bps(100.0, 100.1);
        assert!((s - 9.995).abs() < 0.01, "spread={s}");
    }

    #[test]
    fn spread_bps_invalid_quote_is_zero() {
        assert_eq!(BinanceFuturesExecutor::spread_bps(0.0, 0.0), 0.0); // kota yok
        assert_eq!(BinanceFuturesExecutor::spread_bps(100.0, 100.0), 0.0); // ask==bid
        assert_eq!(BinanceFuturesExecutor::spread_bps(100.0, 99.0), 0.0); // ask<bid (bozuk)
    }

    /// B#3a regresyon kilidi: base_url DOĞRU Binance API host'larına çözülmeli
    /// (eski hatalı `binance.com`/`binancefuture.com` gerçek emri yanlış adrese gönderirdi).
    #[test]
    fn base_url_resolves_to_correct_binance_hosts() {
        let mk = |paper: bool, market: &str| {
            BinanceFuturesExecutor::new_for_market("k".into(), "s".into(), paper, market).base_url
        };
        assert_eq!(mk(false, "futures"), "https://fapi.binance.com"); // futures canlı
        assert_eq!(mk(true, "futures"), "https://testnet.binancefuture.com"); // futures testnet
        assert_eq!(mk(false, "spot"), "https://api.binance.com"); // spot canlı
        assert_eq!(mk(true, "spot"), "https://testnet.binance.vision"); // spot testnet
    }

    /// Bölge/IP/izin bloğu (sistemik) → true; normal emir reddi (-4120/-2010/yetersiz
    /// bakiye, HTTP 400) → false. Devre-kesici yanlış tripleyip canlıyı durdurmasın.
    #[test]
    fn exchange_block_classifies_systemic_vs_order_reject() {
        // Sistemik bloklar → true
        assert!(BinanceFuturesExecutor::is_exchange_block(451, "")); // bölge/legal
        assert!(BinanceFuturesExecutor::is_exchange_block(403, "blocked")); // WAF
        assert!(BinanceFuturesExecutor::is_exchange_block(418, "")); // IP auto-ban
        assert!(BinanceFuturesExecutor::is_exchange_block(429, "")); // rate-limit
        assert!(BinanceFuturesExecutor::is_exchange_block(
            401,
            r#"{"code":-2015,"msg":"Invalid API-key, IP, or permissions"}"#
        ));
        assert!(BinanceFuturesExecutor::is_exchange_block(
            200,
            r#"{"code":-1003,"msg":"Too many requests; IP banned"}"#
        ));
        // Normal emir reddi (HTTP 400, -41xx/-20xx) → false (pozisyon yolu etkilenmez)
        assert!(!BinanceFuturesExecutor::is_exchange_block(
            400,
            r#"{"code":-4120,"msg":"Order type not supported"}"#
        ));
        assert!(!BinanceFuturesExecutor::is_exchange_block(
            400,
            r#"{"code":-2010,"msg":"Account has insufficient balance"}"#
        ));
        assert!(!BinanceFuturesExecutor::is_exchange_block(
            400,
            "Filter failure: LOT_SIZE"
        ));
    }
}
