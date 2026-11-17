//! `String`.

use super::json::Out;
use crate::convert::trim_units;
use crate::object::*;
use crate::text::{self, Text};
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

pub(super) fn install(vm: &mut Vm) {
    let shape = vm.root_shape(vm.realm.object_proto);
    let empty = vm.new_str("");
    let proto = vm.alloc_object(shape, Kind::Boxed(empty), 0).handle();
    vm.realm.string_proto = proto;
    let string = vm.constructor("String", 1, string_constructor, proto);
    vm.method(string, "fromCharCode", 1, from_char_code);
    vm.method(string, "fromCodePoint", 1, from_code_point);
    vm.method(string, "raw", 1, raw);
    for (name, length, func) in [
        ("toString", 0, to_string as NativeFn),
        ("valueOf", 0, to_string),
        ("charAt", 1, char_at),
        ("charCodeAt", 1, char_code_at),
        ("codePointAt", 1, code_point_at),
        ("at", 1, at),
        ("indexOf", 1, index_of),
        ("lastIndexOf", 1, last_index_of),
        ("includes", 1, includes),
        ("startsWith", 1, starts_with),
        ("endsWith", 1, ends_with),
        ("slice", 2, slice),
        ("substring", 2, substring),
        ("substr", 2, substr),
        ("toUpperCase", 0, to_upper),
        ("toLowerCase", 0, to_lower),
        ("toLocaleUpperCase", 0, to_upper),
        ("toLocaleLowerCase", 0, to_lower),
        ("trim", 0, trim),
        ("trimStart", 0, trim_start),
        ("trimEnd", 0, trim_end),
        ("padStart", 2, pad_start),
        ("padEnd", 2, pad_end),
        ("repeat", 1, repeat),
        ("split", 2, split),
        ("concat", 1, concat),
        ("replace", 2, replace),
        ("replaceAll", 2, replace_all),
        ("match", 1, match_),
        ("matchAll", 1, match_all),
        ("search", 1, search),
        ("localeCompare", 1, locale_compare),
        ("normalize", 0, normalize),
    ] {
        vm.method(proto, name, length, func);
    }
    vm.symbol_method(
        proto,
        text::SYMBOL_ITERATOR,
        "[Symbol.iterator]",
        0,
        iterator,
    );
}

fn string_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = if args.count == 0 {
        vm.new_str("")
    } else {
        let value = vm.arg(&args, 0);
        if value.is_symbol() && !args.new_target.is_object() {
            let description = vm.atom_text(crate::text::Atom(value.handle()));
            vm.new_str(&format!("Symbol({description})"))
        } else {
            vm.to_js_string(value)?
        }
    };
    if args.new_target.is_object() {
        let proto = vm.proto_for(&args, vm.realm.string_proto)?;
        return Ok(vm.new_object_with_proto(proto, Kind::Boxed(value)));
    }
    Ok(value)
}

fn from_char_code(vm: &mut Vm, args: Args) -> Run<Value> {
    let mut units = Vec::with_capacity(args.count);
    for index in 0..args.count {
        let value = vm.arg(&args, index);
        units.push(vm.to_int32(value)? as u16);
    }
    Ok(vm.new_text(Text::from_units(&units)))
}

fn from_code_point(vm: &mut Vm, args: Args) -> Run<Value> {
    let mut units = Vec::with_capacity(args.count);
    for index in 0..args.count {
        let value = vm.arg(&args, index);
        let number = vm.to_number(value)?;
        if number.fract() != 0.0 || !(0.0..=1_114_111.0).contains(&number) {
            let shown = crate::convert::number_to_string(number);
            return Err(vm.range_error(&format!("Invalid code point {shown}")));
        }
        let code = number as u32;
        match char::from_u32(code) {
            Some(character) => {
                let mut buffer = [0; 2];
                units.extend_from_slice(character.encode_utf16(&mut buffer));
            }
            None => units.push(code as u16),
        }
    }
    Ok(vm.new_text(Text::from_units(&units)))
}

fn raw(vm: &mut Vm, args: Args) -> Run<Value> {
    let template = vm.arg(&args, 0);
    let strings = vm.get(template, text::RAW)?;
    let length = vm.length_of(strings)?;
    let mut out = Vec::new();
    for index in 0..length {
        let part = vm.get_index(strings, index)?;
        let part = vm.to_js_string(part)?;
        vm.text_of(part).push_to(&mut out);
        if index + 1 < length && index + 1 < args.count {
            let value = vm.arg(&args, index + 1);
            let value = vm.to_js_string(value)?;
            vm.text_of(value).push_to(&mut out);
        }
    }
    Ok(vm.new_text(Text::from_units(&out)))
}

/// The string that a method is working on.
fn this_string(vm: &mut Vm, args: &Args) -> Run<Value> {
    if args.this.is_string() {
        return Ok(args.this);
    }
    if args.this.is_nullish() {
        return Err(vm.type_error("String.prototype method called on null or undefined"));
    }
    if args.this.is_object() {
        if let Kind::Boxed(value) = vm.objects[args.this.index()].kind {
            if value.is_string() {
                return Ok(value);
            }
        }
    }
    vm.to_js_string(args.this)
}

fn to_string(vm: &mut Vm, args: Args) -> Run<Value> {
    this_string(vm, &args)
}

fn char_at(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let index = vm.arg(&args, 0);
    let index = vm.to_integer(index)?;
    if index < 0.0 || index >= vm.text_of(string).len() as f64 {
        return Ok(vm.new_str(""));
    }
    Ok(vm.char_at(string, index as usize))
}

fn char_code_at(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let index = vm.arg(&args, 0);
    let index = if index.is_int() {
        f64::from(index.as_int())
    } else {
        vm.to_integer(index)?
    };
    let text = vm.text_of(string);
    if index < 0.0 || index >= text.len() as f64 {
        return Ok(Value::double(f64::NAN));
    }
    Ok(Value::int(i32::from(text.at(index as usize))))
}

fn code_point_at(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let index = vm.arg(&args, 0);
    let index = vm.to_integer(index)?;
    let text = vm.text_of(string);
    if index < 0.0 || index >= text.len() as f64 {
        return Ok(Value::UNDEFINED);
    }
    let at = index as usize;
    let first = u32::from(text.at(at));
    if (0xD800..0xDC00).contains(&first) && at + 1 < text.len() {
        let second = u32::from(text.at(at + 1));
        if (0xDC00..0xE000).contains(&second) {
            return Ok(Value::int(
                (0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)) as i32,
            ));
        }
    }
    Ok(Value::int(first as i32))
}

fn at(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let index = vm.arg(&args, 0);
    let number = vm.to_integer(index)?;
    let length = vm.text_of(string).len() as f64;
    let index = if number < 0.0 {
        length + number
    } else {
        number
    };
    if index < 0.0 || index >= length {
        return Ok(Value::UNDEFINED);
    }
    Ok(vm.char_at(string, index as usize))
}

fn find_units(haystack: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return (from <= haystack.len()).then_some(from);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    let first = needle[0];
    let last_start = haystack.len() - needle.len();
    let mut at = from;
    while at <= last_start {
        if haystack[at] == first && haystack[at..at + needle.len()] == *needle {
            return Some(at);
        }
        at += 1;
    }
    None
}

fn index_of(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let needle = vm.arg(&args, 0);
    let needle = vm.to_js_string(needle)?;
    let from = vm.arg(&args, 1);
    let from = vm.to_integer(from)?.max(0.0) as usize;
    let found = find_text(vm.text_of(string), vm.text_of(needle), from);
    Ok(found.map_or(Value::int(-1), Value::int_or_double))
}

/// Where some text is in other text, at or after a position, with nothing copied if they're both ASCII.
fn find_text(haystack: &Text, needle: &Text, from: usize) -> Option<usize> {
    match (haystack.ascii(), needle.ascii()) {
        (Some(haystack), Some(needle)) => find_bytes(haystack, needle, from),
        // ASCII text can't have in it what isn't ASCII.
        (Some(_), None) => None,
        _ => {
            let units = haystack.wide().unwrap_or(&[]);
            find_units(units, &needle.units(), from.min(units.len()))
        }
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    let from = from.min(haystack.len());
    if needle.is_empty() {
        return Some(from);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    let first = needle[0];
    let last_start = haystack.len() - needle.len();
    let mut at = from;
    while at <= last_start {
        if haystack[at] == first && haystack[at..at + needle.len()] == *needle {
            return Some(at);
        }
        at += 1;
    }
    None
}

fn last_index_of(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let needle = vm.arg(&args, 0);
    let needle = vm.to_js_string(needle)?;
    let from = vm.arg(&args, 1);
    let number = vm.to_number(from)?;
    let limit = if number.is_nan() {
        f64::INFINITY
    } else {
        number.trunc().max(0.0)
    };
    let (text, needle) = (vm.text_of(string), vm.text_of(needle));
    let found = match (text.ascii(), needle.ascii()) {
        (Some(haystack), Some(needle)) => rfind(haystack, needle, limit),
        (Some(_), None) => None,
        _ => rfind(&text.units(), &needle.units(), limit),
    };
    Ok(found.map_or(Value::int(-1), Value::int_or_double))
}

/// Where the last copy of some units is that starts at or before `limit`.
fn rfind<T: PartialEq>(haystack: &[T], needle: &[T], limit: f64) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    let start = (haystack.len() - needle.len()).min(limit as usize);
    (0..=start)
        .rev()
        .find(|at| haystack[*at..*at + needle.len()] == *needle)
}

fn reject_regexp(vm: &mut Vm, value: Value, method: &str) -> Run<()> {
    if vm.is_regexp(value) {
        return Err(vm.type_error(&format!(
            "First argument to String.prototype.{method} must not be a regular expression"
        )));
    }
    Ok(())
}

fn includes(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let needle = vm.arg(&args, 0);
    reject_regexp(vm, needle, "includes")?;
    let needle = vm.to_js_string(needle)?;
    let from = vm.arg(&args, 1);
    let from = vm.to_integer(from)?.max(0.0) as usize;
    Ok(Value::bool(
        find_text(vm.text_of(string), vm.text_of(needle), from).is_some(),
    ))
}

fn starts_with(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let needle = vm.arg(&args, 0);
    reject_regexp(vm, needle, "startsWith")?;
    let needle = vm.to_js_string(needle)?;
    let from = vm.arg(&args, 1);
    let from = vm.to_integer(from)?.max(0.0) as usize;
    let (text, needle) = (vm.text_of(string), vm.text_of(needle));
    let from = from.min(text.len());
    Ok(Value::bool(match (text.ascii(), needle.ascii()) {
        (Some(text), Some(needle)) => text[from..].starts_with(needle),
        (Some(_), None) => false,
        _ => text.units()[from..].starts_with(&needle.units()),
    }))
}

fn ends_with(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let needle = vm.arg(&args, 0);
    reject_regexp(vm, needle, "endsWith")?;
    let needle = vm.to_js_string(needle)?;
    let end = vm.arg(&args, 1);
    let length = vm.text_of(string).len();
    let end = if end.is_undefined() {
        length
    } else {
        (vm.to_integer(end)?.max(0.0) as usize).min(length)
    };
    let (text, needle) = (vm.text_of(string), vm.text_of(needle));
    Ok(Value::bool(match (text.ascii(), needle.ascii()) {
        (Some(text), Some(needle)) => text[..end].ends_with(needle),
        (Some(_), None) => false,
        _ => text.units()[..end].ends_with(&needle.units()),
    }))
}

fn slice(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let length = vm.text_of(string).len();
    let (start, end) = (vm.arg(&args, 0), vm.arg(&args, 1));
    let start = vm.relative_index(start, length, 0)?;
    let end = vm.relative_index(end, length, length)?;
    if start >= end {
        return Ok(vm.new_str(""));
    }
    if start == 0 && end == length {
        return Ok(string);
    }
    Ok(vm.piece_of(string, start, end))
}

fn substring(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let length = vm.text_of(string).len();
    let (start, end) = (vm.arg(&args, 0), vm.arg(&args, 1));
    let clamp = |number: f64| number.max(0.0).min(length as f64) as usize;
    let start = clamp(vm.to_integer(start)?);
    let end = if end.is_undefined() {
        length
    } else {
        clamp(vm.to_integer(end)?)
    };
    let (start, end) = if start > end {
        (end, start)
    } else {
        (start, end)
    };
    if start == 0 && end == length {
        return Ok(string);
    }
    Ok(vm.piece_of(string, start, end))
}

fn substr(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let length = vm.text_of(string).len();
    let (start, count) = (vm.arg(&args, 0), vm.arg(&args, 1));
    let start = vm.relative_index(start, length, 0)?;
    let count = if count.is_undefined() {
        length - start
    } else {
        (vm.to_integer(count)?.max(0.0) as usize).min(length - start)
    };
    let piece = vm.text_of(string).slice(start, start + count);
    Ok(vm.new_text(piece))
}

fn to_upper(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let text = match vm.text_of(string).ascii() {
        Some(bytes) => Text::map_ascii(bytes, |byte| byte.to_ascii_uppercase()),
        None => Text::from_str(&vm.text_of(string).to_rust().to_uppercase()),
    };
    Ok(vm.new_text(text))
}

fn to_lower(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let text = match vm.text_of(string).ascii() {
        Some(bytes) => Text::map_ascii(bytes, |byte| byte.to_ascii_lowercase()),
        None => Text::from_str(&vm.text_of(string).to_rust().to_lowercase()),
    };
    Ok(vm.new_text(text))
}

fn trim_either(vm: &mut Vm, args: &Args, start: bool, end: bool) -> Run<Value> {
    let string = this_string(vm, args)?;
    if let Some(bytes) = vm.text_of(string).ascii() {
        // The only spaces in ASCII are tabs, line ends, form feeds and spaces.
        let is_space = |byte: &u8| matches!(*byte, 9..=13 | 32);
        let from = if start {
            bytes.iter().position(|byte| !is_space(byte)).unwrap_or(bytes.len())
        } else {
            0
        };
        let to = if end {
            bytes.iter().rposition(|byte| !is_space(byte)).map_or(from, |at| at + 1)
        } else {
            bytes.len()
        };
        if from == 0 && to == bytes.len() {
            return Ok(string);
        }
        let piece = Text::from_ascii(&bytes[from..to.max(from)]);
        return Ok(vm.new_text(piece));
    }
    let units = vm.text_of(string).units();
    let trimmed = trim_units(&units, start, end);
    if trimmed.len() == units.len() {
        return Ok(string);
    }
    Ok(vm.new_text(Text::from_units(trimmed)))
}

fn trim(vm: &mut Vm, args: Args) -> Run<Value> {
    trim_either(vm, &args, true, true)
}

fn trim_start(vm: &mut Vm, args: Args) -> Run<Value> {
    trim_either(vm, &args, true, false)
}

fn trim_end(vm: &mut Vm, args: Args) -> Run<Value> {
    trim_either(vm, &args, false, true)
}

fn pad(vm: &mut Vm, args: &Args, at_start: bool) -> Run<Value> {
    let string = this_string(vm, args)?;
    let length = vm.text_of(string).len();
    let target = vm.arg(args, 0);
    let target = vm.to_integer(target)?;
    if target <= length as f64 {
        return Ok(string);
    }
    if target > 536_870_888.0 {
        return Err(vm.range_error("Invalid string length"));
    }
    let filler = vm.arg(args, 1);
    let filler = if filler.is_undefined() {
        None
    } else {
        Some(vm.to_js_string(filler)?)
    };
    let space = Text::from_ascii(b" ");
    let missing = target as usize - length;
    let (text, filler) = (
        vm.text_of(string),
        filler.map_or(&space, |filler| vm.text_of(filler)),
    );
    if filler.len() == 0 {
        return Ok(string);
    }
    let made = match (text.ascii(), filler.ascii()) {
        (Some(text), Some(filler)) => {
            let mut out = Vec::with_capacity(target as usize);
            if !at_start {
                out.extend_from_slice(text);
            }
            out.extend(filler.iter().cycle().take(missing));
            if at_start {
                out.extend_from_slice(text);
            }
            Text::from_ascii_vec(out)
        }
        _ => {
            let filler = filler.units();
            let mut out = Vec::with_capacity(target as usize);
            if !at_start {
                text.push_to(&mut out);
            }
            out.extend(filler.iter().cycle().take(missing));
            if at_start {
                text.push_to(&mut out);
            }
            Text::from_units(&out)
        }
    };
    Ok(vm.new_text(made))
}

fn pad_start(vm: &mut Vm, args: Args) -> Run<Value> {
    pad(vm, &args, true)
}

fn pad_end(vm: &mut Vm, args: Args) -> Run<Value> {
    pad(vm, &args, false)
}

fn repeat(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let count = vm.arg(&args, 0);
    let count = vm.to_integer(count)?;
    let length = vm.text_of(string).len();
    if count < 0.0 || count.is_infinite() {
        let shown = crate::convert::number_to_string(count);
        return Err(vm.range_error(&format!("Invalid count value: {shown}")));
    }
    if length as f64 * count > 536_870_888.0 {
        return Err(vm.range_error("Invalid string length"));
    }
    let count = count as usize;
    let source = vm.text_of(string);
    let text = match source.wide() {
        Some(_) if count == 0 => Text::empty(),
        Some(units) => Text::Wide(units.repeat(count).into_boxed_slice()),
        None => Text::from_ascii(&source.ascii().unwrap_or(&[]).repeat(count)),
    };
    Ok(vm.new_text(text))
}

fn split(vm: &mut Vm, args: Args) -> Run<Value> {
    let separator = vm.arg(&args, 0);
    let limit = vm.arg(&args, 1);
    check_this(vm, &args)?;
    if let Some(result) = dispatch(vm, separator, text::SYMBOL_SPLIT, args.this, Some(limit))? {
        return Ok(result);
    }
    let string = this_string(vm, &args)?;
    let limit = if limit.is_undefined() {
        u32::MAX
    } else {
        vm.to_int32(limit)? as u32
    };
    let separator = if separator.is_undefined() {
        None
    } else {
        Some(vm.to_js_string(separator)?)
    };
    let Some(separator) = separator else {
        return Ok(if limit == 0 {
            vm.new_array(Vec::new())
        } else {
            vm.new_array(vec![string])
        });
    };
    let mut items = Vec::new();
    if limit == 0 {
        return Ok(vm.new_array(items));
    }
    let length = vm.text_of(string).len();
    let separator_length = vm.text_of(separator).len();
    if separator_length == 0 {
        for index in 0..length.min(limit as usize) {
            let unit = vm.text_of(string).at(index);
            items.push(vm.char_string(unit));
        }
        return Ok(vm.new_array(items));
    }
    let mut from = 0;
    while items.len() < limit as usize {
        match find_text(vm.text_of(string), vm.text_of(separator), from) {
            Some(at) => {
                let piece = vm.text_of(string).slice(from, at);
                items.push(vm.new_text(piece));
                from = at + separator_length;
            }
            None => {
                let piece = vm.text_of(string).slice(from, length);
                items.push(vm.new_text(piece));
                break;
            }
        }
    }
    Ok(vm.new_array(items))
}

fn concat(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let mut out = Out::default();
    out.push_text(vm.text_of(string));
    for index in 0..args.count {
        let part = vm.arg(&args, index);
        let part = vm.to_js_string(part)?;
        out.push_text(vm.text_of(part));
    }
    Ok(vm.new_text(out.into_text()))
}

/// Puts in what `$1`, `$&` and the like stand for. It works on bytes when the text and the template are both ASCII.
fn expand_replacement<T: Copy + PartialEq + From<u8> + Into<u32>>(
    template: &[T],
    input: &[T],
    captures: &[i32],
    names: &[(String, u32)],
    out: &mut Vec<T>,
) {
    let dollar = T::from(b'$');
    let mut at = 0;
    while at < template.len() {
        let unit = template[at];
        if unit != dollar || at + 1 >= template.len() {
            out.push(unit);
            at += 1;
            continue;
        }
        let next = template[at + 1];
        match char::from_u32(next.into()) {
            Some('$') => {
                out.push(dollar);
                at += 2;
            }
            Some('&') => {
                out.extend_from_slice(&input[captures[0] as usize..captures[1] as usize]);
                at += 2;
            }
            Some('`') => {
                out.extend_from_slice(&input[..captures[0] as usize]);
                at += 2;
            }
            Some('\'') => {
                out.extend_from_slice(&input[captures[1] as usize..]);
                at += 2;
            }
            Some(digit) if digit.is_ascii_digit() => {
                let groups = captures.len() / 2;
                let first = digit.to_digit(10).unwrap_or(0) as usize;
                let mut number = first;
                let mut used = 2;
                if let Some(second) = template
                    .get(at + 2)
                    .and_then(|unit| char::from_u32((*unit).into()).and_then(|c| c.to_digit(10)))
                {
                    let two = first * 10 + second as usize;
                    if two > 0 && two < groups {
                        number = two;
                        used = 3;
                    }
                }
                if number > 0 && number < groups {
                    let (from, to) = (captures[number * 2], captures[number * 2 + 1]);
                    if from >= 0 && to >= 0 {
                        out.extend_from_slice(&input[from as usize..to as usize]);
                    }
                    at += used;
                } else {
                    out.push(dollar);
                    at += 1;
                }
            }
            Some('<') if !names.is_empty() => {
                let close = template[at + 2..]
                    .iter()
                    .position(|unit| *unit == T::from(b'>'));
                if let Some(close) = close {
                    let name: Vec<u16> = template[at + 2..at + 2 + close]
                        .iter()
                        .map(|unit| (*unit).into() as u16)
                        .collect();
                    let name = String::from_utf16_lossy(&name);
                    if let Some((_, index)) = names.iter().find(|(found, _)| *found == name) {
                        let (from, to) = (
                            captures[*index as usize * 2],
                            captures[*index as usize * 2 + 1],
                        );
                        if from >= 0 && to >= 0 {
                            out.extend_from_slice(&input[from as usize..to as usize]);
                        }
                    }
                    at += close + 3;
                } else {
                    out.push(dollar);
                    at += 1;
                }
            }
            _ => {
                out.push(dollar);
                at += 1;
            }
        }
    }
}

/// The text with each match put in place of what a template says.
fn expand_all(input: &Text, template: &Text, matches: &[Vec<i32>], names: &[(String, u32)]) -> Text {
    fn each<T: Copy + PartialEq + From<u8> + Into<u32>>(
        input: &[T],
        template: &[T],
        matches: &[Vec<i32>],
        names: &[(String, u32)],
    ) -> Vec<T> {
        let mut out = Vec::with_capacity(input.len());
        let mut last = 0;
        for captures in matches {
            out.extend_from_slice(&input[last..captures[0] as usize]);
            expand_replacement(template, input, captures, names, &mut out);
            last = captures[1] as usize;
        }
        out.extend_from_slice(&input[last..]);
        out
    }
    match (input.ascii(), template.ascii()) {
        (Some(input), Some(template)) => Text::from_ascii_vec(each(input, template, matches, names)),
        _ => Text::from_units(&each(&input.units(), &template.units(), matches, names)),
    }
}

fn replace_with(
    vm: &mut Vm,
    string: Value,
    matches: Vec<Vec<i32>>,
    replacement: Value,
    regexp: Option<Value>,
) -> Run<Value> {
    let names = match regexp {
        Some(regexp) => vm.regexp_names(regexp)?,
        None => Vec::new(),
    };
    if matches.is_empty() {
        return Ok(string);
    }
    if !vm.is_callable(replacement) {
        let template = vm.to_js_string(replacement)?;
        let text = expand_all(vm.text_of(string), vm.text_of(template), &matches, &names);
        return Ok(vm.new_text(text));
    }
    // The function could do anything, so the text it's working on is its own copy.
    let input = vm.text_of(string).clone();
    let mut out = Out::with_capacity(input.len());
    let mut last = 0;
    let callee = vm.prepare_call(replacement);
    for captures in &matches {
        out.push_part(&input, last, captures[0] as usize);
        let mut call_args = Vec::with_capacity(captures.len() / 2 + 3);
        for pair in captures.chunks(2) {
            call_args.push(if pair[0] < 0 || pair[1] < 0 {
                Value::UNDEFINED
            } else {
                vm.new_text(input.slice(pair[0] as usize, pair[1] as usize))
            });
        }
        let groups = if names.is_empty() {
            None
        } else {
            let groups = vm.new_plain();
            for (name, index) in &names {
                let atom = vm.intern_str(name);
                vm.define(groups.handle(), atom, call_args[*index as usize], PLAIN);
            }
            Some(groups)
        };
        call_args.push(Value::int(captures[0]));
        call_args.push(string);
        if let Some(groups) = groups {
            call_args.push(groups);
        }
        let result = vm.call_prepared(&callee, Value::UNDEFINED, &call_args)?;
        let result = vm.to_js_string(result)?;
        out.push_text(vm.text_of(result));
        last = captures[1] as usize;
    }
    out.push_part(&input, last, input.len());
    Ok(vm.new_text(out.into_text()))
}

fn replace(vm: &mut Vm, args: Args) -> Run<Value> {
    let pattern = vm.arg(&args, 0);
    let replacement = vm.arg(&args, 1);
    check_this(vm, &args)?;
    if let Some(result) = dispatch(
        vm,
        pattern,
        text::SYMBOL_REPLACE,
        args.this,
        Some(replacement),
    )? {
        return Ok(result);
    }
    let string = this_string(vm, &args)?;
    let needle = vm.to_js_string(pattern)?;
    let replacement = plain_replacement(vm, replacement)?;
    let length = vm.text_of(needle).len();
    let matches = match find_text(vm.text_of(string), vm.text_of(needle), 0) {
        Some(at) => vec![vec![at as i32, (at + length) as i32]],
        None => Vec::new(),
    };
    replace_with(vm, string, matches, replacement, None)
}

fn replace_all(vm: &mut Vm, args: Args) -> Run<Value> {
    let pattern = vm.arg(&args, 0);
    let replacement = vm.arg(&args, 1);
    check_this(vm, &args)?;
    check_global(
        vm,
        pattern,
        "replaceAll must be called with a global RegExp",
    )?;
    if let Some(result) = dispatch(
        vm,
        pattern,
        text::SYMBOL_REPLACE,
        args.this,
        Some(replacement),
    )? {
        return Ok(result);
    }
    let string = this_string(vm, &args)?;
    let needle = vm.to_js_string(pattern)?;
    let replacement = plain_replacement(vm, replacement)?;
    let (length, total) = (vm.text_of(needle).len(), vm.text_of(string).len());
    let mut matches = Vec::new();
    let mut from = 0;
    while let Some(at) = find_text(vm.text_of(string), vm.text_of(needle), from) {
        matches.push(vec![at as i32, (at + length) as i32]);
        from = at + length.max(1);
        if from > total {
            break;
        }
    }
    replace_with(vm, string, matches, replacement, None)
}

fn to_regexp(vm: &mut Vm, value: Value, flags: &str) -> Run<Value> {
    if vm.is_regexp(value) {
        return Ok(value);
    }
    let pattern = if value.is_undefined() {
        vm.new_str("(?:)")
    } else {
        vm.to_js_string(value)?
    };
    let flags = vm.new_str(flags);
    let proto = vm.realm.regexp_proto;
    vm.new_regexp(pattern, flags, proto)
}

fn match_(vm: &mut Vm, args: Args) -> Run<Value> {
    let pattern = vm.arg(&args, 0);
    check_this(vm, &args)?;
    if let Some(result) = dispatch(vm, pattern, text::SYMBOL_MATCH, args.this, None)? {
        return Ok(result);
    }
    let string = this_string(vm, &args)?;
    let regexp = to_regexp(vm, pattern, "")?;
    match_with(vm, regexp, string)
}

/// What matching a string against a regular expression gives.
fn match_with(vm: &mut Vm, regexp: Value, string: Value) -> Run<Value> {
    // `flags` is a getter on the prototype; the built-in one only gives the flags it was made with.
    let builtin = match regexp.is_object().then(|| &vm.objects[regexp.index()].kind) {
        Some(Kind::RegExp(data)) => Some(data.flags),
        _ => None,
    };
    let flags = match builtin {
        Some(flags) if vm.has_builtin_getter(regexp.handle(), text::FLAGS, Vm::regexp_flags) => flags,
        _ => vm.get(regexp, text::FLAGS)?,
    };
    if !says_flag(vm, flags, 'g')? {
        return match vm.regexp_exec_raw(regexp, string)? {
            Some(captures) => vm.match_array(regexp, string, &captures),
            None => Ok(Value::NULL),
        };
    }
    let matches = vm.regexp_all(regexp, string)?;
    if matches.is_empty() {
        return Ok(Value::NULL);
    }
    let text = vm.text_of(string).clone();
    let items = matches
        .iter()
        .map(|captures| vm.new_text(text.slice(captures[0] as usize, captures[1] as usize)))
        .collect();
    Ok(vm.new_array(items))
}

fn match_all(vm: &mut Vm, args: Args) -> Run<Value> {
    let pattern = vm.arg(&args, 0);
    check_this(vm, &args)?;
    check_global(
        vm,
        pattern,
        "String.prototype.matchAll called with a non-global RegExp",
    )?;
    if let Some(result) = dispatch(vm, pattern, text::SYMBOL_MATCH_ALL, args.this, None)? {
        return Ok(result);
    }
    let string = this_string(vm, &args)?;
    let regexp = to_regexp(vm, pattern, "g")?;
    match_all_with(vm, regexp, string)
}

fn match_all_with(vm: &mut Vm, regexp: Value, string: Value) -> Run<Value> {
    // It works from its own copy, and starts from where the one it was given has got to.
    let source = vm.get(regexp, text::SOURCE)?;
    let flags = vm.get(regexp, text::FLAGS)?;
    let copy = {
        let proto = vm.realm.regexp_proto;
        vm.new_regexp(source, flags, proto)?
    };
    let last = vm.get(regexp, text::LAST_INDEX)?;
    vm.set(copy, text::LAST_INDEX, last)?;
    let is_global = says_flag(vm, flags, 'g')?;
    let matches = if is_global {
        let start = vm.to_integer(last)?.max(0.0) as usize;
        vm.regexp_all_from(copy, string, start)?
    } else {
        vm.regexp_exec_raw(copy, string)?.into_iter().collect()
    };
    let mut items = Vec::with_capacity(matches.len());
    for captures in &matches {
        items.push(vm.match_array(copy, string, captures)?);
    }
    let array = vm.new_array(items);
    Ok(vm.new_native_iter(IterKind::ArrayValues, array))
}

fn search(vm: &mut Vm, args: Args) -> Run<Value> {
    let pattern = vm.arg(&args, 0);
    check_this(vm, &args)?;
    if let Some(result) = dispatch(vm, pattern, text::SYMBOL_SEARCH, args.this, None)? {
        return Ok(result);
    }
    let string = this_string(vm, &args)?;
    let regexp = to_regexp(vm, pattern, "")?;
    search_with(vm, regexp, string)
}

fn search_with(vm: &mut Vm, regexp: Value, string: Value) -> Run<Value> {
    let saved = vm.get(regexp, text::LAST_INDEX)?;
    vm.set(regexp, text::LAST_INDEX, Value::int(0))?;
    let found = vm.regexp_exec_raw(regexp, string)?;
    vm.set(regexp, text::LAST_INDEX, saved)?;
    Ok(found.map_or(Value::int(-1), |captures| Value::int(captures[0])))
}

fn check_this(vm: &mut Vm, args: &Args) -> Run<()> {
    if args.this.is_nullish() {
        return Err(vm.type_error("String.prototype method called on null or undefined"));
    }
    Ok(())
}

/// Lets an object say how it's used in place of a pattern, through a symbol. Regular expressions do it the same way.
fn dispatch(
    vm: &mut Vm,
    pattern: Value,
    symbol: crate::text::Atom,
    this: Value,
    extra: Option<Value>,
) -> Run<Option<Value>> {
    if !pattern.is_object() {
        return Ok(None);
    }
    let method = vm.get(pattern, symbol)?;
    if method.is_nullish() {
        return Ok(None);
    }
    if !vm.is_callable(method) {
        return Err(vm.not_callable(method, false));
    }
    let result = match extra {
        Some(extra) => vm.call(method, pattern, &[this, extra])?,
        None => vm.call(method, pattern, &[this])?,
    };
    Ok(Some(result))
}

/// Whether a regular expression's `flags`, as a string, have this one in them.
fn says_flag(vm: &mut Vm, flags: Value, flag: char) -> Run<bool> {
    let flags = vm.to_js_string(flags)?;
    let text = vm.text_of(flags);
    Ok((0..text.len()).any(|at| text.at(at) == flag as u16))
}

/// A regular expression given to something that goes through every match has to be set up to do that.
fn check_global(vm: &mut Vm, pattern: Value, message: &str) -> Run<()> {
    if vm.is_regexp(pattern) {
        let flags = vm.get(pattern, text::FLAGS)?;
        if flags.is_nullish() {
            return Err(vm.type_error("flags can't be nothing"));
        }
        if !says_flag(vm, flags, 'g')? {
            return Err(vm.type_error(message));
        }
    }
    Ok(())
}

/// A replacement that's not a function is a string, and that's worked out before anything else happens.
fn plain_replacement(vm: &mut Vm, replacement: Value) -> Run<Value> {
    if vm.is_callable(replacement) {
        return Ok(replacement);
    }
    vm.to_js_string(replacement)
}

fn this_regexp(vm: &mut Vm, this: Value) -> Run<Value> {
    if vm.is_regexp(this) {
        return Ok(this);
    }
    Err(vm.type_error("This method only works on a regular expression for now"))
}

fn regexp_symbol_replace(vm: &mut Vm, args: Args) -> Run<Value> {
    let regexp = this_regexp(vm, args.this)?;
    let string = vm.arg(&args, 0);
    let string = vm.to_js_string(string)?;
    let replacement = vm.arg(&args, 1);
    let replacement = plain_replacement(vm, replacement)?;
    let matches = vm.regexp_all(regexp, string)?;
    replace_with(vm, string, matches, replacement, Some(regexp))
}

fn regexp_symbol_match(vm: &mut Vm, args: Args) -> Run<Value> {
    let regexp = this_regexp(vm, args.this)?;
    let string = vm.arg(&args, 0);
    let string = vm.to_js_string(string)?;
    match_with(vm, regexp, string)
}

fn regexp_symbol_match_all(vm: &mut Vm, args: Args) -> Run<Value> {
    let regexp = this_regexp(vm, args.this)?;
    let string = vm.arg(&args, 0);
    let string = vm.to_js_string(string)?;
    match_all_with(vm, regexp, string)
}

fn regexp_symbol_search(vm: &mut Vm, args: Args) -> Run<Value> {
    let regexp = this_regexp(vm, args.this)?;
    let string = vm.arg(&args, 0);
    let string = vm.to_js_string(string)?;
    search_with(vm, regexp, string)
}

fn regexp_symbol_split(vm: &mut Vm, args: Args) -> Run<Value> {
    let regexp = this_regexp(vm, args.this)?;
    let string = vm.arg(&args, 0);
    let string = vm.to_js_string(string)?;
    let limit = vm.arg(&args, 1);
    let limit = if limit.is_undefined() {
        u32::MAX
    } else {
        vm.to_int32(limit)? as u32
    };
    vm.regexp_split(regexp, string, limit)
}

/// The methods strings call on a regular expression, which programs can call too.
pub(super) fn install_regexp_symbols(vm: &mut Vm, proto: u32) {
    vm.symbol_method(
        proto,
        text::SYMBOL_REPLACE,
        "[Symbol.replace]",
        2,
        regexp_symbol_replace,
    );
    vm.symbol_method(
        proto,
        text::SYMBOL_MATCH,
        "[Symbol.match]",
        1,
        regexp_symbol_match,
    );
    vm.symbol_method(
        proto,
        text::SYMBOL_MATCH_ALL,
        "[Symbol.matchAll]",
        1,
        regexp_symbol_match_all,
    );
    vm.symbol_method(
        proto,
        text::SYMBOL_SEARCH,
        "[Symbol.search]",
        1,
        regexp_symbol_search,
    );
    vm.symbol_method(
        proto,
        text::SYMBOL_SPLIT,
        "[Symbol.split]",
        2,
        regexp_symbol_split,
    );
}

fn locale_compare(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let other = vm.arg(&args, 0);
    let other = vm.to_rust_string(other)?;
    let this = vm.text_of(string).to_rust();
    // No real collation yet. This puts things in the order people would mostly expect, ignoring case first.
    let order = this
        .to_lowercase()
        .cmp(&other.to_lowercase())
        .then_with(|| other.cmp(&this));
    Ok(Value::int(order as i32))
}

fn normalize(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    let form = vm.arg(&args, 0);
    let form = if form.is_undefined() {
        super::normalize::Form::Nfc
    } else {
        let name = vm.to_rust_string(form)?;
        match super::normalize::Form::named(&name) {
            Some(form) => form,
            None => {
                return Err(vm.range_error("The normalization form should be one of NFC, NFD, NFKC, NFKD."));
            }
        }
    };
    let text = vm.text_of(string);
    // ASCII is in every form.
    if text.ascii().is_some() {
        return Ok(string);
    }
    let units = text.units();
    match super::normalize::normalize(&units, form) {
        Some(changed) => Ok(vm.new_text(Text::from_units(&changed))),
        None => Ok(string),
    }
}

fn iterator(vm: &mut Vm, args: Args) -> Run<Value> {
    let string = this_string(vm, &args)?;
    Ok(vm.new_native_iter(IterKind::StringValues, string))
}
