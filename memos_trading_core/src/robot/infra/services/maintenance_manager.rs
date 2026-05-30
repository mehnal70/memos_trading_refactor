// maintenance_manager.rs
// Bakım, Günlükleme ve Otomatik Kurtarma Modülü

use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct MaintenanceEvent {
    pub event_id: String,
    pub timestamp: DateTime<Utc>,
    pub description: String,
    pub event_type: MaintenanceType,
    pub resolved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    fn all_events(&self) -> Vec<&MaintenanceEvent>;
}

pub struct SimpleMaintenanceManager {
    // ID bazlı anında erişim için HashMap (O(1) kompleksite)
    pub events: HashMap<String, MaintenanceEvent>,
}

impl Default for SimpleMaintenanceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SimpleMaintenanceManager {
    pub fn new() -> Self {
        Self {
            events: HashMap::with_capacity(100),
        }
    }

    /// Belirli bir tipteki çözülmemiş (açık) olayları döndürür
    pub fn get_unresolved_by_type(&self, m_type: MaintenanceType) -> Vec<&MaintenanceEvent> {
        self.events
            .values()
            .filter(|e| e.event_type == m_type && !e.resolved)
            .collect()
    }
}

impl MaintenanceManager for SimpleMaintenanceManager {
    fn log_event(&mut self, event: MaintenanceEvent) {
        self.events.insert(event.event_id.clone(), event);
    }

    fn resolve_event(&mut self, event_id: &str) {
        // Linear search (iter_mut().find) yerine direkt key-access
        if let Some(ev) = self.events.get_mut(event_id) {
            ev.resolved = true;
            // log::info!("Olay çözüldü: {}", event_id);
        }
    }

    #[inline]
    fn get_event(&self, event_id: &str) -> Option<&MaintenanceEvent> {
        self.events.get(event_id)
    }

    fn all_events(&self) -> Vec<&MaintenanceEvent> {
        // Veriyi kopyalamadan (cloning) referans listesi topluyoruz
        self.events.values().collect()
    }
}
