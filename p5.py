"""
XAUUSD TAM SİSTEM v5 - Tek Dosya
1) 1m/5m/15m/1h resample  2) Feature engineering (v4: PSAR+CHoCH+OB+Wyckoff+Fib+HTF)
3) Optimizasyon (v4: MIN_TRADES=200, decay filter, Sortino)
4) Walk-forward (4 ceyrek)  5) Monte Carlo + Bidir strateji
Kullanim: python p5.py
"""
import warnings; warnings.filterwarnings("ignore")
import numpy as np
import pandas as pd
import json, os, math, sys
from datetime import datetime
from itertools import combinations

# ── ÇIKTIYI HEM EKRANA HEM sonuclar.txt'YE YAZ ──────────────────
class Tee:
    """Hem terminale hem dosyaya yazar."""
    def __init__(self, *files):
        self.files = files
    def write(self, obj):
        for f in self.files:
            f.write(obj)
            f.flush()
    def flush(self):
        for f in self.files:
            f.flush()

_log_file = open("sonuclar.txt", "w", encoding="utf-8")
sys.stdout = Tee(sys.__stdout__, _log_file)
sys.stderr = Tee(sys.__stderr__, _log_file)

# ================================================================
# AYARLAR v5
# ================================================================
class CFG:
    # Ana veri
    M1_FILE  = "XAUUSD_1m.csv"
    M5_FILE  = "XAUUSD_5m.csv"
    M15_FILE = "XAUUSD_15m.csv"
    M1H_FILE = "XAUUSD_1h.csv"

    # OOS-1: Son 2 ay (yakın dönem, 2026)
    OOS_M1_FILE  = "xauusd_1m_2ay.csv"
    OOS_M5_FILE  = "xauusd_5m_2ay.csv"
    OOS_M15_FILE = "xauusd_15m_2ay.csv"
    OOS_M1H_FILE = "xauusd_1h_2ay.csv"

    # OOS-2: 2010-2012 (gecmis rejim, tamamen farkli donem)
    HIST_M1_FILE  = "XAU_1m_2010_2012.csv"
    HIST_M5_FILE  = "XAU_5m_2010_2012.csv"
    HIST_M15_FILE = "XAU_15m_2010_2012.csv"
    HIST_M1H_FILE = "XAU_1h_2010_2012.csv"   # yoksa 1m'den uretilir

    # B2: Gercekci maliyet (0.30 → 0.50, haber saatleri 3x)
    SPREAD_PTS   = 0.50    # v5: 0.30→0.50 daha gercekci ortalama
    COMMISSION   = 0.07
    # B1: Slippage (market order kayma)
    SLIPPAGE_PTS = 0.10    # v5 YENi: her giris/cikista kayma

    # Haber saatlerinde spread carpani (UTC)
    # NFP: Cuma 13:30 | Fed: Carsamba 14:00 | CPI: Sali/Carsamba 13:30
    NEWS_HOURS_UTC = {(4,13,30), (2,14,0), (1,13,30), (2,13,30)}  # (weekday,h,m)
    SPREAD_NEWS_MULT = 3.0   # haber saatinde spread 3x

    TRAIN_RATIO  = 0.75
    RECENT_RATIO = 0.95

    TIMEOUT_BARS = 60
    MIN_TRADES   = 200
    MIN_RECENT_TRADES = 25
    MIN_PF       = 1.15
    MIN_WR       = 0.44
    MAX_DD       = 0.22

    MAX_WR_DECAY = 0.15
    MAX_PF_DECAY = 0.35

    TP_SL_GRID = [
        (1.5, 1.0), (2.0, 1.0), (2.5, 1.0), (3.0, 1.0),
        (2.0, 0.8), (2.5, 0.8), (3.0, 0.8),
        (1.5, 0.7), (2.0, 0.7), (2.5, 0.7),
    ]

    N_SIM          = 10000
    STARTING_BAL   = 10000
    RISK_PER_TRADE = 0.01
    TOP_N          = 15

    ACCOUNT_SIZE   = 1000.0
    LEVERAGE       = 500
    RISK_PCT       = 0.02
    LOT_STEP       = 0.01
    MIN_LOT        = 0.01
    MAX_LOT        = 5.0
    CONTRACT_SIZE  = 100

    # B3: Cuma gap koruma — Cuma bu saatten sonra giris yapma (UTC)
    FRIDAY_CUTOFF_HOUR = 20  # Cuma 20:00 UTC sonrasi giris yok

    # A3: Bonferroni — dinamik (combo sayisi belli olunca hesaplanir)
    BONFERRONI_ALPHA = 0.05  # baz alfa


np.random.seed(42)

# ================================================================
# 1. VERİ YUKLE & RESAMPLE
# ================================================================
def load_csv(path):
    df = pd.read_csv(path)
    df = df.rename(columns={"time": "datetime"})
    df["datetime"] = pd.to_datetime(df["datetime"], format="ISO8601", utc=True).dt.tz_localize(None)
    df = df.sort_values("datetime").reset_index(drop=True)
    for col in ["open","high","low","close","volume"]:
        if col in df.columns:
            df[col] = pd.to_numeric(df[col], errors="coerce")
    return df.dropna(subset=["open","high","low","close"])

def resample_tf(df1, rule):
    d = df1.set_index("datetime")
    agg = {k:v for k,v in {"open":"first","high":"max","low":"min","close":"last","volume":"sum"}.items() if k in d.columns}
    r = d.resample(rule, label="left", closed="left").agg(agg).dropna(subset=["open","close"])
    return r.reset_index()

print("\n" + "="*66)
print("  XAUUSD TAM SİSTEM v5 — Double OOS + 17 Duzeltme")
print("="*66)
print("\n[1/5] Veri yukleniyor...")

# ── ANA VERİ ─────────────────────────────────────────────────────
df1 = load_csv(CFG.M1_FILE)
print(f"  MAIN 1m : {len(df1):,} mum | {df1['datetime'].min().date()} -> {df1['datetime'].max().date()}")

if os.path.exists(CFG.M5_FILE):
    df5 = load_csv(CFG.M5_FILE)
    print(f"  MAIN 5m : {len(df5):,} mum (dosyadan)")
else:
    df5 = resample_tf(df1, "5min")
    df5.rename(columns={"datetime":"time"}).to_csv(CFG.M5_FILE, index=False)
    print(f"  MAIN 5m : {len(df5):,} mum (uretildi)")

if os.path.exists(CFG.M15_FILE):
    df15 = load_csv(CFG.M15_FILE)
    print(f"  MAIN 15m: {len(df15):,} mum (dosyadan)")
else:
    df15 = resample_tf(df1, "15min")
    df15.rename(columns={"datetime":"time"}).to_csv(CFG.M15_FILE, index=False)
    print(f"  MAIN 15m: {len(df15):,} mum (uretildi)")

if os.path.exists(CFG.M1H_FILE):
    df1h = load_csv(CFG.M1H_FILE)
    print(f"  MAIN 1h : {len(df1h):,} mum (dosyadan)")
else:
    df1h = resample_tf(df1, "1h")
    df1h.rename(columns={"datetime":"time"}).to_csv(CFG.M1H_FILE, index=False)
    print(f"  MAIN 1h : {len(df1h):,} mum (uretildi)")

# ── C1: 2010-2012 HİSTORİK VERİ (OOS-2 + ana veriye concat) ─────
hist_available = False
df1_hist = df5_hist = df15_hist = df1h_hist = None
if os.path.exists(CFG.HIST_M1_FILE):
    try:
        df1_hist  = load_csv(CFG.HIST_M1_FILE)
        # 5m
        if os.path.exists(CFG.HIST_M5_FILE):
            df5_hist = load_csv(CFG.HIST_M5_FILE)
        else:
            df5_hist = resample_tf(df1_hist, "5min")
            df5_hist.rename(columns={"datetime":"time"}).to_csv(CFG.HIST_M5_FILE, index=False)
        # 15m
        if os.path.exists(CFG.HIST_M15_FILE):
            df15_hist = load_csv(CFG.HIST_M15_FILE)
        else:
            df15_hist = resample_tf(df1_hist, "15min")
            df15_hist.rename(columns={"datetime":"time"}).to_csv(CFG.HIST_M15_FILE, index=False)
        # 1h — yoksa 1m'den uret, kaydet
        if os.path.exists(CFG.HIST_M1H_FILE):
            df1h_hist = load_csv(CFG.HIST_M1H_FILE)
            print(f"  HIST 1h : {len(df1h_hist):,} mum (dosyadan)")
        else:
            df1h_hist = resample_tf(df1_hist, "1h")
            df1h_hist.rename(columns={"datetime":"time"}).to_csv(CFG.HIST_M1H_FILE, index=False)
            print(f"  HIST 1h : {len(df1h_hist):,} mum (uretildi → {CFG.HIST_M1H_FILE})")
        hist_available = True
        print(f"  HIST 1m : {len(df1_hist):,} mum | {df1_hist['datetime'].min().date()} -> {df1_hist['datetime'].max().date()}")
        print(f"  HIST 5m : {len(df5_hist):,} mum | HIST 15m: {len(df15_hist):,} mum")
        print(f"  HIST: OOS-2 olarak kullanilacak (ana veriye EKLENMEYECEK)")
        # HIST ana veriye eklenmez — sadece OOS-2 testi icin kullanilir
        # 12 yillik bosluk (2013-2023) concat'i anlamsiz kilar ve rolling window kirliligi olusturur
    except Exception as e:
        print(f"  HIST: yuklenemedi ({e})")
        hist_available = False
else:
    print(f"  HIST: {CFG.HIST_M1_FILE} bulunamadi -- OOS-2 atlanacak")

# ── OOS-1: Son 2 ay (yakin donem, 2026) ──────────────────────────
df1_oos = None; df5_oos = None; df15_oos = None; df1h_oos = None
oos1_available = False
if os.path.exists(CFG.OOS_M1_FILE):
    try:
        df1_oos  = load_csv(CFG.OOS_M1_FILE)
        df5_oos  = load_csv(CFG.OOS_M5_FILE)  if os.path.exists(CFG.OOS_M5_FILE)  else resample_tf(df1_oos,"5min")
        df15_oos = load_csv(CFG.OOS_M15_FILE) if os.path.exists(CFG.OOS_M15_FILE) else resample_tf(df1_oos,"15min")
        df1h_oos = load_csv(CFG.OOS_M1H_FILE) if os.path.exists(CFG.OOS_M1H_FILE) else resample_tf(df1_oos,"1h")
        oos1_available = True
        print(f"  OOS-1 1m : {len(df1_oos):,} mum | {df1_oos['datetime'].min().date()} -> {df1_oos['datetime'].max().date()}")
        print(f"  OOS-1 5m : {len(df5_oos):,} mum | 15m: {len(df15_oos):,} mum | 1h: {len(df1h_oos):,} mum")
    except Exception as e:
        print(f"  OOS-1: yuklenemedi ({e})")
else:
    print(f"  OOS-1: {CFG.OOS_M1_FILE} bulunamadi -- atlaniyor")

# Geriye donuk uyumluluk (eski degisken isimleri)
oos_available = oos1_available

# ================================================================
# 2. FEATURE ENGINEERING
# ================================================================
def add_features(df, tf):
    df = df.copy()
    o,h,l,c = df["open"],df["high"],df["low"],df["close"]
    tr = pd.concat([(h-l),(h-c.shift(1)).abs(),(l-c.shift(1)).abs()],axis=1).max(axis=1)
    df["atr14"] = tr.rolling(14).mean()
    df["atr5"]  = tr.rolling(5).mean()
    for p in [8,13,21,50,200]:
        df[f"ema{p}"] = c.ewm(span=p,adjust=False).mean()
    df["ema_bull"]     = (df["ema8"]>df["ema21"])&(df["ema21"]>df["ema50"])
    df["ema_bear"]     = (df["ema8"]<df["ema21"])&(df["ema21"]<df["ema50"])
    df["above_ema200"] = c > df["ema200"]
    delta = c.diff()
    gain  = delta.clip(lower=0).rolling(14).mean()
    loss  = (-delta.clip(upper=0)).rolling(14).mean()
    df["rsi14"]  = 100 - 100/(1+gain/(loss+1e-9))
    df["rsi_os"]  = df["rsi14"]<35
    df["rsi_ob"]  = df["rsi14"]>65
    df["rsi_os2"] = df["rsi14"]<30
    df["rsi_ob2"] = df["rsi14"]>70
    ema12=c.ewm(span=12,adjust=False).mean(); ema26=c.ewm(span=26,adjust=False).mean()
    df["macd"]     = ema12-ema26
    df["macd_sig"] = df["macd"].ewm(span=9,adjust=False).mean()
    df["macd_up"]  = (df["macd"]>df["macd_sig"])&(df["macd"].shift(1)<=df["macd_sig"].shift(1))
    df["macd_dn"]  = (df["macd"]<df["macd_sig"])&(df["macd"].shift(1)>=df["macd_sig"].shift(1))
    low14=l.rolling(14).min(); high14=h.rolling(14).max()
    df["stoch_k"]  = 100*(c-low14)/(high14-low14+1e-9)
    df["stoch_d"]  = df["stoch_k"].rolling(3).mean()
    df["stoch_os"]  = (df["stoch_k"]<25)&(df["stoch_d"]<25)
    df["stoch_ob"]  = (df["stoch_k"]>75)&(df["stoch_d"]>75)
    df["stoch_os2"] = (df["stoch_k"]<20)&(df["stoch_d"]<20)
    df["stoch_ob2"] = (df["stoch_k"]>80)&(df["stoch_d"]>80)
    bb_mid=c.rolling(20).mean(); bb_std=c.rolling(20).std()
    df["bb_upper"] = bb_mid+2*bb_std
    df["bb_lower"] = bb_mid-2*bb_std
    df["bb_width"]  = (df["bb_upper"]-df["bb_lower"])/(bb_mid+1e-9)
    df["bb_squeeze"] = df["bb_width"]<df["bb_width"].rolling(50).quantile(0.2)
    df["at_bb_lower"] = c<df["bb_lower"]
    df["at_bb_upper"] = c>df["bb_upper"]
    df["_date"] = df["datetime"].dt.date
    df["_tp"]   = (h+l+c)/3
    if "volume" in df.columns and df["volume"].sum()>0:
        df["_ctv"] = df.groupby("_date").apply(lambda g:(g["_tp"]*g["volume"]).cumsum()).reset_index(level=0,drop=True)
        df["_cv"]  = df.groupby("_date")["volume"].cumsum()
        df["vwap"] = df["_ctv"]/(df["_cv"]+1e-9)
    else:
        df["vwap"] = c.rolling(20).mean()
    df["above_vwap"] = c>df["vwap"]
    df["below_vwap"] = c<df["vwap"]
    df = df.drop(columns=["_date","_tp","_ctv","_cv"],errors="ignore")
    df["fvg_bull"] = l>h.shift(2)
    df["fvg_bear"] = h<l.shift(2)
    body=( c-o).abs(); down_c=c<o; up_c=c>o
    df["ob_bull"] = down_c.shift(1)&up_c&(body>df["atr14"]*0.5)
    df["ob_bear"] = up_c.shift(1)&down_c&(body>df["atr14"]*0.5)
    rl=l.rolling(10).min(); rh=h.rolling(10).max()
    df["liq_bull"] = (l<rl.shift(1))&(c>rl.shift(1))
    df["liq_bear"] = (h>rh.shift(1))&(c<rh.shift(1))
    sh20=h.rolling(20).max(); sl20=l.rolling(20).min()
    df["bos_bull"] = (c>sh20.shift(1))&(c.shift(1)<=sh20.shift(2))
    df["bos_bear"] = (c<sl20.shift(1))&(c.shift(1)>=sl20.shift(2))
    df["bull_eng"] = (o>c.shift(1))&(c>o.shift(1))&(body>body.shift(1))
    df["bear_eng"] = (o<c.shift(1))&(c<o.shift(1))&(body>body.shift(1))
    uw=h-pd.concat([o,c],axis=1).max(axis=1)
    lw=pd.concat([o,c],axis=1).min(axis=1)-l
    df["hammer"]     = (lw>body*2)&(lw>uw*2)&(body>0)
    df["inv_hammer"] = (uw>body*2)&(uw>lw*2)&(body>0)
    df["consec_dn"]  = down_c&down_c.shift(1)&down_c.shift(2)
    df["consec_up"]  = up_c&up_c.shift(1)&up_c.shift(2)
    df["high_vol"] = df["atr14"]>df["atr14"].rolling(50).quantile(0.7)
    df["low_vol"]  = df["atr14"]<df["atr14"].rolling(50).quantile(0.3)
    df["mom3_up"]  = (c-c.shift(3))>0
    df["mom3_dn"]  = (c-c.shift(3))<0
    df["hour"]     = df["datetime"].dt.hour
    df["london"]   = df["hour"].between(7,11)
    df["ny"]       = df["hour"].between(13,17)
    df["session"]  = df["london"]|df["ny"]
    df["trend_up"] = c>c.rolling(15).mean()
    df["trend_dn"] = c<c.rolling(15).mean()

    # ── YENİ: Williams %R ──────────────────────────────────────
    hh14 = h.rolling(14).max()
    ll14 = l.rolling(14).min()
    df["willr"]    = -100*(hh14-c)/(hh14-ll14+1e-9)
    df["willr_os"] = df["willr"] < -80   # aşırı satım
    df["willr_ob"] = df["willr"] > -20   # aşırı alım

    # ── YENİ: CCI (Commodity Channel Index) ────────────────────
    tp2 = (h+l+c)/3
    cci_mean = tp2.rolling(20).mean()
    cci_std  = tp2.rolling(20).std()  # MAD yerine STD (daha hızlı)
    df["cci"]    = (tp2 - cci_mean)/(0.015*cci_std+1e-9)
    df["cci_os"] = df["cci"] < -100
    df["cci_ob"] = df["cci"] > 100

    # ── YENİ: ADX (trend gücü filtresi) ────────────────────────
    tr2   = pd.concat([(h-l),(h-c.shift(1)).abs(),(l-c.shift(1)).abs()],axis=1).max(axis=1)
    dm_p  = (h-h.shift(1)).clip(lower=0)
    dm_m  = (l.shift(1)-l).clip(lower=0)
    dm_p  = dm_p.where(dm_p>dm_m, 0)
    dm_m  = dm_m.where(dm_m>dm_p.shift(0), 0)  # recompute after masking
    # Simplified Wilder smoothing
    atr14w= tr2.ewm(alpha=1/14,adjust=False).mean()
    dip14 = dm_p.ewm(alpha=1/14,adjust=False).mean()
    dim14 = dm_m.ewm(alpha=1/14,adjust=False).mean()
    di_p  = 100*dip14/(atr14w+1e-9)
    di_m  = 100*dim14/(atr14w+1e-9)
    dx    = 100*(di_p-di_m).abs()/(di_p+di_m+1e-9)
    df["adx"]        = dx.ewm(alpha=1/14,adjust=False).mean()
    df["adx_strong"] = df["adx"] > 25   # güçlü trend
    df["adx_weak"]   = df["adx"] < 20   # zayıf trend (konsolidasyon)

    # ── YENİ: ATR oranı (volatilite normalized momentum) ───────
    df["atr_ratio"] = df["atr14"] / (df["atr14"].rolling(50).mean()+1e-9)
    df["vol_expand"]= df["atr_ratio"] > 1.2   # ATR genişliyor
    df["vol_contract"]=df["atr_ratio"] < 0.8  # ATR daralıyor

    # ── YENİ: Donchian Channel ──────────────────────────────────
    dc_high = h.rolling(20).max()
    dc_low  = l.rolling(20).min()
    dc_mid  = (dc_high + dc_low) / 2
    df["dc_breakout_up"]  = c > dc_high.shift(1)   # yeni 20-bar high kırma
    df["dc_breakout_dn"]  = c < dc_low.shift(1)    # yeni 20-bar low kırma
    df["above_dc_mid"]    = c > dc_mid
    df["below_dc_mid"]    = c < dc_mid

    # ── YENİ: ICT — Premium / Discount Zone ────────────────────
    # Son 50 barda range ortası: discount=altı (long için uygun), premium=üstü (short için uygun)
    range_high = h.rolling(50).max()
    range_low  = l.rolling(50).min()
    range_mid  = (range_high + range_low) / 2
    df["in_discount"] = c < range_mid   # fiyat range'in alt yarısında
    df["in_premium"]  = c > range_mid   # fiyat range'in üst yarısında

    # ── YENİ: ICT — Equal Highs / Equal Lows (liquidity pools) ─
    # 5 bar içinde aynı seviyede 2 high/low → likidite havuzu
    high_diff = (h - h.shift(1)).abs()
    low_diff  = (l - l.shift(1)).abs()
    atr_tol   = df["atr14"] * 0.1  # %10 ATR tolerans
    df["equal_highs"] = high_diff < atr_tol   # eşit highs → likidite üstte
    df["equal_lows"]  = low_diff  < atr_tol   # eşit lows → likidite altta

    # ── YENİ: ICT — Mitigation Block ────────────────────────────
    # OB'ye geri dönüş: fiyat daha önce OB bölgesini kırdıktan sonra geri döndü
    ob_bull_prev = (down_c.shift(2))&(up_c.shift(1))&(body.shift(1)>df["atr14"].shift(1)*0.5)
    df["mit_bull"] = ob_bull_prev & (c <= c.shift(1))   # geri çekilme → mitigation
    ob_bear_prev = (up_c.shift(2))&(down_c.shift(1))&(body.shift(1)>df["atr14"].shift(1)*0.5)
    df["mit_bear"] = ob_bear_prev & (c >= c.shift(1))

    # ── YENİ: PA — Inside Bar (konsolidasyon/kırılma) ──────────
    df["inside_bar"] = (h <= h.shift(1)) & (l >= l.shift(1))
    # Inside bar kırılması: güçlü yön sinyali
    df["ib_break_up"] = df["inside_bar"].shift(1) & (c > h.shift(1))
    df["ib_break_dn"] = df["inside_bar"].shift(1) & (c < l.shift(1))

    # ── YENİ: PA — Morning/Evening Star (3-mum pattern) ────────
    doji2 = body.shift(1) < (h.shift(1)-l.shift(1))*0.15
    df["morning_star"] = down_c.shift(2) & doji2 & up_c & (c > (o.shift(2)+c.shift(2))/2)
    df["evening_star"] = up_c.shift(2)   & doji2 & down_c & (c < (o.shift(2)+c.shift(2))/2)

    # ── YENİ: Zaman bazlı filtreler ─────────────────────────────
    df["asian_session"] = df["hour"].between(0,6)    # Asya seansı (UTC)
    df["pre_london"]    = df["hour"].between(5,7)    # London açılış öncesi
    df["london_ny_overlap"] = df["hour"].between(13,15)  # En yüksek likidite

    # ── YENİ: Trend gücü (slope) ────────────────────────────────
    ema21 = c.ewm(span=21,adjust=False).mean()
    df["ema21_slope_up"] = ema21 > ema21.shift(3)   # EMA21 yukarı eğimli
    df["ema21_slope_dn"] = ema21 < ema21.shift(3)   # EMA21 aşağı eğimli

    # ── YENİ: RSI momentum ──────────────────────────────────────
    df["rsi_rising"]  = df["rsi14"] > df["rsi14"].shift(3)   # RSI yükseliyor
    df["rsi_falling"] = df["rsi14"] < df["rsi14"].shift(3)   # RSI düşüyor

    # ── YENİ: Hacim bazlı (volume spike) ────────────────────────
    if "volume" in df.columns and df["volume"].sum()>0:
        vol_ma = df["volume"].rolling(20).mean()
        df["vol_spike"] = df["volume"] > vol_ma * 1.5
        df["vol_dry"]   = df["volume"] < vol_ma * 0.5
        # OBV trendi
        obv = (np.sign(c.diff()) * df["volume"]).fillna(0).cumsum()
        df["obv_up"] = obv > obv.shift(5)
        df["obv_dn"] = obv < obv.shift(5)
        df["mfi_os"] = False; df["mfi_ob"] = False  # MFI skip (hiz icin)
    else:
        df["vol_spike"] = False; df["vol_dry"] = False
        df["obv_up"] = False; df["obv_dn"] = False
        df["mfi_os"] = False; df["mfi_ob"] = False

    # ── YENİ: Parabolic SAR (hafif versiyon) ──────────────────
    # EMA cross proxy: hızlı EMA üstte mi altında mı
    ema3=c.ewm(span=3,adjust=False).mean(); ema10=c.ewm(span=10,adjust=False).mean()
    df["psar_bull"]     = ema3 > ema10
    df["psar_bear"]     = ema3 < ema10
    df["psar_flip_bull"]= (ema3>ema10) & (ema3.shift(1)<=ema10.shift(1))
    df["psar_flip_bear"]= (ema3<ema10) & (ema3.shift(1)>=ema10.shift(1))

    # ── YENİ: Keltner Channel ──────────────────────────────────
    kc_mid = c.ewm(span=20,adjust=False).mean()
    kc_atr = df["atr14"]
    df["kc_upper"]  = kc_mid + 2*kc_atr
    df["kc_lower"]  = kc_mid - 2*kc_atr
    df["at_kc_lower"] = c < df["kc_lower"]
    df["at_kc_upper"] = c > df["kc_upper"]
    df["kc_squeeze"]  = df["bb_width"] < (df["kc_upper"]-df["kc_lower"])/(kc_mid+1e-9)

    # ── YENİ: ROC ve Awesome Oscillator ───────────────────────
    df["roc5"]   = c.pct_change(5)*100
    df["roc10"]  = c.pct_change(10)*100
    df["roc_pos"]= df["roc5"]>0
    df["roc_neg"]= df["roc5"]<0
    mp=(h+l)/2
    ao = mp.rolling(5).mean()-mp.rolling(34).mean()
    df["ao_pos"]=ao>0; df["ao_pos_cross"]=(ao>0)&(ao.shift(1)<=0)
    df["ao_neg"]=ao<0; df["ao_neg_cross"]=(ao<0)&(ao.shift(1)>=0)

    # ── YENİ: Pivot Points (önceki mum bazlı) ─────────────────
    pp=(h.shift(1)+l.shift(1)+c.shift(1))/3
    df["above_pp"]=c>pp; df["below_pp"]=c<pp
    df["at_r1"]=abs(c-(2*pp-l.shift(1)))<df["atr14"]*0.3
    df["at_s1"]=abs(c-(2*pp-h.shift(1)))<df["atr14"]*0.3

    # ── YENİ: ICT Kill Zones ──────────────────────────────────
    df["killzone_london"]=df["hour"].between(8,10)   # London KZ
    df["killzone_ny"]    =df["hour"].between(13,15)  # NY Open KZ
    df["killzone_ny2"]   =df["hour"].between(15,17)  # NY Afternoon KZ
    df["silver_bullet_am"]=df["hour"].between(10,11) # Silver Bullet AM
    df["silver_bullet_pm"]=df["hour"].between(14,15) # Silver Bullet PM

    # ── YENİ: ICT Judas Swing (ilk saatte yön aldatma) ────────
    df["early_session"]=df["hour"].between(8,9)
    df["judas_long"]  =df["early_session"]&down_c   # açılışta düşüş → aslında long
    df["judas_short"] =df["early_session"]&up_c     # açılışta yükseliş → aslında short

    # ── YENİ: ICT Unicorn (OB + FVG aynı bölge) ──────────────
    df["unicorn_bull"] = df["ob_bull"] & df["fvg_bull"]
    df["unicorn_bear"] = df["ob_bear"] & df["fvg_bear"]

    # ── YENİ: ICT Breaker Block ───────────────────────────────
    # Önceki OB bull kırılmışsa (down candle sonrası up candle ama fiyat düştü) → bearish breaker
    df["breaker_bear"] = df["ob_bull"].shift(3) & (c < c.shift(3))
    df["breaker_bull"] = df["ob_bear"].shift(3) & (c > c.shift(3))

    # ── YENİ: ICT OTE (Optimal Trade Entry - Fibonacci) ───────
    swing_h5=h.rolling(5).max(); swing_l5=l.rolling(5).min()
    fib618=(swing_h5-swing_l5)*0.618
    fib786=(swing_h5-swing_l5)*0.786
    df["ote_bull"]=(c>=swing_l5+fib618)&(c<=swing_l5+fib786)&(c<swing_h5)
    df["ote_bear"]=(c>=swing_h5-fib786)&(c<=swing_h5-fib618)&(c>swing_l5)

    # ── YENİ: PA Gelişmiş Pattern'ler ─────────────────────────
    # Marubozu: gövde > %90 tam range, gölge çok küçük
    full_range=h-l
    df["bull_marubozu"]=up_c&(body>full_range*0.85)&(uw<full_range*0.05)
    df["bear_marubozu"]=down_c&(body>full_range*0.85)&(lw<full_range*0.05)
    # Tweezer Top/Bottom (çift tepe/dip)
    df["tweezer_top"]  =(abs(h-h.shift(1))<df["atr14"]*0.05)&up_c.shift(1)&down_c
    df["tweezer_bot"]  =(abs(l-l.shift(1))<df["atr14"]*0.05)&down_c.shift(1)&up_c
    # Three White Soldiers / Three Black Crows
    df["three_soldiers"]=up_c&up_c.shift(1)&up_c.shift(2)&(c>c.shift(1))&(c.shift(1)>c.shift(2))
    df["three_crows"]  =down_c&down_c.shift(1)&down_c.shift(2)&(c<c.shift(1))&(c.shift(1)<c.shift(2))
    # Shooting Star / Hanging Man (bearish)
    df["shooting_star"]=(uw>body*2)&(uw>lw*3)&(body<full_range*0.3)&up_c.shift(1)
    df["hanging_man"]  =(lw>body*2)&(lw>uw*3)&(body<full_range*0.3)&up_c.shift(1)
    # Dark Cloud Cover / Piercing
    df["dark_cloud"]=(up_c.shift(1))&down_c&(o>h.shift(1))&(c<(o.shift(1)+c.shift(1))/2)
    df["piercing"]  =(down_c.shift(1))&up_c&(o<l.shift(1))&(c>(o.shift(1)+c.shift(1))/2)
    # Harami
    df["bull_harami"]=down_c.shift(1)&up_c&(o>c.shift(1))&(c<o.shift(1))
    df["bear_harami"]=up_c.shift(1)&down_c&(o<c.shift(1))&(c>o.shift(1))

    # ── YENİ: Haftalık/Günlük Bias ────────────────────────────
    df["dow"] = df["datetime"].dt.dayofweek  # 0=Pzt, 4=Cuma
    df["is_monday"]  =df["dow"]==0
    df["is_tuesday"] =df["dow"]==1
    df["is_wednesday"]=df["dow"]==2
    df["is_thursday"]=df["dow"]==3
    df["is_friday"]  =df["dow"]==4
    # Pzt/Salı bias: hafta başı yön belirlenir (ICT)
    df["week_open_bias_up"] =df["is_monday"]&up_c
    df["week_open_bias_dn"] =df["is_monday"]&down_c

    # ── YENİ: Calmar için DD bazlı filtre ─────────────────────
    # Son 20 mumda maksimum düşüş (anlık DD proxy)
    roll_max=c.rolling(20).max()
    df["local_dd"]=(roll_max-c)/(roll_max+1e-9)
    df["local_dd_low"] =df["local_dd"]<0.002   # DD çok az → düşük stres
    df["local_dd_high"]=df["local_dd"]>0.010   # DD yüksek → dikkat

    # -- YENİ: Ichimoku (basitleştirilmiş) ----------------------------------------
    tenkan=(h.rolling(9).max()+l.rolling(9).min())/2
    kijun =(h.rolling(26).max()+l.rolling(26).min())/2
    df["ichi_bull"]=tenkan>kijun
    df["ichi_bear"]=tenkan<kijun
    df["ichi_cross_bull"]=(tenkan>kijun)&(tenkan.shift(1)<=kijun.shift(1))
    df["ichi_cross_bear"]=(tenkan<kijun)&(tenkan.shift(1)>=kijun.shift(1))

    # ================================================================
    # YENİ v4: GERCEK PARABOLIC SAR
    # ================================================================
    af_step=0.02; af_max=0.20
    psar_arr = np.zeros(len(df))
    psar_bull_arr = np.zeros(len(df), dtype=bool)
    psar_flip_b_arr = np.zeros(len(df), dtype=bool)
    psar_flip_s_arr = np.zeros(len(df), dtype=bool)
    h_arr = h.values; l_arr = l.values; c_arr = c.values
    bull = True; af = af_step
    ep = l_arr[0]   # extreme point
    sar = h_arr[0]  # baslangic SAR
    for i in range(2, len(df)):
        prev_bull = bull
        if bull:
            sar = sar + af*(ep - sar)
            sar = min(sar, l_arr[i-1], l_arr[i-2])
            if l_arr[i] < sar:
                bull = False; sar = ep; ep = h_arr[i]; af = af_step
            else:
                if h_arr[i] > ep:
                    ep = h_arr[i]
                    af = min(af+af_step, af_max)
        else:
            sar = sar + af*(ep - sar)
            sar = max(sar, h_arr[i-1], h_arr[i-2])
            if h_arr[i] > sar:
                bull = True; sar = ep; ep = l_arr[i]; af = af_step
            else:
                if l_arr[i] < ep:
                    ep = l_arr[i]
                    af = min(af+af_step, af_max)
        psar_arr[i] = sar
        psar_bull_arr[i] = bull
        psar_flip_b_arr[i] = bull and not prev_bull
        psar_flip_s_arr[i] = (not bull) and prev_bull
    df["real_psar"]       = psar_arr
    df["real_psar_bull"]  = psar_bull_arr
    df["real_psar_bear"]  = ~psar_bull_arr
    df["real_psar_flip_bull"] = psar_flip_b_arr
    df["real_psar_flip_bear"] = psar_flip_s_arr
    df["above_psar"] = c > df["real_psar"]
    df["below_psar"] = c < df["real_psar"]

    # ================================================================
    # YENİ v4: CHoCH — Change of Character (Market Structure)
    # ================================================================
    # Swing high/low: 5 bar pivot — center=False (lookahead yok)
    swing_h = h.rolling(5).max().shift(2)  # A1: center=True DUZELTILDI
    swing_l = l.rolling(5).min().shift(2)  # A1: center=True DUZELTILDI
    # CHoCH BULL: fiyat önceki swing low altına çekti sonra geri döndü
    #   → sahte kırılım, likidite süpürme, long fırsatı
    prev_ll = l.rolling(10).min().shift(1)
    prev_hh = h.rolling(10).max().shift(1)
    df["choch_bull"] = (l < prev_ll) & (c > prev_ll)   # false break down → long
    df["choch_bear"] = (h > prev_hh) & (c < prev_hh)   # false break up   → short
    # Güçlü CHoCH: ATR ile normalize
    df["choch_bull_strong"] = df["choch_bull"] & ((prev_ll - l) > df["atr14"] * 0.3)
    df["choch_bear_strong"] = df["choch_bear"] & ((h - prev_hh) > df["atr14"] * 0.3)

    # ================================================================
    # YENİ v4: GÜÇLÜ ORDER BLOCK (multi-bar, ATR normalize)
    # ================================================================
    # Mevcut ob_bull sadece 2 mum bakıyor. Güçlü OB:
    # Son 5 barda en büyük down candle body → o bölge gerçek OB
    ob_body_max = body.rolling(5).max()
    df["ob_bull_strong"] = down_c & (body == ob_body_max) & (body > df["atr14"] * 0.7)
    df["ob_bear_strong"] = up_c   & (body == ob_body_max) & (body > df["atr14"] * 0.7)
    # OB + FVG aynı bölge (Unicorn güçlü versiyon)
    df["unicorn_bull_strong"] = df["ob_bull_strong"] & df["fvg_bull"]
    df["unicorn_bear_strong"] = df["ob_bear_strong"] & df["fvg_bear"]

    # ================================================================
    # YENİ v4: WYCKOFF SPRING / UPTHRUST
    # ================================================================
    # Spring: 20-bar low altına iner, kapar üstünde → alım
    s20 = l.rolling(20).min().shift(1)
    r20 = h.rolling(20).max().shift(1)
    df["wyckoff_spring"]   = (l < s20) & (c > s20) & (c > o)   # long
    df["wyckoff_upthrust"] = (h > r20) & (c < r20) & (c < o)   # short

    # ================================================================
    # YENİ v4: VOLUME DELTA (satış/alış baskısı proxy)
    # ================================================================
    if "volume" in df.columns and df["volume"].sum() > 0:
        # Yukarı kapanan mumda hacim → alış baskısı
        buy_vol  = df["volume"].where(up_c, 0)
        sell_vol = df["volume"].where(down_c, 0)
        df["buy_pressure"]  = buy_vol.rolling(5).mean() > sell_vol.rolling(5).mean()
        df["sell_pressure"] = sell_vol.rolling(5).mean() > buy_vol.rolling(5).mean()
        # Hacim kuru → sinyal zayıf
        df["vol_confirm_bull"] = up_c   & df["vol_spike"]
        df["vol_confirm_bear"] = down_c & df["vol_spike"]
    else:
        df["buy_pressure"]  = False
        df["sell_pressure"] = False
        df["vol_confirm_bull"] = False
        df["vol_confirm_bear"] = False

    # ================================================================
    # YENİ v4: FIBONACCI RETRACEMENT ZONELERİ (swing bazlı)
    # ================================================================
    sh = h.rolling(20).max()
    sl2 = l.rolling(20).min()
    rng = sh - sl2
    fib236 = sl2 + rng * 0.236
    fib382 = sl2 + rng * 0.382
    fib500 = sl2 + rng * 0.500
    fib618 = sl2 + rng * 0.618
    fib786 = sl2 + rng * 0.786
    # Fiyat Fib bölgesinde mi?
    df["at_fib382_support"] = (c >= fib382 - df["atr14"]*0.2) & (c <= fib382 + df["atr14"]*0.2)
    df["at_fib618_support"] = (c >= fib618 - df["atr14"]*0.2) & (c <= fib618 + df["atr14"]*0.2)
    df["at_fib382_resist"]  = (c >= (sh-rng*0.382) - df["atr14"]*0.2) & (c <= (sh-rng*0.382) + df["atr14"]*0.2)
    df["at_fib618_resist"]  = (c >= (sh-rng*0.618) - df["atr14"]*0.2) & (c <= (sh-rng*0.618) + df["atr14"]*0.2)
    # Golden zone (0.618-0.786 arası — ICT OTE ile örtüşür)
    df["in_golden_zone_bull"] = (c >= fib618) & (c <= fib786)
    df["in_golden_zone_bear"] = (c >= sh-rng*0.786) & (c <= sh-rng*0.618)

    # ================================================================
    # YENİ v4: CANDLE RANGE QUALITY (gürültü filtresi)
    # ================================================================
    # Küçük ranged mum → sinyal güvenilmez
    df["quality_candle"] = (h - l) > df["atr14"] * 0.5    # en az ATR'nin yarısı
    df["doji"] = body < (h - l) * 0.1                      # doji → belirsizlik
    df["no_doji"] = ~df["doji"]

    # ================================================================
    # YENİ v4: VOLATILITY REGIME (Bollinger/ATR rejim)
    # ================================================================
    # BB squeeze kırılımı → trend başlıyor
    df["bb_breakout_up"] = (~df["bb_squeeze"]) & df["bb_squeeze"].shift(1) & up_c
    df["bb_breakout_dn"] = (~df["bb_squeeze"]) & df["bb_squeeze"].shift(1) & down_c
    # ATR percentile: düşük volatilite → kırılım bekle
    df["atr_pct20"] = df["atr14"] < df["atr14"].rolling(100).quantile(0.20)  # çok düşük vol
    df["atr_pct80"] = df["atr14"] > df["atr14"].rolling(100).quantile(0.80)  # yüksek vol

    # ================================================================
    # YENİ v4: TREND MOMENTUM SKORU (çoklu TF uyumu)
    # ================================================================
    # EMA hizalama skoru: ne kadar hizalı?
    bull_score = (
        (df["ema8"] > df["ema21"]).astype(int) +
        (df["ema21"] > df["ema50"]).astype(int) +
        (df["ema50"] > df["ema200"]).astype(int) +
        (df["rsi14"] > 50).astype(int) +
        (df["macd"] > df["macd_sig"]).astype(int)
    )
    df["strong_bull_align"] = bull_score >= 4   # 5 üzerinden 4+ bull
    df["strong_bear_align"] = (5 - bull_score) >= 4

    # ================================================================
    # YENİ v4: ICT POWER OF 3 (akümülasyon/manipülasyon/dağıtım)
    # ================================================================
    # Günün ilk 2 saatinde range → manipulation zone
    df["po3_accumulation"] = df["hour"].between(0, 7)    # Asya akümülasyon
    df["po3_manipulation"] = df["hour"].between(8, 10)   # London manipülasyon
    df["po3_distribution"] = df["hour"].between(13, 17)  # NY dağıtım
    # London manipulation → NY distribution long setup
    df["po3_long_setup"]  = df["po3_distribution"] & up_c
    df["po3_short_setup"] = df["po3_distribution"] & down_c

    # ================================================================
    # YENİ v4: HIGHER HIGH / LOWER LOW YAPISAL TREND
    # ================================================================
    # Son 3 swing'in yönü
    hh3 = h.rolling(3).max()
    ll3 = l.rolling(3).min()
    df["hh_trend"] = (h > hh3.shift(3)) & (l > ll3.shift(3))   # HH + HL = uptrend
    df["ll_trend"] = (h < hh3.shift(3)) & (l < ll3.shift(3))   # LH + LL = downtrend

    # ================================================================
    # YENİ v4: RSI DIVERJANS (fiyat/RSI uyumsuzluğu)
    # ================================================================
    # Gizli bullish diverjans: fiyat düşük, RSI daha yüksek
    df["rsi_div_bull"] = (c < c.shift(5)) & (df["rsi14"] > df["rsi14"].shift(5)) & (df["rsi14"] < 50)
    df["rsi_div_bear"] = (c > c.shift(5)) & (df["rsi14"] < df["rsi14"].shift(5)) & (df["rsi14"] > 50)

    # C2: Rejim etiketi (BULL/BEAR/RANGE) — lookahead yok
    ema50_slope = c.ewm(span=50,adjust=False).mean()
    adx_col     = df["adx"] if "adx" in df.columns else pd.Series(0,index=df.index)
    df["regime_bull"]  = (ema50_slope > ema50_slope.shift(3)) & (adx_col > 25)
    df["regime_bear"]  = (ema50_slope < ema50_slope.shift(3)) & (adx_col > 25)
    df["regime_range"] = adx_col < 20
    # Numerik: 1=bull, -1=bear, 0=range
    regime_num = pd.Series(0, index=df.index)
    regime_num[df["regime_bull"]]  = 1
    regime_num[df["regime_bear"]]  = -1
    df["regime"] = regime_num

    return df.add_suffix(f"_{tf}")

print("\n[2/5] Feature engineering...")
df1  = add_features(df1,  "1m")
df5  = add_features(df5,  "5m")
df15 = add_features(df15, "15m")

# YENi v4: 1h HTF bias feature engineering
def add_features_1h(df):
    df = df.copy()
    c = df["close"]; h2 = df["high"]; l2 = df["low"]
    df["htf_ema20"]  = c.ewm(span=20,adjust=False).mean()
    df["htf_ema50"]  = c.ewm(span=50,adjust=False).mean()
    df["htf_bull"]   = (c > df["htf_ema20"]) & (df["htf_ema20"] > df["htf_ema50"])
    df["htf_bear"]   = (c < df["htf_ema20"]) & (df["htf_ema20"] < df["htf_ema50"])
    delta = c.diff()
    gain  = delta.clip(lower=0).rolling(14).mean()
    loss  = (-delta.clip(upper=0)).rolling(14).mean()
    htf_rsi = 100 - 100/(1+gain/(loss+1e-9))
    df["htf_rsi_bull"]  = htf_rsi > 50
    df["htf_rsi_bear"]  = htf_rsi < 50
    df["htf_uptrend"]   = df["htf_bull"] & df["htf_rsi_bull"]
    df["htf_downtrend"] = df["htf_bear"] & df["htf_rsi_bear"]
    df["htf_hh"] = h2 > h2.shift(1).rolling(5).max()
    df["htf_ll"] = l2 < l2.shift(1).rolling(5).min()
    return df.add_suffix("_1h")

df1h = add_features_1h(df1h)
df1h.rename(columns={"datetime_1h":"datetime"}, inplace=True)

for d,s in [(df1,"1m"),(df5,"5m"),(df15,"15m")]:
    d.rename(columns={f"datetime_{s}":"datetime"}, inplace=True)
df = pd.merge_asof(df1.sort_values("datetime"), df5.sort_values("datetime"),  on="datetime", direction="backward")
df = pd.merge_asof(df.sort_values("datetime"),  df15.sort_values("datetime"), on="datetime", direction="backward")
df = pd.merge_asof(df.sort_values("datetime"),  df1h.sort_values("datetime"), on="datetime", direction="backward")
df = df.dropna().reset_index(drop=True)
print(f"  Birlesik (1m+5m+15m+1h): {len(df):,} satir, {len(df.columns)} kolon")

df_oos = None

N            = len(df)
TRAIN_END    = int(N * CFG.TRAIN_RATIO)
RECENT_START = int(N * CFG.RECENT_RATIO)

close  = df["close_1m"].values
high   = df["high_1m"].values
low    = df["low_1m"].values
open_p = df["open_1m"].values
atr    = df["atr14_1m"].values

# DUZELTME: high_vol/low_vol quantile sadece train bolumunden
train_atr = df["atr14_1m"].iloc[:TRAIN_END]
q70 = float(train_atr.quantile(0.70))
q30 = float(train_atr.quantile(0.30))
df["high_vol_1m"] = (df["atr14_1m"] > q70).astype(bool)
df["low_vol_1m"]  = (df["atr14_1m"] < q30).astype(bool)
print(f"  high_vol esik (train q70): {q70:.4f}")

# ── OOS-1 FEATURE ENGINEERING (q70/q30 ana veriden — sızıntı yok) ──
df_oos = None
if oos1_available and df1_oos is not None:
    try:
        df1_oos  = add_features(df1_oos,  "1m")
        df5_oos  = add_features(df5_oos,  "5m")
        df15_oos = add_features(df15_oos, "15m")
        df1h_oos = add_features_1h(df1h_oos)
        for d,s in [(df1_oos,"1m"),(df5_oos,"5m"),(df15_oos,"15m")]:
            d.rename(columns={f"datetime_{s}":"datetime"}, inplace=True)
        df1h_oos.rename(columns={"datetime_1h":"datetime"}, inplace=True)
        df_oos = pd.merge_asof(df1_oos.sort_values("datetime"), df5_oos.sort_values("datetime"), on="datetime", direction="backward")
        df_oos = pd.merge_asof(df_oos.sort_values("datetime"),  df15_oos.sort_values("datetime"), on="datetime", direction="backward")
        df_oos = pd.merge_asof(df_oos.sort_values("datetime"),  df1h_oos.sort_values("datetime"), on="datetime", direction="backward")
        for col in df.columns:
            if col not in df_oos.columns: df_oos[col] = False
        df_oos = df_oos.dropna().reset_index(drop=True)
        df_oos["high_vol_1m"] = (df_oos["atr14_1m"] > q70).astype(bool)
        df_oos["low_vol_1m"]  = (df_oos["atr14_1m"] < q30).astype(bool)
        print(f"  OOS-1 birlesik: {len(df_oos):,} satir")
    except Exception as e:
        print(f"  OOS-1 FE hata: {e}")
        df_oos = None; oos1_available = False

# ── C1: OOS-2 FEATURE ENGINEERING (2010-2012) ──────────────────────
df_oos2 = None
oos2_available = False
if hist_available and df1_hist is not None:
    try:
        # Hist verileri zaten add_features ile birlestirilmedi — OOS-2 icin ayri islemek lazim
        # df1_hist vs df1: hist ana veriye concat edildi, OOS-2 icin orijinal hist gerekli
        # Tekrar yukle (temiz kopya)
        df1h2  = load_csv(CFG.HIST_M1_FILE)
        if os.path.exists(CFG.HIST_M5_FILE):
            df5h2  = load_csv(CFG.HIST_M5_FILE)
        else:
            df5h2  = resample_tf(df1h2, "5min")
        if os.path.exists(CFG.HIST_M15_FILE):
            df15h2 = load_csv(CFG.HIST_M15_FILE)
        else:
            df15h2 = resample_tf(df1h2, "15min")
        if os.path.exists(CFG.HIST_M1H_FILE):
            df1hh2 = load_csv(CFG.HIST_M1H_FILE)
        else:
            df1hh2 = resample_tf(df1h2, "1h")

        df1h2  = add_features(df1h2,  "1m")
        df5h2  = add_features(df5h2,  "5m")
        df15h2 = add_features(df15h2, "15m")
        df1hh2 = add_features_1h(df1hh2)
        for d,s in [(df1h2,"1m"),(df5h2,"5m"),(df15h2,"15m")]:
            d.rename(columns={f"datetime_{s}":"datetime"}, inplace=True)
        df1hh2.rename(columns={"datetime_1h":"datetime"}, inplace=True)
        df_oos2 = pd.merge_asof(df1h2.sort_values("datetime"),  df5h2.sort_values("datetime"),  on="datetime", direction="backward")
        df_oos2 = pd.merge_asof(df_oos2.sort_values("datetime"), df15h2.sort_values("datetime"), on="datetime", direction="backward")
        df_oos2 = pd.merge_asof(df_oos2.sort_values("datetime"), df1hh2.sort_values("datetime"), on="datetime", direction="backward")
        for col in df.columns:
            if col not in df_oos2.columns: df_oos2[col] = False
        df_oos2 = df_oos2.dropna().reset_index(drop=True)
        df_oos2["high_vol_1m"] = (df_oos2["atr14_1m"] > q70).astype(bool)
        df_oos2["low_vol_1m"]  = (df_oos2["atr14_1m"] < q30).astype(bool)
        oos2_available = True
        print(f"  OOS-2 (2010-2012) birlesik: {len(df_oos2):,} satir")
    except Exception as e:
        print(f"  OOS-2 FE hata: {e}")
        df_oos2 = None; oos2_available = False



def oos_verdict(res, label=""):
    if res is None: return None, "SINYAL YOK"
    ok = res["wr"]>=0.45 and res["pf"]>=1.0 and res["max_dd"]<=0.25
    v  = "GECTI" if ok else "BASARISIZ"
    ci = f"CI:[{res['ci_lo']:.1%}-{res['ci_hi']:.1%}]" if res.get("ci_lo") is not None else ""
    n_warn = " (N<265 az veri)" if res["total"]<265 else ""
    return ok, f"{'ok' if ok else 'x'} {v}  WR:{res['wr']:.1%}{n_warn} {ci}  PF:{res['pf']:.2f}  DD:{res['max_dd']:.1%}  N:{res['total']}"

def wilson_ci(wins, total, z=1.96):
    """B4: Wilson guven araligi."""
    if total == 0: return 0.0, 0.0
    p = wins/total
    center = (p + z**2/(2*total)) / (1 + z**2/total)
    margin = z*math.sqrt(p*(1-p)/total + z**2/(4*total**2)) / (1+z**2/total)
    return max(0.0, center-margin), min(1.0, center+margin)

# ================================================================
# LOT HESAPLAMA — XAUUSD 1:500 KALDIRAÇ
# ================================================================
def calc_lot(account_bal, atr_val, sl_mult):
    """
    Risk bazlı lot hesaplama.
    XAUUSD: 1 lot = 100 oz, pip degeri = $10/pip
    
    Formul: lot = risk_usd / (sl_mesafe * contract_size)
    """
    risk_usd  = account_bal * CFG.RISK_PCT          # $1000 * %2 = $20
    sl_dist   = atr_val * sl_mult                    # ATR * SL carpani
    if sl_dist <= 0: return CFG.MIN_LOT
    lot = risk_usd / (sl_dist * CFG.CONTRACT_SIZE)  # $20 / (5$ * 100) = 0.04
    lot = round(lot / CFG.LOT_STEP) * CFG.LOT_STEP  # 0.01'e yuvarla
    lot = max(CFG.MIN_LOT, min(CFG.MAX_LOT, lot))   # min/max sinir
    return lot

def calc_margin(lot, entry_price):
    """Gereken teminat."""
    return (lot * CFG.CONTRACT_SIZE * entry_price) / CFG.LEVERAGE

def check_margin(lot, entry_price, account_bal):
    """Teminat yeterli mi?"""
    margin = calc_margin(lot, entry_price)
    return margin <= account_bal * 0.8  # max bakiyenin %80'i margin

def ulcer_index(eq_curve):
    """D3: Ulcer Index — uzun sureli DD penalize eder."""
    if len(eq_curve) < 5: return 0.0
    eq = np.array(eq_curve)
    peak = np.maximum.accumulate(eq)
    dd = (peak - eq) / (peak + 1e-9)
    return float(np.sqrt(np.mean(dd**2)))

# ================================================================
# 3. BACKTEST CORE
# ================================================================
# ================================================================
# MALIYET HESAPLAMA YARDIMCILARI (A2, B1, B2, B3)
# ================================================================
# Dow: 0=Pzt ... 4=Cuma
_dow_arr = None  # lazy init

def get_trade_cost(lot, bar_idx):
    """B1+B2: Slippage + degisken spread (haber saatleri)."""
    spread = CFG.SPREAD_PTS
    try:
        dt     = df["datetime"].iloc[bar_idx]
        dow    = dt.weekday()      # 0=Pzt, 4=Cuma
        hour   = dt.hour
        minute = dt.minute
        if (dow, hour, minute) in CFG.NEWS_HOURS_UTC:
            spread = spread * CFG.SPREAD_NEWS_MULT
    except Exception:
        pass
    slip = CFG.SLIPPAGE_PTS
    return (spread + slip) * lot * CFG.CONTRACT_SIZE / 100 + CFG.COMMISSION * lot

def is_friday_cutoff(bar_idx, arr_len=None):
    """B3: Cuma 20:00 UTC sonrası giriş yapma."""
    try:
        dt = df["datetime"].iloc[bar_idx]
        if dt.weekday() == 4 and dt.hour >= CFG.FRIDAY_CUTOFF_HOUR:
            return True
    except Exception:
        pass
    return False

def resolve_outcome_ambiguous(low_j, high_j, sl, tp, direction, seed=None):
    """A2: Aynı barda hem SL hem TP — %50 rastgele seç."""
    sl_hit = (low_j <= sl) if direction=="long" else (high_j >= sl)
    tp_hit = (high_j >= tp) if direction=="long" else (low_j <= tp)
    if sl_hit and tp_hit:
        # Hangisi önce? Rastgele seç (en dürüst yöntem)
        return 1 if np.random.random() < 0.5 else -1
    elif sl_hit:
        return -1
    elif tp_hit:
        return 1
    return None

def get_oos_trade_cost(lot):
    """OOS için sabit maliyet (bar indeksi yok, df_oos kullanılıyor)."""
    return (CFG.SPREAD_PTS + CFG.SLIPPAGE_PTS) * lot * CFG.CONTRACT_SIZE / 100 + CFG.COMMISSION * lot


def run_backtest_range(mask_arr, direction, tp_mult, sl_mult, s, e):
    """WF pencere backtest — A2+B1+B2+B3 dahil."""
    idx = np.where(mask_arr[s:e])[0] + s
    wins=0; losses=0; gp=0.0; gl=0.0
    equity=CFG.ACCOUNT_SIZE; peak=equity; max_dd=0.0; pos_close_bar=-1
    for i in idx:
        if i+CFG.TIMEOUT_BARS>=e: continue
        if i<=pos_close_bar: continue
        if i+1>=e: continue
        if is_friday_cutoff(i): continue              # B3: Cuma gap
        a=atr[i]
        if a==0 or np.isnan(a): continue
        entry=open_p[i+1]
        if entry==0 or np.isnan(entry): continue
        lot=calc_lot(CFG.ACCOUNT_SIZE,a,1.0)
        if direction=="long": tp=entry+tp_mult*a; sl=entry-sl_mult*a
        else:                 tp=entry-tp_mult*a; sl=entry+sl_mult*a
        outcome=None
        for j in range(i+2,min(i+CFG.TIMEOUT_BARS+1,e)):
            o = resolve_outcome_ambiguous(low[j],high[j],sl,tp,direction)  # A2
            if o is not None: outcome=o; pos_close_bar=j; break
        if outcome is None: outcome=-1; pos_close_bar=i+CFG.TIMEOUT_BARS
        if outcome==1: wins+=1; gp+=tp_mult*a
        else:          losses+=1; gl+=sl_mult*a
        tc = get_trade_cost(lot, i)                   # B1+B2
        pnl=(lot*CFG.CONTRACT_SIZE*(tp_mult*a if outcome==1 else -sl_mult*a))-tc
        equity+=pnl
        if equity<=0: equity=0; break
        if equity>peak: peak=equity
        dd=(peak-equity)/peak
        if dd>max_dd: max_dd=dd
    total=wins+losses
    if total<1: return None
    return {"total":total,"wr":wins/total,"pf":gp/(gl+1e-9),"max_dd":max_dd,"final_equity":equity}

def run_backtest(mask_arr, direction, tp_mult, sl_mult, start_idx):
    """Ana backtest — A2+B1+B2+B3 dahil."""
    idx = np.where(mask_arr[start_idx:])[0] + start_idx
    wins=0; losses=0; gp=0.0; gl=0.0
    equity=CFG.ACCOUNT_SIZE; peak=equity; max_dd=0.0; eq_curve=[]; pos_close_bar=-1
    for i in idx:
        if i+CFG.TIMEOUT_BARS>=N: continue
        if i<=pos_close_bar: continue
        if i+1>=N: continue
        if is_friday_cutoff(i): continue              # B3
        a=atr[i]
        if a==0 or np.isnan(a): continue
        entry=open_p[i+1]
        if entry==0 or np.isnan(entry): continue
        lot=calc_lot(CFG.ACCOUNT_SIZE,a,sl_mult)
        if not check_margin(lot,entry,CFG.ACCOUNT_SIZE):
            lot=max(CFG.MIN_LOT,lot*0.5)
        if direction=="long": tp=entry+tp_mult*a; sl=entry-sl_mult*a
        else:                 tp=entry-tp_mult*a; sl=entry+sl_mult*a
        outcome=None
        for j in range(i+2,min(i+CFG.TIMEOUT_BARS+1,N)):
            o=resolve_outcome_ambiguous(low[j],high[j],sl,tp,direction)  # A2
            if o is not None: outcome=o; pos_close_bar=j; break
        if outcome is None: outcome=-1; pos_close_bar=i+CFG.TIMEOUT_BARS
        tc=get_trade_cost(lot,i)                      # B1+B2
        if outcome==1:
            pnl_usd=lot*CFG.CONTRACT_SIZE*(tp_mult*a)-tc; wins+=1; gp+=tp_mult*a
        else:
            pnl_usd=-lot*CFG.CONTRACT_SIZE*(sl_mult*a)-tc; losses+=1; gl+=sl_mult*a
        equity+=pnl_usd
        if equity<=0: equity=0; break
        if equity>peak: peak=equity
        dd=(peak-equity)/peak
        if dd>max_dd: max_dd=dd
        eq_curve.append(equity)
    total=wins+losses
    if total<1: return None
    return {"total":total,"wins":wins,"losses":losses,"wr":wins/total,"pf":gp/(gl+1e-9),
            "max_dd":max_dd,"final_equity":equity,"eq_curve":eq_curve}

# ================================================================
# 4. KOMBİNASYON TANIMLA & TARA
# ================================================================
LONG_ICT  = {
    # v3 orijinal
    "liq_bull":     "liq_bull_1m",
    "fvg_bull":     "fvg_bull_1m",
    "ob_bull":      "ob_bull_1m",
    "bos_bull":     "bos_bull_1m",
    "hammer":       "hammer_1m",
    "at_bb_lower":  "at_bb_lower_1m",
    "consec_dn":    "consec_dn_1m",
    "bull_eng":     "bull_eng_1m",
    "mit_bull":     "mit_bull_1m",
    "in_discount":  "in_discount_1m",
    "equal_lows":   "equal_lows_1m",
    "ib_break_up":  "ib_break_up_1m",
    "morning_star": "morning_star_1m",
    "dc_breakout_up":"dc_breakout_up_1m",
    "unicorn_bull": "unicorn_bull_1m",
    "breaker_bull": "breaker_bull_1m",
    "ote_bull":     "ote_bull_1m",
    "tweezer_bot":  "tweezer_bot_1m",
    "three_soldiers":"three_soldiers_1m",
    "piercing":     "piercing_1m",
    "bull_harami":  "bull_harami_1m",
    "bull_marubozu":"bull_marubozu_1m",
    "at_kc_lower":  "at_kc_lower_1m",
    "psar_flip_bull":"real_psar_flip_bull_1m",  # v4: gercek PSAR
    "judas_long":   "judas_long_1m",
    # v4 YENi ICT/SMC
    "choch_bull":        "choch_bull_1m",
    "choch_bull_strong": "choch_bull_strong_1m",
    "ob_bull_strong":    "ob_bull_strong_1m",
    "unicorn_bull_strong":"unicorn_bull_strong_1m",
    "wyckoff_spring":    "wyckoff_spring_1m",
    "real_psar_flip_bull":"real_psar_flip_bull_1m",
    "in_golden_zone_bull":"in_golden_zone_bull_1m",
    "at_fib618_support":  "at_fib618_support_1m",
    "at_fib382_support":  "at_fib382_support_1m",
    "hh_trend":          "hh_trend_1m",
    "rsi_div_bull":      "rsi_div_bull_1m",
    "bb_breakout_up":    "bb_breakout_up_1m",
    "vol_confirm_bull":  "vol_confirm_bull_1m",
    "strong_bull_align": "strong_bull_align_1m",
    "wyckoff_spring_5m": "wyckoff_spring_5m",
}
LONG_FILT = {
    # osilatör
    "rsi_os":       "rsi_os_1m",
    "rsi_os2":      "rsi_os2_1m",
    "stoch_os":     "stoch_os_1m",
    "stoch_os2":    "stoch_os2_1m",
    "macd_up":      "macd_up_1m",
    "willr_os":     "willr_os_1m",
    "cci_os":       "cci_os_1m",
    "rsi_rising":   "rsi_rising_1m",
    "rsi_div_bull": "rsi_div_bull_1m",
    # trend
    "ema_bull_5m":       "ema_bull_5m",
    "ema_bull_15m":      "ema_bull_15m",
    "above_vwap_5m":     "above_vwap_5m",
    "trend_up_15m":      "trend_up_15m",
    "adx_strong":        "adx_strong_1m",
    "vol_expand":        "vol_expand_1m",
    "above_dc_mid_5m":   "above_dc_mid_5m",
    "ema21_slope_up_5m": "ema21_slope_up_5m",
    "adx_strong_15m":    "adx_strong_15m",
    "ichi_bull":         "ichi_bull_1m",
    "ichi_cross_bull":   "ichi_cross_bull_1m",
    "ichi_bull_15m":     "ichi_bull_15m",
    "real_psar_bull":    "real_psar_bull_1m",
    "real_psar_bull_5m": "real_psar_bull_5m",
    "ao_pos":            "ao_pos_1m",
    "roc_pos":           "roc_pos_1m",
    "obv_up":            "obv_up_1m",
    "buy_pressure":      "buy_pressure_1m",
    "hh_trend":          "hh_trend_1m",
    "strong_bull_align": "strong_bull_align_1m",
    # seans
    "session":           "session_1m",
    "london":            "london_1m",
    "ny":                "ny_1m",
    "killzone_london":   "killzone_london_1m",
    "killzone_ny":       "killzone_ny_1m",
    "london_ny_overlap": "london_ny_overlap_1m",
    "silver_bullet_am":  "silver_bullet_am_1m",
    "po3_distribution":  "po3_distribution_1m",
    # hacim
    "high_vol":          "high_vol_1m",
    "vol_spike":         "vol_spike_1m",
    "mom3_up":           "mom3_up_1m",
    "above_pp":          "above_pp_1m",
    # HTF 1h bias (v4 YENi)
    "htf_uptrend":       "htf_uptrend_1h",
    "htf_bull_1h":       "htf_bull_1h",
    "htf_rsi_bull_1h":   "htf_rsi_bull_1h",
    # kalite
    "quality_candle":    "quality_candle_1m",
    "no_doji":           "no_doji_1m",
    "atr_pct80":         "atr_pct80_1m",
    "in_golden_zone_bull":"in_golden_zone_bull_1m",
}

SHORT_ICT  = {
    # v3 orijinal
    "liq_bear":   "liq_bear_1m",
    "fvg_bear":   "fvg_bear_1m",
    "ob_bear":    "ob_bear_1m",
    "bos_bear":   "bos_bear_1m",
    "inv_hammer": "inv_hammer_1m",
    "at_bb_upper":"at_bb_upper_1m",
    "consec_up":  "consec_up_1m",
    "bear_eng":   "bear_eng_1m",
    "mit_bear":       "mit_bear_1m",
    "in_premium":     "in_premium_1m",
    "equal_highs":    "equal_highs_1m",
    "ib_break_dn":    "ib_break_dn_1m",
    "evening_star":   "evening_star_1m",
    "dc_breakout_dn": "dc_breakout_dn_1m",
    "unicorn_bear":  "unicorn_bear_1m",
    "breaker_bear":  "breaker_bear_1m",
    "ote_bear":      "ote_bear_1m",
    "tweezer_top":   "tweezer_top_1m",
    "three_crows":   "three_crows_1m",
    "dark_cloud":    "dark_cloud_1m",
    "bear_harami":   "bear_harami_1m",
    "bear_marubozu": "bear_marubozu_1m",
    "shooting_star": "shooting_star_1m",
    "at_kc_upper":   "at_kc_upper_1m",
    "psar_flip_bear":"real_psar_flip_bear_1m",  # v4: gercek PSAR
    "judas_short":   "judas_short_1m",
    # v4 YENi ICT/SMC
    "choch_bear":        "choch_bear_1m",
    "choch_bear_strong": "choch_bear_strong_1m",
    "ob_bear_strong":    "ob_bear_strong_1m",
    "unicorn_bear_strong":"unicorn_bear_strong_1m",
    "wyckoff_upthrust":  "wyckoff_upthrust_1m",
    "real_psar_flip_bear":"real_psar_flip_bear_1m",
    "in_golden_zone_bear":"in_golden_zone_bear_1m",
    "at_fib618_resist":   "at_fib618_resist_1m",
    "at_fib382_resist":   "at_fib382_resist_1m",
    "ll_trend":          "ll_trend_1m",
    "rsi_div_bear":      "rsi_div_bear_1m",
    "bb_breakout_dn":    "bb_breakout_dn_1m",
    "vol_confirm_bear":  "vol_confirm_bear_1m",
    "strong_bear_align": "strong_bear_align_1m",
    "wyckoff_upthrust_5m":"wyckoff_upthrust_5m",
}
SHORT_FILT = {
    # osilatör
    "rsi_ob":        "rsi_ob_1m",
    "rsi_ob2":       "rsi_ob2_1m",
    "stoch_ob":      "stoch_ob_1m",
    "stoch_ob2":     "stoch_ob2_1m",
    "macd_dn":       "macd_dn_1m",
    "willr_ob":      "willr_ob_1m",
    "cci_ob":        "cci_ob_1m",
    "rsi_falling":   "rsi_falling_1m",
    "rsi_div_bear":  "rsi_div_bear_1m",
    # trend
    "ema_bear_5m":       "ema_bear_5m",
    "ema_bear_15m":      "ema_bear_15m",
    "below_vwap_5m":     "below_vwap_5m",
    "trend_dn_15m":      "trend_dn_15m",
    "adx_strong":        "adx_strong_1m",
    "vol_expand":        "vol_expand_1m",
    "below_dc_mid_5m":   "below_dc_mid_5m",
    "ema21_slope_dn_5m": "ema21_slope_dn_5m",
    "adx_strong_15m":    "adx_strong_15m",
    "ichi_bear":         "ichi_bear_1m",
    "ichi_cross_bear":   "ichi_cross_bear_1m",
    "ichi_bear_15m":     "ichi_bear_15m",
    "real_psar_bear":    "real_psar_bear_1m",
    "real_psar_bear_5m": "real_psar_bear_5m",
    "ao_neg":            "ao_neg_1m",
    "roc_neg":           "roc_neg_1m",
    "obv_dn":            "obv_dn_1m",
    "sell_pressure":     "sell_pressure_1m",
    "ll_trend":          "ll_trend_1m",
    "strong_bear_align": "strong_bear_align_1m",
    # seans
    "session":           "session_1m",
    "london":            "london_1m",
    "ny":                "ny_1m",
    "killzone_london":   "killzone_london_1m",
    "killzone_ny":       "killzone_ny_1m",
    "london_ny_overlap": "london_ny_overlap_1m",
    "silver_bullet_pm":  "silver_bullet_pm_1m",
    "po3_distribution":  "po3_distribution_1m",
    # hacim
    "high_vol":          "high_vol_1m",
    "vol_spike":         "vol_spike_1m",
    "mom3_dn":           "mom3_dn_1m",
    "below_pp":          "below_pp_1m",
    # HTF 1h bias (v4 YENi)
    "htf_downtrend":     "htf_downtrend_1h",
    "htf_bear_1h":       "htf_bear_1h",
    "htf_rsi_bear_1h":   "htf_rsi_bear_1h",
    # kalite
    "quality_candle":    "quality_candle_1m",
    "no_doji":           "no_doji_1m",
    "atr_pct80":         "atr_pct80_1m",
    "in_golden_zone_bear":"in_golden_zone_bear_1m",
}


avail = set(df.columns)
LONG_ICT   = {k:v for k,v in LONG_ICT.items()   if v in avail}
LONG_FILT  = {k:v for k,v in LONG_FILT.items()  if v in avail}
SHORT_ICT  = {k:v for k,v in SHORT_ICT.items()  if v in avail}
SHORT_FILT = {k:v for k,v in SHORT_FILT.items() if v in avail}

# ================================================================
# BİDİRECTİONAL (İKİ YÖNLÜ) STRATEJİ DESTEĞİ
# ================================================================
# Yön-bağımsız ICT sinyalleri: piyasa durumuna göre long veya short
# Bu sinyaller tek başına yön belirtmez, filtreler yönü belirler
BIDIR_ICT = {
    # Likidite & yapı
    "liq_sweep":   None,   # long: liq_bull, short: liq_bear
    "fvg":         None,
    "ob":          None,
    "bos":         None,
    "eng":         None,
    "mit":         None,
    "ib_break":    None,
    "bb_extreme":  None,
    "kc_extreme":  None,
    "dc_breakout": None,
    "ote":         None,
    "psar_flip":   None,
    "unicorn":     None,
    "breaker":     None,
    "judas":       None,
}

# Yön belirleyici ortak filtreler (her ikisi için çalışan)
BIDIR_FILT = {
    # Trend yon belirleyiciler — (long_kolon, short_kolon)
    "ema_trend_5m":      ("ema_bull_5m",        "ema_bear_5m"),
    "ema_trend_15m":     ("ema_bull_15m",        "ema_bear_15m"),
    "vwap_side_5m":      ("above_vwap_5m",       "below_vwap_5m"),
    "trend_15m":         ("trend_up_15m",         "trend_dn_15m"),
    "dc_mid_5m":         ("above_dc_mid_5m",      "below_dc_mid_5m"),
    "ema21_slope_5m":    ("ema21_slope_up_5m",    "ema21_slope_dn_5m"),
    "roc_dir":           ("roc_pos_1m",            "roc_neg_1m"),
    "ao_dir":            ("ao_pos_1m",             "ao_neg_1m"),
    "obv_dir":           ("obv_up_1m",             "obv_dn_1m"),
    # v4 YENi: gercek PSAR yonu
    "real_psar_1m":      ("real_psar_bull_1m",     "real_psar_bear_1m"),
    "real_psar_5m":      ("real_psar_bull_5m",     "real_psar_bear_5m"),
    # v4 YENi: HTF 1h bias
    "htf_trend_1h":      ("htf_uptrend_1h",        "htf_downtrend_1h"),
    "htf_ema_1h":        ("htf_bull_1h",            "htf_bear_1h"),
    "htf_rsi_1h":        ("htf_rsi_bull_1h",        "htf_rsi_bear_1h"),
    # v4 YENi: momentum + yapisal
    "hh_ll_trend":       ("hh_trend_1m",            "ll_trend_1m"),
    "pressure":          ("buy_pressure_1m",         "sell_pressure_1m"),
    "strong_align":      ("strong_bull_align_1m",   "strong_bear_align_1m"),
    "rsi_div":           ("rsi_div_bull_1m",         "rsi_div_bear_1m"),
}
# None icerenleri temizle
BIDIR_FILT_RAW = {}
for k,(lv,sv) in BIDIR_FILT.items():
    if lv and sv and lv in avail and sv in avail:
        BIDIR_FILT_RAW[k] = (lv,sv)
BIDIR_FILT = BIDIR_FILT_RAW

# Ikili ICT eslestirme: her sinyal icin (long_col, short_col)
BIDIR_ICT_MAP = [
    # v3 orijinal ciftler
    ("liq_bull_1m",           "liq_bear_1m"),
    ("fvg_bull_1m",           "fvg_bear_1m"),
    ("ob_bull_1m",            "ob_bear_1m"),
    ("bos_bull_1m",           "bos_bear_1m"),
    ("bull_eng_1m",           "bear_eng_1m"),
    ("mit_bull_1m",           "mit_bear_1m"),
    ("ib_break_up_1m",        "ib_break_dn_1m"),
    ("at_bb_lower_1m",        "at_bb_upper_1m"),
    ("at_kc_lower_1m",        "at_kc_upper_1m"),
    ("dc_breakout_up_1m",     "dc_breakout_dn_1m"),
    ("ote_bull_1m",           "ote_bear_1m"),
    ("real_psar_flip_bull_1m","real_psar_flip_bear_1m"),  # v4: gercek PSAR
    ("unicorn_bull_1m",       "unicorn_bear_1m"),
    ("breaker_bull_1m",       "breaker_bear_1m"),
    ("judas_long_1m",         "judas_short_1m"),
    ("hammer_1m",             "inv_hammer_1m"),
    ("morning_star_1m",       "evening_star_1m"),
    ("tweezer_bot_1m",        "tweezer_top_1m"),
    ("three_soldiers_1m",     "three_crows_1m"),
    ("bull_harami_1m",        "bear_harami_1m"),
    ("bull_marubozu_1m",      "bear_marubozu_1m"),
    ("piercing_1m",           "dark_cloud_1m"),
    ("in_discount_1m",        "in_premium_1m"),
    ("equal_lows_1m",         "equal_highs_1m"),
    # v4 YENi ciftler
    ("choch_bull_1m",         "choch_bear_1m"),
    ("choch_bull_strong_1m",  "choch_bear_strong_1m"),
    ("ob_bull_strong_1m",     "ob_bear_strong_1m"),
    ("unicorn_bull_strong_1m","unicorn_bear_strong_1m"),
    ("wyckoff_spring_1m",     "wyckoff_upthrust_1m"),
    ("hh_trend_1m",           "ll_trend_1m"),
    ("rsi_div_bull_1m",       "rsi_div_bear_1m"),
    ("bb_breakout_up_1m",     "bb_breakout_dn_1m"),
    ("in_golden_zone_bull_1m","in_golden_zone_bear_1m"),
    ("strong_bull_align_1m",  "strong_bear_align_1m"),
]
# Sadece mevcut kolonlari filtrele
BIDIR_ICT_MAP = [(lc,sc) for lc,sc in BIDIR_ICT_MAP if lc in avail and sc in avail]


def build_combos(ict, filt):
    combos=[]
    # 2-kosul: 1 ICT + 1 filtre (overfit riski dusuk)
    for ik,iv in ict.items():
        for fk,fv in filt.items():
            combos.append({"keys":[ik,fk],"cols":[iv,fv]})
    # 3-kosul: 1 ICT + 2 filtre (sadece MTF olanlarla)
    mtf_filt = {k:v for k,v in filt.items() if "_5m" in v or "_15m" in v}
    onem_filt = {k:v for k,v in filt.items() if "_1m" in v}
    for ik,iv in ict.items():
        for mk,mv in mtf_filt.items():
            for fk,fv in onem_filt.items():
                if fk!=mk:
                    combos.append({"keys":[ik,mk,fk],"cols":[iv,mv,fv]})
    return combos

def build_bidir_combos():
    """
    İki yönlü strateji kombinasyonları.
    Her combo: ICT çifti + yön filtresi.
    Yön filtresi True → long yap, False → short yap.
    """
    combos = []
    filt_items = list(BIDIR_FILT.items())
    # 2-koşul: 1 ICT çifti + 1 yön filtresi
    for lc, sc in BIDIR_ICT_MAP:
        ict_name = lc.replace("_1m","").replace("_bull","").replace("_bear","").replace("_up","").replace("_dn","")
        for fk, (fl, fs) in filt_items:
            combos.append({
                "keys":   [ict_name, fk],
                "long_cols":  [lc, fl],   # bu kolonlar True → long
                "short_cols": [sc, fs],   # bu kolonlar True → short
            })
    # 3-koşul: 1 ICT çifti + 2 yön filtresi (MTF + 1m)
    mtf_filt  = {k:(lv,sv) for k,(lv,sv) in BIDIR_FILT.items() if "_5m" in lv or "_15m" in lv}
    onem_filt = {k:(lv,sv) for k,(lv,sv) in BIDIR_FILT.items() if "_1m" in lv}
    for lc, sc in BIDIR_ICT_MAP:
        ict_name = lc.replace("_1m","").replace("_bull","").replace("_bear","").replace("_up","").replace("_dn","")
        for fk1,(fl1,fs1) in mtf_filt.items():
            for fk2,(fl2,fs2) in onem_filt.items():
                if fk1 != fk2:
                    combos.append({
                        "keys":       [ict_name, fk1, fk2],
                        "long_cols":  [lc,  fl1, fl2],
                        "short_cols": [sc,  fs1, fs2],
                    })
    return combos

long_combos  = build_combos(LONG_ICT,  LONG_FILT)
short_combos = build_combos(SHORT_ICT, SHORT_FILT)
bidir_combos = build_bidir_combos()
total_combos = len(long_combos)+len(short_combos)+len(bidir_combos)

print(f"\n[3/5] Strateji taramasi basliyor...")
print(f"  Tek yonlu kombinasyon: {len(long_combos)+len(short_combos):,} | Cift yonlu: {len(bidir_combos):,}")
print(f"  Toplam kombinasyon: {total_combos:,} | TP/SL grid: {len(CFG.TP_SL_GRID)} | Toplam test: {total_combos*len(CFG.TP_SL_GRID):,}")
print(f"  Filtreler: min {CFG.MIN_TRADES} trade | PF>{CFG.MIN_PF} | WR>{CFG.MIN_WR} | MaxDD<{CFG.MAX_DD:.0%}\n")

all_results = []
tested = 0

def evaluate(combo, direction):
    global tested
    cols = combo["cols"]
    keys = combo["keys"]
    mask = np.ones(N, dtype=bool)
    for col in cols:
        mask &= df[col].values.astype(bool)
    sig_test = mask[TRAIN_END:].sum()
    if sig_test < CFG.MIN_TRADES // 2:
        tested += 1
        return
    best = None
    for tp_mult, sl_mult in CFG.TP_SL_GRID:
        # Ana test (%30)
        r = run_backtest(mask, direction, tp_mult, sl_mult, TRAIN_END)
        if r is None or r["total"] < CFG.MIN_TRADES: continue
        if r["pf"] < CFG.MIN_PF: continue
        if r["wr"] < CFG.MIN_WR: continue
        if r["max_dd"] > CFG.MAX_DD: continue
        # Yakin donem dogrulama (%15)
        rr = run_backtest(mask, direction, tp_mult, sl_mult, RECENT_START)
        rec_wr  = rr["wr"]  if rr else 0
        rec_pf  = rr["pf"]  if rr else 0
        rec_dd  = rr["max_dd"] if rr else 1
        rec_n   = rr["total"] if rr else 0
        # ================================================================
        # OVERFiT FiLTRESi v4 — guclendirilmis
        # ================================================================
        # 1. Yakin donem tutarlilik (daha siki esikler)
        if rec_n < CFG.MIN_RECENT_TRADES: continue
        if rec_pf < CFG.MIN_PF * 0.82: continue
        if rec_wr < CFG.MIN_WR * 0.88: continue
        if rec_dd > CFG.MAX_DD * 1.20: continue
        # 2. v4 YENi: WR ve PF decay kontrol
        wr_decay = (r["wr"] - rec_wr) / (r["wr"]+1e-9)
        pf_decay = (r["pf"] - rec_pf) / (r["pf"]+1e-9)
        if wr_decay > CFG.MAX_WR_DECAY: continue   # WR %15+ duste
        if pf_decay > CFG.MAX_PF_DECAY: continue   # PF %35+ duste
        # 3. Sharpe proxy (1m icin annualize)
        eq_curve = r.get("eq_curve", [])
        if len(eq_curve) > 10:
            eq_arr = np.array(eq_curve)
            rets   = np.diff(eq_arr) / (eq_arr[:-1]+1e-9)
            sharpe = (rets.mean()/(rets.std()+1e-9)) * np.sqrt(252*6.5*60)
            if sharpe < 0.35: continue   # v4: 0.3→0.35
        # 4. Calmar proxy
        total_ret = (r["final_equity"] - CFG.ACCOUNT_SIZE) / CFG.ACCOUNT_SIZE
        calmar = total_ret / (r["max_dd"]+1e-9)
        if calmar < 1.0: continue   # v4: 0.8→1.0
        # 5. Rejim tutarlilik — equity 4e bolunur (eski: 3)
        if len(eq_curve) > 40:
            eq_arr = np.array(eq_curve)
            q = len(eq_arr)//4
            e1 = (eq_arr[q]   - CFG.ACCOUNT_SIZE)   / CFG.ACCOUNT_SIZE
            e2 = (eq_arr[2*q] - eq_arr[q])          / (eq_arr[q]+1e-9)
            e3 = (eq_arr[3*q] - eq_arr[2*q])        / (eq_arr[2*q]+1e-9)
            e4 = (eq_arr[-1]  - eq_arr[3*q])        / (eq_arr[3*q]+1e-9)
            # Hic bir ceyrek -7%den daha fazla kayipli olmamali
            if any(e < -0.07 for e in [e1,e2,e3,e4]): continue
            # Son ceyrek ilk ceyrege gore 20x fazla kazanamaz (curve fitting)
            if e4 > max(e1+0.01, 0.03) * 20: continue
        # 6. v4 YENi: Sortino proxy (sadece negatif volatilite)
        if len(eq_curve) > 10:
            eq_arr = np.array(eq_curve)
            rets   = np.diff(eq_arr) / (eq_arr[:-1]+1e-9)
            neg_rets = rets[rets < 0]
            if len(neg_rets) > 5:
                sortino = (rets.mean()/(neg_rets.std()+1e-9)) * np.sqrt(252*6.5*60)
                if sortino < 0.5: continue
        # 7. Stabilite skorlari
        pf_stability = min(1.0, rec_pf / (r["pf"]+1e-9))
        wr_stability = min(1.0, rec_wr / (r["wr"]+1e-9))
        dd_stability = min(1.0, CFG.MAX_DD / (rec_dd+1e-9))
        decay_penalty = max(0.0, 1.0 - wr_decay*2 - pf_decay)  # decay cezasi
        # 8. Composite score (D3: Ulcer Index + decay penalty)
        ui = ulcer_index(eq_curve)
        ui_factor = 1.0 / (1.0 + ui * 5)  # yüksek Ulcer = düşük skor
        score = (r["pf"] * r["wr"] * (1-r["max_dd"])
                 * pf_stability * wr_stability * dd_stability
                 * decay_penalty * ui_factor
                 * min(2.0, calmar/3))
        if best is None or score > best["score"]:
            best = {
                "name":       direction.upper()+"_"+"_".join(keys),
                "direction":  direction,
                "conditions": cols,
                "keys":       keys,
                "tp_mult":    tp_mult,
                "sl_mult":    sl_mult,
                "rr":         round(tp_mult/sl_mult,2),
                # Ana test
                "total_trades": r["total"],
                "win_rate":     round(r["wr"],4),
                "profit_factor":round(r["pf"],4),
                "max_dd":       round(r["max_dd"],4),
                "final_equity": round(r["final_equity"],2),
                # Yakin donem
                "rec_trades":   rec_n,
                "rec_wr":       round(rec_wr,4),
                "rec_pf":       round(rec_pf,4),
                "rec_dd":       round(rec_dd,4),
                "score":        round(score,6),
                "eq_curve":     r["eq_curve"],
            }
    if best:
        all_results.append(best)
    tested += 1
    if tested % 300 == 0:
        print(f"  {tested:>6}/{total_combos} | Bulunan: {len(all_results)}")

for combo in long_combos:
    evaluate(combo, "long")
for combo in short_combos:
    evaluate(combo, "short")

# ── BİDİRECTİONAL BACKTEST ─────────────────────────────────────
def run_backtest_bidir(long_mask, short_mask, tp_mult, sl_mult, start_idx):
    """
    Her sinyalde piyasanın yönüne göre long veya short girer.
    Long mask True → long gir, Short mask True → short gir.
    Aynı barda ikisi birden True ise LONG önceliklidir (trending bias).
    """
    # Tüm sinyal barlarını birleştir, yönü belirle
    combined = np.zeros(N, dtype=np.int8)  # 0=sinyal yok, 1=long, -1=short
    combined[long_mask]  = 1
    combined[short_mask] = np.where(combined[short_mask] == 0, -1, combined[short_mask])

    idx = np.where(combined[start_idx:] != 0)[0] + start_idx
    wins=0; losses=0; gp=0.0; gl=0.0
    equity = CFG.ACCOUNT_SIZE
    peak   = equity
    max_dd = 0.0
    eq_curve = []
    pos_close_bar = -1
    long_wins=0; long_losses=0; short_wins=0; short_losses=0

    for i in idx:
        if i+CFG.TIMEOUT_BARS >= N: continue
        if i <= pos_close_bar: continue
        if i+1 >= N: continue
        a = atr[i]
        if a == 0 or np.isnan(a): continue
        entry = open_p[i+1]
        if entry == 0 or np.isnan(entry): continue

        if is_friday_cutoff(i): continue               # B3
        direction = "long" if combined[i] == 1 else "short"
        lot = calc_lot(CFG.ACCOUNT_SIZE, a, sl_mult)
        if not check_margin(lot, entry, CFG.ACCOUNT_SIZE):
            lot = max(CFG.MIN_LOT, lot * 0.5)

        if direction == "long":
            tp = entry + tp_mult * a
            sl = entry - sl_mult * a
        else:
            tp = entry - tp_mult * a
            sl = entry + sl_mult * a

        outcome = None
        for j in range(i+2, min(i+CFG.TIMEOUT_BARS+1, N)):
            o = resolve_outcome_ambiguous(low[j],high[j],sl,tp,direction)  # A2
            if o is not None: outcome=o; pos_close_bar=j; break
        if outcome is None:
            outcome = -1
            pos_close_bar = i + CFG.TIMEOUT_BARS

        spread_cost = CFG.SPREAD_PTS * lot * CFG.CONTRACT_SIZE / 100
        comm_cost   = CFG.COMMISSION * lot
        trade_cost  = spread_cost + comm_cost

        if outcome == 1:
            pnl_usd = lot * CFG.CONTRACT_SIZE * (tp_mult * a) - trade_cost
            wins += 1; gp += tp_mult * a
            if direction == "long": long_wins += 1
            else: short_wins += 1
        else:
            pnl_usd = -lot * CFG.CONTRACT_SIZE * (sl_mult * a) - trade_cost
            losses += 1; gl += sl_mult * a
            if direction == "long": long_losses += 1
            else: short_losses += 1

        equity += pnl_usd
        if equity <= 0: equity = 0; break
        if equity > peak: peak = equity
        dd = (peak - equity) / peak
        if dd > max_dd: max_dd = dd
        eq_curve.append(equity)

    total = wins + losses
    if total < 1: return None
    return {
        "total": total, "wins": wins, "losses": losses,
        "wr": wins/total, "pf": gp/(gl+1e-9),
        "max_dd": max_dd, "final_equity": equity, "eq_curve": eq_curve,
        "long_trades": long_wins+long_losses, "short_trades": short_wins+short_losses,
        "long_wr": long_wins/(long_wins+long_losses+1e-9),
        "short_wr": short_wins/(short_wins+short_losses+1e-9),
    }

def run_backtest_bidir_range(long_mask, short_mask, tp_mult, sl_mult, s, e):
    combined = np.zeros(N, dtype=np.int8)
    combined[long_mask]  = 1
    combined[short_mask] = np.where(combined[short_mask] == 0, -1, combined[short_mask])
    idx = np.where(combined[s:e] != 0)[0] + s
    wins=0; losses=0; gp=0.0; gl=0.0
    equity=CFG.ACCOUNT_SIZE; peak=equity; max_dd=0.0; pos_close_bar=-1
    for i in idx:
        if i+CFG.TIMEOUT_BARS >= e: continue
        if i <= pos_close_bar: continue
        if i+1 >= e: continue
        a = atr[i]
        if a==0 or np.isnan(a): continue
        entry = open_p[i+1]
        if entry==0 or np.isnan(entry): continue
        if is_friday_cutoff(i): continue               # B3
        direction = "long" if combined[i]==1 else "short"
        lot = calc_lot(CFG.ACCOUNT_SIZE, a, sl_mult)
        if direction=="long": tp=entry+tp_mult*a; sl=entry-sl_mult*a
        else:                 tp=entry-tp_mult*a; sl=entry+sl_mult*a
        outcome=None
        for j in range(i+2, min(i+CFG.TIMEOUT_BARS+1, e)):
            o=resolve_outcome_ambiguous(low[j],high[j],sl,tp,direction)  # A2
            if o is not None: outcome=o; pos_close_bar=j; break
        if outcome is None: outcome=-1; pos_close_bar=i+CFG.TIMEOUT_BARS
        sc = CFG.SPREAD_PTS*lot*CFG.CONTRACT_SIZE/100+CFG.COMMISSION*lot
        pnl = (lot*CFG.CONTRACT_SIZE*(tp_mult*a if outcome==1 else -sl_mult*a)) - sc
        if outcome==1: wins+=1; gp+=tp_mult*a
        else:          losses+=1; gl+=sl_mult*a
        equity += pnl
        if equity<=0: equity=0; break
        if equity>peak: peak=equity
        dd=(peak-equity)/peak
        if dd>max_dd: max_dd=dd
    total=wins+losses
    if total<1: return None
    return {"total":total,"wr":wins/total,"pf":gp/(gl+1e-9),"max_dd":max_dd,"final_equity":equity}

def evaluate_bidir(combo):
    global tested
    keys      = combo["keys"]
    long_cols = combo["long_cols"]
    short_cols= combo["short_cols"]

    # Maskeleri oluştur
    long_mask  = np.ones(N, dtype=bool)
    short_mask = np.ones(N, dtype=bool)
    for col in long_cols:
        if col not in df.columns: tested+=1; return
        long_mask  &= df[col].values.astype(bool)
    for col in short_cols:
        if col not in df.columns: tested+=1; return
        short_mask &= df[col].values.astype(bool)

    # Test bölümündeki toplam sinyal sayısı
    sig_long  = long_mask[TRAIN_END:].sum()
    sig_short = short_mask[TRAIN_END:].sum()
    sig_total = sig_long + sig_short
    if sig_total < CFG.MIN_TRADES // 2:
        tested += 1; return

    best = None
    for tp_mult, sl_mult in CFG.TP_SL_GRID:
        r = run_backtest_bidir(long_mask, short_mask, tp_mult, sl_mult, TRAIN_END)
        if r is None or r["total"] < CFG.MIN_TRADES: continue
        if r["pf"]    < CFG.MIN_PF:    continue
        if r["wr"]    < CFG.MIN_WR:    continue
        if r["max_dd"]> CFG.MAX_DD:    continue

        # Her iki yön de çalışıyor mu? (en az %20 trade her yönde)
        total_t = r["total"]
        if r["long_trades"]  < total_t * 0.10: continue  # çok az long
        if r["short_trades"] < total_t * 0.10: continue  # çok az short
        # Her iki yönün WR'si de makul olmalı
        if r["long_wr"]  < 0.35: continue
        if r["short_wr"] < 0.35: continue

        # Yakın dönem doğrulama
        rr = run_backtest_bidir(long_mask, short_mask, tp_mult, sl_mult, RECENT_START)
        rec_wr  = rr["wr"]     if rr else 0
        rec_pf  = rr["pf"]     if rr else 0
        rec_dd  = rr["max_dd"] if rr else 1
        rec_n   = rr["total"]  if rr else 0

        # ================================================================
        # OVERFiT FiLTRESi v4 — bidir icin guclendirilmis
        # ================================================================
        if rec_n  < CFG.MIN_RECENT_TRADES: continue
        if rec_pf < CFG.MIN_PF * 0.82:    continue
        if rec_wr < CFG.MIN_WR * 0.88:    continue
        if rec_dd > CFG.MAX_DD * 1.20:    continue
        # WR ve PF decay kontrol
        wr_decay = (r["wr"] - rec_wr) / (r["wr"]+1e-9)
        pf_decay = (r["pf"] - rec_pf) / (r["pf"]+1e-9)
        if wr_decay > CFG.MAX_WR_DECAY: continue
        if pf_decay > CFG.MAX_PF_DECAY: continue
        # Sharpe proxy
        eq_curve = r.get("eq_curve", [])
        if len(eq_curve) > 10:
            eq_arr = np.array(eq_curve)
            rets   = np.diff(eq_arr) / (eq_arr[:-1]+1e-9)
            sharpe = (rets.mean()/(rets.std()+1e-9)) * np.sqrt(252*6.5*60)
            if sharpe < 0.35: continue
        # Calmar proxy
        total_ret = (r["final_equity"] - CFG.ACCOUNT_SIZE) / CFG.ACCOUNT_SIZE
        calmar = total_ret / (r["max_dd"]+1e-9)
        if calmar < 1.0: continue
        # Rejim tutarlilik — 4 ceyrek
        if len(eq_curve) > 40:
            eq_arr = np.array(eq_curve)
            q = len(eq_arr)//4
            e1 = (eq_arr[q]   - CFG.ACCOUNT_SIZE) / CFG.ACCOUNT_SIZE
            e2 = (eq_arr[2*q] - eq_arr[q])        / (eq_arr[q]+1e-9)
            e3 = (eq_arr[3*q] - eq_arr[2*q])      / (eq_arr[2*q]+1e-9)
            e4 = (eq_arr[-1]  - eq_arr[3*q])      / (eq_arr[3*q]+1e-9)
            if any(e < -0.07 for e in [e1,e2,e3,e4]): continue
            if e4 > max(e1+0.01, 0.03) * 20: continue
        # Sortino proxy
        if len(eq_curve) > 10:
            eq_arr = np.array(eq_curve)
            rets   = np.diff(eq_arr) / (eq_arr[:-1]+1e-9)
            neg_rets = rets[rets < 0]
            if len(neg_rets) > 5:
                sortino = (rets.mean()/(neg_rets.std()+1e-9)) * np.sqrt(252*6.5*60)
                if sortino < 0.5: continue
        # Stabilite + decay penalty + D3 Ulcer Index
        pf_stability = min(1.0, rec_pf / (r["pf"]+1e-9))
        wr_stability = min(1.0, rec_wr / (r["wr"]+1e-9))
        dd_stability = min(1.0, CFG.MAX_DD / (rec_dd+1e-9))
        decay_penalty = max(0.0, 1.0 - wr_decay*2 - pf_decay)
        ui = ulcer_index(eq_curve)
        ui_factor = 1.0 / (1.0 + ui * 5)
        score = (r["pf"] * r["wr"] * (1-r["max_dd"])
                 * pf_stability * wr_stability * dd_stability
                 * decay_penalty * ui_factor
                 * min(2.0, calmar/3))

        if best is None or score > best["score"]:
            best = {
                "name":       "BIDIR_"+"_".join(keys),
                "direction":  "bidir",
                "conditions": [],           # bidir için ayrı
                "long_conditions":  long_cols,
                "short_conditions": short_cols,
                "keys":       keys,
                "tp_mult":    tp_mult,
                "sl_mult":    sl_mult,
                "rr":         round(tp_mult/sl_mult, 2),
                "total_trades": r["total"],
                "win_rate":     round(r["wr"],4),
                "profit_factor":round(r["pf"],4),
                "max_dd":       round(r["max_dd"],4),
                "final_equity": round(r["final_equity"],2),
                "long_trades":  r["long_trades"],
                "short_trades": r["short_trades"],
                "long_wr":      round(r["long_wr"],4),
                "short_wr":     round(r["short_wr"],4),
                "rec_trades":   rec_n,
                "rec_wr":       round(rec_wr,4),
                "rec_pf":       round(rec_pf,4),
                "rec_dd":       round(rec_dd,4),
                "score":        round(score,6),
                "eq_curve":     r["eq_curve"],
            }
    if best:
        all_results.append(best)
    tested += 1
    if tested % 300 == 0:
        print(f"  {tested:>6}/{total_combos} | Bulunan: {len(all_results)}")

# Bidir stratejileri de tara
for combo in bidir_combos:
    evaluate_bidir(combo)

print(f"  Toplam: {tested:,} test | Uygun strateji: {len(all_results)}")

# ================================================================
# 5. MONTE CARLO
# ================================================================
print(f"\n[4/5] Monte Carlo analizi ({CFG.N_SIM:,} simulasyon)...")



def monte_carlo(eq_curve_pct, n_trades, tp_mult, sl_mult):
    """
    Log-return bazlı MC: trilyon dolar overflow önlenir.
    Bakiye cap: hesabın max 1000 katı ($1,000,000).
    """
    arr     = np.array(eq_curve_pct)
    log_arr = np.log1p(arr)
    # Trade sayısını normalize et: max 400 trade ile simüle et
    # Böylece 2000+ trade stratejiler adil karşılaştırılır
    sim_size  = min(len(arr), 400)
    final_ret = np.zeros(CFG.N_SIM)
    max_dds   = np.zeros(CFG.N_SIM)
    for s in range(CFG.N_SIM):
        shuf      = np.random.choice(log_arr, size=sim_size, replace=True)
        log_curve = np.cumsum(shuf)
        eq        = CFG.ACCOUNT_SIZE * np.exp(log_curve)
        eq        = np.minimum(eq, CFG.ACCOUNT_SIZE * 200)  # max 200x bakiye
        final_ret[s] = (eq[-1] - CFG.ACCOUNT_SIZE) / CFG.ACCOUNT_SIZE
        pk   = np.maximum.accumulate(eq)
        max_dds[s] = ((pk-eq)/pk).max()
    return {
        "prob_profit":    float((final_ret>0).mean()),
        "med_return":     float(np.median(final_ret)),
        "p5_return":      float(np.percentile(final_ret,5)),
        "p95_return":     float(np.percentile(final_ret,95)),
        "med_maxdd":      float(np.median(max_dds)),
        "p95_maxdd":      float(np.percentile(max_dds,95)),
        "med_final_bal":  float(CFG.ACCOUNT_SIZE*(1+np.median(final_ret))),
        "worst_final_bal":float(CFG.ACCOUNT_SIZE*(1+np.percentile(final_ret,5))),
        "best_final_bal": float(CFG.ACCOUNT_SIZE*(1+np.percentile(final_ret,95))),
    }

def permutation_test(trades_arr, tp_mult, sl_mult, n_perm=5000):
    """
    H0: Bu strateji rastgele %50 WR ile ayni performansi gosteriyor.
    Gercek PF'yi 5000 rastgele (%50 WR) simulasyonuyla karsilastir.
    p-degeri dusuk (<0.05) = edge gercek, sans degil.
    """
    n    = len(trades_arr)
    wins = (trades_arr==1).sum()
    real_pf = (wins*tp_mult)/((n-wins)*sl_mult+1e-9)
    rand_pfs = np.zeros(n_perm)
    for i in range(n_perm):
        rw = np.random.binomial(n, 0.5)
        rand_pfs[i] = (rw*tp_mult)/((n-rw)*sl_mult+1e-9)
    p_val = float((rand_pfs>=real_pf).mean())
    return p_val, real_pf

def get_pnl_arr(mask_arr, direction, tp_mult, sl_mult, start_idx):
    idx = np.where(mask_arr[start_idx:])[0] + start_idx
    trades_out=[]; pnl_out=[]
    pos_close_bar=-1
    for i in idx:
        if i+CFG.TIMEOUT_BARS>=N: continue
        if i <= pos_close_bar: continue
        if i+1>=N: continue
        a=atr[i]
        if a==0 or np.isnan(a): continue
        entry=open_p[i+1]
        if entry==0 or np.isnan(entry): continue
        tp=entry+tp_mult*a if direction=="long" else entry-tp_mult*a
        sl=entry-sl_mult*a if direction=="long" else entry+sl_mult*a
        outcome=None
        for j in range(i+2, min(i+CFG.TIMEOUT_BARS+1,N)):
            if direction=="long":
                if low[j]<=sl:  outcome=-1; pos_close_bar=j; break
                if high[j]>=tp: outcome=1;  pos_close_bar=j; break
            else:
                if high[j]>=sl: outcome=-1; pos_close_bar=j; break
                if low[j]<=tp:  outcome=1;  pos_close_bar=j; break
        if outcome is None:
            outcome=-1; pos_close_bar=i+CFG.TIMEOUT_BARS
        trades_out.append(outcome)
        # MC icin normalize pnl (% olarak, baslangic bakiyeye gore)
        pnl_out.append(tp_mult*CFG.RISK_PCT if outcome==1 else -sl_mult*CFG.RISK_PCT)
    return np.array(trades_out), np.array(pnl_out)

# Sadece top N'i MC'den gecir
all_results.sort(key=lambda x: x["score"], reverse=True)
top_candidates = all_results[:CFG.TOP_N * 3]

# ── ÇOKLU WALK-FORWARD (3 bağımsız pencere) ────────────────────
WF_WINDOWS_CFG = [
    (int(N*0.50), int(N*0.65)),
    (int(N*0.65), int(N*0.80)),
    (int(N*0.80), int(N*0.95)),
]
print(f"  Walk-forward doğrulaması ({len(top_candidates)} aday, 3 pencere)...")
for r in top_candidates:
    wf_pass = 0; wf_details = []
    if r["direction"] == "bidir":
        # Bidir: long ve short maskelerini ayrı ayrı oluştur
        long_mask  = np.ones(N, dtype=bool)
        short_mask = np.ones(N, dtype=bool)
        for col in r.get("long_conditions", []):
            if col in df.columns: long_mask &= df[col].values.astype(bool)
        for col in r.get("short_conditions", []):
            if col in df.columns: short_mask &= df[col].values.astype(bool)
        for (ws, we) in WF_WINDOWS_CFG:
            rw = run_backtest_bidir_range(long_mask, short_mask, r["tp_mult"], r["sl_mult"], ws, we)
            ok = rw and rw["total"]>=15 and rw["pf"]>=CFG.MIN_PF and rw["wr"]>=0.40
            if ok: wf_pass += 1
            wf_details.append({"total":rw["total"] if rw else 0,
                                "wr":round(rw["wr"],3) if rw else 0,
                                "pf":round(rw["pf"],2) if rw else 0,
                                "max_dd":round(rw["max_dd"],3) if rw else 1})
    else:
        mask = np.ones(N, dtype=bool)
        for col in r["conditions"]:
            mask &= df[col].values.astype(bool)
        for (ws, we) in WF_WINDOWS_CFG:
            rw = run_backtest_range(mask, r["direction"], r["tp_mult"], r["sl_mult"], ws, we)
            ok = rw and rw["total"]>=15 and rw["pf"]>=CFG.MIN_PF and rw["wr"]>=0.40
            if ok: wf_pass += 1
            wf_details.append({"total":rw["total"] if rw else 0,
                                "wr":round(rw["wr"],3) if rw else 0,
                                "pf":round(rw["pf"],2) if rw else 0,
                                "max_dd":round(rw["max_dd"],3) if rw else 1})
    r["wf_pass"] = wf_pass; r["wf_details"] = wf_details

top_results = [r for r in top_candidates if r.get("wf_pass",0)>=2]
top_results.sort(key=lambda x: x["score"], reverse=True)
top_results = top_results[:CFG.TOP_N]
print(f"  WF filtresi sonrası: {len(top_results)} strateji kaldı")
if not top_results:
    print("  ⚠ WF filtresi çok katı, tüm adaylar alındı")
    top_results = all_results[:CFG.TOP_N]
    for r in top_results: r["wf_pass"]=0; r["wf_details"]=[]

def get_pnl_arr_bidir(long_mask, short_mask, tp_mult, sl_mult, start_idx):
    combined = np.zeros(N, dtype=np.int8)
    combined[long_mask]  = 1
    combined[short_mask] = np.where(combined[short_mask]==0, -1, combined[short_mask])
    idx = np.where(combined[start_idx:] != 0)[0] + start_idx
    trades_out=[]; pnl_out=[]
    pos_close_bar=-1
    for i in idx:
        if i+CFG.TIMEOUT_BARS>=N: continue
        if i <= pos_close_bar: continue
        if i+1>=N: continue
        a=atr[i]
        if a==0 or np.isnan(a): continue
        entry=open_p[i+1]
        if entry==0 or np.isnan(entry): continue
        direction = "long" if combined[i]==1 else "short"
        tp=entry+tp_mult*a if direction=="long" else entry-tp_mult*a
        sl=entry-sl_mult*a if direction=="long" else entry+sl_mult*a
        outcome=None
        for j in range(i+2, min(i+CFG.TIMEOUT_BARS+1,N)):
            if direction=="long":
                if low[j]<=sl:  outcome=-1; pos_close_bar=j; break
                if high[j]>=tp: outcome=1;  pos_close_bar=j; break
            else:
                if high[j]>=sl: outcome=-1; pos_close_bar=j; break
                if low[j]<=tp:  outcome=1;  pos_close_bar=j; break
        if outcome is None:
            outcome=-1; pos_close_bar=i+CFG.TIMEOUT_BARS
        trades_out.append(outcome)
        pnl_out.append(tp_mult*CFG.RISK_PCT if outcome==1 else -sl_mult*CFG.RISK_PCT)
    return np.array(trades_out), np.array(pnl_out)

for r in top_results:
    if r["direction"] == "bidir":
        long_mask  = np.ones(N, dtype=bool)
        short_mask = np.ones(N, dtype=bool)
        for col in r.get("long_conditions", []):
            if col in df.columns: long_mask &= df[col].values.astype(bool)
        for col in r.get("short_conditions", []):
            if col in df.columns: short_mask &= df[col].values.astype(bool)
        trades_arr, pnl_arr = get_pnl_arr_bidir(long_mask, short_mask, r["tp_mult"], r["sl_mult"], TRAIN_END)
    else:
        mask = np.ones(N, dtype=bool)
        for col in r["conditions"]:
            mask &= df[col].values.astype(bool)
        trades_arr, pnl_arr = get_pnl_arr(mask, r["direction"], r["tp_mult"], r["sl_mult"], TRAIN_END)
    if len(pnl_arr) < 30:
        r["mc"] = None; r["p_value"] = 1.0; continue
    mc   = monte_carlo(pnl_arr, len(trades_arr), r["tp_mult"], r["sl_mult"])
    pval, _ = permutation_test(trades_arr, r["tp_mult"], r["sl_mult"])
    r["mc"]      = mc
    r["p_value"] = pval
    # A3: Bonferroni ve FDR esikleri
    n_total_tests = total_combos * len(CFG.TP_SL_GRID)
    bonferroni_p  = CFG.BONFERRONI_ALPHA / max(1, n_total_tests)
    r["bonferroni_p"] = round(bonferroni_p, 8)
    r["bonferroni_pass"] = pval < bonferroni_p
    # FDR (Benjamini-Hochberg) — sonradan tum listede hesaplanacak
    if pval < bonferroni_p:
        r["edge_label"] = "GUCLU EDGE BONF (%99)"
    elif pval < 0.01:
        r["edge_label"] = "GUCLU EDGE (%99)"
    elif pval < 0.05:
        r["edge_label"] = "ANLAMLI EDGE (%95)"
    elif pval < 0.10:
        r["edge_label"] = "ZAYIF EDGE (%90)"
    else:
        r["edge_label"] = "EDGE YOK"

# ================================================================
# 6. RAPOR
# ================================================================
print(f"\n[5/5] Rapor olusturuluyor...")

SEP = "=" * 66

print(f"\n\n{SEP}")
print(f"  SONUCLAR — En iyi {len(top_results)} strateji")
print(SEP)

if not top_results:
    print("\n  Hic strateji bulunamadi.")
    print("  Onerim: verinizin kalitesini kontrol edin veya filtreleri gevsetin.")
else:
    print(f"\n{'#':<3} {'Strateji':<38} {'TP':<4} {'SL':<4} {'N':>5} {'WR':>6} {'PF':>5} {'DD':>6} {'rPF':>5} {'p-val':>6}")
    print("-"*90)
    for i,r in enumerate(top_results,1):
        nm = r["name"][:37]
        pv = r.get("p_value",1.0)
        rpf= r.get("rec_pf",0)
        print(f"{i:<3} {nm:<38} {r['tp_mult']:<4} {r['sl_mult']:<4} "
              f"{r['total_trades']:>5} {r['win_rate']:>6.1%} {r['profit_factor']:>5.2f} "
              f"{r['max_dd']:>6.1%} {rpf:>5.2f} {pv:>6.4f}")

    print(f"\n  Sutun aciklamalari:")
    print(f"  N=trade sayisi | WR=win rate | PF=profit factor | DD=max drawdown")
    print(f"  rPF=yakin donem PF (%15) | p-val=permutasyon testi (dusuk=iyi)")

    # Detayli rapor - her strateji
    for i, r in enumerate(top_results, 1):
        mc  = r.get("mc")
        pv  = r.get("p_value", 1.0)
        el  = r.get("edge_label", "?")
        print(f"\n{SEP}")
        print(f"  #{i}  {r['name']}")
        print(SEP)
        print(f"  Yon:            {r['direction'].upper()}{' (hem long hem short)' if r['direction']=='bidir' else ''}")
        print(f"  TP/SL:          {r['tp_mult']}x / {r['sl_mult']}x ATR(14)  →  RR 1:{r['rr']}")
        print(f"  Giris kosullari:")
        if r["direction"] == "bidir":
            print(f"    LONG girisi:")
            for col in r.get("long_conditions", []):
                print(f"      + {col}")
            print(f"    SHORT girisi:")
            for col in r.get("short_conditions", []):
                print(f"      + {col}")
        else:
            for col in r["conditions"]:
                print(f"    + {col}")

        print(f"\n  WALK-FORWARD TEST (ana test + 3 bağımsız pencere):")
        print(f"    Trade sayisi:   {r['total_trades']}  ({'ayda ~'+str(int(r['total_trades']/6))+' trade' if r['total_trades']>0 else ''})")
        if r["direction"] == "bidir":
            lt = r.get("long_trades",0); st = r.get("short_trades",0)
            lwr= r.get("long_wr",0);    swr= r.get("short_wr",0)
            print(f"    ↑ LONG:  {lt} trade  WR: {lwr:.1%}")
            print(f"    ↓ SHORT: {st} trade  WR: {swr:.1%}")
        print(f"    Win Rate:       {r['win_rate']:.1%}")
        print(f"    Profit Factor:  {r['profit_factor']:.2f}")
        print(f"    Max Drawdown:   {r['max_dd']:.1%}")
        print(f"    Son bakiye:     ${r['final_equity']:,.2f}  (baslangic ${CFG.ACCOUNT_SIZE:,.0f})")
        wfp = r.get("wf_pass","?"); wfd = r.get("wf_details",[])
        print(f"    WF Pencereler:  {wfp}/3 geçti  {'✓ TUTARLI' if wfp>=2 else '⚠ DİKKAT'}")
        for wi,wd in enumerate(wfd,1):
            ok = "✓" if wd["pf"]>=CFG.MIN_PF and wd["wr"]>=0.40 else "✗"
            print(f"      P{wi}: {ok} WR={wd['wr']:.1%} PF={wd['pf']:.2f} DD={wd['max_dd']:.1%} N={wd['total']}")

        print(f"\n  YAKIN DONEM (%5 — test'ten bağımsız son dönem):")
        print(f"    Trade sayisi:   {r['rec_trades']}")
        print(f"    Win Rate:       {r['rec_wr']:.1%}")
        print(f"    Profit Factor:  {r['rec_pf']:.2f}")
        print(f"    Max Drawdown:   {r['rec_dd']:.1%}")

        print(f"\n  MONTE CARLO ({CFG.N_SIM:,} simulasyon, %{CFG.RISK_PER_TRADE*100:.0f} risk/trade):")
        if mc:
            print(f"    Karli olma ihtimali:  {mc['prob_profit']:.1%}")
            print(f"    Medyan getiri:        {mc['med_return']:+.1%}  → ${mc['med_final_bal']:,.0f}")
            print(f"    %5 kotu senaryo:      {mc['p5_return']:+.1%}  → ${mc['worst_final_bal']:,.0f}")
            print(f"    %95 iyi senaryo:      {mc['p95_return']:+.1%}  → ${mc['best_final_bal']:,.0f}")
            print(f"    Medyan Max DD:        {mc['med_maxdd']:.1%}")
            print(f"    %95 kotu Max DD:      {mc['p95_maxdd']:.1%}  ← buna hazir ol")
        else:
            print(f"    Yeterli trade yok.")

        print(f"\n  EDGE DEGERLENDIRMESI:")
        print(f"    P-degeri:  {pv:.4f}")
        print(f"    Karar:     {el}")
        if pv < 0.05 and r["rec_pf"] >= CFG.MIN_PF and r["max_dd"] < CFG.MAX_DD:
            print(f"    → BU STRATEJİYİ KULLANMAYA UYGUN")
        elif pv < 0.10:
            print(f"    → DIKKATLI KULLAN — kucuk lotla test et")
        else:
            print(f"    → KULLANMA — edge istatistiksel olarak kanitlanamadi")

    # En iyi strateji ozet
    best = top_results[0]
    print(f"\n\n{SEP}")
    print(f"  ★  EN İYİ STRATEJİ OZETI")
    print(SEP)
    print(f"  {best['name']}")
    print(f"  Yon:   {best['direction'].upper()}")
    print(f"  TP:    {best['tp_mult']}x ATR(14)")
    print(f"  SL:    {best['sl_mult']}x ATR(14)")
    print(f"  RR:    1:{best['rr']}")
    print(f"\n  GIRIS:")
    for col in best["conditions"]:
        print(f"    ✓ {col}")
    print(f"\n  CIKIS:")
    print(f"    TP = giris {'+ ' if best['direction']=='long' else '- '}{best['tp_mult']} x ATR(14,close)")
    print(f"    SL = giris {' - ' if best['direction']=='long' else '+ '}{best['sl_mult']} x ATR(14,close)")
    print(f"    Timeout: {CFG.TIMEOUT_BARS} mum")
    print(f"\n  BEKLENEN PERFORMANS:")
    print(f"    Win Rate:       {best['win_rate']:.1%}")
    print(f"    Profit Factor:  {best['profit_factor']:.2f}")
    print(f"    Max Drawdown:   {best['max_dd']:.1%}")
    mc = best.get("mc")
    if mc:
        print(f"    Karli olma:     {mc['prob_profit']:.1%}")
        print(f"    Kotu senaryo:   {mc['p5_return']:+.1%}")
    print(f"\n  EDGE: {best.get('edge_label','?')}  (p={best.get('p_value',1):.4f})")
    # LOT / KALDIRAÇ OZETI
    print(f"\n  GERCEK HESAP SIMULASYONU (${CFG.ACCOUNT_SIZE:.0f}, 1:{CFG.LEVERAGE}):")
    ortalama_atr = float(df["atr14_1m"].iloc[TRAIN_END:].mean())
    ornek_lot = calc_lot(CFG.ACCOUNT_SIZE, ortalama_atr, best["sl_mult"])
    ornek_margin = calc_margin(ornek_lot, float(df["close_1m"].iloc[-1]), )
    ornek_sl_usd = ornek_lot * CFG.CONTRACT_SIZE * ortalama_atr * best["sl_mult"]
    ornek_tp_usd = ornek_lot * CFG.CONTRACT_SIZE * ortalama_atr * best["tp_mult"]
    print(f"    Ortalama ATR:   {ortalama_atr:.2f}$")
    print(f"    Tipik lot:      {ornek_lot:.2f} lot")
    print(f"    Tipik margin:   ${ornek_margin:.2f}")
    print(f"    Tipik SL:       ${ornek_sl_usd:.2f}")
    print(f"    Tipik TP:       ${ornek_tp_usd:.2f}")
    print(f"    Son bakiye (test): ${best['final_equity']:,.2f}")
    print(f"    Getiri:         %{(best['final_equity']/CFG.ACCOUNT_SIZE-1)*100:.1f}")


# ================================================================
# 7. AYLIK / GÜNLÜK DETAYLI RAPOR
# ================================================================
print(f"\n{'='*66}")
print(f"  AYLIK / GÜNLÜK DETAYLI ANALİZ")
print(f"{'='*66}")

# Sadece edge'i kanıtlanmış stratejileri analiz et
valid_strats = [r for r in top_results if r.get("p_value",1) < 0.10]
if not valid_strats:
    valid_strats = top_results[:3]

# Ortalama ATR (test bölümü)
avg_atr_test = float(df["atr14_1m"].iloc[TRAIN_END:].mean())
# Sabit lot: başlangıç hesabına göre
fixed_lot = calc_lot(CFG.ACCOUNT_SIZE, avg_atr_test, 1.0)
fixed_lot = max(CFG.MIN_LOT, min(1.0, fixed_lot))  # max 1 lot sabit modda

test_start_dt = df["datetime"].iloc[TRAIN_END]
test_end_dt   = df["datetime"].iloc[-1]
n_months = max(1, (test_end_dt.year - test_start_dt.year)*12 + 
               (test_end_dt.month - test_start_dt.month) + 1)

print(f"\n  Test: {test_start_dt.strftime('%Y-%m-%d')} → {test_end_dt.strftime('%Y-%m-%d')} (~{n_months} ay)")
print(f"  Hesap: ${CFG.ACCOUNT_SIZE:,.0f} | Kaldıraç: 1:{CFG.LEVERAGE} | Risk: %{CFG.RISK_PCT*100:.0f}/trade")
print(f"  Sabit lot (test): {fixed_lot:.2f} | Aylık rebalance: aybaşı bakiyesine göre")

def monthly_backtest(mask_arr, direction, tp_mult, sl_mult, lot_mode="fixed"):
    """Sabit veya aylık rebalance lotla backtest, aylık/günlük PnL üretir."""
    idx = np.where(mask_arr[TRAIN_END:])[0] + TRAIN_END
    eq   = CFG.ACCOUNT_SIZE
    peak = eq
    max_dd = 0.0
    pos_close_bar = -1
    current_lot   = fixed_lot
    current_month = None
    trades = []
    for i in idx:
        if i+CFG.TIMEOUT_BARS>=N: continue
        if i <= pos_close_bar: continue
        if i+1>=N: continue
        a = atr[i]
        if a==0 or np.isnan(a): continue
        bar_dt    = df["datetime"].iloc[i]
        bar_month = (bar_dt.year, bar_dt.month)
        if lot_mode=="monthly" and bar_month != current_month:
            current_month = bar_month
            current_lot   = calc_lot(eq, avg_atr_test, sl_mult)
            current_lot   = max(CFG.MIN_LOT, min(CFG.MAX_LOT, current_lot))
        lot   = current_lot
        entry = open_p[i+1]
        if entry==0 or np.isnan(entry): continue
        if direction=="long": tp=entry+tp_mult*a; sl=entry-sl_mult*a
        else:                 tp=entry-tp_mult*a; sl=entry+sl_mult*a
        outcome=None
        for j in range(i+2, min(i+CFG.TIMEOUT_BARS+1,N)):
            if direction=="long":
                if low[j]<=sl:  outcome=-1; pos_close_bar=j; break
                if high[j]>=tp: outcome=1;  pos_close_bar=j; break
            else:
                if high[j]>=sl: outcome=-1; pos_close_bar=j; break
                if low[j]<=tp:  outcome=1;  pos_close_bar=j; break
        if outcome is None: outcome=-1; pos_close_bar=i+CFG.TIMEOUT_BARS
        pnl = (lot*CFG.CONTRACT_SIZE*tp_mult*a) if outcome==1 else -(lot*CFG.CONTRACT_SIZE*sl_mult*a)
        eq += pnl
        if eq<=0: eq=0; break
        if eq>peak: peak=eq
        dd=(peak-eq)/peak
        if dd>max_dd: max_dd=dd
        trades.append({"dt":bar_dt,"pnl":pnl,"eq":eq,"outcome":outcome,"lot":lot})
    if not trades: return None, None, None, max_dd
    df_t = pd.DataFrame(trades)
    df_t["month"] = df_t["dt"].dt.to_period("M")
    df_t["date"]  = df_t["dt"].dt.date
    monthly = df_t.groupby("month").agg(
        trades=("pnl","count"), pnl=("pnl","sum"),
        wins=("outcome",lambda x:(x==1).sum()), lot=("lot","mean")
    ).reset_index()
    monthly["wr"]         = monthly["wins"]/monthly["trades"]
    monthly["pct_start"]  = monthly["pnl"]/CFG.ACCOUNT_SIZE*100
    eq_t=CFG.ACCOUNT_SIZE; cmp=[]
    for _,row in monthly.iterrows():
        cmp.append(row["pnl"]/eq_t*100); eq_t+=row["pnl"]
    monthly["pct_cmp"] = cmp
    daily = df_t.groupby("date")["pnl"].sum().reset_index()
    daily["pct"] = daily["pnl"]/CFG.ACCOUNT_SIZE*100
    return df_t, monthly, daily, max_dd

def monthly_backtest_bidir(long_mask, short_mask, tp_mult, sl_mult, lot_mode="fixed"):
    """Bidir strateji için aylık backtest (hem long hem short)."""
    combined = np.zeros(N, dtype=np.int8)
    combined[long_mask]  = 1
    combined[short_mask] = np.where(combined[short_mask]==0, -1, combined[short_mask])
    idx = np.where(combined[TRAIN_END:] != 0)[0] + TRAIN_END
    eq=CFG.ACCOUNT_SIZE; peak=eq; max_dd=0.0; pos_close_bar=-1
    current_lot=fixed_lot; current_month=None; trades=[]
    for i in idx:
        if i+CFG.TIMEOUT_BARS>=N: continue
        if i<=pos_close_bar: continue
        if i+1>=N: continue
        a=atr[i]
        if a==0 or np.isnan(a): continue
        bar_dt=df["datetime"].iloc[i]; bar_month=(bar_dt.year,bar_dt.month)
        if lot_mode=="monthly" and bar_month!=current_month:
            current_month=bar_month
            current_lot=calc_lot(eq,avg_atr_test,sl_mult)
            current_lot=max(CFG.MIN_LOT,min(CFG.MAX_LOT,current_lot))
        lot=current_lot; entry=open_p[i+1]
        if entry==0 or np.isnan(entry): continue
        direction="long" if combined[i]==1 else "short"
        if direction=="long": tp=entry+tp_mult*a; sl=entry-sl_mult*a
        else:                 tp=entry-tp_mult*a; sl=entry+sl_mult*a
        outcome=None
        for j in range(i+2,min(i+CFG.TIMEOUT_BARS+1,N)):
            if direction=="long":
                if low[j]<=sl:  outcome=-1; pos_close_bar=j; break
                if high[j]>=tp: outcome=1;  pos_close_bar=j; break
            else:
                if high[j]>=sl: outcome=-1; pos_close_bar=j; break
                if low[j]<=tp:  outcome=1;  pos_close_bar=j; break
        if outcome is None: outcome=-1; pos_close_bar=i+CFG.TIMEOUT_BARS
        pnl=(lot*CFG.CONTRACT_SIZE*tp_mult*a) if outcome==1 else -(lot*CFG.CONTRACT_SIZE*sl_mult*a)
        eq+=pnl
        if eq<=0: eq=0; break
        if eq>peak: peak=eq
        dd=(peak-eq)/peak
        if dd>max_dd: max_dd=dd
        trades.append({"dt":bar_dt,"pnl":pnl,"eq":eq,"outcome":outcome,"lot":lot,"dir":direction})
    if not trades: return None,None,None,max_dd
    df_t=pd.DataFrame(trades)
    df_t["month"]=df_t["dt"].dt.to_period("M")
    df_t["date"]=df_t["dt"].dt.date
    monthly=df_t.groupby("month").agg(
        trades=("pnl","count"),pnl=("pnl","sum"),
        wins=("outcome",lambda x:(x==1).sum()),lot=("lot","mean")
    ).reset_index()
    monthly["wr"]=monthly["wins"]/monthly["trades"]
    monthly["pct_start"]=monthly["pnl"]/CFG.ACCOUNT_SIZE*100
    eq_t=CFG.ACCOUNT_SIZE; cmp=[]
    for _,row in monthly.iterrows():
        cmp.append(row["pnl"]/eq_t*100); eq_t+=row["pnl"]
    monthly["pct_cmp"]=cmp
    daily=df_t.groupby("date")["pnl"].sum().reset_index()
    daily["pct"]=daily["pnl"]/CFG.ACCOUNT_SIZE*100
    return df_t,monthly,daily,max_dd

for r in valid_strats[:5]:
    if r["direction"] == "bidir":
        long_mask  = np.ones(N, dtype=bool)
        short_mask = np.ones(N, dtype=bool)
        for col in r.get("long_conditions",  []): long_mask  &= df[col].values.astype(bool)
        for col in r.get("short_conditions", []): short_mask &= df[col].values.astype(bool)
        for mode_label, mode in [("SABİT LOT","fixed"),("AYLIK REBALANCE","monthly")]:
            df_t, monthly, daily, max_dd = monthly_backtest_bidir(
                long_mask, short_mask, r["tp_mult"], r["sl_mult"], mode)
            if monthly is None: continue
            final_eq=df_t["eq"].iloc[-1]; wins_t=(df_t["outcome"]==1).sum(); total_t=len(df_t)
            long_t=(df_t["dir"]=="long").sum(); short_t=(df_t["dir"]=="short").sum()
            print(f"\n{'─'*66}")
            print(f"  {r['name']} [{mode_label}]  ↑{long_t}L ↓{short_t}S")
            print(f"{'─'*66}")
            print(f"  Trade: {total_t} | WR: {wins_t/total_t:.1%} | DD: {max_dd:.1%} | Son bakiye: ${final_eq:,.2f}")
            print(f"\n  {'Ay':<10} {'Trade':>5} {'Lot':>5} {'WR':>6} {'PnL$':>9} {'%Başl':>7} {'%Dönem':>8}")
            print(f"  {'─'*58}")
            for _,row in monthly.iterrows():
                flag=" ✓%30+" if row["pct_start"]>=30 else (" ✗" if row["pct_start"]<0 else "")
                print(f"  {str(row['month']):<10} {row['trades']:>5} {row['lot']:>5.2f} {row['wr']:>6.1%} "
                      f"{row['pnl']:>9.2f} {row['pct_start']:>7.1f}% {row['pct_cmp']:>7.1f}%{flag}")
            avg_m=monthly["pct_start"].mean(); m30p=(monthly["pct_start"]>=30).sum()
            d1p=(daily["pct"]>=1.0).sum(); best_d=daily["pct"].max(); worst_d=daily["pct"].min()
            print(f"\n  Ort. aylık: %{avg_m:.1f} | %30+ ay: {m30p}/{len(monthly)}")
            print(f"  Ort. günlük: %{daily['pct'].mean():.2f} | En iyi: %{best_d:.2f} | En kötü: %{worst_d:.2f}")
    else:
        mask = np.ones(N, dtype=bool)
        for col in r["conditions"]:
            mask &= df[col].values.astype(bool)
        for mode_label, mode in [("SABİT LOT","fixed"),("AYLIK REBALANCE","monthly")]:
            df_t, monthly, daily, max_dd = monthly_backtest(
                mask, r["direction"], r["tp_mult"], r["sl_mult"], mode)
            if monthly is None: continue

            final_eq = df_t["eq"].iloc[-1]
            wins_t   = (df_t["outcome"]==1).sum()
            total_t  = len(df_t)
            
            print(f"\n{'─'*66}")
            print(f"  {r['name']} [{mode_label}]")
            print(f"{'─'*66}")
            print(f"  Trade: {total_t} | WR: {wins_t/total_t:.1%} | DD: {max_dd:.1%} | "
                  f"Son bakiye: ${final_eq:,.2f} (+%{(final_eq-CFG.ACCOUNT_SIZE)/CFG.ACCOUNT_SIZE*100:.0f})")
            print(f"\n  {'Ay':<10} {'Trade':>5} {'Lot':>5} {'WR':>6} {'PnL$':>9} "
                  f"{'%Başl':>7} {'%Dönem':>8}")
            print(f"  {'─'*58}")
            
            for _,row in monthly.iterrows():
                flag = " ✓%30+" if row["pct_start"]>=30 else (
                       " ✗" if row["pct_start"]<0 else "")
                print(f"  {str(row['month']):<10} {row['trades']:>5} {row['lot']:>5.2f} "
                      f"{row['wr']:>6.1%} {row['pnl']:>9.2f} "
                      f"{row['pct_start']:>7.1f}% {row['pct_cmp']:>7.1f}%{flag}")
            
            avg_m   = monthly["pct_start"].mean()
            avg_mc  = monthly["pct_cmp"].mean()
            pos_m   = (monthly["pct_start"]>0).sum()
            m30p    = (monthly["pct_start"]>=30).sum()
            avg_d   = daily["pct"].mean()
            d1p     = (daily["pct"]>=1.0).sum()
            best_d  = daily["pct"].max()
            worst_d = daily["pct"].min()
            
            print(f"\n  Ort. aylık (başlangıç): %{avg_m:.1f} | Compound: %{avg_mc:.1f}")
            print(f"  Pozitif ay: {pos_m}/{len(monthly)} | %30+ ay: {m30p}/{len(monthly)}")
            print(f"  Ort. günlük: %{avg_d:.2f} | En iyi gün: %{best_d:.2f} | En kötü: %{worst_d:.2f}")
            print(f"  Günlük %1+ gün sayısı: {d1p}/{len(daily)} (%{d1p/len(daily)*100:.0f})")
        
        # Hedef değerlendirmesi
        print(f"\n  HEDEF DEĞERLENDİRMESİ:")
        print(f"    Aylık %30 hedefi: {'✓ KARŞILANIYOR (%'+str(int(m30p/len(monthly)*100))+'+ ay)' if m30p/len(monthly)>=0.6 else '✗ KARŞILANMIYOR (sadece '+str(m30p)+'/'+str(len(monthly))+' ay)'}")
        print(f"    Günlük %1 hedefi: {'✓ KARŞILANIYOR' if d1p/len(daily)>=0.4 else '~ KISMEN'} "
              f"({d1p}/{len(daily)} gün, her {len(daily)//max(d1p,1):.0f} günde bir)")

# Portfolio analizi
print(f"\n{'='*66}")
print(f"  PORTFOLIO ANALİZİ — En iyi Long + Short (veya Bidir)")
print(f"{'='*66}")
long_strats  = [r for r in valid_strats if r["direction"]=="long"]
short_strats = [r for r in valid_strats if r["direction"]=="short"]
bidir_strats = [r for r in valid_strats if r["direction"]=="bidir"]

# Bidir stratejisi varsa önce onu göster
if bidir_strats:
    best_bidir = bidir_strats[0]
    print(f"\n  En iyi BİDİR strateji: {best_bidir['name']}")
    print(f"  ↑ Long: {best_bidir.get('long_trades',0)} trade WR:{best_bidir.get('long_wr',0):.1%}  "
          f"↓ Short: {best_bidir.get('short_trades',0)} trade WR:{best_bidir.get('short_wr',0):.1%}")

if long_strats and short_strats:
    best_long  = long_strats[0]
    best_short = short_strats[0]
    mask_l = np.ones(N,dtype=bool)
    for col in best_long["conditions"]: mask_l &= df[col].values.astype(bool)
    mask_s = np.ones(N,dtype=bool)
    for col in best_short["conditions"]: mask_s &= df[col].values.astype(bool)
    
    for mode_label, mode in [("SABİT LOT","fixed"),("AYLIK REBALANCE","monthly")]:
        _, ml, dl, _ = monthly_backtest(mask_l, "long",  best_long["tp_mult"],  best_long["sl_mult"],  mode)
        _, ms, ds, _ = monthly_backtest(mask_s, "short", best_short["tp_mult"], best_short["sl_mult"], mode)
        if ml is None or ms is None: continue
        
        # Aylık birleştir
        mp = pd.concat([ml,ms]).groupby("month").agg(
            trades=("trades","sum"), pnl=("pnl","sum"), wins=("wins","sum")
        ).reset_index()
        mp["wr"]        = mp["wins"]/mp["trades"]
        mp["pct_start"] = mp["pnl"]/CFG.ACCOUNT_SIZE*100
        eq_t=CFG.ACCOUNT_SIZE; cmp=[]
        for _,row in mp.iterrows():
            cmp.append(row["pnl"]/eq_t*100); eq_t+=row["pnl"]
        mp["pct_cmp"] = cmp
        dp = pd.concat([dl,ds]).groupby("date")["pct"].sum().reset_index()
        
        print(f"\n  Portfolio [{mode_label}]: {best_long['name']} + {best_short['name']}")
        print(f"  {'Ay':<10} {'Trade':>5} {'WR':>6} {'PnL$':>9} {'%Başl':>7} {'%Dönem':>8}")
        print(f"  {'─'*52}")
        for _,row in mp.iterrows():
            flag = " ✓%30+" if row["pct_start"]>=30 else (" ✗" if row["pct_start"]<0 else "")
            print(f"  {str(row['month']):<10} {row['trades']:>5} {row['wr']:>6.1%} "
                  f"{row['pnl']:>9.2f} {row['pct_start']:>7.1f}% {row['pct_cmp']:>7.1f}%{flag}")
        
        avg_m  = mp["pct_start"].mean()
        m30p   = (mp["pct_start"]>=30).sum()
        d1p    = (dp["pct"]>=1.0).sum()
        print(f"\n  Ort. aylık: %{avg_m:.1f} | %30+ ay: {m30p}/{len(mp)} | Günlük %1+: {d1p}/{len(dp)} gün")

print(f"\n{'='*66}\n")

# ================================================================
# 8. OOS DOĞRULAMA — Tamamen Bağımsız 2 Aylık Gerçek Veri
# ================================================================
if oos_available and df_oos is not None and len(df_oos) > 100:
    print(f"\n{'='*66}")
    print(f"  OOS DOĞRULAMA — Sistem Hiç Görmedi ({df_oos['datetime'].min().date()} → {df_oos['datetime'].max().date()})")
    print(f"{'='*66}")
    print(f"  Bu veri optimizasyona katılmadı. Buradaki sonuçlar gerçek forward-test.")

    N_oos   = len(df_oos)
    close_o = df_oos["close_1m"].values
    high_o  = df_oos["high_1m"].values
    low_o   = df_oos["low_1m"].values
    open_o  = df_oos["open_1m"].values
    atr_o   = df_oos["atr14_1m"].values

    def run_oos(conditions, direction, tp_mult, sl_mult):
        """OOS veri üzerinde tek yönlü backtest."""
        mask_o = np.ones(N_oos, dtype=bool)
        for col in conditions:
            if col in df_oos.columns:
                mask_o &= df_oos[col].values.astype(bool)
            else:
                return None
        idx = np.where(mask_o)[0]
        wins=0; losses=0; gp=0.0; gl=0.0
        equity=CFG.ACCOUNT_SIZE; peak=equity; max_dd=0.0; pos_close_bar=-1
        trades_list = []
        for i in idx:
            if i+CFG.TIMEOUT_BARS>=N_oos: continue
            if i<=pos_close_bar: continue
            if i+1>=N_oos: continue
            a=atr_o[i]
            if a==0 or np.isnan(a): continue
            entry=open_o[i+1]
            if entry==0 or np.isnan(entry): continue
            lot=calc_lot(CFG.ACCOUNT_SIZE, a, sl_mult)
            if direction=="long": tp=entry+tp_mult*a; sl=entry-sl_mult*a
            else:                 tp=entry-tp_mult*a; sl=entry+sl_mult*a
            outcome=None
            for j in range(i+2, min(i+CFG.TIMEOUT_BARS+1,N_oos)):
                if direction=="long":
                    if low_o[j]<=sl:  outcome=-1; pos_close_bar=j; break
                    if high_o[j]>=tp: outcome=1;  pos_close_bar=j; break
                else:
                    if high_o[j]>=sl: outcome=-1; pos_close_bar=j; break
                    if low_o[j]<=tp:  outcome=1;  pos_close_bar=j; break
            if outcome is None: outcome=-1; pos_close_bar=i+CFG.TIMEOUT_BARS
            spread_c=CFG.SPREAD_PTS*lot*CFG.CONTRACT_SIZE/100+CFG.COMMISSION*lot
            if outcome==1: wins+=1; gp+=tp_mult*a; pnl=lot*CFG.CONTRACT_SIZE*tp_mult*a-spread_c
            else:          losses+=1; gl+=sl_mult*a; pnl=-lot*CFG.CONTRACT_SIZE*sl_mult*a-spread_c
            equity+=pnl
            if equity<=0: equity=0; break
            if equity>peak: peak=equity
            dd=(peak-equity)/peak
            if dd>max_dd: max_dd=dd
            trades_list.append({"dt":df_oos["datetime"].iloc[i],"pnl":pnl,"eq":equity,"outcome":outcome})
        total=wins+losses
        if total<5: return None
        return {"total":total,"wr":wins/total,"pf":gp/(gl+1e-9),
                "max_dd":max_dd,"final_eq":equity,"trades":trades_list}

    def run_oos_bidir(long_conditions, short_conditions, tp_mult, sl_mult):
        """OOS veri üzerinde çift yönlü backtest."""
        long_mask_o  = np.ones(N_oos, dtype=bool)
        short_mask_o = np.ones(N_oos, dtype=bool)
        for col in long_conditions:
            if col in df_oos.columns: long_mask_o  &= df_oos[col].values.astype(bool)
            else: return None
        for col in short_conditions:
            if col in df_oos.columns: short_mask_o &= df_oos[col].values.astype(bool)
            else: return None
        combined_o = np.zeros(N_oos, dtype=np.int8)
        combined_o[long_mask_o]  = 1
        combined_o[short_mask_o] = np.where(combined_o[short_mask_o]==0, -1, combined_o[short_mask_o])
        idx = np.where(combined_o != 0)[0]
        wins=0; losses=0; gp=0.0; gl=0.0
        equity=CFG.ACCOUNT_SIZE; peak=equity; max_dd=0.0; pos_close_bar=-1
        trades_list=[]
        for i in idx:
            if i+CFG.TIMEOUT_BARS>=N_oos: continue
            if i<=pos_close_bar: continue
            if i+1>=N_oos: continue
            a=atr_o[i]
            if a==0 or np.isnan(a): continue
            entry=open_o[i+1]
            if entry==0 or np.isnan(entry): continue
            direction="long" if combined_o[i]==1 else "short"
            lot=calc_lot(CFG.ACCOUNT_SIZE, a, sl_mult)
            if direction=="long": tp=entry+tp_mult*a; sl=entry-sl_mult*a
            else:                 tp=entry-tp_mult*a; sl=entry+sl_mult*a
            outcome=None
            for j in range(i+2, min(i+CFG.TIMEOUT_BARS+1,N_oos)):
                if direction=="long":
                    if low_o[j]<=sl:  outcome=-1; pos_close_bar=j; break
                    if high_o[j]>=tp: outcome=1;  pos_close_bar=j; break
                else:
                    if high_o[j]>=sl: outcome=-1; pos_close_bar=j; break
                    if low_o[j]<=tp:  outcome=1;  pos_close_bar=j; break
            if outcome is None: outcome=-1; pos_close_bar=i+CFG.TIMEOUT_BARS
            spread_c=CFG.SPREAD_PTS*lot*CFG.CONTRACT_SIZE/100+CFG.COMMISSION*lot
            if outcome==1: wins+=1; gp+=tp_mult*a; pnl=lot*CFG.CONTRACT_SIZE*tp_mult*a-spread_c
            else:          losses+=1; gl+=sl_mult*a; pnl=-lot*CFG.CONTRACT_SIZE*sl_mult*a-spread_c
            equity+=pnl
            if equity<=0: equity=0; break
            if equity>peak: peak=equity
            dd=(peak-equity)/peak
            if dd>max_dd: max_dd=dd
            trades_list.append({"dt":df_oos["datetime"].iloc[i],"pnl":pnl,"eq":equity,"outcome":outcome,"dir":direction})
        total=wins+losses
        if total<5: return None
        return {"total":total,"wr":wins/total,"pf":gp/(gl+1e-9),
                "max_dd":max_dd,"final_eq":equity,"trades":trades_list}

    oos_summary = []
    for r in top_results:
        pv = r.get("p_value",1.0)
        if pv >= 0.05: continue  # sadece edge kanıtlananları test et
        # Bidir veya tek yön
        if r["direction"] == "bidir":
            res = run_oos_bidir(
                r.get("long_conditions", []),
                r.get("short_conditions", []),
                r["tp_mult"], r["sl_mult"])
        else:
            res = run_oos(r["conditions"], r["direction"], r["tp_mult"], r["sl_mult"])
        if res is None:
            print(f"\n  {r['name'][:55]}: OOS sinyal yok")
            continue
        pnl_pct = (res["final_eq"]-CFG.ACCOUNT_SIZE)/CFG.ACCOUNT_SIZE*100
        verdict = "✓ GEÇTI" if res["wr"]>=0.45 and res["pf"]>=1.0 and res["max_dd"]<=0.25 else "✗ BAŞARISIZ"
        # B4: Wilson CI
        wins_oos = int(round(res["wr"]*res["total"]))
        ci_lo, ci_hi = wilson_ci(wins_oos, res["total"])
        n_warn = " ⚠(N<265)" if res["total"]<265 else ""
        print(f"\n  {r['name'][:55]}")
        print(f"  {'─'*55}")
        print(f"  Trade: {res['total']:>4}{n_warn} | WR: {res['wr']:.1%} [CI:{ci_lo:.1%}-{ci_hi:.1%}] | PF: {res['pf']:.2f} | DD: {res['max_dd']:.1%}")
        print(f"  Getiri: %{pnl_pct:.1f} (${res['final_eq']:,.0f}) | {verdict}")
        # Aylık dağılım (varsa)
        if res["trades"]:
            df_t = pd.DataFrame(res["trades"])
            df_t["month"] = df_t["dt"].dt.to_period("M")
            mp = df_t.groupby("month").agg(n=("pnl","count"),pnl=("pnl","sum"),w=("outcome",lambda x:(x==1).sum())).reset_index()
            for _,row in mp.iterrows():
                pct=row["pnl"]/CFG.ACCOUNT_SIZE*100
                print(f"    {str(row['month'])}: {row['n']:>3} trade WR:{row['w']/row['n']:.0%} PnL:${row['pnl']:>7.0f} (%{pct:.1f})")
        # Backtest vs OOS karşılaştır
        bt_wr = r["win_rate"]; bt_pf = r["profit_factor"]
        oos_wr = res["wr"]; oos_pf = res["pf"]
        decay_wr = (bt_wr - oos_wr)/bt_wr if bt_wr>0 else 0
        decay_pf = (bt_pf - oos_pf)/bt_pf if bt_pf>0 else 0
        print(f"  Backtest WR:{bt_wr:.1%} → OOS WR:{oos_wr:.1%} (kayıp:%{decay_wr*100:.0f})")
        print(f"  Backtest PF:{bt_pf:.2f}  → OOS PF:{oos_pf:.2f}  (kayıp:%{decay_pf*100:.0f})")
        r["oos"] = {"total":res["total"],"wr":round(oos_wr,4),"pf":round(oos_pf,4),
                    "max_dd":round(res["max_dd"],4),"pnl_pct":round(pnl_pct,2),
                    "verdict":verdict,"ci_lo":round(ci_lo,4),"ci_hi":round(ci_hi,4)}
        oos_summary.append((r["name"][:50], res["total"], oos_wr, oos_pf, res["max_dd"], pnl_pct, verdict))

    if oos_summary:
        print(f"\n{'='*66}")
        print(f"  OOS ÖZET TABLOSU")
        print(f"{'='*66}")
        print(f"  {'Strateji':<50} {'N':>4} {'WR':>6} {'PF':>5} {'DD':>6} {'Pct%':>7} {'Karar'}")
        print(f"  {'─'*66+' '+'-'*7}")
        for name,n,wr,pf,dd,pct,v in sorted(oos_summary, key=lambda x:-x[5]):
            print(f"  {name:<50} {n:>4} {wr:>6.1%} {pf:>5.2f} {dd:>6.1%} {pct:>7.1f}% {v}")
else:
    print(f"\n  [OOS] Veri bulunamadı — XAUUSD_1m_2ay.csv gerekli")


# ================================================================
# 8. OOS DOĞRULAMA — Tamamen Bağımsız 2 Aylık Gerçek Veri
# ================================================================
if oos_available and df_oos is not None and len(df_oos) > 100:
    print(f"\n{'='*66}")
    print(f"  OOS DOĞRULAMA — {df_oos['datetime'].min().date()} → {df_oos['datetime'].max().date()}")
    print(f"{'='*66}")
    print(f"  Bu veri optimizasyona KATILMADI. Gerçek forward-test.")
    N_oos=len(df_oos)
    close_o=df_oos["close_1m"].values; high_o=df_oos["high_1m"].values
    low_o=df_oos["low_1m"].values; open_o=df_oos["open_1m"].values; atr_o=df_oos["atr14_1m"].values

    def run_oos2(conditions, direction, tp_mult, sl_mult):
        mask_o=np.ones(N_oos,dtype=bool)
        for col in conditions:
            if col in df_oos.columns: mask_o &= df_oos[col].values.astype(bool)
            else: return None
        idx=np.where(mask_o)[0]
        wins=0;losses=0;gp=0.0;gl=0.0
        equity=CFG.ACCOUNT_SIZE;peak=equity;max_dd=0.0;pos_close_bar=-1
        trades_list=[]
        for i in idx:
            if i+CFG.TIMEOUT_BARS>=N_oos: continue
            if i<=pos_close_bar: continue
            if i+1>=N_oos: continue
            a=atr_o[i]
            if a==0 or np.isnan(a): continue
            entry=open_o[i+1]
            if entry==0 or np.isnan(entry): continue
            lot=calc_lot(CFG.ACCOUNT_SIZE,a,sl_mult)
            if direction=="long": tp=entry+tp_mult*a; sl=entry-sl_mult*a
            else:                 tp=entry-tp_mult*a; sl=entry+sl_mult*a
            outcome=None
            for j in range(i+2,min(i+CFG.TIMEOUT_BARS+1,N_oos)):
                if direction=="long":
                    if low_o[j]<=sl:  outcome=-1;pos_close_bar=j;break
                    if high_o[j]>=tp: outcome=1; pos_close_bar=j;break
                else:
                    if high_o[j]>=sl: outcome=-1;pos_close_bar=j;break
                    if low_o[j]<=tp:  outcome=1; pos_close_bar=j;break
            if outcome is None: outcome=-1;pos_close_bar=i+CFG.TIMEOUT_BARS
            sc=CFG.SPREAD_PTS*lot*CFG.CONTRACT_SIZE/100+CFG.COMMISSION*lot
            if outcome==1: wins+=1;gp+=tp_mult*a;pnl=lot*CFG.CONTRACT_SIZE*tp_mult*a-sc
            else:          losses+=1;gl+=sl_mult*a;pnl=-lot*CFG.CONTRACT_SIZE*sl_mult*a-sc
            equity+=pnl
            if equity<=0: equity=0;break
            if equity>peak: peak=equity
            dd=(peak-equity)/peak
            if dd>max_dd: max_dd=dd
            trades_list.append({"dt":df_oos["datetime"].iloc[i],"pnl":pnl,"outcome":outcome})
        total=wins+losses
        if total<5: return None
        return {"total":total,"wr":wins/total,"pf":gp/(gl+1e-9),"max_dd":max_dd,"final_eq":equity,"trades":trades_list}

    def run_oos2_bidir(long_conditions, short_conditions, tp_mult, sl_mult):
        lm=np.ones(N_oos,dtype=bool); sm=np.ones(N_oos,dtype=bool)
        for col in long_conditions:
            if col in df_oos.columns: lm &= df_oos[col].values.astype(bool)
            else: return None
        for col in short_conditions:
            if col in df_oos.columns: sm &= df_oos[col].values.astype(bool)
            else: return None
        comb=np.zeros(N_oos,dtype=np.int8)
        comb[lm]=1; comb[sm]=np.where(comb[sm]==0,-1,comb[sm])
        idx=np.where(comb!=0)[0]
        wins=0;losses=0;gp=0.0;gl=0.0
        equity=CFG.ACCOUNT_SIZE;peak=equity;max_dd=0.0;pos_close_bar=-1;trades_list=[]
        for i in idx:
            if i+CFG.TIMEOUT_BARS>=N_oos: continue
            if i<=pos_close_bar: continue
            if i+1>=N_oos: continue
            a=atr_o[i]
            if a==0 or np.isnan(a): continue
            entry=open_o[i+1]
            if entry==0 or np.isnan(entry): continue
            direction="long" if comb[i]==1 else "short"
            lot=calc_lot(CFG.ACCOUNT_SIZE,a,sl_mult)
            if direction=="long": tp=entry+tp_mult*a; sl=entry-sl_mult*a
            else:                 tp=entry-tp_mult*a; sl=entry+sl_mult*a
            outcome=None
            for j in range(i+2,min(i+CFG.TIMEOUT_BARS+1,N_oos)):
                if direction=="long":
                    if low_o[j]<=sl:  outcome=-1;pos_close_bar=j;break
                    if high_o[j]>=tp: outcome=1; pos_close_bar=j;break
                else:
                    if high_o[j]>=sl: outcome=-1;pos_close_bar=j;break
                    if low_o[j]<=tp:  outcome=1; pos_close_bar=j;break
            if outcome is None: outcome=-1;pos_close_bar=i+CFG.TIMEOUT_BARS
            sc=CFG.SPREAD_PTS*lot*CFG.CONTRACT_SIZE/100+CFG.COMMISSION*lot
            if outcome==1: wins+=1;gp+=tp_mult*a;pnl=lot*CFG.CONTRACT_SIZE*tp_mult*a-sc
            else:          losses+=1;gl+=sl_mult*a;pnl=-lot*CFG.CONTRACT_SIZE*sl_mult*a-sc
            equity+=pnl
            if equity<=0: equity=0;break
            if equity>peak: peak=equity
            dd=(peak-equity)/peak
            if dd>max_dd: max_dd=dd
            trades_list.append({"dt":df_oos["datetime"].iloc[i],"pnl":pnl,"outcome":outcome})
        total=wins+losses
        if total<5: return None
        return {"total":total,"wr":wins/total,"pf":gp/(gl+1e-9),"max_dd":max_dd,"final_eq":equity,"trades":trades_list}

    oos_summary=[]
    for r in top_results:
        if r.get("p_value",1.0)>=0.05: continue
        if r["direction"] == "bidir":
            res = run_oos2_bidir(r.get("long_conditions",[]), r.get("short_conditions",[]),
                                 r["tp_mult"], r["sl_mult"])
        else:
            res = run_oos2(r["conditions"], r["direction"], r["tp_mult"], r["sl_mult"])
        if res is None: print(f"  {r['name'][:55]}: OOS sinyal yok"); continue
        pnl_pct=(res["final_eq"]-CFG.ACCOUNT_SIZE)/CFG.ACCOUNT_SIZE*100
        ok=res["wr"]>=0.45 and res["pf"]>=1.0 and res["max_dd"]<=0.25
        v="✓ GECTI" if ok else "✗ BASARISIZ"
        print(f"\n  {r['name'][:55]}")
        print(f"  Trade:{res['total']:>4} WR:{res['wr']:.1%} PF:{res['pf']:.2f} DD:{res['max_dd']:.1%} | {v}")
        print(f"  OOS Getiri: %{pnl_pct:.1f} (${res['final_eq']:,.0f})")
        if res["trades"]:
            dft=pd.DataFrame(res["trades"]); dft["month"]=dft["dt"].dt.to_period("M")
            mp=dft.groupby("month").agg(n=("pnl","count"),pnl=("pnl","sum"),w=("outcome",lambda x:(x==1).sum())).reset_index()
            for _,row in mp.iterrows():
                pct=row["pnl"]/CFG.ACCOUNT_SIZE*100
                print(f"    {str(row['month'])}: {row['n']:>3}t WR:{row['w']/row['n']:.0%} ${row['pnl']:>7.0f} (%{pct:.1f})")
        bt_wr=r["win_rate"]; bt_pf=r["profit_factor"]
        print(f"  Backtest: WR={bt_wr:.1%} PF={bt_pf:.2f} → OOS: WR={res['wr']:.1%} PF={res['pf']:.2f}")
        r["oos"]={"total":res["total"],"wr":round(res["wr"],4),"pf":round(res["pf"],4),
                  "max_dd":round(res["max_dd"],4),"pnl_pct":round(pnl_pct,2),"verdict":v}
        oos_summary.append((r["name"][:50],res["total"],res["wr"],res["pf"],res["max_dd"],pnl_pct,v))
    if oos_summary:
        print(f"\n{'='*66}\n  OOS OZET\n{'='*66}")
        print(f"  {'Strateji':<50} {'N':>4} {'WR':>6} {'PF':>5} {'DD':>6} {'%Getiri':>8} {'Karar'}")
        for name,n,wr,pf,dd,pct,v in sorted(oos_summary,key=lambda x:-x[5]):
            print(f"  {name:<50} {n:>4} {wr:>6.1%} {pf:>5.2f} {dd:>6.1%} {pct:>8.1f}% {v}")
else:
    print(f"\n  [OOS] {CFG.OOS_M1_FILE} bulunamadi — atlaniyor")
    print(f"  Dosyalari ayni klasore koyunuz: {CFG.OOS_M1_FILE}")

# ================================================================

# ================================================================
# C4: OOS-2 DOGRULAMA — 2010-2012 (Gecmis Rejim, 14 Yil Once)
# ================================================================



def run_oos_generic(df_test, conditions, direction, tp_mult, sl_mult):
    """Herhangi bir OOS df üzerinde backtest — tek yön."""
    N_t = len(df_test)
    try:
        close_t = df_test["close_1m"].values
        high_t  = df_test["high_1m"].values
        low_t   = df_test["low_1m"].values
        open_t  = df_test["open_1m"].values
        atr_t   = df_test["atr14_1m"].values
    except KeyError as e:
        return None
    mask_t = np.ones(N_t, dtype=bool)
    for col in conditions:
        if col in df_test.columns: mask_t &= df_test[col].values.astype(bool)
        else: return None
    idx = np.where(mask_t)[0]
    wins=0; losses=0; gp=0.0; gl=0.0
    equity=CFG.ACCOUNT_SIZE; peak=equity; max_dd=0.0; pos_close_bar=-1; trades_list=[]
    for i in idx:
        if i+CFG.TIMEOUT_BARS>=N_t or i<=pos_close_bar or i+1>=N_t: continue
        a=atr_t[i]
        if a==0 or np.isnan(a): continue
        entry=open_t[i+1]
        if entry==0 or np.isnan(entry): continue
        lot=calc_lot(CFG.ACCOUNT_SIZE,a,sl_mult)
        if direction=="long": tp=entry+tp_mult*a; sl=entry-sl_mult*a
        else:                 tp=entry-tp_mult*a; sl=entry+sl_mult*a
        outcome=None
        for j in range(i+2,min(i+CFG.TIMEOUT_BARS+1,N_t)):
            o=resolve_outcome_ambiguous(low_t[j],high_t[j],sl,tp,direction)
            if o is not None: outcome=o; pos_close_bar=j; break
        if outcome is None: outcome=-1; pos_close_bar=i+CFG.TIMEOUT_BARS
        sc=get_oos_trade_cost(lot)
        if outcome==1: wins+=1; gp+=tp_mult*a; pnl=lot*CFG.CONTRACT_SIZE*tp_mult*a-sc
        else:          losses+=1; gl+=sl_mult*a; pnl=-lot*CFG.CONTRACT_SIZE*sl_mult*a-sc
        equity+=pnl
        if equity<=0: equity=0; break
        if equity>peak: peak=equity
        dd=(peak-equity)/peak
        if dd>max_dd: max_dd=dd
        trades_list.append({"dt":df_test["datetime"].iloc[i],"pnl":pnl,"outcome":outcome})
    total=wins+losses
    if total<5: return None
    ci_lo, ci_hi = wilson_ci(wins, total)
    return {"total":total,"wr":wins/total,"pf":gp/(gl+1e-9),"max_dd":max_dd,
            "final_eq":equity,"trades":trades_list,"ci_lo":ci_lo,"ci_hi":ci_hi}

def run_oos_generic_bidir(df_test, long_cond, short_cond, tp_mult, sl_mult):
    """Bidir için OOS generic."""
    N_t=len(df_test)
    try:
        high_t=df_test["high_1m"].values; low_t=df_test["low_1m"].values
        open_t=df_test["open_1m"].values; atr_t=df_test["atr14_1m"].values
    except: return None
    lm=np.ones(N_t,dtype=bool); sm=np.ones(N_t,dtype=bool)
    for col in long_cond:
        if col in df_test.columns: lm &= df_test[col].values.astype(bool)
        else: return None
    for col in short_cond:
        if col in df_test.columns: sm &= df_test[col].values.astype(bool)
        else: return None
    comb=np.zeros(N_t,dtype=np.int8); comb[lm]=1
    comb[sm]=np.where(comb[sm]==0,-1,comb[sm])
    idx=np.where(comb!=0)[0]
    wins=0;losses=0;gp=0.0;gl=0.0
    equity=CFG.ACCOUNT_SIZE;peak=equity;max_dd=0.0;pos_close_bar=-1;trades_list=[]
    for i in idx:
        if i+CFG.TIMEOUT_BARS>=N_t or i<=pos_close_bar or i+1>=N_t: continue
        a=atr_t[i]
        if a==0 or np.isnan(a): continue
        entry=open_t[i+1]
        if entry==0 or np.isnan(entry): continue
        direction="long" if comb[i]==1 else "short"
        lot=calc_lot(CFG.ACCOUNT_SIZE,a,sl_mult)
        if direction=="long": tp=entry+tp_mult*a; sl=entry-sl_mult*a
        else:                 tp=entry-tp_mult*a; sl=entry+sl_mult*a
        outcome=None
        for j in range(i+2,min(i+CFG.TIMEOUT_BARS+1,N_t)):
            o=resolve_outcome_ambiguous(low_t[j],high_t[j],sl,tp,direction)
            if o is not None: outcome=o; pos_close_bar=j; break
        if outcome is None: outcome=-1; pos_close_bar=i+CFG.TIMEOUT_BARS
        sc=get_oos_trade_cost(lot)
        if outcome==1: wins+=1;gp+=tp_mult*a;pnl=lot*CFG.CONTRACT_SIZE*tp_mult*a-sc
        else:          losses+=1;gl+=sl_mult*a;pnl=-lot*CFG.CONTRACT_SIZE*sl_mult*a-sc
        equity+=pnl
        if equity<=0: equity=0; break
        if equity>peak: peak=equity
        dd=(peak-equity)/peak
        if dd>max_dd: max_dd=dd
        trades_list.append({"dt":df_test["datetime"].iloc[i],"pnl":pnl,"outcome":outcome})
    total=wins+losses
    if total<5: return None
    ci_lo,ci_hi=wilson_ci(wins,total)
    return {"total":total,"wr":wins/total,"pf":gp/(gl+1e-9),"max_dd":max_dd,
            "final_eq":equity,"trades":trades_list,"ci_lo":ci_lo,"ci_hi":ci_hi}


if oos2_available and df_oos2 is not None and len(df_oos2)>100:
    print(f"\n{'='*66}")
    print(f"  OOS-2 DOGRULAMA — 2010-2012 (14 YIL ONCE, TAMAMEN BAGIMSIZ)")
    print(f"  {df_oos2['datetime'].min().date()} --> {df_oos2['datetime'].max().date()}")
    print(f"{'='*66}")
    print(f"  Sistem bu veriyi HIC GORMEDI. En guclu gecmis dogrulama.")

    oos2_summary=[]
    for r in top_results:
        if r.get("p_value",1.0)>=0.05: continue
        if r["direction"]=="bidir":
            res2=run_oos_generic_bidir(df_oos2,r.get("long_conditions",[]),r.get("short_conditions",[]),r["tp_mult"],r["sl_mult"])
        else:
            res2=run_oos_generic(df_oos2,r["conditions"],r["direction"],r["tp_mult"],r["sl_mult"])
        ok2, verdict2 = oos_verdict(res2)
        pnl2 = (res2["final_eq"]-CFG.ACCOUNT_SIZE)/CFG.ACCOUNT_SIZE*100 if res2 else 0
        print(f"\n  {r['name'][:55]}")
        print(f"  {verdict2}")
        if res2 and res2["trades"]:
            dft=pd.DataFrame(res2["trades"]); dft["year"]=dft["dt"].dt.year
            yp=dft.groupby("year").agg(n=("pnl","count"),pnl=("pnl","sum"),w=("outcome",lambda x:(x==1).sum())).reset_index()
            for _,row in yp.iterrows():
                pct=row["pnl"]/CFG.ACCOUNT_SIZE*100
                print(f"    {row['year']}: {row['n']:>4} trade WR:{row['w']/row['n']:.0%} ${row['pnl']:>8.0f} (%{pct:.1f})")
        r["oos2"] = {"total":res2["total"] if res2 else 0,"wr":round(res2["wr"],4) if res2 else 0,
                     "pf":round(res2["pf"],4) if res2 else 0,"max_dd":round(res2["max_dd"],4) if res2 else 1,
                     "pnl_pct":round(pnl2,2),"verdict":"GECTI" if ok2 else "BASARISIZ",
                     "ci_lo":round(res2["ci_lo"],4) if res2 else 0,"ci_hi":round(res2["ci_hi"],4) if res2 else 0} if res2 else {"verdict":"SINYAL YOK"}
        oos2_summary.append((r["name"][:45], res2["total"] if res2 else 0,
                             res2["wr"] if res2 else 0, res2["pf"] if res2 else 0,
                             res2["max_dd"] if res2 else 1, pnl2,
                             "GECTI" if ok2 else "BASARISIZ"))
    if oos2_summary:
        print(f"\n{'='*66}")
        print(f"  OOS-2 OZET")
        print(f"{'='*66}")
        print(f"  {'Strateji':<45} {'N':>4} {'WR':>6} {'PF':>5} {'DD':>6} {'%Getiri':>8} {'Karar'}")
        for name,n,wr,pf,dd,pct,v in sorted(oos2_summary,key=lambda x:-x[5]):
            print(f"  {name:<45} {n:>4} {wr:>6.1%} {pf:>5.2f} {dd:>6.1%} {pct:>8.1f}% {v}")
else:
    print(f"\n  [OOS-2] {CFG.HIST_M1_FILE} bulunamadi — atlaniyor")

# ================================================================
# ÇIFT OOS KARAR MATRİSİ
# ================================================================
print(f"\n{'='*66}")
print(f"  CIFT OOS KARAR MATRİSİ")
print(f"{'='*66}")
print(f"  {'Strateji':<50} {'OOS-1':>8} {'OOS-2':>8} {'KARAR'}")
print(f"  {'─'*66}")
for r in top_results:
    if r.get("p_value",1.0)>=0.05: continue
    oos1_v = r.get("oos",{}).get("verdict","YOK")
    oos2_v = r.get("oos2",{}).get("verdict","YOK") if oos2_available else "YOK"
    o1ok = "GECTI" in oos1_v if oos1_v!="YOK" else None
    o2ok = "GECTI" in oos2_v if oos2_v!="YOK" else None
    if o1ok and o2ok:
        karar = "✓✓ CIFT ONAY — canliya al"
    elif o1ok and o2ok==False:
        karar = "⚠  MODERN REJIM OZEL — dikkatli"
    elif o1ok==False and o2ok:
        karar = "⚠  ESKI REJIM OZEL — kullanma"
    elif o1ok==False and o2ok==False:
        karar = "✗✗ IKISI DE BASARISIZ — overfit"
    elif o1ok and o2ok is None:
        karar = "✓  OOS-1 GECTI (OOS-2 yok)"
    else:
        karar = "?  Veri yetersiz"
    print(f"  {r['name'][:50]:<50} {('✓' if o1ok else ('✗' if o1ok==False else '?')):>8} {('✓' if o2ok else ('✗' if o2ok==False else '?')):>8}  {karar}")

# ================================================================
# D1: SİNYAL KORELASYON ANALİZİ
# ================================================================
print(f"\n{'='*66}")
print(f"  D1: SiNYAL KORELASYON ANALiZi (top stratejiler)")
print(f"{'='*66}")
edge_strats = [r for r in top_results if r.get("p_value",1.0)<0.05]
if len(edge_strats) >= 2:
    # Her strateji için sinyal dizisi oluştur
    sig_arrays = {}
    for r in edge_strats:
        if r["direction"]=="bidir":
            lm=np.ones(N,dtype=bool); sm=np.ones(N,dtype=bool)
            for col in r.get("long_conditions",[]): 
                if col in df.columns: lm &= df[col].values.astype(bool)
            for col in r.get("short_conditions",[]):
                if col in df.columns: sm &= df[col].values.astype(bool)
            sig_arrays[r["name"][:30]] = (lm | sm)[TRAIN_END:]
        else:
            mask=np.ones(N,dtype=bool)
            for col in r["conditions"]:
                if col in df.columns: mask &= df[col].values.astype(bool)
            sig_arrays[r["name"][:30]] = mask[TRAIN_END:]
    names = list(sig_arrays.keys())
    print(f"\n  {'Strateji A':<32} {'Strateji B':<32} {'Jaccard':>8} {'Uyari'}")
    print(f"  {'─'*80}")
    for i in range(len(names)):
        for j in range(i+1, len(names)):
            a = sig_arrays[names[i]].astype(bool)
            b = sig_arrays[names[j]].astype(bool)
            inter = (a & b).sum()
            union = (a | b).sum()
            jaccard = inter / (union + 1e-9)
            warn = " ⚠ YUKSEK ORTU" if jaccard > 0.5 else ""
            print(f"  {names[i]:<32} {names[j]:<32} {jaccard:>8.2%}{warn}")
    print(f"\n  Jaccard > 0.5 ise iki strateji birbirinin benzeri — portfolio cesitliligi az")

# ================================================================
# D2: HALF-KELLY POZİSYON BOYUTU
# ================================================================
print(f"\n{'='*66}")
print(f"  D2: HALF-KELLY POZiSYON BOYUTU ONERiLERi")
print(f"{'='*66}")
for r in edge_strats[:5]:
    wr  = r["win_rate"]
    rr  = r["tp_mult"] / r["sl_mult"]
    kelly = (wr * rr - (1-wr)) / rr
    half_kelly = max(0.0, kelly * 0.5)
    current = CFG.RISK_PCT
    print(f"  {r['name'][:52]}")
    print(f"    Kelly=%{kelly*100:.1f}  Half-Kelly=%{half_kelly*100:.1f}  Mevcut=%{current*100:.1f}  {'✓ OK' if current <= half_kelly*1.2 else '⚠ Cok yuksek risk!'}")

# ================================================================
# D4: HAFTALIK PnL DAGILIMI
# ================================================================
print(f"\n{'='*66}")
print(f"  D4: HAFTALIK PnL DAGILIMI (edge kanıtlı stratejiler)")
print(f"{'='*66}")

avg_atr_test_w = float(df["atr14_1m"].iloc[TRAIN_END:].mean())
fixed_lot_w    = max(CFG.MIN_LOT, min(1.0, calc_lot(CFG.ACCOUNT_SIZE, avg_atr_test_w, 1.0)))

for r in edge_strats[:3]:
    if r["direction"] == "bidir":
        lm=np.ones(N,dtype=bool); sm=np.ones(N,dtype=bool)
        for col in r.get("long_conditions",[]): 
            if col in df.columns: lm &= df[col].values.astype(bool)
        for col in r.get("short_conditions",[]):
            if col in df.columns: sm &= df[col].values.astype(bool)
        comb_w=np.zeros(N,dtype=np.int8); comb_w[lm]=1
        comb_w[sm]=np.where(comb_w[sm]==0,-1,comb_w[sm])
        idx_w=np.where(comb_w[TRAIN_END:]!=0)[0]+TRAIN_END
        dir_fn = lambda i: "long" if comb_w[i]==1 else "short"
    else:
        mw=np.ones(N,dtype=bool)
        for col in r["conditions"]:
            if col in df.columns: mw &= df[col].values.astype(bool)
        idx_w=np.where(mw[TRAIN_END:])[0]+TRAIN_END
        _dir=r["direction"]; dir_fn=lambda i: _dir
    
    trades_w=[]; pos_cb=-1
    for i in idx_w:
        if i+CFG.TIMEOUT_BARS>=N or i<=pos_cb or i+1>=N: continue
        a=atr[i]
        if a==0 or np.isnan(a): continue
        entry=open_p[i+1]
        if entry==0 or np.isnan(entry): continue
        direction=dir_fn(i)
        tp=entry+r["tp_mult"]*a if direction=="long" else entry-r["tp_mult"]*a
        sl=entry-r["sl_mult"]*a if direction=="long" else entry+r["sl_mult"]*a
        outcome=None
        for j in range(i+2,min(i+CFG.TIMEOUT_BARS+1,N)):
            o=resolve_outcome_ambiguous(low[j],high[j],sl,tp,direction)
            if o is not None: outcome=o; pos_cb=j; break
        if outcome is None: outcome=-1; pos_cb=i+CFG.TIMEOUT_BARS
        sc=get_trade_cost(fixed_lot_w,i)
        pnl=(fixed_lot_w*CFG.CONTRACT_SIZE*(r["tp_mult"]*a if outcome==1 else -r["sl_mult"]*a))-sc
        trades_w.append({"dt":df["datetime"].iloc[i],"pnl":pnl,"outcome":outcome})
    
    if not trades_w: continue
    dft_w=pd.DataFrame(trades_w)
    dft_w["week"]=dft_w["dt"].dt.to_period("W")
    wp=dft_w.groupby("week").agg(n=("pnl","count"),pnl=("pnl","sum"),w=("outcome",lambda x:(x==1).sum())).reset_index()
    wp["wr"]=wp["w"]/wp["n"]
    wp["pct"]=wp["pnl"]/CFG.ACCOUNT_SIZE*100
    
    print(f"\n  {r['name'][:55]}")
    pos_weeks=(wp["pct"]>0).sum(); neg_weeks=(wp["pct"]<0).sum()
    avg_wk=wp["pct"].mean(); best_wk=wp["pct"].max(); worst_wk=wp["pct"].min()
    print(f"  Pozitif hafta: {pos_weeks}/{len(wp)} | Avg: %{avg_wk:.1f} | En iyi: %{best_wk:.1f} | En kotu: %{worst_wk:.1f}")
    # Son 12 hafta
    print(f"  Son 12 hafta:")
    for _,row in wp.tail(12).iterrows():
        bar = "▓"*int(abs(row["pct"])/2) if row["pct"]>0 else "░"*int(abs(row["pct"])/2)
        sign = "+" if row["pct"]>0 else ""
        print(f"    {str(row['week'])}: {sign}{row['pct']:>6.1f}% ({row['n']:>3}t WR:{row['wr']:.0%}) {bar[:20]}")

# ================================================================
# E1+E2: CANLI SİNYAL DOSYASI + STRATEJİ KARTLARI
# ================================================================
print(f"\n{'='*66}")
print(f"  E1: CANLI SiNYAL DOSYASI (current_signals.json)")
print(f"{'='*66}")

# Son barda sinyal var mı?
last_bar = len(df) - 1
current_signals = []
for r in top_results:
    if r.get("p_value",1.0) >= 0.05: continue
    if r["direction"]=="bidir":
        lm=np.ones(N,dtype=bool); sm=np.ones(N,dtype=bool)
        for col in r.get("long_conditions",[]): 
            if col in df.columns: lm &= df[col].values.astype(bool)
        for col in r.get("short_conditions",[]):
            if col in df.columns: sm &= df[col].values.astype(bool)
        long_sig  = bool(lm[last_bar])
        short_sig = bool(sm[last_bar])
        if long_sig or short_sig:
            current_signals.append({
                "strategy": r["name"], "direction": "bidir",
                "active_dir": "long" if long_sig else "short",
                "tp_mult": r["tp_mult"], "sl_mult": r["sl_mult"],
                "bar_time": str(df["datetime"].iloc[last_bar]),
                "atr": round(float(atr[last_bar]),4),
            })
    else:
        mask=np.ones(N,dtype=bool)
        for col in r["conditions"]:
            if col in df.columns: mask &= df[col].values.astype(bool)
        if mask[last_bar]:
            current_signals.append({
                "strategy": r["name"], "direction": r["direction"],
                "active_dir": r["direction"],
                "tp_mult": r["tp_mult"], "sl_mult": r["sl_mult"],
                "bar_time": str(df["datetime"].iloc[last_bar]),
                "atr": round(float(atr[last_bar]),4),
            })

if current_signals:
    print(f"  Son barda {len(current_signals)} aktif sinyal:")
    for s in current_signals:
        print(f"    {s['strategy'][:50]} → {s['active_dir'].upper()}")
else:
    print(f"  Son barda aktif sinyal yok.")

with open("current_signals.json","w",encoding="utf-8") as f:
    json.dump({"timestamp":str(df["datetime"].iloc[last_bar]),"signals":current_signals},f,ensure_ascii=False,indent=2)
print(f"  [✓] current_signals.json kaydedildi.")

# E2: Strateji kartları
strategy_cards = []
for r in top_results:
    if r.get("p_value",1.0)>=0.05: continue
    avg_atr_c = float(df["atr14_1m"].iloc[TRAIN_END:].mean())
    typ_lot   = calc_lot(CFG.ACCOUNT_SIZE, avg_atr_c, r["sl_mult"])
    card = {
        "name": r["name"], "version": "p5",
        "direction": r["direction"],
        "entry_conditions": r.get("conditions", r.get("long_conditions",[])),
        "entry_conditions_short": r.get("short_conditions", []),
        "tp_atr_mult": r["tp_mult"], "sl_atr_mult": r["sl_mult"],
        "rr": r["rr"],
        "typical_lot": round(typ_lot,2),
        "typical_sl_usd": round(typ_lot * CFG.CONTRACT_SIZE * avg_atr_c * r["sl_mult"],2),
        "typical_tp_usd": round(typ_lot * CFG.CONTRACT_SIZE * avg_atr_c * r["tp_mult"],2),
        "backtest_wr": r["win_rate"], "backtest_pf": r["profit_factor"],
        "oos1": r.get("oos",{}), "oos2": r.get("oos2",{}),
        "p_value": r.get("p_value",1), "bonferroni_pass": r.get("bonferroni_pass",False),
        "edge_label": r.get("edge_label","?"),
        "wf_pass": r.get("wf_pass",0),
    }
    strategy_cards.append(card)

with open("strategy_cards.json","w",encoding="utf-8") as f:
    json.dump(strategy_cards,f,ensure_ascii=False,indent=2)
print(f"  [✓] strategy_cards.json kaydedildi ({len(strategy_cards)} strateji).")

# JSON KAYDET
# ================================================================
def clean(obj):
    if isinstance(obj, dict):
        return {k: clean(v) for k, v in obj.items() if k != "eq_curve"}
    if isinstance(obj, list):
        return [clean(v) for v in obj]
    if isinstance(obj, (np.integer,)): return int(obj)
    if isinstance(obj, (np.floating,)): return float(obj)
    if isinstance(obj, (np.bool_,)): return bool(obj)
    return obj

out = {
    "timestamp": datetime.now().isoformat(),
    "version": "p5",
    "config": {
        "train_ratio": CFG.TRAIN_RATIO,
        "min_trades":  CFG.MIN_TRADES,
        "min_pf":      CFG.MIN_PF,
        "min_wr":      CFG.MIN_WR,
        "max_dd":      CFG.MAX_DD,
        "n_sim":       CFG.N_SIM,
        "risk_per_trade": CFG.RISK_PER_TRADE,
        "spread_pts":  CFG.SPREAD_PTS,
        "slippage_pts":CFG.SLIPPAGE_PTS,
        "bonferroni_alpha": CFG.BONFERRONI_ALPHA,
        "oos1_available": oos1_available,
        "oos2_available": oos2_available,
        "hist_available": hist_available,
    },
    "total_strategies_found": len(all_results),
    "long_strategies":  sum(1 for r in all_results if r["direction"]=="long"),
    "short_strategies": sum(1 for r in all_results if r["direction"]=="short"),
    "bidir_strategies": sum(1 for r in all_results if r["direction"]=="bidir"),
    "top_strategies": clean(top_results),
    "strategy_cards": strategy_cards,
}
with open("xauusd_results.json","w",encoding="utf-8") as f:
    json.dump(out, f, ensure_ascii=False, indent=2)
print(f"\n  [\u2713] xauusd_results.json kaydedildi.")
print(f"\n{SEP}")
print(f"  TAMAMLANDI — p5 (Double OOS + 17 Duzeltme)")
print(SEP + "\n")

# ── DOSYAYI KAPAT VE BİLGİ VER ───────────────────────────────────
sys.stdout = sys.__stdout__
sys.stderr = sys.__stderr__
_log_file.close()
print("\n" + "="*50)
print("  sonuclar.txt kaydedildi!")
print("  Notepad++ veya VS Code ile acin.")
print("  Ctrl+F ile strateji ismi aratabilirsiniz.")
print("="*50 + "\n")
with open("xauusd_results.json","w",encoding="utf-8") as f:
    json.dump(out, f, ensure_ascii=False, indent=2)
print(f"\n  [✓] xauusd_results.json kaydedildi.")
print(f"\n{SEP}")
print(f"  TAMAMLANDI")
print(SEP + "\n")