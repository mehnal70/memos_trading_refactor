#!/usr/bin/env bash
# db_coverage backfill planı · 265 sağlıksız seri · eşik: min_rows=300 max_gap=5% max_age=3bar
# FORCE_FULL = iç-gap/seyrek (tüm pencere yeniden) · artımlı = yalnız bayat-kuyruk
# Komutlar SIRALI (serial) koşar; save_candle upsert → tekrar çalıştırmak güvenli.
set -euo pipefail
DB_PATH="${DB_PATH:-/home/ulas/PyCharmMiscProject/memos_trading_refactor/data/trader.db}"
export DB_PATH

echo '▶ futures 1m · FORCE_FULL (iç-gap/seyrek) · 27 sembol'
FORCE_FULL=1 cargo run --release -p memos_trading_core --example download_candles -- futures 1m ADAUSDT,ALPACAUSDT,ALPHAUSDT,ATUSDT,BCHUSDT,BEATUSDT,BNBUSDT,BNXUSDT,BTCUSDC,BTCUSDT,DOGEUSDT,ETHUSDC,ETHUSDT,FLOWUSDT,HIGHUSDT,HYPEUSDT,ONTUSDT,PORT3USDT,RAVEUSDT,RVVUSDT,SOLUSDT,STORJUSDT,SUIUSDT,UNIUSDT,XRPUSDT,ZBTUSDT,ZECUSDT 1

echo '▶ futures 2h · FORCE_FULL (iç-gap/seyrek) · 29 sembol'
FORCE_FULL=1 cargo run --release -p memos_trading_core --example download_candles -- futures 2h ADAUSDT,ALPACAUSDT,ALPHAUSDT,ATUSDT,AVAXUSDT,BCHUSDT,BEATUSDT,BNBUSDT,BNXUSDT,BTCUSDC,BTCUSDT,DOGEUSDT,ETHUSDC,ETHUSDT,FLOWUSDT,HYPEUSDT,ONTUSDT,PORT3USDT,RVVUSDT,SOLUSDT,STORJUSDT,SUIUSDT,TRXUSDT,UNIUSDT,USDCUSDT,XPLUSDT,XRPUSDT,ZBTUSDT,ZECUSDT 6

echo '▶ futures 1m · artımlı (bayat-kuyruk) · 4 sembol'
cargo run --release -p memos_trading_core --example download_candles -- futures 1m AVAXUSDT,TRXUSDT,USDCUSDT,XPLUSDT 1

echo '▶ futures 5m · FORCE_FULL (iç-gap/seyrek) · 14 sembol'
FORCE_FULL=1 cargo run --release -p memos_trading_core --example download_candles -- futures 5m ADAUSDT,ATUSDT,BEATUSDT,BTCUSDT,COAIUSDT,CRCLUSDT,DOTUSDT,ETHUSDT,HIGHUSDT,MYXUSDT,ORDIUSDT,RAVEUSDT,RIVERUSDT,SIRENUSDT 1

echo '▶ futures 15m · FORCE_FULL (iç-gap/seyrek) · 13 sembol'
FORCE_FULL=1 cargo run --release -p memos_trading_core --example download_candles -- futures 15m ADAUSDT,BTCUSDT,COAIUSDT,CRCLUSDT,DOGEUSDT,DOTUSDT,ETHUSDT,HIGHUSDT,MYXUSDT,ORDIUSDT,RAVEUSDT,RIVERUSDT,SIRENUSDT 6

echo '▶ futures 15m · artımlı (bayat-kuyruk) · 8 sembol'
cargo run --release -p memos_trading_core --example download_candles -- futures 15m AAVEUSDT,ALGOUSDT,ASTERUSDT,ATOMUSDT,AXSUSDT,BANDUSDT,BATUSDT,BCHUSDT 1

echo '▶ futures 30m · FORCE_FULL (iç-gap/seyrek) · 12 sembol'
FORCE_FULL=1 cargo run --release -p memos_trading_core --example download_candles -- futures 30m ADAUSDT,BTCUSDT,COAIUSDT,CRCLUSDT,DOTUSDT,ETHUSDT,HIGHUSDT,MYXUSDT,ORDIUSDT,RAVEUSDT,RIVERUSDT,SIRENUSDT 1

echo '▶ futures 1h · FORCE_FULL (iç-gap/seyrek) · 24 sembol'
FORCE_FULL=1 cargo run --release -p memos_trading_core --example download_candles -- futures 1h ADAUSDT,ALPACAUSDT,ALPHAUSDT,BCHUSDT,BNXUSDT,BTCUSDT,COAIUSDT,CRCLUSDT,DOTUSDT,ETHUSDT,HIGHUSDT,HYPEUSDT,MYXUSDT,PORT3USDT,RAVEUSDT,RIVERUSDT,RVVUSDT,SIRENUSDT,SUIUSDT,TRXUSDT,USDCUSDT,XRPUSDT,ZBTUSDT,ZECUSDT 3

echo '▶ futures 1h · artımlı (bayat-kuyruk) · 22 sembol'
cargo run --release -p memos_trading_core --example download_candles -- futures 1h AGIXUSDT,AXSUSDT,BANDUSDT,BATUSDT,BLESSUSDT,CLUSDT,ETHWUSDT,FILUSDT,LINKUSDT,MSTRUSDT,ORDIUSDT,PORTALUSDT,RAREUSDT,SXPUSDT,TRUMPUSDT,TSLAUSDT,UXLINKUSDT,VIDTUSDT,WLDUSDT,XAGUSDT,XAUUSDT,币安人生USDT 1

echo '▶ futures 4h · FORCE_FULL (iç-gap/seyrek) · 10 sembol'
FORCE_FULL=1 cargo run --release -p memos_trading_core --example download_candles -- futures 4h ALPACAUSDT,ALPHAUSDT,BNXUSDT,CLUSDT,HYPEUSDT,PORT3USDT,RVVUSDT,SUIUSDT,USDCUSDT,ZBTUSDT 1

echo '▶ futures 4h · artımlı (bayat-kuyruk) · 28 sembol'
cargo run --release -p memos_trading_core --example download_candles -- futures 4h AGIXUSDT,AXSUSDT,BANDUSDT,BATUSDT,BLESSUSDT,COAIUSDT,CRCLUSDT,DOTUSDT,ETHWUSDT,FILUSDT,HIGHUSDT,LINKUSDT,MSTRUSDT,MYXUSDT,ORDIUSDT,PORTALUSDT,RAREUSDT,RAVEUSDT,RIVERUSDT,SIRENUSDT,SXPUSDT,TRUMPUSDT,TSLAUSDT,VIDTUSDT,WLDUSDT,XAGUSDT,XAUUSDT,币安人生USDT 1

echo '▶ futures 1d · FORCE_FULL (iç-gap/seyrek) · 18 sembol'
FORCE_FULL=1 cargo run --release -p memos_trading_core --example download_candles -- futures 1d ASTERUSDT,ATUSDT,BEATUSDT,BLESSUSDT,COAIUSDT,CRCLUSDT,HYPEUSDT,MSTRUSDT,PORT3USDT,RAVEUSDT,RIVERUSDT,RVVUSDT,TSLAUSDT,XAGUSDT,XAUUSDT,XPLUSDT,ZBTUSDT,币安人生USDT 2

echo '▶ futures 1d · artımlı (bayat-kuyruk) · 56 sembol'
cargo run --release -p memos_trading_core --example download_candles -- futures 1d ALPACAUSDT,ALPHAUSDT,AXSUSDT,BANDUSDT,BATUSDT,BNXUSDT,BTCUSDC,CHZUSDT,COMPUSDT,CRVUSDT,DASHUSDT,DOGEUSDT,DOTUSDT,EGLDUSDT,ENJUSDT,EOSUSDT,ETCUSDT,ETHUSDC,ETHWUSDT,FILUSDT,FLOWUSDT,FTMUSDT,GRTUSDT,HBARUSDT,ICXUSDT,IOTAUSDT,KAVAUSDT,LINKUSDT,LTCUSDT,MANAUSDT,MKRUSDT,MYXUSDT,NEARUSDT,NEOUSDT,OMGUSDT,ONEUSDT,ONTUSDT,ORDIUSDT,PORTALUSDT,QTUMUSDT,RAREUSDT,RUNEUSDT,RVNUSDT,SANDUSDT,SNXUSDT,SOLUSDT,SUIUSDT,SUSHIUSDT,THETAUSDT,TRUMPUSDT,UNIUSDT,USDCUSDT,VETUSDT,XLMUSDT,ZILUSDT,ZRXUSDT 7

echo '✅ backfill tamam — doğrula: cargo run --release -p memos_trading_core --example db_coverage'
