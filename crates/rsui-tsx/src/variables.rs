//! What an element's own variables come to, as `withElementVariables` in web.js works them out: what the rules on it set, as
//! `.m_77c9d27d { --button-bg: var(--mantine-primary-color-filled) }`, is taken out of its look and worked out with what's set
//! around it and on the page, what in its look waits for one of them (`__var name`) is declared again with them put in, and
//! what goes down to what's inside it is given back as a `Map`, as web.js gives it. On Mantine's page an element has about ten
//! of its own and five that wait for one, and this took 60-90 µs of an element's 300 in web.js.
//!
//! It reads and writes the look, the `Map`s and what's noted in the same order web.js does, so what comes out is the same object
//! with the same props in the same order. What's asked of the page is asked of web.js (`variableValue`, which notes what read
//! it), as are the props a property sets (`propertyKeys`), and what `sameValue` says of two objects JSON can't be sure of.

use std::cell::RefCell;
use std::collections::HashMap as StdHashMap;

use rsui_js::{Args, Atom, HostFunction, Root, Value, Vm};

type Run<T> = Result<T, Value>;
type HashMap<K, V> = StdHashMap<K, V, foldhash::fast::FixedState>;

/// How many declarations with their variables put in are kept, as web.js keeps as many.
const KEPT: usize = 5000;

/// The names read from what web.js gives, made once for each engine.
struct Names {
    vars: Atom,
    important: Atom,
    transition_parts: Atom,
    animation_parts: Atom,
    value: Atom,
    font_size: Atom,
    made: Atom,
    over: Atom,
    keys: Atom,
    names: Atom,
    own: Atom,
    inherits: Atom,
    page_value: Atom,
    property_keys: Atom,
    same_value: Atom,
    declare: Atom,
    declaring: Atom,
    registered: Atom,
    own_variables: Atom,
}

impl Names {
    fn new(vm: &mut Vm) -> Names {
        Names {
            vars: vm.atom("__vars"),
            important: vm.atom("__important"),
            transition_parts: vm.atom("__transitionParts"),
            animation_parts: vm.atom("__animationParts"),
            value: vm.atom("value"),
            font_size: vm.atom("fontSize"),
            made: vm.atom("made"),
            over: vm.atom("over"),
            keys: vm.atom("keys"),
            names: vm.atom("names"),
            own: vm.atom("own"),
            inherits: vm.atom("inherits"),
            page_value: vm.atom("pageValue"),
            property_keys: vm.atom("propertyKeys"),
            same_value: vm.atom("sameValue"),
            declare: vm.atom("declare"),
            declaring: vm.atom("declaring"),
            registered: vm.atom("registered"),
            own_variables: vm.atom("ownVariables"),
        }
    }
}

#[derive(Default)]
struct Store {
    names: Option<Names>,
    /// What each of a look's keys is, by its atom: a variable (`--name`), what waits for one (`__var name`, as `name`), or
    /// neither. Kept while atoms are the same, as `cssVariableKeys` keeps them.
    kinds: HashMap<Atom, Key>,
    generation: u32,
    /// The props each property sets, as `propertyKeys` says, which it says the same for always.
    property_keys: HashMap<String, Vec<Atom>>,
    /// Atoms made for names, which last as long as the engine.
    atoms: HashMap<String, Atom>,
    /// What a declaration came to with its variables put in, by its name, text, text size and rem, as web.js's
    /// `resolvedDeclarations` keeps it, and those in the window's units apart, which go once it's another size.
    declared: HashMap<String, Root>,
    declared_viewport: HashMap<String, Root>,
}

#[derive(Clone)]
enum Key {
    Variable(String),
    Waits(String),
    Neither,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store::default());
}

/// Forgets everything, for an engine that starts on a thread another has been on.
pub(crate) fn reset() {
    STORE.with(|store| *store.borrow_mut() = Store::default());
}

/// The spaces `trim()` takes off in JavaScript.
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{2028}' | '\u{2029}'
            | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

fn js_trim(text: &str) -> &str {
    text.trim_matches(js_space)
}

/// `\w`, as a JavaScript regular expression without `u` has it.
fn word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `RUNS_ON` in web.js: `/[\w.%#)-]/`.
fn runs_on(c: char) -> bool {
    word(c) || matches!(c, '.' | '%' | '#' | ')' | '-')
}

/// `apart` in web.js: what a `var()` comes to, with a space before it where it would run into what's before, and after it where
/// it would run into what's next.
fn apart(before: &str, value: &str, next: Option<char>) -> String {
    if value.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(value.len() + 2);
    if before.chars().next_back().is_some_and(runs_on) && value.chars().next().is_some_and(runs_on) {
        out.push(' ');
    }
    out.push_str(value);
    if next.is_some_and(|c| word(c) || matches!(c, '.' | '#' | '-')) && value.chars().next_back().is_some_and(runs_on) {
        out.push(' ');
    }
    out
}

/// `insideParens(text, open)`: what's inside the parentheses that open at `open`, and where they end.
fn inside_parens(text: &str, open: usize) -> (&str, usize) {
    let mut depth = 0i32;
    for (at, byte) in text.bytes().enumerate().skip(open) {
        if byte == b'(' {
            depth += 1;
        } else if byte == b')' {
            depth -= 1;
            if depth == 0 {
                return (&text[open + 1..at], at + 1);
            }
        }
    }
    (&text[(open + 1).min(text.len())..], text.len())
}

/// `namesIn(text)`: the variables `var(--name)` names, as `/var\(\s*(--(?:[\w-]|\\.)+)/g` finds them.
fn names_in(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..].find("var(") {
        let mut at = from + found + 4;
        let rest = &text[at..];
        let spaces = rest.len() - rest.trim_start_matches(js_space).len();
        at += spaces;
        from = from + found + 4;
        if !text[at..].starts_with("--") {
            continue;
        }
        let start = at;
        let mut end = at + 2;
        let mut chars = text[end..].char_indices().peekable();
        let mut length = 0;
        while let Some((place, c)) = chars.next() {
            if word(c) || c == '-' {
                length = place + c.len_utf8();
            } else if c == '\\' {
                match chars.next() {
                    Some((after, next)) if !matches!(next, '\n' | '\r' | '\u{2028}' | '\u{2029}') => length = after + next.len_utf8(),
                    _ => break,
                }
            } else {
                break;
            }
        }
        if length == 0 {
            continue;
        }
        end += length;
        out.push(&text[start..end]);
        from = end;
    }
    out
}

/// `usesViewportUnits(text)`: `/(?:\d|\.)\s*[sld]?v(?:w|h|i|b|min|max)\b/i`.
fn uses_viewport_units(text: &str) -> bool {
    let bytes = text.as_bytes();
    for (at, &byte) in bytes.iter().enumerate() {
        if !(byte.is_ascii_digit() || byte == b'.') {
            continue;
        }
        // Spaces as `\s` has them, which are only ever ASCII ones in CSS that rsui reads here, and any other.
        let rest = &text[at + 1..];
        let rest = rest.trim_start_matches(js_space);
        let lower = rest.as_bytes();
        let mut place = 0;
        if lower.first().is_some_and(|b| matches!(b.to_ascii_lowercase(), b's' | b'l' | b'd'))
            && lower.get(1).is_some_and(|b| b.eq_ignore_ascii_case(&b'v'))
        {
            place = 1;
        }
        if !lower.get(place).is_some_and(|b| b.eq_ignore_ascii_case(&b'v')) {
            continue;
        }
        let after = &lower[place + 1..];
        let unit = |name: &[u8]| after.len() >= name.len() && after[..name.len()].eq_ignore_ascii_case(name);
        for name in [&b"min"[..], b"max", b"w", b"h", b"i", b"b"] {
            if unit(name) {
                let next = after.get(name.len()).copied();
                if !next.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_') {
                    return true;
                }
            }
        }
    }
    false
}

/// `===`, for what's in a look.
fn strictly_same(vm: &Vm, a: Value, b: Value) -> bool {
    if a.is_number() && b.is_number() {
        return a.as_number() == b.as_number();
    }
    a == b || vm.same(a, b)
}

/// Whether `JSON.stringify` says the same of both, as far as that can be told here: `None` for what has to be asked of web.js.
fn json_same(vm: &mut Vm, a: Value, b: Value, depth: u32) -> Option<bool> {
    if depth > 16 {
        return None;
    }
    if a.is_number() && b.is_number() {
        let (x, y) = (a.as_number(), b.as_number());
        return Some((x.is_finite() && y.is_finite() && x == y) || (!x.is_finite() && !y.is_finite()));
    }
    if a.is_string() || b.is_string() {
        return Some(a.is_string() && b.is_string() && vm.same(a, b));
    }
    if a.is_object() && b.is_object() {
        if vm.is_function(a) || vm.is_function(b) {
            return None;
        }
        if let (Some(x), Some(y)) = (vm.array_elements(a).map(<[Value]>::to_vec), vm.array_elements(b).map(<[Value]>::to_vec)) {
            if x.len() != y.len() {
                return Some(false);
            }
            for (one, other) in x.into_iter().zip(y) {
                // In an array, what JSON leaves out is `null`.
                let (one, other) = (array_json(vm, one), array_json(vm, other));
                if !json_same(vm, one, other, depth + 1)? {
                    return Some(false);
                }
            }
            return Some(true);
        }
        if !vm.is_plain_object(a) || !vm.is_plain_object(b) {
            return None;
        }
        let shown = |vm: &mut Vm, object: Value| -> Option<Vec<(Atom, Value)>> {
            let found = vm.properties(object).ok()?;
            let mut out = Vec::with_capacity(found.len());
            for (key, value) in found {
                let name = vm.atom_name(key);
                if name.as_bytes().first().is_some_and(u8::is_ascii_digit) || name == "toJSON" {
                    return None;
                }
                if value.is_undefined() || vm.is_function(value) || value.is_symbol() {
                    continue;
                }
                out.push((key, value));
            }
            Some(out)
        };
        let (x, y) = (shown(vm, a)?, shown(vm, b)?);
        if x.len() != y.len() {
            return Some(false);
        }
        for ((key, one), (other_key, other)) in x.into_iter().zip(y) {
            if key != other_key || !json_same(vm, one, other, depth + 1)? {
                return Some(false);
            }
        }
        return Some(true);
    }
    if a.is_object() || b.is_object() || a.is_symbol() || b.is_symbol() || a.is_bigint() || b.is_bigint() {
        return None;
    }
    // `true`, `false`, `null`, and what JSON leaves out altogether (`undefined`), which is only the same as itself.
    Some(a == b)
}

fn array_json(vm: &Vm, value: Value) -> Value {
    if value.is_undefined() || value.is_symbol() || vm.is_function(value) { Value::NULL } else { value }
}

/// What the look's props and its variables were, for one element, while they're worked out.
struct Work<'a> {
    names: &'a Names,
    /// The element's own variables, in the order its look had them.
    own: Vec<(String, Value)>,
    inherited: Value,
    page_value: Value,
    /// What the page said of each variable it was asked about, once for each.
    page: Vec<(String, Value)>,
    /// Each variable looked for, in order, for `note.names`.
    looked: Vec<String>,
    noting: bool,
}

/// What a variable is, as `setTo` in web.js has it: nothing where it isn't set, or is `initial`.
enum Found {
    Nothing,
    Text(String),
}

impl Work<'_> {
    /// `local(name)` in web.js: what the element or what's around it sets.
    fn local(&mut self, vm: &mut Vm, name: &str) -> Value {
        if self.noting {
            self.looked.push(name.to_owned());
        }
        if let Some((_, value)) = self.own.iter().find(|(own, _)| own == name) {
            return *value;
        }
        if self.inherited.is_nullish() {
            return Value::UNDEFINED;
        }
        vm.map_text_value(self.inherited, name).unwrap_or(Value::UNDEFINED)
    }

    /// `variableValue(name)`, asked of web.js once for each name.
    fn page(&mut self, vm: &mut Vm, name: &str) -> Run<Value> {
        if let Some((_, value)) = self.page.iter().find(|(said, _)| said == name) {
            return Ok(*value);
        }
        let key = vm.new_string(name);
        let value = vm.call_function(self.page_value, Value::UNDEFINED, &[key])?;
        vm.hold(value);
        self.page.push((name.to_owned(), value));
        Ok(value)
    }

    /// `setTo(local(name) ?? variableValue(name))`.
    fn found(&mut self, vm: &mut Vm, name: &str) -> Run<Found> {
        let mut value = self.local(vm, name);
        if value.is_nullish() {
            value = self.page(vm, name)?;
        }
        if value.is_nullish() {
            return Ok(Found::Nothing);
        }
        let Some(text) = vm.string_text(value).map(|text| text.into_owned()) else {
            return Err(vm.new_string("rsui: a variable that isn't text was put into a declaration"));
        };
        if text.encode_utf16().count() < 16 && js_trim(&text).to_lowercase() == "initial" {
            return Ok(Found::Nothing);
        }
        Ok(Found::Text(text))
    }

    /// `resolve(text, depth)` in web.js: `text` with what each `var()` in it stands for put in.
    fn resolve(&mut self, vm: &mut Vm, text: &str, depth: u32) -> Run<String> {
        let Some(first) = text.find("var(") else {
            return Ok(text.to_owned());
        };
        if depth > 16 {
            return Ok(text.to_owned());
        }
        let mut out = String::with_capacity(text.len());
        let mut from = 0;
        let mut at = Some(first);
        while let Some(start) = at {
            out.push_str(&text[from..start]);
            let (inside, end) = inside_parens(text, start + 3);
            let comma = inside.find(',');
            let name = js_trim(comma.map_or(inside, |comma| &inside[..comma]));
            let piece = match self.found(vm, name)? {
                Found::Text(found) => self.resolve(vm, &found, depth + 1)?,
                Found::Nothing => match comma {
                    None => String::new(),
                    Some(comma) => {
                        let otherwise = js_trim(&inside[comma + 1..]).to_owned();
                        self.resolve(vm, &otherwise, depth + 1)?
                    }
                },
            };
            let next = text[end..].chars().next();
            let put = apart(&out, &piece, next);
            out.push_str(&put);
            from = end;
            at = text[from..].find("var(").map(|found| found + from);
        }
        out.push_str(&text[from..]);
        Ok(out)
    }
}

fn atom_of(store: &mut Store, vm: &mut Vm, name: &str) -> Atom {
    if let Some(&atom) = store.atoms.get(name) {
        return atom;
    }
    let atom = vm.atom(name);
    store.atoms.insert(name.to_owned(), atom);
    atom
}

/// The atoms of what an array of names holds, or `None` if it isn't one.
fn atoms_in(vm: &mut Vm, list: Value) -> Run<Option<Vec<Atom>>> {
    let Some(items) = vm.array_elements(list).map(<[Value]>::to_vec) else {
        return Ok(None);
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        out.push(vm.property_atom(item)?);
    }
    Ok(Some(out))
}

/// `said.keys ??= varKeys(name, said.made)`: the props a declaration that waits for a variable sets, what it made with only the
/// page's and what the property sets.
fn var_keys(store: &mut Store, n: &Names, vm: &mut Vm, ctx: Value, said: Value, name: &str, made: &[(Atom, Value)]) -> Run<Vec<Atom>> {
    let (keys_atom, property_keys_atom) = (n.keys, n.property_keys);
    let kept = vm.get_property(said, keys_atom)?;
    if !kept.is_nullish() {
        if let Some(found) = atoms_in(vm, kept)? {
            return Ok(found);
        }
    }
    let mut keys: Vec<Atom> = Vec::new();
    for &(key, _) in made {
        if !vm.atom_name(key).starts_with("__") && !keys.contains(&key) {
            keys.push(key);
        }
    }
    let property = match store.property_keys.get(name) {
        Some(found) => found.clone(),
        None => {
            let function = vm.get_property(ctx, property_keys_atom)?;
            let text = vm.new_string(name);
            let list = vm.call_function(function, Value::UNDEFINED, &[text])?;
            let mut found = Vec::new();
            for item in vm.array_elements(list).map(<[Value]>::to_vec).unwrap_or_default() {
                if let Some(text) = vm.string_text(item).map(|text| text.into_owned()) {
                    found.push(atom_of(store, vm, &text));
                }
            }
            store.property_keys.insert(name.to_owned(), found.clone());
            found
        }
    };
    for key in property {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let texts: Vec<Value> = keys.iter().map(|&key| {
        let name = vm.atom_name(key);
        vm.new_string(&name)
    }).collect();
    let list = vm.new_array(texts);
    vm.set_property(said, keys_atom, list)?;
    Ok(keys)
}

/// `sameValue(a, b)` in web.js: the same, or objects JSON says the same of.
fn same_value(vm: &mut Vm, ctx: Value, same_atom: Atom, a: Value, b: Value) -> Run<bool> {
    if strictly_same(vm, a, b) {
        return Ok(true);
    }
    if !a.is_object() {
        return Ok(false);
    }
    if let Some(same) = json_same(vm, a, b, 0) {
        return Ok(same);
    }
    let function = vm.get_property(ctx, same_atom)?;
    let said = vm.call_function(function, Value::UNDEFINED, &[a, b])?;
    Ok(said == Value::TRUE)
}

/// `cssElementVariables(look, inherited, fresh, note, remSize, ctx)`: what `withElementVariables` does, for a look whose
/// `__vars` is `true`: gives back what goes down to what's inside, as a `Map`, or `null` where the element sets no variables of
/// its own, or `false` where it's left to web.js, as it is for what's inherited that isn't a `Map`. `ctx` has the page's
/// `variableValue` as `pageValue`, `propertyKeys`, `sameValue`, `cssDeclare` as `declare` and `declaring`, `registered`, and
/// `OWN_VARIABLES` as `ownVariables`.
fn host_element_variables(vm: &mut Vm, args: Args) -> Run<Value> {
    let (look, inherited, fresh, note, rem, ctx) =
        (args.get(vm, 0), args.get(vm, 1), args.get(vm, 2) == Value::TRUE, args.get(vm, 3), args.get(vm, 4), args.get(vm, 5));
    if !look.is_object() || !ctx.is_object() || !(inherited.is_nullish() || vm.is_map(inherited)) {
        return Ok(Value::FALSE);
    }
    let mut store = STORE.with(|store| std::mem::take(&mut *store.borrow_mut()));
    let result = element_variables(&mut store, vm, look, inherited, fresh, note, rem, ctx);
    STORE.with(|kept| *kept.borrow_mut() = store);
    result
}

#[allow(clippy::too_many_arguments)]
fn element_variables(store: &mut Store, vm: &mut Vm, look: Value, inherited: Value, fresh: bool, note: Value, rem: Value, ctx: Value) -> Run<Value> {
    if store.names.is_none() {
        store.names = Some(Names::new(vm));
    }
    let names = store.names.take().expect("just made");
    let result = element_variables_with(store, &names, vm, look, inherited, fresh, note, rem, ctx);
    store.names = Some(names);
    result
}

#[allow(clippy::too_many_arguments)]
fn element_variables_with(
    store: &mut Store,
    n: &Names,
    vm: &mut Vm,
    look: Value,
    inherited: Value,
    fresh: bool,
    note: Value,
    rem: Value,
    ctx: Value,
) -> Run<Value> {
    if vm.get_property(look, n.vars)? != Value::TRUE {
        return Ok(Value::NULL);
    }
    vm.delete_property(look, n.vars)?;
    if store.generation != vm.atom_generation() || store.kinds.len() > 4096 {
        store.generation = vm.atom_generation();
        store.kinds.clear();
    }
    // What it sets itself, and what waits for a variable, taken out of its look in the order it has them.
    let mut own: Vec<(String, Value)> = Vec::new();
    let mut used: Vec<(String, Value)> = Vec::new();
    for (key, value) in vm.properties(look)? {
        let kind = match store.kinds.get(&key) {
            Some(kind) => kind.clone(),
            None => {
                let name = vm.atom_name(key);
                let kind = if name.starts_with("--") {
                    Key::Variable(name)
                } else if let Some(rest) = name.strip_prefix("__var ") {
                    Key::Waits(rest.to_owned())
                } else {
                    Key::Neither
                };
                store.kinds.insert(key, kind.clone());
                kind
            }
        };
        match kind {
            Key::Variable(name) => {
                vm.hold(value);
                if let Some(place) = own.iter_mut().find(|(said, _)| *said == name) {
                    place.1 = value;
                } else {
                    own.push((name, value));
                }
                vm.delete_property(look, key)?;
            }
            Key::Waits(name) => {
                if !value.is_nullish() {
                    vm.hold(value);
                    used.push((name, value));
                }
                vm.delete_property(look, key)?;
            }
            Key::Neither => {}
        }
    }
    let page_value = vm.get_property(ctx, n.page_value)?;
    let noting = note.is_object();
    let mut work = Work { names: n, own, inherited, page_value, page: Vec::new(), looked: Vec::new(), noting };
    let result = with_variables(store, &mut work, vm, look, fresh, note, rem, ctx, used);
    // What was looked for goes in `note.names`, as web.js pushes it as it looks.
    if noting && !work.looked.is_empty() {
        let list = vm.get_property(note, work.names.names)?;
        for name in std::mem::take(&mut work.looked) {
            let text = vm.new_string(&name);
            vm.push_item(list, text)?;
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn with_variables(
    store: &mut Store,
    work: &mut Work,
    vm: &mut Vm,
    look: Value,
    fresh: bool,
    note: Value,
    rem: Value,
    ctx: Value,
    used: Vec<(String, Value)>,
) -> Run<Value> {
    let n = work.names;
    for (name, said) in used {
        // What's `!important` isn't gone over by what isn't.
        let said_important = vm.get_property(said, n.important)? == Value::TRUE;
        let look_important = vm.get_property(look, n.important)?;
        let made_value = vm.get_property(said, n.made)?;
        let made = vm.properties(made_value)?;
        if !said_important && !look_important.is_undefined() {
            let keys = var_keys(store, n, vm, ctx, said, &name, &made)?;
            let important = atoms_in(vm, look_important)?.unwrap_or_default();
            if !keys.is_empty() && keys.iter().all(|key| important.contains(key)) {
                continue;
            }
        }
        // Only what an element's own variables change is worked out again.
        let value = vm.get_property(said, n.value)?;
        let Some(text) = vm.string_text(value).map(|text| text.into_owned()) else {
            return Err(vm.new_string("rsui: what waits for a variable isn't text"));
        };
        let mut any = false;
        for one in names_in(&text) {
            if !work.local(vm, one).is_undefined() {
                any = true;
                break;
            }
        }
        if !any {
            continue;
        }
        // What's been said again after it, or isn't what it made any more, stays as that says.
        let over_said = vm.get_property(said, n.over)?;
        let mut over: Vec<Atom> = atoms_in(vm, over_said)?.unwrap_or_default();
        let name_text = vm.new_string(&name);
        for &(key, made_one) in &made {
            if made_one.is_undefined() {
                continue;
            }
            let now = vm.get_property(look, key)?;
            let unchanged = if key == n.transition_parts || key == n.animation_parts {
                let name_atom = vm.property_atom(name_text)?;
                let mine = if now.is_nullish() { Value::UNDEFINED } else { vm.get_property(now, name_atom)? };
                let theirs = if made_one.is_nullish() { Value::UNDEFINED } else { vm.get_property(made_one, name_atom)? };
                strictly_same(vm, mine, theirs)
            } else {
                same_value(vm, ctx, n.same_value, now, made_one)?
            };
            if !unchanged && !(fresh && now.is_undefined()) {
                over.push(key);
            }
        }
        if !over.is_empty() {
            let keys = var_keys(store, n, vm, ctx, said, &name, &made)?;
            if keys.iter().all(|key| over.contains(key)) {
                continue;
            }
        }
        // What it comes to, with the variables put in, is the same for every element where they come to the same.
        let resolved = work.resolve(vm, &text, 0)?;
        let font_size = vm.get_property(said, n.font_size)?;
        let key = format!(
            "{name}\u{0}{resolved}\u{0}{}\u{0}{}",
            if font_size.is_number() { font_size.as_number().to_bits().to_string() } else { vm.plain_text(font_size).unwrap_or_default() },
            if rem.is_number() { rem.as_number().to_bits() } else { 0 }
        );
        let viewport = uses_viewport_units(&resolved);
        let kept = if viewport { store.declared_viewport.get(&key) } else { store.declared.get(&key) }.map(Root::get);
        let props = match kept {
            Some(props) => props,
            None => {
                let declare = vm.get_property(ctx, n.declare)?;
                let declaring = vm.get_property(ctx, n.declaring)?;
                let pair = {
                    let text = vm.new_string(&resolved);
                    vm.new_array(vec![name_text, text])
                };
                let list = vm.new_array(vec![pair]);
                let props = vm.call_function(declare, Value::UNDEFINED, &[list, font_size, rem, declaring])?;
                let root = vm.root(props);
                let cache = if viewport { &mut store.declared_viewport } else { &mut store.declared };
                if cache.len() > KEPT {
                    cache.clear();
                }
                cache.insert(key, root);
                props
            }
        };
        let transition_parts = vm.get_property(look, n.transition_parts)?;
        let animation_parts = vm.get_property(look, n.animation_parts)?;
        vm.hold(transition_parts);
        vm.hold(animation_parts);
        for &(key, _) in &made {
            if !over.contains(&key) {
                vm.delete_property(look, key)?;
            }
        }
        let said_props = vm.properties(props)?;
        for &(key, value) in &said_props {
            if over.is_empty() || !over.contains(&key) {
                vm.set_property(look, key, value)?;
            }
        }
        for (before, atom) in [(transition_parts, n.transition_parts), (animation_parts, n.animation_parts)] {
            if before.is_undefined() {
                continue;
            }
            let said_parts = vm.get_property(props, atom)?;
            let now = vm.get_property(look, atom)?;
            if said_parts.is_undefined() || !strictly_same(vm, now, said_parts) {
                continue;
            }
            let merged = vm.new_object();
            for (key, value) in vm.properties(before)?.into_iter().chain(vm.properties(said_parts)?) {
                vm.set_property(merged, key, value)?;
            }
            vm.set_property(look, atom, merged)?;
        }
    }
    if work.own.is_empty() {
        return Ok(Value::NULL);
    }
    // What goes down to what's inside, as it is here, which is all of them apart from what `@property` says doesn't.
    let registered = vm.get_property(ctx, n.registered)?;
    let down = vm.map_copied(work.inherited);
    let mut own_list = Value::UNDEFINED;
    if work.noting {
        own_list = vm.get_property(note, n.own)?;
        if own_list.is_nullish() {
            own_list = vm.new_array(Vec::new());
            vm.set_property(note, n.own, own_list)?;
        }
    }
    let mut not_down: Vec<(String, String)> = Vec::new();
    let own = std::mem::take(&mut work.own);
    // What's found while each is worked out is what the element set, all of it, as web.js's `own` is.
    work.own = own.clone();
    for (name, value) in &own {
        let inherits = match vm.map_text_value(registered, name) {
            Some(found) if found.is_object() => vm.get_property(found, n.inherits)? != Value::FALSE,
            Some(found) if !found.is_nullish() => {
                // `registered.get(name)?.inherits` of something that isn't an object.
                let atom = n.inherits;
                vm.get_property(found, atom)? != Value::FALSE
            }
            _ => true,
        };
        let Some(text) = vm.string_text(*value).map(|text| text.into_owned()) else {
            return Err(vm.new_string("rsui: a variable an element sets isn't text"));
        };
        let resolved = work.resolve(vm, &text, 0)?;
        if inherits {
            let key = vm.new_string(name);
            let resolved = if resolved == text { *value } else { vm.new_string(&resolved) };
            vm.map_put(down, key, resolved)?;
            if work.noting {
                let pair = vm.new_array(vec![key, resolved]);
                vm.push_item(own_list, pair)?;
            }
        } else {
            not_down.push((name.clone(), resolved));
        }
    }
    if !not_down.is_empty() {
        let own_down = vm.map_copied(down);
        for (name, resolved) in not_down {
            let key = vm.new_string(&name);
            let value = vm.new_string(&resolved);
            vm.map_put(own_down, key, value)?;
        }
        let symbol = vm.get_property(ctx, n.own_variables)?;
        let atom = vm.property_atom(symbol)?;
        vm.set_property(down, atom, own_down)?;
    }
    Ok(down)
}

/// `cssVariablesResized()`: what was kept with the window's units in it goes, as the window is another size.
fn host_resized(_vm: &mut Vm, _args: Args) -> Run<Value> {
    STORE.with(|store| store.borrow_mut().declared_viewport.clear());
    Ok(Value::UNDEFINED)
}

pub(crate) const FUNCTIONS: [(&str, HostFunction); 2] =
    [("cssElementVariables", host_element_variables), ("cssVariablesResized", host_resized)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_found_as_web_js_finds_them() {
        assert_eq!(names_in("var(--a) var( --b-c , 1px) var(--x\\.5)"), vec!["--a", "--b-c", "--x\\.5"]);
        assert_eq!(names_in("var(-a) var(--)"), Vec::<&str>::new());
    }

    #[test]
    fn what_a_var_comes_to_is_kept_apart_from_what_is_beside_it() {
        assert_eq!(apart("var(--tw-duration,.15s)", "ease", Some('x')), " ease ");
        assert_eq!(apart("", "1px", None), "1px");
        assert_eq!(apart("calc(", "1px", Some(')')), "1px");
    }

    #[test]
    fn the_window_units_are_found() {
        assert!(uses_viewport_units("calc(100vh - 2px)"));
        assert!(uses_viewport_units("50 dvw"));
        assert!(uses_viewport_units(".5VMIN"));
        assert!(!uses_viewport_units("1vwx"));
        assert!(!uses_viewport_units("var(--vh)"));
    }
}
