//! `VenueRegistry` — aktif venue'ların kayıt defteri + sembol→venue yönlendirme.
//!
//! Profiller hangi venue'ların aktif olduğunu seçer; motor sembol başına doğru adaptörü
//! buradan ister (`Exchange::classify` tek-kaynak). Kayıtlı olmayan sembol varsayılan
//! borsaya düşer (geriye-uyum: tek-Binance kurulumunda her şey Binance'e gider).

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

static MT5_ENV_LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();

fn mt5_env_lock() -> &'static std::sync::Mutex<()> {
    MT5_ENV_LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn load_mt5_env_from_files(start_dir: &std::path::Path) {
    let mut current = Some(start_dir);
    while let Some(dir) = current {
        let env_path = dir.join(".env");
        if env_path.exists() {
            if let Ok(contents) = std::fs::read_to_string(&env_path) {
                for line in contents.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    if let Some((key, value)) = trimmed.split_once('=') {
                        let key = key.trim();
                        let value = value.trim().trim_matches('"').trim_matches('\'');
                        if key.starts_with("MT5_") || key.starts_with("BINANCE_") {
                            if std::env::var(key).is_err() {
                                std::env::set_var(key, value);
                            }
                        }
                    }
                }
            }
        }
        current = dir.parent();
    }
}

fn ensure_mt5_env_loaded() {
    if std::env::var("MT5_ENV_LOADED").is_ok() {
        return;
    }

    let cwd = std::env::current_dir().ok();
    if let Some(mut dir) = cwd {
        loop {
            load_mt5_env_from_files(&dir);
            if dir.parent().is_none() {
                break;
            }
            dir.pop();
        }
    }

    if std::env::var("MT5_LOGIN").is_ok()
        || std::env::var("MT5_PASSWORD").is_ok()
        || std::env::var("MT5_SERVER").is_ok()
    {
        std::env::set_var("MT5_ENV_LOADED", "1");
    }
}

fn mt5_auto_repair_action() -> Option<&'static str> {
    if env_truthy("MT5_AUTO_REPAIR") {
        Some("all")
    } else if env_truthy("MT5_AUTO_LAUNCH") {
        Some("launch")
    } else if env_truthy("MT5_AUTO_SETUP") {
        Some("install")
    } else {
        None
    }
}

fn mt5_auto_retry_count() -> usize {
    std::env::var("MT5_AUTO_RETRY_COUNT")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(3)
}

fn mt5_auto_retry_delay_secs() -> u64 {
    std::env::var("MT5_AUTO_RETRY_DELAY_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(5)
}

fn mt5_credentials_from_env() -> Option<(String, String, String)> {
    ensure_mt5_env_loaded();
    let login = std::env::var("MT5_LOGIN")
        .ok()
        .filter(|v| !v.trim().is_empty())?;
    let password = std::env::var("MT5_PASSWORD")
        .ok()
        .filter(|v| !v.trim().is_empty())?;
    let server = std::env::var("MT5_SERVER")
        .ok()
        .filter(|v| !v.trim().is_empty())?;
    Some((login, password, server))
}

pub fn try_autostart_mt5_bridge() -> bool {
    ensure_mt5_env_loaded();
    let Some(action) = mt5_auto_repair_action() else {
        return false;
    };

    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let script = repo_root.join("scripts/mt5/setup_and_run.sh");
    if !script.exists() {
        log::warn!(target: "MT5_AUTOSTART", "MT5 otomatik kurulum betiği bulunamadı: {}", script.display());
        return false;
    }

    log::info!(target: "MT5_AUTOSTART", "MT5 otomatik hazırlık başlatılıyor: bash {} {}", script.display(), action);
    let mut cmd = Command::new("bash");
    cmd.arg(&script).arg(action).current_dir(repo_root);
    if let Some((login, password, server)) = mt5_credentials_from_env() {
        cmd.env("MT5_LOGIN", login)
            .env("MT5_PASSWORD", password)
            .env("MT5_SERVER", server)
            .env("MT5_AUTO_LOGIN", "1");
    }

    let output = cmd.output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !stdout.trim().is_empty() {
                log::info!(target: "MT5_AUTOSTART", "{}", stdout.trim());
            }
            if !stderr.trim().is_empty() {
                log::info!(target: "MT5_AUTOSTART", "{}", stderr.trim());
            }
            true
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            log::warn!(target: "MT5_AUTOSTART", "MT5 otomatik hazırlık başarısız ({}): {}", action, stderr.trim());
            false
        }
        Err(err) => {
            log::warn!(target: "MT5_AUTOSTART", "MT5 otomatik hazırlık çalıştırılamadı: {err}");
            false
        }
    }
}

pub fn try_autostart_mt5_bridge_with_retries() -> bool {
    let attempts = mt5_auto_retry_count();
    let delay = mt5_auto_retry_delay_secs();
    for attempt in 1..=attempts {
        if try_autostart_mt5_bridge() {
            return true;
        }
        if attempt < attempts {
            log::info!(target: "MT5_AUTOSTART", "MT5 otomatik hazırlık tekrar denenecek ({}/{}) {} sn sonra", attempt, attempts, delay);
            std::thread::sleep(Duration::from_secs(delay));
        }
    }
    false
}

use crate::core::types::{Exchange, VenueSpec};
use crate::robot::venue::adapter::VenueAdapter;
use crate::robot::venue::BistVenue;
use memos_market_binance::BinanceFuturesExecutor;
use memos_market_binance::BinanceVenueFactory;
use memos_market_ibkr::IbkrVenueFactory;
use memos_market_mt5::Mt5VenueFactory;
use memos_venue_api::{Candle, CandleFallback, VenueFactory, VenueProfile};

pub struct VenueRegistry {
    /// Tam kimlik ile anahtarla: aynı exchange'in spot/futures/cfd örnekleri
    /// birbirinin üzerine yazılmamalıdır.
    venues: HashMap<VenueSpec, Arc<dyn VenueAdapter>>,
    /// Eski sembol formatı market/profil içermediği için exchange başına seçili varsayılan.
    exchange_defaults: HashMap<Exchange, VenueSpec>,
    /// Profil açıkça seçildiğinde kullanılacak varsayılan venue.
    profile_defaults: HashMap<(Exchange, VenueProfile), VenueSpec>,
    default_spec: VenueSpec,
}

struct BuiltinVenueFactory {
    binance_executor: Option<Arc<BinanceFuturesExecutor>>,
}

struct SqliteCandleFallback {
    db_path: String,
}

impl SqliteCandleFallback {
    fn from_env() -> Self {
        Self {
            db_path: std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".to_string()),
        }
    }
}

impl CandleFallback for SqliteCandleFallback {
    fn read_candles(
        &self,
        symbol: &str,
        interval: &str,
        exchange: &str,
        market: &str,
        limit: usize,
    ) -> memos_venue_api::Result<Vec<Candle>> {
        crate::persistence::reader::read_candles_for_venue(
            &self.db_path,
            symbol,
            interval,
            exchange,
            market,
            limit,
        )
        .map_err(|error| error.to_string().into())
    }

    fn read_candles_for_profile(
        &self,
        symbol: &str,
        interval: &str,
        exchange: &str,
        market: &str,
        profile: &str,
        limit: usize,
    ) -> memos_venue_api::Result<Vec<Candle>> {
        crate::persistence::reader::read_candles_for_venue_profile(
            &self.db_path,
            symbol,
            interval,
            exchange,
            market,
            profile,
            limit,
        )
        .map_err(|error| error.to_string().into())
    }
}

impl VenueFactory for BuiltinVenueFactory {
    fn build(&self, spec: VenueSpec) -> memos_venue_api::Result<Arc<dyn VenueAdapter>> {
        match spec.exchange {
            Exchange::Binance => {
                BinanceVenueFactory::new(self.binance_executor.clone()).build(spec)
            }
            Exchange::Mt5 => {
                ensure_mt5_env_loaded();
                Mt5VenueFactory::default().build(spec)
            }
            Exchange::Ibkr => {
                let fallback = Arc::new(SqliteCandleFallback::from_env());
                let factory = IbkrVenueFactory::from_env(fallback);
                factory.build(spec)
            }
            Exchange::Bist => Ok(Arc::new(BistVenue::new(spec.profile))),
            _ => Err(format!("venue '{}' henüz desteklenmiyor", spec.token()).into()),
        }
    }
}

impl VenueRegistry {
    /// Sembolü hiçbir kayıtlı venue karşılamazsa düşülecek varsayılan borsa ile kur.
    pub fn new(default_exchange: Exchange) -> Self {
        let default_spec = VenueSpec::new(default_exchange, crate::core::types::Market::Spot);
        return Self {
            venues: HashMap::new(),
            exchange_defaults: HashMap::new(),
            profile_defaults: HashMap::new(),
            default_spec,
        };
    }

    /// Config venue-spec'lerinden registry kur (operatör seçimi → çalışan registry).
    /// Bilinen borsa (şu an yalnız Binance) için adaptör kurulur; henüz desteklenmeyen borsa
    /// loglanıp atlanır (Faz 1+ eklendikçe açılır). `binance_executor` verilirse Binance venue
    /// auth'lu (veri+yürütme), yoksa data-only (yalnız public veri). default_exchange = ilk spec.
    ///
    /// NOT: registry borsa-anahtarlı → aynı borsanın birden çok market'i (binance:spot +
    /// binance:futures) verilirse SON spec'in market'i kazanır (tek venue/borsa). Çoklu-market/
    /// borsa anahtarı Faz 1+ işi.
    pub fn from_specs(
        specs: &[VenueSpec],
        binance_executor: Option<Arc<BinanceFuturesExecutor>>,
    ) -> Self {
        let default_spec = specs
            .first()
            .copied()
            .unwrap_or_else(|| VenueSpec::new(Exchange::Binance, crate::core::types::Market::Spot));
        let mut reg = Self {
            venues: HashMap::new(),
            exchange_defaults: HashMap::new(),
            profile_defaults: HashMap::new(),
            default_spec,
        };
        let factory = BuiltinVenueFactory { binance_executor };
        reg.populate_from_factory(specs, &factory);
        return reg;
    }

    /// Harici market crate'lerinin factory'siyle registry oluştur.
    pub fn from_factory(specs: &[VenueSpec], factory: &dyn VenueFactory) -> Self {
        let default_spec = specs
            .first()
            .copied()
            .unwrap_or_else(|| VenueSpec::new(Exchange::Binance, crate::core::types::Market::Spot));
        let mut reg = Self {
            venues: HashMap::new(),
            exchange_defaults: HashMap::new(),
            profile_defaults: HashMap::new(),
            default_spec,
        };
        reg.populate_from_factory(specs, factory);
        reg
    }

    fn populate_from_factory(&mut self, specs: &[VenueSpec], factory: &dyn VenueFactory) {
        for spec in specs {
            match factory.build(*spec) {
                Ok(venue) => {
                    self.register_spec(*spec, venue);
                }
                Err(err) => {
                    log::warn!(target: "VENUE", "venue '{}' oluşturulamadı: {}", spec.token(), err);
                }
            }
        }
    }

    /// Bir venue'yu tam `(exchange, market)` kimliğiyle kaydet.
    pub fn register(&mut self, venue: Arc<dyn VenueAdapter>) -> &mut Self {
        let spec = VenueSpec::new(venue.exchange(), venue.market());
        self.register_spec(spec, venue)
    }

    /// Bir adaptörü profil bilgisiyle kaydet. Adaptörün ortak trait'i profil taşımasa da
    /// registry bu kimliği kaybetmez.
    pub fn register_spec(&mut self, spec: VenueSpec, venue: Arc<dyn VenueAdapter>) -> &mut Self {
        self.exchange_defaults.insert(spec.exchange, spec);
        self.profile_defaults
            .insert((spec.exchange, spec.profile), spec);
        self.venues.insert(spec, venue);
        return self;
    }

    /// Exchange başına varsayılan venue (geriye uyumluluk yüzeyi).
    pub fn get(&self, exchange: Exchange) -> Option<&Arc<dyn VenueAdapter>> {
        let spec = self.exchange_defaults.get(&exchange)?;
        self.venues.get(spec)
    }

    /// Tam `(exchange, market)` kimliğiyle venue.
    pub fn get_spec(&self, spec: VenueSpec) -> Option<&Arc<dyn VenueAdapter>> {
        self.venues.get(&spec)
    }

    /// Exchange + profil kimliğiyle varsayılan market venue'su.
    pub fn get_profile(
        &self,
        exchange: Exchange,
        profile: VenueProfile,
    ) -> Option<&Arc<dyn VenueAdapter>> {
        let spec = self.profile_defaults.get(&(exchange, profile))?;
        self.venues.get(spec)
    }

    /// Kayıtlı tam venue kimlikleri.
    pub fn specs(&self) -> impl Iterator<Item = VenueSpec> + '_ {
        self.venues.keys().copied()
    }

    /// Sembolü venue'suna yönlendir + ÇIPLAK sembolü döndür. Explicit routing:
    ///   * "BTCUSDT@bybit" → açık etiket: o borsa kayıtlıysa ORAYA (aynı-şekil semboller
    ///     Binance/Bybit ayrımını classify yapamaz → etiket tek çözüm).
    ///   * "BTCUSDT" (etiketsiz) → `Exchange::classify` (şekil) → kayıtlıysa o, değilse default.
    /// Dönen `&str` çıplak semboldür (etiket soyulmuş) — `fetch_candles`'a bu verilmeli.
    pub fn route<'r, 's>(
        &'r self,
        symbol: &'s str,
    ) -> Option<(&'r Arc<dyn VenueAdapter>, &'s str)> {
        let (bare, tag) = split_venue_tag(symbol);
        if let Some(explicit) = tag {
            // Açık venue etiketi başka bir borsaya sessizce düşmemeli:
            // "@mt5" kayıtlı değilse sembol IBKR/Binance'a yönlenmemeli.
            let spec = self.exchange_defaults.get(&explicit)?;
            return self.venues.get(spec).map(|v| (v, bare));
        }
        let target = Exchange::classify(bare);
        let spec = self
            .exchange_defaults
            .get(&target)
            .or_else(|| self.exchange_defaults.get(&self.default_spec.exchange))?;
        self.venues.get(spec).map(|v| (v, bare))
    }

    /// Market bilgisi biliniyorsa tam venue kimliğiyle route et.
    pub fn route_in_market<'r, 's>(
        &'r self,
        symbol: &'s str,
        market: crate::core::types::Market,
    ) -> Option<(&'r Arc<dyn VenueAdapter>, &'s str)> {
        let (bare, tag) = split_venue_tag(symbol);
        let exchange = tag.unwrap_or_else(|| Exchange::classify(bare));
        let spec = VenueSpec::new(exchange, market);
        self.venues.get(&spec).map(|v| (v, bare))
    }

    /// Sembolü venue'suna yönlendir (yalnız venue; çıplak sembol gerekmiyorsa). [`route`] kısayolu.
    pub fn for_symbol(&self, symbol: &str) -> Option<&Arc<dyn VenueAdapter>> {
        return self.route(symbol).map(|(v, _)| v);
    }

    /// Kayıtlı borsalar.
    pub fn exchanges(&self) -> impl Iterator<Item = Exchange> + '_ {
        return self.exchange_defaults.keys().copied();
    }

    pub fn is_empty(&self) -> bool {
        return self.venues.is_empty();
    }

    pub fn len(&self) -> usize {
        return self.venues.len();
    }
}

/// Sembolden açık venue etiketini ayır: "BTCUSDT@bybit" → ("BTCUSDT", Some(Bybit)).
/// '@' yoksa ya da etiket geçerli bir borsa değilse → (sembol-aynen, None). Kripto/BIST
/// sembolleri alfanümeriktir, '@' içermez → güvenli ayraç. [[project_venue_multimarket]]
pub fn split_venue_tag(symbol: &str) -> (&str, Option<Exchange>) {
    return match symbol.split_once('@') {
        Some((bare, tag)) => match Exchange::from_token(tag) {
            Some(ex) => (bare, Some(ex)),
            None => (symbol, None),
        },
        None => (symbol, None),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mt5_auto_repair_action_selects_expected_mode() {
        let _guard = mt5_env_lock().lock().unwrap();
        std::env::remove_var("MT5_AUTO_REPAIR");
        std::env::remove_var("MT5_AUTO_LAUNCH");
        std::env::remove_var("MT5_AUTO_SETUP");
        assert_eq!(mt5_auto_repair_action(), None);

        std::env::set_var("MT5_AUTO_SETUP", "1");
        assert_eq!(mt5_auto_repair_action(), Some("install"));
        std::env::remove_var("MT5_AUTO_SETUP");

        std::env::set_var("MT5_AUTO_LAUNCH", "1");
        assert_eq!(mt5_auto_repair_action(), Some("launch"));
        std::env::remove_var("MT5_AUTO_LAUNCH");

        std::env::set_var("MT5_AUTO_REPAIR", "1");
        assert_eq!(mt5_auto_repair_action(), Some("all"));
        std::env::remove_var("MT5_AUTO_REPAIR");
    }

    #[test]
    fn mt5_retry_settings_are_read_from_env() {
        let _guard = mt5_env_lock().lock().unwrap();
        std::env::set_var("MT5_AUTO_RETRY_COUNT", "4");
        std::env::set_var("MT5_AUTO_RETRY_DELAY_SECS", "7");
        assert_eq!(mt5_auto_retry_count(), 4);
        assert_eq!(mt5_auto_retry_delay_secs(), 7);
        std::env::remove_var("MT5_AUTO_RETRY_COUNT");
        std::env::remove_var("MT5_AUTO_RETRY_DELAY_SECS");
    }

    #[test]
    fn mt5_env_values_are_loaded_from_dotenv_file() {
        let _guard = mt5_env_lock().lock().unwrap();
        let temp_dir = std::env::temp_dir().join(format!("mt5-env-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::write(
            temp_dir.join(".env"),
            "MT5_LOGIN=demo-login\nMT5_PASSWORD=demo-pass\nMT5_SERVER=demo-server\n",
        )
        .unwrap();

        std::env::remove_var("MT5_LOGIN");
        std::env::remove_var("MT5_PASSWORD");
        std::env::remove_var("MT5_SERVER");

        load_mt5_env_from_files(&temp_dir);

        assert_eq!(std::env::var("MT5_LOGIN").unwrap(), "demo-login");
        assert_eq!(std::env::var("MT5_PASSWORD").unwrap(), "demo-pass");
        assert_eq!(std::env::var("MT5_SERVER").unwrap(), "demo-server");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn registry_keeps_same_exchange_markets_isolated() {
        let specs = [
            VenueSpec::new(Exchange::Binance, crate::core::types::Market::Spot),
            VenueSpec::new(Exchange::Binance, crate::core::types::Market::Futures),
        ];
        let registry = VenueRegistry::from_specs(&specs, None);

        assert_eq!(registry.len(), 2);
        assert!(registry.get_spec(specs[0]).is_some());
        assert!(registry.get_spec(specs[1]).is_some());
    }

    #[test]
    fn registry_keeps_same_market_profiles_isolated() {
        let specs = [
            VenueSpec::with_profile(
                Exchange::Binance,
                crate::core::types::Market::Futures,
                VenueProfile::Demo,
            ),
            VenueSpec::with_profile(
                Exchange::Binance,
                crate::core::types::Market::Futures,
                VenueProfile::Live,
            ),
        ];
        let registry = VenueRegistry::from_specs(&specs, None);

        assert_eq!(registry.len(), 2);
        assert!(registry.get_spec(specs[0]).is_some());
        assert!(registry.get_spec(specs[1]).is_some());
        assert!(registry
            .get_profile(Exchange::Binance, VenueProfile::Demo)
            .is_some());
        assert!(registry
            .get_profile(Exchange::Binance, VenueProfile::Live)
            .is_some());
    }

    #[test]
    fn venue_profile_token_round_trips() {
        let spec = VenueSpec::parse_token("binance:futures:demo").expect("profile token");
        assert_eq!(spec.profile, VenueProfile::Demo);
        assert_eq!(spec.token(), "binance:futures:demo");
        assert_eq!(
            VenueSpec::parse_token("binance:futures")
                .expect("legacy token")
                .profile,
            VenueProfile::Default
        );
    }
}
