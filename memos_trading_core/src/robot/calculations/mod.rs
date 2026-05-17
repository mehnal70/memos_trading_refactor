// src/robot/calculations/mod.rs - Hesaplama ve Gösterge Koordinasyon Merkezi
// Srivastava ATP - Tümden Gelim Tüzüğü (Sıfır Lojik / Sadece Deklarasyon)

pub mod orchestrator; // Tüm matematik ve analiz motorlarını koordine eden asıl işçi odası

// Kütüphane genelinde eski yolların kırılmaması için re-export mühürleri
pub use orchestrator::{CalculationEngine, CalculationEngineAdapter};
