// update_manager.rs
// Otonom Güncelleme ve Sürüm Geçişi Modülü

use chrono::{DateTime, Utc};
use std::fmt;

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub deployed_at: DateTime<Utc>,
    pub status: UpdateStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    Deploying,
    Success,
    Failed(String),
    RolledBack,
}

// Loglama ve Dashboard için Display desteği
impl fmt::Display for UpdateStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deploying => write!(f, "🚀 Dağıtılıyor"),
            Self::Success   => write!(f, "✅ Başarılı"),
            Self::RolledBack => write!(f, "🔄 Geri Alındı"),
            Self::Failed(e) => write!(f, "❌ Hata: {}", e),
        }
    }
}

pub trait UpdateManager: Send + Sync {
    fn deploy_new_version(&mut self, version: &str) -> UpdateStatus;
    fn rollback(&mut self) -> UpdateStatus;
    fn last_update(&self) -> Option<&UpdateInfo>;
    fn history(&self) -> &[UpdateInfo]; // &Vec yerine Slice (&[T])
}

pub struct SimpleUpdateManager {
    /// Güncelleme geçmişi (Audit için saklanır)
    pub updates: Vec<UpdateInfo>,
}

impl Default for SimpleUpdateManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SimpleUpdateManager {
    pub fn new() -> Self {
        Self {
            updates: Vec::with_capacity(10), // Güncellemeler seyrek olduğu için küçük kapasite yeterli
        }
    }
}

impl UpdateManager for SimpleUpdateManager {
    /// Yeni bir sürümü otonom olarak yayına alır
    fn deploy_new_version(&mut self, version: &str) -> UpdateStatus {
        let info = UpdateInfo {
            version: version.to_owned(),
            deployed_at: Utc::now(),
            status: UpdateStatus::Deploying,
        };
        
        self.updates.push(info);
        
        // Pipeline: Burada 'HealthOrchestrator' üzerinden otomatik testler tetiklenebilir
        // Başarılı varsayıyoruz (Simülasyon)
        if let Some(last) = self.updates.last_mut() {
            last.status = UpdateStatus::Success;
        }

        UpdateStatus::Success
    }

    /// Hatalı güncellemeyi milisaniyeler içinde geri alır (Rollback)
    fn rollback(&mut self) -> UpdateStatus {
        // En son başarılı olmayan veya hatalı olan güncellemeyi bul
        if let Some(last) = self.updates.last_mut() {
            if last.status == UpdateStatus::RolledBack {
                return UpdateStatus::Failed("Zaten geri alınmış.".to_owned());
            }
            
            last.status = UpdateStatus::RolledBack;
            println!("[DEPLOY] 🔄 Versiyon {} geri alındı.", last.version);
            UpdateStatus::RolledBack
        } else {
            UpdateStatus::Failed("Geçmişte kayıtlı güncelleme bulunamadı.".to_owned())
        }
    }

    #[inline]
    fn last_update(&self) -> Option<&UpdateInfo> {
        self.updates.last()
    }

    #[inline]
    fn history(&self) -> &[UpdateInfo] {
        &self.updates
    }
}
