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
        let (symbols, interval, symbol_interval, db_path, limit, exchange, market,
             backfill_enabled, backfill_max_requests) = {
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
             st.config.download_candle_limit.max(50),
             st.config.exchange.clone(), st.config.market.clone(),
             tuning.backfill_enabled, tuning.backfill_max_requests)
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
            // Faz 2 gap-farkında çekim: son kayıtlı bardan ŞİMDİYE kadar eksik bar kadar
            // (tampon + base limit; Binance tek-istek tavanı 1000) çek → forward gap kapanır.
            // Kayıt yoksa base limit ile tohumla. Market-SAF son ts (Faz 0+1).
            let iv_secs = crate::robot::data_pipeline::DataNormalizer::parse_interval(&sym_iv).max(1) as i64;
            let last_ms = crate::persistence::reader::last_candle_ts(&db_path, sym, &sym_iv, &market);
            let now_ms = crate::core::time::now_epoch_millis() as i64;
            // Derin gap (>1000 bar geride): startTime-pagination ile gap'in BAŞINDAN ileri
            // doldur (fetch_latest son-1000 çekip aradaki deliği kalıcı bırakırdı). Sığ
            // gap/yeni sembol → tek-istek son-N (mevcut Faz 2 yolu, sıfır regresyon).
            let fetch_fut = match backfill_enabled
                .then(|| deep_gap_start_ms(last_ms, now_ms, iv_secs))
                .flatten()
            {
                Some(start_ms) => {
                    push_state_log(state, format!(
                        "    └─ {} 🕳️ derin gap → backfill (start={}, ≤{}×1000 bar)",
                        sym, start_ms, backfill_max_requests,
                    ));
                    fetcher.fetch_history_market(sym, &sym_iv, &market, start_ms, iv_secs, backfill_max_requests).await
                }
                None => {
                    let fetch_limit = gap_aware_fetch_limit(last_ms, now_ms, iv_secs, limit);
                    fetcher.fetch_latest_market(sym, &sym_iv, &market, fetch_limit).await
                }
            };
            match fetch_fut {
                Ok(candles) => {
                    // Başarılı fetch → delisted sayacını sıfırla (geçici hata
                    // sonrası sembol normalleştiyse yanlış pozitif olmasın).
                    delisted_record_success(sym);
                    let n = candles.len();
                    total_fetched += n;
                    // 3) SQLite yazımı senkron → spawn_blocking
                    let db_path_clone = db_path.clone();
                    let candles_clone = candles.clone();
                    let exchange_c = exchange.clone();
                    let market_c = market.clone();
                    // Yazımı gerçekten say + ilk hatayı yüzeye çıkar (eskiden `let _ =` ile
                    // yutuluyordu → şema uyumsuzluğunda sahte "✓ N mum yazıldı" basılıyordu).
                    let write_result = tokio::task::spawn_blocking(move || -> std::result::Result<(usize, Option<String>), String> {
                        let conn = crate::persistence::open_db(&db_path_clone)
                            .map_err(|e| format!("db open: {}", e))?;
                        // WAL olsa da yazıcı-yazıcı çakışmasında anlık SQLITE_BUSY olabiliyor
                        // (snapshot/engine eşzamanlı yazımı) → busy_timeout ile bekle, "database
                        // is locked" ile mum düşürme.
                        let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
                        let mut written = 0usize;
                        let mut first_err: Option<String> = None;
                        for c in &candles_clone {
                            match crate::persistence::writer::save_candle(&conn, &exchange_c, &market_c, c) {
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
                                match fetcher.fetch_latest_market(sym, htf_interval, &market, htf_limit).await {
                                    Ok(htf_candles) if !htf_candles.is_empty() => {
                                        let htf_n = htf_candles.len();
                                        let db2 = db_path.clone();
                                        let htf_clone = htf_candles.clone();
                                        let (ex2, mk2) = (exchange.clone(), market.clone());
                                        let _ = tokio::task::spawn_blocking(move || {
                                            if let Ok(conn) = crate::persistence::open_db(&db2) {
                                                let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
                                                for c in &htf_clone {
                                                    let _ = crate::persistence::writer::save_candle(&conn, &ex2, &mk2, c);
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
        // Her pool sembolü için aday TF'leri de çek (ana loop yalnız o sembolün sym_iv'sini
        // çeker) → run_backtest_job pool-wide interval eval'i GERÇEK veriyle kıyaslayabilir.
        // auto_interval_candidates TEK KAYNAK (jobs_backtest ile aynı default 15m,1h,4h,1d).
        // Bounded aday + sym_iv atlanır → API yükü sınırlı. [[project_adaptive_regime]] Faz 3.
        let auto_iv_candidates: Vec<String> = super::auto_interval_candidates();
        if !auto_iv_candidates.is_empty() {
            for sym in &symbols {
                let sym_iv = symbol_interval.get(sym).cloned().unwrap_or_else(|| interval.clone());
                for cand in auto_iv_candidates.iter().filter(|c| **c != sym_iv) {
                    match fetcher.fetch_latest_market(sym, cand, &market, limit).await {
                        Ok(c) if !c.is_empty() => {
                            let db2 = db_path.clone();
                            let cc = c.clone();
                            let (ex2, mk2) = (exchange.clone(), market.clone());
                            let _ = tokio::task::spawn_blocking(move || {
                                if let Ok(conn) = crate::persistence::open_db(&db2) {
                                    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
                                    for k in &cc {
                                        let _ = crate::persistence::writer::save_candle(&conn, &ex2, &mk2, k);
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

        // 3c) 💰 FUNDING-CARRY canlı refresh: carry mod açıksa sepet sembollerinin funding'ini artımlı
        // çek (gap-farkında). Funding kitabının taze yakıtı. Mod kapalı → no-op. [[project_funding_carry]]
        Self::refresh_carry_funding(&fetcher, state, &db_path).await;

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

    /// 💰 FUNDING-CARRY canlı funding refresh — carry mod (carry_live.enabled) açıkken sepet
    /// sembollerinin funding-rate'ini ARTIMLI (gap-farkında) çekip DB'ye yazar; mod kapalı → no-op.
    /// Funding yalnız FUTURES'ta var → market sabit "futures" (download.market spot olsa da). Gap-farkında
    /// başlangıç: kayıt varsa son funding_time+1ms, yoksa ~1 yıl tohum (lookback penceresini bolca kapsar).
    /// Funding 8 saatte bir (~3/gün) → her download cycle'ında bir refresh fazlasıyla yeter; artımlıda
    /// tek sayfa yeterli (modest istek tavanı). Per-sembol hata izole. download_funding aracıyla DRY desen.
    async fn refresh_carry_funding(
        fetcher: &crate::robot::data_fetcher::binance::BinanceFetcher,
        state: &Arc<Mutex<AppState>>,
        db_path: &str,
    ) {
        let (enabled, symbols) = state.lock().ok()
            .and_then(|st| st.brain.parameters.read().ok()
                .map(|p| (p.carry_live.enabled, p.carry_live.symbols.clone())))
            .unwrap_or((false, Vec::new()));
        if !enabled || symbols.is_empty() {
            return;
        }
        const FMARKET: &str = "futures"; // funding yalnız futures'ta [[feedback_market_agnostic]]
        let now_ms = crate::core::time::now_epoch_millis() as i64;
        let seed_start = now_ms - 365 * 86_400 * 1000; // tohum: ~1 yıl funding geçmişi
        let (mut ok, mut total) = (0usize, 0usize);
        for sym in &symbols {
            let last = crate::persistence::reader::last_funding_ts(db_path, sym, FMARKET);
            let eff_start = match last { Some(l) => l + 1, None => seed_start };
            if eff_start >= now_ms {
                continue; // güncel → atla
            }
            // Artımlıda tek sayfa (≤1000 funding) yeter; tohumda ~1100 kayıt için birkaç sayfa.
            let max_req = if last.is_some() { 2 } else { 5 };
            match fetcher.fetch_funding_history(sym, FMARKET, eff_start, max_req).await {
                Ok(points) if !points.is_empty() => {
                    let db2 = db_path.to_string();
                    let sym_c = sym.clone();
                    let pts = points.clone();
                    let written = tokio::task::spawn_blocking(move || -> usize {
                        let conn = match crate::persistence::open_db(&db2) { Ok(c) => c, Err(_) => return 0 };
                        let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
                        let mut w = 0usize;
                        for (t, r) in &pts {
                            if crate::persistence::writer::save_funding(&conn, "binance", FMARKET, &sym_c, *t, *r).is_ok() {
                                w += 1;
                            }
                        }
                        w
                    }).await.unwrap_or(0);
                    if written > 0 { ok += 1; total += written; }
                }
                _ => {} // artımlı boş / delisted / geçici hata → sessiz geç (tazelik kapısı kitapta eler)
            }
        }
        if total > 0 {
            log::info!("💰 funding refresh: {} sembol güncel · {} funding kaydı yazıldı", ok, total);
            push_state_log(state, format!("💰 funding refresh: {} sembol · {} kayıt", ok, total));
        }
    }
}

/// Gap-farkında fetch limit (Faz 2): son kayıttan ŞİMDİYE kadar eksik bar + tampon,
/// `[base, 1000]` (Binance tek-istek tavanı) clamp. Kayıt yoksa base ([50,1000] clamp).
/// Saf/test-edilebilir; download döngüsü her sembol×interval için bunu kullanır.
pub(crate) fn gap_aware_fetch_limit(last_ms: Option<i64>, now_ms: i64, iv_secs: i64, base: usize) -> usize {
    match last_ms {
        Some(last) => {
            let step = iv_secs.max(1) * 1000;
            let missing = ((now_ms - last) / step).max(0) as usize;
            (missing + 5).clamp(base, 1000)
        }
        None => base.clamp(50, 1000),
    }
}

/// 🕳️ Derin-gap kararı (Faz 2 follow-up): son kayıt ŞİMDİDEN >1000 bar geride ise
/// pagination'ın başlayacağı `start_ms`'i (= son mum + 1 interval) döndürür; aksi halde
/// `None` (tek-istek son-N yolu yeterli — `gap_aware_fetch_limit` zaten kapatır). Kayıt
/// yoksa (`None`) backfill TETİKLENMEZ: yeni sembolde "derin geçmiş" ≠ "gap", tek-istek
/// tohumlaması yapılır (deep-history seed ayrı bir mod). Saf/test-edilebilir.
pub(crate) fn deep_gap_start_ms(last_ms: Option<i64>, now_ms: i64, iv_secs: i64) -> Option<i64> {
    let last = last_ms?;
    let step = iv_secs.max(1) * 1000;
    let missing = (now_ms - last) / step;
    if missing > 1000 {
        Some(last + step) // son kayıtlı mumdan bir interval ÖTESİ → üst-üste binme yok
    } else {
        None
    }
}

#[cfg(test)]
mod download_tests {
    use super::{gap_aware_fetch_limit, deep_gap_start_ms};

    #[test]
    fn deep_gap_triggers_backfill_from_just_after_last() {
        // 5000 bar (1m) geride → backfill, start = last + 1 interval.
        let iv = 60i64;
        let step = iv * 1000;
        let now = 10_000_000 * step;
        let last = now - 5000 * step;
        assert_eq!(deep_gap_start_ms(Some(last), now, iv), Some(last + step));
    }

    #[test]
    fn shallow_gap_no_backfill() {
        // 1000 bar tam → eşik > 1000 değil → None (tek-istek yolu kapatır).
        let iv = 60i64; let step = iv * 1000;
        let now = 10_000_000 * step;
        assert_eq!(deep_gap_start_ms(Some(now - 1000 * step), now, iv), None);
        // 1001 bar → backfill.
        assert!(deep_gap_start_ms(Some(now - 1001 * step), now, iv).is_some());
    }

    #[test]
    fn no_record_no_backfill() {
        // Kayıt yok → backfill tetiklenmez (yeni sembol tek-istekle tohumlanır).
        assert_eq!(deep_gap_start_ms(None, 1_000_000_000, 60), None);
    }

    #[test]
    fn no_record_seeds_with_base() {
        assert_eq!(gap_aware_fetch_limit(None, 1_000_000, 60, 500), 500);
        assert_eq!(gap_aware_fetch_limit(None, 1_000_000, 60, 20), 50, "base<50 → 50");
        assert_eq!(gap_aware_fetch_limit(None, 1_000_000, 60, 5000), 1000, "base>1000 → 1000");
    }

    #[test]
    fn small_gap_returns_base_floor() {
        // 10 bar (1h) eksik → base 500'ün altında → base zemini.
        let now = 10_000 * 3600_000;
        let last = now - 10 * 3600_000;
        assert_eq!(gap_aware_fetch_limit(Some(last), now, 3600, 500), 500);
    }

    #[test]
    fn medium_gap_covers_missing_plus_buffer() {
        // 100 bar (1h) eksik, base 50 → missing+5 = 105.
        let now = 10_000 * 3600_000;
        let last = now - 100 * 3600_000;
        assert_eq!(gap_aware_fetch_limit(Some(last), now, 3600, 50), 105);
    }

    #[test]
    fn huge_gap_caps_at_1000() {
        // 5000 bar eksik → 1000 tavanı.
        let now = 10_000 * 60_000;
        let last = now - 5000 * 60_000;
        assert_eq!(gap_aware_fetch_limit(Some(last), now, 60, 50), 1000);
    }
}
