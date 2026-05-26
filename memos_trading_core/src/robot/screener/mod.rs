// robot/screener — Otonom sembol tarayıcısı.
//
// `master.rs::run_screener_job` "screener" trigger ateşlendiğinde:
//   1) Aday sembol havuzunu derler (SQLite + SCREENER_EXTRA_SYMBOLS env),
//   2) Her aday için son N mum üzerinden `Backtester` ile hızlı skor çıkartır,
//   3) `select_top_n_diff` ile mevcut orchestrator durumuna göre delta hesaplar,
//   4) Yeni sembolleri `register`, düşürülenleri `stop_symbol` ile uygular.
//
// Bu modül saf yardımcıları (skor + diff) tutar; state mutasyonu master.rs'te
// kalır → engine kurmadan birim test edilebilir.

pub mod score;

pub use score::{score_symbol, select_top_n_diff, HtfBias, ScreenerScore, SelectionDiff};
