// src/core/math.rs - Memos Trading Core Library (Srivastava ATP - Mutlak Matematik Üssü)
// Srivastava ATP - Adli Matematik ve Finansal Hesaplama Birimi
// Bu modül saf veri ve matematik motorudur; hiçbir kilit (lock) veya dış bağımlılık taşımaz.

use std::collections::HashMap;

// =============================================================================
// 1. TEMEL BORSA VE HASSASİYET MATEMATİĞİ (FINANCIAL COMPLIANCE)
// =============================================================================

/// Floating point hassasiyet kaybını engellemek için borsa standartlarında yuvarlama.
/// Kripto piyasası için genellikle 8 hane yeterlidir.
pub fn round_to_precision(val: f64, step: f64) -> f64 {
    if step <= 0.0 { return val; }
    (val / step).round() * step
}

/// Brüt PnL Hesabı (Hassasiyet Korumalı)
pub fn calculate_pnl(entry: f64, current: f64, qty: f64, is_long: bool) -> f64 {
    let diff = if is_long { current - entry } else { entry - current };
    // Hassas yuvarlama: 4 hane (USD/USDT pariteleri için ideal)
    (diff * qty * 10000.0).round() / 10000.0
}

/// ROE (Return on Equity) - Gerçek Sermaye Verimliliği
/// Kaldıraç etkisini dahil ederek marjin üzerinden yüzde hesaplar.
pub fn calculate_roe(entry: f64, current: f64, leverage: f64, is_long: bool) -> f64 {
    if entry <= 0.0 { return 0.0; }
    let price_diff_pct = (current - entry) / entry;
    let direction_mult = if is_long { 1.0 } else { -1.0 };
    
    // Modern: ROE = Fiyat_Değişimi * Yön * Kaldıraç
    price_diff_pct * direction_mult * leverage * 100.0
}

/// Kâr Faktörü (Profit Factor) - Sıfıra Bölme Korumalı
pub fn safe_profit_factor(gross_win: f64, gross_loss: f64) -> f64 {
    let loss_abs = gross_loss.abs();
    if loss_abs < 0.00000001 { // f64::EPSILON yerine daha güvenli bir finansal eşik
        if gross_win > 0.0 { 999.99 } else { 0.0 } // Sonsuz yerine okunabilir tavan
    } else {
        gross_win / loss_abs
    }
}

/// Sharpe Ratio - Risk-Getiri Dengesi (Basit Sürüm)
pub fn calculate_sharpe(avg_return: f64, std_dev: f64) -> f64 {
    if std_dev < 0.00000001 { return 0.0; }
    avg_return / std_dev
}

/// Win Rate - Yüzdesel Kazanma Oranı
pub fn safe_win_rate(wins: usize, total: usize) -> f64 {
    if total == 0 { return 0.0; }
    (wins as f64 / total as f64) * 100.0
}

// =============================================================================
// 2. ADVANCED SCORING VE COGNITIVE REWARDS (AI/ML ENGINE FUEL)
// =============================================================================

/// Srivastava Modernize Skorlama Fonksiyonu
/// Geleneksel skorlamayı, Drawdown ve İşlem Sayısı cezalarıyla harmanlar.
pub fn calculate_advanced_score(
    win_rate: f64,
    profit_factor: f64,
    sharpe_ratio: f64,
    max_dd: f64,
    trade_count: usize,
) -> f64 {
    // 1. Temel Barikatlar (Diskalifiye)
    if trade_count < 3 || profit_factor < 1.0 || max_dd > 40.0 {
        return 0.0;
    }

    // 2. Normalizasyon
    let win_n = (win_rate / 100.0).clamp(0.0, 1.0);
    let pf_n = (profit_factor - 1.0).clamp(0.0, 4.0) / 4.0; // 5 PF ve üstü tam puan
    let sr_n = sharpe_ratio.clamp(0.0, 3.0) / 3.0;

    // 3. Drawdown Cezası (Exponential Decay)
    // DD %10'dan sonra skoru hızla aşağı çeker
    let dd_penalty = (1.0 - (max_dd / 40.0).powi(2)).clamp(0.0, 1.0);

    // 4. İşlem Sayısı Güven Çarpanı (Doygunluk)
    // 30 işlemden sonra güven tam puan (1.0) olur
    let reliability_weight = (trade_count as f64 / 30.0).clamp(0.1, 1.0);

    // 5. Ağırlıklı Sentez
    let raw_score = (win_n * 0.25) + (pf_n * 0.40) + (sr_n * 0.25) + (dd_penalty * 0.10);
    
    (raw_score * reliability_weight * 100.0).round() / 100.0
}

/// Sinyal Güven Skoru: WR, PF ve Anlık Volatiliteyi harmanlar
pub fn calculate_signal_confidence(wr: f64, pf: f64, volatility: f64) -> f64 {
    if pf < 1.0 { return 0.0; }
    
    let base = (wr / 100.0) * 0.4 + (pf / 5.0).clamp(0.0, 1.0) * 0.4;
    let vol_penalty = if volatility > 0.05 { 0.8 } else { 1.0 }; // Aşırı volatilite cezası
    
    (base * vol_penalty).clamp(0.0, 1.0)
}

/// Q-Learning Ödül Skoru: İşleminin kalitesini ve zaman/risk verimliliğini ölçer
pub fn calculate_trade_reward(pnl_pct: f64, hold_time_mins: u64, max_favorable_excursion: f64) -> f64 {
    let mut reward = pnl_pct;
    
    // Verimlilik: Eğer işlem çok uzun sürdüyse ödülü biraz kır (Sermaye maliyeti cezası)
    if hold_time_mins > 240 { reward *= 0.9; }
    
    // Kalite: Eğer fiyat hedefimize gitmeden önce çok fazla ters yöne saptıysa (MFE) 
    // riskli bir işlemdir, ödülü azalt.
    if max_favorable_excursion < pnl_pct.abs() * 0.5 { reward *= 0.8; }
    
    reward
}

// =============================================================================
// 3. İLERİ İSTATİSTİK VE RİSK ANALİTİĞİ (Calculations/Math Garnizonundan Nakledilenler)
// =============================================================================

pub struct Statistics;
impl Statistics {
    /// Median: Sıralama maliyetini düşürmek için in-place olmayan seçim (O(n log n))
    pub fn median(values: &[f64]) -> f64 {
        if values.is_empty() { return 0.0; }
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = sorted.len() / 2;
        if sorted.len() % 2 == 0 { (sorted[mid - 1] + sorted[mid]) / 2.0 } else { sorted[mid] }
    }

    /// Mode: Frekans haritası ile bit tabanlı tam f64 mod hesabı (O(n))
    pub fn mode(values: &[f64]) -> f64 {
        let mut counts = HashMap::new();
        for &val in values {
            let count = counts.entry(val.to_bits()).or_insert(0);
            *count += 1;
        }
        counts.into_iter()
            .max_by_key(|&(_, count)| count)
            .map(|(bits, _)| f64::from_bits(bits))
            .unwrap_or(0.0)
    }
}

pub struct RiskMetrics;
impl RiskMetrics {
    /// Sharpe Ratio: Risk-adjusted kümülatif getiri serisi hesabı (Sıfıra bölünme korumalı)
    pub fn sharpe_ratio(returns: &[f64], risk_free_rate: f64) -> f64 {
        let n = returns.len();
        if n < 2 { return 0.0; }
        let mean = returns.iter().sum::<f64>() / n as f64;
        let var = returns.iter().map(|&r| (r - mean).powi(2)).sum::<f64>() / n as f64;
        let std_dev = var.sqrt();
        if std_dev < f64::EPSILON { 0.0 } else { (mean - risk_free_rate) / std_dev }
    }

    /// Max Drawdown: Peak-to-Trough zirve-dip düşüş yüzdesi (%) - NaN Korumalı
    pub fn max_drawdown(prices: &[f64]) -> f64 {
        if prices.is_empty() { return 0.0; }
        let mut max_price: f64 = prices[0];
        let mut mdd: f64 = 0.0; 

        for &p in prices {
            if p > max_price { max_price = p; }
            let dd = (p - max_price) / (max_price + f64::EPSILON);
            mdd = f64::min(mdd, dd);
        }
        mdd * 100.0
    }
}

pub struct Correlation;
impl Correlation {
    /// Pearson: İki bağımsız sembol serisi arasındaki doğrusal ilişki (O(n))
    pub fn pearson(x: &[f64], y: &[f64]) -> f64 {
        if x.len() != y.len() || x.is_empty() { return 0.0; }
        let n = x.len() as f64;
        let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
        let (mut num, mut sx, mut sy) = (0.0, 0.0, 0.0);
        for (&xi, &yi) in x.iter().zip(y) {
            let (dx, dy) = (xi - mx, yi - my);
            num += dx * dy;
            sx += dx.powi(2);
            sy += dy.powi(2);
        }
        let den = (sx * sy).sqrt();
        if den < f64::EPSILON { 0.0 } else { num / den }
    }
}
