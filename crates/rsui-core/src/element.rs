use std::cmp::Ordering;
use std::fmt;
use std::sync::RwLock;

use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};
use thin_vec::ThinVec;
use triomphe::{Arc, ThinArc};

use crate::Text;
use crate::symbol::Symbol;

/// A shared, immutable element. Reusing a `Node` between two trees lets
/// [`diff`](crate::diff) skip that whole subtree.
pub type Node = Arc<Element>;

/// Number keys from here on were made up for what needs a key and wasn't given one, so they say nothing to someone reading the
/// tree. It's a number nobody would use.
pub const GENERATED_KEYS_FROM: u64 = 1 << 40;

/// Identifies a child among its siblings across updates. Integer and symbol keys
/// cost nothing extra, while a string key needs an allocation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(untagged)]
pub enum Key {
    Int(u64),
    Sym(Symbol),
    Str(Arc<Text>),
}

impl From<Symbol> for Key {
    fn from(value: Symbol) -> Self {
        Key::Sym(value)
    }
}

impl From<u64> for Key {
    fn from(value: u64) -> Self {
        Key::Int(value)
    }
}

impl From<usize> for Key {
    fn from(value: usize) -> Self {
        Key::Int(value as u64)
    }
}

impl From<&str> for Key {
    fn from(value: &str) -> Self {
        Key::Str(Arc::new(value.into()))
    }
}

impl From<String> for Key {
    fn from(value: String) -> Self {
        Key::Str(Arc::new(value.into()))
    }
}

impl From<Text> for Key {
    fn from(value: Text) -> Self {
        Key::Str(Arc::new(value))
    }
}

/// The name of a custom role. Interned like a [`Symbol`], but small enough to
/// keep [`Role`] at 4 bytes. A process can have up to 65,536 custom roles.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CustomRole(u16);

static CUSTOM_ROLES: RwLock<Vec<Symbol>> = RwLock::new(Vec::new());

impl CustomRole {
    pub fn new(name: impl Into<Symbol>) -> Self {
        let name = name.into();
        let position = |roles: &[Symbol]| roles.iter().position(|role| *role == name);
        if let Some(found) = position(&CUSTOM_ROLES.read().expect("lock is not poisoned")) {
            return Self(found as u16);
        }
        let mut roles = CUSTOM_ROLES.write().expect("lock is not poisoned");
        let found = position(&roles).unwrap_or_else(|| {
            roles.push(name);
            roles.len() - 1
        });
        Self(u16::try_from(found).expect("fewer than 65,537 custom roles"))
    }

    pub fn name(self) -> Symbol {
        CUSTOM_ROLES.read().expect("lock is not poisoned")[self.0 as usize]
    }
}

impl From<&str> for CustomRole {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl fmt::Debug for CustomRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.name(), f)
    }
}

impl Serialize for CustomRole {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.name().serialize(serializer)
    }
}

/// What an element is, in terms both agents and assistive technology understand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Role {
    Window,
    Group,
    Text,
    Button,
    Link,
    TextInput,
    Checkbox,
    List,
    ListItem,
    Image,
    /// Something that takes over until it's dealt with.
    Dialog,
    Menu,
    MenuItem,
    Custom(CustomRole),
}

/// Something that can be done to an element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActionKind {
    Click,
    Focus,
    Toggle,
    SetValue,
    Scroll,
    /// A key was pressed. The input is text such as `"enter"` or `"ctrl+s"`.
    KeyPress,
    /// A list has found out how many rows fit in it. The input is that number, and part of a
    /// row counts as part of one.
    Resize,
    /// Someone's finished with a popup, by clicking away from it or pressing escape.
    Dismiss,
    Custom(Symbol),
}

impl Role {
    /// The role's name, as it appears in snapshots.
    pub fn name(self) -> &'static str {
        match self {
            Role::Window => "window",
            Role::Group => "group",
            Role::Text => "text",
            Role::Button => "button",
            Role::Link => "link",
            Role::TextInput => "text_input",
            Role::Checkbox => "checkbox",
            Role::List => "list",
            Role::ListItem => "list_item",
            Role::Image => "image",
            Role::Dialog => "dialog",
            Role::Menu => "menu",
            Role::MenuItem => "menu_item",
            Role::Custom(role) => role.name().as_str(),
        }
    }
}

impl ActionKind {
    /// The action's name, as it appears in snapshots.
    pub fn name(self) -> &'static str {
        match self {
            ActionKind::Click => "click",
            ActionKind::Focus => "focus",
            ActionKind::Toggle => "toggle",
            ActionKind::SetValue => "set_value",
            ActionKind::Scroll => "scroll",
            ActionKind::KeyPress => "key_press",
            ActionKind::Resize => "resize",
            ActionKind::Dismiss => "dismiss",
            ActionKind::Custom(name) => name.as_str(),
        }
    }

    /// The action with this name, if the program knows of one. Safe to call with
    /// names from outside the program, since it never stores anything.
    pub fn from_name(name: &str) -> Option<Self> {
        BUILT_IN_ACTIONS
            .iter()
            .find(|action| action.name() == name)
            .copied()
            .or_else(|| Symbol::existing(name).map(ActionKind::Custom))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Int(key) => write!(f, "{key}"),
            Key::Sym(key) => f.write_str(key.as_str()),
            Key::Str(key) => f.write_str(key),
        }
    }
}

/// The built-in actions, in the order they appear in snapshots. An action's
/// position here is its bit in [`Actions`].
const BUILT_IN_ACTIONS: [ActionKind; 8] = [
    ActionKind::Click,
    ActionKind::Focus,
    ActionKind::Toggle,
    ActionKind::SetValue,
    ActionKind::Scroll,
    ActionKind::KeyPress,
    ActionKind::Resize,
    ActionKind::Dismiss,
];

/// How many low bits of [`Actions`] hold built-in actions. The rest identify a
/// set of custom actions, with zero meaning none.
const BUILT_IN_BITS: u32 = 8;

/// Every distinct set of custom actions, stored once. A set's identifier is its
/// position here plus one.
static CUSTOM_ACTION_SETS: RwLock<Vec<&'static [Symbol]>> = RwLock::new(Vec::new());

/// The set of actions an element supports, in 4 bytes.
///
/// Iteration order is fixed: built-in actions first, then custom ones sorted by
/// name. Custom action names are interned, see [`Symbol`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Actions(u32);

impl Actions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }

    pub fn clear(&mut self) {
        self.0 = 0;
    }

    pub fn insert(&mut self, action: ActionKind) {
        if let ActionKind::Custom(name) = action {
            let mut names = self.custom().to_vec();
            if let Err(position) = names.binary_search(&name) {
                names.insert(position, name);
                let built_in = self.0 & ((1 << BUILT_IN_BITS) - 1);
                self.0 = built_in | (intern_action_set(names) << BUILT_IN_BITS);
            }
        } else if let Some(position) = BUILT_IN_ACTIONS.iter().position(|a| *a == action) {
            self.0 |= 1 << position;
        }
    }

    pub fn contains(&self, action: ActionKind) -> bool {
        if let ActionKind::Custom(name) = action {
            self.custom().binary_search(&name).is_ok()
        } else {
            BUILT_IN_ACTIONS
                .iter()
                .position(|a| *a == action)
                .is_some_and(|position| self.0 & (1 << position) != 0)
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = ActionKind> + use<> {
        let bits = self.0;
        let built_in = BUILT_IN_ACTIONS
            .iter()
            .enumerate()
            .filter(move |(position, _)| bits & (1 << position) != 0)
            .map(|(_, action)| *action);
        built_in.chain(self.custom().iter().map(|name| ActionKind::Custom(*name)))
    }

    fn custom(&self) -> &'static [Symbol] {
        match self.0 >> BUILT_IN_BITS {
            0 => &[],
            set => CUSTOM_ACTION_SETS.read().expect("lock is not poisoned")[set as usize - 1],
        }
    }
}

fn intern_action_set(names: Vec<Symbol>) -> u32 {
    let mut sets = CUSTOM_ACTION_SETS.write().expect("lock is not poisoned");
    let position = sets
        .iter()
        .position(|set| *set == names.as_slice())
        .unwrap_or_else(|| {
            sets.push(names.leak());
            sets.len() - 1
        });
    position as u32 + 1
}

impl Serialize for Actions {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(None)?;
        for action in self.iter() {
            seq.serialize_element(&action)?;
        }
        seq.end()
    }
}

/// A property value, in 16 bytes.
///
/// Floats compare by bit pattern, so a `NaN` equals itself and an unchanged
/// `NaN` produces no patch. JSON has no `NaN`, so snapshots write it as `null`.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum PropValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Arc<Text>),
}

impl PartialEq for PropValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a.to_bits() == b.to_bits(),
            (Self::Str(a), Self::Str(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for PropValue {}

impl From<bool> for PropValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for PropValue {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<f64> for PropValue {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<&str> for PropValue {
    fn from(value: &str) -> Self {
        Self::Str(Arc::new(value.into()))
    }
}

impl From<String> for PropValue {
    fn from(value: String) -> Self {
        Self::Str(Arc::new(value.into()))
    }
}

impl From<Text> for PropValue {
    fn from(value: Text) -> Self {
        Self::Str(Arc::new(value))
    }
}

type PropEntry = (Symbol, PropValue);

/// One rejection bit per name; collisions still use the exact symbol lookup.
#[derive(Clone, Copy, PartialEq, Eq)]
struct PropFilter(u64);

impl PropFilter {
    #[inline]
    fn bit(name: Symbol) -> u64 {
        // Fibonacci hashing spreads aligned interned addresses over all 64 bits.
        let mixed = (name.address() as u64 >> 3).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        1 << (mixed >> 58)
    }

    fn from_names(names: impl Iterator<Item = Symbol>) -> Self {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if !ENABLED.get_or_init(|| std::env::var("RSUI_PROP_FILTER").as_deref() != Ok("0")) {
            return Self(u64::MAX);
        }
        Self(names.fold(0, |mask, name| mask | Self::bit(name)))
    }
}

impl fmt::Debug for PropFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The filter is an implementation detail; preserve Props' debug format.
        fmt::Debug::fmt(&(), f)
    }
}

/// Element properties, kept sorted by name so snapshots are deterministic and
/// two sets can be compared in one pass.
///
/// A set is immutable and shared, so cloning one is one reference count, and
/// comparing two clones of a set is one pointer compare. Giving many elements a
/// clone of the same set is the cheapest way to store them, see
/// [`shared_props!`](crate::shared_props!). An empty set allocates nothing.
///
/// Because a set is immutable, [`insert`](Self::insert) and
/// [`remove`](Self::remove) copy it. To build a set in one step, use
/// `Props::from([(name, value), ...])`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Props(Option<ThinArc<PropFilter, PropEntry>>);

impl Props {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    pub fn len(&self) -> usize {
        self.entries().len()
    }

    pub fn clear(&mut self) {
        self.0 = None;
    }

    /// Replaces this set from a reusable buffer, leaving the buffer empty with
    /// its capacity intact. Entries may be unordered; the last duplicate wins.
    /// An equal set keeps its existing shared allocation.
    pub fn replace_from(&mut self, entries: &mut Vec<(Symbol, PropValue)>) {
        if entries.is_empty() {
            self.clear();
            return;
        }
        entries.sort_by_key(|(name, _)| *name);
        entries.reverse();
        entries.dedup_by_key(|(name, _)| *name);
        entries.reverse();
        if self.entries() == entries.as_slice() {
            entries.clear();
        } else {
            let filter = PropFilter::from_names(entries.iter().map(|(name, _)| *name));
            *self = Self(Some(ThinArc::from_header_and_iter(filter, entries.drain(..))));
        }
    }

    pub fn get(&self, name: &str) -> Option<&PropValue> {
        let position = self.position_of(name)?;
        Some(&self.entries()[position].1)
    }

    /// Like [`get`](Self::get), comparing one pointer per entry instead of searching through
    /// strings, for when this runs often enough to matter.
    #[inline]
    pub fn get_symbol(&self, name: Symbol) -> Option<&PropValue> {
        let entries = self.0.as_ref()?;
        if entries.header.header.0 & PropFilter::bit(name) == 0 {
            return None;
        }
        entries.slice
            .iter()
            .find(|(existing, _)| *existing == name)
            .map(|(_, value)| value)
    }

    /// What this set is, as a number. Two sets with the same number are the same set, so what's
    /// been worked out from one can be reused for the other.
    #[inline]
    pub fn identity(&self) -> usize {
        self.entries().as_ptr() as usize
    }

    pub fn insert(&mut self, name: impl Into<Symbol>, value: impl Into<PropValue>) {
        let name = name.into();
        let mut value = Some(value.into());
        let entries = self.entries();
        let found = entries.binary_search_by(|(existing, _)| existing.cmp(&name));
        let (position, replaced) = match found {
            Ok(position) => (position, 1),
            Err(position) => (position, 0),
        };
        let items = (0..entries.len() + 1 - replaced).map(|index| match index.cmp(&position) {
            Ordering::Less => entries[index].clone(),
            Ordering::Equal => (name, value.take().expect("this position comes once")),
            Ordering::Greater => entries[index + replaced - 1].clone(),
        });
        let filter = PropFilter::from_names(entries.iter().map(|(key, _)| *key).chain(std::iter::once(name)));
        *self = Self(Some(ThinArc::from_header_and_iter(filter, items)));
    }

    pub fn remove(&mut self, name: &str) -> Option<PropValue> {
        let position = self.position_of(name)?;
        let entries = self.entries();
        let removed = entries[position].1.clone();
        let remaining = entries.len() - 1;
        let items = (0..remaining).map(|index| {
            let source = if index < position { index } else { index + 1 };
            entries[source].clone()
        });
        *self = if remaining == 0 {
            Self(None)
        } else {
            let filter = PropFilter::from_names(entries.iter().enumerate()
                .filter(|(index, _)| *index != position).map(|(_, (name, _))| *name));
            Self(Some(ThinArc::from_header_and_iter(filter, items)))
        };
        Some(removed)
    }

    pub fn iter(&self) -> impl Iterator<Item = (Symbol, &PropValue)> {
        self.entries().iter().map(|(name, value)| (*name, value))
    }

    /// These props combined with `other`, whose values win if a name is in both.
    pub fn merged(&self, other: &Props) -> Props {
        Self::from_entries(
            self.entries()
                .iter()
                .chain(other.entries())
                .cloned()
                .collect(),
        )
    }

    pub(crate) fn entries(&self) -> &[PropEntry] {
        match &self.0 {
            Some(entries) => &entries.slice,
            None => &[],
        }
    }

    fn position_of(&self, name: &str) -> Option<usize> {
        self.entries()
            .binary_search_by(|(existing, _)| existing.as_str().cmp(name))
            .ok()
    }

    /// `entries` may be in any order. If a name repeats, the last value wins.
    fn from_entries(mut entries: Vec<PropEntry>) -> Self {
        if entries.is_empty() {
            return Self(None);
        }
        entries.sort_by_key(|(name, _)| *name);
        entries.reverse();
        entries.dedup_by_key(|(name, _)| *name);
        entries.reverse();
        let filter = PropFilter::from_names(entries.iter().map(|(name, _)| *name));
        Self(Some(ThinArc::from_header_and_iter(filter, entries.into_iter())))
    }
}

impl<N: Into<Symbol>, V: Into<PropValue>> FromIterator<(N, V)> for Props {
    fn from_iter<I: IntoIterator<Item = (N, V)>>(entries: I) -> Self {
        Self::from_entries(
            entries
                .into_iter()
                .map(|(name, value)| (name.into(), value.into()))
                .collect(),
        )
    }
}

impl<N: Into<Symbol>, V: Into<PropValue>, const COUNT: usize> From<[(N, V); COUNT]> for Props {
    fn from(entries: [(N, V); COUNT]) -> Self {
        entries.into_iter().collect()
    }
}

/// A set of [`Props`] built once for this line and shared by every element that
/// passes through it. The values must not change between calls.
///
/// ```
/// use rsui_core::{Element, Role, shared_props};
///
/// let button = || Element::new(Role::Button).props(shared_props!("enabled" => true));
/// assert_eq!(button(), button());
/// ```
#[macro_export]
macro_rules! shared_props {
    ($($name:literal => $value:expr),* $(,)?) => {{
        static PROPS: ::std::sync::OnceLock<$crate::Props> = ::std::sync::OnceLock::new();
        PROPS
            .get_or_init(|| {
                let entries: ::std::vec::Vec<($crate::Symbol, $crate::PropValue)> = ::std::vec![
                    $(($crate::Symbol::new($name), $crate::PropValue::from($value))),*
                ];
                entries.into_iter().collect()
            })
            .clone()
    }};
}

impl Serialize for Props {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let entries = self.entries();
        let mut map = serializer.serialize_map(Some(entries.len()))?;
        for (name, value) in entries {
            map.serialize_entry(name, value)?;
        }
        map.end()
    }
}

/// One node of the UI tree. Empty fields are left out of snapshots to keep them
/// small for the agents reading them, and an empty `name` means no name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Element {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<Key>,
    pub role: Role,
    #[serde(skip_serializing_if = "Text::is_empty")]
    pub name: Text,
    #[serde(skip_serializing_if = "Props::is_empty")]
    pub props: Props,
    #[serde(skip_serializing_if = "Actions::is_empty")]
    pub actions: Actions,
    #[serde(skip_serializing_if = "ThinVec::is_empty")]
    pub children: ThinVec<Node>,
}

impl Element {
    pub fn new(role: Role) -> Self {
        Self {
            key: None,
            role,
            name: Text::new(),
            props: Props::new(),
            actions: Actions::new(),
            children: ThinVec::new(),
        }
    }

    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn name(mut self, name: impl Into<Text>) -> Self {
        self.name = name.into();
        self
    }

    pub fn prop(mut self, name: impl Into<Symbol>, value: impl Into<PropValue>) -> Self {
        self.props.insert(name, value);
        self
    }

    /// Replaces all properties. Passing a clone of an existing set shares it.
    pub fn props(mut self, props: impl Into<Props>) -> Self {
        self.props = props.into();
        self
    }

    pub fn action(mut self, action: ActionKind) -> Self {
        self.actions.insert(action);
        self
    }

    /// Adds a child. Pass an existing [`Node`] to share that subtree with
    /// another tree.
    pub fn child(mut self, child: impl IntoNode) -> Self {
        self.children.push(child.into_node());
        self
    }

    pub fn children<C: IntoNode>(mut self, children: impl IntoIterator<Item = C>) -> Self {
        self.children
            .extend(children.into_iter().map(IntoNode::into_node));
        self
    }

    /// Finishes the element, releasing any spare capacity.
    /// Lets the pointer select this text, for copying. Call this after setting props, which replaces them.
    /// What can be selected is a paragraph, so text that's only a name gets its words moved into one part.
    pub fn selectable(mut self) -> Self {
        if self.children.is_empty() && !self.name.is_empty() {
            let words = std::mem::take(&mut self.name);
            self = self.child(Element::new(Role::Text).name(words));
        }
        self.prop("selectable", true)
    }

    pub fn into_node(mut self) -> Node {
        self.shrink_to_fit();
        Arc::new(self)
    }

    pub(crate) fn shrink_to_fit(&mut self) {
        if self.children.len() < self.children.capacity() {
            self.children.shrink_to_fit();
        }
    }

    /// Empties the element for reuse.
    pub(crate) fn reset(&mut self) {
        self.key = None;
        self.name = Text::new();
        self.props.clear();
        self.actions.clear();
        self.children = ThinVec::new();
    }
}

/// Something that can become a child: a finished [`Element`], or a [`Node`] to share.
pub trait IntoNode {
    fn into_node(self) -> Node;
}

impl IntoNode for Element {
    fn into_node(self) -> Node {
        Element::into_node(self)
    }
}

impl IntoNode for Node {
    fn into_node(self) -> Node {
        self
    }
}

#[cfg(test)]
mod prop_filter_tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeMap;

    fn check(props: &Props, expected: &BTreeMap<Symbol, i64>, names: &[Symbol]) {
        for &name in names {
            let value = expected.get(&name).copied().map(PropValue::Int);
            assert_eq!(props.get_symbol(name), value.as_ref(), "{}", name.as_str());
            assert_eq!(props.get(name.as_str()), value.as_ref());
        }
        let entries: Vec<_> = expected.iter().map(|(&name, &value)| (name, PropValue::Int(value))).collect();
        assert_eq!(props.entries(), entries);
        if let Some(bag) = &props.0 {
            let mask = if std::env::var("RSUI_PROP_FILTER").as_deref() == Ok("0") {
                u64::MAX
            } else {
                expected.keys().fold(0, |mask, &name| mask | PropFilter::bit(name))
            };
            assert_eq!(bag.header.header.0, mask);
        }
        assert_eq!(*props, entries.into_iter().collect::<Props>());
    }

    proptest! {
        #[test]
        fn lookup_matches_sorted_map_through_all_mutations(
            operations in prop::collection::vec((0u8..6, 0usize..96, any::<i64>()), 0..140)
        ) {
            let names: Vec<_> = (0..128).map(|i| Symbol::new(&format!("prop-filter-{i:03}"))).collect();
            let mut props = Props::new();
            let mut expected = BTreeMap::new();
            for (operation, key, value) in operations {
                let retained = props.clone();
                let previous = expected.clone();
                assert_eq!(retained.identity(), props.identity());
                match operation {
                    0 => { props.insert(names[key], value); expected.insert(names[key], value); }
                    1 => { assert_eq!(props.remove(names[key].as_str()), expected.remove(&names[key]).map(PropValue::Int)); }
                    2 => {
                        let mut entries = vec![(names[key], PropValue::Int(value)),
                            (names[(key + 1) % 96], PropValue::Int(value.wrapping_add(1))),
                            (names[key], PropValue::Int(value.wrapping_sub(1)))];
                        let capacity = entries.capacity();
                        props.replace_from(&mut entries);
                        assert!(entries.is_empty()); assert_eq!(entries.capacity(), capacity);
                        expected.clear(); expected.insert(names[key], value.wrapping_sub(1));
                        expected.insert(names[(key + 1) % 96], value.wrapping_add(1));
                    }
                    3 => {
                        let other: Props = [(names[key], value), (names[(key + 1) % 96], value.wrapping_add(1)),
                            (names[key], value.wrapping_sub(1))].into();
                        props = props.merged(&other);
                        expected.insert(names[key], value.wrapping_sub(1));
                        expected.insert(names[(key + 1) % 96], value.wrapping_add(1));
                    }
                    4 => { props.clear(); expected.clear(); }
                    _ => {
                        let mut entries: Vec<_> = props.iter().map(|(name, value)| (name, value.clone())).collect();
                        entries.reverse();
                        let identity = props.identity();
                        props.replace_from(&mut entries);
                        assert_eq!(props.identity(), identity);
                    }
                }
                check(&props, &expected, &names);
                check(&retained, &previous, &names);
            }
        }
    }

    #[test]
    fn collisions_never_hide_present_keys_or_create_false_matches() {
        let mut buckets = [None; 64];
        let (one, two) = (0..65).find_map(|i| {
            let name = Symbol::new(&format!("prop-filter-collision-{i}"));
            let bucket = PropFilter::bit(name).trailing_zeros() as usize;
            buckets[bucket].replace(name).map(|previous| (previous, name))
        }).expect("65 keys must collide in 64 bits");
        let mut props: Props = [(one, 1i64)].into();
        assert_eq!(props.get_symbol(two), None);
        props.insert(two, 2i64);
        assert_eq!(props.get_symbol(one), Some(&PropValue::Int(1)));
        assert_eq!(props.get_symbol(two), Some(&PropValue::Int(2)));
        props.remove(one.as_str());
        assert_eq!(props, Props::from([(two, 2i64)]));
        assert_eq!(props.get_symbol(one), None);
        props.remove(two.as_str());
        assert_eq!(props, Props::new());
    }

    #[test]
    fn handle_layout_debug_and_serialization_are_unchanged() {
        assert_eq!(std::mem::size_of::<Props>(), std::mem::size_of::<usize>());
        assert_eq!(std::mem::size_of::<PropFilter>(), 8);
        type Old = triomphe::HeaderSlice<triomphe::HeaderWithLength<()>, [PropEntry; 1]>;
        type New = triomphe::HeaderSlice<triomphe::HeaderWithLength<PropFilter>, [PropEntry; 1]>;
        assert_eq!(std::mem::align_of::<Old>(), std::mem::align_of::<New>());
        assert_eq!(std::mem::offset_of!(New, slice) - std::mem::offset_of!(Old, slice), 8);
        let props: Props = [("z", 2i64), ("a", 1), ("z", 3)].into();
        let old = Some(ThinArc::from_header_and_iter((), props.entries().iter().cloned()));
        assert_eq!(format!("{props:?}"), format!("Props({old:?})"));
        assert_eq!(serde_json::to_string(&props).unwrap(), r#"{"a":1,"z":3}"#);
    }
}
