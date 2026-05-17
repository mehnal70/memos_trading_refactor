// sla.rs - SLA ve Uptime Metrik Yönetimi

use chrono::Utc;
use prometheus::{register_gauge, Gauge};
use std::sync::OnceLock;

// Modern Rust: once_cell::sync::Lazy yerine yerleşik OnceLock kullanımı (Zero-dependency)
static START_TIME: OnceLock<i64> = OnceLock::new();
static UPTIME_GAUGE: OnceLock<Gauge> = OnceLock::new();
static SLA_GAUGE: OnceLock<Gauge> = OnceLock::new();

/// SLA sistemini ve metriklerini güvenli bir şekilde başlatır
pub fn init_sla() {
    START_TIME.get_or_init(|| Utc::now().timestamp());
    
    // Metrikleri kaydet (Fail-safe registration)
    // unwrap() yerine hata durumunda loglama yaparak sistemin çökmesini engelliyoruz
    if let Ok(gauge) = register_gauge!("uptime_seconds", "Sistemin toplam çalışma süresi (saniye)") {
        let _ = UPTIME_GAUGE.set(gauge);
    }

    if let Ok(gauge) = register_gauge!("sla_percent", "Son 30 günde SLA yüzdesi") {
        let _ = SLA_GAUGE.set(gauge);
    }
}

/// SLA metriklerini günceller.
/// Performans: i64 matematiksel işlemlerle O(1) hızında çalışır, kilitlenme (lock) içermez.
pub fn update_sla_metrics(downtime_seconds: i64) {
    let now = Utc::now().timestamp();
    
    // Başlangıç zamanını al (Init edilmemişse şimdiyi kullan)
    let start = *START_TIME.get_or_init(|| now);
    
    // 1. Uptime Hesabı
    let uptime = now - start - downtime_seconds;
    if let Some(g) = UPTIME_GAUGE.get() {
        g.set(uptime as f64);
    }

    // 2. SLA Yüzdesi Hesabı (Son 30 gün baz alınır)
    const TOTAL_MONTH_SECONDS: i64 = 30 * 24 * 3600;
    
    // Safe Math: Downtime'ın toplam süreyi aşmadığından emin ol
    let effective_downtime = downtime_seconds.clamp(0, TOTAL_MONTH_SECONDS);
    let sla_percent = 100.0 * (TOTAL_MONTH_SECONDS - effective_downtime) as f64 / TOTAL_MONTH_SECONDS as f64;

    if let Some(g) = SLA_GAUGE.get() {
        g.set(sla_percent);
    }

    // Kritik SLA ihlali durumunda loglama
    if sla_percent < 99.9 {
        eprintln!("[SLA ALERT] Mevcut SLA hedefinin altında: {:.4}%", sla_percent);
    }
}
