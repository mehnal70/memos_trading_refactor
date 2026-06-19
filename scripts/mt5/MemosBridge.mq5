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
#property version   "1.00"
#property strict

input string InpHost      = "127.0.0.1"; // Rust köprü host
input int    InpPort      = 9001;        // Rust köprü port (MT5_BRIDGE_ADDR ile aynı)
input int    InpPollMs    = 50;          // OnTimer periyodu (ms)
input bool   InpEnableExec = false;      // Faz 2: emir yürütmeyi aç (varsayılan kapalı)

int    g_sock = INVALID_HANDLE;          // soket handle
string g_buf  = "";                      // satır-birleştirme tamponu

//+------------------------------------------------------------------+
int OnInit()
  {
   EventSetMillisecondTimer(InpPollMs);
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
   CloseSock();
   g_sock = SocketCreate();
   if(g_sock == INVALID_HANDLE)
     {
      Print("MemosBridge: SocketCreate başarısız (izin/ayar?)");
      return(false);
     }
   if(!SocketConnect(g_sock, InpHost, InpPort, 1000))
     {
      CloseSock();
      return(false); // köprü henüz ayakta değil; sessiz tekrar dene
     }
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
         break;
      g_buf += CharArrayToString(buf, 0, n, CP_UTF8);
      avail = SocketIsReadable(g_sock);
     }
  }

//+------------------------------------------------------------------+
void SendLine(const string line)
  {
   string out = line + "\n";
   uchar buf[];
   int len = StringToCharArray(out, buf, 0, WHOLE_ARRAY, CP_UTF8) - 1; // sondaki \0 hariç
   if(len > 0)
      SocketSend(g_sock, buf, len);
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
   SendLine(StringFormat("{\"id\":%I64d,\"ok\":false,\"error\":\"%s\"}", id, msg));
  }

//+------------------------------------------------------------------+
void HandleRequest(const string req)
  {
   long   id  = (long)JsonNum(req, "id");
   string cmd = JsonStr(req, "cmd");

   if(cmd == "candles")      { HandleCandles(id, req); return; }
   if(cmd == "tick")         { HandleTick(id, req);    return; }
   if(cmd == "filters")      { HandleFilters(id, req); return; }
   if(cmd == "balance")      { HandleBalance(id);      return; }
   if(cmd == "order")        { HandleOrder(id, req);   return; }
   if(cmd == "cancel_all")   { ReplyError(id, "cancel_all Faz 2");   return; }
   if(cmd == "set_leverage") { ReplyError(id, "set_leverage Faz 2"); return; }
   ReplyError(id, "bilinmeyen cmd: " + cmd);
  }

//+------------------------------------------------------------------+
void HandleCandles(const long id, const string req)
  {
   string sym = JsonStr(req, "symbol");
   string tf  = JsonStr(req, "tf");
   int    lim = (int)JsonNum(req, "limit");
   if(lim <= 0) lim = 200;
   ENUM_TIMEFRAMES period = TfFromString(tf);

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
      arr += StringFormat("[%I64d,%.8f,%.8f,%.8f,%.8f,%.2f]",
                          ts_ms, rates[i].open, rates[i].high,
                          rates[i].low, rates[i].close, (double)rates[i].tick_volume);
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
   SendLine(StringFormat("{\"id\":%I64d,\"ok\":true,\"balance\":%.2f}", id, bal));
  }

//+------------------------------------------------------------------+
//| Faz 2 yürütme. InpEnableExec kapalıyken açık hata (sahte değil).  |
//+------------------------------------------------------------------+
void HandleOrder(const long id, const string req)
  {
   if(!InpEnableExec)
     {
      ReplyError(id, "order Faz 2 (InpEnableExec=false)");
      return;
     }
   // Faz 2 iskeleti: CTrade ile market/limit OrderSend burada uygulanacak.
   ReplyError(id, "order Faz 2 — yürütme henüz uygulanmadı");
  }
//+------------------------------------------------------------------+
