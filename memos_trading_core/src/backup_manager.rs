// backup_manager.rs
// Yedekleme ve Geri Yükleme Modülü
// Otomatik yedekleme, manuel yedekleme, geri yükleme, yedek geçmişi

use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct BackupRecord {
    pub backup_id: String,
    pub timestamp: DateTime<Utc>,
    pub description: String,
    pub backup_type: BackupType,
    pub restored: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BackupType {
    Automatic,
    Manual,
}

pub trait BackupManager {
    fn create_backup(&mut self, record: BackupRecord);
    fn restore_backup(&mut self, backup_id: &str);
    fn get_backup(&self, backup_id: &str) -> Option<&BackupRecord>;
    fn all_backups(&self) -> &Vec<BackupRecord>;
}

pub struct SimpleBackupManager {
    pub backups: Vec<BackupRecord>,
}

impl BackupManager for SimpleBackupManager {
    fn create_backup(&mut self, record: BackupRecord) {
        self.backups.push(record);
    }
    fn restore_backup(&mut self, backup_id: &str) {
        if let Some(b) = self.backups.iter_mut().find(|b| b.backup_id == backup_id) {
            b.restored = true;
        }
    }
    fn get_backup(&self, backup_id: &str) -> Option<&BackupRecord> {
        self.backups.iter().find(|b| b.backup_id == backup_id)
    }
    fn all_backups(&self) -> &Vec<BackupRecord> {
        &self.backups
    }
}
