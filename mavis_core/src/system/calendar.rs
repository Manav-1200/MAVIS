// mavis_core/src/system/calendar.rs
// Minimal iCalendar reader for Evolution's local calendar.
//
// Deliberately not a full RFC 5545 implementation: it reads VEVENT blocks,
// pulls SUMMARY and DTSTART, and returns the next event still in the future.
// Recurring events (RRULE) are skipped rather than guessed at — handling
// recurrence properly needs a real ics crate, and this covers the common
// "what's my next meeting" case without adding a dependency.

use crate::context_snapshot::CalendarEvent;
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};

/// Evolution keeps its default calendar here as plain iCalendar text.
pub fn default_calendar_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let p = std::path::PathBuf::from(home)
        .join(".local/share/evolution/calendar/system/calendar.ics");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// Read the calendar file and return the soonest event that hasn't started yet.
pub fn next_event() -> Option<CalendarEvent> {
    let path = default_calendar_path()?;
    let raw = std::fs::read_to_string(path).ok()?;
    next_event_from_ics(&raw, Local::now())
}

/// Pure parsing, split out so it can be reasoned about (and tested) without
/// touching the filesystem or the real clock.
pub fn next_event_from_ics(raw: &str, now: DateTime<Local>) -> Option<CalendarEvent> {
    let unfolded = unfold(raw);

    let mut best: Option<(DateTime<Local>, String, bool)> = None;
    let mut summary: Option<String> = None;
    let mut start: Option<(DateTime<Local>, bool)> = None;
    let mut has_rrule = false;
    let mut in_event = false;

    for line in unfolded.lines() {
        let line = line.trim_end_matches('\r');

        if line == "BEGIN:VEVENT" {
            in_event = true;
            summary = None;
            start = None;
            has_rrule = false;
            continue;
        }
        if line == "END:VEVENT" {
            if !has_rrule {
                if let (Some(s), Some((dt, all_day))) = (summary.take(), start.take()) {
                    if dt > now {
                        let better = match &best {
                            Some((best_dt, _, _)) => dt < *best_dt,
                            None => true,
                        };
                        if better {
                            best = Some((dt, s, all_day));
                        }
                    }
                }
            }
            in_event = false;
            continue;
        }
        if !in_event {
            continue;
        }

        // Property names may carry parameters after ';' (e.g. DTSTART;TZID=...).
        let (name_part, value) = match line.split_once(':') {
            Some(v) => v,
            None => continue,
        };
        let name = name_part.split(';').next().unwrap_or("");

        match name {
            "SUMMARY" => summary = Some(value.to_string()),
            "RRULE" => has_rrule = true,
            "DTSTART" => start = parse_dtstart(name_part, value),
            _ => {}
        }
    }

    best.map(|(dt, summary, all_day)| CalendarEvent {
        summary,
        start: dt.to_rfc3339(),
        minutes_until: (dt - now).num_minutes().max(0),
        all_day,
    })
}

/// Unfold RFC 5545 line continuations: a line beginning with a space or tab
/// continues the previous one.
fn unfold(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for line in raw.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            out.push_str(line.trim_start());
        } else {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line);
        }
    }
    out
}

/// Handles the three DTSTART forms that actually show up in practice:
///   DTSTART:20260909T150000Z            (UTC)
///   DTSTART;TZID=Region/City:20260909T150000  (local wall time)
///   DTSTART;VALUE=DATE:20260909         (all-day)
/// TZID is treated as local time rather than resolving the named zone —
/// correct for events created in the machine's own timezone, which is the
/// normal case for a local desktop calendar.
fn parse_dtstart(name_part: &str, value: &str) -> Option<(DateTime<Local>, bool)> {
    let is_date_only = name_part.contains("VALUE=DATE") || value.len() == 8;

    if is_date_only {
        let d = NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        let naive = d.and_hms_opt(0, 0, 0)?;
        return Local.from_local_datetime(&naive).single().map(|dt| (dt, true));
    }

    if let Some(stripped) = value.strip_suffix('Z') {
        let naive = NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S").ok()?;
        let utc = Utc.from_utc_datetime(&naive);
        return Some((utc.with_timezone(&Local), false));
    }

    let naive = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    Local.from_local_datetime(&naive).single().map(|dt| (dt, false))
}