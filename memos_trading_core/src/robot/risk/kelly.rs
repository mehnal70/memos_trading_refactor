// src/robot/advanced_risk/kelly.rs - Optimal Pozisyon Büyüklüğü ve Dinamik Risk Ölçekleme
use crate::prelude::*;
#[derive(Default)] pub struct KellyCalculator;
impl KellyCalculator { pub fn validate_allocation(&self, _sig: &Signal, _equity: f64) -> bool { true } }

use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct KellyCriterion {
    pub win_probability: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub kelly_fraction: f64,
}

impl KellyCriterion {
    pub fn calculate(win_probability: f64, avg_win: f64, avg_loss: f64) -> Self {
        let win_prob = win_probability.clamp(0.0, 1.0);
        if avg_win <= 0.0 || avg_loss <= 0.0 {
            return Self { win_probability: win_prob, avg_win, avg_loss, kelly_fraction: 0.0 };
        }
        
        let loss_prob = 1.0 - win_prob;
        let win_ratio = avg_win / avg_loss;
        
        let kelly = if win_ratio > f64::EPSILON {
            (win_prob * win_ratio - loss_prob) / win_ratio
        } else { 0.0 };
        
        Self {
            win_probability: win_prob,
            avg_win,
            avg_loss,
            kelly_fraction: kelly.max(0.0),
        }
    }

    /// Otonom Risk Ölçekleyici: robotic_loop'un içindeki "Loss Streak" ve "ML Confidence" mantığını buraya aldık.
    /// f* = Kelly_Fraction * ML_Conf * (0.80 ^ (Streak/2))
    pub fn calculate_dynamic_scale(
        &self, 
        base_qty: f64, 
        loss_streak: usize, 
        ml_confidence: f64 
    ) -> f64 {
        // 1. Half-Kelly Uygula (Srivastava standardı: Full Kelly çok agresiftir)
        let mut scale = self.kelly_fraction * 0.5;

        // 2. ML Güven Ölçeklemesi (0.75x ile 1.25x arası)
        let ml_factor = 0.75 + (ml_confidence.clamp(0.0, 1.0) * 0.50);
        scale *= ml_factor;

        // 3. Loss Streak Cezası (Watchdog): 5 ve üzeri zararda üstel küçülme
        if loss_streak >= 5 {
            let penalty = 0.80f64.powi(((loss_streak - 4) / 2).max(1) as i32).max(0.25);
            scale *= penalty;
        }

        (base_qty * scale).max(0.0)
    }
    
    #[inline]
    pub fn calculate_position_size(&self, account_size: f64) -> f64 {
        account_size * self.kelly_fraction
    }
}

// KellyRecommendation yapısı aynen korunabilir...

/// Kelly Analiz ve Tavsiye Çıktısı
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KellyRecommendation {
    pub kelly_pct: f64,
    pub half_kelly_pct: f64,
    pub quarter_kelly_pct: f64,
    pub recommendation: &'static str, // String yerine statik referans (Zero-allocation)
}

impl KellyRecommendation {
    /// Mevcut Kelly verisinden otonom tavsiye üretir
    pub fn from_kelly(kelly: &KellyCriterion) -> Self {
        let kelly_pct = kelly.kelly_fraction * 100.0;
        
        // Karar Ağacı - Modern Rust Pattern Matching
        let recommendation = match kelly_pct {
            p if p < 5.0  => "Too risky - use 1/4 Kelly or lower",
            p if p < 10.0 => "Risky - recommend 1/2 Kelly",
            p if p < 25.0 => "Moderate - Full Kelly or 3/4",
            _             => "Very profitable - Full Kelly recommended",
        };
        
        Self {
            kelly_pct,
            half_kelly_pct: kelly_pct * 0.5,
            quarter_kelly_pct: kelly_pct * 0.25,
            recommendation,
        }
    }
}
