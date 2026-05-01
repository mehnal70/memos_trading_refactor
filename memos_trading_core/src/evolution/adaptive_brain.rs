// Adaptive Brain - Öğrenen Yapay Zeka Beyni
// Piyasa rejimlerini tanır, strateji seçimini optimize eder

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

/// Piyasa rejimi (trend/range/volatile/crash)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MarketRegime {
    /// Güçlü yükseliş trendi
    StrongUptrend,
    
    /// Hafif yükseliş trendi
    WeakUptrend,
    
    /// Yatay hareket (range)
    Ranging,
    
    /// Hafif düşüş trendi
    WeakDowntrend,
    
    /// Güçlü düşüş trendi
    StrongDowntrend,
    
    /// Yüksek volatilite
    HighVolatility,
    
    /// Düşük volatilite
    LowVolatility,
    
    /// Bilinmiyor
    Unknown,
}

/// Adaptive Brain - Piyasa koşullarına göre strateji seçen yapay zeka
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AdaptiveBrain {
    /// Şu anki piyasa rejimi
    pub current_regime: MarketRegime,
    
    /// Rejim geçmişi (son N dönem)
    pub regime_history: VecDeque<(MarketRegime, f64)>, // (rejim, timestamp)
    
    /// Her rejimde hangi strateji daha iyi performans göstermiş.
    /// Key: "{:?}" formatındaki rejim adı (örn. "StrongUptrend") — JSON string key olarak persist edilir.
    /// Restart sonrası sıfırlanmaz; evolution_state.json ile kalıcıdır.
    #[serde(default)]
    pub regime_strategy_performance: HashMap<String, HashMap<String, f64>>,
    
    /// Öğrenme hızı (0.0 - 1.0, yüksek = hızlı adapte)
    pub learning_rate: f64,
    
    /// Keşif oranı (exploration rate): Yeni stratejileri deneme eğilimi
    pub exploration_rate: f64,
    
    /// Q-learning tablosu: (state, action) -> expected_reward
    pub q_table: HashMap<String, f64>,
    
    /// Sharpe-normalized ödül geçmişi
    pub reward_history: VecDeque<f64>,

    /// Ham PnL geçmişi (%) — Sharpe hesabı için
    pub pnl_history: VecDeque<f64>,

    /// Toplam öğrenme adımı
    pub total_learning_steps: u64,
}

impl AdaptiveBrain {
    /// Yeni bir adaptive brain oluştur
    pub fn new() -> Self {
        Self {
            current_regime: MarketRegime::Unknown,
            regime_history: VecDeque::with_capacity(100),
            regime_strategy_performance: HashMap::new(),
            learning_rate: 0.1,
            exploration_rate: 0.2, // %20 exploration
            q_table: HashMap::new(),
            reward_history: VecDeque::with_capacity(1000),
            pnl_history:    VecDeque::with_capacity(200),
            total_learning_steps: 0,
        }
    }
    
    /// Piyasa verilerinden rejimi tespit et
    pub fn detect_market_regime(&mut self, closes: &[f64], _volumes: &[f64]) -> MarketRegime {
        if closes.len() < 20 {
            return MarketRegime::Unknown;
        }
        
        // 1. Trend analizi (son 20 bar)
        let recent_closes = &closes[closes.len() - 20..];
        let first_close = recent_closes[0];
        let last_close = *recent_closes.last().unwrap();
        let trend_pct = ((last_close - first_close) / first_close) * 100.0;
        
        // 2. Volatilite analizi (son 20 bar'ın std sapması)
        let mean = recent_closes.iter().sum::<f64>() / recent_closes.len() as f64;
        let variance = recent_closes.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / recent_closes.len() as f64;
        let std_dev = variance.sqrt();
        let volatility_pct = (std_dev / mean) * 100.0;
        
        // 3. Rejim belirle — trend önce kontrol edilir; güçlü trend varsa volatilite ikincil
        let regime = if trend_pct > 5.0 {
            MarketRegime::StrongUptrend
        } else if trend_pct > 2.0 {
            MarketRegime::WeakUptrend
        } else if trend_pct < -5.0 {
            MarketRegime::StrongDowntrend
        } else if trend_pct < -2.0 {
            MarketRegime::WeakDowntrend
        } else if volatility_pct > 3.0 {
            MarketRegime::HighVolatility
        } else if volatility_pct < 0.5 {
            MarketRegime::LowVolatility
        } else {
            MarketRegime::Ranging
        };
        
        // Geçmişe ekle
        self.regime_history.push_back((regime.clone(), chrono::Utc::now().timestamp() as f64));
        if self.regime_history.len() > 100 {
            self.regime_history.pop_front();
        }
        
        self.current_regime = regime.clone();
        regime
    }
    
    /// Hangi stratejiyi kullanacağına karar ver (Q-learning + exploration)
    pub fn select_strategy(&mut self, available_strategies: &[String]) -> String {
        if available_strategies.is_empty() {
            return "default".to_string();
        }
        
        // Exploration vs Exploitation
        if rand_range(0.0, 1.0) < self.exploration_rate {
            // Exploration: Rastgele strateji seç
            let idx = rand_range(0.0, available_strategies.len() as f64) as usize;
            return available_strategies[idx].clone();
        }
        
        // Exploitation: Q-table'dan en yüksek reward'lı stratejiyi seç
        let state_key = format!("{:?}", self.current_regime);
        let mut best_strategy = available_strategies[0].clone();
        let mut best_q_value = f64::NEG_INFINITY;
        
        for strategy in available_strategies {
            let q_key = format!("{}|{}", state_key, strategy);
            let q_value = *self.q_table.get(&q_key).unwrap_or(&0.0);
            
            if q_value > best_q_value {
                best_q_value = q_value;
                best_strategy = strategy.clone();
            }
        }
        
        best_strategy
    }
    
    /// Bir trade sonucundan öğren — Sharpe-normalized ödül ile
    pub fn learn_from_trade(
        &mut self,
        regime: &MarketRegime,
        strategy_used: &str,
        pnl_pct: f64,
    ) {
        // Ham PnL geçmişini güncelle
        self.pnl_history.push_back(pnl_pct);
        if self.pnl_history.len() > 200 { self.pnl_history.pop_front(); }

        // ── Sharpe-normalized ödül ─────────────────────────────────────────
        // Yeterli geçmiş (≥10 trade) varsa rolling z-score normalisation:
        //   reward = (pnl - mean) / std  →  klamp ±3 → /3 → [-1, +1]
        // Daha az geçmiş varsa basit: pnl / 10
        let reward = if self.pnl_history.len() >= 10 {
            let vals: Vec<f64> = self.pnl_history.iter().copied().collect();
            let mean = vals.iter().sum::<f64>() / vals.len() as f64;
            let std  = (vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
                        / vals.len() as f64).sqrt();
            if std > 0.05 {
                ((pnl_pct - mean) / std).clamp(-3.0, 3.0) / 3.0
            } else {
                pnl_pct / 10.0
            }
        } else {
            pnl_pct / 10.0
        };

        // Q-learning update: Q(s,a) ← Q(s,a) + α·[reward − Q(s,a)]
        let state_key = format!("{:?}", regime);
        let q_key     = format!("{}|{}", state_key, strategy_used);
        let current_q = *self.q_table.get(&q_key).unwrap_or(&0.0);
        self.q_table.insert(q_key, current_q + self.learning_rate * (reward - current_q));

        // Rejim-strateji EMA performansı — String key ile persist edilir
        let regime_key = format!("{:?}", regime);
        let perf = self.regime_strategy_performance
            .entry(regime_key).or_default()
            .entry(strategy_used.to_string()).or_insert(0.0);
        *perf = *perf * 0.95 + reward * 0.05;

        // Ödül geçmişi
        self.reward_history.push_back(reward);
        if self.reward_history.len() > 1000 { self.reward_history.pop_front(); }

        self.total_learning_steps += 1;

        // Epsilon decay — her 100 adımda %1 azalt, minimum %5
        if self.total_learning_steps % 100 == 0 {
            self.exploration_rate = (self.exploration_rate * 0.99).max(0.05);
        }
    }
    
    /// Belirli bir rejimdeki en iyi stratejiyi öner
    pub fn recommend_strategy_for_regime(&self, regime: &MarketRegime) -> Option<String> {
        let regime_key = format!("{:?}", regime);
        self.regime_strategy_performance
            .get(&regime_key)
            .and_then(|strategy_map| {
                strategy_map
                    .iter()
                    .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(strategy, _)| strategy.clone())
            })
    }
    
    /// Son N trade'in ortalama reward'ı (performans metriği)
    pub fn get_average_recent_reward(&self, n: usize) -> f64 {
        let recent: Vec<f64> = self.reward_history
            .iter()
            .rev()
            .take(n)
            .copied()
            .collect();
        
        if recent.is_empty() {
            return 0.0;
        }
        
        recent.iter().sum::<f64>() / recent.len() as f64
    }
    
    /// Brain özeti
    pub fn get_summary(&self) -> String {
        format!(
            "Rejim: {:?}, Q-table boyutu: {}, Öğrenme adımı: {}, Exploration: {:.1}%, Ortalama reward (son 100): {:.3}",
            self.current_regime,
            self.q_table.len(),
            self.total_learning_steps,
            self.exploration_rate * 100.0,
            self.get_average_recent_reward(100)
        )
    }
}

impl Default for AdaptiveBrain {
    fn default() -> Self {
        Self::new()
    }
}

// Yardımcı fonksiyon
fn rand_range(min: f64, max: f64) -> f64 {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    rng.gen_range(min..=max)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_regime_detection() {
        let mut brain = AdaptiveBrain::new();
        
        // Uptrend scenario: fiyatlar sürekli artıyor
        let closes: Vec<f64> = (100..120).map(|x| x as f64).collect();
        let volumes: Vec<f64> = vec![1000.0; 20];
        
        let regime = brain.detect_market_regime(&closes, &volumes);
        assert!(matches!(regime, MarketRegime::StrongUptrend | MarketRegime::WeakUptrend));
    }
    
    #[test]
    fn test_learning() {
        let mut brain = AdaptiveBrain::new();
        
        // Simüle et: Ranging'de RSI stratejisi daha iyi
        for _ in 0..100 {
            brain.learn_from_trade(&MarketRegime::Ranging, "RSI", 2.0); // +2% PnL
            brain.learn_from_trade(&MarketRegime::Ranging, "MA", -1.0); // -1% PnL
        }
        
        // RSI'nin Q-value'su MA'dan yüksek olmalı
        let rsi_q = *brain.q_table.get("Ranging|RSI").unwrap_or(&0.0);
        let ma_q = *brain.q_table.get("Ranging|MA").unwrap_or(&0.0);
        
        assert!(rsi_q > ma_q);
    }
    
    #[test]
    fn test_strategy_recommendation() {
        let mut brain = AdaptiveBrain::new();
        
        // Öğrenme yap
        for _ in 0..50 {
            brain.learn_from_trade(&MarketRegime::StrongUptrend, "MA", 3.0);
        }
        
        let recommended = brain.recommend_strategy_for_regime(&MarketRegime::StrongUptrend);
        assert_eq!(recommended, Some("MA".to_string()));
    }
}
