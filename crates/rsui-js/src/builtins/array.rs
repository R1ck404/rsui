//! `Array`.

use crate::object::*;
use crate::text;
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

pub(super) fn install(vm: &mut Vm) {
    let proto = vm.realm.array_proto;
    let array = vm.constructor("Array", 1, array_constructor, proto);
    vm.realm.array_constructor = array;
    vm.method(array, "isArray", 1, is_array);
    vm.method(array, "from", 1, array_from);
    vm.method(array, "of", 0, array_of);
    vm.js_helper(array, "fromAsync", "arrayFromAsync", 1);
    vm.species_getter(array);
    for (name, length, func) in [
        ("push", 1, push as NativeFn),
        ("pop", 0, pop),
        ("shift", 0, shift),
        ("unshift", 1, unshift),
        ("slice", 2, slice),
        ("splice", 2, splice),
        ("concat", 1, concat),
        ("join", 1, join),
        ("reverse", 0, reverse),
        ("indexOf", 1, index_of),
        ("lastIndexOf", 1, last_index_of),
        ("includes", 1, includes),
        ("find", 1, find),
        ("findIndex", 1, find_index),
        ("findLast", 1, find_last),
        ("findLastIndex", 1, find_last_index),
        ("filter", 1, filter),
        ("map", 1, map),
        ("forEach", 1, for_each),
        ("reduce", 1, reduce),
        ("reduceRight", 1, reduce_right),
        ("some", 1, some),
        ("every", 1, every),
        ("sort", 1, sort),
        ("flat", 0, flat),
        ("flatMap", 1, flat_map),
        ("fill", 1, fill),
        ("keys", 0, keys),
        ("entries", 0, entries),
        ("at", 1, at),
        ("copyWithin", 2, copy_within),
        ("toString", 0, to_string),
        ("toLocaleString", 0, to_string),
        ("toSorted", 1, to_sorted),
        ("toReversed", 0, to_reversed),
        ("toSpliced", 2, to_spliced),
        ("with", 2, with),
    ] {
        vm.method(proto, name, length, func);
    }
    let values = vm.intern_str("values");
    let function = vm.new_native(values, 0, array_values, Value::UNDEFINED, false);
    vm.add_property(proto, values, function, HIDDEN);
    vm.add_property(proto, text::SYMBOL_ITERATOR, function, HIDDEN);
    vm.realm.array_values = crate::vm::Value0(function);
}

fn array_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    let items = if args.count == 1 && vm.arg(&args, 0).is_number() {
        let number = vm.arg(&args, 0).as_number();
        if number < 0.0 || number.fract() != 0.0 || number > 4_294_967_295.0 {
            return Err(vm.range_error("Invalid array length"));
        }
        if number > 268_435_456.0 {
            return Err(vm.range_error("An array can't be this long yet"));
        }
        vec![Value::EMPTY; number as usize]
    } else {
        (0..args.count).map(|index| vm.arg(&args, index)).collect()
    };
    let array = vm.new_array(items);
    if args.new_target.is_object() && args.new_target.handle() != args.callee {
        let proto = vm.proto_for(&args, vm.realm.array_proto)?;
        vm.set_proto(array.handle(), proto);
    }
    Ok(array)
}

fn is_array(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    Ok(Value::bool(vm.is_array_value(value)))
}

fn array_from(vm: &mut Vm, args: Args) -> Run<Value> {
    let source = vm.arg(&args, 0);
    let mapper = vm.arg(&args, 1);
    if source.is_nullish() {
        return Err(vm.type_error("Array.from needs something to copy"));
    }
    if !mapper.is_undefined() && !vm.is_callable(mapper) {
        return Err(vm.not_callable(mapper, false));
    }
    let this_arg = vm.arg(&args, 2);
    let method = vm.get(source, text::SYMBOL_ITERATOR)?;
    let result = vm.new_array(Vec::new());
    let mapper_call = vm.prepare_call(mapper);
    let mut index = 0usize;
    let mut add = |vm: &mut Vm, item: Value| -> Run<()> {
        let item = if mapper.is_undefined() {
            item
        } else {
            vm.call_prepared(
                &mapper_call,
                this_arg,
                &[item, Value::number(index as f64)],
            )?
        };
        if let Kind::Array(items) = &mut vm.objects[result.index()].kind {
            items.push(item);
        }
        vm.debt += 8;
        index += 1;
        Ok(())
    };
    let from_iterator = !method.is_nullish();
    if from_iterator {
        vm.iterate(source, add)?;
    } else {
        let source = vm.to_object(source)?;
        let length = vm.length_of(source)?;
        if length > 1 << 28 {
            return Err(vm.range_error("Invalid array length"));
        }
        for at in 0..length {
            let item = vm.get_index(source, at)?;
            add(vm, item)?;
        }
    }
    made_by(vm, args.this, result, !from_iterator)
}

fn array_of(vm: &mut Vm, args: Args) -> Run<Value> {
    let items = (0..args.count).map(|index| vm.arg(&args, index)).collect();
    let result = vm.new_array(items);
    made_by(vm, args.this, result, true)
}

/// What `Array.from` and `Array.of` come to, when they're called on a subclass or anything else that makes things, which
/// is told how long it'll be if that's known in advance.
fn made_by(vm: &mut Vm, this: Value, result: Value, says_length: bool) -> Run<Value> {
    if this == Value::object(vm.realm.array_constructor) || !vm.is_constructor(this) {
        return Ok(result);
    }
    let values = vm.array_items(result);
    let made = if says_length {
        vm.construct(this, &[Value::number(values.len() as f64)])?
    } else {
        vm.construct(this, &[])?
    };
    fill_species(vm, made, &values, true)
}

fn items_mut(vm: &mut Vm, this: Value) -> Option<&mut Items> {
    if !this.is_object() {
        return None;
    }
    let found = &mut vm.objects[this.index()];
    if found.flags & (FROZEN_ELEMENTS | NOT_EXTENSIBLE) != 0 {
        return None;
    }
    match &mut found.kind {
        Kind::Array(items) => Some(items),
        _ => None,
    }
}

fn check_writable(vm: &mut Vm, this: Value) -> Run<()> {
    if this.is_object() && vm.objects[this.index()].flags & (FROZEN_ELEMENTS | NOT_EXTENSIBLE) != 0
    {
        return Err(vm.type_error("Cannot change an array that's frozen"));
    }
    Ok(())
}

fn push(vm: &mut Vm, args: Args) -> Run<Value> {
    // An everyday array only has to have the values put on the end.
    if args.this.is_object() && !vm.prototypes_have_elements {
        let (objects, stack) = (&mut vm.objects, &vm.stack);
        let object = &mut objects[args.this.index()];
        let unusual = FROZEN_ELEMENTS | NOT_EXTENSIBLE | READ_ONLY_LENGTH | SPECIAL_ELEMENTS;
        if object.flags & unusual == 0 {
            if let Kind::Array(items) = &mut object.kind {
                for value in &stack[args.start..args.start + args.count] {
                    items.push(*value);
                }
                let length = items.len();
                vm.debt += args.count * 8;
                return Ok(Value::int_or_double(length));
            }
        }
    }
    check_writable(vm, args.this)?;
    let mut length = vm.length_of(args.this)?;
    if (length + args.count) as f64 > 9_007_199_254_740_991.0 {
        return Err(vm.type_error("That would make it longer than anything can be"));
    }
    for index in 0..args.count {
        let value = vm.arg(&args, index);
        vm.set_index(args.this, length, value)?;
        length += 1;
    }
    vm.set(args.this, text::LENGTH, Value::number(length as f64))?;
    Ok(Value::number(length as f64))
}

fn pop(vm: &mut Vm, args: Args) -> Run<Value> {
    check_writable(vm, args.this)?;
    if let Some(items) = items_mut(vm, args.this) {
        return Ok(match items.pop() {
            Some(value) if !value.is_empty() => value,
            _ => Value::UNDEFINED,
        });
    }
    let length = vm.length_of(args.this)?;
    if length == 0 {
        vm.set(args.this, text::LENGTH, Value::int(0))?;
        return Ok(Value::UNDEFINED);
    }
    let value = vm.get_index(args.this, length - 1)?;
    vm.set(args.this, text::LENGTH, Value::number(length as f64 - 1.0))?;
    Ok(value)
}

fn shift(vm: &mut Vm, args: Args) -> Run<Value> {
    check_writable(vm, args.this)?;
    if let Some(items) = items_mut(vm, args.this) {
        if items.is_empty() {
            return Ok(Value::UNDEFINED);
        }
        let value = items.vec().remove(0);
        return Ok(if value.is_empty() {
            Value::UNDEFINED
        } else {
            value
        });
    }
    let length = vm.length_of(args.this)?;
    if length == 0 {
        vm.set(args.this, text::LENGTH, Value::int(0))?;
        return Ok(Value::UNDEFINED);
    }
    let first = vm.get_index(args.this, 0)?;
    move_elements(vm, args.this, 1, 0, length - 1)?;
    delete_index(vm, args.this, length - 1)?;
    vm.set(args.this, text::LENGTH, Value::number(length as f64 - 1.0))?;
    Ok(first)
}

fn unshift(vm: &mut Vm, args: Args) -> Run<Value> {
    check_writable(vm, args.this)?;
    let new_items: Vec<Value> = (0..args.count).map(|index| vm.arg(&args, index)).collect();
    if let Some(items) = items_mut(vm, args.this) {
        items.vec().splice(0..0, new_items);
        let length = items.len();
        return Ok(Value::number(length as f64));
    }
    let length = vm.length_of(args.this)?;
    if (length + new_items.len()) as f64 > 9_007_199_254_740_991.0 {
        return Err(vm.type_error("That would make it longer than anything can be"));
    }
    move_elements(vm, args.this, 0, new_items.len(), length)?;
    let count = new_items.len();
    for (index, item) in new_items.into_iter().enumerate() {
        vm.set_index(args.this, index, item)?;
    }
    let total = Value::number((length + count) as f64);
    vm.set(args.this, text::LENGTH, total)?;
    Ok(total)
}

fn delete_index(vm: &mut Vm, target: Value, index: usize) -> Run<()> {
    let atom = vm.index_atom(index);
    if !vm.delete(target.handle(), atom)? {
        return Err(vm.type_error(&format!("Cannot delete property '{index}'")));
    }
    Ok(())
}

/// Which of the things that [`Vm::constructor_of`] remembers is for arrays.
const ARRAY_MEMO: usize = 0;

/// What should make the array that a method of this one gives back, if it's anything other than an ordinary array: it's
/// how a subclass gets its own kind of object. An ordinary array only has to compare its constructor.
fn species_of(vm: &mut Vm, original: Value) -> Run<Option<Value>> {
    if !vm.is_array_value(original) {
        return Ok(None);
    }
    let plain = Value::object(vm.realm.array_constructor);
    let mut constructor = vm.constructor_of(original, vm.realm.array_proto, ARRAY_MEMO)?;
    if constructor == plain {
        return Ok(None);
    }
    if constructor.is_object() {
        constructor = vm.get(constructor, text::SYMBOL_SPECIES)?;
        if constructor.is_null() {
            constructor = Value::UNDEFINED;
        }
    }
    if constructor.is_undefined() || constructor == plain {
        return Ok(None);
    }
    if !vm.is_constructor(constructor) {
        return Err(vm.type_error("The constructor of an array has to be a constructor"));
    }
    Ok(Some(constructor))
}

/// Puts what a method came to in what the subclass made, a property at a time, as it could be anything.
fn fill_species(vm: &mut Vm, target: Value, values: &[Value], set_length: bool) -> Run<Value> {
    for (index, value) in values.iter().enumerate() {
        if value.is_empty() {
            continue;
        }
        let descriptor = vm.new_plain();
        vm.define(descriptor.handle(), text::VALUE, *value, PLAIN);
        for flag in [text::WRITABLE, text::ENUMERABLE, text::CONFIGURABLE] {
            vm.define(descriptor.handle(), flag, Value::TRUE, PLAIN);
        }
        let atom = vm.index_atom(index);
        vm.define_from_descriptor(target.handle(), atom, descriptor)?;
    }
    if set_length {
        vm.set(target, text::LENGTH, Value::number(values.len() as f64))?;
    }
    Ok(target)
}

/// The same, from an array made here, which is what's given back if there's nothing to do.
fn in_species(vm: &mut Vm, made: Option<Value>, result: Value, set_length: bool) -> Run<Value> {
    match made {
        None => Ok(result),
        Some(target) => {
            let values = vm.array_items(result);
            fill_species(vm, target, &values, set_length)
        }
    }
}

/// Makes what the subclass wants for a result, before anything else happens, if there's a subclass.
fn make_species(vm: &mut Vm, original: Value, length: f64) -> Run<Option<Value>> {
    match species_of(vm, original)? {
        None => Ok(None),
        Some(constructor) => Ok(Some(vm.construct(constructor, &[Value::number(length)])?)),
    }
}

/// Moves elements along inside something that's array-like but isn't an array, keeping its holes as holes.
fn move_elements(vm: &mut Vm, target: Value, from: usize, to: usize, count: usize) -> Run<()> {
    let backward = to > from;
    for turn in 0..count {
        let offset = if backward { count - 1 - turn } else { turn };
        let from_atom = vm.index_atom(from + offset);
        if vm.has_property_checked(target.handle(), from_atom)? {
            let item = vm.get(target, from_atom)?;
            vm.set_index(target, to + offset, item)?;
        } else {
            delete_index(vm, target, to + offset)?;
        }
    }
    Ok(())
}

/// Refuses to make an array longer than arrays can be.
fn check_can_hold(vm: &mut Vm, length: usize) -> Run<()> {
    if length > 1 << 28 {
        return Err(vm.range_error("Invalid array length"));
    }
    Ok(())
}

/// Whether something array-like has an element here, on it or on a prototype.
fn has_index(vm: &mut Vm, object: Value, index: usize) -> Run<bool> {
    if let Kind::Array(items) = &vm.objects[object.index()].kind {
        if items.get(index).is_some_and(|item| !item.is_empty()) {
            return Ok(true);
        }
        if vm.objects[object.index()].flags & SPECIAL_ELEMENTS == 0 && !vm.prototypes_have_elements
        {
            return Ok(false);
        }
    }
    let atom = vm.index_atom(index);
    vm.has_property_checked(object.handle(), atom)
}

/// Where to start from, for something that takes a position relative to the end.
fn start_index(vm: &mut Vm, value: Value, length: usize) -> Run<usize> {
    let number = vm.to_integer(value)?;
    Ok(if number < 0.0 {
        (length as f64 + number).max(0.0) as usize
    } else {
        number.min(length as f64) as usize
    })
}

/// Everything in something array-like, with the holes kept as they are.
fn list_of(vm: &mut Vm, this: Value) -> Run<Vec<Value>> {
    if this.is_object() {
        if let Kind::Array(items) | Kind::Arguments(items) = &vm.objects[this.index()].kind {
            return Ok(items.to_vec());
        }
    }
    let object = vm.to_object(this)?;
    let length = vm.length_of(object)?;
    check_can_hold(vm, length)?;
    let mut items = Vec::with_capacity(length.min(1 << 20));
    for index in 0..length {
        let item = vm.get_index(object, index)?;
        vm.nursery.push(item);
        items.push(item);
    }
    Ok(items)
}

fn slice(vm: &mut Vm, args: Args) -> Run<Value> {
    let (start_arg, end_arg) = (vm.arg(&args, 0), vm.arg(&args, 1));
    if args.this.is_object() {
        if let Kind::Array(items) | Kind::Arguments(items) = &vm.objects[args.this.index()].kind {
            let length = items.len();
            let start = vm.relative_index(start_arg, length, 0)?;
            let end = vm.relative_index(end_arg, length, length)?;
            let made = make_species(vm, args.this, end.saturating_sub(start) as f64)?;
            if let Some(made) = made {
                let copy = vm.array_items(args.this);
                let copy = copy.get(start..end).unwrap_or_default().to_vec();
                return fill_species(vm, made, &copy, true);
            }
            let (Kind::Array(items) | Kind::Arguments(items)) = &vm.objects[args.this.index()].kind
            else {
                return Ok(vm.new_array(Vec::new()));
            };
            let copy = if start < end && end <= items.len() {
                items[start..end].to_vec()
            } else {
                Vec::new()
            };
            return Ok(vm.new_array(copy));
        }
    }
    let object = vm.to_object(args.this)?;
    let length = vm.length_of(object)?;
    let start = vm.relative_index(start_arg, length, 0)?;
    let end = vm.relative_index(end_arg, length, length)?;
    let count = end.saturating_sub(start);
    let made = make_species(vm, object, count as f64)?;
    check_can_hold(vm, count)?;
    let mut copy = vec![Value::EMPTY; count];
    for (offset, slot) in copy.iter_mut().enumerate() {
        if has_index(vm, object, start + offset)? {
            let item = vm.get_index(object, start + offset)?;
            vm.nursery.push(item);
            *slot = item;
        }
    }
    let result = vm.new_array(copy);
    in_species(vm, made, result, true)
}

fn splice(vm: &mut Vm, args: Args) -> Run<Value> {
    check_writable(vm, args.this)?;
    let is_real_array = items_mut(vm, args.this).is_some();
    let length = vm.length_of(args.this)?;
    let start_arg = vm.arg(&args, 0);
    let start = vm.relative_index(start_arg, length, 0)?;
    let delete = if args.count == 0 {
        0
    } else if args.count == 1 {
        length - start
    } else {
        let count = vm.arg(&args, 1);
        let count = vm.to_integer(count)?;
        count.max(0.0).min((length - start) as f64) as usize
    };
    let new_items: Vec<Value> = (2..args.count).map(|index| vm.arg(&args, index)).collect();
    if (length + new_items.len()).saturating_sub(delete) as f64 > 9_007_199_254_740_991.0 {
        return Err(vm.type_error("That would make it longer than anything can be"));
    }
    check_can_hold(vm, delete)?;
    let made = make_species(vm, args.this, delete as f64)?;
    if !is_real_array {
        let mut removed = Vec::with_capacity(delete);
        for index in 0..delete {
            removed.push(vm.get_index(args.this, start + index)?);
        }
        let added = new_items.len();
        if added != delete {
            move_elements(
                vm,
                args.this,
                start + delete,
                start + added,
                length - start - delete,
            )?;
            for index in (length + added - delete..length).rev() {
                delete_index(vm, args.this, index)?;
            }
        }
        for (index, item) in new_items.into_iter().enumerate() {
            vm.set_index(args.this, start + index, item)?;
        }
        let total = Value::number((length + added - delete) as f64);
        vm.set(args.this, text::LENGTH, total)?;
        let result = vm.new_array(removed);
        return in_species(vm, made, result, true);
    }
    let Some(items) = items_mut(vm, args.this) else {
        unreachable!("it was an array just now");
    };
    let removed: Vec<Value> = items
        .vec()
        .splice(start..start + delete, new_items)
        .collect();
    let result = vm.new_array(removed);
    in_species(vm, made, result, true)
}

fn concat(vm: &mut Vm, args: Args) -> Run<Value> {
    let mut out = Vec::new();
    let this = vm.to_object(args.this)?;
    let made = make_species(vm, this, 0.0)?;
    for index in 0..=args.count {
        let value = if index == 0 {
            this
        } else {
            vm.arg(&args, index - 1)
        };
        let spreadable = if value.is_object() {
            let flag = vm.get(value, text::SYMBOL_IS_CONCAT_SPREADABLE)?;
            if flag.is_undefined() {
                vm.is_array_value(value)
            } else {
                vm.to_boolean(flag)
            }
        } else {
            false
        };
        if spreadable {
            let length = vm.length_of(value)?;
            if (out.len() + length) as f64 > 9_007_199_254_740_991.0 {
                return Err(vm.type_error("That would make it longer than anything can be"));
            }
            check_can_hold(vm, out.len() + length)?;
            for index in 0..length {
                if has_index(vm, value, index)? {
                    let item = vm.get_index(value, index)?;
                    vm.nursery.push(item);
                    out.push(item);
                } else {
                    out.push(Value::EMPTY);
                }
            }
        } else {
            out.push(value);
        }
    }
    let result = vm.new_array(out);
    in_species(vm, made, result, true)
}

fn join(vm: &mut Vm, args: Args) -> Run<Value> {
    let separator = vm.arg(&args, 0);
    let separator = if separator.is_undefined() {
        vec![u16::from(b',')]
    } else {
        vm.to_units(separator)?
    };
    join_with(vm, args.this, &separator)
}

fn join_with(vm: &mut Vm, this: Value, separator: &[u16]) -> Run<Value> {
    if vm.joining.contains(&this) {
        return Ok(vm.new_str(""));
    }
    vm.joining.push(this);
    let result = join_inner(vm, this, separator);
    vm.joining.pop();
    result
}

fn join_inner(vm: &mut Vm, this: Value, separator: &[u16]) -> Run<Value> {
    if let Some(text) = join_plain(vm, this, separator) {
        return Ok(vm.new_text(text));
    }
    let length = vm.length_of(this)?;
    let mut out = Vec::new();
    for index in 0..length {
        if index > 0 {
            out.extend_from_slice(separator);
        }
        let item = vm.get_index(this, index)?;
        if !item.is_nullish() {
            let text = vm.to_js_string(item)?;
            vm.text_of(text).push_to(&mut out);
        }
    }
    Ok(vm.new_text(crate::text::Text::from_units(&out)))
}

/// [`join_inner`] for an everyday array of strings and numbers, where reading each element can't run anything and the
/// strings only need copying, with no units made for each one.
fn join_plain(vm: &Vm, this: Value, separator: &[u16]) -> Option<text::Text> {
    if !this.is_object() || vm.prototypes_have_elements {
        return None;
    }
    let object = &vm.objects[this.index()];
    if object.flags & SPECIAL_ELEMENTS != 0 {
        return None;
    }
    let Kind::Array(items) = &object.kind else {
        return None;
    };
    let mut out = super::json::Out::default();
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_units(separator);
        }
        if item.is_string() {
            out.push_text(&vm.strings.items[item.index()].text);
        } else if item.is_int() {
            out.push_text(&crate::convert::int_text(item.as_int()));
        } else if item.is_double() {
            out.push_text(&crate::convert::number_text(item.as_double()));
        } else if !item.is_nullish() && !item.is_empty() {
            return None;
        }
    }
    Some(out.into_text())
}

fn to_string(vm: &mut Vm, args: Args) -> Run<Value> {
    join_with(vm, args.this, &[u16::from(b',')])
}

fn reverse(vm: &mut Vm, args: Args) -> Run<Value> {
    check_writable(vm, args.this)?;
    if let Some(items) = items_mut(vm, args.this) {
        items.reverse();
        return Ok(args.this);
    }
    let length = vm.length_of(args.this)?;
    for index in 0..length / 2 {
        let other = length - 1 - index;
        let (low, high) = (
            vm.get_index(args.this, index)?,
            vm.get_index(args.this, other)?,
        );
        vm.set_index(args.this, index, high)?;
        vm.set_index(args.this, other, low)?;
    }
    Ok(args.this)
}

fn index_of(vm: &mut Vm, args: Args) -> Run<Value> {
    let wanted = vm.arg(&args, 0);
    let object = vm.to_object(args.this)?;
    let length = vm.length_of(object)?;
    if length == 0 {
        return Ok(Value::int(-1));
    }
    let from = vm.arg(&args, 1);
    let from = start_index(vm, from, length)?;
    // Working out where to start could have run code that changed the array, and then it's gone through as it was.
    if let Some(items) = plain_items(vm, object).filter(|items| items.len() == length) {
        let items = items.get(from..).unwrap_or_default();
        // A hole is never equal to anything, as it's skipped. An int can only be the same int, or a double of it.
        let found = if wanted.is_int() {
            let number = f64::from(wanted.as_int());
            items
                .iter()
                .position(|item| *item == wanted || (item.is_double() && item.as_double() == number))
        } else if wanted.is_string() {
            find_string(vm, items, wanted)
        } else {
            items.iter().position(|item| vm.strict_equals(*item, wanted))
        };
        return Ok(found.map_or(Value::int(-1), |at| Value::int_or_double(from + at)));
    }
    for index in from..length {
        if has_index(vm, object, index)? {
            let item = vm.get_index(object, index)?;
            if vm.strict_equals(item, wanted) {
                return Ok(Value::number(index as f64));
            }
        }
    }
    Ok(Value::int(-1))
}

/// Where a string is among some elements. Strings with different hashes can't be the same, which settles most of them
/// without comparing their text.
fn find_string(vm: &Vm, items: &[Value], wanted: Value) -> Option<usize> {
    let strings = &vm.strings.items;
    let wanted_data = &strings[wanted.index()];
    let hash = wanted_data.hash();
    items.iter().position(|item| {
        *item == wanted
            || (item.is_string() && {
                let data = &strings[item.index()];
                data.hash() == hash && data.text == wanted_data.text
            })
    })
}

/// The elements of an everyday array, which can be read where they are: nothing can be watching them, and a hole only
/// stands for `undefined`, as no prototype has elements.
fn plain_items(vm: &Vm, object: Value) -> Option<&[Value]> {
    if !object.is_object() || vm.prototypes_have_elements {
        return None;
    }
    let found = &vm.objects[object.index()];
    if found.flags & SPECIAL_ELEMENTS != 0 {
        return None;
    }
    match &found.kind {
        Kind::Array(items) => Some(&**items),
        _ => None,
    }
}

fn last_index_of(vm: &mut Vm, args: Args) -> Run<Value> {
    let wanted = vm.arg(&args, 0);
    let object = vm.to_object(args.this)?;
    let length = vm.length_of(object)?;
    if length == 0 {
        return Ok(Value::int(-1));
    }
    let from = if args.count > 1 {
        let from = vm.arg(&args, 1);
        let number = vm.to_integer(from)?;
        if number < 0.0 {
            let back = length as f64 + number;
            if back < 0.0 {
                return Ok(Value::int(-1));
            }
            back as usize
        } else {
            (number.min(length as f64 - 1.0)) as usize
        }
    } else {
        length - 1
    };
    for index in (0..=from).rev() {
        if has_index(vm, object, index)? {
            let item = vm.get_index(object, index)?;
            if vm.strict_equals(item, wanted) {
                return Ok(Value::number(index as f64));
            }
        }
    }
    Ok(Value::int(-1))
}

fn includes(vm: &mut Vm, args: Args) -> Run<Value> {
    let wanted = vm.arg(&args, 0);
    let object = vm.to_object(args.this)?;
    let length = vm.length_of(object)?;
    if length == 0 {
        return Ok(Value::FALSE);
    }
    let from = vm.arg(&args, 1);
    let from = start_index(vm, from, length)?;
    // Working out where to start could have run code that changed the array, and then it's gone through as it was.
    if let Some(items) = plain_items(vm, object).filter(|items| items.len() == length) {
        let items = items.get(from..).unwrap_or_default();
        let found = if wanted.is_int() {
            let number = f64::from(wanted.as_int());
            items
                .iter()
                .any(|item| *item == wanted || (item.is_double() && item.as_double() == number))
        } else if wanted.is_string() {
            find_string(vm, items, wanted).is_some()
        } else {
            items.iter().any(|item| {
                let item = if item.is_empty() { Value::UNDEFINED } else { *item };
                item == wanted || vm.same_value_zero(item, wanted)
            })
        };
        return Ok(Value::bool(found));
    }
    for index in from..length {
        let item = vm.get_index(object, index)?;
        if vm.same_value_zero(item, wanted) {
            return Ok(Value::TRUE);
        }
    }
    Ok(Value::FALSE)
}

fn callback(vm: &mut Vm, args: &Args) -> Run<(Value, Value)> {
    let function = vm.arg(args, 0);
    if !vm.is_callable(function) {
        let shown = vm.brief(function);
        return Err(vm.type_error(&format!("{shown} is not a function")));
    }
    Ok((function, vm.arg(args, 1)))
}

/// Goes through an array, calling a function for each element that's there, for as long as the visitor says.
fn visit(
    vm: &mut Vm,
    args: &Args,
    skip_holes: bool,
    backward: bool,
    visitor: impl FnMut(&mut Vm, usize, Value, Value) -> Run<bool>,
) -> Run<()> {
    let object = vm.to_object(args.this)?;
    let length = vm.length_of(object)?;
    visit_this(vm, args, object, length, skip_holes, backward, visitor)
}

/// [`visit`], for what's already read the length, which only happens once as something could be watching.
fn visit_this(
    vm: &mut Vm,
    args: &Args,
    object: Value,
    length: usize,
    skip_holes: bool,
    backward: bool,
    mut visitor: impl FnMut(&mut Vm, usize, Value, Value) -> Run<bool>,
) -> Run<()> {
    let (function, this_arg) = callback(vm, args)?;
    let callee = vm.prepare_call(function);
    let mark = vm.nursery.len();
    for turn in 0..length {
        let index = if backward { length - 1 - turn } else { turn };
        let item = match &vm.objects[object.index()].kind {
            Kind::Array(items) => items.get(index).copied(),
            _ => None,
        };
        let item = match item {
            Some(item) if !item.is_empty() => item,
            _ => {
                if skip_holes && !has_index(vm, object, index)? {
                    continue;
                }
                vm.get_index(object, index)?
            }
        };
        let result = vm.call_prepared(
            &callee,
            this_arg,
            &[item, Value::int_or_double(index), object],
        )?;
        if !visitor(vm, index, item, result)? {
            break;
        }
        // What a turn made has gone by now, or is held by whatever kept it. A long loop shouldn't hold on to it all.
        vm.nursery.truncate(mark);
    }
    Ok(())
}

fn find(vm: &mut Vm, args: Args) -> Run<Value> {
    let mut found = Value::UNDEFINED;
    visit(vm, &args, false, false, |vm, _, item, result| {
        if vm.to_boolean(result) {
            found = item;
            return Ok(false);
        }
        Ok(true)
    })?;
    Ok(found)
}

fn find_index(vm: &mut Vm, args: Args) -> Run<Value> {
    let mut found = -1.0;
    visit(vm, &args, false, false, |vm, index, _, result| {
        if vm.to_boolean(result) {
            found = index as f64;
            return Ok(false);
        }
        Ok(true)
    })?;
    Ok(Value::number(found))
}

fn find_last(vm: &mut Vm, args: Args) -> Run<Value> {
    let mut found = Value::UNDEFINED;
    visit(vm, &args, false, true, |vm, _, item, result| {
        if vm.to_boolean(result) {
            found = item;
            return Ok(false);
        }
        Ok(true)
    })?;
    Ok(found)
}

fn find_last_index(vm: &mut Vm, args: Args) -> Run<Value> {
    let mut found = -1.0;
    visit(vm, &args, false, true, |vm, index, _, result| {
        if vm.to_boolean(result) {
            found = index as f64;
            return Ok(false);
        }
        Ok(true)
    })?;
    Ok(Value::number(found))
}

fn filter(vm: &mut Vm, args: Args) -> Run<Value> {
    let object = vm.to_object(args.this)?;
    let length = vm.length_of(object)?;
    callback(vm, &args)?;
    let made = make_species(vm, object, 0.0)?;
    let result = vm.new_array(Vec::new());
    visit_this(
        vm,
        &args,
        object,
        length,
        true,
        false,
        |vm, _, item, kept| {
            if vm.to_boolean(kept) {
                if let Kind::Array(items) = &mut vm.objects[result.index()].kind {
                    items.push(item);
                }
                vm.debt += 8;
            }
            Ok(true)
        },
    )?;
    in_species(vm, made, result, false)
}

fn map(vm: &mut Vm, args: Args) -> Run<Value> {
    let object = vm.to_object(args.this)?;
    let length = vm.length_of(object)?;
    callback(vm, &args)?;
    let made = make_species(vm, object, length as f64)?;
    check_can_hold(vm, length)?;
    let result = vm.new_array(vec![Value::EMPTY; length]);
    visit_this(
        vm,
        &args,
        object,
        length,
        true,
        false,
        |vm, index, _, mapped| {
            if let Kind::Array(items) = &mut vm.objects[result.index()].kind {
                items[index] = mapped;
            }
            Ok(true)
        },
    )?;
    in_species(vm, made, result, false)
}

fn for_each(vm: &mut Vm, args: Args) -> Run<Value> {
    visit(vm, &args, true, false, |_, _, _, _| Ok(true))?;
    Ok(Value::UNDEFINED)
}

fn some(vm: &mut Vm, args: Args) -> Run<Value> {
    let mut found = false;
    visit(vm, &args, true, false, |vm, _, _, result| {
        found = vm.to_boolean(result);
        Ok(!found)
    })?;
    Ok(Value::bool(found))
}

fn every(vm: &mut Vm, args: Args) -> Run<Value> {
    let mut all = true;
    visit(vm, &args, true, false, |vm, _, _, result| {
        all = vm.to_boolean(result);
        Ok(all)
    })?;
    Ok(Value::bool(all))
}

fn reduce_either(vm: &mut Vm, args: &Args, backward: bool) -> Run<Value> {
    let object = vm.to_object(args.this)?;
    let length = vm.length_of(object)?;
    let function = vm.arg(args, 0);
    if !vm.is_callable(function) {
        let shown = vm.brief(function);
        return Err(vm.type_error(&format!("{shown} is not a function")));
    }
    let mut accumulator = (args.count > 1).then(|| vm.arg(args, 1));
    let callee = vm.prepare_call(function);
    for turn in 0..length {
        let index = if backward { length - 1 - turn } else { turn };
        let item = match &vm.objects[object.index()].kind {
            Kind::Array(items) => items.get(index).copied(),
            _ => None,
        };
        let item = match item {
            Some(item) if !item.is_empty() => item,
            _ => {
                if !has_index(vm, object, index)? {
                    continue;
                }
                vm.get_index(object, index)?
            }
        };
        accumulator = Some(match accumulator {
            None => item,
            Some(value) => vm.call_prepared(
                &callee,
                Value::UNDEFINED,
                &[value, item, Value::number(index as f64), object],
            )?,
        });
    }
    match accumulator {
        Some(value) => Ok(value),
        None => Err(vm.type_error("Reduce of empty array with no initial value")),
    }
}

fn reduce(vm: &mut Vm, args: Args) -> Run<Value> {
    reduce_either(vm, &args, false)
}

fn reduce_right(vm: &mut Vm, args: Args) -> Run<Value> {
    reduce_either(vm, &args, true)
}

/// A sort that keeps equal things in order, and stops at the first error from the comparison.
fn merge_sort(
    items: &mut Vec<Value>,
    compare: &mut impl FnMut(Value, Value) -> Run<bool>,
) -> Run<()> {
    let length = items.len();
    if length < 2 {
        return Ok(());
    }
    let mut buffer = items.clone();
    let mut width = 1;
    // Short stretches are put in order first, which is quicker than merging from single elements.
    const RUN: usize = 8;
    for chunk in items.chunks_mut(RUN) {
        for next in 1..chunk.len() {
            let value = chunk[next];
            let mut at = next;
            while at > 0 && compare(value, chunk[at - 1])? {
                chunk[at] = chunk[at - 1];
                at -= 1;
            }
            chunk[at] = value;
        }
    }
    width *= RUN;
    while width < length {
        let mut start = 0;
        while start < length {
            let middle = (start + width).min(length);
            let end = (start + width * 2).min(length);
            // Two halves that are in order already only need copying, which makes a sorted list quick to sort.
            if middle == end || !compare(items[middle], items[middle - 1])? {
                buffer[start..end].copy_from_slice(&items[start..end]);
                start = end;
                continue;
            }
            let (mut left, mut right, mut out) = (start, middle, start);
            while left < middle && right < end {
                if compare(items[right], items[left])? {
                    buffer[out] = items[right];
                    right += 1;
                } else {
                    buffer[out] = items[left];
                    left += 1;
                }
                out += 1;
            }
            buffer[out..out + middle - left].copy_from_slice(&items[left..middle]);
            out += middle - left;
            buffer[out..out + end - right].copy_from_slice(&items[right..end]);
            start = end;
        }
        std::mem::swap(items, &mut buffer);
        width *= 2;
    }
    Ok(())
}

/// What a default sort compares an element by: its string, where it already is.
enum SortKey<'a> {
    Ascii(&'a [u8]),
    Wide(&'a [u16]),
}

impl<'a> SortKey<'a> {
    fn of(text: &'a text::Text) -> Self {
        match (text.ascii(), text.wide()) {
            (Some(bytes), _) => Self::Ascii(bytes),
            (_, units) => Self::Wide(units.unwrap_or(&[])),
        }
    }

    /// The first eight bytes, or the first four units, as a number that's in the same order as the strings are, with
    /// what's missing as zero. Two strings with the same number have to be compared the long way.
    fn prefix(&self, all_ascii: bool) -> u64 {
        let mut prefix = 0u64;
        match self {
            Self::Ascii(bytes) if all_ascii => {
                for at in 0..8 {
                    prefix = prefix << 8 | u64::from(bytes.get(at).copied().unwrap_or(0));
                }
            }
            Self::Ascii(bytes) => {
                for at in 0..4 {
                    prefix = prefix << 16 | u64::from(bytes.get(at).copied().unwrap_or(0));
                }
            }
            Self::Wide(units) => {
                for at in 0..4 {
                    prefix = prefix << 16 | u64::from(units.get(at).copied().unwrap_or(0));
                }
            }
        }
        prefix
    }

    /// The order of two strings, unit by unit, as `<` has it.
    fn compare(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Ascii(a), Self::Ascii(b)) => a.cmp(b),
            (Self::Wide(a), Self::Wide(b)) => a.cmp(b),
            (Self::Ascii(a), Self::Wide(b)) => a.iter().map(|byte| u16::from(*byte)).cmp(b.iter().copied()),
            (Self::Wide(a), Self::Ascii(b)) => a.iter().copied().cmp(b.iter().map(|byte| u16::from(*byte))),
        }
    }
}

pub(super) fn sorted(vm: &mut Vm, comparator: Value, list: Vec<Value>) -> Run<Vec<Value>> {
    if !comparator.is_undefined() && !vm.is_callable(comparator) {
        return Err(vm.type_error("The comparison function must be either a function or undefined"));
    }
    let total = list.len();
    // Holes go at the very end, and `undefined` just before them.
    let mut items: Vec<Value> = list
        .iter()
        .copied()
        .filter(|item| !item.is_empty() && !item.is_undefined())
        .collect();
    let undefined_count = list.iter().filter(|item| item.is_undefined()).count();
    // While sorting, the only copy of an element might be here.
    vm.nursery.extend_from_slice(&items);
    if comparator.is_undefined() {
        // Each is compared as its string. Strings and whole numbers need nothing made for that, and anything else is
        // turned into one first, as that could run code, and kept for the collector to see.
        let mut as_strings = Vec::with_capacity(items.len());
        for item in &items {
            if item.is_string() || item.is_int() {
                as_strings.push(*item);
            } else {
                let string = vm.to_js_string(*item)?;
                vm.nursery.push(string);
                as_strings.push(string);
            }
        }
        let digits: Vec<text::Text> = as_strings
            .iter()
            .filter(|value| value.is_int())
            .map(|value| crate::convert::int_text(value.as_int()))
            .collect();
        let mut next_digits = digits.iter();
        let strings = &vm.strings.items;
        let keyed: Vec<(SortKey, Value)> = as_strings
            .iter()
            .zip(&items)
            .map(|(key, item)| {
                let text = if key.is_int() {
                    next_digits.next().unwrap_or(&digits[0])
                } else {
                    &strings[key.index()].text
                };
                (SortKey::of(text), *item)
            })
            .collect();
        // The first few units of each, as one number, settle most comparisons with no looking at the strings.
        let all_ascii = keyed.iter().all(|key| matches!(key.0, SortKey::Ascii(_)));
        let mut prefixed: Vec<(u64, SortKey, Value)> = keyed
            .into_iter()
            .map(|(key, item)| (key.prefix(all_ascii), key, item))
            .collect();
        prefixed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.compare(&b.1)));
        items = prefixed.into_iter().map(|entry| entry.2).collect();
    } else {
        let callee = vm.prepare_call(comparator);
        merge_sort(&mut items, &mut |a, b| {
            let result = vm.call_prepared(&callee, Value::UNDEFINED, &[a, b])?;
            Ok(if result.is_int() {
                result.as_int() < 0
            } else {
                vm.to_number(result)? < 0.0
            })
        })?;
    }
    items.extend(std::iter::repeat_n(Value::UNDEFINED, undefined_count));
    items.resize(total, Value::EMPTY);
    Ok(items)
}

fn sort(vm: &mut Vm, args: Args) -> Run<Value> {
    check_writable(vm, args.this)?;
    let comparator = vm.arg(&args, 0);
    let list = list_of(vm, args.this)?;
    let items = sorted(vm, comparator, list)?;
    if let Some(target) = items_mut(vm, args.this) {
        *target = items.into();
    } else {
        for (index, item) in items.into_iter().enumerate() {
            vm.set_index(args.this, index, item)?;
        }
    }
    Ok(args.this)
}

fn to_sorted(vm: &mut Vm, args: Args) -> Run<Value> {
    let comparator = vm.arg(&args, 0);
    let list = list_of(vm, args.this)?;
    let items = sorted(vm, comparator, list)?;
    Ok(vm.new_array(items))
}

fn to_reversed(vm: &mut Vm, args: Args) -> Run<Value> {
    let mut items = list_of(vm, args.this)?;
    items.reverse();
    Ok(vm.new_array(items))
}

fn to_spliced(vm: &mut Vm, args: Args) -> Run<Value> {
    let mut items = list_of(vm, args.this)?;
    let length = items.len();
    let start_arg = vm.arg(&args, 0);
    let start = vm.relative_index(start_arg, length, 0)?;
    let delete = if args.count == 0 {
        0
    } else if args.count == 1 {
        length - start
    } else {
        let count = vm.arg(&args, 1);
        let count = vm.to_integer(count)?;
        count.max(0.0).min((length - start) as f64) as usize
    };
    let new_items: Vec<Value> = (2..args.count).map(|index| vm.arg(&args, index)).collect();
    items.splice(start..start + delete, new_items);
    Ok(vm.new_array(items))
}

fn with(vm: &mut Vm, args: Args) -> Run<Value> {
    let mut items = list_of(vm, args.this)?;
    let index = vm.arg(&args, 0);
    let number = vm.to_integer(index)?;
    let at = if number < 0.0 {
        items.len() as f64 + number
    } else {
        number
    };
    if at < 0.0 || at >= items.len() as f64 {
        return Err(vm.range_error("Invalid index"));
    }
    items[at as usize] = vm.arg(&args, 1);
    Ok(vm.new_array(items))
}

fn flatten_into(vm: &mut Vm, out: &mut Vec<Value>, source: &[Value], depth: f64) -> Run<()> {
    for item in source {
        if item.is_empty() {
            continue;
        }
        if vm.is_array_value(*item) && depth >= 1.0 {
            let inner = list_of(vm, *item)?;
            flatten_into(vm, out, &inner, depth - 1.0)?;
        } else {
            out.push(*item);
        }
    }
    Ok(())
}

fn flat(vm: &mut Vm, args: Args) -> Run<Value> {
    let depth = vm.arg(&args, 0);
    let depth = if depth.is_undefined() {
        1.0
    } else {
        vm.to_integer(depth)?
    };
    let items = list_of(vm, args.this)?;
    let this = vm.to_object(args.this)?;
    let made = make_species(vm, this, 0.0)?;
    let mut out = Vec::new();
    flatten_into(vm, &mut out, &items, depth)?;
    let result = vm.new_array(out);
    in_species(vm, made, result, false)
}

fn flat_map(vm: &mut Vm, args: Args) -> Run<Value> {
    let object = vm.to_object(args.this)?;
    let length = vm.length_of(object)?;
    callback(vm, &args)?;
    let made = make_species(vm, object, 0.0)?;
    let result = vm.new_array(Vec::new());
    visit_this(
        vm,
        &args,
        object,
        length,
        true,
        false,
        |vm, _, _, mapped| {
            let list = if vm.is_array_value(mapped) {
                list_of(vm, mapped)?
            } else {
                vec![mapped]
            };
            if let Kind::Array(items) = &mut vm.objects[result.index()].kind {
                items
                    .vec()
                    .extend(list.into_iter().filter(|item| !item.is_empty()));
            }
            Ok(true)
        },
    )?;
    in_species(vm, made, result, false)
}

fn fill(vm: &mut Vm, args: Args) -> Run<Value> {
    check_writable(vm, args.this)?;
    let value = vm.arg(&args, 0);
    let length = vm.length_of(args.this)?;
    let (start, end) = (vm.arg(&args, 1), vm.arg(&args, 2));
    let start = vm.relative_index(start, length, 0)?;
    let end = vm.relative_index(end, length, length)?;
    for index in start..end {
        vm.set_index(args.this, index, value)?;
    }
    Ok(args.this)
}

fn keys(vm: &mut Vm, args: Args) -> Run<Value> {
    let object = vm.to_object(args.this)?;
    Ok(vm.new_native_iter(IterKind::ArrayKeys, object))
}

fn entries(vm: &mut Vm, args: Args) -> Run<Value> {
    let object = vm.to_object(args.this)?;
    Ok(vm.new_native_iter(IterKind::ArrayEntries, object))
}

fn array_values(vm: &mut Vm, args: Args) -> Run<Value> {
    let object = vm.to_object(args.this)?;
    Ok(vm.new_native_iter(IterKind::ArrayValues, object))
}

fn at(vm: &mut Vm, args: Args) -> Run<Value> {
    let object = vm.to_object(args.this)?;
    let length = vm.length_of(object)?;
    let index = vm.arg(&args, 0);
    let number = vm.to_integer(index)?;
    let index = if number < 0.0 {
        length as f64 + number
    } else {
        number
    };
    if index < 0.0 || index >= length as f64 {
        return Ok(Value::UNDEFINED);
    }
    vm.get_index(object, index as usize)
}

fn copy_within(vm: &mut Vm, args: Args) -> Run<Value> {
    check_writable(vm, args.this)?;
    let items = list_of(vm, args.this)?;
    let length = items.len();
    let (target, start, end) = (vm.arg(&args, 0), vm.arg(&args, 1), vm.arg(&args, 2));
    let target = vm.relative_index(target, length, 0)?;
    let start = vm.relative_index(start, length, 0)?;
    let end = vm.relative_index(end, length, length)?;
    for offset in 0..end.saturating_sub(start) {
        if target + offset >= length {
            break;
        }
        vm.set_index(args.this, target + offset, items[start + offset])?;
    }
    Ok(args.this)
}
