// reporting_manager.rs
// Raporlama ve Analiz Modülü
// Otomatik rapor üretimi, performans ve risk raporları, dışa aktarma

use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct Report {
    pub report_id: String,
    pub generated_at: DateTime<Utc>,
    pub report_type: ReportType,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReportType {
    Performance,
    Risk,
    Compliance,
    Custom(String),
}

pub trait ReportingManager {
    fn generate_report(&mut self, report_type: ReportType, content: String) -> Report;
    fn get_report(&self, report_id: &str) -> Option<&Report>;
    fn all_reports(&self) -> &Vec<Report>;
}

pub struct SimpleReportingManager {
    pub reports: Vec<Report>,
}

impl ReportingManager for SimpleReportingManager {
    fn generate_report(&mut self, report_type: ReportType, content: String) -> Report {
        let report = Report {
            report_id: format!("{}-{}", match &report_type {
                ReportType::Custom(s) => s.clone(),
                _ => format!("{:?}", report_type),
            }, Utc::now().timestamp()),
            generated_at: Utc::now(),
            report_type,
            content,
        };
        self.reports.push(report.clone());
        report
    }
    fn get_report(&self, report_id: &str) -> Option<&Report> {
        self.reports.iter().find(|r| r.report_id == report_id)
    }
    fn all_reports(&self) -> &Vec<Report> {
        &self.reports
    }
}
