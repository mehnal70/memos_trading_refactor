// robot/backtester/funding_carry.rs — kesitsel FUNDING-CARRY ölçüm harness'i.
//
// NEDEN: mum-türevi dik eksenler (low-vol/BAB/lottery) edge'siz çıktı ([[project_xs_factors]]); kalan
// gerçekten-dik aday FUNDING-CARRY — fiyat sinyali bile değil, perp TAŞIMA getirisi. Perp funding'i
// pozitifken long'lar short'lara öder; carry kitabı yüksek-funding'i SHORT (funding alır) / düşük/negatif-
// funding'i LONG eder → market-nötr book funding SPREAD'ini hasat eder.
//
// KRİTİK: edge funding ÖDEMELERİNDE, fiyat hareketinde DEĞİL → getiri = fiyat_getirisi − funding_nakit.
// Yalnız funding'le SIRALAYIP fiyat-getirisi ölçmek YANLIŞ hipotez olurdu ("funding fiyatı öngörür mü").
//
// DRY: XS makinesini yeniden kullanır — select_books (rank kitabı), finalize_metrics_params (Newey-West
// HAC + WF binom + Sharpe), XsResult (rapor). Burada YALNIZ funding'e özgü hizalama + getiri üreteci var.
// Saf çekirdek DB okumaz → testli.

use std::collections::{BTreeMap, HashSet};
use super::xs_momentum::{finalize_metrics_params, select_books, XsResult};
use crate::robot::data_pipeline::DataNormalizer;
use crate::persistence::reader::{read_candles_market, read_funding_market};

/// Funding-carry koşum parametreleri.
#[derive(Debug, Clone)]
pub struct FundingCarryConfig {
    pub db_path: String,
    pub market: String,       // pratikte "futures" (funding yalnız orada)
    pub interval: String,     // mum TF (funding bu bara bucket'lanır); pratikte "1d"
    pub symbols: Vec<String>,
    pub candle_limit: usize,
    pub funding_limit: usize, // sembol başına okunacak funding kaydı tavanı
    /// Trailing funding ortalaması penceresi (bar) — sinyal = −ortalama funding (yüksek funding → short).
    pub lookback: usize,
    pub top_k: usize,
    pub fee_rate: f64,
    pub long_short: bool,
    pub rebalance_every: usize,
    pub leverage: f64,
    pub wf_window: usize,
    pub bars_per_year: f64,
}

impl Default for FundingCarryConfig {
    fn default() -> Self {
        Self {
            db_path: "data/trader.db".into(),
            market: "futures".into(),
            interval: "1d".into(),
            symbols: Vec::new(),
            candle_limit: 5000,
            funding_limit: 20_000,
            lookback: 7,
            top_k: 3,
            fee_rate: 0.0005,
            long_short: true,
            rebalance_every: 1,
            leverage: 1.0,
            wf_window: 30,
            bars_per_year: 365.0,
        }
    }
}

/// SAF: hizalanmış (closes, funding_bar) → carry portföyünün bar-bar NET getiri + turnover serisi.
/// Sinyal_j = −trailing_avg(funding_bar, lookback) (yüksek funding → düşük skor → short bacak).
/// Getiri_j = w_j·(fiyat_getirisi − funding_bar[t+1]) → carry nakit-akışı dahil. select_books ile
/// rank kitabı (buffer 0 = saf top-k/bottom-k). DB'siz → testli.
pub fn funding_carry_returns(
    closes: &[Vec<Option<f64>>], funding_bar: &[Vec<f64>], cfg: &FundingCarryConfig,
) -> (Vec<f64>, Vec<f64>) {
    let n_bars = closes.len();
    let n_sym = if n_bars > 0 { closes[0].len() } else { 0 };
    let lb = cfg.lookback.max(1);
    if n_bars <= lb + 1 || n_sym < 2 || cfg.top_k == 0 {
        return (Vec::new(), Vec::new());
    }
    let rb = cfg.rebalance_every.max(1);
    let mut prev_w = vec![0.0_f64; n_sym];
    let mut prev_long: HashSet<usize> = HashSet::new();
    let mut prev_short: HashSet<usize> = HashSet::new();
    let mut rets: Vec<f64> = Vec::new();
    let mut turnovers: Vec<f64> = Vec::new();

    for (step, t) in (lb..(n_bars - 1)).enumerate() {
        let mut w = prev_w.clone();
        let mut changed = false;
        if step.is_multiple_of(rb) {
            // Trailing funding ortalaması (yalnız mevcut sembol) → −ortalama (yüksek funding short).
            let mut sig: Vec<(usize, f64)> = Vec::with_capacity(n_sym);
            for j in 0..n_sym {
                if closes[t][j].is_none() { continue; }
                let (mut s, mut c) = (0.0_f64, 0usize);
                for b in (t + 1 - lb)..=t {
                    if closes[b][j].is_some() { s += funding_bar[b][j]; c += 1; }
                }
                if c > 0 { sig.push((j, -(s / c as f64))); }
            }
            let need = if cfg.long_short { 2 * cfg.top_k } else { cfg.top_k };
            if sig.len() >= need {
                sig.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                let (longs, shorts) = select_books(&sig, cfg.top_k, 0, &prev_long, &prev_short);
                if !longs.is_empty() {
                    let mut nw = vec![0.0_f64; n_sym];
                    let lw = 1.0 / longs.len() as f64;
                    for &j in &longs { nw[j] += lw; }
                    if cfg.long_short && !shorts.is_empty() {
                        let sw = 1.0 / shorts.len() as f64;
                        for &j in &shorts { nw[j] -= sw; }
                    }
                    w = nw;
                    prev_long = longs.into_iter().collect();
                    prev_short = shorts.into_iter().collect();
                    changed = true;
                }
            }
        }
        // Kitap kurulana dek (warmup) kaydetme.
        if w.iter().all(|x| *x == 0.0) { continue; }

        // Sonraki-bar NET getirisi: fiyat_getirisi − funding (carry nakit-akışı funding_bar[t+1]).
        let mut port = 0.0_f64;
        for (j, &wj) in w.iter().enumerate() {
            if wj == 0.0 { continue; }
            if let (Some(c0), Some(c1)) = (closes[t][j], closes[t + 1][j]) {
                if c0 > 0.0 {
                    let price_ret = c1 / c0 - 1.0;
                    let fund = funding_bar[t + 1][j];
                    port += wj * (price_ret - fund);
                }
            }
        }
        let turnover: f64 = if changed {
            (0..n_sym).map(|j| (w[j] - prev_w[j]).abs()).sum()
        } else { 0.0 };
        rets.push(port - turnover * cfg.fee_rate);
        turnovers.push(turnover);
        prev_w = w;
    }
    (rets, turnovers)
}

/// Tam-örneklem değerlendirme: getiri serisi → XS ile AYNI metrik dili (Newey-West + WF binom).
pub fn evaluate_funding_carry(
    closes: &[Vec<Option<f64>>], funding_bar: &[Vec<f64>], cfg: &FundingCarryConfig,
) -> XsResult {
    let n_sym = if closes.is_empty() { 0 } else { closes[0].len() };
    let mut res = XsResult { symbols_used: n_sym, ..Default::default() };
    let (rets, turns) = funding_carry_returns(closes, funding_bar, cfg);
    finalize_metrics_params(&mut res, &rets, &turns, cfg.leverage, cfg.bars_per_year,
        cfg.rebalance_every, cfg.wf_window);
    res
}

/// DB'den sepet mumlarını ORTAK ts-ızgarasına hizalar VE funding'i o ızgaranın barlarına bucket'lar.
/// funding_bar[b][j] = (stamps[b−1], stamps[b]] aralığındaki funding rate toplamı (b=0 → 0). Yani
/// funding, ödendiği bar-kapanışına atanır → getiri tarafında funding_bar[t+1] holding-bar nakitini verir.
/// Market-saf; align_closes felsefesiyle aynı union grid.
pub fn align_closes_and_funding(cfg: &FundingCarryConfig) -> (Vec<Vec<Option<f64>>>, Vec<Vec<f64>>) {
    let n_sym = cfg.symbols.len();
    let mut per_close: Vec<BTreeMap<i64, f64>> = Vec::with_capacity(n_sym);
    let mut per_fund: Vec<Vec<(i64, f64)>> = Vec::with_capacity(n_sym);
    let mut grid: BTreeMap<i64, ()> = BTreeMap::new();
    for sym in &cfg.symbols {
        let candles = read_candles_market(&cfg.db_path, sym, &cfg.interval, &cfg.market, cfg.candle_limit)
            .unwrap_or_default();
        let mut m = BTreeMap::new();
        for c in &candles {
            let ts = c.timestamp.timestamp_millis();
            m.insert(ts, c.close);
            grid.insert(ts, ());
        }
        per_close.push(m);
        per_fund.push(read_funding_market(&cfg.db_path, sym, &cfg.market, cfg.funding_limit).unwrap_or_default());
    }
    let stamps: Vec<i64> = grid.keys().copied().collect();
    let n = stamps.len();
    let closes: Vec<Vec<Option<f64>>> = stamps.iter()
        .map(|ts| per_close.iter().map(|m| m.get(ts).copied()).collect())
        .collect();
    // funding bucket: her funding (t,rate) → ilk stamps[b] ≥ t barına (partition_point), kapanışa atanır.
    let mut funding_bar = vec![vec![0.0_f64; n_sym]; n];
    for (j, fv) in per_fund.iter().enumerate() {
        for (ft, rate) in fv {
            let b = stamps.partition_point(|&s| s < *ft);
            if b < n { funding_bar[b][j] += *rate; }
        }
    }
    (closes, funding_bar)
}

/// DB-YÜKLEYEN sürüm: hizala → evaluate_funding_carry.
pub fn run_funding_carry(cfg: &FundingCarryConfig) -> XsResult {
    let (closes, funding_bar) = align_closes_and_funding(cfg);
    evaluate_funding_carry(&closes, &funding_bar, cfg)
}

/// DB-yükleyen NET-getiri serisi (metrik değil) — momentum'la DİKLİK korelasyonu için.
pub fn run_funding_carry_returns(cfg: &FundingCarryConfig) -> (Vec<f64>, Vec<f64>) {
    let (closes, funding_bar) = align_closes_and_funding(cfg);
    funding_carry_returns(&closes, &funding_bar, cfg)
}

/// Interval'i milisaniye adımına çevirir (rapor/yardımcı; çekirdek bucketing stamps kullanır).
pub fn interval_ms(interval: &str) -> i64 {
    DataNormalizer::parse_interval(interval).max(1) as i64 * 1000
}

// ───────────────────────── Walk-forward OOS (look-ahead'siz) ─────────────────────────

/// WF kalibrasyon: aday lookback ızgarasından HER IS penceresinde en iyiyi (IS Sharpe) seç → kör
/// OOS'a uygula → birleştir. Tam-veri "en iyi lookback" optimizmini keser. bb_pool/XS WF ile aynı iskelet.
#[derive(Debug, Clone)]
pub struct FcWfConfig {
    pub is_bars: usize,
    pub oos_bars: usize,
    pub candidates: Vec<usize>, // lookback adayları
}

#[derive(Debug, Clone, Default)]
pub struct FcWfResult {
    pub oos: XsResult,
    pub windows: usize,
    pub selections: Vec<usize>,
    pub is_oos_pairs: Vec<(f64, f64)>, // (IS Sharpe, OOS ort. getiri)
}

/// SAF WF çekirdeği (hizalanmış closes+funding üzerinde, DB'siz testlenir).
pub fn evaluate_funding_carry_walkforward(
    closes: &[Vec<Option<f64>>], funding_bar: &[Vec<f64>], base: &FundingCarryConfig, wf: &FcWfConfig,
) -> FcWfResult {
    let n = closes.len();
    let n_sym = if n > 0 { closes[0].len() } else { 0 };
    let mut oos_rets: Vec<f64> = Vec::new();
    let mut oos_turn: Vec<f64> = Vec::new();
    let mut selections: Vec<usize> = Vec::new();
    let mut is_oos_pairs: Vec<(f64, f64)> = Vec::new();

    let mut oos_start = wf.is_bars;
    while oos_start + wf.oos_bars <= n && !wf.candidates.is_empty() {
        let is_lo = oos_start.saturating_sub(wf.is_bars);
        // 1) IS'te en iyi lookback'i seç (IS Sharpe).
        let mut best: Option<(usize, f64)> = None;
        for &lb in &wf.candidates {
            let cfg = FundingCarryConfig { lookback: lb, ..base.clone() };
            let is_res = evaluate_funding_carry(&closes[is_lo..oos_start], &funding_bar[is_lo..oos_start], &cfg);
            if is_res.bars > 0 && best.is_none_or(|(_, s)| is_res.ann_sharpe > s) {
                best = Some((lb, is_res.ann_sharpe));
            }
        }
        let Some((sel, is_sharpe)) = best else { oos_start += wf.oos_bars; continue; };
        // 2) seçileni OOS'a UYGULA (lookback lead-in dahil dilim; getiri yalnız OOS bölgesinde).
        let lead = sel.min(oos_start);
        let seg_hi = (oos_start + wf.oos_bars).min(n);
        let cfg = FundingCarryConfig { lookback: sel, ..base.clone() };
        let (rets, turns) = funding_carry_returns(
            &closes[oos_start - lead..seg_hi], &funding_bar[oos_start - lead..seg_hi], &cfg);
        let oos_mean = if rets.is_empty() { 0.0 } else { rets.iter().sum::<f64>() / rets.len() as f64 };
        oos_rets.extend(rets);
        oos_turn.extend(turns);
        selections.push(sel);
        is_oos_pairs.push((is_sharpe, oos_mean));
        oos_start += wf.oos_bars;
    }

    let mut oos = XsResult { symbols_used: n_sym, ..Default::default() };
    finalize_metrics_params(&mut oos, &oos_rets, &oos_turn, base.leverage, base.bars_per_year,
        base.rebalance_every, base.wf_window);
    FcWfResult { oos, windows: selections.len(), selections, is_oos_pairs }
}

/// DB-YÜKLEYEN WF sürümü.
pub fn run_funding_carry_walkforward(base: &FundingCarryConfig, wf: &FcWfConfig) -> FcWfResult {
    let (closes, funding_bar) = align_closes_and_funding(base);
    evaluate_funding_carry_walkforward(&closes, &funding_bar, base, wf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(lb: usize) -> FundingCarryConfig {
        FundingCarryConfig { lookback: lb, top_k: 1, fee_rate: 0.0, rebalance_every: 1,
            wf_window: 5, ..Default::default() }
    }

    /// closes düz (fiyat getirisi 0) + sabit funding farkı → carry book SAF funding hasadı yapar.
    /// A yüksek-funding (+), B negatif-funding (−): short A (alır), long B (alır) → pozitif getiri.
    #[test]
    fn harvests_funding_spread_with_flat_prices() {
        let n = 40;
        let closes: Vec<Vec<Option<f64>>> = (0..n).map(|_| vec![Some(100.0), Some(100.0)]).collect();
        // A=+0.001/bar, B=−0.001/bar (kapanışa atanmış toplam).
        let funding_bar: Vec<Vec<f64>> = (0..n).map(|_| vec![0.001, -0.001]).collect();
        let r = evaluate_funding_carry(&closes, &funding_bar, &cfg(5));
        assert!(r.bars > 20, "yeterli bar");
        // long B (−funding → alır), short A (+funding → alır) → her bacak +0.001 → ~+0.002/bar.
        assert!(r.mean_ret > 0.0, "carry spread pozitif olmalı, mean={}", r.mean_ret);
    }

    /// Eşit funding → spread yok → düz fiyatta getiri ~0 (carry differential yok).
    #[test]
    fn no_spread_no_carry() {
        let n = 40;
        let closes: Vec<Vec<Option<f64>>> = (0..n).map(|_| vec![Some(100.0), Some(100.0)]).collect();
        let funding_bar: Vec<Vec<f64>> = (0..n).map(|_| vec![0.0005, 0.0005]).collect();
        let r = evaluate_funding_carry(&closes, &funding_bar, &cfg(5));
        assert!(r.mean_ret.abs() < 1e-9, "eşit funding → carry ~0, mean={}", r.mean_ret);
    }

    /// WF: aday lookback ızgarasından IS seç → OOS uygula → pencereler birleşir.
    #[test]
    fn walkforward_runs_and_pools_oos() {
        let n = 400;
        let closes: Vec<Vec<Option<f64>>> = (0..n).map(|_| vec![Some(100.0), Some(100.0)]).collect();
        let funding_bar: Vec<Vec<f64>> = (0..n).map(|_| vec![0.001, -0.001]).collect();
        let base = cfg(7);
        let wf = FcWfConfig { is_bars: 120, oos_bars: 60, candidates: vec![3, 7, 14] };
        let r = evaluate_funding_carry_walkforward(&closes, &funding_bar, &base, &wf);
        assert!(r.windows >= 2, "en az birkaç OOS penceresi");
        assert!(r.oos.bars > 0, "birleştirilmiş OOS getirisi var");
        assert!(r.oos.mean_ret > 0.0, "sabit spread → OOS'ta da pozitif carry");
    }

    /// Bucketing: funding (t,rate) ilk stamps[b]≥t barına atanır (kapanışa).
    #[test]
    fn funding_buckets_to_closing_bar() {
        // stamps: 100,200,300; funding at t=150 → bar b=1 (ilk ≥150); t=300 → b=2; t=350 → atılır.
        let stamps = [100i64, 200, 300];
        let assign = |ft: i64| stamps.partition_point(|&s| s < ft);
        assert_eq!(assign(150), 1);
        assert_eq!(assign(200), 1, "tam eşit → inclusive üst");
        assert_eq!(assign(300), 2);
        assert_eq!(assign(350), 3, "son barın ötesi → n (atılır)");
    }
}
