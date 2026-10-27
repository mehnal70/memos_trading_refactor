use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock, RwLock,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::core::model::TradingMode;
use crate::robot::robotic_loop::AppState;

#[derive(Debug, Serialize)]
struct LockContentionRecord {
    timestamp: String,
    operation_scope: String,
    trading_mode: String,
    live_dry_run: bool,
    pid: u32,
    process_start_epoch_ms: u128,
    boot_id: String,
    contention_age_ms: u128,
    lock_intent: String,
    thread_states: Vec<String>,
}

static LOCK_INTENT: OnceLock<RwLock<String>> = OnceLock::new();

pub fn set_intent(intent: &str) {
    if let Ok(mut current) = LOCK_INTENT
        .get_or_init(|| RwLock::new(String::new()))
        .write()
    {
        *current = intent.to_string();
    }
}

pub fn clear_intent(intent: &str) {
    if let Ok(mut current) = LOCK_INTENT
        .get_or_init(|| RwLock::new(String::new()))
        .write()
    {
        if current.as_str() == intent {
            current.clear();
        }
    }
}

pub fn spawn(state: Arc<Mutex<AppState>>) {
    let pid = std::process::id();
    let (stop, operation_scope, trading_mode, live_dry_run) = state
        .lock()
        .map(|st| {
            (
                Arc::clone(&st.app_stop_signal),
                operation_scope(&st.config.trading_mode, st.live_dry_run),
                st.config.trading_mode.as_str().to_string(),
                st.live_dry_run,
            )
        })
        .unwrap_or_else(|_| {
            (
                Arc::new(AtomicBool::new(false)),
                "test".to_string(),
                "Unknown".to_string(),
                false,
            )
        });
    let process_start_epoch_ms = epoch_ms();
    let boot_id = read_boot_id();
    let path = diagnostic_path(&operation_scope, pid);

    thread::Builder::new()
        .name("app-state-lock-watchdog".to_string())
        .spawn(move || {
            let mut contention_started: Option<Instant> = None;
            let mut last_report = Instant::now()
                .checked_sub(Duration::from_secs(60))
                .unwrap_or_else(Instant::now);

            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }

                match state.try_lock() {
                    Ok(_) => {
                        contention_started = None;
                    }
                    Err(_) => {
                        let started = contention_started.get_or_insert_with(Instant::now);
                        if started.elapsed() >= Duration::from_secs(5)
                            && last_report.elapsed() >= Duration::from_secs(5)
                        {
                            let record = LockContentionRecord {
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                operation_scope: operation_scope.clone(),
                                trading_mode: trading_mode.clone(),
                                live_dry_run,
                                pid,
                                process_start_epoch_ms,
                                boot_id: boot_id.clone(),
                                contention_age_ms: started.elapsed().as_millis(),
                                lock_intent: LOCK_INTENT
                                    .get_or_init(|| RwLock::new(String::new()))
                                    .read()
                                    .map(|current| current.clone())
                                    .unwrap_or_else(|_| "unavailable".to_string()),
                                thread_states: read_thread_states(),
                            };
                            append_record(&path, &record);
                            last_report = Instant::now();
                        }
                    }
                }
                thread::sleep(Duration::from_millis(250));
            }
        })
        .expect("AppState lock watchdog thread başlatılamadı");
}

fn operation_scope(trading_mode: &TradingMode, live_dry_run: bool) -> String {
    if matches!(trading_mode, TradingMode::Live) && !live_dry_run {
        "live".to_string()
    } else {
        "test".to_string()
    }
}

fn diagnostic_path(operation_scope: &str, pid: u32) -> PathBuf {
    if let Ok(path) = std::env::var("APP_STATE_LOCK_DIAGNOSTIC_PATH") {
        return add_run_identity(PathBuf::from(path), operation_scope, pid);
    }
    if let Ok(path) = std::env::var("HEARTBEAT_PATH") {
        if let Some(parent) = PathBuf::from(path).parent() {
            return parent.join(format!("app_state_lock.{operation_scope}.pid-{pid}.jsonl"));
        }
    }
    PathBuf::from(format!(
        "logs/app_state_lock.{operation_scope}.pid-{pid}.jsonl"
    ))
}

fn add_run_identity(path: PathBuf, operation_scope: &str, pid: u32) -> PathBuf {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return path;
    };
    let decorated = if let Some((stem, extension)) = file_name.rsplit_once('.') {
        format!("{stem}.{operation_scope}.pid-{pid}.{extension}")
    } else {
        format!("{file_name}.{operation_scope}.pid-{pid}")
    };
    path.with_file_name(decorated)
}

fn epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn read_boot_id() -> String {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|value| value.trim().to_string())
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unavailable".to_string())
}

fn read_thread_states() -> Vec<String> {
    let mut states = Vec::new();
    let Ok(entries) = fs::read_dir("/proc/self/task") else {
        return states;
    };
    for entry in entries.flatten() {
        let tid = entry.file_name().to_string_lossy().to_string();
        let comm = fs::read_to_string(entry.path().join("comm"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let wchan = fs::read_to_string(entry.path().join("wchan"))
            .unwrap_or_default()
            .trim()
            .to_string();
        states.push(format!("{tid}:{comm}:wchan={wchan}"));
    }
    states.sort();
    states
}

fn append_record(path: &PathBuf, record: &LockContentionRecord) {
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    if let Ok(json) = serde_json::to_string(record) {
        let _ = writeln!(file, "{json}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_scope_requires_real_live_execution() {
        assert_eq!(operation_scope(&TradingMode::Live, false), "live");
        assert_eq!(operation_scope(&TradingMode::Live, true), "test");
        assert_eq!(operation_scope(&TradingMode::Paper, false), "test");
        assert_eq!(operation_scope(&TradingMode::Backtest, false), "test");
    }

    #[test]
    fn diagnostic_path_contains_scope_and_pid() {
        let path = add_run_identity(PathBuf::from("logs/app_state_lock.jsonl"), "test", 1234);
        assert_eq!(
            path,
            PathBuf::from("logs/app_state_lock.test.pid-1234.jsonl")
        );
    }
}
