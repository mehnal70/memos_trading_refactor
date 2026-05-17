// data_flow_manager.rs
// Veri Akışı ve Kalite Kontrol Modülü

use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct DataSource {
    pub name: String,
    pub last_update: DateTime<Utc>,
    pub status: DataSourceStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataSourceStatus {
    Healthy,
    Delayed,
    Missing,
    Corrupted,
}

pub trait DataFlowManager {
    fn register_source(&mut self, source: DataSource);
    fn update_status(&mut self, name: &str, status: DataSourceStatus);
    fn check_integrity(&self, name: &str) -> DataSourceStatus;
    fn all_sources(&self) -> Vec<&DataSource>;
}

pub struct SimpleDataFlowManager {
    // İsim bazlı O(1) erişim için HashMap
    pub sources: HashMap<String, DataSource>,
}

impl SimpleDataFlowManager {
    pub fn new() -> Self {
        Self {
            sources: HashMap::with_capacity(10),
        }
    }

    /// Veri kaynağının ne kadar süredir güncellenmediğini kontrol eder
    pub fn get_latency_ms(&self, name: &str) -> Option<i64> {
        self.sources.get(name).map(|s| {
            Utc::now().signed_duration_since(s.last_update).num_milliseconds()
        })
    }
}

impl DataFlowManager for SimpleDataFlowManager {
    fn register_source(&mut self, source: DataSource) {
        self.sources.insert(source.name.clone(), source);
    }

    fn update_status(&mut self, name: &str, status: DataSourceStatus) {
        if let Some(src) = self.sources.get_mut(name) {
            src.status = status;
            src.last_update = Utc::now();
        }
    }

    fn check_integrity(&self, name: &str) -> DataSourceStatus {
        self.sources
            .get(name)
            .map(|s| s.status.clone())
            .unwrap_or(DataSourceStatus::Missing)
    }

    fn all_sources(&self) -> Vec<&DataSource> {
        // Bellek kopyalamasını önlemek için referans listesi dönüyoruz
        self.sources.values().collect()
    }
}
