// src/robot/data_pipeline/orchestrator.rs - Veri Akış Hattı Orkestratörü
// Srivastava ATP - İşlevsel Çarklar Odası

use crate::prelude::*;
use super::cache::CandleCache;
use super::synth::CandleSynth;
use super::normalizer::DataNormalizer;
use super::validator::DataValidator;

/// DataPipeline - Sistemin Veri Orkestratörü
pub struct DataPipeline<'a> {
    pub cache: CandleCache,
    pub synth: CandleSynth<'a>,
}

impl<'a> DataPipeline<'a> {
    pub fn new(
        max_size: usize, 
        symbol: &str, 
        callback: Box<dyn Fn(&Candle) + Send + Sync + 'a>
    ) -> Self {
        Self {
            cache: CandleCache::new(max_size),
            synth: CandleSynth::new(symbol, callback),
        }
    }

    /// Senin belirttiğin HTF hiyerarşisi — Merkezi Otorite
    pub fn get_htf_interval(interval: &str) -> &'static str {
        match interval {
            "1m" | "5m"   => "1h",
            "15m" | "30m" => "4h",
            "1h"          => "4h",
            "4h" | "1d"   => "1d",
            _             => "1d",
        }
    }

    /// Tam Pipeline Akışı: Temizle -> Normalize Et -> Sentezle -> Önbelleğe Al
    /// Bu metod robotic_loop'taki veri işleme yükünü tamamen üzerine alır.
    pub fn process_tick(&mut self, incoming_1m: Vec<Candle>) -> Vec<Candle> {
        if incoming_1m.is_empty() { return Vec::new(); }

        // 1. Adım: Processor Mantığı (Temizle + Normalize + Valide)
        // DataNormalizer::process_and_standardize hem temizliği hem normalizasyonu 
        // otonom birleştirir (Spike koruması dahil).
        let refined_1m = DataNormalizer::process_and_standardize(incoming_1m);

        // 2. Adım: Sentezleme ve Önbellek (Pipeline Mantığı)
        let mut newly_formed = Vec::new();
        for candle in refined_1m {
            // Validation: Sadece OHLC kurallarına uyan mumları içeri al
            if DataValidator::validate_ohlc(&candle).is_ok() {
                self.cache.push_bulk(vec![candle.clone()]);
                newly_formed.extend(self.synth.push_1m(&candle));
            }
        }

        newly_formed
    }

    /// Sadece normalizasyon veya temizlik gerekirse statik erişim sağlar
    pub fn clean_only(candles: &mut Vec<Candle>) -> Vec<Candle> {
        DataNormalizer::process_and_standardize(candles.clone())
    }
}
