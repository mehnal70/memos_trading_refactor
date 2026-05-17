// robot/scalp_swing/slot_guard.rs - Pozisyon Slot ve Çakışma Denetçisi

use super::TradeType;

/// §64.1: Açık pozisyonun stratejik özetini temsil eden hafif yapı.
#[derive(Debug, Clone)]
pub struct OpenSlot {
    pub symbol:     String,
    pub trade_type: TradeType,
    pub is_long:    bool,
}

pub struct SlotGuard;

impl SlotGuard {
    /// Yeni bir pozisyonun açılma uygunluğunu otonom denetler.
    /// Srivastava ATP Standartları: Hedge engeli ve Slot limit denetimi.
    pub fn can_open(
        existing:   &[OpenSlot],
        symbol:     &str,
        trade_type: TradeType,
        is_long:    bool,
        max_scalp:  usize,
        max_swing:  usize,
    ) -> (bool, &'static str) {
        // Sembol özelindeki aktif pozisyonları süz (Zero-allocation iteratör)
        let sym_positions: Vec<&OpenSlot> = existing.iter()
            .filter(|s| s.symbol == symbol)
            .collect();

        // 1. OTONOM SLOT KAPASİTE DENETİMİ
        let type_count = sym_positions.iter()
            .filter(|s| s.trade_type == trade_type)
            .count();
            
        let max_slots = match trade_type {
            TradeType::Scalp   => max_scalp,
            TradeType::Swing   => max_swing,
            TradeType::Regular => usize::MAX, 
        };
        
        if type_count >= max_slots {
            return (false, "Slot kapasitesi dolu (Max per symbol)");
        }

        // 2. STRATEJİK ÇAKIŞMA VE HEDGE ENGELİ
        // Aynı sembolde yan motorların (Scalp/Swing) karşıt yönlü işlem açmasını engeller.
        // Amaç: Sermaye verimliliğini korumak ve gereksiz spread kaybını önlemek.
        if trade_type != TradeType::Regular {
            let opposite_exists = sym_positions.iter()
                .filter(|s| s.trade_type != TradeType::Regular)
                .any(|s| s.is_long != is_long);
                
            if opposite_exists {
                return (false, "Zıt yönde aktif yan pozisyon tespit edildi (Hedge Guard)");
            }
        }

        (true, "Uygun")
    }
}
