// src/robot/security/tracker.rs - Oran Sınırlandırıcı Sayaç Motoru
// Srivastava ATP - Akış Limiti Muhafızı

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// Rate Limit Kuralı
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitRule {
    pub limit_type: String,
    pub max_per_second: u32,
    pub applies_to: String,
}

impl Default for RateLimitRule {
    fn default() -> Self {
        Self {
            limit_type: "trades_per_minute".to_string(),
            max_per_second: 1, 
            applies_to: "all".to_string(),
        }
    }
}

/// Rate Limiter Tracker
#[derive(Debug, Default)]
pub struct RateLimiterTracker {
    counters: HashMap<String, (u32, DateTime<Utc>)>,
}

impl RateLimiterTracker {
    pub fn new() -> Self {
        Self { counters: HashMap::new() }
    }
    
    /// Belirli bir anahtar için işlemin limit sınırları dahilinde olup olmadığını doğrular
    pub fn check_limit(&mut self, key: &str, max_per_second: u32) -> bool {
        let now = Utc::now();
        
        match self.counters.get_mut(key) {
            Some((count, reset_time)) => {
                if now >= *reset_time {
                    *count = 1;
                    // Fail-safe: try_seconds emniyeti ile süre taşma riskleri sönümlendirildi
                    *reset_time = now + chrono::Duration::try_seconds(1).unwrap_or_else(chrono::Duration::zero);
                    true
                } else if *count < max_per_second {
                    *count += 1;
                    true
                } else {
                    false
                }
            }
            None => {
                let reset_time = now + chrono::Duration::try_seconds(1).unwrap_or_else(chrono::Duration::zero);
                self.counters.insert(key.to_string(), (1, reset_time));
                true
            }
        }
    }
}
