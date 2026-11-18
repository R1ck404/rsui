//! What `node/zlib.js` comes down to: DEFLATE (`zlib/deflate.rs` and `zlib/inflate.rs`) inside a zlib or gzip
//! wrapper or neither, Brotli (`zlib/brotli.rs`), and Zstandard (`zlib/zstd.rs`). Each works a piece at a time, through a number standing for it
//! that a stream holds on to, and all at once, here or on another thread. A failure comes back as text,
//! `code:message`, so the error itself is made on the JavaScript side.

mod brotli;
mod brotli_encode;
mod deflate;
mod huffman;
mod inflate;
mod zstd;
mod zstd_encode;

use std::cell::RefCell;

use crate::background::Outcome;
use crate::object::Kind;
use crate::value::Value;
use crate::vm::{Args, Run, Vm};
use deflate::{Deflater, Flush};
use huffman::Stop;
use inflate::Inflater;

/// `bytes` as one Zstandard frame at `level`, with `dictionary` (raw content) as what came before, for the engine's own
/// use, as for the files a packed program keeps (see packed.rs).
pub(crate) fn zstd_frame(bytes: &[u8], level: i32, dictionary: Option<&[u8]>) -> Vec<u8> {
    let mut encoder = zstd::Encoder::new(level, Vec::new(), Some(bytes.len() as u64), dictionary.map(<[u8]>::to_vec));
    let mut out = Vec::new();
    // Writing all of something that's all there, with a dictionary that's only content, can't fail.
    encoder
        .push(bytes, 2, &mut out)
        .expect("a whole frame is written");
    out
}

/// The `size` bytes a Zstandard frame from [`zstd_frame`] holds, with the same `dictionary`, or `None` if that isn't
/// what's there.
pub(crate) fn zstd_unframe(frame: &[u8], size: usize, dictionary: Option<&[u8]>) -> Option<Vec<u8>> {
    let mut decoder = zstd::Decoder::new(dictionary.map(<[u8]>::to_vec), 0).ok()?;
    let mut out = Vec::with_capacity(size);
    decoder.push(frame, &mut out).ok()?;
    (out.len() == size).then_some(out)
}

#[cfg(test)]
mod frame_tests {
    #[test]
    fn a_frame_gives_back_what_went_in() {
        let text = "export default createLucideIcon('StarIcon', ForwardRef);\n".repeat(40);
        for dictionary in [None, Some(b"createLucideIcon ForwardRef export default".as_slice())] {
            let frame = super::zstd_frame(text.as_bytes(), 19, dictionary);
            assert!(frame.len() < text.len() / 4, "{}", frame.len());
            assert_eq!(super::zstd_unframe(&frame, text.len(), dictionary).as_deref(), Some(text.as_bytes()));
        }
        assert_eq!(super::zstd_unframe(b"not a frame", 11, None), None);
        let frame = super::zstd_frame(b"cut short", 3, None);
        assert_eq!(super::zstd_unframe(&frame[..frame.len() - 1], 9, None), None);
        assert_eq!(super::zstd_unframe(&frame, 8, None), None);
    }

    /// How small the files in `RSUI_ZSTD_TRY` (a folder) come out, each alone and with a dictionary from a sample of
    /// them, and how long that takes. Run with `--ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, on a folder that has to be given"]
    fn how_small_files_come_out() {
        let Some(folder) = std::env::var_os("RSUI_ZSTD_TRY") else { return };
        let mut files: Vec<Vec<u8>> = std::fs::read_dir(folder)
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ending| ending == "js"))
            .map(|entry| std::fs::read(entry.path()).unwrap())
            .collect();
        files.sort();
        let raw: usize = files.iter().map(Vec::len).sum();
        let mut sample = Vec::new();
        for file in files.iter().step_by((files.len() / 200).max(1)) {
            sample.extend_from_slice(file);
            if sample.len() > 110 * 1024 {
                break;
            }
        }
        for level in [3, 9, 19] {
            for dictionary in [None, Some(sample.as_slice())] {
                let started = std::time::Instant::now();
                let made: usize = files.iter().map(|file| super::zstd_frame(file, level, dictionary).len()).sum();
                let took = started.elapsed();
                let started = std::time::Instant::now();
                for file in files.iter().take(500) {
                    let frame = super::zstd_frame(file, level, dictionary);
                    assert_eq!(super::zstd_unframe(&frame, file.len(), dictionary).as_deref(), Some(file.as_slice()));
                }
                let _ = started;
                println!(
                    "level {level:2} dictionary {:5}: {} files, {raw} bytes to {made} ({:.1}x, + {} of dictionary), {took:?}",
                    dictionary.is_some(),
                    files.len(),
                    raw as f64 / made as f64,
                    dictionary.map_or(0, <[u8]>::len)
                );
            }
        }
    }
}

/// What's around the DEFLATE data.
#[derive(Clone, Copy)]
pub(super) enum Wrap {
    Raw,
    Zlib,
    Gzip,
    /// Either of the two with a header, whichever it turns out to be, for `unzip`.
    Auto,
}

fn crc_table() -> &'static [[u32; 256]; 16] {
    static TABLE: std::sync::OnceLock<Box<[[u32; 256]; 16]>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = Box::new([[0; 256]; 16]);
        for (n, entry) in table[0].iter_mut().enumerate() {
            let mut c = n as u32;
            for _ in 0..8 {
                c = if c & 1 == 1 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *entry = c;
        }
        // Each of the other tables goes a byte further, so sixteen bytes are done at once.
        for step in 1..16 {
            let (done, rest) = table.split_at_mut(step);
            for (entry, before) in rest[0].iter_mut().zip(done[step - 1]) {
                *entry = (before >> 8) ^ done[0][(before & 255) as usize];
            }
        }
        table
    })
}

pub(super) fn crc32(bytes: &[u8], start: u32) -> u32 {
    #[cfg(target_arch = "x86_64")]
    if bytes.len() >= 128 && folded::usable() {
        return folded::crc32_checked(bytes, start);
    }
    crc32_by_tables(bytes, start)
}

fn crc32_by_tables(bytes: &[u8], start: u32) -> u32 {
    let table = crc_table();
    let mut crc = !start;
    let mut chunks = bytes.chunks_exact(16);
    for chunk in &mut chunks {
        let chunk: &[u8; 16] = chunk.try_into().unwrap_or(&[0; 16]);
        let first = crc ^ u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let mut next = 0;
        for (at, byte) in first.to_le_bytes().into_iter().chain(chunk[4..].iter().copied()).enumerate() {
            next ^= table[15 - at][usize::from(byte)];
        }
        crc = next;
    }
    for byte in chunks.remainder() {
        crc = table[0][((crc ^ u32::from(*byte)) & 255) as usize] ^ (crc >> 8);
    }
    !crc
}

/// CRC-32 sixteen bytes at a time with the processor's carry-less multiply, which every x86-64 made since about 2010
/// has: four lanes of 128 bits are folded forward over 64 bytes at a time and then into one, which is brought down to
/// 32 bits the way Intel's paper on it describes (Gopal et al., "Fast CRC Computation for Generic Polynomials Using
/// PCLMULQDQ"), with the constants every implementation of it for this polynomial uses. It's about five times as fast
/// as the tables, which matters for gunzip, where it would otherwise be a fifth of the time.
#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code)]
mod folded {
    use std::arch::x86_64::{
        __m128i, _mm_and_si128, _mm_clmulepi64_si128, _mm_cvtsi32_si128, _mm_extract_epi32, _mm_loadu_si128, _mm_set_epi32,
        _mm_set_epi64x, _mm_srli_si128, _mm_xor_si128,
    };

    const K1: i64 = 0x1_5444_2bd4;
    const K2: i64 = 0x1_c6e4_1596;
    const K3: i64 = 0x1_7519_97d0;
    const K4: i64 = 0x0_ccaa_009e;
    const K5: i64 = 0x1_63cd_6124;
    const POLYNOMIAL: i64 = 0x1_db71_0641;
    const MU: i64 = 0x1_f701_1641;

    pub(super) fn usable() -> bool {
        std::arch::is_x86_feature_detected!("pclmulqdq") && std::arch::is_x86_feature_detected!("sse4.1")
    }

    pub(super) fn crc32_checked(bytes: &[u8], start: u32) -> u32 {
        debug_assert!(usable());
        // SAFETY: `usable` said the processor has what this is compiled to use.
        unsafe { crc32(bytes, start) }
    }

    #[target_feature(enable = "pclmulqdq,sse4.1")]
    fn load(bytes: &[u8], at: usize) -> __m128i {
        let sixteen: &[u8; 16] = bytes[at..at + 16].try_into().unwrap_or(&[0; 16]);
        // SAFETY: it reads sixteen bytes from an array of sixteen, and unaligned reads are what this one is for.
        unsafe { _mm_loadu_si128(sixteen.as_ptr().cast()) }
    }

    /// `a` moved forward past 128 bits more, by multiplying each half by what that is for it, and added to `b`.
    #[target_feature(enable = "pclmulqdq,sse4.1")]
    fn fold(a: __m128i, b: __m128i, keys: __m128i) -> __m128i {
        _mm_xor_si128(_mm_xor_si128(b, _mm_clmulepi64_si128(a, keys, 0x00)), _mm_clmulepi64_si128(a, keys, 0x11))
    }

    /// For 64 bytes or more.
    #[target_feature(enable = "pclmulqdq,sse4.1")]
    fn crc32(bytes: &[u8], start: u32) -> u32 {
        let (mut x3, mut x2, mut x1, mut x0) = (load(bytes, 0), load(bytes, 16), load(bytes, 32), load(bytes, 48));
        x3 = _mm_xor_si128(x3, _mm_cvtsi32_si128(!start as i32));
        let mut at = 64;
        let k1k2 = _mm_set_epi64x(K2, K1);
        while bytes.len() - at >= 64 {
            x3 = fold(x3, load(bytes, at), k1k2);
            x2 = fold(x2, load(bytes, at + 16), k1k2);
            x1 = fold(x1, load(bytes, at + 32), k1k2);
            x0 = fold(x0, load(bytes, at + 48), k1k2);
            at += 64;
        }
        let k3k4 = _mm_set_epi64x(K4, K3);
        let mut x = fold(fold(fold(x3, x2, k3k4), x1, k3k4), x0, k3k4);
        while bytes.len() - at >= 16 {
            x = fold(x, load(bytes, at), k3k4);
            at += 16;
        }
        // From 128 bits to 64, then Barrett's reduction to 32, for bits that go from the lowest up.
        let low = _mm_set_epi32(0, 0, 0, !0);
        let x = _mm_xor_si128(_mm_clmulepi64_si128(x, k3k4, 0x10), _mm_srli_si128(x, 8));
        let x = _mm_xor_si128(_mm_clmulepi64_si128(_mm_and_si128(x, low), _mm_set_epi64x(0, K5), 0x00), _mm_srli_si128(x, 4));
        let keys = _mm_set_epi64x(MU, POLYNOMIAL);
        let t1 = _mm_clmulepi64_si128(_mm_and_si128(x, low), keys, 0x10);
        let t2 = _mm_clmulepi64_si128(_mm_and_si128(t1, low), keys, 0x00);
        let crc = _mm_extract_epi32(_mm_xor_si128(x, t2), 1) as u32;
        super::crc32_by_tables(&bytes[at..], !crc)
    }
}

/// Adler-32 carried on from what it was for what came before, which starts at 1.
pub(super) fn adler32_from(bytes: &[u8], start: u32) -> u32 {
    let (mut a, mut b) = (u64::from(start & 0xffff), u64::from(start >> 16));
    // Sixteen bytes side by side at a time, in rows: each column's sum, and the sum of those after each row, which
    // is what each byte adds to `b` (one for each byte from it to the end) but for where it is in its row. 256 rows
    // keep both in a u32, and the processor does the sixteen columns at once.
    const ROWS: usize = 256;
    let mut blocks = bytes.chunks_exact(16 * ROWS);
    let columns = |block: &[u8], a: &mut u64, b: &mut u64| {
        let (mut sums, mut totals) = ([0u32; 16], [0u32; 16]);
        for row in block.chunks_exact(16) {
            let row: &[u8; 16] = row.try_into().unwrap_or(&[0; 16]);
            for column in 0..16 {
                sums[column] += u32::from(row[column]);
                totals[column] += sums[column];
            }
        }
        let size = block.len() as u64;
        let (mut sum, mut weighed) = (0u64, 0u64);
        for column in 0..16 {
            sum += u64::from(sums[column]);
            weighed += 16 * u64::from(totals[column]) - column as u64 * u64::from(sums[column]);
        }
        *b = (*b + size * *a + weighed) % 65521;
        *a = (*a + sum) % 65521;
    };
    for block in &mut blocks {
        columns(block, &mut a, &mut b);
    }
    let rest = blocks.remainder();
    let whole = rest.len() / 16 * 16;
    columns(&rest[..whole], &mut a, &mut b);
    for byte in &rest[whole..] {
        a += u64::from(*byte);
        b += a;
    }
    (((b % 65521) << 16) | (a % 65521)) as u32
}

/// Something that turns bytes into others a piece at a time.
enum Codec {
    Deflate(Box<Deflater>),
    Inflate(Box<Inflater>),
    BrotliEncode(Box<brotli::Encoder>),
    BrotliDecode(Box<brotli::Decoder>),
    ZstdEncode(Box<zstd::Encoder>),
    ZstdDecode(Box<zstd::Decoder>),
}

/// How to make a codec: which of the eleven kinds, by its number in `node/zlib.js`, and with what. For Zstandard,
/// the size hint is the size it's said the input will be, plus one, and the rest is in `params`, as zstd's own
/// parameters by number.
#[derive(Clone)]
struct Recipe {
    kind: u32,
    level: i32,
    window_bits: u32,
    memory: u32,
    strategy: u8,
    dictionary: Option<Vec<u8>>,
    size_hint: usize,
    params: Vec<(u32, i32)>,
}

impl Recipe {
    fn make(&self) -> Result<Codec, String> {
        let deflate = |wrap| {
            let window_bits = if self.window_bits == 0 { 15 } else { self.window_bits };
            Codec::Deflate(Box::new(Deflater::new(wrap, self.level, window_bits, self.memory, self.strategy, self.dictionary.clone())))
        };
        let inflate = |wrap| Codec::Inflate(Box::new(Inflater::new(wrap, self.dictionary.clone())));
        Ok(match self.kind {
            0 => deflate(Wrap::Zlib),
            1 => inflate(Wrap::Zlib),
            2 => deflate(Wrap::Gzip),
            3 => inflate(Wrap::Gzip),
            4 => deflate(Wrap::Raw),
            5 => inflate(Wrap::Raw),
            6 => inflate(Wrap::Auto),
            7 => Codec::BrotliEncode(Box::new(brotli::Encoder::new(self.level, self.window_bits, self.memory, self.size_hint))),
            8 => Codec::BrotliDecode(Box::new(brotli::Decoder::new())),
            9 => {
                let pledged = self.size_hint.checked_sub(1).map(|size| size as u64);
                Codec::ZstdEncode(Box::new(zstd::Encoder::new(3, self.params.clone(), pledged, self.dictionary.clone())))
            }
            _ => {
                let window_log = self.params.iter().rev().find(|(key, _)| *key == 100).map_or(0, |(_, value)| *value as u32);
                let decoder = zstd::Decoder::new(self.dictionary.clone(), window_log)
                    .map_err(|_| "ERR_ZLIB_INITIALIZATION_FAILED:Failed to load zstd dictionary".to_owned())?;
                Codec::ZstdDecode(Box::new(decoder))
            }
        })
    }
}

impl Codec {
    /// Takes some bytes, giving what comes of them. `flush` is a zlib flush for DEFLATE, a Brotli operation for
    /// Brotli, and an end directive for Zstandard.
    fn push(&mut self, input: &[u8], flush: u32, out: &mut Vec<u8>) -> Result<(), Stop> {
        match self {
            Self::Deflate(deflater) => {
                let flush = match flush {
                    0 => Flush::None,
                    1 | 5 => Flush::Partial,
                    2 => Flush::Sync,
                    3 => Flush::Full,
                    _ => Flush::Finish,
                };
                deflater.push(input, flush, out);
                Ok(())
            }
            Self::Inflate(inflater) => inflater.push(input, flush == 4, out),
            Self::BrotliEncode(encoder) => {
                encoder.push(input, flush, out);
                Ok(())
            }
            Self::BrotliDecode(decoder) => decoder.push(input, flush == 2, out),
            Self::ZstdEncode(encoder) => encoder.push(input, flush, out),
            Self::ZstdDecode(decoder) => decoder.push(input, out),
        }
    }
}

thread_local! {
    /// The codecs streams are using, by number. Each engine is on a thread of its own, so this is its own too.
    static CODECS: RefCell<Vec<Option<(Codec, Recipe)>>> = const { RefCell::new(Vec::new()) };
}

/// Runs `with` on the bytes a typed array (or an `ArrayBuffer`) sees, without copying them.
fn with_bytes<T>(vm: &mut Vm, value: Value, with: impl FnOnce(&[u8]) -> T) -> Run<T> {
    if value.is_object() {
        let (buffer, offset, length) = match &vm.objects[value.index()].kind {
            Kind::ArrayBuffer(data) => return Ok(with(&data.bytes)),
            Kind::TypedArray(data) => (data.buffer, data.offset, vm.typed_length(data) * data.element.size()),
            _ => return Err(vm.type_error("Expected an ArrayBuffer or a view of one")),
        };
        if let Kind::ArrayBuffer(store) = &vm.objects[buffer.index()].kind {
            return Ok(with(&store.bytes[offset..offset + length]));
        }
    }
    Err(vm.type_error("Expected an ArrayBuffer or a view of one"))
}

fn number(vm: &mut Vm, args: &Args, index: usize, otherwise: f64) -> Run<f64> {
    let value = vm.arg(args, index);
    if value.is_undefined() {
        return Ok(otherwise);
    }
    let found = vm.to_number(value)?;
    Ok(if found.is_nan() { otherwise } else { found })
}

/// The recipe from `(kind, level, windowBits, memLevel, strategy, dictionary, sizeHint, params)`, starting at
/// `from`, where `params` is a typed array of keys each followed by its value.
fn recipe(vm: &mut Vm, args: &Args, from: usize) -> Run<Recipe> {
    let kind = number(vm, args, from, 0.0)? as u32;
    let level = number(vm, args, from + 1, -1.0)? as i32;
    let window_bits = number(vm, args, from + 2, 0.0)? as u32;
    let memory = number(vm, args, from + 3, 8.0)? as u32;
    let strategy = number(vm, args, from + 4, 0.0)? as u8;
    let given = vm.arg(args, from + 5);
    let dictionary = if given.is_object() { Some(with_bytes(vm, given, <[u8]>::to_vec)?) } else { None };
    let size_hint = number(vm, args, from + 6, 0.0)? as usize;
    let given = vm.arg(args, from + 7);
    let mut params = Vec::new();
    if given.is_object() {
        let numbers = with_bytes(vm, given, |bytes| bytes.chunks_exact(4).map(|four| i32::from_le_bytes([four[0], four[1], four[2], four[3]])).collect::<Vec<_>>())?;
        params = numbers.chunks_exact(2).map(|pair| (pair[0] as u32, pair[1])).collect();
    }
    Ok(Recipe { kind, level, window_bits, memory, strategy, dictionary, size_hint, params })
}

fn failure_text(stop: &Stop) -> String {
    match stop {
        Stop::Short => "Z_BUF_ERROR:unexpected end of file".to_owned(),
        Stop::Bad(code, message) => format!("{code}:{message}"),
    }
}

/// All of something at once, with the last flush as `finish`.
fn all_at_once(recipe: &Recipe, input: &[u8], finish: u32) -> Result<Vec<u8>, String> {
    let mut recipe = recipe.clone();
    if recipe.size_hint == 0 && recipe.kind == 7 {
        // Knowing how much there is lets Brotli keep no more than that. Zstandard knows from being given it all at once.
        recipe.size_hint = input.len().max(1);
    }
    let mut codec = recipe.make()?;
    // What undoes hands over what it made, so only what squeezes needs room made first.
    let squeezes = matches!(codec, Codec::Deflate(_) | Codec::BrotliEncode(_) | Codec::ZstdEncode(_));
    let mut out = if squeezes { Vec::with_capacity(input.len() / 2 + 64) } else { Vec::new() };
    codec.push(input, finish, &mut out).map_err(|stop| failure_text(&stop))?;
    // The engine counts what it's handed by its room, which was guessed at, so a lot more than was used goes back.
    if out.capacity() - out.len() > out.len() / 8 + 4096 {
        out.shrink_to_fit();
    }
    Ok(out)
}

/// `zlib(bytes, finish, kind, level, windowBits, memLevel, strategy, dictionary, sizeHint, params)`: an `ArrayBuffer` of
/// what comes out, or text saying why it couldn't be done.
fn zlib(vm: &mut Vm, args: Args) -> Run<Value> {
    let recipe = recipe(vm, &args, 2)?;
    let finish = number(vm, &args, 1, 4.0)? as u32;
    let input = vm.arg(&args, 0);
    match with_bytes(vm, input, |bytes| all_at_once(&recipe, bytes, finish))? {
        Ok(bytes) => Ok(vm.new_array_buffer(bytes)),
        Err(text) => Ok(vm.new_str(&text)),
    }
}

/// The same on another thread, as a promise for the bytes, or rejected with `code:message`.
fn zlib_later(vm: &mut Vm, args: Args) -> Run<Value> {
    let recipe = recipe(vm, &args, 2)?;
    let finish = number(vm, &args, 1, 4.0)? as u32;
    let input = vm.arg(&args, 0);
    let input = with_bytes(vm, input, <[u8]>::to_vec)?;
    Ok(vm.in_background(move || match all_at_once(&recipe, &input, finish) {
        Ok(bytes) => Outcome::Bytes(bytes),
        Err(text) => Outcome::Failed(text),
    }))
}

/// `zlibOpen(kind, level, windowBits, memLevel, strategy, dictionary, sizeHint, params)`: the number of a new codec,
/// or text saying why it couldn't be made.
fn zlib_open(vm: &mut Vm, args: Args) -> Run<Value> {
    let recipe = recipe(vm, &args, 0)?;
    let codec = match recipe.make() {
        Ok(codec) => codec,
        Err(text) => return Ok(vm.new_str(&text)),
    };
    let id = CODECS.with_borrow_mut(|codecs| {
        let slot = codecs.iter().position(Option::is_none).unwrap_or_else(|| {
            codecs.push(None);
            codecs.len() - 1
        });
        codecs[slot] = Some((codec, recipe));
        slot
    });
    Ok(Value::number(id as f64))
}

fn codec_id(vm: &mut Vm, args: &Args) -> Run<usize> {
    Ok(number(vm, args, 0, -1.0)? as usize)
}

/// `zlibPush(id, bytes, flush)`: an `ArrayBuffer` of what came of them, or text saying why it couldn't be done.
fn zlib_push(vm: &mut Vm, args: Args) -> Run<Value> {
    let id = codec_id(vm, &args)?;
    let flush = number(vm, &args, 2, 0.0)? as u32;
    let input = vm.arg(&args, 1);
    let went = with_bytes(vm, input, |bytes| {
        CODECS.with_borrow_mut(|codecs| {
            let Some(Some((codec, _))) = codecs.get_mut(id) else {
                return Err("ERR_ZLIB_CLOSED:zlib binding closed".to_owned());
            };
            let mut out = Vec::new();
            codec.push(bytes, flush, &mut out).map(|()| out).map_err(|stop| failure_text(&stop))
        })
    })?;
    match went {
        Ok(bytes) => Ok(vm.new_array_buffer(bytes)),
        Err(text) => Ok(vm.new_str(&text)),
    }
}

/// `zlibParams(id, level, strategy)`, for `params()`.
fn zlib_params(vm: &mut Vm, args: Args) -> Run<Value> {
    let id = codec_id(vm, &args)?;
    let level = number(vm, &args, 1, -1.0)? as i32;
    let strategy = number(vm, &args, 2, 0.0)? as u8;
    CODECS.with_borrow_mut(|codecs| {
        if let Some(Some((Codec::Deflate(deflater), recipe))) = codecs.get_mut(id) {
            deflater.set_params(level, strategy);
            recipe.level = level;
            recipe.strategy = strategy;
        }
    });
    Ok(Value::UNDEFINED)
}

/// `zlibReset(id)`, which starts the codec again as it was made.
fn zlib_reset(vm: &mut Vm, args: Args) -> Run<Value> {
    let id = codec_id(vm, &args)?;
    CODECS.with_borrow_mut(|codecs| {
        if let Some(Some((codec, recipe))) = codecs.get_mut(id) {
            if let Codec::Inflate(inflater) = codec {
                inflater.reset();
            } else if let Ok(made) = recipe.make() {
                *codec = made;
            }
        }
    });
    Ok(Value::UNDEFINED)
}

/// `zlibClose(id)`, after which the number can be another codec's.
fn zlib_close(vm: &mut Vm, args: Args) -> Run<Value> {
    let id = codec_id(vm, &args)?;
    CODECS.with_borrow_mut(|codecs| {
        if let Some(slot) = codecs.get_mut(id) {
            *slot = None;
        }
    });
    Ok(Value::UNDEFINED)
}

/// `zlibEnded(id)`: whether what's being undone has come to its end, after which more is ignored.
fn zlib_ended(vm: &mut Vm, args: Args) -> Run<Value> {
    let id = codec_id(vm, &args)?;
    let ended = CODECS.with_borrow(|codecs| match codecs.get(id) {
        Some(Some((Codec::Inflate(inflater), _))) => inflater.ended(),
        Some(Some((Codec::BrotliDecode(decoder), _))) => decoder.ended(),
        Some(Some((Codec::ZstdDecode(decoder), _))) => decoder.ended(),
        _ => false,
    });
    Ok(Value::bool(ended))
}

/// `crc32(bytes, start)`, where `start` is already a 32-bit number.
fn crc32_native(vm: &mut Vm, args: Args) -> Run<Value> {
    let input = vm.arg(&args, 0);
    let start = vm.arg(&args, 1);
    let start = vm.to_int32(start)? as u32;
    let sum = with_bytes(vm, input, |bytes| crc32(bytes, start))?;
    Ok(Value::number(f64::from(sum)))
}

pub(super) const NATIVES: [(&str, u8, crate::object::NativeFn); 9] = [
    ("zlib", 10, zlib),
    ("zlibLater", 10, zlib_later),
    ("zlibOpen", 8, zlib_open),
    ("zlibPush", 3, zlib_push),
    ("zlibParams", 3, zlib_params),
    ("zlibReset", 1, zlib_reset),
    ("zlibClose", 1, zlib_close),
    ("zlibEnded", 1, zlib_ended),
    ("crc32", 2, crc32_native),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe(kind: u32, level: i32) -> Recipe {
        Recipe { kind, level, window_bits: 0, memory: 8, strategy: 0, dictionary: None, size_hint: 0, params: Vec::new() }
    }

    /// Text, numbers and repeats, like the files that get squeezed.
    pub(super) fn sample(size: usize) -> Vec<u8> {
        let words = ["the ", "quick ", "brown ", "fox ", "jumps ", "over ", "lazy ", "dog ", "{\"id\": ", "}, ", "\n"];
        let mut out = Vec::with_capacity(size);
        let mut seed = 12345u32;
        while out.len() < size {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            out.extend_from_slice(words[(seed >> 16) as usize % words.len()].as_bytes());
            if seed % 7 == 0 {
                out.extend_from_slice((seed % 1000).to_string().as_bytes());
            }
        }
        out.truncate(size);
        out
    }

    fn random(size: usize) -> Vec<u8> {
        let mut seed = 99u64;
        (0..size)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect()
    }

    /// Runs a program from a file of its own, as it needs `require`, and gives what it logged.
    pub(super) fn run(name: &str, source: &str) -> String {
        let file = std::env::temp_dir().join(format!("rsui_js_zlib_{name}_{}.js", std::process::id()));
        std::fs::write(&file, source).unwrap();
        let output = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let mut engine = crate::Engine::new();
        let sink = output.clone();
        engine.set_printer(move |text| sink.borrow_mut().push_str(text));
        let result = engine.run_file(&file).map(|_| ()).and_then(|()| engine.run_until_idle());
        drop(engine);
        std::fs::remove_file(&file).ok();
        assert_eq!(result.map_err(|error| error.message), Ok(()));
        let shown = output.borrow().replace("\r\n", "\n");
        shown.trim_end().to_owned()
    }

    #[test]
    fn streams_give_out_what_they_can_as_it_comes() {
        let shown = run("streams", r#"
            const zlib = require('zlib');
            const text = 'streams of text, '.repeat(5000);
            const gzip = zlib.createGzip();
            const parts = [];
            gzip.on('data', (part) => parts.push(part));
            gzip.write(text.slice(0, 40000));
            gzip.flush(() => {
                // A flush gives out all that's been written, so it can be read before the end.
                const sofar = zlib.gunzipSync(Buffer.concat(parts), { finishFlush: zlib.constants.Z_SYNC_FLUSH });
                console.log(parts.length > 0, String(sofar) === text.slice(0, 40000));
                gzip.end(text.slice(40000));
            });
            gzip.on('end', () => {
                const whole = Buffer.concat(parts);
                const back = [];
                const gunzip = zlib.createGunzip({ chunkSize: 1024 });
                gunzip.on('data', (part) => back.push(part.length));
                gunzip.on('end', () => console.log(back.every((size) => size <= 1024), back.reduce((a, b) => a + b) === text.length, gunzip.bytesWritten === whole.length));
                for (let at = 0; at < whole.length; at += 100) gunzip.write(whole.subarray(at, at + 100));
                gunzip.end();
            });
            const bad = zlib.createInflate();
            bad.on('error', (error) => console.log(error.message, error.code, error.errno));
            bad.end(Buffer.from('not zlib at all'));
            zlib.deflate('hello', (error, packed) => zlib.inflate(packed, (error, back) => console.log('callback', String(back))));
            zlib.gunzip(Buffer.from('nope'), (error) => console.log('callback', error.code));
            const dict = Buffer.from('a dictionary of words');
            console.log(String(zlib.inflateSync(zlib.deflateSync('words of a dictionary', { dictionary: dict }), { dictionary: dict })));
            try { zlib.inflateSync(zlib.deflateSync('x', { dictionary: dict })); } catch (error) { console.log(error.message, error.errno); }
            try { zlib.deflateSync('x', { level: 12 }); } catch (error) { console.log(error.code, error.name); }
        "#);
        // The streams run here, in an order of their own, but `zlib.deflate` and `zlib.gunzip` are done on other threads,
        // and when those come back depends on how busy the machine is, as with Node's thread pool.
        let (later, here): (Vec<&str>, Vec<&str>) = shown.lines().partition(|line| line.starts_with("callback "));
        assert_eq!(
            here.join("\n"),
            "words of a dictionary\nMissing dictionary 2\nERR_OUT_OF_RANGE RangeError\ntrue true\n\
             incorrect header check Z_DATA_ERROR -3\ntrue true true"
        );
        let mut later = later;
        later.sort_unstable();
        assert_eq!(later, ["callback Z_DATA_ERROR", "callback hello"], "{shown}");
    }

    #[test]
    fn what_is_written_comes_back_at_every_level_and_in_any_pieces() {
        // Small ones are written with the fixed codes, where every byte from 144 up has a code of nine bits.
        let every_byte: Vec<u8> = (0..=255).collect();
        let inputs = [
            Vec::new(),
            b"a".to_vec(),
            "hello hello hello, \u{e9}".as_bytes().to_vec(),
            every_byte,
            sample(300_000),
            random(70_000),
            vec![7u8; 100_000],
        ];
        // What zlib itself writes for an "\u{e9}", with those codes.
        assert_eq!(all_at_once(&recipe(4, 6), &[0xc3, 0xa9], 4).unwrap(), [0x3b, 0xbc, 0x12, 0x00]);
        for input in &inputs {
            for level in [0, 1, 3, 4, 6, 9] {
                for (pack, unpack) in [(0, 1), (2, 3), (4, 5), (0, 6), (2, 6)] {
                    let packed = all_at_once(&recipe(pack, level), input, 4).unwrap();
                    let unpacked = all_at_once(&recipe(unpack, -1), &packed, 4);
                    assert_eq!(unpacked.as_ref(), Ok(input), "level {level} kind {pack} size {}", input.len());
                }
            }
        }
        // In pieces of all sizes, with flushes between some.
        let input = sample(200_000);
        let mut packer = recipe(2, 6).make().unwrap();
        let mut packed = Vec::new();
        for (n, piece) in input.chunks(7777).enumerate() {
            packer.push(piece, [0, 0, 2, 3][n % 4], &mut packed).unwrap();
        }
        packer.push(&[], 4, &mut packed).unwrap();
        let mut unpacker = recipe(3, -1).make().unwrap();
        let mut unpacked = Vec::new();
        for piece in packed.chunks(13) {
            unpacker.push(piece, 0, &mut unpacked).unwrap();
        }
        unpacker.push(&[], 4, &mut unpacked).unwrap();
        assert_eq!(unpacked, input);
    }

    #[test]
    fn text_is_squeezed_about_as_well_as_zlib_does() {
        let input = sample(1_000_000);
        let packed = all_at_once(&recipe(0, 6), &input, 4).unwrap();
        // zlib 1.3 makes 322 KB of this at level 6.
        assert!(packed.len() < 340_000, "{}", packed.len());
        assert!(all_at_once(&recipe(0, 1), &input, 4).unwrap().len() < 420_000);
    }

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len()).step_by(2).map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap()).collect()
    }

    #[test]
    fn brotlis_dictionary_is_the_one_in_the_rfc() {
        let words = brotli::dictionary_for_test();
        assert_eq!(words.len(), 122_784);
        let sum: String = crate::builtins::system::sha256(words).iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(sum, "20e42eb1b511c21806d4d227d07e5dd06877d8ce7b3a817f378f313653f35c70");
    }

    #[test]
    fn what_node_writes_in_brotli_is_read_even_a_byte_at_a_time() {
        let text = "The quick brown fox jumps over the lazy dog. This is a test of the dictionary, with the words in it, \
                    and THE WORLD of transforms: Information, Government, international.";
        // Node's own Brotli, at its quality 11, which uses the dictionary and its transforms, and at 5.
        let made = [
            "1ba800a08cd30d777274122250e5481b5f169ff0c17f9fbcdfccc2c8eac9420f1f369120da14a5121a6e63eb69a74248249f331b42\
             4bac3c254401e0acaacd6d3c04dd2a751485d2842e090b029a18f210cd14945f739d3975a5cd590d",
            "1bfa010004049953d9c62f8b9f66d200f4ebe480bd2f4c033970141460015b0090def0b996ac1197c0b8abf1794b71acef42b37e0c\
             f7bc9dac4f7dfcc058a7fcffa35a5b314962ce559f17f5ac89a25d5300f0301ef26ba7a7e65857c8bcd0964d79e4cb396db34e29444\
             3402917587706f019f249240b",
        ];
        for (made, times) in made.iter().zip([1, 3]) {
            let bytes = hex(made);
            let want = text.repeat(times).into_bytes();
            assert_eq!(all_at_once(&recipe(8, -1), &bytes, 2).unwrap(), want);
            let mut decoder = recipe(8, -1).make().unwrap();
            let mut out = Vec::new();
            for byte in &bytes {
                decoder.push(std::slice::from_ref(byte), 0, &mut out).unwrap();
            }
            decoder.push(&[], 2, &mut out).unwrap();
            assert_eq!(out, want);
            assert_eq!(all_at_once(&recipe(8, -1), &bytes[..bytes.len() - 3], 2).unwrap_err(), "Z_BUF_ERROR:unexpected end of file");
        }
        assert!(all_at_once(&recipe(8, -1), b"nonsense", 2).unwrap_err().starts_with("ERR__ERROR_FORMAT_"));
        // Nothing at all, as Node writes it with its usual window.
        assert_eq!(all_at_once(&recipe(7, 11), b"", 2).unwrap(), [0x3b]);
    }

    #[test]
    fn brotli_comes_back_at_every_quality_and_in_any_pieces() {
        let inputs = [Vec::new(), b"a".to_vec(), sample(400_000), random(70_000), vec![7u8; 100_000]];
        for input in &inputs {
            for quality in [0, 1, 2, 4, 5, 6, 9, 11] {
                let packed = all_at_once(&recipe(7, quality), input, 2).unwrap();
                let unpacked = all_at_once(&recipe(8, -1), &packed, 2);
                assert_eq!(unpacked.as_ref(), Ok(input), "quality {quality} size {}", input.len());
            }
        }
        let input = sample(300_000);
        let mut packer = Recipe { window_bits: 16, ..recipe(7, 5) }.make().unwrap();
        let mut packed = Vec::new();
        for (n, piece) in input.chunks(5555).enumerate() {
            packer.push(piece, [0, 0, 1][n % 3], &mut packed).unwrap();
        }
        packer.push(&[], 2, &mut packed).unwrap();
        let mut unpacker = recipe(8, -1).make().unwrap();
        let mut unpacked = Vec::new();
        for piece in packed.chunks(7) {
            unpacker.push(piece, 0, &mut unpacked).unwrap();
        }
        unpacker.push(&[], 2, &mut unpacked).unwrap();
        assert_eq!(unpacked, input);
        // It squeezes, more than DEFLATE does.
        let deflated = all_at_once(&recipe(0, 6), &input, 4).unwrap().len();
        let squeezed = all_at_once(&recipe(7, 11), &input, 2).unwrap().len();
        assert!(squeezed < deflated, "{squeezed} {deflated}");
    }

    fn zstd(level: i32, checksum: bool) -> Recipe {
        Recipe { params: vec![(100, level), (201, i32::from(checksum))], ..recipe(9, 0) }
    }

    /// Reads a frame a byte at a time, as a stream that gets it in the smallest pieces there are would.
    fn zstd_bytewise(recipe: &Recipe, packed: &[u8]) -> Vec<u8> {
        let mut unpacker = recipe.make().unwrap();
        let mut out = Vec::new();
        for byte in packed {
            unpacker.push(std::slice::from_ref(byte), 0, &mut out).unwrap();
        }
        out
    }

    #[test]
    fn zstd_comes_back_at_every_level_and_in_any_pieces() {
        let every_byte: Vec<u8> = (0..=255).collect();
        let inputs = [Vec::new(), b"a".to_vec(), every_byte, sample(300_000), random(70_000), vec![7u8; 100_000]];
        for input in &inputs {
            for level in [-5, 1, 3, 5, 9, 13, 16, 19] {
                let packed = all_at_once(&zstd(level, level % 2 != 0), input, 2).unwrap();
                let unpacked = all_at_once(&recipe(10, 0), &packed, 2);
                assert_eq!(unpacked.as_ref(), Ok(input), "level {level} size {}", input.len());
            }
        }
        // In pieces, with flushes between some, read back a byte at a time.
        let input = sample(200_000);
        let mut packer = zstd(3, true).make().unwrap();
        let mut packed = Vec::new();
        for (n, piece) in input.chunks(7777).enumerate() {
            packer.push(piece, [0, 0, 1][n % 3], &mut packed).unwrap();
        }
        packer.push(&[], 2, &mut packed).unwrap();
        let mut unpacker = recipe(10, 0).make().unwrap();
        let mut unpacked = Vec::new();
        for byte in &packed {
            unpacker.push(std::slice::from_ref(byte), 0, &mut unpacked).unwrap();
        }
        assert_eq!(unpacked, input);
        // Another frame in a piece of its own is read too, but one after the first in the same piece is the end,
        // as in Node.
        unpacker.push(&packed, 0, &mut unpacked).unwrap();
        assert_eq!(unpacked.len(), 2 * input.len());
        assert_eq!(all_at_once(&recipe(10, 0), &[&packed[..], &packed[..]].concat(), 2).unwrap(), input);
        // Skippable frames are skipped.
        let skipped = [&[0x5a, 0x2a, 0x4d, 0x18, 3, 0, 0, 0, 1, 2, 3][..], &packed].concat();
        assert_eq!(zstd_bytewise(&recipe(10, 0), &skipped), input);
    }

    #[test]
    fn small_inputs_of_every_byte_come_back() {
        // A deflate shipped with a bug only small inputs with high bytes hit, so there are many of them, of all kinds.
        let mut seed = 1u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for n in 0..2000 {
            let size = (next() % 301) as usize;
            let kind = next() % 4;
            let pattern: Vec<u8> = (0..1 + next() % 8).map(|_| next() as u8).collect();
            let input: Vec<u8> = (0..size)
                .map(|at| match kind {
                    0 => next() as u8,
                    1 => 128 | next() as u8,
                    2 => pattern[at % pattern.len()],
                    _ => (next() % 3) as u8 * 100 + 55,
                })
                .collect();
            let level = [-3, 1, 2, 3, 5, 9, 13, 19][n % 8];
            let packed = all_at_once(&zstd(level, n % 3 == 0), &input, 2).unwrap();
            assert_eq!(all_at_once(&recipe(10, 0), &packed, 2).unwrap(), input, "{n}: level {level} size {size}");
            if n % 10 == 0 {
                assert_eq!(zstd_bytewise(&recipe(10, 0), &packed), input);
            }
        }
    }

    #[test]
    fn what_node_writes_in_zstd_is_read_even_a_byte_at_a_time() {
        // Node's own zstd, at level 19 and 1, with the frame's sum: literals in Huffman codes, and sequences in
        // codes made for them.
        let made = [
            ("28b52ffd64bc011d080032c7181970790340a943ad1794fe30ffff0552e4baba6b9a2efd5501041d8cf2a7bfcbec419db5568f3c25f560524a6164e8fe91fdf9dd97df1482d97765fd7eb7527ff705da5166a9e25d29b46330d7d495045d4d6b3669c51e3d000703748d2c16f7c6114f53a8d10ea582c2ec8c0110108220b33a1030aa2610195a031d603587f17b98d6ec3eb61551ee8af3cdd6095db2426c135940bf5b0656100ee3ef2604b5137a8b50f22b1bc4f25c99deff49b220626628304201e35cc616299fd7a091d991ce89fb4634695ef8c44aae1c3d420e941dea2db5db199a5c770fad9124058d5a0898213a7c3387019a0c65afa307e9d17de93a237e9e17d5f0778f6a5ef1037fff7ee5", 700),
            ("28b52ffd642c007d05006247191c60a9731882ca7b1697a867b8b4dc0804415ca4bb97e28014c10dab048d4af49dc53b15ef729dab4bfdb4c47317a2cef53b438ec4ef4c3508e5e6e6aa72b8a0f813c88d776e105a8a0702263a186120448de39ddf990106638c851b842c4d8ba32c4d8b131d0054e5b95960e7005c8744cc9d9e013a2e40330158d81930e00ca40ead3183532a996703980eb6c230540654c74822f11a0d77e1e198e2c81cd8550e35358372ade704ef9a636b5a3bad", 300),
        ];
        for (made, size) in made {
            let bytes = hex(made);
            assert_eq!(all_at_once(&recipe(10, 0), &bytes, 2).unwrap(), sample(size));
            assert_eq!(zstd_bytewise(&recipe(10, 0), &bytes), sample(size));
            // What's cut short is no error in Node, and what's whole of it comes out.
            assert!(all_at_once(&recipe(10, 0), &bytes[..bytes.len() - 5], 2).unwrap().is_empty());
            let mut wrong = bytes.clone();
            *wrong.last_mut().unwrap() ^= 1;
            assert_eq!(all_at_once(&recipe(10, 0), &wrong, 2).unwrap_err(), "ZSTD_error_checksum_wrong:Restored data doesn't match checksum");
        }
        let fails = |bytes: &[u8], recipe: &Recipe| all_at_once(recipe, bytes, 2).unwrap_err();
        assert_eq!(fails(b"nonsense", &recipe(10, 0)), "ZSTD_error_prefix_unknown:Unknown frame descriptor");
        assert_eq!(fails(&hex("28b52ffd2800010000"), &recipe(10, 0)), "ZSTD_error_frameParameter_unsupported:Unsupported frame parameter");
        assert_eq!(fails(&hex("28b52ffd2001070000"), &recipe(10, 0)), "ZSTD_error_corruption_detected:Data corruption detected");
        assert_eq!(fails(&hex("28b52ffd2101020304010000"), &recipe(10, 0)), "ZSTD_error_dictionary_wrong:Dictionary mismatch");
        let big = all_at_once(&zstd(1, false), &vec![0; 1 << 20], 2).unwrap();
        let small_window = Recipe { params: vec![(100, 10)], ..recipe(10, 0) };
        assert_eq!(fails(&big, &small_window), "ZSTD_error_frameParameter_windowTooLarge:Frame requires too much memory for decoding");
        // These are the bytes Node writes for nothing at all, and for what can't be squeezed.
        assert_eq!(all_at_once(&zstd(3, false), b"", 2).unwrap(), hex("28b52ffd2000010000"));
        assert_eq!(all_at_once(&zstd(3, false), b"hello", 2).unwrap(), hex("28b52ffd200529000068656c6c6f"));
        assert_eq!(all_at_once(&zstd(3, true), b"hello", 2).unwrap(), hex("28b52ffd240529000068656c6c6fa36d9f88"));
    }

    #[test]
    fn zstd_squeezes_about_as_well_as_zstd_does() {
        let input = sample(1_000_000);
        // Node's zstd makes 247,616 bytes of this at level 3, and 192,711 at level 19.
        let three = all_at_once(&zstd(3, false), &input, 2).unwrap().len();
        assert!(three < 260_000, "{three}");
        let nineteen = all_at_once(&zstd(19, false), &input[..300_000], 2).unwrap().len();
        assert!(nineteen < all_at_once(&zstd(3, false), &input[..300_000], 2).unwrap().len() * 9 / 10, "{nineteen}");
    }

    #[test]
    fn zstd_dictionaries_are_what_came_before() {
        let content = sample(2000);
        // Node's frame for part of what's in a dictionary that's all content.
        let made = hex("28b52ffd60f400450000000100f1dfa91908");
        let raw = Recipe { dictionary: Some(content.clone()), ..recipe(10, 0) };
        assert_eq!(all_at_once(&raw, &made, 2).unwrap(), &sample(1000)[500..]);
        assert!(all_at_once(&recipe(10, 0), &made, 2).is_err());
        // One with tables of its own, which Node's frames start with, even its Huffman code with nothing before it.
        let tables = zstd_encode::dictionary_for_test(&content);
        let with_tables = Recipe { dictionary: Some(tables.clone()), ..recipe(10, 0) };
        let made = hex("28b52ffd66611ea0010d030072071411b0b90122a48a1d492142ba0813014c0944e6c1564a63b7534c8f8d47b5c3cc7bbdcfbabc2427af1629f66f9557e2f77ec8079d065f4e124e946757abb109f25972d0a8f64ebe5ab3775d8cf28a6a45030028470155a8ff48d7f4066c06714ab6d8");
        let text = [&b"Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. ".repeat(3)[..], &sample(300)].concat();
        assert_eq!(all_at_once(&with_tables, &made, 2).unwrap(), text);
        assert_eq!(zstd_bytewise(&with_tables, &made), text);
        for dictionary in [content, tables] {
            for level in [1, 3, 19] {
                let packer = Recipe { dictionary: Some(dictionary.clone()), ..zstd(level, true) };
                let packed = all_at_once(&packer, &text, 2).unwrap();
                let unpacker = Recipe { dictionary: Some(dictionary.clone()), ..recipe(10, 0) };
                assert_eq!(all_at_once(&unpacker, &packed, 2).unwrap(), text);
            }
        }
        // A dictionary with tables that are wrong can't be used, and says so the way Node does.
        let broken = zstd_encode::dictionary_for_test(b"")[..20].to_vec();
        assert_eq!(Recipe { dictionary: Some(broken.clone()), ..recipe(10, 0) }.make().err().unwrap(), "ERR_ZLIB_INITIALIZATION_FAILED:Failed to load zstd dictionary");
        let packer = Recipe { dictionary: Some(broken), ..zstd(3, false) };
        assert_eq!(all_at_once(&packer, b"x", 2).unwrap_err(), "ZSTD_error_memory_allocation:Allocation error : not enough memory");
    }

    #[test]
    fn zstd_in_javascript_is_as_node_has_it() {
        let shown = run("zstd", r#"
            const zlib = require('zlib');
            const { constants } = zlib;
            console.log(Object.keys(constants).filter((name) => name.startsWith('ZSTD')).length, constants.ZSTD_c_checksumFlag, constants.ZSTD_error_srcSize_wrong, constants.ZSTD_btultra2);
            const text = 'zstd, as Node has it. '.repeat(400);
            console.log(String(zlib.zstdDecompressSync(zlib.zstdCompressSync(text, { params: { [constants.ZSTD_c_compressionLevel]: 19 } }))) === text);
            const attempt = (what) => { try { what(); } catch (error) { console.log(error.name, error.code, error.errno, error.message); } };
            attempt(() => zlib.zstdCompressSync('x', { params: { 999: 1 } }));
            attempt(() => zlib.zstdCompressSync('x', { params: { 101: 40 } }));
            attempt(() => zlib.zstdCompressSync('x', { params: { 100: 'x' } }));
            attempt(() => zlib.zstdDecompressSync(Buffer.from('nonsense')));
            attempt(() => zlib.zstdCompressSync('x', { flush: 3 }));
            // One after another, so what each says comes in the same order everywhere.
            const strict = zlib.createZstdDecompress({ rejectGarbageAfterEnd: true });
            strict.on('data', (part) => console.log(String(part)));
            strict.on('error', (error) => {
                console.log(error.code);
                const short = zlib.createZstdCompress({ pledgedSrcSize: 9 });
                short.on('error', (error) => {
                    console.log(error.code, error.errno, error.message);
                    const parts = [];
                    const packer = zlib.createZstdCompress({ pledgedSrcSize: 5 });
                    packer.on('data', (part) => parts.push(part.toString('hex')));
                    packer.on('end', () => {
                        console.log(parts.join(' '));
                        zlib.zstdCompress('later', (error, packed) => zlib.zstdDecompress(packed, { info: true }, (again, { buffer, engine }) => console.log(String(buffer), engine.constructor.name, engine._closed)));
                    });
                    packer.write('hel');
                    packer.flush(() => packer.end('lo'));
                });
                short.end('hello');
            });
            strict.end(Buffer.concat([zlib.zstdCompressSync('once'), zlib.zstdCompressSync('twice')]));
        "#);
        assert_eq!(
            shown,
            "63 201 72 9\ntrue\nRangeError ERR_ZSTD_INVALID_PARAM undefined 999 is not a valid zstd parameter\n\
             Error ERR_ZLIB_INITIALIZATION_FAILED undefined Setting parameter failed\n\
             TypeError ERR_INVALID_ARG_TYPE undefined The \"options.params[key]\" property must be of type number. Received type string ('x')\n\
             Error ZSTD_error_prefix_unknown 10 Unknown frame descriptor\n\
             RangeError ERR_OUT_OF_RANGE undefined The value of \"options.flush\" is out of range. It must be >= 0 and <= 2. Received 3\n\
             once\nERR_TRAILING_JUNK_AFTER_STREAM_END\nZSTD_error_srcSize_wrong 72 Src size is incorrect\n\
             28b52ffd200518000068656c 1100006c6f\nlater ZstdDecompress true"
        );
    }

    #[test]
    fn sums_are_what_zlib_says() {
        assert_eq!(crc32(b"hello", 0), 907_060_870);
        assert_eq!(crc32(b"hello world, and more", 0), crc32(b" and more", crc32(b"hello world,", 0)));
        assert_eq!(adler32_from(b"Wikipedia", 1), 0x11e6_0398);
        // Each way of working them out gives what the simplest does, for every length around where each changes.
        let bytes = random(20_000);
        let simple_adler = |bytes: &[u8], start: u32| {
            let (mut a, mut b) = (start & 0xffff, start >> 16);
            for byte in bytes {
                a = (a + u32::from(*byte)) % 65521;
                b = (b + a) % 65521;
            }
            b << 16 | a
        };
        for length in (0..300).chain([4095, 4096, 4097, 5552, 5553, 8191, 12345, 20_000]) {
            for start in [0, 1, 0x1234_5678, u32::MAX] {
                assert_eq!(crc32(&bytes[..length], start), crc32_by_tables(&bytes[..length], start), "{length}");
                let start = ((start & 0xffff) % 65521) | (((start >> 16) % 65521) << 16);
                assert_eq!(adler32_from(&bytes[..length], start), simple_adler(&bytes[..length], start), "{length}");
            }
        }
        let mut simple_crc = !0u32;
        for byte in &bytes {
            simple_crc ^= u32::from(*byte);
            for _ in 0..8 {
                simple_crc = if simple_crc & 1 == 1 { 0xedb8_8320 ^ (simple_crc >> 1) } else { simple_crc >> 1 };
            }
        }
        assert_eq!(crc32(&bytes, 0), !simple_crc);
    }

    #[test]
    fn what_zlib_made_is_read_and_what_is_wrong_says_so() {
        // "made by something else", as zlib gzips it, with a fixed-code block.
        let made = [
            0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 2, 0xff, 0xcb, 0x4d, 0x4c, 0x49, 0x55, 0x48, 0xaa, 0x54, 0x28, 0xce, 0xcf,
            0x4d, 0x2d, 0xc9, 0xc8, 0xcc, 0x4b, 0x57, 0x48, 0xcd, 0x29, 0x4e, 5, 0, 0x9f, 0x14, 0x61, 0x24, 0x16, 0,
            0, 0,
        ];
        assert_eq!(all_at_once(&recipe(3, -1), &made, 4).unwrap(), b"made by something else");
        assert_eq!(all_at_once(&recipe(3, -1), &made[..20], 4).unwrap_err(), "Z_BUF_ERROR:unexpected end of file");
        assert_eq!(all_at_once(&recipe(1, -1), b"hello", 4).unwrap_err(), "Z_DATA_ERROR:incorrect header check");
        // What's cut short isn't wrong if the last flush isn't a finish.
        assert_eq!(all_at_once(&recipe(3, -1), &made[..20], 2).unwrap(), b"made by s");
        // These are the bytes Node writes for nothing at all.
        assert_eq!(all_at_once(&recipe(0, -1), b"", 4).unwrap(), [0x78, 0x9c, 3, 0, 0, 0, 0, 1]);
        assert_eq!(all_at_once(&recipe(4, 0), b"", 4).unwrap(), [1, 0, 0, 0xff, 0xff]);
    }
}
