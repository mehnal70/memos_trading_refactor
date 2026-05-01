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

use crate::robot::robotic_loop::SharedTradingState;
use crate::types::Market;
use futures_util::{SinkExt, StreamExt};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};

// ── Binance miniTicker mesaj formatı ─────────────────────────────────────────
#[derive(serde::Deserialize)]
struct MiniTicker {
    #[serde(rename = "s")] symbol: String,
    #[serde(rename = "c")] close:  String,
    #[serde(rename = "o")] open:   String,
    #[serde(rename = "h")] high:   String,
    #[serde(rename = "l")] low:    String,
    #[serde(rename = "v")] volume: String,
}

// ── RealtimePriceFeed ─────────────────────────────────────────────────────────
/// Binance miniTicker WS akışı — `spawn()` ile arka planda başlatılır.
///
/// `stop_signal` set edilene kadar çalışır; bağlantı kesilirse
/// 1 → 2 → 4 → … → 60 sn backoff ile otomatik yeniden bağlanır.
pub struct RealtimePriceFeed {
    pub symbol:      String,
    pub market:      Market,
    pub live_state:  SharedTradingState,
    pub stop_signal: Arc<AtomicBool>,
}

impl RealtimePriceFeed {
    /// Yeni bir akış oluştur.
    pub fn new(
        symbol:      impl Into<String>,
        market:      Market,
        live_state:  SharedTradingState,
        stop_signal: Arc<AtomicBool>,
    ) -> Self {
        Self { symbol: symbol.into(), market, live_state, stop_signal }
    }

    /// Arka planda WS akışını başlat; `JoinHandle` döner.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move { self.run().await })
    }

    /// Stop signal set edilene kadar reconnect döngüsünde çalışır.
    pub async fn run(self) {
        let mut backoff = 1u64;
        loop {
            if self.stop_signal.load(Ordering::Relaxed) { break; }
            match self.stream_once().await {
                Ok(())  => { backoff = 1; }   // temiz kapanış — hemen yeniden bağlan
                Err(_e) => {
                    // Hata durumunda bekle, sonra tekrar dene
                    tokio::time::sleep(Duration::from_secs(backoff)).await;
                    backoff = (backoff * 2).min(60);
                }
            }
            if self.stop_signal.load(Ordering::Relaxed) { break; }
        }
    }

    /// Tek WS oturumu — mesaj geldiği sürece çalışır, bağlantı kesilince döner.
    ///
    /// WebSocketStream split **edilmez**: hem `StreamExt::next()` (okuma) hem
    /// `SinkExt::send()` (Pong yazma) aynı değişken üzerinden yapılır.
    /// Binance'in Ping frame'lerine Pong yanıtı verilmezse akış donar;
    /// bu fonksiyon her Ping için hemen Pong göndererek bağlantıyı canlı tutar.
    async fn stream_once(&self) -> Result<(), String> {
        let base = match self.market {
            Market::Futures | Market::Coinm => "wss://fstream.binance.com/ws",
            _                               => "wss://stream.binance.com:9443/ws",
        };
        let url = format!("{}/{}@miniTicker", base, self.symbol.to_lowercase());

        let (mut ws, _) = connect_async(&url).await
            .map_err(|e| format!("miniTicker WS bağlantı hatası: {e}"))?;

        while let Some(msg_result) = ws.next().await {
            if self.stop_signal.load(Ordering::Relaxed) { return Ok(()); }
            let msg = msg_result.map_err(|e| e.to_string())?;

            match msg {
                // ── Fiyat verisi ──────────────────────────────────────────────
                Message::Text(text) => {
                    let tick: MiniTicker = match serde_json::from_str(&text) {
                        Ok(t)  => t,
                        Err(_) => continue, // miniTicker dışı mesaj (nadiren gelebilir)
                    };
                    let close = tick.close.parse::<f64>().unwrap_or(0.0);
                    if close <= 0.0 { continue; }
                    let open = tick.open.parse::<f64>().unwrap_or(0.0);
                    self.live_state.update_price(|pd| {
                        pd.symbol     = tick.symbol.clone();
                        pd.close      = close;
                        pd.open       = open;
                        pd.high       = tick.high  .parse::<f64>().unwrap_or(pd.high);
                        pd.low        = tick.low   .parse::<f64>().unwrap_or(pd.low);
                        pd.volume     = tick.volume.parse::<f64>().unwrap_or(pd.volume);
                        pd.change_pct = if open > 0.0 { (close - open) / open * 100.0 } else { 0.0 };
                        pd.ts         = chrono::Local::now().format("%H:%M:%S").to_string();
                    });
                }

                // ── Ping → Pong (bağlantı canlı tutma) ───────────────────────
                // Binance ~3 dk'da bir Ping gönderir. Pong yanıtı verilmezse
                // sunucu veri akışını dondurur ve ~5 dk sonra bağlantıyı kapar.
                Message::Ping(data) => {
                    ws.send(Message::Pong(data)).await
                        .map_err(|e| format!("Pong gönderilemedi: {e}"))?;
                }

                // ── Sunucu bağlantıyı kapattı — temiz çıkış ──────────────────
                Message::Close(_) => break,

                // Binary, Pong vb. — yoksay
                _ => {}
            }
        }
        Ok(())
    }
}
