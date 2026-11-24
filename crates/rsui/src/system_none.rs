//! What programs ask of the system now and then, on systems other than Windows (`system.rs` there).

/// Whether the system's set to dark colors for apps. GNOME and others say so in the theme they give GTK, which is what's
/// to go by until the desktop's settings portal is read.
pub fn system_is_dark() -> bool {
    std::env::var("GTK_THEME").is_ok_and(|theme| theme.to_ascii_lowercase().contains("dark"))
}

/// The shades of the accent color the user's chosen, which only Windows says yet (see `system.rs`).
#[cfg_attr(target_os = "android", allow(dead_code))]
pub(crate) fn accent_palette() -> Option<[[u8; 4]; 8]> {
    None
}

/// Whether this is something it's safe to hand to whatever opens it. Text in a program can come from anywhere, and what
/// opens `file:` or a program's own scheme can run anything.
pub fn can_open_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    !rest.is_empty()
        && (scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("mailto"))
}

/// Opens a web page in the browser, or an address in the mail program, without waiting for that. Anything else is
/// ignored. See [`can_open_url`].
pub fn open_url(url: &str) {
    if !can_open_url(url) {
        return;
    }
    let opener = if cfg!(target_vendor = "apple") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(opener)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_web_and_mail_are_opened() {
        assert!(can_open_url("https://example.com"));
        assert!(can_open_url("mailto:someone@example.com"));
        assert!(!can_open_url("file:///etc/passwd"));
        assert!(!can_open_url("https:"));
    }
}
