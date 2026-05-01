// Srivastava ATP Mimarisi - Advanced Risk Metrics
//
// Sharpe Ratio, Sortino, Kelly Criterion, Value at Risk (VaR)
// Profesyonel risk yönetimi metrikleri

pub mod metrics;
pub mod kelly;
pub mod var;

pub use metrics::*;
pub use kelly::*;
pub use var::*;

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sharpe_ratio_calculation() {
        let returns = vec![0.01, 0.02, 0.015, 0.005, 0.03];
        let rf_rate = 0.001;
        
        let sharpe = SharpeCalculator::calculate(&returns, rf_rate);
        assert!(sharpe > 0.0);
    }
}
