// anomaly_detector.rs - Gerçek zamanlı anomali tespiti ve otomatik bloklama

use crate::robot::infra::compliance::immutable_log::append_immutable_log;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct AnomalyDetector {
    /// Arc<AtomicBool> çok çekirdekli sistemlerde güvenli ve hızlı durum paylaşımı sağlar.
    pub blocked: Arc<AtomicBool>,
}

impl AnomalyDetector {
    #[inline]
    pub fn new() -> Self {
        Self { 
            blocked: Arc::new(AtomicBool::new(false)) 
        }
    }

    /// Olağandışı kayıp/kazanç veya şüpheli davranış tespiti.
    /// Modernizasyon: Formatlama işlemini sadece eşik aşıldığında (lazy) yapıyoruz.
    pub fn check_and_block(&self, event: &str, value: f64, threshold: f64) {
        // Hızlı kontrol: Önce değerin eşiği aşıp aşmadığına bakılır (CPU dostu)
        if value.abs() > threshold {
            // Relaxed yerine SeqCst kullanımı, tüm çekirdeklerin bu bloklamayı anında görmesini sağlar.
            self.blocked.store(true, Ordering::SeqCst);

            // Performans: Mesaj sadece bloklama gerçekleştiğinde oluşturulur (Allocation on demand)
            let msg = format!(
                "Anomali tespit edildi: {} değeri {:.2} (eşik: {:.2})", 
                event, value, threshold
            );

            // Değişmez log kaydı ve terminal çıktısı
            append_immutable_log("ANOMALY_BLOCK", &msg);
            eprintln!("[ANOMALY] {}", msg); // Hata çıktıları için stderr (eprintln) daha uygundur.
        }
    }

    /// Sistem bloklu mu? 
    /// Acquire ordering, store edilen değerin tüm thread'lerde güncelliğini garanti eder.
    #[inline]
    pub fn is_blocked(&self) -> bool {
        self.blocked.load(Ordering::Acquire)
    }

    /// Gerektiğinde blokajı manuel kaldırmak için (Admin action)
    pub fn reset_block(&self) {
        self.blocked.store(false, Ordering::SeqCst);
    }
}