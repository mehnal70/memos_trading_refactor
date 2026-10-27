// src/robot/engines/master/infra_fleet.rs — Altyapı filosu: spawn_* arka plan task'ları
// Faz 1 modülerleştirme: master.rs'ten taşındı (davranış birebir korunur).
use super::*;
use chrono::{Datelike, Timelike};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::time::sleep;

fn main_loop_is_stalled(overdue_secs: u64, threshold_secs: u64) -> bool {
    overdue_secs > threshold_secs
}

fn bist_market_window_open(now_utc: chrono::DateTime<chrono::Utc>) -> bool {
    let turkey_time = now_utc + chrono::Duration::hours(3);
    if crate::robot::bist_calendar::is_non_trading_day(turkey_time.date_naive()) {
        return false;
    }
    let minutes = turkey_time.hour() * 60 + turkey_time.minute();
    let open = env_parse("BIST_MARKET_OPEN_MINUTES", 10 * 60u64) as u32;
    let close = env_parse("BIST_MARKET_CLOSE_MINUTES", 18 * 60u64) as u32;
    let grace = env_parse("BIST_MARKET_CLOSE_GRACE_MINS", 30u64) as u32;
    minutes >= open && minutes <= close.saturating_add(grace)
}

#[cfg(test)]
mod tests {
    use super::{bist_market_window_open, main_loop_is_stalled};

    #[test]
    fn main_loop_stall_requires_threshold_to_be_exceeded() {
        assert!(!main_loop_is_stalled(6, 15));
        assert!(!main_loop_is_stalled(15, 15));
        assert!(main_loop_is_stalled(16, 15));
    }

    #[test]
    fn bist_market_window_excludes_weekends_and_out_of_session_hours() {
        let saturday = chrono::DateTime::parse_from_rfc3339("2026-09-26T08:00:00Z")
            .expect("valid timestamp")
            .with_timezone(&chrono::Utc);
        let before_open = chrono::DateTime::parse_from_rfc3339("2026-09-28T06:59:00Z")
            .expect("valid timestamp")
            .with_timezone(&chrono::Utc);
        let in_session = chrono::DateTime::parse_from_rfc3339("2026-09-28T10:00:00Z")
            .expect("valid timestamp")
            .with_timezone(&chrono::Utc);
        let after_grace = chrono::DateTime::parse_from_rfc3339("2026-09-28T15:31:00Z")
            .expect("valid timestamp")
            .with_timezone(&chrono::Utc);

        assert!(!bist_market_window_open(saturday));
        assert!(!bist_market_window_open(before_open));
        assert!(bist_market_window_open(in_session));
        assert!(!bist_market_window_open(after_grace));
    }
}

struct InFlightTriggerGuard {
    trigger_name: String,
    in_flight: Arc<Mutex<std::collections::HashSet<String>>>,
}

fn spawn_autonomous_health_supervisor(state: Arc<Mutex<AppState>>) {
    let enabled = std::env::var("AUTONOMOUS_HEALTH_ENABLED")
        .map(|value| value != "false" && value != "0")
        .unwrap_or(true);
    let recovery_enabled = std::env::var("AUTONOMOUS_HEALTH_RECOVERY")
        .map(|value| value != "false" && value != "0")
        .unwrap_or(true);
    let interval_secs = env_parse("AUTONOMOUS_HEALTH_EVERY_SECS", 30u64).max(10);
    if !enabled {
        push_state_log(&state, "🩺 Autonomous health supervisor devre dışı".into());
        return;
    }

    tokio::spawn(async move {
        sleep(Duration::from_secs(interval_secs)).await;
        loop {
            let (stop, loop_age, mt5_healthy, stale_accounts) = match state.lock() {
                Ok(st) => {
                    let now = crate::core::time::now_epoch_secs();
                    let last_tick = st.fleet.last_loop_tick.load(Ordering::Relaxed);
                    let stale_accounts = st
                        .venue_runtime
                        .read()
                        .map(|runtimes| {
                            runtimes
                                .values()
                                .filter(|runtime| {
                                    runtime
                                        .account_snapshot
                                        .as_ref()
                                        .map(|account| {
                                            account.status
                                                != memos_venue_api::AccountSnapshotStatus::Fresh
                                                || chrono::Utc::now()
                                                    .signed_duration_since(account.timestamp)
                                                    .num_seconds()
                                                    > 600
                                        })
                                        .unwrap_or(true)
                                })
                                .count()
                        })
                        .unwrap_or(0);
                    (
                        st.app_stop_signal.load(Ordering::Relaxed),
                        now.saturating_sub(last_tick),
                        st.mt5_bridge_healthy.load(Ordering::Relaxed),
                        stale_accounts,
                    )
                }
                Err(_) => (true, u64::MAX, false, 0),
            };
            if stop {
                break;
            }

            let loop_ok = loop_age <= interval_secs.saturating_mul(2);
            if !loop_ok || !mt5_healthy || stale_accounts > 0 {
                push_state_log(
                    &state,
                    format!(
                        "🩺 Health: loop={}s mt5={} stale_accounts={} recovery={}",
                        loop_age,
                        if mt5_healthy { "ok" } else { "degraded" },
                        stale_accounts,
                        if recovery_enabled { "on" } else { "off" }
                    ),
                );
            }

            // Güvenli recovery: yalnız MT5 bridge yeniden-probe edilir; emir gönderilmez.
            if recovery_enabled && !mt5_healthy {
                let _ = Engine::gate_mt5_bridge_health(&state).await;
            }

            sleep(Duration::from_secs(interval_secs)).await;
        }
    });
}

fn spawn_bist_intraday_confirmation(state: Arc<Mutex<AppState>>) {
    let enabled = std::env::var("BIST_INTRADAY_ENABLED")
        .map(|v| {
            !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off"
            )
        })
        .unwrap_or(false);
    if !enabled {
        return;
    }
    let every_mins = env_parse("BIST_INTRADAY_EVERY_MINS", 15u64).max(1);
    tokio::spawn(async move {
        let config = match crate::robot::bist_trend::BistIntradayConfig::from_env() {
            Ok(config) => config,
            Err(error) => {
                push_state_log(
                    &state,
                    format!("❌ BIST intraday yapılandırması geçersiz: {error}"),
                );
                return;
            }
        };
        loop {
            let stopped = state
                .lock()
                .map(|st| st.app_stop_signal.load(Ordering::Relaxed))
                .unwrap_or(true);
            if stopped {
                break;
            }
            match crate::robot::bist_trend::update_and_confirm_intraday(&config).await {
                Ok(results) => {
                    let alerts = results
                        .iter()
                        .filter(|r| r.status == "CONFIRMED" || r.status == "WARNING")
                        .collect::<Vec<_>>();
                    let skipped = results
                        .iter()
                        .filter(|r| r.status == "SKIPPED")
                        .collect::<Vec<_>>();
                    if !skipped.is_empty() {
                        push_state_log(
                            &state,
                            format!(
                                "⚠️ BIST intraday: {} sembol/interval atlandı; ayrıntı: {}",
                                skipped.len(),
                                config.report_path
                            ),
                        );
                    }
                    if !alerts.is_empty() {
                        push_state_log(
                            &state,
                            format!(
                                "📈 BIST intraday confirmation: {} bildirim → {}",
                                alerts.len(),
                                config.report_path
                            ),
                        );
                        if let Some(notifier) =
                            crate::robot::infra::telegram_notifier::TelegramNotifier::from_env()
                        {
                            let mut message = format!(
                                "<b>📈 BIST Intraday Confirmation</b>\n{} bildirim\n",
                                alerts.len()
                            );
                            for item in alerts {
                                message.push_str(&format!(
                                    "\n<b>{}</b> {} [{}] · {:.2}\n{}\n",
                                    item.status,
                                    item.symbol,
                                    item.interval,
                                    item.close,
                                    item.reason
                                ));
                            }
                            if let Err(error) = notifier.send_raw(&message).await {
                                push_state_log(
                                    &state,
                                    format!("⚠️ BIST intraday Telegram gönderilemedi: {error}"),
                                );
                            }
                        }
                    }
                }
                Err(error) => {
                    push_state_log(&state, format!("❌ BIST intraday başarısız: {error}"))
                }
            }
            sleep(Duration::from_secs(every_mins * 60)).await;
        }
    });
}
fn spawn_bist_trend_pipeline(state: Arc<Mutex<AppState>>) {
    let enabled = std::env::var("BIST_TREND_ENABLED")
        .map(|v| {
            !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off"
            )
        })
        .unwrap_or(false);
    if !enabled {
        return;
    }

    let every_mins = env_parse("BIST_TREND_EVERY_MINS", 1440u64).max(1);
    tokio::spawn(async move {
        let config = match crate::robot::bist_trend::BistTrendConfig::from_env() {
            Ok(config) => config,
            Err(error) => {
                push_state_log(
                    &state,
                    format!("❌ BIST trend yapılandırması geçersiz: {error}"),
                );
                return;
            }
        };
        loop {
            let stopped = state
                .lock()
                .map(|st| st.app_stop_signal.load(Ordering::Relaxed))
                .unwrap_or(true);
            if stopped {
                break;
            }
            if !bist_market_window_open(chrono::Utc::now()) {
                sleep(Duration::from_secs(every_mins * 60)).await;
                continue;
            }
            match crate::robot::bist_trend::update_and_scan(&config).await {
                Ok(decisions) => {
                    if let Ok(health) =
                        crate::robot::bist_trend::read_health_status(&config.status_path)
                    {
                        if matches!(health.status.as_str(), "DEGRADED" | "STALE" | "UNAVAILABLE") {
                            if let Some(notifier) =
                                crate::robot::infra::telegram_notifier::TelegramNotifier::from_env()
                            {
                                notifier.notify(
                                    "BIST-DATA-HEALTH",
                                    crate::robot::infra::telegram_notifier::Severity::Warning,
                                    &format!(
                                        "<b>BIST veri sağlığı: {}</b>\n{}/{} indirme · son candle: {}{}",
                                        health.status,
                                        health.successful_download_count,
                                        health.expected_download_count,
                                        health.latest_candle_timestamp
                                            .map(|ts| ts.to_rfc3339())
                                            .unwrap_or_else(|| "yok".into()),
                                        health
                                            .errors
                                            .first()
                                            .map(|e| format!("\n{}", e))
                                            .unwrap_or_default()
                                    ),
                                );
                            }
                        }
                    }
                    let opportunity_path = std::env::var("BIST_OPPORTUNITY_STATE_PATH")
                        .unwrap_or_else(|_| "data/bist_opportunities.json".into());
                    let mut lifecycle_decisions = decisions;
                    let actionable = match crate::robot::bist_trend::update_opportunity_lifecycle(
                        &opportunity_path,
                        &mut lifecycle_decisions,
                    ) {
                        Ok(actionable) => actionable,
                        Err(error) => {
                            push_state_log(
                                &state,
                                format!("❌ BIST opportunity lifecycle başarısız: {error}"),
                            );
                            Vec::new()
                        }
                    };
                    push_state_log(
                        &state,
                        format!(
                            "📊 BIST trend pipeline tamamlandı: {} karar, {} yeni alarm → {}",
                            lifecycle_decisions.len(),
                            actionable.len(),
                            config.report_path
                        ),
                    );
                    if !actionable.is_empty() {
                        if let Some(notifier) =
                            crate::robot::infra::telegram_notifier::TelegramNotifier::from_env()
                        {
                            let mut message = format!(
                                "<b>📊 BIST Trend Alarmı</b>\n{} karar · {} alarm\n",
                                lifecycle_decisions.len(),
                                actionable.len()
                            );
                            for decision in actionable {
                                message.push_str(&format!(
                                    "\n<b>{}</b> {} [{}] · kapanış {:.2}\n{}",
                                    decision.action,
                                    decision.symbol,
                                    decision.interval,
                                    decision.close,
                                    decision.reason
                                ));
                                if let Some(entry) = decision.suggested_entry {
                                    message.push_str(&format!(
                                        "\nAlım/kırılım referansı: {:.2}",
                                        entry
                                    ));
                                }
                                if let Some(pyramid) = &decision.pyramid {
                                    message.push_str(&format!(
                                        "\nKademeli plan: ilk {:.2} → {:.2} → {:.2} · koruyucu stop {:.2}",
                                        pyramid.initial_entry,
                                        pyramid.add_after_atr[0],
                                        pyramid.add_after_atr[1],
                                        pyramid.protective_stop
                                    ));
                                }
                                if let Some(exit) = decision.exit_reference {
                                    message.push_str(&format!(
                                        "\nSatış/çıkış referansı: {:.2} · {}",
                                        exit,
                                        decision.exit_condition.as_deref().unwrap_or("koşul")
                                    ));
                                }
                                message.push_str(&format!("\n{}\n", decision.manual_note));
                            }
                            if let Err(error) = notifier.send_raw(&message).await {
                                push_state_log(
                                    &state,
                                    format!("⚠️ BIST Telegram alarmı gönderilemedi: {error}"),
                                );
                            }
                        }
                    }
                }
                Err(error) => {
                    push_state_log(&state, format!("❌ BIST trend pipeline başarısız: {error}"));
                }
            }
            sleep(Duration::from_secs(every_mins * 60)).await;
        }
    });
}

impl InFlightTriggerGuard {
    fn new(trigger_name: String, in_flight: Arc<Mutex<std::collections::HashSet<String>>>) -> Self {
        Self {
            trigger_name,
            in_flight,
        }
    }
}

impl Drop for InFlightTriggerGuard {
    fn drop(&mut self) {
        match self.in_flight.lock() {
            Ok(mut set) => {
                set.remove(&self.trigger_name);
            }
            Err(poison) => {
                let mut set = poison.into_inner();
                set.remove(&self.trigger_name);
            }
        }
    }
}

impl Engine {
    fn read_last_lines(path: &str, max_lines: usize) -> Vec<String> {
        use std::collections::VecDeque;
        use std::io::BufRead;

        if max_lines == 0 {
            return Vec::new();
        }
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        let reader = std::io::BufReader::new(file);
        let mut ring: VecDeque<String> = VecDeque::with_capacity(max_lines.min(4096));
        for line in reader.lines().map_while(Result::ok) {
            if ring.len() >= max_lines {
                ring.pop_front();
            }
            ring.push_back(line);
        }
        ring.into_iter().collect()
    }
    fn run_periodic_log_scan(state: &Arc<Mutex<AppState>>, recovery_enabled: bool) {
        let now_epoch = crate::core::time::now_epoch_secs();
        let heartbeat_path =
            std::env::var("HEARTBEAT_PATH").unwrap_or_else(|_| "logs/heartbeat.jsonl".to_string());
        let trading_path = std::env::var("TRADING_LOG_PATH")
            .unwrap_or_else(|_| "logs/robotic_trading.log".to_string());

        let hb_lines = Self::read_last_lines(&heartbeat_path, 240);
        let tr_lines = Self::read_last_lines(&trading_path, 1600);

        let mut hb_nonzero_anomaly = 0u64;
        let mut hb_risk_breach = 0u64;
        let mut hb_api_error = 0u64;
        for line in &hb_lines {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                if v.get("anomalies").and_then(|x| x.as_u64()).unwrap_or(0) > 0 {
                    hb_nonzero_anomaly += 1;
                }
                if let Some(m) = v.get("anomalies_by_kind") {
                    hb_risk_breach += m.get("RiskBreach").and_then(|x| x.as_u64()).unwrap_or(0);
                    hb_api_error += m.get("ApiError").and_then(|x| x.as_u64()).unwrap_or(0);
                }
            }
        }

        let mut trading_negative_hits = 0u64;
        let mut trading_recovering_hits = 0u64;
        for l in &tr_lines {
            let s = l.to_ascii_lowercase();
            if s.contains("panic")
                || s.contains("\u{274c}")
                || s.contains(" başarısız")
                || s.contains(" failed")
                || s.contains("join hatası")
            {
                trading_negative_hits += 1;
            }
            if s.contains("recovering") || s.contains("anomali onarımı") {
                trading_recovering_hits += 1;
            }
        }

        let adverse = hb_nonzero_anomaly > 0 || hb_risk_breach > 0 || trading_negative_hits >= 3;

        if let Ok(mut st) = state.lock() {
            Self::inc_counter(&mut st, "logscan.runs");
            st.fleet
                .autonomy_counters
                .insert("logscan.last_run_epoch".to_string(), now_epoch);
            st.push_log(format!(
                "🩺 LogScan: hb_anom={} risk_breach={} api_err={} log_neg={} recovering={} [{}]",
                hb_nonzero_anomaly,
                hb_risk_breach,
                hb_api_error,
                trading_negative_hits,
                trading_recovering_hits,
                if adverse { "olumsuz" } else { "normal" },
            ));

            if adverse {
                Self::inc_counter(&mut st, "logscan.adverse");
                st.fleet
                    .autonomy_counters
                    .insert("logscan.last_adverse_epoch".to_string(), now_epoch);
                let entry = format!(
                    "[{}] logscan: adverse hb_anom={} risk={} api={} neg={} rec={}",
                    chrono::Local::now().format("%H:%M:%S"),
                    hb_nonzero_anomaly,
                    hb_risk_breach,
                    hb_api_error,
                    trading_negative_hits,
                    trading_recovering_hits,
                );
                st.guardian.repair_log.push_back(entry);
                while st.guardian.repair_log.len() > 100 {
                    st.guardian.repair_log.pop_front();
                }
            }

            if adverse && recovery_enabled {
                let mut actions: Vec<String> = Vec::new();

                let queue = |trigger: &str, st: &mut AppState, actions: &mut Vec<String>| {
                    if let Some(t) = st.fleet.triggers.get(trigger) {
                        if !t.swap(true, Ordering::AcqRel) {
                            Self::inc_counter(st, &format!("trigger.logscan.{}.enqueued", trigger));
                            actions.push(format!("{}:enqueued", trigger));
                        } else {
                            Self::inc_counter(
                                st,
                                &format!("trigger.logscan.{}.coalesced", trigger),
                            );
                            actions.push(format!("{}:coalesced", trigger));
                        }
                    }
                };

                if hb_nonzero_anomaly > 0 || hb_api_error > 0 {
                    queue("download", &mut st, &mut actions);
                    queue("ml", &mut st, &mut actions);
                }
                if hb_risk_breach > 0 || trading_negative_hits >= 10 {
                    queue("backtest", &mut st, &mut actions);
                }

                if actions.is_empty() {
                    st.push_log(
                        "🛠️ LogScan recovery: adverse tespit edildi ama uygun trigger bulunamadı"
                            .into(),
                    );
                } else {
                    st.fleet
                        .autonomy_counters
                        .insert("logscan.last_recovery_epoch".to_string(), now_epoch);
                    st.push_log(format!("🛠️ LogScan recovery: {}", actions.join(" · "),));
                }
            }
        }
    }
    fn inc_counter(st: &mut AppState, key: &str) {
        *st.fleet
            .autonomy_counters
            .entry(key.to_string())
            .or_insert(0) += 1;
    }

    pub(crate) async fn spawn_infrastructure_fleet(state: Arc<Mutex<AppState>>) {
        push_state_log(&state, "⚡ Altyapı filosu sevk edildi: snapshot(5s) · heartbeat-file(60s) · heartbeat(1s) · phase(2s) · price-poll(5s) · trigger(250ms) · scheduler(60s) · psync(30s) · ws-user-data · balance-sync(5dk)".into());
        spawn_autonomous_health_supervisor(Arc::clone(&state));
        spawn_bist_trend_pipeline(Arc::clone(&state));
        spawn_bist_intraday_confirmation(Arc::clone(&state));

        // ── 🧠 OTONOM REFLEKS ENTEGRASYONU: Statik profil dosyalarını tamamen devre dışı bırakan katman ──
        if let Ok(mut st) = state.lock() {
            if let Some(current_exchange) =
                crate::core::types::Exchange::from_token(&st.config.exchange)
            {
                if let Some(active_venue) = st.venue_registry.get(current_exchange) {
                    let otonom_reflexes =
                        crate::robot::config_engine::OtonomConfigEngine::evaluate_market_reflexes(
                            active_venue,
                        );

                    // KESİN ÇÖZÜM: Parametrelerin kilit alanlarını ayrı değişkenlerde hesaplayıp,
                    // kilitleme işlemini izole bir iç blokta gerçekleştiriyoruz.
                    let carry_enabled = otonom_reflexes
                        .get("CARRY_LIVE_ENABLED")
                        .map(|v| v == "1")
                        .unwrap_or(false);
                    let blend_enabled = otonom_reflexes
                        .get("BLEND_LIVE_ENABLED")
                        .map(|v| v == "1")
                        .unwrap_or(false);

                    {
                        // Kilit (RwLockWriteGuard) sadece bu parantezlerin içinde yaşar ve kapanış parantezinde derhal drop olur.
                        if let Ok(mut params) = st.brain.parameters.write() {
                            params.carry_live.enabled = carry_enabled;
                            params.blend_live.enabled = blend_enabled;
                        }
                    } // params ve geçici kilit nesneleri BURADA biter (destructor tetiklenir).

                    // Artık st referansı tamamen özgürdür, push_log çakışma yaratmadan güvenle çalışır.
                    st.push_log(format!(
                        "🤖 Otonom Refleks Keşfi Tamamlandı ⇒ Carry Mod: {} · Blend Mod: {}",
                        if carry_enabled { "AKTİF" } else { "KAPALI" },
                        if blend_enabled { "AKTİF" } else { "KAPALI" }
                    ));
                }
            }
        }
        // ───────────────────────────────────────────────────────────────
        // ── 🌾 KODUN TAM YERİ BURASIDIR: AppState kilidi açıldıktan hemen sonra asenkron ateşleme ──
        let harvester_state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(e) =
                crate::robot::elite_harvester::EliteSymbolHarvester::execute_harvest_and_inject(
                    &harvester_state,
                )
                .await
            {
                log::warn!("⚠️ Otonom kalburüstü sembol hasat motoru başarısız: {}", e);
            }
        });
        // ───────────────────────────────────────────────────────────────────────────────────────────

        let snapshot_disabled = std::env::var("SNAPSHOT_WRITER_DISABLE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if !snapshot_disabled {
            let snapshot_path = std::env::var("MISSION_CONTROL_SNAPSHOT_PATH")
                .unwrap_or_else(|_| "data/mission_control.json".to_string());
            let snapshot_secs: u64 = env_parse("MISSION_CONTROL_SNAPSHOT_SECS", 5u64).max(1);
            crate::robot::infra::snapshot_writer::spawn_snapshot_writer(
                Arc::clone(&state),
                snapshot_path,
                snapshot_secs,
            );
        } else if let Ok(mut st) = state.lock() {
            st.push_log("📤 Snapshot writer devre dışı (SNAPSHOT_WRITER_DISABLE)".into());
        }

        let heartbeat_disabled = std::env::var("HEARTBEAT_WRITER_DISABLE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if !heartbeat_disabled {
            let heartbeat_path = std::env::var("HEARTBEAT_PATH")
                .unwrap_or_else(|_| "logs/heartbeat.jsonl".to_string());
            let heartbeat_secs: u64 = env_parse("HEARTBEAT_SECS", 60u64).max(1);
            crate::robot::infra::heartbeat_writer::spawn_heartbeat_writer(
                Arc::clone(&state),
                heartbeat_path,
                heartbeat_secs,
            );
        } else if let Ok(mut st) = state.lock() {
            st.push_log("💓 Heartbeat writer devre dışı (HEARTBEAT_WRITER_DISABLE)".into());
        }

        let st_hb = Arc::clone(&state);
        tokio::spawn(async move {
            use crate::robot::data_pipeline::{AnomalyKind, AnomalySeverity, StepStatus};
            let stall_threshold_secs: u64 = env_parse("MAIN_LOOP_STALL_SECS", 15u64).max(1);
            let mut stall_reported = false;
            loop {
                let now_epoch = crate::core::time::now_epoch_secs();

                let mut overdue_anomaly_msg: Option<String> = None;
                let mut loop_status: Option<StepStatus> = None;
                let mut loop_last_tick: u64 = 0;
                let mut loop_overdue: u64 = 0;

                let stop = {
                    let st = match st_hb.lock() {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    if st.app_stop_signal.load(Ordering::Relaxed) {
                        true
                    } else {
                        let last_tick = st.fleet.last_loop_tick.load(Ordering::Relaxed);
                        let overdue = if last_tick == 0 {
                            0
                        } else {
                            now_epoch.saturating_sub(last_tick).saturating_sub(1)
                        };

                        if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                            let status = if last_tick == 0 {
                                StepStatus::Idle
                            } else {
                                StepStatus::Running
                            };
                            pipe.record_step("main_loop", status, last_tick, overdue);
                            loop_status = Some(status);
                            loop_last_tick = last_tick;
                            loop_overdue = overdue;
                            let stalled = main_loop_is_stalled(overdue, stall_threshold_secs);
                            if stalled && !stall_reported {
                                let msg = format!("main_loop gecikti: +{}s", overdue);
                                pipe.push_anomaly(
                                    AnomalySeverity::Warning,
                                    AnomalyKind::DataStall,
                                    msg.clone(),
                                );
                                overdue_anomaly_msg = Some(msg);
                            } else if !stalled {
                                pipe.clear_warning_kind(AnomalyKind::DataStall);
                            }
                            stall_reported = stalled;
                        }
                        false
                    }
                };
                if stop {
                    break;
                }
                if let Some(status) = loop_status {
                    emit_pipeline_step_event(
                        &st_hb,
                        "main_loop",
                        status,
                        loop_last_tick,
                        loop_overdue,
                    );
                }
                if let Some(msg) = overdue_anomaly_msg {
                    emit_pipeline_anomaly_event(
                        &st_hb,
                        AnomalySeverity::Warning,
                        AnomalyKind::DataStall,
                        &msg,
                    );
                }
                sleep(Duration::from_secs(1)).await;
            }
        });
        let st_px = Arc::clone(&state);
        tokio::spawn(async move {
            use crate::robot::data_pipeline::{AnomalyKind, AnomalySeverity, StepStatus};
            use crate::robot::venue::MarketData;

            if env_parse("PRICE_POLL_ENABLED", 1_u64) == 0 {
                push_state_log(
                    &st_px,
                    "📊 Price-poll devre dışı; günlük signal-only import korunuyor".into(),
                );
                return;
            }

            let started_at = std::time::Instant::now();
            let poll_secs = env_parse("PRICE_POLL_SECS", 5_u64).max(5);
            let market_data_timeout_secs =
                env_parse("VENUE_MARKET_DATA_TIMEOUT_SECS", 15_u64).max(1);
            let mut first_summary_pending = true;
            let mut last_error_summary_at: u64 = 0;
            let mut last_error_summary_msg: String = String::new();

            loop {
                let (symbols, interval, registry, stop, lock_diagnostic) = {
                    let st = match st_px.lock() {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    if st.app_stop_signal.load(Ordering::Relaxed) {
                        (
                            vec![],
                            String::new(),
                            Arc::clone(&st.venue_registry),
                            true,
                            Arc::clone(&st.runtime_lock_diagnostic),
                        )
                    } else {
                        let tuning = Arc::clone(&st.tuning);
                        let eligible = |s: &str| {
                            tuning.symbol_eligible_for_live(s)
                                && crate::robot::config_engine::OtonomConfigEngine::symbol_allowed(
                                    s,
                                )
                        };

                        let mut syms: Vec<String> = vec![];
                        if eligible(&st.config.symbol) {
                            syms.push(st.config.symbol.clone());
                        }
                        if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                            for w in orch.get_worker_status() {
                                if !eligible(&w.symbol) {
                                    continue;
                                }
                                if !syms.contains(&w.symbol) {
                                    syms.push(w.symbol);
                                }
                            }
                        }
                        if let Ok(positions) = st.finance.live_positions.read() {
                            for sym in positions.keys() {
                                if !eligible(sym) {
                                    continue;
                                }
                                if !syms.contains(sym) {
                                    syms.push(sym.clone());
                                }
                            }
                        }
                        if let Ok(raw) = std::env::var("AUTO_IBKR_SYMBOLS") {
                            for sym in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                                if !syms.contains(&sym.to_string()) {
                                    syms.push(sym.to_string());
                                }
                            }
                        }
                        if let Ok(raw) = std::env::var("AUTO_BINANCE_SYMBOLS") {
                            for sym in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                                if !syms.contains(&sym.to_string()) {
                                    syms.push(sym.to_string());
                                }
                            }
                        }
                        if let Ok(raw) = std::env::var("AUTO_BIST_SYMBOLS") {
                            for sym in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                                if !syms.contains(&sym.to_string()) {
                                    syms.push(sym.to_string());
                                }
                            }
                        }
                        (
                            syms,
                            st.config.interval.clone(),
                            Arc::clone(&st.venue_registry),
                            false,
                            Arc::clone(&st.runtime_lock_diagnostic),
                        )
                    }
                };
                if stop {
                    break;
                }

                let mut new_prices: Vec<(String, f64)> = Vec::with_capacity(symbols.len());
                let mut new_ticks_meta: Vec<(String, f64, String, String)> =
                    Vec::with_capacity(symbols.len());
                let mut errors: Vec<(String, String)> = Vec::new();

                let interval_secs =
                    crate::robot::data_pipeline::DataNormalizer::parse_interval(&interval) as i64;
                let base_max_age: i64 = env_parse("PRICE_POLL_MAX_CANDLE_AGE_SECS", 300);
                let max_candle_age: i64 = effective_max_candle_age(base_max_age, interval_secs);
                let mut stale_skipped: Vec<String> = Vec::new();

                for sym in &symbols {
                    if sym.is_empty() {
                        continue;
                    }
                    let Some((venue, bare)) = registry.route(sym) else {
                        errors.push((sym.clone(), "venue bulunamadı (registry boş?)".to_string()));
                        continue;
                    };
                    // Market Gözetimi için canlı quote'u tercih et; özellikle IBKR'de
                    // mum tarihi/formatı geçici olarak çözülemezse ekran 0.0'a düşmemeli.
                    if let Ok(mut diagnostic) = lock_diagnostic.write() {
                        diagnostic.owner = "price_poll".to_string();
                        diagnostic.phase = format!("book_ticker:{sym}");
                        diagnostic.started_epoch = crate::core::time::now_epoch_secs() as i64;
                    }
                    let quote_venue = Arc::clone(venue);
                    let quote_symbol = bare.to_string();
                    let quote = tokio::time::timeout(
                        Duration::from_secs(market_data_timeout_secs),
                        quote_venue.book_ticker(&quote_symbol),
                    )
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .and_then(|(bid, ask)| {
                        let mid = (bid + ask) / 2.0;
                        mid.is_finite().then_some(mid).filter(|price| *price > 0.0)
                    });
                    let candle_venue = Arc::clone(venue);
                    let candle_symbol = bare.to_string();
                    let candle_interval = interval.clone();
                    let skip_candle_poll = venue.exchange()
                        == crate::core::types::Exchange::Binance
                        && quote.is_some();
                    if let Ok(mut diagnostic) = lock_diagnostic.write() {
                        diagnostic.owner = "price_poll".to_string();
                        diagnostic.phase = format!("fetch_candles:{sym}");
                        diagnostic.started_epoch = crate::core::time::now_epoch_secs() as i64;
                    }
                    let candles = if skip_candle_poll {
                        Ok(Vec::new())
                    } else {
                        tokio::time::timeout(
                            Duration::from_secs(market_data_timeout_secs),
                            candle_venue.fetch_candles(&candle_symbol, &candle_interval, 1),
                        )
                        .await
                        .map_err(|_| format!("market-data timeout ({}s)", market_data_timeout_secs))
                        .and_then(|result| result.map_err(|e| e.to_string()))
                    };
                    match candles {
                        Ok(candles) => {
                            delisted_record_success(sym);
                            if let Some(last) = candles.last() {
                                if last.close <= 0.0 && quote.is_none() {
                                    continue;
                                }
                                let age = (chrono::Utc::now() - last.timestamp).num_seconds();
                                if quote.is_none() && max_candle_age > 0 && age > max_candle_age {
                                    stale_skipped.push(format!("{}({}s)", sym, age));
                                    continue;
                                }
                                let price = quote.unwrap_or(last.close);
                                new_prices.push((sym.clone(), price));
                                new_ticks_meta.push((
                                    sym.clone(),
                                    price,
                                    venue.exchange().as_str().to_string(),
                                    venue.market().as_str().to_string(),
                                ));
                            } else if let Some(price) = quote {
                                new_prices.push((sym.clone(), price));
                                new_ticks_meta.push((
                                    sym.clone(),
                                    price,
                                    venue.exchange().as_str().to_string(),
                                    venue.market().as_str().to_string(),
                                ));
                            }
                        }
                        Err(e) => {
                            if let Some(price) = quote {
                                new_prices.push((sym.clone(), price));
                                new_ticks_meta.push((
                                    sym.clone(),
                                    price,
                                    venue.exchange().as_str().to_string(),
                                    venue.market().as_str().to_string(),
                                ));
                            } else {
                                errors.push((sym.clone(), e));
                            }
                        }
                    }
                }

                if !stale_skipped.is_empty()
                    && log_throttle_should_emit("price_poll", "stale_candle", 300)
                {
                    log::warn!(
                        "price_poll: {} sembol için stale candle (>{}sn) — live_price güncellenmedi: {}",
                        stale_skipped.len(), max_candle_age, stale_skipped.join(","),
                    );
                }
                let dl_threshold = delisted_detection_threshold();
                let mut to_purge: Vec<(String, u32)> = Vec::new();
                if dl_threshold > 0 {
                    for (sym, e) in &errors {
                        let Some((venue, _)) = registry.route(sym) else {
                            continue;
                        };
                        if !venue.is_delisting_error(e) {
                            continue;
                        }
                        let n = delisted_record_failure(sym);
                        if n >= dl_threshold {
                            to_purge.push((sym.clone(), n));
                        }
                    }
                }

                let now_secs = started_at.elapsed().as_secs();
                let summary_msg: Option<String> = if first_summary_pending && !new_prices.is_empty()
                {
                    first_summary_pending = false;
                    Some(format!(
                        "📡 Price-poll çalışıyor: {} sembolden ilk fiyatlar alındı ({})",
                        new_prices.len(),
                        new_prices
                            .iter()
                            .map(|(s, p)| format!("{}={:.2}", s, p))
                            .take(3)
                            .collect::<Vec<_>>()
                            .join(" · "),
                    ))
                } else if !errors.is_empty()
                    && now_secs.saturating_sub(last_error_summary_at) >= 300
                {
                    let msg = format!(
                        "⚠️ Price-poll: {}/{} sembolde hata. Örn: {}",
                        errors.len(),
                        symbols.len(),
                        errors
                            .first()
                            .map(|(s, e)| format!("{}: {}", s, e))
                            .unwrap_or_default(),
                    );
                    if msg == last_error_summary_msg {
                        None
                    } else {
                        last_error_summary_at = now_secs;
                        last_error_summary_msg = msg.clone();
                        Some(msg)
                    }
                } else {
                    None
                };

                let now_epoch_secs: u64 = crate::core::time::now_epoch_secs();
                let price_poll_status = if errors.is_empty() {
                    StepStatus::Done
                } else {
                    StepStatus::Degraded
                };
                let mut closed_5m_for_db: Vec<(String, String, Candle)> =
                    Vec::with_capacity(symbols.len());
                let mut closed_15m_for_db: Vec<(String, String, Candle)> =
                    Vec::with_capacity(symbols.len());
                let mut flush_db_path: Option<(String, String)> = None;
                let mut closed_5m_count: u64 = 0;
                let mut closed_15m_count: u64 = 0;
                if let Ok(mut st) = st_px.lock() {
                    flush_db_path = Some((
                        st.config.db_path.clone(),
                        st.config.venue_profile.as_str().to_string(),
                    ));
                    if let Ok(mut prices) = st.fleet.live_price.write() {
                        for (sym, px) in &new_prices {
                            prices.insert(sym.clone(), *px);
                        }
                    }
                    if let Ok(mut updated_at) = st.fleet.live_price_updated_at.write() {
                        let now = chrono::Utc::now();
                        for (sym, px) in &new_prices {
                            if px.is_finite() && *px > 0.0 {
                                updated_at.insert(sym.clone(), now);
                            }
                        }
                    }
                    if let Ok(mut accs) = st.fleet.opportunity_tick5m.write() {
                        let now = chrono::Utc::now();
                        let bucket = now.timestamp() - (now.timestamp() % 300);
                        for (sym, px, ex, mk) in &new_ticks_meta {
                            if *px <= 0.0 {
                                continue;
                            }
                            let entry = accs.entry(sym.clone()).or_insert_with(|| {
                                crate::robot::robotic_loop::TickBarAccumulator {
                                    bucket_start_epoch: bucket,
                                    open: *px,
                                    high: *px,
                                    low: *px,
                                    close: *px,
                                }
                            });
                            if entry.bucket_start_epoch == bucket {
                                entry.close = *px;
                                if *px > entry.high {
                                    entry.high = *px;
                                }
                                if *px < entry.low {
                                    entry.low = *px;
                                }
                            } else {
                                if let Some(ts) =
                                    chrono::DateTime::from_timestamp(entry.bucket_start_epoch, 0)
                                {
                                    let closed = Candle {
                                        timestamp: ts,
                                        open: entry.open,
                                        high: entry.high,
                                        low: entry.low,
                                        close: entry.close,
                                        volume: 0.0,
                                        symbol: sym.clone(),
                                        interval: "5m".to_string(),
                                    };
                                    st.fleet
                                        .opportunity_cache
                                        .push_synthesized(vec![closed.clone()]);
                                    closed_5m_for_db.push((ex.clone(), mk.clone(), closed));
                                    closed_5m_count += 1;
                                }
                                *entry = crate::robot::robotic_loop::TickBarAccumulator {
                                    bucket_start_epoch: bucket,
                                    open: *px,
                                    high: *px,
                                    low: *px,
                                    close: *px,
                                };
                            }
                        }
                    }
                    if let Ok(mut accs) = st.fleet.opportunity_tick15m.write() {
                        let now = chrono::Utc::now();
                        let bucket = now.timestamp() - (now.timestamp() % 900);
                        for (sym, px, ex, mk) in &new_ticks_meta {
                            if *px <= 0.0 {
                                continue;
                            }
                            let entry = accs.entry(sym.clone()).or_insert_with(|| {
                                crate::robot::robotic_loop::TickBarAccumulator {
                                    bucket_start_epoch: bucket,
                                    open: *px,
                                    high: *px,
                                    low: *px,
                                    close: *px,
                                }
                            });
                            if entry.bucket_start_epoch == bucket {
                                entry.close = *px;
                                if *px > entry.high {
                                    entry.high = *px;
                                }
                                if *px < entry.low {
                                    entry.low = *px;
                                }
                            } else {
                                if let Some(ts) =
                                    chrono::DateTime::from_timestamp(entry.bucket_start_epoch, 0)
                                {
                                    let closed = Candle {
                                        timestamp: ts,
                                        open: entry.open,
                                        high: entry.high,
                                        low: entry.low,
                                        close: entry.close,
                                        volume: 0.0,
                                        symbol: sym.clone(),
                                        interval: "15m".to_string(),
                                    };
                                    st.fleet
                                        .opportunity_cache
                                        .push_synthesized(vec![closed.clone()]);
                                    closed_15m_for_db.push((ex.clone(), mk.clone(), closed));
                                    closed_15m_count += 1;
                                }
                                *entry = crate::robot::robotic_loop::TickBarAccumulator {
                                    bucket_start_epoch: bucket,
                                    open: *px,
                                    high: *px,
                                    low: *px,
                                    close: *px,
                                };
                            }
                        }
                    }
                    if closed_5m_count > 0 {
                        Self::inc_counter(&mut st, "price_poll.tick5m.closed_batches");
                        *st.fleet
                            .autonomy_counters
                            .entry("price_poll.tick5m.closed_rows".to_string())
                            .or_insert(0) += closed_5m_count;
                        if log_throttle_should_emit("price_poll", "tick5m_closed", 60) {
                            st.push_log(format!(
                                "🧩 Hibrit 5m: {} kapanan bar sentezlendi (cache+DB kuyruğu)",
                                closed_5m_count,
                            ));
                        }
                    }
                    if closed_15m_count > 0 {
                        Self::inc_counter(&mut st, "price_poll.tick15m.closed_batches");
                        *st.fleet
                            .autonomy_counters
                            .entry("price_poll.tick15m.closed_rows".to_string())
                            .or_insert(0) += closed_15m_count;
                        if log_throttle_should_emit("price_poll", "tick15m_closed", 60) {
                            st.push_log(format!(
                                "🧩 Hibrit 15m: {} kapanan bar sentezlendi (cache+DB kuyruğu)",
                                closed_15m_count,
                            ));
                        }
                    }
                    if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                        pipe.record_step("price_poll", price_poll_status, now_epoch_secs, 0);
                        for (sym, e) in &errors {
                            if to_purge.iter().any(|(s, _)| s == sym) {
                                continue;
                            }
                            pipe.push_anomaly(
                                AnomalySeverity::Warning,
                                AnomalyKind::ApiError,
                                format!("fiyat çekme hatası ({}): {}", sym, e),
                            );
                        }
                        if errors.is_empty() {
                            pipe.clear_warning_kind(AnomalyKind::ApiError);
                        }
                    }
                    if let Some(msg) = summary_msg {
                        st.push_log(msg);
                    }
                }

                for (sym, px, ex, mk) in &new_ticks_meta {
                    emit_canonical_event(
                        &st_px,
                        "market",
                        "MARKET_PRICE_UPDATE",
                        &format!("{}:{}", ex, mk),
                        sym,
                        serde_json::json!({
                            "price": px,
                            "interval": interval.as_str(),
                            "source": "price_poll",
                            "poll_secs": poll_secs,
                        }),
                    );
                }

                for (sym, err) in &errors {
                    emit_canonical_event(
                        &st_px,
                        "market",
                        "MARKET_POLL_ERROR",
                        "unknown",
                        sym,
                        serde_json::json!({
                            "error": err,
                            "source": "price_poll",
                            "interval": interval.as_str(),
                            "poll_secs": poll_secs,
                        }),
                    );
                }

                if !closed_5m_for_db.is_empty() || !closed_15m_for_db.is_empty() {
                    if let Some((db_path, profile)) = flush_db_path {
                        if let Ok(mut diagnostic) = lock_diagnostic.write() {
                            diagnostic.owner = "price_poll".to_string();
                            diagnostic.phase = "db_flush".to_string();
                            diagnostic.started_epoch = crate::core::time::now_epoch_secs() as i64;
                        }
                        let rows_5m = closed_5m_for_db.clone();
                        let rows_15m = closed_15m_for_db.clone();
                        let expected_rows = rows_5m.len() + rows_15m.len();
                        let wrote = tokio::time::timeout(
                            Duration::from_secs(
                                env_parse("PRICE_POLL_DB_FLUSH_TIMEOUT_SECS", 10_u64).max(1),
                            ),
                            tokio::task::spawn_blocking(move || -> usize {
                                if let Ok(conn) = crate::persistence::open_db(&db_path) {
                                    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
                                    let mut ok = 0usize;
                                    for (ex, mk, c) in &rows_5m {
                                        if crate::persistence::writer::save_candle_for_profile(
                                            &conn, &profile, ex, mk, c,
                                        )
                                        .is_ok()
                                        {
                                            ok += 1;
                                        }
                                    }
                                    for (ex, mk, c) in &rows_15m {
                                        if crate::persistence::writer::save_candle_for_profile(
                                            &conn, &profile, ex, mk, c,
                                        )
                                        .is_ok()
                                        {
                                            ok += 1;
                                        }
                                    }
                                    ok
                                } else {
                                    0
                                }
                            }),
                        )
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .unwrap_or(0);
                        if let Ok(mut st) = st_px.lock() {
                            if wrote > 0 {
                                Self::inc_counter(&mut st, "price_poll.tick15m.db_flush_ok");
                                *st.fleet
                                    .autonomy_counters
                                    .entry("price_poll.tick15m.db_rows_written".to_string())
                                    .or_insert(0) += wrote as u64;
                                st.fleet.autonomy_counters.insert(
                                    "price_poll.tick15m.last_db_flush_epoch".to_string(),
                                    now_epoch_secs,
                                );
                            }
                            if wrote < expected_rows {
                                Self::inc_counter(
                                    &mut st,
                                    "price_poll.tick15m.db_flush_partial_or_err",
                                );
                                st.fleet.autonomy_counters.insert(
                                    "price_poll.tick15m.last_db_partial_epoch".to_string(),
                                    now_epoch_secs,
                                );
                                st.fleet.autonomy_counters.insert(
                                    "price_poll.tick15m.last_db_partial_missing_rows".to_string(),
                                    ((closed_5m_for_db.len() + closed_15m_for_db.len())
                                        .saturating_sub(wrote))
                                        as u64,
                                );
                                if log_throttle_should_emit("price_poll", "tick15m_db_partial", 120)
                                {
                                    st.push_log(format!(
                                        "⚠️ Hibrit DB flush kısmi: {}/{} satır yazıldı",
                                        wrote,
                                        closed_5m_for_db.len() + closed_15m_for_db.len(),
                                    ));
                                }

                                emit_pipeline_step_event(
                                    &st_px,
                                    "price_poll",
                                    price_poll_status,
                                    now_epoch_secs,
                                    0,
                                );
                                for (sym, err) in &errors {
                                    if to_purge.iter().any(|(s, _)| s == sym) {
                                        continue;
                                    }
                                    let msg = format!("fiyat çekme hatası ({}): {}", sym, err);
                                    emit_pipeline_anomaly_event(
                                        &st_px,
                                        AnomalySeverity::Warning,
                                        AnomalyKind::ApiError,
                                        &msg,
                                    );
                                }
                            }
                        }
                    }
                }

                for (sym, n) in &to_purge {
                    Self::purge_delisted_symbol(&st_px, sym, *n);
                }

                // Son tamamlanan çağrı eski bir kilitlenme gibi görünmesin.
                if let Ok(mut diagnostic) = lock_diagnostic.write() {
                    diagnostic.owner.clear();
                    diagnostic.phase = "idle".to_string();
                    diagnostic.started_epoch = 0;
                }

                sleep(Duration::from_secs(poll_secs)).await;
            }
        });

        let st_trig = Arc::clone(&state);
        tokio::spawn(async move {
            use crate::robot::data_pipeline::StepStatus;
            let in_flight: Arc<Mutex<std::collections::HashSet<String>>> =
                Arc::new(Mutex::new(std::collections::HashSet::new()));
            loop {
                let (fired, stop) = {
                    let st = match st_trig.lock() {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    if st.app_stop_signal.load(Ordering::Relaxed) {
                        (vec![], true)
                    } else {
                        let mut fired = Vec::new();
                        for (name, flag) in st.fleet.triggers.iter() {
                            if flag.swap(false, Ordering::AcqRel) {
                                fired.push(name.clone());
                            }
                        }
                        (fired, false)
                    }
                };
                if stop {
                    break;
                }

                if !fired.is_empty() {
                    let now_secs: u64 = crate::core::time::now_epoch_secs();

                    for name in &fired {
                        let label = format!("trigger:{}", name);

                        let accepted = {
                            match in_flight.lock() {
                                Ok(mut set) => set.insert(name.clone()),
                                Err(poison) => {
                                    let mut set = poison.into_inner();
                                    set.insert(name.clone())
                                }
                            }
                        };
                        if !accepted {
                            if let Ok(mut st) = st_trig.lock() {
                                Self::inc_counter(
                                    &mut st,
                                    &format!("trigger.dispatch.{}.inflight_skipped", name),
                                );
                                st.push_log(format!(
                                    "⏭️ Tetik [{}] atlandı: önceki job hâlâ çalışıyor",
                                    name,
                                ));
                                if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                                    pipe.record_step(
                                        label.clone(),
                                        // Önceki job hâlâ çalışıyorsa adım gerçekten aktiftir;
                                        // yalnızca tekrar tetikleme isteği coalesced edilir.
                                        StepStatus::Running,
                                        now_secs,
                                        0,
                                    );
                                }
                            }
                            continue;
                        }

                        if let Ok(mut st) = st_trig.lock() {
                            Self::inc_counter(
                                &mut st,
                                &format!("trigger.dispatch.{}.accepted", name),
                            );
                            let n_anom = st
                                .guardian
                                .live_pipeline
                                .read()
                                .map(|p| p.anomalies.len())
                                .unwrap_or(0);
                            let context = if n_anom > 0 {
                                "otonom (anomali)"
                            } else {
                                "manuel veya zamanlı"
                            };
                            let detail = match name.as_str() {
                                "ml"       => "GBT modeli yeniden eğitilecek, best_params güncellenecek",
                                "backtest" => "Walk-forward grid search çalışacak, aktif strateji yeniden seçilecek",
                                "download" => "Pipeline mum tamamlama görevi koşacak",
                                "screener" => "Sembol tarayıcı yeniden çalışacak",
                                _ => "Bilinmeyen tetikleyici",
                            };
                            st.push_log(format!("🎮 Tetik [{}] ⇒ {}: {}", context, name, detail));
                            if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                                pipe.record_step(label.clone(), StepStatus::Running, now_secs, 0);
                            }
                        }

                        let state_clone = Arc::clone(&st_trig);
                        let trigger_name = name.clone();
                        let in_flight_clone = Arc::clone(&in_flight);

                        tokio::spawn(async move {
                            let _in_flight_guard =
                                InFlightTriggerGuard::new(trigger_name.clone(), in_flight_clone);
                            if let Ok(mut st) = state_clone.lock() {
                                Self::inc_counter(
                                    &mut st,
                                    &format!("job.{}.started", trigger_name),
                                );
                            }
                            let mut final_status = StepStatus::Done;
                            match trigger_name.as_str() {
                                "ml" => {
                                    Self::mark_pipeline_stage(
                                        &state_clone,
                                        crate::robot::data_pipeline::canon::PipelineStage::Optimize,
                                        crate::robot::data_pipeline::StepStatus::Running,
                                    );
                                    let st_for_job = Arc::clone(&state_clone);
                                    let out = tokio::task::spawn_blocking(move || {
                                        Self::run_ml_retrain_job(&st_for_job)
                                    })
                                    .await;
                                    match out {
                                        Ok(Ok(())) => {
                                            Self::mark_pipeline_stage(
                                                &state_clone,
                                                crate::robot::data_pipeline::canon::PipelineStage::Optimize,
                                                crate::robot::data_pipeline::StepStatus::Done,
                                            );
                                        }
                                        Ok(Err(e)) => {
                                            if let Ok(mut st) = state_clone.lock() {
                                                st.push_log(format!(
                                                    "❌ ML Retrain başarısız: {}",
                                                    e
                                                ));
                                            }
                                            final_status = StepStatus::Failed;
                                            Self::mark_pipeline_stage(
                                                &state_clone,
                                                crate::robot::data_pipeline::canon::PipelineStage::Optimize,
                                                crate::robot::data_pipeline::StepStatus::Failed,
                                            );
                                        }
                                        Err(e) => {
                                            log::warn!("🧠 ML retrain join hatası: {}", e);
                                            final_status = StepStatus::Failed;
                                            Self::mark_pipeline_stage(
                                                &state_clone,
                                                crate::robot::data_pipeline::canon::PipelineStage::Optimize,
                                                crate::robot::data_pipeline::StepStatus::Failed,
                                            );
                                        }
                                    }
                                }
                                "backtest" => {
                                    Self::mark_pipeline_stage(
                                        &state_clone,
                                        crate::robot::data_pipeline::canon::PipelineStage::Optimize,
                                        crate::robot::data_pipeline::StepStatus::Running,
                                    );
                                    let st_for_job = Arc::clone(&state_clone);
                                    let out = tokio::task::spawn_blocking(move || {
                                        Self::run_backtest_job(&st_for_job)
                                    })
                                    .await;
                                    match out {
                                        Ok(Ok(())) => {
                                            Self::mark_pipeline_stage(
                                                &state_clone,
                                                crate::robot::data_pipeline::canon::PipelineStage::Optimize,
                                                crate::robot::data_pipeline::StepStatus::Done,
                                            );
                                        }
                                        Ok(Err(e)) => {
                                            log::warn!("🔬 Backtest başarısız: {}", e);
                                            if let Ok(mut st) = state_clone.lock() {
                                                st.push_log(format!(
                                                    "❌ Backtest başarısız: {}",
                                                    e
                                                ));
                                            }
                                            final_status = StepStatus::Failed;
                                            Self::mark_pipeline_stage(
                                                &state_clone,
                                                crate::robot::data_pipeline::canon::PipelineStage::Optimize,
                                                crate::robot::data_pipeline::StepStatus::Failed,
                                            );
                                        }
                                        Err(e) => {
                                            log::warn!("🔬 Backtest join hatası: {}", e);
                                            final_status = StepStatus::Failed;
                                            Self::mark_pipeline_stage(
                                                &state_clone,
                                                crate::robot::data_pipeline::canon::PipelineStage::Optimize,
                                                crate::robot::data_pipeline::StepStatus::Failed,
                                            );
                                        }
                                    }
                                }
                                "download" => {
                                    let st_for_dl = Arc::clone(&state_clone);
                                    if let Err(e) = Self::run_download_job(&st_for_dl).await {
                                        log::warn!("🌐 Download başarısız: {}", e);
                                        if let Ok(mut st) = state_clone.lock() {
                                            st.push_log(format!("❌ Download başarısız: {}", e));
                                        }
                                        final_status = StepStatus::Failed;
                                    }
                                }
                                "screener" => {
                                    log::info!("🔭 E2: Sembol tarayıcısı başladı (otonom multi-symbol seçimi)");
                                    let st_for_scr = Arc::clone(&state_clone);
                                    let out = tokio::task::spawn_blocking(move || {
                                        Self::run_screener_job(&st_for_scr)
                                    })
                                    .await;
                                    match out {
                                        Ok(Ok(())) => {}
                                        Ok(Err(e)) => {
                                            log::warn!("🔭 Screener başarısız: {}", e);
                                            if let Ok(mut st) = state_clone.lock() {
                                                st.push_log(format!(
                                                    "❌ Screener başarısız: {}",
                                                    e
                                                ));
                                            }
                                            final_status = StepStatus::Failed;
                                        }
                                        Err(e) => {
                                            log::warn!("🔭 Screener join hatası: {}", e);
                                            final_status = StepStatus::Failed;
                                        }
                                    }
                                }
                                _ => {
                                    log::warn!(
                                        "⚠️ Bilinmeyen tetikleyici sinyali alındı: {}",
                                        trigger_name
                                    );
                                    final_status = StepStatus::Skipped;
                                }
                            }

                            if let Ok(mut st) = state_clone.lock() {
                                let outcome = match final_status {
                                    StepStatus::Done => "done",
                                    StepStatus::Failed => "failed",
                                    StepStatus::Degraded => "degraded",
                                    StepStatus::Skipped => "skipped",
                                    StepStatus::Running => "running",
                                    StepStatus::Idle => "idle",
                                };
                                Self::inc_counter(
                                    &mut st,
                                    &format!("job.{}.{}", trigger_name, outcome),
                                );
                                if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                                    pipe.record_step(label, final_status, now_secs, 0);
                                }
                            }
                        });
                    }
                }
                sleep(Duration::from_millis(250)).await;
            }
        });
        // ── Task 2: Phase tracker — fleet.phase değişimini pipeline step'i olarak işle.
        let st_pipe = Arc::clone(&state);
        tokio::spawn(async move {
            use crate::robot::data_pipeline::StepStatus;
            let mut last_phase = String::new();
            loop {
                let (current_phase, stop) = {
                    let st = match st_pipe.lock() {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    if st.app_stop_signal.load(Ordering::Relaxed) {
                        (String::new(), true)
                    } else {
                        (st.fleet.phase.clone(), false)
                    }
                };
                if stop {
                    break;
                }

                if current_phase != last_phase {
                    let now_epoch_secs: u64 = crate::core::time::now_epoch_secs();
                    if let Ok(st) = st_pipe.lock() {
                        if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                            pipe.record_step(
                                format!("phase:{}", current_phase),
                                StepStatus::Done,
                                now_epoch_secs,
                                0,
                            );
                        }
                    }
                    last_phase = current_phase;
                }
                sleep(Duration::from_secs(2)).await;
            }
        });

        // ── Task 5: Scheduler — config'deki periyodlara göre download/backtest tetiği.
        let st_sched = Arc::clone(&state);
        tokio::spawn(async move {
            const WARMUP_SECS: u64 = 30;
            const CHECK_EVERY_SECS: u64 = 60;

            let mut last_download_at: Option<std::time::Instant> = None;
            let mut last_priority_download_at: Option<std::time::Instant> = None;
            let mut last_backtest_at: Option<std::time::Instant> = None;
            let mut last_screener_at: Option<std::time::Instant> = None;
            let mut last_ml_at: Option<std::time::Instant> = None;
            let mut last_symstatus_at: Option<std::time::Instant> = None;
            let mut last_report_at: Option<std::time::Instant> = None;
            let mut last_logscan_at: Option<std::time::Instant> = None;
            let mut bist_priority_day: Option<chrono::NaiveDate> = None;
            let mut bist_priority_attempts = 0u32;
            let mut counter_baseline_day = chrono::Local::now().date_naive();
            let mut counter_baseline: std::collections::BTreeMap<String, u64> =
                std::collections::BTreeMap::new();
            let mut warmup_done = false;

            let report_period: u64 = env_parse("SCHEDULER_REPORT_EVERY_MINS", 1440);
            let symstatus_period: u64 = env_parse("SCHEDULER_SYMSTATUS_EVERY_MINS", 360);

            let logscan_enabled = std::env::var("SCHEDULER_LOGSCAN_ENABLED")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true);
            let logscan_period: u64 = env_parse("SCHEDULER_LOGSCAN_EVERY_MINS", 60u64).max(60);
            let logscan_recovery_enabled = std::env::var("SCHEDULER_LOGSCAN_RECOVERY_ENABLE")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true);

            let screener_enabled = std::env::var("SCHEDULER_SCREENER_ENABLED")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true);
            let screener_period: u64 = env_parse("SCHEDULER_SCREENER_EVERY_MINS", 30);

            let ml_periodic_enabled = std::env::var("SCHEDULER_ML_ENABLED")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true);
            let ml_period: u64 = env_parse("SCHEDULER_ML_EVERY_MINS", 120);

            let bt_period_override: Option<u64> = std::env::var("SCHEDULER_BACKTEST_EVERY_MINS")
                .ok()
                .and_then(|s| s.parse().ok());
            let bt_warmup = env_truthy("SCHEDULER_BACKTEST_WARMUP");
            let binance_active = std::env::var("VENUES")
                .ok()
                .map(|venues| {
                    venues.split(',').any(|venue| {
                        venue.trim().split_once(':').map_or(
                            venue.trim().eq_ignore_ascii_case("binance"),
                            |(exchange, _)| exchange.trim().eq_ignore_ascii_case("binance"),
                        )
                    })
                })
                .unwrap_or(true);

            sleep(Duration::from_secs(WARMUP_SECS)).await;
            loop {
                crate::robot::infra::app_state_lock_watchdog::set_intent("scheduler:cycle");
                let stop = st_sched
                    .lock()
                    .map(|s| s.app_stop_signal.load(Ordering::Relaxed))
                    .unwrap_or(true);
                if stop {
                    break;
                }

                let (dl_enabled, dl_period, bt_enabled, bt_period) = {
                    let st = match st_sched.lock() {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    (
                        st.config.download_enabled,
                        st.config.download_every_mins,
                        st.config.pipeline_enabled,
                        bt_period_override.unwrap_or(st.config.pipeline_every_mins),
                    )
                };

                let now = std::time::Instant::now();
                let data_completion_needed = {
                    let freshness_secs = env_parse("DATA_COVERAGE_FRESHNESS_SECS", 45 * 60).max(60);
                    let cutoff =
                        chrono::Utc::now() - chrono::Duration::seconds(freshness_secs as i64);
                    let (mut symbols, venue_registry, opportunity_cache, bist_download_failed) = {
                        let st = match st_sched.lock() {
                            Ok(st) => st,
                            Err(_) => break,
                        };
                        let mut symbols = std::collections::BTreeSet::new();
                        if !st.config.symbol.is_empty() {
                            symbols.insert(st.config.symbol.clone());
                        }
                        if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                            symbols.extend(
                                orch.get_worker_status()
                                    .into_iter()
                                    .map(|worker| worker.symbol),
                            );
                        }
                        if let Ok(positions) = st.finance.live_positions.read() {
                            symbols.extend(positions.keys().cloned());
                        }
                        symbols.extend(st.config.pinned_symbols.iter().cloned());
                        let bist_download_failed = st
                            .fleet
                            .download_progress
                            .read()
                            .map(|progress| progress.failed_symbols > 0)
                            .unwrap_or(false);
                        (
                            symbols,
                            Arc::clone(&st.venue_registry),
                            Arc::clone(&st.fleet.opportunity_cache),
                            bist_download_failed,
                        )
                    };
                    [
                        "AUTO_BINANCE_SYMBOLS",
                        "AUTO_MT5_SYMBOLS",
                        "AUTO_IBKR_SYMBOLS",
                        "AUTO_BIST_SYMBOLS",
                    ]
                    .into_iter()
                    .filter_map(|key| std::env::var(key).ok())
                    .flat_map(|raw| {
                        raw.split(',')
                            .map(str::trim)
                            .filter(|symbol| !symbol.is_empty())
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    })
                    .for_each(|symbol| {
                        symbols.insert(symbol);
                    });

                    symbols.into_iter().any(|symbol| {
                        let is_bist = venue_registry.route(&symbol).map(|(venue, _)| {
                            venue.exchange() == crate::core::types::Exchange::Bist
                        }) == Some(true);
                        if is_bist && bist_download_failed {
                            return true;
                        }
                        if is_bist {
                            let turkey_now = chrono::Utc::now() + chrono::Duration::hours(3);
                            if matches!(
                                turkey_now.weekday(),
                                chrono::Weekday::Sat | chrono::Weekday::Sun
                            ) || bist_market_window_open(chrono::Utc::now())
                            {
                                return false;
                            }
                            let today = turkey_now.date_naive();
                            return opportunity_cache
                                .get_latest(&symbol, "1d", 1)
                                .last()
                                .map(|candle| {
                                    (candle.timestamp + chrono::Duration::hours(3)).date_naive()
                                        < today
                                })
                                .unwrap_or(true);
                        }
                        opportunity_cache
                            .get_latest(&symbol, "15m", 1)
                            .last()
                            .map(|candle| candle.timestamp < cutoff)
                            .unwrap_or(true)
                    })
                };

                if !warmup_done {
                    warmup_done = true;
                    if dl_enabled {
                        let queued = {
                            if let Ok(st) = st_sched.lock() {
                                if let Some(t) = st.fleet.triggers.get("download") {
                                    !t.swap(true, Ordering::AcqRel)
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        };
                        if let Ok(mut st) = st_sched.lock() {
                            if queued {
                                Self::inc_counter(&mut st, "trigger.scheduler.download.enqueued");
                                st.push_log(
                                    "⏰ Scheduler: warmup tamamlandı → ilk download tetiği".into(),
                                );
                            } else {
                                Self::inc_counter(&mut st, "trigger.scheduler.download.coalesced");
                                st.push_log("🔁 Scheduler: warmup download tetiği zaten kuyruktaydı (coalesced)".into());
                            }
                        }
                        last_download_at = Some(now);
                    }

                    let skip_warmup_ml = env_truthy("SCHEDULER_ML_WARMUP_SKIP");
                    if ml_periodic_enabled && !skip_warmup_ml {
                        let queued = {
                            if let Ok(st) = st_sched.lock() {
                                if let Some(t) = st.fleet.triggers.get("ml") {
                                    !t.swap(true, Ordering::AcqRel)
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        };
                        if let Ok(mut st) = st_sched.lock() {
                            if queued {
                                Self::inc_counter(&mut st, "trigger.scheduler.ml.enqueued");
                                st.push_log(
                                    "⏰ Scheduler: warmup → ilk ML retrain tetiği (GBT cold-train)"
                                        .into(),
                                );
                            } else {
                                Self::inc_counter(&mut st, "trigger.scheduler.ml.coalesced");
                                st.push_log("🔁 Scheduler: warmup ML retrain tetiği zaten kuyruktaydı (coalesced)".into());
                            }
                        }
                        last_ml_at = Some(now);
                    }

                    if bt_enabled && bt_warmup {
                        let queued = {
                            if let Ok(st) = st_sched.lock() {
                                if let Some(t) = st.fleet.triggers.get("backtest") {
                                    !t.swap(true, Ordering::AcqRel)
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        };
                        if let Ok(mut st) = st_sched.lock() {
                            if queued {
                                Self::inc_counter(&mut st, "trigger.scheduler.backtest.enqueued");
                                st.push_log("⏰ Scheduler: warmup → ilk backtest tetiği (SCHEDULER_BACKTEST_WARMUP)".into());
                            } else {
                                Self::inc_counter(&mut st, "trigger.scheduler.backtest.coalesced");
                                st.push_log("🔁 Scheduler: warmup backtest tetiği zaten kuyruktaydı (coalesced)".into());
                            }
                        }
                        last_backtest_at = Some(now);
                    }
                }
                let priority_due = data_completion_needed
                    && last_priority_download_at
                        .map(|t| now.duration_since(t) >= Duration::from_secs(5 * 60))
                        .unwrap_or(true)
                    && (!st_sched
                        .lock()
                        .map(|st| st.config.exchange.eq_ignore_ascii_case("bist"))
                        .unwrap_or(false)
                        || bist_priority_attempts
                            < env_parse("BIST_MAX_DAILY_RETRIES", 4u64) as u32);
                let turkey_today = chrono::Utc::now()
                    .checked_add_signed(chrono::Duration::hours(3))
                    .map(|value| value.date_naive())
                    .unwrap_or_else(|| chrono::Utc::now().date_naive());
                if bist_priority_day != Some(turkey_today) {
                    bist_priority_day = Some(turkey_today);
                    bist_priority_attempts = 0;
                }
                if dl_enabled && (priority_due || dl_period > 0) {
                    let due = match last_download_at {
                        Some(t) => now.duration_since(t) >= Duration::from_secs(dl_period * 60),
                        None => true,
                    } || priority_due;
                    if due {
                        let is_bist_exchange = st_sched
                            .lock()
                            .map(|st| st.config.exchange.eq_ignore_ascii_case("bist"))
                            .unwrap_or(false);
                        let queued = {
                            if let Ok(st) = st_sched.lock() {
                                if let Some(t) = st.fleet.triggers.get("download") {
                                    !t.swap(true, Ordering::AcqRel)
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        };
                        if let Ok(mut st) = st_sched.lock() {
                            if queued {
                                Self::inc_counter(&mut st, "trigger.scheduler.download.enqueued");
                                if priority_due {
                                    if is_bist_exchange {
                                        bist_priority_attempts =
                                            bist_priority_attempts.saturating_add(1);
                                    }
                                    Self::inc_counter(
                                        &mut st,
                                        "trigger.scheduler.download.priority_enqueued",
                                    );
                                    st.push_log(
                                        "🚨 Scheduler: veri kapsamı eksik/bayat → öncelikli download tetiği"
                                            .into(),
                                    );
                                } else {
                                    st.push_log(format!(
                                        "⏰ Scheduler: periyodik download tetiği (her {} dk)",
                                        dl_period
                                    ));
                                }
                            } else {
                                Self::inc_counter(&mut st, "trigger.scheduler.download.coalesced");
                                st.push_log(format!("🔁 Scheduler: periyodik download tetiği zaten kuyruktaydı (coalesced, {} dk)", dl_period));
                            }
                        }
                        last_download_at = Some(now);
                        if priority_due {
                            last_priority_download_at = Some(now);
                        }
                    }
                }

                if bt_enabled && bt_period > 0 {
                    let due = match last_backtest_at {
                        Some(t) => now.duration_since(t) >= Duration::from_secs(bt_period * 60),
                        None => false,
                    };
                    if due {
                        let queued = {
                            if let Ok(st) = st_sched.lock() {
                                if let Some(t) = st.fleet.triggers.get("backtest") {
                                    !t.swap(true, Ordering::AcqRel)
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        };
                        if let Ok(mut st) = st_sched.lock() {
                            if queued {
                                Self::inc_counter(&mut st, "trigger.scheduler.backtest.enqueued");
                                st.push_log(format!(
                                    "⏰ Scheduler: periyodik backtest tetiği (her {} dk)",
                                    bt_period
                                ));
                            } else {
                                Self::inc_counter(&mut st, "trigger.scheduler.backtest.coalesced");
                                st.push_log(format!("🔁 Scheduler: periyodik backtest tetiği zaten kuyruktaydı (coalesced, {} dk)", bt_period));
                            }
                        }
                        last_backtest_at = Some(now);
                    } else if last_backtest_at.is_none() {
                        last_backtest_at = Some(now);
                    }
                }

                if screener_enabled && screener_period > 0 {
                    let due = match last_screener_at {
                        Some(t) => {
                            now.duration_since(t) >= Duration::from_secs(screener_period * 60)
                        }
                        None => true,
                    };
                    if due {
                        let queued = {
                            if let Ok(st) = st_sched.lock() {
                                if let Some(t) = st.fleet.triggers.get("screener") {
                                    !t.swap(true, Ordering::AcqRel)
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        };
                        if let Ok(mut st) = st_sched.lock() {
                            if queued {
                                Self::inc_counter(&mut st, "trigger.scheduler.screener.enqueued");
                                st.push_log(format!(
                                    "⏰ Scheduler: periyodik screener tetiği (her {} dk)",
                                    screener_period
                                ));
                            } else {
                                Self::inc_counter(&mut st, "trigger.scheduler.screener.coalesced");
                                st.push_log(format!("🔁 Scheduler: periyodik screener tetiği zaten kuyruktaydı (coalesced, {} dk)", screener_period));
                            }
                        }
                        last_screener_at = Some(now);
                    }
                }
                if ml_periodic_enabled && ml_period > 0 {
                    let due = match last_ml_at {
                        Some(t) => now.duration_since(t) >= Duration::from_secs(ml_period * 60),
                        None => false,
                    };
                    if due {
                        let queued = {
                            if let Ok(st) = st_sched.lock() {
                                if let Some(t) = st.fleet.triggers.get("ml") {
                                    !t.swap(true, Ordering::AcqRel)
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        };
                        if let Ok(mut st) = st_sched.lock() {
                            if queued {
                                Self::inc_counter(&mut st, "trigger.scheduler.ml.enqueued");
                                st.push_log(format!(
                                    "⏰ Scheduler: periyodik ML retrain tetiği (her {} dk)",
                                    ml_period
                                ));
                            } else {
                                Self::inc_counter(&mut st, "trigger.scheduler.ml.coalesced");
                                st.push_log(format!("🔁 Scheduler: periyodik ML retrain tetiği zaten kuyruktaydı (coalesced, {} dk)", ml_period));
                            }
                        }
                        last_ml_at = Some(now);
                    } else if last_ml_at.is_none() {
                        last_ml_at = Some(now);
                    }
                }

                if report_period > 0 {
                    let due = match last_report_at {
                        Some(t) => now.duration_since(t) >= Duration::from_secs(report_period * 60),
                        None => true,
                    };
                    if due {
                        if let Ok(mut st) = st_sched.lock() {
                            let today = chrono::Local::now().date_naive();
                            if today != counter_baseline_day {
                                counter_baseline_day = today;
                                counter_baseline = st.fleet.autonomy_counters.clone();
                                st.push_log(
                                    "📆 Otonom sayaç baseline sıfırlandı (yeni gün)".into(),
                                );
                            }

                            let eq = st.finance.equity;
                            let start = st.finance.starting_capital.max(1.0);
                            let ret = (eq - start) / start * 100.0;
                            let (n_open, open_pnl) = st
                                .finance
                                .live_positions
                                .read()
                                .ok()
                                .map(|p| {
                                    (p.len(), p.values().map(|x| x.calculate_pnl()).sum::<f64>())
                                })
                                .unwrap_or((0, 0.0));
                            let closed = st.finance.closed_trades_total.load(Ordering::Relaxed);
                            let fees = st
                                .finance
                                .live_execution_costs
                                .read()
                                .ok()
                                .map(|c| c.commission_usd)
                                .unwrap_or(0.0);

                            let mut deltas: Vec<(String, u64)> = st
                                .fleet
                                .autonomy_counters
                                .iter()
                                .filter_map(|(k, v)| {
                                    let base = counter_baseline.get(k).copied().unwrap_or(0);
                                    let d = v.saturating_sub(base);
                                    if d > 0 {
                                        Some((k.clone(), d))
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            deltas.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                            let delta_total: u64 = deltas.iter().map(|(_, d)| *d).sum();
                            let delta_coalesced: u64 = deltas
                                .iter()
                                .filter(|(k, _)| {
                                    k.contains("coalesced") || k.contains("inflight_skipped")
                                })
                                .map(|(_, d)| *d)
                                .sum();
                            let delta_failed: u64 = deltas
                                .iter()
                                .filter(|(k, _)| k.ends_with(".failed"))
                                .map(|(_, d)| *d)
                                .sum();

                            let is_priority_counter = |k: &str| {
                                k.ends_with(".failed")
                                    || k.contains(".coalesced")
                                    || k.contains(".inflight_skipped")
                            };
                            let mut top_priority: Vec<(String, u64)> = deltas
                                .iter()
                                .filter(|(k, _)| is_priority_counter(k))
                                .cloned()
                                .collect();
                            top_priority.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                            let mut top_general: Vec<(String, u64)> = deltas
                                .iter()
                                .filter(|(k, _)| !k.starts_with("indicator."))
                                .cloned()
                                .collect();
                            top_general.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                            let top = if top_priority.is_empty() {
                                top_general
                                    .iter()
                                    .take(3)
                                    .map(|(k, d)| format!("{}=+{}", k, d))
                                    .collect::<Vec<_>>()
                                    .join(" · ")
                            } else {
                                top_priority
                                    .iter()
                                    .take(3)
                                    .map(|(k, d)| format!("{}=+{}", k, d))
                                    .collect::<Vec<_>>()
                                    .join(" · ")
                            };
                            let indicator_top = deltas
                                .iter()
                                .filter(|(k, _)| k.starts_with("indicator."))
                                .take(3)
                                .map(|(k, d)| format!("{}=+{}", k, d))
                                .collect::<Vec<_>>()
                                .join(" · ");
                            let indicator_used: u64 = deltas
                                .iter()
                                .filter(|(k, _)| {
                                    k.starts_with("indicator.") && k.ends_with(".used")
                                })
                                .map(|(_, d)| *d)
                                .sum();
                            let indicator_gated: u64 = deltas
                                .iter()
                                .filter(|(k, _)| {
                                    k.starts_with("indicator.") && k.contains(".gated_")
                                })
                                .map(|(_, d)| *d)
                                .sum();
                            let indicator_win: u64 = deltas
                                .iter()
                                .filter(|(k, _)| {
                                    k.starts_with("indicator.") && k.ends_with(".win_contrib")
                                })
                                .map(|(_, d)| *d)
                                .sum();
                            let indicator_loss: u64 = deltas
                                .iter()
                                .filter(|(k, _)| {
                                    k.starts_with("indicator.") && k.ends_with(".loss_contrib")
                                })
                                .map(|(_, d)| *d)
                                .sum();

                            Self::inc_counter(&mut st, "scheduler.report.sent");
                            let msg = format!(
                                "📊 Memos özet · equity ${:.2} ({:+.2}%) · açık {} (P&L ${:+.2}) · kapanan {} · komisyon ${:.2} · otonomΔ +{} (coal {} / fail {}){}{}",
                                eq, ret, n_open, open_pnl, closed, fees, delta_total, delta_coalesced, delta_failed,
                                if top.is_empty() { String::new() } else { format!(" · top {}", top) },
                                if indicator_top.is_empty() { String::new() } else {
                                    format!(
                                        " · ind used+{} gated+{} w/l {}:{} · {}",
                                        indicator_used, indicator_gated, indicator_win, indicator_loss, indicator_top,
                                    )
                                },
                            );
                            st.push_alert(
                                "portfolio-report",
                                crate::robot::infra::telegram_notifier::Severity::Info,
                                msg,
                            );
                        }
                        last_report_at = Some(now);
                    }
                }

                if binance_active && symstatus_period > 0 {
                    let due = match last_symstatus_at {
                        Some(t) => {
                            now.duration_since(t) >= Duration::from_secs(symstatus_period * 60)
                        }
                        None => true,
                    };
                    if due {
                        last_symstatus_at = Some(now);
                        let st_job = Arc::clone(&st_sched);
                        tokio::spawn(async move {
                            if let Err(e) = Self::run_symbol_status_refresh(&st_job).await {
                                log::warn!("symbol_status refresh: {}", e);
                            } else if let Err(e) =
                                Self::run_binance_universe_discovery(&st_job).await
                            {
                                log::warn!("binance universe discovery: {}", e);
                            }
                            for (exchange, env_key, tag, market) in [
                                (
                                    crate::core::types::Exchange::Mt5,
                                    "AUTO_MT5_DISCOVERY_SYMBOLS",
                                    "mt5",
                                    "cfd",
                                ),
                                (
                                    crate::core::types::Exchange::Ibkr,
                                    "AUTO_IBKR_DISCOVERY_SYMBOLS",
                                    "ibkr",
                                    "cfd",
                                ),
                            ] {
                                if let Err(e) = Self::run_configured_venue_discovery(
                                    &st_job, exchange, env_key, tag, market,
                                )
                                .await
                                {
                                    log::warn!("{} universe discovery: {}", tag, e);
                                }
                            }
                        });
                    }
                }

                if logscan_enabled && logscan_period > 0 {
                    let due = match last_logscan_at {
                        Some(t) => {
                            now.duration_since(t) >= Duration::from_secs(logscan_period * 60)
                        }
                        None => false,
                    };
                    if due {
                        last_logscan_at = Some(now);
                        let st_job = Arc::clone(&st_sched);
                        tokio::spawn(async move {
                            let st2 = Arc::clone(&st_job);
                            let _ = tokio::task::spawn_blocking(move || {
                                Self::run_periodic_log_scan(&st2, logscan_recovery_enabled);
                            })
                            .await;
                        });
                    } else if last_logscan_at.is_none() {
                        last_logscan_at = Some(now);
                    }
                }

                crate::robot::infra::app_state_lock_watchdog::clear_intent("scheduler:cycle");
                sleep(Duration::from_secs(CHECK_EVERY_SECS)).await;
            }
        });
        // ─── KALDIRILDI (mutabakat tek-otorite kararı) ────────────────────────────
        // Burada 30 sn'de bir çalışan ikinci bir pozisyon-senkron görevi vardı:
        // her sembol için `exec.fetch_open_symbols()` çağırıp sembol borsada listede
        // yoksa "SL/TP tetiklendi" varsayıp `cancel_all` + local pozisyonu kapatıyordu.
        //
        // NEDEN KALDIRILDI:
        //   1) `BinanceTradeExecutor::fetch_open_symbols()` bir STUB'dı (koşulsuz `vec![]`)
        //      → her canlı Binance pozisyonu "borsada yok" okunuyor, 30 sn'de bir
        //      YANLIŞ kapatma + `cancel_all` üretiyordu. Tüm `live-*.conf` profillerinde
        //      aktifti (LIVE_DRY_RUN=false).
        //   2) MT5 tarafı `Handle::current().block_on()` ile senkron blokluyordu; async
        //      bağlamdan çağrıldığı için panikleyip bu görevi sessizce öldürüyordu.
        //   3) Yalnız sembol ADI karşılaştırıyordu: miktar/yön/giriş fiyatı farkını
        //      göremiyordu ve kapanış sebebini sabit `TrailingStop` diye yalan yazıyordu.
        //   4) İkinci bir veri kaynağı olduğu için `reconcile_diff` ile çelişebiliyordu.
        //
        // Tek otorite artık `Engine::reconcile_live_positions_with_exchange`
        // (persistence.rs): `fetch_account_positions()` ile beslenir, miktar+yön+giriş
        // fiyatını karşılaştırır (stale/mismatched/unknown) ve stale'i kapalı işlem
        // olarak kaydeder. loop_core.rs:29 (boot) + :96 (her cycle) üzerinden çalışır.

        let reports_dir =
            std::env::var("TRADE_REPORT_DIR").unwrap_or_else(|_| "data/reports".to_owned());
        let report_every = std::env::var("TRADE_REPORT_EVERY_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300);
        crate::robot::infra::reporting::trade_summary::spawn_trade_summary(
            Arc::clone(&state),
            reports_dir,
            report_every,
        );
        Self::spawn_sr_updater(Arc::clone(&state));
        Self::spawn_trail_feedback_processor(Arc::clone(&state));
        Self::spawn_scalp_swing_tuner(Arc::clone(&state));
    }
}

fn effective_max_candle_age(base: i64, interval_secs: i64) -> i64 {
    if base <= 0 {
        base
    } else {
        base.max(interval_secs * 2)
    }
}
