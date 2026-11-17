//! Immutable build-time module bytes, under the original module identities.
//!
//! Separate from debugger source filters: transformed files may be recorded as
//! bytecode, and packing must write exactly the source that was compiled.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct SourceOverlay(Arc<HashMap<String, String>>);

impl SourceOverlay {
    pub fn new(files: impl IntoIterator<Item = (PathBuf, String)>) -> Self {
        Self(Arc::new(
            files
                .into_iter()
                .map(|(path, source)| {
                    (
                        crate::packed::key(&crate::modules::canonical(&path)),
                        source,
                    )
                })
                .collect(),
        ))
    }

    pub fn get(&self, path: &Path) -> Option<&str> {
        self.0.get(&crate::packed::key(path)).map(String::as_str)
    }
}

thread_local! {
    static ACTIVE: RefCell<Option<SourceOverlay>> = const { RefCell::new(None) };
}

/// Runs a build with immutable replacement bytes. Nested scopes and unwinding
/// restore the previous view; ordinary engines never install an overlay.
pub fn with_source_overlay<R>(overlay: SourceOverlay, run: impl FnOnce() -> R) -> R {
    struct Restore(Option<SourceOverlay>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE.with(|active| *active.borrow_mut() = self.0.take());
        }
    }
    let previous = ACTIVE.with(|active| active.replace(Some(overlay)));
    let _restore = Restore(previous);
    run()
}

pub(crate) fn contains(path: &Path) -> bool {
    ACTIVE.with(|active| {
        active
            .borrow()
            .as_ref()
            .is_some_and(|overlay| overlay.get(path).is_some())
    })
}

pub(crate) fn is_active() -> bool {
    ACTIVE.with(|active| active.borrow().as_ref().is_some_and(|overlay| !overlay.0.is_empty()))
}

pub(crate) fn source(path: &Path) -> Option<String> {
    ACTIVE.with(|active| active.borrow().as_ref()?.get(path).map(str::to_owned))
}

pub(crate) fn bytes(path: &Path) -> std::io::Result<Vec<u8>> {
    match source(path) {
        Some(source) => Ok(source.into_bytes()),
        None => std::fs::read(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_overlays_keep_identity_record_bytecode_and_restore_the_disk_view() {
        let folder =
            std::env::temp_dir().join(format!("rsui-source-overlay-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("app.tsx");
        let disk = "export const value = 'original';";
        std::fs::write(&file, disk).unwrap();
        let file = crate::modules::canonical(&file);
        let transformed = "\u{feff}#!/usr/bin/env node\nexport function compiled() { return 'transformed'; }\nglobalThis.result = compiled() + '|' + compiled.toString() + '|' + import.meta.url;";
        let overlay = SourceOverlay::new([(file.clone(), transformed.to_owned())]);
        with_source_overlay(overlay, || {
            let source = crate::modules::read_source(&file).unwrap();
            assert!(source.starts_with("//#!"));
            let mut engine = crate::Engine::new();
            let printed = std::rc::Rc::new(RefCell::new(String::new()));
            let into = printed.clone();
            engine.set_printer(move |text| into.borrow_mut().push_str(text));
            engine.keep_compiled_code();
            engine.run_file(&file).unwrap();
            engine.eval("console.log(result)", "check.js").unwrap();
            let text = printed.borrow();
            assert!(text.contains("transformed|function compiled()"), "{text}");
            assert!(text.trim_end().replace('\\', "/").ends_with("/app.tsx"), "{text}");
            assert!(!engine.take_compiled_code().unwrap().is_empty());
            assert_eq!(bytes(&file).unwrap(), transformed.as_bytes());
            with_source_overlay(SourceOverlay::default(), || {
                assert_eq!(crate::modules::read_source(&file).unwrap(), disk)
            });
            assert!(contains(&file));
        });
        assert!(!contains(&file));
        assert_eq!(crate::modules::read_source(&file).unwrap(), disk);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), disk);
        let _ = std::fs::remove_dir_all(folder);
    }
}
