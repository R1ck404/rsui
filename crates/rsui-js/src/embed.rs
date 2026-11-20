//! What a Rust program uses to work with the engine: reading and making values, calling functions, and holding on to
//! values between calls.
//!
//! A [`Value`] on its own is only safe until JavaScript next runs, as the collector doesn't know Rust has it. To keep one
//! for longer, make it a [`Root`].

use std::borrow::Cow;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::object::*;
use crate::text::{Atom, AtomKind};
use crate::value::Value;
use crate::vm::{Args, Vm};

#[path = "data_surface.rs"]
mod data_surface;
pub use data_surface::{DataProperty, DataSurfaceError};
pub(crate) use data_surface::SelectorIntrinsic;

/// What a Rust function looks like to JavaScript.
pub type HostFunction = fn(&mut Vm, Args) -> Result<Value, Value>;

thread_local! {
    /// Host functions made while `RSUI_SPLIT` is set, each with its part, by the number [`timed_host`] is made with.
    static TIMED_HOSTS: RefCell<Vec<(HostFunction, u16)>> = const { RefCell::new(Vec::new()) };
}

/// A host function, with its time charged to its own part.
fn timed_host(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let at = vm.native_data_of(&args).as_number() as usize;
    let (function, part) = TIMED_HOSTS.with(|hosts| hosts.borrow()[at]);
    let _split = rsui_alloc::split::enter(part);
    function(vm, args)
}

#[derive(Default)]
pub(crate) struct Held {
    /// Each value, with how many `Root`s there are of it. A free place has a count of zero.
    slots: RefCell<Vec<(Value, u32)>>,
    free: RefCell<Vec<u32>>,
}

impl Held {
    /// How many places there are, how many of them are free, and how many there's room for.
    pub(crate) fn counts(&self) -> (usize, usize, usize) {
        let slots = self.slots.borrow();
        (slots.len(), self.free.borrow().len(), slots.capacity())
    }

    /// Every value being held, for the collector.
    pub(crate) fn each(&self, mut visit: impl FnMut(Value)) {
        for (value, count) in self.slots.borrow().iter() {
            if *count > 0 {
                visit(*value);
            }
        }
    }
}

/// A value that stays alive for as long as this does. Cloning one is cheap.
pub struct Root {
    table: Rc<Held>,
    slot: u32,
}

impl Clone for Root {
    fn clone(&self) -> Self {
        self.table.slots.borrow_mut()[self.slot as usize].1 += 1;
        Self {
            table: self.table.clone(),
            slot: self.slot,
        }
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let mut slots = self.table.slots.borrow_mut();
        let entry = &mut slots[self.slot as usize];
        entry.1 -= 1;
        if entry.1 == 0 {
            entry.0 = Value::UNDEFINED;
            self.table.free.borrow_mut().push(self.slot);
        }
    }
}

impl Root {
    /// The value being held.
    #[inline]
    pub fn get(&self) -> Value {
        self.table.slots.borrow()[self.slot as usize].0
    }
}

impl std::fmt::Debug for Root {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Root({:?})", self.get())
    }
}

impl Args {
    /// The argument at this position, or undefined if there aren't that many.
    #[inline]
    pub fn get(&self, vm: &Vm, index: usize) -> Value {
        vm.arg(self, index)
    }

    /// What `this` is.
    #[inline]
    pub fn this(&self) -> Value {
        self.this
    }

    /// How many arguments there are.
    #[inline]
    pub fn count(&self) -> usize {
        self.count
    }
}

impl Vm {
    /// The files that have been run as modules so far, for a host function that wants to know what the program is made of.
    pub fn loaded_files(&self) -> Vec<PathBuf> {
        self.modules.loaded_files()
    }

    /// How many [`loaded_files`](Self::loaded_files) there are, without making the list.
    pub fn loaded_file_count(&self) -> usize {
        self.modules.loaded_file_count()
    }

    /// Where `require` would find `name` from `dir`, with packages' `exports` read with these conditions as well as the usual
    /// ones, as a bundler finds a package's stylesheet with `style`. It's `None` if there's nothing there.
    pub fn resolve_with_conditions(&mut self, dir: &std::path::Path, name: &str, conditions: &[&str]) -> Option<PathBuf> {
        let usual = std::mem::replace(
            &mut self.modules.conditions,
            conditions.iter().map(|one| (*one).to_owned()).collect(),
        );
        let found = self.resolve_file(dir, name, false);
        self.modules.conditions = usual;
        found.ok().map(|found| crate::modules::canonical(&found))
    }

    /// Holds on to a value for as long as the [`Root`] lasts.
    pub fn root(&mut self, value: Value) -> Root {
        let slot = if let Some(slot) = self.held.free.borrow_mut().pop() {
            self.held.slots.borrow_mut()[slot as usize] = (value, 1);
            slot
        } else {
            let mut slots = self.held.slots.borrow_mut();
            slots.push((value, 1));
            (slots.len() - 1) as u32
        };
        Root {
            table: self.held.clone(),
            slot,
        }
    }

    /// The name of a property, which is quicker to use than its text. It stays for as long as the engine does, so it
    /// can be kept. An atom from anywhere else, such as [`properties`](Self::properties), is only good until JavaScript
    /// next runs, as a collection then can free one that nothing uses, and give its number to another.
    pub fn atom(&mut self, name: &str) -> Atom {
        self.intern_str(name)
    }

    /// Goes up whenever a collection frees atoms, so whatever Rust remembers by an atom's [`number`](Atom::number) has
    /// to be forgotten when this changes, other than what's about atoms from [`atom`](Self::atom).
    pub fn atom_generation(&self) -> u32 {
        self.atoms.generation
    }

    /// The text of a property's name.
    pub fn atom_name(&self, atom: Atom) -> String {
        self.atom_text(atom)
    }

    /// Reads a property, as `object.name` does.
    pub fn get_property(&mut self, object: Value, name: Atom) -> Result<Value, Value> {
        self.get_by_atom(object, name)
    }

    /// Writes a property, as `object.name = value` does.
    pub fn set_property(&mut self, object: Value, name: Atom, value: Value) -> Result<(), Value> {
        self.set(object, name, value)
    }

    /// Whether this is an object an object literal makes (or one with no prototype), rather than an array, a function or any
    /// other kind, whose properties are all there is to it.
    pub fn is_plain_object(&self, value: Value) -> bool {
        if !value.is_object() {
            return false;
        }
        let object = &self.objects[value.index()];
        if !matches!(object.kind, Kind::Plain) {
            return false;
        }
        let proto = self.shapes.proto_of(object.shape);
        proto == self.realm.object_proto || proto == NONE
    }

    /// Takes a property out, as `delete object.name` does.
    pub fn delete_property(&mut self, object: Value, name: Atom) -> Result<bool, Value> {
        if !object.is_object() {
            return Ok(true);
        }
        self.delete(object.handle(), name)
    }

    /// What a value is as the name of a property, as `object[key]` takes it: a symbol is its own.
    pub fn property_atom(&mut self, key: Value) -> Result<Atom, Value> {
        self.to_atom(key)
    }

    /// Puts a value on the end of an array, as `array.push(value)` does on one that's plain.
    pub fn push_item(&mut self, array: Value, value: Value) -> Result<(), Value> {
        if let Some(Kind::Array(items)) = array.is_object().then(|| &mut self.objects[array.index()].kind) {
            items.push(value);
            self.debt += 8;
            return Ok(());
        }
        let push = self.intern_str("push");
        let method = self.get(array, push)?;
        self.call(method, array, &[value])?;
        Ok(())
    }

    /// Keeps a value alive until this host function gives back, as one it made itself would be, for one it's taken from where
    /// the program had it, as a property it's deleted, while it calls into JavaScript.
    pub fn hold(&mut self, value: Value) {
        if value.is_object() || value.is_string() || value.is_bigint() {
            self.nursery.push(value);
        }
    }

    /// Calls a function. Whatever it gives back is safe until this comes back to the top, as it's on the list of what
    /// natives are holding, unless [`release_new_values`](Self::release_new_values) gets rid of it first.
    pub fn call_function(
        &mut self,
        function: Value,
        this: Value,
        args: &[Value],
    ) -> Result<Value, Value> {
        let _split = rsui_alloc::split::enter(rsui_alloc::split::JS);
        let _span = rsui_spans::span(rsui_spans::Kind::Script);
        self.call(function, this, args)
    }

    /// Where the list of what natives are holding has got to, for
    /// [`release_new_values`](Self::release_new_values).
    #[inline]
    pub fn new_values_mark(&self) -> usize {
        self.nursery.len()
    }

    /// Stops holding what was made since this mark, once Rust has finished with those values.
    #[inline]
    pub fn release_new_values(&mut self, mark: usize) {
        self.nursery.truncate(mark);
    }

    /// A new string.
    pub fn new_string(&mut self, text: &str) -> Value {
        self.new_str(text)
    }

    /// A new object with no properties of its own.
    pub fn new_object(&mut self) -> Value {
        self.new_plain()
    }

    /// Runs something on another thread, and gives a promise for how it turns out: text, `null` if there's nothing to say,
    /// or an error. The host passes on what's finished by calling [`Engine::run_jobs`](crate::Engine::run_jobs).
    pub fn later(
        &mut self,
        job: impl FnOnce() -> Result<Option<String>, String> + Send + 'static,
    ) -> Value {
        self.in_background(move || match job() {
            Ok(Some(text)) => crate::background::Outcome::Text(text),
            Ok(None) => crate::background::Outcome::Null,
            Err(reason) => crate::background::Outcome::Failed(reason),
        })
    }

    /// A function that JavaScript can call.
    pub fn host_function(&mut self, name: &str, length: u8, function: HostFunction) -> Value {
        // With `RSUI_SPLIT` set, each host function's time is its own part: see `split.rs` in rsui-alloc.
        if rsui_alloc::split::on() {
            let part = rsui_alloc::split::named(&format!("host:{name}"));
            let at = TIMED_HOSTS.with(|hosts| {
                let mut hosts = hosts.borrow_mut();
                hosts.push((function, part));
                hosts.len() - 1
            });
            let name = self.intern_str(name);
            return self.new_native(name, length, timed_host, Value::number(at as f64), false);
        }
        let name = self.intern_str(name);
        self.new_native(name, length, function, Value::UNDEFINED, false)
    }

    /// Runs a module from [`Engine::add_module`](crate::Engine::add_module) if nothing has yet, and gives what it exports.
    pub fn load_host_module(&mut self, name: &str) -> Result<Value, Value> {
        match self.load_builtin_module(name)? {
            Some(module) => self.exports_of(module),
            None => Ok(Value::UNDEFINED),
        }
    }

    /// Makes a global variable.
    pub fn set_global(&mut self, name: &str, value: Value) {
        let name = self.intern_str(name);
        let global = self.global;
        self.define(global, name, value, HIDDEN);
    }

    /// Reads a global variable. It's undefined if there's no such thing.
    pub fn get_global(&mut self, name: &str) -> Result<Value, Value> {
        let name = self.intern_str(name);
        self.get(Value::object(self.global), name)
    }

    /// Whether this can be called.
    #[inline]
    pub fn is_function(&self, value: Value) -> bool {
        self.is_callable(value)
    }

    /// The text of a string, without copying it if that can be helped.
    pub fn string_text(&self, value: Value) -> Option<Cow<'_, str>> {
        if !value.is_string() {
            return None;
        }
        let text = self.text_of(value);
        Some(match text.ascii() {
            // ASCII is always valid.
            Some(bytes) => String::from_utf8_lossy(bytes),
            None => Cow::Owned(text.to_rust()),
        })
    }

    /// Finds packages as a bundler for a browser does, with `exports`' `browser` condition and the `browser` field of a
    /// `package.json`, for a program whose code is written for a page, so that a library's build for browsers is the one it gets.
    pub fn set_browser_packages(&mut self, on: bool) {
        self.modules.browser = on;
    }

    /// The modules that come with the engine, or that the program gave it, that have been started so far, in order.
    pub fn loaded_builtins(&self) -> Vec<String> {
        self.modules.loaded_builtins()
    }

    /// What `String(value)` would give for a number or a string, without running any of the program's code.
    pub fn plain_text(&mut self, value: Value) -> Option<String> {
        if value.is_string() {
            return Some(self.text_of(value).to_rust());
        }
        if value.is_int() {
            return Some(value.as_int().to_string());
        }
        if value.is_number() {
            return Some(crate::convert::number_to_string(value.as_number()));
        }
        None
    }

    /// Where the bytes an `ArrayBuffer` holds are, or the part of one that a typed array or `DataView` looks at, as the buffer
    /// and the range in it. `None` for anything else, or a buffer that's been given away.
    fn byte_range(&self, value: Value) -> Option<(Value, std::ops::Range<usize>)> {
        if !value.is_object() {
            return None;
        }
        let (buffer, start, length) = match &self.objects[value.index()].kind {
            Kind::ArrayBuffer(data) => (value, 0, data.bytes.len()),
            Kind::TypedArray(data) => (data.buffer, data.offset, self.typed_length(data) * data.element.size()),
            Kind::DataView(view) => {
                let total = self.buffer_of(view.buffer)?.bytes.len();
                let length = if view.follows_buffer { total.saturating_sub(view.offset) } else { view.length };
                (view.buffer, view.offset, length)
            }
            _ => return None,
        };
        let data = self.buffer_of(buffer)?;
        if data.detached || start + length > data.bytes.len() {
            return None;
        }
        Some((buffer, start..start + length))
    }

    /// The bytes an `ArrayBuffer` holds, or that a typed array or `DataView` looks at, which is how a host function reads what
    /// a program put in one without copying it.
    pub fn bytes(&self, value: Value) -> Option<&[u8]> {
        let (buffer, range) = self.byte_range(value)?;
        Some(&self.buffer_of(buffer)?.bytes[range])
    }

    /// The same, to change them, as a host function that fills in what the program reads does.
    pub fn bytes_mut(&mut self, value: Value) -> Option<&mut [u8]> {
        let (buffer, range) = self.byte_range(value)?;
        match &mut self.objects[buffer.index()].kind {
            Kind::ArrayBuffer(data) => Some(&mut data.bytes[range]),
            _ => None,
        }
    }

    /// A new `ArrayBuffer` holding these bytes.
    pub fn new_buffer(&mut self, bytes: Vec<u8>) -> Value {
        self.new_array_buffer(bytes)
    }

    /// The elements of an array. A hole reads as a value that's none of the kinds there are, so skip what isn't wanted.
    pub fn array_elements(&self, value: Value) -> Option<&[Value]> {
        if !value.is_object() {
            return None;
        }
        match &self.objects[value.index()].kind {
            Kind::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The same as [`properties`](Self::properties), into a list that can be used again. This runs for
    /// everything drawn, and making a list each time was a good part of what that costs.
    pub fn properties_into(
        &mut self,
        object: Value,
        out: &mut Vec<(Atom, Value)>,
    ) -> Result<(), Value> {
        out.clear();
        if !object.is_object() {
            return Ok(());
        }
        let found = &self.objects[object.index()];
        if matches!(found.kind, Kind::Plain) && self.shapes.is_all_plain(found.shape) {
            let slots = found.slots();
            let atoms = &self.atoms;
            let backwards = self.shapes.visit_entries(found.shape, |key, slot| {
                if atoms.kind_of(key) == AtomKind::Name {
                    out.push((key, slots[slot as usize]));
                }
            });
            if backwards {
                out.reverse();
            }
            return Ok(());
        }
        *out = self.properties(object)?;
        Ok(())
    }

    /// An object's own properties that a loop would find, with their values.
    pub fn properties(&mut self, object: Value) -> Result<Vec<(Atom, Value)>, Value> {
        if !object.is_object() {
            return Ok(Vec::new());
        }
        let found = &self.objects[object.index()];
        if matches!(found.kind, Kind::Plain) && self.shapes.is_all_plain(found.shape) {
            let entries = self.shapes.entries(found.shape);
            let slots = self.objects[object.index()].slots();
            return Ok(entries
                .into_iter()
                .filter(|(key, _, _)| self.atoms.kind_of(*key) == AtomKind::Name)
                .map(|(key, slot, _)| (key, slots[slot as usize]))
                .collect());
        }
        let names = self.own_enumerable_names(object.handle());
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            let value = self.get(object, name)?;
            out.push((name, value));
        }
        Ok(out)
    }

    /// Whether two values are the same, as `Object.is` says.
    pub fn same(&self, a: Value, b: Value) -> bool {
        self.same_value(a, b)
    }

    /// Whether two objects have the same properties holding the same things, as a component's props are compared. This
    /// never runs any of the program's code, and objects that were built differently count as different.
    pub fn shallow_equal(&self, a: Value, b: Value) -> bool {
        if a == b {
            return true;
        }
        if !a.is_object() || !b.is_object() {
            return self.same_value(a, b);
        }
        let (first, second) = (&self.objects[a.index()], &self.objects[b.index()]);
        if !matches!(first.kind, Kind::Plain)
            || !matches!(second.kind, Kind::Plain)
            || first.shape != second.shape
            || self.shapes.is_dict(first.shape)
        {
            return false;
        }
        first
            .slots()
            .iter()
            .zip(second.slots())
            .all(|(x, y)| x == y || self.same_value(*x, *y))
    }

    /// Whether nothing an object holds is an object or a function itself, so that nothing it holds can say something else later
    /// without being given anew. Something that isn't an object holds nothing. An object that wasn't built plainly isn't known to.
    pub fn holds_no_objects(&self, value: Value) -> bool {
        if !value.is_object() {
            return true;
        }
        let object = &self.objects[value.index()];
        matches!(object.kind, Kind::Plain)
            && !self.shapes.is_dict(object.shape)
            && object.slots().iter().all(|slot| !slot.is_object())
    }

    /// Whether a component given the props `new`, after `old`, would show and do the same, so that it needn't render again, as
    /// React Compiler has it by keeping the element it made. Each prop is the same value, or does the same thing: a function
    /// made from the same code that sees the same values (`() => select(i)` made again with the same `select` and `i`), the
    /// same function given the same things by `bind`, or a plain object or array made again holding what's the same in that
    /// way, as an element or a `style` written in place is. Nothing more than [`EQUIVALENT_LOOKS`] values deep, or
    /// [`EQUIVALENT_DEPTH`] down, is looked at, and anything past that isn't the same.
    ///
    /// The same object is the same, as it is for React's `memo`, and so it's for code that doesn't change what it's given,
    /// as React's rules have it: rsui-tsx only asks this of what an app's own JSX made (see `Parser::jsx_name`), and of what
    /// says `memo`. With `cautious`, for what didn't say `memo`, the same object isn't the same even as itself when it holds a
    /// function, or `current`, as a ref does: what's in a ref changes without a render, and an object with methods can say
    /// something else when they're called, as TanStack Table's `row.getIsSelected()` does, while a component can read either
    /// as it renders. What a closure sees is still the same as itself, as that's only read when it's called. An object holding
    /// `opaque` is only the same as itself too, which is how rsui-tsx says an element wasn't made by the app's own JSX, so
    /// that what's in it is only taken to be what it was when it's the same element.
    ///
    /// This never runs any of the program's code: a getter, or anything that isn't a plain object, an array or a function, is
    /// only the same as itself.
    pub fn props_equivalent(&self, old: Value, new: Value, cautious: bool, opaque: Value) -> bool {
        self.props_equivalent_in(old, new, cautious, &mut Looks::new(EQUIVALENT_LOOKS, opaque))
    }

    /// [`props_equivalent`](Self::props_equivalent), and also a handler, a prop named as `onClick` is, anywhere in the props or
    /// in the elements and objects they hold, that's made from the same code as before but sees something else, as one
    /// calling a function that sees the selection does. That's what `useEvent`, or a ref holding the latest, does by hand:
    /// what the component has is then made to see what the new one does (its captures are the new one's, in the same place,
    /// so that it's the same function doing what the new one would), and so whatever has it, as what it drew and its effects,
    /// calls what the new render made. That's only for a function written in the app's own files, which isn't async or a
    /// generator, as one of those can be waiting part of the way through, and whose own properties nobody's asked for.
    ///
    /// It's for what React's rules say a handler is: something called when something happens, not while rendering, as a
    /// component that called `onChange` as it rendered would have shown what the old one gave. What else holds the old
    /// function, as a ref the parent kept it in to call later, calls the new one too.
    pub fn props_equivalent_handing_on(&mut self, old: Value, new: Value, cautious: bool, opaque: Value) -> bool {
        let mut looks = Looks::new(EQUIVALENT_LOOKS, opaque);
        looks.hand_on = true;
        if !self.props_equivalent_in(old, new, cautious, &mut looks) {
            return false;
        }
        self.hand_on_and_look_again(&mut looks);
        true
    }

    /// [`props_equivalent_handing_on`](Self::props_equivalent_handing_on) for an element of a page: the same object it's given
    /// is the same, as a ref it's attached to or a `style`, but what's in its `children` is compared cautiously, as it would be
    /// if a component the app's JSX made was given it.
    pub fn page_props_equivalent_handing_on(&mut self, old: Value, new: Value, opaque: Value) -> bool {
        let mut looks = Looks::new(EQUIVALENT_LOOKS, opaque);
        looks.hand_on = true;
        looks.cautious_in = Some(self.atom("children"));
        if !self.props_equivalent_in(old, new, false, &mut looks) {
            return false;
        }
        self.hand_on_and_look_again(&mut looks);
        true
    }

    /// Hands on what `looks` noted, and has what read a function handed on as it rendered looked at again (see
    /// `reads.rs`), as that might now read something else.
    fn hand_on_and_look_again(&mut self, looks: &mut Looks) {
        let handed = self.hand_on(looks);
        self.look_again_at_readers(crate::reads::NO_READS, handed, looks.opaque);
    }

    /// Makes each old handler noted in `looks` do what the new one does: see
    /// [`props_equivalent_handing_on`](Self::props_equivalent_handing_on).
    /// Each old one comes to hold the new one's captured variables, with whatever held the same ones as it (see
    /// `Vm::join_handed`), so that whatever's handed on to any of them later reaches all of them. What it hands on that now
    /// sees something else is given back, as only what read one of those can read something else.
    pub(crate) fn hand_on(&mut self, looks: &mut Looks) -> Vec<(Value, Value)> {
        let mut handed = std::mem::take(&mut looks.handed);
        handed.retain(|&(old, new)| self.join_handed(old, new));
        handed
    }

    fn props_equivalent_in(&self, old: Value, new: Value, cautious: bool, looks: &mut Looks) -> bool {
        if old == new {
            return true;
        }
        if !old.is_object() || !new.is_object() {
            return self.same_value(old, new);
        }
        let (first, second) = (&self.objects[old.index()], &self.objects[new.index()]);
        if !matches!(first.kind, Kind::Plain)
            || !matches!(second.kind, Kind::Plain)
            || first.shape != second.shape
            || !self.plain_shape(first.shape)
        {
            return false;
        }
        let Some(inside) = looks.cautious_in.take() else {
            return self.same_slots(first, second, EQUIVALENT_DEPTH, looks, cautious, false);
        };
        // Each prop by itself, the one named `inside` cautiously.
        let opaque = looks.opaque;
        for (at, (x, y)) in first.slots().iter().zip(second.slots()).enumerate() {
            if *x == opaque || *y == opaque {
                return false;
            }
            let careful = cautious || self.shapes.key_at(first.shape, at as u32) == Some(inside);
            let handed = looks.handed.len();
            if self.equivalent(*x, *y, EQUIVALENT_DEPTH, looks, careful) {
                continue;
            }
            looks.handed.truncate(handed);
            if !(looks.hand_on && self.hands_on(first.shape, at as u32, *x, *y, looks)) {
                return false;
            }
        }
        true
    }

    /// Whether two objects of the same shape hold what's equivalent in each place, for
    /// [`equivalent`](Self::equivalent), each looked at `below` down. A function's own properties, as its `name`, aren't
    /// plain ones, as they're not enumerable, but they hold values.
    fn same_slots(
        &self,
        first: &Object,
        second: &Object,
        below: u32,
        looks: &mut Looks,
        cautious: bool,
        of_function: bool,
    ) -> bool {
        if first.shape != second.shape
            || self.shapes.is_dict(first.shape)
            || !(of_function || self.shapes.is_all_plain(first.shape))
        {
            return false;
        }
        let opaque = looks.opaque;
        for (at, (x, y)) in first.slots().iter().zip(second.slots()).enumerate() {
            if *x == opaque || *y == opaque {
                return false;
            }
            let handed = looks.handed.len();
            if self.equivalent(*x, *y, below, looks, cautious) {
                continue;
            }
            // What was to be handed on inside what turned out not to be the same isn't.
            looks.handed.truncate(handed);
            if !(looks.hand_on && !of_function && self.hands_on(first.shape, at as u32, *x, *y, looks)) {
                return false;
            }
        }
        true
    }

    /// Whether `old` and `new`, in the place `at` of objects of `shape`, are a handler that can be handed on: see
    /// [`props_equivalent_handing_on`](Self::props_equivalent_handing_on). If so it's noted, to be done once everything else
    /// is the same.
    fn hands_on(&self, shape: u32, at: u32, old: Value, new: Value, looks: &mut Looks) -> bool {
        let named = self.shapes.key_at(shape, at).is_some_and(|key| {
            let name = self.atom_text(key);
            let name = name.as_bytes();
            name.len() > 2 && name.starts_with(b"on") && name[2].is_ascii_uppercase()
        });
        // A function named otherwise, as `renderRow` or `pane`, can be called as something renders, and is only handed on
        // when each such call's reads were noted, so that what made them can be looked at again (see `reads.rs`).
        let can = if named { self.handler_pair(old, new) } else { self.function_pair(old, new) };
        can && self.note_handed(old, new, looks)
    }

    /// Whether a component function says `'use no memo'`, or is a class, which renders whenever what it's in does, as it
    /// would in React, unless what it's given is the same plain values.
    pub fn says_no_memo(&self, function: Value) -> bool {
        if !function.is_object() {
            return false;
        }
        match &self.objects[function.index()].kind {
            Kind::Function { proto, .. } => {
                let proto = &self.protos[*proto as usize];
                proto.no_memo() || proto.flags & crate::op::IS_CLASS != 0
            }
            _ => false,
        }
    }

    /// Whether a function's code is in one of the app's own files, rather than in `node_modules` or what comes with the engine.
    pub fn made_by_app(&self, function: Value) -> bool {
        if !function.is_object() {
            return false;
        }
        match &self.objects[function.index()].kind {
            Kind::Function { proto, .. } => self.protos[*proto as usize].says & crate::op::SAYS_APP_FILE != 0,
            _ => false,
        }
    }

    /// Whether objects of this shape only have properties that hold values, with nothing for a getter to work out.
    fn plain_shape(&self, shape: u32) -> bool {
        !self.shapes.is_dict(shape) && self.shapes.is_all_plain(shape)
    }

    /// Whether this is a plain object with `current` and at most one more property, which is what a ref is.
    fn holds_current(&self, value: Value) -> bool {
        if !value.is_object() {
            return false;
        }
        let object = &self.objects[value.index()];
        matches!(object.kind, Kind::Plain)
            && object.slots().len() <= 2
            && self
                .shapes
                .entries(object.shape)
                .iter()
                .any(|(key, _, _)| self.atom_text(*key) == "current")
    }

    /// Whether a plain object or an array holds a function, or is a ref, so that it can say something else without being changed
    /// or given anew.
    fn can_change_unseen(&self, value: Value) -> bool {
        if !value.is_object() {
            return false;
        }
        let held: &[Value] = match &self.objects[value.index()].kind {
            Kind::Plain => self.objects[value.index()].slots(),
            Kind::Array(items) => items,
            _ => return false,
        };
        held.iter().any(|one| self.is_callable(*one)) || self.holds_current(value)
    }

    /// Whether two values would do the same, for [`props_equivalent`](Self::props_equivalent), looking at no more than
    /// `looks` objects and `depth` down, `cautious` as that says.
    pub(crate) fn equivalent(&self, a: Value, b: Value, depth: u32, looks: &mut Looks, cautious: bool) -> bool {
        if a == b || self.same_value(a, b) {
            return !(cautious && self.can_change_unseen(a));
        }
        if depth == 0 || looks.left == 0 || !a.is_object() || !b.is_object() {
            return false;
        }
        looks.left -= 1;
        // What's already being compared, further up, is taken to be the same here, so that what refers to itself, as an object
        // of handlers that each see the object, is the same when everything else in it is, rather than running out of depth
        // going round. That's sound for what's compared here, which only holds values: if nothing anywhere differs, they do the
        // same whatever path is taken through them.
        let open = looks.open;
        if looks.comparing[..open].contains(&(a, b)) {
            return true;
        }
        if open < looks.comparing.len() {
            looks.comparing[open] = (a, b);
            looks.open = open + 1;
        }
        let same = self.equivalent_inside(a, b, depth, looks, cautious);
        looks.open = open;
        same
    }

    /// What [`equivalent`](Self::equivalent) compares of two objects, once it's looking into them.
    fn equivalent_inside(&self, a: Value, b: Value, depth: u32, looks: &mut Looks, cautious: bool) -> bool {
        let (first, second) = (&self.objects[a.index()], &self.objects[b.index()]);
        let below = depth - 1;
        let same_slots =
            |looks: &mut Looks, of_function: bool| self.same_slots(first, second, below, looks, cautious, of_function);
        match (&first.kind, &second.kind) {
            // Only what an object literal makes, as an instance of a class can keep more than its properties say.
            (Kind::Plain, Kind::Plain) => {
                let proto = self.shapes.proto_of(first.shape);
                (proto == self.realm.object_proto || proto == NONE) && same_slots(looks, false)
            }
            (Kind::Array(x), Kind::Array(y)) => {
                x.len() == y.len()
                    && x.len() <= EQUIVALENT_ITEMS
                    && same_slots(looks, false)
                    && x
                        .iter()
                        .zip(y.iter())
                        .all(|(x, y)| self.equivalent(*x, *y, below, looks, cautious))
            }
            (
                Kind::Function {
                    proto,
                    home,
                    upvalues,
                    ..
                },
                Kind::Function {
                    proto: other_proto,
                    home: other_home,
                    upvalues: other_upvalues,
                    ..
                },
            ) => {
                if proto != other_proto || home != other_home || upvalues.len() != other_upvalues.len() {
                    return false;
                }
                // Its own properties are only made when they're asked for, so two that nobody's asked are the same.
                let untouched = |object: &Object| {
                    object.flags & LAZY_FUNCTION != 0 && object.slots().is_empty()
                };
                if !(untouched(first) && untouched(second)) && !same_slots(looks, true) {
                    return false;
                }
                // What a closure sees is either the value itself, of a variable that's never given another, or the cell
                // the variable is in, which is compared by what's in it now, as each render has cells of its own.
                let captures = &self.protos[*proto as usize].captures;
                let same = upvalues
                    .iter()
                    .zip(other_upvalues.iter())
                    .zip(captures.iter())
                    .all(|((x, y), capture)| {
                        if capture.by_value || x == y {
                            return self.equivalent(*x, *y, below, looks, false);
                        }
                        let (x, y) = (self.cells[x.index()], self.cells[y.index()]);
                        !x.is_empty() && !y.is_empty() && self.equivalent(x, y, below, looks, false)
                    });
                // Two that do the same are joined as handing on would, so that what keeps the old one, as a child that
                // skips does while what it's in keeps the new one, follows what either is handed later.
                if same && looks.hand_on && self.function_pair(a, b) {
                    return self.note_handed(a, b, looks);
                }
                same
            }
            (Kind::Bound(x), Kind::Bound(y)) => {
                x.args.len() == y.args.len()
                    && same_slots(looks, true)
                    && self.equivalent(x.target, y.target, below, looks, false)
                    && self.equivalent(x.this, y.this, below, looks, false)
                    && x
                        .args
                        .iter()
                        .zip(y.args.iter())
                        .all(|(x, y)| self.equivalent(*x, *y, below, looks, false))
            }
            _ => false,
        }
    }

    /// What an error says, with its stack if it has one.
    pub fn error_text(&mut self, error: Value) -> String {
        crate::builtins::inspect_value(self, error)
    }
}

/// What's left of what [`Vm::props_equivalent`] can look into, what says an object is only the same as itself, and the pairs
/// being compared further up, which are taken to be the same again inside.
pub(crate) struct Looks {
    left: u32,
    pub(crate) opaque: Value,
    comparing: [(Value, Value); EQUIVALENT_DEPTH as usize + 1],
    open: usize,
    /// Whether a handler made from the same code can be handed on, and those that are to be, the old with the new.
    pub(crate) hand_on: bool,
    pub(crate) handed: Vec<(Value, Value)>,
    /// A prop compared cautiously whatever the rest are, as a page element's `children` are.
    cautious_in: Option<Atom>,
}

impl Looks {
    pub(crate) fn new(left: u32, opaque: Value) -> Looks {
        Looks {
            left,
            opaque,
            comparing: [(Value::UNDEFINED, Value::UNDEFINED); EQUIVALENT_DEPTH as usize + 1],
            open: 0,
            hand_on: false,
            handed: Vec::new(),
            cautious_in: None,
        }
    }
}

/// How many objects [`Vm::props_equivalent`] looks into for one component's props, so that comparing costs much less than
/// rendering would, however much it's given. The file explorer's panes are given a function that sees an object of 40 handlers,
/// each seeing what the app's made of its state, which takes about 700.
pub(crate) const EQUIVALENT_LOOKS: u32 = 1024;
/// How far down [`Vm::props_equivalent`] looks: far enough for a prop holding an element whose props hold a function that
/// sees a function, as `children` and handlers made in place do, and for a page's `<div>` holding a component given a function
/// that sees the app's handlers, which see what they're made from (13 in the file explorer). What refers to itself takes no more
/// for it, as what's being compared further up is taken to be the same.
pub(crate) const EQUIVALENT_DEPTH: u32 = 16;
/// The longest array [`Vm::props_equivalent`] compares item by item.
const EQUIVALENT_ITEMS: usize = 64;

impl crate::Engine {
    /// The engine itself, for what's in this module.
    #[inline]
    pub fn vm(&mut self) -> &mut Vm {
        &mut self.vm
    }

    /// Adds a module that importing this name gives, instead of looking for it in `node_modules`. It's written as
    /// CommonJS, and only compiled if something asks for it.
    pub fn add_module(&mut self, name: &'static str, source: &'static str) {
        self.vm.modules.host.push((name, source));
    }

    /// The modules that come with the engine, or that [`add_module`](Self::add_module) gave it, that have been run, in the
    /// order they were.
    pub fn loaded_builtins(&self) -> Vec<String> {
        self.vm.modules.loaded_builtins()
    }

    /// The files that have been run as modules.
    pub fn loaded_files(&self) -> Vec<PathBuf> {
        self.vm.modules.loaded_files()
    }

    /// Forgets which files have been run, so running one again reads it all again. What comes with the engine, and what
    /// [`add_module`](Self::add_module) gave it, stay as they are.
    pub fn forget_files(&mut self) {
        self.vm.modules.forget_files();
    }

    /// Turns an error into text.
    pub fn error_message(&mut self, error: Value) -> crate::JsError {
        self.error_of(error)
    }
}
