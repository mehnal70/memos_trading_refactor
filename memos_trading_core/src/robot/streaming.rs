// robot/streaming.rs - Gerçek Zamanlı Veri Akışı ve Ultra Düşük Gecikme
// WebSocket, event-driven pipeline, async veri işleme

use crate::types::Candle;
use async_trait::async_trait;

#[async_trait]
pub trait StreamingSource: Send + Sync {
    async fn subscribe(&self, symbol: &str, interval: &str, on_candle: Box<dyn Fn(Candle) + Send + Sync>);
    fn source_name(&self) -> &'static str;
}

pub struct DummyWebSocket;

#[async_trait]
impl StreamingSource for DummyWebSocket {
    async fn subscribe(&self, symbol: &str, interval: &str, on_candle: Box<dyn Fn(Candle) + Send + Sync>) {
        // Dummy: Her saniye yeni candle üret
            #[cfg(not(target_arch = "wasm32"))]
            {
                use tokio::time::{sleep, Duration};
                for i in 0..10 {
                    let candle = Candle {
                        timestamp: chrono::Utc::now(),
                        open: 100.0 + i as f64,
                        high: 101.0 + i as f64,
                        low: 99.0 + i as f64,
                        close: 100.5 + i as f64,
                        volume: 1.0,
                        symbol: symbol.to_string(),
                        interval: interval.to_string(),
                    };
                    on_candle(candle);
                    sleep(Duration::from_millis(500)).await;
                }
            }
    }
    fn source_name(&self) -> &'static str {
        "DummyWebSocket"
    }
}
