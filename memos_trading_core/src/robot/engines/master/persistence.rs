// src/robot/engines/master/persistence.rs — DB şema/hidrasyon/persist + delisted purge + BIST filtresi
// Faz 1 modülerleştirme: master.rs'ten taşındı (davranış birebir korunur).
use super::*;

/// Adım 5.5 — DB persist'lerini serileştiren süreç-global kilit. `spawn_blocking` ile
/// arka plana atılan yazımların aynı anda birden çok SQLite bağlantısı açıp
/// "database is locked" hatasıyla SESSİZCE kaybolmasını önler (yazımlar `let _ =`).
/// Yazımlar kısa → contention düşük.
static DB_PERSIST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Bloklayan DB yazımını async hot-path'i bloklamadan çalıştırır (Adım 5.5: "emir
/// anında arka planda DB'ye yazılır"). Tokio runtime içindeysek `spawn_blocking`
/// (detached, fire-and-forget) → execute_trade_cycle worker'ı SQLite I/O'da bloklanmaz;
/// runtime yoksa (senkron testler / runtime-dışı çağrı) inline çalışır (backward-compat).
/// Sıralama: yazımlar DB_PERSIST_LOCK ile serileşir; ardışık hızlı yazımlarda kesin
/// "son kazanır" garantisi yok ama yazımlar sık (her open/close) → bayat snapshot
/// hemen ezilir. Veri çağrı anında klonlandığı için kilit içeriği taşımaz.
fn spawn_db_write<F: FnOnce() + Send + 'static>(f: F) {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => { handle.spawn_blocking(f); }
        Err(_) => f(),
    }
}

impl Engine {

    /// Boot'ta SQLite şemasını defensive yaratır. Cold-start'ta candle tablosu
    /// yoksa ML retrain trigger'ı her 500ms hata atıyordu. Her iki tablo da
    /// `CREATE IF NOT EXISTS` idempotent — mevcut DB'lere zarar vermez.
    pub(crate) fn ensure_db_schema(state: &Arc<Mutex<AppState>>) {
        let db_path = match state.lock() {
            Ok(st) => st.config.db_path.clone(),
            Err(_) => return,
        };
        let conn = match crate::persistence::open_db(&db_path) {
            Ok(c) => c,
            Err(e) => {
                push_state_log(state, format!("⚠️ ensure_db_schema: DB açılamadı ({}) — devam ediliyor", e));
                return;
            }
        };
        if let Err(e) = crate::persistence::writer::ensure_candles_table(&conn) {
            log::warn!("⚠️ candles tablosu kurulamadı: {}", e);
            push_state_log(state, format!("⚠️ candles tablosu kurulamadı: {}", e));
        } else if let Ok(mut st) = state.lock() {
            st.push_log_mirror(
                "📐 SQLite şeması doğrulandı (candles + open_positions_snapshot + account_state)".to_string(),
            );
        }
        // Faz 0: candle şemasını market-farkında unique key'e taşı (idempotent;
        // taşınmışsa no-op). Boot'ta yazımlardan ÖNCE çalışır → spot/futures çarpışması biter.
        if let Err(e) = crate::persistence::writer::migrate_candle_schema(&conn) {
            log::warn!("⚠️ candle şema migration başarısız: {}", e);
            push_state_log(state, format!("⚠️ candle şema migration başarısız: {}", e));
        }
        // open_positions_snapshot ayrıca save_open_positions_snapshot içinde
        // ilk INSERT öncesi yaratılıyor; ek bir CREATE çağrısına gerek yok.
        if let Err(e) = crate::persistence::writer::ensure_account_state_table(&conn) {
            log::warn!("⚠️ account_state tablosu kurulamadı: {}", e);
            push_state_log(state, format!("⚠️ account_state tablosu kurulamadı: {}", e));
        }
    }

    /// 🌱 edge_scan SEED görünürlüğü (TUI state-log + kalıcı dosya logu). `ParameterStore.symbol_strategy`
    /// boot'ta YALNIZ seed'den dolar (ParameterStore tek-kaynak `from_env`, disk reload yok; backtest job
    /// henüz koşmadı) → buradaki sayım = seed'lenen sembol sayısı. TUI'de logger backend yok →
    /// `push_state_log` paneli sağlar (bellek-içi ring; rtc_tui + headless ortak). AYRICA `log::info!` ile
    /// TF'li TAM liste `robotic_trading.log`'a düşer → ring kaymadan kalıcı kalır, `grep "edge seed"` ile
    /// her boot'ta yetkili seed bütünlüğü okunur (panel önizlemesi 8'le sınırlı, TF'siz; dosya logu değil).
    /// EDGE_SEED_REPORT set ama 0 yüklendiyse "WF-onaylı aday yok" notu (sessiz-0 yerine görünür sinyal).
    /// [[project_edge_scan]].
    /// 🌱→🚮 Boot'ta seed'i registry'den geçir: GERÇEKTEN dışlanmış (oturum-içi delisted-skip ya da
    /// exchangeInfo'da açıkça TRADING-dışı: BREAK/HALT/delisted) seed sembolleri symbol_strategy +
    /// symbol_interval'den at → force-pinned olup canlıda işlem görmeyecek/purge gürültüsü yapacak
    /// sembol seed'i sürüklemesin (ALPACAUSDT-BREAK tipi). LENIENT: registry'de OLMAYAN (bilinmeyen)
    /// sembol KORUNUR — registry eksik/bayatsa geçerli seed'i yanlış atmamak için (`is_symbol_tradeable`
    /// bilinmeyene true döner → yalnız AÇIKÇA non-TRADING düşer). `report_edge_seed` ÖNCESİ çağrılır →
    /// log nihai (prune sonrası) seti yansıtır. [[project_symbol_status_registry]] [[project_edge_scan]].
    pub(crate) fn prune_seed_ineligible(state: &Arc<Mutex<AppState>>) {
        let dropped: Vec<String> = {
            let Ok(st) = state.lock() else { return };
            let Ok(mut params) = st.brain.parameters.write() else { return };
            if params.symbol_strategy.is_empty() { return; }
            let drop: Vec<String> = params.symbol_strategy.keys()
                .filter(|s| super::is_delisted_skipped(s) || !super::is_symbol_tradeable(s))
                .cloned().collect();
            for s in &drop {
                params.symbol_strategy.remove(s);
                params.symbol_interval.remove(s);
            }
            drop
        };
        if dropped.is_empty() { return; }
        let mut sorted = dropped;
        sorted.sort();
        push_state_log(state, format!(
            "🚮 edge seed: {} sembol registry-dışlandı (delisted/non-TRADING) → seed'den atıldı: {}",
            sorted.len(), sorted.join(", ")));
        log::info!("🚮 edge seed registry-prune: {} atıldı — {}", sorted.len(), sorted.join(", "));
    }

    pub(crate) fn report_edge_seed(state: &Arc<Mutex<AppState>>) {
        let seed_path = std::env::var("EDGE_SEED_REPORT").ok().filter(|s| !s.trim().is_empty());
        // (sembol, interval, strateji) — TF'yi de taşı ki dosya logu Fix A'yı (BB 1d'de) teyit edebilsin.
        let mut entries: Vec<(String, String, String)> = match state.lock() {
            Ok(st) => st.brain.parameters.read().ok()
                .map(|p| p.symbol_strategy.iter()
                    .map(|(sym, strat)| (
                        sym.clone(),
                        p.symbol_interval.get(sym).cloned().unwrap_or_else(|| "?".into()),
                        strat.clone(),
                    ))
                    .collect())
                .unwrap_or_default(),
            Err(_) => return,
        };
        entries.sort(); // deterministik dosya logu (sembol alfabetik).
        if !entries.is_empty() {
            // Panel: kısa önizleme (ilk 8 sembol→strateji, bellek-içi).
            let mut preview: Vec<String> =
                entries.iter().take(8).map(|(s, _iv, st)| format!("{s}→{st}")).collect();
            if entries.len() > 8 { preview.push(format!("+{} daha", entries.len() - 8)); }
            push_state_log(state, format!(
                "🌱 edge seed: {} sembol→strateji yüklendi ({})", entries.len(), preview.join(", ")));
            // Dosya logu: TF'li TAM liste (kalıcı; ring kaymadan grep'lenebilir).
            let full: Vec<String> =
                entries.iter().map(|(s, iv, st)| format!("{s} {iv}/{st}")).collect();
            log::info!("🌱 edge seed: {} aday yüklendi — {}", entries.len(), full.join(", "));
        } else if let Some(path) = seed_path {
            push_state_log(state, format!(
                "🌱 edge seed: '{}' okundu ama 0 WF-onaylı aday → symbol_strategy boş (global/auto sürer)", path));
            log::info!("🌱 edge seed: '{}' okundu ama 0 WF-onaylı aday → symbol_strategy boş", path);
        }
        // EDGE_SEED_REPORT yokken ve map boşken: log YOK (gürültüsüz).
    }

    /// Boot sırasında önceki run'un `open_positions_snapshot` tablosundan
    /// açık pozisyonları okur ve `live_positions` HashMap'ine hidrate eder.
    /// - Tablo yoksa / kayıt yoksa: sessiz geçer (cold-start).
    /// - DB açılamazsa: hata log'una düşer ama engine devam eder.
    /// - Halihazırda live_positions'ta aynı sembol varsa: DB tarafı ezilir
    ///   (recovery sırasında live state boş olmalı; defensive).
    pub(crate) async fn hydrate_open_positions_from_db(state: &Arc<Mutex<AppState>>) {
        let (db_path, interval, tuning) = match state.lock() {
            Ok(st) => (st.config.db_path.clone(), st.config.interval.clone(), Arc::clone(&st.tuning)),
            Err(_) => return,
        };
        match crate::persistence::reader::recover_open_positions(&db_path) {
            Ok(positions) if !positions.is_empty() => {
                // İki kademeli filtre:
                //   1) Borsa eligibility (canlı feed yok → cycle dışı; market-agnostik).
                //   2) Candles existence — sembol+interval için en az 1 candle.
                // Atlananlar repair_log'a düşürülür; operatör görür.
                let mut loaded  = Vec::new();
                let mut stale   = Vec::new();   // candles yok
                let mut no_feed = Vec::new();   // borsasının canlı feed'i yok (örn. BIST)
                for pos in positions {
                    if !tuning.symbol_eligible_for_live(&pos.symbol) {
                        no_feed.push(pos);
                        continue;
                    }
                    let has_candles = crate::persistence::reader::read_candles(
                        &db_path, &pos.symbol, &interval, 1,
                    ).map(|v| !v.is_empty()).unwrap_or(false);
                    if has_candles { loaded.push(pos); }
                    else            { stale.push(pos); }
                }
                let n_loaded  = loaded.len();
                let n_stale   = stale.len();
                let n_no_feed = no_feed.len();
                let stale_syms:   Vec<String> = stale.iter().map(|p| p.symbol.clone()).collect();
                let no_feed_syms: Vec<String> = no_feed.iter().map(|p| p.symbol.clone()).collect();

                let market_for_orch = {
                    let st = match state.lock() { Ok(s) => s, Err(_) => return };
                    st.config.market.clone()
                };
                let loaded_syms: Vec<String> = loaded.iter().map(|p| p.symbol.clone()).collect();
                if let Ok(st) = state.lock() {
                    if let Ok(mut map) = st.finance.live_positions.write() {
                        for pos in loaded { map.insert(pos.symbol.clone(), pos); }
                    }
                }
                // Recovery edilen pozisyon sembollerini orchestrator'a register et.
                // Aksi halde run_download_job sadece pinned + screener workers'ı
                // dolduruyor → ATUSDT/BCHUSDT gibi recovery sembollerinin candle'ı
                // güncellenmiyor (DB 2-3 ay eski kalıyor). Idempotent: zaten
                // varsa register no-op.
                if let Ok(st) = state.lock() {
                    if let Ok(mut orch) = st.fleet.symbol_orchestrator.write() {
                        for sym in &loaded_syms {
                            orch.register(sym, &market_for_orch, &interval);
                        }
                    }
                }
                if let Ok(mut st) = state.lock() {
                    let mut msg = format!("♻️ [RECOVERY] {} yüklendi", n_loaded);
                    if n_stale > 0 {
                        msg.push_str(&format!(
                            " · {} stale ({} interval'inde candles yok): {}",
                            n_stale, interval, stale_syms.join(","),
                        ));
                    }
                    if n_no_feed > 0 {
                        msg.push_str(&format!(
                            " · {} feed'siz borsa-excluded (FORCE_LIVE_EXCHANGES ile aç): {}",
                            n_no_feed, no_feed_syms.join(","),
                        ));
                    }
                    st.push_log_mirror(msg);
                    if n_stale > 0 || n_no_feed > 0 {
                        st.guardian.repair_log.push_back(format!(
                            "[{}] recovery: yüklendi={} stale={} no_feed={}",
                            chrono::Local::now().format("%H:%M:%S"),
                            n_loaded, n_stale, n_no_feed,
                        ));
                        while st.guardian.repair_log.len() > 100 { st.guardian.repair_log.pop_front(); }
                    }
                }
            }
            Ok(_) => {
                if let Ok(mut st) = state.lock() {
                    st.push_log_mirror("♻️ [RECOVERY] DB snapshot boş — cold-start".to_string());
                }
            }
            Err(e) => {
                log::warn!("⚠️ [RECOVERY] snapshot okunamadı: {} — cold-start'a düşülüyor", e);
                push_state_log(state, format!(
                    "⚠️ [RECOVERY] open_positions_snapshot okunamadı: {} (cold-start'a düşülüyor)",
                    e,
                ));
            }
        }
    }

    /// Heuristik BIST hisse kodu tespiti.
    ///
    /// Kural:
    ///   - 3-6 karakter uzunluğunda
    ///   - Sadece A-Z + 0-9 (BIST: AKBNK, ALARK, A1CAP, ADGYO ...)
    ///   - Yaygın crypto quote suffix'i YOK (USDT/USDC/BUSD/FDUSD/TUSD/DAI)
    ///
    /// Yanılgı payı: 5-6 char crypto pair'leri (ETHBTC, BNBBTC vb.) BIST sayılabilir.
    /// Bu yüzden BIST exclude default ON ama opt-out env (ALLOW_BIST=1) var.
    /// Operatör kripto-only çalışıyorsa default ON güvenli (BIST verisi pratikte yok).
    /// Geriye-uyum sarmalayıcı: artık tek-kaynak `Exchange::classify`'a delege eder.
    /// Yeni kodda market-agnostik `RuntimeTuning::symbol_eligible_for_live` tercih edilmeli.
    pub fn looks_like_bist_symbol(sym: &str) -> bool {
        crate::core::types::Exchange::classify(sym) == crate::core::types::Exchange::Bist
    }

    /// Mevcut `live_positions` haritasını DB'ye snapshot'lar. Pozisyon açılış
    /// ve kapanış sonunda çağrılır; ENGINE crash + restart durumunda recovery
    /// bu snapshot'ı okuyup haritayı yeniden kurar.
    /// Senkron — Connection::open + INSERT/UPDATE; UI lock dışında çağrılmalı.
    pub fn persist_open_positions_snapshot(state: &Arc<Mutex<AppState>>) {
        let (db_path, positions) = match state.lock() {
            Ok(st) => {
                let db_path = st.config.db_path.clone();
                let positions: Vec<_> = st.finance.live_positions.read()
                    .map(|m| m.values().cloned().collect())
                    .unwrap_or_default();
                (db_path, positions)
            }
            Err(_) => return,
        };
        // Adım 5.5: yazım arka planda (detached) + serileştirilmiş → cycle bloklanmaz.
        spawn_db_write(move || {
            let _guard = DB_PERSIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            if let Ok(conn) = crate::persistence::open_db(&db_path) {
                let _ = crate::persistence::writer::save_open_positions_snapshot(&conn, &positions);
            }
        });
    }

    /// Boot'ta `account_state` tablosundan equity/peak/closed_count'ı okur ve
    /// FinanceVault'a uygular. Sanity guard: persisted starting_capital config
    /// ile uyuşmuyorsa (operatör cüzdanı resetledi) recovery atlanır.
    /// Tablo yoksa veya kayıt yoksa → cold-start (varsayılan değerler korunur).
    pub(crate) fn hydrate_account_state_from_db(state: &Arc<Mutex<AppState>>) {
        let (db_path, config_capital) = match state.lock() {
            Ok(st) => (st.config.db_path.clone(), st.config.capital),
            Err(_) => return,
        };
        match crate::persistence::reader::load_account_state(&db_path) {
            Ok(Some(rec)) => {
                // Operatör starting_capital'ı değiştirdiyse (config yeniden yazıldı)
                // eski equity'i hidrate etmek tutarsız olur → cold-start.
                let capital_match = (rec.starting_capital - config_capital).abs() < 1e-6;
                if !capital_match {
                    if let Ok(mut st) = state.lock() {
                        st.push_log_mirror(format!(
                            "♻️ [RECOVERY] account_state atlandı — starting_capital değişmiş (DB={:.2}, cfg={:.2})",
                            rec.starting_capital, config_capital,
                        ));
                    }
                    return;
                }
                if let Ok(mut st) = state.lock() {
                    st.finance.equity = rec.equity;
                    st.finance.peak_equity = rec.peak_equity.max(rec.equity);
                    st.finance.closed_trades_total.store(
                        rec.closed_trades_count, Ordering::Relaxed,
                    );
                    // Equity history baseline'ı: persist edilen equity ile başla.
                    if let Ok(mut hist) = st.finance.equity_history.write() {
                        hist.clear();
                        hist.push_back(rec.equity);
                    }
                    st.push_log_mirror(format!(
                        "♻️ [RECOVERY] equity=${:.2} peak=${:.2} closed={} (snapshot: {})",
                        rec.equity, rec.peak_equity, rec.closed_trades_count, rec.updated_at,
                    ));
                }
            }
            Ok(None) => {
                if let Ok(mut st) = state.lock() {
                    st.push_log_mirror(
                        "♻️ [RECOVERY] account_state boş — cold-start (equity başlangıç)".to_string(),
                    );
                }
            }
            Err(e) => {
                log::warn!("⚠️ [RECOVERY] account_state okunamadı: {} — cold-start'a düşülüyor", e);
                push_state_log(state, format!(
                    "⚠️ [RECOVERY] account_state okunamadı: {} (cold-start'a düşülüyor)",
                    e,
                ));
            }
        }
    }

    /// 🗂️ Boot'ta sembol-statü registry'sini DB'den cache'e yükler. Restart sonrası
    /// ilk exchangeInfo fetch'ini (scheduler warmup) beklemeden BREAK/delisted semboller
    /// dışlanmış olur. Tablo yoksa / boşsa sessiz geçer (refresh job dolduracak).
    pub(crate) fn hydrate_symbol_status_from_db(state: &Arc<Mutex<AppState>>) {
        let db_path = match state.lock() { Ok(st) => st.config.db_path.clone(), Err(_) => return };
        if let Ok(entries) = crate::persistence::reader::load_symbol_statuses(&db_path) {
            if !entries.is_empty() {
                let n_break = entries.iter().filter(|(_, s)| s != "TRADING").count();
                super::set_symbol_statuses(&entries);
                if let Ok(mut st) = state.lock() {
                    st.push_log_mirror(format!(
                        "🗂️ [RECOVERY] sembol statü registry: {} sembol ({} TRADING-dışı)",
                        entries.len(), n_break,
                    ));
                }
            }
        }
    }

    /// Delisted sembolü orchestrator + live_positions'tan temizler. Açık pozisyon
    /// varsa ClosedTradeModel'a PnL=0/reason=DELISTED ile push edilir (gerçek
    /// kapanış mümkün değil — Binance Invalid symbol döndürüyor).
    /// Idempotent: tekrar çağırma güvenli (orchestrator/live_positions yoksa no-op).
    pub fn purge_delisted_symbol(state: &Arc<Mutex<AppState>>, symbol: &str, n_fail: u32) {
        log::warn!(
            "🚮 Delisted detection: {} sembolü {} ardışık fetch hatasından sonra purge ediliyor",
            symbol, n_fail,
        );
        // Kalıcı dışlama: symbol_eligible_for_live artık reddeder → screener tekrar
        // seçse bile price_poll/cycle/download yoklamaz (geri gelme döngüsü biter).
        super::mark_delisted_skip(symbol);
        if let Ok(mut st) = state.lock() {
            // 1) Orchestrator'dan çıkar (stop_symbol = workers.remove + stop signal)
            let removed_from_orch = st.fleet.symbol_orchestrator.write()
                .map(|mut o| o.stop_symbol(symbol)).unwrap_or(false);

            // 2) live_price map'ten sil
            if let Ok(mut prices) = st.fleet.live_price.write() {
                prices.remove(symbol);
            }

            // 3) Live pozisyon varsa force-close. Gerçek emir gönderilmiyor
            //    (Binance Invalid symbol) — paper-equivalent: pozisyon kapatılır,
            //    PnL 0 (giriş fiyatından), ClosedTradeModel arşivlenir.
            let closed_pos = st.finance.live_positions.write()
                .ok().and_then(|mut map| map.remove(symbol));
            let mut force_close_msg = String::new();
            if let Some(pos) = closed_pos {
                let closed_trade = crate::core::model::ClosedTradeModel {
                    symbol: pos.symbol.clone(),
                    is_long: pos.is_long,
                    exit_reason: "DELISTED".into(),
                    pnl: 0.0,
                    pnl_pct: 0.0,
                    net_pnl: 0.0,
                    net_pnl_pct: 0.0,
                    commission: 0.0,
                    closed_at: chrono::Utc::now().to_rfc3339(),
                    opened_at: pos.opened_at.clone(),
                    leverage: pos.leverage,
                    entry_price: pos.entry_price,
                    exit_price: pos.entry_price, // delisted force-close: PnL 0 → çıkış=giriş
                };
                if let Ok(mut closed_list) = st.finance.live_closed_trades.write() {
                    closed_list.push(closed_trade);
                }
                st.finance.closed_trades_total.fetch_add(1, Ordering::Relaxed);
                force_close_msg = format!(
                    " · live pozisyon force-close (qty={:.4}, entry=${:.4}, PnL=0)",
                    pos.qty, pos.entry_price,
                );
            }

            // 4) Anomaly + log (operatöre görünür)
            if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                use crate::robot::data_pipeline::{AnomalyKind, AnomalySeverity};
                pipe.push_anomaly(
                    AnomalySeverity::Warning,
                    AnomalyKind::Custom,
                    format!("DELISTED: {} purge edildi ({} ardışık fetch hatası){}",
                        symbol, n_fail, force_close_msg),
                );
            }
            st.push_log(format!(
                "🚮 {} DELISTED → orchestrator removed={}{}",
                symbol, removed_from_orch, force_close_msg,
            ));
        }
        // 5) Snapshot'ları yenile (kalıcılık)
        Self::persist_open_positions_snapshot(state);
        Self::persist_account_state(state);
        // 6) Sayacı sıfırla — purge sonrası gereksiz yere tekrar tetiklenmesin.
        delisted_record_success(symbol);
    }

    /// En son trade gerçekleştiği epoch saniyesini AppState'e mühürler. Heartbeat
    /// snapshot bu değere bakarak "son N saniyede trade var → phase=Executing"
    /// sticky raporlaması yapar. Tek anlık phase ~500ms yaşadığı için 60sn
    /// heartbeat snapshot pencereisinde nadiren yakalanıyordu.
    pub fn mark_execution_epoch(state: &Arc<Mutex<AppState>>) {
        let now_secs = crate::core::time::now_epoch_secs();
        if let Ok(st) = state.lock() {
            st.fleet.last_execution_epoch.store(now_secs, Ordering::Relaxed);
        }
    }

    /// Equity, peak, starting capital ve toplam kapalı trade sayacını DB'ye
    /// mühürler. Pozisyon açılış/kapanış noktalarında çağrılır.
    /// Senkron — UI lock dışında çağrılmalı.
    pub fn persist_account_state(state: &Arc<Mutex<AppState>>) {
        let (db_path, equity, peak, starting, closed_count) = match state.lock() {
            Ok(st) => (
                st.config.db_path.clone(),
                st.finance.equity,
                st.finance.peak_equity,
                st.finance.starting_capital,
                st.finance.closed_trades_total.load(Ordering::Relaxed),
            ),
            Err(_) => return,
        };
        // Adım 5.5: yazım arka planda (detached) + serileştirilmiş → cycle bloklanmaz.
        spawn_db_write(move || {
            let _guard = DB_PERSIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            if let Ok(conn) = crate::persistence::open_db(&db_path) {
                let _ = crate::persistence::writer::save_account_state(
                    &conn, equity, peak, starting, closed_count,
                );
            }
        });
    }
}

#[cfg(test)]
mod persist_offload_tests {
    use super::spawn_db_write;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn runs_inline_without_runtime() {
        // Tokio runtime yokken (senkron bağlam) yazım inline çalışır (backward-compat).
        let flag = Arc::new(AtomicBool::new(false));
        let f = flag.clone();
        spawn_db_write(move || f.store(true, Ordering::SeqCst));
        assert!(flag.load(Ordering::SeqCst), "runtime yokken inline yazım hemen koşmalı");
    }

    #[tokio::test]
    async fn offloads_under_runtime() {
        // Runtime içindeyken spawn_blocking'e atılır (cycle bloklanmaz) ve nihayetinde koşar.
        let flag = Arc::new(AtomicBool::new(false));
        let f = flag.clone();
        spawn_db_write(move || f.store(true, Ordering::SeqCst));
        // Detached spawn_blocking → tamamlanması için kısa bekleme.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(flag.load(Ordering::SeqCst), "spawn_blocking arka planda koşmalı");
    }
}
