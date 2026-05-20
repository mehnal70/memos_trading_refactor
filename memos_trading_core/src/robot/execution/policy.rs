// robot/execution/policy.rs — Yürütme öncesi politika zinciri.
//
// Tasarım — RiskFilter chain pattern'ı ile aynı çizgide:
//   - Plug-in noktası: `trait ExecutionPolicy`. Her policy salt-okunur bağlam
//     alır ve `ExecutionDecision::{Allow, Skip{reason}}` döndürür.
//   - Zincir: `evaluate_chain` ilk Skip'te kısa-devre olur (RiskFilter ilk
//     Deny ile aynı).
//   - Her policy iki yüzlü:
//        (a) `evaluate(&ExecutionContext)` — trait imzası, zincirin çağırdığı
//            genel uç.
//        (b) `evaluate_<inputs>(...)` — saf yardımcı; bağımlılığı yok, doğrudan
//            test edilebilir. Trait yöntemi yalnız bağlamı söker ve (b)'yi
//            çağırır.
//   - `default_chain()` projenin varsayılan 3 policy'sini (MarketHours →
//     IdleStrategy → BasketEmpty) verir; runtime'da `push_policy` ile yenisi
//     eklenebilir, `with_policies` ile baştan farklı zincir kurulabilir.
//
// Bu commit'te policy'ler yalnız "skip" üretir (Risk filter "Deny"i yerine);
// `RoboticTradeExecutor::execute_basket` skip alan sembolü atlar, kalanlarla
// devam eder. Halt/safe-mode kavramı yürütme katmanı yerine RiskFilter
// chain'in yetkisi olarak tutulur (yetki ayrımı).

use crate::core::types::Signal;

// ─────────────────────────────────────────────────────────────────────────────
// Bağlam ve karar tipi
// ─────────────────────────────────────────────────────────────────────────────

/// Tek bir emir/sembol değerlendirmesinin paylaşılan salt-okunur bağlamı.
///
/// `current_hour` injekte edilir (test edilebilirlik için); production'da
/// `RoboticTradeExecutor` `chrono::Utc::now().hour()` ile doldurur.
pub struct ExecutionContext<'a> {
    pub signal: &'a Signal,
    pub symbol: &'a str,
    pub amount: f64,
    /// Adı `IDLE...` ile başlayan stratejiler veto edilir. Master.rs cycle
    /// içinde aynı sentinel'i okuyor; aynı kuralın executor tarafında da
    /// uygulanması savunma katmanı oluşturur.
    pub strategy_name: Option<&'a str>,
    /// (start_hour, end_hour) — UTC saat dilimi. None ise 7/24 açık.
    pub market_hours: Option<(u32, u32)>,
    /// UTC current hour (0..=23). Test için injekte; production'da
    /// `chrono::Utc::now().hour()` doldurur.
    pub current_hour: u32,
    /// Aktif basket büyüklüğü; 0 ise herhangi bir sembolde emir mantıksız.
    pub basket_size: usize,
}

/// Bir policy'nin verdiği karar. Skip → sembol atlanır, reason loglara akar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionDecision {
    Allow,
    Skip { reason: String },
}

impl ExecutionDecision {
    pub fn is_allow(&self) -> bool {
        matches!(self, ExecutionDecision::Allow)
    }
    pub fn skip_reason(&self) -> Option<&str> {
        match self {
            ExecutionDecision::Skip { reason } => Some(reason.as_str()),
            _ => None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Plug-in trait
// ─────────────────────────────────────────────────────────────────────────────

pub trait ExecutionPolicy: Send + Sync {
    fn name(&self) -> &str;
    fn evaluate(&self, ctx: &ExecutionContext<'_>) -> ExecutionDecision;
}

/// Zincir değerlendirme: ilk Skip kısa-devre. Hangi policy tetiklediğini de
/// döndürür ki çağıran loga "policy=market_hours skip: …" formatında yazabilsin.
pub fn evaluate_chain(
    chain: &[Box<dyn ExecutionPolicy>],
    ctx: &ExecutionContext<'_>,
) -> (ExecutionDecision, Option<String>) {
    for p in chain {
        let d = p.evaluate(ctx);
        if !d.is_allow() {
            return (d, Some(p.name().to_string()));
        }
    }
    (ExecutionDecision::Allow, None)
}

// ─────────────────────────────────────────────────────────────────────────────
// 1) MarketHoursPolicy — borsa saatleri dışında veto eder.
// ─────────────────────────────────────────────────────────────────────────────

pub struct MarketHoursPolicy;

impl MarketHoursPolicy {
    /// Saf çekirdek: market_hours ve current_hour üzerinden karar.
    pub fn evaluate_hours(
        &self,
        market_hours: Option<(u32, u32)>,
        current_hour: u32,
    ) -> ExecutionDecision {
        match market_hours {
            None => ExecutionDecision::Allow,
            Some((start, end)) => {
                if current_hour >= start && current_hour < end {
                    ExecutionDecision::Allow
                } else {
                    ExecutionDecision::Skip {
                        reason: format!(
                            "Market kapalı: saat={current_hour:02} ∉ [{start:02}, {end:02})"
                        ),
                    }
                }
            }
        }
    }
}

impl ExecutionPolicy for MarketHoursPolicy {
    fn name(&self) -> &str { "market_hours" }
    fn evaluate(&self, ctx: &ExecutionContext<'_>) -> ExecutionDecision {
        self.evaluate_hours(ctx.market_hours, ctx.current_hour)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 2) IdleStrategyPolicy — strateji adı `IDLE` ile başlıyorsa veto eder.
// ─────────────────────────────────────────────────────────────────────────────

pub struct IdleStrategyPolicy;

impl IdleStrategyPolicy {
    pub fn evaluate_name(&self, name: Option<&str>) -> ExecutionDecision {
        match name {
            Some(n) if n.trim().to_uppercase().starts_with("IDLE") => {
                ExecutionDecision::Skip {
                    reason: format!("Savunma rejimi: strateji={n}"),
                }
            }
            _ => ExecutionDecision::Allow,
        }
    }
}

impl ExecutionPolicy for IdleStrategyPolicy {
    fn name(&self) -> &str { "idle_strategy" }
    fn evaluate(&self, ctx: &ExecutionContext<'_>) -> ExecutionDecision {
        self.evaluate_name(ctx.strategy_name)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 3) BasketEmptyPolicy — basket boşsa veto eder (ön-kontrol).
// ─────────────────────────────────────────────────────────────────────────────

pub struct BasketEmptyPolicy;

impl BasketEmptyPolicy {
    pub fn evaluate_size(&self, basket_size: usize) -> ExecutionDecision {
        if basket_size == 0 {
            ExecutionDecision::Skip {
                reason: "Basket boş — emir gönderilmeyecek".to_string(),
            }
        } else {
            ExecutionDecision::Allow
        }
    }
}

impl ExecutionPolicy for BasketEmptyPolicy {
    fn name(&self) -> &str { "basket_empty" }
    fn evaluate(&self, ctx: &ExecutionContext<'_>) -> ExecutionDecision {
        self.evaluate_size(ctx.basket_size)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Default chain
// ─────────────────────────────────────────────────────────────────────────────

pub fn default_chain() -> Vec<Box<dyn ExecutionPolicy>> {
    vec![
        Box::new(MarketHoursPolicy),
        Box::new(IdleStrategyPolicy),
        Box::new(BasketEmptyPolicy),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(strategy: Option<&'a str>, hour: u32, basket: usize) -> ExecutionContext<'a> {
        ExecutionContext {
            signal: &Signal::Buy,
            symbol: "BTCUSDT",
            amount: 1.0,
            strategy_name: strategy,
            market_hours: None,
            current_hour: hour,
            basket_size: basket,
        }
    }

    // ── MarketHoursPolicy ───────────────────────────────────────────────

    #[test]
    fn market_hours_allows_when_no_window() {
        let p = MarketHoursPolicy;
        assert!(p.evaluate_hours(None, 3).is_allow());
    }

    #[test]
    fn market_hours_allows_within_window() {
        let p = MarketHoursPolicy;
        assert!(p.evaluate_hours(Some((9, 18)), 10).is_allow());
        assert!(p.evaluate_hours(Some((9, 18)), 9).is_allow());
    }

    #[test]
    fn market_hours_skips_outside_window() {
        let p = MarketHoursPolicy;
        let d = p.evaluate_hours(Some((9, 18)), 18); // end exclusive
        assert!(!d.is_allow());
        assert!(d.skip_reason().unwrap().contains("kapalı"));
    }

    // ── IdleStrategyPolicy ──────────────────────────────────────────────

    #[test]
    fn idle_policy_skips_idle_prefix() {
        let p = IdleStrategyPolicy;
        let d = p.evaluate_name(Some("IDLE_PROTECT"));
        assert!(!d.is_allow());
        assert!(d.skip_reason().unwrap().contains("Savunma"));
    }

    #[test]
    fn idle_policy_allows_real_strategy() {
        let p = IdleStrategyPolicy;
        assert!(p.evaluate_name(Some("SUPERTREND")).is_allow());
        assert!(p.evaluate_name(None).is_allow());
    }

    // ── BasketEmptyPolicy ───────────────────────────────────────────────

    #[test]
    fn basket_empty_skips_when_size_zero() {
        let p = BasketEmptyPolicy;
        let d = p.evaluate_size(0);
        assert!(!d.is_allow());
        assert!(d.skip_reason().unwrap().contains("Basket"));
    }

    #[test]
    fn basket_empty_allows_when_not_empty() {
        let p = BasketEmptyPolicy;
        assert!(p.evaluate_size(3).is_allow());
    }

    // ── Chain davranışları ──────────────────────────────────────────────

    #[test]
    fn default_chain_has_three_policies_in_order() {
        let c = default_chain();
        let names: Vec<&str> = c.iter().map(|p| p.name()).collect();
        assert_eq!(names, vec!["market_hours", "idle_strategy", "basket_empty"]);
    }

    #[test]
    fn chain_allows_when_all_policies_allow() {
        let c = default_chain();
        let ctx = ctx(Some("SUPERTREND"), 12, 3);
        let (d, who) = evaluate_chain(&c, &ctx);
        assert!(d.is_allow());
        assert!(who.is_none());
    }

    #[test]
    fn chain_short_circuits_on_first_skip() {
        // MarketHours bilinçli olarak skip dönsün diye open window kapatılır,
        // sonra IdleStrategy de IDLE veto eder; ama chain ilk policy'de durur.
        let c = default_chain();
        let mut ctx = ctx(Some("IDLE_PROTECT"), 3, 3);
        ctx.market_hours = Some((9, 18));
        let (d, who) = evaluate_chain(&c, &ctx);
        assert!(!d.is_allow());
        assert_eq!(who.as_deref(), Some("market_hours"));
    }

    #[test]
    fn chain_runs_through_to_idle_policy_when_market_open() {
        let c = default_chain();
        let mut ctx = ctx(Some("IDLE_PROTECT"), 12, 3);
        ctx.market_hours = Some((9, 18));
        let (d, who) = evaluate_chain(&c, &ctx);
        assert!(!d.is_allow());
        assert_eq!(who.as_deref(), Some("idle_strategy"));
    }
}
