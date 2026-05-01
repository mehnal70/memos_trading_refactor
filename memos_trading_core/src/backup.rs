// backup.rs - Otomatik, şifreli yedekleme ve kurtarma sistemi
// Portföy, işlem geçmişi ve konfigürasyonun düzenli yedeği
// Türkçe açıklamalar ile

use chrono::Utc;
use std::fs;
use aes_gcm::{Aes256Gcm, Key, Nonce};
use aes_gcm::aead::{Aead, OsRng, generic_array::GenericArray};
use sha2::Sha256;

pub fn backup_file_encrypted(src: &str, backup_dir: &str, passphrase: &str) -> Result<(), String> {
    let data = fs::read(src).map_err(|e| format!("Kaynak dosya okunamadı: {e}"))?;
    let key = Key::from_slice(&Sha256::digest(passphrase.as_bytes()));
    let cipher = Aes256Gcm::new(key);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher.encrypt(&nonce, &data).map_err(|_| "Şifreleme hatası".to_string())?;
    let now = Utc::now().format("%Y%m%d_%H%M%S");
    let backup_path = format!("{}/backup_{}.enc", backup_dir, now);
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ciphertext);
    fs::write(&backup_path, out).map_err(|e| format!("Yedek dosya yazılamadı: {e}"))?;
    Ok(())
}

pub fn restore_file_encrypted(backup_path: &str, dest: &str, passphrase: &str) -> Result<(), String> {
    let data = fs::read(backup_path).map_err(|e| format!("Yedek dosya okunamadı: {e}"))?;
    if data.len() < 12 { return Err("Yedek dosya çok küçük".to_string()); }
    let (nonce_bytes, ciphertext) = data.split_at(12);
    let key = Key::from_slice(&Sha256::digest(passphrase.as_bytes()));
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher.decrypt(nonce, ciphertext).map_err(|_| "Şifre çözme hatası".to_string())?;
    fs::write(dest, plaintext).map_err(|e| format!("Dosya geri yüklenemedi: {e}"))?;
    Ok(())
}

// Kullanım örneği:
// backup_file_encrypted("data/trader.db", "backups", "parolaniz").unwrap();
// restore_file_encrypted("backups/backup_20260207_120000.enc", "data/trader.db", "parolaniz").unwrap();
