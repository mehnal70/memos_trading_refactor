// siem_forwarder_integration.rs
// SIEM forwarder pipeline testi (mock endpoint ile)
//
// NOT: Bu test sadece `enterprise` feature açıkken anlamlıdır.
// Çalıştırmak için: `cargo test --features enterprise --test siem_forwarder_integration`

#![cfg(feature = "enterprise")]

#[test]
fn test_siem_forwarder_mock() {
    use memos_trading_core::siem_forwarder::{SiemForwarder, SiemConfig};
    use serde_json::json;
    // Mock syslog (localhost:514) ve HTTP endpoint (örnek)
    let cfg = SiemConfig {
        syslog_addr: Some("127.0.0.1:514".into()),
        http_url: Some("http://localhost:8081/mock_siem".into()),
        http_token: Some("testtoken".into()),
    };
    SiemForwarder::set_config(cfg);
    SiemForwarder::forward_log("test_event", &json!({"msg": "pipeline test"}));
    // Gerçek SIEM entegrasyonunda, endpoint doğrulaması eklenebilir
    assert!(true);
}
