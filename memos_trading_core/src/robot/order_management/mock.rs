// Order Management System - Mock/Test Implementation
// 
// Srivastava mimarisi: Testing ve simulation için
// Gerçek API çağrısı yapmadan test etmek için kullanılır

use super::types::*;
use crate::Result as MemosTradingResult;
use async_trait::async_trait;
use super::base::OrderManager;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;

/// Mock Order Manager - Testing ve Backtesting için
pub struct MockOrderManager {
    /// In-memory orders storage
    orders: Arc<RwLock<HashMap<OrderId, Order>>>,
    
    /// Son order ID
    next_order_id: Arc<RwLock<u64>>,
    
    /// Emir yerleştirmenin başarılı olma ihtimali (0.0 - 1.0)
    success_rate: f64,
    
    /// Simule edilmiş slippage
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
    
    /// Success rate ayarla (0.0 - 1.0)
    pub fn with_success_rate(mut self, rate: f64) -> Self {
        self.success_rate = rate.max(0.0).min(1.0);
        self
    }
    
    /// Simüle edilmiş slippage ayarla
    pub fn with_slippage(mut self, slippage_pct: f64) -> Self {
        self.simulated_slippage_pct = slippage_pct;
        self
    }
    
    /// Tüm emirleri silindir (test cleanup için)
    pub async fn clear(&self) {
        let mut orders = self.orders.write().await;
        orders.clear();
        let mut id = self.next_order_id.write().await;
        *id = 1;
    }
    
    /// Order ID oluştur
    async fn generate_order_id(&self) -> OrderId {
        let mut id = self.next_order_id.write().await;
        let order_id = OrderId(*id);
        *id += 1;
        order_id
    }
}

impl Default for MockOrderManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl OrderManager for MockOrderManager {
    async fn place_order(&self, order: &Order) -> MemosTradingResult<OrderId> {
        // Success rate kontrol et (check in a block to avoid Send trait issue)
        {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            if rng.gen::<f64>() > self.success_rate {
                return Err("Mock: Emir reddedildi (success_rate)".into());
            }
        }
        
        let order_id = self.generate_order_id().await;
        let mut order_mut = order.clone();
        order_mut.id = Some(order_id);
        order_mut.created_at = Some(chrono::Utc::now());
        
        // Mock slippage ekle
        if self.simulated_slippage_pct > 0.0 && order_mut.price.is_some() {
            let original_price = order_mut.price.unwrap();
            let slipped_price = original_price * (1.0 + self.simulated_slippage_pct / 100.0);
            order_mut.average_price = slipped_price;
        }
        
        // Market order ise anında doldur
        if order.order_type == OrderType::Market {
            order_mut.status = OrderStatus::Filled;
            order_mut.filled_quantity = order_mut.quantity;
        } else {
            order_mut.status = OrderStatus::New;
        }
        
        let mut orders = self.orders.write().await;
        orders.insert(order_id, order_mut);
        
        println!("✅ Mock emir yerleştirildi: ID={}, Sembol={}", order_id, order.symbol);
        
        Ok(order_id)
    }
    
    async fn cancel_order(&self, order_id: OrderId) -> MemosTradingResult<()> {
        let mut orders = self.orders.write().await;
        
        if let Some(order) = orders.get_mut(&order_id) {
            if order.status != OrderStatus::Filled {
                order.status = OrderStatus::Canceled;
                println!("✅ Mock emir iptal: ID={}", order_id);
                Ok(())
            } else {
                Err("Doldurulmuş emiri iptal edemez".into())
            }
        } else {
            Err("Emir bulunamadı".into())
        }
    }
    
    async fn get_order_status(&self, order_id: OrderId) -> MemosTradingResult<OrderStatus> {
        let orders = self.orders.read().await;
        orders
            .get(&order_id)
            .map(|o| o.status)
            .ok_or("Emir bulunamadı".into())
    }
    
    async fn get_order(&self, order_id: OrderId) -> MemosTradingResult<Order> {
        let orders = self.orders.read().await;
        orders
            .get(&order_id)
            .cloned()
            .ok_or("Emir bulunamadı".into())
    }
    
    async fn list_active_orders(&self, symbol: Option<&str>) -> MemosTradingResult<Vec<Order>> {
        let orders = self.orders.read().await;
        let active: Vec<Order> = orders
            .values()
            .filter(|o| {
                let status_ok = o.status == OrderStatus::New
                    || o.status == OrderStatus::PartiallyFilled;
                let symbol_ok = symbol.is_none() || symbol == Some(&o.symbol[..]);
                status_ok && symbol_ok
            })
            .cloned()
            .collect();
        Ok(active)
    }
    
    async fn get_order_history(
        &self,
        symbol: Option<&str>,
        limit: Option<usize>,
    ) -> MemosTradingResult<Vec<Order>> {
        let orders = self.orders.read().await;
        let mut history: Vec<Order> = orders
            .values()
            .filter(|o| symbol.is_none() || symbol == Some(&o.symbol[..]))
            .cloned()
            .collect();
        
        history.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        
        if let Some(lim) = limit {
            history.truncate(lim);
        }
        
        Ok(history)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_mock_place_market_order() {
        let oms = MockOrderManager::new();
        
        let order = Order::market(
            "BTCUSDT".to_string(),
            OrderSide::Buy,
            1.0
        );
        
        let order_id = oms.place_order(&order).await.unwrap();
        let retrieved = oms.get_order(order_id).await.unwrap();
        
        // Market order anında doldurulmalı
        assert_eq!(retrieved.status, OrderStatus::Filled);
        assert_eq!(retrieved.filled_quantity, 1.0);
    }
    
    #[tokio::test]
    async fn test_mock_with_failure_rate() {
        let oms = MockOrderManager::new()
            .with_success_rate(0.0); // %0 başarı
        
        let order = Order::market(
            "BTCUSDT".to_string(),
            OrderSide::Buy,
            1.0
        );
        
        // Birden fazla deneme yapınca sonunda başarısız olmalı
        let mut failed_count = 0;
        for _ in 0..10 {
            if oms.place_order(&order).await.is_err() {
                failed_count += 1;
            }
        }
        
        assert!(failed_count > 0);
    }
    
    #[tokio::test]
    async fn test_mock_limit_order_stays_new() {
        let oms = MockOrderManager::new();
        
        let order = Order::limit(
            "BTCUSDT".to_string(),
            OrderSide::Buy,
            1.0,
            45000.0
        );
        
        let order_id = oms.place_order(&order).await.unwrap();
        let retrieved = oms.get_order(order_id).await.unwrap();
        
        // Limit order başlangıçta NEW durumunda kalmalı
        assert_eq!(retrieved.status, OrderStatus::New);
    }
}
