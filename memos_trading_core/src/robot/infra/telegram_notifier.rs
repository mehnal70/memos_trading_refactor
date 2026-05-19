// robot/infra/telegram_notifier.rs — Kritik trading olayları için push kanalı.
//
// Tasarım:
// 1. Fire-and-forget asenkron mimari — trading döngüsünü ms bile bloklamaz
// 2. Severity (Info / Warning / Critical) — mesaja otomatik emoji prefix
// 3. Per-key cooldown — aynı uyarı türü cooldown süresi içinde tekrar gönderilmez
//    (BALANCE-MISMATCH gibi her tick atılabilen olaylarda spam koruması)
// 4. Saf yardımcılar (`format_message`, `build_payload`, `should_send`) — IO'suz test edilebilir
// 5. WASM koruması — ağ katmanı sadece native target'ta derlenir

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(not(target_arch = "wasm32"))]
use reqwest::Client;

/// Uyarı şiddet seviyesi — mesaja eklenen emoji prefix'i belirler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

impl Severity {
    /// Mesaj başına eklenecek görsel ön ek.
    pub fn prefix(&self) -> &'static str {
        match self {
            Severity::Info     => "ℹ️",
            Severity::Warning  => "⚠️",
            Severity::Critical => "🚨",
        }
    }
}

/// Severity prefix + ham metin birleşimi. UI/Telegram için saf formatter.
pub fn format_message(severity: Severity, msg: &str) -> String {
    format!("{} {}", severity.prefix(), msg)
}

/// Telegram sendMessage payload'ı. Saf JSON; ağ çağrısı yapmaz.
pub fn build_payload(chat_id: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "chat_id":    chat_id,
        "text":       text,
        "parse_mode": "HTML",
    })
}

/// Cooldown kararı: aynı `key` için son gönderim üzerinden `cooldown` geçti mi?
/// `last` None ise (ilk kez) her zaman true.
pub fn should_send(now: Instant, last: Option<Instant>, cooldown: Duration) -> bool {
    match last {
        None => true,
        Some(t) => now.duration_since(t) >= cooldown,
    }
}

/// TELEGRAM_COOLDOWN_SECS ortam değişkeninden okur, parse edilemezse default 60 sn.
pub fn parse_cooldown_from_env() -> Duration {
    let secs = std::env::var("TELEGRAM_COOLDOWN_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(60);
    Duration::from_secs(secs)
}

/// TelegramNotifier — kritik trading olayları için otonom push kanalı.
#[cfg(not(target_arch = "wasm32"))]
pub struct TelegramNotifier {
    api_url:  String,
    chat_id:  String,
    client:   Client,
    cooldown: Duration,
    /// Per-key son gönderim zamanı — dedup/throttle için.
    last_sent: Mutex<HashMap<String, Instant>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl TelegramNotifier {
    /// Env'den otonom yapılandırıcı. TELEGRAM_BOT_TOKEN ve TELEGRAM_CHAT_ID
    /// her ikisi de boş olmadan set edilmemişse None döner (Telegram bağlı değil).
    pub fn from_env() -> Option<Self> {
        let token   = std::env::var("TELEGRAM_BOT_TOKEN").ok()?;
        let chat_id = std::env::var("TELEGRAM_CHAT_ID").ok()?;
        let (token, chat_id) = (token.trim(), chat_id.trim());
        if token.is_empty() || chat_id.is_empty() {
            return None;
        }
        Some(Self {
            api_url: format!("https://api.telegram.org/bot{}/sendMessage", token),
            chat_id: chat_id.to_string(),
            client: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            cooldown: parse_cooldown_from_env(),
            last_sent: Mutex::new(HashMap::new()),
        })
    }

    /// Cooldown süresini doğrudan vermek istediğin testler / programatik kullanım için.
    pub fn new(token: &str, chat_id: &str, cooldown: Duration) -> Self {
        Self {
            api_url: format!("https://api.telegram.org/bot{}/sendMessage", token),
            chat_id: chat_id.to_string(),
            client: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            cooldown,
            last_sent: Mutex::new(HashMap::new()),
        }
    }

    /// Throttle kararı: `key` için cooldown süresi geçtiyse `last_sent`'i şimdiye günceller
    /// ve true döner. Aksi halde false (skip).
    fn check_and_mark(&self, key: &str, now: Instant) -> bool {
        let mut map = match self.last_sent.lock() {
            Ok(m) => m,
            Err(p) => p.into_inner(),
        };
        let last = map.get(key).copied();
        if should_send(now, last, self.cooldown) {
            map.insert(key.to_string(), now);
            true
        } else {
            false
        }
    }

    /// Asenkron tek mesaj gönderimi. Throttle yapmaz — caller `notify` kullanmalı.
    pub async fn send_raw(&self, text: &str) -> Result<(), String> {
        let payload = build_payload(&self.chat_id, text);
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

    /// Severity + cooldown'lu bildirim. Trading döngüsünü bloklamaz (tokio::spawn).
    /// `key` aynı türden uyarıları gruplamak için (örn. "BALANCE-MISMATCH").
    /// Cooldown içindeyse sessizce skip — true: gönderildi, false: throttle.
    pub fn notify(&self, key: &str, severity: Severity, msg: &str) -> bool {
        if !self.check_and_mark(key, Instant::now()) {
            return false;
        }
        let url   = self.api_url.clone();
        let chat  = self.chat_id.clone();
        let text  = format_message(severity, msg);
        let client = self.client.clone();
        tokio::spawn(async move {
            let payload = build_payload(&chat, &text);
            let res = client.post(&url).json(&payload).send().await;
            match res.and_then(|r| r.error_for_status()) {
                Ok(_) => {}
                Err(e) => eprintln!("[Telegram] gönderim hatası: {}", e),
            }
        });
        true
    }
}

/// Notifier None ise sessizce geçer; varsa severity + cooldown ile gönderir.
#[macro_export]
macro_rules! tg_notify {
    ($notifier:expr, $key:expr, $sev:expr, $msg:expr) => {
        if let Some(ref n) = $notifier {
            n.notify($key, $sev, $msg);
        }
    };
    ($notifier:expr, $key:expr, $sev:expr, $fmt:literal, $($arg:tt)*) => {
        if let Some(ref n) = $notifier {
            n.notify($key, $sev, &format!($fmt, $($arg)*));
        }
    };
}
