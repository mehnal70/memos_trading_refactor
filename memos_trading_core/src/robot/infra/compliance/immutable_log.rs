// immutable_log.rs - Değiştirilemez (append-only) log sistemi

use chrono::Utc;
use std::fs::{self, OpenOptions, File};
use std::io::{Write, BufWriter};
use std::sync::{Mutex, OnceLock};

// Modern Rust: lazy_static yerine OnceLock kullanımı (Zero-dependency)
static LOG_FILE: OnceLock<Mutex<Option<BufWriter<File>>>> = OnceLock::new();

/// Global log yöneticisine güvenli erişim sağlayan dahili yardımcı
fn get_logger() -> &'static Mutex<Option<BufWriter<File>>> {
    LOG_FILE.get_or_init(|| Mutex::new(init_logger()))
}

/// Dosyayı bir kez açar ve dizinleri kontrol eder
fn init_logger() -> Option<BufWriter<File>> {
    // logs klasörü yoksa oluştur
    if let Err(e) = fs::create_dir_all("logs") {
        eprintln!("Log dizini oluşturulamadı: {}", e);
        return None;
    }

    OpenOptions::new()
        .create(true)
        .append(true)
        .open("logs/immutable_audit.log")
        .ok()
        .map(BufWriter::new) // Performans için tamponlama (buffering)
}

pub fn append_immutable_log(event_type: &str, message: &str) {
    let ts = Utc::now().to_rfc3339();
    let log_entry = format!("[{}] [{}] {}\n", ts, event_type, message);

    // Adli Güvenlik: Dosya yoksa oluşturulur, varsa sadece sonuna eklenir (Append-only)
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("logs/immutable_audit.log") 
    {
        let _ = file.write_all(log_entry.as_bytes());
    }
}
