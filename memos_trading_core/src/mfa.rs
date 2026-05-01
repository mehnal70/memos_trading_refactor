// mfa.rs
// Gerçek üretim seviyesinde, JWT ile entegre TOTP tabanlı MFA modülü
// Türkçe açıklamalar ile, brute-force korumalı, QR kod üretimli

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use chrono::Utc;
use otpauth::TOTP;
use std::str::FromStr;
use rand::RngCore;
use base32::{Alphabet, encode as base32_encode};
use qrcode::QrCode;
use qrcode::render::unicode;
use serde::{Serialize, Deserialize};

// Kullanıcı MFA secret'ları ve brute-force koruma için bellek içi store (örnek: prod'da DB)
lazy_static::lazy_static! {
    static ref MFA_STORE: Arc<Mutex<HashMap<String, UserMfaSecret>>> = Arc::new(Mutex::new(HashMap::new()));
}

#[derive(Clone, Serialize, Deserialize)]
pub struct UserMfaSecret {
    pub username: String,
    pub secret: String, // base32 encoded
    pub failed_attempts: u32,
    pub last_failed: Option<i64>, // timestamp
}

pub struct MfaManager;

impl MfaManager {
    // Kullanıcıya yeni secret üret ve QR kodunu döndür
    pub fn enroll_user(username: &str, issuer: &str) -> (String, String) {
        let mut rng = rand::thread_rng();
        let mut secret_bytes = [0u8; 32];
        rng.fill_bytes(&mut secret_bytes);
        let secret = base32_encode(Alphabet::RFC4648 { padding: false }, &secret_bytes);
        let _totp = TOTP::new(&secret);
        // otpauth URL elle oluşturuluyor
        let url = format!(
            "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits=6&period=30",
            issuer, username, secret, issuer
        );
        let code = QrCode::new(url.as_bytes()).unwrap();
        let qr = code.render::<unicode::Dense1x2>().build();
        let user_secret = UserMfaSecret {
            username: username.to_string(),
            secret: secret.clone(),
            failed_attempts: 0,
            last_failed: None,
        };
        MFA_STORE.lock().unwrap().insert(username.to_string(), user_secret);
        (secret, qr)
    }

    // Kullanıcıdan gelen kodu doğrula, brute-force korumalı
    pub fn verify(username: &str, code: &str) -> bool {
        let mut store = MFA_STORE.lock().unwrap();
        if let Some(user) = store.get_mut(username) {
            // Brute-force koruması: 5 başarısız denemede 5dk kilit
            if user.failed_attempts >= 5 {
                if let Some(last) = user.last_failed {
                    if Utc::now().timestamp() - last < 300 {
                        return false;
                    } else {
                        user.failed_attempts = 0;
                    }
                }
            }
            let totp = TOTP::new(&user.secret);
            // Kod string'den u32'ye çevrilmeli
            if let Ok(code_u32) = u32::from_str(code) {
                if totp.verify(code_u32, 0, Utc::now().timestamp() as u64) {
                    user.failed_attempts = 0;
                    return true;
                } else {
                    user.failed_attempts += 1;
                    user.last_failed = Some(Utc::now().timestamp());
                    return false;
                }
            } else {
                return false;
            }
        } else {
            false
        }
    }
}

// JWT ile entegrasyon için: MFA doğrulandı flag'i ekle
#[derive(Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub mfa_verified: bool,
}

// API endpoint örnekleri (axum ile):
// POST /api/mfa/enroll {username} => dönen: {secret, qr}
// POST /api/mfa/verify {username, code} => dönen: {success, mfa_verified}
// JWT token'a mfa_verified=true eklenir

// Not: Gerçek ortamda secret'lar DB'de şifreli saklanmalı, brute-force ve rate limit zorunlu.
