//! `console`, and showing a value the way Node does.

use crate::object::*;
use crate::text::{self, Atom, AtomKind};
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

const MAX_DEPTH: usize = 2;
const LINE_WIDTH: usize = 72;

pub(super) fn install(vm: &mut Vm) {
    let console = vm.plain_proto();
    let atom = vm.intern_str("console");
    vm.add_property(vm.global, atom, Value::object(console), HIDDEN);
    for name in ["log", "info", "debug", "error", "warn", "trace"] {
        vm.method(console, name, 0, log);
    }
}

fn log(vm: &mut Vm, args: Args) -> Run<Value> {
    let mut line = String::new();
    for index in 0..args.count {
        if index > 0 {
            line.push(' ');
        }
        let value = vm.arg(&args, index);
        if value.is_string() {
            line.push_str(&vm.text_of(value).to_rust());
        } else {
            line.push_str(&inspect_value(vm, value));
        }
    }
    line.push('\n');
    let mut printer = std::mem::replace(&mut vm.printer, Box::new(|_| {}));
    printer(&line);
    vm.printer = printer;
    Ok(Value::UNDEFINED)
}

fn quoted(text: &str) -> String {
    let quote = if !text.contains('\'') {
        '\''
    } else if !text.contains('"') {
        '"'
    } else if !text.contains('`') {
        '`'
    } else {
        '\''
    };
    let mut out = String::with_capacity(text.len() + 2);
    out.push(quote);
    for character in text.chars() {
        match character {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

fn is_plain_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c == '$' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c == '$' || c.is_ascii_alphanumeric())
}

fn key_text(vm: &Vm, key: Atom) -> String {
    let name = vm.atom_text(key);
    if vm.atoms.kind_of(key) == AtomKind::Symbol {
        return format!("[Symbol({name})]");
    }
    if is_plain_name(&name) {
        name
    } else {
        quoted(&name)
    }
}

/// The name a constructor gives what it makes, which is what's shown in front of unusual objects.
fn constructor_name(vm: &mut Vm, object: u32) -> Option<String> {
    let mut at = vm.proto_of(object);
    while at != NONE {
        let shape = vm.objects[at as usize].shape;
        if let Some((slot, flags)) = vm.shapes.lookup(shape, text::CONSTRUCTOR) {
            if flags & ACCESSOR == 0 {
                let constructor = vm.objects[at as usize].slots()[slot as usize];
                if constructor.is_object() {
                    if let Ok(name) = vm.get(constructor, text::NAME) {
                        if name.is_string() {
                            return Some(vm.text_of(name).to_rust());
                        }
                    }
                }
            }
            return None;
        }
        at = vm.proto_of(at);
    }
    None
}

struct Shower {
    seen: Vec<Value>,
}

impl Shower {
    /// What something says it should look like, with the method Node looks for. If that goes wrong, it's shown as usual.
    fn own_way_of_showing(&mut self, vm: &mut Vm, value: Value, depth: usize) -> Option<String> {
        let symbol_type = vm.get_named(Value::object(vm.global), "Symbol").ok()?;
        let registry = vm.get_named(symbol_type, "for").ok()?;
        let name = vm.new_str("nodejs.util.inspect.custom");
        let symbol = vm.call(registry, symbol_type, &[name]).ok()?;
        let method = vm.get(value, Atom(symbol.handle())).ok()?;
        if !vm.is_callable(method) {
            return None;
        }
        let options = vm.new_plain();
        let result = vm
            .call(method, value, &[Value::number(depth as f64), options])
            .ok()?;
        if result.is_string() {
            return Some(vm.text_of(result).to_rust());
        }
        // Something giving back itself would go round for ever.
        (result != value).then(|| self.show(vm, result, depth, false))
    }

    fn show(&mut self, vm: &mut Vm, value: Value, depth: usize, top: bool) -> String {
        if value.is_string() {
            let text = vm.text_of(value).to_rust();
            return if top { text } else { quoted(&text) };
        }
        if value.is_number() {
            let number = value.as_number();
            if number == 0.0 && number.is_sign_negative() {
                return "-0".to_owned();
            }
            return crate::convert::number_to_string(number);
        }
        if value.is_symbol() {
            let text = vm.atom_text(Atom(value.handle()));
            return format!("Symbol({text})");
        }
        if value.is_bigint() {
            return format!("{}n", vm.big_of(value).to_radix(10));
        }
        if value.is_object() {
            if let Some(shown) = self.own_way_of_showing(vm, value, depth) {
                return shown;
            }
        }
        if !value.is_object() {
            return if value.is_undefined() {
                "undefined"
            } else if value.is_null() {
                "null"
            } else if value == Value::TRUE {
                "true"
            } else if value == Value::FALSE {
                "false"
            } else {
                "<empty>"
            }
            .to_owned();
        }
        if self.seen.contains(&value) {
            return "[Circular *1]".to_owned();
        }
        self.object(vm, value, depth)
    }

    /// A module's namespace, with what each of its names has now, as Node shows it.
    fn namespace(&mut self, vm: &mut Vm, value: Value, depth: usize) -> String {
        let shown = vm.new_object_with_proto(NONE, Kind::Plain);
        let keys = vm.own_keys_checked(value.handle()).unwrap_or_default();
        for (key, _) in keys {
            if key == text::SYMBOL_TO_STRING_TAG {
                continue;
            }
            let item = vm.get(value, key).unwrap_or(Value::UNDEFINED);
            vm.define(shown.handle(), key, item, PLAIN);
        }
        let text = self.object(vm, shown, depth);
        text.replacen("[Object: null prototype]", "[Module: null prototype]", 1)
    }

    fn object(&mut self, vm: &mut Vm, value: Value, depth: usize) -> String {
        if vm.namespace_target(value).is_some() {
            return self.namespace(vm, value, depth);
        }
        if let Some((target, _)) = vm.proxy_of(value.handle()) {
            return self.show(vm, target, depth, false);
        }
        if vm.typed_data(value).is_some() {
            let name = constructor_name(vm, value.handle()).unwrap_or_default();
            let length = vm.length_of(value).unwrap_or(0);
            let mut items = Vec::new();
            for index in 0..length.min(100) {
                let item = vm.get_index(value, index).unwrap_or(Value::UNDEFINED);
                items.push(self.show(vm, item, depth + 1, false));
            }
            return if items.is_empty() {
                format!("{name}(0) []")
            } else {
                format!("{name}({length}) [ {} ]", items.join(", "))
            };
        }
        let handle = value.handle();
        let name = constructor_name(vm, handle);
        let mut prefix = String::new();
        let mut items = Vec::new();
        let mut brackets = ('{', '}');
        let mut skip_index_keys = false;
        let mut only_prefix = false;
        let class_like = vm.is_callable(value);
        match &vm.objects[handle as usize].kind {
            Kind::Array(_) => {
                brackets = ('[', ']');
                skip_index_keys = true;
                if name.as_deref().is_some_and(|name| name != "Array") {
                    let length = match &vm.objects[handle as usize].kind {
                        Kind::Array(found) => found.len(),
                        _ => 0,
                    };
                    prefix = format!("{}({length}) ", name.clone().unwrap_or_default());
                }
            }
            Kind::Arguments(_) => {
                brackets = ('[', ']');
                skip_index_keys = true;
                prefix = "[Arguments] ".to_owned();
            }
            Kind::Function { proto, .. } => {
                let flags = vm.protos[*proto as usize].flags;
                let fname = vm.atom_text(vm.protos[*proto as usize].name);
                prefix = if flags & crate::op::IS_CLASS != 0 {
                    if fname.is_empty() {
                        "[class (anonymous)]".to_owned()
                    } else {
                        format!("[class {fname}]")
                    }
                } else if fname.is_empty() {
                    "[Function (anonymous)]".to_owned()
                } else {
                    format!("[Function: {fname}]")
                };
                only_prefix = true;
            }
            Kind::Native(native) => {
                let fname = vm.atom_text(native.name);
                prefix = if fname.is_empty() {
                    "[Function (anonymous)]".to_owned()
                } else {
                    format!("[Function: {fname}]")
                };
                only_prefix = true;
            }
            Kind::Bound(_) => {
                prefix = "[Function: bound]".to_owned();
                only_prefix = true;
            }
            Kind::Error => {
                let stack = vm.get(value, text::STACK).unwrap_or(Value::UNDEFINED);
                prefix = if stack.is_string() {
                    vm.text_of(stack).to_rust()
                } else {
                    let heading = vm.to_rust_string(value);
                    heading.unwrap_or_else(|_| "Error".to_owned())
                };
                only_prefix = true;
            }
            Kind::Boxed(inner) => {
                let inner = *inner;
                let shown = self.show(vm, inner, depth, false);
                let kind = vm.class_name(handle);
                prefix = format!("[{kind}: {shown}]");
                only_prefix = true;
            }
            Kind::Date(time) => {
                let time = *time;
                prefix = if time.is_nan() {
                    "Invalid Date".to_owned()
                } else {
                    let to_iso = vm.get_named(value, "toISOString");
                    match to_iso.and_then(|method| vm.call(method, value, &[])) {
                        Ok(text) if text.is_string() => vm.text_of(text).to_rust(),
                        _ => "Invalid Date".to_owned(),
                    }
                };
                only_prefix = true;
            }
            Kind::RegExp(_) => {
                let shown = vm.to_rust_string(value);
                prefix = shown.unwrap_or_default();
                only_prefix = true;
            }
            Kind::ArrayBuffer(data) => {
                prefix = format!("ArrayBuffer {{ byteLength: {} }}", data.bytes.len());
                only_prefix = true;
            }
            Kind::Map(data) | Kind::WeakMap(data) => {
                let live = data.live;
                prefix = format!("Map({live}) ");
            }
            Kind::Set(data) | Kind::WeakSet(data) => {
                let live = data.live;
                prefix = format!("Set({live}) ");
            }
            Kind::Promise(_) => prefix = "Promise ".to_owned(),
            Kind::Generator(_) => prefix = "Object [Generator] ".to_owned(),
            Kind::Plain => {
                if vm.proto_of(handle) == NONE {
                    prefix = "[Object: null prototype] ".to_owned();
                } else if let Some(name) = &name {
                    if name != "Object" {
                        prefix = format!("{name} ");
                    }
                }
            }
            _ => {}
        }
        if only_prefix {
            let keys = visible_keys(vm, handle, skip_index_keys, class_like);
            if keys.is_empty() {
                return prefix;
            }
            prefix.push(' ');
        }
        if depth > MAX_DEPTH {
            return match brackets {
                ('[', _) => "[Array]".to_owned(),
                _ => match name {
                    Some(name) if name != "Object" => format!("[{name}]"),
                    _ => "[Object]".to_owned(),
                },
            };
        }
        self.seen.push(value);
        match &vm.objects[handle as usize].kind {
            Kind::Array(found) | Kind::Arguments(found) => {
                let found = found.to_vec();
                let mut holes = 0;
                let flush = |holes: &mut i32, items: &mut Vec<String>| {
                    if *holes > 0 {
                        let plural = if *holes == 1 { "item" } else { "items" };
                        items.push(format!("<{holes} empty {plural}>"));
                        *holes = 0;
                    }
                };
                for (index, item) in found.iter().enumerate() {
                    if index >= 100 {
                        flush(&mut holes, &mut items);
                        items.push(format!("... {} more items", found.len() - 100));
                        break;
                    }
                    if item.is_empty() {
                        holes += 1;
                    } else {
                        flush(&mut holes, &mut items);
                        items.push(self.show(vm, *item, depth + 1, false));
                    }
                }
                flush(&mut holes, &mut items);
            }
            Kind::Map(data) | Kind::WeakMap(data) => {
                let entries = data.entries.clone();
                for (key, item) in entries {
                    if !key.is_empty() {
                        let key = self.show(vm, key, depth + 1, false);
                        let item = self.show(vm, item, depth + 1, false);
                        items.push(format!("{key} => {item}"));
                    }
                }
            }
            Kind::Set(data) | Kind::WeakSet(data) => {
                let entries = data.entries.clone();
                for (key, _) in entries {
                    if !key.is_empty() {
                        items.push(self.show(vm, key, depth + 1, false));
                    }
                }
            }
            Kind::Promise(data) => {
                let (state, inner) = (data.state, data.value);
                items.push(match state {
                    PromiseState::Pending => "<pending>".to_owned(),
                    PromiseState::Fulfilled => self.show(vm, inner, depth + 1, false),
                    PromiseState::Rejected => {
                        format!("<rejected> {}", self.show(vm, inner, depth + 1, false))
                    }
                });
            }
            _ => {}
        }
        for key in visible_keys(vm, handle, skip_index_keys, class_like) {
            let shape = vm.objects[handle as usize].shape;
            let shown_value = match vm.shapes.lookup(shape, key) {
                Some((slot, flags)) if flags & ACCESSOR != 0 => {
                    let holder = vm.objects[handle as usize].slots()[slot as usize];
                    match vm.objects[holder.index()].kind {
                        Kind::Accessor { get, set } => {
                            match (get.is_undefined(), set.is_undefined()) {
                                (false, false) => "[Getter/Setter]".to_owned(),
                                (false, true) => "[Getter]".to_owned(),
                                _ => "[Setter]".to_owned(),
                            }
                        }
                        _ => "[Getter]".to_owned(),
                    }
                }
                _ => {
                    let inner = vm.get(value, key).unwrap_or(Value::UNDEFINED);
                    self.show(vm, inner, depth + 1, false)
                }
            };
            items.push(format!("{}: {shown_value}", key_text(vm, key)));
        }
        self.seen.pop();
        if items.is_empty() {
            return format!("{prefix}{}{}", brackets.0, brackets.1);
        }
        let total: usize = items.iter().map(|item| item.len() + 2).sum();
        if total + prefix.len() <= LINE_WIDTH && !items.iter().any(|item| item.contains('\n')) {
            return format!("{prefix}{} {} {}", brackets.0, items.join(", "), brackets.1);
        }
        let indent = "  ".repeat(depth + 1);
        let closing_indent = "  ".repeat(depth);
        let body: Vec<String> = items.iter().map(|item| format!("{indent}{item}")).collect();
        format!(
            "{prefix}{}\n{}\n{closing_indent}{}",
            brackets.0,
            body.join(",\n"),
            brackets.1
        )
    }
}

fn visible_keys(vm: &mut Vm, object: u32, skip_index_keys: bool, is_function: bool) -> Vec<Atom> {
    vm.own_keys(object)
        .into_iter()
        .filter(|(key, flags)| {
            flags & ENUMERABLE != 0
                && !(skip_index_keys && vm.atoms.index_of(*key).is_some())
                && !(is_function && (*key == text::NAME || *key == text::LENGTH))
        })
        .map(|(key, _)| key)
        .collect()
}

/// A value as `console.log` shows it.
pub(crate) fn inspect_value(vm: &mut Vm, value: Value) -> String {
    let mut shower = Shower { seen: Vec::new() };
    shower.show(vm, value, 0, false)
}
