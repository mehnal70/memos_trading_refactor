//! Persistence katmanı — DB CRUD ve in-memory cache.
//!
//! ## Yapı
//!
//! - [`memory`] — In-memory database + anomaly detection (eski `database.rs`)
//! - [`reader`] — Read operations: candles, symbols, portfolio, paper trading sonuçları
//! - [`writer`] — Write operations: candle save, table creation, snapshot mgmt
//!
//! ## Migration Notu
//!
//! Bu modül eski `crate::database`, `crate::database_reader`, `crate::database_writer`
//! modüllerinin yerini alır. Geriye uyumluluk için `lib.rs` üzerinden bu eski isimler
//! hâlâ erişilebilir (deprecated). Yeni kod doğrudan `persistence::reader` vb. kullanmalı.
//!
//! ## Repository Pattern (gelecek)
//!
//! Şu an `reader` ve `writer` standalone fonksiyonlar olarak organize edilmiştir.
//! İlerideki bir refactor'da bu fonksiyonlar `CandleRepository`, `BacktestRepository`,
//! `PatternRepository` gibi trait'ler altında toplanacak — mock-based test yazımını
//! kolaylaştırmak için.

pub mod memory;
pub mod reader;
pub mod writer;

// ── Yaygın kullanılan tipleri kolay erişim için re-export ────────────────────
pub use memory::{DatabaseError, DatabaseEngine, MemoryDatabase};
pub use reader::{SymbolInfo, PaperTradingResult, PortfolioPosition};
pub use writer::DBWriter;
