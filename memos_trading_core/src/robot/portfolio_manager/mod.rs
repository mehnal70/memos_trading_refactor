// Srivastava ATP Mimarisi - Portfolio Manager
//
// Multi-position tracking, PnL hesaplama, drawdown monitoring
// Portföy durumu ve risk metrikleri
// Extended dengan: trailing stop, scale-in/out, partial fills

pub mod types;
pub mod manager;
pub mod dynamic_position;

pub use types::*;
pub use manager::*;
pub use dynamic_position::*;

#[cfg(test)]
mod tests {
    use super::*;
    

    #[test]
    fn test_portfolio_creation() {
        let portfolio = PortfolioManager::new(10000.0);
        assert_eq!(portfolio.total_capital(), 10000.0);
        assert_eq!(portfolio.positions_count(), 0);
    }
}
