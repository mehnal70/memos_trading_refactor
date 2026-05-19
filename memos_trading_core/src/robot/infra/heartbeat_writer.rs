// src/robot/infra/heartbeat_writer.rs
//
// Periyodik kalp atışı (heartbeat) dosyası: AppState'ten equity, açık/kapalı pozisyon
// sayısı, anomali sayısı, peak equity, drawdown, faz ve aktif strateji okunur ve
// `logs/heartbeat.jsonl` dosyasına JSONL formatında append edilir.
//
// Master engine'in RAM'deki "💓 Devriye #N | …" log'u program kapanınca uçar; bu
// yazıcı aynı metriği kalıcı diske düşürür — post-mortem, equity zaman serisi,
// faz/strateji geçişleri replay edilebilsin diye.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::robot::robotic_loop::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatRecord {
    pub timestamp: String,
    pub tick: u64,
    pub phase: String,
    pub equity: f64,
    pub peak_equity: f64,
    pub drawdown_pct: f64,
    pub open_positions: usize,
    pub closed_trades: usize,
    pub anomalies: usize,
    pub strategy: String,
    pub ml_confidence: f64,
}

impl HeartbeatRecord {
    /// AppState'ten anlık metrikleri okur. Mutex zaten dışarıda tutulduğu için
    /// `&AppState` ile çalışır — IO çağrı sahibinin sorumluluğunda.
    pub fn snapshot(state: &AppState, tick: u64) -> Self {
        let equity = state.finance.equity;
        let peak = state.finance.peak_equity;
        let drawdown_pct = if peak > 0.0 {
            ((peak - equity).max(0.0) / peak) * 100.0
        } else {
            0.0
        };
        let open_positions = state.finance.live_positions.read()
            .map(|p| p.len()).unwrap_or(0);
        let closed_trades = state.finance.live_closed_trades.read()
            .map(|t| t.len()).unwrap_or(0);
        let anomalies = state.guardian.live_pipeline.read()
            .map(|p| p.anomalies.len()).unwrap_or(0);
        let strategy = state.brain.live_strategy.read()
            .map(|s| s.clone()).unwrap_or_else(|_| "?".to_string());

        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            tick,
            phase: state.fleet.phase.clone(),
            equity,
            peak_equity: peak,
            drawdown_pct,
            open_positions,
            closed_trades,
            anomalies,
            strategy,
            ml_confidence: state.brain.ml_confidence,
        }
    }
}

/// Periyodik heartbeat yazıcısı. Default 60 sn — dakikalık çözünürlük.
pub fn spawn_heartbeat_writer(
    state: Arc<Mutex<AppState>>,
    path: String,
    interval_secs: u64,
) {
    tokio::spawn(async move {
        // Dosyanın ebeveyn dizinini hazırla (logs/ vb.)
        if let Some(parent) = Path::new(&path).parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }

        let interval = Duration::from_secs(interval_secs.max(1));
        let mut tick: u64 = 0;
        loop {
            // Çıkış kontrolü + snapshot üretimi tek kilit altında
            let record_opt = {
                let st = match state.lock() {
                    Ok(g) => g,
                    Err(_) => break,
                };
                if st.app_stop_signal.load(Ordering::Relaxed) {
                    None
                } else {
                    Some(HeartbeatRecord::snapshot(&st, tick))
                }
            };
            let record = match record_opt {
                Some(r) => r,
                None => break,
            };

            // JSONL append (her satır bir JSON). Hata olursa ilk turda push_log,
            // sonra spam'i durdur.
            let write_result = append_record(&path, &record);
            if let Err(e) = write_result {
                if tick == 0 {
                    if let Ok(mut st) = state.lock() {
                        st.push_log(format!(
                            "⚠️ heartbeat_writer: {} yazılamadı ({})",
                            path, e
                        ));
                    }
                }
            } else if tick == 0 {
                if let Ok(mut st) = state.lock() {
                    st.push_log(format!(
                        "💓 Heartbeat writer aktif: {} (her {}s)",
                        path, interval_secs
                    ));
                }
            }

            tick = tick.wrapping_add(1);
            tokio::time::sleep(interval).await;
        }
    });
}

fn append_record(path: &str, record: &HeartbeatRecord) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(record)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    writeln!(file, "{}", line)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::RoboticLoopConfig;

    #[test]
    fn snapshot_reads_default_state() {
        let cfg = RoboticLoopConfig::default();
        let st = AppState::new(cfg);
        let rec = HeartbeatRecord::snapshot(&st, 42);
        assert_eq!(rec.tick, 42);
        assert!(rec.equity > 0.0, "default equity capital olmalı");
        assert_eq!(rec.open_positions, 0);
        assert_eq!(rec.closed_trades, 0);
        assert_eq!(rec.anomalies, 0);
        assert!(rec.drawdown_pct.abs() < f64::EPSILON);
    }

    #[test]
    fn drawdown_pct_computes_when_equity_below_peak() {
        let cfg = RoboticLoopConfig::default();
        let mut st = AppState::new(cfg);
        st.finance.peak_equity = 10_000.0;
        st.finance.equity = 9_500.0;
        let rec = HeartbeatRecord::snapshot(&st, 0);
        // (10000 - 9500) / 10000 * 100 = 5.0
        assert!((rec.drawdown_pct - 5.0).abs() < 1e-9,
            "drawdown_pct beklenen 5.0, gerçek {}", rec.drawdown_pct);
    }

    #[test]
    fn append_record_writes_jsonl_line() {
        let path = format!("/tmp/memos_heartbeat_test_{}.jsonl", std::process::id());
        let _ = std::fs::remove_file(&path);

        let rec = HeartbeatRecord {
            timestamp: "2026-05-19T12:00:00Z".to_string(),
            tick: 1,
            phase: "Scanning".to_string(),
            equity: 10_000.0,
            peak_equity: 10_000.0,
            drawdown_pct: 0.0,
            open_positions: 0,
            closed_trades: 0,
            anomalies: 0,
            strategy: "MA_CROSSOVER".to_string(),
            ml_confidence: 0.0,
        };
        append_record(&path, &rec).unwrap();
        append_record(&path, &rec).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 2);
        let parsed: HeartbeatRecord = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed.tick, 1);
        assert_eq!(parsed.strategy, "MA_CROSSOVER");

        let _ = std::fs::remove_file(&path);
    }
}
