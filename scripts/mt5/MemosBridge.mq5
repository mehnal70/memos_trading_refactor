//+------------------------------------------------------------------+
//|                                                 MemosBridge.mq5   |
//|        Memos trading-core <-> MetaTrader 5 köprü Expert Advisor   |
//+------------------------------------------------------------------+
//| Rust tarafı TCP SERVER'dır (MQL5'te server socket yok); bu EA     |
//| native CLIENT olarak Rust köprüsüne BAĞLANIR ve döngüde:          |
//|   bir istek satırı OKU (NDJSON) -> işle -> bir yanıt satırı YAZ.  |
//|                                                                   |
//| Protokol (bkz. memos_trading_core/src/robot/venue/mt5/protocol):  |
//|   istek : {"id":N,"cmd":"candles","symbol":"EURUSD","tf":"1h",    |
//|            "limit":200}                                            |
//|   yanıt : {"id":N,"ok":true,"candles":[[ts_ms,o,h,l,c,v],...]}    |
//|           ya da {"id":N,"ok":false,"error":"..."}                 |
//|                                                                   |
//| KURULUM:                                                          |
//|   1) Bu dosyayı MT5'in MQL5/Experts klasörüne kopyala, derle.     |
//|   2) Tools > Options > Expert Advisors:                           |
//|        "Allow algorithmic trading" + "Allow DLL imports" GEREKMEZ |
//|        (saf MQL5 soket). Soket adresi için "Allow WebRequest /     |
//|        socket" listesine 127.0.0.1 EKLE (build >=1930 soket izni).|
//|   3) Önce Rust köprüsünü başlat (dinlemeye geçsin), sonra EA'yı   |
//|      bir grafiğe ekle. EA bağlanır ve istekleri yanıtlar.         |
//+------------------------------------------------------------------+
#property copyright "Memos"
#property version   "1.10"
#property strict

#include <Trade/Trade.mqh>              // Faz 2: CTrade market/limit OrderSend

input string InpHost      = "127.0.0.1"; // Rust köprü host
input int    InpPort      = 9001;        // Rust köprü port (MT5_BRIDGE_ADDR ile aynı)
input int    InpPollMs    = 50;          // OnTimer periyodu (ms)
input int    InpReconnectMinMs = 1000;   // Başarısız bağlantı sonrası ilk bekleme
input int    InpReconnectMaxMs = 30000;  // Yeniden bağlanma beklemesinin üst sınırı
input bool   InpEnableExec = false;      // Faz 2: emir yürütmeyi aç (varsayılan kapalı)
input long   InpMagic     = 770077;      // Faz 2: emir magic numarası (memos kimliği)
input long   InpAllowedLogin = 52926538; // Yalnızca izinli demo hesap
input string InpAllowedServer = "ICMarketsSC-Demo"; // Yalnızca izinli demo sunucu

int     g_sock = INVALID_HANDLE;         // soket handle
string  g_buf  = "";                     // satır-birleştirme tamponu
CTrade  g_trade;                         // Faz 2: yürütme yardımcısı
ulong   g_next_connect_ms = 0;           // Sıradaki bağlantı denemesinin zamanı
int     g_connect_backoff_ms = 1000;     // Başarısız bağlantıların üstel beklemesi

//+------------------------------------------------------------------+
int OnInit()
  {
   g_trade.SetExpertMagicNumber(InpMagic);
   g_trade.SetAsyncMode(false); // senkron: yanıt yazmadan önce sonucu öğren
   EventSetMillisecondTimer(InpPollMs);
   g_connect_backoff_ms = MathMax(InpReconnectMinMs, 250);
   PrintFormat("MemosBridge: %s:%d hedefine bağlanılacak (poll %dms, exec=%s)",
               InpHost, InpPort, InpPollMs, (string)InpEnableExec);
   return(INIT_SUCCEEDED);
  }

//+------------------------------------------------------------------+
void OnDeinit(const int reason)
  {
   EventKillTimer();
   CloseSock();
  }

//+------------------------------------------------------------------+
void CloseSock()
  {
   if(g_sock != INVALID_HANDLE)
     {
      SocketClose(g_sock);
      g_sock = INVALID_HANDLE;
     }
   g_buf = "";
  }

//+------------------------------------------------------------------+
//| Bağlı değilse bağlan. Başarısızsa false.                          |
//+------------------------------------------------------------------+
bool EnsureConnected()
  {
   if(g_sock != INVALID_HANDLE && SocketIsConnected(g_sock))
      return(true);
   ulong now = GetTickCount64();
   if(now < g_next_connect_ms)
      return(false);
   CloseSock();
   g_sock = SocketCreate();
   if(g_sock == INVALID_HANDLE)
     {
      Print("MemosBridge: SocketCreate başarısız (izin/ayar?)");
      g_next_connect_ms = now + (ulong)g_connect_backoff_ms;
      return(false);
     }
   if(!SocketConnect(g_sock, InpHost, InpPort, 1000))
     {
      CloseSock();
      g_next_connect_ms = now + (ulong)g_connect_backoff_ms;
      g_connect_backoff_ms = MathMin(
         MathMax(InpReconnectMinMs, g_connect_backoff_ms * 2),
         MathMax(InpReconnectMinMs, InpReconnectMaxMs)
      );
      return(false); // köprü henüz ayakta değil; artan beklemeyle tekrar dene
     }
   g_connect_backoff_ms = MathMax(InpReconnectMinMs, 250);
   g_next_connect_ms = 0;
   Print("MemosBridge: köprüye bağlanıldı");
   return(true);
  }

//+------------------------------------------------------------------+
//| Soketten okunabilen baytları tampona ekle.                        |
//+------------------------------------------------------------------+
void DrainSocket()
  {
   uint avail = SocketIsReadable(g_sock);
   while(avail > 0)
     {
      uchar buf[];
      int n = SocketRead(g_sock, buf, (int)avail, 50);
      if(n <= 0)
        {
         CloseSock();
         return;
        }
      g_buf += CharArrayToString(buf, 0, n, CP_UTF8);
      avail = SocketIsReadable(g_sock);
     }
  }

//+------------------------------------------------------------------+
void SendLine(const string line)
  {
   if(g_sock == INVALID_HANDLE)
      return;
   string out = line + "\n";
   uchar buf[];
   int len = StringToCharArray(out, buf, 0, WHOLE_ARRAY, CP_UTF8) - 1; // sondaki \0 hariç
   if(len > 0)
     {
      int sent = SocketSend(g_sock, buf, len);
      if(sent != len)
       {
        PrintFormat("MemosBridge: yanıt gönderilemedi/kısmi gönderildi (%d/%d)", sent, len);
        CloseSock();
       }
     }
  }

//+------------------------------------------------------------------+
void OnTimer()
  {
   if(!EnsureConnected())
      return;
   DrainSocket();
   // Tampondaki tam satırları işle.
   int nl;
   while((nl = StringFind(g_buf, "\n")) >= 0)
     {
      string line = StringSubstr(g_buf, 0, nl);
      g_buf = StringSubstr(g_buf, nl + 1);
      StringTrimRight(line);
      StringTrimLeft(line);
      if(StringLen(line) > 0)
         HandleRequest(line);
     }
  }

//+------------------------------------------------------------------+
//| Basit JSON çıkarıcılar (kontrollü düz protokol — tam parser yok). |
//+------------------------------------------------------------------+
string JsonStr(const string json, const string key)
  {
   string pat = "\"" + key + "\"";
   int k = StringFind(json, pat);
   if(k < 0) return("");
   int c = StringFind(json, ":", k);
   if(c < 0) return("");
   int q1 = StringFind(json, "\"", c + 1);
   if(q1 < 0) return("");
   int q2 = StringFind(json, "\"", q1 + 1);
   if(q2 < 0) return("");
   return(StringSubstr(json, q1 + 1, q2 - q1 - 1));
  }

double JsonNum(const string json, const string key)
  {
   string pat = "\"" + key + "\"";
   int k = StringFind(json, pat);
   if(k < 0) return(0.0);
   int c = StringFind(json, ":", k);
   if(c < 0) return(0.0);
   // ':' sonrası sayıyı topla.
   int i = c + 1;
   string num = "";
   int len = StringLen(json);
   while(i < len)
     {
      ushort ch = StringGetCharacter(json, i);
      if((ch >= '0' && ch <= '9') || ch == '-' || ch == '+' || ch == '.' || ch == 'e' || ch == 'E')
         num += ShortToString(ch);
      else if(StringLen(num) > 0)
         break;
      else if(ch == ' ')
        { i++; continue; }
      else
         break;
      i++;
     }
   return(StringToDouble(num));
  }

//+------------------------------------------------------------------+
// Boolean JSON alanını ayrıştırır: "key":true → true, diğer → false.
// JsonStr string değil boolean döndüren alanlar (reduce_only gibi) için.
//+------------------------------------------------------------------+
bool JsonBool(const string json, const string key)
  {
   string pat = "\"" + key + "\"";
   int k = StringFind(json, pat);
   if(k < 0) return(false);
   int c = StringFind(json, ":", k);
   if(c < 0) return(false);
   int i = c + 1;
   int len = StringLen(json);
   while(i < len && StringGetCharacter(json, i) == ' ') i++;
   return(i + 4 <= len && StringSubstr(json, i, 4) == "true");
  }

//+------------------------------------------------------------------+
ENUM_TIMEFRAMES TfFromString(const string tf)
  {
   if(tf == "1m")  return(PERIOD_M1);
   if(tf == "3m")  return(PERIOD_M3);
   if(tf == "5m")  return(PERIOD_M5);
   if(tf == "15m") return(PERIOD_M15);
   if(tf == "30m") return(PERIOD_M30);
   if(tf == "1h")  return(PERIOD_H1);
   if(tf == "2h")  return(PERIOD_H2);
   if(tf == "4h")  return(PERIOD_H4);
   if(tf == "6h")  return(PERIOD_H6);
   if(tf == "12h") return(PERIOD_H12);
   if(tf == "1d")  return(PERIOD_D1);
   if(tf == "1w")  return(PERIOD_W1);
   if(tf == "1M")  return(PERIOD_MN1);
   return(PERIOD_CURRENT);
  }

//+------------------------------------------------------------------+
void ReplyError(const long id, const string msg)
  {
   string escaped = msg;
   StringReplace(escaped, "\\", "\\\\");
   StringReplace(escaped, "\"", "\\\"");
   StringReplace(escaped, "\r", "\\r");
   StringReplace(escaped, "\n", "\\n");
   SendLine(StringFormat("{\"id\":%I64d,\"ok\":false,\"error\":\"%s\"}", id, escaped));
  }

bool AccountAllowed()
  {
   long login = AccountInfoInteger(ACCOUNT_LOGIN);
   string server = AccountInfoString(ACCOUNT_SERVER);
   if(InpAllowedLogin > 0 && login != InpAllowedLogin) return false;
   if(StringLen(InpAllowedServer) > 0 && server != InpAllowedServer) return false;
   return true;
  }

//+------------------------------------------------------------------+
void HandleRequest(const string req)
  {
   long   id  = (long)JsonNum(req, "id");
   string cmd = JsonStr(req, "cmd");

   if(cmd == "candles")      { HandleCandles(id, req);    return; }
   if(cmd == "tick")         { HandleTick(id, req);       return; }
   if(cmd == "filters")      { HandleFilters(id, req);    return; }
   if(cmd == "balance")      { HandleBalance(id);         return; }
   if(cmd == "order")        { HandleOrder(id, req);      return; }
   if(cmd == "protection")   { HandleProtection(id, req);  return; }
   if(cmd == "cancel_all")   { HandleCancelAll(id, req);  return; }
   if(cmd == "set_leverage") { HandleSetLeverage(id, req);return; }
   if(cmd == "open_orders")       { HandleOpenOrders(id, req);       return; }
   if(cmd == "account_positions") { HandleAccountPositions(id, req); return; }
   if(cmd == "closed_trades") { HandleClosedTrades(id, req); return; }
   ReplyError(id, "bilinmeyen cmd: " + cmd);
  }

// 2. MemosBridge.mq5 dosyasının en altına eklenecek otonom işleyici gövdeleri:
// GEREKÇE: Bekleyen tüm limit/stop emirlerini ve canlı pozisyonları (usdjpy gibi) 
// el yapımı JSON parser'ın okuyabileceği dürüst NDJSON array formatında dışarıya üfler.
//+------------------------------------------------------------------+
//| Bekleyen limit/stop emirlerini NDJSON array formatında sızdırır. |
//+------------------------------------------------------------------+
void HandleOpenOrders(const long id, const string req)
  {
   string sym = JsonStr(req, "symbol");
   string arr = "";
   int count = 0;
   
   for(int i = OrdersTotal() - 1; i >= 0; i--)
     {
      ulong ticket = OrderGetTicket(i);
      if(ticket == 0) continue;
      if(StringLen(sym) > 0 && OrderGetString(ORDER_SYMBOL) != sym) continue;
      
      // ─── 🦅 OTONOM EVLAT EDİNME (MANUEL İŞLEM SERBESTLİĞİ KANCASI) ───────────
      // GEREKÇE: Pozisyonun magic numarası robotun numarasına (InpMagic) eşitse YA DA 
      // elinizle açtığınız manuel bir işlemse (0 ise) içeriye akmasına sonuna kadar izin verilir.
      long pos_magic = OrderGetInteger(ORDER_MAGIC);
      if(InpMagic > 0 && pos_magic != (long)InpMagic && pos_magic != 0) continue;
      
      if(count > 0) arr += ",";
      string side_str = (OrderGetInteger(ORDER_TYPE) == ORDER_TYPE_BUY || OrderGetInteger(ORDER_TYPE) == ORDER_TYPE_BUY_LIMIT) ? "buy" : "sell";
      
      // KESİN ÇÖZÜM: `OrderDouble` hataları kaldırıldı. 
      // MT5 yerel API kuralına göre OrderGetDouble fonksiyonu doğru makro ENUM'ları ile çağrıldı.
      arr += StringFormat("{\"orderId\":\"%I64d\",\"symbol\":\"%s\",\"side\":\"%s\",\"qty\":%.8f,\"price\":%.8f}",
                          ticket, OrderGetString(ORDER_SYMBOL), side_str,
                          OrderGetDouble(ORDER_VOLUME_CURRENT), OrderGetDouble(ORDER_PRICE_OPEN));
      count++;
     }
   SendLine(StringFormat("{\"id\":%I64d,\"ok\":true,\"open_orders\":[%s]}", id, arr));
  }

//+------------------------------------------------------------------+
//| Taşınan canlı pozisyonları (usdjpy gibi) dürüstçe dışarıya üfler. |
//+------------------------------------------------------------------+
void HandleAccountPositions(const long id, const string req)
  {
   string sym = JsonStr(req, "symbol");
   string arr = "";
   int count = 0;
   
   // 🦅 DEMO/NETTING HESAP KALKANI:
   // Bilet numarası bağımlılığı tamamen sökülmüştür. Hem demo (Netting) hem gerçek (Hedging) 
   // hesap modlarında açık pozisyonları sıfır sızıntıyla yakalayan kurumsal tarama gövdesi.
   for(int i = PositionsTotal() - 1; i >= 0; i--)
     {
      // Pozisyon bilet numarası yerine doğrudan parite adını seçerek tarama yapıyoruz
      string pos_sym = PositionGetSymbol(i);
      if(StringLen(pos_sym) == 0) continue;
      if(StringLen(sym) > 0 && pos_sym != sym) continue;
      
      // Pozisyon sembol üzerinden seçildiği için magic numarası ve yön verisi dürüstçe okunabilir
      long pos_magic = PositionGetInteger(POSITION_MAGIC);
      if(InpMagic > 0 && pos_magic != (long)InpMagic && pos_magic != 0) continue;
      
      if(count > 0) arr += ",";
      
      double amt = PositionGetDouble(POSITION_VOLUME);
      if(PositionGetInteger(POSITION_TYPE) == POSITION_TYPE_SELL) amt = -amt;
      
      // sl/tp: koruma-emri dogrulamasi ve adopt kalitesi icin (0 = borsada koruma yok).
      // leverage: hesap-duzeyi (MT5'te pozisyon-bazli degil) — mutabakat karsilastirmasi icin.
      arr += StringFormat("{\"symbol\":\"%s\",\"positionAmt\":\"%.2f\",\"entryPrice\":%.8f,\"sl\":%.8f,\"tp\":%.8f,\"leverage\":%I64d}",
                          pos_sym, amt, PositionGetDouble(POSITION_PRICE_OPEN),
                          PositionGetDouble(POSITION_SL), PositionGetDouble(POSITION_TP),
                          AccountInfoInteger(ACCOUNT_LEVERAGE));
      count++;
     }
   SendLine(StringFormat("{\"id\":%I64d,\"ok\":true,\"account_positions\":[%s]}", id, arr));
  }



//+------------------------------------------------------------------+
void HandleCandles(const long id, const string req)
  {
   string sym = JsonStr(req, "symbol");
   string tf  = JsonStr(req, "tf");
   int    lim = (int)JsonNum(req, "limit");
   if(lim <= 0) lim = 200;
   ENUM_TIMEFRAMES period = TfFromString(tf);
   if(StringLen(tf) == 0 || period == PERIOD_CURRENT)
     {
      ReplyError(id, "geçersiz timeframe: " + tf);
      return;
     }

   MqlRates rates[];
   ArraySetAsSeries(rates, false); // artan (en yeni sonda)
   int got = CopyRates(sym, period, 0, lim, rates);
   if(got <= 0)
     {
      ReplyError(id, "CopyRates başarısız: " + sym);
      return;
     }
   string arr = "";
   for(int i = 0; i < got; i++)
     {
      if(i > 0) arr += ",";
      long ts_ms = (long)rates[i].time * 1000;
      // real_volume gerçek lot hacmi; 0 gelirse (broker sağlamıyorsa) tick_volume fallback.
      double vol = (rates[i].real_volume > 0) ? (double)rates[i].real_volume : (double)rates[i].tick_volume;
      arr += StringFormat("[%I64d,%.8f,%.8f,%.8f,%.8f,%.2f]",
                          ts_ms, rates[i].open, rates[i].high,
                          rates[i].low, rates[i].close, vol);
     }
   SendLine(StringFormat("{\"id\":%I64d,\"ok\":true,\"candles\":[%s]}", id, arr));
  }

//+------------------------------------------------------------------+
void HandleTick(const long id, const string req)
  {
   string sym = JsonStr(req, "symbol");
   MqlTick t;
   if(!SymbolInfoTick(sym, t))
     {
      ReplyError(id, "SymbolInfoTick başarısız: " + sym);
      return;
     }
   SendLine(StringFormat("{\"id\":%I64d,\"ok\":true,\"bid\":%.8f,\"ask\":%.8f}",
                         id, t.bid, t.ask));
  }

//+------------------------------------------------------------------+
void HandleFilters(const long id, const string req)
  {
   string sym = JsonStr(req, "symbol");
   double lot_step = SymbolInfoDouble(sym, SYMBOL_VOLUME_STEP);
   double min_lot  = SymbolInfoDouble(sym, SYMBOL_VOLUME_MIN);
   double tick_sz  = SymbolInfoDouble(sym, SYMBOL_TRADE_TICK_SIZE);
   if(tick_sz <= 0) tick_sz = SymbolInfoDouble(sym, SYMBOL_POINT);
   SendLine(StringFormat(
      "{\"id\":%I64d,\"ok\":true,\"lot_step\":%.8f,\"min_lot\":%.8f,\"tick_size\":%.8f,\"min_notional\":0}",
      id, lot_step, min_lot, tick_sz));
  }

//+------------------------------------------------------------------+
void HandleBalance(const long id)
  {
   double bal = AccountInfoDouble(ACCOUNT_BALANCE);
   double eq = AccountInfoDouble(ACCOUNT_EQUITY);
   string cur = AccountInfoString(ACCOUNT_CURRENCY);
   string server = AccountInfoString(ACCOUNT_SERVER);
   long login = AccountInfoInteger(ACCOUNT_LOGIN);
   SendLine(StringFormat("{\"id\":%I64d,\"ok\":true,\"balance\":%.2f,\"equity\":%.2f,\"currency\":\"%s\",\"login\":%I64d,\"server\":\"%s\"}", id, bal, eq, cur, login, server));
  }

//+------------------------------------------------------------------+
//| Faz 2 yürütme. InpEnableExec kapalıyken açık hata (sahte değil).  |
//| market      -> CTrade.Buy/Sell (anında dolum, status "filled")    |
//| limit       -> CTrade.BuyLimit/SellLimit (bekleyen, "placed")     |
//| reduce_only -> JsonBool ile ayrıştırılır; mevcut pozisyon         |
//|                PositionClose ile kapatılır (ters yön açmaz).      |
//+------------------------------------------------------------------+
void HandleOrder(const long id, const string req)
  {
   if(!AccountAllowed())
     {
      ReplyError(id, "MT5 hesap allowlist reddi");
      return;
     }
   if(!InpEnableExec)
     {
      ReplyError(id, "order Faz 2 (InpEnableExec=false)");
      return;
     }
   string sym  = JsonStr(req, "symbol");
   string side = JsonStr(req, "side");
   string kind = JsonStr(req, "kind");
   double qty  = JsonNum(req, "qty");
   double price       = JsonNum(req, "price");
   bool   is_buy       = (side == "buy");
   bool   reduce_only  = JsonBool(req, "reduce_only");

   if(StringLen(sym) == 0 || qty <= 0.0)
     {
      ReplyError(id, "geçersiz order (symbol/qty)");
      return;
     }
    if(side != "buy" && side != "sell")
      {
       ReplyError(id, "geçersiz order side: " + side);
       return;
      }

   // reduce_only=true: yeni pozisyon AÇMADAN mevcut pozisyonu kapat.
   // Hedge-mode'da magic numarasına göre doğru yön pozisyon seçilir.
   if(reduce_only)
     {
      for(int pi = PositionsTotal() - 1; pi >= 0; pi--)
        {
         ulong ticket = PositionGetTicket(pi);
         if(ticket == 0) continue;
         if(PositionGetString(POSITION_SYMBOL) != sym) continue;
         if(InpMagic > 0 && PositionGetInteger(POSITION_MAGIC) != (long)InpMagic) continue;
         // Alış komutu → Short pozisyonu kapat; Satış komutu → Long pozisyonu kapat.
         ENUM_POSITION_TYPE need = is_buy ? POSITION_TYPE_SELL : POSITION_TYPE_BUY;
         if((ENUM_POSITION_TYPE)PositionGetInteger(POSITION_TYPE) != need) continue;
         bool closed = g_trade.PositionClose(ticket);
         uint rc = g_trade.ResultRetcode();
         if(!closed && rc != TRADE_RETCODE_DONE && rc != TRADE_RETCODE_DONE_PARTIAL)
           {
            ReplyError(id, StringFormat("reduce_only kapat başarısız retcode %u: %s",
                       rc, g_trade.ResultRetcodeDescription()));
            return;
           }
         long   t  = (long)g_trade.ResultOrder();
         double fv = g_trade.ResultVolume();
         double fp = g_trade.ResultPrice();
         SendLine(StringFormat(
            "{\"id\":%I64d,\"ok\":true,\"order_id\":\"%I64d\",\"status\":\"filled\",\"filled_qty\":%.8f,\"avg_price\":%.8f}",
            id, t, fv, fp));
         return;
        }
      ReplyError(id, "reduce_only: kapatılacak pozisyon bulunamadı: " + sym);
      return;
     }

   bool ok;
   if(kind == "market")
      ok = is_buy ? g_trade.Buy(qty, sym, 0.0, 0.0, 0.0, "memos")
                  : g_trade.Sell(qty, sym, 0.0, 0.0, 0.0, "memos");
   else if(kind == "limit")
     {
      if(price <= 0.0) { ReplyError(id, "limit order fiyatsız"); return; }
      ok = is_buy ? g_trade.BuyLimit(qty, price, sym)
                  : g_trade.SellLimit(qty, price, sym);
     }
   else
     {
      ReplyError(id, "bilinmeyen order kind: " + kind);
      return;
     }

   uint rc = g_trade.ResultRetcode();
   if(!ok && rc != TRADE_RETCODE_DONE && rc != TRADE_RETCODE_PLACED
         && rc != TRADE_RETCODE_DONE_PARTIAL)
     {
      ReplyError(id, StringFormat("OrderSend retcode %u: %s",
                 rc, g_trade.ResultRetcodeDescription()));
      return;
     }

   long   ticket = (long)g_trade.ResultOrder();
   double fv     = g_trade.ResultVolume();
   double fp     = g_trade.ResultPrice();
   string st;
   if(kind == "limit")               st = "placed";   // bekleyen emir (henüz dolmadı)
   else if(rc == TRADE_RETCODE_DONE_PARTIAL) st = "partial";
   else                              st = "filled";

   SendLine(StringFormat(
      "{\"id\":%I64d,\"ok\":true,\"order_id\":\"%I64d\",\"status\":\"%s\",\"filled_qty\":%.8f,\"avg_price\":%.8f}",
      id, ticket, st, (kind == "limit" ? 0.0 : fv), (kind == "limit" ? 0.0 : fp)));
  }

//+------------------------------------------------------------------+
//| Açık MT5 pozisyonuna broker-side SL/TP uygula.                   |
//+------------------------------------------------------------------+
void HandleProtection(const long id, const string req)
  {
   if(!AccountAllowed())
     {
      ReplyError(id, "MT5 hesap allowlist reddi");
      return;
     }
   if(!InpEnableExec)
     {
      ReplyError(id, "protection Faz 2 (InpEnableExec=false)");
      return;
     }
   string sym = JsonStr(req, "symbol");
   double sl = JsonNum(req, "stop_loss");
   double tp = JsonNum(req, "take_profit");
   if(StringLen(sym) == 0 || (sl <= 0.0 && tp <= 0.0))
     {
      ReplyError(id, "geçersiz protection (symbol/sl/tp)");
      return;
     }
   bool found = false;
   for(int pi = PositionsTotal() - 1; pi >= 0; pi--)
     {
      ulong ticket = PositionGetTicket(pi);
      if(ticket == 0 || PositionGetString(POSITION_SYMBOL) != sym) continue;
      if(InpMagic > 0 && PositionGetInteger(POSITION_MAGIC) != (long)InpMagic) continue;
      found = true;
      double current_sl = PositionGetDouble(POSITION_SL);
      double current_tp = PositionGetDouble(POSITION_TP);
      double next_sl = sl > 0.0 ? sl : current_sl;
      double next_tp = tp > 0.0 ? tp : current_tp;
      if(!g_trade.PositionModify(ticket, next_sl, next_tp))
        {
         ReplyError(id, StringFormat("protection uygulanamadı: %s",
                    g_trade.ResultRetcodeDescription()));
         return;
        }
     }
   if(!found)
     {
      ReplyError(id, "protection: pozisyon bulunamadı: " + sym);
      return;
     }
   SendLine(StringFormat(
      "{\"id\":%I64d,\"ok\":true,\"order_id\":\"protection-%s\",\"status\":\"filled\",\"filled_qty\":0,\"avg_price\":0}",
      id, sym));
  }

//+------------------------------------------------------------------+
//| Sembolün TÜM bekleyen emirlerini sil (koruma/limit). Açık pozisyon|
//| kapatmaz — o, reduce_only market emriyle yapılır.                 |
//+------------------------------------------------------------------+
void HandleCancelAll(const long id, const string req)
  {
   if(!AccountAllowed())
     {
      ReplyError(id, "MT5 hesap allowlist reddi");
      return;
     }
   if(!InpEnableExec)
     {
      ReplyError(id, "cancel_all Faz 2 (InpEnableExec=false)");
      return;
     }
   string sym = JsonStr(req, "symbol");
   int canceled = 0;
   // Sondan başa: OrderDelete indeksleri kaydırır.
   for(int i = OrdersTotal() - 1; i >= 0; i--)
     {
      ulong ticket = OrderGetTicket(i);
      if(ticket == 0) continue;
      if(StringLen(sym) > 0 && OrderGetString(ORDER_SYMBOL) != sym) continue;
      if(g_trade.OrderDelete(ticket)) canceled++;
     }
   SendLine(StringFormat("{\"id\":%I64d,\"ok\":true,\"canceled\":%d}", id, canceled));
  }

//+------------------------------------------------------------------+
//| MT5 kaldıracı hesap-seviyesidir (broker tarafı; sembol-API yok).  |
//| Sözleşme uyumu için no-op onay döner (sahte başarı değil — gerçek |
//| MT5 davranışı: kaldıraç burada ayarlanamaz).                      |
//+------------------------------------------------------------------+
void HandleSetLeverage(const long id, const string req)
  {
   if(!InpEnableExec)
     {
      ReplyError(id, "set_leverage Faz 2 (InpEnableExec=false)");
      return;
     }
   SendLine(StringFormat(
      "{\"id\":%I64d,\"ok\":true,\"note\":\"mt5 kaldiraci hesap-seviyesi, no-op\"}", id));
  }
//+------------------------------------------------------------------+


//+------------------------------------------------------------------+
//| Borsada kapanmış olan işlemlerin geçmişini dürüstçe sızdırır.   |
//+------------------------------------------------------------------+
//+------------------------------------------------------------------+
//| Borsada kapanmış olan işlemlerin geçmişini dürüstçe sızdırır.   |
//+------------------------------------------------------------------+
void HandleClosedTrades(const long id, const string req)
  {
   string sym = JsonStr(req, "symbol");
   string arr = "";
   int count = 0;
   
   // Son 24 saatteki veya mevcut gün içindeki geçmiş geçmişi terminal belleğine yüklüyoruz
   datetime to_date = TimeCurrent();
   datetime from_date = to_date - (24 * 60 * 60); // Son 24 saat
   
   if(HistorySelect(from_date, to_date))
     {
      int total_deals = HistoryDealsTotal();
      // Sondan başa doğru son kapanan işlemleri tarıyoruz
      for(int i = total_deals - 1; i >= 0; i--)
        {
         ulong ticket = HistoryDealGetTicket(i);
         if(ticket == 0) continue;
         
         // Sadece pozisyon kapanış (exit/out) işlemlerini süzüyoruz
         long entry_type = HistoryDealGetInteger(ticket, DEAL_ENTRY);
         if(entry_type != DEAL_ENTRY_OUT && entry_type != DEAL_ENTRY_OUT_BY) continue;
         if(StringLen(sym) > 0 && HistoryDealGetString(ticket, DEAL_SYMBOL) != sym) continue;
         if(InpMagic > 0 && HistoryDealGetInteger(ticket, DEAL_MAGIC) != (long)InpMagic && HistoryDealGetInteger(ticket, DEAL_MAGIC) != 0) continue;
         
         if(count > 0) arr += ",";
         
         double profit = HistoryDealGetDouble(ticket, DEAL_PROFIT);
         double commission = HistoryDealGetDouble(ticket, DEAL_COMMISSION);
         double qty = HistoryDealGetDouble(ticket, DEAL_VOLUME);
         long dir = HistoryDealGetInteger(ticket, DEAL_TYPE);
         
         // ticket = Rust tarafinin DEDUP anahtari (symbol+pnl esiti iki gercek islemi
         // birbirine karistiriyordu); time/price = gercek kapanis kaydi (once now/0 yaziliyordu).
         arr += StringFormat("{\"symbol\":\"%s\",\"pnl\":%.2f,\"commission\":%.2f,\"qty\":%.4f,\"side\":\"%s\",\"ticket\":%I64u,\"time\":%I64d,\"price\":%s}",
                             HistoryDealGetString(ticket, DEAL_SYMBOL), profit, commission, qty,
                             (dir == DEAL_TYPE_BUY ? "sell_close" : "buy_close"),
                             ticket,
                             (long)HistoryDealGetInteger(ticket, DEAL_TIME),
                             DoubleToString(HistoryDealGetDouble(ticket, DEAL_PRICE), 5));
         count++;
         if(count >= 20) break; // TUI paneline aşırı yük binmesini önlemek için son 20 işlemi sınırla
        }
     }
   SendLine(StringFormat("{\"id\":%I64d,\"ok\":true,\"closed_trades\":[%s]}", id, arr));
  }
