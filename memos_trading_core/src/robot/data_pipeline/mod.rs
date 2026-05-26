// src/robot/data_pipeline/mod.rs - Veri Hattı Garnizon Kapısı
// Srivastava ATP - Tümden Gelim Tüzüğü (Sıfır Lojik / Sadece Deklarasyon)

pub mod cache;        // Önbellek katmanı (CandleCache)
pub mod synth;        // Üst zaman dilimi sentez motoru (CandleSynth)
pub mod cleaner;      // Ham veri temizleme motoru (DataCleaner)
pub mod normalizer;   // Standart forma sokma motoru (DataNormalizer)
pub mod validator;    // Doğruluk onay muhafızı (DataValidator)
pub mod orchestrator; // Merkezi veri orkestratörü (Asıl İşçi Motor)
pub mod status;       // Pipeline çalışma zamanı durumu (chain_steps + anomalies)
pub mod canon;        // 7 kanonik faz: ingest → extract → eval → risk → execute → learn → optimize
pub mod htf_loader;   // Multi-TF (Faz B): HTF mum yükleyici (DB + 1m→HTF fallback)

// Kütüphane geneline (prelude / lib.rs) kolay erişim için re-export mühürleri
pub use cache::CandleCache;
pub use synth::CandleSynth;
pub use cleaner::DataCleaner;
pub use normalizer::DataNormalizer;
pub use validator::DataValidator;
pub use orchestrator::DataPipeline;
pub use status::{PipelineStatus, PipelineStepRuntime, PipelineAnomalyRuntime,
                 StepStatus, AnomalySeverity, AnomalyKind};
pub use canon::PipelineStage;
pub use htf_loader::{load_htf_candles, aggregate_1m_to, aggregate_to, HTF_MIN_REQUIRED};
