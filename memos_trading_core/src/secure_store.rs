// secure_store.rs
// API anahtarlarını ve hassas verileri bellekte şifreli tutan modül
// Endüstri standardı: AES-GCM ile şifreleme, anahtarlar sadece çalışma anında çözülür

use aes_gcm::{Aes256Gcm, Key, Nonce};
use aes_gcm::aead::{Aead, KeyInit};
use once_cell::sync::Lazy;
use std::sync::Mutex;

static MASTER_KEY: Lazy<Key<Aes256Gcm>> = Lazy::new(|| {
    // Gerçek uygulamada: Anahtar .env, vault veya HSM'den alınmalı
    let key_bytes = std::env::var("SECURE_STORE_MASTER_KEY")
        .expect("SECURE_STORE_MASTER_KEY env değişkeni gerekli!")
        .as_bytes()
        .to_vec();
    Key::<Aes256Gcm>::from_slice(&key_bytes).clone()
});

static STORE: Lazy<Mutex<Vec<(String, Vec<u8>, Vec<u8>)>>> = Lazy::new(|| Mutex::new(Vec::new()));

pub fn store_secret(id: &str, secret: &str) {
    let cipher = Aes256Gcm::new(&*MASTER_KEY);
    let nonce = rand::random::<[u8; 12]>();
    let ciphertext = cipher.encrypt(Nonce::from_slice(&nonce), secret.as_bytes()).unwrap();
    STORE.lock().unwrap().push((id.to_string(), nonce.to_vec(), ciphertext));
}

pub fn get_secret(id: &str) -> Option<String> {
    let cipher = Aes256Gcm::new(&*MASTER_KEY);
    for (sid, nonce, ciphertext) in STORE.lock().unwrap().iter() {
        if sid == id {
            let plaintext = cipher.decrypt(Nonce::from_slice(&nonce), ciphertext.as_slice()).ok()?;
            return Some(String::from_utf8(plaintext).ok()?);
        }
    }
    None
}
