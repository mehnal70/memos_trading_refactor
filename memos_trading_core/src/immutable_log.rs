// immutable_log.rs - Değiştirilemez (append-only) log sistemi
// Kritik işlemler ve olaylar için güvenli, silinemez loglama
// Türkçe açıklamalar ile

use chrono::Utc;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;

lazy_static::lazy_static! {
    static ref LOG_LOCK: Mutex<()> = Mutex::new(());
}

pub fn append_immutable_log(event: &str, details: &str) {
    let _guard = LOG_LOCK.lock().unwrap();
    let now = Utc::now().to_rfc3339();
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("logs/immutable_audit.log")
        .unwrap();
    writeln!(file, "[{}] {} | {}", now, event, details).ok();
    // Dosya asla truncate edilmez, sadece ekleme yapılır
}

// Kullanım örneği:
// append_immutable_log("ORDER_PLACED", "BTCUSDT 1.0 @ 50000");
