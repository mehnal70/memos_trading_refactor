use tokio::time::{sleep, Duration};
#[allow(dead_code)]
    /// Otomatik retry/backoff ile emir gönderme
    async fn place_order_with_retry(order_manager: &impl OrderManager, order: &Order, retry_policy: &RetryPolicy) -> MemosTradingResult<OrderId> {
        let mut attempt = 0;
        loop {
            match order_manager.place_order(order).await {
                Ok(order_id) => return Ok(order_id),
                Err(e) => {
                    if attempt >= retry_policy.max_retries {
                        log::error!("Emir gönderilemedi, retry limit aşıldı: {e}");
                        return Err(e);
                    }
                    let delay = retry_policy.get_delay_for_attempt(attempt);
                    log::warn!("Emir gönderilemedi, tekrar denenecek (attempt={attempt}, delay={delay}ms): {e}");
                    sleep(Duration::from_millis(delay)).await;
                    attempt += 1;
                }
            }
        }
    }

#[allow(dead_code)]
    /// Kısmi dolumda otomatik order flow (kalan miktar için yeni emir)
    async fn handle_partial_fill(order_manager: &impl OrderManager, order: &Order) -> MemosTradingResult<()> {
        if !order.is_fully_filled() && order.remaining_quantity() > 0.0 {
            log::info!("Kısmi dolum tespit edildi, kalan miktar için yeni emir oluşturuluyor: {}", order.remaining_quantity());
            let mut new_order = order.clone();
            new_order.quantity = order.remaining_quantity();
            order_manager.place_order(&new_order).await?;
        }
        Ok(())
    }

#[allow(dead_code)]
    /// Kritik slippage durumunda otomatik aksiyon (ör: emir iptal veya yeniden fiyatlama)
    async fn handle_critical_slippage(order_manager: &impl OrderManager, order: &Order, slippage: &SlippageInfo) -> MemosTradingResult<()> {
        if slippage.level == SlippageLevel::Critical {
            log::error!("Kritik slippage tespit edildi! Emir iptal ediliyor: {}", order.id.map(|id| id.to_string()).unwrap_or_default());
            if let Some(order_id) = order.id {
                order_manager.cancel_order(order_id).await?;
            }
            // Alternatif: yeni fiyatla tekrar emir oluşturulabilir
        }
        Ok(())
    }
// Order Management System - Base Trait ve Generic Implementation
// 
// Srivastava mimarisi: OMS bir service olarak hareket eder
// Trait-based design sayesinde Binance, KuCoin vb. kolayca eklenebilir

use super::types::*;
use crate::Result as MemosTradingResult;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Order Management System Trait - Tüm implementations bunu implement etmeli
#[async_trait]
pub trait OrderManager: Send + Sync {
    /// Yeni emir gönder
    async fn place_order(&self, order: &Order) -> MemosTradingResult<OrderId>;
    
    /// Emiri iptal et
    async fn cancel_order(&self, order_id: OrderId) -> MemosTradingResult<()>;
    
    /// Emir durumunu sorgula
    async fn get_order_status(&self, order_id: OrderId) -> MemosTradingResult<OrderStatus>;
    
    /// Emir detaylarını al
    async fn get_order(&self, order_id: OrderId) -> MemosTradingResult<Order>;
    
    /// Aktif emirleri listele
    async fn list_active_orders(&self, symbol: Option<&str>) -> MemosTradingResult<Vec<Order>>;
    
    /// Emir geçmişini al (pagination destekli)
    async fn get_order_history(
        &self,
        symbol: Option<&str>,
        limit: Option<usize>,
    ) -> MemosTradingResult<Vec<Order>>;
}

/// Slippage Detection Trait
pub trait SlippageDetector: Send + Sync {
    /// Slippage'ı tespit et ve rapor et
    fn detect(&self, expected_price: f64, actual_price: f64) -> SlippageInfo;
    
    /// Slippage limiti aşıldı mı?
    fn is_critical(&self, slippage: &SlippageInfo) -> bool;
}

/// Varsayılan Slippage Detector
pub struct DefaultSlippageDetector {
    /// Critical slippage threshold (%)
    pub critical_threshold: f64,
}

impl DefaultSlippageDetector {
    pub fn new(critical_threshold: f64) -> Self {
        Self { critical_threshold }
    }
}

impl Default for DefaultSlippageDetector {
    fn default() -> Self {
        Self::new(1.0) // Default: 1% critical
    }
}

impl SlippageDetector for DefaultSlippageDetector {
    fn detect(&self, expected_price: f64, actual_price: f64) -> SlippageInfo {
        let slippage_pct = if expected_price == 0.0 {
            0.0
        } else {
            ((actual_price - expected_price) / expected_price).abs() * 100.0
        };
        
        let level = if slippage_pct < 0.1 {
            SlippageLevel::Low
        } else if slippage_pct < 0.5 {
            SlippageLevel::Medium
        } else if slippage_pct < 1.0 {
            SlippageLevel::High
        } else {
            SlippageLevel::Critical
        };
        
        SlippageInfo {
            expected_price,
            actual_price,
            slippage_pct,
            level,
        }
    }
    
    fn is_critical(&self, slippage: &SlippageInfo) -> bool {
        slippage.slippage_pct >= self.critical_threshold
    }
}

/// Generic OMS Implementation with in-memory order tracking
/// 
/// Bu implementation mock/test için kullanılabilir.
/// Gerçek exchange'ler bu trait'i override edecek.
pub struct BaseOrderManagementSystem {
    /// Yerleşik emirler storage (ID → Order)
    orders: Arc<RwLock<HashMap<OrderId, Order>>>,
    
    /// Son order ID
    next_order_id: Arc<RwLock<u64>>,
    
    /// Slippage detector
    slippage_detector: Arc<dyn SlippageDetector>,
    
    /// Retry politikası
    retry_policy: RetryPolicy,
}

impl BaseOrderManagementSystem {
    pub fn new(slippage_detector: Arc<dyn SlippageDetector>) -> Self {
        Self {
            orders: Arc::new(RwLock::new(HashMap::new())),
            next_order_id: Arc::new(RwLock::new(1)),
            slippage_detector,
            retry_policy: RetryPolicy::default(),
        }
    }
    
    /// In-memory order tracking (testing için)
    pub fn with_custom_retry(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }
    
    /// Retry politikasını al
    pub fn retry_policy(&self) -> &RetryPolicy {
        &self.retry_policy
    }
    
    /// Slippage detector'ü al
    pub fn slippage_detector(&self) -> &Arc<dyn SlippageDetector> {
        &self.slippage_detector
    }
    
    /// Order ID oluştur (internal use)
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
        
        let mut orders = self.orders.write().await;
        orders.insert(order_id, order_mut);
        
        // Türkçe yorum: Emiri oluşturduk ve takip etmeye başladık
        println!("📍 Emir yerleştirildi: ID={}, Sembol={}", order_id, order.symbol);
        
        Ok(order_id)
    }
    
    async fn cancel_order(&self, order_id: OrderId) -> MemosTradingResult<()> {
        let mut orders = self.orders.write().await;
        
        if let Some(order) = orders.get_mut(&order_id) {
            if order.status != OrderStatus::Filled {
                order.status = OrderStatus::Canceled;
                println!("❌ Emir iptal edildi: ID={}", order_id);
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

/// OMS Factory - İhtiyaca göre doğru OMS'yi oluştur
pub struct OrderManagementSystem;

impl OrderManagementSystem {
    /// Binance OMS'yi oluştur (gerçek BinanceOrderManager)
    pub fn binance(api_key: &str, api_secret: &str) -> Arc<dyn OrderManager> {
        Arc::new(super::binance::BinanceOrderManager::new(
            api_key.to_string(),
            api_secret.to_string(),
        ))
    }
    
    /// Mock/Test OMS'yi oluştur
    pub fn mock() -> Arc<dyn OrderManager> {
        let detector = Arc::new(DefaultSlippageDetector::default());
        Arc::new(BaseOrderManagementSystem::new(detector))
    }
    
    /// Custom detector ile OMS oluştur
    pub fn with_detector(detector: Arc<dyn SlippageDetector>) -> Arc<dyn OrderManager> {
        Arc::new(BaseOrderManagementSystem::new(detector))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_place_and_get_order() {
        let oms = BaseOrderManagementSystem::new(
            Arc::new(DefaultSlippageDetector::default())
        );
        
        let order = Order::market(
            "BTCUSDT".to_string(),
            OrderSide::Buy,
            1.0
        );
        
        let order_id = oms.place_order(&order).await.unwrap();
        let status = oms.get_order_status(order_id).await.unwrap();
        
        assert_eq!(status, OrderStatus::New);
    }
    
    #[tokio::test]
    async fn test_cancel_order() {
        let oms = BaseOrderManagementSystem::new(
            Arc::new(DefaultSlippageDetector::default())
        );
        
        let order = Order::market(
            "BTCUSDT".to_string(),
            OrderSide::Buy,
            1.0
        );
        
        let order_id = oms.place_order(&order).await.unwrap();
        let _ = oms.cancel_order(order_id).await;
        let status = oms.get_order_status(order_id).await.unwrap();
        
        assert_eq!(status, OrderStatus::Canceled);
    }
    
    #[test]
    fn test_slippage_detection() {
        let detector = DefaultSlippageDetector::new(1.0);
        
        // 0.05% slippage (Low)
        let slippage = detector.detect(100.0, 100.05); // (100.05-100)/100 = 0.0005 = 0.05%
        assert_eq!(slippage.level, SlippageLevel::Low);
        assert!(!detector.is_critical(&slippage));
        
        // 0.3% slippage (Medium) 
        let slippage = detector.detect(100.0, 100.3); // (100.3-100)/100 = 0.003 = 0.3%
        assert_eq!(slippage.level, SlippageLevel::Medium);
        assert!(!detector.is_critical(&slippage));
        
        // 0.7% slippage (High)
        let slippage = detector.detect(100.0, 100.7); // (100.7-100)/100 = 0.007 = 0.7%
        assert_eq!(slippage.level, SlippageLevel::High);
        assert!(!detector.is_critical(&slippage));
        
        // 1.5% slippage (Critical)
        let slippage = detector.detect(100.0, 101.5); // (101.5-100)/100 = 0.015 = 1.5%
        assert_eq!(slippage.level, SlippageLevel::Critical);
        assert!(detector.is_critical(&slippage));
    }
}
