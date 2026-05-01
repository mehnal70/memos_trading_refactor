// Dinamik Pozisyon Yönetimi - Trailing stop, scale-in/out, partial fill'ler
// Dynamic Position Management: trailing stop, scale-in/out, partial fills

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use crate::MemosTradingError;

type Result<T> = std::result::Result<T, MemosTradingError>;

/// Trailing Stop Konfigürasyonu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrailingStopConfig {
    /// Trailing stop baz yüzdesi (örn: 2.5% = fiyat tavan değeri * 0.975)
    pub trailing_pct: f64,
    
    /// Minimum trailing stop hareketi (baz puan cinsinden)
    pub min_movement_bps: u32, // 100 bps = 1%
    
    /// Aktif mi?
    pub enabled: bool,
}

impl Default for TrailingStopConfig {
    fn default() -> Self {
        Self {
            trailing_pct: 2.5,
            min_movement_bps: 50, // 0.5%
            enabled: true,
        }
    }
}

/// Kademeli Giriş Konfigürasyonu (Scale-In)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleInConfig {
    /// Trend günü kuvvetlenirse ek girişleri mı yapacağız?
    pub enabled: bool,
    
    /// Maksimum scale-in sayısı (toplam pozisyon size limitlemek için)
    pub max_scalein_count: usize,
    
    /// Her scale-in için ek porsiyon (% cinsinden bazı pozisyon)
    pub scale_in_pct: f64,
    
    /// Scale-in tetikleyicisi: fiyat son ATH'ye ne kadar yaklaşıyor (%)
    pub trigger_proximity_pct: f64,
}

impl Default for ScaleInConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_scalein_count: 3,
            scale_in_pct: 50.0, // İlk girişin %50'si kadar
            trigger_proximity_pct: 0.5, // ATH'ye yakınsa
        }
    }
}

/// Kademeli Çıkış Konfigürasyonu (Scale-Out)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleOutConfig {
    /// Kademeli çıkış mı yapacağız?
    pub enabled: bool,
    
    /// Maksimum scale-out sayısı
    pub max_scaleout_count: usize,
    
    /// Her scale-out'ta kapatacağımız % (toplam pozisyon büyüklüğünün)
    pub scaleout_pct: f64,
    
    /// Profit hedefleri (% cinsinden)
    pub profit_targets: Vec<f64>, // [2.0, 5.0, 10.0] = %2 kâr, %5 kâr, %10 kâr
}

impl Default for ScaleOutConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_scaleout_count: 3,
            scaleout_pct: 33.33, // Her target'ta 1/3'ünü kapamak
            profit_targets: vec![2.0, 5.0, 10.0],
        }
    }
}

/// Partial Fill Verisi
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialFill {
    /// Hangi fiyattan?
    pub fill_price: f64,
    
    /// Ne kadar quantity fill oldu?
    pub fill_quantity: f64,
    
    /// Zamanı
    pub fill_time: DateTime<Utc>,
    
    /// Partial fill türü: "entry" | "exit" | "trailing_sl" | "take_profit"
    pub fill_type: String,
    
    /// Commission/fee (varsa)
    pub fee: f64,
}

impl PartialFill {
    pub fn pnl(&self) -> f64 {
        self.fill_quantity * self.fill_price - self.fee
    }
}

/// Extended Position - Trailing stop, scale-in/out, partial fills ile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicPosition {
    /// Base position info
    pub symbol: String,
    pub entry_price: f64,
    pub quantity: f64,
    pub direction: f64, // 1.0 = long, -1.0 = short
    pub entry_time: DateTime<Utc>,
    pub current_price: f64,
    
    /// Static SL/TP
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    
    // ====== TRAILING STOP ======
    pub trailing_config: TrailingStopConfig,
    /// Trailing stop'ın en iyi fiyat kaydı (en yüksek fiyat long pozisyonda)
    pub highest_price: f64,
    /// Şimdiki trailing stop level
    pub current_trailing_sl: Option<f64>,
    
    // ====== SCALE-IN/OUT ======
    pub scalein_config: ScaleInConfig,
    pub scaleout_config: ScaleOutConfig,
    /// Kaç kez scale-in yapılmış?
    pub scalein_count: usize,
    /// Kaç kez scale-out yapılmış?
    pub scaleout_count: usize,
    
    // ====== PARTIAL FILLS ======
    /// Tüm partial fill'ler (entry ve exit)
    pub partial_fills: Vec<PartialFill>,
    /// Kalan açık quantity
    pub open_quantity: f64,
}

impl DynamicPosition {
    pub fn new(
        symbol: String,
        entry_price: f64,
        quantity: f64,
        direction: f64,
        stop_loss: Option<f64>,
        take_profit: Option<f64>,
    ) -> Self {
        Self {
            symbol,
            entry_price,
            quantity,
            direction,
            entry_time: Utc::now(),
            current_price: entry_price,
            stop_loss,
            take_profit,
            trailing_config: TrailingStopConfig::default(),
            highest_price: entry_price,
            current_trailing_sl: None,
            scalein_config: ScaleInConfig::default(),
            scaleout_config: ScaleOutConfig::default(),
            scalein_count: 0,
            scaleout_count: 0,
            partial_fills: vec![],
            open_quantity: quantity,
        }
    }
    
    /// Config helper profilleri ile pozisyon oluştur
    pub fn with_configs(
        symbol: String,
        entry_price: f64,
        quantity: f64,
        direction: f64,
        stop_loss: Option<f64>,
        take_profit: Option<f64>,
        trailing_config: TrailingStopConfig,
        scalein_config: ScaleInConfig,
        scaleout_config: ScaleOutConfig,
    ) -> Self {
        Self {
            symbol,
            entry_price,
            quantity,
            direction,
            entry_time: Utc::now(),
            current_price: entry_price,
            stop_loss,
            take_profit,
            trailing_config,
            highest_price: entry_price,
            current_trailing_sl: None,
            scalein_config,
            scaleout_config,
            scalein_count: 0,
            scaleout_count: 0,
            partial_fills: vec![],
            open_quantity: quantity,
        }
    }
    
    /// Builder pattern: Mevcut pozisyona config'leri uygula
    pub fn apply_configs(
        &mut self,
        trailing_config: TrailingStopConfig,
        scalein_config: ScaleInConfig,
        scaleout_config: ScaleOutConfig,
    ) -> &mut Self {
        self.trailing_config = trailing_config;
        self.scalein_config = scalein_config;
        self.scaleout_config = scaleout_config;
        self
    }
    
    // ============ TRAILING STOP ============
    
    /// Mevcut fiyata göre trailing stop'u güncelle
    /// Trailing SL'ye vuruş varsa true döndür
    pub fn update_trailing_stop(&mut self) -> bool {
        if !self.trailing_config.enabled {
            return false;
        }
        
        let is_long = self.direction > 0.0;
        
        if is_long {
            // Long pozisyonda: en yüksek fiyatı takip et
            if self.current_price > self.highest_price {
                self.highest_price = self.current_price;
            }
            
            // Trailing stop level'i hesapla
            let new_sl = self.highest_price * (1.0 - self.trailing_config.trailing_pct / 100.0);
            self.current_trailing_sl = Some(new_sl);
            
            // SL triggered mi?
            if self.current_price <= new_sl {
                return true;
            }
        } else {
            // Short pozisyonda: en düşük fiyatı takip et
            if self.current_price < self.highest_price {
                self.highest_price = self.current_price;
            }
            
            // Trailing stop level'i hesapla (short'da inverse logic)
            let new_sl = self.highest_price * (1.0 + self.trailing_config.trailing_pct / 100.0);
            self.current_trailing_sl = Some(new_sl);
            
            // SL triggered mi?
            if self.current_price >= new_sl {
                return true;
            }
        }
        
        false
    }
    
    // ============ SCALE-IN ============
    
    /// Scale-in mümkün mü? (Trend kuvvetle devam ediyor mu?)
    pub fn can_scalein(&self, _peak_price: f64) -> bool {
        if !self.scalein_config.enabled {
            return false;
        }
        
        if self.scalein_count >= self.scalein_config.max_scalein_count {
            return false;
        }
        
        let is_long = self.direction > 0.0;
        
        if is_long {
            // Long: fiyat peak'e yakınsa (kuvvetle) scale-in yap
            let proximity = ((self.current_price - self.entry_price) / self.entry_price) * 100.0;
            proximity >= self.scalein_config.trigger_proximity_pct
        } else {
            // Short: fiyat peak'e yakınsa scale-in yap
            let proximity = ((self.entry_price - self.current_price) / self.entry_price) * 100.0;
            proximity >= self.scalein_config.trigger_proximity_pct
        }
    }
    
    /// Scale-in miktarını hesapla
    pub fn calculate_scalein_quantity(&self) -> f64 {
        self.quantity * (self.scalein_config.scale_in_pct / 100.0)
    }
    
    /// Scale-in işlemini kaydet
    pub fn record_scalein(&mut self, quantity: f64, fill_price: f64, fee: f64) -> Result<()> {
        if self.scalein_count >= self.scalein_config.max_scalein_count {
            return Err(MemosTradingError::Unknown(
                "Maximum scale-in count reached".to_string()
            ));
        }
        
        let fill = PartialFill {
            fill_price,
            fill_quantity: quantity,
            fill_time: Utc::now(),
            fill_type: "entry".to_string(),
            fee,
        };
        
        self.partial_fills.push(fill);
        self.open_quantity += quantity;
        self.scalein_count += 1;
        
        Ok(())
    }
    
    // ============ SCALE-OUT ============
    
    /// Hangi profit target'a ulaştık?
    pub fn active_profit_target(&self) -> Option<f64> {
        let pnl_pct = self.unrealized_pnl_pct();
        
        self.scaleout_config
            .profit_targets
            .iter()
            .filter(|&&t| pnl_pct >= t)
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .copied()
    }
    
    /// Scale-out miktarını hesapla
    pub fn calculate_scaleout_quantity(&self) -> f64 {
        self.open_quantity * (self.scaleout_config.scaleout_pct / 100.0)
    }
    
    /// Scale-out işlemini kaydet
    pub fn record_scaleout(&mut self, quantity: f64, exit_price: f64, fee: f64) -> Result<()> {
        if self.scaleout_count >= self.scaleout_config.max_scaleout_count {
            return Err(MemosTradingError::Unknown(
                "Maximum scale-out count reached".to_string()
            ));
        }
        
        if quantity > self.open_quantity {
            return Err(MemosTradingError::Unknown(
                format!("Cannot scale-out {} > {}", quantity, self.open_quantity)
            ));
        }
        
        let fill = PartialFill {
            fill_price: exit_price,
            fill_quantity: quantity,
            fill_time: Utc::now(),
            fill_type: "exit".to_string(),
            fee,
        };
        
        self.partial_fills.push(fill);
        self.open_quantity -= quantity;
        self.scaleout_count += 1;
        
        Ok(())
    }
    
    // ============ METRICS ============
    
    pub fn unrealized_pnl(&self) -> f64 {
        (self.current_price - self.entry_price) * self.open_quantity * self.direction
    }
    
    pub fn unrealized_pnl_pct(&self) -> f64 {
        if self.entry_price == 0.0 {
            return 0.0;
        }
        ((self.current_price - self.entry_price) / self.entry_price) * 100.0
    }
    
    pub fn realized_pnl(&self) -> f64 {
        self.partial_fills
            .iter()
            .filter(|f| f.fill_type == "exit")
            .map(|f| {
                match self.direction > 0.0 {
                    true => (f.fill_price - self.entry_price) * f.fill_quantity - f.fee,
                    false => (self.entry_price - f.fill_price) * f.fill_quantity - f.fee,
                }
            })
            .sum()
    }
    
    pub fn total_pnl(&self) -> f64 {
        self.unrealized_pnl() + self.realized_pnl()
    }
    
    pub fn position_value(&self) -> f64 {
        self.current_price * self.open_quantity
    }
    
    pub fn age_seconds(&self) -> i64 {
        (Utc::now() - self.entry_time).num_seconds()
    }
    
    /// Trailing stop'ın şimdiki seviyesini döndür (static SL'den daha iyiyse)
    pub fn effective_stop_loss(&self) -> Option<f64> {
        match (self.stop_loss, self.current_trailing_sl) {
            (Some(static_sl), Some(trailing_sl)) => {
                // Long: en yüksek SL'yi kullan (daha az risk)
                // Short: en düşük SL'yi kullan
                if self.direction > 0.0 {
                    Some(static_sl.max(trailing_sl))
                } else {
                    Some(static_sl.min(trailing_sl))
                }
            }
            (Some(sl), None) => Some(sl),
            (None, Some(tsl)) => Some(tsl),
            (None, None) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trailing_stop_long() {
        let mut pos = DynamicPosition::new(
            "BTCUSDT".to_string(),
            45000.0,
            1.0,
            1.0,
            Some(44000.0),
            Some(50000.0),
        );
        
        // Fiyat artışı
        pos.current_price = 46000.0;
        assert!(!pos.update_trailing_stop()); // Hâlâ açık
        
        // Daha fazla artışı
        pos.current_price = 47000.0;
        assert!(!pos.update_trailing_stop());
        
        // Trailing stop triggered
        pos.current_price = 45750.0;
        assert!(pos.update_trailing_stop());
    }

    #[test]
    fn test_scalein() {
        let mut pos = DynamicPosition::new(
            "BTCUSDT".to_string(),
            45000.0,
            1.0,
            1.0,
            Some(44000.0),
            Some(50000.0),
        );
        
        // Update current price first to trigger scale-in
        pos.current_price = 45500.0;
        
        assert!(pos.can_scalein(46000.0));
        pos.record_scalein(0.5, 45500.0, 0.0).unwrap();
        assert_eq!(pos.scalein_count, 1);
        assert_eq!(pos.open_quantity, 1.5);
    }

    #[test]
    fn test_scaleout() {
        let mut pos = DynamicPosition::new(
            "BTCUSDT".to_string(),
            45000.0,
            1.0,
            1.0,
            Some(44000.0),
            Some(50000.0),
        );
        
        pos.current_price = 45900.0; // %2 kâr
        assert_eq!(pos.active_profit_target(), Some(2.0));
        
        let qty_to_exit = pos.calculate_scaleout_quantity();
        pos.record_scaleout(qty_to_exit, 45900.0, 0.0).unwrap();
        
        assert_eq!(pos.scaleout_count, 1);
        assert!(pos.open_quantity < 1.0);
    }
}
