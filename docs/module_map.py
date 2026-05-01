#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Memos Trading — Modüler Yapı Haritası PDF Belgesi
Çalıştırma: python3 docs/module_map.py
Çıktı:       docs/module_map.pdf
"""

from reportlab.lib.pagesizes import A4
from reportlab.lib.units import cm
from reportlab.lib import colors
from reportlab.lib.styles import getSampleStyleSheet, ParagraphStyle
from reportlab.platypus import (
    SimpleDocTemplate, Paragraph, Spacer, Table, TableStyle,
    HRFlowable, KeepTogether, PageBreak
)
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.lib.enums import TA_LEFT, TA_CENTER, TA_JUSTIFY
import os

FONT_DIR = "/usr/share/fonts/truetype/dejavu"
pdfmetrics.registerFont(TTFont("DejaVu",     f"{FONT_DIR}/DejaVuSans.ttf"))
pdfmetrics.registerFont(TTFont("DejaVuB",    f"{FONT_DIR}/DejaVuSans-Bold.ttf"))
pdfmetrics.registerFont(TTFont("DejaVuI",    f"{FONT_DIR}/DejaVuSans-Oblique.ttf"))
pdfmetrics.registerFont(TTFont("DejaVuMono", f"{FONT_DIR}/DejaVuSansMono.ttf"))
pdfmetrics.registerFontFamily("DejaVu", normal="DejaVu", bold="DejaVuB", italic="DejaVuI")

C_DARK    = colors.HexColor("#1A1A2E")
C_ACCENT  = colors.HexColor("#0F3460")
C_BLUE    = colors.HexColor("#16213E")
C_TEAL    = colors.HexColor("#0F7A8C")
C_GREEN   = colors.HexColor("#2ECC71")
C_RED     = colors.HexColor("#E74C3C")
C_YELLOW  = colors.HexColor("#F39C12")
C_GRAY    = colors.HexColor("#555555")
C_LGRAY   = colors.HexColor("#F5F5F5")
C_WHITE   = colors.white
C_BG1     = colors.HexColor("#EAF4FB")
C_BG2     = colors.HexColor("#EAFAF1")
C_BG3     = colors.HexColor("#FEF9E7")
C_BG4     = colors.HexColor("#FDEDEC")
C_BG5     = colors.HexColor("#F5EEF8")
C_BG6     = colors.HexColor("#EBF5FB")
C_PURPLE  = colors.HexColor("#7D3C98")
C_INDIGO  = colors.HexColor("#1A5276")

W, H = A4
MARGIN = 1.5 * cm

def make_styles():
    base = getSampleStyleSheet()
    S = {}
    S["title"] = ParagraphStyle("title", fontName="DejaVuB", fontSize=20, textColor=C_WHITE,
                                 alignment=TA_CENTER, spaceAfter=4)
    S["subtitle"] = ParagraphStyle("subtitle", fontName="DejaVuI", fontSize=11, textColor=colors.HexColor("#AED6F1"),
                                    alignment=TA_CENTER, spaceAfter=2)
    S["section"] = ParagraphStyle("section", fontName="DejaVuB", fontSize=13, textColor=C_WHITE,
                                   spaceAfter=4, spaceBefore=2)
    S["body"] = ParagraphStyle("body", fontName="DejaVu", fontSize=8.5, textColor=C_DARK,
                                 leading=13, spaceAfter=4)
    S["code"] = ParagraphStyle("code", fontName="DejaVuMono", fontSize=7.5, textColor=C_DARK,
                                 leading=12, spaceAfter=2)
    S["bold"] = ParagraphStyle("bold", fontName="DejaVuB", fontSize=8.5, textColor=C_DARK,
                                 leading=13)
    S["small"] = ParagraphStyle("small", fontName="DejaVu", fontSize=7.5, textColor=C_GRAY,
                                  leading=11)
    S["mod_title"] = ParagraphStyle("mod_title", fontName="DejaVuB", fontSize=9, textColor=C_WHITE,
                                     alignment=TA_LEFT, spaceAfter=2)
    S["item"] = ParagraphStyle("item", fontName="DejaVuMono", fontSize=7.5, textColor=C_DARK,
                                 leading=11, leftIndent=6)
    return S

S = make_styles()

def section_header(text, bg=C_ACCENT):
    tbl = Table([[Paragraph(text, S["section"])]], colWidths=[W - 2*MARGIN])
    tbl.setStyle(TableStyle([
        ("BACKGROUND",  (0,0), (-1,-1), bg),
        ("TOPPADDING",  (0,0), (-1,-1), 6),
        ("BOTTOMPADDING", (0,0), (-1,-1), 6),
        ("LEFTPADDING", (0,0), (-1,-1), 10),
        ("ROUNDEDCORNERS", [4]),
    ]))
    return tbl

def mod_box(title, bg, items, col_width=None):
    """Bir modülü kutu içinde gösteren tablo."""
    cw = col_width or (W - 2*MARGIN)
    header = Table([[Paragraph(title, S["mod_title"])]], colWidths=[cw])
    header.setStyle(TableStyle([
        ("BACKGROUND",    (0,0), (-1,-1), bg),
        ("TOPPADDING",    (0,0), (-1,-1), 4),
        ("BOTTOMPADDING", (0,0), (-1,-1), 4),
        ("LEFTPADDING",   (0,0), (-1,-1), 8),
    ]))
    body_rows = [[Paragraph(it, S["item"])] for it in items]
    body = Table(body_rows, colWidths=[cw])
    body.setStyle(TableStyle([
        ("BACKGROUND",    (0,0), (-1,-1), colors.HexColor("#FAFAFA")),
        ("TOPPADDING",    (0,0), (-1,-1), 2),
        ("BOTTOMPADDING", (0,0), (-1,-1), 2),
        ("LEFTPADDING",   (0,0), (-1,-1), 8),
        ("GRID",          (0,0), (-1,-1), 0.3, colors.HexColor("#DDDDDD")),
        ("BOX",           (0,0), (-1,-1), 0.5, bg),
    ]))
    return KeepTogether([header, body, Spacer(1, 0.2*cm)])

def two_col_boxes(left_title, left_bg, left_items,
                   right_title, right_bg, right_items):
    """İki modülü yan yana göster."""
    cw = (W - 2*MARGIN - 0.4*cm) / 2

    def make_cell_table(title, bg, items, cw):
        rows = []
        h_row = [Paragraph(title, S["mod_title"])]
        rows.append(h_row)
        for it in items:
            rows.append([Paragraph(it, S["item"])])
        t = Table(rows, colWidths=[cw])
        styles = [
            ("BACKGROUND",    (0,0), (0,0), bg),
            ("BACKGROUND",    (0,1), (-1,-1), colors.HexColor("#FAFAFA")),
            ("TOPPADDING",    (0,0), (-1,-1), 2),
            ("BOTTOMPADDING", (0,0), (-1,-1), 2),
            ("LEFTPADDING",   (0,0), (-1,-1), 6),
            ("GRID",          (0,1), (-1,-1), 0.3, colors.HexColor("#DDDDDD")),
            ("BOX",           (0,0), (-1,-1), 0.5, bg),
        ]
        t.setStyle(TableStyle(styles))
        return t

    lt = make_cell_table(left_title,  left_bg,  left_items,  cw)
    rt = make_cell_table(right_title, right_bg, right_items, cw)
    outer = Table([[lt, rt]], colWidths=[cw, cw], hAlign="LEFT")
    outer.setStyle(TableStyle([
        ("VALIGN",        (0,0), (-1,-1), "TOP"),
        ("LEFTPADDING",   (0,0), (-1,-1), 0),
        ("RIGHTPADDING",  (0,0), (-1,-1), 0),
        ("TOPPADDING",    (0,0), (-1,-1), 0),
        ("BOTTOMPADDING", (0,0), (-1,-1), 0),
        ("COLPADDING",    (0,0), (-1,-1), 4),
    ]))
    return KeepTogether([outer, Spacer(1, 0.25*cm)])

def relation_table(rows, headers=None):
    """Bağlantı/kullanım tablosu."""
    col_w = [4.5*cm, 4.5*cm, W - 2*MARGIN - 9.0*cm]
    if headers:
        data = [[Paragraph(h, S["bold"]) for h in headers]]
    else:
        data = []
    for r in rows:
        data.append([Paragraph(str(c), S["small"]) for c in r])
    tbl = Table(data, colWidths=col_w, repeatRows=1 if headers else 0)
    style = [
        ("BACKGROUND",    (0,0), (-1,0), C_ACCENT if headers else C_LGRAY),
        ("TEXTCOLOR",     (0,0), (-1,0), C_WHITE if headers else C_DARK),
        ("GRID",          (0,0), (-1,-1), 0.4, colors.HexColor("#CCCCCC")),
        ("TOPPADDING",    (0,0), (-1,-1), 3),
        ("BOTTOMPADDING", (0,0), (-1,-1), 3),
        ("LEFTPADDING",   (0,0), (-1,-1), 5),
        ("ROWBACKGROUNDS",(0,1), (-1,-1), [C_WHITE, C_LGRAY]),
    ]
    tbl.setStyle(TableStyle(style))
    return tbl

# ── Belge oluşturma ───────────────────────────────────────────────────────────
def build_doc():
    out_dir = os.path.dirname(os.path.abspath(__file__))
    out_path = os.path.join(out_dir, "module_map.pdf")
    doc = SimpleDocTemplate(out_path, pagesize=A4,
                            leftMargin=MARGIN, rightMargin=MARGIN,
                            topMargin=MARGIN, bottomMargin=MARGIN)
    story = []

    # ── Kapak ─────────────────────────────────────────────────────────────────
    cover = Table(
        [[Paragraph("Memos Trading", S["title"])],
         [Paragraph("Modüler Yapı Haritası", S["title"])],
         [Paragraph("Modüller · Struct'lar · Trait'ler · Bağlantılar", S["subtitle"])],
         [Paragraph("memos_trading_core — Mimari Referans", S["subtitle"])]],
        colWidths=[W - 2*MARGIN])
    cover.setStyle(TableStyle([
        ("BACKGROUND",    (0,0), (-1,-1), C_DARK),
        ("TOPPADDING",    (0,0), (-1,-1), 12),
        ("BOTTOMPADDING", (0,0), (-1,-1), 12),
        ("LEFTPADDING",   (0,0), (-1,-1), 10),
        ("ROUNDEDCORNERS", [6]),
    ]))
    story.append(cover)
    story.append(Spacer(1, 0.5*cm))

    story.append(Paragraph(
        "Bu belge memos_trading_core kütüphanesindeki tüm modülleri, "
        "ana struct/trait/enum isimlerini ve aralarındaki çağrı/kullanım "
        "ilişkilerini gruplanmış biçimde sunar. "
        "Amacı: kod tabanını bağlamına göre daha modüler hale getirirken "
        "referans olarak kullanmak.", S["body"]))
    story.append(Spacer(1, 0.3*cm))
    story.append(HRFlowable(width="100%", thickness=1, color=C_ACCENT))
    story.append(Spacer(1, 0.3*cm))

    # ══════════════════════════════════════════════════════════════════════════
    # BÖLÜM 1 — Workspace & Crate Yapısı
    # ══════════════════════════════════════════════════════════════════════════
    story.append(section_header("BÖLÜM 1 — Workspace & Crate Yapısı", C_DARK))
    story.append(Spacer(1, 0.25*cm))

    ws_rows = [
        ["memos_trading_core",   "src/lib.rs + src/",         "Ana kütüphane — tüm modüller burada"],
        ["memos_trading_wasm",   "wasm/",                      "WebAssembly hedefi (Tauri/web)"],
        ["memos_trading_desktop","desktop/",                   "Tauri masaüstü uygulaması"],
        ["trading_cli",          "trading_cli/",               "Basit CLI yardımcı aracı"],
        ["rtc_cli (bin)",        "src/bin/rtc_cli.rs",         "Ana TUI binary (ratatui) — 16 000+ satır"],
        ["main_robotic (bin)",   "src/main_robotic.rs",        "Headless trading binary (BinanceLiveAdapter)"],
        ["diag (bin)",           "src/bin/diag.rs",            "Sistem tanı / debug binary"],
    ]
    story.append(relation_table(ws_rows, ["Crate / Binary", "Konum", "Açıklama"]))
    story.append(Spacer(1, 0.4*cm))

    # ══════════════════════════════════════════════════════════════════════════
    # BÖLÜM 2 — lib.rs Modül Haritası (üst seviye)
    # ══════════════════════════════════════════════════════════════════════════
    story.append(section_header("BÖLÜM 2 — lib.rs Üst Seviye Modüller", C_ACCENT))
    story.append(Spacer(1, 0.25*cm))

    lib_rows = [
        ["robot",            "pub mod robot",           "Ana trading döngüsü, strateji, backtest, ML, order mgmt"],
        ["evolution",        "pub mod evolution",       "Genetik algoritma — AdaptiveBrain, PopulationManager"],
        ["candle_synth",     "pub mod candle_synth",    "1m mumlardan HTF sentezi (5m,15m,1h,4h,1d)"],
        ["exchanges",        "pub mod exchanges",       "REST/WS soyutlaması — Binance/Bybit/KuCoin/Coinbase"],
        ["database",         "pub mod database",        "SQLite CRUD (rusqlite bundled)"],
        ["database_reader",  "pub mod database_reader", "Okuma odaklı DB helpers"],
        ["database_writer",  "pub mod database_writer", "Yazma odaklı DB helpers"],
        ["indicators",       "pub mod indicators",      "Temel indikatör hesaplamaları (üst seviye)"],
        ["advanced",         "pub mod advanced",        "Gelişmiş indikatörler + risk + strateji motoru"],
        ["strategies",       "pub mod strategies",      "Üst seviye strateji wrapperları"],
        ["config",           "pub mod config",          "Config, TradingMode"],
        ["types",            "pub mod types",           "Trade, Signal, Candle, RiskParams, PositionId"],
        ["health_monitor",   "pub mod health_monitor",  "HealthStatus, AnomalyDetector, HealthReport"],
        ["ml_anomaly",       "pub mod ml_anomaly",      "Z-score tabanlı anomali tespiti"],
        ["audit_trail",      "pub mod audit_trail",     "Değiştirilemez işlem kaydı"],
        ["gdpr",             "pub mod gdpr",            "GDPR veri gizliliği"],
        ["mfa",              "pub mod mfa",             "TOTP çok faktörlü doğrulama"],
        ["market_regime",    "pub mod market_regime",   "Piyasa rejimi tespiti"],
        ["bist",             "pub mod bist",            "BIST hisse verisi (Yahoo Finance)"],
        ["auto_trading_engine","pub mod auto_trading_engine","Otonom trading motoru"],
        ["pipeline_supervisor","pub mod pipeline_supervisor","Pipeline süpervizörü"],
        ["risk_limits",      "pub mod risk_limits",     "Global risk limitleri"],
        ["rbac (*ent.)",     "#[cfg(enterprise)]",      "RBAC + SSO/LDAP (enterprise)"],
        ["metrics (*ent.)",  "#[cfg(enterprise)]",      "Prometheus metrics (enterprise)"],
        ["hsm (*ent.)",      "#[cfg(enterprise)]",      "HSM/pkcs11 entegrasyonu (enterprise)"],
        ["siem_forwarder (*ent.)", "#[cfg(enterprise)]","SIEM log iletimi (enterprise)"],
    ]
    story.append(relation_table(lib_rows, ["Modül", "Tanım", "İçerik / Amaç"]))
    story.append(Spacer(1, 0.4*cm))

    # ══════════════════════════════════════════════════════════════════════════
    # BÖLÜM 3 — robot/ Ana Modül
    # ══════════════════════════════════════════════════════════════════════════
    story.append(PageBreak())
    story.append(section_header("BÖLÜM 3 — robot/ Modülü (Alt Modüller)", C_TEAL))
    story.append(Spacer(1, 0.25*cm))

    robot_rows = [
        ["robotic_loop",       "robot/robotic_loop.rs",       "Ana döngü (~1200 satır) — RunMode, LivePriceData, RoboticLoop, TradingStateInner"],
        ["autonomous_control", "robot/autonomous_control.rs", "FSM — AutonomousState, AutonomousController, RiskGate"],
        ["autonomous_trader",  "robot/autonomous_trader.rs",  "Otonom trader — AutonomousTrader, StrategyPerformance"],
        ["strategies",         "robot/strategies.rs",         "14 strateji struct — RSI/MACD/BB/EMA/ICT/SMC/MaCrossover..."],
        ["strategy_scorer",    "robot/strategy_scorer.rs",    "UCB1 bandit — StrategyScorer, Arm"],
        ["strategy_selector",  "robot/strategy_selector.rs",  "StrategySelector (strateji seçim mantığı)"],
        ["signal_evaluator",   "robot/signal_evaluator.rs",   "TradeQualityConfig, TrendBias, FilterBlock — filtre pipeline"],
        ["optimizer",          "robot/optimizer.rs",          "HyperOptimizer, AdvancedOptimizer, StrategyGroup, CompositeScore"],
        ["hyperopt",           "robot/hyperopt.rs",           "HyperOpt, HyperOptResult, HyperOptEntry"],
        ["adaptive_params",    "robot/adaptive_params.rs",    "AdaptiveTradeParams — dinamik SL/TP/leverage"],
        ["sr_detector",        "robot/sr_detector.rs",        "SrDetector, SrZone, ZoneType — destek/direnç"],
        ["interfaces",         "robot/interfaces.rs",         "Trait'ler: DataFetcher, StrategyEngine, TradeExecutor, RiskAnalyzer..."],
        ["binance_executor",   "robot/binance_executor.rs",   "Canlı Binance emir yürütücüsü"],
        ["executor",           "robot/executor.rs",           "Genel executor soyutlaması"],
        ["trade_executor",     "robot/trade_executor.rs",     "TradeExecutor trait implementasyonları"],
        ["price_feed",         "robot/price_feed.rs",         "WebSocket canlı fiyat akışı"],
        ["position_manager",   "robot/position_manager.rs",   "Pozisyon yönetim yardımcıları"],
        ["pattern_matcher",    "robot/pattern_matcher.rs",    "Candlestick pattern tespiti"],
        ["symbol_orchestrator","robot/symbol_orchestrator.rs","Çok-sembol orkestrasyon döngüsü"],
        ["telegram_notifier",  "robot/telegram_notifier.rs",  "Telegram bot bildirimleri"],
        ["file_logger",        "robot/file_logger.rs",        "Dosya tabanlı trade logu"],
        ["logger",             "robot/logger.rs",             "Genel log yardımcıları"],
        ["risk_guardrails",    "robot/risk_guardrails.rs",    "Son savunma risk kontrolleri"],
        ["risk_adapter",       "robot/risk_adapter.rs",       "Risk modülü adaptörü"],
        ["integration_advanced","robot/integration_advanced.rs","AdvancedRoboticTrader — tüm modülleri entegre eder"],
        ["automl",             "robot/automl.rs",             "Otomatik ML parametre arama"],
        ["backtest_scheduler", "robot/backtest_scheduler.rs", "Zamanlanmış backtest tetikleyicisi"],
        ["autonomous_audit",   "robot/autonomous_audit.rs",   "Otonom işlem denetim kaydı"],
    ]
    story.append(relation_table(robot_rows, ["Alt Modül", "Dosya", "Ana Tipler / Amaç"]))
    story.append(Spacer(1, 0.4*cm))

    # ══════════════════════════════════════════════════════════════════════════
    # BÖLÜM 4 — robot/ Alt Klasörler (sub-crates)
    # ══════════════════════════════════════════════════════════════════════════
    story.append(PageBreak())
    story.append(section_header("BÖLÜM 4 — robot/ Alt Klasörleri (Sub-Modüller)", C_INDIGO))
    story.append(Spacer(1, 0.25*cm))

    # 4a — backtester
    story.append(mod_box(
        "backtester/  —  Backtest Motoru",
        C_TEAL,
        [
            "engine.rs       → BacktestConfig, Backtester, BacktestResult, SimulatedTrade, PosOptResult",
            "parameter_optimizer.rs → ParameterOptimizer, ParameterSet, OptimizationResult",
            "walk_forward.rs → WalkForwardTester, WalkForwardConfig, WalkForwardResult, WindowResult",
            "mod.rs          → re-export",
        ]
    ))

    # 4b — ml_engine
    story.append(mod_box(
        "ml_engine/  —  Makine Öğrenmesi Motoru",
        C_PURPLE,
        [
            "decision_tree.rs    → DecisionTree, GradientBoostedTrees (GBT), GbtTuneResult",
            "linear_regressor.rs → LinearRegressor, Prediction",
            "feature_extractor.rs→ FeatureExtractor, FeatureVector",
            "signal_predictor.rs → MLSignalPredictor, MLSignalPrediction, FeatureImportance",
            "trade_classifier.rs → TradePatternClassifier (GNB), ClassifierInput, ClassifierSnapshot",
            "drift_detector.rs   → DriftDetector (konsept drift izleme)",
            "mod.rs              → re-export",
        ]
    ))

    # 4c — order_management
    story.append(mod_box(
        "order_management/  —  Emir Yönetimi",
        colors.HexColor("#1A5276"),
        [
            "base.rs         → trait OrderManager, trait SlippageDetector, BaseOrderManagementSystem, DefaultSlippageDetector",
            "types.rs        → OrderId, Order, OrderSide, OrderType, OrderStatus, SlippageInfo, RetryPolicy",
            "binance.rs      → BinanceOrderManager (canlı Binance emirleri)",
            "paper_executor.rs → PaperTradingExecutor, ExecutionCostConfig, ExecutionCostBreakdown",
            "orderbook_sim.rs → OrderBook, OrderBookSimulator, BookLevel, FillResult, SyntheticBookConfig",
            "validator.rs    → OrderValidator, ValidationRules",
            "mock.rs         → MockOrderManager (test)",
            "mod.rs          → re-export",
        ]
    ))

    # 4d — portfolio_manager
    story.append(mod_box(
        "portfolio_manager/  —  Dinamik Portföy Yönetimi",
        colors.HexColor("#145A32"),
        [
            "dynamic_position.rs → DynamicPosition, TrailingStopConfig, ScaleInConfig, ScaleOutConfig, PartialFill",
            "types.rs            → Position, ClosedTrade, PortfolioMetrics",
            "manager.rs          → PortfolioManager",
            "mod.rs              → re-export",
        ]
    ))

    story.append(two_col_boxes(
        "error_recovery/  —  Hata Kurtarma",
        colors.HexColor("#922B21"),
        [
            "recovery_state.rs → RecoveryState (enum), RecoveryAction,",
            "  RecoveryStateMachine, HealthMetric",
            "circuit_breaker.rs → CircuitBreaker (devre kesici)",
            "failover.rs        → FailoverManager",
            "mod.rs             → re-export",
        ],
        "hot_reload/  —  Sıfır-Downtime Güncelleme",
        colors.HexColor("#1A5276"),
        [
            "zero_downtime.rs  → ZeroDowntimeUpdateManager, UpdateState,",
            "  UpdateAction, UpdateProcessInfo",
            "strategy_loader.rs → StrategyLoader, LoadedStrategy,",
            "  StrategyLoadError",
            "version_manager.rs → sürüm kontrolü",
            "mod.rs             → re-export",
        ]
    ))

    story.append(two_col_boxes(
        "scalp_swing/  —  Scalp & Swing Motoru",
        colors.HexColor("#0E6655"),
        [
            "mod.rs          → TradeType, ScalpSwingConfig,",
            "  ScalpSwingStats, TradeOpportunity, ParamBounds",
            "mode_selector.rs → TradeMode, ModeSelector",
            "scalp_engine.rs  → ScalpEngine",
            "swing_engine.rs  → SwingEngine",
            "slot_guard.rs    → SlotGuard, OpenSlot",
        ],
        "advanced_risk/  —  İleri Risk Metrikleri",
        colors.HexColor("#6E2F8A"),
        [
            "kelly.rs     → KellyCriterion, KellyRecommendation",
            "var.rs        → ValueAtRisk, VaRMethod, VaRLimits,",
            "  MonteCarloSimulator, MonteCarloResult",
            "metrics.rs    → SharpeCalculator, SortinoCalculator,",
            "  CalmarCalculator, OmegaCalculator, InformationRatio",
            "mod.rs        → re-export",
        ]
    ))

    story.append(two_col_boxes(
        "data_fetcher/  —  Veri Çekme",
        C_TEAL,
        [
            "mod.rs         → re-export",
            "binance.rs     → BinanceFetcher",
            "bist_fetcher.rs→ BistFetcher (Yahoo Finance)",
            "finnhub.rs     → FinnhubFetcher",
            "hybrid.rs      → HybridBinanceFetcher, FetchMode",
            "live_adapter.rs → BinanceLiveAdapter",
            "market_fetcher.rs → trait MarketFetcher",
            "websocket.rs   → BinanceKlineUpdate, BinanceKline",
        ],
        "data_pipeline/  —  Veri Pipeline",
        C_INDIGO,
        [
            "mod.rs          → DataPipeline, FetchParams",
            "cache.rs        → DataCache, CacheStats",
            "normalizer.rs   → DataNormalizer",
            "sources.rs      → trait DataSource, CsvDataSource,",
            "  DatabaseDataSource, ApiDataSource, HybridDataSource",
        ]
    ))

    story.append(two_col_boxes(
        "calculations/  —  Hesaplama Motoru",
        colors.HexColor("#1F618D"),
        [
            "mod.rs          → CalculationEngine, CalculationEngineAdapter",
            "indicators.rs   → IndicatorEngine, SMA, RSI, MACD, ATR,",
            "  ADX, BollingerBands, Stochastic, CCI, VWAP,",
            "  SuperTrend, DonchianChannel, TEMA, Ichimoku,",
            "  KeltnerChannel, StochasticRSI",
            "math.rs         → Math, MovingAverage, StandardDeviation,",
            "  PercentageChange, Statistics, Correlation, RiskMetrics",
        ],
        "safety/  —  Güvenlik & İzleme",
        colors.HexColor("#7D6608"),
        [
            "safety_manager.rs → SafetyManager, SafetyRules,",
            "  SafetyDrawdownMonitor, SafetyStatus, SafetyMetrics",
            "alerts.rs        → AlertManager, TradingAlert,",
            "  TradingAlertLevel, AlertCode",
            "metrics.rs       → TradingMetrics, EquityTrend",
            "dashboard.rs     → PaperTradingDashboard, DashboardData,",
            "  OpenPosition, DashboardState",
            "mod.rs           → re-export",
        ]
    ))

    story.append(two_col_boxes(
        "persistence/  —  Kalıcılık",
        colors.HexColor("#117A65"),
        [
            "repository.rs → TradeRepository,",
            "  AccountStateRepository, CandleRepository",
            "service.rs    → PersistenceService, TradeResponse,",
            "  CandleResponse, StatsResponse",
            "mod.rs        → re-export",
        ],
        "symbol_manager/  —  Sembol Yönetimi",
        colors.HexColor("#1A5276"),
        [
            "manager.rs → SymbolManager,",
            "  SymbolState, PortfolioStats",
            "mod.rs     → re-export",
        ]
    ))

    story.append(two_col_boxes(
        "advanced_monitoring/  —  Gelişmiş İzleme",
        colors.HexColor("#512E5F"),
        [
            "alert_system.rs → AlertSystem, Alert, AlertConfig,",
            "  AlertLevel, AlertChannel",
            "dashboard.rs    → RealtimeDashboard, DashboardMetrics,",
            "  MetricSnapshot, SystemStatus",
            "performance_trending.rs → PerformanceTrendingEngine,",
            "  TrendAnalysis, TrendData, PerformanceTrend",
            "mod.rs          → re-export",
        ],
        "advanced/  —  Gelişmiş Strateji",
        colors.HexColor("#1B4F72"),
        [
            "combined_strategy.rs → kombine strateji çerçevesi",
            "strategy_selector.rs → gelişmiş seçici (AdvancedStrategySelector)",
            "mod.rs               → re-export",
        ]
    ))

    # ══════════════════════════════════════════════════════════════════════════
    # BÖLÜM 5 — evolution/ Modülü
    # ══════════════════════════════════════════════════════════════════════════
    story.append(PageBreak())
    story.append(section_header("BÖLÜM 5 — evolution/ Modülü (Evrimsel AI)", C_DARK))
    story.append(Spacer(1, 0.25*cm))

    evo_rows = [
        ["adaptive_brain.rs",   "AdaptiveBrain",         "Q-table + epsilon-greedy; rejim başına en iyi strateji seçimi"],
        ["adaptive_brain.rs",   "MarketRegime (enum)",   "Bull/Bear/Sideways/HighVol — piyasa rejimi"],
        ["population_manager.rs","PopulationManager",    "Genetik popülasyon yönetimi; seçim + üreme"],
        ["population_manager.rs","GenerationStats",      "Nesil istatistikleri"],
        ["population_manager.rs","SelectionStrategy",    "Tournament / Roulette / Elitist seçim türleri"],
        ["strategy_genome.rs",  "StrategyGenome",        "Strateji genomunun parametrik temsili"],
        ["strategy_genome.rs",  "GeneticParams",         "GA hiperparametreleri (crossover_rate, mutation_rate...)"],
        ["fitness_evaluator.rs","FitnessScore",          "Uyum skoru (PnL + Sharpe + drawdown bileşeni)"],
        ["fitness_evaluator.rs","PerformanceMetrics",    "Win-rate, max-drawdown, Sharpe gibi ölçütler"],
        ["mutation_engine.rs",  "MutationEngine",        "Mutasyon operatörleri (Gaussian, boundary, random)"],
        ["mutation_engine.rs",  "MutationType",          "Mutasyon türleri enumu"],
        ["mutation_engine.rs",  "MutationStats",         "Mutasyon istatistikleri"],
    ]
    story.append(relation_table(evo_rows, ["Dosya", "Tip", "Açıklama"]))
    story.append(Spacer(1, 0.4*cm))

    # ══════════════════════════════════════════════════════════════════════════
    # BÖLÜM 6 — Ana Trait'ler
    # ══════════════════════════════════════════════════════════════════════════
    story.append(section_header("BÖLÜM 6 — Ana Trait'ler ve Implementasyonları", C_ACCENT))
    story.append(Spacer(1, 0.25*cm))

    trait_rows = [
        ["DataFetcher",       "robot/interfaces.rs",       "fetch_candles()", "BinanceFetcher, BistFetcher, FinnhubFetcher"],
        ["LiveDataFetcher",   "robot/interfaces.rs",       "get_live_price(), get_live_candles()", "BinanceLiveAdapter, HybridBinanceFetcher"],
        ["StrategyEngine",    "robot/interfaces.rs",       "generate_signal()", "tüm strateji struct'ları"],
        ["TradeExecutor",     "robot/interfaces.rs",       "execute_trade()", "BinanceTradeExecutor, DummyExecutor, PaperTradingExecutor"],
        ["Calculator",        "robot/interfaces.rs",       "calculate_indicators()", "CalculationEngineAdapter"],
        ["Reporter",          "robot/interfaces.rs",       "generate_report()", "PersistenceService"],
        ["RiskAnalyzer",      "robot/interfaces.rs",       "analyze_risk()", "RiskAdapter"],
        ["OrderManager",      "order_management/base.rs",  "place_order(), cancel_order(), get_order()", "BinanceOrderManager, MockOrderManager"],
        ["SlippageDetector",  "order_management/base.rs",  "detect_slippage()", "DefaultSlippageDetector"],
        ["MarketFetcher",     "data_fetcher/market_fetcher.rs","fetch_market_data()", "HybridBinanceFetcher"],
        ["DataSource",        "data_pipeline/sources.rs",  "fetch_candles()", "CsvDataSource, DatabaseDataSource, ApiDataSource, HybridDataSource"],
        ["Strategy (robotic_loop)", "robot/robotic_loop.rs","signal()", "MaCrossoverStrategy, RsiStrategy, MacdStrategy..."],
    ]
    story.append(relation_table(trait_rows, ["Trait", "Tanımlandığı Yer", "Temel Metot(lar)", "Implementasyonlar"]))
    story.append(Spacer(1, 0.4*cm))

    # ══════════════════════════════════════════════════════════════════════════
    # BÖLÜM 7 — Modüller Arası Bağımlılık Akışı
    # ══════════════════════════════════════════════════════════════════════════
    story.append(PageBreak())
    story.append(section_header("BÖLÜM 7 — Modüller Arası Bağımlılık Akışı", C_TEAL))
    story.append(Spacer(1, 0.25*cm))

    story.append(Paragraph(
        "Aşağıdaki tablo hangi modülün hangi diğer modülü kullandığını "
        "(bağımlılık yönünü) gösterir. rtc_cli.rs tüm bu modüllerin "
        "kullanıcısıdır.", S["body"]))
    story.append(Spacer(1, 0.2*cm))

    dep_rows = [
        ["robotic_loop",        "data_fetcher, calculations, strategies, evolution, order_management, scalp_swing, sr_detector, portfolio_manager"],
        ["autonomous_control",  "evolution (AdaptiveBrain, PopulationManager), calculations, strategies"],
        ["autonomous_trader",   "strategies, backtester, evolution, calculations"],
        ["backtester/engine",   "strategies, calculations, indicators, types"],
        ["parameter_optimizer", "backtester/engine, calculations, types"],
        ["walk_forward",        "backtester/engine, parameter_optimizer"],
        ["ml_engine/*",         "calculations/indicators, types, database_reader"],
        ["strategy_scorer",     "evolution (AdaptiveBrain), types"],
        ["signal_evaluator",    "calculations, strategies, types, advanced_risk/kelly"],
        ["optimizer",           "backtester, calculations, strategies, types"],
        ["hyperopt",            "optimizer, backtester, types"],
        ["portfolio_manager",   "types, calculations"],
        ["order_management",    "types, exchanges (Binance API)"],
        ["error_recovery",      "types, health_monitor, monitoring"],
        ["hot_reload",          "strategies, types"],
        ["scalp_swing",         "strategies, calculations, types"],
        ["advanced_risk",       "types, calculations/math"],
        ["data_pipeline",       "data_fetcher, database, types, candle_synth"],
        ["symbol_manager",      "types, portfolio_manager, data_fetcher"],
        ["evolution/*",         "types, strategies, calculations"],
        ["persistence",         "database, types"],
        ["safety",              "types, calculations, health_monitor"],
        ["advanced_monitoring", "types, safety, metrics"],
    ]
    dep_col = [3.5*cm, W - 2*MARGIN - 3.5*cm]
    dep_data = [[Paragraph(h, S["bold"]) for h in ["Modül", "Kullandığı Modüller"]]]
    for r in dep_rows:
        dep_data.append([Paragraph(r[0], S["code"]), Paragraph(r[1], S["small"])])
    dep_tbl = Table(dep_data, colWidths=dep_col, repeatRows=1)
    dep_tbl.setStyle(TableStyle([
        ("BACKGROUND",    (0,0), (-1,0), C_ACCENT),
        ("TEXTCOLOR",     (0,0), (-1,0), C_WHITE),
        ("GRID",          (0,0), (-1,-1), 0.4, colors.HexColor("#CCCCCC")),
        ("TOPPADDING",    (0,0), (-1,-1), 3),
        ("BOTTOMPADDING", (0,0), (-1,-1), 3),
        ("LEFTPADDING",   (0,0), (-1,-1), 5),
        ("ROWBACKGROUNDS",(0,1), (-1,-1), [C_WHITE, C_LGRAY]),
        ("VALIGN",        (0,0), (-1,-1), "TOP"),
    ]))
    story.append(dep_tbl)
    story.append(Spacer(1, 0.4*cm))

    # ══════════════════════════════════════════════════════════════════════════
    # BÖLÜM 8 — rtc_cli.rs İç Yapısı (TUI)
    # ══════════════════════════════════════════════════════════════════════════
    story.append(PageBreak())
    story.append(section_header("BÖLÜM 8 — rtc_cli.rs İç Yapısı (TUI Binary)", C_DARK))
    story.append(Spacer(1, 0.25*cm))

    story.append(Paragraph(
        "rtc_cli.rs (~16 000 satır) tüm modülleri birleştiren TUI ikiliğidir. "
        "İçindeki ana struct/enum/fn gruplaması:", S["body"]))
    story.append(Spacer(1, 0.2*cm))

    tui_rows = [
        ["OtoConfig",           "Config struct",          "JSON'dan yüklenen tüm ayarlar (exchange, symbol, interval, pipeline, kelly...)"],
        ["AppState",            "Ana durum struct",       "Ekran verisi — TUI'ye sunulan tüm canlı veri (~100 alan)"],
        ["LiveState",           "Canlı trading durumu",   "Pozisyonlar, trade log, fiyatlar (iç kilit)"],
        ["OptimizedParamsCache","Config struct",          "Cache'lenmiş optimizer sonuçları"],
        ["SessionFilterConfig", "Config struct",          "İzin verilen işlem saatleri"],
        ["ProfileConfig",       "Config struct",          "robotic_profiles.json pozisyon profili"],
        ["PipelineChainStep",   "İzleme struct",          "Her pipeline adımının durumu (Ok/Running/Stale/Failed/Pending)"],
        ["ChainStepStatus",     "Enum",                   "Pipeline adım durum enumı"],
        ["LivePipelineHealth",  "Durum struct",           "Tüm zincir adımlarının anlık sağlığı"],
        ["draw_dashboard()",    "Render fn",              "Tab 1 — dashboard çizimi"],
        ["draw_ai_center()",    "Render fn",              "Tab 2 — AI merkezi çizimi"],
        ["draw_logs()",         "Render fn",              "Tab 3 — günlük log çizimi"],
        ["draw_positions()",    "Render fn",              "Tab 4 — açık pozisyonlar"],
        ["draw_live_prices()",  "Render fn",              "Tab 5 — canlı fiyatlar + pipeline chain"],
        ["draw_intervals()",    "Render fn",              "Tab 6 — MTF interval türevleri"],
        ["draw_charts()",       "Render fn",              "Tab 7 — grafik çizimi"],
        ["spawn_trading_loop()","Arka plan thread",       "Robotic döngüsünü Arc<Mutex<AppState>> üzerinden başlatır"],
        ["spawn_chain_monitor()","Arka plan thread",      "Pipeline adımlarını izler, stale olunca yeniden tetikler"],
        ["spawn_ml_worker()",   "Arka plan thread",       "GBT/GNB modeli periyodik eğitim thread'i"],
        ["spawn_p5_worker()",   "Arka plan thread",       "P5 screener analizi periyodik çalışır"],
        ["load_oto_config()",   "Config yükleyici",       "rtc_config.json'dan OtoConfig yükler"],
        ["save_oto_config()",   "Config kaydedici",       "OtoConfig'i JSON'a yazar"],
    ]
    story.append(relation_table(tui_rows, ["İsim", "Tür", "Açıklama"]))
    story.append(Spacer(1, 0.4*cm))

    # ══════════════════════════════════════════════════════════════════════════
    # BÖLÜM 9 — Connector Modülleri (Exchange)
    # ══════════════════════════════════════════════════════════════════════════
    story.append(section_header("BÖLÜM 9 — Connector ve Exchange Modülleri", C_ACCENT))
    story.append(Spacer(1, 0.25*cm))

    conn_rows = [
        ["exchanges.rs",         "Genel exchange soyutlama", "fetch_candles(), interval_to_ms(), parse_candle() — tüm exchange'ler için"],
        ["binance_connector.rs", "Binance bağlantısı",       "REST + WS Binance Spot/Futures/CoinM"],
        ["bybit_connector.rs",   "Bybit bağlantısı",         "Bybit REST API"],
        ["kucoin_connector.rs",  "KuCoin bağlantısı",        "KuCoin REST API"],
        ["coinbase_connector.rs","Coinbase bağlantısı",      "Coinbase Advanced Trade API"],
        ["exchange_connector.rs","Ortak connector trait",    "ExchangeConnector trait"],
        ["bist.rs",              "BIST hisse verisi",         "Yahoo Finance üzerinden Türk hisse verileri"],
        ["candle_synth.rs",      "Mum sentezi",              "1m'den 5m/15m/1h/4h/1d türetme — DB'ye yaz"],
        ["data_fetcher/binance.rs","Binance veri çekici",    "BinanceFetcher (REST)"],
        ["data_fetcher/websocket.rs","WS mesaj tipleri",     "BinanceKlineUpdate, BinanceKline"],
    ]
    story.append(relation_table(conn_rows, ["Modül", "Tür", "İçerik"]))
    story.append(Spacer(1, 0.4*cm))

    # ══════════════════════════════════════════════════════════════════════════
    # BÖLÜM 10 — Güvenlik ve Compliance Modülleri
    # ══════════════════════════════════════════════════════════════════════════
    story.append(section_header("BÖLÜM 10 — Güvenlik & Compliance Modülleri", C_RED))
    story.append(Spacer(1, 0.25*cm))

    sec_rows = [
        ["mfa.rs",           "TOTP 2FA",              "otpauth ile zaman bazlı tek kullanımlık şifre"],
        ["jwt_auth.rs",      "JWT",                   "jsonwebtoken ile token üretimi ve doğrulama"],
        ["audit_trail.rs",   "Denetim Kaydı",         "Değiştirilemez işlem logu (immutable)"],
        ["gdpr.rs",          "GDPR",                  "Kişisel veri silme/anonimleştirme"],
        ["secure_store.rs",  "AES-GCM",               "API anahtarı şifreli depolama"],
        ["robot/security/",  "SecurityManager",       "User, UserRole, AuditEvent, RateLimitRule, ApiKeyManager"],
        ["rbac.rs (*ent.)",  "RBAC",                  "Rol bazlı erişim kontrolü (enterprise)"],
        ["sso_ldap.rs (*ent.)","SSO/LDAP",            "Kurumsal kimlik doğrulama (enterprise)"],
        ["hsm.rs (*ent.)",   "HSM/pkcs11",            "Donanım güvenlik modülü (enterprise)"],
        ["metrics.rs (*ent.)","Prometheus",           "Telemetri ve metrik dışa aktarımı (enterprise)"],
        ["siem_forwarder.rs (*ent.)","SIEM",          "Güvenlik olay yönetimi log iletimi (enterprise)"],
        ["fcm_push.rs (*ent.)","FCM",                 "Firebase mobil push bildirimleri (enterprise)"],
    ]
    story.append(relation_table(sec_rows, ["Modül", "Teknoloji", "Açıklama"]))
    story.append(Spacer(1, 0.4*cm))

    # ══════════════════════════════════════════════════════════════════════════
    # BÖLÜM 11 — Modüler Ayrıştırma Önerileri
    # ══════════════════════════════════════════════════════════════════════════
    story.append(PageBreak())
    story.append(section_header("BÖLÜM 11 — Gelecek Modüler Ayrıştırma Önerileri", C_DARK))
    story.append(Spacer(1, 0.25*cm))

    story.append(Paragraph(
        "Mevcut yapının daha modüler ve sürdürülebilir hale getirilmesi için "
        "önerilen dönüşüm planı:", S["body"]))
    story.append(Spacer(1, 0.2*cm))

    refactor_rows = [
        ["rtc_cli.rs bölünmesi",
         "rtc_cli.rs ~16 000 satır — TUI render fn'lerini ayrı dosyalara taşı.",
         "src/bin/tui/ altında tabs/dashboard.rs, tabs/ai_center.rs, tabs/positions.rs vb."],
        ["Merkezi IndicatorEngine",
         "Her strateji kendi RSI/ATR/MACD hesaplıyor — 5× tekrar.",
         "robot/calculations/central_cache.rs — IndicatorCache<'interval, Vec<Candle>>"],
        ["Strateji trait kırılması",
         "Strategy trait tek; scalp ve swing aynı interface'i kullanan farklı sınıflar.",
         "ScalpStrategy ve SwingStrategy sub-trait'leri ekle, ModeSelector bunlara dispatch etsin."],
        ["Config facade",
         "OtoConfig, ProfileConfig, SessionFilterConfig, OptimizedParamsCache dağınık.",
         "robot/config/ altında merkezi AppConfig{ oto, profile, session, cache }"],
        ["DB soyutlama katmanı",
         "database.rs, database_reader.rs, database_writer.rs üç ayrı dosya — çakışan fonksiyonlar var.",
         "persistence/ altında tek Repository<T> generic soyutlama; CRUD trait'i."],
        ["Evolution ↔ Strategy bridge",
         "AdaptiveBrain Q-table ve StrategyScorer UCB1 birbirinden bağımsız çalışıyor.",
         "Ortak StrategyRanker trait'i — her ikisi de aynı interface'e uyarlanır."],
        ["Pipeline supervisor",
         "ChainMonitor döngüsü rtc_cli.rs içinde — iş mantığı ve TUI birbirine karışmış.",
         "robot/pipeline_supervisor.rs'yi güçlendir; rtc_cli sadece state'i oku."],
        ["Test altyapısı",
         "Integration testleri doğrudan src/tests/ içinde — unit ile karışık.",
         "tests/ klasörünü integration/, unit/, fixtures/ olarak ayır."],
    ]
    ref_col = [4.5*cm, 5.5*cm, W - 2*MARGIN - 10.0*cm]
    ref_data = [[Paragraph(h, S["bold"]) for h in ["Konu", "Mevcut Durum", "Öneri"]]]
    for r in refactor_rows:
        ref_data.append([Paragraph(c, S["small"]) for c in r])
    ref_tbl = Table(ref_data, colWidths=ref_col, repeatRows=1)
    ref_tbl.setStyle(TableStyle([
        ("BACKGROUND",    (0,0), (-1,0), C_DARK),
        ("TEXTCOLOR",     (0,0), (-1,0), C_WHITE),
        ("GRID",          (0,0), (-1,-1), 0.4, colors.HexColor("#CCCCCC")),
        ("TOPPADDING",    (0,0), (-1,-1), 4),
        ("BOTTOMPADDING", (0,0), (-1,-1), 4),
        ("LEFTPADDING",   (0,0), (-1,-1), 5),
        ("ROWBACKGROUNDS",(0,1), (-1,-1), [C_WHITE, colors.HexColor("#FEF9E7")]),
        ("VALIGN",        (0,0), (-1,-1), "TOP"),
    ]))
    story.append(ref_tbl)
    story.append(Spacer(1, 0.4*cm))

    # ── Footer not ────────────────────────────────────────────────────────────
    story.append(HRFlowable(width="100%", thickness=1, color=C_ACCENT))
    story.append(Spacer(1, 0.2*cm))
    story.append(Paragraph(
        "memos_trading_core — Modüler Yapı Haritası  |  "
        "Oluşturulma: 2026-04-28  |  python3 docs/module_map.py",
        S["small"]))

    doc.build(story)
    print(f"PDF oluşturuldu: {out_path}")

if __name__ == "__main__":
    build_doc()
