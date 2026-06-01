// robot/data_pipeline/status.rs - Pipeline çalışma zamanı durumu
//
// Engine'in spawn_infrastructure_fleet'i bu yapıyı doldurur; bridge.rs ise
// MissionControl.pipeline_steps + anomalies alanlarına dönüştürür.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum StepStatus { #[default]
Idle, Running, Done, Failed, Skipped }


#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipelineStepRuntime {
    pub label: String,
    pub status: StepStatus,
    pub last_run_secs: u64,   // En son ne zaman koştu (saniye, epoch'tan)
    pub overdue_secs: u64,    // Aşılan beklenen interval (0 = zamanında)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum AnomalySeverity { #[default]
Info, Warning, Critical }


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum AnomalyKind { DataStall, ApiError, Drift, RiskBreach, #[default]
Custom }


#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipelineAnomalyRuntime {
    pub severity: AnomalySeverity,
    pub kind: AnomalyKind,
    pub message: String,
    pub fix_hint: Option<String>,
    pub auto_fixed: bool,
    /// UNIX epoch saniye — anomaly ilk push edildiği an. purge_stale_warnings
    /// bu değere bakarak eski Warning'leri active sayımdan düşer.
    /// `#[serde(default)]`: eski snapshot/JSON'larla geriye uyumlu (0 = bilinmeyen).
    #[serde(default)]
    pub created_at: u64,
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
        let msg = message.into();
        // Dedupe: aynı severity+kind+message zaten kuyrukta varsa tekrar ekleme.
        // Engine her ~500ms aynı RiskGate Skipped (Kelly edge negatif) anomaly'sini
        // basıyordu → 50 cap'i doluyor, gerçek olaylar (örn. DataStall) eski anomaly'yi
        // attığı için kayboluyordu. Dedupe sonrası queue gerçek olay çeşitliliğini korur.
        if self.anomalies.iter().any(|a| a.severity == severity && a.kind == kind && a.message == msg) {
            return;
        }
        // Stderr'a düşür ki headless modda root cause analizi için görünür olsun;
        // push_log yalnız TUI panel buffer'ına gider, dosya log'una yansımaz.
        log::warn!("anomaly[{:?}/{:?}] {}", severity, kind, msg);
        let now = crate::core::time::now_epoch_secs();
        self.anomalies.push(PipelineAnomalyRuntime {
            severity, kind,
            message: msg,
            fix_hint: None,
            auto_fixed: false,
            created_at: now,
        });
        // Kuyruğu sınırla — eski anomalileri at
        if self.anomalies.len() > 50 { self.anomalies.remove(0); }
    }

    /// Belirli bir yaştan eski Warning anomaly'leri kuyruktan çıkar. Critical
    /// her zaman korunur (operatör görmeli). Stale BEATUSDT/BLESSUSDT ApiError
    /// gibi günler boyu kalıcı warning'lerin perform_anomaly_recovery'i her
    /// cycle tetiklemesini engeller. created_at=0 olan eski kayıtlar default
    /// olarak "stale" sayılır (boot anında zaten purge tetiklenmez).
    /// Döndürdüğü değer: kaç anomaly purge edildi.
    pub fn purge_stale_warnings(&mut self, now_secs: u64, max_age_secs: u64) -> usize {
        if max_age_secs == 0 { return 0; }
        let before = self.anomalies.len();
        self.anomalies.retain(|a| {
            // Critical her zaman tut.
            if matches!(a.severity, AnomalySeverity::Critical) { return true; }
            // created_at=0 (eski snapshot'lardan gelmiş veya bilinmiyor) → koru
            // (purge sadece yaşı bilinenleri etkilesin).
            if a.created_at == 0 { return true; }
            let age = now_secs.saturating_sub(a.created_at);
            age <= max_age_secs
        });
        before - self.anomalies.len()
    }

    /// Kanonik fazların her birini başlangıç anında Idle olarak kayıt eder; UI
    /// pipeline timeline'ı bot açılır açılmaz tüm 7 fazı doğru sırada gösterir
    /// (henüz koşulmamış olarak). Çağrılmazsa fazlar ilk işaretlemede sıralı
    /// gözükmeyebilir (HashMap-veri-yapısı değil ama record_step push order
    /// kanonik sıra yerine ilk-tetiklenen-önce sırası verir).
    pub fn init_canon_stages(&mut self) {
        for stage in super::canon::PipelineStage::ALL {
            self.record_step(stage.label(), StepStatus::Idle, 0, 0);
        }
    }

    /// Bir kanonik aşamanın bittiğini işaretler. `last_run_secs` çağrı anına
    /// (UNIX epoch saniye) eşitlenir; bridge.rs "X saniye önce" yaşını oradan
    /// hesaplar. `status` Done/Failed/Skipped olabilir.
    pub fn mark_stage_completed(&mut self, stage: super::canon::PipelineStage, status: StepStatus) {
        let now = crate::core::time::now_epoch_secs();
        self.record_step(stage.label(), status, now, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_anomaly(severity: AnomalySeverity, msg: &str, created_at: u64) -> PipelineAnomalyRuntime {
        PipelineAnomalyRuntime {
            severity, kind: AnomalyKind::ApiError,
            message: msg.into(),
            fix_hint: None, auto_fixed: false,
            created_at,
        }
    }

    #[test]
    fn purge_keeps_recent_warning_drops_stale_warning() {
        let now: u64 = 10_000;
        let mut p = PipelineStatus::new();
        p.anomalies.push(make_anomaly(AnomalySeverity::Warning, "fresh", now - 100));
        p.anomalies.push(make_anomaly(AnomalySeverity::Warning, "stale", now - 5_000));
        // max_age=1000 → 100sn taze, 5000sn stale → 1 silinir.
        let n = p.purge_stale_warnings(now, 1_000);
        assert_eq!(n, 1, "stale warning silinmedi");
        assert_eq!(p.anomalies.len(), 1);
        assert_eq!(p.anomalies[0].message, "fresh");
    }

    #[test]
    fn purge_never_drops_critical_regardless_of_age() {
        let now: u64 = 200_000;
        let mut p = PipelineStatus::new();
        p.anomalies.push(make_anomaly(AnomalySeverity::Critical, "ancient crit", now - 100_000));
        let n = p.purge_stale_warnings(now, 60);
        assert_eq!(n, 0, "Critical purge edildi (olmamalıydı)");
        assert_eq!(p.anomalies.len(), 1);
    }

    #[test]
    fn purge_keeps_created_at_zero_warnings_for_backward_compat() {
        // created_at=0 → eski snapshot/JSON'dan deserialize edilmiş olabilir.
        // Yaşı bilinmeyenleri purge etme; sadece açıkça yaşlı olanları sil.
        let now: u64 = 10_000;
        let mut p = PipelineStatus::new();
        p.anomalies.push(make_anomaly(AnomalySeverity::Warning, "no-ts", 0));
        let n = p.purge_stale_warnings(now, 60);
        assert_eq!(n, 0);
        assert_eq!(p.anomalies.len(), 1);
    }

    #[test]
    fn purge_with_zero_max_age_is_noop() {
        // max_age_secs=0 → guard kapalı, hiçbir şey silinmesin.
        let now: u64 = 10_000;
        let mut p = PipelineStatus::new();
        p.anomalies.push(make_anomaly(AnomalySeverity::Warning, "old", now - 5_000));
        let n = p.purge_stale_warnings(now, 0);
        assert_eq!(n, 0);
        assert_eq!(p.anomalies.len(), 1);
    }

    #[test]
    fn push_anomaly_stamps_created_at() {
        let mut p = PipelineStatus::new();
        p.push_anomaly(AnomalySeverity::Warning, AnomalyKind::ApiError, "test");
        assert_eq!(p.anomalies.len(), 1);
        // created_at şu anki epoch'a yakın olmalı (>0)
        assert!(p.anomalies[0].created_at > 0, "created_at otomatik damgalanmadı");
    }
}
