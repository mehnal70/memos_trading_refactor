// robot/infra/telegram_notifier.rs - Srivastava ATP Otonom Bildirim Ünitesi
//
// Modernizasyon Notları:
// 1. Fire-and-Forget (Bloklamayan) asenkron mimari
// 2. HTML parse-mode ile zenginleştirilmiş bildirim desteği
// 3. Makro tabanlı otonom hata yutma (Safe-by-default)
// 4. WASM korumalı ağ katmanı

#[cfg(not(target_arch = "wasm32"))]
use reqwest::Client;

/// §84.4: TelegramNotifier - Kritik trading olayları için otonom push kanalı.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct TelegramNotifier {
    api_url: String,
    chat_id: String,
    client:  Client,
}

#[cfg(not(target_arch = "wasm32"))]
impl TelegramNotifier {
    /// Çevresel değişkenlerden (Env) otonom yapılandırıcı.
    pub fn from_env() -> Option<Self> {
        let token   = std::env::var("TELEGRAM_BOT_TOKEN").ok()?;
        let chat_id = std::env::var("TELEGRAM_CHAT_ID").ok()?;
        
        match (token.trim(), chat_id.trim()) {
            (t, c) if !t.is_empty() && !c.is_empty() => Some(Self {
                api_url: format!("https://api.telegram.org/bot{}/sendMessage", t),
                chat_id: c.to_string(),
                client:  Client::builder()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()
                    .unwrap_or_default(),
            }),
            _ => None,
        }
    }

    /// Mesajı asenkron olarak iletir.
    pub async fn send(&self, text: &str) -> Result<(), String> {
        let payload = serde_json::json!({
            "chat_id":    self.chat_id,
            "text":       text,
            "parse_mode": "HTML",
        });

        self.client
            .post(&self.api_url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Arka plan görevi (Fire-and-forget): Trading döngüsünü milisaniyelik bile olsa durdurmaz.
    pub fn send_fire_forget(&self, text: &str) {
        let notif = self.clone();
        let msg   = text.to_string();
        tokio::spawn(async move {
            if let Err(e) = notif.send(&msg).await {
                eprintln!("[Srivastava-Telegram] Bildirim iletilemedi: {}", e);
            }
        });
    }
}

// --- 2. OTONOM BİLDİRİM MAKROSU ---

/// Notifier None ise sessizce geçer, varsa formatlı mesaj gönderir.
#[macro_export]
macro_rules! send_alert {
    ($notifier:expr, $msg:expr) => {
        if let Some(ref n) = $notifier {
            n.send_fire_forget($msg);
        }
    };
    ($notifier:expr, $fmt:literal, $($arg:tt)*) => {
        if let Some(ref n) = $notifier {
            n.send_fire_forget(&format!($fmt, $($arg)*));
        }
    };
}
