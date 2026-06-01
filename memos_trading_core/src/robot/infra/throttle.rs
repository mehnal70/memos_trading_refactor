//! Genel amaçlı, epoch-tabanlı log/olay throttle çekirdeği.
//!
//! Eskiden iki ayrı kopya vardı: `logger.rs` SIGNAL/RISK_BLOCK event'lerini
//! kendi `last_signal` map'iyle, `master/mod.rs` ise state-log'ları ayrı bir
//! `LOG_THROTTLE_MAP` ile throttle ediyordu — aynı "map kilitle → epoch
//! karşılaştır → insert" mantığının iki kopyası. Artık ikisi de bu tek
//! [`Throttle`] tipinden geçer (tek algoritma, ayrı instance'lar):
//!   - logger: her sink kendi instance'ını tutar (test izolasyonu korunur),
//!   - master: process-global tek instance (`log_throttle_should_emit`).

use std::collections::HashMap;
use std::sync::Mutex;

/// Anahtar → son emit epoch (sn). Tek başına thread-safe (içsel `Mutex`),
/// `&self` ile çağrılır → hem instance alan hem `static` olarak kullanılabilir.
#[derive(Debug, Default)]
pub struct Throttle {
    last: Mutex<HashMap<String, u64>>,
}

impl Throttle {
    pub fn new() -> Self {
        Self::default()
    }

    /// `key` için son emit'ten bu yana `cooldown_secs` geçtiyse (veya ilk kez
    /// görülüyorsa) `true` döner ve damgayı günceller; aksi halde `false` (yut).
    ///
    /// `cooldown_secs == 0` → throttle kapalı, daima `true`.
    /// Kilit poisoned ise `true` döner (log'u kaybetmektense bas — eski davranış).
    pub fn should_emit(&self, key: &str, cooldown_secs: u64) -> bool {
        if cooldown_secs == 0 {
            return true;
        }
        let now = crate::core::time::now_epoch_secs();
        let mut guard = match self.last.lock() {
            Ok(g) => g,
            Err(_) => return true,
        };
        if let Some(prev) = guard.get(key) {
            if now.saturating_sub(*prev) < cooldown_secs {
                return false;
            }
        }
        // Lookup yolu (yaygın hâl, false dönen) alloc'suz: `get(&str)`.
        // Alloc yalnız insert anında (cooldown başına ~1 kez).
        guard.insert(key.to_string(), now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ilk_emit_gecer_tekrar_yutulur() {
        let t = Throttle::new();
        assert!(t.should_emit("a|b", 60), "ilk geçiş emit etmeli");
        assert!(!t.should_emit("a|b", 60), "cooldown içinde tekrar yutulmalı");
    }

    #[test]
    fn farkli_anahtar_bagimsiz() {
        let t = Throttle::new();
        assert!(t.should_emit("a|b", 60));
        assert!(t.should_emit("a|c", 60), "farklı anahtar bağımsız geçmeli");
    }

    #[test]
    fn cooldown_sifir_kapali() {
        let t = Throttle::new();
        for _ in 0..5 {
            assert!(t.should_emit("k", 0), "cooldown=0 → daima emit");
        }
    }

    #[test]
    fn instancelar_izole() {
        let a = Throttle::new();
        let b = Throttle::new();
        assert!(a.should_emit("x", 60));
        assert!(b.should_emit("x", 60), "ayrı instance ayrı durum tutmalı");
    }
}
