// src/robot/data_pipeline/mod.rs - Veri Hattı Garnizon Kapısı
// Srivastava ATP - Tümden Gelim Tüzüğü (Sıfır Lojik / Sadece Deklarasyon)

pub mod cache;        // Önbellek katmanı (CandleCache)
pub mod synth;        // Üst zaman dilimi sentez motoru (CandleSynth)
pub mod cleaner;      // Ham veri temizleme motoru (DataCleaner)
pub mod normalizer;   // Standart forma sokma motoru (DataNormalizer)
pub mod validator;    // Doğruluk onay muhafızı (DataValidator)
pub mod orchestrator; // Merkezi veri orkestratörü (Asıl İşçi Motor)
pub mod status;       // Pipeline çalışma zamanı durumu (chain_steps + anomalies)

// Kütüphane geneline (prelude / lib.rs) kolay erişim için re-export mühürleri
pub use cache::CandleCache;
pub use synth::CandleSynth;
pub use cleaner::DataCleaner;
pub use normalizer::DataNormalizer;
pub use validator::DataValidator;
pub use orchestrator::DataPipeline;
pub use status::{PipelineStatus, PipelineStepRuntime, PipelineAnomalyRuntime,
                 StepStatus, AnomalySeverity, AnomalyKind};
