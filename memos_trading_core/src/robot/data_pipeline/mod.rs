// src/robot/data_pipeline/mod.rs - Veri Hattı Garnizon Kapısı
// Srivastava ATP - Tümden Gelim Tüzüğü (Sıfır Lojik / Sadece Deklarasyon)

pub mod cache; // Önbellek katmanı (CandleCache)
pub mod canon; // 7 kanonik faz: ingest → extract → eval → risk → execute → learn → optimize
pub mod cleaner; // Ham veri temizleme motoru (DataCleaner)
pub mod health;
pub mod htf_loader; // Multi-TF (Faz B): HTF mum yükleyici (DB + 1m→HTF fallback)
pub mod normalizer; // Standart forma sokma motoru (DataNormalizer)
pub mod orchestrator; // Merkezi veri orkestratörü (Asıl İşçi Motor)
pub mod status; // Pipeline çalışma zamanı durumu (chain_steps + anomalies)
pub mod synth; // Üst zaman dilimi sentez motoru (CandleSynth)
pub mod validator; // Doğruluk onay muhafızı (DataValidator) // Faz 3: candle veri-sağlık kapısı (CandleHealth + HealthThresholds)

// Kütüphane geneline (prelude / lib.rs) kolay erişim için re-export mühürleri
pub use cache::{CandleCache, MemoryOpportunitySnapshot};
pub use canon::PipelineStage;
pub use cleaner::DataCleaner;
pub use health::{CandleHealth, HealthThresholds};
pub use htf_loader::{
    aggregate_1m_to, aggregate_to, load_htf_candles, load_htf_candles_ns,
    load_htf_candles_ns_for_profile, HTF_MIN_REQUIRED,
};
pub use normalizer::DataNormalizer;
pub use orchestrator::DataPipeline;
pub use status::{
    AnomalyKind, AnomalySeverity, PipelineAnomalyRuntime, PipelineStatus, PipelineStepRuntime,
    StepStatus,
};
pub use synth::CandleSynth;
pub use validator::DataValidator;
