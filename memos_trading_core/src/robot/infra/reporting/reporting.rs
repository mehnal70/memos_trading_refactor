// robot/infra/reporting/outputs.rs - Srivastava ATP Çok Kanallı Raporlama Ünitesi
//
// Modernizasyon Notları:
// 1. Fonksiyonel Tablo Oluşturma (Iteratör bazlı)
// 2. TUI Safe Reporting (Side-effect yönetimi)
// 3. Match-Guard ile dinamik dışa aktarım
// 4. Zero-Copy String formatlama

use crate::robot::infra::interfaces::Reporter;
use crate::core::types::{Trade, Signal};
use crate::Result;


// src/robot/infra/reporting.rs - Srivastava ATP Adli Raporlama ve Hata Kayıt Birimi

use std::fs::OpenOptions;
use std::io::Write;
use chrono::Local;

/// Robotun operasyonel hatalarını mühürleyen ana yapı
pub struct ErrorLogger;

impl ErrorLogger {
    /// Kritik bir hatayı hem dosyaya hem de sistem loguna mühürler
    pub fn log_error(context: &str, error: &str) {
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let log_entry = format!("[{}] 🚨 [ERROR] @ {}: {}\n", ts, context, error);

        // 1. Konsola bas (TUI/Headless için)
        eprint!("{}", log_entry);

        // 2. Adli dosyaya mühürle (logs/error_audit.log)
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open("logs/error_audit.log") 
        {
            let _ = file.write_all(log_entry.as_bytes());
        }
    }

    /// Robotun 'Self-Healing' (Onarım) girişimlerini belgeler
    pub fn log_repair(step_id: &str, message: &str) {
        let ts = Local::now().format("%H:%M:%S").to_string();
        println!("[{}] 🔧 [REPAIR] {}: {}", ts, step_id, message);
    }
}


pub struct UniversalReporter;

impl Reporter for UniversalReporter {
    /// İşlemi otonom raporlar. Not: TUI modunda sadece log kaydı tutar.
    fn report_trade(&self, _trade: &Trade) -> Result<()> {
        // §84.1: Side-effect Guard: TUI ekranını bozmamak için direkt çıktı verilmez.
        Ok(())
    }

    fn report_strategy(&self, _name: &str, _signal: &Signal) -> Result<()> {
        Ok(())
    }

    fn export_json(&self, _data: &serde_json::Value) -> Result<()> {
        // İleride dosya sistemine (persistence) mühürlenebilir.
        Ok(())
    }

    fn export_csv(&self, _data: &str) -> Result<()> {
        Ok(())
    }
}

impl UniversalReporter {
    /// §84.2: Performanslı Tablo Oluşturucu
    /// İşlem listesini bellek dostu iteratörler ile formatlar.
    pub fn report_trades_table(&self, trades: &[Trade]) {
        if trades.is_empty() { return; }

        let header = "|   ID   |  Symbol  | Entry  | Exit   | Amount | PnL    | Strategy   |";
        let separator = "-".repeat(header.len());

        println!("{}", header);
        println!("{}", separator);

        // Functional Loop: Veriyi tek geçişte (Single Pass) formatla
        trades.iter().for_each(|t| {
            println!(
                "| {:<6} | {:<8} | {:<6.2} | {:<6.2} | {:<6.2} | {:<6.2} | {:<10} |",
                t.id.map(|i| i.to_string()).unwrap_or_else(|| "-".into()).chars().take(6).collect::<String>(), // ID kısaltma
                t.symbol,
                t.entry_price,
                t.exit_price.unwrap_or(0.0),
                t.amount,
                t.pnl.unwrap_or(0.0),
                t.strategy
            );
        });
        
        println!("{}", separator);
    }
}

