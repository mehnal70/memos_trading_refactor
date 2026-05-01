// Version Manager - Strateji Versiyonlaması ve Yönetimi
//
// Semantik versiyonlama: major.minor.patch
// Rollback desteği, version history

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use std::collections::VecDeque;

/// Strateji Versiyonu
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyVersion {
    /// major.minor.patch formatında
    pub version: String,
    /// major
    pub major: u32,
    /// minor
    pub minor: u32,
    /// patch
    pub patch: u32,
}

impl StrategyVersion {
    /// Yeni version oluştur
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            version: format!("{}.{}.{}", major, minor, patch),
            major,
            minor,
            patch,
        }
    }

    /// String'den parse et (örnek: "1.2.3")
    pub fn parse(version_str: &str) -> Result<Self, String> {
        let parts: Vec<&str> = version_str.split('.').collect();
        if parts.len() != 3 {
            return Err("Version must be in format major.minor.patch".to_string());
        }

        let major = parts[0]
            .parse::<u32>()
            .map_err(|_| "Invalid major version")?;
        let minor = parts[1]
            .parse::<u32>()
            .map_err(|_| "Invalid minor version")?;
        let patch = parts[2]
            .parse::<u32>()
            .map_err(|_| "Invalid patch version")?;

        Ok(Self::new(major, minor, patch))
    }

    /// Bir versiyondan diğerine uyumlu mu?
    pub fn is_compatible(&self, other: &StrategyVersion) -> bool {
        // Minor ve patch değişiklikleri uyumlu, major ise uyumsuz
        self.major == other.major
    }

    /// Versiyonu karşılaştır
    pub fn compare(&self, other: &StrategyVersion) -> std::cmp::Ordering {
        match self.major.cmp(&other.major) {
            std::cmp::Ordering::Equal => match self.minor.cmp(&other.minor) {
                std::cmp::Ordering::Equal => self.patch.cmp(&other.patch),
                other => other,
            },
            other => other,
        }
    }
}

/// Yayınlanan Versiyon Bilgisi
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    /// Strateji adı
    pub strategy_name: String,
    /// Versiyon
    pub version: StrategyVersion,
    /// Yayın zamanı
    pub released_at: DateTime<Utc>,
    /// Release notes
    pub release_notes: String,
    /// Kütüphane gereksinimleri
    pub requirements: Vec<String>,
    /// Breaking changes var mı?
    pub breaking_changes: bool,
}

impl VersionInfo {
    /// Yeni version info oluştur
    pub fn new(
        strategy_name: String,
        version: StrategyVersion,
        release_notes: String,
    ) -> Self {
        Self {
            strategy_name,
            version,
            released_at: Utc::now(),
            release_notes,
            requirements: vec![],
            breaking_changes: false,
        }
    }
}

/// Version Manager
pub struct VersionManager {
    /// Versiyon history (en son = ilk eleman)
    history: VecDeque<VersionInfo>,
    /// Mevcut versiyon
    current_version: Option<StrategyVersion>,
    /// Max history size
    max_history: usize,
}

impl VersionManager {
    /// Yeni Version Manager oluştur
    pub fn new() -> Self {
        Self {
            history: VecDeque::new(),
            current_version: None,
            max_history: 20,  // Son 20 versiyonu tut
        }
    }

    /// Versiyonu yayınla
    pub fn release_version(&mut self, info: VersionInfo) {
        self.current_version = Some(info.version.clone());
        self.history.push_front(info);

        // History boyutunu kontrol et
        while self.history.len() > self.max_history {
            self.history.pop_back();
        }
    }

    /// Mevcut versiyonu al
    pub fn current_version(&self) -> Option<&StrategyVersion> {
        self.current_version.as_ref()
    }

    /// Mevcut version info'sunu al
    pub fn current_info(&self) -> Option<&VersionInfo> {
        self.history.front()
    }

    /// Versiyonlar arasında upgrade mümkün mü?
    pub fn can_upgrade(&self, from: &StrategyVersion, to: &StrategyVersion) -> bool {
        // Önceki versiyondan daha yeni versiyona gitmeli
        from.compare(to) == std::cmp::Ordering::Less
    }

    /// Versiyonlar arasında downgrade mümkün mü?
    pub fn can_downgrade(&self, from: &StrategyVersion, to: &StrategyVersion) -> bool {
        // Mevcut versiyondan daha eski versiyona gitmeli
        from.compare(to) == std::cmp::Ordering::Greater
        // Breaking changes olmayan minor/patch versiyonları
    }

    /// Belirli bir versiyona geri dön (rollback)
    pub fn rollback(&mut self, target_version: &str) -> Result<VersionInfo, String> {
        for info in &self.history {
            if info.version.version == target_version {
                self.current_version = Some(info.version.clone());
                // Bu versiyonu en başa koy
                if let Some(pos) = self.history.iter().position(|v| v.version.version == target_version) {
                    let rolled_back = self.history.remove(pos).unwrap();
                    self.history.push_front(rolled_back.clone());
                    return Ok(rolled_back);
                }
            }
        }

        Err(format!("Version not found in history: {}", target_version))
    }

    /// Sürüm geçmişini al
    pub fn history(&self) -> Vec<&VersionInfo> {
        self.history.iter().collect()
    }

    /// Son N versiyonu al
    pub fn recent_versions(&self, count: usize) -> Vec<&VersionInfo> {
        self.history.iter().take(count).collect()
    }

    /// Değişikliklerin özeti (eski versiyon ile yeni versiyon arasında)
    pub fn changelog(&self, from_version: &str, to_version: &str) -> Result<Vec<String>, String> {
        let mut changes = Vec::new();

        let mut include = false;
        for info in &self.history {
            if info.version.version == to_version {
                include = true;
            }

            if include {
                changes.push(format!(
                    "{}: {}",
                    info.version.version, info.release_notes
                ));
            }

            if info.version.version == from_version {
                break;
            }
        }

        if changes.is_empty() {
            return Err("No changes found between versions".to_string());
        }

        Ok(changes)
    }
}

impl Default for VersionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_creation() {
        let v = StrategyVersion::new(1, 2, 3);
        assert_eq!(v.version, "1.2.3");
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }

    #[test]
    fn test_version_parse() {
        let v = StrategyVersion::parse("1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }

    #[test]
    fn test_version_parse_invalid() {
        let result = StrategyVersion::parse("1.2");
        assert!(result.is_err());
    }

    #[test]
    fn test_version_compatibility() {
        let v1 = StrategyVersion::new(1, 0, 0);
        let v2 = StrategyVersion::new(1, 1, 0);
        let v3 = StrategyVersion::new(2, 0, 0);

        assert!(v1.is_compatible(&v2));  // Same major
        assert!(!v1.is_compatible(&v3));  // Different major
    }

    #[test]
    fn test_version_comparison() {
        let v1 = StrategyVersion::new(1, 0, 0);
        let v2 = StrategyVersion::new(1, 1, 0);
        let v3 = StrategyVersion::new(1, 1, 1);

        assert_eq!(v1.compare(&v2), std::cmp::Ordering::Less);
        assert_eq!(v2.compare(&v3), std::cmp::Ordering::Less);
        assert_eq!(v3.compare(&v3), std::cmp::Ordering::Equal);
    }

    #[test]
    fn test_version_manager_release() {
        let mut manager = VersionManager::new();
        let v = StrategyVersion::new(1, 0, 0);
        let info = VersionInfo::new(
            "ma_cross".to_string(),
            v.clone(),
            "Initial release".to_string(),
        );

        manager.release_version(info);

        assert_eq!(manager.current_version().unwrap().version, "1.0.0");
    }

    #[test]
    fn test_version_manager_upgrade() {
        let manager = VersionManager::new();
        let v1 = StrategyVersion::new(1, 0, 0);
        let v2 = StrategyVersion::new(1, 1, 0);

        assert!(manager.can_upgrade(&v1, &v2));
        assert!(!manager.can_upgrade(&v2, &v1));
    }

    #[test]
    fn test_version_manager_rollback() {
        let mut manager = VersionManager::new();
        let v1 = StrategyVersion::new(1, 0, 0);
        let v2 = StrategyVersion::new(1, 1, 0);

        let info1 = VersionInfo::new(
            "ma_cross".to_string(),
            v1,
            "Initial release".to_string(),
        );
        let info2 = VersionInfo::new(
            "ma_cross".to_string(),
            v2,
            "Minor update".to_string(),
        );

        manager.release_version(info1);
        manager.release_version(info2);

        let rollback = manager.rollback("1.0.0");
        assert!(rollback.is_ok());
        assert_eq!(manager.current_version().unwrap().version, "1.0.0");
    }

    #[test]
    fn test_version_manager_history() {
        let mut manager = VersionManager::new();
        let v1 = StrategyVersion::new(1, 0, 0);
        let v2 = StrategyVersion::new(1, 1, 0);

        let info1 = VersionInfo::new(
            "ma_cross".to_string(),
            v1,
            "Initial".to_string(),
        );
        let info2 = VersionInfo::new(
            "ma_cross".to_string(),
            v2,
            "Update".to_string(),
        );

        manager.release_version(info1);
        manager.release_version(info2);

        let history = manager.history();
        assert_eq!(history.len(), 2);
    }
}
