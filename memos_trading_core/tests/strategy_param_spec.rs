// Param uzayı bütünlük testi: registry'deki HER stratejinin param_spec()'i
// tutarlı mı? — (1) her parametre adı apply_param tarafından TANINIR (typo → sessiz
// no-op'u yakalar), (2) örnek değer gerçekten StrategyParams'a yazılır, (3) min<max,
// step>0. Yeni strateji eklenince bu test onu otomatik kapsar (canonical_pool).

use memos_trading_core::core::types::StrategyParams;
use memos_trading_core::robot::strategies::{default_registry, apply_param, build_params, keys};

#[test]
fn her_strateji_param_spec_tutarli() {
    let reg = default_registry();
    for name in reg.canonical_pool() {
        let strat = reg.make(&name);
        let specs = strat.param_spec();
        for spec in &specs {
            // (1) ad tanınıyor mu? (keys::ALL tek-kaynak sözlük)
            assert!(
                keys::is_known(spec.name),
                "Strateji '{}' param_spec'inde bilinmeyen alan '{}' — apply_param sessizce yok sayar (typo?)",
                name, spec.name,
            );
            // (3) aralık geçerli mi?
            assert!(spec.min < spec.max, "'{}': {} min<max değil", name, spec.name);
            assert!(spec.step > 0.0, "'{}': {} step>0 değil", name, spec.name);

            // (2) tek-alan uygulanınca gerçekten yazılıyor mu?
            let mut p = StrategyParams::default();
            apply_param(&mut p, spec.name, spec.sample(0.5));
            assert!(
                p.get(spec.name).is_some(),
                "'{}': '{}' apply_param sonrası StrategyParams'a yazılmadı",
                name, spec.name,
            );
        }

        // build_params: tüm spec'leri default değerleriyle (min) uygula → panik yok.
        let mins: Vec<f64> = specs.iter().map(|s| s.min).collect();
        let _ = build_params(&specs, &mins);
    }
}

#[test]
fn crossover_stratejilerde_fast_slow_ortusmez() {
    // fast.max < slow.min → her örneklemde fast < slow garanti (dejenere combo yok).
    let reg = default_registry();
    for name in ["MA_CROSSOVER", "EMA_CROSSOVER", "MACD"] {
        let specs = reg.make(name).param_spec();
        let fast = specs.iter().find(|s| s.name == "fast");
        let slow = specs.iter().find(|s| s.name == "slow");
        if let (Some(f), Some(s)) = (fast, slow) {
            assert!(
                f.max < s.min,
                "'{}': fast.max ({}) < slow.min ({}) değil → fast>=slow dejenere combo mümkün",
                name, f.max, s.min,
            );
        }
    }
}
