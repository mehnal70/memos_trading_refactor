/// SlotGuard — Sembol başına scalp/swing slot yöneticisi
///
/// Aynı sembolde hem scalp hem swing pozisyon açılabilir ama:
///   - Aynı türde (Scalp+Scalp veya Swing+Swing) max_per_symbol aşılamaz
///   - Aynı yönde (Long+Long) bir scalp + swing açılabilir (farklı zaman dilimi)
///   - Zıt yönde (Long scalp + Short swing) açılmasına izin verilmez (net hedge engeli)

use super::TradeType;

pub struct SlotGuard;

/// Açık pozisyon özetini temsil eder (OpenPosition'dan bağımsız)
#[derive(Debug, Clone)]
pub struct OpenSlot {
    pub symbol:     String,
    pub trade_type: TradeType,
    pub is_long:    bool,
}

impl SlotGuard {
    /// Yeni bir pozisyon açılabilir mi?
    ///
    /// `existing`: mevcut açık pozisyonların özeti (symbol + trade_type + yön)
    /// `symbol`  : açılmak istenen sembol
    /// `trade_type`: açılmak istenen tür
    /// `is_long` : açılmak istenen yön
    /// `max_scalp`, `max_swing`: sembol başına maksimum slot sayısı
    ///
    /// Returns: (allowed: bool, reason: &str)
    pub fn can_open(
        existing:   &[OpenSlot],
        symbol:     &str,
        trade_type: TradeType,
        is_long:    bool,
        max_scalp:  usize,
        max_swing:  usize,
    ) -> (bool, &'static str) {
        let sym_positions: Vec<&OpenSlot> = existing.iter()
            .filter(|s| s.symbol == symbol)
            .collect();

        // 1. Tür bazında slot sayısı kontrolü
        let type_count = sym_positions.iter()
            .filter(|s| s.trade_type == trade_type)
            .count();
        let max_slots = match trade_type {
            TradeType::Scalp   => max_scalp,
            TradeType::Swing   => max_swing,
            TradeType::Regular => usize::MAX,
        };
        if type_count >= max_slots {
            return (false, "slot dolu");
        }

        // 2. Zıt yön engeli: Regular hariç, aynı sembolde karşı yönlü açık pozisyon varsa engelle
        if trade_type != TradeType::Regular {
            let opposite_exists = sym_positions.iter()
                .filter(|s| s.trade_type != TradeType::Regular)
                .any(|s| s.is_long != is_long);
            if opposite_exists {
                return (false, "zıt yönde açık pozisyon var (hedge engeli)");
            }
        }

        (true, "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(symbol: &str, tt: TradeType, is_long: bool) -> OpenSlot {
        OpenSlot { symbol: symbol.to_string(), trade_type: tt, is_long }
    }

    /// Boş liste → her tür açılabilir
    #[test]
    fn test_empty_slots_allows_any() {
        let (ok, _) = SlotGuard::can_open(&[], "BTCUSDT", TradeType::Scalp, true, 1, 1);
        assert!(ok);
    }

    /// Scalp slot dolunca reddedilmeli
    #[test]
    fn test_scalp_slot_full_blocks() {
        let existing = vec![slot("BTCUSDT", TradeType::Scalp, true)];
        let (ok, reason) = SlotGuard::can_open(&existing, "BTCUSDT", TradeType::Scalp, true, 1, 1);
        assert!(!ok);
        assert_eq!(reason, "slot dolu");
    }

    /// Swing slot dolunca reddedilmeli
    #[test]
    fn test_swing_slot_full_blocks() {
        let existing = vec![slot("BTCUSDT", TradeType::Swing, false)];
        let (ok, reason) = SlotGuard::can_open(&existing, "BTCUSDT", TradeType::Swing, false, 1, 1);
        assert!(!ok);
        assert_eq!(reason, "slot dolu");
    }

    /// Farklı sembolde dolu slot engellemez
    #[test]
    fn test_other_symbol_does_not_block() {
        let existing = vec![slot("ETHUSDT", TradeType::Scalp, true)];
        let (ok, _) = SlotGuard::can_open(&existing, "BTCUSDT", TradeType::Scalp, true, 1, 1);
        assert!(ok);
    }

    /// Zıt yönde scalp varken swing açılamaz (hedge engeli)
    #[test]
    fn test_opposite_direction_blocked() {
        // Long scalp açık → short swing engellenmeli
        let existing = vec![slot("BTCUSDT", TradeType::Scalp, true)];
        let (ok, reason) = SlotGuard::can_open(&existing, "BTCUSDT", TradeType::Swing, false, 1, 2);
        assert!(!ok);
        assert!(reason.contains("zıt yönde"));
    }

    /// Aynı yönde scalp + swing birlikte açılabilir
    #[test]
    fn test_same_direction_scalp_plus_swing_allowed() {
        let existing = vec![slot("BTCUSDT", TradeType::Scalp, true)];
        let (ok, _) = SlotGuard::can_open(&existing, "BTCUSDT", TradeType::Swing, true, 1, 1);
        assert!(ok);
    }

    /// Regular türü zıt yön engelinden muaf
    #[test]
    fn test_regular_bypasses_opposite_direction_block() {
        let existing = vec![slot("BTCUSDT", TradeType::Scalp, true)];
        let (ok, _) = SlotGuard::can_open(&existing, "BTCUSDT", TradeType::Regular, false, 1, 1);
        assert!(ok);
    }

    /// max_scalp=2 ile iki scalp açılabilir, üçüncüsü reddedilir
    #[test]
    fn test_max_scalp_two() {
        let existing = vec![
            slot("BTCUSDT", TradeType::Scalp, true),
            slot("BTCUSDT", TradeType::Scalp, true),
        ];
        let (ok_before, _) = SlotGuard::can_open(&existing[..1], "BTCUSDT", TradeType::Scalp, true, 2, 1);
        let (ok_after, _)  = SlotGuard::can_open(&existing,       "BTCUSDT", TradeType::Scalp, true, 2, 1);
        assert!(ok_before);
        assert!(!ok_after);
    }
}
