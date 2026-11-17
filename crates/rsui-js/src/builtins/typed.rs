//! `ArrayBuffer`, the typed arrays and `DataView`.

use crate::object::*;
use crate::text;
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

use super::bigint::BigData;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Element {
    Int8,
    Uint8,
    Uint8Clamped,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Float16,
    Float32,
    Float64,
    BigInt64,
    BigUint64,
}

const ELEMENTS: [(&str, Element); 12] = [
    ("Int8Array", Element::Int8),
    ("Uint8Array", Element::Uint8),
    ("Uint8ClampedArray", Element::Uint8Clamped),
    ("Int16Array", Element::Int16),
    ("Uint16Array", Element::Uint16),
    ("Int32Array", Element::Int32),
    ("Uint32Array", Element::Uint32),
    ("Float16Array", Element::Float16),
    ("Float32Array", Element::Float32),
    ("Float64Array", Element::Float64),
    ("BigInt64Array", Element::BigInt64),
    ("BigUint64Array", Element::BigUint64),
];

/// The nearest 16-bit float to a number, as its bits, with halves going to the even one.
pub(crate) fn f64_to_f16_bits(number: f64) -> u16 {
    let sign = if number.is_sign_negative() {
        0x8000u16
    } else {
        0
    };
    if number.is_nan() {
        return 0x7E00;
    }
    let magnitude = number.abs();
    if magnitude == 0.0 {
        return sign;
    }
    if magnitude.is_infinite() || magnitude >= 65520.0 {
        return sign | 0x7C00;
    }
    // Everything a 16-bit float can be is a whole number of this smallest step, or of a power of two times it.
    let exponent = magnitude.log2().floor() as i32;
    let exponent = if magnitude < 2f64.powi(exponent) {
        exponent - 1
    } else {
        exponent
    };
    let (step, offset) = if exponent < -14 {
        (2f64.powi(-24), 0u16)
    } else {
        (2f64.powi(exponent - 10), ((exponent + 15) as u16) << 10)
    };
    let scaled = magnitude / step;
    let floor = scaled.floor();
    let rounded = match scaled - floor {
        difference if difference < 0.5 => floor,
        difference if difference > 0.5 => floor + 1.0,
        _ if floor % 2.0 == 0.0 => floor,
        _ => floor + 1.0,
    } as u16;
    if exponent < -14 {
        // Below the normal range, the bits are just the count. It can roll up into the normal range, correctly.
        return sign | rounded;
    }
    // The count includes the top bit that isn't stored. If it rolled over, that carries into the exponent, correctly.
    sign | (offset + (rounded - 1024))
}

/// The bytes of element `index`, for elements of `N` bytes.
#[inline(always)]
fn read_at<const N: usize>(bytes: &[u8], index: usize) -> [u8; N] {
    let at = index * N;
    bytes[at..at + N].try_into().expect("N bytes")
}

/// Element `index` of these bytes, for any kind but the BigInt ones.
#[inline(always)]
fn read_plain(element: Element, bytes: &[u8], index: usize) -> Option<Value> {
    Some(match element {
        Element::Int8 => Value::int(i32::from(bytes[index] as i8)),
        Element::Uint8 | Element::Uint8Clamped => Value::int(i32::from(bytes[index])),
        Element::Int16 => Value::int(i32::from(i16::from_le_bytes(read_at(bytes, index)))),
        Element::Uint16 => Value::int(i32::from(u16::from_le_bytes(read_at(bytes, index)))),
        Element::Int32 => Value::int(i32::from_le_bytes(read_at(bytes, index))),
        Element::Uint32 => Value::int_or_double(u32::from_le_bytes(read_at(bytes, index)) as usize),
        Element::Float32 => Value::number(f64::from(f32::from_le_bytes(read_at(bytes, index)))),
        Element::Float64 => Value::number(f64::from_le_bytes(read_at(bytes, index))),
        Element::Float16 => {
            Value::number(f16_bits_to_f64(u16::from_le_bytes(read_at(bytes, index))))
        }
        Element::BigInt64 | Element::BigUint64 => return None,
    })
}

fn f16_bits_to_f64(bits: u16) -> f64 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from(bits >> 10 & 31);
    let fraction = f64::from(bits & 1023);
    sign * match exponent {
        0 => fraction * 2f64.powi(-24),
        31 if fraction == 0.0 => f64::INFINITY,
        31 => f64::NAN,
        _ => (1.0 + fraction / 1024.0) * 2f64.powi(exponent - 15),
    }
}

impl Element {
    /// The name of the constructor that makes these.
    pub(crate) fn constructor_name(self) -> &'static str {
        ELEMENTS
            .iter()
            .find(|(_, element)| *element == self)
            .map_or("Uint8Array", |(name, _)| name)
    }

    pub(crate) fn size(self) -> usize {
        match self {
            Self::Int8 | Self::Uint8 | Self::Uint8Clamped => 1,
            Self::Int16 | Self::Uint16 | Self::Float16 => 2,
            Self::Int32 | Self::Uint32 | Self::Float32 => 4,
            Self::Float64 | Self::BigInt64 | Self::BigUint64 => 8,
        }
    }

    pub(crate) fn is_big(self) -> bool {
        matches!(self, Self::BigInt64 | Self::BigUint64)
    }

    fn read(self, vm: &mut Vm, bytes: &[u8; 8]) -> Value {
        let [b0, b1, b2, b3, ..] = *bytes;
        match self {
            Self::Int8 => Value::int(i32::from(b0 as i8)),
            Self::Uint8 | Self::Uint8Clamped => Value::int(i32::from(b0)),
            Self::Int16 => Value::int(i32::from(i16::from_le_bytes([b0, b1]))),
            Self::Uint16 => Value::int(i32::from(u16::from_le_bytes([b0, b1]))),
            Self::Int32 => Value::int(i32::from_le_bytes([b0, b1, b2, b3])),
            Self::Uint32 => Value::number(f64::from(u32::from_le_bytes([b0, b1, b2, b3]))),
            Self::Float16 => Value::number(f16_bits_to_f64(u16::from_le_bytes([b0, b1]))),
            Self::Float32 => Value::number(f64::from(f32::from_le_bytes([b0, b1, b2, b3]))),
            Self::Float64 => Value::number(f64::from_le_bytes(*bytes)),
            Self::BigInt64 => vm.bigint_from_i64(i64::from_le_bytes(*bytes)),
            Self::BigUint64 => vm.new_bigint(BigData::from_u64(u64::from_le_bytes(*bytes))),
        }
    }

    /// The bytes for a value, which has been made into the right kind already.
    fn encode(self, vm: &Vm, item: Value, out: &mut [u8; 8]) {
        if self.is_big() {
            *out = vm.big_of(item).low_u64().to_le_bytes();
            return;
        }
        self.encode_number(item.as_number(), out);
    }

    /// The bytes for a number, for any kind but the bigint ones.
    fn encode_number(self, number: f64, out: &mut [u8; 8]) {
        let wrapped = crate::convert::f64_to_int32(number);
        match self {
            Self::Int8 | Self::Uint8 => out[0] = wrapped as u8,
            Self::Uint8Clamped => {
                out[0] = if number.is_nan() {
                    0
                } else {
                    // Halves go to the even number here, as the language says.
                    let clamped = number.clamp(0.0, 255.0);
                    let floor = clamped.floor();
                    let rounded = match clamped - floor {
                        difference if difference < 0.5 => floor,
                        difference if difference > 0.5 => floor + 1.0,
                        _ if floor % 2.0 == 0.0 => floor,
                        _ => floor + 1.0,
                    };
                    rounded as u8
                };
            }
            Self::Int16 | Self::Uint16 => {
                out[..2].copy_from_slice(&(wrapped as u16).to_le_bytes());
            }
            Self::Int32 | Self::Uint32 => out[..4].copy_from_slice(&wrapped.to_le_bytes()),
            Self::Float16 => out[..2].copy_from_slice(&f64_to_f16_bits(number).to_le_bytes()),
            Self::Float32 => out[..4].copy_from_slice(&(number as f32).to_le_bytes()),
            Self::Float64 => out[..8].copy_from_slice(&number.to_le_bytes()),
            Self::BigInt64 | Self::BigUint64 => unreachable!("handled above"),
        }
    }

    /// Makes a value into what this kind of element holds.
    fn convert(self, vm: &mut Vm, item: Value) -> Run<Value> {
        if self.is_big() {
            return super::bigint::to_bigint(vm, item);
        }
        Ok(Value::number(vm.to_number(item)?))
    }
}

pub(crate) struct BufferData {
    pub(crate) bytes: Vec<u8>,
    /// How big it can grow to, if it's resizable.
    pub(crate) max: Option<usize>,
    pub(crate) detached: bool,
}

pub(crate) struct TypedData {
    pub(crate) element: Element,
    pub(crate) buffer: Value,
    pub(crate) offset: usize,
    /// The length it was made with. If it follows its buffer's size, that's ignored.
    pub(crate) length: usize,
    pub(crate) follows_buffer: bool,
    /// Where its first element is in its buffer's memory, for the interpreter's quick reads and writes
    /// ([`TypedData::read_quick`], [`TypedData::write_quick`]), with `quick_length` elements from there. A buffer's
    /// memory only moves or goes when it's resized or transferred, which changes [`Vm::buffer_epoch`], so these are
    /// right while `epoch` is that. Until then they're worked out again ([`Vm::typed_refresh`]), and anything other
    /// than a plain view of numbers (see `Vm::typed_plain_view`) gets no quick elements, going the long way. Going
    /// through the view, its buffer and the buffer's bytes for each element was most of what reading one cost.
    pub(crate) quick: *mut u8,
    pub(crate) quick_length: usize,
    pub(crate) epoch: u64,
}

impl TypedData {
    /// Element `index`, which is below `quick_length` while `epoch` is [`Vm::buffer_epoch`].
    ///
    /// # Safety
    ///
    /// As that says: `quick` then has `quick_length` elements of this kind.
    #[inline(always)]
    #[allow(unsafe_code)]
    pub(crate) unsafe fn read_quick(&self, index: usize) -> Value {
        let at = self.quick;
        unsafe {
            match self.element {
                Element::Int8 => Value::int(i32::from(*at.add(index) as i8)),
                Element::Uint8 | Element::Uint8Clamped => Value::int(i32::from(*at.add(index))),
                Element::Int16 => Value::int(i32::from(i16::from_le(
                    at.cast::<i16>().add(index).read_unaligned(),
                ))),
                Element::Uint16 => Value::int(i32::from(u16::from_le(
                    at.cast::<u16>().add(index).read_unaligned(),
                ))),
                Element::Int32 => Value::int(i32::from_le(at.cast::<i32>().add(index).read_unaligned())),
                Element::Uint32 => Value::int_or_double(
                    u32::from_le(at.cast::<u32>().add(index).read_unaligned()) as usize,
                ),
                Element::Float32 => Value::number(f64::from(f32::from_bits(u32::from_le(
                    at.cast::<u32>().add(index).read_unaligned(),
                )))),
                Element::Float64 => Value::number(f64::from_bits(u64::from_le(
                    at.cast::<u64>().add(index).read_unaligned(),
                ))),
                // These never have quick elements.
                Element::Float16 | Element::BigInt64 | Element::BigUint64 => Value::UNDEFINED,
            }
        }
    }

    /// Writes a number to element `index`, as for [`TypedData::read_quick`].
    ///
    /// # Safety
    ///
    /// As for `read_quick`, and `item` is a number.
    #[inline(always)]
    #[allow(unsafe_code)]
    pub(crate) unsafe fn write_quick(&self, index: usize, item: Value) {
        if !item.is_int() {
            return unsafe { self.write_double_quick(index, item.as_double()) };
        }
        let (at, int) = (self.quick, item.as_int());
        unsafe {
            match self.element {
                Element::Int8 | Element::Uint8 => *at.add(index) = int as u8,
                Element::Uint8Clamped => *at.add(index) = int.clamp(0, 255) as u8,
                Element::Int16 | Element::Uint16 => {
                    at.cast::<u16>().add(index).write_unaligned((int as u16).to_le());
                }
                Element::Int32 | Element::Uint32 => {
                    at.cast::<i32>().add(index).write_unaligned(int.to_le());
                }
                Element::Float32 => {
                    at.cast::<u32>().add(index).write_unaligned((int as f32).to_bits().to_le());
                }
                Element::Float64 => {
                    at.cast::<u64>().add(index).write_unaligned(f64::from(int).to_bits().to_le());
                }
                Element::Float16 | Element::BigInt64 | Element::BigUint64 => {}
            }
        }
    }

    /// `write_quick` for a number that isn't an int, which takes more code than belongs in the interpreter's loop.
    #[inline(never)]
    #[allow(unsafe_code)]
    unsafe fn write_double_quick(&self, index: usize, number: f64) {
        let element = self.element;
        let mut piece = [0u8; 8];
        element.encode_number(number, &mut piece);
        let size = element.size();
        unsafe { std::ptr::copy_nonoverlapping(piece.as_ptr(), self.quick.add(index * size), size) };
    }
}

pub(crate) struct ViewData {
    pub(crate) buffer: Value,
    pub(crate) offset: usize,
    pub(crate) length: usize,
    pub(crate) follows_buffer: bool,
}

impl Vm {
    pub(crate) fn new_array_buffer(&mut self, bytes: Vec<u8>) -> Value {
        self.debt += bytes.capacity();
        let proto = self.realm.array_buffer_proto;
        self.new_object_with_proto(
            proto,
            Kind::ArrayBuffer(Box::new(BufferData {
                bytes,
                max: None,
                detached: false,
            })),
        )
    }

    pub(crate) fn buffer_of(&self, value: Value) -> Option<&BufferData> {
        if !value.is_object() {
            return None;
        }
        match &self.objects[value.index()].kind {
            Kind::ArrayBuffer(data) => Some(data),
            _ => None,
        }
    }

    pub(crate) fn typed_data(&self, value: Value) -> Option<&TypedData> {
        if !value.is_object() {
            return None;
        }
        match &self.objects[value.index()].kind {
            Kind::TypedArray(data) => Some(data),
            _ => None,
        }
    }

    /// The length a typed array has right now, which is nothing if its buffer has gone or shrunk from under it.
    pub(crate) fn typed_length(&self, data: &TypedData) -> usize {
        let Some(buffer) = self.buffer_of(data.buffer) else {
            return 0;
        };
        if buffer.detached {
            return 0;
        }
        let total = buffer.bytes.len();
        if data.follows_buffer {
            return total.saturating_sub(data.offset) / data.element.size();
        }
        if data.offset + data.length * data.element.size() > total {
            return 0;
        }
        data.length
    }

    /// The same, from where its elements are (see [`TypedData::quick`]) when that's known: a plain view has a
    /// pointer there, and its length is all of its elements.
    #[inline(always)]
    pub(crate) fn typed_length_now(&self, data: &TypedData) -> usize {
        if data.epoch == self.buffer_epoch && !data.quick.is_null() {
            data.quick_length
        } else {
            self.typed_length(data)
        }
    }

    /// Whether a typed array can't be used at all, because of what's happened to its buffer.
    fn typed_is_broken(&self, data: &TypedData) -> bool {
        let Some(buffer) = self.buffer_of(data.buffer) else {
            return true;
        };
        if buffer.detached {
            return true;
        }
        let total = buffer.bytes.len();
        if data.follows_buffer {
            data.offset > total
        } else {
            data.offset + data.length * data.element.size() > total
        }
    }

    pub(crate) fn typed_get(&mut self, value: Value, index: usize) -> Option<Value> {
        let data = self.typed_data(value)?;
        if index >= self.typed_length(data) {
            return None;
        }
        let (element, offset, buffer) = (data.element, data.offset, data.buffer);
        let bytes = &self.buffer_of(buffer)?.bytes;
        let at = offset + index * element.size();
        let mut piece = [0u8; 8];
        piece[..element.size()].copy_from_slice(bytes.get(at..at + element.size())?);
        Some(element.read(self, &piece))
    }

    /// Where a typed array's elements are, as (buffer, first byte, length), if it's the usual kind: with a fixed
    /// length that's all still inside a buffer that's there. Anything else is for the long way.
    #[inline(always)]
    fn typed_plain_view(&self, data: &TypedData) -> Option<(u32, usize, usize)> {
        let Kind::ArrayBuffer(buffer) = &self.objects[data.buffer.index()].kind else {
            return None;
        };
        if data.follows_buffer
            || buffer.detached
            || data.offset + data.length * data.element.size() > buffer.bytes.len()
        {
            return None;
        }
        Some((data.buffer.handle(), data.offset, data.length))
    }

    /// Works out again where a typed array's quick elements are (see [`TypedData::quick`]), once a buffer may have
    /// moved since they last were, or for the first time.
    #[inline(never)]
    #[allow(unsafe_code)]
    pub(crate) fn typed_refresh(&mut self, typed: u32) {
        let Kind::TypedArray(data) = &self.objects[typed as usize].kind else {
            return;
        };
        let view = match data.element {
            Element::Float16 | Element::BigInt64 | Element::BigUint64 => None,
            _ => self.typed_plain_view(data),
        };
        let (quick, quick_length) = match view {
            Some((buffer, offset, length)) => match &mut self.objects[buffer as usize].kind {
                // A plain view is all inside its buffer's bytes.
                Kind::ArrayBuffer(store) => (unsafe { store.bytes.as_mut_ptr().add(offset) }, length),
                _ => (std::ptr::null_mut(), 0),
            },
            None => (std::ptr::null_mut(), 0),
        };
        let epoch = self.buffer_epoch;
        if let Kind::TypedArray(data) = &mut self.objects[typed as usize].kind {
            data.quick = quick;
            data.quick_length = quick_length;
            data.epoch = epoch;
        }
    }

    /// Makes the instruction that's just run a SetElemTyped, if it's a SetElem (see `Op::SetElemTyped`). The frame's
    /// `pc` is past it.
    #[inline(never)]
    pub(crate) fn go_typed_set(&mut self) {
        use crate::op::Op;
        let Some(frame) = self.frames.last() else {
            return;
        };
        if let Some(op) = self.code.get_mut(frame.pc.wrapping_sub(1) as usize) {
            if let Op::SetElem { object, key, value } = *op {
                *op = Op::SetElemTyped { object, key, value };
            }
        }
    }

    /// Makes the SetElemTyped that's just run a SetElem again.
    #[inline(never)]
    pub(crate) fn go_untyped_set(&mut self) {
        use crate::op::Op;
        let Some(frame) = self.frames.last() else {
            return;
        };
        if let Some(op) = self.code.get_mut(frame.pc.wrapping_sub(1) as usize) {
            if let Op::SetElemTyped { object, key, value } = *op {
                *op = Op::SetElem { object, key, value };
            }
        }
    }

    /// Says a buffer's memory has moved or gone, so that no typed array uses where it was (see [`TypedData::quick`]).
    pub(crate) fn buffers_moved(&mut self) {
        self.buffer_epoch += 1;
    }

    /// Reads an element the short way, for the interpreter when it couldn't use the quick elements: `None` means it
    /// has to go the long way.
    #[inline(never)]
    #[allow(unsafe_code)]
    pub(crate) fn typed_get_quick(&mut self, typed: u32, index: usize) -> Option<Value> {
        let Kind::TypedArray(data) = &self.objects[typed as usize].kind else {
            return None;
        };
        if data.epoch != self.buffer_epoch {
            self.typed_refresh(typed);
            let Kind::TypedArray(data) = &self.objects[typed as usize].kind else {
                return None;
            };
            if index < data.quick_length {
                // Just worked out.
                return Some(unsafe { data.read_quick(index) });
            }
        }
        let Kind::TypedArray(data) = &self.objects[typed as usize].kind else {
            return None;
        };
        let element = data.element;
        let (buffer, offset, length) = self.typed_plain_view(data)?;
        if index >= length {
            return Some(Value::UNDEFINED);
        }
        let Kind::ArrayBuffer(buffer) = &self.objects[buffer as usize].kind else {
            return None;
        };
        read_plain(element, &buffer.bytes[offset..], index)
    }

    /// Writes a number to an element the short way, for the interpreter. It says whether it did; if not, the long way
    /// has to, as it's something that needs converting first, or a position that isn't there. (Unlike reads, this
    /// isn't in the interpreter's loop itself, which made writes to arrays slower.)
    #[inline(never)]
    #[allow(unsafe_code)]
    pub(crate) fn typed_set_quick(&mut self, typed: u32, index: usize, item: Value) -> bool {
        let Kind::TypedArray(data) = &self.objects[typed as usize].kind else {
            return false;
        };
        if data.epoch == self.buffer_epoch {
            if index < data.quick_length && item.is_number() {
                unsafe { data.write_quick(index, item) };
                return true;
            }
        } else {
            self.typed_refresh(typed);
            let Kind::TypedArray(data) = &self.objects[typed as usize].kind else {
                return false;
            };
            if index < data.quick_length && item.is_number() {
                // Just worked out.
                unsafe { data.write_quick(index, item) };
                return true;
            }
        }
        let Kind::TypedArray(data) = &self.objects[typed as usize].kind else {
            return false;
        };
        let element = data.element;
        if element.is_big() || !item.is_number() {
            return false;
        }
        let Some((buffer, offset, length)) = self.typed_plain_view(data) else {
            return false;
        };
        if index >= length {
            return false;
        }
        let mut piece = [0u8; 8];
        if item.is_int() {
            let int = item.as_int();
            match element {
                Element::Uint8Clamped => piece[0] = int.clamp(0, 255) as u8,
                Element::Float32 => piece[..4].copy_from_slice(&(int as f32).to_le_bytes()),
                Element::Float64 => piece = f64::from(int).to_le_bytes(),
                Element::Float16 => element.encode(self, item, &mut piece),
                _ => piece[..4].copy_from_slice(&int.to_le_bytes()),
            }
        } else {
            element.encode(self, item, &mut piece);
        }
        let Kind::ArrayBuffer(store) = &mut self.objects[buffer as usize].kind else {
            return false;
        };
        let size = element.size();
        let at = offset + index * size;
        store.bytes[at..at + size].copy_from_slice(&piece[..size]);
        true
    }

    /// What `target.set(source, at)` does when the source is a typed array too and both are the usual kind, without
    /// going through each element as a value. It says whether it could.
    fn typed_copy_quick(&mut self, target: Value, source: Value, at: usize) -> bool {
        let (Some(to), Some(from)) = (self.typed_data(target), self.typed_data(source)) else {
            return false;
        };
        let (to_element, from_element) = (to.element, from.element);
        if to_element != from_element && (to_element.is_big() || from_element.is_big()) {
            return false;
        }
        let (
            Some((to_buffer, to_offset, to_length)),
            Some((from_buffer, from_offset, from_length)),
        ) = (self.typed_plain_view(to), self.typed_plain_view(from))
        else {
            return false;
        };
        if at + from_length > to_length {
            return false;
        }
        let Kind::ArrayBuffer(store) = &self.objects[from_buffer as usize].kind else {
            return false;
        };
        // A copy first, as the two may share memory.
        let source_bytes =
            &store.bytes[from_offset..from_offset + from_length * from_element.size()];
        let bytes = if to_element == from_element {
            source_bytes.to_vec()
        } else {
            let size = to_element.size();
            let mut out = vec![0u8; from_length * size];
            let mut piece = [0u8; 8];
            for index in 0..from_length {
                let Some(item) = read_plain(from_element, source_bytes, index) else {
                    return false;
                };
                to_element.encode(self, item, &mut piece);
                out[index * size..(index + 1) * size].copy_from_slice(&piece[..size]);
            }
            out
        };
        let Kind::ArrayBuffer(store) = &mut self.objects[to_buffer as usize].kind else {
            return false;
        };
        let start = to_offset + at * to_element.size();
        store.bytes[start..start + bytes.len()].copy_from_slice(&bytes);
        true
    }

    /// Writes an element. A position past the end is quietly nothing, as the language says.
    pub(crate) fn typed_set(&mut self, value: Value, index: usize, item: Value) -> Run<()> {
        let Some(data) = self.typed_data(value) else {
            return Ok(());
        };
        let element = data.element;
        let converted = element.convert(self, item)?;
        let Some(data) = self.typed_data(value) else {
            return Ok(());
        };
        if index >= self.typed_length(data) {
            return Ok(());
        }
        let (buffer, offset) = (data.buffer, data.offset);
        let mut piece = [0u8; 8];
        element.encode(self, converted, &mut piece);
        if let Kind::ArrayBuffer(store) = &mut self.objects[buffer.index()].kind {
            let at = offset + index * element.size();
            if let Some(to) = store.bytes.get_mut(at..at + element.size()) {
                to.copy_from_slice(&piece[..element.size()]);
            }
        }
        Ok(())
    }

    fn new_typed(
        &mut self,
        element: Element,
        buffer: Value,
        offset: usize,
        length: usize,
        follows_buffer: bool,
        proto: u32,
    ) -> Value {
        self.new_object_with_proto(
            proto,
            Kind::TypedArray(Box::new(TypedData {
                element,
                buffer,
                offset,
                length,
                follows_buffer,
                quick: std::ptr::null_mut(),
                quick_length: 0,
                // Never what `buffer_epoch` is, so the first read or write works them out.
                epoch: 0,
            })),
        )
    }

    /// A new typed array of the same kind as another, with its own memory.
    fn typed_like(&mut self, this: Value, length: usize) -> Run<Value> {
        let Some(data) = self.typed_data(this) else {
            return Err(self.type_error("this is not a typed array."));
        };
        let element = data.element;
        let buffer = self.new_array_buffer(vec![0; length * element.size()]);
        let proto = self.proto_of(this.handle());
        Ok(self.new_typed(element, buffer, 0, length, false, proto))
    }
}

/// A typed array's kind and length, for something that needs it to be usable.
fn this_typed(vm: &mut Vm, this: Value) -> Run<(Element, usize)> {
    let Some(data) = vm.typed_data(this) else {
        return Err(vm.type_error("this is not a typed array."));
    };
    if vm.typed_is_broken(data) {
        return Err(vm.type_error("Cannot perform this on a detached ArrayBuffer"));
    }
    let length = vm.typed_length(data);
    let Some(data) = vm.typed_data(this) else {
        return Err(vm.type_error("this is not a typed array."));
    };
    Ok((data.element, length))
}

/// Reads a position or a length, which has to be a whole number that isn't negative or huge.
fn to_index(vm: &mut Vm, value: Value) -> Run<usize> {
    let number = vm.to_integer(value)?;
    if !(0.0..=9_007_199_254_740_991.0).contains(&number) {
        return Err(vm.range_error("Invalid index"));
    }
    Ok(number as usize)
}

/// The biggest buffer that's worth trying to make. Asking for more is much more likely a mistake.
const MAX_BUFFER: usize = 1 << 33;

fn array_buffer_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    if !args.new_target.is_object() {
        return Err(vm.type_error("Constructor ArrayBuffer requires 'new'"));
    }
    let length = vm.arg(&args, 0);
    let length = to_index(vm, length)?;
    let options = vm.arg(&args, 1);
    let max = if options.is_object() {
        let max = vm.get_named(options, "maxByteLength")?;
        if max.is_undefined() {
            None
        } else {
            Some(to_index(vm, max)?)
        }
    } else {
        None
    };
    let proto = vm.proto_for(&args, vm.realm.array_buffer_proto)?;
    if length > MAX_BUFFER || max.is_some_and(|max| length > max || max > MAX_BUFFER * 4) {
        return Err(vm.range_error("Array buffer allocation failed"));
    }
    let buffer = vm.new_array_buffer(vec![0; length]);
    if let Kind::ArrayBuffer(data) = &mut vm.objects[buffer.index()].kind {
        data.max = max;
    }
    if proto != vm.realm.array_buffer_proto {
        vm.set_proto(buffer.handle(), proto);
    }
    Ok(buffer)
}

fn array_buffer_is_view(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    Ok(Value::bool(
        value.is_object()
            && matches!(
                vm.objects[value.index()].kind,
                Kind::TypedArray(_) | Kind::DataView(_)
            ),
    ))
}

fn this_buffer(vm: &mut Vm, this: Value) -> Run<&mut BufferData> {
    if this.is_object() && matches!(vm.objects[this.index()].kind, Kind::ArrayBuffer(_)) {
        let Kind::ArrayBuffer(data) = &mut vm.objects[this.index()].kind else {
            unreachable!("just checked");
        };
        return Ok(data);
    }
    Err(vm.type_error("this is not an ArrayBuffer."))
}

fn buffer_byte_length(vm: &mut Vm, args: Args) -> Run<Value> {
    let data = this_buffer(vm, args.this)?;
    Ok(Value::number(data.bytes.len() as f64))
}

fn buffer_max_byte_length(vm: &mut Vm, args: Args) -> Run<Value> {
    let data = this_buffer(vm, args.this)?;
    let max = if data.detached {
        0
    } else {
        data.max.unwrap_or(data.bytes.len())
    };
    Ok(Value::number(max as f64))
}

fn buffer_resizable(vm: &mut Vm, args: Args) -> Run<Value> {
    let data = this_buffer(vm, args.this)?;
    Ok(Value::bool(data.max.is_some()))
}

fn buffer_detached(vm: &mut Vm, args: Args) -> Run<Value> {
    let data = this_buffer(vm, args.this)?;
    Ok(Value::bool(data.detached))
}

fn buffer_resize(vm: &mut Vm, args: Args) -> Run<Value> {
    let data = this_buffer(vm, args.this)?;
    if data.max.is_none() {
        return Err(vm.type_error("This ArrayBuffer isn't resizable"));
    }
    let length = vm.arg(&args, 0);
    let length = to_index(vm, length)?;
    let data = this_buffer(vm, args.this)?;
    if data.detached {
        return Err(vm.type_error("Cannot perform this on a detached ArrayBuffer"));
    }
    if data.max.is_some_and(|max| length > max) {
        return Err(
            vm.range_error("The new length is more than the most this ArrayBuffer can have")
        );
    }
    data.bytes.resize(length, 0);
    vm.buffers_moved();
    Ok(Value::UNDEFINED)
}

fn transfer_buffer(vm: &mut Vm, args: &Args, keep_resizable: bool) -> Run<Value> {
    this_buffer(vm, args.this)?;
    let length = vm.arg(args, 0);
    let new_length = if length.is_undefined() {
        None
    } else {
        Some(to_index(vm, length)?)
    };
    let data = this_buffer(vm, args.this)?;
    if data.detached {
        return Err(vm.type_error("Cannot perform this on a detached ArrayBuffer"));
    }
    let max = if keep_resizable { data.max } else { None };
    let new_length = new_length.unwrap_or(data.bytes.len());
    if max.is_some_and(|max| new_length > max) || new_length > MAX_BUFFER {
        return Err(vm.range_error("The new length is too big"));
    }
    let mut bytes = std::mem::take(&mut data.bytes);
    data.detached = true;
    vm.buffers_moved();
    bytes.resize(new_length, 0);
    let buffer = vm.new_array_buffer(bytes);
    if let Kind::ArrayBuffer(data) = &mut vm.objects[buffer.index()].kind {
        data.max = max;
    }
    Ok(buffer)
}

fn buffer_transfer(vm: &mut Vm, args: Args) -> Run<Value> {
    transfer_buffer(vm, &args, true)
}

fn buffer_transfer_fixed(vm: &mut Vm, args: Args) -> Run<Value> {
    transfer_buffer(vm, &args, false)
}

fn buffer_slice(vm: &mut Vm, args: Args) -> Run<Value> {
    let data = this_buffer(vm, args.this)?;
    if data.detached {
        return Err(vm.type_error("Cannot perform this on a detached ArrayBuffer"));
    }
    let length = data.bytes.len();
    let (start, end) = (vm.arg(&args, 0), vm.arg(&args, 1));
    let start = vm.relative_index(start, length, 0)?;
    let end = vm.relative_index(end, length, length)?;
    let data = this_buffer(vm, args.this)?;
    if data.detached {
        return Err(vm.type_error("Cannot perform this on a detached ArrayBuffer"));
    }
    let end = end.min(data.bytes.len());
    let copy = if start < end {
        data.bytes[start..end].to_vec()
    } else {
        Vec::new()
    };
    Ok(vm.new_array_buffer(copy))
}

/// The kind of element a constructor makes: it knows by its name.
fn element_of_constructor(vm: &mut Vm, args: &Args) -> Element {
    let callee = match &vm.objects[args.callee as usize].kind {
        Kind::Native(native) => native.name,
        _ => text::EMPTY_NAME,
    };
    let name = vm.atom_text(callee);
    ELEMENTS
        .iter()
        .find(|(found, _)| *found == name)
        .map_or(Element::Uint8, |(_, element)| *element)
}

/// Everything in an iterable or an array-like, read one at a time.
fn read_list(vm: &mut Vm, source: Value) -> Run<Vec<Value>> {
    let method = vm.get(source, text::SYMBOL_ITERATOR)?;
    if !method.is_nullish() {
        if !vm.is_callable(method) {
            return Err(vm.type_error("Symbol.iterator is not a function"));
        }
        return vm.iterate_to_vec(source);
    }
    let length = vm.length_of(source)?;
    if length > MAX_BUFFER {
        return Err(vm.range_error("Invalid typed array length"));
    }
    let mut items = Vec::with_capacity(length.min(1 << 16));
    for index in 0..length {
        let item = vm.get_index(source, index)?;
        vm.nursery.push(item);
        items.push(item);
    }
    Ok(items)
}

fn typed_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    if !args.new_target.is_object() {
        return Err(vm.type_error("A typed array's constructor requires 'new'"));
    }
    let element = element_of_constructor(vm, &args);
    let callee = Value::object(args.callee);
    let default = vm.get(callee, text::PROTOTYPE)?.handle();
    let first = vm.arg(&args, 0);
    if !first.is_object() {
        let length = to_index(vm, first)?;
        let proto = vm.proto_for(&args, default)?;
        if length.saturating_mul(element.size()) > MAX_BUFFER {
            return Err(vm.range_error("Invalid typed array length"));
        }
        let buffer = vm.new_array_buffer(vec![0; length * element.size()]);
        return Ok(vm.new_typed(element, buffer, 0, length, false, proto));
    }
    if vm.buffer_of(first).is_some() {
        let proto = vm.proto_for(&args, default)?;
        let (offset, length) = (vm.arg(&args, 1), vm.arg(&args, 2));
        let offset = to_index(vm, offset)?;
        if offset % element.size() != 0 {
            return Err(vm.range_error(&format!(
                "start offset of typed array should be a multiple of {}",
                element.size()
            )));
        }
        let new_length = if length.is_undefined() {
            None
        } else {
            Some(to_index(vm, length)?)
        };
        let Some(data) = vm.buffer_of(first) else {
            unreachable!("just checked");
        };
        if data.detached {
            return Err(vm.type_error("Cannot perform this on a detached ArrayBuffer"));
        }
        let total = data.bytes.len();
        let resizable = data.max.is_some();
        if new_length.is_none() && resizable {
            if offset > total {
                return Err(vm.range_error("Start offset is outside the bounds of the buffer"));
            }
            return Ok(vm.new_typed(element, first, offset, 0, true, proto));
        }
        let length = match new_length {
            None => {
                if total % element.size() != 0 {
                    return Err(vm.range_error(&format!(
                        "byte length of typed array should be a multiple of {}",
                        element.size()
                    )));
                }
                if offset > total {
                    return Err(vm.range_error("Start offset is outside the bounds of the buffer"));
                }
                (total - offset) / element.size()
            }
            Some(length) => {
                if offset.saturating_add(length.saturating_mul(element.size())) > total {
                    return Err(vm.range_error("Invalid typed array length"));
                }
                length
            }
        };
        return Ok(vm.new_typed(element, first, offset, length, false, proto));
    }
    let proto = vm.proto_for(&args, default)?;
    let items = if let Some(data) = vm.typed_data(first) {
        if vm.typed_is_broken(data) {
            return Err(vm.type_error("Cannot perform this on a detached ArrayBuffer"));
        }
        let other_big = data.element.is_big();
        if other_big != element.is_big() {
            return Err(
                vm.type_error("Cannot mix BigInt and other types, use explicit conversions")
            );
        }
        let length = vm.length_of(first)?;
        let mut items = Vec::with_capacity(length);
        for index in 0..length {
            items.push(vm.get_index(first, index)?);
        }
        items
    } else {
        read_list(vm, first)?
    };
    vm.nursery.extend_from_slice(&items);
    if items.len().saturating_mul(element.size()) > MAX_BUFFER {
        return Err(vm.range_error("Invalid typed array length"));
    }
    let buffer = vm.new_array_buffer(vec![0; items.len() * element.size()]);
    let typed = vm.new_typed(element, buffer, 0, items.len(), false, proto);
    for (index, item) in items.into_iter().enumerate() {
        vm.typed_set(typed, index, item)?;
    }
    Ok(typed)
}

fn typed_from(vm: &mut Vm, args: Args) -> Run<Value> {
    if !vm.is_constructor(args.this) {
        return Err(vm.not_callable(args.this, true));
    }
    let source = vm.arg(&args, 0);
    let mapper = vm.arg(&args, 1);
    if !mapper.is_undefined() && !vm.is_callable(mapper) {
        return Err(vm.not_callable(mapper, false));
    }
    let this_arg = vm.arg(&args, 2);
    let mut items = read_list(vm, source)?;
    vm.nursery.extend_from_slice(&items);
    if !mapper.is_undefined() {
        let callee = vm.prepare_call(mapper);
        for (index, item) in items.iter_mut().enumerate() {
            *item = vm.call_prepared(&callee, this_arg, &[*item, Value::number(index as f64)])?;
        }
    }
    let list = vm.new_array(items);
    vm.construct(args.this, &[list])
}

fn typed_of(vm: &mut Vm, args: Args) -> Run<Value> {
    if !vm.is_constructor(args.this) {
        return Err(vm.not_callable(args.this, true));
    }
    let items = (0..args.count).map(|index| vm.arg(&args, index)).collect();
    let list = vm.new_array(items);
    vm.construct(args.this, &[list])
}

impl Vm {
    /// What a typed array's `length` getter would give, if this getter is the built-in one, without calling it.
    #[inline]
    pub(crate) fn typed_length_by_getter(&self, getter: Value, target: Value) -> Option<Value> {
        let Kind::Native(native) = &self.objects[getter.index()].kind else {
            return None;
        };
        if native.func as usize != typed_length_getter as NativeFn as usize {
            return None;
        }
        if !target.is_object() {
            return None;
        }
        let Kind::TypedArray(data) = &self.objects[target.index()].kind else {
            return None;
        };
        Some(Value::int_or_double(self.typed_length(data)))
    }
}

fn typed_length_getter(vm: &mut Vm, args: Args) -> Run<Value> {
    let Some(data) = vm.typed_data(args.this) else {
        return Err(vm.type_error("this is not a typed array."));
    };
    Ok(Value::number(vm.typed_length(data) as f64))
}

fn typed_byte_length(vm: &mut Vm, args: Args) -> Run<Value> {
    let Some(data) = vm.typed_data(args.this) else {
        return Err(vm.type_error("this is not a typed array."));
    };
    Ok(Value::number(
        (vm.typed_length(data) * data.element.size()) as f64,
    ))
}

fn typed_byte_offset(vm: &mut Vm, args: Args) -> Run<Value> {
    let Some(data) = vm.typed_data(args.this) else {
        return Err(vm.type_error("this is not a typed array."));
    };
    Ok(Value::number(if vm.typed_is_broken(data) {
        0.0
    } else {
        data.offset as f64
    }))
}

fn typed_buffer(vm: &mut Vm, args: Args) -> Run<Value> {
    match vm.typed_data(args.this) {
        Some(data) => Ok(data.buffer),
        None => Err(vm.type_error("this is not a typed array.")),
    }
}

/// Runs what arrays do, on something that's been checked to be a usable typed array.
fn checked_array_method(vm: &mut Vm, args: Args) -> Run<Value> {
    this_typed(vm, args.this)?;
    let function = vm.native_data_of(&args);
    let list: Vec<Value> = (0..args.count).map(|index| vm.arg(&args, index)).collect();
    vm.call(function, args.this, &list)
}

fn typed_set_method(vm: &mut Vm, args: Args) -> Run<Value> {
    let Some(data) = vm.typed_data(args.this) else {
        return Err(vm.type_error("this is not a typed array."));
    };
    let is_big = data.element.is_big();
    let offset = vm.arg(&args, 1);
    let offset = vm.to_integer(offset)?;
    if offset < 0.0 {
        return Err(vm.range_error("offset is out of bounds"));
    }
    let (_, length) = this_typed(vm, args.this)?;
    let source = vm.arg(&args, 0);
    if let Some(other) = vm.typed_data(source) {
        if vm.typed_is_broken(other) {
            return Err(vm.type_error("Cannot perform this on a detached ArrayBuffer"));
        }
        if other.element.is_big() != is_big {
            return Err(
                vm.type_error("Cannot mix BigInt and other types, use explicit conversions")
            );
        }
    }
    let source = vm.to_object(source)?;
    let count = vm.length_of(source)?;
    if offset + count as f64 > length as f64 {
        return Err(vm.range_error("offset is out of bounds"));
    }
    let offset = offset as usize;
    if vm.typed_copy_quick(args.this, source, offset) {
        return Ok(Value::UNDEFINED);
    }
    // Read everything first, as the two may share memory.
    let mut items = Vec::with_capacity(count);
    for index in 0..count {
        let item = vm.get_index(source, index)?;
        vm.nursery.push(item);
        items.push(item);
    }
    for (index, item) in items.into_iter().enumerate() {
        vm.typed_set(args.this, offset + index, item)?;
    }
    Ok(Value::UNDEFINED)
}

fn typed_subarray(vm: &mut Vm, args: Args) -> Run<Value> {
    let Some(data) = vm.typed_data(args.this) else {
        return Err(vm.type_error("this is not a typed array."));
    };
    let (element, buffer, offset, follows) =
        (data.element, data.buffer, data.offset, data.follows_buffer);
    let length = vm.typed_length(data);
    let (start, end) = (vm.arg(&args, 0), vm.arg(&args, 1));
    let start = vm.relative_index(start, length, 0)?;
    let end_is_given = !end.is_undefined();
    let end = vm.relative_index(end, length, length)?;
    let proto = vm.proto_of(args.this.handle());
    Ok(vm.new_typed(
        element,
        buffer,
        offset + start * element.size(),
        end.saturating_sub(start),
        follows && !end_is_given,
        proto,
    ))
}

fn typed_slice(vm: &mut Vm, args: Args) -> Run<Value> {
    let (_, length) = this_typed(vm, args.this)?;
    let (start, end) = (vm.arg(&args, 0), vm.arg(&args, 1));
    let start = vm.relative_index(start, length, 0)?;
    let end = vm.relative_index(end, length, length)?;
    let count = end.saturating_sub(start);
    let result = vm.typed_like(args.this, count)?;
    if count > 0 {
        this_typed(vm, args.this)?;
    }
    for index in 0..count {
        let item = vm.get_index(args.this, start + index)?;
        vm.typed_set(result, index, item)?;
    }
    Ok(result)
}

fn typed_fill(vm: &mut Vm, args: Args) -> Run<Value> {
    let (element, length) = this_typed(vm, args.this)?;
    let value = vm.arg(&args, 0);
    let value = element.convert(vm, value)?;
    let (start, end) = (vm.arg(&args, 1), vm.arg(&args, 2));
    let start = vm.relative_index(start, length, 0)?;
    let end = vm.relative_index(end, length, length)?;
    this_typed(vm, args.this)?;
    for index in start..end {
        vm.typed_set(args.this, index, value)?;
    }
    Ok(args.this)
}

fn typed_map(vm: &mut Vm, args: Args) -> Run<Value> {
    let (_, length) = this_typed(vm, args.this)?;
    let function = vm.arg(&args, 0);
    if !vm.is_callable(function) {
        return Err(vm.not_callable(function, false));
    }
    let this_arg = vm.arg(&args, 1);
    let result = vm.typed_like(args.this, length)?;
    let callee = vm.prepare_call(function);
    for index in 0..length {
        let item = vm.get_index(args.this, index)?;
        let mapped = vm.call_prepared(
            &callee,
            this_arg,
            &[item, Value::number(index as f64), args.this],
        )?;
        vm.typed_set(result, index, mapped)?;
    }
    Ok(result)
}

fn typed_filter(vm: &mut Vm, args: Args) -> Run<Value> {
    let (_, length) = this_typed(vm, args.this)?;
    let function = vm.arg(&args, 0);
    if !vm.is_callable(function) {
        return Err(vm.not_callable(function, false));
    }
    let this_arg = vm.arg(&args, 1);
    let mut kept = Vec::new();
    let callee = vm.prepare_call(function);
    for index in 0..length {
        let item = vm.get_index(args.this, index)?;
        let result = vm.call_prepared(
            &callee,
            this_arg,
            &[item, Value::number(index as f64), args.this],
        )?;
        if vm.to_boolean(result) {
            vm.nursery.push(item);
            kept.push(item);
        }
    }
    let result = vm.typed_like(args.this, kept.len())?;
    for (index, item) in kept.into_iter().enumerate() {
        vm.typed_set(result, index, item)?;
    }
    Ok(result)
}

fn typed_reverse(vm: &mut Vm, args: Args) -> Run<Value> {
    let (_, length) = this_typed(vm, args.this)?;
    for index in 0..length / 2 {
        let (low, high) = (
            vm.get_index(args.this, index)?,
            vm.get_index(args.this, length - 1 - index)?,
        );
        vm.typed_set(args.this, index, high)?;
        vm.typed_set(args.this, length - 1 - index, low)?;
    }
    Ok(args.this)
}

fn sort_numbers(vm: &mut Vm, element: Element, items: &mut [Value]) {
    if element.is_big() {
        items.sort_by(|a, b| {
            if vm.bigint_less_than(*a, *b) {
                std::cmp::Ordering::Less
            } else if vm.bigint_less_than(*b, *a) {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });
    } else {
        // NaNs go last, and negative zero before zero: which is the order the bits say, once the sign's dealt with.
        items.sort_by(|a, b| {
            let (x, y) = (a.as_number(), b.as_number());
            match (x.is_nan(), y.is_nan()) {
                (true, true) => std::cmp::Ordering::Equal,
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => x.total_cmp(&y),
            }
        });
    }
}

fn typed_sort(vm: &mut Vm, args: Args) -> Run<Value> {
    let comparator = vm.arg(&args, 0);
    if !comparator.is_undefined() && !vm.is_callable(comparator) {
        return Err(vm.type_error("The comparison function must be either a function or undefined"));
    }
    let (element, length) = this_typed(vm, args.this)?;
    let mut items = Vec::with_capacity(length);
    for index in 0..length {
        let item = vm.get_index(args.this, index)?;
        vm.nursery.push(item);
        items.push(item);
    }
    if comparator.is_undefined() {
        sort_numbers(vm, element, &mut items);
    } else {
        items = super::array::sorted(vm, comparator, items)?;
    }
    for (index, item) in items.into_iter().enumerate() {
        vm.typed_set(args.this, index, item)?;
    }
    Ok(args.this)
}

fn typed_to_sorted(vm: &mut Vm, args: Args) -> Run<Value> {
    let comparator = vm.arg(&args, 0);
    if !comparator.is_undefined() && !vm.is_callable(comparator) {
        return Err(vm.type_error("The comparison function must be either a function or undefined"));
    }
    let (element, length) = this_typed(vm, args.this)?;
    let result = vm.typed_like(args.this, length)?;
    let mut items = Vec::with_capacity(length);
    for index in 0..length {
        let item = vm.get_index(args.this, index)?;
        vm.nursery.push(item);
        items.push(item);
    }
    if comparator.is_undefined() {
        sort_numbers(vm, element, &mut items);
    } else {
        items = super::array::sorted(vm, comparator, items)?;
    }
    for (index, item) in items.into_iter().enumerate() {
        vm.typed_set(result, index, item)?;
    }
    Ok(result)
}

fn typed_to_reversed(vm: &mut Vm, args: Args) -> Run<Value> {
    let (_, length) = this_typed(vm, args.this)?;
    let result = vm.typed_like(args.this, length)?;
    for index in 0..length {
        let item = vm.get_index(args.this, length - 1 - index)?;
        vm.typed_set(result, index, item)?;
    }
    Ok(result)
}

fn typed_with(vm: &mut Vm, args: Args) -> Run<Value> {
    let (element, length) = this_typed(vm, args.this)?;
    let index = vm.arg(&args, 0);
    let number = vm.to_integer(index)?;
    let at = if number < 0.0 {
        length as f64 + number
    } else {
        number
    };
    let value = vm.arg(&args, 1);
    let value = element.convert(vm, value)?;
    let Some(data) = vm.typed_data(args.this) else {
        return Err(vm.type_error("this is not a typed array."));
    };
    let current = vm.typed_length(data);
    if at < 0.0 || at >= current as f64 {
        return Err(vm.range_error("Invalid typed array index"));
    }
    let result = vm.typed_like(args.this, length)?;
    for index in 0..length {
        let item = if index == at as usize {
            value
        } else {
            vm.get_index(args.this, index)?
        };
        vm.typed_set(result, index, item)?;
    }
    Ok(result)
}

fn typed_copy_within(vm: &mut Vm, args: Args) -> Run<Value> {
    let (_, length) = this_typed(vm, args.this)?;
    let (target, start, end) = (vm.arg(&args, 0), vm.arg(&args, 1), vm.arg(&args, 2));
    let target = vm.relative_index(target, length, 0)?;
    let start = vm.relative_index(start, length, 0)?;
    let end = vm.relative_index(end, length, length)?;
    let (_, current) = this_typed(vm, args.this)?;
    let count = end.saturating_sub(start).min(length - target);
    let mut items = Vec::with_capacity(count);
    for offset in 0..count {
        if start + offset >= current {
            break;
        }
        items.push(vm.get_index(args.this, start + offset)?);
    }
    for (offset, item) in items.into_iter().enumerate() {
        vm.typed_set(args.this, target + offset, item)?;
    }
    Ok(args.this)
}

fn data_view_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    if !args.new_target.is_object() {
        return Err(vm.type_error("Constructor DataView requires 'new'"));
    }
    let buffer = vm.arg(&args, 0);
    if vm.buffer_of(buffer).is_none() {
        return Err(vm.type_error("First argument to DataView constructor must be an ArrayBuffer"));
    }
    let (offset, length) = (vm.arg(&args, 1), vm.arg(&args, 2));
    let offset = to_index(vm, offset)?;
    let new_length = if length.is_undefined() {
        None
    } else {
        Some(to_index(vm, length)?)
    };
    let check = |vm: &mut Vm| -> Run<(usize, bool)> {
        let Some(data) = vm.buffer_of(buffer) else {
            unreachable!("just checked");
        };
        if data.detached {
            return Err(vm.type_error("Cannot perform this on a detached ArrayBuffer"));
        }
        let found = (data.bytes.len(), data.max.is_some());
        if offset > found.0 {
            return Err(vm.range_error("Start offset is outside the bounds of the buffer"));
        }
        if new_length.is_some_and(|length| offset.saturating_add(length) > found.0) {
            return Err(vm.range_error("Invalid DataView length"));
        }
        Ok(found)
    };
    check(vm)?;
    let proto = vm.proto_for(&args, vm.realm.data_view_proto)?;
    // Finding the prototype can run code, which can do anything to the buffer.
    let (total, resizable) = check(vm)?;
    Ok(vm.new_object_with_proto(
        proto,
        Kind::DataView(Box::new(ViewData {
            buffer,
            offset,
            length: new_length.unwrap_or(total - offset),
            follows_buffer: new_length.is_none() && resizable,
        })),
    ))
}

/// A view's buffer, where it starts, and how long it is right now. It's an error if it can't be used.
fn view_parts(vm: &mut Vm, this: Value) -> Run<(Value, usize, usize)> {
    let view = if this.is_object() {
        match &vm.objects[this.index()].kind {
            Kind::DataView(data) => {
                Some((data.buffer, data.offset, data.length, data.follows_buffer))
            }
            _ => None,
        }
    } else {
        None
    };
    let Some((buffer, offset, length, follows)) = view else {
        return Err(vm.type_error("this is not a DataView."));
    };
    let Some(data) = vm.buffer_of(buffer) else {
        return Err(vm.type_error("this is not a DataView."));
    };
    let total = data.bytes.len();
    if data.detached {
        return Err(vm.type_error("Cannot perform this on a detached ArrayBuffer"));
    }
    if (follows && offset > total) || (!follows && offset + length > total) {
        return Err(vm.type_error("The DataView is outside the bounds of its buffer"));
    }
    Ok((
        buffer,
        offset,
        if follows { total - offset } else { length },
    ))
}

fn view_get(vm: &mut Vm, args: &Args, element: Element) -> Run<Value> {
    if !args.this.is_object() || !matches!(vm.objects[args.this.index()].kind, Kind::DataView(_)) {
        return Err(vm.type_error("this is not a DataView."));
    }
    let position = vm.arg(args, 0);
    let position = to_index(vm, position)?;
    let little = vm.arg(args, 1);
    let little = vm.to_boolean(little);
    let (buffer, offset, length) = view_parts(vm, args.this)?;
    if position.saturating_add(element.size()) > length {
        return Err(vm.range_error("Offset is outside the bounds of the DataView"));
    }
    let at = offset + position;
    let Some(data) = vm.buffer_of(buffer) else {
        return Err(vm.type_error("this is not a DataView."));
    };
    let mut piece = [0u8; 8];
    piece[..element.size()].copy_from_slice(&data.bytes[at..at + element.size()]);
    if !little {
        piece[..element.size()].reverse();
    }
    Ok(element.read(vm, &piece))
}

fn view_set(vm: &mut Vm, args: &Args, element: Element) -> Run<Value> {
    if !args.this.is_object() || !matches!(vm.objects[args.this.index()].kind, Kind::DataView(_)) {
        return Err(vm.type_error("this is not a DataView."));
    }
    let position = vm.arg(args, 0);
    let position = to_index(vm, position)?;
    let value = vm.arg(args, 1);
    let value = element.convert(vm, value)?;
    let little = vm.arg(args, 2);
    let little = vm.to_boolean(little);
    let (buffer, offset, length) = view_parts(vm, args.this)?;
    if position.saturating_add(element.size()) > length {
        return Err(vm.range_error("Offset is outside the bounds of the DataView"));
    }
    let at = offset + position;
    let mut piece = [0u8; 8];
    element.encode(vm, value, &mut piece);
    if !little {
        piece[..element.size()].reverse();
    }
    if let Kind::ArrayBuffer(data) = &mut vm.objects[buffer.index()].kind {
        data.bytes[at..at + element.size()].copy_from_slice(&piece[..element.size()]);
    }
    Ok(Value::UNDEFINED)
}

macro_rules! view_methods {
    ($($get:ident, $set:ident, $element:expr;)*) => {
        $(
            fn $get(vm: &mut Vm, args: Args) -> Run<Value> {
                view_get(vm, &args, $element)
            }
            fn $set(vm: &mut Vm, args: Args) -> Run<Value> {
                view_set(vm, &args, $element)
            }
        )*
    };
}

view_methods! {
    get_int8, set_int8, Element::Int8;
    get_uint8, set_uint8, Element::Uint8;
    get_int16, set_int16, Element::Int16;
    get_uint16, set_uint16, Element::Uint16;
    get_int32, set_int32, Element::Int32;
    get_uint32, set_uint32, Element::Uint32;
    get_float16, set_float16, Element::Float16;
    get_float32, set_float32, Element::Float32;
    get_float64, set_float64, Element::Float64;
    get_big_int64, set_big_int64, Element::BigInt64;
    get_big_uint64, set_big_uint64, Element::BigUint64;
}

fn view_byte_length(vm: &mut Vm, args: Args) -> Run<Value> {
    let (_, _, length) = view_parts(vm, args.this)?;
    Ok(Value::number(length as f64))
}

fn view_byte_offset(vm: &mut Vm, args: Args) -> Run<Value> {
    let (_, offset, _) = view_parts(vm, args.this)?;
    Ok(Value::number(offset as f64))
}

fn view_buffer(vm: &mut Vm, args: Args) -> Run<Value> {
    if args.this.is_object() {
        if let Kind::DataView(data) = &vm.objects[args.this.index()].kind {
            return Ok(data.buffer);
        }
    }
    Err(vm.type_error("this is not a DataView."))
}

fn typed_tag(vm: &mut Vm, args: Args) -> Run<Value> {
    let Some(data) = vm.typed_data(args.this) else {
        return Ok(Value::UNDEFINED);
    };
    let element = data.element;
    let name = ELEMENTS
        .iter()
        .find(|(_, found)| *found == element)
        .map_or("", |(name, _)| name);
    Ok(vm.new_str(name))
}

fn f16round(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let number = vm.to_number(value)?;
    Ok(Value::number(f16_bits_to_f64(f64_to_f16_bits(number))))
}

/// What all typed array constructors inherit from, which can't be called itself.
fn abstract_constructor(vm: &mut Vm, _: Args) -> Run<Value> {
    Err(vm.type_error("Abstract class TypedArray not directly constructable"))
}

pub(super) fn install(vm: &mut Vm) {
    let buffer_proto = vm.plain_proto();
    vm.realm.array_buffer_proto = buffer_proto;
    let buffer = vm.constructor("ArrayBuffer", 1, array_buffer_constructor, buffer_proto);
    vm.method(buffer, "isView", 1, array_buffer_is_view);
    vm.species_getter(buffer);
    vm.getter(buffer_proto, "byteLength", buffer_byte_length);
    vm.getter(buffer_proto, "maxByteLength", buffer_max_byte_length);
    vm.getter(buffer_proto, "resizable", buffer_resizable);
    vm.getter(buffer_proto, "detached", buffer_detached);
    vm.method(buffer_proto, "slice", 2, buffer_slice);
    vm.method(buffer_proto, "resize", 1, buffer_resize);
    vm.method(buffer_proto, "transfer", 0, buffer_transfer);
    vm.method(
        buffer_proto,
        "transferToFixedLength",
        0,
        buffer_transfer_fixed,
    );
    let tag = vm.new_str("ArrayBuffer");
    vm.add_property(buffer_proto, text::SYMBOL_TO_STRING_TAG, tag, CONFIGURABLE);

    // What all typed arrays have in common. A lot of what arrays do works on them just the same.
    let shared = vm.plain_proto();
    let array_proto = Value::object(vm.realm.array_proto);
    for name in [
        "join",
        "indexOf",
        "lastIndexOf",
        "includes",
        "find",
        "findIndex",
        "findLast",
        "findLastIndex",
        "forEach",
        "reduce",
        "reduceRight",
        "some",
        "every",
        "at",
        "keys",
        "entries",
        "values",
        "toString",
        "toLocaleString",
    ] {
        let function = vm.get_named(array_proto, name).unwrap_or(Value::UNDEFINED);
        let atom = vm.intern_str(name);
        let length = vm
            .get(function, text::LENGTH)
            .map_or(0, |found| found.as_int() as u8);
        let checked = vm.new_native(atom, length, checked_array_method, function, false);
        vm.add_property(shared, atom, checked, HIDDEN);
        if name == "values" {
            vm.add_property(shared, text::SYMBOL_ITERATOR, checked, HIDDEN);
        }
    }
    for (name, length, func) in [
        ("set", 1, typed_set_method as NativeFn),
        ("subarray", 2, typed_subarray),
        ("slice", 2, typed_slice),
        ("fill", 1, typed_fill),
        ("map", 1, typed_map),
        ("filter", 1, typed_filter),
        ("reverse", 0, typed_reverse),
        ("sort", 1, typed_sort),
        ("toSorted", 1, typed_to_sorted),
        ("toReversed", 0, typed_to_reversed),
        ("with", 2, typed_with),
        ("copyWithin", 2, typed_copy_within),
    ] {
        vm.method(shared, name, length, func);
    }
    vm.getter(shared, "length", typed_length_getter);
    vm.getter(shared, "byteLength", typed_byte_length);
    vm.getter(shared, "byteOffset", typed_byte_offset);
    vm.getter(shared, "buffer", typed_buffer);
    let tag_atom = vm.intern_str("get [Symbol.toStringTag]");
    let tag_getter = vm.new_native(tag_atom, 0, typed_tag, Value::UNDEFINED, false);
    vm.define_accessor(
        shared,
        text::SYMBOL_TO_STRING_TAG,
        Some(tag_getter),
        None,
        CONFIGURABLE,
    );

    let abstract_name = vm.intern_str("TypedArray");
    let parent = vm.new_native(
        abstract_name,
        0,
        abstract_constructor,
        Value::UNDEFINED,
        true,
    );
    vm.materialize_if_lazy(parent.handle());
    vm.add_property(parent.handle(), text::PROTOTYPE, Value::object(shared), 0);
    vm.add_property(shared, text::CONSTRUCTOR, parent, HIDDEN);
    vm.method(parent.handle(), "from", 1, typed_from);
    vm.method(parent.handle(), "of", 0, typed_of);
    vm.species_getter(parent.handle());

    for (name, element) in ELEMENTS {
        let proto = vm.proto_with_parent(shared);
        let constructor = vm.constructor(name, 3, typed_constructor, proto);
        vm.set_proto(constructor, parent.handle());
        let size = Value::int(element.size() as i32);
        vm.constant(constructor, "BYTES_PER_ELEMENT", size);
        vm.constant(proto, "BYTES_PER_ELEMENT", size);
    }

    let view_proto = vm.plain_proto();
    vm.realm.data_view_proto = view_proto;
    vm.constructor("DataView", 1, data_view_constructor, view_proto);
    for (name, length, func) in [
        ("getInt8", 1, get_int8 as NativeFn),
        ("setInt8", 2, set_int8),
        ("getUint8", 1, get_uint8),
        ("setUint8", 2, set_uint8),
        ("getInt16", 1, get_int16),
        ("setInt16", 2, set_int16),
        ("getUint16", 1, get_uint16),
        ("setUint16", 2, set_uint16),
        ("getInt32", 1, get_int32),
        ("setInt32", 2, set_int32),
        ("getUint32", 1, get_uint32),
        ("setUint32", 2, set_uint32),
        ("getFloat16", 1, get_float16),
        ("setFloat16", 2, set_float16),
        ("getFloat32", 1, get_float32),
        ("setFloat32", 2, set_float32),
        ("getFloat64", 1, get_float64),
        ("setFloat64", 2, set_float64),
        ("getBigInt64", 1, get_big_int64),
        ("setBigInt64", 2, set_big_int64),
        ("getBigUint64", 1, get_big_uint64),
        ("setBigUint64", 2, set_big_uint64),
    ] {
        vm.method(view_proto, name, length, func);
    }
    vm.getter(view_proto, "byteLength", view_byte_length);
    vm.getter(view_proto, "byteOffset", view_byte_offset);
    vm.getter(view_proto, "buffer", view_buffer);
    let tag = vm.new_str("DataView");
    vm.add_property(view_proto, text::SYMBOL_TO_STRING_TAG, tag, CONFIGURABLE);

    let math = vm
        .get_named(Value::object(vm.global), "Math")
        .unwrap_or(Value::UNDEFINED);
    if math.is_object() {
        vm.method(math.handle(), "f16round", 1, f16round);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_bit_floats_round_to_the_nearest_they_can_be() {
        for (number, bits) in [
            (0.0, 0x0000u16),
            (-0.0, 0x8000),
            (1.0, 0x3C00),
            (-2.0, 0xC000),
            (0.5, 0x3800),
            (65504.0, 0x7BFF),
            (65520.0, 0x7C00),
            (65519.99, 0x7BFF),
            (6.103515625e-5, 0x0400),
            (5.960464477539063e-8, 0x0001),
            (2.0e-8, 0x0000),
            (1.0000000001, 0x3C00),
            (f64::INFINITY, 0x7C00),
        ] {
            assert_eq!(f64_to_f16_bits(number), bits, "{number}");
        }
        assert_eq!(f16_bits_to_f64(0x3C00), 1.0);
        assert_eq!(f16_bits_to_f64(0x7BFF), 65504.0);
        assert_eq!(f16_bits_to_f64(0x0001), 5.960464477539063e-8);
        assert!(f16_bits_to_f64(0x7E00).is_nan());
        assert_eq!(f16_bits_to_f64(0xFC00), f64::NEG_INFINITY);
    }
}
