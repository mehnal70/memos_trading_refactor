// robot/state/symbol_orchestrator.rs - Çoklu Sembol Orkestrasyon Merkezi
//
// Modernizasyon Notları:
// 1. Thread-Safe yaşam döngüsü yönetimi (AtomicBool & Arc)
// 2. Fonksiyonel Map-Reducer ile portföy PnL hesabı
// 3. TUI için optimize edilmiş telemetri modelleri
// 4. Graceful Shutdown (Durdurma) garantisi

use crate::robot::state::{LivePositionMap, LivePriceData};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};
use std::time::Instant;

// --- 1. WORKER HANDLE (SINYAL HATTI) ---

pub struct SymbolHandle {
    pub symbol: String,
    pub market: String,
    pub interval: String,
    pub stop_signal: Arc<AtomicBool>,
    pub pause_signal: Arc<AtomicBool>,
    pub live_price: Arc<RwLock<LivePriceData>>,
    pub started_at: Instant,
}

impl SymbolHandle {
    pub fn new(symbol: &str, market: &str, interval: &str) -> Self {
        return Self {
            symbol: symbol.to_string(),
            market: market.to_string(),
            interval: interval.to_string(),
            stop_signal: Arc::new(AtomicBool::new(false)),
            pause_signal: Arc::new(AtomicBool::new(false)),
            live_price: Arc::new(RwLock::new(LivePriceData::default())),
            started_at: Instant::now(),
        };
    }

    pub fn uptime_secs(&self) -> u64 {
        return self.started_at.elapsed().as_secs();
    }

    pub fn is_paused(&self) -> bool {
        return self.pause_signal.load(Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.stop_signal.store(true, Ordering::Relaxed);
    }

    pub fn pause(&self) {
        self.pause_signal.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.pause_signal.store(false, Ordering::Relaxed);
    }
}

// --- 2. ORCHESTRATOR (ANA KOMUTA) ---

pub struct SymbolOrchestrator {
    pub workers: HashMap<String, SymbolHandle>,
    pub max_workers: usize,
    pub live_positions: Arc<RwLock<LivePositionMap>>,
}

impl SymbolOrchestrator {
    pub fn new(max_workers: usize, live_positions: Arc<RwLock<LivePositionMap>>) -> Self {
        return Self {
            workers: HashMap::new(),
            max_workers,
            live_positions,
        };
    }

    pub fn register(
        &mut self,
        symbol: &str,
        market: &str,
        interval: &str,
    ) -> Option<(Arc<AtomicBool>, Arc<AtomicBool>, Arc<RwLock<LivePriceData>>)> {
        match (
            self.workers.len() < self.max_workers,
            self.workers.contains_key(symbol),
        ) {
            (false, false) => return None,
            _ => {
                if let Some(old) = self.workers.remove(symbol) {
                    old.stop();
                }

                let handle = SymbolHandle::new(symbol, market, interval);
                let out = (
                    Arc::clone(&handle.stop_signal),
                    Arc::clone(&handle.pause_signal),
                    Arc::clone(&handle.live_price),
                );
                let _ = self.workers.insert(symbol.to_string(), handle);
                return Some(out);
            }
        }
    }

    pub fn stop_symbol(&mut self, symbol: &str) -> bool {
        return self
            .workers
            .remove(symbol)
            .map(|h| {
                h.stop();
                return true;
            })
            .unwrap_or(false);
    }

    pub fn stop_all(&mut self) {
        self.workers.drain().for_each(|(_, h)| h.stop());
    }

    pub fn build_price_map(&self, extra: Option<&RwLock<LivePriceData>>) -> HashMap<String, f64> {
        let mut map = HashMap::with_capacity(self.workers.len() + 1);

        if let Some(Ok(pd)) = extra.map(|arc| arc.read()) {
            if pd.close > 0.0 {
                let _ = map.insert(pd.symbol.clone(), pd.close);
            }
        }

        self.workers.iter().for_each(|(sym, h)| {
            if let Ok(pd) = h.live_price.read() {
                if pd.close > 0.0 {
                    let _ = map.insert(sym.clone(), pd.close);
                }
            }
        });
        return map;
    }
    pub fn total_open_pnl(&self, primary_price: Option<&RwLock<LivePriceData>>) -> f64 {
        let price_map = self.build_price_map(primary_price);
        let Ok(positions) = self.live_positions.read() else {
            return 0.0;
        };

        return positions
            .values()
            .map(|p| {
                let cur = price_map
                    .get(&p.symbol)
                    .copied()
                    .filter(|&v| v > 0.0)
                    .unwrap_or(p.current_price);
                return pos_pnl(cur, p.entry_price, p.qty, p.is_long);
            })
            .sum();
    }

    pub fn get_worker_status(&self) -> Vec<WorkerStatus> {
        let mut v: Vec<_> = self
            .workers
            .values()
            .map(|h| WorkerStatus {
                symbol: h.symbol.clone(),
                market: h.market.clone(),
                interval: h.interval.clone(),
                uptime_secs: h.uptime_secs(),
                paused: h.is_paused(),
            })
            .collect();
        v.sort_by(|a, b| a.symbol.cmp(&b.symbol));
        return v;
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
        return format!(
            "{:<12} {:<8} {:<5} {:>10}  {}",
            self.symbol,
            self.market,
            self.interval,
            format_uptime(self.uptime_secs),
            if self.paused { "DURAKLADI" } else { "AKTİF" }
        );
    }
}

#[inline]
pub fn pos_pnl(cur: f64, entry: f64, qty: f64, is_long: bool) -> f64 {
    if is_long {
        return (cur - entry) * qty;
    } else {
        return (entry - cur) * qty;
    }
}

fn format_uptime(secs: u64) -> String {
    return format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    );
}
