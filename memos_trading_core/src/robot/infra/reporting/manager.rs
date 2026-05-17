// robot/infra/reporting/manager.rs - Srivastava ATP Raporlama ve Arşiv Merkezi
//
// Modernizasyon Notları:
// 1. Match-Guard ile dinamik ID üretimi
// 2. Sahiplik (Ownership) optimizasyonu: Insert sonrası referans dönüşü
// 3. Fonksiyonel Iteratörler ile toplu rapor listeleme
// 4. Panic-free HashMap yönetimi

use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Report {
    pub report_id: String,
    pub generated_at: DateTime<Utc>,
    pub report_type: ReportType,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportType {
    Performance,
    Risk,
    Compliance,
    Custom(String),
}

pub trait ReportingManager: Send + Sync {
    fn generate_report(&mut self, report_type: ReportType, content: String) -> Report;
    fn get_report(&self, report_id: &str) -> Option<&Report>;
    fn all_reports(&self) -> Vec<&Report>;
}

pub struct SimpleReportingManager {
    /// §84.3: Hafıza Otonomisi - ID bazlı hızlı erişim deposu
    pub reports: HashMap<String, Report>,
}

impl SimpleReportingManager {
    pub fn new() -> Self {
        Self { reports: HashMap::with_capacity(64) }
    }

    /// Pattern Matching ile standartlaştırılmış ID üretimi
    fn generate_id(&self, report_type: &ReportType) -> String {
        let prefix = match report_type {
            ReportType::Performance => "PERF",
            ReportType::Risk        => "RISK",
            ReportType::Compliance  => "COMP",
            ReportType::Custom(s)   => s.as_str(),
        };
        format!("{}-{}", prefix, Utc::now().timestamp_nanos_opt().unwrap_or(0))
    }
}

impl ReportingManager for SimpleReportingManager {
    /// Yeni bir rapor üretir ve otonom arşive mühürler.
    fn generate_report(&mut self, report_type: ReportType, content: String) -> Report {
        let report_id = self.generate_id(&report_type);
        
        let report = Report {
            report_id: report_id.clone(),
            generated_at: Utc::now(),
            report_type,
            content,
        };

        // Verimlilik: Raporu arşive eklerken kopyasını otonom oluşturur.
        self.reports.insert(report_id, report.clone());
        report
    }

    #[inline]
    fn get_report(&self, report_id: &str) -> Option<&Report> {
        self.reports.get(report_id)
    }

    /// Tüm raporları referans bazlı döner (Zero-allocation listeleme).
    fn all_reports(&self) -> Vec<&Report> {
        self.reports.values().collect()
    }
}
