// robot/ml_engine/intelligence_hub.rs - Srivastava ATP AI ve Evrim Karar Merkezi
//
// Modernizasyon Standartları:
// 1. Match-Guard ile evrimsel hiyerarşi kontrolü
// 2. Kapsüllü geri besleme (Feedback loop) mekanizması
// 3. Fonksiyonel ticaret hafızası yönetimi
// 4. Panic-free HashMap ve Rejim eşleştirme
use crate::prelude::*; // Evrensel çekirdek kontratları ve AdaptiveThresholds yapısını bağlar
use std::collections::{HashMap, VecDeque};
use std::time::Instant;
use crate::evolution::AutonomousController;
use crate::robot::ml_engine::{DriftDetector, TradePatternClassifier, StrategyScorer};
use crate::core::types::PositionId;

//use crate::robot::{App};

use std::sync::{Arc, Mutex};
use crate::robot::robotic_loop::AppState;

/// §88.4: IntelligenceHub - Sistemin Öğrenme ve Adaptasyon Merkezi
/// 🧠 İSTİHBARAT BAŞKANLIĞI (ASIL İŞÇİ MOTOR)
pub struct IntelligenceHub {
    pub controller: AutonomousController,
    pub drift_detector: DriftDetector,
    pub pattern_classifier: TradePatternClassifier,
    pub strategy_scorer: StrategyScorer,
    /// §88.5: Bekleyen İşlemler - (Piyasa Rejimi, Strateji Adı)
    pub pending_trades: HashMap<PositionId, (crate::evolution::MarketRegime, String)>,

    // YENİ EKLEME: Sabitleri yıkan dinamik bariyer alanları hafıza hücresine kilitlendi
    pub thresholds: AdaptiveThresholds,
    pub drift_history: VecDeque<f64>, // Son 100 döngünün sapma hafıza havuzu

    /// Drift-tetikli ML retrain'in son ateşlenme zamanı (process-içi).
    /// `tick_intelligence_hub` `drift_retrain_armed(cooldown)` ile burayı
    /// kontrol eder; sürekli yüksek drift'te her tick'te yeni trigger
    /// üretilmesin diye `ML_DRIFT_COOLDOWN_SECS` env (default 600s = 10 dk)
    /// kadar bekleme tutar. Process restart'ta None'a döner (kasıtlı —
    /// yeniden başlatma drift'i tekrar değerlendirsin).
    pub last_drift_retrain_at: Option<Instant>,
}

impl IntelligenceHub {
    pub fn new(controller: AutonomousController) -> Self {
        Self {
            controller,
            drift_detector: DriftDetector::default(),
            pattern_classifier: TradePatternClassifier::default(),
            strategy_scorer: StrategyScorer::default(),
            pending_trades: HashMap::new(),

            // --- 🧬 SRIVASTAVA FAIL-SAFE BAŞLANGIÇ DEĞERLERİ ---
            // Yeni eklenen alanlar burada nesne üretilirken RAM'e güvenle kilitlenir:
            thresholds: AdaptiveThresholds::default(),
            drift_history: VecDeque::with_capacity(100), // Maksimum 100 döngü kapasiteli temiz kuyruk
            last_drift_retrain_at: None,
        }
    }

    // ── Drift-tetikli retrain cooldown yardımcıları ──────────────────────
    //
    // İki yüzlü API (RiskFilter chain pattern'ı ile aynı çizgide):
    //   (a) `drift_retrain_armed(cooldown)` — trait/saf yüz, mutate etmez,
    //       cooldown boyunca dolmadıysa false döner.
    //   (b) `mark_drift_retrain_fired()` — trigger fire edildikten sonra
    //       timestamp'i şimdi yapar.
    //
    // Test edilebilirlik için `drift_retrain_armed_at(now, cooldown)`
    // varyantı zaman injekte edilmiş hâlidir.

    /// Şu an cooldown süresi geçmiş mi? İlk çağrıda (None) her zaman armed.
    pub fn drift_retrain_armed(&self, cooldown_secs: u64) -> bool {
        self.drift_retrain_armed_at(Instant::now(), cooldown_secs)
    }

    /// Saf çekirdek: now injekte edilir. Birim test bunu kullanır.
    pub fn drift_retrain_armed_at(&self, now: Instant, cooldown_secs: u64) -> bool {
        match self.last_drift_retrain_at {
            None => true,
            Some(t) => now.saturating_duration_since(t).as_secs() >= cooldown_secs,
        }
    }

    /// Drift-tetikli retrain ateşlendikten sonra çağrılır; cooldown timer'ını
    /// şimdi olarak işaretler.
    pub fn mark_drift_retrain_fired(&mut self) {
        self.last_drift_retrain_at = Some(Instant::now());
    }

    /// Saf çekirdek: now injekte edilir (test için).
    pub fn mark_drift_retrain_fired_at(&mut self, now: Instant) {
        self.last_drift_retrain_at = Some(now);
    }

    /// 🧠 OTONOM DEĞERLENDİRME: ML modelinin piyasadan sapıp sapmadığını dinamik olarak ölçer
    pub fn should_retrain(&self, current_drift: f64) -> bool {
        let history = &self.drift_history;
        let n = history.len();
        
        // Soğuk Başlangıç Koruması: Hafızada yeterli veri yoksa (ilk 10 döngü) varsayılan baseline'a bak
        if n < 10 { 
            return current_drift > self.thresholds.drift_baseline; 
        }
        
        // 1. Geçmiş sapmaların ortalamasını (mean) hesapla
        let sum: f64 = history.iter().sum();
        let mean = sum / n as f64;
        
        // 2. Standart Sapmayı (Std Dev) hesapla (Single-pass variance)
        let variance_sum: f64 = history.iter()
            .map(|&d| (d - mean).powi(2))
            .sum();
        let std_dev = (variance_sum / n as f64).sqrt();
        
        // 3. DİNAMİK BARİKAT: Eşik = Ortalama + (2.0 * Standart Sapma)
        let dynamic_threshold = mean + (2.0 * std_dev); 
        
        current_drift > dynamic_threshold
    }

    /// §88.6: Evrimsel Döngü Kontrolü (Legacy: maybe_evolve)
    /// Pattern Matching ile otonom tetikleme.
    pub fn tick_evolution(&mut self) {
        match (self.controller.evolution_enabled, self.controller.should_evolve()) {
            (true, true) => {
                log::info!("Srivastava-ATP: Popülasyon evrimi tetiklendi. Otonom adaptasyon başlıyor.");
                self.controller.evolve_population();
            },
            (true, false) => { /* Henüz evrim zamanı değil, denge korunuyor */ },
            _ => { /* Evrimsel motor devre dışı */ }
        }
    }

    /// §88.7: Post-Trade Feedback - Kapanan işlemden otonom öğrenme.
    pub fn learn_from_exit(&mut self, pos_id: PositionId, pnl_pct: f64) {
        // Functional Entry Management: İşlemi hafızadan çıkar ve işle
        if let Some((regime, strategy)) = self.pending_trades.remove(&pos_id) {
            // Evrimsel beyne tecrübe aktarımı
            self.controller.learn_from_trade(pnl_pct, &regime, &strategy);
            
            // Reaktif Evrim: Eğer zarar edildiyse, stratejiyi otonom sorgula
            match pnl_pct {
                p if p < 0.0 => {
                    log::warn!("Srivastava-ATP: Kayıplı işlem sonrası mikro-evrim denetleniyor. PnL: {:.2}%", p);
                    self.tick_evolution();
                },
                _ => log::info!("Srivastava-ATP: Başarılı işlem tecrübe hanesine mühürlendi."),
            }
        }
    }

    /// Yeni bir işlemi evrimsel takip listesine mühürler.
    pub fn track_trade(&mut self, id: PositionId, regime: crate::evolution::MarketRegime, strategy: String) {
        self.pending_trades.insert(id, (regime, strategy));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn fresh_hub() -> IntelligenceHub {
        use crate::evolution::AutonomousControllerConfig;
        IntelligenceHub::new(AutonomousController::new(AutonomousControllerConfig::default()))
    }

    #[test]
    fn drift_retrain_armed_when_never_fired() {
        let hub = fresh_hub();
        assert!(hub.drift_retrain_armed(600),
            "ilk çağrıda (timestamp yok) armed olmalı");
    }

    #[test]
    fn drift_retrain_blocked_inside_cooldown() {
        let mut hub = fresh_hub();
        let t0 = Instant::now();
        hub.mark_drift_retrain_fired_at(t0);
        // 5 dk sonra hâlâ cooldown'da (default 600s = 10 dk).
        let t1 = t0 + Duration::from_secs(300);
        assert!(!hub.drift_retrain_armed_at(t1, 600),
            "cooldown içinde armed olmamalı");
    }

    #[test]
    fn drift_retrain_rearms_after_cooldown_passes() {
        let mut hub = fresh_hub();
        let t0 = Instant::now();
        hub.mark_drift_retrain_fired_at(t0);
        // Tam eşiğe ulaşır (saturating_duration_since(>=cooldown)).
        let t1 = t0 + Duration::from_secs(600);
        assert!(hub.drift_retrain_armed_at(t1, 600),
            "tam cooldown sonunda yeniden armed olmalı");
    }

    #[test]
    fn drift_retrain_zero_cooldown_always_armed() {
        let mut hub = fresh_hub();
        let t0 = Instant::now();
        hub.mark_drift_retrain_fired_at(t0);
        // cooldown=0 → her zaman armed (cooldown devre dışı modu).
        assert!(hub.drift_retrain_armed_at(t0, 0),
            "cooldown=0 ile her tick fire edebilmeli");
    }

    #[test]
    fn mark_then_arm_cycle_is_monotonic() {
        let mut hub = fresh_hub();
        let t0 = Instant::now();
        hub.mark_drift_retrain_fired_at(t0);
        assert!(!hub.drift_retrain_armed_at(t0 + Duration::from_secs(1), 60));
        // İkinci kez geç-zamanlı mark; cooldown timer ileri sarar.
        let t1 = t0 + Duration::from_secs(120);
        hub.mark_drift_retrain_fired_at(t1);
        assert!(!hub.drift_retrain_armed_at(t1 + Duration::from_secs(1), 60));
        assert!(hub.drift_retrain_armed_at(t1 + Duration::from_secs(60), 60));
    }
}
