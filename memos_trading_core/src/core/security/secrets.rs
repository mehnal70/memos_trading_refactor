// secrets.rs - Şifreli API Anahtarı ve Sır Yönetimi

use aes_gcm::{Aes256Gcm, Key, Nonce, aead::{Aead, KeyInit, OsRng}};
use sha2::{Sha256, Digest};
use std::fs;
use std::path::Path;

/// Şifreli dosyadan API anahtarlarını güvenli bir şekilde yükler.
pub fn load_api_keys_from_encrypted_file(path: &str, passphrase: &str) -> Result<(String, String), String> {
    // 1. Dosya Okuma
    let data = fs::read(path).map_err(|e| format!("Anahtar dosyası okunamadı: {e}"))?;
    
    // GCM Nonce 12 byte uzunluğundadır
    if data.len() < 12 { 
        return Err("Geçersiz anahtar dosyası: Veri bütünlüğü bozuk".to_owned()); 
    }

    let (nonce_bytes, ciphertext) = data.split_at(12);

    // 2. Anahtar Türetme (SHA256 hash ile 32-byte anahtar)
    let key_hash = Sha256::digest(passphrase.as_bytes());
    let key = Key::<Aes256Gcm>::from_slice(&key_hash);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce_bytes);

    // 3. Şifre Çözme (Plaintext bellek kopyalaması minimize edildi)
    let plaintext = cipher.decrypt(nonce, ciphertext)
        .map_err(|_| "Şifre çözme hatası: Parola yanlış olabilir".to_owned())?;

    // 4. Veri Ayrıştırma
    let s = String::from_utf8(plaintext).map_err(|_| "Bozuk UTF-8 verisi".to_owned())?;
    let mut lines = s.lines();

    // let Some(...) else yapısı ile unwrap riskini sıfırladık
    let api_key = lines.next().ok_or("API Key bulunamadı")?.to_owned();
    let api_secret = lines.next().ok_or("API Secret bulunamadı")?.to_owned();

    Ok((api_key, api_secret))
}

/// API anahtarlarını AES-256-GCM kullanarak şifreli dosyaya kaydeder.
pub fn save_api_keys_encrypted(path: &str, passphrase: &str, api_key: &str, api_secret: &str) -> Result<(), String> {
    // Klasör kontrolü
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent).ok();
    }

    let plaintext = format!("{}\n{}", api_key, api_secret);

    // Anahtar ve şifreleme hazırlığı
    let key_hash = Sha256::digest(passphrase.as_bytes());
    let key = Key::<Aes256Gcm>::from_slice(&key_hash);
    let cipher = Aes256Gcm::new(key);
    
    // Kriptografik güvenli rastgele sayı üretimi (OsRng)
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

    // Şifreleme
    let ciphertext = cipher.encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| "Şifreleme sırasında kritik hata oluştu".to_owned())?;

    // Çıktı paketleme (Nonce + Ciphertext) - Allocation-optimized
    let mut final_data = Vec::with_capacity(nonce.len() + ciphertext.len());
    final_data.extend_from_slice(&nonce);
    final_data.extend_from_slice(&ciphertext);

    // Dosyaya güvenli yazma
    fs::write(path, final_data).map_err(|e| format!("Dosya yazılamadı: {e}"))?;

    Ok(())
}
