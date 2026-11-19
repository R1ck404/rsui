//! Whether the page's rules about more than an element itself are about it, as `ruleMatches` in web.js says: the rules of a
//! plan (see `planOf`) that have something above the element in them, as `.m_d08caa0 :where(h1)` has, which Mantine has some
//! sixty of for every element, nearly none of them about it. web.js reads each rule's selector into this once, keeps the number
//! it's given on the rule, and asks about all of a plan's at once for an element; what they say is still put on in web.js.
//!
//! Each step is matched as `stepMatches` does it, and what's found for each place above an element is remembered as
//! `matchedAbove` remembers it, for as long as what CSS says and what's been found of how things flow are the same, so that
//! what's inside one element shares what was found above it. `+` and `~` look at what's before an element among what it's in
//! (`prevs`, which web.js gives each element it describes), with the same chain above them. What can't be told here is asked
//! of web.js again (`ASK`): a selector with `:has()`, validity, `:nth-child(… of S)`, `@container` and `:scope` is never read
//! in, and `:focus-visible` on what's focused without a ring, or an attribute whose value is an object, is asked about when
//! it's met.

use std::cell::RefCell;
use std::collections::HashMap as StdHashMap;
use std::rc::Rc;

use rsui_js::{Args, Atom, HostFunction, Value, Vm};

type Run<T> = Result<T, Value>;
/// A map with a quick hash, as what's looked up here is looked up for each rule of each element.
type HashMap<K, V> = StdHashMap<K, V, foldhash::fast::FixedState>;

const HOVER: u32 = 1;
const PRESS: u32 = 2;
const FOCUS: u32 = 4;
const WITHIN: u32 = 8;
const RING: u32 = 32;
/// Every state something can be in, as web.js's `ALL_STATES`.
const ALL_STATES: u32 = HOVER | PRESS | FOCUS | WITHIN | RING;

/// What `cssMatchItems` says of each rule: not about it, about it, about it only in some other state (`stateNeeds`), or ask
/// web.js.
const NO: u8 = 0;
const YES: u8 = 1;
const SOME_STATE: u8 = 2;
const ASK: u8 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum M {
    No,
    Yes,
    Ask,
}

impl M {
    fn of(yes: bool) -> M {
        if yes { M::Yes } else { M::No }
    }

    fn and(self, other: M) -> M {
        match (self, other) {
            (M::No, _) | (_, M::No) => M::No,
            (M::Ask, _) | (_, M::Ask) => M::Ask,
            _ => M::Yes,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Link {
    Descendant,
    Child,
    /// `+`: the element just before, among what they're in.
    Adjacent,
    /// `~`: any element before, among what they're in.
    Sibling,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Op {
    Has,
    Equals,
    Word,
    Prefix,
    Suffix,
    Contains,
    Dash,
    Never,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Attr {
    /// The prop it reads, as React has it (`className`, `htmlFor`).
    prop: String,
    /// `aria-` and `data-` attributes say `false` as the word, as React writes them.
    spelled: bool,
    op: Op,
    value: String,
}

/// `an+b`, as `nthMatches` reads it.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Nth {
    Odd,
    Even,
    Formula(f64, f64),
    Index(f64),
}

#[derive(Clone, PartialEq, Debug)]
enum Pseudo {
    Hover,
    Active,
    Focus,
    FocusVisible,
    FocusWithin,
    Checked,
    Disabled,
    Enabled,
    Required,
    Optional,
    ReadOnly,
    ReadWrite,
    PlaceholderShown,
    Empty,
    /// `:not()`, with the selectors in it.
    Not(Vec<u32>),
    /// `:is()`, `:where()` and `:matches()`.
    Is(Vec<u32>),
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    OnlyOfType,
    NthChild(Nth),
    NthLastChild(Nth),
    NthOfType(Nth),
    NthLastOfType(Nth),
    Link,
    Modal,
    Open,
    Root,
    Always(bool),
}

#[derive(Debug)]
struct Step {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<u32>,
    attrs: Vec<Attr>,
    pseudo: Vec<Pseudo>,
    link: Link,
}

/// The names read from what web.js gives, made once for each engine.
struct Names {
    tag: Atom,
    id: Atom,
    classes: Atom,
    props: Atom,
    state: Atom,
    nth: Atom,
    empty: Atom,
    index: Atom,
    count: Atom,
    type_index: Atom,
    type_count: Atom,
    el: Atom,
    up: Atom,
    node: Atom,
    name: Atom,
    arg: Atom,
    list: Atom,
    of: Atom,
    attrs: Atom,
    pseudo: Atom,
    link: Atom,
    op: Atom,
    value: Atom,
    unknown: Atom,
    checked: Atom,
    disabled: Atom,
    required: Atom,
    read_only: Atom,
    placeholder: Atom,
    href: Atom,
    modal: Atom,
    open: Atom,
    prevs: Atom,
}

impl Names {
    fn new(vm: &mut Vm) -> Names {
        Names {
            tag: vm.atom("tag"),
            id: vm.atom("id"),
            classes: vm.atom("classes"),
            props: vm.atom("props"),
            state: vm.atom("state"),
            nth: vm.atom("nth"),
            empty: vm.atom("empty"),
            index: vm.atom("index"),
            count: vm.atom("count"),
            type_index: vm.atom("typeIndex"),
            type_count: vm.atom("typeCount"),
            el: vm.atom("el"),
            up: vm.atom("up"),
            node: vm.atom("__matchNode"),
            name: vm.atom("name"),
            arg: vm.atom("arg"),
            list: vm.atom("list"),
            of: vm.atom("of"),
            attrs: vm.atom("attrs"),
            pseudo: vm.atom("pseudo"),
            link: vm.atom("link"),
            op: vm.atom("op"),
            value: vm.atom("value"),
            unknown: vm.atom("unknown"),
            checked: vm.atom("checked"),
            disabled: vm.atom("disabled"),
            required: vm.atom("required"),
            read_only: vm.atom("readOnly"),
            placeholder: vm.atom("placeholder"),
            href: vm.atom("href"),
            modal: vm.atom("__modal"),
            open: vm.atom("open"),
            prevs: vm.atom("prevs"),
        }
    }
}

/// An element as a selector sees it, read once for each `cssMatchItems`.
struct El {
    tag: Option<String>,
    id: Option<String>,
    /// Its classes, by the numbers the steps have them by, or `u32::MAX` for one no step is about.
    classes: Vec<u32>,
    props: Value,
    state: u32,
    /// Where it is among what's around it: `index`, `count`, `typeIndex`, `typeCount`, each `NaN` where it isn't said.
    nth: Option<[f64; 4]>,
    empty: bool,
    /// What's before it among what it's in, as web.js describes them (`own.prevs`), or `undefined`.
    prevs: Value,
}

#[derive(Default)]
struct Store {
    names: Option<Names>,
    steps: Vec<Rc<Step>>,
    selectors: Vec<Vec<u32>>,
    selector_ids: HashMap<String, u32>,
    classes: HashMap<String, u32>,
    attr_atoms: HashMap<String, Atom>,
    /// What's been found for a place above something, as `matchedAbove` keeps it: by the place's number, the selector, the
    /// step, and whether the page is beyond it and it's `matchFrom`'s rather than `matchChain`'s.
    memo: HashMap<(u32, u32, u16, u8), bool>,
    epoch: (f64, f64),
    next_node: u32,
    els: HashMap<u64, El>,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store { epoch: (f64::NAN, f64::NAN), ..Store::default() });
}

/// Forgets everything, for an engine that starts on a thread another has been on: what's kept is by its atoms and objects.
pub(crate) fn reset() {
    STORE.with(|store| *store.borrow_mut() = Store { epoch: (f64::NAN, f64::NAN), ..Store::default() });
}

/// The spaces `\s` matches in JavaScript.
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{2028}' | '\u{2029}'
            | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

/// `Number(text)` for what an `an+b` holds: decimal numbers, `0x` hex, and nothing, which is 0.
fn js_number(text: &str) -> f64 {
    let text = text.trim_matches(js_space);
    if text.is_empty() {
        return 0.0;
    }
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16).map_or(f64::NAN, |n| n as f64);
    }
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    if body == "Infinity" {
        return if text.starts_with('-') { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    if body.is_empty() || !body.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-')) {
        return f64::NAN;
    }
    text.parse::<f64>().unwrap_or(f64::NAN)
}

impl Nth {
    /// As `nthMatches` reads its argument: spaces taken out, `odd`, `even`, `an+b`, or a number.
    fn read(arg: &str) -> Nth {
        let text: String = arg.chars().filter(|c| !js_space(*c)).collect();
        match text.as_str() {
            "odd" => return Nth::Odd,
            "even" => return Nth::Even,
            _ => {}
        }
        // /^([+-]?\d*)n([+-]\d+)?$/
        if let Some(n) = text.find('n') {
            let (a, rest) = (&text[..n], &text[n + 1..]);
            let a_ok = a.strip_prefix(['+', '-']).unwrap_or(a).bytes().all(|b| b.is_ascii_digit());
            let b_ok = rest.is_empty()
                || (rest.len() > 1 && rest.starts_with(['+', '-']) && rest[1..].bytes().all(|b| b.is_ascii_digit()));
            if a_ok && b_ok {
                let a = match a {
                    "" | "+" => 1.0,
                    "-" => -1.0,
                    _ => js_number(a),
                };
                return Nth::Formula(a, if rest.is_empty() { 0.0 } else { js_number(rest) });
            }
        }
        Nth::Index(js_number(&text))
    }

    fn matches(self, index: f64) -> bool {
        match self {
            Nth::Odd => index % 2.0 == 1.0,
            Nth::Even => index % 2.0 == 0.0,
            Nth::Index(at) => index == at,
            Nth::Formula(a, b) => {
                if a == 0.0 {
                    index == b
                } else {
                    (index - b) / a >= 0.0 && (index - b) % a == 0.0
                }
            }
        }
    }
}

/// `text.split(/\s+/).includes(word)`: a run of spaces is one break, with an empty word before spaces at the start and after
/// spaces at the end.
fn has_word(text: &str, word: &str) -> bool {
    let mut start = 0;
    let mut space_from: Option<usize> = None;
    for (at, c) in text.char_indices() {
        if js_space(c) {
            if space_from.is_none() {
                space_from = Some(at);
            }
        } else if let Some(from) = space_from.take() {
            if &text[start..from] == word {
                return true;
            }
            start = at;
        }
    }
    match space_from {
        Some(from) => &text[start..from] == word || word.is_empty(),
        None => &text[start..] == word,
    }
}

fn text_of(vm: &Vm, value: Value) -> Option<String> {
    vm.string_text(value).map(|text| text.into_owned())
}

impl Store {
    fn names(&mut self, vm: &mut Vm) {
        if self.names.is_none() {
            self.names = Some(Names::new(vm));
        }
    }

    fn n(&self) -> &Names {
        self.names.as_ref().expect("names are made first")
    }

    fn class_number(&mut self, name: &str) -> u32 {
        if let Some(&number) = self.classes.get(name) {
            return number;
        }
        let number = self.classes.len() as u32;
        self.classes.insert(name.to_owned(), number);
        number
    }

    fn attr_atom(&mut self, vm: &mut Vm, name: &str) -> Atom {
        if let Some(&atom) = self.attr_atoms.get(name) {
            return atom;
        }
        let atom = vm.atom(name);
        self.attr_atoms.insert(name.to_owned(), atom);
        atom
    }

    /// A selector's steps, as `readSelector` made them, as the number it's kept by, or `None` for what's left to web.js. The
    /// first of a rule's steps is what it belongs to, which `ruleMatches` never looks at, so `owned` reads it as anything.
    fn read_selector(&mut self, vm: &mut Vm, steps: Value, owned: bool) -> Run<Option<u32>> {
        let Some(items) = vm.array_elements(steps).map(<[Value]>::to_vec) else {
            return Ok(None);
        };
        if items.is_empty() {
            return Ok(None);
        }
        let mut ids = Vec::with_capacity(items.len());
        for (at, step) in items.into_iter().enumerate() {
            let id = if owned && at == 0 { self.any_step() } else { self.read_step(vm, step)? };
            let Some(id) = id else {
                return Ok(None);
            };
            ids.push(id);
        }
        let key = ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        if let Some(&found) = self.selector_ids.get(&key) {
            return Ok(Some(found));
        }
        let number = self.selectors.len() as u32;
        self.selectors.push(ids);
        self.selector_ids.insert(key, number);
        Ok(Some(number))
    }

    fn any_step(&mut self) -> Option<u32> {
        self.keep_step(Step { tag: None, id: None, classes: Vec::new(), attrs: Vec::new(), pseudo: Vec::new(), link: Link::Descendant })
    }

    fn keep_step(&mut self, step: Step) -> Option<u32> {
        // Steps aren't shared between selectors by what they say, as each is small and there are only as many as rules.
        self.steps.push(Rc::new(step));
        Some(self.steps.len() as u32 - 1)
    }

    fn read_step(&mut self, vm: &mut Vm, step: Value) -> Run<Option<u32>> {
        if !step.is_object() {
            return Ok(None);
        }
        self.names(vm);
        let n = self.n();
        let (tag_name, id_name, classes_name, attrs_name, pseudo_name, link_name, unknown_name) =
            (n.tag, n.id, n.classes, n.attrs, n.pseudo, n.link, n.unknown);
        let (name_name, op_name, value_name, arg_name, list_name, of_name) = (n.name, n.op, n.value, n.arg, n.list, n.of);
        if vm.get_property(step, unknown_name)? == Value::TRUE {
            return Ok(None);
        }
        let tag = vm.get_property(step, tag_name)?;
        let tag = if tag.is_nullish() {
            None
        } else {
            match text_of(vm, tag) {
                Some(text) => Some(text),
                None => return Ok(None),
            }
        };
        let id = vm.get_property(step, id_name)?;
        let id = if id.is_nullish() {
            None
        } else {
            match text_of(vm, id) {
                Some(text) => Some(text),
                None => return Ok(None),
            }
        };
        let link = vm.get_property(step, link_name)?;
        let link = if link.is_nullish() {
            Link::Descendant
        } else {
            match text_of(vm, link).as_deref() {
                Some("descendant") => Link::Descendant,
                Some("child") => Link::Child,
                Some("adjacent") => Link::Adjacent,
                Some("sibling") => Link::Sibling,
                _ => return Ok(None),
            }
        };
        let mut classes = Vec::new();
        let list = vm.get_property(step, classes_name)?;
        for one in vm.array_elements(list).map(<[Value]>::to_vec).unwrap_or_default() {
            let Some(name) = text_of(vm, one) else {
                return Ok(None);
            };
            classes.push(self.class_number(&name));
        }
        let mut attrs = Vec::new();
        let list = vm.get_property(step, attrs_name)?;
        for one in vm.array_elements(list).map(<[Value]>::to_vec).unwrap_or_default() {
            if !one.is_object() {
                return Ok(None);
            }
            let name = vm.get_property(one, name_name)?;
            let Some(name) = text_of(vm, name) else {
                return Ok(None);
            };
            let op = vm.get_property(one, op_name)?;
            let op = if op.is_undefined() {
                Op::Has
            } else {
                match text_of(vm, op).as_deref() {
                    Some("=") => Op::Equals,
                    Some("~=") => Op::Word,
                    Some("^=") => Op::Prefix,
                    Some("$=") => Op::Suffix,
                    Some("*=") => Op::Contains,
                    Some("|=") => Op::Dash,
                    Some(_) => Op::Never,
                    None => return Ok(None),
                }
            };
            let value = vm.get_property(one, value_name)?;
            let value = if op == Op::Has || op == Op::Never {
                String::new()
            } else {
                match text_of(vm, value) {
                    Some(text) => text,
                    // `undefined` as text is what `String(value) === undefined` never is.
                    None if value.is_undefined() => {
                        attrs.push(Attr { prop: String::new(), spelled: false, op: Op::Never, value: String::new() });
                        continue;
                    }
                    None => return Ok(None),
                }
            };
            let prop = match name.as_str() {
                "class" => "className".to_owned(),
                "for" => "htmlFor".to_owned(),
                _ => name.clone(),
            };
            let spelled = name.starts_with("aria-") || name.starts_with("data-");
            attrs.push(Attr { prop, spelled, op, value });
        }
        let mut pseudo = Vec::new();
        let list = vm.get_property(step, pseudo_name)?;
        for one in vm.array_elements(list).map(<[Value]>::to_vec).unwrap_or_default() {
            if !one.is_object() {
                return Ok(None);
            }
            let name = vm.get_property(one, name_name)?;
            let Some(name) = text_of(vm, name) else {
                return Ok(None);
            };
            let of = vm.get_property(one, of_name)?;
            let arg = vm.get_property(one, arg_name)?;
            let arg = text_of(vm, arg).unwrap_or_default();
            let nth = || Nth::read(&arg);
            let read = match name.as_str() {
                "hover" => Pseudo::Hover,
                "active" => Pseudo::Active,
                "focus" => Pseudo::Focus,
                "focus-visible" => Pseudo::FocusVisible,
                "focus-within" => Pseudo::FocusWithin,
                "checked" => Pseudo::Checked,
                "disabled" => Pseudo::Disabled,
                "enabled" => Pseudo::Enabled,
                "required" => Pseudo::Required,
                "optional" => Pseudo::Optional,
                "read-only" => Pseudo::ReadOnly,
                "read-write" => Pseudo::ReadWrite,
                "placeholder-shown" => Pseudo::PlaceholderShown,
                "empty" => Pseudo::Empty,
                "first-child" => Pseudo::FirstChild,
                "last-child" => Pseudo::LastChild,
                "only-child" => Pseudo::OnlyChild,
                "first-of-type" => Pseudo::FirstOfType,
                "last-of-type" => Pseudo::LastOfType,
                "only-of-type" => Pseudo::OnlyOfType,
                "nth-child" | "nth-last-child" if !of.is_undefined() => return Ok(None),
                "nth-child" => Pseudo::NthChild(nth()),
                "nth-last-child" => Pseudo::NthLastChild(nth()),
                "nth-of-type" => Pseudo::NthOfType(nth()),
                "nth-last-of-type" => Pseudo::NthLastOfType(nth()),
                "link" | "any-link" => Pseudo::Link,
                "modal" => Pseudo::Modal,
                "open" => Pseudo::Open,
                "root" => Pseudo::Root,
                "in-range" => Pseudo::Always(true),
                "visited" | "target" | "host" | "host-context" | "indeterminate" | "out-of-range" | "fullscreen"
                | "popover-open" => Pseudo::Always(false),
                "not" | "is" | "where" | "matches" => {
                    let list = vm.get_property(one, list_name)?;
                    let mut selectors = Vec::new();
                    for steps in vm.array_elements(list).map(<[Value]>::to_vec).unwrap_or_default() {
                        // What couldn't be read is left out, as `selector !== null` has it.
                        if steps.is_null() {
                            continue;
                        }
                        let Some(selector) = self.read_selector(vm, steps, false)? else {
                            return Ok(None);
                        };
                        selectors.push(selector);
                    }
                    if name == "not" { Pseudo::Not(selectors) } else { Pseudo::Is(selectors) }
                }
                // `:has()`, validity, `@container`, `:scope` and the rest are web.js's.
                _ => return Ok(None),
            };
            pseudo.push(read);
        }
        Ok(self.keep_step(Step { tag, id, classes, attrs, pseudo, link }))
    }

    /// Reads an element, `own` or one above it, once for each call.
    fn el(&mut self, vm: &mut Vm, value: Value) -> Run<()> {
        if self.els.contains_key(&value.bits()) {
            return Ok(());
        }
        let n = self.n();
        let (tag_name, id_name, classes_name, props_name, state_name, nth_name, empty_name) =
            (n.tag, n.id, n.classes, n.props, n.state, n.nth, n.empty);
        let (index_name, count_name, type_index_name, type_count_name) = (n.index, n.count, n.type_index, n.type_count);
        let prevs_name = n.prevs;
        let found = if value.is_object() {
            let tag = vm.get_property(value, tag_name)?;
            let id = vm.get_property(value, id_name)?;
            let list = vm.get_property(value, classes_name)?;
            let classes = vm
                .array_elements(list)
                .map(|items| {
                    items.iter().map(|&one| vm.string_text(one).and_then(|text| self.classes.get(text.as_ref()).copied()).unwrap_or(u32::MAX)).collect()
                })
                .unwrap_or_default();
            let props = vm.get_property(value, props_name)?;
            let state = vm.get_property(value, state_name)?;
            let nth = vm.get_property(value, nth_name)?;
            let nth = if nth.is_object() {
                let mut at = [f64::NAN; 4];
                for (slot, name) in [index_name, count_name, type_index_name, type_count_name].into_iter().enumerate() {
                    let one = vm.get_property(nth, name)?;
                    if one.is_number() {
                        at[slot] = one.as_number();
                    }
                }
                Some(at)
            } else {
                None
            };
            let empty = vm.get_property(value, empty_name)? == Value::TRUE;
            let prevs = vm.get_property(value, prevs_name)?;
            El {
                tag: text_of(vm, tag),
                id: text_of(vm, id),
                classes,
                props,
                state: if state.is_number() { state.as_number() as u32 } else { 0 },
                nth,
                empty,
                prevs,
            }
        } else {
            El { tag: None, id: None, classes: Vec::new(), props: Value::UNDEFINED, state: 0, nth: None, empty: false, prevs: Value::UNDEFINED }
        };
        self.els.insert(value.bits(), found);
        Ok(())
    }

    fn prop(&self, vm: &mut Vm, props: Value, name: Atom) -> Run<Value> {
        if props.is_object() { vm.get_property(props, name) } else { Ok(Value::UNDEFINED) }
    }

    fn attr_matches(&mut self, vm: &mut Vm, attr: &Attr, props: Value) -> Run<M> {
        if attr.op == Op::Never && attr.prop.is_empty() {
            return Ok(M::No);
        }
        let atom = self.attr_atom(vm, &attr.prop);
        let value = self.prop(vm, props, atom)?;
        let spelled = value == Value::FALSE && attr.spelled;
        if value.is_undefined() || value.is_null() || (value == Value::FALSE && !spelled) {
            return Ok(M::No);
        }
        if attr.op == Op::Has {
            return Ok(M::Yes);
        }
        let said = if value.is_string() {
            text_of(vm, value).unwrap_or_default()
        } else if value.is_bool() {
            (if value == Value::TRUE { "true" } else { "false" }).to_owned()
        } else if value.is_number() {
            match vm.plain_text(value) {
                Some(text) => text,
                None => return Ok(M::Ask),
            }
        } else {
            // `String()` of an object or a symbol is web.js's to say.
            return Ok(M::Ask);
        };
        let want = attr.value.as_str();
        Ok(M::of(match attr.op {
            Op::Equals => said == want,
            Op::Word => has_word(&said, want),
            Op::Prefix => said.starts_with(want),
            Op::Suffix => said.ends_with(want),
            Op::Contains => said.contains(want),
            Op::Dash => said == want || (said.starts_with(want) && said[want.len()..].starts_with('-')),
            Op::Has => true,
            Op::Never => false,
        }))
    }

    /// `stepMatches`, with `state` as the element's states.
    fn step_matches(&mut self, vm: &mut Vm, step: u32, el: Value, state: u32, chain: Value, page: Value) -> Run<M> {
        let step = Rc::clone(&self.steps[step as usize]);
        let (props, empty, nth, tag_is_html, field) = {
            let found = &self.els[&el.bits()];
            if step.tag.as_ref().is_some_and(|tag| found.tag.as_deref() != Some(tag.as_str()))
                || step.id.as_ref().is_some_and(|id| found.id.as_deref() != Some(id.as_str()))
                || !step.classes.iter().all(|want| found.classes.contains(want))
            {
                return Ok(M::No);
            }
            let field = matches!(found.tag.as_deref(), Some("input" | "textarea" | "select"));
            (found.props, found.empty, found.nth, found.tag.as_deref() == Some("html"), field)
        };
        let mut result = M::Yes;
        for attr in &step.attrs {
            match self.attr_matches(vm, attr, props)? {
                M::No => return Ok(M::No),
                M::Ask => result = M::Ask,
                M::Yes => {}
            }
        }
        for pseudo in &step.pseudo {
            let n = self.n();
            let (checked, disabled, required, read_only, placeholder, href, modal, open, value_name) =
                (n.checked, n.disabled, n.required, n.read_only, n.placeholder, n.href, n.modal, n.open, n.value);
            let at = nth.unwrap_or([f64::NAN; 4]);
            let placed = nth.is_some();
            let here = match pseudo {
                Pseudo::Hover => M::of(state & HOVER != 0),
                Pseudo::Active => M::of(state & PRESS != 0),
                Pseudo::Focus => M::of(state & FOCUS != 0),
                Pseudo::FocusVisible => {
                    if state & FOCUS == 0 {
                        M::No
                    } else if state & RING != 0 {
                        M::Yes
                    } else {
                        // Whether it's typed in (`typedIn`) is web.js's to say.
                        M::Ask
                    }
                }
                Pseudo::FocusWithin => M::of(state & (FOCUS | WITHIN) != 0),
                Pseudo::Checked => M::of(self.prop(vm, props, checked)? == Value::TRUE),
                Pseudo::Disabled => M::of(self.prop(vm, props, disabled)? == Value::TRUE),
                Pseudo::Enabled => M::of(self.prop(vm, props, disabled)? != Value::TRUE),
                Pseudo::Required | Pseudo::Optional => {
                    let said = self.prop(vm, props, required)?;
                    let says = said == Value::TRUE || vm.string_text(said).is_some_and(|text| text.is_empty());
                    M::of(field && (says == matches!(pseudo, Pseudo::Required)))
                }
                Pseudo::ReadOnly => M::of(self.prop(vm, props, read_only)? == Value::TRUE),
                Pseudo::ReadWrite => M::of(self.prop(vm, props, read_only)? != Value::TRUE),
                Pseudo::PlaceholderShown => {
                    let placeholder = self.prop(vm, props, placeholder)?;
                    let value = self.prop(vm, props, value_name)?;
                    // `(props.value ?? '') === ''`.
                    M::of(!placeholder.is_undefined() && (value.is_nullish() || vm.string_text(value).is_some_and(|text| text.is_empty())))
                }
                Pseudo::Empty => M::of(empty),
                Pseudo::Not(list) => {
                    let mut out = M::Yes;
                    for &selector in list {
                        match self.selector_matches(vm, selector, el, state, chain, page)? {
                            M::Yes => {
                                out = M::No;
                                break;
                            }
                            M::Ask => out = M::Ask,
                            M::No => {}
                        }
                    }
                    out
                }
                Pseudo::Is(list) => {
                    let mut out = M::No;
                    for &selector in list {
                        match self.selector_matches(vm, selector, el, state, chain, page)? {
                            M::Yes => {
                                out = M::Yes;
                                break;
                            }
                            M::Ask => out = M::Ask,
                            M::No => {}
                        }
                    }
                    out
                }
                Pseudo::FirstChild => M::of(placed && at[0] == 0.0),
                Pseudo::LastChild => M::of(placed && at[0] == at[1] - 1.0),
                Pseudo::OnlyChild => M::of(placed && at[1] == 1.0),
                Pseudo::FirstOfType => M::of(placed && at[2] == 0.0),
                Pseudo::LastOfType => M::of(placed && at[2] == at[3] - 1.0),
                Pseudo::OnlyOfType => M::of(placed && at[3] == 1.0),
                Pseudo::NthChild(arg) => M::of(placed && arg.matches(at[0] + 1.0)),
                Pseudo::NthLastChild(arg) => M::of(placed && arg.matches(at[1] - at[0])),
                Pseudo::NthOfType(arg) => M::of(placed && arg.matches(at[2] + 1.0)),
                Pseudo::NthLastOfType(arg) => M::of(placed && arg.matches(at[3] - at[2])),
                Pseudo::Link => M::of(!self.prop(vm, props, href)?.is_undefined()),
                Pseudo::Modal => M::of(self.prop(vm, props, modal)? == Value::TRUE),
                Pseudo::Open => M::of(self.prop(vm, props, open)? == Value::TRUE),
                Pseudo::Root => M::of(tag_is_html),
                Pseudo::Always(yes) => M::of(*yes),
            };
            match here {
                M::No => return Ok(M::No),
                M::Ask => result = M::Ask,
                M::Yes => {}
            }
        }
        Ok(result)
    }

    /// Whether a step could be about an element of this kind, by its tag, id and classes alone, and the same of what's last
    /// in at least one of each `:is()` and `:where()` list it has: what a plan's made for has the same of those (see
    /// `planOf`), so a step that couldn't be is about none of the elements the plan is for, whatever else they're given.
    fn could_be(&self, step: u32, el: &El, depth: u32) -> bool {
        let step = &self.steps[step as usize];
        if step.tag.as_ref().is_some_and(|tag| el.tag.as_deref() != Some(tag.as_str()))
            || step.id.as_ref().is_some_and(|id| el.id.as_deref() != Some(id.as_str()))
            || !step.classes.iter().all(|want| el.classes.contains(want))
        {
            return false;
        }
        if depth < 4 {
            for pseudo in &step.pseudo {
                if let Pseudo::Is(list) = pseudo
                    && !list.iter().any(|&selector| {
                        let last = *self.selectors[selector as usize].last().expect("a selector has steps");
                        self.could_be(last, el, depth + 1)
                    })
                {
                    return false;
                }
            }
        }
        true
    }

    /// `selectorMatches`: `matchChain` from the last step, with the page as it's given.
    fn selector_matches(&mut self, vm: &mut Vm, selector: u32, own: Value, state: u32, chain: Value, page: Value) -> Run<M> {
        let last = self.selectors[selector as usize].len() - 1;
        self.match_chain(vm, selector, last, own, Some(state), chain, page)
    }

    fn state_of(&self, el: Value, state: Option<u32>) -> u32 {
        state.unwrap_or_else(|| self.els[&el.bits()].state)
    }

    /// What a place in a chain is (`el`) and what's above it (`up`), with `el` read.
    fn up_of(&mut self, vm: &mut Vm, chain: Value) -> Run<(Value, Value)> {
        let (el_name, up_name) = (self.n().el, self.n().up);
        let el = vm.get_property(chain, el_name)?;
        let up = vm.get_property(chain, up_name)?;
        self.el(vm, el)?;
        Ok((el, if up.is_object() { up } else { Value::NULL }))
    }

    /// `matchChain`.
    #[allow(clippy::too_many_arguments)]
    fn match_chain(&mut self, vm: &mut Vm, selector: u32, at: usize, own: Value, state: Option<u32>, chain: Value, page: Value) -> Run<M> {
        let step = self.selectors[selector as usize][at];
        let state = self.state_of(own, state);
        let here = self.step_matches(vm, step, own, state, chain, page)?;
        if here == M::No || at == 0 {
            return Ok(here);
        }
        let above = match self.steps[step as usize].link {
            Link::Child => {
                if chain.is_object() {
                    let (el, up) = self.up_of(vm, chain)?;
                    self.match_chain(vm, selector, at - 1, el, None, up, page)?
                } else if page.is_object() {
                    self.el(vm, page)?;
                    self.match_chain(vm, selector, at - 1, page, None, Value::NULL, Value::NULL)?
                } else {
                    M::No
                }
            }
            Link::Descendant => self.matched_above(vm, selector, at - 1, chain, page, false)?,
            Link::Adjacent | Link::Sibling => self.before(vm, selector, at - 1, own, chain, page, false)?,
        };
        Ok(here.and(above))
    }

    /// Whether step `at` is about what's before `own` among what they're in (the one just before for `+`, any for `~`), with the
    /// same chain above, as `matchChain` (or `matchFrom`, with `from`) has it.
    #[allow(clippy::too_many_arguments)]
    fn before(&mut self, vm: &mut Vm, selector: u32, at: usize, own: Value, chain: Value, page: Value, from: bool) -> Run<M> {
        let prevs = self.els[&own.bits()].prevs;
        let Some(prevs) = vm.array_elements(prevs).map(<[Value]>::to_vec) else {
            return Ok(M::No);
        };
        let link = self.steps[self.selectors[selector as usize][at + 1] as usize].link;
        let which: &[Value] = if link == Link::Adjacent { prevs.last().map(std::slice::from_ref).unwrap_or(&[]) } else { &prevs };
        let mut out = M::No;
        for &one in which {
            self.el(vm, one)?;
            let found = if from {
                self.match_from(vm, selector, at, one, None, chain, page)?
            } else {
                self.match_chain(vm, selector, at, one, None, chain, page)?
            };
            match found {
                M::Yes => return Ok(M::Yes),
                M::Ask => out = M::Ask,
                M::No => {}
            }
        }
        Ok(out)
    }

    /// `matchFrom`, which goes no further than the rule's second step, as its first is what it belongs to.
    #[allow(clippy::too_many_arguments)]
    fn match_from(&mut self, vm: &mut Vm, selector: u32, at: usize, own: Value, state: Option<u32>, chain: Value, page: Value) -> Run<M> {
        let step = self.selectors[selector as usize][at];
        let state = self.state_of(own, state);
        let here = self.step_matches(vm, step, own, state, chain, page)?;
        if here == M::No {
            return Ok(M::No);
        }
        let link = self.steps[step as usize].link;
        if at == 1 {
            // What's before it is what the rule belongs to, so it's about it as a descendant, or as a child at the top.
            return Ok(here.and(M::of(link == Link::Descendant || (link == Link::Child && !chain.is_object()))));
        }
        if at == 0 {
            return Ok(here);
        }
        let above = match link {
            Link::Child => {
                if chain.is_object() {
                    let (el, up) = self.up_of(vm, chain)?;
                    self.match_from(vm, selector, at - 1, el, None, up, page)?
                } else {
                    M::No
                }
            }
            Link::Descendant => self.matched_above(vm, selector, at - 1, chain, page, true)?,
            Link::Adjacent | Link::Sibling => self.before(vm, selector, at - 1, own, chain, page, true)?,
        };
        Ok(here.and(above))
    }

    /// The number a place in a chain is remembered by, kept on it.
    fn node(&mut self, vm: &mut Vm, chain: Value) -> Run<u32> {
        let name = self.n().node;
        let found = vm.get_property(chain, name)?;
        if found.is_number() {
            return Ok(found.as_number() as u32);
        }
        self.next_node += 1;
        vm.set_property(chain, name, Value::number(f64::from(self.next_node)))?;
        Ok(self.next_node)
    }

    /// `matchedAbove`: whether step `at` ends on anything in `chain`, remembered for each place in it, going up it in a loop
    /// rather than deeper on the stack for each place.
    fn matched_above(&mut self, vm: &mut Vm, selector: u32, at: usize, mut chain: Value, page: Value, from: bool) -> Run<M> {
        let flags = u8::from(page.is_object()) | (u8::from(from) << 1);
        let mut passed: Vec<u32> = Vec::new();
        let result = loop {
            if !chain.is_object() {
                break if !from && page.is_object() {
                    self.el(vm, page)?;
                    self.match_chain(vm, selector, at, page, None, Value::NULL, Value::NULL)?
                } else {
                    M::No
                };
            }
            let node = self.node(vm, chain)?;
            if let Some(&found) = self.memo.get(&(node, selector, at as u16, flags)) {
                break M::of(found);
            }
            let (el, up) = self.up_of(vm, chain)?;
            let here = if from {
                self.match_from(vm, selector, at, el, None, up, page)?
            } else {
                self.match_chain(vm, selector, at, el, None, up, page)?
            };
            passed.push(node);
            match here {
                M::Yes => break M::Yes,
                M::Ask => break M::Ask,
                M::No => chain = up,
            }
        };
        if result != M::Ask {
            for node in passed {
                self.memo.insert((node, selector, at as u16, flags), result == M::Yes);
            }
        }
        Ok(result)
    }
}

/// `cssSelector(steps)`: the number a rule's selector is kept by here, or -1 if it's one web.js matches itself.
fn host_selector(vm: &mut Vm, args: Args) -> Run<Value> {
    let steps = args.get(vm, 0);
    STORE.with(|store| {
        let mut store = store.borrow_mut();
        store.names(vm);
        let found = store.read_selector(vm, steps, true)?;
        Ok(Value::number(found.map_or(-1.0, f64::from)))
    })
}

/// `cssMatchItems(count, given, own, chain, page, version, flow, out)`: for each of the first `count` rules `given` (a
/// `Float64Array`) names, two numbers each, its selector (`cssSelector`, or less than 0 for one to skip) and which states it's
/// about (`needsOf`), whether it's about `own`, with `chain` above it and `page` beyond that, as `ruleMatches` and `stateNeeds`
/// say: written into `out` (a `Uint8Array`) at its place, 0 not, 1 it is, 2 it isn't but would be in some state it's about, 3
/// for web.js to work out. `version` and `flow` are web.js's, for as long as which what's above has been found for stays.
/// It gives back which states all but those for web.js care about, put together, as `ruleOnSaid` would give each, so that
/// web.js only goes through the rules that are about the element, and those it's to work out.
fn host_match_items(vm: &mut Vm, args: Args) -> Run<Value> {
    let (count, given, own, chain) = (args.get(vm, 0), args.get(vm, 1), args.get(vm, 2), args.get(vm, 3));
    let (page, version, flow, out) = (args.get(vm, 4), args.get(vm, 5), args.get(vm, 6), args.get(vm, 7));
    let count = if count.is_number() { count.as_number().max(0.0) as usize } else { 0 };
    let wanted: Vec<(f64, u32)> = {
        let bytes = vm.bytes(given).unwrap_or(&[]);
        let number = |at: usize| bytes.get(at * 8..at * 8 + 8).map_or(f64::NAN, |chunk| f64::from_le_bytes(chunk.try_into().unwrap()));
        (0..count.min(bytes.len() / 16)).map(|at| (number(at * 2), number(at * 2 + 1).max(0.0) as u32)).collect()
    };
    let chain = if chain.is_object() { chain } else { Value::NULL };
    let page = if page.is_object() { page } else { Value::NULL };
    let mut needs_all = 0u32;
    let codes = STORE.with(|store| -> Run<Vec<u8>> {
        let mut store = store.borrow_mut();
        store.names(vm);
        let epoch = (if version.is_number() { version.as_number() } else { f64::NAN }, if flow.is_number() { flow.as_number() } else { f64::NAN });
        if store.epoch != epoch || store.memo.len() > 500_000 {
            store.memo.clear();
            store.epoch = epoch;
        }
        store.els.clear();
        store.el(vm, own)?;
        let own_state = store.els[&own.bits()].state;
        let mut codes = Vec::with_capacity(wanted.len());
        for (selector, needs) in wanted {
            if selector == COULD_NOT_BE {
                needs_all |= said_needs(NO, needs, own_state);
                codes.push(NO);
                continue;
            }
            if !(selector >= 0.0) || selector as usize >= store.selectors.len() {
                codes.push(ASK);
                continue;
            }
            let selector = selector as u32;
            let last = store.selectors[selector as usize].len() - 1;
            if last == 0 {
                codes.push(ASK);
                continue;
            }
            let code = match store.match_from(vm, selector, last, own, Some(own_state), chain, page)? {
                M::Yes => YES,
                M::Ask => ASK,
                // As `stateNeeds`: only what a state it's about, and isn't in, would change is looked at again.
                M::No if needs == 0 || own_state & needs != 0 => NO,
                M::No => match store.match_from(vm, selector, last, own, Some(ALL_STATES), chain, page)? {
                    M::Yes => SOME_STATE,
                    M::Ask => ASK,
                    M::No => NO,
                },
            };
            needs_all |= said_needs(code, needs, own_state);
            codes.push(code);
        }
        store.els.clear();
        Ok(codes)
    })?;
    if let Some(bytes) = vm.bytes_mut(out) {
        for (code, place) in codes.iter().zip(bytes.iter_mut()) {
            *place = *code;
        }
    }
    Ok(Value::number(f64::from(needs_all)))
}

/// Which states a rule Rust has said of cares about, for an element in `state`, as `ruleOnSaid` gives it: all it's about if it's
/// about the element, or would be in some state, or if it's in a state it's about; none for one web.js is to work out.
fn said_needs(code: u8, needs: u32, state: u32) -> u32 {
    match code {
        YES | SOME_STATE => needs,
        NO => needs & if state & needs != 0 { u32::MAX } else { 0 },
        _ => 0,
    }
}

/// What `cssMatchPlan` puts in place of a selector that couldn't be about an element a plan is for, which `cssMatchItems` says
/// no to without looking.
const COULD_NOT_BE: f64 = -3.0;

/// `cssMatchPlan(count, given, own)`: once for each plan, puts `COULD_NOT_BE` in `given` (as `cssMatchItems` is given it) in
/// place of each selector whose last step couldn't be about `own` by its tag, id and classes (`Store::could_be`), which every
/// element the plan is for has the same of. Mantine's typography is `.m_d08caa0 :where(h1, h2, …)`, found for every element by
/// its `*`, and the like are most of the sixty rules about more than the element itself each of its elements had to look at.
fn host_match_plan(vm: &mut Vm, args: Args) -> Run<Value> {
    let (count, given, own) = (args.get(vm, 0), args.get(vm, 1), args.get(vm, 2));
    let count = if count.is_number() { count.as_number().max(0.0) as usize } else { 0 };
    let skip: Vec<usize> = STORE.with(|store| -> Run<Vec<usize>> {
        let mut store = store.borrow_mut();
        store.names(vm);
        store.els.clear();
        store.el(vm, own)?;
        let bytes = vm.bytes(given).unwrap_or(&[]);
        let number = |at: usize| bytes.get(at * 8..at * 8 + 8).map_or(f64::NAN, |chunk| f64::from_le_bytes(chunk.try_into().unwrap()));
        let el = &store.els[&own.bits()];
        let mut skip = Vec::new();
        for slot in 0..count.min(bytes.len() / 16) {
            let selector = number(slot * 2);
            if !(selector >= 0.0) || selector as usize >= store.selectors.len() {
                continue;
            }
            let steps = &store.selectors[selector as usize];
            if steps.len() > 1 && !store.could_be(*steps.last().expect("a selector has steps"), el, 0) {
                skip.push(slot);
            }
        }
        store.els.clear();
        Ok(skip)
    })?;
    if let Some(bytes) = vm.bytes_mut(given) {
        for slot in skip {
            bytes[slot * 16..slot * 16 + 8].copy_from_slice(&COULD_NOT_BE.to_le_bytes());
        }
    }
    Ok(Value::UNDEFINED)
}

pub(crate) const FUNCTIONS: [(&str, HostFunction); 3] =
    [("cssSelector", host_selector), ("cssMatchItems", host_match_items), ("cssMatchPlan", host_match_plan)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_as_javascript_splits_them() {
        assert!(has_word("a b", "b"));
        assert!(has_word("  a", ""));
        assert!(has_word("a  ", ""));
        assert!(!has_word("ab", "a"));
        assert!(has_word("x\ty", "x"));
    }

    #[test]
    fn an_plus_b_reads_as_nth_matches_reads_it() {
        assert!(Nth::read("2n+1").matches(3.0));
        assert!(!Nth::read("2n+1").matches(2.0));
        assert!(Nth::read("odd").matches(1.0));
        assert!(Nth::read("-n+3").matches(3.0));
        assert!(!Nth::read("-n+3").matches(4.0));
        assert!(Nth::read("3").matches(3.0));
        assert!(Nth::read(" 2n ").matches(4.0));
    }
}
