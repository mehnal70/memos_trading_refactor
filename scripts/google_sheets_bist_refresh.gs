/**
 * Memos Trading BIST OHCL refresh.
 *
 * Sheet tabs:
 *   Symbols: symbol,enabled,manual_veto,risk_multiplier
 *   OHCL_INBOX_1d: symbol,date,open,high,low,close,volume,interval,enabled,manual_veto,risk_multiplier
 *
 * GOOGLEFINANCE is delayed and may return incomplete rows. This script writes
 * only complete OHLC rows and leaves the Rust importer fail-closed.
 */
const SYMBOLS_SHEET = 'Symbols';
const OHCL_SHEET = 'OHCL_INBOX_1d';
const LATEST_SHEET = 'Latest';
const IMPORT_LOG_SHEET = 'ImportLog_Runtime';
const IMPORT_LOG_SHEET_FALLBACK = 'ImportLog_Runtime_v2';
const GOOGLE_FINANCE_BUFFER_SHEET = '_GF_BUFFER';
const IMPORT_LOG_HEADERS = [
  'run_started_at', 'finished_at', 'source_rows', 'rows_imported',
  'rows_rejected', 'status', 'error_summary',
];
const LOOKBACK_DAYS = 250;
const GOOGLEFINANCE_WAIT_MS = 1500;
const MAX_SYMBOLS_PER_RUN = 10;
const CURSOR_PROPERTY = 'BIST_OHLC_CURSOR';
const SCHEDULED_DATE_PROPERTY = 'BIST_SCHEDULED_SESSION_DATE';
const BIST_TIMEZONE = 'Europe/Istanbul';
const BIST_REFRESH_HOUR = 18;
const BIST_REFRESH_MINUTE = 20;
const BIST_MORNING_RETRY_START_HOUR = 8;
const BIST_MARKET_OPEN_HOUR = 10;
const SOURCE_END_DATE_BUFFER_DAYS = 1;

function refreshBistOhlc() {
  const lock = LockService.getScriptLock();
  lock.waitLock(30000);
  const startedAt = new Date();
  let imported = 0;
  let rejected = 0;
  let sourceRows = 0;
  let status = 'FAILED';
  let errorSummary = '';
  const spreadsheet = SpreadsheetApp.getActive();
  try {
    const symbols = readEnabledSymbols(spreadsheet);
    if (symbols.length === 0) throw new Error('Aktif BIST sembolü bulunamadı');

    const inbox = readInbox(spreadsheet);
    const cursor = Number(PropertiesService.getScriptProperties().getProperty(CURSOR_PROPERTY) || 0);
    const batch = symbols.slice(cursor, cursor + MAX_SYMBOLS_PER_RUN);
    let nextCursor = cursor;
    for (const item of batch) {
      try {
        const result = fetchSymbolIncremental(spreadsheet, item, inbox);
        sourceRows += result.sourceRows;
        imported += result.imported;
        rejected += result.rejected;
      } catch (error) {
        rejected += 1;
        Logger.log(`${item.symbol}: sembol senkronizasyonu başarısız: ${error}`);
      }
    }
    let symbolsWithLatest = countLatestSymbols(inbox, symbols);
    if (imported > 0) {
      writeInbox(spreadsheet, inbox);
      symbolsWithLatest = writeLatest(spreadsheet, inbox, symbols);
    } else {
      Logger.log('Yeni geçerli OHLC satırı alınamadı; mevcut OHCL_INBOX_1d ve Latest korunuyor');
    }
    if (rejected === 0) {
      nextCursor = (cursor + batch.length) % symbols.length;
    } else {
      Logger.log(`Batch içinde ${rejected} hata var; cursor ilerletilmiyor (${cursor}/${symbols.length})`);
    }
    const sessionDate = getExpectedSessionDate(new Date());
    const symbolsWithSessionDate = countSymbolsWithDate(inbox, symbols, sessionDate);
    PropertiesService.getScriptProperties().setProperty(CURSOR_PROPERTY, String(nextCursor));
    const dataComplete = symbolsWithLatest === symbols.length &&
      symbolsWithSessionDate === symbols.length &&
      rejected === 0;
    if (dataComplete) {
      status = 'COMPLETED';
      nextCursor = 0;
    } else {
      status = 'INCOMPLETE';
    }
    saveRunSummary({
      status, cursor: nextCursor, total_symbols: symbols.length,
      batch_symbols: batch.length, symbols_with_latest: symbolsWithLatest,
      symbols_with_session_date: symbolsWithSessionDate, session_date: sessionDate,
      source_rows: sourceRows, rows_imported: imported, rows_rejected: rejected,
      completed_at: new Date().toISOString(),
    });
    Logger.log(`BIST_SYNC_STATUS status=${status} cursor=${nextCursor}/${symbols.length} `
      + `latest=${symbolsWithLatest}/${symbols.length} rejected=${rejected}`);
    appendImportLog(spreadsheet, {
      startedAt, finishedAt: new Date(), sourceRows, imported, rejected,
      status, errorSummary: '',
    });
  } catch (error) {
    errorSummary = String(error && error.message || error);
    saveRunSummary({
      status: 'FAILED', error_summary: errorSummary,
      completed_at: new Date().toISOString(),
    });
    appendImportLog(spreadsheet, {
      startedAt, finishedAt: new Date(), sourceRows, imported, rejected,
      status, errorSummary,
    });
    throw error;
  } finally {
    lock.releaseLock();
  }
}

function readEnabledSymbols(spreadsheet) {
  const sheet = spreadsheet.getSheetByName(SYMBOLS_SHEET);
  if (!sheet || sheet.getLastRow() < 2) return [];
  const values = sheet.getDataRange().getValues();
  const headers = values.shift().map(String).map((value) => value.trim().toLowerCase());
  const column = (name) => headers.indexOf(name);
  const symbolColumn = column('symbol');
  if (symbolColumn < 0) throw new Error('Symbols sekmesinde symbol kolonu yok');
  const enabledColumn = column('enabled');
  const vetoColumn = column('manual_veto');
  const riskColumn = column('risk_multiplier');
  return values.map((row) => ({
    symbol: String(row[symbolColumn] || '').trim().toUpperCase(),
    enabled: enabledColumn < 0 || truthy(row[enabledColumn], true),
    veto: vetoColumn >= 0 && truthy(row[vetoColumn], false),
    risk: riskColumn >= 0 && Number(row[riskColumn]) > 0 ? Number(row[riskColumn]) : 1,
  })).filter((item) => item.symbol && item.enabled && !item.veto);
}

function readInbox(spreadsheet) {
  const sheet = spreadsheet.getSheetByName(OHCL_SHEET);
  const rows = sheet && sheet.getLastRow() > 1 ? sheet.getDataRange().getValues().slice(1) : [];
  const map = new Map();
  rows.forEach((row) => {
    const symbol = String(row[0] || '').trim().toUpperCase();
    const date = normalizeDate(row[1]);
    if (symbol && date && Number(row[5]) > 0) map.set(`${symbol}|${date.getTime()}`, row);
  });
  return map;
}

function fetchSymbolIncremental(spreadsheet, item, inbox) {
  // Son candle'a göre yalnızca iki gün geriden başlamak, daha eski boşlukları
  // kalıcı olarak kaçırıyordu. Her çalışmada lookback penceresini yeniden tara;
  // mevcut satırlar Map anahtarıyla korunur, eksik günler geri doldurulur.
  const start = new Date(Date.now() - LOOKBACK_DAYS * 86400000);
  const scratch = spreadsheet.getSheetByName(GOOGLE_FINANCE_BUFFER_SHEET) ||
    spreadsheet.insertSheet(GOOGLE_FINANCE_BUFFER_SHEET);
  let sourceRows = 0;
  let imported = 0;
  let rejected = 0;
  try {
    scratch.getRange('A1:F400').clearContent();
    const end = new Date(Date.now() + SOURCE_END_DATE_BUFFER_DAYS * 86400000);
    const startText = Utilities.formatDate(start, BIST_TIMEZONE, 'yyyy-MM-dd');
    const endText = Utilities.formatDate(end, BIST_TIMEZONE, 'yyyy-MM-dd');
    scratch.getRange('A1').setFormula(
      `=GOOGLEFINANCE("IST:${item.symbol}";"all";DATE(${start.getFullYear()};${start.getMonth() + 1};${start.getDate()});DATE(${end.getFullYear()};${end.getMonth() + 1};${end.getDate()});"DAILY")`
    );
    SpreadsheetApp.flush();
    Utilities.sleep(GOOGLEFINANCE_WAIT_MS);
    const rows = scratch.getRange('A1:F400').getValues();
    rows.slice(1).forEach((row) => {
      const date = normalizeDate(row[0]);
      const open = Number(row[1]);
      const high = Number(row[2]);
      const low = Number(row[3]);
      const close = Number(row[4]);
      const volume = Number(row[5] || 0);
      if (!date) return;
      sourceRows += 1;
      if (![open, high, low, close, volume].every(Number.isFinite) ||
          close <= 0 || high < Math.max(open, close) || low > Math.min(open, close)) {
        rejected += 1;
        return;
      }
      inbox.set(`${item.symbol}|${date.getTime()}`, [
        item.symbol, date, open, high, low, close, volume, '1d', 1, 0, item.risk,
      ]);
      imported += 1;
    });
    Logger.log(`${item.symbol}: ${startText}..${endText} aralığından ${imported} işlem günü mumu işlendi`);
  } finally {
    Logger.log(`${item.symbol}: sabit Google Finance buffer korundu`);
  }
  return {sourceRows, imported, rejected};
}

function writeInbox(spreadsheet, inbox) {
  const sheet = spreadsheet.getSheetByName(OHCL_SHEET) || spreadsheet.insertSheet(OHCL_SHEET);
  const header = [['symbol', 'date', 'open', 'high', 'low', 'close', 'volume',
    'interval', 'enabled', 'manual_veto', 'risk_multiplier']];
  const rows = Array.from(inbox.values()).sort((a, b) =>
    String(a[0]).localeCompare(String(b[0])) || normalizeDate(a[1]) - normalizeDate(b[1]));
  const outputRows = rows.map((row) => {
    const output = row.slice();
    const date = normalizeDate(output[1]);
    output[1] = date ? Utilities.formatDate(date, BIST_TIMEZONE, 'yyyy-MM-dd') : '';
    return output;
  });
  const columnCount = header[0].length;
  const previousRows = Math.max(sheet.getLastRow(), 1);
  retrySpreadsheetOperation('OHCL_INBOX temizleme', () => {
    sheet.getRange(1, 1, Math.max(previousRows, outputRows.length + 1), columnCount).clearContent();
  });
  retrySpreadsheetOperation('OHCL_INBOX başlık yazma', () => {
    sheet.getRange(1, 1, 1, columnCount).setValues(header);
  });
  for (let offset = 0; offset < outputRows.length; offset += 500) {
    const chunk = outputRows.slice(offset, offset + 500);
    const startRow = offset + 2;
    retrySpreadsheetOperation(`OHCL_INBOX satırları yazma (${startRow}-${startRow + chunk.length - 1})`, () => {
      sheet.getRange(startRow, 1, chunk.length, columnCount).setValues(chunk);
    });
  }
}

function retrySpreadsheetOperation(label, operation) {
  const maxAttempts = 3;
  let lastError = null;
  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    try {
      operation();
      return;
    } catch (error) {
      lastError = error;
      Logger.log(`${label} başarısız (deneme ${attempt}/${maxAttempts}): ${error}`);
      if (attempt < maxAttempts) Utilities.sleep(attempt * 2000);
    }
  }
  throw new Error(`${label} ${maxAttempts} denemede tamamlanamadı: ${lastError}`);
}

function writeLatest(spreadsheet, inbox, symbols) {
  const latest = new Map();
  inbox.forEach((row) => {
    const current = latest.get(row[0]);
    if (!current || normalizeDate(row[1]) > normalizeDate(current[1])) latest.set(row[0], row);
  });
  const rows = [['symbol', 'date', 'open', 'high', 'low', 'close', 'volume', 'interval', 'updated_at']];
  symbols.forEach((item) => {
    const row = latest.get(item.symbol);
    if (row) {
      const date = normalizeDate(row[1]);
      rows.push([
        row[0],
        date ? Utilities.formatDate(date, BIST_TIMEZONE, 'yyyy-MM-dd') : '',
        row[2],
        row[3],
        row[4],
        row[5],
        row[6],
        row[7],
        Utilities.formatDate(new Date(), BIST_TIMEZONE, "yyyy-MM-dd'T'HH:mm:ssXXX"),
      ]);
    }
  });
  const sheet = spreadsheet.getSheetByName(LATEST_SHEET) || spreadsheet.insertSheet(LATEST_SHEET);
  sheet.clearContents();
  sheet.getRange(1, 1, rows.length, rows[0].length).setValues(rows);
  return rows.length - 1;
}

function countSymbolsWithDate(inbox, symbols, sessionDate) {
  const latest = new Map();
  inbox.forEach((row) => {
    const current = latest.get(row[0]);
    if (!current || normalizeDate(row[1]) > normalizeDate(current[1])) {
      latest.set(row[0], row);
    }
  });
  return symbols.filter((item) => {
    const row = latest.get(item.symbol);
    return row && Utilities.formatDate(normalizeDate(row[1]), BIST_TIMEZONE, 'yyyy-MM-dd') === sessionDate;
  }).length;
}

function countLatestSymbols(inbox, symbols) {
  const latest = new Set();
  inbox.forEach((row) => latest.add(String(row[0]).trim().toUpperCase()));
  return symbols.filter((item) => latest.has(item.symbol)).length;
}

function saveRunSummary(summary) {
  PropertiesService.getScriptProperties().setProperty(
    'BIST_LAST_RUN_SUMMARY', JSON.stringify(summary)
  );
}

function appendImportLog(spreadsheet, result) {
  const payload = {
    run_started_at: result.startedAt instanceof Date ? result.startedAt.toISOString() : String(result.startedAt),
    finished_at: result.finishedAt instanceof Date ? result.finishedAt.toISOString() : String(result.finishedAt),
    source_rows: Number(result.sourceRows) || 0,
    rows_imported: Number(result.imported) || 0,
    rows_rejected: Number(result.rejected) || 0,
    status: String(result.status || ''),
    error_summary: String(result.errorSummary || ''),
  };
  try {
    const sheet = ensurePlainImportLog(spreadsheet);
    if (!sheet) throw new Error('ImportLog sekmesi oluşturulamadı');
    sheet.getRange(sheet.getLastRow() + 1, 1, 1, 7).setValues([[
      String(payload.run_started_at), String(payload.finished_at),
      String(payload.source_rows), String(payload.rows_imported),
      String(payload.rows_rejected), String(payload.status),
      String(payload.error_summary),
    ]]);
  } catch (error) {
    PropertiesService.getScriptProperties().setProperty(
      'BIST_LAST_IMPORT_LOG', JSON.stringify(payload)
    );
    Logger.log(`ImportLog yazılamadı; son kayıt PropertiesService'e alındı: ${error}`);
  }
}

function ensurePlainImportLog(spreadsheet) {
  const properties = PropertiesService.getScriptProperties();
  let sheet = spreadsheet.getSheetByName(IMPORT_LOG_SHEET);
  const hasExpectedHeader = (candidate) => {
    if (!candidate || candidate.getLastRow() === 0 || candidate.getLastColumn() < IMPORT_LOG_HEADERS.length) {
      return false;
    }
    const header = candidate
      .getRange(1, 1, 1, IMPORT_LOG_HEADERS.length)
      .getDisplayValues()[0]
      .map((value) => String(value).trim());
    return JSON.stringify(header) === JSON.stringify(IMPORT_LOG_HEADERS);
  };

  if (properties.getProperty('BIST_IMPORT_LOG_PLAIN_V2') === '1' &&
      hasExpectedHeader(sheet)) {
    return sheet;
  }

  if (!hasExpectedHeader(sheet)) {
    try {
      sheet = spreadsheet.getSheetByName(IMPORT_LOG_SHEET_FALLBACK) ||
        spreadsheet.insertSheet(IMPORT_LOG_SHEET_FALLBACK);
      if (sheet.getLastRow() === 0) {
        sheet.getRange(1, 1, 1, IMPORT_LOG_HEADERS.length)
          .setValues([IMPORT_LOG_HEADERS]);
      }
    } catch (error) {
      Logger.log(`ImportLog sekmesi oluşturulamadı: ${error}`);
      return null;
    }
  }
  properties.setProperty('BIST_IMPORT_LOG_PLAIN_V2', '1');
  return sheet;
}

function normalizeDate(value) {
  if (value instanceof Date && !isNaN(value.getTime())) return new Date(value.getFullYear(), value.getMonth(), value.getDate());
  const date = new Date(value);
  return isNaN(date.getTime()) ? null : new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function installBistRefreshTrigger() {
  ScriptApp.getProjectTriggers()
    .filter((trigger) => ['scheduledBistRefresh', 'refreshBistOhlc']
      .includes(trigger.getHandlerFunction()))
    .forEach((trigger) => ScriptApp.deleteTrigger(trigger));
  ScriptApp.newTrigger('scheduledBistRefresh').timeBased().everyMinutes(15).create();
}

function scheduledBistRefresh() {
  const now = new Date();
  const local = getBistLocalDate(now);
  const hour = local.getHours();
  const afterClose = local.getHours() > BIST_REFRESH_HOUR ||
    (local.getHours() === BIST_REFRESH_HOUR && local.getMinutes() >= BIST_REFRESH_MINUTE);
  const morningRetry = hour >= BIST_MORNING_RETRY_START_HOUR &&
    hour < BIST_MARKET_OPEN_HOUR;
  if (!afterClose && !morningRetry) {
    Logger.log('BIST_SCHEDULE status=SKIPPED reason=outside_refresh_window');
    return;
  }

  const sessionDate = getExpectedSessionDate(now);
  const lastScheduledDate = PropertiesService.getScriptProperties()
    .getProperty(SCHEDULED_DATE_PROPERTY);
  const rawSummary = PropertiesService.getScriptProperties()
    .getProperty('BIST_LAST_RUN_SUMMARY');
  let summary = null;
  if (rawSummary) {
    try {
      summary = JSON.parse(rawSummary);
    } catch (error) {
      Logger.log(`BIST_SCHEDULE summary parse edilemedi: ${error}`);
    }
  }
  if (lastScheduledDate === sessionDate &&
      summary &&
      summary.status === 'COMPLETED' &&
      summary.session_date === sessionDate &&
      summary.symbols_with_session_date === summary.total_symbols) {
    Logger.log(`BIST_SCHEDULE status=SKIPPED reason=already_completed date=${sessionDate}`);
    return;
  }

  refreshBistOhlc();
  const updated = PropertiesService.getScriptProperties()
    .getProperty('BIST_LAST_RUN_SUMMARY');
  if (updated) {
    const result = JSON.parse(updated);
    if (result.status === 'COMPLETED') {
      PropertiesService.getScriptProperties()
        .setProperty(SCHEDULED_DATE_PROPERTY, sessionDate);
    }
    Logger.log(`BIST_SCHEDULE date=${sessionDate} status=${result.status}`);
  }
}

function getBistLocalDate(date) {
  const text = Utilities.formatDate(date, BIST_TIMEZONE, 'yyyy-MM-dd HH:mm:ss');
  const parts = text.split(/[- :]/).map(Number);
  return new Date(parts[0], parts[1] - 1, parts[2], parts[3], parts[4], parts[5]);
}

function getExpectedSessionDate(date) {
  const local = getBistLocalDate(date);
  const hour = local.getHours();
  const afterClose = hour > BIST_REFRESH_HOUR ||
    (hour === BIST_REFRESH_HOUR && local.getMinutes() >= BIST_REFRESH_MINUTE);
  let target = new Date(local.getTime());
  if (!afterClose || target.getDay() === 0 || target.getDay() === 6) {
    target.setDate(target.getDate() - 1);
  }
  while (target.getDay() === 0 || target.getDay() === 6) {
    target.setDate(target.getDate() - 1);
  }
  return Utilities.formatDate(target, BIST_TIMEZONE, 'yyyy-MM-dd');
}

function syncBistSymbols() {
  const spreadsheet = SpreadsheetApp.getActive();
  const sheet = spreadsheet.getSheetByName(SYMBOLS_SHEET) ||
    spreadsheet.insertSheet(SYMBOLS_SHEET);
  const headers = ['symbol', 'enabled', 'manual_veto', 'risk_multiplier'];
  const targetSymbols = 'AEFES,AKBNK,AKSA,AKSEN,ALARK,ALTNY,ANSGR,ARCLK,ASELS,ASTOR,BALSU,BERA,BIMAS,BRSAN,BRYAT,BSOKE,BTCIM,CANTE,CCOLA,CIMSA,CVKMD,CWENE,DAPGM,DOAS,DOHOL,DSTKF,ECILC,EFOR,EKGYO,ENERY,ENJSA,ENKAI,EREGL,ESEN,EUPWR,EUREN,FENER,FROTO,GARAN,GENIL,GESAN,GLRMK,GRSEL,GRTHO,GSRAY,GUBRF,HALKB,HEKTS,IEYHO,ISCTR,ISMEN,IZENR,KCHOL,KLRHO,KRDMD,KTLEV,KUYAS,MAGEN,MAVI,MGROS,MIATK,MPARK,OBAMS,ODAS,ODINE,OTKAR,OYAKC,PAHOL,PASEU,PATEK,PETKM,PGSUS,PSGYO,QUAGR,RALYH,REEDR,SAHOL,SARKY,SASA,SISE,SKBNK,SOKM,TAVHL,TCELL,THYAO,TKFEN,TOASO,TRALT,TRENJ,TRMET,TSKB,TTKOM,TUKAS,TUPRS,TURSG,ULKER,VAKBN,VESTL,YKBNK,ZOREN'.split(',');
  const lastColumn = Math.max(sheet.getLastColumn(), headers.length);
  if (sheet.getLastRow() === 0) {
    sheet.getRange(1, 1, 1, headers.length).setValues([headers]);
  }
  const currentHeaders = sheet.getRange(1, 1, 1, lastColumn).getValues()[0]
    .map((value) => String(value).trim().toLowerCase());
  const column = (name) => currentHeaders.indexOf(name);
  const symbolColumn = column('symbol');
  const enabledColumn = column('enabled');
  const vetoColumn = column('manual_veto');
  const riskColumn = column('risk_multiplier');
  if ([symbolColumn, enabledColumn, vetoColumn, riskColumn].some((value) => value < 0)) {
    throw new Error('Symbols sekmesinde symbol/enabled/manual_veto/risk_multiplier başlıkları eksik');
  }

  const existing = new Set();
  if (sheet.getLastRow() > 1) {
    sheet.getRange(2, symbolColumn + 1, sheet.getLastRow() - 1, 1)
      .getValues()
      .forEach((row) => {
        const symbol = String(row[0] || '').trim().toUpperCase();
        if (symbol) existing.add(symbol);
      });
  }
  const newRows = targetSymbols
    .filter((symbol) => !existing.has(symbol))
    .map((symbol) => {
      const row = Array(lastColumn).fill('');
      row[symbolColumn] = symbol;
      row[enabledColumn] = 1;
      row[vetoColumn] = 0;
      row[riskColumn] = 1;
      return row;
    });
  if (newRows.length > 0) {
    sheet.getRange(sheet.getLastRow() + 1, 1, newRows.length, lastColumn).setValues(newRows);
  }
  Logger.log(`Symbols senkronizasyonu tamamlandı: ${newRows.length} yeni sembol, mevcut ayarlar korundu`);
}

function truthy(value, fallback) {
  if (value === '' || value === null || value === undefined) return fallback;
  return !['0', 'false', 'off', 'no', 'n'].includes(String(value).trim().toLowerCase());
}

function doGet(e) {
  const rawSyncStatus = PropertiesService.getScriptProperties()
    .getProperty('BIST_LAST_RUN_SUMMARY');
  let syncStatus = null;
  if (rawSyncStatus) {
    try {
      syncStatus = JSON.parse(rawSyncStatus);
    } catch (error) {
      syncStatus = {status: 'UNKNOWN', error_summary: String(error)};
    }
  }

  // Refresh worker status pollingi için tüm OHCL sayfasını taşımak pahalıdır.
  // Tam veri export'u yalnızca varsayılan endpoint davranışında korunur.
  if (e && e.parameter && e.parameter.status === '1') {
    return ContentService
      .createTextOutput(JSON.stringify({
        ok: true,
        sync_status: syncStatus,
      }))
      .setMimeType(ContentService.MimeType.JSON);
  }

  const spreadsheet = SpreadsheetApp.getActive();
  const sheet = spreadsheet.getSheetByName('OHCL_INBOX_1d');

  if (!sheet) {
    return ContentService
      .createTextOutput(JSON.stringify({
        ok: false,
        error: 'OHCL_INBOX_1d bulunamadı'
      }))
      .setMimeType(ContentService.MimeType.JSON);
  }

  const values = sheet.getDataRange().getValues();
  const timezone = spreadsheet.getSpreadsheetTimeZone() || Session.getScriptTimeZone();
  const apiValues = values.map((row) => row.map((value) => {
    if (value instanceof Date && !isNaN(value.getTime())) {
      return Utilities.formatDate(value, timezone, 'yyyy-MM-dd');
    }
    return String(value ?? '');
  }));
  return ContentService
    .createTextOutput(JSON.stringify({
      ok: true,
      range: 'OHCL_INBOX_1d',
      values: apiValues,
      sync_status: syncStatus,
    }))
    .setMimeType(ContentService.MimeType.JSON);
}