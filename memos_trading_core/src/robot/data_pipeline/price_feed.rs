//! Binance miniTicker WebSocket fiyat akışı — saniye bazlı gerçek zamanlı güncelleme.
//!
//! REST polling'den (10sn) bağımsız çalışır; `SharedTradingState.live_price` arc'ını
//! doğrudan günceller. Bağlantı kesilirse exponential backoff ile otomatik yeniden bağlanır.
//!
//! Spot  : `wss://stream.binance.com:9443/ws/<symbol>@miniTicker`
//! Futures: `wss://fstream.binance.com/ws/<symbol>@miniTicker`
//!
//! # Ping / Pong
//! Binance her ~3 dakikada bir Ping frame gönderir. Pong yanıtı gelmezse sunucu
//! veri akışını dondurur ve ardından bağlantıyı kapar. Bu nedenle `stream_once`
//! WebSocketStream'i **split etmez** — tek akış üzerinden hem okur hem Pong yazar.

// robot/data_pipeline/price_feed.rs - Srivastava ATP Gerçek Zamanlı Fiyat Akış Ünitesi

use crate::robot::state::SharedTradingState;
use crate::types::Market;
use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use serde::Deserialize;

#[derive(Deserialize)]
struct MiniTicker {
    #[serde(rename = "s")] symbol: String,
    #[serde(rename = "c")] close:  String,
    #[serde(rename = "o")] open:   String,
    #[serde(rename = "h")] high:   String,
    #[serde(rename = "l")] low:    String,
    #[serde(rename = "v")] volume: String,
}

pub struct RealtimePriceFeed {
    pub symbol:      String,
    pub market:      Market,
    pub live_state:  SharedTradingState,
    pub stop_signal: Arc<AtomicBool>,
}

impl RealtimePriceFeed {
    pub fn new(symbol: impl Into<String>, market: Market, live_state: SharedTradingState, stop_signal: Arc<AtomicBool>) -> Self {
        Self { symbol: symbol.into(), market, live_state, stop_signal }
    }

    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move { self.run().await })
    }

    pub async fn run(self) {
        let mut backoff = 1u64;
        while !self.stop_signal.load(Ordering::Relaxed) {
            match self.stream_once().await {
                Ok(_) => backoff = 1, // Temiz kapanışta resetle
                Err(e) => {
                    log::error!("WS Besleme Hatası [{}]: {}. {}sn içinde yeniden deneniyor.", self.symbol, e, backoff);
                    tokio::time::sleep(Duration::from_secs(backoff)).await;
                    backoff = (backoff * 2).min(60);
                }
            }
        }
    }

    async fn stream_once(&self) -> Result<(), String> {
        // End-point Otonomisi
        let base = match self.market {
            Market::Futures | Market::Coinm => "wss://fstream.binance.com/ws",
            _                               => "wss://stream.binance.com:9443/ws",
        };
        let url = format!("{}/{}@miniTicker", base, self.symbol.to_lowercase());

        let (mut ws, _) = connect_async(&url).await.map_err(|e| e.to_string())?;

        while let Some(msg_result) = ws.next().await {
            if self.stop_signal.load(Ordering::Relaxed) { break; }
            
            match msg_result.map_err(|e| e.to_string())? {
                Message::Text(text) => self.handle_ticker_data(&text),
                Message::Ping(data) => { ws.send(Message::Pong(data)).await.ok(); },
                Message::Close(_) => break,
                _ => {}
            }
        }
        Ok(())
    }

    /// Ham veriyi rafine eder ve SharedState'i otonom günceller.
    fn handle_ticker_data(&self, text: &str) {
        let Ok(tick) = serde_json::from_str::<MiniTicker>(text) else { return; };
        
        let parse = |s: &str| s.parse::<f64>().unwrap_or(0.0);
        let (close, open) = (parse(&tick.close), parse(&tick.open));

        if close > 0.0 {
            self.live_state.update_price(|pd| {
                pd.symbol     = tick.symbol.clone();
                pd.close      = close;
                pd.open       = open;
                pd.high       = parse(&tick.high);
                pd.low        = parse(&tick.low);
                pd.volume     = parse(&tick.volume);
                pd.change_pct = if open > 0.0 { (close - open) / open * 100.0 } else { 0.0 };
                pd.ts         = chrono::Local::now().format("%H:%M:%S").to_string();
                pd.last_updated_ms = crate::core::time::now_epoch_millis() as u64;
            });
        }
    }
}
