// hsm.rs
// Donanım Güvenlik Modülü (HSM) entegrasyonu - Gerçek üretim seviyesinde
// PKCS#11 (SoftHSM, YubiHSM, CloudHSM) desteği ile anahtar yönetimi
// Türkçe açıklamalar ile

use std::sync::Mutex;
use pkcs11::types::*;
use pkcs11::*;
use once_cell::sync::Lazy;

// HSM oturumu ve modül handle'ı (global singleton)
static HSM_CTX: Lazy<Mutex<Option<HsmContext>>> = Lazy::new(|| Mutex::new(None));

pub struct HsmContext {
    pub pkcs11: Ctx,
    pub session: CK_SESSION_HANDLE,
}

impl HsmContext {
    // HSM modülünü başlat (ör: /usr/lib/softhsm/libsofthsm2.so)
    pub fn init(module_path: &str, pin: &str) -> Result<(), String> {
        let mut pkcs11 = Ctx::new(module_path).map_err(|e| format!("PKCS11 yüklenemedi: {e}"))?;
        pkcs11.initialize(Some(CK_C_INITIALIZE_ARGS::default())).map_err(|e| format!("PKCS11 init hata: {e:?}"))?;
        let slots = pkcs11.get_slot_list(true).map_err(|e| format!("Slot list hata: {e:?}"))?;
        let slot = *slots.first().ok_or("HSM slot bulunamadı")?;
        let session = pkcs11.open_session(slot, CKF_SERIAL_SESSION | CKF_RW_SESSION, None, None)
            .map_err(|e| format!("Session açılamadı: {e:?}"))?;
        pkcs11.login(session, CKU_USER, Some(pin)).map_err(|e| format!("Login hata: {e:?}"))?;
        let ctx = HsmContext { pkcs11, session };
        *HSM_CTX.lock().unwrap() = Some(ctx);
        Ok(())
    }

    // HSM'de anahtar oluştur (ör: AES anahtarı)
    pub fn generate_aes_key(label: &str, bits: u64) -> Result<CK_OBJECT_HANDLE, String> {
        let ctx = HSM_CTX.lock().unwrap();
        let ctx = ctx.as_ref().ok_or("HSM başlatılmadı")?;
        let key_template = vec![
            CK_ATTRIBUTE::new(CKA_LABEL).with_string(label),
            CK_ATTRIBUTE::new(CKA_CLASS).with_ck_ulong(&CKO_SECRET_KEY),
            CK_ATTRIBUTE::new(CKA_KEY_TYPE).with_ck_ulong(&CKK_AES),
            CK_ATTRIBUTE::new(CKA_VALUE_LEN).with_ck_ulong(&(bits / 8)),
            CK_ATTRIBUTE::new(CKA_ENCRYPT).with_bool(&1),
            CK_ATTRIBUTE::new(CKA_DECRYPT).with_bool(&1),
            CK_ATTRIBUTE::new(CKA_TOKEN).with_bool(&1),
        ];
        ctx.pkcs11.generate_key(ctx.session, &CK_MECHANISM { mechanism: CKM_AES_KEY_GEN, pParameter: std::ptr::null_mut(), ulParameterLen: 0 }, &key_template)
            .map_err(|e| format!("AES anahtar üretilemedi: {e:?}"))
    }

    // HSM'den anahtar ile veri şifrele
    pub fn encrypt_with_key(key: CK_OBJECT_HANDLE, data: &[u8]) -> Result<Vec<u8>, String> {
        let ctx = HSM_CTX.lock().unwrap();
        let ctx = ctx.as_ref().ok_or("HSM başlatılmadı")?;
        ctx.pkcs11.encrypt_init(ctx.session, &CK_MECHANISM { mechanism: CKM_AES_ECB, pParameter: std::ptr::null_mut(), ulParameterLen: 0 }, key)
            .map_err(|e| format!("Encrypt init hata: {e:?}"))?;
        ctx.pkcs11.encrypt(ctx.session, data).map_err(|e| format!("Şifreleme hata: {e:?}"))
    }

    // HSM'den anahtar ile veri çöz
    pub fn decrypt_with_key(key: CK_OBJECT_HANDLE, data: &[u8]) -> Result<Vec<u8>, String> {
        let ctx = HSM_CTX.lock().unwrap();
        let ctx = ctx.as_ref().ok_or("HSM başlatılmadı")?;
        ctx.pkcs11.decrypt_init(ctx.session, &CK_MECHANISM { mechanism: CKM_AES_ECB, pParameter: std::ptr::null_mut(), ulParameterLen: 0 }, key)
            .map_err(|e| format!("Decrypt init hata: {e:?}"))?;
        ctx.pkcs11.decrypt(ctx.session, data).map_err(|e| format!("Çözme hata: {e:?}"))
    }
}

// Not: Gerçek ortamda HSM modül yolu, slot, pin gibi bilgiler config dosyasından veya environment değişkeninden alınmalı.
// API anahtarları ve kritik şifreler HSM'de oluşturulmalı ve sadece HSM üzerinden kullanılmalı.
// SoftHSM ile test, gerçek donanımda prod kullanımı mümkündür.
