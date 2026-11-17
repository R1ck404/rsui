use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, RwLock};

use foldhash::fast::RandomState;
use serde::{Serialize, Serializer};

/// A name stored once for the life of the process, so that every use of it costs
/// one pointer and comparing two of them is one pointer compare.
///
/// Interned names are never freed. They suit names that come from code, like
/// property names and custom roles, and must not be built from unbounded input.
#[derive(Clone, Copy)]
pub struct Symbol(&'static &'static str);

static SYMBOLS: LazyLock<RwLock<HashMap<&'static str, Symbol, RandomState>>> =
    LazyLock::new(Default::default);

impl Symbol {
    /// A number that's the same for the same name and different for any other, while the program
    /// runs, for looking symbols up faster than comparing them one at a time.
    #[inline]
    pub fn address(self) -> usize {
        std::ptr::from_ref(self.0) as usize
    }

    /// Finds or stores `name`. To avoid the lookup on repeated calls, use [`symbol!`](crate::symbol!).
    pub fn new(name: &str) -> Self {
        if let Some(symbol) = SYMBOLS.read().expect("lock is not poisoned").get(name) {
            return *symbol;
        }
        let mut symbols = SYMBOLS.write().expect("lock is not poisoned");
        if let Some(symbol) = symbols.get(name) {
            return *symbol;
        }
        let name: &'static str = Box::leak(name.into());
        let symbol = Self(Box::leak(Box::new(name)));
        symbols.insert(name, symbol);
        symbol
    }

    /// Finds `name` if it's already stored, without storing it. Use this for names
    /// from outside the program, since stored names are never freed.
    pub fn existing(name: &str) -> Option<Self> {
        SYMBOLS
            .read()
            .expect("lock is not poisoned")
            .get(name)
            .copied()
    }

    pub fn as_str(self) -> &'static str {
        self.0
    }
}

/// A [`Symbol`] looked up only the first time this line runs.
#[macro_export]
macro_rules! symbol {
    ($name:literal) => {{
        static SYMBOL: ::std::sync::OnceLock<$crate::Symbol> = ::std::sync::OnceLock::new();
        *SYMBOL.get_or_init(|| $crate::Symbol::new($name))
    }};
}

impl PartialEq for Symbol {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl Eq for Symbol {}

impl Hash for Symbol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::ptr::hash(self.0, state);
    }
}

/// Orders by name, so the order is the same in every run of a program.
impl Ord for Symbol {
    fn cmp(&self, other: &Self) -> Ordering {
        if self == other {
            Ordering::Equal
        } else {
            self.as_str().cmp(other.as_str())
        }
    }
}

impl PartialOrd for Symbol {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl Serialize for Symbol {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl From<&str> for Symbol {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl From<String> for Symbol {
    fn from(name: String) -> Self {
        Self::new(&name)
    }
}
