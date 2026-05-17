// backup_manager.rs
// Yedekleme ve Geri Yükleme Modülü

use chrono::{DateTime, Utc};
use std::collections::HashMap;

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
    fn all_backups(&self) -> Vec<&BackupRecord>;
}

pub struct SimpleBackupManager {
    // ID bazlı anında erişim için HashMap
    pub backups: HashMap<String, BackupRecord>,
}

impl SimpleBackupManager {
    pub fn new() -> Self {
        Self {
            backups: HashMap::with_capacity(20), // Yedek kayıtları genellikle daha az sayıdadır
        }
    }

    /// Yeni bir yedek kaydı oluşturmak için yardımcı metot
    pub fn generate_backup(&mut self, description: &str, b_type: BackupType) -> String {
        let id = format!("bak-{}-{}", b_type_to_str(&b_type), Utc::now().timestamp());
        let record = BackupRecord {
            backup_id: id.clone(),
            timestamp: Utc::now(),
            description: description.to_owned(),
            backup_type: b_type,
            restored: false,
        };
        self.create_backup(record);
        id
    }
}

impl BackupManager for SimpleBackupManager {
    fn create_backup(&mut self, record: BackupRecord) {
        self.backups.insert(record.backup_id.clone(), record);
    }

    fn restore_backup(&mut self, backup_id: &str) {
        // O(1) erişim ile statü güncelleme
        if let Some(b) = self.backups.get_mut(backup_id) {
            b.restored = true;
            println!("[Backup] {} başarıyla geri yüklendi.", backup_id);
        }
    }

    fn get_backup(&self, backup_id: &str) -> Option<&BackupRecord> {
        self.backups.get(backup_id)
    }

    fn all_backups(&self) -> Vec<&BackupRecord> {
        // Diğer manager'larla uyumlu referans listesi dönüşü
        self.backups.values().collect()
    }
}

// Yardımcı fonksiyon: ID oluşturma için
fn b_type_to_str(b_type: &BackupType) -> &'static str {
    match b_type {
        BackupType::Automatic => "auto",
        BackupType::Manual => "man",
    }
}
