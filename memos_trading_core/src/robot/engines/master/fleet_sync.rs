// src/robot/engines/master/fleet_sync.rs — Canlı borsa senkron task'ları:
// hesap bakiye senkronu + WebSocket userDataStream.
// Faz 2 modülerleştirme: infra_fleet.rs'ten ayrıldı (davranış birebir korunur).
use super::*;
use std::collections::BTreeMap;
use std::sync::RwLock;

fn update_venue_runtime(
    runtime_store: &Arc<RwLock<BTreeMap<String, crate::robot::robotic_loop::VenueRuntimeState>>>,
    venue_key: &str,
    update: impl FnOnce(&mut crate::robot::robotic_loop::VenueRuntimeState),
) {
    if let Ok(mut runtimes) = runtime_store.write() {
        update(runtimes.entry(venue_key.to_string()).or_default());
    }
}

#[cfg(test)]
mod tests {
    use super::{balance_sync_allowed, update_venue_runtime};
    use crate::core::model::TradingMode;
    use crate::robot::robotic_loop::VenueRuntimeState;
    use std::collections::BTreeMap;
    use std::sync::{Arc, RwLock};
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn venue_runtime_updates_are_isolated_during_peer_timeout() {
        let runtimes = Arc::new(RwLock::new(BTreeMap::<String, VenueRuntimeState>::new()));
        let timed_out = Arc::clone(&runtimes);
        let healthy = Arc::clone(&runtimes);

        let timeout_task = tokio::spawn(async move {
            update_venue_runtime(&timed_out, "mt5:cfd", |runtime| {
                runtime.scheduler_phase = "AccountSync".to_string();
            });
            sleep(Duration::from_millis(20)).await;
            update_venue_runtime(&timed_out, "mt5:cfd", |runtime| {
                runtime.scheduler_phase = "AccountTimeout".to_string();
                runtime.last_error = Some("simulated timeout".to_string());
                runtime.consecutive_failures = 1;
            });
        });
        let healthy_task = tokio::spawn(async move {
            update_venue_runtime(&healthy, "binance:testnet", |runtime| {
                runtime.scheduler_phase = "AccountReady".to_string();
                runtime.scheduler_cycle_completed_epoch = 42;
            });
        });

        timeout_task.await.unwrap();
        healthy_task.await.unwrap();

        let runtimes = runtimes.read().unwrap();
        let mt5 = runtimes.get("mt5:cfd").unwrap();
        let binance = runtimes.get("binance:testnet").unwrap();
        assert_eq!(mt5.scheduler_phase, "AccountTimeout");
        assert_eq!(mt5.consecutive_failures, 1);
        assert_eq!(mt5.last_error.as_deref(), Some("simulated timeout"));
        assert_eq!(binance.scheduler_phase, "AccountReady");
        assert_eq!(binance.scheduler_cycle_completed_epoch, 42);
        assert_eq!(binance.consecutive_failures, 0);
        assert!(binance.last_error.is_none());
    }

    #[test]
    fn balance_sync_is_disabled_for_dry_run() {
        assert!(!balance_sync_allowed(&TradingMode::Live, true));
        assert!(balance_sync_allowed(&TradingMode::Live, false));
        assert!(!balance_sync_allowed(&TradingMode::Paper, false));
    }
}

fn balance_sync_allowed(mode: &crate::core::model::TradingMode, dry_run: bool) -> bool {
    matches!(mode, crate::core::model::TradingMode::Live) && !dry_run
}

fn push_state_log_try(state: &Arc<Mutex<AppState>>, msg: String) {
    if let Ok(mut st) = state.try_lock() {
        st.push_log(msg);
    }
}

impl Engine {
    /// 💰 Periyodik hesap bakiye senkronu — Live mode için.
    ///
    /// İki katmanlı karar:
    ///   - Mismatch %1+ tek seferlik gözlem → ⚠️ uyarı (henüz onarım yok)
    ///   - Mismatch N kez (default 3) ardışık → 🩹 otomatik onarım (equity = borsa)
    /// Eşik altına döner dönmez sayaç sıfırlanır.
    pub(crate) fn spawn_balance_sync(state: Arc<Mutex<AppState>>) {
        tokio::spawn(async move {
            let interval_secs: u64 = env_parse("BALANCE_SYNC_EVERY_SECS", 300);
            let account_timeout_secs: u64 = env_parse("BALANCE_SYNC_TIMEOUT_SECS", 30).max(1);
            let mismatch_pct_threshold: f64 = env_parse("BALANCE_MISMATCH_PCT", 1.0);
            // Otomatik onarım için ardışık gözlem eşiği. 0 → autofix kapalı.
            let autofix_after_n: u32 = env_parse("BALANCE_AUTOFIX_AFTER_N_OBS", 3);
            let autofix_enabled: bool = std::env::var("BALANCE_AUTOFIX_ENABLED")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true);

            // Sadece Live + non-dry-run modunda çalış
            let (balance_venues, active_profile, venue_runtime, lock_diagnostic, sync_allowed) = {
                let st = match state.lock() {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let active_profile = st.config.venue_profile;
                let balance_venues = st
                    .config
                    .venues
                    .iter()
                    .filter(|spec| spec.profile == active_profile)
                    .filter_map(|spec| st.venue_registry.get_spec(*spec))
                    // Hesap/pozisyon gözlemi emir yürütme yeteneğinden bağımsızdır.
                    // Özellikle IBKR read-only modunda execution_engine None olsa da
                    // account_snapshot() ile manuel işlemler izlenebilmelidir.
                    .cloned()
                    .collect::<Vec<_>>();
                let read_only_account_observation = std::env::var("IBKR_READ_ONLY")
                    .map(|value| {
                        matches!(
                            value.trim().to_ascii_lowercase().as_str(),
                            "1" | "true" | "yes" | "on"
                        )
                    })
                    .unwrap_or(false)
                    && balance_venues
                        .iter()
                        .any(|venue| venue.exchange() == crate::core::types::Exchange::Ibkr);
                let sync_allowed = balance_sync_allowed(&st.config.trading_mode, st.live_dry_run)
                    || read_only_account_observation;
                (
                    balance_venues,
                    active_profile,
                    Arc::clone(&st.venue_runtime),
                    Arc::clone(&st.runtime_lock_diagnostic),
                    sync_allowed,
                )
            };
            if balance_venues.is_empty() {
                if let Ok(mut runtimes) = venue_runtime.write() {
                    for runtime in runtimes.values_mut() {
                        if runtime.scheduler_phase == "NotStarted" {
                            runtime.scheduler_phase = "Unavailable".to_string();
                            runtime.last_error =
                                Some("aktif profile için venue bulunamadı".to_string());
                        }
                    }
                }
                push_state_log(
                    &state,
                    "💰 Balance sync: aktif profile için account gözlem capability yok (venue bulunamadı); scheduler pasif"
                        .into(),
                );
                return;
            }
            if !sync_allowed {
                if let Ok(mut runtimes) = venue_runtime.write() {
                    for runtime in runtimes.values_mut() {
                        if runtime.scheduler_phase == "NotStarted" {
                            runtime.scheduler_phase = "DryRun".to_string();
                            runtime.last_error = None;
                        }
                    }
                }
                push_state_log(
                    &state,
                    "💰 Balance sync: dry-run profili; hesap API gözlemi atlandı".into(),
                );
                return;
            }

            // İlk turda 30 sn warmup (boot anomalilerinden kaçınmak için)
            sleep(Duration::from_secs(30)).await;

            let mut consecutive_mismatch: u32 = 0;

            loop {
                let sync_started = std::time::Instant::now();
                let lock_intent = "balance_sync:stop";
                crate::robot::infra::app_state_lock_watchdog::set_intent(lock_intent);
                let stop = match state.try_lock() {
                    Ok(s) => Some(s.app_stop_signal.load(Ordering::Relaxed)),
                    Err(_) => None,
                };
                crate::robot::infra::app_state_lock_watchdog::clear_intent(lock_intent);
                let Some(stop) = stop else {
                    sleep(Duration::from_secs(interval_secs)).await;
                    continue;
                };
                if stop {
                    break;
                }

                let mut balance_total: Option<f64> = None;
                let mut aggregate_currency: Option<String> = None;
                let mut balance_errors = Vec::new();
                push_state_log_try(
                    &state,
                    format!(
                        "💰 [BALANCE-SYNC] cycle başladı: profile={} venue_count={}",
                        active_profile.as_str(),
                        balance_venues.len()
                    ),
                );
                for venue in &balance_venues {
                    let venue_key = venue.name();
                    let cycle_epoch = crate::core::time::now_epoch_secs() as i64;
                    if let Ok(mut diagnostic) = lock_diagnostic.write() {
                        diagnostic.owner = "balance_sync".to_string();
                        diagnostic.phase = format!("account_snapshot:{venue_key}");
                        diagnostic.started_epoch = cycle_epoch;
                    }
                    update_venue_runtime(&venue_runtime, &venue_key, |runtime| {
                        runtime.scheduler_phase = "AccountSync".to_string();
                        runtime.scheduler_cycle_started_epoch = cycle_epoch;
                    });
                    let venue_started = std::time::Instant::now();
                    let account_snapshot = tokio::time::timeout(
                        Duration::from_secs(account_timeout_secs),
                        venue.account_snapshot(),
                    )
                    .await;
                    match account_snapshot {
                        Err(_) => {
                            let error = format!(
                                "{}:{} account snapshot zaman aşımı ({}s)",
                                venue.exchange().as_str(),
                                venue.market().as_str(),
                                account_timeout_secs
                            );
                            update_venue_runtime(&venue_runtime, &venue_key, |runtime| {
                                runtime.scheduler_phase = "AccountTimeout".to_string();
                                runtime.last_error = Some(error.clone());
                                runtime.consecutive_failures =
                                    runtime.consecutive_failures.saturating_add(1);
                            });
                            balance_errors.push(error);
                        }
                        Ok(Err(error)) => {
                            let message = format!(
                                "{}:{} {:?} süre={}ms",
                                venue.exchange().as_str(),
                                venue.market().as_str(),
                                error,
                                venue_started.elapsed().as_millis()
                            );
                            update_venue_runtime(&venue_runtime, &venue_key, |runtime| {
                                runtime.scheduler_phase = "AccountError".to_string();
                                runtime.last_error = Some(message.clone());
                                runtime.consecutive_failures =
                                    runtime.consecutive_failures.saturating_add(1);
                            });
                            balance_errors.push(message);
                        }
                        Ok(Ok(snapshot))
                            if snapshot.balance.is_finite() && snapshot.balance >= 0.0 =>
                        {
                            let equity = snapshot.equity;
                            let currency = snapshot.currency.clone();
                            update_venue_runtime(&venue_runtime, &venue_key, |runtime| {
                                runtime.account_snapshot = Some(snapshot.clone());
                                runtime.last_error = None;
                                runtime.last_sync_epoch =
                                    crate::core::time::now_epoch_secs() as i64;
                                runtime.scheduler_phase = "AccountReady".to_string();
                                runtime.scheduler_cycle_completed_epoch =
                                    crate::core::time::now_epoch_secs() as i64;
                                runtime.consecutive_failures = 0;
                            });
                            // Sadece taze, raporlanmış para birimi bilinen ve aynı
                            // para birimindeki equity değerleri matematiksel olarak toplanır.
                            if snapshot.status == memos_venue_api::AccountSnapshotStatus::Fresh {
                                if let (Some(ccy), Some(eq)) = (currency.as_deref(), equity) {
                                    if eq.is_finite() && !ccy.is_empty() {
                                        let compatible = aggregate_currency
                                            .as_deref()
                                            .map(|current| current == ccy)
                                            .unwrap_or(true);
                                        if compatible {
                                            aggregate_currency
                                                .get_or_insert_with(|| ccy.to_string());
                                            *balance_total.get_or_insert(0.0) += eq;
                                        }
                                    }
                                }
                            }
                            push_state_log_try(
                                &state,
                                format!(
                                    "💰 [BALANCE-SYNC] {}:{} balance={:.2} equity={:?} currency={:?}",
                                    venue.exchange().as_str(),
                                    venue.market().as_str(),
                                    snapshot.balance,
                                    equity,
                                    currency
                                ),
                            );
                            push_state_log_try(
                                &state,
                                format!(
                                    "✅ [BALANCE-SYNC] {}:{} tamamlandı süre={}ms",
                                    venue.exchange().as_str(),
                                    venue.market().as_str(),
                                    venue_started.elapsed().as_millis()
                                ),
                            );
                        }
                        Ok(Ok(snapshot)) => {
                            let error = format!(
                                "{}:{} geçersiz balance={} süre={}ms",
                                venue.exchange().as_str(),
                                venue.market().as_str(),
                                snapshot.balance,
                                venue_started.elapsed().as_millis()
                            );
                            update_venue_runtime(&venue_runtime, &venue_key, |runtime| {
                                runtime.scheduler_phase = "InvalidAccount".to_string();
                                runtime.last_error = Some(error.clone());
                                runtime.consecutive_failures =
                                    runtime.consecutive_failures.saturating_add(1);
                            });
                            balance_errors.push(error);
                        }
                    }
                }

                if !balance_errors.is_empty() && balance_total.is_none() {
                    push_state_log_try(
                        &state,
                        format!(
                            "⚠️ [BALANCE-SYNC] tüm venue balance çağrıları başarısız: {}",
                            balance_errors.join(" | ")
                        ),
                    );
                    // Hata yolundaki `continue`, normal döngü sonundaki beklemeyi
                    // atlar; bağlantı yokken log ve bridge çağrısı fırtınasını önle.
                    sleep(Duration::from_secs(interval_secs)).await;
                    continue;
                }

                if !balance_errors.is_empty() {
                    push_state_log_try(
                        &state,
                        format!(
                            "⚠️ [BALANCE-SYNC] kısmi balance sonucu: {}",
                            balance_errors.join(" | ")
                        ),
                    );
                }

                match balance_total {
                    Some(exchange_balance) => {
                        let local_equity =
                            state.try_lock().map(|s| s.finance.equity).unwrap_or(0.0);
                        let diff = (exchange_balance - local_equity).abs();
                        let pct = if local_equity.abs() > f64::EPSILON {
                            (diff / local_equity) * 100.0
                        } else {
                            0.0
                        };

                        if pct > mismatch_pct_threshold {
                            // Eşik aşıldı → mismatch sayacı bir artar
                            consecutive_mismatch = consecutive_mismatch.saturating_add(1);

                            // Önce uyarı log'u + Telegram (BALANCE-MISMATCH key ile throttle)
                            if let Ok(mut st) = state.try_lock() {
                                st.push_alert(
                                    "BALANCE-MISMATCH",
                                    crate::robot::infra::telegram_notifier::Severity::Warning,
                                    format!(
                                        "[BALANCE-MISMATCH] borsa=${:.2} local=${:.2} fark=${:.2} ({:.2}%) > {:.2}% (gözlem #{} / {})",
                                        exchange_balance, local_equity, diff, pct, mismatch_pct_threshold,
                                        consecutive_mismatch, autofix_after_n,
                                    ),
                                );
                                st.guardian.repair_log.push_back(format!(
                                    "[{}] mismatch obs#{}: exchange=${:.2} local=${:.2} ({:.2}%)",
                                    chrono::Local::now().format("%H:%M:%S"),
                                    consecutive_mismatch,
                                    exchange_balance,
                                    local_equity,
                                    pct,
                                ));
                                while st.guardian.repair_log.len() > 100 {
                                    st.guardian.repair_log.pop_front();
                                }
                            }

                            // Autofix tetikleyici: N ardışık gözlem
                            if autofix_enabled
                                && autofix_after_n > 0
                                && consecutive_mismatch >= autofix_after_n
                            {
                                // Otomatik onarım: local equity'yi borsaya hizala
                                if let Ok(mut st) = state.try_lock() {
                                    let old_equity = st.finance.equity;
                                    let delta = exchange_balance - old_equity;
                                    st.finance.equity = exchange_balance;
                                    // peak_equity revize: yeni equity peak'in üzerindeyse güncelle
                                    if exchange_balance > st.finance.peak_equity {
                                        st.finance.peak_equity = exchange_balance;
                                    }
                                    st.push_alert(
                                        "BALANCE-AUTOFIX",
                                        crate::robot::infra::telegram_notifier::Severity::Critical,
                                        format!(
                                            "[BALANCE-AUTOFIX] {} ardışık mismatch sonrası onarım: ${:.2} → ${:.2} (Δ={:+.2})",
                                            consecutive_mismatch, old_equity, exchange_balance, delta,
                                        ),
                                    );
                                    st.guardian.repair_log.push_back(format!(
                                        "[{}] AUTOFIX: equity ${:.2} → ${:.2} (Δ={:+.2})",
                                        chrono::Local::now().format("%H:%M:%S"),
                                        old_equity,
                                        exchange_balance,
                                        delta,
                                    ));
                                    while st.guardian.repair_log.len() > 100 {
                                        st.guardian.repair_log.pop_front();
                                    }
                                }
                                consecutive_mismatch = 0; // sayaç reset
                            }
                        } else {
                            // Eşik altına düştü → sayacı toparla
                            if consecutive_mismatch > 0 {
                                push_state_log_try(&state, format!(
                                    "💰 [BALANCE-SYNC] mismatch toparlandı (sayaç sıfırlandı): borsa=${:.2} ≈ local=${:.2}",
                                    exchange_balance, local_equity,
                                ));
                            } else if let Ok(mut st) = state.try_lock() {
                                st.push_log(format!(
                                    "💰 [BALANCE-SYNC] borsa=${:.2} ≈ local=${:.2} (fark {:.2}%, eşik altı)",
                                    exchange_balance, local_equity, pct,
                                ));
                            }
                            consecutive_mismatch = 0;
                        }
                    }
                    None => {
                        push_state_log_try(
                            &state,
                            "⚠️ [BALANCE-SYNC] uyumlu aynı-para-birimi taze equity yok; toplam kullanılamıyor".into(),
                        );
                    }
                }
                push_state_log_try(
                    &state,
                    format!(
                        "🏁 [BALANCE-SYNC] cycle tamamlandı: profile={} toplam={:?} {} süre={}ms",
                        active_profile.as_str(),
                        balance_total,
                        aggregate_currency.as_deref().unwrap_or("UNKNOWN"),
                        sync_started.elapsed().as_millis()
                    ),
                );

                sleep(Duration::from_secs(interval_secs)).await;
            }
        });
    }

    /// 🛰️ WebSocket userDataStream task'ı — Live mode fill event'leri için.
    pub(crate) fn spawn_user_data_stream(state: Arc<Mutex<AppState>>) {
        let (lifecycle, dry_run, mode) = {
            let st = match state.lock() {
                Ok(s) => s,
                Err(_) => return,
            };
            (
                st.live_venue_lifecycle.clone(),
                st.live_dry_run,
                st.config.trading_mode.clone(),
            )
        };
        if !crate::robot::venue::should_start_live_lifecycle(&mode, dry_run, lifecycle.is_some()) {
            push_state_log(
                &state,
                "🛰️ WS userDataStream: Paper/DryRun mod, task pasif".into(),
            );
            return;
        }
        let lifecycle = match lifecycle {
            Some(lifecycle) => lifecycle,
            None => {
                push_state_log(
                    &state,
                    "🛰️ WS userDataStream: canlı venue seçilmedi, task pasif".into(),
                );
                return;
            }
        };
        let executor = match lifecycle.binance_user_data() {
            Some(executor) => executor,
            None => {
                push_state_log(
                    &state,
                    "🛰️ WS userDataStream: venue user-data capability yok, task pasif".into(),
                );
                return;
            }
        };

        tokio::spawn(async move {
            use futures::StreamExt;
            use tokio_tungstenite::{connect_async, tungstenite::Message};

            // Reconnect döngüsü
            let mut backoff_secs: u64 = 5;
            loop {
                let stop = state
                    .lock()
                    .map(|s| s.app_stop_signal.load(Ordering::Relaxed))
                    .unwrap_or(true);
                if stop {
                    break;
                }

                // 1. listenKey al
                let listen_key = match executor.create_listen_key().await {
                    Ok(k) => k,
                    Err(e) => {
                        push_state_log(
                            &state,
                            format!(
                                "🛰️ WS listenKey hatası: {:?} (backoff={}s)",
                                e, backoff_secs,
                            ),
                        );
                        sleep(Duration::from_secs(backoff_secs)).await;
                        backoff_secs = (backoff_secs * 2).min(60);
                        continue;
                    }
                };
                let ws_url = executor.user_data_stream_url(&listen_key);
                push_state_log(
                    &state,
                    format!("🛰️ WS userDataStream bağlanıyor: {}", ws_url),
                );

                // 2. WS bağlan
                let (ws_stream, _) = match connect_async(&ws_url).await {
                    Ok(p) => p,
                    Err(e) => {
                        if let Ok(mut st) = state.lock() {
                            st.push_alert(
                                "WS-CONNECT-FAIL",
                                crate::robot::infra::telegram_notifier::Severity::Warning,
                                format!(
                                    "[WS-CONNECT-FAIL] userDataStream bağlanılamadı: {:?} (backoff={}s)",
                                    e, backoff_secs,
                                ),
                            );
                        }
                        sleep(Duration::from_secs(backoff_secs)).await;
                        backoff_secs = (backoff_secs * 2).min(60);
                        continue;
                    }
                };
                push_state_log(
                    &state,
                    "🛰️ WS userDataStream bağlı ✓ — fill event'leri dinleniyor".into(),
                );
                backoff_secs = 5; // başarılı bağlantı, backoff reset

                // 3. Keepalive timer (30 dk'da bir listenKey yenile)
                let ka_exec = Arc::clone(&executor);
                let ka_state = Arc::clone(&state);
                let ka_key = listen_key.clone();
                let keepalive_handle = tokio::spawn(async move {
                    loop {
                        sleep(Duration::from_secs(30 * 60)).await;
                        let stop = ka_state
                            .lock()
                            .map(|s| s.app_stop_signal.load(Ordering::Relaxed))
                            .unwrap_or(true);
                        if stop {
                            break;
                        }
                        if let Err(e) = ka_exec.keepalive_listen_key(&ka_key).await {
                            if let Ok(mut st) = ka_state.lock() {
                                st.push_log(format!("🛰️ WS keepalive hatası: {:?}", e));
                            }
                            break;
                        }
                    }
                });

                // 4. Mesaj döngüsü
                let (_write, mut read) = ws_stream.split();
                while let Some(msg) = read.next().await {
                    let stop = state
                        .lock()
                        .map(|s| s.app_stop_signal.load(Ordering::Relaxed))
                        .unwrap_or(true);
                    if stop {
                        break;
                    }
                    match msg {
                        Ok(Message::Text(txt)) => {
                            Self::handle_user_data_event(&state, &txt).await;
                        }
                        Ok(Message::Ping(p)) => {
                            let _ = p; /* yanıt tungstenite tarafında otomatik */
                        }
                        Ok(Message::Close(_)) => {
                            push_state_log(
                                &state,
                                "🛰️ WS sunucu Close gönderdi — yeniden bağlanılacak".into(),
                            );
                            break;
                        }
                        Err(e) => {
                            push_state_log(
                                &state,
                                format!("🛰️ WS okuma hatası: {:?} — reconnect", e),
                            );
                            break;
                        }
                        _ => {}
                    }
                }

                keepalive_handle.abort();
                sleep(Duration::from_secs(backoff_secs)).await;
                backoff_secs = (backoff_secs * 2).min(60);
            }
        });
    }
}
