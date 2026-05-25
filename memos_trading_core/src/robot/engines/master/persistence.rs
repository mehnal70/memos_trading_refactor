// src/robot/engines/master/persistence.rs — DB şema/hidrasyon/persist + delisted purge + BIST filtresi
// Faz 1 modülerleştirme: master.rs'ten taşındı (davranış birebir korunur).
use super::*;

impl Engine {

    /// Boot'ta SQLite şemasını defensive yaratır. Cold-start'ta candle tablosu
    /// yoksa ML retrain trigger'ı her 500ms hata atıyordu. Her iki tablo da
    /// `CREATE IF NOT EXISTS` idempotent — mevcut DB'lere zarar vermez.
    pub(crate) fn ensure_db_schema(state: &Arc<Mutex<AppState>>) {
        let db_path = match state.lock() {
            Ok(st) => st.config.db_path.clone(),
            Err(_) => return,
        };
        let conn = match rusqlite::Connection::open(&db_path) {
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
        // open_positions_snapshot ayrıca save_open_positions_snapshot içinde
        // ilk INSERT öncesi yaratılıyor; ek bir CREATE çağrısına gerek yok.
        if let Err(e) = crate::persistence::writer::ensure_account_state_table(&conn) {
            log::warn!("⚠️ account_state tablosu kurulamadı: {}", e);
            push_state_log(state, format!("⚠️ account_state tablosu kurulamadı: {}", e));
        }
    }

    /// Boot sırasında önceki run'un `open_positions_snapshot` tablosundan
    /// açık pozisyonları okur ve `live_positions` HashMap'ine hidrate eder.
    /// - Tablo yoksa / kayıt yoksa: sessiz geçer (cold-start).
    /// - DB açılamazsa: hata log'una düşer ama engine devam eder.
    /// - Halihazırda live_positions'ta aynı sembol varsa: DB tarafı ezilir
    ///   (recovery sırasında live state boş olmalı; defensive).
    pub(crate) async fn hydrate_open_positions_from_db(state: &Arc<Mutex<AppState>>) {
        let (db_path, interval) = match state.lock() {
            Ok(st) => (st.config.db_path.clone(), st.config.interval.clone()),
            Err(_) => return,
        };
        // BIST exclude: default ON. BIST canlı feed pratik olarak yok → cycle'a
        // BIST koymak DataIngest/PriceFetch Failed → anomaly birikimi.
        // ALLOW_BIST=1 ile geri açılır (geçmiş data backtest senaryoları için).
        let allow_bist = env_truthy("ALLOW_BIST");
        match crate::persistence::reader::recover_open_positions(&db_path) {
            Ok(positions) if !positions.is_empty() => {
                // İki kademeli filtre:
                //   1) BIST heuristic (allow_bist=false ise BIST'leri atla).
                //   2) Candles existence — sembol+interval için en az 1 candle.
                // Atlananlar repair_log'a düşürülür; operatör görür.
                let mut loaded = Vec::new();
                let mut stale  = Vec::new();   // candles yok
                let mut bist   = Vec::new();   // BIST exclude
                for pos in positions {
                    if !allow_bist && Self::looks_like_bist_symbol(&pos.symbol) {
                        bist.push(pos);
                        continue;
                    }
                    let has_candles = crate::persistence::reader::read_candles(
                        &db_path, &pos.symbol, &interval, 1,
                    ).map(|v| !v.is_empty()).unwrap_or(false);
                    if has_candles { loaded.push(pos); }
                    else            { stale.push(pos); }
                }
                let n_loaded = loaded.len();
                let n_stale  = stale.len();
                let n_bist   = bist.len();
                let stale_syms: Vec<String> = stale.iter().map(|p| p.symbol.clone()).collect();
                let bist_syms:  Vec<String> = bist.iter().map(|p| p.symbol.clone()).collect();

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
                    if n_bist > 0 {
                        msg.push_str(&format!(
                            " · {} BIST-excluded (canlı feed yok; ALLOW_BIST=1 ile aç): {}",
                            n_bist, bist_syms.join(","),
                        ));
                    }
                    st.push_log_mirror(msg);
                    if n_stale > 0 || n_bist > 0 {
                        st.guardian.repair_log.push_back(format!(
                            "[{}] recovery: yüklendi={} stale={} bist={}",
                            chrono::Local::now().format("%H:%M:%S"),
                            n_loaded, n_stale, n_bist,
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
    pub fn looks_like_bist_symbol(sym: &str) -> bool {
        if sym.len() < 3 || sym.len() > 6 { return false; }
        if !sym.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
            return false;
        }
        const CRYPTO_QUOTES: &[&str] = &["USDT", "USDC", "BUSD", "FDUSD", "TUSD", "DAI"];
        for q in CRYPTO_QUOTES {
            if sym.ends_with(q) { return false; }
        }
        true
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
        let conn = match rusqlite::Connection::open(&db_path) {
            Ok(c) => c,
            Err(_) => return, // sessiz fail; bir sonraki snapshot dener
        };
        let _ = crate::persistence::writer::save_open_positions_snapshot(
            &conn, &positions,
        );
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

    /// Delisted sembolü orchestrator + live_positions'tan temizler. Açık pozisyon
    /// varsa ClosedTradeModel'a PnL=0/reason=DELISTED ile push edilir (gerçek
    /// kapanış mümkün değil — Binance Invalid symbol döndürüyor).
    /// Idempotent: tekrar çağırma güvenli (orchestrator/live_positions yoksa no-op).
    pub fn purge_delisted_symbol(state: &Arc<Mutex<AppState>>, symbol: &str, n_fail: u32) {
        log::warn!(
            "🚮 Delisted detection: {} sembolü {} ardışık fetch hatasından sonra purge ediliyor",
            symbol, n_fail,
        );
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
                    closed_at: chrono::Utc::now().to_rfc3339(),
                    opened_at: pos.opened_at.clone(),
                    leverage: pos.leverage,
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
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
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
        let conn = match rusqlite::Connection::open(&db_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let _ = crate::persistence::writer::save_account_state(
            &conn, equity, peak, starting, closed_count,
        );
    }
}
