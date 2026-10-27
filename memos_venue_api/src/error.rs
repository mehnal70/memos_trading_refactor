//! Venue katmanının yalın hata tipi.
//!
//! NEDEN AYRI: çekirdeğin `MemosTradingError`'ı `rusqlite` ve `reqwest`'e `#[from]` ile
//! bağlı. O tip buraya taşınsaydı her piyasa crate'i transitif olarak bundled SQLite'a
//! bağımlı olurdu → izolasyon hedefi çökerdi. Bu yüzden contract yalın bir hata tipi
//! tanımlar; çekirdek `From<VenueError> for MemosTradingError` ile köprüler.
//!
//! ⚠️ `is_exchange_block()` CANLI DEVRE-KESİCİYİ tetikler (`positions.rs` → `trip_live_block`).
//! Köprüde `ApiBlocked → ApiBlocked` eşlemesi KORUNMAK ZORUNDA; testle mühürlendi.

#[derive(Debug, thiserror::Error)]
pub enum VenueError {
    #[error("API Bağlantı Hatası: {0}")]
    Api(String),

    /// Borsa bizi SİSTEMİK reddetti: bölge/legal bloğu (451), WAF (403), geçersiz
    /// key-IP-izin (-2015), IP auto-ban/rate-limit (-1003/429/418). Normal emir
    /// reddinden AYRI — devre-kesici tetikleyicisi.
    #[error("Borsa Bloğu (bölge/IP/izin): {0}")]
    ApiBlocked(String),

    #[error("Veri Çözümleme (Serde) Hatası: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("Giriş/Çıkış (IO) Hatası: {0}")]
    Io(#[from] std::io::Error),

    /// Ağ katmanı hatası. `reqwest::Error` BİLEREK sarılmaz (bağımlılık contract'a
    /// girmesin) — piyasa crate'i sınırda string'e indirir.
    #[error("Ağ Hatası: {0}")]
    Network(String),

    #[error("Konfigürasyon Hatası: {0}")]
    Config(String),

    /// Veri-only venue'da emir yürütme istendi vb.
    #[error("Desteklenmeyen işlem: {0}")]
    Unsupported(String),

    #[error("Bilinmeyen Hata: {0}")]
    Unknown(String),
}

impl VenueError {
    /// Borsa SİSTEMİK reddi mi → canlı emir denemesi cooldown boyunca durmalı.
    pub fn is_exchange_block(&self) -> bool {
        matches!(self, Self::ApiBlocked(_))
    }
}

impl From<String> for VenueError {
    fn from(s: String) -> Self {
        Self::Unknown(s)
    }
}

impl From<&str> for VenueError {
    fn from(s: &str) -> Self {
        Self::Unknown(s.to_string())
    }
}

pub type Result<T> = std::result::Result<T, VenueError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_api_blocked_trips_circuit_breaker() {
        assert!(VenueError::ApiBlocked("451".into()).is_exchange_block());
        assert!(!VenueError::Api("bakiye yetersiz".into()).is_exchange_block());
        assert!(!VenueError::Network("timeout".into()).is_exchange_block());
        assert!(!VenueError::Unsupported("veri-only".into()).is_exchange_block());
    }
}
