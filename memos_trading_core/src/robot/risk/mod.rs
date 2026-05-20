// src/robot/risk/mod.rs - Risk Muhafız Garnizon Kapısı
// Srivastava ATP - Tümden Gelim Tüzüğü (Sıfır Lojik / Sadece Deklarasyon)

pub mod risk_gate;       // Sinyal onay muhafızı (DD/Daily loss/Notional/ML conf)
pub mod guardrails;      // Temel limitler (Bakiye, Kaldıraç Sınırları)
pub mod kelly;           // Kelly Criterion otonom sermaye hesaplayıcı
pub mod var;             // Value at Risk portföy maruziyet motoru
pub mod metrics;         // Performans risk metrikleri (Sharpe/Sortino/Calmar/IR/Omega)
pub mod filter;          // Faz 4 c1: plug-in risk zinciri (RiskFilter trait + 3 filtre)
pub mod manager;         // Merkezi risk yöneticisi (RiskFilter chain orkestratörü)

// Kütüphane geneline (prelude / lib.rs) kolay erişim için re-export mühürleri
pub use risk_gate::RiskGate;
pub use guardrails::Guardrails;
pub use manager::RiskManager;
pub use filter::{RiskFilter, RiskContext, RiskGateFilter, KellyEdgeFilter, VarFilter};
