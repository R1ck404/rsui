//! A web view, on systems that don't have one of their own here yet (WebView2 is Windows'; WebKitGTK, WKWebView and
//! Android's WebView are what the others will use). Each one says it's unavailable once it's put somewhere, as one on
//! Windows does without a runtime, so what shows it can say so in its place.

use std::cell::{Cell, RefCell};

pub use crate::types::{Bounds, Cookie, Event, Key, Png, Settings, net_error};

/// A web view. Dropping it closes it.
pub struct WebView {
    sink: RefCell<Box<dyn FnMut(Event)>>,
    /// Whether it's said it's unavailable, which it does once.
    said: Cell<bool>,
}

impl WebView {
    /// One that tells `sink` what happens in it. Nothing's made until it's put somewhere.
    pub fn new(_settings: Settings, sink: impl FnMut(Event) + 'static) -> Self {
        Self { sink: RefCell::new(Box::new(sink)), said: Cell::new(false) }
    }

    /// Puts it somewhere, which here only says there's nothing to put.
    pub fn place(&self, _parent: isize, bounds: Option<&Bounds>) {
        if bounds.is_some() && !self.said.replace(true) {
            (self.sink.borrow_mut())(Event::Unavailable(crate::runtime::missing()));
        }
    }

    pub fn window_moved(&self) {}
    pub fn navigate(&self, _url: &str) {}
    pub fn back(&self) {}
    pub fn forward(&self) {}
    pub fn reload(&self) {}
    pub fn stop(&self) {}

    pub fn execute(&self, _script: &str, call: u64) {
        (self.sink.borrow_mut())(Event::ScriptDone { call, result: Err(crate::runtime::missing()) });
    }

    pub fn post(&self, _json: &str) {}
    pub fn set_user_agent(&self, _agent: &str) {}
    pub fn open_dev_tools(&self) {}
    pub fn focus(&self) {}
    pub fn blur(&self) {}
    pub fn set_zoom(&self, _factor: f64) {}

    pub fn cookies(&self, _url: &str, call: u64) {
        (self.sink.borrow_mut())(Event::Cookies { call, result: Err(crate::runtime::missing()) });
    }

    pub fn set_cookie(&self, _cookie: Cookie, call: u64) {
        (self.sink.borrow_mut())(Event::CookieSet { call, result: Err(crate::runtime::missing()) });
    }

    pub fn remove_cookies(&self, _name: &str, _url: &str) {}
    pub fn clear_cookies(&self) {}

    pub fn url(&self) -> Option<String> {
        None
    }

    pub fn title(&self) -> Option<String> {
        None
    }

    pub fn history(&self) -> (bool, bool) {
        (false, false)
    }

    pub fn browser_process(&self) -> Option<u32> {
        None
    }
}
