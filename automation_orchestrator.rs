// Otomasyon Test Orchestrator (Rust)
// Bu script, ana uygulamanın otomasyon zincirini dışarıdan test eder ve raporlar.
// Testler: Sinyal üretimi, trade, risk yönetimi, log kontrolü, mod geçişi, hata tespiti


use std::thread::sleep;
use std::time::Duration;
use reqwest::blocking::Client;
use serde_json::Value;

fn send_alert(subject: &str, message: &str) {
    // Burada e-posta, Slack veya başka bir entegrasyon eklenebilir
    println!("[ALERT] {}: {}", subject, message);
}

fn main() {
    println!("[Orchestrator] Otomasyon zinciri sürekli test döngüsü başlatıldı...");
    let client = Client::new();
    let base_url = "http://localhost:5173/api"; // Tauri backend API endpoint (örnek)
    let mut consecutive_errors = 0;
    let max_errors = 3;

    loop {
        let mut critical_error = false;
        println!("\n[Orchestrator] Yeni test döngüsü başlıyor...");

        // 1. Sinyal üretimi testi
        let ma_signal = client.post(&format!("{}/ma_signal", base_url))
            .json(&serde_json::json!({"closes": [10.0, 10.0, 10.0, 10.0, 100.0], "fast": 2, "slow": 4}))
            .send().and_then(|r| r.json::<Value>());
        match ma_signal {
            Ok(val) => println!("[OK] MA sinyal testi: {:?}", val),
            Err(e) => {
                println!("[ERR] MA sinyal testi başarısız: {}", e);
                send_alert("MA Sinyal Hatası", &e.to_string());
                critical_error = true;
            }
        }

        // 2. Trade zinciri testi
        let trades = client.get(&format!("{}/get_auto_trades", base_url))
            .send().and_then(|r| r.json::<Value>());
        match trades {
            Ok(val) => println!("[OK] Trade zinciri testi: Son işlemler: {:?}", val),
            Err(e) => {
                println!("[ERR] Trade zinciri testi başarısız: {}", e);
                send_alert("Trade Zinciri Hatası", &e.to_string());
                critical_error = true;
            }
        }

        // 3. Risk yönetimi testi
        let risk = client.get(&format!("{}/get_risk_status", base_url))
            .send().and_then(|r| r.json::<Value>());
        match risk {
            Ok(val) => println!("[OK] Risk yönetimi testi: {:?}", val),
            Err(e) => {
                println!("[ERR] Risk yönetimi testi başarısız: {}", e);
                send_alert("Risk Yönetimi Hatası", &e.to_string());
                critical_error = true;
            }
        }

        // 4. Log ve hata kontrolü
        let logs = client.get(&format!("{}/get_trading_logs", base_url))
            .send().and_then(|r| r.json::<Value>());
        match logs {
            Ok(val) => println!("[OK] Log kontrolü: Son loglar: {:?}", val),
            Err(e) => {
                println!("[ERR] Log kontrolü başarısız: {}", e);
                send_alert("Log Hatası", &e.to_string());
                critical_error = true;
            }
        }

        // 5. Manuel/otomatik mod geçişi testi (örnek endpoint)
        let mode = client.post(&format!("{}/set_mode", base_url))
            .json(&serde_json::json!({"mode": "auto"}))
            .send().and_then(|r| r.json::<Value>());
        match mode {
            Ok(val) => println!("[OK] Mod geçiş testi: {:?}", val),
            Err(e) => {
                println!("[ERR] Mod geçiş testi başarısız: {}", e);
                send_alert("Mod Geçiş Hatası", &e.to_string());
                critical_error = true;
            }
        }

        // 6. Healthcheck (örnek endpoint)
        let health = client.get(&format!("{}/system_readiness_check", base_url))
            .send().and_then(|r| r.json::<Value>());
        match health {
            Ok(val) => println!("[OK] Healthcheck: {:?}", val),
            Err(e) => {
                println!("[ERR] Healthcheck başarısız: {}", e);
                send_alert("Healthcheck Hatası", &e.to_string());
                critical_error = true;
            }
        }

        if critical_error {
            consecutive_errors += 1;
            println!("[Orchestrator] Kritik hata! ({} ardışık hata)", consecutive_errors);
            if consecutive_errors >= max_errors {
                println!("[Orchestrator] Maksimum hata sayısına ulaşıldı, otomasyon zinciri durduruluyor!");
                send_alert("Otomasyon Durduruldu", "Ardışık kritik hatalar nedeniyle zincir otomatik olarak durduruldu.");
                break;
            }
        } else {
            consecutive_errors = 0;
        }

        println!("[Orchestrator] Döngü tamamlandı. 30 saniye sonra tekrar denenecek...");
        sleep(Duration::from_secs(30));
    }
    println!("[Orchestrator] İzleme ve test servisi sonlandı.");
}
