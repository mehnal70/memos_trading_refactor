// src/robot/engines/master/jobs.rs — Periyodik işler: intel hub tick + anomali onarımı + sembol-statü + ML retrain.
// Faz 2 modülerleştirme: screener/backtest/download job'ları kardeş dosyalara ayrıldı (davranış birebir).
use super::*;

#[derive(Debug, Clone)]
struct AuxResearchConfig {
    market: String,
    interval: String,
    symbols: Vec<String>,
    min_bars: usize,
    limit: usize,
}

#[derive(Debug, Clone)]
struct AuxResearchSeries {
    symbol: String,
    bars: usize,
    last_ts: Option<chrono::DateTime<chrono::Utc>>,
    candles: Vec<crate::core::types::Candle>,
}

impl Engine {
    fn aux_research_config() -> Option<AuxResearchConfig> {
        if !env_truthy("AUX_RESEARCH_ENABLED") {
            return None;
        }
        let symbols: Vec<String> = std::env::var("AUX_RESEARCH_SYMBOLS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        if symbols.is_empty() {
            return None;
        }
        Some(AuxResearchConfig {
            market: std::env::var("AUX_RESEARCH_MARKET").unwrap_or_else(|_| "mt5".to_string()),
            interval: std::env::var("AUX_RESEARCH_INTERVAL").unwrap_or_else(|_| "1h".to_string()),
            min_bars: env_parse("AUX_RESEARCH_MIN_BARS", 300usize).max(50),
            limit: env_parse("AUX_RESEARCH_LIMIT", 2000usize).max(200),
            symbols,
        })
    }

    fn load_aux_research_series(
        db_path: &str,
        profile: &str,
        cfg: &AuxResearchConfig,
    ) -> Vec<AuxResearchSeries> {
        cfg.symbols
            .iter()
            .filter_map(|sym| {
                let (bare, venue_tag) = crate::robot::venue::registry::split_venue_tag(sym);
                let candles = match venue_tag {
                    Some(exchange) => crate::persistence::reader::read_candles_for_venue_profile(
                        db_path,
                        bare,
                        &cfg.interval,
                        exchange.as_str(),
                        &cfg.market,
                        profile,
                        cfg.limit,
                    )
                    .unwrap_or_default(),
                    None => crate::persistence::reader::read_candles_market(
                        db_path,
                        sym,
                        &cfg.interval,
                        &cfg.market,
                        cfg.limit,
                    )
                    .unwrap_or_default(),
                };
                if candles.len() < cfg.min_bars {
                    return None;
                }
                Some(AuxResearchSeries {
                    symbol: sym.clone(),
                    bars: candles.len(),
                    last_ts: candles.last().map(|c| c.timestamp),
                    candles,
                })
            })
            .collect()
    }

    pub(crate) fn log_aux_research_snapshot(state: &Arc<Mutex<AppState>>, db_path: &str) {
        let Some(cfg) = Self::aux_research_config() else {
            return;
        };
        let profile = state
            .lock()
            .ok()
            .map(|st| st.config.venue_profile.as_str().to_string())
            .unwrap_or_else(|| "default".to_string());
        let series = Self::load_aux_research_series(db_path, &profile, &cfg);
        if series.is_empty() {
            push_state_log(
                state,
                format!(
                    "🧪 AUX research: {}:{} için yeterli history yok (min={} bar)",
                    cfg.market, cfg.interval, cfg.min_bars,
                ),
            );
            return;
        }
        let parts: Vec<String> = series
            .iter()
            .take(4)
            .map(|s| {
                let last = s
                    .last_ts
                    .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "?".to_string());
                format!("{}={}bar@{}", s.symbol, s.bars, last)
            })
            .collect();
        push_state_log(
            state,
            format!(
                "🧪 AUX research hazır: {} / {} sembol · {}:{} · {}",
                series.len(),
                cfg.symbols.len(),
                cfg.market,
                cfg.interval,
                parts.join(" · "),
            ),
        );
    }

    pub(crate) fn log_aux_research_backtest_shadow(
        state: &Arc<Mutex<AppState>>,
        db_path: &str,
        capital: f64,
        strategy_name: &str,
        use_htf: bool,
        edge_min: Option<f64>,
        orderbook_sim: Option<String>,
        tp_pct: f64,
        sl_pct: f64,
        pos_size: f64,
        wf_is: usize,
        wf_oos: usize,
        wf_step: usize,
    ) {
        let Some(cfg) = Self::aux_research_config() else {
            return;
        };
        let profile = state
            .lock()
            .ok()
            .map(|st| st.config.venue_profile.as_str().to_string())
            .unwrap_or_else(|| "default".to_string());
        let series = Self::load_aux_research_series(db_path, &profile, &cfg);
        if series.is_empty() {
            push_state_log(
                state,
                format!(
                    "🧪 AUX MT5 shadow: history yetersiz ({}:{} min={} bar)",
                    cfg.market, cfg.interval, cfg.min_bars,
                ),
            );
            return;
        }

        let mut summaries: Vec<String> = Vec::new();
        let mut score_sum = 0.0f64;
        let mut eligible = 0usize;
        for s in &series {
            let windows = crate::robot::backtester::walk_forward::wf_oos_windows(
                s.candles.len(),
                wf_is,
                wf_oos,
                wf_step,
            );
            if windows.is_empty() {
                continue;
            }
            let cfg_bt = crate::robot::backtester::BacktestConfig {
                symbol: s.symbol.clone(),
                interval: cfg.interval.clone(),
                initial_balance: capital,
                max_position_size: pos_size,
                take_profit_pct: tp_pct,
                stop_loss_pct: sl_pct,
                strategy_name: strategy_name.to_string(),
                commission_pct: 0.001,
                use_htf,
                edge_min_score: edge_min,
                orderbook_sim: orderbook_sim.clone(),
                ..Default::default()
            };
            let pf = crate::robot::backtester::walk_forward::score_config_over_windows(
                &cfg_bt, &s.candles, &windows,
            );
            let cross = crate::robot::backtester::wf_cross_check(&cfg_bt, &s.candles, &windows);
            eligible += 1;
            score_sum += pf;
            summaries.push(format!(
                "{} PF={:.2} C={:.0}% n={}",
                s.symbol,
                pf,
                cross.consistency() * 100.0,
                cross.trades,
            ));
        }

        if eligible == 0 {
            push_state_log(
                state,
                format!(
                    "🧪 AUX MT5 shadow: {}:{} için WF penceresi oluşmadı",
                    cfg.market, cfg.interval,
                ),
            );
            return;
        }

        push_state_log(
            state,
            format!(
                "🧪 AUX MT5 shadow ({strategy_name}): {} sembol · avgPF={:.2} · {}",
                eligible,
                score_sum / eligible as f64,
                summaries.join(" · "),
            ),
        );
    }

    /// 🧠 IntelligenceHub periyodik tick: drift hesabı + evrim + retrain kararı.
    ///
    /// Akış:
    /// 1. Aktif sembolün son 200 mumundan FeatureVector çıkar
    /// 2. hub.drift_detector.update(fv) → drift_score güncellenir
    /// 3. brain.drift_history'e push (TUI snapshot için)
    /// 4. hub.should_retrain(drift_score) true ise: triggers["ml"].store(true) ve repair_log
    /// 5. hub.tick_evolution() — controller cycle_id'yi artırır, periyot dolduğunda evolve
    pub(crate) async fn tick_intelligence_hub(state: &Arc<Mutex<AppState>>) {
        use crate::robot::ml_engine::feature_extractor::FeatureVector;

        // 1) Mum verisini ve aktif sembolü al (kilit kısa)
        let (symbol, interval, db_path, _market) = {
            let st = match state.lock() {
                Ok(s) => s,
                Err(_) => return,
            };
            (
                st.config.symbol.clone(),
                st.config.interval.clone(),
                st.config.db_path.clone(),
                st.config.market.clone(),
            )
        };

        // Mum varsa drift güncellemesi yap; yoksa sadece evrim tick'i çalışır.
        let candles_opt = Self::read_candles_routed(state, &db_path, &symbol, &interval, 200).ok();
        let fv: Option<FeatureVector> = candles_opt
            .as_ref()
            .filter(|c| c.len() >= 50)
            .map(|c| crate::robot::ml_engine::feature_extractor::FeatureExtractor::extract(c));

        // 2-5) Hub mutasyonu — tek mutex altında
        let mut st = match state.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        // Drift-tetikli retrain cooldown: sürekli yüksek drift'te her tick'te
        // yeni trigger basılmasın diye ML_DRIFT_COOLDOWN_SECS (default 600)
        // boyunca bekle. Cooldown=0 → her tick fire (testing modu).
        let ml_drift_cooldown: u64 = env_parse("ML_DRIFT_COOLDOWN_SECS", 600);
        let mut should_retrain = false;
        let mut armed = true;
        let mut drift_score = 0.0;
        let mut controller_cycle = 0u64;
        let mut evolved = false;
        if let Ok(mut hub) = st.brain.intelligence_hub.write() {
            if let Some(ref fv) = fv {
                hub.drift_detector.update(fv);
                drift_score = hub.drift_detector.drift_score;
                hub.drift_history.push_back(drift_score);
                while hub.drift_history.len() > 100 {
                    hub.drift_history.pop_front();
                }
                should_retrain = hub.should_retrain(drift_score);
                armed = hub.drift_retrain_armed(ml_drift_cooldown);
                if should_retrain && armed {
                    hub.mark_drift_retrain_fired();
                }
            }

            // Evrim tick — mum olsa da olmasa da controller cycle ilerler
            hub.controller.begin_cycle();
            controller_cycle = hub.controller.cycle_id;
            if hub.controller.should_evolve() {
                hub.controller.evolve_population();
                evolved = true;
            }
        }
        // brain'in dış drift_history aynalaması (TUI/AI Center için)
        st.brain.drift_history.push_back(drift_score);
        while st.brain.drift_history.len() > 100 {
            st.brain.drift_history.pop_front();
        }

        match (should_retrain, armed) {
            (true, true) => {
                let mut queued = false;
                if let Some(t) = st.fleet.triggers.get("ml") {
                    queued = !t.swap(true, Ordering::AcqRel);
                }
                if queued {
                    *st.fleet
                        .autonomy_counters
                        .entry("trigger.hub.ml.enqueued".to_string())
                        .or_insert(0) += 1;
                    st.push_log(format!(
                        "🧠 Hub: drift={:.3} eşik aşıldı ⇒ ml retrain tetiklendi (cycle={})",
                        drift_score, controller_cycle,
                    ));
                } else {
                    *st.fleet
                        .autonomy_counters
                        .entry("trigger.hub.ml.coalesced".to_string())
                        .or_insert(0) += 1;
                    st.push_log(format!(
                        "🔁 Hub: drift={:.3} yüksek ama ml retrain zaten kuyruktaydı (coalesced, cycle={})",
                        drift_score, controller_cycle,
                    ));
                }
                // Repair log'a da düşür — kaynak "drift" olarak işaretli.
                st.guardian.repair_log.push_back(format!(
                    "[{}] hub: drift-driven retrain (drift={:.3}, cooldown={}s)",
                    chrono::Local::now().format("%H:%M:%S"),
                    drift_score,
                    ml_drift_cooldown,
                ));
                while st.guardian.repair_log.len() > 100 {
                    st.guardian.repair_log.pop_front();
                }
            }
            (true, false) => {
                // Drift devam ediyor ama cooldown'da — tek satır bilgi log'u
                // (her tick spam'lemesin diye sadece guardian repair_log'a yaz,
                // ana UI log'una basma; orada görünmemesi normal akış).
                st.guardian.repair_log.push_back(format!(
                    "[{}] hub: drift={:.3} eşik aşıldı ama cooldown'da ({}s) — fire atlandı",
                    chrono::Local::now().format("%H:%M:%S"),
                    drift_score,
                    ml_drift_cooldown,
                ));
                while st.guardian.repair_log.len() > 100 {
                    st.guardian.repair_log.pop_front();
                }
            }
            _ => { /* drift normal — sessiz */ }
        }
        if evolved {
            st.push_log(format!(
                "🧬 Hub evrim: popülasyon evrimleştirildi (cycle={})",
                controller_cycle,
            ));
        }
    }

    /// 🛡️ ANOMALİ ONARIMI: aktif anomali sayısı > 0 ise ML retrain tetiklenir,
    /// anomali türleri ve onarım kaydı guardian.repair_log'a düşürülür.
    ///
    /// Cooldown (`ANOMALY_ML_TRIGGER_COOLDOWN_SECS`, default 300sn):
    /// Anomali aralıksız sürüyorsa (ör. recovery edilen pasif semboller
    /// nedeniyle DataIngest Failed) her 500ms ML trigger spam'i oluşurdu.
    /// Cooldown süresince ML trigger ATIlMAZ; phase ve log girdisi yine yazılır.
    pub(crate) fn perform_anomaly_recovery(state: &Arc<Mutex<AppState>>, snap: &MissionControl) {
        if snap.active_anomalies == 0 {
            return;
        }
        let mut st = lock_state_recover(state);
        // Phase precedence: Booting/Executing > Recovering > Scanning.
        // Aynı tick'te trade yapıldıysa (execute_trade_cycle phase'i Executing'e
        // yazmışsa) onu ezme — operatör Executing'i görmeli. Stale ApiError
        // anomaly'leri (BEATUSDT/BLESSUSDT) sürekli Recovering basıp 1540
        // tick'te sadece 1 Executing görünmesine yol açıyordu.
        if !matches!(st.fleet.phase.as_str(), "Executing" | "Booting") {
            st.fleet.phase = "Recovering".into();
        }

        // Anomalilerin özetini çıkar (severity sayım)
        let mut warning_n = 0u32;
        let mut critical_n = 0u32;
        let mut kinds: Vec<String> = Vec::new();
        for a in &snap.anomalies {
            if a.severity.contains("Critical") {
                critical_n += 1;
            } else {
                warning_n += 1;
            }
            if !kinds.contains(&a.kind) {
                kinds.push(a.kind.clone());
            }
        }
        let has_retrainable_anomaly = snap.anomalies.iter().any(|anomaly| {
            !matches!(
                anomaly.kind.as_str(),
                "AccountStateUnavailable" | "PositionProtectionGap"
            )
        });
        if !has_retrainable_anomaly {
            let cooldown_secs: u64 = env_parse("ANOMALY_OPERATIONAL_LOG_COOLDOWN_SECS", 300);
            let now_secs = crate::core::time::now_epoch_secs();
            let last_logged = ANOMALY_OPERATIONAL_LAST_LOG_EPOCH.load(Ordering::Relaxed);
            if last_logged == 0 || now_secs.saturating_sub(last_logged) >= cooldown_secs {
                st.push_log(format!(
                    "🛡️ Anomali onarımı: {} aktif, operasyonel riskler için ML retrain atlandı (türler: {})",
                    snap.active_anomalies,
                    kinds.join(","),
                ));
                ANOMALY_OPERATIONAL_LAST_LOG_EPOCH.store(now_secs, Ordering::Relaxed);
            }
            return;
        }

        // Cooldown denetimi: bir önceki ML trigger'dan beri yeterli süre geçti mi?
        let cooldown_secs: u64 = env_parse("ANOMALY_ML_TRIGGER_COOLDOWN_SECS", 300);
        let now_secs = crate::core::time::now_epoch_secs();
        let last_fired = ANOMALY_ML_LAST_TRIGGER_EPOCH.load(Ordering::Relaxed);
        let armed = last_fired == 0 || now_secs.saturating_sub(last_fired) >= cooldown_secs;

        // ML retrain'i tetikle (zaten ml job'u kendi loglarını basacak)
        // push_log + repair_log SADECE armed iken yazılır — cooldown'da spam yapmaz.
        // Daha önce her cycle (500ms) "ML retrain tetiklendi" log basıyordu ama cooldown
        // gerçek tetiği bastırıyordu → mesaj yanıltıcı + olay günlüğü doluyor.
        if armed {
            let mut queued = false;
            if let Some(t) = st.fleet.triggers.get("ml") {
                queued = !t.swap(true, Ordering::AcqRel);
            }
            ANOMALY_ML_LAST_TRIGGER_EPOCH.store(now_secs, Ordering::Relaxed);

            if queued {
                *st.fleet
                    .autonomy_counters
                    .entry("trigger.anomaly.ml.enqueued".to_string())
                    .or_insert(0) += 1;
                st.push_log(format!(
                    "🚨 Anomali onarımı: {} aktif ({} kritik / {} uyarı), türler: {} ⇒ ML retrain tetiklendi",
                    snap.active_anomalies, critical_n, warning_n,
                    kinds.join(","),
                ));
            } else {
                *st.fleet
                    .autonomy_counters
                    .entry("trigger.anomaly.ml.coalesced".to_string())
                    .or_insert(0) += 1;
                st.push_log(format!(
                    "🔁 Anomali onarımı: {} aktif, ML retrain zaten kuyruktaydı (coalesced)",
                    snap.active_anomalies,
                ));
            }

            // Repair log: onarım adımının izi
            let repair_entry = format!(
                "[{}] auto-fix: ml-retrain dispatched (anomaly_count={})",
                chrono::Local::now().format("%H:%M:%S"),
                snap.active_anomalies,
            );
            st.guardian.repair_log.push_back(repair_entry);
            if st.guardian.repair_log.len() > 100 {
                st.guardian.repair_log.pop_front();
            }
        }
    }

    /// 🗂️ Sembol-statü registry refresh: Binance exchangeInfo'dan tüm sembollerin
    /// `status`'ünü (TRADING/BREAK/HALT…) çeker, cache'i günceller + DB'ye persist eder.
    /// `symbol_eligible_for_live` bunu okur → halted/delisted (ALPACAUSDT BREAK gibi)
    /// otoritatif dışlanır; de/re-list otomatik yansır. Public endpoint (key gerekmez,
    /// paper modda da çalışır). Scheduler periyodik çağırır + boot warmup'ta bir kez.
    pub(crate) async fn run_symbol_status_refresh(
        state: &Arc<Mutex<AppState>>,
    ) -> std::result::Result<(), String> {
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
        if !binance_active {
            return Ok(());
        }
        let (market, db_path) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            (st.config.market.clone(), st.config.db_path.clone())
        };
        let testnet = std::env::var("BINANCE_TESTNET")
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        let url = if market.eq_ignore_ascii_case("futures") {
            if testnet {
                "https://testnet.binancefuture.com/fapi/v1/exchangeInfo"
            } else {
                "https://fapi.binance.com/fapi/v1/exchangeInfo"
            }
        } else {
            if testnet {
                "https://testnet.binance.vision/api/v3/exchangeInfo"
            } else {
                "https://api.binance.com/api/v3/exchangeInfo"
            }
        };
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .pool_idle_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("client: {}", e))?;
        let v: serde_json::Value = client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("exchangeInfo fetch: {}", e))?
            .json()
            .await
            .map_err(|e| format!("exchangeInfo parse: {}", e))?;
        let arr = v
            .get("symbols")
            .and_then(|s| s.as_array())
            .ok_or_else(|| "exchangeInfo: symbols dizisi yok".to_string())?;
        let mut entries: Vec<(String, String)> = Vec::with_capacity(arr.len());
        for s in arr {
            if let (Some(sym), Some(status)) = (
                s.get("symbol").and_then(|x| x.as_str()),
                s.get("status").and_then(|x| x.as_str()),
            ) {
                entries.push((sym.to_string(), status.to_string()));
            }
        }
        if entries.is_empty() {
            return Err("exchangeInfo: 0 sembol döndü".to_string());
        }
        let n = entries.len();
        let n_break = entries.iter().filter(|(_, s)| s != "TRADING").count();

        // Hot-path eligibility'nin okuduğu cache'i güncelle.
        set_symbol_statuses(&entries);

        // DB'ye persist (restart hydrate için) — senkron, spawn_blocking.
        let db_clone = db_path.clone();
        let entries_clone = entries.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(conn) = crate::persistence::open_db(&db_clone) {
                let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
                if let Err(e) =
                    crate::persistence::writer::save_symbol_statuses(&conn, &entries_clone)
                {
                    log::warn!("symbol_status persist: {:?}", e);
                }
            }
        })
        .await;

        push_state_log(
            state,
            format!(
                "🗂️ Sembol statü registry ✓ {} sembol ({} TRADING-dışı dışlandı)",
                n, n_break,
            ),
        );
        Ok(())
    }

    /// Binance Futures için kontrollü aday keşfi. Bu işlev yalnızca public ticker ve
    /// exchangeInfo verisini kullanır; mevcut pinned sembolleri değiştirmez ve adayları
    /// düşük bir üst sınırla worker registry'ye ekler. Emir kapıları yine account,
    /// reconciliation, risk ve order-filter kontrollerinden geçer.
    pub(crate) async fn run_binance_universe_discovery(
        state: &Arc<Mutex<AppState>>,
    ) -> std::result::Result<(), String> {
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
        if !binance_active {
            return Ok(());
        }
        let enabled = std::env::var("AUTO_BINANCE_DISCOVERY_ENABLED")
            .ok()
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if !enabled {
            return Ok(());
        }

        let max_candidates = std::env::var("AUTO_BINANCE_DISCOVERY_MAX")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(4)
            .clamp(1, 8);
        let min_quote_volume = std::env::var("AUTO_BINANCE_DISCOVERY_MIN_QUOTE_VOLUME")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(25_000_000.0)
            .max(0.0);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .pool_idle_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("binance discovery client: {}", e))?;
        let api_host = if std::env::var("BINANCE_TESTNET")
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false)
        {
            "https://testnet.binancefuture.com"
        } else {
            "https://fapi.binance.com"
        };
        let exchange_info: serde_json::Value = client
            .get(format!("{api_host}/fapi/v1/exchangeInfo"))
            .send()
            .await
            .map_err(|e| format!("binance discovery exchangeInfo: {}", e))?
            .json()
            .await
            .map_err(|e| format!("binance discovery exchangeInfo parse: {}", e))?;
        let tickers: serde_json::Value = client
            .get(format!("{api_host}/fapi/v1/ticker/24hr"))
            .send()
            .await
            .map_err(|e| format!("binance discovery ticker: {}", e))?
            .json()
            .await
            .map_err(|e| format!("binance discovery ticker parse: {}", e))?;

        let eligible: std::collections::HashSet<String> = exchange_info
            .get("symbols")
            .and_then(|value| value.as_array())
            .ok_or_else(|| "binance discovery: symbols dizisi yok".to_string())?
            .iter()
            .filter_map(|symbol| {
                let name = symbol.get("symbol")?.as_str()?.to_ascii_uppercase();
                let status = symbol.get("status")?.as_str()?;
                let quote_asset = symbol.get("quoteAsset")?.as_str()?;
                let contract_type = symbol.get("contractType")?.as_str()?;
                let excluded_shape = name.starts_with("1000")
                    || name.ends_with("UPUSDT")
                    || name.ends_with("DOWNUSDT")
                    || name.ends_with("BULLUSDT")
                    || name.ends_with("BEARUSDT");
                (status == "TRADING"
                    && quote_asset.eq_ignore_ascii_case("USDT")
                    && contract_type.eq_ignore_ascii_case("PERPETUAL")
                    && !excluded_shape)
                    .then_some(name)
            })
            .collect();

        let mut ranked: Vec<(String, f64)> = tickers
            .as_array()
            .ok_or_else(|| "binance discovery: ticker dizisi yok".to_string())?
            .iter()
            .filter_map(|ticker| {
                let symbol = ticker.get("symbol")?.as_str()?.to_ascii_uppercase();
                let quote_volume = ticker.get("quoteVolume")?.as_str()?.parse::<f64>().ok()?;
                (eligible.contains(&symbol)
                    && quote_volume.is_finite()
                    && quote_volume >= min_quote_volume)
                    .then_some((symbol, quote_volume))
            })
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        ranked.truncate(max_candidates);

        if ranked.is_empty() {
            return Err(format!(
                "binance discovery: eşik üstünde aday yok (min_quote_volume={:.0})",
                min_quote_volume
            ));
        }

        let symbols: Vec<String> = ranked
            .iter()
            .map(|(symbol, _)| format!("{}@binance", symbol))
            .collect();
        let interval = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            st.config.interval.clone()
        };
        let registered = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            let mut orchestrator = st
                .fleet
                .symbol_orchestrator
                .write()
                .map_err(|e| format!("symbol orchestrator lock: {}", e))?;
            symbols
                .iter()
                .filter(|symbol| {
                    orchestrator
                        .register(symbol, "futures", &interval)
                        .is_some()
                })
                .count()
        };

        push_state_log(
            state,
            format!(
                "🔎 Binance universe discovery ✓ {} aday seçildi, {} worker kaydedildi (min quote volume=${:.0})",
                symbols.len(),
                registered,
                min_quote_volume
            ),
        );
        Ok(())
    }

    /// MT5/IBKR bridge'lerinde sembol-listesi sözleşmesi olmadığı için adayları
    /// broker'a iki gerçek veri sorgusuyla doğrular. Başarısız adaylar registry'ye
    /// alınmaz; bu işlem execution'ı etkinleştirmez.
    pub(crate) async fn run_configured_venue_discovery(
        state: &Arc<Mutex<AppState>>,
        exchange: crate::core::types::Exchange,
        env_key: &str,
        tag: &str,
        market: &str,
    ) -> std::result::Result<(), String> {
        let raw = match std::env::var(env_key) {
            Ok(raw) if !raw.trim().is_empty() => raw,
            _ => return Ok(()),
        };
        let candidates: Vec<String> = raw
            .split(',')
            .map(|symbol| {
                symbol
                    .trim()
                    .trim_end_matches(&format!("@{}", tag))
                    .to_uppercase()
            })
            .filter(|symbol| !symbol.is_empty())
            .take(32)
            .collect();
        if candidates.is_empty() {
            return Ok(());
        }

        let adapter = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            st.venue_registry
                .get(exchange)
                .cloned()
                .ok_or_else(|| format!("{} adapter bulunamadı", tag))?
        };
        let mut validated = Vec::new();
        for symbol in candidates {
            let ticker = match adapter.book_ticker(&symbol).await {
                Ok((bid, ask)) if bid.is_finite() && ask.is_finite() && bid > 0.0 && ask >= bid => {
                    (bid, ask)
                }
                Ok((bid, ask)) => {
                    log::debug!(
                        "{} discovery {} reddedildi: geçersiz ticker {bid}/{ask}",
                        tag,
                        symbol
                    );
                    continue;
                }
                Err(error) => {
                    log::debug!("{} discovery {} reddedildi: ticker {}", tag, symbol, error);
                    continue;
                }
            };
            if adapter.symbol_filters(&symbol).await.is_err() {
                log::debug!("{} discovery {} reddedildi: symbol filters", tag, symbol);
                continue;
            }
            validated.push((symbol, ticker));
        }

        let interval = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            st.config.interval.clone()
        };
        let registered = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            let mut orchestrator = st
                .fleet
                .symbol_orchestrator
                .write()
                .map_err(|e| format!("symbol orchestrator lock: {}", e))?;
            validated
                .iter()
                .filter(|(symbol, _)| {
                    orchestrator
                        .register(&format!("{}@{}", symbol, tag), market, &interval)
                        .is_some()
                })
                .count()
        };
        push_state_log(
            state,
            format!(
                "🔎 {} universe discovery ✓ {}/{} aday doğrulandı, {} worker kaydedildi",
                tag.to_uppercase(),
                validated.len(),
                raw.split(',')
                    .filter(|value| !value.trim().is_empty())
                    .count()
                    .min(32),
                registered
            ),
        );
        Ok(())
    }

    /// 🧠 ML Retrain (Faz 4 - "ml" trigger):
    /// Aktif sembolde ParameterOptimizer.random_search çalıştırır, en iyi TP/SL/PS setini
    /// brain.best_params'a yazar ve config/best_params.json'a atomik mühürler.
    pub(crate) fn run_ml_retrain_job(
        state: &Arc<Mutex<AppState>>,
    ) -> std::result::Result<(), String> {
        // log::info!("🧠 E2: ML Retrain başlatıldı (random search 60 iter)...");
        push_state_log(
            state,
            format!("🧠 E2: ML Retrain başlatıldı (random search 60 iter)..."),
        );

        // 1) Çalışma sembolü ve mum kuyruğu — kilidi job boyunca tutmuyoruz.
        let (candidate_symbols, interval, db_path, capital, _market) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            let mut candidates = Vec::new();
            candidates.push(st.config.symbol.clone());
            candidates.extend(st.config.pinned_symbols.iter().cloned());
            if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                candidates.extend(orch.get_worker_status().into_iter().map(|w| w.symbol));
            }
            candidates.dedup();
            (
                candidates,
                st.config.interval.clone(),
                st.config.db_path.clone(),
                st.finance.equity,
                st.config.market.clone(),
            )
        };

        let (symbol, mut candles) = candidate_symbols
            .iter()
            .find_map(|candidate| {
                let loaded =
                    Self::read_candles_routed(state, &db_path, candidate, &interval, 1000).ok()?;
                (loaded.len() >= 50).then(|| (candidate.clone(), loaded))
            })
            .ok_or_else(|| {
                format!(
                    "yetersiz mum verisi: aktif adayların hiçbiri 50 muma ulaşmadı ({})",
                    candidate_symbols.join(",")
                )
            })?;

        push_state_log(
            state,
            format!(
                "🧠 ML Retrain başladı: sembol={} aralık={} kapital=${:.0}",
                symbol, interval, capital,
            ),
        );

        // Route-farkında okuma. Eski koddaki "etiket soyulmuş isimle fallback" yaması
        // artık gereksiz: read_candles_routed sembolü venue'ya çözerken ÇIPLAK forma
        // indirir ve doğru (exchange, market) kovasından okur — yamanın çözdüğü sorun
        // yapısal olarak ortadan kalktı (namespace migration + kanonik yazım).
        if candles.len() < 50 {
            let (bare, _tag) = crate::robot::venue::registry::split_venue_tag(&symbol);
            if bare != symbol {
                if let Ok(alt) = Self::read_candles_routed(state, &db_path, bare, &interval, 1000) {
                    if alt.len() > candles.len() {
                        candles = alt;
                    }
                }
            }
        }
        if candles.len() < 50 {
            return Err(format!("yetersiz mum verisi: {} mum", candles.len()));
        }

        let strategy_name = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            st.brain
                .live_strategy
                .read()
                .map(|s| s.clone())
                .unwrap_or_else(|_| "MA_CROSSOVER".into())
        };
        let strategy_name = if strategy_name.eq_ignore_ascii_case("default")
            || strategy_name.eq_ignore_ascii_case("auto")
            || strategy_name.is_empty()
        {
            "MA_CROSSOVER".to_string()
        } else {
            strategy_name
        };

        push_state_log(
            state,
            format!(
                "🧠 ML Retrain: {} mum yüklendi, strateji={}, random search 60 iter çalışıyor...",
                candles.len(),
                strategy_name,
            ),
        );
        Self::log_aux_research_snapshot(state, &db_path);

        // Giriş kalitesi edge filtresi (#4): ML retrain'in TP/SL/PS aramasının da
        // canlı edge hunisini görmesi için (best_params canlıya gider). run_backtest_job
        // ile aynı env + default. bkz parse_edge_filter.
        let edge_min = parse_edge_filter(
            std::env::var("BACKTEST_EDGE_FILTER").ok(),
            Some(Self::dynamic_edge_threshold(0.0)),
            Self::dynamic_edge_threshold(0.0),
        );
        // Canlı çıkış modeli (TP/SL re-opt): random_search da trailing + breakeven ile
        // birlikte arasın → best_params canlının gerçek çıkışını yansıtsın (run_backtest_job
        // ile aynı temsili mult: default_target / serinin_noise_floor, clamp[1.5,30]).
        let exit_trail_mult: f64 = {
            let default_target = state
                .lock()
                .ok()
                .and_then(|st| {
                    st.brain
                        .parameters
                        .read()
                        .ok()
                        .map(|p| p.target_trail_pct_for_strategy("default"))
                })
                .unwrap_or(0.7);
            match crate::robot::parameters::window_noise_floor_pct(&candles) {
                Some(nf) if nf > 0.0 => (default_target / nf).clamp(1.5, 30.0),
                _ => 2.0,
            }
        };
        let opt = crate::robot::backtester::parameter_optimizer::ParameterOptimizer::new(
            symbol.clone(),
            interval.clone(),
            capital,
            strategy_name.clone(),
        )
        .with_edge_min_score(edge_min)
        .with_exit_model(Some(exit_trail_mult), Some(1.0));
        let result = opt
            .random_search(&candles, 60)
            .map_err(|e| format!("random_search: {:?}", e))?;

        // 2) brain.best_params + ParameterStore.trade_risk'e yaz + ml_confidence güncelle.
        //    best_params HashMap legacy okuyucular için kalır; store yeni canonical
        //    kaynaktır (engine cycle pozisyon açılışta önce store'a bakar).
        let conf = (result.best_result.sharpe_ratio / 3.0).clamp(0.0, 1.0);
        {
            let mut st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            st.brain.best_params.insert(
                "take_profit_pct".into(),
                result.best_parameters.take_profit_pct,
            );
            st.brain
                .best_params
                .insert("stop_loss_pct".into(), result.best_parameters.stop_loss_pct);
            st.brain.best_params.insert(
                "max_position_size".into(),
                result.best_parameters.max_position_size,
            );
            st.brain
                .best_params
                .insert("ml_score".into(), result.best_result.sharpe_ratio);
            if let Ok(mut params) = st.brain.parameters.write() {
                params.apply_optimization(
                    result.best_parameters.take_profit_pct,
                    result.best_parameters.stop_loss_pct,
                    result.best_parameters.max_position_size,
                );
            }
            st.brain.ml_confidence = conf;
            st.brain.hyperopt_score = result.best_result.sharpe_ratio;
            st.push_log(format!(
                "🧠 ML Retrain ✓ {} TP={:.2}% SL={:.2}% PS={:.2} | Sharpe={:.2} ({} kombinasyon)",
                strategy_name,
                result.best_parameters.take_profit_pct,
                result.best_parameters.stop_loss_pct,
                result.best_parameters.max_position_size,
                result.best_result.sharpe_ratio,
                result.total_tested,
            ));
        }

        // 2b) GBT (Gradient Boosted Trees) eğitimi — cycle başına dinamik
        //     ml_confidence için. build_training_set forward-return işaretini
        //     hedef alır; gbt_grid_search hyperparam'i seçer; final model
        //     `IntelligenceHub.gbt`'ye yazılır. Yetersiz veri/eğitim
        //     başarısızlığı sessiz fallback: statik ml_confidence yolunda kalınır.
        let gbt_window_bars: usize = env_parse("GBT_WINDOW_BARS", 50);
        let gbt_forward_bars: usize = env_parse("GBT_FORWARD_BARS", 5);

        // 🌐 HEDEF MİMARİ: GBT regime AI'ı GENİŞ TF'de (4h/1d) çalışmalı → modeli de
        // HTF mumlarıyla eğit (regime_for_cycle skoru aynı TF'den besler, train/infer
        // tutarlı). multi_tf açık + GBT_TRAIN_HTF (default true) + HTF DB'de yeterli ise
        // HTF; değilse base TF'ye düş (eski davranış). HTF mumları candles tablosunda
        // gerçek interval string'iyle ("4h") durur → doğrudan read_candles.
        let multi_tf_enabled = state
            .lock()
            .ok()
            .and_then(|st| st.brain.parameters.read().ok().map(|p| p.multi_tf.enabled))
            .unwrap_or(true);
        let gbt_train_htf = !matches!(
            std::env::var("GBT_TRAIN_HTF").ok().as_deref(),
            Some("0") | Some("false") | Some("off"),
        );
        let htf_interval =
            crate::robot::data_pipeline::orchestrator::DataPipeline::get_htf_interval(&interval);
        // build_training_set(window+forward) için ≥ ~20 örnek → en az ~window+forward+20 mum.
        const GBT_HTF_MIN_BARS: usize = 80;
        let htf_candles: Vec<crate::core::types::Candle> =
            if gbt_train_htf && multi_tf_enabled && htf_interval != interval {
                Self::read_candles_routed(state, &db_path, &symbol, htf_interval, 2000)
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
        let (train_slice, train_interval): (&[crate::core::types::Candle], &str) =
            if htf_candles.len() >= GBT_HTF_MIN_BARS {
                (&htf_candles, htf_interval)
            } else {
                (&candles, interval.as_str())
            };
        let gbt_ds = crate::robot::ml_engine::build_training_set(
            train_slice,
            gbt_window_bars,
            gbt_forward_bars,
        );
        if gbt_ds.len() < 20 {
            push_state_log(
                state,
                format!(
                    "🌲 GBT atlandı: yetersiz training örneği ({} < 20)",
                    gbt_ds.len(),
                ),
            );
        } else {
            use crate::robot::ml_engine::{gbt_grid_search, GradientBoostedTrees};
            let tune = gbt_grid_search(&gbt_ds);
            let (n_est, lr, depth, oos_acc) = match tune {
                Some(r) => (r.n_estimators, r.learning_rate, r.max_depth, r.oos_accuracy),
                None => (5, 0.10, 3, f64::NAN),
            };
            let mut gbt = GradientBoostedTrees::new(n_est, lr, depth);
            gbt.train(&gbt_ds);
            let ready = gbt.is_ready();
            if ready {
                if let Ok(mut st) = state.lock() {
                    if let Ok(mut hub) = st.brain.intelligence_hub.write() {
                        hub.gbt = gbt;
                        // Hangi TF'de eğitildi → regime_for_cycle skoru aynı TF'den besler.
                        hub.gbt_trained_interval = Some(train_interval.to_string());
                    }
                    let acc_str = if oos_acc.is_nan() {
                        "-".into()
                    } else {
                        format!("{:.1}%", oos_acc)
                    };
                    st.push_log(format!(
                        "🌲 GBT ✓ TF={train_interval} n_est={n_est} lr={lr:.2} depth={depth} | OOS acc={acc_str} | {} örnek",
                        gbt_ds.len(),
                    ));
                }
            } else if let Ok(mut st) = state.lock() {
                st.push_log("🌲 GBT eğitim başarısız (is_ready=false)".into());
            }
        }

        // 3) Diske atomik mühürle (BestParamsCache → seal_config_to_disk).
        let snapshot = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            st.brain.best_params.clone()
        };
        crate::persistence::writer::seal_config_to_disk("config/best_params.json", &snapshot)
            .map_err(|e| format!("seal: {:?}", e))?;
        Ok(())
    }
}
