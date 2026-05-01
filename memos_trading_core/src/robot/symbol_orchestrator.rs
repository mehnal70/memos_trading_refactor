// robot/symbol_orchestrator.rs
// Çoklu sembol worker yaşam döngüsü yöneticisi.
// Her sembol bağımsız bir RoboticLoop thread'inde çalışır;
// bu modül onları spawn/stop/pause eder ve portfolio risk hesaplar.

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};
use std::time::Instant;

use crate::robot::robotic_loop::{LivePriceData, LivePositionMap};

// ─── Per-sembol çalışma zamanı tutucusu ──────────────────────────────────────

pub struct SymbolHandle {
    pub symbol:       String,
    pub market:       String,   // "spot" | "futures"
    pub interval:     String,   // "1m" | "1h" vb.
    /// Bu worker'a ait durdurma sinyali (true → loop sona erer)
    pub stop_signal:  Arc<AtomicBool>,
    /// Bu worker'a ait duraklatma sinyali
    pub pause_signal: Arc<AtomicBool>,
    /// Bu worker'ın yazdığı canlı fiyat — TUI aktif sembol için okur
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

    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    pub fn is_paused(&self) -> bool {
        self.pause_signal.load(Ordering::Relaxed)
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

// ─── Orchestrator ─────────────────────────────────────────────────────────────

pub struct SymbolOrchestrator {
    /// Aktif worker'lar — key: sembol adı ("BTCUSDT")
    pub workers:     HashMap<String, SymbolHandle>,
    /// Eş zamanlı maksimum sembol sayısı
    pub max_workers: usize,
    /// Tüm semboller tarafından paylaşılan açık pozisyon haritası
    pub live_positions: Arc<RwLock<LivePositionMap>>,
}

impl SymbolOrchestrator {
    pub fn new(max_workers: usize, live_positions: Arc<RwLock<LivePositionMap>>) -> Self {
        Self {
            workers: HashMap::new(),
            max_workers,
            live_positions,
        }
    }

    // ── Sorgu yardımcıları ────────────────────────────────────────────────────

    pub fn is_running(&self, symbol: &str) -> bool {
        self.workers.contains_key(symbol)
    }

    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    pub fn can_spawn(&self) -> bool {
        self.workers.len() < self.max_workers
    }

    pub fn active_symbols(&self) -> Vec<String> {
        let mut syms: Vec<String> = self.workers.keys().cloned().collect();
        syms.sort();
        syms
    }

    // ── Worker ekleme/kaldırma ────────────────────────────────────────────────

    /// Yeni bir SymbolHandle oluşturur ve kaydeder.
    /// Spawn logic (thread başlatma) çağıran tarafın sorumluluğundadır —
    /// bu fonksiyon sadece kaydı tutar ve Arc'ları döndürür.
    /// Kapasite doluysa ve sembol zaten çalışmıyorsa None döner.
    pub fn register(&mut self, symbol: &str, market: &str, interval: &str)
        -> Option<(Arc<AtomicBool>, Arc<AtomicBool>, Arc<RwLock<LivePriceData>>)>
    {
        if !self.can_spawn() && !self.workers.contains_key(symbol) {
            return None; // kapasite dolu
        }
        // Zaten varsa önce dur
        if let Some(old) = self.workers.remove(symbol) {
            old.stop();
        }
        let handle = SymbolHandle::new(symbol, market, interval);
        let stop   = Arc::clone(&handle.stop_signal);
        let pause  = Arc::clone(&handle.pause_signal);
        let price  = Arc::clone(&handle.live_price);
        self.workers.insert(symbol.to_string(), handle);
        Some((stop, pause, price))
    }

    /// Belirtilen sembol worker'ını durdurur ve map'ten kaldırır.
    /// Döndürülen bool: sembol gerçekten çalışıyorduysa true.
    pub fn stop_symbol(&mut self, symbol: &str) -> bool {
        if let Some(h) = self.workers.remove(symbol) {
            h.stop();
            true
        } else {
            false
        }
    }

    /// Tüm worker'ları durdurur.
    pub fn stop_all(&mut self) {
        for (_, h) in self.workers.drain() {
            h.stop();
        }
    }

    // ── Pause / Resume ────────────────────────────────────────────────────────

    pub fn pause_symbol(&self, symbol: &str) {
        if let Some(h) = self.workers.get(symbol) { h.pause(); }
    }

    pub fn resume_symbol(&self, symbol: &str) {
        if let Some(h) = self.workers.get(symbol) { h.resume(); }
    }

    pub fn pause_all(&self) {
        for h in self.workers.values() { h.pause(); }
    }

    pub fn resume_all(&self) {
        for h in self.workers.values() { h.resume(); }
    }

    // ── Fiyat erişimi ─────────────────────────────────────────────────────────

    /// Belirli sembolün canlı fiyat Arc'ını döndürür.
    pub fn live_price_for(&self, symbol: &str) -> Option<Arc<RwLock<LivePriceData>>> {
        self.workers.get(symbol).map(|h| Arc::clone(&h.live_price))
    }

    /// Tüm worker arc'larından taze `{sembol → close}` haritası oluşturur.
    /// Opsiyonel `extra` ile primary `live_price` arc'ı da eklenebilir.
    pub fn build_price_map(&self, extra: Option<&RwLock<LivePriceData>>) -> HashMap<String, f64> {
        let mut map = HashMap::new();
        if let Some(arc) = extra {
            if let Ok(pd) = arc.read() {
                if pd.close > 0.0 { map.insert(pd.symbol.clone(), pd.close); }
            }
        }
        for (sym, h) in &self.workers {
            if let Ok(pd) = h.live_price.read() {
                if pd.close > 0.0 { map.insert(sym.clone(), pd.close); }
            }
        }
        map
    }

    // ── Portfolio Risk ────────────────────────────────────────────────────────

    /// Tüm açık pozisyonların anlık PnL toplamını döndürür (USDT).
    /// `primary_price`: birincil sembolün live_price arc'ı — worker haritasında yoksa buradan alınır.
    pub fn total_open_pnl(&self, primary_price: Option<&RwLock<LivePriceData>>) -> f64 {
        let price_map = self.build_price_map(primary_price);
        let Ok(positions) = self.live_positions.read() else { return 0.0 };
        positions.values().map(|p| {
            let cur = price_map.get(&p.symbol).copied()
                .filter(|&v| v > 0.0)
                .unwrap_or(p.current_price);
            pos_pnl(cur, p.entry_price, p.qty, p.is_long)
        }).sum()
    }

    /// Her worker için (sembol, market, interval, uptime_secs, paused) bilgisi.
    pub fn worker_status(&self) -> Vec<WorkerStatus> {
        let mut v: Vec<WorkerStatus> = self.workers.values().map(|h| WorkerStatus {
            symbol:      h.symbol.clone(),
            market:      h.market.clone(),
            interval:    h.interval.clone(),
            uptime_secs: h.uptime_secs(),
            paused:      h.is_paused(),
        }).collect();
        v.sort_by(|a, b| a.symbol.cmp(&b.symbol));
        v
    }
}

// ─── Durum özeti (TUI için) ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WorkerStatus {
    pub symbol:      String,
    pub market:      String,
    pub interval:    String,
    pub uptime_secs: u64,
    pub paused:      bool,
}

impl WorkerStatus {
    /// TUI satır formatı: "BTCUSDT  futures  1h  02:14:33  Çalışıyor"
    pub fn format_row(&self) -> String {
        let uptime = format_uptime(self.uptime_secs);
        let state  = if self.paused { "Duraklatıldı" } else { "Çalışıyor" };
        format!("{:<12} {:<8} {:<5} {:>10}  {}",
            self.symbol, self.market, self.interval, uptime, state)
    }
}

/// Açık pozisyon PnL hesabı: long → (cur-entry)*qty, short → (entry-cur)*qty
#[inline]
pub fn pos_pnl(cur: f64, entry: f64, qty: f64, is_long: bool) -> f64 {
    if is_long { (cur - entry) * qty } else { (entry - cur) * qty }
}

/// Açık pozisyon PnL yüzdesi (entry > 0 garantisi gerekir)
#[inline]
pub fn pos_pnl_pct(cur: f64, entry: f64, is_long: bool) -> f64 {
    if entry > 0.0 {
        if is_long { (cur - entry) / entry * 100.0 } else { (entry - cur) / entry * 100.0 }
    } else { 0.0 }
}

fn format_uptime(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}
