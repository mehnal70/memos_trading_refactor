// src/robot/logic/autonomous_trader.rs - Tam Otonom AI/ML Trading Sistemi
// Exchange, market, symbol, interval, strategy seçimlerini dinamik ve otonom gerçekleştirir.
// Paper trading -> Live trading otomatik geçiş mekanizması.
use std::fs;
use std::path::Path;
use crate::prelude::*; // Evrensel prelude odası (AppState, Candle, RoboticLoopConfig otomatik geldi)
use crate::persistence::reader::{list_available_tables, list_symbols, read_candles};
use crate::persistence::writer::save_candle; // K4/K5 uyumlu bulk veya tekil kayıt köprüsü

use std::fs::{OpenOptions, File};
use std::io::{BufRead, BufReader, Write};
use std::collections::{HashMap, HashSet};
use chrono::{DateTime, Utc};

// --- GLOBAL SERSERİ VE KISITLAMALAR ---
const AUTONOMOUS_PROGRESS_CSV: &str = "backup_autonomous_progress/autonomous_progress.csv";
const BINANCE_SUPPORTED_INTERVALS: &[&str] = &[
    "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h", "1d", "3d", "1w", "1M"
];

// Mock veya gerçek dış bağlantı traiti (Projenizin alt modüllerine göredir)
pub trait MarketFetcher {
    fn fetch_latest<'a>(&'a self, symbol: &'a str, interval: &'a str, limit: usize) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Candle>, String>> + Send + 'a>>;
}
pub struct BinanceFetcher;
impl MarketFetcher for BinanceFetcher {
    fn fetch_latest<'a>(&'a self, _symbol: &'a str, _interval: &'a str, _limit: usize) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Candle>, String>> + Send + 'a>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}
pub struct BistFetcher;
impl MarketFetcher for BistFetcher {
    fn fetch_latest<'a>(&'a self, _symbol: &'a str, _interval: &'a str, _limit: usize) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Candle>, String>> + Send + 'a>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

// Dummy veya mock modeller (ML entegrasyonlarınızın yoluna göre)
pub struct MLSignalPredictor;
pub struct PredictionResult { pub confidence: f64, pub ml_score: f64 }
impl MLSignalPredictor {
    pub fn new(_t: f64) -> Self { Self }
    pub fn predict(&self, _c: &[Candle]) -> Result<PredictionResult, String> {
        Ok(PredictionResult { confidence: 0.75, ml_score: 0.6 })
    }
}
pub struct FeatureExtractor;

// =============================================================================
// 1. OTONOM VERİ MODELLERİ VE STRUKTLAR
// =============================================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AutonomousConfig {
    pub db_path: String,
    pub min_candles: usize,
    pub top_symbols_count: usize,
    pub evaluation_interval_secs: u64,
    pub paper_trade_min_success_rate: f64,
    pub paper_trade_min_trades: u32,
    pub live_trade_enabled: bool,
}

impl Default for AutonomousConfig {
    fn default() -> Self {
        Self {
            db_path: "data/trader.db".to_string(),
            min_candles: 200,
            top_symbols_count: 5,
            evaluation_interval_secs: 300,
            paper_trade_min_success_rate: 0.65,
            paper_trade_min_trades: 20,
            live_trade_enabled: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SymbolPerformance {
    pub exchange: String,
    pub market: String,
    pub symbol: String,
    pub interval: String,
    pub volatility_score: f64,
    pub volume_score: f64,
    pub trend_strength: f64,
    pub ml_signal_confidence: f64,
    pub overall_score: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StrategyPerformance {
    pub strategy_name: String,
    pub total_trades: u32,
    pub winning_trades: u32,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub avg_pnl_per_trade: f64,
    pub sharpe_ratio: f64,
    pub max_drawdown: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GraduationDecision {
    pub ready_for_live: bool,
    pub reasons: Vec<String>,
    pub symbol: String,
    pub strategy: String,
    pub performance: StrategyPerformance,
}

// =============================================================================
// 2. ANA OTONOM İŞLEYİCİ MOTOR (AUTONOMOUS TRADER)
// =============================================================================

pub struct AutonomousTrader {
    pub config: AutonomousConfig,
    pub ml_predictor: MLSignalPredictor,
    pub feature_extractor: FeatureExtractor,
    pub symbol_performances: HashMap<String, SymbolPerformance>,
    pub strategy_performances: HashMap<String, StrategyPerformance>,
    pub last_evaluation: Option<DateTime<Utc>>,
    pub cycle_count: u64,
}

impl AutonomousTrader {
    pub fn new(config: AutonomousConfig) -> Self {
        Self {
            config,
            ml_predictor: MLSignalPredictor::new(0.6),
            feature_extractor: FeatureExtractor,
            symbol_performances: HashMap::new(),
            strategy_performances: HashMap::new(),
            last_evaluation: None,
            cycle_count: 0,
        }
    }

    fn load_completed_symbols() -> HashSet<String> {
        let mut set = HashSet::new();
        if let Ok(file) = File::open(AUTONOMOUS_PROGRESS_CSV) {
            let reader = BufReader::new(file);
            for line in reader.lines().flatten() {
                let parts: Vec<_> = line.split(',').collect();
                if parts.len() >= 4 {
                    let key = format!("{}:{}:{}:{}", parts[0], parts[1], parts[2], parts[3]);
                    set.insert(key);
                }
            }
        }
        set
    }

    fn save_completed_symbol(exchange: &str, market: &str, symbol: &str, interval: &str, count: usize) {
        if let Some(parent) = Path::new(AUTONOMOUS_PROGRESS_CSV).parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(AUTONOMOUS_PROGRESS_CSV) {
            let ts = Utc::now().to_rfc3339();
            let _ = writeln!(file, "{},{},{},{},{},{}", exchange, market, symbol, interval, count, ts);
        }
    }

    /// 🧠 ML TABANLI OTONOM SEÇİM DÖNGÜSÜ
    pub async fn select_best_symbols(&mut self) -> Result<Vec<SymbolPerformance>, crate::MemosTradingError> {
        println!("🔍 AŞAMA 1: DB tabloları okunuyor...");
        let tables = list_available_tables(&self.config.db_path)?;

        if tables.is_empty() {
            return Err(crate::MemosTradingError::Database("❌ Hiç DB tablosu bulunamadı.".into()));
        }

        println!("🔍 AŞAMA 2: Semboller taranıyor ({} tablo bulundu)...", tables.len());
        let mut all_performances = Vec::new();

        let binance_fetcher = BinanceFetcher;
        let bist_fetcher = BistFetcher;
        let conn = crate::persistence::open_db(&self.config.db_path)?;

        let completed = Self::load_completed_symbols();

        // Örnek tablo süzgeci (Kendi mimarinizdeki dinamik parsellemeye göre)
        for tbl in tables {
            let parts: Vec<&str> = tbl.split('_').collect();
            if parts.len() < 3 { continue; }
            let exchange = "binance".to_string(); // Varsayılan veya parselleme sonucun
            let market = "futures".to_string();

            let symbols = list_symbols(&self.config.db_path)?;

            let is_binance = exchange.to_lowercase() == "binance";
            let is_bist = exchange.to_lowercase() == "bist";

            for sym in symbols {
                let interval = "1m".to_string(); // Şemadan gelen veya varsayılan değer
                let checkpoint_key = format!("{}:{}:{}:{}", exchange, market, sym, interval);
                
                if completed.contains(&checkpoint_key) {
                    println!("    ⏭️  {} ({}) daha önce tamamlanmış, atlanıyor.", sym, interval);
                    continue;
                }

                if !BINANCE_SUPPORTED_INTERVALS.contains(&interval.as_str()) {
                    continue;
                }

                let fetcher: Option<&dyn MarketFetcher> = if is_binance {
                    Some(&binance_fetcher)
                } else if is_bist {
                    Some(&bist_fetcher)
                } else {
                    None
                };

                let fetcher = match fetcher {
                    Some(f) => f,
                    None => continue,
                };

                // --- CANLI MUM TAZELEME VE HASAT ---
                let live_candles = match fetcher.fetch_latest(&sym, &interval, self.config.min_candles).await {
                    Ok(c) => c,
                    Err(_) => { continue; }
                };

                for candle in &live_candles {
                    let _ = save_candle(&conn, &exchange, &market, candle);
                }

                let candles = match read_candles(&self.config.db_path, &sym, &interval, self.config.min_candles) {
                    Ok(c) => c,
                    Err(_) => { continue; }
                };

                if candles.is_empty() { continue; }

                println!("  ⚙️ AŞAMA 3: {} için skorlar hesaplanıyor...", sym);
                let volatility = self.calculate_volatility(&candles);
                let volume_score = self.calculate_volume_score(&candles);
                let trend_strength = self.calculate_trend_strength(&candles);
                let ml_confidence = self.calculate_ml_confidence(&candles);

                let overall_score = 0.3 * volatility + 0.2 * volume_score + 0.3 * trend_strength + 0.2 * ml_confidence;

                let perf = SymbolPerformance {
                    exchange: exchange.clone(),
                    market: market.clone(),
                    symbol: sym.clone(),
                    interval: interval.clone(),
                    volatility_score: volatility,
                    volume_score,
                    trend_strength,
                    ml_signal_confidence: ml_confidence,
                    overall_score,
                };

                all_performances.push(perf.clone());
                self.symbol_performances.insert(format!("{}:{}:{}", exchange, market, sym), perf);
                Self::save_completed_symbol(&exchange, &market, &sym, &interval, candles.len());
            }
        }

        if all_performances.is_empty() {
            return Err(crate::MemosTradingError::Database("❌ Analiz edilebilecek sembol bulunamadı.".into()));
        }

        all_performances.sort_by(|a, b| b.overall_score.partial_cmp(&a.overall_score).unwrap_or(std::cmp::Ordering::Equal));
        let top_symbols: Vec<SymbolPerformance> = all_performances.into_iter().take(self.config.top_symbols_count).collect();

        Ok(top_symbols)
    }

    /// ML İLE EN İYİ STRATEJİ SEÇİCİ (FALLBACK KORUMALI)
    pub fn select_best_strategy(&self, _symbol: &str, candles: &[Candle]) -> String {
        if let Ok(prediction) = self.ml_predictor.predict(candles) {
            if prediction.confidence > 0.8 {
                if prediction.ml_score > 0.5 { "ML_SIGNAL".to_string() } else { "MACD".to_string() }
            } else if prediction.confidence > 0.6 {
                "MA_CROSSOVER".to_string()
            } else if prediction.confidence > 0.4 {
                "RSI".to_string()
            } else {
                "BOLLINGER_BANDS".to_string()
            }
        } else {
            "MA_CROSSOVER".to_string()
        }
    }

    /// PAPER → LIVE MEZUNİYET RADARI
    pub fn evaluate_graduation(&self, symbol: &str, strategy: &str) -> GraduationDecision {
        let key = format!("{}:{}", symbol, strategy);
        
        if let Some(perf) = self.strategy_performances.get(&key) {
            let mut reasons = Vec::new();
            let mut ready = true;

            if perf.total_trades < self.config.paper_trade_min_trades {
                ready = false;
                reasons.push(format!("Yetersiz trade sayısı: {} < {}", perf.total_trades, self.config.paper_trade_min_trades));
            } else {
                reasons.push(format!("✓ Trade sayısı yeterli: {}", perf.total_trades));
            }

            if perf.win_rate < self.config.paper_trade_min_success_rate {
                ready = false;
                reasons.push(format!("Düşük kazanma oranı: {:.2}%", perf.win_rate * 100.0));
            } else {
                reasons.push(format!("✓ Kazanma oranı yeterli: {:.2}%", perf.win_rate * 100.0));
            }

            if perf.total_pnl <= 0.0 {
                ready = false;
                reasons.push(format!("Negatif toplam PnL: ${:.2}", perf.total_pnl));
            } else {
                reasons.push(format!("✓ Pozitif toplam PnL: ${:.2}", perf.total_pnl));
            }

            if perf.sharpe_ratio < 1.0 {
                ready = false;
                reasons.push(format!("Düşük Sharpe Ratio: {:.2}", perf.sharpe_ratio));
            } else {
                reasons.push(format!("✓ İyi Sharpe Ratio: {:.2}", perf.sharpe_ratio));
            }

            if perf.max_drawdown > 15.0 {
                ready = false;
                reasons.push(format!("Yüksek Max Drawdown: {:.2}%", perf.max_drawdown));
            } else {
                reasons.push(format!("✓ Kontrollü Max Drawdown: {:.2}%", perf.max_drawdown));
            }

            if !self.config.live_trade_enabled {
                ready = false;
                reasons.push("⚠️ Live trade sistem ayarlarında kapalı".to_string());
            }

            GraduationDecision { ready_for_live: ready, reasons, symbol: symbol.to_string(), strategy: strategy.to_string(), performance: perf.clone() }
        } else {
            GraduationDecision {
                ready_for_live: false,
                reasons: vec!["Performans verisi bulunamadı".to_string()],
                symbol: symbol.to_string(),
                strategy: strategy.to_string(),
                performance: StrategyPerformance { strategy_name: strategy.to_string(), total_trades: 0, winning_trades: 0, win_rate: 0.0, total_pnl: 0.0, avg_pnl_per_trade: 0.0, sharpe_ratio: 0.0, max_drawdown: 0.0 },
            }
        }
    }

    pub fn update_strategy_performance(&mut self, symbol: &str, strategy: &str, performance: StrategyPerformance) {
        let key = format!("{}:{}", symbol, strategy);
        self.strategy_performances.insert(key, performance);
    }

    // =============================================================================
    // 3. MATEMATİKSEL SKORLAMA VE VOLATİLİTE SÜZGEÇLERİ
    // =============================================================================

    fn calculate_volatility(&self, candles: &[Candle]) -> f64 {
        if candles.len() < 2 { return 0.0; }
        let returns: Vec<f64> = candles.windows(2).map(|w| (w[1].close - w[0].close) / w[0].close).collect();
        let mean = returns.iter().sum::<f64>() / returns.len() as f64;
        let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / returns.len() as f64;
        variance.sqrt() * 100.0
    }

    fn calculate_volume_score(&self, candles: &[Candle]) -> f64 {
        if candles.is_empty() { return 0.0; }
        let avg_volume = candles.iter().map(|c| c.volume).sum::<f64>() / candles.len() as f64;
        let max_volume = candles.iter().map(|c| c.volume).fold(0.0, f64::max);
        if max_volume > 0.0 { (avg_volume / max_volume).min(1.0) } else { 0.0 }
    }

    fn calculate_trend_strength(&self, candles: &[Candle]) -> f64 {
        if candles.len() < 20 { return 0.0; }
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let sma20 = closes[closes.len() - 20..].iter().sum::<f64>() / 20.0;
        let current_price = candles.last().map(|c| c.close).unwrap_or(0.0);
        ((current_price - sma20) / sma20).abs().min(1.0)
    }

    fn calculate_ml_confidence(&self, candles: &[Candle]) -> f64 {
        if let Ok(prediction) = self.ml_predictor.predict(candles) { prediction.confidence } else { 0.0 }
    }
}
