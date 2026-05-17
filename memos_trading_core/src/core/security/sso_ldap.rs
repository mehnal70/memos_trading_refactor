// sso_ldap.rs
// Kurumsal Kimlik Doğrulama (SSO) ve LDAP Entegrasyon Modülü

use ldap3::{LdapConnAsync, LdapConnSettings, Scope, SearchEntry};
use serde::{Serialize, Deserialize};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LdapUser {
    pub dn: String,
    pub cn: String,
    pub mail: Option<String>,
    pub member_of: Vec<String>,
}

pub struct SsoManager {
    ldap_url: String,
    bind_dn: String,
    bind_pw: String,
}

impl SsoManager {
    pub fn new(url: &str, dn: &str, pw: &str) -> Self {
        Self {
            ldap_url: url.to_owned(),
            bind_dn: dn.to_owned(),
            bind_pw: pw.to_owned(),
        }
    }

    /// LDAP üzerinden kullanıcı kimlik doğrulaması yapar (Asenkron)
    pub async fn authenticate(&self, username: &str, password: &str) -> Result<bool, String> {
        // Pipeline sağlığı için LDAP bağlantısına timeout ekliyoruz
        let settings = LdapConnSettings::new()
            .set_conn_timeout(Duration::from_secs(5));

        let (conn, mut ldap) = LdapConnAsync::with_settings(settings, &self.ldap_url)
            .await
            .map_err(|e| format!("LDAP bağlantı hatası: {e}"))?;

        ldap3::drive!(conn);

        // Kullanıcıyı doğrula (Bind işlemi)
        // Not: Gerçek senaryoda önce teknik kullanıcı ile bind yapıp 
        // kullanıcı DN'ini aramak gerekebilir.
        let user_dn = format!("uid={},ou=users,dc=memos,dc=trading", username);
        
        match ldap.simple_bind(&user_dn, password).await {
            Ok(res) => {
                let status = res.success().map_err(|e| e.to_string())?;
                Ok(status)
            }
            Err(_) => Ok(false), // Yanlış şifre
        }
    }

    /// Kullanıcı detaylarını ve gruplarını (RBAC için) getirir
    pub async fn fetch_user_details(&self, username: &str) -> Result<Option<LdapUser>, String> {
        let (conn, mut ldap) = LdapConnAsync::new(&self.ldap_url)
            .await
            .map_err(|e| format!("LDAP erişim hatası: {e}"))?;

        ldap3::drive!(conn);

        // Teknik kullanıcı ile yetkili arama yap
        ldap.simple_bind(&self.bind_dn, &self.bind_pw).await
            .map_err(|e| format!("Admin bind hatası: {e}"))?;

        let filter = format!("(uid={})", username);
        let (rs, _res) = ldap.search(
            "ou=users,dc=memos,dc=trading",
            Scope::Subtree,
            &filter,
            vec!["cn", "mail", "memberOf"]
        ).await.map_err(|e| e.to_string())?;

        if let Some(entry) = rs.into_iter().next() {
            let entry = SearchEntry::construct(entry);
            return Ok(Some(LdapUser {
                dn: entry.dn,
                cn: entry.attrs.get("cn").and_then(|v| v.first()).cloned().unwrap_or_default(),
                mail: entry.attrs.get("mail").and_then(|v| v.first()).cloned(),
                member_of: entry.attrs.get("memberOf").cloned().unwrap_or_default(),
            }));
        }

        Ok(None)
    }
}
