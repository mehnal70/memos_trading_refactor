// robot/file_logger.rs - Dosya tabanlı loglama
use std::fs::{OpenOptions};
use std::io::Write;
use chrono::Utc;
use crate::robot::infra::error::ErrorLogger;

pub struct FileLogger {
    pub path: String,
}

impl FileLogger {
    pub fn new(path: &str) -> Self {
        Self { path: path.to_string() }
    }
    fn write_line(&self, line: &str) {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(file, "{}", line);
        }
    }
}

impl ErrorLogger for FileLogger {
    fn log_error(&self, context: &str, msg: &str) {
        let ts = Utc::now();
        self.write_line(&format!("[{}][ERROR][{}] {}", ts, context, msg));
    }
    fn log_info(&self, context: &str, msg: &str) {
        let ts = Utc::now();
        self.write_line(&format!("[{}][INFO][{}] {}", ts, context, msg));
    }
        // --- 🧬 F1: FILE LOGGER ADLİ ONARIM DELEGESİNİN EKİLMESİ ---
    // Trait kontratındaki eksik halka kalıcı dosya arşivi için de kapatıldı, 'log_repair' vizelendi.
    fn log_repair(&self, step_id: &str, message: &str) {
        let ts = chrono::Utc::now().to_rfc3339();
        let log_entry = format!("[{}][REPAIR][{}] 🔧 {}\n", ts, step_id, message);

        // Adli Güvenlik: logs/app_repair_audit.log dosyasına kilitlenme (lock) maliyeti olmadan append-only yazar
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("logs/app_repair_audit.log") 
        {
            let _ = std::io::Write::write_all(&mut file, log_entry.as_bytes());
        }
    }
}
