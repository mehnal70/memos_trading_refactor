// SymbolWatchManager: Extreme auto trading için otomatik sembol/market izleme ve pipeline yönetimi
// Türkçe açıklamalar ile, insan müdahalesi olmadan çalışacak şekilde tasarlanmıştır.

use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::pipeline_supervisor::PipelineSupervisor;

/// Otomatik izlenen sembol ve marketlerin durumu

#[derive(Debug, Default)]
pub struct SymbolWatchManager {
    pub watched_symbols: Arc<Mutex<HashSet<String>>>,
    pub pipeline_supervisor: PipelineSupervisor,
}

impl SymbolWatchManager {
    /// Otomatik sembol keşfi ve öneri algoritması
    /// Hacim, volatilite ve trend analizi ile en iyi sembolleri önerir
    pub async fn discover_symbols(&self) -> Vec<String> {
        // TODO: Gerçek borsa API ile hacim, volatilite ve trend verisi çekilecek
        // Örnek dummy algoritma:
        let all_symbols = vec!["BTCUSDT", "ETHUSDT", "SOLUSDT", "AVAXUSDT", "BNBUSDT", "DOGEUSDT", "XRPUSDT", "ADAUSDT", "LINKUSDT", "DOTUSDT"];
        // Dummy: En yüksek hacimli ve volatil olanları seç
        let recommended = all_symbols.into_iter()
            .filter(|s| s.ends_with("USDT"))
            .take(5)
            .map(|s| s.to_string())
            .collect();
        recommended
    }

    /// Sembol izlemeye başla (pipeline otomatik başlatılır)
    pub async fn start_watch(&self, symbol: &str) {
        let mut watched = self.watched_symbols.lock().await;
        if watched.contains(symbol) {
            return;
        }
        watched.insert(symbol.to_string());
        // PipelineSupervisor ile otomatik pipeline başlat (artık sadece symbol parametresi alıyor)
        self.pipeline_supervisor.start_pipeline(symbol).await;
    }

    /// Sembol izlemeyi durdur (pipeline otomatik durdurulur)
    pub async fn stop_watch(&self, symbol: &str) {
        let mut watched = self.watched_symbols.lock().await;
        watched.remove(symbol);
        // PipelineSupervisor ile otomatik pipeline durdur
        self.pipeline_supervisor.stop_pipeline(symbol);
    }

    /// Tüm izlenen sembolleri ve pipeline durumunu göster
    pub async fn status(&self) {
        let watched = self.watched_symbols.lock().await;
        println!("[SymbolWatchManager] İzlenen semboller: {:?}", watched);
        self.pipeline_supervisor.status();
    }
}

// Kullanım örneği:
// let manager = SymbolWatchManager::default();
// manager.start_watch("BTCUSDT").await;
// manager.status();
// manager.stop_watch("BTCUSDT");
