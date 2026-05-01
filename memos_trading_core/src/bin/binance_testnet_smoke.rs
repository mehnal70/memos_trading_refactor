use memos_trading_core::robot::order_management::{BinanceOrderManager, Order, OrderManager, OrderSide, OrderStatus};
use std::env;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let symbol = env::var("SMOKE_SYMBOL").unwrap_or_else(|_| "BTCUSDT".to_string());
    let quantity = env::var("SMOKE_QTY")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.001);

    if env::var("BINANCE_API_KEY").unwrap_or_default().is_empty()
        || env::var("BINANCE_API_SECRET").unwrap_or_default().is_empty()
    {
        eprintln!("❌ BINANCE_API_KEY veya BINANCE_API_SECRET eksik.");
        eprintln!("Örnek: TRADING_ENV=testnet BINANCE_API_KEY=... BINANCE_API_SECRET=... cargo run -p memos_trading_core --bin binance_testnet_smoke");
        std::process::exit(1);
    }

    println!("🚀 Binance testnet smoke test başlıyor...");
    println!("   symbol={} qty={}", symbol, quantity);

    let manager = BinanceOrderManager::from_env_or_encrypted();

    let order = Order::market(symbol.clone(), OrderSide::Buy, quantity);

    let order_id = match manager.place_order(&order).await {
        Ok(order_id) => {
            println!("✅ Place order başarılı: {}", order_id);
            order_id
        }
        Err(err) => {
            eprintln!("❌ Place order başarısız: {}", err);
            std::process::exit(2);
        }
    };

    sleep(Duration::from_millis(700)).await;

    match manager.get_order_status(order_id).await {
        Ok(status) => println!("✅ Order status: {}", status),
        Err(err) => {
            eprintln!("❌ Status sorgusu başarısız: {}", err);
            std::process::exit(3);
        }
    }

    match manager.get_order(order_id).await {
        Ok(ord) => println!(
            "✅ Order detail: id={} symbol={} side={} type={} status={} filled={}/{}",
            ord.id.map(|x| x.0).unwrap_or_default(),
            ord.symbol,
            ord.side,
            ord.order_type,
            ord.status,
            ord.filled_quantity,
            ord.quantity,
        ),
        Err(err) => {
            eprintln!("❌ Order detail başarısız: {}", err);
            std::process::exit(4);
        }
    }

    let current_status = match manager.get_order_status(order_id).await {
        Ok(status) => status,
        Err(err) => {
            eprintln!("❌ Cancel öncesi status alınamadı: {}", err);
            std::process::exit(5);
        }
    };

    if current_status == OrderStatus::Filled {
        println!("ℹ️ Emir zaten FILLED, cancel adımı atlandı.");
    } else {
        match manager.cancel_order(order_id).await {
            Ok(_) => println!("✅ Cancel başarılı: {}", order_id),
            Err(err) => println!("⚠️ Cancel başarısız (bazı durumlarda normal olabilir): {}", err),
        }
    }

    match manager.get_order_history(Some(&symbol), Some(5)).await {
        Ok(history) => {
            println!("✅ History (son {} kayıt):", history.len());
            for h in history {
                println!(
                    "   - id={} {} {} qty={} status={}",
                    h.id.map(|x| x.0).unwrap_or_default(),
                    h.symbol,
                    h.side,
                    h.quantity,
                    h.status
                );
            }
        }
        Err(err) => {
            eprintln!("❌ History alınamadı: {}", err);
            std::process::exit(6);
        }
    }

    println!("🎯 Smoke test tamamlandı.");
}
