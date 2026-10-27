// src/robot/security/mod.rs - Güvenlik Garnizon Kapısı
// Srivastava ATP - Tümden Gelim Tüzüğü (Sıfır Lojik / Sadece Deklarasyon)

pub mod manager;
pub mod tracker; // Akış limitleyici (Rate Limiter) motoru
pub mod types; // Rol, Kullanıcı ve AuditEvent veri kontratları
pub mod vault; // API Anahtar yöneticisi ve maskeleme kalkanı // Merkezi güvenlik yöneticisi (SecurityManager)

// Kütüphane genelinde (prelude vb.) kolay erişim için re-export mühürleri
pub use manager::SecurityManager;
pub use tracker::{RateLimitRule, RateLimiterTracker};
pub use types::{AuditEvent, User, UserRole};
pub use vault::ApiKeyManager;
