// maintenance_manager.rs
// Bakım, Günlükleme ve Otomatik Kurtarma Modülü
// Planlı bakım, hata loglama, otomatik kurtarma, bakım geçmişi

use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct MaintenanceEvent {
    pub event_id: String,
    pub timestamp: DateTime<Utc>,
    pub description: String,
    pub event_type: MaintenanceType,
    pub resolved: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MaintenanceType {
    Scheduled,
    Unscheduled,
    Recovery,
    Log,
}

pub trait MaintenanceManager {
    fn log_event(&mut self, event: MaintenanceEvent);
    fn resolve_event(&mut self, event_id: &str);
    fn get_event(&self, event_id: &str) -> Option<&MaintenanceEvent>;
    fn all_events(&self) -> &Vec<MaintenanceEvent>;
}

pub struct SimpleMaintenanceManager {
    pub events: Vec<MaintenanceEvent>,
}

impl MaintenanceManager for SimpleMaintenanceManager {
    fn log_event(&mut self, event: MaintenanceEvent) {
        self.events.push(event);
    }
    fn resolve_event(&mut self, event_id: &str) {
        if let Some(ev) = self.events.iter_mut().find(|e| e.event_id == event_id) {
            ev.resolved = true;
        }
    }
    fn get_event(&self, event_id: &str) -> Option<&MaintenanceEvent> {
        self.events.iter().find(|e| e.event_id == event_id)
    }
    fn all_events(&self) -> &Vec<MaintenanceEvent> {
        &self.events
    }
}
