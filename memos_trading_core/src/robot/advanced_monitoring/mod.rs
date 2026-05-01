// Srivastava ATP Mimarisi - Advanced Monitoring (Tier 4)
//
// Gerçek-zamanlı dashboard, uyarı sistemi (email/Slack), performans trendleri
// Sistem sağlığı ve ticaret performansı izleme

pub mod dashboard;
pub mod alert_system;
pub mod performance_trending;

pub use dashboard::{RealtimeDashboard, DashboardMetrics, MetricSnapshot};
pub use alert_system::{AlertSystem, AlertLevel, Alert, AlertChannel, AlertConfig};
pub use performance_trending::{PerformanceTrendingEngine, TrendData, PerformanceTrend, TrendAnalysis};

#[cfg(test)]
mod tests {
    

    #[test]
    fn test_advanced_monitoring_module_loads() {
        // Modül başarıyla yüklendiğini kontrol et
        println!("✓ Advanced Monitoring module loaded successfully");
    }
}
