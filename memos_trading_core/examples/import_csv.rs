// import_csv — evrensel OHLC CSV → DB içe-aktarıcı (dünya piyasaları, kaynak-agnostik).
//
// [[project_world_markets]] Faz A: BIST/forex/emtia/ABD ücretsiz oto-çekim bot-kapısına takıldı
// (Yahoo 429 / Stooq PoW / TD-ücretsiz kilit / isyatirim 401). KESİN çözüm: veriyi nereden kolaysa
// al (yfinance, TradingView export, broker, isyatirim web), CSV olarak ver — buradan DB'ye aktar.
// download_twelvedata/yahoo/isyatirim ile AYNI market etiketi → ölçüm (xs_momentum) kaynak-agnostik.
//
// Başlık otomatik-algılanır (büyük/küçük + TR/EN): date/open/high/low/close[/volume]. Tarih biçimi
// esnek (ISO, DD-MM-YYYY, MM/DD/YYYY, unix). Sayı esnek (1,234.56 / 1234,56 / 1234.56).
//
// Kullanım:
//   cargo run --release --example import_csv -- <market> <interval> <dosya_veya_klasör> [SYMBOL]
//   - dosya  → SYMBOL verilmezse dosya adı (uzantısız) sembol olur
//   - klasör → içindeki her .csv ayrı sembol (dosya adı = sembol)
// Örnek:
//   cargo run --release --example import_csv -- bist 1d ./bist_csv          (klasör: THYAO.csv,...)
//   cargo run --release --example import_csv -- forex 1d ./EURUSD.csv EURUSD
//
// Env: DB_PATH (default data/trader.db).

use std::path::Path;

use memos_trading_core::core::types::Candle;
use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc, DateTime};

/// Sayıyı esnek parse et: US binlik (1,234.56), AB ondalık (1234,56), düz (1234.56).
fn parse_num(s: &str) -> Option<f64> {
    let t = s.trim().trim_matches('"');
    if t.is_empty() { return None; }
    let cleaned = if t.contains(',') && t.contains('.') {
        t.replace(',', "")            // virgül = binlik ayraç
    } else if t.contains(',') {
        t.replace(',', ".")           // virgül = ondalık
    } else {
        t.to_string()
    };
    cleaned.parse::<f64>().ok()
}

/// Esnek tarih parse (yaygın CSV biçimleri + unix).
fn parse_date(s: &str) -> Option<DateTime<Utc>> {
    let t = s.trim().trim_matches('"');
    // Unix saniye/ms (tam sayı)
    if t.chars().all(|c| c.is_ascii_digit()) && (t.len() == 10 || t.len() == 13) {
        if let Ok(n) = t.parse::<i64>() {
            let secs = if t.len() == 13 { n / 1000 } else { n };
            return DateTime::from_timestamp(secs, 0);
        }
    }
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M:%SZ"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(t, fmt) {
            return Some(Utc.from_utc_datetime(&dt));
        }
    }
    for fmt in ["%Y-%m-%d", "%d-%m-%Y", "%d.%m.%Y", "%d/%m/%Y", "%m/%d/%Y"] {
        if let Ok(d) = NaiveDate::parse_from_str(t, fmt) {
            return Some(Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap()));
        }
    }
    None
}

/// Başlıkta öncelik listesine göre TAM-eşleşen ilk kolonun indeksini bul (büyük/küçük duyarsız).
fn find_col(headers: &[String], names: &[&str]) -> Option<usize> {
    for name in names {
        if let Some(i) = headers.iter().position(|h| h.eq_ignore_ascii_case(name)) {
            return Some(i);
        }
    }
    None
}

/// CSV içeriğini `Candle`'a parse et — SAF (ağsız test). Başlık otomatik-algılanır.
fn parse_csv(content: &str, symbol: &str, interval: &str) -> Result<Vec<Candle>, String> {
    let mut lines = content.lines().filter(|l| !l.trim().is_empty());
    let header_line = lines.next().ok_or_else(|| format!("CSV boş [{symbol}]"))?;
    let delim = if header_line.contains(';') && !header_line.contains(',') { ';' } else { ',' };
    let headers: Vec<String> = header_line.split(delim).map(|h| h.trim().trim_matches('"').to_string()).collect();

    let i_date = find_col(&headers, &["date", "datetime", "time", "tarih", "timestamp"])
        .ok_or_else(|| format!("CSV: tarih kolonu yok [{symbol}] (başlık: {headers:?})"))?;
    let i_open = find_col(&headers, &["open", "açılış", "acilis"])
        .ok_or_else(|| format!("CSV: open kolonu yok [{symbol}]"))?;
    let i_high = find_col(&headers, &["high", "yüksek", "yuksek", "max"])
        .ok_or_else(|| format!("CSV: high kolonu yok [{symbol}]"))?;
    let i_low = find_col(&headers, &["low", "düşük", "dusuk", "min"])
        .ok_or_else(|| format!("CSV: low kolonu yok [{symbol}]"))?;
    let i_close = find_col(&headers, &["close", "kapanış", "kapanis", "son", "last", "price"])
        .ok_or_else(|| format!("CSV: close kolonu yok [{symbol}]"))?;
    let i_vol = find_col(&headers, &["volume", "vol", "hacim"]); // opsiyonel

    let mut candles = Vec::new();
    for line in lines {
        let cols: Vec<&str> = line.split(delim).collect();
        let get = |i: usize| cols.get(i).copied().unwrap_or("");
        let ts = match parse_date(get(i_date)) { Some(t) => t, None => continue };
        let (open, high, low, close) = match (
            parse_num(get(i_open)), parse_num(get(i_high)), parse_num(get(i_low)), parse_num(get(i_close)),
        ) {
            (Some(o), Some(h), Some(l), Some(c)) => (o, h, l, c),
            _ => continue,
        };
        let volume = i_vol.and_then(|i| parse_num(get(i))).unwrap_or(0.0).max(0.0);
        if memos_trading_core::robot::data_fetcher::validate_ohlcv(open, high, low, close, volume).is_err() {
            continue;
        }
        candles.push(Candle { timestamp: ts, open, high, low, close, volume, symbol: symbol.to_string(), interval: interval.to_string() });
    }
    candles.sort_by_key(|c| c.timestamp);
    candles.dedup_by_key(|c| c.timestamp);
    Ok(candles)
}

fn symbol_from_path(p: &Path) -> String {
    p.file_stem().and_then(|s| s.to_str()).unwrap_or("UNKNOWN").to_uppercase()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let market = args.get(1).cloned().unwrap_or_default();
    let interval = args.get(2).map(|s| s.as_str()).unwrap_or("1d").to_string();
    let path_arg = args.get(3).cloned().unwrap_or_default();
    let sym_override = args.get(4).map(|s| s.to_uppercase());
    if market.is_empty() || path_arg.is_empty() {
        eprintln!("⚠️  Kullanım: import_csv -- <market> <interval> <dosya_veya_klasör> [SYMBOL]");
        std::process::exit(2);
    }
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "data/trader.db".into());
    let conn = memos_trading_core::persistence::open_db(&db_path).expect("DB açılamadı");

    // Dosya listesi: klasörse tüm .csv, dosyaysa kendisi.
    let p = Path::new(&path_arg);
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if p.is_dir() {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                let fp = e.path();
                if fp.extension().and_then(|x| x.to_str()).map(|x| x.eq_ignore_ascii_case("csv")).unwrap_or(false) {
                    files.push(fp);
                }
            }
        }
        files.sort();
    } else {
        files.push(p.to_path_buf());
    }
    if files.is_empty() {
        eprintln!("⚠️  CSV bulunamadı: {path_arg}");
        std::process::exit(2);
    }

    println!("📥 import_csv · market='{market}' · interval={interval} · {} dosya · db={db_path}", files.len());
    let (mut ok, mut total, mut failed) = (0usize, 0usize, 0usize);
    for fp in &files {
        let symbol = sym_override.clone().unwrap_or_else(|| symbol_from_path(fp));
        let content = match std::fs::read_to_string(fp) {
            Ok(c) => c,
            Err(e) => { println!("  ✗ {:12} okunamadı: {e}", symbol); failed += 1; continue; }
        };
        match parse_csv(&content, &symbol, &interval) {
            Ok(candles) if !candles.is_empty() => {
                let mut saved = 0usize;
                for c in &candles {
                    if memos_trading_core::persistence::writer::save_candle(&conn, &market, &market, c).is_ok() {
                        saved += 1;
                    }
                }
                let first = candles.first().map(|c| c.timestamp.format("%Y-%m-%d").to_string()).unwrap_or_default();
                let last = candles.last().map(|c| c.timestamp.format("%Y-%m-%d").to_string()).unwrap_or_default();
                println!("  ✅ {:12} {} mum ({} → {})", symbol, saved, first, last);
                ok += 1; total += saved;
            }
            Ok(_) => { println!("  ⚠️ {:12} geçerli satır yok", symbol); failed += 1; }
            Err(e) => { println!("  ✗ {:12} {e}", symbol); failed += 1; }
        }
    }
    println!("\n→ {} / {} dosya · {} başarısız · toplam {} mum yazıldı (market='{}').",
        ok, files.len(), failed, total, market);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_num_variants() {
        assert_eq!(parse_num("1234.56"), Some(1234.56));
        assert_eq!(parse_num("1,234.56"), Some(1234.56)); // US binlik
        assert_eq!(parse_num("1234,56"), Some(1234.56));   // AB ondalık
        assert_eq!(parse_num("\"100\""), Some(100.0));
        assert_eq!(parse_num(""), None);
    }

    #[test]
    fn parse_date_variants() {
        let d = NaiveDate::from_ymd_opt(2024, 6, 18).unwrap();
        let want = Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap());
        assert_eq!(parse_date("2024-06-18"), Some(want));
        assert_eq!(parse_date("18-06-2024"), Some(want));
        assert_eq!(parse_date("06/18/2024"), Some(want));
        assert_eq!(parse_date("1718668800"), DateTime::from_timestamp(1718668800, 0));
        assert!(parse_date("saçma").is_none());
    }

    #[test]
    fn parse_csv_yfinance_header() {
        // yfinance: Date,Open,High,Low,Close,Adj Close,Volume — "Adj Close" 'close'a karışmamalı.
        let csv = "Date,Open,High,Low,Close,Adj Close,Volume\n2024-06-17,100,103,99,102,101.5,1000\n2024-06-18,102,108,101,107,106.4,1200\n";
        let c = parse_csv(csv, "THYAO", "1d").unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].close, 102.0, "Adj Close değil Close seçilmeli");
        assert_eq!(c[1].volume, 1200.0);
        assert!(c[0].timestamp < c[1].timestamp);
    }

    #[test]
    fn parse_csv_tr_header_semicolon() {
        let csv = "Tarih;Açılış;Yüksek;Düşük;Kapanış;Hacim\n18.06.2024;300,5;308,0;299,0;307,0;1000000\n";
        let c = parse_csv(csv, "X", "1d").unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].open, 300.5);
        assert_eq!(c[0].close, 307.0);
    }

    #[test]
    fn parse_csv_missing_close_col_is_err() {
        let csv = "Date,Open,High,Low,Volume\n2024-06-18,1,2,0.5,100\n";
        assert!(parse_csv(csv, "X", "1d").is_err());
    }
}
