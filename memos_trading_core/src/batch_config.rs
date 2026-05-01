// Otomatik veri indirme ve strateji fonksiyonları için konfigürasyon yapısı
// Bu yapı, fonksiyonlara dışarıdan parametre ile kolayca aktarılabilir.

#[derive(Debug, Clone)]
pub struct BatchFetchConfig {
    pub concurrency_limit: usize, // Paralel istek sayısı
    pub wait_ms: u64,            // Her deneme arası bekleme süresi (ms)
    pub max_retries: usize,      // Maksimum deneme sayısı
}

impl Default for BatchFetchConfig {
    fn default() -> Self {
        Self {
            concurrency_limit: 4,
            wait_ms: 1200,
            max_retries: 5,
        }
    }
}
