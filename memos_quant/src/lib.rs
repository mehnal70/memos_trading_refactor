//! `memos_quant` — piyasa-agnostik algoritma çekirdeği.
//!
//! NEDEN AYRI CRATE: dikey-dilim mimarisinde her piyasa kendi crate'inde yaşar, ama
//! algoritmalar (indikatör, rejim tespiti, risk matematiği) TEK KOPYA olmalı. Ölçtük:
//! strateji/pipeline/backtester/screener katmanlarında borsaya özel referans yalnız
//! 3 dosyada — yani bu kod gerçekten piyasa-agnostik ve kopyalanması felaket olurdu.
//!
//! Katman kuralı: `memos_venue_api` ← `memos_quant` ← `memos_market_*` / `memos_trading_core`.
//! Bu crate çekirdeğe ASLA bağımlı olmaz (`cargo tree` ile mühürlendi).

// Contract hata tipi kök seviyede: taşınan dosyalardaki `crate::Result<..>`
// tam-nitelikli atıfları bozulmadan çalışsın.
pub use memos_venue_api::{Result, VenueError};

pub mod indicators;
pub mod market_regime;
pub mod math;
pub mod metrics;
pub mod params;
pub mod regime;
pub mod risk;
pub mod strategies;
pub mod types;
