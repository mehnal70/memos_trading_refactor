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
    ///
    /// Görünürlük: bir filtre veto ederse Deny.reasons listesinin ilk elemanına
    /// `[filter_name]` prefix'i takılır. Sinyal→trade gap'ini operatörün
    /// trades.jsonl + robotic_trading.log üzerinden tek bakışta görmesi için.
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
                RiskDecision::Deny { reasons, enter_safe_mode, halt } => {
                    let tagged = Self::tag_reasons(filter.name(), reasons);
                    return RiskDecision::Deny {
                        reasons: tagged,
                        enter_safe_mode,
                        halt,
                    };
                }
            }
        }
        RiskDecision::Allow
    }

    /// Deny gerekçelerine veto eden filtrenin adını prefix olarak ekler.
    /// `[risk_gate] Kritik Max DD aşıldı: 11.2%` gibi.
    fn tag_reasons(filter_name: &str, reasons: Vec<String>) -> Vec<String> {
        if reasons.is_empty() {
            return vec![format!("[{}] (gerekçe yok)", filter_name)];
        }
        reasons
            .into_iter()
            .enumerate()
            .map(|(i, r)| {
                if i == 0 {
                    format!("[{}] {}", filter_name, r)
                } else {
                    r
                }
            })
            .collect()
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

    #[test]
    fn tag_reasons_prefixes_only_first_entry() {
        let reasons = vec![
            "Kritik Max DD aşıldı: 11.2%".to_string(),
            "Günlük kayıp sınırı ihlal edildi: 3.5%".to_string(),
        ];
        let out = RiskManager::tag_reasons("risk_gate", reasons);
        assert_eq!(out[0], "[risk_gate] Kritik Max DD aşıldı: 11.2%");
        assert_eq!(out[1], "Günlük kayıp sınırı ihlal edildi: 3.5%");
    }

    #[test]
    fn tag_reasons_handles_empty_list() {
        let out = RiskManager::tag_reasons("kelly_edge", vec![]);
        assert_eq!(out.len(), 1);
        assert!(out[0].starts_with("[kelly_edge]"));
    }

    #[test]
    fn tag_reasons_preserves_extra_lines_unprefixed() {
        let reasons = vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string(),
        ];
        let out = RiskManager::tag_reasons("value_at_risk", reasons);
        assert!(out[0].starts_with("[value_at_risk] first"));
        assert_eq!(out[1], "second");
        assert_eq!(out[2], "third");
    }
}
