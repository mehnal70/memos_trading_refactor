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

use memos_venue_api::Result;

/// Köprü varsayılan dinleme adresi (yalnız loopback — EA aynı makinedeki MT5 terminali).
pub const MT5_DEFAULT_ADDR: &str = "127.0.0.1:9001";
pub const MT5_DEFAULT_EXECUTOR_ADDR: &str = "tcp://127.0.0.1:9002";

pub fn normalize_bridge_addr(addr: &str) -> String {
    let trimmed = addr.trim();
    if let Some(stripped) = trimmed.strip_prefix("tcp://") {
        stripped.to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn normalize_executor_addr(addr: &str) -> String {
    let trimmed = addr.trim();
    if trimmed.starts_with("tcp://") {
        trimmed.to_string()
    } else {
        format!("tcp://{trimmed}")
    }
}

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

    /// Varsayılan loopback adresi + makul zaman aşımları (istek 30s, kabul 15s).
    /// `io_timeout` 30s: broker market-order fill yanıtı 10-30s sürebilir; veri
    /// istekleri (candles/tick) çok daha hızlı gelir ama aynı timeout kullansalar da güvenlidir.
    pub fn with_defaults(addr: Option<String>) -> Self {
        Self::new(
            addr.unwrap_or_else(|| MT5_DEFAULT_ADDR.to_string()),
            Duration::from_secs(30),
            Duration::from_secs(15),
        )
    }

    /// Sıradaki istek kimliği.
    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Dinleyiciyi (gerekirse) bağla ve bağlı adresi döndür. Tembel: ilk çağrıda bind eder.
    /// (Test fake-EA'nın bağlanacağı portu öğrenmek için de kullanılır — port 0 → OS atar.)
    // src/robot/venue/mt5/bridge.rs içerisindeki `ensure_bound` ve `accept` fonksiyonlarının nihai mühürlü hali:
    // GEREKÇE: `std::result::Result` ve `tokio::io::BufReader` mutlak adreslemeleri (absolute path) işlenerek,
    // projedeki tek argümanlı kurumsal Result alias'ı bypass edilmiş ve derleme hataları tamamen eritilmiştir.
    // src/robot/venue/mt5/bridge.rs - `ensure_bound` fonksiyonunun içindeki `match TcpListener::bind` alanı:
    // GEREKÇE: `ss -tlpn` çıktısının boş dönmesiyle kanıtlanan mikrosaniyelik thread yarışları (race condition),
    // işletim sistemi seviyesindeki anlık port esnemeleriyle uyumlu hale getirilerek temizlenmiştir.
    pub async fn ensure_bound(&self) -> std::result::Result<SocketAddr, String> {
        let mut lg = self.listener.lock().await;

        if lg.is_none() {
            match TcpListener::bind(&self.addr).await {
                Ok(l) => {
                    log::info!(target: "MT5", "🦅 [SINGLETON-PORT] Memos TCP Server başarıyla kilit altına alındı ve dinliyor: {}", self.addr);
                    *lg = Some(l);
                }
                Err(e) => {
                    let err_text = e.to_string();
                    // 💡 ULUSLARARASI ALTYAPI KALKANI:
                    // Eğer işletim sistemi mikrosaniyelik uyuşmazlıktan dolayı "already in use" fırlattıysa,
                    // Tokio runtime'ın hattı toparlaması için milisaniyeler seviyesinde (Örn: 10ms) bir anlık nefes aldırıyoruz.
                    // Bu sayede arkadan gelen işçi hatları porta çarpmadan, bir önceki thread'in atadığı `Some(l)` nesnesini güvenle teslim alır!
                    if err_text.contains("already in use") || err_text.contains("os error 98") {
                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                        // Nefes sonrası kilit altındaki nesneyi dürüstçe kontrol et, atanmışsa başarıyla dön:
                        if lg.is_some() {
                            if let Ok(addr) = self.addr.parse::<SocketAddr>() {
                                return Ok(addr);
                            }
                        }
                    }
                    return Err(format!(
                        "MT5 köprü bind '{}' başarısız: {err_text}",
                        self.addr
                    ));
                }
            }
        }

        return lg
            .as_ref()
            .unwrap()
            .local_addr()
            .map_err(|e| format!("MT5 köprü local_addr: {e}"));
    }

    /// EA bağlantısını kabul et (zaman aşımıyla). Listener tembel bağlanır.
    async fn accept(&self) -> std::result::Result<tokio::io::BufReader<TcpStream>, String> {
        self.ensure_bound().await?;

        let lg = self.listener.lock().await;
        let listener = match lg.as_ref() {
            Some(l) => l,
            None => {
                return Err(
                    "ensure_bound sonrası listener boşa düştü, mutabakat tazeleyin".to_string(),
                )
            }
        };

        let (stream, peer) = match timeout(self.accept_timeout, listener.accept()).await {
            Ok(Ok((s, p))) => (s, p),
            Ok(Err(e)) => return Err(format!("MT5 accept hatası: {e}")),
            Err(_) => return Err(format!("MT5 EA bağlantısı zaman aşımı ({})", self.addr)),
        };

        let _ = stream.set_nodelay(true);
        log::info!(target: "MT5", "🚀 [PORT-CONNECT] MQL5 Expert Advisor istemcisi başarıyla otobana bağlandı: {peer}");
        return Ok(tokio::io::BufReader::new(stream));
    }

    /// Persistan EA bağlantısı varken birikmiş reconnect soketlerini sınırlı şekilde tüket.
    async fn drain_pending_connections(&self) {
        let listener_guard = self.listener.lock().await;
        let Some(listener) = listener_guard.as_ref() else {
            return;
        };
        for _ in 0..64 {
            match timeout(Duration::from_millis(1), listener.accept()).await {
                Ok(Ok((stream, peer))) => {
                    log::warn!(
                        target: "MT5",
                        "🧹 Eski MT5 reconnect bağlantısı kuyruktan temizlendi: {peer}"
                    );
                    drop(stream);
                }
                Ok(Err(error)) => {
                    log::warn!(target: "MT5", "Bekleyen reconnect temizlenemedi: {error}");
                    break;
                }
                Err(_) => break,
            }
        }
    }

    /// Bir istek satırı gönder, bir yanıt satırı al (seri). Bağlantı yoksa kurar; IO hatasında
    /// bağlantıyı düşürüp açık `Err` döner (sonraki istek yeniden bağlanır).
    pub async fn request(&self, line: &str) -> std::result::Result<String, String> {
        // KABLOLAMA DÜZELTMESİ: `conn`'u dolduran hiçbir yol yoktu — `accept()` kabul
        // ettiği stream'i çağırana döndürüyordu ama kimse çağırmıyordu; `request()` ise
        // kimsenin doldurmadığı `conn`'a bakıp her seferinde "EA henüz bağlanmadı"
        // döndürüyordu. Yani köprü, EA gerçekten bağlansa bile HİÇ konuşamıyordu
        // (4 fake-EA testinin kırmızı olma sebebi). Akış artık tasarlandığı gibi:
        //   conn yoksa → accept() ile EA'yı bekle ve conn'a MÜHÜRLE (kalıcı bağlantı)
        //   conn varsa → exchange() ile zaman-aşımlı tek yaz+oku
        //   IO hatasında → conn düşürülür; sonraki istek yeniden accept eder.
        // NOT: eski gövde BufReader'ı ikinci kez sarıyordu — tamponlanmış baytlar
        // çöpe gidiyordu (NDJSON çerçeveleme bozulması). exchange() tek sarmalayıcıyla
        // ve io_timeout ile çalışır.
        let mut conn_guard = timeout(self.io_timeout, self.conn.lock())
            .await
            .map_err(|_| "MT5 bağlantı kuyruğu zaman aşımı".to_string())?;

        if conn_guard.is_none() {
            *conn_guard = Some(self.accept().await?);
        } else {
            self.drain_pending_connections().await;
        }
        let stream = conn_guard
            .as_mut()
            .ok_or_else(|| "MT5 EA bağlantısı oluşturulamadı".to_string())?;

        match Self::exchange(stream, line, self.io_timeout).await {
            Ok(resp) => Ok(resp.trim().to_string()),
            Err(e) => {
                // Bağlantı bozuldu: düşür ki sonraki istek yeniden accept etsin.
                *conn_guard = None;
                Err(format!("MT5 bridge IO: {e}"))
            }
        }
    }

    /// Otonom çalışan sistem için köprünün canlılığını hızlı doğrular.
    /// Bir `balance` isteği ile EA'ya “yaşıyor musun?” sorar; false/timeout ise açık
    /// hata döner. Bu kontrol, heartbeat kadar sistemin ana itki katmanı için kritik.
    pub async fn health_check(&self) -> Result<()> {
        let id = self.next_id();
        let line = format!("{{\"id\":{},\"cmd\":\"balance\"}}", id);
        let resp = self.request(&line).await?;
        let v: serde_json::Value = serde_json::from_str(resp.trim())
            .map_err(|e| format!("MT5 health_check JSON parse: {e}"))?;
        if v.get("ok").and_then(|b| b.as_bool()) == Some(true) {
            Ok(())
        } else {
            let err = v
                .get("error")
                .and_then(|m| m.as_str())
                .unwrap_or("health_check failed");
            Err(format!("MT5 bridge health_check failed: {err}").into())
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

    #[test]
    fn bridge_addresses_are_normalized_for_tcp_listener() {
        assert_eq!(
            normalize_bridge_addr("tcp://127.0.0.1:9001"),
            "127.0.0.1:9001"
        );
        assert_eq!(normalize_bridge_addr("127.0.0.1:9001"), "127.0.0.1:9001");
    }

    #[test]
    fn executor_addresses_are_normalized_for_zmq() {
        assert_eq!(
            normalize_executor_addr("127.0.0.1:9002"),
            "tcp://127.0.0.1:9002"
        );
        assert_eq!(
            normalize_executor_addr("tcp://127.0.0.1:9002"),
            "tcp://127.0.0.1:9002"
        );
    }

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

        let resp = bridge
            .request(r#"{"id":1,"cmd":"tick","symbol":"EURUSD"}"#)
            .await
            .expect("request");
        assert!(resp.contains("\"bid\":1.0"), "yanıt alındı: {resp}");
        ea.await.unwrap();
    }

    #[tokio::test]
    async fn health_check_succeeds_when_ea_responds() {
        let bridge = Mt5Bridge::new(
            "127.0.0.1:0".into(),
            Duration::from_secs(2),
            Duration::from_secs(2),
        );
        let addr = bridge.ensure_bound().await.expect("bind");

        let ea = tokio::spawn(async move {
            let stream = TcpStream::connect(addr).await.expect("EA connect");
            let mut r = BufReader::new(stream);
            let mut req = String::new();
            let _ = r.read_line(&mut req).await.expect("EA read req");
            assert!(
                req.contains("\"cmd\":\"balance\""),
                "healthcheck request: {req}"
            );
            r.get_mut()
                .write_all(b"{\"id\":1,\"ok\":true,\"balance\":1000.0}\n")
                .await
                .expect("EA write health resp");
            r.get_mut().flush().await.unwrap();
        });

        bridge
            .health_check()
            .await
            .expect("health_check should succeed");
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
