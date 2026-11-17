//! What `node/buffer.js` does a byte at a time: turning text into bytes and back in each of Node's encodings, and
//! comparing and searching bytes. Each does exactly what the JavaScript it replaced did, including where that's not
//! quite what Node does (hex with anything else in it is nothing, and a byte order mark is dropped from UTF-8).

use crate::builtins::typed::Element;
use crate::text::Text;
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The bytes a `Uint8Array` (or a `Uint8ClampedArray`) sees, as where they are, or `None` if it's anything else.
fn byte_view(vm: &Vm, value: Value) -> Option<(Value, usize, usize)> {
    let data = vm.typed_data(value)?;
    if !matches!(data.element, Element::Uint8 | Element::Uint8Clamped) {
        return None;
    }
    Some((data.buffer, data.offset, vm.typed_length(data)))
}

fn bytes_in(vm: &Vm, (buffer, offset, length): (Value, usize, usize)) -> &[u8] {
    match vm.buffer_of(buffer) {
        Some(store) => &store.bytes[offset..offset + length],
        None => &[],
    }
}

fn hex_digit(unit: u16) -> Option<u8> {
    match unit {
        0x30..=0x39 => Some((unit - 0x30) as u8),
        0x41..=0x46 => Some((unit - 0x41 + 10) as u8),
        0x61..=0x66 => Some((unit - 0x61 + 10) as u8),
        _ => None,
    }
}

/// Text as bytes, by the number `node/buffer.js` gives each encoding: UTF-8, hex, base64 (either alphabet), Latin-1 (and
/// ASCII, which is the same going this way), and UTF-16.
fn encode(text: &Text, kind: u32) -> Vec<u8> {
    let units: std::borrow::Cow<'_, [u16]> = match (text.ascii(), text.wide()) {
        (Some(ascii), _) => {
            match kind {
                // ASCII text is its own UTF-8 and its own Latin-1.
                0 | 3 => return ascii.to_vec(),
                _ => std::borrow::Cow::Owned(ascii.iter().map(|byte| u16::from(*byte)).collect()),
            }
        }
        (_, wide) => std::borrow::Cow::Borrowed(wide.unwrap_or(&[])),
    };
    match kind {
        0 => String::from_utf16_lossy(&units).into_bytes(),
        1 => {
            // Anything that isn't a hex digit makes it nothing, and a last digit on its own is left out.
            let Some(digits) = units.iter().map(|unit| hex_digit(*unit)).collect::<Option<Vec<u8>>>() else {
                return Vec::new();
            };
            digits.chunks_exact(2).map(|pair| pair[0] << 4 | pair[1]).collect()
        }
        2 => {
            // Either alphabet, with anything that's in neither (padding too) left out.
            let mut out = Vec::with_capacity(units.len() * 3 / 4);
            let (mut group, mut bits) = (0u32, 0);
            for unit in units.iter() {
                let digit = match *unit {
                    0x41..=0x5a => unit - 0x41,
                    0x61..=0x7a => unit - 0x61 + 26,
                    0x30..=0x39 => unit - 0x30 + 52,
                    0x2b | 0x2d => 62,
                    0x2f | 0x5f => 63,
                    _ => continue,
                };
                group = group << 6 | u32::from(digit);
                bits += 6;
                if bits >= 8 {
                    bits -= 8;
                    out.push((group >> bits) as u8);
                }
            }
            out
        }
        3 => {
            // A byte for each character, so a pair of surrogates is one, from the first of them.
            let mut out = Vec::with_capacity(units.len());
            let mut at = 0;
            while at < units.len() {
                let unit = units[at];
                out.push(unit as u8);
                let paired = (0xd800..0xdc00).contains(&unit)
                    && units.get(at + 1).is_some_and(|next| (0xdc00..0xe000).contains(next));
                at += if paired { 2 } else { 1 };
            }
            out
        }
        _ => units.iter().flat_map(|unit| unit.to_le_bytes()).collect(),
    }
}

fn base64(bytes: &[u8], url: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        let digits = [group >> 18, group >> 12, group >> 6, group].map(|digit| BASE64[digit as usize & 63]);
        let kept = chunk.len() + 1;
        out.extend_from_slice(&digits[..kept]);
        if !url {
            out.extend(std::iter::repeat_n(b'=', 4 - kept));
        }
    }
    if url {
        for byte in &mut out {
            match *byte {
                b'+' => *byte = b'-',
                b'/' => *byte = b'_',
                _ => {}
            }
        }
    }
    out
}

/// Bytes as text, by the number `node/buffer.js` gives each encoding.
fn decode(bytes: &[u8], kind: u32) -> Text {
    match kind {
        0 => {
            if bytes.is_ascii() {
                return Text::from_ascii(bytes);
            }
            let text = String::from_utf8_lossy(bytes);
            Text::from_str(text.strip_prefix('\u{FEFF}').unwrap_or(&text))
        }
        1 => {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let out: Vec<u8> = bytes
                .iter()
                .flat_map(|byte| [HEX[usize::from(byte >> 4)], HEX[usize::from(byte & 15)]])
                .collect();
            Text::from_ascii_vec(out)
        }
        2 | 3 => Text::from_ascii_vec(base64(bytes, kind == 3)),
        4 => {
            if bytes.is_ascii() {
                return Text::from_ascii(bytes);
            }
            let units: Vec<u16> = bytes.iter().map(|byte| u16::from(*byte)).collect();
            Text::from_units(&units)
        }
        5 => Text::map_ascii(bytes, |byte| byte & 127),
        _ => {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            Text::from_units(&units)
        }
    }
}

/// `bufferFrom(text, kind)`: an `ArrayBuffer` of the text's bytes.
fn buffer_from(vm: &mut Vm, args: Args) -> Run<Value> {
    let text = vm.arg(&args, 0);
    let kind = vm.arg(&args, 1);
    if !text.is_string() || !kind.is_number() {
        return Err(vm.type_error("bufferFrom takes text and a number"));
    }
    let bytes = encode(vm.text_of(text), kind.as_number() as u32);
    Ok(vm.new_array_buffer(bytes))
}

/// `bufferText(bytes, kind)`: what a `Uint8Array`'s bytes say.
fn buffer_text(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let kind = vm.arg(&args, 1);
    let (Some(view), true) = (byte_view(vm, value), kind.is_number()) else {
        return Err(vm.type_error("bufferText takes a Uint8Array and a number"));
    };
    let text = decode(bytes_in(vm, view), kind.as_number() as u32);
    Ok(vm.new_text(text))
}

/// `bufferCompare(a, b)`: -1, 0 or 1 by the bytes of two `Uint8Array`s, or `undefined` if either is anything else.
fn buffer_compare(vm: &mut Vm, args: Args) -> Run<Value> {
    let (a, b) = (vm.arg(&args, 0), vm.arg(&args, 1));
    let (Some(a), Some(b)) = (byte_view(vm, a), byte_view(vm, b)) else {
        return Ok(Value::UNDEFINED);
    };
    let order = bytes_in(vm, a).cmp(bytes_in(vm, b));
    Ok(Value::int(order as i32))
}

/// `bufferIndexOf(haystack, needle, from, last)`: where the bytes of one `Uint8Array` first (or, if `last`, last) are
/// in another, at or after (or before) `from`, which is a whole number at least 0. -1 if they aren't, and `undefined`
/// if either isn't a `Uint8Array`.
fn buffer_index_of(vm: &mut Vm, args: Args) -> Run<Value> {
    let (haystack, needle) = (vm.arg(&args, 0), vm.arg(&args, 1));
    let from = vm.arg(&args, 2).as_number();
    let last = vm.arg(&args, 3) == Value::TRUE;
    let (Some(haystack), Some(needle)) = (byte_view(vm, haystack), byte_view(vm, needle)) else {
        return Ok(Value::UNDEFINED);
    };
    let (haystack, needle) = (bytes_in(vm, haystack), bytes_in(vm, needle));
    let Some(room) = haystack.len().checked_sub(needle.len()) else {
        return Ok(Value::int(-1));
    };
    let from = from as usize;
    let found = if last {
        (0..=room.min(from)).rev().find(|at| haystack[*at..].starts_with(needle))
    } else if needle.is_empty() {
        (from <= room).then_some(from)
    } else if from > room {
        None
    } else if needle.len() == 1 {
        haystack[from..].iter().position(|byte| *byte == needle[0]).map(|at| at + from)
    } else {
        haystack[from..]
            .windows(needle.len())
            .position(|window| window == needle)
            .map(|at| at + from)
    };
    Ok(Value::number(found.map_or(-1.0, |at| at as f64)))
}

pub(super) const NATIVES: [(&str, u8, crate::object::NativeFn); 4] = [
    ("bufferFrom", 2, buffer_from),
    ("bufferText", 2, buffer_text),
    ("bufferCompare", 2, buffer_compare),
    ("bufferIndexOf", 4, buffer_index_of),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn text(text: &str) -> Text {
        Text::from_str(text)
    }

    #[test]
    fn text_becomes_bytes_as_node_js_buffer_did() {
        assert_eq!(encode(&text("héllo"), 0), "héllo".as_bytes());
        assert_eq!(encode(&text("6869a"), 1), b"hi");
        assert_eq!(encode(&text("68x9"), 1), b"");
        assert_eq!(encode(&text("aGk"), 2), b"hi");
        assert_eq!(encode(&text("-_=="), 2), [0xfb]);
        assert_eq!(encode(&text("-_8"), 2), [0xfb, 0xff]);
        assert_eq!(encode(&text("é😀"), 3), [0xe9, 0x3d]);
        assert_eq!(encode(&text("hi"), 4), b"h\0i\0");
    }

    #[test]
    fn bytes_become_text_as_node_js_buffer_did() {
        let shown = |bytes: &[u8], kind| decode(bytes, kind).to_rust();
        assert_eq!(shown("héllo".as_bytes(), 0), "héllo");
        assert_eq!(shown(b"\xef\xbb\xbfok", 0), "ok");
        assert_eq!(shown(b"\x00\xff", 1), "00ff");
        assert_eq!(shown("héllo".as_bytes(), 2), "aMOpbGxv");
        assert_eq!(shown(b"h", 2), "aA==");
        assert_eq!(shown(&[0xfb, 0xff], 3), "-_8");
        assert_eq!(shown(&[0xe9], 4), "é");
        assert_eq!(shown(&[0xe9], 5), "i");
        assert_eq!(shown(b"h\0i\0!", 6), "hi");
    }
}
