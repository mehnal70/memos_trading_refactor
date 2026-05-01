// regulatory_manager.rs
// Regülasyon ve Uyum Yönetimi Modülü
// KYC/AML kontrolleri, regülasyon logları, uyum denetimi

use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct ComplianceEvent {
    pub event_id: String,
    pub timestamp: DateTime<Utc>,
    pub description: String,
    pub event_type: ComplianceType,
    pub resolved: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComplianceType {
    KycCheck,
    AmlCheck,
    Audit,
    RegulationChange,
}

pub trait RegulatoryManager {
    fn log_event(&mut self, event: ComplianceEvent);
    fn resolve_event(&mut self, event_id: &str);
    fn get_event(&self, event_id: &str) -> Option<&ComplianceEvent>;
    fn all_events(&self) -> &Vec<ComplianceEvent>;
}

pub struct SimpleRegulatoryManager {
    pub events: Vec<ComplianceEvent>,
}

impl RegulatoryManager for SimpleRegulatoryManager {
    fn log_event(&mut self, event: ComplianceEvent) {
        self.events.push(event);
    }
    fn resolve_event(&mut self, event_id: &str) {
        if let Some(ev) = self.events.iter_mut().find(|e| e.event_id == event_id) {
            ev.resolved = true;
        }
    }
    fn get_event(&self, event_id: &str) -> Option<&ComplianceEvent> {
        self.events.iter().find(|e| e.event_id == event_id)
    }
    fn all_events(&self) -> &Vec<ComplianceEvent> {
        &self.events
    }
}
