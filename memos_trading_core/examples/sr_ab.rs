// sr_ab — SR (destek/direnç) giriş filtresi A/B ölçümü.
//
// Soru: SR'ı KARARA bağlamak ([[project_sr_display_only]]) net edge KATAR mı? "Dirence alma, desteğe
// satma" filtresini live_path backtest yoluna ek-kapı olarak koyup baseline (filtresiz) ile kıyaslar.
// SR filtresi GİRİŞ SAYISINI düşürür (bazı adayları eler) → soru kalite (işlem-başı %) yeterince artıyor
// mu, Σpnl% düşmüyor mu. SADECE net pozitifse canlıya bağlanır (autonomy-first + verify-before-commit).
//
// Kullanım:
//   cargo run --release --example sr_ab
// Env: DB_PATH (data/trader.db), TRADE_MARKET (futures), SR_AB_INTERVAL (1h), SR_AB_LIMIT (5000),
//   SR_AB_SYMBOLS (csv; boş→15-majör default), SR_AB_BANDS (csv %; default 0.3,0.5,1.0),
//   SR_AB_MIN_STRENGTH (1.0), SR_AB_EDGE_MIN (0.20).

use memos_trading_core::robot::backtester::live_path::{run, LivePathConfig, Sizing};
use memos_trading_core::persistence::reader::read_candles_market;

fn env(k: &str, d: &str) -> String { std::env::var(k).ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| d.into()) }
fn env_f64(k: &str, d: f64) -> f64 { std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(d) }
fn env_usize(k: &str, d: usize) -> usize { std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(d) }

#[derive(Default, Clone)]
struct Agg { trades: usize, wins: f64, sum_pnl_pct: f64, symbols: usize }
impl Agg {
    fn add(&mut self, r: &memos_trading_core::robot::backtester::live_path::LivePathResult) {
        self.trades += r.total_trades;
        self.wins += r.win_rate * r.total_trades as f64;
        self.sum_pnl_pct += r.sum_trade_pnl_pct;
        if r.total_trades > 0 { self.symbols += 1; }
    }
    fn per_trade(&self) -> f64 { if self.trades > 0 { self.sum_pnl_pct / self.trades as f64 } else { 0.0 } }
    // win_rate LivePathResult'ta zaten yüzde (0-100); wins = Σ(win_rate·trades) → ortalama = wins/trades.
    fn win_pct(&self) -> f64 { if self.trades > 0 { self.wins / self.trades as f64 } else { 0.0 } }
    fn line(&self, tag: &str) -> String {
        format!("{:<24} işlem={:>5} kazanç%={:>5.1} Σpnl%={:>9.2} işlem-başı%={:>7.4} ({} sembol)",
            tag, self.trades, self.win_pct(), self.sum_pnl_pct, self.per_trade(), self.symbols)
    }
}

fn main() {
    let db = env("DB_PATH", "data/trader.db");
    let market = env("TRADE_MARKET", "futures");
    let interval = env("SR_AB_INTERVAL", "1h");
    let limit = env_usize("SR_AB_LIMIT", 5000);
    let min_strength = env_f64("SR_AB_MIN_STRENGTH", 1.0);
    let edge_min = env_f64("SR_AB_EDGE_MIN", 0.20);
    let bands: Vec<f64> = env("SR_AB_BANDS", "0.3,0.5,1.0")
        .split(',').filter_map(|s| s.trim().parse().ok()).collect();
    let symbols: Vec<String> = {
        let s = env("SR_AB_SYMBOLS", "");
        if s.is_empty() {
            ["BTCUSDT","ETHUSDT","BCHUSDT","XRPUSDT","TRXUSDT","ADAUSDT","BNBUSDT",
             "DOGEUSDT","SOLUSDT","UNIUSDT","AVAXUSDT","LTCUSDT","LINKUSDT","ETCUSDT","FILUSDT"]
                .iter().map(|x| x.to_string()).collect()
        } else {
            s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
        }
    };

    let base_cfg = LivePathConfig {
        interval: interval.clone(),
        sizing: Sizing::NotionalPct(0.10),
        edge_reverse_penalty: 1.0 - edge_min.min(1.0), // edge_min ~ eşik; reverse penalty ile uyumlu kalsın
        ..Default::default()
    };

    println!("🔬 SR giriş filtresi A/B · market={} interval={} · {} sembol · edge_min={} min_strength={}",
        market, interval, symbols.len(), edge_min, min_strength);
    println!("   (filtre: dirence yakın long / desteğe yakın short ELENİR; band = fiyatın %'i)\n");

    // Mumları bir kez yükle (her kol aynı veride).
    let mut data: Vec<(String, Vec<_>)> = Vec::new();
    for sym in &symbols {
        match read_candles_market(&db, sym, &interval, &market, limit) {
            Ok(c) if c.len() >= 250 => data.push((sym.clone(), c)),
            _ => {}
        }
    }
    if data.is_empty() {
        eprintln!("❌ Yeterli mum yok (interval={} market={}). DB'de bu seri var mı?", interval, market);
        std::process::exit(2);
    }

    // BASELINE (filtre kapalı).
    let mut base = Agg::default();
    for (_s, c) in &data { base.add(&run(c, &base_cfg)); }
    println!("{}", base.line("BASELINE (filtresiz)"));

    // Her band için SR-filtreli kol.
    let mut best: Option<(f64, f64)> = None; // (band, Σpnl%)
    for &band in &bands {
        let mut cfg = base_cfg.clone();
        cfg.sr_filter_band_pct = Some(band);
        cfg.sr_min_strength = min_strength;
        let mut arm = Agg::default();
        for (_s, c) in &data { arm.add(&run(c, &cfg)); }
        let d_sum = arm.sum_pnl_pct - base.sum_pnl_pct;
        let d_pt = arm.per_trade() - base.per_trade();
        println!("{}  Δσ={:+.2} Δişlem-başı%={:+.4}", arm.line(&format!("SR band=%{:.2}", band)), d_sum, d_pt);
        if best.map(|(_, s)| arm.sum_pnl_pct > s).unwrap_or(true) { best = Some((band, arm.sum_pnl_pct)); }
    }

    // Verdikt: SR herhangi bir bandda Σpnl%'yi VE işlem-başı kaliteyi iyileştiriyor mu.
    println!("\n📊 VERDİKT:");
    if let Some((bb, bs)) = best {
        let improves_total = bs > base.sum_pnl_pct;
        let mut bcfg = base_cfg.clone();
        bcfg.sr_filter_band_pct = Some(bb); bcfg.sr_min_strength = min_strength;
        let mut barm = Agg::default();
        for (_s, c) in &data { barm.add(&run(c, &bcfg)); }
        let improves_quality = barm.per_trade() > base.per_trade();
        println!("   En iyi band=%{:.2} → Σpnl% {:.2} (baseline {:.2}), işlem-başı% {:.4} (baseline {:.4})",
            bb, bs, base.sum_pnl_pct, barm.per_trade(), base.per_trade());
        if improves_total && improves_quality {
            println!("   ✅ SR filtresi hem toplam hem kaliteyi İYİLEŞTİRİYOR → canlıya bağlamaya değer (WF ile teyit et).");
        } else if improves_quality && !improves_total {
            println!("   ⚠️ Kalite↑ ama toplam↓ (çok iyi işlem de eleniyor) → marjinal; net kazanç YOK.");
        } else {
            println!("   ❌ SR filtresi net kazanç KATMIYOR → SR salt-gösterim kalsın ([[project_sr_display_only]]).");
        }
    }
}
