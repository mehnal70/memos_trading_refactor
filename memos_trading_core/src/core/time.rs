//! Alt-katman (core) zaman okuyucuları — tek-nokta.
//!
//! `SystemTime::now().duration_since(UNIX_EPOCH).map(...).unwrap_or(0)` boilerplate'i
//! taban boyunca ~20+ yerde tekrar ediyordu. Bu modül onu DRY'lar
//! ([[project_modernization_roadmap]] Faz 1 DRY maddesi). Per-call (cache yok) →
//! env/saat mutasyonlu testlerle uyumlu.
//!
//! NOT: Saat UNIX epoch'tan ÖNCEYE giderse (gerçekte imkânsız) `duration_since`
//! `Err` döner; bu helper'lar 0'a düşer. Bu davranışı kabul EDEMEYEN kritik yollar
//! (ör. Binance imzalı istek timestamp'i, RNG seed'i) bilinçle kendi idiomunu tutar.

use std::time::{SystemTime, UNIX_EPOCH};

/// UNIX epoch'tan beri tam saniye; saat hatasında 0.
pub fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// UNIX epoch'tan beri milisaniye; saat hatasında 0.
pub fn now_epoch_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_secs_is_plausible_and_monotonic_nondecreasing() {
        // 2026-01-01 ≈ 1_767_225_600; üst sınırı geniş tut (sadece "makul" kontrolü).
        let s = now_epoch_secs();
        assert!(s > 1_700_000_000, "epoch saniyesi makul aralıkta değil: {s}");
        let s2 = now_epoch_secs();
        assert!(s2 >= s, "saniye geriye gitti: {s} → {s2}");
    }

    #[test]
    fn epoch_millis_consistent_with_secs() {
        let ms = now_epoch_millis();
        let s = now_epoch_secs();
        // ms/1000 ile s arasında en fazla birkaç saniye fark olmalı (yarış payı).
        let ms_secs = (ms / 1000) as u64;
        assert!(ms_secs.abs_diff(s) <= 2, "millis vs secs tutarsız: {ms_secs} vs {s}");
    }
}
