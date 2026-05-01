use crate::robot::data_fetcher::market_fetcher::MarketFetcher;
use crate::robot::data_fetcher::bist_fetcher::BistFetcher;
use std::fs::{OpenOptions, File};
use std::io::{BufRead, BufReader, Write};
/// Checkpoint dosya yolu (sabit, opsiyonel)
const AUTONOMOUS_PROGRESS_CSV: &str = "backup_autonomous_progress/autonomous_progress.csv";
/// Binance API'nin desteklediği interval listesi
const BINANCE_SUPPORTED_INTERVALS: &[&str] = &[
    "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h", "1d", "3d", "1w", "1M"
];
// robot/autonomous_trader.rs - Tam Otonom AI/ML Trading Sistemi
// Exchange, market, symbol, interval, strategy seçimlerini ML ile yapar
// Paper trading → Live trading otomatik geçiş mantığı

use crate::types::Candle;
use crate::robot::ml_engine::{FeatureExtractor, MLSignalPredictor};
use crate::database_reader::{list_available_tables, list_symbols, read_candles};
use crate::robot::data_fetcher::binance::BinanceFetcher;
use crate::database_writer::{open_connection, save_candles_bulk};
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};

/// Otonom trader konfigürasyonu
#[derive(Debug, Clone, Serialize, Deserialize)]
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
            evaluation_interval_secs: 300, // 5 dakika
            paper_trade_min_success_rate: 0.65, // %65 kazanma oranı
            paper_trade_min_trades: 20,
            live_trade_enabled: false, // Varsayılan: paper-only
        }
    }
}

/// Sembol performans verisi
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

/// Strateji performans verisi
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Paper → Live geçiş kararı
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraduationDecision {
    pub ready_for_live: bool,
    pub reasons: Vec<String>,
    pub symbol: String,
    pub strategy: String,
    pub performance: StrategyPerformance,
}

/// Otonom AI/ML Trading Sistemi
pub struct AutonomousTrader {
    pub config: AutonomousConfig,
    pub ml_predictor: MLSignalPredictor,
    pub feature_extractor: FeatureExtractor,
    pub symbol_performances: HashMap<String, SymbolPerformance>,
    pub strategy_performances: HashMap<String, StrategyPerformance>,
    pub last_evaluation: Option<DateTime<Utc>>,
    pub cycle_count: u64,
    pub audit_logger: Option<crate::robot::AutonomousAuditLogger>,
}

impl AutonomousTrader {
        /// Checkpoint dosyasından tamamlanan sembolleri oku
        fn load_completed_symbols() -> std::collections::HashSet<String> {
            let mut set = std::collections::HashSet::new();
            if let Ok(file) = File::open(AUTONOMOUS_PROGRESS_CSV) {
                let reader = BufReader::new(file);
                for line in reader.lines().flatten() {
                    let parts: Vec<_> = line.split(',').collect();
                    if parts.len() >= 4 {
                        // Key: exchange:market:symbol:interval
                        let key = format!("{}:{}:{}:{}", parts[0], parts[1], parts[2], parts[3]);
                        set.insert(key);
                    }
                }
            }
            set
        }

        /// Checkpoint dosyasına sembol tamamlandı olarak ekle
        fn save_completed_symbol(exchange: &str, market: &str, symbol: &str, interval: &str, count: usize) {
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(AUTONOMOUS_PROGRESS_CSV) {
                let ts = chrono::Utc::now().to_rfc3339();
                let _ = writeln!(file, "{},{},{},{},{},{}", exchange, market, symbol, interval, count, ts);
            }
        }
    /// Yeni otonom trader oluştur
    pub fn new(config: AutonomousConfig) -> Self {
        let audit_logger = Some(crate::robot::AutonomousAuditLogger::new("logs/autonomous"));
        
        Self {
            config,
            ml_predictor: MLSignalPredictor::new(0.6), // %60 confidence threshold
            feature_extractor: FeatureExtractor,
            symbol_performances: HashMap::new(),
            strategy_performances: HashMap::new(),
            last_evaluation: None,
            cycle_count: 0,
            audit_logger,
        }
    }

    /// ML ile en iyi sembolleri seç (volatilite, volume, trend analizi)
    /// En iyi sembolleri seçerken canlı veri çek ve DB'yi güncelle
    pub async fn select_best_symbols(&mut self) -> Result<Vec<SymbolPerformance>, String> {
        println!("🔍 AŞAMA 1: DB tabloları okunuyor...");
        let tables = list_available_tables(&self.config.db_path)
            .map_err(|e| format!("DB tables okunamadı: {}", e))?;

        if tables.is_empty() {
            return Err("❌ Hiç DB tablosu bulunamadı. Lütfen önce veri indirin.".to_string());
        }

        println!("🔍 AŞAMA 2: Semboller taranıyor ({} tablo bulundu)...", tables.len());
        let mut all_performances = Vec::new();

        // BinanceFetcher ve DB bağlantısı oluştur
        let binance_fetcher = BinanceFetcher;
        let bist_fetcher = BistFetcher;
        let conn = match open_connection(&self.config.db_path) {
            Ok(c) => c,
            Err(e) => return Err(format!("DB bağlantısı açılamadı: {}", e)),
        };

        // --- Checkpoint: tamamlanan sembolleri yükle ---
        let completed = Self::load_completed_symbols();

        for (exchange, market) in tables {
            let symbols = list_symbols(&self.config.db_path, &exchange, &market)
                .map_err(|e| format!("Semboller okunamadı ({}/{}): {}", exchange, market, e))?;

            println!("  📊 {}/{} - {} sembol bulundu", exchange, market, symbols.len());

            let is_binance = exchange.to_lowercase() == "binance" && (market.to_lowercase() == "spot" || market.to_lowercase() == "futures");
            let is_bist = exchange.to_lowercase() == "bist" && market.to_lowercase().starts_with("bist");

            for symbol_info in symbols {
                let checkpoint_key = format!("{}:{}:{}:{}", exchange, market, symbol_info.symbol, symbol_info.interval);
                if completed.contains(&checkpoint_key) {
                    println!("    ⏭️  {} ({}) daha önce tamamlanmış, atlanıyor.", symbol_info.symbol, symbol_info.interval);
                    continue;
                }

                // --- INTERVAL DOĞRULAMA ---
                if !BINANCE_SUPPORTED_INTERVALS.contains(&symbol_info.interval.as_str()) {
                    println!("    ⏭️  {} atlandı (desteklenmeyen interval: {})", symbol_info.symbol, symbol_info.interval);
                    continue;
                }


                // --- FETCHER SEÇİMİ ---
                let fetcher: Option<&dyn MarketFetcher> = if is_binance {
                    Some(&binance_fetcher)
                } else if is_bist {
                    Some(&bist_fetcher)
                } else {
                    None
                };
                if fetcher.is_none() {
                    println!("    ⏭️  {} atlandı (canlı veri çekme desteklenmiyor: {} / {})", symbol_info.symbol, exchange, market);
                    continue;
                }
                let fetcher = fetcher.unwrap();

                // --- EKSİK VERİYİ TAMAMLAMA ---
                let mut current_count = symbol_info.count;

                if current_count < self.config.min_candles {
                    let missing = self.config.min_candles - current_count;
                    println!("    ℹ️  {} için eksik veri tespit edildi ({} < {}), {} mum çekilecek...", symbol_info.symbol, current_count, self.config.min_candles, missing);
                    // Eksik kadar canlı veri çek
                    let live_candles = match fetcher.fetch_latest(
                        &symbol_info.symbol,
                        &symbol_info.interval,
                        missing
                    ).await {
                        Ok(c) => c,
                        Err(e) => {
                            println!("    ⚠️  Canlı veri çekilemedi: {}", e);
                            continue;
                        }
                    };
                    // DB'ye yaz
                    match save_candles_bulk(&conn, &exchange, &market, &live_candles) {
                        Ok((inserted, skipped)) => {
                            println!("    💾 DB güncellendi: {} yeni, {} atlandı (dupe)", inserted, skipped);
                        },
                        Err(e) => {
                            println!("    ⚠️  DB güncellenemedi: {}", e);
                            continue;
                        }
                    }
                    current_count += live_candles.len();
                }

                // --- YETERLİ VERİ VAR MI KONTROL ET ---
                if current_count < self.config.min_candles {
                    println!("    ⏭️  {} atlandı (veri tamamlanamadı: {} < {})", symbol_info.symbol, current_count, self.config.min_candles);
                    continue;
                }

                // --- CANLI VERİ GÜNCELLEME (tamamlayıcıdan bağımsız, en güncel mumlar için) ---

                println!("    🌐 {} için en güncel veri çekiliyor...", symbol_info.symbol);
                let live_candles = match fetcher.fetch_latest(
                    &symbol_info.symbol,
                    &symbol_info.interval,
                    self.config.min_candles
                ).await {
                    Ok(c) => c,
                    Err(e) => {
                        println!("    ⚠️  Canlı veri çekilemedi: {}", e);
                        continue;
                    }
                };
                // DB'ye yaz
                match save_candles_bulk(&conn, &exchange, &market, &live_candles) {
                    Ok((inserted, skipped)) => {
                        println!("    💾 DB güncellendi: {} yeni, {} atlandı (dupe)", inserted, skipped);
                    },
                    Err(e) => {
                        println!("    ⚠️  DB güncellenemedi: {}", e);
                        continue;
                    }
                }

                // --- ANALİZ İÇİN GÜNCEL CANDLE'LARI OKU ---
                let candles = match read_candles(
                    &self.config.db_path,
                    &exchange,
                    &market,
                    &symbol_info.symbol,
                    &symbol_info.interval,
                    Some(self.config.min_candles),
                ) {
                    Ok(c) => c,
                    Err(e) => {
                        println!("    ⚠️  Candle okunamadı (DB): {}", e);
                        continue;
                    }
                };

                if candles.is_empty() {
                    println!("    ⚠️  {} için candle verisi boş, atlanıyor", symbol_info.symbol);
                    continue;
                }

                // Volatilite skoru (fiyat değişim yüzdesi)
                println!("  ⚙️ AŞAMA 3: {} için skorlar hesaplanıyor...", symbol_info.symbol);
                let volatility = self.calculate_volatility(&candles);
                
                // Volume skoru (normalize edilmiş ortalama hacim)
                let volume_score = self.calculate_volume_score(&candles);
                
                // Trend gücü (momentum)
                let trend_strength = self.calculate_trend_strength(&candles);

                // ML sinyal güveni
                let ml_confidence = self.calculate_ml_confidence(&candles);

                // Genel skor (ağırlıklı ortalama)
                let overall_score = 
                    0.3 * volatility + 
                    0.2 * volume_score + 
                    0.3 * trend_strength + 
                    0.2 * ml_confidence;

                let perf = SymbolPerformance {
                    exchange: exchange.clone(),
                    market: market.clone(),
                    symbol: symbol_info.symbol.clone(),
                    interval: symbol_info.interval.clone(),
                    volatility_score: volatility,
                    volume_score,
                    trend_strength,
                    ml_signal_confidence: ml_confidence,
                    overall_score,
                };

                all_performances.push(perf.clone());
                self.symbol_performances.insert(
                    format!("{}:{}:{}", exchange, market, symbol_info.symbol),
                    perf
                );

                // --- Checkpoint: sembol tamamlandı olarak kaydet ---
                Self::save_completed_symbol(&exchange, &market, &symbol_info.symbol, &symbol_info.interval, current_count);
            }
        }

        if all_performances.is_empty() {
            return Err("❌ Analiz edilebilecek sembol bulunamadı. Veri yetersiz veya yok.".to_string());
        }

        println!("📊 AŞAMA 4: Sembol skorları sıralanıyor ({} sembol)...", all_performances.len());
        // En yüksek skorlara göre sırala
        all_performances.sort_by(|a, b| {
            b.overall_score.partial_cmp(&a.overall_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        
        let top_symbols: Vec<SymbolPerformance> = all_performances
            .into_iter()
            .take(self.config.top_symbols_count)
            .collect();
        if top_symbols.is_empty() {
            return Err("❌ Top sembol seçilemedi.".to_string());
        }


        println!("✅ ML Symbol Selection: {} sembol seçildi", top_symbols.len());
        for (i, sym) in top_symbols.iter().enumerate() {
            println!("  {}. {} - Skor: {:.4}", i+1, sym.symbol, sym.overall_score);
        }

        Ok(top_symbols)
    }

    /// ML ile en iyi stratejiyi seç (her sembol için)
    pub fn select_best_strategy(&self, _symbol: &str, candles: &[Candle]) -> String {
        // ML tahminlerini al
        if let Ok(prediction) = self.ml_predictor.predict(candles) {
            // Confidence'a göre strateji seçimi
            if prediction.confidence > 0.8 {
                if prediction.ml_score > 0.5 {
                    "ML_SIGNAL".to_string()
                } else {
                    "MACD".to_string()
                }
            } else if prediction.confidence > 0.6 {
                "MA_CROSSOVER".to_string()
            } else if prediction.confidence > 0.4 {
                "RSI".to_string()
            } else {
                "BOLLINGER_BANDS".to_string()
            }
        } else {
            "MA_CROSSOVER".to_string() // Fallback
        }
    }

    /// Paper trading performansını değerlendir ve live'a geçiş kararı ver
    pub fn evaluate_graduation(&self, symbol: &str, strategy: &str) -> GraduationDecision {
        println!("🧪 AŞAMA 5: {} için graduation kriterleri değerlendiriliyor...", symbol);
        let key = format!("{}:{}", symbol, strategy);
        
        if let Some(perf) = self.strategy_performances.get(&key) {
            let mut reasons = Vec::new();
            let mut ready = true;

            // Kriter 1: Minimum trade sayısı
            if perf.total_trades < self.config.paper_trade_min_trades {
                ready = false;
                reasons.push(format!(
                    "Yetersiz trade sayısı: {} < {}",
                    perf.total_trades,
                    self.config.paper_trade_min_trades
                ));
            } else {
                reasons.push(format!("✓ Trade sayısı yeterli: {}", perf.total_trades));
            }

            // Kriter 2: Kazanma oranı
            if perf.win_rate < self.config.paper_trade_min_success_rate {
                ready = false;
                reasons.push(format!(
                    "Düşük kazanma oranı: {:.2}% < {:.2}%",
                    perf.win_rate * 100.0,
                    self.config.paper_trade_min_success_rate * 100.0
                ));
            } else {
                reasons.push(format!("✓ Kazanma oranı yeterli: {:.2}%", perf.win_rate * 100.0));
            }

            // Kriter 3: Pozitif toplam PnL
            if perf.total_pnl <= 0.0 {
                ready = false;
                reasons.push(format!("Negatif toplam PnL: ${:.2}", perf.total_pnl));
            } else {
                reasons.push(format!("✓ Pozitif toplam PnL: ${:.2}", perf.total_pnl));
            }

            // Kriter 4: Sharpe ratio
            if perf.sharpe_ratio < 1.0 {
                ready = false;
                reasons.push(format!("Düşük Sharpe Ratio: {:.2} < 1.0", perf.sharpe_ratio));
            } else {
                reasons.push(format!("✓ İyi Sharpe Ratio: {:.2}", perf.sharpe_ratio));
            }

            // Kriter 5: Max drawdown
            if perf.max_drawdown > 15.0 {
                ready = false;
                reasons.push(format!("Yüksek Max Drawdown: {:.2}% > 15%", perf.max_drawdown));
            } else {
                reasons.push(format!("✓ Kontrollü Max Drawdown: {:.2}%", perf.max_drawdown));
            }

            // Live trade enabled kontrolü
            if !self.config.live_trade_enabled {
                ready = false;
                reasons.push("⚠️ Live trade sistem ayarlarında kapalı".to_string());
            }

            GraduationDecision {
                ready_for_live: ready,
                reasons,
                symbol: symbol.to_string(),
                strategy: strategy.to_string(),
                performance: perf.clone(),
            }
        } else {
            GraduationDecision {
                ready_for_live: false,
                reasons: vec!["Performans verisi bulunamadı".to_string()],
                symbol: symbol.to_string(),
                strategy: strategy.to_string(),
                performance: StrategyPerformance {
                    strategy_name: strategy.to_string(),
                    total_trades: 0,
                    winning_trades: 0,
                    win_rate: 0.0,
                    total_pnl: 0.0,
                    avg_pnl_per_trade: 0.0,
                    sharpe_ratio: 0.0,
                    max_drawdown: 0.0,
                },
            }
        }
    }

    /// Strateji performansını kaydet/güncelle
    pub fn update_strategy_performance(
        &mut self,
        symbol: &str,
        strategy: &str,
        performance: StrategyPerformance,
    ) {
        let key = format!("{}:{}", symbol, strategy);
        self.strategy_performances.insert(key, performance);
    }

    // === YARDIMCI FONKSİYONLAR ===

    fn calculate_volatility(&self, candles: &[Candle]) -> f64 {
        if candles.len() < 2 {
            return 0.0;
        }

        let returns: Vec<f64> = candles
            .windows(2)
            .map(|w| (w[1].close - w[0].close) / w[0].close)
            .collect();

        let mean = returns.iter().sum::<f64>() / returns.len() as f64;
        let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / returns.len() as f64;
        
        variance.sqrt() * 100.0 // Yüzde cinsinden
    }

    fn calculate_volume_score(&self, candles: &[Candle]) -> f64 {
        if candles.is_empty() {
            return 0.0;
        }

        let avg_volume = candles.iter().map(|c| c.volume).sum::<f64>() / candles.len() as f64;
        let max_volume = candles.iter().map(|c| c.volume).fold(0.0, f64::max);
        
        if max_volume > 0.0 {
            (avg_volume / max_volume).min(1.0)
        } else {
            0.0
        }
    }

    fn calculate_trend_strength(&self, candles: &[Candle]) -> f64 {
        if candles.len() < 20 {
            return 0.0;
        }

        // SMA20 ile trend yönü
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let sma20 = closes[closes.len() - 20..].iter().sum::<f64>() / 20.0;
        let current_price = candles.last()
            .map(|c| c.close)
            .unwrap_or(0.0);

        let trend_pct = ((current_price - sma20) / sma20).abs();
        trend_pct.min(1.0)
    }

    fn calculate_ml_confidence(&self, candles: &[Candle]) -> f64 {
        if let Ok(prediction) = self.ml_predictor.predict(candles) {
            prediction.confidence
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autonomous_trader_creation() {
        let config = AutonomousConfig::default();
        let trader = AutonomousTrader::new(config);
        assert_eq!(trader.symbol_performances.len(), 0);
    }

    #[test]
    fn test_graduation_decision() {
        let config = AutonomousConfig::default();
        let trader = AutonomousTrader::new(config);
        
        let decision = trader.evaluate_graduation("BTCUSDT", "MA_CROSSOVER");
        assert!(!decision.ready_for_live);
    }
}
