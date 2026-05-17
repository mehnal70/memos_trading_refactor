// robot/error.rs - Merkezi Hata Yönetimi ve Otonom Log Kayıt Sistemi

/// §83.2: ErrorLogger - Sistemin tüm bileşenleri için ortak raporlama arayüzü.
/// Send + Sync sayesinde asenkron (tokio) döngülerde güvenle paylaşılabilir.
pub trait ErrorLogger: Send + Sync {
    fn log_error(&self, context: &str, msg: &str);
    fn log_info(&self, context: &str, msg: &str);
    
    // Opsiyonel: Kritik uyarılar için genişletilebilir
    fn log_warn(&self, context: &str, msg: &str) {
        println!("[WARN][{}] {}", context, msg);
    }
}

/// Standart Konsol Çıktısı Sağlayıcısı
pub struct StdoutLogger;

impl ErrorLogger for StdoutLogger {
    fn log_error(&self, context: &str, msg: &str) {
        eprintln!("[ERROR][{}] {}", context, msg);
    }
    fn log_info(&self, context: &str, msg: &str) {
        println!("[INFO][{}] {}", context, msg);
    }
}

/// NOP (No-Operation) Logger - Testler veya sessiz mod için
pub struct NoopLogger;
impl ErrorLogger for NoopLogger {
    fn log_error(&self, _ctx: &str, _m: &str) {}
    fn log_info(&self, _ctx: &str, _m: &str) {}
}
