// src/evolution/autonomous_controller.rs - Otonom Strateji ve Evrim Yönetimi

use crate::evolution::{PopulationManager, AdaptiveBrain, MarketRegime, StrategyGenome};
use serde::{Serialize, Deserialize};
use std::time::{Instant, Duration};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AutonomousState {
    Observe,    // Sadece izle, veri topla
    Optimize,   // Parametreleri iyileştir
    Trade,      // Aktif işlem yap
    SafeMode,   // Riskli rejim, hacim düşür
    Halted,     // Acil durdurma
}

impl std::fmt::Display for AutonomousState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

pub struct AutonomousControllerConfig {
    pub evolution_enabled: bool,
    pub evolve_every_n_cycles: u64,
    pub failure_threshold: usize,
}

impl Default for AutonomousControllerConfig {
    fn default() -> Self {
        Self {
            evolution_enabled: true,
            evolve_every_n_cycles: 24, // Örn: 24 saatlik döngü
            failure_threshold: 5,
        }
    }
}

#[derive(Clone)]
pub struct AutonomousController {
    pub state: AutonomousState,
    pub cycle_id: u64,
    pub evolution_enabled: bool,
    pub evolve_every_n_cycles: u64,
    pub consecutive_failures: usize,
    pub adaptive_brain: Option<AdaptiveBrain>,
    pub population_manager: Option<PopulationManager>,
    pub current_strategy_genome: Option<StrategyGenome>,
    pub last_evolution_at: Option<Instant>,
}

impl AutonomousController {
    pub fn new(config: AutonomousControllerConfig) -> Self {
        Self {
            state: AutonomousState::Observe,
            cycle_id: 0,
            evolution_enabled: config.evolution_enabled,
            evolve_every_n_cycles: config.evolve_every_n_cycles,
            consecutive_failures: 0,
            adaptive_brain: None,
            population_manager: None,
            current_strategy_genome: None,
            last_evolution_at: None,
        }
    }

    /// robotic_loop içindeki can_trade() kontrolü
    pub fn can_trade(&self) -> bool {
        matches!(self.state, AutonomousState::Trade | AutonomousState::SafeMode | AutonomousState::Observe)
    }

    /// Yeni bir döngü başlat (Kısım 56'daki mantık)
    pub fn begin_cycle(&mut self) {
        self.cycle_id += 1;
    }

    /// Evrim zamanı gelmiş mi?
    pub fn should_evolve(&self) -> bool {
        self.evolution_enabled && self.cycle_id.is_multiple_of(self.evolve_every_n_cycles)
    }

    /// Nüfusu evrimleştir ve en iyi genoma geç
    pub fn evolve_population(&mut self) {
        if let Some(ref mut pop_mgr) = self.population_manager {
            pop_mgr.evolve();
            self.current_strategy_genome = pop_mgr.get_best_strategy().cloned();
            self.last_evolution_at = Some(Instant::now());
        }
    }

    /// İşlem sonucundan öğren (Kısım 45'teki mantık)
    pub fn learn_from_trade(&mut self, pnl_pct: f64, regime: &MarketRegime, strategy_name: &str) {
        if let Some(ref mut brain) = self.adaptive_brain {
            brain.record_performance(regime, strategy_name, pnl_pct);
        }

        if pnl_pct < 0.0 {
            self.consecutive_failures += 1;
            if self.consecutive_failures >= 5 {
                self.state = AutonomousState::SafeMode;
            }
        } else {
            self.consecutive_failures = 0;
            if self.state == AutonomousState::SafeMode {
                self.state = AutonomousState::Trade;
            }
        }
    }

    pub fn force_safe_mode(&mut self, _dummy: u32) {
        self.state = AutonomousState::SafeMode;
    }

    pub fn force_observe(&mut self) {
        self.state = AutonomousState::Observe;
    }

    pub fn transition_success(&mut self) {
        if self.state == AutonomousState::Observe && self.cycle_id > 10 {
            self.state = AutonomousState::Trade;
        }
    }

    pub fn transition_failure(&mut self, _reason: &str) {
        self.consecutive_failures += 1;
    }
}
