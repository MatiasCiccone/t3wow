//! When a snooze ends, worked out on this PC's clock and in its time zone (the game's clock
//! may not be the PC's): a time typed in the game ("45m", "5pm", "fri 3pm"), or one of T3's
//! snooze presets (Settings > Snooze options), labelled as T3 labels them.
//!
//! The typed-time reading follows t3code-wow's `bridge/src/snooze.js` (MIT, Ryan Hughes,
//! https://github.com/ryanrhughes/t3code-wow/pull/18).

use regex::Regex;
use serde_json::{json, Value};
use std::sync::LazyLock;

/// A time zone: its offset from UTC, in seconds, at an instant (epoch seconds).
pub type Zone = dyn Fn(i64) -> i64;

const DAY: i64 = 86_400;
const WEEKDAYS: [&str; 7] = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Days since 1970-01-01 from a civil date (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, mo: i64, d: i64) -> i64 {
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date (year, month, day) of a day since 1970-01-01.
pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(mo <= 2), mo, d)
}

/// 0 for Sunday.
fn weekday_of(days: i64) -> i64 {
    (days + 4).rem_euclid(7)
}

/// This PC's offset from UTC at `utc`, daylight saving included.
#[cfg(windows)]
pub fn system_offset(utc: i64) -> i64 {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::Time::SystemTimeToTzSpecificLocalTime;
    let days = utc.div_euclid(DAY);
    let rest = utc.rem_euclid(DAY);
    let (y, mo, d) = civil_from_days(days);
    let Ok(year) = u16::try_from(y) else { return 0 };
    let universal = SYSTEMTIME {
        wYear: year,
        wMonth: mo as u16,
        wDayOfWeek: 0,
        wDay: d as u16,
        wHour: (rest / 3600) as u16,
        wMinute: (rest % 3600 / 60) as u16,
        wSecond: (rest % 60) as u16,
        wMilliseconds: 0,
    };
    let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
    if unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &universal, &mut local) } == 0 {
        return 0;
    }
    let local_days = days_from_civil(i64::from(local.wYear), i64::from(local.wMonth), i64::from(local.wDay));
    let local_seconds = local_days * DAY + i64::from(local.wHour) * 3600 + i64::from(local.wMinute) * 60 + i64::from(local.wSecond);
    local_seconds - utc
}

/// This Mac's offset from UTC at `utc`, daylight saving included, from the time zone set
/// in System Settings (or `TZ`).
#[cfg(target_os = "macos")]
pub fn system_offset(utc: i64) -> i64 {
    let at: libc::time_t = utc;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&at, &mut tm) }.is_null() {
        return 0;
    }
    tm.tm_gmtoff
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn system_offset(_utc: i64) -> i64 {
    0
}

/// A local wall-clock time (seconds since 1970-01-01 on the local calendar) as epoch seconds.
fn local_to_utc(local: i64, zone: &Zone) -> i64 {
    let guess = local - zone(local);
    local - zone(guess)
}

/// "5:00 PM".
fn clock_text(local: i64) -> String {
    let minutes = local.rem_euclid(DAY) / 60;
    let (h, m) = (minutes / 60, minutes % 60);
    let twelve = if h % 12 == 0 { 12 } else { h % 12 };
    format!("{twelve}:{m:02} {}", if h < 12 { "AM" } else { "PM" })
}

fn short_weekday(days: i64) -> String {
    let name = WEEKDAYS[weekday_of(days) as usize];
    capitalized(&name[..3])
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| first.to_ascii_uppercase().to_string() + chars.as_str())
}

/// When a snooze ends, as the game says it: "5:00 PM" today, "Fri 3:00 PM" within the week,
/// else "Dec 1 3:00 PM".
pub fn describe(until: i64, now: i64, zone: &Zone) -> String {
    let (local, local_now) = (until + zone(until), now + zone(now));
    let (day, today) = (local.div_euclid(DAY), local_now.div_euclid(DAY));
    let time = clock_text(local);
    if day == today {
        time
    } else if until - now < 6 * DAY {
        format!("{} {time}", short_weekday(day))
    } else {
        let (_, mo, d) = civil_from_days(day);
        format!("{} {d} {time}", MONTHS[(mo - 1) as usize])
    }
}

/// Why a typed time wasn't taken.
#[derive(Debug, PartialEq, Eq)]
pub enum SnoozeError {
    Unreadable,
    Passed,
    TooSoon,
    TooFar,
}

impl SnoozeError {
    /// What the game says about `text`.
    pub fn message(&self, text: &str) -> String {
        let shown: String = text.trim().chars().take(40).collect();
        match self {
            SnoozeError::Unreadable => format!("couldn't read \"{shown}\" as a time: try 45m, 2h, 3d, 5pm, tomorrow 9am or fri 3pm"),
            SnoozeError::Passed => format!("\"{shown}\" has already passed: give a time ahead"),
            SnoozeError::TooSoon => "a snooze needs at least a minute".into(),
            SnoozeError::TooFar => format!("\"{shown}\" is more than a year away"),
        }
    }
}

/// A typed time, read but not yet placed: a length of time (seconds), or a local wall-clock
/// time (seconds on the local calendar).
#[derive(Debug, PartialEq, Eq)]
enum Typed {
    After(i64),
    At(i64),
}

static AMPM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([ap])\.m\.?").unwrap());
static PUNCTUATION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[,.]+(\s|$)").unwrap());
const UNITS: &str = r"(minutes?|mins?|m|hours?|hrs?|h|days?|d|weeks?|wks?|w)";
static LENGTH: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"^(?:\d+(?:\.\d+)?\s*{UNITS}\s*)+$")).unwrap());
static LENGTH_PART: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"(\d+(?:\.\d+)?)\s*{UNITS}")).unwrap());
static CLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{1,2})(?::(\d{2}))?\s*(am|pm)?$").unwrap());
static DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{4})-(\d{2})-(\d{2})$").unwrap());

/// "5pm", "5:30 pm", "17:00", "noon", "midnight" as minutes into the day. A bare number
/// isn't a time.
fn parse_clock(text: &str) -> Option<i64> {
    match text {
        "noon" => return Some(12 * 60),
        "midnight" => return Some(0),
        _ => {}
    }
    let caps = CLOCK.captures(text)?;
    if caps.get(2).is_none() && caps.get(3).is_none() {
        return None;
    }
    let mut hours: i64 = caps[1].parse().ok()?;
    let minutes: i64 = caps.get(2).map_or(Some(0), |m| m.as_str().parse().ok())?;
    if let Some(half) = caps.get(3) {
        if !(1..=12).contains(&hours) {
            return None;
        }
        hours = hours % 12 + if half.as_str() == "pm" { 12 } else { 0 };
    }
    (hours < 24 && minutes < 60).then_some(hours * 60 + minutes)
}

fn weekday_named(word: &str) -> Option<i64> {
    let n = match word {
        "sun" | "sunday" => 0,
        "mon" | "monday" => 1,
        "tue" | "tues" | "tuesday" => 2,
        "wed" | "weds" | "wednesday" => 3,
        "thu" | "thur" | "thurs" | "thursday" => 4,
        "fri" | "friday" => 5,
        "sat" | "saturday" => 6,
        _ => return None,
    };
    Some(n)
}

/// Reads a typed time at `local_now` (local calendar seconds): a length of time ("45m",
/// "3 hours", "1h30m", "2d"), a clock time ("5pm", "17:30": today, or tomorrow once that
/// has passed), or a day with an optional time ("tomorrow", "fri 3pm", "2026-11-14 09:00";
/// 9:00 when no time is given, 20:00 for "tonight"). A weekday whose time has passed today
/// means next week's.
fn parse_typed(text: &str, local_now: i64) -> Result<Typed, SnoozeError> {
    let lowered = text.to_lowercase();
    let lowered = AMPM.replace_all(&lowered, "${1}m");
    let lowered = PUNCTUATION.replace_all(&lowered, " ");
    let words: Vec<&str> = lowered
        .split_whitespace()
        .filter(|w| !matches!(*w, "in" | "at" | "for" | "until" | "till" | "on"))
        .map(|w| if matches!(w, "a" | "an") { "1" } else { w })
        .collect();
    let s = words.join(" ");
    if s.is_empty() {
        return Err(SnoozeError::Unreadable);
    }

    if LENGTH.is_match(&s) {
        let mut seconds = 0.0;
        for part in LENGTH_PART.captures_iter(&s) {
            let amount: f64 = part[1].parse().map_err(|_| SnoozeError::Unreadable)?;
            let unit = match &part[2][..1] {
                "m" => 60.0,
                "h" => 3600.0,
                "d" => DAY as f64,
                _ => 7.0 * DAY as f64,
            };
            seconds += amount * unit;
        }
        if seconds < 60.0 {
            return Err(SnoozeError::TooSoon);
        }
        // Far past a year either way: kept in range so it reads as too far, not wrapped.
        return Ok(Typed::After(seconds.min(1e12).round() as i64));
    }

    let today = local_now.div_euclid(DAY);
    let first = words[0];
    let rest = words[1..].join(" ");
    let weekday = weekday_named(first);
    let (day, explicit_day, default_clock) = if first == "today" {
        (today, true, 9 * 60)
    } else if first == "tonight" {
        (today, true, 20 * 60)
    } else if first == "tomorrow" {
        (today + 1, true, 9 * 60)
    } else if let Some(caps) = DATE.captures(first) {
        let (y, mo, d): (i64, i64, i64) = (caps[1].parse().unwrap_or(0), caps[2].parse().unwrap_or(0), caps[3].parse().unwrap_or(0));
        let day = days_from_civil(y, mo, d);
        if !(1..=12).contains(&mo) || civil_from_days(day) != (y, mo, d) {
            return Err(SnoozeError::Unreadable);
        }
        (day, true, 9 * 60)
    } else if let Some(n) = weekday {
        (today + (n - weekday_of(today)).rem_euclid(7), true, 9 * 60)
    } else {
        (today, false, 0)
    };
    let clock_text = if explicit_day { rest } else { s.clone() };
    let minutes = if clock_text.is_empty() { Some(default_clock) } else { parse_clock(&clock_text) };
    let minutes = minutes.ok_or(SnoozeError::Unreadable)?;
    let mut at = day * DAY + minutes * 60;
    if at <= local_now {
        if !explicit_day {
            at += DAY;
        } else if weekday.is_some() {
            at += 7 * DAY;
        } else {
            return Err(SnoozeError::Passed);
        }
    }
    Ok(Typed::At(at))
}

/// When a snooze typed in the game ends (epoch seconds), at `now` on `zone`'s clock: ahead
/// of now, and no more than `max_minutes` on.
pub fn resolve_typed(text: &str, now: i64, zone: &Zone, max_minutes: i64) -> Result<i64, SnoozeError> {
    let until = match parse_typed(text, now + zone(now))? {
        Typed::After(seconds) => now.saturating_add(seconds),
        Typed::At(local) => local_to_utc(local, zone),
    };
    if until <= now {
        return Err(SnoozeError::Passed);
    }
    if until - now > max_minutes * 60 {
        return Err(SnoozeError::TooFar);
    }
    Ok(until)
}

// ---- T3's presets ----

/// T3 Code's own presets, when its settings have none (its DEFAULT_SNOOZE_PRESETS, 0.1.9).
pub fn t3_default_presets() -> Vec<Value> {
    vec![
        json!({ "kind": "after", "minutes": 60 }),
        json!({ "kind": "after", "minutes": 180 }),
        json!({ "kind": "at", "day": "today", "time": "18:00" }),
        json!({ "kind": "at", "day": "tomorrow", "time": "09:00" }),
        json!({ "kind": "at", "day": "monday", "time": "09:00" }),
    ]
}

/// What the game offered before it read T3's presets: for a T3 whose settings this PC
/// can't read.
pub fn fixed_presets() -> Vec<Value> {
    [60, 240, 1440, 10_080].iter().map(|m| json!({ "kind": "after", "minutes": m })).collect()
}

/// T3 keeps at most this many presets.
const MAX_PRESETS: usize = 8;
/// T3's longest relative preset: 30 days.
const MAX_PRESET_MINUTES: i64 = 43_200;

/// One preset as the game offers it.
#[derive(Debug, PartialEq, Eq)]
pub struct Choice {
    /// How the game names it back ("after:60", "at:monday:09:00").
    pub key: String,
    pub label: String,
    /// The clock time, for one at a fixed time; none for a length of time, whose time
    /// moves every minute.
    pub when: Option<String>,
    pub until: i64,
}

fn plural(n: i64, unit: &str) -> String {
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// A preset as `(day, minutes into it)` for "at", or minutes for "after"; None if malformed.
enum Preset<'a> {
    After(i64),
    At(&'a str, i64),
}

fn preset_of(preset: &Value) -> Option<Preset<'_>> {
    match preset["kind"].as_str()? {
        "after" => preset["minutes"].as_i64().filter(|m| (1..=MAX_PRESET_MINUTES).contains(m)).map(Preset::After),
        "at" => {
            let day = preset["day"].as_str().filter(|d| *d == "today" || *d == "tomorrow" || WEEKDAYS.contains(d))?;
            let time = preset["time"].as_str().filter(|t| t.len() == 5 && t.as_bytes()[2] == b':')?;
            let (h, m): (i64, i64) = (time[..2].parse().ok()?, time[3..].parse().ok()?);
            (time[..2].bytes().all(|b| b.is_ascii_digit()) && time[3..].bytes().all(|b| b.is_ascii_digit()) && h < 24 && m < 60).then_some(Preset::At(day, h * 60 + m))
        }
        _ => None,
    }
}

/// A preset as a choice at `now`, as T3's menu shows it; None once a time today has passed,
/// or for a preset T3 wouldn't take.
pub fn preset_choice(preset: &Value, now: i64, zone: &Zone) -> Option<Choice> {
    match preset_of(preset)? {
        Preset::After(minutes) => {
            let length = if minutes % 1440 == 0 {
                plural(minutes / 1440, "day")
            } else if minutes % 60 == 0 {
                plural(minutes / 60, "hour")
            } else {
                plural(minutes, "minute")
            };
            Some(Choice { key: format!("after:{minutes}"), label: format!("In {length}"), when: None, until: now + minutes * 60 })
        }
        Preset::At(day, minutes) => {
            let local_now = now + zone(now);
            let today = local_now.div_euclid(DAY);
            // A weekday that is today means next week's.
            let offset = match day {
                "today" => 0,
                "tomorrow" => 1,
                _ => {
                    let n = WEEKDAYS.iter().position(|w| *w == day)? as i64;
                    match (n - weekday_of(today)).rem_euclid(7) {
                        0 => 7,
                        days => days,
                    }
                }
            };
            let local = (today + offset) * DAY + minutes * 60;
            if local <= local_now {
                return None;
            }
            let label = match offset {
                0 if minutes < 12 * 60 => "This morning".to_string(),
                0 if minutes < 17 * 60 => "This afternoon".to_string(),
                0 => "This evening".to_string(),
                1 => "Tomorrow".to_string(),
                _ if day == "monday" => "Next week".to_string(),
                7 => format!("Next {}", capitalized(day)),
                _ => capitalized(day),
            };
            let time = clock_text(local);
            let when = if label == "Next week" { format!("{} {time}", short_weekday(today + offset)) } else { time };
            let key = format!("at:{day}:{:02}:{:02}", minutes / 60, minutes % 60);
            Some(Choice { key, label, when: Some(when), until: local_to_utc(local, zone) })
        }
    }
}

/// The choices to offer at `now`, soonest first, without two that land on the same minute.
pub fn choices(presets: &[Value], now: i64, zone: &Zone) -> Vec<Choice> {
    let mut list: Vec<Choice> = presets.iter().take(MAX_PRESETS).filter_map(|p| preset_choice(p, now, zone)).collect();
    list.sort_by_key(|c| c.until);
    list.dedup_by_key(|c| c.until.div_euclid(60));
    list
}

/// The preset a choice's key names ("after:60", "at:monday:09:00").
pub fn preset_from_key(key: &str) -> Option<Value> {
    if let Some(minutes) = key.strip_prefix("after:") {
        return Some(json!({ "kind": "after", "minutes": minutes.parse::<i64>().ok()? }));
    }
    let (day, time) = key.strip_prefix("at:")?.split_once(':')?;
    Some(json!({ "kind": "at", "day": day, "time": time }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tuesday 2026-11-10, 14:30 on a clock five hours behind UTC.
    const OFFSET: i64 = -5 * 3600;
    fn zone(_: i64) -> i64 {
        OFFSET
    }
    fn local(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> i64 {
        days_from_civil(y, mo, d) * DAY + h * 3600 + mi * 60
    }
    fn now() -> i64 {
        local(2026, 11, 10, 14, 30) - OFFSET
    }
    /// (day of month, hour, minute) on the local clock, or the error.
    fn at(text: &str) -> Result<(i64, i64, i64), SnoozeError> {
        resolve_typed(text, now(), &zone, 525_600).map(|until| {
            let local = until + OFFSET;
            let (_, _, d) = civil_from_days(local.div_euclid(DAY));
            (d, local.rem_euclid(DAY) / 3600, local.rem_euclid(3600) / 60)
        })
    }

    #[test]
    fn the_test_clock_is_a_tuesday_afternoon() {
        assert_eq!(weekday_of((now() + OFFSET).div_euclid(DAY)), 2);
    }

    #[test]
    fn a_length_of_time_counts_from_now() {
        assert_eq!(at("45m"), Ok((10, 15, 15)));
        assert_eq!(at("2h"), Ok((10, 16, 30)));
        assert_eq!(at("3d"), Ok((13, 14, 30)));
        assert_eq!(at("3 hours"), Ok((10, 17, 30)));
        assert_eq!(at("in 2 hours"), Ok((10, 16, 30)));
        assert_eq!(at("1h30m"), Ok((10, 16, 0)));
        assert_eq!(at("1.5h"), Ok((10, 16, 0)));
        assert_eq!(at("an hour"), Ok((10, 15, 30)));
        assert_eq!(at("1 week"), Ok((17, 14, 30)));
        assert_eq!(at("90 MINS"), Ok((10, 16, 0)));
    }

    #[test]
    fn a_clock_time_is_later_today_or_else_tomorrow() {
        assert_eq!(at("5pm"), Ok((10, 17, 0)));
        assert_eq!(at("5:30 PM"), Ok((10, 17, 30)));
        assert_eq!(at("at 5 p.m."), Ok((10, 17, 0)));
        assert_eq!(at("17:45"), Ok((10, 17, 45)));
        // Already past today: tomorrow's.
        assert_eq!(at("9am"), Ok((11, 9, 0)));
        assert_eq!(at("2:30pm"), Ok((11, 14, 30)), "the minute it is now has passed");
        assert_eq!(at("2:31pm"), Ok((10, 14, 31)));
        assert_eq!(at("noon"), Ok((11, 12, 0)));
        assert_eq!(at("midnight"), Ok((11, 0, 0)));
    }

    #[test]
    fn a_day_with_or_without_a_time() {
        assert_eq!(at("tomorrow"), Ok((11, 9, 0)));
        assert_eq!(at("tomorrow 9am"), Ok((11, 9, 0)));
        assert_eq!(at("Tomorrow at 7:15 a.m."), Ok((11, 7, 15)));
        assert_eq!(at("tonight"), Ok((10, 20, 0)));
        assert_eq!(at("today 6pm"), Ok((10, 18, 0)));
        assert_eq!(at("2026-11-14 09:00"), Ok((14, 9, 0)));
        assert_eq!(at("2026-11-14"), Ok((14, 9, 0)));
    }

    #[test]
    fn a_weekday_is_the_next_one_and_rolls_to_next_week_once_passed() {
        assert_eq!(at("fri 3pm"), Ok((13, 15, 0)));
        assert_eq!(at("friday"), Ok((13, 9, 0)));
        assert_eq!(at("mon 9am"), Ok((16, 9, 0)), "past the weekend into next week");
        assert_eq!(at("sun noon"), Ok((15, 12, 0)));
        // Today is Tuesday: today while still ahead, else a week on.
        assert_eq!(at("tuesday 4pm"), Ok((10, 16, 0)));
        assert_eq!(at("tue 9am"), Ok((17, 9, 0)));
        assert_eq!(at("tues"), Ok((17, 9, 0)));
    }

    #[test]
    fn a_weekday_rolls_over_the_end_of_the_month() {
        // Saturday 2026-10-31, 22:00: Monday is November 2nd.
        let now = local(2026, 10, 31, 22, 0) - OFFSET;
        let until = resolve_typed("mon 8am", now, &zone, 525_600).unwrap() + OFFSET;
        assert_eq!(civil_from_days(until.div_euclid(DAY)), (2026, 11, 2));
        assert_eq!(until.rem_euclid(DAY), 8 * 3600);
    }

    #[test]
    fn bad_input_says_why() {
        for bad in ["", "   ", "soon", "5", "13pm", "0pm", "25:00", "5:75pm", "2026-02-31", "2026-13-01", "fri 3pm please", "next fri", "1 month", "h"] {
            assert_eq!(at(bad), Err(SnoozeError::Unreadable), "{bad:?}");
        }
        for passed in ["today 1pm", "today", "2026-11-09 09:00", "2026-11-10 14:00"] {
            assert_eq!(at(passed), Err(SnoozeError::Passed), "{passed:?}");
        }
        assert_eq!(at("0m"), Err(SnoozeError::TooSoon));
        assert_eq!(at("30 seconds"), Err(SnoozeError::Unreadable));
        assert_eq!(at("53w"), Err(SnoozeError::TooFar));
        assert_eq!(at("2027-12-01"), Err(SnoozeError::TooFar));
        assert_eq!(at("99999999999999999999d"), Err(SnoozeError::TooFar));
        assert_eq!(at("52w"), Ok((9, 14, 30)));
        assert!(SnoozeError::Unreadable.message("soon").contains("\"soon\""));
        assert!(SnoozeError::Passed.message("today 1pm").contains("passed"));
    }

    #[test]
    fn a_wall_clock_time_keeps_to_daylight_saving() {
        // UTC-4 until 2026-11-01 06:00 UTC, UTC-5 after (US Eastern).
        let switch = local(2026, 11, 1, 6, 0);
        let eastern = move |utc: i64| if utc < switch { -4 * 3600 } else { -5 * 3600 };
        // Saturday Oct 31, 20:00 local (EDT): "mon 9am" is 09:00 EST, 14:00 UTC.
        let now = local(2026, 10, 31, 20, 0) + 4 * 3600;
        assert_eq!(resolve_typed("mon 9am", now, &eastern, 525_600), Ok(local(2026, 11, 2, 14, 0)));
        // A length of time is a length, whatever the clock does.
        assert_eq!(resolve_typed("1d", now, &eastern, 525_600), Ok(now + DAY));
    }

    #[test]
    fn t3s_default_presets_read_as_its_menu_does() {
        let list = choices(&t3_default_presets(), now(), &zone);
        let shown: Vec<(&str, &str, Option<&str>)> = list.iter().map(|c| (c.key.as_str(), c.label.as_str(), c.when.as_deref())).collect();
        assert_eq!(
            shown,
            vec![
                ("after:60", "In 1 hour", None),
                ("after:180", "In 3 hours", None),
                ("at:today:18:00", "This evening", Some("6:00 PM")),
                ("at:tomorrow:09:00", "Tomorrow", Some("9:00 AM")),
                ("at:monday:09:00", "Next week", Some("Mon 9:00 AM")),
            ]
        );
        assert_eq!(list[2].until, local(2026, 11, 10, 18, 0) - OFFSET);
        assert_eq!(list[4].until, local(2026, 11, 16, 9, 0) - OFFSET);
    }

    #[test]
    fn a_preset_that_has_passed_or_is_malformed_is_not_offered() {
        let presets = vec![
            json!({ "kind": "at", "day": "today", "time": "13:00" }),
            json!({ "kind": "at", "day": "tuesday", "time": "16:00" }),
            json!({ "kind": "at", "day": "friday", "time": "09:00" }),
            json!({ "kind": "after", "minutes": 120 }),
            json!({ "kind": "at", "day": "today", "time": "16:30" }),
            json!({ "kind": "after", "minutes": 0 }),
            json!({ "kind": "after", "minutes": 90 }),
            json!({ "kind": "at", "day": "someday", "time": "09:00" }),
            // Past T3's eight.
            json!({ "kind": "after", "minutes": 30 }),
        ];
        let labels: Vec<String> = choices(&presets, now(), &zone).into_iter().map(|c| c.label).collect();
        // Two land on 16:30: the first listed stays. Tuesday is today, so next week's.
        assert_eq!(labels, vec!["In 90 minutes", "In 2 hours", "Friday", "Next Tuesday"]);
        for bad in [json!({ "kind": "at", "day": "today", "time": "25:00" }), json!({ "kind": "at", "day": "today", "time": "+1:00" }), json!(null), json!({ "kind": "after", "minutes": 43_201 })] {
            assert_eq!(preset_choice(&bad, now(), &zone), None, "{bad}");
        }
        assert_eq!(choices(&fixed_presets(), now(), &zone).into_iter().map(|c| c.label).collect::<Vec<_>>(), vec!["In 1 hour", "In 4 hours", "In 1 day", "In 7 days"]);
    }

    #[test]
    fn a_key_names_its_preset_back() {
        for preset in t3_default_presets() {
            let choice = preset_choice(&preset, now(), &zone).unwrap();
            assert_eq!(preset_from_key(&choice.key), Some(preset));
        }
        assert_eq!(preset_from_key("after:x"), None);
        assert_eq!(preset_from_key("at:friday"), None);
    }

    #[test]
    fn the_game_is_told_when_in_words() {
        let n = now();
        assert_eq!(describe(local(2026, 11, 10, 17, 0) - OFFSET, n, &zone), "5:00 PM");
        assert_eq!(describe(local(2026, 11, 13, 15, 0) - OFFSET, n, &zone), "Fri 3:00 PM");
        assert_eq!(describe(local(2026, 12, 1, 0, 5) - OFFSET, n, &zone), "Dec 1 12:05 AM");
    }

    /// The offset matches what `date` says for the same instants, winter and summer.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_macs_offset_is_its_time_zones() {
        for at in [local(2026, 1, 15, 12, 0), local(2026, 7, 15, 12, 0), crate::config::now_ms() / 1000] {
            let out = std::process::Command::new("/bin/date").args(["-r", &at.to_string(), "+%z"]).output().expect("date");
            let said = String::from_utf8(out.stdout).unwrap();
            let said = said.trim();
            let sign = if said.starts_with('-') { -1 } else { 1 };
            let expected = sign * (said[1..3].parse::<i64>().unwrap() * 3600 + said[3..5].parse::<i64>().unwrap() * 60);
            assert_eq!(system_offset(at), expected, "at {at}, date says {said}");
        }
    }
}
