use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Timelike, Utc, Weekday};
use chrono_tz::America::New_York;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntrySessionDecision {
    Allow,
    Closed { reason: String },
}

fn observed_fixed_holiday(year: i32, month: u32, day: u32) -> NaiveDate {
    let date = NaiveDate::from_ymd_opt(year, month, day).expect("valid holiday");
    match date.weekday() {
        Weekday::Sat => date - Duration::days(1),
        Weekday::Sun => date + Duration::days(1),
        _ => date,
    }
}

fn nth_weekday(year: i32, month: u32, weekday: Weekday, ordinal: u32) -> NaiveDate {
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("valid month");
    let offset = (weekday.num_days_from_monday() + 7 - first.weekday().num_days_from_monday())
        % 7;
    first + Duration::days((offset + 7 * (ordinal - 1)) as i64)
}

fn last_weekday(year: i32, month: u32, weekday: Weekday) -> NaiveDate {
    let next_month = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1).expect("valid date")
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1).expect("valid date")
    };
    let mut date = next_month - Duration::days(1);
    while date.weekday() != weekday {
        date -= Duration::days(1);
    }
    date
}

fn easter_sunday(year: i32) -> NaiveDate {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = (h + l - 7 * m + 114) % 31 + 1;
    NaiveDate::from_ymd_opt(year, month as u32, day as u32).expect("valid Easter")
}

fn nyse_holiday(date: NaiveDate) -> bool {
    let year = date.year();
    let easter = easter_sunday(year);
    [
        observed_fixed_holiday(year, 1, 1),
        nth_weekday(year, 1, Weekday::Mon, 3),
        nth_weekday(year, 2, Weekday::Mon, 3),
        easter - Duration::days(2),
        last_weekday(year, 5, Weekday::Mon),
        observed_fixed_holiday(year, 6, 19),
        observed_fixed_holiday(year, 7, 4),
        nth_weekday(year, 9, Weekday::Mon, 1),
        nth_weekday(year, 11, Weekday::Thu, 4),
        observed_fixed_holiday(year, 12, 25),
    ]
    .contains(&date)
}

fn nyse_close_minutes(date: NaiveDate) -> u32 {
    let thanksgiving = nth_weekday(date.year(), 11, Weekday::Thu, 4);
    let black_friday = thanksgiving + Duration::days(1);
    let christmas_eve = NaiveDate::from_ymd_opt(date.year(), 12, 24).expect("valid date");
    if date == black_friday || date == christmas_eve {
        13 * 60
    } else {
        16 * 60
    }
}

fn ibkr_us_equity(now: DateTime<Utc>) -> EntrySessionDecision {
    let local = now.with_timezone(&New_York);
    let date = local.date_naive();
    let minutes = local.hour() * 60 + local.minute();
    let open = 9 * 60 + 30;
    let close = nyse_close_minutes(date);
    if date.weekday() == Weekday::Sat || date.weekday() == Weekday::Sun {
        return EntrySessionDecision::Closed {
            reason: "IBKR/NYSE hafta sonu seansı kapalı".to_string(),
        };
    }
    if nyse_holiday(date) {
        return EntrySessionDecision::Closed {
            reason: format!("IBKR/NYSE tatil takvimi kapalı: {date}"),
        };
    }
    if minutes < open || minutes >= close {
        return EntrySessionDecision::Closed {
            reason: format!(
                "IBKR/NYSE seansı kapalı: New York {:02}:{:02}, pencere 09:30-{close_min:02}:{close_sec:02}",
                local.hour(),
                local.minute(),
                close_min = close / 60,
                close_sec = close % 60
            ),
        };
    }
    EntrySessionDecision::Allow
}

fn ibkr_fx_cfd(now: DateTime<Utc>) -> EntrySessionDecision {
    let weekday = now.weekday();
    let minute = now.hour() * 60 + now.minute();
    let open = 21 * 60;
    let close = 21 * 60;
    let week_minute = weekday.num_days_from_sunday() * 1440 + minute as u32;
    if weekday == Weekday::Sat
        || (weekday == Weekday::Sun && minute < open)
        || (weekday == Weekday::Fri && minute >= close)
        || (weekday != Weekday::Sat
            && weekday != Weekday::Sun
            && week_minute >= 6 * 1440 + close)
    {
        return EntrySessionDecision::Closed {
            reason: "IBKR FX/CFD genel 24/5 seansı kapalı".to_string(),
        };
    }
    EntrySessionDecision::Allow
}

pub fn entry_session_for_symbol(symbol: &str, now: DateTime<Utc>) -> EntrySessionDecision {
    let normalized = symbol.to_ascii_lowercase();
    if normalized.ends_with("@binance") || normalized.starts_with("binance:") {
        return EntrySessionDecision::Allow;
    }
    if normalized.ends_with("@mt5") || normalized.starts_with("mt5:") {
        return ibkr_fx_cfd(now);
    }
    if normalized.ends_with("@ibkr") || normalized.starts_with("ibkr:") {
        let profile = std::env::var("IBKR_SESSION_PROFILE")
            .unwrap_or_else(|_| "us_equity".to_string())
            .to_ascii_lowercase();
        return if profile == "fx" || profile == "cfd" {
            ibkr_fx_cfd(now)
        } else {
            ibkr_us_equity(now)
        };
    }
    if normalized.ends_with("@bist") || normalized.starts_with("bist:") {
        return EntrySessionDecision::Closed {
            reason: "BIST yürütme bağlantısı tanımlı değil; signal-only".to_string(),
        };
    }
    EntrySessionDecision::Closed {
        reason: "Bilinmeyen venue seansı; fail-closed".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    #[test]
    fn binance_is_always_open() {
        assert_eq!(
            entry_session_for_symbol("BTCUSDT@binance", utc("2026-10-04T08:00:00Z")),
            EntrySessionDecision::Allow
        );
    }

    #[test]
    fn ibkr_us_equity_uses_new_york_and_holiday_calendar() {
        assert_eq!(
            entry_session_for_symbol("XLE@ibkr", utc("2026-10-02T15:00:00Z")),
            EntrySessionDecision::Allow
        );
        assert!(matches!(
            entry_session_for_symbol("XLE@ibkr", utc("2026-07-03T15:00:00Z")),
            EntrySessionDecision::Closed { .. }
        ));
    }

    #[test]
    fn unknown_venue_is_fail_closed() {
        assert!(matches!(
            entry_session_for_symbol("ABC@unknown", utc("2026-10-04T08:00:00Z")),
            EntrySessionDecision::Closed { .. }
        ));
    }
}
