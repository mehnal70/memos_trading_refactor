use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::Result;

pub const IBKR_DEFAULT_ADDR: &str = "127.0.0.1:9101";

pub struct IbkrBridge {
    addr: String,
    io_timeout: Duration,
    connect_timeout: Duration,
    conn: Mutex<Option<BufReader<TcpStream>>>,
    next_id: AtomicU64,
}

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
        Self::new(
            addr.unwrap_or_else(|| IBKR_DEFAULT_ADDR.to_string()),
            Duration::from_secs(10),
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

    async fn exchange(stream: &mut BufReader<TcpStream>, line: &str, io_timeout: Duration) -> Result<String> {
        let payload = format!("{}\n", line.trim_end());
        timeout(io_timeout, stream.get_mut().write_all(payload.as_bytes()))
            .await
            .map_err(|_| "IBKR yazma zaman aşımı")?
            .map_err(|e| format!("IBKR yazma hatası: {e}"))?;
        timeout(io_timeout, stream.get_mut().flush())
            .await
            .map_err(|_| "IBKR flush zaman aşımı")?
            .map_err(|e| format!("IBKR flush hatası: {e}"))?;

        let mut resp = String::new();
        let n = timeout(io_timeout, stream.read_line(&mut resp))
            .await
            .map_err(|_| "IBKR okuma zaman aşımı")?
            .map_err(|e| format!("IBKR okuma hatası: {e}"))?;
        if n == 0 {
            return Err("IBKR bridge bağlantısı kapandı (boş okuma)".into());
        }
        Ok(resp)
    }

    pub async fn request(&self, line: &str) -> Result<String> {
        let mut guard = self.conn.lock().await;
        if guard.is_none() {
            *guard = Some(self.connect().await?);
        }
        let stream = guard.as_mut().expect("ibkr conn initialized");
        match Self::exchange(stream, line, self.io_timeout).await {
            Ok(resp) => Ok(resp),
            Err(e) => {
                *guard = None;
                Err(e)
            }
        }
    }

    pub async fn health_check(&self) -> Result<()> {
        let line = crate::robot::venue::ibkr::protocol::req_ping(self.next_id());
        let body = self.request(&line).await?;
        Ok(crate::robot::venue::ibkr::protocol::parse_ack("ibkr-health", &body)?)
    }
}