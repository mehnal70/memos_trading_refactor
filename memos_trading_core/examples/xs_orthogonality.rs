// xs_orthogonality — momentum ile funding-carry DİK mi, ve BİRLEŞİK portföy additif mi?
//
// Amaç: iki doğrulanmış pooled edge var — XS momentum ([[project_xs_momentum]]) ve funding-carry
// ([[project_funding_carry]]). Soru: carry momentum'a EKLENİYOR mu (ayrı bilgi) yoksa ÖRTÜŞÜYOR mu?
// Ölçüt: (1) iki NET-getiri serisinin Pearson korelasyonu ρ — düşükse dik; (2) 50/50 BİRLEŞİK
// portföyün Sharpe'ı tek tek her ikisinden BELİRGİN yüksekse → diversifikasyon kazancı = additif.
// İki market-nötr edge düşük-korelasyonluysa birleşim Sharpe'ı √2'ye kadar artırabilir (asıl ödül).
//
// AYNI sepet/bar üzerinde tail-hizalı ölçüm (warmup farkı sondan eşitlenir). Metrikler XS makinesinden
// (series_metrics → Newey-West + Sharpe). DB'de hem mum hem funding olmalı (download_funding ön-koşul).
//
// Kullanım:
//   cargo run --release --example xs_orthogonality -- [market] [interval] [SYM1,SYM2,...]
// Env: DB_PATH, O_MOM_LB (momentum lookback, default 14), O_CARRY_LB (carry lookback, default 14),
//      O_WEIGHT (birleşimde carry ağırlığı, default 0.5), O_TOP_K (3), O_FEE_RATE (0.0005),
//      O_MOM_REBALANCE (momentum rebalance bar, default 1), O_CARRY_REBALANCE (carry rebalance, default 1),
//      O_CANDLE_LIMIT (5000), O_FUNDING_LIMIT (20000).
//
// PORTFÖY TP/DD OVERLAY TARAMASI (cycle-içi kâr-al zamanlaması hipotezi — [[project_funding_carry]]):
//   Canlı book_core kuralını aynalar: bir rebalance cycle'ı içinde kümülatif kitap-kârı +TP%'e
//   ulaşırsa kitabı FLAT'a çek (kalan barlar 0), sonraki rebalance'ta yeniden kur. Hipotez: kitap-kârı
//   eşiği aşınca cycle dolmadan geri döner mi → erken TP net Sharpe'ı artırır mı? CANLI ile birebir
//   olması için rebalance=14 ver (BLEND_LIVE_REBALANCE_BARS) ve carry-ağırlıklı w=0.6.
//   Env: O_TP_SWEEP (csv TP% ızgarası, ör. "0,5,7,10,15"; 0=kapalı=saf tut), O_TP_DD (DD freni %, default 8
//        — canlı BLEND_LIVE_MAX_DD_PCT), O_TP_FLATTEN_TURN (erken-kapama turnover birimi, default 2.0).
//   Örnek (canlı paper-blend'e sadık):
//     O_WEIGHT=0.6 O_MOM_REBALANCE=14 O_CARRY_REBALANCE=14 O_TP_SWEEP=0,5,7,10,15 \
//       cargo run --release --example xs_orthogonality -- futures 1d BTCUSDT,ETHUSDT,SOLUSDT,XRPUSDT,BNBUSDT,ADAUSDT,DOGEUSDT,AVAXUSDT,LINKUSDT,LTCUSDT,BCHUSDT,DOTUSDT

use memos_trading_core::robot::backtester::{
    run_xs_returns, run_funding_carry_returns, series_metrics, XsConfig, XsSignal, FundingCarryConfig,
};

fn csv(arg: Option<&String>) -> Vec<String> {
    match arg.map(|s| s.as_str()) {
        None | Some("all") | Some("") => Vec::new(),
        Some(s) => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
    }
}

fn bars_per_year(interval: &str) -> f64 {
    match interval { "1h" => 8_760.0, "4h" => 2_190.0, "1d" => 365.0, _ => 365.0 }
}

/// İki seriyi sondan eşitle (warmup farkını ele; ikisi de son bara biter) → ortak uzunluk dilimleri.
fn tail_align<'a>(a: &'a [f64], b: &'a [f64]) -> (&'a [f64], &'a [f64]) {
    let n = a.len().min(b.len());
    (&a[a.len() - n..], &b[b.len() - n..])
}

/// Portföy-düzeyi TAKE-PROFIT/DD overlay'i — canlı `book_core` kuralını aynalar. Bir rebalance cycle'ı
/// (turnover>0 ile başlar) içinde kümülatif kitap-kârı +tp_pct'e ulaşırsa kitabı FLAT'a çek (kalan
/// barlarda 0 getiri), sonraki rebalance'ta yeniden kurulur. dd_pct simetrik felaket-freni. Erken-kapama
/// ek maliyeti flatten_turn×fee_rate olarak düşülür. `rets` ZATEN net (bar-fee dahil); `turns`>0 →
/// rebalance bar'ı. Döner: (overlay'li getiri, flatten olay sayısı, flat-tutulan bar sayısı).
fn tp_dd_overlay(
    rets: &[f64], turns: &[f64], tp_pct: f64, dd_pct: f64, flatten_turn: f64, fee_rate: f64,
) -> (Vec<f64>, usize, usize) {
    let mut out = rets.to_vec();
    let mut cum = 1.0_f64;
    let mut flat = false;
    let (mut n_flat, mut flat_bars) = (0usize, 0usize);
    for i in 0..rets.len() {
        if turns.get(i).copied().unwrap_or(0.0) > 1e-9 { cum = 1.0; flat = false; } // yeni cycle: yeniden kur
        if flat { out[i] = 0.0; flat_bars += 1; continue; }
        cum *= 1.0 + rets[i];
        let gain = (cum - 1.0) * 100.0;
        if (tp_pct > 0.0 && gain >= tp_pct) || (dd_pct > 0.0 && gain <= -dd_pct) {
            out[i] -= flatten_turn * fee_rate; // erken kitap-kapama maliyeti
            flat = true;
            n_flat += 1;
        }
    }
    (out, n_flat, flat_bars)
}

fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n < 3 { return 0.0; }
    let (ma, mb) = (a.iter().sum::<f64>() / n as f64, b.iter().sum::<f64>() / n as f64);
    let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (da, db) = (a[i] - ma, b[i] - mb);
        cov += da * db; va += da * da; vb += db * db;
    }
    if va <= 0.0 || vb <= 0.0 { 0.0 } else { cov / (va.sqrt() * vb.sqrt()) }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let market = args.get(1).map(|s| s.as_str()).unwrap_or("futures").to_string();
    let interval = args.get(2).map(|s| s.as_str()).unwrap_or("1d").to_string();
    let symbols = csv(args.get(3));
    if symbols.len() < 4 {
        eprintln!("⚠️  En az 4 sembollük bir sepet ver.");
        eprintln!("    cargo run --release --example xs_orthogonality -- futures 1d BTCUSDT,ETHUSDT,...");
        std::process::exit(2);
    }

    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());
    let mom_lb: usize = std::env::var("O_MOM_LB").ok().and_then(|s| s.parse().ok()).unwrap_or(14);
    let carry_lb: usize = std::env::var("O_CARRY_LB").ok().and_then(|s| s.parse().ok()).unwrap_or(14);
    let w_carry: f64 = std::env::var("O_WEIGHT").ok().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.5).clamp(0.0, 1.0);
    let top_k: usize = std::env::var("O_TOP_K").ok().and_then(|s| s.parse().ok()).unwrap_or(3);
    let fee_rate: f64 = std::env::var("O_FEE_RATE").ok().and_then(|s| s.parse().ok()).unwrap_or(0.0005);
    let candle_limit: usize = std::env::var("O_CANDLE_LIMIT").ok().and_then(|s| s.parse().ok()).unwrap_or(5000);
    let funding_limit: usize = std::env::var("O_FUNDING_LIMIT").ok().and_then(|s| s.parse().ok()).unwrap_or(20_000);
    // Carry YAVAŞ sinyal → fee-dayanıklılık için iki-haftalık rebalance (turnover kesintisi). Default 1.
    let carry_rb: usize = std::env::var("O_CARRY_REBALANCE").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    // Momentum rebalance (TP-overlay testinde canlıyla hizalamak için 14 ver). Default 1 (geriye-uyum).
    let mom_rb: usize = std::env::var("O_MOM_REBALANCE").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let bpy = bars_per_year(&interval);

    // Momentum getiri serisi.
    let mom_cfg = XsConfig {
        db_path: db_path.clone(), market: market.clone(), interval: interval.clone(),
        symbols: symbols.clone(), candle_limit, signal: XsSignal::Momentum, lookback: mom_lb,
        top_k, fee_rate, long_short: true, rebalance_every: mom_rb, bars_per_year: bpy, ..Default::default()
    };
    let (mom_rets, mom_turn) = run_xs_returns(&mom_cfg);

    // Funding-carry getiri serisi.
    let carry_cfg = FundingCarryConfig {
        db_path: db_path.clone(), market: market.clone(), interval: interval.clone(),
        symbols: symbols.clone(), candle_limit, funding_limit, lookback: carry_lb,
        top_k, fee_rate, long_short: true, rebalance_every: carry_rb, bars_per_year: bpy, ..Default::default()
    };
    let (carry_rets, carry_turn) = run_funding_carry_returns(&carry_cfg);

    if mom_rets.len() < 10 || carry_rets.len() < 10 {
        eprintln!("⚠️  Yetersiz seri (momentum={} bar, carry={} bar). Funding indirildi mi? Sepet yeterli mi?",
            mom_rets.len(), carry_rets.len());
        std::process::exit(1);
    }

    // Tail-hizalı ortak dilim → aynı barlarda ρ + birleşik portföy.
    let (m, c) = tail_align(&mom_rets, &carry_rets);
    let (mt, ct) = tail_align(&mom_turn, &carry_turn);
    let n = m.len();
    let rho = pearson(m, c);

    let mom_m = series_metrics(m, mt, 1.0, bpy, 1, 30);
    let car_m = series_metrics(c, ct, 1.0, bpy, 1, 30);

    println!("🧭 DİKLİK & BİRLEŞİM · market={market} · interval={interval} · sepet={} sembol · ortak {n} bar",
        symbols.len());
    println!("   momentum(lb={mom_lb}) ⊕ funding-carry(lb={carry_lb})");
    println!();
    println!("   {:<16} {:>8} {:>7} {:>7} {:>7}", "portföy", "annRet%", "Sharpe", "NW-t", "NW-p");
    println!("   {}", "-".repeat(52));
    let row = |name: &str, r: &memos_trading_core::robot::backtester::XsResult| {
        println!("   {:<16} {:>8.1} {:>7.2} {:>7.2} {:>7.3}",
            name, 100.0 * r.ann_return, r.ann_sharpe, r.nw_t_stat, r.nw_t_pvalue());
    };
    row("momentum", &mom_m);
    row("funding-carry", &car_m);

    // Sabit O_WEIGHT birleşim (referans).
    let combo = |w: f64| -> Vec<f64> { (0..n).map(|i| (1.0 - w) * m[i] + w * c[i]).collect() };
    let combo_turn = |w: f64| -> Vec<f64> { (0..n).map(|i| (1.0 - w) * mt[i] + w * ct[i]).collect() };
    let fixed = series_metrics(&combo(w_carry), &combo_turn(w_carry), 1.0, bpy, 1, 30);
    row(&format!("birleşik w={w_carry:.2}"), &fixed);

    // AĞIRLIK TARAMASI: carry ağırlığı 0..1 (0.05 adım) → en yüksek Sharpe (eşit-ağırlık güçlü edge'i
    // seyreltir; iki dik edge'in optimal birleşimi ayrı bir noktada). 50/50 false-negative'ini önler.
    let mut best_w = w_carry;
    let mut best = series_metrics(&combo(best_w), &combo_turn(best_w), 1.0, bpy, 1, 30);
    let mut w = 0.0;
    while w <= 1.0001 {
        let r = series_metrics(&combo(w), &combo_turn(w), 1.0, bpy, 1, 30);
        if r.ann_sharpe > best.ann_sharpe { best = r; best_w = w; }
        w += 0.05;
    }
    row(&format!("birleşik w*={best_w:.2}"), &best);

    println!();
    println!("   ρ(momentum, carry) = {:+.3}", rho);
    // Teorik tavan: iki KORELASYONSUZ edge için max Sharpe = √(Sm²+Sc²) (ann-Sharpe-invariant).
    let ceiling = (mom_m.ann_sharpe.powi(2) + car_m.ann_sharpe.powi(2)).sqrt();
    let best_single = mom_m.ann_sharpe.max(car_m.ann_sharpe);
    println!("   teorik tavan √(Sm²+Sc²) = {:.2} (ρ=0 varsayımı) · en iyi tekil = {:.2}", ceiling, best_single);

    println!();
    let dik = if rho.abs() < 0.3 { "✓ DİK" } else if rho.abs() < 0.6 { "~ kısmi örtüşme" } else { "✗ örtüşük" };
    println!("   Diklik: {dik} (|ρ|={:.2})", rho.abs());
    let gain = if best_single != 0.0 { (best.ann_sharpe - best_single) / best_single.abs() } else { 0.0 };
    if rho.abs() < 0.4 && best.ann_sharpe > best_single + 0.02 {
        println!("   ✅ ADDİTİF: optimal birleşim (w*={best_w:.2}) Sharpe {:.2} > en iyi tekil {:.2} (+{:.0}%).",
            best.ann_sharpe, best_single, 100.0 * gain);
        println!("      İki düşük-korelasyonlu market-nötr edge → diversifikasyon. Eşit-ağırlık güçlüyü seyreltir;");
        println!("      doğru ağırlıkta birleşim teorik tavana (√Σ) yaklaşır → momentum + carry BİRLİKTE taşınmalı.");
    } else if best.ann_sharpe > best_single + 0.02 {
        println!("   ~ Optimal birleşim Sharpe {:.2} > tekil {:.2} ama ρ yüksek → kazanç kısmi (örtüşme).",
            best.ann_sharpe, best_single);
    } else {
        println!("   ✗ Optimal birleşim Sharpe {:.2} ≤ tekil {:.2} → diversifikasyon kazancı yok.",
            best.ann_sharpe, best_single);
    }

    // ── PORTFÖY TP/DD OVERLAY TARAMASI (cycle-içi kâr-al zamanlaması) ──────────────────────────
    let tp_grid: Vec<f64> = std::env::var("O_TP_SWEEP").ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_default();
    if !tp_grid.is_empty() {
        let dd_pct: f64 = std::env::var("O_TP_DD").ok().and_then(|s| s.parse().ok()).unwrap_or(8.0);
        let flatten_turn: f64 = std::env::var("O_TP_FLATTEN_TURN").ok().and_then(|s| s.parse().ok()).unwrap_or(2.0);
        let series = combo(w_carry);
        let turns_series = combo_turn(w_carry);
        let n_rebal = turns_series.iter().filter(|&&t| t > 1e-9).count();
        println!();
        println!("💰 PORTFÖY TP/DD OVERLAY · w_carry={w_carry:.2} · mom_rb={mom_rb} carry_rb={carry_rb} · cycle=turnover>0 ({n_rebal} rebalance) · DD-fren=−%{dd_pct:.0} · erken-kapama {flatten_turn:.1}×fee");
        println!("   hipotez: kitap-kârı +%X'i aşınca cycle dolmadan geri döner mi → erken TP net'i artırır mı?");
        println!("   {:<10} {:>8} {:>7} {:>7} {:>7} {:>9} {:>7}", "TP", "annRet%", "Sharpe", "NW-t", "NW-p", "#flatten", "flat%");
        println!("   {}", "-".repeat(64));
        for &tp in &tp_grid {
            let (ov, n_flat, flat_bars) = tp_dd_overlay(&series, &turns_series, tp, dd_pct, flatten_turn, fee_rate);
            let r = series_metrics(&ov, &turns_series, 1.0, bpy, 1, 30);
            let label = if tp <= 0.0 { "kapalı".to_string() } else { format!("+%{tp:.0}") };
            println!("   {:<10} {:>8.1} {:>7.2} {:>7.2} {:>7.3} {:>9} {:>6.1}%",
                label, 100.0 * r.ann_return, r.ann_sharpe, r.nw_t_stat, r.nw_t_pvalue(),
                n_flat, 100.0 * flat_bars as f64 / series.len().max(1) as f64);
        }
        println!();
        println!("   Okuma: 'kapalı' = saf 14-bar tut (yalnız DD freni) = TP müdahalesiz taban. Erken TP satırının");
        println!("   Sharpe'ı hem 'kapalı'dan hem +%10'dan BELİRGİN yüksekse → erken kâr-al haklı, %5'e çek.");
        println!("   Değilse (ince-edge'de kazananı budamak tipik olarak Sharpe'ı düşürür) → +%10 doğru, dokunma.");
    }
}
