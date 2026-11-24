//! `Date`, in the user's time zone and format, as Windows knows them.

use crate::convert::Hint;
use crate::object::*;
use crate::text;
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

const MS_PER_DAY: f64 = 86_400_000.0;
/// The most a date can be from 1970, in milliseconds.
const LARGEST_TIME: f64 = 8.64e15;

/// Days since 1970 for a date, by the proleptic Gregorian calendar.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The year, month (1 to 12) and day for a number of days since 1970.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The language's `TimeClip`.
fn time_clip(time: f64) -> f64 {
    if !time.is_finite() || time.abs() > LARGEST_TIME {
        f64::NAN
    } else {
        // This also turns -0 into +0.
        time.trunc() + 0.0
    }
}

/// A time from its parts, which can be out of range and carry over. It isn't clipped, as local time can go a bit
/// past what's allowed and be allowed once it's been turned into UTC.
fn make_time(parts: &[f64; 7]) -> f64 {
    if parts.iter().any(|part| !part.is_finite()) {
        return f64::NAN;
    }
    let [year, month, day, hour, minute, second, ms] = parts.map(f64::trunc);
    let year = year + (month / 12.0).floor();
    // Anything further away than this would be out of range whatever else there is.
    if year.abs() > 400_000.0 {
        return f64::NAN;
    }
    let month = month.rem_euclid(12.0);
    let days = days_from_civil(year as i64, month as i64 + 1, 1) as f64 + day - 1.0;
    days * MS_PER_DAY + hour * 3_600_000.0 + minute * 60_000.0 + second * 1000.0 + ms
}

struct Fields {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    ms: i64,
    weekday: i64,
}

fn fields_of(time: f64) -> Fields {
    let days = (time / MS_PER_DAY).floor();
    let within = (time - days * MS_PER_DAY) as i64;
    let (year, month, day) = civil_from_days(days as i64);
    Fields {
        year,
        month,
        day,
        hour: within / 3_600_000,
        minute: within / 60_000 % 60,
        second: within / 1000 % 60,
        ms: within % 1000,
        weekday: (days as i64 + 4).rem_euclid(7),
    }
}

// Time zones.

#[cfg(windows)]
#[allow(unsafe_code)]
mod system {
    use std::cell::{Cell, OnceCell};

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(super) struct SystemTime {
        pub year: u16,
        pub month: u16,
        pub weekday: u16,
        pub day: u16,
        pub hour: u16,
        pub minute: u16,
        pub second: u16,
        pub ms: u16,
    }

    /// `DYNAMIC_TIME_ZONE_INFORMATION`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ZoneInfo {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SystemTime,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SystemTime,
        daylight_bias: i32,
        key_name: [u16; 128],
        no_dynamic_daylight: u8,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDynamicTimeZoneInformation(info: *mut ZoneInfo) -> u32;
        fn SystemTimeToTzSpecificLocalTimeEx(
            info: *const ZoneInfo,
            universal: *const SystemTime,
            local: *mut SystemTime,
        ) -> i32;
        fn GetDateFormatEx(
            locale: *const u16,
            flags: u32,
            date: *const SystemTime,
            format: *const u16,
            out: *mut u16,
            length: i32,
            calendar: *const u16,
        ) -> i32;
        fn GetTimeFormatEx(
            locale: *const u16,
            flags: u32,
            time: *const SystemTime,
            format: *const u16,
            out: *mut u16,
            length: i32,
        ) -> i32;
    }

    const CACHED_OFFSETS: usize = 64;
    /// How long an offset can be taken to last. Zones change on a quarter of an hour at the least.
    const CACHE_STEP: f64 = 900_000.0;

    thread_local! {
        static ZONE: OnceCell<Option<ZoneInfo>> = const { OnceCell::new() };
        static OFFSETS: [Cell<(i64, i32)>; CACHED_OFFSETS] =
            const { [const { Cell::new((i64::MIN, 0)) }; CACHED_OFFSETS] };
    }

    fn with_zone<T>(run: impl FnOnce(&ZoneInfo) -> T) -> Option<T> {
        ZONE.with(|zone| {
            let zone = zone.get_or_init(|| {
                // SAFETY: this only writes to what it's given, which is all zeros to begin with, and valid as that.
                unsafe {
                    let mut info: ZoneInfo = std::mem::zeroed();
                    (GetDynamicTimeZoneInformation(&raw mut info) != u32::MAX).then_some(info)
                }
            });
            zone.as_ref().map(run)
        })
    }

    /// A year Windows can work with that has the same calendar as this one, for dates it can't.
    fn year_it_can_do(year: i64) -> i64 {
        if (1602..=30000).contains(&year) {
            return year;
        }
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let first_day = (super::days_from_civil(year, 1, 1) + 4).rem_euclid(7);
        (2000..2028)
            .find(|&candidate: &i64| {
                let candidate_leap = candidate % 4 == 0;
                candidate_leap == leap
                    && (super::days_from_civil(candidate, 1, 1) + 4).rem_euclid(7) == first_day
            })
            .unwrap_or(2000)
    }

    pub(super) fn to_system_time(fields: &super::Fields, year: i64) -> SystemTime {
        SystemTime {
            year: year as u16,
            month: fields.month as u16,
            weekday: fields.weekday as u16,
            day: fields.day as u16,
            hour: fields.hour as u16,
            minute: fields.minute as u16,
            second: fields.second as u16,
            ms: fields.ms as u16,
        }
    }

    /// How far ahead of UTC local time is at this moment, in milliseconds.
    pub(super) fn offset_at(time: f64) -> f64 {
        if !time.is_finite() {
            return 0.0;
        }
        let key = (time / CACHE_STEP).floor() as i64;
        let slot = key.rem_euclid(CACHED_OFFSETS as i64) as usize;
        let cached = OFFSETS.with(|offsets| offsets[slot].get());
        if cached.0 == key {
            return f64::from(cached.1);
        }
        let fields = super::fields_of(time);
        let year = year_it_can_do(fields.year);
        let universal = to_system_time(&fields, year);
        let offset = with_zone(|zone| {
            let mut local = SystemTime::default();
            // SAFETY: everything here is valid for the call, and it only writes to `local`.
            let done =
                unsafe { SystemTimeToTzSpecificLocalTimeEx(zone, &universal, &raw mut local) };
            if done == 0 {
                return 0;
            }
            let moment = |at: &SystemTime| {
                super::days_from_civil(i64::from(at.year), i64::from(at.month), i64::from(at.day))
                    * 86_400_000
                    + i64::from(at.hour) * 3_600_000
                    + i64::from(at.minute) * 60_000
                    + i64::from(at.second) * 1000
            };
            (moment(&local) - moment(&universal)) as i32
        })
        .unwrap_or(0);
        OFFSETS.with(|offsets| offsets[slot].set((key, offset)));
        f64::from(offset)
    }

    /// What the zone's called at this moment.
    pub(super) fn zone_name(time: f64) -> String {
        with_zone(|zone| {
            // The standard offset comes from the bias, which is minutes behind UTC.
            let standard = -f64::from(zone.bias + zone.standard_bias) * 60_000.0;
            let name = if offset_at(time) == standard {
                &zone.standard_name
            } else {
                &zone.daylight_name
            };
            let end = name
                .iter()
                .position(|&unit| unit == 0)
                .unwrap_or(name.len());
            String::from_utf16_lossy(&name[..end])
        })
        .unwrap_or_else(|| "Coordinated Universal Time".to_owned())
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    /// Formats a date or a time as the system would for this locale, or the user's. `None` if it can't.
    pub(super) fn format(
        locale: Option<&str>,
        at: &super::Fields,
        date: bool,
        picture: Option<&str>,
        flags: u32,
    ) -> Option<String> {
        if !(1601..=30000).contains(&at.year) {
            return None;
        }
        let when = to_system_time(at, at.year);
        let locale = locale.map(wide);
        let locale = locale
            .as_ref()
            .map_or(std::ptr::null(), |name| name.as_ptr());
        let picture = picture.map(wide);
        let picture = picture
            .as_ref()
            .map_or(std::ptr::null(), |text| text.as_ptr());
        let mut out = [0u16; 128];
        // SAFETY: the strings end in a zero and outlive the call, and it's told how long `out` is.
        let written = unsafe {
            if date {
                GetDateFormatEx(
                    locale,
                    flags,
                    &when,
                    picture,
                    out.as_mut_ptr(),
                    out.len() as i32,
                    std::ptr::null(),
                )
            } else {
                GetTimeFormatEx(
                    locale,
                    flags,
                    &when,
                    picture,
                    out.as_mut_ptr(),
                    out.len() as i32,
                )
            }
        };
        (written > 0).then(|| String::from_utf16_lossy(&out[..written as usize - 1]))
    }
}

/// Elsewhere the local zone is the system's zone file, as `/etc/localtime` or what `$TZ` names: a list of the moments
/// its offset changes, and a rule, as `$TZ` would be written, for after the last of them.
#[cfg(not(windows))]
mod system {
    use std::sync::OnceLock;

    /// One of the offsets a zone has had: seconds ahead of UTC, whether it's summer time, and what it's called.
    #[derive(Clone)]
    struct Kind {
        offset: i32,
        summer: bool,
        name: String,
    }

    /// A rule such as `CET-1CEST,M3.5.0,M10.5.0/3`: the standard offset, and the summer one with when it starts and ends.
    #[derive(Clone)]
    struct Rule {
        standard: Kind,
        summer: Option<(Kind, Change, Change)>,
    }

    /// When in a year an offset changes: a month, a week of it (5 is the last) and a day of the week, at a time of day,
    /// or a day of the year.
    #[derive(Clone, Copy)]
    enum Day {
        MonthWeekDay(u32, u32, u32),
        /// Counting from 1, never counting the 29th of February.
        Julian(u32),
        /// Counting from 0, counting the 29th of February.
        Ordinal(u32),
    }

    #[derive(Clone, Copy)]
    struct Change {
        day: Day,
        seconds: i32,
    }

    struct Zone {
        changes: Vec<(i64, usize)>,
        kinds: Vec<Kind>,
        rule: Option<Rule>,
    }

    fn zone() -> Option<&'static Zone> {
        static ZONE: OnceLock<Option<Zone>> = OnceLock::new();
        ZONE.get_or_init(|| {
            let said = std::env::var("TZ").ok().filter(|said| !said.is_empty());
            if let Some(said) = &said {
                let name = said.strip_prefix(':').unwrap_or(said);
                if let Some(zone) = find_file(name).and_then(|bytes| read_tzif(&bytes)) {
                    return Some(zone);
                }
                if let Some(rule) = read_rule(said) {
                    return Some(Zone { changes: Vec::new(), kinds: Vec::new(), rule: Some(rule) });
                }
            }
            if cfg!(target_os = "android") {
                return None;
            }
            std::fs::read("/etc/localtime").ok().and_then(|bytes| read_tzif(&bytes))
        })
        .as_ref()
    }

    /// The file for a zone's name, as `Europe/Amsterdam`, in the places systems keep them.
    fn find_file(name: &str) -> Option<Vec<u8>> {
        if name.starts_with('/') {
            return std::fs::read(name).ok();
        }
        if name.contains("..") {
            return None;
        }
        ["/usr/share/zoneinfo", "/var/db/timezone/zoneinfo", "/usr/lib/zoneinfo", "/usr/share/lib/zoneinfo"]
            .iter()
            .find_map(|folder| std::fs::read(format!("{folder}/{name}")).ok())
    }

    /// Reads a TZif file, version 2 or later if it is one, as the 64-bit times it has after the 32-bit ones.
    fn read_tzif(bytes: &[u8]) -> Option<Zone> {
        let header = |at: usize| -> Option<[usize; 6]> {
            if bytes.get(at..at + 4)? != b"TZif" {
                return None;
            }
            let count = |n: usize| -> Option<usize> {
                let from = at + 20 + n * 4;
                Some(u32::from_be_bytes(bytes.get(from..from + 4)?.try_into().ok()?) as usize)
            };
            // UT/local, standard/wall, leap seconds, transitions, types, abbreviation bytes.
            Some([count(0)?, count(1)?, count(2)?, count(3)?, count(4)?, count(5)?])
        };
        let first = header(0)?;
        let version = *bytes.get(4)?;
        let size = |counts: &[usize; 6], time: usize| {
            counts[3] * time + counts[3] + counts[4] * 6 + counts[5] + counts[2] * (time + 4) + counts[1] + counts[0]
        };
        let (at, counts, time) = if version >= b'2' {
            let second = 44 + size(&first, 4);
            (second, header(second)?, 8)
        } else {
            (0, first, 4)
        };
        let [_, _, _, transitions, types, abbreviations] = counts;
        let mut read = at + 44;
        let mut changes = Vec::with_capacity(transitions);
        let times = bytes.get(read..read + transitions * time)?;
        read += transitions * time;
        let indexes = bytes.get(read..read + transitions)?;
        read += transitions;
        for (index, &kind) in indexes.iter().enumerate() {
            let raw = &times[index * time..index * time + time];
            let when = if time == 8 {
                i64::from_be_bytes(raw.try_into().ok()?)
            } else {
                i64::from(i32::from_be_bytes(raw.try_into().ok()?))
            };
            changes.push((when, usize::from(kind)));
        }
        let records = bytes.get(read..read + types * 6)?;
        read += types * 6;
        let names = bytes.get(read..read + abbreviations)?;
        let mut kinds = Vec::with_capacity(types);
        for record in records.chunks_exact(6) {
            let offset = i32::from_be_bytes(record[..4].try_into().ok()?);
            let start = usize::from(record[5]).min(names.len());
            let end = names[start..].iter().position(|&byte| byte == 0).map_or(names.len(), |end| start + end);
            kinds.push(Kind { offset, summer: record[4] != 0, name: String::from_utf8_lossy(&names[start..end]).into_owned() });
        }
        if kinds.is_empty() || changes.iter().any(|(_, kind)| *kind >= kinds.len()) {
            return None;
        }
        // After the second block comes the rule for what's past its last change, between two newlines.
        let rule = if version >= b'2' {
            let end = at + 44 + size(&counts, 8);
            bytes.get(end..).and_then(|rest| {
                let rest = std::str::from_utf8(rest).ok()?;
                read_rule(rest.trim_matches('\n').lines().next()?)
            })
        } else {
            None
        };
        Some(Zone { changes, kinds, rule })
    }

    /// Reads a rule written as `$TZ` is: `EST5EDT,M3.2.0,M11.1.0`. Offsets in it are hours behind UTC.
    fn read_rule(text: &str) -> Option<Rule> {
        let mut rest = text.as_bytes();
        let name = |rest: &mut &[u8]| -> Option<String> {
            if rest.first() == Some(&b'<') {
                let end = rest.iter().position(|&byte| byte == b'>')?;
                let name = String::from_utf8_lossy(&rest[1..end]).into_owned();
                *rest = &rest[end + 1..];
                return Some(name);
            }
            let end = rest.iter().position(|byte| !byte.is_ascii_alphabetic()).unwrap_or(rest.len());
            if end < 3 {
                return None;
            }
            let name = String::from_utf8_lossy(&rest[..end]).into_owned();
            *rest = &rest[end..];
            Some(name)
        };
        let time = |rest: &mut &[u8]| -> Option<i32> {
            let sign = match rest.first() {
                Some(b'-') => {
                    *rest = &rest[1..];
                    -1
                }
                Some(b'+') => {
                    *rest = &rest[1..];
                    1
                }
                _ => 1,
            };
            let mut total = 0;
            for (part, scale) in [3600, 60, 1].into_iter().enumerate() {
                let end = rest.iter().position(|byte| !byte.is_ascii_digit()).unwrap_or(rest.len());
                if end == 0 {
                    if part == 0 {
                        return None;
                    }
                    break;
                }
                let number: i32 = std::str::from_utf8(&rest[..end]).ok()?.parse().ok()?;
                total += number * scale;
                *rest = &rest[end..];
                if rest.first() == Some(&b':') && part < 2 {
                    *rest = &rest[1..];
                } else {
                    break;
                }
            }
            Some(sign * total)
        };
        let standard_name = name(&mut rest)?;
        let standard_offset = -time(&mut rest)?;
        let standard = Kind { offset: standard_offset, summer: false, name: standard_name };
        if rest.is_empty() {
            return Some(Rule { standard, summer: None });
        }
        let summer_name = name(&mut rest)?;
        let summer_offset = if rest.first().is_some_and(|byte| *byte != b',') { -time(&mut rest)? } else { standard_offset + 3600 };
        let change = |rest: &mut &[u8]| -> Option<Change> {
            if rest.first() != Some(&b',') {
                return None;
            }
            *rest = &rest[1..];
            let number = |rest: &mut &[u8]| -> Option<u32> {
                let end = rest.iter().position(|byte| !byte.is_ascii_digit()).unwrap_or(rest.len());
                let number = std::str::from_utf8(&rest[..end]).ok()?.parse().ok()?;
                *rest = &rest[end..];
                Some(number)
            };
            let day = match rest.first() {
                Some(b'M') => {
                    *rest = &rest[1..];
                    let month = number(rest)?;
                    *rest = rest.strip_prefix(b".")?;
                    let week = number(rest)?;
                    *rest = rest.strip_prefix(b".")?;
                    Day::MonthWeekDay(month, week, number(rest)?)
                }
                Some(b'J') => {
                    *rest = &rest[1..];
                    Day::Julian(number(rest)?)
                }
                _ => Day::Ordinal(number(rest)?),
            };
            let seconds = if rest.first() == Some(&b'/') {
                *rest = &rest[1..];
                time(rest)?
            } else {
                7200
            };
            Some(Change { day, seconds })
        };
        let starts = change(&mut rest)?;
        let ends = change(&mut rest)?;
        let summer = Kind { offset: summer_offset, summer: true, name: summer_name };
        Some(Rule { standard, summer: Some((summer, starts, ends)) })
    }

    /// The moment, in seconds from 1970 in UTC, that `change` comes in `year`, for a zone that's `offset` ahead then.
    fn moment_of(change: Change, year: i64, offset: i32) -> i64 {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let days = match change.day {
            Day::Julian(day) => {
                let day = i64::from(day.clamp(1, 365));
                super::days_from_civil(year, 1, 1) + day - 1 + i64::from(leap && day > 59)
            }
            Day::Ordinal(day) => super::days_from_civil(year, 1, 1) + i64::from(day.min(365)),
            Day::MonthWeekDay(month, week, weekday) => {
                let month = i64::from(month.clamp(1, 12));
                let first = super::days_from_civil(year, month, 1);
                // 1970-01-01 was a Thursday.
                let first_weekday = (first + 4).rem_euclid(7);
                let mut day = first + (i64::from(weekday) - first_weekday).rem_euclid(7) + (i64::from(week.clamp(1, 5)) - 1) * 7;
                let next_month = if month == 12 { super::days_from_civil(year + 1, 1, 1) } else { super::days_from_civil(year, month + 1, 1) };
                while day >= next_month {
                    day -= 7;
                }
                day
            }
        };
        days * 86_400 + i64::from(change.seconds) - i64::from(offset)
    }

    fn kind_by_rule(rule: &Rule, seconds: i64) -> &Kind {
        let Some((summer, starts, ends)) = &rule.summer else { return &rule.standard };
        let year = super::fields_of(seconds as f64 * 1000.0).year;
        let start = moment_of(*starts, year, rule.standard.offset);
        let end = moment_of(*ends, year, summer.offset);
        let in_summer = if start < end { seconds >= start && seconds < end } else { seconds >= start || seconds < end };
        if in_summer { summer } else { &rule.standard }
    }

    /// What the zone is at a moment, in milliseconds from 1970 in UTC.
    fn kind_at(time: f64) -> Option<Kind> {
        let zone = zone()?;
        let seconds = (time / 1000.0).floor() as i64;
        let after = zone.changes.partition_point(|(when, _)| *when <= seconds);
        if after == zone.changes.len() && let Some(rule) = &zone.rule {
            return Some(kind_by_rule(rule, seconds).clone());
        }
        let index = match after {
            // Before the first change it's the first kind that isn't summer time, as the format says.
            0 => zone.kinds.iter().position(|kind| !kind.summer).unwrap_or(0),
            after => zone.changes[after - 1].1,
        };
        zone.kinds.get(index).cloned().or_else(|| zone.rule.as_ref().map(|rule| rule.standard.clone()))
    }

    pub(super) fn offset_at(time: f64) -> f64 {
        if !time.is_finite() {
            return 0.0;
        }
        kind_at(time).map_or(0.0, |kind| f64::from(kind.offset) * 1000.0)
    }

    pub(super) fn zone_name(time: f64) -> String {
        match kind_at(time) {
            Some(kind) if !kind.name.is_empty() => kind.name,
            _ => "Coordinated Universal Time".to_owned(),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_rule_says_when_summer_time_is() {
            let rule = read_rule("CET-1CEST,M3.5.0,M10.5.0/3").expect("it reads");
            assert_eq!(rule.standard.offset, 3600);
            // 2024-07-01 is in summer time, 2024-01-15 isn't; summer starts 2024-03-31 at 01:00 UTC.
            let july = super::super::days_from_civil(2024, 7, 1) * 86_400;
            let january = super::super::days_from_civil(2024, 1, 15) * 86_400;
            assert_eq!(kind_by_rule(&rule, july).offset, 7200);
            assert_eq!(kind_by_rule(&rule, january).offset, 3600);
            let starts = super::super::days_from_civil(2024, 3, 31) * 86_400 + 3600;
            assert_eq!(kind_by_rule(&rule, starts - 1).offset, 3600);
            assert_eq!(kind_by_rule(&rule, starts).offset, 7200);
            let us = read_rule("EST5EDT,M3.2.0,M11.1.0").expect("it reads");
            assert_eq!(us.standard.offset, -5 * 3600);
            assert_eq!(kind_by_rule(&us, july).name, "EDT");
            assert_eq!(read_rule("<+0330>-3:30").expect("it reads").standard.offset, 3 * 3600 + 1800);
        }
    }

    pub(super) fn format(
        _: Option<&str>,
        _: &super::Fields,
        _: bool,
        _: Option<&str>,
        _: u32,
    ) -> Option<String> {
        None
    }
}

/// A moment in local time, from one in UTC.
fn local_time(time: f64) -> f64 {
    time + system::offset_at(time)
}

/// A moment in UTC, from one in local time. Clocks going forward skip some local times, and going back repeat some.
/// The language says a skipped time is read with the offset from before, and a repeated one means the first time.
fn utc_from_local(local: f64) -> f64 {
    if !local.is_finite() {
        return f64::NAN;
    }
    let before = system::offset_at(local - MS_PER_DAY);
    let after = system::offset_at(local + MS_PER_DAY);
    if before == after {
        return local - before;
    }
    let (with_before, with_after) = (local - before, local - after);
    let before_fits = system::offset_at(with_before) == before;
    let after_fits = system::offset_at(with_after) == after;
    match (before_fits, after_fits) {
        (true, true) => with_before.min(with_after),
        (false, true) => with_after,
        _ => with_before,
    }
}

// Reading dates.

/// The format the language defines: dates alone are UTC, and dates with times are local unless they say otherwise.
fn parse_iso(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let number = |at: &mut usize, count: usize| -> Option<f64> {
        let piece = bytes.get(*at..*at + count)?;
        if !piece.iter().all(u8::is_ascii_digit) {
            return None;
        }
        *at += count;
        std::str::from_utf8(piece).ok()?.parse().ok()
    };
    let mut at = 0;
    let year = match bytes.first() {
        Some(b'+') => {
            at += 1;
            number(&mut at, 6)?
        }
        Some(b'-') => {
            at += 1;
            let year = number(&mut at, 6)?;
            // There's one way to write the year zero, and it isn't this.
            if year == 0.0 {
                return None;
            }
            -year
        }
        _ => number(&mut at, 4)?,
    };
    let mut parts = [year, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
    if bytes.get(at) == Some(&b'-') {
        at += 1;
        parts[1] = number(&mut at, 2)? - 1.0;
        if bytes.get(at) == Some(&b'-') {
            at += 1;
            parts[2] = number(&mut at, 2)?;
        }
    }
    if !(0.0..=11.0).contains(&parts[1]) {
        return None;
    }
    // The 30th of February is the 1st of March, as it is in other engines.
    if !(1.0..=31.0).contains(&parts[2]) {
        return None;
    }
    let mut offset = None;
    if matches!(bytes.get(at), Some(b'T' | b't' | b' ')) {
        at += 1;
        parts[3] = number(&mut at, 2)?;
        if bytes.get(at) != Some(&b':') {
            return None;
        }
        at += 1;
        parts[4] = number(&mut at, 2)?;
        if bytes.get(at) == Some(&b':') {
            at += 1;
            parts[5] = number(&mut at, 2)?;
            if bytes.get(at) == Some(&b'.') {
                at += 1;
                let start = at;
                while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                    at += 1;
                }
                if at == start {
                    return None;
                }
                let digits = std::str::from_utf8(&bytes[start..at]).ok()?;
                let fraction: f64 = format!("0.{digits}").parse().ok()?;
                parts[6] = (fraction * 1000.0).floor();
            }
        }
        // Midnight at the end of a day can be written as 24:00, and nothing else can go past 23:59.
        let end_of_day = parts[3] == 24.0 && parts[4] == 0.0 && parts[5] == 0.0 && parts[6] == 0.0;
        if (parts[3] >= 24.0 && !end_of_day) || parts[4] >= 60.0 || parts[5] >= 60.0 {
            return None;
        }
        match bytes.get(at) {
            Some(b'Z' | b'z') => {
                at += 1;
                offset = Some(0.0);
            }
            Some(sign @ (b'+' | b'-')) => {
                let sign = if *sign == b'+' { 1.0 } else { -1.0 };
                at += 1;
                let hours = number(&mut at, 2)?;
                if bytes.get(at) == Some(&b':') {
                    at += 1;
                }
                let minutes = number(&mut at, 2)?;
                if hours > 23.0 || minutes > 59.0 {
                    return None;
                }
                offset = Some(sign * (hours * 3_600_000.0 + minutes * 60_000.0));
            }
            _ => {
                // With a time, and with nothing said about where, it's here.
                if at != bytes.len() {
                    return None;
                }
                return Some(time_clip(utc_from_local(make_time(&parts))));
            }
        }
    }
    if at != bytes.len() {
        return None;
    }
    Some(time_clip(make_time(&parts) - offset.unwrap_or(0.0)))
}

/// What people write, and what `toString` and `toUTCString` give: `Tue Mar 01 2022 10:00:00 GMT+0100 (Name)`,
/// `Tue, 01 Mar 2022 10:00:00 GMT`, `March 7, 2021`, `3/7/2021 5:30 PM`, and the like. Local unless it says otherwise.
fn parse_loose(text: &str) -> Option<f64> {
    // What's in brackets is only a comment.
    let mut clean = String::with_capacity(text.len());
    let mut depth = 0;
    for character in text.chars() {
        match character {
            '(' => depth += 1,
            ')' => depth = (depth - 1).max(0),
            _ if depth == 0 => clean.push(character),
            _ => {}
        }
    }
    let (mut year, mut month, mut day) = (None, None, None);
    let mut time = [0.0f64; 4];
    let mut offset: Option<f64> = None;
    let mut afternoon = None;
    let mut numbers: Vec<f64> = Vec::new();
    let mut slashes = false;
    for word in clean
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|word| !word.is_empty())
    {
        let lower = word.to_ascii_lowercase();
        if lower == "am" || lower == "pm" {
            afternoon = Some(lower == "pm");
            continue;
        }
        if word.contains(':') {
            // A time, perhaps with a fraction of a second.
            let mut bits = word.splitn(3, ':');
            time[0] = bits.next()?.parse().ok()?;
            time[1] = bits.next()?.parse().ok()?;
            if let Some(seconds) = bits.next() {
                let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
                time[2] = whole.parse().ok()?;
                if !fraction.is_empty() {
                    let fraction: f64 = format!("0.{fraction}").parse().ok()?;
                    time[3] = (fraction * 1000.0).floor();
                }
            }
            continue;
        }
        if let Some(zone) = lower
            .strip_prefix("gmt")
            .or_else(|| lower.strip_prefix("utc"))
            .or_else(|| (lower == "z").then_some(""))
        {
            offset = Some(if zone.is_empty() {
                0.0
            } else {
                read_offset(zone)?
            });
            continue;
        }
        if (word.starts_with('+') || word.starts_with('-')) && word.len() >= 3 && offset.is_none() {
            if let Some(found) = read_offset(word) {
                offset = Some(found);
                continue;
            }
        }
        if lower
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        {
            let short: String = lower.chars().take(3).collect();
            if let Some(at) = MONTHS
                .iter()
                .position(|name| name.eq_ignore_ascii_case(&short))
            {
                month = Some(at as f64);
            } else if !DAYS.iter().any(|name| name.eq_ignore_ascii_case(&short)) {
                return None;
            }
            continue;
        }
        if word.contains('/') || word.contains('-') {
            slashes = true;
            let separator = if word.contains('/') { '/' } else { '-' };
            for piece in word.split(separator) {
                numbers.push(piece.parse().ok()?);
            }
            continue;
        }
        numbers.push(word.parse().ok()?);
    }
    if month.is_some() {
        // With the month as a word, a small number is the day, and the other is the year.
        for number in numbers {
            if day.is_none() && (1.0..=31.0).contains(&number) {
                day = Some(number);
            } else if year.is_none() {
                year = Some(number);
            } else {
                return None;
            }
        }
    } else if slashes {
        match numbers.as_slice() {
            // A year first is the one way to read it, and otherwise it's month first, as it is in JavaScript.
            [first, second, third] if *first > 31.0 => {
                year = Some(*first);
                month = Some(second - 1.0);
                day = Some(*third);
            }
            [first, second, third] => {
                month = Some(first - 1.0);
                day = Some(*second);
                year = Some(*third);
            }
            _ => return None,
        }
    } else {
        return None;
    }
    let year = year?;
    let year = if (0.0..50.0).contains(&year) {
        year + 2000.0
    } else if (50.0..100.0).contains(&year) {
        year + 1900.0
    } else {
        year
    };
    let month = month?;
    let day = day.unwrap_or(1.0);
    if !(0.0..=11.0).contains(&month) || !(1.0..=31.0).contains(&day) {
        return None;
    }
    let mut hour = time[0];
    match afternoon {
        Some(true) if hour < 12.0 => hour += 12.0,
        Some(false) if hour == 12.0 => hour = 0.0,
        _ => {}
    }
    if hour >= 25.0 || time[1] >= 60.0 || time[2] >= 60.0 {
        return None;
    }
    let made = make_time(&[year, month, day, hour, time[1], time[2], time[3]]);
    Some(time_clip(match offset {
        Some(offset) => made - offset,
        None => utc_from_local(made),
    }))
}

/// `+0100`, `-05:30` or `+1`, in milliseconds.
fn read_offset(text: &str) -> Option<f64> {
    let (sign, rest) = match text.as_bytes().first()? {
        b'+' => (1.0, &text[1..]),
        b'-' => (-1.0, &text[1..]),
        _ => return None,
    };
    let digits: String = rest.chars().filter(|c| *c != ':').collect();
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let (hours, minutes) = if digits.len() <= 2 {
        (digits.parse::<f64>().ok()?, 0.0)
    } else {
        let split = digits.len() - 2;
        (
            digits[..split].parse::<f64>().ok()?,
            digits[split..].parse::<f64>().ok()?,
        )
    };
    Some(sign * (hours * 3_600_000.0 + minutes * 60_000.0))
}

fn parse_date(text: &str) -> f64 {
    let text = text.trim();
    parse_iso(text)
        .or_else(|| parse_loose(text))
        .unwrap_or(f64::NAN)
}

// Writing them.

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn iso_string(time: f64) -> String {
    let fields = fields_of(time);
    let year = if (0..=9999).contains(&fields.year) {
        format!("{:04}", fields.year)
    } else if fields.year < 0 {
        format!("-{:06}", -fields.year)
    } else {
        format!("+{:06}", fields.year)
    };
    format!(
        "{year}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        fields.month, fields.day, fields.hour, fields.minute, fields.second, fields.ms
    )
}

/// A year the way the strings from `toString` write it.
fn plain_year(year: i64) -> String {
    if year < 0 {
        format!("-{:04}", -year)
    } else {
        format!("{year:04}")
    }
}

fn date_part(time: f64) -> String {
    let fields = fields_of(local_time(time));
    format!(
        "{} {} {:02} {}",
        DAYS[fields.weekday as usize],
        MONTHS[fields.month as usize - 1],
        fields.day,
        plain_year(fields.year)
    )
}

fn time_part(time: f64) -> String {
    let fields = fields_of(local_time(time));
    let offset = (system::offset_at(time) / 60_000.0) as i64;
    let sign = if offset < 0 { '-' } else { '+' };
    format!(
        "{:02}:{:02}:{:02} GMT{sign}{:02}{:02} ({})",
        fields.hour,
        fields.minute,
        fields.second,
        offset.abs() / 60,
        offset.abs() % 60,
        system::zone_name(time)
    )
}

fn date_string(time: f64) -> String {
    if time.is_nan() {
        return "Invalid Date".to_owned();
    }
    format!("{} {}", date_part(time), time_part(time))
}

fn this_time(vm: &mut Vm, this: Value) -> Run<f64> {
    if this.is_object() {
        if let Kind::Date(time) = vm.objects[this.index()].kind {
            return Ok(time);
        }
    }
    Err(vm.type_error("this is not a Date object."))
}

fn date_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    // What's made from the time now isn't pure (see `NativeData::pure`); a date of a time that's given is.
    if (!args.new_target.is_object() || args.count == 0) && vm.keeping.working != 0 {
        vm.keeping.spoiled = true;
    }
    if !args.new_target.is_object() {
        let now = Vm::now_ms();
        let text = date_string(now);
        return Ok(vm.new_str(&text));
    }
    let time = if args.count == 0 {
        Vm::now_ms().floor()
    } else if args.count == 1 {
        let value = vm.arg(&args, 0);
        if value.is_object() && matches!(vm.objects[value.index()].kind, Kind::Date(_)) {
            this_time(vm, value)?
        } else {
            let primitive = vm.to_primitive(value, Hint::Default)?;
            if primitive.is_string() {
                parse_date(&vm.text_of(primitive).to_rust())
            } else {
                let number = vm.to_number(primitive)?;
                time_clip(number)
            }
        }
    } else {
        let parts = parts_from_args(vm, &args)?;
        time_clip(utc_from_local(make_time(&parts)))
    };
    let proto = vm.proto_for(&args, vm.realm.date_proto)?;
    Ok(vm.new_object_with_proto(proto, Kind::Date(time)))
}

/// The arguments of `new Date(year, month, ...)` or `Date.UTC`. A year from 0 to 99 is in the 1900s.
fn parts_from_args(vm: &mut Vm, args: &Args) -> Run<[f64; 7]> {
    let mut parts = [f64::NAN, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
    for (index, part) in parts.iter_mut().enumerate() {
        if index < args.count {
            let value = vm.arg(args, index);
            *part = vm.to_number(value)?;
        }
    }
    let whole_year = parts[0].trunc();
    if (0.0..=99.0).contains(&whole_year) {
        parts[0] = whole_year + 1900.0;
    }
    Ok(parts)
}

fn now(_: &mut Vm, _: Args) -> Run<Value> {
    Ok(Value::number(Vm::now_ms().floor()))
}

fn parse(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let text = vm.to_rust_string(value)?;
    Ok(Value::number(parse_date(&text)))
}

fn utc(vm: &mut Vm, args: Args) -> Run<Value> {
    let parts = parts_from_args(vm, &args)?;
    Ok(Value::number(time_clip(make_time(&parts))))
}

fn get_time(vm: &mut Vm, args: Args) -> Run<Value> {
    Ok(Value::number(this_time(vm, args.this)?))
}

fn to_iso_string(vm: &mut Vm, args: Args) -> Run<Value> {
    let time = this_time(vm, args.this)?;
    if time.is_nan() {
        return Err(vm.range_error("Invalid time value"));
    }
    Ok(vm.new_str(&iso_string(time)))
}

/// This works on anything with a `toISOString`, as the language says.
fn to_json(vm: &mut Vm, args: Args) -> Run<Value> {
    let object = vm.to_object(args.this)?;
    let primitive = vm.to_primitive(object, Hint::Number)?;
    if primitive.is_number() && !primitive.as_number().is_finite() {
        return Ok(Value::NULL);
    }
    let name = vm.intern_str("toISOString");
    let method = vm.get(object, name)?;
    if !vm.is_callable(method) {
        return Err(vm.type_error("toISOString is not a function"));
    }
    vm.call(method, object, &[])
}

/// What most of the strings do with a date that isn't one.
fn string_of(vm: &mut Vm, args: &Args, make: impl FnOnce(f64) -> String) -> Run<Value> {
    let time = this_time(vm, args.this)?;
    if time.is_nan() {
        return Ok(vm.new_str("Invalid Date"));
    }
    let made = make(time);
    Ok(vm.new_str(&made))
}

fn to_string(vm: &mut Vm, args: Args) -> Run<Value> {
    string_of(vm, &args, date_string)
}

fn to_date_string(vm: &mut Vm, args: Args) -> Run<Value> {
    string_of(vm, &args, date_part)
}

fn to_time_string(vm: &mut Vm, args: Args) -> Run<Value> {
    string_of(vm, &args, time_part)
}

fn to_utc_string(vm: &mut Vm, args: Args) -> Run<Value> {
    string_of(vm, &args, |time| {
        let fields = fields_of(time);
        format!(
            "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
            DAYS[fields.weekday as usize],
            fields.day,
            MONTHS[fields.month as usize - 1],
            plain_year(fields.year),
            fields.hour,
            fields.minute,
            fields.second
        )
    })
}

// `toLocale...String`. There's no `Intl`, so this asks Windows, which knows the user's format and every language's names
// for things.

const DATE_SHORT: u32 = 1;

/// What's asked for in the options of `toLocaleString` and its kind.
#[derive(Default)]
struct LocaleOptions {
    locale: Option<String>,
    weekday: Option<String>,
    year: Option<String>,
    month: Option<String>,
    day: Option<String>,
    hour: Option<String>,
    minute: Option<String>,
    second: Option<String>,
    hour12: Option<bool>,
    in_utc: bool,
    /// The digits it's written in, by `numberingSystem` or the locale's `-u-nu-`.
    numbering: Option<String>,
}

impl LocaleOptions {
    fn any_date(&self) -> bool {
        self.weekday.is_some() || self.year.is_some() || self.month.is_some() || self.day.is_some()
    }

    fn any_time(&self) -> bool {
        self.hour.is_some() || self.minute.is_some() || self.second.is_some()
    }
}

fn read_locale_options(vm: &mut Vm, args: &Args) -> Run<LocaleOptions> {
    let mut options = LocaleOptions::default();
    let locale = vm.arg(args, 0);
    let locale = match vm.array_elements(locale) {
        Some(list) => list.first().copied().unwrap_or(Value::UNDEFINED),
        None => locale,
    };
    if locale.is_string() {
        // A language Windows doesn't know, as `"default"` isn't one, is the user's, as a browser has it.
        let given = super::locale::locale_of(Some(&vm.to_rust_string(locale)?));
        options.locale = given.name;
        options.numbering = given.numbering;
    }
    let given = vm.arg(args, 1);
    if !given.is_object() {
        return Ok(options);
    }
    let read = |vm: &mut Vm, name: &str| -> Run<Option<String>> {
        let atom = vm.intern_str(name);
        let value = vm.get(given, atom)?;
        if value.is_undefined() {
            return Ok(None);
        }
        Ok(Some(vm.to_rust_string(value)?))
    };
    options.weekday = read(vm, "weekday")?;
    options.year = read(vm, "year")?;
    options.month = read(vm, "month")?;
    options.day = read(vm, "day")?;
    options.hour = read(vm, "hour")?;
    options.minute = read(vm, "minute")?;
    options.second = read(vm, "second")?;
    options.hour12 = read(vm, "hour12")?.map(|value| value == "true");
    options.in_utc = read(vm, "timeZone")?.is_some_and(|zone| zone.eq_ignore_ascii_case("utc"));
    if let Some(system) =
        read(vm, "numberingSystem")?.filter(|system| super::locale::zero_of(system).is_some())
    {
        options.numbering = Some(system);
    }
    // `dateStyle` and `timeStyle` come down to the same thing.
    match read(vm, "dateStyle")?.as_deref() {
        Some("full") => {
            options.weekday = Some("long".to_owned());
            options.month = Some("long".to_owned());
            options.day = Some("numeric".to_owned());
            options.year = Some("numeric".to_owned());
        }
        Some("long") => {
            options.month = Some("long".to_owned());
            options.day = Some("numeric".to_owned());
            options.year = Some("numeric".to_owned());
        }
        Some("medium") => {
            options.month = Some("short".to_owned());
            options.day = Some("numeric".to_owned());
            options.year = Some("numeric".to_owned());
        }
        Some(_) => {
            options.month = Some("numeric".to_owned());
            options.day = Some("numeric".to_owned());
            options.year = Some("2-digit".to_owned());
        }
        None => {}
    }
    match read(vm, "timeStyle")?.as_deref() {
        Some("short") => {
            options.hour = Some("numeric".to_owned());
            options.minute = Some("2-digit".to_owned());
        }
        Some(_) => {
            options.hour = Some("numeric".to_owned());
            options.minute = Some("2-digit".to_owned());
            options.second = Some("2-digit".to_owned());
        }
        None => {}
    }
    Ok(options)
}

/// The locale to go by: the one asked for, or the user's.
#[cfg(not(windows))]
fn locale_or_users(locale: Option<&str>) -> String {
    locale.map(str::to_owned).unwrap_or_else(|| {
        ["LC_ALL", "LC_TIME", "LANG"]
            .into_iter()
            .filter_map(|name| std::env::var(name).ok())
            .find(|value| !value.is_empty() && value != "C" && value != "POSIX")
            .map(|value| value.split(['.', '@']).next().unwrap_or_default().replace('_', "-"))
            .unwrap_or_else(|| "en-US".to_owned())
    })
}

/// Which way around a language writes dates, and what goes between the numbers, for the languages most used; others
/// write the day first with slashes, as most of the world does.
#[cfg(not(windows))]
fn date_order_of(locale: Option<&str>) -> (DateOrder, String) {
    let locale = locale_or_users(locale).to_ascii_lowercase();
    let language = locale.split('-').next().unwrap_or_default();
    let american = ["en-us", "en", "en-ph", "en-ca"];
    if american.contains(&locale.as_str()) || language == "fil" {
        return (DateOrder::MonthDayYear, "/".to_owned());
    }
    match language {
        "zh" | "ja" => (DateOrder::YearMonthDay, "/".to_owned()),
        "hu" | "lt" | "sv" => (DateOrder::YearMonthDay, "-".to_owned()),
        "fr" if locale == "fr-ca" => (DateOrder::YearMonthDay, "-".to_owned()),
        "ko" => (DateOrder::YearMonthDay, ". ".to_owned()),
        "de" | "ru" | "pl" | "cs" | "sk" | "fi" | "nb" | "no" | "da" | "tr" | "uk" | "ro" | "bg" | "hr" | "sl" | "et"
        | "lv" | "is" | "sr" | "az" | "kk" => (DateOrder::DayMonthYear, ".".to_owned()),
        "nl" => (DateOrder::DayMonthYear, "-".to_owned()),
        _ => (DateOrder::DayMonthYear, "/".to_owned()),
    }
}

/// Whether a language writes hours from 1 to 12, as American English does, rather than up to 24.
#[cfg(not(windows))]
fn uses_twelve_hours(locale: Option<&str>) -> bool {
    let locale = locale_or_users(locale).to_ascii_lowercase();
    let language = locale.split('-').next().unwrap_or_default();
    matches!(locale.as_str(), "en-us" | "en" | "en-au" | "en-ca" | "en-in" | "en-nz" | "en-ph" | "es-mx" | "es-us" | "zh-tw")
        || matches!(language, "hi" | "ar" | "ko" | "bn" | "ur" | "fil")
}

/// Which way around a language writes dates, and what goes between the numbers. Windows knows this for every language, and
/// this is only asked about ones besides American English.
#[cfg(windows)]
#[allow(unsafe_code)]
fn date_order_of(locale: Option<&str>) -> (DateOrder, String) {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocaleInfoEx(name: *const u16, kind: u32, out: *mut u16, length: i32) -> i32;
    }
    const ORDER: u32 = 0x22;
    const SEPARATOR: u32 = 0x1D;
    let american = (DateOrder::MonthDayYear, "/".to_owned());
    if locale.is_some_and(|locale| {
        locale.eq_ignore_ascii_case("en-US") || locale.eq_ignore_ascii_case("en")
    }) {
        return american;
    }
    let name: Option<Vec<u16>> = locale.map(|locale| locale.encode_utf16().chain([0]).collect());
    let pointer = name.as_ref().map_or(std::ptr::null(), |name| name.as_ptr());
    let ask = |kind: u32| -> Option<String> {
        let mut out = [0u16; 8];
        // SAFETY: the name ends in a terminator or isn't there, and it writes no more than the room it's told there is.
        let length = unsafe { GetLocaleInfoEx(pointer, kind, out.as_mut_ptr(), out.len() as i32) };
        (length > 1).then(|| String::from_utf16_lossy(&out[..length as usize - 1]))
    };
    let Some(order) = ask(ORDER) else {
        return american;
    };
    let order = match order.as_str() {
        "1" => DateOrder::DayMonthYear,
        "2" => DateOrder::YearMonthDay,
        _ => DateOrder::MonthDayYear,
    };
    (order, ask(SEPARATOR).unwrap_or_else(|| "/".to_owned()))
}

/// Whether a language writes hours from 1 to 12, as American English does, rather than up to 24.
#[cfg(windows)]
#[allow(unsafe_code)]
fn uses_twelve_hours(locale: Option<&str>) -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocaleInfoEx(name: *const u16, kind: u32, out: *mut u16, length: i32) -> i32;
    }
    const TIME_FORMAT: u32 = 0x23;
    if locale.is_some_and(|locale| {
        locale.eq_ignore_ascii_case("en-US") || locale.eq_ignore_ascii_case("en")
    }) {
        return true;
    }
    let name: Option<Vec<u16>> = locale.map(|locale| locale.encode_utf16().chain([0]).collect());
    let pointer = name.as_ref().map_or(std::ptr::null(), |name| name.as_ptr());
    let mut out = [0u16; 4];
    // SAFETY: the name ends in a terminator or isn't there, and it writes no more than the room it's told there is.
    let length =
        unsafe { GetLocaleInfoEx(pointer, TIME_FORMAT, out.as_mut_ptr(), out.len() as i32) };
    length > 1 && out[0] == u16::from(b'0')
}

#[derive(Clone, Copy, PartialEq)]
enum DateOrder {
    MonthDayYear,
    DayMonthYear,
    YearMonthDay,
}

/// The picture Windows takes for a date with these parts, in the order its language has them.
fn date_picture(options: &LocaleOptions) -> String {
    let (order, separator) = date_order_of(options.locale.as_deref());
    let year = options
        .year
        .as_deref()
        .map(|year| if year == "2-digit" { "yy" } else { "yyyy" });
    let day = options
        .day
        .as_deref()
        .map(|day| if day == "2-digit" { "dd" } else { "d" });
    let weekday = options
        .weekday
        .as_deref()
        .map(|name| if name == "long" { "dddd" } else { "ddd" });
    let in_words = matches!(options.month.as_deref(), Some("long" | "short" | "narrow"));
    let mut picture = String::new();
    if let Some(weekday) = weekday {
        picture.push_str(weekday);
    }
    if in_words {
        if !picture.is_empty() {
            picture.push_str(", ");
        }
        let month = if options.month.as_deref() == Some("long") {
            "MMMM"
        } else {
            "MMM"
        };
        match order {
            DateOrder::MonthDayYear => {
                picture.push_str(month);
                if let Some(day) = day {
                    picture.push(' ');
                    picture.push_str(day);
                }
                if let Some(year) = year {
                    picture.push_str(if day.is_some() { ", " } else { " " });
                    picture.push_str(year);
                }
            }
            DateOrder::DayMonthYear => {
                if let Some(day) = day {
                    picture.push_str(day);
                    picture.push(' ');
                }
                picture.push_str(month);
                if let Some(year) = year {
                    picture.push(' ');
                    picture.push_str(year);
                }
            }
            DateOrder::YearMonthDay => {
                if let Some(year) = year {
                    picture.push_str(year);
                    picture.push(' ');
                }
                picture.push_str(month);
                if let Some(day) = day {
                    picture.push(' ');
                    picture.push_str(day);
                }
            }
        }
    } else {
        let month = options
            .month
            .as_deref()
            .map(|month| if month == "2-digit" { "MM" } else { "M" });
        let in_order = match order {
            DateOrder::MonthDayYear => [month, day, year],
            DateOrder::DayMonthYear => [day, month, year],
            DateOrder::YearMonthDay => [year, month, day],
        };
        let parts: Vec<&str> = in_order.into_iter().flatten().collect();
        if !parts.is_empty() {
            if !picture.is_empty() {
                picture.push_str(", ");
            }
            // Anything that isn't a letter that means something goes through as it is, and quotes keep those safe.
            picture.push_str(&parts.join(&format!("'{}'", separator.replace('\'', "''"))));
        }
    }
    picture
}

fn time_picture(options: &LocaleOptions) -> Option<String> {
    // With no opinion on this, the locale decides, which needs no picture when all of it's wanted.
    let twelve = options.hour12;
    if twelve.is_none()
        && options.hour.is_some()
        && options.minute.is_some()
        && options.second.is_some()
    {
        return None;
    }
    let twelve = twelve.or_else(|| Some(uses_twelve_hours(options.locale.as_deref())));
    let mut parts = Vec::new();
    if options.hour.is_some() {
        parts.push(match (twelve.unwrap_or(false), options.hour.as_deref()) {
            (true, Some("2-digit")) => "hh",
            (true, _) => "h",
            (false, Some("2-digit")) => "HH",
            (false, _) => "H",
        });
    }
    if options.minute.is_some() {
        parts.push("mm");
    }
    if options.second.is_some() {
        parts.push("ss");
    }
    let mut picture = parts.join(":");
    if twelve == Some(true) && options.hour.is_some() {
        picture.push_str(" tt");
    }
    Some(picture)
}

#[derive(Clone, Copy, PartialEq)]
enum LocaleWants {
    Both,
    Date,
    Time,
}

fn locale_string(vm: &mut Vm, args: &Args, wants: LocaleWants) -> Run<Value> {
    let time = this_time(vm, args.this)?;
    if time.is_nan() {
        return Ok(vm.new_str("Invalid Date"));
    }
    let options = read_locale_options(vm, args)?;
    let at = if options.in_utc {
        time
    } else {
        local_time(time)
    };
    let fields = fields_of(at);
    let locale = options.locale.clone();
    let locale = locale.as_deref();
    let chosen = options.any_date() || options.any_time();
    let show_date = if chosen {
        options.any_date()
    } else {
        wants != LocaleWants::Time
    };
    let show_time = if chosen {
        options.any_time()
    } else {
        wants != LocaleWants::Date
    };
    let mut pieces = Vec::new();
    if show_date {
        let picture = options.any_date().then(|| date_picture(&options));
        let flags = if picture.is_some() { 0 } else { DATE_SHORT };
        let made =
            system::format(locale, &fields, true, picture.as_deref(), flags).unwrap_or_else(|| {
                format!(
                    "{}/{}/{}",
                    fields.month,
                    fields.day,
                    plain_year(fields.year)
                )
            });
        pieces.push(made);
    }
    if show_time {
        let picture = if options.any_time() {
            time_picture(&options)
        } else {
            None
        };
        let made =
            system::format(locale, &fields, false, picture.as_deref(), 0).unwrap_or_else(|| {
                format!(
                    "{:02}:{:02}:{:02}",
                    fields.hour, fields.minute, fields.second
                )
            });
        pieces.push(made);
    }
    let text = pieces.join(", ");
    Ok(vm.new_str(&match options.numbering.as_deref() {
        Some(system) => super::locale::digits_in(&text, system, false),
        None => text,
    }))
}

fn to_locale_string(vm: &mut Vm, args: Args) -> Run<Value> {
    locale_string(vm, &args, LocaleWants::Both)
}

fn to_locale_date_string(vm: &mut Vm, args: Args) -> Run<Value> {
    locale_string(vm, &args, LocaleWants::Date)
}

fn to_locale_time_string(vm: &mut Vm, args: Args) -> Run<Value> {
    locale_string(vm, &args, LocaleWants::Time)
}

fn get_timezone_offset(vm: &mut Vm, args: Args) -> Run<Value> {
    let time = this_time(vm, args.this)?;
    Ok(if time.is_nan() {
        Value::double(f64::NAN)
    } else {
        Value::number((time - local_time(time)) / 60_000.0)
    })
}

macro_rules! field_getter {
    ($utc:ident, $local:ident, $field:ident, $adjust:expr) => {
        fn $utc(vm: &mut Vm, args: Args) -> Run<Value> {
            let time = this_time(vm, args.this)?;
            if time.is_nan() {
                return Ok(Value::double(f64::NAN));
            }
            let adjust: i64 = $adjust;
            Ok(Value::number((fields_of(time).$field + adjust) as f64))
        }

        fn $local(vm: &mut Vm, args: Args) -> Run<Value> {
            let time = this_time(vm, args.this)?;
            if time.is_nan() {
                return Ok(Value::double(f64::NAN));
            }
            let adjust: i64 = $adjust;
            Ok(Value::number(
                (fields_of(local_time(time)).$field + adjust) as f64,
            ))
        }
    };
}

field_getter!(get_utc_year, get_year, year, 0);
field_getter!(get_utc_month, get_month, month, -1);
field_getter!(get_utc_date, get_date, day, 0);
field_getter!(get_utc_day, get_day, weekday, 0);
field_getter!(get_utc_hours, get_hours, hour, 0);
field_getter!(get_utc_minutes, get_minutes, minute, 0);
field_getter!(get_utc_seconds, get_seconds, second, 0);
field_getter!(get_utc_ms, get_ms, ms, 0);

/// Sets some of a date's parts, starting from part `first`. All the arguments are read before anything else happens, as
/// reading them can run code, and a date that isn't one stays that way, unless what's being set is the year.
fn set_parts(vm: &mut Vm, args: &Args, first: usize, most: usize, local: bool) -> Run<Value> {
    let time = this_time(vm, args.this)?;
    let mut given = [f64::NAN; 4];
    let count = most.min(args.count.max(1));
    for (offset, slot) in given.iter_mut().enumerate().take(count) {
        let value = vm.arg(args, offset);
        *slot = vm.to_number(value)?;
    }
    if time.is_nan() && first != 0 {
        return Ok(Value::double(f64::NAN));
    }
    let start = if time.is_nan() {
        0.0
    } else if local {
        local_time(time)
    } else {
        time
    };
    let fields = fields_of(start);
    let mut parts = [
        fields.year as f64,
        (fields.month - 1) as f64,
        fields.day as f64,
        fields.hour as f64,
        fields.minute as f64,
        fields.second as f64,
        fields.ms as f64,
    ];
    parts[first..first + count].copy_from_slice(&given[..count]);
    let made = make_time(&parts);
    let result = time_clip(if local { utc_from_local(made) } else { made });
    if let Kind::Date(found) = &mut vm.objects[args.this.index()].kind {
        *found = result;
    }
    Ok(Value::number(result))
}

macro_rules! field_setter {
    ($utc:ident, $local:ident, $first:expr, $most:expr) => {
        fn $utc(vm: &mut Vm, args: Args) -> Run<Value> {
            set_parts(vm, &args, $first, $most, false)
        }

        fn $local(vm: &mut Vm, args: Args) -> Run<Value> {
            set_parts(vm, &args, $first, $most, true)
        }
    };
}

field_setter!(set_utc_full_year, set_full_year, 0, 3);
field_setter!(set_utc_month, set_month, 1, 2);
field_setter!(set_utc_date, set_date, 2, 1);
field_setter!(set_utc_hours, set_hours, 3, 4);
field_setter!(set_utc_minutes, set_minutes, 4, 3);
field_setter!(set_utc_seconds, set_seconds, 5, 2);
field_setter!(set_utc_ms, set_ms, 6, 1);

fn set_time(vm: &mut Vm, args: Args) -> Run<Value> {
    this_time(vm, args.this)?;
    let value = vm.arg(&args, 0);
    let time = time_clip(vm.to_number(value)?);
    if let Kind::Date(found) = &mut vm.objects[args.this.index()].kind {
        *found = time;
    }
    Ok(Value::number(time))
}

/// An old way of getting the year, which counts from 1900.
fn get_old_year(vm: &mut Vm, args: Args) -> Run<Value> {
    let time = this_time(vm, args.this)?;
    if time.is_nan() {
        return Ok(Value::double(f64::NAN));
    }
    Ok(Value::number(
        (fields_of(local_time(time)).year - 1900) as f64,
    ))
}

fn set_old_year(vm: &mut Vm, args: Args) -> Run<Value> {
    let time = this_time(vm, args.this)?;
    let value = vm.arg(&args, 0);
    let year = vm.to_number(value)?;
    if year.is_nan() {
        if let Kind::Date(found) = &mut vm.objects[args.this.index()].kind {
            *found = f64::NAN;
        }
        return Ok(Value::double(f64::NAN));
    }
    let whole = year.trunc();
    let year = if (0.0..=99.0).contains(&whole) {
        whole + 1900.0
    } else {
        year
    };
    let fields = fields_of(if time.is_nan() { 0.0 } else { local_time(time) });
    let made = make_time(&[
        year,
        (fields.month - 1) as f64,
        fields.day as f64,
        fields.hour as f64,
        fields.minute as f64,
        fields.second as f64,
        fields.ms as f64,
    ]);
    let result = time_clip(utc_from_local(made));
    if let Kind::Date(found) = &mut vm.objects[args.this.index()].kind {
        *found = result;
    }
    Ok(Value::number(result))
}

/// A date turns into text unless it's asked for a number, which isn't what most objects do.
fn to_primitive(vm: &mut Vm, args: Args) -> Run<Value> {
    if !args.this.is_object() {
        return Err(vm.type_error("Date.prototype[Symbol.toPrimitive] needs an object"));
    }
    let hint = vm.arg(&args, 0);
    let wanted = if hint.is_string() {
        vm.text_of(hint).to_rust()
    } else {
        String::new()
    };
    let order = match wanted.as_str() {
        "string" | "default" => [text::TO_STRING, text::VALUE_OF],
        "number" => [text::VALUE_OF, text::TO_STRING],
        _ => return Err(vm.type_error("Invalid hint for a date")),
    };
    for name in order {
        let method = vm.get(args.this, name)?;
        if vm.is_callable(method) {
            let result = vm.call(method, args.this, &[])?;
            if !result.is_object() {
                return Ok(result);
            }
        }
    }
    Err(vm.type_error("Cannot convert object to primitive value"))
}

pub(super) fn install(vm: &mut Vm) {
    let proto = vm.plain_proto();
    vm.realm.date_proto = proto;
    let date = vm.constructor("Date", 7, date_constructor, proto);
    vm.method(date, "now", 0, now);
    vm.method(date, "parse", 1, parse);
    vm.method(date, "UTC", 7, utc);
    for (name, length, func) in [
        ("getTime", 0, get_time as NativeFn),
        ("valueOf", 0, get_time),
        ("toISOString", 0, to_iso_string),
        ("toJSON", 1, to_json),
        ("toString", 0, to_string),
        ("toUTCString", 0, to_utc_string),
        ("toDateString", 0, to_date_string),
        ("toTimeString", 0, to_time_string),
        ("toLocaleString", 0, to_locale_string),
        ("toLocaleDateString", 0, to_locale_date_string),
        ("toLocaleTimeString", 0, to_locale_time_string),
        ("getTimezoneOffset", 0, get_timezone_offset),
        ("getFullYear", 0, get_year),
        ("getUTCFullYear", 0, get_utc_year),
        ("getMonth", 0, get_month),
        ("getUTCMonth", 0, get_utc_month),
        ("getDate", 0, get_date),
        ("getUTCDate", 0, get_utc_date),
        ("getDay", 0, get_day),
        ("getUTCDay", 0, get_utc_day),
        ("getHours", 0, get_hours),
        ("getUTCHours", 0, get_utc_hours),
        ("getMinutes", 0, get_minutes),
        ("getUTCMinutes", 0, get_utc_minutes),
        ("getSeconds", 0, get_seconds),
        ("getUTCSeconds", 0, get_utc_seconds),
        ("getMilliseconds", 0, get_ms),
        ("getUTCMilliseconds", 0, get_utc_ms),
        ("getYear", 0, get_old_year),
        ("setYear", 1, set_old_year),
        ("setTime", 1, set_time),
        ("setFullYear", 3, set_full_year),
        ("setUTCFullYear", 3, set_utc_full_year),
        ("setMonth", 2, set_month),
        ("setUTCMonth", 2, set_utc_month),
        ("setDate", 1, set_date),
        ("setUTCDate", 1, set_utc_date),
        ("setHours", 4, set_hours),
        ("setUTCHours", 4, set_utc_hours),
        ("setMinutes", 3, set_minutes),
        ("setUTCMinutes", 3, set_utc_minutes),
        ("setSeconds", 2, set_seconds),
        ("setUTCSeconds", 2, set_utc_seconds),
        ("setMilliseconds", 1, set_ms),
        ("setUTCMilliseconds", 1, set_utc_ms),
    ] {
        vm.method(proto, name, length, func);
    }
    // These two are the same function, as the language says.
    let utc_string = vm.intern_str("toUTCString");
    if let Ok(function) = vm.get(Value::object(proto), utc_string) {
        let gmt_string = vm.intern_str("toGMTString");
        vm.add_property(proto, gmt_string, function, HIDDEN);
    }
    let function = vm.symbol_method(
        proto,
        text::SYMBOL_TO_PRIMITIVE,
        "[Symbol.toPrimitive]",
        1,
        to_primitive,
    );
    vm.define(proto, text::SYMBOL_TO_PRIMITIVE, function, CONFIGURABLE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_and_dates_turn_into_each_other() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
        assert_eq!(civil_from_days(11017), (2000, 3, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn iso_dates_read_and_write() {
        assert_eq!(parse_date("1970-01-01T00:00:00.000Z"), 0.0);
        assert_eq!(parse_date("2000-01-01"), 946_684_800_000.0);
        assert_eq!(parse_date("2000-01-01T01:00:00+01:00"), 946_684_800_000.0);
        assert!(parse_date("not a date").is_nan());
        assert_eq!(parse_date("2000-02-30"), parse_date("2000-03-01"));
        assert!(parse_date("2000-02-32").is_nan());
        assert!(parse_date("-000000-01-01T00:00:00Z").is_nan());
        assert_eq!(iso_string(946_684_800_000.0), "2000-01-01T00:00:00.000Z");
    }

    #[test]
    fn a_date_and_time_with_nothing_said_about_where_is_local() {
        let local = parse_date("2024-07-15T12:30:00");
        let utc = parse_date("2024-07-15T12:30:00Z");
        assert_eq!(utc - local, system::offset_at(local));
    }

    #[test]
    fn what_this_writes_it_can_read_back() {
        let time = 1_721_046_600_000.0;
        assert_eq!(parse_date(&date_string(time)), time);
        assert_eq!(parse_date("Mon, 15 Jul 2024 12:30:00 GMT"), time);
        assert_eq!(parse_date("Jul 15, 2024 12:30:00 GMT"), time);
        assert_eq!(parse_date("15 July 2024 12:30:00 UTC"), time);
        assert_eq!(parse_date("7/15/2024 12:30:00 PM GMT"), time);
        assert_eq!(parse_date("2024/07/15 12:30 GMT"), time);
    }

    #[test]
    fn local_time_going_there_and_back_is_where_it_started() {
        // A moment every 13 hours over several years, which covers any clock changes.
        let mut time = 1_577_836_800_000.0;
        while time < 1_735_689_600_000.0 {
            let local = local_time(time);
            let back = utc_from_local(local);
            // Times that come round twice can go to the first of them, which is an hour away at most.
            assert!(
                back == time || (back - time).abs() <= 3_600_000.0,
                "{time} came back as {back}"
            );
            time += 13.0 * 3_600_000.0;
        }
    }
}
