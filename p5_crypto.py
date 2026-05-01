"""
p5_crypto.py — p5.py'nin SQLite DB adaptörü
Kullanim: python3 p5_crypto.py --db data/trader.db --symbol ZECUSDT \
          --market spot --exchange binance --interval 15m --out-dir data/p5_results

Farkliliklari:
  - CSV yerine SQLite candles tablosundan okur
  - Crypto maliyet modeli (spread=0, commission=%0.05 taker, slippage=%0.02)
  - Cuma cutoff / haber saati filtresi YOK (opsiyonel session feature olarak kaldi)
  - MIN_TRADES dusuk (15m+ TF'de veri az olabilir)
  - Sonuclari JSON'a yazar → rtc_cli TUI paneli okur
"""
import warnings; warnings.filterwarnings("ignore")
import numpy as np
import pandas as pd
import json, os, math, sys, sqlite3, argparse
from datetime import datetime
from itertools import combinations

np.random.seed(42)

# ── CLI ARGS ──────────────────────────────────────────────────────
parser = argparse.ArgumentParser()
parser.add_argument("--db",       default="data/trader.db")
parser.add_argument("--symbol",   default="ZECUSDT")
parser.add_argument("--market",   default="spot")
parser.add_argument("--exchange", default="binance")
parser.add_argument("--interval", default="15m")
parser.add_argument("--out-dir",  default="data/p5_results")
parser.add_argument("--min-trades", type=int, default=40)
parser.add_argument("--n-sim",    type=int, default=5000)
args = parser.parse_args()

OUT_DIR = args.out_dir
os.makedirs(OUT_DIR, exist_ok=True)

LOG_FILE = os.path.join(OUT_DIR, f"{args.symbol}_{args.interval}.log")
_lf = open(LOG_FILE, "w", encoding="utf-8")
def log(msg):
    print(msg); _lf.write(msg+"\n"); _lf.flush()

def write_status(state, msg, extra=None):
    d = {"state": state, "msg": msg, "ts": datetime.now().isoformat(),
         "symbol": args.symbol, "interval": args.interval,
         "exchange": args.exchange, "market": args.market}
    if extra: d.update(extra)
    with open(os.path.join(OUT_DIR, "status.json"), "w") as f:
        json.dump(d, f, ensure_ascii=False, indent=2)

write_status("running", "Başlatıldı")

# ================================================================
# AYARLAR — Crypto uyarlanmış
# ================================================================
class CFG:
    COMMISSION_PCT  = 0.0005   # %0.05 taker (Binance varsayilan)
    SLIPPAGE_PCT    = 0.0002   # %0.02 kayma
    TRAIN_RATIO     = 0.75
    RECENT_RATIO    = 0.92
    TIMEOUT_BARS    = 40       # p5: 60 (1m) → burada TF daha yuksek
    MIN_TRADES      = args.min_trades
    MIN_RECENT_TRADES = max(10, args.min_trades // 4)
    MIN_PF          = 1.15
    MIN_WR          = 0.44
    MAX_DD          = 0.25
    MAX_WR_DECAY    = 0.18
    MAX_PF_DECAY    = 0.38
    TP_SL_GRID = [
        (1.5, 1.0), (2.0, 1.0), (2.5, 1.0), (3.0, 1.0),
        (2.0, 0.8), (2.5, 0.8), (3.0, 0.8),
        (1.5, 0.7), (2.0, 0.7), (2.5, 0.7),
    ]
    N_SIM           = args.n_sim
    STARTING_BAL    = 10000.0
    RISK_PER_TRADE  = 0.01
    TOP_N           = 10
    BONFERRONI_ALPHA= 0.05

# TF resample haritasi: her base interval icin ustu TF'ler
TF_RESAMPLE = {
    "1m":  {"mid": "5min",  "high": "15min", "bias": "1h"},
    "3m":  {"mid": "15min", "high": "30min", "bias": "2h"},
    "5m":  {"mid": "15min", "high": "1h",    "bias": "4h"},
    "15m": {"mid": "1h",    "high": "4h",    "bias": "1D"},
    "30m": {"mid": "2h",    "high": "6h",    "bias": "1D"},
    "1h":  {"mid": "4h",    "high": "1D",    "bias": "3D"},
    "4h":  {"mid": "1D",    "high": "3D",    "bias": "1W"},
}

# ================================================================
# 1. VERİ YÜKLE — SQLite DB
# ================================================================
log(f"\n{'='*60}")
log(f"  p5_crypto — {args.exchange}:{args.market} {args.symbol} {args.interval}")
log(f"{'='*60}")
log(f"\n[1/5] Veri yükleniyor — {args.db}")

def load_from_db(db_path, exchange, market, symbol, interval):
    conn = sqlite3.connect(db_path)
    df = pd.read_sql_query(
        """SELECT timestamp as datetime, open, high, low, close, volume
           FROM candles
           WHERE exchange=? AND market=? AND symbol=? AND interval=?
           ORDER BY timestamp ASC""",
        conn, params=(exchange, market, symbol, interval)
    )
    conn.close()
    if df.empty:
        return df
    df["datetime"] = pd.to_datetime(df["datetime"], unit="ms", utc=True).dt.tz_localize(None)
    for col in ["open","high","low","close","volume"]:
        df[col] = pd.to_numeric(df[col], errors="coerce")
    return df.dropna(subset=["open","high","low","close"]).reset_index(drop=True)

def resample_tf(df_base, rule):
    d = df_base.set_index("datetime")
    agg = {k:v for k,v in {"open":"first","high":"max","low":"min","close":"last","volume":"sum"}.items() if k in d.columns}
    r = d.resample(rule, label="left", closed="left").agg(agg).dropna(subset=["open","close"])
    return r.reset_index()

df_base = load_from_db(args.db, args.exchange, args.market, args.symbol, args.interval)
if len(df_base) < 100:
    log(f"  HATA: Yetersiz veri ({len(df_base)} mum). En az 100 mum gerekli.")
    write_status("error", f"Yetersiz veri: {len(df_base)} mum", {"bars": len(df_base)})
    sys.exit(1)

log(f"  BASE ({args.interval}): {len(df_base):,} mum | {df_base['datetime'].min().date()} → {df_base['datetime'].max().date()}")

resample_rules = TF_RESAMPLE.get(args.interval, {"mid":"4h","high":"1D","bias":"1W"})
df_mid  = resample_tf(df_base, resample_rules["mid"])
df_high = resample_tf(df_base, resample_rules["high"])
df_bias = resample_tf(df_base, resample_rules["bias"])
log(f"  MID  ({resample_rules['mid']}): {len(df_mid):,} mum")
log(f"  HIGH ({resample_rules['high']}): {len(df_high):,} mum")
log(f"  BIAS ({resample_rules['bias']}): {len(df_bias):,} mum")

# ================================================================
# 2. FEATURE ENGINEERING
# ================================================================
log(f"\n[2/5] Feature engineering...")

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
    df["rsi_rising"]  = df["rsi14"] > df["rsi14"].shift(3)
    df["rsi_falling"] = df["rsi14"] < df["rsi14"].shift(3)
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
    df["bb_upper"] = bb_mid+2*bb_std; df["bb_lower"] = bb_mid-2*bb_std
    df["bb_width"]   = (df["bb_upper"]-df["bb_lower"])/(bb_mid+1e-9)
    df["bb_squeeze"] = df["bb_width"]<df["bb_width"].rolling(50).quantile(0.2)
    df["at_bb_lower"]= c<df["bb_lower"]; df["at_bb_upper"]= c>df["bb_upper"]
    df["_date"] = df["datetime"].dt.date; df["_tp"] = (h+l+c)/3
    if "volume" in df.columns and df["volume"].sum()>0:
        try:
            df["_ctv"] = df.groupby("_date").apply(lambda g:(g["_tp"]*g["volume"]).cumsum()).reset_index(level=0,drop=True)
            df["_cv"]  = df.groupby("_date")["volume"].cumsum()
            df["vwap"] = df["_ctv"]/(df["_cv"]+1e-9)
        except Exception:
            df["vwap"] = c.rolling(20).mean()
    else:
        df["vwap"] = c.rolling(20).mean()
    df["above_vwap"]= c>df["vwap"]; df["below_vwap"]= c<df["vwap"]
    df = df.drop(columns=["_date","_tp","_ctv","_cv"],errors="ignore")
    df["fvg_bull"] = l>h.shift(2); df["fvg_bear"] = h<l.shift(2)
    body=(c-o).abs(); down_c=c<o; up_c=c>o
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
    df["mom3_up"]  = (c-c.shift(3))>0; df["mom3_dn"]  = (c-c.shift(3))<0
    df["hour"] = df["datetime"].dt.hour
    df["london"] = df["hour"].between(7,11); df["ny"] = df["hour"].between(13,17)
    df["session"] = df["london"]|df["ny"]
    df["trend_up"] = c>c.rolling(15).mean(); df["trend_dn"] = c<c.rolling(15).mean()
    # Williams %R
    hh14b=h.rolling(14).max(); ll14b=l.rolling(14).min()
    df["willr"] = -100*(hh14b-c)/(hh14b-ll14b+1e-9)
    df["willr_os"]= df["willr"]<-80; df["willr_ob"]= df["willr"]>-20
    # CCI
    tp2=(h+l+c)/3; md=tp2.rolling(20).apply(lambda x:np.mean(np.abs(x-x.mean())))
    df["cci"]    = (tp2-tp2.rolling(20).mean())/(0.015*md+1e-9)
    df["cci_os"] = df["cci"]<-100; df["cci_ob"] = df["cci"]>100
    # ADX
    dmplus=h.diff().clip(lower=0); dmminus=(-l.diff()).clip(lower=0)
    tr2=pd.concat([(h-l),(h-c.shift(1)).abs(),(l-c.shift(1)).abs()],axis=1).max(axis=1)
    dip=100*(dmplus.rolling(14).mean()/(tr2.rolling(14).mean()+1e-9))
    dim=100*(dmminus.rolling(14).mean()/(tr2.rolling(14).mean()+1e-9))
    dx=100*(dip-dim).abs()/(dip+dim+1e-9)
    df["adx"]       = dx.rolling(14).mean()
    df["adx_strong"]= df["adx"]>25
    # Volume
    if "volume" in df.columns and df["volume"].sum()>0:
        vol_ma = df["volume"].rolling(20).mean()
        df["vol_spike"] = df["volume"]>vol_ma*1.5
        df["vol_dry"]   = df["volume"]<vol_ma*0.5
        df["vol_expand"]= df["volume"]>vol_ma*1.2
        obv=(np.where(up_c,df["volume"],-df["volume"])).cumsum()
        df["obv_up"] = obv>np.roll(obv,5); df["obv_dn"] = obv<np.roll(obv,5)
    else:
        df["vol_spike"]=False; df["vol_dry"]=False; df["vol_expand"]=False
        df["obv_up"]=False; df["obv_dn"]=False
    # Parabolic SAR (tam impl.)
    af_step=0.02; af_max=0.20
    psar_arr=np.zeros(len(df)); psar_bull_arr=np.zeros(len(df),dtype=bool)
    psar_flip_b=np.zeros(len(df),dtype=bool); psar_flip_s=np.zeros(len(df),dtype=bool)
    h_arr=h.values; l_arr=l.values
    bull=True; af=af_step; ep=l_arr[0]; sar=h_arr[0]
    for i in range(2,len(df)):
        prev_bull=bull
        if bull:
            sar=sar+af*(ep-sar); sar=min(sar,l_arr[i-1],l_arr[i-2])
            if l_arr[i]<sar: bull=False; sar=ep; ep=h_arr[i]; af=af_step
            elif h_arr[i]>ep: ep=h_arr[i]; af=min(af+af_step,af_max)
        else:
            sar=sar+af*(ep-sar); sar=max(sar,h_arr[i-1],h_arr[i-2])
            if h_arr[i]>sar: bull=True; sar=ep; ep=l_arr[i]; af=af_step
            elif l_arr[i]<ep: ep=l_arr[i]; af=min(af+af_step,af_max)
        psar_arr[i]=sar; psar_bull_arr[i]=bull
        psar_flip_b[i]=bull and not prev_bull; psar_flip_s[i]=(not bull) and prev_bull
    df["real_psar"]=psar_arr; df["real_psar_bull"]=psar_bull_arr
    df["real_psar_bear"]=~psar_bull_arr
    df["real_psar_flip_bull"]=psar_flip_b; df["real_psar_flip_bear"]=psar_flip_s
    df["above_psar"]=c>df["real_psar"]; df["below_psar"]=c<df["real_psar"]
    # CHoCH
    prev_ll=l.rolling(10).min().shift(1); prev_hh=h.rolling(10).max().shift(1)
    df["choch_bull"]=(l<prev_ll)&(c>prev_ll); df["choch_bear"]=(h>prev_hh)&(c<prev_hh)
    df["choch_bull_strong"]=df["choch_bull"]&((prev_ll-l)>df["atr14"]*0.3)
    df["choch_bear_strong"]=df["choch_bear"]&((h-prev_hh)>df["atr14"]*0.3)
    # OB güçlü
    ob_body_max=body.rolling(5).max()
    df["ob_bull_strong"]=down_c&(body==ob_body_max)&(body>df["atr14"]*0.7)
    df["ob_bear_strong"]=up_c&(body==ob_body_max)&(body>df["atr14"]*0.7)
    df["unicorn_bull"]=df["ob_bull"]&df["fvg_bull"]; df["unicorn_bear"]=df["ob_bear"]&df["fvg_bear"]
    df["unicorn_bull_strong"]=df["ob_bull_strong"]&df["fvg_bull"]
    df["unicorn_bear_strong"]=df["ob_bear_strong"]&df["fvg_bear"]
    # Wyckoff
    s20b=l.rolling(20).min().shift(1); r20b=h.rolling(20).max().shift(1)
    df["wyckoff_spring"]=(l<s20b)&(c>s20b)&(c>o); df["wyckoff_upthrust"]=(h>r20b)&(c<r20b)&(c<o)
    # Fibonacci
    sh2=h.rolling(20).max(); sl2b=l.rolling(20).min(); rng=(sh2-sl2b)
    fib618=sl2b+rng*0.618; fib786=sl2b+rng*0.786
    df["at_fib618_support"]=(c>=fib618-df["atr14"]*0.2)&(c<=fib618+df["atr14"]*0.2)
    df["at_fib382_support"]=(c>=(sl2b+rng*0.382)-df["atr14"]*0.2)&(c<=(sl2b+rng*0.382)+df["atr14"]*0.2)
    df["at_fib618_resist"] =(c>=(sh2-rng*0.618)-df["atr14"]*0.2)&(c<=(sh2-rng*0.618)+df["atr14"]*0.2)
    df["at_fib382_resist"] =(c>=(sh2-rng*0.382)-df["atr14"]*0.2)&(c<=(sh2-rng*0.382)+df["atr14"]*0.2)
    df["in_golden_zone_bull"]=(c>=fib618)&(c<=fib786)
    df["in_golden_zone_bear"]=(c>=(sh2-rng*0.786))&(c<=(sh2-rng*0.618))
    # Ichimoku (basit)
    tenkan=(h.rolling(9).max()+l.rolling(9).min())/2; kijun=(h.rolling(26).max()+l.rolling(26).min())/2
    df["ichi_bull"]=tenkan>kijun; df["ichi_bear"]=tenkan<kijun
    df["ichi_cross_bull"]=(tenkan>kijun)&(tenkan.shift(1)<=kijun.shift(1))
    df["ichi_cross_bear"]=(tenkan<kijun)&(tenkan.shift(1)>=kijun.shift(1))
    # HH/LL trend
    hh3=h.rolling(3).max(); ll3=l.rolling(3).min()
    df["hh_trend"]=(h>hh3.shift(3))&(l>ll3.shift(3))
    df["ll_trend"]=(h<hh3.shift(3))&(l<ll3.shift(3))
    # RSI diverjans
    df["rsi_div_bull"]=(c<c.shift(5))&(df["rsi14"]>df["rsi14"].shift(5))&(df["rsi14"]<50)
    df["rsi_div_bear"]=(c>c.shift(5))&(df["rsi14"]<df["rsi14"].shift(5))&(df["rsi14"]>50)
    # Buy/sell pressure
    if "volume" in df.columns and df["volume"].sum()>0:
        buy_vol=df["volume"].where(up_c,0); sell_vol=df["volume"].where(down_c,0)
        df["buy_pressure"]=buy_vol.rolling(5).mean()>sell_vol.rolling(5).mean()
        df["sell_pressure"]=sell_vol.rolling(5).mean()>buy_vol.rolling(5).mean()
        df["vol_confirm_bull"]=up_c&df["vol_spike"]; df["vol_confirm_bear"]=down_c&df["vol_spike"]
    else:
        df["buy_pressure"]=False; df["sell_pressure"]=False
        df["vol_confirm_bull"]=False; df["vol_confirm_bear"]=False
    # BB breakout, Keltner
    df["bb_breakout_up"]=(~df["bb_squeeze"])&df["bb_squeeze"].shift(1)&up_c
    df["bb_breakout_dn"]=(~df["bb_squeeze"])&df["bb_squeeze"].shift(1)&down_c
    kc_mid=c.ewm(span=20,adjust=False).mean()
    df["kc_upper"]=kc_mid+2*df["atr14"]; df["kc_lower"]=kc_mid-2*df["atr14"]
    df["at_kc_lower"]=c<df["kc_lower"]; df["at_kc_upper"]=c>df["kc_upper"]
    # ROC, AO
    df["roc5"]=c.pct_change(5)*100; df["roc_pos"]=df["roc5"]>0; df["roc_neg"]=df["roc5"]<0
    mp=(h+l)/2; ao=mp.rolling(5).mean()-mp.rolling(34).mean()
    df["ao_pos"]=ao>0; df["ao_neg"]=ao<0
    # EMA slope (5m eşdeğeri)
    ema21=c.ewm(span=21,adjust=False).mean()
    df["ema21_slope_up"]=ema21>ema21.shift(3); df["ema21_slope_dn"]=ema21<ema21.shift(3)
    # Donchian mid
    dc_h=h.rolling(20).max(); dc_l=l.rolling(20).min(); dc_mid=(dc_h+dc_l)/2
    df["above_dc_mid"]=c>dc_mid; df["below_dc_mid"]=c<dc_mid
    df["dc_breakout_up"]=(c>dc_h.shift(1)); df["dc_breakout_dn"]=(c<dc_l.shift(1))
    # Pivot, quality
    pp=(h.shift(1)+l.shift(1)+c.shift(1))/3
    df["above_pp"]=c>pp; df["below_pp"]=c<pp
    full_range=h-l
    df["quality_candle"]=(h-l)>df["atr14"]*0.5; df["doji"]=body<(h-l)*0.1; df["no_doji"]=~df["doji"]
    # Strong bull/bear align
    bull_score=((df["ema8"]>df["ema21"]).astype(int)+(df["ema21"]>df["ema50"]).astype(int)
                +(df["rsi14"]>50).astype(int)+(df["macd"]>df["macd_sig"]).astype(int))
    df["strong_bull_align"]=bull_score>=3; df["strong_bear_align"]=(4-bull_score)>=3
    # ATR percentile
    df["atr_pct80"]=df["atr14"]>df["atr14"].rolling(100).quantile(0.80)
    df["atr_pct20"]=df["atr14"]<df["atr14"].rolling(100).quantile(0.20)
    # Candle patterns
    df["bull_marubozu"]=up_c&(body>full_range*0.85)&(uw<full_range*0.05)
    df["bear_marubozu"]=down_c&(body>full_range*0.85)&(lw<full_range*0.05)
    df["three_soldiers"]=up_c&up_c.shift(1)&up_c.shift(2)&(c>c.shift(1))&(c.shift(1)>c.shift(2))
    df["three_crows"]=down_c&down_c.shift(1)&down_c.shift(2)&(c<c.shift(1))&(c.shift(1)<c.shift(2))
    df["tweezer_top"]=(abs(h-h.shift(1))<df["atr14"]*0.05)&up_c.shift(1)&down_c
    df["tweezer_bot"]=(abs(l-l.shift(1))<df["atr14"]*0.05)&down_c.shift(1)&up_c
    df["shooting_star"]=(uw>body*2)&(uw>lw*3)&(body<full_range*0.3)&up_c.shift(1)
    df["dark_cloud"]=(up_c.shift(1))&down_c&(o>h.shift(1))&(c<(o.shift(1)+c.shift(1))/2)
    df["piercing"]=(down_c.shift(1))&up_c&(o<l.shift(1))&(c>(o.shift(1)+c.shift(1))/2)
    df["bull_harami"]=down_c.shift(1)&up_c&(o>c.shift(1))&(c<o.shift(1))
    df["bear_harami"]=up_c.shift(1)&down_c&(o<c.shift(1))&(c>o.shift(1))
    df["breaker_bull"]=df["ob_bear"].shift(3)&(c>c.shift(3))
    df["breaker_bear"]=df["ob_bull"].shift(3)&(c<c.shift(3))
    # OTE
    swing_h5b=h.rolling(5).max(); swing_l5b=l.rolling(5).min()
    fib618b=(swing_h5b-swing_l5b)*0.618; fib786b=(swing_h5b-swing_l5b)*0.786
    df["ote_bull"]=(c>=swing_l5b+fib618b)&(c<=swing_l5b+fib786b)&(c<swing_h5b)
    df["ote_bear"]=(c>=swing_h5b-fib786b)&(c<=swing_h5b-fib618b)&(c>swing_l5b)
    # Session (crypto 24/7 — opsiyonel filtre)
    df["killzone_london"]=df["hour"].between(8,10)
    df["killzone_ny"]=df["hour"].between(13,15)
    df["po3_distribution"]=df["hour"].between(13,17)
    # Rejim
    ema50s=c.ewm(span=50,adjust=False).mean()
    df["regime_bull"]=(ema50s>ema50s.shift(3))&(df["adx"]>25)
    df["regime_bear"]=(ema50s<ema50s.shift(3))&(df["adx"]>25)
    return df.add_suffix(f"_{tf}")

def add_features_htf(df):
    """HTF bias (eski '1h' rolü)."""
    df=df.copy(); c=df["close"]; h2=df["high"]; l2=df["low"]
    df["htf_ema20"]=c.ewm(span=20,adjust=False).mean()
    df["htf_ema50"]=c.ewm(span=50,adjust=False).mean()
    df["htf_bull"]=(c>df["htf_ema20"])&(df["htf_ema20"]>df["htf_ema50"])
    df["htf_bear"]=(c<df["htf_ema20"])&(df["htf_ema20"]<df["htf_ema50"])
    delta=c.diff(); gain=delta.clip(lower=0).rolling(14).mean()
    loss=(-delta.clip(upper=0)).rolling(14).mean()
    htf_rsi=100-100/(1+gain/(loss+1e-9))
    df["htf_rsi_bull"]=htf_rsi>50; df["htf_rsi_bear"]=htf_rsi<50
    df["htf_uptrend"]=df["htf_bull"]&df["htf_rsi_bull"]
    df["htf_downtrend"]=df["htf_bear"]&df["htf_rsi_bear"]
    df["htf_hh"]=h2>h2.shift(1).rolling(5).max()
    df["htf_ll"]=l2<l2.shift(1).rolling(5).min()
    return df.add_suffix("_1h")

df1  = add_features(df_base, "1m")
df5  = add_features(df_mid,  "5m")
df15 = add_features(df_high, "15m")
df1h = add_features_htf(df_bias)

for d,s in [(df1,"1m"),(df5,"5m"),(df15,"15m")]:
    d.rename(columns={f"datetime_{s}":"datetime"}, inplace=True)
df1h.rename(columns={"datetime_1h":"datetime"}, inplace=True)

df = pd.merge_asof(df1.sort_values("datetime"),  df5.sort_values("datetime"),  on="datetime", direction="backward")
df = pd.merge_asof(df.sort_values("datetime"),   df15.sort_values("datetime"), on="datetime", direction="backward")
df = pd.merge_asof(df.sort_values("datetime"),   df1h.sort_values("datetime"), on="datetime", direction="backward")
df = df.dropna().reset_index(drop=True)
log(f"  Birleşik: {len(df):,} satır, {len(df.columns)} kolon")

if len(df) < 80:
    log(f"  HATA: Birleşik veri yeterli değil ({len(df)} satır).")
    write_status("error", f"Birleşik veri az: {len(df)} satır", {"bars": len(df)})
    sys.exit(1)

N          = len(df)
TRAIN_END  = int(N * CFG.TRAIN_RATIO)
RECENT_START=int(N * CFG.RECENT_RATIO)

close  = df["close_1m"].values
high   = df["high_1m"].values
low    = df["low_1m"].values
open_p = df["open_1m"].values
atr    = df["atr14_1m"].values

train_atr = df["atr14_1m"].iloc[:TRAIN_END]
q70 = float(train_atr.quantile(0.70)); q30 = float(train_atr.quantile(0.30))
df["high_vol_1m"] = (df["atr14_1m"]>q70).astype(bool)
df["low_vol_1m"]  = (df["atr14_1m"]<q30).astype(bool)

# ================================================================
# 3. BACKTEST CORE — Crypto maliyet modeli
# ================================================================
def get_trade_cost_crypto(entry_price, lot=1.0):
    """Crypto round-trip maliyet: taker commission + slippage."""
    comm     = entry_price * lot * CFG.COMMISSION_PCT * 2
    slippage = entry_price * lot * CFG.SLIPPAGE_PCT   * 2
    return comm + slippage

def resolve_outcome(low_j, high_j, sl, tp, direction):
    sl_hit = (low_j<=sl)  if direction=="long"  else (high_j>=sl)
    tp_hit = (high_j>=tp) if direction=="long"  else (low_j<=tp)
    if sl_hit and tp_hit:
        return 1 if np.random.random()<0.5 else -1
    elif sl_hit: return -1
    elif tp_hit: return 1
    return None

def run_backtest(mask_arr, direction, tp_mult, sl_mult, start_idx):
    idx = np.where(mask_arr[start_idx:])[0]+start_idx
    wins=0; losses=0; gp=0.0; gl=0.0
    equity=CFG.STARTING_BAL; peak=equity; max_dd=0.0; eq_curve=[]; pos_close=-1
    for i in idx:
        if i+CFG.TIMEOUT_BARS>=N: continue
        if i<=pos_close: continue
        if i+1>=N: continue
        a=atr[i]
        if a==0 or np.isnan(a): continue
        entry=open_p[i+1]
        if entry==0 or np.isnan(entry): continue
        risk_usd = equity * CFG.RISK_PER_TRADE
        lot = risk_usd / (sl_mult * a + 1e-9)
        if direction=="long": tp=entry+tp_mult*a; sl=entry-sl_mult*a
        else:                 tp=entry-tp_mult*a; sl=entry+sl_mult*a
        outcome=None
        for j in range(i+2, min(i+CFG.TIMEOUT_BARS+1,N)):
            o=resolve_outcome(low[j],high[j],sl,tp,direction)
            if o is not None: outcome=o; pos_close=j; break
        if outcome is None: outcome=-1; pos_close=i+CFG.TIMEOUT_BARS
        tc = get_trade_cost_crypto(entry, lot)
        if outcome==1:
            pnl=lot*(tp_mult*a)-tc; wins+=1; gp+=tp_mult*a
        else:
            pnl=-lot*(sl_mult*a)-tc; losses+=1; gl+=sl_mult*a
        equity+=pnl
        if equity<=0: equity=0; break
        if equity>peak: peak=equity
        dd=(peak-equity)/peak
        if dd>max_dd: max_dd=dd
        eq_curve.append(equity)
    total=wins+losses
    if total<1: return None
    return {"total":total,"wins":wins,"losses":losses,"wr":wins/total,
            "pf":gp/(gl+1e-9),"max_dd":max_dd,"final_equity":equity,"eq_curve":eq_curve}

def run_backtest_range(mask_arr, direction, tp_mult, sl_mult, s, e):
    idx=np.where(mask_arr[s:e])[0]+s
    wins=0; losses=0; gp=0.0; gl=0.0
    equity=CFG.STARTING_BAL; peak=equity; max_dd=0.0; pos_close=-1
    for i in idx:
        if i+CFG.TIMEOUT_BARS>=e: continue
        if i<=pos_close: continue
        if i+1>=e: continue
        a=atr[i]
        if a==0 or np.isnan(a): continue
        entry=open_p[i+1]
        if entry==0 or np.isnan(entry): continue
        risk_usd=equity*CFG.RISK_PER_TRADE; lot=risk_usd/(sl_mult*a+1e-9)
        if direction=="long": tp=entry+tp_mult*a; sl=entry-sl_mult*a
        else:                 tp=entry-tp_mult*a; sl=entry+sl_mult*a
        outcome=None
        for j in range(i+2, min(i+CFG.TIMEOUT_BARS+1,e)):
            o=resolve_outcome(low[j],high[j],sl,tp,direction)
            if o is not None: outcome=o; pos_close=j; break
        if outcome is None: outcome=-1; pos_close=i+CFG.TIMEOUT_BARS
        tc=get_trade_cost_crypto(entry,lot)
        pnl=(lot*(tp_mult*a)-tc) if outcome==1 else (-lot*(sl_mult*a)-tc)
        if outcome==1: wins+=1; gp+=tp_mult*a
        else:          losses+=1; gl+=sl_mult*a
        equity+=pnl
        if equity<=0: equity=0; break
        if equity>peak: peak=equity
        dd=(peak-equity)/peak
        if dd>max_dd: max_dd=dd
    total=wins+losses
    if total<1: return None
    return {"total":total,"wr":wins/total,"pf":gp/(gl+1e-9),"max_dd":max_dd,"final_equity":equity}

def ulcer_index(eq_curve):
    if len(eq_curve)<5: return 0.0
    eq=np.array(eq_curve); peak=np.maximum.accumulate(eq)
    dd=(peak-eq)/(peak+1e-9)
    return float(np.sqrt(np.mean(dd**2)))

# ================================================================
# 4. KOMBİNASYON TANIMLA & TARA
# ================================================================
avail = set(df.columns)

LONG_ICT = {k:v for k,v in {
    "fvg_bull":"fvg_bull_1m","ob_bull":"ob_bull_1m","liq_bull":"liq_bull_1m",
    "bos_bull":"bos_bull_1m","bull_eng":"bull_eng_1m","hammer":"hammer_1m",
    "at_bb_lower":"at_bb_lower_1m","consec_dn":"consec_dn_1m",
    "choch_bull":"choch_bull_1m","choch_bull_strong":"choch_bull_strong_1m",
    "ob_bull_strong":"ob_bull_strong_1m","unicorn_bull":"unicorn_bull_1m",
    "unicorn_bull_strong":"unicorn_bull_strong_1m","wyckoff_spring":"wyckoff_spring_1m",
    "breaker_bull":"breaker_bull_1m","ote_bull":"ote_bull_1m",
    "tweezer_bot":"tweezer_bot_1m","three_soldiers":"three_soldiers_1m",
    "piercing":"piercing_1m","bull_harami":"bull_harami_1m",
    "bull_marubozu":"bull_marubozu_1m","at_kc_lower":"at_kc_lower_1m",
    "psar_flip_bull":"real_psar_flip_bull_1m","in_golden_zone_bull":"in_golden_zone_bull_1m",
    "hh_trend":"hh_trend_1m","rsi_div_bull":"rsi_div_bull_1m",
    "bb_breakout_up":"bb_breakout_up_1m","vol_confirm_bull":"vol_confirm_bull_1m",
}.items() if v in avail}

SHORT_ICT = {k:v for k,v in {
    "fvg_bear":"fvg_bear_1m","ob_bear":"ob_bear_1m","liq_bear":"liq_bear_1m",
    "bos_bear":"bos_bear_1m","bear_eng":"bear_eng_1m","inv_hammer":"inv_hammer_1m",
    "at_bb_upper":"at_bb_upper_1m","consec_up":"consec_up_1m",
    "choch_bear":"choch_bear_1m","choch_bear_strong":"choch_bear_strong_1m",
    "ob_bear_strong":"ob_bear_strong_1m","unicorn_bear":"unicorn_bear_1m",
    "unicorn_bear_strong":"unicorn_bear_strong_1m","wyckoff_upthrust":"wyckoff_upthrust_1m",
    "breaker_bear":"breaker_bear_1m","ote_bear":"ote_bear_1m",
    "tweezer_top":"tweezer_top_1m","three_crows":"three_crows_1m",
    "dark_cloud":"dark_cloud_1m","bear_harami":"bear_harami_1m",
    "bear_marubozu":"bear_marubozu_1m","at_kc_upper":"at_kc_upper_1m",
    "psar_flip_bear":"real_psar_flip_bear_1m","in_golden_zone_bear":"in_golden_zone_bear_1m",
    "ll_trend":"ll_trend_1m","rsi_div_bear":"rsi_div_bear_1m",
    "bb_breakout_dn":"bb_breakout_dn_1m","vol_confirm_bear":"vol_confirm_bear_1m",
}.items() if v in avail}

LONG_FILT = {k:v for k,v in {
    "rsi_os":"rsi_os_1m","rsi_os2":"rsi_os2_1m","stoch_os":"stoch_os_1m",
    "macd_up":"macd_up_1m","willr_os":"willr_os_1m","cci_os":"cci_os_1m",
    "rsi_rising":"rsi_rising_1m","rsi_div_bull":"rsi_div_bull_1m",
    "ema_bull_5m":"ema_bull_5m","ema_bull_15m":"ema_bull_15m",
    "above_vwap_5m":"above_vwap_5m","trend_up_15m":"trend_up_15m",
    "adx_strong":"adx_strong_1m","vol_expand":"vol_expand_1m",
    "above_dc_mid_5m":"above_dc_mid_5m","ema21_slope_up_5m":"ema21_slope_up_5m",
    "adx_strong_15m":"adx_strong_15m","ichi_bull":"ichi_bull_1m",
    "real_psar_bull":"real_psar_bull_1m","ao_pos":"ao_pos_1m",
    "roc_pos":"roc_pos_1m","obv_up":"obv_up_1m","buy_pressure":"buy_pressure_1m",
    "hh_trend":"hh_trend_1m","strong_bull_align":"strong_bull_align_1m",
    "high_vol":"high_vol_1m","vol_spike":"vol_spike_1m","mom3_up":"mom3_up_1m",
    "htf_uptrend":"htf_uptrend_1h","htf_bull_1h":"htf_bull_1h",
    "quality_candle":"quality_candle_1m","no_doji":"no_doji_1m",
    "atr_pct80":"atr_pct80_1m","in_golden_zone_bull":"in_golden_zone_bull_1m",
}.items() if v in avail}

SHORT_FILT = {k:v for k,v in {
    "rsi_ob":"rsi_ob_1m","rsi_ob2":"rsi_ob2_1m","stoch_ob":"stoch_ob_1m",
    "macd_dn":"macd_dn_1m","willr_ob":"willr_ob_1m","cci_ob":"cci_ob_1m",
    "rsi_falling":"rsi_falling_1m","rsi_div_bear":"rsi_div_bear_1m",
    "ema_bear_5m":"ema_bear_5m","ema_bear_15m":"ema_bear_15m",
    "below_vwap_5m":"below_vwap_5m","trend_dn_15m":"trend_dn_15m",
    "adx_strong":"adx_strong_1m","vol_expand":"vol_expand_1m",
    "below_dc_mid_5m":"below_dc_mid_5m","ema21_slope_dn_5m":"ema21_slope_dn_5m",
    "adx_strong_15m":"adx_strong_15m","ichi_bear":"ichi_bear_1m",
    "real_psar_bear":"real_psar_bear_1m","ao_neg":"ao_neg_1m",
    "roc_neg":"roc_neg_1m","obv_dn":"obv_dn_1m","sell_pressure":"sell_pressure_1m",
    "ll_trend":"ll_trend_1m","strong_bear_align":"strong_bear_align_1m",
    "high_vol":"high_vol_1m","vol_spike":"vol_spike_1m","mom3_dn":"mom3_dn_1m",
    "htf_downtrend":"htf_downtrend_1h","htf_bear_1h":"htf_bear_1h",
    "quality_candle":"quality_candle_1m","no_doji":"no_doji_1m",
    "atr_pct80":"atr_pct80_1m","in_golden_zone_bear":"in_golden_zone_bear_1m",
}.items() if v in avail}

def build_combos(ict, filt):
    combos=[]
    for ik,iv in ict.items():
        for fk,fv in filt.items():
            combos.append({"keys":[ik,fk],"cols":[iv,fv]})
    mtf={k:v for k,v in filt.items() if "_5m" in v or "_15m" in v}
    onem={k:v for k,v in filt.items() if "_1m" in v}
    for ik,iv in ict.items():
        for mk,mv in mtf.items():
            for fk,fv in onem.items():
                if fk!=mk:
                    combos.append({"keys":[ik,mk,fk],"cols":[iv,mv,fv]})
    return combos

long_combos  = build_combos(LONG_ICT, LONG_FILT)
short_combos = build_combos(SHORT_ICT, SHORT_FILT)
total_combos = len(long_combos)+len(short_combos)

log(f"\n[3/5] Strateji taraması başlıyor...")
log(f"  Kombinasyon: {total_combos:,} | TP/SL grid: {len(CFG.TP_SL_GRID)} | Toplam test: {total_combos*len(CFG.TP_SL_GRID):,}")
log(f"  Filtreler: min {CFG.MIN_TRADES} trade | PF>{CFG.MIN_PF} | WR>{CFG.MIN_WR} | MaxDD<{CFG.MAX_DD:.0%}")
write_status("scanning", f"Kombinasyon taraması ({total_combos:,})", {"total_combos": total_combos})

all_results=[]; tested=0

def evaluate(combo, direction):
    global tested
    cols=combo["cols"]; keys=combo["keys"]
    mask=np.ones(N,dtype=bool)
    for col in cols:
        mask &= df[col].values.astype(bool)
    if mask[TRAIN_END:].sum() < CFG.MIN_TRADES//2:
        tested+=1; return
    best=None
    for tp_mult,sl_mult in CFG.TP_SL_GRID:
        r=run_backtest(mask,direction,tp_mult,sl_mult,TRAIN_END)
        if r is None or r["total"]<CFG.MIN_TRADES: continue
        if r["pf"]<CFG.MIN_PF or r["wr"]<CFG.MIN_WR or r["max_dd"]>CFG.MAX_DD: continue
        rr=run_backtest(mask,direction,tp_mult,sl_mult,RECENT_START)
        rec_wr=rr["wr"] if rr else 0; rec_pf=rr["pf"] if rr else 0
        rec_dd=rr["max_dd"] if rr else 1; rec_n=rr["total"] if rr else 0
        if rec_n<CFG.MIN_RECENT_TRADES: continue
        if rec_pf<CFG.MIN_PF*0.82 or rec_wr<CFG.MIN_WR*0.88 or rec_dd>CFG.MAX_DD*1.2: continue
        wr_decay=(r["wr"]-rec_wr)/(r["wr"]+1e-9); pf_decay=(r["pf"]-rec_pf)/(r["pf"]+1e-9)
        if wr_decay>CFG.MAX_WR_DECAY or pf_decay>CFG.MAX_PF_DECAY: continue
        eq_curve=r.get("eq_curve",[])
        if len(eq_curve)>10:
            eq_arr=np.array(eq_curve); rets=np.diff(eq_arr)/(eq_arr[:-1]+1e-9)
            if (rets.mean()/(rets.std()+1e-9))<0.3: continue
        total_ret=(r["final_equity"]-CFG.STARTING_BAL)/CFG.STARTING_BAL
        calmar=total_ret/(r["max_dd"]+1e-9)
        if calmar<0.8: continue
        pf_s=min(1.0,rec_pf/(r["pf"]+1e-9)); wr_s=min(1.0,rec_wr/(r["wr"]+1e-9))
        decay_p=max(0.0,1.0-wr_decay*2-pf_decay)
        ui=ulcer_index(eq_curve); ui_f=1.0/(1.0+ui*5)
        score=(r["pf"]*r["wr"]*(1-r["max_dd"])*pf_s*wr_s*decay_p*ui_f*min(2.0,calmar/3))
        if best is None or score>best["score"]:
            best={"name":direction.upper()+"_"+"_".join(keys),"direction":direction,
                  "conditions":cols,"keys":keys,"tp_mult":tp_mult,"sl_mult":sl_mult,
                  "rr":round(tp_mult/sl_mult,2),
                  "total_trades":r["total"],"win_rate":round(r["wr"],4),
                  "profit_factor":round(r["pf"],4),"max_dd":round(r["max_dd"],4),
                  "final_equity":round(r["final_equity"],2),
                  "rec_trades":rec_n,"rec_wr":round(rec_wr,4),
                  "rec_pf":round(rec_pf,4),"rec_dd":round(rec_dd,4),
                  "score":round(score,6),"eq_curve":eq_curve}
    if best: all_results.append(best)
    tested+=1
    if tested%500==0:
        log(f"  {tested:>6}/{total_combos} | Bulunan: {len(all_results)}")
        write_status("scanning",f"{tested}/{total_combos} test | {len(all_results)} aday",
                     {"tested": tested, "found": len(all_results)})

for combo in long_combos:  evaluate(combo,"long")
for combo in short_combos: evaluate(combo,"short")
log(f"  Toplam: {tested:,} test | Uygun: {len(all_results)}")

# ================================================================
# 5. WALK-FORWARD
# ================================================================
log(f"\n[4/5] Walk-forward doğrulaması...")
all_results.sort(key=lambda x:x["score"],reverse=True)
top_candidates=all_results[:CFG.TOP_N*3]

WF_WINDOWS = [
    (int(N*0.50),int(N*0.65)),
    (int(N*0.65),int(N*0.80)),
    (int(N*0.80),int(N*0.95)),
]
for r in top_candidates:
    mask=np.ones(N,dtype=bool)
    for col in r["conditions"]: mask &= df[col].values.astype(bool)
    wf_pass=0; wf_details=[]
    for (ws,we) in WF_WINDOWS:
        rw=run_backtest_range(mask,r["direction"],r["tp_mult"],r["sl_mult"],ws,we)
        ok=rw and rw["total"]>=10 and rw["pf"]>=CFG.MIN_PF and rw["wr"]>=0.40
        if ok: wf_pass+=1
        wf_details.append({"total":rw["total"] if rw else 0,
                            "wr":round(rw["wr"],3) if rw else 0,
                            "pf":round(rw["pf"],2) if rw else 0,
                            "max_dd":round(rw["max_dd"],3) if rw else 1})
    r["wf_pass"]=wf_pass; r["wf_details"]=wf_details

top_results=[r for r in top_candidates if r.get("wf_pass",0)>=2]
top_results.sort(key=lambda x:x["score"],reverse=True)
top_results=top_results[:CFG.TOP_N]
log(f"  WF sonrası: {len(top_results)} strateji")
if not top_results:
    log("  ⚠ WF katı, tüm adaylar alındı")
    top_results=all_results[:CFG.TOP_N]
    for r in top_results: r["wf_pass"]=0; r["wf_details"]=[]

# ================================================================
# 6. MONTE CARLO + PERMUTATION TEST
# ================================================================
log(f"\n[5/5] Monte Carlo ({CFG.N_SIM:,} simülasyon)...")

def monte_carlo(pnl_arr):
    arr=np.array(pnl_arr); log_arr=np.log1p(arr)
    sim_size=min(len(arr),300)
    final_ret=np.zeros(CFG.N_SIM); max_dds=np.zeros(CFG.N_SIM)
    for s in range(CFG.N_SIM):
        shuf=np.random.choice(log_arr,size=sim_size,replace=True)
        log_curve=np.cumsum(shuf); eq=CFG.STARTING_BAL*np.exp(log_curve)
        eq=np.minimum(eq,CFG.STARTING_BAL*200)
        final_ret[s]=(eq[-1]-CFG.STARTING_BAL)/CFG.STARTING_BAL
        pk=np.maximum.accumulate(eq); max_dds[s]=((pk-eq)/pk).max()
    return {"prob_profit":float((final_ret>0).mean()),
            "med_return":float(np.median(final_ret)),"p5_return":float(np.percentile(final_ret,5)),
            "p95_return":float(np.percentile(final_ret,95)),
            "med_maxdd":float(np.median(max_dds)),"p95_maxdd":float(np.percentile(max_dds,95)),
            "ruin_pct":float((final_ret<-0.50).mean())}

def permutation_test(trades_arr, tp_mult, sl_mult, n_perm=3000):
    n=len(trades_arr); wins=(trades_arr==1).sum()
    real_pf=(wins*tp_mult)/((n-wins)*sl_mult+1e-9)
    rand_pfs=np.zeros(n_perm)
    for i in range(n_perm):
        rw=np.random.binomial(n,0.5)
        rand_pfs[i]=(rw*tp_mult)/((n-rw)*sl_mult+1e-9)
    return float((rand_pfs>=real_pf).mean())

def get_pnl_arr(mask_arr, direction, tp_mult, sl_mult, start_idx):
    idx=np.where(mask_arr[start_idx:])[0]+start_idx
    trades_out=[]; pnl_out=[]; pos_close=-1
    for i in idx:
        if i+CFG.TIMEOUT_BARS>=N: continue
        if i<=pos_close: continue
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
                if low[j]<=sl:  outcome=-1; pos_close=j; break
                if high[j]>=tp: outcome=1;  pos_close=j; break
            else:
                if high[j]>=sl: outcome=-1; pos_close=j; break
                if low[j]<=tp:  outcome=1;  pos_close=j; break
        if outcome is None: outcome=-1; pos_close=i+CFG.TIMEOUT_BARS
        trades_out.append(outcome)
        pnl_out.append(tp_mult*CFG.RISK_PER_TRADE if outcome==1 else -sl_mult*CFG.RISK_PER_TRADE)
    return np.array(trades_out), np.array(pnl_out)

n_total_tests = total_combos * len(CFG.TP_SL_GRID)
bonferroni_p  = CFG.BONFERRONI_ALPHA / max(1, n_total_tests)

for r in top_results:
    mask=np.ones(N,dtype=bool)
    for col in r["conditions"]: mask &= df[col].values.astype(bool)
    trades_arr, pnl_arr = get_pnl_arr(mask, r["direction"], r["tp_mult"], r["sl_mult"], TRAIN_END)
    if len(pnl_arr)<20:
        r["mc"]=None; r["p_value"]=1.0; r["bonferroni_pass"]=False
        r["edge_label"]="YETERSIZ_VERI"; continue
    mc = monte_carlo(pnl_arr)
    pval = permutation_test(trades_arr, r["tp_mult"], r["sl_mult"])
    r["mc"]=mc; r["p_value"]=round(pval,5); r["bonferroni_p"]=round(bonferroni_p,8)
    r["bonferroni_pass"]=pval<bonferroni_p
    if pval<0.01:   r["edge_label"]="GUCLU_EDGE"
    elif pval<0.05: r["edge_label"]="EDGE_VAR"
    elif pval<0.10: r["edge_label"]="ZAYIF_EDGE"
    else:           r["edge_label"]="EDGE_YOK"

# ── Son barda aktif sinyaller ──────────────────────────────────
last_bar=N-1
current_signals=[]
for r in top_results:
    if r.get("p_value",1.0)>=0.10: continue
    mask=np.ones(N,dtype=bool)
    for col in r["conditions"]:
        if col in df.columns: mask &= df[col].values.astype(bool)
    if mask[last_bar]:
        current_signals.append({
            "strategy": r["name"], "direction": r["direction"],
            "tp_mult": r["tp_mult"], "sl_mult": r["sl_mult"],
            "bar_time": str(df["datetime"].iloc[last_bar]),
            "atr": round(float(atr[last_bar]),6),
            "p_value": r.get("p_value",1.0),
        })

log(f"\n  Son barda {len(current_signals)} aktif sinyal")
if current_signals:
    for s in current_signals:
        log(f"    {s['strategy'][:50]} → {s['direction'].upper()}")

# ================================================================
# 7. SONUÇLARI KAYDET
# ================================================================
def clean(obj):
    if isinstance(obj,dict): return {k:clean(v) for k,v in obj.items() if k!="eq_curve"}
    if isinstance(obj,list): return [clean(v) for v in obj]
    if isinstance(obj,(np.integer,)): return int(obj)
    if isinstance(obj,(np.floating,)): return float(obj)
    if isinstance(obj,(np.bool_,)): return bool(obj)
    return obj

valid_strats = [r for r in top_results if r.get("p_value",1)<0.10]
best_strategy = valid_strats[0] if valid_strats else (top_results[0] if top_results else None)

wf_consistency = 0.0
if top_results:
    wf_passes=[r.get("wf_pass",0) for r in top_results]
    wf_consistency=round(sum(1 for p in wf_passes if p>=2)/len(wf_passes),3)

ruin_pct = 0.0
if best_strategy and best_strategy.get("mc"):
    ruin_pct = best_strategy["mc"].get("ruin_pct",0.0)

mc_prob_profit = 0.0
if best_strategy and best_strategy.get("mc"):
    mc_prob_profit = best_strategy["mc"].get("prob_profit",0.0)

out = {
    "timestamp": datetime.now().isoformat(),
    "version": "p5_crypto_v1",
    "symbol": args.symbol, "interval": args.interval,
    "market": args.market, "exchange": args.exchange,
    "bars_total": N, "bars_train": TRAIN_END,
    "config": {"min_trades": CFG.MIN_TRADES, "min_pf": CFG.MIN_PF,
               "min_wr": CFG.MIN_WR, "max_dd": CFG.MAX_DD,
               "commission_pct": CFG.COMMISSION_PCT, "slippage_pct": CFG.SLIPPAGE_PCT,
               "n_sim": CFG.N_SIM, "risk_per_trade": CFG.RISK_PER_TRADE},
    "summary": {
        "total_combos_tested": n_total_tests,
        "candidates_found": len(all_results),
        "wf_passed": len(top_results),
        "edge_confirmed": len(valid_strats),
        "wf_consistency": wf_consistency,
        "ruin_pct": ruin_pct,
        "mc_prob_profit": mc_prob_profit,
        "active_signals_now": len(current_signals),
        "best_name": best_strategy["name"] if best_strategy else "",
        "best_wr": best_strategy["win_rate"] if best_strategy else 0,
        "best_pf": best_strategy["profit_factor"] if best_strategy else 0,
        "best_dd": best_strategy["max_dd"] if best_strategy else 0,
        "best_edge": best_strategy.get("edge_label","") if best_strategy else "",
        "best_p_value": best_strategy.get("p_value",1.0) if best_strategy else 1.0,
    },
    "top_strategies": clean(top_results),
    "current_signals": current_signals,
}

out_path = os.path.join(OUT_DIR, f"{args.symbol}_{args.interval}_{args.exchange}_{args.market}_results.json")
with open(out_path,"w",encoding="utf-8") as f:
    json.dump(out, f, ensure_ascii=False, indent=2)

sig_path = os.path.join(OUT_DIR, "current_signals.json")
with open(sig_path,"w",encoding="utf-8") as f:
    json.dump({"timestamp":datetime.now().isoformat(),"symbol":args.symbol,
               "interval":args.interval,"signals":current_signals},f,ensure_ascii=False,indent=2)

log(f"\n  [✓] {out_path}")
log(f"  [✓] {sig_path}")
if best_strategy:
    log(f"\n  ★ EN İYİ: {best_strategy['name']}")
    log(f"    WR={best_strategy['win_rate']:.1%}  PF={best_strategy['profit_factor']:.2f}  DD={best_strategy['max_dd']:.1%}")
    log(f"    Edge={best_strategy.get('edge_label','?')}  WF={best_strategy.get('wf_pass',0)}/3")

write_status("done", f"Tamamlandı — {len(valid_strats)} strateji", {
    "strategies_found": len(valid_strats),
    "best_name": best_strategy["name"] if best_strategy else "",
    "best_wr": best_strategy["win_rate"] if best_strategy else 0,
    "best_pf": best_strategy["profit_factor"] if best_strategy else 0,
    "mc_prob_profit": mc_prob_profit,
    "ruin_pct": ruin_pct,
    "active_signals": len(current_signals),
    "out_path": out_path,
})
log(f"\n{'='*60}")
log(f"  Bitti. Süre: {datetime.now().strftime('%H:%M:%S')}")
_lf.close()
