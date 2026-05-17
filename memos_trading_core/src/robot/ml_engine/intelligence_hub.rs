// robot/ml_engine/intelligence_hub.rs - Srivastava ATP AI ve Evrim Karar Merkezi
//
// Modernizasyon Standartları:
// 1. Match-Guard ile evrimsel hiyerarşi kontrolü
// 2. Kapsüllü geri besleme (Feedback loop) mekanizması
// 3. Fonksiyonel ticaret hafızası yönetimi
// 4. Panic-free HashMap ve Rejim eşleştirme
use crate::prelude::*; // Evrensel çekirdek kontratları ve AdaptiveThresholds yapısını bağlar
use std::collections::{HashMap, VecDeque};
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
        }
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
