// src/robot/engines/master/fleet_sync.rs — Canlı borsa senkron task'ları:
// hesap bakiye senkronu + WebSocket userDataStream.
// Faz 2 modülerleştirme: infra_fleet.rs'ten ayrıldı (davranış birebir korunur).
use super::*;

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
            let mismatch_pct_threshold: f64 = env_parse("BALANCE_MISMATCH_PCT", 1.0);
            // Otomatik onarım için ardışık gözlem eşiği. 0 → autofix kapalı.
            let autofix_after_n: u32 = env_parse("BALANCE_AUTOFIX_AFTER_N_OBS", 3);
            let autofix_enabled: bool = std::env::var("BALANCE_AUTOFIX_ENABLED")
                .map(|v| v != "false" && v != "0").unwrap_or(true);

            // Sadece Live + non-dry-run modunda çalış
            let (executor, dry_run) = {
                let st = match state.lock() { Ok(s) => s, Err(_) => return };
                (st.live_executor.clone(), st.live_dry_run)
            };
            let executor = match executor {
                Some(e) if !dry_run => e,
                _ => {
                    push_state_log(&state, "💰 Balance sync: Paper/DryRun mod, task pasif".into());
                    return;
                }
            };

            // İlk turda 30 sn warmup (boot anomalilerinden kaçınmak için)
            sleep(Duration::from_secs(30)).await;

            let mut consecutive_mismatch: u32 = 0;

            loop {
                let stop = state.lock().map(|s| s.app_stop_signal.load(Ordering::Relaxed)).unwrap_or(true);
                if stop { break; }

                match executor.get_balance().await {
                    Ok(exchange_balance) => {
                        let local_equity = state.lock().map(|s| s.finance.equity).unwrap_or(0.0);
                        let diff = (exchange_balance - local_equity).abs();
                        let pct = if local_equity.abs() > f64::EPSILON {
                            (diff / local_equity) * 100.0
                        } else { 0.0 };

                        if pct > mismatch_pct_threshold {
                            // Eşik aşıldı → mismatch sayacı bir artar
                            consecutive_mismatch = consecutive_mismatch.saturating_add(1);

                            // Önce uyarı log'u + Telegram (BALANCE-MISMATCH key ile throttle)
                            if let Ok(mut st) = state.lock() {
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
                                    consecutive_mismatch, exchange_balance, local_equity, pct,
                                ));
                                while st.guardian.repair_log.len() > 100 { st.guardian.repair_log.pop_front(); }
                            }

                            // Autofix tetikleyici: N ardışık gözlem
                            if autofix_enabled && autofix_after_n > 0 && consecutive_mismatch >= autofix_after_n {
                                // Otomatik onarım: local equity'yi borsaya hizala
                                if let Ok(mut st) = state.lock() {
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
                                        old_equity, exchange_balance, delta,
                                    ));
                                    while st.guardian.repair_log.len() > 100 { st.guardian.repair_log.pop_front(); }
                                }
                                consecutive_mismatch = 0; // sayaç reset
                            }
                        } else {
                            // Eşik altına düştü → sayacı toparla
                            if consecutive_mismatch > 0 {
                                push_state_log(&state, format!(
                                    "💰 [BALANCE-SYNC] mismatch toparlandı (sayaç sıfırlandı): borsa=${:.2} ≈ local=${:.2}",
                                    exchange_balance, local_equity,
                                ));
                            } else if let Ok(mut st) = state.lock() {
                                st.push_log(format!(
                                    "💰 [BALANCE-SYNC] borsa=${:.2} ≈ local=${:.2} (fark {:.2}%, eşik altı)",
                                    exchange_balance, local_equity, pct,
                                ));
                            }
                            consecutive_mismatch = 0;
                        }
                    }
                    Err(e) => {
                        push_state_log(&state, format!("⚠️ [BALANCE-SYNC] get_balance hatası: {:?}", e));
                    }
                }

                sleep(Duration::from_secs(interval_secs)).await;
            }
        });
    }

    /// 🛰️ WebSocket userDataStream task'ı — Live mode fill event'leri için.
    pub(crate) fn spawn_user_data_stream(state: Arc<Mutex<AppState>>) {
        tokio::spawn(async move {
            use futures::StreamExt;
            use tokio_tungstenite::{connect_async, tungstenite::Message};

            // Sadece Live + non-dry-run modunda çalış
            let (executor, dry_run) = {
                let st = match state.lock() { Ok(s) => s, Err(_) => return };
                (st.live_executor.clone(), st.live_dry_run)
            };
            let executor = match executor {
                Some(e) if !dry_run => e,
                _ => {
                    push_state_log(&state, "🛰️ WS userDataStream: Paper/DryRun mod, task pasif".into());
                    return;
                }
            };

            // Reconnect döngüsü
            let mut backoff_secs: u64 = 5;
            loop {
                let stop = state.lock().map(|s| s.app_stop_signal.load(Ordering::Relaxed)).unwrap_or(true);
                if stop { break; }

                // 1. listenKey al
                let listen_key = match executor.create_listen_key().await {
                    Ok(k) => k,
                    Err(e) => {
                        push_state_log(&state, format!(
                            "🛰️ WS listenKey hatası: {:?} (backoff={}s)", e, backoff_secs,
                        ));
                        sleep(Duration::from_secs(backoff_secs)).await;
                        backoff_secs = (backoff_secs * 2).min(60);
                        continue;
                    }
                };
                let ws_url = executor.user_data_stream_url(&listen_key);
                push_state_log(&state, format!("🛰️ WS userDataStream bağlanıyor: {}", ws_url));

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
                push_state_log(&state, "🛰️ WS userDataStream bağlı ✓ — fill event'leri dinleniyor".into());
                backoff_secs = 5; // başarılı bağlantı, backoff reset

                // 3. Keepalive timer (30 dk'da bir listenKey yenile)
                let ka_exec = Arc::clone(&executor);
                let ka_state = Arc::clone(&state);
                let ka_key = listen_key.clone();
                let keepalive_handle = tokio::spawn(async move {
                    loop {
                        sleep(Duration::from_secs(30 * 60)).await;
                        let stop = ka_state.lock().map(|s| s.app_stop_signal.load(Ordering::Relaxed)).unwrap_or(true);
                        if stop { break; }
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
                    let stop = state.lock().map(|s| s.app_stop_signal.load(Ordering::Relaxed)).unwrap_or(true);
                    if stop { break; }
                    match msg {
                        Ok(Message::Text(txt)) => {
                            Self::handle_user_data_event(&state, &txt).await;
                        }
                        Ok(Message::Ping(p)) => { let _ = p; /* yanıt tungstenite tarafında otomatik */ }
                        Ok(Message::Close(_)) => {
                            push_state_log(&state, "🛰️ WS sunucu Close gönderdi — yeniden bağlanılacak".into());
                            break;
                        }
                        Err(e) => {
                            push_state_log(&state, format!("🛰️ WS okuma hatası: {:?} — reconnect", e));
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
