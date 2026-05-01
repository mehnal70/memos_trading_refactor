// update_manager.rs
// Otonom Güncelleme ve Sürüm Geçişi Modülü
// Zero-downtime deployment, rollback, otomatik test sonrası canlıya alma

use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub deployed_at: DateTime<Utc>,
    pub status: UpdateStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateStatus {
    Deploying,
    Success,
    Failed(String),
    RolledBack,
}

pub trait UpdateManager {
    fn deploy_new_version(&mut self, version: &str) -> UpdateStatus;
    fn rollback(&mut self) -> UpdateStatus;
    fn last_update(&self) -> Option<&UpdateInfo>;
}

pub struct SimpleUpdateManager {
    pub updates: Vec<UpdateInfo>,
}

impl UpdateManager for SimpleUpdateManager {
    fn deploy_new_version(&mut self, version: &str) -> UpdateStatus {
        let info = UpdateInfo {
            version: version.to_string(),
            deployed_at: Utc::now(),
            status: UpdateStatus::Deploying,
        };
        self.updates.push(info);
        // Otomatik test ve canlıya alma simülasyonu
        // Gerçek ortamda burada test ve deploy scriptleri tetiklenir
        UpdateStatus::Success
    }
    fn rollback(&mut self) -> UpdateStatus {
        if let Some(last) = self.updates.last_mut() {
            last.status = UpdateStatus::RolledBack;
            UpdateStatus::RolledBack
        } else {
            UpdateStatus::Failed("Rollback için güncelleme yok".to_string())
        }
    }
    fn last_update(&self) -> Option<&UpdateInfo> {
        self.updates.last()
    }
}
