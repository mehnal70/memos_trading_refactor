// Zero-Downtime Update Manager - Kesintisiz Güncelleme
//
// Blue-Green deployment pattern
// Mevcut stratejiler çalışmaya devam ederken yeni versiyonu hazırla ve geç

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};

/// Update Durumları
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateState {
    /// Güncelleme yok
    Idle,
    /// Yeni versiyon hazırlıyor (blue-green'in blue adımı)
    Preparing,
    /// Yeni versiyon hazır (test ediliyor)
    Ready,
    /// Kesintisiz geçiş yapıyor
    Transitioning,
    /// Güncelleme tamamlandı
    Completed,
    /// Güncelleme iptal edildi veya başarısız
    Rollback,
}

/// Update Aksiyonları
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UpdateAction {
    /// Hiçbir aksiyon
    None,
    
    /// Yeni versiyonu hazırla
    PrepareNewVersion {
        strategy_name: String,
        new_version: String,
    },
    
    /// Yeni versiyonu test et
    TestNewVersion {
        strategy_name: String,
        test_timeout_secs: u64,
    },
    
    /// Blue-Green geçişi yap
    ExecuteBlueGreenSwitch {
        from_version: String,
        to_version: String,
    },
    
    /// Rollback yap
    RollbackToVersion {
        version: String,
        reason: String,
    },
    
    /// Güncellemeyi iptal et
    CancelUpdate {
        reason: String,
    },
}

/// Güncelleme Süreci Bilgisi
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateProcessInfo {
    /// Strateji adı
    pub strategy_name: String,
    /// Eski versiyon
    pub old_version: String,
    /// Yeni versiyon
    pub new_version: String,
    /// Güncelleme başlama zamanı
    pub started_at: DateTime<Utc>,
    /// Tahmin edilen tamamlanma
    pub estimated_completion: DateTime<Utc>,
    /// Mevcut durum
    pub current_state: UpdateState,
    /// Hata mesajı (varsa)
    pub error_message: Option<String>,
}

/// Zero-Downtime Update Manager
pub struct ZeroDowntimeUpdateManager {
    /// Mevcut update süreci (birden fazla olamaz)
    current_update: Option<UpdateProcessInfo>,
    /// Update durumu
    state: UpdateState,
    /// Güncellemelerin sayısı
    total_updates: u32,
    /// Başarılı güncellemelerin sayısı
    successful_updates: u32,
    /// Başarısız güncellemelerin sayısı
    failed_updates: u32,
    /// Rollback'ların sayısı
    rollbacks: u32,
}

impl ZeroDowntimeUpdateManager {
    /// Yeni Update Manager oluştur
    pub fn new() -> Self {
        Self {
            current_update: None,
            state: UpdateState::Idle,
            total_updates: 0,
            successful_updates: 0,
            failed_updates: 0,
            rollbacks: 0,
        }
    }

    /// Update başlat
    pub fn start_update(
        &mut self,
        strategy_name: String,
        old_version: String,
        new_version: String,
    ) -> Result<UpdateAction, String> {
        // Kontrol: Zaten update var mı?
        if self.state != UpdateState::Idle {
            return Err("Update already in progress".to_string());
        }

        self.state = UpdateState::Preparing;
        self.current_update = Some(UpdateProcessInfo {
            strategy_name: strategy_name.clone(),
            old_version,
            new_version: new_version.clone(),
            started_at: Utc::now(),
            estimated_completion: Utc::now() + chrono::Duration::minutes(5),
            current_state: UpdateState::Preparing,
            error_message: None,
        });
        self.total_updates += 1;

        Ok(UpdateAction::PrepareNewVersion {
            strategy_name,
            new_version,
        })
    }

    /// Yeni versiyonu test et
    pub fn test_version(&mut self) -> Result<UpdateAction, String> {
        if self.state != UpdateState::Preparing {
            return Err("Not in preparing state".to_string());
        }

        if let Some(update) = &mut self.current_update {
            self.state = UpdateState::Ready;
            update.current_state = UpdateState::Ready;

            Ok(UpdateAction::TestNewVersion {
                strategy_name: update.strategy_name.clone(),
                test_timeout_secs: 300,  // 5 dakika test süresi
            })
        } else {
            Err("No update in progress".to_string())
        }
    }

    /// Blue-Green geçişi yap
    pub fn execute_blue_green_switch(&mut self) -> Result<UpdateAction, String> {
        if self.state != UpdateState::Ready {
            return Err("Version not ready for deployment".to_string());
        }

        if let Some(update) = &mut self.current_update {
            self.state = UpdateState::Transitioning;
            update.current_state = UpdateState::Transitioning;

            Ok(UpdateAction::ExecuteBlueGreenSwitch {
                from_version: update.old_version.clone(),
                to_version: update.new_version.clone(),
            })
        } else {
            Err("No update in progress".to_string())
        }
    }

    /// Güncellemelerin tamamlandığını işaretle
    pub fn mark_completed(&mut self) -> Result<(), String> {
        if self.state != UpdateState::Transitioning {
            return Err("Not in transitioning state".to_string());
        }

        if let Some(update) = &mut self.current_update {
            self.state = UpdateState::Completed;
            update.current_state = UpdateState::Completed;
            self.successful_updates += 1;

            println!(
                "✓ Güncelleme tamamlandı: {} {} → {}",
                update.strategy_name, update.old_version, update.new_version
            );

            self.current_update = None;
            self.state = UpdateState::Idle;
            Ok(())
        } else {
            Err("No update in progress".to_string())
        }
    }

    /// Güncelleme başarısız oldu - Rollback yap
    pub fn rollback(&mut self, reason: String) -> Result<UpdateAction, String> {
        if let Some(update) = &mut self.current_update {
            self.state = UpdateState::Rollback;
            update.current_state = UpdateState::Rollback;
            update.error_message = Some(reason.clone());
            self.failed_updates += 1;
            self.rollbacks += 1;

            println!(
                "⚠ Rollback: {} versiyonuna geri dönüldü. Neden: {}",
                update.old_version, reason
            );

            let old_version = update.old_version.clone();
            self.current_update = None;
            self.state = UpdateState::Idle;

            Ok(UpdateAction::RollbackToVersion {
                version: old_version,
                reason,
            })
        } else {
            Err("No update in progress".to_string())
        }
    }

    /// Update'i iptal et
    pub fn cancel_update(&mut self, reason: String) -> Result<UpdateAction, String> {
        if self.state == UpdateState::Idle {
            return Err("No update in progress".to_string());
        }

        self.state = UpdateState::Idle;
        self.current_update = None;

        Ok(UpdateAction::CancelUpdate { reason })
    }

    /// Mevcut update süreci bilgisini al
    pub fn current_update_info(&self) -> Option<&UpdateProcessInfo> {
        self.current_update.as_ref()
    }

    /// Güncelleme durumu
    pub fn state(&self) -> UpdateState {
        self.state
    }

    /// İstatistikleri al
    pub fn stats(&self) -> (u32, u32, u32, u32) {
        (self.total_updates, self.successful_updates, self.failed_updates, self.rollbacks)
    }
}

impl Default for ZeroDowntimeUpdateManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manager_creation() {
        let manager = ZeroDowntimeUpdateManager::new();
        assert_eq!(manager.state(), UpdateState::Idle);
    }

    #[test]
    fn test_start_update() {
        let mut manager = ZeroDowntimeUpdateManager::new();
        let result = manager.start_update(
            "ma_cross".to_string(),
            "1.0.0".to_string(),
            "1.1.0".to_string(),
        );

        assert!(result.is_ok());
        assert_eq!(manager.state(), UpdateState::Preparing);
        assert!(manager.current_update_info().is_some());
    }

    #[test]
    fn test_cannot_start_update_while_in_progress() {
        let mut manager = ZeroDowntimeUpdateManager::new();
        let _ = manager.start_update(
            "ma_cross".to_string(),
            "1.0.0".to_string(),
            "1.1.0".to_string(),
        );

        let result = manager.start_update(
            "rsi".to_string(),
            "1.0.0".to_string(),
            "1.1.0".to_string(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_full_update_cycle() {
        let mut manager = ZeroDowntimeUpdateManager::new();

        // 1. Start update
        let _ = manager.start_update(
            "ma_cross".to_string(),
            "1.0.0".to_string(),
            "1.1.0".to_string(),
        );
        assert_eq!(manager.state(), UpdateState::Preparing);

        // 2. Test version
        let _ = manager.test_version();
        assert_eq!(manager.state(), UpdateState::Ready);

        // 3. Blue-Green switch
        let _ = manager.execute_blue_green_switch();
        assert_eq!(manager.state(), UpdateState::Transitioning);

        // 4. Mark completed
        let result = manager.mark_completed();
        assert!(result.is_ok());
        assert_eq!(manager.state(), UpdateState::Idle);

        let (total, successful, failed, rollbacks) = manager.stats();
        assert_eq!(total, 1);
        assert_eq!(successful, 1);
        assert_eq!(failed, 0);
        assert_eq!(rollbacks, 0);
    }

    #[test]
    fn test_rollback_scenario() {
        let mut manager = ZeroDowntimeUpdateManager::new();

        let _ = manager.start_update(
            "ma_cross".to_string(),
            "1.0.0".to_string(),
            "1.1.0".to_string(),
        );
        let _ = manager.test_version();
        let _ = manager.execute_blue_green_switch();

        // Test başarısız
        let result = manager.rollback("Test failed".to_string());
        assert!(result.is_ok());
        assert_eq!(manager.state(), UpdateState::Idle);

        let (total, successful, failed, rollbacks) = manager.stats();
        assert_eq!(total, 1);
        assert_eq!(successful, 0);
        assert_eq!(failed, 1);
        assert_eq!(rollbacks, 1);
    }

    #[test]
    fn test_cancel_update() {
        let mut manager = ZeroDowntimeUpdateManager::new();

        let _ = manager.start_update(
            "ma_cross".to_string(),
            "1.0.0".to_string(),
            "1.1.0".to_string(),
        );

        let result = manager.cancel_update("User requested".to_string());
        assert!(result.is_ok());
        assert_eq!(manager.state(), UpdateState::Idle);
    }
}
