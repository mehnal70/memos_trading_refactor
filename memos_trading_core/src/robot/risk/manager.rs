// src/robot/risk/manager.rs - Risk plug-in zincirinin orkestratörü (Faz 4 c1)
//
// authorize() artık sert-kodlu 3-aşama (Gate → Kelly → VaR) değil:
// `Vec<Box<dyn RiskFilter>>` üzerinde dolaşıp ilk Deny'da zinciri kısa-devre eder.
// Default chain `filter::default_chain()` ile aynı 3 filtreyi sırasıyla içerir
// (geriye uyumluluk: davranış değişmedi). Yeni filtreler `push_filter` ile
// eklenebilir; tamamen özel zincirler `with_filters` ile kurulur.

use crate::prelude::*;
use super::risk_gate::RiskDecision;
use super::filter::{RiskFilter, RiskContext, default_chain};

/// Srivastava ATP - Entegre Risk Yönetim Merkezi
pub struct RiskManager {
    /// Sırayla çalışan plug-in zinciri. İlk Deny zinciri durdurur.
    pub filters: Vec<Box<dyn RiskFilter>>,
}

impl Default for RiskManager {
    fn default() -> Self { Self::new() }
}

impl RiskManager {
    /// Default chain: RiskGateFilter → KellyEdgeFilter → VarFilter.
    pub fn new() -> Self {
        Self { filters: default_chain() }
    }

    /// Tamamen özel filtre listesiyle inşa et (boş chain her şeyi Allow eder).
    pub fn with_filters(filters: Vec<Box<dyn RiskFilter>>) -> Self {
        Self { filters }
    }

    /// Mevcut zincire yeni bir plug-in ekle (sonuna). Adaptive override'lar veya
    /// rejim-bazlı filtreler için.
    pub fn push_filter(&mut self, filter: Box<dyn RiskFilter>) {
        self.filters.push(filter);
    }

    /// ⚔️ NİHAİ OTORİZASYON: Bir işlemin borsaya gidip gitmeyeceğine karar verir.
    ///
    /// Tüm filtreleri sırasıyla çalıştırır; ilk Deny döndüren filtre zinciri keser.
    /// Boş zincir => Allow.
    pub fn authorize(
        &self,
        signal: &Signal,
        snap: &MissionControl,
        edge_score: f64,
        requested_notional_usd: f64,
    ) -> RiskDecision {
        let ctx = RiskContext {
            signal,
            snap,
            edge_score,
            requested_notional_usd,
        };
        for filter in &self.filters {
            match filter.evaluate(&ctx) {
                RiskDecision::Allow => continue,
                decision @ RiskDecision::Deny { .. } => return decision,
            }
        }
        RiskDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_manager_has_three_filters() {
        let m = RiskManager::new();
        assert_eq!(m.filters.len(), 3);
        let names: Vec<&str> = m.filters.iter().map(|f| f.name()).collect();
        assert_eq!(names, vec!["risk_gate", "kelly_edge", "value_at_risk"]);
    }

    #[test]
    fn with_filters_can_build_empty_chain() {
        let m = RiskManager::with_filters(vec![]);
        assert!(m.filters.is_empty());
    }

    #[test]
    fn push_filter_appends_to_chain() {
        use super::super::filter::KellyEdgeFilter;
        let mut m = RiskManager::with_filters(vec![]);
        m.push_filter(Box::new(KellyEdgeFilter::default()));
        assert_eq!(m.filters.len(), 1);
        assert_eq!(m.filters[0].name(), "kelly_edge");
    }
}
