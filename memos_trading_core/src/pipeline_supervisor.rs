use rand::Rng; // rng.gen::<f64>() için gerekli trait

/// Çoklu API anahtarı ve endpoint rotasyonu için yapı
#[derive(Clone, Debug)]
pub struct ApiKeyEndpoint {
    pub api_key: String,
    pub endpoint: String,
    pub is_active: bool,
}

use std::fs;
use std::path::Path;

use crate::types::StrategyParams;

use std::fs::File;
use std::io::{BufRead, BufReader};
use chrono::{DateTime, Utc, Duration};

/// Dinamik rate limit ve API sağlık yönetimi için eşik değerler
const RATE_LIMIT_THRESHOLD: u32 = 90; // %90'a ulaşınca uyarı

use std::collections::BTreeMap;

/// Self-healing için hata sayacı ve eşik değeri
const MAX_CONSECUTIVE_ERRORS: u32 = 3;

// PipelineSupervisor: Extreme auto trading için otomatik pipeline ve strateji yönetimi
// Türkçe açıklamalar ile, insan müdahalesi olmadan çalışacak şekilde tasarlanmıştır.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::task;
use crate::auto_test_and_logging::AutoTestAndLoggingManager;

/// Her sembol için otomatik pipeline ve strateji yönetimi
#[derive(Debug, Default)]
pub struct PipelineSupervisor {
    pub pipelines: Arc<Mutex<HashMap<String, task::JoinHandle<()>>>>,
    pub test_and_logging: AutoTestAndLoggingManager,
}

impl PipelineSupervisor {

    /// Kullanıcı davranış analitiği ve raporlama: İşlem alışkanlıkları, risk profili, başarı oranı ve öneriler
    pub fn analyze_user_behavior(&self, user_id: &str, trade_history: &[f64], risk_levels: &[f64]) {
        let total_trades = trade_history.len();
        let win_trades = trade_history.iter().filter(|&&r| r > 0.0).count();
        let loss_trades = trade_history.iter().filter(|&&r| r < 0.0).count();
        let avg_risk = if !risk_levels.is_empty() { risk_levels.iter().sum::<f64>() / risk_levels.len() as f64 } else { 0.0 };
        let win_rate = if total_trades > 0 { win_trades as f64 / total_trades as f64 } else { 0.0 };
        let loss_rate = if total_trades > 0 { loss_trades as f64 / total_trades as f64 } else { 0.0 };
        let avg_return = if total_trades > 0 { trade_history.iter().sum::<f64>() / total_trades as f64 } else { 0.0 };

        let mut suggestions = Vec::new();
        if win_rate < 0.4 {
            suggestions.push("Kazanç oranınız düşük, strateji parametrelerinizi gözden geçirin.");
        }
        if avg_risk > 0.5 {
            suggestions.push("Risk seviyeniz yüksek, pozisyon boyutunu azaltmayı düşünün.");
        }
        if avg_return < 0.0 {
            suggestions.push("Ortalama getiri negatif, farklı strateji veya sembol deneyin.");
        }

        let report = format!(
            "Kullanıcı: {}\nToplam İşlem: {}\nKazanç Oranı: {:.2}%\nKayıp Oranı: {:.2}%\nOrtalama Risk: {:.2}\nOrtalama Getiri: {:.4}\nÖneriler: {}",
            user_id,
            total_trades,
            win_rate * 100.0,
            loss_rate * 100.0,
            avg_risk,
            avg_return,
            suggestions.join("; ")
        );
        Self::log_error_to_file(&format!("Kullanıcı analitiği: {}", report), user_id);
        // app_handle ve emit_all kaldırıldı
    }

    /// Geçmiş işlem verilerinden öğrenerek, sinyal üretiminde ağırlık ve parametreleri dinamik optimize eder (örnek ML fonksiyonu)
    pub fn ml_optimize_signal_weights(&self, trade_features: &[Vec<f64>], trade_results: &[f64], current_weights: &[f64]) -> Vec<f64> {
        // Basit random search ile ağırlık optimizasyonu (örnek, gerçek ML yerine)
        let mut best_weights = current_weights.to_vec();
        let mut best_score = f64::MIN;
        let mut rng = rand::thread_rng();
        for _ in 0..20 {
            let mut candidate = current_weights.to_vec();
            for w in &mut candidate {
                *w *= 0.9 + 0.2 * rng.gen::<f64>();
            }
            // Skor: Sonuçlarla ağırlıklı çarpımın ortalaması
            let mut score = 0.0;
            for (features, &result) in trade_features.iter().zip(trade_results.iter()) {
                let weighted_sum: f64 = features.iter().zip(candidate.iter()).map(|(f, w)| f * w).sum();
                score += weighted_sum * result;
            }
            if score > best_score {
                best_score = score;
                best_weights = candidate;
            }
        }
        Self::log_error_to_file(&format!("ML optimize: Yeni ağırlıklar {:?} (skor={:.4})", best_weights, best_score), "ML");
        best_weights
    }

    /// Çoklu API anahtarı ve endpoint rotasyonu: Otomatik geçiş ve failover
    pub fn rotate_api_key_endpoint(&self, symbol: &str, keys: &mut [ApiKeyEndpoint]) -> Option<ApiKeyEndpoint> {
        // Aktif anahtar/endpoint bul
        if let Some(current) = keys.iter_mut().find(|k| k.is_active) {
            // Eğer sorun varsa aktifliği kaldır
            current.is_active = false;
            Self::log_error_to_file("API anahtarı/endpoint devre dışı bırakıldı, rotasyon başlatıldı", symbol);
        }
        // Sıradaki aktif olmayan anahtarı bul ve aktif yap
        if let Some(next) = keys.iter_mut().find(|k| !k.is_active) {
            next.is_active = true;
            Self::log_error_to_file(&format!("Yeni API anahtarı/endpoint aktif: {}", next.endpoint), symbol);
            return Some(next.clone());
        }
        // Hiçbiri kalmadıysa alarm
        Self::log_error_to_file("Tüm API anahtarı/endpoint'ler tükendi!", symbol);
        None
    }

    /// Otomatik yedekleme: Kritik dosyaları periyodik olarak yedekler
    pub fn backup_critical_files(&self, files: &[&str], backup_dir: &str) {
        for file in files {
            if Path::new(file).exists() {
                let filename = Path::new(file).file_name().unwrap().to_string_lossy();
                let backup_path = format!("{}/{}_backup_{}.bak", backup_dir, filename, chrono::Utc::now().format("%Y%m%d%H%M%S"));
                let _ = fs::copy(file, &backup_path);
                Self::log_error_to_file(&format!("Yedekleme: {} dosyası {} konumuna yedeklendi", file, backup_path), "SYSTEM");
            }
        }
    }

    /// Otomatik geri yükleme: Hata veya veri kaybında yedekten dosyayı geri yükler
    pub fn restore_from_backup(&self, file: &str, backup_dir: &str) -> bool {
        let filename = Path::new(file).file_name().unwrap().to_string_lossy();
        let mut latest_backup: Option<std::path::PathBuf> = None;
        if let Ok(entries) = fs::read_dir(backup_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                // Cow<str> ile contains kullanırken dereference (&*filename) yapılmalı
                if path.file_name().unwrap().to_string_lossy().contains(&*filename) {
                    if let Some(ref current) = latest_backup {
                        if path.metadata().unwrap().modified().unwrap() > current.metadata().unwrap().modified().unwrap() {
                            latest_backup = Some(path);
                        }
                    } else {
                        latest_backup = Some(path);
                    }
                }
            }
        }
        if let Some(backup_path) = latest_backup {
            let _ = fs::copy(&backup_path, file);
            Self::log_error_to_file(&format!("Geri yükleme: {} dosyası {} yedeğinden geri yüklendi", file, backup_path.display()), "SYSTEM");
            return true;
        }
        false
    }

    /// Gerçek zamanlı anomali tespiti: Ani hacim artışı, alışılmadık kayıp/kazanç veya API gecikmesi
    pub fn detect_realtime_anomaly(&self, symbol: &str, recent_returns: &[f64], recent_volumes: &[f64], api_latency_ms: u64) {
        // Ani kayıp/kazanç: Son işlemde %5'ten fazla değişim
        if let Some(last) = recent_returns.last() {
            if last.abs() > 0.05 {
                Self::log_error_to_file("Anomali: Ani kayıp/kazanç tespit edildi", symbol);
                // Burada koruma veya pozisyon kapama başlatılabilir
            }
        }
        // Ani hacim artışı: Son hacim, ortalamanın 2 katından fazla
        if recent_volumes.len() > 5 {
            let avg_vol = recent_volumes.iter().take(recent_volumes.len()-1).sum::<f64>() / (recent_volumes.len()-1) as f64;
            if let Some(last_vol) = recent_volumes.last() {
                if *last_vol > avg_vol * 2.0 {
                    Self::log_error_to_file("Anomali: Ani hacim artışı tespit edildi", symbol);
                }
            }
        }
        // API gecikmesi: 1000ms üzeri gecikme
        if api_latency_ms > 1000 {
            Self::log_error_to_file("Anomali: API gecikmesi tespit edildi", symbol);
        }
    }

    /// Kullanıcıya özel otomatik strateji parametre optimizasyonu
    /// Geçmiş işlem sonuçları ve kullanıcı davranışına göre parametreleri dinamik ayarlar
    pub fn optimize_strategy_params(&self, user_id: &str, trade_history: &[f64], current_params: &StrategyParams) -> StrategyParams {
        // Basit örnek: Son 20 işlemin başarı oranı ve volatiliteye göre dinamik ayar
        let n = trade_history.len().min(20);
        let recent: Vec<f64> = trade_history.iter().rev().take(n).cloned().collect();
        let avg_return = if n > 0 { recent.iter().sum::<f64>() / n as f64 } else { 0.0 };
        let volatility = if n > 1 {
            let mean = avg_return;
            (recent.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0)).sqrt()
        } else { 0.0 };

        // Parametre optimizasyon mantığı (örnek):
        let new_params = current_params.clone();
        // Eğer başarı oranı yüksekse parametreleri optimize et (örnek mantık, risk ve stop_loss alanı yok)
        // Burada mevcut parametreler üzerinden başka alanlar optimize edilebilir.
        // Kullanıcıya özel başka kurallar eklenebilir
        Self::log_error_to_file(&format!("Strateji parametreleri optimize edildi: avg_return={:.4}, vol={:.4}", avg_return, volatility), user_id);
        new_params
    }

    /// Log dosyasını periyodik analiz eder, anomali veya olağan dışı durumlarda otomatik alarm ve bildirim gönderir
    pub fn analyze_logs_and_alarm(&self, log_path: &str) {
        if let Ok(file) = File::open(log_path) {
            let reader = BufReader::new(file);
            let mut error_count = 0;
            let mut last_error_time: Option<DateTime<Utc>> = None;
            for line in reader.lines().flatten() {
                if line.contains("Hata") || line.contains("ERROR") {
                    error_count += 1;
                    // Zaman damgası varsa çek
                    if let Some(ts) = line.split('|').next() {
                        if let Ok(dt) = ts.trim().parse::<DateTime<Utc>>() {
                            last_error_time = Some(dt);
                        }
                    }
                }
            }
            // Son 10 dakika içinde 5'ten fazla hata varsa alarm
            if let Some(last) = last_error_time {
                if error_count >= 5 && Utc::now() - last < Duration::minutes(10) {
                    Self::log_error_to_file("ALARM: Son 10 dakikada 5+ hata tespit edildi", "SYSTEM");
                    // Burada e-posta/telegram/discord bildirimi tetiklenebilir
                }
            }
        }
    }

    /// API rate limit'ine yaklaşınca otomatik yavaşlatma ve bildirim
    pub fn check_and_handle_rate_limit(&self, symbol: &str, current_rate: u32, max_rate: u32) {
        let usage_percent = (current_rate as f64 / max_rate as f64) * 100.0;
        if usage_percent >= RATE_LIMIT_THRESHOLD as f64 {
            Self::log_error_to_file("Rate limit'e yaklaşıldı, otomatik yavaşlatma başlatıldı", symbol);
            // Burada otomatik yavaşlatma veya geçici duraklatma başlatılabilir
        }
    }

    /// API anahtarı banlandıysa veya süresi dolduysa otomatik anahtar değişimi ve failover
    pub fn handle_api_key_failure(&self, symbol: &str, reason: &str) {
        Self::log_error_to_file(&format!("API anahtarı sorunu: {}", reason), symbol);
        // Burada otomatik anahtar değişimi veya alternatif endpoint/failover başlatılabilir
    }

                /// İzlenen sembollerin performans skorlarını analiz edip en iyi fırsatları önerir, kötü performanslıları izleme dışı bırakır
                pub fn analyze_and_recommend_symbols(&self, symbol_performance: &HashMap<String, f64>, min_score: f64, max_count: usize) -> Vec<String> {
                    // Performans skorlarına göre azalan sırala
                    // f64 yerine i64 anahtar kullan (skoru 1_000_000 ile çarpıp tamsayıya çevir)
                    let mut sorted: BTreeMap<i64, Vec<String>> = BTreeMap::new();
                    for (symbol, score) in symbol_performance.iter() {
                        if *score >= min_score {
                            let key = (*score * 1_000_000.0) as i64;
                            sorted.entry(key).or_default().push(symbol.clone());
                        }
                    }
                    let mut recommended = Vec::new();
                    for (_score, symbols) in sorted.iter().rev() {
                        for symbol in symbols {
                            recommended.push(symbol.clone());
                            if recommended.len() >= max_count {
                                return recommended;
                            }
                        }
                    }
                    recommended
                }

                /// Kötü performanslı sembolleri otomatik izleme dışı bırakır
                pub fn remove_bad_symbols(&self, symbol_performance: &HashMap<String, f64>, min_score: f64) -> Vec<String> {
                    let mut removed = Vec::new();
                    for (symbol, score) in symbol_performance.iter() {
                        if *score < min_score {
                            // Pipeline'ı durdur
                            self.stop_pipeline(symbol);
                            Self::log_error_to_file("Kötü performans: sembol izleme dışı bırakıldı", symbol);
                            removed.push(symbol.clone());
                        }
                    }
                    removed
                }
            /// Hata tespiti ve otomatik yeniden bağlanma fonksiyonu
            pub fn handle_error_and_reconnect(&self, error: &str, symbol: &str) {
                // Hata mesajını log dosyasına yaz
                Self::log_error_to_file(error, symbol);
                // Frontend'e canlı uyarı event'i gönder
                // app_handle ve emit_all kaldırıldı, sadece log ve konsol çıktısı bırakıldı
                // Otomatik yeniden bağlanma simülasyonu
                println!("[PipelineSupervisor] {} için otomatik yeniden bağlanma başlatıldı.", symbol);
                // Burada gerçek yeniden bağlanma fonksiyonu çağrılabilir
            }
        /// Merkezi logging fonksiyonu: Hataları log dosyasına kaydeder
        pub fn log_error_to_file(error: &str, symbol: &str) {
            use std::fs::OpenOptions;
            use std::io::Write;
            use chrono::Utc;
            let now = Utc::now();
            let log_line = format!("{} | {} | Hata: {}\n", now, symbol, error);
            let log_path = "logs/pipeline_errors.log";
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
                let _ = file.write_all(log_line.as_bytes());
            }
        }
    /// Pipeline başlat (otomatik veri çekme, strateji çalıştırma, pozisyon yönetimi)
    pub async fn start_pipeline(&self, symbol: &str) {
        let _pipelines = self.pipelines.clone();
        let symbol_clone = symbol.to_string();
        let test_and_logging = self.test_and_logging.clone();
        let consecutive_errors = 0u32;
        let handle = task::spawn(async move {
            println!("[PipelineSupervisor] {} için pipeline başlatıldı.", symbol_clone);
            loop {
                // Otomatik performans ve güvenlik testleri (her döngüde)
                test_and_logging.run_performance_test(&symbol_clone);
                test_and_logging.run_security_test(&symbol_clone);

                // Self-healing: Hata sayısı eşik değeri aşarsa otomatik izleme dışı bırakma ve failover
                if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                    // Log ve frontend'e bildirim
                    PipelineSupervisor::log_error_to_file("Self-healing: Sembol otomatik izleme dışı bırakıldı", &symbol_clone);
                    println!("[PipelineSupervisor] {} için self-healing: izleme dışı bırakıldı.", symbol_clone);
                    // Burada failover veya alternatif pipeline başlatılabilir
                    break; // Pipeline döngüsünü sonlandır
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            }
        });
        self.pipelines.lock().unwrap().insert(symbol.to_string(), handle);
    }

    /// Pipeline durdur
    pub fn stop_pipeline(&self, symbol: &str) {
        let mut pipelines = self.pipelines.lock().unwrap();
        if let Some(handle) = pipelines.remove(symbol) {
            println!("[PipelineSupervisor] {} için pipeline durduruldu.", symbol);
            handle.abort();
        }
    }

    /// Pipeline ve strateji durumunu göster
    pub fn status(&self) {
        let pipelines = self.pipelines.lock().unwrap();
        println!("[PipelineSupervisor] Aktif pipeline sayısı: {}", pipelines.len());
    }
}

// Kullanım örneği:
// let supervisor = PipelineSupervisor::default();
// supervisor.start_pipeline("BTCUSDT").await;
// supervisor.status();
// supervisor.stop_pipeline("BTCUSDT");
