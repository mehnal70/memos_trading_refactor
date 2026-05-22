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
        // brain.live_strategy "Default"/"Auto"/"" iken motor her cycle'da rejime
        // göre otonom strateji seçiyor (process_symbol_cycle'da
        // StrategySelector::select_best). Tek statik isim yansıtmak yanıltıcı
        // olur; bridge.rs'le aynı normalleştirme uygulanır → operatör heartbeat
        // jsonl'da gerçek davranışı görür.
        let strategy = state.brain.live_strategy.read()
            .map(|s| {
                let raw = s.clone();
                if raw.eq_ignore_ascii_case("default")
                    || raw.eq_ignore_ascii_case("auto")
                    || raw.is_empty()
                {
                    "Otonom (rejime göre)".to_string()
                } else {
                    raw
                }
            })
            .unwrap_or_else(|_| "?".to_string());

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
        // Muhasebe denetim baseline'ı: tick-arası farkı görmek için.
        let mut prev_open: Option<usize> = None;
        let mut prev_closed: Option<usize> = None;
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

            // Muhasebe boşluğu denetimi: önceki tick'e göre açık pozisyon azaldıysa
            // closed_trades en az o kadar artmalı. Aksi halde "yetim kapanış" →
            // anomaly emit + log. İlk turda baseline kurulduğu için karşılaştırma yok.
            if let (Some(po), Some(pc)) = (prev_open, prev_closed) {
                let open_lost = po.saturating_sub(record.open_positions);
                let closed_gain = record.closed_trades.saturating_sub(pc);
                if open_lost > closed_gain {
                    let gap = open_lost - closed_gain;
                    if let Ok(mut st) = state.lock() {
                        if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                            use crate::robot::data_pipeline::{AnomalyKind, AnomalySeverity};
                            pipe.push_anomaly(
                                AnomalySeverity::Warning,
                                AnomalyKind::Custom,
                                format!(
                                    "Muhasebe boşluğu: {} pozisyon kapandı ama closed_trades sadece +{} arttı (yetim={})",
                                    open_lost, closed_gain, gap,
                                ),
                            );
                        }
                        st.push_log(format!(
                            "🧾 [ACCOUNTING-GAP] tick={} open {} → {} (kayıp {}), closed +{} (yetim {})",
                            tick, po, record.open_positions, open_lost, closed_gain, gap,
                        ));
                    }
                }
            }
            prev_open = Some(record.open_positions);
            prev_closed = Some(record.closed_trades);

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
    fn strategy_normalizes_default_and_auto_to_otonom() {
        let cfg = RoboticLoopConfig::default();
        let st = AppState::new(cfg);
        // Default boot değeri "AUTO" idi — snapshot "Otonom (rejime göre)" göstermeli.
        let rec = HeartbeatRecord::snapshot(&st, 0);
        assert_eq!(rec.strategy, "Otonom (rejime göre)",
            "AUTO sentinel normalize edilmedi: {}", rec.strategy);

        // Manuel "Default" da aynı şekilde normalize edilmeli (eski boot'lar).
        {
            let mut s = st.brain.live_strategy.write().unwrap();
            *s = "Default".to_string();
        }
        let rec2 = HeartbeatRecord::snapshot(&st, 1);
        assert_eq!(rec2.strategy, "Otonom (rejime göre)");

        // Açıkça bir strateji set edilirse aynen yansır.
        {
            let mut s = st.brain.live_strategy.write().unwrap();
            *s = "SUPERTREND".to_string();
        }
        let rec3 = HeartbeatRecord::snapshot(&st, 2);
        assert_eq!(rec3.strategy, "SUPERTREND");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn accounting_gap_detection_emits_anomaly() {
        use crate::core::model::PositionModel;
        let path = format!("/tmp/memos_hb_gap_{}.jsonl", std::process::id());
        let _ = std::fs::remove_file(&path);

        let cfg = RoboticLoopConfig::default();
        let st = Arc::new(Mutex::new(AppState::new(cfg)));

        // İlk tick: 2 açık pozisyon, 0 closed.
        {
            let s = st.lock().unwrap();
            let mut p = s.finance.live_positions.write().unwrap();
            for sym in ["AAA", "BBB"] {
                p.insert(sym.into(), PositionModel {
                    pos_id: format!("test-{}", sym),
                    symbol: sym.into(),
                    entry_price: 1.0,
                    current_price: 1.0,
                    qty: 1.0,
                    leverage: 1.0,
                    is_long: true,
                    trade_type: "scalp".into(),
                    opened_at: "2026-01-01T00:00:00Z".into(),
                    stop_loss: 0.0,
                    take_profit: 0.0,
                    trailing_stop: 0.0,
                    max_favorable_price: 1.0,
                    breakeven_activated: false,
                });
            }
        }

        // 1sn interval ile başlat → 1. tick'te baseline kurulur.
        spawn_heartbeat_writer(Arc::clone(&st), path.clone(), 1);
        tokio::time::sleep(Duration::from_millis(1200)).await;

        // Şimdi 1 pozisyon silelim ama closed_trades'a YAZMAYALIM (yetim simülasyonu).
        {
            let s = st.lock().unwrap();
            s.finance.live_positions.write().unwrap().remove("BBB");
        }
        // 2. tick'in muhasebe denetimini yakalamak için bekle.
        tokio::time::sleep(Duration::from_millis(1300)).await;

        // Stop ve doğrula.
        st.lock().unwrap().app_stop_signal.store(true, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(1100)).await;

        let pipe = st.lock().unwrap().guardian.live_pipeline.read().unwrap().anomalies.clone();
        let has_gap = pipe.iter().any(|a| a.message.contains("Muhasebe boşluğu"));
        assert!(has_gap, "Yetim kapanış anomaly emit edilmedi: {:?}",
            pipe.iter().map(|a| a.message.clone()).collect::<Vec<_>>());

        let _ = std::fs::remove_file(&path);
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
