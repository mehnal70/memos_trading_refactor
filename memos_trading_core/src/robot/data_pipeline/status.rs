// robot/data_pipeline/status.rs - Pipeline çalışma zamanı durumu
//
// Engine'in spawn_infrastructure_fleet'i bu yapıyı doldurur; bridge.rs ise
// MissionControl.pipeline_steps + anomalies alanlarına dönüştürür.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus { Idle, Running, Done, Failed, Skipped }

impl Default for StepStatus { fn default() -> Self { StepStatus::Idle } }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipelineStepRuntime {
    pub label: String,
    pub status: StepStatus,
    pub last_run_secs: u64,   // En son ne zaman koştu (saniye, epoch'tan)
    pub overdue_secs: u64,    // Aşılan beklenen interval (0 = zamanında)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnomalySeverity { Info, Warning, Critical }

impl Default for AnomalySeverity { fn default() -> Self { AnomalySeverity::Info } }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnomalyKind { DataStall, ApiError, Drift, RiskBreach, Custom }

impl Default for AnomalyKind { fn default() -> Self { AnomalyKind::Custom } }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipelineAnomalyRuntime {
    pub severity: AnomalySeverity,
    pub kind: AnomalyKind,
    pub message: String,
    pub fix_hint: Option<String>,
    pub auto_fixed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipelineStatus {
    pub chain_steps: Vec<PipelineStepRuntime>,
    pub anomalies:   Vec<PipelineAnomalyRuntime>,
}

impl PipelineStatus {
    pub fn new() -> Self { Self::default() }

    pub fn record_step(&mut self, label: impl Into<String>, status: StepStatus, last_run_secs: u64, overdue_secs: u64) {
        let label = label.into();
        if let Some(s) = self.chain_steps.iter_mut().find(|s| s.label == label) {
            s.status = status;
            s.last_run_secs = last_run_secs;
            s.overdue_secs = overdue_secs;
        } else {
            self.chain_steps.push(PipelineStepRuntime { label, status, last_run_secs, overdue_secs });
        }
    }

    pub fn push_anomaly(&mut self, severity: AnomalySeverity, kind: AnomalyKind, message: impl Into<String>) {
        self.anomalies.push(PipelineAnomalyRuntime {
            severity, kind,
            message: message.into(),
            fix_hint: None,
            auto_fixed: false,
        });
        // Kuyruğu sınırla — eski anomalileri at
        if self.anomalies.len() > 50 { self.anomalies.remove(0); }
    }
}
