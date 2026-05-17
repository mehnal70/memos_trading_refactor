// src/robot/engines/mod.rs - Motor Garnizon Kapısı
// Srivastava ATP - Tümden Gelim Tüzüğü (Sıfır Lojik / Sadece Deklarasyon)

pub mod base;    // Ortak motor arayüzü ve konfigürasyon veri kontratları
pub mod spot;    // Spot piyasa motoru infaz çarkları
pub mod futures; // Vadeli işlemler (Futures) motoru infaz çarkları
pub mod master;  // Otonom döngüyü ve filoları yöneten asıl işçi (Engine / Master)
pub mod executor;
pub mod binance_executor;

// Kütüphane genelinde (prelude vb.) yolların kırılmaması için re-export mühürleri
pub use base::{EngineConfig, TradingEngine};
pub use master::Engine;
