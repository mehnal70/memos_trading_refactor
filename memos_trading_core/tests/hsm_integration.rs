// HSM entegrasyonu için pipeline test dosyası (SoftHSM ile çalışır)
//
// NOT: Bu test sadece `enterprise` feature açıkken anlamlıdır (HSM modülü gated).
// Çalıştırmak için: `cargo test --features enterprise --test hsm_integration`

#![cfg(feature = "enterprise")]

#[test]
fn test_hsm_generate_and_encrypt() {
    use memos_trading_core::hsm::HsmContext;
    // Test ortamı: SoftHSM modül yolu ve pin
    let module = "/usr/lib/softhsm/libsofthsm2.so";
    let pin = "1234";
    let _ = HsmContext::init(module, pin);
    let key = HsmContext::generate_aes_key("ci_test", 256).expect("AES anahtar üretilmeli");
    let data = b"testdata";
    let enc = HsmContext::encrypt_with_key(key, data).expect("Şifreleme başarılı olmalı");
    let dec = HsmContext::decrypt_with_key(key, &enc).expect("Çözme başarılı olmalı");
    assert_eq!(data.to_vec(), dec);
}
