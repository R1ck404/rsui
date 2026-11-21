//! `Math`, `Number`, `Boolean` and the global functions that go with numbers and text.

use crate::convert::{f64_to_int32, number_to_string, trim_units};
use crate::object::*;
use crate::text::{self, Text};
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

macro_rules! unary {
    ($name:ident, $function:expr) => {
        fn $name(vm: &mut Vm, args: Args) -> Run<Value> {
            let value = vm.arg(&args, 0);
            let number = vm.to_number(value)?;
            let function: fn(f64) -> f64 = $function;
            Ok(Value::number(function(number)))
        }
    };
}

unary!(abs, f64::abs);
unary!(ceil, f64::ceil);
unary!(floor, f64::floor);
unary!(trunc, f64::trunc);
unary!(sqrt, f64::sqrt);
unary!(cbrt, f64::cbrt);
unary!(sin, f64::sin);
unary!(cos, f64::cos);
unary!(tan, f64::tan);
unary!(asin, f64::asin);
unary!(acos, f64::acos);
unary!(atan, f64::atan);
unary!(sinh, f64::sinh);
unary!(cosh, f64::cosh);
unary!(tanh, f64::tanh);
unary!(asinh, f64::asinh);
unary!(acosh, f64::acosh);
unary!(atanh, f64::atanh);
unary!(exp, f64::exp);
unary!(expm1, f64::exp_m1);
unary!(log, f64::ln);
unary!(log2, f64::log2);
unary!(log10, f64::log10);
unary!(log1p, f64::ln_1p);
fn sign_of(number: f64) -> f64 {
    if number.is_nan() || number == 0.0 {
        number
    } else {
        number.signum()
    }
}

fn rounded(number: f64) -> f64 {
    // JavaScript rounds halves up, even when that's towards zero.
    let floor = number.floor();
    if number - floor >= 0.5 {
        let up = floor + 1.0;
        if up == 0.0 && number < 0.0 { -0.0 } else { up }
    } else if floor == 0.0 && number.is_sign_negative() {
        -0.0
    } else {
        floor
    }
}

fn single(number: f64) -> f64 {
    f64::from(number as f32)
}

fn leading_zeros(number: f64) -> f64 {
    f64::from((f64_to_int32(number) as u32).leading_zeros())
}

unary!(sign, sign_of);
unary!(round, rounded);
unary!(fround, single);
unary!(clz32, leading_zeros);

fn power(a: f64, b: f64) -> f64 {
    crate::convert::js_pow(a, b)
}

fn pow(vm: &mut Vm, args: Args) -> Run<Value> {
    let (a, b) = (vm.arg(&args, 0), vm.arg(&args, 1));
    let (a, b) = (vm.to_number(a)?, vm.to_number(b)?);
    Ok(Value::number(power(a, b)))
}

fn atan2(vm: &mut Vm, args: Args) -> Run<Value> {
    let (a, b) = (vm.arg(&args, 0), vm.arg(&args, 1));
    let (a, b) = (vm.to_number(a)?, vm.to_number(b)?);
    Ok(Value::number(a.atan2(b)))
}

fn times_int32(a: f64, b: f64) -> f64 {
    f64::from(f64_to_int32(a).wrapping_mul(f64_to_int32(b)))
}

fn imul(vm: &mut Vm, args: Args) -> Run<Value> {
    let (a, b) = (vm.arg(&args, 0), vm.arg(&args, 1));
    let (a, b) = (vm.to_int32(a)?, vm.to_int32(b)?);
    Ok(Value::int(a.wrapping_mul(b)))
}

/// The smaller of what `Math.min` has so far and the next number, which is NaN once either is.
fn min_step(result: f64, number: f64) -> f64 {
    if number.is_nan() || result.is_nan() {
        f64::NAN
    } else if number < result || (number == 0.0 && result == 0.0 && number.is_sign_negative()) {
        number
    } else {
        result
    }
}

fn max_step(result: f64, number: f64) -> f64 {
    if number.is_nan() || result.is_nan() {
        f64::NAN
    } else if number > result || (number == 0.0 && result == 0.0 && result.is_sign_negative()) {
        number
    } else {
        result
    }
}

fn min_of_two(a: f64, b: f64) -> f64 {
    min_step(min_step(f64::INFINITY, a), b)
}

fn max_of_two(a: f64, b: f64) -> f64 {
    max_step(max_step(f64::NEG_INFINITY, a), b)
}

fn min(vm: &mut Vm, args: Args) -> Run<Value> {
    let mut result = f64::INFINITY;
    for index in 0..args.count {
        let value = vm.arg(&args, index);
        let number = vm.to_number(value)?;
        result = min_step(result, number);
    }
    Ok(Value::number(result))
}

fn max(vm: &mut Vm, args: Args) -> Run<Value> {
    let mut result = f64::NEG_INFINITY;
    for index in 0..args.count {
        let value = vm.arg(&args, index);
        let number = vm.to_number(value)?;
        result = max_step(result, number);
    }
    Ok(Value::number(result))
}

fn hypot_of_two(a: f64, b: f64) -> f64 {
    (0.0 + a * a + b * b).sqrt()
}

fn hypot(vm: &mut Vm, args: Args) -> Run<Value> {
    let mut sum = 0.0;
    for index in 0..args.count {
        let value = vm.arg(&args, index);
        let number = vm.to_number(value)?;
        sum += number * number;
    }
    Ok(Value::number(sum.sqrt()))
}

/// What a function of `Math` the interpreter can call itself works out from its numbers.
#[derive(Clone, Copy)]
pub(crate) enum Quick {
    One(fn(f64) -> f64),
    Two(fn(f64, f64) -> f64),
}

/// The functions of `Math` that `Op::CallMath` calls itself, by their place here, when it's called with numbers and
/// is still the built-in one: the name, the native, and what it works out, which is what the native does to numbers.
/// It saves making the native's arguments, calling it, and its looking at what they are. Each takes as many numbers
/// as the length it's made with.
pub(crate) const QUICK: [(&str, NativeFn, Quick); 30] = [
    ("abs", abs, Quick::One(f64::abs)),
    ("ceil", ceil, Quick::One(f64::ceil)),
    ("floor", floor, Quick::One(f64::floor)),
    ("trunc", trunc, Quick::One(f64::trunc)),
    ("round", round, Quick::One(rounded)),
    ("sign", sign, Quick::One(sign_of)),
    ("sqrt", sqrt, Quick::One(f64::sqrt)),
    ("cbrt", cbrt, Quick::One(f64::cbrt)),
    ("sin", sin, Quick::One(f64::sin)),
    ("cos", cos, Quick::One(f64::cos)),
    ("tan", tan, Quick::One(f64::tan)),
    ("asin", asin, Quick::One(f64::asin)),
    ("acos", acos, Quick::One(f64::acos)),
    ("atan", atan, Quick::One(f64::atan)),
    ("sinh", sinh, Quick::One(f64::sinh)),
    ("cosh", cosh, Quick::One(f64::cosh)),
    ("tanh", tanh, Quick::One(f64::tanh)),
    ("exp", exp, Quick::One(f64::exp)),
    ("expm1", expm1, Quick::One(f64::exp_m1)),
    ("log", log, Quick::One(f64::ln)),
    ("log2", log2, Quick::One(f64::log2)),
    ("log10", log10, Quick::One(f64::log10)),
    ("log1p", log1p, Quick::One(f64::ln_1p)),
    ("fround", fround, Quick::One(single)),
    ("pow", pow, Quick::Two(power)),
    ("atan2", atan2, Quick::Two(f64::atan2)),
    ("imul", imul, Quick::Two(times_int32)),
    ("min", min, Quick::Two(min_of_two)),
    ("max", max, Quick::Two(max_of_two)),
    ("hypot", hypot, Quick::Two(hypot_of_two)),
];

/// The place in [`QUICK`] of `Math.name` called with this many arguments, if it's there.
pub(crate) fn quick_index(name: &str, argc: u16) -> Option<u16> {
    QUICK
        .iter()
        .position(|(known, _, quick)| {
            *known == name && argc == if matches!(quick, Quick::One(_)) { 1 } else { 2 }
        })
        .map(|at| at as u16)
}

/// How many arguments `Math` function `which` in [`QUICK`] takes, which is how many a CallMath of it has.
pub(crate) fn quick_argc(which: u16) -> u16 {
    match QUICK[which as usize].2 {
        Quick::One(_) => 1,
        Quick::Two(_) => 2,
    }
}

/// The native that `Math` function `which` in [`QUICK`] is while it's the built-in one, for compiled code to compare
/// what it calls with, as `math_quick` does.
#[cfg(feature = "jit")]
pub(crate) fn quick_native(which: u16) -> NativeFn {
    QUICK[which as usize].1
}

/// Its name, for compiled code to do some itself.
#[cfg(feature = "jit")]
pub(crate) fn quick_name(which: u16) -> &'static str {
    QUICK[which as usize].0
}

/// What `Math` function `which` in [`QUICK`] works out from numbers, for compiled code, which has checked it's the
/// built-in one and has numbers (`y` is ignored by the ones that take one). The answer is what `math_quick` makes a
/// `Value::number` of.
/// On x86-64 compiled code calls with Windows' convention on every system; elsewhere with the system's own.
#[cfg(all(feature = "jit", target_arch = "x86_64"))]
pub(crate) extern "win64" fn quick_apply(which: u64, x: f64, y: f64) -> f64 {
    quick_work(which, x, y)
}
#[cfg(all(feature = "jit", not(target_arch = "x86_64")))]
pub(crate) extern "C" fn quick_apply(which: u64, x: f64, y: f64) -> f64 {
    quick_work(which, x, y)
}

#[cfg(feature = "jit")]
#[inline(always)]
fn quick_work(which: u64, x: f64, y: f64) -> f64 {
    match QUICK[which as usize].2 {
        Quick::One(function) => function(x),
        Quick::Two(function) => function(x, y),
    }
}

impl Vm {
    /// What `Math` function `which` in [`QUICK`] gives for the arguments from `args` on, if `callee` is that function
    /// and they're numbers; otherwise it has to be called (see `Op::CallMath`).
    ///
    /// # Safety
    ///
    /// `args` has as many arguments as the function takes.
    /// Makes the CallMath that's just run a Call, as what it calls isn't what it was made for, or with numbers.
    #[inline(never)]
    pub(crate) fn go_plain_call(&mut self) {
        use crate::op::Op;
        let Some(frame) = self.frames.last() else {
            return;
        };
        if let Some(op) = self.code.get_mut(frame.pc.wrapping_sub(1) as usize) {
            if let Op::CallMath { dst, f, which } = *op {
                let argc = quick_argc(which);
                *op = Op::Call { dst, f, argc };
            }
        }
    }

    #[inline(never)]
    #[allow(unsafe_code)]
    pub(crate) unsafe fn math_quick(&self, which: u16, callee: Value, args: *const Value) -> Option<Value> {
        let (_, native, quick) = *QUICK.get(which as usize)?;
        if !callee.is_object() {
            return None;
        }
        let Kind::Native(found) = &self.objects[callee.index()].kind else {
            return None;
        };
        if found.func as usize != native as usize {
            return None;
        }
        let x = unsafe { *args };
        if !x.is_number() {
            return None;
        }
        let x = x.as_number();
        Some(Value::number(match quick {
            Quick::One(function) => function(x),
            Quick::Two(function) => {
                let y = unsafe { *args.add(1) };
                if !y.is_number() {
                    return None;
                }
                function(x, y.as_number())
            }
        }))
    }
}

fn random(vm: &mut Vm, _: Args) -> Run<Value> {
    // A small generator: xorshift64*. It's for shuffling, not for secrets.
    if vm.random_state == 0 {
        let now = crate::inspect::random_seed().unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |time| time.as_nanos() as u64)
        });
        vm.random_state = now | 1;
    }
    let mut state = vm.random_state;
    state ^= state >> 12;
    state ^= state << 25;
    state ^= state >> 27;
    vm.random_state = state;
    let bits = state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11;
    Ok(Value::double(bits as f64 / (1u64 << 53) as f64))
}

fn this_number(vm: &mut Vm, this: Value) -> Run<f64> {
    if this.is_number() {
        return Ok(this.as_number());
    }
    if this.is_object() {
        if let Kind::Boxed(value) = vm.objects[this.index()].kind {
            if value.is_number() {
                return Ok(value.as_number());
            }
        }
    }
    Err(vm.type_error("This isn't a number"))
}

fn number_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = if args.count == 0 {
        0.0
    } else {
        let value = vm.arg(&args, 0);
        let numeric = vm.to_numeric(value)?;
        if numeric.is_bigint() {
            vm.big_of(numeric).to_f64()
        } else {
            numeric.as_number()
        }
    };
    let value = Value::number(number);
    if args.new_target.is_object() {
        let proto = vm.proto_for(&args, vm.realm.number_proto)?;
        return Ok(vm.new_object_with_proto(proto, Kind::Boxed(value)));
    }
    Ok(value)
}

const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

fn to_radix(mut number: f64, radix: u32) -> String {
    if number.is_nan() {
        return "NaN".to_owned();
    }
    if number.is_infinite() {
        return if number > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        }
        .to_owned();
    }
    let negative = number < 0.0;
    number = number.abs();
    let mut whole = number.trunc();
    let mut fraction = number - whole;
    let mut out = Vec::new();
    if whole == 0.0 {
        out.push(b'0');
    }
    let base = f64::from(radix);
    while whole > 0.0 {
        let digit = (whole % base) as usize;
        out.push(DIGITS[digit]);
        whole = (whole / base).trunc();
    }
    if negative {
        out.push(b'-');
    }
    out.reverse();
    if fraction > 0.0 {
        out.push(b'.');
        // As many digits as it takes, up to what a double can actually say.
        let mut precision = f64::EPSILON.max(number * f64::EPSILON / 2.0);
        for _ in 0..1100 {
            fraction *= base;
            precision *= base;
            let digit = fraction.trunc();
            out.push(DIGITS[digit as usize]);
            fraction -= digit;
            if fraction < precision || 1.0 - fraction < precision {
                break;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn number_to_string_method(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = this_number(vm, args.this)?;
    let radix = vm.arg(&args, 0);
    let radix = if radix.is_undefined() {
        10.0
    } else {
        vm.to_integer(radix)?
    };
    if !(2.0..=36.0).contains(&radix) {
        return Err(vm.range_error("toString() radix must be between 2 and 36"));
    }
    let text = if radix == 10.0 {
        number_to_string(number)
    } else {
        to_radix(number, radix as u32)
    };
    Ok(vm.new_str(&text))
}

fn value_of(vm: &mut Vm, args: Args) -> Run<Value> {
    Ok(Value::number(this_number(vm, args.this)?))
}

fn to_locale_string(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = this_number(vm, args.this)?;
    let locale = {
        let given = vm.arg(&args, 0);
        let first = match vm.array_elements(given) {
            Some(list) => list.first().copied().unwrap_or(Value::UNDEFINED),
            None => given,
        };
        if first.is_string() {
            Some(vm.to_rust_string(first)?)
        } else {
            None
        }
    };
    let given = vm.arg(&args, 1);
    let options = NumberOptions::read(vm, given)?;
    // A language Windows doesn't know, as `"default"` isn't one, is the user's, as a browser has it.
    let locale = super::locale::locale_of(locale.as_deref());
    let text = options.format(number, locale.name.as_deref());
    Ok(vm.new_str(
        &match options.numbering.as_deref().or(locale.numbering.as_deref()) {
            Some(system) => super::locale::digits_in(&text, system, true),
            None => text,
        },
    ))
}

/// What's asked for in the options of `toLocaleString`, as `Intl.NumberFormat` has them.
struct NumberOptions {
    kind: NumberKind,
    least_digits: usize,
    most_digits: usize,
    whole_digits: usize,
    grouping: bool,
    compact: bool,
    /// How many places were asked for, if they were, which goes for compact numbers too.
    chosen_digits: Option<(usize, usize)>,
    /// The digits it's written in (`numberingSystem`), if they're ones that are known.
    numbering: Option<String>,
}

#[derive(PartialEq)]
enum NumberKind {
    Plain,
    Percent,
    Currency(String),
}

impl NumberOptions {
    fn read(vm: &mut Vm, given: Value) -> Run<Self> {
        let read = |vm: &mut Vm, name: &str| -> Run<Value> {
            if !given.is_object() {
                return Ok(Value::UNDEFINED);
            }
            let atom = vm.intern_str(name);
            vm.get(given, atom)
        };
        let text = |vm: &mut Vm, name: &str| -> Run<Option<String>> {
            let value = read(vm, name)?;
            if value.is_undefined() {
                Ok(None)
            } else {
                Ok(Some(vm.to_rust_string(value)?))
            }
        };
        let digits = |vm: &mut Vm, name: &str, least: f64, most: f64| -> Run<Option<usize>> {
            let value = read(vm, name)?;
            if value.is_undefined() {
                return Ok(None);
            }
            let number = vm.to_number(value)?;
            if !(number >= least && number <= most) {
                return Err(vm.range_error(&format!("{name} value is out of range.")));
            }
            Ok(Some(number.floor() as usize))
        };
        let kind = match text(vm, "style")?.as_deref() {
            None | Some("decimal") => NumberKind::Plain,
            Some("percent") => NumberKind::Percent,
            Some("currency") => match text(vm, "currency")? {
                Some(code) => NumberKind::Currency(code.to_uppercase()),
                None => return Err(vm.type_error("Currency code is required with currency style.")),
            },
            Some(other) => {
                return Err(vm.range_error(&format!(
                    "Value {other} out of range for Intl.NumberFormat options property style"
                )));
            }
        };
        let (usual_least, usual_most) = match &kind {
            NumberKind::Plain => (0, 3),
            NumberKind::Percent => (0, 0),
            // Some money doesn't have anything smaller than a whole one.
            NumberKind::Currency(code)
                if matches!(code.as_str(), "JPY" | "KRW" | "VND" | "CLP") =>
            {
                (0, 0)
            }
            NumberKind::Currency(_) => (2, 2),
        };
        let least = digits(vm, "minimumFractionDigits", 0.0, 100.0)?;
        let most = digits(vm, "maximumFractionDigits", 0.0, 100.0)?;
        let chosen_digits = (least.is_some() || most.is_some()).then(|| {
            let least = least.unwrap_or(0);
            (least, most.unwrap_or(usual_most.max(least)))
        });
        let (least_digits, most_digits) = match (least, most) {
            (None, None) => (usual_least, usual_most),
            (Some(least), None) => (least, usual_most.max(least)),
            (None, Some(most)) => (usual_least.min(most), most),
            (Some(least), Some(most)) => {
                if least > most {
                    return Err(vm.range_error("maximumFractionDigits value is out of range."));
                }
                (least, most)
            }
        };
        let whole_digits = digits(vm, "minimumIntegerDigits", 1.0, 21.0)?.unwrap_or(1);
        let grouping = read(vm, "useGrouping")?;
        let grouping = grouping.is_undefined() || vm.to_boolean(grouping);
        let compact = text(vm, "notation")?.as_deref() == Some("compact");
        let numbering =
            text(vm, "numberingSystem")?.filter(|system| super::locale::zero_of(system).is_some());
        Ok(Self {
            numbering,
            kind,
            least_digits,
            most_digits,
            whole_digits,
            grouping,
            compact,
            chosen_digits,
        })
    }

    fn format(&self, number: f64, locale: Option<&str>) -> String {
        let (decimal, group, percent_after, money_after) = separators_of(locale);
        let negative = number.is_sign_negative() && !number.is_nan();
        let mut body = if number.is_nan() {
            "NaN".to_owned()
        } else if number.is_infinite() {
            "\u{221E}".to_owned()
        } else {
            let shift = if self.kind == NumberKind::Percent {
                2
            } else {
                0
            };
            self.digits_of(number.abs(), shift, &decimal, &group)
        };
        match &self.kind {
            NumberKind::Plain => {}
            NumberKind::Percent => {
                body = if percent_after {
                    format!("{body}\u{A0}%")
                } else {
                    format!("{body}%")
                };
                if locale.is_none_or(|locale| locale.starts_with("en")) {
                    body = body.replace('\u{A0}', "");
                }
            }
            NumberKind::Currency(code) => {
                let symbol = match code.as_str() {
                    "USD" => "$",
                    "EUR" => "\u{20AC}",
                    "GBP" => "\u{A3}",
                    "JPY" => "\u{A5}",
                    "INR" => "\u{20B9}",
                    "KRW" => "\u{20A9}",
                    "CNY" => "CN\u{A5}",
                    "BRL" => "R$",
                    "CAD" => "CA$",
                    "AUD" => "A$",
                    "MXN" => "MX$",
                    other => other,
                };
                let spaced =
                    symbol.len() == 3 && symbol.is_ascii() && symbol != "CA$" && symbol != "MX$";
                body = if money_after {
                    format!("{body}\u{A0}{symbol}")
                } else if spaced {
                    format!("{symbol}\u{A0}{body}")
                } else {
                    format!("{symbol}{body}")
                };
            }
        }
        if negative {
            body.insert(0, '-');
        }
        body
    }

    fn digits_of(&self, value: f64, shift: i32, decimal: &str, group: &str) -> String {
        if self.compact {
            let moved = value * 10f64.powi(shift);
            return compact_digits(moved, decimal, group, self.chosen_digits);
        }
        let exact = rounded_decimal(value, shift, self.most_digits);
        let (whole, fraction) = exact
            .split_once('.')
            .map_or((exact.as_str(), ""), |(whole, fraction)| (whole, fraction));
        let fraction = {
            let kept = fraction.trim_end_matches('0').len().max(self.least_digits);
            &fraction[..kept.min(fraction.len())]
        };
        let padded = format!(
            "{}{whole}",
            "0".repeat(self.whole_digits.saturating_sub(whole.len()))
        );
        let mut out = grouped(&padded, if self.grouping { group } else { "" });
        if !fraction.is_empty() {
            out.push_str(decimal);
            out.push_str(fraction);
        }
        out
    }
}

fn grouped(whole: &str, group: &str) -> String {
    let mut out = String::with_capacity(whole.len() + whole.len() / 3);
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            out.push_str(group);
        }
        out.push(digit);
    }
    out
}

/// A number rounded to this many places, after moving its point `shift` places to the right, with halves going up. This
/// goes by the shortest digits that read back as the same number, as people expect, so that `1.005` rounds up to `1.01`,
/// though the closest a number can get to that is slightly below it.
fn rounded_decimal(value: f64, shift: i32, places: usize) -> String {
    // Moving the point doesn't make any more of nothing.
    let shift = if value == 0.0 { 0 } else { shift };
    let written = format!("{value:e}");
    let (digits, power) = written.split_once('e').unwrap_or((&written, "0"));
    let mut digits: Vec<u8> = digits.bytes().filter(|byte| *byte != b'.').collect();
    let mut point = power.parse::<i32>().unwrap_or(0) + 1 + shift;
    let keep = point + places as i32;
    if keep < 0 {
        digits.clear();
    } else if (keep as usize) < digits.len() {
        let up = digits[keep as usize] >= b'5';
        digits.truncate(keep as usize);
        if up {
            let mut at = digits.len();
            loop {
                if at == 0 {
                    digits.insert(0, b'1');
                    point += 1;
                    break;
                }
                at -= 1;
                if digits[at] == b'9' {
                    digits[at] = b'0';
                } else {
                    digits[at] += 1;
                    break;
                }
            }
        }
    }
    let digit_at = |index: i32| -> char {
        if index >= 0 && (index as usize) < digits.len() {
            digits[index as usize] as char
        } else {
            '0'
        }
    };
    let mut out = String::new();
    if point <= 0 {
        out.push('0');
    } else {
        out.extend((0..point).map(digit_at));
    }
    if places > 0 {
        out.push('.');
        out.extend((0..places as i32).map(|index| digit_at(point + index)));
    }
    out
}

/// `1.2K` and `34M`, as English writes them: two digits, or as many as there are above the point.
fn compact_digits(
    value: f64,
    decimal: &str,
    group: &str,
    chosen: Option<(usize, usize)>,
) -> String {
    const STEPS: [(f64, &str); 5] = [(1e12, "T"), (1e9, "B"), (1e6, "M"), (1e3, "K"), (1.0, "")];
    let at = STEPS
        .iter()
        .position(|(step, _)| value >= *step)
        .unwrap_or(STEPS.len() - 1);
    let write = |at: usize| {
        let scaled = value / STEPS[at].0;
        let places = if let Some((_, most)) = chosen {
            most
        } else if scaled <= 0.0 {
            0
        } else {
            (1 - scaled.log10().floor() as i32).max(0) as usize
        };
        rounded_decimal(scaled, 0, places)
    };
    let mut at = at;
    let mut text = write(at);
    // Rounding can take it up to the next one: 999.9 is `1K`, not `1000`.
    while at > 0 && text.split('.').next().is_some_and(|whole| whole.len() > 3) {
        at -= 1;
        text = write(at);
    }
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let least = chosen.map_or(0, |(least, _)| least);
    let kept = fraction.trim_end_matches('0').len().max(least);
    let fraction = &fraction[..kept.min(fraction.len())];
    let mut out = grouped(whole, group);
    if !fraction.is_empty() {
        out.push_str(decimal);
        out.push_str(fraction);
    }
    out.push_str(STEPS[at].1);
    out
}

/// What a language writes between the parts of a number, and whether it writes `%` and money after the number and
/// a space, for the languages most used. Others write numbers as English does.
#[cfg(not(windows))]
fn separators_of(locale: Option<&str>) -> (String, String, bool, bool) {
    let users = || {
        ["LC_ALL", "LC_NUMERIC", "LANG"]
            .into_iter()
            .filter_map(|name| std::env::var(name).ok())
            .find(|value| !value.is_empty() && value != "C" && value != "POSIX")
            .map(|value| value.split(['.', '@']).next().unwrap_or_default().replace('_', "-"))
            .unwrap_or_default()
    };
    let locale = locale.map_or_else(users, str::to_owned).to_ascii_lowercase();
    let language = locale.split('-').next().unwrap_or_default();
    let text = |decimal: &str, group: &str, percent_after: bool, money_after: bool| {
        (decimal.to_owned(), group.to_owned(), percent_after, money_after)
    };
    match language {
        "de" if locale == "de-ch" => text(".", "\u{2019}", false, false),
        "de" | "nl" | "id" | "it" | "es" | "pt" | "tr" | "da" | "el" | "ro" | "hr" | "sl" | "sr" | "vi" => {
            text(",", ".", language == "de" || language == "da" || language == "tr", true)
        }
        "fr" | "ru" | "pl" | "cs" | "sk" | "fi" | "nb" | "no" | "sv" | "uk" | "bg" | "hu" | "lt" | "lv" | "et" => {
            text(",", "\u{A0}", true, true)
        }
        _ => text(".", ",", false, false),
    }
}

/// What a language writes between the parts of a number, and whether it writes `%` and money after the number and
/// a space. Windows knows this for every language, and it's only asked about ones other than the most common.
#[cfg(windows)]
#[allow(unsafe_code)]
fn separators_of(locale: Option<&str>) -> (String, String, bool, bool) {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocaleInfoEx(name: *const u16, kind: u32, out: *mut u16, length: i32) -> i32;
    }
    const DECIMAL: u32 = 0x0E;
    const GROUP: u32 = 0x0F;
    const MONEY_ORDER: u32 = 0x1B;
    const PERCENT_ORDER: u32 = 0x75;
    let english = (".".to_owned(), ",".to_owned(), false, false);
    if locale.is_some_and(|locale| locale == "en" || locale.starts_with("en-")) {
        return english;
    }
    let name: Option<Vec<u16>> = locale.map(|locale| locale.encode_utf16().chain([0]).collect());
    let ask = |kind: u32| -> Option<String> {
        let mut out = [0u16; 16];
        let pointer = name.as_ref().map_or(std::ptr::null(), |name| name.as_ptr());
        // SAFETY: the name ends in a terminator or isn't there, which means the user's language, and it writes
        // no more than the room it's told there is.
        let length = unsafe { GetLocaleInfoEx(pointer, kind, out.as_mut_ptr(), out.len() as i32) };
        (length > 1).then(|| String::from_utf16_lossy(&out[..length as usize - 1]))
    };
    let (Some(decimal), Some(group)) = (ask(DECIMAL), ask(GROUP)) else {
        return english;
    };
    // Spaces in numbers don't break across lines, and what ICU writes isn't what Windows does for all of them.
    let group = group.replace(' ', "\u{A0}");
    let percent_after = ask(PERCENT_ORDER).as_deref() == Some("0");
    let money_after = matches!(ask(MONEY_ORDER).as_deref(), Some("1" | "3"));
    (decimal, group, percent_after, money_after)
}

/// Rounds a number's exact decimal expansion to this many places, with halves going away from zero.
pub(crate) fn fixed(number: f64, places: usize) -> String {
    if let Some(text) = fixed_by_integers(number, places) {
        return text;
    }
    let negative = number < 0.0;
    let exact = format!("{:.*}", places + 40, number.abs());
    let keep = exact.len() - 40;
    let round_up = exact.as_bytes()[keep] >= b'5';
    let mut digits: Vec<u8> = exact.as_bytes()[..keep].to_vec();
    if digits.last() == Some(&b'.') {
        digits.pop();
    }
    if round_up {
        let mut at = digits.len();
        let mut carry = true;
        while carry && at > 0 {
            at -= 1;
            if digits[at] == b'.' {
                continue;
            }
            if digits[at] == b'9' {
                digits[at] = b'0';
            } else {
                digits[at] += 1;
                carry = false;
            }
        }
        if carry {
            digits.insert(0, b'1');
        }
    }
    let text = String::from_utf8_lossy(&digits).into_owned();
    if negative { format!("-{text}") } else { text }
}

/// [`fixed`] with whole numbers. A double is a whole number times a power of two, so moving its point `places` to the
/// right and rounding is exact in 128 bits for anything below about 2^70 with up to 20 places, which is what people
/// write.
fn fixed_by_integers(number: f64, places: usize) -> Option<String> {
    if places > 20 || !number.is_finite() {
        return None;
    }
    let bits = number.abs().to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = u128::from(bits & ((1 << 52) - 1));
    let (mantissa, power) = if exponent == 0 {
        (fraction, -1074)
    } else {
        (fraction | 1 << 52, exponent - 1075)
    };
    let scale = 10u128.pow(places as u32);
    let scaled = if power >= 0 {
        if power > 64 {
            return None;
        }
        (mantissa << power).checked_mul(scale)?
    } else {
        // The product is below 2^120, so adding a half for rounding up can't overflow, and past 127 places it's all
        // below a half.
        let shift = power.unsigned_abs();
        let product = mantissa * scale;
        if shift >= 128 {
            0
        } else {
            (product + (1u128 << (shift - 1))) >> shift
        }
    };
    let mut digits = scaled.to_string().into_bytes();
    if digits.len() <= places {
        let mut padded = vec![b'0'; places + 1 - digits.len()];
        padded.append(&mut digits);
        digits = padded;
    }
    let mut out = String::with_capacity(digits.len() + 2);
    if number < 0.0 {
        out.push('-');
    }
    let point = digits.len() - places;
    out.extend(digits[..point].iter().map(|byte| char::from(*byte)));
    if places > 0 {
        out.push('.');
        out.extend(digits[point..].iter().map(|byte| char::from(*byte)));
    }
    Some(out)
}

fn to_fixed(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = this_number(vm, args.this)?;
    let places = vm.arg(&args, 0);
    let places = vm.to_integer(places)?;
    if !(0.0..=100.0).contains(&places) {
        return Err(vm.range_error("toFixed() digits argument must be between 0 and 100"));
    }
    if !number.is_finite() || number.abs() >= 1e21 {
        return Ok(vm.new_str(&number_to_string(number)));
    }
    Ok(vm.new_str(&fixed(number, places as usize)))
}

fn exponent_form(number: f64, digits: Option<usize>) -> String {
    let formatted = match digits {
        Some(digits) => format!("{number:.digits$e}"),
        None => format!("{number:e}"),
    };
    let (mantissa, exponent) = formatted
        .split_once('e')
        .expect("this format always has an exponent");
    let exponent: i32 = exponent
        .parse()
        .expect("this format's exponent is a number");
    format!(
        "{mantissa}e{}{}",
        if exponent >= 0 { '+' } else { '-' },
        exponent.abs()
    )
}

fn to_exponential(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = this_number(vm, args.this)?;
    let digits = vm.arg(&args, 0);
    if !number.is_finite() {
        return Ok(vm.new_str(&number_to_string(number)));
    }
    let digits = if digits.is_undefined() {
        None
    } else {
        let count = vm.to_integer(digits)?;
        if !(0.0..=100.0).contains(&count) {
            return Err(vm.range_error("toExponential() argument must be between 0 and 100"));
        }
        Some(count as usize)
    };
    Ok(vm.new_str(&exponent_form(number, digits)))
}

fn to_precision(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = this_number(vm, args.this)?;
    let precision = vm.arg(&args, 0);
    if precision.is_undefined() || !number.is_finite() {
        return Ok(vm.new_str(&number_to_string(number)));
    }
    let precision = vm.to_integer(precision)?;
    if !(1.0..=100.0).contains(&precision) {
        return Err(vm.range_error("toPrecision() argument must be between 1 and 100"));
    }
    let precision = precision as usize;
    if number == 0.0 {
        return Ok(vm.new_str(&fixed(0.0, precision - 1)));
    }
    let formatted = format!("{:.*e}", precision - 1, number.abs());
    let exponent: i32 = formatted
        .split_once('e')
        .and_then(|(_, exponent)| exponent.parse().ok())
        .unwrap_or(0);
    if exponent < -6 || exponent >= precision as i32 {
        return Ok(vm.new_str(&exponent_form(number, Some(precision - 1))));
    }
    let places = (precision as i32 - 1 - exponent).max(0) as usize;
    Ok(vm.new_str(&fixed(number, places)))
}

fn parse_float(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let units = vm.to_units(value)?;
    let trimmed = trim_units(&units, true, false);
    let text = String::from_utf16_lossy(trimmed);
    let bytes = text.as_bytes();
    let mut end = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        end = 1;
    }
    if text[end..].starts_with("Infinity") {
        let infinity = if bytes[0] == b'-' {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
        return Ok(Value::double(infinity));
    }
    let digits_start = end;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    if bytes.get(end) == Some(&b'.') {
        end += 1;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
    }
    if end == digits_start || (end == digits_start + 1 && bytes[digits_start] == b'.') {
        return Ok(Value::double(f64::NAN));
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut after = end + 1;
        if matches!(bytes.get(after), Some(b'+' | b'-')) {
            after += 1;
        }
        if bytes.get(after).is_some_and(u8::is_ascii_digit) {
            while bytes.get(after).is_some_and(u8::is_ascii_digit) {
                after += 1;
            }
            end = after;
        }
    }
    Ok(Value::number(text[..end].parse().unwrap_or(f64::NAN)))
}

fn parse_int(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    if value.is_int() && args.count < 2 {
        return Ok(value);
    }
    let units = vm.to_units(value)?;
    let trimmed = trim_units(&units, true, false);
    let radix = vm.arg(&args, 1);
    let mut radix = vm.to_int32(radix)?;
    let mut at = 0;
    let mut negative = false;
    if let Some(first) = trimmed.first() {
        if *first == u16::from(b'-') || *first == u16::from(b'+') {
            negative = *first == u16::from(b'-');
            at = 1;
        }
    }
    if radix != 0 && !(2..=36).contains(&radix) {
        return Ok(Value::double(f64::NAN));
    }
    let has_hex_prefix = trimmed.get(at) == Some(&u16::from(b'0'))
        && matches!(trimmed.get(at + 1), Some(&unit) if unit == u16::from(b'x') || unit == u16::from(b'X'));
    if (radix == 0 || radix == 16) && has_hex_prefix {
        at += 2;
        radix = 16;
    }
    if radix == 0 {
        radix = 10;
    }
    let mut result = 0f64;
    let mut any = false;
    for unit in &trimmed[at..] {
        let Some(digit) = char::from_u32(u32::from(*unit)).and_then(|c| c.to_digit(radix as u32))
        else {
            break;
        };
        result = result * f64::from(radix) + f64::from(digit);
        any = true;
    }
    if !any {
        return Ok(Value::double(f64::NAN));
    }
    Ok(Value::number(if negative { -result } else { result }))
}

fn is_nan_global(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    Ok(Value::bool(vm.to_number(value)?.is_nan()))
}

fn is_finite_global(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    Ok(Value::bool(vm.to_number(value)?.is_finite()))
}

fn number_is_nan(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    Ok(Value::bool(value.is_number() && value.as_number().is_nan()))
}

fn number_is_finite(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    Ok(Value::bool(
        value.is_number() && value.as_number().is_finite(),
    ))
}

fn number_is_integer(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    Ok(Value::bool(
        value.is_number() && value.as_number().is_finite() && value.as_number().fract() == 0.0,
    ))
}

fn number_is_safe_integer(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    Ok(Value::bool(
        value.is_number()
            && value.as_number().fract() == 0.0
            && value.as_number().abs() <= 9_007_199_254_740_991.0,
    ))
}

fn boolean_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let value = Value::bool(vm.to_boolean(value));
    if args.new_target.is_object() {
        let proto = vm.proto_for(&args, vm.realm.boolean_proto)?;
        return Ok(vm.new_object_with_proto(proto, Kind::Boxed(value)));
    }
    Ok(value)
}

fn this_boolean(vm: &mut Vm, this: Value) -> Run<Value> {
    if this.is_bool() {
        return Ok(this);
    }
    if this.is_object() {
        if let Kind::Boxed(value) = vm.objects[this.index()].kind {
            if value.is_bool() {
                return Ok(value);
            }
        }
    }
    Err(vm.type_error("This isn't a boolean"))
}

fn boolean_to_string(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = this_boolean(vm, args.this)?;
    vm.to_js_string(value)
}

fn boolean_value_of(vm: &mut Vm, args: Args) -> Run<Value> {
    this_boolean(vm, args.this)
}

// Encoding text for URLs.

const URI_RESERVED: &str = ";/?:@&=+$,#";
const URI_UNESCAPED: &str = "-_.!~*'()";

fn encode(vm: &mut Vm, value: Value, keep: &str) -> Run<Value> {
    let units = vm.to_units(value)?;
    let mut out = String::with_capacity(units.len());
    for decoded in char::decode_utf16(units.iter().copied()) {
        let Ok(character) = decoded else {
            return Err(vm.make_uri_error());
        };
        if character.is_ascii_alphanumeric() || keep.contains(character) {
            out.push(character);
        } else {
            let mut buffer = [0; 4];
            for byte in character.encode_utf8(&mut buffer).bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    Ok(vm.new_str(&out))
}

fn decode(vm: &mut Vm, value: Value, keep: &str) -> Run<Value> {
    let units = vm.to_units(value)?;
    let mut out: Vec<u16> = Vec::with_capacity(units.len());
    let mut at = 0;
    let hex_at = |at: usize| -> Option<u8> {
        if units.get(at) != Some(&u16::from(b'%')) {
            return None;
        }
        let high = char::from_u32(u32::from(*units.get(at + 1)?))?.to_digit(16)?;
        let low = char::from_u32(u32::from(*units.get(at + 2)?))?.to_digit(16)?;
        Some((high * 16 + low) as u8)
    };
    while at < units.len() {
        if units[at] != u16::from(b'%') {
            out.push(units[at]);
            at += 1;
            continue;
        }
        let Some(first) = hex_at(at) else {
            return Err(vm.make_uri_error());
        };
        if first < 0x80 {
            if keep.contains(first as char) {
                out.extend_from_slice(&units[at..at + 3]);
            } else {
                out.push(u16::from(first));
            }
            at += 3;
            continue;
        }
        let count = match first {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => return Err(vm.make_uri_error()),
        };
        let mut bytes = Vec::with_capacity(count);
        for index in 0..count {
            let Some(byte) = hex_at(at + index * 3) else {
                return Err(vm.make_uri_error());
            };
            bytes.push(byte);
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return Err(vm.make_uri_error());
        };
        out.extend(text.encode_utf16());
        at += count * 3;
    }
    Ok(vm.new_text(Text::from_units(&out)))
}

impl Vm {
    fn make_uri_error(&mut self) -> Value {
        match self.get_named(Value::object(self.global), "URIError") {
            Ok(constructor) => match self.get(constructor, text::PROTOTYPE) {
                Ok(proto) if proto.is_object() => self.make_error(proto.handle(), "URI malformed"),
                _ => self.type_error("URI malformed"),
            },
            Err(error) => error,
        }
    }
}

fn escape(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let units = vm.to_units(value)?;
    let mut out = String::with_capacity(units.len());
    for unit in units {
        match char::from_u32(u32::from(unit)) {
            Some(c) if c.is_ascii_alphanumeric() || "@*_+-./".contains(c) => out.push(c),
            _ if unit < 256 => out.push_str(&format!("%{unit:02X}")),
            _ => out.push_str(&format!("%u{unit:04X}")),
        }
    }
    Ok(vm.new_str(&out))
}

fn unescape(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let units = vm.to_units(value)?;
    let hex = |slice: &[u16]| -> Option<u32> {
        let mut result = 0;
        for unit in slice {
            result = result * 16 + char::from_u32(u32::from(*unit))?.to_digit(16)?;
        }
        Some(result)
    };
    let mut out = Vec::with_capacity(units.len());
    let mut at = 0;
    while at < units.len() {
        if units[at] == u16::from(b'%') {
            if units.get(at + 1) == Some(&u16::from(b'u')) {
                if let Some(code) = units.get(at + 2..at + 6).and_then(hex) {
                    out.push(code as u16);
                    at += 6;
                    continue;
                }
            } else if let Some(code) = units.get(at + 1..at + 3).and_then(hex) {
                out.push(code as u16);
                at += 3;
                continue;
            }
        }
        out.push(units[at]);
        at += 1;
    }
    Ok(vm.new_text(Text::from_units(&out)))
}

fn encode_uri_component(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    encode(vm, value, URI_UNESCAPED)
}

fn encode_uri(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    encode(vm, value, &format!("{URI_RESERVED}{URI_UNESCAPED}"))
}

fn decode_uri_component(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    decode(vm, value, "")
}

fn decode_uri(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    decode(vm, value, URI_RESERVED)
}

pub(super) fn install(vm: &mut Vm) {
    let math = vm.plain_proto();
    let math_atom = vm.intern_str("Math");
    vm.add_property(vm.global, math_atom, Value::object(math), HIDDEN);
    for (name, value) in [
        ("PI", std::f64::consts::PI),
        ("E", std::f64::consts::E),
        ("LN2", std::f64::consts::LN_2),
        ("LN10", std::f64::consts::LN_10),
        ("LOG2E", std::f64::consts::LOG2_E),
        ("LOG10E", std::f64::consts::LOG10_E),
        ("SQRT2", std::f64::consts::SQRT_2),
        ("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2),
    ] {
        vm.constant(math, name, Value::double(value));
    }
    for (name, length, func) in [
        ("abs", 1, abs as NativeFn),
        ("ceil", 1, ceil),
        ("floor", 1, floor),
        ("trunc", 1, trunc),
        ("round", 1, round),
        ("sign", 1, sign),
        ("sqrt", 1, sqrt),
        ("cbrt", 1, cbrt),
        ("sin", 1, sin),
        ("cos", 1, cos),
        ("tan", 1, tan),
        ("asin", 1, asin),
        ("acos", 1, acos),
        ("atan", 1, atan),
        ("sinh", 1, sinh),
        ("cosh", 1, cosh),
        ("tanh", 1, tanh),
        ("asinh", 1, asinh),
        ("acosh", 1, acosh),
        ("atanh", 1, atanh),
        ("exp", 1, exp),
        ("expm1", 1, expm1),
        ("log", 1, log),
        ("log2", 1, log2),
        ("log10", 1, log10),
        ("log1p", 1, log1p),
        ("fround", 1, fround),
        ("clz32", 1, clz32),
        ("pow", 2, pow),
        ("atan2", 2, atan2),
        ("imul", 2, imul),
        ("min", 2, min),
        ("max", 2, max),
        ("hypot", 2, hypot),
        ("random", 0, random),
    ] {
        vm.method(math, name, length, func);
    }
    let tag = vm.new_str("Math");
    vm.add_property(math, text::SYMBOL_TO_STRING_TAG, tag, CONFIGURABLE);

    let shape = vm.root_shape(vm.realm.object_proto);
    let number_proto = vm
        .alloc_object(shape, Kind::Boxed(Value::int(0)), 0)
        .handle();
    vm.realm.number_proto = number_proto;
    let number = vm.constructor("Number", 1, number_constructor, number_proto);
    for (name, length, func) in [
        ("toString", 1, number_to_string_method as NativeFn),
        ("valueOf", 0, value_of),
        ("toFixed", 1, to_fixed),
        ("toExponential", 1, to_exponential),
        ("toPrecision", 1, to_precision),
        ("toLocaleString", 0, to_locale_string),
    ] {
        vm.method(number_proto, name, length, func);
    }
    for (name, value) in [
        ("MAX_SAFE_INTEGER", 9_007_199_254_740_991.0),
        ("MIN_SAFE_INTEGER", -9_007_199_254_740_991.0),
        ("MAX_VALUE", f64::MAX),
        ("MIN_VALUE", 5e-324),
        ("EPSILON", f64::EPSILON),
        ("POSITIVE_INFINITY", f64::INFINITY),
        ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
        ("NaN", f64::NAN),
    ] {
        vm.constant(number, name, Value::double(value));
    }
    vm.method(number, "isNaN", 1, number_is_nan);
    vm.method(number, "isFinite", 1, number_is_finite);
    vm.method(number, "isInteger", 1, number_is_integer);
    vm.method(number, "isSafeInteger", 1, number_is_safe_integer);
    vm.method(number, "parseFloat", 1, parse_float);
    vm.method(number, "parseInt", 2, parse_int);

    let boolean_proto = vm
        .alloc_object(shape, Kind::Boxed(Value::FALSE), 0)
        .handle();
    vm.realm.boolean_proto = boolean_proto;
    vm.constructor("Boolean", 1, boolean_constructor, boolean_proto);
    vm.method(boolean_proto, "toString", 0, boolean_to_string);
    vm.method(boolean_proto, "valueOf", 0, boolean_value_of);

    vm.global_function("parseInt", 2, parse_int);
    vm.global_function("parseFloat", 1, parse_float);
    vm.global_function("isNaN", 1, is_nan_global);
    vm.global_function("isFinite", 1, is_finite_global);
    vm.global_function("encodeURIComponent", 1, encode_uri_component);
    vm.global_function("encodeURI", 1, encode_uri);
    vm.global_function("decodeURIComponent", 1, decode_uri_component);
    vm.global_function("decodeURI", 1, decode_uri);
    vm.global_function("escape", 1, escape);
    vm.global_function("unescape", 1, unescape);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_rounds_the_exact_value_and_ties_go_up() {
        assert_eq!(fixed(1.005, 2), "1.00");
        assert_eq!(fixed(0.125, 2), "0.13");
        assert_eq!(fixed(-0.001, 2), "-0.00");
        assert_eq!(fixed(1e20, 2), "100000000000000000000.00");
        assert_eq!(fixed(123.456, 0), "123");
        assert_eq!(fixed(5e-324, 3), "0.000");
        assert_eq!(fixed(0.000001, 7), "0.0000010");
        assert_eq!(fixed(2.5, 0), "3");
        assert_eq!(fixed(1.45, 1), "1.4");
        assert_eq!(fixed(0.5, 0), "1");
        assert_eq!(fixed(9.995, 2), "9.99");
        assert_eq!(fixed(99.99, 1), "100.0");
        assert_eq!(fixed(-1.5, 0), "-2");
        assert_eq!(fixed(0.0, 2), "0.00");
    }

    #[test]
    fn radix_text_is_right_for_whole_numbers_and_fractions() {
        assert_eq!(to_radix(255.0, 16), "ff");
        assert_eq!(to_radix(-255.0, 2), "-11111111");
        assert_eq!(to_radix(0.5, 2), "0.1");
        assert_eq!(to_radix(0.0, 16), "0");
    }
}
