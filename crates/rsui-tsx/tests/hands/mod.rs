//! Using an app the way a person does: real presses where things are on screen, the wheel, and keys. Nothing here looks
//! inside it, other than to find out where things are.

#![allow(dead_code)]

use std::path::Path;
use std::time::{Duration, Instant};

pub use rsui::{App, KeyCode, KeyPress};

/// How much stack the script may use before it's told its code is nested too deeply. A debug build's frames are several
/// times a release build's, and a test thread has 2 MB, so a debug build gets most of that, and a page that the release
/// build reads with room to spare is still read.
pub const STACK_LIMIT: usize = if cfg!(debug_assertions) { 1536 * 1024 } else { 512 * 1024 };

/// Where something with this key is, in the tree.
pub fn path_of(node: &rsui::core::Node, key: &str, path: &mut Vec<u32>) -> bool {
    if node
        .key
        .as_ref()
        .is_some_and(|found| found.to_string() == key)
    {
        return true;
    }
    for (index, child) in node.children.iter().enumerate() {
        path.push(index as u32);
        if path_of(child, key, path) {
            return true;
        }
        path.pop();
    }
    false
}

pub struct Hands {
    pub app: App,
}

impl Hands {
    pub fn open(file: &Path, size: (u32, u32)) -> Self {
        let entry = rsui_tsx::load(file, STACK_LIMIT).unwrap();
        let text = rsui::default_text_engine().unwrap();
        let mut app = App::new(rsui_tsx::root, entry, text, size, 1.0);
        app.update();
        app.scene();
        Self { app }
    }

    pub fn settle(&mut self) {
        self.app.update();
        self.app.scene();
    }

    pub fn has(&self, key: &str) -> bool {
        path_of(self.app.runtime().root(), key, &mut Vec::new())
    }

    pub fn says(&self, text: &str) -> bool {
        self.app.outline().contains(text)
    }

    /// Gives what's happening on other threads time to come back, until this says it's there.
    pub fn wait_for(&mut self, text: &str) {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(10) {
            self.settle();
            if self.says(text) {
                return;
            }
            self.app.runtime().wait(Duration::from_millis(5));
        }
        panic!("{text} never showed\n{}", self.app.outline());
    }

    pub fn middle_of(&mut self, key: &str) -> (f32, f32) {
        self.settle();
        let mut path = Vec::new();
        assert!(
            path_of(self.app.runtime().root(), key, &mut path),
            "no {key} in\n{}",
            self.app.outline()
        );
        let rect = self.app.layout().rect(&path).expect("it's been laid out");
        (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
    }

    pub fn click(&mut self, key: &str) {
        let (x, y) = self.middle_of(key);
        self.click_at(x, y);
    }

    /// As a window does it: the pointer gets there, the button goes down, and it comes up.
    pub fn click_at(&mut self, x: f32, y: f32) {
        self.app.pointer_move(x, y);
        self.settle();
        self.app.pointer_down(x, y, 1);
        self.settle();
        let _ = self.app.pointer_up(x, y);
        self.settle();
    }

    pub fn type_text(&mut self, text: &str) {
        for character in text.chars() {
            self.app.type_text(&character.to_string()).unwrap();
            self.settle();
        }
    }

    pub fn press(&mut self, code: KeyCode) {
        self.app.key(KeyPress::new(code)).unwrap();
        self.settle();
    }

    pub fn press_with_ctrl(&mut self, letter: char) {
        self.app
            .key(KeyPress::new(KeyCode::Character(letter)).ctrl())
            .unwrap();
        self.settle();
    }

    /// A key going down, what it types if it types anything and nothing's held but shift, and it coming up, as a window has it.
    pub fn press_key(&mut self, key: KeyPress) {
        self.app.key(key).unwrap();
        self.settle();
        if let KeyCode::Character(character) = key.code {
            if !key.ctrl && !key.alt {
                let typed = if key.shift {
                    character.to_ascii_uppercase()
                } else {
                    character
                };
                self.app.type_text(&typed.to_string()).unwrap();
                self.settle();
            }
        }
        self.app
            .key_up(KeyPress {
                repeat: false,
                ..key
            })
            .unwrap();
        self.settle();
    }

    /// Where something with this key is, as `[x, y, width, height]`.
    pub fn rect_of(&mut self, key: &str) -> [f32; 4] {
        self.settle();
        let mut path = Vec::new();
        assert!(
            path_of(self.app.runtime().root(), key, &mut path),
            "no {key} in\n{}",
            self.app.outline()
        );
        let rect = self.app.layout().rect(&path).expect("it's been laid out");
        [rect.x, rect.y, rect.width, rect.height]
    }

    /// The middle of the first thing in the tree with this role.
    pub fn middle_of_role(&mut self, role: rsui::core::Role) -> (f32, f32) {
        fn find(node: &rsui::core::Node, role: rsui::core::Role, path: &mut Vec<u32>) -> bool {
            if node.role == role {
                return true;
            }
            for (at, child) in node.children.iter().enumerate() {
                path.push(at as u32);
                if find(child, role, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
        self.settle();
        let mut path = Vec::new();
        assert!(
            find(self.app.runtime().root(), role, &mut path),
            "no {role:?} in\n{}",
            self.app.outline()
        );
        let rect = self.app.layout().rect(&path).expect("it's been laid out");
        (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
    }

    pub fn move_at(&mut self, x: f32, y: f32) {
        self.app.pointer_move(x, y);
        self.settle();
    }

    pub fn down_at(&mut self, x: f32, y: f32) {
        self.app.pointer_move(x, y);
        self.settle();
        self.app.pointer_down(x, y, 1);
        self.settle();
    }

    pub fn up_at(&mut self, x: f32, y: f32) {
        self.app.pointer_move(x, y);
        self.settle();
        let _ = self.app.pointer_up(x, y);
        self.settle();
    }

    /// A click with keys held, which are let go of after.
    pub fn click_holding(&mut self, key: &str, held: rsui::runtime::Modifiers) {
        self.app.set_modifiers(held);
        self.click(key);
        self.app.set_modifiers(rsui::runtime::Modifiers::default());
    }
}
