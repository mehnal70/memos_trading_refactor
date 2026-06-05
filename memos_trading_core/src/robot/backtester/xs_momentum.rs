//! 📐 KESİTSEL (cross-sectional) RELATİF-GÜÇ sinyali — ÖLÇÜM harness'i.
//!
//! NEDEN: per-(sembol,TF,strateji) edge taraması (edge_scan) çoklu-test + küçük-örneklem
//! duvarına tosladı — WF-robust kazananlar ~hepsi binom anlamlılığını geçemeyen yazı-tura
//! flukeleriydi ([[project_edge_scan]] hedefli 1d-majör testi: 15 majörden yalnız ETH p=0.084,
//! o da Šidák'ı geçmiyor). KÖK kısıt İSTATİSTİKSEL: her edge ~20 OOS penceresinde ölçülüyor.
//!
//! BU MODÜLÜN DİK ÇÖZÜMÜ: edge'i tek bir sembolde değil, majör SEPETİ üzerinde **kesitsel**
//! sorar. Her bar (gün) sepeti relatif momentuma göre sıralar, en güçlüleri LONG / en zayıfları
//! SHORT (market-nötr spread) → getiri TEK portföy-zaman-serisidir. N = rebalance sayısı (yüzlerce),
//! 20-pencere/sembol değil → t-istatistiği gerçek güce kavuşur. Kriptoda kesitsel momentum/reversal
//! belgelenmiş nadir robust faktörlerden; per-sembol fiyat-örüntü stratejilerinden DİK bağımsız.
//!
//! Bu bir ÖLÇÜM aracıdır (edge_scan gibi keşfeder, ilan etmez): saf çekirdek `evaluate_xs`
//! (DB'siz testlenir) + `run_xs_momentum` (DB yükler+hizalar). Anlamlılık: hem klasik t-stat
//! hem projenin tek-kaynak binom pencere kapısı (`WfCrossCheck::window_significance`).

use std::collections::BTreeMap;

use crate::persistence::reader::read_candles_market;
use super::walk_forward::WfCrossCheck;

/// Kesitsel strateji parametreleri. Sepet + momentum/reversal yönü + maliyet + anlamlılık penceresi.
#[derive(Debug, Clone)]
pub struct XsConfig {
    pub db_path: String,
    pub market: String,
    pub interval: String,
    /// Sepet sembolleri (majörler). En az 2*top_k gerekir.
    pub symbols: Vec<String>,
    pub candle_limit: usize,
    /// Momentum geriye-bakış (bar): sinyal_s = close[t]/close[t−lookback] − 1.
    pub lookback: usize,
    /// Sepet kenarı: kaç sembol long / kaç sembol short. ≥1.
    pub top_k: usize,
    /// İşlem maliyeti — turnover BİRİMİ başına tek-yön oran (fee+slippage). Σ|Δw|·fee_rate düşülür.
    pub fee_rate: f64,
    /// true = momentum (en güçlü long / en zayıf short); false = reversal (ters).
    pub momentum: bool,
    /// true = market-nötr long-short (gross=2); false = long-only top (gross=1).
    pub long_short: bool,
    /// Rebalance kadansı (bar): hedef ağırlıklar yalnız her N bar'da yeniden hesaplanır+turnover ödenir;
    /// arada hedef SABİT tutulur (periyodik-rebalance yaklaşımı). 1 = her bar. Yavaş (14-30 gün) momentum
    /// sinyalini her gün dengelemek gereksiz churn → kadansı büyütmek turnover'ı ~N× kısar, net edge'i tavana yaklaştırır.
    pub rebalance_every: usize,
    /// Anlamlılık penceresi (bar): return serisini bu uzunlukta ARDIŞIK parçalara böler → WfCrossCheck.
    pub wf_window: usize,
    /// Yıllık bar sayısı (Sharpe/yıllık-getiri annualize). 1d → 365, 1h → 8760.
    pub bars_per_year: f64,
}

impl Default for XsConfig {
    fn default() -> Self {
        Self {
            db_path: "data/trader.db".into(),
            market: "futures".into(),
            interval: "1d".into(),
            symbols: Vec::new(),
            candle_limit: 5000,
            lookback: 7,
            top_k: 3,
            fee_rate: 0.0005, // 5 bps / turnover birimi (round-turn ~10bps futures taker+slippage)
            momentum: true,
            long_short: true,
            rebalance_every: 1, // her bar (geri-uyumlu); yavaş sinyalde XS_REBALANCE>1 ile churn kıs
            wf_window: 30,    // ~aylık pencere (1d) → tutarlılık binom kapısı
            bars_per_year: 365.0,
        }
    }
}

/// Kesitsel koşumun ÖLÇÜM çıktısı. t_stat = asıl güç; wf.window_significance() = projenin binom kapısı.
#[derive(Debug, Clone, Default)]
pub struct XsResult {
    pub bars: usize,         // değerlendirilen rebalance (return) sayısı = N
    pub symbols_used: usize, // hizalanmış matriste sembol sütunu sayısı
    pub total_return: f64,   // bileşik net getiri (maliyet sonrası)
    pub ann_return: f64,
    pub ann_sharpe: f64,
    pub win_rate: f64,       // pozitif-getirili bar oranı
    pub mean_ret: f64,       // bar başı ortalama net getiri
    pub std_ret: f64,
    pub t_stat: f64,         // mean / (std/√N) — tek-yanlı (H0: getiri≤0)
    pub avg_turnover: f64,
    pub wf: WfCrossCheck,    // pencere-tutarlılığı → window_significance() binom p-değeri
}

impl XsResult {
    /// Tek-yanlı t-stat'ı normal-yaklaşımla p-değerine çevirir (büyük N; H0: ortalama getiri ≤ 0).
    /// Otokorelasyon düzeltmesi YOK (günlük getiri kabaca bağımsız) → muhafazakâr ön-eleme.
    pub fn t_pvalue(&self) -> f64 { one_sided_normal_sf(self.t_stat) }
}

/// Std normal üst-kuyruk P(Z≥z) — Abramowitz-Stegun 7.1.26 erf yaklaşımı (saf, bağımlılıksız).
fn one_sided_normal_sf(z: f64) -> f64 {
    // P(Z≥z) = 0.5·erfc(z/√2)
    0.5 * erfc(z / std::f64::consts::SQRT_2)
}

fn erfc(x: f64) -> f64 {
    // A&S 7.1.26: |hata| < 1.5e-7. erf(x) tek fonksiyon → negatif x simetriyle.
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let y = 1.0
        - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
            + 0.254829592)
            * t
            * (-x * x).exp();
    1.0 - sign * y // erfc(x) = 1 − erf(x)
}

/// SAF ÇEKİRDEK: hizalanmış kapanış matrisinden (`closes[bar][sym]`, eksik = None) kesitsel
/// portföyü kurar ve metrikleri üretir. DB'siz → birim-testlenir. Her bar:
///  1) sinyal_j = close[t]/close[t−lookback]−1 (ikisi de Some); yeterli sembol yoksa bar atlanır.
///  2) sırala → momentum: top-k long / bottom-k short; reversal: ters. Ağırlık long Σ=+1, short Σ=−1.
///  3) gerçekleşen getiri = Σ w_j·(close[t+1]/close[t]−1); maliyet = Σ|w−w_prev|·fee_rate.
pub fn evaluate_xs(closes: &[Vec<Option<f64>>], cfg: &XsConfig) -> XsResult {
    let n_sym = if closes.is_empty() { 0 } else { closes[0].len() };
    let mut res = XsResult { symbols_used: n_sym, ..Default::default() };
    let (rets, turnovers) = xs_returns(closes, cfg);
    finalize_metrics(&mut res, &rets, &turnovers, cfg);
    res
}

/// SAF: kesitsel portföyün bar-bar NET getiri + turnover serisini üretir (metriklerden ayrı →
/// WF gibi çağrılar OOS dilimlerinin HAM getirisini birleştirebilir). evaluate_xs bunu sarar.
fn xs_returns(closes: &[Vec<Option<f64>>], cfg: &XsConfig) -> (Vec<f64>, Vec<f64>) {
    let n_bars = closes.len();
    let n_sym = if n_bars > 0 { closes[0].len() } else { 0 };
    if n_bars <= cfg.lookback + 1 || n_sym < 2 || cfg.top_k == 0 {
        return (Vec::new(), Vec::new());
    }

    let rb = cfg.rebalance_every.max(1);
    let mut prev_w = vec![0.0_f64; n_sym]; // mevcut hedef (turnover referansı)
    let mut step = 0usize;                 // POZİSYON kurulduktan sonra ilerleyen kadans saati
    let mut rets: Vec<f64> = Vec::new();
    let mut turnovers: Vec<f64> = Vec::new();

    for t in cfg.lookback..(n_bars - 1) {
        // Kadans: yalnız her rb bar'da hedefi yeniden hesapla; arada SABİT tut (periyodik-rebalance).
        let is_rebalance = step.is_multiple_of(rb);
        let w = if is_rebalance {
            target_weights(closes, t, cfg).unwrap_or_else(|| prev_w.clone()) // hesaplanamazsa tut
        } else {
            prev_w.clone()
        };
        // Henüz pozisyon yok (warmup: yeterli sembol yok) → kaydetme, kadans saatini başlatma.
        if w.iter().all(|x| *x == 0.0) {
            continue;
        }
        step += 1;

        // gerçekleşen sonraki-bar getirisi + turnover maliyeti (yalnız rebalance bar'ında)
        let mut port = 0.0_f64;
        for j in 0..n_sym {
            if w[j] != 0.0 {
                if let (Some(c0), Some(c1)) = (closes[t][j], closes[t + 1][j]) {
                    if c0 > 0.0 {
                        port += w[j] * (c1 / c0 - 1.0);
                    }
                }
            }
        }
        let turnover: f64 = if is_rebalance {
            (0..n_sym).map(|j| (w[j] - prev_w[j]).abs()).sum()
        } else {
            0.0
        };
        rets.push(port - turnover * cfg.fee_rate);
        turnovers.push(turnover);
        prev_w = w;
    }
    (rets, turnovers)
}

/// SAF: t anındaki kesitsel hedef ağırlık vektörü (long Σ=+1, short Σ=−1). Yeterli sembol yoksa None.
/// sinyal_j = close[t]/close[t−lookback]−1; sırala → momentum: top-k long/bottom-k short, reversal: ters.
fn target_weights(closes: &[Vec<Option<f64>>], t: usize, cfg: &XsConfig) -> Option<Vec<f64>> {
    let n_sym = closes[t].len();
    let mut sig: Vec<(usize, f64)> = Vec::with_capacity(n_sym);
    for (j, (now, past)) in closes[t].iter().zip(&closes[t - cfg.lookback]).enumerate() {
        if let (Some(c0), Some(cl)) = (now, past) {
            if *cl > 0.0 && *c0 > 0.0 {
                sig.push((j, c0 / cl - 1.0));
            }
        }
    }
    let need = if cfg.long_short { 2 * cfg.top_k } else { cfg.top_k };
    if sig.len() < need {
        return None;
    }
    sig.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let k = cfg.top_k;
    type Rank<'a> = &'a [(usize, f64)];
    let (longs, shorts): (Rank, Rank) = if cfg.momentum {
        (&sig[..k], &sig[sig.len() - k..])
    } else {
        (&sig[sig.len() - k..], &sig[..k])
    };
    let mut w = vec![0.0_f64; n_sym];
    let lw = 1.0 / k as f64;
    for &(j, _) in longs {
        w[j] += lw; // long bacak Σ=+1
    }
    if cfg.long_short {
        for &(j, _) in shorts {
            w[j] -= lw; // short bacak Σ=−1
        }
    }
    Some(w)
}

/// Net-getiri serisinden tüm metrikleri doldurur (saf, ayrı → testlenir).
fn finalize_metrics(res: &mut XsResult, rets: &[f64], turnovers: &[f64], cfg: &XsConfig) {
    let n = rets.len();
    res.bars = n;
    if n == 0 {
        return;
    }
    let mean = rets.iter().sum::<f64>() / n as f64;
    let var = if n > 1 {
        rets.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0)
    } else {
        0.0
    };
    let std = var.sqrt();
    res.mean_ret = mean;
    res.std_ret = std;
    res.win_rate = rets.iter().filter(|r| **r > 0.0).count() as f64 / n as f64;
    res.total_return = rets.iter().fold(1.0, |acc, r| acc * (1.0 + r)) - 1.0;
    res.ann_return = mean * cfg.bars_per_year;
    res.ann_sharpe = if std > 0.0 { mean / std * cfg.bars_per_year.sqrt() } else { 0.0 };
    res.t_stat = if std > 0.0 { mean / (std / (n as f64).sqrt()) } else { 0.0 };
    res.avg_turnover = turnovers.iter().sum::<f64>() / n as f64;
    res.wf = windowed_consistency(rets, cfg.wf_window);
}

/// Getiri serisini `window` uzunluklu ARDIŞIK (örtüşmesiz) parçalara böler → WfCrossCheck.
/// profitable_window = parça toplamı>0; pooled_pf = Σ(+getiri)/|Σ(−getiri)|. trades = bar sayısı.
/// edge_scan'le AYNI WfCrossCheck → aynı `window_significance()` binom kapısı uygulanabilir (DRY).
fn windowed_consistency(rets: &[f64], window: usize) -> WfCrossCheck {
    let w = window.max(1);
    let mut windows = 0usize;
    let mut profitable = 0usize;
    for chunk in rets.chunks(w) {
        // son parça yarım olabilir → yine de tutarlılık örneği say (muhafazakâr: az pencere → büyük p)
        windows += 1;
        if chunk.iter().sum::<f64>() > 0.0 {
            profitable += 1;
        }
    }
    let gains: f64 = rets.iter().filter(|r| **r > 0.0).sum();
    let losses: f64 = rets.iter().filter(|r| **r < 0.0).map(|r| r.abs()).sum();
    let pooled_pf = if losses > 0.0 { gains / losses } else if gains > 0.0 { 999.0 } else { 0.0 };
    WfCrossCheck { windows, profitable_windows: profitable, pooled_pf, trades: rets.len() }
}

/// DB'den sepet kapanışlarını ORTAK timestamp ızgarasına hizalar (union grid, eksik = None →
/// o bar o sembolü sıralamaya katmaz). Market-saf. evaluate_xs/WF bunu paylaşır (DRY).
pub fn align_closes(cfg: &XsConfig) -> Vec<Vec<Option<f64>>> {
    // sembol → (ts_ms → close)
    let mut per_sym: Vec<BTreeMap<i64, f64>> = Vec::with_capacity(cfg.symbols.len());
    let mut grid: BTreeMap<i64, ()> = BTreeMap::new();
    for sym in &cfg.symbols {
        let candles =
            read_candles_market(&cfg.db_path, sym, &cfg.interval, &cfg.market, cfg.candle_limit)
                .unwrap_or_default();
        let mut m = BTreeMap::new();
        for c in &candles {
            let ts = c.timestamp.timestamp_millis();
            m.insert(ts, c.close);
            grid.insert(ts, ());
        }
        per_sym.push(m);
    }
    let stamps: Vec<i64> = grid.keys().copied().collect();
    stamps
        .iter()
        .map(|ts| per_sym.iter().map(|m| m.get(ts).copied()).collect())
        .collect()
}

/// DB-YÜKLEYEN sürüm: kapanışları hizalar → `evaluate_xs`.
pub fn run_xs_momentum(cfg: &XsConfig) -> XsResult {
    evaluate_xs(&align_closes(cfg), cfg)
}

/// Rolling walk-forward kalibrasyonu: aday config kümesinden HER IS penceresinde en iyiyi (IS Sharpe)
/// seç → SONRAKİ OOS penceresine kör uygula → OOS getirilerini birleştir. OOS = sinyalin GÖRMEDİĞİ
/// veri → look-ahead'siz dürüst test. Sweep'in "tüm-veride en iyi config" optimizmini KESER.
#[derive(Debug, Clone)]
pub struct XsWfConfig {
    pub is_bars: usize,                 // kalibrasyon penceresi uzunluğu (bar)
    pub oos_bars: usize,                // kör-test penceresi (örtüşmesiz → OOS getiri çift-saymaz)
    pub candidates: Vec<(usize, bool)>, // (lookback, momentum) aday ızgara — IS bunlardan seçer
}

/// WF sonucu. `oos` = BİRLEŞTİRİLMİŞ OOS serisinde metrik (dürüst t-stat + binom). `selections` =
/// pencere başına seçilen config (kararlılık teşhisi). `is_oos_pairs` = (IS Sharpe, OOS ort. getiri).
#[derive(Debug, Clone, Default)]
pub struct XsWfResult {
    pub oos: XsResult,
    pub windows: usize,
    pub selections: Vec<(usize, bool)>,
    pub is_oos_pairs: Vec<(f64, f64)>,
}

/// SAF WF çekirdeği (hizalanmış matris üzerinde, DB'siz testlenir).
pub fn evaluate_xs_walkforward(
    closes: &[Vec<Option<f64>>],
    base: &XsConfig,
    wf: &XsWfConfig,
) -> XsWfResult {
    let n = closes.len();
    let n_sym = if n > 0 { closes[0].len() } else { 0 };
    let mut oos_rets: Vec<f64> = Vec::new();
    let mut oos_turn: Vec<f64> = Vec::new();
    let mut selections: Vec<(usize, bool)> = Vec::new();
    let mut is_oos_pairs: Vec<(f64, f64)> = Vec::new();

    let mut oos_start = wf.is_bars;
    while oos_start + wf.oos_bars <= n && !wf.candidates.is_empty() {
        let is_lo = oos_start.saturating_sub(wf.is_bars);
        // 1) IS'te en iyi adayı seç (IS Sharpe; n sabitken t-stat ile aynı sıralama).
        let mut best: Option<((usize, bool), f64)> = None;
        for &(lb, mom) in &wf.candidates {
            let cfg = XsConfig { lookback: lb, momentum: mom, ..base.clone() };
            let is_res = evaluate_xs(&closes[is_lo..oos_start], &cfg);
            if is_res.bars > 0 && best.is_none_or(|(_, s)| is_res.ann_sharpe > s) {
                best = Some(((lb, mom), is_res.ann_sharpe));
            }
        }
        let Some((sel, _)) = best else {
            oos_start += wf.oos_bars;
            continue;
        };
        // 2) seçileni OOS'a UYGULA (lookback lead-in dahil dilim, getiri yalnız OOS bölgesinde).
        let lead = sel.0.min(oos_start);
        let seg_hi = (oos_start + wf.oos_bars).min(n);
        let cfg = XsConfig { lookback: sel.0, momentum: sel.1, ..base.clone() };
        let (rets, turns) = xs_returns(&closes[oos_start - lead..seg_hi], &cfg);
        let oos_mean = if rets.is_empty() { 0.0 } else { rets.iter().sum::<f64>() / rets.len() as f64 };
        let is_sharpe = evaluate_xs(&closes[is_lo..oos_start],
            &XsConfig { lookback: sel.0, momentum: sel.1, ..base.clone() }).ann_sharpe;
        oos_rets.extend(rets);
        oos_turn.extend(turns);
        selections.push(sel);
        is_oos_pairs.push((is_sharpe, oos_mean));
        oos_start += wf.oos_bars;
    }

    let mut oos = XsResult { symbols_used: n_sym, ..Default::default() };
    finalize_metrics(&mut oos, &oos_rets, &oos_turn, base);
    XsWfResult { oos, windows: selections.len(), selections, is_oos_pairs }
}

/// DB-YÜKLEYEN WF sürümü: hizala → `evaluate_xs_walkforward`.
pub fn run_xs_walkforward(base: &XsConfig, wf: &XsWfConfig) -> XsWfResult {
    evaluate_xs_walkforward(&align_closes(base), base, wf)
}

#[cfg(test)]
mod tests {
    use super::*;

    // erf yaklaşımı → bilinen p-değerleri (tek-yanlı normal SF).
    #[test]
    fn t_pvalue_normal_known_quantiles() {
        let p = |z: f64| one_sided_normal_sf(z);
        assert!((p(0.0) - 0.5).abs() < 1e-6, "z=0 → 0.5");
        assert!((p(1.645) - 0.05).abs() < 1e-3, "z=1.645 → ~0.05");
        assert!((p(1.96) - 0.025).abs() < 1e-3, "z=1.96 → ~0.025");
        assert!((p(2.326) - 0.01).abs() < 1e-3, "z=2.326 → ~0.01");
        assert!(p(-1.645) > 0.94, "negatif z → büyük p (kuyruk diğer yanda)");
    }

    // SAF ÇEKİRDEK: deterministik "her zaman lider devam eder" matrisi → momentum POZİTİF spread.
    // 3 sembol: A sürekli yükselir, C sürekli düşer, B sabit. Momentum long A / short C → her bar +.
    #[test]
    fn evaluate_xs_momentum_captures_persistent_spread() {
        // 12 bar, A:×1.05/bar, B:sabit, C:×0.95/bar
        let mut a = 100.0;
        let mut c = 100.0;
        let mut closes = Vec::new();
        for _ in 0..12 {
            closes.push(vec![Some(a), Some(100.0), Some(c)]);
            a *= 1.05;
            c *= 0.95;
        }
        let cfg = XsConfig {
            symbols: vec!["A".into(), "B".into(), "C".into()],
            lookback: 2, top_k: 1, fee_rate: 0.0, momentum: true, long_short: true,
            wf_window: 3, ..Default::default()
        };
        let r = evaluate_xs(&closes, &cfg);
        assert!(r.bars > 0, "rebalance üretilmeli");
        assert!(r.mean_ret > 0.0, "kalıcı momentum → pozitif ortalama spread");
        assert!(r.t_stat > 0.0 && r.win_rate > 0.9, "her bar long-A/short-C kazanır");
        // reversal AYNI veride simetrik NEGATİF olmalı (mekanizma yön-duyarlı)
        let rev = evaluate_xs(&closes, &XsConfig { momentum: false, ..cfg.clone() });
        assert!(rev.mean_ret < 0.0, "reversal aynı trend-veride kaybeder (yön simetrisi)");
    }

    // Maliyet turnover'a bağlı: sıfır-fee kârlı kurulum, yüksek fee'de net NEGATİFE döner.
    #[test]
    fn evaluate_xs_fees_erode_via_turnover() {
        // long/short her bar yer değiştiren testere → yüksek turnover; küçük spread.
        let closes = vec![
            vec![Some(100.0), Some(100.0)],
            vec![Some(101.0), Some(100.0)],
            vec![Some(100.0), Some(101.0)],
            vec![Some(101.0), Some(100.0)],
            vec![Some(100.0), Some(101.0)],
            vec![Some(101.0), Some(100.0)],
        ];
        let base = XsConfig {
            symbols: vec!["A".into(), "B".into()],
            lookback: 1, top_k: 1, momentum: true, long_short: true, wf_window: 2,
            fee_rate: 0.0, ..Default::default()
        };
        let free = evaluate_xs(&closes, &base);
        let costly = evaluate_xs(&closes, &XsConfig { fee_rate: 0.05, ..base.clone() });
        assert!(costly.total_return < free.total_return, "fee turnover üzerinden getiriyi aşındırır");
        assert!(costly.avg_turnover > 0.0, "yer değiştiren sepet → pozitif turnover");
    }

    // Rebalance kadansı turnover'ı kısar: aynı testere veride rb=3, rb=1'den daha az turnover öder.
    #[test]
    fn rebalance_cadence_cuts_turnover() {
        // her bar yer değiştiren testere → rb=1 her bar turnover öder; rb=3 yalnız 3 bar'da bir.
        let mut closes = Vec::new();
        for i in 0..18 {
            // A ve B sırayla lider olur → her bar rank flip (maksimum churn)
            if i % 2 == 0 { closes.push(vec![Some(101.0), Some(100.0)]); }
            else { closes.push(vec![Some(100.0), Some(101.0)]); }
        }
        let base = XsConfig {
            symbols: vec!["A".into(), "B".into()],
            lookback: 1, top_k: 1, momentum: true, long_short: true, wf_window: 3,
            fee_rate: 0.001, ..Default::default()
        };
        let daily = evaluate_xs(&closes, &XsConfig { rebalance_every: 1, ..base.clone() });
        let held = evaluate_xs(&closes, &XsConfig { rebalance_every: 3, ..base.clone() });
        assert!(held.avg_turnover < daily.avg_turnover,
            "kadans büyüdükçe ortalama turnover düşer ({} < {})", held.avg_turnover, daily.avg_turnover);
    }

    // windowed_consistency edge_scan'le aynı WfCrossCheck → window_significance() uygulanabilir.
    #[test]
    fn windowed_consistency_feeds_binomial_gate() {
        // 9 getiri, 3'erli 3 pencere; hepsi pozitif-toplam → 3/3, p = 0.125.
        let rets = vec![0.01, 0.02, -0.005, 0.01, 0.01, 0.01, 0.02, -0.01, 0.03];
        let wf = windowed_consistency(&rets, 3);
        assert_eq!(wf.windows, 3);
        assert_eq!(wf.profitable_windows, 3, "üç pencerenin toplamı da pozitif");
        assert!(wf.pooled_pf > 1.0, "net kârlı seri → PF>1");
        assert!((wf.window_significance() - 0.125).abs() < 1e-9, "3/3 → 0.5³ = 0.125");
    }

    // WF: kalıcı momentum trendinde IS momentum'u seçer, OOS POZİTİF döner (look-ahead'siz).
    #[test]
    fn walkforward_selects_and_validates_persistent_momentum() {
        // 120 bar: A↑, C↓ kalıcı → her IS penceresinde momentum kazanır, OOS'ta da kazanmalı.
        let mut a = 100.0;
        let mut c = 100.0;
        let mut closes = Vec::new();
        for _ in 0..120 {
            closes.push(vec![Some(a), Some(100.0), Some(c)]);
            a *= 1.03;
            c *= 0.97;
        }
        let base = XsConfig {
            symbols: vec!["A".into(), "B".into(), "C".into()],
            top_k: 1, fee_rate: 0.0, long_short: true, wf_window: 10, ..Default::default()
        };
        let wf = XsWfConfig {
            is_bars: 40, oos_bars: 20,
            candidates: vec![(3, true), (3, false), (7, true), (7, false)], // mom + rev adayları
        };
        let r = evaluate_xs_walkforward(&closes, &base, &wf);
        assert!(r.windows >= 2, "birden çok OOS penceresi üretilmeli");
        assert!(r.oos.bars > 0 && r.oos.mean_ret > 0.0, "OOS getiri pozitif (trend kör-test'te sürer)");
        assert!(r.selections.iter().all(|(_, mom)| *mom),
            "kalıcı trendde her pencere MOMENTUM seçmeli (reversal değil)");
    }

    // Yetersiz veri / dejenere giriş → boş ama panik-yok sonuç.
    #[test]
    fn evaluate_xs_guards_degenerate_input() {
        let cfg = XsConfig { symbols: vec!["A".into()], lookback: 5, ..Default::default() };
        assert_eq!(evaluate_xs(&[], &cfg).bars, 0, "boş matris → 0 bar");
        let short = vec![vec![Some(1.0), Some(1.0)]; 3];
        assert_eq!(evaluate_xs(&short, &cfg).bars, 0, "lookback'ten kısa → 0 bar");
    }
}
