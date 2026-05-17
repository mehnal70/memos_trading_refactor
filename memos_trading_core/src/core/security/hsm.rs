// hsm.rs - Donanım Güvenlik Modülü (HSM) Entegrasyonu

use std::sync::{Mutex, OnceLock};
use pkcs11::types::*;
use pkcs11::*;

// Modern Rust: Lazy yerine standart kütüphane OnceLock kullanımı
static HSM_INSTANCE: OnceLock<Mutex<Option<HsmContext>>> = OnceLock::new();

pub struct HsmContext {
    pub pkcs11: Ctx,
    pub session: CK_SESSION_HANDLE,
}

impl HsmContext {
    /// Global HSM örneğine güvenli erişim sağlayan dahili yardımcı
    fn global() -> &'static Mutex<Option<HsmContext>> {
        HSM_INSTANCE.get_or_init(|| Mutex::new(None))
    }

    /// HSM modülünü ve oturumu başlatır (Fail-safe initialization)
    pub fn init(module_path: &str, pin: &str) -> Result<(), String> {
        let pkcs11 = Ctx::new(module_path).map_err(|e| format!("PKCS11 yüklenemedi: {e}"))?;
        
        pkcs11.initialize(Some(CK_C_INITIALIZE_ARGS::default()))
            .map_err(|e| format!("PKCS11 init hata: {e:?}"))?;
            
        let slots = pkcs11.get_slot_list(true).map_err(|e| format!("Slot list hata: {e:?}"))?;
        let &slot = slots.first().ok_or("HSM slot bulunamadı")?;
        
        let session = pkcs11.open_session(slot, CKF_SERIAL_SESSION | CKF_RW_SESSION, None, None)
            .map_err(|e| format!("Session açılamadı: {e:?}"))?;
            
        pkcs11.login(session, CKU_USER, Some(pin)).map_err(|e| format!("Login hata: {e:?}"))?;
        
        let mut guard = Self::global().lock().map_err(|_| "HSM Mutex kilitlenme hatası")?;
        *guard = Some(HsmContext { pkcs11, session });
        
        Ok(())
    }

    /// HSM üzerinde donanımsal AES anahtarı oluşturur
    pub fn generate_aes_key(label: &str, bits: u64) -> Result<CK_OBJECT_HANDLE, String> {
        let guard = Self::global().lock().map_err(|_| "Mutex error")?;
        let ctx = guard.as_ref().ok_or("HSM başlatılmadı")?;

        // Atribütler: Bellek dostu vec allocation
        let key_template = vec![
            CK_ATTRIBUTE::new(CKA_LABEL).with_string(label),
            CK_ATTRIBUTE::new(CKA_CLASS).with_ck_ulong(&CKO_SECRET_KEY),
            CK_ATTRIBUTE::new(CKA_KEY_TYPE).with_ck_ulong(&CKK_AES),
            CK_ATTRIBUTE::new(CKA_VALUE_LEN).with_ck_ulong(&(bits / 8)),
            CK_ATTRIBUTE::new(CKA_ENCRYPT).with_bool(&true),
            CK_ATTRIBUTE::new(CKA_DECRYPT).with_bool(&true),
            CK_ATTRIBUTE::new(CKA_TOKEN).with_bool(&true),
        ];

        let mut mech = CK_MECHANISM { mechanism: CKM_AES_KEY_GEN, pParameter: std::ptr::null_mut(), ulParameterLen: 0 };
        
        ctx.pkcs11.generate_key(ctx.session, &mut mech, &key_template)
            .map_err(|e| format!("AES anahtar üretilemedi: {e:?}"))
    }

    /// HSM donanımı üzerinden veri şifreleme (Zero-copy payload)
    pub fn encrypt_with_key(key: CK_OBJECT_HANDLE, data: &[u8]) -> Result<Vec<u8>, String> {
        let guard = Self::global().lock().map_err(|_| "Mutex error")?;
        let ctx = guard.as_ref().ok_or("HSM başlatılmadı")?;

        let mut mech = CK_MECHANISM { mechanism: CKM_AES_ECB, pParameter: std::ptr::null_mut(), ulParameterLen: 0 };

        ctx.pkcs11.encrypt_init(ctx.session, &mut mech, key)
            .map_err(|e| format!("Encrypt init hata: {e:?}"))?;
            
        ctx.pkcs11.encrypt(ctx.session, data).map_err(|e| format!("Şifreleme hata: {e:?}"))
    }

    /// HSM donanımı üzerinden veri çözme
    pub fn decrypt_with_key(key: CK_OBJECT_HANDLE, data: &[u8]) -> Result<Vec<u8>, String> {
        let guard = Self::global().lock().map_err(|_| "Mutex error")?;
        let ctx = guard.as_ref().ok_or("HSM başlatılmadı")?;

        let mut mech = CK_MECHANISM { mechanism: CKM_AES_ECB, pParameter: std::ptr::null_mut(), ulParameterLen: 0 };

        ctx.pkcs11.decrypt_init(ctx.session, &mut mech, key)
            .map_err(|e| format!("Decrypt init hata: {e:?}"))?;
            
        ctx.pkcs11.decrypt(ctx.session, data).map_err(|e| format!("Çözme hata: {e:?}"))
    }
}
