//! `BigInt`: whole numbers of any size. Each is a sign and a list of 32-bit digits, lowest first, with no zero on
//! the end, so that equal numbers are always stored the same.

use crate::object::*;
use crate::text;
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct BigData {
    pub(crate) negative: bool,
    pub(crate) digits: Vec<u32>,
}

/// More bits than this is almost certainly a mistake, and would take all the memory there is.
const MAX_BITS: u64 = 1 << 26;

fn trim(digits: &mut Vec<u32>) {
    while digits.last() == Some(&0) {
        digits.pop();
    }
}

fn compare_magnitude(a: &[u32], b: &[u32]) -> std::cmp::Ordering {
    a.len()
        .cmp(&b.len())
        .then_with(|| a.iter().rev().cmp(b.iter().rev()))
}

fn add_magnitude(a: &[u32], b: &[u32]) -> Vec<u32> {
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut out = Vec::with_capacity(long.len() + 1);
    let mut carry = 0u64;
    for (index, digit) in long.iter().enumerate() {
        let sum = u64::from(*digit) + u64::from(short.get(index).copied().unwrap_or(0)) + carry;
        out.push(sum as u32);
        carry = sum >> 32;
    }
    if carry != 0 {
        out.push(carry as u32);
    }
    out
}

/// `a - b`, where `a` is the bigger.
fn sub_magnitude(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i64;
    for (index, digit) in a.iter().enumerate() {
        let difference = i64::from(*digit) - i64::from(b.get(index).copied().unwrap_or(0)) - borrow;
        if difference < 0 {
            out.push((difference + (1 << 32)) as u32);
            borrow = 1;
        } else {
            out.push(difference as u32);
            borrow = 0;
        }
    }
    trim(&mut out);
    out
}

fn mul_magnitude(a: &[u32], b: &[u32]) -> Vec<u32> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = vec![0u32; a.len() + b.len()];
    for (i, x) in a.iter().enumerate() {
        let mut carry = 0u64;
        for (j, y) in b.iter().enumerate() {
            let total = u64::from(*x) * u64::from(*y) + u64::from(out[i + j]) + carry;
            out[i + j] = total as u32;
            carry = total >> 32;
        }
        out[i + b.len()] = carry as u32;
    }
    trim(&mut out);
    out
}

fn shift_left_magnitude(a: &[u32], bits: u64) -> Vec<u32> {
    if a.is_empty() {
        return Vec::new();
    }
    let words = (bits / 32) as usize;
    let rest = (bits % 32) as u32;
    let mut out = vec![0u32; words];
    out.reserve(a.len() + 1);
    let mut carry = 0u32;
    for digit in a {
        if rest == 0 {
            out.push(*digit);
        } else {
            out.push(digit << rest | carry);
            carry = digit >> (32 - rest);
        }
    }
    if carry != 0 {
        out.push(carry);
    }
    out
}

fn shift_right_magnitude(a: &[u32], bits: u64) -> Vec<u32> {
    let words = (bits / 32) as usize;
    if words >= a.len() {
        return Vec::new();
    }
    let rest = (bits % 32) as u32;
    let mut out = Vec::with_capacity(a.len() - words);
    for index in words..a.len() {
        let low = a[index] >> rest;
        let high = if rest != 0 && index + 1 < a.len() {
            a[index + 1] << (32 - rest)
        } else {
            0
        };
        out.push(low | high);
    }
    trim(&mut out);
    out
}

/// Long division, a digit at a time. It gives the quotient and the remainder.
fn divide_magnitude(a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
    if compare_magnitude(a, b) == std::cmp::Ordering::Less {
        return (Vec::new(), a.to_vec());
    }
    if b.len() == 1 {
        let divisor = u64::from(b[0]);
        let mut quotient = vec![0u32; a.len()];
        let mut remainder = 0u64;
        for index in (0..a.len()).rev() {
            let current = remainder << 32 | u64::from(a[index]);
            quotient[index] = (current / divisor) as u32;
            remainder = current % divisor;
        }
        trim(&mut quotient);
        let mut rest = vec![remainder as u32];
        trim(&mut rest);
        return (quotient, rest);
    }
    // Knuth's algorithm: scale so the divisor's top digit is big, then guess each digit from the top two.
    let shift = u64::from(b[b.len() - 1].leading_zeros());
    let divisor = shift_left_magnitude(b, shift);
    let mut dividend = shift_left_magnitude(a, shift);
    dividend.resize(a.len() + 2, 0);
    let top = u64::from(divisor[divisor.len() - 1]);
    let next = u64::from(divisor[divisor.len() - 2]);
    let count = dividend.len() - divisor.len();
    let mut quotient = vec![0u32; count];
    for at in (0..count).rev() {
        let high = u64::from(dividend[at + divisor.len()]) << 32
            | u64::from(dividend[at + divisor.len() - 1]);
        let mut guess = high / top;
        let mut rest = high % top;
        while guess >> 32 != 0
            || guess * next > (rest << 32 | u64::from(dividend[at + divisor.len() - 2]))
        {
            guess -= 1;
            rest += top;
            if rest >> 32 != 0 {
                break;
            }
        }
        let mut borrow = 0i64;
        let mut carry = 0u64;
        for (index, digit) in divisor.iter().enumerate() {
            let product = guess * u64::from(*digit) + carry;
            carry = product >> 32;
            let difference = i64::from(dividend[at + index]) - borrow - i64::from(product as u32);
            dividend[at + index] = difference as u32;
            borrow = i64::from(difference < 0);
        }
        let difference = i64::from(dividend[at + divisor.len()]) - borrow - carry as i64;
        dividend[at + divisor.len()] = difference as u32;
        if difference < 0 {
            guess -= 1;
            let mut carry = 0u64;
            for (index, digit) in divisor.iter().enumerate() {
                let sum = u64::from(dividend[at + index]) + u64::from(*digit) + carry;
                dividend[at + index] = sum as u32;
                carry = sum >> 32;
            }
            dividend[at + divisor.len()] = (u64::from(dividend[at + divisor.len()]) + carry) as u32;
        }
        quotient[at] = guess as u32;
    }
    trim(&mut quotient);
    trim(&mut dividend);
    let remainder = shift_right_magnitude(&dividend, shift);
    (quotient, remainder)
}

impl BigData {
    fn new(negative: bool, mut digits: Vec<u32>) -> Self {
        trim(&mut digits);
        Self {
            negative: negative && !digits.is_empty(),
            digits,
        }
    }

    pub(crate) fn is_zero(&self) -> bool {
        self.digits.is_empty()
    }

    fn from_i64(value: i64) -> Self {
        let magnitude = value.unsigned_abs();
        Self::new(value < 0, vec![magnitude as u32, (magnitude >> 32) as u32])
    }

    pub(crate) fn from_u64(value: u64) -> Self {
        Self::new(false, vec![value as u32, (value >> 32) as u32])
    }

    /// The bottom 64 bits, as two's complement.
    pub(crate) fn low_u64(&self) -> u64 {
        let digits = self.twos_complement(self.digits.len().max(2) + 1);
        u64::from(digits[0]) | u64::from(digits[1]) << 32
    }

    pub(crate) fn from_f64(value: f64) -> Self {
        let negative = value < 0.0;
        let magnitude = value.abs();
        if magnitude < 1.0 {
            return Self::default();
        }
        let bits = magnitude.to_bits();
        let exponent = ((bits >> 52) & 0x7FF) as i64 - 1075;
        let mantissa = (bits & ((1 << 52) - 1)) | (1 << 52);
        let base = vec![mantissa as u32, (mantissa >> 32) as u32];
        let digits = if exponent >= 0 {
            shift_left_magnitude(&base, exponent as u64)
        } else {
            shift_right_magnitude(&base, (-exponent) as u64)
        };
        Self::new(negative, digits)
    }

    /// The nearest double, rounding halves to even like everything else does.
    pub(crate) fn to_f64(&self) -> f64 {
        let bits = self.bit_length();
        let value = if bits <= 64 {
            let mut low = 0u64;
            for (index, digit) in self.digits.iter().enumerate() {
                low |= u64::from(*digit) << (index * 32);
            }
            low as f64
        } else if bits > 1024 {
            f64::INFINITY
        } else {
            // The top 64 bits, with a sticky bit for whatever's below, round the same as all of them.
            let shift = bits - 64;
            let top = shift_right_magnitude(&self.digits, shift);
            let mut high = u64::from(top[0]) | u64::from(*top.get(1).unwrap_or(&0)) << 32;
            let below = shift_left_magnitude(&top, shift);
            if below != self.digits {
                high |= 1;
            }
            high as f64 * 2f64.powi(shift as i32)
        };
        if self.negative { -value } else { value }
    }

    fn bit_length(&self) -> u64 {
        match self.digits.last() {
            Some(top) => self.digits.len() as u64 * 32 - u64::from(top.leading_zeros()),
            None => 0,
        }
    }

    pub(crate) fn to_radix(&self, radix: u32) -> String {
        if self.is_zero() {
            return "0".to_owned();
        }
        let mut out = Vec::new();
        let mut current = self.digits.clone();
        // A chunk at a time, as dividing a long number by anything takes a while.
        let mut chunk_digits = 1;
        let mut chunk = u64::from(radix);
        while chunk * u64::from(radix) <= u64::from(u32::MAX) {
            chunk *= u64::from(radix);
            chunk_digits += 1;
        }
        while !current.is_empty() {
            let mut remainder = 0u64;
            for digit in current.iter_mut().rev() {
                let value = remainder << 32 | u64::from(*digit);
                *digit = (value / chunk) as u32;
                remainder = value % chunk;
            }
            trim(&mut current);
            for _ in 0..chunk_digits {
                out.push(
                    char::from_digit((remainder % u64::from(radix)) as u32, radix).unwrap_or('0')
                        as u8,
                );
                remainder /= u64::from(radix);
                if remainder == 0 && current.is_empty() {
                    break;
                }
            }
        }
        if self.negative {
            out.push(b'-');
        }
        out.reverse();
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Reads digits in a base, with nothing else around them.
    pub(crate) fn parse_digits(text: &str, radix: u32) -> Option<Self> {
        if text.is_empty() {
            return None;
        }
        let mut digits: Vec<u32> = Vec::new();
        for character in text.chars() {
            let digit = character.to_digit(radix)?;
            let mut carry = u64::from(digit);
            for existing in &mut digits {
                let value = u64::from(*existing) * u64::from(radix) + carry;
                *existing = value as u32;
                carry = value >> 32;
            }
            if carry != 0 {
                digits.push(carry as u32);
            }
        }
        Some(Self::new(false, digits))
    }

    /// Reads a string as `BigInt("...")` does.
    fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return Some(Self::default());
        }
        if let Some(rest) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            return Self::parse_digits(rest, 16);
        }
        if let Some(rest) = text.strip_prefix("0o").or_else(|| text.strip_prefix("0O")) {
            return Self::parse_digits(rest, 8);
        }
        if let Some(rest) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
            return Self::parse_digits(rest, 2);
        }
        let (negative, rest) = match text.as_bytes()[0] {
            b'-' => (true, &text[1..]),
            b'+' => (false, &text[1..]),
            _ => (false, text),
        };
        let parsed = Self::parse_digits(rest, 10)?;
        Some(Self::new(negative, parsed.digits))
    }

    fn add(&self, other: &Self) -> Self {
        if self.negative == other.negative {
            return Self::new(self.negative, add_magnitude(&self.digits, &other.digits));
        }
        match compare_magnitude(&self.digits, &other.digits) {
            std::cmp::Ordering::Less => {
                Self::new(other.negative, sub_magnitude(&other.digits, &self.digits))
            }
            _ => Self::new(self.negative, sub_magnitude(&self.digits, &other.digits)),
        }
    }

    fn negated(&self) -> Self {
        Self::new(!self.negative, self.digits.clone())
    }

    fn sub(&self, other: &Self) -> Self {
        self.add(&other.negated())
    }

    fn mul(&self, other: &Self) -> Self {
        Self::new(
            self.negative != other.negative,
            mul_magnitude(&self.digits, &other.digits),
        )
    }

    fn div_rem(&self, other: &Self) -> (Self, Self) {
        let (quotient, remainder) = divide_magnitude(&self.digits, &other.digits);
        (
            Self::new(self.negative != other.negative, quotient),
            Self::new(self.negative, remainder),
        )
    }

    fn shifted_left(&self, bits: u64) -> Self {
        Self::new(self.negative, shift_left_magnitude(&self.digits, bits))
    }

    /// A shift to the right, which rounds towards negative infinity.
    fn shifted_right(&self, bits: u64) -> Self {
        let shifted = shift_right_magnitude(&self.digits, bits);
        if self.negative && shift_left_magnitude(&shifted, bits) != self.digits {
            return Self::new(true, add_magnitude(&shifted, &[1]));
        }
        Self::new(self.negative, shifted)
    }

    /// The digits of two's complement, with enough of them to hold this and its sign.
    fn twos_complement(&self, words: usize) -> Vec<u32> {
        let mut out = self.digits.clone();
        out.resize(words, 0);
        if self.negative {
            let mut carry = 1u64;
            for digit in &mut out {
                let value = u64::from(!*digit) + carry;
                *digit = value as u32;
                carry = value >> 32;
            }
        }
        out
    }

    fn from_twos_complement(mut digits: Vec<u32>) -> Self {
        let negative = digits.last().is_some_and(|top| top >> 31 != 0);
        if negative {
            let mut carry = 1u64;
            for digit in &mut digits {
                let value = u64::from(!*digit) + carry;
                *digit = value as u32;
                carry = value >> 32;
            }
        }
        Self::new(negative, digits)
    }

    fn bitwise(&self, other: &Self, op: fn(u32, u32) -> u32) -> Self {
        let words = self.digits.len().max(other.digits.len()) + 1;
        let (a, b) = (self.twos_complement(words), other.twos_complement(words));
        Self::from_twos_complement(a.iter().zip(&b).map(|(x, y)| op(*x, *y)).collect())
    }

    /// The bottom `bits` bits, as a number that's never negative.
    fn low_bits(&self, bits: u64) -> Self {
        if bits == 0 {
            return Self::default();
        }
        let words = (bits.div_ceil(32) as usize).max(self.digits.len()) + 1;
        let mut out = self.twos_complement(words);
        out.truncate(bits.div_ceil(32) as usize);
        let rest = (bits % 32) as u32;
        if rest != 0 {
            if let Some(top) = out.last_mut() {
                *top &= (1u32 << rest) - 1;
            }
        }
        Self::new(false, out)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BigOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    UShr,
}

impl Vm {
    pub(crate) fn new_bigint(&mut self, data: BigData) -> Value {
        self.debt += data.digits.capacity() * 4;
        let shape = self.realm.object_shape;
        let object = self.alloc_object(shape, Kind::BigInt(Box::new(data)), 0);
        let value = Value::bigint(object.handle());
        self.nursery.push(value);
        value
    }

    pub(crate) fn big_of(&self, value: Value) -> &BigData {
        match &self.objects[value.index()].kind {
            Kind::BigInt(data) => data,
            _ => unreachable!("a value tagged as a BigInt is one"),
        }
    }

    pub(crate) fn bigint_from_i64(&mut self, value: i64) -> Value {
        self.new_bigint(BigData::from_i64(value))
    }

    /// Does an operator on two values, if either is a BigInt. They both have to be.
    pub(crate) fn bigint_op(&mut self, op: BigOp, a: Value, b: Value) -> Run<Value> {
        if !a.is_bigint() || !b.is_bigint() {
            return Err(
                self.type_error("Cannot mix BigInt and other types, use explicit conversions")
            );
        }
        let (x, y) = (self.big_of(a).clone(), self.big_of(b).clone());
        let result = match op {
            BigOp::Add => x.add(&y),
            BigOp::Sub => x.sub(&y),
            BigOp::Mul => {
                if x.bit_length() + y.bit_length() > MAX_BITS {
                    return Err(self.range_error("Maximum BigInt size exceeded"));
                }
                x.mul(&y)
            }
            BigOp::Div | BigOp::Mod => {
                if y.is_zero() {
                    return Err(self.range_error("Division by zero"));
                }
                let (quotient, remainder) = x.div_rem(&y);
                if op == BigOp::Div {
                    quotient
                } else {
                    remainder
                }
            }
            BigOp::Pow => {
                if y.negative {
                    return Err(self.range_error("Exponent must be non-negative"));
                }
                let one = BigData::from_i64(1);
                if x == one || x.is_zero() && !y.is_zero() {
                    x
                } else if y.is_zero() {
                    one
                } else if x == one.negated() {
                    if y.digits[0] & 1 == 0 { one } else { x }
                } else {
                    if y.digits.len() > 1 || u64::from(y.digits[0]) * x.bit_length() > MAX_BITS {
                        return Err(self.range_error("Maximum BigInt size exceeded"));
                    }
                    let mut result = one;
                    let mut base = x;
                    let mut exponent = y.digits[0];
                    while exponent > 0 {
                        if exponent & 1 == 1 {
                            result = result.mul(&base);
                        }
                        exponent >>= 1;
                        if exponent > 0 {
                            base = base.mul(&base);
                        }
                    }
                    result
                }
            }
            BigOp::And => x.bitwise(&y, |p, q| p & q),
            BigOp::Or => x.bitwise(&y, |p, q| p | q),
            BigOp::Xor => x.bitwise(&y, |p, q| p ^ q),
            BigOp::Shl | BigOp::Shr => {
                let left = (op == BigOp::Shl) != y.negative;
                if y.digits.len() > 1 {
                    if left && !x.is_zero() {
                        return Err(self.range_error("Maximum BigInt size exceeded"));
                    }
                    if x.negative {
                        BigData::from_i64(-1)
                    } else {
                        BigData::default()
                    }
                } else {
                    let bits = u64::from(y.digits.first().copied().unwrap_or(0));
                    if left {
                        if x.bit_length() + bits > MAX_BITS {
                            return Err(self.range_error("Maximum BigInt size exceeded"));
                        }
                        x.shifted_left(bits)
                    } else {
                        x.shifted_right(bits)
                    }
                }
            }
            BigOp::UShr => {
                return Err(self.type_error("BigInts have no unsigned right shift, use >> instead"));
            }
        };
        Ok(self.new_bigint(result))
    }

    pub(crate) fn bigint_negate(&mut self, value: Value) -> Value {
        let result = self.big_of(value).negated();
        self.new_bigint(result)
    }

    pub(crate) fn bigint_not(&mut self, value: Value) -> Value {
        let result = self.big_of(value).negated().sub(&BigData::from_i64(1));
        self.new_bigint(result)
    }

    pub(crate) fn bigint_step(&mut self, value: Value, by: i64) -> Value {
        let result = self.big_of(value).add(&BigData::from_i64(by));
        self.new_bigint(result)
    }

    /// Whether a BigInt is the same as a number, exactly.
    pub(crate) fn bigint_equals_number(&self, big: Value, number: f64) -> bool {
        number.is_finite()
            && number.fract() == 0.0
            && *self.big_of(big) == BigData::from_f64(number)
    }

    /// Whether `a < b`, for a BigInt and a number in either order, or `None` if the number is NaN.
    pub(crate) fn bigint_less_than_number(
        &self,
        big: Value,
        number: f64,
        big_first: bool,
    ) -> Option<bool> {
        if number.is_nan() {
            return None;
        }
        if number.is_infinite() {
            return Some((number > 0.0) == big_first);
        }
        let floor = BigData::from_f64(number.floor());
        let data = self.big_of(big);
        let order = compare_big(data, &floor);
        use std::cmp::Ordering::{Equal, Greater, Less};
        Some(match (order, number.fract() != 0.0, big_first) {
            (Less, _, true) => true,
            (Equal, fractional, true) => fractional,
            (Greater, _, true) => false,
            (Less | Equal, _, false) => false,
            (Greater, _, false) => true,
        })
    }

    pub(crate) fn bigint_less_than(&self, a: Value, b: Value) -> bool {
        compare_big(self.big_of(a), self.big_of(b)) == std::cmp::Ordering::Less
    }

    /// Reads a string as a BigInt, for comparing with one. `None` means it isn't one.
    pub(crate) fn parse_bigint(&mut self, text: &str) -> Option<Value> {
        let data = BigData::parse(text)?;
        Some(self.new_bigint(data))
    }
}

fn compare_big(a: &BigData, b: &BigData) -> std::cmp::Ordering {
    use std::cmp::Ordering::{Greater, Less};
    match (a.negative, b.negative) {
        (false, true) => Greater,
        (true, false) => Less,
        (false, false) => compare_magnitude(&a.digits, &b.digits),
        (true, true) => compare_magnitude(&b.digits, &a.digits),
    }
}

pub(crate) fn to_bigint(vm: &mut Vm, value: Value) -> Run<Value> {
    let primitive = vm.to_primitive(value, crate::convert::Hint::Number)?;
    if primitive.is_bigint() {
        return Ok(primitive);
    }
    if primitive.is_bool() {
        return Ok(vm.bigint_from_i64(i64::from(primitive == Value::TRUE)));
    }
    if primitive.is_string() {
        let text = vm.text_of(primitive).to_rust();
        return match vm.parse_bigint(&text) {
            Some(big) => Ok(big),
            None => Err(vm.syntax_error(&format!("Cannot convert {text} to a BigInt"))),
        };
    }
    let shown = vm.brief(primitive);
    Err(vm.type_error(&format!("Cannot convert {shown} to a BigInt")))
}

fn bigint_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    if args.new_target.is_object() {
        return Err(vm.type_error("BigInt is not a constructor"));
    }
    let value = vm.arg(&args, 0);
    let primitive = vm.to_primitive(value, crate::convert::Hint::Number)?;
    if primitive.is_number() {
        let number = primitive.as_number();
        if !number.is_finite() || number.fract() != 0.0 {
            let shown = crate::convert::number_to_string(number);
            return Err(vm.range_error(&format!(
                "The number {shown} cannot be converted to a BigInt because it is not an integer"
            )));
        }
        return Ok(vm.new_bigint(BigData::from_f64(number)));
    }
    to_bigint(vm, primitive)
}

fn this_bigint(vm: &mut Vm, this: Value) -> Run<Value> {
    if this.is_bigint() {
        return Ok(this);
    }
    if this.is_object() {
        if let Kind::Boxed(inner) = vm.objects[this.index()].kind {
            if inner.is_bigint() {
                return Ok(inner);
            }
        }
    }
    Err(vm.type_error("This isn't a BigInt"))
}

fn to_string(vm: &mut Vm, args: Args) -> Run<Value> {
    let big = this_bigint(vm, args.this)?;
    let radix = vm.arg(&args, 0);
    let radix = if radix.is_undefined() {
        10.0
    } else {
        vm.to_integer(radix)?
    };
    if !(2.0..=36.0).contains(&radix) {
        return Err(vm.range_error("toString() radix must be between 2 and 36"));
    }
    let text = vm.big_of(big).to_radix(radix as u32);
    Ok(vm.new_str(&text))
}

fn value_of(vm: &mut Vm, args: Args) -> Run<Value> {
    this_bigint(vm, args.this)
}

fn as_uint_n(vm: &mut Vm, args: Args) -> Run<Value> {
    let bits = vm.arg(&args, 0);
    let bits = vm.to_integer(bits)?;
    if !(0.0..=9_007_199_254_740_991.0).contains(&bits) {
        return Err(vm.range_error("Invalid value: not (convertible to) a safe integer"));
    }
    let value = vm.arg(&args, 1);
    let big = to_bigint(vm, value)?;
    let result = vm.big_of(big).low_bits(bits as u64);
    Ok(vm.new_bigint(result))
}

fn as_int_n(vm: &mut Vm, args: Args) -> Run<Value> {
    let bits = vm.arg(&args, 0);
    let bits = vm.to_integer(bits)?;
    if !(0.0..=9_007_199_254_740_991.0).contains(&bits) {
        return Err(vm.range_error("Invalid value: not (convertible to) a safe integer"));
    }
    let value = vm.arg(&args, 1);
    let big = to_bigint(vm, value)?;
    let bits = bits as u64;
    let low = vm.big_of(big).low_bits(bits);
    let result = if bits > 0 && low.bit_length() == bits {
        low.sub(&BigData::from_i64(1).shifted_left(bits))
    } else {
        low
    };
    Ok(vm.new_bigint(result))
}

pub(super) fn install(vm: &mut Vm) {
    let proto = vm.plain_proto();
    vm.realm.bigint_proto = proto;
    let constructor = vm.constructor("BigInt", 1, bigint_constructor, proto);
    vm.method(constructor, "asUintN", 2, as_uint_n);
    vm.method(constructor, "asIntN", 2, as_int_n);
    vm.method(proto, "toString", 0, to_string);
    vm.method(proto, "toLocaleString", 0, to_string);
    vm.method(proto, "valueOf", 0, value_of);
    let tag = vm.new_str("BigInt");
    vm.add_property(proto, text::SYMBOL_TO_STRING_TAG, tag, CONFIGURABLE);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big(text: &str) -> BigData {
        BigData::parse(text).unwrap()
    }

    #[test]
    fn numbers_read_and_write_in_any_base() {
        for text in [
            "0",
            "1",
            "-1",
            "4294967296",
            "123456789012345678901234567890",
            "-99999999999999999999",
        ] {
            assert_eq!(big(text).to_radix(10), text);
        }
        assert_eq!(big("255").to_radix(16), "ff");
        assert_eq!(big("0xff").to_radix(10), "255");
        assert_eq!(big("0b101").to_radix(2), "101");
        assert_eq!(big("-8").to_radix(2), "-1000");
        assert_eq!(BigData::parse("12x"), None);
        assert_eq!(big("  7 ").to_radix(10), "7");
        assert!(big("-0").is_zero() && !big("-0").negative);
    }

    #[test]
    fn arithmetic_is_right_at_any_size() {
        let a = big("123456789012345678901234567890");
        let b = big("987654321098765432109876543210");
        assert_eq!(a.add(&b).to_radix(10), "1111111110111111111011111111100");
        assert_eq!(a.sub(&b).to_radix(10), "-864197532086419753208641975320");
        assert_eq!(
            a.mul(&b).to_radix(10),
            "121932631137021795226185032733622923332237463801111263526900"
        );
        assert_eq!(big("-7").div_rem(&big("2")).0.to_radix(10), "-3");
        assert_eq!(big("-7").div_rem(&big("2")).1.to_radix(10), "-1");
        assert_eq!(b.div_rem(&a).0.to_radix(10), "8");
        assert_eq!(b.div_rem(&a).1.to_radix(10), "9000000000900000000090");
    }

    #[test]
    fn long_division_agrees_with_multiplying_back() {
        // Cases near where the guessed digit needs correcting.
        let cases = [
            (
                "340282366920938463463374607431768211455",
                "18446744073709551616",
            ),
            (
                "340282366920938463463374607431768211456",
                "18446744073709551615",
            ),
            ("79228162514264337593543950335", "4294967296"),
            (
                "1000000000000000000000000000000000000000",
                "3000000000000000000001",
            ),
            (
                "6277101735386680763835789423207666416083908700390324961279",
                "340282366920938463463374607431768211455",
            ),
        ];
        for (x, y) in cases {
            let (x, y) = (big(x), big(y));
            let (quotient, remainder) = x.div_rem(&y);
            assert_eq!(quotient.mul(&y).add(&remainder), x);
            assert_eq!(
                compare_magnitude(&remainder.digits, &y.digits),
                std::cmp::Ordering::Less
            );
        }
    }

    #[test]
    fn shifts_round_towards_negative_infinity() {
        assert_eq!(big("5").shifted_right(1).to_radix(10), "2");
        assert_eq!(big("-5").shifted_right(1).to_radix(10), "-3");
        assert_eq!(big("-4").shifted_right(1).to_radix(10), "-2");
        assert_eq!(
            big("1").shifted_left(100).to_radix(16),
            format!("1{}", "0".repeat(25))
        );
    }

    #[test]
    fn bit_operations_treat_negatives_as_twos_complement() {
        let check = |a: &str, b: &str, op: fn(u32, u32) -> u32, expected: &str| {
            assert_eq!(big(a).bitwise(&big(b), op).to_radix(10), expected);
        };
        check("-1", "255", |p, q| p & q, "255");
        check("-256", "255", |p, q| p | q, "-1");
        check("-1", "-1", |p, q| p ^ q, "0");
        check("12", "10", |p, q| p & q, "8");
        check("4294967296", "-4294967296", |p, q| p & q, "4294967296");
    }

    #[test]
    fn doubles_go_there_and_back() {
        assert_eq!(
            BigData::from_f64(1e21).to_radix(10),
            "1000000000000000000000"
        );
        assert_eq!(BigData::from_f64(-5.0).to_radix(10), "-5");
        assert_eq!(big("1000000000000000000000").to_f64(), 1e21);
        assert_eq!(big("-5").to_f64(), -5.0);
        assert_eq!(big("9007199254740993").to_f64(), 9007199254740992.0);
        assert_eq!(
            BigData::from_i64(1).shifted_left(2000).to_f64(),
            f64::INFINITY
        );
        assert_eq!(
            BigData::from_i64(1).shifted_left(1000).to_f64(),
            2f64.powi(1000)
        );
    }
}
