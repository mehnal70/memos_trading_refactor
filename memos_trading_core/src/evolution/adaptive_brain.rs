// evolution/adaptive_brain.rs - Otonom Öğrenen Yapay Zeka Beyni (Q-Learning)

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use chrono::Utc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MarketRegime {
    StrongUptrend, WeakUptrend, Ranging, WeakDowntrend, StrongDowntrend,
    HighVolatility, LowVolatility, Unknown,
}

impl MarketRegime {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::StrongUptrend => "StrongUptrend", Self::WeakUptrend => "WeakUptrend",
            Self::Ranging => "Ranging", Self::WeakDowntrend => "WeakDowntrend",
            Self::StrongDowntrend => "StrongDowntrend", Self::HighVolatility => "HighVolatility",
            Self::LowVolatility => "LowVolatility", Self::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveBrain {
    pub current_regime: MarketRegime,
    pub regime_history: VecDeque<(MarketRegime, i64)>,
    #[serde(default)]
    pub regime_strategy_performance: HashMap<MarketRegime, HashMap<String, f64>>,
    pub learning_rate: f64,
    pub exploration_rate: f64,
    pub q_table: HashMap<(MarketRegime, String), f64>,
    pub reward_history: VecDeque<f64>,
    pub pnl_history: VecDeque<f64>,
    pub total_learning_steps: u64,
}

impl AdaptiveBrain {
    pub fn new() -> Self {
        Self {
            current_regime: MarketRegime::Unknown,
            regime_history: VecDeque::with_capacity(100),
            regime_strategy_performance: HashMap::with_capacity(8),
            learning_rate: 0.1, exploration_rate: 0.2,
            q_table: HashMap::with_capacity(50),
            reward_history: VecDeque::with_capacity(1000),
            pnl_history: VecDeque::with_capacity(200),
            total_learning_steps: 0,
        }
    }

    // --- CONTROLLER KÖPRÜSÜ (Hata Giderici Metod) ---

    /// AutonomousController'ın beklediği kritik metod. 
    /// 'learn_from_trade' mantığını kullanarak performansı mühürler.
    pub fn record_performance(&mut self, regime: &MarketRegime, strategy_name: &str, pnl_pct: f64) {
        self.current_regime = *regime;
        self.learn_from_trade(*regime, strategy_name, pnl_pct);
    }

    // --- ÖĞRENME VE KARAR ÇEKİRDEĞİ ---

    pub fn detect_market_regime(&mut self, closes: &[f64], _volumes: &[f64]) -> MarketRegime {
        let n = closes.len();
        if n < 20 { return MarketRegime::Unknown; }
        let recent = &closes[n - 20..];
        let trend_pct = ((recent[19] - recent[0]) / recent[0]) * 100.0;
        let mean = recent.iter().sum::<f64>() / 20.0;
        let std_dev = (recent.iter().map(|&c| (c - mean).powi(2)).sum::<f64>() / 20.0).sqrt();
        let volatility_pct = (std_dev / mean) * 100.0;

        let regime = match trend_pct {
            t if t > 5.0 => MarketRegime::StrongUptrend,
            t if t > 2.0 => MarketRegime::WeakUptrend,
            t if t < -5.0 => MarketRegime::StrongDowntrend,
            t if t < -2.0 => MarketRegime::WeakDowntrend,
            _ if volatility_pct > 3.0 => MarketRegime::HighVolatility,
            _ if volatility_pct < 0.5 => MarketRegime::LowVolatility,
            _ => MarketRegime::Ranging,
        };
        self.current_regime = regime;
        self.regime_history.push_back((regime, Utc::now().timestamp()));
        if self.regime_history.len() > 100 { self.regime_history.pop_front(); }
        regime
    }

    pub fn select_strategy(&mut self, available_strategies: &[String]) -> String {
        if available_strategies.is_empty() { return "default".to_owned(); }
        use rand::Rng;
        if rand::thread_rng().gen::<f64>() < self.exploration_rate {
            let idx = rand::thread_rng().gen_range(0..available_strategies.len());
            return available_strategies[idx].clone();
        }
        available_strategies.iter()
            .map(|s| (s, self.q_table.get(&(self.current_regime, s.clone())).unwrap_or(&0.0)))
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(s, _)| s.clone()).unwrap_or_else(|| available_strategies[0].clone())
    }

    pub fn learn_from_trade(&mut self, regime: MarketRegime, strategy_used: &str, pnl_pct: f64) {
        self.pnl_history.push_back(pnl_pct);
        if self.pnl_history.len() > 200 { self.pnl_history.pop_front(); }

        let reward = if self.pnl_history.len() >= 10 {
            let mean = self.pnl_history.iter().sum::<f64>() / self.pnl_history.len() as f64;
            let std = (self.pnl_history.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / self.pnl_history.len() as f64).sqrt();
            if std > 0.05 { ((pnl_pct - mean) / std).clamp(-3.0, 3.0) / 3.0 } else { pnl_pct / 10.0 }
        } else { pnl_pct / 10.0 };

        let q_key = (regime, strategy_used.to_owned());
        let current_q = *self.q_table.get(&q_key).unwrap_or(&0.0);
        self.q_table.insert(q_key, current_q + self.learning_rate * (reward - current_q));

        let perf = self.regime_strategy_performance.entry(regime).or_default().entry(strategy_used.to_owned()).or_insert(0.0);
        *perf = *perf * 0.95 + reward * 0.05;

        self.reward_history.push_back(reward);
        if self.reward_history.len() > 1000 { self.reward_history.pop_front(); }
        self.total_learning_steps += 1;
        if self.total_learning_steps.is_multiple_of(100) { self.exploration_rate = (self.exploration_rate * 0.99).max(0.05); }
    }

    pub fn get_summary(&self) -> String {
        let count = self.reward_history.len().min(100);
        let avg_reward = if count == 0 { 0.0 } else { self.reward_history.iter().rev().take(count).sum::<f64>() / count as f64 };
        format!("Regime: {:?} | Steps: {} | Explore: {:.1}% | AvgReward: {:.3}", 
                self.current_regime, self.total_learning_steps, self.exploration_rate * 100.0, avg_reward)
    }
}

impl Default for AdaptiveBrain { fn default() -> Self { Self::new() } }
