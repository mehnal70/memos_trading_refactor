// fcm_push.rs
// Firebase Cloud Messaging (FCM) ile mobil push notification entegrasyonu

use reqwest::Client;
use serde_json::json;

pub struct FcmPush;

impl FcmPush {
    /// Mobil cihaza bildirim gönderir (Asenkron ve Optimize)
    pub async fn send_push(
        token: &str, 
        title: &str, 
        body: &str, 
        fcm_server_key: &str
    ) -> Result<(), String> {
        // HTTP Client'ı her seferinde oluşturmak yerine reuse (yeniden kullanım) önerilir, 
        // ancak bu yapı için asenkron hale getirilmesi önceliklidir.
        let client = Client::new();

        let msg = json!({
            "to": token,
            "notification": {
                "title": title,
                "body": body
            },
            "priority": "high" // Trading sinyalleri için kritik öncelik
        });

        // .json() metodu kullanarak manuel String dönüşüm maliyetinden (to_string) kurtulduk
        let res = client
            .post("https://fcm.googleapis.com/fcm/send")
            .header("Authorization", format!("key={}", fcm_server_key))
            .json(&msg) 
            .send()
            .await // Ana thread'i bloklamaz
            .map_err(|e| format!("FCM ağ hatası: {e}"))?;

        if res.status().is_success() {
            Ok(())
        } else {
            let status = res.status();
            Err(format!("FCM sunucu hatası: {status}"))
        }
    }
}
