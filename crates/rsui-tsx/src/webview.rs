//! Web pages from elsewhere, for `rsui/webview` (`webview.js`). In a window that's really running, each `<WebView>` is a
//! WebView2 view kept where its element is, through `rsui::embed`, and the browser is started only when the first one's
//! there. Otherwise, as with `--do`, `--agent`, `--png` and in tests, nothing's started: what would load is said, a load
//! comes back as one that worked, titled with the address's host, and the `webview` steps make the page do the rest.
//!
//! What happens in a view goes to the page as JSON, through the desktop's events.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use rsui_js::{Args, HostFunction, Value, Vm};
use rsui_webview::{Bounds, Event, Key, Settings, WebView};
use serde_json::{Value as Json, json};

use crate::bridge;
use crate::native::{number_at, text_at};

pub(crate) const FUNCTIONS: [(&str, HostFunction); 5] = [
    ("webviewOpen", host_open),
    ("webviewDo", host_do),
    ("webviewAsk", host_ask),
    ("webviewClose", host_close),
    ("webviewRuntime", host_runtime),
];

type KeptKeys = Vec<(Key, String)>;

/// A view that isn't really there, and what it would have.
#[derive(Default)]
struct Pretend {
    action: u32,
    keys: KeptKeys,
    history: Vec<String>,
    at: usize,
    title: String,
}

struct NativeView {
    view: WebView,
    keys: Rc<RefCell<KeptKeys>>,
}

thread_local! {
    static PRETEND: RefCell<HashMap<u64, Pretend>> = RefCell::default();
    /// Real views, beside what `rsui::embed` holds, which is what keeps them, for what's asked of them by number.
    static REAL: RefCell<HashMap<u64, Weak<NativeView>>> = RefCell::default();
    /// Which view the `webview` steps are about: the last one opened that's still there.
    static LAST: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

/// Tells the page what happened in view `id`, through `action`.
fn tell(action: u32, id: u64, mut said: Json) {
    if let Json::Object(fields) = &mut said {
        fields.insert("id".to_owned(), json!(id));
    }
    bridge::fire_desktop_event(action, Some(said.to_string()));
}

/// A key the app keeps, by the name it was given as, such as `Ctrl+T`.
fn key_json(key: &Key, name: &str) -> Json {
    json!({ "name": name, "code": key.code, "ctrl": key.ctrl, "shift": key.shift, "alt": key.alt })
}

fn key_names(data: &Json) -> KeptKeys {
    data.as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let name = value.as_str()?;
            Some((Key::parse(name)?, name.to_owned()))
        })
        .collect()
}

fn name_of_key(key: Key, keys: &[(Key, String)]) -> Option<&str> {
    keys.iter().find(|(known, _)| *known == key).map(|(_, name)| name.as_str())
}

fn event_json(event: &Event, keys: &[(Key, String)]) -> Option<Json> {
    Some(match event {
        Event::Ready => json!({ "type": "ready" }),
        Event::Unavailable(why) => json!({ "type": "unavailable", "message": why }),
        Event::Navigating { url, redirected, user } => {
            json!({ "type": "navigating", "url": url, "redirected": redirected, "user": user })
        }
        Event::Navigated { url } => json!({ "type": "navigated", "url": url }),
        Event::NavigatedInPage { url } => json!({ "type": "inPage", "url": url }),
        Event::DomReady => json!({ "type": "domReady" }),
        Event::Finished => json!({ "type": "finished" }),
        Event::Failed { url, code, description } => {
            json!({ "type": "failed", "url": url, "code": code, "description": description })
        }
        Event::Title(title) => json!({ "type": "title", "title": title }),
        Event::NewWindow { url, user } => json!({ "type": "newWindow", "url": url, "user": user }),
        Event::Message(message) => json!({ "type": "message", "json": message }),
        Event::Focus(on) => json!({ "type": "focus", "on": on }),
        Event::FocusLeft { forward } => json!({ "type": "focusLeft", "forward": forward }),
        Event::Crashed { kind } => json!({ "type": "crashed", "kind": kind }),
        Event::CloseRequested => json!({ "type": "close" }),
        Event::History => json!({ "type": "history" }),
        Event::Key(key) => {
            json!({ "type": "key", "key": key_json(key, name_of_key(*key, keys)?) })
        }
        Event::FullScreen(on) => json!({ "type": "fullScreen", "on": on }),
        // Still pictures are moved straight into the renderer by the event sink.
        Event::Still(_) => return None,
        Event::ScriptDone { call, result } => match result {
            Ok(value) => json!({ "type": "script", "call": call, "json": value }),
            Err(error) => json!({ "type": "script", "call": call, "error": error }),
        },
        // Cookies one by one aren't asked for here: a view can only forget all of them.
        Event::Cookies { .. } | Event::CookieSet { .. } => return None,
    })
}

/// The browser's own folder for an app: what pages keep, their cookies, storage and cache, which is the app's and never
/// the person's own Edge's.
fn data_folder() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    rsui_webview::data_folder(&base.join(bridge::program_name()))
}

/// The host of an address, which is what a page that's only pretended is titled.
fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    host.rsplit_once('@').map_or(host, |(_, host)| host).to_owned()
}

fn host_runtime(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    Ok(match rsui_webview::runtime::find() {
        Some(runtime) => vm.new_string(&runtime.version),
        None => vm.new_string(""),
    })
}

/// Opens view `id`: `webviewOpen(id, action, settings)`, with what it's set up with as JSON: `profile`, `private`,
/// `userAgent`, `keys` (as `Ctrl+T`) and `scripts`.
fn host_open(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = u64::from(number_at(vm, args, 0));
    let action = number_at(vm, args, 1);
    let given: Json = serde_json::from_str(&text_at(vm, args, 2)).unwrap_or(Json::Null);
    let text = |name: &str| given.get(name).and_then(Json::as_str).map(str::to_owned).filter(|text| !text.is_empty());
    let texts = |name: &str| -> Vec<String> {
        given.get(name).and_then(Json::as_array).map(|all| all.iter().filter_map(Json::as_str).map(str::to_owned).collect()).unwrap_or_default()
    };
    let profile = text("profile");
    let private = given.get("private").and_then(Json::as_bool).unwrap_or(false);
    let keys = key_names(&given["keys"]);
    LAST.with(|last| {
        let mut last = last.borrow_mut();
        last.retain(|one| *one != id);
        last.push(id);
    });
    if !crate::native::real() {
        let whose = match (&profile, private) {
            (_, true) => " privately".to_owned(),
            (Some(profile), false) => format!(", in the profile {profile}"),
            (None, false) => String::new(),
        };
        bridge::say(format!("it would show a web page in WebView2{whose}"));
        PRETEND.with(|views| views.borrow_mut().insert(id, Pretend { action, keys, ..Pretend::default() }));
        return Ok(Value::UNDEFINED);
    }
    let settings = Settings {
        data_folder: data_folder(),
        profile: profile.map(|name| rsui_webview::profile_name(&name)),
        in_private: private,
        user_agent: text("userAgent"),
        scripts: texts("scripts"),
        dev_tools: true,
        keys: keys.iter().map(|(key, _)| *key).collect(),
    };
    let keys = Rc::new(RefCell::new(keys));
    let event_keys = Rc::clone(&keys);
    let mut still = None;
    let view = WebView::new(settings, move |event| {
        let message = match event {
            Event::Still(Some(png)) => {
                let image = still.insert(rsui::render::MemoryImage::new(png.0));
                Some(json!({ "type": "still", "url": image.source() }))
            }
            Event::Still(None) => {
                still.take();
                Some(json!({ "type": "still", "url": null }))
            }
            other => event_json(&other, &event_keys.borrow()),
        };
        if let Some(said) = message {
            tell(action, id, said);
        }
    });
    let view = Rc::new(NativeView { view, keys });
    REAL.with(|real| real.borrow_mut().insert(id, Rc::downgrade(&view)));
    rsui::embed::embed(id, Box::new(Hosted(view)));
    Ok(Value::UNDEFINED)
}

/// A real view, kept where its element is.
struct Hosted(Rc<NativeView>);

impl rsui::embed::Embedded for Hosted {
    fn place(&mut self, window: isize, place: Option<&rsui::embed::Place>) {
        let sides = |at: &rsui::embed::PixelBox| [at.left, at.top, at.right, at.bottom];
        let bounds = place.map(|place| Bounds {
            rect: sides(&place.rect),
            shown: sides(&place.shown),
            covered: place.covered.iter().map(sides).collect(),
            see_through: place.see_through.iter().map(sides).collect(),
            still: place.still,
            radius: place.radius,
        });
        self.0.view.place(window, bounds.as_ref());
    }

    fn window_moved(&mut self) {
        self.0.view.window_moved();
    }
}

fn real_view(id: u64) -> Option<Rc<NativeView>> {
    REAL.with(|real| real.borrow().get(&id).and_then(Weak::upgrade))
}

/// Does something to view `id`: `webviewDo(id, what, given, call)`.
fn host_do(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = u64::from(number_at(vm, args, 0));
    let what = text_at(vm, args, 1);
    let given = text_at(vm, args, 2);
    let call = u64::from(number_at(vm, args, 3));
    let native = real_view(id);
    if what == "keys" {
        let keys = key_names(&serde_json::from_str(&given).unwrap_or(Json::Null));
        if let Some(native) = native {
            native.view.set_keys(keys.iter().map(|(key, _)| *key).collect());
            *native.keys.borrow_mut() = keys;
        } else {
            with_view(id, |view| view.keys = keys);
        }
        return Ok(Value::UNDEFINED);
    }
    if let Some(native) = native {
        let view = &native.view;
        match what.as_str() {
            "navigate" => view.navigate(&given),
            "back" => view.back(),
            "forward" => view.forward(),
            "reload" => view.reload(),
            "stop" => view.stop(),
            "script" => view.execute(&given, call),
            "post" => view.post(&given),
            "userAgent" => view.set_user_agent(&given),
            "devTools" => view.open_dev_tools(),
            "focus" => view.focus(),
            "blur" => view.blur(),
            "zoom" => view.set_zoom(given.parse().unwrap_or(1.0)),
            "clearCookies" => view.clear_cookies(),
            _ => return Err(vm.type_error(&format!("a web view can't {what}"))),
        }
        return Ok(Value::UNDEFINED);
    }
    pretend_do(id, &what, &given, call).map_err(|message| vm.type_error(&message))?;
    Ok(Value::UNDEFINED)
}

fn with_view<R>(id: u64, run: impl FnOnce(&mut Pretend) -> R) -> Option<R> {
    PRETEND.with(|views| views.borrow_mut().get_mut(&id).map(run))
}

/// Goes to `url` in a view that isn't there, as following a link would, dropping what was ahead of it.
fn pretend_go(id: u64, url: &str) {
    with_view(id, |view| {
        if !view.history.is_empty() {
            view.history.truncate(view.at + 1);
        }
        view.history.push(url.to_owned());
        view.at = view.history.len() - 1;
    });
}

/// What a page that loads says, in the order it says it.
fn pretend_load(id: u64, action: u32, url: &str, user: bool) {
    let title = host_of(url);
    with_view(id, |view| view.title = title.clone());
    tell(action, id, json!({ "type": "navigating", "url": url, "redirected": false, "user": user }));
    tell(action, id, json!({ "type": "navigated", "url": url }));
    tell(action, id, json!({ "type": "history" }));
    tell(action, id, json!({ "type": "title", "title": title }));
    tell(action, id, json!({ "type": "domReady" }));
    tell(action, id, json!({ "type": "finished" }));
}

/// What a view that isn't there does, as a page that loads would.
fn pretend_do(id: u64, what: &str, given: &str, call: u64) -> Result<(), String> {
    let Some(action) = with_view(id, |view| view.action) else {
        return Ok(());
    };
    let step = |forward: bool| {
        with_view(id, |view| {
            let to = if forward { view.at + 1 } else { view.at.checked_sub(1)? };
            let url = view.history.get(to)?.clone();
            view.at = to;
            Some(url)
        })
        .flatten()
    };
    match what {
        "navigate" => {
            bridge::say(format!("it would load {given}"));
            pretend_go(id, given);
            pretend_load(id, action, given, false);
        }
        "back" | "forward" => {
            if let Some(url) = step(what == "forward") {
                bridge::say(format!("it would go {what} to {url}"));
                pretend_load(id, action, &url, false);
            }
        }
        "reload" => {
            if let Some(url) = with_view(id, |view| view.history.get(view.at).cloned()).flatten() {
                bridge::say(format!("it would load {url} again"));
                pretend_load(id, action, &url, false);
            }
        }
        "stop" | "focus" | "blur" | "zoom" | "userAgent" => {}
        "devTools" => bridge::say("it would open the page's developer tools".to_owned()),
        "clearCookies" => bridge::say("it would forget the cookies of the view's profile".to_owned()),
        // There's no page to run it in, so it comes to nothing, as it would on a blank page.
        "script" => tell(action, id, json!({ "type": "script", "call": call, "json": "null" })),
        "post" => bridge::say(format!("it would send the page {given}")),
        _ => return Err(format!("a web view can't {what}")),
    }
    Ok(())
}

/// Asks view `id` something that's answered right away: `url`, `title`, `back` or `forward`, whether it can go there.
fn host_ask(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = u64::from(number_at(vm, args, 0));
    let what = text_at(vm, args, 1);
    if let Some(native) = real_view(id) {
        let view = &native.view;
        return Ok(match what.as_str() {
            "url" => view.url().map_or(Value::UNDEFINED, |url| vm.new_string(&url)),
            "title" => view.title().map_or(Value::UNDEFINED, |title| vm.new_string(&title)),
            "back" => Value::bool(view.history().0),
            "forward" => Value::bool(view.history().1),
            _ => Value::UNDEFINED,
        });
    }
    let answer = with_view(id, |view| match what.as_str() {
        "url" => view.history.get(view.at).cloned().map(Json::from),
        "title" => Some(Json::from(view.title.clone())),
        "back" => Some(Json::from(view.at > 0 && !view.history.is_empty())),
        "forward" => Some(Json::from(view.at + 1 < view.history.len())),
        _ => None,
    })
    .flatten();
    Ok(match answer {
        Some(Json::String(text)) => vm.new_string(&text),
        Some(Json::Bool(on)) => Value::bool(on),
        _ => Value::UNDEFINED,
    })
}

fn host_close(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = u64::from(number_at(vm, args, 0));
    forget(id);
    Ok(Value::UNDEFINED)
}

fn forget(id: u64) {
    LAST.with(|last| last.borrow_mut().retain(|one| *one != id));
    PRETEND.with(|views| views.borrow_mut().remove(&id));
    REAL.with(|real| real.borrow_mut().remove(&id));
    drop(rsui::embed::unembed(id));
}

/// Everything goes, for a new version of the app's code to make again.
pub(crate) fn let_go_of_everything() {
    let mut all: Vec<u64> = LAST.with(|last| last.borrow().clone());
    all.extend(REAL.with(|real| real.borrow().keys().copied().collect::<Vec<_>>()));
    for id in all {
        forget(id);
    }
}

/// The `webview` steps of `--do`: what the page in the last view opened does, as it isn't there.
///
/// - `webview load URL`: it goes there itself, as following a link does, and loads.
/// - `webview inpage URL`: it goes somewhere else in the same page.
/// - `webview fail URL [CODE]`: going there fails, with one of Chromium's errors, -105 unless it says.
/// - `webview title TEXT`: its title changes.
/// - `webview popup URL`: it asks for another window, as a link with `target=_blank` does.
/// - `webview key KEYS`: one of the keys the app keeps is pressed in it, such as `Ctrl+T`.
/// - `webview fullscreen on|off`: something in it goes full screen, or comes back.
/// - `webview message JSON`: it sends the app this, with `chrome.webview.postMessage`.
/// - `webview close`, `webview crash`, `webview unavailable`: it asks to be closed, its process goes, or there's no
///   WebView2 on this computer.
pub(crate) fn step(words: &[String]) -> Result<(), String> {
    let id = LAST.with(|last| last.borrow().last().copied()).ok_or("there's no web view")?;
    let action = with_view(id, |view| view.action).ok_or("the web view is a real one, which the page in it uses")?;
    let word = |at: usize| words.get(at).cloned().ok_or_else(|| format!("webview {} needs more", words.first().map_or("", String::as_str)));
    match words.first().map(String::as_str) {
        Some("load") => {
            let url = word(1)?;
            pretend_go(id, &url);
            pretend_load(id, action, &url, true);
        }
        Some("inpage") => {
            let url = word(1)?;
            pretend_go(id, &url);
            tell(action, id, json!({ "type": "inPage", "url": url }));
            tell(action, id, json!({ "type": "history" }));
        }
        Some("fail") => {
            let url = word(1)?;
            let code: i32 = words.get(2).map_or(Ok(-105), |code| code.parse()).map_err(|_| "an error is a number, like -105".to_owned())?;
            let description = (0..=20).map(rsui_webview::net_error).find(|(known, _)| *known == code).map_or("ERR_FAILED", |(_, name)| name);
            pretend_go(id, &url);
            tell(action, id, json!({ "type": "navigating", "url": url, "redirected": false, "user": true }));
            tell(action, id, json!({ "type": "failed", "url": url, "code": code, "description": description }));
        }
        Some("title") => {
            let title = words[1..].join(" ");
            with_view(id, |view| view.title = title.clone());
            tell(action, id, json!({ "type": "title", "title": title }));
        }
        Some("popup") => tell(action, id, json!({ "type": "newWindow", "url": word(1)?, "user": true })),
        Some("key") => {
            let keys = word(1)?;
            let key = Key::parse(&keys).ok_or_else(|| format!("{keys} isn't a key, like Ctrl+T"))?;
            let name = with_view(id, |view| name_of_key(key, &view.keys).map(str::to_owned)).flatten();
            if let Some(name) = name {
                tell(action, id, json!({ "type": "key", "key": key_json(&key, &name) }));
            }
        }
        Some("fullscreen") => tell(action, id, json!({ "type": "fullScreen", "on": word(1)? != "off" })),
        Some("message") => tell(action, id, json!({ "type": "message", "json": words[1..].join(" ") })),
        Some("close") => tell(action, id, json!({ "type": "close" })),
        Some("crash") => tell(action, id, json!({ "type": "crashed", "kind": 1 })),
        Some("unavailable") => tell(action, id, json!({ "type": "unavailable", "message": rsui_webview::runtime::missing() })),
        _ => {
            return Err(
                "webview takes load, inpage, fail, title, popup, key, fullscreen, message, close, crash or unavailable".to_owned(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pretended_page_is_titled_with_its_host() {
        assert_eq!(host_of("https://user@news.example.com:8080/a?b#c"), "news.example.com:8080");
        assert_eq!(host_of("about:blank"), "about:blank");
    }
}
