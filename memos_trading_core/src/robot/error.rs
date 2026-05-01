// robot/error.rs - Merkezi hata yönetimi ve logging trait'i



pub trait ErrorLogger: Send + Sync {
    fn log_error(&self, context: &str, msg: &str);
    fn log_info(&self, context: &str, msg: &str);
}

pub struct StdoutLogger;

impl ErrorLogger for StdoutLogger {
    fn log_error(&self, context: &str, msg: &str) {
        eprintln!("[ERROR][{}] {}", context, msg);
    }
    fn log_info(&self, context: &str, msg: &str) {
        println!("[INFO][{}] {}", context, msg);
    }
}
