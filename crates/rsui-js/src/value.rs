//! A JavaScript value in 8 bytes. A double is itself, and everything else hides in the space of NaNs.

const TAG_SHIFT: u32 = 48;
const INT: u64 = 0xFFF9 << TAG_SHIFT;
const OBJECT: u64 = 0xFFFA << TAG_SHIFT;
const STRING: u64 = 0xFFFB << TAG_SHIFT;
const SPECIAL: u64 = 0xFFFC << TAG_SHIFT;
const SYMBOL: u64 = 0xFFFD << TAG_SHIFT;
const CELL: u64 = 0xFFFE << TAG_SHIFT;
const BIGINT: u64 = 0xFFFF << TAG_SHIFT;
const LOW: u64 = 0xFFFF_FFFF;
const CANONICAL_NAN: u64 = 0x7FF8 << TAG_SHIFT;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Value(pub(crate) u64);

impl Value {
    pub const UNDEFINED: Self = Self(SPECIAL);
    pub const NULL: Self = Self(SPECIAL | 1);
    pub const FALSE: Self = Self(SPECIAL | 2);
    pub const TRUE: Self = Self(SPECIAL | 3);
    /// A hole in an array, or a variable that hasn't started yet. Programs never see it.
    pub(crate) const EMPTY: Self = Self(SPECIAL | 4);
    /// What's thrown through a generator to run its `finally` blocks when it's told to return.
    pub(crate) const GENERATOR_RETURN: Self = Self(SPECIAL | 5);
    /// What a function the interpreter calls fails with when it has made or ended a frame, so the interpreter goes on
    /// with the frame that's on top (`Vm::call_indirect`, `Vm::step_in_place`). It never leaves the interpreter, and
    /// programs never see it.
    pub(crate) const ENTERED: Self = Self(SPECIAL | 6);

    #[inline(always)]
    pub const fn int(value: i32) -> Self {
        Self(INT | value as u32 as u64)
    }

    /// A number exactly as it is, with no looking at whether it could be an int.
    #[inline(always)]
    pub fn double(value: f64) -> Self {
        if value.is_nan() {
            return Self(CANONICAL_NAN);
        }
        Self(value.to_bits())
    }

    /// The result of arithmetic on numbers. A NaN that arithmetic makes is one of the two kinds that are fine as they
    /// are, and any it's given it just passes on, so there's no need to look for one.
    #[inline(always)]
    pub(crate) fn computed(value: f64) -> Self {
        Self(value.to_bits())
    }

    /// A length or a position, which is nearly always small enough to be an int.
    #[inline(always)]
    pub fn int_or_double(value: usize) -> Self {
        if value <= i32::MAX as usize {
            Self::int(value as i32)
        } else {
            Self(f64::to_bits(value as f64))
        }
    }

    /// Whether both of these are doubles (numbers that aren't ints), with only one thing to test.
    #[inline(always)]
    pub(crate) fn both_doubles(a: Self, b: Self) -> bool {
        a.0.max(b.0) < INT
    }

    /// Whether both of these are ints, with only one thing to test.
    #[inline(always)]
    pub(crate) fn both_ints(a: Self, b: Self) -> bool {
        ((a.0 ^ INT) | (b.0 ^ INT)) >> 32 == 0
    }

    /// A number, as an int if that's what it is.
    #[inline]
    pub fn number(value: f64) -> Self {
        let int = value as i32;
        if f64::from(int) == value && (int != 0 || value.is_sign_positive()) {
            return Self::int(int);
        }
        Self::double(value)
    }

    #[inline(always)]
    pub const fn bool(value: bool) -> Self {
        if value { Self::TRUE } else { Self::FALSE }
    }

    #[inline(always)]
    pub(crate) const fn object(handle: u32) -> Self {
        Self(OBJECT | handle as u64)
    }

    #[inline(always)]
    pub(crate) const fn string(handle: u32) -> Self {
        Self(STRING | handle as u64)
    }

    #[inline(always)]
    pub(crate) const fn symbol(atom: u32) -> Self {
        Self(SYMBOL | atom as u64)
    }

    #[inline(always)]
    pub(crate) const fn bigint(handle: u32) -> Self {
        Self(BIGINT | handle as u64)
    }

    #[inline(always)]
    pub(crate) const fn cell(handle: u32) -> Self {
        Self(CELL | handle as u64)
    }

    #[inline(always)]
    pub fn is_int(self) -> bool {
        self.0 >> TAG_SHIFT == INT >> TAG_SHIFT
    }

    #[inline(always)]
    pub fn is_double(self) -> bool {
        self.0 < INT
    }

    #[inline(always)]
    pub fn is_number(self) -> bool {
        self.0 < OBJECT
    }

    #[inline(always)]
    pub fn is_object(self) -> bool {
        self.0 >> TAG_SHIFT == OBJECT >> TAG_SHIFT
    }

    #[inline(always)]
    pub fn is_string(self) -> bool {
        self.0 >> TAG_SHIFT == STRING >> TAG_SHIFT
    }

    #[inline(always)]
    pub fn is_symbol(self) -> bool {
        self.0 >> TAG_SHIFT == SYMBOL >> TAG_SHIFT
    }

    #[inline(always)]
    pub fn is_bigint(self) -> bool {
        self.0 >> TAG_SHIFT == BIGINT >> TAG_SHIFT
    }

    #[inline(always)]
    #[allow(dead_code)]
    pub(crate) fn is_cell(self) -> bool {
        self.0 >> TAG_SHIFT == CELL >> TAG_SHIFT
    }

    /// Whether this can be `===` to a value with other bits: a number (an int and the same double, or 0 and -0), a
    /// string (another with the same text) or a bigint. Anything else is only ever `===` to itself.
    #[inline(always)]
    pub(crate) fn has_other_forms(self) -> bool {
        self.is_number() || self.is_string() || self.is_bigint()
    }
    #[inline(always)]
    pub fn is_undefined(self) -> bool {
        self == Self::UNDEFINED
    }

    #[inline(always)]
    pub fn is_null(self) -> bool {
        self == Self::NULL
    }

    #[inline(always)]
    pub fn is_nullish(self) -> bool {
        self.0 | 1 == Self::NULL.0
    }

    #[inline(always)]
    pub fn is_bool(self) -> bool {
        self.0 | 1 == Self::TRUE.0
    }

    #[inline(always)]
    pub(crate) fn is_empty(self) -> bool {
        self == Self::EMPTY
    }

    /// The bits of this value, which are the same for as long as it's the same thing.
    #[inline(always)]
    pub const fn bits(self) -> u64 {
        self.0
    }

    #[inline(always)]
    pub const fn as_int(self) -> i32 {
        self.0 as u32 as i32
    }

    #[inline(always)]
    pub(crate) fn as_double(self) -> f64 {
        f64::from_bits(self.0)
    }

    /// The number this is, which it has to be.
    #[inline(always)]
    pub fn as_number(self) -> f64 {
        if self.is_int() {
            f64::from(self.as_int())
        } else {
            self.as_double()
        }
    }

    /// The top bits that say what kind of value this is, other than a double. Anything below [`Value::OBJECT_TAG`] is a
    /// number. Only objects, strings, cells and bigints (the two tags from [`Value::CELL_TAG`] up) are things the
    /// collector has to look at.
    #[inline(always)]
    pub(crate) const fn tag(self) -> u64 {
        self.0 >> TAG_SHIFT
    }

    pub(crate) const OBJECT_TAG: u64 = OBJECT >> TAG_SHIFT;
    pub(crate) const STRING_TAG: u64 = STRING >> TAG_SHIFT;
    /// Symbols, cells and bigints have the three tags from this one up.
    pub(crate) const SYMBOL_TAG: u64 = SYMBOL >> TAG_SHIFT;
    pub(crate) const CELL_TAG: u64 = CELL >> TAG_SHIFT;

    /// The number of the object, string, symbol or cell this is.
    #[inline(always)]
    pub(crate) const fn handle(self) -> u32 {
        (self.0 & LOW) as u32
    }

    #[inline(always)]
    pub(crate) fn index(self) -> usize {
        self.handle() as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_eight_bytes() {
        assert_eq!(size_of::<Value>(), 8);
    }

    #[test]
    fn numbers_keep_what_they_are() {
        assert!(Value::int(-5).is_int());
        assert_eq!(Value::int(-5).as_int(), -5);
        assert_eq!(Value::int(-5).as_number(), -5.0);
        assert!(Value::double(1.5).is_double());
        assert!(Value::double(f64::NEG_INFINITY).is_double());
        assert_eq!(
            Value::double(f64::NEG_INFINITY).as_number(),
            f64::NEG_INFINITY
        );
        assert!(Value::int(1).is_number() && Value::double(1.5).is_number());
        assert!(!Value::UNDEFINED.is_number() && !Value::object(1).is_number());
    }

    #[test]
    fn a_nan_of_any_sign_is_still_a_number() {
        let negative = f64::from_bits(0xFFFF_FFFF_FFFF_FFFF);
        let value = Value::double(negative);
        assert!(value.is_double());
        assert!(value.as_number().is_nan());
    }

    #[test]
    fn only_whole_numbers_become_ints() {
        assert!(Value::number(3.0).is_int());
        assert!(!Value::number(3.5).is_int());
        assert!(!Value::number(-0.0).is_int());
        assert!(!Value::number(3e10).is_int());
        assert!(!Value::number(f64::NAN).is_int());
    }

    #[test]
    fn tags_are_told_apart() {
        assert!(Value::object(7).is_object() && Value::object(7).handle() == 7);
        assert!(Value::string(7).is_string() && !Value::string(7).is_object());
        assert!(Value::symbol(7).is_symbol());
        assert!(Value::UNDEFINED.is_nullish() && Value::NULL.is_nullish());
        assert!(!Value::FALSE.is_nullish() && !Value::EMPTY.is_nullish());
        assert!(Value::TRUE.is_bool() && Value::FALSE.is_bool() && !Value::NULL.is_bool());
    }
}
