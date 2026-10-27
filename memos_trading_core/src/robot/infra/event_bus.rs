// robot/infra/event_bus.rs - Kanonik iç event akışı + replay log

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast;

use crate::MemosTradingError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalEvent {
    pub timestamp: String,
    pub domain: String,
    pub event_type: String,
    pub venue: String,
    pub symbol: String,
    pub payload: serde_json::Value,
}

impl CanonicalEvent {
    pub fn from_trade_event(ev: &crate::robot::infra::logger::TradeEvent) -> Self {
        let (_, tagged_venue) = crate::robot::venue::registry::split_venue_tag(&ev.symbol);
        let venue = tagged_venue
            .unwrap_or_else(|| crate::core::types::Exchange::classify(&ev.symbol))
            .as_str()
            .to_string();

        let domain = if ev.event_type.starts_with("ORDER_") || ev.event_type.starts_with("TRADE_") {
            "execution"
        } else if ev.event_type == "RISK_BLOCK" {
            "risk"
        } else if ev.event_type == "ERROR" {
            "system"
        } else {
            "signal"
        };

        Self {
            timestamp: Utc::now().to_rfc3339(),
            domain: domain.to_string(),
            event_type: ev.event_type.clone(),
            venue,
            symbol: ev.symbol.clone(),
            payload: json!({
                "signal": ev.signal,
                "strategy": ev.strategy,
                "price": ev.price,
                "quantity": ev.quantity,
                "pnl": ev.pnl,
                "equity": ev.equity,
                "message": ev.message,
            }),
        }
    }
}

#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<CanonicalEvent>,
    replay_path: String,
}

impl EventBus {
    pub fn new(capacity: usize, replay_path: &str) -> Result<Self, MemosTradingError> {
        if let Some(parent) = Path::new(replay_path).parent() {
            std::fs::create_dir_all(parent).map_err(MemosTradingError::Io)?;
        }
        let (sender, _) = broadcast::channel(capacity.max(64));
        Ok(Self {
            sender,
            replay_path: replay_path.to_string(),
        })
    }

    pub fn new_from_env() -> Result<Self, MemosTradingError> {
        let capacity = std::env::var("EVENT_BUS_CAPACITY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(2048);
        let replay_path = std::env::var("EVENT_REPLAY_LOG_PATH")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "logs/events_replay.jsonl".to_string());
        Self::new(capacity, &replay_path)
    }

    pub fn publish_trade_event(
        &self,
        ev: &crate::robot::infra::logger::TradeEvent,
    ) -> Result<(), MemosTradingError> {
        self.publish(CanonicalEvent::from_trade_event(ev))
    }

    pub fn publish(&self, event: CanonicalEvent) -> Result<(), MemosTradingError> {
        let _ = self.sender.send(event.clone());
        self.append_replay(&event)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CanonicalEvent> {
        self.sender.subscribe()
    }

    fn append_replay(&self, event: &CanonicalEvent) -> Result<(), MemosTradingError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.replay_path)
            .map_err(MemosTradingError::Io)?;
        let line = serde_json::to_string(event).map_err(MemosTradingError::Serde)?;
        writeln!(file, "{}", line).map_err(MemosTradingError::Io)?;
        Ok(())
    }
}
