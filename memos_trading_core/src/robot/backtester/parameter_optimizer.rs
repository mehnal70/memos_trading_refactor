// parameter_optimizer.rs - Yüksek Performanslı Strateji Optimizasyon Motoru

use crate::core::types::Candle;
use crate::robot::backtester::{Backtester, BacktestConfig, BacktestResult};
use crate::Result;
use crate::MemosTradingError;
use serde::{Deserialize, Serialize};
use rayon::prelude::*; // Paralel işleme desteği

// --- 1. KOMPOZİT SKORLAMA MOTORU ---

/// Çok boyutlu başarı skoru hesaplar.
/// Sharpe (%35), Profit Factor (%25), WinRate (%25), Drawdown (%15 penalty)
fn calculate_composite_score(r: &BacktestResult) -> f64 {
    if r.total_trades < 3 { return f64::NEG_INFINITY; }
    
    let wr_norm = r.win_rate / 100.0;
    // Logaritmik Profit Factor normalizasyonu (O(1) kompleksite)
    let pf_norm = if r.profit_factor > 0.0 { 
        (r.profit_factor.ln() + 1.0).max(0.0) 
    } else { 0.0 };
    
    let dd_penalty = (r.max_drawdown_pct / 100.0).clamp(0.0, 1.0);
    
    (r.sharpe_ratio * 0.35) + (pf_norm * 0.25) + (wr_norm * 0.25) - (dd_penalty * 0.15)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterSet {
    pub take_profit_pct: f64,
    pub stop_loss_pct: f64,
    pub max_position_size: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationResult {
    pub best_parameters: ParameterSet,
    pub best_result: BacktestResult,
    pub total_tested: usize,
}

// --- 2. OPTİMİZASYON YÖNETİCİSİ ---

pub struct ParameterOptimizer {
    symbol: String,
    interval: String,
    initial_balance: f64,
    strategy_name: String,
    /// Giriş kalitesi filtresi (#4): create_config → BacktestConfig.edge_min_score.
    /// Default None (filtre yok); `with_edge_min_score` ile set edilir.
    edge_min_score: Option<f64>,
    /// Orderbook icrası (#c): create_config → BacktestConfig.orderbook_sim. Default None.
    orderbook_sim: Option<String>,
    /// Canlı çıkış modeli: ATR-trail çarpanı. TP/SL araması canlının uyguladığı
    /// trailing'le BİRLİKTE yapılsın diye. Default None (geriye-uyum: trailing'siz).
    atr_trail_mult: Option<f64>,
    /// Canlı çıkış modeli: breakeven RR eşiği (canlı default 1.0). Default None.
    breakeven_at_rr: Option<f64>,
}

impl ParameterOptimizer {
    pub fn new(symbol: String, interval: String, initial_balance: f64, strategy_name: String) -> Self {
        Self { symbol, interval, initial_balance, strategy_name,
               edge_min_score: None, orderbook_sim: None,
               atr_trail_mult: None, breakeven_at_rr: None }
    }

    /// Canlı çıkış modelini ayarlar (TP/SL araması trailing + breakeven'ı görür).
    /// `None,None` → trailing'siz (eski davranış). Backtest/ML job canlı-temsili
    /// trail mult'u (target_trail_pct / noise_floor) + breakeven 1.0 ile doldurur.
    pub fn with_exit_model(mut self, atr_trail_mult: Option<f64>, breakeven_at_rr: Option<f64>) -> Self {
        self.atr_trail_mult = atr_trail_mult;
        self.breakeven_at_rr = breakeven_at_rr;
        self
    }

    /// Giriş kalitesi edge eşiğini ayarlar (TP/SL/PS aramasının tüm alt-backtest'leri
    /// canlının edge hunisini görür). `None` → filtre yok.
    pub fn with_edge_min_score(mut self, edge_min_score: Option<f64>) -> Self {
        self.edge_min_score = edge_min_score;
        self
    }

    /// Orderbook icra profilini ayarlar (slippage). `None` → fill=close.
    pub fn with_orderbook_sim(mut self, orderbook_sim: Option<String>) -> Self {
        self.orderbook_sim = orderbook_sim;
        self
    }

    /// Grid Search: Tüm kombinasyonları paralel olarak test eder.
    /// Performans: Rayon ile tüm işlemci çekirdeklerini kullanır.
    pub fn optimize_parallel(
        &self,
        candles: &[Candle],
        tp_range: (f64, f64, f64),
        sl_range: (f64, f64, f64),
        ps_range: (f64, f64, f64),
    ) -> Result<OptimizationResult> {
        if candles.is_empty() { return Err(MemosTradingError::Strategy("Boş veri".to_owned())); }

        // Kombinasyon listesini önceden oluştur (Allocation-optimized)
        let mut configs = Vec::new();
        let mut t = tp_range.0;
        while t <= tp_range.1 {
            let mut s = sl_range.0;
            while s <= sl_range.1 {
                let mut p = ps_range.0;
                while p <= ps_range.1 {
                    configs.push(ParameterSet { take_profit_pct: t, stop_loss_pct: s, max_position_size: p });
                    p += ps_range.2;
                }
                s += sl_range.2;
            }
            t += tp_range.2;
        }

        let total_combinations = configs.len();

        // PARALEL İŞLEME: Backtest'leri eş zamanlı çalıştır
        let results: Vec<(ParameterSet, BacktestResult)> = configs.into_par_iter()
            .filter_map(|params| {
                let config = self.create_config(&params);
                let mut backtester = Backtester::new(config);
                backtester.run(candles).ok().map(|res| (params, res))
            })
            .collect();

        // En iyi sonucu bul (Composite Score bazlı)
        results.into_iter()
            .max_by(|a, b| {
                let score_a = calculate_composite_score(&a.1);
                let score_b = calculate_composite_score(&b.1);
                score_a.partial_cmp(&score_b).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(best_params, best_res)| OptimizationResult {
                best_parameters: best_params,
                best_result: best_res,
                total_tested: total_combinations,
            })
            .ok_or_else(|| MemosTradingError::Strategy("Geçerli sonuç bulunamadı".to_owned()))
    }

    /// Random Search: Geniş parametre uzayında hızlı keşif yapar.
    pub fn random_search(
        &self,
        candles: &[Candle],
        n_iter: usize,
    ) -> Result<OptimizationResult> {
        use rand::Rng;
        let mut _rng = rand::thread_rng();

        let results: Vec<(ParameterSet, BacktestResult)> = (0..n_iter)
            .into_par_iter()
            .filter_map(|_| {
                let mut local_rng = rand::thread_rng();
                let params = ParameterSet {
                    take_profit_pct: local_rng.gen_range(2.0..20.0),
                    stop_loss_pct: local_rng.gen_range(1.0..8.0),
                    max_position_size: local_rng.gen_range(0.1..1.0),
                };

                if params.take_profit_pct <= params.stop_loss_pct { return None; }

                let config = self.create_config(&params);
                Backtester::new(config).run(candles).ok().map(|res| (params, res))
            })
            .collect();

        results.into_iter()
            .max_by(|a, b| {
                calculate_composite_score(&a.1).partial_cmp(&calculate_composite_score(&b.1))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(p, r)| OptimizationResult { best_parameters: p, best_result: r, total_tested: n_iter })
            .ok_or_else(|| MemosTradingError::Strategy("Arama başarısız".to_owned()))
    }

    // --- YARDIMCI METODLAR ---

    fn create_config(&self, p: &ParameterSet) -> BacktestConfig {
        BacktestConfig {
            symbol: self.symbol.clone(),
            interval: self.interval.clone(),
            initial_balance: self.initial_balance,
            max_position_size: p.max_position_size,
            take_profit_pct: p.take_profit_pct,
            stop_loss_pct: p.stop_loss_pct,
            strategy_name: self.strategy_name.clone(),
            strategy_params: None,
            commission_pct: 0.001,
            edge_min_score: self.edge_min_score,
            orderbook_sim: self.orderbook_sim.clone(),
            atr_trail_mult: self.atr_trail_mult,
            breakeven_at_rr: self.breakeven_at_rr,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Belirgin zigzag (~%3 tepe-dip) → trailing tetiklenir; çıkış modeli TP/SL
    /// aramasının sonucunu fiilen değiştirmeli (eskiden trailing'siz seçiliyordu).
    fn zigzag(n: usize) -> Vec<Candle> {
        (0..n).map(|i| {
            let phase = (i / 6) % 2;
            let dir = if phase == 0 { 1.0 } else { -1.0 };
            let base = 100.0 + dir * (i % 6) as f64 * 0.5;
            let c = base + dir * 0.5;
            Candle { open: base, high: c.max(base) + 0.5, low: c.min(base) - 0.5, close: c,
                     volume: 1000.0, symbol: "T".into(), interval: "1h".into(),
                     ..Default::default() }
        }).collect()
    }

    #[test]
    fn exit_model_is_wired_into_tp_sl_search() {
        let candles = zigzag(300);
        let mk = || ParameterOptimizer::new(
            "T".into(), "1h".into(), 10_000.0, "EMA_CROSSOVER".into());
        let ranges = ((2.0, 6.0, 2.0), (1.0, 3.0, 1.0), (0.2, 0.4, 0.2));

        let no_trail = mk().optimize_parallel(&candles, ranges.0, ranges.1, ranges.2)
            .expect("trailing'siz arama Ok");
        let with_trail = mk().with_exit_model(Some(2.0), Some(1.0))
            .optimize_parallel(&candles, ranges.0, ranges.1, ranges.2)
            .expect("trailing'li arama Ok");

        assert!(no_trail.best_result.total_pnl.is_finite());
        assert!(with_trail.best_result.total_pnl.is_finite());
        // Trailing çıkışı değiştirdiği için en az bir metrik farklılaşmalı
        // (seçilen TP/SL ya da realized PnL). Aynıysa çıkış modeli bağlanmamış demektir.
        let differs = (with_trail.best_result.total_pnl - no_trail.best_result.total_pnl).abs() > 1e-9
            || with_trail.best_parameters.take_profit_pct != no_trail.best_parameters.take_profit_pct
            || with_trail.best_parameters.stop_loss_pct != no_trail.best_parameters.stop_loss_pct;
        assert!(differs, "çıkış modeli TP/SL aramasını etkilemeli (trailing bağlı değil?)");
    }

    #[test]
    fn exit_model_defaults_to_none() {
        let opt = ParameterOptimizer::new("T".into(), "1h".into(), 10_000.0, "RSI".into());
        let cfg = opt.create_config(&ParameterSet {
            take_profit_pct: 3.0, stop_loss_pct: 1.5, max_position_size: 0.3 });
        assert_eq!(cfg.atr_trail_mult, None, "default trailing'siz (geriye-uyum)");
        assert_eq!(cfg.breakeven_at_rr, None);
    }
}

