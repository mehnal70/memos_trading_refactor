// src/robot/security/mod.rs - Güvenlik Garnizon Kapısı
// Srivastava ATP - Tümden Gelim Tüzüğü (Sıfır Lojik / Sadece Deklarasyon)

pub mod types;   // Rol, Kullanıcı ve AuditEvent veri kontratları
pub mod tracker; // Akış limitleyici (Rate Limiter) motoru
pub mod vault;   // API Anahtar yöneticisi ve maskeleme kalkanı
pub mod manager; // Merkezi güvenlik yöneticisi (SecurityManager)

// Kütüphane genelinde (prelude vb.) kolay erişim için re-export mühürleri
pub use types::{User, UserRole, AuditEvent};
pub use tracker::{RateLimitRule, RateLimiterTracker};
pub use vault::ApiKeyManager;
pub use manager::SecurityManager;
