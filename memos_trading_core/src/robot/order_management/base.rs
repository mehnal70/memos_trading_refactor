// robot/order_management/base.rs - Srivastava OMS Mimari Temeli

use crate::core::model::{OrderId,OrderStatus};
use crate::core::model::Order;
use crate::robot::order_management::{RetryPolicy, SlippageInfo,SlippageLevel};
use crate::Result as MemosTradingResult;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

// --- 1. YARDIMCI İNFAZ PROTOKOLLERİ ---

/// Otonom Retry ve Akış Yönetimi
pub struct OrderFlowHandler;

impl OrderFlowHandler {
    /// Üstel geri çekilme (backoff) ile emir gönderme denemesi
    pub async fn place_with_retry(
        order_manager: &impl OrderManager, 
        order: &Order, 
        retry_policy: &RetryPolicy
    ) -> MemosTradingResult<OrderId> {
        let mut attempt = 0;
        loop {
            match order_manager.place_order(order).await {
                Ok(id) => return Ok(id),
                Err(e) => {
                    if attempt >= retry_policy.max_retries { return Err(e); }
                    let delay = retry_policy.get_delay_for_attempt(attempt);
                    log::warn!("Emir hatası, {}ms sonra tekrar denenecek. Deneme: {}", delay, attempt + 1);
                    sleep(Duration::from_millis(delay)).await;
                    attempt += 1;
                }
            }
        }
    }

    /// Kısmi dolumları otonom tamamlar
    pub async fn recover_partial_fill(order_manager: &impl OrderManager, order: &Order) -> MemosTradingResult<()> {
        if !order.is_fully_filled() && order.remaining_quantity() > 0.0 {
            let mut remaining_order = order.clone();
            remaining_order.quantity = order.remaining_quantity();
            order_manager.place_order(&remaining_order).await?;
        }
        Ok(())
    }
}

// --- 2. TRAIT TANIMLARI ---

#[async_trait]
pub trait OrderManager: Send + Sync {
    async fn place_order(&self, order: &Order) -> MemosTradingResult<OrderId>;
    async fn cancel_order(&self, order_id: OrderId) -> MemosTradingResult<()>;
    async fn get_order_status(&self, order_id: OrderId) -> MemosTradingResult<OrderStatus>;
    async fn get_order(&self, order_id: OrderId) -> MemosTradingResult<Order>;
    async fn list_active_orders(&self, symbol: Option<&str>) -> MemosTradingResult<Vec<Order>>;
    async fn get_order_history(&self, symbol: Option<&str>, limit: Option<usize>) -> MemosTradingResult<Vec<Order>>;
}

pub trait SlippageDetector: Send + Sync {
    fn detect(&self, expected: f64, actual: f64) -> SlippageInfo;
    fn is_critical(&self, slippage: &SlippageInfo) -> bool;
}

// --- 3. VARSAYILAN UYGULAMALAR (OMS CORE) ---

pub struct DefaultSlippageDetector { pub critical_threshold: f64 }

impl Default for DefaultSlippageDetector {
    fn default() -> Self {
        Self {
            critical_threshold: 1.0, // Varsayılan %1 kayma eşiği
        }
    }
}
impl DefaultSlippageDetector { pub fn new(threshold: f64) -> Self { Self { critical_threshold: threshold } } }

impl SlippageDetector for DefaultSlippageDetector {
    fn detect(&self, expected: f64, actual: f64) -> SlippageInfo {
        let slippage_pct = if expected == 0.0 { 0.0 } else { ((actual - expected) / expected).abs() * 100.0 };
        let level = match slippage_pct {
            p if p < 0.1 => SlippageLevel::Low,
            p if p < 0.5 => SlippageLevel::Medium,
            p if p < 1.0 => SlippageLevel::High,
            _ => SlippageLevel::Critical,
        };
        SlippageInfo { expected_price: expected, actual_price: actual, slippage_pct, level }
    }
    fn is_critical(&self, slippage: &SlippageInfo) -> bool { slippage.slippage_pct >= self.critical_threshold }
}

pub struct BaseOrderManagementSystem {
    orders: Arc<RwLock<HashMap<OrderId, Order>>>,
    next_order_id: Arc<RwLock<u64>>,
    pub slippage_detector: Arc<dyn SlippageDetector>,
    pub retry_policy: RetryPolicy,
}

impl BaseOrderManagementSystem {
    pub fn new(detector: Arc<dyn SlippageDetector>) -> Self {
        Self {
            orders: Arc::new(RwLock::new(HashMap::new())),
            next_order_id: Arc::new(RwLock::new(1)),
            slippage_detector: detector,
            retry_policy: RetryPolicy::default(),
        }
    }

    async fn generate_order_id(&self) -> OrderId {
        let mut id = self.next_order_id.write().await;
        let order_id = OrderId(*id);
        *id += 1;
        order_id
    }
}

#[async_trait]
impl OrderManager for BaseOrderManagementSystem {
    async fn place_order(&self, order: &Order) -> MemosTradingResult<OrderId> {
        let order_id = self.generate_order_id().await;
        let mut order_mut = order.clone();
        order_mut.id = Some(order_id);
        order_mut.status = OrderStatus::New;
        order_mut.created_at = Some(chrono::Utc::now());
        
        self.orders.write().await.insert(order_id, order_mut);
        println!("📍 Emir OMS'ye eklendi: ID={}, Sembol={}", order_id, order.symbol);
        Ok(order_id)
    }

    async fn cancel_order(&self, order_id: OrderId) -> MemosTradingResult<()> {
        let mut orders = self.orders.write().await;
        if let Some(order) = orders.get_mut(&order_id) {
            if order.status != OrderStatus::Filled {
                order.status = OrderStatus::Canceled;
                Ok(())
            } else { Err("Doldurulmuş emir iptal edilemez".into()) }
        } else { Err("Emir bulunamadı".into()) }
    }

    async fn get_order_status(&self, order_id: OrderId) -> MemosTradingResult<OrderStatus> {
        self.orders.read().await.get(&order_id).map(|o| o.status).ok_or("Emir bulunamadı".into())
    }

    async fn get_order(&self, order_id: OrderId) -> MemosTradingResult<Order> {
        self.orders.read().await.get(&order_id).cloned().ok_or("Emir bulunamadı".into())
    }

    async fn list_active_orders(&self, symbol: Option<&str>) -> MemosTradingResult<Vec<Order>> {
        let orders = self.orders.read().await;
        Ok(orders.values().filter(|o| {
            (o.status == OrderStatus::New || o.status == OrderStatus::PartiallyFilled) &&
            (symbol.is_none() || symbol == Some(&o.symbol))
        }).cloned().collect())
    }

    async fn get_order_history(&self, symbol: Option<&str>, limit: Option<usize>) -> MemosTradingResult<Vec<Order>> {
        let orders = self.orders.read().await;
        let mut history: Vec<_> = orders.values()
            .filter(|o| symbol.is_none() || symbol == Some(&o.symbol)).cloned().collect();
        history.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        if let Some(lim) = limit { history.truncate(lim); }
        Ok(history)
    }
}

pub struct OrderManagementSystem;

impl OrderManagementSystem {
    pub fn mock() -> Arc<dyn OrderManager> {
        Arc::new(BaseOrderManagementSystem::new(Arc::new(DefaultSlippageDetector::default())))
    }
}
