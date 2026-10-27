//! Strateji bankası — memos_quant'a taşındı (dikey-dilim refactor'ü, Adım 3).
//!
//! Gövde `memos_quant::strategies` içinde BİREBİR yaşıyor. Burada yalnız yeniden-yayın
//! kabuğu var: `crate::robot::strategies::{Strategy, RsiStrategy, base::*, keys, ...}`
//! gibi mevcut yollar (prelude dahil) AYNEN çalışır → davranış nötr.
//!
//! NOT: `funding.rs` şimdilik burada; plana göre `memos_market_binance` dikey dilimine
//! taşınacak (funding rate kripto-perp'e özgü), Adım 6.
pub use memos_quant::strategies::*;
