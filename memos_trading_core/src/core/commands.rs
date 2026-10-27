// src/core/commands.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RobotCommand {
    TriggerMl,           // [m]
    TriggerBacktest,     // [b]
    ToggleAutoMode,      // [s]
    StartDownload,       // [d]
    StartMt5Bootstrap,   // [x]
    StartVenueRecovery,  // [r] Venue Sağlığı
    ForceCloseAll,       // [c]
    SyncExchange,        // [o]
    SetInterval(String), // [i]
    UpdateCapital(f64),  // Ayarlar
    GracefulShutdown,    // [q]
    ResetPaperBalance,   // [z]
}
