//! What comes in from a keyboard and a pointer, apart from any window.

use std::fmt::Write;

pub use rsui_runtime::Modifiers;

/// A key on the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    Tab,
    Enter,
    Escape,
    Backspace,
    Delete,
    Space,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    /// A letter, digit or other key that types something, in lowercase, as it's set out on this keyboard.
    Character(char),
    /// `F1` to `F24`.
    Function(u8),
    Insert,
    Shift,
    Control,
    Alt,
    /// The Windows key.
    Meta,
    CapsLock,
    NumLock,
    ScrollLock,
    Pause,
    PrintScreen,
    /// The key that brings up a menu, as clicking with the other button does.
    ContextMenu,
    /// A key that a way of typing, as for Chinese, Japanese or Korean, is using to put something together.
    Process,
}

impl KeyCode {
    fn name(self, out: &mut String) {
        out.push_str(match self {
            Self::Tab => "tab",
            Self::Enter => "enter",
            Self::Escape => "escape",
            Self::Backspace => "backspace",
            Self::Delete => "delete",
            Self::Space => "space",
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
            Self::Home => "home",
            Self::End => "end",
            Self::PageUp => "page_up",
            Self::PageDown => "page_down",
            Self::Insert => "insert",
            Self::Shift => "shift",
            Self::Control => "control",
            Self::Alt => "alt",
            Self::Meta => "meta",
            Self::CapsLock => "caps_lock",
            Self::NumLock => "num_lock",
            Self::ScrollLock => "scroll_lock",
            Self::Pause => "pause",
            Self::PrintScreen => "print_screen",
            Self::ContextMenu => "context_menu",
            Self::Process => "process",
            Self::Function(number) => {
                let _ = write!(out, "f{number}");
                return;
            }
            Self::Character(character) => {
                out.push(character);
                return;
            }
        });
    }

    /// Whether this is only something held along with other keys.
    pub fn is_modifier(self) -> bool {
        matches!(self, Self::Shift | Self::Control | Self::Alt | Self::Meta)
    }
}

/// A key being pressed, along with what was held down at the time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPress {
    pub code: KeyCode,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// Whether this came from the key being held down, rather than pressed.
    pub repeat: bool,
    /// Whether the Windows key was held.
    pub meta: bool,
    /// Where the key is on the keyboard, as the system's scan code, with `0xE000` added for the keys it calls extended. 0
    /// when that isn't known, as when an agent presses a key, which is then taken to be where it is on a US keyboard.
    pub scan: u16,
    /// What the key types on this keyboard, with what's held, if it's known and types anything. `None` goes by `code`.
    pub typed: Option<char>,
}

impl KeyPress {
    pub const fn new(code: KeyCode) -> Self {
        Self {
            code,
            ctrl: false,
            shift: false,
            alt: false,
            repeat: false,
            meta: false,
            scan: 0,
            typed: None,
        }
    }

    pub const fn ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    pub const fn shift(mut self) -> Self {
        self.shift = true;
        self
    }

    pub const fn alt(mut self) -> Self {
        self.alt = true;
        self
    }

    pub const fn meta(mut self) -> Self {
        self.meta = true;
        self
    }

    /// What's held along with it.
    pub const fn modifiers(self) -> Modifiers {
        Modifiers {
            ctrl: self.ctrl,
            shift: self.shift,
            alt: self.alt,
            meta: self.meta,
        }
    }

    /// As given to [`ActionKind::KeyPress`](rsui_core::ActionKind::KeyPress) handlers, such as
    /// `"enter"`, or `"ctrl+shift+s"`, with what was held always in that order.
    pub fn describe(self) -> String {
        let mut out = String::new();
        for (held, name) in [
            (self.ctrl, "ctrl"),
            (self.alt, "alt"),
            (self.shift, "shift"),
            (self.meta, "win"),
        ] {
            if held {
                let _ = write!(out, "{name}+");
            }
        }
        self.code.name(&mut out);
        out
    }

    /// What the web's `KeyboardEvent.key` says for this: what it means as the keyboard's set out, as `"a"`, `"A"` with
    /// shift, `"Enter"`, `"ArrowLeft"`, `"F5"` or `"Shift"`.
    pub fn web_key(self) -> String {
        let named = match self.code {
            KeyCode::Tab => "Tab",
            KeyCode::Enter => "Enter",
            KeyCode::Escape => "Escape",
            KeyCode::Backspace => "Backspace",
            KeyCode::Delete => "Delete",
            KeyCode::Space => " ",
            KeyCode::Left => "ArrowLeft",
            KeyCode::Right => "ArrowRight",
            KeyCode::Up => "ArrowUp",
            KeyCode::Down => "ArrowDown",
            KeyCode::Home => "Home",
            KeyCode::End => "End",
            KeyCode::PageUp => "PageUp",
            KeyCode::PageDown => "PageDown",
            KeyCode::Insert => "Insert",
            KeyCode::Shift => "Shift",
            KeyCode::Control => "Control",
            KeyCode::Alt => "Alt",
            KeyCode::Meta => "Meta",
            KeyCode::CapsLock => "CapsLock",
            KeyCode::NumLock => "NumLock",
            KeyCode::ScrollLock => "ScrollLock",
            KeyCode::Pause => "Pause",
            KeyCode::PrintScreen => "PrintScreen",
            KeyCode::ContextMenu => "ContextMenu",
            KeyCode::Process => "Process",
            KeyCode::Function(number) => return format!("F{number}"),
            KeyCode::Character(character) => {
                if let Some(typed) = self.typed.filter(|typed| !typed.is_control()) {
                    return typed.to_string();
                }
                return if self.shift {
                    us_shifted(character).to_string()
                } else {
                    character.to_string()
                };
            }
        };
        named.to_owned()
    }

    /// What the web's `KeyboardEvent.code` says for this: where the key is, whatever it means there, as `"KeyA"`,
    /// `"Digit1"` or `"ShiftLeft"`. A shortcut that goes by this stays in the same place on any keyboard.
    pub fn web_code(self) -> &'static str {
        if self.scan != 0 {
            if let Some(code) = code_of_scan(self.scan) {
                return code;
            }
        }
        us_code(self.code)
    }

    /// The web's `KeyboardEvent.location`: 1 for the one on the left of two, 2 for the right, 3 for the number pad, and 0
    /// for the rest.
    pub fn web_location(self) -> u8 {
        let code = self.web_code();
        if code.ends_with("Left") && !code.starts_with("Arrow") {
            1
        } else if code.ends_with("Right") && !code.starts_with("Arrow") {
            2
        } else if code.starts_with("Numpad") {
            3
        } else {
            0
        }
    }
}

/// What a key types with shift held, on a US keyboard.
fn us_shifted(character: char) -> char {
    match character {
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        '\\' => '|',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        '`' => '~',
        other => other.to_uppercase().next().unwrap_or(other),
    }
}

/// Where a key is on a US keyboard, as the web says it, for a key whose place isn't known.
fn us_code(code: KeyCode) -> &'static str {
    const LETTERS: [&str; 26] = [
        "KeyA", "KeyB", "KeyC", "KeyD", "KeyE", "KeyF", "KeyG", "KeyH", "KeyI", "KeyJ", "KeyK",
        "KeyL", "KeyM", "KeyN", "KeyO", "KeyP", "KeyQ", "KeyR", "KeyS", "KeyT", "KeyU", "KeyV",
        "KeyW", "KeyX", "KeyY", "KeyZ",
    ];
    const DIGITS: [&str; 10] = [
        "Digit0", "Digit1", "Digit2", "Digit3", "Digit4", "Digit5", "Digit6", "Digit7", "Digit8",
        "Digit9",
    ];
    const FUNCTIONS: [&str; 24] = [
        "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14",
        "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
    ];
    match code {
        KeyCode::Tab => "Tab",
        KeyCode::Enter => "Enter",
        KeyCode::Escape => "Escape",
        KeyCode::Backspace => "Backspace",
        KeyCode::Delete => "Delete",
        KeyCode::Space => "Space",
        KeyCode::Left => "ArrowLeft",
        KeyCode::Right => "ArrowRight",
        KeyCode::Up => "ArrowUp",
        KeyCode::Down => "ArrowDown",
        KeyCode::Home => "Home",
        KeyCode::End => "End",
        KeyCode::PageUp => "PageUp",
        KeyCode::PageDown => "PageDown",
        KeyCode::Insert => "Insert",
        KeyCode::Shift => "ShiftLeft",
        KeyCode::Control => "ControlLeft",
        KeyCode::Alt => "AltLeft",
        KeyCode::Meta => "MetaLeft",
        KeyCode::CapsLock => "CapsLock",
        KeyCode::NumLock => "NumLock",
        KeyCode::ScrollLock => "ScrollLock",
        KeyCode::Pause => "Pause",
        KeyCode::PrintScreen => "PrintScreen",
        KeyCode::ContextMenu => "ContextMenu",
        KeyCode::Process => "",
        KeyCode::Function(number) => FUNCTIONS
            .get(usize::from(number).wrapping_sub(1))
            .copied()
            .unwrap_or(""),
        KeyCode::Character(character) => match character.to_ascii_lowercase() {
            letter @ 'a'..='z' => LETTERS[(letter as u8 - b'a') as usize],
            digit @ '0'..='9' => DIGITS[(digit as u8 - b'0') as usize],
            '-' => "Minus",
            '=' => "Equal",
            '[' => "BracketLeft",
            ']' => "BracketRight",
            '\\' => "Backslash",
            ';' => "Semicolon",
            '\'' => "Quote",
            ',' => "Comma",
            '.' => "Period",
            '/' => "Slash",
            '`' => "Backquote",
            _ => "",
        },
    }
}

/// Where a key is, as the web says it, from the system's scan code for it.
fn code_of_scan(scan: u16) -> Option<&'static str> {
    Some(match scan {
        0x01 => "Escape",
        0x02 => "Digit1",
        0x03 => "Digit2",
        0x04 => "Digit3",
        0x05 => "Digit4",
        0x06 => "Digit5",
        0x07 => "Digit6",
        0x08 => "Digit7",
        0x09 => "Digit8",
        0x0A => "Digit9",
        0x0B => "Digit0",
        0x0C => "Minus",
        0x0D => "Equal",
        0x0E => "Backspace",
        0x0F => "Tab",
        0x10 => "KeyQ",
        0x11 => "KeyW",
        0x12 => "KeyE",
        0x13 => "KeyR",
        0x14 => "KeyT",
        0x15 => "KeyY",
        0x16 => "KeyU",
        0x17 => "KeyI",
        0x18 => "KeyO",
        0x19 => "KeyP",
        0x1A => "BracketLeft",
        0x1B => "BracketRight",
        0x1C => "Enter",
        0x1D => "ControlLeft",
        0x1E => "KeyA",
        0x1F => "KeyS",
        0x20 => "KeyD",
        0x21 => "KeyF",
        0x22 => "KeyG",
        0x23 => "KeyH",
        0x24 => "KeyJ",
        0x25 => "KeyK",
        0x26 => "KeyL",
        0x27 => "Semicolon",
        0x28 => "Quote",
        0x29 => "Backquote",
        0x2A => "ShiftLeft",
        0x2B => "Backslash",
        0x2C => "KeyZ",
        0x2D => "KeyX",
        0x2E => "KeyC",
        0x2F => "KeyV",
        0x30 => "KeyB",
        0x31 => "KeyN",
        0x32 => "KeyM",
        0x33 => "Comma",
        0x34 => "Period",
        0x35 => "Slash",
        0x36 => "ShiftRight",
        0x37 => "NumpadMultiply",
        0x38 => "AltLeft",
        0x39 => "Space",
        0x3A => "CapsLock",
        0x3B => "F1",
        0x3C => "F2",
        0x3D => "F3",
        0x3E => "F4",
        0x3F => "F5",
        0x40 => "F6",
        0x41 => "F7",
        0x42 => "F8",
        0x43 => "F9",
        0x44 => "F10",
        0x45 => "Pause",
        0x46 => "ScrollLock",
        0x47 => "Numpad7",
        0x48 => "Numpad8",
        0x49 => "Numpad9",
        0x4A => "NumpadSubtract",
        0x4B => "Numpad4",
        0x4C => "Numpad5",
        0x4D => "Numpad6",
        0x4E => "NumpadAdd",
        0x4F => "Numpad1",
        0x50 => "Numpad2",
        0x51 => "Numpad3",
        0x52 => "Numpad0",
        0x53 => "NumpadDecimal",
        0x56 => "IntlBackslash",
        0x57 => "F11",
        0x58 => "F12",
        0x64 => "F13",
        0x65 => "F14",
        0x66 => "F15",
        0x67 => "F16",
        0x68 => "F17",
        0x69 => "F18",
        0x6A => "F19",
        0x6B => "F20",
        0x6C => "F21",
        0x6D => "F22",
        0x6E => "F23",
        0x76 => "F24",
        0xE01C => "NumpadEnter",
        0xE01D => "ControlRight",
        0xE035 => "NumpadDivide",
        0xE037 => "PrintScreen",
        0xE038 => "AltRight",
        0xE045 => "NumLock",
        0xE047 => "Home",
        0xE048 => "ArrowUp",
        0xE049 => "PageUp",
        0xE04B => "ArrowLeft",
        0xE04D => "ArrowRight",
        0xE04F => "End",
        0xE050 => "ArrowDown",
        0xE051 => "PageDown",
        0xE052 => "Insert",
        0xE053 => "Delete",
        0xE05B => "MetaLeft",
        0xE05C => "MetaRight",
        0xE05D => "ContextMenu",
        _ => return None,
    })
}

/// A button on the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PointerButton {
    /// The left one, for most people.
    #[default]
    Main,
    Middle,
    /// The right one, for most people, which brings up a menu.
    Secondary,
    Back,
    Forward,
}

impl PointerButton {
    /// What the web's `MouseEvent.button` says for it.
    pub const fn web_button(self) -> u8 {
        match self {
            Self::Main => 0,
            Self::Middle => 1,
            Self::Secondary => 2,
            Self::Back => 3,
            Self::Forward => 4,
        }
    }

    /// Its bit in the web's `MouseEvent.buttons`.
    pub const fn web_bit(self) -> u8 {
        match self {
            Self::Main => 1,
            Self::Secondary => 2,
            Self::Middle => 4,
            Self::Back => 8,
            Self::Forward => 16,
        }
    }
}

/// What the pointer should look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cursor {
    #[default]
    Arrow,
    /// Over something that can be clicked.
    Hand,
    /// Over text that can be edited.
    Text,
}

/// Somewhere to copy text to and paste it from.
pub trait Clipboard {
    fn read(&mut self) -> Option<String>;
    fn write(&mut self, text: &str);
}

/// A clipboard that only this app can see, for tests and for running without a window.
#[derive(Debug, Clone, Default)]
pub struct MemoryClipboard(Option<String>);

impl Clipboard for MemoryClipboard {
    fn read(&mut self) -> Option<String> {
        self.0.clone()
    }

    fn write(&mut self, text: &str) {
        self.0 = Some(text.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_say_what_the_web_calls_them() {
        let a = KeyPress::new(KeyCode::Character('a'));
        assert_eq!((a.web_key().as_str(), a.web_code()), ("a", "KeyA"));
        assert_eq!(a.shift().web_key(), "A");
        let one = KeyPress::new(KeyCode::Character('1')).shift();
        assert_eq!((one.web_key().as_str(), one.web_code()), ("!", "Digit1"));
        assert_eq!(KeyPress::new(KeyCode::Function(5)).web_key(), "F5");
        assert_eq!(KeyPress::new(KeyCode::Function(12)).web_code(), "F12");
        assert_eq!(KeyPress::new(KeyCode::Left).web_key(), "ArrowLeft");
        assert_eq!(KeyPress::new(KeyCode::Space).web_key(), " ");
        let shift = KeyPress::new(KeyCode::Shift);
        assert_eq!((shift.web_key().as_str(), shift.web_location()), ("Shift", 1));
        // A French keyboard's A is where a US keyboard's Q is, which is what the code says.
        let french = KeyPress {
            scan: 0x10,
            typed: Some('a'),
            ..KeyPress::new(KeyCode::Character('a'))
        };
        assert_eq!((french.web_key().as_str(), french.web_code()), ("a", "KeyQ"));
        let right_control = KeyPress {
            scan: 0xE01D,
            ..KeyPress::new(KeyCode::Control)
        };
        assert_eq!(right_control.web_location(), 2);
        assert_eq!(KeyPress::new(KeyCode::Function(2)).describe(), "f2");
        assert_eq!(a.ctrl().meta().describe(), "ctrl+win+a");
    }
}
