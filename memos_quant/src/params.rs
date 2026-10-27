//! Strateji parametre uzayı — allocation-free `Copy` gösterim (grid search kritik).
//!
//! `memos_trading_core/src/core/types.rs`'ten BİREBİR taşındı (Adım 3). Çekirdek
//! `core::types::StrategyParams` yolunu `pub use` ile korur → çağrı yerleri değişmedi.

use serde::{Deserialize, Serialize};

/// Tek bir parametre adının inline (heap'siz) gösterimi. `Copy` + ömürsüz olması
/// `StrategyParams`'ın allocation-free `Copy` kalmasını sağlar (grid search kritik).
/// Param adları kısa literaller (`signal_period`=13, `funding_threshold`=17 …);
/// 24 bayt fazlasıyla yeter. Daha uzunu (olmaması beklenir) kırpılır.
#[derive(Debug, Clone, Copy)]
struct ParamName {
    bytes: [u8; 24],
    len: u8,
}

impl ParamName {
    fn from_str(s: &str) -> Self {
        let mut bytes = [0u8; 24];
        let n = s.len().min(24);
        bytes[..n].copy_from_slice(&s.as_bytes()[..n]);
        Self {
            bytes,
            len: n as u8,
        }
    }
    #[inline]
    fn matches(&self, key: &str) -> bool {
        let n = self.len as usize;
        key.len() == n && &self.bytes[..n] == key.as_bytes()
    }
    #[inline]
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

/// Strateji parametreleri — **açık-kelime-dağarcıklı, `Copy`, allocation-free inline KV-torba**.
///
/// Eski sabit 10-alanlı "god bag" yerine: her strateji KENDİ parametre uzayını
/// `Strategy::param_spec()` ile bildirir, değerler buraya ADIYLA yazılır/okunur
/// (`set`/`get`). Yeni bir eşik/periyot açmak için yalnız (a) `strategies::keys`'e bir
/// sabit ve (b) stratejide bir `f64_or/usize_or` okuması gerekir — struct alanı + match
/// kolu + serde üçlemesi biter. Anahtarlar `strategies::keys` modülünde tek-kaynak.
///
/// `CAP=16`: mevcut en geniş strateji ~4 param bildirir; config/store override'ları için
/// bol pay. Dolarsa `set` sessizce (warn ile) yok sayar — panik yok.
#[derive(Debug, Clone, Copy)]
pub struct StrategyParams {
    names: [ParamName; Self::CAP],
    vals: [f64; Self::CAP],
    len: usize,
}

impl StrategyParams {
    pub const CAP: usize = 16;

    #[inline]
    pub const fn new() -> Self {
        Self {
            names: [ParamName {
                bytes: [0u8; 24],
                len: 0,
            }; Self::CAP],
            vals: [0.0; Self::CAP],
            len: 0,
        }
    }

    /// Anahtarın ham `f64` değeri; yoksa `None`.
    #[inline]
    pub fn get(&self, key: &str) -> Option<f64> {
        self.names[..self.len]
            .iter()
            .position(|n| n.matches(key))
            .map(|i| self.vals[i])
    }

    /// Periyot/bar sayısı okuması: yoksa `default`. Ham değer `round().max(1)` ile
    /// usize'a yuvarlanır (yuvarlama politikası tek-noktada, okuma anında).
    #[inline]
    pub fn usize_or(&self, key: &str, default: usize) -> usize {
        self.get(key)
            .map(|v| v.round().max(1.0) as usize)
            .unwrap_or(default)
    }

    /// Sürekli eşik/çarpan okuması: yoksa `default`.
    #[inline]
    pub fn f64_or(&self, key: &str, default: f64) -> f64 {
        self.get(key).unwrap_or(default)
    }

    /// Anahtarı upsert eder. Kapasite dolu ve anahtar yeniyse sessizce (warn) atlar.
    pub fn set(&mut self, key: &str, val: f64) {
        if let Some(i) = self.names[..self.len].iter().position(|n| n.matches(key)) {
            self.vals[i] = val;
        } else if self.len < Self::CAP {
            self.names[self.len] = ParamName::from_str(key);
            self.vals[self.len] = val;
            self.len += 1;
        } else {
            log::warn!(
                "StrategyParams kapasitesi ({}) doldu; '{}' atlandı",
                Self::CAP,
                key
            );
        }
    }

    /// Zincirlenebilir kurucu (test/kurulum): `StrategyParams::new().with("fast", 5.0)`.
    #[inline]
    pub fn with(mut self, key: &str, val: f64) -> Self {
        self.set(key, val);
        self
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Dolu (anahtar, değer) çiftleri üzerinde yineleyici (serde/rapor için).
    pub fn iter(&self) -> impl Iterator<Item = (&str, f64)> + '_ {
        (0..self.len).map(move |i| (self.names[i].as_str(), self.vals[i]))
    }
}

impl Default for StrategyParams {
    fn default() -> Self {
        Self::new()
    }
}

// Serde: harita ({"fast": 5.0, ...}) formu. Diske persist edilip geri okunmuyor
// (ParameterStore from_env tek-kaynak), ama rapor/test için temiz round-trip.
impl Serialize for StrategyParams {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = ser.serialize_map(Some(self.len))?;
        for (k, v) in self.iter() {
            map.serialize_entry(k, &v)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for StrategyParams {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
        let m = std::collections::BTreeMap::<String, f64>::deserialize(de)?;
        let mut p = StrategyParams::new();
        for (k, v) in m {
            p.set(&k, v);
        }
        Ok(p)
    }
}
