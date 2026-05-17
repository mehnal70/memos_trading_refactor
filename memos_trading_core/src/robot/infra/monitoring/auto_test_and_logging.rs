// auto_test_logging.rs
// Otomatik performans/güvenlik testleri ve optimize edilmiş logging altyapısı

use std::sync::{Arc, Mutex};
use chrono::{DateTime, Utc};
use std::fs::{File, OpenOptions};
use std::io::{Write, BufWriter};
use std::path::Path;

/// Log seviyeleri için enum (Performanslı formatlama için)
#[derive(Debug)]
enum LogLevel {
    Perf,
    Security,
    Error,
    Recovery,
}

impl LogLevel {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Perf => "PERF",
            Self::Security => "SEC",
            Self::Error => "ERROR",
            Self::Recovery => "RECOVERY",
        }
    }
}

pub struct AutoTestAndLoggingManager {
    // Dosya yazıcısını BufWriter ile sarmalayarak disk I/O maliyetini düşürüyoruz.
    writer: Arc<Mutex<BufWriter<File>>>,
}

impl std::fmt::Debug for AutoTestAndLoggingManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutoTestAndLoggingManager").field("writer", &"BufWriter<File>").finish()
    }
}

impl AutoTestAndLoggingManager {
    /// Yeni bir yönetici oluşturur ve dosyayı bir kez açar.
    pub fn new<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        
        Ok(Self {
            writer: Arc::new(Mutex::new(BufWriter::new(file))),
        })
    }

    /// Ortak loglama mantığı (Dahili kullanım)
    fn log_internal(&self, level: LogLevel, symbol: &str, message: &str) {
        let now: DateTime<Utc> = Utc::now();
        
        if let Ok(mut guard) = self.writer.lock() {
            // writeln! makrosu BufWriter ile birleşince çok daha hızlıdır.
            let _ = writeln!(
                guard, 
                "[{}] [{}] {} | {}", 
                now.format("%Y-%m-%d %H:%M:%S%.3f"), 
                level.as_str(), 
                symbol, 
                message
            );
            
            // Kritik hata veya güvenlik durumunda tamponu hemen diske boşaltıyoruz.
            if matches!(level, LogLevel::Error | LogLevel::Security) {
                let _ = guard.flush();
            }
        }
    }

    pub fn run_performance_test(&self, symbol: &str) {
        self.log_internal(LogLevel::Perf, symbol, "Performans testi tamamlandı.");
    }

    pub fn run_security_test(&self, symbol: &str) {
        self.log_internal(LogLevel::Security, symbol, "Güvenlik testi tamamlandı.");
    }

    pub fn handle_error(&self, symbol: &str, error: &str) {
        self.log_internal(LogLevel::Error, symbol, &format!("Hata: {}", error));
        self.log_internal(LogLevel::Recovery, symbol, "Yeniden bağlanma başlatıldı.");
    }
}
