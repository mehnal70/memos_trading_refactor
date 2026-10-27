// src/robot/engines/master/loop_core.rs — Otonom döngü + cycle orkestrasyonu.
// Faz 2 modülerleştirme: edge/rejim yardımcıları edge_regime.rs'e ayrıldı (davranış birebir).
use super::*;

impl Engine {
    /// 🚀 ANA OTONOM DÖNGÜ (Engine Girişi)
    pub async fn run_autonomous_loop(state: Arc<Mutex<AppState>>) {
        log::info!("🚀 Master Engine Ateşlendi. Otonom devriye başlatıldı.");
        if let Ok(mut st) = state.lock() {
            st.fleet.phase = "Booting".into();
            st.push_log("🚀 Master Engine ateşlendi. Otonom devriye başladı.".into());
            let venue_status = st
                .config
                .venues
                .iter()
                .filter_map(|spec| {
                    st.venue_registry.get_spec(*spec).map(|venue| {
                        format!(
                            "{} profile={} data=ok execution={} market={}",
                            spec.exchange.as_str(),
                            spec.profile.as_str(),
                            if venue.execution_engine().is_some() {
                                "ok"
                            } else {
                                "yok"
                            },
                            spec.market.as_str()
                        )
                    })
                })
                .collect::<Vec<_>>();
            let configured_venues = st.config.venues.len();
            let registered_venues = st.venue_registry.len();
            st.push_log(format!(
                "🏛️ Venue başlangıç kontrolü: configured={} registered={} · {}",
                configured_venues,
                registered_venues,
                if venue_status.is_empty() {
                    "aktif venue yok".to_string()
                } else {
                    venue_status.join(" | ")
                }
            ));
            if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                pipe.init_canon_stages();
            }
        }

        // Balance sync kendi capability kararını verebilir; uzun altyapı başlangıcını beklemesin.
        Self::spawn_balance_sync(Arc::clone(&state));
        log::info!("🚀 Master Engine başlangıç aşaması tamamlandı: balance sync");
        tokio::task::yield_now().await;

        push_state_log(
            &state,
            "🔧 Sıralı başlangıç: SQLite schema doğrulaması başlıyor".into(),
        );
        Self::ensure_db_schema(&state);
        push_state_log(
            &state,
            "✅ Sıralı başlangıç: SQLite schema doğrulaması tamamlandı".into(),
        );

        push_state_log(
            &state,
            "🔄 Sıralı başlangıç: position/account recovery başlıyor".into(),
        );
        let recovery_state = Arc::clone(&state);
        tokio::task::spawn_blocking(move || {
            Self::hydrate_open_positions_from_db(&recovery_state);
            Self::hydrate_account_state_from_db(&recovery_state);
            Self::hydrate_uncertain_orders_from_db(&recovery_state);
            Self::hydrate_pending_closes_from_db(&recovery_state);
            Self::hydrate_symbol_status_from_db(&recovery_state);
            Self::prune_seed_ineligible(&recovery_state);
            Self::report_edge_seed(&recovery_state);
            push_state_log(
                &recovery_state,
                "✅ Sıralı başlangıç: local position/account recovery tamamlandı".into(),
            );
        });

        Self::spawn_infrastructure_fleet(Arc::clone(&state)).await;
        log::info!("🚀 Master Engine başlangıç aşaması tamamlandı: altyapı filosu");
        Self::spawn_user_data_stream(Arc::clone(&state));
        log::info!("🚀 Master Engine başlangıç aşaması tamamlandı: user-data stream");
        let reconcile_state = Arc::clone(&state);
        tokio::spawn(async move {
            push_state_log(
                &reconcile_state,
                "🔄 Arka plan başlangıç venue reconcile başladı".into(),
            );
            Self::reconcile_live_positions_with_exchange(&reconcile_state).await;
            push_state_log(
                &reconcile_state,
                "✅ Arka plan başlangıç venue reconcile tamamlandı".into(),
            );
        });
        log::info!("🚀 Master Engine ana devriye döngüsüne giriyor");

        let heartbeat_log_disabled = env_truthy("HEARTBEAT_UI_LOG_DISABLE");
        let heartbeat_log_ticks: u64 = std::env::var("HEARTBEAT_UI_LOG_TICKS")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n| *n > 0)
            .unwrap_or(600);
        let mut tick_count: u64 = 0;
        let mut startup_lock_probe = true;
        loop {
            let now = crate::core::time::now_epoch_secs();
            if state.try_lock().is_err() {
                if startup_lock_probe {
                    log::warn!("⚠️ Master Engine ana döngüsü AppState kilidini alamıyor; başlangıç task'ı kilit tutuyor");
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
            if startup_lock_probe {
                log::info!("✅ Master Engine ana döngüsü AppState kilidini aldı");
                startup_lock_probe = false;
            }
            let is_stop = {
                let mut st = match state.try_lock() {
                    Ok(st) => st,
                    Err(std::sync::TryLockError::Poisoned(poison)) => poison.into_inner(),
                    Err(std::sync::TryLockError::WouldBlock) => continue,
                };
                st.fleet.last_loop_tick.store(now, Ordering::Relaxed);
                st.fleet.phase = "Scanning".into();
                st.app_stop_signal.load(Ordering::Relaxed)
            };
            if is_stop {
                break;
            }
            tick_count += 1;
            if tick_count == 1 {
                log::info!("✅ Master Engine ilk trade cycle başlıyor");
            }

            crate::robot::infra::app_state_lock_watchdog::set_intent(
                "master_loop:execute_trade_cycle",
            );
            // MT5 sağlık SONDASI — artık cycle'ı BLOKLAMAZ. Eskiden `continue` vardı:
            // MT5 köprüsü sağlıksızken (EA kapalı/bağlanmadı) TÜM motor duruyordu —
            // Binance sembolleri dahil hiçbir şey işlenmiyor, heartbeat düşmüyordu
            // (3 entegrasyon testinin 15-20s timeout sebebi). Tek venue'nun sağlığı
            // global kill-switch olamaz. Sonda `mt5_bridge_healthy` bayrağını ve
            // alert'i güncellemeye devam eder; MT5'e route edilen ADAYLAR
            // execute_trade_cycle içinde sembol-düzeyinde atlanır.
            if state.try_lock().is_err() {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
            let snap = {
                match state.try_lock() {
                    Ok(st) => crate::core::bridge::get_snapshot(&st),
                    Err(std::sync::TryLockError::Poisoned(poison)) => {
                        crate::core::bridge::get_snapshot(&poison.into_inner())
                    }
                    Err(std::sync::TryLockError::WouldBlock) => continue,
                }
            };

            let cycle_started = std::time::Instant::now();
            Self::execute_trade_cycle(&state, &snap).await;
            crate::robot::infra::app_state_lock_watchdog::clear_intent(
                "master_loop:execute_trade_cycle",
            );
            let cycle_elapsed_ms = cycle_started.elapsed().as_millis();
            if tick_count == 1 {
                log::info!(
                    "✅ Master Engine ilk trade cycle tamamlandı ({}ms)",
                    cycle_elapsed_ms
                );
            }

            if tick_count.is_multiple_of(60) {
                let max_age: u64 = env_parse("ANOMALY_MAX_AGE_SECS", 1800);
                let now_secs = crate::core::time::now_epoch_secs();
                if let Ok(st) = state.lock() {
                    if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                        let n_purged = pipe.purge_stale_warnings(now_secs, max_age);
                        if n_purged > 0 {
                            log::info!(
                                "anomaly purge: {} stale Warning silindi (>{}sn)",
                                n_purged,
                                max_age
                            );
                        }
                    }
                }
            }
            let recovery_started = std::time::Instant::now();
            Self::perform_anomaly_recovery(&state, &snap);
            let recovery_elapsed_ms = recovery_started.elapsed().as_millis();

            if tick_count.is_multiple_of(5) {
                if let Ok(mut st) = state.lock() {
                    let equity = st.finance.equity;
                    if equity > st.finance.peak_equity {
                        st.finance.peak_equity = equity;
                    }
                    if let Ok(mut hist) = st.finance.equity_history.write() {
                        hist.push_back(equity);
                        while hist.len() > 120 {
                            hist.pop_front();
                        }
                    }
                }
            }

            if tick_count.is_multiple_of(20) {
                Self::tick_intelligence_hub(&state).await;

                // 2. Adım: 🛡️ OTONOM EMİR VE TUI SNAPSHOT SENKRONİZASYON TETİĞİ
                // Her ~10 saniyede bir (20 cycle × 500ms) tüm borsaları otonom tarayarak
                // askıdaki emir defterini Mission Control paneline dürüstçe yansıtır.
                let orders_started = std::time::Instant::now();
                Self::reconcile_live_open_orders_with_exchange(&state).await;
                let orders_elapsed_ms = orders_started.elapsed().as_millis();
                let positions_started = std::time::Instant::now();
                Self::reconcile_live_positions_with_exchange(&state).await; // 🎯 KODUN BAĞLANACAĞI KANCA BURASIDIR!
                let positions_elapsed_ms = positions_started.elapsed().as_millis();
                let closed_started = std::time::Instant::now();
                Self::reconcile_live_closed_trades_with_exchange(&state).await;
                let closed_elapsed_ms = closed_started.elapsed().as_millis();
                push_state_log(
                    &state,
                    format!(
                        "⏱️ Pipeline ölçüm #{}: cycle={}ms recovery={}ms orders={}ms positions={}ms closed={}ms",
                        tick_count,
                        cycle_elapsed_ms,
                        recovery_elapsed_ms,
                        orders_elapsed_ms,
                        positions_elapsed_ms,
                        closed_elapsed_ms,
                    ),
                );
            }

            if !heartbeat_log_disabled
                && (tick_count == 1 || tick_count.is_multiple_of(heartbeat_log_ticks))
            {
                if let Ok(mut st) = state.lock() {
                    let n_open = st
                        .finance
                        .live_positions
                        .read()
                        .map(|p| p.len())
                        .unwrap_or(0);
                    let n_closed = st
                        .finance
                        .live_closed_trades
                        .read()
                        .map(|t| t.len())
                        .unwrap_or(0);
                    let n_anom = st
                        .guardian
                        .live_pipeline
                        .read()
                        .map(|p| p.anomalies.len())
                        .unwrap_or(0);
                    let equity = st.finance.equity;
                    st.push_log(format!(
                        "💓 Devriye #{} | Equity: {:.2} | Açık: {} | Kapalı: {} | Anomali: {}",
                        tick_count, equity, n_open, n_closed, n_anom,
                    ));
                }
            }

            sleep(Duration::from_millis(500)).await;
        }

        if let Ok(mut st) = state.lock() {
            st.fleet.phase = "Stopped".into();
            st.push_log("🛑 Master Engine devriyesi durduruldu.".into());
        }
    }

    fn should_emit_bridge_alert(previous: Option<&str>, current: &str) -> bool {
        match previous {
            Some(prev) if prev == current => false,
            _ => true,
        }
    }

    // loop_core.rs içerisindeki gate_mt5_bridge_health fonksiyonunu şu şekilde tazeleyin:
    pub(crate) async fn gate_mt5_bridge_health(state: &Arc<Mutex<AppState>>) -> bool {
        let venue_registry = {
            let Ok(st) = state.try_lock() else {
                return true;
            };
            // Eğer config'de MT5 borsa spec'i tanımlı değilse mutabakatı doğrudan bypass et
            let has_mt5 = st
                .config
                .venues
                .iter()
                .any(|v| v.exchange == crate::core::types::Exchange::Mt5);
            if !has_mt5 {
                return true;
            }
            Arc::clone(&st.venue_registry)
        };

        // KESİN ÇÖZÜM: Sıfırdan `Mt5Bridge::with_defaults` denilerek port işgal EDİLMEZ.
        // AppState boot anında porta zaten dürüstçe bağlanmış olan paylaşımlı adaptör çekilir.
        let Some(venue) = venue_registry.get(crate::core::types::Exchange::Mt5) else {
            return true;
        };

        // Mevcut hat üzerinden hafif bir borsa emniyet filtre sorgusu tetiklenerek
        // soket trafiği yaratmadan hattın canlılığı pazar-agnostik test edilir.
        match tokio::time::timeout(std::time::Duration::from_secs(3), venue.balance()).await {
            Err(_) => {
                if let Ok(st) = state.try_lock() {
                    st.mt5_bridge_healthy.store(false, Ordering::Relaxed);
                }
                false
            }
            Ok(Ok(_)) => {
                if let Ok(st) = state.lock() {
                    st.mt5_bridge_healthy.store(true, Ordering::Relaxed);
                    if let Ok(mut err) = st.mt5_bridge_last_error.write() {
                        *err = None;
                    }
                }
                true
            }
            Ok(Err(err)) => {
                let err_text = err.to_string();

                // Eğer hata gerçek bir 'bilinmeyen cmd' veya hesap bakiyesi yanıtı ise bridge FİZİKSEL OLARAK CANLIDIR.
                // Sadece protokol kelimesi bekliyordur. Soket kopukluğu (Connection Refused) değilse true dön.
                let is_physically_alive =
                    err_text.contains("bilinmeyen cmd") || err_text.contains("balance");

                if is_physically_alive {
                    if let Ok(st) = state.lock() {
                        st.mt5_bridge_healthy.store(true, Ordering::Relaxed);
                    }
                    return true;
                }

                if let Ok(mut st) = state.lock() {
                    st.mt5_bridge_healthy.store(false, Ordering::Relaxed);
                    let previous = st.mt5_bridge_last_error.read().ok().and_then(|m| m.clone());
                    if let Ok(mut m) = st.mt5_bridge_last_error.write() {
                        *m = Some(err_text.clone());
                    }
                    if Self::should_emit_bridge_alert(previous.as_deref(), &err_text) {
                        st.push_alert(
                            "MT5-BRIDGE-HEALTH",
                            crate::robot::infra::telegram_notifier::Severity::Warning,
                            format!("MT5 bridge canlı değil: {}", err_text),
                        );
                    }
                }
                false
            }
        }
    }

    pub(crate) async fn execute_trade_cycle(state: &Arc<Mutex<AppState>>, snap: &MissionControl) {
        let ibkr_health_bridge = state
            .lock()
            .ok()
            .and_then(|st| st.ibkr_health_bridge.clone());
        if let Some(bridge) = ibkr_health_bridge {
            let health_result = bridge.farm_health().await;
            if let Ok(st) = state.lock() {
                match health_result {
                    Ok(health) => {
                        let healthy = health.connected
                            && health.market_data
                            && health.historical
                            && health.security_definition;
                        st.ibkr_farm_healthy.store(healthy, Ordering::Relaxed);
                        if healthy {
                            if let Ok(mut error) = st.ibkr_farm_last_error.write() {
                                *error = None;
                            }
                        } else if let Ok(mut error) = st.ibkr_farm_last_error.write() {
                            *error = Some(format!(
                                "IBKR farm sağlığı eksik: connected={} market_data={} historical={} security_definition={}",
                                health.connected,
                                health.market_data,
                                health.historical,
                                health.security_definition
                            ));
                        }
                    }
                    Err(error) => {
                        st.ibkr_farm_healthy.store(false, Ordering::Relaxed);
                        if let Ok(mut last_error) = st.ibkr_farm_last_error.write() {
                            *last_error = Some(error.to_string());
                        }
                    }
                }
            }
        }
        let (
            candidates,
            db_path,
            interval,
            symbol_interval,
            symbol_strategy,
            symbol_tracks,
            live_strategy,
            ml_confidence,
            tuning,
        ) = {
            let st = match state.lock() {
                Ok(s) => s,
                Err(_) => return,
            };
            let price_map = st
                .fleet
                .live_price
                .read()
                .ok()
                .map(|g| g.clone())
                .unwrap_or_default();
            if let Ok(mut positions) = st.finance.live_positions.write() {
                for pos in positions.values_mut() {
                    if let Some(&live) = price_map.get(&pos.symbol) {
                        if live > 0.0 {
                            pos.current_price = live;
                        }
                    }
                }
            }
            // Snapshot ve dashboard için canonical kopyayı anlık fiyatla eşitle.
            st.refresh_canonical_positions();
            let mt5_down = !st.mt5_bridge_healthy.load(Ordering::Relaxed)
                && st
                    .config
                    .venues
                    .iter()
                    .any(|v| v.exchange == crate::core::types::Exchange::Mt5);
            let ibkr_down = !st.ibkr_farm_healthy.load(Ordering::Relaxed)
                && st
                    .config
                    .venues
                    .iter()
                    .any(|v| v.exchange == crate::core::types::Exchange::Ibkr);
            let is_mt5_symbol = |sym: &str| {
                st.venue_registry
                    .route(sym)
                    .map(|(v, _)| v.exchange() == crate::core::types::Exchange::Mt5)
                    .unwrap_or(false)
            };
            let is_ibkr_symbol = |sym: &str| {
                st.venue_registry
                    .route(sym)
                    .map(|(v, _)| v.exchange() == crate::core::types::Exchange::Ibkr)
                    .unwrap_or(false)
            };
            let mut candidates = Vec::new();
            let xs_basket: std::collections::HashSet<String> = st
                .brain
                .parameters
                .read()
                .ok()
                .map(|p| {
                    let mut set: std::collections::HashSet<String> =
                        std::collections::HashSet::new();
                    if p.xs_live.enabled {
                        set.extend(p.xs_live.symbols.iter().cloned());
                    }
                    if p.carry_live.enabled {
                        set.extend(p.carry_live.symbols.iter().cloned());
                    }
                    if p.blend_live.enabled {
                        set.extend(p.blend_live.symbols.iter().cloned());
                    }
                    return set;
                })
                .unwrap_or_default();
            let tuning = Arc::clone(&st.tuning);
            if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                for worker in orch.get_worker_status() {
                    if !tuning.symbol_eligible_for_live(&worker.symbol) {
                        continue;
                    }
                    if xs_basket.contains(&worker.symbol) {
                        continue;
                    }
                    // MT5 köprüsü ölüyken MT5 adayı işlemek anlamsız (emir gidemez) —
                    // sembol-düzeyi atla; diğer venue'lar normal akar.
                    if mt5_down && is_mt5_symbol(&worker.symbol) {
                        if log_throttle_should_emit(&worker.symbol, "mt5_down_skip", 300) {
                            log::warn!(target: "MT5", "⛔ {} atlandı: MT5 köprüsü sağlıksız (yeni sinyal işlenmiyor; köprü gelince otomatik açılır)", worker.symbol);
                        }
                        continue;
                    }
                    if ibkr_down && is_ibkr_symbol(&worker.symbol) {
                        if log_throttle_should_emit(&worker.symbol, "ibkr_farm_down_skip", 300) {
                            log::warn!(
                                target: "IBKR",
                                "⛔ {} atlandı: IBKR farm sağlığı doğrulanamadı (yeni giriş işlenmiyor)",
                                worker.symbol
                            );
                        }
                        continue;
                    }
                    candidates.push(worker.symbol.clone());
                }
            }
            if let Ok(positions) = st.finance.live_positions.read() {
                for sym in positions.keys() {
                    if !tuning.symbol_eligible_for_live(sym) {
                        continue;
                    }
                    if xs_basket.contains(sym) {
                        continue;
                    }
                    if !candidates.contains(sym) {
                        candidates.push(sym.clone());
                    }
                }
            }
            let live_strategy = st
                .brain
                .live_strategy
                .read()
                .map(|s| s.clone())
                .unwrap_or_else(|_| "MA_CROSSOVER".to_string());
            let (symbol_interval, symbol_strategy, symbol_tracks) = st
                .brain
                .parameters
                .read()
                .ok()
                .map(|p| {
                    (
                        p.symbol_interval.clone(),
                        p.symbol_strategy.clone(),
                        p.symbol_tracks.clone(),
                    )
                })
                .unwrap_or_default();
            (
                candidates,
                st.config.db_path.clone(),
                st.config.interval.clone(),
                symbol_interval,
                symbol_strategy,
                symbol_tracks,
                live_strategy,
                st.brain.ml_confidence,
                tuning,
            )
        };
        Self::process_xs_book(state).await;
        Self::process_carry_book(state).await;
        Self::process_blend_book(state).await;

        // Ana döngü, her sembol için `JoinHandle` bekleyerek bloklanmamalıdır.
        // Aksi halde `execute_trade_cycle` içinde çalıştırılan çoklu sembol task'leri
        // 15s+ boyunca `last_loop_tick` güncellemesini durdurur ve `DataStall`
        // tetiklenir. Task'leri arka planda yayınlayıp main loop'un nabzını açık tutuyoruz.
        let cycle_task_cap: usize = std::env::var("TRADE_CYCLE_TASK_CAP")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(16)
            .max(1);

        let mut scheduled = 0usize;
        for symbol in candidates {
            if scheduled >= cycle_task_cap {
                if log_throttle_should_emit("global", "trade_cycle_task_cap", 300) {
                    push_state_log(state, format!(
                        "⏱️ trade_cycle task cap doldu: {} sembol planlandı, fazlası bu tur atlandı",
                        cycle_task_cap,
                    ));
                }
                break;
            }
            scheduled += 1;

            let state_clone = Arc::clone(state);
            let db_path_c = db_path.clone();
            let interval_c = symbol_interval
                .get(&symbol)
                .cloned()
                .unwrap_or_else(|| interval.clone());
            let seed_strat = symbol_strategy
                .get(&symbol)
                .cloned()
                .filter(|s| !s.trim().is_empty());
            let has_seed_strategy = seed_strat.is_some();
            let live_strategy_c = seed_strat.unwrap_or_else(|| live_strategy.clone());
            let tracks: Vec<(String, String)> =
                symbol_tracks.get(&symbol).cloned().unwrap_or_default();
            let snap_clone = snap.clone();
            let tuning_c = Arc::clone(&tuning);
            tokio::spawn(async move {
                if tracks.len() > 1 {
                    if symbol_has_open_position(&state_clone, &symbol) {
                        Self::process_symbol_cycle(
                            &state_clone,
                            &symbol,
                            &db_path_c,
                            &interval_c,
                            &live_strategy_c,
                            true,
                            ml_confidence,
                            &snap_clone,
                            &tuning_c,
                        )
                        .await;
                    } else {
                        for (iv, strat) in &tracks {
                            Self::process_symbol_cycle(
                                &state_clone,
                                &symbol,
                                &db_path_c,
                                iv,
                                strat,
                                true,
                                ml_confidence,
                                &snap_clone,
                                &tuning_c,
                            )
                            .await;
                            if symbol_has_open_position(&state_clone, &symbol) {
                                break;
                            }
                        }
                    }
                } else {
                    Self::process_symbol_cycle(
                        &state_clone,
                        &symbol,
                        &db_path_c,
                        &interval_c,
                        &live_strategy_c,
                        has_seed_strategy,
                        ml_confidence,
                        &snap_clone,
                        &tuning_c,
                    )
                    .await;
                }
            });
        }
    }

    pub(crate) async fn process_symbol_cycle(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        db_path: &str,
        interval: &str,
        live_strategy: &str,
        has_seed_strategy: bool,
        ml_confidence: f64,
        snap: &MissionControl,
        tuning: &RuntimeTuning,
    ) {
        use crate::robot::data_pipeline::canon::PipelineStage;
        use crate::robot::data_pipeline::StepStatus;
        let risk_manager = crate::robot::risk::RiskManager::new();

        {
            let candles = match Self::cycle_load_candles(state, symbol, db_path, interval, tuning) {
                Some(c) => c,
                None => return,
            };

            if Self::cycle_try_close_open_position(state, symbol, interval, &candles).await {
                return;
            }
            if symbol_has_open_position(state, symbol) {
                if let Ok(mut st) = state.lock() {
                    let trade_type = st
                        .finance
                        .live_positions
                        .read()
                        .ok()
                        .and_then(|p| p.get(symbol).map(|pos| pos.trade_type.clone()))
                        .unwrap_or_else(|| "generic".to_string());
                    Self::bump_indicator_counters(&mut st, &trade_type, "position_managed");
                }
                if Self::enforce_blend_overlay_rebalance_for_open_symbol(state, symbol, &candles)
                    .await
                {
                    return;
                }
                Self::try_add_graded_tranche(state, symbol, &candles, db_path).await;
                return;
            }

            if tuning.data_gate_enabled {
                let health =
                    crate::robot::data_pipeline::CandleHealth::from_candles(&candles, interval);
                if !health.is_healthy(&tuning.health_thresholds(), interval) {
                    if log_throttle_should_emit(
                        symbol,
                        "data_health_gate",
                        tuning.log_dataingest_cooldown_secs,
                    ) {
                        push_state_log(
                            state,
                            format!(
                                "🩺 {} açılış atlandı: veri sağlık kapısı başarısız (rows={} gap={:.1}% stale={}sn)",
                                symbol, health.rows, health.gap_pct, health.stale_secs
                            ),
                        );
                    }
                    return;
                }
            }

            let interval_secs =
                crate::robot::data_pipeline::DataNormalizer::parse_interval(interval) as i64;
            let stale_bound =
                effective_stale_feed_age(tuning.stale_feed_max_age_secs, interval_secs);
            if stale_bound > 0 {
                if let Some(last) = candles.last() {
                    if !candle_is_fresh_within(&last.timestamp, stale_bound) {
                        let age = (chrono::Utc::now() - last.timestamp).num_seconds();
                        if log_throttle_should_emit(
                            symbol,
                            "stale_feed_skip",
                            tuning.log_dataingest_cooldown_secs,
                        ) {
                            let reason = if age < 0 {
                                format!(
                                    "son mum zaman damgası {}sn gelecekte — saat senkronu/venue zamanı kontrol edilmeli",
                                    age.saturating_abs()
                                )
                            } else {
                                format!("feed bayat (son mum {}sn eski > {}sn)", age, stale_bound)
                            };
                            push_state_log(
                                state,
                                format!(
                                    "🧊 {} açılış atlandı: {} — phantom giriş koruması",
                                    symbol, reason
                                ),
                            );
                        }
                        return;
                    }
                }
            }

            let signal_candles: &[Candle] = closed_bar_window(
                &candles,
                interval_secs,
                tuning.signal_closed_bar_only,
                chrono::Utc::now(),
            );

            let (multi_tf_enabled, multi_tf_min) = state
                .lock()
                .ok()
                .and_then(|st| {
                    st.brain
                        .parameters
                        .read()
                        .ok()
                        .map(|p| (p.multi_tf.enabled, p.multi_tf.min_required))
                })
                .unwrap_or((true, crate::robot::data_pipeline::HTF_MIN_REQUIRED));
            let htf_candles_vec = if multi_tf_enabled {
                let account_profile = state
                    .lock()
                    .ok()
                    .map(|st| st.config.venue_profile.as_str().to_string())
                    .unwrap_or_else(|| "default".to_string());
                match Self::resolve_read_ns(state, symbol) {
                    Some((ex, mk, bare)) => {
                        crate::robot::data_pipeline::load_htf_candles_ns_for_profile(
                            db_path,
                            &bare,
                            interval,
                            &ex,
                            &mk,
                            &account_profile,
                            multi_tf_min,
                        )
                    }
                    None => {
                        let market = state
                            .lock()
                            .ok()
                            .map(|st| st.config.market.clone())
                            .unwrap_or_default();
                        crate::robot::data_pipeline::load_htf_candles(
                            db_path,
                            symbol,
                            interval,
                            &market,
                            multi_tf_min,
                        )
                    }
                }
            } else {
                Vec::new()
            };
            let htf_slice: Option<&[crate::core::types::Candle]> = if htf_candles_vec.is_empty() {
                None
            } else {
                Some(&htf_candles_vec)
            };
            let regime = Self::regime_for_cycle(
                state,
                symbol,
                signal_candles,
                interval,
                htf_slice,
                tuning.regime_context_ttl_secs,
                tuning.regime_gbt,
                tuning.regime_adaptive_pctl,
            );
            Self::ensure_regime_patch(state, regime.as_str());
            Self::observe_regime_drift(state, regime.as_str());

            let regime_directional_eff = state
                .lock()
                .ok()
                .and_then(|st| {
                    st.brain.parameters.read().ok().map(|p| {
                        p.regime_directional_for(regime.as_str(), tuning.regime_directional)
                    })
                })
                .unwrap_or(tuning.regime_directional);

            if !(has_seed_strategy && tuning.seed_strategy_priority)
                && Self::try_open_scalp_swing(
                    state,
                    symbol,
                    signal_candles,
                    regime,
                    regime_directional_eff,
                )
                .await
            {
                return;
            }
            let regime_thr = tuning.regime_thresholds(signal_candles);
            let venue_strategy = super::venue_profile::strategy_override_for_symbol(symbol);
            let venue_auto =
                !has_seed_strategy && super::venue_profile::strategy_auto_for_symbol(symbol);
            let strategy_name = if let Some(strategy) = venue_strategy {
                strategy
            } else if venue_auto
                || live_strategy.eq_ignore_ascii_case("default")
                || live_strategy.eq_ignore_ascii_case("auto")
                || live_strategy.is_empty()
            {
                if tuning.strategy_select_eval {
                    use crate::robot::logic::market_regime::{detect_adx_regime_with, AdxRegime};
                    if matches!(
                        detect_adx_regime_with(signal_candles, &regime_thr),
                        AdxRegime::Volatile
                    ) {
                        crate::robot::ml_engine::strategy_selector::IDLE_PROTECT.to_string()
                    } else {
                        let ps = state
                            .lock()
                            .ok()
                            .map(|st| std::sync::Arc::clone(&st.brain.parameters));
                        let strict_pool = super::enforce_trend_strict_pool(
                            super::venue_profile::strategy_pool_for_symbol(symbol),
                        );
                        let strict_pool_refs: Vec<&str> =
                            strict_pool.iter().map(|s| s.as_str()).collect();
                        let sel = crate::robot::strategies::strategy_selector::StrategySelector::from_registry(
                            &crate::robot::strategies::default_registry(),
                            &strict_pool_refs,
                        );
                        let (best_name, _sig) =
                            sel.select_best_name_resolved(signal_candles, |name| {
                                return ps
                                    .as_ref()
                                    .and_then(|p| {
                                        p.read().ok().map(|g| g.resolve_strategy_params(name))
                                    })
                                    .unwrap_or_default();
                            });
                        best_name
                    }
                } else {
                    let sel = crate::robot::ml_engine::strategy_selector::StrategySelector::new();
                    sel.select_best_with(
                        signal_candles,
                        &crate::core::types::StrategyParams::default(),
                        &regime_thr,
                    )
                    .to_string()
                }
            } else {
                live_strategy.to_string()
            };

            if !crate::robot::execution::IdleStrategyPolicy
                .evaluate_name(Some(&strategy_name))
                .is_allow()
            {
                return;
            }

            let strategy = crate::robot::logic::optimizer::make_strategy_pub(&strategy_name);
            let memory_candles =
                Self::read_candles_routed(state, &db_path, symbol, "15m", 30).unwrap_or_default();
            let opportunity_snapshot =
                crate::robot::data_pipeline::MemoryOpportunitySnapshot::from_candles(
                    symbol,
                    "15m",
                    &memory_candles,
                );
            let strat_params = state
                .lock()
                .ok()
                .and_then(|st| {
                    st.brain.parameters.read().ok().map(|p| {
                        p.resolve_strategy_params_for_opportunity(
                            symbol,
                            interval,
                            &strategy_name,
                            opportunity_snapshot.as_ref(),
                        )
                    })
                })
                .unwrap_or_default();

            let signal =
                match strategy.generate_signal(signal_candles, &strat_params, None, htf_slice) {
                    Ok(s) => {
                        Self::mark_pipeline_stage(
                            state,
                            PipelineStage::StrategyEval,
                            StepStatus::Done,
                        );
                        s
                    }
                    Err(e) => {
                        Self::mark_pipeline_stage(
                            state,
                            PipelineStage::StrategyEval,
                            StepStatus::Failed,
                        );
                        emit_trade_event(state, || {
                            crate::robot::infra::logger::TradeEvent::error(&format!(
                                "{} sinyal üretim hatası: {:?}",
                                symbol, e
                            ))
                        });
                        return;
                    }
                };
            let signal_label = match signal {
                Signal::Buy => "BUY",
                Signal::Sell => "SELL",
                Signal::Hold => "HOLD",
            };
            record_decision_audit(
                state,
                symbol,
                &strategy_name,
                signal_label,
                regime.as_str(),
                0.0,
                0.0,
                0.0,
                "signal",
                if matches!(signal, Signal::Hold) {
                    "NO_SIGNAL"
                } else {
                    "CANDIDATE"
                },
                if matches!(signal, Signal::Hold) {
                    "strategy_hold"
                } else {
                    "signal_generated"
                },
                signal_candles.last().map(|c| c.timestamp.to_rfc3339()),
                interval,
            );
            let shadow_policy =
                crate::robot::config_engine::OtonomConfigEngine::load_central_config()
                    .decision_policy;
            let shadow_setup_result = Self::read_candles_routed(
                state,
                &db_path,
                symbol,
                &shadow_policy.setup_interval,
                200,
            );
            let shadow_trigger_result = Self::read_candles_routed(
                state,
                &db_path,
                symbol,
                &shadow_policy.trigger_interval,
                200,
            );
            let (shadow_signal, shadow_status, shadow_reason) =
                if shadow_setup_result.is_err() || shadow_trigger_result.is_err() {
                    ("HOLD", "DATA_ERROR", "setup_or_trigger_data_read_failed")
                } else {
                    let shadow_setup = shadow_setup_result.unwrap_or_default();
                    let shadow_trigger = shadow_trigger_result.unwrap_or_default();
                    if shadow_setup.len() < 30 || shadow_trigger.len() < 30 {
                        (
                            "HOLD",
                            "INSUFFICIENT_DATA",
                            "setup_or_trigger_history_below_minimum",
                        )
                    } else {
                        let setup_signal =
                            strategy.generate_signal(&shadow_setup, &strat_params, None, None);
                        let trigger_signal =
                            strategy.generate_signal(&shadow_trigger, &strat_params, None, None);
                        match (setup_signal, trigger_signal) {
                            (Ok(setup_signal), Ok(trigger_signal)) => {
                                let (shadow_signal, shadow_reason) =
                                    super::combine_shadow_signals(setup_signal, trigger_signal);
                                (shadow_signal, "EVALUATED", shadow_reason)
                            }
                            _ => (
                                "HOLD",
                                "STRATEGY_ERROR",
                                "shadow_strategy_evaluation_failed",
                            ),
                        }
                    }
                };
            record_shadow_decision(state, symbol, shadow_signal, shadow_status, shadow_reason);
            if matches!(signal, Signal::Hold)
                && log_throttle_should_emit(symbol, "decision_no_signal", 60)
            {
                push_state_log(
                    state,
                    format!(
                        "🔎 {} karar: NO_SIGNAL (strateji={} neden=strategy_hold bar={})",
                        symbol,
                        strategy_name,
                        signal_candles
                            .last()
                            .map(|c| c.timestamp.to_rfc3339())
                            .unwrap_or_else(|| "yok".to_string())
                    ),
                );
            }

            let mut symbolic_diag: Option<super::symbolic_candles::SymbolicDiagnosis> = None;
            let mut symbolic_high_conf_conflict = false;
            let symbolic_effective_min_conf = if tuning.symbolic_gating_adaptive {
                let base = tuning.symbolic_gating_min_conf;
                let lo = tuning
                    .symbolic_gating_adaptive_min_conf
                    .min(tuning.symbolic_gating_adaptive_max_conf);
                let hi = tuning
                    .symbolic_gating_adaptive_max_conf
                    .max(tuning.symbolic_gating_adaptive_min_conf);
                let alignment = snap
                    .symbolic_health
                    .get("alignment_pct")
                    .copied()
                    .unwrap_or(50.0)
                    / 100.0;
                let conflict = snap
                    .symbolic_health
                    .get("conflict_pct")
                    .copied()
                    .unwrap_or(50.0)
                    / 100.0;
                let target_alignment =
                    (tuning.symbolic_gating_target_alignment_pct / 100.0).clamp(0.0, 1.0);
                let delta_align = (target_alignment - alignment) * 0.25;
                let delta_conflict = (conflict - 0.40) * 0.20;
                (base + delta_align + delta_conflict).clamp(lo, hi)
            } else {
                tuning.symbolic_gating_min_conf
            };
            if let Ok(mut st) = state.lock() {
                *st.fleet
                    .autonomy_counters
                    .entry("symbolic.scan".to_string())
                    .or_insert(0) += 1;
            }
            if let Some(sym_diag) = super::symbolic_candles::analyze_recent_patterns(signal_candles)
            {
                symbolic_diag = Some(sym_diag.clone());
                if let Ok(mut st) = state.lock() {
                    *st.fleet
                        .autonomy_counters
                        .entry("symbolic.detected".to_string())
                        .or_insert(0) += 1;
                    let bias_key = format!("symbolic.bias.{}", sym_diag.bias.as_str());
                    *st.fleet.autonomy_counters.entry(bias_key).or_insert(0) += 1;
                    for pat in &sym_diag.patterns {
                        let k = format!("symbolic.pattern.{}", pat);
                        *st.fleet.autonomy_counters.entry(k).or_insert(0) += 1;
                    }

                    match signal {
                        Signal::Buy
                            if matches!(
                                sym_diag.bias,
                                super::symbolic_candles::SymbolicBias::Bullish
                            ) =>
                        {
                            *st.fleet
                                .autonomy_counters
                                .entry("symbolic.signal.aligned_buy".to_string())
                                .or_insert(0) += 1;
                            Self::bump_indicator_counters(
                                &mut st,
                                &strategy_name,
                                "symbolic_aligned",
                            );
                        }
                        Signal::Sell
                            if matches!(
                                sym_diag.bias,
                                super::symbolic_candles::SymbolicBias::Bearish
                            ) =>
                        {
                            *st.fleet
                                .autonomy_counters
                                .entry("symbolic.signal.aligned_sell".to_string())
                                .or_insert(0) += 1;
                            Self::bump_indicator_counters(
                                &mut st,
                                &strategy_name,
                                "symbolic_aligned",
                            );
                        }
                        Signal::Buy | Signal::Sell => {
                            if !matches!(
                                sym_diag.bias,
                                super::symbolic_candles::SymbolicBias::Neutral
                            ) {
                                *st.fleet
                                    .autonomy_counters
                                    .entry("symbolic.signal.conflict".to_string())
                                    .or_insert(0) += 1;
                                Self::bump_indicator_counters(
                                    &mut st,
                                    &strategy_name,
                                    "symbolic_conflict",
                                );
                            }
                        }
                        Signal::Hold => {
                            *st.fleet
                                .autonomy_counters
                                .entry("symbolic.signal.hold".to_string())
                                .or_insert(0) += 1;
                        }
                    }

                    let is_directional_signal = matches!(signal, Signal::Buy | Signal::Sell);
                    let is_directional_conflict = (matches!(signal, Signal::Buy)
                        && matches!(
                            sym_diag.bias,
                            super::symbolic_candles::SymbolicBias::Bearish
                        ))
                        || (matches!(signal, Signal::Sell)
                            && matches!(
                                sym_diag.bias,
                                super::symbolic_candles::SymbolicBias::Bullish
                            ));
                    if is_directional_signal
                        && is_directional_conflict
                        && sym_diag.confidence >= symbolic_effective_min_conf
                    {
                        symbolic_high_conf_conflict = true;
                        *st.fleet
                            .autonomy_counters
                            .entry("symbolic.compare.would_block".to_string())
                            .or_insert(0) += 1;
                        Self::bump_indicator_counters(
                            &mut st,
                            &strategy_name,
                            "symbolic_would_block",
                        );
                    }
                    if is_directional_signal && tuning.symbolic_gating_adaptive {
                        *st.fleet
                            .autonomy_counters
                            .entry("symbolic.adaptive.applied".to_string())
                            .or_insert(0) += 1;
                        if symbolic_effective_min_conf > tuning.symbolic_gating_min_conf + 1e-6 {
                            *st.fleet
                                .autonomy_counters
                                .entry("symbolic.adaptive.tightened".to_string())
                                .or_insert(0) += 1;
                        } else if symbolic_effective_min_conf + 1e-6
                            < tuning.symbolic_gating_min_conf
                        {
                            *st.fleet
                                .autonomy_counters
                                .entry("symbolic.adaptive.relaxed".to_string())
                                .or_insert(0) += 1;
                        }
                    }
                }

                if matches!(signal, Signal::Buy | Signal::Sell)
                    && sym_diag.confidence >= 0.60
                    && log_throttle_should_emit(symbol, "symbolic_diag", 120)
                {
                    let sig = match signal {
                        Signal::Buy => "BUY",
                        Signal::Sell => "SELL",
                        Signal::Hold => "HOLD",
                    };
                    let pats = if sym_diag.patterns.is_empty() {
                        "none".to_string()
                    } else {
                        sym_diag.patterns.join("|")
                    };
                    push_state_log(state, format!(
                        "🕯️ {} symbolic={} conf={:.2} score={:.2} patterns={} signal={} (dry-run)",
                        symbol, sym_diag.bias.as_str(), sym_diag.confidence, sym_diag.score, pats, sig,
                    ));
                }

                if !tuning.symbolic_gating_enable
                    && symbolic_high_conf_conflict
                    && log_throttle_should_emit(symbol, "symbolic_compare", 120)
                {
                    push_state_log(state, format!(
                        "🧪 {} symbolic compare: conf={:.2} bias={} sinyal ile ÇELİŞİYOR → gate AÇIK OLSAYDI bloklanırdı",
                        symbol, sym_diag.confidence, sym_diag.bias.as_str(),
                    ));
                }

                if tuning.symbolic_gating_adaptive
                    && matches!(signal, Signal::Buy | Signal::Sell)
                    && log_throttle_should_emit(symbol, "symbolic_adaptive", 180)
                {
                    push_state_log(
                        state,
                        format!(
                            "🧠 {} symbolic adaptive: min_conf {:.2} (base {:.2})",
                            symbol, symbolic_effective_min_conf, tuning.symbolic_gating_min_conf,
                        ),
                    );
                }
            }
            let ml_conf_used: f64 = if tuning.gbt_edge_legacy {
                let gbt_conf = if signal_candles.len() >= 30 {
                    let tail = &signal_candles[signal_candles.len().saturating_sub(200)..];
                    let fv = crate::robot::ml_engine::FeatureExtractor::extract(tail);
                    state.lock().ok().and_then(|st| {
                        st.brain
                            .intelligence_hub
                            .read()
                            .ok()
                            .and_then(|hub| hub.predict_confidence(&fv, &signal))
                    })
                } else {
                    None
                };
                if let Some(c) = gbt_conf {
                    if let Ok(mut st) = state.lock() {
                        st.brain.ml_confidence = c;
                    }
                }
                gbt_conf.unwrap_or(ml_confidence)
            } else {
                ml_confidence
            };

            let edge = Self::compute_edge_score(signal_candles, &signal, ml_conf_used);
            Self::mark_pipeline_stage(state, PipelineStage::FeatureExtract, StepStatus::Done);
            let edge_threshold = state
                .lock()
                .ok()
                .and_then(|st| {
                    st.brain
                        .parameters
                        .read()
                        .ok()
                        .map(|p| p.edge_threshold_for(regime.as_str(), ml_conf_used))
                })
                .unwrap_or_else(|| Self::dynamic_edge_threshold(ml_conf_used));
            let edge_log_floor = edge_threshold * 0.75;
            record_decision_audit(
                state,
                symbol,
                &strategy_name,
                signal_label,
                regime.as_str(),
                edge,
                edge_threshold,
                ml_conf_used,
                "signal",
                if matches!(signal, Signal::Hold) {
                    "NO_SIGNAL"
                } else {
                    "CANDIDATE"
                },
                if matches!(signal, Signal::Hold) {
                    "strategy_hold"
                } else {
                    "signal_generated"
                },
                signal_candles.last().map(|c| c.timestamp.to_rfc3339()),
                interval,
            );

            let pos_dir: Option<bool> = {
                let st = match state.lock() {
                    Ok(s) => s,
                    Err(_) => return,
                };
                st.finance
                    .live_positions
                    .read()
                    .ok()
                    .and_then(|p| p.get(symbol).map(|pos| pos.is_long))
            };

            if matches!(signal, Signal::Buy | Signal::Sell) {
                if let Ok(mut st) = state.lock() {
                    Self::bump_indicator_counters(&mut st, &strategy_name, "used");
                }
            }

            if tuning.signal_closed_bar_only && matches!(signal, Signal::Buy | Signal::Sell) {
                if let Some(bar_ts) = signal_candles.last().map(|c| c.timestamp) {
                    let fresh_bar = state
                        .lock()
                        .ok()
                        .map(|st| st.finance.claim_signal_bar(symbol, bar_ts))
                        .unwrap_or(true);
                    if !fresh_bar {
                        return;
                    }
                }
            }

            if matches!(signal, Signal::Buy | Signal::Sell) {
                let signal_price = state
                    .lock()
                    .ok()
                    .and_then(|st| {
                        st.fleet
                            .live_price
                            .read()
                            .ok()
                            .and_then(|m| m.get(symbol).copied())
                    })
                    .filter(|&v| v > 0.0)
                    .unwrap_or_else(|| candles.last().map(|c| c.close).unwrap_or(0.0));
                emit_trade_event(state, || {
                    crate::robot::infra::logger::TradeEvent::signal(symbol, signal, signal_price)
                });
            }
            match (signal, pos_dir) {
                (crate::core::types::Signal::Buy, None)
                | (crate::core::types::Signal::Sell, None) => {
                    if symbolic_high_conf_conflict {
                        let gate_in_scope = tuning.symbolic_gating_enabled_for(symbol);
                        if gate_in_scope {
                            if let Ok(mut st) = state.lock() {
                                *st.fleet
                                    .autonomy_counters
                                    .entry("symbolic.gated".to_string())
                                    .or_insert(0) += 1;
                                Self::bump_indicator_counters(
                                    &mut st,
                                    &strategy_name,
                                    "gated_symbolic",
                                );
                            }
                            if let Some(diag) = symbolic_diag.as_ref() {
                                if log_throttle_should_emit(symbol, "symbolic_gate_block", 60) {
                                    push_state_log(state, format!(
                                        "🕯️ {} {} ⇒ REDDEDİLDİ (symbolic gate: bias={} conf={:.2} >= {:.2})",
                                        symbol, signal_label, diag.bias.as_str(), diag.confidence, symbolic_effective_min_conf,
                                    ));
                                }
                            }
                            record_decision_audit(
                                state,
                                symbol,
                                &strategy_name,
                                signal_label,
                                regime.as_str(),
                                edge,
                                edge_threshold,
                                ml_conf_used,
                                "symbolic",
                                "REJECTED",
                                "symbolic_gate",
                                signal_candles.last().map(|c| c.timestamp.to_rfc3339()),
                                interval,
                            );
                            return;
                        } else if tuning.symbolic_gating_enable {
                            if let Ok(mut st) = state.lock() {
                                *st.fleet
                                    .autonomy_counters
                                    .entry("symbolic.compare.out_of_scope".to_string())
                                    .or_insert(0) += 1;
                                Self::bump_indicator_counters(
                                    &mut st,
                                    &strategy_name,
                                    "symbolic_out_of_scope",
                                );
                            }
                            if log_throttle_should_emit(symbol, "symbolic_scope_skip", 120) {
                                push_state_log(state, format!(
                                    "🧪 {} symbolic compare: high-conf conflict var ama rollout kapsamı dışında", symbol,
                                ));
                            }
                        }
                    }

                    if regime_directional_eff
                        && !crate::robot::logic::market_regime::regime_confirms_direction(
                            regime,
                            matches!(signal, crate::core::types::Signal::Buy),
                        )
                    {
                        if let Ok(mut st) = state.lock() {
                            Self::bump_indicator_counters(
                                &mut st,
                                &strategy_name,
                                "gated_regime_direction",
                            );
                        }
                        if log_throttle_should_emit(symbol, "regime_dir_block", 60) {
                            push_state_log(
                                state,
                                format!(
                                    "🧭 {} {} ⇒ REDDEDİLDİ (rejim-yön teyidi yok, rejim={})",
                                    symbol,
                                    signal_label,
                                    regime.as_str(),
                                ),
                            );
                        }
                        emit_canonical_event(
                            state,
                            "decision",
                            "DECISION_REJECTED",
                            "internal",
                            symbol,
                            serde_json::json!({"outcome":"rejected","reason":"regime_direction","regime":regime.as_str(),"strategy":strategy_name,"edge":edge}),
                        );
                        record_decision_audit(
                            state,
                            symbol,
                            &strategy_name,
                            signal_label,
                            regime.as_str(),
                            edge,
                            edge_threshold,
                            ml_conf_used,
                            "regime_direction",
                            "REJECTED",
                            "regime_direction",
                            signal_candles.last().map(|c| c.timestamp.to_rfc3339()),
                            interval,
                        );
                        return;
                    }
                    if tuning.regime_strategy_fit
                        && !crate::robot::logic::market_regime::strategy_fits_regime(
                            &strategy_name,
                            regime,
                        )
                    {
                        if let Ok(mut st) = state.lock() {
                            Self::bump_indicator_counters(
                                &mut st,
                                &strategy_name,
                                "gated_regime_style",
                            );
                        }
                        if log_throttle_should_emit(symbol, "regime_style_block", 60) {
                            push_state_log(state, format!(
                                "🧭 {} {} ⇒ REDDEDİLDİ (strateji-stili rejimle uyumsuz, strat={} rejim={})",
                                symbol, signal_label, strategy_name, regime.as_str(),
                            ));
                        }
                        emit_canonical_event(
                            state,
                            "decision",
                            "DECISION_REJECTED",
                            "internal",
                            symbol,
                            serde_json::json!({"outcome":"rejected","reason":"regime_strategy_fit","regime":regime.as_str(),"strategy":strategy_name,"edge":edge}),
                        );
                        record_decision_audit(
                            state,
                            symbol,
                            &strategy_name,
                            signal_label,
                            regime.as_str(),
                            edge,
                            edge_threshold,
                            ml_conf_used,
                            "regime_strategy_fit",
                            "REJECTED",
                            "regime_strategy_fit",
                            signal_candles.last().map(|c| c.timestamp.to_rfc3339()),
                            interval,
                        );
                        return;
                    }
                    if edge < edge_threshold {
                        if let Ok(mut st) = state.lock() {
                            Self::bump_indicator_counters(&mut st, &strategy_name, "gated_edge");
                        }
                        if edge >= edge_log_floor
                            && log_throttle_should_emit(symbol, "edge_weak_block", 60)
                        {
                            push_state_log(state, format!(
                                "📊 {} {} edge={:.2} eşik={:.2} ⇒ REDDEDİLDİ (zayıf edge, strat={})",
                                symbol, signal_label, edge, edge_threshold, strategy_name,
                            ));
                        }
                        emit_canonical_event(
                            state,
                            "decision",
                            "DECISION_REJECTED",
                            "internal",
                            symbol,
                            serde_json::json!({"outcome":"rejected","reason":"edge_threshold","regime":regime.as_str(),"strategy":strategy_name,"edge":edge,"threshold":edge_threshold}),
                        );
                        record_decision_audit(
                            state,
                            symbol,
                            &strategy_name,
                            signal_label,
                            regime.as_str(),
                            edge,
                            edge_threshold,
                            ml_conf_used,
                            "edge",
                            "REJECTED",
                            "edge_threshold",
                            signal_candles.last().map(|c| c.timestamp.to_rfc3339()),
                            interval,
                        );
                        return;
                    }
                    match crate::core::session::entry_session_for_symbol(symbol, chrono::Utc::now())
                    {
                        crate::core::session::EntrySessionDecision::Allow => {}
                        crate::core::session::EntrySessionDecision::Closed { reason } => {
                            Self::mark_pipeline_stage(
                                state,
                                PipelineStage::RiskGate,
                                StepStatus::Skipped,
                            );
                            if let Ok(mut st) = state.lock() {
                                Self::bump_indicator_counters(
                                    &mut st,
                                    &strategy_name,
                                    "gated_session",
                                );
                            }
                            if log_throttle_should_emit(symbol, "session_closed", 60) {
                                push_state_log(
                                    state,
                                    format!(
                                        "🕒 {} {} giriş kapısı: {}",
                                        symbol, signal_label, reason
                                    ),
                                );
                            }
                            record_decision_audit(
                                state,
                                symbol,
                                &strategy_name,
                                signal_label,
                                regime.as_str(),
                                edge,
                                edge_threshold,
                                ml_conf_used,
                                "session",
                                "REJECTED",
                                &reason,
                                signal_candles.last().map(|c| c.timestamp.to_rfc3339()),
                                interval,
                            );
                            return;
                        }
                    }
                    // req_notional artık sabit %10 değil: gerçek boyutlandırmanın ÜST SINIRI.
                    // positions.rs: alloc = equity·base_alloc_fraction·risk_appetite·kelly_scale
                    // (kelly_scale ≤ 0.625) → base_alloc üst sınırdır. Sabit 0.10 ise tuning'den
                    // ve risk_appetite'tan (0.5–2.0) kopuktu; equity 2×'te gerçek tutar kapıya
                    // sunulanı AŞABİLİYORDU.
                    let req_notional = {
                        let (frac, appetite) = state
                            .lock()
                            .ok()
                            .map(|st| {
                                (
                                    st.tuning.base_alloc_fraction,
                                    st.finance.calculate_risk_appetite(),
                                )
                            })
                            .unwrap_or((0.10, 1.0));
                        snap.finance.total_equity * frac * appetite
                    };
                    let decision = risk_manager.authorize(&signal, snap, edge, req_notional);
                    if let crate::robot::risk::risk_gate::RiskDecision::Deny {
                        reasons,
                        enter_safe_mode,
                        halt,
                    } = decision
                    {
                        let gate_status = if enter_safe_mode || halt {
                            StepStatus::Skipped
                        } else {
                            StepStatus::Done
                        };
                        Self::mark_pipeline_stage(state, PipelineStage::RiskGate, gate_status);
                        if let Ok(mut st) = state.lock() {
                            Self::bump_indicator_counters(&mut st, &strategy_name, "gated_risk");
                        }
                        let mode = if halt {
                            "HALT"
                        } else if enter_safe_mode {
                            "SAFE-MODE"
                        } else {
                            "DENY"
                        };
                        let block_reason = format!("[{}] {}", mode, reasons.join(" · "));
                        // YAPTIRIM (Adım 4a): karar artık yalnız loglanmıyor, mühürleniyor.
                        // Eskiden halt/enter_safe_mode SADECE log etiketiydi → kill-switch yoktu.
                        // HALT safe-mode'u kapsar (yeni giriş tamamen durur) → else-if.
                        if halt {
                            Self::trip_risk_halt(state, &reasons.join(" · "));
                        } else if enter_safe_mode {
                            Self::trip_safe_mode(state, &reasons.join(" · "));
                        }
                        let should_log =
                            halt || log_throttle_should_emit(symbol, "risk_block_safemode", 60);
                        if should_log {
                            push_state_log(
                                state,
                                format!(
                                    "🛡️ {} {} edge={:.2} ✓ ama RiskManager [{}]: {}",
                                    symbol,
                                    signal_label,
                                    edge,
                                    mode,
                                    reasons.join(" · "),
                                ),
                            );
                        }
                        emit_trade_event(state, || {
                            crate::robot::infra::logger::TradeEvent::risk_block(
                                &block_reason,
                                symbol,
                            )
                        });
                        emit_canonical_event(
                            state,
                            "decision",
                            "DECISION_REJECTED",
                            "internal",
                            symbol,
                            serde_json::json!({"outcome":"rejected","reason":"risk_gate","regime":regime.as_str(),"strategy":strategy_name,"edge":edge,"details":reasons}),
                        );
                        record_decision_audit(
                            state,
                            symbol,
                            &strategy_name,
                            signal_label,
                            regime.as_str(),
                            edge,
                            edge_threshold,
                            ml_conf_used,
                            "risk",
                            "REJECTED",
                            "risk_gate",
                            signal_candles.last().map(|c| c.timestamp.to_rfc3339()),
                            interval,
                        );
                        return;
                    }
                    Self::mark_pipeline_stage(state, PipelineStage::RiskGate, StepStatus::Done);
                    if log_throttle_should_emit(symbol, "open_attempt", 60) {
                        push_state_log(
                            state,
                            format!(
                                "📊 {} {} edge={:.2} ✓ + risk ✓ ⇒ POZİSYON AÇILIYOR (strat={})",
                                symbol, signal_label, edge, strategy_name,
                            ),
                        );
                    }
                    if let Ok(mut st) = state.lock() {
                        Self::bump_indicator_counters(&mut st, &strategy_name, "entry_attempt");
                    }
                    emit_canonical_event(
                        state,
                        "decision",
                        "DECISION_ACCEPTED",
                        "internal",
                        symbol,
                        serde_json::json!({"outcome":"accepted","regime":regime.as_str(),"strategy":strategy_name,"edge":edge,"requested_notional":req_notional}),
                    );
                    record_decision_audit(
                        state,
                        symbol,
                        &strategy_name,
                        signal_label,
                        regime.as_str(),
                        edge,
                        edge_threshold,
                        ml_conf_used,
                        "entry",
                        "ACCEPTED",
                        "risk_and_entry_gates_passed",
                        signal_candles.last().map(|c| c.timestamp.to_rfc3339()),
                        interval,
                    );
                    Self::open_paper_position(
                        state,
                        symbol,
                        &signal,
                        &candles,
                        &strategy_name,
                        None,
                        None,
                    )
                    .await;
                }
                (crate::core::types::Signal::Sell, Some(true))
                | (crate::core::types::Signal::Buy, Some(false)) => {
                    let _ = (signal_label, edge);
                    Self::close_paper_position(state, symbol, &candles, ExitReason::StrategySignal)
                        .await;
                }
                (crate::core::types::Signal::Buy, Some(true))
                | (crate::core::types::Signal::Sell, Some(false)) => {
                    if let Ok(mut st) = state.lock() {
                        Self::bump_indicator_counters(
                            &mut st,
                            &strategy_name,
                            "gated_position_aligned",
                        );
                    }
                    let cooldown = tuning.risk_block_log_cooldown_secs;
                    if log_throttle_should_emit(symbol, "risk_block_pos_aligned", cooldown) {
                        let dir_label = if matches!(signal, Signal::Buy) {
                            "LONG"
                        } else {
                            "SHORT"
                        };
                        emit_trade_event(state, || {
                            crate::robot::infra::logger::TradeEvent::risk_block(
                                &format!(
                                    "[position-aligned] {} sinyali, pozisyon zaten {}",
                                    signal_label, dir_label
                                ),
                                symbol,
                            )
                        });
                    }
                    record_decision_audit(
                        state,
                        symbol,
                        &strategy_name,
                        signal_label,
                        regime.as_str(),
                        edge,
                        edge_threshold,
                        ml_conf_used,
                        "position",
                        "REJECTED",
                        "position_aligned",
                        signal_candles.last().map(|c| c.timestamp.to_rfc3339()),
                        interval,
                    );
                }
                _ => {}
            }
        }
    }
    pub(crate) fn cycle_load_candles(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        db_path: &str,
        interval: &str,
        tuning: &RuntimeTuning,
    ) -> Option<Vec<Candle>> {
        use crate::robot::data_pipeline::canon::PipelineStage;
        use crate::robot::data_pipeline::StepStatus;
        // 🗄️ Kanonik namespace + ÇIPLAK sembol — TEK KAYNAK: venue.storage_namespace().
        //
        // ÖNCEDEN: market `Mt5|Ibkr => "cfd"` diye sabitleniyor, sembol ise ETİKETLİ
        // ("EURUSD@mt5") sorgulanıyordu. Yazma yolu ise çıplak sembol + "mt5" kullanıyordu.
        // İki uçlu uyumsuzluk → canlı döngü enstrümanın geçmişinin ince bir dilimini
        // görüyordu (ölçüm: AUDUSD 15m'de 68 bar okunurken 3508 bar başka kovada duruyordu).
        //
        // exchange filtresi ŞART: mt5:cfd ile ibkr:cfd aynı market etiketini paylaşır,
        // market-only okuma iki borsanın serisini karıştırırdı → read_candles_ns.
        let ns = Self::resolve_read_ns(state, symbol);
        let read = match &ns {
            Some((ex, mk, bare)) => {
                crate::persistence::reader::read_candles_ns(db_path, bare, interval, ex, mk, 200)
            }
            // Route edilemeyen sembol (etiketsiz + sınıflandırılamayan) → eski davranış.
            None => crate::persistence::reader::read_candles(db_path, symbol, interval, 200),
        };
        match read {
            Ok(c) if !c.is_empty() => {
                Self::mark_pipeline_stage(state, PipelineStage::DataIngest, StepStatus::Done);
                return Some(c);
            }
            Ok(_) => {
                let cooldown = tuning.log_dataingest_cooldown_secs;
                if log_throttle_should_emit(symbol, "dataingest_empty", cooldown) {
                    log::warn!(
                        "DataIngest empty: {} {} (candles tablo'da 1m kayıt yok)",
                        symbol,
                        interval
                    );
                }
                if let Ok(st) = state.lock() {
                    if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                        pipe.mark_stage_completed(PipelineStage::DataIngest, StepStatus::Failed);
                    }
                }
                return None;
            }
            Err(e) => {
                let cooldown = tuning.log_dataingest_cooldown_secs;
                if log_throttle_should_emit(symbol, "dataingest_error", cooldown) {
                    log::warn!("DataIngest error: {} {} → {}", symbol, interval, e);
                }
                Self::mark_pipeline_stage(state, PipelineStage::DataIngest, StepStatus::Failed);
                return None;
            }
        }
    }
    pub(crate) async fn cycle_try_close_open_position(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        interval: &str,
        candles: &[Candle],
    ) -> bool {
        let is_book_leg = state
            .lock()
            .ok()
            .and_then(|st| {
                st.finance.live_positions.read().ok().and_then(|m| {
                    m.get(symbol)
                        .map(|p| book_core::is_book_managed_tag(&p.trade_type))
                })
            })
            .unwrap_or(false);
        if is_book_leg {
            return true;
        }
        let candle_close = candles.last().map(|c| c.close).unwrap_or(0.0);
        let atr_value = Self::calc_atr(candles, 14);
        let (live_price, exit_reason) = {
            let st = match state.lock() {
                Ok(s) => s,
                Err(_) => return true,
            };
            let default_mult = st
                .brain
                .best_params
                .get("pos_atr_trail_mult")
                .copied()
                .unwrap_or(2.0);
            let pos_strategy: String = st
                .finance
                .live_positions
                .read()
                .ok()
                .and_then(|m| m.get(symbol).map(|p| p.trade_type.clone()))
                .unwrap_or_else(|| "default".to_string());
            let regime_str = Self::classify_regime(candles).as_str().to_string();
            let atr_mult = st
                .brain
                .parameters
                .read()
                .ok()
                .map(|p| {
                    p.resolve_atr_mult_for_regime(
                        symbol,
                        interval,
                        &pos_strategy,
                        default_mult,
                        Some(&regime_str),
                    )
                })
                .unwrap_or(default_mult);
            let be_rr = st
                .brain
                .best_params
                .get("pos_breakeven_at_rr")
                .copied()
                .unwrap_or(1.0);
            let maker_like = (pos_strategy
                == crate::robot::engines::master::xs_live::XS_STRATEGY_TAG
                || pos_strategy == crate::robot::engines::master::carry_live::CARRY_STRATEGY_TAG
                || pos_strategy == crate::robot::engines::master::blend_live::BLEND_STRATEGY_TAG)
                && st.tuning.use_limit_entry;
            let exit_commission_rate = if maker_like {
                st.tuning.maker_commission_rate
            } else {
                st.tuning.commission_rate
            };
            let fleet_price = st
                .fleet
                .live_price
                .read()
                .ok()
                .and_then(|m| m.get(symbol).copied())
                .filter(|&v| v > 0.0);
            let live_price = fleet_price.unwrap_or(candle_close);
            let reason_opt = if let Ok(mut positions) = st.finance.live_positions.write() {
                if let Some(pos) = positions.get_mut(symbol) {
                    pos.current_price = live_price;
                    Self::check_exit_conditions(
                        pos,
                        live_price,
                        atr_value,
                        atr_mult,
                        be_rr,
                        exit_commission_rate,
                    )
                } else {
                    None
                }
            } else {
                None
            };
            st.refresh_canonical_positions();
            (live_price, reason_opt)
        };
        if let Some(reason) = exit_reason {
            push_state_log(
                state,
                format!(
                    "{} {} {} koşulu tetiklendi @ {:.4}",
                    reason.emoji(),
                    symbol,
                    reason.as_str(),
                    live_price,
                ),
            );
            Self::close_paper_position(state, symbol, candles, reason).await;
            return true;
        }
        return false;
    }
}

pub(crate) fn effective_stale_feed_age(configured: i64, interval_secs: i64) -> i64 {
    match configured {
        0 => return 0,
        n if n < 0 => return (interval_secs * 2).max(60),
        n => return n,
    }
}

pub(crate) fn last_bar_is_forming(
    candles: &[Candle],
    interval_secs: i64,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if interval_secs <= 0 {
        return false;
    }
    match candles.last() {
        Some(c) => return c.timestamp + chrono::Duration::seconds(interval_secs) > now,
        None => return false,
    }
}
/// SAF: GİRİŞ-KARARI penceresi (strateji sinyali + rejim + edge). `enabled` ve son bar forming ise
/// son barı dışla → live, backtest'in kapalı-bar karar semantiğiyle hizalanır (repaint/skew biter).
/// Dışlayınca ≥1 bar kalmazsa ya da bar kapalıysa pencere olduğu gibi döner (aşırı-düşme yok).
/// ÇIKIŞLAR bu yolu KULLANMAZ — SL/TP fleet.live_price ile anlık kalır. Testli.
pub(crate) fn closed_bar_window<'a>(
    candles: &'a [Candle],
    interval_secs: i64,
    enabled: bool,
    now: chrono::DateTime<chrono::Utc>,
) -> &'a [Candle] {
    if enabled && candles.len() >= 2 && last_bar_is_forming(candles, interval_secs, now) {
        return &candles[..candles.len() - 1];
    } else {
        return candles;
    }
}

/// Sembolün AÇIK pozisyonu var mı (live_positions symbol-keyed). Çoklu-iz dispatch'i bununla
/// tek-pozisyon invariantını korur: flat'ken izleri sırayla dener, biri açınca durur. Lock
/// edinilemezse "var" sayar (güvenli taraf → fazladan açılış denenmez).
fn symbol_has_open_position(state: &Arc<Mutex<AppState>>, symbol: &str) -> bool {
    return state
        .lock()
        .ok()
        .and_then(|st| {
            st.finance
                .live_positions
                .read()
                .ok()
                .map(|p| p.contains_key(symbol))
        })
        .unwrap_or(true);
}

#[cfg(test)]
mod loop_core_bridge_tests {
    use super::Engine;

    #[test]
    fn mt5_bridge_alert_is_deduplicated_for_same_error() {
        assert!(Engine::should_emit_bridge_alert(
            None,
            "MT5 köprü bind '127.0.0.1:9001': Address already in use"
        ));
        assert!(!Engine::should_emit_bridge_alert(
            Some("MT5 köprü bind '127.0.0.1:9001': Address already in use"),
            "MT5 köprü bind '127.0.0.1:9001': Address already in use",
        ));
        assert!(Engine::should_emit_bridge_alert(
            Some("MT5 köprü bind '127.0.0.1:9001': Address already in use"),
            "MT5 köprü bind '127.0.0.1:9001': timeout",
        ));
    }
}
