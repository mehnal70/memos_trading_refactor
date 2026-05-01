// fcm_push.rs
// Firebase Cloud Messaging (FCM) ile mobil push notification entegrasyonu
// Türkçe açıklamalar ile

use reqwest::blocking::Client;
use serde_json::json;

pub struct FcmPush;

impl FcmPush {
    pub fn send_push(token: &str, title: &str, body: &str, fcm_server_key: &str) -> Result<(), String> {
        let client = Client::new();
        let msg = json!({
            "to": token,
            "notification": {
                "title": title,
                "body": body
            }
        });
        let res = client.post("https://fcm.googleapis.com/fcm/send")
            .header("Authorization", format!("key={}", fcm_server_key))
            .header("Content-Type", "application/json")
            .body(msg.to_string())
            .send()
            .map_err(|e| format!("FCM gönderim hatası: {e}"))?;
        if res.status().is_success() {
            Ok(())
        } else {
            Err(format!("FCM başarısız: {}", res.status()))
        }
    }
}
