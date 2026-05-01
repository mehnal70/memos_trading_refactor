// secrets.rs - Şifreli API Anahtarı Yönetimi (Gerçek Entegrasyon)
// AES256 ile şifreli dosyadan anahtar okuma
// Türkçe açıklamalar ile

use aes_gcm::{Aes256Gcm, Key, Nonce};
use aes_gcm::aead::{Aead, OsRng, generic_array::GenericArray};
use std::fs;
use std::env;

/// Şifreli anahtar dosyasını çöz ve anahtarları döndür
pub fn load_api_keys_from_encrypted_file(path: &str, passphrase: &str) -> Result<(String, String), String> {
    let data = fs::read(path).map_err(|e| format!("Dosya okunamadı: {e}"))?;
    if data.len() < 12 { return Err("Dosya çok küçük".to_string()); }
    let (nonce_bytes, ciphertext) = data.split_at(12);
    let key = Key::from_slice(&sha2::Sha256::digest(passphrase.as_bytes()));
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher.decrypt(nonce, ciphertext)
        .map_err(|_| "Şifre çözme hatası".to_string())?;
    let s = String::from_utf8(plaintext).map_err(|_| "UTF8 hatası".to_string())?;
    let mut lines = s.lines();
    let api_key = lines.next().unwrap_or("").to_string();
    let api_secret = lines.next().unwrap_or("").to_string();
    Ok((api_key, api_secret))
}

/// Anahtarları şifreli dosyaya kaydet (setup için)
pub fn save_api_keys_encrypted(path: &str, passphrase: &str, api_key: &str, api_secret: &str) -> Result<(), String> {
    let plaintext = format!("{}\n{}", api_key, api_secret);
    let key = Key::from_slice(&sha2::Sha256::digest(passphrase.as_bytes()));
    let cipher = Aes256Gcm::new(key);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher.encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| "Şifreleme hatası".to_string())?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ciphertext);
    fs::write(path, out).map_err(|e| format!("Dosya yazılamadı: {e}"))?;
    Ok(())
}

// Kullanım örneği:
// save_api_keys_encrypted("binance.keys.enc", "parolaniz", "APIKEY", "SECRET").unwrap();
// let (key, secret) = load_api_keys_from_encrypted_file("binance.keys.enc", "parolaniz").unwrap();
