// pipeline_supervisor.rs
// Otonom Pipeline ve Strateji Yönetim Modülü

use rand::Rng;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::io::{BufRead, BufReader, Write, BufWriter};
use std::collections::{HashMap, BTreeMap};
use std::sync::{Arc, Mutex};
use tokio::task;
use chrono::{DateTime, Utc, Duration};
use crate::robot::infra::monitoring::auto_test_and_logging::AutoTestAndLoggingManager;
use crate::core::types::StrategyParams;

#[derive(Clone, Debug)]
pub struct ApiKeyEndpoint {
    pub api_key: String,
    pub endpoint: String,
    pub is_active: bool,
}

#[derive(Debug, Default)]
pub struct PipelineSupervisor {
    pub pipelines: Arc<Mutex<HashMap<String, task::JoinHandle<()>>>>,
    /// Disk I/O içeren test+log yöneticisi; lazımsa create_or_default() ile init edilir.
    pub test_and_logging: Option<AutoTestAndLoggingManager>,
}

impl PipelineSupervisor {
    /// Sembol için pipeline başlatır — şimdilik kayıt tutmak için no-op JoinHandle.
    /// Gerçek pipeline tetikleyici task spawn'ı entegre edildiğinde burada wire'lanır.
    pub async fn start_pipeline(&self, symbol: &str) {
        let mut pipes = self.pipelines.lock().unwrap();
        if pipes.contains_key(symbol) { return; }
        let sym = symbol.to_string();
        let handle = task::spawn(async move {
            // Placeholder: gerçek pipeline döngüsü burada koşacak.
            let _ = sym;
        });
        pipes.insert(symbol.to_string(), handle);
    }

    /// Sembol pipeline'ını durdurur (handle abort + map'ten çıkar).
    pub fn stop_pipeline(&self, symbol: &str) {
        if let Ok(mut pipes) = self.pipelines.lock() {
            if let Some(handle) = pipes.remove(symbol) {
                handle.abort();
            }
        }
    }

    /// Aktif pipeline sayısını stdout'a yazar (basit durum raporu).
    pub fn status(&self) {
        if let Ok(pipes) = self.pipelines.lock() {
            println!("[PipelineSupervisor] aktif pipeline sayısı: {}", pipes.len());
        }
    }

    /// Kullanıcı davranış analitiği: İteratörlerle optimize edildi (Sıfır kopyalama)
    pub fn analyze_user_behavior(&self, user_id: &str, trade_history: &[f64], risk_levels: &[f64]) {
        let total_trades = trade_history.len();
        if total_trades == 0 { return; }

        let win_trades = trade_history.iter().filter(|&&r| r > 0.0).count();
        let loss_trades = total_trades - win_trades;
        
        let avg_risk = if !risk_levels.is_empty() { 
            risk_levels.iter().sum::<f64>() / risk_levels.len() as f64 
        } else { 0.0 };

        let win_rate = win_trades as f64 / total_trades as f64;
        let avg_return = trade_history.iter().sum::<f64>() / total_trades as f64;

        let mut suggestions = Vec::with_capacity(3);
        if win_rate < 0.4 { suggestions.push("Düşük kazanç oranı."); }
        if avg_risk > 0.5 { suggestions.push("Yüksek risk seviyesi."); }
        if avg_return < 0.0 { suggestions.push("Negatif ortalama getiri."); }

        let report = format!(
            "UID: {} | Win: {:.2}% | Loss: {:.2}% | Risk: {:.2} | Ret: {:.4} | Öneriler: {}",
            user_id, win_rate * 100.0, (loss_trades as f64 / total_trades as f64) * 100.0,
            avg_risk, avg_return, suggestions.join(" ")
        );
        
        Self::log_error_to_file(&format!("Kullanıcı analitiği: {}", report), user_id);
    }

    /// ML Sinyal Ağırlık Optimizasyonu: Random Search (Functional style)
    pub fn ml_optimize_signal_weights(&self, trade_features: &[Vec<f64>], trade_results: &[f64], current_weights: &[f64]) -> Vec<f64> {
        let mut best_weights = current_weights.to_vec();
        let mut best_score = f64::MIN;
        let mut rng = rand::thread_rng();

        for _ in 0..20 {
            let candidate: Vec<f64> = current_weights.iter()
                .map(|&w| w * (0.9 + 0.2 * rng.gen::<f64>()))
                .collect();

            let score: f64 = trade_features.iter()
                .zip(trade_results)
                .map(|(features, &result)| {
                    features.iter().zip(&candidate).map(|(f, w)| f * w).sum::<f64>() * result
                })
                .sum();

            if score > best_score {
                best_score = score;
                best_weights = candidate;
            }
        }
        Self::log_error_to_file(&format!("ML optimize: Skor={:.4}", best_score), "ML");
        best_weights
    }

    /// API ve Endpoint Rotasyonu: Failover mekanizması
    pub fn rotate_api_key_endpoint(&self, symbol: &str, keys: &mut [ApiKeyEndpoint]) -> Option<ApiKeyEndpoint> {
        if let Some(current) = keys.iter_mut().find(|k| k.is_active) {
            current.is_active = false;
            Self::log_error_to_file("API Rotasyonu tetiklendi", symbol);
        }
        
        let next = keys.iter_mut().find(|k| !k.is_active)?;
        next.is_active = true;
        Some(next.clone())
    }

    /// Güvenli Yedekleme: BufWriter ile disk I/O optimizasyonu
    pub fn backup_critical_files(&self, files: &[&str], backup_dir: &str) {
        let _ = fs::create_dir_all(backup_dir);
        for file in files {
            let path = Path::new(file);
            if path.exists() {
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
                let backup_path = format!("{}/{}_bak_{}.bak", backup_dir, filename, Utc::now().format("%Y%m%d%H%M%S"));
                if fs::copy(file, &backup_path).is_ok() {
                    Self::log_error_to_file(&format!("Yedeklendi: {}", backup_path), "SYSTEM");
                }
            }
        }
    }

    /// Anomali Tespiti: Real-time (Slicing ile kopyalamasız kontrol)
    pub fn detect_realtime_anomaly(&self, symbol: &str, recent_returns: &[f64], recent_volumes: &[f64], _api_latency_ms: u64) {
        // Ani PnL Değişimi
        if recent_returns.last().is_some_and(|&r| r.abs() > 0.05) {
            Self::log_error_to_file("Anomali: %5+ PnL değişimi!", symbol);
        }

        // Hacim Patlaması
        let v_len = recent_volumes.len();
        if v_len > 5 {
            let avg_vol = recent_volumes[..v_len-1].iter().sum::<f64>() / (v_len - 1) as f64;
            if recent_volumes.last().is_some_and(|&last| last > avg_vol * 2.0) {
                Self::log_error_to_file("Anomali: Hacim patlaması!", symbol);
            }
        }
    }

    pub fn log_error_to_file(error: &str, symbol: &str) {
        let log_path = "logs/pipeline_errors.log";
        let Ok(file) = fs::OpenOptions::new().create(true).append(true).open(log_path) else { return; };
        let mut writer = BufWriter::new(file);
        let _ = writeln!(writer, "{} | {} | {}", Utc::now().to_rfc3339(), symbol, error);
    }
}
