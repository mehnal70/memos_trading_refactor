// Performance Trending Engine - Performans Trendlerini Analiz Et
//
// Tarihsel verileri analiz et, trend çizgilerini hesapla
// Gelecek performans tahmini (basit linear regression)

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use crate::health_monitor::{HealthCheck, AnomalyDetector, HealthStatus, AnomalyType};

/// Trend Verileri
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendData {
    /// Zaman damgası
    pub timestamp: DateTime<Utc>,
    
    /// PnL değeri
    pub pnl: f64,
    
    /// Win rate (%)
    pub win_rate: f64,
    
    /// Sharpe ratio
    pub sharpe_ratio: f64,
}

/// Trend Türü
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PerformanceTrend {
    /// Güçlü yükselişte
    StrongUptrend,
    /// Hafif yükselişte
    Uptrend,
    /// Sabit
    Sideways,
    /// Hafif düşüşte
    Downtrend,
    /// Güçlü düşüşte
    StrongDowntrend,
}

/// Trend Analiz Sonuçları
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendAnalysis {
    /// Trend türü
    pub trend: PerformanceTrend,
    
    /// Trend gücü (0.0 - 1.0)
    pub strength: f64,
    
    /// Slope (PnL/gün)
    pub slope: f64,
    
    /// Korrelasyon katsayısı
    pub correlation: f64,
    
    /// Tahmin edilen sonraki PnL
    pub predicted_pnl_24h: f64,
    
    /// Volatilite (%)
    pub volatility: f64,
    
    /// Volatilite trendi
    pub volatility_trend: PerformanceTrend,
}

impl Default for TrendAnalysis {
    fn default() -> Self {
        Self {
            trend: PerformanceTrend::Sideways,
            strength: 0.0,
            slope: 0.0,
            correlation: 0.0,
            predicted_pnl_24h: 0.0,
            volatility: 0.0,
            volatility_trend: PerformanceTrend::Sideways,
        }
    }
}

/// Performance Trending Engine
pub struct PerformanceTrendingEngine {
    /// Trend verileri geçmişi
    trend_data: VecDeque<TrendData>,
    
    /// Şimdiki analiz
    current_analysis: TrendAnalysis,
    
    /// Maksimum veri tutma (örn: 30 günlük veri = 30 snapshots)
    max_data_points: usize,
    
    /// Son analiz zamanı
    last_analysis_time: Option<DateTime<Utc>>,
}

impl PerformanceTrendingEngine {
    /// Yeni Trending Engine oluştur
    pub fn new(max_data_points: usize) -> Self {
        Self {
            trend_data: VecDeque::new(),
            current_analysis: TrendAnalysis::default(),
            max_data_points,
            last_analysis_time: None,
        }
    }
}

// PerformanceTrendingEngine için HealthCheck ve AnomalyDetector trait implementasyonları
impl HealthCheck for PerformanceTrendingEngine {
    fn check_health(&self) -> HealthStatus {
        let a = &self.current_analysis;
        if matches!(a.trend, PerformanceTrend::StrongDowntrend) && a.strength > 0.7 {
            HealthStatus::Warning("Güçlü düşüş trendi tespit edildi".to_string())
        } else if a.slope < 0.0 && a.strength > 0.5 {
            HealthStatus::Warning("Negatif eğimli trend tespit edildi".to_string())
        } else {
            HealthStatus::Healthy
        }
    }
}

impl AnomalyDetector for PerformanceTrendingEngine {
    fn detect_anomaly(&self) -> Option<AnomalyType> {
        let a = &self.current_analysis;
        if matches!(a.trend, PerformanceTrend::StrongDowntrend) && a.strength > 0.9 {
            return Some(AnomalyType::Custom("Aşırı güçlü düşüş trendi!".to_string()));
        }
        if a.slope < -100.0 {
            return Some(AnomalyType::Custom(format!("Çok negatif slope: {:.2}", a.slope)));
        }
        None
    }
}

impl PerformanceTrendingEngine {
    /// Trend verileri ekle
    pub fn add_data(
        &mut self,
        pnl: f64,
        win_rate: f64,
        sharpe_ratio: f64,
    ) -> Result<(), String> {
        let data = TrendData {
            timestamp: Utc::now(),
            pnl,
            win_rate,
            sharpe_ratio,
        };

        self.trend_data.push_back(data);

        // Maksimum limit kontrol
        if self.trend_data.len() > self.max_data_points {
            self.trend_data.pop_front();
        }

        // Otomatik analiz yap (en az 5 veri noktası gerekli)
        if self.trend_data.len() >= 5 {
            self.analyze()?;
        }

        Ok(())
    }

    /// Trendi analiz et
    pub fn analyze(&mut self) -> Result<(), String> {
        if self.trend_data.len() < 2 {
            return Err("Insufficient data for analysis".to_string());
        }

        // PnL trendi analiz et
        let pnl_trend = Self::analyze_pnl_trend(&self.trend_data);
        
        // Volatilite trendi analiz et
        let vol_trend = Self::analyze_volatility_trend(&self.trend_data);
        
        // Linear regression hesapla
        let (slope, correlation, predicted) = Self::linear_regression(&self.trend_data);

        self.current_analysis = TrendAnalysis {
            trend: pnl_trend.0,
            strength: pnl_trend.1,
            slope,
            correlation,
            predicted_pnl_24h: predicted,
            volatility: Self::calculate_volatility(&self.trend_data),
            volatility_trend: vol_trend,
        };

        self.last_analysis_time = Some(Utc::now());
        Ok(())
    }

    /// PnL trendini analiz et
    fn analyze_pnl_trend(data: &VecDeque<TrendData>) -> (PerformanceTrend, f64) {
        if data.len() < 2 {
            return (PerformanceTrend::Sideways, 0.0);
        }

        let pnls: Vec<f64> = data.iter().map(|d| d.pnl).collect();
        let first_half_avg = pnls[..pnls.len() / 2].iter().sum::<f64>() / (pnls.len() / 2) as f64;
        let second_half_avg = pnls[pnls.len() / 2..].iter().sum::<f64>() / (pnls.len() - pnls.len() / 2) as f64;

        let change_pct = ((second_half_avg - first_half_avg) / first_half_avg.abs() * 100.0).abs();

        let trend = if second_half_avg > first_half_avg {
            if change_pct > 15.0 {
                PerformanceTrend::StrongUptrend
            } else if change_pct > 5.0 {
                PerformanceTrend::Uptrend
            } else {
                PerformanceTrend::Sideways
            }
        } else {
            if change_pct > 15.0 {
                PerformanceTrend::StrongDowntrend
            } else if change_pct > 5.0 {
                PerformanceTrend::Downtrend
            } else {
                PerformanceTrend::Sideways
            }
        };

        (trend, (change_pct / 100.0).min(1.0))
    }

    /// Volatilite trendini analiz et
    fn analyze_volatility_trend(data: &VecDeque<TrendData>) -> PerformanceTrend {
        if data.len() < 4 {
            return PerformanceTrend::Sideways;
        }

        let pnls: Vec<f64> = data.iter().map(|d| d.pnl).collect();
        let first_half_vol = Self::calc_std_dev(&pnls[..pnls.len() / 2]);
        let second_half_vol = Self::calc_std_dev(&pnls[pnls.len() / 2..]);

        if second_half_vol > first_half_vol * 1.2 {
            PerformanceTrend::StrongUptrend
        } else if second_half_vol < first_half_vol * 0.8 {
            PerformanceTrend::StrongDowntrend
        } else {
            PerformanceTrend::Sideways
        }
    }

    /// Linear Regression - Slope, Correlation, Prediction
    fn linear_regression(data: &VecDeque<TrendData>) -> (f64, f64, f64) {
        if data.len() < 2 {
            return (0.0, 0.0, 0.0);
        }

        let n = data.len() as f64;
        let pnls: Vec<f64> = data.iter().map(|d| d.pnl).collect();
        
        // X = 0, 1, 2, ... (gün indeksi)
        let x_sum: f64 = (0..data.len()).map(|i| i as f64).sum();
        let y_sum: f64 = pnls.iter().sum();
        let x_mean = x_sum / n;
        let y_mean = y_sum / n;

        // Slope hesapla
        let mut numerator = 0.0;
        let mut denominator = 0.0;
        for (i, pnl) in pnls.iter().enumerate() {
            let x_diff = i as f64 - x_mean;
            let y_diff = pnl - y_mean;
            numerator += x_diff * y_diff;
            denominator += x_diff * x_diff;
        }

        let slope = if denominator != 0.0 {
            numerator / denominator
        } else {
            0.0
        };

        // Correlation (R²) hesapla
        let mut ss_tot = 0.0;
        let mut ss_res = 0.0;
        for (i, pnl) in pnls.iter().enumerate() {
            let y_pred = y_mean + slope * (i as f64 - x_mean);
            ss_tot += (pnl - y_mean).powi(2);
            ss_res += (pnl - y_pred).powi(2);
        }

        let correlation = if ss_tot != 0.0 {
            1.0 - (ss_res / ss_tot)
        } else {
            0.0
        };

        // 24 saat sonrası tahmin (bir sonraki nokta)
        let next_x = n;
        let predicted = y_mean + slope * (next_x - x_mean);

        (slope, correlation.max(0.0), predicted)
    }

    /// Volatilite hesapla
    fn calculate_volatility(data: &VecDeque<TrendData>) -> f64 {
        let pnls: Vec<f64> = data.iter().map(|d| d.pnl).collect();
        Self::calc_std_dev(&pnls)
    }

    /// Standard sapma hesapla
    fn calc_std_dev(values: &[f64]) -> f64 {
        if values.is_empty() {
            return 0.0;
        }

        let mean: f64 = values.iter().sum::<f64>() / values.len() as f64;
        let variance: f64 = values.iter()
            .map(|v| (v - mean).powi(2))
            .sum::<f64>() / values.len() as f64;

        variance.sqrt()
    }

    /// Mevcut analizi al
    pub fn current_analysis(&self) -> &TrendAnalysis {
        &self.current_analysis
    }

    /// Veri sayısı
    pub fn data_count(&self) -> usize {
        self.trend_data.len()
    }

    /// Trend verileri geçmişini al
    pub fn get_trend_data(&self, limit: usize) -> Vec<TrendData> {
        self.trend_data
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }
}

impl Default for PerformanceTrendingEngine {
    fn default() -> Self {
        Self::new(30)  // 30 gün = 30 snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_creation() {
        let engine = PerformanceTrendingEngine::new(30);
        assert_eq!(engine.data_count(), 0);
    }

    #[test]
    fn test_add_trend_data() {
        let mut engine = PerformanceTrendingEngine::new(30);
        
        engine.add_data(100.0, 65.0, 1.5).unwrap();
        assert_eq!(engine.data_count(), 1);
        
        engine.add_data(110.0, 66.0, 1.6).unwrap();
        assert_eq!(engine.data_count(), 2);
    }

    #[test]
    fn test_insufficient_data_for_analysis() {
        let mut engine = PerformanceTrendingEngine::new(30);
        
        engine.add_data(100.0, 65.0, 1.5).unwrap();
        engine.add_data(110.0, 66.0, 1.6).unwrap();
        
        // 2 veri noktası ile analiz başarısız (en az 5 gerekli)
        assert_eq!(engine.data_count(), 2);
    }

    #[test]
    fn test_uptrend_detection() {
        let mut engine = PerformanceTrendingEngine::new(30);
        
        // Yükselişte veri ekle
        for i in 0..5 {
            engine.add_data(100.0 + i as f64 * 20.0, 65.0, 1.5).ok();
        }
        
        let analysis = engine.current_analysis();
        assert!(matches!(analysis.trend, PerformanceTrend::Uptrend | PerformanceTrend::StrongUptrend));
    }

    #[test]
    fn test_downtrend_detection() {
        let mut engine = PerformanceTrendingEngine::new(30);
        
        // Düşüşte veri ekle
        for i in 0..5 {
            engine.add_data(200.0 - i as f64 * 20.0, 65.0, 1.5).ok();
        }
        
        let analysis = engine.current_analysis();
        assert!(matches!(analysis.trend, PerformanceTrend::Downtrend | PerformanceTrend::StrongDowntrend));
    }

    #[test]
    fn test_sideways_trend() {
        let mut engine = PerformanceTrendingEngine::new(30);
        
        // Sabit veri ekle
        for _ in 0..5 {
            engine.add_data(150.0, 65.0, 1.5).ok();
        }
        
        let analysis = engine.current_analysis();
        assert_eq!(analysis.trend, PerformanceTrend::Sideways);
    }

    #[test]
    fn test_data_limit() {
        let mut engine = PerformanceTrendingEngine::new(5);
        
        for i in 0..10 {
            engine.add_data(i as f64 * 10.0, 65.0, 1.5).ok();
        }
        
        assert_eq!(engine.data_count(), 5);  // Maximum limited to 5
    }

    #[test]
    fn test_prediction() {
        let mut engine = PerformanceTrendingEngine::new(30);
        
        // Doğrusal yükselişte veri ekle
        for i in 0..5 {
            engine.add_data(100.0 + i as f64 * 10.0, 65.0, 1.5).ok();
        }
        
        let analysis = engine.current_analysis();
        // Tahmini değer, sonraki trend noktasında yüksek olmalı
        assert!(analysis.predicted_pnl_24h > 100.0);
    }

    #[test]
    fn test_volatility_calculation() {
        let mut engine = PerformanceTrendingEngine::new(30);
        
        // Yüksek volatilite veri ekle
        for i in 0..5 {
            let pnl = if i % 2 == 0 { 100.0 } else { 200.0 };
            engine.add_data(pnl, 65.0, 1.5).ok();
        }
        
        let analysis = engine.current_analysis();
        assert!(analysis.volatility > 0.0);
    }
}
