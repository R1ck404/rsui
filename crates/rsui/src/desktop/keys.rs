//! Keys as they're written, like `ctrl+shift+k`.

/// A key and what's held with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Keys {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub windows: bool,
    /// The system's number for the key.
    pub key: u16,
}

/// Reads keys as this writes them, or as Electron does, like `ctrl+shift+k` or `CommandOrControl+Shift+K`. On Windows, `Command`
/// means Control, as that's what a shortcut written for a Mac would mean here.
pub fn parse_keys(text: &str) -> Result<Keys, String> {
    let bad = |why: &str| Err(format!("{text:?} {why}"));
    let mut keys = Keys {
        control: false,
        alt: false,
        shift: false,
        windows: false,
        key: 0,
    };
    // The plus key comes after a `+` like any other, and on its own it's just itself.
    let (held, last) = if text == "+" {
        ("", Some("plus"))
    } else if let Some(before) = text.strip_suffix("++") {
        (before, Some("plus"))
    } else {
        match text.rsplit_once('+') {
            Some((held, last)) => (held, Some(last)),
            None => ("", Some(text)),
        }
    };
    for part in held.split('+').filter(|part| !part.is_empty()) {
        match part.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmd" | "command" | "cmdorctrl" | "commandorcontrol" => {
                keys.control = true;
            }
            "alt" | "option" | "altgr" => keys.alt = true,
            "shift" => keys.shift = true,
            "win" | "windows" | "super" | "meta" => keys.windows = true,
            _ => {
                return bad(&format!(
                    "has {part:?} held down, which isn't a key to hold"
                ));
            }
        }
    }
    let Some(last) = last.map(str::trim).filter(|last| !last.is_empty()) else {
        return bad("doesn't say which key");
    };
    let Some(key) = key_number(last) else {
        return bad(&format!("ends in {last:?}, which isn't a key this knows"));
    };
    keys.key = key;
    Ok(keys)
}

fn key_number(name: &str) -> Option<u16> {
    let lower = name.to_ascii_lowercase();
    if let [one] = lower.as_bytes() {
        return match one {
            b'a'..=b'z' => Some(u16::from(one.to_ascii_uppercase())),
            b'0'..=b'9' => Some(u16::from(*one)),
            b';' => Some(0xBA),
            b'=' => Some(0xBB),
            b',' => Some(0xBC),
            b'-' => Some(0xBD),
            b'.' => Some(0xBE),
            b'/' => Some(0xBF),
            b'`' => Some(0xC0),
            b'[' => Some(0xDB),
            b'\\' => Some(0xDC),
            b']' => Some(0xDD),
            b'\'' => Some(0xDE),
            _ => None,
        };
    }
    if let Some(number) = lower.strip_prefix('f').and_then(|n| n.parse::<u16>().ok()) {
        return (1..=24).contains(&number).then_some(0x6F + number);
    }
    Some(match lower.as_str() {
        "space" => 0x20,
        "enter" | "return" => 0x0D,
        "tab" => 0x09,
        "esc" | "escape" => 0x1B,
        "backspace" => 0x08,
        "del" | "delete" => 0x2E,
        "ins" | "insert" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "up" => 0x26,
        "down" => 0x28,
        "left" => 0x25,
        "right" => 0x27,
        "plus" => 0xBB,
        "minus" => 0xBD,
        "printscreen" => 0x2C,
        "volumeup" => 0xAF,
        "volumedown" => 0xAE,
        "volumemute" => 0xAD,
        "medianexttrack" => 0xB0,
        "mediaprevioustrack" => 0xB1,
        "mediastop" => 0xB2,
        "mediaplaypause" => 0xB3,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(control: bool, alt: bool, shift: bool, windows: bool, key: u16) -> Keys {
        Keys {
            control,
            alt,
            shift,
            windows,
            key,
        }
    }

    #[test]
    fn keys_read_as_they_are_written_here_and_in_electron() {
        let wanted = keys(true, false, true, false, u16::from(b'K'));
        assert_eq!(parse_keys("ctrl+shift+k"), Ok(wanted));
        assert_eq!(parse_keys("CommandOrControl+Shift+K"), Ok(wanted));
        assert_eq!(parse_keys("CmdOrCtrl+Shift+K"), Ok(wanted));
        assert_eq!(parse_keys(" Control + Shift + k "), Ok(wanted));
    }

    #[test]
    fn other_keys_have_their_own_numbers() {
        assert_eq!(parse_keys("f5"), Ok(keys(false, false, false, false, 0x74)));
        assert_eq!(
            parse_keys("Alt+F24"),
            Ok(keys(false, true, false, false, 0x87))
        );
        assert_eq!(
            parse_keys("super+1"),
            Ok(keys(false, false, false, true, u16::from(b'1')))
        );
        assert_eq!(
            parse_keys("MediaPlayPause"),
            Ok(keys(false, false, false, false, 0xB3))
        );
        assert_eq!(
            parse_keys("ctrl+,"),
            Ok(keys(true, false, false, false, 0xBC))
        );
    }

    #[test]
    fn the_plus_key_can_be_asked_for() {
        assert_eq!(
            parse_keys("ctrl++"),
            Ok(keys(true, false, false, false, 0xBB))
        );
        assert_eq!(
            parse_keys("ctrl+plus"),
            Ok(keys(true, false, false, false, 0xBB))
        );
        assert_eq!(parse_keys("+"), Ok(keys(false, false, false, false, 0xBB)));
    }

    #[test]
    fn what_is_not_a_key_says_so() {
        assert!(parse_keys("").unwrap_err().contains("which key"));
        assert!(parse_keys("ctrl+").unwrap_err().contains("which key"));
        assert!(parse_keys("ctrl+nothing").unwrap_err().contains("nothing"));
        assert!(parse_keys("hyper+k").unwrap_err().contains("hyper"));
        assert!(parse_keys("f25").unwrap_err().contains("f25"));
        assert!(parse_keys("f0").is_err());
    }
}
