// decision_communication.rs
// Karar ve İletişim Yönetimi Modülü

use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct DecisionRecord {
    pub decision_id: String,
    pub timestamp: DateTime<Utc>,
    pub description: String,
    pub action: String,
    pub notified: bool,
}

#[derive(Debug, Clone)]
pub struct NotificationLog {
    pub decision_id: String,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

pub trait DecisionCommunicator {
    fn record_decision(&mut self, record: DecisionRecord);
    fn notify_operator(&mut self, decision_id: &str, message: &str);
    fn get_decision(&self, decision_id: &str) -> Option<&DecisionRecord>;
    fn all_decisions(&self) -> Vec<&DecisionRecord>;
}

pub struct SimpleDecisionCommunicator {
    // ID bazlı hızlı erişim için kararlar HashMap'te tutulur
    pub records: HashMap<String, DecisionRecord>,
    // Bildirim geçmişi kronolojik olduğu için Vec kalabilir
    pub notifications: Vec<NotificationLog>,
}

impl SimpleDecisionCommunicator {
    pub fn new() -> Self {
        Self {
            records: HashMap::with_capacity(100),
            notifications: Vec::with_capacity(100),
        }
    }
}

impl DecisionCommunicator for SimpleDecisionCommunicator {
    fn record_decision(&mut self, record: DecisionRecord) {
        self.records.insert(record.decision_id.clone(), record);
    }

    fn notify_operator(&mut self, decision_id: &str, message: &str) {
        let log = NotificationLog {
            decision_id: decision_id.to_owned(),
            message: message.to_owned(),
            timestamp: Utc::now(),
        };
        
        self.notifications.push(log);

        // İlgili kararı bul ve 'notified' durumunu anında güncelle
        if let Some(rec) = self.records.get_mut(decision_id) {
            rec.notified = true;
            // Buraya gerçek zamanlı Telegram/Email/Discord gönderim trigger'ı gelebilir
        }
    }

    #[inline]
    fn get_decision(&self, decision_id: &str) -> Option<&DecisionRecord> {
        self.records.get(decision_id)
    }

    fn all_decisions(&self) -> Vec<&DecisionRecord> {
        // Pipeline standartlarına uygun referans listesi dönüşü
        self.records.values().collect()
    }
}
