// rbac.rs - Rol Tabanlı Erişim Kontrolü (RBAC) Modülü

use std::collections::{HashMap, HashSet};
use std::sync::{OnceLock, RwLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] // Copy eklendi (hafif enum)
pub enum Role {
    Admin,
    Trader,
    Viewer,
}

// Modern Rust: once_cell yerine yerleşik OnceLock, Mutex yerine okuma hızı için RwLock
static USER_ROLES: OnceLock<RwLock<HashMap<String, HashSet<Role>>>> = OnceLock::new();

pub struct Rbac;

impl Rbac {
    /// Roller haritasına güvenli erişim sağlayan dahili yardımcı
    fn get_store() -> &'static RwLock<HashMap<String, HashSet<Role>>> {
        USER_ROLES.get_or_init(|| RwLock::new(HashMap::with_capacity(100)))
    }

    /// Kullanıcıya rol ata (Yazma işlemi - Exclusive Lock)
    pub fn assign(username: &str, role: Role) {
        if let Ok(mut map) = Self::get_store().write() {
            // to_owned() kullanarak string sahipliğini temiz yönettik
            map.entry(username.to_owned()).or_default().insert(role);
        }
    }

    /// Kullanıcıda belirli bir rol var mı? (Okuma işlemi - Shared Lock)
    pub fn has_role(username: &str, role: Role) -> bool {
        Self::get_store()
            .read()
            .map(|map| {
                map.get(username)
                    .map_or(false, |set| set.contains(&role))
            })
            .unwrap_or(false)
    }

    /// Kullanıcının tüm rollerini döndürür (Zero-allocation filtering)
    pub fn roles(username: &str) -> Vec<Role> {
        Self::get_store()
            .read()
            .map(|map| {
                map.get(username)
                    .map(|set| set.iter().copied().collect()) // Cloned yerine copied (enum Copy olduğu için)
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    }

    /// Kullanıcının bir rolünü kaldırır
    pub fn revoke(username: &str, role: Role) {
        if let Ok(mut map) = Self::get_store().write() {
            if let Some(set) = map.get_mut(username) {
                set.remove(&role);
            }
        }
    }
}
