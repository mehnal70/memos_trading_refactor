//! IBKR venue'unun core'dan bağımsız transport katmanı.
//!
//! IBKR bridge, protokol ve venue adapter'ı bu crate'te yaşar. Persistence wiring
//! `CandleFallback` portu üzerinden dışarıdan enjekte edilir.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::timeout;

pub use memos_venue_api::Result;
pub mod protocol;
pub mod venue;

pub const IBKR_DEFAULT_ADDR: &str = "127.0.0.1:9101";

pub struct IbkrBridge {
    addr: String,
    io_timeout: Duration,
    connect_timeout: Duration,
    conn: Mutex<Option<BufReader<TcpStream>>>,
    next_id: AtomicU64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IbkrFarmHealth {
    pub connected: bool,
    pub market_data: bool,
    pub historical: bool,
    pub security_definition: bool,
}

pub use venue::{IbkrVenue, IbkrVenueFactory};

impl IbkrBridge {
    pub fn new(addr: String, io_timeout: Duration, connect_timeout: Duration) -> Self {
        Self {
            addr,
            io_timeout,
            connect_timeout,
            conn: Mutex::new(None),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn with_defaults(addr: Option<String>) -> Self {
        let io_timeout_secs = std::env::var("IBKR_IO_TIMEOUT_SEC")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(10);
        Self::new(
            addr.unwrap_or_else(|| IBKR_DEFAULT_ADDR.to_string()),
            Duration::from_secs(io_timeout_secs),
            Duration::from_secs(5),
        )
    }

    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }

    async fn connect(&self) -> Result<BufReader<TcpStream>> {
        let stream = timeout(self.connect_timeout, TcpStream::connect(self.addr.clone()))
            .await
            .map_err(|_| "IBKR bridge bağlantı zaman aşımı")?
            .map_err(|e| format!("IBKR bridge bağlantı hatası: {e}"))?;
        let _ = stream.set_nodelay(true);
        Ok(BufReader::new(stream))
    }

    async fn exchange(
        stream: &mut BufReader<TcpStream>,
        line: &str,
        io_timeout: Duration,
    ) -> Result<String> {
        let payload = format!("{}\n", line.trim_end());
        timeout(io_timeout, stream.get_mut().write_all(payload.as_bytes()))
            .await
            .map_err(|_| "IBKR yazma zaman aşımı")?
            .map_err(|e| format!("IBKR yazma hatası: {e}"))?;
        timeout(io_timeout, stream.get_mut().flush())
            .await
            .map_err(|_| "IBKR flush zaman aşımı")?
            .map_err(|e| format!("IBKR flush hatası: {e}"))?;

        let mut response = String::new();
        let n = timeout(io_timeout, stream.read_line(&mut response))
            .await
            .map_err(|_| "IBKR okuma zaman aşımı")?
            .map_err(|e| format!("IBKR okuma hatası: {e}"))?;
        if n == 0 {
            return Err("IBKR bridge bağlantısı kapandı (boş okuma)".into());
        }
        Ok(response)
    }

    pub async fn request(&self, line: &str) -> Result<String> {
        let mut guard = timeout(self.io_timeout, self.conn.lock())
            .await
            .map_err(|_| "IBKR bağlantı kuyruğu zaman aşımı")?;
        if guard.is_none() {
            *guard = Some(self.connect().await?);
        }
        let stream = guard
            .as_mut()
            .ok_or_else(|| "IBKR bağlantısı oluşturulamadı".to_string())?;
        match Self::exchange(stream, line, self.io_timeout).await {
            Ok(response) => Ok(response),
            Err(error) => {
                *guard = None;
                Err(error)
            }
        }
    }

    pub async fn health_check(&self) -> Result<()> {
        let body = self
            .request(&format!("{{\"id\":{},\"cmd\":\"ping\"}}", self.next_id()))
            .await?;
        let value: serde_json::Value = serde_json::from_str(body.trim())
            .map_err(|e| format!("IBKR health yanıtı parse edilemedi: {e}"))?;
        if value.get("ok").and_then(|ok| ok.as_bool()) == Some(true) {
            Ok(())
        } else {
            Err("IBKR health yanıtı başarısız".into())
        }
    }

    pub async fn farm_health(&self) -> Result<IbkrFarmHealth> {
        let body = self
            .request(&protocol::req_health(self.next_id()))
            .await?;
        let value: serde_json::Value = serde_json::from_str(body.trim())
            .map_err(|e| format!("IBKR farm health yanıtı parse edilemedi: {e}"))?;
        if value.get("ok").and_then(|ok| ok.as_bool()) != Some(true) {
            return Err("IBKR farm health yanıtı başarısız".into());
        }
        let farms = value
            .get("farms")
            .and_then(|farms| farms.as_object())
            .ok_or_else(|| "IBKR farm health yanıtında farms yok".to_string())?;
        let read = |name: &str| {
            farms
                .get(name)
                .and_then(|value| value.as_bool())
                .ok_or_else(|| format!("IBKR farm health alanı eksik: {name}"))
        };
        Ok(IbkrFarmHealth {
            connected: value
                .get("connected")
                .and_then(|connected| connected.as_bool())
                .unwrap_or(false),
            market_data: read("market_data")?,
            historical: read("historical")?,
            security_definition: read("security_definition")?,
        })
    }
}
