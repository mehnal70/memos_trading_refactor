// memos_trading_core/src/core/bridge.rs

use crate::core::model::{
    AiBrainSnapshot, AnomalyModel, BistDataStatusModel, ChartSnapshot, ClosedTradeModel,
    DataCoverageModel, DecisionTraceModel, FinanceSnapshot, LogEntry, MarketAnalysisModel,
    MissionControl, OpportunitySnapshotModel, PipelineStep, PositionModel, PyramidPerformanceModel,
    RuntimeLockDiagnosticSnapshot, SrZoneModel, StrategyPerformanceModel, TradeDistribution,
    TradeTypeStats, VenueHealthModel, VenuePortfolioSnapshot, VenueRuntimeSnapshot,
    VenueSymbolSnapshot, WorkerModel,
};
use crate::robot::robotic_loop::AppState;
use std::collections::BTreeMap;

fn read_candles_for_symbol(
    st: &AppState,
    symbol: &str,
    interval: &str,
    limit: usize,
) -> Vec<crate::core::types::Candle> {
    match st.venue_registry.route(symbol) {
        Some((venue, bare)) => {
            let (exchange, market) = venue.storage_namespace();
            let bare_candles = crate::persistence::reader::read_candles_for_venue_profile(
                &st.config.db_path,
                bare,
                interval,
                exchange,
                market,
                st.config.venue_profile.as_str(),
                limit,
            )
            .unwrap_or_default();
            let labeled_candles = if symbol != bare {
                crate::persistence::reader::read_candles_for_venue_profile(
                    &st.config.db_path,
                    symbol,
                    interval,
                    exchange,
                    market,
                    st.config.venue_profile.as_str(),
                    limit,
                )
                .unwrap_or_default()
            } else {
                Vec::new()
            };
            let bare_last = bare_candles.last().map(|c| c.timestamp);
            let labeled_last = labeled_candles.last().map(|c| c.timestamp);
            if labeled_last > bare_last {
                labeled_candles
            } else {
                bare_candles
            }
        }
        None => crate::persistence::reader::read_candles_market(
            &st.config.db_path,
            symbol,
            interval,
            &st.config.market,
            limit,
        )
        .unwrap_or_default(),
    }
}

/// - AppState'den saf bir Snapshot (Anlık Görüntü) çıkarır.
/// Bu fonksiyon robotun 'Adli Tercümanı'dır.
pub fn get_snapshot(st: &AppState) -> MissionControl {
    // Snapshot üretimi, cycle içindeki fiyat güncellemelerinden bağımsız olarak
    // canonical pozisyon kopyasını güncel live_positions kaynağıyla eşitler.
    st.refresh_canonical_positions();
    if let Ok(mut trades) = st.finance.live_closed_trades.try_write() {
        let _ = ClosedTradeModel::deduplicate(&mut trades);
    }
    // 1. FİNANSAL HASAT (Anlık PnL Hesaplamaları)
    // Live profilinde reel muhasebe yalnızca Binance hesabını kapsar;
    // MT5 ve IBKR demo bakiyeleri global equity'ye eklenmez.
    let reporting_venue = std::env::var("REAL_EQUITY_VENUE")
        .unwrap_or_else(|_| "binance".to_string())
        .trim()
        .to_ascii_lowercase();
    let live_binance_reporting = reporting_venue == "binance";
    let open_pnl: f64 = if live_binance_reporting {
        st.finance
            .live_positions
            .try_read()
            .ok()
            .map(|positions| {
                positions
                    .values()
                    .filter(|position| {
                        position
                            .venue
                            .as_ref()
                            .map(|venue| venue.token().to_ascii_lowercase().starts_with("binance:"))
                            .unwrap_or(false)
                    })
                    .map(PositionModel::calculate_pnl)
                    .sum()
            })
            .unwrap_or(0.0)
    } else {
        st.fleet
            .symbol_orchestrator
            .try_read()
            .ok()
            .map(|orch| orch.total_open_pnl(None))
            .unwrap_or(0.0)
    };
    let total_fees: f64 = st
        .finance
        .live_execution_costs
        .try_read()
        .ok()
        .map(|c| c.total_cost_usd)
        .unwrap_or(0.0);
    let (venue_accounts, reconciliation_statuses) = st
        .venue_runtime
        .try_read()
        .map(|runtimes| {
            let accounts: BTreeMap<String, memos_venue_api::AccountSnapshot> = runtimes
                .iter()
                .filter_map(|(venue, runtime)| {
                    runtime
                        .account_snapshot
                        .clone()
                        .map(|snapshot| (venue.clone(), snapshot))
                })
                .collect();
            let statuses: BTreeMap<String, String> = runtimes
                .iter()
                .filter(|(_, runtime)| !runtime.reconciliation_status.is_empty())
                .map(|(venue, runtime)| (venue.clone(), runtime.reconciliation_status.clone()))
                .collect();
            (accounts, statuses)
        })
        .unwrap_or_default();
    let now = chrono::Utc::now();
    let mut compatible_balance = None;
    let mut compatible_equity = None;
    let mut aggregate_currency: Option<String> = None;
    let mut excluded_accounts = false;
    let currencies_compatible = |left: &str, right: &str| {
        (left == right)
            || ((left == "USD" || left == "USDT") && (right == "USD" || right == "USDT"))
    };
    for (account_key, account) in &venue_accounts {
        if live_binance_reporting && !account_key.to_ascii_lowercase().starts_with("binance:") {
            excluded_accounts = true;
            continue;
        }
        let fresh = matches!(
            account.status,
            memos_venue_api::AccountSnapshotStatus::Fresh
                | memos_venue_api::AccountSnapshotStatus::Partial
        ) && now.signed_duration_since(account.timestamp).num_seconds() <= 600;
        let inferred_currency = account.currency.as_deref().or_else(|| {
            let key = account_key.to_ascii_lowercase();
            (key.starts_with("mt5:") || key.starts_with("ibkr:")).then_some("USD")
        });
        if let (Some(currency), Some(equity)) = (inferred_currency, account.equity) {
            if fresh && equity.is_finite() && !currency.is_empty() {
                let same_currency = aggregate_currency
                    .as_deref()
                    .map(|current| currencies_compatible(current, currency))
                    .unwrap_or(true);
                if same_currency {
                    aggregate_currency.get_or_insert_with(|| currency.to_string());
                    *compatible_balance.get_or_insert(0.0) += account.balance;
                    *compatible_equity.get_or_insert(0.0) += equity;
                    continue;
                }
            }
        }
        excluded_accounts = true;
    }
    let aggregate_status = if compatible_equity.is_some() {
        if excluded_accounts {
            memos_venue_api::AccountSnapshotStatus::Partial
        } else {
            memos_venue_api::AccountSnapshotStatus::Fresh
        }
    } else {
        memos_venue_api::AccountSnapshotStatus::Unavailable
    };
    // Venue equity zaten venue tarafındaki floating PnL'yi içerir; tekrar
    // symbol_orchestrator PnL'si eklenirse toplam iki kez şişer.
    let total_equity = compatible_equity.unwrap_or_else(|| st.finance.equity + open_pnl);
    let reporting_starting_capital = if live_binance_reporting {
        std::env::var("REAL_PNL_BASELINE_USD")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or_else(|| total_equity.max(0.0))
    } else {
        st.config.capital
    };
    let finance = FinanceSnapshot {
        total_equity,
        realize_pnl: total_equity - reporting_starting_capital - open_pnl,
        open_pnl,
        starting_capital: reporting_starting_capital,
        total_fees,
        balance: compatible_balance,
        currency: aggregate_currency,
        timestamp: Some(now),
        status: aggregate_status,
        venue_accounts: venue_accounts.clone(),
    };

    // 2. POZİSYON DÖNÜŞTÜRÜCÜ — clone yeterli (struct serialize-friendly)
    let positions: Vec<PositionModel> = st
        .finance
        .live_positions
        .try_read()
        .ok()
        .map(|m| m.values().cloned().collect())
        .unwrap_or_default();

    // 3. ADLİ LOG VE ARŞİV HASADI (Son 100 log ve 50 işlem)
    // Timestamp log satırının kendisinde mühürlü (push_log "[HH:MM:SS] msg"
    // formatında yazıyor); burada PARSE edilir. Daha önce Local::now()
    // çağrısıyla her snapshot'ta yeniden üretiliyordu → log paneli her render'da
    // "değişti" görünüyor, TUI'de görsel flicker yaratıyordu.
    let mut logs: Vec<LogEntry> = st
        .guardian
        .log
        .iter()
        .rev()
        .take(100)
        .map(|line| {
            let level = if line.contains("ERROR") {
                "ERROR"
            } else if line.contains("WARN") {
                "WARN"
            } else if line.contains("SIGNAL") {
                "SIGNAL"
            } else {
                "INFO"
            };
            let (timestamp, message) = parse_log_prefix(line);
            LogEntry {
                timestamp,
                message,
                level: level.to_string(),
            }
        })
        .collect();
    let active_symbols: Option<std::collections::BTreeSet<String>> =
        std::env::var("PAPER_SYMBOLS").ok().map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|symbol| !symbol.is_empty())
                .map(str::to_owned)
                .collect()
        });
    let mut decision_trace: Vec<DecisionTraceModel> = logs
        .iter()
        .filter_map(|l| parse_decision_trace(l, &st.venue_registry))
        .collect();
    // Karar audit'i log rotasyonu veya son 100 satır sınırı nedeniyle görünmez
    // olsa bile, snapshot karar zincirini doğrudan son audit kayıtlarından taşı.
    if let Ok(decisions) = st.fleet.latest_decisions.read() {
        for audit in decisions.values() {
            if active_symbols
                .as_ref()
                .map(|symbols| symbols.contains(&audit.symbol))
                .unwrap_or(true)
            {
                let venue_profile =
                    crate::robot::engines::master::venue_profile::VenueProfile::for_symbol(
                        &audit.symbol,
                        &st.venue_registry,
                    );
                decision_trace.push(DecisionTraceModel {
                    timestamp: audit.timestamp.clone(),
                    symbol: audit.symbol.clone(),
                    outcome: audit.outcome.clone(),
                    strategy: audit.strategy.clone(),
                    regime: audit.regime.clone(),
                    edge: audit.edge.is_finite().then_some(audit.edge),
                    pyramid_limit: venue_profile.max_pyramid_tranches,
                    max_atr_pct: venue_profile.max_atr_pct,
                    reason: audit.reason.clone(),
                    shadow_signal: audit.shadow_signal.clone(),
                    shadow_status: audit.shadow_status.clone(),
                    shadow_reason: audit.shadow_reason.clone(),
                });
            }
        }
    }
    decision_trace.sort_by(|a, b| {
        b.timestamp
            .cmp(&a.timestamp)
            .then_with(|| a.symbol.cmp(&b.symbol))
    });
    decision_trace.dedup_by(|a, b| {
        a.symbol == b.symbol && a.timestamp == b.timestamp && a.reason == b.reason
    });
    decision_trace.truncate(20);

    let mut trade_history: Vec<ClosedTradeModel> = vec![];
    let equity_series: Vec<f64> = st
        .finance
        .equity_history
        .try_read()
        .map(|h| h.iter().copied().collect())
        .unwrap_or_default();
    let peak = st.finance.peak_equity.max(st.finance.equity);
    let current_dd = if peak > 0.0 {
        ((peak - st.finance.equity) / peak * 100.0).max(0.0)
    } else {
        0.0
    };
    let mut charts = ChartSnapshot {
        distributions: vec![],
        total_closed_pnl: 0.0,
        total_trade_count: 0,
        equity_series,
        current_drawdown_pct: current_dd,
        peak_equity: peak,
    };

    if let Ok(trades) = st.finance.live_closed_trades.try_read() {
        trade_history = trades
            .iter()
            .rev()
            .take(50)
            .map(|t| ClosedTradeModel {
                symbol: t.symbol.clone(),
                is_long: t.is_long,
                pnl: t.pnl,
                pnl_pct: t.pnl_pct,
                net_pnl: t.net_pnl,
                net_pnl_pct: t.net_pnl_pct,
                commission: t.commission,
                exit_reason: t.exit_reason.clone(),
                closed_at: t.closed_at.clone(),
                opened_at: t.opened_at.clone(),
                leverage: t.leverage,
                entry_price: t.entry_price,
                exit_price: t.exit_price,
                strategy: t.strategy.clone(),
                venue: t.venue.clone(),
                pyramid_tranches: t.pyramid_tranches,
                exchange_ref: t.exchange_ref.clone(),
            })
            .collect();

        // Pasta Grafiği (Trade Dağılımı) Lojiği
        let mut dist_map = std::collections::HashMap::new();
        for t in trades.iter() {
            let net_pnl = t.effective_net_pnl();
            let entry = dist_map
                .entry(t.symbol.clone())
                .or_insert((0.0_f64, 0u32, 0u32));
            entry.0 += net_pnl;
            entry.1 += 1;
            if net_pnl > 0.0 {
                entry.2 += 1;
            }
        }
        charts.total_closed_pnl = trades.iter().map(ClosedTradeModel::effective_net_pnl).sum();
        charts.total_trade_count = trades.len();
        charts.distributions = dist_map
            .into_iter()
            .map(|(sym, (pnl, cnt, wins))| TradeDistribution {
                symbol: sym,
                pnl,
                trade_count: cnt,
                win_rate: if cnt > 0 {
                    (wins as f64 / cnt as f64) * 100.0
                } else {
                    0.0
                },
            })
            .collect();
    }
    let strategy_performance = compute_strategy_performance(&trade_history);
    let pyramid_performance = compute_pyramid_performance(&trade_history);
    let venue_portfolios = build_venue_portfolios(&venue_accounts, &positions, &trade_history);

    // 4. FİLO VE PAZAR ALGISI (Worker Status + S/R Zones)
    let live_price_map = st
        .fleet
        .live_price
        .try_read()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    // 24h % değişim paydası: spawn_sr_updater'ın kilit-dışı hesapladığı ~24h önceki referans fiyat.
    let ref_price_24h_map = st
        .fleet
        .live_ref_price_24h
        .try_read()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    let fleet: Vec<WorkerModel> = st
        .fleet
        .symbol_orchestrator
        .try_read()
        .ok()
        .map(|orch| {
            orch.get_worker_status()
                .into_iter()
                .filter(|worker| {
                    active_symbols
                        .as_ref()
                        .map(|symbols| symbols.contains(&worker.symbol))
                        .unwrap_or(true)
                })
                .map(|w| {
                    let price = live_price_map.get(&w.symbol).copied().unwrap_or(0.0);
                    WorkerModel {
                        symbol: w.symbol,
                        market: w.market,
                        interval: w.interval,
                        price,
                        change_pct: 0.0, // change_pct ileride live_price snapshot'tan beslenecek
                        uptime_secs: w.uptime_secs,
                        is_paused: w.paused,
                        score: 0.0, // skor ml_engine'den geldiğinde wire'lanır
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let mut opportunity_symbols: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    for p in &positions {
        if active_symbols
            .as_ref()
            .map(|symbols| symbols.contains(&p.symbol))
            .unwrap_or(true)
        {
            opportunity_symbols.insert(p.symbol.clone());
        }
    }
    for w in &fleet {
        opportunity_symbols.insert(w.symbol.clone());
    }
    let expected_opportunity_symbols = opportunity_symbols.len();
    let expected_symbol_names = opportunity_symbols.clone();
    let mut opportunity_snapshots: Vec<OpportunitySnapshotModel> = opportunity_symbols
        .into_iter()
        .filter_map(|symbol| {
            // Guard-içi route: fırsat kartları da sembolün KENDİ venue namespace'inden okur.
            // Global market_label fallback'ı burada açıkça eğer venue_registry mevcutsa
            // devre dışı bırakılır; böylece MT5/Binance gibi aynı-isim semboller karışmaz.
            let candles = read_candles_for_symbol(st, &symbol, "15m", 30);
            crate::robot::data_pipeline::MemoryOpportunitySnapshot::from_candles(
                &symbol, "15m", &candles,
            )
            .map(|snap_15m| OpportunitySnapshotModel {
                symbol: snap_15m.symbol,
                interval: snap_15m.interval,
                bars: snap_15m.bars,
                span_minutes: snap_15m.span_minutes,
                opportunity_score: snap_15m
                    .trend_bias
                    .abs()
                    .max(snap_15m.scalp_bias.abs())
                    .max(snap_15m.swing_bias.abs()),
                phase: snap_15m.phase,
                rsi_14: snap_15m.rsi_14,
                momentum_pct: snap_15m.momentum_pct,
                volatility_pct: snap_15m.volatility_pct,
                trend_bias: snap_15m.trend_bias,
                scalp_bias: snap_15m.scalp_bias,
                swing_bias: snap_15m.swing_bias,
                last_ts: snap_15m.last_ts,
            })
        })
        .collect();
    opportunity_snapshots.sort_by(|a, b| {
        b.opportunity_score
            .partial_cmp(&a.opportunity_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let freshness_budget_secs = 45 * 60;
    let now = chrono::Utc::now();
    let mut stale_symbol_names = Vec::new();
    let mut max_age_secs = None;
    let fresh_symbols = opportunity_snapshots
        .iter()
        .filter(|snapshot| {
            let Some(last_ts) = snapshot.last_ts else {
                stale_symbol_names.push(snapshot.symbol.clone());
                return false;
            };
            let age_secs = (now - last_ts).num_seconds().max(0);
            max_age_secs = Some(max_age_secs.unwrap_or(0).max(age_secs));
            if age_secs > freshness_budget_secs {
                stale_symbol_names.push(snapshot.symbol.clone());
                false
            } else {
                true
            }
        })
        .count();
    let missing_symbols = expected_opportunity_symbols.saturating_sub(opportunity_snapshots.len());
    let evaluated_symbol_names: std::collections::BTreeSet<String> = opportunity_snapshots
        .iter()
        .map(|snapshot| snapshot.symbol.clone())
        .collect();
    let missing_symbol_names: Vec<String> = expected_symbol_names
        .difference(&evaluated_symbol_names)
        .cloned()
        .collect();
    let stale_symbols = stale_symbol_names.len();
    let coverage_status = if missing_symbols > 0 {
        "MISSING"
    } else if stale_symbols > 0 {
        "STALE"
    } else {
        "FRESH"
    };
    let data_coverage = DataCoverageModel {
        expected_symbols: expected_opportunity_symbols,
        evaluated_symbols: opportunity_snapshots.len(),
        fresh_symbols,
        stale_symbols,
        missing_symbols,
        max_age_secs,
        freshness_budget_secs,
        status: coverage_status.to_string(),
        priority: if missing_symbols > 0 || stale_symbols > 0 {
            "DATA_COMPLETION".to_string()
        } else {
            "SIGNAL_EVALUATION".to_string()
        },
        stale_symbol_names,
        missing_symbol_names,
    };
    opportunity_snapshots.truncate(8);
    let download_progress = st
        .fleet
        .download_progress
        .read()
        .map(|progress| progress.clone())
        .unwrap_or_default();
    let bist_symbols: Vec<String> = expected_symbol_names
        .iter()
        .filter(|symbol| {
            st.venue_registry
                .route(symbol)
                .map(|(venue, _)| venue.exchange() == crate::core::types::Exchange::Bist)
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    let bist_latest: Vec<chrono::DateTime<chrono::Utc>> = bist_symbols
        .iter()
        .filter_map(|symbol| {
            read_candles_for_symbol(st, symbol, "1d", 1)
                .last()
                .map(|candle| candle.timestamp)
        })
        .collect();
    let bist_now = chrono::Utc::now();
    let bist_freshness_budget_secs =
        crate::core::env::env_parse_or("BIST_DATA_FRESHNESS_SECS", 48 * 60 * 60_i64);
    let rejected_rows = crate::robot::bist_trend::LAST_SHEET_REJECTED_ROWS
        .load(std::sync::atomic::Ordering::Relaxed);
    let bist_fresh_symbols = bist_latest
        .iter()
        .filter(|timestamp| {
            (bist_now - **timestamp).num_seconds().max(0) <= bist_freshness_budget_secs
        })
        .count();
    let bist_latest_candle = bist_latest.iter().copied().max();
    let bist_latest_age_secs =
        bist_latest_candle.map(|timestamp| (bist_now - timestamp).num_seconds().max(0));
    let bist_source_sync_status = if download_progress.active {
        "IN_PROGRESS"
    } else if download_progress.failed_symbols > 0 || !download_progress.last_error.is_empty() {
        "ERROR"
    } else if rejected_rows > 0 {
        "DEGRADED"
    } else if download_progress.total_symbols > 0
        && download_progress.completed_symbols == download_progress.total_symbols
    {
        "COMPLETED"
    } else {
        "WAITING"
    };
    let market_state = crate::robot::bist_calendar::market_state(bist_now);
    let daily_candle_expectation = match market_state {
        "SESSION_ACTIVE" => "WAITING_FOR_CLOSE",
        "POST_CLOSE" => {
            if download_progress.active {
                "SYNCING_DAILY_DATA"
            } else if bist_fresh_symbols < bist_symbols.len() {
                "DAILY_DATA_RETRY_REQUIRED"
            } else {
                "DAILY_DATA_COMPLETED"
            }
        }
        _ => "NO_NEW_CANDLE_EXPECTED",
    };
    let soak_health = if bist_symbols.is_empty() {
        "WAITING"
    } else if bist_source_sync_status == "ERROR"
        || bist_source_sync_status == "DEGRADED"
        || daily_candle_expectation == "DAILY_DATA_RETRY_REQUIRED"
    {
        "DEGRADED"
    } else if download_progress.active {
        "RECOVERING"
    } else if bist_fresh_symbols == bist_symbols.len() {
        "HEALTHY"
    } else {
        "WAITING"
    };
    let bist_data_status = BistDataStatusModel {
        data_source: std::env::var("BIST_DATA_PROVIDER")
            .unwrap_or_else(|_| "google_sheets".to_string()),
        source_sync_status: bist_source_sync_status.to_string(),
        latest_candle: bist_latest_candle.map(|timestamp| timestamp.to_rfc3339()),
        latest_age_secs: bist_latest_age_secs,
        expected_symbols: bist_symbols.len(),
        fresh_symbols: bist_fresh_symbols,
        stale_symbols: bist_symbols.len().saturating_sub(bist_fresh_symbols),
        rejected_rows,
        market_state: market_state.to_string(),
        daily_candle_expectation: daily_candle_expectation.to_string(),
        soak_health: soak_health.to_string(),
        readiness_status: "SIGNAL_ONLY".to_string(),
        execution_status: "DISABLED".to_string(),
    };

    let mut market_fleet: Vec<MarketAnalysisModel> = st
        .fleet
        .live_sr_zones
        .try_read()
        .ok()
        .map(|zones_map| {
            zones_map
                .iter()
                .map(|(sym, zones)| {
                    let current_price = live_price_map.get(sym).copied().unwrap_or(0.0);
                    // En yakın destek: midpoint ≤ price (mevcut fiyatın altındaki en yüksek destek).
                    // En yakın direnç: midpoint ≥ price (mevcut fiyatın üstündeki en düşük direnç).
                    // Fiyat 0 ise (henüz live price yok) en yüksek destek / en düşük direnç fallback.
                    let nearest_support = zones
                        .iter()
                        .filter(|z| {
                            matches!(z.zone_type, crate::robot::sr_detector::ZoneType::Support)
                                && (current_price <= 0.0 || z.midpoint <= current_price)
                        })
                        .max_by(|a, b| {
                            a.midpoint
                                .partial_cmp(&b.midpoint)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                        .map(|z| z.midpoint);
                    let nearest_resistance = zones
                        .iter()
                        .filter(|z| {
                            matches!(z.zone_type, crate::robot::sr_detector::ZoneType::Resistance)
                                && (current_price <= 0.0 || z.midpoint >= current_price)
                        })
                        .min_by(|a, b| {
                            a.midpoint
                                .partial_cmp(&b.midpoint)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                        .map(|z| z.midpoint);
                    let zones_converted = zones
                        .iter()
                        .map(|z| SrZoneModel {
                            zone_type: format!("{:?}", z.zone_type),
                            price_low: z.price_low,
                            price_high: z.price_high,
                            strength: z.strength,
                            touch_count: z.touch_count,
                        })
                        .collect();
                    // 24h % değişim: (güncel − ~24h önceki referans)/referans·100. Referans yoksa (SR updater
                    // henüz koşmadı / candle yetersiz) ya da fiyat 0 → 0.0 (gösterimde nötr).
                    let change_24h = match ref_price_24h_map.get(sym).copied() {
                        Some(r) if r > 0.0 && current_price > 0.0 => {
                            (current_price - r) / r * 100.0
                        }
                        _ => 0.0,
                    };
                    let market = st
                        .venue_registry
                        .route(sym)
                        .map(|(venue, _)| venue.market().as_str().to_string())
                        .unwrap_or_else(|| st.config.market.clone());
                    MarketAnalysisModel {
                        symbol: sym.clone(),
                        market,
                        current_price,
                        change_24h,
                        zones: zones_converted,
                        nearest_support,
                        nearest_resistance,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // 4b) Blend ekraninda MT5 veri gorunurlugu: MT5 sembollerini market_fleet'e
    // izleme modeli olarak ekle (zone yok, fiyat + tazelik). Boylece panelde ikinci
    // blokta gorunur; blend trade evrenine mudahele etmez.
    let mt5_interval = st.config.interval.clone();
    let mt5_symbols = parse_mt5_symbols_from_env();
    if !mt5_symbols.is_empty() {
        let now = chrono::Utc::now();
        let (fresh_window_mins, fresh_window_label) = mt5_freshness_window(now);
        let mut newest_ts: Option<chrono::DateTime<chrono::Utc>> = None;
        let mut newest_fresh_ts: Option<chrono::DateTime<chrono::Utc>> = None;
        let mut present = 0usize;
        let mut fresh = 0usize;

        for bare_sym in &mt5_symbols {
            let candles = crate::persistence::reader::read_candles_ns_for_profile(
                &st.config.db_path,
                bare_sym,
                &mt5_interval,
                "mt5",
                "cfd",
                st.config.venue_profile.as_str(),
                200,
            )
            .unwrap_or_default();

            let tagged = format!("{}@mt5", bare_sym);
            let already_in_market_fleet = market_fleet
                .iter()
                .any(|m| m.symbol == tagged || m.symbol == *bare_sym);
            let current_price = live_price_map
                .get(&tagged)
                .copied()
                .filter(|price| *price > 0.0)
                .or_else(|| candles.last().map(|candle| candle.close))
                .unwrap_or(0.0);
            let change_24h = monitor_change_24h(&candles, current_price);
            if let Some(c) = candles.last() {
                present += 1;
                if newest_ts.map_or(true, |x| c.timestamp > x) {
                    newest_ts = Some(c.timestamp);
                }
                let age_mins = (now - c.timestamp).num_minutes();
                if age_mins >= 0 && age_mins <= fresh_window_mins {
                    fresh += 1;
                    if newest_fresh_ts.map_or(true, |x| c.timestamp > x) {
                        newest_fresh_ts = Some(c.timestamp);
                    }
                }
            }

            if !already_in_market_fleet {
                push_unique_market_analysis(
                    &mut market_fleet,
                    MarketAnalysisModel {
                        symbol: tagged.clone(),
                        market: "mt5-monitor".to_string(),
                        current_price,
                        change_24h,
                        zones: vec![],
                        nearest_support: None,
                        nearest_resistance: None,
                    },
                );
            }
        }

        let newest_s = newest_ts
            .map(format_utc_local_ts)
            .unwrap_or_else(|| "yok".to_string());
        let newest_fresh_s = newest_fresh_ts
            .map(format_utc_local_ts)
            .unwrap_or_else(|| "yok".to_string());
        let newest_age_mins = newest_ts
            .map(|ts| (now - ts).num_minutes().max(0))
            .unwrap_or(i64::MAX);
        let mt5_data_status = if newest_age_mins == i64::MAX {
            "YOK"
        } else if newest_age_mins <= fresh_window_mins {
            "TAZE"
        } else {
            "BAYAT"
        };
        let newest_age_s = if newest_age_mins == i64::MAX {
            "yok".to_string()
        } else if newest_age_mins < 60 {
            format!("{}dk", newest_age_mins)
        } else {
            format!("{}s {}dk", newest_age_mins / 60, newest_age_mins % 60)
        };
        let hint = if mt5_data_status == "BAYAT" {
            " · not: takvim açık olsa da EA/bridge akışı gecikmiş olabilir"
        } else {
            ""
        };
        let ts = chrono::Local::now().format("%H:%M:%S").to_string();
        logs.insert(
            0,
            LogEntry {
                timestamp: ts,
                message: format!(
                    "📡 MT5 veri özeti: profile={} exchange=mt5 market=cfd interval={} · {}/{} sembol DB'de · durum={} · taze(<={})={} · taze_bar={} · yaş={} · son_bar={}{}",
                    st.config.venue_profile.as_str(),
                    mt5_interval,
                    present,
                    mt5_symbols.len(),
                    mt5_data_status,
                    fresh_window_label,
                    fresh,
                    newest_fresh_s,
                    newest_age_s,
                    newest_s,
                    hint,
                ),
                level: "INFO".to_string(),
            },
        );
    }

    // IBKR sembollerini de Market Gözetimi'nde canlı fiyatlarıyla görünür kıl.
    let ibkr_symbols =
        parse_venue_symbols_from_env("AUTO_IBKR_SYMBOLS", crate::core::types::Exchange::Ibkr);
    for bare_sym in ibkr_symbols {
        let symbol = format!("{}@ibkr", bare_sym);
        if market_fleet
            .iter()
            .any(|m| m.symbol == symbol || m.symbol == bare_sym)
        {
            continue;
        }
        let candles = crate::persistence::reader::read_candles_ns(
            &st.config.db_path,
            &bare_sym,
            &st.config.interval,
            "ibkr",
            "cfd",
            200,
        )
        .unwrap_or_default();
        let current_price = live_price_map
            .get(&symbol)
            .copied()
            .filter(|price| *price > 0.0)
            .or_else(|| candles.last().map(|candle| candle.close))
            .unwrap_or(0.0);
        push_unique_market_analysis(
            &mut market_fleet,
            MarketAnalysisModel {
                symbol: symbol.clone(),
                market: "ibkr-monitor".to_string(),
                current_price,
                change_24h: monitor_change_24h(&candles, current_price),
                zones: vec![],
                nearest_support: None,
                nearest_resistance: None,
            },
        );
    }

    // Binance sembollerini de Market Gözetimi'nde S/R bölgesi olmasa bile göster.
    let binance_symbols = parse_venue_symbols_from_env(
        "AUTO_BINANCE_SYMBOLS",
        crate::core::types::Exchange::Binance,
    );
    for bare_sym in binance_symbols {
        let tagged = format!("{}@binance", bare_sym);
        if market_fleet
            .iter()
            .any(|m| m.symbol == tagged || m.symbol == bare_sym)
        {
            continue;
        }
        let current_price = live_price_map
            .get(&tagged)
            .or_else(|| live_price_map.get(&bare_sym))
            .copied()
            .unwrap_or(0.0);
        let candles = crate::persistence::reader::read_candles_ns(
            &st.config.db_path,
            &bare_sym,
            &st.config.interval,
            "binance",
            &st.config.market,
            200,
        )
        .unwrap_or_default();
        push_unique_market_analysis(
            &mut market_fleet,
            MarketAnalysisModel {
                symbol: tagged,
                market: st.config.market.clone(),
                current_price,
                change_24h: monitor_change_24h(&candles, current_price),
                zones: vec![],
                nearest_support: None,
                nearest_resistance: None,
            },
        );
    }
    // BIST sembollerini de veri-only venue olarak Market Gözetimi'ne ekle.
    let bist_symbols =
        parse_venue_symbols_from_env("AUTO_BIST_SYMBOLS", crate::core::types::Exchange::Bist);
    for bare_sym in bist_symbols {
        let tagged = format!("{}@bist", bare_sym);
        if market_fleet
            .iter()
            .any(|m| m.symbol == tagged || m.symbol == bare_sym)
        {
            continue;
        }
        let candles = crate::persistence::reader::read_candles_ns_for_profile(
            &st.config.db_path,
            &bare_sym,
            &st.config.interval,
            "bist",
            "spot",
            st.config.venue_profile.as_str(),
            200,
        )
        .unwrap_or_default();
        let current_price = candles.last().map(|candle| candle.close).unwrap_or(0.0);
        push_unique_market_analysis(
            &mut market_fleet,
            MarketAnalysisModel {
                symbol: tagged,
                market: "bist-monitor".to_string(),
                current_price,
                change_24h: monitor_change_24h(&candles, current_price),
                zones: vec![],
                nearest_support: None,
                nearest_resistance: None,
            },
        );
    }
    let mt5_bridge_is_healthy = st
        .mt5_bridge_healthy
        .load(std::sync::atomic::Ordering::Relaxed);
    let venue_health: Vec<VenueHealthModel> = [
        ("Binance", "@binance"),
        ("MT5", "@mt5"),
        ("IBKR", "@ibkr"),
        ("BIST", "@bist"),
    ]
    .into_iter()
    .map(|(venue, suffix)| {
        let symbols: Vec<_> = market_fleet
            .iter()
            .filter(|item| {
                item.symbol.ends_with(suffix) || (venue == "Binance" && !item.symbol.contains('@'))
            })
            .collect();
        let priced_symbols = symbols
            .iter()
            .filter(|item| item.current_price > 0.0)
            .count();
        let oldest_price_age_secs = symbols
            .iter()
            .filter_map(|item| {
                let price = live_price_map.get(&item.symbol)?;
                (*price > 0.0).then_some(0_i64)
            })
            .max();
        let data_status = if symbols.is_empty() {
            "YOK"
        } else if priced_symbols == symbols.len() {
            "OK"
        } else {
            "KISMİ"
        };
        let bridge_status = match venue {
            "MT5" if mt5_bridge_is_healthy => "OK",
            "MT5" => "KONTROL",
            "IBKR" if priced_symbols > 0 => "OK",
            "IBKR" => "KONTROL",
            "BIST" if priced_symbols > 0 => "VERİ-ONLY",
            "BIST" => "KONTROL",
            _ => "YEREL",
        };
        let venue_prefix = venue.to_ascii_lowercase();
        let account_status = venue_accounts
            .iter()
            .find(|(key, _)| key.to_ascii_lowercase().starts_with(&venue_prefix))
            .map(|(_, account)| format!("{:?}", account.status).to_uppercase())
            .unwrap_or_else(|| {
                let has_position = positions
                    .iter()
                    .any(|position| position.symbol.ends_with(suffix));
                if symbols.is_empty() || !has_position {
                    "N/A".to_string()
                } else {
                    "UNAVAILABLE".to_string()
                }
            });
        let protection_status = if symbols.is_empty() {
            "N/A".to_string()
        } else {
            let venue_positions: Vec<_> = positions
                .iter()
                .filter(|position| position.symbol.ends_with(suffix))
                .collect();
            if venue_positions.is_empty() {
                "N/A".to_string()
            } else if venue_positions
                .iter()
                .all(|position| position.stop_loss > 0.0 || position.take_profit > 0.0)
            {
                "OK".to_string()
            } else if venue_positions
                .iter()
                .any(|position| position.market.eq_ignore_ascii_case("cfd"))
            {
                "CRITICAL".to_string()
            } else {
                "UNKNOWN".to_string()
            }
        };
        let reconciliation_status = if symbols.is_empty() {
            "N/A".to_string()
        } else {
            reconciliation_statuses
                .iter()
                .find(|(key, _)| {
                    key.to_ascii_lowercase()
                        .starts_with(&format!("{}:", venue.to_ascii_lowercase()))
                })
                .map(|(_, status)| status.clone())
                .unwrap_or_else(|| "UNKNOWN".to_string())
        };
        let venue_positions: Vec<_> = positions
            .iter()
            .filter(|position| position.symbol.ends_with(suffix))
            .collect();
        let notional: f64 = venue_positions
            .iter()
            .filter_map(|position| {
                let value = position.entry_price.abs() * position.qty.abs();
                value.is_finite().then_some(value)
            })
            .sum();
        let exposure_status = if venue_positions.is_empty() {
            "N/A".to_string()
        } else if !finance.total_equity.is_finite() || finance.total_equity <= 0.0 {
            "UNKNOWN".to_string()
        } else {
            let exposure_ratio = notional / finance.total_equity;
            if exposure_ratio > 5.0 {
                "CRITICAL".to_string()
            } else if exposure_ratio > 1.0 {
                "WARNING".to_string()
            } else {
                "OK".to_string()
            }
        };
        let overall_status = if symbols.is_empty()
            && account_status == "N/A"
            && protection_status == "N/A"
            && venue_positions.is_empty()
        {
            "INACTIVE".to_string()
        } else if protection_status == "CRITICAL"
            || (account_status == "UNAVAILABLE" && !st.live_dry_run)
            || (account_status == "STALE" && !st.live_dry_run)
            || exposure_status == "CRITICAL"
        {
            "CRITICAL".to_string()
        } else if data_status != "OK"
            || bridge_status == "KONTROL"
            || protection_status == "UNKNOWN"
            || reconciliation_status == "UNKNOWN"
            || exposure_status == "WARNING"
            || exposure_status == "UNKNOWN"
            || account_status == "PARTIAL"
        {
            "WARNING".to_string()
        } else {
            "OK".to_string()
        };
        let account_snapshot = venue_accounts
            .iter()
            .find(|(key, _)| key.to_ascii_lowercase().starts_with(&venue_prefix))
            .map(|(_, account)| account.clone());
        let account_fresh_positive = account_snapshot
            .map(|account| {
                account.status == memos_venue_api::AccountSnapshotStatus::Fresh
                    && chrono::Utc::now()
                        .signed_duration_since(account.timestamp)
                        .num_seconds()
                        <= 600
                    && account
                        .equity
                        .map(|equity| equity.is_finite() && equity > 0.0)
                        .unwrap_or(false)
            })
            .unwrap_or(false);
        let execution_enabled = match venue {
            "Binance" => st
                .venue_registry
                .get(crate::core::types::Exchange::Binance)
                .map(|adapter| adapter.execution_engine().is_some())
                .unwrap_or(false),
            "MT5" => st
                .venue_registry
                .get(crate::core::types::Exchange::Mt5)
                .map(|adapter| adapter.execution_engine().is_some())
                .unwrap_or(false),
            "IBKR" => st
                .venue_registry
                .get(crate::core::types::Exchange::Ibkr)
                .map(|adapter| adapter.execution_engine().is_some())
                .unwrap_or(false),
            _ => false,
        };
        let execution_status = if venue == "BIST" {
            "SIGNAL_ONLY".to_string()
        } else if execution_enabled {
            "ENABLED".to_string()
        } else {
            "DISABLED".to_string()
        };
        let mut readiness_blockers = Vec::new();
        if venue != "BIST" {
            if bridge_status == "KONTROL" || data_status != "OK" {
                readiness_blockers.push("bağlantı veya fiyat verisi hazır değil".to_string());
            }
            if !account_fresh_positive && !st.live_dry_run {
                readiness_blockers.push("fresh ve pozitif hesap snapshot yok".to_string());
            }
            if execution_enabled && !st.live_dry_run && reconciliation_status != "MATCHED" {
                readiness_blockers.push("pozisyon mutabakatı doğrulanmadı".to_string());
            }
            if protection_status == "CRITICAL" || protection_status == "UNKNOWN" {
                readiness_blockers.push("aktif pozisyon koruması doğrulanmadı".to_string());
            }
            if exposure_status == "CRITICAL"
                || exposure_status == "WARNING"
                || exposure_status == "UNKNOWN"
            {
                readiness_blockers.push("risk maruziyeti kritik".to_string());
            }
            if !execution_enabled {
                readiness_blockers.push("venue execution kapalı".to_string());
            }
        }
        let critical_readiness_blocker = protection_status == "CRITICAL"
            || (account_status == "UNAVAILABLE" && !st.live_dry_run)
            || (account_status == "STALE" && !st.live_dry_run)
            || exposure_status == "CRITICAL";
        let readiness_status = crate::robot::venue::health::decide_readiness(
            venue == "BIST",
            !readiness_blockers.is_empty(),
            critical_readiness_blocker,
            !symbols.is_empty() || venue == "BIST",
        )
        .as_str();
        VenueHealthModel {
            venue: venue.to_string(),
            watched_symbols: symbols.len(),
            priced_symbols,
            data_status: data_status.to_string(),
            bridge_status: bridge_status.to_string(),
            oldest_price_age_secs,
            connectivity_status: bridge_status.to_string(),
            account_status,
            protection_status,
            reconciliation_status,
            exposure_status,
            overall_status,
            execution_status,
            readiness_status: readiness_status.to_string(),
            readiness_blockers,
            symbol_filters_status: if venue == "BIST" {
                "NOT_APPLICABLE".to_string()
            } else {
                "ENFORCED_ON_ORDER".to_string()
            },
        }
    })
    .collect();
    let venue_symbols = build_venue_symbols(st, &market_fleet, &venue_health);
    if let Ok(conn) = crate::persistence::open_db(&st.config.db_path) {
        if let Err(error) = crate::persistence::writer::save_venue_symbol_snapshots(
            &conn,
            st.config.venue_profile.as_str(),
            &venue_symbols,
        ) {
            log::warn!("venue symbol catalog yazılamadı: {error}");
        }
    }

    // 4c) Venue/market saat görünürlüğü: her configured venue için açık/kapalı
    // durumu + seans aralığı + sonraki açılış/kapanış UTC olarak log'a yaz.
    let venue_tokens = configured_venues_from_env(&st.config.market);
    if !venue_tokens.is_empty() {
        let now = chrono::Utc::now();
        let ts = chrono::Local::now().format("%H:%M:%S").to_string();
        for token in venue_tokens.into_iter().rev() {
            let (is_open, session_label, next_open, next_close) = venue_market_window(&token, now);
            let status = if is_open {
                "AÇIK (takvim)"
            } else {
                "KAPALI (takvim)"
            };
            let next_open_s = next_open
                .map(format_utc_local_ts)
                .unwrap_or_else(|| "—".to_string());
            let next_close_s = next_close
                .map(format_utc_local_ts)
                .unwrap_or_else(|| "yok".to_string());
            logs.insert(
                0,
                LogEntry {
                    timestamp: ts.clone(),
                    message: format!(
                        "🕒 Pazar saatleri [{}] {} · seans={} · sonraki_açılış={} · sonraki_kapanış={}",
                        token,
                        status,
                        session_label,
                        next_open_s,
                        next_close_s,
                    ),
                    level: "INFO".to_string(),
                },
            );
        }
    }

    // 5. AI BEYİN, PİPELİNE VE ANOMALİLER
    // step.last_run_secs aslında Unix epoch saniyesi (record_step çağrılarına
    // last_tick = SystemTime::now()...as_secs() geçiriliyor). UI'da "X saniye önce"
    // yaşı göstermek için şimdiyle farkı alıyoruz; 0 → henüz hiç koşulmadı.
    let now_epoch_secs: u64 = crate::core::time::now_epoch_secs();
    let (steps, mut anomalies): (Vec<PipelineStep>, Vec<AnomalyModel>) = st
        .guardian
        .live_pipeline
        .try_read()
        .ok()
        .map(|ph| {
            let s = ph
                .chain_steps
                .iter()
                .map(|step| PipelineStep {
                    label: step.label.clone(),
                    status: format!("{:?}", step.status),
                    last_run_age_secs: if step.last_run_secs == 0 {
                        0
                    } else {
                        now_epoch_secs.saturating_sub(step.last_run_secs)
                    },
                    overdue_secs: step.overdue_secs as i64,
                })
                .collect();
            let a = ph
                .anomalies
                .iter()
                .map(|anom| AnomalyModel {
                    severity: format!("{:?}", anom.severity),
                    kind: format!("{:?}", anom.kind),
                    message: anom.message.clone(),
                    fix_hint: anom.fix_hint.clone().unwrap_or_default(),
                    auto_fixed: anom.auto_fixed,
                })
                .collect();
            (s, a)
        })
        .unwrap_or_else(|| (vec![], vec![]));
    let (decision_pipeline_steps, runtime_pipeline_steps): (Vec<PipelineStep>, Vec<PipelineStep>) =
        steps.iter().cloned().partition(|step| {
            let mut chars = step.label.trim_start().chars();
            matches!(
                (chars.next(), chars.next()),
                (Some(c), Some('.')) if c.is_ascii_digit()
            )
        });
    let requires_account_readiness = venue_health
        .iter()
        .any(|health| {
            health.watched_symbols > 0
                && health.execution_status == "ENABLED"
                && !st.live_dry_run
        });
    if aggregate_status == memos_venue_api::AccountSnapshotStatus::Unavailable
        && requires_account_readiness
    {
        anomalies.push(AnomalyModel {
            severity: "Critical".to_string(),
            kind: "AccountStateUnavailable".to_string(),
            message: "Portföy/account snapshot mevcut değil; canlı yeni emirler güvenli kabul edilmemeli."
                .to_string(),
            fix_hint: "./memos doctor ve venue account reconcile çalıştır".to_string(),
            auto_fixed: false,
        });
    }
    for position in &positions {
        if position.stop_loss <= 0.0 && position.take_profit <= 0.0 {
            anomalies.push(AnomalyModel {
                severity: "Critical".to_string(),
                kind: "PositionProtectionGap".to_string(),
                message: format!(
                    "{} pozisyonunda broker-side SL/TP görünmüyor; yalnız local exit takibi var.",
                    position.symbol
                ),
                fix_hint: "Broker-side SL/TP oluştur veya yeni canlı emirleri durdur.".to_string(),
                auto_fixed: false,
            });
        }
    }

    let mt5_bridge_healthy = st
        .mt5_bridge_healthy
        .load(std::sync::atomic::Ordering::Relaxed);
    let mt5_bridge_last_error = st
        .mt5_bridge_last_error
        .try_read()
        .map(|err| err.clone())
        .unwrap_or(None);

    // total_trades alanı yeni AppState'de yok; kapanmış işlem sayısı en yakın kaynak.
    let trade_count = st
        .finance
        .live_closed_trades
        .try_read()
        .map(|t| t.len())
        .unwrap_or(0);

    // IntelligenceHub'tan canlı veriler — kullanılamıyorsa muhafazakar varsayılana düş.
    let (
        hub_drift,
        hub_pending,
        hub_cycle,
        hub_state,
        hub_evolution_active,
        hub_failures,
        hub_drift_series,
    ) = st
        .brain
        .intelligence_hub
        .try_read()
        .map(|h| {
            (
                h.drift_detector.drift_score,
                h.pending_trades.len(),
                h.controller.cycle_id,
                h.controller.state.to_string(),
                h.controller.evolution_enabled,
                h.controller.consecutive_failures as u32,
                // Drift tarihçesinin son 60 noktası (AI Center sparkline için).
                h.drift_history
                    .iter()
                    .rev()
                    .take(60)
                    .rev()
                    .cloned()
                    .collect::<Vec<f64>>(),
            )
        })
        .unwrap_or((0.0, 0, 0, "Unknown".into(), false, 0, vec![]));

    // brain.live_strategy "Default"/"Auto" iken motor zaten her cycle'da rejime göre
    // sembol başına dinamik strateji seçiyor (StrategySelector::select_best); tek bir
    // statik isim yansıtmak yanıltıcı olur. Tek-nokta normalize için
    // core::model::normalize_strategy_label kullanılır.
    let live_strategy_name = st
        .brain
        .live_strategy
        .try_read()
        .map(|s| crate::core::model::normalize_strategy_label(&s))
        .unwrap_or_else(|_| "—".to_string());
    let best_tp_pct = st
        .brain
        .best_params
        .get("take_profit_pct")
        .copied()
        .unwrap_or(0.0);
    let best_sl_pct = st
        .brain
        .best_params
        .get("stop_loss_pct")
        .copied()
        .unwrap_or(0.0);
    let best_position_size = st
        .brain
        .best_params
        .get("max_position_size")
        .copied()
        .unwrap_or(0.0);

    let ai_brain = AiBrainSnapshot {
        genome_id: format!(
            "Srivastava-Alpha-9 [cycle={} · ctrl={}]",
            hub_cycle, hub_state
        ),
        fitness: finance.total_equity,
        win_rate: charts.distributions.iter().map(|d| d.win_rate).sum::<f64>()
            / charts.distributions.len().max(1) as f64,
        trade_count,
        gbt_score: Some(st.brain.hyperopt_score),
        exploration_rate: 0.1,
        drift_score: hub_drift,
        mc_ruin_prob: 0.01,
        is_evolution_active: hub_evolution_active,
        next_evolution_secs: 300_u64.saturating_sub(hub_cycle % 300),
        live_strategy: live_strategy_name,
        controller_state: hub_state,
        controller_cycle: hub_cycle,
        consecutive_failures: hub_failures,
        pending_trades: hub_pending,
        drift_series: hub_drift_series,
        best_tp_pct,
        best_sl_pct,
        best_position_size,
    };

    // 6. ÖZEL İSTATİSTİKLER — closed_trades'ten holding period'a göre Scalp/Swing ayrımı.
    //    Eşik Faz 2 ParameterStore'dan okunuyor (önce env, runtime'da HyperOpt güncelleyebilir).
    //    opened_at/closed_at parse edilemeyen veya boş kayıtlar atlanır.
    let scalp_swing_threshold_min = st
        .brain
        .parameters
        .try_read()
        .map(|p| p.scalp_swing_threshold_min)
        .unwrap_or(60);
    let (scalp_stats, swing_stats) = st
        .finance
        .live_closed_trades
        .try_read()
        .ok()
        .map(|trades| compute_scalp_swing_stats(&trades, scalp_swing_threshold_min))
        .unwrap_or_else(|| {
            (
                TradeTypeStats {
                    label: "SCALP".into(),
                    win_rate: 0.0,
                    profit_factor: 0.0,
                    avg_win: 0.0,
                    avg_loss: 0.0,
                    current_streak: 0,
                },
                TradeTypeStats {
                    label: "SWING".into(),
                    win_rate: 0.0,
                    profit_factor: 0.0,
                    avg_win: 0.0,
                    avg_loss: 0.0,
                    current_streak: 0,
                },
            )
        });

    // anomalies aşağıda MissionControl'a move edilmeden önce sayıyı + tip
    // dağılımını yakala. boot anomaly 50 olduğunda hangi tipin baskın olduğu
    // görünür (TUI Risk Center).
    let active_anomalies = anomalies.len();
    let mut anomalies_by_kind: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for a in &anomalies {
        *anomalies_by_kind.entry(a.kind.clone()).or_insert(0) += 1;
    }
    let mut autonomy_counters = st.fleet.autonomy_counters.clone();
    if let Ok(decisions) = st.fleet.latest_decisions.read() {
        let mut evaluated = 0_u64;
        let mut aligned = 0_u64;
        let mut divergent = 0_u64;
        let mut insufficient = 0_u64;
        let mut errors = 0_u64;
        for audit in decisions.values() {
            match audit.shadow_status.as_str() {
                "EVALUATED" => {
                    evaluated += 1;
                    if audit.shadow_signal == audit.signal {
                        aligned += 1;
                    } else {
                        divergent += 1;
                    }
                }
                "INSUFFICIENT_DATA" => insufficient += 1,
                "DATA_ERROR" | "STRATEGY_ERROR" => errors += 1,
                _ => {}
            }
        }
        autonomy_counters.insert("shadow.current_evaluated".to_string(), evaluated);
        autonomy_counters.insert("shadow.current_aligned".to_string(), aligned);
        autonomy_counters.insert("shadow.current_divergent".to_string(), divergent);
        autonomy_counters.insert("shadow.current_insufficient_data".to_string(), insufficient);
        autonomy_counters.insert("shadow.current_errors".to_string(), errors);
    }
    let indicator_health = compute_indicator_health(&autonomy_counters);
    let symbolic_health = compute_symbolic_health(&autonomy_counters);
    let repair_log: Vec<String> = st
        .guardian
        .repair_log
        .iter()
        .rev()
        .take(50)
        .cloned()
        .collect();
    let venue_runtime = st
        .venue_runtime
        .try_read()
        .map(|runtimes| {
            runtimes
                .iter()
                .map(|(venue, runtime)| {
                    (
                        venue.clone(),
                        VenueRuntimeSnapshot {
                            scheduler_phase: runtime.scheduler_phase.clone(),
                            scheduler_cycle_started_epoch: runtime.scheduler_cycle_started_epoch,
                            scheduler_cycle_completed_epoch: runtime
                                .scheduler_cycle_completed_epoch,
                            consecutive_failures: runtime.consecutive_failures,
                            last_error: runtime.last_error.clone(),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let runtime_lock_diagnostic = st
        .runtime_lock_diagnostic
        .read()
        .map(|diagnostic| RuntimeLockDiagnosticSnapshot {
            owner: diagnostic.owner.clone(),
            phase: diagnostic.phase.clone(),
            started_epoch: diagnostic.started_epoch,
        })
        .unwrap_or_default();

    MissionControl {
        finance,
        positions,
        ai_brain,
        phase: st.fleet.phase.clone(),
        pipeline_steps: steps,
        decision_pipeline_steps,
        runtime_pipeline_steps,
        anomalies,
        repair_log,
        scalp_stats,
        swing_stats,
        logs,
        trade_history,
        market_fleet,
        charts,
        fleet,
        active_anomalies,
        anomalies_by_kind,
        autonomy_counters,
        indicator_health,
        symbolic_health,
        opportunity_snapshots,
        data_coverage,
        download_progress,
        bist_data_status,
        decision_trace,
        venue_health,
        strategy_performance,
        pyramid_performance,
        mt5_bridge_healthy,
        mt5_bridge_last_error,
        venue_accounts,
        venue_runtime,
        runtime_lock_diagnostic,
        venue_portfolios,
        venue_symbols,
    }
}

fn build_venue_symbols(
    st: &AppState,
    market_fleet: &[MarketAnalysisModel],
    health: &[VenueHealthModel],
) -> Vec<VenueSymbolSnapshot> {
    let mut candidates = market_fleet.to_vec();
    for (env_key, venue, market) in [
        ("AUTO_BINANCE_SYMBOLS", "binance", "futures"),
        ("AUTO_MT5_SYMBOLS", "mt5", "cfd"),
        ("AUTO_IBKR_SYMBOLS", "ibkr", "cfd"),
        ("AUTO_BIST_SYMBOLS", "bist", "spot"),
    ] {
        if let Ok(raw) = std::env::var(env_key) {
            for bare in raw
                .split(',')
                .map(str::trim)
                .filter(|symbol| !symbol.is_empty())
            {
                let tagged = format!("{bare}@{venue}");
                if !candidates.iter().any(|item| item.symbol == tagged) {
                    candidates.push(MarketAnalysisModel {
                        symbol: tagged,
                        market: market.into(),
                        current_price: 0.0,
                        change_24h: 0.0,
                        zones: vec![],
                        nearest_support: None,
                        nearest_resistance: None,
                    });
                }
            }
        }
    }
    let mut catalog = candidates
        .iter()
        .filter_map(|item| {
            let (symbol, venue) = item
                .symbol
                .rsplit_once('@')
                .unwrap_or((item.symbol.as_str(), "unknown"));
            if venue == "unknown" {
                return None;
            }
            let venue = venue.to_ascii_lowercase();
            let key = match venue.as_str() {
                "binance" => "binance:futures",
                "mt5" => "mt5:cfd",
                "ibkr" => "ibkr:cfd",
                "bist" => "bist:spot",
                _ => venue.as_str(),
            }
            .to_owned();
            let health_item = health
                .iter()
                .find(|entry| entry.venue.eq_ignore_ascii_case(&venue));
            let candles = read_candles_for_symbol(st, &item.symbol, &st.config.interval, 1);
            let last_candle_at = candles.last().map(|candle| candle.timestamp);
            let candle_status = match last_candle_at {
                Some(timestamp) => {
                    let age = (chrono::Utc::now() - timestamp).num_seconds();
                    if age <= 2 * 60 * 60 {
                        "FRESH"
                    } else {
                        "STALE"
                    }
                }
                None => "MISSING",
            };
            let is_bist = venue == "bist";
            Some(VenueSymbolSnapshot {
                venue: key,
                symbol: symbol.to_owned(),
                market: item.market.clone(),
                status: if is_bist { "DATA_ONLY" } else { "WATCHED" }.into(),
                tradeable: !is_bist,
                current_price: item.current_price,
                change_24h: item.change_24h,
                last_candle_at,
                candle_status: candle_status.into(),
                filters_status: health_item
                    .map(|entry| entry.symbol_filters_status.clone())
                    .unwrap_or_else(|| "UNKNOWN".into()),
                source: if is_bist {
                    "bist-cache".into()
                } else {
                    "venue-adapter".into()
                },
            })
        })
        .collect::<Vec<_>>();
    catalog.sort_by(|left, right| {
        left.venue
            .cmp(&right.venue)
            .then(left.symbol.cmp(&right.symbol))
    });
    catalog.dedup_by(|left, right| left.venue == right.venue && left.symbol == right.symbol);
    catalog
}

fn venue_scope(raw: &str) -> String {
    raw.split(':')
        .next()
        .unwrap_or(raw)
        .trim()
        .to_ascii_lowercase()
}

fn build_venue_portfolios(
    accounts: &std::collections::BTreeMap<String, memos_venue_api::AccountSnapshot>,
    positions: &[PositionModel],
    trades: &[ClosedTradeModel],
) -> Vec<VenuePortfolioSnapshot> {
    let mut scopes = std::collections::BTreeSet::new();
    for key in accounts.keys() {
        scopes.insert(venue_scope(key));
    }
    for position in positions {
        if let Some(venue) = &position.venue {
            scopes.insert(venue_scope(&venue.token()));
        }
    }
    for trade in trades {
        if !trade.venue.trim().is_empty() {
            scopes.insert(venue_scope(&trade.venue));
        }
    }

    scopes
        .into_iter()
        .map(|venue| {
            let account_keys = accounts
                .keys()
                .filter(|key| venue_scope(key) == venue)
                .cloned()
                .collect::<Vec<_>>();
            let balance = account_keys
                .iter()
                .filter_map(|key| accounts.get(key).map(|account| account.balance))
                .sum::<f64>();
            let equity_values = account_keys
                .iter()
                .filter_map(|key| accounts.get(key).and_then(|account| account.equity))
                .collect::<Vec<_>>();
            let position_count = positions
                .iter()
                .filter(|position| {
                    position
                        .venue
                        .as_ref()
                        .map(|key| venue_scope(&key.token()) == venue)
                        .unwrap_or(false)
                })
                .count();
            let venue_trades = trades
                .iter()
                .filter(|trade| venue_scope(&trade.venue) == venue)
                .collect::<Vec<_>>();
            let open_pnl = positions
                .iter()
                .filter(|position| {
                    position
                        .venue
                        .as_ref()
                        .map(|key| venue_scope(&key.token()) == venue)
                        .unwrap_or(false)
                })
                .map(PositionModel::calculate_pnl)
                .sum();
            let realized_pnl = venue_trades
                .iter()
                .map(|trade| trade.effective_net_pnl())
                .sum();
            let total_fees = venue_trades.iter().map(|trade| trade.commission).sum();
            let has_accounts = !account_keys.is_empty();
            VenuePortfolioSnapshot {
                venue,
                account_keys,
                balance: if has_accounts { Some(balance) } else { None },
                equity: if equity_values.is_empty() {
                    None
                } else {
                    Some(equity_values.into_iter().sum())
                },
                open_pnl,
                realized_pnl,
                total_fees,
                position_count,
                closed_trade_count: venue_trades.len(),
            }
        })
        .collect()
}

fn compute_pyramid_performance(trades: &[ClosedTradeModel]) -> Vec<PyramidPerformanceModel> {
    let mut groups: std::collections::BTreeMap<u8, (u32, f64, u32)> =
        std::collections::BTreeMap::new();
    for trade in trades {
        let net_pnl = trade.effective_net_pnl();
        let entry = groups
            .entry(trade.pyramid_tranches.max(1))
            .or_insert((0, 0.0, 0));
        entry.0 += 1;
        entry.1 += net_pnl;
        if net_pnl > 0.0 {
            entry.2 += 1;
        }
    }
    groups
        .into_iter()
        .map(
            |(tranches, (trades, net_pnl, wins))| PyramidPerformanceModel {
                tranches,
                trades,
                net_pnl,
                win_rate: wins as f64 / trades.max(1) as f64 * 100.0,
            },
        )
        .collect()
}

fn compute_strategy_performance(trades: &[ClosedTradeModel]) -> Vec<StrategyPerformanceModel> {
    let mut groups: std::collections::BTreeMap<(String, String), (u32, f64, u32, f64)> =
        std::collections::BTreeMap::new();
    for trade in trades {
        let net_pnl = trade.effective_net_pnl();
        let venue = if trade.venue.is_empty() {
            crate::robot::venue::registry::split_venue_tag(&trade.symbol)
                .1
                .unwrap_or_else(|| crate::core::types::Exchange::classify(&trade.symbol))
                .as_str()
                .to_string()
        } else {
            trade.venue.clone()
        };
        let strategy = if trade.strategy.is_empty() {
            "legacy".to_string()
        } else {
            trade.strategy.clone()
        };
        let entry = groups.entry((venue, strategy)).or_insert((0, 0.0, 0, 0.0));
        entry.0 += 1;
        entry.1 += net_pnl;
        if net_pnl > 0.0 {
            entry.2 += 1;
        } else {
            entry.3 += net_pnl.abs();
        }
    }
    groups
        .into_iter()
        .map(
            |((venue, strategy), (trades, net_pnl, wins, losses))| StrategyPerformanceModel {
                venue,
                strategy,
                trades,
                net_pnl,
                win_rate: wins as f64 / trades.max(1) as f64 * 100.0,
                profit_factor: if losses > 0.0 {
                    (net_pnl.max(0.0) + losses) / losses
                } else {
                    0.0
                },
            },
        )
        .collect()
}

fn parse_decision_trace(
    log: &LogEntry,
    venue_registry: &crate::robot::venue::registry::VenueRegistry,
) -> Option<DecisionTraceModel> {
    let message = log.message.as_str();
    let is_decision = message.contains("REDDEDİLDİ")
        || message.contains("POZİSYON AÇILIYOR")
        || message.contains("RiskManager")
        || message.contains("kademe ");
    if !is_decision {
        return None;
    }
    let symbol = message
        .split_whitespace()
        .find(|part| {
            part.contains('@')
                || part
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
        })
        .unwrap_or("—")
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '@')
        .to_string();
    let outcome = if message.contains("POZİSYON AÇILIYOR") || message.contains("kademe ") {
        "KABUL".to_string()
    } else {
        "RED".to_string()
    };
    let strategy = extract_trace_value(message, "strat=").unwrap_or_else(|| "—".to_string());
    let regime = extract_trace_value(message, "rejim=").unwrap_or_else(|| "—".to_string());
    let edge = extract_trace_value(message, "edge=").and_then(|value| value.parse::<f64>().ok());
    let venue_profile = crate::robot::engines::master::venue_profile::VenueProfile::for_symbol(
        &symbol,
        venue_registry,
    );
    Some(DecisionTraceModel {
        timestamp: log.timestamp.clone(),
        symbol,
        outcome,
        strategy,
        regime,
        edge,
        pyramid_limit: venue_profile.max_pyramid_tranches,
        max_atr_pct: venue_profile.max_atr_pct,
        reason: message.to_string(),
        shadow_signal: String::new(),
        shadow_status: String::new(),
        shadow_reason: String::new(),
    })
}

fn extract_trace_value(message: &str, key: &str) -> Option<String> {
    let value = message.split_once(key)?.1;
    let end = value
        .find(|ch: char| ch == ')' || ch == ',' || ch.is_whitespace())
        .unwrap_or(value.len());
    let value = value[..end].trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// `indicator.<tag>.<suffix>` sayaçlarından 0..100 aralığında sağlık skoru üretir.
///
/// Skor bileşenleri:
/// - kalite: win/(win+loss) (Laplace smoothing ile)
/// - akış sağlığı: 1 - gated/(used+gated)
///
/// Nihai skor = 0.6*kalite + 0.4*akış, sonra 0..100'e ölçeklenir.
fn compute_indicator_health(
    counters: &std::collections::BTreeMap<String, u64>,
) -> std::collections::BTreeMap<String, f64> {
    #[derive(Default)]
    struct Stat {
        used: u64,
        gated: u64,
        win: u64,
        loss: u64,
    }

    let mut agg: std::collections::BTreeMap<String, Stat> = std::collections::BTreeMap::new();
    for (key, val) in counters {
        if *val == 0 {
            continue;
        }
        let Some(rest) = key.strip_prefix("indicator.") else {
            continue;
        };
        let Some((tag, suffix)) = rest.rsplit_once('.') else {
            continue;
        };
        let s = agg.entry(tag.to_string()).or_default();
        if suffix == "used" {
            s.used = s.used.saturating_add(*val);
        } else if suffix == "win_contrib" {
            s.win = s.win.saturating_add(*val);
        } else if suffix == "loss_contrib" {
            s.loss = s.loss.saturating_add(*val);
        } else if suffix.starts_with("gated_") {
            s.gated = s.gated.saturating_add(*val);
        }
    }

    let mut out: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();
    for (tag, s) in agg {
        let q_num = (s.win + 1) as f64;
        let q_den = (s.win + s.loss + 2) as f64;
        let quality = q_num / q_den;

        let flow_den = (s.used + s.gated + 1) as f64;
        let flow = 1.0 - (s.gated as f64 / flow_den);

        let score = ((0.6 * quality + 0.4 * flow) * 100.0).clamp(0.0, 100.0);
        out.insert(tag, score);
    }
    out
}

/// `symbolic.*` sayaçlarından A.5.2 özet sağlık metrikleri üretir (0..100).
///
/// Üretilen anahtarlar:
/// - coverage_pct: detected/scan
/// - alignment_pct: aligned/(aligned+conflict)
/// - conflict_pct: conflict/(aligned+conflict)
/// - confidence_score: coverage ve uyumdan türetilen birleşik skor
/// - would_block_pct: compare aşamasında "gate olsaydı blok" oranı
/// - gated_pct: gate açıkken fiilen bloklanan oran
/// - out_of_scope_pct: gate açıkken rollout kapsamı dışına düşen conflict oranı
/// - adaptive_applied_pct / adaptive_tightened_pct / adaptive_relaxed_pct
fn compute_symbolic_health(
    counters: &std::collections::BTreeMap<String, u64>,
) -> std::collections::BTreeMap<String, f64> {
    let scan = counters.get("symbolic.scan").copied().unwrap_or(0) as f64;
    let detected = counters.get("symbolic.detected").copied().unwrap_or(0) as f64;
    let aligned_buy = counters
        .get("symbolic.signal.aligned_buy")
        .copied()
        .unwrap_or(0) as f64;
    let aligned_sell = counters
        .get("symbolic.signal.aligned_sell")
        .copied()
        .unwrap_or(0) as f64;
    let conflict = counters
        .get("symbolic.signal.conflict")
        .copied()
        .unwrap_or(0) as f64;
    let would_block = counters
        .get("symbolic.compare.would_block")
        .copied()
        .unwrap_or(0) as f64;
    let gated = counters.get("symbolic.gated").copied().unwrap_or(0) as f64;
    let out_of_scope = counters
        .get("symbolic.compare.out_of_scope")
        .copied()
        .unwrap_or(0) as f64;
    let adaptive_applied = counters
        .get("symbolic.adaptive.applied")
        .copied()
        .unwrap_or(0) as f64;
    let adaptive_tightened = counters
        .get("symbolic.adaptive.tightened")
        .copied()
        .unwrap_or(0) as f64;
    let adaptive_relaxed = counters
        .get("symbolic.adaptive.relaxed")
        .copied()
        .unwrap_or(0) as f64;

    let aligned = aligned_buy + aligned_sell;
    let directional_total = aligned + conflict;

    let coverage = if scan > 0.0 { detected / scan } else { 0.0 };
    let alignment = if directional_total > 0.0 {
        aligned / directional_total
    } else {
        0.0
    };
    let conflict_ratio = if directional_total > 0.0 {
        conflict / directional_total
    } else {
        0.0
    };
    let would_block_ratio = if directional_total > 0.0 {
        would_block / directional_total
    } else {
        0.0
    };
    let gated_ratio = if directional_total > 0.0 {
        gated / directional_total
    } else {
        0.0
    };
    let out_of_scope_ratio = if directional_total > 0.0 {
        out_of_scope / directional_total
    } else {
        0.0
    };
    let adaptive_applied_ratio = if scan > 0.0 {
        adaptive_applied / scan
    } else {
        0.0
    };
    let adaptive_tightened_ratio = if adaptive_applied > 0.0 {
        adaptive_tightened / adaptive_applied
    } else {
        0.0
    };
    let adaptive_relaxed_ratio = if adaptive_applied > 0.0 {
        adaptive_relaxed / adaptive_applied
    } else {
        0.0
    };

    let confidence_score = ((0.45 * coverage) + (0.55 * alignment)) * 100.0;

    let mut out = std::collections::BTreeMap::new();
    out.insert(
        "coverage_pct".to_string(),
        (coverage * 100.0).clamp(0.0, 100.0),
    );
    out.insert(
        "alignment_pct".to_string(),
        (alignment * 100.0).clamp(0.0, 100.0),
    );
    out.insert(
        "conflict_pct".to_string(),
        (conflict_ratio * 100.0).clamp(0.0, 100.0),
    );
    out.insert(
        "confidence_score".to_string(),
        confidence_score.clamp(0.0, 100.0),
    );
    out.insert(
        "would_block_pct".to_string(),
        (would_block_ratio * 100.0).clamp(0.0, 100.0),
    );
    out.insert(
        "gated_pct".to_string(),
        (gated_ratio * 100.0).clamp(0.0, 100.0),
    );
    out.insert(
        "out_of_scope_pct".to_string(),
        (out_of_scope_ratio * 100.0).clamp(0.0, 100.0),
    );
    out.insert(
        "adaptive_applied_pct".to_string(),
        (adaptive_applied_ratio * 100.0).clamp(0.0, 100.0),
    );
    out.insert(
        "adaptive_tightened_pct".to_string(),
        (adaptive_tightened_ratio * 100.0).clamp(0.0, 100.0),
    );
    out.insert(
        "adaptive_relaxed_pct".to_string(),
        (adaptive_relaxed_ratio * 100.0).clamp(0.0, 100.0),
    );
    out
}

/// Log satırının başındaki `[HH:MM:SS]` prefix'ini ayırır. Prefix yoksa
/// timestamp boş döner, message tüm satırı kapsar. Saf, side-effect yok.
fn parse_log_prefix(line: &str) -> (String, String) {
    // Beklenen format: "[HH:MM:SS] message" (push_log böyle yazıyor).
    let bytes = line.as_bytes();
    if bytes.len() >= 11 && bytes[0] == b'[' && bytes[9] == b']' && bytes[10] == b' ' {
        let ts = &line[1..9];
        if ts.bytes().all(|b| b.is_ascii_digit() || b == b':') {
            return (ts.to_string(), line[11..].to_string());
        }
    }
    (String::new(), line.to_string())
}

fn parse_venue_symbols_from_env(key: &str, exchange: crate::core::types::Exchange) -> Vec<String> {
    let raw = std::env::var(key).unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for tok in raw.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let (bare, tag) = crate::robot::venue::registry::split_venue_tag(tok);
        if tag == Some(exchange) {
            let b = bare.to_string();
            if !out.contains(&b) {
                out.push(b);
            }
        }
    }
    out
}

fn parse_mt5_symbols_from_env() -> Vec<String> {
    parse_venue_symbols_from_env("AUTO_MT5_SYMBOLS", crate::core::types::Exchange::Mt5)
}

fn push_unique_market_analysis(
    market_fleet: &mut Vec<MarketAnalysisModel>,
    item: MarketAnalysisModel,
) {
    if market_fleet
        .iter()
        .any(|m| m.symbol == item.symbol && m.market == item.market)
    {
        return;
    }
    market_fleet.push(item);
}

fn monitor_change_24h(candles: &[crate::core::types::Candle], current_price: f64) -> f64 {
    let Some(latest) = candles.last() else {
        return 0.0;
    };
    if current_price <= 0.0 {
        return 0.0;
    }
    let cutoff = latest.timestamp - chrono::Duration::hours(24);
    let reference = candles
        .iter()
        .rev()
        .find(|candle| candle.timestamp <= cutoff)
        .map(|candle| candle.close)
        .filter(|price| *price > 0.0);
    reference
        .map(|price| (current_price - price) / price * 100.0)
        .unwrap_or(0.0)
}

fn mt5_freshness_window(now: chrono::DateTime<chrono::Utc>) -> (i64, &'static str) {
    match chrono::Datelike::weekday(&now) {
        chrono::Weekday::Sat | chrono::Weekday::Sun => (72 * 60, "72saat/hafta-sonu"),
        _ => (3 * 60, "3saat"),
    }
}

fn configured_venues_from_env(fallback_market: &str) -> Vec<String> {
    let raw = std::env::var("VENUES").unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for tok in raw.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let normalized = tok.to_lowercase();
        if !out.contains(&normalized) {
            out.push(normalized);
        }
    }
    if out.is_empty() {
        out.push(format!("binance:{}", fallback_market.to_lowercase()));
    }
    out
}

fn venue_market_window(
    token: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> (
    bool,
    &'static str,
    Option<chrono::DateTime<chrono::Utc>>,
    Option<chrono::DateTime<chrono::Utc>>,
) {
    let t = token.to_lowercase();
    if t == "mt5" || t.starts_with("mt5:") {
        // MT5 FX/CFD genel seansı: Paz 21:00 UTC açılış, Cum 21:00 UTC kapanış.
        // (Sembol-bazlı istisnalar brokera göre değişebilir; bu genel pencereyi loglarız.)
        let sunday_open = chrono::Weekday::Sun;
        let friday_close = chrono::Weekday::Fri;
        let minute_of_week = chrono::Datelike::weekday(&now).num_days_from_sunday() as i64 * 1440
            + chrono::Timelike::hour(&now) as i64 * 60
            + chrono::Timelike::minute(&now) as i64;
        let open_min = 21 * 60; // Pazar 21:00
        let close_min = 5 * 1440 + 21 * 60; // Cuma 21:00
        let is_open = minute_of_week >= open_min && minute_of_week < close_min;

        if is_open {
            let next_close = next_weekday_time_utc(now, friday_close, 21, 0);
            (true, "Paz 21:00 - Cum 21:00 UTC", None, Some(next_close))
        } else {
            let next_open = next_weekday_time_utc(now, sunday_open, 21, 0);
            let next_close = next_open + chrono::Duration::days(5);
            (
                false,
                "Paz 21:00 - Cum 21:00 UTC",
                Some(next_open),
                Some(next_close),
            )
        }
    } else {
        // Binance/Bybit ve çoğu kripto marketi 7/24 kabul edilir.
        (true, "7/24 (UTC 00:00-24:00)", None, None)
    }
}

fn next_weekday_time_utc(
    now: chrono::DateTime<chrono::Utc>,
    weekday: chrono::Weekday,
    hour: u32,
    minute: u32,
) -> chrono::DateTime<chrono::Utc> {
    let now_w = chrono::Datelike::weekday(&now).num_days_from_monday() as i64;
    let target_w = weekday.num_days_from_monday() as i64;
    let mut days_ahead = (target_w - now_w + 7) % 7;
    let today_target = now
        .date_naive()
        .and_hms_opt(hour, minute, 0)
        .expect("valid time");
    if days_ahead == 0 && now.naive_utc() >= today_target {
        days_ahead = 7;
    }
    let date = now.date_naive() + chrono::Duration::days(days_ahead);
    let naive = date.and_hms_opt(hour, minute, 0).expect("valid time");
    chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc)
}

fn format_utc_local_ts(ts: chrono::DateTime<chrono::Utc>) -> String {
    let local = ts.with_timezone(&chrono::Local);
    format!(
        "{} / {}",
        ts.format("%Y-%m-%d %H:%M UTC"),
        local.format("%Y-%m-%d %H:%M %:z"),
    )
}

/// Closed trades listesini holding period'a göre Scalp/Swing'e ayırıp her grup için
/// (win_rate, profit_factor, avg_win, avg_loss, current_streak) hesaplar.
///
/// - `threshold_min`: Scalp/Swing ayrım eşiği (dakika). Holding < eşik → Scalp.
///   Faz 2 öncesi env `SCALP_SWING_THRESHOLD_MIN`'den okunuyordu; artık ParameterStore'dan
///   geliyor, ama unit testler kolay olsun diye parametre olarak geçiyor.
/// - opened_at/closed_at boş veya parse edilemeyen kayıtlar atlanır (eski şema toleransı).
/// - current_streak: en son kayıttan başlayarak ardışık aynı yön (kâr/zarar) sayısı,
///   kâr → pozitif, zarar → negatif. Yön değişince durur.
fn compute_scalp_swing_stats(
    trades: &[ClosedTradeModel],
    threshold_min: i64,
) -> (TradeTypeStats, TradeTypeStats) {
    let threshold_secs = threshold_min.saturating_mul(60);

    // Holding period'u hesapla, geçersiz kayıtları atla.
    let with_holding: Vec<(&ClosedTradeModel, i64)> = trades
        .iter()
        .filter_map(|t| {
            if t.opened_at.is_empty() || t.closed_at.is_empty() {
                return None;
            }
            let o = chrono::DateTime::parse_from_rfc3339(&t.opened_at).ok()?;
            let c = chrono::DateTime::parse_from_rfc3339(&t.closed_at).ok()?;
            let secs = (c - o).num_seconds();
            if secs < 0 {
                return None;
            } // bozuk sıralama
            Some((t, secs))
        })
        .collect();

    let mut scalp: Vec<&ClosedTradeModel> = vec![];
    let mut swing: Vec<&ClosedTradeModel> = vec![];
    for (t, h) in &with_holding {
        if *h < threshold_secs {
            scalp.push(*t);
        } else {
            swing.push(*t);
        }
    }
    (
        stats_for_group("SCALP", &scalp),
        stats_for_group("SWING", &swing),
    )
}

fn stats_for_group(label: &str, group: &[&ClosedTradeModel]) -> TradeTypeStats {
    let n = group.len();
    if n == 0 {
        return TradeTypeStats {
            label: label.into(),
            win_rate: 0.0,
            profit_factor: 0.0,
            avg_win: 0.0,
            avg_loss: 0.0,
            current_streak: 0,
        };
    }
    let wins: Vec<f64> = group
        .iter()
        .map(|t| t.effective_net_pnl())
        .filter(|pnl| *pnl > 0.0)
        .collect();
    let losses: Vec<f64> = group
        .iter()
        .map(|t| t.effective_net_pnl())
        .filter(|pnl| *pnl < 0.0)
        .map(f64::abs)
        .collect();
    let sum_wins: f64 = wins.iter().sum();
    let sum_losses: f64 = losses.iter().sum();

    let win_rate = wins.len() as f64 / n as f64 * 100.0;
    // Loss yokken sonsuz olur; UI/JSON'da NaN/inf üretmemek için 100.0'a cap'liyoruz.
    let profit_factor = if sum_losses > 0.0 {
        (sum_wins / sum_losses).min(100.0)
    } else if sum_wins > 0.0 {
        100.0
    } else {
        0.0
    };
    let avg_win = if !wins.is_empty() {
        sum_wins / wins.len() as f64
    } else {
        0.0
    };
    let avg_loss = if !losses.is_empty() {
        sum_losses / losses.len() as f64
    } else {
        0.0
    };

    // Streak: en son trade'den geriye, aynı yönde gittiği sürece say.
    let mut streak: i32 = 0;
    if let Some(last) = group.last() {
        let last_sign = last.effective_net_pnl().signum();
        if last_sign != 0.0 {
            for t in group.iter().rev() {
                if t.effective_net_pnl().signum() == last_sign {
                    streak += if last_sign > 0.0 { 1 } else { -1 };
                } else {
                    break;
                }
            }
        }
    }

    TradeTypeStats {
        label: label.into(),
        win_rate,
        profit_factor,
        avg_win,
        avg_loss,
        current_streak: streak,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_trace_parses_entry_fields() {
        let log = LogEntry {
            timestamp: "12:34:56".into(),
            level: "INFO".into(),
            message: "📊 BTCUSDT@binance BUY edge=0.67 ✓ + risk ✓ ⇒ POZİSYON AÇILIYOR (strat=EMA_CROSSOVER)".into(),
        };
        let reg = crate::robot::venue::registry::VenueRegistry::from_specs(&[], None);
        let trace = parse_decision_trace(&log, &reg).expect("entry trace");
        assert_eq!(trace.symbol, "BTCUSDT@binance");
        assert_eq!(trace.outcome, "KABUL");
        assert_eq!(trace.strategy, "EMA_CROSSOVER");
        assert_eq!(trace.edge, Some(0.67));
    }

    #[test]
    fn decision_trace_parses_regime_rejection() {
        let log = LogEntry {
            timestamp: "12:34:56".into(),
            level: "INFO".into(),
            message: "🧭 XLE@ibkr BUY ⇒ REDDEDİLDİ (rejim-yön teyidi yok, rejim=WeakDowntrend)"
                .into(),
        };
        let reg = crate::robot::venue::registry::VenueRegistry::from_specs(&[], None);
        let trace = parse_decision_trace(&log, &reg).expect("rejection trace");
        assert_eq!(trace.symbol, "XLE@ibkr");
        assert_eq!(trace.outcome, "RED");
        assert_eq!(trace.regime, "WeakDowntrend");
    }

    #[test]
    fn market_fleet_deduplicates_symbol_reentry() {
        let entry = MarketAnalysisModel {
            symbol: "EURUSD@mt5".to_string(),
            market: "mt5-monitor".to_string(),
            current_price: 1.12,
            change_24h: 0.0,
            zones: vec![],
            nearest_support: None,
            nearest_resistance: None,
        };
        let mut fleet = vec![MarketAnalysisModel {
            symbol: "EURUSD".to_string(),
            market: "market".to_string(),
            current_price: 1.12,
            change_24h: 0.0,
            zones: vec![],
            nearest_support: None,
            nearest_resistance: None,
        }];

        push_unique_market_analysis(&mut fleet, entry.clone());
        push_unique_market_analysis(&mut fleet, entry);

        assert_eq!(fleet.iter().filter(|m| m.symbol == "EURUSD@mt5").count(), 1);
    }

    #[test]
    fn strategy_performance_groups_by_venue_and_strategy() {
        let mut binance = t("2026-08-27T00:00:00Z", "2026-08-27T01:00:00Z", 10.0);
        binance.venue = "binance".into();
        binance.strategy = "EMA_CROSSOVER".into();
        let mut ibkr = t("2026-08-27T00:00:00Z", "2026-08-27T01:00:00Z", -4.0);
        ibkr.venue = "ibkr".into();
        ibkr.strategy = "MA_CROSSOVER".into();
        let matrix = compute_strategy_performance(&[binance, ibkr]);
        assert_eq!(matrix.len(), 2);
        assert!(matrix
            .iter()
            .any(|row| row.venue == "binance" && row.net_pnl == 10.0));
        assert!(matrix
            .iter()
            .any(|row| row.venue == "ibkr" && row.net_pnl == -4.0));
    }

    #[test]
    fn pyramid_performance_groups_by_filled_tranches() {
        let mut single = t("2026-08-27T00:00:00Z", "2026-08-27T01:00:00Z", 3.0);
        single.pyramid_tranches = 1;
        let mut pyramid = t("2026-08-27T00:00:00Z", "2026-08-27T01:00:00Z", 8.0);
        pyramid.pyramid_tranches = 3;
        let rows = compute_pyramid_performance(&[single, pyramid]);
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .any(|row| row.tranches == 3 && row.net_pnl == 8.0));
    }

    #[test]
    fn parse_log_prefix_extracts_timestamp_and_strips_brackets() {
        let (ts, msg) = parse_log_prefix("[12:34:56] 🚀 Devriye başladı");
        assert_eq!(ts, "12:34:56");
        assert_eq!(msg, "🚀 Devriye başladı");
    }

    #[test]
    fn parse_log_prefix_without_bracket_returns_empty_timestamp() {
        let (ts, msg) = parse_log_prefix("plain line without prefix");
        assert_eq!(ts, "");
        assert_eq!(msg, "plain line without prefix");
    }

    #[test]
    fn parse_log_prefix_rejects_non_digit_inside_brackets() {
        // "[ABCDEFGH]" yapısal eşleşse de digit/`:` değil → fallback yol.
        let (ts, msg) = parse_log_prefix("[ABCDEFGH] hello");
        assert_eq!(ts, "");
        assert_eq!(msg, "[ABCDEFGH] hello");
    }

    #[test]
    fn parse_log_prefix_handles_empty_line() {
        let (ts, msg) = parse_log_prefix("");
        assert_eq!(ts, "");
        assert_eq!(msg, "");
    }

    #[test]
    fn mt5_freshness_window_is_wider_on_weekend() {
        let sat = chrono::DateTime::parse_from_rfc3339("2026-07-18T12:00:00Z")
            .expect("parse sat")
            .with_timezone(&chrono::Utc);
        let (mins, label) = mt5_freshness_window(sat);
        assert_eq!(mins, 72 * 60);
        assert_eq!(label, "72saat/hafta-sonu");
    }

    #[test]
    fn mt5_freshness_window_is_strict_on_weekday() {
        let mon = chrono::DateTime::parse_from_rfc3339("2026-07-20T12:00:00Z")
            .expect("parse mon")
            .with_timezone(&chrono::Utc);
        let (mins, label) = mt5_freshness_window(mon);
        assert_eq!(mins, 3 * 60);
        assert_eq!(label, "3saat");
    }

    #[test]
    fn venue_market_window_mt5_is_closed_on_saturday() {
        let sat = chrono::DateTime::parse_from_rfc3339("2026-07-18T12:00:00Z")
            .expect("parse sat")
            .with_timezone(&chrono::Utc);
        let (is_open, session, next_open, next_close) = venue_market_window("mt5:cfd", sat);
        assert!(!is_open);
        assert_eq!(session, "Paz 21:00 - Cum 21:00 UTC");
        assert!(next_open.is_some());
        assert!(next_close.is_some());
    }

    #[test]
    fn venue_market_window_crypto_is_always_open() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-07-18T12:00:00Z")
            .expect("parse now")
            .with_timezone(&chrono::Utc);
        let (is_open, session, next_open, next_close) = venue_market_window("binance:futures", now);
        assert!(is_open);
        assert_eq!(session, "7/24 (UTC 00:00-24:00)");
        assert!(next_open.is_none());
        assert!(next_close.is_none());
    }

    fn t(opened: &str, closed: &str, pnl: f64) -> ClosedTradeModel {
        ClosedTradeModel {
            symbol: "BTCUSDT".into(),
            is_long: true,
            pnl,
            pnl_pct: 0.0,
            net_pnl: pnl,
            net_pnl_pct: 0.0,
            commission: 0.0,
            exit_reason: "TP".into(),
            opened_at: opened.into(),
            closed_at: closed.into(),
            leverage: 1.0,
            entry_price: 0.0,
            exit_price: 0.0,
            strategy: "EMA_CROSSOVER".into(),
            venue: "binance".into(),
            pyramid_tranches: 1,
            exchange_ref: String::new(),
        }
    }

    /// Default eşik (60 dk) ile compute_scalp_swing_stats çağırır — Faz 2 öncesi
    /// env-tabanlı davranışın test ergonomisini korur.
    fn call_default(trades: &[ClosedTradeModel]) -> (TradeTypeStats, TradeTypeStats) {
        compute_scalp_swing_stats(trades, 60)
    }

    #[test]
    fn empty_trades_yield_zero_stats() {
        let (s, w) = call_default(&[]);
        assert_eq!(s.win_rate, 0.0);
        assert_eq!(w.win_rate, 0.0);
        assert_eq!(s.label, "SCALP");
        assert_eq!(w.label, "SWING");
    }

    #[test]
    fn short_holding_goes_to_scalp() {
        // 10 dakikalık holding → SCALP grubu (default eşik 60 dk).
        let trades = vec![
            t(
                "2026-05-20T10:00:00+00:00",
                "2026-05-20T10:10:00+00:00",
                5.0,
            ),
            t(
                "2026-05-20T11:00:00+00:00",
                "2026-05-20T11:10:00+00:00",
                -2.0,
            ),
        ];
        let (s, w) = call_default(&trades);
        assert!((s.win_rate - 50.0).abs() < 1e-9);
        assert!(
            (s.profit_factor - 2.5).abs() < 1e-9,
            "5/2=2.5; got={}",
            s.profit_factor
        );
        assert_eq!(w.win_rate, 0.0, "Swing grubu boş kalmalı");
    }

    #[test]
    fn long_holding_goes_to_swing() {
        // 2 saatlik holding → SWING grubu.
        let trades = vec![t(
            "2026-05-20T10:00:00+00:00",
            "2026-05-20T12:00:00+00:00",
            10.0,
        )];
        let (s, w) = call_default(&trades);
        assert_eq!(s.win_rate, 0.0);
        assert!((w.win_rate - 100.0).abs() < 1e-9);
        assert!(
            (w.profit_factor - 100.0).abs() < 1e-9,
            "loss yok → cap 100; got={}",
            w.profit_factor
        );
    }

    #[test]
    fn current_streak_counts_consecutive_same_sign() {
        // Son üç trade kazanç → streak = +3
        let trades = vec![
            t(
                "2026-05-20T10:00:00+00:00",
                "2026-05-20T10:05:00+00:00",
                -1.0,
            ),
            t(
                "2026-05-20T11:00:00+00:00",
                "2026-05-20T11:05:00+00:00",
                2.0,
            ),
            t(
                "2026-05-20T12:00:00+00:00",
                "2026-05-20T12:05:00+00:00",
                3.0,
            ),
            t(
                "2026-05-20T13:00:00+00:00",
                "2026-05-20T13:05:00+00:00",
                1.0,
            ),
        ];
        let (s, _) = call_default(&trades);
        assert_eq!(s.current_streak, 3);
    }

    #[test]
    fn current_streak_negative_when_last_trades_are_losses() {
        let trades = vec![
            t(
                "2026-05-20T10:00:00+00:00",
                "2026-05-20T10:05:00+00:00",
                5.0,
            ),
            t(
                "2026-05-20T11:00:00+00:00",
                "2026-05-20T11:05:00+00:00",
                -2.0,
            ),
            t(
                "2026-05-20T12:00:00+00:00",
                "2026-05-20T12:05:00+00:00",
                -1.0,
            ),
        ];
        let (s, _) = call_default(&trades);
        assert_eq!(s.current_streak, -2);
    }

    #[test]
    fn explicit_threshold_moves_trades_between_groups() {
        // Eşik 5 dk → 10 dk holding artık SWING'e düşer.
        let trades = vec![t(
            "2026-05-20T10:00:00+00:00",
            "2026-05-20T10:10:00+00:00",
            7.0,
        )];
        let (s, w) = compute_scalp_swing_stats(&trades, 5);
        assert_eq!(s.win_rate, 0.0);
        assert!((w.win_rate - 100.0).abs() < 1e-9);
    }

    #[test]
    fn missing_opened_at_is_skipped() {
        let trades = vec![
            t("", "2026-05-20T10:10:00+00:00", 5.0), // opened_at boş → skip
            t("2026-05-20T11:00:00+00:00", "", -2.0), // closed_at boş → skip
            t(
                "2026-05-20T12:00:00+00:00",
                "2026-05-20T12:05:00+00:00",
                3.0,
            ),
        ];
        let (s, _) = call_default(&trades);
        // Sadece son trade dahil → win_rate 100
        assert!((s.win_rate - 100.0).abs() < 1e-9);
    }
}
