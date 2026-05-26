// src/robot/engines/master/loop_core.rs — Otonom döngü + cycle + edge/rejim yardımcıları
// Faz 1 modülerleştirme: master.rs'ten taşındı (davranış birebir korunur).
use super::*;

impl Engine {
    /// 🚀 ANA OTONOM DÖNGÜ (Engine Garnizonu Girişi)
    pub async fn run_autonomous_loop(state: Arc<Mutex<AppState>>) {
        log::info!("🚀 Master Engine Ateşlendi. Otonom devriye başlatıldı.");
        // Engine ateşlendi mesajını TUI log paneline de düşür + Booting fazı
        if let Ok(mut st) = state.lock() {
            st.fleet.phase = "Booting".into();
            st.push_log("🚀 Master Engine ateşlendi. Otonom devriye başladı.".into());
            // Pipeline timeline'ında 7 kanonik fazı baştan Idle olarak göster —
            // ilk cycle henüz çalışmadan bile TUI'de doğru sıralı görünür.
            if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                pipe.init_canon_stages();
            }
        }

        // 0a. SCHEMA GUARD: candles + open_positions_snapshot tablolarını
        //     defensive olarak yarat. Cold-start'ta hiç candle indirilmemişken
        //     ML retrain trigger'ı her 500ms "no such table: candles" hatasıyla
        //     log'u kirletiyordu. Tablo "CREATE IF NOT EXISTS" idempotent.
        Self::ensure_db_schema(&state);

        // 0b. POZİSYON RECOVERY: önceki run'un open_positions_snapshot tablosunu
        //     oku → live_positions'a hidrate et. Tablo yoksa veya boşsa sessizce
        //     geçer (cold-start). Recovery sayısı TUI log'a yansır.
        Self::hydrate_open_positions_from_db(&state).await;

        // 0c. ACCOUNT RECOVERY: önceki run'un equity/peak/closed_count'ını yükle.
        //     Yoksa cold-start (config.capital ile başla). Bu adım olmadan
        //     her restart equity'i 10000'e döndürüyordu → 44 saatte ~3500 USDT
        //     PnL kaybolmuş gibi görünüyordu (trades.jsonl ile tutarsızlık).
        Self::hydrate_account_state_from_db(&state);

        // 1. INFRASTRUCTURE FLEET (WS, Diagnostic, Pipeline)
        Self::spawn_infrastructure_fleet(Arc::clone(&state)).await;

        // Ana döngü heartbeat'i — TUI log paneline canlılık mesajı.
        // Daha önce 30 sn'de bir basıyordu, kullanıcı operatörlük açısından log
        // panelini gürültülü buluyor; default 5 dk'ya çıkarıldı.
        //   HEARTBEAT_UI_LOG_TICKS  → her N tick'te bir (500ms × tick). Default 600 (5 dk).
        //   HEARTBEAT_UI_LOG_DISABLE=1/true → tamamen kapat.
        // İlk tick log'u (tick_count == 1) "sistem ayakta" işareti olarak korunur,
        // sadece disable=true ise atılmaz.
        let heartbeat_log_disabled = env_truthy("HEARTBEAT_UI_LOG_DISABLE");
        let heartbeat_log_ticks: u64 = std::env::var("HEARTBEAT_UI_LOG_TICKS")
            .ok().and_then(|s| s.parse().ok()).filter(|n| *n > 0).unwrap_or(600);
        let mut tick_count: u64 = 0;
        loop {
            // Çıkış kontrolü + heartbeat tick mühürlemesi (her tur)
            let is_stop = {
                let mut st = state.lock().unwrap();
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs()).unwrap_or(0);
                st.fleet.last_loop_tick.store(now, Ordering::Relaxed);
                // Her turun başında fazı taze "Scanning"e çevir (execute_trade_cycle ve
                // perform_anomaly_recovery aksiyon yaparsa kendi içinde Executing/Recovering yazar).
                st.fleet.phase = "Scanning".into();
                st.app_stop_signal.load(Ordering::Relaxed)
            };
            if is_stop { break; }
            tick_count += 1;

            // Snapshot üretimi
            let snap = {
                let st = state.lock().unwrap();
                crate::core::bridge::get_snapshot(&st)
            };

            // 2. İNFAZ DÖNGÜSÜ (ML + Q-Table + Risk)
            Self::execute_trade_cycle(&state, &snap).await;

            // 3a. Stale anomaly purge: günlerce kalan Warning'leri (BEATUSDT/
            // BLESSUSDT ApiError gibi) active sayımdan düş. Eşik default 1800sn
            // (30 dk); env `ANOMALY_MAX_AGE_SECS` ile ayarlanır. Critical hiç
            // silinmez. Her 60 tick (~30sn) bir kontrol yeter.
            if tick_count % 60 == 0 {
                let max_age: u64 = env_parse("ANOMALY_MAX_AGE_SECS", 1800);
                let now_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                if let Ok(st) = state.lock() {
                    if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                        let n_purged = pipe.purge_stale_warnings(now_secs, max_age);
                        if n_purged > 0 {
                            log::info!("anomaly purge: {} stale Warning silindi (>{}sn)", n_purged, max_age);
                        }
                    }
                }
            }

            // 3. SAVUNMA (Anomali Onarımı) — aktif anomali varsa phase = Recovering
            Self::perform_anomaly_recovery(&state, &snap);

            // 4a. Equity tarihçesi: her 5 turda bir (≈2.5 sn) push edilir; sparkline ve drawdown.
            if tick_count % 5 == 0 {
                if let Ok(mut st) = state.lock() {
                    let equity = st.finance.equity;
                    if equity > st.finance.peak_equity { st.finance.peak_equity = equity; }
                    if let Ok(mut hist) = st.finance.equity_history.write() {
                        hist.push_back(equity);
                        while hist.len() > 120 { hist.pop_front(); }
                    }
                }
            }

            // 4b. IntelligenceHub güncellemesi: her 20 turda (≈10 sn)
            //  - FeatureVector çıkar → DriftDetector.update
            //  - should_retrain ise ml trigger pulse'u
            //  - tick_evolution (controller içinde N cycle'da bir gerçek evrim)
            if tick_count % 20 == 0 {
                Self::tick_intelligence_hub(&state).await;
            }

            // 4. Periyodik canlılık logu: HEARTBEAT_UI_LOG_TICKS (default 600 = 5 dk).
            // İlk turu da yakala (sistem ayakta işareti). disable=true ise hiç basma.
            if !heartbeat_log_disabled
                && (tick_count == 1 || tick_count % heartbeat_log_ticks == 0)
            {
                if let Ok(mut st) = state.lock() {
                    let n_open = st.finance.live_positions.read().map(|p| p.len()).unwrap_or(0);
                    let n_closed = st.finance.live_closed_trades.read().map(|t| t.len()).unwrap_or(0);
                    let n_anom = st.guardian.live_pipeline.read().map(|p| p.anomalies.len()).unwrap_or(0);
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

    /// ⚔️ STRATEJİK İNFAZ: Pozisyonların güncel fiyatla PnL'ini günceller ve sinyal avı yapar.
    ///
    /// Akış (Faz 3): live_price → mark-to-market → strateji seçimi (brain.live_strategy)
    /// → edge skoru (signal × ml_confidence) → RiskManager zinciri (Guardrails+Kelly+VaR+Gate)
    /// → aç/kapat kararı.
    pub(crate) async fn execute_trade_cycle(state: &Arc<Mutex<AppState>>, snap: &MissionControl) {
        // 1) Mark-to-market: aktif pozisyonların current_price'ı güncel.
        let (candidates, db_path, interval, live_strategy, ml_confidence, tuning) = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            let price_map = st.fleet.live_price.read().ok().map(|g| g.clone()).unwrap_or_default();
            if let Ok(mut positions) = st.finance.live_positions.write() {
                for pos in positions.values_mut() {
                    if let Some(&live) = price_map.get(&pos.symbol) {
                        if live > 0.0 { pos.current_price = live; }
                    }
                }
            }
            let mut candidates = Vec::new();
            // Canlı feed'i olmayan borsa sembolleri (örn. BIST) cycle'a alınmaz →
            // fiyatsız satırlar DataIngest/PriceFetch Failed → anomaly birikimi yapardı.
            // Karar market-agnostik tek noktada: RuntimeTuning.symbol_eligible_for_live.
            let tuning = Arc::clone(&st.tuning);
            if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                for worker in orch.get_worker_status() {
                    if !tuning.symbol_eligible_for_live(&worker.symbol) {
                        continue;
                    }
                    candidates.push(worker.symbol.clone());
                }
            }
            // Yetim pozisyonları da işle: orchestrator worker'ı yok ama açık pozisyon
            // var → SL/TP/Trailing denetimi en azından buradan akar, current_price güncel kalır.
            if let Ok(positions) = st.finance.live_positions.read() {
                for sym in positions.keys() {
                    if !tuning.symbol_eligible_for_live(sym) { continue; }
                    if !candidates.contains(sym) { candidates.push(sym.clone()); }
                }
            }
            let live_strategy = st.brain.live_strategy.read()
                .map(|s| s.clone()).unwrap_or_else(|_| "MA_CROSSOVER".to_string());
            (candidates, st.config.db_path.clone(), st.config.interval.clone(),
             live_strategy, st.brain.ml_confidence, tuning)
        };

        // 2) Paralel sembol infazı — her sembol için ayrı tokio task. State Arc<Mutex> üzerinden
        //    paylaşılır; lock contention'ı kısa tutmak için her closure içinde minimal scope kullanılır.
        //
        // Sıralı→paralel kazanımı: 100 sembol × 5 ms DB read = 500 ms (sıralı) ≈ 30-80 ms (paralel,
        // Tokio multi-thread). State mutex contention 100 sembolde de tolere edilebilir çünkü
        // her sembol için tek kısa açış+kapanış+okuma turu yapılır.
        let mut handles: Vec<tokio::task::JoinHandle<()>> = Vec::with_capacity(candidates.len());
        for symbol in candidates {
            let state_clone = Arc::clone(state);
            let db_path_c = db_path.clone();
            let interval_c = interval.clone();
            let live_strategy_c = live_strategy.clone();
            let snap_clone = snap.clone();
            let tuning_c = Arc::clone(&tuning);
            handles.push(tokio::spawn(async move {
                Self::process_symbol_cycle(
                    &state_clone, &symbol, &db_path_c, &interval_c,
                    &live_strategy_c, ml_confidence, &snap_clone, &tuning_c,
                ).await;
            }));
        }
        // Tüm sembollerin tamamlanmasını bekle (timeout yok — her biri kısa).
        for h in handles { let _ = h.await; }
    }

    /// Bir sembol için tam tur: exit denetimi → strateji üretimi → edge filtresi →
    /// risk gate → pozisyon aç/kapat. `execute_trade_cycle` her sembol için bu fonksiyonu
    /// paralel spawn eder.
    pub(crate) async fn process_symbol_cycle(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        db_path: &str,
        interval: &str,
        live_strategy: &str,
        ml_confidence: f64,
        snap: &MissionControl,
        tuning: &RuntimeTuning,
    ) {
        use crate::robot::data_pipeline::canon::PipelineStage;
        use crate::robot::data_pipeline::StepStatus;
        let risk_manager = crate::robot::risk::RiskManager::new();

        // Tek sembol için iş bloğu — orijinal `for symbol in candidates` gövdesinin içeriği.
        // Aşağıda `continue` yerine `return` kullanılır (kısa devre tek sembolde).
        {
            // ─── Faz 1 (DataIngest): SQLite'tan son 200 candle ────────────
            let candles = match Self::cycle_load_candles(state, symbol, db_path, interval, tuning) {
                Some(c) => c,
                None => return,
            };

            // === 1.5) AÇIK POZİSYON İSE: önce SL/TP/Trailing/Breakeven denetle ===
            // Tetiklenirse close_paper_position çağrılır ve tur biter.
            if Self::cycle_try_close_open_position(state, symbol, interval, &candles).await {
                return; // bu sembolde tur bitti, yeniden açılış aynı turda denenmesin
            }

            // ─── ScalpSwing A2: alt-kanal fırsat avı ──────────────────────────
            // scalp_swing_config enabled (SCALP_SWING_ENABLE=1) ise ScalpEngine
            // ve SwingEngine fırsat üretmeye çalışır; SlotGuard ile kanal-bazlı
            // limit + hedge kontrolü yapılır. Uygun ise açılış doğrudan
            // ScalpSwing patikasından gider (kind=Some(TradeType)) — bu turda
            // klasik strateji yolu (Strategy.generate_signal) pas geçilir.
            // Disabled ise sessiz false → eski davranış aynen.
            if Self::try_open_scalp_swing(state, symbol, &candles).await {
                return;
            }

            // 3) Strateji seçimi: brain.live_strategy "Default"/"AUTO" ise otonom seç.
            let strategy_name = if live_strategy.eq_ignore_ascii_case("default")
                                  || live_strategy.eq_ignore_ascii_case("auto")
                                  || live_strategy.is_empty() {
                if tuning.strategy_select_eval {
                    // Değerlendirme-tabanlı: her aday KENDİ resolve'lu paramıyla
                    // mini-backtest skoruna göre yarışır (param_spec optimizasyonu
                    // seçime de girer). Volatile rejimde IDLE savunması korunur.
                    use crate::robot::logic::market_regime::{detect_adx_regime, AdxRegime};
                    if matches!(detect_adx_regime(&candles), AdxRegime::Volatile) {
                        crate::robot::ml_engine::strategy_selector::IDLE_PROTECT.to_string()
                    } else {
                        let ps = state.lock().ok().map(|st| std::sync::Arc::clone(&st.brain.parameters));
                        let sel = crate::robot::strategies::strategy_selector::StrategySelector::from_registry(
                            &crate::robot::strategies::default_registry(),
                            &["SUPERTREND", "MA_CROSSOVER", "EMA_CROSSOVER", "RSI", "MACD", "BB", "DONCHIAN"],
                        );
                        let (best_name, _sig) = sel.select_best_name_resolved(&candles, |name| {
                            ps.as_ref()
                                .and_then(|p| p.read().ok().map(|g| g.resolve_strategy_params(name)))
                                .unwrap_or_default()
                        });
                        best_name
                    }
                } else {
                    // Default: rejim→strateji lookup (param-free, hızlı, kanıtlı savunma).
                    let sel = crate::robot::ml_engine::strategy_selector::StrategySelector::new();
                    sel.select_best(&candles, &crate::core::types::StrategyParams::default()).to_string()
                }
            } else {
                live_strategy.to_string()
            };

            // Savunma rejimleri (IDLE_PROTECT vb.) — kural IdleStrategyPolicy'de;
            // master.rs ve RoboticTradeExecutor aynı kararı tek source of truth
            // üzerinden okur (Faz 4 c4).
            if !crate::robot::execution::IdleStrategyPolicy
                .evaluate_name(Some(&strategy_name))
                .is_allow()
            {
                return;
            }

            let strategy = crate::robot::logic::optimizer::make_strategy_pub(&strategy_name);
            // Faz 2: yapısal (indikatör) parametreler artık ParameterStore'dan —
            // backtest job'ın param_spec araması ile bulduğu en iyi set (yoksa
            // default). Eskiden burada HER ZAMAN default() geçiliyordu → optimize
            // edilen indikatör paramları canlıya hiç ulaşmıyordu (kaçak). Tek-kaynak.
            let strat_params = state.lock().ok()
                .and_then(|st| st.brain.parameters.read().ok()
                    .map(|p| p.resolve_strategy_params(&strategy_name)))
                .unwrap_or_default();

            // ─── Multi-TF Faz B c2/c3: HTF mumlarını yükle (env+param gate) ───
            // load_htf_candles önce DB'den (HTF interval), yetersizse 1m'den
            // CandleSynth ile aggregate. Boş Vec → strategies/utils
            // htf_trend_filter `len() < slow` guard'ı ile pass-through yapar.
            // MULTI_TF_ENABLED=false → htf=None ile legacy single-TF davranış.
            let (multi_tf_enabled, multi_tf_min) = state.lock().ok()
                .and_then(|st| st.brain.parameters.read().ok()
                    .map(|p| (p.multi_tf.enabled, p.multi_tf.min_required)))
                .unwrap_or((true, crate::robot::data_pipeline::HTF_MIN_REQUIRED));
            let htf_candles_vec = if multi_tf_enabled {
                crate::robot::data_pipeline::load_htf_candles(
                    db_path, symbol, interval, multi_tf_min,
                )
            } else {
                Vec::new()
            };
            let htf_slice: Option<&[crate::core::types::Candle]> =
                if htf_candles_vec.is_empty() { None } else { Some(&htf_candles_vec) };

            // ─── Faz 3 (StrategyEval): sinyal üretimi ─────────────────────
            let signal = match strategy.generate_signal(&candles, &strat_params, None, htf_slice) {
                Ok(s) => {
                    Self::mark_pipeline_stage(state, PipelineStage::StrategyEval, StepStatus::Done);
                    s
                }
                Err(e) => {
                    Self::mark_pipeline_stage(state, PipelineStage::StrategyEval, StepStatus::Failed);
                    if let Some(logger) = state.lock().ok().and_then(|s| s.trading_logger.clone()) {
                        let ev = crate::robot::infra::logger::TradeEvent::error(
                            &format!("{} sinyal üretim hatası: {:?}", symbol, e),
                        );
                        let _ = logger.log_event(&ev);
                    }
                    return;
                }
            };

            // ─── Faz 2 (FeatureExtract): edge skoru + ATR + (gerek olursa) ML feature.
            // compute_edge_score momentum'u ATR ile normalize edip ML confidence ile harmanlar;
            // başka indikatör/feature hesapları (S/R, regime classify) cycle dışında periyodik
            // task'larda yapılır. Bu nokta tek atımlık feature extraction'ı temsil eder.
            //
            // GBT inference (cycle başına dinamik ml_confidence): IntelligenceHub.gbt
            // hazırsa son ~50 mumdan FeatureVector → predict_confidence(fv, signal)
            // hibrit conf üretir; yoksa eski statik brain.ml_confidence yolu.
            // Sinyal yönü + GBT skoru uyumu cycle bazında değişir → her sembolde
            // farklı conf üretebilir.
            let ml_conf_used: f64 = {
                let gbt_conf = if candles.len() >= 30 {
                    let tail = &candles[candles.len().saturating_sub(200)..];
                    let fv = crate::robot::ml_engine::FeatureExtractor::extract(tail);
                    state.lock().ok().and_then(|st| {
                        st.brain.intelligence_hub.read().ok()
                            .and_then(|hub| hub.predict_confidence(&fv, &signal))
                    })
                } else { None };
                // GBT canlı çalıştığında brain.ml_confidence'ı last-cycle örneği
                // ile güncelle → heartbeat ve operatör dinamik değeri görür.
                // Yoksa retrain sonrası set edilen sharpe-bazlı statik değer kalır.
                if let Some(c) = gbt_conf {
                    if let Ok(mut st) = state.lock() {
                        st.brain.ml_confidence = c;
                    }
                }
                gbt_conf.unwrap_or(ml_confidence)
            };

            let edge = Self::compute_edge_score(&candles, &signal, ml_conf_used);
            Self::mark_pipeline_stage(state, PipelineStage::FeatureExtract, StepStatus::Done);
            // ML henüz hazır değilse (cold-start) gevşek eşik; modele güven arttıkça katılaşır.
            // Faz 2 c4: edge_threshold rejim-bazlı override'a açık.
            // Faz 3 c1: rejim ilk kez görülüyorsa adaptive heuristic patch otomatik.
            // Faz 3 c3: rejim drift değişimi → ekstra savunmacı tighten + bildirim.
            let regime = Self::classify_regime(&candles);
            Self::ensure_regime_patch(state, regime.as_str());
            Self::observe_regime_drift(state, regime.as_str());
            let edge_threshold = state.lock().ok()
                .and_then(|st| st.brain.parameters.read().ok()
                    .map(|p| p.edge_threshold_for(regime.as_str(), ml_conf_used)))
                .unwrap_or_else(|| Self::dynamic_edge_threshold(ml_conf_used));
            // Aday log eşiği: kabul edilen edge'in %75'inin altındaki sinyaller spam sayılır.
            let edge_log_floor = edge_threshold * 0.75;

            // Pozisyonun yönü: None = pozisyon yok, Some(true) = LONG, Some(false) = SHORT.
            // Yön bilgisi kritik; aksi halde aynı yöndeki tekrar sinyalleri pozisyonu kapatır
            // ve aç/kapa döngüsü oluşur (komisyon erozyonu).
            let pos_dir: Option<bool> = {
                let st = match state.lock() { Ok(s) => s, Err(_) => return };
                st.finance.live_positions.read().ok()
                    .and_then(|p| p.get(symbol).map(|pos| pos.is_long))
            };

            let signal_label = match signal {
                Signal::Buy => "BUY", Signal::Sell => "SELL", Signal::Hold => "HOLD",
            };

            // SIGNAL eventi: yalnız Buy/Sell için logla (HOLD spam yapmasın).
            if matches!(signal, Signal::Buy | Signal::Sell) {
                // Referans fiyat: fleet.live_price (5sn REST) > candle close — exit
                // denetimiyle (cycle_try_close_open_position) birebir aynı öncelik.
                let signal_price = state.lock().ok()
                    .and_then(|st| st.fleet.live_price.read().ok()
                        .and_then(|m| m.get(symbol).copied()))
                    .filter(|&v| v > 0.0)
                    .unwrap_or_else(|| candles.last().map(|c| c.close).unwrap_or(0.0));
                if let Some(logger) = state.lock().ok().and_then(|s| s.trading_logger.clone()) {
                    let ev = crate::robot::infra::logger::TradeEvent::signal(symbol, signal, signal_price);
                    let _ = logger.log_event(&ev);
                }
            }

            match (signal, pos_dir) {
                // Pozisyon yokken: yalnız yüksek edge'de açılış denenir.
                (crate::core::types::Signal::Buy, None) | (crate::core::types::Signal::Sell, None) => {
                    if edge < edge_threshold {
                        // Spam'i kısmak için sadece eşiğe yakın aday sinyalleri logla.
                        if edge >= edge_log_floor {
                            push_state_log(state, format!(
                                "📊 {} {} edge={:.2} eşik={:.2} ⇒ REDDEDİLDİ (zayıf edge, strat={})",
                                symbol, signal_label, edge, edge_threshold, strategy_name,
                            ));
                        }
                        return;
                    }
                    // ─── Faz 4 (RiskGate): Guardrails + Kelly + VaR + Gate ───
                    // Notional yaklaşımı: equity'nin %10'u (RiskGatePolicy ayrıca clamp eder).
                    let req_notional = snap.finance.total_equity * 0.10;
                    let decision = risk_manager.authorize(&signal, snap, edge, req_notional);
                    if let crate::robot::risk::risk_gate::RiskDecision::Deny { reasons, enter_safe_mode, halt } = decision {
                        Self::mark_pipeline_stage(state, PipelineStage::RiskGate, StepStatus::Skipped);
                        let mode = if halt { "HALT" }
                            else if enter_safe_mode { "SAFE-MODE" }
                            else { "DENY" };
                        let block_reason = format!("[{}] {}", mode, reasons.join(" · "));
                        // 60sn throttle per sembol: aynı sembolde Kelly edge negatif sebebiyle
                        // her cycle (500ms) blok log'u oluşuyordu → olay günlüğü saniyede 6+ satır
                        // birikti, gerçek olaylar görünmüyordu. HALT ise throttle yok (kritik).
                        let should_log = halt
                            || log_throttle_should_emit(symbol, "risk_block_safemode", 60);
                        if should_log {
                            push_state_log(state, format!(
                                "🛡️ {} {} edge={:.2} ✓ ama RiskManager [{}]: {}",
                                symbol, signal_label, edge, mode, reasons.join(" · "),
                            ));
                        }
                        if let Some(logger) = state.lock().ok().and_then(|s| s.trading_logger.clone()) {
                            let ev = crate::robot::infra::logger::TradeEvent::risk_block(
                                &block_reason, symbol,
                            );
                            let _ = logger.log_event(&ev);
                        }
                        return;
                    }
                    Self::mark_pipeline_stage(state, PipelineStage::RiskGate, StepStatus::Done);
                    push_state_log(state, format!(
                        "📊 {} {} edge={:.2} ✓ + risk ✓ ⇒ POZİSYON AÇILIYOR (strat={})",
                        symbol, signal_label, edge, strategy_name,
                    ));
                    Self::open_paper_position(state, symbol, &signal, &candles, &strategy_name, None).await;
                }
                // Pozisyon varken TERS yönde sinyal → kapanış (edge filtresi gevşek).
                // Long + Sell ya da Short + Buy: trend dönmüş demektir.
                // Log'u close_paper_position'a delege ediyoruz: yaş guard'ı reddederse
                // "⇒ KAPANIŞ" yanıltıcı olur (TUI'de saniyede 1 satır çift log spamı
                // oluyordu: "⇒ KAPANIŞ" + "⏳ erken kapanış reddedildi"). Kapandığında
                // close_paper_position kendi emoji'li başarı satırını basar.
                (crate::core::types::Signal::Sell, Some(true))
                | (crate::core::types::Signal::Buy,  Some(false)) => {
                    let _ = (signal_label, edge); // log kaldırıldı; vars artık kullanılmıyor
                    Self::close_paper_position(state, symbol, &candles, ExitReason::StrategySignal).await;
                }
                // Aynı yöndeki tekrar sinyaller: pozisyon zaten o yönde, dokunma.
                // (Aksi halde aç/kapa döngüsü ve komisyon erozyonu doğar.)
                // Görünürlük: throttle'lı RISK_BLOCK olarak işaretle → operatör
                // "neden trade yok?" sorusunu trades.jsonl'dan yanıtlayabilsin.
                // Throttle: cycle başına SIGNAL bastığımız için 24h'de bir sembolde
                // ~2000+ aynı satır birikiyordu (audit'te %99.96 RISK_BLOCK position-aligned).
                // log_throttle_should_emit ile sembol+kind başına default 60sn cooldown.
                (crate::core::types::Signal::Buy,  Some(true))
                | (crate::core::types::Signal::Sell, Some(false)) => {
                    let cooldown = tuning.risk_block_log_cooldown_secs;
                    if log_throttle_should_emit(symbol, "risk_block_pos_aligned", cooldown) {
                        if let Some(logger) = state.lock().ok().and_then(|s| s.trading_logger.clone()) {
                            let dir_label = if matches!(signal, Signal::Buy) { "LONG" } else { "SHORT" };
                            let ev = crate::robot::infra::logger::TradeEvent::risk_block(
                                &format!("[position-aligned] {} sinyali, pozisyon zaten {}", signal_label, dir_label),
                                symbol,
                            );
                            let _ = logger.log_event(&ev);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    /// Faz 1 (DataIngest): sembol için son 200 mumu DB'den okur. Empty/Err
    /// durumlarında pipeline aşamasını işaretler + throttle'lı log basar ve None
    /// döner (caller cycle'ı kısa-devre eder). process_symbol_cycle'dan birebir taşındı.
    pub(crate) fn cycle_load_candles(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        db_path: &str,
        interval: &str,
        tuning: &RuntimeTuning,
    ) -> Option<Vec<Candle>> {
        use crate::robot::data_pipeline::canon::PipelineStage;
        use crate::robot::data_pipeline::StepStatus;
        // Üç ayrım: Ok(non-empty) Done. Ok(empty) sessiz Failed (sembol için 1m
        // candle DB'de yok = veri kaynağı eksikliği, alarm değil). Err = gerçek DB hatası.
        match crate::persistence::reader::read_candles(db_path, symbol, interval, 200) {
            Ok(c) if !c.is_empty() => {
                Self::mark_pipeline_stage(state, PipelineStage::DataIngest, StepStatus::Done);
                Some(c)
            }
            Ok(_) => {
                let cooldown = tuning.log_dataingest_cooldown_secs;
                if log_throttle_should_emit(symbol, "dataingest_empty", cooldown) {
                    log::warn!("DataIngest empty: {} {} (candles tablo'da 1m kayıt yok)", symbol, interval);
                }
                if let Ok(st) = state.lock() {
                    if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                        pipe.mark_stage_completed(PipelineStage::DataIngest, StepStatus::Failed);
                    }
                }
                None
            }
            Err(e) => {
                let cooldown = tuning.log_dataingest_cooldown_secs;
                if log_throttle_should_emit(symbol, "dataingest_error", cooldown) {
                    log::warn!("DataIngest error: {} {} → {}", symbol, interval, e);
                }
                Self::mark_pipeline_stage(state, PipelineStage::DataIngest, StepStatus::Failed);
                None
            }
        }
    }

    /// Açık pozisyon varsa en taze fiyatla (fleet.live_price > candle close) SL/TP/
    /// Trailing/Breakeven denetler; tetiklenirse kapatır. `true` → bu sembolde tur
    /// bitti (caller return etmeli). Pozisyon yok / exit yok → `false`.
    /// process_symbol_cycle'dan birebir taşındı (lock skopları dahil).
    pub(crate) async fn cycle_try_close_open_position(
        state: &Arc<Mutex<AppState>>,
        symbol: &str,
        interval: &str,
        candles: &[Candle],
    ) -> bool {
        // En taze fiyat önceliği: 1) fleet.live_price (5sn REST), 2) candle close.
        let candle_close = candles.last().map(|c| c.close).unwrap_or(0.0);
        let atr_value = Self::calc_atr(candles, 14);
        let (live_price, exit_reason) = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return true };
            // ATR-trail mult: sembol×interval noise floor + pozisyonu açan stratejinin
            // target_pct'i. pos.trade_type açılışta strategy_name ile mühürleniyor.
            let default_mult = st.brain.best_params.get("pos_atr_trail_mult").copied().unwrap_or(2.0);
            let pos_strategy: String = st.finance.live_positions.read().ok()
                .and_then(|m| m.get(symbol).map(|p| p.trade_type.clone()))
                .unwrap_or_else(|| "default".to_string());
            let atr_mult = st.brain.parameters.read().ok()
                .map(|p| p.resolve_atr_mult(symbol, interval, &pos_strategy, default_mult))
                .unwrap_or(default_mult);
            let be_rr = st.brain.best_params.get("pos_breakeven_at_rr").copied().unwrap_or(1.0);
            let fleet_price = st.fleet.live_price.read().ok()
                .and_then(|m| m.get(symbol).copied())
                .filter(|&v| v > 0.0);
            let live_price = fleet_price.unwrap_or(candle_close);
            let reason_opt = if let Ok(mut positions) = st.finance.live_positions.write() {
                if let Some(pos) = positions.get_mut(symbol) {
                    pos.current_price = live_price;
                    Self::check_exit_conditions(pos, live_price, atr_value, atr_mult, be_rr)
                } else { None }
            } else { None };
            (live_price, reason_opt)
        };
        if let Some(reason) = exit_reason {
            push_state_log(state, format!(
                "{} {} {} koşulu tetiklendi @ {:.4}",
                reason.emoji(), symbol, reason.as_str(), live_price,
            ));
            Self::close_paper_position(state, symbol, candles, reason).await;
            return true;
        }
        false
    }


    /// Edge skoru: son 20 mumun momentum gücü (ATR'ye göre normalize) ile ML confidence ortalaması.
    /// Sinyal yönü momentum ile uyumlu değilse ceza uygulanır.
    ///
    /// Momentum gücü = |ham getiri / ATR%|, 1.0'a clamp'lenir. Yani 20 mum içinde fiyatın ATR'nin
    /// en az 1 katı yön yapması "tam güç" sayılır. Ham getiriyi kullanmak yerine ATR normalizasyonu
    /// 1m gibi düşük volatilite timeframe'lerinde edge'in pratik olarak ölçülebilir kalmasını sağlar.
    pub(crate) fn compute_edge_score(candles: &[Candle], signal: &Signal, ml_confidence: f64) -> f64 {
        // Canlı yol sabit ters-momentum cezası 0.4 ile çağırır (davranış birebir).
        // Parametrik sürüm backtest A/B'si (#3) için ayrı eşik denemesine izin verir.
        Self::compute_edge_score_with(candles, signal, ml_confidence, 0.4)
    }

    /// `compute_edge_score`'un ters-momentum cezası parametrik sürümü. `reverse_penalty`:
    /// sinyal momentumla ters yöndeyse uygulanan dir_match çarpanı (canlı: 0.4).
    /// Düşürmek (örn. 0.2) ters girişleri daha çok bastırır; 1.0 cezayı kaldırır.
    pub(crate) fn compute_edge_score_with(
        candles: &[Candle], signal: &Signal, ml_confidence: f64, reverse_penalty: f64,
    ) -> f64 {
        if candles.len() < 20 { return 0.0; }
        let recent = &candles[candles.len() - 20..];
        let first = recent.first().map(|c| c.close).unwrap_or(0.0);
        let last  = recent.last().map(|c| c.close).unwrap_or(0.0);
        if first <= 0.0 || last <= 0.0 { return 0.0; }
        let mom = ((last - first) / first).clamp(-1.0, 1.0); // göreli getiri

        // Momentum'u ATR%'ye göre normalize et: kaç ATR yön yapıldı?
        let atr = Self::calc_atr(candles, 14);
        let atr_pct = if last > 0.0 { (atr / last).max(1e-6) } else { 1e-6 };
        let mom_strength = if atr_pct > 1e-6 {
            (mom.abs() / atr_pct).clamp(0.0, 1.0)
        } else {
            mom.abs().clamp(0.0, 1.0)
        };

        let dir_match = match signal {
            Signal::Buy  if mom > 0.0  => 1.0,
            Signal::Sell if mom < 0.0  => 1.0,
            Signal::Hold               => 0.0,
            _                          => reverse_penalty, // ters yön sinyali → ceza
        };
        let ml = ml_confidence.clamp(0.0, 1.0);
        // ML henüz hazır değilse (0.0) momentum tek başına baskın olsun.
        let ml_w = if ml < f64::EPSILON { 0.0 } else { 0.5 };
        let mom_w = 1.0 - ml_w;
        (dir_match * (mom_strength * mom_w + ml * ml_w)).clamp(0.0, 1.0)
    }

    /// Dinamik edge eşiği: ML modeli henüz hazır değilken (confidence ≈ 0) momentum tek başına
    /// taşıyıcı, bu yüzden daha gevşek eşik. ML hazırlandıkça daha katı bir filtreye geçilir.
    pub(crate) fn dynamic_edge_threshold(ml_confidence: f64) -> f64 {
        if ml_confidence < 0.05 { 0.20 }
        else if ml_confidence < 0.30 { 0.35 }
        else { 0.55 }
    }

    /// Faz 3 c3: rejim drift gözlemi. Önceki cycle'dan farklı bir rejime
    /// geçildiyse store kendi içinde patch'i bir basamak daha sıkılaştırır;
    /// burada push_alert ile kullanıcıya bildirim gönderiyoruz (Telegram + UI log).
    /// İlk gözlem değişim sayılmaz (cold start).
    pub(crate) fn observe_regime_drift(state: &Arc<Mutex<AppState>>, regime: &str) {
        let drift = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            st.brain.parameters.write().ok().map(|mut p| p.observe_regime(regime)).unwrap_or(false)
        };
        if !drift { return; }
        if let Ok(mut st) = state.lock() {
            let key = format!("REGIME-DRIFT-{}", regime);
            let msg = format!(
                "🌪️ Rejim drift: '{}' → patch sıkılaştırıldı (savunmacı duruş)",
                regime,
            );
            st.push_alert(
                &key,
                crate::robot::infra::telegram_notifier::Severity::Warning,
                msg,
            );
        }
    }

    /// Faz 3 c1: ilk kez görülen rejim için ParameterStore'da otomatik heuristic
    /// patch yerleştirir. Patch zaten varsa (manuel ya da önceki cycle'da yazıldı)
    /// dokunmaz — HyperOpt veya manuel override'ın ezilmesini önler.
    /// Boş heuristic patch ise (Weak* / Unknown) hiç yazılmaz.
    pub(crate) fn ensure_regime_patch(state: &Arc<Mutex<AppState>>, regime: &str) {
        let needs_patch = state.lock().ok()
            .and_then(|st| st.brain.parameters.read().ok()
                .map(|p| !p.regime_overrides.contains_key(regime)))
            .unwrap_or(false);
        if !needs_patch { return; }

        let patch = crate::robot::parameters::adaptive::default_patch_for_regime(regime);
        if patch.is_empty() { return; }

        if let Ok(mut st) = state.lock() {
            if let Ok(mut params) = st.brain.parameters.write() {
                params.set_regime_patch(regime, patch);
            }
            st.push_log(format!(
                "📐 Rejim '{}' ilk kez görüldü → adaptive patch uygulandı (Faz 3)",
                regime,
            ));
        }
    }

    /// Pipeline canon aşamasını "bitti" olarak işaretler ve Failed/Skipped
    /// durumlarda otomatik bir Anomaly emit eder (TUI Pipeline sekmesindeki
    /// "🛡️ Aktif Anomaliler" panelinde gözükür).
    ///
    /// - `Done` → sadece chain_steps güncellenir.
    /// - `Failed` → Critical anomaly + chain_steps.
    /// - `Skipped` → Warning anomaly + chain_steps (örn. RiskGate Deny; bot
    ///   sinyali bilerek reddetti, kullanıcının görmesi faydalı).
    /// - diğer (Idle/Running) → sadece chain_steps; anomaly üretilmez.
    ///
    /// state.lock() + live_pipeline.write() kısa scope'lu; lock contention yok.
    pub(crate) fn mark_pipeline_stage(
        state: &Arc<Mutex<AppState>>,
        stage: crate::robot::data_pipeline::canon::PipelineStage,
        status: crate::robot::data_pipeline::StepStatus,
    ) {
        use crate::robot::data_pipeline::{AnomalyKind, AnomalySeverity, StepStatus};
        if let Ok(st) = state.lock() {
            if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                pipe.mark_stage_completed(stage, status);
                let (severity, kind, msg) = match status {
                    StepStatus::Failed => (
                        AnomalySeverity::Critical,
                        AnomalyKind::DataStall,
                        format!("{} fazı başarısız: cycle bu aşamada koptu", stage.label()),
                    ),
                    StepStatus::Skipped => (
                        AnomalySeverity::Warning,
                        AnomalyKind::RiskBreach,
                        format!("{} fazı atlandı: koruma/red akışı tetiklendi", stage.label()),
                    ),
                    _ => return, // Done / Idle / Running anomaly üretmez
                };
                pipe.push_anomaly(severity, kind, msg);
            }
        }
    }

    /// 🌐 Mum dizisinden evolution::MarketRegime çıkar (IntelligenceHub'a yöne duyarlı sinyal).
    /// AdxRegime'i momentumla zenginleştirir.
    pub(crate) fn classify_regime(candles: &[Candle]) -> crate::evolution::MarketRegime {
        use crate::evolution::MarketRegime;
        use crate::robot::logic::market_regime::{detect_adx_regime, AdxRegime};
        if candles.len() < 20 { return MarketRegime::Unknown; }
        let adx = detect_adx_regime(candles);
        let recent = &candles[candles.len() - 20..];
        let first = recent.first().map(|c| c.close).unwrap_or(0.0);
        let last  = recent.last().map(|c| c.close).unwrap_or(0.0);
        if first <= 0.0 { return MarketRegime::Unknown; }
        let mom_pct = (last - first) / first * 100.0;
        match adx {
            AdxRegime::Volatile => MarketRegime::HighVolatility,
            AdxRegime::Ranging  => MarketRegime::Ranging,
            AdxRegime::Trending if mom_pct >  2.0 => MarketRegime::StrongUptrend,
            AdxRegime::Trending if mom_pct >  0.0 => MarketRegime::WeakUptrend,
            AdxRegime::Trending if mom_pct < -2.0 => MarketRegime::StrongDowntrend,
            AdxRegime::Trending                   => MarketRegime::WeakDowntrend,
            AdxRegime::Neutral if mom_pct.abs() < 0.5 => MarketRegime::LowVolatility,
            AdxRegime::Neutral                        => MarketRegime::Unknown,
        }
    }

    /// 📏 ATR (Average True Range) — son N mum üzerinde Wilder-style.
    /// Tarihçe yetersizse 0.0 döner (trailing devre dışı sayılır).
    pub(crate) fn calc_atr(candles: &[Candle], period: usize) -> f64 {
        let n = candles.len();
        if n < period + 1 { return 0.0; }
        let slice = &candles[n - period - 1..];
        let mut sum = 0.0;
        for w in slice.windows(2) {
            let prev = &w[0]; let cur = &w[1];
            let h_l = cur.high - cur.low;
            let h_pc = (cur.high - prev.close).abs();
            let l_pc = (cur.low  - prev.close).abs();
            sum += h_l.max(h_pc).max(l_pc);
        }
        sum / period as f64
    }
}
