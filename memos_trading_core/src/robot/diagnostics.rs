// robot/diagnostics.rs — Faz 4 c4: Plug-in keşfedilebilirlik raporu.
//
// Faz 4 boyunca üç plug-in ekseni tanımlandı:
//   - Strategy registry      (c2: robot/strategies/registry.rs)
//   - Risk filter chain      (c1: robot/risk/filter.rs)
//   - Execution policy chain (c3: robot/execution/policy.rs)
//
// Çalışma anında "hangi plug-in'ler kayıtlı / aktif?" sorusu üç farklı modülde
// dolaşmadan cevaplanabilsin diye `PluginRegistry::snapshot()` üçünü tek
// noktada toplar. Snapshot saf veri; çağıran kendi formatına çevirebilir
// (loglar, UI, telegram) — varsayılan formatlayıcı `report()`'tur.
//
// Bu modül yan etki içermez, plug-in instance'larını yaratmaz (yalnız
// adlarını okur), `default_registry()` / `default_chain()`'i çağırdığı için
// engine state'ine bağımlı değildir → test edilebilir + ucuzdur.

use crate::robot::execution::default_chain as default_execution_chain;
use crate::robot::risk::filter::default_chain as default_risk_chain;
use crate::robot::strategies::default_registry as default_strategy_registry;

/// Üç plug-in eksenindeki kayıtlı adların donmuş anlık görüntüsü.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRegistry {
    /// StrategyRegistry'deki canonical isim listesi (sıralı, alias dahil).
    pub strategies: Vec<String>,
    /// RiskManager default zincirindeki filter adları (sıralı, chain sırası korunur).
    pub risk_filters: Vec<String>,
    /// RoboticTradeExecutor default zincirindeki policy adları (sıralı, chain sırası korunur).
    pub execution_policies: Vec<String>,
}

impl PluginRegistry {
    /// Default plug-in setinden snapshot al. Yalnız ad okur → instance yaratmanın
    /// yan etkisi olmaz (factory closure'ları çağrılmaz).
    pub fn snapshot() -> Self {
        let strategies = default_strategy_registry().names();
        let risk_filters = default_risk_chain()
            .iter()
            .map(|f| f.name().to_string())
            .collect();
        let execution_policies = default_execution_chain()
            .iter()
            .map(|p| p.name().to_string())
            .collect();
        Self { strategies, risk_filters, execution_policies }
    }

    /// İnsan-okunabilir çok-satırlı rapor. Loglara, dashboard'a, telegram
    /// alert'ine yapıştırılabilir.
    pub fn report(&self) -> String {
        let mut out = String::new();
        out.push_str("=== Plug-in Registry ===\n");
        out.push_str(&format!("Stratejiler ({}):\n", self.strategies.len()));
        for n in &self.strategies {
            out.push_str(&format!("  • {n}\n"));
        }
        out.push_str(&format!("Risk filtreleri ({}, chain sırası):\n", self.risk_filters.len()));
        for (i, n) in self.risk_filters.iter().enumerate() {
            out.push_str(&format!("  {}. {n}\n", i + 1));
        }
        out.push_str(&format!("Yürütme policy'leri ({}, chain sırası):\n", self.execution_policies.len()));
        for (i, n) in self.execution_policies.iter().enumerate() {
            out.push_str(&format!("  {}. {n}\n", i + 1));
        }
        out
    }

    /// Hızlı bütünlük kontrolü: üç eksen de en az bir kayda sahip mi?
    /// Boş çıkarsa engine yanlış init edilmiş demektir → çağıran erken
    /// fail edebilsin diye bool döner.
    pub fn is_healthy(&self) -> bool {
        !self.strategies.is_empty()
            && !self.risk_filters.is_empty()
            && !self.execution_policies.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_collects_three_axes_from_defaults() {
        let snap = PluginRegistry::snapshot();
        // Faz 4 c1 default risk chain: 3 filter (risk_gate / kelly_edge / value_at_risk).
        assert_eq!(snap.risk_filters,
            vec!["risk_gate", "kelly_edge", "value_at_risk"]);
        // Faz 4 c3 default execution chain: market_hours / idle_strategy / basket_empty.
        assert_eq!(snap.execution_policies,
            vec!["market_hours", "idle_strategy", "basket_empty"]);
        // c2 default strategy registry en az 15 girdi (alias dahil).
        assert!(snap.strategies.len() >= 15,
            "registry'de en az 15 kayıt beklenir, gelen: {}", snap.strategies.len());
    }

    #[test]
    fn snapshot_strategies_are_sorted() {
        let snap = PluginRegistry::snapshot();
        let mut sorted = snap.strategies.clone();
        sorted.sort();
        assert_eq!(snap.strategies, sorted);
    }

    #[test]
    fn is_healthy_when_all_axes_present() {
        let snap = PluginRegistry::snapshot();
        assert!(snap.is_healthy());
    }

    #[test]
    fn is_healthy_false_when_any_axis_empty() {
        let mut snap = PluginRegistry::snapshot();
        snap.execution_policies.clear();
        assert!(!snap.is_healthy());
    }

    #[test]
    fn report_includes_all_axis_headers() {
        let snap = PluginRegistry::snapshot();
        let r = snap.report();
        assert!(r.contains("Stratejiler"));
        assert!(r.contains("Risk filtreleri"));
        assert!(r.contains("Yürütme policy"));
        // Chain sırası numaralı olmalı.
        assert!(r.contains("1. market_hours"));
        assert!(r.contains("1. risk_gate"));
    }
}
