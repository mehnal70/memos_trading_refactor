// robot/data_pipeline/canon.rs — Otonom karar-icra hattının kanonik fazları.
//
// Engine'in `process_symbol_cycle` akışı ve periyodik task'lar bu 7 aşamayı
// `PipelineStatus.mark_stage_completed` ile sırasıyla işaretler. TUI Pipeline
// timeline'ı (tuş 8) bu sayede ad-hoc step listesi yerine kanonik bir akış
// gösterir; her aşamanın son çalışma yaşı + son durumu izlenebilir.
//
// Faz 1'in sonraki commit'lerinde Execute/Learn/Optimize bağlantıları
// genişletilecek; bu dosya tek değiştirme noktasıdır — yeni faz eklenirse
// hem enum hem `ALL` hem `label()` aynı yerde güncellenir.

use serde::{Deserialize, Serialize};

/// Pipeline'ın yedi kanonik fazı — sırayla, deterministik label'larla.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PipelineStage {
    /// Mum/fiyat akışı: SQLite okuma, WS price update, normalize.
    DataIngest,
    /// İndikatör ve özellik hesabı (ATR, Supertrend, S/R, ML feature vector).
    FeatureExtract,
    /// Strateji seçimi + sinyal üretimi (StrategySelector, generate_signal).
    StrategyEval,
    /// Risk kapısı: RiskManager (Guardrails + Kelly + VaR + Gate) onayı.
    RiskGate,
    /// Pozisyon icra: paper open/close veya live executor dispatch.
    Execute,
    /// Sonuçtan öğrenme: IntelligenceHub.learn_from_exit, controller geri besleme.
    Learn,
    /// Periyodik optimizasyon: HyperOpt, walk-forward backtest, parametre güncelleme.
    Optimize,
}

impl PipelineStage {
    /// Tüm aşamalar sırayla — UI ve init için kanonik referans.
    pub const ALL: &'static [PipelineStage] = &[
        PipelineStage::DataIngest,
        PipelineStage::FeatureExtract,
        PipelineStage::StrategyEval,
        PipelineStage::RiskGate,
        PipelineStage::Execute,
        PipelineStage::Learn,
        PipelineStage::Optimize,
    ];

    /// TUI/log için stabil, numaralı insan-okur etiket. Sıralı olarak yazılır
    /// ki timeline doğal sırada görünsün ("1. Veri Akışı", "2. Özellik...").
    pub fn label(self) -> &'static str {
        match self {
            PipelineStage::DataIngest     => "1. Veri Akışı",
            PipelineStage::FeatureExtract => "2. Özellik Çıkarımı",
            PipelineStage::StrategyEval   => "3. Strateji & Sinyal",
            PipelineStage::RiskGate       => "4. Risk Kapısı",
            PipelineStage::Execute        => "5. İcra",
            PipelineStage::Learn          => "6. Öğrenme",
            PipelineStage::Optimize       => "7. Optimizasyon",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robot::data_pipeline::{PipelineStatus, StepStatus};

    #[test]
    fn all_stages_are_seven_in_canonical_order() {
        assert_eq!(PipelineStage::ALL.len(), 7);
        assert_eq!(PipelineStage::ALL[0], PipelineStage::DataIngest);
        assert_eq!(PipelineStage::ALL[6], PipelineStage::Optimize);
    }

    #[test]
    fn labels_start_with_numeric_prefix() {
        for (i, stage) in PipelineStage::ALL.iter().enumerate() {
            let expected_prefix = format!("{}.", i + 1);
            assert!(stage.label().starts_with(&expected_prefix),
                "stage {:?} etiketi {} ile başlamalı, gerçek: {}",
                stage, expected_prefix, stage.label());
        }
    }

    #[test]
    fn init_canon_stages_seeds_seven_idle_entries_in_order() {
        let mut pipe = PipelineStatus::new();
        pipe.init_canon_stages();
        assert_eq!(pipe.chain_steps.len(), 7);
        for (i, step) in pipe.chain_steps.iter().enumerate() {
            assert_eq!(step.label, PipelineStage::ALL[i].label());
            assert_eq!(step.status, StepStatus::Idle);
        }
    }

    #[test]
    fn mark_stage_completed_updates_existing_entry_in_place() {
        let mut pipe = PipelineStatus::new();
        pipe.init_canon_stages();
        let before_len = pipe.chain_steps.len();
        pipe.mark_stage_completed(PipelineStage::DataIngest, StepStatus::Done);
        // Yeni satır eklenmedi, var olan güncellendi.
        assert_eq!(pipe.chain_steps.len(), before_len);
        let ingest = pipe.chain_steps.iter()
            .find(|s| s.label == PipelineStage::DataIngest.label())
            .expect("DataIngest etiketi olmalı");
        assert_eq!(ingest.status, StepStatus::Done);
        assert!(ingest.last_run_secs > 0, "last_run_secs şimdiye eşitlenmeli");
    }

    #[test]
    fn mark_stage_supports_failed_and_skipped() {
        let mut pipe = PipelineStatus::new();
        pipe.init_canon_stages();
        pipe.mark_stage_completed(PipelineStage::StrategyEval, StepStatus::Failed);
        pipe.mark_stage_completed(PipelineStage::RiskGate,     StepStatus::Skipped);

        let eval = pipe.chain_steps.iter()
            .find(|s| s.label == PipelineStage::StrategyEval.label()).unwrap();
        let risk = pipe.chain_steps.iter()
            .find(|s| s.label == PipelineStage::RiskGate.label()).unwrap();
        assert_eq!(eval.status, StepStatus::Failed);
        assert_eq!(risk.status, StepStatus::Skipped);
    }
}
