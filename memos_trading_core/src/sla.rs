// sla.rs
// SLA ve uptime metrikleri, otomatik alarm entegrasyonu
// Türkçe açıklamalar ile

use chrono::Utc;
use prometheus_exporter::prometheus::{register_gauge, Gauge};
use std::sync::Mutex;
use once_cell::sync::Lazy;

static START_TIME: Lazy<Mutex<i64>> = Lazy::new(|| Mutex::new(Utc::now().timestamp()));
pub static UPTIME_SECONDS: Lazy<Gauge> = Lazy::new(|| register_gauge!("uptime_seconds", "Sistemin toplam çalışma süresi (saniye)").unwrap());
pub static SLA_PERCENT: Lazy<Gauge> = Lazy::new(|| register_gauge!("sla_percent", "Son 30 günde SLA yüzdesi").unwrap());

pub fn update_sla_metrics(downtime_seconds: i64) {
    let now = Utc::now().timestamp();
    let start = *START_TIME.lock().unwrap();
    let uptime = now - start - downtime_seconds;
    UPTIME_SECONDS.set(uptime as f64);
    let total = 30 * 24 * 3600;
    let sla = 100.0 * (total - downtime_seconds).max(0) as f64 / total as f64;
    SLA_PERCENT.set(sla);
}
