// src/robot/robotic_loop.rs - Srivastava ATP Reaktif Mimari Çekirdeği
// 4 Büyük Krallık (Bakanlık) Düzeni
use crate::prelude::*;
use std::sync::{Arc, RwLock, atomic::{AtomicBool, AtomicU64, Ordering}};
use std::collections::{HashMap, VecDeque};
use tokio::time::Instant;
use crate::core::model::{PositionModel, RoboticLoopConfig, ClosedTradeModel};
use crate::robot::risk::RiskGate;
use crate::robot::data_pipeline::PipelineStatus;
use crate::robot::sr_detector::SrZone;
use crate::robot::state::symbol_orchestrator::SymbolOrchestrator;
use rusqlite::Connection;

/// 💸 Yürütme maliyetleri (komisyon + slipaj) izlemesi.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecutionCosts {
    pub total_cost_usd: f64,
    pub commission_usd: f64,
    pub slippage_usd:   f64,
    pub trade_count:    usize,
}

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
    pub peak_equity: f64,
    pub live_positions: Arc<RwLock<HashMap<String, PositionModel>>>,
    pub live_closed_trades: Arc<RwLock<Vec<ClosedTradeModel>>>,
    /// Komisyon + slipaj birikimleri (UI raporu + risk hesaplama için).
    pub live_execution_costs: Arc<RwLock<ExecutionCosts>>,
    /// Equity tarihçesi (en eski → en yeni). Sparkline ve drawdown hesabı için.
    /// Ana döngü her ~2.5 sn'de bir push eder, kapasite 120 nokta (~5 dk).
    pub equity_history: Arc<RwLock<VecDeque<f64>>>,
    /// 💱 Live mode'da sembol başına entry/SL/TP order_id eşlemesi.
    /// Açılışta yazılır, kapatma anında hedefli cancel_order için okunur.
    pub live_orders: Arc<RwLock<HashMap<String, crate::core::model::LiveOrderRefs>>>,
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

    /// 🧠 Otonom Öğrenme Merkezi (drift, pattern, post-trade learning, evolution).
    /// Ana döngü her tur drift güncellemesi ve periyodik evrim için bu hub'a yazar.
    pub intelligence_hub: Arc<RwLock<crate::robot::ml_engine::intelligence_hub::IntelligenceHub>>,
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
    pub phase: String,
    pub download_active: bool,
    pub triggers: std::collections::HashMap<String, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub live_price: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, f64>>>,
    
    // [DÜZELTME]: live_sr_zones'un içindeki veri tipi anayasal SrZone olarak tescillendi 🎯
    pub live_sr_zones: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, Vec<SrZone>>>>,
    pub symbol_orchestrator: std::sync::Arc<std::sync::RwLock<SymbolOrchestrator>>,
    /// Ana otonom döngünün en son nabız anı (UNIX epoch saniye). Heartbeat task'ı bu alanı
    /// okuyarak `main_loop` adımının gerçekten ilerleyip ilerlemediğini denetler.
    pub last_loop_tick: Arc<AtomicU64>,
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
    /// Çalışma zamanı pipeline durumu (chain_steps + anomalies kaynağı).
    pub live_pipeline: Arc<RwLock<PipelineStatus>>,
    /// Otonom onarım (auto-fix) günlüğü — son N kayıt UI'a yansır.
    pub repair_log: VecDeque<String>,
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

    /// 💱 LIVE Mode Köprüsü (config.trading_mode == Live + API key varsa)
    /// None ise tüm pozisyon açış/kapanış paper-mode'da kalır.
    pub live_executor: Option<Arc<crate::robot::engines::binance_executor::BinanceFuturesExecutor>>,
    /// LIVE_DRY_RUN=true ise executor olsa bile gerçek emir gönderilmez,
    /// sadece "şu emir gönderilecekti" log'u atılır. Paper akışı çalışır.
    pub live_dry_run: bool,
    /// Tek emir için sert notional tavan (USD). Env LIVE_MAX_NOTIONAL_USD,
    /// default $100. RiskGate'in üstünde bir bariyer.
    pub live_max_notional_usd: f64,

    /// 📣 Telegram push kanalı (kritik olaylar için). TELEGRAM_BOT_TOKEN +
    /// TELEGRAM_CHAT_ID set değilse None — kod sessizce çalışmaya devam eder.
    pub notifier: Option<Arc<crate::robot::infra::telegram_notifier::TelegramNotifier>>,

    /// 📝 Periyodik dosya logu (logs/robotic_trading.log + logs/trades.jsonl).
    /// `TRADING_LOGGER_DISABLE=1` set ise None; aksi halde her zaman aktif.
    /// SIGNAL/TRADE_OPEN/TRADE_CLOSE/RISK_BLOCK/ERROR olayları için kullanılır.
    pub trading_logger: Option<Arc<crate::robot::infra::logger::TradingLogger>>,

    // Global Durdurma Sinyalleri
    pub app_stop_signal: Arc<AtomicBool>,
    pub pause_signal: Arc<AtomicBool>,
}

impl AppState {
    /// Yeni bir otonom organizma oluşturur (Srivastava ATP - F1 Mühürlü)
    pub fn new(config: RoboticLoopConfig) -> Self {
        // Global durdurma sinyallerini oluştur
        let stop_sig = Arc::new(AtomicBool::new(false));
        
        // Bakanlıkları varsayılan (safe) değerlerle kur
        // Live positions Arc paylaşılır: FinanceVault yazar, SymbolOrchestrator okur.
        let live_positions = Arc::new(RwLock::new(HashMap::new()));

        let mut initial_history = VecDeque::with_capacity(120);
        initial_history.push_back(config.capital);
        let finance = FinanceVault {
            equity: config.capital,
            starting_capital: config.capital,
            peak_equity: config.capital,
            live_positions: Arc::clone(&live_positions),
            live_closed_trades: Arc::new(RwLock::new(Vec::new())),
            live_execution_costs: Arc::new(RwLock::new(ExecutionCosts::default())),
            equity_history: Arc::new(RwLock::new(initial_history)),
            live_orders: Arc::new(RwLock::new(HashMap::new())),
        };

        let intelligence_hub = crate::robot::ml_engine::intelligence_hub::IntelligenceHub::new(
            crate::evolution::AutonomousController::new(
                crate::evolution::AutonomousControllerConfig::default(),
            ),
        );
        let brain = BrainBox {
            ml_signal: "HOLD".to_string(),
            ml_confidence: 0.0,
            hyperopt_score: 0.0,
            best_params: HashMap::new(),
            live_strategy: Arc::new(RwLock::new("Default".to_string())),
            thresholds: AdaptiveThresholds { drift_baseline: 0.15, volatility_regime: 1.0 },
            drift_history: VecDeque::with_capacity(100),
            intelligence_hub: Arc::new(RwLock::new(intelligence_hub)),
        };

        // --- 🧬 F1: ORKESTRATÖRÜNÜN SAFİLEŞTİRİLMESİ VE FİLO KAYDI ---
        // 1. SymbolOrchestrator nesnesini ham olarak üret
        let mut symbol_orchestrator = SymbolOrchestrator::new(16, Arc::clone(&live_positions));

        // 2. Sabitlenmiş elit sembolleri (Pinned Symbols) doğrudan orkestratöre kaydet
        for sym in &config.pinned_symbols {
            symbol_orchestrator.register(
                sym, 
                &config.market, 
                &config.interval
            );
        }

        // 3. Kalıcı hafızadaki (SQLite) en çok kazandıran sembolleri hasat et (Elite Fleet)
        // Eğer veritabanı yolu üzerinde kayıtlı semboller varsa, onları da otonom listeye ekler
        if let Ok(elite_symbols) = crate::persistence::reader::list_symbols(&config.db_path) {
            for sym in elite_symbols {
                // Mükerrer kaydı engellemek için kontrol bariyeri (Double-execution koruması)
                if !config.pinned_symbols.contains(&sym) {
                    symbol_orchestrator.register(
                        &sym,
                        &config.market,
                        &config.interval
                    );
                }
            }
        }

        // 4. Hazırlanan ve sembollerle doldurulan orkestratörü FleetCommand'a teslim et
        let fleet = FleetCommand {
            phase: "Idle".to_string(),
            download_active: false,
            triggers: Self::init_default_triggers(),
            live_price: Arc::new(RwLock::new(HashMap::new())),
            live_sr_zones: Arc::new(RwLock::new(HashMap::new())),
            symbol_orchestrator: Arc::new(RwLock::new(symbol_orchestrator)),
            // Başlangıç değeri 0 → heartbeat'i hemen DataStall uyarısı vermesin diye
            // sonradan ana döngünün ilk turunda doldurulur.
            last_loop_tick: Arc::new(AtomicU64::new(0)),
        };

        let guardian = GuardianShield {
            log: VecDeque::with_capacity(300),
            diag_alerts: VecDeque::with_capacity(20),
            risk_gate: RiskGate::default(),
            anomaly_count: 0,
            db_conn: Connection::open(&config.db_path).ok(),
            live_pipeline: Arc::new(RwLock::new(PipelineStatus::new())),
            repair_log: VecDeque::with_capacity(100),
        };

        // Adli Tamirat Günlüğü
        log::info!(target:"STATE_INIT",
            "F1: Otonom filo, pinned ve SQLite geçmiş sembolleriyle donatılarak ayağa kaldırıldı. Seviye INFO"
        );

        // 💱 LIVE Mode köprüsü — TradingMode::Live + API key varsa BinanceFuturesExecutor kurulur.
        // Yoksa otomatik olarak paper-fallback (live_executor = None).
        let live_executor = if matches!(config.trading_mode, crate::core::model::TradingMode::Live) {
            match (config.get_api_key(), config.get_secret_key()) {
                (Some(k), Some(s)) if !k.is_empty() && !s.is_empty() => {
                    log::info!(target:"STATE_INIT",
                        "💱 Live mode aktif: BinanceFuturesExecutor kuruldu (market={})", config.market);
                    Some(Arc::new(
                        crate::robot::engines::binance_executor::BinanceFuturesExecutor::new_for_market(
                            k, s, /*is_paper=*/false, &config.market,
                        )
                    ))
                }
                _ => {
                    log::warn!(target:"STATE_INIT",
                        "⚠️ TradingMode::Live seçildi ama API key/secret yok → Paper-fallback");
                    None
                }
            }
        } else {
            None
        };

        // LIVE_DRY_RUN: executor olsa bile gerçek emir gönderme; sadece "gönderilecekti" log'la.
        let live_dry_run = std::env::var("LIVE_DRY_RUN")
            .map(|v| v == "true" || v == "1").unwrap_or(false);
        // LIVE_MAX_NOTIONAL_USD: tek emir için sert tavan. Default $100 (test güvenli).
        let live_max_notional_usd = std::env::var("LIVE_MAX_NOTIONAL_USD")
            .ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(100.0).max(0.0);

        // 📣 Telegram notifier — TELEGRAM_BOT_TOKEN+TELEGRAM_CHAT_ID set ise kurulur.
        // None: push devre dışı; tg_notify! sessizce geçer.
        let notifier = crate::robot::infra::telegram_notifier::TelegramNotifier::from_env()
            .map(Arc::new);
        if notifier.is_some() {
            log::info!(target:"STATE_INIT", "📣 Telegram notifier aktif (TELEGRAM_BOT_TOKEN/CHAT_ID set)");
        } else {
            log::info!(target:"STATE_INIT", "📣 Telegram notifier devre dışı (env yok)");
        }

        // 📝 TradingLogger — runtime SIGNAL/TRADE/RISK_BLOCK olayları için kalıcı dosya logu.
        // `TRADING_LOGGER_DISABLE=1` veya `=true` ise devre dışı bırakılır.
        let trading_logger_disabled = std::env::var("TRADING_LOGGER_DISABLE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let trading_logger = if trading_logger_disabled {
            log::info!(target:"STATE_INIT", "📝 TradingLogger devre dışı (TRADING_LOGGER_DISABLE)");
            None
        } else {
            match crate::robot::infra::logger::TradingLogger::new(
                "logs/robotic_trading.log",
                "logs/trades.jsonl",
            ) {
                Ok(lg) => {
                    log::info!(target:"STATE_INIT",
                        "📝 TradingLogger aktif: logs/robotic_trading.log + logs/trades.jsonl");
                    Some(Arc::new(lg))
                }
                Err(e) => {
                    log::warn!(target:"STATE_INIT",
                        "📝 TradingLogger kurulamadı ({:?}) — dosya logu devre dışı", e);
                    None
                }
            }
        };

        Self {
            config,
            finance,
            brain,
            fleet,
            guardian,
            live_executor,
            live_dry_run,
            live_max_notional_usd,
            notifier,
            trading_logger,
            app_stop_signal: stop_sig,
            pause_signal: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Hem `push_log`'a yazar hem (notifier varsa) Telegram'a yollar.
    /// `key`: throttle anahtarı (örn. "BALANCE-AUTOFIX"). Aynı key cooldown
    /// süresince tekrar yollanmaz; UI log'una her zaman düşer.
    pub fn push_alert(
        &mut self,
        key: &str,
        severity: crate::robot::infra::telegram_notifier::Severity,
        msg: String,
    ) {
        let formatted = crate::robot::infra::telegram_notifier::format_message(severity, &msg);
        self.push_log(formatted);
        if let Some(n) = self.notifier.as_ref() {
            n.notify(key, severity, &msg);
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
        let _regime_volatility = self.fleet.calculate_current_volatility();
        
        // 2. Risk Toleransı (Kasa durumuna göre dinamik vites)
        let risk_multiplier = self.finance.calculate_risk_appetite(); 

        // 3. Otonom Karar: "Tetiklemeli miyim?"
        if self.brain.ml_confidence * risk_multiplier > self.guardian.dynamic_safety_barrier() {
             self.fleet.triggers.get("execution").map(|t| t.store(true, Ordering::Relaxed));
             self.push_log("🚀 Otonom Karar: Koşullar optimal, harekât başlatıldı.".into());
        }
    }
}
