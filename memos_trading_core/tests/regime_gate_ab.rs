// Adaptif rejim Volatile-kapısı A/B harness'i — [[project_autonomy_backlog]] #1.
//
// Soru: canlı `Volatile → IDLE_PROTECT` (giriş bastırma) kararını SABİT eşikle
// (ATR%>7) mu yoksa sembolün KENDİ ATR% dağılımının persentiliyle mi (adaptif)
// vermeli? Sabit-strateji backtest'inde rejimin ölçülebilir tek etkisi budur
// (strateji-seçimi ve regime_overrides bu yolda devrede değil).
//
// Karşılaştırma kolları (BacktestConfig.regime_gate):
//   Off       → kapı yok (mevcut PRODUCTION baseline, rejim girişi bastırmıyor).
//   Absolute  → ATR%>7 Volatile barda giriş yok (canlı IDLE aynası, sabit eşik).
//   Adaptive  → Volatile sınırı sembolün kendi rolling ATR% persentili.
//
// VERİ-BAĞIMLI → #[ignore] (test hijyeni: ham mum DB'si CI'da yok). Elle koşum:
//   cargo test --test regime_gate_ab -- --ignored --nocapture
// DB yolu env `MEMOS_AB_DB` ile ezilebilir; yoksa aday yollar denenir.

use memos_trading_core::persistence::reader::read_candles;
use memos_trading_core::robot::backtester::{BacktestConfig, Backtester, RegimeGate};

/// İlk var olan aday DB yolu (CWD = crate kökü `memos_trading_core/`).
fn resolve_db() -> Option<String> {
    if let Ok(p) = std::env::var("MEMOS_AB_DB") {
        return if std::path::Path::new(&p).exists() { Some(p) } else { None };
    }
    ["../data/trader.db", "data/trader.db", "../data/memos_trading.db"]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
        .map(String::from)
}

#[derive(Default, Clone, Copy)]
struct Agg {
    runs: u32,
    trades: usize,
    wins: f64,
    sum_pnl_pct: f64, // nominal≈sermaye boyutlamasıyla ≈ getiri %
    sum_pf: f64,
    sum_sharpe: f64,
    pf_runs: u32,
}

impl Agg {
    fn add(&mut self, r: &memos_trading_core::robot::backtester::BacktestResult) {
        self.runs += 1;
        self.trades += r.total_trades;
        self.wins += r.win_rate / 100.0 * r.total_trades as f64;
        self.sum_pnl_pct += r.total_pnl_pct;
        self.sum_sharpe += r.sharpe_ratio;
        if r.total_trades > 0 {
            self.sum_pf += r.profit_factor.min(50.0); // 999 tavanını kıs (ortalama bozulmasın)
            self.pf_runs += 1;
        }
    }
    fn win_rate(&self) -> f64 {
        if self.trades == 0 { 0.0 } else { self.wins / self.trades as f64 * 100.0 }
    }
    fn report(&self, label: &str) {
        eprintln!(
            "{label:<10} | koşum {:>3} | işlem {:>6} | kazanma %{:>5.1} | Σpnl% {:>9.1} | ort.pnl%/koşum {:>7.2} | ort.PF {:>5.2} | ort.Sharpe {:>6.3}",
            self.runs,
            self.trades,
            self.win_rate(),
            self.sum_pnl_pct,
            if self.runs > 0 { self.sum_pnl_pct / self.runs as f64 } else { 0.0 },
            if self.pf_runs > 0 { self.sum_pf / self.pf_runs as f64 } else { 0.0 },
            if self.runs > 0 { self.sum_sharpe / self.runs as f64 } else { 0.0 },
        );
    }
}

fn cfg_for(symbol: &str, strat: &str, ref_price: f64, gate: RegimeGate) -> BacktestConfig {
    let initial = 10_000.0;
    BacktestConfig {
        symbol: symbol.to_string(),
        interval: "1h".to_string(),
        initial_balance: initial,
        // Nominal ≈ sermaye → total_pnl_pct semboller arası kıyaslanabilir.
        max_position_size: if ref_price > 0.0 { initial / ref_price } else { 1.0 },
        take_profit_pct: 4.0,
        stop_loss_pct: 2.0,
        strategy_name: strat.to_string(),
        commission_pct: 0.0004,
        breakeven_at_rr: Some(1.0),
        atr_trail_mult: Some(2.0),
        regime_gate: gate,
        ..Default::default()
    }
}

#[test]
#[ignore = "veri-bağımlı (ham mum DB'si); elle: cargo test --test regime_gate_ab -- --ignored --nocapture"]
fn regime_volatile_gate_ab() {
    let Some(db) = resolve_db() else {
        eprintln!("⏭  DB bulunamadı (MEMOS_AB_DB ya da ../data/trader.db) — A/B atlandı.");
        return;
    };
    eprintln!("📂 DB: {db}");

    let symbols = [
        "BTCUSDT", "ETHUSDT", "BNBUSDT", "ADAUSDT",
        "XRPUSDT", "TRXUSDT", "DOGEUSDT", "ZECUSDT",
    ];
    let strategies = ["SUPERTREND", "EMA_CROSSOVER", "MACD", "RSI", "BB"];
    let limit = 6000; // ~8 ay 1h

    let mut off = Agg::default();
    let mut abs_ = Agg::default();
    let mut ad80 = Agg::default();
    let mut ad90 = Agg::default();

    let mut used_symbols = 0;
    for sym in symbols {
        let candles = match read_candles(&db, sym, "1h", limit) {
            Ok(c) if c.len() >= 200 => c,
            _ => { eprintln!("   {sym}: yetersiz mum, atlandı"); continue; }
        };
        used_symbols += 1;
        let ref_price = candles.iter().map(|c| c.close).sum::<f64>() / candles.len() as f64;
        for strat in strategies {
            let run = |gate| Backtester::new(cfg_for(sym, strat, ref_price, gate)).run(&candles);
            if let Ok(r) = run(RegimeGate::Off) { off.add(&r); }
            if let Ok(r) = run(RegimeGate::Absolute) { abs_.add(&r); }
            if let Ok(r) = run(RegimeGate::Adaptive { pctl: 0.80 }) { ad80.add(&r); }
            if let Ok(r) = run(RegimeGate::Adaptive { pctl: 0.90 }) { ad90.add(&r); }
        }
    }

    eprintln!("\n=== Rejim Volatile-Kapısı A/B ({used_symbols} sembol × {} strateji, 1h, {limit} mum) ===", strategies.len());
    off.report("Off");
    abs_.report("Absolute");
    ad80.report("Adaptive80");
    ad90.report("Adaptive90");
    eprintln!("\nKarar kriteri: Adaptive, Absolute'a göre kazanma%/ort.pnl%'i bozmadan");
    eprintln!("artırıyorsa benimse; aksi halde sabit eşik (Off/Absolute) kalsın.\n");

    assert!(used_symbols > 0, "hiç sembol işlenemedi — DB içeriğini kontrol et");
}
