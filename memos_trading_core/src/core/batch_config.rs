// batch_fetch_config.rs
// Veri indirme ve ağ operasyonları için optimize edilmiş konfigürasyon

/// Paralel istek ve yeniden deneme mekanizması için yapılandırma
#[derive(Debug, Clone, Copy)] // Hafif bir yapı olduğu için Copy eklendi
pub struct BatchFetchConfig {
    /// Aynı anda yapılacak maksimum ağ isteği sayısı
    pub concurrency_limit: usize,
    /// Başarısız denemeler arası bekleme süresi (ms)
    pub retry_wait_ms: u64,
    /// Bir istek hata aldığında yapılacak maksimum deneme sayısı
    pub max_retries: usize,
    /// İsteklerin zaman aşımı süresi (opsiyonel ama pipeline sağlığı için kritik)
    pub timeout_sec: u64,
}

impl Default for BatchFetchConfig {
    fn default() -> Self {
        Self {
            concurrency_limit: 4,
            retry_wait_ms: 1200,
            max_retries: 5,
            timeout_sec: 30, // Varsayılan 30 saniye timeout
        }
    }
}

impl BatchFetchConfig {
    /// Akıcı (fluent) arayüz için hızlı yapılandırıcılar
    pub fn with_concurrency(mut self, limit: usize) -> Self {
        self.concurrency_limit = limit;
        self
    }

    pub fn with_retries(mut self, count: usize) -> Self {
        self.max_retries = count;
        self
    }

    /// Toplam bekleme süresini hesaplayan yardımcı metot (Pipeline planlaması için)
    pub fn total_max_wait_ms(&self) -> u64 {
        self.retry_wait_ms * self.max_retries as u64
    }
}
