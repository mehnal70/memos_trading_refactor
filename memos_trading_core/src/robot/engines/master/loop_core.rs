// src/robot/engines/master/loop_core.rs — Otonom döngü + cycle orkestrasyonu.
// Faz 2 modülerleştirme: edge/rejim yardımcıları edge_regime.rs'e ayrıldı (davranış birebir).
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
        Self::hydrate_symbol_status_from_db(&state);

        // 0c2. SEED REGISTRY-PRUNE: registry hydrate edildikten SONRA, force-pinned seed'lerden
        //      açıkça dışlanmışları (delisted-skip / exchangeInfo non-TRADING) at → ölü seed canlıda
        //      purge gürültüsü yapmasın. LENIENT (bilinmeyen korunur). report ÖNCESİ → log nihai seti.
        Self::prune_seed_ineligible(&state);

        // 0d. EDGE SEED GÖRÜNÜRLÜĞÜ: EDGE_SEED_REPORT ile yüklenen per-symbol stratejileri
        //     TUI state-log paneline düşür (TUI'de logger backend yok → log::info! görünmez).
        Self::report_edge_seed(&state);

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
                let now = crate::core::time::now_epoch_secs();
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
            if tick_count.is_multiple_of(60) {
                let max_age: u64 = env_parse("ANOMALY_MAX_AGE_SECS", 1800);
                let now_secs = crate::core::time::now_epoch_secs();
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
            if tick_count.is_multiple_of(5) {
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
            if tick_count.is_multiple_of(20) {
                Self::tick_intelligence_hub(&state).await;
            }

            // 4. Periyodik canlılık logu: HEARTBEAT_UI_LOG_TICKS (default 600 = 5 dk).
            // İlk turu da yakala (sistem ayakta işareti). disable=true ise hiç basma.
            if !heartbeat_log_disabled
                && (tick_count == 1 || tick_count.is_multiple_of(heartbeat_log_ticks))
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
        let (candidates, db_path, interval, symbol_interval, symbol_strategy, symbol_tracks, live_strategy, ml_confidence, tuning) = {
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
            // 📐 Kesitsel ADANMIŞ MOD sepeti: bu semboller SADECE process_xs_book (market-nötr kitap)
            // tarafından yönetilir → normal per-sembol döngüden + yetim-pozisyon yolundan HARİÇ tutulur
            // (çift-yönetim/çakışma yok, tek-pozisyon invariantı temiz). Mod kapalı → boş set, sıfır regresyon.
            let xs_basket: std::collections::HashSet<String> = st.brain.parameters.read().ok()
                .filter(|p| p.xs_live.enabled)
                .map(|p| p.xs_live.symbols.iter().cloned().collect())
                .unwrap_or_default();
            // Canlı feed'i olmayan borsa sembolleri (örn. BIST) cycle'a alınmaz →
            // fiyatsız satırlar DataIngest/PriceFetch Failed → anomaly birikimi yapardı.
            // Karar market-agnostik tek noktada: RuntimeTuning.symbol_eligible_for_live.
            let tuning = Arc::clone(&st.tuning);
            if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                for worker in orch.get_worker_status() {
                    if !tuning.symbol_eligible_for_live(&worker.symbol) {
                        continue;
                    }
                    if xs_basket.contains(&worker.symbol) { continue; } // XS-yönetimli → atla
                    candidates.push(worker.symbol.clone());
                }
            }
            // Yetim pozisyonları da işle: orchestrator worker'ı yok ama açık pozisyon
            // var → SL/TP/Trailing denetimi en azından buradan akar, current_price güncel kalır.
            if let Ok(positions) = st.finance.live_positions.read() {
                for sym in positions.keys() {
                    if !tuning.symbol_eligible_for_live(sym) { continue; }
                    if xs_basket.contains(sym) { continue; } // XS-yönetimli pozisyon → kitap yönetir
                    if !candidates.contains(sym) { candidates.push(sym.clone()); }
                }
            }
            let live_strategy = st.brain.live_strategy.read()
                .map(|s| s.clone()).unwrap_or_else(|_| "MA_CROSSOVER".to_string());
            // Per-sembol otonom interval + strateji + çoklu-iz map'leri (boş → global/auto'ya düşer).
            let (symbol_interval, symbol_strategy, symbol_tracks) = st.brain.parameters.read().ok()
                .map(|p| (p.symbol_interval.clone(), p.symbol_strategy.clone(), p.symbol_tracks.clone()))
                .unwrap_or_default();
            (candidates, st.config.db_path.clone(), st.config.interval.clone(),
             symbol_interval, symbol_strategy, symbol_tracks, live_strategy, st.brain.ml_confidence, tuning)
        };

        // 📐 KESİTSEL ADANMIŞ MOD: sepeti tek market-nötr kitap olarak yönet (per-sembol döngüden ÖNCE,
        // sepet sembolleri candidates'tan zaten hariç). Mod kapalı/sepet yetersiz → anında no-op.
        Self::process_xs_book(state).await;

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
            // Per-sembol otonom interval; map'te yoksa config.interval (sıfır regresyon).
            let interval_c = symbol_interval.get(&symbol).cloned().unwrap_or_else(|| interval.clone());
            // Per-sembol otonom strateji (precedence 1); yoksa global live_strategy ("auto" ise
            // process_symbol_cycle içinde select_best'e düşer). [[project_edge_scan]].
            let seed_strat = symbol_strategy.get(&symbol).cloned()
                .filter(|s| !s.trim().is_empty());
            // Keşfedilmiş açık edge ataması var mı → fırsatçı ScalpSwing bu sembolde pas geçilir
            // (seed_strategy_priority açıkken). Yoksa sembol global/auto'ya düşer + ScalpSwing avlanır.
            let has_seed_strategy = seed_strat.is_some();
            let live_strategy_c = seed_strat.unwrap_or_else(|| live_strategy.clone());
            // 🪢 Çoklu-iz: sembolün >1 WF-onaylı (TF,strateji) edge'i varsa (EDGE_SEED_MULTI_TF açık →
            // symbol_tracks dolu) izleri SIRALI dener; aksi (boş) → tek-edge anchor. Boş Vec = eski yol.
            let tracks: Vec<(String, String)> = symbol_tracks.get(&symbol).cloned().unwrap_or_default();
            let snap_clone = snap.clone();
            let tuning_c = Arc::clone(&tuning);
            handles.push(tokio::spawn(async move {
                if tracks.len() > 1 {
                    // Tek-pozisyon invariantı: yalnız FLAT iken izleri dene; pozisyon açıkken
                    // tek anchor çağrı (exit yönetimi, iz-arası ani re-entry churn'ü önlenir).
                    // Her iz KENDİ TF mumunu yükler (1d çapa + 1h/30m daha sık fırsat); ilk açan durur.
                    if symbol_has_open_position(&state_clone, &symbol) {
                        Self::process_symbol_cycle(
                            &state_clone, &symbol, &db_path_c, &interval_c,
                            &live_strategy_c, true, ml_confidence, &snap_clone, &tuning_c,
                        ).await;
                    } else {
                        for (iv, strat) in &tracks {
                            Self::process_symbol_cycle(
                                &state_clone, &symbol, &db_path_c, iv,
                                strat, true, ml_confidence, &snap_clone, &tuning_c,
                            ).await;
                            // İz pozisyon açtıysa dur (sembol-başına-tek-pozisyon).
                            if symbol_has_open_position(&state_clone, &symbol) { break; }
                        }
                    }
                } else {
                    Self::process_symbol_cycle(
                        &state_clone, &symbol, &db_path_c, &interval_c,
                        &live_strategy_c, has_seed_strategy, ml_confidence, &snap_clone, &tuning_c,
                    ).await;
                }
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
        has_seed_strategy: bool,
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

            // === 1.6) 🪜 KADEMELİ GİRİŞ: pozisyon hâlâ açıksa (exit tetiklenmedi) rejime-göre ek kademe
            // dene (pyramiding/averaging, HTF-teyitli) ve TUR'U BİTİR — yeni-açılış mantığına geçme
            // (tek-pozisyon/sembol invariantı; açık pozisyonda zaten yeni açılış bloklanır). Kademeli
            // giriş kapalı / kademe dolu / eşik karşılanmadı → ek kademe açılmaz ama tur yine biter.
            // Açık pozisyon YOKSA düş → normal açılış yolu. (trailing/SL/breakeven 1.5'te güncellendi.)
            if symbol_has_open_position(state, symbol) {
                Self::try_add_graded_tranche(state, symbol, &candles, db_path).await;
                return;
            }

            // === 1.6) 🧊 STALE-FEED KAPISI: feed pratikte ölmüşse YENİ açılış yapma ===
            // BTCUSDC örneği: mum günlerce eski + live_price donuk ($87.840,60 sabit) →
            // donuk fiyat üzerinden phantom giriş/çıkış, sahte SL/TP ve komisyon erozyonu.
            // Açık pozisyon yönetimi (1.5) etkilenmez; yalnız yeni açılış kısa-devre.
            // Eşik RuntimeTuning'den (STALE_FEED_MAX_AGE_SECS); 0 → kapalı.
            // Eşik INTERVAL-FARKINDA: candle.timestamp bar AÇILIŞ zamanı → forming bar
            // yaşı sağlıklı durumda bile [0,interval). Sabit eşik (eski 3600) 1m'i fazla
            // gevşek (60 bar bayata izin), 4h'i fazla sıkı (3600<14400 → taze forming bar
            // 'stale' sanılır) bırakıyordu. effective_stale_feed_age: auto(<0)→2×interval
            // ("feed canlı = son bar < 2 bar eski"), >0 operatör sabit override, 0 kapalı.
            let interval_secs =
                crate::robot::data_pipeline::DataNormalizer::parse_interval(interval) as i64;
            let stale_bound = effective_stale_feed_age(tuning.stale_feed_max_age_secs, interval_secs);
            if stale_bound > 0 {
                if let Some(last) = candles.last() {
                    if !candle_is_fresh_within(&last.timestamp, stale_bound) {
                        let age = (chrono::Utc::now() - last.timestamp).num_seconds();
                        if log_throttle_should_emit(symbol, "stale_feed_skip", tuning.log_dataingest_cooldown_secs) {
                            push_state_log(state, format!(
                                "🧊 {} açılış atlandı: feed bayat (son mum {}sn eski > {}sn) — phantom giriş koruması",
                                symbol, age, stale_bound,
                            ));
                        }
                        return;
                    }
                }
            }

            // ─── 📐 GİRİŞ-KARARI PENCERESİ: kapalı-bar (forming dışlanmış) ───────────────
            // Live'da SQLite'ın son barı forming (REST kline forming barı da yazar) → strateji/edge/
            // rejim tamamlanmamış bar üzerinde = backtest'in kapalı-bar kararıyla skew + bar-içi repaint.
            // signal_candles forming barı dışlar → tüm GİRİŞ kararları (rejim/scalp-swing/strateji/sinyal/
            // edge) bunu kullanır → live=backtest. ÇIKIŞLAR (1.5 yukarıda) ve giriş FİYATI
            // (open_paper_position → fleet.live_price) tam mumu/anlık fiyatı kullanmaya devam eder.
            // Escape: SIGNAL_CLOSED_BAR_ONLY=0 → tam pencere (eski davranış). [[project_math_audit]]
            let signal_candles: &[Candle] = closed_bar_window(
                &candles, interval_secs, tuning.signal_closed_bar_only, chrono::Utc::now());

            // ─── Adım 1: rejim bağlamı (HTF-tercihli, cache'li) — ERKEN, tek-kaynak ───
            // Eskiden rejim regular yolda (aşağıda) hesaplanıyordu; scalp_swing açarsa
            // atlanıyordu → scalp/swing auto-gate base-1m classify_regime kullanıyordu
            // (1m hep "Ranging" → scalp-only). Artık HTF rejim ERKEN hesaplanıp hem
            // scalp/swing gate'ini hem regular yolu besler → denge geniş TF rejimle
            // otonom sürülür (TRADE_INTERVAL/threshold zorlamadan). ensure_regime_patch
            // de artık her cycle çalışır (scalp açsa bile) → TP/SL rejim patch'i tutarlı.
            let (multi_tf_enabled, multi_tf_min) = state.lock().ok()
                .and_then(|st| st.brain.parameters.read().ok()
                    .map(|p| (p.multi_tf.enabled, p.multi_tf.min_required)))
                .unwrap_or((true, crate::robot::data_pipeline::HTF_MIN_REQUIRED));
            // Faz 1: HTF yüklemesi de market-saf (config.market).
            let market = state.lock().ok().map(|st| st.config.market.clone()).unwrap_or_default();
            let htf_candles_vec = if multi_tf_enabled {
                crate::robot::data_pipeline::load_htf_candles(db_path, symbol, interval, &market, multi_tf_min)
            } else {
                Vec::new()
            };
            let htf_slice: Option<&[crate::core::types::Candle]> =
                if htf_candles_vec.is_empty() { None } else { Some(&htf_candles_vec) };
            let regime = Self::regime_for_cycle(
                state, symbol, signal_candles, interval, htf_slice,
                tuning.regime_context_ttl_secs, tuning.regime_gbt, tuning.regime_adaptive_pctl,
            );
            Self::ensure_regime_patch(state, regime.as_str());
            Self::observe_regime_drift(state, regime.as_str());

            // 🧭 Rejim-yön disiplini: ÖNCE per-rejim otonom policy (değerlendirme job'ı
            // backtest A/B ile regime_overrides'a yazar), YOKSA RuntimeTuning env'i. Tek
            // noktada çözülüp her iki açılış yoluna (ScalpSwing + regular) verilir.
            let regime_directional_eff = state.lock().ok()
                .and_then(|st| st.brain.parameters.read().ok()
                    .map(|p| p.regime_directional_for(regime.as_str(), tuning.regime_directional)))
                .unwrap_or(tuning.regime_directional);

            // ─── ScalpSwing A2: alt-kanal fırsat avı (auto-gate yukarıdaki HTF rejimle) ──
            // SCALP_SWING_ENABLE=1 ise Scalp/SwingEngine fırsat üretir; SlotGuard kanal-bazlı
            // limit + hedge kontrolü yapar. Uygun ise açılış ScalpSwing patikasından gider
            // (kind=Some(TradeType)); bu turda klasik strateji pas geçilir. Disabled → false.
            // 🌱 Seed önceliği: sembolün edge_scan'le keşfedilmiş açık strateji ataması varsa
            // (has_seed_strategy) fırsatçı ScalpSwing PAS geçilir → keşfedilmiş edge o sembolde
            // gerçekten koşar (aksi halde ScalpSwing önce açıp baypas ederdi). [[project_edge_scan]].
            if !(has_seed_strategy && tuning.seed_strategy_priority)
                && Self::try_open_scalp_swing(state, symbol, signal_candles, regime, regime_directional_eff).await {
                return;
            }

            // 3) Strateji seçimi: brain.live_strategy "Default"/"AUTO" ise otonom seç.
            // Rejim eşikleri (sabit ya da adaptif sembol-relatif) tek noktada üretilir;
            // hem eval-path Volatile savunması hem default-path select_best aynısını kullanır.
            let regime_thr = tuning.regime_thresholds(signal_candles);
            let strategy_name = if live_strategy.eq_ignore_ascii_case("default")
                                  || live_strategy.eq_ignore_ascii_case("auto")
                                  || live_strategy.is_empty() {
                if tuning.strategy_select_eval {
                    // Değerlendirme-tabanlı: her aday KENDİ resolve'lu paramıyla
                    // mini-backtest skoruna göre yarışır (param_spec optimizasyonu
                    // seçime de girer). Volatile rejimde IDLE savunması korunur.
                    use crate::robot::logic::market_regime::{detect_adx_regime_with, AdxRegime};
                    if matches!(detect_adx_regime_with(signal_candles, &regime_thr), AdxRegime::Volatile) {
                        crate::robot::ml_engine::strategy_selector::IDLE_PROTECT.to_string()
                    } else {
                        let ps = state.lock().ok().map(|st| std::sync::Arc::clone(&st.brain.parameters));
                        let sel = crate::robot::strategies::strategy_selector::StrategySelector::from_registry(
                            &crate::robot::strategies::default_registry(),
                            &["SUPERTREND", "MA_CROSSOVER", "EMA_CROSSOVER", "RSI", "MACD", "BB", "DONCHIAN"],
                        );
                        let (best_name, _sig) = sel.select_best_name_resolved(signal_candles, |name| {
                            ps.as_ref()
                                .and_then(|p| p.read().ok().map(|g| g.resolve_strategy_params(name)))
                                .unwrap_or_default()
                        });
                        best_name
                    }
                } else {
                    // Default: rejim→strateji lookup (param-free, hızlı, kanıtlı savunma).
                    let sel = crate::robot::ml_engine::strategy_selector::StrategySelector::new();
                    sel.select_best_with(signal_candles, &crate::core::types::StrategyParams::default(), &regime_thr).to_string()
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

            // (HTF mumları + rejim yukarıda ERKEN yüklendi — htf_slice/regime burada hazır.)

            // ─── Faz 3 (StrategyEval): sinyal üretimi ─────────────────────
            let signal = match strategy.generate_signal(signal_candles, &strat_params, None, htf_slice) {
                Ok(s) => {
                    Self::mark_pipeline_stage(state, PipelineStage::StrategyEval, StepStatus::Done);
                    s
                }
                Err(e) => {
                    Self::mark_pipeline_stage(state, PipelineStage::StrategyEval, StepStatus::Failed);
                    emit_trade_event(state, || crate::robot::infra::logger::TradeEvent::error(
                        &format!("{} sinyal üretim hatası: {:?}", symbol, e),
                    ));
                    return;
                }
            };

            // ─── Faz 2 (FeatureExtract): edge skoru (HIZLI MATEMATİK MATRİSİ) ───────
            // Hedef mimari: edge/sizing/trigger saf matematik; AI (GBT/ONNX) YALNIZ
            // Adım 1 (regime) yolunda, geniş TF'de SEYREK çalışır. Bu yüzden GBT artık
            // burada (per-tick) ÇAĞRILMAZ — yön kanaati regime'e taşındı (regime_for_cycle).
            // ml_conf: retrain'in yavaş sharpe-bazlı brain.ml_confidence değeri (per-tick
            // model inference değil). compute_edge_score momentum'u ATR ile normalize eder;
            // ml_conf eşik kademesine (cold/warm/hot) girer.
            //
            // GBT_EDGE_LEGACY=1 → eski per-tick predict_confidence yolu (geri-dönüş).
            let ml_conf_used: f64 = if tuning.gbt_edge_legacy {
                let gbt_conf = if signal_candles.len() >= 30 {
                    let tail = &signal_candles[signal_candles.len().saturating_sub(200)..];
                    let fv = crate::robot::ml_engine::FeatureExtractor::extract(tail);
                    state.lock().ok().and_then(|st| {
                        st.brain.intelligence_hub.read().ok()
                            .and_then(|hub| hub.predict_confidence(&fv, &signal))
                    })
                } else { None };
                if let Some(c) = gbt_conf {
                    if let Ok(mut st) = state.lock() { st.brain.ml_confidence = c; }
                }
                gbt_conf.unwrap_or(ml_confidence)
            } else {
                ml_confidence
            };

            let edge = Self::compute_edge_score(signal_candles, &signal, ml_conf_used);
            Self::mark_pipeline_stage(state, PipelineStage::FeatureExtract, StepStatus::Done);
            // ML henüz hazır değilse (cold-start) gevşek eşik; modele güven arttıkça katılaşır.
            // Faz 2 c4: edge_threshold rejim-bazlı override'a açık.
            // Faz 3 c1: rejim ilk kez görülüyorsa adaptive heuristic patch otomatik.
            // Faz 3 c3: rejim drift değişimi → ekstra savunmacı tighten + bildirim.
            // (Rejim + ensure_regime_patch + observe_regime_drift yukarıda ERKEN yapıldı.)
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

            // 🔂 Bar-başına tek-işlem (churn kökü): kapalı-bar sinyali bar boyu SABİT kalır
            // (signal_closed_bar_only). Motor her tarama (~1/dk) aynı kapalı-barın Buy/Sell'ini
            // yeniden yayıp yeniden işliyordu → tek bar içinde aç→stop→aynı-bar-Buy→tekrar-aç
            // (60sn re-entry cooldown yalnız bant-yardımı, barın sinyali değişmediği için kapatamaz)
            // + her tarama SIGNAL log spam'i. Bu sembolde bu kapalı-barı zaten işlediysek (cur==last)
            // yeni bar kapanana kadar pas geç → açılış/kapanış/blok hepsi bar başına BİR kez. Çıkışlar
            // (SL/TP/trailing) ayrı denetimden (cycle_try_close_open_position) gittiği için ETKİLENMEZ.
            // xs_last_rebalance_bar'ın regular-yol per-sembol ikizi; tek state-lock altında atomik
            // check-and-set (TOCTOU yok). Escape: SIGNAL_CLOSED_BAR_ONLY=0 → kapalı (forming-bar kimliği
            // akışkan = eski davranış). [[project_closed_bar_signal]]
            if tuning.signal_closed_bar_only && matches!(signal, Signal::Buy | Signal::Sell) {
                if let Some(bar_ts) = signal_candles.last().map(|c| c.timestamp) {
                    let fresh_bar = state.lock().ok()
                        .map(|st| st.finance.claim_signal_bar(symbol, bar_ts))
                        .unwrap_or(true); // state lock zehirliyse fail-open
                    if !fresh_bar { return; }
                }
            }

            // SIGNAL eventi: yalnız Buy/Sell için logla (HOLD spam yapmasın).
            if matches!(signal, Signal::Buy | Signal::Sell) {
                // Referans fiyat: fleet.live_price (5sn REST) > candle close — exit
                // denetimiyle (cycle_try_close_open_position) birebir aynı öncelik.
                let signal_price = state.lock().ok()
                    .and_then(|st| st.fleet.live_price.read().ok()
                        .and_then(|m| m.get(symbol).copied()))
                    .filter(|&v| v > 0.0)
                    .unwrap_or_else(|| candles.last().map(|c| c.close).unwrap_or(0.0));
                emit_trade_event(state, || crate::robot::infra::logger::TradeEvent::signal(symbol, signal, signal_price));
            }

            match (signal, pos_dir) {
                // Pozisyon yokken: yalnız yüksek edge'de açılış denenir.
                (crate::core::types::Signal::Buy, None) | (crate::core::types::Signal::Sell, None) => {
                    // 🧭 Rejim-yön kapısı (opt-in): canlı motor Sell→short açıyor (Both modu);
                    // bu kapı ters-trend girişini eler (A/B: Both -661 → RegimeDirectional +980).
                    // Tek-kaynak `regime_confirms_direction`; default kapalı → davranış değişmez.
                    if regime_directional_eff
                        && !crate::robot::logic::market_regime::regime_confirms_direction(
                            regime, matches!(signal, crate::core::types::Signal::Buy))
                    {
                        if log_throttle_should_emit(symbol, "regime_dir_block", 60) {
                            push_state_log(state, format!(
                                "🧭 {} {} ⇒ REDDEDİLDİ (rejim-yön teyidi yok, rejim={})",
                                symbol, signal_label, regime.as_str(),
                            ));
                        }
                        return;
                    }
                    if edge < edge_threshold {
                        // Spam'i kısmak için: yalnız eşiğe yakın aday (edge≥floor) VE per-sembol
                        // 60sn throttle. Aksi halde choppy rejimde aynı sembol (örn. seed TRX
                        // edge≈0.40, eşik 0.45) her cycle (500ms) tekrar basıyordu → rejim-yön
                        // ve risk bloklarıyla aynı throttle disiplini.
                        if edge >= edge_log_floor
                            && log_throttle_should_emit(symbol, "edge_weak_block", 60)
                        {
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
                        emit_trade_event(state, || crate::robot::infra::logger::TradeEvent::risk_block(
                            &block_reason, symbol,
                        ));
                        return;
                    }
                    Self::mark_pipeline_stage(state, PipelineStage::RiskGate, StepStatus::Done);
                    // Sembol başına throttle: açılış re-entry cooldown / stale-feed ile
                    // bloklanınca bu "AÇILIYOR" satırı her cycle basılıp olay günlüğünü
                    // taşırıyordu. Gerçek açılış open_paper_position'da "🚀 açıldı" ile onaylanır.
                    if log_throttle_should_emit(symbol, "open_attempt", 60) {
                        push_state_log(state, format!(
                            "📊 {} {} edge={:.2} ✓ + risk ✓ ⇒ POZİSYON AÇILIYOR (strat={})",
                            symbol, signal_label, edge, strategy_name,
                        ));
                    }
                    Self::open_paper_position(state, symbol, &signal, &candles, &strategy_name, None, None).await;
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
                        let dir_label = if matches!(signal, Signal::Buy) { "LONG" } else { "SHORT" };
                        emit_trade_event(state, || crate::robot::infra::logger::TradeEvent::risk_block(
                            &format!("[position-aligned] {} sinyali, pozisyon zaten {}", signal_label, dir_label),
                            symbol,
                        ));
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
        // Faz 1: market-saf okuma (spot/futures karışmasını önler). config.market boşsa
        // market-kör okumaya düş (geriye-uyum). state zaten elde → imza değişmeden.
        let market = state.lock().ok().map(|st| st.config.market.clone()).filter(|m| !m.is_empty());
        let read = match &market {
            Some(m) => crate::persistence::reader::read_candles_market(db_path, symbol, interval, m, 200),
            None => crate::persistence::reader::read_candles(db_path, symbol, interval, 200),
        };
        // Üç ayrım: Ok(non-empty) Done. Ok(empty) sessiz Failed (sembol için 1m
        // candle DB'de yok = veri kaynağı eksikliği, alarm değil). Err = gerçek DB hatası.
        match read {
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
            // Rejim-farkında trail: open path (open_paper_position) ile AYNI kaynak
            // (classify_regime(candles)) → per-rejim A/B hedefi open/exit'te tutarlı.
            let regime_str = Self::classify_regime(candles).as_str().to_string();
            let atr_mult = st.brain.parameters.read().ok()
                .map(|p| p.resolve_atr_mult_for_regime(
                    symbol, interval, &pos_strategy, default_mult, Some(&regime_str)))
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
}

/// Stale-feed kapısı eşiği — interval-farkında (yeni açılış koruması).
///
/// `candle.timestamp` bar AÇILIŞ zamanı olduğundan forming bar yaşı `[0, interval)`
/// arasında; sabit eşik kısa interval'i fazla gevşek, uzun interval'i fazla sıkı
/// bırakır. `configured`: `<0` → auto = `2×interval` (feed canlı = son bar < 2 bar
/// eski; sub-1m için 60s taban), `0` → kapalı, `>0` → operatör sabit override.
pub(crate) fn effective_stale_feed_age(configured: i64, interval_secs: i64) -> i64 {
    match configured {
        0 => 0,
        n if n < 0 => (interval_secs * 2).max(60),
        n => n,
    }
}

/// SAF: son bar hâlâ forming (tamamlanmamış) mı? `candle.timestamp` bar AÇILIŞ zamanı →
/// kapanış = `timestamp + interval`. Kapanış `now`'dan ilerideyse bar henüz oluşuyor (forming).
/// `interval_secs <= 0` (bilinmiyor) → false (güvenli: forming sayma → dışlama). Testli.
pub(crate) fn last_bar_is_forming(
    candles: &[Candle], interval_secs: i64, now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if interval_secs <= 0 { return false; }
    match candles.last() {
        Some(c) => c.timestamp + chrono::Duration::seconds(interval_secs) > now,
        None => false,
    }
}

/// SAF: GİRİŞ-KARARI penceresi (strateji sinyali + rejim + edge). `enabled` ve son bar forming ise
/// son barı dışla → live, backtest'in kapalı-bar karar semantiğiyle hizalanır (repaint/skew biter).
/// Dışlayınca ≥1 bar kalmazsa ya da bar kapalıysa pencere olduğu gibi döner (aşırı-düşme yok).
/// ÇIKIŞLAR bu yolu KULLANMAZ — SL/TP fleet.live_price ile anlık kalır. Testli.
pub(crate) fn closed_bar_window<'a>(
    candles: &'a [Candle], interval_secs: i64, enabled: bool,
    now: chrono::DateTime<chrono::Utc>,
) -> &'a [Candle] {
    if enabled && candles.len() >= 2 && last_bar_is_forming(candles, interval_secs, now) {
        &candles[..candles.len() - 1]
    } else {
        candles
    }
}

/// Sembolün AÇIK pozisyonu var mı (live_positions symbol-keyed). Çoklu-iz dispatch'i bununla
/// tek-pozisyon invariantını korur: flat'ken izleri sırayla dener, biri açınca durur. Lock
/// edinilemezse "var" sayar (güvenli taraf → fazladan açılış denenmez).
fn symbol_has_open_position(state: &Arc<Mutex<AppState>>, symbol: &str) -> bool {
    state.lock().ok()
        .and_then(|st| st.finance.live_positions.read().ok().map(|p| p.contains_key(symbol)))
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::effective_stale_feed_age;

    #[test]
    fn stale_feed_age_is_interval_aware() {
        // auto (-1): 2×interval, sub-1m 60s taban
        assert_eq!(effective_stale_feed_age(-1, 60), 120, "1m: 2 bar (eski 3600 → 60 bar gevşekti)");
        assert_eq!(effective_stale_feed_age(-1, 900), 1800, "15m: 2 bar");
        assert_eq!(effective_stale_feed_age(-1, 3600), 7200, "1h: 2 bar");
        assert_eq!(effective_stale_feed_age(-1, 14400), 28800, "4h: 2 bar (eski 3600 forming barı bloklardı)");
        assert_eq!(effective_stale_feed_age(-1, 10), 60, "sub-1m: 60s taban");
        // 0 → kapalı
        assert_eq!(effective_stale_feed_age(0, 3600), 0, "0 → gate kapalı");
        // >0 → operatör sabit override (interval'den bağımsız)
        assert_eq!(effective_stale_feed_age(300, 3600), 300, "operatör 300s zorlar");
    }

    use super::{last_bar_is_forming, closed_bar_window};
    use crate::core::types::Candle;

    fn c_at(ts: chrono::DateTime<chrono::Utc>, close: f64) -> Candle {
        Candle { timestamp: ts, open: close, high: close, low: close, close, volume: 1.0, ..Default::default() }
    }

    #[test]
    fn forming_bar_detection() {
        let now = chrono::Utc::now();
        let iv = 3600; // 1h
        // Son bar 10dk önce açıldı → kapanış 50dk ileride → forming.
        let forming = vec![c_at(now - chrono::Duration::minutes(10), 100.0)];
        assert!(last_bar_is_forming(&forming, iv, now), "açılış+interval > now → forming");
        // Son bar 2 saat önce açıldı → kapanış 1 saat önce → kapalı.
        let closed = vec![c_at(now - chrono::Duration::hours(2), 100.0)];
        assert!(!last_bar_is_forming(&closed, iv, now), "açılış+interval ≤ now → kapalı");
        // interval bilinmiyor → forming sayma (güvenli).
        assert!(!last_bar_is_forming(&forming, 0, now), "interval≤0 → false");
        assert!(!last_bar_is_forming(&[], iv, now), "boş → false");
    }

    #[test]
    fn closed_bar_window_drops_only_forming() {
        let now = chrono::Utc::now();
        let iv = 3600;
        // [kapalı, kapalı, forming] → enabled ise son (forming) düşer.
        let v = vec![
            c_at(now - chrono::Duration::hours(3), 1.0),
            c_at(now - chrono::Duration::hours(2), 2.0),
            c_at(now - chrono::Duration::minutes(5), 3.0), // forming
        ];
        let w = closed_bar_window(&v, iv, true, now);
        assert_eq!(w.len(), 2, "forming bar dışlandı");
        assert_eq!(w.last().unwrap().close, 2.0, "son = kapalı bar");
        // enabled=false → escape: tam pencere.
        assert_eq!(closed_bar_window(&v, iv, false, now).len(), 3, "escape → tam");
        // Son bar kapalıysa → tam pencere (aşırı-düşme yok).
        let closed = vec![c_at(now - chrono::Duration::hours(3), 1.0), c_at(now - chrono::Duration::hours(2), 2.0)];
        assert_eq!(closed_bar_window(&closed, iv, true, now).len(), 2, "kapalı son bar düşmez");
        // Tek forming bar (len<2) → düşme (boş pencere üretme).
        let one = vec![c_at(now - chrono::Duration::minutes(5), 3.0)];
        assert_eq!(closed_bar_window(&one, iv, true, now).len(), 1, "tek bar korunur");
    }
}
