// robot/order_management/mock.rs - Otonom Test ve Backtest Emir Yöneticisi

use crate::core::model::{OrderId,OrderStatus,OrderSide,OrderType};
use crate::core::model::Order;
use crate::Result as MemosTradingResult;
use async_trait::async_trait;
use super::base::OrderManager;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;

/// MockOrderManager: Borsayı simüle eden, hata ve kayma (slippage) enjekte edebilen yapı.
pub struct MockOrderManager {
    orders: Arc<RwLock<HashMap<OrderId, Order>>>,
    next_order_id: Arc<RwLock<u64>>,
    /// Emirlerin başarıyla iletilme ihtimali (0.0 - 1.0)
    success_rate: f64,
    /// Simüle edilmiş fiyat kayması (%)
    simulated_slippage_pct: f64,
}

impl MockOrderManager {
    pub fn new() -> Self {
        Self {
            orders: Arc::new(RwLock::new(HashMap::new())),
            next_order_id: Arc::new(RwLock::new(1)),
            success_rate: 1.0,
            simulated_slippage_pct: 0.0,
        }
    }

    pub fn with_success_rate(mut self, rate: f64) -> Self {
        self.success_rate = rate.clamp(0.0, 1.0);
        self
    }

    pub fn with_slippage(mut self, slippage_pct: f64) -> Self {
        self.simulated_slippage_pct = slippage_pct;
        self
    }

    pub async fn clear(&self) {
        self.orders.write().await.clear();
        *self.next_order_id.write().await = 1;
    }

    async fn generate_order_id(&self) -> OrderId {
        let mut id = self.next_order_id.write().await;
        let order_id = OrderId(*id);
        *id += 1;
        order_id
    }
}

impl Default for MockOrderManager { fn default() -> Self { Self::new() } }

#[async_trait]
impl OrderManager for MockOrderManager {
    async fn place_order(&self, order: &Order) -> MemosTradingResult<OrderId> {
        // 1. Otonom Başarı Denetimi
        {
            use rand::Rng;
            if rand::thread_rng().gen::<f64>() > self.success_rate {
                return Err("Mock: Borsa bağlantı hatası veya emir reddi".into());
            }
        }

        let order_id = self.generate_order_id().await;
        let mut order_mut = order.clone();
        order_mut.id = Some(order_id);
        order_mut.created_at = Some(chrono::Utc::now());

        // 2. Otonom Slippage Enjeksiyonu
        if self.simulated_slippage_pct > 0.0 && order_mut.price.is_some() {
            let original = order_mut.price.unwrap();
            order_mut.average_price = original * (1.0 + self.simulated_slippage_pct / 100.0);
        }

        // 3. İnfaz Otonomisi
        if order.order_type == OrderType::Market {
            order_mut.status = OrderStatus::Filled;
            order_mut.filled_quantity = order_mut.quantity;
        } else {
            order_mut.status = OrderStatus::New;
        }

        self.orders.write().await.insert(order_id, order_mut);
        println!("✅ Mock Emir: ID={}, {} {}", order_id, order.symbol, 
            if order.order_type == OrderType::Market { "Dolduruldu" } else { "Yerleştirildi" });
        
        Ok(order_id)
    }

    async fn cancel_order(&self, order_id: OrderId) -> MemosTradingResult<()> {
        let mut orders = self.orders.write().await;
        if let Some(order) = orders.get_mut(&order_id) {
            if order.status != OrderStatus::Filled {
                order.status = OrderStatus::Canceled;
                Ok(())
            } else { Err("Gerçekleşmiş emir iptal edilemez".into()) }
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
            let status_ok = o.status == OrderStatus::New || o.status == OrderStatus::PartiallyFilled;
            let symbol_ok = symbol.is_none() || symbol == Some(&o.symbol);
            status_ok && symbol_ok
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
