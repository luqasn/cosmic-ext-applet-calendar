// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

pub mod backend;
pub mod cache;
pub mod eds;
pub mod error;
pub mod ical;
pub mod meeting;

pub use backend::{CalendarBackend, CompositeBackend, LocalIcsBackend, MockBackend};
pub use cache::EventCache;
pub use eds::EdsBackend;
pub use error::CalendarError;

use jiff::{ToSpan, Zoned, civil::Date};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarEvent {
    pub id: String,
    pub summary: String,
    pub start: Zoned,
    pub end: Zoned,
    pub is_all_day: bool,
    pub location: Option<String>,
    pub url: Option<String>,
    /// Joinable online meeting link, restricted to [`meeting::MEETING_PROVIDERS`].
    pub meeting_url: Option<String>,
}

/// Validates that a URL is strictly an HTTP or HTTPS web link.
/// Rejects dangerous schemes like javascript:, file:, data:, or shell commands.
pub fn is_safe_web_url(raw_url: &str) -> bool {
    let trimmed = raw_url.trim();
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return false;
    }

    if let Ok(parsed) = url::Url::parse(trimmed) {
        (parsed.scheme() == "https" || parsed.scheme() == "http") && parsed.host().is_some()
    } else {
        false
    }
}

/// Calculates all dates covered by an event.
/// In RFC 5545, for all-day events (or date-only ranges), DTEND is exclusive: [start, end).
pub fn covered_dates(event: &CalendarEvent) -> Vec<Date> {
    let start_date = event.start.date();
    let mut end_date = event.end.date();

    if end_date < start_date {
        end_date = start_date;
    }

    if event.is_all_day {
        // RFC 5545 exclusive DTEND rule: if DTEND is next day at 00:00,
        // the covered end date is the day before.
        if end_date > start_date
            && let Ok(prev) = end_date.checked_sub(1.days())
        {
            end_date = prev;
        }
    } else if event.end.time() == jiff::civil::time(0, 0, 0, 0) && event.end > event.start {
        // Timed event ending exactly at midnight (00:00:00) of the subsequent day.
        // Under RFC 5545 and calendar UX, midnight marks the end of the prior day,
        // so it must not spill over to the subsequent date.
        if end_date > start_date
            && let Ok(prev) = end_date.checked_sub(1.days())
        {
            end_date = prev;
        }
    }

    let mut dates = Vec::new();
    let mut curr = start_date;
    while curr <= end_date {
        dates.push(curr);
        if let Ok(next) = curr.checked_add(1.days()) {
            curr = next;
        } else {
            break;
        }
    }
    dates
}

/// Returns a set of all unique dates in the given events slice.
pub fn covered_dates_for_events(events: &[CalendarEvent]) -> HashSet<Date> {
    let mut set = HashSet::new();
    for ev in events {
        for d in covered_dates(ev) {
            set.insert(d);
        }
    }
    set
}

/// Filters and returns all events that are active on the specified date,
/// sorted chronologically (all-day events first, then by start time).
pub fn filter_events_for_date(events: &[CalendarEvent], date: Date) -> Vec<CalendarEvent> {
    let mut filtered: Vec<CalendarEvent> = events
        .iter()
        .filter(|ev| covered_dates(ev).contains(&date))
        .cloned()
        .collect();

    filtered.sort_by(|a, b| match (a.is_all_day, b.is_all_day) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.start.cmp(&b.start),
    });

    filtered
}

/// Returns the meeting to show in the panel: a timed (non all-day) event that is in
/// progress right now, or otherwise the earliest one that starts later today or tomorrow.
/// If several meetings overlap, the most recently started one wins.
pub fn current_or_next_event<'a>(
    events: &'a [CalendarEvent],
    now: &Zoned,
) -> Option<&'a CalendarEvent> {
    let timed = || events.iter().filter(|ev| !ev.is_all_day);

    if let Some(current) = timed()
        .filter(|ev| ev.start <= *now && *now < ev.end)
        .max_by(|a, b| a.start.cmp(&b.start))
    {
        return Some(current);
    }

    let tomorrow = now.date().tomorrow().ok()?;
    timed()
        .filter(|ev| ev.start > *now)
        .filter(|ev| ev.start.with_time_zone(now.time_zone().clone()).date() <= tomorrow)
        .min_by(|a, b| a.start.cmp(&b.start))
}

/// Number of whole minutes until `start`, rounded up so that a meeting 30 seconds
/// away still reads as one minute rather than zero.
pub fn minutes_until(now: &Zoned, start: &Zoned) -> i64 {
    let secs = start.timestamp().as_second() - now.timestamp().as_second();
    if secs <= 0 { 0 } else { (secs + 59) / 60 }
}

/// Formats a countdown in minutes as a compact string such as `5m` or `1h 05m`.
/// Returns `None` when the meeting has started.
pub fn format_countdown(minutes: i64) -> Option<String> {
    if minutes < 1 {
        None
    } else if minutes < 60 {
        Some(format!("{minutes}m"))
    } else {
        Some(format!("{}h {:02}m", minutes / 60, minutes % 60))
    }
}

/// Generates mock events for the given year and month for prototype and verification.
pub fn mock_events_for_month(year: i16, month: i8) -> Vec<CalendarEvent> {
    use jiff::civil::{date, time};
    let tz = jiff::tz::TimeZone::system();
    let mut events = Vec::new();

    // Event 1: Morning meeting on 5th of the month
    if let Ok(d5) = date(year, month, 5).to_zoned(tz.clone())
        && let (Ok(start), Ok(end)) = (
            d5.with().time(time(10, 0, 0, 0)).build(),
            d5.with().time(time(11, 0, 0, 0)).build(),
        )
    {
        events.push(CalendarEvent {
            id: "mock-1".to_string(),
            summary: "COSMIC Team Sync".to_string(),
            start,
            end,
            is_all_day: false,
            location: Some("Online".to_string()),
            url: None,
            meeting_url: Some("https://meet.google.com/abc-defg-hij".to_string()),
        });
    }

    // Event 2: All-day event on 15th (RFC 5545 DTEND is 16th exclusive)
    if let Ok(d15) = date(year, month, 15).to_zoned(tz.clone()) {
        let d16 = date(year, month, 16)
            .to_zoned(tz.clone())
            .unwrap_or_else(|_| d15.clone());
        events.push(CalendarEvent {
            id: "mock-2".to_string(),
            summary: "Pop!_OS Release Planning".to_string(),
            start: d15,
            end: d16,
            is_all_day: true,
            location: None,
            url: None,
            meeting_url: None,
        });
    }

    // Event 3: Afternoon meeting on 20th of the month
    if let Ok(d20) = date(year, month, 20).to_zoned(tz)
        && let (Ok(start), Ok(end)) = (
            d20.with().time(time(14, 30, 0, 0)).build(),
            d20.with().time(time(15, 30, 0, 0)).build(),
        )
    {
        events.push(CalendarEvent {
            id: "mock-3".to_string(),
            summary: "Architecture Review".to_string(),
            start,
            end,
            is_all_day: false,
            location: Some("Meeting Room B".to_string()),
            url: Some("https://zoom.us/j/123456789".to_string()),
            meeting_url: None,
        });
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn test_is_safe_web_url() {
        assert!(is_safe_web_url("https://meet.google.com/abc-defg-hij"));
        assert!(is_safe_web_url("http://localhost:8080/meeting"));

        // Dangerous or invalid schemes must be rejected
        assert!(!is_safe_web_url("javascript:alert(1)"));
        assert!(!is_safe_web_url("file:///etc/passwd"));
        assert!(!is_safe_web_url("data:text/html,<script>alert(1)</script>"));
        assert!(!is_safe_web_url("sh -c 'rm -rf /'"));
        assert!(!is_safe_web_url("not a url"));
        assert!(!is_safe_web_url("https://"));
    }

    #[test]
    fn test_allday_exclusive_dtend() {
        let tz = jiff::tz::TimeZone::UTC;
        let start = date(2026, 3, 5).to_zoned(tz.clone()).unwrap();
        let end = date(2026, 3, 6).to_zoned(tz).unwrap();

        let event = CalendarEvent {
            id: "all-day-test".to_string(),
            summary: "Single Day All-Day".to_string(),
            start,
            end,
            is_all_day: true,
            location: None,
            url: None,
            meeting_url: None,
        };

        let dates = covered_dates(&event);
        assert_eq!(dates, vec![date(2026, 3, 5)]);
    }

    fn timed_event(id: &str, start: Zoned, minutes: i64) -> CalendarEvent {
        CalendarEvent {
            id: id.to_string(),
            summary: id.to_string(),
            end: start.checked_add(minutes.minutes()).unwrap(),
            start,
            is_all_day: false,
            location: None,
            url: None,
            meeting_url: None,
        }
    }

    fn at(d: Date, h: i8, m: i8) -> Zoned {
        d.at(h, m, 0, 0).to_zoned(jiff::tz::TimeZone::UTC).unwrap()
    }

    #[test]
    fn test_current_or_next_event() {
        let today = date(2026, 10, 6);
        let all_day = CalendarEvent {
            id: "all-day".to_string(),
            summary: "all-day".to_string(),
            start: today.to_zoned(jiff::tz::TimeZone::UTC).unwrap(),
            end: date(2026, 10, 7).to_zoned(jiff::tz::TimeZone::UTC).unwrap(),
            is_all_day: true,
            location: None,
            url: None,
            meeting_url: None,
        };
        let events = vec![
            all_day,
            timed_event("past", at(today, 8, 0), 30),
            timed_event("in-progress", at(today, 9, 45), 30),
            timed_event("later", at(today, 15, 0), 30),
            timed_event("soon", at(today, 10, 30), 30),
            timed_event("tomorrow", at(date(2026, 10, 7), 9, 0), 30),
        ];

        // A meeting in progress is shown, even with another one coming up.
        let current = current_or_next_event(&events, &at(today, 10, 0)).unwrap();
        assert_eq!(current.id, "in-progress");

        // Once it ends, the next upcoming meeting is shown.
        let next = current_or_next_event(&events, &at(today, 10, 15)).unwrap();
        assert_eq!(next.id, "soon");

        // Still shown at its exact start time and until it ends.
        let started = current_or_next_event(&events, &at(today, 10, 30)).unwrap();
        assert_eq!(started.id, "soon");
        let ending = current_or_next_event(&events, &at(today, 10, 59)).unwrap();
        assert_eq!(ending.id, "soon");

        // Nothing left today, so tomorrow's first meeting is shown.
        let tomorrow = current_or_next_event(&events, &at(today, 16, 0)).unwrap();
        assert_eq!(tomorrow.id, "tomorrow");

        // Meetings further out than tomorrow are not shown.
        let day_after_only = vec![timed_event("day-after", at(date(2026, 10, 8), 8, 0), 30)];
        assert!(current_or_next_event(&day_after_only, &at(today, 16, 0)).is_none());
    }

    #[test]
    fn test_overlapping_meetings_prefer_most_recent_start() {
        let today = date(2026, 10, 6);
        let events = vec![
            timed_event("long", at(today, 9, 0), 180),
            timed_event("short", at(today, 10, 0), 30),
        ];
        let current = current_or_next_event(&events, &at(today, 10, 10)).unwrap();
        assert_eq!(current.id, "short");
    }

    #[test]
    fn test_minutes_until_rounds_up() {
        let today = date(2026, 10, 6);
        let start = at(today, 10, 30);
        assert_eq!(minutes_until(&at(today, 10, 0), &start), 30);
        let almost = today
            .at(10, 29, 30, 0)
            .to_zoned(jiff::tz::TimeZone::UTC)
            .unwrap();
        assert_eq!(minutes_until(&almost, &start), 1);
        assert_eq!(minutes_until(&at(today, 10, 31), &start), 0);
    }

    #[test]
    fn test_format_countdown() {
        assert_eq!(format_countdown(0), None);
        assert_eq!(format_countdown(1).as_deref(), Some("1m"));
        assert_eq!(format_countdown(59).as_deref(), Some("59m"));
        assert_eq!(format_countdown(60).as_deref(), Some("1h 00m"));
        assert_eq!(format_countdown(125).as_deref(), Some("2h 05m"));
    }

    #[test]
    fn test_midnight_ending_event_does_not_spill_over() {
        use jiff::civil::time;
        let tz = jiff::tz::TimeZone::UTC;
        let start = date(2026, 6, 20)
            .to_zoned(tz.clone())
            .unwrap()
            .with()
            .time(time(23, 0, 0, 0))
            .build()
            .unwrap();
        let end = date(2026, 6, 21)
            .to_zoned(tz)
            .unwrap()
            .with()
            .time(time(0, 0, 0, 0))
            .build()
            .unwrap();

        let event = CalendarEvent {
            id: "midnight-test".to_string(),
            summary: "Late Night Sync".to_string(),
            start,
            end,
            is_all_day: false,
            location: None,
            url: None,
            meeting_url: None,
        };

        let dates = covered_dates(&event);
        // Must strictly cover only June 20, NOT June 21
        assert_eq!(dates, vec![date(2026, 6, 20)]);
    }
}
