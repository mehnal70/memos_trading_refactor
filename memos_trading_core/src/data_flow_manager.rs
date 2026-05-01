// data_flow_manager.rs
// Veri Akışı ve Kalite Kontrol Modülü
// Veri kaynağı yönetimi, veri bütünlüğü, gecikme ve eksiklik tespiti

use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct DataSource {
    pub name: String,
    pub last_update: DateTime<Utc>,
    pub status: DataSourceStatus,
}

#[derive(Debug, Clone, PartialEq)]
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
    fn all_sources(&self) -> &Vec<DataSource>;
}

pub struct SimpleDataFlowManager {
    pub sources: Vec<DataSource>,
}

impl DataFlowManager for SimpleDataFlowManager {
    fn register_source(&mut self, source: DataSource) {
        self.sources.push(source);
    }
    fn update_status(&mut self, name: &str, status: DataSourceStatus) {
        if let Some(src) = self.sources.iter_mut().find(|s| s.name == name) {
            src.status = status;
            src.last_update = Utc::now();
        }
    }
    fn check_integrity(&self, name: &str) -> DataSourceStatus {
        self.sources.iter().find(|s| s.name == name).map(|s| s.status.clone()).unwrap_or(DataSourceStatus::Missing)
    }
    fn all_sources(&self) -> &Vec<DataSource> {
        &self.sources
    }
}
