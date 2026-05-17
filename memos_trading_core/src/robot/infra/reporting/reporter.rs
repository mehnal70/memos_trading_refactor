// src/robot/infra/reporting/reporter.rs

use crate::core::models::MissionControl;
use crate::core::math;
use crate::Result;
use chrono::Utc;
use std::fs::OpenOptions;
use std::io::Write;
use serde_json;

pub struct MissionReporter;

impl MissionReporter {
    /// 1. TUI/Android/Web için Snapshot Yayınlar
    /// Bu fonksiyon main.rs içindeki "veri hazırlama" yükünü bitirir.
    pub fn broadcast_snapshot(&self, snap: &MissionControl) -> Result<()> {
        // İleride burası bir WebSocket veya gRPC kanalına bağlanacak (Android desteği)
        // Şimdilik adli izleme için JSON olarak mühürlenebilir.
        Ok(())
    }

    /// 2. Adli İşlem Raporu (Hassas Matematik Kontrollü)
    /// main.rs içindeki manuel PnL yazdırma lojiklerini buraya alıyoruz.
    pub fn report_trade_settlement(&self, trade: &crate::core::models::ClosedTradeModel) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let status = if trade.pnl >= 0.0 { "PROFIT" } else { "LOSS" };
        
        let report = format!(
            "[{}] [TRADE] {} | {} | PnL: {:.2} ({:.2}%) | Reason: {}",
            now, trade.symbol, status, trade.pnl, trade.pnl_pct, trade.exit_reason
        );

        self.write_to_file("logs/trade_ledger.log", &report)?;
        println!("{}", report); // Headless modda görünürlük sağlar
        Ok(())
    }

    /// 3. Sistem Sağlık Karnesi (Pipeline Sağlığı)
    /// main.rs içindeki anomalileri ve pipeline gecikmelerini denetler.
    pub fn audit_system_health(&self, snap: &MissionControl) -> Result<()> {
        if snap.active_anomalies > 0 {
            let error_msg = format!("⚠ Dikkat: {} aktif anomali tespit edildi!", snap.active_anomalies);
            self.notify_critical_error(&error_msg)?;
        }

        // Geciken pipeline adımlarını raporla
        for step in &snap.pipeline_steps {
            if step.status == "Stale" || step.status == "Failed" {
                let alert = format!("🚨 KRİTİK GECİKME: {} adımı {} saniye saptı!", step.label, step.overdue_secs);
                self.write_to_file("logs/pipeline_alerts.log", &alert)?;
            }
        }
        Ok(())
    }

    /// 4. Adli Özet Dosyalama
    pub fn log_summary_report(&self, summary: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.write_to_file("logs/summary_report.log", &format!("[{}] {}", now, summary))
    }

    /// 5. Kritik Hata Bildirimi (Telegram/Webhook Köprüsü Buraya Gelecek)
    pub fn notify_critical_error(&self, error: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let msg = format!("[{}] CRITICAL: {}", now, error);
        self.write_to_file("logs/critical_errors.log", &msg)?;
        // TODO: Headless modda Telegram botuna 'error' mühürlü mesaj at.
        Ok(())
    }

    // YİNELENEN KOD TEMİZLİĞİ: Yardımcı dosya yazıcı
    fn write_to_file(&self, path: &str, content: &str) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(file, "{}", content)?;
        Ok(())
    }
}
