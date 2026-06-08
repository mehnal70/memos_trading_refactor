// src/robot/engines/master.rs - Master Engine Otonom İnfaz Merkezi
// Srivastava ATP - İşlevsel Çarklar Odası (Unified Master Engine - Final Safe Compilation)

use crate::prelude::*;
use super::base::{EngineConfig, TradingEngine};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::time::{sleep, Duration};

/// Anomali-tetikli ML retrain'in son fire epoch'u (saniye). 0 = hiç fire edilmedi.
/// `perform_anomaly_recovery` her cycle çağrılır ama bu cooldown sayesinde
/// ML trigger spam'i kapanır (default 300sn, `ANOMALY_ML_TRIGGER_COOLDOWN_SECS`).
static ANOMALY_ML_LAST_TRIGGER_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Sembol bazlı log throttle: `symbol|kind` → son emit epoch. Aynı sembol için
/// "DataIngest empty" log'u her cycle (500ms) tekrarlanmasın diye throttle'lanır
/// (örn. 300sn, env `LOG_DATAINGEST_COOLDOWN_SECS`). Map büyümesi sınırlı —
/// sembol sayısı orchestrator'la beraber tipik <100. Algoritma logger ile ortak
/// ([`crate::robot::infra::throttle::Throttle`]); burası process-global tek instance.
static LOG_THROTTLE: std::sync::OnceLock<crate::robot::infra::throttle::Throttle>
    = std::sync::OnceLock::new();

/// Phase C: TRAILING_STOP kapanışları sonrası 60sn olgunluk gözlem kuyruğu.
/// close_paper_position TrailingStop branch'ı buraya enqueue eder; periyodik
/// processor (spawn_trail_feedback_processor) olgunlaşmış kayıtları evalue edip
/// ParameterStore.record_trailing_outcome'a iletir. Static — AppState alanı eklemiyoruz.
static PENDING_TRAIL_OBS: std::sync::OnceLock<std::sync::Mutex<
    std::collections::VecDeque<crate::robot::parameters::PendingTrailObservation>
>> = std::sync::OnceLock::new();

/// Delisted sembol tespit sayacı: sembol → ardışık fetch hatası sayısı.
/// Eşik (DELISTED_DETECTION_THRESHOLD env, default 3) aşılınca
/// orchestrator'dan çıkarılır + live pozisyonu varsa force-close. Başarılı
/// fetch sayacı sıfırlar (geçici Binance hatasının yanlış pozitif olmaması için).
static DELISTED_FAIL_COUNTERS: std::sync::OnceLock<std::sync::Mutex<
    std::collections::HashMap<String, u32>
>> = std::sync::OnceLock::new();

fn delisted_counters() -> &'static std::sync::Mutex<std::collections::HashMap<String, u32>> {
    DELISTED_FAIL_COUNTERS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Ardışık fetch hatası sayacını artırır ve yeni sayıyı döner.
pub fn delisted_record_failure(symbol: &str) -> u32 {
    let mut guard = match delisted_counters().lock() {
        Ok(g) => g,
        Err(_) => return 0,
    };
    let counter = guard.entry(symbol.to_string()).or_insert(0);
    *counter += 1;
    *counter
}

/// Başarılı fetch sonrası sayacı sıfırlar (geçici hata yanlış pozitif vermesin).
pub fn delisted_record_success(symbol: &str) {
    if let Ok(mut guard) = delisted_counters().lock() {
        guard.remove(symbol);
    }
}

/// Sayacı sorgular (test ve teşhis için).
pub fn delisted_failure_count(symbol: &str) -> u32 {
    delisted_counters().lock().ok().and_then(|g| g.get(symbol).copied()).unwrap_or(0)
}

/// Eşik (env DELISTED_DETECTION_THRESHOLD, default 3). 0 verilirse
/// auto-detect kapanır (manuel müdahale için).
pub fn delisted_detection_threshold() -> u32 {
    std::env::var("DELISTED_DETECTION_THRESHOLD")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(3)
}

/// Delisted olarak mühürlenmiş semboller (purge sonrası). `symbol_eligible_for_live`
/// bunları reddeder → price_poll/cycle/download/hydrate (hepsi eligible kullanıyor)
/// artık yoklamaz → ApiError storm + "Recovering" sticky biter, sembol geri gelmez.
/// Oturum-içi (restart'ta price_poll ~threshold×poll_secs içinde yeniden tespit eder).
static DELISTED_SKIP: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>>
    = std::sync::OnceLock::new();

fn delisted_skip_set() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    DELISTED_SKIP.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// Sembolü delisted-skip setine ekle (purge_delisted_symbol çağırır).
pub fn mark_delisted_skip(symbol: &str) {
    if let Ok(mut s) = delisted_skip_set().lock() { s.insert(symbol.to_string()); }
}

/// Sembol delisted-skip setinde mi (eligibility gate okur).
pub fn is_delisted_skipped(symbol: &str) -> bool {
    delisted_skip_set().lock().ok().map(|s| s.contains(symbol)).unwrap_or(false)
}

/// 🗂️ Binance exchangeInfo sembol-statü registry'si (symbol → status, örn "TRADING"/
/// "BREAK"/"HALT"). Periyodik `run_symbol_status_refresh` doldurup DB'ye persist eder;
/// boot'ta `hydrate_symbol_status_from_db` DB'den yükler. `symbol_eligible_for_live`
/// okur → status != TRADING olan sembol (ALPACAUSDT BREAK gibi halted/delisted)
/// otoritatif dışlanır (heuristic'e gerek yok; de/re-list otomatik yansır).
static SYMBOL_STATUS: std::sync::OnceLock<
    std::sync::RwLock<std::collections::HashMap<String, String>>
> = std::sync::OnceLock::new();

fn symbol_status_map() -> &'static std::sync::RwLock<std::collections::HashMap<String, String>> {
    SYMBOL_STATUS.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()))
}

/// Registry'yi tam snapshot ile değiştir (exchangeInfo'dan ya da DB'den).
pub fn set_symbol_statuses(entries: &[(String, String)]) {
    if let Ok(mut m) = symbol_status_map().write() {
        m.clear();
        for (sym, status) in entries {
            m.insert(sym.clone(), status.clone());
        }
    }
}

/// Sembol işlem görebilir mi: status == "TRADING" YA DA registry'de yok (bilinmiyor →
/// izin ver; registry dolmadan startup kırılmasın). Yalnız açıkça TRADING-dışı → false.
pub fn is_symbol_tradeable(symbol: &str) -> bool {
    match symbol_status_map().read() {
        Ok(m) => m.get(symbol).map(|s| s == "TRADING").unwrap_or(true),
        Err(_) => true,
    }
}

/// Registry'deki sembol sayısı (teşhis/log).
pub fn symbol_status_registry_len() -> usize {
    symbol_status_map().read().map(|m| m.len()).unwrap_or(0)
}

fn trail_obs_queue() -> &'static std::sync::Mutex<
    std::collections::VecDeque<crate::robot::parameters::PendingTrailObservation>
> {
    PENDING_TRAIL_OBS.get_or_init(|| std::sync::Mutex::new(std::collections::VecDeque::new()))
}

pub fn enqueue_trail_observation(obs: crate::robot::parameters::PendingTrailObservation) {
    if let Ok(mut q) = trail_obs_queue().lock() {
        // Kuyruğu sınırla — pathological case'de 1000+ gözlem birikmesin.
        while q.len() >= 500 { q.pop_front(); }
        q.push_back(obs);
    }
}

pub fn log_throttle_should_emit(symbol: &str, kind: &'static str, cooldown_secs: u64) -> bool {
    let throttle = LOG_THROTTLE.get_or_init(crate::robot::infra::throttle::Throttle::new);
    throttle.should_emit(&format!("{}|{}", symbol, kind), cooldown_secs)
}

/// `state` kilidini kısa süreliğine alıp tek bir log satırı düşürür.
/// `if let Ok(mut st) = state.lock() { st.push_log(msg); }` boilerplate'ini DRY'lar.
/// Davranış birebir: kilit poisoned ise sessizce geçer (eski blok da öyleydi).
pub(crate) fn push_state_log(state: &Arc<Mutex<AppState>>, msg: String) {
    if let Ok(mut st) = state.lock() {
        st.push_log(msg);
    }
}

/// `state` kilidini kısa süreliğine alıp `trading_logger` Arc'ını clone'lar; varsa
/// `make_event()` ile üretilen olayı yazar. Kilit yalnız clone süresince tutulur,
/// IO (`log_event`) kilit DIŞINDA yapılır. Olay closure ile **lazy** üretilir →
/// logger yoksa format/alloc maliyeti hiç oluşmaz. loop_core'da 4 ayrı yerde
/// tekrarlanan `if let Some(logger) = state.lock().ok().and_then(|s| s.trading_logger.clone())`
/// bloğunu DRY'lar. (positions.rs zaten tutulan guard'dan clone+drop ettiği için
/// o kalıbı kasıtlı olarak bu helper'a almıyoruz — yeniden kilit gerekirdi.)
pub(crate) fn emit_trade_event(
    state: &Arc<Mutex<AppState>>,
    make_event: impl FnOnce() -> crate::robot::infra::logger::TradeEvent,
) {
    let logger = state.lock().ok().and_then(|s| s.trading_logger.clone());
    if let Some(logger) = logger {
        let _ = logger.log_event(&make_event());
    }
}

/// Env değişkenini `T`'ye parse eder; eksik/geçersizse `default`. Per-call (cache yok)
/// → env-mutasyonlu testlerle uyumlu. Kanonik `core::env::env_parse_or`'a delege eder
/// (üst-katman ergonomik adı korunur; mantık tek-nokta).
pub(crate) fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    crate::core::env::env_parse_or(key, default)
}

/// "Açık mı?" tarzı bayrak: yalnızca "1" / "true" (case-insensitive) → true; aksi
/// halde false. ALLOW_BIST, *_DISABLE, *_ENABLED gibi default-false toggle'lar için.
/// Kanonik `core::env::env_truthy`'ye delege eder.
pub(crate) fn env_truthy(key: &str) -> bool {
    crate::core::env::env_truthy(key)
}

/// Otonom interval-seçim aday TF'leri — TEK KAYNAK (jobs_backtest interval A/B + jobs_download
/// aday-TF çekimi ikisi de bunu kullanır; default tek yerde). WF-onaylı edge'in çoğu YÜKSEK
/// TF'lerde (1d/4h, [[project_edge_scan]] sweep bulgusu) → default ladder 15m,1h,4h,1d. Operatör
/// `AUTO_INTERVAL_CANDIDATES` ile override eder (örn. yalnız "1h,4h,1d"). Geçersiz/boş → default.
pub(crate) fn auto_interval_candidates() -> Vec<String> {
    let raw = std::env::var("AUTO_INTERVAL_CANDIDATES").unwrap_or_else(|_| "15m,1h,4h,1d".into());
    let v: Vec<String> = raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    if v.is_empty() { vec!["15m".into(), "1h".into(), "4h".into(), "1d".into()] } else { v }
}

/// Boot'ta env'den BİR KEZ okunan runtime ayar paketi. `AppState.tuning` alanında
/// `Arc` olarak tutulur; cycle hot-path'i her tur `getenv` yapmak yerine bu struct'tan
/// okur. Env hâlâ kaynaktır (operatör override eder) ama yalnızca `AppState::new`'da
/// okunur → per-cycle syscall + alloc yok. OnceLock global'inden farkı: her `AppState`
/// kendi env snapshot'ını alır → env-mutasyonlu testlerle de uyumlu.
#[derive(Debug, Clone)]
pub struct RuntimeTuning {
    /// Default'ta canlı feed'i OLMAYAN ama operatörün yine de cycle'a zorladığı
    /// borsalar (market-agnostik override). Default boş. Eligibility kararı
    /// `symbol_eligible_for_live` üzerinden tek noktada verilir → motorun hiçbir
    /// yerinde borsa-adı sabiti yok. Env: FORCE_LIVE_EXCHANGES=bist,kucoin
    /// (+ geriye uyum: ALLOW_BIST=1 → listeye Bist ekler).
    pub force_live_exchanges: Vec<crate::core::types::Exchange>,
    /// Tek emir komisyon oranı (entry+exit'te ayrı ayrı uygulanır). Default 0.001.
    /// Binance USDⓈ-M taker ≈ 0.0004 → gerçekçi live için `COMMISSION_RATE=0.0004`.
    pub commission_rate: f64,
    /// "Winner'ı koştur": açılışta sabit TP'yi çok uzağa it → kâr çıkışını ATR
    /// trailing yönetir. Env `LET_WINNERS_RUN`, default false. Backtest (bt_ab_exit_mgmt)
    /// yüksek zaman diliminde net pozitif, 1m'de marjinal gösterdi → opt-in.
    pub let_winners_run: bool,
    /// AUTO strateji seçimi: true ise rejim→tek-strateji lookup yerine adayları
    /// KENDİ resolve'lu paramlarıyla mini-backtest skoruna göre seçer (param_spec
    /// optimizasyonu seçime de girer). Volatile rejimde yine IDLE savunması. Env
    /// `STRATEGY_SELECT_EVAL`, default false (canlı seçim davranışı değişmez; opt-in
    /// + A/B). Bkz [[project_param_modularity]].
    pub strategy_select_eval: bool,
    /// StrategySignal kapanışı için min tutma süresi (sn). SL/TP/Trailing etkilenmez.
    pub min_holding_secs_strategy: i64,
    /// Açılışta entry↔candle sapma tavanı (%). 0 → price sanity guard kapalı.
    pub max_entry_price_dev_pct: f64,
    /// Candle "taze" eşiği (sn). Bundan eski candle.close fiyat referansı sayılmaz.
    pub candle_freshness_secs: i64,
    /// DataIngest empty/error log throttle penceresi (sn, sembol başına).
    pub log_dataingest_cooldown_secs: u64,
    /// Position-aligned RISK_BLOCK log throttle penceresi (sn, sembol başına).
    pub risk_block_log_cooldown_secs: u64,
    /// 🌐 Rejim bağlamı (Adım 1) cache TTL'i (sn). Cycle hot-path bu süre içinde
    /// rejimi yeniden hesaplamaz, RegimeContext cache'inden okur (seyrek tespit).
    /// 0 → her cycle yeniden hesapla (legacy per-cycle davranış). Default 900 (15 dk).
    pub regime_context_ttl_secs: u64,
    /// 🌐 GBT rejim yönünü beslesin mi (Adım 1). true → RegimeContext refresh'inde GBT
    /// skoru Trending yönünü belirler (seyrek, base-TF'de). Default true (hedef mimari).
    pub regime_gbt: bool,
    /// GBT'yi ESKİ gibi per-tick EDGE yolunda kullan (geri-dönüş anahtarı). true → her
    /// cycle predict_confidence çağrılır (eski davranış). Default false → GBT yalnız
    /// regime'de; edge saf matematik (momentum + yavaş brain.ml_confidence). [[regime_context]]
    pub gbt_edge_legacy: bool,
    /// 💱 Canlı girişi taker MARKET yerine maker LIMIT (POST_ONLY) ile aç. Default
    /// false (opt-in). Açıkken `place_smart_limit_entry` best_bid/ask'e katılır;
    /// dolmazsa `limit_entry_fallback_market`'a göre davranır. Paper yolu etkilenmez.
    /// Env: USE_LIMIT_ENTRY.
    pub use_limit_entry: bool,
    /// Maker giriş: tek denemede fill bekleme süresi (ms). Env LIMIT_ENTRY_TIMEOUT_MS, default 2000.
    pub limit_entry_timeout_ms: u64,
    /// Maker giriş: re-quote deneme sayısı. Env LIMIT_ENTRY_MAX_ATTEMPTS, default 3.
    pub limit_entry_max_attempts: u32,
    /// Maker giriş: spread guard (bps). Spread bunun üstündeyse o deneme atlanır.
    /// 0 → guard kapalı. Env LIMIT_ENTRY_MAX_SPREAD_BPS, default 50.
    pub limit_entry_max_spread_bps: f64,
    /// Maker giriş: N deneme dolmazsa taker MARKET'e düş (true) ya da trade'i atla
    /// (false). Env LIMIT_ENTRY_FALLBACK_MARKET, default true (trade kaçırma).
    pub limit_entry_fallback_market: bool,
    /// Maker dolumda uygulanan komisyon oranı (taker `commission_rate`'ten düşük).
    /// Default = commission_rate (ayrı set edilmezse fark yok). Env MAKER_COMMISSION_RATE.
    pub maker_commission_rate: f64,
    /// Pozisyon başına temel tahsisat = equity × bu oran × risk_appetite. Default 0.10
    /// (%10). Env BASE_ALLOC_FRACTION.
    pub base_alloc_fraction: f64,
    /// Tahsisat tabanı: dinamik Kelly ölçeği base_alloc × bu oranın altına inemez.
    /// Default 0.25. Env ALLOC_FLOOR_FRACTION.
    pub alloc_floor_fraction: f64,
    /// Kelly loss-streak penceresi: son N kapanışta zarar sayımı (dinamik ölçek girdisi).
    /// Default 5. Env KELLY_LOSS_STREAK_WINDOW.
    pub kelly_loss_streak_window: usize,
    /// Kelly istatistik penceresi: win_prob/avg_win/avg_loss için son N kapanış. Default 50.
    /// Env KELLY_STATS_WINDOW.
    pub kelly_stats_window: usize,
    /// TP fallback (%): ParameterStore trade_risk okunamazsa kullanılan son-çare. Default 3.0.
    /// Asıl kaynak ParameterStore; bu yalnız hata yolundadır. Env FALLBACK_TP_PCT.
    pub fallback_tp_pct: f64,
    /// SL fallback (%): ParameterStore trade_risk okunamazsa kullanılan son-çare. Default 1.5.
    /// Env FALLBACK_SL_PCT.
    pub fallback_sl_pct: f64,
    /// 🧊 Stale-feed kapısı: en yeni mum bu süreden (sn) eskiyse sembolde YENİ açılış
    /// yapılmaz — donuk/ölü feed'de phantom giriş koruması (BTCUSDC: mum günlerce eski,
    /// live_price donuk → sahte SL/TP + churn). Açık pozisyon yönetimi etkilenmez.
    /// `-1` (default) → AUTO: interval-farkında `2×interval` (forming bar yaşı
    /// [0,interval) olduğundan sabit eşik kısa interval'i gevşek/uzun interval'i sıkı
    /// bırakıyordu). `0` → kapalı. `>0` → operatör sabit override (sn). Env STALE_FEED_MAX_AGE_SECS.
    pub stale_feed_max_age_secs: i64,
    /// 📐 KAPALI-BAR GİRİŞ KARARI: live'da SQLite'ın son barı forming (oluşmakta olan) olur
    /// (REST kline endpoint'i forming barı da yazar) → strateji sinyali/edge/rejim tamamlanmamış bar
    /// üzerinde hesaplanır = backtest'le (kapalı-bar karar) train/serve skew + bar-içi repaint churn.
    /// true (default) → giriş-kararı penceresinden forming bar dışlanır (live=backtest). ÇIKIŞLAR
    /// ETKİLENMEZ (fleet.live_price ile anlık). 0/false/off → kapalı (escape). Env SIGNAL_CLOSED_BAR_ONLY.
    pub signal_closed_bar_only: bool,
    /// 🔢 Eş-zamanlı açık LONG pozisyon tavanı (0 → sınırsız). Env MAX_CONCURRENT_LONGS.
    /// open_paper_position'da uçuş-rezervasyonuyla atomik uygulanır (paralel açılış race'i).
    pub max_concurrent_longs: u32,
    /// 🔢 Eş-zamanlı açık SHORT pozisyon tavanı (0 → sınırsız). Env MAX_CONCURRENT_SHORTS.
    pub max_concurrent_shorts: u32,
    /// ⏳ Re-entry cooldown (sn): bir sembolde pozisyon kapandıktan sonra bu süre
    /// içinde YENİ açılış engellenir — churn/flip-flop (aç→kapa→hemen aç) koruması.
    /// 0 → kapalı (default, sıfır regresyon). Önerilen ~60. Env REENTRY_COOLDOWN_SECS.
    pub reentry_cooldown_secs: u64,
    /// 🎚️ Adaptif rejim Volatile eşiği: `Some(pctl)` → Volatile sınırı sembolün KENDİ
    /// rolling ATR% dağılımının `pctl` persentilinden türetilir (sabit ATR%>7 yerine
    /// sembol-relatif). `None` (default) → sabit eşik (mevcut davranış birebir). A/B
    /// (regime_gate harness) 1h'de Adaptive90'ı sabit/Off'a üstün buldu → opt-in.
    /// Env `REGIME_ADAPTIVE_PCTL` (örn. 0.90); (0,1) dışı/geçersiz → None. ADX bandları
    /// her durumda sabit kalır. [[project_autonomy_backlog]] #1.
    pub regime_adaptive_pctl: Option<f64>,
    /// 🧭 Rejim-yön teyidi: true → canlı açılış sinyali rejim yönüyle hizalı olmalı
    /// (long yalnız non-downtrend, short yalnız non-uptrend). Canlı motor zaten Sell→short
    /// açıyor (Both modu); bu kapı ters-trend girişlerini eler. Backtest A/B (1h, 8×5):
    /// LongOnly Σpnl% -441, Both -661 (DAHA KÖTÜ), RegimeDirectional +980 (PF 1.51) →
    /// teyit shorting'i kâra çeviren bileşen. Default false (opt-in; WF+slippage doğrulaması
    /// önce). Env `REGIME_DIRECTIONAL`. [[project_autonomy_backlog]] [[project_adaptive_regime]].
    pub regime_directional: bool,
    /// 🩺 Veri-sağlık kapısı (Faz 3): otonom işler (screener pool, backtest) bu eşikleri
    /// geçmeyen sembol×interval'i atlar → bayat/sparse veride yanıltıcı verdikt yok.
    /// `data_gate_enabled=false` (default) → kapı kapalı (sıfır regresyon). Açıkken
    /// metrikler ZATEN yüklü mumlardan hesaplanır (ek IO yok). Env DATA_GATE_ENABLED.
    pub data_gate_enabled: bool,
    /// Asgari bar sayısı (altı → sağlıksız). Env DATA_GATE_MIN_ROWS.
    pub data_min_rows: usize,
    /// İzin verilen maksimum gap% (mevcut veri gappy → gevşek default; Faz 2 sonrası
    /// sıkılaştırılır). Env DATA_GATE_MAX_GAP_PCT.
    pub data_max_gap_pct: f64,
    /// Son bar en fazla bu kadar BAR-yaşında olabilir (interval-farkında). Env DATA_GATE_MAX_STALE_BARS.
    pub data_max_stale_bars: u64,
    /// 🕳️ Derin tarihsel gap backfill (Faz 2 follow-up): son kayıt ŞİMDİDEN >1000 bar
    /// geride ise `fetch_latest` son-1000'i çekip aradaki deliği kalıcı bırakır. Açıkken
    /// download job `startTime`-pagination ile gap'in başından ileri doldurur (bounded).
    /// `true` (default) → veri-doğruluğu; gap'siz seriler backtest/canlı kararını sağlamlaştırır.
    /// Env `BACKFILL_ENABLED` (0/false/off → kapat).
    pub backfill_enabled: bool,
    /// Backfill'in bir cycle'da bir sembol×interval için yapacağı azami istek (her biri
    /// ≤1000 bar). Tavanı aşan gap sonraki cycle'larda yakınsar → API yükü sınırlı kalır.
    /// Env `BACKFILL_MAX_REQUESTS` (default 50 → ~50k bar/cycle).
    pub backfill_max_requests: usize,
    /// 🌱 Seed-strateji önceliği: bir sembolün edge_scan'le KEŞFEDİLMİŞ açık `symbol_strategy`
    /// ataması varsa o sembol O stratejiyle işlem görür; fırsatçı ScalpSwing alt-kanalı o turda
    /// PAS geçilir (yoksa ScalpSwing satır 327'de önce açıp keşfedilmiş edge'i baypas ediyordu →
    /// seed→strateji kanalı dekoratif kalıyordu). ScalpSwing edge'siz sembollerde avlanmaya devam
    /// eder. `true` (default) → keşfedilmiş alfa canlıda gerçekten ifade edilir. Env
    /// `SEED_STRATEGY_PRIORITY` (0/false/off → eski davranış: ScalpSwing her sembolde önce).
    /// [[project_edge_scan]] [[feedback_autonomy_first]].
    pub seed_strategy_priority: bool,
}

impl Default for RuntimeTuning {
    fn default() -> Self {
        Self {
            force_live_exchanges: Vec::new(),
            commission_rate: 0.001,
            let_winners_run: false,
            strategy_select_eval: false,
            min_holding_secs_strategy: 30,
            max_entry_price_dev_pct: 5.0,
            candle_freshness_secs: 300,
            log_dataingest_cooldown_secs: 300,
            risk_block_log_cooldown_secs: 60,
            regime_context_ttl_secs: 900,
            regime_gbt: true,
            gbt_edge_legacy: false,
            use_limit_entry: false,
            limit_entry_timeout_ms: 2000,
            limit_entry_max_attempts: 3,
            limit_entry_max_spread_bps: 50.0,
            limit_entry_fallback_market: true,
            maker_commission_rate: 0.001,
            base_alloc_fraction: 0.10,
            alloc_floor_fraction: 0.25,
            kelly_loss_streak_window: 5,
            kelly_stats_window: 50,
            fallback_tp_pct: 3.0,
            fallback_sl_pct: 1.5,
            stale_feed_max_age_secs: -1, // -1 = auto (interval-farkında: 2×interval). 0=kapalı, >0=sabit override.
            signal_closed_bar_only: true, // giriş kararı kapalı-barda (live=backtest); çıkışlar etkilenmez
            max_concurrent_longs: 5,   // dokümante edilen niyet (adaptive_params); 0=sınırsız
            max_concurrent_shorts: 2,
            reentry_cooldown_secs: 0,
            regime_adaptive_pctl: None,
            regime_directional: false,
            data_gate_enabled: false,   // opt-in (sıfır regresyon); açınca bayat/sparse elenir
            data_min_rows: 100,
            data_max_gap_pct: 90.0,     // gevşek (mevcut veri ~%22-49 gappy); Faz 2 sonrası sık
            data_max_stale_bars: 10,    // son bar en fazla 10×interval yaşında
            backfill_enabled: true,     // veri-doğruluğu (gap'i doldur); bounded → güvenli
            backfill_max_requests: 50,  // 50×1000 = ~50k bar/cycle; büyük gap cycle'larda yakınsar
            seed_strategy_priority: true, // keşfedilmiş edge'i olan sembol o stratejiyle işlem görür (ScalpSwing baypas etmez)
        }
    }
}

impl RuntimeTuning {
    /// Env'den okur; eksik/geçersiz değerlerde `Default` kullanılır. AppState::new'da
    /// bir kez çağrılır. Eski inline env okumalarıyla birebir aynı default'lar.
    pub fn from_env() -> Self {
        let d = Self::default();
        let commission_rate = {
            let v = env_parse("COMMISSION_RATE", d.commission_rate);
            if v.is_finite() && v >= 0.0 { v } else { d.commission_rate }
        };
        Self {
            force_live_exchanges: Self::parse_force_live_exchanges(),
            commission_rate,
            let_winners_run: env_truthy("LET_WINNERS_RUN"),
            strategy_select_eval: env_truthy("STRATEGY_SELECT_EVAL"),
            min_holding_secs_strategy: env_parse("MIN_HOLDING_SECS_STRATEGY", d.min_holding_secs_strategy),
            max_entry_price_dev_pct: env_parse("MAX_ENTRY_PRICE_DEVIATION_PCT", d.max_entry_price_dev_pct),
            candle_freshness_secs: env_parse("CANDLE_FRESHNESS_SECS", d.candle_freshness_secs),
            log_dataingest_cooldown_secs: env_parse("LOG_DATAINGEST_COOLDOWN_SECS", d.log_dataingest_cooldown_secs),
            risk_block_log_cooldown_secs: env_parse("RISK_BLOCK_LOG_COOLDOWN_SECS", d.risk_block_log_cooldown_secs),
            regime_context_ttl_secs: env_parse("REGIME_CONTEXT_TTL_SECS", d.regime_context_ttl_secs),
            // REGIME_GBT default açık (0/false → kapat). GBT_EDGE_LEGACY default kapalı.
            regime_gbt: !matches!(std::env::var("REGIME_GBT").ok().as_deref(), Some("0") | Some("false") | Some("off")),
            gbt_edge_legacy: env_truthy("GBT_EDGE_LEGACY"),
            use_limit_entry: env_truthy("USE_LIMIT_ENTRY"),
            limit_entry_timeout_ms: env_parse("LIMIT_ENTRY_TIMEOUT_MS", d.limit_entry_timeout_ms),
            limit_entry_max_attempts: env_parse("LIMIT_ENTRY_MAX_ATTEMPTS", d.limit_entry_max_attempts),
            limit_entry_max_spread_bps: env_parse("LIMIT_ENTRY_MAX_SPREAD_BPS", d.limit_entry_max_spread_bps),
            // Default açık (0/false/off → kapat) — opt-in maker'ın trade kaçırmaması için.
            limit_entry_fallback_market: !matches!(std::env::var("LIMIT_ENTRY_FALLBACK_MARKET").ok().as_deref(), Some("0") | Some("false") | Some("off")),
            maker_commission_rate: {
                let v = env_parse("MAKER_COMMISSION_RATE", commission_rate);
                if v.is_finite() && v >= 0.0 { v } else { commission_rate }
            },
            base_alloc_fraction: {
                let v = env_parse("BASE_ALLOC_FRACTION", d.base_alloc_fraction);
                if v.is_finite() && v > 0.0 { v } else { d.base_alloc_fraction }
            },
            alloc_floor_fraction: {
                let v = env_parse("ALLOC_FLOOR_FRACTION", d.alloc_floor_fraction);
                if v.is_finite() && v >= 0.0 { v } else { d.alloc_floor_fraction }
            },
            // Pencereler en az 1 (0 → istatistik anlamsız); geçersizde default.
            kelly_loss_streak_window: env_parse("KELLY_LOSS_STREAK_WINDOW", d.kelly_loss_streak_window).max(1),
            kelly_stats_window: env_parse("KELLY_STATS_WINDOW", d.kelly_stats_window).max(1),
            fallback_tp_pct: {
                let v = env_parse("FALLBACK_TP_PCT", d.fallback_tp_pct);
                if v.is_finite() && v > 0.0 { v } else { d.fallback_tp_pct }
            },
            fallback_sl_pct: {
                let v = env_parse("FALLBACK_SL_PCT", d.fallback_sl_pct);
                if v.is_finite() && v > 0.0 { v } else { d.fallback_sl_pct }
            },
            stale_feed_max_age_secs: env_parse("STALE_FEED_MAX_AGE_SECS", d.stale_feed_max_age_secs),
            // Default açık (0/false/off → kapat) — giriş kararı kapalı-barda; live=backtest, repaint churn yok.
            signal_closed_bar_only: !matches!(std::env::var("SIGNAL_CLOSED_BAR_ONLY").ok().as_deref(), Some("0") | Some("false") | Some("off")),
            max_concurrent_longs:  env_parse("MAX_CONCURRENT_LONGS", d.max_concurrent_longs),
            max_concurrent_shorts: env_parse("MAX_CONCURRENT_SHORTS", d.max_concurrent_shorts),
            reentry_cooldown_secs: env_parse("REENTRY_COOLDOWN_SECS", d.reentry_cooldown_secs),
            // (0,1) aralığında geçerli persentil → Some; aksi (set edilmemiş/0/1/NaN) → None (sabit eşik).
            regime_adaptive_pctl: std::env::var("REGIME_ADAPTIVE_PCTL").ok()
                .and_then(|s| s.parse::<f64>().ok())
                .filter(|p| p.is_finite() && *p > 0.0 && *p < 1.0),
            regime_directional: env_truthy("REGIME_DIRECTIONAL"),
            data_gate_enabled: env_truthy("DATA_GATE_ENABLED"),
            data_min_rows: env_parse("DATA_GATE_MIN_ROWS", d.data_min_rows),
            data_max_gap_pct: env_parse("DATA_GATE_MAX_GAP_PCT", d.data_max_gap_pct),
            data_max_stale_bars: env_parse("DATA_GATE_MAX_STALE_BARS", d.data_max_stale_bars),
            // Default açık (0/false/off → kapat) — gap doldurma veri-doğruluğu; bounded.
            backfill_enabled: !matches!(std::env::var("BACKFILL_ENABLED").ok().as_deref(), Some("0") | Some("false") | Some("off")),
            backfill_max_requests: env_parse("BACKFILL_MAX_REQUESTS", d.backfill_max_requests).max(1),
            // Default açık (0/false/off → kapat) — keşfedilmiş edge sembolde ScalpSwing'i baypas etmesin.
            seed_strategy_priority: !matches!(std::env::var("SEED_STRATEGY_PRIORITY").ok().as_deref(), Some("0") | Some("false") | Some("off")),
        }
    }

    /// Faz 3 veri-sağlık eşikleri (HealthThresholds). `data_gate_enabled` ile kapı açık mı
    /// ayrı kontrol edilir; bu yalnız eşik değerlerini paketler.
    pub fn health_thresholds(&self) -> crate::robot::data_pipeline::HealthThresholds {
        crate::robot::data_pipeline::HealthThresholds {
            min_rows: self.data_min_rows,
            max_gap_pct: self.data_max_gap_pct,
            max_stale_bars: self.data_max_stale_bars,
        }
    }

    /// Bu cycle için rejim eşikleri. `regime_adaptive_pctl` set ise sembolün kendi
    /// ATR% dağılımından adaptif Volatile sınırı; değilse sabit (`Default`) eşikler.
    /// Tek nokta: hem RegimeContext sınıflandırması hem Volatile→IDLE gate'i bunu kullanır.
    pub fn regime_thresholds(
        &self, candles: &[crate::core::types::Candle],
    ) -> crate::robot::logic::market_regime::RegimeThresholds {
        use crate::robot::logic::market_regime::{adaptive_thresholds, RegimeThresholds};
        match self.regime_adaptive_pctl {
            Some(p) => adaptive_thresholds(candles, p),
            None => RegimeThresholds::default(),
        }
    }

    /// FORCE_LIVE_EXCHANGES env'ini (virgül/boşluk ayraçlı) parse eder; bilinmeyen
    /// token'lar yok sayılır. Geriye uyum: ALLOW_BIST=1 → listeye Bist eklenir.
    fn parse_force_live_exchanges() -> Vec<crate::core::types::Exchange> {
        use crate::core::types::Exchange;
        let mut out: Vec<Exchange> = std::env::var("FORCE_LIVE_EXCHANGES")
            .unwrap_or_default()
            .split([',', ' ', ';'])
            .filter_map(Exchange::from_token)
            .collect();
        if env_truthy("ALLOW_BIST") && !out.contains(&Exchange::Bist) {
            out.push(Exchange::Bist); // legacy alias
        }
        out
    }

    /// Sembol canlı cycle'a (DataIngest + trade) uygun mu? Borsasının canlı feed'i
    /// varsa ya da operatör force ettiyse true. **Tüm market-bazlı dışlama tek nokta** —
    /// motorun başka hiçbir yerinde borsa-adı sabiti olmamalı.
    pub fn symbol_eligible_for_live(&self, symbol: &str) -> bool {
        // Delisted tespit edilmiş sembol → her zaman uygun değil (tüm seçim noktaları
        // bu gate'i kullandığı için tek noktada dışlanır).
        if is_delisted_skipped(symbol) { return false; }
        // exchangeInfo statü registry'si: açıkça TRADING-dışı (BREAK/HALT/delisted) → uygun değil.
        // (Bilinmeyen sembol registry dolmadan izinli; refresh job + boot hydrate doldurur.)
        if !is_symbol_tradeable(symbol) { return false; }
        let ex = crate::core::types::Exchange::classify(symbol);
        ex.has_live_feed() || self.force_live_exchanges.contains(&ex)
    }
}

/// `MAX_ENTRY_PRICE_DEVIATION_PCT` env'ini parse eder; geçersiz/eksikse 5.0 döner.
/// 0 verilirse price sanity guard kapanır (test ve operatör override için).
pub fn price_deviation_threshold_from_env() -> f64 {
    std::env::var("MAX_ENTRY_PRICE_DEVIATION_PCT")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(5.0)
}

/// Entry ile reference (DB son mum) arasındaki yüzde sapma. Reference <= 0 ise 0 döner.
pub fn price_deviation_pct(entry: f64, reference: f64) -> f64 {
    if reference > 0.0 {
        ((entry - reference).abs() / reference) * 100.0
    } else { 0.0 }
}

/// Price sanity sınırı aşıldı mı? `max_dev_pct <= 0` ise guard kapalı (false döner).
pub fn price_deviation_exceeds(entry: f64, reference: f64, max_dev_pct: f64) -> bool {
    if max_dev_pct <= 0.0 || reference <= 0.0 { return false; }
    price_deviation_pct(entry, reference) > max_dev_pct
}

/// Candle'ın "fresh" olduğunu doğrular: now - candle.timestamp <= eşik.
/// Eşik env `CANDLE_FRESHNESS_SECS` (default 300sn). DB candles günlerce
/// eski olabilir (BIST veya pasif sembol) → bu durumda candle.close referans
/// olarak güvenilmez; price sanity guard pas geçer.
pub fn candle_is_fresh(candle_ts: &chrono::DateTime<chrono::Utc>) -> bool {
    candle_is_fresh_within(candle_ts, env_parse("CANDLE_FRESHNESS_SECS", 300))
}

/// Saf freshness kontrolü: 0 <= (now - ts) <= max_age_secs. `candle_is_fresh` bunun
/// env-default sarmalayıcısı (testler default'la çağırıyor); hot-path RuntimeTuning'den
/// gelen cached eşikle bu saf sürümü çağırır → per-call getenv yok.
pub fn candle_is_fresh_within(candle_ts: &chrono::DateTime<chrono::Utc>, max_age_secs: i64) -> bool {
    let age_secs = (chrono::Utc::now() - *candle_ts).num_seconds();
    age_secs >= 0 && age_secs <= max_age_secs
}

// Projedeki gerçek tiplerin ve trait'lerin bağlanması için ön hazırlık (Agnostik Katman)
pub struct MLModel;
pub struct Monitor;
pub trait MarketRegimeDetector {}
pub trait StrategyLifecycleManager {}

pub struct Engine {
    pub config: EngineConfig,
    pub ml_model: Option<MLModel>,
    pub monitor: Option<Monitor>,
    pub last_cycle_at: std::time::Instant,
    pub regime_detector: Box<dyn MarketRegimeDetector + Send>,
    pub strategy_manager: Box<dyn StrategyLifecycleManager + Send>,
}

/// Bir pozisyonun kapanış sebebi — `ClosedTradeModel.exit_reason` string'i bu enum'dan üretilir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    StopLoss,
    TakeProfit,
    TrailingStop,
    Breakeven,
    StrategySignal,
}

impl ExitReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ExitReason::StopLoss        => "STOP_LOSS",
            ExitReason::TakeProfit      => "TAKE_PROFIT",
            ExitReason::TrailingStop    => "TRAILING_STOP",
            ExitReason::Breakeven       => "BREAKEVEN",
            ExitReason::StrategySignal  => "STRATEGY_SIGNAL",
        }
    }
    pub fn emoji(self) -> &'static str {
        match self {
            ExitReason::StopLoss        => "🔻",
            ExitReason::TakeProfit      => "🎯",
            ExitReason::TrailingStop    => "🪤",
            ExitReason::Breakeven       => "⚖️",
            ExitReason::StrategySignal  => "🏁",
        }
    }
}


#[cfg(test)]
mod tuning_tests {
    use super::RuntimeTuning;
    use crate::core::types::Candle;
    use chrono::{TimeZone, Utc};

    fn calm_candles(n: usize) -> Vec<Candle> {
        // Sıkı bantlı (düşük ATR%) yükseliş — adaptif sınır absolute 7.0'ın altına insin.
        (0..n).map(|i| {
            let c = 100.0 + i as f64;
            Candle {
                timestamp: Utc.timestamp_opt(1_700_000_000 + i as i64 * 60, 0).unwrap(),
                open: c, high: c + 0.5, low: c - 0.5, close: c,
                volume: 1.0, symbol: "T".into(), interval: "1m".into(),
            }
        }).collect()
    }

    #[test]
    fn regime_thresholds_none_is_default() {
        // regime_adaptive_pctl=None → sabit eşik (mevcut davranış birebir).
        let t = RuntimeTuning { regime_adaptive_pctl: None, ..RuntimeTuning::default() };
        assert_eq!(t.regime_thresholds(&calm_candles(80)), Default::default());
    }

    #[test]
    fn regime_thresholds_adaptive_is_symbol_relative() {
        // Some(pctl) → sembolün kendi dağılımından adaptif Volatile sınırı (< absolute 7.0).
        let t = RuntimeTuning { regime_adaptive_pctl: Some(0.90), ..RuntimeTuning::default() };
        let thr = t.regime_thresholds(&calm_candles(80));
        assert!(thr.atr_volatile_pct < 7.0 && thr.atr_volatile_pct > 0.0,
            "adaptif sınır absolute'ın altında olmalı: {}", thr.atr_volatile_pct);
        assert_eq!((thr.adx_ranging, thr.adx_trending), (20.0, 25.0), "ADX bandları sabit kalır");
    }
}

// ── Faz 1: impl Engine sorumluluk modüllerine bölündü (davranış birebir) ──
mod loop_core;
pub(crate) mod xs_live; // kesitsel relatif-güç adanmış mod (market-nötr long/short kitabı)
pub(crate) mod graded_entry; // kademeli giriş (XS hariç): rejime-göre pyramiding/averaging, HTF-teyitli
mod edge_regime;
mod infra_fleet;
mod fleet_tuners;
mod fleet_sync;
mod userdata;
mod positions;
mod positions_close;
mod jobs;
mod jobs_screener;
mod jobs_backtest;
mod jobs_download;
mod persistence;

// Edge-filter parse helper: backtest ve ML retrain job ortak kullanir (Faz 2 paylasimli tek-nokta).

/// `BACKTEST_EDGE_FILTER` env'ini giriş-kalitesi edge eşiğine çözer (#4). Backtest'in
/// canlı `process_symbol_cycle` edge hunisini aynalamasını ayarlar:
///   - unset → `default` (job'ın kararı; canlıyı aynalamak için Some(on_value))
///   - "0"/"false"/"off"/"none" → `None` (filtre yok, legacy: her Buy'da açılış)
///   - "1"/"true"/"on" → `Some(on_value)` (canlı cold-start eşiği = dynamic_edge_threshold(0))
///   - geçerli pozitif float → `Some(f)` (daha katı/gevşek elle eşik)
///   - geçersiz metin → `default` (sessiz fallback)
/// Serbest fonksiyon → env'siz unit-test edilebilir.
pub(crate) fn parse_edge_filter(
    raw: Option<String>, default: Option<f64>, on_value: f64,
) -> Option<f64> {
    match raw {
        None => default,
        Some(v) => {
            let v = v.trim();
            if v.eq_ignore_ascii_case("0") || v.eq_ignore_ascii_case("false")
                || v.eq_ignore_ascii_case("off") || v.eq_ignore_ascii_case("none") {
                None
            } else if v.eq_ignore_ascii_case("1") || v.eq_ignore_ascii_case("true")
                || v.eq_ignore_ascii_case("on") {
                Some(on_value)
            } else {
                match v.parse::<f64>() {
                    Ok(f) if f > 0.0 => Some(f),
                    Ok(_) => None,        // ≤0 → kapalı
                    Err(_) => default,    // çöp girdi → default
                }
            }
        }
    }
}

#[cfg(test)]
mod edge_filter_tests {
    use super::parse_edge_filter;

    #[test]
    fn unset_uses_default() {
        assert_eq!(parse_edge_filter(None, Some(0.20), 0.20), Some(0.20));
        assert_eq!(parse_edge_filter(None, None, 0.20), None);
    }

    #[test]
    fn off_tokens_disable() {
        for t in ["0", "false", "FALSE", "off", "none", "  off  "] {
            assert_eq!(parse_edge_filter(Some(t.into()), Some(0.20), 0.20), None, "token={t}");
        }
    }

    #[test]
    fn on_tokens_use_on_value() {
        for t in ["1", "true", "TRUE", "on"] {
            assert_eq!(parse_edge_filter(Some(t.into()), None, 0.20), Some(0.20), "token={t}");
        }
    }

    #[test]
    fn float_override() {
        assert_eq!(parse_edge_filter(Some("0.35".into()), None, 0.20), Some(0.35));
        assert_eq!(parse_edge_filter(Some("-1".into()), Some(0.20), 0.20), None); // ≤0 → kapalı
        assert_eq!(parse_edge_filter(Some("çöp".into()), Some(0.20), 0.20), Some(0.20)); // fallback
    }
}
