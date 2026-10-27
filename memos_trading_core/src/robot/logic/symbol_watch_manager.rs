// symbol_watch_manager.rs
// Otonom Sembol ve Market İzleme / Pipeline Yönetimi

use crate::robot::logic::pipeline_supervisor::PipelineSupervisor;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock; // Mutex yerine okuma ağırlıklı performans için RwLock

/// Otomatik izlenen sembol ve marketlerin orkestratörü
#[derive(Debug, Default)]
pub struct SymbolWatchManager {
    /// İzlenen semboller seti - RwLock ile çoklu thread okumasına izin verir
    pub watched_symbols: Arc<RwLock<HashSet<String>>>,
    pub pipeline_supervisor: PipelineSupervisor,
}

impl SymbolWatchManager {
    /// Otomatik sembol keşfi ve öneri algoritması
    /// İleride ML-based 'Discovery Engine' buraya bağlanacak şekilde optimize edildi
    pub async fn discover_symbols(&self) -> Vec<String> {
        // Pipeline: Hacim ve volatiliteye göre dinamik seçim (Dummy Data)
        let all_symbols = [
            "BTCUSDT", "ETHUSDT", "SOLUSDT", "AVAXUSDT", "BNBUSDT", "DOGEUSDT", "XRPUSDT",
            "ADAUSDT", "LINKUSDT", "DOTUSDT",
        ];

        // İteratörlerle hızlı filtreleme ve allocation-optimized dönüş
        all_symbols
            .iter()
            .filter(|&&s| s.ends_with("USDT"))
            .take(5)
            .map(|&s| s.to_owned())
            .collect()
    }

    /// Sembol izlemeye başla - Otonom Pipeline tetikleyicisi
    pub async fn start_watch(&self, symbol: &str) {
        // Önce okuma kilidiyle kontrol et (Hızlı yol)
        {
            let watched = self.watched_symbols.read().await;
            if watched.contains(symbol) {
                return;
            }
        }

        // Kayıt ve pipeline başlatmayı ayır: harici/asenkron iş yazma kilidi altında
        // beklememeli.
        let inserted = {
            let mut watched = self.watched_symbols.write().await;
            watched.insert(symbol.to_owned())
        };
        if inserted {
            println!("[WATCH] + {} izleme listesine eklendi.", symbol);
            // PipelineSupervisor üzerinden otonom süreci başlat
            self.pipeline_supervisor.start_pipeline(symbol).await;
        }
    }

    /// Sembol izlemeyi durdur - Safe-shutdown mekanizması
    pub async fn stop_watch(&self, symbol: &str) {
        let removed = {
            let mut watched = self.watched_symbols.write().await;
            watched.remove(symbol)
        };
        if removed {
            println!("[WATCH] - {} izleme durduruldu.", symbol);
            // Pipeline güvenli bir şekilde kapatılır
            self.pipeline_supervisor.stop_pipeline(symbol);
        }
    }

    /// Tüm izlenen sembolleri ve pipeline sağlığını raporla
    pub async fn report_status(&self) {
        let symbols_list: Vec<String> = {
            let watched = self.watched_symbols.read().await;
            watched.iter().cloned().collect()
        };

        println!(
            "\n[STATUS] SymbolWatchManager | Aktif Semboller: {} | Liste: {:?}",
            symbols_list.len(),
            symbols_list
        );

        // Alt modül durum raporu
        self.pipeline_supervisor.status();
    }
}
