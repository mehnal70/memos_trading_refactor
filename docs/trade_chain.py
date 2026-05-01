#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Memos Trading — Trade Zinciri PDF Belgesi
Çalıştırma: python3 docs/trade_chain.py
Çıktı:       docs/trade_chain.pdf
"""

from reportlab.lib.pagesizes import A4
from reportlab.lib.units import cm
from reportlab.lib import colors
from reportlab.lib.styles import getSampleStyleSheet, ParagraphStyle
from reportlab.platypus import (
    SimpleDocTemplate, Paragraph, Spacer, Table, TableStyle,
    HRFlowable, KeepTogether
)
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.lib.enums import TA_LEFT, TA_CENTER, TA_JUSTIFY
import os

# ── Font kayıt ────────────────────────────────────────────────────────────────
FONT_DIR = "/usr/share/fonts/truetype/dejavu"
pdfmetrics.registerFont(TTFont("DejaVu",     f"{FONT_DIR}/DejaVuSans.ttf"))
pdfmetrics.registerFont(TTFont("DejaVuB",    f"{FONT_DIR}/DejaVuSans-Bold.ttf"))
pdfmetrics.registerFont(TTFont("DejaVuI",    f"{FONT_DIR}/DejaVuSans-Oblique.ttf"))
pdfmetrics.registerFont(TTFont("DejaVuMono", f"{FONT_DIR}/DejaVuSansMono.ttf"))
pdfmetrics.registerFontFamily("DejaVu", normal="DejaVu", bold="DejaVuB", italic="DejaVuI")

# ── Renk paleti ───────────────────────────────────────────────────────────────
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
C_STEP_BG = colors.HexColor("#EAF4FB")
C_WARN_BG = colors.HexColor("#FEF9E7")
C_ERR_BG  = colors.HexColor("#FDEDEC")
C_OK_BG   = colors.HexColor("#EAFAF1")

W, H = A4
MARGIN = 1.8 * cm

# ── Stil tanımları ────────────────────────────────────────────────────────────
def make_styles():
    s = {}
    base = dict(fontName="DejaVu", leading=14)

    s["title"] = ParagraphStyle("title",
        fontName="DejaVuB", fontSize=22, leading=28,
        textColor=C_WHITE, alignment=TA_CENTER, spaceAfter=6)

    s["subtitle"] = ParagraphStyle("subtitle",
        fontName="DejaVu", fontSize=11, leading=16,
        textColor=colors.HexColor("#CCDDEE"), alignment=TA_CENTER, spaceAfter=4)

    s["h1"] = ParagraphStyle("h1",
        fontName="DejaVuB", fontSize=14, leading=20,
        textColor=C_WHITE, spaceBefore=16, spaceAfter=4,
        backColor=C_ACCENT, leftIndent=0, borderPad=6)

    s["h2"] = ParagraphStyle("h2",
        fontName="DejaVuB", fontSize=11, leading=16,
        textColor=C_ACCENT, spaceBefore=10, spaceAfter=3)

    s["h3"] = ParagraphStyle("h3",
        fontName="DejaVuB", fontSize=10, leading=14,
        textColor=C_TEAL, spaceBefore=6, spaceAfter=2)

    s["body"] = ParagraphStyle("body",
        fontName="DejaVu", fontSize=9, leading=14,
        textColor=C_DARK, spaceAfter=3, alignment=TA_JUSTIFY)

    s["mono"] = ParagraphStyle("mono",
        fontName="DejaVuMono", fontSize=8, leading=12,
        textColor=colors.HexColor("#333333"), backColor=colors.HexColor("#F4F4F4"),
        leftIndent=8, spaceAfter=4)

    s["bullet"] = ParagraphStyle("bullet",
        fontName="DejaVu", fontSize=9, leading=13,
        textColor=C_DARK, leftIndent=14, bulletIndent=4,
        spaceAfter=2)

    s["note"] = ParagraphStyle("note",
        fontName="DejaVuI", fontSize=8.5, leading=13,
        textColor=C_GRAY, leftIndent=10, spaceAfter=3)

    s["label_green"] = ParagraphStyle("label_green",
        fontName="DejaVuB", fontSize=8.5, leading=12,
        textColor=C_GREEN)

    s["label_red"] = ParagraphStyle("label_red",
        fontName="DejaVuB", fontSize=8.5, leading=12,
        textColor=C_RED)

    s["label_yellow"] = ParagraphStyle("label_yellow",
        fontName="DejaVuB", fontSize=8.5, leading=12,
        textColor=C_YELLOW)

    s["center"] = ParagraphStyle("center",
        fontName="DejaVu", fontSize=9, leading=13,
        textColor=C_DARK, alignment=TA_CENTER)

    s["step_num"] = ParagraphStyle("step_num",
        fontName="DejaVuB", fontSize=18, leading=22,
        textColor=C_TEAL, alignment=TA_CENTER)

    return s

S = make_styles()

# ── Yardımcı bileşenler ───────────────────────────────────────────────────────
def hr(color=C_TEAL, thickness=0.5):
    return HRFlowable(width="100%", thickness=thickness, color=color, spaceAfter=4, spaceBefore=4)

def section_header(title, color=C_ACCENT):
    data = [[Paragraph(title, S["h1"])]]
    t = Table(data, colWidths=[W - 2 * MARGIN])
    t.setStyle(TableStyle([
        ("BACKGROUND", (0,0), (-1,-1), color),
        ("TOPPADDING",    (0,0), (-1,-1), 6),
        ("BOTTOMPADDING", (0,0), (-1,-1), 6),
        ("LEFTPADDING",   (0,0), (-1,-1), 10),
        ("RIGHTPADDING",  (0,0), (-1,-1), 8),
        ("ROUNDEDCORNERS", [4]),
    ]))
    return t

def step_block(num, title, desc_paras, color=C_STEP_BG, num_color=C_TEAL):
    """Numaralı adım bloğu: solda büyük rakam, sağda başlık + açıklama"""
    left_cell  = [Paragraph(str(num), S["step_num"])]
    right_cell = [Paragraph(title, S["h2"])] + desc_paras
    data = [[left_cell, right_cell]]
    t = Table(data, colWidths=[1.4*cm, W - 2*MARGIN - 1.6*cm])
    t.setStyle(TableStyle([
        ("BACKGROUND",    (0,0), (-1,-1), color),
        ("VALIGN",        (0,0), (0,-1), "MIDDLE"),
        ("VALIGN",        (1,0), (1,-1), "TOP"),
        ("TOPPADDING",    (0,0), (-1,-1), 8),
        ("BOTTOMPADDING", (0,0), (-1,-1), 8),
        ("LEFTPADDING",   (0,0), (0,-1), 6),
        ("LEFTPADDING",   (1,0), (1,-1), 10),
        ("RIGHTPADDING",  (0,0), (-1,-1), 8),
        ("BOX",           (0,0), (-1,-1), 0.5, C_TEAL),
        ("LINEAFTER",     (0,0), (0,-1), 1.5, num_color),
    ]))
    return t

def filter_table(rows):
    """Filtre/kontrol tablosu: Kontrol | Parametre | Sonuç"""
    header = [
        Paragraph("Kontrol", S["h3"]),
        Paragraph("Kaynak / Parametre", S["h3"]),
        Paragraph("Blok Davranışı", S["h3"]),
    ]
    data = [header] + [
        [Paragraph(r[0], S["body"]),
         Paragraph(r[1], S["mono"]),
         Paragraph(r[2], S["body"])]
        for r in rows
    ]
    cw = [5.8*cm, 6.5*cm, W - 2*MARGIN - 12.5*cm]
    t = Table(data, colWidths=cw)
    t.setStyle(TableStyle([
        ("BACKGROUND",    (0,0), (-1,0),  C_ACCENT),
        ("TEXTCOLOR",     (0,0), (-1,0),  C_WHITE),
        ("FONTNAME",      (0,0), (-1,0),  "DejaVuB"),
        ("ROWBACKGROUNDS",(0,1), (-1,-1), [C_WHITE, C_LGRAY]),
        ("GRID",          (0,0), (-1,-1), 0.3, colors.HexColor("#CCCCCC")),
        ("TOPPADDING",    (0,0), (-1,-1), 4),
        ("BOTTOMPADDING", (0,0), (-1,-1), 4),
        ("LEFTPADDING",   (0,0), (-1,-1), 6),
        ("VALIGN",        (0,0), (-1,-1), "TOP"),
    ]))
    return t

def two_col(left_items, right_items, left_title="", right_title=""):
    l = []
    if left_title:  l.append(Paragraph(left_title, S["h3"]))
    l.extend(left_items)
    r = []
    if right_title: r.append(Paragraph(right_title, S["h3"]))
    r.extend(right_items)
    t = Table([[l, r]], colWidths=[(W-2*MARGIN)/2]*2)
    t.setStyle(TableStyle([
        ("VALIGN",   (0,0), (-1,-1), "TOP"),
        ("TOPPADDING",(0,0),(-1,-1), 0),
        ("LEFTPADDING",(1,0),(1,-1), 12),
    ]))
    return t

# ─────────────────────────────────────────────────────────────────────────────
# İÇERİK
# ─────────────────────────────────────────────────────────────────────────────

def build_content():
    story = []

    # ── Kapak alanı ──────────────────────────────────────────────────────────
    cover_data = [[
        Paragraph("Memos Trading Bot", S["title"]),
        Spacer(1, 4),
        Paragraph("Trade Zinciri — Adım Adım Teknik Referans", S["subtitle"]),
        Spacer(1, 4),
        Paragraph("Versiyon 1.0  •  Nisan 2026  •  memos_trading_core", S["subtitle"]),
    ]]
    cover_tbl = Table(cover_data, colWidths=[W - 2*MARGIN])
    cover_tbl.setStyle(TableStyle([
        ("BACKGROUND", (0,0), (-1,-1), C_DARK),
        ("TOPPADDING",    (0,0), (-1,-1), 22),
        ("BOTTOMPADDING", (0,0), (-1,-1), 22),
        ("LEFTPADDING",   (0,0), (-1,-1), 16),
        ("RIGHTPADDING",  (0,0), (-1,-1), 16),
        ("ROUNDEDCORNERS", [6]),
    ]))
    story.append(cover_tbl)
    story.append(Spacer(1, 0.5*cm))

    # ── Özet kutusu ──────────────────────────────────────────────────────────
    story.append(Paragraph(
        "Bu belge, bir trade sinyalinin doğuşundan pozisyon kapanışına kadar sistemin içinden geçtiği "
        "tüm katmanları sıralı olarak açıklar. Her adımın hangi modülde gerçekleştiği, "
        "hangi parametre/eşik değerlerin devreye girdiği ve olası blok noktaları gösterilmektedir.",
        S["body"]))
    story.append(Spacer(1, 0.3*cm))

    # ════════════════════════════════════════════════════════════════════════
    story.append(section_header("BÖLÜM 1 — Sistem Mimarisi ve Worker Yapısı", C_DARK))
    story.append(Spacer(1, 0.2*cm))

    story.append(Paragraph("Worker Rolleri", S["h2"]))
    arch_rows = [
        ["Primary Worker", "rtc_cli.rs → spawn_worker(is_primary=true)",
         "Tam pipeline (indirme, backtest, ML, p5, sinyal). Tam loglama (disk + TUI).\nBu worker'ın seçtiği sembol/interval tüm zinciri yönetir."],
        ["Secondary Worker", "rtc_cli.rs → orchestrator.spawn(symbol, interval)",
         "Yalnızca scalp/swing engine + Regular sinyal değerlendirmesi.\nLog yalnızca TUI belleğine (disk'e yazılmaz). Dedup filtresi ile spam önlenir."],
        ["Orphan Processor", "robotic_loop.rs → process_orphans()",
         "Primary worker'ın loop'unda çalışır. Önceki oturumdan kalan pozisyonları SL/TP ile kapatır."],
    ]
    story.append(filter_table(arch_rows))
    story.append(Spacer(1, 0.3*cm))

    story.append(Paragraph("Worker Seçim Döngüsü (Primary Rotasyon)", S["h2"]))
    story.append(Paragraph(
        "Orchestrator her döngüde tüm sembol/interval kombinasyonlarına skor hesaplar. "
        "Mevcut primary'nin skoru yeni adaydan %X veya daha fazla gerideyse ve minimum bekleme süresi "
        "dolmuşsa geçiş yapılır. Geçiş sırasında önceki primary worker durdurulur, yeni biri başlatılır.",
        S["body"]))
    story.append(Paragraph(
        "Örnek log: 🎯 Geçiş: BEATUSDT/1m (skor=0.2408) → ZECUSDT/1d (skor=0.8230) | +241.8% fark | 68sn beklendi",
        S["mono"]))
    story.append(Spacer(1, 0.3*cm))

    # ════════════════════════════════════════════════════════════════════════
    story.append(section_header("BÖLÜM 2 — Analiz Pipeline Zinciri (Tab 5)", C_ACCENT))
    story.append(Spacer(1, 0.2*cm))

    story.append(Paragraph(
        "Pipeline, primary worker başladığında otomatik devreye girer. "
        "Chain monitor thread her 60 saniyede durumları kontrol eder ve geciken adımları tetikler "
        "(cooldown: 300 saniye / adım).",
        S["body"]))
    story.append(Spacer(1, 0.2*cm))

    pipeline_steps = [
        (1, "İndir (Download)",
         [Paragraph("Binance API'den OHLCV mumları çeker, SQLite DB'ye yazar (data/trader.db).", S["body"]),
          Paragraph("• Her <b>download_every_mins</b> dakikada bir (varsayılan: 5 dk)", S["bullet"]),
          Paragraph("• <b>download_candle_limit</b>: kaç mum indirileceği (varsayılan: 1500)", S["bullet"]),
          Paragraph("• <b>download_top_n</b>: kaç sembol taranacağı (varsayılan: 10)", S["bullet"]),
          Paragraph("• Blok: <b>download_active=true</b> sırasında yeniden tetiklenmez; "
                    "900 saniye timeout → download_active sıfırlanır.", S["note"])]),

        (2, "Backtest (BTest)",
         [Paragraph("İndirilen mumlar üzerinde strateji kombinasyonları test edilir. "
                    "En iyi parametreler config/optimized_params alanına yazılır.", S["body"]),
          Paragraph("• Her <b>backtest_every_mins</b> dakikada bir (varsayılan: 60 dk)", S["bullet"]),
          Paragraph("• <b>backtest_candle_limit</b>: test penceresi (varsayılan: 1000 mum)", S["bullet"]),
          Paragraph("• Blok: Download tamamlanmamışsa tetiklenmez.", S["note"])]),

        (3, "ML Eğitim (ML Train)",
         [Paragraph("Backtest sonuçlarıyla z-score anomaly + Linear Regression modelini eğitir. "
                    "GBT (Gradient Boosting) skoru güncellenir.", S["body"]),
          Paragraph("• <b>backtest_every_mins + 30</b> dakikada bir", S["bullet"]),
          Paragraph("• Risk modülü: <b>gbt_last_score</b>, <b>ml_running</b> flag'i", S["bullet"]),
          Paragraph("• Blok: Backtest tamamlanmamışsa tetiklenmez.", S["note"])]),

        (4, "p5 Analizi (p5Ana)",
         [Paragraph("Python scripti (p5_crypto.py) çalışır. 11.704+ strateji kombinasyonunu tarar, "
                    "walk-forward doğrulama + Monte Carlo simülasyonu yapar. "
                    "Sonuçlar data/p5_results/ klasörüne yazılır.", S["body"]),
          Paragraph("• ML tamamlandıktan sonra tetiklenir (p5_age > ml_age + 120 saniye)", S["bullet"]),
          Paragraph("• Filtreler: PF > 1.15, WR > 44%, MaxDD < 25%, p_value < 0.10", S["bullet"]),
          Paragraph("• Çıktı: status.json, current_signals.json, SYMBOL_interval_results.json", S["bullet"]),
          Paragraph("• Blok: Cooldown 300 saniye; p5_running=true ise yeniden başlatılmaz.", S["note"])]),

        (5, "Tarayıcı (Screener)",
         [Paragraph("Tüm listelenmiş semboller için çoklu-interval composite skor hesaplar. "
                    "En yüksek skorlu semboller secondary worker listesine alınır.", S["body"]),
          Paragraph("• Her <b>screener_interval_hours</b> saatte bir (varsayılan: 4 saat)", S["bullet"]),
          Paragraph("• Skor bileşenleri: strateji skoru + MTF bonus + trend gücü", S["bullet"])]),

        (6, "MTF Tarama",
         [Paragraph("Multi-Timeframe sinyal taraması. 4h/1d gibi yüksek zaman diliminde "
                    "güçlü sinyal bulunan semboller 'MTF bonus' alır.", S["body"]),
          Paragraph("• Her 2 saatte bir (sabit interval)", S["bullet"]),
          Paragraph("• Bulunan sinyaller: 🚨 MTF SİNYAL logu olarak görünür", S["bullet"])]),
    ]

    for num, title, desc in pipeline_steps:
        story.append(KeepTogether([step_block(num, title, desc), Spacer(1, 0.2*cm)]))

    # ════════════════════════════════════════════════════════════════════════
    story.append(section_header("BÖLÜM 3 — Sinyal Üretimi (Her Candle Close)", C_DARK))
    story.append(Spacer(1, 0.2*cm))

    story.append(Paragraph(
        "Her candle kapandığında (interval ne ise: 1m, 15m, 4h, 1d...) robotic_loop "
        "aşağıdaki sırayla çalışır:",
        S["body"]))
    story.append(Spacer(1, 0.15*cm))

    signal_steps = [
        (1, "Mum Verisi Çekme",
         [Paragraph("DbCachingLiveAdapter → Binance API'den mum verir, DB cache'e yazar.", S["body"]),
          Paragraph("• Bant: <b>candle_limit</b> kadar mum (ana interval + HTF türevleri)", S["bullet"]),
          Paragraph("• HTF türetme: 1m → 5m/15m/30m/1h/4h/1d otomatik türetilir (DB'ye yazılır)", S["bullet"])]),

        (2, "Strateji Seçimi",
         [Paragraph("Optimized params + backtest sonuçlarına göre en iyi strateji seçilir:", S["body"]),
          Paragraph("• Seçenekler: ICT_COMPOSITE, SMC, MA, RSI, BB, MACD, CCI, "
                    "STOCHASTIC, SUPERTREND, PRICE_ACTION, ADX, STOCH_RSI, WILLIAMS, "
                    "DONCHIAN, FUNDING_RATE_CONTRARIAN", S["bullet"]),
          Paragraph("• ADX rejimine göre: Nötr/Trending/Volatile/Kaos", S["bullet"]),
          Paragraph("• compare_strategies: birden fazla strateji paralel değerlendirilebilir", S["bullet"])]),

        (3, "Ham Sinyal Üretimi",
         [Paragraph("Seçilen strateji BUY / SELL / HOLD üretir.", S["body"]),
          Paragraph("• Yüzde oylama: BUY% / SELL% eşiğe (varsayılan: %44) ulaşmalı", S["bullet"]),
          Paragraph("• En iyi skor negatifse → HOLD (örn: MA crossover skoru < 0)", S["bullet"]),
          Paragraph("• HOLD durumunda geri kalanlar atlanır; pozisyon açılmaz.", S["bullet"])]),

        (4, "ML Filtreleme",
         [Paragraph("GBT model skoru (HyperOpt) negatifse sinyal HOLD'a zorlanır.", S["body"]),
          Paragraph("• Eşik: <b>gbt_last_score</b> negatif → 🚫 ML sinyal HOLD'a zorlandı", S["bullet"]),
          Paragraph("• ML modeli henüz eğitilmediyse (score.abs() < 0.01) bu gate atlanır", S["bullet"]),
          Paragraph("• conf değeri: 0.3 minimum (short_min_composite_score)", S["bullet"])]),

        (5, "HTF Bias Hesabı",
         [Paragraph("Yüksek zaman dilimi trend yönü (Bullish / Bearish / Neutral) hesaplanır. "
                    "Bu değer sonraki filtrelerde kullanılır.", S["body"]),
          Paragraph("• HTF candles: ana interval'ın bir üstü (1m → 1h gibi)", S["bullet"]),
          Paragraph("• Bias: EMA/SuperTrend/trend analizi ile belirlenir", S["bullet"])]),

        (6, "Composite Skor Eşiği",
         [Paragraph("Sinyal birden fazla gösterge oylamasıyla doğrulanır. "
                    "Eşiğin altındaki sinyaller filtrelenir:", S["body"]),
          Paragraph("• SHORT için: <b>short_min_composite_score</b> (varsayılan: 0.30)", S["bullet"]),
          Paragraph("• Skor bileşenleri: trend uyumu, hacim, momentum, HTF bias", S["bullet"])]),
    ]

    for num, title, desc in signal_steps:
        story.append(KeepTogether([step_block(num, title, desc, C_WARN_BG), Spacer(1, 0.15*cm)]))

    # ════════════════════════════════════════════════════════════════════════
    story.append(section_header("BÖLÜM 4 — Adaptif Filtreler (Giriş Kapıları)", C_ACCENT))
    story.append(Spacer(1, 0.2*cm))

    story.append(Paragraph(
        "Sinyal HOLD değilse aşağıdaki adaptif filtreler sırayla kontrol edilir. "
        "Herhangi biri return ederse trade açılmaz. Parametreler "
        "config/adaptive_params.json'dan gelir ve her N işlemde otomatik güncellenir.",
        S["body"]))
    story.append(Spacer(1, 0.2*cm))

    story.append(Paragraph("SHORT (SELL) Filtreleri", S["h2"]))
    short_filters = [
        ["1. HTF Bullish Bloğu",
         "short_htf_block: true\nhtf_bias = Bullish",
         "HTF yükseliş trendinde SHORT tamamen engellenir. En güçlü filtre."],
        ["2. Ardışık SHORT Kaybı",
         "short_loss_streak_current ≥\nshort_loss_streak_pause (=5)",
         "Belirtilen sayıda ardışık SHORT kaybından sonra yeni SHORT duraklatılır."],
        ["3. Eşzamanlı SHORT Limiti",
         "total_open_shorts ≥\nmax_concurrent_shorts (=6)",
         "Tüm semboller toplamında açık SHORT sayısı limite ulaştığında bloke."],
        ["4. GBT Bearish Güveni",
         "futures_short_min_conf: 0.30\ngbt_last_score threshold",
         "Futures SHORT için GBT skoru yeterince negatif değilse engellenir."],
        ["5. Mean-Rev + HTF",
         "strateji ∈ {RSI,BB,STOCH,...}\nhtf_bias = Bullish (spot)",
         "Spot piyasada mean-reversion SELL + HTF Bullish kombinasyonu engellenir."],
    ]
    story.append(filter_table(short_filters))
    story.append(Spacer(1, 0.2*cm))

    story.append(Paragraph("LONG (BUY) Filtreleri", S["h2"]))
    long_filters = [
        ["1. HTF Bearish Bloğu",
         "long_htf_block: true\nhtf_bias = Bearish",
         "HTF düşüş trendinde LONG tamamen engellenir."],
        ["2. Sembol Zaten LONG",
         "open_positions[symbol].is_long",
         "Aynı sembolde açık LONG varsa çift pozisyon engellenir."],
        ["3. Eşzamanlı LONG Limiti",
         "total_open_longs ≥\nmax_concurrent_longs (=6)",
         "Global LONG limiti dolduğunda yeni giriş engellenir."],
        ["4. Sembol Seri Kaybı",
         "symbol_consec_loss[sym] ≥\nmax_concurrent_longs + 2",
         "Aynı sembolde art arda kayıp varsa o sembol için LONG durdurulur."],
        ["5. Global Kayıp Serisi",
         "loss_streak ≥\nmax_consecutive_losses (=8)",
         "Global ardışık kayıp eşiği aşıldığında tüm girişler duraklatılır."],
        ["6. Günlük SL Limiti",
         "daily_sl_count[symbol] ≥\nmax_daily_sl_per_symbol (=3)",
         "Bir sembol bugün 3 SL yediyse o sembolde yeni giriş yok."],
    ]
    story.append(filter_table(long_filters))
    story.append(Spacer(1, 0.2*cm))

    story.append(Paragraph("Ortak Filtreler (Her İki Yön)", S["h2"]))
    common_filters = [
        ["AI (StrategyScorer)",
         "reg_disabled / scalp_disabled\n/ swing_disabled",
         "UCB1 bandit: belirli rejimde pnl < -20 ve win < %35 ise o trade tipi devre dışı. "
         "Yeniden etkin: pnl > +10 ve win > %55."],
        ["Volatilite (ADX)",
         "adx_trend_threshold: 25.0\nadx_regime",
         "PatternML ve bazı stratejiler yüksek volatilite rejiminde engellenir."],
        ["Portföy Limiti",
         "max_open_positions\n(config.max_open_positions)",
         "Toplam açık pozisyon sayısı global limite ulaştığında yeni giriş yok."],
        ["Ticaret Kalitesi",
         "config/trade_quality.json\nmin_rr, min_score vb.",
         "Minimum RR (risk/reward) ve kalite skoru sağlanmazsa giriş engellenir."],
        ["SL Cooldown",
         "sl_cooldown_map[symbol]\nsl_cooldown_secs (=600)",
         "Son SL'den itibaren 10 dakika (scalp/swing için daha uzun) bekler."],
    ]
    story.append(filter_table(common_filters))
    story.append(Spacer(1, 0.2*cm))

    # ════════════════════════════════════════════════════════════════════════
    story.append(section_header("BÖLÜM 5 — Scalp / Swing Engine (Paralel Yol)", C_DARK))
    story.append(Spacer(1, 0.2*cm))

    story.append(Paragraph(
        "Regular sinyal zinciriyle paralel olarak her candle close'ta scalp ve swing engine "
        "ayrı değerlendirilir. Bu engine'ler kendi eşik ve filtrelerine sahiptir.",
        S["body"]))
    story.append(Spacer(1, 0.15*cm))

    scalp_steps = [
        (1, "Scalp Engine (SCP)",
         [Paragraph("EMA5/EMA13 crossover + RSI7 + Bollinger Band + hacim spike analizi.", S["body"]),
          Paragraph("• Eşik: skor ≥ 0.60 (Regular'dan bağımsız)", S["bullet"]),
          Paragraph("• SCP HTF Bloğu: short_htf_block=true + HTF Bullish → SHORT engellenir", S["bullet"]),
          Paragraph("• SCP SL Cooldown: 15/30/60 dk ardışık SL sayısına göre artar", S["bullet"]),
          Paragraph("• Swing engeli: rejim=Volatile/Kaos ve yerel ATR > eşik → SWING bloke", S["note"])]),

        (2, "Swing Engine (SWG)",
         [Paragraph("Daha uzun vade: EMA/trend + fiyat yapısı analizi.", S["body"]),
          Paragraph("• ATR filtresi: yerel ATR > volatile eşik ise swing açılmaz", S["bullet"]),
          Paragraph("• SWG SL Cooldown: 2/4/8 saat (ardışık SL'ye göre)", S["bullet"]),
          Paragraph("• Cooldown decay: son kayıptan 3 saat sonra streak 1 azalır", S["bullet"])]),

        (3, "SlotGuard Kontrolü",
         [Paragraph("Scalp/Swing pozisyonu açılmadan önce SlotGuard.can_open() çağrılır.", S["body"]),
          Paragraph("• Aynı sembolde aynı yönde açık pozisyon var mı?", S["bullet"]),
          Paragraph("• Tip bazlı maksimum eşzamanlı slot doldu mu?", S["bullet"]),
          Paragraph("• Dup key kontrolü: SYMBOL_LONG veya SYMBOL_SHORT çakışması", S["bullet"])]),
    ]

    for num, title, desc in scalp_steps:
        story.append(KeepTogether([step_block(num, title, desc, C_OK_BG, C_GREEN), Spacer(1, 0.15*cm)]))

    # ════════════════════════════════════════════════════════════════════════
    story.append(section_header("BÖLÜM 6 — Pozisyon Açılışı", C_ACCENT))
    story.append(Spacer(1, 0.2*cm))

    open_steps = [
        (1, "Giriş Fiyatı Hesabı",
         [Paragraph("WS (WebSocket) veri yaşına göre ağırlıklı fiyat:", S["body"]),
          Paragraph("• WS taze (< 5sn): entry = VWAP (son 5 mum)", S["bullet"]),
          Paragraph("• WS gecikmeli (5-30sn): entry = w×VWAP + (1-w)×REST_mid", S["bullet"]),
          Paragraph("• WS durdu (> 30sn): entry = REST bookTicker mid-price", S["bullet"])]),

        (2, "SL/TP Hesabı",
         [Paragraph("ATR bazlı dinamik stop-loss ve take-profit:", S["body"]),
          Paragraph("• SL = entry × (1 ± sl_atr_multiplier × ATR%)", S["bullet"]),
          Paragraph("• TP = entry × (1 ± tp_atr_multiplier × ATR%)", S["bullet"]),
          Paragraph("• sl_atr_multiplier: 2.0  |  tp_atr_multiplier: 2.2 (adaptive_params)", S["bullet"]),
          Paragraph("• Trailing SL: trailing_sl_activation_pct = 0.90 (kârın %90'ında aktif)", S["bullet"])]),

        (3, "Dinamik Kaldıraç",
         [Paragraph("compute_effective_leverage(): birden fazla faktöre göre kaldıraç ayarlanır:", S["body"]),
          Paragraph("• Temel: base_leverage=9.5, max_leverage=20.0", S["bullet"]),
          Paragraph("• Azaltan: yüksek drawdown, kayıp serisi, çok açık pozisyon", S["bullet"]),
          Paragraph("• Artıran: HTF uyumu, yüksek composite skor, güçlü session RR", S["bullet"]),
          Paragraph("• Spot'ta kaldıraç her zaman 1x", S["note"])]),

        (4, "Miktar (Qty) Hesabı",
         [Paragraph("trade_amount × leverage / entry_price formülü ile pozisyon büyüklüğü:", S["body"]),
          Paragraph("• Regular: config.trade_amount (varsayılan: capital × 0.02)", S["bullet"]),
          Paragraph("• Scalp: base × skor_çarpanı (yüksek skor → büyük lot)", S["bullet"]),
          Paragraph("• Swing: base × 1.3 (risk_multiplier)", S["bullet"]),
          Paragraph("• Maksimum kayıp limiti: max_trade_loss_pct = %0.5", S["bullet"])]),

        (5, "Emir Gönderimi",
         [Paragraph("RoboticTradeExecutor → BinanceTradeExecutor (canlı) veya DummyTradeExecutor (paper):", S["body"]),
          Paragraph("• Paper mode (BINANCE_PAPER_MODE=true): API çağrısı yapılmaz", S["bullet"]),
          Paragraph("• Canlı: Binance API market order + SL/TP order'ları yerleştirilir", S["bullet"]),
          Paragraph("• Açık pozisyon live_positions haritasına ve DB snapshot'a yazılır", S["bullet"])]),
    ]

    for num, title, desc in open_steps:
        story.append(KeepTogether([step_block(num, title, desc, C_STEP_BG), Spacer(1, 0.15*cm)]))

    # ════════════════════════════════════════════════════════════════════════
    story.append(section_header("BÖLÜM 7 — Pozisyon İzleme ve Kapanış", C_DARK))
    story.append(Spacer(1, 0.2*cm))

    monitor_steps = [
        (1, "Anlık Fiyat Güncellemesi",
         [Paragraph("Her 1 saniyede bir (background task):", S["body"]),
          Paragraph("• WS stream'den canlı fiyat → live_positions.current_price güncellenir", S["bullet"]),
          Paragraph("• WS yoksa: DB'deki 1m mum kullanılır (<b>mum yaşı < 30 dk</b> kontrolü)", S["bullet"]),
          Paragraph("• Eski mum (> 30 dk): fiyat güncellenmez → yanlış SL tetiklemesi önlenir", S["note"])]),

        (2, "SL/TP Kontrol Döngüsü",
         [Paragraph("Her candle close'ta process_open_position() çağrılır:", S["body"]),
          Paragraph("• Static SL: current_price ≤ sl_price → stop-loss tetiklenir", S["bullet"]),
          Paragraph("• Static TP: current_price ≥ tp_price → take-profit tetiklenir", S["bullet"]),
          Paragraph("• Trailing SL: kâr trailing_sl_activation_pct × TP mesafesine ulaşınca "
                    "SL trailing başlar (her yeni yüksek/düşükte güncellenir)", S["bullet"]),
          Paragraph("• Breakeven: breakeven_at_rr karşılanınca SL giriş fiyatına çekilir", S["bullet"])]),

        (3, "Orphan Pozisyon İşleme",
         [Paragraph("Önceki oturumdan kalan (sembol değişmiş, primary dışı) pozisyonlar:", S["body"]),
          Paragraph("• process_orphans(): primary loop'unda her iteration çalışır", S["bullet"]),
          Paragraph("• current_price ≤ 0.0 ise atlanır (startup koruması)", S["bullet"]),
          Paragraph("• Normal SL/TP mantığıyla kapatılır veya manuel çıkış beklenir", S["bullet"]),
          Paragraph("• Tüm orphan'lar kapanınca DB snapshot temizlenir", S["bullet"])]),

        (4, "Kapanış İşlemleri",
         [Paragraph("Pozisyon kapanınca:", S["body"]),
          Paragraph("• PnL hesaplanır, trade kaydı DB'ye yazılır", S["bullet"]),
          Paragraph("• StrategyScorer.record(trade_type, regime, pnl) → UCB1 güncellenir", S["bullet"]),
          Paragraph("• Kayıp ise: loss_streak, short_loss_streak, symbol_consec_loss artar", S["bullet"]),
          Paragraph("• SL ise: sl_cooldown_map[symbol] = Instant::now()", S["bullet"]),
          Paragraph("• Her EVAL_EVERY (5) kapanıştan sonra scorer.evaluate() → tip disable kararı", S["bullet"]),
          Paragraph("• Adaptive params periyodik güncelleme: her adjust_every_n_trades (=20) işlemde", S["bullet"])]),
    ]

    for num, title, desc in monitor_steps:
        story.append(KeepTogether([step_block(num, title, desc, C_ERR_BG, C_RED), Spacer(1, 0.15*cm)]))

    # ════════════════════════════════════════════════════════════════════════
    story.append(section_header("BÖLÜM 8 — Evrimsel AI ve Otonom Kontrol", C_ACCENT))
    story.append(Spacer(1, 0.2*cm))

    story.append(Paragraph(
        "AUTONOMOUS_ENABLED=true ise her RoboticLoop başlangıcında etkinleşir:", S["body"]))
    story.append(Spacer(1, 0.15*cm))

    ai_rows = [
        ["AdaptiveBrain (Q-table)", "evolution/",
         "Durum-aksiyon çiftleri için Q değerleri güncellenir. "
         "Her kapanan trade bir 'reward' sinyali taşır."],
        ["PopulationManager", "evolution/",
         "Strateji parametrelerini genetik algoritmayla evrimleştirir. "
         "En iyi popülasyon config/evolution_state.json'a yazılır."],
        ["StrategyScorer (UCB1)", "robot/strategy_scorer.rs",
         "Regular / Scalp / Swing × ADX Rejimi matrisinde bandit öğrenmesi. "
         "Kaybeden kombinasyonlar devre dışı bırakılır."],
        ["AutonomousControl (FSM)", "robot/autonomous_control.rs",
         "Idle → Active → Paused → Emergency durumları yönetir. "
         "config/fsm_state.json'a periyodik kaydedilir."],
        ["HotReload", "robot/hot_reload/",
         "config/ klasöründeki değişiklikleri izler. "
         "Sıfır-downtime ile parametre güncellemesi yapılır."],
    ]
    story.append(filter_table(ai_rows))
    story.append(Spacer(1, 0.2*cm))

    # ════════════════════════════════════════════════════════════════════════
    story.append(section_header("BÖLÜM 9 — Hata Kurtarma ve Koruma Katmanları", C_DARK))
    story.append(Spacer(1, 0.2*cm))

    recovery_rows = [
        ["CircuitBreaker", "robot/error_recovery/",
         "Art arda başarısız API çağrılarında devre keser, cooldown sonrası yeniden dener."],
        ["FailoverManager", "robot/error_recovery/",
         "Birincil exchange başarısız olunca yedek exchange'e geçer."],
        ["RecoveryStateMachine", "robot/error_recovery/",
         "DNS hatası / WS kopması → otomatik yeniden bağlanma. "
         "Snapshot'tan pozisyonlar restore edilir."],
        ["Stale Fiyat Koruması", "rtc_cli.rs (background task)",
         "DB mumu 30 dakikadan eskiyse current_price güncellenmez. "
         "Yanlış SL tetiklemesi önlenir."],
        ["Download Active Timeout", "rtc_cli.rs (pipeline)",
         "download_active=true 900 saniyeyi aşarsa sıfırlanır. "
         "Pipeline'ın kalıcı donması önlenir."],
        ["Chain Monitor Cooldown", "rtc_cli.rs (chain monitor)",
         "Aynı pipeline adımı 300 saniyede en fazla 1 kez otomatik tetiklenir. "
         "Spam / çakışan işlem önlenir."],
    ]
    story.append(filter_table(recovery_rows))
    story.append(Spacer(1, 0.3*cm))

    # ════════════════════════════════════════════════════════════════════════
    story.append(section_header("BÖLÜM 10 — Trade Zinciri Özet Akışı", C_ACCENT))
    story.append(Spacer(1, 0.2*cm))

    story.append(Paragraph(
        "Aşağıdaki şema, bir trade'in başlangıçtan kapanışa kadar geçtiği tüm "
        "adımları tek akışta göstermektedir:", S["body"]))
    story.append(Spacer(1, 0.2*cm))

    # Akış şeması tablo olarak
    flow = [
        ["CANDLE CLOSE", "Interval süresi doldu (1m/15m/4h/1d)"],
        ["▼", ""],
        ["VERİ ÇEKME", "DbCachingLiveAdapter → DB → HTF türetme"],
        ["▼", ""],
        ["STRATEJİ SEÇİMİ", "Optimized params + ADX rejimi"],
        ["▼", ""],
        ["HAM SİNYAL", "BUY / SELL / HOLD → yüzde oylama"],
        ["▼ (HOLD ise dur)", ""],
        ["ML FİLTRESİ", "GBT skor negatif? → HOLD'a zorla"],
        ["▼", ""],
        ["HTF BIAS", "Bullish / Bearish / Neutral hesapla"],
        ["▼", ""],
        ["ADAPTİF FİLTRELER", "HTF blok / seri kayıp / limit / GBT güven / cooldown"],
        ["▼ (Blok ise dur)", ""],
        ["GİRİŞ FİYATI", "WS taze? VWAP : REST mid-price karışımı"],
        ["▼", ""],
        ["SL/TP/KALDIRAÇ", "ATR bazlı dinamik hesap"],
        ["▼", ""],
        ["EMİR", "DummyExecutor (paper) veya BinanceExecutor (canlı)"],
        ["▼", ""],
        ["POZİSYON İZLEME", "Her saniye fiyat güncelle → SL/TP/Trailing kontrol"],
        ["▼", ""],
        ["KAPANIŞ", "PnL → DB → StrategyScorer → AdaptiveBrain → Decay"],
        ["▼", ""],
        ["PARALEL:", "Scalp/Swing engine aynı anda bağımsız çalışır [SCP] [SWG]"],
    ]

    flow_data = []
    for step, detail in flow:
        is_arrow  = step == "▼" or step == "▼ (HOLD ise dur)" or step == "▼ (Blok ise dur)"
        is_par    = step.startswith("PARALEL")
        bg = C_WHITE
        if is_arrow:   bg = C_WHITE
        elif is_par:   bg = C_OK_BG
        else:          bg = C_STEP_BG

        flow_data.append([
            Paragraph(step,   S["h3"] if not is_arrow else S["center"]),
            Paragraph(detail, S["body"]),
        ])

    flow_tbl = Table(flow_data, colWidths=[4.5*cm, W - 2*MARGIN - 4.7*cm])
    style_cmds = [
        ("GRID",     (0,0), (-1,-1), 0.3, colors.HexColor("#CCCCCC")),
        ("TOPPADDING",    (0,0), (-1,-1), 4),
        ("BOTTOMPADDING", (0,0), (-1,-1), 4),
        ("LEFTPADDING",   (0,0), (-1,-1), 8),
        ("VALIGN",   (0,0), (-1,-1), "MIDDLE"),
    ]
    # Renklendir
    for i, (step, _) in enumerate(flow):
        is_arrow = step.startswith("▼")
        is_par   = step.startswith("PARALEL")
        is_stop  = "dur)" in step
        if is_par:
            style_cmds.append(("BACKGROUND", (0,i), (-1,i), C_OK_BG))
            style_cmds.append(("TEXTCOLOR",  (0,i), (0,i),  C_GREEN))
        elif is_stop:
            style_cmds.append(("BACKGROUND", (0,i), (-1,i), C_ERR_BG))
            style_cmds.append(("TEXTCOLOR",  (0,i), (0,i),  C_RED))
        elif is_arrow:
            style_cmds.append(("BACKGROUND", (0,i), (-1,i), C_WHITE))
            style_cmds.append(("TEXTCOLOR",  (0,i), (0,i),  C_TEAL))
        elif i % 2 == 0:
            style_cmds.append(("BACKGROUND", (0,i), (-1,i), C_STEP_BG))
        else:
            style_cmds.append(("BACKGROUND", (0,i), (-1,i), C_LGRAY))

    flow_tbl.setStyle(TableStyle(style_cmds))
    story.append(flow_tbl)
    story.append(Spacer(1, 0.3*cm))

    # ── Footer notu ──────────────────────────────────────────────────────────
    story.append(hr())
    story.append(Paragraph(
        "memos_trading_core  •  rtc_cli (TUI binary)  •  robotic_loop.rs  •  "
        "Nisan 2026  •  Tüm parametreler config/ klasöründen hot-reload edilebilir",
        S["note"]))

    return story


# ─────────────────────────────────────────────────────────────────────────────
# MAIN
# ─────────────────────────────────────────────────────────────────────────────
def main():
    out_dir = os.path.join(os.path.dirname(__file__))
    os.makedirs(out_dir, exist_ok=True)
    out_path = os.path.join(out_dir, "trade_chain.pdf")

    doc = SimpleDocTemplate(
        out_path,
        pagesize=A4,
        leftMargin=MARGIN, rightMargin=MARGIN,
        topMargin=1.5*cm, bottomMargin=1.5*cm,
        title="Memos Trading — Trade Zinciri",
        author="memos_trading_core",
    )

    story = build_content()
    doc.build(story)
    print(f"PDF oluşturuldu: {out_path}")


if __name__ == "__main__":
    main()
