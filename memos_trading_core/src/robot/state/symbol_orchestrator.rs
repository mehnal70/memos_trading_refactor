// robot/symbol_orchestrator.rs
// Çoklu sembol worker yaşam döngüsü yöneticisi.
// Her sembol bağımsız bir RoboticLoop thread'inde çalışır;
// bu modül onları spawn/stop/pause eder ve portfolio risk hesaplar.


// robot/state/symbol_orchestrator.rs - Srivastava ATP Çoklu Sembol Orkestrasyon Merkezi
//
// Modernizasyon Notları:
// 1. Thread-Safe yaşam döngüsü yönetimi (AtomicBool & Arc)
// 2. Fonksiyonel Map-Reducer ile portföy PnL hesabı
// 3. TUI için optimize edilmiş telemetri modelleri
// 4. Graceful Shutdown (Durdurma) garantisi

use std::collections::HashMap;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc, RwLock};
use std::time::Instant;
use crate::robot::state::{LivePriceData, LivePositionMap};

// --- 1. WORKER HANDLE (SINYAL HATTI) ---

pub struct SymbolHandle {
    pub symbol:       String,
    pub market:       String,
    pub interval:     String,
    pub stop_signal:  Arc<AtomicBool>,
    pub pause_signal: Arc<AtomicBool>,
    pub live_price:   Arc<RwLock<LivePriceData>>,
    pub started_at:   Instant,
}

impl SymbolHandle {
    pub fn new(symbol: &str, market: &str, interval: &str) -> Self {
        Self {
            symbol:      symbol.to_string(),
            market:      market.to_string(),
            interval:    interval.to_string(),
            stop_signal: Arc::new(AtomicBool::new(false)),
            pause_signal: Arc::new(AtomicBool::new(false)),
            live_price:  Arc::new(RwLock::new(LivePriceData::default())),
            started_at:  Instant::now(),
        }
    }

    pub fn uptime_secs(&self) -> u64 { self.started_at.elapsed().as_secs() }
    pub fn is_paused(&self) -> bool { self.pause_signal.load(Ordering::Relaxed) }
    pub fn stop(&self)    { self.stop_signal.store(true, Ordering::Relaxed); }
    pub fn pause(&self)   { self.pause_signal.store(true, Ordering::Relaxed); }
    pub fn resume(&self)  { self.pause_signal.store(false, Ordering::Relaxed); }
}

// --- 2. ORCHESTRATOR (ANA KOMUTA) ---

pub struct SymbolOrchestrator {
    pub workers:        HashMap<String, SymbolHandle>,
    pub max_workers:    usize,
    pub live_positions: Arc<RwLock<LivePositionMap>>,
}

impl SymbolOrchestrator {
    pub fn new(max_workers: usize, live_positions: Arc<RwLock<LivePositionMap>>) -> Self {
        Self { workers: HashMap::new(), max_workers, live_positions }
    }

    // -- Kayıt ve Yaşam Döngüsü --

    pub fn register(&mut self, symbol: &str, market: &str, interval: &str) 
        -> Option<(Arc<AtomicBool>, Arc<AtomicBool>, Arc<RwLock<LivePriceData>>)> 
    {
        // Kapasite Denetimi (Match Guard)
        match (self.workers.len() < self.max_workers, self.workers.contains_key(symbol)) {
            (false, false) => None,
            _ => {
                // Eski worker varsa durdur ve temizle
                if let Some(old) = self.workers.remove(symbol) { old.stop(); }
                
                let handle = SymbolHandle::new(symbol, market, interval);
                let out = (Arc::clone(&handle.stop_signal), Arc::clone(&handle.pause_signal), Arc::clone(&handle.live_price));
                self.workers.insert(symbol.to_string(), handle);
                Some(out)
            }
        }
    }

    pub fn stop_symbol(&mut self, symbol: &str) -> bool {
        self.workers.remove(symbol).map(|h| { h.stop(); true }).unwrap_or(false)
    }

    pub fn stop_all(&mut self) {
        self.workers.drain().for_each(|(_, h)| h.stop());
    }

    // -- Fiyat ve Risk Analitiği --

    /// Tüm worker'lardan anlık fiyat haritası oluşturur (Zero-clone odaklı)
    pub fn build_price_map(&self, extra: Option<&RwLock<LivePriceData>>) -> HashMap<String, f64> {
        let mut map = HashMap::with_capacity(self.workers.len() + 1);
        
        // Ekstra fiyat verisi (Primary symbol)
        if let Some(Ok(pd)) = extra.map(|arc| arc.read()) {
            if pd.close > 0.0 { map.insert(pd.symbol.clone(), pd.close); }
        }

        // Worker fiyatları
        self.workers.iter().for_each(|(sym, h)| {
            if let Ok(pd) = h.live_price.read() {
                if pd.close > 0.0 { map.insert(sym.clone(), pd.close); }
            }
        });
        map
    }

    /// Portföy seviyesinde anlık PnL (USDT)
    pub fn total_open_pnl(&self, primary_price: Option<&RwLock<LivePriceData>>) -> f64 {
        let price_map = self.build_price_map(primary_price);
        let Ok(positions) = self.live_positions.read() else { return 0.0 };

        positions.values().map(|p| {
            let cur = price_map.get(&p.symbol).copied().filter(|&v| v > 0.0).unwrap_or(p.current_price);
            pos_pnl(cur, p.entry_price, p.qty, p.is_long)
        }).sum()
    }

    // -- Telemetri (TUI) --

    pub fn get_worker_status(&self) -> Vec<WorkerStatus> {
        let mut v: Vec<_> = self.workers.values().map(|h| WorkerStatus {
            symbol: h.symbol.clone(),
            market: h.market.clone(),
            interval: h.interval.clone(),
            uptime_secs: h.uptime_secs(),
            paused: h.is_paused(),
        }).collect();
        v.sort_by(|a, b| a.symbol.cmp(&b.symbol));
        v
    }
}

// --- 3. YARDIMCI MODELLER ---

#[derive(Debug, Clone)]
pub struct WorkerStatus {
    pub symbol: String,
    pub market: String,
    pub interval: String,
    pub uptime_secs: u64,
    pub paused: bool,
}

impl WorkerStatus {
    pub fn format_row(&self) -> String {
        format!("{:<12} {:<8} {:<5} {:>10}  {}",
            self.symbol, self.market, self.interval, 
            format_uptime(self.uptime_secs),
            if self.paused { "DURAKLADI" } else { "AKTİF" })
    }
}

#[inline]
pub fn pos_pnl(cur: f64, entry: f64, qty: f64, is_long: bool) -> f64 {
    if is_long { (cur - entry) * qty } else { (entry - cur) * qty }
}

fn format_uptime(secs: u64) -> String {
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}
