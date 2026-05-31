// src/robot/engines/master/jobs_download.rs — Download job: mum indirme + HTF + symbol-stats.
// Faz 2 modülerleştirme: jobs.rs'ten ayrıldı (davranış birebir korunur).
use super::*;

impl Engine {

    /// 🌐 Data Pipeline Download (Faz 4 - "download" trigger):
    /// Aktif sembol filosundaki her sembol için BinanceFetcher ile son N mumu çekip
    /// SQLite'a (candles_{symbol}_{interval} tablosu) yazar.
    ///
    /// Akış:
    /// 1. State'ten sembol listesi + interval + db_path + limit topla
    /// 2. Her sembol için fetch_latest çağır (paralel değil, sıralı — rate-limit dostu)
    /// 3. Başarılı çekimi spawn_blocking ile save_candle'a aktar (SQLite senkron API)
    /// 4. Her sembol için tek satır log + sonda toplam özet
    pub(crate) async fn run_download_job(state: &Arc<Mutex<AppState>>) -> std::result::Result<(), String> {
        use crate::robot::data_fetcher::binance::BinanceFetcher;
        use crate::robot::data_fetcher::market_fetcher::MarketFetcher;

        log::info!("🌐 E2: Data pipeline download başlatıldı...");

        // 1) Çalışma listesi — kilit kısa
        // Canlı feed'i olmayan borsa sembolleri (örn. BIST) download'a gönderilmez →
        // aksi halde "Veri Format Hatası" log kirliliği. Karar market-agnostik tek nokta:
        // RuntimeTuning.symbol_eligible_for_live (hydrate/price_poll/cycle ile aynı).
        let (symbols, interval, symbol_interval, db_path, limit) = {
            let st = state.lock().map_err(|e| format!("state lock: {}", e))?;
            let tuning = Arc::clone(&st.tuning);
            let eligible = |s: &str| tuning.symbol_eligible_for_live(s);

            let mut syms: Vec<String> = vec![];
            if eligible(&st.config.symbol) { syms.push(st.config.symbol.clone()); }
            // SymbolOrchestrator + pinned
            if let Ok(orch) = st.fleet.symbol_orchestrator.read() {
                for w in orch.get_worker_status() {
                    if !eligible(&w.symbol) { continue; }
                    if !syms.contains(&w.symbol) { syms.push(w.symbol); }
                }
            }
            for s in &st.config.pinned_symbols {
                if !eligible(s) { continue; }
                if !syms.contains(s) { syms.push(s.clone()); }
            }
            // Açık pozisyon sembolleri (recovery sonrası orchestrator register'ı
            // başarısız olursa veya trade pipeline-dışı bir sembolde açıldıysa
            // defensive olarak buradan da yakalıyoruz). Aksi halde stale candle
            // → price-sanity guard tetiklenir, pozisyon kapatılamaz.
            if let Ok(positions) = st.finance.live_positions.read() {
                for sym in positions.keys() {
                    if !eligible(sym) { continue; }
                    if !syms.contains(sym) { syms.push(sym.clone()); }
                }
            }
            syms.retain(|s| !s.is_empty());
            let symbol_interval = st.brain.parameters.read().ok()
                .map(|p| p.symbol_interval.clone()).unwrap_or_default();
            (syms, st.config.interval.clone(), symbol_interval, st.config.db_path.clone(),
             st.config.download_candle_limit.max(50))
        };

        if symbols.is_empty() {
            return Err("indirilecek sembol yok (config.symbol + pinned + orchestrator boş)".into());
        }

        log::info!(
            "🌐 Download başladı: {} sembol × {} mum (interval={}) → {}",
            symbols.len(), limit, interval, symbols.join(","),
        );
        push_state_log(state, format!(
            "🌐 Download başladı: {} sembol × {} mum (interval={})",
            symbols.len(), limit, interval,
        ));

        // 2) Her sembol için sırayla mum çek + DB'ye yaz
        let fetcher = BinanceFetcher::new();
        let mut total_fetched = 0usize;
        let mut total_failed = 0usize;
        let mut per_symbol_summary: Vec<String> = Vec::new();

        for sym in &symbols {
            // Per-sembol otonom interval; map'te yoksa config.interval (sıfır regresyon).
            let sym_iv = symbol_interval.get(sym).cloned().unwrap_or_else(|| interval.clone());
            match fetcher.fetch_latest(sym, &sym_iv, limit).await {
                Ok(candles) => {
                    // Başarılı fetch → delisted sayacını sıfırla (geçici hata
                    // sonrası sembol normalleştiyse yanlış pozitif olmasın).
                    delisted_record_success(sym);
                    let n = candles.len();
                    total_fetched += n;
                    // 3) SQLite yazımı senkron → spawn_blocking
                    let db_path_clone = db_path.clone();
                    let candles_clone = candles.clone();
                    // Yazımı gerçekten say + ilk hatayı yüzeye çıkar (eskiden `let _ =` ile
                    // yutuluyordu → şema uyumsuzluğunda sahte "✓ N mum yazıldı" basılıyordu).
                    let write_result = tokio::task::spawn_blocking(move || -> std::result::Result<(usize, Option<String>), String> {
                        let conn = rusqlite::Connection::open(&db_path_clone)
                            .map_err(|e| format!("db open: {}", e))?;
                        // WAL olsa da yazıcı-yazıcı çakışmasında anlık SQLITE_BUSY olabiliyor
                        // (snapshot/engine eşzamanlı yazımı) → busy_timeout ile bekle, "database
                        // is locked" ile mum düşürme.
                        let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
                        let mut written = 0usize;
                        let mut first_err: Option<String> = None;
                        for c in &candles_clone {
                            match crate::persistence::writer::save_candle(&conn, "binance", "spot", c) {
                                Ok(()) => written += 1,
                                Err(e) => if first_err.is_none() { first_err = Some(e.to_string()); },
                            }
                        }
                        Ok((written, first_err))
                    }).await;
                    match write_result {
                        // Fetch başarılı ama DB'ye HİÇ yazılamadıysa: bunu başarısızlık say,
                        // gerçek hatayı logla (sessiz veri donması yerine görünür sinyal).
                        Ok(Ok((0, err))) if n > 0 => {
                            total_failed += 1;
                            push_state_log(state, format!(
                                "    └─ {} ❌ fetch {} mum ama DB yazımı 0 (hata: {})",
                                sym, n, err.as_deref().unwrap_or("?"),
                            ));
                        }
                        Ok(Ok((written, err))) => {
                            let warn = match &err {
                                Some(e) => format!(" ⚠️ {} atlandı: {}", n.saturating_sub(written), e),
                                None => String::new(),
                            };
                            per_symbol_summary.push(format!("{}={}", sym, written));
                            // Otonom katman: sembol+interval bazlı noise floor hesabı.
                            // compute_symbol_stats min 64 candle istiyor (14 ATR + 50 sample);
                            // limit ≥50 garantili ama yetersizse None döner ve store
                            // güncellenmez → resolve_atr_mult fallback'e düşer.
                            if let Some(stats) = crate::robot::parameters::compute_symbol_stats(&candles) {
                                if let Ok(st) = state.lock() {
                                    if let Ok(mut params) = st.brain.parameters.write() {
                                        params.update_symbol_stats(sym, &sym_iv, stats);
                                    }
                                }
                            }
                            push_state_log(state, format!("    └─ {} ✓ {} mum yazıldı{}", sym, written, warn));

                            // Multi-TF Faz B c2/c3: HTF (üst zaman dilimi) mumlarını da indir.
                            // get_htf_interval base ile aynıysa atla (1d → 1d). HTF fetch
                            // başarısızsa sessiz geç — htf_trend_filter eksiklikte
                            // pass-through yapar, cycle yine de döner.
                            // MULTI_TF_DOWNLOAD=false → HTF fetch skip (base interval yeterli).
                            let download_htf = state.lock().ok()
                                .and_then(|st| st.brain.parameters.read().ok()
                                    .map(|p| p.multi_tf.enabled && p.multi_tf.download_htf))
                                .unwrap_or(true);
                            let htf_interval = crate::robot::data_pipeline::DataPipeline::get_htf_interval(&sym_iv);
                            if download_htf && htf_interval != sym_iv {
                                let htf_limit = (limit / 4).max(50);
                                match fetcher.fetch_latest(sym, htf_interval, htf_limit).await {
                                    Ok(htf_candles) if !htf_candles.is_empty() => {
                                        let htf_n = htf_candles.len();
                                        let db2 = db_path.clone();
                                        let htf_clone = htf_candles.clone();
                                        let _ = tokio::task::spawn_blocking(move || {
                                            if let Ok(conn) = rusqlite::Connection::open(&db2) {
                                                let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
                                                for c in &htf_clone {
                                                    let _ = crate::persistence::writer::save_candle(&conn, "binance", "spot", c);
                                                }
                                            }
                                        }).await;
                                        push_state_log(state, format!("        └─ {} HTF {} ✓ {} mum", sym, htf_interval, htf_n));
                                    }
                                    _ => {
                                        // HTF eksikliği fatal değil — loader fallback'i 1m varsa
                                        // CandleSynth ile in-memory üretebilir.
                                    }
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            total_failed += 1;
                            push_state_log(state, format!("    └─ {} ❌ yazma hatası: {}", sym, e));
                        }
                        Err(e) => {
                            total_failed += 1;
                            push_state_log(state, format!("    └─ {} ❌ blocking join hatası: {}", sym, e));
                        }
                    }
                }
                Err(e) => {
                    total_failed += 1;
                    log::warn!("🌐 Download fetch hatası: {} → {}", sym, e);
                    push_state_log(state, format!("    └─ {} ❌ fetch hatası: {}", sym, e));
                    // Delisted auto-detect: ardışık başarısızlık sayacı.
                    let n_fail = delisted_record_failure(sym);
                    let threshold = delisted_detection_threshold();
                    if threshold > 0 && n_fail >= threshold {
                        Self::purge_delisted_symbol(state, sym, n_fail);
                    }
                }
            }
        }

        // 3b) Faz 3 — Otonom interval eval'in chicken-egg'i: POOL-WIDE aday-TF download.
        // Her pool sembolü için AUTO_INTERVAL_CANDIDATES TF'lerini de çek (ana loop yalnız
        // o sembolün sym_iv'sini çeker) → run_backtest_job pool-wide interval eval'i GERÇEK
        // veriyle kıyaslayabilir. Bounded aday + sym_iv atlanır → API yükü sınırlı.
        // [[project_adaptive_regime]] Faz 3 · [[feedback_modular_dry_perf]].
        let auto_iv_candidates: Vec<String> = std::env::var("AUTO_INTERVAL_CANDIDATES")
            .unwrap_or_else(|_| "15m,1h".into())
            .split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        if !auto_iv_candidates.is_empty() {
            for sym in &symbols {
                let sym_iv = symbol_interval.get(sym).cloned().unwrap_or_else(|| interval.clone());
                for cand in auto_iv_candidates.iter().filter(|c| **c != sym_iv) {
                    match fetcher.fetch_latest(sym, cand, limit).await {
                        Ok(c) if !c.is_empty() => {
                            let db2 = db_path.clone();
                            let cc = c.clone();
                            let _ = tokio::task::spawn_blocking(move || {
                                if let Ok(conn) = rusqlite::Connection::open(&db2) {
                                    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
                                    for k in &cc {
                                        let _ = crate::persistence::writer::save_candle(&conn, "binance", "spot", k);
                                    }
                                }
                            }).await;
                            log::info!("🌐 aday-TF mum: {} {} ({} mum)", sym, cand, c.len());
                        }
                        _ => {}
                    }
                }
            }
        }

        // 4) Özet
        log::info!(
            "🌐 Download ✓ tamamlandı: {} mum (başarılı={}, başarısız={}) · {}",
            total_fetched,
            symbols.len() - total_failed,
            total_failed,
            per_symbol_summary.join(" · "),
        );
        if let Ok(mut st) = state.lock() {
            st.fleet.download_active = false;
            st.push_log(format!(
                "🌐 Download ✓ tamamlandı: {} mum (başarılı={}, başarısız={}) · {}",
                total_fetched,
                symbols.len() - total_failed,
                total_failed,
                per_symbol_summary.join(" · "),
            ));
        }

        if total_failed == symbols.len() {
            Err(format!("tüm {} sembolde indirme başarısız", symbols.len()))
        } else {
            Ok(())
        }
    }
}
