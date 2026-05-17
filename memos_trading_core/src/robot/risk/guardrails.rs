// robot/risk/guardrails.rs - Srivastava ATP Birleştirilmiş Savunma Sistemi
//
// Bu dosya hem finansal sınırları (Drawdown/Slippage) hem de 
// sistemsel sağlığı (Watchdog) otonom yönetir.
use crate::prelude::*;
#[derive(Default)] pub struct Guardrails;
impl Guardrails { pub fn check_safety(&self, _sig: &Signal) -> bool { true } }



use chrono::{DateTime, Utc};

// --- 1. FİNANSAL GUARDRAILS (YENİ EKLENENLER) ---

#[derive(Debug, Clone)]
pub struct DrawdownMonitor {
    pub initial_equity: f64,
    pub peak_equity: f64,
    pub current_equity: f64,
    pub max_drawdown_pct: f64,
}

impl DrawdownMonitor {
    pub fn new(initial: f64, limit: f64) -> Self {
        Self { initial_equity: initial, peak_equity: initial, current_equity: initial, max_drawdown_pct: limit }
    }

    pub fn update_equity(&mut self, new_equity: f64) -> DrawdownStatus {
        self.current_equity = new_equity;
        self.peak_equity = self.peak_equity.max(new_equity);

        let dd = match self.peak_equity {
            p if p > 0.0 => ((p - new_equity) / p) * 100.0,
            _ => 0.0,
        };

        match dd {
            v if v >= self.max_drawdown_pct => DrawdownStatus::LimitExceeded { current_dd: v, limit: self.max_drawdown_pct },
            v => DrawdownStatus::Safe { current_dd: v, equity: new_equity },
        }
    }
}

#[derive(Debug, Clone)]
pub enum DrawdownStatus {
    Safe { current_dd: f64, equity: f64 },
    LimitExceeded { current_dd: f64, limit: f64 },
}

pub struct LiquidityMonitor {
    pub max_spread_pct: f64,
    pub min_depth_usd: f64,
}

impl LiquidityMonitor {
    pub fn new(max_spread: f64, depth: f64) -> Self {
        Self { max_spread_pct: max_spread, min_depth_usd: depth }
    }

    pub fn can_execute(&self, bid: f64, ask: f64, target_qty: f64, is_buy: bool) -> bool {
        let mid = (ask + bid) / 2.0;
        let spread = if mid > 0.0 { ((ask - bid) / mid) * 100.0 } else { 100.0 };
        let depth = if is_buy { target_qty * ask } else { target_qty * bid };

        // Match-Guard ile otonom onay
        match (spread, depth) {
            (s, d) if s <= self.max_spread_pct && d >= self.min_depth_usd => true,
            _ => false,
        }
    }
}

// --- 2. SİSTEMSEL GUARDRAILS (MEVCUT WATCHDOG) ---

pub struct PipelineWatchdog;

impl PipelineWatchdog {
    /// §80.1: Kritik anomalileri otonom onarır mı?
    /// match-guard ile karar hiyerarşisi modernize edildi.
    pub fn evaluate_health(open_health_count: u32, last_error_ts: Option<DateTime<Utc>>) -> WatchdogAction {
        let now = Utc::now();
        
        match (open_health_count, last_error_ts) {
            (c, _) if c >= 6 => WatchdogAction::ForceReset, // 30 dk blokaj varsa reset
            (_, Some(ts)) if (now - ts).num_minutes() < 5 => WatchdogAction::CoolingDown,
            _ => WatchdogAction::Healthy,
        }
    }

    /// 80. Kısımdaki takılı kalan pozisyon kontrolü
    pub fn is_position_stuck(opened_at_str: &str, max_hours: u64) -> bool {
        // Fonksiyonel zaman ayrıştırma
        chrono::DateTime::parse_from_rfc3339(opened_at_str)
            .map(|dt| (Utc::now() - dt.with_timezone(&Utc)).num_hours() as u64 >= max_hours)
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WatchdogAction {
    Healthy,
    CoolingDown,
    ForceReset,
}
