// automation_scheduler.rs
// Otomasyon Scheduler Modülü - Performans ve Asenkron Odaklı

use tokio::time::{Duration, sleep};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock; // Mutex yerine RwLock: Okuma operasyonları çok daha hızlıdır.
use chrono::{Utc, DateTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)] // Copy eklendi, hafif enum
pub enum AutomationTask {
    DataSync,
    Backtest,
    RiskCheck,
    Reporting,
    Backup,
    LogMonitor,
    UpdateCheck,
}

#[derive(Debug, Clone)]
pub struct ScheduledTask {
    pub task: AutomationTask,
    pub interval_sec: u64,
    pub last_run: Option<DateTime<Utc>>,
    pub enabled: bool,
}

#[derive(Debug, Default)]
pub struct AutomationScheduler {
    // Okuma (get_status) ağırlıklı olduğu için RwLock kullanımı performansı artırır.
    pub tasks: Arc<RwLock<HashMap<String, ScheduledTask>>>,
}

impl AutomationScheduler {
    pub fn new() -> Self {
        let mut tasks = HashMap::with_capacity(7); // Allocation optimizasyonu
        
        // Helper closure: Kod tekrarını azaltır
        let mut add_task = |name: &str, task: AutomationTask, secs: u64| {
            tasks.insert(name.to_string(), ScheduledTask {
                task,
                interval_sec: secs,
                last_run: None,
                enabled: true,
            });
        };

        add_task("data_sync", AutomationTask::DataSync, 3600);
        add_task("backtest", AutomationTask::Backtest, 86400);
        add_task("risk_check", AutomationTask::RiskCheck, 600);
        add_task("reporting", AutomationTask::Reporting, 86400);
        add_task("backup", AutomationTask::Backup, 43200);
        add_task("log_monitor", AutomationTask::LogMonitor, 300);
        add_task("update_check", AutomationTask::UpdateCheck, 86400);

        Self {
            tasks: Arc::new(RwLock::new(tasks)),
        }
    }

    pub async fn start(&self) {
        let tasks_ptr = self.tasks.clone();
        
        tokio::spawn(async move {
            loop {
                let now = Utc::now();
                // Task listesini tara (Write lock yerine sadece gerektiğinde kilit açacağız)
                let mut tasks_to_run = Vec::new();

                {
                    let tasks_read = tasks_ptr.read().await;
                    for (name, task) in tasks_read.iter() {
                        if task.enabled {
                            let should_run = match task.last_run {
                                Some(last) => (now - last).num_seconds() >= task.interval_sec as i64,
                                None => true,
                            };
                            if should_run {
                                tasks_to_run.push((name.clone(), task.task));
                            }
                        }
                    }
                } // Read lock burada düşer (drop)

                // Görevleri paralel başlat
                for (name, task_kind) in tasks_to_run {
                    let tasks_write_ptr = tasks_ptr.clone();
                    tokio::spawn(async move {
                        // Görevi çalıştır
                        Self::run_task(task_kind).await;
                        
                        // Son çalışma zamanını güncelle
                        let mut write_guard = tasks_write_ptr.write().await;
                        if let Some(t) = write_guard.get_mut(&name) {
                            t.last_run = Some(Utc::now());
                        }
                    });
                }

                sleep(Duration::from_secs(10)).await; // Kontrol periyodu sıkılaştırıldı
            }
        });
    }

    pub async fn run_task(task: AutomationTask) {
        // Modern Rust: Match kollarında loglama
        match task {
            AutomationTask::DataSync => println!("[Automation] 🔄 DataSync başlatıldı"),
            AutomationTask::RiskCheck => println!("[Automation] 🛡️ RiskCheck başlatıldı"),
            AutomationTask::LogMonitor => println!("[Automation] 🔍 LogMonitor başlatıldı"),
            _ => println!("[Automation] {:?} çalıştırılıyor...", task),
        }
    }

    pub async fn enable_task(&self, name: &str, enable: bool) {
        let mut guard = self.tasks.write().await;
        if let Some(task) = guard.get_mut(name) {
            task.enabled = enable;
        }
    }

    pub async fn get_status(&self) -> HashMap<String, ScheduledTask> {
        self.tasks.read().await.clone()
    }
}
