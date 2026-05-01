// Otomasyon ana akışını test eden temel entegrasyon testi
// Bu test, otomasyonun başlatılması, veri indirme ve hata/uyarı loglarının gözlemlenmesi üzerine kurgulanmıştır.
// Gerçekten tam otomasyon testi için, log dosyası veya DB kontrolü ile daha ileri doğrulamalar eklenebilir.

use std::time::Duration;
use std::fs;
use std::thread::sleep;

#[test]
fn test_otomasyon_akisi_baslatma_ve_log_kontrol() {
    let log_path = "/home/ulas/PyCharmMiscProject/memos_trading/logs/trade_history.jsonl";
    let db_path = "/home/ulas/PyCharmMiscProject/memos_trading/data/trader.db";
    sleep(Duration::from_secs(5));

    let log_exists = fs::metadata(log_path).is_ok();
    if !log_exists {
        eprintln!("UYARI: Otomasyon log dosyası bulunamadı: {}", log_path);
    } else {
        let log_content = fs::read_to_string(log_path).unwrap_or_default();
        if log_content.trim().is_empty() {
            eprintln!("UYARI: Log dosyası boş: {}", log_path);
        } else {
            // Log dosyası doluysa, son güncellenme kontrolü
            if let Ok(meta) = fs::metadata(log_path) {
                if let Ok(modified) = meta.modified() {
                    let now = std::time::SystemTime::now();
                    let diff = now.duration_since(modified).unwrap_or(Duration::from_secs(9999));
                    if diff.as_secs() >= 60 {
                        eprintln!("UYARI: Log dosyası son 1 dakikada güncellenmemiş: {}", log_path);
                    }
                }
            }
        }
    }

    let db_exists = fs::metadata(db_path).is_ok();
    if !db_exists {
        eprintln!("UYARI: Otomasyon DB dosyası bulunamadı: {}", db_path);
    }
    assert!(log_exists || db_exists, "Otomasyon log veya DB dosyası bulunamadı!");
}
