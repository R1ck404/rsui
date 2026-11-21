//! Where a web view's runtime is, on systems without one here yet: nowhere.

use std::path::PathBuf;

/// Where a runtime is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    pub version: String,
    /// The library that makes environments.
    pub client: PathBuf,
    /// Whether it's a runtime in a folder of its own, rather than the one that's installed.
    pub fixed: bool,
}

/// There isn't one.
pub fn find() -> Option<Runtime> {
    None
}

/// What's said when there's no runtime.
pub fn missing() -> String {
    "Web views aren't on this system yet.".to_owned()
}
