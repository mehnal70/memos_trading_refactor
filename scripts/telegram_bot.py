#!/usr/bin/env python3
# scripts/telegram_bot.py — Telegram SALT-OKUMA sorgu botu.
#
# getUpdates ile komut dinler, data/snapshot.json'dan (motorun her 1s yazdığı canlı durum) yanıt kurar.
# Motordan BAĞIMSIZ (push'u motor yapar; bu yalnız gelen komutlara cevap verir → tek getUpdates tüketicisi).
# YALNIZ yetkili TELEGRAM_CHAT_ID'ye cevap verir. Hiçbir KONTROL yok (durdur/başlat/reset) → sadece sorgu.
#
# Komutlar: /status /pos /pnl /trades /report /help
# Kullanım: python3 scripts/telegram_bot.py   (token+chat .launch.conf'tan)
import json, os, sys, time, urllib.request, urllib.parse

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# Profil sistemi: sırlar (token/chat) .env'de, aktif profil ayarları (SNAPSHOT_PATH) .launch.conf'ta.
# İkisini de oku; .env sonra → token/chat .env'den (override). [[project_profiles]]
CONF_FILES = [os.path.join(REPO, "scripts", ".launch.conf"), os.path.join(REPO, ".env")]

def load_conf():
    d = {}
    for path in CONF_FILES:
        try:
            for line in open(path, encoding="utf-8"):
                line = line.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                k, v = line.split("=", 1)
                v = v.strip()
                if v:  # boş değer öncekini EZMESİN (.env'de boş key varsa launch.conf'taki kalsın)
                    d[k.strip()] = v
        except FileNotFoundError:
            pass
    return d

CONFV   = load_conf()
TOKEN   = CONFV.get("TELEGRAM_BOT_TOKEN", "")
CHAT_ID = CONFV.get("TELEGRAM_CHAT_ID", "")
# Aktif profilin snapshot'ı (SNAPSHOT_PATH, .launch.conf'ta use ile yazılır); yoksa eski default.
_snap = CONFV.get("SNAPSHOT_PATH", "data/snapshot.json")
SNAP = _snap if os.path.isabs(_snap) else os.path.join(REPO, _snap)
API     = f"https://api.telegram.org/bot{TOKEN}"

def api(method, params=None, timeout=35):
    data = urllib.parse.urlencode(params or {}).encode()
    req = urllib.request.Request(f"{API}/{method}", data=data)
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)

def send(text):
    try:
        api("sendMessage", {"chat_id": CHAT_ID, "text": text, "parse_mode": "HTML"}, timeout=15)
    except Exception as e:
        print("send hata:", e, file=sys.stderr)

def snap():
    return json.load(open(SNAP, encoding="utf-8"))

def fmt(n, d=2):
    try: return f"{float(n):,.{d}f}"
    except Exception: return "—"

def snap_age():
    try: return int(time.time() - os.path.getmtime(SNAP))
    except Exception: return 99999

# ── Komut yanıtları (snapshot.json'dan) ─────────────────────────────────────
def cmd_status(d):
    f = d.get("finance", {}); eq = f.get("total_equity", 0); st = f.get("starting_capital", 10000) or 10000
    ret = (eq - st) / st * 100
    anom = d.get("anomalies", []); anom = len(anom) if isinstance(anom, list) else anom
    age = snap_age(); fresh = "🟢 canlı" if age <= 30 else f"🟡 {age}s bayat (motor?)"
    return (f"<b>📊 Durum</b>  {fresh}\n"
            f"Equity: <b>${fmt(eq)}</b> ({ret:+.2f}%)\n"
            f"Açık: {len(d.get('positions', []))} pozisyon · açık P&L ${fmt(f.get('open_pnl',0))}\n"
            f"Faz: {d.get('phase','—')} · Anomali: {anom}")

def cmd_pos(d):
    P = d.get("positions", [])
    if not P: return "📂 Açık pozisyon yok."
    out = [f"<b>📂 Açık pozisyonlar ({len(P)})</b>"]
    for p in P:
        e, c, lev = p.get("entry_price",0), p.get("current_price",0), p.get("leverage",1) or 1
        long = p.get("is_long"); up = ((c-e)/e*100)*(1 if long else -1)*lev if e else 0
        mark = "🟢" if up >= 0 else "🔴"
        out.append(f"{mark} <b>{p.get('symbol')}</b> {'LONG' if long else 'SHORT'} {up:+.2f}% "
                   f"· {p.get('trade_type','')} {p.get('interval','')} "
                   f"· SL {fmt(p.get('stop_loss'),4)} TP {fmt(p.get('take_profit'),4)}")
    return "\n".join(out)

def cmd_pnl(d):
    f = d.get("finance", {})
    return (f"<b>💰 P&amp;L</b>\n"
            f"Realize: ${fmt(f.get('realize_pnl',0))}\n"
            f"Açık (unrealized): ${fmt(f.get('open_pnl',0))}\n"
            f"Komisyon: ${fmt(f.get('total_fees',0))}\n"
            f"Equity: <b>${fmt(f.get('total_equity',0))}</b>")

def cmd_trades(d):
    T = d.get("trade_history", [])[-10:][::-1]
    if not T: return "📜 Henüz kapanan işlem yok."
    out = ["<b>📜 Son kapananlar</b>"]
    for t in T:
        net = t.get("net_pnl", t.get("pnl", 0)); mark = "🟢" if net >= 0 else "🔴"
        ep, xp = t.get("entry_price", 0), t.get("exit_price", 0)
        # Giriş→çıkış fiyatı (eski kayıtlarda 0 → atla, yanıltıcı sıfır gösterme).
        px = f" · {fmt(ep,4)}→{fmt(xp,4)}" if (ep and xp) else ""
        out.append(f"{mark} {t.get('symbol')} {'LONG' if t.get('is_long') else 'SHORT'}{px} "
                   f"net ${fmt(net)} · {str(t.get('exit_reason','')).replace('_',' ')}")
    return "\n".join(out)

def cmd_report(d):
    return cmd_status(d) + "\n\n" + cmd_pnl(d)

HELP = ("<b>🤖 Memos RTC — komutlar</b>\n"
        "/status — equity, açık, anomali\n"
        "/pos — açık pozisyonlar + uPnL\n"
        "/pnl — realize/açık P&amp;L + komisyon\n"
        "/trades — son kapananlar (net P&amp;L)\n"
        "/report — durum + P&amp;L özeti\n"
        "/help — bu liste\n"
        "<i>(salt-okuma; kontrol komutu yok)</i>")

HANDLERS = {"status": cmd_status, "pos": cmd_pos, "positions": cmd_pos,
            "pnl": cmd_pnl, "trades": cmd_trades, "report": cmd_report}

def handle(text):
    # Çıplak "/" veya "/@bot" gibi komutsuz girdi → boş parça; sessiz yoksay (çökme yerine).
    parts = text.lstrip("/").split("@")[0].split()
    if not parts:
        return None
    cmd = parts[0].lower()
    if cmd == "help" or cmd == "start":
        return HELP
    h = HANDLERS.get(cmd)
    if not h: return None  # bilinmeyen → sessiz
    try:
        return h(snap())
    except FileNotFoundError:
        return "⚠️ snapshot yok — motor çalışıyor mu?"
    except Exception as e:
        return f"⚠️ hata: {e}"

def main():
    if not TOKEN or not CHAT_ID:
        print("token/chat_id boş (.launch.conf) → bot kapalı", file=sys.stderr); sys.exit(1)
    # Telegram menüsüne komutları kaydet (kullanıcı '/' yazınca liste çıkar)
    try:
        cmds = [{"command": c, "description": d} for c, d in
                [("status","Durum: equity/açık/anomali"),("pos","Açık pozisyonlar"),
                 ("pnl","P&L özeti"),("trades","Son kapananlar"),("report","Durum+P&L"),("help","Komut listesi")]]
        api("setMyCommands", {"commands": json.dumps(cmds)}, timeout=15)
    except Exception as e:
        print("setMyCommands hata:", e, file=sys.stderr)
    # Boot'ta birikmiş eski komutları atla: en güncel update_id'nin ötesine geç.
    offset = 0
    try:
        u = api("getUpdates", {"timeout": 0}, timeout=15).get("result", [])
        if u: offset = u[-1]["update_id"] + 1
    except Exception: pass
    print(f"🤖 Telegram bot dinliyor (chat={CHAT_ID})")
    while True:
        try:
            res = api("getUpdates", {"offset": offset, "timeout": 30}, timeout=35).get("result", [])
        except Exception as e:
            print("getUpdates hata:", e, file=sys.stderr); time.sleep(5); continue
        for upd in res:
            offset = upd["update_id"] + 1
            msg = upd.get("message") or upd.get("edited_message") or {}
            chat = str((msg.get("chat") or {}).get("id", ""))
            text = (msg.get("text") or "").strip()
            if chat != str(CHAT_ID):  # YETKİSİZ → sessiz yoksay
                continue
            if not text.startswith("/"):
                continue
            reply = handle(text)
            if reply:
                send(reply)

if __name__ == "__main__":
    main()
