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
pub mod config;

// ── Yaygın kullanılan tipleri kolay erişim için re-export ────────────────────
pub use memory::{DatabaseError, DatabaseEngine, MemoryDatabase};
pub use crate::core::model::PositionModel as PortfolioPosition;
pub use crate::core::model::{SymbolInfo, PaperTradingResult};
pub use writer::DBWriter;

/// Kanonik SQLite bağlantı açıcı — TÜM açılışlar buradan geçmeli (tek-nokta).
///
/// `Connection::open` üstüne iki ayar uygular:
/// - **WAL** (`journal_mode=WAL`): okuyucu yazıcıyı, yazıcı okuyucuyu BLOKLAMAZ →
///   download yazımı sırasında cycle okuması artık "database is locked" almaz
///   (eskiden `read_candles` çıplak `Connection::open` ile açıp kilitleniyordu).
/// - **busy_timeout (5s)**: yazıcı-yazıcı çakışmasında (snapshot + download eşzamanlı)
///   anlık `SQLITE_BUSY` yerine bekler. Eskiden yalnız BAZI sitelerde elle vardı.
///
/// Pragma'lar best-effort (başarısızsa bağlantı yine döner); yalnız `open`'ın
/// kendisi hata verirse `Err`. WAL kalıcı (DB dosyası moduna yazılır) ama her
/// açılışta yeniden set etmek idempotent ve ucuz.
pub fn open_db(path: &str) -> rusqlite::Result<rusqlite::Connection> {
    let conn = rusqlite::Connection::open(path)?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    // WAL + NORMAL senkron: eşzamanlı okuma/yazma + makul dayanıklılık (WAL best-practice).
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    let _ = conn.pragma_update(None, "synchronous", "NORMAL");
    Ok(conn)
}
