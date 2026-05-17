// secure_store.rs
// API anahtarlarını ve hassas verileri bellekte şifreli tutan modül

use aes_gcm::{Aes256Gcm, Key, Nonce, aead::{Aead, KeyInit}};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

// Modern Rust: Harici lazy_static yerine yerleşik OnceLock kullanımı
static MASTER_KEY: OnceLock<Key<Aes256Gcm>> = OnceLock::new();
static STORE: OnceLock<Mutex<HashMap<String, SecretData>>> = OnceLock::new();

struct SecretData {
    nonce: [u8; 12],
    ciphertext: Vec<u8>,
}

/// Ana anahtarı hiyerarşik bir şekilde yükler (Dahili kullanım)
fn get_master_key() -> &'static Key<Aes256Gcm> {
    MASTER_KEY.get_or_init(|| {
        let key_str = std::env::var("SECURE_STORE_MASTER_KEY")
            .expect("KRİTİK HATA: SECURE_STORE_MASTER_KEY env değişkeni eksik!");
        
        // AES-256 için anahtar tam 32 byte olmalıdır
        let mut key_bytes = [0u8; 32];
        let src = key_str.as_bytes();
        let len = src.len().min(32);
        key_bytes[..len].copy_from_slice(&src[..len]);
        
        *Key::<Aes256Gcm>::from_slice(&key_bytes)
    })
}

/// Gizli veri deposuna güvenli erişim sağlayan yardımcı
fn get_store() -> &'static Mutex<HashMap<String, SecretData>> {
    STORE.get_or_init(|| Mutex::new(HashMap::with_capacity(10)))
}

/// Hassas veriyi şifreleyerek belleğe kaydeder
pub fn store_secret(id: &str, secret: &str) {
    let cipher = Aes256Gcm::new(get_master_key());
    let nonce_raw = rand::random::<[u8; 12]>();
    let nonce = Nonce::from_slice(&nonce_raw);

    // Modernize: unwrap yerine güvenli hata yönetimi
    if let Ok(ciphertext) = cipher.encrypt(nonce, secret.as_bytes()) {
        if let Ok(mut map) = get_store().lock() {
            map.insert(id.to_owned(), SecretData {
                nonce: nonce_raw,
                ciphertext,
            });
        }
    }
}

/// Bellekteki şifreli veriyi çözer ve döndürür (O(1) performans)
pub fn get_secret(id: &str) -> Option<String> {
    let map = get_store().lock().ok()?;
    let data = map.get(id)?;
    
    let cipher = Aes256Gcm::new(get_master_key());
    let nonce = Nonce::from_slice(&data.nonce);

    let plaintext = cipher.decrypt(nonce, data.ciphertext.as_slice()).ok()?;
    String::from_utf8(plaintext).ok()
}
