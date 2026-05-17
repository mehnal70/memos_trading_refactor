// robot/streaming.rs - Gerçek Zamanlı Veri Akışı ve Ultra Düşük Gecikme
// WebSocket, event-driven pipeline, async veri işleme

// robot/data_pipeline/streaming.rs - Srivastava ATP Canlı Veri Akış Arayüzü
//
// Modernizasyon Notları:
// 1. Trait-based asenkron abonelik mimarisi.
// 2. Fonksiyonel callback yönetimi.
// 3. Platform bağımsız (WASM/Native) çalışma desteği.
// 4. Test odaklı Dummy implementasyonu (Pattern Matching ile).

use crate::types::Candle;
use async_trait::async_trait;

/// §83.4: StreamingSource - Canlı veri kaynakları için anayasal standart.
#[async_trait]
pub trait StreamingSource: Send + Sync {
    async fn subscribe(&self, symbol: &str, interval: &str, on_candle: Box<dyn Fn(Candle) + Send + Sync>);
    fn source_name(&self) -> &'static str;
}

/// Geliştirme ve simülasyon süreçleri için sahte veri akış motoru.
pub struct DummyWebSocket;

#[async_trait]
impl StreamingSource for DummyWebSocket {
    async fn subscribe(&self, symbol: &str, interval: &str, on_candle: Box<dyn Fn(Candle) + Send + Sync>) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            use tokio::time::{sleep, Duration};
            use chrono::Utc;

            // Functional loop: Her 500ms'de simüle edilmiş veri üretir.
            for i in 0..10 {
                let base_price = 100.0 + i as f64;
                let candle = Candle {
                    timestamp: Utc::now(),
                    open:   base_price,
                    high:   base_price + 1.0,
                    low:    base_price - 1.0,
                    close:  base_price + 0.5,
                    volume: 1.0,
                    symbol: symbol.to_string(),
                    interval: interval.to_string(),
                };

                on_candle(candle);
                sleep(Duration::from_millis(500)).await;
            }
        }
    }

    fn source_name(&self) -> &'static str { "Dummy-Stream-v1" }
}

/// Global Erişim Yardımcısı (Factory Pattern)
pub struct StreamingFactory;
impl StreamingFactory {
    pub fn build_dummy() -> Box<dyn StreamingSource> {
        Box::new(DummyWebSocket)
    }
}
