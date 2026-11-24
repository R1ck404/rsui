//! `native` from `rsui`: the system's own popups, which `native_popups.js` asks for, and `rsui::popups` shows. As with the
//! rest of what's asked of the desktop, they only come up for a program that's really running: with `--do` and `--agent`
//! and in tests what's asked is said, and the `dialog` step says what the next one's answered with.

use rsui::popups::{Answer, Day, Popup, TimeOfDay};
use rsui_js::{Args, HostFunction, Value, Vm};
use serde_json::Value as Json;

use crate::bridge;
use crate::native::{failed, number_at, real, text_at};

/// What JavaScript is told, through its desktop events: `=` and the answer, `-` for none, or `!` and why it couldn't be
/// asked.
fn said(answer: Result<Answer, String>) -> String {
    match answer {
        Ok(Answer::Confirmed(go)) => format!("={go}"),
        Ok(Answer::Done | Answer::Chosen(None) | Answer::Day(None) | Answer::Time(None)) => "-".to_owned(),
        Ok(Answer::Chosen(Some(chosen))) => format!("={chosen}"),
        Ok(Answer::Day(Some(day))) => format!("={day}"),
        Ok(Answer::Time(Some(time))) => format!("={time}"),
        Err(reason) => format!("!{reason}"),
    }
}

/// The popup that `kind` and its options, as JSON, say.
fn popup_from(kind: &str, options: &str) -> Result<Popup, String> {
    let options: Json = serde_json::from_str(options).map_err(|error| error.to_string())?;
    let text = |name: &str| options.get(name).and_then(Json::as_str).unwrap_or_default().to_owned();
    let given = |name: &str| options.get(name).and_then(Json::as_str).map(str::to_owned);
    let day = |name: &str| match options.get(name).and_then(Json::as_str) {
        None => Ok(None),
        Some(said) => Day::parse(said).map(Some).ok_or_else(|| format!("{said:?} isn't a day")),
    };
    Ok(match kind {
        "confirm" => Popup::Confirm {
            title: text("title"),
            message: text("message"),
            confirm: given("confirm"),
            cancel: given("cancel"),
            destructive: options.get("destructive").and_then(Json::as_bool).unwrap_or(false),
        },
        "alert" => Popup::Alert { title: text("title"), message: text("message"), ok: given("ok") },
        "sheet" => {
            let list = options.get("options").and_then(Json::as_array).ok_or("a sheet needs its options, as a list")?;
            let options_given: Vec<String> = list
                .iter()
                .map(|option| option.as_str().map(str::to_owned).ok_or("each of a sheet's options is text"))
                .collect::<Result<_, _>>()?;
            if options_given.is_empty() {
                return Err("a sheet needs something to choose".to_owned());
            }
            Popup::Sheet { title: text("title"), options: options_given, cancel: given("cancel") }
        }
        "date" => Popup::PickDate { value: day("value")?, min: day("min")?, max: day("max")? },
        "time" => Popup::PickTime {
            value: match given("value") {
                None => None,
                Some(said) => Some(TimeOfDay::parse(&said).ok_or_else(|| format!("{said:?} isn't a time of day"))?),
            },
        },
        "share" => Popup::Share { title: text("title"), text: text("text"), url: text("url") },
        _ => return Err(format!("there's no popup called {kind:?}")),
    })
}

/// What a popup gets when nobody's there: what the `dialog` step said to give the next one, or otherwise it's cancelled.
/// What it's asked, and what it got, is said.
fn pretend(popup: &Popup) -> Result<Answer, String> {
    let cancelled = |answer: &str| ["cancel", "no", "false"].iter().any(|word| answer.eq_ignore_ascii_case(word));
    match popup {
        Popup::Confirm { title, message, cancel, .. } => {
            let answer = bridge::next_dialog_answer();
            let go = answer.as_deref().is_some_and(|answer| {
                !cancelled(answer) && !cancel.as_deref().is_some_and(|cancel| cancel.eq_ignore_ascii_case(answer))
            });
            bridge::say(format!("it asked to confirm {title:?}, {message:?}, and got {go}"));
            Ok(Answer::Confirmed(go))
        }
        Popup::Alert { title, message, .. } => {
            bridge::say(format!("it showed the message {title:?}, {message:?}"));
            Ok(Answer::Done)
        }
        Popup::Sheet { title, options, .. } => {
            let chosen = match bridge::next_dialog_answer() {
                None => None,
                Some(answer) if cancelled(&answer) => None,
                Some(answer) => match options.iter().position(|option| *option == answer) {
                    Some(at) => Some(at),
                    None => match answer.parse::<usize>() {
                        Ok(at) if at < options.len() => Some(at),
                        _ => return Err(format!("the sheet {title:?} doesn't have {answer:?}. It has {options:?}")),
                    },
                },
            };
            let got = chosen.map_or("nothing".to_owned(), |at| format!("{:?}", options[at]));
            bridge::say(format!("it asked {title:?} with {options:?}, and got {got}"));
            Ok(Answer::Chosen(chosen))
        }
        Popup::PickDate { min, max, .. } => {
            let picked = match bridge::next_dialog_answer() {
                None => None,
                Some(answer) if cancelled(&answer) => None,
                Some(answer) => {
                    let day = Day::parse(&answer).ok_or_else(|| format!("{answer:?} isn't a day, which is written as 2026-11-21"))?;
                    if min.is_some_and(|min| day < min) || max.is_some_and(|max| day > max) {
                        return Err(format!("{day} isn't one of the days that can be picked"));
                    }
                    Some(day)
                }
            };
            bridge::say(format!("it asked for a day, and got {}", picked.map_or("nothing".to_owned(), |day| day.to_string())));
            Ok(Answer::Day(picked))
        }
        Popup::PickTime { .. } => {
            let picked = match bridge::next_dialog_answer() {
                None => None,
                Some(answer) if cancelled(&answer) => None,
                Some(answer) => Some(
                    TimeOfDay::parse(&answer).ok_or_else(|| format!("{answer:?} isn't a time of day, which is written as 14:05"))?,
                ),
            };
            bridge::say(format!("it asked for a time, and got {}", picked.map_or("nothing".to_owned(), |time| time.to_string())));
            Ok(Answer::Time(picked))
        }
        Popup::Share { title, text, url } => {
            bridge::say(format!("it asked to share {title:?}, {text:?}, {url:?}"));
            Ok(Answer::Done)
        }
    }
}

/// `popupAsk(kind, options as JSON, what to tell)`, which tells it what was answered: see [`said`].
fn host_popup_ask(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (kind, options, id) = (text_at(vm, args, 0), text_at(vm, args, 1), number_at(vm, args, 2));
    let popup = popup_from(&kind, &options).map_err(|message| failed(vm, &message))?;
    if !real() {
        // It's told on the next turn, as it would be by the system.
        bridge::fire_desktop_event(id, Some(said(pretend(&popup))));
        return Ok(Value::UNDEFINED);
    }
    let tell = bridge::desktop_events_from_anywhere();
    rsui::popups::ask(popup, move |answer| tell(id, Some(said(answer))));
    Ok(Value::UNDEFINED)
}

/// `popupToast(text, long)`.
fn host_popup_toast(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let text = text_at(vm, args, 0);
    let long = args.get(vm, 1) == Value::TRUE;
    if !real() {
        bridge::say(format!("it showed the toast {text:?}{}", if long { ", for a while" } else { "" }));
        return Ok(Value::UNDEFINED);
    }
    rsui::popups::toast(&text, long).map_err(|message| failed(vm, &message))?;
    Ok(Value::UNDEFINED)
}

pub(crate) const FUNCTIONS: [(&str, HostFunction); 2] = [("popupAsk", host_popup_ask), ("popupToast", host_popup_toast)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_javascript_asks_for_is_the_popup_it_means() {
        assert_eq!(
            popup_from("confirm", r#"{"title":"Delete?","confirm":"Delete","destructive":true}"#),
            Ok(Popup::Confirm {
                title: "Delete?".into(),
                message: String::new(),
                confirm: Some("Delete".into()),
                cancel: None,
                destructive: true,
            })
        );
        assert_eq!(
            popup_from("date", r#"{"value":"2026-11-21","max":"2026-12-31"}"#),
            Ok(Popup::PickDate {
                value: Some(Day { year: 2026, month: 11, day: 21 }),
                min: None,
                max: Some(Day { year: 2026, month: 12, day: 31 }),
            })
        );
        assert!(popup_from("sheet", r#"{"options":[]}"#).is_err());
        assert!(popup_from("sheet", r#"{"options":[1]}"#).is_err());
        assert!(popup_from("time", r#"{"value":"25:00"}"#).is_err());
        assert!(popup_from("menu", "{}").is_err());
    }

    #[test]
    fn javascript_is_told_the_answer_or_that_there_was_none() {
        assert_eq!(said(Ok(Answer::Confirmed(false))), "=false");
        assert_eq!(said(Ok(Answer::Chosen(Some(2)))), "=2");
        assert_eq!(said(Ok(Answer::Chosen(None))), "-");
        assert_eq!(said(Ok(Answer::Time(Some(TimeOfDay { hours: 9, minutes: 5 })))), "=09:05");
        assert_eq!(said(Err("no".into())), "!no");
    }
}
