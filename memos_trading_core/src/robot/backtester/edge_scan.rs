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

use serde::{Deserialize, Serialize};

use crate::persistence::reader::{list_series, read_candles_market, CandleSeriesRef};
use crate::robot::backtester::{Backtester, BacktestConfig, ParameterOptimizer};
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
    /// PF≥1.0 VE işlem≥min_trades → net kârlı edge.
    pub profitable: bool,
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
    pub summary: Vec<GroupSummary>,
    /// PF AZALAN sıralı satırlar.
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
            profitable: r.profit_factor >= 1.0 && r.total_trades >= cfg.min_trades,
        };
        if is_better(&cand, best.as_ref(), cfg.min_trades) { best = Some(cand); }
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
            trades, win_rate: 0.5, profit_factor: pf, expectancy: 0.0, sharpe: 0.0, profitable: false,
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
            trades: 20, win_rate: 0.5, profit_factor: pf, expectancy: 0.0, sharpe: 0.0, profitable,
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
