use crate::types::Candle;
use crate::robot::backtester::{Backtester, BacktestConfig, BacktestResult};
use crate::Result;
use crate::MemosTradingError;
use serde::{Deserialize, Serialize};

// ─── Çok-metrik skor ─────────────────────────────────────────────────────────
//
// Sadece total_pnl_pct maksimize etmek curve-fitting'e yol açar.
// Kompozit skor birden fazla boyutu dengeler:
//
//   Sharpe     35% — risk-adjusted return
//   WinRate    25% — tutarlılık
//   ProfitFact 25% — kazanç/kayıp oranı
//   MaxDD       15% — ceza (büyük drawdown = kötü)
//
// Yeterli trade yoksa skor NEG_INFINITY → bu kombinasyon elenir.

fn composite_score(r: &BacktestResult) -> f64 {
    if r.total_trades < 3 { return f64::NEG_INFINITY; }
    let wr_norm = r.win_rate / 100.0;
    let pf_norm = if r.profit_factor > 0.0 { (r.profit_factor.ln() + 1.0).max(0.0) } else { 0.0 };
    let dd_pen  = (r.max_drawdown_pct / 100.0).min(1.0);
    r.sharpe_ratio * 0.35 + pf_norm * 0.25 + wr_norm * 0.25 - dd_pen * 0.15
}

/// Parametre kombinasyonu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterSet {
    pub take_profit_pct: f64,
    pub stop_loss_pct: f64,
    pub max_position_size: f64,
}

/// Optimizasyon sonucu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationResult {
    pub best_parameters: ParameterSet,
    pub best_result: BacktestResult,
    pub all_results: Vec<(ParameterSet, BacktestResult)>,
    pub total_combinations_tested: usize,
}

/// Parameter optimizer
pub struct ParameterOptimizer {
    symbol: String,
    interval: String,
    initial_balance: f64,
    strategy_name: String,
}

impl ParameterOptimizer {
    pub fn new(symbol: String, interval: String, initial_balance: f64, strategy_name: String) -> Self {
        Self {
            symbol,
            interval,
            initial_balance,
            strategy_name,
        }
    }

    /// Parametreleri optimize et
    pub fn optimize(
        &self,
        candles: &[Candle],
        tp_range: (f64, f64, f64),      // (min, max, step)
        sl_range: (f64, f64, f64),      // (min, max, step)
        ps_range: (f64, f64, f64),      // (min, max, step)
    ) -> Result<OptimizationResult> {
        if candles.is_empty() {
            return Err(MemosTradingError::Strategy(
                "Hiç mum verisi sağlanmadı".to_string(),
            ));
        }

        let mut all_results = Vec::new();
        let mut best_result: Option<(ParameterSet, BacktestResult)> = None;
        let mut combinations = 0;

        // Integer adımlarla iterate et — float birikimi kaçınır (0.1×10 ≠ 1.0 kesin değil)
        let tp_steps = ((tp_range.1 - tp_range.0) / tp_range.2).round() as usize + 1;
        let sl_steps = ((sl_range.1 - sl_range.0) / sl_range.2).round() as usize + 1;
        let ps_steps = ((ps_range.1 - ps_range.0) / ps_range.2).round() as usize + 1;

        for ti in 0..tp_steps {
            let tp = tp_range.0 + ti as f64 * tp_range.2;
            for si in 0..sl_steps {
                let sl = sl_range.0 + si as f64 * sl_range.2;
                for pi in 0..ps_steps {
                    let ps = ps_range.0 + pi as f64 * ps_range.2;
                    combinations += 1;

                    let config = BacktestConfig {
                        symbol: self.symbol.clone(),
                        interval: self.interval.clone(),
                        initial_balance: self.initial_balance,
                        max_position_size: ps,
                        take_profit_pct: tp,
                        stop_loss_pct: sl,
                        strategy_name: self.strategy_name.clone(),
                        position_profile: None,
                        security_profile: None,
                        strategy_params: None,
                        commission_pct: 0.001,
                        breakeven_at_rr: None,
                        atr_trail_mult: None,
                        partial_tp_ratio: None,
                    };

                    let mut backtester = Backtester::new(config);
                    if let Ok(result) = backtester.run(candles) {
                        let params = ParameterSet {
                            take_profit_pct: tp,
                            stop_loss_pct: sl,
                            max_position_size: ps,
                        };

                        // En iyi sonucu çok-metrik skorla seç
                        let score = composite_score(&result);
                        if let Some((_, ref best)) = best_result {
                            if score > composite_score(best) {
                                best_result = Some((params.clone(), result.clone()));
                            }
                        } else {
                            best_result = Some((params.clone(), result.clone()));
                        }

                        all_results.push((params, result));
                    }
                }
            }
        }

        if let Some((best_params, best_res)) = best_result {
            Ok(OptimizationResult {
                best_parameters: best_params,
                best_result: best_res,
                all_results,
                total_combinations_tested: combinations,
            })
        } else {
            Err(MemosTradingError::Strategy(
                "Hiçbir geçerli kombinasyon test edilemedi".to_string(),
            ))
        }
    }

    /// Hızlı optimizasyon (daha az kombinasyon)
    pub fn optimize_quick(
        &self,
        candles: &[Candle],
    ) -> Result<OptimizationResult> {
        self.optimize(
            candles,
            (3.0, 15.0, 3.0),   // TP: 3%, 6%, 9%, 12%, 15%
            (1.0, 5.0, 1.0),    // SL: 1%, 2%, 3%, 4%, 5%
            (0.1, 1.0, 0.3),    // PS: 0.1, 0.4, 0.7, 1.0
        )
    }

    /// Random Search — geniş parametre uzayında hızlı arama.
    ///
    /// Grid search'ten farklı olarak eş aralıklı değil, rastgele noktalar dener.
    /// Büyük arama uzaylarında genellikle grid'den daha iyi sonuç verir.
    ///
    /// n_iter: kaç kombinasyon deneneceği (100–500 önerilir)
    pub fn random_search(
        &self,
        candles: &[Candle],
        n_iter: usize,
        seed: Option<u64>,
    ) -> Result<OptimizationResult> {
        if candles.is_empty() {
            return Err(MemosTradingError::Strategy("Hiç mum verisi sağlanmadı".to_string()));
        }

        let mut state: u64 = seed.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(42)
        });
        let next_u64 = |s: &mut u64| -> u64 {
            *s = s.wrapping_mul(6_364_136_223_846_793_005)
                   .wrapping_add(1_442_695_040_888_963_407);
            *s
        };
        let rand_f64 = |s: &mut u64, lo: f64, hi: f64| -> f64 {
            lo + (next_u64(s) as f64 / u64::MAX as f64) * (hi - lo)
        };

        let mut all_results  = Vec::new();
        let mut best_result: Option<(ParameterSet, BacktestResult)> = None;

        for _ in 0..n_iter {
            let tp = rand_f64(&mut state, 2.0, 20.0);
            let sl = rand_f64(&mut state, 1.0,  8.0);
            let ps = rand_f64(&mut state, 0.1,  1.0);

            if tp <= sl { continue; } // geçersiz RR

            let config = BacktestConfig {
                symbol:           self.symbol.clone(),
                interval:         self.interval.clone(),
                initial_balance:  self.initial_balance,
                max_position_size: ps,
                take_profit_pct:  tp,
                stop_loss_pct:    sl,
                strategy_name:    self.strategy_name.clone(),
                position_profile: None,
                security_profile: None,
                strategy_params:  None,
                commission_pct:   0.001,
                breakeven_at_rr:  None,
                atr_trail_mult:   None,
                partial_tp_ratio: None,
            };

            if let Ok(result) = Backtester::new(config).run(candles) {
                let params = ParameterSet { take_profit_pct: tp, stop_loss_pct: sl, max_position_size: ps };
                let score = composite_score(&result);
                if let Some((_, ref best)) = best_result {
                    if score > composite_score(best) {
                        best_result = Some((params.clone(), result.clone()));
                    }
                } else {
                    best_result = Some((params.clone(), result.clone()));
                }
                all_results.push((params, result));
            }
        }

        if let Some((best_params, best_res)) = best_result {
            Ok(OptimizationResult {
                best_parameters: best_params,
                best_result: best_res,
                all_results,
                total_combinations_tested: n_iter,
            })
        } else {
            Err(MemosTradingError::Strategy("Hiçbir geçerli kombinasyon test edilemedi".to_string()))
        }
    }

    /// Çok-metrik skor ile optimize et (Sharpe+WinRate+PF ağırlıklı).
    /// Grid search ile aynı ama sonuçları composite score'a göre sıralar.
    pub fn optimize_multi_metric(
        &self,
        candles: &[Candle],
    ) -> Result<OptimizationResult> {
        let mut result = self.optimize_quick(candles)?;
        // Zaten composite_score kullanıyor; sadece all_results'ı skor sırasına diz
        result.all_results.sort_by(|(_, a), (_, b)| {
            composite_score(b).partial_cmp(&composite_score(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_candles() -> Vec<Candle> {
        let mut candles = Vec::new();
        let mut price = 100.0;

        for i in 0..100 {
            candles.push(Candle {
                symbol: "BTC".to_string(),
                interval: "1h".to_string(),
                timestamp: chrono::Utc::now() + chrono::Duration::hours(i),
                open: price,
                high: price + 2.0,
                low: price - 1.0,
                close: price + 1.0,
                volume: 1000.0,
            });

            price += (i as f64 * 0.5) % 3.0 - 1.0; // Trend
        }

        candles
    }

    #[test]
    fn test_parameter_optimizer_creation() {
        let optimizer = ParameterOptimizer::new(
            "BTC".to_string(),
            "1h".to_string(),
            1000.0,
            "MA_Crossover".to_string(),
        );

        assert_eq!(optimizer.symbol, "BTC");
    }

    #[test]
    fn test_parameter_optimizer_quick() {
        let optimizer = ParameterOptimizer::new(
            "BTC".to_string(),
            "1h".to_string(),
            1000.0,
            "MA_Crossover".to_string(),
        );

        let candles = create_test_candles();
        let result = optimizer.optimize_quick(&candles);

        assert!(result.is_ok());

        if let Ok(opt_result) = result {
            assert!(opt_result.total_combinations_tested > 0);
            assert!(!opt_result.all_results.is_empty());
        }
    }

    #[test]
    fn test_parameter_optimizer_empty_candles() {
        let optimizer = ParameterOptimizer::new(
            "BTC".to_string(),
            "1h".to_string(),
            1000.0,
            "MA_Crossover".to_string(),
        );

        let result = optimizer.optimize_quick(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parameter_set_creation() {
        let params = ParameterSet {
            take_profit_pct: 10.0,
            stop_loss_pct: 5.0,
            max_position_size: 1.0,
        };

        assert_eq!(params.take_profit_pct, 10.0);
    }
}
