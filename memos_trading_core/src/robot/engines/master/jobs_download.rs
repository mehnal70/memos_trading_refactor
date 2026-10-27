// src/robot/engines/master/jobs_download.rs — Download job: mum indirme + HTF + symbol-stats.
// Faz 2 modülerleştirme: jobs.rs'ten ayrıldı (davranış birebir korunur).
use super::*;
use crate::robot::venue::adapter::{ExchangeCapability, VenueAdapter};
use crate::robot::venue::registry::{split_venue_tag, VenueRegistry};
use std::sync::atomic::Ordering;
use std::time::Duration;

fn download_symbol_has_registered_exchange(symbol: &str, registry: &VenueRegistry) -> bool {
    let (bare, explicit_exchange) = split_venue_tag(symbol);
    let exchange =
        explicit_exchange.unwrap_or_else(|| crate::core::types::Exchange::classify(bare));
    registry
        .exchanges()
        .any(|registered| registered == exchange)
}

fn download_symbol_is_eligible(symbol: &str, tuning: &RuntimeTuning) -> bool {
    if tuning.symbol_eligible_for_live(symbol) {
        return true;
    }
    let (_bare, explicit_exchange) = split_venue_tag(symbol);
    explicit_exchange == Some(crate::core::types::Exchange::Bist)
        && !super::is_delisted_skipped(symbol)
        && super::is_symbol_tradeable(symbol)
}

fn resolve_download_context(
    venue: &Arc<dyn VenueAdapter>,
    _fallback_market: &str,
) -> (String, String, String) {
    // Kanonik namespace TEK KAYNAKTAN: venue.storage_namespace().
    // ÖNCEDEN: MT5 için "mt5", diğerleri için global config.market kullanılıyordu; okuma
    // yolu ise "cfd" bekliyordu → aynı enstrüman 5 kovaya dağılıyordu. Artık yazma ve
    // okuma aynı fonksiyondan besleniyor.
    let (ex, mk) = venue.storage_namespace();
    let exchange = ex.to_string();
    let market = mk.to_string();
    // Kaynak etiketi venue adından türetilir ("mt5-live", "ibkr-live") — teşhiste
    // hangi venue'nun indirdiği görünür; eski hal Binance-dışını "venue-live"e eziyordu.
    let source = format!("{}-live", venue.exchange().as_str());
    (exchange, market, source)
}

struct DownloadActiveGuard {
    state: Arc<Mutex<AppState>>,
}

impl DownloadActiveGuard {
    fn arm(state: &Arc<Mutex<AppState>>) -> Self {
        if let Ok(mut st) = state.lock() {
            st.fleet.download_active = true;
            if let Ok(mut progress) = st.fleet.download_progress.write() {
                progress.active = true;
                progress.completed_symbols = 0;
                progress.current_symbol.clear();
                progress.current_interval.clear();
                progress.current_phase.clear();
                progress.last_completed_symbol.clear();
                progress.fetched_candles = 0;
                progress.failed_symbols = 0;
                progress.failed_symbol_names.clear();
                progress.last_error.clear();
                progress.started_at = Some(chrono::Utc::now().to_rfc3339());
            }
        }
        return Self {
            state: Arc::clone(state),
        };
    }
}

impl Drop for DownloadActiveGuard {
    fn drop(&mut self) {
        if let Ok(mut st) = self.state.lock() {
            st.fleet.download_active = false;
            if let Ok(mut progress) = st.fleet.download_progress.write() {
                progress.active = false;
                progress.current_symbol.clear();
                progress.current_interval.clear();
            }
        }
    }
}

impl Engine {
    pub(crate) async fn run_download_job(
        state: &Arc<Mutex<AppState>>,
    ) -> std::result::Result<(), String> {
        let retry_symbols = state
            .lock()
            .ok()
            .and_then(|st| {
                st.fleet
                    .download_progress
                    .read()
                    .ok()
                    .map(|progress| progress.failed_symbol_names.clone())
            })
            .unwrap_or_default();
        let _download_active_guard = DownloadActiveGuard::arm(state);

        let (
            symbols,
            interval,
            symbol_interval,
            db_path,
            limit,
            _exchange,
            market,
            profile,
            backfill_enabled,
            backfill_max_requests,
            venue_registry,
        ) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            let tuning = Arc::clone(&st.tuning);
            let venue_registry = Arc::clone(&st.venue_registry);
            let eligible = |s: &str| download_symbol_is_eligible(s, &tuning);

            // KESİN ÇÖZÜM: Statik "EURUSDT" enjeksiyonu tamamen söküldü. Vektör boş başlar
            // ve sadece alt taraftaki meşru canlı hatlardan gelen otonom verilerle hidrate olur.
            let mut syms: Vec<String> = Vec::new();

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

            for s in &st.config.pinned_symbols {
                if !eligible(s) {
                    continue;
                }
                if !syms.contains(s) {
                    syms.push(s.clone());
                }
            }

            for key in [
                "AUTO_BINANCE_SYMBOLS",
                "AUTO_MT5_SYMBOLS",
                "AUTO_IBKR_SYMBOLS",
                "AUTO_BIST_SYMBOLS",
            ] {
                if let Ok(raw) = std::env::var(key) {
                    for sym in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                        if !eligible(sym) {
                            continue;
                        }
                        if !syms.iter().any(|current| current == sym) {
                            syms.push(sym.to_string());
                        }
                    }
                }
            }

            syms.retain(|s| !s.is_empty());
            syms.retain(|symbol| {
                venue_registry.route(symbol).is_some()
                    && download_symbol_has_registered_exchange(symbol, &venue_registry)
            });
            if !retry_symbols.is_empty() {
                syms.retain(|symbol| retry_symbols.iter().any(|failed| failed == symbol));
            }
            let symbol_interval = st
                .brain
                .parameters
                .read()
                .ok()
                .map(|p| p.symbol_interval.clone())
                .unwrap_or_default();

            // 🎯 GÜVENLİK KAPISI standard dönüş sarmalı:
            (
                syms,
                st.config.interval.clone(),
                symbol_interval,
                st.config.db_path.clone(),
                st.config.download_candle_limit.max(50),
                st.config.exchange.clone(),
                st.config.market.clone(),
                st.config.venue_profile.as_str().to_string(),
                tuning.backfill_enabled,
                tuning.backfill_max_requests,
                venue_registry,
            )
        };

        if symbols.is_empty() {
            return Err(
                "indirilecek sembol yok (config.symbol + pinned + orchestrator boş)".into(),
            );
        }
        if let Ok(st) = state.lock() {
            if let Ok(mut progress) = st.fleet.download_progress.write() {
                progress.total_symbols = symbols.len();
            }
        }

        log::info!(
            "🌐 Download başladı: {} sembol × {} mum (interval={}) → {}",
            symbols.len(),
            limit,
            interval,
            symbols.join(","),
        );
        push_state_log(
            state,
            format!(
                "🌐 Download başladı: {} sembol × {} mum (interval={})",
                symbols.len(),
                limit,
                interval,
            ),
        );
        emit_canonical_event(
            state,
            "pipeline",
            "DOWNLOAD_STARTED",
            "internal",
            "download",
            serde_json::json!({"symbols": symbols.len(), "limit": limit, "interval": interval}),
        );
        let mut total_fetched = 0usize;
        let mut total_failed = 0usize;
        let mut failed_symbol_names = Vec::new();
        let mut per_symbol_summary: Vec<String> = Vec::new();
        let download_started = std::time::Instant::now();
        macro_rules! mark_failed {
            ($symbol:expr) => {{
                total_failed += 1;
                if !failed_symbol_names.iter().any(|failed| failed == $symbol) {
                    failed_symbol_names.push($symbol.clone());
                }
            }};
        }

        let resolve_symbol_interval = |sym: &str| -> String {
            if let Some(iv) = symbol_interval.get(sym) {
                return iv.clone();
            }
            return interval.clone();
        };

        for sym in &symbols {
            if let Ok(st) = state.lock() {
                if let Ok(mut progress) = st.fleet.download_progress.write() {
                    progress.current_symbol = sym.clone();
                    progress.current_interval = resolve_symbol_interval(sym);
                    progress.current_phase = "primary".into();
                }
            }
            //push_state_log(state, format!("SEMBOL ---> : {} ", sym));
            // ─── 🔌 GERÇEK PAZAR-AGNOSTİK BÖLGESEL ROUTING ÇEKİRDEĞİ ───────────────────────
            // Küresel pazar ayarlarını eziyoruz. Sembolün ait olduğu gerçek borsa ve pazar kimliğini
            // registry rotasından dinamik çözüyoruz. Böylece MT5 her modda bağımsız veri indirebilir.
            let Some((venue, bare)) = venue_registry.route(sym) else {
                mark_failed!(sym);
                push_state_log(
                    state,
                    format!("    └─ {} ❌ Registry rotası bulunamadı", sym),
                );
                continue;
            };

            let current_exchange = venue.exchange();
            let is_binance_venue = current_exchange == crate::core::types::Exchange::Binance;

            // Dinamik yerel pazar süzgeçleri (Veritabanı kirliliğini ve -1121 hatalarını önler)
            let (local_exchange, local_market, source_label) =
                resolve_download_context(&venue, &market);

            // 🛡️ SIZINTI KORUMASI: Eğer borsa Binance ise ve Forex paritesi sızdıysa istek atmadan atla!
            if is_binance_venue
                && (sym.contains("@mt5")
                    || sym.contains("EURUSD")
                    || sym.contains("GBPUSD")
                    || sym.contains("XAUUSD"))
            {
                continue;
            }
            // ─────────────────────────────────────────────────────────────────────────────────

            /*             let sym_iv = resolve_symbol_interval(sym);

            let iv_secs = crate::robot::data_pipeline::DataNormalizer::parse_interval(&sym_iv).max(1) as i64;
            let last_ms = crate::persistence::reader::last_candle_ts(&db_path, sym, &sym_iv, &market);
            let now_ms = crate::core::time::now_epoch_millis() as i64;

            let Some((venue, bare)) = venue_registry.route(sym) else {
                total_failed += 1;
                push_state_log(state, format!("    └─ {} ❌ Registry rotası bulunamadı", sym));
                continue;
            };

            let source_label = format!("{}-live", venue.exchange().as_str());

            let fetch_limit = gap_aware_fetch_limit(last_ms, now_ms, iv_secs, limit);

            let fetch_fut = if is_binance_venue && backfill_enabled && deep_gap_start_ms(last_ms, now_ms, iv_secs).is_some() {
                let start_ms = deep_gap_start_ms(last_ms, now_ms, iv_secs).unwrap_or(0);
                push_state_log(state, format!(
                    "    └─ {} [{}] 🕳️ derin gap → backfill (start={}, ≤{}×1000 bar)",
                    sym, source_label, start_ms, backfill_max_requests,
                ));
                venue.download_candles(bare, &sym_iv, Some(start_ms), backfill_max_requests as usize).await.map_err(|e| e.to_string())
            } else {
                venue.fetch_candles(bare, &sym_iv, fetch_limit).await.map_err(|e| e.to_string())
            }; */

            let sym_iv = resolve_symbol_interval(sym);
            let iv_secs =
                crate::robot::data_pipeline::DataNormalizer::parse_interval(&sym_iv).max(1) as i64;

            // KESİN ÇÖZÜM: `market` yerine dinamik çözülen `local_market` kullanılıyor!
            let last_ms = crate::persistence::reader::last_candle_ts_for_profile(
                &db_path,
                sym,
                &sym_iv,
                &local_market,
                &profile,
            );
            let now_ms = crate::core::time::now_epoch_millis() as i64;

            let fetch_limit = gap_aware_fetch_limit(last_ms, now_ms, iv_secs, limit);

            // Venue özel-durumu YOK: `download_candles` trait default'u backfill
            // desteklemeyen venue'da fetch_candles'a döner (adapter.rs) — dallanma
            // trait'te zaten çözülmüş, burada Binance kontrolü polimorfizmi elle
            // tekrarlıyordu (yeni backfill'li venue eklenince burası unutulurdu).
            let fetch_timeout_secs = std::env::var("DATA_FETCH_TIMEOUT_SECS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(90)
                .max(1);
            let fetch_fut = match tokio::time::timeout(
                std::time::Duration::from_secs(fetch_timeout_secs),
                async {
                    if backfill_enabled && deep_gap_start_ms(last_ms, now_ms, iv_secs).is_some() {
                        let start_ms = deep_gap_start_ms(last_ms, now_ms, iv_secs).unwrap_or(0);
                        push_state_log(
                            state,
                            format!(
                                "    └─ {} [{}] 🕳️ derin gap → backfill (start={}, ≤{}×1000 bar)",
                                sym, source_label, start_ms, backfill_max_requests,
                            ),
                        );
                        venue
                            .download_candles(
                                bare,
                                &sym_iv,
                                Some(start_ms),
                                backfill_max_requests as usize,
                            )
                            .await
                            .map_err(|e| e.to_string())
                    } else {
                        venue
                            .fetch_candles(bare, &sym_iv, fetch_limit)
                            .await
                            .map_err(|e| e.to_string())
                    }
                },
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err(format!(
                    "veri çekimi {} saniyelik timeout'u aştı",
                    fetch_timeout_secs
                )),
            };

            match fetch_fut {
                Ok(candles) => {
                    delisted_record_success(sym);
                    let n = candles.len();
                    if n > 0 {
                        if let Ok(st) = state.lock() {
                            st.fleet.opportunity_cache.push_bulk(candles.clone());
                        }
                    }
                    total_fetched += n;
                    if let Ok(st) = state.lock() {
                        if let Ok(mut progress) = st.fleet.download_progress.write() {
                            progress.completed_symbols += 1;
                            progress.last_completed_symbol = sym.clone();
                            progress.fetched_candles += n;
                        }
                    }

                    let db_path_clone = db_path.clone();
                    let candles_clone = candles.clone();
                    let exchange_c = local_exchange.clone();
                    let market_c = local_market.clone();
                    let profile_c = profile.clone();

                    let write_result = tokio::task::spawn_blocking(
                        move || -> std::result::Result<(usize, Option<String>), String> {
                            let conn = crate::persistence::open_db(&db_path_clone)
                                .map_err(|e| format!("db open: {}", e))?;
                            let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
                            let mut written = 0usize;
                            let mut first_err: Option<String> = None;
                            for c in &candles_clone {
                                match crate::persistence::writer::save_candle_for_profile(
                                    &conn,
                                    &profile_c,
                                    &exchange_c,
                                    &market_c,
                                    c,
                                ) {
                                    Ok(()) => written += 1,
                                    Err(e) => {
                                        if first_err.is_none() {
                                            first_err = Some(e.to_string());
                                        }
                                    }
                                }
                            }
                            return Ok((written, first_err));
                        },
                    )
                    .await;
                    match write_result {
                        Ok(Ok((0, _))) if n == 0 => {
                            mark_failed!(sym);
                            push_state_log(
                                state,
                                format!(
                                    "    └─ {} [{}] ⚠️ veri sağlayıcı boş candle döndürdü",
                                    sym, source_label,
                                ),
                            );
                        }
                        Ok(Ok((0, err))) if n > 0 => {
                            mark_failed!(sym);
                            push_state_log(
                                state,
                                format!(
                                    "    └─ {} [{}] ❌ fetch {} mum ama DB yazımı 0 (hata: {})",
                                    sym,
                                    source_label,
                                    n,
                                    err.as_deref().unwrap_or("?"),
                                ),
                            );
                        }
                        Ok(Ok((written, err))) => {
                            let warn = match &err {
                                Some(e) => {
                                    format!(" ⚠️ {} atlandı: {}", n.saturating_sub(written), e)
                                }
                                None => String::new(),
                            };
                            per_symbol_summary.push(format!("{}={}", sym, written));

                            if let Some(stats) =
                                crate::robot::parameters::compute_symbol_stats(&candles)
                            {
                                if let Ok(st) = state.lock() {
                                    if let Ok(mut params) = st.brain.parameters.write() {
                                        params.update_symbol_stats(sym, &sym_iv, stats);
                                    }
                                }
                            }
                            push_state_log(
                                state,
                                format!(
                                    "    └─ {} [{}] ✅ canlı çekim {} mum yazıldı{}",
                                    sym, source_label, written, warn,
                                ),
                            );
                            emit_canonical_event(
                                state,
                                "market",
                                "DOWNLOAD_SYMBOL_COMPLETED",
                                &format!("{}:{}", local_exchange, local_market),
                                sym,
                                serde_json::json!({"written": written, "source": source_label, "interval": sym_iv}),
                            );

                            let (multi_tf_enabled, download_htf) = state
                                .lock()
                                .ok()
                                .and_then(|st| {
                                    st.brain
                                        .parameters
                                        .read()
                                        .ok()
                                        .map(|p| (p.multi_tf.enabled, p.multi_tf.download_htf))
                                })
                                .unwrap_or((true, true));
                            let htf_interval =
                                crate::robot::data_pipeline::DataPipeline::get_htf_interval(
                                    &sym_iv,
                                );

                            if download_htf && htf_interval != sym_iv {
                                let htf_limit = (limit / 4).max(50);
                                match venue.fetch_candles(bare, htf_interval, htf_limit).await {
                                    Ok(htf_candles) if !htf_candles.is_empty() => {
                                        let htf_n = htf_candles.len();
                                        if let Ok(st) = state.lock() {
                                            st.fleet
                                                .opportunity_cache
                                                .push_bulk(htf_candles.clone());
                                        }
                                        let db2 = db_path.clone();
                                        let htf_clone = htf_candles.clone();
                                        let (ex2, mk2) =
                                            (local_exchange.clone(), local_market.clone());
                                        let profile_h = profile.clone();
                                        let _ = tokio::task::spawn_blocking(move || {
                                            if let Ok(conn) = crate::persistence::open_db(&db2) {
                                                let _ = conn.busy_timeout(
                                                    std::time::Duration::from_secs(5),
                                                );
                                                for c in &htf_clone {
                                                    let _ = crate::persistence::writer::save_candle_for_profile(
                                                        &conn,
                                                        &profile_h, &ex2, &mk2, c,
                                                    );
                                                }
                                            }
                                        })
                                        .await;
                                        push_state_log(
                                            state,
                                            format!(
                                                "        └─ {} HTF {} ✓ {} mum",
                                                sym, htf_interval, htf_n
                                            ),
                                        );
                                    }
                                    _ => {}
                                }
                            }

                            let trigger_interval = crate::robot::config_engine::OtonomConfigEngine::load_central_config()
                                .decision_policy
                                .trigger_interval;
                            if multi_tf_enabled
                                && trigger_interval != sym_iv
                                && trigger_interval != htf_interval
                            {
                                let trigger_limit = (limit / 3).max(200);
                                match venue.fetch_candles(bare, &trigger_interval, trigger_limit).await {
                                    Ok(trigger_candles) if !trigger_candles.is_empty() => {
                                        let trigger_n = trigger_candles.len();
                                        if let Ok(st) = state.lock() {
                                            st.fleet
                                                .opportunity_cache
                                                .push_bulk(trigger_candles.clone());
                                        }
                                        let db2 = db_path.clone();
                                        let trigger_clone = trigger_candles.clone();
                                        let (ex2, mk2) =
                                            (local_exchange.clone(), local_market.clone());
                                        let profile_t = profile.clone();
                                        let _ = tokio::task::spawn_blocking(move || {
                                            if let Ok(conn) = crate::persistence::open_db(&db2) {
                                                let _ = conn.busy_timeout(
                                                    std::time::Duration::from_secs(5),
                                                );
                                                for c in &trigger_clone {
                                                    let _ = crate::persistence::writer::save_candle_for_profile(
                                                        &conn,
                                                        &profile_t, &ex2, &mk2, c,
                                                    );
                                                }
                                            }
                                        })
                                        .await;
                                        push_state_log(
                                            state,
                                            format!(
                                                "        └─ {} trigger {} ✓ {} mum",
                                                sym, trigger_interval, trigger_n
                                            ),
                                        );
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            mark_failed!(sym);
                            if let Ok(st) = state.lock() {
                                if let Ok(mut progress) = st.fleet.download_progress.write() {
                                    progress.last_error =
                                        format!("{} [{}] yazma hatası: {}", sym, source_label, e);
                                }
                            }
                            push_state_log(
                                state,
                                format!("    └─ {} [{}] ❌ yazma hatası: {}", sym, source_label, e),
                            );
                        }
                        Err(e) => {
                            mark_failed!(sym);
                            if let Ok(st) = state.lock() {
                                if let Ok(mut progress) = st.fleet.download_progress.write() {
                                    progress.completed_symbols += 1;
                                    progress.failed_symbols += 1;
                                    progress.last_error = e.to_string();
                                }
                            }
                            push_state_log(
                                state,
                                format!(
                                    "    └─ {} [{}] ❌ blocking join hatası: {}",
                                    sym, source_label, e
                                ),
                            );
                        }
                    }
                }
                Err(e) => {
                    mark_failed!(sym);
                    if let Ok(st) = state.lock() {
                        if let Ok(mut progress) = st.fleet.download_progress.write() {
                            progress.last_error =
                                format!("{} [{}] fetch hatası: {}", sym, source_label, e);
                        }
                    }
                    push_state_log(
                        state,
                        format!("    └─ {} [{}] ❌ fetch hatası: {}", sym, source_label, e),
                    );

                    if venue.is_delisting_error(&e) {
                        let n_fail = delisted_record_failure(sym);
                        let threshold = delisted_detection_threshold();
                        if threshold > 0 && n_fail >= threshold {
                            Self::purge_delisted_symbol(state, sym, n_fail);
                        }
                    }
                }
            }
        }
        let mut auto_iv_candidates: Vec<String> = super::auto_interval_candidates();
        // Opportunity snapshot'larının kanonik zaman dilimi her zaman 15m'dir.
        // Operatör AUTO_INTERVAL_CANDIDATES ile başka TF'ler seçse bile 15m
        // tamamlayıcısı kaldırılmamalıdır; aksi halde scheduler tetiklense de
        // opportunity cache bayat kalabilir.
        if !auto_iv_candidates.iter().any(|interval| interval == "15m") {
            auto_iv_candidates.insert(0, "15m".into());
        }
        if !auto_iv_candidates.is_empty() {
            for sym in &symbols {
                let sym_iv = resolve_symbol_interval(sym);
                let Some((venue, bare)) = venue_registry.route(sym) else {
                    continue;
                };
                if venue.exchange() == crate::core::types::Exchange::Bist {
                    continue;
                }
                let (local_exchange_for_auto, local_market_for_auto, _) =
                    resolve_download_context(&venue, &market);

                for cand in auto_iv_candidates.iter().filter(|c| **c != sym_iv) {
                    if let Ok(st) = state.lock() {
                        if let Ok(mut progress) = st.fleet.download_progress.write() {
                            progress.current_symbol = sym.clone();
                            progress.current_interval = cand.clone();
                            progress.current_phase = "supplemental".into();
                        }
                    }
                    match venue.fetch_candles(bare, cand, limit).await {
                        Ok(c) if !c.is_empty() => {
                            if let Ok(st) = state.lock() {
                                st.fleet.opportunity_cache.push_bulk(c.clone());
                            }
                            let db2 = db_path.clone();
                            let cc = c.clone();
                            let (ex2, mk2) = (
                                local_exchange_for_auto.clone(),
                                local_market_for_auto.clone(),
                            );
                            let profile_c = profile.clone();
                            let _ = tokio::task::spawn_blocking(move || {
                                if let Ok(conn) = crate::persistence::open_db(&db2) {
                                    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
                                    for k in &cc {
                                        let _ = crate::persistence::writer::save_candle_for_profile(
                                            &conn, &profile_c, &ex2, &mk2, k,
                                        );
                                    }
                                }
                            })
                            .await;
                            log::info!("🌐 aday-TF mum: {} {} ({} mum)", sym, cand, c.len());
                            if let Ok(st) = state.lock() {
                                if let Ok(mut progress) = st.fleet.download_progress.write() {
                                    progress.fetched_candles += c.len();
                                    progress.last_completed_symbol = format!("{} [{}]", sym, cand);
                                }
                            }
                        }
                        Err(e) => {
                            if let Ok(st) = state.lock() {
                                if let Ok(mut progress) = st.fleet.download_progress.write() {
                                    progress.last_error = format!("{} [{}]: {}", sym, cand, e);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        Self::refresh_carry_funding(state, &db_path).await;

        if let Ok(st) = state.lock() {
            if let Ok(mut progress) = st.fleet.download_progress.write() {
                progress.failed_symbols = total_failed;
                progress.failed_symbol_names = failed_symbol_names;
            }
        }

        log::info!(
            "🌐 Download ✓ tamamlandı: profile={} {} mum (başarılı={}, başarısız={}, süre={}ms) · {}",
            profile,
            total_fetched,
            symbols.len() - total_failed,
            total_failed,
            download_started.elapsed().as_millis(),
            per_symbol_summary.join(" · "),
        );
        if let Ok(mut st) = state.lock() {
            st.push_log(format!(
                "🌐 Download ✓ tamamlandı: profile={} {} mum (başarılı={}, başarısız={}, süre={}ms) · {}",
                profile,
                total_fetched,
                symbols.len() - total_failed,
                total_failed,
                download_started.elapsed().as_millis(),
                per_symbol_summary.join(" · "),
            ));
        }
        emit_canonical_event(
            state,
            "pipeline",
            "DOWNLOAD_COMPLETED",
            "internal",
            "download",
            serde_json::json!({
                "profile": profile,
                "fetched": total_fetched,
                "succeeded": symbols.len() - total_failed,
                "failed": total_failed,
                "duration_ms": download_started.elapsed().as_millis()
            }),
        );

        if total_failed == symbols.len() {
            return Err(format!("tüm {} sembolde indirme başarısız", symbols.len()));
        } else {
            return Ok(());
        }
    }

    #[cfg(test)]
    fn _dummy() {}

    async fn refresh_carry_funding(state: &Arc<Mutex<AppState>>, db_path: &str) {
        let (symbols, venue_registry) = match state.lock().ok().map(|st| {
            let mut syms = vec![];
            if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                for w in orch.get_worker_status() {
                    syms.push(w.symbol.clone());
                }
            }
            if let Ok(positions) = st.finance.live_positions.read() {
                for sym in positions.keys() {
                    if !syms.contains(sym) {
                        syms.push(sym.clone());
                    }
                }
            }
            (syms, Arc::clone(&st.venue_registry))
        }) {
            Some(res) => res,
            None => return,
        };

        if symbols.is_empty() {
            return;
        }
        const FMARKET: &str = "futures";
        let now_ms = crate::core::time::now_epoch_millis() as i64;
        let seed_start = now_ms - 365 * 86_400 * 1000;
        let (mut ok, mut total) = (0usize, 0usize);

        for sym in &symbols {
            let Some((venue, bare)) = venue_registry.route(sym) else {
                continue;
            };

            if !venue
                .capabilities()
                .contains(&ExchangeCapability::FundingHistory)
            {
                continue;
            }

            let last = crate::persistence::reader::last_funding_ts(db_path, sym, FMARKET);
            let eff_start = match last {
                Some(l) => l + 1,
                None => seed_start,
            };
            if eff_start >= now_ms {
                continue;
            }

            let max_req = if last.is_some() { 2 } else { 5 };

            match venue
                .download_funding_history(bare, FMARKET, eff_start, max_req)
                .await
            {
                Ok(points) if !points.is_empty() => {
                    let db2 = db_path.to_string();
                    let sym_c = sym.clone();
                    let pts = points.clone();
                    let ex_name = venue.exchange().as_str().to_string();
                    let written = tokio::task::spawn_blocking(move || -> usize {
                        let conn = match crate::persistence::open_db(&db2) {
                            Ok(c) => c,
                            Err(_) => return 0,
                        };
                        let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
                        let mut w = 0usize;
                        for (t, r) in &pts {
                            if crate::persistence::writer::save_funding(
                                &conn, &ex_name, FMARKET, &sym_c, *t, *r,
                            )
                            .is_ok()
                            {
                                w += 1;
                            }
                        }
                        return w;
                    })
                    .await
                    .unwrap_or(0);
                    if written > 0 {
                        ok += 1;
                        total += written;
                    }
                }
                _ => {}
            }
        }
        if total > 0 {
            log::info!(
                "💰 funding refresh: {} sembol güncel · {} funding kaydı yazıldı",
                ok,
                total
            );
            push_state_log(
                state,
                format!("💰 funding refresh: {} sembol · {} kayıt", ok, total),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{download_symbol_has_registered_exchange, resolve_download_context};
    use crate::core::model::SymbolFilters;
    use crate::core::types::{AssetClass, Candle, Exchange, Market, Signal, Trade};
    use crate::robot::infra::interfaces::TradeExecutor;
    use crate::robot::venue::adapter::{MarketData, OrderExecution, VenueAdapter};
    use crate::robot::venue::types::{OrderReceipt, OrderRequest};
    use async_trait::async_trait;
    use memos_venue_api::Result;
    use std::sync::Arc;

    struct FakeVenue {
        exchange: Exchange,
        market: Market,
    }

    #[async_trait]
    impl MarketData for FakeVenue {
        async fn fetch_candles(
            &self,
            _symbol: &str,
            _interval: &str,
            _limit: usize,
        ) -> Result<Vec<Candle>> {
            Ok(vec![])
        }

        async fn book_ticker(&self, _symbol: &str) -> Result<(f64, f64)> {
            Ok((0.0, 0.0))
        }

        async fn symbol_filters(&self, _symbol: &str) -> Result<SymbolFilters> {
            Ok(SymbolFilters::default())
        }
    }

    #[async_trait]
    impl OrderExecution for FakeVenue {
        async fn submit_order(&self, _req: &OrderRequest) -> Result<OrderReceipt> {
            Err("unsupported".into())
        }

        async fn cancel_all(&self, _symbol: &str) -> memos_venue_api::Result<()> {
            Ok(())
        }

        async fn set_leverage(&self, _symbol: &str, _leverage: u32) -> Result<()> {
            Ok(())
        }

        async fn balance(&self) -> Result<f64> {
            Ok(0.0)
        }

        async fn fetch_open_orders(&self) -> core::result::Result<Vec<serde_json::Value>, String> {
            Ok(vec![])
        }
        async fn fetch_account_positions(
            &self,
        ) -> core::result::Result<Vec<serde_json::Value>, String> {
            Ok(vec![])
        }
    }

    impl VenueAdapter for FakeVenue {
        fn exchange(&self) -> Exchange {
            self.exchange
        }
        fn market(&self) -> Market {
            self.market
        }
        fn is_delisting_error(&self, _error_msg: &str) -> bool {
            false
        }
        fn execution_engine(&self) -> Option<Arc<dyn TradeExecutor>> {
            None
        }
        fn asset_class(&self) -> AssetClass {
            self.exchange.asset_class()
        }
    }

    #[test]
    fn resolve_download_context_uses_venue_specific_namespace() {
        let binance: Arc<dyn VenueAdapter> = Arc::new(FakeVenue {
            exchange: Exchange::Binance,
            market: Market::Futures,
        });
        let mt5: Arc<dyn VenueAdapter> = Arc::new(FakeVenue {
            exchange: Exchange::Mt5,
            market: Market::Cfd,
        });

        let (exchange1, market1, source1) = resolve_download_context(&binance, "spot");
        assert_eq!(exchange1, "binance");
        assert_eq!(market1, "futures");
        assert_eq!(source1, "binance-live");

        // MT5 artık KANONİK namespace kullanır: market = venue.market() = "cfd".
        // ÖNCEDEN "mt5" sabitiydi; okuma yolu ise "cfd" bekliyordu → aynı enstrümanın
        // geçmişi iki kovaya bölünüyordu. `exchange` mt5 ile ibkr'ı zaten ayırdığı için
        // ikisinin de "cfd" olması güvenli.
        let (exchange2, market2, source2) = resolve_download_context(&mt5, "futures");
        assert_eq!(exchange2, "mt5");
        assert_eq!(market2, "cfd", "kanonik: venue.market().as_str()");
        // Kaynak etiketi artık venue adından türer ("mt5-live") — eski "venue-live"
        // tüm Binance-dışını tek etikete eziyordu, teşhiste venue ayırt edilemiyordu.
        assert_eq!(source2, "mt5-live");

        // fallback_market ARTIK YOK SAYILIR: eskiden bilinmeyen venue'larda global
        // config.market sızıyordu (binance|futures altına forex serileri yazılmasının
        // sebebi buydu — 16.000 satır yanlış etiketlenmişti).
        let ibkr: Arc<dyn VenueAdapter> = Arc::new(FakeVenue {
            exchange: Exchange::Ibkr,
            market: Market::Cfd,
        });
        let (ex3, mk3, _) = resolve_download_context(&ibkr, "futures");
        assert_eq!(ex3, "ibkr");
        assert_eq!(mk3, "cfd", "fallback_market ('futures') sızmamalı");
    }

    #[test]
    fn download_candidates_do_not_fall_back_to_an_unrelated_default_venue() {
        let registry = crate::robot::venue::registry::VenueRegistry::from_specs(
            &[crate::core::types::VenueSpec::new(
                Exchange::Ibkr,
                Market::Cfd,
            )],
            None,
        );

        assert!(download_symbol_has_registered_exchange(
            "XLE@ibkr", &registry
        ));
        assert!(!download_symbol_has_registered_exchange(
            "BTCUSDT", &registry
        ));
        assert!(!download_symbol_has_registered_exchange(
            "BTCUSDT@binance",
            &registry
        ));
    }
}

pub(crate) fn gap_aware_fetch_limit(
    last_ms: Option<i64>,
    now_ms: i64,
    iv_secs: i64,
    base: usize,
) -> usize {
    return match last_ms {
        Some(last) => {
            let step = iv_secs.max(1) * 1000;
            let missing = ((now_ms - last) / step).max(0) as usize;
            (missing + 5).clamp(base, 1000)
        }
        None => base.clamp(50, 1000),
    };
}

pub(crate) fn deep_gap_start_ms(last_ms: Option<i64>, now_ms: i64, iv_secs: i64) -> Option<i64> {
    let last = last_ms?;
    let step = iv_secs.max(1) * 1000;
    let missing = (now_ms - last) / step;
    if missing > 1000 {
        return Some(last + step);
    } else {
        return None;
    }
}
