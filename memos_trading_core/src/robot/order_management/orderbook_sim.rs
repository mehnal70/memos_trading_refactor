// robot/order_management/orderbook_sim.rs
//
// L2 Order Book Simülatörü
//
// paper_executor.rs'deki sabit spread+slippage modelinden farklı olarak bu modül
// gerçek bir L2 order book'u simüle eder:
//
//   Ask seviyeleri: [(fiyat, miktar), ...]  → artan sıra (en iyi ask en düşük)
//   Bid seviyeleri: [(fiyat, miktar), ...]  → azalan sıra (en iyi bid en yüksek)
//
// Market emri fill mantığı:
//   Alış emri: ask seviyelerini en düşükten yukarı tüketir.
//   Satış emri: bid seviyelerini en yüksekten aşağı tüketir.
//   Emr tam dolmazsa "partial fill" döner.
//
// Sentetik book oluşturma:
//   Gerçek veri yoksa mid-price + yapılandırılabilir spread/derinlikten
//   üretilmiş sentetik bir book kullanılabilir.

use serde::{Deserialize, Serialize};

// ─── Temel tipler ──────────────────────────────────────────────────────────────

/// Tek bir order book seviyesi
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct BookLevel {
    /// Fiyat
    pub price: f64,
    /// Bu seviyedeki toplam miktar (base currency)
    pub qty: f64,
}

/// Market emri fill sonucu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillResult {
    /// İstenen miktar
    pub requested_qty: f64,
    /// Gerçekleşen miktar
    pub filled_qty: f64,
    /// Ağırlıklı ortalama fill fiyatı
    pub avg_fill_price: f64,
    /// Mid-price'tan sapma (slippage %)
    pub slippage_pct: f64,
    /// Kalan doldurulmayan miktar (tam fill ise 0)
    pub unfilled_qty: f64,
    /// Tüketilen seviyelerin detayı
    pub fills: Vec<PartialFill>,
    /// Tam fill gerçekleşti mi?
    pub is_full_fill: bool,
}

/// Tek bir seviyde yapılan kısmi fill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialFill {
    pub price: f64,
    pub qty:   f64,
    pub notional: f64,
}

// ─── OrderBook ────────────────────────────────────────────────────────────────

/// L2 Order Book: bids (azalan) + asks (artan)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    /// Alış tarafı — azalan sırada (en iyi bid [0])
    pub bids: Vec<BookLevel>,
    /// Satış tarafı — artan sırada (en iyi ask [0])
    pub asks: Vec<BookLevel>,
    /// Referans mid-price (istatistik için)
    pub mid_price: f64,
}

impl OrderBook {
    pub fn new(bids: Vec<BookLevel>, asks: Vec<BookLevel>) -> Self {
        let mid = if let (Some(b), Some(a)) = (bids.first(), asks.first()) {
            (b.price + a.price) / 2.0
        } else {
            0.0
        };
        Self { bids, asks, mid_price: mid }
    }

    /// Best bid (en yüksek alış fiyatı)
    pub fn best_bid(&self) -> Option<f64> { self.bids.first().map(|l| l.price) }
    /// Best ask (en düşük satış fiyatı)
    pub fn best_ask(&self) -> Option<f64> { self.asks.first().map(|l| l.price) }
    /// Spread (%)
    pub fn spread_pct(&self) -> f64 {
        match (self.best_bid(), self.best_ask()) {
            (Some(b), Some(a)) if b > 0.0 => (a - b) / b * 100.0,
            _ => 0.0,
        }
    }
    /// Toplam bid likiditesi (base ccy)
    pub fn total_bid_liquidity(&self) -> f64 { self.bids.iter().map(|l| l.qty).sum() }
    /// Toplam ask likiditesi (base ccy)
    pub fn total_ask_liquidity(&self) -> f64 { self.asks.iter().map(|l| l.qty).sum() }

    /// Piyasa alış emri (qty: base currency)
    /// Ask seviyelerini düşükten yukarı tüketir.
    pub fn fill_market_buy(&self, qty: f64) -> FillResult {
        self.fill_order(qty, &self.asks, true)
    }

    /// Piyasa satış emri (qty: base currency)
    /// Bid seviyelerini yüksekten aşağı tüketir.
    pub fn fill_market_sell(&self, qty: f64) -> FillResult {
        self.fill_order(qty, &self.bids, false)
    }

    fn fill_order(&self, qty: f64, levels: &[BookLevel], is_buy: bool) -> FillResult {
        let mut remaining   = qty;
        let mut total_cost  = 0.0;
        let mut total_filled= 0.0;
        let mut fills       = Vec::new();

        for level in levels {
            if remaining <= 1e-10 { break; }
            let take = remaining.min(level.qty);
            let notional = take * level.price;
            fills.push(PartialFill { price: level.price, qty: take, notional });
            total_cost   += notional;
            total_filled += take;
            remaining    -= take;
        }

        let avg_price = if total_filled > 0.0 { total_cost / total_filled } else { 0.0 };
        let slippage  = if self.mid_price > 0.0 && avg_price > 0.0 {
            if is_buy {
                (avg_price - self.mid_price) / self.mid_price * 100.0
            } else {
                (self.mid_price - avg_price) / self.mid_price * 100.0
            }
        } else {
            0.0
        };

        FillResult {
            requested_qty:  qty,
            filled_qty:     total_filled,
            avg_fill_price: avg_price,
            slippage_pct:   slippage.max(0.0),
            unfilled_qty:   remaining,
            is_full_fill:   remaining <= 1e-10,
            fills,
        }
    }
}

// ─── Sentetik Book Üretici ────────────────────────────────────────────────────

/// Gerçek book verisi olmadığında sentetik L2 book oluşturur.
/// Binance tipi eksponansiyel derinlik dağılımını yaklaşık modeller.
#[derive(Debug, Clone)]
pub struct SyntheticBookConfig {
    /// Mid-price
    pub mid_price: f64,
    /// Kaç seviye oluşturulsun (her taraf)
    pub depth_levels: usize,
    /// En iyi bid-ask spread (%)
    pub spread_pct: f64,
    /// Her seviyede fiyat adımı (mid-price'ın %'si)
    pub tick_pct: f64,
    /// En iyi seviyedeki miktar (base ccy)
    pub top_qty: f64,
    /// Her seviyede miktar büyüme çarpanı (>1 → derin book)
    pub qty_multiplier: f64,
}

impl SyntheticBookConfig {
    /// Binance BTC/USDT benzeri likit book
    pub fn liquid(mid_price: f64) -> Self {
        Self {
            mid_price,
            depth_levels:    10,
            spread_pct:      0.02,
            tick_pct:        0.01,
            top_qty:          1.0,   // 1 BTC en iyi seviyede
            qty_multiplier:   1.5,
        }
    }

    /// Düşük likidite (altcoin)
    pub fn illiquid(mid_price: f64) -> Self {
        Self {
            mid_price,
            depth_levels:    5,
            spread_pct:      0.15,
            tick_pct:        0.05,
            top_qty:          0.1,
            qty_multiplier:   1.3,
        }
    }
}

/// Sentetik book oluştur
pub fn build_synthetic_book(cfg: &SyntheticBookConfig) -> OrderBook {
    let half_spread = cfg.mid_price * cfg.spread_pct / 200.0;
    let best_ask    = cfg.mid_price + half_spread;
    let best_bid    = cfg.mid_price - half_spread;
    let tick        = cfg.mid_price * cfg.tick_pct / 100.0;

    let mut asks = Vec::with_capacity(cfg.depth_levels);
    let mut bids = Vec::with_capacity(cfg.depth_levels);
    let mut qty  = cfg.top_qty;

    for i in 0..cfg.depth_levels {
        asks.push(BookLevel { price: best_ask + i as f64 * tick, qty });
        bids.push(BookLevel { price: best_bid - i as f64 * tick, qty });
        qty *= cfg.qty_multiplier;
    }

    OrderBook::new(bids, asks)
}

// ─── OrderBook Simülatörü (tam entegrasyon) ───────────────────────────────────

/// Bir trade için gerçekçi fill simülasyonu yapar.
/// paper_executor ile kombinlenebilir: bu modül fill fiyatını,
/// paper_executor komisyon+impact maliyetlerini hesaplar.
pub struct OrderBookSimulator {
    pub config: SyntheticBookConfig,
}

impl OrderBookSimulator {
    pub fn new(config: SyntheticBookConfig) -> Self {
        Self { config }
    }

    /// Mevcut mid-price'ı güncelle (her mum kapanışında çağrılmalı)
    pub fn update_price(&mut self, new_mid: f64) {
        self.config.mid_price = new_mid;
    }

    /// Market alış emrinin gerçekleşeceği fiyatı simüle et
    pub fn simulate_buy(&self, qty: f64) -> FillResult {
        let book = build_synthetic_book(&self.config);
        book.fill_market_buy(qty)
    }

    /// Market satış emrinin gerçekleşeceği fiyatı simüle et
    pub fn simulate_sell(&self, qty: f64) -> FillResult {
        let book = build_synthetic_book(&self.config);
        book.fill_market_sell(qty)
    }

    /// Notional bazlı alış (kaç $'lık alacağız)
    pub fn simulate_buy_notional(&self, notional_usd: f64) -> FillResult {
        let qty = if self.config.mid_price > 0.0 {
            notional_usd / self.config.mid_price
        } else {
            0.0
        };
        self.simulate_buy(qty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_book() -> OrderBook {
        // Basit 3-seviyeli test book'u
        // Asks: 100.1, 100.2, 100.3 (her biri 1 BTC)
        // Bids: 99.9, 99.8, 99.7
        let asks = vec![
            BookLevel { price: 100.1, qty: 1.0 },
            BookLevel { price: 100.2, qty: 2.0 },
            BookLevel { price: 100.3, qty: 3.0 },
        ];
        let bids = vec![
            BookLevel { price: 99.9, qty: 1.0 },
            BookLevel { price: 99.8, qty: 2.0 },
            BookLevel { price: 99.7, qty: 3.0 },
        ];
        OrderBook::new(bids, asks)
    }

    #[test]
    fn test_best_bid_ask() {
        let book = make_book();
        assert_eq!(book.best_ask(), Some(100.1));
        assert_eq!(book.best_bid(), Some(99.9));
        assert!((book.spread_pct() - 0.2002).abs() < 0.001);
    }

    #[test]
    fn test_market_buy_single_level() {
        let book = make_book();
        let fill = book.fill_market_buy(0.5); // 0.5 BTC → tamamen 100.1'den alınır
        assert!(fill.is_full_fill);
        assert!((fill.avg_fill_price - 100.1).abs() < 1e-9);
        assert!((fill.filled_qty - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_market_buy_multi_level() {
        let book = make_book();
        let fill = book.fill_market_buy(2.5); // 1@100.1 + 1.5@100.2 → ortalama fiyat
        assert!(fill.is_full_fill);
        assert_eq!(fill.fills.len(), 2);
        let expected_avg = (1.0 * 100.1 + 1.5 * 100.2) / 2.5;
        assert!((fill.avg_fill_price - expected_avg).abs() < 1e-6);
        assert!(fill.slippage_pct >= 0.0);
    }

    #[test]
    fn test_market_buy_partial_fill() {
        let book = make_book();
        let fill = book.fill_market_buy(10.0); // book'ta toplam 6 BTC var
        assert!(!fill.is_full_fill);
        assert!((fill.filled_qty - 6.0).abs() < 1e-9);
        assert!((fill.unfilled_qty - 4.0).abs() < 1e-9);
    }

    #[test]
    fn test_market_sell() {
        let book = make_book();
        let fill = book.fill_market_sell(1.0); // 1 BTC → 99.9'dan satılır
        assert!(fill.is_full_fill);
        assert!((fill.avg_fill_price - 99.9).abs() < 1e-9);
    }

    #[test]
    fn test_synthetic_book_liquid() {
        let book = build_synthetic_book(&SyntheticBookConfig::liquid(50_000.0));
        assert_eq!(book.asks.len(), 10);
        assert_eq!(book.bids.len(), 10);
        // Best ask > mid > best bid
        assert!(book.best_ask().unwrap() > 50_000.0);
        assert!(book.best_bid().unwrap() < 50_000.0);
        // Spread %0.02 civarında
        assert!(book.spread_pct() < 0.05);
    }

    #[test]
    fn test_simulator_buy_notional() {
        let sim = OrderBookSimulator::new(SyntheticBookConfig::liquid(100.0));
        let fill = sim.simulate_buy_notional(50.0); // 50$ lık alış = 0.5 BTC
        assert!(fill.filled_qty > 0.0);
        assert!(fill.avg_fill_price > 100.0); // ask tarafı > mid
    }
}
