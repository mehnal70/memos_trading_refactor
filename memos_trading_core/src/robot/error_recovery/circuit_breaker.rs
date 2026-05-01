// Circuit Breaker Pattern - Hata Algılama ve Durum Yönetimi
//
// Hata oranı belirli bir eşiği aştığında, sistemi otomatik olarak durdur
// ve belirli bir süre sonra kurtarmaya çalış (halfopen state)

use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Circuit Breaker Durumları
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CircuitBreakerState {
    /// Normal çalışma durumu
    Closed,
    /// Hata oranı yüksek - istekleri reddet
    Open,
    /// Iyileşmeyi test etme durumu
    HalfOpen,
}

/// Circuit Breaker Hatası
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CircuitBreakerError {
    /// Circuit açık durumda (hata oranı yüksek)
    CircuitOpen {
        message: String,
        retry_after_secs: u64,
    },
    /// Threshold aşıldı
    ThresholdExceeded {
        error_rate: f64,
        threshold: f64,
    },
    /// HalfOpen durumunda başarısız deneme
    TestFailed {
        message: String,
    },
}

/// Circuit Breaker Konfigürasyonu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// Hata oranı threshold'u (0.0 - 1.0, örnek: 0.5 = %50)
    pub failure_threshold: f64,
    
    /// Threshold'u kontrol etmek için minimum istekler
    pub failure_count_threshold: u32,
    
    /// Open durumda kalmış olunacak süre (saniye)
    pub timeout_secs: u64,
    
    /// HalfOpen durumunda başarılı olması gereken test sayısı
    pub success_count_to_close: u32,
    
    /// Gözlem penceresi (saniye) - bu sürede ki hataları saymak
    pub sliding_window_secs: u64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 0.5,      // %50 hata oranında aç
            failure_count_threshold: 5,  // En az 5 hata gerekli
            timeout_secs: 60,            // 1 dakika sonra test et
            success_count_to_close: 3,   // 3 başarılı test sonra kapat
            sliding_window_secs: 300,    // 5 dakikalık window
        }
    }
}

/// Circuit Breaker
pub struct CircuitBreaker {
    state: CircuitBreakerState,
    config: CircuitBreakerConfig,
    
    // Hata sayaçları
    total_requests: Arc<AtomicU32>,
    failed_requests: Arc<AtomicU32>,
    
    // Zaman takibi
    last_open_time: Option<DateTime<Utc>>,
    last_state_change: DateTime<Utc>,
    
    // HalfOpen durumunda test sayaçları
    half_open_success_count: u32,
}

impl CircuitBreaker {
    /// Yeni Circuit Breaker oluştur
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: CircuitBreakerState::Closed,
            config,
            total_requests: Arc::new(AtomicU32::new(0)),
            failed_requests: Arc::new(AtomicU32::new(0)),
            last_open_time: None,
            last_state_change: Utc::now(),
            half_open_success_count: 0,
        }
    }

    /// Varsayılan konfigürasyon ile Circuit Breaker
    pub fn default() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }

    /// Mevcut durumu döndür
    pub fn state(&self) -> CircuitBreakerState {
        self.state
    }

    /// Bir işlemin başarılı olduğunu kaydet
    pub fn record_success(&mut self) -> Result<(), CircuitBreakerError> {
        self.total_requests.fetch_add(1, Ordering::SeqCst);

        match self.state {
            CircuitBreakerState::Closed => {
                // Başarılı işlem, sayaçları kontrol et
                let _ = self.update_failure_rate();
                Ok(())
            }
            CircuitBreakerState::Open => {
                // Timeout'u kontrol et
                if let Some(open_time) = self.last_open_time {
                    let elapsed = Utc::now().signed_duration_since(open_time);
                    if elapsed.num_seconds() as u64 >= self.config.timeout_secs {
                        // HalfOpen durumuna geç
                        self.state = CircuitBreakerState::HalfOpen;
                        self.half_open_success_count = 1;
                        self.last_state_change = Utc::now();
                        Ok(())
                    } else {
                        Err(CircuitBreakerError::CircuitOpen {
                            message: "Circuit breaker is open".to_string(),
                            retry_after_secs: self.config.timeout_secs 
                                - elapsed.num_seconds() as u64,
                        })
                    }
                } else {
                    Ok(())
                }
            }
            CircuitBreakerState::HalfOpen => {
                // Test başarılı, sayaç arttır
                self.half_open_success_count += 1;
                
                if self.half_open_success_count >= self.config.success_count_to_close {
                    // Circuit kapat
                    self.state = CircuitBreakerState::Closed;
                    self.total_requests = Arc::new(AtomicU32::new(0));
                    self.failed_requests = Arc::new(AtomicU32::new(0));
                    self.last_state_change = Utc::now();
                }
                Ok(())
            }
        }
    }

    /// Bir işlemin başarısız olduğunu kaydet
    pub fn record_failure(&mut self, _error: &str) -> Result<(), CircuitBreakerError> {
        self.total_requests.fetch_add(1, Ordering::SeqCst);
        self.failed_requests.fetch_add(1, Ordering::SeqCst);

        match self.state {
            CircuitBreakerState::Closed => {
                self.update_failure_rate()?;
                Ok(())
            }
            CircuitBreakerState::Open => {
                // Zaten açık
                Err(CircuitBreakerError::CircuitOpen {
                    message: "Circuit breaker is open".to_string(),
                    retry_after_secs: self.config.timeout_secs,
                })
            }
            CircuitBreakerState::HalfOpen => {
                // Bir test başarısız oldu, tekrar aç
                self.state = CircuitBreakerState::Open;
                self.last_open_time = Some(Utc::now());
                self.last_state_change = Utc::now();
                self.half_open_success_count = 0;
                
                Err(CircuitBreakerError::TestFailed {
                    message: "Test failed in half-open state".to_string(),
                })
            }
        }
    }

    /// Hata oranını güncelle ve durumu ayarla
    fn update_failure_rate(&mut self) -> Result<(), CircuitBreakerError> {
        let total = self.total_requests.load(Ordering::SeqCst);
        let failed = self.failed_requests.load(Ordering::SeqCst);

        if total < self.config.failure_count_threshold {
            return Ok(());
        }

        let failure_rate = failed as f64 / total as f64;

        if failure_rate >= self.config.failure_threshold {
            // Circuit aç
            self.state = CircuitBreakerState::Open;
            self.last_open_time = Some(Utc::now());
            self.last_state_change = Utc::now();

            return Err(CircuitBreakerError::ThresholdExceeded {
                error_rate: failure_rate,
                threshold: self.config.failure_threshold,
            });
        }

        Ok(())
    }

    /// Circuit Breaker istatistiklerini döndür
    pub fn stats(&self) -> (u32, u32, CircuitBreakerState) {
        (
            self.total_requests.load(Ordering::SeqCst),
            self.failed_requests.load(Ordering::SeqCst),
            self.state,
        )
    }

    /// Circuit Breaker'ı sıfırla
    pub fn reset(&mut self) {
        self.state = CircuitBreakerState::Closed;
        self.total_requests = Arc::new(AtomicU32::new(0));
        self.failed_requests = Arc::new(AtomicU32::new(0));
        self.last_open_time = None;
        self.last_state_change = Utc::now();
        self.half_open_success_count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_breaker_closed_state() {
        let breaker = CircuitBreaker::default();
        assert_eq!(breaker.state(), CircuitBreakerState::Closed);
    }

    #[test]
    fn test_circuit_breaker_opens_on_high_failure_rate() {
        let mut breaker = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 0.5,
            failure_count_threshold: 3,
            timeout_secs: 60,
            success_count_to_close: 2,
            sliding_window_secs: 300,
        });

        // 3 başarısız, 1 başarılı = %75 hata oranı
        let _ = breaker.record_failure("test");
        let _ = breaker.record_failure("test");
        let _ = breaker.record_failure("test");
        let _ = breaker.record_success();

        assert_eq!(breaker.state(), CircuitBreakerState::Open);
    }

    #[test]
    fn test_circuit_breaker_rejects_in_open_state() {
        let mut breaker = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 0.5,
            failure_count_threshold: 2,
            timeout_secs: 60,
            success_count_to_close: 2,
            sliding_window_secs: 300,
        });

        // Açmak için
        let _ = breaker.record_failure("test");
        let _ = breaker.record_failure("test");

        // Şimdi kapalı olmalı
        assert_eq!(breaker.state(), CircuitBreakerState::Open);

        // Yeni istekler reddedilmeli
        let result = breaker.record_success();
        assert!(result.is_err());
    }

    #[test]
    fn test_circuit_breaker_half_open_recovery() {
        let config = CircuitBreakerConfig {
            failure_threshold: 0.5,
            failure_count_threshold: 2,
            timeout_secs: 0,  // Hemen timeout
            success_count_to_close: 2,
            sliding_window_secs: 300,
        };
        let mut breaker = CircuitBreaker::new(config);

        // Açmak için
        let _ = breaker.record_failure("test");
        let _ = breaker.record_failure("test");
        assert_eq!(breaker.state(), CircuitBreakerState::Open);

        // Timeout sonra HalfOpen'a geç (timeout=0 olduğundan hemen)
        // Simülasyon: timeout'u başarı kaydederek geç
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _ = breaker.record_success();
        
        // HalfOpen veya Closed olmalı
        assert!(breaker.state() != CircuitBreakerState::Open);
    }

    #[test]
    fn test_circuit_breaker_stats() {
        let mut breaker = CircuitBreaker::default();

        let _ = breaker.record_success();
        let _ = breaker.record_success();
        let _ = breaker.record_failure("test");

        let (total, failed, state) = breaker.stats();
        assert_eq!(total, 3);
        assert_eq!(failed, 1);
        assert_eq!(state, CircuitBreakerState::Closed);
    }

    #[test]
    fn test_circuit_breaker_reset() {
        let mut breaker = CircuitBreaker::default();
        let _ = breaker.record_failure("test");
        let _ = breaker.record_failure("test");

        breaker.reset();
        let (total, failed, state) = breaker.stats();
        assert_eq!(total, 0);
        assert_eq!(failed, 0);
        assert_eq!(state, CircuitBreakerState::Closed);
    }
}
