// xfunding — CROSS-EXCHANGE funding-spread ÖLÇÜM aracı (venue Faz 1+ (f)).
//
// Soru: aynı perp'in (ör. BTCUSDT) Binance ve Bybit funding'i farklı → bu fark hasat edilebilir
// bir edge mi? Klasik funding-arb: YÜKSEK-funding borsada SHORT (funding alır) + DÜŞÜK-funding
// borsada LONG (funding öder) → fiyat-riski iki bacakta birbirini götürür (delta-nötr, cross-exchange),
// her funding aralığında |spread| = funding_H − funding_L hasat edilir. Getiri ≈ |spread|/2 (sermaye
// iki bacağa yayılır) eksi icra maliyeti (yön değişince rebalance).
//
// ÖLÇÜM-ÖNCE (edge-first): mekanik çoklu-venue altyapısına yatırım yapmadan önce edge VAR MI? Bu araç
// onu söyler. Maliyet > spread ise (f) çürür; aksi halde gerçek yürütme katmanını (b) haklı çıkarır.
//
// ÖN-KOŞUL: her İKİ borsanın funding'i DB'de olmalı:
//   cargo run --release --example download_funding -- futures BTCUSDT,ETHUSDT,... 2
//   EXCHANGE=bybit cargo run --release --example download_funding -- futures BTCUSDT,ETHUSDT,... 2
//
// Kullanım:
//   cargo run --release --example xfunding -- [SYM1,SYM2,...]
// Env: DB_PATH (default data/trader.db), XF_FEE_PER_LEG (taker bps/100, default 0.0005=5bps),
//      XF_MARKET (default futures), XF_FUNDING_LIMIT (default 20000 kayıt/sembol/borsa).
//
// NOT: pooled değil PORTFÖY-serisi Sharpe/t (her aralıkta mevcut sembollerin ortalaması) — küçük-örneklem
// diklik şişmesini önler ([[project_funding_carry]] dersi). Maliyet flip-frekansından amortize edilir.

use std::collections::BTreeMap;

use memos_trading_core::persistence::reader::read_funding_exchange;

#[derive(Clone, Copy)]
struct XfCfg {
    fee_per_leg: f64,      // tek bacak taker komisyonu (oran)
    fundings_per_year: f64, // 8h funding → 3/gün × 365 ≈ 1095
}

/// Tek sembolün hizalanmış cross-exchange funding serisi: (funding_time_ms, binance_rate, bybit_rate).
struct SymSeries {
    symbol: String,
    aligned: Vec<(i64, f64, f64)>,
}

struct XfResult {
    n_symbols: usize,
    n_obs: usize,             // pooled (sembol×aralık) gözlem
    n_portfolio_intervals: usize,
    mean_signed_spread: f64,  // E[bybit − binance] (yapısal yön var mı; ~0 beklenir)
    mean_abs_spread: f64,     // E[|spread|]
    std_spread: f64,
    gross_ann_return: f64,    // |spread|/2 yıllık (maliyet öncesi)
    net_ann_return: f64,      // maliyet sonrası yıllık
    sharpe_net: f64,          // portföy-serisi net Sharpe (yıllık)
    t_stat_net: f64,          // portföy net getiri t-istatistiği
    flip_rate: f64,           // spread işaret-değiştirme oranı (turnover vekili)
    pct_portfolio_positive: f64,
    lag1_autocorr: f64,       // spread kalıcılığı (yüksek → düşük turnover)
    per_symbol: Vec<(String, usize, f64)>, // (sembol, n, mean_abs_spread)
}

/// İki borsanın funding serisini funding_time üzerinde İÇ-BİRLEŞTİR (her ikisinde de olan aralıklar).
/// Linear USDT-perp her iki borsada da 8h sınırlarında funding'lediği için zaman damgaları örtüşür.
fn align(binance: &[(i64, f64)], bybit: &[(i64, f64)]) -> Vec<(i64, f64, f64)> {
    let bmap: BTreeMap<i64, f64> = binance.iter().copied().collect();
    let mut out = Vec::new();
    for &(t, by_rate) in bybit {
        if let Some(&bi_rate) = bmap.get(&t) {
            out.push((t, bi_rate, by_rate));
        }
    }
    out.sort_by_key(|(t, _, _)| *t);
    out
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() { return 0.0; }
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// Örneklem standart sapması (n−1).
fn std_dev(xs: &[f64]) -> f64 {
    let n = xs.len();
    if n < 2 { return 0.0; }
    let m = mean(xs);
    let var = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n as f64 - 1.0);
    var.sqrt()
}

/// Lag-1 otokorelasyon (spread kalıcılığı).
fn lag1_autocorr(xs: &[f64]) -> f64 {
    let n = xs.len();
    if n < 3 { return 0.0; }
    let m = mean(xs);
    let denom: f64 = xs.iter().map(|x| (x - m).powi(2)).sum();
    if denom <= 0.0 { return 0.0; }
    let num: f64 = (1..n).map(|i| (xs[i] - m) * (xs[i - 1] - m)).sum();
    num / denom
}

/// Saf çekirdek: hizalanmış cross-exchange serilerinden edge ölçümü.
fn analyze(series: &[SymSeries], cfg: &XfCfg) -> XfResult {
    // Pooled spread istatistikleri (karakterizasyon) + per-sembol özet.
    let mut all_spreads: Vec<f64> = Vec::new();
    let mut per_symbol: Vec<(String, usize, f64)> = Vec::new();
    let mut n_symbols = 0usize;

    // flip-rate (yön değişimi) per-sembol hesaplanıp ortalanır.
    let mut flip_rates: Vec<f64> = Vec::new();

    // Portföy net getiri serisi: her funding_time'da mevcut sembollerin net getiri ortalaması.
    // net_birim_getiri(t,sym) = |spread|/2 − (yön bu aralıkta değiştiyse) 2·fee.
    let mut port_acc: BTreeMap<i64, (f64, usize)> = BTreeMap::new();

    for s in series {
        if s.aligned.is_empty() { continue; }
        n_symbols += 1;
        let spreads: Vec<f64> = s.aligned.iter().map(|(_, bi, by)| by - bi).collect();
        let mean_abs = mean(&spreads.iter().map(|x| x.abs()).collect::<Vec<_>>());
        per_symbol.push((s.symbol.clone(), spreads.len(), mean_abs));
        all_spreads.extend(&spreads);

        // Per-aralık net getiri + flip tespiti.
        let mut flips = 0usize;
        let mut prev_sign = 0i8;
        for (k, &(t, _, _)) in s.aligned.iter().enumerate() {
            let sp = spreads[k];
            let sign = if sp > 0.0 { 1i8 } else if sp < 0.0 { -1i8 } else { 0i8 };
            let flipped = prev_sign != 0 && sign != 0 && sign != prev_sign;
            if flipped { flips += 1; }
            if sign != 0 { prev_sign = sign; }
            // Hasat: sermayenin iki bacağa yayıldığı varsayımıyla |spread|/2; flip'te 2·fee (kapat+aç, iki venue).
            let gross = sp.abs() / 2.0;
            let cost = if flipped { 2.0 * cfg.fee_per_leg } else { 0.0 };
            let net = gross - cost;
            let e = port_acc.entry(t).or_insert((0.0, 0));
            e.0 += net;
            e.1 += 1;
        }
        if s.aligned.len() > 1 {
            flip_rates.push(flips as f64 / (s.aligned.len() as f64 - 1.0));
        }
    }

    let port_series: Vec<f64> = port_acc.values().map(|(sum, n)| sum / *n as f64).collect();
    let abs_spreads: Vec<f64> = all_spreads.iter().map(|x| x.abs()).collect();

    let mean_abs_spread = mean(&abs_spreads);
    let mean_signed_spread = mean(&all_spreads);
    let std_spread = std_dev(&all_spreads);
    let flip_rate = mean(&flip_rates);

    let m_port = mean(&port_series);
    let sd_port = std_dev(&port_series);
    let n_port = port_series.len();
    let gross_ann_return = (mean_abs_spread / 2.0) * cfg.fundings_per_year;
    let net_ann_return = m_port * cfg.fundings_per_year;
    let sharpe_net = if sd_port > 0.0 {
        (m_port / sd_port) * cfg.fundings_per_year.sqrt()
    } else { 0.0 };
    let t_stat_net = if sd_port > 0.0 && n_port > 1 {
        m_port / (sd_port / (n_port as f64).sqrt())
    } else { 0.0 };
    let pct_portfolio_positive = if n_port > 0 {
        port_series.iter().filter(|x| **x > 0.0).count() as f64 / n_port as f64
    } else { 0.0 };
    let lag1 = lag1_autocorr(&all_spreads);

    per_symbol.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    XfResult {
        n_symbols,
        n_obs: all_spreads.len(),
        n_portfolio_intervals: n_port,
        mean_signed_spread,
        mean_abs_spread,
        std_spread,
        gross_ann_return,
        net_ann_return,
        sharpe_net,
        t_stat_net,
        flip_rate,
        pct_portfolio_positive,
        lag1_autocorr: lag1,
        per_symbol,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let default_basket = "BTCUSDT,ETHUSDT,SOLUSDT,XRPUSDT,BNBUSDT,ADAUSDT,DOGEUSDT,AVAXUSDT,LINKUSDT,LTCUSDT,BCHUSDT,TRXUSDT";
    let symbols: Vec<String> = args.get(1)
        .map(|s| s.as_str())
        .unwrap_or(default_basket)
        .split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect();

    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());
    let market = std::env::var("XF_MARKET").unwrap_or_else(|_| "futures".into());
    let fee_per_leg: f64 = std::env::var("XF_FEE_PER_LEG").ok().and_then(|s| s.parse().ok()).unwrap_or(0.0005);
    let funding_limit: usize = std::env::var("XF_FUNDING_LIMIT").ok().and_then(|s| s.parse().ok()).unwrap_or(20_000);
    let cfg = XfCfg { fee_per_leg, fundings_per_year: 3.0 * 365.0 };

    println!("🔀 xfunding · {} sembol · market={market} · fee/bacak={:.4} ({:.1}bps) · db={db_path}",
        symbols.len(), fee_per_leg, fee_per_leg * 10_000.0);

    let mut series = Vec::new();
    let mut missing = Vec::new();
    for sym in &symbols {
        let bi = read_funding_exchange(&db_path, "binance", sym, &market, funding_limit).unwrap_or_default();
        let by = read_funding_exchange(&db_path, "bybit", sym, &market, funding_limit).unwrap_or_default();
        if bi.is_empty() || by.is_empty() {
            missing.push((sym.clone(), bi.len(), by.len()));
            continue;
        }
        let aligned = align(&bi, &by);
        if aligned.is_empty() {
            missing.push((sym.clone(), bi.len(), by.len()));
            continue;
        }
        series.push(SymSeries { symbol: sym.clone(), aligned });
    }

    if !missing.is_empty() {
        println!("\n⚠️  Veri eksik/hizalanmadı ({} sembol) — atlanıyor:", missing.len());
        for (s, nb, ny) in &missing {
            println!("    {:12} binance={} bybit={}", s, nb, ny);
        }
        println!("    Önce her iki borsa için download_funding çalıştır (EXCHANGE=bybit dahil).");
    }
    if series.is_empty() {
        println!("\n❌ Hizalanmış cross-exchange serisi yok → ölçüm yapılamadı.");
        return;
    }

    let r = analyze(&series, &cfg);

    println!("\n── Spread karakterizasyonu (bybit − binance) ──────────────");
    println!("  semboller        : {} (hizalı), {} pooled gözlem", r.n_symbols, r.n_obs);
    println!("  E[spread] (signed): {:+.6} ({:+.3} bps)  ← ~0 yapısal yön yok demek", r.mean_signed_spread, r.mean_signed_spread * 10_000.0);
    println!("  E[|spread|]       : {:.6} ({:.3} bps)", r.mean_abs_spread, r.mean_abs_spread * 10_000.0);
    println!("  std(spread)       : {:.6}", r.std_spread);
    println!("  lag-1 otokorr.    : {:+.3}  ← yüksek = kalıcı = düşük turnover", r.lag1_autocorr);
    println!("  işaret-flip oranı : {:.1}%  ← rebalance/maliyet vekili", r.flip_rate * 100.0);

    println!("\n── Hasat edilebilirlik (delta-nötr cross-exchange book) ────");
    println!("  portföy aralığı   : {}", r.n_portfolio_intervals);
    println!("  GROSS yıllık      : {:+.2}%  (|spread|/2, maliyet öncesi)", r.gross_ann_return * 100.0);
    println!("  NET yıllık        : {:+.2}%  (flip-maliyeti sonrası)", r.net_ann_return * 100.0);
    println!("  NET Sharpe        : {:.2}", r.sharpe_net);
    println!("  NET t-stat        : {:.2}", r.t_stat_net);
    println!("  net-pozitif aralık: {:.1}%", r.pct_portfolio_positive * 100.0);

    println!("\n── Per-sembol mean |spread| (bps) ─────────────────────────");
    for (s, n, ma) in r.per_symbol.iter().take(20) {
        println!("  {:12} n={:5}  {:.3} bps", s, n, ma * 10_000.0);
    }

    println!("\n→ KARAR:");
    if r.net_ann_return > 0.0 && r.t_stat_net >= 2.0 {
        println!("  ✅ NET pozitif & t≥2 → cross-exchange funding spread HASAT EDİLEBİLİR görünüyor.");
        println!("     Sonraki: WF/OOS doğrulama + gerçek-icra (b) maliyet/slippage gerçekçiliği.");
    } else if r.gross_ann_return > 0.0 && r.net_ann_return <= 0.0 {
        println!("  ❌ GROSS var ama NET maliyetle siliniyor (fee/bacak={:.1}bps) → mevcut maliyette EDGE YOK.", fee_per_leg * 10_000.0);
        println!("     (b) yürütme yatırımı bu maliyette haklı çıkmaz; maker/daha düşük fee gerekir.");
    } else {
        println!("  ❌ NET edge zayıf/yok (t={:.2}) → bu sepet/maliyette cross-exchange funding edge YOK.", r.t_stat_net);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_inner_joins_on_time() {
        let binance = vec![(100, 0.001), (200, 0.002), (300, 0.003)];
        let bybit = vec![(200, 0.0025), (300, 0.0035), (400, 0.004)];
        let a = align(&binance, &bybit);
        assert_eq!(a.len(), 2, "yalnız ortak zaman damgaları (200, 300)");
        assert_eq!(a[0], (200, 0.002, 0.0025));
        assert_eq!(a[1], (300, 0.003, 0.0035));
    }

    #[test]
    fn constant_spread_no_flips_is_profitable_gross() {
        // bybit hep binance'ten 10bps yüksek → sabit pozitif spread, hiç flip yok.
        let aligned: Vec<(i64, f64, f64)> = (0..100)
            .map(|i| (i as i64 * 28_800_000, 0.0001, 0.0001 + 0.0010))
            .collect();
        let series = vec![SymSeries { symbol: "X".into(), aligned }];
        // Sıfır fee → net == gross; spread sabit → std 0 → Sharpe 0 ama getiri pozitif.
        let cfg = XfCfg { fee_per_leg: 0.0, fundings_per_year: 1095.0 };
        let r = analyze(&series, &cfg);
        assert!(r.flip_rate.abs() < 1e-9, "sabit işaret → flip yok");
        assert!((r.mean_abs_spread - 0.0010).abs() < 1e-9);
        // gross yıllık ≈ (0.0010/2)*1095
        assert!((r.gross_ann_return - 0.0005 * 1095.0).abs() < 1e-6);
        assert!((r.net_ann_return - r.gross_ann_return).abs() < 1e-9, "fee yok → net=gross");
    }

    #[test]
    fn fee_wipes_out_when_spread_flips_every_interval() {
        // Spread her aralıkta işaret değiştiriyor → her aralık flip → ağır maliyet.
        let aligned: Vec<(i64, f64, f64)> = (0..50)
            .map(|i| {
                let s = if i % 2 == 0 { 0.0002 } else { -0.0002 };
                (i as i64 * 28_800_000, 0.0001, 0.0001 + s)
            })
            .collect();
        let series = vec![SymSeries { symbol: "X".into(), aligned }];
        // gross/aralık = |0.0002|/2 = 1bp; flip maliyeti = 2*fee = 2*5bps = 10bps her flip → net negatif.
        let cfg = XfCfg { fee_per_leg: 0.0005, fundings_per_year: 1095.0 };
        let r = analyze(&series, &cfg);
        assert!(r.flip_rate > 0.9, "neredeyse her aralık flip");
        assert!(r.net_ann_return < 0.0, "maliyet gross'u silmeli");
    }

    #[test]
    fn lag1_autocorr_detects_persistence() {
        // Monoton-pozitif kalıcı seri → yüksek lag-1.
        let persistent: Vec<f64> = (0..50).map(|i| 0.001 + i as f64 * 1e-6).collect();
        assert!(lag1_autocorr(&persistent) > 0.9);
        // Alternating → negatif lag-1.
        let alt: Vec<f64> = (0..50).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
        assert!(lag1_autocorr(&alt) < 0.0);
    }
}
