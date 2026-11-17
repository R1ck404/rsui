//! Why a JavaScript component rendered, in sentences, and what its hooks hold, by the names they're given in the code.
//!
//! The runtime knows a component rendered because it's new, because its props changed, or because its state did. This
//! says which prop, and where the function that's new each time is made, or which state was set, from what to what and on
//! which line. Hooks go by the variable they're put in, as in `const [count, setCount] = useState(0)`, which is read from
//! the line that called them.
//!
//! None of it happens unless something's looking. Then the layer notes where each hook is called from, once, and where
//! state is set from, each time.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;

use std::collections::VecDeque;

use rsui_js::{Args, HostFunction, Origin, Root, Value, Vm};

/// How long a value can be when it's shown.
const LONGEST_VALUE: usize = 60;
/// How many things a component keeps to say about why it's going to render, before it's told.
const MOST_NOTES: usize = 6;
/// How many changes of state are kept to go back through. Each keeps what it changed from and to alive.
const MOST_CHANGES: usize = 1000;

/// A change of a component's state, with what it was before and after, to go back and forward through.
pub(crate) struct Change {
    /// Counting from 1, from when keeping them started.
    pub(crate) number: u32,
    /// The component, and what happened, as a sentence.
    pub(crate) component: String,
    pub(crate) said: String,
    /// The layer's object for the component, and which of its hooks it was.
    pub(crate) instance: Root,
    pub(crate) index: u32,
    pub(crate) slot: u32,
    pub(crate) before: Root,
    pub(crate) after: Root,
}

#[derive(Default)]
struct History {
    changes: VecDeque<Change>,
    next: u32,
    /// The change things have been put back to, if they have, rather than being as they are after the last.
    at: Option<u32>,
}

#[derive(Default)]
struct Why {
    on: Cell<bool>,
    /// What's been noted about each component, by its slot, until it renders.
    notes: RefCell<HashMap<u32, Vec<String>>>,
    /// Each file's lines, once one's been read to name a hook.
    sources: RefCell<HashMap<String, Option<Rc<[String]>>>>,
    /// The slot of each component the runtime knows, by the runtime it's in and how that numbers it, and the other way.
    slots: RefCell<HashMap<(u32, u32, u32), u32>>,
    components: RefCell<HashMap<u32, (u32, u32, u32)>>,
    history: RefCell<History>,
}

thread_local! {
    static WHY: Why = Why::default();
}

pub(crate) fn tracing() -> bool {
    WHY.with(|why| why.on.get())
}

/// Says whether the layer is to note what it does, and forgets what it noted if not. Returns whether that changed.
pub(crate) fn set_tracing(on: bool) -> bool {
    WHY.with(|why| {
        if why.on.replace(on) == on {
            return false;
        }
        if !on {
            why.notes.borrow_mut().clear();
            why.slots.borrow_mut().clear();
            why.components.borrow_mut().clear();
            *why.history.borrow_mut() = History::default();
            why.sources.borrow_mut().clear();
        }
        true
    })
}

pub(crate) fn remember_slot(runtime: u32, component: (u32, u32), slot: u32) {
    WHY.with(|why| {
        let key = (runtime, component.0, component.1);
        if let Some(old) = why.components.borrow_mut().insert(slot, key) {
            if old != key {
                why.slots.borrow_mut().remove(&old);
            }
        }
        why.slots.borrow_mut().insert(key, slot);
    });
}

pub(crate) fn slot_of(runtime: u32, component: (u32, u32)) -> Option<u32> {
    WHY.with(|why| {
        why.slots
            .borrow()
            .get(&(runtime, component.0, component.1))
            .copied()
    })
}

pub(crate) fn forget_slot(slot: u32) {
    WHY.with(|why| {
        if !why.on.get() {
            return;
        }
        why.notes.borrow_mut().remove(&slot);
        if let Some(key) = why.components.borrow_mut().remove(&slot) {
            why.slots.borrow_mut().remove(&key);
        }
    });
}

fn note(slot: u32, text: String) {
    WHY.with(|why| {
        let mut notes = why.notes.borrow_mut();
        let kept = notes.entry(slot).or_default();
        if kept.len() < MOST_NOTES && !kept.contains(&text) {
            kept.push(text);
        }
    });
}

/// Something that's happened to every component, which each says when it next renders.
pub(crate) fn note_all(slots: u32, text: &str) {
    if tracing() {
        for slot in 0..slots {
            note(slot, text.to_owned());
        }
    }
}

/// What's been noted about a component since it last rendered.
pub(crate) fn take_notes(slot: u32) -> Vec<String> {
    WHY.with(|why| why.notes.borrow_mut().remove(&slot).unwrap_or_default())
}

pub(crate) const FUNCTIONS: [(&str, HostFunction); 3] = [
    ("hookSite", host_hook_site),
    ("noteSet", host_note_set),
    ("noteContext", host_note_context),
];

/// The lines of the app's own code that are running, innermost first.
fn app_callers(vm: &Vm, most: usize) -> Vec<Origin> {
    vm.callers(most, |file| file.is_empty() || file.starts_with("node:"))
}

/// Where the hook being called is called from: the line, and if that's in a hook of the app's own, the line that called
/// that, as a list of `file`, `line` and `function` with a tab between them, one to a line.
fn host_hook_site(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    let mut site = String::new();
    for origin in app_callers(vm, 3) {
        let _ = writeln!(
            site,
            "{}\t{}\t{}",
            origin.file, origin.line, origin.function
        );
        if !is_hook_name(&origin.function) {
            break;
        }
    }
    Ok(vm.new_string(site.trim_end()))
}

fn is_hook_name(name: &str) -> bool {
    name.strip_prefix("use")
        .and_then(|rest| rest.chars().next())
        .is_some_and(|first| first.is_ascii_uppercase())
}

/// `noteSet(slot, index, site, before, after, kind, action, instance)`: state was set, so its component's going to render.
fn host_note_set(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let slot = args.get(vm, 0).as_number() as u32;
    let index = args.get(vm, 1).as_number() as u32;
    let site = args.get(vm, 2);
    let site = vm.string_text(site).map(|site| site.into_owned());
    let before = args.get(vm, 3);
    let after = args.get(vm, 4);
    let kind = args.get(vm, 5);
    let reducer = vm.string_text(kind).is_some_and(|kind| kind == "reducer");
    let action = args.get(vm, 6);
    let name = site.as_deref().and_then(hook_name).map_or_else(
        || format!("state number {}", index + 1),
        |name| format!("`{name}`"),
    );
    let mut text = if reducer {
        format!(
            "{name} was changed by its reducer, given {},",
            vm.describe(action, LONGEST_VALUE)
        )
    } else {
        format!("{name} was set")
    };
    let _ = write!(
        text,
        " from {} to {}",
        vm.describe(before, LONGEST_VALUE),
        vm.describe(after, LONGEST_VALUE)
    );
    if let Some(at) = app_callers(vm, 1).first() {
        let _ = write!(text, " at {}", short_place(at));
        if !at.function.is_empty() {
            let _ = write!(text, ", in {}", at.function);
        }
    }
    text.push('.');
    note(slot, text.clone());
    let instance = args.get(vm, 7);
    if instance.is_object() {
        let component = crate::bridge::instance_name(vm, instance);
        let change = Change {
            number: 0,
            component,
            said: text,
            instance: vm.root(instance),
            index,
            slot,
            before: vm.root(before),
            after: vm.root(after),
        };
        WHY.with(|why| why.history.borrow_mut().push(change));
    }
    Ok(Value::UNDEFINED)
}

impl History {
    fn push(&mut self, mut change: Change) {
        // Changing something after going back leaves out what had come after it, as undoing does.
        // What comes next is numbered on from where it went back to.
        if let Some(at) = self.at.take() {
            self.changes.retain(|kept| kept.number <= at);
            self.next = at;
        }
        self.next += 1;
        change.number = self.next;
        self.changes.push_back(change);
        while self.changes.len() > MOST_CHANGES {
            self.changes.pop_front();
        }
    }

    fn now(&self) -> u32 {
        self.at
            .unwrap_or_else(|| self.changes.back().map_or(0, |change| change.number))
    }
}

thread_local! {
    /// Why the app's code is next taken in again, if it isn't that a file was saved.
    static RELOAD_REASON: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

/// Says why the app's code is about to be taken in again, for what renders then.
pub(crate) fn reloading_because(why: &'static str) {
    RELOAD_REASON.with(|reason| reason.set(Some(why)));
}

pub(crate) fn take_reload_reason() -> Option<&'static str> {
    RELOAD_REASON.with(std::cell::Cell::take)
}

/// What's said about going to a change, after `put_back` hooks have been.
pub(crate) fn went_to(change: u32, put_back: usize) -> String {
    format!(
        "it's as it was {}, with {put_back} {} put back. What isn't state, such as refs and files, stays as it is",
        if change == 0 {
            "before all of them".to_owned()
        } else {
            format!("after change {change}")
        },
        if put_back == 1 { "hook" } else { "hooks" }
    )
}

/// Which change things are as they were after: the newest, unless something's gone back.
pub(crate) fn history_now() -> u32 {
    WHY.with(|why| why.history.borrow().now())
}

/// The changes of state kept: each one's number, component, and what happened.
pub(crate) fn history_entries() -> Vec<(u32, String, String)> {
    WHY.with(|why| {
        why.history
            .borrow()
            .changes
            .iter()
            .map(|change| (change.number, change.component.clone(), change.said.clone()))
            .collect()
    })
}

/// The changes of state kept, a line each, with where things are now among them.
pub(crate) fn history_text() -> String {
    WHY.with(|why| {
        let history = why.history.borrow();
        if history.changes.is_empty() {
            return "no state has changed since this started keeping it".to_owned();
        }
        let now = history.now();
        let mut out = String::new();
        if let Some(first) = history.changes.front() {
            if first.number > 1 {
                let _ = writeln!(
                    out,
                    "({} earlier changes are no longer kept)",
                    first.number - 1
                );
            }
        }
        for change in &history.changes {
            let _ = writeln!(
                out,
                "{}. {}: {}{}",
                change.number,
                change.component,
                change.said,
                if change.number == now && history.at.is_some() {
                    "  ← it's back to here"
                } else {
                    ""
                }
            );
        }
        if history.at == Some(0) {
            out.push_str("It's back to before all of these.\n");
        }
        out.trim_end().to_owned()
    })
}

/// What to put back, to get to how things were just after change `target`, or before any for 0: each component's hook,
/// with the value it gets. Things are counted as being there from now on.
pub(crate) fn plan_goto(target: u32) -> Result<Vec<(Root, u32, u32, Root)>, String> {
    WHY.with(|why| {
        let mut history = why.history.borrow_mut();
        let now = history.now();
        let oldest = history.changes.front().map_or(1, |change| change.number);
        let newest = history.changes.back().map_or(0, |change| change.number);
        // Going to the one before the oldest kept undoes all of them.
        if target + 1 < oldest || target > newest {
            return Err(format!(
                "there's no change {target} to go to. It can go to any from {} to {newest}",
                oldest - 1
            ));
        }
        let mut plan = Vec::new();
        if target < now {
            for change in history.changes.iter().rev() {
                if change.number > target && change.number <= now {
                    plan.push((
                        change.instance.clone(),
                        change.index,
                        change.slot,
                        change.before.clone(),
                    ));
                }
            }
        } else {
            for change in &history.changes {
                if change.number > now && change.number <= target {
                    plan.push((
                        change.instance.clone(),
                        change.index,
                        change.slot,
                        change.after.clone(),
                    ));
                }
            }
        }
        history.at = (target != newest).then_some(target);
        Ok(plan)
    })
}

/// Says why a component that's been put back is going to render.
pub(crate) fn note_put_back(slot: u32, target: u32) {
    note(
        slot,
        if target == 0 {
            "Its state was put back to how it was before any change that's kept.".to_owned()
        } else {
            format!("Its state was put back to how it was after change {target}.")
        },
    );
}

/// `noteContext(slot, site)`: what a component reads from a context changed.
fn host_note_context(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let slot = args.get(vm, 0).as_number() as u32;
    let site = args.get(vm, 1);
    let name = vm.string_text(site).and_then(|site| hook_name(&site));
    note(
        slot,
        match name {
            Some(name) => format!("The context it reads as `{name}` changed."),
            None => "A context it reads changed.".to_owned(),
        },
    );
    Ok(Value::UNDEFINED)
}

/// A place in a file, with the file as it's named from the app's folder.
pub(crate) fn short_place(origin: &Origin) -> String {
    format!("{}:{}", short_file(&origin.file), origin.line)
}

fn short_file(file: &str) -> String {
    let app = crate::bridge::app_file();
    let folder = std::path::absolute(&app)
        .ok()
        .and_then(|app| {
            app.parent()
                .map(|folder| folder.to_string_lossy().replace('\\', "/"))
        })
        .unwrap_or_default();
    let file = file.replace('\\', "/");
    // Windows doesn't mind which case a path's in.
    if !folder.is_empty()
        && file.len() > folder.len()
        && file.is_char_boundary(folder.len())
        && file[..folder.len()].eq_ignore_ascii_case(&folder)
    {
        return file[folder.len()..].trim_start_matches('/').to_owned();
    }
    file
}

/// The places in a site the layer kept, innermost first.
fn site_places(site: &str) -> Vec<Origin> {
    site.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            Some(Origin {
                file: parts.next()?.to_owned(),
                line: parts.next()?.parse().ok()?,
                function: parts.next().unwrap_or("").to_owned(),
            })
        })
        .collect()
}

/// What a hook's called in the code, from where it's called: `count` for `const [count, setCount] = useState(0)`. If
/// it's inside a hook of the app's own, that's said too, as `useTodos → items`.
pub(crate) fn hook_name(site: &str) -> Option<String> {
    let places = site_places(site);
    let innermost = places.first()?;
    let mut name = binding_on(&innermost.file, innermost.line)?;
    if is_hook_name(&innermost.function) {
        name = format!("{} → {name}", innermost.function);
    }
    Some(name)
}

/// What's given a value on this line from a call to a hook: the first name in `const [a, b] = useX(`, the name in
/// `const a = useX(`, or the names in `const { a, b } = useX(`.
fn binding_on(file: &str, line: u32) -> Option<String> {
    let text = line_of(file, line)?;
    let call = find_hook_call(&text)?;
    let before = text[..call].trim_end().strip_suffix('=')?.trim();
    let target = ["const ", "let ", "var "]
        .iter()
        .find_map(|word| before.rsplit_once(word).map(|(_, target)| target))
        .unwrap_or(before)
        .trim();
    // A type annotation isn't part of the name.
    let target = match target.find(':') {
        Some(colon) if !target.starts_with('{') => target[..colon].trim(),
        _ => target,
    };
    if let Some(inside) = target.strip_prefix('[') {
        let first = inside.split([',', ']']).next()?.trim();
        return (!first.is_empty()).then(|| first.to_owned());
    }
    if target.starts_with('{') {
        let inside = target.trim_start_matches('{').trim_end_matches('}');
        let names: Vec<&str> = inside
            .split(',')
            .map(|part| part.split(':').next().unwrap_or("").trim())
            .filter(|part| !part.is_empty())
            .collect();
        return (!names.is_empty()).then(|| format!("{{ {} }}", names.join(", ")));
    }
    let name = target.rsplit(['.', ' ']).next()?.trim();
    (!name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$'))
    .then(|| name.to_owned())
}

/// Where the first call to something named like a hook starts on a line.
fn find_hook_call(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(found) = text[from..].find("use") {
        let at = from + found;
        let starts_word = at == 0
            || !(bytes[at - 1].is_ascii_alphanumeric()
                || bytes[at - 1] == b'_'
                || bytes[at - 1] == b'$');
        let rest = &text[at..];
        let name_end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
            .unwrap_or(rest.len());
        let calls = rest[name_end..].trim_start().starts_with(['(', '<']);
        if starts_word && is_hook_name(&rest[..name_end]) && calls {
            return Some(at);
        }
        from = at + 3;
    }
    None
}

fn line_of(file: &str, line: u32) -> Option<String> {
    let lines = WHY.with(|why| {
        why.sources
            .borrow_mut()
            .entry(file.to_owned())
            .or_insert_with(|| {
                std::fs::read_to_string(file)
                    .ok()
                    .map(|text| text.lines().map(str::to_owned).collect())
            })
            .clone()
    })?;
    lines.get((line as usize).checked_sub(1)?).cloned()
}

/// The component whose code makes a function, from the components around one, as the layer has them: the nearest whose
/// own code is in the same file and starts before it.
fn maker_of(vm: &mut Vm, made: &Origin, around: Value) -> Option<String> {
    let (type_name, parent_name, name) = (vm.atom("type"), vm.atom("parent"), vm.atom("name"));
    let mut at = around;
    for _ in 0..64 {
        if !at.is_object() {
            return None;
        }
        let function = vm.get_property(at, type_name).ok()?;
        if let Some(origin) = vm.function_origin(function) {
            if origin.file == made.file && origin.line <= made.line {
                let named = vm.get_property(function, name).ok()?;
                return vm.plain_text(named).filter(|named| !named.is_empty());
            }
        }
        at = vm.get_property(at, parent_name).ok()?;
    }
    None
}

/// Why a component that was given new props rendered: which of them aren't the same, and why not. `around` is the
/// layer's object for the component it's in.
pub(crate) fn props_why(
    vm: &mut Vm,
    old: Value,
    new: Value,
    parent: &str,
    around: Value,
) -> Vec<String> {
    let (Ok(old_props), Ok(new_props)) = (vm.properties(old), vm.properties(new)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (atom, value) in &new_props {
        let name = vm.atom_name(*atom);
        let was = old_props
            .iter()
            .find(|(old_atom, _)| old_atom == atom)
            .map(|(_, was)| *was);
        let Some(was) = was else {
            out.push(format!("`{name}` is new."));
            continue;
        };
        if vm.same(was, *value) {
            continue;
        }
        out.push(prop_change(vm, &name, was, *value, parent, around));
    }
    for (atom, _) in &old_props {
        if !new_props.iter().any(|(new_atom, _)| new_atom == atom) {
            out.push(format!("`{}` was taken away.", vm.atom_name(*atom)));
        }
    }
    if out.is_empty() {
        out.push(format!(
            "{parent} gave it the same props, but not in the same order, which counts as different."
        ));
    }
    out
}

fn prop_change(
    vm: &mut Vm,
    name: &str,
    was: Value,
    is: Value,
    parent: &str,
    around: Value,
) -> String {
    if vm.is_function(is) && vm.is_function(was) {
        let made = vm.function_origin(is);
        let same_code = made.is_some() && made == vm.function_origin(was);
        let mut text = format!("`{name}` is a new function");
        if same_code {
            text.push_str(" each time");
        }
        text.push('.');
        if let Some(made) = made {
            let maker = maker_of(vm, &made, around).unwrap_or_else(|| parent.to_owned());
            let _ = write!(text, " {maker} makes it at {}.", short_place(&made));
        }
        if same_code {
            text.push_str(" useCallback would keep the same one.");
        }
        return text;
    }
    if is.is_object() && was.is_object() {
        let what = if vm.array_elements(is).is_some() {
            "array"
        } else {
            "object"
        };
        if name == "children" {
            return format!(
                "`children` is a new {what}, as JSX makes one each time {parent} renders."
            );
        }
        if vm.shallow_equal(was, is) {
            return format!(
                "`{name}` is a new {what} with the same things in it. useMemo would keep the same one."
            );
        }
        return format!("`{name}` is a different {what}.");
    }
    format!(
        "`{name}` changed from {} to {}.",
        vm.describe(was, LONGEST_VALUE),
        vm.describe(is, LONGEST_VALUE)
    )
}

/// What a component's hooks hold, a line each: its name if it has one, its kind, what it holds and where it's called.
pub(crate) fn describe_hooks(vm: &mut Vm, hooks: Value) -> Vec<String> {
    let Some(hooks) = vm.array_elements(hooks).map(<[Value]>::to_vec) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (index, hook) in hooks.into_iter().enumerate() {
        let Some(parts) = vm.array_elements(hook).map(<[Value]>::to_vec) else {
            continue;
        };
        let [kind, site, value] = parts[..] else {
            continue;
        };
        let kind = vm
            .string_text(kind)
            .map(|kind| kind.into_owned())
            .unwrap_or_default();
        let site = vm.string_text(site).map(|site| site.into_owned());
        let name = site.as_deref().and_then(hook_name);
        let shown = vm.describe(value, LONGEST_VALUE);
        let mut line = match name {
            Some(name) => format!("{name} = {shown}"),
            None => format!("{} {} = {shown}", kind, index + 1),
        };
        let _ = write!(line, "  ({kind}");
        if let Some(place) = site
            .as_deref()
            .and_then(|site| site_places(site).into_iter().next())
        {
            let _ = write!(line, ", {}", short_place(&place));
        }
        line.push(')');
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(line: &str) -> Option<String> {
        let call = find_hook_call(line)?;
        let _ = call;
        // As `binding_on` reads it, without a file.
        let before = line[..find_hook_call(line)?]
            .trim_end()
            .strip_suffix('=')?
            .trim();
        let target = ["const ", "let ", "var "]
            .iter()
            .find_map(|word| before.rsplit_once(word).map(|(_, target)| target))
            .unwrap_or(before)
            .trim();
        Some(target.to_owned())
    }

    #[test]
    fn a_hook_call_is_found_where_it_starts() {
        assert_eq!(
            find_hook_call("    const [count, setCount] = useState(0);"),
            Some(30)
        );
        assert_eq!(
            find_hook_call("const user = reuse(x) + useMemo(() => 1, [])"),
            Some(24)
        );
        assert_eq!(find_hook_call("const x = useless;"), None);
        assert_eq!(find_hook_call("const s = useState<number>(0)"), Some(10));
        assert_eq!(
            named("const { a, b } = useThing()").as_deref(),
            Some("{ a, b }")
        );
    }

    #[test]
    fn a_hook_inside_a_hook_says_both() {
        let dir = std::env::temp_dir().join(format!("rsui-why-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("app.tsx");
        std::fs::write(
            &file,
            "function useTodos() {\n    const [items, setItems] = useState([]);\n}\nconst ref: Ref = useRef(null);\nconst { a, b: c } = useThing();\n",
        )
        .unwrap();
        let file = file.to_string_lossy().into_owned();
        assert_eq!(
            hook_name(&format!("{file}\t2\tuseTodos\n{file}\t9\tApp")).as_deref(),
            Some("useTodos → items")
        );
        assert_eq!(
            hook_name(&format!("{file}\t4\tApp")).as_deref(),
            Some("ref")
        );
        assert_eq!(
            hook_name(&format!("{file}\t5\tApp")).as_deref(),
            Some("{ a, b }")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
