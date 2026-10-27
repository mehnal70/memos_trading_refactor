use chrono::{DateTime, Datelike, Timelike, Utc};

pub fn is_holiday(date: chrono::NaiveDate) -> bool {
    std::env::var("BIST_HOLIDAYS")
        .ok()
        .into_iter()
        .flat_map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter_map(|value| chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok())
        .any(|holiday| holiday == date)
}

pub fn is_non_trading_day(date: chrono::NaiveDate) -> bool {
    matches!(
        date.weekday(),
        chrono::Weekday::Sat | chrono::Weekday::Sun
    ) || is_holiday(date)
}

pub fn market_state(now_utc: DateTime<Utc>) -> &'static str {
    let turkey_now = now_utc + chrono::Duration::hours(3);
    if is_holiday(turkey_now.date_naive()) {
        return "MARKET_HOLIDAY";
    }
    if matches!(
        turkey_now.weekday(),
        chrono::Weekday::Sat | chrono::Weekday::Sun
    ) {
        return "MARKET_CLOSED";
    }
    let minutes = turkey_now.hour() * 60 + turkey_now.minute();
    let open = crate::core::env::env_parse_or("BIST_MARKET_OPEN_MINUTES", 10 * 60u64);
    let close = crate::core::env::env_parse_or("BIST_MARKET_CLOSE_MINUTES", 18 * 60u64);
    let grace = crate::core::env::env_parse_or("BIST_MARKET_CLOSE_GRACE_MINS", 30u64);
    if minutes >= open as u32 && minutes <= close.saturating_add(grace) as u32 {
        "SESSION_ACTIVE"
    } else {
        "POST_CLOSE"
    }
}
