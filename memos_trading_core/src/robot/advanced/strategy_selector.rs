// StrategySelector struct ve ilgili fonksiyonlar burada tanımlanmalı
use crate::types::{Candle, StrategyParams};

// Temel StrategySelector struct ve fonksiyon tanımı
pub struct StrategySelector;

impl StrategySelector {
	pub fn new() -> Self {
		StrategySelector
	}
	
	pub fn select_best(&self, _candles: &[Candle], _params: &StrategyParams) -> &'static str {
		// Örnek: her zaman "MA_CROSSOVER" döndür
		// TODO: Gerçek strateji seçimi mantığı eklenecek
		"MA_CROSSOVER"
	}
}