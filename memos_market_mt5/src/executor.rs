use crate::bridge::normalize_executor_addr;
use memos_venue_api::Result;
use memos_venue_api::TradeExecutor;
use memos_venue_api::{Signal, Trade};
use serde::{Deserialize, Serialize};

// MQL5 EA'nizin 9002 portundan bekleyeceği emir formatı
#[derive(Serialize)]
struct Mql5OrderRequest {
    pub action: &'static str, // "BUY", "SELL", "CANCEL"
    pub symbol: String,
    pub volume: f64,
}

// MQL5 EA'nizin emri işledikten sonra döneceği cevap formatı
#[derive(Deserialize)]
struct Mql5OrderResponse {
    pub success: bool,
    pub ticket: u64,
    pub error_msg: Option<String>,
    pub open_symbols: Option<Vec<String>>,
}

pub struct Mt5TradeExecutor {
    // Port 9002 TCP adresiniz (örn: "tcp://127.0.0.1:9002")
    _zmq_address: String,
}

impl Mt5TradeExecutor {
    pub fn new(address: &str) -> Self {
        Self {
            _zmq_address: normalize_executor_addr(address),
        }
    }

    // ZMQ REQ/REP üzerinden MT5 Expert Advisor ile konuşan iç fonksiyon
    async fn send_mql5_request(&self, req: Mql5OrderRequest) -> Result<Mql5OrderResponse> {
        let _ = req;
        Err("MT5 legacy executor etkin değil; VenueAdapter::submit_order üzerinden 9001 köprüsünü kullanın".into())
    }
}

impl TradeExecutor for Mt5TradeExecutor {
    fn execute(&self, _signal: Signal, symbol: &str, amount: f64) -> Result<Trade> {
        let action = if amount > 0.0 { "BUY" } else { "SELL" };
        let req = Mql5OrderRequest {
            action,
            symbol: symbol.to_string(),
            volume: amount.abs(),
        };

        // Altyapı filomuz senkron beklediği için asenkron ZMQ çağrısını tokio ile blokluyoruz
        let runtime = tokio::runtime::Handle::current();
        let res = runtime
            .block_on(self.send_mql5_request(req))
            .map_err(|e| format!("MT5 ZMQ Bağlantı Hatası: {:?}", e))?;

        if res.success {
            Ok(Trade {
                id: Some(res.ticket),
                symbol: symbol.to_string(),
                entry_price: 0.0,
                exit_price: None,
                amount: amount.abs(),
                entry_time: chrono::Utc::now(),
                exit_time: None,
                pnl: None,
                pnl_pct: None,
                strategy: "MT5_Otonom".to_string(),
            })
        } else {
            Err(format!("MT5 Emir Reddedildi: {:?}", res.error_msg).into())
        }
    }

    fn cancel_all(&self, symbol: &str) -> Result<()> {
        let req = Mql5OrderRequest {
            action: "CANCEL",
            symbol: symbol.to_string(),
            volume: 0.0,
        };
        let runtime = tokio::runtime::Handle::current();
        let _ = runtime.block_on(self.send_mql5_request(req));
        Ok(())
    }

    fn fetch_open_symbols(&self) -> Vec<String> {
        let req = Mql5OrderRequest {
            action: "FETCH_OPEN",
            symbol: String::new(),
            volume: 0.0,
        };
        let runtime = tokio::runtime::Handle::current();
        if let Ok(res) = runtime.block_on(self.send_mql5_request(req)) {
            res.open_symbols.unwrap_or_default()
        } else {
            return vec![];
        }
    }
}
