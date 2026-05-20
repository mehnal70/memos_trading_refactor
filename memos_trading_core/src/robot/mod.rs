// robot/mod.rs - Srivastava ATP Merkezi Entegrasyon ve Modül Santralı

// --- 1. TEMEL MODÜL TANIMLARI ---

// Ortak Alt Yapı ve Çekirdek Modüller
pub mod engines;    //robot
pub mod infra;      //robot
pub mod logic;      //robot
pub mod config;     //robot
pub mod state; // state/mod.rs veya state.rs dosyasını tanır

//pub mod error;      //dosya - robot klasörünün altında

// Tier 1: Veri ve İnfaz (Core Execution)
pub mod data_fetcher;
pub mod data_pipeline;
pub mod diagnostics;     // Faz 4 c4: plug-in keşfedilebilirlik snapshot'ı
pub mod execution;       // Faz 4 c3: yürütme öncesi policy plug-in zinciri
pub mod order_management;
//pub mod indicators_v2_ek;
//pub mod executor;   // dosya
//pub mod interfaces; // dosya


// Tier 2: Stratejik Zeka ve Risk (Intelligence)
pub mod ml_engine;
pub mod portfolio_manager;
pub mod symbol_manager;
pub mod risk;
pub mod parameters;  // Faz 2: dinamik parametre store'u (HyperOpt + IntelligenceHub yazar, engine okur)
//pub mod market_regime;      //dosya
pub mod signal_evaluator;   //dosya

// Tier 3: Otonom Kontrol ve Kurtarma (Control & Recovery)
pub mod error_recovery;
pub mod safety;
//pub mod autonomous_audit;   //dosya
//pub mod autonomous_control; //dosya
//pub mod autonomous_trader;  //dosya

// Tier 4: Optimizasyon ve İzleme (Optimization & Ops)
pub mod hot_reload;
pub mod backtester;
//pub mod hyperopt;
//pub mod automl;
//pub mod logger;
//pub mod file_logger;
//pub mod persistence;
//pub mod backtest_scheduler; //dosya
//pub mod config_helpers;     //dosya
//pub mod optimizer;          //dosya

// Tier 5: Entegrasyon ve Arayüz (Interface)
//pub mod symbol_orchestrator;
//pub mod telegram_notifier;
//pub mod streaming;
//pub mod monitor;
//pub mod test_orchestrator;
pub mod scalp_swing;
//pub mod api;        //dosya
//pub mod dashboard;      //dosya
pub mod integration_advanced;   //dosya
pub mod sr_detector;        //dosya
pub mod robotic_loop;       //dosya
pub mod user_profile;       //dosya

// Security Arayüzü
pub mod security;

// Strategies
pub mod strategies;
pub mod calculations;
