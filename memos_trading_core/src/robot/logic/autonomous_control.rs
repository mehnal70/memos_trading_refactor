use std::fmt;
use crate::evolution::{AdaptiveBrain, PopulationManager, GeneticParams, StrategyGenome, MarketRegime};
use crate::evolution::strategy_genome::SelectionMethod;

/// AutonomousControllerConfig: Otonom kontrolcünün çalışma kuralları.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AutonomousControllerConfig {
    /// Evrim mekanizması aktif mi?
    pub evolution_enabled: bool,
    /// Kaç işlem döngüsünde bir evrim tetiklensin? (Örn: 24)
    pub evolve_every_n_cycles: u64,
    /// Ardışık kaç kayıptan sonra SafeMode aktif olsun? (Örn: 5)
    pub failure_threshold: usize,
    /// Başlangıç durumu (Observe/Trade/SafeMode)
    pub initial_state: AutonomousState,
}

impl Default for AutonomousControllerConfig {
    fn default() -> Self {
        Self {
            evolution_enabled: true,
            evolve_every_n_cycles: 24, 
            failure_threshold: 5,
            initial_state: AutonomousState::Observe,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AutonomousState {
    Observe,
    Decide,
    Validate,
    Execute,
    Verify,
    Adapt,
    SafeMode,
    Halted,
}

impl fmt::Display for AutonomousState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            AutonomousState::Observe => "Observe",
            AutonomousState::Decide => "Decide",
            AutonomousState::Validate => "Validate",
            AutonomousState::Execute => "Execute",
            AutonomousState::Verify => "Verify",
            AutonomousState::Adapt => "Adapt",
            AutonomousState::SafeMode => "SafeMode",
            AutonomousState::Halted => "Halted",
        };
        write!(f, "{}", name)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AutonomousConfig {
    pub max_failures_before_safe_mode: usize,
    pub max_failures_before_halt: usize,
    pub safe_mode_cooldown_cycles: u64,
}

impl Default for AutonomousConfig {
    fn default() -> Self {
        Self {
            max_failures_before_safe_mode: 3,
            max_failures_before_halt: 7,
            safe_mode_cooldown_cycles: 5,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AutonomousTransition {
    pub from: AutonomousState,
    pub to: AutonomousState,
    pub reason: String,
    pub cycle_id: u64,
}

#[derive(Debug, Clone)]
pub struct AutonomousController {
    pub state: AutonomousState,
    pub cycle_id: u64,
    pub consecutive_failures: usize,
    pub config: AutonomousConfig,
    safe_mode_until_cycle: Option<u64>,
    
    // 🧬 Evrimsel AI Bileşenleri
    pub adaptive_brain: Option<AdaptiveBrain>,
    pub population_manager: Option<PopulationManager>,
    pub current_strategy_genome: Option<StrategyGenome>,
    pub evolution_enabled: bool,
    pub evolve_every_n_cycles: u64,
}

impl AutonomousController {
    pub fn new(config: AutonomousConfig) -> Self {
        Self {
            state: AutonomousState::Observe,
            cycle_id: 0,
            consecutive_failures: 0,
            config,
            safe_mode_until_cycle: None,
            adaptive_brain: None,
            population_manager: None,
            current_strategy_genome: None,
            evolution_enabled: false,
            evolve_every_n_cycles: 50, // Her 50 cycle'da bir evrimleşir
        }
    }
    
    /// Evrimsel AI'yi aktifleştir
    pub fn enable_evolution(&mut self, strategy_type: String) {
        self.adaptive_brain = Some(AdaptiveBrain::new());
        
        let genetic_params = GeneticParams {
            population_size: 15,
            mutation_rate: 0.2,
            mutation_strength: 0.15,
            selection_method: SelectionMethod::Tournament,
            max_generations: 100,
            elitism_count: 3,
            crossover_rate: 0.7,
        };
        
        self.population_manager = Some(PopulationManager::new(strategy_type, genetic_params));
        self.evolution_enabled = true;
        
        // İlk stratejiyi seç
        if let Some(pm) = &self.population_manager {
            self.current_strategy_genome = pm.get_best_strategy().cloned();
        }
    }
    
    /// Piyasa rejimini tespit et ve strateji seç
    pub fn detect_and_select_strategy(&mut self, closes: &[f64], volumes: &[f64]) -> Option<String> {
        if !self.evolution_enabled {
            return None;
        }
        
        // Adaptive brain ile piyasa rejimini tespit et
        if let Some(brain) = &mut self.adaptive_brain {
            let _regime = brain.detect_market_regime(closes, volumes);
            
            // Mevcut popülasyondan stratejileri al
            if let Some(pm) = &self.population_manager {
                let strategies: Vec<String> = pm.current_population
                    .iter()
                    .map(|g| g.strategy_type.clone())
                    .collect();
                
                if !strategies.is_empty() {
                    let selected = brain.select_strategy(&strategies);
                    return Some(selected);
                }
            }
        }
        
        None
    }
    
    /// Trade sonucundan öğren
    pub fn learn_from_trade(&mut self, pnl_pct: f64, regime: &MarketRegime, strategy_used: &str) {
        if !self.evolution_enabled {
            return;
        }
        
        // Adaptive brain öğrensin
        if let Some(brain) = &mut self.adaptive_brain {
            brain.learn_from_trade(*regime, strategy_used, pnl_pct);
        }
        
        // Mevcut genomun performansını güncelle
        if let Some(genome) = &mut self.current_strategy_genome {
            genome.trade_count += 1;
            genome.total_pnl_pct += pnl_pct;
            
            if pnl_pct > 0.0 {
                genome.win_rate = ((genome.win_rate * (genome.trade_count - 1) as f64) + 100.0) / genome.trade_count as f64;
            } else {
                genome.win_rate = (genome.win_rate * (genome.trade_count - 1) as f64) / genome.trade_count as f64;
            }
            
            genome.survival_cycles = self.cycle_id as u32;
            genome.calculate_fitness();
        }
    }
    
    /// Evrimleşme zamanı geldi mi kontrol et.
    ///
    /// Ek şart: mevcut genomun en az `MIN_GENOME_TRADES` işlem görmüş olması gerekir.
    /// Daha az işlemle hesaplanan fitness istatistiksel olarak güvenilmez.
    pub fn should_evolve(&self) -> bool {
        const MIN_GENOME_TRADES: usize = 10;
        if !self.evolution_enabled { return false; }
        if self.cycle_id == 0 || self.cycle_id % self.evolve_every_n_cycles != 0 { return false; }
        // Yeterli işlem görülmeden evrimleşme — fitness güvenilir değil
        let genome_trades = self.current_strategy_genome.as_ref()
            .map(|g| g.trade_count)
            .unwrap_or(0);
        genome_trades >= MIN_GENOME_TRADES
    }
    
    /// Yeni nesil oluştur (evrim adımı)
    pub fn evolve_population(&mut self) {
        if let Some(pm) = &mut self.population_manager {
            pm.evolve_next_generation();
            
            // Yeni neslin en iyisini seç
            self.current_strategy_genome = pm.get_best_strategy().cloned();
            
            // TUI modunda stdout'a yazma — logger üzerinden geliyor
            let _ = (pm.get_summary(), self.adaptive_brain.as_ref().map(|b| b.get_summary()).unwrap_or_default());
        }
    }
    
    /// Evrimsel AI durumu özeti
    pub fn get_evolution_summary(&self) -> String {
        if !self.evolution_enabled {
            return "Evolution: Disabled".to_string();
        }
        
        let brain_status = self.adaptive_brain.as_ref()
            .map(|b| format!("Brain: {}", b.get_summary()))
            .unwrap_or_else(|| "Brain: N/A".to_string());
        
        let pop_status = self.population_manager.as_ref()
            .map(|pm| format!("Pop: {}", pm.get_summary()))
            .unwrap_or_else(|| "Pop: N/A".to_string());
        
        let genome_status = self.current_strategy_genome.as_ref()
            .map(|g| format!("Current: {} (fitness={:.1}, trades={})", g.id, g.fitness, g.trade_count))
            .unwrap_or_else(|| "Current: None".to_string());
        
        format!("{} | {} | {}", brain_status, pop_status, genome_status)
    }

    pub fn begin_cycle(&mut self) {
        self.cycle_id += 1;
        if self.state == AutonomousState::SafeMode {
            if let Some(until_cycle) = self.safe_mode_until_cycle {
                if self.cycle_id >= until_cycle {
                    self.state = AutonomousState::Observe;
                    self.safe_mode_until_cycle = None;
                }
            }
        }
    }

    pub fn transition_success(&mut self) -> Option<AutonomousTransition> {
        let previous = self.state;
        let next = match self.state {
            AutonomousState::Observe => AutonomousState::Decide,
            AutonomousState::Decide => AutonomousState::Validate,
            AutonomousState::Validate => AutonomousState::Execute,
            AutonomousState::Execute => AutonomousState::Verify,
            AutonomousState::Verify => AutonomousState::Adapt,
            AutonomousState::Adapt => {
                self.consecutive_failures = 0;
                AutonomousState::Observe
            }
            AutonomousState::SafeMode | AutonomousState::Halted => return None,
        };

        self.state = next;
        Some(AutonomousTransition {
            from: previous,
            to: next,
            reason: "success".to_string(),
            cycle_id: self.cycle_id,
        })
    }

    pub fn transition_failure(&mut self, reason: &str) -> AutonomousTransition {
        self.consecutive_failures += 1;
        let from = self.state;

        if self.consecutive_failures >= self.config.max_failures_before_halt {
            self.state = AutonomousState::Halted;
            return AutonomousTransition {
                from,
                to: AutonomousState::Halted,
                reason: reason.to_string(),
                cycle_id: self.cycle_id,
            };
        }

        if self.consecutive_failures >= self.config.max_failures_before_safe_mode {
            self.state = AutonomousState::SafeMode;
            self.safe_mode_until_cycle = Some(self.cycle_id + self.config.safe_mode_cooldown_cycles);
            return AutonomousTransition {
                from,
                to: AutonomousState::SafeMode,
                reason: reason.to_string(),
                cycle_id: self.cycle_id,
            };
        }

        self.state = AutonomousState::Observe;
        AutonomousTransition {
            from,
            to: AutonomousState::Observe,
            reason: reason.to_string(),
            cycle_id: self.cycle_id,
        }
    }

    pub fn can_trade(&self) -> bool {
        self.state != AutonomousState::SafeMode && self.state != AutonomousState::Halted
    }

    /// Risk gate gibi dış bir sistemin doğrudan SafeMode tetiklemesi için.
    /// `cooldown_cycles`: kaç cycle sonra SafeMode'dan çıkılsın (0 = config default).
    pub fn force_safe_mode(&mut self, cooldown_cycles: u64) {
        self.state = AutonomousState::SafeMode;
        let cycles = if cooldown_cycles == 0 {
            self.config.safe_mode_cooldown_cycles
        } else {
            cooldown_cycles
        };
        self.safe_mode_until_cycle = Some(self.cycle_id + cycles);
    }

    /// Watchdog tarafından çağrılır: SafeMode veya Halted'dan Observe'e zorla çıkar.
    /// Hata sayacını sıfırlar — sistem yeniden denemeye başlar.
    pub fn force_observe(&mut self) {
        self.state = AutonomousState::Observe;
        self.safe_mode_until_cycle = None;
        self.consecutive_failures = 0;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RiskGatePolicy {
    pub max_notional_usd: f64,
    pub max_daily_loss_pct: f64,
    pub max_drawdown_pct: f64,
    pub min_model_confidence: f64,
}

impl Default for RiskGatePolicy {
    fn default() -> Self {
        Self {
            max_notional_usd: 5_000.0,
            max_daily_loss_pct: 3.0,
            max_drawdown_pct: 10.0,
            min_model_confidence: 0.55,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RiskInput {
    pub account_equity: f64,
    pub day_start_equity: f64,
    pub peak_equity: f64,
    pub requested_notional_usd: f64,
    pub model_confidence: f64,
}

#[derive(Debug, Clone)]
pub enum RiskDecision {
    Allow,
    Deny {
        reasons: Vec<String>,
        enter_safe_mode: bool,
        halt: bool,
    },
}

#[derive(Debug, Clone)]
pub struct RiskGate {
    pub policy: RiskGatePolicy,
}

impl RiskGate {
    pub fn new(policy: RiskGatePolicy) -> Self {
        Self { policy }
    }

    pub fn evaluate(&self, input: RiskInput) -> RiskDecision {
        let mut reasons = Vec::new();
        let mut enter_safe_mode = false;
        let mut halt = false;

        if input.requested_notional_usd > self.policy.max_notional_usd {
            reasons.push(format!(
                "Notional limiti aşıldı: {:.2} > {:.2}",
                input.requested_notional_usd, self.policy.max_notional_usd
            ));
        }

        if input.model_confidence < self.policy.min_model_confidence {
            reasons.push(format!(
                "Model confidence düşük: {:.3} < {:.3}",
                input.model_confidence, self.policy.min_model_confidence
            ));
        }

        let daily_loss_pct = if input.day_start_equity > 0.0 {
            ((input.day_start_equity - input.account_equity) / input.day_start_equity) * 100.0
        } else {
            0.0
        };

        if daily_loss_pct >= self.policy.max_daily_loss_pct {
            reasons.push(format!(
                "Günlük kayıp limiti aşıldı: {:.2}% >= {:.2}%",
                daily_loss_pct, self.policy.max_daily_loss_pct
            ));
            enter_safe_mode = true;
        }

        let drawdown_pct = if input.peak_equity > 0.0 {
            ((input.peak_equity - input.account_equity) / input.peak_equity) * 100.0
        } else {
            0.0
        };

        if drawdown_pct >= self.policy.max_drawdown_pct {
            reasons.push(format!(
                "Max drawdown limiti aşıldı: {:.2}% >= {:.2}%",
                drawdown_pct, self.policy.max_drawdown_pct
            ));
            halt = true;
        }

        if reasons.is_empty() {
            RiskDecision::Allow
        } else {
            RiskDecision::Deny {
                reasons,
                enter_safe_mode,
                halt,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutonomousRecoveryAction {
    Retry,
    EnterSafeMode,
    Halt,
}

#[derive(Debug, Clone)]
pub struct RecoverySupervisor {
    pub retry_limit: usize,
    pub safe_mode_threshold: usize,
    pub halt_threshold: usize,
}

impl Default for RecoverySupervisor {
    fn default() -> Self {
        Self {
            retry_limit: 2,
            safe_mode_threshold: 3,
            halt_threshold: 7,
        }
    }
}

impl RecoverySupervisor {
    pub fn next_action(&self, failure_count: usize) -> AutonomousRecoveryAction {
        if failure_count >= self.halt_threshold {
            AutonomousRecoveryAction::Halt
        } else if failure_count >= self.safe_mode_threshold {
            AutonomousRecoveryAction::EnterSafeMode
        } else {
            let _ = self.retry_limit;
            AutonomousRecoveryAction::Retry
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_gate_allows_when_within_limits() {
        let gate = RiskGate::new(RiskGatePolicy::default());
        let input = RiskInput {
            account_equity: 10_000.0,
            day_start_equity: 10_100.0,
            peak_equity: 10_300.0,
            requested_notional_usd: 1_000.0,
            model_confidence: 0.85,
        };

        assert!(matches!(gate.evaluate(input), RiskDecision::Allow));
    }

    #[test]
    fn risk_gate_halts_on_drawdown_breach() {
        let gate = RiskGate::new(RiskGatePolicy {
            max_drawdown_pct: 10.0,
            ..RiskGatePolicy::default()
        });

        let input = RiskInput {
            account_equity: 8_000.0,
            day_start_equity: 9_200.0,
            peak_equity: 10_000.0,
            requested_notional_usd: 1_500.0,
            model_confidence: 0.90,
        };

        let decision = gate.evaluate(input);
        match decision {
            RiskDecision::Deny { halt, .. } => assert!(halt),
            _ => panic!("Risk gate deny bekleniyordu"),
        }
    }

    #[test]
    fn controller_moves_to_safe_mode_and_back() {
        let mut controller = AutonomousController::new(AutonomousConfig {
            max_failures_before_safe_mode: 2,
            max_failures_before_halt: 5,
            safe_mode_cooldown_cycles: 2,
        });

        controller.begin_cycle();
        controller.transition_failure("network timeout");
        assert_eq!(controller.state, AutonomousState::Observe);

        controller.begin_cycle();
        controller.transition_failure("exchange timeout");
        assert_eq!(controller.state, AutonomousState::SafeMode);
        assert!(!controller.can_trade());

        controller.begin_cycle();
        assert_eq!(controller.state, AutonomousState::SafeMode);
        controller.begin_cycle();
        assert_eq!(controller.state, AutonomousState::Observe);
        assert!(controller.can_trade());
    }
}