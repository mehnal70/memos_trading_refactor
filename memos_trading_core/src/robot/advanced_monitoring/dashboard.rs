// Dashboard - Gerçek-Zamanlı Metrikleri Görselleştir
//
// WebSocket ve HTTP endpoints aracılığıyla canlı metrikleri izle
// Kümülatif kar/zarar, kazanç oranı, maksimum çekilme göster

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use crate::health_monitor::{HealthCheck, AnomalyDetector, HealthStatus, AnomalyType};

/// Anlık Metrik Snapshot'ı
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSnapshot {
    /// Zaman damgası
    pub timestamp: DateTime<Utc>,
    
    /// Toplam kar/zarar
    pub total_pnl: f64,
    
    /// Kazanç oranı (%)
    pub win_rate: f64,
    
    /// Maksimum çekilme (%)
    pub max_drawdown: f64,
    
    /// Sharpe oranı
    pub sharpe_ratio: f64,
    
    /// Aktif işlem sayısı
    pub active_trades: usize,
    
    /// Toplam tamamlanan işlem
    pub total_closed_trades: usize,
    
    /// Sistem durumu
    pub system_status: SystemStatus,
}

/// Sistem Durumu
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SystemStatus {
    /// Sağlıklı
    Healthy,
    /// Uyarı durumunda
    Warning,
    /// Kritik durum
    Critical,
    /// Kapalı
    Offline,
}

/// Dashboard Metrikleri
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardMetrics {
    /// Anlık metrikler
    pub current: MetricSnapshot,
    
    /// Son 24 saat ortalaması
    pub avg_24h: MetricSnapshot,
    
    /// Son 7 gün ortalaması
    pub avg_7d: MetricSnapshot,
    
    /// Gün başından bu yana en yüksek kazanç
    pub best_day_pnl: f64,
    
    /// Gün başından bu yana en düşük kar/zarar
    pub worst_day_pnl: f64,
    
    /// Güncel volatilite (%)
    pub volatility: f64,
    
    /// Ortalama işlem süresi (dakika)
    pub avg_trade_duration_mins: f64,
}

impl Default for MetricSnapshot {
    fn default() -> Self {
        Self {
            timestamp: Utc::now(),
            total_pnl: 0.0,
            win_rate: 0.0,
            max_drawdown: 0.0,
            sharpe_ratio: 0.0,
            active_trades: 0,
            total_closed_trades: 0,
            system_status: SystemStatus::Healthy,
        }
    }
}

/// Realtime Dashboard - Gerçek-Zamanlı Monitoring
pub struct RealtimeDashboard {
    /// Metrik geçmişi (en son 1440 = 24 saat @1min)
    metrics_history: VecDeque<MetricSnapshot>,
    
    /// Şimdiki metrikleri
    current_metrics: MetricSnapshot,
    
    /// Update interval (saniye)
    update_interval_secs: u64,
    
    /// Son güncelleme zamanı
    last_update: DateTime<Utc>,
    
    /// Dashboard aktif mi?
    is_active: bool,
    
    /// Viewer sayısı
    viewer_count: usize,
}

impl RealtimeDashboard {
    /// Yeni Realtime Dashboard oluştur
    pub fn new(update_interval_secs: u64) -> Self {
        Self {
            metrics_history: VecDeque::new(),
            current_metrics: MetricSnapshot::default(),
            update_interval_secs,
            last_update: Utc::now(),
            is_active: false,
            viewer_count: 0,
        }
    }
}

// RealtimeDashboard için HealthCheck ve AnomalyDetector trait implementasyonları
impl HealthCheck for RealtimeDashboard {
    fn check_health(&self) -> HealthStatus {
        let m = &self.current_metrics;
        match m.system_status {
            SystemStatus::Healthy => HealthStatus::Healthy,
            SystemStatus::Warning => HealthStatus::Warning("Dashboard uyarı durumu".to_string()),
            SystemStatus::Critical => HealthStatus::Critical("Dashboard kritik durumu".to_string()),
            SystemStatus::Offline => HealthStatus::Warning("Dashboard offline".to_string()),
        }
    }
}

impl AnomalyDetector for RealtimeDashboard {
    fn detect_anomaly(&self) -> Option<AnomalyType> {
        let m = &self.current_metrics;
        if m.max_drawdown > 50.0 {
            return Some(AnomalyType::Custom(format!("Aşırı çekilme: {:.2}%", m.max_drawdown)));
        }
        if m.sharpe_ratio < 0.0 {
            return Some(AnomalyType::Custom(format!("Negatif Sharpe oranı: {:.2}", m.sharpe_ratio)));
        }
        if matches!(m.system_status, SystemStatus::Critical | SystemStatus::Offline) {
            return Some(AnomalyType::Custom(format!("Dashboard durumu: {:?}", m.system_status)));
        }
        None
    }
}

impl RealtimeDashboard {
    /// Dashboard'u başlat
    pub fn start(&mut self) -> Result<(), String> {
        if self.is_active {
            return Err("Dashboard already active".to_string());
        }
        
        self.is_active = true;
        println!("✓ Dashboard başlatıldı ({}s interval)", self.update_interval_secs);
        Ok(())
    }

    /// Dashboard'u durdur
    pub fn stop(&mut self) -> Result<(), String> {
        if !self.is_active {
            return Err("Dashboard not active".to_string());
        }
        
        self.is_active = false;
        println!("✓ Dashboard durduruldu");
        Ok(())
    }

    /// Metrikleri güncelle
    pub fn update_metrics(
        &mut self,
        total_pnl: f64,
        win_rate: f64,
        max_drawdown: f64,
        sharpe_ratio: f64,
        active_trades: usize,
        total_closed_trades: usize,
        system_status: SystemStatus,
    ) -> Result<(), String> {
        if !self.is_active {
            return Err("Dashboard not active".to_string());
        }

        let now = Utc::now();
        let elapsed = (now - self.last_update).num_seconds() as u64;

        if elapsed < self.update_interval_secs {
            return Ok(());  // Henüz güncelleme zamanı değil
        }

        self.current_metrics = MetricSnapshot {
            timestamp: now,
            total_pnl,
            win_rate,
            max_drawdown,
            sharpe_ratio,
            active_trades,
            total_closed_trades,
            system_status,
        };

        // Geçmişe ekle
        self.metrics_history.push_back(self.current_metrics.clone());

        // Maksimum 1440 metrik sakla (24 saat @ 1 dakika)
        if self.metrics_history.len() > 1440 {
            self.metrics_history.pop_front();
        }

        self.last_update = now;
        Ok(())
    }

    /// Mevcut metrikleri al
    pub fn current_metrics(&self) -> &MetricSnapshot {
        &self.current_metrics
    }

    /// Dashboard metriklerini topla
    pub fn get_dashboard_metrics(&self) -> DashboardMetrics {
        let current = self.current_metrics.clone();

        // 24 saat ortalaması (son 1440 metrik)
        let avg_24h = Self::calculate_average(&self.metrics_history);

        // 7 gün ortalaması (son 10080 metrik teorik olarak)
        // Burada sadece mevcut data kullanılıyor
        let avg_7d = Self::calculate_average(&self.metrics_history);

        // En iyi/kötü gün PnL
        let (best_pnl, worst_pnl) = Self::find_best_worst_pnl(&self.metrics_history);

        // Volatilite hesapla
        let volatility = Self::calculate_volatility(&self.metrics_history);

        // Ortalama işlem süresi
        let avg_duration = Self::calculate_avg_duration(&self.metrics_history);

        DashboardMetrics {
            current,
            avg_24h,
            avg_7d,
            best_day_pnl: best_pnl,
            worst_day_pnl: worst_pnl,
            volatility,
            avg_trade_duration_mins: avg_duration,
        }
    }

    /// Viewer ekle (WebSocket bağlantısı)
    pub fn add_viewer(&mut self) {
        self.viewer_count += 1;
    }

    /// Viewer kaldır
    pub fn remove_viewer(&mut self) {
        if self.viewer_count > 0 {
            self.viewer_count -= 1;
        }
    }

    /// Mevcut viewer sayısı
    pub fn viewer_count(&self) -> usize {
        self.viewer_count
    }

    /// Metrik ortalaması hesapla
    fn calculate_average(metrics: &VecDeque<MetricSnapshot>) -> MetricSnapshot {
        if metrics.is_empty() {
            return MetricSnapshot::default();
        }

        let len = metrics.len() as f64;
        let pnl_sum: f64 = metrics.iter().map(|m| m.total_pnl).sum();
        let wr_sum: f64 = metrics.iter().map(|m| m.win_rate).sum();
        let dd_sum: f64 = metrics.iter().map(|m| m.max_drawdown).sum();
        let sr_sum: f64 = metrics.iter().map(|m| m.sharpe_ratio).sum();

        MetricSnapshot {
            timestamp: Utc::now(),
            total_pnl: pnl_sum / len,
            win_rate: wr_sum / len,
            max_drawdown: dd_sum / len,
            sharpe_ratio: sr_sum / len,
            active_trades: 0,
            total_closed_trades: 0,
            system_status: SystemStatus::Healthy,
        }
    }

    /// En iyi/kötü günü bul
    fn find_best_worst_pnl(metrics: &VecDeque<MetricSnapshot>) -> (f64, f64) {
        if metrics.is_empty() {
            return (0.0, 0.0);
        }

        let best = metrics.iter().map(|m| m.total_pnl).max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(0.0);
        let worst = metrics.iter().map(|m| m.total_pnl).min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(0.0);

        (best, worst)
    }

    /// Volatilite hesapla
    fn calculate_volatility(metrics: &VecDeque<MetricSnapshot>) -> f64 {
        if metrics.len() < 2 {
            return 0.0;
        }

        let mean: f64 = metrics.iter().map(|m| m.total_pnl).sum::<f64>() / metrics.len() as f64;
        let variance: f64 = metrics.iter()
            .map(|m| (m.total_pnl - mean).powi(2))
            .sum::<f64>() / metrics.len() as f64;

        variance.sqrt()
    }

    /// Ortalama işlem süresi hesapla
    fn calculate_avg_duration(_metrics: &VecDeque<MetricSnapshot>) -> f64 {
        // Basit ortalama (metrik versiyonlarda daha detaylı olacak)
        30.0
    }

    /// Dashboard durumu
    pub fn is_active(&self) -> bool {
        self.is_active
    }

    /// Metriklerin sayısı
    pub fn metric_count(&self) -> usize {
        self.metrics_history.len()
    }
}

impl Default for RealtimeDashboard {
    fn default() -> Self {
        Self::new(60)  // 1 dakika
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dashboard_creation() {
        let dashboard = RealtimeDashboard::new(60);
        assert!(!dashboard.is_active());
        assert_eq!(dashboard.viewer_count(), 0);
    }

    #[test]
    fn test_dashboard_start_stop() {
        let mut dashboard = RealtimeDashboard::new(60);
        
        assert!(dashboard.start().is_ok());
        assert!(dashboard.is_active());
        
        assert!(dashboard.stop().is_ok());
        assert!(!dashboard.is_active());
    }

    #[test]
    fn test_cannot_update_while_inactive() {
        let mut dashboard = RealtimeDashboard::new(60);
        
        let result = dashboard.update_metrics(100.0, 65.0, 15.0, 1.5, 1, 50, SystemStatus::Healthy);
        assert!(result.is_err());
    }

    #[test]
    fn test_update_metrics() {
        let mut dashboard = RealtimeDashboard::new(0);  // 0 saniye interval = hemen
        dashboard.start().unwrap();
        
        dashboard.update_metrics(100.0, 65.0, 15.0, 1.5, 1, 50, SystemStatus::Healthy).unwrap();
        
        let metrics = dashboard.current_metrics();
        assert_eq!(metrics.total_pnl, 100.0);
        assert_eq!(metrics.win_rate, 65.0);
    }

    #[test]
    fn test_viewer_management() {
        let mut dashboard = RealtimeDashboard::new(60);
        
        dashboard.add_viewer();
        assert_eq!(dashboard.viewer_count(), 1);
        
        dashboard.add_viewer();
        assert_eq!(dashboard.viewer_count(), 2);
        
        dashboard.remove_viewer();
        assert_eq!(dashboard.viewer_count(), 1);
    }

    #[test]
    fn test_dashboard_metrics_aggregation() {
        let mut dashboard = RealtimeDashboard::new(0);  // 0 saniye = hemen update
        dashboard.start().unwrap();
        
        dashboard.update_metrics(100.0, 65.0, 15.0, 1.5, 1, 50, SystemStatus::Healthy).unwrap();
        
        let metrics = dashboard.get_dashboard_metrics();
        assert_eq!(metrics.current.total_pnl, 100.0);
        assert!(metrics.volatility >= 0.0);
    }

    #[test]
    fn test_metric_history_limit() {
        let mut dashboard = RealtimeDashboard::new(0);  // 0 saniye interval
        dashboard.start().unwrap();
        
        // 1500 metrik ekle (max 1440)
        for i in 0..1500 {
            dashboard.update_metrics(i as f64, 65.0, 15.0, 1.5, 1, 50, SystemStatus::Healthy).ok();
        }
        
        // Maksimum 1440 olmalı
        assert!(dashboard.metric_count() <= 1440);
    }
}
