// rate_limiter.rs - API ve işlem rate limit (gerçek uygulama)
// Her kullanıcı/endpoint için saniyelik/dakikalık limit
// Türkçe açıklamalar ile

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use lazy_static::lazy_static;

lazy_static! {
    static ref RATE_LIMITS: Mutex<HashMap<String, (usize, Instant)>> = Mutex::new(HashMap::new());
}

pub fn check_rate_limit(key: &str, max_per_sec: usize) -> bool {
    let mut map = RATE_LIMITS.lock().unwrap();
    let now = Instant::now();
    let entry = map.entry(key.to_string()).or_insert((0, now));
    if now.duration_since(entry.1) > Duration::from_secs(1) {
        entry.0 = 1;
        entry.1 = now;
        true
    } else {
        if entry.0 < max_per_sec {
            entry.0 += 1;
            true
        } else {
            false
        }
    }
}

// Kullanım örneği:
// if !check_rate_limit("/api/portfolio:user1", 5) { /* 429 Too Many Requests */ }
