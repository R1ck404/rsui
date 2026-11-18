//! Web pages from elsewhere inside an rsui window, as `rsui/webview`'s `<WebView>` shows them. rsui draws an app's own UI
//! itself, with no browser. Someone else's site is written for a browser, with its logins, captchas and checks, so it gets
//! one: WebView2, Edge's runtime, which comes with Windows 11.
//!
//! Nothing of it is loaded until the first view is put somewhere: the runtime is found in the registry then, and started
//! with its own library, through WebView2's COM interfaces, written out here for what this uses (`ffi.rs`). Without a
//! runtime, a view says so with [`Event::Unavailable`], and [`runtime::missing`] says what to do about it.
//!
//! A view sits where rsui puts it, which `rsui::embed` says, as a window inside the app's window: see [`WebView::place`].
//! What a page keeps, its cookies and storage, is in a folder of the app's, in a profile of its own for each name a view is
//! given ([`profile_name`]), and never in the person's own Edge.

mod ffi;
mod handler;
pub mod runtime;
mod view;

use std::path::{Path, PathBuf};

/// Says what's being done, and what came of it, if `RSUI_WEBVIEW_LOG` is set, for finding out why a view doesn't show.
pub(crate) fn log(said: impl FnOnce() -> String) {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("RSUI_WEBVIEW_LOG").is_some_and(|on| !on.is_empty() && on != "0")) {
        eprintln!("rsui-webview: {}", said());
    }
}

pub use view::{Bounds, Cookie, Event, Key, Png, Settings, WebView, net_error};

/// The WebView2 profile a view's profile `name` is kept in. A profile's name can have letters, digits and a few marks, and
/// be 64 long at most; anything else becomes `_`, and a name that had to change keeps a short hash of what it was, so that
/// two that differ still differ.
pub fn profile_name(name: &str) -> String {
    let mut profile: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "#@$()+-_~. ".contains(c) { c } else { '_' })
        .collect();
    if profile != name || profile.len() > 64 {
        let hash = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3));
        profile.truncate(48);
        profile.push_str(&format!("-{:08x}", hash as u32));
    }
    profile
}

/// The folder that the browser keeps what pages keep in, for an app whose own data is in `user_data`.
pub fn data_folder(user_data: &Path) -> PathBuf {
    user_data.join("WebView2")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_names_are_what_webview2_allows_and_two_that_differ_still_differ() {
        assert_eq!(profile_name("work"), "work");
        let one = profile_name("a/b");
        let other = profile_name("a\\b");
        assert!(one.starts_with("a_b-"), "{one}");
        assert_ne!(one, other);
        assert!(profile_name(&"x".repeat(200)).len() <= 64);
    }

    #[test]
    fn what_pages_keep_is_in_the_apps_own_folder() {
        assert_eq!(data_folder(Path::new(r"C:\Data\app")), PathBuf::from(r"C:\Data\app\WebView2"));
    }
}
