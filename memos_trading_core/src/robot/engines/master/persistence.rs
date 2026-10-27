// src/robot/engines/master/persistence.rs — DB şema/hidrasyon/persist + delisted purge + BIST filtresi
// Faz 1 modülerleştirme: master.rs'ten taşındı (davranış birebir korunur).
use super::*;

/// Restart reconciliation planı (borsa OTORİTE): boot'ta DB-hidre local pozisyonların
/// borsadaki gerçek durumla farkı. [[project_persistence_restart]]
/// Borsa tarafındaki tek pozisyonun mutabakat görünümü.
///
/// `sl`/`tp` `Option`: Binance `positionRisk` bu alanları HİÇ döndürmez (koruma ayrı
/// emirlerdedir) → `None` = "bilgi yok", `Some(0.0)` = "borsa koruma YOK diyor"
/// (MT5 pozisyon-üstü SL/TP). İkisini karıştırmak her Binance pozisyonunu korumasız
/// diye yanlış alarmlatırdı. `leverage` 0 = bilinmiyor.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct ExchangePos {
    pub amt: f64,
    pub entry: f64,
    pub leverage: f64,
    pub sl: Option<f64>,
    pub tp: Option<f64>,
}

/// JSON alanını sayı YA DA string olarak oku. Binance sayısalları string döndürür
/// ("0.001"), MT5 EA ham sayı basar (%.8f) — eski kod yalnız string bekleyip MT5
/// entry'sini 0 okuyordu.
pub(crate) fn json_f64(v: Option<&serde_json::Value>) -> Option<f64> {
    match v {
        Some(serde_json::Value::Number(n)) => n.as_f64(),
        Some(serde_json::Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct ReconcilePlan {
    /// Local'de var, borsada flat → phantom, kaldırılmalı.
    pub stale: Vec<String>,
    /// İkisinde de var ama yön/qty uyuşmuyor → borsaya senkronla (borsa otorite).
    pub mismatched: Vec<(String, ExchangePos)>,
    /// Borsada var, local'de yok → bilinmeyen, operatöre alert (oto-adopt edilmez).
    pub unknown: Vec<String>,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct CanonicalReconcilePlan {
    pub stale: Vec<memos_venue_api::CanonicalPositionKey>,
    pub mismatched: Vec<(memos_venue_api::CanonicalPositionKey, ExchangePos)>,
    pub unknown: Vec<memos_venue_api::CanonicalPositionKey>,
}

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
        Ok(handle) => {
            handle.spawn_blocking(f);
        }
        Err(_) => f(),
    }
}

impl Engine {
    pub(crate) fn hydrate_uncertain_orders_from_db(state: &Arc<Mutex<AppState>>) {
        let (db_path, profile) = match state.lock() {
            Ok(st) => (
                st.config.state_db_path.clone(),
                st.config.venue_profile.as_str().to_string(),
            ),
            Err(_) => return,
        };
        match crate::persistence::reader::load_uncertain_orders_for_profile(&db_path, &profile) {
            Ok(orders) => {
                let count = orders.len();
                if let Ok(st) = state.lock() {
                    if let Ok(mut uncertain) = st.finance.uncertain_orders.write() {
                        uncertain.clear();
                        for order in orders {
                            uncertain.insert(order.symbol.clone(), order);
                        }
                    }
                }
                if count > 0 {
                    push_state_log(
                        state,
                        format!(
                            "⚠️ {} çözülmemiş belirsiz emir recovery edildi; canlı girişler reconciliation/operator çözümüne kadar kapalı",
                            count
                        ),
                    );
                }
            }
            Err(error) => push_state_log(
                state,
                format!(
                    "⚠️ uncertain_orders recovery okunamadı ({}): {}",
                    db_path, error
                ),
            ),
        }
    }

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
                push_state_log(
                    state,
                    format!("⚠️ ensure_db_schema: DB açılamadı ({}) — devam ediliyor", e),
                );
                return;
            }
        };
        if let Err(e) = crate::persistence::writer::ensure_candles_table(&conn) {
            log::warn!("⚠️ candles tablosu kurulamadı: {}", e);
            push_state_log(state, format!("⚠️ candles tablosu kurulamadı: {}", e));
        } else if let Ok(mut st) = state.lock() {
            st.push_log_mirror(
                "📐 SQLite şeması doğrulandı (candles + open_positions_snapshot + account_state)"
                    .to_string(),
            );
        }
        if let Err(e) = crate::persistence::writer::migrate_candle_schema(&conn) {
            log::warn!("⚠️ candle şema migration başarısız: {}", e);
            push_state_log(state, format!("⚠️ candle şema migration başarısız: {}", e));
        }
        if let Err(e) = crate::persistence::writer::ensure_account_state_table(&conn) {
            log::warn!("⚠️ account_state tablosu kurulamadı: {}", e);
            push_state_log(state, format!("⚠️ account_state tablosu kurulamadı: {}", e));
        }
        if let Err(e) = crate::persistence::writer::ensure_closed_trades_snapshot_table(&conn) {
            log::warn!("⚠️ closed_trades_snapshot tablosu kurulamadı: {}", e);
            push_state_log(
                state,
                format!("⚠️ closed_trades_snapshot tablosu kurulamadı: {}", e),
            );
        }
        if let Err(e) = crate::persistence::writer::ensure_uncertain_orders_table(&conn) {
            log::warn!("⚠️ uncertain_orders tablosu kurulamadı: {}", e);
            push_state_log(
                state,
                format!("⚠️ uncertain_orders tablosu kurulamadı: {}", e),
            );
        }
        if let Err(e) = crate::persistence::writer::ensure_pending_closes_table(&conn) {
            log::warn!("⚠️ pending_closes tablosu kurulamadı: {}", e);
            push_state_log(
                state,
                format!("⚠️ pending_closes tablosu kurulamadı: {}", e),
            );
        }
    }

    pub(crate) fn hydrate_pending_closes_from_db(state: &Arc<Mutex<AppState>>) {
        let (db_path, profile) = match state.lock() {
            Ok(st) => (
                st.config.state_db_path.clone(),
                st.config.venue_profile.as_str().to_string(),
            ),
            Err(_) => return,
        };
        match crate::persistence::reader::load_pending_closes_for_profile(&db_path, &profile) {
            Ok(pending) => {
                let count = pending.len();
                if let Ok(st) = state.lock() {
                    if let Ok(mut map) = st.finance.pending_closes.write() {
                        map.clear();
                        map.extend(pending);
                    }
                }
                if count > 0 {
                    push_state_log(
                        state,
                        format!("🔄 {} bekleyen kapanış emri recovery edildi", count),
                    );
                }
            }

            Err(error) => push_state_log(
                state,
                format!(
                    "⚠️ pending_closes recovery okunamadı ({}): {}",
                    db_path, error
                ),
            ),
        }
    }

    pub(crate) async fn persist_pending_close(
        state: &Arc<Mutex<AppState>>,
        key: memos_venue_api::CanonicalPositionKey,
        broker_order_id: String,
    ) {
        let (db_path, profile) = match state.lock() {
            Ok(st) => (
                st.config.state_db_path.clone(),
                st.config.venue_profile.as_str().to_string(),
            ),
            Err(_) => return,
        };
        let result = tokio::task::spawn_blocking(move || {
            let conn = crate::persistence::open_db(&db_path).map_err(|e| e.to_string())?;
            crate::persistence::writer::save_pending_close_for_profile(
                &conn,
                &profile,
                &key,
                &broker_order_id,
            )
            .map_err(|e| e.to_string())
        })
        .await;
        match result {
            Err(error) => log::warn!("⚠️ pending close kalıcı yazılamadı: {}", error),
            Ok(Err(error)) => log::warn!("⚠️ pending close kalıcı yazılamadı: {}", error),
            Ok(Ok(())) => {}
        }
    }

    pub(crate) async fn remove_persisted_pending_close(
        state: &Arc<Mutex<AppState>>,
        key: memos_venue_api::CanonicalPositionKey,
    ) {
        let (db_path, profile) = match state.lock() {
            Ok(st) => (
                st.config.state_db_path.clone(),
                st.config.venue_profile.as_str().to_string(),
            ),
            Err(_) => return,
        };
        let result = tokio::task::spawn_blocking(move || {
            let conn = crate::persistence::open_db(&db_path).map_err(|e| e.to_string())?;
            crate::persistence::writer::remove_pending_close_for_profile(&conn, &profile, &key)
                .map_err(|e| e.to_string())
        })
        .await;
        match result {
            Err(error) => log::warn!("⚠️ pending close kalıcı kaydı silinemedi: {}", error),
            Ok(Err(error)) => log::warn!("⚠️ pending close kalıcı kaydı silinemedi: {}", error),
            Ok(Ok(())) => {}
        }
    }

    pub(crate) fn prune_seed_ineligible(state: &Arc<Mutex<AppState>>) {
        let dropped: Vec<String> = {
            let Ok(st) = state.lock() else { return };
            let Ok(mut params) = st.brain.parameters.write() else {
                return;
            };
            if params.symbol_strategy.is_empty() {
                return;
            }
            let drop: Vec<String> = params
                .symbol_strategy
                .keys()
                .filter(|s| super::is_delisted_skipped(s) || !super::is_symbol_tradeable(s))
                .cloned()
                .collect();
            for s in &drop {
                params.symbol_strategy.remove(s);
                params.symbol_interval.remove(s);
            }
            drop
        };
        if dropped.is_empty() {
            return;
        }
        let mut sorted = dropped;
        sorted.sort();
        push_state_log(state, format!(
            "🚮 edge seed: {} sembol registry-dışlandı (delisted/non-TRADING) → seed'den atıldı: {}",
            sorted.len(), sorted.join(", ")));
        log::info!(
            "🚮 edge seed registry-prune: {} atıldı — {}",
            sorted.len(),
            sorted.join(", ")
        );
    }
    pub(crate) fn report_edge_seed(state: &Arc<Mutex<AppState>>) {
        let seed_path = std::env::var("EDGE_SEED_REPORT")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let mut entries: Vec<(String, String, String)> = match state.lock() {
            Ok(st) => st
                .brain
                .parameters
                .read()
                .ok()
                .map(|p| {
                    p.symbol_strategy
                        .iter()
                        .map(|(sym, strat)| {
                            (
                                sym.clone(),
                                p.symbol_interval
                                    .get(sym)
                                    .cloned()
                                    .unwrap_or_else(|| "?".into()),
                                strat.clone(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Err(_) => return,
        };
        entries.sort();
        if !entries.is_empty() {
            let mut preview: Vec<String> = entries
                .iter()
                .take(8)
                .map(|(s, _iv, st)| format!("{s}→{st}"))
                .collect();
            if entries.len() > 8 {
                preview.push(format!("+{} daha", entries.len() - 8));
            }
            push_state_log(
                state,
                format!(
                    "🌱 edge seed: {} sembol→strateji yüklendi ({})",
                    entries.len(),
                    preview.join(", ")
                ),
            );
            let full: Vec<String> = entries
                .iter()
                .map(|(s, iv, st)| format!("{s} {iv}/{st}"))
                .collect();
            log::info!(
                "🌱 edge seed: {} aday yüklendi — {}",
                entries.len(),
                full.join(", ")
            );
        } else if let Some(path) = seed_path {
            push_state_log(state, format!(
                "🌱 edge seed: '{}' okundu ama 0 WF-onaylı aday → symbol_strategy boş (global/auto sürer)", path));
            log::info!(
                "🌱 edge seed: '{}' okundu ama 0 WF-onaylı aday → symbol_strategy boş",
                path
            );
        }
    }

    pub(crate) fn hydrate_open_positions_from_db(state: &Arc<Mutex<AppState>>) {
        let (db_path, state_db_path, interval, profile, tuning) = match state.lock() {
            Ok(st) => (
                st.config.db_path.clone(),
                st.config.state_db_path.clone(),
                st.config.interval.clone(),
                st.config.venue_profile.as_str().to_string(),
                Arc::clone(&st.tuning),
            ),
            Err(_) => return,
        };
        match crate::persistence::reader::recover_open_positions_for_profile(
            &state_db_path,
            &profile,
        ) {
            Ok(mut positions) if !positions.is_empty() => {
                let before_filter = positions.len();
                positions.retain(|position| {
                    position.qty.is_finite()
                        && position.qty > f64::EPSILON
                        && position.entry_price.is_finite()
                        && position.entry_price > f64::EPSILON
                });
                let invalid_snapshot_count = before_filter.saturating_sub(positions.len());
                if invalid_snapshot_count > 0 {
                    push_state_log(
                        state,
                        format!(
                            "🧹 [RECOVERY] geçersiz sıfır pozisyon snapshot'ı atlandı: {}",
                            invalid_snapshot_count
                        ),
                    );
                }
                if positions.is_empty() {
                    if let Ok(mut st) = state.lock() {
                        st.push_log_mirror(
                            "♻️ [RECOVERY] yalnızca geçersiz pozisyon snapshot'ı bulundu — cold-start"
                                .to_string(),
                        );
                    }
                    Self::persist_open_positions_snapshot(state);
                    return;
                }
                let mut loaded = Vec::new();
                let mut stale = Vec::new();
                let mut no_feed = Vec::new();
                for pos in positions {
                    if !tuning.symbol_eligible_for_live(&pos.symbol) {
                        no_feed.push(pos);
                        continue;
                    }
                    let has_candles = crate::persistence::reader::read_candles(
                        &db_path,
                        &pos.symbol,
                        &interval,
                        1,
                    )
                    .map(|v| !v.is_empty())
                    .unwrap_or(false);
                    if has_candles {
                        loaded.push(pos);
                    } else {
                        stale.push(pos);
                    }
                }
                let n_loaded = loaded.len();
                let n_stale = stale.len();
                let n_no_feed = no_feed.len();
                let stale_syms: Vec<String> = stale.iter().map(|p| p.symbol.clone()).collect();
                let no_feed_syms: Vec<String> = no_feed.iter().map(|p| p.symbol.clone()).collect();

                let market_for_orch = {
                    let st = match state.lock() {
                        Ok(s) => s,
                        Err(_) => return,
                    };
                    st.config.market.clone()
                };
                let loaded_syms: Vec<String> = loaded.iter().map(|p| p.symbol.clone()).collect();
                if let Ok(st) = state.lock() {
                    if let Ok(mut map) = st.finance.live_positions.write() {
                        for pos in loaded {
                            map.insert(pos.symbol.clone(), pos);
                        }
                    }
                    st.refresh_canonical_positions();
                }
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
                            n_stale,
                            interval,
                            stale_syms.join(","),
                        ));
                    }
                    if n_no_feed > 0 {
                        msg.push_str(&format!(
                            " · {} feed'siz borsa-excluded (FORCE_LIVE_EXCHANGES ile aç): {}",
                            n_no_feed,
                            no_feed_syms.join(","),
                        ));
                    }
                    st.push_log_mirror(msg);
                    if n_stale > 0 || n_no_feed > 0 {
                        st.guardian.repair_log.push_back(format!(
                            "[{}] recovery: yüklendi={} stale={} no_feed={}",
                            chrono::Local::now().format("%H:%M:%S"),
                            n_loaded,
                            n_stale,
                            n_no_feed,
                        ));
                        while st.guardian.repair_log.len() > 100 {
                            st.guardian.repair_log.pop_front();
                        }
                    }
                }
            }
            Ok(_) => {
                if let Ok(mut st) = state.lock() {
                    st.push_log_mirror("♻️ [RECOVERY] DB snapshot boş — cold-start".to_string());
                }
            }
            Err(e) => {
                log::warn!(
                    "⚠️ [RECOVERY] snapshot okunamadı: {} — cold-start'a düşülüyor",
                    e
                );
                push_state_log(
                    state,
                    format!(
                    "⚠️ [RECOVERY] open_positions_snapshot okunamadı: {} (cold-start'a düşülüyor)",
                    e,
                ),
                );
            }
        }
    }

    /* pub(crate) async fn reconcile_live_positions_with_exchange(state: &Arc<Mutex<AppState>>) {
        let (executor, dry_run, primary_exchange) = match state.lock() {
            Ok(st) => (
                st.live_executor.clone(),
                st.live_dry_run,
                st.config.exchange.to_ascii_lowercase(),
            ),
            Err(_) => return,
        };
        let executor = match executor {
            Some(e) if !dry_run => e,
            _ => return,
        };
        if executor.is_spot {
            push_state_log(state, "♻️ [RECONCILE] spot — futures-only reconcile atlandı".to_string());
            return;
        }
        let ex_positions = match executor.get_all_positions().await {
            Ok(p) => p,
            Err(e) => {
                push_state_log(state, format!(
                    "⚠️ [RECONCILE] borsa pozisyon sorgusu başarısız: {:?} — local korunuyor (silme YOK)", e));
                return;
            }
        };
        let exchange: std::collections::HashMap<String, (f64, f64)> = ex_positions.iter().filter_map(|p| {
            let sym = p.get("symbol")?.as_str()?.to_string();
            let amt = p.get("positionAmt")?.as_str()?.parse::<f64>().ok()?;
            let entry = p.get("entryPrice").and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
            Some((sym, (amt, entry)))
        }).collect();

        let local: Vec<(String, bool, f64)> = match state.lock() {
            Ok(st) => st.finance.live_positions.read().ok()
                .map(|m| m.values().map(|p| (p.symbol.clone(), p.is_long, p.qty)).collect())
                .unwrap_or_default(),
            Err(_) => return,
        };

        // Borsa adaptörü açılış anında geçici olarak boş pozisyon döndürebilir.
        // Bu pencerede local state'i phantom sayıp silmek yerine korumayı tercih et.
        // Varsayılan koruma: yalnız mt5. Env ile genişletilebilir:
        // RECONCILE_EMPTY_PROTECT_EXCHANGES=mt5,binance,ibkr
        let protect_empty_for: std::collections::HashSet<String> = std::env::var("RECONCILE_EMPTY_PROTECT_EXCHANGES")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(|s| s.trim().to_ascii_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_else(|| {
                let mut s = std::collections::HashSet::new();
                s.insert("mt5".to_string());
                s.insert("ibkr".to_string());
                s
            });
        if protect_empty_for.contains(&primary_exchange) && !local.is_empty() && exchange.is_empty() {
            push_state_log(
                state,
                format!(
                    "⚠️ [RECONCILE] {} boş pozisyon snapshot döndürdü; local korunuyor ({} pozisyon)",
                    primary_exchange,
                    local.len(),
                ),
            );
            return;
        }

        let plan = Self::reconcile_diff(&local, &exchange);
        if plan.stale.is_empty() && plan.mismatched.is_empty() && plan.unknown.is_empty() {
            push_state_log(state, format!("♻️ [RECONCILE] borsa ile uyumlu ({} pozisyon)", local.len()));
            return;
        }

        if let Ok(mut st) = state.lock() {
            if let Ok(mut map) = st.finance.live_positions.write() {
                for sym in &plan.stale { map.remove(sym); }
                for (sym, ex_long, ex_qty, ex_entry) in &plan.mismatched {
                    if let Some(p) = map.get_mut(sym) {
                        p.is_long = *ex_long;
                        p.qty = *ex_qty;
                        if *ex_entry > 0.0 { p.entry_price = *ex_entry; }
                    }
                }
            }
        }
        let summary = format!(
            "♻️ [RECONCILE] borsa-otorite → {} phantom kaldırıldı [{}] · {} senkronlandı [{}] · {} bilinmeyen [{}]",
            plan.stale.len(), plan.stale.join(","),
            plan.mismatched.len(), plan.mismatched.iter().map(|m| m.0.as_str()).collect::<Vec<_>>().join(","),
            plan.unknown.len(), plan.unknown.join(","),
        );
        if let Ok(mut st) = state.lock() {
            st.push_log_mirror(summary.clone());
            st.guardian.repair_log.push_back(format!(
                "[{}] reconcile: phantom={} sync={} unknown={}",
                chrono::Local::now().format("%H:%M:%S"),
                plan.stale.len(), plan.mismatched.len(), plan.unknown.len(),
            ));
            while st.guardian.repair_log.len() > 100 { st.guardian.repair_log.pop_front(); }
            if !plan.unknown.is_empty() {
                if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                    use crate::robot::data_pipeline::{AnomalyKind, AnomalySeverity};
                    pipe.push_anomaly(
                        AnomalySeverity::Warning, AnomalyKind::Custom,
                        format!("reconcile: borsada bot-dışı pozisyon ({}) — manuel kontrol", plan.unknown.join(",")),
                    );
                }
            }
            st.push_alert(
                "RECONCILE-DRIFT",
                crate::robot::infra::telegram_notifier::Severity::Warning,
                summary,
            );
        }
        Self::persist_open_positions_snapshot(state);
    } */

    /// 🛡️ OTONOM ÇOKLU BORSA POZİSYON MUTABAKAT MOTORU
    /// Binance, MT5 ve IBKR borsalarında taşınan aktif canlı pozisyonları dinamik gezer,
    /// borsa otoritesini baz alarak local hafızayı (`st.finance.live_positions`) saniyeler içinde
    /// senkronize eder. Görseldeki MT5 usdjpy pozisyon kilitlenmelerini kalıcı olarak çözer.
    /// 🛡️ OTONOM ÇOKLU BORSA POZİSYON MUTABAKAT VE TOPLU KEŞİF MOTORU
    /// GEREKÇE: Listede (`pinned_symbols`) bulunmayan ama borsada (MT5/IBKR) anlık açılmış olan
    /// tüm yabancı hareketleri (`USDJPY@mt5` gibi) canlı döngü esnasında otonom keşfeder (Exchange Sweeper Policy),
    /// anında Canlı İşçi Hattını (`SymbolOrchestrator`) spawn eder ve TUI snapshot paneline akıtır.
    pub async fn reconcile_live_positions_with_exchange(state: &Arc<Mutex<AppState>>) {
        let (venue_registry, current_market, current_interval, global_dry_run) = {
            let Ok(st) = state.lock() else {
                return;
            };
            (
                Arc::clone(&st.venue_registry),
                st.config.market.clone(),
                st.config.interval.clone(),
                st.live_dry_run,
            )
        };

        if venue_registry.is_empty() {
            return;
        }
        // Paper/Demo profillerinde borsa otoritesi yoktur; boş exchange cevabı
        // local simülasyon pozisyonlarını "stale" diye silemez.
        if global_dry_run {
            if let Ok(st) = state.lock() {
                if let Ok(mut runtimes) = st.venue_runtime.write() {
                    for spec in venue_registry.specs() {
                        let runtime_key = format!(
                            "{}:{}",
                            spec.exchange.as_str().to_ascii_lowercase(),
                            spec.market.as_str().to_ascii_lowercase()
                        );
                        runtimes
                            .entry(runtime_key)
                            .or_default()
                            .reconciliation_status = "NOT_APPLICABLE".to_string();
                    }
                }
            }
            return;
        }

        let mut borsa_toplam_pozisyonlari: std::collections::HashMap<String, ExchangePos> =
            std::collections::HashMap::new();
        let mut canonical_exchange_positions: std::collections::HashMap<
            memos_venue_api::CanonicalPositionKey,
            ExchangePos,
        > = std::collections::HashMap::new();
        let mut symbol_venues: std::collections::HashMap<
            String,
            std::collections::HashSet<memos_venue_api::VenueKey>,
        > = std::collections::HashMap::new();
        let mut yeni_kesfedilen_semboller: Vec<String> = Vec::new();
        let mut failed_exchanges = std::collections::HashSet::new();
        let reconcile_timeout_secs =
            crate::core::env::env_parse_or("RECONCILE_VENUE_TIMEOUT_SECS", 5u64).max(1);

        // 1. Adım: Tüm aktif borsaları pazar-agnostik kurallarla bulk (toptan) tarıyoruz
        for spec in venue_registry.specs() {
            let Some(venue) = venue_registry.get_spec(spec) else {
                continue;
            };
            let current_exchange = venue.exchange();

            if global_dry_run && current_exchange == crate::core::types::Exchange::Binance {
                continue;
            }

            // ⚠️ DÜZELTME: eskiden `fetch_open_orders()` çağrılıyordu (= /fapi/v1/openOrders,
            // EMİR şeması) ama yanıt `positionAmt`/`entryPrice` ile parse ediliyordu — bunlar
            // /fapi/v2/positionRisk (= `fetch_account_positions()`) alanları. Alanlar hiç
            // bulunamadığı için borsa haritası HER ZAMAN boş kalıyor, dolayısıyla TÜM local
            // pozisyonlar `stale` sayılıp hafizadan siliniyordu → borsadaki pozisyon açık ve
            // YÖNETİLMEZ kalıyordu ([CLOSE-NO-POS] yetim logları bunun semptomuydu).
            match tokio::time::timeout(
                std::time::Duration::from_secs(reconcile_timeout_secs),
                venue.fetch_account_positions(),
            )
            .await
            {
                Ok(Ok(positions_resp)) => {
                    for p in positions_resp {
                        if let (Some(sym), Some(amt)) = (
                            p.get("symbol")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            json_f64(p.get("positionAmt")),
                        ) {
                            // Binance positionRisk aktif olmayan sözleşmeleri de
                            // positionAmt=0 ile döndürebilir; bunlar pozisyon değildir.
                            if !amt.is_finite() || amt.abs() <= f64::EPSILON {
                                continue;
                            }
                            let entry = json_f64(p.get("entryPrice")).unwrap_or(0.0);
                            let ex_pos = ExchangePos {
                                amt,
                                entry,
                                leverage: json_f64(p.get("leverage")).unwrap_or(0.0),
                                // Alan JSON'da yoksa None (bilgi yok) — Some(0.0)'a düşürme!
                                sl: p.get("sl").and_then(|v| json_f64(Some(v))),
                                tp: p.get("tp").and_then(|v| json_f64(Some(v))),
                            };

                            // 🎯 2. YÖNTEM ÇEKİRDEĞİ: OTONOM BORSA TARAYICI (EXCHANGE SWEEPER)
                            // Borsadan gelen etiketli sembol (Örn: "USDJPY@mt5") bizim local listemizde var mı?
                            let lists_contain = {
                                let Ok(st) = state.lock() else {
                                    continue;
                                };
                                st.config.pinned_symbols.contains(&sym)
                            };

                            // Eğer local listede yoksa bu yabancı sembolü otonom dinamik hasat listesine alıyoruz.
                            if !lists_contain && !yeni_kesfedilen_semboller.contains(&sym) {
                                yeni_kesfedilen_semboller.push(sym.clone());
                            }

                            let venue_key = venue.venue_key();
                            symbol_venues
                                .entry(sym.clone())
                                .or_default()
                                .insert(venue_key);
                            canonical_exchange_positions.insert(
                                memos_venue_api::CanonicalPositionKey::new(venue_key, sym.clone()),
                                ex_pos.clone(),
                            );
                            let _ = borsa_toplam_pozisyonlari.insert(sym, ex_pos);
                        }
                    }
                }
                Ok(Err(error)) => {
                    failed_exchanges.insert(current_exchange);
                    push_state_log(
                        state,
                        format!(
                            "⚠️ [RECONCILE] {:?} account positions okunamadı; local pozisyonlar korunuyor: {}",
                            current_exchange, error
                        ),
                    );
                }
                Err(_) => {
                    failed_exchanges.insert(current_exchange);
                    push_state_log(
                        state,
                        format!(
                            "⚠️ [RECONCILE] {:?} account positions zaman aşımına uğradı ({}s); local pozisyonlar korunuyor",
                            current_exchange, reconcile_timeout_secs
                        ),
                    );
                }
            }
        }

        // Önceki sürümlerde sıfır miktarlı positionRisk satırları local'e adopt
        // edilebildi. Bunları stale/closed trade olarak saymadan güvenle temizle.
        if let Ok(mut st) = state.lock() {
            let removed_invalid = if let Ok(mut positions) = st.finance.live_positions.write() {
                let invalid: Vec<String> = positions
                    .iter()
                    .filter(|(_, position)| {
                        !position.qty.is_finite()
                            || position.qty <= f64::EPSILON
                            || !position.entry_price.is_finite()
                            || position.entry_price <= f64::EPSILON
                    })
                    .map(|(symbol, _)| symbol.clone())
                    .collect();
                for symbol in &invalid {
                    positions.remove(symbol);
                }
                invalid.len()
            } else {
                0
            };
            if removed_invalid > 0 {
                st.push_log(format!(
                    "🧹 [RECONCILE] Geçersiz sıfır pozisyonlar temizlendi: {}",
                    removed_invalid
                ));
            }
        }

        let (pending_close_symbols, settled_pending_closes) = {
            let Ok(st) = state.lock() else {
                return;
            };
            let Ok(mut pending) = st.finance.pending_closes.write() else {
                return;
            };
            let settled: Vec<memos_venue_api::CanonicalPositionKey> = pending
                .keys()
                .filter(|key| !borsa_toplam_pozisyonlari.contains_key(&key.symbol))
                .cloned()
                .collect();
            for key in &settled {
                pending.remove(key);
            }
            let pending_symbols = pending
                .keys()
                .map(|key| key.symbol.clone())
                .collect::<std::collections::HashSet<_>>();
            (pending_symbols, settled)
        };
        for key in settled_pending_closes {
            Self::remove_persisted_pending_close(state, key).await;
        }
        let ambiguous_symbols: std::collections::HashSet<String> = symbol_venues
            .into_iter()
            .filter_map(|(symbol, venues)| (venues.len() > 1).then_some(symbol))
            .collect();
        if !ambiguous_symbols.is_empty() {
            push_state_log(
                state,
                format!(
                    "⚠️ [RECONCILE] {} canonical pozisyon içinden aynı sembol birden fazla venue'da bulundu; sembol-tabanlı mutation atlandı: {}",
                    canonical_exchange_positions.len(),
                    ambiguous_symbols.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            );
        }
        yeni_kesfedilen_semboller.retain(|symbol| {
            !pending_close_symbols.contains(symbol) && !ambiguous_symbols.contains(symbol)
        });

        // 🚀 2. Adım: KEŞFEDİLEN YABANCI SEMBOLLERİN CANLI DÖNGÜDE EVLAT EDİNİLMESİ (DYNAMIC SPAWN)
        if !yeni_kesfedilen_semboller.is_empty() {
            let Ok(mut st) = state.lock() else {
                return;
            };
            for sym in &yeni_kesfedilen_semboller {
                if !crate::robot::config_engine::OtonomConfigEngine::symbol_allowed(sym) {
                    continue;
                }
                if !st.config.pinned_symbols.contains(sym) {
                    st.config.pinned_symbols.push(sym.clone());

                    // Canlı döngü koştururken otonom olarak Canlı İşçi Hattını (Worker Thread) anında spawn ediyoruz!
                    if let Ok(mut orch) = st.fleet.symbol_orchestrator.write() {
                        let _ = orch.register(sym, &current_market, &current_interval);
                    }
                }
            }
            st.push_log_mirror(format!(
                "🦅 [DYNAMIC-ADOPT] Listede olmayan aktif borsa pariteleri otonom keşfedildi ve işçi hatları canlı döngüde spawn edildi: {}",
                yeni_kesfedilen_semboller.join(", ")
            ));
        }

        // 3. Adım: Local Hafızadaki Mevcut Pozisyon Durumunu Çek
        let local: Vec<(String, bool, f64)> = match state.lock() {
            Ok(st) => {
                let mut local = st
                    .finance
                    .canonical_positions
                    .read()
                    .ok()
                    .map(|positions| {
                        positions
                            .values()
                            .filter(|p| {
                                venue_registry
                                    .for_symbol(&p.symbol)
                                    .map(|venue| !failed_exchanges.contains(&venue.exchange()))
                                    .unwrap_or(true)
                            })
                            .map(|p| (p.symbol.clone(), p.is_long, p.qty))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                // Eski snapshot kayıtları venue taşımayabilir; bunlar canonical
                // kitaba giremediği için legacy map'ten geçici olarak korunur.
                if let Ok(legacy) = st.finance.live_positions.read() {
                    local.extend(
                        legacy
                            .values()
                            .filter(|p| p.venue.is_none())
                            .filter(|p| {
                                venue_registry
                                    .for_symbol(&p.symbol)
                                    .map(|venue| !failed_exchanges.contains(&venue.exchange()))
                                    .unwrap_or(true)
                            })
                            .map(|p| (p.symbol.clone(), p.is_long, p.qty)),
                    );
                }
                local
            }
            Err(_) => return,
        };

        // 4. Adım: Delta Fark Kontrolü ve Planlama
        let mut plan = Self::reconcile_diff(&local, &borsa_toplam_pozisyonlari);
        plan.stale
            .retain(|symbol| !ambiguous_symbols.contains(symbol));
        plan.mismatched
            .retain(|(symbol, _)| !ambiguous_symbols.contains(symbol));
        plan.unknown.retain(|symbol| {
            !pending_close_symbols.contains(symbol) && !ambiguous_symbols.contains(symbol)
        });
        let canonical_local = match state.lock() {
            Ok(st) => st
                .finance
                .canonical_positions
                .read()
                .ok()
                .map(|positions| {
                    positions
                        .iter()
                        .filter(|(key, _)| !failed_exchanges.contains(&key.venue.exchange))
                        .map(|(key, position)| (key.clone(), position.is_long, position.qty))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            Err(_) => return,
        };
        let mut canonical_plan =
            Self::reconcile_canonical_diff(&canonical_local, &canonical_exchange_positions);
        Self::resolve_uncertain_orders_with_exchange_positions(
            state,
            &canonical_exchange_positions,
            &borsa_toplam_pozisyonlari,
            failed_exchanges.is_empty(),
        );
        canonical_plan.stale.retain(|key| {
            !pending_close_symbols.contains(&key.symbol) && !ambiguous_symbols.contains(&key.symbol)
        });
        canonical_plan.mismatched.retain(|(key, _)| {
            !pending_close_symbols.contains(&key.symbol) && !ambiguous_symbols.contains(&key.symbol)
        });
        canonical_plan.unknown.retain(|key| {
            !pending_close_symbols.contains(&key.symbol) && !ambiguous_symbols.contains(&key.symbol)
        });
        if let Ok(st) = state.lock() {
            if let Ok(mut runtimes) = st.venue_runtime.write() {
                for spec in venue_registry.specs() {
                    let exchange_name = format!(
                        "{}:{}",
                        spec.exchange.as_str().to_ascii_lowercase(),
                        spec.market.as_str().to_ascii_lowercase()
                    );
                    let status = if failed_exchanges.contains(&spec.exchange) {
                        "UNKNOWN"
                    } else {
                        let has_drift = canonical_plan.stale.iter().any(|key| {
                            key.venue.exchange == spec.exchange && key.venue.market == spec.market
                        }) || canonical_plan.mismatched.iter().any(|(key, _)| {
                            key.venue.exchange == spec.exchange && key.venue.market == spec.market
                        }) || canonical_plan.unknown.iter().any(|key| {
                            key.venue.exchange == spec.exchange && key.venue.market == spec.market
                        });
                        if has_drift {
                            "DRIFT"
                        } else {
                            "MATCHED"
                        }
                    };
                    runtimes
                        .entry(exchange_name)
                        .or_default()
                        .reconciliation_status = status.to_string();
                }
            }
        }
        let canonical_symbols: std::collections::HashSet<String> = canonical_local
            .iter()
            .map(|(key, _, _)| key.symbol.clone())
            .collect();
        plan.stale
            .retain(|symbol| !canonical_symbols.contains(symbol));
        plan.mismatched
            .retain(|(symbol, _)| !canonical_symbols.contains(symbol));
        plan.unknown
            .retain(|symbol| !canonical_symbols.contains(symbol));

        // 5. Adım: Farkları Atomik Kilit Altında Local Hafızaya Eşitle
        if let Ok(mut st) = state.lock() {
            // ⚠️ DÜZELTME: `stale` pozisyonlar eskiden SESSİZCE `map.remove` ile siliniyordu →
            // kapalı-işlem kaydı, PnL ve çıkış sebebi kayboluyordu (istatistik + adaptif katman
            // kör kalıyordu). Artık borsa-kapanışı olarak kaydedilip alarm çıkarılıyor.
            // Kayıt, live_positions kilidi BIRAKILDIKTAN sonra yazılır (iç içe kilit yok).
            let mut exchange_closed: Vec<crate::core::model::PositionModel> = Vec::new();
            let mut adopted_unprotected: Vec<String> = Vec::new();
            let mut protection_gaps: Vec<String> = Vec::new();
            if let Ok(mut map) = st.finance.live_positions.write() {
                for key in &canonical_plan.stale {
                    let legacy_key = map
                        .iter()
                        .find(|(_, position)| position.canonical_key().as_ref() == Some(key))
                        .map(|(legacy_key, _)| legacy_key.clone());
                    if let Some(legacy_key) = legacy_key {
                        if let Some(pos) = map.remove(&legacy_key) {
                            exchange_closed.push(pos);
                        }
                    }
                }
                for sym in &plan.stale {
                    if let Some(pos) = map.remove(sym) {
                        exchange_closed.push(pos);
                    }
                }

                // KORUMA BOŞLUĞU taraması: local SL/TP bekleyen ama borsada koruması
                // olmayan pozisyonlar (yalnız SL/TP raporlayan venue'larda — MT5).
                for (sym, p) in map.iter() {
                    let exchange_position = p
                        .canonical_key()
                        .and_then(|key| canonical_exchange_positions.get(&key))
                        .or_else(|| borsa_toplam_pozisyonlari.get(sym));
                    if let Some(ex) = exchange_position {
                        if Self::protection_gap(p.stop_loss, p.take_profit, ex.sl, ex.tp) {
                            protection_gaps.push(sym.clone());
                        }
                    }
                }

                // Miktar/yön uyuşmazlığı olan bacakları borsa otoritesine göre düzelt.
                // Kaldıraç da borsa otoritesidir: local 3× sanıp borsada 10× olmak
                // notional/ROE hesabını sessizce bozar (leverage=0 → bilgi yok, dokunma).
                for (sym, ex) in &plan.mismatched {
                    if let Some(p) = map.get_mut(sym) {
                        p.is_long = ex.amt > 0.0;
                        p.qty = ex.amt.abs();
                        if ex.entry > 0.0 {
                            p.entry_price = ex.entry;
                        }

                        if ex.leverage > 0.0 {
                            p.leverage = ex.leverage;
                        }
                    }
                }
                for (key, ex) in &canonical_plan.mismatched {
                    if let Some((_, p)) = map
                        .iter_mut()
                        .find(|(_, position)| position.canonical_key().as_ref() == Some(key))
                    {
                        p.is_long = ex.amt > 0.0;
                        p.qty = ex.amt.abs();
                        if ex.entry > 0.0 {
                            p.entry_price = ex.entry;
                        }
                        if ex.leverage > 0.0 {
                            p.leverage = ex.leverage;
                        }
                    }
                }

                // Bilinmeyen (borsada var, local'de yok) pozisyonu evlat edin.
                // ESKİ HAL: market:"cfd", interval:"1m", leverage:1.0 SABİTLERİ + SL/TP 0
                // — Binance pozisyonu "cfd" diye etiketleniyor, kaldıraç yanlış, koruma
                // bilgisi düşürülüyordu. Artık venue'dan türetilir; SL/TP borsa söylüyorsa
                // (MT5 pozisyon-üstü) aynen taşınır.
                for key in &canonical_plan.unknown {
                    let sym = &key.symbol;
                    if let Some(ex) = canonical_exchange_positions.get(key) {
                        let (mkt, iv) = venue_registry
                            .route(sym)
                            .map(|(v, _)| {
                                (
                                    v.storage_namespace().1.to_string(),
                                    current_interval.clone(),
                                )
                            })
                            .unwrap_or_else(|| ("cfd".to_string(), "1m".to_string()));
                        let new_pos = crate::core::model::PositionModel {
                            pos_id: format!(
                                "adopt_{}_{}",
                                sym,
                                crate::core::time::now_epoch_secs()
                            ),
                            symbol: sym.clone(),
                            venue: Some(key.venue),
                            entry_price: ex.entry,
                            current_price: ex.entry,
                            qty: ex.amt.abs(),
                            leverage: if ex.leverage > 0.0 { ex.leverage } else { 1.0 },
                            market: mkt,
                            interval: iv,
                            is_long: ex.amt > 0.0,
                            trade_type: "MANUEL_ADOPT".to_string(),
                            opened_at: chrono::Utc::now().to_rfc3339(),
                            stop_loss: ex.sl.unwrap_or(0.0),
                            take_profit: ex.tp.unwrap_or(0.0),
                            trailing_stop: 0.0,
                            max_favorable_price: ex.entry,
                            breakeven_activated: false,
                            kind: None,
                            entry_commission: 0.0,
                        };
                        let unprotected = new_pos.stop_loss <= 0.0 && new_pos.take_profit <= 0.0;
                        let _ = map.insert(sym.clone(), new_pos);
                        if unprotected {
                            adopted_unprotected.push(sym.clone());
                        }
                    }
                }
            } // live_positions kilidi burada bırakılır

            // KORUMA BOŞLUĞU alarmı: pozisyon local'de SL/TP'li görünüyor ama borsada
            // koruma emri YOK (iptal edilmiş / hiç kurulmamış / EA restart'ında düşmüş).
            // Saatlik throttle: her 20 tick'te alarm spam'i olmasın.
            for sym in &protection_gaps {
                if crate::robot::engines::master::log_throttle_should_emit(
                    sym,
                    "protection_gap",
                    3600,
                ) {
                    st.push_alert(
                        "PROTECTION-GAP",
                        crate::robot::infra::telegram_notifier::Severity::Critical,
                        format!(
                            "[MUTABAKAT] {} local SL/TP bekliyor ama BORSADA KORUMA YOK — emir iptal edilmiş ya da hiç kurulmamış olabilir. Motor local exit takibi yapıyor; borsa-tarafı ağ yeniden kurulana dek bağlantı kopuşu riski AÇIK.",
                            sym,
                        ),
                    );
                }
            }

            // KORUMASIZ adopt alarmı: borsadan alınan pozisyonun ne SL'i ne TP'si var —
            // motor exit-condition takibi yapar ama borsa-tarafı güvenlik ağı YOK.
            if !adopted_unprotected.is_empty() {
                st.push_alert(
                    "ADOPT-UNPROTECTED",
                    crate::robot::infra::telegram_notifier::Severity::Critical,
                    format!(
                        "[MUTABAKAT] {} pozisyon borsadan KORUMASIZ evlat edinildi (SL/TP yok): {}. Motor local exit takibi yapacak; borsa-tarafı koruma emri elle/otonom kurulana dek risk açık.",
                        adopted_unprotected.len(), adopted_unprotected.join(", "),
                    ),
                );
            }

            // Borsa tarafında kapanmış pozisyonları KAPALI İŞLEM olarak mühürle.
            for pos in &exchange_closed {
                let exit_price = if pos.current_price > 0.0 {
                    pos.current_price
                } else {
                    pos.entry_price
                };
                let pnl = crate::core::math::calculate_pnl(
                    pos.entry_price,
                    exit_price,
                    pos.qty,
                    pos.is_long,
                );
                let notional = pos.entry_price * pos.qty;
                let pnl_pct = if notional > 0.0 {
                    pnl / notional * 100.0
                } else {
                    0.0
                };
                let closed_trade = crate::core::model::ClosedTradeModel {
                    symbol: pos.symbol.clone(),
                    is_long: pos.is_long,
                    pnl,
                    pnl_pct,
                    exit_reason: super::ExitReason::ExchangeClosed.as_str().to_string(),
                    closed_at: chrono::Utc::now().to_rfc3339(),
                    opened_at: pos.opened_at.clone(),
                    leverage: pos.leverage,
                    net_pnl: pnl,
                    net_pnl_pct: pnl_pct,
                    commission: 0.0,
                    entry_price: pos.entry_price,
                    exit_price,
                    strategy: pos.trade_type.clone(),
                    venue: crate::robot::venue::registry::split_venue_tag(&pos.symbol)
                        .1
                        .unwrap_or_else(|| crate::core::types::Exchange::classify(&pos.symbol))
                        .as_str()
                        .to_string(),
                    pyramid_tranches: 1,
                    exchange_ref: pos.pos_id.clone(),
                };
                if let Ok(mut closed_list) = st.finance.live_closed_trades.write() {
                    if !closed_list
                        .iter()
                        .any(|existing| existing.same_identity(&closed_trade))
                    {
                        closed_list.push(closed_trade);
                    }
                }
                st.finance
                    .closed_trades_total
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            if !exchange_closed.is_empty() {
                let syms: Vec<String> = exchange_closed.iter().map(|p| p.symbol.clone()).collect();
                st.push_alert(
                    "EXCHANGE-CLOSED",
                    crate::robot::infra::telegram_notifier::Severity::Warning,
                    format!(
                        "[MUTABAKAT] {} pozisyon borsada kapanmış (SL/TP/likidasyon/manuel) → kapalı işlem olarak kaydedildi: {}. Çıkış fiyatı son bilinen fiyattır (yaklaşık).",
                        exchange_closed.len(), syms.join(", "),
                    ),
                );
            }
        }

        if let Ok(st) = state.lock() {
            st.refresh_canonical_positions();
        }
        Self::persist_open_positions_snapshot(state);
        return;
    }

    /// Koruma-emri boşluğu: local pozisyon SL/TP ile yönetildiğini SANIYOR ama borsa
    /// tarafında koruma YOK. Yalnız borsanın gerçekten bilgi verdiği durumda (Some)
    /// tetiklenir — `None` = "bu venue pozisyon-üstü SL/TP raporlamıyor" (Binance
    /// positionRisk) → yanlış alarm üretme.
    pub(crate) fn protection_gap(
        local_sl: f64,
        local_tp: f64,
        ex_sl: Option<f64>,
        ex_tp: Option<f64>,
    ) -> bool {
        let local_expects = local_sl > 0.0 || local_tp > 0.0;
        let exchange_reports = ex_sl.is_some() || ex_tp.is_some();
        let exchange_has_none = ex_sl.unwrap_or(0.0) <= 0.0 && ex_tp.unwrap_or(0.0) <= 0.0;
        local_expects && exchange_reports && exchange_has_none
    }

    fn resolve_uncertain_orders_with_exchange_positions(
        state: &Arc<Mutex<AppState>>,
        canonical_exchange_positions: &std::collections::HashMap<
            memos_venue_api::CanonicalPositionKey,
            ExchangePos,
        >,
        exchange_positions: &std::collections::HashMap<String, ExchangePos>,
        positions_reconcile_healthy: bool,
    ) {
        let Some((uncertain_symbols, open_order_symbols, open_orders_healthy)) = (match state.lock()
        {
            Ok(st) => st.finance.uncertain_orders.read().ok().map(|orders| {
                let symbols = orders.keys().cloned().collect::<Vec<_>>();
                let open = st
                    .finance
                    .open_order_symbols
                    .read()
                    .map(|items| items.clone())
                    .unwrap_or_default();
                let healthy = st
                    .finance
                    .open_orders_reconcile_healthy
                    .load(std::sync::atomic::Ordering::Acquire);
                (symbols, open, healthy)
            }),
            Err(_) => return,
        }) else {
            return;
        };
        if uncertain_symbols.is_empty() || !positions_reconcile_healthy || !open_orders_healthy {
            if let Ok(st) = state.lock() {
                if let Ok(mut streaks) = st.finance.uncertain_reconcile_streak.write() {
                    streaks.clear();
                }
            }
            return;
        }
        let (resolved_symbols, health_updates) = match state.lock() {
            Ok(st) => {
                let mut streaks = st.finance.uncertain_reconcile_streak.write().ok();
                let mut resolved = Vec::new();
                let mut updates = Vec::new();
                for symbol in uncertain_symbols {
                    if open_order_symbols.contains(&symbol) {
                        if let Some(map) = streaks.as_mut() {
                            map.insert(symbol.clone(), 0);
                        }
                        if let Ok(mut orders) = st.finance.uncertain_orders.write() {
                            if let Some(order) = orders.get_mut(&symbol) {
                                order.healthy_reconcile_count = 0;
                                order.last_checked_at = chrono::Utc::now().to_rfc3339();
                            }
                        }
                        updates.push((symbol.clone(), 0));
                        continue;
                    }
                    let broker_position_exists = exchange_positions.contains_key(&symbol)
                        || canonical_exchange_positions
                            .keys()
                            .any(|key| key.symbol == symbol);
                    let next = if broker_position_exists {
                        2
                    } else {
                        streaks
                            .as_mut()
                            .and_then(|map| map.get(&symbol).copied())
                            .unwrap_or(0)
                            .saturating_add(1)
                    };
                    if let Some(map) = streaks.as_mut() {
                        map.insert(symbol.clone(), next);
                    }
                    if let Ok(mut orders) = st.finance.uncertain_orders.write() {
                        if let Some(order) = orders.get_mut(&symbol) {
                            order.healthy_reconcile_count = u32::from(next);
                            order.last_checked_at = chrono::Utc::now().to_rfc3339();
                        }
                    }
                    updates.push((symbol.clone(), next));
                    if next >= 2 {
                        resolved.push(symbol);
                    }
                }
                (resolved, updates)
            }
            Err(_) => return,
        };
        if resolved_symbols.is_empty() {
            return;
        }
        let (db_path, profile) = match state.lock() {
            Ok(st) => (
                st.config.state_db_path.clone(),
                st.config.venue_profile.as_str().to_string(),
            ),
            Err(_) => return,
        };
        let now = chrono::Utc::now().to_rfc3339();
        let Ok(conn) = crate::persistence::open_db(&db_path) else {
            return;
        };
        for (symbol, count) in &health_updates {
            if let Err(error) = crate::persistence::writer::update_uncertain_order_health(
                &conn,
                &profile,
                symbol,
                u32::from(*count),
                &now,
            ) {
                log::warn!(
                    "⚠️ [UNCERTAIN-ORDER] {} reconciliation sayacı mühürlenemedi: {}",
                    symbol,
                    error
                );
            }
        }
        let mut resolved = Vec::new();
        for symbol in resolved_symbols {
            match crate::persistence::writer::resolve_uncertain_order(
                &conn, &profile, &symbol, &now,
            ) {
                Ok(()) => resolved.push(symbol),
                Err(error) => log::warn!(
                    "⚠️ [UNCERTAIN-ORDER] {} broker doğrulaması sonrası çözülemedi: {}",
                    symbol,
                    error
                ),
            }
        }
        if let Ok(st) = state.lock() {
            if let Ok(mut orders) = st.finance.uncertain_orders.write() {
                for symbol in resolved {
                    orders.remove(&symbol);
                }
            }
        }
    }

    pub(crate) fn reconcile_diff(
        local: &[(String, bool, f64)],
        exchange: &std::collections::HashMap<String, ExchangePos>,
    ) -> ReconcilePlan {
        let mut plan = ReconcilePlan::default();
        for (sym, is_long, qty) in local {
            match exchange.get(sym) {
                None => plan.stale.push(sym.clone()),
                Some(ex) => {
                    let ex_is_long = ex.amt > 0.0;
                    let ex_qty = ex.amt.abs();
                    let tol = (ex_qty * 0.001).max(1e-9);
                    if ex_is_long != *is_long || (qty - ex_qty).abs() > tol {
                        plan.mismatched.push((sym.clone(), ex.clone()));
                    }
                }
            }
        }
        let local_syms: std::collections::HashSet<&String> =
            local.iter().map(|(s, _, _)| s).collect();
        for sym in exchange.keys() {
            if !local_syms.contains(sym) {
                plan.unknown.push(sym.clone());
            }
        }
        plan.stale.sort();
        plan.mismatched.sort_by(|a, b| a.0.cmp(&b.0));
        plan.unknown.sort();
        return plan;
    }

    pub(crate) fn reconcile_canonical_diff(
        local: &[(memos_venue_api::CanonicalPositionKey, bool, f64)],
        exchange: &std::collections::HashMap<memos_venue_api::CanonicalPositionKey, ExchangePos>,
    ) -> CanonicalReconcilePlan {
        let mut plan = CanonicalReconcilePlan::default();
        for (key, is_long, qty) in local {
            match exchange.get(key) {
                None => plan.stale.push(key.clone()),
                Some(ex) => {
                    let ex_is_long = ex.amt > 0.0;
                    let ex_qty = ex.amt.abs();
                    let tolerance = (ex_qty * 0.001).max(1e-9);
                    if ex_is_long != *is_long || (qty - ex_qty).abs() > tolerance {
                        plan.mismatched.push((key.clone(), ex.clone()));
                    }
                }
            }
        }
        let local_keys: std::collections::HashSet<&memos_venue_api::CanonicalPositionKey> =
            local.iter().map(|(key, _, _)| key).collect();
        for key in exchange.keys() {
            if !local_keys.contains(key) {
                plan.unknown.push(key.clone());
            }
        }
        plan.stale.sort_by_key(|key| key.token());
        plan.mismatched.sort_by_key(|(key, _)| key.token());
        plan.unknown.sort_by_key(|key| key.token());
        plan
    }

    pub fn looks_like_bist_symbol(sym: &str) -> bool {
        // Borsa sembolleri daima büyük harf gelir; küçük harf içeren serbest metin
        // (örn. log/JSON'dan gelen ham anahtar) BIST sembolü sayılmamalı — Exchange::classify
        // normalize ettiği için (to_ascii_uppercase) burada ayrıca kontrol ediyoruz.
        if sym.trim().chars().any(|c| c.is_ascii_lowercase()) {
            return false;
        }
        crate::core::types::Exchange::classify(sym) == crate::core::types::Exchange::Bist
    }

    pub fn persist_open_positions_snapshot(state: &Arc<Mutex<AppState>>) {
        let (state_db_path, profile, positions) = match state.lock() {
            Ok(st) => {
                let state_db_path = st.config.state_db_path.clone();
                let profile = st.config.venue_profile.as_str().to_string();
                let positions: Vec<_> = st
                    .finance
                    .live_positions
                    .read()
                    .map(|m| m.values().cloned().collect())
                    .unwrap_or_default();
                (state_db_path, profile, positions)
            }
            Err(_) => return,
        };
        spawn_db_write(move || {
            let _guard = DB_PERSIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            if let Ok(conn) = crate::persistence::open_db(&state_db_path) {
                let _ = crate::persistence::writer::save_open_positions_snapshot_for_profile(
                    &conn, &profile, &positions,
                );
            }
        });
    }
    pub(crate) fn hydrate_account_state_from_db(state: &Arc<Mutex<AppState>>) {
        let (state_db_path, profile, config_capital) = match state.lock() {
            Ok(st) => (
                st.config.state_db_path.clone(),
                st.config.venue_profile.as_str().to_string(),
                st.config.capital,
            ),
            Err(_) => return,
        };
        let mut purged_unconfirmed_adopt_closes = false;
        match crate::persistence::reader::load_account_state_for_profile(&state_db_path, &profile) {
            Ok(Some(rec)) => {
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
                    st.finance
                        .closed_trades_total
                        .store(rec.closed_trades_count, Ordering::Relaxed);
                    match crate::persistence::reader::load_closed_trades_snapshot_for_profile(
                        &state_db_path,
                        &profile,
                    ) {
                        Ok(Some(snapshot)) => {
                            let mut restored_trade_count = None;
                            if let Ok(mut closed) = st.finance.live_closed_trades.write() {
                                let before = snapshot.trades.len();
                                *closed = snapshot.trades;
                                closed.retain(|trade| !trade.is_unconfirmed_adopt_close());
                                purged_unconfirmed_adopt_closes = before != closed.len();
                                crate::core::model::ClosedTradeModel::deduplicate(&mut closed);
                                restored_trade_count = Some(closed.len());
                            }
                            if let Some(restored_trade_count) = restored_trade_count {
                                if restored_trade_count != rec.closed_trades_count {
                                    st.push_log_mirror(format!(
                                        "⚠️ [RECOVERY] kapalı işlem sayısı düzeltildi: DB={} snapshot={}",
                                        rec.closed_trades_count, restored_trade_count
                                    ));
                                }
                                st.finance
                                    .closed_trades_total
                                    .store(restored_trade_count, Ordering::Relaxed);
                            }
                            if let Ok(mut hist) = st.finance.equity_history.write() {
                                hist.clear();
                                hist.extend(snapshot.equity_history);
                                if hist.is_empty() {
                                    hist.push_back(rec.equity);
                                }
                            }
                        }
                        Ok(None) => {
                            if let Ok(mut hist) = st.finance.equity_history.write() {
                                hist.clear();
                                hist.push_back(rec.equity);
                            }
                        }
                        Err(e) => {
                            log::warn!(
                                "⚠️ Kapalı işlem/equity snapshot recovery başarısız ({}): {}",
                                profile,
                                e
                            );
                            if let Ok(mut hist) = st.finance.equity_history.write() {
                                hist.clear();
                                hist.push_back(rec.equity);
                            }
                        }
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
                        "♻️ [RECOVERY] account_state boş — cold-start (equity başlangıç)"
                            .to_string(),
                    );
                }
            }
            Err(e) => {
                log::warn!(
                    "⚠️ [RECOVERY] account_state okunamadı: {} — cold-start'a düşülüyor",
                    e
                );
                push_state_log(
                    state,
                    format!(
                        "⚠️ [RECOVERY] account_state okunamadı: {} (cold-start'a düşülüyor)",
                        e,
                    ),
                );
            }
        }
        if purged_unconfirmed_adopt_closes {
            push_state_log(
                state,
                "🧹 [RECOVERY] broker doğrulaması olmayan MANUEL_ADOPT/TRAILING_STOP kayıtları temizlendi"
                    .to_string(),
            );
            Self::persist_account_state(state);
        }
    }

    pub(crate) fn hydrate_symbol_status_from_db(state: &Arc<Mutex<AppState>>) {
        let db_path = match state.lock() {
            Ok(st) => st.config.db_path.clone(),
            Err(_) => return,
        };
        if let Ok(entries) = crate::persistence::reader::load_symbol_statuses(&db_path) {
            if !entries.is_empty() {
                let n_break = entries.iter().filter(|(_, s)| s != "TRADING").count();
                super::set_symbol_statuses(&entries);
                if let Ok(mut st) = state.lock() {
                    st.push_log_mirror(format!(
                        "🗂️ [RECOVERY] sembol statü registry: {} sembol ({} TRADING-dışı)",
                        entries.len(),
                        n_break,
                    ));
                }
            }
        }
    }
    pub fn purge_delisted_symbol(state: &Arc<Mutex<AppState>>, symbol: &str, n_fail: u32) {
        log::warn!(
            "🚮 Delisted detection: {} sembolü {} ardışık fetch hatasından sonra purge ediliyor",
            symbol,
            n_fail,
        );
        super::mark_delisted_skip(symbol);
        let mut delisted_anomaly_msg: Option<String> = None;
        if let Ok(mut st) = state.lock() {
            let removed_from_orch = st
                .fleet
                .symbol_orchestrator
                .write()
                .map(|mut o| o.stop_symbol(symbol))
                .unwrap_or(false);

            if let Ok(mut prices) = st.fleet.live_price.write() {
                prices.remove(symbol);
            }
            if let Ok(mut updated_at) = st.fleet.live_price_updated_at.write() {
                updated_at.remove(symbol);
            }

            let closed_pos = st
                .finance
                .live_positions
                .write()
                .ok()
                .and_then(|mut map| map.remove(symbol));

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
                    exit_price: pos.entry_price,
                    strategy: pos.trade_type.clone(),
                    venue: crate::robot::venue::registry::split_venue_tag(&pos.symbol)
                        .1
                        .unwrap_or_else(|| crate::core::types::Exchange::classify(&pos.symbol))
                        .as_str()
                        .to_string(),
                    pyramid_tranches: 1,
                    exchange_ref: pos.pos_id.clone(),
                };
                if let Ok(mut closed_list) = st.finance.live_closed_trades.write() {
                    if !closed_list
                        .iter()
                        .any(|existing| existing.same_identity(&closed_trade))
                    {
                        closed_list.push(closed_trade);
                    }
                }
                st.finance
                    .closed_trades_total
                    .fetch_add(1, Ordering::Relaxed);
                force_close_msg = format!(
                    " · live pozisyon force-close (qty={:.4}, entry=${:.4}, PnL=0)",
                    pos.qty, pos.entry_price,
                );
            }

            if let Ok(mut pipe) = st.guardian.live_pipeline.write() {
                use crate::robot::data_pipeline::{AnomalyKind, AnomalySeverity};
                let msg = format!(
                    "DELISTED: {} purge edildi ({} ardışık fetch hatası){}",
                    symbol, n_fail, force_close_msg
                );
                pipe.push_anomaly(AnomalySeverity::Warning, AnomalyKind::Custom, msg.clone());
                delisted_anomaly_msg = Some(msg);
            }
            st.push_log(format!(
                "🚮 {} DELISTED → orchestrator removed={}{}",
                symbol, removed_from_orch, force_close_msg,
            ));
        }
        if let Some(msg) = delisted_anomaly_msg {
            emit_pipeline_anomaly_event(
                state,
                crate::robot::data_pipeline::AnomalySeverity::Warning,
                crate::robot::data_pipeline::AnomalyKind::Custom,
                &msg,
            );
        }
        Self::persist_open_positions_snapshot(state);
        Self::persist_account_state(state);
        delisted_record_success(symbol);
    }

    pub fn mark_execution_epoch(state: &Arc<Mutex<AppState>>) {
        let now_secs = crate::core::time::now_epoch_secs();
        if let Ok(st) = state.lock() {
            st.fleet
                .last_execution_epoch
                .store(now_secs, Ordering::Relaxed);
        }
    }

    pub fn persist_account_state(state: &Arc<Mutex<AppState>>) {
        let (
            state_db_path,
            profile,
            equity,
            peak,
            starting,
            closed_count,
            mut trades,
            equity_history,
        ) = match state.lock() {
            Ok(st) => (
                st.config.state_db_path.clone(),
                st.config.venue_profile.as_str().to_string(),
                st.finance.equity,
                st.finance.peak_equity,
                st.finance.starting_capital,
                st.finance.closed_trades_total.load(Ordering::Relaxed),
                st.finance
                    .live_closed_trades
                    .read()
                    .map(|trades| trades.clone())
                    .unwrap_or_default(),
                st.finance
                    .equity_history
                    .read()
                    .map(|history| history.iter().copied().collect::<Vec<_>>())
                    .unwrap_or_default(),
            ),
            Err(_) => return,
        };
        crate::core::model::ClosedTradeModel::deduplicate(&mut trades);
        spawn_db_write(move || {
            let _guard = DB_PERSIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            match crate::persistence::open_db(&state_db_path) {
                Ok(conn) => {
                    if let Err(e) = crate::persistence::writer::save_account_state_for_profile(
                        &conn,
                        &profile,
                        equity,
                        peak,
                        starting,
                        closed_count,
                    ) {
                        log::warn!("⚠️ Account state persistence başarısız: {}", e);
                    }
                    if let Err(e) =
                        crate::persistence::writer::save_closed_trades_snapshot_for_profile(
                            &conn,
                            &profile,
                            &trades,
                            &equity_history,
                        )
                    {
                        log::warn!(
                            "⚠️ Kapalı işlem/equity snapshot persistence başarısız: {}",
                            e
                        );
                    }
                }
                Err(e) => {
                    log::warn!("⚠️ Account state DB açılamadı: {}", e);
                }
            }
        });
    }

    /// 🛡️ OTONOM ÇOKLU BORSA EMİR MUTABAKAT MOTORU (Vetosuz ve Sabitsiz)
    /// Binance, MT5 ve IBKR borsalarında askıda bekleyen açık emirleri (Working/Pending Orders)
    /// otonom olarak çeker, local registry'de olmayan yabancı sembolleri anında evlat edinir (Adopt),
    /// TUI (Mission Control) snapshot ekranını %100 dürüst seviyeye senkronize eder.
    pub async fn reconcile_live_open_orders_with_exchange(state: &Arc<Mutex<AppState>>) {
        let (venue_registry, current_market, current_interval) = {
            let Ok(st) = state.lock() else {
                return;
            };
            (
                Arc::clone(&st.venue_registry),
                st.config.market.clone(),
                st.config.interval.clone(),
            )
        };

        // Dry-run modunda veya borsa adaptör hattı ayağa kalkmadıysa işlem yapma
        if venue_registry.is_empty() {
            return;
        }

        let mut borsa_aktif_emirleri: std::collections::HashMap<String, Vec<serde_json::Value>> =
            std::collections::HashMap::new();
        let mut open_orders_healthy = true;

        // 1. Adım: Tüm kayıtlı aktif borsaları (Binance, MT5, IBKR) dinamik geziyoruz (Pazar-Agnostik)
        for spec in venue_registry.specs() {
            let Some(venue) = venue_registry.get_spec(spec) else {
                continue;
            };

            // Somut borsa adaptör arayüzünden açık emir listesini bulk veya parite bazlı talep et
            // Not: Yeni eklenen IBKR/MT5 adaptör trait'lerinizdeki `fetch_open_orders` asenkron metoduna delege edilir.
            match venue.fetch_open_orders().await {
                Ok(open_orders_resp) => {
                    for order in open_orders_resp {
                        if let Some(sym) = order
                            .get("symbol")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                        {
                            borsa_aktif_emirleri.entry(sym).or_default().push(order);
                        }
                    }
                }
                Err(error) => {
                    open_orders_healthy = false;
                    push_state_log(
                        state,
                        format!(
                            "⚠️ [OPEN-ORDERS] {:?} sorgusu başarısız; uncertain çözümleme bu cycle atlandı: {}",
                            spec.exchange, error
                        ),
                    );
                }
            }
        }

        if let Ok(st) = state.lock() {
            if let Ok(mut symbols) = st.finance.open_order_symbols.write() {
                symbols.clear();
                symbols.extend(borsa_aktif_emirleri.keys().cloned());
            }
            st.finance
                .open_orders_reconcile_healthy
                .store(open_orders_healthy, std::sync::atomic::Ordering::Release);
        }

        // 2. Adım: Otonom Evlat Edinme (Auto-Discovery & Adopt Policy)
        // Eğer borsada açık bir emir varsa ama bu sembol bizim işçi hatlarımızda veya pinned listemizde yoksa sisteme enjekte et.
        let mut yeni_kesfedilenler = Vec::new();
        {
            let Ok(mut st) = state.lock() else {
                return;
            };
            for sym in borsa_aktif_emirleri.keys() {
                if !crate::robot::config_engine::OtonomConfigEngine::symbol_allowed(sym) {
                    continue;
                }
                let has_symbol = st.config.pinned_symbols.contains(sym);
                if !has_symbol {
                    st.config.pinned_symbols.push(sym.clone());
                    yeni_kesfedilenler.push(sym.clone());
                }
            }

            // Keşfedilen yeni semboller için arka planda anlık otonom Canlı İşçi Hattı (Worker Thread) spawn et
            if !yeni_kesfedilenler.is_empty() {
                if let Ok(mut orch) = st.fleet.symbol_orchestrator.write() {
                    for sym in &yeni_kesfedilenler {
                        let _ = orch.register(sym, &current_market, &current_interval);
                    }
                }
                st.push_log(format!(
                    "🦅 [AUTO-ADOPT] Borsada bot-dışı açık emirli semboller keşfedildi ve otonom evlat edinildi: {}",
                    yeni_kesfedilenler.join(", ")
                ));
            }
        }

        // 3. Adım: TUI (Mission Control) `live_orders` Havuzunun Senkronizasyonu
        // Borsadan gelen dürüst veriyi atomik write() kilidi altında local snapshot haritasına mühürle.
        if let Ok(st) = state.lock() {
            if let Ok(mut live_orders_map) = st.finance.live_orders.write() {
                // Borsada artık var olmayan (dolmuş veya manuel iptal edilmiş) tüm hayalet emirleri temizle
                live_orders_map.clear();

                for (sym, orders) in borsa_aktif_emirleri {
                    for order in orders {
                        let entry_id = order.get("orderId").map(|v| v.to_string());
                        let sl_id = order
                            .get("clientOrderId")
                            .and_then(|v| v.as_str())
                            .filter(|s| s.contains("sl_"))
                            .map(|s| s.to_string());
                        let tp_id = order
                            .get("clientOrderId")
                            .and_then(|v| v.as_str())
                            .filter(|s| s.contains("tp_"))
                            .map(|s| s.to_string());
                        let number = |key: &str| {
                            order
                                .get(key)
                                .and_then(|value| value.as_f64())
                                .unwrap_or(0.0)
                        };
                        let limit_price = order.get("price").and_then(|value| value.as_f64());
                        let account = order
                            .get("account")
                            .and_then(|value| value.as_str())
                            .map(str::to_string);

                        let refs = crate::core::model::LiveOrderRefs {
                            entry_order_id: entry_id,
                            sl_order_id: sl_id,
                            tp_order_id: tp_id,
                            placed_at: chrono::Utc::now().to_rfc3339(),
                            status: order
                                .get("status")
                                .and_then(|value| value.as_str())
                                .unwrap_or("unknown")
                                .to_string(),
                            side: order
                                .get("side")
                                .and_then(|value| value.as_str())
                                .unwrap_or("")
                                .to_string(),
                            quantity: number("qty"),
                            filled_quantity: number("filled_qty"),
                            remaining_quantity: number("remaining_qty"),
                            limit_price,
                            account,
                        };
                        let _ = live_orders_map.insert(sym.clone(), refs);
                    }
                }
            }
        }

        return;
    }

    // persistence.rs içerisine eklenecek periyodik kapanan işlem toplayıcısı:
    /// 🛡️ OTONOM ÇOKLU BORSA KAPANMIŞ İŞLEM MUTABAKAT MOTORU (Tam Kilitli Sürüm)
    /// GEREKÇE: Kullanıcının uyarısı uyarınca, `state.lock()` ile yakalanan `st` sarmalı üzerinden
    /// porta zaten bağlı olan meşru ve paylaşımlı tekil pointer çağrılmıştır.
    /// Trait boyut (`Sized`) kısıtlamaları Rust standart kütüphane kurallarıyla tamamen eritilmiştir.
    /// 🛡️ OTONOM ÇOKLU BORSA KAPANMIŞ İŞLEM MUTABAKAT MOTORU (Kesin Çözüm Sürümü)
    /// GEREKÇE: Tuple parçalama (destructuring) sökülerek, her bir Arc pointer'ı kendi bağımsız
    /// scope satırında açıkça klonlanmıştır. Yaşam ömrü (`Lifetime`) bağımlılığı ve asenkron
    /// `.await` noktası kilit taşınma çelişkisi (`Send`) aynı anda tamamen eritilmiştir.
    /// 🛡️ OTONOM ÇOKLU BORSA KAPANMIŞ İŞLEM MUTABAKAT MOTORU (Nihai Yaşam Ömrü Kalkanı)
    /// GEREKÇE: Fonksiyonun en başında tüm gerekli `Arc` pointer'ları tek bir kilit altında klonlanmış,
    /// ardından `drop(st);` ile senkron MutexGuard asenkron `.await` hattı öncesinde fiziksel olarak imha edilmiştir.
    /// Derleyicinin inatçı `does not live long enough` (Lifetime) ve `Send` hataları kökten kazınmıştır.
    /// Borsa tarafında kapanmış işlemleri local kapalı-işlem listesine yansıtır.
    ///
    /// YENİDEN YAZILDI (mutabakat tek-otorite çizgisi):
    ///   * Eski gövde YALNIZ MT5'e bakıyor ve kayıtlı venue'yu kullanmak yerine her
    ///     çağrıda SIFIRDAN bir `Mt5Bridge` kuruyordu → aynı porta ikinci listener
    ///     (çekişme) + EA asla bu yeni hatta bağlı değil → fonksiyon HİÇ çalışmıyordu.
    ///     Artık registry'deki TÜM venue'ların `fetch_closed_trades()` yeteneği gezilir
    ///     (paylaşımlı adaptör; desteklemeyen venue default boş döner).
    ///   * Dedup `symbol + |pnl|<1e-4` idi → aynı sembolde aynı PnL'li iki GERÇEK işlem
    ///     kayboluyordu. Artık borsa referansı (`ticket`) birincil anahtar; ticket
    ///     yoksa (eski EA) symbol+pnl+qty'ye düşer.
    ///   * Kayıt kalitesi: sembol ÇIPLAK (venue ayrı alanda — DB kanonikleştirmesiyle
    ///     tutarlı), kapanış zamanı EA `time`'ından, fiyat `price`'tan (eskiden now/0).
    pub(crate) async fn reconcile_live_closed_trades_with_exchange(state: &Arc<Mutex<AppState>>) {
        let (venue_registry, closed_trades_arc, closed_total_atomic) = {
            let Ok(st) = state.lock() else {
                return;
            };
            (
                Arc::clone(&st.venue_registry),
                Arc::clone(&st.finance.live_closed_trades),
                Arc::clone(&st.finance.closed_trades_total),
            )
        };
        if venue_registry.is_empty() {
            return;
        }

        let mut imported_total = 0usize;
        for exchange in venue_registry.exchanges() {
            let Some(venue) = venue_registry.get(exchange) else {
                continue;
            };
            let Ok(trades) = venue.fetch_closed_trades().await else {
                continue;
            };
            if trades.is_empty() {
                continue;
            }

            let venue_name = venue.exchange().as_str().to_string();
            let Ok(mut closed_list) = closed_trades_arc.write() else {
                continue;
            };
            for t in &trades {
                let Some(sym) = t.get("symbol").and_then(|v| v.as_str()) else {
                    continue;
                };
                let pnl = t.get("pnl").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let qty = t.get("qty").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let ticket = t
                    .get("ticket")
                    .map(|v| {
                        v.as_u64()
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| v.to_string())
                    })
                    .unwrap_or_default();

                let already = if !ticket.is_empty() {
                    closed_list
                        .iter()
                        .any(|c| c.venue == venue_name && c.exchange_ref == ticket)
                } else {
                    // Eski EA (ticket alanı yok): kırılgan ama tek seçenek olan anahtar.
                    closed_list.iter().any(|c| {
                        c.venue == venue_name && c.symbol == sym && (c.pnl - pnl).abs() < 1e-4
                    })
                };
                if already {
                    continue;
                }

                let closed_at = t
                    .get("time")
                    .and_then(|v| v.as_i64())
                    .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
                let exit_price = t.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let commission = t.get("commission").and_then(|v| v.as_f64()).unwrap_or(0.0);

                closed_list.push(crate::core::model::ClosedTradeModel {
                    symbol: sym.to_string(), // ÇIPLAK — venue ayrımı `venue` alanında
                    is_long: t.get("side").and_then(|v| v.as_str()) == Some("buy_close"),
                    exit_reason: super::ExitReason::ExchangeClosed.as_str().to_string(),
                    pnl,
                    pnl_pct: 0.0, // giriş fiyatı EA yanıtında yok → oran hesaplanamaz
                    net_pnl: pnl - commission.abs(),
                    net_pnl_pct: 0.0,
                    commission,
                    closed_at,
                    opened_at: String::new(), // bilinmiyor — sahte now() yazmak yerine boş
                    leverage: 0.0,            // bilinmiyor (0 = bilinmiyor; 1.0 yanıltıcıydı)
                    entry_price: 0.0,
                    exit_price,
                    strategy: "EXCHANGE_SIDE".into(),
                    venue: venue_name.clone(),
                    pyramid_tranches: 1,
                    exchange_ref: ticket,
                });
                let _ = qty; // qty ClosedTradeModel'de alan değil; dedup fallback'te kullanılabilir
                closed_total_atomic.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                imported_total += 1;
            }
        }
        if imported_total > 0 {
            Self::persist_account_state(state);
            if let Ok(mut st) = state.lock() {
                st.push_log(format!(
                    "🔁 Kapanmış işlem mutabakatı: {} venue işlemi local geçmişe eklendi",
                    imported_total
                ));
            }
        }
        return;
    }
}

#[cfg(test)]
mod reconcile_tests {
    use super::*;

    fn ex(m: &[(&str, f64, f64)]) -> std::collections::HashMap<String, ExchangePos> {
        m.iter()
            .map(|(s, amt, entry)| {
                (
                    s.to_string(),
                    ExchangePos {
                        amt: *amt,
                        entry: *entry,
                        ..Default::default()
                    },
                )
            })
            .collect()
    }

    /// Borsa haritası BOŞ + local dolu → hepsi stale. Bu, `fetch_open_orders` yanlış
    /// endpoint hatasının imzasıydı: pozisyonlar sessizce siliniyordu. Motorun bu davranışı
    /// DOĞRU (girdi boşsa hepsi gerçekten yok); hata besleme tarafındaydı. Kilit olarak
    /// burada tutuluyor ki besleme yine bozulursa test niyeti belgelesin.
    #[test]
    fn empty_exchange_map_marks_everything_stale() {
        let local = vec![("BTCUSDT".to_string(), true, 1.0)];
        let plan = Engine::reconcile_diff(&local, &ex(&[]));
        assert_eq!(plan.stale, vec!["BTCUSDT"]);
        assert!(plan.mismatched.is_empty() && plan.unknown.is_empty());
    }

    #[test]
    fn canonical_diff_keeps_same_symbol_venues_separate() {
        let binance = memos_venue_api::CanonicalPositionKey::new(
            memos_venue_api::VenueKey::new(
                crate::core::types::Exchange::Binance,
                crate::core::types::Market::Futures,
                memos_venue_api::VenueProfile::Default,
            ),
            "BTCUSDT",
        );
        let ibkr = memos_venue_api::CanonicalPositionKey::new(
            memos_venue_api::VenueKey::new(
                crate::core::types::Exchange::Ibkr,
                crate::core::types::Market::Cfd,
                memos_venue_api::VenueProfile::Paper,
            ),
            "BTCUSDT",
        );
        let local = vec![(binance.clone(), true, 1.0)];
        let mut exchange = std::collections::HashMap::new();
        exchange.insert(
            binance,
            ExchangePos {
                amt: 1.0,
                entry: 100.0,
                ..Default::default()
            },
        );
        exchange.insert(
            ibkr.clone(),
            ExchangePos {
                amt: -2.0,
                entry: 101.0,
                ..Default::default()
            },
        );

        let plan = Engine::reconcile_canonical_diff(&local, &exchange);
        assert!(plan.stale.is_empty());
        assert!(plan.mismatched.is_empty());
        assert_eq!(plan.unknown, vec![ibkr]);
    }

    #[test]
    fn matching_position_produces_no_diff() {
        let local = vec![("BTCUSDT".to_string(), true, 1.0)];
        let plan = Engine::reconcile_diff(&local, &ex(&[("BTCUSDT", 1.0, 100.0)]));
        assert!(plan.stale.is_empty() && plan.mismatched.is_empty() && plan.unknown.is_empty());
    }

    /// Miktar farkı toleransı: %0.1. Altı eşleşme, üstü mismatch.
    #[test]
    fn quantity_drift_respects_tolerance() {
        let local = vec![("BTCUSDT".to_string(), true, 1.0)];
        // 1.0 vs 1.0005 → tolerans (0.001) içinde → fark YOK
        let plan = Engine::reconcile_diff(&local, &ex(&[("BTCUSDT", 1.0005, 100.0)]));
        assert!(
            plan.mismatched.is_empty(),
            "tolerans içi fark bildirilmemeli: {plan:?}"
        );
        // 1.0 vs 1.5 → mismatch, borsa otoritesi
        let plan = Engine::reconcile_diff(&local, &ex(&[("BTCUSDT", 1.5, 100.0)]));
        assert_eq!(plan.mismatched.len(), 1);
        let (sym, exp) = &plan.mismatched[0];
        assert_eq!(sym, "BTCUSDT");
        assert!(exp.amt > 0.0);
        assert_eq!(exp.amt.abs(), 1.5);
        assert_eq!(exp.entry, 100.0);
    }

    /// Yön uyuşmazlığı: local LONG, borsa SHORT (negatif positionAmt).
    #[test]
    fn side_mismatch_is_detected() {
        let local = vec![("BTCUSDT".to_string(), true, 1.0)];
        let plan = Engine::reconcile_diff(&local, &ex(&[("BTCUSDT", -1.0, 100.0)]));
        assert_eq!(plan.mismatched.len(), 1, "LONG↔SHORT farkı yakalanmalı");
        assert!(plan.mismatched[0].1.amt < 0.0, "borsa tarafı SHORT olmalı");
    }

    /// Borsada var, local'de yok → unknown (evlat edinilecek).
    #[test]
    fn exchange_only_position_is_unknown() {
        let local: Vec<(String, bool, f64)> = vec![];
        let plan = Engine::reconcile_diff(&local, &ex(&[("EURUSD@mt5", 0.5, 1.1)]));
        assert_eq!(plan.unknown, vec!["EURUSD@mt5"]);
    }

    /// Üç kategori aynı anda ayrışmalı.
    #[test]
    fn all_three_categories_separate_correctly() {
        let local = vec![
            ("GONE".to_string(), true, 1.0),  // borsada yok  → stale
            ("DRIFT".to_string(), true, 1.0), // miktar farkı → mismatched
            ("OK".to_string(), false, 2.0),   // birebir      → fark yok
        ];
        let plan = Engine::reconcile_diff(
            &local,
            &ex(&[("DRIFT", 3.0, 50.0), ("OK", -2.0, 10.0), ("NEW", 1.0, 7.0)]),
        );
        assert_eq!(plan.stale, vec!["GONE"]);
        assert_eq!(plan.mismatched.len(), 1);
        assert_eq!(plan.mismatched[0].0, "DRIFT");
        assert_eq!(plan.unknown, vec!["NEW"]);
    }

    /// json_f64 hem Binance string'ini ("0.001") hem MT5 ham sayısını okumalı.
    /// Eski kod yalnız string bekliyordu → MT5 entryPrice HEP 0 parse ediliyordu.
    #[test]
    fn json_f64_accepts_both_number_and_string() {
        let v: serde_json::Value = serde_json::json!({
            "s": "0.001", "n": 1.08542, "bad": true
        });
        assert_eq!(json_f64(v.get("s")), Some(0.001));
        assert_eq!(json_f64(v.get("n")), Some(1.08542));
        assert_eq!(json_f64(v.get("bad")), None);
        assert_eq!(json_f64(None), None);
    }

    /// Kaldıraç mismatch'te BORSA otoritesiyle taşınır (0 = bilinmiyor → plan yine kurulur,
    /// uygulama tarafı 0'ı yok sayar).
    #[test]
    fn mismatch_carries_exchange_leverage() {
        let local = vec![("BTCUSDT".to_string(), true, 1.0)];
        let mut m = std::collections::HashMap::new();
        m.insert(
            "BTCUSDT".to_string(),
            ExchangePos {
                amt: 2.0,
                entry: 100.0,
                leverage: 10.0,
                sl: None,
                tp: None,
            },
        );
        let plan = Engine::reconcile_diff(&local, &m);
        assert_eq!(plan.mismatched.len(), 1);
        assert_eq!(
            plan.mismatched[0].1.leverage, 10.0,
            "borsa kaldıracı plana taşınmalı"
        );
    }

    /// SL için None (bilgi yok — Binance positionRisk) ile Some(0.0) (borsa 'koruma yok'
    /// diyor — MT5) FARKLI şeylerdir; parse katmanı bu ayrımı korumalı.
    #[test]
    fn missing_sl_field_is_none_not_zero() {
        let mt5: serde_json::Value = serde_json::json!({"sl": 0.0, "tp": 1.10});
        let binance: serde_json::Value = serde_json::json!({"leverage": "10"});
        assert_eq!(
            mt5.get("sl").and_then(|v| json_f64(Some(v))),
            Some(0.0),
            "MT5: koruma yok bilgisi"
        );
        assert_eq!(
            binance.get("sl").and_then(|v| json_f64(Some(v))),
            None,
            "Binance: bilgi yok"
        );
    }

    /// Koruma-boşluğu matrisi: yalnız "local bekliyor + borsa RAPORLUYOR + borsada yok"
    /// üçlüsü tetikler. None (Binance: bilgi yok) asla tetiklemez.
    #[test]
    fn protection_gap_matrix() {
        // MT5: borsa Some(0) diyor, local SL bekliyor → BOŞLUK
        assert!(Engine::protection_gap(95.0, 110.0, Some(0.0), Some(0.0)));
        assert!(
            Engine::protection_gap(95.0, 0.0, Some(0.0), Some(0.0)),
            "yalnız SL beklentisi de yeter"
        );
        // Borsa koruma raporluyor ve VAR → boşluk yok
        assert!(!Engine::protection_gap(95.0, 110.0, Some(94.8), Some(0.0)));
        assert!(!Engine::protection_gap(95.0, 110.0, Some(0.0), Some(111.0)));
        // Binance: alanlar yok (None) → bilgi yok, ASLA tetikleme
        assert!(!Engine::protection_gap(95.0, 110.0, None, None));
        // Local zaten koruma beklemiyor → boşluk yok
        assert!(!Engine::protection_gap(0.0, 0.0, Some(0.0), Some(0.0)));
    }

    /// Borsa-kapanışı çıkış sebebi artık TrailingStop diye YALAN yazılmıyor.
    #[test]
    fn exchange_closed_has_its_own_exit_reason() {
        assert_eq!(
            super::ExitReason::ExchangeClosed.as_str(),
            "EXCHANGE_CLOSED"
        );
        assert_ne!(
            super::ExitReason::ExchangeClosed.as_str(),
            super::ExitReason::TrailingStop.as_str(),
        );
    }
}
