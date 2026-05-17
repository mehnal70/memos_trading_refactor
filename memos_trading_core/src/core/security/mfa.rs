// mfa.rs - Çok Faktörlü Kimlik Doğrulama (MFA) Modülü

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use chrono::Utc;
use otpauth::TOTP;
use std::str::FromStr;
use rand::{rngs::OsRng, RngCore};
use base32::{Alphabet, encode as base32_encode};
use qrcode::QrCode;
use qrcode::render::unicode;
use serde::{Serialize, Deserialize};

// Modern Rust: lazy_static yerine OnceLock kullanımı
static MFA_STORE: OnceLock<Mutex<HashMap<String, UserMfaSecret>>> = OnceLock::new();

#[derive(Clone, Serialize, Deserialize)]
pub struct UserMfaSecret {
    pub username: String,
    pub secret: String, 
    pub failed_attempts: u32,
    pub last_failed: Option<i64>,
}

pub struct MfaManager;

impl MfaManager {
    /// Global mağazaya güvenli erişim sağlayan yardımcı
    fn store() -> &'static Mutex<HashMap<String, UserMfaSecret>> {
        MFA_STORE.get_or_init(|| Mutex::new(HashMap::with_capacity(100)))
    }

    /// Kullanıcıya yeni secret üretir ve kurulum için QR kodunu döndürür
    pub fn enroll_user(username: &str, issuer: &str) -> (String, String) {
        // Güvenlik: Kriptografik rastgele sayı üretimi için OsRng kullanımı
        let mut secret_bytes = [0u8; 20]; // TOTP için 160-bit (20 byte) yeterli ve standarttır
        OsRng.fill_bytes(&mut secret_bytes);
        
        let secret = base32_encode(Alphabet::RFC4648 { padding: false }, &secret_bytes);
        
        // OTP URL formatlama (Allocation-optimized)
        let url = format!(
            "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits=6&period=30",
            issuer, username, secret, issuer
        );

        // QR kod üretimi (Hata yönetimi ile)
        let qr = QrCode::new(url.as_bytes())
            .map(|code| code.render::<unicode::Dense1x2>().build())
            .unwrap_or_else(|_| "QR_ERROR".to_owned());

        let user_secret = UserMfaSecret {
            username: username.to_owned(),
            secret: secret.clone(),
            failed_attempts: 0,
            last_failed: None,
        };

        if let Ok(mut guard) = Self::store().lock() {
            guard.insert(username.to_owned(), user_secret);
        }

        (secret, qr)
    }

    /// Kullanıcıdan gelen TOTP kodunu doğrular (Brute-force korumalı)
    pub fn verify(username: &str, code: &str) -> bool {
        let mut guard = match Self::store().lock() {
            Ok(g) => g,
            Err(_) => return false,
        };

        let Some(user) = guard.get_mut(username) else { return false; };

        // 1. Brute-force koruması: 5 hatalı denemede 5 dakika kilit
        let now = Utc::now().timestamp();
        if user.failed_attempts >= 5 {
            if let Some(last) = user.last_failed {
                if now - last < 300 {
                    return false;
                } else {
                    // Süre dolmuşsa sayacı sıfırla
                    user.failed_attempts = 0;
                }
            }
        }

        // 2. Kod doğrulama
        let totp = TOTP::new(&user.secret);
        let current_ts = now as u64;

        if let Ok(code_u32) = u32::from_str(code) {
            if totp.verify(code_u32, 0, current_ts) {
                user.failed_attempts = 0;
                user.last_failed = None;
                true
            } else {
                user.failed_attempts += 1;
                user.last_failed = Some(now);
                false
            }
        } else {
            false
        }
    }
}

/// JWT entegrasyonu için Claims yapısı
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub mfa_verified: bool,
}
