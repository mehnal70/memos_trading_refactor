// robot/screener/score.rs — Saf skor + selection delta yardımcıları.
//
// Skor: kısa bir backtest (active strategy, varsayılan TP/SL/PS) çalıştırır
// ve composite skor üretir; ek olarak likitite proxy (ortalama volume) ve
// volatilite proxy (ATR%) tutulur. Skor formülü intentionally simple
// (Sharpe ağırlıklı + WR + DD penalty); ileride ML-tabanlı skorlamayla
// değiştirilebilir.
//
// Selection: mevcut orchestrator worker listesi + pinned semboller + skorlu
// aday listesi → eklenecekler ve düşürülecekler. Pinned semboller hiçbir
// koşulda düşürülmez. Max worker kapasitesi kullanıcı tarafına bırakılmaz
// (orchestrator.max_workers caller'da uygulanır).

use crate::core::indicators::CoreIndicatorEngine;
use crate::core::types::Candle;
use crate::robot::backtester::{Backtester, BacktestConfig};

/// Screener HTF trend tanımı — sinyal yolundaki `htf_trend_filter` ile aynı
/// SMA(10)/SMA(30). Tek kaynak: değişirse seçim ve sinyal yolları birlikte
/// gözden geçirilmeli (strategies/utils.rs htf_trend_filter çağrıları 10/30).
pub const HTF_BIAS_FAST: usize = 10;
pub const HTF_BIAS_SLOW: usize = 30;

/// Üst zaman dilimi (HTF) trend hizası — screener sıralamasında sembol/strateji
/// seçimine üst-TF yönünü katar. Sistem long-bias olduğundan boğa hizası lehte,
/// ayı hizası aleyhte sayılır.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HtfBias {
    Bullish, // HTF fast MA > slow MA
    Bearish, // HTF fast MA < slow MA
    Neutral, // veri yetersiz / MA eşit → seçimi etkilemez
}

/// HTF mumlarından SMA(fast)/SMA(slow) ile trend yönü.
/// `htf_trend_filter` ile birebir aynı kaynak (`CoreIndicatorEngine::sma`)
/// → sembol SEÇİMİ ile sinyal ÜRETİMİ aynı HTF görüşünü paylaşır.
/// `htf` None veya `slow`'dan az mum → `Neutral` (etkisiz).
pub fn htf_bias(htf: Option<&[Candle]>, fast: usize, slow: usize) -> HtfBias {
    let h = match htf {
        Some(h) if h.len() >= slow => h,
        _ => return HtfBias::Neutral,
    };
    let fast_ma = CoreIndicatorEngine::sma(h, fast);
    let slow_ma = CoreIndicatorEngine::sma(h, slow);
    if fast_ma == 0.0 || slow_ma == 0.0 {
        return HtfBias::Neutral;
    }
    if fast_ma > slow_ma {
        HtfBias::Bullish
    } else if fast_ma < slow_ma {
        HtfBias::Bearish
    } else {
        HtfBias::Neutral
    }
}

/// HTF hizasının composite skora **additif** katkısı (boğa → +delta,
/// ayı → −delta, nötr → 0). Additif çünkü composite negatif olabilir
/// (negatif sharpe); çarpım işareti ters çevirir, sıralamayı bozardı.
/// `delta == 0.0` → HTF tamamen devre dışı (legacy davranış).
pub fn htf_bias_adjustment(bias: HtfBias, delta: f64) -> f64 {
    match bias {
        HtfBias::Bullish => delta,
        HtfBias::Bearish => -delta,
        HtfBias::Neutral => 0.0,
    }
}

/// Tek bir sembolün screener çıktısı. Composite skora göre sıralanır;
/// likitite/volatilite alanları diagnostik amaçlı tutulur.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenerScore {
    pub avg_volume:  f64, // son N mumun ortalama volume'u
    pub atr_pct:     f64, // ATR% (volatilite proxy)
    pub sharpe:      f64, // backtest sharpe (skor bileşeni)
    pub win_rate:    f64, // backtest win rate (%)
    pub max_dd_pct:  f64, // backtest max drawdown (%)
    pub trades:      usize,
    pub htf_bias:    HtfBias, // seçim anındaki üst-TF hizası (telemetri + sıralama izi)
    pub composite:   f64, // sıralama anahtarı (HTF-ayarlı); yüksek = iyi
}

impl ScreenerScore {
    /// Hiç işlem üretemediyse veya backtest başarısızsa skor sıfır
    /// (sıralamada hep alta düşer ama negatif değer üretmez).
    /// HTF hizası yalnız gerçek skorlu sembollere uygulanır → empty Neutral kalır
    /// (0 işlemli bir sembol HTF boğa diye seçime itilmemeli).
    pub fn empty(avg_volume: f64, atr_pct: f64) -> Self {
        Self {
            avg_volume, atr_pct,
            sharpe: 0.0, win_rate: 0.0, max_dd_pct: 0.0,
            trades: 0, htf_bias: HtfBias::Neutral, composite: 0.0,
        }
    }
}

/// Mum dizisinden bir stratejinin hızlı backtest skorunu çıkartır.
/// `tp_pct`/`sl_pct`/`ps` skor karşılaştırması için sabit varsayılanlar
/// kullanır (her aday aynı parametrelerle test edilir → adil sıralama).
/// Yetersiz veri (< 50 mum) veya yetersiz işlem (< 3) → `ScreenerScore::empty`.
///
/// `htf` verilirse (üst zaman dilimi mumları) ve `htf_bias_delta > 0` ise
/// composite skoruna HTF trend hizası additif katılır → seçim üst-TF'yi görür.
/// `htf=None` veya `delta=0.0` → legacy tek-TF davranış (etkisiz).
pub fn score_symbol(
    candles: &[Candle],
    strategy_name: &str,
    tp_pct: f64,
    sl_pct: f64,
    ps: f64,
    initial_balance: f64,
    htf: Option<&[Candle]>,
    htf_bias_delta: f64,
) -> ScreenerScore {
    let avg_volume = if candles.is_empty() {
        0.0
    } else {
        candles.iter().map(|c| c.volume).sum::<f64>() / candles.len() as f64
    };
    let atr_pct = compute_atr_pct(candles);

    if candles.len() < 50 {
        return ScreenerScore::empty(avg_volume, atr_pct);
    }

    let cfg = BacktestConfig {
        symbol: "SCREENER".into(),
        interval: "1h".into(),
        initial_balance,
        max_position_size: ps,
        take_profit_pct: tp_pct,
        stop_loss_pct: sl_pct,
        strategy_name: strategy_name.to_string(),
        commission_pct: 0.001,
        ..Default::default()
    };
    let res = match Backtester::new(cfg).run(candles) {
        Ok(r) => r,
        Err(_) => return ScreenerScore::empty(avg_volume, atr_pct),
    };
    if res.total_trades < 3 {
        return ScreenerScore::empty(avg_volume, atr_pct);
    }

    let base = composite_score(res.sharpe_ratio, res.win_rate, res.max_drawdown_pct);
    // HTF hizası yalnız gerçek skorlu (≥3 işlem) sembollere uygulanır.
    let bias = htf_bias(htf, HTF_BIAS_FAST, HTF_BIAS_SLOW);
    let composite = base + htf_bias_adjustment(bias, htf_bias_delta);
    ScreenerScore {
        avg_volume, atr_pct,
        sharpe: res.sharpe_ratio,
        win_rate: res.win_rate,
        max_dd_pct: res.max_drawdown_pct,
        trades: res.total_trades,
        htf_bias: bias,
        composite,
    }
}

/// Composite skor: Sharpe 50% + WinRate 30% − Drawdown 20%.
/// WinRate ve drawdown 0..100 ölçeğinde geldiği için normalize edilir.
fn composite_score(sharpe: f64, win_rate_pct: f64, dd_pct: f64) -> f64 {
    let wr = (win_rate_pct / 100.0).clamp(0.0, 1.0);
    let dd = (dd_pct / 100.0).clamp(0.0, 1.0);
    sharpe * 0.50 + wr * 0.30 - dd * 0.20
}

fn compute_atr_pct(candles: &[Candle]) -> f64 {
    let n = candles.len();
    if n < 15 { return 0.0; }
    let last = &candles[n.saturating_sub(14)..];
    let mut sum_tr = 0.0;
    for w in last.windows(2) {
        let (prev, cur) = (&w[0], &w[1]);
        let h_l = cur.high - cur.low;
        let h_pc = (cur.high - prev.close).abs();
        let l_pc = (cur.low  - prev.close).abs();
        sum_tr += h_l.max(h_pc).max(l_pc);
    }
    let atr = sum_tr / (last.len() - 1).max(1) as f64;
    let last_close = candles.last().map(|c| c.close).unwrap_or(0.0);
    if last_close <= 0.0 { 0.0 } else { atr / last_close * 100.0 }
}

// ─────────────────────────────────────────────────────────────────────────────
// Selection delta
// ─────────────────────────────────────────────────────────────────────────────

/// Bir screener turunun orchestrator üzerinde uygulanacak değişikliği:
/// `to_add` register edilecek, `to_remove` stop_symbol ile düşürülecek.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionDiff {
    pub to_add:    Vec<String>,
    pub to_remove: Vec<String>,
    pub selected:  Vec<String>, // Diff sonrası nihai liste (telemetri için).
}

/// Skorlu aday listesinden orchestrator'a uygulanacak delta hesapla.
/// Kurallar:
///   - `pinned` her zaman seçilenler arasında kalır (en başa konur).
///   - Geriye kalan kapasiteye composite skor sırasıyla aday eklenir.
///   - `max_workers` mutlak üst sınır; pinned dahi olsa aşılmaz (pinned önceliklidir).
///   - `current_workers`'tan pinned olmayan + selected olmayan her sembol `to_remove`.
///
/// `scored` zaten composite skora göre büyükten küçüğe sıralı olmalı.
pub fn select_top_n_diff(
    current_workers: &[String],
    pinned: &[String],
    scored: &[(String, ScreenerScore)],
    top_n: usize,
    max_workers: usize,
) -> SelectionDiff {
    let mut selected: Vec<String> = Vec::new();
    let cap = top_n.min(max_workers);

    // 1) Pinned'i öne koy (max_workers'ı aşmadan).
    for p in pinned {
        if selected.len() >= cap { break; }
        if !selected.iter().any(|s| s == p) {
            selected.push(p.clone());
        }
    }

    // 2) Skorlu adaylardan kalan slotları doldur (pinned ile dup atla).
    for (name, _) in scored {
        if selected.len() >= cap { break; }
        if !selected.iter().any(|s| s == name) {
            selected.push(name.clone());
        }
    }

    let to_add: Vec<String> = selected.iter()
        .filter(|s| !current_workers.iter().any(|c| c == *s))
        .cloned()
        .collect();
    let pinned_set: std::collections::HashSet<&String> = pinned.iter().collect();
    let to_remove: Vec<String> = current_workers.iter()
        .filter(|c| !selected.iter().any(|s| s == *c) && !pinned_set.contains(*c))
        .cloned()
        .collect();

    SelectionDiff { to_add, to_remove, selected }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cs(closes: &[f64], vol: f64) -> Vec<Candle> {
        closes.iter().map(|&c| Candle {
            open: c, high: c + 0.5, low: c - 0.5, close: c, volume: vol,
            ..Default::default()
        }).collect()
    }

    // ── score_symbol ────────────────────────────────────────────────────

    #[test]
    fn score_empty_when_candles_too_few() {
        let c = cs(&[100.0; 10], 50.0);
        let s = score_symbol(&c, "MA_CROSSOVER", 4.0, 2.0, 0.3, 10_000.0, None, 0.0);
        assert_eq!(s.trades, 0);
        assert_eq!(s.composite, 0.0);
        assert_eq!(s.avg_volume, 50.0);
        assert_eq!(s.htf_bias, HtfBias::Neutral);
    }

    #[test]
    fn score_records_volume_and_atr_even_with_no_trades() {
        let c = cs(&[100.0; 100], 200.0);
        let s = score_symbol(&c, "MA_CROSSOVER", 4.0, 2.0, 0.3, 10_000.0, None, 0.0);
        assert_eq!(s.avg_volume, 200.0);
        assert!(s.atr_pct >= 0.0);
    }

    #[test]
    fn score_is_deterministic_on_same_input() {
        let c: Vec<Candle> = (0..200).map(|i| Candle {
            open: 100.0 + (i as f64) * 0.5,
            high: 100.5 + (i as f64) * 0.5,
            low:  99.5  + (i as f64) * 0.5,
            close: 100.0 + (i as f64) * 0.5,
            volume: 100.0,
            ..Default::default()
        }).collect();
        let a = score_symbol(&c, "MA_CROSSOVER", 4.0, 2.0, 0.3, 10_000.0, None, 0.0);
        let b = score_symbol(&c, "MA_CROSSOVER", 4.0, 2.0, 0.3, 10_000.0, None, 0.0);
        assert_eq!(a, b);
    }

    // ── HTF bias ────────────────────────────────────────────────────────

    /// Yükselen seri → SMA(10) > SMA(30) → Bullish; düşen seri → Bearish.
    #[test]
    fn htf_bias_detects_trend_direction() {
        let up: Vec<Candle> = cs(&(0..60).map(|i| 100.0 + i as f64).collect::<Vec<_>>(), 1.0);
        let down: Vec<Candle> = cs(&(0..60).map(|i| 160.0 - i as f64).collect::<Vec<_>>(), 1.0);
        assert_eq!(htf_bias(Some(&up), HTF_BIAS_FAST, HTF_BIAS_SLOW), HtfBias::Bullish);
        assert_eq!(htf_bias(Some(&down), HTF_BIAS_FAST, HTF_BIAS_SLOW), HtfBias::Bearish);
    }

    #[test]
    fn htf_bias_neutral_when_insufficient_or_none() {
        let few = cs(&[100.0; 10], 1.0); // slow=30'dan az
        assert_eq!(htf_bias(Some(&few), HTF_BIAS_FAST, HTF_BIAS_SLOW), HtfBias::Neutral);
        assert_eq!(htf_bias(None, HTF_BIAS_FAST, HTF_BIAS_SLOW), HtfBias::Neutral);
    }

    #[test]
    fn htf_bias_adjustment_signs() {
        assert_eq!(htf_bias_adjustment(HtfBias::Bullish, 0.3), 0.3);
        assert_eq!(htf_bias_adjustment(HtfBias::Bearish, 0.3), -0.3);
        assert_eq!(htf_bias_adjustment(HtfBias::Neutral, 0.3), 0.0);
        // delta=0 → her hizada etkisiz.
        assert_eq!(htf_bias_adjustment(HtfBias::Bullish, 0.0), 0.0);
    }

    /// Skorlu (≥3 işlem) bir sembolde boğa HTF composite'i tam +delta kaydırır,
    /// ayı −delta; delta=0 → değişmez. Zig-zag seri MA_CROSSOVER'da işlem üretir.
    #[test]
    fn score_applies_htf_bias_to_real_trades() {
        // 200 mumluk testere dişi (period ~20) → MA kesişimleri → işlemler.
        let closes: Vec<f64> = (0..200)
            .map(|i| 100.0 + 8.0 * ((i as f64) * std::f64::consts::PI / 10.0).sin())
            .collect();
        let c = cs(&closes, 100.0);
        let up: Vec<Candle> = cs(&(0..60).map(|i| 100.0 + i as f64).collect::<Vec<_>>(), 1.0);
        let down: Vec<Candle> = cs(&(0..60).map(|i| 160.0 - i as f64).collect::<Vec<_>>(), 1.0);

        let base = score_symbol(&c, "MA_CROSSOVER", 4.0, 2.0, 0.3, 10_000.0, None, 0.0);
        assert!(base.trades >= 3, "zig-zag yeterli işlem üretmeli (n={})", base.trades);

        let bull = score_symbol(&c, "MA_CROSSOVER", 4.0, 2.0, 0.3, 10_000.0, Some(&up), 0.3);
        let bear = score_symbol(&c, "MA_CROSSOVER", 4.0, 2.0, 0.3, 10_000.0, Some(&down), 0.3);
        assert_eq!(bull.htf_bias, HtfBias::Bullish);
        assert_eq!(bear.htf_bias, HtfBias::Bearish);
        assert!((bull.composite - (base.composite + 0.3)).abs() < 1e-9);
        assert!((bear.composite - (base.composite - 0.3)).abs() < 1e-9);
        assert!(bull.composite > bear.composite);

        // delta=0 → HTF verilse bile composite değişmez.
        let zero = score_symbol(&c, "MA_CROSSOVER", 4.0, 2.0, 0.3, 10_000.0, Some(&up), 0.0);
        assert!((zero.composite - base.composite).abs() < 1e-9);
    }

    // ── composite_score ────────────────────────────────────────────────

    #[test]
    fn composite_penalizes_drawdown() {
        let a = composite_score(1.0, 60.0, 10.0);
        let b = composite_score(1.0, 60.0, 40.0); // daha derin DD
        assert!(a > b, "büyük DD daha düşük composite vermeli: a={a} b={b}");
    }

    #[test]
    fn composite_rewards_higher_sharpe() {
        let a = composite_score(2.0, 55.0, 10.0);
        let b = composite_score(0.5, 55.0, 10.0);
        assert!(a > b);
    }

    // ── select_top_n_diff ──────────────────────────────────────────────

    fn scored(names: &[&str]) -> Vec<(String, ScreenerScore)> {
        names.iter().enumerate().map(|(i, n)| {
            let mut s = ScreenerScore::empty(0.0, 0.0);
            s.composite = (names.len() - i) as f64;
            (n.to_string(), s)
        }).collect()
    }

    #[test]
    fn diff_adds_new_keeps_pinned_removes_unselected() {
        let current = vec!["BTCUSDT".into(), "ETHUSDT".into(), "OLDCOIN".into()];
        let pinned  = vec!["BTCUSDT".into()];
        let s = scored(&["ETHUSDT", "SOLUSDT", "AVAXUSDT"]);
        let d = select_top_n_diff(&current, &pinned, &s, 3, 16);
        assert_eq!(d.selected, vec!["BTCUSDT".to_string(), "ETHUSDT".into(), "SOLUSDT".into()]);
        assert_eq!(d.to_add, vec!["SOLUSDT".to_string()]);
        assert_eq!(d.to_remove, vec!["OLDCOIN".to_string()]);
    }

    #[test]
    fn diff_never_removes_pinned_even_if_score_low() {
        let current = vec!["BTCUSDT".into()];
        let pinned  = vec!["BTCUSDT".into()];
        // Hiç skorlu aday yok — pinned tek başına kalmalı, kimse to_remove'a girmemeli.
        let d = select_top_n_diff(&current, &pinned, &[], 5, 16);
        assert_eq!(d.selected, vec!["BTCUSDT".to_string()]);
        assert!(d.to_add.is_empty());
        assert!(d.to_remove.is_empty());
    }

    #[test]
    fn diff_caps_at_max_workers_even_with_many_candidates() {
        let pinned = vec!["BTC".into()];
        let s = scored(&["ETH", "SOL", "AVAX", "BNB", "ADA", "DOT"]);
        let d = select_top_n_diff(&[], &pinned, &s, 10, 4);
        // max_workers=4: pinned BTC + en yüksek skorlu 3 → 4 toplam
        assert_eq!(d.selected.len(), 4);
        assert_eq!(d.selected[0], "BTC");
        assert_eq!(&d.selected[1..], &["ETH", "SOL", "AVAX"]);
    }

    #[test]
    fn diff_deduplicates_pinned_appearing_in_scored() {
        let pinned = vec!["BTC".into()];
        let s = scored(&["BTC", "ETH", "SOL"]); // BTC skorlu listede de var
        let d = select_top_n_diff(&[], &pinned, &s, 3, 16);
        // BTC bir kere
        assert_eq!(d.selected.iter().filter(|s| *s == "BTC").count(), 1);
        assert_eq!(d.selected, vec!["BTC".to_string(), "ETH".into(), "SOL".into()]);
    }

    #[test]
    fn diff_empty_when_already_aligned() {
        let current = vec!["BTC".into(), "ETH".into()];
        let pinned = vec!["BTC".into()];
        let s = scored(&["ETH"]);
        let d = select_top_n_diff(&current, &pinned, &s, 2, 16);
        assert!(d.to_add.is_empty());
        assert!(d.to_remove.is_empty());
    }
}
