//! One web view: a window of WebView2's inside a window of this program's, where it's put.
//!
//! It's made the first time it's put somewhere, as WebView2 needs the window it's in, and that's when the runtime is
//! started, if nothing else has started it. Making it takes a moment, and what's asked of it meanwhile is done once it's
//! there, in order. It sits inside a window of its own, cut to what shows of its element, so that what scrolls around it
//! and what's drawn on top of it cut it off as they would on a page, and it only shows once its first page has arrived, so
//! what's drawn under it, its address and that it's loading, shows until then.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::{Rc, Weak};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_WINDOW, CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, HBRUSH, HGDIOBJ, RGN_DIFF, SetWindowRgn,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, HRESULT, HSTRING, Interface, PCWSTR, PWSTR, w};

use crate::ffi::*;
use crate::handler::{self, Made};
use crate::runtime;

pub use crate::types::{Bounds, Cookie, Event, Key, Png, Settings, net_error};

/// Where the still picture of the page is.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Still {
    /// There's none, as nothing see-through is over it.
    #[default]
    None,
    /// It's being taken.
    Taking,
    /// It's been said.
    Said,
}

#[derive(Debug, Clone)]
enum Command {
    Navigate(String),
    Back,
    Forward,
    Reload,
    Stop,
    Script(String, u64),
    Post(String),
    UserAgent(String),
    DevTools,
    Focus,
    Zoom(f64),
    GetCookies(String, u64),
    SetCookie(Cookie, u64),
    RemoveCookies(String, String),
    ClearCookies,
}

struct Live {
    controller: ICoreWebView2Controller,
    core: ICoreWebView2,
    /// What it's been told to call back, which it holds too, kept so that it lives as long.
    _handlers: Vec<Made>,
}

#[derive(Default)]
struct State {
    parent: Option<HWND>,
    host: Option<HWND>,
    live: Option<Live>,
    starting: bool,
    failed: Option<String>,
    queued: Vec<Command>,
    bounds: Option<Bounds>,
    /// Whether a page has arrived, before which what's under it shows.
    arrived: bool,
    still: Still,
}

struct Shared {
    settings: Settings,
    sink: RefCell<Box<dyn FnMut(Event)>>,
    state: RefCell<State>,
    /// Events that happened while one was being told, which follow it.
    pending: RefCell<Vec<Event>>,
    telling: Cell<bool>,
}

impl Shared {
    fn tell(&self, event: Event) {
        crate::log(|| format!("{event:?}"));
        self.pending.borrow_mut().push(event);
        if self.telling.replace(true) {
            return;
        }
        loop {
            let next = {
                let mut pending = self.pending.borrow_mut();
                if pending.is_empty() {
                    break;
                }
                pending.remove(0)
            };
            (self.sink.borrow_mut())(next);
        }
        self.telling.set(false);
    }
}

/// A web view. Dropping it closes it.
pub struct WebView(Rc<Shared>);

impl WebView {
    /// One that tells `sink` what happens in it. Nothing's made until it's put somewhere.
    pub fn new(settings: Settings, sink: impl FnMut(Event) + 'static) -> Self {
        Self(Rc::new(Shared {
            settings,
            sink: RefCell::new(Box::new(sink)),
            state: RefCell::new(State::default()),
            pending: RefCell::new(Vec::new()),
            telling: Cell::new(false),
        }))
    }

    /// Puts it in the window `parent`, a window handle, where `bounds` says, or hides it.
    pub fn place(&self, parent: isize, bounds: Option<&Bounds>) {
        let parent = HWND(parent as *mut c_void);
        crate::log(|| format!("put in window {parent:?} at {bounds:?}"));
        let start = {
            let mut state = self.0.state.borrow_mut();
            state.bounds = bounds.cloned();
            if bounds.is_some() && state.parent != Some(parent) {
                if let Some(host) = state.host {
                    // SAFETY: moving a window of this one's to another window of this thread's.
                    unsafe {
                        let _ = SetParent(host, Some(parent));
                    }
                }
                state.parent = Some(parent);
            }
            bounds.is_some() && state.live.is_none() && !state.starting && state.failed.is_none()
        };
        if start {
            self.start();
        }
        self.apply_bounds();
        keep_still(&self.0);
    }

    /// The window it's in has moved on the screen.
    pub fn window_moved(&self) {
        if let Some(live) = &self.0.state.borrow().live {
            // SAFETY: a call on a live controller.
            unsafe {
                let _ = live.controller.NotifyParentWindowPositionChanged();
            }
        }
    }

    pub fn navigate(&self, url: &str) {
        self.run(Command::Navigate(url.to_owned()));
    }

    pub fn back(&self) {
        self.run(Command::Back);
    }

    pub fn forward(&self) {
        self.run(Command::Forward);
    }

    pub fn reload(&self) {
        self.run(Command::Reload);
    }

    pub fn stop(&self) {
        self.run(Command::Stop);
    }

    /// Runs `script` in the page, and says what it came to, as JSON, with `call`.
    pub fn execute(&self, script: &str, call: u64) {
        self.run(Command::Script(script.to_owned(), call));
    }

    /// Sends the page a message, as JSON, which it hears with `chrome.webview.addEventListener('message')`.
    pub fn post(&self, json: &str) {
        self.run(Command::Post(json.to_owned()));
    }

    pub fn set_user_agent(&self, agent: &str) {
        self.run(Command::UserAgent(agent.to_owned()));
    }

    pub fn open_dev_tools(&self) {
        self.run(Command::DevTools);
    }

    /// Gives it the keyboard.
    pub fn focus(&self) {
        self.run(Command::Focus);
    }

    /// Gives the keyboard back to the window it's in, if the page has it.
    pub fn blur(&self) {
        let parent = self.0.state.borrow().parent;
        if let Some(parent) = parent {
            // SAFETY: a window of this thread's.
            unsafe {
                let _ = SetFocus(Some(parent));
            }
        }
    }

    pub fn set_zoom(&self, factor: f64) {
        self.run(Command::Zoom(factor));
    }

    /// The cookies `url` would be sent, or all of them if it's empty, said with `call`.
    pub fn cookies(&self, url: &str, call: u64) {
        self.run(Command::GetCookies(url.to_owned(), call));
    }

    pub fn set_cookie(&self, cookie: Cookie, call: u64) {
        self.run(Command::SetCookie(cookie, call));
    }

    /// Forgets the cookies called `name` that `url` would be sent.
    pub fn remove_cookies(&self, name: &str, url: &str) {
        self.run(Command::RemoveCookies(name.to_owned(), url.to_owned()));
    }

    pub fn clear_cookies(&self) {
        self.run(Command::ClearCookies);
    }

    /// Where it is now, once it's been made.
    pub fn url(&self) -> Option<String> {
        let state = self.0.state.borrow();
        let live = state.live.as_ref()?;
        // SAFETY: a call on a live view, whose answer is freed by `take`.
        unsafe {
            let mut uri = PWSTR::null();
            live.core.get_Source(&mut uri).ok().ok()?;
            Some(take(uri))
        }
    }

    pub fn title(&self) -> Option<String> {
        let state = self.0.state.borrow();
        let live = state.live.as_ref()?;
        // SAFETY: as for `url`.
        unsafe {
            let mut title = PWSTR::null();
            live.core.get_DocumentTitle(&mut title).ok().ok()?;
            Some(take(title))
        }
    }

    /// Whether there's somewhere to go back to, and forward to.
    pub fn history(&self) -> (bool, bool) {
        let state = self.0.state.borrow();
        let Some(live) = state.live.as_ref() else {
            return (false, false);
        };
        let (mut back, mut forward) = (BOOL(0), BOOL(0));
        // SAFETY: calls on a live view.
        unsafe {
            let _ = live.core.get_CanGoBack(&mut back);
            let _ = live.core.get_CanGoForward(&mut forward);
        }
        (back.as_bool(), forward.as_bool())
    }

    /// The process the browser's in, once it's made, for what wants to know what it costs.
    pub fn browser_process(&self) -> Option<u32> {
        let state = self.0.state.borrow();
        let live = state.live.as_ref()?;
        let mut id = 0;
        // SAFETY: a call on a live view.
        unsafe { live.core.get_BrowserProcessId(&mut id).ok().ok()? };
        Some(id)
    }

    fn run(&self, command: Command) {
        let ready = self.0.state.borrow().live.is_some();
        if ready {
            run_now(&self.0, command);
        } else if self.0.state.borrow().failed.is_none() {
            self.0.state.borrow_mut().queued.push(command);
        } else if let Some(event) = failed_event(&command, self.0.state.borrow().failed.as_deref().unwrap_or_default()) {
            self.0.tell(event);
        }
    }

    fn start(&self) {
        let host = {
            let mut state = self.0.state.borrow_mut();
            state.starting = true;
            let Some(parent) = state.parent else {
                return;
            };
            match make_host(parent) {
                Ok(host) => {
                    crate::log(|| format!("made its window {host:?} in {parent:?}"));
                    state.host = Some(host);
                    host
                }
                Err(error) => {
                    drop(state);
                    fail(&self.0, error);
                    return;
                }
            }
        };
        let weak = Rc::downgrade(&self.0);
        runtime::environment(&self.0.settings.data_folder, move |environment| {
            let Some(shared) = weak.upgrade() else {
                return;
            };
            match environment {
                Ok(environment) => {
                    if let Err(error) = make_controller(&shared, &environment, host) {
                        fail(&shared, error);
                    }
                }
                Err(error) => fail(&shared, error),
            }
        });
    }

    fn apply_bounds(&self) {
        apply_bounds(&self.0);
    }
}

impl Drop for WebView {
    fn drop(&mut self) {
        let mut state = self.0.state.borrow_mut();
        if let Some(live) = state.live.take() {
            // SAFETY: closing a live controller, which is what lets its processes go.
            unsafe {
                let _ = live.controller.Close();
            }
        }
        if let Some(host) = state.host.take() {
            // SAFETY: a window this made, on this thread.
            unsafe {
                let _ = DestroyWindow(host);
            }
        }
    }
}

/// What's said about a command that can't be done, as there's no view.
fn failed_event(command: &Command, why: &str) -> Option<Event> {
    Some(match command {
        Command::Script(_, call) => Event::ScriptDone { call: *call, result: Err(why.to_owned()) },
        Command::GetCookies(_, call) => Event::Cookies { call: *call, result: Err(why.to_owned()) },
        Command::SetCookie(_, call) => Event::CookieSet { call: *call, result: Err(why.to_owned()) },
        _ => return None,
    })
}

fn fail(shared: &Rc<Shared>, error: String) {
    crate::log(|| format!("failed: {error}"));
    let queued = {
        let mut state = shared.state.borrow_mut();
        state.starting = false;
        state.failed = Some(error.clone());
        std::mem::take(&mut state.queued)
    };
    shared.tell(Event::Unavailable(error.clone()));
    for command in queued {
        if let Some(event) = failed_event(&command, &error) {
            shared.tell(event);
        }
    }
}

/// Reads a string WebView2 gave, and frees it.
///
/// # Safety
/// `text` must be null or a string WebView2 allocated with `CoTaskMemAlloc`.
unsafe fn take(text: PWSTR) -> String {
    if text.is_null() {
        return String::new();
    }
    // SAFETY: as the caller says.
    unsafe {
        let read = text.to_string().unwrap_or_default();
        CoTaskMemFree(Some(text.0.cast()));
        read
    }
}

/// The window a view sits in.
fn make_host(parent: HWND) -> Result<HWND, String> {
    thread_local! {
        static REGISTERED: Cell<bool> = const { Cell::new(false) };
    }
    unsafe extern "system" fn host_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // SAFETY: the arguments are the ones this was given.
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }
    // SAFETY: registering a class once per thread, and making a hidden child window of a window of this thread's.
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|error| error.message())?;
        if !REGISTERED.with(|done| done.replace(true)) {
            let class = WNDCLASSW {
                lpfnWndProc: Some(host_proc),
                hInstance: instance.into(),
                lpszClassName: w!("rsui-webview-host"),
                hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as usize as *mut c_void),
                ..Default::default()
            };
            RegisterClassW(&class);
        }
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("rsui-webview-host"),
            PCWSTR::null(),
            WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            1,
            1,
            Some(parent),
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|error| format!("the web view's window couldn't be made: {}", error.message()))
    }
}

fn make_controller(shared: &Rc<Shared>, environment: &ICoreWebView2Environment, host: HWND) -> Result<(), String> {
    let weak = Rc::downgrade(shared);
    let made = handler::make(handlers::CONTROLLER_CREATED, {
        let mut weak = Some(weak);
        move |first, second| {
            let Some(shared) = weak.take().and_then(|weak| weak.upgrade()) else {
                return HRESULT(0);
            };
            let code = handler::code(first);
            crate::log(|| format!("the view's made: {code:?}, {}", if second == 0 { "with nothing" } else { "with a controller" }));
            // SAFETY: WebView2 gives a controller, or nothing, with the result.
            match unsafe { handler::borrowed::<ICoreWebView2Controller>(second) } {
                Some(controller) if code.is_ok() => {
                    if let Err(error) = set_up(&shared, controller) {
                        fail(&shared, error);
                    }
                }
                _ => fail(&shared, format!("the web view couldn't be made ({:#010x}): {}", code.0 as u32, code.message())),
            }
            HRESULT(0)
        }
    });
    let settings = &shared.settings;
    // SAFETY: calls on a live environment, with what they're given living for the call; the handler's held by WebView2.
    let result = unsafe {
        match environment.cast::<ICoreWebView2Environment10>() {
            Ok(newer) if settings.profile.is_some() || settings.in_private => {
                let mut options = None;
                newer.CreateCoreWebView2ControllerOptions(&mut options).ok().map_err(|error| error.message())?;
                let options = options.ok_or("WebView2 gave no options")?;
                if let Some(profile) = &settings.profile {
                    let name = HSTRING::from(profile.as_str());
                    let _ = options.put_ProfileName(PCWSTR(name.as_ptr()));
                }
                let _ = options.put_IsInPrivateModeEnabled(BOOL::from(settings.in_private));
                newer.CreateCoreWebView2ControllerWithOptions(host, options.as_raw(), made.raw())
            }
            _ => environment.CreateCoreWebView2Controller(host, made.raw()),
        }
    };
    drop(made);
    crate::log(|| format!("asked for a view in {host:?}, with profiles {}: {result:?}", environment.cast::<ICoreWebView2Environment10>().is_ok()));
    result.ok().map_err(|error| format!("the web view couldn't be made ({:#010x}): {}", error.code().0 as u32, error.message()))
}

/// Says what's to be heard from a view, sets it up as it was asked to be, and does what was asked of it meanwhile.
fn set_up(shared: &Rc<Shared>, controller: ICoreWebView2Controller) -> Result<(), String> {
    let mut core = None;
    // SAFETY: a call on a live controller.
    unsafe { controller.get_CoreWebView2(&mut core).ok().map_err(|error| error.message())? };
    let core = core.ok_or("WebView2 gave no view")?;
    let weak = Rc::downgrade(shared);
    let mut kept = Vec::new();
    // SAFETY: each of these is a call on a live view or controller, given a handler of the interface it asks for.
    unsafe {
        let mut token = 0;
        let made = listen(&weak, handlers::NAVIGATION_STARTING, |_, _, args| {
            let args: ICoreWebView2NavigationStartingEventArgs = handler::borrowed(args)?;
            let (mut uri, mut redirected, mut user) = (PWSTR::null(), BOOL(0), BOOL(0));
            let _ = args.get_Uri(&mut uri);
            let _ = args.get_IsRedirected(&mut redirected);
            let _ = args.get_IsUserInitiated(&mut user);
            Some(Event::Navigating { url: take(uri), redirected: redirected.as_bool(), user: user.as_bool() })
        });
        core.add_NavigationStarting(made.raw(), &mut token).ok().map_err(|error| error.message())?;
        kept.push(made);
        let made = listen(&weak, handlers::CONTENT_LOADING, |shared, sender, _| {
            let core: ICoreWebView2 = handler::borrowed(sender)?;
            let mut uri = PWSTR::null();
            let _ = core.get_Source(&mut uri);
            shared.state.borrow_mut().arrived = true;
            apply_bounds(shared);
            Some(Event::Navigated { url: take(uri) })
        });
        core.add_ContentLoading(made.raw(), &mut token).ok().map_err(|error| error.message())?;
        kept.push(made);
        let made = listen(&weak, handlers::SOURCE_CHANGED, |_, sender, args| {
            let args: ICoreWebView2SourceChangedEventArgs = handler::borrowed(args)?;
            let mut new = BOOL(0);
            let _ = args.get_IsNewDocument(&mut new);
            if new.as_bool() {
                return None;
            }
            let core: ICoreWebView2 = handler::borrowed(sender)?;
            let mut uri = PWSTR::null();
            let _ = core.get_Source(&mut uri);
            Some(Event::NavigatedInPage { url: take(uri) })
        });
        core.add_SourceChanged(made.raw(), &mut token).ok().map_err(|error| error.message())?;
        kept.push(made);
        let made = listen(&weak, handlers::HISTORY_CHANGED, |_, _, _| Some(Event::History));
        core.add_HistoryChanged(made.raw(), &mut token).ok().map_err(|error| error.message())?;
        kept.push(made);
        let made = listen(&weak, handlers::NAVIGATION_COMPLETED, |_, sender, args| {
            let args: ICoreWebView2NavigationCompletedEventArgs = handler::borrowed(args)?;
            let (mut success, mut status) = (BOOL(0), 0);
            let _ = args.get_IsSuccess(&mut success);
            let _ = args.get_WebErrorStatus(&mut status);
            if success.as_bool() {
                return Some(Event::Finished);
            }
            let core: ICoreWebView2 = handler::borrowed(sender)?;
            let mut uri = PWSTR::null();
            let _ = core.get_Source(&mut uri);
            let (code, description) = net_error(status);
            Some(Event::Failed { url: take(uri), code, description })
        });
        core.add_NavigationCompleted(made.raw(), &mut token).ok().map_err(|error| error.message())?;
        kept.push(made);
        let made = listen(&weak, handlers::DOCUMENT_TITLE_CHANGED, |_, sender, _| {
            let core: ICoreWebView2 = handler::borrowed(sender)?;
            let mut title = PWSTR::null();
            let _ = core.get_DocumentTitle(&mut title);
            Some(Event::Title(take(title)))
        });
        core.add_DocumentTitleChanged(made.raw(), &mut token).ok().map_err(|error| error.message())?;
        kept.push(made);
        let made = listen(&weak, handlers::NEW_WINDOW_REQUESTED, |_, _, args| {
            let args: ICoreWebView2NewWindowRequestedEventArgs = handler::borrowed(args)?;
            let (mut uri, mut user) = (PWSTR::null(), BOOL(0));
            let _ = args.get_Uri(&mut uri);
            let _ = args.get_IsUserInitiated(&mut user);
            // It never gets a window of WebView2's own: what the app says goes.
            let _ = args.put_Handled(BOOL(1));
            Some(Event::NewWindow { url: take(uri), user: user.as_bool() })
        });
        core.add_NewWindowRequested(made.raw(), &mut token).ok().map_err(|error| error.message())?;
        kept.push(made);
        let made = listen(&weak, handlers::WEB_MESSAGE_RECEIVED, |_, _, args| {
            let args: ICoreWebView2WebMessageReceivedEventArgs = handler::borrowed(args)?;
            let mut json = PWSTR::null();
            let _ = args.get_WebMessageAsJson(&mut json);
            Some(Event::Message(take(json)))
        });
        core.add_WebMessageReceived(made.raw(), &mut token).ok().map_err(|error| error.message())?;
        kept.push(made);
        let made = listen(&weak, handlers::PROCESS_FAILED, |_, _, args| {
            let args: ICoreWebView2ProcessFailedEventArgs = handler::borrowed(args)?;
            let mut kind = 0;
            let _ = args.get_ProcessFailedKind(&mut kind);
            Some(Event::Crashed { kind })
        });
        core.add_ProcessFailed(made.raw(), &mut token).ok().map_err(|error| error.message())?;
        kept.push(made);
        let made = listen(&weak, handlers::WINDOW_CLOSE_REQUESTED, |_, _, _| Some(Event::CloseRequested));
        core.add_WindowCloseRequested(made.raw(), &mut token).ok().map_err(|error| error.message())?;
        kept.push(made);
        if let Ok(newer) = core.cast::<ICoreWebView2_2>() {
            let made = listen(&weak, handlers::DOM_CONTENT_LOADED, |_, _, _| Some(Event::DomReady));
            newer.add_DOMContentLoaded(made.raw(), &mut token).ok().map_err(|error| error.message())?;
            kept.push(made);
        }
        let made = listen(&weak, handlers::MOVE_FOCUS_REQUESTED, |shared, _, args| {
            let args: ICoreWebView2MoveFocusRequestedEventArgs = handler::borrowed(args)?;
            let mut reason = 0;
            let _ = args.get_Reason(&mut reason);
            let _ = args.put_Handled(BOOL(1));
            if let Some(parent) = shared.state.borrow().parent {
                let _ = SetFocus(Some(parent));
            }
            Some(Event::FocusLeft { forward: reason != 2 })
        });
        controller.add_MoveFocusRequested(made.raw(), &mut token).ok().map_err(|error| error.message())?;
        kept.push(made);
        let made = listen(&weak, handlers::ACCELERATOR_KEY_PRESSED, |shared, _, args| {
            let args: ICoreWebView2AcceleratorKeyPressedEventArgs = handler::borrowed(args)?;
            let (mut kind, mut code) = (0, 0);
            let _ = args.get_KeyEventKind(&mut kind);
            let _ = args.get_VirtualKey(&mut code);
            // Only presses: what's let go of, and repeats of what's held, are the page's.
            if kind != 0 && kind != 2 {
                return None;
            }
            let held = |key: i32| GetKeyState(key) < 0;
            let pressed = Key { code, ctrl: held(0x11), shift: held(0x10), alt: kind == 2 || held(0x12) };
            if !shared.settings.keys.contains(&pressed) {
                return None;
            }
            let _ = args.put_Handled(BOOL(1));
            Some(Event::Key(pressed))
        });
        controller.add_AcceleratorKeyPressed(made.raw(), &mut token).ok().map_err(|error| error.message())?;
        kept.push(made);
        let made = listen(&weak, handlers::FULL_SCREEN_CHANGED, |_, sender, _| {
            let core: ICoreWebView2 = handler::borrowed(sender)?;
            let mut on = BOOL(0);
            let _ = core.get_ContainsFullScreenElement(&mut on);
            Some(Event::FullScreen(on.as_bool()))
        });
        core.add_ContainsFullScreenElementChanged(made.raw(), &mut token).ok().map_err(|error| error.message())?;
        kept.push(made);
        let made = listen(&weak, handlers::FOCUS_CHANGED, |_, _, _| Some(Event::Focus(true)));
        controller.add_GotFocus(made.raw(), &mut token).ok().map_err(|error| error.message())?;
        kept.push(made);
        let made = listen(&weak, handlers::FOCUS_CHANGED, |_, _, _| Some(Event::Focus(false)));
        controller.add_LostFocus(made.raw(), &mut token).ok().map_err(|error| error.message())?;
        kept.push(made);
        let mut settings = None;
        if core.get_Settings(&mut settings).is_ok()
            && let Some(settings) = settings
        {
            let _ = settings.put_AreDevToolsEnabled(BOOL::from(shared.settings.dev_tools));
            let _ = settings.put_IsStatusBarEnabled(BOOL(0));
            if let (Some(agent), Ok(newer)) = (&shared.settings.user_agent, settings.cast::<ICoreWebView2Settings2>()) {
                let agent = HSTRING::from(agent.as_str());
                let _ = newer.put_UserAgent(PCWSTR(agent.as_ptr()));
            }
        }
    }
    // What runs in each page goes in before anything's loaded, and what was asked for meanwhile waits for it.
    let scripts = shared.settings.scripts.clone();
    let left = Rc::new(Cell::new(scripts.len()));
    {
        let mut state = shared.state.borrow_mut();
        state.live = Some(Live {
            controller,
            core: core.clone(),
            _handlers: kept,
        });
        state.starting = false;
    }
    apply_bounds(shared);
    if scripts.is_empty() {
        ready(shared);
        return Ok(());
    }
    for script in scripts {
        let weak = Rc::downgrade(shared);
        let left = left.clone();
        let made = handler::make(handlers::ADD_SCRIPT_COMPLETED, {
            let mut once = true;
            move |_, _| {
                if std::mem::take(&mut once) {
                    left.set(left.get() - 1);
                    if left.get() == 0
                        && let Some(shared) = weak.upgrade()
                    {
                        ready(&shared);
                    }
                }
                HRESULT(0)
            }
        });
        let script = HSTRING::from(script.as_str());
        // SAFETY: a call on a live view, with the script living for the call, and the handler held by WebView2.
        unsafe {
            core.AddScriptToExecuteOnDocumentCreated(PCWSTR(script.as_ptr()), made.raw())
                .ok()
                .map_err(|error| error.message())?;
        }
    }
    Ok(())
}

/// It's all set up: what was asked of it meanwhile is done now.
fn ready(shared: &Rc<Shared>) {
    shared.tell(Event::Ready);
    let queued = std::mem::take(&mut shared.state.borrow_mut().queued);
    for command in queued {
        run_now(shared, command);
    }
}

/// A handler that tells the view's sink what `read` makes of what it's given, as long as the view's there.
fn listen(
    weak: &Weak<Shared>,
    interface: windows::core::GUID,
    read: impl Fn(&Rc<Shared>, usize, usize) -> Option<Event> + 'static,
) -> Made {
    let weak = weak.clone();
    handler::make(interface, move |sender, args| {
        if let Some(shared) = weak.upgrade()
            && let Some(event) = read(&shared, sender, args)
        {
            shared.tell(event);
        }
        HRESULT(0)
    })
}

fn rect(sides: [i32; 4]) -> RECT {
    RECT { left: sides[0], top: sides[1], right: sides[2], bottom: sides[3] }
}

fn apply_bounds(shared: &Rc<Shared>) {
    let state = shared.state.borrow();
    let Some(host) = state.host else {
        return;
    };
    let showing = state.arrived && state.live.is_some();
    // SAFETY: moving and cutting a window this made, and a live controller in it.
    unsafe {
        let Some(bounds) = state.bounds.as_ref().filter(|_| showing) else {
            let _ = ShowWindow(host, SW_HIDE);
            if let Some(live) = &state.live {
                let _ = live.controller.put_IsVisible(BOOL(0));
            }
            return;
        };
        let [left, top, right, bottom] = bounds.shown;
        let _ = SetWindowPos(
            host,
            Some(HWND_TOP),
            left,
            top,
            right - left,
            bottom - top,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        // Its corners are cut as round as its element's, and what's covered is cut out of it, so that what's drawn there shows.
        // What's under something see-through is too, once what it's in draws a still picture of it there.
        let radius = bounds.radius.max(0);
        let cuts: Vec<&[i32; 4]> = bounds.covered.iter().chain(bounds.see_through.iter().filter(|_| bounds.still)).collect();
        let region = (!cuts.is_empty() || radius > 0).then(|| {
            let [x, y, far_x, far_y] = bounds.rect;
            let whole = if radius > 0 {
                // A round rectangle's region leaves out its right and bottom edges, so it's one bigger.
                CreateRoundRectRgn(x - left, y - top, far_x - left + 1, far_y - top + 1, radius * 2, radius * 2)
            } else {
                CreateRectRgn(0, 0, right - left, bottom - top)
            };
            for cover in cuts {
                let cut = CreateRectRgn(cover[0] - left, cover[1] - top, cover[2] - left, cover[3] - top);
                CombineRgn(Some(whole), Some(whole), Some(cut), RGN_DIFF);
                let _ = DeleteObject(HGDIOBJ(cut.0));
            }
            whole
        });
        // The system owns a region once it's given one.
        SetWindowRgn(host, region, true);
        if let Some(live) = &state.live {
            let [x, y, far_x, far_y] = bounds.rect;
            let _ = live.controller.put_Bounds(rect([x - left, y - top, far_x - left, far_y - top]));
            let _ = live.controller.put_IsVisible(BOOL(1));
        }
    }
}

/// Takes a still picture of the page while something see-through is over it, and lets it go once nothing is (see
/// [`Event::Still`]). A picture's only taken once a page has arrived, and once each time something comes over it.
fn keep_still(shared: &Rc<Shared>) {
    let (wanted, still, core) = {
        let state = shared.state.borrow();
        let wanted = state.bounds.as_ref().is_some_and(|bounds| !bounds.see_through.is_empty());
        let core = state.live.as_ref().filter(|_| state.arrived).map(|live| live.core.clone());
        (wanted, state.still, core)
    };
    match (wanted, still, core) {
        (true, Still::None, Some(core)) => {
            shared.state.borrow_mut().still = Still::Taking;
            if let Err(error) = take_still(shared, &core) {
                crate::log(|| format!("couldn't take a still picture: {error}"));
                shared.state.borrow_mut().still = Still::None;
            }
        }
        (false, Still::Taking | Still::Said, _) => {
            shared.state.borrow_mut().still = Still::None;
            shared.tell(Event::Still(None));
        }
        _ => {}
    }
}

/// Asks the page for a picture of itself, as a PNG, into a stream in memory, and says it once it's there, unless it's no
/// longer wanted by then.
fn take_still(shared: &Rc<Shared>, core: &ICoreWebView2) -> Result<(), String> {
    use windows::Win32::System::Com::{IStream, STREAM_SEEK_END, STREAM_SEEK_SET};
    use windows::Win32::UI::Shell::SHCreateMemStream;
    // SAFETY: a stream made empty, to be written by WebView2.
    let stream: IStream = unsafe { SHCreateMemStream(None) }.ok_or("there's no memory for a picture")?;
    let weak = Rc::downgrade(shared);
    let kept = stream.clone();
    let made = handler::make(handlers::CAPTURE_PREVIEW_COMPLETED, {
        let mut once = true;
        move |first, _| {
            if !std::mem::take(&mut once) {
                return HRESULT(0);
            }
            let Some(shared) = weak.upgrade() else {
                return HRESULT(0);
            };
            let read = || -> windows::core::Result<Vec<u8>> {
                handler::code(first).ok()?;
                let mut size = 0;
                // SAFETY: reading back what WebView2 wrote, into room as big as it is.
                unsafe {
                    kept.Seek(0, STREAM_SEEK_END, Some(&mut size))?;
                    kept.Seek(0, STREAM_SEEK_SET, None)?;
                    let mut bytes = vec![0u8; size as usize];
                    let mut got = 0;
                    kept.Read(bytes.as_mut_ptr().cast(), bytes.len() as u32, Some(&mut got)).ok()?;
                    bytes.truncate(got as usize);
                    Ok(bytes)
                }
            };
            let wanted = shared.state.borrow().still == Still::Taking;
            match read() {
                Ok(png) if wanted => {
                    shared.state.borrow_mut().still = Still::Said;
                    shared.tell(Event::Still(Some(Png(png))));
                }
                Ok(_) => {}
                Err(error) => {
                    crate::log(|| format!("couldn't take a still picture: {}", error.message()));
                    shared.state.borrow_mut().still = Still::None;
                }
            }
            HRESULT(0)
        }
    });
    // SAFETY: a call on a live view, with a stream and a handler WebView2 holds until it's done.
    unsafe { core.CapturePreview(0, stream.as_raw(), made.raw()).ok().map_err(|error| error.message()) }
}

fn run_now(shared: &Rc<Shared>, command: Command) {
    let (core, controller) = {
        let state = shared.state.borrow();
        let Some(live) = &state.live else {
            return;
        };
        (live.core.clone(), live.controller.clone())
    };
    // SAFETY: calls on a live view and controller, with what they're given living for the call, and handlers held by
    // WebView2.
    unsafe {
        match command {
            Command::Navigate(url) => {
                let wide = HSTRING::from(url.as_str());
                let result = core.Navigate(PCWSTR(wide.as_ptr()));
                crate::log(|| format!("navigate to {url}: {result:?}"));
                if result.is_err() {
                    let (code, description) = (-300, "ERR_INVALID_URL");
                    shared.tell(Event::Failed { url, code, description });
                }
            }
            Command::Back => {
                let _ = core.GoBack();
            }
            Command::Forward => {
                let _ = core.GoForward();
            }
            Command::Reload => {
                let _ = core.Reload();
            }
            Command::Stop => {
                let _ = core.Stop();
            }
            Command::Script(script, call) => {
                let weak = Rc::downgrade(shared);
                let made = handler::make(handlers::EXECUTE_SCRIPT_COMPLETED, {
                    let mut once = true;
                    move |first, second| {
                        if std::mem::take(&mut once)
                            && let Some(shared) = weak.upgrade()
                        {
                            let code = handler::code(first);
                            let result = if code.is_ok() {
                                Ok(PCWSTR(second as *const u16).to_string().unwrap_or_default())
                            } else {
                                Err(code.message())
                            };
                            shared.tell(Event::ScriptDone { call, result });
                        }
                        HRESULT(0)
                    }
                });
                let wide = HSTRING::from(script.as_str());
                if let Err(error) = core.ExecuteScript(PCWSTR(wide.as_ptr()), made.raw()).ok() {
                    shared.tell(Event::ScriptDone { call, result: Err(error.message()) });
                }
            }
            Command::Post(json) => {
                let wide = HSTRING::from(json.as_str());
                let _ = core.PostWebMessageAsJson(PCWSTR(wide.as_ptr()));
            }
            Command::UserAgent(agent) => {
                let mut settings = None;
                if core.get_Settings(&mut settings).is_ok()
                    && let Some(newer) = settings.and_then(|settings| settings.cast::<ICoreWebView2Settings2>().ok())
                {
                    let wide = HSTRING::from(agent.as_str());
                    let _ = newer.put_UserAgent(PCWSTR(wide.as_ptr()));
                }
            }
            Command::DevTools => {
                let _ = core.OpenDevToolsWindow();
            }
            Command::Focus => {
                let _ = controller.MoveFocus(0);
            }
            Command::Zoom(factor) => {
                let _ = controller.put_ZoomFactor(factor);
            }
            Command::GetCookies(url, call) => match cookie_manager(&core) {
                Some(manager) => {
                    let weak = Rc::downgrade(shared);
                    let made = handler::make(handlers::GET_COOKIES_COMPLETED, {
                        let mut once = true;
                        move |first, second| {
                            if std::mem::take(&mut once)
                                && let Some(shared) = weak.upgrade()
                            {
                                let code = handler::code(first);
                                let list = handler::borrowed::<ICoreWebView2CookieList>(second);
                                let result = match list {
                                    Some(list) if code.is_ok() => Ok(read_cookies(&list)),
                                    _ => Err(code.message()),
                                };
                                shared.tell(Event::Cookies { call, result });
                            }
                            HRESULT(0)
                        }
                    });
                    let wide = HSTRING::from(url.as_str());
                    if let Err(error) = manager.GetCookies(PCWSTR(wide.as_ptr()), made.raw()).ok() {
                        shared.tell(Event::Cookies { call, result: Err(error.message()) });
                    }
                }
                None => shared.tell(Event::Cookies { call, result: Err(NO_COOKIES.to_owned()) }),
            },
            Command::SetCookie(cookie, call) => {
                let result = cookie_manager(&core).ok_or(NO_COOKIES.to_owned()).and_then(|manager| set_cookie(&manager, &cookie));
                shared.tell(Event::CookieSet { call, result });
            }
            Command::RemoveCookies(name, url) => {
                if let Some(manager) = cookie_manager(&core) {
                    let (name, url) = (HSTRING::from(name.as_str()), HSTRING::from(url.as_str()));
                    let _ = manager.DeleteCookies(PCWSTR(name.as_ptr()), PCWSTR(url.as_ptr()));
                }
            }
            Command::ClearCookies => {
                if let Some(manager) = cookie_manager(&core) {
                    let _ = manager.DeleteAllCookies();
                }
            }
        }
    }
}

const NO_COOKIES: &str = "this WebView2 runtime is too old to have cookies";

fn cookie_manager(core: &ICoreWebView2) -> Option<ICoreWebView2CookieManager> {
    let newer = core.cast::<ICoreWebView2_2>().ok()?;
    let mut manager = None;
    // SAFETY: a call on a live view.
    unsafe { newer.get_CookieManager(&mut manager).ok().ok()? };
    manager
}

/// Each cookie in a list WebView2 gave.
///
/// # Safety
/// `list` must be a live cookie list.
unsafe fn read_cookies(list: &ICoreWebView2CookieList) -> Vec<Cookie> {
    let mut count = 0;
    let mut out = Vec::new();
    // SAFETY: calls on a live list and the cookies in it, whose strings `take` frees.
    unsafe {
        let _ = list.get_Count(&mut count);
        for index in 0..count {
            let mut cookie = None;
            if list.GetValueAtIndex(index, &mut cookie).is_err() {
                continue;
            }
            let Some(cookie) = cookie else {
                continue;
            };
            let text = |read: unsafe fn(&ICoreWebView2Cookie, *mut PWSTR) -> HRESULT| {
                let mut value = PWSTR::null();
                let _ = read(&cookie, &mut value);
                take(value)
            };
            let flag = |read: unsafe fn(&ICoreWebView2Cookie, *mut BOOL) -> HRESULT| {
                let mut value = BOOL(0);
                let _ = read(&cookie, &mut value);
                value.as_bool()
            };
            let mut expires = 0.0;
            let _ = cookie.get_Expires(&mut expires);
            let mut same_site = 0;
            let _ = cookie.get_SameSite(&mut same_site);
            let session = flag(ICoreWebView2Cookie::get_IsSession);
            out.push(Cookie {
                name: text(ICoreWebView2Cookie::get_Name),
                value: text(ICoreWebView2Cookie::get_Value),
                domain: text(ICoreWebView2Cookie::get_Domain),
                path: text(ICoreWebView2Cookie::get_Path),
                expires: (!session).then_some(expires),
                http_only: flag(ICoreWebView2Cookie::get_IsHttpOnly),
                secure: flag(ICoreWebView2Cookie::get_IsSecure),
                same_site,
            });
        }
    }
    out
}

fn set_cookie(manager: &ICoreWebView2CookieManager, cookie: &Cookie) -> Result<(), String> {
    let wide = |text: &str| HSTRING::from(text);
    let (name, value, domain, path) = (wide(&cookie.name), wide(&cookie.value), wide(&cookie.domain), wide(&cookie.path));
    let mut made = None;
    // SAFETY: calls on a live cookie manager and the cookie it made, with what they're given living for the call.
    unsafe {
        manager
            .CreateCookie(PCWSTR(name.as_ptr()), PCWSTR(value.as_ptr()), PCWSTR(domain.as_ptr()), PCWSTR(path.as_ptr()), &mut made)
            .ok()
            .map_err(|error| error.message())?;
        let made = made.ok_or("WebView2 made no cookie")?;
        if let Some(expires) = cookie.expires {
            let _ = made.put_Expires(expires);
        }
        let _ = made.put_IsHttpOnly(BOOL::from(cookie.http_only));
        let _ = made.put_IsSecure(BOOL::from(cookie.secure));
        let _ = made.put_SameSite(cookie.same_site);
        manager.AddOrUpdateCookie(made.as_raw()).ok().map_err(|error| error.message())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_read_as_shortcuts_are_written() {
        assert_eq!(Key::parse("Ctrl+Shift+T"), Some(Key { code: 0x54, ctrl: true, shift: true, alt: false }));
        assert_eq!(Key::parse("Alt+ArrowLeft"), Some(Key { code: 0x25, ctrl: false, shift: false, alt: true }));
        assert_eq!(Key::parse("F5"), Some(Key { code: 0x74, ..Key::default() }));
        assert_eq!(Key::parse("Ctrl+,"), Some(Key { code: 0xBC, ctrl: true, ..Key::default() }));
        assert_eq!(Key::parse("Ctrl+Nothing"), None);
        assert_eq!(Key::parse("Ctrl"), None);
    }

    #[test]
    fn errors_are_chromiums_by_number_and_name() {
        assert_eq!(net_error(13), (-105, "ERR_NAME_NOT_RESOLVED"));
        assert_eq!(net_error(14), (-3, "ERR_ABORTED"));
        assert_eq!(net_error(0), (-2, "ERR_FAILED"));
        assert_eq!(net_error(99), (-2, "ERR_FAILED"));
    }

    #[test]
    fn what_is_asked_before_it_is_made_waits_and_nothing_is_started_until_it_is_put_somewhere() {
        let told = Rc::new(RefCell::new(Vec::new()));
        let view = WebView::new(Settings::default(), {
            let told = told.clone();
            move |event| told.borrow_mut().push(event)
        });
        view.navigate("https://example.com/");
        view.execute("1 + 1", 4);
        assert_eq!(view.0.state.borrow().queued.len(), 2);
        assert!(view.url().is_none());
        assert!(told.borrow().is_empty());
        // Hiding it before it's anywhere doesn't start anything either.
        view.place(0, None);
        assert!(!view.0.state.borrow().starting);
    }

    #[test]
    fn what_can_not_be_done_without_a_view_says_so_once_it_is_known_there_is_none() {
        let told = Rc::new(RefCell::new(Vec::new()));
        let view = WebView::new(Settings::default(), {
            let told = told.clone();
            move |event| told.borrow_mut().push(event)
        });
        view.execute("1", 1);
        view.navigate("https://example.com/");
        fail(&view.0, "no runtime".to_owned());
        view.cookies("", 2);
        let told = told.borrow();
        assert_eq!(told[0], Event::Unavailable("no runtime".to_owned()));
        assert_eq!(told[1], Event::ScriptDone { call: 1, result: Err("no runtime".to_owned()) });
        assert_eq!(told[2], Event::Cookies { call: 2, result: Err("no runtime".to_owned()) });
        assert_eq!(told.len(), 3);
    }
}
