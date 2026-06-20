//! `Mt5Bridge` — MT5 EA ile satır-sınırlı (NDJSON) istek/yanıt köprüsü.
//!
//! Rust **server**'dır: yerel bir TCP portunu dinler, MT5 EA dışarı bağlanır (MQL5'te server
//! socket yok). Bağlantı tek ve persistandır; bir istek = bir satır YAZ + bir satır OKU. Tüm
//! çevrim bir muteks altında seri çalışır (eşzamanlı sembol istekleri sıraya girer) — bu,
//! tek-soketli req/resp modelinin bilinçli sadeliğidir.
//!
//! Dayanıklılık: bağlantı/IO hatasında soket düşürülür → sonraki istek yeni EA bağlantısını
//! kabul eder. Bağlantı yokken `accept` zaman aşımına uğrarsa açık `Err` döner (sahte değer yok).
//! Listener tembel bağlanır (registry kurulumu sync; bind async) — ilk istekte ayağa kalkar.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::Result;

/// Köprü varsayılan dinleme adresi (yalnız loopback — EA aynı makinedeki MT5 terminali).
pub const MT5_DEFAULT_ADDR: &str = "127.0.0.1:9001";

pub struct Mt5Bridge {
    addr: String,
    /// Tembel bağlanan dinleyici (ilk istekte ayağa kalkar; sync kurulumla uyumlu).
    listener: Mutex<Option<TcpListener>>,
    /// Kabul edilmiş EA bağlantısı; IO hatasında None'a çekilir → yeniden kabul.
    conn: Mutex<Option<BufReader<TcpStream>>>,
    /// Tek istek (yaz+oku) için zaman aşımı.
    io_timeout: Duration,
    /// EA'nın bağlanmasını bekleme zaman aşımı.
    accept_timeout: Duration,
    /// İstek-yanıt eşleme/teşhis kimliği sayacı.
    next_id: AtomicU64,
}

impl Mt5Bridge {
    /// Verilen `addr` (host:port) üzerinde köprü (henüz bağlanmadan). `io_timeout` tek istek,
    /// `accept_timeout` EA bağlantısını bekleme süresi.
    pub fn new(addr: String, io_timeout: Duration, accept_timeout: Duration) -> Self {
        Self {
            addr,
            listener: Mutex::new(None),
            conn: Mutex::new(None),
            io_timeout,
            accept_timeout,
            next_id: AtomicU64::new(1),
        }
    }

    /// Varsayılan loopback adresi + makul zaman aşımları (istek 10s, kabul 15s).
    pub fn with_defaults(addr: Option<String>) -> Self {
        Self::new(
            addr.unwrap_or_else(|| MT5_DEFAULT_ADDR.to_string()),
            Duration::from_secs(10),
            Duration::from_secs(15),
        )
    }

    /// Sıradaki istek kimliği.
    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Dinleyiciyi (gerekirse) bağla ve bağlı adresi döndür. Tembel: ilk çağrıda bind eder.
    /// (Test fake-EA'nın bağlanacağı portu öğrenmek için de kullanılır — port 0 → OS atar.)
    pub async fn ensure_bound(&self) -> Result<SocketAddr> {
        let mut lg = self.listener.lock().await;
        if lg.is_none() {
            let l = TcpListener::bind(&self.addr)
                .await
                .map_err(|e| format!("MT5 köprü bind '{}': {e}", self.addr))?;
            log::info!(target: "MT5", "köprü dinliyor: {}", self.addr);
            *lg = Some(l);
        }
        lg.as_ref()
            .unwrap()
            .local_addr()
            .map_err(|e| format!("MT5 köprü local_addr: {e}").into())
    }

    /// EA bağlantısını kabul et (zaman aşımıyla). Listener tembel bağlanır.
    async fn accept(&self) -> Result<BufReader<TcpStream>> {
        self.ensure_bound().await?;
        let lg = self.listener.lock().await;
        let listener = lg.as_ref().expect("ensure_bound sonrası listener var");
        let (stream, peer) = timeout(self.accept_timeout, listener.accept())
            .await
            .map_err(|_| format!("MT5 EA bağlantısı zaman aşımı ({})", self.addr))?
            .map_err(|e| format!("MT5 accept: {e}"))?;
        let _ = stream.set_nodelay(true);
        log::info!(target: "MT5", "EA bağlandı: {peer}");
        Ok(BufReader::new(stream))
    }

    /// Bir istek satırı gönder, bir yanıt satırı al (seri). Bağlantı yoksa kurar; IO hatasında
    /// bağlantıyı düşürüp açık `Err` döner (sonraki istek yeniden bağlanır).
    pub async fn request(&self, line: &str) -> Result<String> {
        let mut guard = self.conn.lock().await;
        if guard.is_none() {
            *guard = Some(self.accept().await?);
        }
        let stream = guard.as_mut().expect("bağlantı kuruldu");
        match Self::exchange(stream, line, self.io_timeout).await {
            Ok(resp) => Ok(resp),
            Err(e) => {
                // Kırık soketi düşür → sonraki istek taze EA bağlantısını kabul etsin.
                *guard = None;
                Err(e)
            }
        }
    }

    /// Tek yaz+oku çevrimi (zaman aşımlı). Boş okuma (`n == 0`) = karşı taraf kapattı.
    async fn exchange(
        stream: &mut BufReader<TcpStream>,
        line: &str,
        io_timeout: Duration,
    ) -> Result<String> {
        let payload = format!("{}\n", line.trim_end());
        timeout(io_timeout, stream.get_mut().write_all(payload.as_bytes()))
            .await
            .map_err(|_| "MT5 yazma zaman aşımı")?
            .map_err(|e| format!("MT5 yazma: {e}"))?;
        timeout(io_timeout, stream.get_mut().flush())
            .await
            .map_err(|_| "MT5 flush zaman aşımı")?
            .map_err(|e| format!("MT5 flush: {e}"))?;

        let mut resp = String::new();
        let n = timeout(io_timeout, stream.read_line(&mut resp))
            .await
            .map_err(|_| "MT5 okuma zaman aşımı")?
            .map_err(|e| format!("MT5 okuma: {e}"))?;
        if n == 0 {
            return Err("MT5 EA bağlantısı kapandı (boş okuma)".into());
        }
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loopback round-trip: sahte EA bağlanır, isteği okur, sabit yanıt yazar. Köprünün
    /// server-tarafı yaz/oku akışını (ağ-stub'sız, gerçek soketle) doğrular.
    #[tokio::test]
    async fn roundtrip_with_fake_ea() {
        // Port 0 → OS efemeral port atar (çakışma yok).
        let bridge = Mt5Bridge::new(
            "127.0.0.1:0".into(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let addr = bridge.ensure_bound().await.expect("bind");

        // Sahte EA istemcisi: bağlan, bir satır oku, sabit yanıt yaz.
        let ea = tokio::spawn(async move {
            let stream = TcpStream::connect(addr).await.expect("EA connect");
            let mut r = BufReader::new(stream);
            let mut req = String::new();
            r.read_line(&mut req).await.expect("EA read req");
            assert!(req.contains("\"cmd\":\"tick\""), "istek geçti: {req}");
            r.get_mut()
                .write_all(b"{\"ok\":true,\"bid\":1.0,\"ask\":1.1}\n")
                .await
                .expect("EA write resp");
            r.get_mut().flush().await.unwrap();
        });

        let resp = bridge.request(r#"{"id":1,"cmd":"tick","symbol":"EURUSD"}"#).await.expect("request");
        assert!(resp.contains("\"bid\":1.0"), "yanıt alındı: {resp}");
        ea.await.unwrap();
    }

    #[test]
    fn ids_are_monotonic() {
        let b = Mt5Bridge::with_defaults(None);
        let a = b.next_id();
        let c = b.next_id();
        assert!(c > a);
    }
}
