// robot/autonomous_audit.rs - Otonom Trader Audit & Tracking Sistemi

use serde::{Serialize, Deserialize};
use std::fs;
use std::path::Path;
use chrono::Utc;

/// Aşama durumu
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AuditStageStatus {
    #[serde(rename = "PENDING")]
    Pending,
    #[serde(rename = "IN_PROGRESS")]
    InProgress,
    #[serde(rename = "COMPLETED")]
    Completed,
    #[serde(rename = "ERROR")]
    Error,
    #[serde(rename = "READY_FOR_APPROVAL")]
    ReadyForApproval,
}

/// Bir aşamanın detaylı kaydı
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageRecord {
    pub stage: usize,
    pub name: String,
    pub status: AuditStageStatus,
    pub duration_ms: u128,
    pub input: serde_json::Value,
    pub output: serde_json::Value,
    pub error: Option<String>,
    pub validation: Option<ValidationResult>,
}

/// Doğrulama sonucu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub checks: Vec<ValidationCheck>,
    pub error_details: Option<String>,
}

/// Doğrulama kontrolü
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationCheck {
    pub check_name: String,
    pub passed: bool,
    pub expected: String,
    pub actual: String,
}

/// Bir cycle'ın tam kaydı
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleRecord {
    pub cycle_id: u64,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub total_duration_ms: u128,
    pub status: String,  // RUNNING, COMPLETED, ERROR
    pub stages: Vec<StageRecord>,
    pub summary: CycleSummary,
    pub error_message: Option<String>,
}

/// Cycle özeti
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleSummary {
    pub symbols_scanned: usize,
    pub symbols_selected: usize,
    pub symbols_processed: usize,
    pub graduations_pending: usize,
    pub graduations_approved: usize,
    pub errors_count: usize,
    pub average_stage_duration_ms: u128,
}

impl CycleRecord {
    pub fn new(cycle_id: u64) -> Self {
        Self {
            cycle_id,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            total_duration_ms: 0,
            status: "RUNNING".to_string(),
            stages: Vec::new(),
            summary: CycleSummary {
                symbols_scanned: 0,
                symbols_selected: 0,
                symbols_processed: 0,
                graduations_pending: 0,
                graduations_approved: 0,
                errors_count: 0,
                average_stage_duration_ms: 0,
            },
            error_message: None,
        }
    }

    pub fn add_stage(&mut self, stage: StageRecord) {
        self.stages.push(stage);
    }

    pub fn complete(&mut self, started_at: chrono::DateTime<Utc>) {
        self.completed_at = Some(Utc::now().to_rfc3339());
        self.total_duration_ms = Utc::now()
            .signed_duration_since(started_at)
            .num_milliseconds() as u128;
        self.status = "COMPLETED".to_string();

        // Özet hesapla
        self.summary.errors_count = self.stages
            .iter()
            .filter(|s| s.status == AuditStageStatus::Error)
            .count();

        if !self.stages.is_empty() {
            self.summary.average_stage_duration_ms = 
                self.stages.iter().map(|s| s.duration_ms).sum::<u128>() / self.stages.len() as u128;
        }
    }

    pub fn set_error(&mut self, error: String) {
        self.status = "ERROR".to_string();
        self.error_message = Some(error);
    }

    pub fn to_json_string(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

/// JSON Logger - Her cycle'ı dosyaya yaz
pub struct AutonomousAuditLogger {
    pub logs_dir: String,
    pub max_files: usize,
}

impl AutonomousAuditLogger {
    pub fn new(logs_dir: &str) -> Self {
        // Klasörü oluştur
        if !Path::new(logs_dir).exists() {
            let _ = fs::create_dir_all(logs_dir);
        }
        
        Self {
            logs_dir: logs_dir.to_string(),
            max_files: 100,  // Son 100 cycle logunu tut
        }
    }

    /// Cycle kaydını JSON dosyasına yaz
    pub fn save_cycle(&self, cycle: &CycleRecord) -> Result<String, String> {
        let filename = format!(
            "{}/autonomous_cycle_{:06}_{}.json",
            self.logs_dir,
            cycle.cycle_id,
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        );

        let json = cycle.to_json_string();
        fs::write(&filename, json)
            .map_err(|e| format!("JSON yazılamadı: {}", e))?;

        println!("📝 Cycle {} kaydı: {}", cycle.cycle_id, filename);
        self.cleanup_old_files()?;

        Ok(filename)
    }

    /// Eski dosyaları temizle
    fn cleanup_old_files(&self) -> Result<(), String> {
        let entries = fs::read_dir(&self.logs_dir)
            .map_err(|e| format!("Klasör okunamadı: {}", e))?;

        let mut files: Vec<_> = entries
            .filter_map(|e| {
                e.ok().and_then(|entry| {
                    let path = entry.path();
                    if path.extension().map_or(false, |ext| ext == "json") {
                        let modified = entry.metadata()
                            .ok()?
                            .modified()
                            .ok()?;
                        Some((path, modified))
                    } else {
                        None
                    }
                })
            })
            .collect();

        files.sort_by_key(|f| std::cmp::Reverse(f.1));

        for (path, _) in files.iter().skip(self.max_files) {
            let _ = fs::remove_file(path);
        }

        Ok(())
    }

    /// Bütün cycle loglarını oku
    pub fn read_all_cycles(&self) -> Result<Vec<CycleRecord>, String> {
        let mut cycles = Vec::new();
        
        let entries = fs::read_dir(&self.logs_dir)
            .map_err(|e| format!("Klasör okunamadı: {}", e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Entry hatasız: {}", e))?;
            let path = entry.path();
            
            if path.extension().map_or(false, |ext| ext == "json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(cycle) = serde_json::from_str::<CycleRecord>(&content) {
                        cycles.push(cycle);
                    }
                }
            }
        }

        cycles.sort_by_key(|c| std::cmp::Reverse(c.cycle_id));
        Ok(cycles)
    }

    /// Son N cycle'ı oku
    pub fn read_recent_cycles(&self, count: usize) -> Result<Vec<CycleRecord>, String> {
        let mut cycles = self.read_all_cycles()?;
        cycles.truncate(count);
        Ok(cycles)
    }

    /// Seçili alanları CSV formatında export et
    pub fn export_to_csv(&self, cycles: &[CycleRecord]) -> Result<String, String> {
        let mut csv = String::from("cycle_id,started_at,duration_ms,status,symbols_scanned,symbols_selected,graduations_pending,errors\n");

        for cycle in cycles {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                cycle.cycle_id,
                cycle.started_at,
                cycle.total_duration_ms,
                cycle.status,
                cycle.summary.symbols_scanned,
                cycle.summary.symbols_selected,
                cycle.summary.graduations_pending,
                cycle.summary.errors_count,
            ));
        }

        Ok(csv)
    }

    /// HTML rapor oluştur
    pub fn export_to_html(&self, cycles: &[CycleRecord]) -> Result<String, String> {
        let mut html = String::from(
            r#"<!DOCTYPE html>
<html lang="tr">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Autonomous Trader - Audit Report</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 20px; background: #f5f5f5; }
        h1 { color: #333; }
        .cycle { background: white; padding: 15px; margin: 10px 0; border-radius: 8px; }
        .summary { display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px; margin: 10px 0; }
        .stat { background: #f9f9f9; padding: 10px; border-radius: 4px; text-align: center; }
        .stat-label { color: #666; font-size: 0.9em; }
        .stat-value { font-size: 1.5em; font-weight: bold; color: #333; }
        .status-ok { color: #10b981; }
        .status-error { color: #ef4444; }
        table { width: 100%; border-collapse: collapse; margin: 10px 0; }
        th, td { padding: 10px; text-align: left; border-bottom: 1px solid #ddd; }
        th { background: #f0f0f0; }
    </style>
</head>
<body>
    <h1>🤖 Autonomous Trader - Audit Report</h1>
"#
        );

        for cycle in cycles {
            html.push_str(&format!(
                r#"
    <div class="cycle">
        <h2>Cycle #{} <span class="status-{}">[{}]</span></h2>
        <p>Started: {} | Duration: {}ms</p>
        <div class="summary">
            <div class="stat">
                <div class="stat-label">Scanned</div>
                <div class="stat-value">{}</div>
            </div>
            <div class="stat">
                <div class="stat-label">Selected</div>
                <div class="stat-value">{}</div>
            </div>
            <div class="stat">
                <div class="stat-label">Pending</div>
                <div class="stat-value">{}</div>
            </div>
            <div class="stat">
                <div class="stat-label">Errors</div>
                <div class="stat-value">{}</div>
            </div>
        </div>
        <table>
            <thead>
                <tr>
                    <th>Stage</th>
                    <th>Name</th>
                    <th>Status</th>
                    <th>Duration</th>
                </tr>
            </thead>
            <tbody>
"#,
                cycle.cycle_id,
                if cycle.status == "COMPLETED" { "ok" } else { "error" },
                cycle.status,
                cycle.started_at,
                cycle.total_duration_ms,
                cycle.summary.symbols_scanned,
                cycle.summary.symbols_selected,
                cycle.summary.graduations_pending,
                cycle.summary.errors_count,
            ));

            for stage in &cycle.stages {
                html.push_str(&format!(
                    r#"                <tr>
                    <td>{}</td>
                    <td>{}</td>
                    <td class="status-{}">{:?}</td>
                    <td>{}ms</td>
                </tr>
"#,
                    stage.stage,
                    stage.name,
                    if stage.status == AuditStageStatus::Error { "error" } else { "ok" },
                    stage.status,
                    stage.duration_ms,
                ));
            }

            html.push_str("            </tbody>\n        </table>\n    </div>\n");
        }

        html.push_str("</body>\n</html>");
        Ok(html)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cycle_record() {
        let mut cycle = CycleRecord::new(1);
        assert_eq!(cycle.status, "RUNNING");
        
        let stage = StageRecord {
            stage: 1,
            name: "Test".to_string(),
            status: AuditStageStatus::Completed,
            duration_ms: 100,
            input: serde_json::json!({}),
            output: serde_json::json!({}),
            error: None,
            validation: None,
        };
        
        cycle.add_stage(stage);
        assert_eq!(cycle.stages.len(), 1);
    }
}
