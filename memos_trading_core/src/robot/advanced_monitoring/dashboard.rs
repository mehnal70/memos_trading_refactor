// dashboard.rs - Gerçek Zamanlı Metrik İzleme ve Görselleştirme

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use crate::health_monitor::{HealthCheck, AnomalyDetector, HealthStatus, AnomalyType};

// --- 1. VERİ YAPILARI ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSnapshot {
    pub timestamp: DateTime<Utc>,
    pub total_pnl: f64,
    pub win_rate: f64,
    pub max_drawdown: f64,
    pub sharpe_ratio: f64,
    pub active_trades: usize,
    pub total_closed_trades: usize,
    pub system_status: SystemStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SystemStatus {
    Healthy,
    Warning,
    Critical,
    Offline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardMetrics {
    pub current: MetricSnapshot,
    pub avg_24h: MetricSnapshot,
    pub avg_7d: MetricSnapshot,
    pub best_day_pnl: f64,
    pub worst_day_pnl: f64,
    pub volatility: f64,
    pub avg_trade_duration_mins: f64,
}

impl Default for MetricSnapshot {
    fn default() -> Self {
        Self {
            timestamp: Utc::now(),
            total_pnl: 0.0, win_rate: 0.0, max_drawdown: 0.0,
            sharpe_ratio: 0.0, active_trades: 0, total_closed_trades: 0,
            system_status: SystemStatus::Healthy,
        }
    }
}

// --- 2. DASHBOARD MOTORU ---

pub struct RealtimeDashboard {
    metrics_history: VecDeque<MetricSnapshot>,
    current_metrics: MetricSnapshot,
    update_interval_secs: u64,
    last_update: DateTime<Utc>,
    is_active: bool,
    viewer_count: usize,
}

impl RealtimeDashboard {
    pub fn new(update_interval_secs: u64) -> Self {
        Self {
            metrics_history: VecDeque::with_capacity(1440),
            current_metrics: MetricSnapshot::default(),
            update_interval_secs,
            last_update: Utc::now(),
            is_active: false,
            viewer_count: 0,
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        if self.is_active { return Err("Dashboard zaten aktif".to_owned()); }
        self.is_active = true;
        println!("✓ Dashboard otonom izleme modunda başlatıldı");
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        if !self.is_active { return Err("Dashboard aktif değil".to_owned()); }
        self.is_active = false;
        Ok(())
    }

    /// Metrikleri günceller - Performans: Zaman eşiği kontrolü ile gereksiz hesaplamayı önler.
    pub fn update_metrics(
        &mut self,
        total_pnl: f64, win_rate: f64, max_drawdown: f64,
        sharpe_ratio: f64, active_trades: usize, total_closed_trades: usize,
        system_status: SystemStatus,
    ) -> Result<(), String> {
        if !self.is_active { return Err("Dashboard aktif değil".to_owned()); }

        let now = Utc::now();
        if (now - self.last_update).num_seconds() < self.update_interval_secs as i64 {
            return Ok(());
        }

        self.current_metrics = MetricSnapshot {
            timestamp: now, total_pnl, win_rate, max_drawdown,
            sharpe_ratio, active_trades, total_closed_trades, system_status,
        };

        self.metrics_history.push_back(self.current_metrics.clone());
        if self.metrics_history.len() > 1440 { self.metrics_history.pop_front(); }

        self.last_update = now;
        Ok(())
    }

    /// Tüm dashboard raporunu derler - Performans: Verileri kopyalamadan (zero-copy) analiz eder.
    pub fn get_dashboard_metrics(&self) -> DashboardMetrics {
        let (best_pnl, worst_pnl) = self.find_best_worst_pnl();
        
        DashboardMetrics {
            current: self.current_metrics.clone(),
            avg_24h: self.calculate_average(),
            avg_7d: self.calculate_average(), // Mevcut buffer'ı kullanır
            best_day_pnl: best_pnl,
            worst_day_pnl: worst_pnl,
            volatility: self.calculate_volatility(),
            avg_trade_duration_mins: 30.0, // Statik veya dinamik bağlanabilir
        }
    }

    // --- ANALİZ YARDIMCILARI (INTERNAL) ---

    fn calculate_average(&self) -> MetricSnapshot {
        let n = self.metrics_history.len();
        if n == 0 { return MetricSnapshot::default(); }

        let (mut pnl, mut wr, mut dd, mut sr) = (0.0, 0.0, 0.0, 0.0);
        for m in &self.metrics_history {
            pnl += m.total_pnl;
            wr += m.win_rate;
            dd += m.max_drawdown;
            sr += m.sharpe_ratio;
        }

        let len = n as f64;
        MetricSnapshot {
            timestamp: Utc::now(),
            total_pnl: pnl / len,
            win_rate: wr / len,
            max_drawdown: dd / len,
            sharpe_ratio: sr / len,
            active_trades: 0,
            total_closed_trades: 0,
            system_status: SystemStatus::Healthy,
        }
    }

    fn find_best_worst_pnl(&self) -> (f64, f64) {
        self.metrics_history.iter()
            .fold((f64::MIN, f64::MAX), |(max, min), m| {
                (max.max(m.total_pnl), min.min(m.total_pnl))
            })
    }

    fn calculate_volatility(&self) -> f64 {
        let n = self.metrics_history.len();
        if n < 2 { return 0.0; }

        let mean = self.metrics_history.iter().map(|m| m.total_pnl).sum::<f64>() / n as f64;
        let variance = self.metrics_history.iter()
            .map(|m| (m.total_pnl - mean).powi(2))
            .sum::<f64>() / n as f64;

        variance.sqrt()
    }

    // --- VIEWER YÖNETİMİ ---

    pub fn add_viewer(&mut self) { self.viewer_count += 1; }
    pub fn remove_viewer(&mut self) { self.viewer_count = self.viewer_count.saturating_sub(1); }
    pub fn viewer_count(&self) -> usize { self.viewer_count }
    pub fn current_metrics(&self) -> &MetricSnapshot { &self.current_metrics }
}

// --- TRAIT ENTEGRASYONLARI ---

impl HealthCheck for RealtimeDashboard {
    fn check_health(&self) -> HealthStatus {
        match self.current_metrics.system_status {
            SystemStatus::Healthy => HealthStatus::Healthy,
            SystemStatus::Warning => HealthStatus::Warning("Dashboard uyarı eşiğinde".to_owned()),
            SystemStatus::Critical => HealthStatus::Critical("Dashboard kritik düzeyde!".to_owned()),
            SystemStatus::Offline => HealthStatus::Warning("Dashboard veri akışı kesildi (Offline)".to_owned()),
        }
    }
}

impl AnomalyDetector for RealtimeDashboard {
    fn detect_anomaly(&self) -> Option<AnomalyType> {
        let m = &self.current_metrics;
        if m.max_drawdown > 50.0 {
            return Some(AnomalyType::Custom(format!("Aşırı Çekilme Anomalisi: {:.2}%", m.max_drawdown)));
        }
        if m.sharpe_ratio < -1.0 {
            return Some(AnomalyType::Custom("Negatif Performans Anomalisi".to_owned()));
        }
        None
    }
}
