// Tauri Integration Example - Srivastava ATP Module Demo
// Bu dosya OMS ve Data Processor modüllerinin nasıl kullanılacağını gösteriyor

#[cfg(test)]
mod demo {
    use crate::robot::{
        OrderManagementSystem, Order, OrderSide, DataProcessor, DataValidator,
        OrderManager,
    };
    use crate::types::Candle;
    use chrono::Utc;
    use std::sync::Arc;

    /// Demo: Order Management System kullanımı
    #[tokio::test]
    async fn demo_oms() -> Result<(), Box<dyn std::error::Error>> {
        println!("=== Order Management System Demo ===\n");
        
        // 1. Mock OMS oluştur (testing için)
        let oms: Arc<dyn OrderManager> = OrderManagementSystem::mock();
        println!("✓ Mock OMS oluşturuldu");
        
        // 2. Market order gönder
        let market_order = Order::market(
            "BTCUSDT".into(),
            OrderSide::Buy,
            0.5
        );
        let order_id = oms.place_order(&market_order).await?;
        println!("✓ Market order gönderildi: ID={}", order_id);
        
        // 3. Durum kontrol et
        let status = oms.get_order_status(order_id).await?;
        println!("✓ Order durumu: {:?}", status);
        
        // 4. Limit order gönder
        let limit_order = Order::limit(
            "ETHUSDT".into(),
            OrderSide::Sell,
            2.0,
            2500.0
        );
        let limit_id = oms.place_order(&limit_order).await?;
        println!("✓ Limit order gönderildi: ID={}", limit_id);
        
        // 5. Aktif emirleri listele
        let active = oms.list_active_orders(None).await?;
        println!("✓ Toplam aktif emirler: {}", active.len());
        
        // 6. Emri iptal et
        let _ = oms.cancel_order(limit_id).await;
        println!("✓ Limit order iptal edildi");
        
        Ok(())
    }

    /// Demo: Data Processor kullanımı
    #[test]
    fn demo_data_processor() -> Result<(), Box<dyn std::error::Error>> {
        println!("\n=== Data Processor Demo ===\n");
        
        // 1. Ham OHLCV verisini oluştur
        let raw_candles = vec![
            Candle {
                timestamp: Utc::now(),
                open: 100.0,
                high: 105.0,
                low: 95.0,
                close: 102.0,
                volume: 1000.5,
                symbol: "btcusdt".to_string(), // lowercase (inconsistent)
                interval: "1H".to_string(),    // uppercase (inconsistent)
            },
            Candle {
                timestamp: Utc::now(),
                open: 102.0,
                high: 107.5,
                low: 101.0,
                close: 105.0,
                volume: 1200.123456, // Fazla decimal
                symbol: "BTCUSDT".to_string(),
                interval: "1h".to_string(),
            }
        ];
        
        println!("Raw candles ({}): Inconsistent formatting", raw_candles.len());
        
        // 2. Full pipeline: Temizle + Normalize + Valide et
        let clean_candles = DataProcessor::process_candles(raw_candles)?;
        
        println!("\n✓ Data Processing Pipeline Tamamlandı:");
        println!("  - Candles processed: {}", clean_candles.len());
        
        for (i, c) in clean_candles.iter().enumerate() {
            println!("\n  Candle {}:", i + 1);
            println!("    - Symbol: {} (normalized)", c.symbol);
            println!("    - Interval: {} (normalized)", c.interval);
            println!("    - Volume: {:.2} (rounded)", c.volume);
        }
        
        // 3. Validation check
        for candle in &clean_candles {
            DataValidator::validate_ohlc(candle)?;
        }
        println!("\n✓ OHLC validation: OK");
        
        Ok(())
    }
}
