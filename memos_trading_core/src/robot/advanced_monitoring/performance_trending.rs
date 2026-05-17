// performance_trending_engine.rs - Performans Trend Analiz ve Tahmin Motoru

use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use crate::health_monitor::{HealthCheck, AnomalyDetector, HealthStatus, AnomalyType};

// --- 1. VERİ MODELLERİ ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendData {
    pub timestamp: DateTime<Utc>,
    pub pnl: f64,
    pub win_rate: f64,
    pub sharpe_ratio: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PerformanceTrend {
    StrongUptrend,
    Uptrend,
    Sideways,
    Downtrend,
    StrongDowntrend,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendAnalysis {
    pub trend: PerformanceTrend,
    pub strength: f64,
    pub slope: f64,
    pub correlation: f64,
    pub predicted_pnl_24h: f64,
    pub volatility: f64,
    pub volatility_trend: PerformanceTrend,
}

impl Default for TrendAnalysis {
    fn default() -> Self {
        Self {
            trend: PerformanceTrend::Sideways,
            strength: 0.0, slope: 0.0, correlation: 0.0,
            predicted_pnl_24h: 0.0, volatility: 0.0,
            volatility_trend: PerformanceTrend::Sideways,
        }
    }
}

// --- 2. ANALİZ MOTORU ---

pub struct PerformanceTrendingEngine {
    trend_data: VecDeque<TrendData>,
    current_analysis: TrendAnalysis,
    max_data_points: usize,
    last_analysis_time: Option<DateTime<Utc>>,
}

impl PerformanceTrendingEngine {
    pub fn new(max_data_points: usize) -> Self {
        Self {
            trend_data: VecDeque::with_capacity(max_data_points),
            current_analysis: TrendAnalysis::default(),
            max_data_points,
            last_analysis_time: None,
        }
    }

    pub fn add_data(&mut self, pnl: f64, win_rate: f64, sharpe_ratio: f64) -> Result<(), String> {
        let data = TrendData { timestamp: Utc::now(), pnl, win_rate, sharpe_ratio };
        self.trend_data.push_back(data);

        if self.trend_data.len() > self.max_data_points {
            self.trend_data.pop_front();
        }

        if self.trend_data.len() >= 5 { self.analyze()?; }
        Ok(())
    }

    pub fn analyze(&mut self) -> Result<(), String> {
        let n = self.trend_data.len();
        if n < 2 { return Err("Analiz için yetersiz veri".to_owned()); }

        let pnl_trend = self.analyze_pnl_trend();
        let vol_trend = self.analyze_volatility_trend();
        let (slope, correlation, predicted) = self.calculate_linear_regression();

        self.current_analysis = TrendAnalysis {
            trend: pnl_trend.0,
            strength: pnl_trend.1,
            slope,
            correlation,
            predicted_pnl_24h: predicted,
            volatility: self.calculate_current_volatility(),
            volatility_trend: vol_trend,
        };

        self.last_analysis_time = Some(Utc::now());
        Ok(())
    }

    // --- ÖZEL MATEMATİKSEL MOTORLAR (OPTIMIZED) ---

    fn analyze_pnl_trend(&self) -> (PerformanceTrend, f64) {
        let n = self.trend_data.len();
        let mid = n / 2;
        
        let first_half_avg: f64 = self.trend_data.iter().take(mid).map(|d| d.pnl).sum::<f64>() / mid as f64;
        let second_half_avg: f64 = self.trend_data.iter().skip(mid).map(|d| d.pnl).sum::<f64>() / (n - mid) as f64;

        let diff = second_half_avg - first_half_avg;
        let change_pct = (diff / first_half_avg.abs().max(f64::EPSILON) * 100.0).abs();

        let trend = match diff {
            d if d > 0.0 => if change_pct > 15.0 { PerformanceTrend::StrongUptrend } else if change_pct > 5.0 { PerformanceTrend::Uptrend } else { PerformanceTrend::Sideways },
            _ => if change_pct > 15.0 { PerformanceTrend::StrongDowntrend } else if change_pct > 5.0 { PerformanceTrend::Downtrend } else { PerformanceTrend::Sideways },
        };

        (trend, (change_pct / 100.0).min(1.0))
    }

    fn analyze_volatility_trend(&self) -> PerformanceTrend {
        let n = self.trend_data.len();
        if n < 4 { return PerformanceTrend::Sideways; }
        
        let mid = n / 2;
        let vol1 = self.calc_slice_std_dev(0, mid);
        let vol2 = self.calc_slice_std_dev(mid, n);

        if vol2 > vol1 * 1.2 { PerformanceTrend::StrongUptrend }
        else if vol2 < vol1 * 0.8 { PerformanceTrend::StrongDowntrend }
        else { PerformanceTrend::Sideways }
    }

    fn calculate_linear_regression(&self) -> (f64, f64, f64) {
        let n_usize = self.trend_data.len();
        let n = n_usize as f64;
        
        let x_sum: f64 = (0..n_usize).map(|i| i as f64).sum();
        let y_sum: f64 = self.trend_data.iter().map(|d| d.pnl).sum();
        let x_mean = x_sum / n;
        let y_mean = y_sum / n;

        let (mut num, mut den) = (0.0, 0.0);
        for (i, d) in self.trend_data.iter().enumerate() {
            let x_diff = i as f64 - x_mean;
            let y_diff = d.pnl - y_mean;
            num += x_diff * y_diff;
            den += x_diff * x_diff;
        }

        let slope = if den.abs() > f64::EPSILON { num / den } else { 0.0 };

        // R² (Korelasyon) Hesabı
        let (mut ss_tot, mut ss_res) = (0.0, 0.0);
        for (i, d) in self.trend_data.iter().enumerate() {
            let y_pred = y_mean + slope * (i as f64 - x_mean);
            ss_tot += (d.pnl - y_mean).powi(2);
            ss_res += (d.pnl - y_pred).powi(2);
        }

        let correlation = if ss_tot > f64::EPSILON { (1.0 - (ss_res / ss_tot)).max(0.0) } else { 0.0 };
        let predicted = y_mean + slope * (n - x_mean);

        (slope, correlation, predicted)
    }

    fn calculate_current_volatility(&self) -> f64 {
        self.calc_slice_std_dev(0, self.trend_data.len())
    }

    fn calc_slice_std_dev(&self, start: usize, end: usize) -> f64 {
        let count = end - start;
        if count == 0 { return 0.0; }
        
        let slice_sum: f64 = self.trend_data.iter().skip(start).take(count).map(|d| d.pnl).sum();
        let mean = slice_sum / count as f64;
        
        let var_sum: f64 = self.trend_data.iter().skip(start).take(count)
            .map(|d| (d.pnl - mean).powi(2)).sum();
        
        (var_sum / count as f64).sqrt()
    }

    // --- GETTERS & UTILS ---
    pub fn current_analysis(&self) -> &TrendAnalysis { &self.current_analysis }
    pub fn data_count(&self) -> usize { self.trend_data.len() }
    pub fn get_trend_data(&self, limit: usize) -> Vec<TrendData> {
        self.trend_data.iter().rev().take(limit).cloned().collect()
    }
}

// --- TRAIT ENTEGRASYONLARI ---

impl HealthCheck for PerformanceTrendingEngine {
    fn check_health(&self) -> HealthStatus {
        let a = &self.current_analysis;
        match a.trend {
            PerformanceTrend::StrongDowntrend if a.strength > 0.7 => HealthStatus::Warning("Kritik Performans Düşüşü".to_owned()),
            _ if a.slope < -50.0 => HealthStatus::Warning("Negatif Trend Eğimi".to_owned()),
            _ => HealthStatus::Healthy,
        }
    }
}

impl AnomalyDetector for PerformanceTrendingEngine {
    fn detect_anomaly(&self) -> Option<AnomalyType> {
        let a = &self.current_analysis;
        if a.correlation > 0.9 && a.slope < -100.0 {
            return Some(AnomalyType::Custom("Lineer Çöküş Anomalisi".to_owned()));
        }
        None
    }
}

impl Default for PerformanceTrendingEngine {
    fn default() -> Self { Self::new(30) }
}
