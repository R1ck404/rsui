//! The system's own popups, each asked for with one call: a question to confirm, a message, a sheet of choices, a day or a
//! time of day to pick, the share sheet, and a toast. None of them is drawn by rsui, so nothing in the app covers or clips
//! them, and nothing is loaded for them until one's shown.
//!
//! On Android they're Android's own: `AlertDialog`, a list `AlertDialog` for a sheet, `DatePickerDialog`,
//! `TimePickerDialog`, the share chooser and `Toast`, which rsui's activity shows (`RsuiActivity.java` in rsui-android).
//! On Windows the questions, messages and sheets are task dialogs, the share sheet is the system's, and the pickers are a
//! dialog around Windows' own month calendar or time control, as Windows has no picker dialog of its own. Its toast is a
//! small window of its own, under the middle of the app's window, that fades away by itself, as Windows has nothing like it.
//! Elsewhere none of these is there yet, and each says so.
//!
//! [`ask`] doesn't wait: what was answered is handed to `answered` when it's known, on another thread.

#[cfg(target_os = "android")]
pub mod android;
#[cfg(windows)]
mod win32;

#[cfg(windows)]
#[doc(hidden)]
pub use self::win32::{FIRST_OPTION, ask_on_this_thread, toast_on_this_thread};

/// A day, as a calendar has it, with no time or place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Day {
    pub year: i32,
    /// From 1 to 12.
    pub month: u8,
    /// From 1.
    pub day: u8,
}

impl Day {
    /// From `2026-11-21`.
    pub fn parse(text: &str) -> Option<Day> {
        let mut parts = text.trim().splitn(3, '-');
        let year = parts.next()?.parse().ok()?;
        let month: u8 = parts.next()?.parse().ok()?;
        let day: u8 = parts.next()?.parse().ok()?;
        ((1..=12).contains(&month) && (1..=days_in_month(year, month)).contains(&day)).then_some(Day { year, month, day })
    }
}

impl std::fmt::Display for Day {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// A time of day, on a 24-hour clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimeOfDay {
    /// From 0 to 23.
    pub hours: u8,
    /// From 0 to 59.
    pub minutes: u8,
}

impl TimeOfDay {
    /// From `14:05`.
    pub fn parse(text: &str) -> Option<TimeOfDay> {
        let (hours, minutes) = text.trim().split_once(':')?;
        let (hours, minutes): (u8, u8) = (hours.parse().ok()?, minutes.parse().ok()?);
        (hours < 24 && minutes < 60).then_some(TimeOfDay { hours, minutes })
    }
}

impl std::fmt::Display for TimeOfDay {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{:02}:{:02}", self.hours, self.minutes)
    }
}

/// What to ask.
#[derive(Debug, Clone, PartialEq)]
pub enum Popup {
    /// A question with a button that goes ahead, which says `confirm`, and one that doesn't, which says `cancel`.
    /// `destructive` is for going ahead with something that can't be undone, which the system shows as it shows that.
    /// Each button that isn't given says what the system's own does, in its language.
    Confirm { title: String, message: String, confirm: Option<String>, cancel: Option<String>, destructive: bool },
    /// Something to know, with one button, which says `ok`, or the system's own OK.
    Alert { title: String, message: String, ok: Option<String> },
    /// A list to choose one of, and `cancel` to choose none, or the system's own Cancel, which Windows always has, as its
    /// options are command links and so would a button of the app's own be.
    Sheet { title: String, options: Vec<String>, cancel: Option<String> },
    /// A day, starting from `value`, from `min` to `max`.
    PickDate { value: Option<Day>, min: Option<Day>, max: Option<Day> },
    /// A time of day, starting from `value`.
    PickTime { value: Option<TimeOfDay> },
    /// The system's share sheet, with this text and address, which `title` says what is.
    Share { title: String, text: String, url: String },
}

/// What was answered.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// To a [`Popup::Confirm`], whether to go ahead.
    Confirmed(bool),
    /// To a [`Popup::Alert`], which was seen, or a [`Popup::Share`], whose sheet came up. Where it was shared to isn't said.
    Done,
    /// To a [`Popup::Sheet`], which one, by its place in the list, or none.
    Chosen(Option<usize>),
    /// To a [`Popup::PickDate`], the day, or none.
    Day(Option<Day>),
    /// To a [`Popup::PickTime`], the time, or none.
    Time(Option<TimeOfDay>),
}

/// What's told what was answered, or why it couldn't be asked.
pub type Answered = Box<dyn FnOnce(Result<Answer, String>) + Send + 'static>;

/// Asks this, with the system's own popup, belonging to the app's window. It doesn't wait: `answered` is told, on another
/// thread, once it's answered, or straight away if it can't be asked here.
pub fn ask(popup: Popup, answered: impl FnOnce(Result<Answer, String>) + Send + 'static) {
    let answered: Answered = Box::new(answered);
    #[cfg(windows)]
    win32::ask(popup, answered);
    #[cfg(target_os = "android")]
    android::ask(popup, answered);
    #[cfg(not(any(windows, target_os = "android")))]
    {
        let _ = popup;
        answered(Err(NOT_HERE.to_owned()));
    }
}

/// Shows a little message that goes away by itself, for a short while or, with `long`, a longer one.
pub fn toast(text: &str, long: bool) -> Result<(), String> {
    #[cfg(windows)]
    return win32::toast(text, long);
    #[cfg(target_os = "android")]
    return android::toast(text, long);
    #[cfg(not(any(windows, target_os = "android")))]
    {
        let _ = (text, long);
        Err(NOT_HERE.to_owned())
    }
}

#[cfg(not(any(windows, target_os = "android")))]
const NOT_HERE: &str = "the system's popups aren't on this system yet";

/// What the answer to `popup` is, from what Android's activity says: `true` or `false`, a place in the list, a day as
/// `2026-11-21` or a time as `14:05`, with nothing being none.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn answer_from(popup: &Popup, said: Option<&str>) -> Result<Answer, String> {
    let unexpected = || format!("the popup's answer {said:?} doesn't make sense");
    Ok(match popup {
        Popup::Confirm { .. } => Answer::Confirmed(said == Some("true")),
        Popup::Alert { .. } | Popup::Share { .. } => Answer::Done,
        Popup::Sheet { options, .. } => match said {
            None => Answer::Chosen(None),
            Some(said) => {
                let chosen: usize = said.parse().map_err(|_| unexpected())?;
                if chosen >= options.len() {
                    return Err(unexpected());
                }
                Answer::Chosen(Some(chosen))
            }
        },
        Popup::PickDate { .. } => Answer::Day(match said {
            None => None,
            Some(said) => Some(Day::parse(said).ok_or_else(unexpected)?),
        }),
        Popup::PickTime { .. } => Answer::Time(match said {
            None => None,
            Some(said) => Some(TimeOfDay::parse(said).ok_or_else(unexpected)?),
        }),
    })
}

/// The popup as JSON, for Android's activity, which reads it with `org.json`.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn to_json(popup: &Popup) -> (&'static str, String) {
    fn text(out: &mut String, name: &str, value: &str) {
        if out.len() > 1 {
            out.push(',');
        }
        out.push('"');
        out.push_str(name);
        out.push_str("\":");
        quote(out, value);
    }
    fn maybe(out: &mut String, name: &str, value: &Option<String>) {
        if let Some(value) = value {
            text(out, name, value);
        }
    }
    fn quote(out: &mut String, value: &str) {
        out.push('"');
        for character in value.chars() {
            match character {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                character if u32::from(character) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(character))),
                character => out.push(character),
            }
        }
        out.push('"');
    }
    let mut out = String::from("{");
    let kind = match popup {
        Popup::Confirm { title, message, confirm, cancel, destructive } => {
            text(&mut out, "title", title);
            text(&mut out, "message", message);
            maybe(&mut out, "confirm", confirm);
            maybe(&mut out, "cancel", cancel);
            out.push_str(if *destructive { ",\"destructive\":true" } else { ",\"destructive\":false" });
            "confirm"
        }
        Popup::Alert { title, message, ok } => {
            text(&mut out, "title", title);
            text(&mut out, "message", message);
            maybe(&mut out, "ok", ok);
            "alert"
        }
        Popup::Sheet { title, options, cancel } => {
            text(&mut out, "title", title);
            maybe(&mut out, "cancel", cancel);
            out.push_str(",\"options\":[");
            for (at, option) in options.iter().enumerate() {
                if at > 0 {
                    out.push(',');
                }
                quote(&mut out, option);
            }
            out.push(']');
            "sheet"
        }
        Popup::PickDate { value, min, max } => {
            for (name, day) in [("value", value), ("min", min), ("max", max)] {
                if let Some(day) = day {
                    text(&mut out, name, &day.to_string());
                }
            }
            "date"
        }
        Popup::PickTime { value } => {
            if let Some(value) = value {
                text(&mut out, "value", &value.to_string());
            }
            "time"
        }
        Popup::Share { title, text: said, url } => {
            text(&mut out, "title", title);
            text(&mut out, "text", said);
            text(&mut out, "url", url);
            "share"
        }
    };
    out.push('}');
    (kind, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_and_times_read_back_what_they_say() {
        let day = Day { year: 2024, month: 2, day: 29 };
        assert_eq!(Day::parse(&day.to_string()), Some(day));
        assert_eq!(Day::parse("2023-02-29"), None);
        assert_eq!(Day::parse("2026-13-01"), None);
        assert_eq!(Day::parse("nonsense"), None);
        let time = TimeOfDay { hours: 7, minutes: 5 };
        assert_eq!(time.to_string(), "07:05");
        assert_eq!(TimeOfDay::parse("07:05"), Some(time));
        assert_eq!(TimeOfDay::parse("24:00"), None);
    }

    #[test]
    fn what_the_activity_says_is_the_answer_to_what_was_asked() {
        let sheet = Popup::Sheet { title: String::new(), options: vec!["a".into(), "b".into()], cancel: None };
        assert_eq!(answer_from(&sheet, Some("1")), Ok(Answer::Chosen(Some(1))));
        assert_eq!(answer_from(&sheet, None), Ok(Answer::Chosen(None)));
        assert!(answer_from(&sheet, Some("2")).is_err());
        let confirm = Popup::Confirm {
            title: "t".into(),
            message: String::new(),
            confirm: None,
            cancel: Some("Cancel".into()),
            destructive: false,
        };
        assert_eq!(answer_from(&confirm, Some("true")), Ok(Answer::Confirmed(true)));
        assert_eq!(answer_from(&confirm, Some("false")), Ok(Answer::Confirmed(false)));
        let date = Popup::PickDate { value: None, min: None, max: None };
        assert_eq!(answer_from(&date, Some("2026-11-21")), Ok(Answer::Day(Some(Day { year: 2026, month: 11, day: 21 }))));
    }

    #[test]
    fn the_activity_gets_json_with_what_needs_escaping_escaped() {
        let sheet = Popup::Sheet { title: "Say \"hi\"\n".into(), options: vec!["a\\b".into(), "c".into()], cancel: Some("No".into()) };
        let (kind, json) = to_json(&sheet);
        assert_eq!(kind, "sheet");
        assert_eq!(json, r#"{"title":"Say \"hi\"\n","cancel":"No","options":["a\\b","c"]}"#);
        let (kind, json) = to_json(&Popup::PickTime { value: Some(TimeOfDay { hours: 9, minutes: 30 }) });
        assert_eq!((kind, json.as_str()), ("time", r#"{"value":"09:30"}"#));
        let (_, json) = to_json(&Popup::PickDate { value: None, min: None, max: None });
        assert_eq!(json, "{}");
    }
}
