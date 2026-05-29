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
use memos_trading_core::robot::backtester::{BacktestConfig, Backtester, DirectionMode, RegimeGate};

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

fn cfg_for(symbol: &str, strat: &str, ref_price: f64, gate: RegimeGate, dir: DirectionMode) -> BacktestConfig {
    cfg_full(symbol, strat, ref_price, gate, dir, None)
}

fn cfg_full(
    symbol: &str, strat: &str, ref_price: f64, gate: RegimeGate, dir: DirectionMode, ob: Option<&str>,
) -> BacktestConfig {
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
        direction: dir,
        orderbook_sim: ob.map(|s| s.to_string()),
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
            let run = |gate| Backtester::new(cfg_for(sym, strat, ref_price, gate, DirectionMode::LongOnly)).run(&candles);
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

#[test]
#[ignore = "veri-bağımlı; elle: cargo test --test regime_gate_ab direction_ab -- --ignored --nocapture"]
fn direction_ab() {
    // Yapısal kaldıraç: canlı sistem long-only; stratejiler Signal::Sell de üretiyor ve
    // hesap futures → short bacağı ölçülür. Adaptif Volatile-kapısı (Adaptive90, #1'de
    // benimsendi) sabit tutulur; YALNIZ yön modu değişir → "shorting kâra geçirir mi?".
    //   LongOnly        : yalnız Buy→long (mevcut canlı davranış, baseline)
    //   BothDirections  : Sell→short (strateji ne derse, simetrik)
    //   RegimeDirectional: + rejim yönü teyidi (ters-trend giriş elenir)
    let Some(db) = resolve_db() else {
        eprintln!("⏭  DB bulunamadı — yön A/B atlandı.");
        return;
    };
    eprintln!("📂 DB: {db}");

    let symbols = [
        "BTCUSDT", "ETHUSDT", "BNBUSDT", "ADAUSDT",
        "XRPUSDT", "TRXUSDT", "DOGEUSDT", "ZECUSDT",
    ];
    let strategies = ["SUPERTREND", "EMA_CROSSOVER", "MACD", "RSI", "BB"];
    let limit = 6000;
    let gate = RegimeGate::Adaptive { pctl: 0.90 }; // #1 kazananı sabit

    let mut long_only = Agg::default();
    let mut both = Agg::default();
    let mut regime_dir = Agg::default();

    let mut used_symbols = 0;
    for sym in symbols {
        let candles = match read_candles(&db, sym, "1h", limit) {
            Ok(c) if c.len() >= 200 => c,
            _ => continue,
        };
        used_symbols += 1;
        let ref_price = candles.iter().map(|c| c.close).sum::<f64>() / candles.len() as f64;
        for strat in strategies {
            let run = |d| Backtester::new(cfg_for(sym, strat, ref_price, gate, d)).run(&candles);
            if let Ok(r) = run(DirectionMode::LongOnly) { long_only.add(&r); }
            if let Ok(r) = run(DirectionMode::BothDirections) { both.add(&r); }
            if let Ok(r) = run(DirectionMode::RegimeDirectional) { regime_dir.add(&r); }
        }
    }

    eprintln!("\n=== Yön A/B (Adaptive90 gate sabit · {used_symbols} sembol × {} strateji, 1h, {limit} mum) ===", strategies.len());
    long_only.report("LongOnly");
    both.report("Both");
    regime_dir.report("RegimeDir");
    eprintln!("\nKarar: Both/RegimeDir, LongOnly'nin Σpnl%/kazanma%/PF'ini artırıyorsa");
    eprintln!("yapısal kaldıraç gerçek → canlıya opt-in bağla; aksi halde long-only kalsın.\n");

    assert!(used_symbols > 0);
}

/// RegimeDirectional + Adaptive90 üstüne vol-adaptif katmanlar (ATR-exits / vol-target).
fn cfg_va(
    symbol: &str, strat: &str, ref_price: f64, ob: Option<&str>,
    atr: Option<(f64, f64)>, vt: Option<f64>,
) -> BacktestConfig {
    let mut c = cfg_full(symbol, strat, ref_price,
        RegimeGate::Adaptive { pctl: 0.90 }, DirectionMode::RegimeDirectional, ob);
    if let Some((sl, tp)) = atr { c.atr_sl_mult = Some(sl); c.atr_tp_mult = Some(tp); }
    c.vol_target_pct = vt;
    c
}

#[test]
#[ignore = "veri-bağımlı; elle: cargo test --test regime_gate_ab vol_adaptive -- --ignored --nocapture"]
fn vol_adaptive_ab() {
    // Kaldıraç #2+#3: RegimeDir'in zayıf/tutarsız dilimlerini (benign dönemde geri kalma)
    // ATR-relatif exit + vol-hedefli sizing toparlıyor mu? Slippage AÇIK (gerçekçi).
    //   A: RegimeDir baz (sabit %4/%2 exit, sabit qty)
    //   B: + ATR-exits (SL 1.5×ATR, TP 3.0×ATR → vol-relatif 2:1)
    //   C: + ATR-exits + vol-target (%1 risk/trade → rejim-koşullu sizing)
    let Some(db) = resolve_db() else { eprintln!("⏭  DB yok — atlandı."); return; };
    eprintln!("📂 DB: {db}");
    let symbols = ["BTCUSDT","ETHUSDT","BNBUSDT","ADAUSDT","XRPUSDT","TRXUSDT","DOGEUSDT","ZECUSDT"];
    let strategies = ["SUPERTREND","EMA_CROSSOVER","MACD","RSI","BB"];
    let (limit, folds) = (6000, 6);
    let ob = Some("liquid");

    let mut by_fold: Vec<[Agg; 3]> = (0..folds).map(|_| [Agg::default(); 3]).collect();
    for sym in symbols {
        let candles = match read_candles(&db, sym, "1h", limit) {
            Ok(c) if c.len() >= folds * 300 => c, _ => continue,
        };
        let mut cs = candles; cs.sort_by_key(|c| c.timestamp);
        let ref_price = cs.iter().map(|c| c.close).sum::<f64>() / cs.len() as f64;
        let fold_len = cs.len() / folds;
        for f in 0..folds {
            let start = f * fold_len;
            let end = if f == folds - 1 { cs.len() } else { (f + 1) * fold_len };
            let seg = &cs[start..end];
            for strat in strategies {
                let run = |atr, vt| Backtester::new(cfg_va(sym, strat, ref_price, ob, atr, vt)).run(seg);
                if let Ok(r) = run(None, None) { by_fold[f][0].add(&r); }
                if let Ok(r) = run(Some((1.5, 3.0)), None) { by_fold[f][1].add(&r); }
                if let Ok(r) = run(Some((1.5, 3.0)), Some(1.0)) { by_fold[f][2].add(&r); }
            }
        }
    }

    eprintln!("\n=== Vol-Adaptif A/B (RegimeDir+Adaptive90+slippage, {folds} dilim, 1h) ===");
    eprintln!("{:>5} | {:>14} | {:>18} | {:>22}", "dilim", "A:baz Σpnl%", "B:ATRexit Σpnl%", "C:ATR+volTgt Σpnl%");
    for f in 0..folds {
        eprintln!("{:>5} | {:>14.1} | {:>18.1} | {:>22.1}",
            f + 1, by_fold[f][0].sum_pnl_pct, by_fold[f][1].sum_pnl_pct, by_fold[f][2].sum_pnl_pct);
    }
    let agg = |i: usize| by_fold.iter().fold(Agg::default(), |mut a, arr| {
        a.runs += arr[i].runs; a.trades += arr[i].trades; a.wins += arr[i].wins;
        a.sum_pnl_pct += arr[i].sum_pnl_pct; a.sum_pf += arr[i].sum_pf;
        a.pf_runs += arr[i].pf_runs; a.sum_sharpe += arr[i].sum_sharpe; a
    });
    eprintln!("\nAggregate:");
    agg(0).report("A:baz");
    agg(1).report("B:ATRexit");
    agg(2).report("C:ATR+volTgt");
    let pos_a = (0..folds).filter(|&f| by_fold[f][0].sum_pnl_pct > 0.0).count();
    let pos_c = (0..folds).filter(|&f| by_fold[f][2].sum_pnl_pct > 0.0).count();
    eprintln!("\nPozitif dilim: A={pos_a}/{folds}, C={pos_c}/{folds}. Karar: C, A'nın PF/Sharpe");
    eprintln!("ve pozitif-dilim sayısını artırıyorsa vol-adaptif katman benimse.\n");
    assert!(by_fold.iter().any(|a| a[0].runs > 0));
}

#[test]
#[ignore = "veri-bağımlı; elle: cargo test --test regime_gate_ab direction_robustness -- --ignored --nocapture"]
fn direction_robustness_ab() {
    // Doğrulama kapıları #1 (walk-forward/OOS tutarlılık) + #2 (slippage). Aggregate
    // +980 tek bir ayı dönemine mi bağlı? Seriyi K ardışık ZAMAN DİLİMİNE böl; her dilimde
    // LongOnly vs RegimeDirectional vs RegimeDirectional+slippage. Tutarlılık = RegimeDir'in
    // LongOnly'yi YENDİĞİ dilim sayısı (tek-dönem artefaktı değilse çoğunu yenmeli).
    let Some(db) = resolve_db() else {
        eprintln!("⏭  DB bulunamadı — robustluk A/B atlandı.");
        return;
    };
    eprintln!("📂 DB: {db}");

    let symbols = [
        "BTCUSDT", "ETHUSDT", "BNBUSDT", "ADAUSDT",
        "XRPUSDT", "TRXUSDT", "DOGEUSDT", "ZECUSDT",
    ];
    let strategies = ["SUPERTREND", "EMA_CROSSOVER", "MACD", "RSI", "BB"];
    let limit = 6000;
    let folds = 6;
    let gate = RegimeGate::Adaptive { pctl: 0.90 };

    // Dilim başına 3 kol: [LongOnly, RegimeDir, RegimeDir+slippage(liquid)].
    let mut by_fold: Vec<[Agg; 3]> = (0..folds).map(|_| [Agg::default(); 3]).collect();

    for sym in symbols {
        let candles = match read_candles(&db, sym, "1h", limit) {
            Ok(c) if c.len() >= folds * 300 => c,
            _ => continue,
        };
        // read_candles DESC döner; zaman sırasına sok ki dilimler kronolojik olsun.
        let mut cs = candles;
        cs.sort_by_key(|c| c.timestamp);
        let ref_price = cs.iter().map(|c| c.close).sum::<f64>() / cs.len() as f64;
        let fold_len = cs.len() / folds;

        for f in 0..folds {
            let start = f * fold_len;
            let end = if f == folds - 1 { cs.len() } else { (f + 1) * fold_len };
            let seg = &cs[start..end];
            for strat in strategies {
                let run = |dir, ob| Backtester::new(cfg_full(sym, strat, ref_price, gate, dir, ob)).run(seg);
                if let Ok(r) = run(DirectionMode::LongOnly, None) { by_fold[f][0].add(&r); }
                if let Ok(r) = run(DirectionMode::RegimeDirectional, None) { by_fold[f][1].add(&r); }
                if let Ok(r) = run(DirectionMode::RegimeDirectional, Some("liquid")) { by_fold[f][2].add(&r); }
            }
        }
    }

    eprintln!("\n=== Yön Robustluk: zaman-dilimi tutarlılığı + slippage ({folds} dilim, 1h) ===");
    eprintln!("{:>5} | {:>22} | {:>22} | {:>22}", "dilim", "LongOnly Σpnl%", "RegimeDir Σpnl%", "RegimeDir+slip Σpnl%");
    let (mut wins, mut wins_slip) = (0, 0);
    for f in 0..folds {
        let lo = by_fold[f][0].sum_pnl_pct;
        let rd = by_fold[f][1].sum_pnl_pct;
        let rs = by_fold[f][2].sum_pnl_pct;
        if rd > lo { wins += 1; }
        if rs > lo { wins_slip += 1; }
        eprintln!("{:>5} | {:>22.1} | {:>22.1} | {:>22.1}{}", f + 1, lo, rd, rs,
            if rd > lo { "  ✓RD" } else { "" });
    }
    eprintln!("\nTutarlılık: RegimeDir {folds} dilimin {wins}'inde LongOnly'yi yendi; \
               slippage'la {wins_slip}/{folds}.");
    eprintln!("Aggregate PF/Sharpe (slippage'lı):");
    by_fold.iter().fold(Agg::default(), |mut a, arr| {
        a.runs += arr[2].runs; a.trades += arr[2].trades; a.wins += arr[2].wins;
        a.sum_pnl_pct += arr[2].sum_pnl_pct; a.sum_pf += arr[2].sum_pf; a.pf_runs += arr[2].pf_runs;
        a.sum_sharpe += arr[2].sum_sharpe; a
    }).report("RD+slip");
    eprintln!("\nKarar: RegimeDir dilimlerin ÇOĞUNDA (≥4/6) ve slippage'la kazanıyorsa edge");
    eprintln!("dönem-bağımsız → canlı opt-in'e tam güven; aksi halde tek-dönem artefaktı.\n");

    assert!(by_fold.iter().any(|a| a[0].runs > 0), "hiç veri işlenemedi");
}
