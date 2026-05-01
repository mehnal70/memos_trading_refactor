// robot/order_management/paper_executor.rs
//
// Gerçekçi paper trading simülatörü.
//
// Teori → Kod bağlantısı:
//
//   Strateji sinyali (beklenen fiyat: P₀)
//         ↓
//   Bilgi sızıntısı  → P₀ × market_impact_pct          (HFT emir niyetini fark eder,
//         ↓              [√(notional/ref_notional) ile   fiyatı aleyhimize iter — alışta
//                         ölçeklenir]                     yukari, satışta aşağı)
//   Spread maliyeti  → P₀ × spread_pct/2               (market emrinde ask tarafına geçersin)
//         ↓
//   Slippage         → P₀ × slippage_pct               (büyük emir order book'u iter)
//         ↓
//   Gerçekleşen fiyat P₁ = P₀ × (1 + impact + spread/2 + slippage)   [alış]
//                     P₁ = P₀ × (1 - impact - spread/2 - slippage)   [satış]
//         ↓
//   Komisyon         → notional × commission_pct   (her iki tarafta)
//
// Bilgi sızıntısı (Information Leakage / Market Impact) nedir?
//   10 BTC almak için emir hazırladığınızda, order book'u izleyen HFT botlar
//   bu niyeti (large order, urgency sinyali) okur ve fiyatı yukarı çeker.
//   Siz daha yüksek fiyata alırsınız — bu "önceden yerleşmiş" olumsuz fiyat
//   hareketinin maliyetidir. Slippage'dan FARKLIDIR: slippage emir sırasında
//   oluşur, impact emir öncesi/sırasında piyasanın tepkisidir.
//
//   Standart mikroyapı modeli (Almgren-Chriss):
//     impact_pct = market_impact_factor × √(notional / ref_notional)
//   ref_notional = "ortalama işlem büyüklüğü" kalibrasyonu ($10_000)
//   Binance BTC/USDT için tipik değer aralığı:
//     $1k  emir → ~0.001–0.005%  (ihmal edilebilir)
//     $10k emir → ~0.003–0.015%
//     $100k emir→ ~0.01–0.05%
//     $1M  emir → ~0.03–0.15%
//
// Paper-to-live gap:
//   Komisyon simüle edilmezse: 100 işlem × %0.2 komisyon = %20 fazla kâr görünür.
//   Slippage+impact simüle edilmezse: MA Crossover gibi sık işlem yapan stratejilerde
//   her işlemde %0.05-0.15 birikerek toplam getiriyi silip süpürür.

use crate::types::Trade;
use crate::Result;
use chrono::Utc;
use super::orderbook_sim::{OrderBookSimulator, SyntheticBookConfig};

// ─── Execution Cost Config ────────────────────────────────────────────────────

/// İşlem maliyeti parametreleri.
/// Exchange ve piyasa koşuluna göre değişir.
#[derive(Debug, Clone)]
pub struct ExecutionCostConfig {
    /// Spread simülasyonu — market emrinde spread'in yarısını ödersin.
    /// Ask tarafına geçiş = half-spread maliyeti.
    ///   Binance BTC/USDT spot (likit): ~0.01%
    ///   Daha az likit altcoin'ler:      ~0.05–0.1%
    pub spread_pct: f64,

    /// Slippage — emir büyüklüğü order book'u ne kadar iter.
    ///   Küçük emirler (< $1000 notional):  ~0.01–0.05%
    ///   Orta emirler  ($1000–$10000):       ~0.05–0.1%
    ///   Büyük emirler (> $10000):           ~0.1–0.5%
    pub slippage_pct: f64,

    /// Komisyon — exchange ücreti, notional üzerinden.
    ///   Binance spot  (VIP0): %0.10 maker, %0.10 taker → market emrinde %0.10
    ///   Binance futures(VIP0): %0.02 maker, %0.04 taker → market emrinde %0.04
    ///   BNB ile ödeme:         %25 indirim → spot %0.075, futures %0.03
    pub commission_pct: f64,

    /// Bilgi sızıntısı / market impact katsayısı.
    /// Standart √-model:  impact_pct = market_impact_factor × √(notional / ref_notional)
    /// HFT'nin emir niyetini okuyarak fiyatı aleyhimize itmesini modeller.
    ///   Binance BTC (likit):     ~0.01   → $10k emirde ~%0.003
    ///   Altcoin (daha az likit): ~0.05   → $10k emirde ~%0.016
    ///   0.0 = bilgi sızıntısı yok (eski davranış)
    pub market_impact_factor: f64,

    /// Referans notional ($) — impact ölçekleme kalibrasyon noktası.
    /// "Tipik" bir işlem büyüklüğü olarak düşün.
    ///   Binance BTC/USDT için makul: $10_000
    pub ref_notional: f64,
}

impl ExecutionCostConfig {
    /// Binance Spot — VIP0, market order, BNB ödemesiz
    /// Küçük-orta pozisyon ($1k–$10k), likit BTC/USDT çifti
    pub fn binance_spot() -> Self {
        Self {
            spread_pct:           0.02,
            slippage_pct:         0.03,
            commission_pct:       0.10,
            market_impact_factor: 0.01,   // BTC/USDT likit → düşük impact
            ref_notional:         10_000.0,
        }
    }

    /// Binance Futures — VIP0, market order, BNB ödemesiz
    pub fn binance_futures() -> Self {
        Self {
            spread_pct:           0.01,
            slippage_pct:         0.02,
            commission_pct:       0.04,
            market_impact_factor: 0.008,  // futures daha derin order book
            ref_notional:         10_000.0,
        }
    }

    /// Daha az likit altcoin için yüksek impact preset
    pub fn altcoin_spot() -> Self {
        Self {
            spread_pct:           0.08,
            slippage_pct:         0.10,
            commission_pct:       0.10,
            market_impact_factor: 0.05,   // ince order book → HFT etkisi büyük
            ref_notional:         5_000.0,
        }
    }

    /// Sıfır maliyet — eski davranış, karşılaştırma için
    pub fn zero() -> Self {
        Self {
            spread_pct:           0.0,
            slippage_pct:         0.0,
            commission_pct:       0.0,
            market_impact_factor: 0.0,
            ref_notional:         10_000.0,
        }
    }

    /// Bilgi sızıntısı (market impact) yüzdesini hesapla.
    /// Almgren-Chriss √-modeli:  impact = factor × √(notional / ref_notional)
    /// Büyük emirler orantısız daha fazla sızdırır — √ bunu yakalar.
    pub fn market_impact_pct(&self, notional: f64) -> f64 {
        if self.market_impact_factor == 0.0 || self.ref_notional <= 0.0 {
            return 0.0;
        }
        self.market_impact_factor * (notional / self.ref_notional).sqrt()
    }

    /// Toplam round-trip (alış + satış) maliyet yüzdesi (sabit bileşenler).
    /// Notional'a bağlı market_impact dahil değil — `round_trip_with_impact` kullan.
    pub fn round_trip_cost_pct(&self) -> f64 {
        // Alış:  impact + spread/2 + slippage + komisyon
        // Satış: impact + spread/2 + slippage + komisyon
        // (impact'ı ref_notional ile normalize edilmiş olarak ekle)
        self.spread_pct
            + self.slippage_pct * 2.0
            + self.commission_pct * 2.0
            + self.market_impact_factor * 2.0   // ref_notional'da normalize değer
    }

    /// Belirli notional için gerçek round-trip maliyeti
    pub fn round_trip_with_impact(&self, notional: f64) -> f64 {
        let impact = self.market_impact_pct(notional);
        self.spread_pct + self.slippage_pct * 2.0 + self.commission_pct * 2.0 + impact * 2.0
    }

    /// Bir işlemin kâra geçebilmesi için minimum fiyat hareketi (break-even)
    pub fn break_even_pct(&self) -> f64 {
        self.round_trip_cost_pct()
    }
}

impl Default for ExecutionCostConfig {
    fn default() -> Self {
        Self::binance_spot()
    }
}

// ─── Execution Cost Report ────────────────────────────────────────────────────

/// Tek bir işlemin gerçekleşme maliyeti özeti.
#[derive(Debug, Clone)]
pub struct ExecutionCostBreakdown {
    pub expected_price:        f64,   // stratejinin gördüğü fiyat
    pub executed_price:        f64,   // gerçekleşen fiyat (tüm etkiler sonrası)
    pub market_impact_cost_usd:f64,   // bilgi sızıntısı maliyeti ($) — HFT front-run etkisi
    pub spread_cost_usd:       f64,   // spread maliyeti ($)
    pub slippage_cost_usd:     f64,   // slippage maliyeti ($)
    pub commission_usd:        f64,   // exchange komisyonu ($)
    pub total_cost_usd:        f64,   // toplam maliyet ($)
    pub total_cost_pct:        f64,   // toplam maliyet (%)
}

// ─── Open Position ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct OpenPosition {
    symbol:        String,
    entry_price:   f64,   // gerçekleşen giriş fiyatı (spread+slippage uygulanmış)
    #[allow(dead_code)]
    ideal_price:   f64,   // stratejinin beklediği fiyat (spread/slippage öncesi)
    amount:        f64,
    entry_time:    chrono::DateTime<Utc>,
    entry_cost:    ExecutionCostBreakdown,
}

// ─── PaperTradingExecutor ─────────────────────────────────────────────────────

pub struct PaperTradingExecutor {
    balance:         f64,
    initial_balance: f64,
    trades:          Vec<Trade>,
    open_position:   Option<OpenPosition>,
    cost_config:     ExecutionCostConfig,

    // Kümülatif maliyet takibi — paper-to-live gap analizi için
    total_commission_paid:    f64,
    total_spread_cost:        f64,
    total_slippage_cost:      f64,
    total_market_impact_cost: f64,   // bilgi sızıntısı kümülatifi
    trade_count:              usize,

    /// L2 order book simülatörü — set edilirse slippage_pct yerine gerçek
    /// seviye yürüyüşü kullanılır. `None` ise Almgren-Chriss yüzde modeli.
    orderbook_sim:   Option<OrderBookSimulator>,
}

impl PaperTradingExecutor {
    pub fn new(initial_balance: f64) -> Self {
        Self::with_costs(initial_balance, ExecutionCostConfig::default())
    }

    pub fn with_costs(initial_balance: f64, cost_config: ExecutionCostConfig) -> Self {
        Self {
            balance: initial_balance,
            initial_balance,
            trades: vec![],
            open_position: None,
            cost_config,
            total_commission_paid:    0.0,
            total_spread_cost:        0.0,
            total_slippage_cost:      0.0,
            total_market_impact_cost: 0.0,
            trade_count:              0,
            orderbook_sim:            None,
        }
    }

    /// L2 order book simülatörü ile başlat.
    /// `mid_price`: mevcut orta fiyat; kitap dinamik olarak güncellenir.
    /// `liquid`: true → BTC/USDT tipi derin kitap, false → ince altcoin kitabı.
    pub fn with_orderbook(initial_balance: f64, cost_config: ExecutionCostConfig,
                          mid_price: f64, liquid: bool) -> Self {
        let book_cfg = if liquid {
            SyntheticBookConfig::liquid(mid_price)
        } else {
            SyntheticBookConfig::illiquid(mid_price)
        };
        let sim = OrderBookSimulator::new(book_cfg);
        Self {
            balance: initial_balance,
            initial_balance,
            trades: vec![],
            open_position: None,
            cost_config,
            total_commission_paid:    0.0,
            total_spread_cost:        0.0,
            total_slippage_cost:      0.0,
            total_market_impact_cost: 0.0,
            trade_count:              0,
            orderbook_sim:            Some(sim),
        }
    }

    /// Fiyat değişince order book kitabını güncelle (L2 modu için).
    /// Almgren-Chriss modunda no-op.
    pub fn update_price(&mut self, new_price: f64) {
        if let Some(sim) = &mut self.orderbook_sim {
            sim.update_price(new_price);
        }
    }

    pub fn balance(&self) -> f64 { self.balance }
    pub fn initial_balance(&self) -> f64 { self.initial_balance }
    pub fn trades(&self) -> &[Trade] { &self.trades }
    pub fn has_open_position(&self) -> bool { self.open_position.is_some() }

    pub fn total_return_pct(&self) -> f64 {
        (self.balance - self.initial_balance) / self.initial_balance * 100.0
    }

    pub fn realized_pnl(&self) -> f64 {
        self.trades.iter().filter_map(|t| t.pnl).sum()
    }

    pub fn win_rate(&self) -> f64 {
        if self.trades.is_empty() { return 0.0; }
        let wins = self.trades.iter().filter(|t| t.pnl.unwrap_or(0.0) > 0.0).count();
        wins as f64 / self.trades.len() as f64 * 100.0
    }

    pub fn unrealized_pnl(&self, current_price: f64) -> f64 {
        if let Some(pos) = &self.open_position {
            (current_price - pos.entry_price) * pos.amount
        } else { 0.0 }
    }

    // ── Execution Cost Helpers ────────────────────────────────────────────────

    /// Alış emrinin gerçekleşen fiyatını hesapla.
    /// Sıralama: bilgi sızıntısı (pre-trade) → spread → slippage
    ///   impact    : HFT emir niyetini okur, fiyat yukarı iter (√-model)
    ///   half-spread: market emrinde ask tarafına geçiş
    ///   slippage  : büyük emir order book'u tüketir
    fn apply_buy_leakage(&self, ideal_price: f64, notional: f64) -> f64 {
        let impact_pct    = self.cost_config.market_impact_pct(notional);
        let impact        = ideal_price * impact_pct    / 100.0;
        let spread_impact = ideal_price * self.cost_config.spread_pct   / 200.0; // half-spread
        let slip_impact   = ideal_price * self.cost_config.slippage_pct / 100.0;
        ideal_price + impact + spread_impact + slip_impact
    }

    /// Satış emrinin gerçekleşen fiyatını hesapla.
    ///   impact    : HFT satış niyetini görür, fiyat aşağı iter
    ///   half-spread: bid tarafına geçiş
    ///   slippage  : emir kitabını tüketir
    fn apply_sell_leakage(&self, ideal_price: f64, notional: f64) -> f64 {
        let impact_pct    = self.cost_config.market_impact_pct(notional);
        let impact        = ideal_price * impact_pct    / 100.0;
        let spread_impact = ideal_price * self.cost_config.spread_pct   / 200.0;
        let slip_impact   = ideal_price * self.cost_config.slippage_pct / 100.0;
        ideal_price - impact - spread_impact - slip_impact
    }

    /// Notional üzerinden komisyon ($)
    fn commission(&self, price: f64, amount: f64) -> f64 {
        price * amount * self.cost_config.commission_pct / 100.0
    }

    /// Alış için tam maliyet dökümü — her bileşen ayrı izlenir
    fn buy_cost_breakdown(&self, ideal_price: f64, amount: f64) -> ExecutionCostBreakdown {
        let notional               = ideal_price * amount;
        let impact_pct             = self.cost_config.market_impact_pct(notional);
        let market_impact_cost_usd = ideal_price * impact_pct / 100.0 * amount;

        let executed_price     = self.apply_buy_leakage(ideal_price, notional);
        let price_diff         = executed_price - ideal_price;
        // impact çıkartılınca kalan fark spread+slippage
        let remaining_diff     = price_diff - ideal_price * impact_pct / 100.0;
        let spread_cost_usd    = remaining_diff * amount * self.cost_config.spread_pct
                                  / (self.cost_config.spread_pct / 200.0
                                     + self.cost_config.slippage_pct / 100.0
                                     + f64::EPSILON)
                                  / 200.0; // orantılı paylaştır
        let slippage_cost_usd  = remaining_diff * amount - spread_cost_usd;
        let commission_usd     = self.commission(executed_price, amount);
        let total_cost_usd     = market_impact_cost_usd + spread_cost_usd
                                  + slippage_cost_usd + commission_usd;
        let total_cost_pct     = total_cost_usd / notional * 100.0;

        ExecutionCostBreakdown {
            expected_price: ideal_price,
            executed_price,
            market_impact_cost_usd,
            spread_cost_usd,
            slippage_cost_usd,
            commission_usd,
            total_cost_usd,
            total_cost_pct,
        }
    }

    /// Satış için tam maliyet dökümü
    fn sell_cost_breakdown(&self, ideal_price: f64, amount: f64) -> ExecutionCostBreakdown {
        let notional               = ideal_price * amount;
        let impact_pct             = self.cost_config.market_impact_pct(notional);
        let market_impact_cost_usd = ideal_price * impact_pct / 100.0 * amount;

        let executed_price     = self.apply_sell_leakage(ideal_price, notional);
        let price_diff         = ideal_price - executed_price;
        let remaining_diff     = price_diff - ideal_price * impact_pct / 100.0;
        let spread_cost_usd    = remaining_diff * amount * self.cost_config.spread_pct
                                  / (self.cost_config.spread_pct / 200.0
                                     + self.cost_config.slippage_pct / 100.0
                                     + f64::EPSILON)
                                  / 200.0;
        let slippage_cost_usd  = remaining_diff * amount - spread_cost_usd;
        let commission_usd     = self.commission(executed_price, amount);
        let total_cost_usd     = market_impact_cost_usd + spread_cost_usd
                                  + slippage_cost_usd + commission_usd;
        let total_cost_pct     = total_cost_usd / notional * 100.0;

        ExecutionCostBreakdown {
            expected_price: ideal_price,
            executed_price,
            market_impact_cost_usd,
            spread_cost_usd,
            slippage_cost_usd,
            commission_usd,
            total_cost_usd,
            total_cost_pct,
        }
    }

    // ── Ana İşlemler ──────────────────────────────────────────────────────────

    /// Alış emri — spread + slippage + komisyon uygulanır.
    /// L2 modu aktifse slippage order book seviye yürüyüşüyle hesaplanır.
    pub fn buy(&mut self, symbol: &str, ideal_price: f64, amount: f64) -> Result<ExecutionCostBreakdown> {
        if self.open_position.is_some() {
            return Err("Zaten açık bir pozisyon var".into());
        }

        // L2 order book modu: slippage_pct'yi book'tan türet
        let breakdown = if let Some(sim) = &mut self.orderbook_sim {
            let fill = sim.simulate_buy(amount);
            if fill.avg_fill_price > 0.0 {
                // Book tabanlı executed_price; impact + komisyon Almgren-Chriss gibi hesaplanır
                let notional           = ideal_price * amount;
                let impact_pct         = self.cost_config.market_impact_pct(notional);
                let market_impact_cost = ideal_price * impact_pct / 100.0 * amount;
                let spread_cost        = ideal_price * self.cost_config.spread_pct / 200.0 * amount;
                let slippage_cost      = (fill.avg_fill_price - ideal_price) * amount;
                let executed_price     = fill.avg_fill_price
                    + ideal_price * impact_pct / 100.0
                    + ideal_price * self.cost_config.spread_pct / 200.0;
                let commission_usd     = self.cost_config.commission_pct / 100.0 * executed_price * amount;
                let total_cost_usd     = market_impact_cost + spread_cost + slippage_cost.max(0.0) + commission_usd;
                ExecutionCostBreakdown {
                    expected_price:         ideal_price,
                    executed_price,
                    market_impact_cost_usd: market_impact_cost,
                    spread_cost_usd:        spread_cost,
                    slippage_cost_usd:      slippage_cost.max(0.0),
                    commission_usd,
                    total_cost_usd,
                    total_cost_pct:         total_cost_usd / notional * 100.0,
                }
            } else {
                self.buy_cost_breakdown(ideal_price, amount)
            }
        } else {
            self.buy_cost_breakdown(ideal_price, amount)
        };
        let total_cost = breakdown.executed_price * amount + breakdown.commission_usd;

        if total_cost > self.balance {
            return Err(format!(
                "Yetersiz bakiye: gerekli={:.2} mevcut={:.2}",
                total_cost, self.balance
            ).into());
        }

        // Maliyetleri bakiyeden düş
        self.balance -= total_cost;

        // Kümülatif maliyet takibi
        self.total_market_impact_cost += breakdown.market_impact_cost_usd;
        self.total_spread_cost        += breakdown.spread_cost_usd;
        self.total_slippage_cost      += breakdown.slippage_cost_usd;
        self.total_commission_paid    += breakdown.commission_usd;

        self.open_position = Some(OpenPosition {
            symbol:      symbol.to_string(),
            entry_price: breakdown.executed_price,
            ideal_price,
            amount,
            entry_time:  Utc::now(),
            entry_cost:  breakdown.clone(),
        });

        Ok(breakdown)
    }

    /// Pozisyonu kapat — satış tarafına da spread + slippage + komisyon uygulanır.
    /// L2 modu aktifse slippage order book seviye yürüyüşüyle hesaplanır.
    pub fn close_position(&mut self, ideal_exit_price: f64) -> Result<(Trade, ExecutionCostBreakdown)> {
        let pos = self.open_position.take()
            .ok_or("Açık pozisyon yok")?;

        let exit_breakdown = if let Some(sim) = &mut self.orderbook_sim {
            let fill = sim.simulate_sell(pos.amount);
            if fill.avg_fill_price > 0.0 {
                let notional           = ideal_exit_price * pos.amount;
                let impact_pct         = self.cost_config.market_impact_pct(notional);
                let market_impact_cost = ideal_exit_price * impact_pct / 100.0 * pos.amount;
                let spread_cost        = ideal_exit_price * self.cost_config.spread_pct / 200.0 * pos.amount;
                let slippage_cost      = (ideal_exit_price - fill.avg_fill_price) * pos.amount;
                let executed_price     = fill.avg_fill_price
                    - ideal_exit_price * impact_pct / 100.0
                    - ideal_exit_price * self.cost_config.spread_pct / 200.0;
                let commission_usd     = self.cost_config.commission_pct / 100.0 * executed_price.max(0.0) * pos.amount;
                let total_cost_usd     = market_impact_cost + spread_cost + slippage_cost.max(0.0) + commission_usd;
                ExecutionCostBreakdown {
                    expected_price:         ideal_exit_price,
                    executed_price,
                    market_impact_cost_usd: market_impact_cost,
                    spread_cost_usd:        spread_cost,
                    slippage_cost_usd:      slippage_cost.max(0.0),
                    commission_usd,
                    total_cost_usd,
                    total_cost_pct:         total_cost_usd / notional * 100.0,
                }
            } else {
                self.sell_cost_breakdown(ideal_exit_price, pos.amount)
            }
        } else {
            self.sell_cost_breakdown(ideal_exit_price, pos.amount)
        };

        // Satış geliri — komisyon düşülmüş
        let proceeds = exit_breakdown.executed_price * pos.amount - exit_breakdown.commission_usd;
        self.balance += proceeds;

        // Kümülatif maliyet takibi
        self.total_market_impact_cost += exit_breakdown.market_impact_cost_usd;
        self.total_spread_cost        += exit_breakdown.spread_cost_usd;
        self.total_slippage_cost      += exit_breakdown.slippage_cost_usd;
        self.total_commission_paid    += exit_breakdown.commission_usd;
        self.trade_count              += 1;

        // PnL: gerçekleşen giriş/çıkış fiyatlarına göre
        let pnl     = (exit_breakdown.executed_price - pos.entry_price) * pos.amount
                      - pos.entry_cost.commission_usd
                      - exit_breakdown.commission_usd;
        let pnl_pct = pnl / (pos.entry_price * pos.amount) * 100.0;

        let trade = Trade {
            id:          Some(self.trades.len() as u64 + 1),
            symbol:      pos.symbol,
            entry_price: pos.entry_price,
            exit_price:  Some(exit_breakdown.executed_price),
            amount:      pos.amount,
            entry_time:  pos.entry_time,
            exit_time:   Some(Utc::now()),
            pnl:         Some(pnl),
            pnl_pct:     Some(pnl_pct),
            strategy:    "paper".to_string(),
        };

        self.trades.push(trade.clone());
        Ok((trade, exit_breakdown))
    }

    // ── Paper-to-Live Gap Raporu ──────────────────────────────────────────────

    /// Gerçekçi simülasyon ile sıfır-maliyetli simülasyon arasındaki farkı gösterir.
    /// Bu fark backtest'te görünen kâr ile gerçek kârın neden farklı olduğunu açıklar.
    pub fn execution_cost_report(&self) -> ExecutionCostReport {
        let total_cost = self.total_commission_paid
            + self.total_spread_cost
            + self.total_slippage_cost
            + self.total_market_impact_cost;

        // "Sıfır maliyet" varsayımıyla olması gereken bakiye
        let ideal_balance = self.initial_balance + self.realized_pnl() + total_cost;

        ExecutionCostReport {
            trade_count:               self.trade_count,
            total_commission_paid:     self.total_commission_paid,
            total_spread_cost:         self.total_spread_cost,
            total_slippage_cost:       self.total_slippage_cost,
            total_market_impact_cost:  self.total_market_impact_cost,
            total_cost_usd:            total_cost,
            total_cost_pct:            total_cost / self.initial_balance * 100.0,
            paper_to_live_gap_usd:     ideal_balance - self.balance,
            avg_cost_per_trade:        if self.trade_count > 0 {
                total_cost / self.trade_count as f64
            } else { 0.0 },
        }
    }
}

/// Paper-to-live gap özet raporu.
/// Backtest kârının gerçekte neden daha düşük olduğunu sayısal olarak gösterir.
#[derive(Debug, Clone)]
pub struct ExecutionCostReport {
    pub trade_count:              usize,
    pub total_commission_paid:    f64,
    pub total_spread_cost:        f64,
    pub total_slippage_cost:      f64,
    pub total_market_impact_cost: f64,  // bilgi sızıntısı kümülatifi
    pub total_cost_usd:           f64,
    pub total_cost_pct:           f64,  // başlangıç sermayesine göre
    pub paper_to_live_gap_usd:    f64,  // sıfır-maliyet ile gerçekçi arasındaki fark
    pub avg_cost_per_trade:       f64,
}

impl std::fmt::Display for ExecutionCostReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f,
            "İşlem={} | Komisyon=${:.2} Spread=${:.2} Slippage=${:.2} Impact=${:.2} \
             | Toplam=${:.2} ({:.3}%) | Paper-Live Fark=${:.2} | Ort/İşlem=${:.2}",
            self.trade_count,
            self.total_commission_paid,
            self.total_spread_cost,
            self.total_slippage_cost,
            self.total_market_impact_cost,
            self.total_cost_usd,
            self.total_cost_pct,
            self.paper_to_live_gap_usd,
            self.avg_cost_per_trade,
        )
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn zero_cost_exec() -> PaperTradingExecutor {
        PaperTradingExecutor::with_costs(10_000.0, ExecutionCostConfig::zero())
    }

    fn realistic_exec() -> PaperTradingExecutor {
        PaperTradingExecutor::with_costs(10_000.0, ExecutionCostConfig::binance_spot())
    }

    #[test]
    fn test_zero_cost_profitable() {
        let mut exec = zero_cost_exec();
        exec.buy("BTCUSDT", 100.0, 10.0).unwrap();
        let (trade, _) = exec.close_position(110.0).unwrap();
        assert!((trade.pnl.unwrap() - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_commission_reduces_profit() {
        let mut exec = realistic_exec();
        exec.buy("BTCUSDT", 100.0, 10.0).unwrap();
        let (trade, _) = exec.close_position(110.0).unwrap();
        // Komisyon her iki tarafta da alınır → gerçek kâr < 100
        let pnl = trade.pnl.unwrap();
        assert!(pnl < 100.0, "Komisyon kârı düşürmeli: {:.4}", pnl);
    }

    #[test]
    fn test_break_even_requires_price_move() {
        let config = ExecutionCostConfig::binance_spot();
        let break_even = config.break_even_pct();
        // Binance spot round-trip: spread(%0.02) + slippage×2(%0.06) + komisyon×2(%0.20) = ~0.28%
        assert!(break_even > 0.1, "Break-even en az %0.1 olmalı: {:.4}", break_even);
    }

    #[test]
    fn test_execution_cost_report() {
        let mut exec = realistic_exec();
        exec.buy("BTCUSDT", 50_000.0, 0.1).unwrap();
        exec.close_position(50_500.0).unwrap();
        let report = exec.execution_cost_report();
        assert!(report.total_cost_usd > 0.0);
        assert_eq!(report.trade_count, 1);
    }

    #[test]
    fn test_paper_to_live_gap_accumulates() {
        // 100 işlem × spread/slippage/komisyon birikimi
        let mut exec = realistic_exec();
        for _ in 0..10 {
            exec.buy("BTCUSDT", 100.0, 1.0).unwrap();
            exec.close_position(101.5).unwrap(); // %1.5 kâr
        }
        let report = exec.execution_cost_report();
        // Toplam maliyet sıfırdan büyük olmalı
        assert!(report.total_cost_usd > 0.0);
        // Paper-to-live fark: komisyonsuz hesapta daha fazla kâr görünür
        assert!(report.paper_to_live_gap_usd >= 0.0);
    }

    #[test]
    fn test_insufficient_balance() {
        let mut exec = PaperTradingExecutor::new(100.0);
        assert!(exec.buy("BTCUSDT", 1000.0, 1.0).is_err());
    }

    /// Bilgi sızıntısı: küçük emir vs büyük emir — √-model testi
    #[test]
    fn test_market_impact_sqrt_scaling() {
        let config = ExecutionCostConfig::binance_spot();
        // $10k emirde impact = factor × √(10k/10k) = factor × 1.0
        let impact_ref = config.market_impact_pct(10_000.0);
        assert!((impact_ref - config.market_impact_factor).abs() < 1e-9);

        // $40k emirde impact = factor × √(40k/10k) = factor × 2.0
        let impact_4x = config.market_impact_pct(40_000.0);
        assert!((impact_4x - config.market_impact_factor * 2.0).abs() < 1e-9,
            "√-model: 4× notional → 2× impact bekleniyordu, got {:.6}", impact_4x);

        // Büyük emir daha yüksek impact yaratmalı
        assert!(impact_4x > impact_ref);
    }

    /// Bilgi sızıntısı: büyük emirde alış fiyatı daha da yüksek olmalı
    #[test]
    fn test_large_order_higher_impact() {
        let config = ExecutionCostConfig::binance_spot();
        let mut small = PaperTradingExecutor::with_costs(1_000_000.0, config.clone());
        let mut large = PaperTradingExecutor::with_costs(1_000_000.0, config);

        // Küçük emir: $1k notional
        let small_bd = small.buy("BTCUSDT", 50_000.0, 0.02).unwrap();
        // Büyük emir: $500k notional (aynı fiyattan)
        let large_bd = large.buy("BTCUSDT", 50_000.0, 10.0).unwrap();

        // Büyük emirde executed_price daha yüksek olmalı (daha fazla impact)
        assert!(
            large_bd.executed_price > small_bd.executed_price,
            "Büyük emir daha yüksek impact maliyeti vermeli: small={:.2} large={:.2}",
            small_bd.executed_price, large_bd.executed_price
        );
        assert!(
            large_bd.market_impact_cost_usd > small_bd.market_impact_cost_usd,
            "Impact maliyeti ($) büyük emirde daha yüksek olmalı"
        );
    }

    /// Sıfır impact ile pozitif impact karşılaştırması
    #[test]
    fn test_impact_reduces_profit_vs_zero() {
        let price_in  = 50_000.0_f64;
        let price_out = 50_500.0_f64;
        let amount    = 0.2_f64; // $10k notional

        let mut zero    = PaperTradingExecutor::with_costs(100_000.0, ExecutionCostConfig::zero());
        let mut realistic = PaperTradingExecutor::with_costs(100_000.0, ExecutionCostConfig::binance_spot());

        zero.buy("BTCUSDT", price_in, amount).unwrap();
        let (zero_trade, _) = zero.close_position(price_out).unwrap();

        realistic.buy("BTCUSDT", price_in, amount).unwrap();
        let (real_trade, _) = realistic.close_position(price_out).unwrap();

        let zero_pnl = zero_trade.pnl.unwrap();
        let real_pnl = real_trade.pnl.unwrap();

        assert!(real_pnl < zero_pnl,
            "Gerçekçi simülasyon (impact+spread+slip+komisyon) daha düşük kâr vermeli: \
             zero={:.4} realistic={:.4}", zero_pnl, real_pnl);

        let gap = zero_pnl - real_pnl;
        let report = realistic.execution_cost_report();
        assert!(report.total_market_impact_cost > 0.0,
            "Market impact maliyeti raporlanmalı");
        println!("Paper-live gap: ${:.4} | Rapor: {}", gap, report);
    }
}
