//! What a web view is told and tells, the same on every system: how it's set up, what happens in it, and where it goes.

use std::path::PathBuf;

/// How a view is set up, which can't change once it's made.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    /// Where the browser keeps what it keeps: cookies, storage and its cache. Views with the same folder share one set of
    /// browser processes.
    pub data_folder: PathBuf,
    /// Which of that folder's profiles it uses, each with cookies and storage of its own, or the default one.
    pub profile: Option<String>,
    /// Whether what it keeps is forgotten once it's closed, as a private window's is.
    pub in_private: bool,
    /// What it says it is, instead of the runtime's own.
    pub user_agent: Option<String>,
    /// Scripts run in each page before its own, in order.
    pub scripts: Vec<String>,
    /// Whether the developer tools can be opened.
    pub dev_tools: bool,
    /// Keys the app keeps for itself while a page has the keyboard, such as a browser's Ctrl+T: the page never sees them,
    /// and each is said as [`Event::Key`] instead.
    pub keys: Vec<Key>,
}

/// A key with the modifiers held for it, as Windows numbers keys.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Key {
    /// Its virtual-key code, such as `0x54` for T or `0x74` for F5.
    pub code: u32,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Key {
    /// Reads `Ctrl+Shift+T`, `Alt+Left` or `F5`: modifiers, then one key, a letter, a digit or one of the names here.
    pub fn parse(text: &str) -> Option<Self> {
        let mut key = Self::default();
        let mut code = None;
        for part in text.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => key.ctrl = true,
                "shift" => key.shift = true,
                "alt" => key.alt = true,
                name => code = Some(key_code(name)?),
            }
        }
        key.code = code?;
        Some(key)
    }
}

/// The virtual-key code of a key by its name, lower case.
fn key_code(name: &str) -> Option<u32> {
    let mut chars = name.chars();
    if let (Some(one), None) = (chars.next(), chars.clone().next())
        && one.is_ascii_alphanumeric()
    {
        return Some(u32::from(one.to_ascii_uppercase()));
    }
    if let Some(number) = name.strip_prefix('f').and_then(|rest| rest.parse::<u32>().ok())
        && (1..=24).contains(&number)
    {
        return Some(0x6F + number);
    }
    Some(match name {
        "tab" => 0x09,
        "enter" => 0x0D,
        "escape" | "esc" => 0x1B,
        "space" => 0x20,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "end" => 0x23,
        "home" => 0x24,
        "left" | "arrowleft" => 0x25,
        "up" | "arrowup" => 0x26,
        "right" | "arrowright" => 0x27,
        "down" | "arrowdown" => 0x28,
        "delete" => 0x2E,
        "=" | "plus" => 0xBB,
        "," => 0xBC,
        "-" | "minus" => 0xBD,
        "." => 0xBE,
        "/" => 0xBF,
        "[" => 0xDB,
        "\\" => 0xDC,
        "]" => 0xDD,
        _ => return None,
    })
}

/// A cookie, as WebView2 has it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    /// When it ends, in seconds since 1970, or `None` for one that ends with the session.
    pub expires: Option<f64>,
    pub http_only: bool,
    pub secure: bool,
    /// 0 for none, 1 for lax and 2 for strict, as WebView2 numbers them.
    pub same_site: i32,
}

/// What happened in a view.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// It's been made, and what was asked of it meanwhile has been done.
    Ready,
    /// It couldn't be made, and why.
    Unavailable(String),
    /// Going somewhere new, whether that's a redirect, and whether the person did it rather than code.
    Navigating { url: String, redirected: bool, user: bool },
    /// What's somewhere new has started arriving, so it's there now.
    Navigated { url: String },
    /// Somewhere else in the same page, as a link to a part of it or `history.pushState` does.
    NavigatedInPage { url: String },
    /// The page's own document has been read.
    DomReady,
    /// Everything's loaded.
    Finished,
    /// Getting somewhere failed, with the error's number and name as Chromium has them.
    Failed { url: String, code: i32, description: &'static str },
    Title(String),
    /// The page wants another window, which it doesn't get by itself.
    NewWindow { url: String, user: bool },
    /// A message from the page, as JSON, which it sent with `chrome.webview.postMessage`.
    Message(String),
    /// It's got the keyboard, or lost it.
    Focus(bool),
    /// Tab went past its first or last thing, so the keyboard's back with the window it's in.
    FocusLeft { forward: bool },
    /// A process it needs has gone, of the kind WebView2 numbers.
    Crashed { kind: i32 },
    /// The page asked to be closed, with `window.close()`.
    CloseRequested,
    /// Whether there's anything to go back or forward to may have changed.
    History,
    /// One of the [`Settings::keys`] was pressed while the page had the keyboard.
    Key(Key),
    /// Something in the page went full screen, as a video does, or came back.
    FullScreen(bool),
    /// A still picture of the page, for what it's in to draw while something see-through is over it, as a page can't be drawn
    /// over; or `None` once nothing is, for it to let go of the picture.
    Still(Option<Png>),
    /// What a script came to, as JSON.
    ScriptDone { call: u64, result: Result<String, String> },
    Cookies { call: u64, result: Result<Vec<Cookie>, String> },
    /// A cookie's been set, or couldn't be.
    CookieSet { call: u64, result: Result<(), String> },
}

/// Where it goes, in physical pixels from the top left of the window it's in, as `rsui::embed::Place` has it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bounds {
    /// Its whole box, as left, top, right and bottom.
    pub rect: [i32; 4],
    /// What of that shows.
    pub shown: [i32; 4],
    /// What of that's covered by something drawn on top of it.
    pub covered: Vec<[i32; 4]>,
    /// What of that's under something see-through drawn on top of it, as a dimmed backdrop is, which only shows over a still
    /// picture of the page (see [`Event::Still`]).
    pub see_through: Vec<[i32; 4]>,
    /// Whether what it's in is drawing the still picture of it, so it can be cut where it's seen through.
    pub still: bool,
    /// How round its corners are, which it's cut to.
    pub radius: i32,
}

/// A picture as a PNG file's bytes, which is only said by its size.
#[derive(Clone, PartialEq)]
pub struct Png(pub Vec<u8>);

impl std::fmt::Debug for Png {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a PNG of {} bytes", self.0.len())
    }
}

/// What a WebView2 error status is as one of Chromium's network errors, with its number and name.
pub fn net_error(status: i32) -> (i32, &'static str) {
    match status {
        1 => (-200, "ERR_CERT_COMMON_NAME_INVALID"),
        2 => (-201, "ERR_CERT_DATE_INVALID"),
        3 => (-117, "ERR_BAD_SSL_CLIENT_AUTH_CERT"),
        4 => (-206, "ERR_CERT_REVOKED"),
        5 => (-207, "ERR_CERT_INVALID"),
        6 => (-109, "ERR_ADDRESS_UNREACHABLE"),
        7 => (-7, "ERR_TIMED_OUT"),
        8 => (-320, "ERR_INVALID_RESPONSE"),
        9 => (-103, "ERR_CONNECTION_ABORTED"),
        10 => (-101, "ERR_CONNECTION_RESET"),
        11 => (-106, "ERR_INTERNET_DISCONNECTED"),
        12 => (-102, "ERR_CONNECTION_REFUSED"),
        13 => (-105, "ERR_NAME_NOT_RESOLVED"),
        14 => (-3, "ERR_ABORTED"),
        15 => (-310, "ERR_TOO_MANY_REDIRECTS"),
        16 => (-9, "ERR_UNEXPECTED"),
        17 => (-338, "ERR_INVALID_AUTH_CREDENTIALS"),
        18 => (-127, "ERR_PROXY_AUTH_REQUESTED"),
        _ => (-2, "ERR_FAILED"),
    }
}

