// robot/backtester/bb_pool.rs — "1d-BB havuzlanmış" hipotez ölçüm harness'i.
//
// NEDEN: edge_scan'in per-sembol ekseni çoklu-test (Šidák) altında tükendi; tek yapısal ipucu
// 1d'de BB(Bollinger) stratejisinin birden çok majörde robust çıkmasıydı. Tek tek hiçbiri
// aile-düzeyini geçmez ve aynı strateji+TF oldukları için bağımsız kanıt değiller. Doğru test:
// BB-ortalama-dönüşü PER-SEMBOL değil, majör sepetinde TEK PORTFÖY-SERİSİ olarak havuzla → küçük
// örneklem dik kesilir, gerçek edge varsa N büyür ve p-değeri anlamlıya gider (XS momentum'un
// gerçek lead'e dönüşme mantığı — [[project_xs_momentum]]).
//
// DRY: XS makinesini yeniden kullanır — align_closes (DB hizalama), finalize_metrics_params
// (Newey-West HAC + WF binom + Sharpe), XsResult (rapor), WfCrossCheck. Burada YALNIZ BB-pool'a
// özgü getiri üreteci (bb_pool_returns) ve WF-OOS sarmalı vardır. Saf çekirdek DB okumaz → testli.

use crate::core::types::Candle;
use super::walk_forward::WfCrossCheck;
use super::xs_momentum::{align_closes, finalize_metrics_params, XsConfig, XsResult};

/// BB-pool koşum parametreleri. Operatör/algoritmik ayrımı XS ile aynı felsefede.
#[derive(Debug, Clone)]
pub struct BbPoolConfig {
    pub db_path: String,
    pub market: String,
    pub interval: String,
    /// Sepet sembolleri (majörler). Havuzun anlamı için ≥5 önerilir.
    pub symbols: Vec<String>,
    pub candle_limit: usize,
    /// Bollinger periyodu (orta = SMA(period)).
    pub bb_period: usize,
    /// Bant katsayısı k: alt/üst = SMA ∓ k·σ (popülasyon std, calculate_bollinger ile aynı konvansiyon).
    pub bb_k: f64,
    /// Turnover birimi başına tek-yön maliyet (fee+slippage). Σ|Δw|·fee_rate düşülür.
    pub fee_rate: f64,
    /// true = market-nötr long+short (alt-bandın altı long, üst-bandın üstü short);
    /// false = yalnız-long mean-reversion (alt-bandın altı long, üst → flat).
    pub long_short: bool,
    /// Rebalance kadansı (bar): hedef ağırlıklar yalnız her N bar'da yeniden hesaplanır.
    pub rebalance_every: usize,
    /// Kaldıraç (gross çarpanı). t-stat/Sharpe-invariant; yalnız bileşik büyümeyi ölçekler.
    pub leverage: f64,
    /// WF binom penceresi (bar): return serisini bu uzunlukta ardışık parçalara böler.
    pub wf_window: usize,
    /// Yıllık bar (annualize). 1d → 365.
    pub bars_per_year: f64,
}

impl Default for BbPoolConfig {
    fn default() -> Self {
        Self {
            db_path: "data/trader.db".into(),
            market: "futures".into(),
            interval: "1d".into(),
            symbols: Vec::new(),
            candle_limit: 5000,
            bb_period: 20,
            bb_k: 2.0,
            fee_rate: 0.0005,
            long_short: true,
            rebalance_every: 1,
            leverage: 1.0,
            wf_window: 30,
            bars_per_year: 365.0,
        }
    }
}

impl BbPoolConfig {
    /// align_closes için XsConfig köprüsü (yalnız DB/market/interval/sembol/limit alanları okunur).
    fn align_cfg(&self) -> XsConfig {
        XsConfig {
            db_path: self.db_path.clone(),
            market: self.market.clone(),
            interval: self.interval.clone(),
            symbols: self.symbols.clone(),
            candle_limit: self.candle_limit,
            ..Default::default()
        }
    }
}

/// SAF: bar t'de her sembol için BB konumundan hedef ağırlık vektörü. longs alt-bandın altında
/// (mean-reversion: yukarı dönüş beklentisi → +), shorts üst-bandın üstünde (long_short ise → −).
/// Her bacak EŞİT-AĞIRLIK ve kendi içinde Σ=±1 (long sepeti +1, short sepeti −1). Sinyal yoksa
/// FLAT (tüm-sıfır → mevcut kitaptan çıkış turnover'ı). Bant hesaplanamayan sembol (eksik/σ≤0) atlanır.
fn bb_weights(closes: &[Vec<Option<f64>>], t: usize, cfg: &BbPoolConfig) -> Vec<f64> {
    let n_sym = closes[0].len();
    let p = cfg.bb_period;
    let mut longs: Vec<usize> = Vec::new();
    let mut shorts: Vec<usize> = Vec::new();
    for j in 0..n_sym {
        // Trailing pencere closes[t-p+1 ..= t] — hepsi Some olmalı.
        if t + 1 < p { continue; }
        let lo = t + 1 - p;
        let mut s = 0.0;
        let mut s2 = 0.0;
        let mut ok = true;
        for bar in closes.iter().take(t + 1).skip(lo) {
            match bar[j] {
                Some(v) => { s += v; s2 += v * v; }
                None => { ok = false; break; }
            }
        }
        if !ok { continue; }
        let Some(c_t) = closes[t][j] else { continue };
        let pf = p as f64;
        let mean = s / pf;
        let var = (s2 / pf - mean * mean).max(0.0);
        let std = var.sqrt();
        if std <= 0.0 { continue; }
        let lower = mean - cfg.bb_k * std;
        let upper = mean + cfg.bb_k * std;
        if c_t < lower {
            longs.push(j);
        } else if cfg.long_short && c_t > upper {
            shorts.push(j);
        }
    }
    let mut w = vec![0.0_f64; n_sym];
    if !longs.is_empty() {
        let lw = 1.0 / longs.len() as f64;
        for j in longs { w[j] = lw; }
    }
    if !shorts.is_empty() {
        let sw = -1.0 / shorts.len() as f64;
        for j in shorts { w[j] = sw; }
    }
    w
}

/// SAF ÇEKİRDEK: hizalanmış kapanış matrisinden BB-pool portföyünün bar-bar NET getiri + turnover
/// serisini üretir (metriklerden ayrı → WF OOS dilimlerini birleştirebilir). DB'siz → testli.
pub fn bb_pool_returns(closes: &[Vec<Option<f64>>], cfg: &BbPoolConfig) -> (Vec<f64>, Vec<f64>) {
    let n_bars = closes.len();
    let n_sym = if n_bars > 0 { closes[0].len() } else { 0 };
    if n_bars <= cfg.bb_period + 1 || n_sym < 1 {
        return (Vec::new(), Vec::new());
    }
    let rb = cfg.rebalance_every.max(1);
    let mut prev_w = vec![0.0_f64; n_sym];
    let mut rets: Vec<f64> = Vec::new();
    let mut turnovers: Vec<f64> = Vec::new();

    // step = döngü-içi sıra (kadans saati); t = bar indeksi. step her bar artar → step = t − bb_period.
    for (step, t) in (cfg.bb_period..(n_bars - 1)).enumerate() {
        // Kadans: yalnız her rb bar'da hedefi yeniden hesapla; arada SABİT tut.
        let mut w = prev_w.clone();
        let mut turn = 0.0;
        if step.is_multiple_of(rb) {
            let tw = bb_weights(closes, t, cfg);
            turn = tw.iter().zip(&prev_w).map(|(a, b)| (a - b).abs()).sum();
            prev_w = tw.clone();
            w = tw;
        }
        // Portföyün SONRAKİ-bar getirisi: Σ w_j · (close[t+1]/close[t] − 1).
        let mut gross = 0.0;
        for (j, &wj) in w.iter().enumerate() {
            if wj == 0.0 { continue; }
            if let (Some(c0), Some(c1)) = (closes[t][j], closes[t + 1][j]) {
                if c0 > 0.0 { gross += wj * (c1 / c0 - 1.0); }
            }
        }
        let net = gross - turn * cfg.fee_rate;
        rets.push(net);
        turnovers.push(turn);
    }
    (rets, turnovers)
}

/// Tam-örneklem değerlendirme: getiri serisi → XS ile AYNI metrik dili (Newey-West + WF binom).
pub fn evaluate_bb_pool(closes: &[Vec<Option<f64>>], cfg: &BbPoolConfig) -> XsResult {
    let n_sym = if closes.is_empty() { 0 } else { closes[0].len() };
    let mut res = XsResult { symbols_used: n_sym, ..Default::default() };
    let (rets, turns) = bb_pool_returns(closes, cfg);
    finalize_metrics_params(&mut res, &rets, &turns, cfg.leverage, cfg.bars_per_year,
        cfg.rebalance_every, cfg.wf_window);
    res
}

/// DB-YÜKLEYEN sürüm: align_closes (XS köprüsü) → evaluate_bb_pool.
pub fn run_bb_pool(cfg: &BbPoolConfig) -> XsResult {
    let closes = align_closes(&cfg.align_cfg());
    evaluate_bb_pool(&closes, cfg)
}

// ───────────────────────── Walk-forward OOS (look-ahead'siz) ─────────────────────────

/// WF kalibrasyon: aday (period, k) ızgarasından HER IS penceresinde en iyiyi (IS Sharpe) seç →
/// SONRAKİ OOS penceresine kör uygula → OOS getirilerini birleştir. Tam-veri "en iyi config"
/// optimizmini keser. XS WF ile birebir aynı iskelet ([[project_xs_momentum]]).
#[derive(Debug, Clone)]
pub struct BbWfConfig {
    pub is_bars: usize,
    pub oos_bars: usize,
    pub candidates: Vec<(usize, f64)>, // (bb_period, bb_k)
}

#[derive(Debug, Clone, Default)]
pub struct BbWfResult {
    pub oos: XsResult,
    pub windows: usize,
    pub selections: Vec<(usize, f64)>,
    pub is_oos_pairs: Vec<(f64, f64)>, // (IS Sharpe, OOS ort. getiri)
}

/// SAF WF çekirdeği (hizalanmış matris üzerinde, DB'siz testlenir).
pub fn evaluate_bb_pool_walkforward(
    closes: &[Vec<Option<f64>>], base: &BbPoolConfig, wf: &BbWfConfig,
) -> BbWfResult {
    let n = closes.len();
    let n_sym = if n > 0 { closes[0].len() } else { 0 };
    let mut oos_rets: Vec<f64> = Vec::new();
    let mut oos_turn: Vec<f64> = Vec::new();
    let mut selections: Vec<(usize, f64)> = Vec::new();
    let mut is_oos_pairs: Vec<(f64, f64)> = Vec::new();

    let mut oos_start = wf.is_bars;
    while oos_start + wf.oos_bars <= n && !wf.candidates.is_empty() {
        let is_lo = oos_start.saturating_sub(wf.is_bars);
        // 1) IS'te en iyi adayı seç (IS Sharpe).
        let mut best: Option<((usize, f64), f64)> = None;
        for &(period, k) in &wf.candidates {
            let cfg = BbPoolConfig { bb_period: period, bb_k: k, ..base.clone() };
            let is_res = evaluate_bb_pool(&closes[is_lo..oos_start], &cfg);
            if is_res.bars > 0 && best.is_none_or(|(_, s)| is_res.ann_sharpe > s) {
                best = Some(((period, k), is_res.ann_sharpe));
            }
        }
        let Some((sel, is_sharpe)) = best else { oos_start += wf.oos_bars; continue; };
        // 2) seçileni OOS'a UYGULA (period lead-in dahil dilim, getiri yalnız OOS bölgesinde).
        let lead = sel.0.min(oos_start);
        let seg_hi = (oos_start + wf.oos_bars).min(n);
        let cfg = BbPoolConfig { bb_period: sel.0, bb_k: sel.1, ..base.clone() };
        let (rets, turns) = bb_pool_returns(&closes[oos_start - lead..seg_hi], &cfg);
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
    BbWfResult { oos, windows: selections.len(), selections, is_oos_pairs }
}

/// DB-YÜKLEYEN WF sürümü.
pub fn run_bb_pool_walkforward(base: &BbPoolConfig, wf: &BbWfConfig) -> BbWfResult {
    let closes = align_closes(&base.align_cfg());
    evaluate_bb_pool_walkforward(&closes, base, wf)
}

/// WfCrossCheck'i (binom kapısı) dışarı taşıyan ufak yardımcı (rapor için tek-nokta).
pub fn oos_binom_pvalue(wf: &WfCrossCheck) -> f64 { wf.window_significance() }

#[cfg(test)]
mod tests {
    use super::*;

    /// closes matrisi kur: her sembol için verilen fiyat serisi (hepsi Some, eşit uzunluk).
    fn matrix(series: &[Vec<f64>]) -> Vec<Vec<Option<f64>>> {
        let n_bars = series[0].len();
        (0..n_bars).map(|t| series.iter().map(|s| Some(s[t])).collect()).collect()
    }

    fn cfg(period: usize, long_short: bool) -> BbPoolConfig {
        BbPoolConfig { bb_period: period, bb_k: 1.0, fee_rate: 0.0, long_short,
            rebalance_every: 1, wf_window: 5, ..Default::default() }
    }

    #[test]
    fn mean_reverting_basket_is_profitable() {
        // Zikzak (ortalama-dönüşlü) seri: alt-banda değince yukarı, üst-banda değince aşağı döner.
        // BB long-short mean-reversion POZİTİF ortalama getiri vermeli.
        let mut a = Vec::new();
        for i in 0..120 { a.push(100.0 + 6.0 * ((i as f64) * 0.6).sin()); }
        // İkinci sembol faz-kaymalı → her bar kitap dengeli dolsun.
        let mut b = Vec::new();
        for i in 0..120 { b.push(100.0 + 6.0 * ((i as f64) * 0.6 + 1.6).sin()); }
        let m = matrix(&[a, b]);
        let r = evaluate_bb_pool(&m, &cfg(10, true));
        assert!(r.bars > 50, "yeterli bar");
        assert!(r.mean_ret > 0.0, "ortalama-dönüşlü sepette BB-pool pozitif olmalı, mean={}", r.mean_ret);
    }

    #[test]
    fn trending_up_basket_loses_with_shorts() {
        // Monoton yükselen seri: close sürekli üst-bandı kırar → short → ertesi bar yine yukarı →
        // short kaybeder. long_short mean-reversion NEGATİF ortalama vermeli (trend-karşıtı).
        let up: Vec<f64> = (0..120).map(|i| 100.0 + i as f64).collect();
        let up2: Vec<f64> = (0..120).map(|i| 100.0 + 1.1 * i as f64).collect();
        let m = matrix(&[up, up2]);
        let r = evaluate_bb_pool(&m, &cfg(10, true));
        assert!(r.bars > 50);
        assert!(r.mean_ret <= 0.0, "trendde mean-reversion short'ları kaybetmeli, mean={}", r.mean_ret);
    }

    #[test]
    fn flat_when_no_band_break() {
        // Sabit fiyat → σ=0 → hiç sinyal yok → tüm getiriler 0.
        let flat = vec![100.0; 80];
        let m = matrix(&[flat.clone(), flat]);
        let r = evaluate_bb_pool(&m, &cfg(10, true));
        assert_eq!(r.total_return, 0.0, "bant kırılmadan pozisyon olmamalı");
    }

    #[test]
    fn walkforward_runs_and_pools_oos() {
        // Zikzak sepet + aday ızgara → WF OOS pencereleri birleşir, pozitif OOS.
        let a: Vec<f64> = (0..400).map(|i| 100.0 + 6.0 * ((i as f64) * 0.6).sin()).collect();
        let b: Vec<f64> = (0..400).map(|i| 100.0 + 6.0 * ((i as f64) * 0.6 + 1.6).sin()).collect();
        let m = matrix(&[a, b]);
        let base = cfg(10, true);
        let wf = BbWfConfig { is_bars: 120, oos_bars: 60, candidates: vec![(8, 1.0), (10, 1.0), (14, 2.0)] };
        let r = evaluate_bb_pool_walkforward(&m, &base, &wf);
        assert!(r.windows >= 2, "en az birkaç OOS penceresi");
        assert!(r.oos.bars > 0, "birleştirilmiş OOS getirisi var");
    }
}
