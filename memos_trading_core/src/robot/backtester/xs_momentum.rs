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

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::core::types::Candle;
use crate::persistence::reader::read_candles_market;
use crate::robot::sr_detector::{SrDetector, SrDetectorConfig, SrZone, ZoneType};
use super::walk_forward::WfCrossCheck;

/// Kesitsel sıralama EKSENİ (sinyal kaynağı). Hepsinde "yüksek skor = long bacak" olacak şekilde
/// normalize edilir (düşük-uç long isteyen eksenlerde işaret ters) → sıralama/select_books/ağırlık
/// makinesi DEĞİŞMEZ, yalnız HANGİ sinyalle sıralandığı değişir. `momentum=false` her eksende yönü
/// çevirir (A/B). Momentum dışındaki eksenler `lookback`'i geriye-dönük PENCERE olarak kullanır.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum XsSignal {
    /// close[t]/close[t−lookback]−1 (yüksek = güçlü → long). Default (geri-uyum, sıfır regresyon).
    #[default]
    Momentum,
    /// −realized_vol(lookback) → DÜŞÜK vol long / yüksek vol short (low-vol anomali).
    LowVol,
    /// −β (sepet-ortalamasına regresyon, lookback pencere) → DÜŞÜK beta long (betting-against-beta).
    Beta,
    /// −max(günlük getiri, lookback) → "piyango" (aşırı tek-bar getirili) isimleri SHORT (MAX etkisi).
    MaxLottery,
}

/// Kesitsel strateji parametreleri. Sepet + sinyal ekseni + yön + maliyet + anlamlılık penceresi.
#[derive(Debug, Clone)]
pub struct XsConfig {
    pub db_path: String,
    pub market: String,
    pub interval: String,
    /// Sepet sembolleri (majörler). En az 2*top_k gerekir.
    pub symbols: Vec<String>,
    pub candle_limit: usize,
    /// Kesitsel sıralama ekseni (Momentum | LowVol | Beta | MaxLottery). Default Momentum.
    pub signal: XsSignal,
    /// Geriye-bakış (bar): Momentum'da sinyal_s = close[t]/close[t−lookback]−1; diğer eksenlerde
    /// istatistiğin (vol/β/max) hesaplandığı geriye-dönük pencere.
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
    /// NO-TRADE BAND (rank-histerezisi): mevcut bir pozisyonu, sıralamadan `top_k + exit_buffer` dışına
    /// DÜŞENE kadar TUT (marjinal daha-iyi aday için churn etme). 0 = band yok (saf top-k, eski davranış).
    /// Kadanstan AKILLI: girişi zamana değil sinyal-anlamlılığına bağlar → turnover'ı net'i bozmadan kısar.
    pub exit_buffer: usize,
    /// Kaldıraç (gross-exposure çarpanı). Market-nötr L/S'te getiriyi VE turnover-maliyetini eşit ölçekler
    /// → t-stat/Sharpe DEĞİŞMEZ (anlamlılık kaldıraçla üretilemez); yalnız bileşik büyümeyi ölçekler +
    /// volatilite-sürüklenmesi ekler (L² ile). Risk-hedefleme knob'u, edge knob'u DEĞİL. Default 1.0.
    pub leverage: f64,
    /// Anlamlılık penceresi (bar): return serisini bu uzunlukta ARDIŞIK parçalara böler → WfCrossCheck.
    pub wf_window: usize,
    /// Yıllık bar sayısı (Sharpe/yıllık-getiri annualize). 1d → 365, 1h → 8760.
    pub bars_per_year: f64,
    /// 🧱 S/R EĞİMİ (opt-in, default 0 = KAPALI = close-only eski yol birebir). Kesitsel skoru sıralama
    /// ÖNCESİ S/R-karşıtlığına göre 0'a doğru kısar: long-eğilimli aday yakın güçlü DİRENCİN altındaysa
    /// (veya short-eğilimli aday güçlü DESTEĞİN üstündeyse) skor·(1 − sr_tilt·karşıtlık). Kapı DEĞİL eğim →
    /// market-nötr k-long/k-short dengesi korunur (yalnız HANGİ sembol seçilir değişir). OHLC ister
    /// (align_ohlc); 0 ise OHLC hiç yüklenmez. A/B kanıtlanmadan canlıya bağlanmaz. [[project_sr_display_only]]
    pub sr_tilt: f64,
    /// S/R yakınlık bandı (%): fiyata bu kadar %'den yakın engelleyici zone karşıtlık=1'e yaklaşır, band
    /// dışı → 0. Default 3.0.
    pub sr_band_pct: f64,
    /// Yalnız bu güç-eşiğinin üstündeki zone'lar karşıtlık sayılır (SrDetector kendi min_strength'ini de
    /// uygular). Default 0 = dedektörün döndürdüğü tüm zone'lar.
    pub sr_min_strength: f64,
    /// S/R tespiti için sembol başına geriye-dönük mum penceresi (bar). Default 120 (~4 ay 1d).
    pub sr_window: usize,
}

impl Default for XsConfig {
    fn default() -> Self {
        Self {
            db_path: "data/trader.db".into(),
            market: "futures".into(),
            interval: "1d".into(),
            symbols: Vec::new(),
            candle_limit: 5000,
            signal: XsSignal::Momentum,
            lookback: 7,
            top_k: 3,
            fee_rate: 0.0005, // 5 bps / turnover birimi (round-turn ~10bps futures taker+slippage)
            momentum: true,
            long_short: true,
            rebalance_every: 1, // her bar (geri-uyumlu); yavaş sinyalde XS_REBALANCE>1 ile churn kıs
            exit_buffer: 0,     // band yok (saf top-k); >0 ile rank-histerezisi turnover'ı kısar
            leverage: 1.0,      // risk-hedefleme; Sharpe/t-invariant
            wf_window: 30,    // ~aylık pencere (1d) → tutarlılık binom kapısı
            bars_per_year: 365.0,
            sr_tilt: 0.0,       // S/R eğimi KAPALI (opt-in; default = close-only eski yol birebir)
            sr_band_pct: 3.0,
            sr_min_strength: 0.0,
            sr_window: 120,
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
    pub t_stat: f64,         // mean / (std/√N) — tek-yanlı (H0: getiri≤0), OTOKORELASYON DÜZELTMESİZ
    pub nw_t_stat: f64,      // Newey-West HAC t — band/kadans tutuşu otokorelasyon → naif t'yi şişirir; bu düzeltir
    pub nw_lag: usize,       // kullanılan Bartlett bant-genişliği (truncation lag)
    pub avg_turnover: f64,
    pub wf: WfCrossCheck,    // pencere-tutarlılığı → window_significance() binom p-değeri
}

impl XsResult {
    /// Naif t-stat'ı normal-yaklaşımla p-değerine çevirir (büyük N; H0: ortalama getiri ≤ 0).
    /// Otokorelasyon düzeltmesi YOK → örtüşen/tutulan getiride İYİMSER (şişmiş). Karşılaştırma için.
    pub fn t_pvalue(&self) -> f64 { one_sided_normal_sf(self.t_stat) }
    /// Newey-West HAC t-stat'ın tek-yanlı p-değeri — DÜRÜST anlamlılık (otokorelasyona dayanıklı).
    pub fn nw_t_pvalue(&self) -> f64 { one_sided_normal_sf(self.nw_t_stat) }
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
    evaluate_xs_with_ohlc(closes, None, cfg)
}

/// `evaluate_xs`'in OHLC-FARKINDA sürümü: S/R eğimi (`cfg.sr_tilt`>0) için `closes` ile birebir hizalı
/// OHLC matrisi alır. `ohlc=None` → close-only (evaluate_xs ile BİREBİR, sıfır regresyon); `Some` +
/// sr_tilt>0 → compute_weights sıralama-öncesi S/R eğimini uygular.
pub fn evaluate_xs_with_ohlc(
    closes: &[Vec<Option<f64>>], ohlc: Option<&[Vec<Option<Candle>>]>, cfg: &XsConfig,
) -> XsResult {
    let n_sym = if closes.is_empty() { 0 } else { closes[0].len() };
    let mut res = XsResult { symbols_used: n_sym, ..Default::default() };
    let (rets, turnovers) = xs_returns(closes, ohlc, cfg);
    finalize_metrics(&mut res, &rets, &turnovers, cfg);
    res
}

/// SAF: kesitsel portföyün bar-bar NET getiri + turnover serisini üretir (metriklerden ayrı →
/// WF gibi çağrılar OOS dilimlerinin HAM getirisini birleştirebilir). evaluate_xs bunu sarar.
fn xs_returns(
    closes: &[Vec<Option<f64>>], ohlc: Option<&[Vec<Option<Candle>>]>, cfg: &XsConfig,
) -> (Vec<f64>, Vec<f64>) {
    let n_bars = closes.len();
    let n_sym = if n_bars > 0 { closes[0].len() } else { 0 };
    if n_bars <= cfg.lookback + 1 || n_sym < 2 || cfg.top_k == 0 {
        return (Vec::new(), Vec::new());
    }

    let rb = cfg.rebalance_every.max(1);
    let mut prev_w = vec![0.0_f64; n_sym]; // mevcut hedef (turnover referansı)
    let mut prev_long: HashSet<usize> = HashSet::new();  // mevcut long kitabı (histerezis için)
    let mut prev_short: HashSet<usize> = HashSet::new();
    let mut step = 0usize;                 // POZİSYON kurulduktan sonra ilerleyen kadans saati
    let mut rets: Vec<f64> = Vec::new();
    let mut turnovers: Vec<f64> = Vec::new();

    for t in cfg.lookback..(n_bars - 1) {
        // Kadans: yalnız her rb bar'da hedefi yeniden hesapla; arada SABİT tut (periyodik-rebalance).
        let is_rebalance = step.is_multiple_of(rb);
        let mut w = prev_w.clone();
        let mut changed = false;
        if is_rebalance {
            // No-trade band: MEVCUT kitabı (prev_long/short) histerezisle koruyarak yeni hedef.
            if let Some((tw, longs, shorts)) = compute_weights(closes, ohlc, t, cfg, &prev_long, &prev_short) {
                w = tw;
                prev_long = longs;
                prev_short = shorts;
                changed = true;
            }
        }
        // Henüz pozisyon yok (warmup: yeterli sembol yok) → kaydetme, kadans saatini başlatma.
        if w.iter().all(|x| *x == 0.0) {
            continue;
        }
        step += 1;

        // gerçekleşen sonraki-bar getirisi + turnover maliyeti (yalnız ağırlık değiştiğinde)
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
        // band çoğu rebalance'ta kitabı korur → w≈prev_w → turnover doğal olarak küçülür.
        let turnover: f64 = if changed {
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

/// SAF: bir sembolün t'de biten `lb`-bar penceresindeki bar-bar getirileri (Option; eksik/≤0 → None).
/// Uzunluk = lb. `window_returns_opt` Beta market-hizalamasında ve vol/max'ta ortak kullanılır.
fn window_returns_opt(closes: &[Vec<Option<f64>>], t: usize, lb: usize, j: usize) -> Vec<Option<f64>> {
    let start = (t + 1).saturating_sub(lb);
    (start..=t).map(|b| {
        if b == 0 { return None; }
        match (closes[b][j], closes[b - 1][j]) {
            (Some(c1), Some(c0)) if c0 > 0.0 && c1 > 0.0 => Some(c1 / c0 - 1.0),
            _ => None,
        }
    }).collect()
}

/// SAF: popülasyon std (BB konvansiyonuyla aynı). <2 örnek → 0.
fn stddev(v: &[f64]) -> f64 {
    let n = v.len();
    if n < 2 { return 0.0; }
    let m = v.iter().sum::<f64>() / n as f64;
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / n as f64).sqrt()
}

/// SAF: hizalı (rj, rm) Option çiftlerinden β = cov(rj,rm)/var(rm). Ortak nokta <2 veya var(rm)=0 → None.
fn beta_paired(rj: &[Option<f64>], rm: &[Option<f64>]) -> Option<f64> {
    let pairs: Vec<(f64, f64)> = rj.iter().zip(rm)
        .filter_map(|(a, b)| match (a, b) { (Some(x), Some(y)) => Some((*x, *y)), _ => None })
        .collect();
    let n = pairs.len();
    if n < 2 { return None; }
    let mx = pairs.iter().map(|p| p.0).sum::<f64>() / n as f64;
    let my = pairs.iter().map(|p| p.1).sum::<f64>() / n as f64;
    let cov = pairs.iter().map(|(x, y)| (x - mx) * (y - my)).sum::<f64>() / n as f64;
    let var = pairs.iter().map(|(_, y)| (y - my).powi(2)).sum::<f64>() / n as f64;
    if var <= 0.0 { None } else { Some(cov / var) }
}

/// SAF: t anında her sembol için kesitsel sinyal `(idx, skor)` — yüksek skor = LONG bacak. Eksene göre
/// dallanır; Momentum dışı eksenler `lookback`'i geriye-dönük pencere alır. Skoru hesaplanamayan
/// sembol listeye girmez (compute_weights'in eski None-atlama mantığıyla birebir). Saf → testli.
fn build_signals(closes: &[Vec<Option<f64>>], t: usize, cfg: &XsConfig) -> Vec<(usize, f64)> {
    let n_sym = closes[t].len();
    let lb = cfg.lookback.max(1);
    match cfg.signal {
        XsSignal::Momentum => {
            // close[t]/close[t−lb]−1 (eski yol birebir).
            let mut sig = Vec::with_capacity(n_sym);
            for (j, (now, past)) in closes[t].iter().zip(&closes[t - lb]).enumerate() {
                if let (Some(c0), Some(cl)) = (now, past) {
                    if *cl > 0.0 && *c0 > 0.0 { sig.push((j, c0 / cl - 1.0)); }
                }
            }
            sig
        }
        XsSignal::LowVol => {
            // −realized_vol(lb): düşük-vol uca yüksek skor → long.
            let mut sig = Vec::with_capacity(n_sym);
            for j in 0..n_sym {
                let vals: Vec<f64> = window_returns_opt(closes, t, lb, j).into_iter().flatten().collect();
                if vals.len() >= 2 { sig.push((j, -stddev(&vals))); }
            }
            sig
        }
        XsSignal::MaxLottery => {
            // −max(günlük getiri, lb): "piyango" (yüksek aşırı-getiri) düşük skor → short.
            let mut sig = Vec::with_capacity(n_sym);
            for j in 0..n_sym {
                let vals: Vec<f64> = window_returns_opt(closes, t, lb, j).into_iter().flatten().collect();
                if !vals.is_empty() {
                    let mx = vals.iter().cloned().fold(f64::MIN, f64::max);
                    sig.push((j, -mx));
                }
            }
            sig
        }
        XsSignal::Beta => {
            // β = sepet-ortalaması (eşit-ağırlık) market getirisine regresyon → −β long (BAB).
            let wr: Vec<Vec<Option<f64>>> = (0..n_sym)
                .map(|j| window_returns_opt(closes, t, lb, j)).collect();
            // Market[pos] = pencere pozisyonunda mevcut sembollerin ortalama getirisi.
            let mkt: Vec<Option<f64>> = (0..lb).map(|pos| {
                let xs: Vec<f64> = wr.iter().filter_map(|r| r.get(pos).copied().flatten()).collect();
                if xs.is_empty() { None } else { Some(xs.iter().sum::<f64>() / xs.len() as f64) }
            }).collect();
            let mut sig = Vec::with_capacity(n_sym);
            for (j, rj) in wr.iter().enumerate() {
                if let Some(b) = beta_paired(rj, &mkt) { sig.push((j, -b)); }
            }
            sig
        }
    }
}

/// SAF: t anındaki kesitsel hedef ağırlık + (long_set, short_set). Yeterli sembol yoksa None.
/// Sinyal `build_signals` ile eksene göre üretilir (yüksek=long); reversal'da ters çevrilir.
/// `select_books` rank-histerezisiyle mevcut kitabı korur (no-trade band). long Σ=+1, short Σ=−1.
fn compute_weights(
    closes: &[Vec<Option<f64>>], ohlc: Option<&[Vec<Option<Candle>>]>, t: usize, cfg: &XsConfig,
    prev_long: &HashSet<usize>, prev_short: &HashSet<usize>,
) -> Option<(Vec<f64>, HashSet<usize>, HashSet<usize>)> {
    let n_sym = closes[t].len();
    let mut sig: Vec<(usize, f64)> = build_signals(closes, t, cfg);
    let need = if cfg.long_short { 2 * cfg.top_k } else { cfg.top_k };
    if sig.len() < need {
        return None;
    }
    // 🧱 S/R EĞİMİ (opt-in): SIRALAMA ÖNCESİ skoru S/R-karşıtlığına göre 0'a doğru kıs → karşıtı yüksek
    // sembol sıralamada uca çıkamaz (top/bottom-k dışına düşer). KAPI DEĞİL eğim: k-long/k-short sayısı
    // korunur → market-nötrlük bozulmaz. ohlc=None || sr_tilt=0 → atla (close-only birebir). dir: momentum'da
    // pozitif skor=long-eğilim; reversal'da ters (sig.reverse aşağıda yönü çevirir, eğim ham skor·dir ile).
    if let (Some(ohlc), true) = (ohlc, cfg.sr_tilt > 0.0) {
        let detector = SrDetector::new(SrDetectorConfig::default());
        let dir = if cfg.momentum { 1.0 } else { -1.0 };
        for (j, s) in sig.iter_mut() {
            let price = match closes[t][*j] { Some(p) if p > 0.0 => p, _ => continue };
            let window = collect_sym_window(ohlc, t, *j, cfg.sr_window);
            if window.len() < 2 { continue; }
            let zones = detector.detect(&window);
            if zones.is_empty() { continue; }
            let lean_long = (*s * dir) >= 0.0;
            let opp = sr_opposition(&zones, price, lean_long, cfg.sr_band_pct, cfg.sr_min_strength);
            *s *= 1.0 - cfg.sr_tilt * opp; // karşıtlık büyükse skor 0'a çekilir → uçtan geri düşer
        }
    }
    sig.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    if !cfg.momentum {
        sig.reverse(); // reversal: zayıf uç başa gelsin → "long edilecek" hep baş (histerezis simetrik)
    }
    let (longs, shorts) = select_books(&sig, cfg.top_k, cfg.exit_buffer, prev_long, prev_short);
    if longs.is_empty() {
        return None;
    }
    let mut w = vec![0.0_f64; n_sym];
    let lw = 1.0 / longs.len() as f64;
    for &j in &longs {
        w[j] += lw; // long bacak Σ=+1
    }
    if cfg.long_short && !shorts.is_empty() {
        let sw = 1.0 / shorts.len() as f64;
        for &j in &shorts {
            w[j] -= sw; // short bacak Σ=−1
        }
    }
    Some((w, longs.into_iter().collect(), shorts.into_iter().collect()))
}

/// SAF: `ohlc` matrisinden sembol `j` için t'de biten son `window` barlık BİTİŞİK mum dizisini toplar
/// (None'ları atlar → hizalama boşlukları S/R'yi bozmaz). Look-ahead'siz (yalnız ≤t; t barı kapalı).
fn collect_sym_window(ohlc: &[Vec<Option<Candle>>], t: usize, j: usize, window: usize) -> Vec<Candle> {
    let lo = (t + 1).saturating_sub(window.max(1));
    (lo..=t)
        .filter_map(|b| ohlc.get(b).and_then(|row| row.get(j)).and_then(|c| c.clone()))
        .collect()
}

/// SAF: işlem yönüne KARŞI duran en yakın güçlü S/R'nin yakınlığı ∈[0,1] (0=engel yok/uzak, 1=fiyat tam
/// engelde). long-eğilim → ÜSTteki DİRENÇ karşıt; short-eğilim → ALTtaki DESTEK karşıt. Engelin fiyata
/// bakan kenarı kullanılır (direnç alt=price_low, destek üst=price_high); band içinde lineer
/// (1 − mesafe%/band), band dışı 0, fiyat zone İÇİNDEyse 1. Yanlış taraftaki zone karşıt değil. Testli.
fn sr_opposition(zones: &[SrZone], price: f64, lean_long: bool, band_pct: f64, min_strength: f64) -> f64 {
    if price <= 0.0 || band_pct <= 0.0 {
        return 0.0;
    }
    let mut best = 0.0_f64;
    for z in zones {
        if z.strength < min_strength {
            continue;
        }
        let opposes = if lean_long {
            matches!(z.zone_type, ZoneType::Resistance)
        } else {
            matches!(z.zone_type, ZoneType::Support)
        };
        if !opposes {
            continue;
        }
        let prox = if z.contains(price) {
            1.0
        } else if lean_long && z.price_low > price {
            (1.0 - (z.price_low - price) / price * 100.0 / band_pct).max(0.0)
        } else if !lean_long && z.price_high < price {
            (1.0 - (price - z.price_high) / price * 100.0 / band_pct).max(0.0)
        } else {
            0.0 // engel yanlış tarafta (long için altta kalan direnç vb.) → karşıt değil
        };
        best = best.max(prox);
    }
    best.clamp(0.0, 1.0)
}

/// CANLI hedef-kitap skorlayıcısı (DRY: backtest `select_books`'in sembol-anahtarlı sarmalayıcısı).
/// `signals` = (sembol, momentum_sinyali=close[t]/close[t−lb]−1). Mevcut kitabı (prev_long/short)
/// rank-histerezisiyle koruyarak hedef long/short sembol listelerini üretir → canlı motor bunu çağırıp
/// hedef yönleri infaz eder. momentum=false → reversal (sinyal ters). Backtest ile BİT-AYNI seçim mantığı.
pub fn xs_target_book(
    signals: &[(String, f64)], top_k: usize, exit_buffer: usize, momentum: bool,
    prev_long: &HashSet<String>, prev_short: &HashSet<String>,
) -> (Vec<String>, Vec<String>) {
    // sembol↔indeks köprüsü; sinyale göre GÜÇ-AZALAN sırala (reversal → ters), select_books ile aynı.
    let mut idx_sig: Vec<(usize, f64)> =
        signals.iter().enumerate().map(|(i, (_, s))| (i, *s)).collect();
    idx_sig.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    if !momentum {
        idx_sig.reverse();
    }
    let sym_of = |i: usize| signals[i].0.clone();
    let to_idx = |set: &HashSet<String>| -> HashSet<usize> {
        signals.iter().enumerate().filter(|(_, (s, _))| set.contains(s)).map(|(i, _)| i).collect()
    };
    let (longs, shorts) =
        select_books(&idx_sig, top_k, exit_buffer, &to_idx(prev_long), &to_idx(prev_short));
    (longs.into_iter().map(sym_of).collect(), shorts.into_iter().map(&sym_of).collect())
}

/// RANK-HİSTEREZİSİ (no-trade band): tam-k long/short kitabı seçer ama MEVCUT üyeleri `k+buffer`
/// dışına düşene dek TUTAR (marjinal daha-iyi aday için churn YOK). buffer=0 → saf top-k/bottom-k
/// (eski davranış birebir). `sig` GÜÇ-AZALAN sıralı (baş=long edilecek uç). Saf → testli.
pub(crate) fn select_books(
    sig: &[(usize, f64)], k: usize, buffer: usize,
    prev_long: &HashSet<usize>, prev_short: &HashSet<usize>,
) -> (Vec<usize>, Vec<usize>) {
    let n = sig.len();
    let k = k.min(n / 2);
    if k == 0 {
        return (Vec::new(), Vec::new());
    }
    let rank: HashMap<usize, usize> = sig.iter().enumerate().map(|(r, (i, _))| (*i, r)).collect();

    // LONG = baş (düşük rank). Önce k+buffer içinde kalan mevcut long'ları TUT, sonra en güçlülerden doldur.
    let mut longs: Vec<usize> = prev_long.iter().copied()
        .filter(|i| rank.get(i).is_some_and(|r| *r < k + buffer))
        .collect();
    longs.sort_by_key(|i| rank[i]);
    longs.truncate(k);
    for &(idx, _) in sig.iter() {
        if longs.len() >= k { break; }
        if !longs.contains(&idx) { longs.push(idx); }
    }

    // SHORT = kuyruk (yüksek rank). Önce n−(k+buffer)..n içinde kalan mevcut short'ları TUT (long'lar hariç),
    // sonra en zayıflardan (yüksek rank) doldur.
    let short_keep = n.saturating_sub(k + buffer);
    let mut shorts: Vec<usize> = prev_short.iter().copied()
        .filter(|i| rank.get(i).is_some_and(|r| *r >= short_keep) && !longs.contains(i))
        .collect();
    shorts.sort_by_key(|i| std::cmp::Reverse(rank[i]));
    shorts.truncate(k);
    for &(idx, _) in sig.iter().rev() {
        if shorts.len() >= k { break; }
        if !longs.contains(&idx) && !shorts.contains(&idx) { shorts.push(idx); }
    }
    (longs, shorts)
}

/// Net-getiri serisinden tüm metrikleri doldurur (saf, ayrı → testlenir). Kaldıraç (L) bar-getiriyi
/// L× ölçekler: mean/std/annRet L× → t_stat & Sharpe DEĞİŞMEZ (anlamlılık L-invariant). total_return
/// bileşik (1+L·r) → L'de NONLİNEER: volatilite-sürüklenmesi (L² ile) burada yakalanır. win_rate L-invariant.
fn finalize_metrics(res: &mut XsResult, rets: &[f64], turnovers: &[f64], cfg: &XsConfig) {
    finalize_metrics_params(res, rets, turnovers, cfg.leverage, cfg.bars_per_year,
        cfg.rebalance_every, cfg.wf_window);
}

/// `finalize_metrics`'in config-bağımsız çekirdeği — herhangi bir NET-getiri serisini (kesitsel,
/// BB-pool, vb.) AYNI metrik diline (Newey-West HAC + WF binom) döker. `hold_lag` = pozisyon tutuş
/// ufku (NW bant-genişliği alt sınırı; XS'te rebalance_every). DRY: XS ve bb_pool ortak çağırır.
pub(crate) fn finalize_metrics_params(
    res: &mut XsResult, rets: &[f64], turnovers: &[f64],
    leverage: f64, bars_per_year: f64, hold_lag: usize, wf_window: usize,
) {
    let n = rets.len();
    res.bars = n;
    if n == 0 {
        return;
    }
    let lev = leverage;
    let mean = lev * rets.iter().sum::<f64>() / n as f64;
    let var = if n > 1 {
        let m0 = rets.iter().sum::<f64>() / n as f64;
        lev * lev * rets.iter().map(|r| (r - m0).powi(2)).sum::<f64>() / (n as f64 - 1.0)
    } else {
        0.0
    };
    let std = var.sqrt();
    res.mean_ret = mean;
    res.std_ret = std;
    res.win_rate = rets.iter().filter(|r| **r > 0.0).count() as f64 / n as f64;
    // Bileşik: kaldıraçlı bar-getiri L·r → vol-sürüklenmesi (L>1'de büyüme ann.Ret'ten sapar).
    res.total_return = rets.iter().fold(1.0, |acc, r| acc * (1.0 + lev * r)) - 1.0;
    res.ann_return = mean * bars_per_year;
    res.ann_sharpe = if std > 0.0 { mean / std * bars_per_year.sqrt() } else { 0.0 };
    res.t_stat = if std > 0.0 { mean / (std / (n as f64).sqrt()) } else { 0.0 };
    // Newey-West HAC: band/kadans pozisyonu birkaç bar tuttuğundan getiriler otokorelasyonlu →
    // naif t şişer. Bant-genişliği NW(1994) plug-in kuralı, tutuş ufkuyla (hold_lag) ALTTAN
    // sınırlanır. Kaldıraç-invariant (μ,γ ortak ölçeklenir) → base rets üzerinde hesaplanır.
    let auto_lag = (4.0 * (n as f64 / 100.0).powf(2.0 / 9.0)).floor() as usize;
    let lag = auto_lag.max(hold_lag).max(1).min(n.saturating_sub(1));
    res.nw_lag = lag;
    res.nw_t_stat = newey_west_tstat(rets, lag);
    res.avg_turnover = turnovers.iter().sum::<f64>() / n as f64;
    res.wf = windowed_consistency(rets, wf_window);
}

/// SAF: Newey-West (Bartlett kernel, HAC) tek-yanlı t-stat'ı = mean / sqrt(S/n), S = uzun-dönem varyans
/// = γ₀ + 2·Σ_{l=1}^{lag} (1 − l/(lag+1))·γ_l, γ_l = otokovaryans. Pozitif otokorelasyon S'i şişirir →
/// SE büyür → t küçülür (naif t'nin örtüşme-iyimserliğini düzeltir). Kaldıraç-invariant. Saf → testli.
fn newey_west_tstat(rets: &[f64], lag: usize) -> f64 {
    let n = rets.len();
    if n < 2 {
        return 0.0;
    }
    let mean = rets.iter().sum::<f64>() / n as f64;
    let dev: Vec<f64> = rets.iter().map(|r| r - mean).collect();
    let gamma0 = dev.iter().map(|d| d * d).sum::<f64>() / n as f64;
    let mut s = gamma0;
    for l in 1..=lag.min(n - 1) {
        let w = 1.0 - l as f64 / (lag as f64 + 1.0); // Bartlett ağırlığı
        let mut g = 0.0;
        for t in l..n {
            g += dev[t] * dev[t - l];
        }
        s += 2.0 * w * g / n as f64; // 2·w_l·γ_l
    }
    if s <= 0.0 {
        return 0.0;
    }
    mean / (s / n as f64).sqrt() // var(mean) = S/n
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

/// `align_closes` ile AYNI union ts-ızgarasında hizalı OHLC matrisi (S/R eğimi için). closes ile
/// bar×sym indeksleri BİREBİR örtüşür (eksik = None). Market-saf. Yalnız sr_tilt>0'da yüklenir (maliyet).
pub fn align_ohlc(cfg: &XsConfig) -> Vec<Vec<Option<Candle>>> {
    let mut per_sym: Vec<BTreeMap<i64, Candle>> = Vec::with_capacity(cfg.symbols.len());
    let mut grid: BTreeMap<i64, ()> = BTreeMap::new();
    for sym in &cfg.symbols {
        let candles =
            read_candles_market(&cfg.db_path, sym, &cfg.interval, &cfg.market, cfg.candle_limit)
                .unwrap_or_default();
        let mut m = BTreeMap::new();
        for c in &candles {
            let ts = c.timestamp.timestamp_millis();
            m.insert(ts, c.clone());
            grid.insert(ts, ());
        }
        per_sym.push(m);
    }
    let stamps: Vec<i64> = grid.keys().copied().collect();
    stamps
        .iter()
        .map(|ts| per_sym.iter().map(|m| m.get(ts).cloned()).collect())
        .collect()
}

/// DB-YÜKLEYEN sürüm: kapanışları hizalar → `evaluate_xs`. sr_tilt>0 ise OHLC de yüklenir → S/R eğimi.
pub fn run_xs_momentum(cfg: &XsConfig) -> XsResult {
    let closes = align_closes(cfg);
    if cfg.sr_tilt > 0.0 {
        let ohlc = align_ohlc(cfg);
        evaluate_xs_with_ohlc(&closes, Some(&ohlc), cfg)
    } else {
        evaluate_xs(&closes, cfg)
    }
}

/// Herhangi bir NET-getiri serisinden XsResult metrikleri (Newey-West HAC + WF binom + Sharpe).
/// Eksenler-arası BİRLEŞİK portföyü (ör. momentum+carry) ölçmek için pub sarmalayıcı —
/// finalize_metrics_params'ın dışa açık yüzü. `hold_lag` = NW bant-genişliği alt sınırı (rebalance).
pub fn series_metrics(
    rets: &[f64], turnovers: &[f64], leverage: f64, bars_per_year: f64, hold_lag: usize, wf_window: usize,
) -> XsResult {
    let mut res = XsResult::default();
    finalize_metrics_params(&mut res, rets, turnovers, leverage, bars_per_year, hold_lag, wf_window);
    res
}

/// DB-yükleyen NET-getiri serisi (metrik değil ham seri) — eksenler-arası DİKLİK kontrolü için
/// (iki eksenin getiri serileri düşük korelasyonluysa gerçekten ortogonal). turnover ikinci eleman.
pub fn run_xs_returns(cfg: &XsConfig) -> (Vec<f64>, Vec<f64>) {
    let closes = align_closes(cfg);
    if cfg.sr_tilt > 0.0 {
        let ohlc = align_ohlc(cfg);
        xs_returns(&closes, Some(&ohlc), cfg)
    } else {
        xs_returns(&closes, None, cfg)
    }
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
    ohlc: Option<&[Vec<Option<Candle>>]>,
    base: &XsConfig,
    wf: &XsWfConfig,
) -> XsWfResult {
    // closes ile birebir hizalı OHLC dilimi (S/R eğimi IS-seçim + OOS-uygulamada AYNI uygulanmalı → A/B sadık).
    let osl = |lo: usize, hi: usize| ohlc.map(|o| &o[lo..hi]);
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
            let is_res = evaluate_xs_with_ohlc(&closes[is_lo..oos_start], osl(is_lo, oos_start), &cfg);
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
        let (rets, turns) = xs_returns(&closes[oos_start - lead..seg_hi], osl(oos_start - lead, seg_hi), &cfg);
        let oos_mean = if rets.is_empty() { 0.0 } else { rets.iter().sum::<f64>() / rets.len() as f64 };
        let is_sharpe = evaluate_xs_with_ohlc(&closes[is_lo..oos_start], osl(is_lo, oos_start),
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
    let closes = align_closes(base);
    if base.sr_tilt > 0.0 {
        let ohlc = align_ohlc(base);
        evaluate_xs_walkforward(&closes, Some(&ohlc), base, wf)
    } else {
        evaluate_xs_walkforward(&closes, None, base, wf)
    }
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

    // NO-TRADE BAND: histerezis mevcut pozisyonu marjinal daha-iyi aday için churn ETMEZ.
    #[test]
    fn select_books_buffer_retains_incumbent() {
        // sig güç-azalan: idx 10,11,12,13 → rank 0,1,2,3. k=2.
        let sig = vec![(10usize, 0.4), (11, 0.3), (12, 0.2), (13, 0.1)];
        let held_long: HashSet<usize> = [12].into_iter().collect(); // rank 2 = top-k DIŞINDA
        let empty = HashSet::new();
        // buffer=0 → saf top-2: incumbent 12 churn'lenir, kitap {10,11}.
        let (l0, _) = select_books(&sig, 2, 0, &held_long, &empty);
        assert!(l0.contains(&10) && l0.contains(&11) && !l0.contains(&12),
            "band yok (buffer=0) → eski top-k davranışı: incumbent churn'lenir");
        // buffer=1 → 12 (rank2<3) TUTULUR; kalan slot en güçlü (10) ile dolar → {12,10}, 11 ALINMAZ.
        let (l1, _) = select_books(&sig, 2, 1, &held_long, &empty);
        assert!(l1.contains(&12) && l1.contains(&10) && !l1.contains(&11),
            "band: incumbent (rank2) tutulur, marjinal daha-iyi (11) için churn YOK");
    }

    // Band turnover'ı kısar: dalgalanan sıralamada buffer>0, buffer=0'dan az turnover öder ama kazancı korur.
    #[test]
    fn buffer_cuts_turnover_on_noisy_ranking() {
        // 3 sembol, sıralama bar-bar gürültülü oynar → buffer churn'ü emer.
        let mut closes = Vec::new();
        let mut a = 100.0; let mut b = 100.0; let mut c = 100.0;
        for i in 0..40 {
            closes.push(vec![Some(a), Some(b), Some(c)]);
            // A genel yukarı trend + gürültü; B,C zayıf → sıralama uçlarda gürültüyle takas olur
            a *= 1.02; b *= if i % 2 == 0 { 1.005 } else { 0.995 }; c *= 0.99;
        }
        let base = XsConfig {
            symbols: vec!["A".into(), "B".into(), "C".into()],
            top_k: 1, lookback: 2, fee_rate: 0.001, wf_window: 5, ..Default::default()
        };
        let no_band = evaluate_xs(&closes, &XsConfig { exit_buffer: 0, ..base.clone() });
        let band = evaluate_xs(&closes, &XsConfig { exit_buffer: 1, ..base.clone() });
        assert!(band.avg_turnover <= no_band.avg_turnover,
            "band turnover'ı azaltır ({} ≤ {})", band.avg_turnover, no_band.avg_turnover);
    }

    // NEWEY-WEST: pozitif otokorelasyon naif t'yi şişirir → HAC t onu deflate eder.
    #[test]
    fn newey_west_deflates_positively_autocorrelated() {
        // periyodiksiz beyaz gürültü (LCG) → IID; MA(3) düzleştirme lag-1,2 POZİTİF otokorelasyon yaratır.
        let mut seed = 12345u64;
        let mut nz = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 40) as f64 / (1u64 << 24) as f64 - 0.5 // ~[-0.5,0.5], ortalama~0, periyodiksiz
        };
        let raw: Vec<f64> = (0..600).map(|_| nz()).collect();
        // IID seri: NW ≈ lag0 (otokorelasyon ~0).
        let iid: Vec<f64> = raw.iter().map(|x| 0.01 + 0.03 * x).collect();
        let i0 = newey_west_tstat(&iid, 0);
        let i8 = newey_west_tstat(&iid, 8);
        assert!(i0 > 0.0, "mean>0 → pozitif t");
        assert!((i8 - i0).abs() / i0 < 0.30, "IID'de NW ≈ naif (otokorelasyon yok): {} vs {}", i8, i0);
        // MA(3) düzleştirme → pozitif kalıcılık → NW t naif'ten KÜÇÜK.
        let ma: Vec<f64> = (2..raw.len()).map(|t| 0.01 + 0.03 * (raw[t] + raw[t - 1] + raw[t - 2]) / 3.0).collect();
        let m0 = newey_west_tstat(&ma, 0);
        let m8 = newey_west_tstat(&ma, 8);
        assert!(m0 > 0.0 && m8 > 0.0, "mean>0 → pozitif t");
        assert!(m8 < m0 * 0.95, "pozitif otokorelasyon (MA) → NW t naif t'den KÜÇÜK ({} < {})", m8, m0);
    }

    // KALDIRAÇ: getiriyi/büyümeyi ölçekler ama t-stat'ı (anlamlılığı) DEĞİŞTİRMEZ.
    #[test]
    fn leverage_scales_growth_not_tstat() {
        // VOLATİL trend (std>0 → t-stat sonlu): A yukarı-eğilimli ama bar-bar dalgalı, C tersi.
        let mut a = 100.0; let mut c = 100.0; let mut closes = Vec::new();
        for i in 0..40 {
            closes.push(vec![Some(a), Some(100.0), Some(c)]);
            let up = if i % 2 == 0 { 1.05 } else { 1.01 }; // ort. yukarı, dalgalı → varyans gerçek
            a *= up; c *= 2.0 - up;
        }
        let base = XsConfig {
            symbols: vec!["A".into(), "B".into(), "C".into()],
            top_k: 1, lookback: 2, fee_rate: 0.0, wf_window: 5, ..Default::default()
        };
        let r1 = evaluate_xs(&closes, &base);
        let r3 = evaluate_xs(&closes, &XsConfig { leverage: 3.0, ..base.clone() });
        assert!(r1.std_ret > 0.0, "test verisi volatil (std>0) olmalı");
        let rel = (r1.t_stat - r3.t_stat).abs() / r1.t_stat.abs().max(1.0);
        assert!(rel < 1e-9, "kaldıraç t-stat'ı DEĞİŞTİRMEZ (anlamlılık L-invariant): {} vs {}", r1.t_stat, r3.t_stat);
        assert!((r3.mean_ret - 3.0 * r1.mean_ret).abs() < 1e-9, "ortalama getiri tam L× ölçeklenir");
        assert!(r3.total_return > r1.total_return, "bileşik büyüme L ile artar (pozitif edge'de)");
    }

    // CANLI hedef-kitap: sembol-anahtarlı sarmalayıcı backtest seçimiyle aynı sonucu verir.
    #[test]
    fn xs_target_book_symbol_keyed() {
        let sig = vec![
            ("A".to_string(), 0.5), ("B".into(), 0.3), ("C".into(), 0.1),
            ("D".into(), -0.2), ("E".into(), -0.4),
        ];
        let empty = HashSet::new();
        // momentum top_k=2: long en güçlü {A,B}, short en zayıf {E,D}.
        let (l, s) = xs_target_book(&sig, 2, 0, true, &empty, &empty);
        assert!(l.contains(&"A".to_string()) && l.contains(&"B".to_string()), "long = en güçlü 2");
        assert!(s.contains(&"E".to_string()) && s.contains(&"D".to_string()), "short = en zayıf 2");
        // reversal: long/short yer değiştirir.
        let (lr, sr) = xs_target_book(&sig, 2, 0, false, &empty, &empty);
        assert!(lr.contains(&"E".to_string()) && sr.contains(&"A".to_string()), "reversal → ters kitap");
        // histerezis: C (rank2) mevcut long ve buffer=1 → top-2 dışında olsa da TUTULUR.
        let held: HashSet<String> = ["C".to_string()].into_iter().collect();
        let (lh, _) = xs_target_book(&sig, 2, 1, true, &held, &empty);
        assert!(lh.contains(&"C".to_string()), "band: incumbent C (rank2<k+buffer=3) tutulur");
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
        let r = evaluate_xs_walkforward(&closes, None, &base, &wf);
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

    // ─── S/R EĞİMİ ───
    fn zone(zt: ZoneType, low: f64, high: f64, strength: f64) -> SrZone {
        SrZone {
            price_low: low, price_high: high, midpoint: (low + high) / 2.0,
            zone_type: zt, strength, touch_count: 3, vol_weight: 1.0,
        }
    }
    fn cdl(close: f64) -> Candle {
        Candle {
            timestamp: chrono::Utc::now(), open: close, high: close, low: close,
            close, volume: 1.0, symbol: "X".into(), interval: "1d".into(),
        }
    }

    // sr_opposition: yön + band + içeride + yanlış-taraf + güç-eşiği.
    #[test]
    fn sr_opposition_directional_and_banded() {
        let band = 3.0;
        // long-eğilim: ÜSTteki direnç karşıt. [101,102] → dist=1% → 1−1/3≈0.667.
        let near = vec![zone(ZoneType::Resistance, 101.0, 102.0, 5.0)];
        assert!((sr_opposition(&near, 100.0, true, band, 0.0) - (1.0 - 1.0 / 3.0)).abs() < 1e-6);
        // band dışı direnç (10%) → 0.
        let far = vec![zone(ZoneType::Resistance, 110.0, 111.0, 5.0)];
        assert_eq!(sr_opposition(&far, 100.0, true, band, 0.0), 0.0, "uzak direnç karşıtlık yok");
        // fiyat direnç İÇİNDE → 1.0 (en güçlü engel).
        let inside = vec![zone(ZoneType::Resistance, 99.5, 100.5, 5.0)];
        assert_eq!(sr_opposition(&inside, 100.0, true, band, 0.0), 1.0, "zone içinde tam karşıtlık");
        // long-eğilime ALTtaki destek karşıt DEĞİL.
        let sup = vec![zone(ZoneType::Support, 98.0, 99.0, 5.0)];
        assert_eq!(sr_opposition(&sup, 100.0, true, band, 0.0), 0.0, "long'a destek karşıt değil");
        // short-eğilim: ALTtaki destek karşıt. [98,99] → dist=1% → ≈0.667.
        assert!((sr_opposition(&sup, 100.0, false, band, 0.0) - (1.0 - 1.0 / 3.0)).abs() < 1e-6);
        // KIRILMIŞ direnç (fiyatın altında) long'a karşıt değil.
        let broken = vec![zone(ZoneType::Resistance, 98.0, 99.0, 5.0)];
        assert_eq!(sr_opposition(&broken, 100.0, true, band, 0.0), 0.0, "altta kalan direnç karşıt değil");
        // güç-eşiği: zayıf zone elenir.
        assert_eq!(sr_opposition(&near, 100.0, true, band, 9.0), 0.0, "min_strength altı zone sayılmaz");
    }

    // collect_sym_window: None'ları atlar, son `window` barı toplar, look-ahead'siz (>t dahil değil).
    #[test]
    fn collect_window_skips_gaps_and_respects_horizon() {
        // 5 bar × 2 sembol; sembol 0'da bar 2 None.
        let ohlc: Vec<Vec<Option<Candle>>> = vec![
            vec![Some(cdl(10.0)), Some(cdl(20.0))],
            vec![Some(cdl(11.0)), None],
            vec![None,            Some(cdl(22.0))],
            vec![Some(cdl(13.0)), Some(cdl(23.0))],
            vec![Some(cdl(14.0)), Some(cdl(24.0))],
        ];
        // t=3, window=3 → barlar 1..=3, sembol 0: bar2 None → [11.0, 13.0].
        let w = collect_sym_window(&ohlc, 3, 0, 3);
        assert_eq!(w.iter().map(|c| c.close).collect::<Vec<_>>(), vec![11.0, 13.0]);
        // t=2 → bar 4'ü (gelecek) ASLA içermez (look-ahead yok).
        let w2 = collect_sym_window(&ohlc, 2, 1, 10);
        assert!(w2.iter().all(|c| c.close <= 22.0), "gelecek bar sızmamalı");
    }

    // SIFIR REGRESYON: ohlc=None VEYA sr_tilt=0 → evaluate_xs ile BİREBİR (S/R eklentisi eski yolu bozmaz).
    #[test]
    fn sr_tilt_zero_is_bit_identical() {
        let mut a = 100.0;
        let mut c = 100.0;
        let mut closes = Vec::new();
        for _ in 0..60 {
            closes.push(vec![Some(a), Some(100.0), Some(c)]);
            a *= 1.02;
            c *= 0.98;
        }
        let cfg = XsConfig {
            symbols: vec!["A".into(), "B".into(), "C".into()],
            top_k: 1, lookback: 5, fee_rate: 0.001, ..Default::default()
        };
        let base = evaluate_xs(&closes, &cfg);
        // ohlc=None → birebir.
        let none = evaluate_xs_with_ohlc(&closes, None, &cfg);
        assert_eq!(base.bars, none.bars);
        assert_eq!(base.total_return.to_bits(), none.total_return.to_bits(), "None → birebir");
        // Some(ohlc) ama sr_tilt=0 → eğim bloğu atlanır → yine birebir (ohlc içeriği önemsiz).
        let dummy: Vec<Vec<Option<Candle>>> = vec![vec![None, None, None]; closes.len()];
        let tilt0 = evaluate_xs_with_ohlc(&closes, Some(&dummy), &XsConfig { sr_tilt: 0.0, ..cfg.clone() });
        assert_eq!(base.total_return.to_bits(), tilt0.total_return.to_bits(), "sr_tilt=0 → birebir");
        assert_eq!(base.t_stat.to_bits(), tilt0.t_stat.to_bits());
    }

    // ───────── XsSignal eksen genelleştirmesi ─────────

    /// closes matrisi: her sembol kendi fiyat serisi (hepsi Some).
    fn closes_of(series: &[Vec<f64>]) -> Vec<Vec<Option<f64>>> {
        let n = series[0].len();
        (0..n).map(|t| series.iter().map(|s| Some(s[t])).collect()).collect()
    }

    #[test]
    fn lowvol_signal_ranks_calm_symbol_higher() {
        // A: sakin (küçük salınım), B: çalkantılı (büyük salınım). LowVol → A daha yüksek skor (long).
        let a: Vec<f64> = (0..40).map(|i| 100.0 + 0.2 * (i as f64).sin()).collect();
        let b: Vec<f64> = (0..40).map(|i| 100.0 + 5.0 * (i as f64).sin()).collect();
        let m = closes_of(&[a, b]);
        let cfg = XsConfig { signal: XsSignal::LowVol, lookback: 20, ..Default::default() };
        let sig = build_signals(&m, 39, &cfg);
        let sa = sig.iter().find(|(j, _)| *j == 0).unwrap().1;
        let sb = sig.iter().find(|(j, _)| *j == 1).unwrap().1;
        assert!(sa > sb, "sakin sembol (A) daha yüksek skor almalı: A={sa} B={sb}");
    }

    #[test]
    fn lottery_signal_penalizes_spike() {
        // A: düz; B: tek büyük sıçrama (piyango). MaxLottery → B daha DÜŞÜK skor (short edilir).
        let a: Vec<f64> = (0..30).map(|i| 100.0 + 0.1 * i as f64).collect();
        let mut b = a.clone();
        b[20] *= 1.30; // %30 spike
        let m = closes_of(&[a, b]);
        let cfg = XsConfig { signal: XsSignal::MaxLottery, lookback: 15, ..Default::default() };
        let sig = build_signals(&m, 25, &cfg);
        let sa = sig.iter().find(|(j, _)| *j == 0).unwrap().1;
        let sb = sig.iter().find(|(j, _)| *j == 1).unwrap().1;
        assert!(sa > sb, "piyango sembolü (B) daha düşük skor almalı: A={sa} B={sb}");
    }

    #[test]
    fn beta_signal_ranks_lowbeta_higher() {
        // Market = ortak faktör. A düşük-β (faktörün 0.3'ü), B yüksek-β (1.5'i) + ortak gürültü.
        // −β long → A (düşük β) daha yüksek skor.
        let mut a = Vec::new();
        let mut b = Vec::new();
        let (mut pa, mut pb) = (100.0_f64, 100.0_f64);
        for i in 0..60 {
            let f = ((i as f64) * 0.5).sin() * 0.02; // ortak faktör getirisi
            pa *= 1.0 + 0.3 * f;
            pb *= 1.0 + 1.5 * f;
            a.push(pa);
            b.push(pb);
        }
        let m = closes_of(&[a, b]);
        let cfg = XsConfig { signal: XsSignal::Beta, lookback: 30, ..Default::default() };
        let sig = build_signals(&m, 59, &cfg);
        let sa = sig.iter().find(|(j, _)| *j == 0).unwrap().1;
        let sb = sig.iter().find(|(j, _)| *j == 1).unwrap().1;
        assert!(sa > sb, "düşük-β sembol (A) daha yüksek skor almalı: A={sa} B={sb}");
    }

    #[test]
    fn momentum_axis_unchanged_by_generalization() {
        // build_signals(Momentum) eski inline döngüyle BİREBİR olmalı (regresyon güvencesi).
        let a: Vec<f64> = (0..30).map(|i| 100.0 * 1.01_f64.powi(i)).collect(); // güçlü yukarı
        let b: Vec<f64> = (0..30).map(|i| 100.0 * 0.99_f64.powi(i)).collect(); // aşağı
        let m = closes_of(&[a, b]);
        let cfg = XsConfig { signal: XsSignal::Momentum, lookback: 10, ..Default::default() };
        let sig = build_signals(&m, 29, &cfg);
        let sa = sig.iter().find(|(j, _)| *j == 0).unwrap().1;
        let sb = sig.iter().find(|(j, _)| *j == 1).unwrap().1;
        assert!(sa > 0.0 && sb < 0.0 && sa > sb, "momentum: yukarı + / aşağı −");
    }
}
