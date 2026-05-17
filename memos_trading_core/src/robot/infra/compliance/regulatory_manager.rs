// regulatory_manager.rs
// Regülasyon ve Uyum Yönetimi Modülü

use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ComplianceEvent {
    pub event_id: String,
    pub timestamp: DateTime<Utc>,
    pub description: String,
    pub event_type: ComplianceType,
    pub resolved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)] // Copy ve Eq eklendi
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
    fn all_events(&self) -> Vec<&ComplianceEvent>;
}

pub struct SimpleRegulatoryManager {
    // ID bazlı O(1) erişim için HashMap
    pub events: HashMap<String, ComplianceEvent>,
}

impl SimpleRegulatoryManager {
    pub fn new() -> Self {
        Self {
            events: HashMap::with_capacity(100),
        }
    }

    /// Çözülmemiş (unresolved) kritik AML/KYC olaylarını filtreler
    pub fn get_pending_compliance_issues(&self) -> Vec<&ComplianceEvent> {
        self.events
            .values()
            .filter(|e| !e.resolved && (e.event_type == ComplianceType::AmlCheck || e.event_type == ComplianceType::KycCheck))
            .collect()
    }
}

impl RegulatoryManager for SimpleRegulatoryManager {
    fn log_event(&mut self, event: ComplianceEvent) {
        self.events.insert(event.event_id.clone(), event);
    }

    fn resolve_event(&mut self, event_id: &str) {
        // Linear search (iter_mut().find) yerine direkt anahtar erişimi
        if let Some(ev) = self.events.get_mut(event_id) {
            ev.resolved = true;
        }
    }

    #[inline]
    fn get_event(&self, event_id: &str) -> Option<&ComplianceEvent> {
        self.events.get(event_id)
    }

    fn all_events(&self) -> Vec<&ComplianceEvent> {
        // Pipeline standartlarına uygun referans listesi dönüşü
        self.events.values().collect()
    }
}
