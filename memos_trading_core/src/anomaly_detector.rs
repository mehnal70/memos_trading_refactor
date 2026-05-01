// anomaly_detector.rs - Gerçek zamanlı anomali tespiti ve otomatik bloklama
// Olağandışı işlem, API abuse, beklenmeyen kayıp/kazanç için alarm ve aksiyon
// Türkçe açıklamalar ile

use chrono::Utc;
use crate::immutable_log::append_immutable_log;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct AnomalyDetector {
    pub blocked: Arc<AtomicBool>,
}

impl AnomalyDetector {
    pub fn new() -> Self {
        Self { blocked: Arc::new(AtomicBool::new(false)) }
    }

    /// Olağandışı kayıp/kazanç, API abuse, şüpheli davranış tespiti
    pub fn check_and_block(&self, event: &str, value: f64, threshold: f64) {
        if value.abs() > threshold {
            self.blocked.store(true, Ordering::SeqCst);
            let msg = format!("Anomali tespit edildi: {} değeri {} (eşik: {})", event, value, threshold);
            append_immutable_log("ANOMALY_BLOCK", &msg);
            println!("[ANOMALY] {}", msg);
            // Burada sistem otomatik olarak işlemleri durdurabilir
        }
    }

    /// Sistem bloklu mu?
    pub fn is_blocked(&self) -> bool {
        self.blocked.load(Ordering::SeqCst)
    }
}

// Kullanım örneği:
// let detector = AnomalyDetector::new();
// detector.check_and_block("PNL", -5000.0, 3000.0);
// if detector.is_blocked() { /* işlemleri durdur */ }
