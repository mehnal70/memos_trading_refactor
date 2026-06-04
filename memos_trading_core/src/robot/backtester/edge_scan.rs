// src/robot/backtester/edge_scan.rs — Gross-EDGE tarayıcı (tekrar koşulabilir araç çekirdeği).
//
// Amaç: DB'deki TARAMAYA-DEĞER tüm (exchange/market/symbol/interval) serilerinde strateji+param
// ızgarasını AYNI dürüst metodolojiyle (veri-sağlık kapısı → holdout %IS/%OOS → strateji havuzu →
// OOS pooled PF) backtest edip "hangi seri+strateji NET KÂRLI edge (PF≥1.0) taşıyor" sorusunu
// sayıyla yanıtlamak. `examples/edge_scan.rs` CLI bunu sarmalar; rapor JSON'a mühürlenip tekrar
// koşularda biriktirilebilir. Çekirdek burada (lib) → birim-testli + runtime'dan da çağrılabilir.
//
// Tek-kaynak yeniden kullanım ([[feedback_modular_dry_perf]]): list_series (reader), CandleHealth
// (Faz 3 sağlık kapısı), ParameterOptimizer/Backtester (holdout), default_registry (strateji havuzu),
// window_noise_floor_pct (canlı-temsili trailing). Yeni iş yalnız orkestrasyon + raporlama.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::persistence::reader::{list_series, read_candles_market, CandleSeriesRef};
use crate::robot::backtester::{Backtester, BacktestConfig, ParameterOptimizer, WfCrossCheck};
use crate::robot::data_pipeline::CandleHealth;
use crate::robot::parameters::window_noise_floor_pct;
use crate::robot::strategies::default_registry;

/// Edge-tarama konfigürasyonu (operatör-ayarı; CLI/örnek doldurur). Filtreler boşsa "hepsi".
#[derive(Debug, Clone)]
pub struct EdgeScanConfig {
    pub db_path: String,
    /// Yalnız bu market (örn. "futures"); None → tüm marketler.
    pub market_filter: Option<String>,
    /// Yalnız bu semboller (boş → hepsi). Büyük/küçük harf duyarsız eşleşir.
    pub symbol_filter: Vec<String>,
    /// Yalnız bu interval'ler (boş → hepsi).
    pub interval_filter: Vec<String>,
    /// Her seri için kaç bar okunacak (en yeni N).
    pub candle_limit: usize,
    pub capital: f64,
    /// Giriş edge hunisi eşiği (canlı ile aynı, 0.20).
    pub edge_min: f64,
    /// Breakeven RR (canlı çıkış modeli).
    pub breakeven_rr: f64,
    /// Holdout IS yüzdesi (70 → ilk %70 optimize, son %30 OOS ölç).
    pub holdout_is_pct: usize,
    /// PF'in güvenilir sayılması için asgari OOS işlem.
    pub min_trades: usize,
    /// Sağlık: taramaya-değer asgari bar (holdout+OOS için yeterli).
    pub min_rows: usize,
    /// Sağlık: izin verilen azami gap%.
    pub max_gap_pct: f64,
    /// Güvenli üst sınır: en fazla bu kadar seri taranır (bounded; en zengin seriler önce).
    pub max_series: usize,
    /// Grid: (başlangıç, bitiş, adım) — TP%, SL%, pozisyon-fraksiyonu.
    pub tp_grid: (f64, f64, f64),
    pub sl_grid: (f64, f64, f64),
    pub ps_grid: (f64, f64, f64),
    /// Komisyon (tek bacak; backtest simetrik uygular).
    pub commission_pct: f64,
    /// WF çapraz-kontrol pencere parametreleri (kazanan config'i çoklu OOS penceresinde dener).
    pub wf_is: usize,
    pub wf_oos: usize,
    pub wf_step: usize,
    /// `wf_robust` için: pooled PF≥1.0 VE kâr-eden-pencere oranı ≥ bu VE pencere ≥ wf_min_windows.
    pub wf_min_consistency: f64,
    pub wf_min_windows: usize,
}

impl Default for EdgeScanConfig {
    fn default() -> Self {
        Self {
            db_path: "data/trader.db".into(),
            market_filter: None,
            symbol_filter: Vec::new(),
            interval_filter: Vec::new(),
            candle_limit: 5000,
            capital: 10_000.0,
            edge_min: 0.20,
            breakeven_rr: 1.0,
            holdout_is_pct: 70,
            min_trades: 10,
            min_rows: 400,      // holdout(%70)+OOS(%30) anlamlı olsun
            max_gap_pct: 50.0,  // çok-gappy seri taramaya değmez
            max_series: 300,
            tp_grid: (2.0, 6.0, 2.0),
            sl_grid: (1.0, 3.0, 1.0),
            ps_grid: (0.2, 0.4, 0.1),
            commission_pct: 0.001,
            wf_is: 300,
            wf_oos: 100,
            wf_step: 100,
            wf_min_consistency: 0.5,  // pencerelerin en az yarısı kârlı olmalı
            wf_min_windows: 3,        // en az 3 işlemli pencere (tek-pencere fluke'u eler)
        }
    }
}

/// Bir serinin EN İYİ (OOS) sonucu — serde (JSON rapor + tekrar-koşu birikimi).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EdgeRow {
    pub exchange: String,
    pub market: String,
    pub symbol: String,
    pub interval: String,
    pub rows: usize,
    pub gap_pct: f64,
    pub stale_days: f64,
    pub best_strategy: String,
    pub take_profit_pct: f64,
    pub stop_loss_pct: f64,
    pub max_position_size: f64,
    pub trades: usize,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub expectancy: f64,
    pub sharpe: f64,
    /// Serinin günlük quote-volume ortalaması (USDT-yaklaşık turnover) — market-agnostik
    /// "majörlük"/likidite ölçütü. Seed bar'ı (min_daily_quote_volume) illikit-alt seri'leri
    /// (canlı feed'de purge edilen MYX/SIREN tipi) bununla eler. serde(default): eski raporda
    /// alan yok → 0.0 → qvol kapısı NO-OP (sıfır regresyon; yalnız taze rapor + env aktive eder).
    #[serde(default)]
    pub avg_daily_quote_volume: f64,
    /// PF≥1.0 VE işlem≥min_trades → net kârlı edge (tek-holdout).
    pub profitable: bool,
    /// Kazanan config'in çoklu-pencere WF çapraz-kontrolü (tek-holdout fluke'una karşı).
    #[serde(default)]
    pub wf: WfCrossCheck,
    /// WF-onaylı sağlam edge: pooled PF≥1.0 + tutarlılık ≥ eşik + yeterli pencere. Seed bunu arar.
    #[serde(default)]
    pub wf_robust: bool,
}

/// (market, interval) grubu için özet — toplu taramada "nerede edge var" survey'i.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GroupSummary {
    pub market: String,
    pub interval: String,
    /// Bu grupta taranan (sonuç üreten) seri.
    pub scanned: usize,
    /// PF≥1.0 net-kârlı seri.
    pub profitable: usize,
    /// Gruptaki en iyi PF + onu veren sembol/strateji.
    pub best_pf: f64,
    pub best_symbol: String,
    pub best_strategy: String,
}

/// Tüm tarama raporu (serde → JSON; tekrar koşularda karşılaştır/biriktir).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeScanReport {
    pub generated_at: String,
    pub db_path: String,
    pub market_filter: Option<String>,
    /// DB'deki aday seri sayısı (filtre sonrası).
    pub series_candidates: usize,
    /// Fiilen taranan (sağlık + veri geçen) seri.
    pub series_scanned: usize,
    /// Sağlık/veri yetersizliğinden atlanan.
    pub series_skipped: usize,
    /// PF≥1.0 net-kârlı seri sayısı.
    pub profitable_count: usize,
    /// (market, interval) kırılımlı özet — en iyi PF AZALAN sıralı.
    /// serde(default): şema evrimine tolerans (eski/özetsiz rapor da yüklenir → seed kırılmaz).
    #[serde(default)]
    pub summary: Vec<GroupSummary>,
    /// PF AZALAN sıralı satırlar.
    #[serde(default)]
    pub rows: Vec<EdgeRow>,
}

/// Satırlardan (market, interval) grup özeti çıkarır — en iyi PF AZALAN sıralı. Saf → testli.
pub fn summarize_by_group(rows: &[EdgeRow]) -> Vec<GroupSummary> {
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<(String, String), GroupSummary> = BTreeMap::new();
    for r in rows {
        let g = groups.entry((r.market.clone(), r.interval.clone())).or_insert(GroupSummary {
            market: r.market.clone(), interval: r.interval.clone(),
            scanned: 0, profitable: 0, best_pf: f64::NEG_INFINITY,
            best_symbol: String::new(), best_strategy: String::new(),
        });
        g.scanned += 1;
        if r.profitable { g.profitable += 1; }
        if r.profit_factor > g.best_pf {
            g.best_pf = r.profit_factor;
            g.best_symbol = r.symbol.clone();
            g.best_strategy = r.best_strategy.clone();
        }
    }
    let mut out: Vec<GroupSummary> = groups.into_values()
        .map(|mut g| { if !g.best_pf.is_finite() { g.best_pf = 0.0; } g })
        .collect();
    out.sort_by(|a, b| b.best_pf.partial_cmp(&a.best_pf).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Bir seriyi tara (holdout: IS'te optimize, OOS'ta ölç; strateji havuzunun en iyisi).
/// `candles` KRONOLOJİK (ASC) olmalı. Sağlık/veri yetersizse veya hiçbir strateji
/// sonuç vermezse `None`. Saf (DB okumaz) → birim-testli.
pub fn scan_one_series(cfg: &EdgeScanConfig, series: &CandleSeriesRef, candles: &[crate::core::types::Candle]) -> Option<EdgeRow> {
    let n = candles.len();
    if n < cfg.min_rows { return None; }
    let health = CandleHealth::from_candles(candles, &series.interval);
    if health.gap_pct > cfg.max_gap_pct { return None; }
    let stale_days = health.stale_secs as f64 / 86_400.0;
    // Likidite (majörlük) ölçütü — seri-seviyesi, strateji-bağımsız → döngü öncesi bir kez.
    let qvol = avg_daily_quote_volume(candles, &series.interval);

    // Holdout split.
    let split = (n * cfg.holdout_is_pct.min(95).max(50)) / 100;
    if split < 2 || n - split < 40 { return None; }
    let (is_slice, oos_slice) = candles.split_at(split);

    // Canlı-temsili trailing mult: target(0.7) / pencere_noise_floor%, clamp[1.5,30].
    let trail_mult = match window_noise_floor_pct(candles) {
        Some(nf) if nf > 0.0 => (0.7 / nf).clamp(1.5, 30.0),
        _ => 2.0,
    };

    let pool = default_registry().canonical_pool();
    let mut best: Option<EdgeRow> = None;
    for strat in &pool {
        let opt = ParameterOptimizer::new(series.symbol.clone(), series.interval.clone(), cfg.capital, strat.clone())
            .with_edge_min_score(Some(cfg.edge_min))
            .with_exit_model(Some(trail_mult), Some(cfg.breakeven_rr));
        let Ok(res) = opt.optimize_parallel(is_slice, cfg.tp_grid, cfg.sl_grid, cfg.ps_grid) else { continue; };
        let p = &res.best_parameters;
        // OOS ölçüm: IS'te bulunan en iyi param ile son dilimi koş (dürüst PF).
        let oos_cfg = BacktestConfig {
            symbol: series.symbol.clone(),
            interval: series.interval.clone(),
            initial_balance: cfg.capital,
            max_position_size: p.max_position_size,
            take_profit_pct: p.take_profit_pct,
            stop_loss_pct: p.stop_loss_pct,
            strategy_name: strat.clone(),
            commission_pct: cfg.commission_pct,
            edge_min_score: Some(cfg.edge_min),
            atr_trail_mult: Some(trail_mult),
            breakeven_at_rr: Some(cfg.breakeven_rr),
            ..Default::default()
        };
        let Ok(r) = Backtester::new(oos_cfg).run(oos_slice) else { continue; };
        let expectancy = if r.total_trades > 0 { r.total_pnl / r.total_trades as f64 } else { 0.0 };
        let cand = EdgeRow {
            exchange: series.exchange.clone(),
            market: series.market.clone(),
            symbol: series.symbol.clone(),
            interval: series.interval.clone(),
            rows: n,
            gap_pct: health.gap_pct,
            stale_days,
            best_strategy: strat.clone(),
            take_profit_pct: p.take_profit_pct,
            stop_loss_pct: p.stop_loss_pct,
            max_position_size: p.max_position_size,
            trades: r.total_trades,
            win_rate: r.win_rate,
            profit_factor: r.profit_factor,
            expectancy,
            sharpe: r.sharpe_ratio,
            avg_daily_quote_volume: qvol,
            profitable: r.profit_factor >= 1.0 && r.total_trades >= cfg.min_trades,
            wf: WfCrossCheck::default(),
            wf_robust: false,
        };
        if is_better(&cand, best.as_ref(), cfg.min_trades) { best = Some(cand); }
    }

    // ─── WF çoklu-pencere çapraz-kontrol (yalnız KAZANAN config için; tek-holdout fluke'una karşı).
    // Kazanan strateji+param'ı TÜM seride rolling OOS pencerelerinde dener → pooled PF + kâr-eden
    // pencere oranı. wf_robust: pooled PF≥1.0 + tutarlılık ≥ eşik + yeterli pencere. Seed bunu arar.
    if let Some(b) = best.as_mut() {
        let win_cfg = BacktestConfig {
            symbol: series.symbol.clone(),
            interval: series.interval.clone(),
            initial_balance: cfg.capital,
            max_position_size: b.max_position_size,
            take_profit_pct: b.take_profit_pct,
            stop_loss_pct: b.stop_loss_pct,
            strategy_name: b.best_strategy.clone(),
            commission_pct: cfg.commission_pct,
            edge_min_score: Some(cfg.edge_min),
            atr_trail_mult: Some(trail_mult),
            breakeven_at_rr: Some(cfg.breakeven_rr),
            ..Default::default()
        };
        let windows = crate::robot::backtester::walk_forward::wf_oos_windows(n, cfg.wf_is, cfg.wf_oos, cfg.wf_step);
        let cc = crate::robot::backtester::wf_cross_check(&win_cfg, candles, &windows);
        b.wf_robust = cc.pooled_pf >= 1.0
            && cc.windows >= cfg.wf_min_windows
            && cc.consistency() >= cfg.wf_min_consistency;
        b.wf = cc;
    }
    best
}

/// Aday daha mı iyi: önce "yeterli-işlemli" tercih (az-işlemli yüksek-PF fluke'unu ele),
/// sonra PF. interval_scan'daki seçim disiplininin tek-kaynak hali.
fn is_better(cand: &EdgeRow, best: Option<&EdgeRow>, min_trades: usize) -> bool {
    match best {
        None => true,
        Some(b) => {
            let (cand_ok, b_ok) = (cand.trades >= min_trades, b.trades >= min_trades);
            match (cand_ok, b_ok) {
                (true, false) => true,
                (false, true) => false,
                _ => cand.profit_factor > b.profit_factor,
            }
        }
    }
}

/// Serinin günlük quote-volume (USDT turnover) ortalaması — market-agnostik likidite/"majörlük"
/// ölçütü. `Candle.volume` ZATEN quote-volume'dür (Binance kline idx 7 = quote asset volume,
/// bkz. data_fetcher::binance:188) → mumun turnover'ı `volume`'ün KENDİSİDİR. (Eskiden `close×volume`
/// ile bir kez daha fiyatla çarpılıp ~fiyat kadar şişiyordu: BTCUSDT 30m 1e15 USDT/gün gibi imkânsız
/// değerler → likidite kapısı kullanılamazdı.) Interval ile GÜNLÜĞE normalize edilir → 1h/4h/1d
/// kıyaslanabilir. Majörler (BTC/ETH) yüksek, illikit-alt düşük döner. Saf → testli. [[feedback_market_agnostic]].
pub fn avg_daily_quote_volume(candles: &[crate::core::types::Candle], interval: &str) -> f64 {
    if candles.is_empty() { return 0.0; }
    let per_candle: f64 = candles.iter().map(|c| c.volume).sum::<f64>() / candles.len() as f64;
    let secs = crate::robot::data_pipeline::DataNormalizer::parse_interval(interval).max(1) as f64;
    per_candle * (86_400.0 / secs)
}

/// Bir CandleSeriesRef filtreyi geçiyor mu (sembol/interval/min_rows). Saf → testli.
fn series_passes(cfg: &EdgeScanConfig, s: &CandleSeriesRef) -> bool {
    if s.rows < cfg.min_rows { return false; }
    if !cfg.symbol_filter.is_empty()
        && !cfg.symbol_filter.iter().any(|f| f.eq_ignore_ascii_case(&s.symbol)) { return false; }
    if !cfg.interval_filter.is_empty()
        && !cfg.interval_filter.iter().any(|f| f == &s.interval) { return false; }
    true
}

/// Tam taramayı koşar (ilerleme bildirimsiz). Bkz. [`run_edge_scan_with_progress`].
pub fn run_edge_scan(cfg: &EdgeScanConfig) -> EdgeScanReport {
    run_edge_scan_with_progress(cfg, |_, _, _| {})
}

/// Tam taramayı koşar: serileri sırala/filtrele → her birini tara → PF azalan rapor + grup özeti.
/// Her seri ÖNCESİ `on_progress(idx, total, series)` çağrılır (uzun toplu koşuda görünürlük;
/// lib decoupled kalır, yazımı çağıran yapar). Seri döngüsü SIRALI (optimizer içte rayon →
/// çift-paralellik yok, bounded). DB hatası/boş seri sessiz atlanır (sayıma yansır).
pub fn run_edge_scan_with_progress<F>(cfg: &EdgeScanConfig, mut on_progress: F) -> EdgeScanReport
where
    F: FnMut(usize, usize, &CandleSeriesRef),
{
    let all = list_series(&cfg.db_path, cfg.market_filter.as_deref()).unwrap_or_default();
    let candidates: Vec<CandleSeriesRef> = all.into_iter()
        .filter(|s| series_passes(cfg, s))
        .take(cfg.max_series)
        .collect();
    let series_candidates = candidates.len();

    let mut rows: Vec<EdgeRow> = Vec::new();
    let mut skipped = 0usize;
    for (i, s) in candidates.iter().enumerate() {
        on_progress(i + 1, series_candidates, s);
        let candles = read_candles_market(&cfg.db_path, &s.symbol, &s.interval, &s.market, cfg.candle_limit)
            .unwrap_or_default();
        match scan_one_series(cfg, s, &candles) {
            Some(row) => rows.push(row),
            None => skipped += 1,
        }
    }
    rows.sort_by(|a, b| b.profit_factor.partial_cmp(&a.profit_factor).unwrap_or(std::cmp::Ordering::Equal));
    let profitable_count = rows.iter().filter(|r| r.profitable).count();
    let summary = summarize_by_group(&rows);

    EdgeScanReport {
        generated_at: chrono::Utc::now().to_rfc3339(),
        db_path: cfg.db_path.clone(),
        market_filter: cfg.market_filter.clone(),
        series_candidates,
        series_scanned: rows.len(),
        series_skipped: skipped,
        profitable_count,
        summary,
        rows,
    }
}

/// Seed robustluk barı: bir edge_scan satırını CANLIYA seed etmeye değer mi? edge_scan'ın
/// küçük-örneklem fluke uyarısını eler — `min_trades` (10 değil ~30) + `min_pf` (1.0 sınırı
/// değil ~1.2 margin) ile yalnız sağlam adaylar PRIOR olur; online backtest job sonra doğrular.
#[derive(Debug, Clone, Copy)]
pub struct SeedRobustness {
    pub min_trades: usize,
    pub min_pf: f64,
    /// ÜST sanity cap: PF bunu AŞARSA fluke kabul edilip ELENİR. 10-40 işlemlik küçük örneklemde
    /// aşırı PF (örn. illikit-alt RAVEUSDT 1h PF 61 ya da 18.52, tek-işlem 999) sürdürülebilir edge
    /// değil fat-tail kazadır — yüksek PF cazibesine kapılıp böyle adayı canlıya seed etme. KALİBRASYON
    /// (2026-06-04 sweep, 25 satır wf_robust): meşru likit majör edge'leri PF≤~5.5 (ZEC 4.42, XRP 4.63,
    /// SUI 5.45) kümelenir; flukeler kopuk üstte (RAVE 18.52) → 10.0 cap ikisini temiz ayırır (eski 25
    /// RAVE 18.52'yi kaçırıyordu). İllikidite ayrı eksen → `min_daily_quote_volume`. EDGE_SEED_MAX_PF
    /// ile gevşet (devre dışı: çok büyük değer ver).
    pub max_pf: f64,
    /// true → yalnız WF-onaylı (çoklu-pencere) satırlar seed'lenir (tek-holdout fluke'unu eler).
    pub require_wf_robust: bool,
    /// MAJÖR (likidite) tabanı: serinin günlük quote-volume'ü bunun ALTINDAysa seed'lenmez.
    /// Canlı feed'de purge edilen illikit-alt edge'leri (MYX/SIREN tipi) ELE → seed canlı-tradeable
    /// majörlere daralır. Default 0.0 = KAPALI (eski rapor + sıfır regresyon); EDGE_SEED_MIN_QVOL
    /// (USDT/gün) + TAZE rapor (avg_daily_quote_volume'lı) ile aktive. [[feedback_market_agnostic]].
    pub min_daily_quote_volume: f64,
}

impl Default for SeedRobustness {
    fn default() -> Self {
        Self { min_trades: 30, min_pf: 1.2, max_pf: 10.0, require_wf_robust: true, min_daily_quote_volume: 0.0 }
    }
}

/// Bir sembol için seed'lenen PLAN: (market, interval, strateji) ÜÇLÜSÜ. Edge bir (TF, strateji)
/// çiftidir (örn. BTCUSDT 1d-BB); ikisi BİRLİKTE taşınmalı — yoksa strateji yanlış TF'de
/// koşar (BB'yi 1m'de = edge yok). `market` de taşınır → seed yükleyici (store::from_env)
/// engine market'ına uymayan satırı eler (spot-only edge futures engine'e seed edilmesin).
/// ParameterStore.symbol_interval + symbol_strategy'ye yazılır.
#[derive(Debug, Clone, PartialEq)]
pub struct SeedEntry {
    pub market: String,
    pub interval: String,
    pub strategy: String,
}

/// Çoklu-iz seed: her sembol için robustluk barını geçen TÜM (market,TF,strateji) edge'lerini tutar
/// (tek-edge `seed_symbol_plan`'ın çoklu hali — Approach A çoklu-TF düzeneği [[project_edge_scan]]).
/// Aynı (market,TF,strateji) birden çok seride → en yüksek PF ile dedup; sonuç PF AZALAN sıralı,
/// `max_tracks` (≥1) ile bounded ([[feedback_modular_dry_perf]] pool döngüsünü sınırlı tut). Engine
/// bu izleri sırayla dener; tek-pozisyon invariantı korunur (flat'ken ilk tetikleyen açar). Yalnız
/// `profitable` + (require_wf_robust ise WF✓) + [min_pf,max_pf] + (min_qvol>0 ise likidite tabanı)
/// barını geçen satırlar. Market filtresi (engine market'ına uyum) çağıran (store) tarafında. Saf → testli.
pub fn seed_symbol_multi_plan(report: &EdgeScanReport, r: SeedRobustness, max_tracks: usize)
    -> HashMap<String, Vec<SeedEntry>>
{
    // sembol → ((market,interval,strategy) → (SeedEntry, en iyi PF)) ile dedup.
    type EdgeKey = (String, String, String);
    type SymEdges = HashMap<EdgeKey, (SeedEntry, f64)>;
    let mut acc: HashMap<String, SymEdges> = HashMap::new();
    for row in &report.rows {
        if !row.profitable || row.trades < r.min_trades { continue; }
        if row.profit_factor < r.min_pf || row.profit_factor > r.max_pf { continue; }
        if r.require_wf_robust && !row.wf_robust { continue; }
        // MAJÖR kapısı: illikit-alt seri (canlı feed'de purge edilen) seed'lenmesin. >0 ise aktif.
        if r.min_daily_quote_volume > 0.0 && row.avg_daily_quote_volume < r.min_daily_quote_volume { continue; }
        let key = (row.market.clone(), row.interval.clone(), row.best_strategy.clone());
        let e = acc.entry(row.symbol.clone()).or_default()
            .entry(key)
            .or_insert_with(|| (SeedEntry { market: String::new(), interval: String::new(), strategy: String::new() }, f64::NEG_INFINITY));
        if row.profit_factor > e.1 {
            *e = (SeedEntry { market: row.market.clone(), interval: row.interval.clone(), strategy: row.best_strategy.clone() }, row.profit_factor);
        }
    }
    acc.into_iter().map(|(sym, m)| {
        let mut v: Vec<(SeedEntry, f64)> = m.into_values().collect();
        // PF azalan (yüksek-PF "çapa" edge önce; eşitlikte deterministik (TF,strateji) sıralaması).
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| (a.0.interval.as_str(), a.0.strategy.as_str()).cmp(&(b.0.interval.as_str(), b.0.strategy.as_str()))));
        v.truncate(max_tracks.max(1));
        (sym, v.into_iter().map(|(e, _)| e).collect())
    }).collect()
}

/// Bir rapordan robustluk barını geçen `sembol → SeedEntry(interval, strateji)` planı (tek-edge,
/// sembol başına EN YÜKSEK PF). `seed_symbol_multi_plan(.., 1)`'e delege eder (TEK KAYNAK; filtre
/// mantığı tek yerde). Saf → testli.
pub fn seed_symbol_plan(report: &EdgeScanReport, r: SeedRobustness) -> HashMap<String, SeedEntry> {
    seed_symbol_multi_plan(report, r, 1).into_iter()
        .filter_map(|(sym, mut tracks)| if tracks.is_empty() { None } else { Some((sym, tracks.remove(0))) })
        .collect()
}

/// Çoklu-iz seed için sembol başına azami iz (TF,strateji) sayısı — pool döngüsü bounded kalsın
/// (track başına ayrı mum yükü). EDGE_SEED_MAX_TRACKS ile aşılır.
pub const SEED_MAX_TRACKS_DEFAULT: usize = 3;

/// JSON rapor DOSYASINDAN seed planı (dosya yok/parse hatası → boş, sessiz). Boot'ta
/// `EDGE_SEED_REPORT` env'i bu yola işaret ederse symbol_interval + symbol_strategy'ye yüklenir.
pub fn seed_symbol_plan_from_file(path: &str, r: SeedRobustness) -> HashMap<String, SeedEntry> {
    let Ok(txt) = std::fs::read_to_string(path) else { return HashMap::new() };
    match serde_json::from_str::<EdgeScanReport>(&txt) {
        Ok(report) => seed_symbol_plan(&report, r),
        Err(_) => HashMap::new(),
    }
}

/// JSON rapor DOSYASINDAN çoklu-iz seed planı (dosya yok/parse hatası → boş, sessiz).
/// `EDGE_SEED_MULTI_TF` açıkken boot'ta symbol_tracks'e yüklenir [[project_edge_scan]].
pub fn seed_symbol_multi_plan_from_file(path: &str, r: SeedRobustness, max_tracks: usize)
    -> HashMap<String, Vec<SeedEntry>>
{
    let Ok(txt) = std::fs::read_to_string(path) else { return HashMap::new() };
    match serde_json::from_str::<EdgeScanReport>(&txt) {
        Ok(report) => seed_symbol_multi_plan(&report, r, max_tracks),
        Err(_) => HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::Candle;
    use chrono::{TimeZone, Utc};

    fn series_ref(sym: &str, iv: &str, rows: usize) -> CandleSeriesRef {
        CandleSeriesRef { exchange: "binance".into(), market: "futures".into(),
            symbol: sym.into(), interval: iv.into(), rows }
    }

    #[test]
    fn series_passes_respects_filters_and_min_rows() {
        let cfg = EdgeScanConfig { min_rows: 400, symbol_filter: vec!["BTCUSDT".into()],
            interval_filter: vec!["1h".into()], ..Default::default() };
        assert!(series_passes(&cfg, &series_ref("BTCUSDT", "1h", 500)));
        assert!(!series_passes(&cfg, &series_ref("BTCUSDT", "1h", 100)), "min_rows altı elenir");
        assert!(!series_passes(&cfg, &series_ref("ETHUSDT", "1h", 500)), "sembol filtresi");
        assert!(!series_passes(&cfg, &series_ref("BTCUSDT", "15m", 500)), "interval filtresi");
        // Büyük/küçük harf duyarsız sembol eşleşmesi.
        let cfg2 = EdgeScanConfig { symbol_filter: vec!["btcusdt".into()], ..Default::default() };
        assert!(series_passes(&cfg2, &series_ref("BTCUSDT", "1h", 500)));
    }

    #[test]
    fn is_better_prefers_sufficient_trades_then_pf() {
        let mk = |trades: usize, pf: f64| EdgeRow {
            exchange: "b".into(), market: "f".into(), symbol: "S".into(), interval: "1h".into(),
            rows: 500, gap_pct: 0.0, stale_days: 0.0, best_strategy: "X".into(),
            take_profit_pct: 4.0, stop_loss_pct: 2.0, max_position_size: 0.3,
            trades, win_rate: 0.5, profit_factor: pf, expectancy: 0.0, sharpe: 0.0,
            avg_daily_quote_volume: 1e9, profitable: false,
            wf: WfCrossCheck::default(), wf_robust: false,
        };
        // Yeterli-işlemli düşük-PF, az-işlemli yüksek-PF'i yener (fluke koruması).
        let suff_low = mk(20, 1.1);
        let few_high = mk(3, 5.0);
        assert!(is_better(&suff_low, Some(&few_high), 10));
        assert!(!is_better(&few_high, Some(&suff_low), 10));
        // İkisi de yeterli → PF kazanır.
        assert!(is_better(&mk(20, 1.5), Some(&mk(20, 1.2)), 10));
    }

    #[test]
    fn summarize_by_group_counts_and_ranks() {
        let mk = |market: &str, iv: &str, sym: &str, pf: f64, profitable: bool| EdgeRow {
            exchange: "b".into(), market: market.into(), symbol: sym.into(), interval: iv.into(),
            rows: 500, gap_pct: 0.0, stale_days: 0.0, best_strategy: "X".into(),
            take_profit_pct: 4.0, stop_loss_pct: 2.0, max_position_size: 0.3,
            trades: 20, win_rate: 0.5, profit_factor: pf, expectancy: 0.0, sharpe: 0.0,
            avg_daily_quote_volume: 1e9, profitable,
            wf: WfCrossCheck::default(), wf_robust: false,
        };
        let rows = vec![
            mk("futures", "1h", "A", 1.5, true),
            mk("futures", "1h", "B", 0.8, false),
            mk("futures", "15m", "C", 2.0, true),
            mk("spot", "1h", "D", 0.5, false),
        ];
        let s = summarize_by_group(&rows);
        assert_eq!(s.len(), 3, "3 grup (futures1h, futures15m, spot1h)");
        // En iyi PF azalan → futures/15m (2.0) başta.
        assert_eq!((s[0].market.as_str(), s[0].interval.as_str()), ("futures", "15m"));
        assert!((s[0].best_pf - 2.0).abs() < 1e-9 && s[0].best_symbol == "C");
        let f1h = s.iter().find(|g| g.market == "futures" && g.interval == "1h").unwrap();
        assert_eq!((f1h.scanned, f1h.profitable), (2, 1), "futures/1h: 2 taranan, 1 kârlı");
        assert!((f1h.best_pf - 1.5).abs() < 1e-9 && f1h.best_symbol == "A");
    }

    #[test]
    fn report_json_roundtrips_through_seed_loader() {
        // serde round-trip: rapor yaz → seed_symbol_strategy_from_file oku (gerçek dosya dikişi).
        let report = EdgeScanReport {
            generated_at: "t".into(), db_path: "d".into(), market_filter: Some("futures".into()),
            series_candidates: 1, series_scanned: 1, series_skipped: 0, profitable_count: 1,
            summary: vec![],
            rows: vec![EdgeRow {
                exchange: "binance".into(), market: "futures".into(), symbol: "BTCUSDT".into(),
                interval: "1h".into(), rows: 5000, gap_pct: 0.0, stale_days: 0.0,
                best_strategy: "ICT_COMPOSITE".into(), take_profit_pct: 6.0, stop_loss_pct: 1.0,
                max_position_size: 0.3, trades: 40, win_rate: 0.4, profit_factor: 1.5,
                expectancy: 5.0, sharpe: 0.5, avg_daily_quote_volume: 1e9, profitable: true,
                wf: WfCrossCheck { windows: 6, profitable_windows: 5, pooled_pf: 1.4, trades: 40 },
                wf_robust: true,
            }],
        };
        let dir = std::env::temp_dir();
        let path = dir.join(format!("edge_roundtrip_{}.json", std::process::id()));
        let p = path.to_string_lossy().to_string();
        std::fs::write(&p, serde_json::to_string_pretty(&report).unwrap()).unwrap();
        let seed = seed_symbol_plan_from_file(&p, SeedRobustness::default());
        let _ = std::fs::remove_file(&p);
        let e = seed.get("BTCUSDT").expect("BTCUSDT seed'lenmeli");
        assert_eq!((e.market.as_str(), e.interval.as_str(), e.strategy.as_str()),
            ("futures", "1h", "ICT_COMPOSITE"),
            "rapor JSON seed loader'dan (market+interval+strateji) round-trip etmeli");
    }

    #[test]
    fn seed_filters_flukes_and_keeps_best_per_symbol() {
        let mk = |sym: &str, iv: &str, strat: &str, pf: f64, trades: usize, profitable: bool, wf_robust: bool| EdgeRow {
            exchange: "b".into(), market: "futures".into(), symbol: sym.into(), interval: iv.into(),
            rows: 5000, gap_pct: 0.0, stale_days: 0.0, best_strategy: strat.into(),
            take_profit_pct: 4.0, stop_loss_pct: 2.0, max_position_size: 0.3,
            trades, win_rate: 0.5, profit_factor: pf, expectancy: 0.0, sharpe: 0.0,
            avg_daily_quote_volume: 1e9, profitable,
            wf: WfCrossCheck::default(), wf_robust,
        };
        let report = EdgeScanReport {
            generated_at: "t".into(), db_path: "d".into(), market_filter: None,
            series_candidates: 6, series_scanned: 6, series_skipped: 0, profitable_count: 5,
            summary: vec![],
            rows: vec![
                mk("RAVEUSDT", "1h", "MA_CROSSOVER", 18.5, 16, true, true),  // fluke: trades<30 → elenir
                mk("BTCUSDT", "1h", "ICT_COMPOSITE", 1.5, 40, true, true),   // robust + WF-onaylı → girer
                mk("BTCUSDT", "4h", "MACD", 1.3, 35, true, true),            // aynı sembol düşük PF → kaybeder
                mk("ETHUSDT", "1h", "RSI", 1.1, 50, true, true),             // PF<1.2 → elenir
                mk("ADAUSDT", "1h", "BB", 2.0, 40, false, true),             // profitable=false → elenir
                mk("XRPUSDT", "1h", "CCI", 1.4, 40, true, false),            // WF-onaysız → elenir (yeni)
            ],
        };
        let seed = seed_symbol_plan(&report, SeedRobustness::default());
        assert_eq!(seed.len(), 1, "yalnız BTCUSDT tüm barları (trades+PF+WF) geçer");
        let btc = seed.get("BTCUSDT").expect("BTCUSDT seed'lenmeli");
        // En yüksek PF satırı (1h ICT, 1.5 > 4h MACD 1.3) → market+interval+strateji birlikte taşınır.
        assert_eq!((btc.market.as_str(), btc.interval.as_str(), btc.strategy.as_str()),
            ("futures", "1h", "ICT_COMPOSITE"));
        assert!(!seed.contains_key("RAVEUSDT"), "16 işlem fluke elenir");
        assert!(!seed.contains_key("ETHUSDT"), "PF 1.1 < 1.2 elenir");
        assert!(!seed.contains_key("XRPUSDT"), "WF-onaysız (wf_robust=false) elenir");
        // require_wf_robust=false → XRP (WF-onaysız ama PF/trades geçer) artık girer.
        let loose = SeedRobustness { require_wf_robust: false, ..SeedRobustness::default() };
        let seed2 = seed_symbol_plan(&report, loose);
        assert!(seed2.contains_key("XRPUSDT"), "WF şartı gevşeyince XRP girer");
    }

    #[test]
    fn seed_max_pf_cap_drops_fluke_and_falls_back_to_real_edge() {
        let mk = |sym: &str, iv: &str, strat: &str, pf: f64, trades: usize| EdgeRow {
            exchange: "b".into(), market: "futures".into(), symbol: sym.into(), interval: iv.into(),
            rows: 5000, gap_pct: 0.0, stale_days: 0.0, best_strategy: strat.into(),
            take_profit_pct: 4.0, stop_loss_pct: 2.0, max_position_size: 0.3,
            trades, win_rate: 0.5, profit_factor: pf, expectancy: 0.0, sharpe: 0.0,
            avg_daily_quote_volume: 1e9, profitable: true,
            wf: WfCrossCheck::default(), wf_robust: true,
        };
        let report = EdgeScanReport {
            generated_at: "t".into(), db_path: "d".into(), market_filter: None,
            series_candidates: 3, series_scanned: 3, series_skipped: 0, profitable_count: 3,
            summary: vec![],
            rows: vec![
                // Aynı sembol: 1h fluke (PF 61 > cap) ELENMELİ → 4h gerçek edge (PF 3.0) seçilmeli.
                mk("RAVEUSDT", "1h", "MA_CROSSOVER", 61.0, 40),
                mk("RAVEUSDT", "4h", "DONCHIAN", 3.0, 40),
                mk("SIRENUSDT", "1h", "DONCHIAN", 2.0, 40),
            ],
        };
        let seed = seed_symbol_plan(&report, SeedRobustness::default()); // max_pf=10 (default)
        let rave = seed.get("RAVEUSDT").expect("RAVE'nin gerçek (4h) edge'i kalmalı");
        assert_eq!((rave.interval.as_str(), rave.strategy.as_str()), ("4h", "DONCHIAN"),
            "PF 61 fluke elenince sembolün cap-altı ikinci edge'i seçilir (yüksek-PF cazibesine kapılmaz)");
        assert!(seed.contains_key("SIRENUSDT"), "cap-altı sağlam edge etkilenmez");
        // Cap'i gevşetince (max_pf çok büyük) fluke 1h geri kazanır → cap'in fiilen elediği doğrulanır.
        let loose = SeedRobustness { max_pf: 1e9, ..SeedRobustness::default() };
        let seed2 = seed_symbol_plan(&report, loose);
        assert_eq!(seed2.get("RAVEUSDT").unwrap().interval.as_str(), "1h",
            "cap kalkınca en yüksek PF (61, 1h) kazanır → cap'in elemesi ispatlanır");
    }

    #[test]
    fn seed_multi_plan_keeps_all_validated_tf_edges_bounded_and_sorted() {
        let mk = |sym: &str, iv: &str, strat: &str, pf: f64| EdgeRow {
            exchange: "b".into(), market: "futures".into(), symbol: sym.into(), interval: iv.into(),
            rows: 5000, gap_pct: 0.0, stale_days: 0.0, best_strategy: strat.into(),
            take_profit_pct: 4.0, stop_loss_pct: 2.0, max_position_size: 0.3,
            trades: 40, win_rate: 0.5, profit_factor: pf, expectancy: 0.0, sharpe: 0.0,
            avg_daily_quote_volume: 1e9, profitable: true,
            wf: WfCrossCheck::default(), wf_robust: true,
        };
        // ZEC-benzeri: 4 WF✓ edge (biri PF-cap üstü fluke) + tek-edge bir sembol.
        let report = EdgeScanReport {
            generated_at: "t".into(), db_path: "d".into(), market_filter: None,
            series_candidates: 5, series_scanned: 5, series_skipped: 0, profitable_count: 5,
            summary: vec![],
            rows: vec![
                mk("ZECUSDT", "1d", "STOCH_RSI", 4.42),
                mk("ZECUSDT", "1h", "MACD", 2.27),
                mk("ZECUSDT", "30m", "SUPERTREND", 2.02),
                mk("ZECUSDT", "5m", "RSI", 50.0),   // fluke (>max_pf=10) → elenmeli, ize girmemeli
                mk("XRPUSDT", "1d", "MACD", 4.63),
            ],
        };
        let r = SeedRobustness::default(); // max_pf=10
        let multi = seed_symbol_multi_plan(&report, r.clone(), SEED_MAX_TRACKS_DEFAULT);
        let zec = multi.get("ZECUSDT").expect("ZEC çoklu-iz");
        // Fluke 5m elenir → 3 gerçek edge; PF AZALAN sıralı (1d > 1h > 30m).
        assert_eq!(zec.len(), 3, "cap-altı 3 WF✓ edge tutulur (fluke 5m elenir)");
        assert_eq!(zec.iter().map(|e| (e.interval.as_str(), e.strategy.as_str())).collect::<Vec<_>>(),
            vec![("1d","STOCH_RSI"), ("1h","MACD"), ("30m","SUPERTREND")], "PF azalan sıra");
        assert_eq!(multi.get("XRPUSDT").map(|v| v.len()), Some(1), "tek-edge sembol → tek iz");
        // max_tracks=2 → yalnız en iyi 2 iz (bounded).
        let capped = seed_symbol_multi_plan(&report, r.clone(), 2);
        assert_eq!(capped.get("ZECUSDT").unwrap().len(), 2, "max_tracks ile bounded");
        // Tek-edge seed_symbol_plan = çoklu'nun top-1'i (DRY delegasyon kanıtı).
        let single = seed_symbol_plan(&report, r);
        assert_eq!((single["ZECUSDT"].interval.as_str(), single["ZECUSDT"].strategy.as_str()),
            ("1d","STOCH_RSI"), "tek-plan = çoklu top-1");
    }

    #[test]
    fn avg_daily_quote_volume_normalizes_by_interval() {
        // volume ZATEN quote-volume (idx 7); mum-başı turnover = volume = 1000. 1h: günde 24 mum → 24_000/gün.
        let mk = |iv: &str, n: usize| -> Vec<Candle> {
            (0..n).map(|i| Candle {
                timestamp: Utc.timestamp_opt(i as i64 * 3600, 0).single().unwrap(),
                open: 100.0, high: 100.0, low: 100.0, close: 100.0, volume: 1000.0,
                symbol: "S".into(), interval: iv.into(),
            }).collect()
        };
        let h1 = avg_daily_quote_volume(&mk("1h", 5), "1h");
        assert!((h1 - 24_000.0).abs() < 1e-6, "1h: 1000×24=24000/gün, got {h1}");
        // Aynı mum-başı hacim 1d'de günde 1 mum → 1000/gün (interval normalizasyonu kanıtı).
        let d1 = avg_daily_quote_volume(&mk("1d", 5), "1d");
        assert!((d1 - 1_000.0).abs() < 1e-6, "1d: 1000×1=1000/gün, got {d1}");
        assert_eq!(avg_daily_quote_volume(&[], "1h"), 0.0, "boş → 0");
    }

    #[test]
    fn seed_min_qvol_filters_illiquid_keeps_majors() {
        let mk = |sym: &str, qvol: f64| EdgeRow {
            exchange: "b".into(), market: "futures".into(), symbol: sym.into(), interval: "1h".into(),
            rows: 5000, gap_pct: 0.0, stale_days: 0.0, best_strategy: "DONCHIAN".into(),
            take_profit_pct: 4.0, stop_loss_pct: 2.0, max_position_size: 0.3,
            trades: 40, win_rate: 0.5, profit_factor: 1.5, expectancy: 0.0, sharpe: 0.0,
            avg_daily_quote_volume: qvol, profitable: true,
            wf: WfCrossCheck::default(), wf_robust: true,
        };
        let report = EdgeScanReport {
            generated_at: "t".into(), db_path: "d".into(), market_filter: None,
            series_candidates: 2, series_scanned: 2, series_skipped: 0, profitable_count: 2,
            summary: vec![],
            rows: vec![
                mk("AVAXUSDT", 500_000_000.0), // majör: 500M/gün → kalır
                mk("MYXUSDT", 2_000_000.0),    // illikit-alt: 2M/gün → elenir
            ],
        };
        // Kapı 0.0 (default) → ikisi de geçer (sıfır regresyon).
        assert_eq!(seed_symbol_plan(&report, SeedRobustness::default()).len(), 2);
        // Majör tabanı 50M/gün → yalnız AVAX kalır, MYX elenir.
        let major = SeedRobustness { min_daily_quote_volume: 50_000_000.0, ..SeedRobustness::default() };
        let seed = seed_symbol_plan(&report, major);
        assert_eq!(seed.len(), 1, "yalnız majör (AVAX) seed'lenir");
        assert!(seed.contains_key("AVAXUSDT") && !seed.contains_key("MYXUSDT"),
            "illikit-alt (MYX) majör tabanının altında → elenir (canlı purge gürültüsü kesilir)");
    }

    #[test]
    fn scan_one_series_skips_thin_and_gappy() {
        let cfg = EdgeScanConfig::default();
        // Az bar → None (min_rows altı).
        let thin: Vec<Candle> = (0..50).map(|i| Candle {
            timestamp: Utc.timestamp_opt(i * 3600, 0).single().unwrap(),
            open: 100.0, high: 100.5, low: 99.5, close: 100.0, volume: 1.0,
            symbol: "S".into(), interval: "1h".into(),
        }).collect();
        assert!(scan_one_series(&cfg, &series_ref("S", "1h", 50), &thin).is_none());
    }
}
