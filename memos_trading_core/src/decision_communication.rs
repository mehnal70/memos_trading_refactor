// decision_communication.rs
// Karar ve İletişim Yönetimi Modülü
// Otomatik karar kaydı, bildirim, alarm, insan operatör ile iletişim

use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct DecisionRecord {
    pub decision_id: String,
    pub timestamp: DateTime<Utc>,
    pub description: String,
    pub action: String,
    pub notified: bool,
}

pub trait DecisionCommunicator {
    fn record_decision(&mut self, record: DecisionRecord);
    fn notify_operator(&mut self, decision_id: &str, message: &str);
    fn get_decision(&self, decision_id: &str) -> Option<&DecisionRecord>;
    fn all_decisions(&self) -> &Vec<DecisionRecord>;
}

pub struct SimpleDecisionCommunicator {
    pub records: Vec<DecisionRecord>,
    pub notifications: Vec<NotificationLog>,
}

#[derive(Debug, Clone)]
pub struct NotificationLog {
    pub decision_id: String,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

impl DecisionCommunicator for SimpleDecisionCommunicator {
    fn record_decision(&mut self, record: DecisionRecord) {
        self.records.push(record);
    }
    fn notify_operator(&mut self, decision_id: &str, message: &str) {
        let log = NotificationLog {
            decision_id: decision_id.to_string(),
            message: message.to_string(),
            timestamp: Utc::now(),
        };
        self.notifications.push(log);
        if let Some(rec) = self.records.iter_mut().find(|r| r.decision_id == decision_id) {
            rec.notified = true;
        }
    }
    fn get_decision(&self, decision_id: &str) -> Option<&DecisionRecord> {
        self.records.iter().find(|r| r.decision_id == decision_id)
    }
    fn all_decisions(&self) -> &Vec<DecisionRecord> {
        &self.records
    }
}
