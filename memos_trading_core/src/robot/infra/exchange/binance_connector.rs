// binance_connector.rs - Borsa Trait Arayüzü ve Köprü Katmanı

use async_trait::async_trait;
use crate::robot::infra::exchange::exchange_connector::ExchangeConnector;
use crate::robot::engines::binance_executor::BinanceFuturesExecutor;
use crate::core::types::{Candle, Trade, Exchange, Market};
use crate::Result;
use serde_json::Value;

/// BinanceConnector: Üst seviye komutları (Trait) alt seviye motorlara (Executor) bağlar.
pub struct BinanceConnector {
    executor: BinanceFuturesExecutor,
    market_type: Market,
}

impl BinanceConnector {
    /// Yeni bir bağlantı oluşturur
    pub fn new(api_key: String, api_secret: String, is_paper: bool, market: Market) -> Self {
        let market_str = market.as_str(); // "spot", "futures" vb.
        Self {
            executor: BinanceFuturesExecutor::new_for_market(api_key, api_secret, is_paper, market_str),
            market_type: market,
        }
    }
}

#[async_trait]
impl ExchangeConnector for BinanceConnector {
    /// Borsa kimliğini döner
    fn exchange(&self) -> Exchange {
        Exchange::Binance
    }

    /// Borsa adını statik olarak döner (Loglama için)
    fn exchange_name(&self) -> &'static str {
        "Binance"
    }

    /// Borsadan ham mum verilerini çeker ve botun Candle tipine dönüştürür
    async fn fetch_candles(&self, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Candle>> {
        let path = if self.executor.is_spot { "/api/v3/klines" } else { "/fapi/v1/klines" };
        let url = format!("{}{}?symbol={}&interval={}&limit={}", self.executor.base_url, path, symbol, interval, limit);
        
        let resp: Vec<Value> = self.executor.client.get(&url).send().await?.json().await?;
        
        // Ham veriyi otonom Candle yapısına haritala (her kline iç array olarak gelir).
        Ok(resp.into_iter()
            .filter_map(|k| k.as_array().and_then(|arr|
                crate::persistence::writer::parse_binance_kline(arr.as_slice(), symbol, interval)
            ))
            .collect())
    }

    /// Yeni bir emir iletir (Market veya Limit/Post-Only seçimiyle)
    async fn place_order(&self, symbol: &str, side: &str, qty: f64, price: Option<f64>) -> Result<Trade> {
        let result = if let Some(p) = price {
            // Fiyat varsa düşük komisyonlu Maker emri gönder
            self.executor.place_post_only_limit_order(symbol, side, qty, p).await?
        } else {
            // Fiyat yoksa anlık Market emri gönder
            self.executor.place_market_order(symbol, side, qty).await?
        };

        // Borsadan gelen yanıtı botun Trade kaydına dönüştür
        Ok(Trade {
            symbol: symbol.to_owned(),
            entry_price: result["price"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0),
            amount: qty,
            strategy: "Autonomous_Execution".to_owned(),
            ..Default::default() 
        })
    }

    /// Tekil varlık bakiyesi sorgular
    async fn get_balance(&self, _asset: &str) -> Result<f64> {
        // Executor zaten market-aware bakiye hesaplıyor
        self.executor.get_balance().await
    }

    /// Tüm cüzdanı otonom olarak tarar
    async fn fetch_portfolio(&self) -> Result<Vec<(String, f64)>> {
        let bal = self.get_balance("USDT").await?;
        Ok(vec![("USDT".to_owned(), bal)])
    }

    /// Gerçek zamanlı veri akışını (WebSocket) başlatır
    async fn start_signal_stream(&self) -> Result<()> {
        log::info!("Binance WebSocket sinyal kanalları otonom olarak dinleniyor...");
        // TODO: robot/data_pipeline tarafındaki WS dinleyicileri buraya bağlanacak
        Ok(())
    }

    /// Pozisyon güncellemelerini takip eder
    async fn start_position_stream(&self) -> Result<()> {
        log::info!("Binance pozisyon takip akışı (User Data Stream) aktif.");
        Ok(())
    }
        /// İstek öncesi borsa bazlı rate-limit kontrolü yapar.
    /// Şimdilik her isteğe izin verir, ileride Executor'dan gelen 'weight' verisine göre dolacaktır.
    fn check_rate_limit(&self, _endpoint: &str) -> bool {
        // Gerçek implementasyonda: borsa kurallarına göre saniyelik limit kontrolü yapılır.
        true 
    }

    /// Bağlantı kopmalarında IP rotasyonu veya WebSocket'i yeniden başlatır.
    /// Mutable erişim gerektirdiğinden &mut self kullanılır.
    async fn handle_reconnect(&mut self) {
        log::warn!("Binance bağlantısı kopma sinyali aldı, otonom kurtarma prosedürü başlatılıyor...");
        // Re-handshake veya yeni WS session başlatma mantığı buraya gelir.
    }
}
