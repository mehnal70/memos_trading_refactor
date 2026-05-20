// Healthcheck invariant fonksiyonları.
//
// Her fonksiyon bir invariant kontrol eder; pass/fail bilgisi konsola yazılır,
// bool döner. Failing invariant exit code'a yansır.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;

use memos_trading_core::robot::robotic_loop::AppState;

/// Heartbeat JSONL dosyası mevcut + son yazım `max_age_secs` saniye içinde.
pub fn check_heartbeat_fresh(path: &Path, max_age_secs: u64) -> bool {
    let label = format!("heartbeat ({})", path.display());
    if !path.exists() {
        println!("  ✗ {} — dosya yok", label);
        return false;
    }
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            println!("  ✗ {} — stat hatası: {}", label, e);
            return false;
        }
    };
    let age = metadata.modified().ok()
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs())
        .unwrap_or(u64::MAX);
    if age > max_age_secs {
        println!("  ✗ {} — son yazım {}s önce (eşik {}s)", label, age, max_age_secs);
        return false;
    }
    // Son satırı parse et — JSONL bütünlüğü
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let last_line = content.lines().last().unwrap_or("");
    if last_line.trim().is_empty() {
        println!("  ✗ {} — boş", label);
        return false;
    }
    if serde_json::from_str::<serde_json::Value>(last_line).is_err() {
        println!("  ✗ {} — son satır JSON parse edilemedi", label);
        return false;
    }
    println!("  ✓ {} — taze ({}s önce, son satır parse OK)", label, age);
    true
}

/// MissionControl JSON snapshot dosyası mevcut + mtime taze.
pub fn check_snapshot_fresh(path: &Path, max_age_secs: u64) -> bool {
    let label = format!("snapshot ({})", path.display());
    if !path.exists() {
        println!("  ✗ {} — dosya yok", label);
        return false;
    }
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            println!("  ✗ {} — stat hatası: {}", label, e);
            return false;
        }
    };
    let age = metadata.modified().ok()
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs())
        .unwrap_or(u64::MAX);
    if age > max_age_secs {
        println!("  ✗ {} — son yazım {}s önce (eşik {}s)", label, age, max_age_secs);
        return false;
    }
    // İçerik geçerli JSON mu?
    let content = std::fs::read_to_string(path).unwrap_or_default();
    if serde_json::from_str::<serde_json::Value>(&content).is_err() {
        println!("  ✗ {} — JSON parse edilemedi", label);
        return false;
    }
    println!("  ✓ {} — taze ({}s önce, JSON OK)", label, age);
    true
}

/// `fleet.phase` "Idle" / "Booting"den ileri geçmiş olmalı.
pub fn check_phase_advanced(state: &Arc<Mutex<AppState>>) -> bool {
    let phase = match state.lock() {
        Ok(s) => s.fleet.phase.clone(),
        Err(_) => {
            println!("  ✗ phase — state lock alınamadı");
            return false;
        }
    };
    let stuck = phase == "Idle" || phase == "Booting" || phase.is_empty();
    if stuck {
        println!("  ✗ phase — boot fazından çıkmadı (current=\"{}\")", phase);
        return false;
    }
    println!("  ✓ phase — boot fazından çıktı (current=\"{}\")", phase);
    true
}

/// Otonom loop nabzı ilerliyor. `last_loop_tick` smoke süresince artmış olmalı.
pub fn check_loop_tick_advanced(state: &Arc<Mutex<AppState>>, start_tick: u64) -> bool {
    let end_tick = state.lock().ok()
        .map(|s| s.fleet.last_loop_tick.load(Ordering::Relaxed))
        .unwrap_or(0);
    if end_tick <= start_tick {
        println!("  ✗ loop_tick — ilerlemedi (start={}, end={})", start_tick, end_tick);
        return false;
    }
    println!("  ✓ loop_tick — ilerledi (start={}, end={}, Δ={})",
        start_tick, end_tick, end_tick - start_tick);
    true
}

/// Equity sağlık aralığında: >0 ve drawdown limit altı (default %50).
pub fn check_equity_sane(state: &Arc<Mutex<AppState>>, max_drawdown_pct: f64) -> bool {
    let (equity, peak) = match state.lock() {
        Ok(s) => (s.finance.equity, s.finance.peak_equity),
        Err(_) => {
            println!("  ✗ equity — state lock alınamadı");
            return false;
        }
    };
    if equity <= 0.0 {
        println!("  ✗ equity — sıfır veya negatif ({:.2})", equity);
        return false;
    }
    let dd_pct = if peak > 0.0 { (1.0 - equity / peak) * 100.0 } else { 0.0 };
    if dd_pct > max_drawdown_pct {
        println!("  ✗ equity — drawdown %{:.2} > eşik %{:.2} (equity={:.2}, peak={:.2})",
            dd_pct, max_drawdown_pct, equity, peak);
        return false;
    }
    println!("  ✓ equity — sağlıklı (equity={:.2}, peak={:.2}, dd=%{:.2})",
        equity, peak, dd_pct);
    true
}

/// guardian.log içinde kritik anomali (🚨 prefix veya OVERFILL) yok.
pub fn check_no_critical_alerts(state: &Arc<Mutex<AppState>>) -> bool {
    let critical: Vec<String> = match state.lock() {
        Ok(s) => s.guardian.log.iter()
            .filter(|l| l.contains("🚨") || l.contains("PARTIAL-ANOMALY-OVERFILL"))
            .cloned()
            .collect(),
        Err(_) => {
            println!("  ✗ critical_alerts — state lock alınamadı");
            return false;
        }
    };
    if !critical.is_empty() {
        println!("  ✗ critical_alerts — {} kritik uyarı tespit edildi:", critical.len());
        for line in critical.iter().take(3) {
            println!("      · {}", line);
        }
        return false;
    }
    println!("  ✓ critical_alerts — temiz");
    true
}

/// guardian.log içinde belirli bir substring'i içeren en az 1 satır olmalı.
pub fn check_log_contains(state: &Arc<Mutex<AppState>>, needle: &str, label: &str) -> bool {
    let saw = match state.lock() {
        Ok(s) => s.guardian.log.iter().any(|l| l.contains(needle)),
        Err(_) => {
            println!("  ✗ {} — state lock alınamadı", label);
            return false;
        }
    };
    if !saw {
        println!("  ✗ {} — log'da \"{}\" bulunamadı", label, needle);
        return false;
    }
    println!("  ✓ {} — log'da \"{}\" var", label, needle);
    true
}
