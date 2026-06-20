//! MT5 venue katmanı — MetaTrader 5 köprüsü (forex/emtia/endeks CFD).
//!
//! * [`protocol`] — satır-sınırlı JSON istek/yanıt (SAF; ağsız test edilir).
//! * [`bridge::Mt5Bridge`] — tokio TCP **server**; MT5 EA native client olarak bağlanır.
//! * [`venue::Mt5Venue`] — `VenueAdapter` impl (Faz 1: veri; yürütme = Faz 2 açık `Err`).
//!
//! Rust'ın server olmasının sebebi: MQL5'te server/listen socket yok; EA yalnız dışa
//! bağlanabilir (`SocketConnect`). Köprü DLL gerektirmez (saf MQL5 + saf tokio).

pub mod bridge;
pub mod protocol;
pub mod venue;

pub use bridge::{Mt5Bridge, MT5_DEFAULT_ADDR};
pub use venue::Mt5Venue;
