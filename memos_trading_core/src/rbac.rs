// rbac.rs
// Rol tabanlı erişim kontrolü (RBAC) modülü
// Türkçe açıklamalar ile

use std::collections::{HashMap, HashSet};
use once_cell::sync::Lazy;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Admin,
    Trader,
    Viewer,
}

static USER_ROLES: Lazy<Mutex<HashMap<String, HashSet<Role>>>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub struct Rbac;

impl Rbac {
    // Kullanıcıya rol ata
    pub fn assign(username: &str, role: Role) {
        let mut map = USER_ROLES.lock().unwrap();
        map.entry(username.to_string()).or_default().insert(role);
    }
    // Kullanıcıda rol var mı?
    pub fn has_role(username: &str, role: Role) -> bool {
        USER_ROLES.lock().unwrap().get(username).map_or(false, |set| set.contains(&role))
    }
    // Kullanıcının tüm rolleri
    pub fn roles(username: &str) -> Vec<Role> {
        USER_ROLES.lock().unwrap().get(username).map_or(vec![], |set| set.iter().cloned().collect())
    }
}
