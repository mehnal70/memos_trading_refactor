//! Telegram Bot API entegrasyonu — kritik trading olayları için push bildirimi.
//!
//! # Yapılandırma (ortam değişkenleri)
//! | Değişken | Açıklama |
//! |---|---|
//! | `TELEGRAM_BOT_TOKEN` | BotFather'dan alınan token (örn. `123456:ABC-DEF...`) |
//! | `TELEGRAM_CHAT_ID`   | Hedef chat/grup ID (örn. `-100123456789` veya `987654321`) |
//!
//! # Kullanım
//! ```rust
//! if let Some(notif) = TelegramNotifier::from_env() {
//!     notif.send_fire_forget("⚡ BTCUSDT LONG açıldı @ 84200");
//! }
//! ```
//!
//! Gönderim başarısız olursa sessizce geçer — trading döngüsü bloklanmaz.

#[cfg(not(target_arch = "wasm32"))]
use reqwest::Client;

/// Telegram Bot API bildirici.
/// `from_env()` ile oluşturulur; env var yoksa `None` döner (özellik devre dışı).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct TelegramNotifier {
    bot_token: String,
    chat_id:   String,
    client:    Client,
}

#[cfg(not(target_arch = "wasm32"))]
impl TelegramNotifier {
    /// `TELEGRAM_BOT_TOKEN` ve `TELEGRAM_CHAT_ID` env var'larından oluştur.
    /// İkisi de tanımlı değilse `None` döner.
    pub fn from_env() -> Option<Self> {
        let token   = std::env::var("TELEGRAM_BOT_TOKEN").ok()?;
        let chat_id = std::env::var("TELEGRAM_CHAT_ID").ok()?;
        if token.trim().is_empty() || chat_id.trim().is_empty() {
            return None;
        }
        Some(Self {
            bot_token: token,
            chat_id,
            client: Client::new(),
        })
    }

    /// Oluşturulmuş URL — `sendMessage` endpoint'i.
    fn api_url(&self) -> String {
        format!("https://api.telegram.org/bot{}/sendMessage", self.bot_token)
    }

    /// Mesajı async olarak gönder. Başarısız olursa `Err` döner ama caller genellikle yoksayar.
    pub async fn send(&self, text: &str) -> Result<(), String> {
        let payload = serde_json::json!({
            "chat_id":    self.chat_id,
            "text":       text,
            "parse_mode": "HTML",
        });
        self.client
            .post(self.api_url())
            .json(&payload)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Fire-and-forget: tokio::spawn ile arka planda gönder, döngüyü bloklamaz.
    /// Hata olursa sessizce atılır.
    pub fn send_fire_forget(&self, text: &str) {
        let notif = self.clone();
        let msg   = text.to_string();
        tokio::spawn(async move {
            if let Err(e) = notif.send(&msg).await {
                // Bloke etme — sadece uyarı yaz (stderr, logger yok)
                eprintln!("[telegram] Gönderim başarısız: {}", e);
            }
        });
    }
}

// ── Kolaylaştırıcı makro: notifier None ise sessizce geç ─────────────────────
/// `send_alert!(notifier_opt, "mesaj")` — notifier None ise no-op.
#[macro_export]
macro_rules! send_alert {
    ($notifier:expr, $msg:expr) => {
        if let Some(ref __n) = $notifier {
            __n.send_fire_forget($msg);
        }
    };
    ($notifier:expr, $fmt:literal, $($arg:tt)*) => {
        if let Some(ref __n) = $notifier {
            __n.send_fire_forget(&format!($fmt, $($arg)*));
        }
    };
}
