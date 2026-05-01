// robot/position_manager.rs
//
// Açık pozisyon takibi — PositionId kimliği, trailing SL/TP, PnL hesabı.
// robotic_loop.rs monolitinden ayrıştırıldı; bağımsız test edilebilir.
//
// Dışa bağımlılıklar: yalnızca crate::types (Market, PositionId, RiskParams)

use crate::types::{Market, PositionId, RiskParams};

fn default_tp1_close_ratio() -> f64 { 0.40 }

/// Açık pozisyon takibi — döngüler arası trailing SL/TP için.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct OpenPosition {
    /// Evrensel tekil kimlik — dedup ve cross-reference için
    pub(crate) id: PositionId,
    /// Pozisyonun ait olduğu sembol (örn. "BTCUSDT")
    pub(crate) symbol: String,
    /// Pozisyonun ait olduğu market
    pub(crate) market: Market,
    /// Giriş fiyatı (spread/slippage uygulanmış etkin fiyat)
    pub(crate) entry_price: f64,
    /// İşlem miktarı (kaldıraçlı notional / entry_price)
    pub(crate) qty: f64,
    /// Yön: true = long (BUY), false = short (SELL)
    pub(crate) is_long: bool,
    /// Sabit stop-loss seviyesi (fiyat cinsinden)
    pub(crate) static_sl: f64,
    /// Sabit take-profit seviyesi (fiyat cinsinden)
    pub(crate) static_tp: f64,
    /// Trailing SL için en iyi fiyat (long'da en yüksek, short'da en düşük)
    pub(crate) best_price: f64,
    /// Şu anki trailing SL seviyesi (None = kâra geçilmedi)
    pub(crate) trailing_sl: Option<f64>,
    /// Trailing SL yüzdesi (config'den kopyalanır, None = trailing kapalı)
    /// ATR tabanlı trailing etkinse check_live_sl_tp tarafından her döngüde güncellenir.
    pub(crate) trailing_pct: Option<f64>,
    /// TSL aktif olmadan önce gerekli min kâr yüzdesi (adaptive_params'tan gelir).
    /// Örn. 1.50 → pozisyon %1.5 kârda olmadan trailing_sl set edilmez.
    /// None = herhangi bir kâr yeterli (eski davranış).
    #[serde(default)]
    pub(crate) trailing_activation_pct: Option<f64>,
    /// Uygulanan kaldıraç (1.0 = kaldıraçsız, 7.0–10.0 = dinamik)
    pub(crate) leverage: f64,
    /// Tasfiye fiyatı — long: entry×(1−0.9/lev), short: entry×(1+0.9/lev)
    /// SL her zaman bu fiyattan daha güvenli tarafta olmalı.
    pub(crate) liquidation_price: f64,
    // ── B1: Breakeven stop ────────────────────────────────────────────────────
    /// Kâr mesafesi (|entry − original_sl|) — breakeven hesabı için sabitlenir
    pub(crate) risk_distance: f64,
    /// Breakeven tetiklenme R çarpanı: kâr ≥ breakeven_at_rr × risk_distance → SL giriş fiyatına taşı
    /// Örn. 0.5 → yarı risk mesafesi kadar kârdayken SL entry'e çekil
    pub(crate) breakeven_at_rr: Option<f64>,
    /// Breakeven zaten tetiklendi mi? (tek seferlik)
    pub(crate) breakeven_triggered: bool,
    // ── B2: ATR tabanlı trailing stop ─────────────────────────────────────────
    /// ATR trailing çarpanı — None = ATR trail yok, sabit trailing_pct kullanılır
    /// check_live_sl_tp döngüsünde trailing_pct = atr_trail_mult * atr_pct olarak güncellenir
    pub(crate) atr_trail_mult: Option<f64>,
    // ── B3: Kısmi TP ──────────────────────────────────────────────────────────
    /// TP seviyesine ilk ulaşınca bu oranda pozisyon kapat (örn. 0.5 = %50)
    /// Kalan kısım trailing SL ile devam eder ve SL breakeven'a çekilir
    pub(crate) partial_tp_ratio: Option<f64>,
    /// Kısmi TP zaten gerçekleşti mi? (ikinci kez partial_tp dönmesini engeller)
    pub(crate) partial_tp_triggered: bool,
    /// Pozisyon açılış UTC damgası — süre hesabı için. Eski snapshot'larda boş string.
    #[serde(default)]
    pub(crate) opened_at: String,
    /// Açılış anındaki normalize edilmiş ML öznitelik dizisi (19 eleman).
    /// Pozisyon kapanınca gerçek PnL ile online LR eğitimi için kullanılır.
    /// None = eski snapshot'tan gelen pozisyon (öznitelik kaydedilmemiş).
    #[serde(default)]
    pub(crate) entry_features: Option<[f64; 19]>,
    // ── TP Merdiveni (TP1) ────────────────────────────────────────────────────
    /// Ara TP seviyesi — entry ile static_tp arasının %50'si.
    /// Fiyat buraya ulaşınca `tp1_close_ratio` kadar pozisyon kapatılır,
    /// SL breakeven'a çekilir, kalan trailing ile devam eder.
    /// None = TP1 kapalı (tek seviyeli TP davranışı).
    #[serde(default)]
    pub(crate) tp1_price: Option<f64>,
    /// TP1 tetiklenince kapatılacak oran (örn. 0.40 = %40).
    #[serde(default = "default_tp1_close_ratio")]
    pub(crate) tp1_close_ratio: f64,
    /// TP1 zaten gerçekleşti mi? (tek seferlik tetik)
    #[serde(default)]
    pub(crate) tp1_triggered: bool,

    /// Pozisyonu açan motor türü: Regular | Scalp | Swing
    #[serde(default)]
    pub(crate) trade_type: crate::robot::scalp_swing::TradeType,

    /// Exchange'de karşılığı olmayan veya fiyatı çok sapmış bayat pozisyon.
    /// true → otomatik SL/TP uygulanmaz; kullanıcı manuel çıkış yapmalı.
    #[serde(default)]
    pub(crate) manual_exit_required: bool,
}

impl OpenPosition {
    /// Yeni pozisyon oluştur.
    /// `entry_price`: spread/slippage sonrası etkin giriş fiyatı.
    /// `leverage`: uygulanan kaldıraç (1.0 = kaldıraçsız).
    /// `breakeven_at_rr`: kâr R katına ulaşınca SL entry'e taşı (None = devre dışı)
    /// `atr_trail_mult`: ATR tabanlı trailing çarpanı (None = sabit trailing_pct kullanılır)
    /// `partial_tp_ratio`: TP'de kapatılacak oran (None = tam kapat)
    /// `trailing_activation_pct`: TSL aktif olmadan önce gerekli min kâr % (None = sıfır eşik)
    pub(crate) fn new(
        symbol: String,
        market: Market,
        entry_price: f64,
        qty: f64,
        is_long: bool,
        risk: &RiskParams,
        leverage: f64,
        breakeven_at_rr: Option<f64>,
        atr_trail_mult: Option<f64>,
        partial_tp_ratio: Option<f64>,
        trailing_activation_pct: Option<f64>,
    ) -> Self {
        let lev = leverage.max(1.0);
        let static_sl = if is_long {
            entry_price * (1.0 - risk.stop_loss_pct / 100.0)
        } else {
            entry_price * (1.0 + risk.stop_loss_pct / 100.0)
        };
        let static_tp = if is_long {
            entry_price * (1.0 + risk.take_profit_pct / 100.0)
        } else {
            entry_price * (1.0 - risk.take_profit_pct / 100.0)
        };
        // Tasfiye fiyatı: marjinin %90'ı eridikten sonra borsa zorla kapatır.
        // Güvenlik tamponu olarak %10 bırakıyoruz (0.9 / lev).
        let liquidation_price = if is_long {
            entry_price * (1.0 - 0.9 / lev)
        } else {
            entry_price * (1.0 + 0.9 / lev)
        };
        // risk_distance: başlangıç SL ile entry arası mutlak mesafe — breakeven hesabı için sabitlenir
        let risk_distance = (entry_price - static_sl).abs().max(f64::EPSILON);
        // TP1: entry ile static_tp arasının %50'si → erken kâr alma
        // Yalnızca partial_tp_ratio ayarlıysa (TP merdiveni aktifse) hesapla
        let tp1_price = if partial_tp_ratio.is_some() {
            let mid = if is_long {
                entry_price + 0.5 * (static_tp - entry_price)
            } else {
                entry_price - 0.5 * (entry_price - static_tp)
            };
            Some(mid)
        } else {
            None
        };
        Self {
            id: PositionId::new(),
            symbol,
            market,
            entry_price,
            qty,
            is_long,
            static_sl,
            static_tp,
            best_price: entry_price,
            trailing_sl: None,
            trailing_pct: risk.trailing_stop_pct,
            trailing_activation_pct,
            leverage: lev,
            liquidation_price,
            risk_distance,
            breakeven_at_rr,
            breakeven_triggered: false,
            atr_trail_mult,
            partial_tp_ratio,
            partial_tp_triggered: false,
            opened_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            entry_features: None,
            tp1_price,
            tp1_close_ratio: 0.40,
            tp1_triggered: false,
            trade_type: crate::robot::scalp_swing::TradeType::Regular,
            manual_exit_required: false,
        }
    }

    /// Fiyatı güncelle; trailing SL / breakeven / partial TP hesapla.
    ///
    /// Döndürür:
    /// - `Some("trailing_sl")` — trailing stop tetiklendi
    /// - `Some("static_sl")`   — sabit stop-loss tetiklendi
    /// - `Some("partial_tp")`  — kısmi TP (ilk kez), pozisyon kapanmaz — caller'ın qty küçültmesi gerekir
    /// - `Some("take_profit")` — take-profit hedefi aşıldı (tam kapat)
    /// - `None`                — pozisyon devam ediyor
    pub(crate) fn update(&mut self, current_price: f64) -> Option<&'static str> {
        // En iyi fiyatı güncelle
        if self.is_long {
            if current_price > self.best_price {
                self.best_price = current_price;
            }
        } else if current_price < self.best_price {
            self.best_price = current_price;
        }

        // Trailing SL: yalnızca kâr bölgesinde aktif
        // Not: ATR tabanlı trailing için trailing_pct dışarıdan (check_live_sl_tp) güncellenir.
        if let Some(tpct) = self.trailing_pct {
            // TSL aktivasyon eşiği: pozisyon yeterli kâra geçmeden trailing set edilmez.
            // ⚠ MATEMATİKSEL ZORUNLULUK: activation_pct > trailing_pct olmalı.
            //   Aksi hâlde TSL aktive anında best_price * (1 - tpct) < entry_price olur →
            //   profit_floor devreye girer → trailing_sl = entry → fiyat entry'e revert edince
            //   ANİNDE KAPANIR (komisyon kaybı, süre: 1 saniye).
            //   Güvenli minimum: activation_pct = tpct + 1.0 (en az %1 buffer)
            let raw_act = self.trailing_activation_pct.unwrap_or(0.0);
            let act = raw_act.max(tpct + 1.0); // matematiksel zemin — entry'nin üstünde kalır
            let in_profit = if self.is_long {
                current_price >= self.entry_price * (1.0 + act / 100.0)
            } else {
                current_price <= self.entry_price * (1.0 - act / 100.0)
            };
            if in_profit {
                // Derin kâr trailing sıkılaştırması: 2R+ kârda TSL daralır, kârı korur
                let profit_rr = {
                    let rd = self.risk_distance.max(f64::EPSILON);
                    if self.is_long { (current_price - self.entry_price) / rd }
                    else            { (self.entry_price - current_price) / rd }
                };
                let effective_tpct = if profit_rr >= 3.0 {
                    tpct * 0.50   // 3R+ kârda trailing yarıya iner
                } else if profit_rr >= 2.0 {
                    tpct * 0.70   // 2R kârda %30 sıkılaştırma
                } else {
                    tpct
                };
                let tight_tsl = if self.is_long {
                    self.best_price * (1.0 - effective_tpct / 100.0)
                } else {
                    self.best_price * (1.0 + effective_tpct / 100.0)
                };
                // Ratchet: TSL sadece kazançlı yönde ilerler, ATR büyüyünce geri çekilmez
                let ratcheted = if self.is_long {
                    self.trailing_sl.map_or(tight_tsl, |old| old.max(tight_tsl))
                } else {
                    self.trailing_sl.map_or(tight_tsl, |old| old.min(tight_tsl))
                };
                // Kâr kilidı: TSL yalnızca pozisyon *gerçekten* kârdayken (2×tpct gerekli buffer)
                // entry'ye floorlenir. Bu anlık revert kapanışını engeller.
                let has_buffer = if self.is_long {
                    current_price >= self.entry_price * (1.0 + tpct * 2.0 / 100.0)
                } else {
                    current_price <= self.entry_price * (1.0 - tpct * 2.0 / 100.0)
                };
                let floored = if has_buffer {
                    if self.is_long && ratcheted < self.entry_price {
                        self.entry_price
                    } else if !self.is_long && ratcheted > self.entry_price {
                        self.entry_price
                    } else {
                        ratcheted
                    }
                } else {
                    ratcheted
                };
                self.trailing_sl = Some(floored);
            }
        }

        // ── B1: Breakeven stop — kâr belirli R katına ulaşınca SL giriş fiyatına çek ──────────
        if !self.breakeven_triggered {
            if let Some(be_rr) = self.breakeven_at_rr {
                let trigger = if self.is_long {
                    current_price >= self.entry_price + be_rr * self.risk_distance
                } else {
                    current_price <= self.entry_price - be_rr * self.risk_distance
                };
                if trigger {
                    self.static_sl = self.entry_price;
                    self.breakeven_triggered = true;
                }
            }
        }

        // ── Çıkış koşulları ──────────────────────────────────────────────────────────────────
        // Öncelik: TP > trailing_sl > static_sl
        //
        // TP önce kontrol edilir: trailing_sl kademeli yükselince static_tp'yi aşabilir.
        // Bu durumda trailing'in TP'yi bypass ederek imkânsız fill (tsl > piyasa) üretmesini önler.
        // Trailing amacı downside koruma; fiyat hedefe ulaştıysa TP kazanır.

        // ── TP Merdiveni (TP1) — LONG: fiyat ≥ tp1, SHORT: fiyat ≤ tp1 ────────────────────
        if !self.tp1_triggered {
            if let Some(tp1) = self.tp1_price {
                let hit = if self.is_long { current_price >= tp1 } else { current_price <= tp1 };
                if hit { return Some("tp1"); }
            }
        }

        // ── Kısmi TP (B3) ────────────────────────────────────────────────────────────────────
        if !self.partial_tp_triggered {
            if self.partial_tp_ratio.is_some() {
                if self.is_long  && current_price >= self.static_tp { return Some("partial_tp"); }
                if !self.is_long && current_price <= self.static_tp { return Some("partial_tp"); }
            }
        }

        // ── Tam TP ───────────────────────────────────────────────────────────────────────────
        if self.is_long  && current_price >= self.static_tp { return Some("take_profit"); }
        if !self.is_long && current_price <= self.static_tp { return Some("take_profit"); }

        // ── Trailing SL (TP geçilmemişse devreye girer) ──────────────────────────────────────
        if let Some(tsl) = self.trailing_sl {
            if self.is_long && current_price <= tsl  { return Some("trailing_sl"); }
            if !self.is_long && current_price >= tsl { return Some("trailing_sl"); }
        }

        // ── Statik SL ────────────────────────────────────────────────────────────────────────
        if self.is_long  && current_price <= self.static_sl { return Some("static_sl"); }
        if !self.is_long && current_price >= self.static_sl { return Some("static_sl"); }

        None
    }

    /// Komisyon dahil gerçekleşmiş PnL.
    ///
    /// `commission_pct`: tek taraf oranı (örn. 0.001 = %0.1).
    /// Giriş + çıkış her ikisine de uygulanır (round-trip).
    pub(crate) fn realized_pnl_with_commission(
        &self,
        close_price: f64,
        commission_pct: f64,
    ) -> f64 {
        let gross = if self.is_long {
            (close_price - self.entry_price) * self.qty
        } else {
            (self.entry_price - close_price) * self.qty
        };
        let entry_notional = self.entry_price * self.qty;
        let exit_notional  = close_price * self.qty;
        let commission     = (entry_notional + exit_notional) * commission_pct;
        gross - commission
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Market, RiskParams};

    fn risk() -> RiskParams {
        RiskParams {
            stop_loss_pct: 2.0,
            take_profit_pct: 4.0,
            max_position_size_pct: None,
            max_portfolio_risk_pct: None,
            use_kelly_criterion: false,
            trailing_stop_pct: None,
        }
    }

    #[test]
    fn static_sl_long() {
        let mut pos = OpenPosition::new(
            "BTCUSDT".into(), Market::Spot, 100.0, 1.0, true, &risk(), 1.0, None, None, None, None
        );
        // SL = 100 * 0.98 = 98.0
        assert_eq!(pos.update(98.5), None);
        assert_eq!(pos.update(97.9), Some("static_sl"));
    }

    #[test]
    fn take_profit_short() {
        let mut pos = OpenPosition::new(
            "ETHUSDT".into(), Market::Futures, 100.0, 1.0, false, &risk(), 1.0, None, None, None, None
        );
        // TP = 100 * 0.96 = 96.0
        assert_eq!(pos.update(97.0), None);
        assert_eq!(pos.update(95.9), Some("take_profit"));
    }

    #[test]
    fn pnl_commission_roundtrip() {
        let pos = OpenPosition::new(
            "BTCUSDT".into(), Market::Spot, 100.0, 1.0, true, &risk(), 1.0, None, None, None, None
        );
        // gross = (110 - 100) * 1 = 10.0
        // commission = (100 + 110) * 0.001 = 0.21
        let pnl = pos.realized_pnl_with_commission(110.0, 0.001);
        assert!((pnl - 9.79).abs() < 0.001);
    }

    #[test]
    fn unique_ids() {
        let r = risk();
        let p1 = OpenPosition::new("BTCUSDT".into(), Market::Spot, 100.0, 1.0, true, &r, 1.0, None, None, None, None);
        let p2 = OpenPosition::new("BTCUSDT".into(), Market::Spot, 100.0, 1.0, true, &r, 1.0, None, None, None, None);
        assert_ne!(p1.id, p2.id, "Her pozisyon eşsiz PositionId taşımalı");
    }

    #[test]
    fn leverage_liquidation_price() {
        let pos = OpenPosition::new(
            "BTCUSDT".into(), Market::Futures, 100.0, 10.0, true, &risk(), 10.0, None, None, None, None
        );
        // 10x long: liq = 100 * (1 - 0.9/10) = 91.0
        assert!((pos.liquidation_price - 91.0).abs() < 0.001);
        // SL (2%) at 98 > liq (91) → safe
        assert!(pos.static_sl > pos.liquidation_price);
    }

    // ── B1: Breakeven stop ────────────────────────────────────────────────────

    #[test]
    fn breakeven_triggers_at_half_r() {
        // entry=100, SL=98 → risk_distance=2, TP=104
        // breakeven_at_rr=0.5 → trigger when price >= 100 + 0.5*2 = 101
        let mut pos = OpenPosition::new(
            "BTCUSDT".into(), Market::Spot, 100.0, 1.0, true, &risk(), 1.0,
            Some(0.5), None, None, None,
        );
        assert_eq!(pos.breakeven_triggered, false);
        assert_eq!(pos.update(100.9), None); // 100.9 < 101 → henüz tetiklenme
        assert_eq!(pos.breakeven_triggered, false);
        assert_eq!(pos.update(101.0), None); // tam eşik → tetiklenmeli
        assert_eq!(pos.breakeven_triggered, true);
        assert!((pos.static_sl - 100.0).abs() < f64::EPSILON); // SL entry'e taşındı
    }

    #[test]
    fn breakeven_protects_from_sl() {
        // Breakeven tetiklendikten sonra fiyat entry'e dönerse SL ile kapanmalı
        let mut pos = OpenPosition::new(
            "BTCUSDT".into(), Market::Spot, 100.0, 1.0, true, &risk(), 1.0,
            Some(0.5), None, None, None,
        );
        pos.update(101.0); // breakeven tetikle
        assert_eq!(pos.breakeven_triggered, true);
        // Fiyat entry'e dönüyor — breakeven SL tetiklemeli
        assert_eq!(pos.update(99.9), Some("static_sl"));
    }

    #[test]
    fn breakeven_disabled_by_default() {
        let mut pos = OpenPosition::new(
            "BTCUSDT".into(), Market::Spot, 100.0, 1.0, true, &risk(), 1.0,
            None, None, None, None,
        );
        pos.update(103.0);
        assert_eq!(pos.breakeven_triggered, false);
        // SL hâlâ orijinal seviyede (98.0)
        assert!((pos.static_sl - 98.0).abs() < 0.001);
    }

    // ── B3: Kısmi TP ─────────────────────────────────────────────────────────

    #[test]
    fn partial_tp_fires_once_then_take_profit() {
        // entry=100, TP=104 (4%), partial_tp_ratio=0.5
        let mut pos = OpenPosition::new(
            "BTCUSDT".into(), Market::Spot, 100.0, 1.0, true, &risk(), 1.0,
            None, None, Some(0.5), None,
        );
        assert_eq!(pos.partial_tp_triggered, false);
        // TP seviyesi aşıldı — ilk seferde partial_tp dönmeli
        assert_eq!(pos.update(104.0), Some("partial_tp"));
        // Caller partial_tp_triggered'i true yapar (dışarıda yapılıyor)
        pos.partial_tp_triggered = true;
        // Sonraki check'te normal take_profit dönmeli
        assert_eq!(pos.update(104.0), Some("take_profit"));
    }

    #[test]
    fn partial_tp_disabled_returns_take_profit() {
        // partial_tp_ratio = None → direkt take_profit
        let mut pos = OpenPosition::new(
            "ETHUSDT".into(), Market::Spot, 100.0, 1.0, true, &risk(), 1.0,
            None, None, None, None,
        );
        assert_eq!(pos.update(104.0), Some("take_profit"));
    }
}
