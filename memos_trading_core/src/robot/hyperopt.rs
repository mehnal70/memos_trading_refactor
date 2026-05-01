// robot/hyperopt.rs
//
// Hiperparametre Optimizasyonu
//
// Strateji parametrelerini (RSI period, MA penceresi, BB band vb.) backtest
// üzerinden optimize eder. Üç arama yöntemi:
//
//   1. Grid Search:   Parametre ızgarasını tam tarar. Küçük arama uzayı için.
//   2. Random Search: Rastgele örnekleme. Geniş arama uzayı + hız için.
//   3. Bayesian-like: Önceki sonuçlardan öğrenen ağırlıklı örnekleme.
//                     (Tam GP değil; basit UCB-tabanlı yaklaşım.)
//
// Değerlendirme fonksiyonu: Backtester ile çalışır.
// Skor = Sharpe×0.35 + ProfitFactor×0.25 + WinRate×0.25 - MaxDD×0.15
//
// Tüm sonuçlar kaydedilir → en iyi parametre seti döner.

use crate::types::{Candle, StrategyParams};
use crate::robot::backtester::{Backtester, BacktestConfig};
use serde::{Deserialize, Serialize};

// ─── Sonuç tipi ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HyperOptResult {
    pub best_params:      StrategyParams,
    pub best_score:       f64,
    pub best_win_rate:    f64,
    pub best_pnl_pct:     f64,
    pub best_sharpe:      f64,
    pub best_pf:          f64,
    pub best_max_dd:      f64,
    pub combinations_tested: usize,
    /// Tüm test edilen parametreler (skor azalan sırada ilk 20)
    pub top_results:      Vec<HyperOptEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HyperOptEntry {
    pub params: StrategyParams,
    pub score:  f64,
    pub win_rate: f64,
    pub pnl_pct:  f64,
    pub sharpe:   f64,
}

// ─── Kompozit skor ───────────────────────────────────────────────────────────

fn composite_score(sharpe: f64, pf: f64, win_rate: f64, max_dd: f64, n_trades: usize) -> f64 {
    if n_trades < 3 { return f64::NEG_INFINITY; } // çok az trade → güvenilmez
    // win_rate 0–100 → normalize et
    let wr_norm = win_rate / 100.0;
    // profit_factor 1.0 üzeri iyi; log scale ile ölçekle
    let pf_norm = if pf > 0.0 { (pf.ln() + 1.0).max(0.0) } else { 0.0 };
    // max_dd ceza (yüksek DD = kötü)
    let dd_penalty = (max_dd / 100.0).min(1.0);

    sharpe * 0.35 + pf_norm * 0.25 + wr_norm * 0.25 - dd_penalty * 0.15
}

// ─── HyperOpt ────────────────────────────────────────────────────────────────

pub struct HyperOpt;

impl HyperOpt {
    // ── Grid Search ──────────────────────────────────────────────────────────

    /// Strateji parametrelerini backtest üzerinden grid search ile optimize et.
    ///
    /// `param_grid`: test edilecek StrategyParams listesi (çağıran hazırlar).
    pub fn grid_search(
        candles:      &[Candle],
        param_grid:   &[StrategyParams],
        backtest_cfg: &BacktestConfig,
    ) -> Option<HyperOptResult> {
        if candles.is_empty() || param_grid.is_empty() { return None; }

        let mut entries: Vec<HyperOptEntry> = Vec::new();

        for params in param_grid {
            let mut cfg = backtest_cfg.clone();
            cfg.strategy_params = Some(params.clone());

            if let Ok(r) = Backtester::new(cfg).run(candles) {
                let score = composite_score(r.sharpe_ratio, r.profit_factor,
                                            r.win_rate, r.max_drawdown_pct, r.total_trades);
                entries.push(HyperOptEntry {
                    params: params.clone(),
                    score,
                    win_rate: r.win_rate,
                    pnl_pct:  r.total_pnl_pct,
                    sharpe:   r.sharpe_ratio,
                });
            }
        }

        Self::build_result(entries)
    }

    // ── Random Search ────────────────────────────────────────────────────────

    /// Parametre uzayını rastgele örnekle.
    ///
    /// RSI / MA / Bollinger parametrelerini otomatik aralıkta dener.
    /// `n_iter`: kaç kombinasyon deneneceği
    pub fn random_search(
        candles:      &[Candle],
        n_iter:       usize,
        backtest_cfg: &BacktestConfig,
        seed:         Option<u64>,
    ) -> Option<HyperOptResult> {
        if candles.is_empty() || n_iter == 0 { return None; }

        // LCG PRNG — harici bağımlılık gereksiz
        let mut state: u64 = seed.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(12345)
        });
        let next_u64 = |s: &mut u64| -> u64 {
            *s = s.wrapping_mul(6_364_136_223_846_793_005)
                   .wrapping_add(1_442_695_040_888_963_407);
            *s
        };
        let rand_range = |s: &mut u64, lo: u64, hi: u64| -> u64 {
            lo + next_u64(s) % (hi - lo + 1)
        };
        let rand_f64 = |s: &mut u64, lo: f64, hi: f64| -> f64 {
            lo + (next_u64(s) as f64 / u64::MAX as f64) * (hi - lo)
        };

        let mut entries: Vec<HyperOptEntry> = Vec::new();

        for _ in 0..n_iter {
            let fast   = rand_range(&mut state, 3, 15) as usize;
            let slow   = rand_range(&mut state, fast as u64 + 5, 60) as usize;
            let period = rand_range(&mut state, 7, 21) as usize;
            let ob     = rand_f64(&mut state, 65.0, 82.0);
            let os     = rand_f64(&mut state, 18.0, 35.0);
            let bb_per = rand_range(&mut state, 10, 30) as usize;

            let params = StrategyParams {
                fast:       Some(fast),
                slow:       Some(slow),
                period:     Some(period),
                overbought: Some(ob),
                oversold:   Some(os),
                bb_period:  Some(bb_per),
                ..Default::default()
            };

            let mut cfg = backtest_cfg.clone();
            cfg.strategy_params = Some(params.clone());

            if let Ok(r) = Backtester::new(cfg).run(candles) {
                let score = composite_score(r.sharpe_ratio, r.profit_factor,
                                            r.win_rate, r.max_drawdown_pct, r.total_trades);
                entries.push(HyperOptEntry {
                    params,
                    score,
                    win_rate: r.win_rate,
                    pnl_pct:  r.total_pnl_pct,
                    sharpe:   r.sharpe_ratio,
                });
            }
        }

        Self::build_result(entries)
    }

    // ── Bayesian-like (Exploitation/Exploration UCB) ─────────────────────────

    /// Önce rastgele keşif (n_explore), sonra en iyi bölgeden yoğunlaşma (n_exploit).
    /// Tam Gaussian Process değil; basit UCB yaklaşımı:
    ///   - İlk n_explore rastgele tarama
    ///   - Top-%20 parametre bölgesini daraltarak n_exploit iterasyon
    pub fn bayesian_search(
        candles:      &[Candle],
        n_explore:    usize,
        n_exploit:    usize,
        backtest_cfg: &BacktestConfig,
    ) -> Option<HyperOptResult> {
        if candles.is_empty() { return None; }

        // Adım 1: keşif aşaması
        let explore = Self::random_search(candles, n_explore, backtest_cfg, Some(42))?;

        // Adım 2: en iyi parametrelerin etrafında yoğunlaş
        let best = &explore.best_params;
        let mut exploit_grid: Vec<StrategyParams> = Vec::with_capacity(n_exploit);

        // LCG ile küçük pertürbasyon
        let mut state: u64 = 99991;
        let next_u64 = |s: &mut u64| -> u64 {
            *s = s.wrapping_mul(6_364_136_223_846_793_005)
                   .wrapping_add(1_442_695_040_888_963_407);
            *s
        };

        for _ in 0..n_exploit {
            let delta_fast   = (next_u64(&mut state) % 3) as i64 - 1; // -1,0,+1
            let delta_slow   = (next_u64(&mut state) % 5) as i64 - 2; // -2..+2
            let delta_period = (next_u64(&mut state) % 3) as i64 - 1;

            let fast   = ((best.fast.unwrap_or(10) as i64 + delta_fast).max(3)) as usize;
            let slow   = ((best.slow.unwrap_or(30) as i64 + delta_slow).max(fast as i64 + 1)) as usize;
            let period = ((best.period.unwrap_or(14) as i64 + delta_period).max(5)) as usize;

            exploit_grid.push(StrategyParams {
                fast:   Some(fast),
                slow:   Some(slow),
                period: Some(period),
                overbought: best.overbought,
                oversold:   best.oversold,
                bb_period:  best.bb_period,
                ..Default::default()
            });
        }

        let exploit = Self::grid_search(candles, &exploit_grid, backtest_cfg)?;

        // Adım 3: keşif + yoğunlaşma sonuçlarını birleştir
        let mut all_entries = explore.top_results;
        all_entries.extend(exploit.top_results);
        all_entries.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        let n_total = explore.combinations_tested + exploit.combinations_tested;
        let mut result = Self::build_result(all_entries)?;
        result.combinations_tested = n_total;
        Some(result)
    }

    // ── Yardımcı ─────────────────────────────────────────────────────────────

    fn build_result(mut entries: Vec<HyperOptEntry>) -> Option<HyperOptResult> {
        if entries.is_empty() { return None; }

        entries.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // Önce ihtiyaç duyulan değerleri kopyala, sonra entries'i truncate et
        let n             = entries.len();
        let best_params   = entries[0].params.clone();
        let best_score    = entries[0].score;
        let best_win_rate = entries[0].win_rate;
        let best_pnl_pct  = entries[0].pnl_pct;
        let best_sharpe   = entries[0].sharpe;

        // En iyi 20 taneyi sakla
        entries.truncate(20);

        Some(HyperOptResult {
            best_params,
            best_score,
            best_win_rate,
            best_pnl_pct,
            best_sharpe,
            best_pf:     0.0,
            best_max_dd: 0.0,
            combinations_tested: n,
            top_results: entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_candles(n: usize) -> Vec<Candle> {
        let mut price = 100.0f64;
        (0..n).map(|i| {
            price += (i as f64 * 0.4) % 3.5 - 1.5;
            Candle {
                symbol:    "BTC".into(),
                interval:  "1h".into(),
                timestamp: Utc::now() + chrono::Duration::hours(i as i64),
                open:  price,
                high:  price + 2.0,
                low:   price - 1.0,
                close: price + 0.3,
                volume: 500.0 + i as f64 * 5.0,
            }
        }).collect()
    }

    fn base_cfg() -> BacktestConfig {
        BacktestConfig {
            symbol: "BTC".into(), interval: "1h".into(),
            initial_balance: 10_000.0, max_position_size: 1.0,
            take_profit_pct: 5.0, stop_loss_pct: 2.0,
            strategy_name: "RSI".into(),
            position_profile: None, security_profile: None,
            strategy_params: None, commission_pct: 0.001,
            breakeven_at_rr: None, atr_trail_mult: None, partial_tp_ratio: None,
        }
    }

    #[test]
    fn test_random_search_returns_result() {
        let candles = make_candles(150);
        let result  = HyperOpt::random_search(&candles, 10, &base_cfg(), Some(42));
        assert!(result.is_some());
        let r = result.unwrap();
        assert!(r.combinations_tested > 0);
        assert!(r.best_params.fast.is_some());
    }

    #[test]
    fn test_grid_search() {
        let candles = make_candles(150);
        let grid = vec![
            StrategyParams { fast: Some(5), slow: Some(20), period: Some(14), ..Default::default() },
            StrategyParams { fast: Some(8), slow: Some(30), period: Some(14), ..Default::default() },
        ];
        let result = HyperOpt::grid_search(&candles, &grid, &base_cfg());
        assert!(result.is_some());
    }

    #[test]
    fn test_composite_score_penalizes_low_trades() {
        let score = composite_score(1.0, 2.0, 55.0, 10.0, 2);
        assert_eq!(score, f64::NEG_INFINITY);
    }
}
