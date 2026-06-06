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

/// Kesitsel pozisyonların strateji/trade_type etiketi — açılışta mühürlenir, kapanış + komisyon
/// muhasebesi bununla XS pozisyonunu tanır (maker icra: USE_LIMIT_ENTRY iken maker komisyon oranı).
pub(crate) const XS_STRATEGY_TAG: &str = "XS_MOMENTUM";

/// Kesitsel mod sizing+kaldıraç override'ı (`open_paper_position`'a Some olarak verilir): eşit-ağırlık
/// alloc (Kelly bypass, market-nötr 1/k dengesi) + SABİT kaldıraç (resolve_leverage rejim-değişkenini
/// bypass; anlamlılık L-invariant → marjinal nötr edge'de mütevazı L). None → mevcut Kelly+resolve.
#[derive(Debug, Clone, Copy)]
pub(crate) struct XsSizing {
    pub alloc_frac: f64,
    pub leverage: f64,
}

/// Kesitsel adanmış mod aksiyonu (saf plan → imperatif infaz). flip = Close + Open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum XsAction {
    OpenLong(String),
    OpenShort(String),
    Close(String), // →flat ya da flip'in kapatma yarısı (önce kapanışlar infaz edilir)
}

/// SAF: rejim XS kitabını bloklar mı? Kriz/yüksek-vol'da kesitsel momentum bozulur (korelasyon→1)
/// → HighVolatility'de kitap FLAT'a çekilir. Tek-kaynak koşul (testli).
pub(crate) fn regime_blocks_xs(regime: crate::evolution::MarketRegime) -> bool {
    matches!(regime, crate::evolution::MarketRegime::HighVolatility)
}

/// SAF: açık XS bacaklarının toplam realize-olmamış PnL'i (USD) → equity yüzdesi. Portföy-düzeyi
/// devre kesici bunu `−max_drawdown_pct` ile karşılaştırır (XS stopsuz → kitap-geneli felaket freni).
/// equity<=0 → 0 (bölme koruması). Testli.
pub(crate) fn xs_book_drawdown_pct(open_pnl_sum: f64, equity: f64) -> f64 {
    if equity <= 0.0 {
        return 0.0;
    }
    open_pnl_sum / equity * 100.0
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

        // 🧊 STALE-FEED KAPISI (XS): bayat-mumlu sembolü kitaba SOKMA. Normal yolun phantom-giriş
        // koruması (process_symbol_cycle, [[project_stale_feed_gate]]) XS'i KAPSAMIYORDU çünkü sepet
        // sembolleri o döngüden hariç + cycle_load_candles bayatlık filtrelemez. ONT −$82.96 artefaktı
        // kökü: bayat boot mumuyla (@0.0505) giriş → download tazeleyince fantom flip (@0.0463). Eşik
        // interval-farkında auto=2×bar (effective_stale_feed_age DRY, loop_core); 0 → kapalı (escape).
        let interval_secs =
            crate::robot::data_pipeline::DataNormalizer::parse_interval(&xs.interval) as i64;
        let stale_bound =
            super::loop_core::effective_stale_feed_age(tuning.stale_feed_max_age_secs, interval_secs);

        // 1) sepet sembollerinin son mumlarını yükle + momentum sinyali (eligibility + tazelik kapısından geçenler).
        let mut signals: Vec<(String, f64)> = Vec::new();
        let mut candles_map: HashMap<String, Vec<Candle>> = HashMap::new();
        for sym in &xs.symbols {
            if !tuning.symbol_eligible_for_live(sym) {
                continue;
            }
            if let Some(c) = Self::cycle_load_candles(state, sym, &db_path, &xs.interval, &tuning) {
                // Bayat feed → sinyal setinden DIŞLA (ne kitaba girer ne fantom flip yaratır).
                if stale_bound > 0 {
                    if let Some(last) = c.last() {
                        if !candle_is_fresh_within(&last.timestamp, stale_bound) {
                            let age = (chrono::Utc::now() - last.timestamp).num_seconds();
                            log::debug!("📐 kesitsel: {} bayat mum ({}sn > {}sn) → sinyalden dışlandı (phantom giriş koruması)",
                                sym, age, stale_bound);
                            continue;
                        }
                    }
                }
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

        // 2) mevcut kitap + DEVRE KESİCİ girdileri (tek lock): açık XS pozisyon yönleri (symbol→is_long),
        //    açık bacakların toplam realize-olmamış PnL'i (mark-to-market), equity ve cooldown durumu.
        let (current, open_pnl_sum, equity, cb_until): (HashMap<String, bool>, f64, f64, Option<std::time::Instant>) = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            let (cur, pnl) = st.finance.live_positions.read().ok()
                .map(|p| {
                    let cur: HashMap<String, bool> = xs.symbols.iter()
                        .filter_map(|s| p.get(s).map(|m| (s.clone(), m.is_long))).collect();
                    let pnl: f64 = xs.symbols.iter().filter_map(|s| p.get(s)).map(|m| m.calculate_pnl()).sum();
                    (cur, pnl)
                })
                .unwrap_or_default();
            let cb_until = st.finance.xs_circuit_breaker_until.read().ok().and_then(|c| *c);
            (cur, pnl, st.finance.equity, cb_until)
        };
        let prev_long: HashSet<String> = current.iter().filter(|(_, l)| **l).map(|(s, _)| s.clone()).collect();
        let prev_short: HashSet<String> = current.iter().filter(|(_, l)| !**l).map(|(s, _)| s.clone()).collect();

        // 3) hedef kitap (backtest çekirdeği ile DRY) + saf aksiyon planı.
        let (mut longs, mut shorts) = crate::robot::backtester::xs_target_book(
            &signals, xs.top_k, xs.exit_buffer, xs.momentum, &prev_long, &prev_short);

        // PORTFÖY-DÜZEYİ DEVRE KESİCİ (per-bacak stop YERİNE — XS rank-rebalance ile yönetilir, bacak
        // stopu market-nötr yapıyı bozar): açık kitabın toplam realize-olmamış zararı equity'nin
        // max_drawdown_pct'ini aşarsa TÜM kitabı flat'a çek + cb_cooldown_secs boyunca yeniden kurma.
        // 0 → kapalı (rejim-gate birincil koruma; bu hızlı/bağımsız felaket frenidir). [[project_xs_momentum]]
        let cb_now = std::time::Instant::now();
        let in_cb_cooldown = cb_until.map(|u| cb_now < u).unwrap_or(false);
        let dd_pct = xs_book_drawdown_pct(open_pnl_sum, equity);
        if xs.max_drawdown_pct > 0.0 && dd_pct <= -xs.max_drawdown_pct {
            // TETİK: cooldown mührü (tek lock); kitap flat → plan açık XS'i kapatır, yeni açmaz.
            if let Ok(st) = state.lock() {
                if let Ok(mut cb) = st.finance.xs_circuit_breaker_until.write() {
                    *cb = Some(cb_now + std::time::Duration::from_secs(xs.cb_cooldown_secs));
                }
            }
            let msg = format!(
                "🔌 kesitsel DEVRE KESİCİ: kitap DD %{:.2} ≤ −%{:.2} → FLAT + {}sn cooldown (felaket freni)",
                dd_pct, xs.max_drawdown_pct, xs.cb_cooldown_secs);
            push_state_log(state, msg.clone());
            log::warn!("{}", msg);
            longs.clear();
            shorts.clear();
        } else if in_cb_cooldown {
            // Cooldown sürüyor → kitabı flat tut (felaket sonrası aceleci yeniden-giriş churn'ü önlenir).
            longs.clear();
            shorts.clear();
        }

        // REJİM-GATE: market bellwether'ı (BTC, yoksa en derin sepet serisi) Volatile ise kitabı FLAT'a
        // çek (kriz/yüksek-vol'da kesitsel momentum bozulur). Hedef boşalınca plan mevcut XS'i kapatır,
        // yeni açmaz; rejim sakinleşince yeniden kurulur. Tek-kaynak classify_regime [[feedback_autonomy_first]].
        if xs.regime_gate {
            let proxy = candles_map.get("BTCUSDT")
                .or_else(|| candles_map.values().max_by_key(|c| c.len()));
            if let Some(pc) = proxy {
                if regime_blocks_xs(Self::classify_regime(pc)) {
                    if !longs.is_empty() || !shorts.is_empty() {
                        log::info!("📐 kesitsel REJİM-GATE: Volatile → kitap FLAT'a çekiliyor (kriz koruması)");
                    }
                    longs.clear();
                    shorts.clear();
                }
            }
        }

        // ⏱️ BAR-BAŞINA KADANS KAPISI: rank-tabanlı rebalance sinyal-barı başına BİR kez. Sinyal
        // (latest_signal) son mumun close'undan hesaplanır; 1d modunda son mum = devam eden bar →
        // close'u her veri tazelemesinde oynar → marjinal rank-k bacaklar bar-içi yer değiştirir →
        // close+reopen churn'ü (canlı kök: ~15dk'lık zarar zinciri = download tazeleme kadansı). Edge
        // WF-OOS + no-trade band ile BAR/1-REBALANCE kadansında doğrulandı; bar-içi işlem turnover'ı
        // edge'i yiyor. cur_bar (son mum open-time) bar içinde STABİL → cur_bar==last → rank rebalance
        // ATLA (kitabı tut). FORCE-FLAT (devre-kesici/rejim-gate/cooldown → longs&shorts boş) MUAF:
        // hızlı felaket freni responsive kalmalı; bar da işaretlenmez → koruma kalkınca kitap anında
        // yeniden kurulur. [[project_xs_momentum]] [[feedback_autonomy_first]]
        let cur_bar = candles_map.values().filter_map(|c| c.last()).map(|k| k.timestamp).max();
        let forced_flat = longs.is_empty() && shorts.is_empty();
        if !forced_flat {
            // Aynı bar mı? (skip) — değilse barı atomik işaretle (tek lock). Force-flat'ta işaretlenmez.
            let skip = match state.lock() {
                Ok(st) => {
                    let last = st.finance.xs_last_rebalance_bar.read().ok().and_then(|b| *b);
                    if cur_bar.is_some() && cur_bar == last {
                        true
                    } else {
                        if let Ok(mut b) = st.finance.xs_last_rebalance_bar.write() { *b = cur_bar; }
                        false
                    }
                }
                Err(_) => return,
            };
            if skip {
                return; // aynı bar → kitabı tut (bar-içi rank churn'ü yok)
            }
        }

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
        //    mevcut tek-nokta makinesini kullanır (muhasebe/log/cooldown ortak). Eşit-ağırlık alloc +
        //    sabit kaldıraç override (XsSizing). reentry_cooldown flip'i bir cycle geciktirebilir (kabul).
        let sizing = XsSizing { alloc_frac: xs.position_pct, leverage: xs.leverage };
        for act in actions {
            match act {
                XsAction::Close(sym) => {
                    if let Some(c) = candles_map.get(&sym) {
                        Self::close_paper_position(state, &sym, c, ExitReason::StrategySignal).await;
                    }
                }
                XsAction::OpenLong(sym) => {
                    if let Some(c) = candles_map.get(&sym) {
                        // EŞİT-AĞIRLIK alloc + SABİT kaldıraç (market-nötr 1/k dengesi + risk kontrolü).
                        Self::open_paper_position(state, &sym, &crate::core::types::Signal::Buy, c, XS_STRATEGY_TAG, None, Some(sizing)).await;
                    }
                }
                XsAction::OpenShort(sym) => {
                    if let Some(c) = candles_map.get(&sym) {
                        Self::open_paper_position(state, &sym, &crate::core::types::Signal::Sell, c, XS_STRATEGY_TAG, None, Some(sizing)).await;
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
    fn regime_gate_blocks_only_high_volatility() {
        use crate::evolution::MarketRegime::*;
        assert!(regime_blocks_xs(HighVolatility), "kriz/yüksek-vol → kitap flat");
        for r in [StrongUptrend, WeakUptrend, Ranging, WeakDowntrend, StrongDowntrend, LowVolatility, Unknown] {
            assert!(!regime_blocks_xs(r), "{:?} → kitap normal işler", r);
        }
    }

    #[test]
    fn book_drawdown_pct_basic() {
        assert!((xs_book_drawdown_pct(-500.0, 10_000.0) - (-5.0)).abs() < 1e-9, "−$500/$10k = −%5");
        assert!((xs_book_drawdown_pct(200.0, 10_000.0) - 2.0).abs() < 1e-9, "+$200/$10k = +%2");
        assert_eq!(xs_book_drawdown_pct(-100.0, 0.0), 0.0, "equity 0 → bölme koruması");
        // Devre kesici eşik mantığı: DD ≤ −threshold tetikler.
        let dd = xs_book_drawdown_pct(-800.0, 10_000.0); // −%8
        assert!(dd <= -5.0, "−%8, −%5 eşiğini tetiklemeli");
        assert!(!(dd <= -10.0), "−%8, −%10 eşiğini tetiklememeli");
    }

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
