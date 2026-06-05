// src/robot/engines/master/xs_live.rs — Kesitsel (cross-sectional) relatif-güç ADANMIŞ MOD.
//
// XS_LIVE_ENABLED iken sepet sembolleri SADECE kesitsel kitapla (market-nötr long/short) yönetilir;
// ScalpSwing/seed yalnız sepet-DIŞI sembollerde çalışır → tek-pozisyon/sembol invariantı temiz kalır.
// Skorlama backtest çekirdeğiyle BİT-AYNI (`xs_target_book` → `select_books`, DRY): her cycle sepeti
// momentum sinyaline göre sırala, no-trade band ile hedef kitabı belirle, mevcut pozisyonları hedefe
// taşı (aç/kapat/flip). Backtest+WF-OOS+Newey-West doğrulamasından gelen edge'in canlı ifadesi.
// [[project_xs_momentum]] [[feedback_autonomy_first]] (sabit env serpme yok → ParameterStore.xs_live).
use super::*;
use std::collections::{HashMap, HashSet};

/// Kesitsel adanmış mod aksiyonu (saf plan → imperatif infaz). flip = Close + Open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum XsAction {
    OpenLong(String),
    OpenShort(String),
    Close(String), // →flat ya da flip'in kapatma yarısı (önce kapanışlar infaz edilir)
}

/// SAF: son kapanıştan lookback-bar geriye momentum sinyali = close[t]/close[t−lb]−1. Yetersiz → None.
pub(crate) fn latest_signal(closes: &[f64], lookback: usize) -> Option<f64> {
    let n = closes.len();
    if n <= lookback {
        return None;
    }
    let (c0, cl) = (closes[n - 1], closes[n - 1 - lookback]);
    if cl > 0.0 && c0 > 0.0 {
        Some(c0 / cl - 1.0)
    } else {
        None
    }
}

/// SAF (testli): hedef long/short kitabı + mevcut pozisyon yönleri (symbol→is_long) → aksiyon listesi.
/// Hedefle aynı yön → no-op (tut). Yön değişimi → Close (flat) ya da Close+Open (flip). Kapanışlar
/// listede AÇILIŞLARDAN ÖNCE gelir → flip'te önce kapat sonra aç (infaz bu sırayı korur).
pub(crate) fn xs_plan_actions(
    longs: &[String], shorts: &[String], current: &HashMap<String, bool>,
) -> Vec<XsAction> {
    let long_set: HashSet<&String> = longs.iter().collect();
    let short_set: HashSet<&String> = shorts.iter().collect();
    let mut actions = Vec::new();
    // 1) mevcut pozisyonlar: doğru yöndeyse tut, değilse kapat (flip'in kapatma yarısı dahil).
    for (sym, &is_long) in current {
        let keep = (is_long && long_set.contains(sym)) || (!is_long && short_set.contains(sym));
        if !keep {
            actions.push(XsAction::Close(sym.clone()));
        }
    }
    // 2) hedef yönde olmayan long/short'ları aç (yeni giriş + flip'in açma yarısı).
    for sym in longs {
        if current.get(sym) != Some(&true) {
            actions.push(XsAction::OpenLong(sym.clone()));
        }
    }
    for sym in shorts {
        if current.get(sym) != Some(&false) {
            actions.push(XsAction::OpenShort(sym.clone()));
        }
    }
    actions
}

impl Engine {
    /// Kesitsel adanmış mod cycle adımı: sepeti skorla → hedef kitap → aksiyonları infaz et.
    /// `execute_trade_cycle` per-sembol döngüsünden ÖNCE çağırır; sepet sembolleri normal döngüden
    /// HARİÇ tutulur (çift-yönetim yok). Mod kapalı/sepet yetersiz → no-op (sıfır regresyon).
    pub(crate) async fn process_xs_book(state: &Arc<Mutex<AppState>>) {
        let (xs, db_path, tuning) = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            let xs = st.brain.parameters.read().ok().map(|p| p.xs_live.clone());
            (xs, st.config.db_path.clone(), Arc::clone(&st.tuning))
        };
        let xs = match xs {
            Some(x) if x.enabled && x.top_k >= 1 && x.symbols.len() >= 2 * x.top_k => x,
            _ => return,
        };

        // 1) sepet sembollerinin son mumlarını yükle + momentum sinyali (eligibility kapısından geçenler).
        let mut signals: Vec<(String, f64)> = Vec::new();
        let mut candles_map: HashMap<String, Vec<Candle>> = HashMap::new();
        for sym in &xs.symbols {
            if !tuning.symbol_eligible_for_live(sym) {
                continue;
            }
            if let Some(c) = Self::cycle_load_candles(state, sym, &db_path, &xs.interval, &tuning) {
                let closes: Vec<f64> = c.iter().map(|k| k.close).collect();
                if let Some(s) = latest_signal(&closes, xs.lookback) {
                    signals.push((sym.clone(), s));
                    candles_map.insert(sym.clone(), c);
                }
            }
        }
        if signals.len() < 2 * xs.top_k {
            // Teşhis (throttle'sız ama nadir koşul): neden kitap kurulamadı (veri/eligibility/lookback).
            log::debug!("📐 kesitsel: yetersiz sinyal ({}/{} sembol geçerli, ≥{} gerek; interval={} lookback={}) → pas",
                signals.len(), xs.symbols.len(), 2 * xs.top_k, xs.interval, xs.lookback);
            return;
        }

        // 2) mevcut kitap: sepet sembollerinin açık pozisyon yönleri (symbol→is_long).
        let current: HashMap<String, bool> = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            st.finance.live_positions.read().ok()
                .map(|p| xs.symbols.iter().filter_map(|s| p.get(s).map(|m| (s.clone(), m.is_long))).collect())
                .unwrap_or_default()
        };
        let prev_long: HashSet<String> = current.iter().filter(|(_, l)| **l).map(|(s, _)| s.clone()).collect();
        let prev_short: HashSet<String> = current.iter().filter(|(_, l)| !**l).map(|(s, _)| s.clone()).collect();

        // 3) hedef kitap (backtest çekirdeği ile DRY) + saf aksiyon planı.
        let (longs, shorts) = crate::robot::backtester::xs_target_book(
            &signals, xs.top_k, xs.exit_buffer, xs.momentum, &prev_long, &prev_short);
        let actions = xs_plan_actions(&longs, &shorts, &current);
        if actions.is_empty() {
            return; // kitap zaten hedefte (no-trade band churn'ü emdi) → işlem yok
        }
        // Panel + DOSYA logu (seed görünürlük fix'i deseni): operatör `grep "kesitsel rebalance"` ile
        // her rebalance'ın kitabını (long/short + aksiyon sayısı) kalıcı izleyebilsin (push_state_log
        // ring'i 100 kayıtta kayar, stdout'a düşmez). [[project_edge_scan]] c06f780.
        push_state_log(state, format!(
            "📐 kesitsel rebalance: long={:?} short={:?} → {} aksiyon", longs, shorts, actions.len()));
        log::info!("📐 kesitsel rebalance: long={:?} short={:?} → {} aksiyon", longs, shorts, actions.len());

        // 4) infaz: önce kapanışlar (flat/flip), sonra açılışlar (plan bu sırada). open/close
        //    mevcut tek-nokta makinesini kullanır (muhasebe/log/cooldown ortak). Sizing şimdilik
        //    Kelly (faz 2: eşit-ağırlık). reentry_cooldown flip'i bir cycle geciktirebilir (kabul).
        for act in actions {
            match act {
                XsAction::Close(sym) => {
                    if let Some(c) = candles_map.get(&sym) {
                        Self::close_paper_position(state, &sym, c, ExitReason::StrategySignal).await;
                    }
                }
                XsAction::OpenLong(sym) => {
                    if let Some(c) = candles_map.get(&sym) {
                        Self::open_paper_position(state, &sym, &crate::core::types::Signal::Buy, c, "XS_MOMENTUM", None).await;
                    }
                }
                XsAction::OpenShort(sym) => {
                    if let Some(c) = candles_map.get(&sym) {
                        Self::open_paper_position(state, &sym, &crate::core::types::Signal::Sell, c, "XS_MOMENTUM", None).await;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod xs_live_tests {
    use super::*;

    #[test]
    fn latest_signal_basic() {
        assert!((latest_signal(&[100.0, 110.0, 121.0], 2).unwrap() - 0.21).abs() < 1e-9); // 121/100−1
        assert_eq!(latest_signal(&[100.0, 110.0], 5), None, "yetersiz mum → None");
    }

    fn s(v: &[&str]) -> Vec<String> { v.iter().map(|x| x.to_string()).collect() }

    #[test]
    fn plan_opens_new_book_when_flat() {
        let actions = xs_plan_actions(&s(&["A", "B"]), &s(&["E", "D"]), &HashMap::new());
        // hepsi flat → 2 long + 2 short açılış, kapanış yok.
        assert_eq!(actions.iter().filter(|a| matches!(a, XsAction::Close(_))).count(), 0);
        assert!(actions.contains(&XsAction::OpenLong("A".into())) && actions.contains(&XsAction::OpenShort("E".into())));
    }

    #[test]
    fn plan_holds_matching_and_closes_dropped() {
        let mut cur = HashMap::new();
        cur.insert("A".to_string(), true);  // long, hedefte long → tut
        cur.insert("X".to_string(), true);  // long ama hedefte yok → kapat
        cur.insert("E".to_string(), false); // short, hedefte short → tut
        let actions = xs_plan_actions(&s(&["A", "B"]), &s(&["E", "D"]), &cur);
        assert!(actions.contains(&XsAction::Close("X".into())), "düşen pozisyon kapanır");
        assert!(!actions.iter().any(|a| matches!(a, XsAction::OpenLong(x) if x=="A")), "A zaten long → tutulur");
        assert!(!actions.iter().any(|a| matches!(a, XsAction::Close(x) if x=="A")), "A kapanmaz");
        assert!(actions.contains(&XsAction::OpenLong("B".into())), "yeni B long açılır");
        assert!(actions.contains(&XsAction::OpenShort("D".into())), "yeni D short açılır");
    }

    #[test]
    fn plan_flip_closes_before_opens() {
        let mut cur = HashMap::new();
        cur.insert("A".to_string(), false); // şu an SHORT, hedef LONG → flip
        let actions = xs_plan_actions(&s(&["A"]), &s(&[]), &cur);
        let close_idx = actions.iter().position(|a| matches!(a, XsAction::Close(x) if x == "A"));
        let open_idx = actions.iter().position(|a| matches!(a, XsAction::OpenLong(x) if x == "A"));
        assert!(close_idx.is_some() && open_idx.is_some(), "flip = kapat + aç");
        assert!(close_idx < open_idx, "flip'te kapanış açılıştan ÖNCE gelir");
    }
}
