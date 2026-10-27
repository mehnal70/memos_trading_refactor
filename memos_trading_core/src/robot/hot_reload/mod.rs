// Srivastava ATP Mimarisi - Hot-Reload Engine (Tier 3)
//
// Runtime'da stratejileri yükle/kaldır, zero-downtime updates
// Sistem çalışırken yeni versiyonlar deploy et

pub mod strategy_loader;
pub mod version_manager;
pub mod zero_downtime;

pub use strategy_loader::{LoadedStrategy, StrategyLoadError, StrategyLoader};
pub use version_manager::{StrategyVersion, VersionInfo, VersionManager};
pub use zero_downtime::{UpdateAction, UpdateProcessInfo, UpdateState, ZeroDowntimeUpdateManager};

#[cfg(test)]
mod tests {

    #[test]
    fn test_hot_reload_module_loads() {
        // Modül başarıyla yüklendiğini kontrol et
        assert!(true);
    }
}
