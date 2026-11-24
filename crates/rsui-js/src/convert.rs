//! Turning values into other kinds of values, as JavaScript says to, and comparing them.

use crate::builtins::BigOp;
use crate::object::*;
use crate::text::{self, Atom, Text};
use crate::value::Value;
use crate::vm::{Run, Vm};

/// JavaScript's `**`, which differs from `powf` for a base of one with an exponent that isn't a number.
pub(crate) fn js_pow(base: f64, exponent: f64) -> f64 {
    if exponent.is_nan() {
        return f64::NAN;
    }
    if exponent == 0.0 {
        return 1.0;
    }
    if base.abs() == 1.0 && exponent.is_infinite() {
        return f64::NAN;
    }
    base.powf(exponent)
}

/// Room for a number written as JavaScript writes it, on the stack: the longest is a sign, 17 digits, a point and an
/// exponent, or a sign, `0.00000` and 17 digits.
struct NumberBuffer {
    bytes: [u8; 40],
    length: usize,
}

impl std::fmt::Write for NumberBuffer {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let end = self.length + text.len();
        let place = self.bytes.get_mut(self.length..end).ok_or(std::fmt::Error)?;
        place.copy_from_slice(text.as_bytes());
        self.length = end;
        Ok(())
    }
}

/// The number as text, in the fewest digits that read back as the same number, with nothing on the heap if it's short.
pub(crate) fn number_text(number: f64) -> Text {
    use std::fmt::Write;
    let abs = number.abs();
    let mut buffer = NumberBuffer {
        bytes: [0; 40],
        length: 0,
    };
    // From a millionth up to 10^21 JavaScript writes the digits with a point and no exponent, as Rust does, and whole
    // numbers below 10^15 as they are.
    let written = if number.fract() == 0.0 && (1.0..1e15).contains(&abs) {
        write!(buffer, "{}", number as i64)
    } else if (1e-6..1e21).contains(&abs) {
        write!(buffer, "{number}")
    } else {
        Err(std::fmt::Error)
    };
    match written {
        Ok(()) => Text::from_ascii(&buffer.bytes[..buffer.length]),
        Err(_) => Text::from_str(&number_to_string(number)),
    }
}

/// The number as text, in the fewest digits that read back as the same number.
pub(crate) fn number_to_string(number: f64) -> String {
    if number.is_nan() {
        return "NaN".to_owned();
    }
    if number == 0.0 {
        return "0".to_owned();
    }
    if number.is_infinite() {
        return if number > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        }
        .to_owned();
    }
    if number.fract() == 0.0 && number.abs() < 1e15 {
        return format!("{}", number as i64);
    }
    // Rust gives the shortest digits, in the form `d.ddde±x`.
    let formatted = format!("{:e}", number.abs());
    let (mantissa, exponent) = formatted
        .split_once('e')
        .expect("this format always has an exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let exponent: i32 = exponent
        .parse()
        .expect("this format's exponent is a number");
    let count = digits.len() as i32;
    let point = exponent + 1;
    let mut out = String::new();
    if number < 0.0 {
        out.push('-');
    }
    if count <= point && point <= 21 {
        out.push_str(&digits);
        for _ in 0..point - count {
            out.push('0');
        }
    } else if 0 < point && point <= 21 {
        out.push_str(&digits[..point as usize]);
        out.push('.');
        out.push_str(&digits[point as usize..]);
    } else if -6 < point && point <= 0 {
        out.push_str("0.");
        for _ in 0..-point {
            out.push('0');
        }
        out.push_str(&digits);
    } else {
        out.push_str(&digits[..1]);
        if count > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if exponent >= 0 { '+' } else { '-' });
        out.push_str(&exponent.abs().to_string());
    }
    out
}

/// A whole number as text. It's short enough to need no allocation.
pub(crate) fn int_text(value: i32) -> Text {
    let mut digits = [0u8; 12];
    let mut at = digits.len();
    let mut rest = value.unsigned_abs();
    loop {
        at -= 1;
        digits[at] = b'0' + (rest % 10) as u8;
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    if value < 0 {
        at -= 1;
        digits[at] = b'-';
    }
    Text::from_ascii(&digits[at..])
}

fn is_js_space(unit: u16) -> bool {
    matches!(
        unit,
        0x09 | 0x0A | 0x0B | 0x0C | 0x0D | 0x20 | 0xA0 | 0x1680 | 0x2000
            ..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0xFEFF
    )
}

pub(crate) fn trim_units(units: &[u16], start: bool, end: bool) -> &[u16] {
    let mut from = 0;
    let mut to = units.len();
    if start {
        while from < to && is_js_space(units[from]) {
            from += 1;
        }
    }
    if end {
        while to > from && is_js_space(units[to - 1]) {
            to -= 1;
        }
    }
    &units[from..to]
}

pub(crate) fn string_to_number(text: &Text) -> f64 {
    // Most strings made into numbers are a few plain digits, which need nothing copied or parsed the long way.
    if let Some(bytes) = text.ascii() {
        if !bytes.is_empty() && bytes.len() <= 9 && bytes.iter().all(u8::is_ascii_digit) {
            let value = bytes
                .iter()
                .fold(0u32, |value, byte| value * 10 + u32::from(byte - b'0'));
            return f64::from(value);
        }
    }
    let units = text.units();
    let trimmed = trim_units(&units, true, true);
    if trimmed.is_empty() {
        return 0.0;
    }
    let Ok(text) = String::from_utf16(trimmed) else {
        return f64::NAN;
    };
    let bytes = text.as_bytes();
    if bytes.len() > 2 && bytes[0] == b'0' {
        let radix = match bytes[1] | 0x20 {
            b'x' => 16,
            b'o' => 8,
            b'b' => 2,
            _ => 0,
        };
        if radix != 0 {
            let mut value = 0f64;
            for byte in &bytes[2..] {
                let Some(digit) = (*byte as char).to_digit(radix) else {
                    return f64::NAN;
                };
                value = value * f64::from(radix) + f64::from(digit);
            }
            return value;
        }
    }
    match text.as_str() {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    // Rust reads a few things that JavaScript doesn't, like `inf` and `nan`.
    if !bytes
        .iter()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        return f64::NAN;
    }
    text.parse().unwrap_or(f64::NAN)
}

pub(crate) fn f64_to_int32(number: f64) -> i32 {
    if number.is_nan() || number.is_infinite() {
        return 0;
    }
    if number.abs() < 2_147_483_648.0 {
        return number as i32;
    }
    let wrapped = number.trunc().rem_euclid(4_294_967_296.0);
    wrapped as u32 as i32
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Hint {
    Default,
    Number,
    String,
}

/// What a number, `undefined`, `null` or a boolean is as a number, which takes no conversion that could run code or
/// make a string. Nothing for anything else.
#[inline(always)]
fn plain_number(value: Value) -> Option<f64> {
    if value.is_number() {
        Some(value.as_number())
    } else if value == Value::UNDEFINED {
        Some(f64::NAN)
    } else if value == Value::NULL || value == Value::FALSE {
        Some(0.0)
    } else if value == Value::TRUE {
        Some(1.0)
    } else {
        None
    }
}

impl Vm {
    #[inline]
    pub(crate) fn to_boolean(&self, value: Value) -> bool {
        if value.is_bool() {
            return value == Value::TRUE;
        }
        if value.is_int() {
            return value.as_int() != 0;
        }
        if value.is_double() {
            let number = value.as_double();
            return number != 0.0 && !number.is_nan();
        }
        if value.is_string() {
            return self.strings.items[value.index()].text.len() != 0;
        }
        if value.is_bigint() {
            return !self.big_of(value).is_zero();
        }
        !value.is_nullish()
    }

    pub(crate) fn to_primitive(&mut self, value: Value, hint: Hint) -> Run<Value> {
        if !value.is_object() {
            return Ok(value);
        }
        let special = self.get(value, text::SYMBOL_TO_PRIMITIVE)?;
        if !special.is_nullish() {
            let name = match hint {
                Hint::Default => "default",
                Hint::Number => "number",
                Hint::String => "string",
            };
            let name = self.new_str(name);
            let result = self.call(special, value, &[name])?;
            if result.is_object() {
                return Err(self.type_error("Cannot convert object to primitive value"));
            }
            return Ok(result);
        }
        let order = if hint == Hint::String {
            [text::TO_STRING, text::VALUE_OF]
        } else {
            [text::VALUE_OF, text::TO_STRING]
        };
        for name in order {
            let method = self.get(value, name)?;
            if self.is_callable(method) {
                let result = self.call(method, value, &[])?;
                if !result.is_object() {
                    return Ok(result);
                }
            }
        }
        Err(self.type_error("Cannot convert object to primitive value"))
    }

    #[inline(never)]
    pub(crate) fn to_number(&mut self, value: Value) -> Run<f64> {
        if value.is_number() {
            return Ok(value.as_number());
        }
        if value.is_undefined() {
            return Ok(f64::NAN);
        }
        if value.is_null() || value == Value::FALSE {
            return Ok(0.0);
        }
        if value == Value::TRUE {
            return Ok(1.0);
        }
        if value.is_string() {
            return Ok(string_to_number(&self.strings.items[value.index()].text));
        }
        if value.is_symbol() {
            return Err(self.type_error("Cannot convert a Symbol value to a number"));
        }
        if value.is_bigint() {
            return Err(self.type_error("Cannot convert a BigInt value to a number"));
        }
        let primitive = self.to_primitive(value, Hint::Number)?;
        self.to_number(primitive)
    }

    /// A number or a BigInt, which are the two things arithmetic works on.
    #[inline(never)]
    pub(crate) fn to_numeric(&mut self, value: Value) -> Run<Value> {
        if value.is_number() || value.is_bigint() {
            return Ok(value);
        }
        let primitive = self.to_primitive(value, Hint::Number)?;
        if primitive.is_bigint() {
            return Ok(primitive);
        }
        Ok(Value::number(self.to_number(primitive)?))
    }

    pub(crate) fn to_int32(&mut self, value: Value) -> Run<i32> {
        if value.is_int() {
            return Ok(value.as_int());
        }
        Ok(f64_to_int32(self.to_number(value)?))
    }

    /// A number with its fraction taken off, and with NaN as zero.
    pub(crate) fn to_integer(&mut self, value: Value) -> Run<f64> {
        let number = self.to_number(value)?;
        if number.is_nan() {
            return Ok(0.0);
        }
        Ok(number.trunc())
    }

    /// Turns a relative position, which might be negative or too big, into one inside a length.
    pub(crate) fn relative_index(
        &mut self,
        value: Value,
        length: usize,
        default: usize,
    ) -> Run<usize> {
        if value.is_undefined() {
            return Ok(default);
        }
        let number = self.to_integer(value)?;
        Ok(if number < 0.0 {
            (length as f64 + number).max(0.0) as usize
        } else {
            number.min(length as f64) as usize
        })
    }

    /// Makes a string of a value, as a string value.
    pub(crate) fn to_js_string(&mut self, value: Value) -> Run<Value> {
        if value.is_string() {
            return Ok(value);
        }
        if value.is_int() {
            let small = value.as_int();
            if (0..10).contains(&small) {
                return Ok(self.char_string(b'0' as u16 + small as u16));
            }
            return Ok(self.number_string(value));
        }
        if value.is_double() {
            return Ok(self.number_string(value));
        }
        if value.is_undefined() {
            return Ok(self.name_string(text::TYPE_UNDEFINED));
        }
        if value.is_null() {
            return Ok(self.name_string(text::WORD_NULL));
        }
        if value == Value::TRUE {
            return Ok(self.name_string(text::WORD_TRUE));
        }
        if value == Value::FALSE {
            return Ok(self.name_string(text::WORD_FALSE));
        }
        if value.is_symbol() {
            return Err(self.type_error("Cannot convert a Symbol value to a string"));
        }
        if value.is_bigint() {
            let text = self.big_of(value).to_radix(10);
            return Ok(self.new_str(&text));
        }
        let primitive = self.to_primitive(value, Hint::String)?;
        self.to_js_string(primitive)
    }

    /// A number as a string: the same string as last time, if it was turned into one since the last collection.
    #[inline]
    fn number_string(&mut self, value: Value) -> Value {
        let bits = value.0;
        let place = ((bits ^ (bits >> 29) ^ (bits >> 47)) as usize) % crate::vm::NUMBER_STRINGS;
        let (known, handle) = self.number_strings[place];
        if known == bits && handle != NONE {
            // Held as a new string is, as the nursery may have let go of it since it was made.
            let string = Value::string(handle);
            self.nursery.push(string);
            return string;
        }
        let text = if value.is_int() {
            int_text(value.as_int())
        } else {
            number_text(value.as_double())
        };
        let string = self.new_text(text);
        self.number_strings[place] = (bits, string.handle());
        string
    }

    /// The part of a string from `start` to `end`: the same string as last time, if this was asked for since the last
    /// collection. `Object.prototype.toString.call(x).slice(0, 7)`, which libraries check types with (chart.js's
    /// `isArray`), slices the same tag string the same way every time.
    pub(crate) fn piece_of(&mut self, string: Value, start: usize, end: usize) -> Value {
        let handle = string.handle();
        let place = (handle as usize ^ start.wrapping_mul(31) ^ end.wrapping_mul(17)) % self.piece_cache.len();
        let (known, from, to, piece) = self.piece_cache[place];
        if known == handle && from as usize == start && to as usize == end && piece != NONE {
            let piece = Value::string(piece);
            self.nursery.push(piece);
            return piece;
        }
        let text = self.text_of(string).slice(start, end);
        let piece = self.new_text(text);
        if let (Ok(from), Ok(to)) = (u32::try_from(start), u32::try_from(end)) {
            self.piece_cache[place] = (handle, from, to, piece.handle());
        }
        piece
    }

    /// The string of a name that's always there, which needs nothing making or looking up.
    #[inline]
    pub(crate) fn name_string(&self, atom: Atom) -> Value {
        Value::string(self.atoms.string_of(atom).unwrap_or(0))
    }

    pub(crate) fn to_rust_string(&mut self, value: Value) -> Run<String> {
        let string = self.to_js_string(value)?;
        Ok(self.strings.items[string.index()].text.to_rust())
    }

    /// The text of a value, for messages, without running any of the program's code.
    pub(crate) fn to_string_lossy(&mut self, value: Value) -> Run<String> {
        if value.is_object() {
            return Ok("[object]".to_owned());
        }
        if value.is_symbol() {
            return Ok("Symbol()".to_owned());
        }
        self.to_rust_string(value)
    }

    /// The units of a value's string, as a list of its own.
    pub(crate) fn to_units(&mut self, value: Value) -> Run<Vec<u16>> {
        let string = self.to_js_string(value)?;
        Ok(self.strings.items[string.index()].text.units())
    }

    pub(crate) fn text_of(&self, string: Value) -> &Text {
        &self.strings.items[string.index()].text
    }

    pub(crate) fn to_atom(&mut self, value: Value) -> Run<Atom> {
        if value.is_string() {
            let handle = value.handle();
            return Ok(self.atoms.intern_string(&mut self.strings, handle));
        }
        if value.is_symbol() {
            return Ok(Atom(value.handle()));
        }
        if value.is_int() && value.as_int() >= 0 {
            if let Some(atom) = Atom::from_index(value.as_int() as usize) {
                return Ok(atom);
            }
        }
        if value.is_int() {
            return Ok(self.intern_key_str(&value.as_int().to_string()));
        }
        let primitive = self.to_primitive(value, Hint::String)?;
        if primitive.is_symbol() {
            return Ok(Atom(primitive.handle()));
        }
        let string = self.to_js_string(primitive)?;
        let handle = string.handle();
        Ok(self.atoms.intern_string(&mut self.strings, handle))
    }

    pub(crate) fn to_object(&mut self, value: Value) -> Run<Value> {
        if value.is_object() {
            return Ok(value);
        }
        if value.is_nullish() {
            return Err(self.type_error("Cannot convert undefined or null to object"));
        }
        let proto = if value.is_string() {
            self.realm.string_proto
        } else if value.is_bigint() {
            self.realm.bigint_proto
        } else if value.is_number() {
            self.realm.number_proto
        } else if value.is_bool() {
            self.realm.boolean_proto
        } else {
            self.realm.symbol_proto
        };
        Ok(self.new_object_with_proto(proto, Kind::Boxed(value)))
    }

    #[inline]
    pub(crate) fn type_of(&self, value: Value) -> Value {
        self.name_string(if value.is_number() {
            text::TYPE_NUMBER
        } else if value.is_string() {
            text::TYPE_STRING
        } else if value.is_bool() {
            text::TYPE_BOOLEAN
        } else if value.is_undefined() {
            text::TYPE_UNDEFINED
        } else if value.is_symbol() {
            text::TYPE_SYMBOL
        } else if value.is_bigint() {
            text::TYPE_BIGINT
        } else if self.is_callable(value) {
            text::TYPE_FUNCTION
        } else {
            text::TYPE_OBJECT
        })
    }

    #[inline]
    pub(crate) fn strict_equals(&self, a: Value, b: Value) -> bool {
        if a == b {
            // NaN is the one thing that isn't equal to itself.
            return !(a.is_double() && a.as_double().is_nan());
        }
        if a.is_number() && b.is_number() {
            return a.as_number() == b.as_number();
        }
        if a.is_string() && b.is_string() {
            return self.different_strings_are_equal(a, b);
        }
        if a.is_bigint() && b.is_bigint() {
            return self.big_of(a) == self.big_of(b);
        }
        false
    }

    /// Two strings that aren't the same one could still have the same text.
    fn different_strings_are_equal(&self, a: Value, b: Value) -> bool {
        let (first, second) = (
            &self.strings.items[a.index()],
            &self.strings.items[b.index()],
        );
        // Each text has one atom, so two strings that both know theirs have the answer right there.
        if first.atom != 0 && second.atom != 0 {
            return first.atom == second.atom;
        }
        first.text == second.text
    }

    pub(crate) fn same_value_zero(&self, a: Value, b: Value) -> bool {
        if a.is_double() && b.is_double() && a.as_double().is_nan() && b.as_double().is_nan() {
            return true;
        }
        self.strict_equals(a, b)
    }

    pub(crate) fn same_value(&self, a: Value, b: Value) -> bool {
        if a.is_number() && b.is_number() {
            let (x, y) = (a.as_number(), b.as_number());
            if x.is_nan() && y.is_nan() {
                return true;
            }
            return x == y && (x != 0.0 || x.is_sign_negative() == y.is_sign_negative());
        }
        self.strict_equals(a, b)
    }

    #[inline(never)]
    pub(crate) fn loose_equals(&mut self, a: Value, b: Value) -> Run<bool> {
        if (a.is_number() && b.is_number())
            || (a.is_string() && b.is_string())
            || (a.is_object() && b.is_object())
            || (a.is_bool() && b.is_bool())
            || (a.is_symbol() && b.is_symbol())
            || (a.is_bigint() && b.is_bigint())
        {
            return Ok(self.strict_equals(a, b));
        }
        if a.is_nullish() || b.is_nullish() {
            return Ok(a.is_nullish() && b.is_nullish());
        }
        if a.is_object() {
            let a = self.to_primitive(a, Hint::Default)?;
            return self.loose_equals(a, b);
        }
        if b.is_object() {
            let b = self.to_primitive(b, Hint::Default)?;
            return self.loose_equals(a, b);
        }
        if a.is_symbol() || b.is_symbol() {
            return Ok(false);
        }
        if a.is_bigint() || b.is_bigint() {
            let (big, other) = if a.is_bigint() { (a, b) } else { (b, a) };
            if other.is_string() {
                let text = self.text_of(other).to_rust();
                return Ok(match self.parse_bigint(&text) {
                    Some(parsed) => self.strict_equals(big, parsed),
                    None => false,
                });
            }
            let number = self.to_number(other)?;
            return Ok(self.bigint_equals_number(big, number));
        }
        Ok(self.to_number(a)? == self.to_number(b)?)
    }

    pub(crate) fn instance_of(&mut self, value: Value, target: Value) -> Run<bool> {
        if !target.is_object() {
            return Err(self.type_error("Right-hand side of 'instanceof' is not callable"));
        }
        if let Some(prototype) = self.instance_of_prototype(target.handle()) {
            return Ok(value.is_object() && self.inherits_from(value.handle(), prototype));
        }
        let special = self.get(target, text::SYMBOL_HAS_INSTANCE)?;
        if !special.is_nullish() && special != self.function_has_instance {
            let result = self.call(special, target, &[value])?;
            return Ok(self.to_boolean(result));
        }
        if !self.is_callable(target) {
            return Err(self.type_error("Right-hand side of 'instanceof' is not callable"));
        }
        let found = self.ordinary_has_instance(value, target)?;
        if special == self.function_has_instance {
            self.remember_instance_of(target.handle());
        }
        Ok(found)
    }

    /// The `prototype` an `instanceof` with this function on the right compares with, if what it found last time on
    /// functions of its shape says that's all it has to do. Looking up `Symbol.hasInstance` and `prototype` each time
    /// was 7% of a bench that does it on classes.
    #[inline]
    fn instance_of_prototype(&self, function: u32) -> Option<u32> {
        let object = &self.objects[function as usize];
        let cached = &self.instance_of_cache[object.shape as usize % self.instance_of_cache.len()];
        if cached.shape != object.shape
            || cached.epoch != self.epoch
            || object.flags & LAZY_FUNCTION != 0
            || !matches!(object.kind, Kind::Function { .. })
        {
            return None;
        }
        let holder = if cached.holder == NONE {
            object
        } else {
            &self.objects[cached.holder as usize]
        };
        if holder.slots()[cached.has_slot as usize] != self.function_has_instance {
            return None;
        }
        let prototype = object.slots()[cached.prototype_slot as usize];
        prototype.is_object().then(|| prototype.handle())
    }

    /// Remembers for [`instance_of_prototype`](Self::instance_of_prototype) where a function's `Symbol.hasInstance`
    /// and `prototype` are, once an `instanceof` has found the usual `Symbol.hasInstance` on it, if both are plain
    /// values found through shapes.
    fn remember_instance_of(&mut self, function: u32) {
        let object = &self.objects[function as usize];
        let shape = object.shape;
        if object.flags & LAZY_FUNCTION != 0
            || !matches!(object.kind, Kind::Function { .. })
            || self.shapes.is_dict(shape)
        {
            return;
        }
        let Some((prototype_slot, flags)) = self.shapes.lookup(shape, text::PROTOTYPE) else {
            return;
        };
        if flags & ACCESSOR != 0 {
            return;
        }
        let mut at = function;
        let (holder, has_slot) = loop {
            let found = &self.objects[at as usize];
            if matches!(found.kind, Kind::Proxy(_)) {
                return;
            }
            if let Some((slot, flags)) = self.shapes.lookup(found.shape, text::SYMBOL_HAS_INSTANCE)
            {
                if flags & ACCESSOR != 0
                    || found.slots()[slot as usize] != self.function_has_instance
                {
                    return;
                }
                break (if at == function { NONE } else { at }, slot);
            }
            at = self.shapes.items[found.shape as usize].proto;
            if at == NONE {
                return;
            }
        };
        let way = shape as usize % self.instance_of_cache.len();
        self.instance_of_cache[way] = crate::vm::InstanceOfCache {
            shape,
            epoch: self.epoch,
            holder,
            has_slot,
            prototype_slot,
        };
    }

    /// Whether `prototype` is on the way up from `object`, as `instanceof` looks.
    fn inherits_from(&self, object: u32, prototype: u32) -> bool {
        let mut at = self.proto_of(object);
        while at != NONE {
            if at == prototype {
                return true;
            }
            at = self.proto_of(at);
        }
        false
    }

    pub(crate) fn ordinary_has_instance(&mut self, value: Value, target: Value) -> Run<bool> {
        if !value.is_object() {
            return Ok(false);
        }
        if let Kind::Bound(bound) = &self.objects[target.index()].kind {
            let inner = bound.target;
            return self.instance_of(value, inner);
        }
        let prototype = self.get(target, text::PROTOTYPE)?;
        if !prototype.is_object() {
            return Err(self.type_error("Function has non-object prototype in instanceof check"));
        }
        Ok(self.inherits_from(value.handle(), prototype.handle()))
    }

    // The slow ways through the operators.

    /// `(a + b) | 0` for AddInt32, when they aren't both ints: what Add and then BitOrInt would give.
    #[inline(never)]
    pub(crate) fn add_int32_slow(&mut self, a: Value, b: Value) -> Run<Value> {
        if a.is_number() && b.is_number() {
            return Ok(Value::int(f64_to_int32(a.as_number() + b.as_number())));
        }
        let sum = self.add_slow(a, b)?;
        self.bitwise_slow(BigOp::Or, sum, Value::int(0))
    }

    #[inline(never)]
    pub(crate) fn add_slow(&mut self, a: Value, b: Value) -> Run<Value> {
        // Numbers with `undefined`, `null` or a boolean, as from a parameter with no argument: no conversion to do.
        if let (Some(a), Some(b)) = (plain_number(a), plain_number(b)) {
            return Ok(Value::number(a + b));
        }
        if a.is_string() {
            if b.is_string() {
                return Ok(self.concat_two(a, b));
            }
            if b.is_int() {
                if let Some(joined) = self.short_join(&[a, b]) {
                    return Ok(joined);
                }
            }
        } else if a.is_int() && b.is_string() {
            if let Some(joined) = self.short_join(&[a, b]) {
                return Ok(joined);
            }
        }
        let a = self.to_primitive(a, Hint::Default)?;
        let b = self.to_primitive(b, Hint::Default)?;
        if a.is_string() || b.is_string() {
            let a = self.to_js_string(a)?;
            let b = self.to_js_string(b)?;
            return Ok(self.concat_two(a, b));
        }
        let (a, b) = (self.to_numeric(a)?, self.to_numeric(b)?);
        if a.is_bigint() || b.is_bigint() {
            return self.bigint_op(BigOp::Add, a, b);
        }
        Ok(Value::number(a.as_number() + b.as_number()))
    }

    fn concat_two(&mut self, a: Value, b: Value) -> Value {
        let left = self.strings.items[a.index()].text.len();
        let right = self.strings.items[b.index()].text.len();
        if right == 0 {
            return a;
        }
        if left == 0 {
            return b;
        }
        let (joined, new_bytes) = self.strings.concat(a.handle(), b.handle());
        self.debt += new_bytes + 32;
        let value = Value::string(self.strings.add(joined));
        self.nursery.push(value);
        value
    }

    /// Joins what's in some registers, as one string with only one copy of it all.
    #[inline(never)]
    pub(crate) fn concat_registers(&mut self, start: usize, count: usize) -> Run<Value> {
        if count <= 16 {
            let mut parts = [Value::UNDEFINED; 16];
            parts[..count].copy_from_slice(&self.stack[start..start + count]);
            if let Some(joined) = self.short_join(&parts[..count]) {
                return Ok(joined);
            }
        }
        let mut length = 0;
        let mut all_ascii = true;
        for at in 0..count {
            let value = self.stack[start + at];
            let string = if value.is_string() {
                value
            } else {
                // The string goes in the register, which keeps it for as long as it's wanted.
                let string = self.to_js_string(value)?;
                self.stack[start + at] = string;
                string
            };
            let text = &self.strings.items[string.index()].text;
            length += text.len();
            all_ascii &= text.ascii().is_some();
        }
        if count == 1 {
            return Ok(self.stack[start]);
        }
        let text = if all_ascii {
            let mut out = Vec::with_capacity(length);
            for at in 0..count {
                let part = self.stack[start + at];
                if let Some(bytes) = self.strings.items[part.index()].text.ascii() {
                    out.extend_from_slice(bytes);
                }
            }
            Text::from_ascii_vec(out)
        } else {
            let mut out = Vec::with_capacity(length);
            for at in 0..count {
                let part = self.stack[start + at];
                self.strings.items[part.index()].text.push_to(&mut out);
            }
            Text::Wide(out.into_boxed_slice())
        };
        Ok(self.new_text(text))
    }

    /// Strings and whole numbers joined, with no string made for each number and nothing on the heap, if they're all
    /// ASCII and it comes to something short.
    fn short_join(&mut self, parts: &[Value]) -> Option<Value> {
        let mut out = [0u8; 64];
        let mut length = 0;
        for part in parts {
            if part.is_string() {
                let bytes = self.strings.items[part.index()].text.ascii()?;
                out.get_mut(length..length + bytes.len())?
                    .copy_from_slice(bytes);
                length += bytes.len();
            } else if part.is_int() {
                let digits = int_text(part.as_int());
                let digits = digits.ascii()?;
                out.get_mut(length..length + digits.len())?
                    .copy_from_slice(digits);
                length += digits.len();
            } else {
                return None;
            }
        }
        Some(self.new_text(Text::from_ascii(&out[..length])))
    }

    /// An operator with something that's not a plain number on one side.
    fn numeric_slow(
        &mut self,
        op: BigOp,
        a: Value,
        b: Value,
        f: fn(f64, f64) -> f64,
    ) -> Run<Value> {
        let (a, b) = (self.to_numeric(a)?, self.to_numeric(b)?);
        if a.is_bigint() || b.is_bigint() {
            return self.bigint_op(op, a, b);
        }
        Ok(Value::number(f(a.as_number(), b.as_number())))
    }

    #[inline(never)]
    pub(crate) fn sub_slow(&mut self, a: Value, b: Value) -> Run<Value> {
        self.numeric_slow(BigOp::Sub, a, b, |x, y| x - y)
    }

    #[inline(never)]
    pub(crate) fn mul_slow(&mut self, a: Value, b: Value) -> Run<Value> {
        self.numeric_slow(BigOp::Mul, a, b, |x, y| x * y)
    }

    #[inline(never)]
    pub(crate) fn div_slow(&mut self, a: Value, b: Value) -> Run<Value> {
        self.numeric_slow(BigOp::Div, a, b, |x, y| x / y)
    }

    #[inline(never)]
    pub(crate) fn mod_slow(&mut self, a: Value, b: Value) -> Run<Value> {
        self.numeric_slow(BigOp::Mod, a, b, |x, y| x % y)
    }

    #[inline(never)]
    pub(crate) fn pow_slow(&mut self, a: Value, b: Value) -> Run<Value> {
        self.numeric_slow(BigOp::Pow, a, b, js_pow)
    }

    /// A bitwise operator on what isn't two ints.
    #[inline(never)]
    pub(crate) fn bitwise_slow(&mut self, op: BigOp, a: Value, b: Value) -> Run<Value> {
        let (a, b) = (self.to_numeric(a)?, self.to_numeric(b)?);
        if a.is_bigint() || b.is_bigint() {
            return self.bigint_op(op, a, b);
        }
        let (x, y) = (self.to_int32(a)?, self.to_int32(b)?);
        Ok(match op {
            BigOp::And => Value::int(x & y),
            BigOp::Or => Value::int(x | y),
            BigOp::Xor => Value::int(x ^ y),
            BigOp::Shl => Value::int(x.wrapping_shl(y as u32 & 31)),
            BigOp::Shr => Value::int(x.wrapping_shr(y as u32 & 31)),
            _ => Value::int_or_double((x as u32).wrapping_shr(y as u32 & 31) as usize),
        })
    }

    /// `++` or `--` on what isn't an int, or an int with no room left.
    #[inline(never)]
    pub(crate) fn step_slow(&mut self, value: Value, by: i32) -> Run<Value> {
        let numeric = self.to_numeric(value)?;
        Ok(if numeric.is_bigint() {
            self.bigint_step(numeric, i64::from(by))
        } else {
            Value::number(numeric.as_number() + f64::from(by))
        })
    }

    /// `-` on what isn't an int that can just change sign.
    #[inline(never)]
    pub(crate) fn negate_slow(&mut self, value: Value) -> Run<Value> {
        let numeric = self.to_numeric(value)?;
        Ok(if numeric.is_bigint() {
            self.bigint_negate(numeric)
        } else {
            Value::number(-numeric.as_number())
        })
    }

    /// The `TYPE_` number of what `typeof` would say.
    #[inline]
    pub(crate) fn type_kind(&self, value: Value) -> u16 {
        use crate::op::*;
        if value.is_number() {
            TYPE_NUMBER
        } else if value.is_string() {
            TYPE_STRING
        } else if value.is_object() {
            if self.is_callable(value) {
                TYPE_FUNCTION
            } else {
                TYPE_OBJECT
            }
        } else if value.is_undefined() {
            TYPE_UNDEFINED
        } else if value.is_bool() {
            TYPE_BOOLEAN
        } else if value.is_symbol() {
            TYPE_SYMBOL
        } else if value.is_bigint() {
            TYPE_BIGINT
        } else {
            TYPE_OBJECT
        }
    }

    /// `a < b` the long way. `None` means one of them is NaN.
    fn compare(&mut self, a: Value, b: Value, left_first: bool) -> Run<Option<bool>> {
        let (a, b) = if left_first {
            let a = self.to_primitive(a, Hint::Number)?;
            let b = self.to_primitive(b, Hint::Number)?;
            (a, b)
        } else {
            let b = self.to_primitive(b, Hint::Number)?;
            let a = self.to_primitive(a, Hint::Number)?;
            (a, b)
        };
        if a.is_string() && b.is_string() {
            let left = &self.strings.items[a.index()].text;
            let right = &self.strings.items[b.index()].text;
            let less = match (left.ascii(), right.ascii()) {
                (Some(x), Some(y)) => x < y,
                _ => left.units() < right.units(),
            };
            return Ok(Some(less));
        }
        if a.is_bigint() || b.is_bigint() {
            return self.compare_with_bigint(a, b);
        }
        let (x, y) = (self.to_number(a)?, self.to_number(b)?);
        if x.is_nan() || y.is_nan() {
            return Ok(None);
        }
        Ok(Some(x < y))
    }

    fn compare_with_bigint(&mut self, a: Value, b: Value) -> Run<Option<bool>> {
        if a.is_bigint() && b.is_bigint() {
            return Ok(Some(self.bigint_less_than(a, b)));
        }
        let big_first = a.is_bigint();
        let (big, other) = if big_first { (a, b) } else { (b, a) };
        if other.is_string() {
            let text = self.text_of(other).to_rust();
            let Some(parsed) = self.parse_bigint(&text) else {
                return Ok(None);
            };
            return Ok(Some(if big_first {
                self.bigint_less_than(big, parsed)
            } else {
                self.bigint_less_than(parsed, big)
            }));
        }
        let number = self.to_number(other)?;
        Ok(self.bigint_less_than_number(big, number, big_first))
    }

    #[inline(never)]
    pub(crate) fn less_than(&mut self, a: Value, b: Value) -> Run<bool> {
        Ok(self.compare(a, b, true)? == Some(true))
    }

    #[inline(never)]
    pub(crate) fn greater_than(&mut self, a: Value, b: Value) -> Run<bool> {
        Ok(self.compare(b, a, false)? == Some(true))
    }

    #[inline(never)]
    pub(crate) fn less_equal(&mut self, a: Value, b: Value) -> Run<bool> {
        Ok(self.compare(b, a, false)? == Some(false))
    }

    #[inline(never)]
    pub(crate) fn greater_equal(&mut self, a: Value, b: Value) -> Run<bool> {
        Ok(self.compare(a, b, true)? == Some(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_as_javascript_writes_them() {
        for (number, expected) in [
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (1.5e300, "1.5e+300"),
            (1e-7, "1e-7"),
            (0.000001, "0.000001"),
            (123456.789, "123456.789"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
            (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
            (0.30000000000000004, "0.30000000000000004"),
        ] {
            assert_eq!(number_to_string(number), expected);
            assert_eq!(number_text(number).to_rust(), expected);
        }
    }

    #[test]
    fn strings_become_numbers_as_javascript_says() {
        let read = |text: &str| string_to_number(&Text::from_str(text));
        assert_eq!(read("  42  "), 42.0);
        assert_eq!(read(""), 0.0);
        assert_eq!(read("0x10"), 16.0);
        assert_eq!(read("0b11"), 3.0);
        assert_eq!(read("1e3"), 1000.0);
        assert_eq!(read("-Infinity"), f64::NEG_INFINITY);
        assert!(read("inf").is_nan());
        assert!(read("nan").is_nan());
        assert!(read("12px").is_nan());
    }

    #[test]
    fn numbers_wrap_to_32_bits() {
        assert_eq!(f64_to_int32(2_147_483_648.0), -2_147_483_648);
        assert_eq!(f64_to_int32(4_294_967_296.0), 0);
        assert_eq!(f64_to_int32(-1.5), -1);
        assert_eq!(f64_to_int32(f64::NAN), 0);
        assert_eq!(f64_to_int32(-4_294_967_297.0), -1);
    }
}
