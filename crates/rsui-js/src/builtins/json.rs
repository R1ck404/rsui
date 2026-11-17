//! `JSON`.

use std::collections::HashMap;
use std::rc::Rc;

use crate::object::*;
use crate::text::{self, Atom, AtomKind, Text};
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

pub(super) fn install(vm: &mut Vm) {
    let json = vm.plain_proto();
    let atom = vm.intern_str("JSON");
    vm.add_property(vm.global, atom, Value::object(json), HIDDEN);
    vm.method(json, "parse", 2, parse);
    vm.method(json, "stringify", 3, stringify);
    let tag = vm.new_str("JSON");
    vm.add_property(json, text::SYMBOL_TO_STRING_TAG, tag, CONFIGURABLE);
}

/// What JSON text can be made of: bytes if it's all ASCII, which needs no widening, or else units.
trait Unit: Copy + Into<u16> + PartialEq {
    fn text(units: &[Self]) -> Text;
}

impl Unit for u8 {
    fn text(units: &[Self]) -> Text {
        Text::from_ascii(units)
    }
}

impl Unit for u16 {
    fn text(units: &[Self]) -> Text {
        Text::from_units(units)
    }
}

struct Reader<'a, U> {
    units: &'a [U],
    at: usize,
    /// How much of the list of new things was there at the start.
    mark: usize,
    /// Keys read lately, as where they are in the text, how long they are, and their atom. A document mostly has the
    /// same few keys over and over, and finding one here is quicker than making its text and looking that up.
    keys: [(u32, u32, Atom); KEY_WAYS],
}

const KEY_WAYS: usize = 64;

const MAX_DEPTH: u32 = 512;

impl<U: Unit> Reader<'_, U> {
    #[inline]
    fn skip_space(&mut self) {
        while let Some(unit) = self.units.get(self.at) {
            if !matches!((*unit).into(), 9 | 10 | 13 | 32) {
                break;
            }
            self.at += 1;
        }
    }

    #[inline]
    fn peek(&self) -> Option<u16> {
        self.units.get(self.at).map(|unit| (*unit).into())
    }

    fn error(&self, vm: &mut Vm) -> Value {
        match self.peek() {
            Some(unit) => {
                let shown = String::from_utf16_lossy(&[unit]);
                vm.syntax_error(&format!(
                    "Unexpected token '{shown}', at position {} is not valid JSON",
                    self.at
                ))
            }
            None => vm.syntax_error("Unexpected end of JSON input"),
        }
    }

    fn value(&mut self, vm: &mut Vm, depth: u32) -> Run<Value> {
        if depth > MAX_DEPTH {
            return Err(vm.range_error("This JSON is nested too deeply"));
        }
        self.skip_space();
        let Some(unit) = self.peek() else {
            return Err(self.error(vm));
        };
        match char::from_u32(u32::from(unit)) {
            Some('{') => self.object(vm, depth),
            Some('[') => self.array(vm, depth),
            Some('"') => {
                let text = self.string(vm)?;
                Ok(vm.new_text(text))
            }
            Some('t') => self.word(vm, "true", Value::TRUE),
            Some('f') => self.word(vm, "false", Value::FALSE),
            Some('n') => self.word(vm, "null", Value::NULL),
            Some('-' | '0'..='9') => self.number(vm),
            _ => Err(self.error(vm)),
        }
    }

    fn word(&mut self, vm: &mut Vm, word: &str, value: Value) -> Run<Value> {
        for byte in word.bytes() {
            if self.peek() != Some(u16::from(byte)) {
                return Err(self.error(vm));
            }
            self.at += 1;
        }
        Ok(value)
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while self.peek().is_some_and(|unit| (48..58).contains(&unit)) {
            self.at += 1;
        }
        self.at - start
    }

    fn number(&mut self, vm: &mut Vm) -> Run<Value> {
        let start = self.at;
        // A short whole number needs nothing but adding up.
        let mut small = 0i32;
        let mut count = 0;
        while let Some(digit @ 48..58) = self.peek() {
            small = small.wrapping_mul(10).wrapping_add(i32::from(digit - 48));
            count += 1;
            self.at += 1;
            if count > 9 {
                break;
            }
        }
        let first_is_zero = count > 1 && self.units[start].into() == 48;
        if count > 0 && count <= 9 && !first_is_zero && !matches!(self.peek(), Some(46 | 101 | 69))
        {
            return Ok(Value::int(small));
        }
        self.at = start;
        if self.peek() == Some(45) {
            self.at += 1;
        }
        if self.peek() == Some(48) {
            self.at += 1;
        } else if self.digits() == 0 {
            return Err(self.error(vm));
        }
        if self.peek() == Some(46) {
            self.at += 1;
            if self.digits() == 0 {
                return Err(self.error(vm));
            }
        }
        if matches!(self.peek(), Some(101 | 69)) {
            self.at += 1;
            if matches!(self.peek(), Some(43 | 45)) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return Err(self.error(vm));
            }
        }
        // These are all digits, signs, points or an `e`, so a unit is a character.
        let text: String = self.units[start..self.at]
            .iter()
            .map(|unit| char::from((*unit).into() as u8))
            .collect();
        Ok(Value::number(text.parse().unwrap_or(f64::NAN)))
    }

    fn string(&mut self, vm: &mut Vm) -> Run<Text> {
        self.at += 1;
        let start = self.at;
        // Most strings have no escapes in them, and then the text is just what's there.
        while let Some(unit) = self.units.get(self.at) {
            let unit: u16 = (*unit).into();
            if unit == 34 {
                let text = U::text(&self.units[start..self.at]);
                self.at += 1;
                return Ok(text);
            }
            if unit == 92 || unit < 32 {
                break;
            }
            self.at += 1;
        }
        let mut out: Vec<u16> = self.units[start..self.at]
            .iter()
            .map(|unit| (*unit).into())
            .collect();
        loop {
            let Some(unit) = self.peek() else {
                return Err(self.error(vm));
            };
            self.at += 1;
            match unit {
                34 => return Ok(Text::from_units(&out)),
                92 => {
                    let Some(escape) = self.peek() else {
                        return Err(self.error(vm));
                    };
                    self.at += 1;
                    match char::from_u32(u32::from(escape)) {
                        Some('"') => out.push(34),
                        Some('\\') => out.push(92),
                        Some('/') => out.push(47),
                        Some('b') => out.push(8),
                        Some('f') => out.push(12),
                        Some('n') => out.push(10),
                        Some('r') => out.push(13),
                        Some('t') => out.push(9),
                        Some('u') => {
                            let mut code = 0u32;
                            for _ in 0..4 {
                                let digit = self
                                    .peek()
                                    .and_then(|unit| char::from_u32(u32::from(unit)))
                                    .and_then(|c| c.to_digit(16));
                                let Some(digit) = digit else {
                                    return Err(self.error(vm));
                                };
                                code = code * 16 + digit;
                                self.at += 1;
                            }
                            out.push(code as u16);
                        }
                        _ => {
                            self.at -= 1;
                            return Err(self.error(vm));
                        }
                    }
                }
                0..=31 => {
                    self.at -= 1;
                    return Err(self.error(vm));
                }
                _ => out.push(unit),
            }
        }
    }

    /// A key, as an atom. One with no escapes in it that was read lately needs nothing new made.
    fn key(&mut self, vm: &mut Vm) -> Run<Atom> {
        let start = self.at + 1;
        let mut end = start;
        let mut hash = 0u32;
        while let Some(unit) = self.units.get(end) {
            let unit: u16 = (*unit).into();
            if unit == 34 {
                let length = (end - start) as u32;
                let way = (hash ^ length) as usize % KEY_WAYS;
                let (known_start, known_length, atom) = self.keys[way];
                if known_length == length {
                    let known = known_start as usize;
                    if self.units[known..known + length as usize] == self.units[start..end] {
                        self.at = end + 1;
                        return Ok(atom);
                    }
                }
                let atom = vm
                    .atoms
                    .intern_key(&mut vm.strings, U::text(&self.units[start..end]));
                self.keys[way] = (start as u32, length, atom);
                self.at = end + 1;
                return Ok(atom);
            }
            if unit == 92 || unit < 32 {
                break;
            }
            hash = hash.wrapping_mul(31).wrapping_add(u32::from(unit));
            end += 1;
        }
        let key = self.string(vm)?;
        Ok(vm.atoms.intern_key(&mut vm.strings, key))
    }

    fn array(&mut self, vm: &mut Vm, depth: u32) -> Run<Value> {
        self.at += 1;
        let mut items = Vec::new();
        self.skip_space();
        if self.peek() == Some(93) {
            self.at += 1;
            return Ok(vm.new_array(items));
        }
        loop {
            items.push(self.value(vm, depth + 1)?);
            vm.nursery.truncate(self.mark);
            self.skip_space();
            match self.peek() {
                Some(44) => self.at += 1,
                Some(93) => {
                    self.at += 1;
                    return Ok(vm.new_array(items));
                }
                _ => return Err(self.error(vm)),
            }
        }
    }

    fn object(&mut self, vm: &mut Vm, depth: u32) -> Run<Value> {
        self.at += 1;
        let object = vm.new_plain();
        self.skip_space();
        if self.peek() == Some(125) {
            self.at += 1;
            return Ok(object);
        }
        loop {
            self.skip_space();
            if self.peek() != Some(34) {
                return Err(self.error(vm));
            }
            let atom = self.key(vm)?;
            self.skip_space();
            if self.peek() != Some(58) {
                return Err(self.error(vm));
            }
            self.at += 1;
            let value = self.value(vm, depth + 1)?;
            vm.define(object.handle(), atom, value, PLAIN);
            // Nothing can run the collector in here, so the list of new things needn't grow with the whole document.
            vm.nursery.truncate(self.mark);
            self.skip_space();
            match self.peek() {
                Some(44) => self.at += 1,
                Some(125) => {
                    self.at += 1;
                    return Ok(object);
                }
                _ => return Err(self.error(vm)),
            }
        }
    }
}

fn revive(vm: &mut Vm, holder: Value, key: Value, reviver: Value) -> Run<Value> {
    let atom = vm.to_atom(key)?;
    let value = vm.get(holder, atom)?;
    if value.is_object() {
        let names = vm.own_enumerable_names(value.handle());
        for name in names {
            let name_value = vm.atom_value(name);
            let revived = revive(vm, value, name_value, reviver)?;
            if revived.is_undefined() {
                vm.delete(value.handle(), name)?;
            } else {
                vm.define(value.handle(), name, revived, PLAIN);
            }
        }
    }
    vm.call(reviver, holder, &[key, value])
}

fn read_document<U: Unit>(vm: &mut Vm, units: &[U]) -> Run<Value> {
    let mut reader = Reader {
        units,
        at: 0,
        mark: vm.nursery.len(),
        keys: [(0, u32::MAX, Atom(0)); KEY_WAYS],
    };
    let value = reader.value(vm, 0)?;
    reader.skip_space();
    if reader.peek().is_some() {
        return Err(reader.error(vm));
    }
    vm.nursery.truncate(reader.mark);
    vm.nursery.push(value);
    Ok(value)
}

fn parse(vm: &mut Vm, args: Args) -> Run<Value> {
    let source = vm.arg(&args, 0);
    let source = vm.to_js_string(source)?;
    let text = vm.text_of(source).clone();
    let value = match (text.ascii(), text.wide()) {
        (Some(bytes), _) => read_document(vm, bytes)?,
        (_, units) => read_document(vm, units.unwrap_or(&[]))?,
    };
    drop(text);
    let reviver = vm.arg(&args, 1);
    if vm.is_callable(reviver) {
        let root = vm.new_plain();
        let key = vm.new_str("");
        let atom = vm.to_atom(key)?;
        vm.define(root.handle(), atom, value, PLAIN);
        return revive(vm, root, key, reviver);
    }
    Ok(value)
}

/// The name of what's being written. For an array's elements it's a string that almost nothing asks for.
#[derive(Clone, Copy)]
enum Key {
    Index(usize),
    Made(Value),
    Name(Atom),
}

impl Key {
    fn make(self, vm: &mut Vm) -> Value {
        match self {
            Self::Index(index) => vm.new_str(&index.to_string()),
            Self::Made(value) => value,
            Self::Name(atom) => vm.atom_value(atom),
        }
    }
}

/// What's been written. It stays as bytes for as long as it's all ASCII.
#[derive(Default)]
pub(super) struct Out {
    ascii: Vec<u8>,
    wide: Vec<u16>,
    is_wide: bool,
    /// Whether some was taken off after it went wide, which could have taken off all that wasn't ASCII.
    cut_since_widening: bool,
}

impl Out {
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            ascii: Vec::with_capacity(capacity),
            ..Self::default()
        }
    }

    pub(super) fn len(&self) -> usize {
        if self.is_wide {
            self.wide.len()
        } else {
            self.ascii.len()
        }
    }

    #[inline]
    pub(super) fn push(&mut self, unit: u16) {
        if self.is_wide {
            self.wide.push(unit);
        } else if unit < 128 {
            self.ascii.push(unit as u8);
        } else {
            self.widen();
            self.wide.push(unit);
        }
    }

    #[cold]
    fn widen(&mut self) {
        self.wide = self.ascii.iter().map(|byte| u16::from(*byte)).collect();
        self.ascii = Vec::new();
        self.is_wide = true;
    }

    pub(super) fn push_units(&mut self, units: &[u16]) {
        if self.is_wide {
            self.wide.extend_from_slice(units);
            return;
        }
        for unit in units {
            self.push(*unit);
        }
    }

    /// The units from `start` to `end` of some text.
    pub(super) fn push_part(&mut self, text: &Text, start: usize, end: usize) {
        match (text.ascii(), text.wide()) {
            (Some(bytes), _) => self.push_ascii_bytes(&bytes[start..end]),
            (_, units) => self.push_units(&units.unwrap_or(&[])[start..end]),
        }
    }

    pub(super) fn push_text(&mut self, text: &Text) {
        match (text.ascii(), text.wide()) {
            (Some(bytes), _) => self.push_ascii_bytes(bytes),
            (_, units) => self.push_units(units.unwrap_or(&[])),
        }
    }

    #[inline]
    fn push_ascii(&mut self, text: &str) {
        if self.is_wide {
            self.wide.extend(text.bytes().map(u16::from));
        } else {
            self.ascii.extend_from_slice(text.as_bytes());
        }
    }

    fn truncate(&mut self, length: usize) {
        if self.is_wide {
            self.wide.truncate(length);
            self.cut_since_widening = true;
        } else {
            self.ascii.truncate(length);
        }
    }

    #[inline]
    pub(super) fn push_ascii_bytes(&mut self, bytes: &[u8]) {
        if self.is_wide {
            self.wide.extend(bytes.iter().map(|byte| u16::from(*byte)));
        } else {
            self.ascii.extend_from_slice(bytes);
        }
    }

    pub(super) fn into_text(self) -> Text {
        if self.is_wide && self.cut_since_widening {
            Text::from_units(&self.wide)
        } else if self.is_wide {
            // It only went wide for a unit that isn't ASCII, and that's still there, so there's nothing to look for.
            Text::Wide(self.wide.into_boxed_slice())
        } else {
            Text::from_ascii_vec(self.ascii)
        }
    }
}

/// How to write a plain object of one shape: each key that's written, in order, where its value is kept (or
/// [`ACCESSOR_SLOT`] for a getter, which has to be called), and the key already quoted with its colon after it.
struct Plan {
    shape: u32,
    /// [`crate::object::Shapes::generation`] when this was made: once shapes have been freed, `shape` can be another's.
    generation: u32,
    keys: Vec<(Atom, u32)>,
    quoted: Vec<u8>,
    /// Where each key's quoted text ends in `quoted`.
    ends: Vec<u32>,
}

const ACCESSOR_SLOT: u32 = u32::MAX;

/// Which bytes in a string written as JSON need an escape: quotes, backslashes and control characters.
static NEEDS_ESCAPE: [bool; 128] = {
    let mut table = [false; 128];
    let mut at = 0;
    while at < 32 {
        table[at] = true;
        at += 1;
    }
    table[b'"' as usize] = true;
    table[b'\\' as usize] = true;
    table
};

struct Writer {
    out: Out,
    /// Shapes that were found to have no `toJSON`, and when that was, as the epoch and the shapes' generation then (a
    /// collection that frees shapes can give their places to others), so most objects needn't look. A document mostly
    /// has only a few kinds of object in it.
    no_to_json: [(u32, u32, u32); 8],
    /// How to write the objects of each shape that's been seen, or nothing for one that has to be written the long way.
    plans: HashMap<u32, Option<Rc<Plan>>>,
    /// [`crate::object::Shapes::generation`] when those were found. A collection that frees shapes can give their
    /// places to others, as what's written can run code, and they're forgotten then.
    generation: u32,
    gap: Vec<u16>,
    indent: Vec<u16>,
    replacer: Value,
    allowed: Option<Vec<crate::text::Atom>>,
    stack: Vec<Value>,
}

/// Writes some text in quotes.
fn quote(out: &mut Out, text: &Text) {
    out.push(34);
    match (text.ascii(), text.wide()) {
        (Some(bytes), _) => {
            let mut from = 0;
            for (at, byte) in bytes.iter().enumerate() {
                if NEEDS_ESCAPE[usize::from(*byte & 127)] {
                    out.push_ascii_bytes(&bytes[from..at]);
                    quote_unit(out, u16::from(*byte));
                    from = at + 1;
                }
            }
            out.push_ascii_bytes(&bytes[from..]);
        }
        (_, units) => {
            let units = units.unwrap_or(&[]);
            for (index, unit) in units.iter().enumerate() {
                match *unit {
                    0xD800..=0xDBFF
                        if units
                            .get(index + 1)
                            .is_some_and(|next| (0xDC00..0xE000).contains(next)) =>
                    {
                        out.push(*unit);
                    }
                    0xDC00..=0xDFFF
                        if index > 0 && (0xD800..0xDC00).contains(&units[index - 1]) =>
                    {
                        out.push(*unit);
                    }
                    _ => quote_unit(out, *unit),
                }
            }
        }
    }
    out.push(34);
}

fn quote_unit(out: &mut Out, unit: u16) {
    match unit {
        34 => out.push_ascii("\\\""),
        92 => out.push_ascii("\\\\"),
        8 => out.push_ascii("\\b"),
        12 => out.push_ascii("\\f"),
        10 => out.push_ascii("\\n"),
        13 => out.push_ascii("\\r"),
        9 => out.push_ascii("\\t"),
        0..=31 | 0xD800..=0xDFFF => out.push_ascii(&format!("\\u{unit:04x}")),
        _ => out.push(unit),
    }
}

impl Writer {
    /// Writes a value. It says if there was anything to write, as a function has no JSON form.
    fn value(&mut self, vm: &mut Vm, holder: Value, key: Key, value: Value) -> Run<bool> {
        let mut value = value;
        if value.is_object() {
            let shape = vm.objects[value.index()].shape;
            let way = shape as usize % self.no_to_json.len();
            if self.no_to_json[way] != (shape, vm.epoch, vm.shapes.generation) {
                let method = vm.get(value, text::TO_JSON)?;
                if vm.is_callable(method) {
                    let key = key.make(vm);
                    value = vm.call(method, value, &[key])?;
                } else if method.is_undefined()
                    && matches!(vm.objects[value.index()].kind, Kind::Plain | Kind::Array(_))
                    && !vm.shapes.is_dict(shape)
                {
                    self.no_to_json[way] = (shape, vm.epoch, vm.shapes.generation);
                }
            }
        }
        if vm.is_callable(self.replacer) {
            let key = key.make(vm);
            value = vm.call(self.replacer, holder, &[key, value])?;
        }
        if value.is_object() {
            if let Kind::Boxed(inner) = vm.objects[value.index()].kind {
                value = if inner.is_number() {
                    let number = vm.to_number(value)?;
                    Value::number(number)
                } else if inner.is_string() {
                    vm.to_js_string(value)?
                } else {
                    inner
                };
            }
        }
        if value.is_bigint() {
            return Err(vm.type_error("Do not know how to serialize a BigInt"));
        }
        if value.is_null() {
            self.out.push_ascii("null");
        } else if value == Value::TRUE {
            self.out.push_ascii("true");
        } else if value == Value::FALSE {
            self.out.push_ascii("false");
        } else if value.is_string() {
            quote(&mut self.out, vm.text_of(value));
        } else if value.is_int() {
            let number = crate::convert::int_text(value.as_int());
            self.out.push_ascii_bytes(number.ascii().unwrap_or(&[]));
        } else if value.is_number() {
            let number = value.as_number();
            if number.is_finite() {
                self.out
                    .push_text(&crate::convert::number_text(number));
            } else {
                self.out.push_ascii("null");
            }
        } else if value.is_object() && !vm.is_callable(value) {
            if self.stack.contains(&value) {
                return Err(vm.type_error("Converting circular structure to JSON"));
            }
            if self.stack.len() > MAX_DEPTH as usize {
                return Err(vm.range_error("Maximum call stack size exceeded"));
            }
            self.stack.push(value);
            let is_array = vm.is_array_value(value);
            let result = if is_array {
                self.array(vm, value)
            } else {
                self.object(vm, value)
            };
            self.stack.pop();
            result?;
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    fn open(&mut self, opening: u16) -> Vec<u16> {
        self.out.push(opening);
        let saved = self.indent.clone();
        self.indent.extend_from_slice(&self.gap.clone());
        saved
    }

    fn line(&mut self) {
        if !self.gap.is_empty() {
            self.out.push(10);
            self.out.push_units(&self.indent);
        }
    }

    fn close(&mut self, closing: u16, saved: Vec<u16>, any: bool) {
        self.indent = saved;
        if any {
            self.line();
        }
        self.out.push(closing);
    }

    fn array(&mut self, vm: &mut Vm, value: Value) -> Run<()> {
        let saved = self.open(91);
        let length = vm.length_of(value)?;
        for index in 0..length {
            if index > 0 {
                self.out.push(44);
            }
            self.line();
            let item = vm.get_index(value, index)?;
            if !self.value(vm, value, Key::Index(index), item)? {
                self.out.push_ascii("null");
            }
        }
        self.close(93, saved, length > 0);
        Ok(())
    }

    /// Forgets the plans, if shapes have been freed since they were made. (What's in `no_to_json` goes by the
    /// generation too.)
    fn check_shapes(&mut self, vm: &Vm) {
        if self.generation != vm.shapes.generation {
            self.generation = vm.shapes.generation;
            self.plans.clear();
        }
    }

    /// The plan for writing objects of this one's shape, if they can be written that way.
    fn plan_for(&mut self, vm: &mut Vm, value: Value) -> Option<Rc<Plan>> {
        self.check_shapes(vm);
        let found = &vm.objects[value.index()];
        if self.allowed.is_some()
            || !matches!(found.kind, Kind::Plain)
            || found.flags & LAZY_FUNCTION != 0
            || vm.shapes.is_dict(found.shape)
        {
            return None;
        }
        let shape = found.shape;
        if let Some(plan) = self.plans.get(&shape) {
            return plan.clone();
        }
        let plan = make_plan(vm, shape).map(Rc::new);
        self.plans.insert(shape, plan.clone());
        plan
    }

    fn object_by_plan(&mut self, vm: &mut Vm, value: Value, plan: &Plan) -> Run<()> {
        let saved = self.open(123);
        let mut any = false;
        let mut from = 0;
        for (index, (atom, slot)) in plan.keys.iter().enumerate() {
            // What's written could change the object, and then its value is read the usual way.
            let object = &vm.objects[value.index()];
            let item = if object.shape == plan.shape
                && vm.shapes.generation == plan.generation
                && *slot != ACCESSOR_SLOT
            {
                object.slots()[*slot as usize]
            } else {
                vm.get_by_atom(value, *atom)?
            };
            let length_before = self.out.len();
            if any {
                self.out.push(44);
            }
            self.line();
            let end = plan.ends[index] as usize;
            self.out.push_ascii_bytes(&plan.quoted[from..end]);
            from = end;
            if !self.gap.is_empty() {
                self.out.push(32);
            }
            if self.value(vm, value, Key::Name(*atom), item)? {
                any = true;
            } else {
                self.out.truncate(length_before);
            }
        }
        self.close(125, saved, any);
        Ok(())
    }

    fn object(&mut self, vm: &mut Vm, value: Value) -> Run<()> {
        if let Some(plan) = self.plan_for(vm, value) {
            return self.object_by_plan(vm, value, &plan);
        }
        let saved = self.open(123);
        let quick_names = if self.allowed.is_none() {
            vm.plain_object_keys(value.handle(), false)
        } else {
            None
        };
        let names = match (quick_names, &self.allowed) {
            (Some(keys), _) => keys,
            (None, Some(allowed)) => allowed
                .clone()
                .into_iter()
                .map(|atom| vm.atom_value(atom))
                .collect(),
            (None, None) => vm
                .own_enumerable_names(value.handle())
                .into_iter()
                .map(|atom| vm.atom_value(atom))
                .collect(),
        };
        let mut any = false;
        for key in names {
            let name = vm.to_atom(key)?;
            let item = vm.get_by_atom(value, name)?;
            let length_before = self.out.len();
            if any {
                self.out.push(44);
            }
            self.line();
            quote(&mut self.out, vm.text_of(key));
            let key = Key::Made(key);
            self.out.push(58);
            if !self.gap.is_empty() {
                self.out.push(32);
            }
            if self.value(vm, value, key, item)? {
                any = true;
            } else {
                self.out.truncate(length_before);
            }
        }
        self.close(125, saved, any);
        Ok(())
    }
}

/// Works out how to write objects of a shape, if they're simple enough: no keys that are numbers, which go first, and
/// keys that are all ASCII.
fn make_plan(vm: &mut Vm, shape: u32) -> Option<Plan> {
    let mut plan = Plan {
        shape,
        generation: vm.shapes.generation,
        keys: Vec::new(),
        quoted: Vec::new(),
        ends: Vec::new(),
    };
    for (key, slot, flags) in vm.shapes.entries(shape) {
        if vm.atoms.index_of(key).is_some() {
            return None;
        }
        if flags & ENUMERABLE == 0 || vm.atoms.kind_of(key) != AtomKind::Name {
            continue;
        }
        let name = vm.atom_value(key);
        let mut quoted = Out::default();
        quote(&mut quoted, vm.text_of(name));
        if quoted.is_wide {
            return None;
        }
        plan.quoted.extend_from_slice(&quoted.ascii);
        plan.quoted.push(b':');
        plan.ends.push(plan.quoted.len() as u32);
        let slot = if flags & ACCESSOR != 0 {
            ACCESSOR_SLOT
        } else {
            slot
        };
        plan.keys.push((key, slot));
    }
    Some(plan)
}

fn stringify(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let replacer = vm.arg(&args, 1);
    let space = vm.arg(&args, 2);
    let gap = if space.is_number() {
        let count = space.as_number().clamp(0.0, 10.0) as usize;
        vec![32u16; count]
    } else if space.is_string() {
        let mut units = vm.text_of(space).units();
        units.truncate(10);
        units
    } else {
        Vec::new()
    };
    let allowed =
        if replacer.is_object() && matches!(vm.objects[replacer.index()].kind, Kind::Array(_)) {
            let mut list = Vec::new();
            for item in vm.array_items(replacer) {
                let atom = vm.to_atom(item)?;
                if vm.atoms.kind_of(atom) == AtomKind::Name && !list.contains(&atom) {
                    list.push(atom);
                }
            }
            Some(list)
        } else {
            None
        };
    let mut writer = Writer {
        out: Out::default(),
        no_to_json: [(NONE, 0, 0); 8],
        plans: HashMap::new(),
        generation: vm.shapes.generation,
        gap,
        indent: Vec::new(),
        replacer,
        allowed,
        stack: Vec::new(),
    };
    let holder = vm.new_plain();
    let empty_key = vm.new_str("");
    let empty_atom = vm.to_atom(empty_key)?;
    vm.define(holder.handle(), empty_atom, value, PLAIN);
    if !writer.value(vm, holder, Key::Made(empty_key), value)? {
        return Ok(Value::UNDEFINED);
    }
    Ok(vm.new_text(writer.out.into_text()))
}
