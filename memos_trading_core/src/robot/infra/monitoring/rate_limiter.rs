// rate_limiter.rs - API ve İşlem Rate Limit Sistemi

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// Modern Rust: lazy_static yerine yerleşik OnceLock kullanımı
static RATE_LIMITS: OnceLock<Mutex<HashMap<String, RateEntry>>> = OnceLock::new();

struct RateEntry {
    count: usize,
    last_reset: Instant,
}

pub struct RateLimiter;

impl RateLimiter {
    /// Global limite güvenli erişim sağlayan dahili yardımcı
    fn get_store() -> &'static Mutex<HashMap<String, RateEntry>> {
        RATE_LIMITS.get_or_init(|| Mutex::new(HashMap::with_capacity(100)))
    }

    /// Rate limit kontrolü yapar.
    /// Performans: Key sadece kayıt bulunamadığında kopyalanır (O(1) optimal durum).
    pub fn check(key: &str, max_per_sec: usize) -> bool {
        let mut map = match Self::get_store().lock() {
            Ok(guard) => guard,
            Err(_) => return false, // Poisoned mutex durumu
        };

        let now = Instant::now();
        
        // HashMap Entry API ile tek geçişte (single-pass) kontrol ve güncelleme
        let entry = map.entry(key.to_owned()).or_insert_with(|| RateEntry {
            count: 0,
            last_reset: now,
        });

        // 1 saniyelik pencere kontrolü
        if now.duration_since(entry.last_reset) >= Duration::from_secs(1) {
            entry.count = 1;
            entry.last_reset = now;
            true
        } else if entry.count < max_per_sec {
            entry.count += 1;
            true
        } else {
            // Limit aşıldı
            false
        }
    }

    /// Belirli bir anahtarın limitini manuel temizlemek için (ör: Admin action)
    pub fn reset(key: &str) {
        if let Ok(mut map) = Self::get_store().lock() {
            map.remove(key);
        }
    }
}
