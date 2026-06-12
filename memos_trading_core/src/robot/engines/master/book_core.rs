// src/robot/engines/master/book_core.rs — KESİTSEL KİTAP ORTAK MOTORU (DRY çekirdek).
//
// Hem `xs_live` (momentum) hem `carry_live` (funding-carry) AYNI market-nötr long/short kitap
// makinesini paylaşır: sepeti bir per-sembol skalar sinyalle sırala → no-trade band ile hedef kitabı
// belirle → mevcut pozisyonları hedefe taşı (aç/kapat/flip). FARK YALNIZCA sinyalin nereden geldiği
// (fiyat momentumu mu, funding taşıması mı), hangi state alanına yazıldığı (BookKind) ve rebalance
// kadansı (`rebalance_min_bars`: momentum=1 → bar-başına; carry=14 → iki-haftalık). Devre-kesici /
// rejim-gate / take-profit / cooldown / maker-icra / stale-feed kapısı BİREBİR ortak.
// [[project_funding_carry]] [[project_xs_momentum]] [[feedback_modular_dry_perf]]
use super::*;
use std::collections::{HashMap, HashSet};

/// Kitap kimliği: hangi faktör + hangi state alanları + log/bildirim etiketleri. `BookConfig` (tunable
/// sayılar) bilgiden ayrı tutulur → motor ikisini birleştirir. Tag açılışta pozisyona mühürlenir
/// (maker komisyon muhasebesi + kapanış bununla tanır).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BookKind {
    Momentum,
    Carry,
    Blend, // iki-faktör z-score harman (Faz 2): tek market-nötr kitap, momentum⊕carry
}

impl BookKind {
    /// Strateji/trade_type etiketi (open/close_paper_position'a mühürlenir, maker icra bununla tanır).
    pub(crate) fn tag(&self) -> &'static str {
        match self {
            BookKind::Momentum => super::xs_live::XS_STRATEGY_TAG,
            BookKind::Carry => super::carry_live::CARRY_STRATEGY_TAG,
            BookKind::Blend => super::blend_live::BLEND_STRATEGY_TAG,
        }
    }
    /// Log/teşhis etiketi (operatör `grep` ile rebalance'ı izler).
    pub(crate) fn label(&self) -> &'static str {
        match self {
            BookKind::Momentum => "kesitsel",
            BookKind::Carry => "carry",
            BookKind::Blend => "harman",
        }
    }
    fn cb_notify_key(&self) -> &'static str {
        match self {
            BookKind::Momentum => "xs-circuit-breaker",
            BookKind::Carry => "carry-circuit-breaker",
            BookKind::Blend => "blend-circuit-breaker",
        }
    }
    fn tp_notify_key(&self) -> &'static str {
        match self {
            BookKind::Momentum => "xs-take-profit",
            BookKind::Carry => "carry-take-profit",
            BookKind::Blend => "blend-take-profit",
        }
    }
    /// Portföy-düzeyi devre kesici / take-profit cooldown'u (bu kitaba özel state alanı).
    fn cb_lock<'a>(
        &self, fin: &'a crate::robot::robotic_loop::FinanceVault,
    ) -> &'a std::sync::RwLock<Option<std::time::Instant>> {
        match self {
            BookKind::Momentum => &fin.xs_circuit_breaker_until,
            BookKind::Carry => &fin.carry_circuit_breaker_until,
            BookKind::Blend => &fin.blend_circuit_breaker_until,
        }
    }
    /// Son rank-rebalance edilen bar (bu kitaba özel state alanı) — kadans kapısı bununla sayar.
    fn bar_lock<'a>(
        &self, fin: &'a crate::robot::robotic_loop::FinanceVault,
    ) -> &'a std::sync::RwLock<Option<chrono::DateTime<chrono::Utc>>> {
        match self {
            BookKind::Momentum => &fin.xs_last_rebalance_bar,
            BookKind::Carry => &fin.carry_last_rebalance_bar,
            BookKind::Blend => &fin.blend_last_rebalance_bar,
        }
    }
}

/// Kitap motorunun TUNABLE konfigürasyonu — `XsLiveParams`/`CarryLiveParams`'ın paylaşılan görünümü.
/// Sinyal nasıl hesaplanır (lookback/funding penceresi) sarmalayıcının `signal_fn` closure'ında kalır;
/// burada yalnız sıralama/sizing/koruma sayıları + kadans var.
#[derive(Debug, Clone)]
pub(crate) struct BookConfig {
    pub symbols: Vec<String>,
    pub interval: String,
    /// Sinyal geriye-bakış (yalnız teşhis logu için; gerçek hesap signal_fn'de).
    pub lookback: usize,
    pub top_k: usize,
    pub exit_buffer: usize,
    /// true = en güçlü skoru LONG (momentum / düşük-funding); xs_target_book yön bayrağı.
    pub momentum: bool,
    pub position_pct: f64,
    pub leverage: f64,
    pub regime_gate: bool,
    pub max_drawdown_pct: f64,
    pub cb_cooldown_secs: u64,
    pub take_profit_pct: f64,
    pub tp_cooldown_secs: u64,
    /// ⏱️ KADANS: rank-rebalance en az bu kadar bar geçmeden tekrar tetiklenmez. Momentum=1
    /// (bar-başına, aynı-bar churn'ü kapar); carry=14 (iki-haftalık, fee'ye dayanıklı turnover).
    pub rebalance_min_bars: usize,
}

/// Kitap pozisyonu sizing+kaldıraç override'ı (`open_paper_position`'a Some olarak verilir): eşit-ağırlık
/// alloc (Kelly bypass, market-nötr 1/k dengesi) + SABİT kaldıraç (resolve_leverage rejim-değişkenini
/// bypass; anlamlılık L-invariant). None → mevcut Kelly+resolve.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BookSizing {
    pub alloc_frac: f64,
    pub leverage: f64,
}

/// Kitap aksiyonu (saf plan → imperatif infaz). flip = Close + Open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BookAction {
    OpenLong(String),
    OpenShort(String),
    Close(String), // →flat ya da flip'in kapatma yarısı (önce kapanışlar infaz edilir)
}

/// SAF: rejim kitabı bloklar mı? Kriz/yüksek-vol'da kesitsel yapı bozulur (korelasyon→1)
/// → HighVolatility'de kitap FLAT'a çekilir. Tek-kaynak koşul (testli).
pub(crate) fn regime_blocks_book(regime: crate::evolution::MarketRegime) -> bool {
    matches!(regime, crate::evolution::MarketRegime::HighVolatility)
}

/// SAF: açık bacakların toplam realize-olmamış PnL'i (USD) → equity yüzdesi. Portföy-düzeyi devre
/// kesici bunu `−max_drawdown_pct` ile karşılaştırır. equity<=0 → 0 (bölme koruması). Testli.
pub(crate) fn book_drawdown_pct(open_pnl_sum: f64, equity: f64) -> f64 {
    if equity <= 0.0 {
        return 0.0;
    }
    open_pnl_sum / equity * 100.0
}

/// SAF: iki bar-zaman-damgası arası geçen TAM bar sayısı (kadans kapısı için). interval_secs<=0 →
/// farklıysa 1, aynıysa 0 (interval bilinmiyorsa "bir bar farkı" yeterli). Testli.
pub(crate) fn bars_between(
    cur: chrono::DateTime<chrono::Utc>, last: chrono::DateTime<chrono::Utc>, interval_secs: i64,
) -> i64 {
    if interval_secs <= 0 {
        return if cur != last { 1 } else { 0 };
    }
    (cur - last).num_seconds() / interval_secs
}

/// SAF: kesitsel z-score normalizasyonu — her sembolün skoru (v−μ)/σ (popülasyon σ). σ≈0 (tüm
/// skorlar eşit) → hepsi 0.0 (bölme koruması; harmanda o faktör nötr katkı). İki faktörü AYNI ölçeğe
/// taşır → ağırlıklı harman anlamlı. Testli.
pub(crate) fn zscore_map(raw: &[(String, f64)]) -> HashMap<String, f64> {
    let n = raw.len();
    if n == 0 {
        return HashMap::new();
    }
    let mean = raw.iter().map(|(_, v)| *v).sum::<f64>() / n as f64;
    let var = raw.iter().map(|(_, v)| (*v - mean).powi(2)).sum::<f64>() / n as f64;
    let sd = var.sqrt();
    raw.iter()
        .map(|(s, v)| (s.clone(), if sd > 1e-12 { (*v - mean) / sd } else { 0.0 }))
        .collect()
}

/// SAF: iki-faktör KESİTSEL HARMAN skoru = w_mom·z(momentum) + w_carry·z(carry). Her faktör önce
/// kesitsel z-score'lanır (ortak ölçek), sonra ağırlıklı toplanır. Yalnız HER İKİ faktörde de skoru
/// olan semboller döner (eksik faktör → harman tanımsız). Tek market-nötr kitap için sıralanabilir skor.
/// Doğrulanmış optimal carry-ağırlıklı (w_carry≈0.6). [[project_funding_carry]] Testli.
pub(crate) fn blend_zscores(
    mom: &[(String, f64)], carry: &[(String, f64)], w_mom: f64, w_carry: f64,
) -> Vec<(String, f64)> {
    let zm = zscore_map(mom);
    let zc = zscore_map(carry);
    let mut out: Vec<(String, f64)> = Vec::new();
    for (sym, m) in &zm {
        if let Some(c) = zc.get(sym) {
            out.push((sym.clone(), w_mom * m + w_carry * c));
        }
    }
    out
}

/// SAF (testli): hedef long/short kitabı + mevcut pozisyon yönleri (symbol→is_long) → aksiyon listesi.
/// Hedefle aynı yön → no-op (tut). Yön değişimi → Close (flat) ya da Close+Open (flip). Kapanışlar
/// listede AÇILIŞLARDAN ÖNCE gelir → flip'te önce kapat sonra aç (infaz bu sırayı korur).
pub(crate) fn plan_actions(
    longs: &[String], shorts: &[String], current: &HashMap<String, bool>,
) -> Vec<BookAction> {
    let long_set: HashSet<&String> = longs.iter().collect();
    let short_set: HashSet<&String> = shorts.iter().collect();
    let mut actions = Vec::new();
    // 1) mevcut pozisyonlar: doğru yöndeyse tut, değilse kapat (flip'in kapatma yarısı dahil).
    for (sym, &is_long) in current {
        let keep = (is_long && long_set.contains(sym)) || (!is_long && short_set.contains(sym));
        if !keep {
            actions.push(BookAction::Close(sym.clone()));
        }
    }
    // 2) hedef yönde olmayan long/short'ları aç (yeni giriş + flip'in açma yarısı).
    for sym in longs {
        if current.get(sym) != Some(&true) {
            actions.push(BookAction::OpenLong(sym.clone()));
        }
    }
    for sym in shorts {
        if current.get(sym) != Some(&false) {
            actions.push(BookAction::OpenShort(sym.clone()));
        }
    }
    actions
}

impl Engine {
    /// Kesitsel kitap ORTAK cycle adımı (momentum + carry paylaşır): sepeti `signal_fn` ile skorla →
    /// hedef kitap → aksiyonları infaz et. `execute_trade_cycle` per-sembol döngüsünden ÖNCE çağrılır;
    /// sepet sembolleri normal döngüden HARİÇ tutulur (çift-yönetim yok). Sepet yetersiz → no-op.
    ///
    /// `signal_source(&candles_map) -> Vec<(sym, skor)>`: KESİTSEL sinyal üreteci. Tüm taze+eligible
    /// sembollerin mumlarını alır, kitaba özel skoru üretir (momentum=fiyat-getirisi, carry=−trailing
    /// funding, blend=z-score harmanı). Kesiti TÜM görür → z-score normalizasyonu (harman) mümkün.
    /// Skoru olmayan sembol vec'e GİRMEZ (veri/tazelik yetersiz → kitaptan dışlanır).
    pub(crate) async fn process_book<F>(
        state: &Arc<Mutex<AppState>>,
        cfg: &BookConfig,
        kind: BookKind,
        tuning: &Arc<RuntimeTuning>,
        db_path: &str,
        signal_source: F,
    ) where
        F: Fn(&HashMap<String, Vec<Candle>>) -> Vec<(String, f64)>,
    {
        let label = kind.label();
        let interval_secs =
            crate::robot::data_pipeline::DataNormalizer::parse_interval(&cfg.interval) as i64;
        // 🧊 STALE-FEED KAPISI: bayat-mumlu sembolü kitaba SOKMA (phantom-giriş koruması). Eşik
        // interval-farkında auto=2×bar (effective_stale_feed_age DRY); 0 → kapalı. [[project_stale_feed_gate]]
        let stale_bound =
            super::loop_core::effective_stale_feed_age(tuning.stale_feed_max_age_secs, interval_secs);

        // 1) sepet sembollerinin son mumlarını yükle (eligibility + stale-feed kapısından geçenler) →
        // candles_map (TAM mum: fiyat referansı + execution + cur_bar + regime proxy). Sonra kesitsel
        // sinyal üreteci tüm kesiti skorlar (z-score harmanı için kesit-bütününü görmek ŞART).
        let mut candles_map: HashMap<String, Vec<Candle>> = HashMap::new();
        for sym in &cfg.symbols {
            if !tuning.symbol_eligible_for_live(sym) {
                continue;
            }
            if let Some(c) = Self::cycle_load_candles(state, sym, db_path, &cfg.interval, tuning) {
                // Bayat feed → kitaptan DIŞLA (ne girer ne fantom flip yaratır).
                if stale_bound > 0 {
                    if let Some(last) = c.last() {
                        if !candle_is_fresh_within(&last.timestamp, stale_bound) {
                            let age = (chrono::Utc::now() - last.timestamp).num_seconds();
                            log::debug!("📐 {}: {} bayat mum ({}sn > {}sn) → kitaptan dışlandı (phantom giriş koruması)",
                                label, sym, age, stale_bound);
                            continue;
                        }
                    }
                }
                candles_map.insert(sym.clone(), c);
            }
        }
        let signals: Vec<(String, f64)> = signal_source(&candles_map);
        if signals.len() < 2 * cfg.top_k {
            log::debug!("📐 {}: yetersiz sinyal ({}/{} sembol geçerli, ≥{} gerek; interval={} lookback={}) → pas",
                label, signals.len(), cfg.symbols.len(), 2 * cfg.top_k, cfg.interval, cfg.lookback);
            return;
        }

        // 2) mevcut kitap + DEVRE KESİCİ girdileri (tek lock): açık pozisyon yönleri, açık bacakların
        //    toplam realize-olmamış PnL'i (mark-to-market), equity ve cooldown durumu.
        let (current, open_pnl_sum, equity, cb_until): (HashMap<String, bool>, f64, f64, Option<std::time::Instant>) = {
            let st = match state.lock() { Ok(s) => s, Err(_) => return };
            let (cur, pnl) = st.finance.live_positions.read().ok()
                .map(|p| {
                    let cur: HashMap<String, bool> = cfg.symbols.iter()
                        .filter_map(|s| p.get(s).map(|m| (s.clone(), m.is_long))).collect();
                    let pnl: f64 = cfg.symbols.iter().filter_map(|s| p.get(s)).map(|m| m.calculate_pnl()).sum();
                    (cur, pnl)
                })
                .unwrap_or_default();
            let cb_until = kind.cb_lock(&st.finance).read().ok().and_then(|c| *c);
            (cur, pnl, st.finance.equity, cb_until)
        };
        let prev_long: HashSet<String> = current.iter().filter(|(_, l)| **l).map(|(s, _)| s.clone()).collect();
        let prev_short: HashSet<String> = current.iter().filter(|(_, l)| !**l).map(|(s, _)| s.clone()).collect();

        // 3) hedef kitap (backtest çekirdeği ile DRY) + saf aksiyon planı.
        let (mut longs, mut shorts) = crate::robot::backtester::xs_target_book(
            &signals, cfg.top_k, cfg.exit_buffer, cfg.momentum, &prev_long, &prev_short);

        // PORTFÖY-DÜZEYİ DEVRE KESİCİ (per-bacak stop YERİNE — bacak stopu market-nötr yapıyı bozar):
        // açık kitabın toplam realize-olmamış zararı equity'nin max_drawdown_pct'ini aşarsa TÜM kitabı
        // flat'a çek + cb_cooldown_secs boyunca yeniden kurma. 0 → kapalı. [[project_xs_momentum]]
        let cb_now = std::time::Instant::now();
        let in_cb_cooldown = cb_until.map(|u| cb_now < u).unwrap_or(false);
        let dd_pct = book_drawdown_pct(open_pnl_sum, equity);
        if cfg.max_drawdown_pct > 0.0 && dd_pct <= -cfg.max_drawdown_pct {
            if let Ok(st) = state.lock() {
                if let Ok(mut cb) = kind.cb_lock(&st.finance).write() {
                    *cb = Some(cb_now + std::time::Duration::from_secs(cfg.cb_cooldown_secs));
                }
                if let Some(n) = st.notifier.as_ref() {
                    n.notify(kind.cb_notify_key(),
                        crate::robot::infra::telegram_notifier::Severity::Critical,
                        &format!("🔌 {} DEVRE KESİCİ: kitap DD %{:.2} → FLAT + {}sn cooldown",
                            label, dd_pct, cfg.cb_cooldown_secs));
                }
            }
            let msg = format!(
                "🔌 {} DEVRE KESİCİ: kitap DD %{:.2} ≤ −%{:.2} → FLAT + {}sn cooldown (felaket freni)",
                label, dd_pct, cfg.max_drawdown_pct, cfg.cb_cooldown_secs);
            push_state_log(state, msg.clone());
            log::warn!("{}", msg);
            longs.clear();
            shorts.clear();
        } else if in_cb_cooldown {
            // Cooldown sürüyor → kitabı flat tut (felaket/kâr-al sonrası aceleci yeniden-giriş churn'ü önlenir).
            longs.clear();
            shorts.clear();
        } else if cfg.take_profit_pct > 0.0 && dd_pct >= cfg.take_profit_pct {
            // 💰 PORTFÖY-DÜZEYİ TAKE-PROFIT (devre kesicinin KÂR-tarafı simetrik ikizi). [[project_xs_momentum]]
            if let Ok(st) = state.lock() {
                if let Ok(mut cb) = kind.cb_lock(&st.finance).write() {
                    *cb = Some(cb_now + std::time::Duration::from_secs(cfg.tp_cooldown_secs));
                }
                if let Some(n) = st.notifier.as_ref() {
                    n.notify(kind.tp_notify_key(),
                        crate::robot::infra::telegram_notifier::Severity::Info,
                        &format!("💰 {} TAKE-PROFIT: kitap kârı %{:.2} ≥ %{:.2} → FLAT + {}sn cooldown (kâr realize)",
                            label, dd_pct, cfg.take_profit_pct, cfg.tp_cooldown_secs));
                }
            }
            let msg = format!(
                "💰 {} TAKE-PROFIT: kitap kârı %{:.2} ≥ %{:.2} → FLAT + {}sn cooldown (kâr realize edildi)",
                label, dd_pct, cfg.take_profit_pct, cfg.tp_cooldown_secs);
            push_state_log(state, msg.clone());
            log::info!("{}", msg);
            longs.clear();
            shorts.clear();
        }

        // REJİM-GATE: market bellwether'ı (BTC, yoksa en derin sepet serisi) Volatile ise kitabı FLAT'a
        // çek. Tek-kaynak classify_regime [[feedback_autonomy_first]].
        if cfg.regime_gate {
            let proxy = candles_map.get("BTCUSDT")
                .or_else(|| candles_map.values().max_by_key(|c| c.len()));
            if let Some(pc) = proxy {
                if regime_blocks_book(Self::classify_regime(pc)) {
                    if !longs.is_empty() || !shorts.is_empty() {
                        log::info!("📐 {} REJİM-GATE: Volatile → kitap FLAT'a çekiliyor (kriz koruması)", label);
                    }
                    longs.clear();
                    shorts.clear();
                }
            }
        }

        // ⏱️ KADANS KAPISI: rank-rebalance son rebalance'tan beri < rebalance_min_bars bar geçtiyse ATLA
        // (kitabı tut). Momentum min_bars=1 → aynı-bar skip (bar-içi rank churn'ü kapanır); carry=14 →
        // iki-haftalık turnover (fee'ye dayanıklı). FORCE-FLAT (devre-kesici/rejim-gate/cooldown → kitap
        // boş) MUAF: hızlı felaket freni responsive kalmalı; bar da işaretlenmez → koruma kalkınca kitap
        // anında yeniden kurulur. [[project_xs_momentum]] [[project_funding_carry]]
        let cur_bar = candles_map.values().filter_map(|c| c.last()).map(|k| k.timestamp).max();
        let forced_flat = longs.is_empty() && shorts.is_empty();
        if !forced_flat {
            let skip = match state.lock() {
                Ok(st) => {
                    let last = kind.bar_lock(&st.finance).read().ok().and_then(|b| *b);
                    let due = match (cur_bar, last) {
                        (Some(cur), Some(l)) => bars_between(cur, l, interval_secs) >= cfg.rebalance_min_bars as i64,
                        (Some(_), None) => true,  // henüz hiç rebalance yok → kur
                        (None, _) => false,       // mevcut bar yok → kitabı tut (rebalance edemeyiz)
                    };
                    if due {
                        if let Ok(mut b) = kind.bar_lock(&st.finance).write() { *b = cur_bar; }
                        false
                    } else {
                        true
                    }
                }
                Err(_) => return,
            };
            if skip {
                return; // kadans dolmadı → kitabı tut (bar-içi/erken rebalance churn'ü yok)
            }
        }

        let actions = plan_actions(&longs, &shorts, &current);
        if actions.is_empty() {
            return; // kitap zaten hedefte (no-trade band churn'ü emdi) → işlem yok
        }
        // Panel + DOSYA logu: operatör `grep "{label} rebalance"` ile her rebalance'ın kitabını izleyebilsin.
        push_state_log(state, format!(
            "📐 {} rebalance: long={:?} short={:?} → {} aksiyon", label, longs, shorts, actions.len()));
        log::info!("📐 {} rebalance: long={:?} short={:?} → {} aksiyon", label, longs, shorts, actions.len());

        // 4) infaz: önce kapanışlar (flat/flip), sonra açılışlar (plan bu sırada). Eşit-ağırlık alloc +
        //    sabit kaldıraç override (BookSizing). reentry_cooldown flip'i bir cycle geciktirebilir (kabul).
        let sizing = BookSizing { alloc_frac: cfg.position_pct, leverage: cfg.leverage };
        let tag = kind.tag();
        for act in actions {
            match act {
                BookAction::Close(sym) => {
                    if let Some(c) = candles_map.get(&sym) {
                        Self::close_paper_position(state, &sym, c, ExitReason::StrategySignal).await;
                    }
                }
                BookAction::OpenLong(sym) => {
                    if let Some(c) = candles_map.get(&sym) {
                        Self::open_paper_position(state, &sym, &crate::core::types::Signal::Buy, c, tag, None, Some(sizing)).await;
                    }
                }
                BookAction::OpenShort(sym) => {
                    if let Some(c) = candles_map.get(&sym) {
                        Self::open_paper_position(state, &sym, &crate::core::types::Signal::Sell, c, tag, None, Some(sizing)).await;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod book_core_tests {
    use super::*;

    #[test]
    fn regime_gate_blocks_only_high_volatility() {
        use crate::evolution::MarketRegime::*;
        assert!(regime_blocks_book(HighVolatility), "kriz/yüksek-vol → kitap flat");
        for r in [StrongUptrend, WeakUptrend, Ranging, WeakDowntrend, StrongDowntrend, LowVolatility, Unknown] {
            assert!(!regime_blocks_book(r), "{:?} → kitap normal işler", r);
        }
    }

    #[test]
    fn book_drawdown_pct_basic() {
        assert!((book_drawdown_pct(-500.0, 10_000.0) - (-5.0)).abs() < 1e-9, "−$500/$10k = −%5");
        assert!((book_drawdown_pct(200.0, 10_000.0) - 2.0).abs() < 1e-9, "+$200/$10k = +%2");
        assert_eq!(book_drawdown_pct(-100.0, 0.0), 0.0, "equity 0 → bölme koruması");
        let dd = book_drawdown_pct(-800.0, 10_000.0); // −%8
        assert!(dd <= -5.0, "−%8, −%5 eşiğini tetiklemeli");
        assert!(!(dd <= -10.0), "−%8, −%10 eşiğini tetiklememeli");
        let tp = book_drawdown_pct(600.0, 10_000.0); // +%6
        assert!(tp >= 5.0, "+%6, +%5 take-profit eşiğini tetiklemeli");
        assert!(!(tp >= 10.0), "+%6, +%10 take-profit eşiğini tetiklememeli");
    }

    #[test]
    fn bars_between_cadence() {
        use chrono::{TimeZone, Utc};
        let t0 = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let day = 86_400i64;
        let t14 = t0 + chrono::Duration::days(14);
        let t13 = t0 + chrono::Duration::days(13);
        assert_eq!(bars_between(t0, t0, day), 0, "aynı bar → 0");
        assert_eq!(bars_between(t14, t0, day), 14, "14 gün → 14 bar");
        // Carry kadansı (min 14): 13 < 14 → henüz değil, 14 ≥ 14 → due.
        assert!(bars_between(t13, t0, day) < 14, "13 bar carry kadansını doldurmaz");
        assert!(bars_between(t14, t0, day) >= 14, "14 bar carry kadansını doldurur");
        // Momentum kadansı (min 1): farklı bar → ≥1, aynı bar → 0<1 skip.
        assert!(bars_between(t0 + chrono::Duration::days(1), t0, day) >= 1, "yeni bar momentum rebalance");
        assert!(bars_between(t0, t0, day) < 1, "aynı bar momentum skip");
        // interval bilinmiyorsa (0): farklıysa 1, aynıysa 0.
        assert_eq!(bars_between(t14, t0, 0), 1);
        assert_eq!(bars_between(t0, t0, 0), 0);
    }

    #[test]
    fn zscore_centers_and_scales() {
        let z = zscore_map(&[("A".into(), 1.0), ("B".into(), 2.0), ("C".into(), 3.0)]);
        // μ=2, σ=√(2/3)≈0.8165 → A=(1-2)/0.8165≈-1.2247, C≈+1.2247, B=0.
        assert!((z["B"]).abs() < 1e-9, "ortadaki z=0");
        assert!((z["A"] + z["C"]).abs() < 1e-9, "simetrik → toplam 0");
        assert!(z["C"] > z["A"], "büyük değer büyük z");
        // σ=0 (hepsi eşit) → tüm z=0 (bölme koruması).
        let z0 = zscore_map(&[("A".into(), 5.0), ("B".into(), 5.0)]);
        assert_eq!(z0["A"], 0.0);
        assert_eq!(z0["B"], 0.0);
    }

    #[test]
    fn blend_weights_two_factors_intersection() {
        // momentum: A güçlü, B zayıf; carry: A zayıf, B güçlü. Eşit ağırlıkta nötrleşmeli; carry-ağırlıkta B önde.
        let mom = vec![("A".into(), 2.0), ("B".into(), 1.0), ("C".into(), 1.5)];
        let car = vec![("A".into(), 1.0), ("B".into(), 2.0)]; // C carry'de YOK → harmana girmez
        let blended = blend_zscores(&mom, &car, 0.4, 0.6);
        let map: HashMap<String, f64> = blended.into_iter().collect();
        assert!(!map.contains_key("C"), "tek faktörlü sembol harmana girmez");
        assert_eq!(map.len(), 2, "yalnız iki faktörde de olan A,B");
        // carry-ağırlıklı (0.6) → carry'de güçlü B, momentum'da güçlü A'dan yüksek skorlu olmalı.
        assert!(map["B"] > map["A"], "carry-ağırlıklı harmanda carry-güçlü B önde");
    }

    fn s(v: &[&str]) -> Vec<String> { v.iter().map(|x| x.to_string()).collect() }

    #[test]
    fn plan_opens_new_book_when_flat() {
        let actions = plan_actions(&s(&["A", "B"]), &s(&["E", "D"]), &HashMap::new());
        assert_eq!(actions.iter().filter(|a| matches!(a, BookAction::Close(_))).count(), 0);
        assert!(actions.contains(&BookAction::OpenLong("A".into())) && actions.contains(&BookAction::OpenShort("E".into())));
    }

    #[test]
    fn plan_holds_matching_and_closes_dropped() {
        let mut cur = HashMap::new();
        cur.insert("A".to_string(), true);  // long, hedefte long → tut
        cur.insert("X".to_string(), true);  // long ama hedefte yok → kapat
        cur.insert("E".to_string(), false); // short, hedefte short → tut
        let actions = plan_actions(&s(&["A", "B"]), &s(&["E", "D"]), &cur);
        assert!(actions.contains(&BookAction::Close("X".into())), "düşen pozisyon kapanır");
        assert!(!actions.iter().any(|a| matches!(a, BookAction::OpenLong(x) if x=="A")), "A zaten long → tutulur");
        assert!(!actions.iter().any(|a| matches!(a, BookAction::Close(x) if x=="A")), "A kapanmaz");
        assert!(actions.contains(&BookAction::OpenLong("B".into())), "yeni B long açılır");
        assert!(actions.contains(&BookAction::OpenShort("D".into())), "yeni D short açılır");
    }

    #[test]
    fn plan_flip_closes_before_opens() {
        let mut cur = HashMap::new();
        cur.insert("A".to_string(), false); // şu an SHORT, hedef LONG → flip
        let actions = plan_actions(&s(&["A"]), &s(&[]), &cur);
        let close_idx = actions.iter().position(|a| matches!(a, BookAction::Close(x) if x == "A"));
        let open_idx = actions.iter().position(|a| matches!(a, BookAction::OpenLong(x) if x == "A"));
        assert!(close_idx.is_some() && open_idx.is_some(), "flip = kapat + aç");
        assert!(close_idx < open_idx, "flip'te kapanış açılıştan ÖNCE gelir");
    }
}
