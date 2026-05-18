// TradingMode env-string parser testleri.
// rtc_headless ve rtc_tui ikisi de TRADING_MODE env'inden bu parse'ı çağırır.

use memos_trading_core::core::model::TradingMode;

#[test]
fn parses_live_case_insensitive() {
    assert_eq!(TradingMode::from_env_str("Live"),     TradingMode::Live);
    assert_eq!(TradingMode::from_env_str("LIVE"),     TradingMode::Live);
    assert_eq!(TradingMode::from_env_str("live"),     TradingMode::Live);
    assert_eq!(TradingMode::from_env_str("  Live  "), TradingMode::Live);
}

#[test]
fn parses_backtest_case_insensitive() {
    assert_eq!(TradingMode::from_env_str("Backtest"), TradingMode::Backtest);
    assert_eq!(TradingMode::from_env_str("BACKTEST"), TradingMode::Backtest);
}

#[test]
fn parses_paper_explicit() {
    assert_eq!(TradingMode::from_env_str("Paper"),    TradingMode::Paper);
    assert_eq!(TradingMode::from_env_str("paper"),    TradingMode::Paper);
}

#[test]
fn unknown_falls_back_to_paper_safe_default() {
    // Güvenlik: bilinmeyen değer Live'a YANLIŞLIKLA düşmemeli
    assert_eq!(TradingMode::from_env_str(""),         TradingMode::Paper);
    assert_eq!(TradingMode::from_env_str("xyz"),      TradingMode::Paper);
    assert_eq!(TradingMode::from_env_str("LIVE-typo"), TradingMode::Paper);
    assert_eq!(TradingMode::from_env_str("1"),        TradingMode::Paper);
}

#[test]
fn as_str_round_trip() {
    for mode in [TradingMode::Paper, TradingMode::Live, TradingMode::Backtest] {
        let s = mode.as_str();
        let back = TradingMode::from_env_str(s);
        assert_eq!(mode, back, "as_str ↔ from_env_str round-trip kırık: {:?}", mode);
    }
}
