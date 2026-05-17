// src/robot/robotic_loop.rs - Srivastava ATP Reaktif Mimari Çekirdeği
// 4 Büyük Krallık (Bakanlık) Düzeni

use std::sync::{Arc, RwLock, atomic::{AtomicBool, Ordering}};
use std::collections::{HashMap, VecDeque};
use tokio::time::Instant;
use crate::core::model::{PositionModel, RoboticLoopConfig, ClosedTradeModel};
use crate::robot::risk::RiskGate;
use rusqlite::Connection;

/// 🧬 Srivastava ATP - Otonom Eşik Muhafazası
#[derive(Debug, Clone, Copy, Default)]
pub struct AdaptiveThresholds {
    pub drift_baseline: f64,      // Sapma ortalaması
    pub volatility_regime: f64,   // Pazarın o anki oynaklık karakteri (Örn: ATR standard sapması)
}

// =============================================================================
// 1. BAKANLIKLAR (ALT BİRİMLER)
// =============================================================================

/// 🏦 FİNANS BAKANLIĞI: Cüzdan, Bakiyeler ve Açık Pozisyonlar
pub struct FinanceVault {
    pub equity: f64,
    pub starting_capital: f64,
    pub live_positions: Arc<RwLock<HashMap<String, PositionModel>>>,
    pub live_closed_trades: Arc<RwLock<Vec<ClosedTradeModel>>>,
}

impl FinanceVault {
    /// Mevcut sermayenin başlangıca oranı → risk iştahı çarpanı.
    /// >1.0 ise kasada kar var (daha cesur), <1.0 ise zarar (daha temkinli). [0.5, 2.0] aralığına clamp.
    pub fn calculate_risk_appetite(&self) -> f64 {
        if self.starting_capital <= 0.0 { return 1.0; }
        (self.equity / self.starting_capital).clamp(0.5, 2.0)
    }
}

/// 🧠 İSTİHBARAT BAŞKANLIĞI: ML Tahminleri, Sinyaller ve HyperOpt Parametreleri
pub struct BrainBox {
    pub ml_signal: String,
    pub ml_confidence: f64,
    pub hyperopt_score: f64,
    pub best_params: HashMap<String, f64>,
    pub live_strategy: Arc<RwLock<String>>,

    // YENİ EKLEME: Sabitleri yıkan dinamik bariyer alanı buraya entegre edilir.
    pub thresholds: AdaptiveThresholds, 
    pub drift_history: std::collections::VecDeque<f64>, // Son 100 döngünün sapma hafızası
}

/// 🧠 OTONOM DEĞERLENDİRME: `ml_engine::IntelligenceHub`'dan gelen ham 
/// drift verisini, kendi geçmiş hafızasıyla (drift_history) kıyaslar.
impl BrainBox {
    /// 🧠 OTONOM DEĞERLENDİRME: `ml_engine::IntelligenceHub`'dan gelen ham 
    /// drift verisini, kendi geçmiş hafızasıyla (drift_history) kıyaslar.
    pub fn should_retrain(&self, current_drift: f64) -> bool {
        let history = &self.drift_history;
        let n = history.len();
        
        if n < 10 { return false; }
        
        let sum: f64 = history.iter().sum();
        let mean = sum / n as f64;
        
        let variance_sum: f64 = history.iter()
            .map(|&d| (d - mean).powi(2))
            .sum();
        let std_dev = (variance_sum / n as f64).sqrt();
        
        let dynamic_threshold = mean + (2.0 * std_dev); 
        
        current_drift > dynamic_threshold
    }
}

/// ⚔️ HAREKÂT VE FİLO KOMUTANLIĞI: İş Akışı, Semboller ve Tetikleyiciler
pub struct FleetCommand {
    pub phase: String, // "Idle", "Download", "ML", "Trade"
    pub download_active: bool,
    pub triggers: HashMap<String, Arc<AtomicBool>>,
    pub live_price: Arc<RwLock<HashMap<String, f64>>>,
}

impl FleetCommand {
    /// Live price snapshot'ından kaba bir volatilite tahmini.
    /// Mevcut fiyatların standart sapması / ortalaması (CV). Veri yoksa 0.0.
    pub fn calculate_current_volatility(&self) -> f64 {
        let guard = match self.live_price.read() {
            Ok(g) => g,
            Err(_) => return 0.0,
        };
        let prices: Vec<f64> = guard.values().copied().filter(|p| *p > 0.0).collect();
        let n = prices.len();
        if n < 2 { return 0.0; }
        let mean = prices.iter().sum::<f64>() / n as f64;
        if mean == 0.0 { return 0.0; }
        let variance = prices.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / n as f64;
        (variance.sqrt() / mean).min(1.0)
    }
}

/// 🛡️ ADLİ VE SAĞLIK MUHAFIZLIĞI: Risk Kontrolü ve Hata Kayıtları
pub struct GuardianShield {
    pub log: VecDeque<String>,
    pub diag_alerts: VecDeque<String>,
    pub risk_gate: RiskGate,
    pub anomaly_count: u32,
    pub db_conn: Option<Connection>,
}

impl GuardianShield {
    /// Anomali yoğunluğuna göre dinamik güven barajı.
    /// Anomali yoksa düşük bar (0.5), arttıkça daha sıkı (0.5 → 0.9 doğrusal).
    pub fn dynamic_safety_barrier(&self) -> f64 {
        let extra = (self.anomaly_count as f64 * 0.05).min(0.4);
        0.5 + extra
    }
}

// =============================================================================
// 2. ANA MERKEZ (APP STATE)
// =============================================================================

/// Srivastava ATP - Otonom Sistem Merkezi (Orkestratör)
pub struct AppState {
    pub config: RoboticLoopConfig,
    
    // Bakanlıklar
    pub finance: FinanceVault,
    pub brain: BrainBox,
    pub fleet: FleetCommand,
    pub guardian: GuardianShield,

    // Global Durdurma Sinyalleri
    pub app_stop_signal: Arc<AtomicBool>,
    pub pause_signal: Arc<AtomicBool>,
}

impl AppState {
    /// Yeni bir otonom organizma oluşturur
    pub fn new(config: RoboticLoopConfig) -> Self {
        // Global durdurma sinyallerini oluştur
        let stop_sig = Arc::new(AtomicBool::new(false));
        
        // Bakanlıkları varsayılan (safe) değerlerle kur
        let finance = FinanceVault {
            equity: config.capital,
            starting_capital: config.capital,
            live_positions: Arc::new(RwLock::new(HashMap::new())),
            live_closed_trades: Arc::new(RwLock::new(Vec::new())),
        };

        let brain = BrainBox {
            ml_signal: "HOLD".to_string(),
            ml_confidence: 0.0,
            hyperopt_score: 0.0,
            best_params: HashMap::new(),
            live_strategy: Arc::new(RwLock::new("Default".to_string())),
            thresholds: AdaptiveThresholds { drift_baseline: 0.0, volatility_regime: 0.0 },
            drift_history: VecDeque::with_capacity(100),
        };

        let fleet = FleetCommand {
            phase: "Idle".to_string(),
            download_active: false,
            triggers: Self::init_default_triggers(),
            live_price: Arc::new(RwLock::new(HashMap::new())),
        };

        let guardian = GuardianShield {
            log: VecDeque::with_capacity(300),
            diag_alerts: VecDeque::with_capacity(20),
            risk_gate: RiskGate::default(),
            anomaly_count: 0,
            db_conn: Connection::open(&config.db_path).ok(),
        };

        Self {
            config,
            finance,
            brain,
            fleet,
            guardian,
            app_stop_signal: stop_sig,
            pause_signal: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Tüm otonom tetikleyicileri (m, b, d, s) tek seferde mühürler
    fn init_default_triggers() -> HashMap<String, Arc<AtomicBool>> {
        let mut t = HashMap::new();
        t.insert("ml".to_string(), Arc::new(AtomicBool::new(false)));
        t.insert("backtest".to_string(), Arc::new(AtomicBool::new(false)));
        t.insert("download".to_string(), Arc::new(AtomicBool::new(false)));
        t.insert("screener".to_string(), Arc::new(AtomicBool::new(false)));
        t
    }

    /// Adli Log Kaydı: guardian üzerinden mühürlenir
    pub fn push_log(&mut self, msg: String) {
        let ts = chrono::Local::now().format("%H:%M:%S").to_string();
        self.guardian.log.push_back(format!("[{}] {}", ts, msg));
        if self.guardian.log.len() > 300 { self.guardian.log.pop_front(); }
    }
        /// Otonom Karar Verici: Bakanlıklar arası dengeyi gözeterek aksiyon alır.
    pub fn orchestrate_autonomy(&mut self) {
        // 1. Pazar Rejimi Tespiti (Vites Belirleme)
        let regime_volatility = self.fleet.calculate_current_volatility();
        
        // 2. Risk Toleransı (Kasa durumuna göre dinamik vites)
        let risk_multiplier = self.finance.calculate_risk_appetite(); 

        // 3. Otonom Karar: "Tetiklemeli miyim?"
        if self.brain.ml_confidence * risk_multiplier > self.guardian.dynamic_safety_barrier() {
             self.fleet.triggers.get("execution").map(|t| t.store(true, Ordering::Relaxed));
             self.push_log("🚀 Otonom Karar: Koşullar optimal, harekât başlatıldı.".into());
        }
    }
}
