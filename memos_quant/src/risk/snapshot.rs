//! `RiskSnapshot` — risk zincirinin motor durumundan bağımsızlaştırılması.
//!
//! NEDEN: risk filtreleri eskiden `&MissionControl` alıyordu; o tip motorun tam
//! durum snapshot'ı olduğu için risk katmanı çekirdeğe çivilenmişti. Ölçtük: filtreler
//! snapshot'tan yalnız BEŞ salt-okunur değer okuyor. Bu trait o beşini sözleşmeye
//! çevirir → risk zinciri `memos_quant`'a taşınabildi ve her dikey dilim kendi risk
//! filtresini aynı sözleşmeyle ekleyebilir (`MarketRuntime::risk_filters()`).
//!
//! Çekirdek `impl RiskSnapshot for MissionControl` ile bağlanır; okunan alanlar
//! birebir aynı → davranış nötr.
pub trait RiskSnapshot {
    /// Anlık toplam özkaynak (`finance.total_equity`).
    fn total_equity(&self) -> f64;
    /// Gün/dönem başı sermaye (`finance.starting_capital`).
    fn starting_capital(&self) -> f64;
    /// Zirve özkaynak (`charts.peak_equity`) — drawdown tabanı.
    fn peak_equity(&self) -> f64;
    /// Kapanmış işlemlerin mutlak PnL serisi (`trade_history[].pnl`).
    fn trade_pnls(&self) -> Vec<f64>;
    /// Kapanmış işlemlerin oransal getiri serisi (`trade_history[].pnl_pct / 100`).
    fn trade_returns(&self) -> Vec<f64>;
}
