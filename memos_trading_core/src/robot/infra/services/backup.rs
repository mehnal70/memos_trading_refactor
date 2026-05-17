// backup.rs - Otomatik, şifreli yedekleme ve kurtarma sistemi

use chrono::Utc;
use std::fs;
use std::path::Path;
use aes_gcm::{Aes256Gcm, Key, Nonce, aead::{Aead, AeadCore, KeyInit, OsRng}};
use sha2::{Sha256, Digest};

/// Dosyayı şifreleyerek yedekler.
pub fn backup_file_encrypted(src: &str, backup_dir: &str, passphrase: &str) -> Result<String, String> {
    // 1. Klasör kontrolü
    if !Path::new(backup_dir).exists() {
        fs::create_dir_all(backup_dir).map_err(|e| format!("Dizin oluşturulamadı: {e}"))?;
    }

    // 2. Veri Okuma
    let data = fs::read(src).map_err(|e| format!("Kaynak dosya okunamadı: {e}"))?;

    // 3. Şifreleme Hazırlığı (SHA256 ile anahtar türetme)
    let key_hash = Sha256::digest(passphrase.as_bytes());
    let key = Key::<Aes256Gcm>::from_slice(&key_hash);
    let cipher = Aes256Gcm::new(key);
    
    // GCM için benzersiz numara (nonce)
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

    // 4. Şifreleme
    let ciphertext = cipher.encrypt(&nonce, data.as_slice())
        .map_err(|_| "Şifreleme sırasında hata oluştu".to_string())?;

    // 5. Çıktı Paketleme (Nonce + Ciphertext)
    // Performans: Kapasiteyi önceden belirleyerek re-allocation'ı engelliyoruz.
    let mut final_output = Vec::with_capacity(nonce.len() + ciphertext.len());
    final_output.extend_from_slice(&nonce);
    final_output.extend_from_slice(&ciphertext);

    // 6. Dosyaya Yazma
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let backup_path = format!("{}/backup_{}.enc", backup_dir, timestamp);
    
    fs::write(&backup_path, final_output)
        .map_err(|e| format!("Yedek dosya yazılamadı: {e}"))?;

    Ok(backup_path)
}

/// Şifreli yedeği geri yükler.
pub fn restore_file_encrypted(backup_path: &str, dest: &str, passphrase: &str) -> Result<(), String> {
    let data = fs::read(backup_path).map_err(|e| format!("Yedek dosya okunamadı: {e}"))?;
    
    // GCM Nonce genellikle 12 byte'tır
    if data.len() < 12 { 
        return Err("Geçersiz veya bozuk yedek dosyası (boyut yetersiz)".to_string()); 
    }

    let (nonce_bytes, ciphertext) = data.split_at(12);
    
    // Anahtar türetme
    let key_hash = Sha256::digest(passphrase.as_bytes());
    let key = Key::<Aes256Gcm>::from_slice(&key_hash);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce_bytes);

    // Şifre Çözme
    let plaintext = cipher.decrypt(nonce, ciphertext)
        .map_err(|_| "Şifre çözme hatası! Parola yanlış olabilir veya dosya bozulmuş.".to_string())?;

    // Hedefe Yazma
    fs::write(dest, plaintext).map_err(|e| format!("Geri yükleme başarısız: {e}"))?;

    Ok(())
}
