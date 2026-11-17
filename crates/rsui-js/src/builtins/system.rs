//! What `os`, `crypto` and `child_process` come down to. It's only reached by a program that asks for them, through
//! the natives `node/fs.js` is given, so the rest pay nothing for it but the code.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::windows::io::AsHandle;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

use super::host::{bytes_of, new_bytes};
use crate::background::Outcome;
use crate::net::Stream;
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

// Hashes. Each of these is what its standard says, one block at a time.

fn compress_md5(state: &mut [u32; 4], chunk: &[u8]) {
    const SHIFTS: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];

    let mut words = [0u32; 16];
    for (word, bytes) in words.iter_mut().zip(chunk.chunks_exact(4)) {
        *word = u32::from_le_bytes(bytes.try_into().expect("four bytes"));
    }
    let [mut a, mut b, mut c, mut d] = *state;
    for (round, shift) in SHIFTS.iter().enumerate() {
        let (mixed, index) = match round / 16 {
            0 => ((b & c) | (!b & d), round),
            1 => ((d & b) | (!d & c), (5 * round + 1) % 16),
            2 => (b ^ c ^ d, (3 * round + 5) % 16),
            _ => (c ^ (b | !d), (7 * round) % 16),
        };
        let constant = (((round + 1) as f64).sin().abs() * 4_294_967_296.0) as u32;
        let sum = a
            .wrapping_add(mixed)
            .wrapping_add(constant)
            .wrapping_add(words[index]);
        a = d;
        d = c;
        c = b;
        b = b.wrapping_add(sum.rotate_left(*shift));
    }
    for (kept, value) in state.iter_mut().zip([a, b, c, d]) {
        *kept = kept.wrapping_add(value);
    }
}

fn compress_sha1(state: &mut [u32; 5], chunk: &[u8]) {
    let mut words = [0u32; 80];
    for (word, bytes) in words.iter_mut().zip(chunk.chunks_exact(4)) {
        *word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    }
    for at in 16..80 {
        words[at] =
            (words[at - 3] ^ words[at - 8] ^ words[at - 14] ^ words[at - 16]).rotate_left(1);
    }
    let [mut a, mut b, mut c, mut d, mut e] = *state;
    for (round, word) in words.iter().enumerate() {
        let (mixed, constant) = match round / 20 {
            0 => ((b & c) | (!b & d), 0x5a82_7999),
            1 => (b ^ c ^ d, 0x6ed9_eba1),
            2 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
            _ => (b ^ c ^ d, 0xca62_c1d6u32),
        };
        let next = a
            .rotate_left(5)
            .wrapping_add(mixed)
            .wrapping_add(e)
            .wrapping_add(constant)
            .wrapping_add(*word);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = next;
    }
    for (kept, value) in state.iter_mut().zip([a, b, c, d, e]) {
        *kept = kept.wrapping_add(value);
    }
}

fn compress_sha256(state: &mut [u32; 8], chunk: &[u8]) {
    let constants = sha2_constants();

    let mut words = [0u32; 64];
    for (word, bytes) in words.iter_mut().zip(chunk.chunks_exact(4)) {
        *word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    }
    for at in 16..64 {
        let one = words[at - 15].rotate_right(7)
            ^ words[at - 15].rotate_right(18)
            ^ (words[at - 15] >> 3);
        let two =
            words[at - 2].rotate_right(17) ^ words[at - 2].rotate_right(19) ^ (words[at - 2] >> 10);
        words[at] = words[at - 16]
            .wrapping_add(one)
            .wrapping_add(words[at - 7])
            .wrapping_add(two);
    }
    let mut now: [u32; 8] = *state;
    for at in 0..64 {
        let [a, b, c, d, e, f, g, h] = now;
        let big_one = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choose = (e & f) ^ (!e & g);
        let first = h
            .wrapping_add(big_one)
            .wrapping_add(choose)
            .wrapping_add((constants[at] >> 32) as u32)
            .wrapping_add(words[at]);
        let big_zero = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let second = big_zero.wrapping_add(majority);
        now = [
            first.wrapping_add(second),
            a,
            b,
            c,
            d.wrapping_add(first),
            e,
            f,
            g,
        ];
    }
    for (kept, value) in state.iter_mut().zip(now) {
        *kept = kept.wrapping_add(value);
    }
}

fn compress_sha512_of(state: &mut [u64; 8], chunk: &[u8]) {
    let constants = sha2_constants();

    let mut words = [0u64; 80];
    for (word, bytes) in words.iter_mut().zip(chunk.chunks_exact(8)) {
        *word = u64::from_be_bytes(bytes.try_into().expect("this is eight bytes"));
    }
    for at in 16..80 {
        let one =
            words[at - 15].rotate_right(1) ^ words[at - 15].rotate_right(8) ^ (words[at - 15] >> 7);
        let two =
            words[at - 2].rotate_right(19) ^ words[at - 2].rotate_right(61) ^ (words[at - 2] >> 6);
        words[at] = words[at - 16]
            .wrapping_add(one)
            .wrapping_add(words[at - 7])
            .wrapping_add(two);
    }
    let mut now: [u64; 8] = *state;
    for at in 0..80 {
        let [a, b, c, d, e, f, g, h] = now;
        let big_one = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
        let choose = (e & f) ^ (!e & g);
        let first = h
            .wrapping_add(big_one)
            .wrapping_add(choose)
            .wrapping_add(constants[at])
            .wrapping_add(words[at]);
        let big_zero = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let second = big_zero.wrapping_add(majority);
        now = [
            first.wrapping_add(second),
            a,
            b,
            c,
            d.wrapping_add(first),
            e,
            f,
            g,
        ];
    }
    for (kept, value) in state.iter_mut().zip(now) {
        *kept = kept.wrapping_add(value);
    }
}

/// The first bits of the fractional part of the cube root of each of the first primes, which is what SHA-2 starts from.
/// Working them out takes less room than writing them down.
fn sha2_constants() -> &'static [u64; 80] {
    static FOUND: OnceLock<[u64; 80]> = OnceLock::new();
    FOUND.get_or_init(|| {
        let mut out = [0u64; 80];
        let mut prime = 2u32;
        for slot in &mut out {
            while !(2..prime)
                .take_while(|at| at * at <= prime)
                .all(|at| prime % at != 0)
            {
                prime += 1;
            }
            *slot = fractional_root(prime, 3);
            prime += 1;
        }
        out
    })
}

/// The first 64 bits after the point of the square or cube root of a small number, found exactly in whole numbers.
fn fractional_root(number: u32, power: u32) -> u64 {
    // The root to 64 places, found a bit at a time, as `found^power <= number << (64 * power)`.
    let mut found = [0u64; 2];
    let wanted = Big::from(number).shifted(64 * power);
    for bit in (0..72).rev() {
        let mut trying = found;
        if bit >= 64 {
            trying[1] |= 1 << (bit - 64);
        } else {
            trying[0] |= 1 << bit;
        }
        let root = Big::from_pair(trying);
        let raised = if power == 2 {
            root.times(&root)
        } else {
            root.times(&root).times(&root)
        };
        if raised.at_most(&wanted) {
            found = trying;
        }
    }
    found[0]
}

/// A whole number as large as these need, least significant part first.
#[derive(Clone)]
struct Big(Vec<u32>);

impl Big {
    fn from(number: u32) -> Self {
        Self(vec![number])
    }

    fn from_pair([low, high]: [u64; 2]) -> Self {
        Self(vec![
            low as u32,
            (low >> 32) as u32,
            high as u32,
            (high >> 32) as u32,
        ])
    }

    fn shifted(&self, bits: u32) -> Self {
        let mut parts = vec![0u32; (bits / 32) as usize];
        parts.extend_from_slice(&self.0);
        debug_assert!(bits % 32 == 0);
        Self(parts)
    }

    fn times(&self, other: &Self) -> Self {
        let mut out = vec![0u32; self.0.len() + other.0.len()];
        for (at, a) in self.0.iter().enumerate() {
            let mut carry = 0u64;
            for (place, b) in other.0.iter().enumerate() {
                let sum = u64::from(*a) * u64::from(*b) + u64::from(out[at + place]) + carry;
                out[at + place] = sum as u32;
                carry = sum >> 32;
            }
            out[at + other.0.len()] = carry as u32;
        }
        Self(out)
    }

    fn at_most(&self, other: &Self) -> bool {
        let longest = self.0.len().max(other.0.len());
        for at in (0..longest).rev() {
            let (a, b) = (
                self.0.get(at).copied().unwrap_or(0),
                other.0.get(at).copied().unwrap_or(0),
            );
            if a != b {
                return a < b;
            }
        }
        true
    }
}

/// The same for square roots of the first primes, which is what SHA-2 starts from.
fn sha2_starts() -> &'static [u64] {
    static FOUND: OnceLock<Vec<u64>> = OnceLock::new();
    FOUND.get_or_init(|| {
        first_primes(16)
            .into_iter()
            .map(|prime| fractional_root(prime, 2))
            .collect()
    })
}

fn first_primes(count: usize) -> Vec<u32> {
    let mut out = Vec::with_capacity(count);
    let mut prime = 2u32;
    while out.len() < count {
        if (2..prime)
            .take_while(|at| at * at <= prime)
            .all(|at| prime % at != 0)
        {
            out.push(prime);
        }
        prime += 1;
    }
    out
}

/// Private value-owned chaining words; SHA-384 shares SHA-512 compression.
#[derive(Clone)]
enum HashWords {
    Md5([u32; 4]),
    Sha1([u32; 5]),
    Sha256([u32; 8]),
    Sha512([u64; 8]),
}

impl HashWords {
    fn compress(&mut self, block: &[u8]) {
        match self {
            Self::Md5(words) => compress_md5(words, block),
            Self::Sha1(words) => compress_sha1(words, block),
            Self::Sha256(words) => compress_sha256(words, block),
            Self::Sha512(words) => compress_sha512_of(words, block),
        }
    }
}

/// Bounded tail and explicit byte count, with no borrowed data or heap storage.
/// Counts wrap modulo 2^128; final lengths are the low 64 or 128 bits of the
/// bit count, as in the original one-shot padding (MD5 uses little endian).
#[derive(Clone)]
struct HashState {
    words: HashWords,
    algorithm: Pbkdf2Hash,
    tail: [u8; 128],
    used: usize,
    bytes: u128,
}

impl HashState {
    fn new(algorithm: Pbkdf2Hash) -> Self {
        let words = match algorithm {
            Pbkdf2Hash::Md5 => HashWords::Md5([0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476]),
            Pbkdf2Hash::Sha1 => HashWords::Sha1([
                0x6745_2301,
                0xefcd_ab89,
                0x98ba_dcfe,
                0x1032_5476,
                0xc3d2_e1f0,
            ]),
            Pbkdf2Hash::Sha256 => {
                HashWords::Sha256(std::array::from_fn(|i| (sha2_starts()[i] >> 32) as u32))
            }
            Pbkdf2Hash::Sha384 | Pbkdf2Hash::Sha512 => {
                let offset = if matches!(algorithm, Pbkdf2Hash::Sha384) {
                    8
                } else {
                    0
                };
                HashWords::Sha512(std::array::from_fn(|i| sha2_starts()[offset + i]))
            }
        };
        Self {
            words,
            algorithm,
            tail: [0; 128],
            used: 0,
            bytes: 0,
        }
    }

    fn update(&mut self, mut input: &[u8]) {
        self.bytes = self.bytes.wrapping_add(input.len() as u128);
        let block = self.algorithm.sizes().0;
        if self.used != 0 {
            let take = input.len().min(block - self.used);
            self.tail[self.used..self.used + take].copy_from_slice(&input[..take]);
            self.used += take;
            input = &input[take..];
            if self.used != block {
                return;
            }
            self.words.compress(&self.tail[..block]);
            self.used = 0;
        }
        let mut chunks = input.chunks_exact(block);
        for chunk in &mut chunks {
            self.words.compress(chunk);
        }
        let rest = chunks.remainder();
        self.tail[..rest.len()].copy_from_slice(rest);
        self.used = rest.len();
    }

    fn finish(mut self) -> [u8; 64] {
        let block = self.algorithm.sizes().0;
        let length_bytes = if block == 128 { 16 } else { 8 };
        let bits = self.bytes.wrapping_mul(8);
        self.tail[self.used] = 0x80;
        self.tail[self.used + 1..block].fill(0);
        if self.used >= block - length_bytes {
            self.words.compress(&self.tail[..block]);
            self.tail[..block].fill(0);
        }
        if matches!(self.algorithm, Pbkdf2Hash::Md5) {
            self.tail[block - length_bytes..block]
                .copy_from_slice(&bits.to_le_bytes()[..length_bytes]);
        } else {
            self.tail[block - length_bytes..block]
                .copy_from_slice(&bits.to_be_bytes()[16 - length_bytes..]);
        }
        self.words.compress(&self.tail[..block]);
        let mut out = [0; 64];
        match self.words {
            HashWords::Md5(words) => {
                for (slot, word) in out.chunks_exact_mut(4).zip(words) {
                    slot.copy_from_slice(&word.to_le_bytes());
                }
            }
            HashWords::Sha1(words) => {
                for (slot, word) in out.chunks_exact_mut(4).zip(words) {
                    slot.copy_from_slice(&word.to_be_bytes());
                }
            }
            HashWords::Sha256(words) => {
                for (slot, word) in out.chunks_exact_mut(4).zip(words) {
                    slot.copy_from_slice(&word.to_be_bytes());
                }
            }
            HashWords::Sha512(words) => {
                for (slot, word) in out.chunks_exact_mut(8).zip(words) {
                    slot.copy_from_slice(&word.to_be_bytes());
                }
            }
        }
        out
    }
}

fn md5(message: &[u8]) -> Vec<u8> {
    let mut state = HashState::new(Pbkdf2Hash::Md5);
    state.update(message);
    state.finish()[..Pbkdf2Hash::Md5.sizes().1].to_vec()
}

fn sha1(message: &[u8]) -> Vec<u8> {
    let mut state = HashState::new(Pbkdf2Hash::Sha1);
    state.update(message);
    state.finish()[..Pbkdf2Hash::Sha1.sizes().1].to_vec()
}

pub(crate) fn sha256(message: &[u8]) -> Vec<u8> {
    let mut state = HashState::new(Pbkdf2Hash::Sha256);
    state.update(message);
    state.finish()[..Pbkdf2Hash::Sha256.sizes().1].to_vec()
}

fn sha384(message: &[u8]) -> Vec<u8> {
    let mut state = HashState::new(Pbkdf2Hash::Sha384);
    state.update(message);
    state.finish()[..Pbkdf2Hash::Sha384.sizes().1].to_vec()
}

fn sha512(message: &[u8]) -> Vec<u8> {
    let mut state = HashState::new(Pbkdf2Hash::Sha512);
    state.update(message);
    state.finish()[..Pbkdf2Hash::Sha512.sizes().1].to_vec()
}

/// `hash(name, bytes)`, which is `undefined` for one this doesn't have.
fn hash(vm: &mut Vm, args: Args) -> Run<Value> {
    let name = vm.arg(&args, 0);
    let name = vm.to_rust_string(name)?.to_lowercase();
    let given = vm.arg(&args, 1);
    let bytes = bytes_of(vm, given)?;
    let digest = match name.replace('-', "").as_str() {
        "md5" => md5(&bytes),
        "sha1" => sha1(&bytes),
        "sha256" => sha256(&bytes),
        "sha384" => sha384(&bytes),
        "sha512" => sha512(&bytes),
        _ => return Ok(Value::UNDEFINED),
    };
    new_bytes(vm, digest)
}

#[derive(Clone, Copy, Debug)]
enum Pbkdf2Hash {
    Md5,
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl Pbkdf2Hash {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "md5" => Some(Self::Md5),
            "sha1" => Some(Self::Sha1),
            "sha256" => Some(Self::Sha256),
            "sha384" => Some(Self::Sha384),
            "sha512" => Some(Self::Sha512),
            _ => None,
        }
    }

    #[cfg(test)]
    fn digest(self, bytes: &[u8]) -> Vec<u8> {
        match self {
            Self::Md5 => md5(bytes),
            Self::Sha1 => sha1(bytes),
            Self::Sha256 => sha256(bytes),
            Self::Sha384 => sha384(bytes),
            Self::Sha512 => sha512(bytes),
        }
    }

    fn sizes(self) -> (usize, usize) {
        match self {
            Self::Md5 => (64, 16),
            Self::Sha1 => (64, 20),
            Self::Sha256 => (64, 32),
            Self::Sha384 => (128, 48),
            Self::Sha512 => (128, 64),
        }
    }
}

/// Key normalization and the two keyed prefix blocks are computed once.
/// Each PRF clones only fixed storage; neither inputs nor digests allocate.
struct HmacState {
    inner: HashState,
    outer: HashState,
}

impl HmacState {
    fn new(algorithm: Pbkdf2Hash, password: &[u8]) -> Self {
        let (block, hlen) = algorithm.sizes();
        let normalized;
        let key = if password.len() > block {
            let mut state = HashState::new(algorithm);
            state.update(password);
            normalized = state.finish();
            &normalized[..hlen]
        } else {
            password
        };
        let mut pad = [0x36; 128];
        for (slot, byte) in pad.iter_mut().zip(key) {
            *slot ^= byte;
        }
        let mut inner = HashState::new(algorithm);
        inner.update(&pad[..block]);
        for slot in &mut pad[..block] {
            *slot ^= 0x36 ^ 0x5c;
        }
        let mut outer = HashState::new(algorithm);
        outer.update(&pad[..block]);
        Self { inner, outer }
    }

    fn digest(&self, first: &[u8], second: &[u8]) -> [u8; 64] {
        let mut inner = self.inner.clone();
        inner.update(first);
        inner.update(second);
        let digest = inner.finish();
        let mut outer = self.outer.clone();
        outer.update(&digest[..self.outer.algorithm.sizes().1]);
        outer.finish()
    }
}

/// RFC 8018 section 5.2, with prehashed HMAC prefixes and fixed U/T buffers.
/// Only the returned output allocates after the shared hash constants have
/// initialized. No VM access, input copies or normalization inside the rounds.
fn pbkdf2(
    password: &[u8],
    salt: &[u8],
    rounds: u32,
    length: usize,
    algorithm: Pbkdf2Hash,
) -> Result<Vec<u8>, &'static str> {
    if rounds == 0 {
        return Err("PBKDF2 rounds must be positive");
    }
    let (block, hlen) = algorithm.sizes();
    let blocks = length.div_ceil(hlen);
    let blocks = u32::try_from(blocks).map_err(|_| "PBKDF2 output is too long")?;
    if length > i32::MAX as usize {
        return Err("PBKDF2 output exceeds the native limit");
    }
    if length == 0 {
        return Ok(Vec::new());
    }
    let input_length = salt.len().checked_add(4).ok_or("PBKDF2 salt is too long")?;
    let scratch_length = block
        .checked_add(input_length.max(hlen))
        .ok_or("PBKDF2 input is too long")?;
    // Preserve the existing input-size limit, although the input and padding
    // now stream through bounded storage instead of a concatenated allocation.
    scratch_length
        .checked_add(block * 2)
        .filter(|&n| n <= isize::MAX as usize)
        .ok_or("PBKDF2 input is too long")?;
    let mut out = Vec::new();
    out.try_reserve_exact(length)
        .map_err(|_| "PBKDF2 output allocation failed")?;
    let hmac = HmacState::new(algorithm, password);
    for index in 1..=blocks {
        let mut u = hmac.digest(salt, &index.to_be_bytes());
        let mut t = u;
        for _ in 1..rounds {
            u = hmac.digest(&u[..hlen], &[]);
            for (to, byte) in t[..hlen].iter_mut().zip(&u[..hlen]) {
                *to ^= byte;
            }
        }
        out.extend_from_slice(&t[..hlen.min(length - out.len())]);
    }
    Ok(out)
}

/// A strict internal entry; the JS wrapper keeps coercions on its old path.
/// All input bytes are owned before computing or calling `new_bytes` (which
/// can execute JS). This remains synchronous even in the async microtask.
fn pbkdf2_native(vm: &mut Vm, args: Args) -> Run<Value> {
    let name = vm.arg(&args, 4);
    if !name.is_string() {
        return Err(vm.type_error("PBKDF2 digest must be a string"));
    }
    let name = vm.to_rust_string(name)?.to_lowercase().replace('-', "");
    let Some(algorithm) = Pbkdf2Hash::from_name(&name) else {
        return Err(vm.type_error("Digest method not supported"));
    };
    let rounds = vm.arg(&args, 2);
    let length = vm.arg(&args, 3);
    let checked = |value: Value, minimum: f64| {
        value.is_number()
            && value.as_number().is_finite()
            && value.as_number().fract() == 0.0
            && value.as_number() >= minimum
            && value.as_number() <= i32::MAX as f64
    };
    if !checked(rounds, 1.0) {
        return Err(vm.range_error("PBKDF2 rounds must be a positive native integer"));
    }
    if !checked(length, 0.0) {
        return Err(vm.range_error("PBKDF2 length must be a nonnegative native integer"));
    }
    let password = vm.arg(&args, 0);
    let password = bytes_of(vm, password)?;
    let salt = vm.arg(&args, 1);
    let salt = bytes_of(vm, salt)?;
    let result = pbkdf2(
        &password,
        &salt,
        rounds.as_number() as u32,
        length.as_number() as usize,
        algorithm,
    )
    .map_err(|message| vm.range_error(message))?;
    new_bytes(vm, result)
}

// About the machine.

#[repr(C)]
#[derive(Default)]
struct MemoryStatus {
    length: u32,
    load: u32,
    total: u64,
    available: u64,
    page_total: u64,
    page_available: u64,
    virtual_total: u64,
    virtual_available: u64,
    extended: u64,
}

#[repr(C)]
struct VersionInfo {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform: u32,
    service_pack: [u16; 128],
}

#[allow(unsafe_code)]
mod windows {
    use super::{MemoryStatus, VersionInfo};
    use std::os::windows::io::{AsRawHandle, OwnedHandle};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
        fn GetTickCount64() -> u64;
        fn GetCurrentProcessId() -> u32;
        fn WaitForSingleObject(handle: isize, wait: u32) -> u32;
        fn GetExitCodeProcess(process: isize, code: *mut u32) -> i32;
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut VersionInfo) -> i32;
    }

    pub(super) fn memory() -> (u64, u64) {
        let mut status = MemoryStatus {
            length: size_of::<MemoryStatus>() as u32,
            ..MemoryStatus::default()
        };
        // SAFETY: it's the size it says it is, as this call wants.
        unsafe {
            GlobalMemoryStatusEx(&raw mut status);
        }
        (status.total, status.available)
    }

    pub(super) fn milliseconds_up() -> u64 {
        // SAFETY: this takes nothing, and can't fail.
        unsafe { GetTickCount64() }
    }

    pub(super) fn process_id() -> u32 {
        // SAFETY: this takes nothing, and can't fail.
        unsafe { GetCurrentProcessId() }
    }

    /// Waits, using nothing while it does, for another program to finish, and gives its exit code.
    pub(super) fn wait_for_exit(process: &OwnedHandle) -> Option<i32> {
        const INFINITE: u32 = u32::MAX;
        const WAIT_OBJECT_0: u32 = 0;
        let raw = process.as_raw_handle() as isize;
        let mut code = 0;
        // SAFETY: the handle is open for as long as `process` is, which is through both calls, and the code has room.
        unsafe {
            if WaitForSingleObject(raw, INFINITE) != WAIT_OBJECT_0 {
                return None;
            }
            if GetExitCodeProcess(raw, &raw mut code) == 0 {
                return None;
            }
        }
        Some(code as i32)
    }

    pub(super) fn version() -> String {
        let mut info = VersionInfo {
            size: size_of::<VersionInfo>() as u32,
            major: 0,
            minor: 0,
            build: 0,
            platform: 0,
            service_pack: [0; 128],
        };
        // SAFETY: it's the size it says it is, as this call wants.
        unsafe {
            RtlGetVersion(&raw mut info);
        }
        format!("{}.{}.{}", info.major, info.minor, info.build)
    }
}

/// `machine()`, which is what `os` is written with.
fn machine(vm: &mut Vm, _: Args) -> Run<Value> {
    let (total, free) = windows::memory();
    let cpus = std::thread::available_parallelism().map_or(1, usize::from);
    let release = windows::version();
    let out = vm.new_plain();
    let set_number = |vm: &mut Vm, name: &str, value: f64| {
        let atom = vm.intern_str(name);
        vm.define(
            out.handle(),
            atom,
            Value::number(value),
            crate::object::PLAIN,
        );
    };
    set_number(vm, "totalmem", total as f64);
    set_number(vm, "freemem", free as f64);
    set_number(vm, "cpus", cpus as f64);
    set_number(vm, "uptime", windows::milliseconds_up() as f64 / 1000.0);
    set_number(vm, "pid", f64::from(windows::process_id()));
    let text = vm.new_str(&release);
    let atom = vm.intern_str("release");
    vm.define(out.handle(), atom, text, crate::object::PLAIN);
    Ok(out)
}

// Other programs. What they read and write goes through pipes that the one thread for sockets looks after, and each has a
// thread of its own only to wait for it to finish, which uses nothing while it waits.

struct Running {
    child: Child,
    stdin: Option<Arc<Stream>>,
    stdout: Option<Arc<Stream>>,
    stderr: Option<Arc<Stream>>,
}

fn running() -> &'static Mutex<HashMap<u32, Running>> {
    static RUNNING: OnceLock<Mutex<HashMap<u32, Running>>> = OnceLock::new();
    RUNNING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// What `start` and `run` are both given: the program, what to give it, where, and with what in its environment.
struct Program {
    file: String,
    arguments: Vec<String>,
    folder: Option<String>,
    environment: Option<Vec<(String, String)>>,
    shell: bool,
}

fn read_list_of_text(vm: &mut Vm, value: Value) -> Run<Vec<String>> {
    let mut out = Vec::new();
    if value.is_object() {
        let length = vm.get(value, crate::text::LENGTH)?;
        let length = vm.to_number(length)? as usize;
        for index in 0..length {
            let item = vm.get_index(value, index)?;
            out.push(vm.to_rust_string(item)?);
        }
    }
    Ok(out)
}

/// `(file, [arguments], folder, [[name, value]...], whether it's a command for the shell)`.
fn read_program(vm: &mut Vm, args: &Args) -> Run<Program> {
    let file = vm.arg(args, 0);
    let file = vm.to_rust_string(file)?;
    let arguments = vm.arg(args, 1);
    let arguments = read_list_of_text(vm, arguments)?;
    let folder = vm.arg(args, 2);
    let folder = if folder.is_string() {
        Some(vm.to_rust_string(folder)?)
    } else {
        None
    };
    let given = vm.arg(args, 3);
    let environment = if given.is_object() {
        let length = vm.get(given, crate::text::LENGTH)?;
        let length = vm.to_number(length)? as usize;
        let mut pairs = Vec::with_capacity(length);
        for index in 0..length {
            let pair = vm.get_index(given, index)?;
            let name = vm.get_index(pair, 0)?;
            let value = vm.get_index(pair, 1)?;
            pairs.push((vm.to_rust_string(name)?, vm.to_rust_string(value)?));
        }
        Some(pairs)
    } else {
        None
    };
    let shell = vm.arg(args, 4) == Value::TRUE;
    Ok(Program {
        file,
        arguments,
        folder,
        environment,
        shell,
    })
}

impl Program {
    fn command(&self) -> Command {
        // Without this, each one would bring up a console window from a program with a window of its own.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut command = if self.shell {
            let mut command =
                Command::new(std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_owned()));
            command.raw_arg("/d /s /c");
            command.raw_arg(format!("\"{}\"", self.file));
            command
        } else {
            let mut command = Command::new(&self.file);
            command.args(&self.arguments);
            command
        };
        command.creation_flags(CREATE_NO_WINDOW);
        if let Some(folder) = &self.folder {
            command.current_dir(folder);
        }
        if let Some(environment) = &self.environment {
            command.env_clear();
            command.envs(environment.iter().map(|(name, value)| (name, value)));
        }
        command
    }
}

fn spawn_error(vm: &mut Vm, error: &std::io::Error, file: &str) -> Value {
    let code = match error.kind() {
        std::io::ErrorKind::NotFound => "ENOENT",
        std::io::ErrorKind::PermissionDenied => "EACCES",
        _ => "EIO",
    };
    let thrown = vm.make_error(vm.realm.error_proto, &format!("spawn {file} {code}"));
    let code_text = vm.new_str(code);
    let atom = vm.intern_str("code");
    vm.define(thrown.handle(), atom, code_text, crate::object::PLAIN);
    thrown
}

/// `start(...)`, as [`read_program`] reads it. It gives the program's number here and its process id.
fn start(vm: &mut Vm, args: Args) -> Run<Value> {
    let program = read_program(vm, &args)?;
    let pipes = crate::net::child_pipe(false).and_then(|input| {
        let output = crate::net::child_pipe(true)?;
        Ok((input, output, crate::net::child_pipe(true)?))
    });
    let ((stdin, their_stdin), (stdout, their_stdout), (stderr, their_stderr)) = match pipes {
        Ok(pipes) => pipes,
        Err(error) => return Err(spawn_error(vm, &error, &program.file)),
    };
    let mut command = program.command();
    command
        .stdin(Stdio::from(their_stdin))
        .stdout(Stdio::from(their_stdout))
        .stderr(Stdio::from(their_stderr));
    let spawned = command.spawn();
    // That holds this side's copies of the ends it gave, which have to go for the end of what it writes to be seen.
    drop(command);
    let child = match spawned {
        Ok(child) => child,
        Err(error) => return Err(spawn_error(vm, &error, &program.file)),
    };
    let process_id = child.id();
    let running_one = Running {
        stdin: Some(stdin),
        stdout: Some(stdout),
        stderr: Some(stderr),
        child,
    };
    let number = next_number();
    if let Ok(mut all) = running().lock() {
        all.insert(number, running_one);
    }
    let out = vm.new_plain();
    let atom = vm.intern_str("number");
    vm.define(
        out.handle(),
        atom,
        Value::number(f64::from(number)),
        crate::object::PLAIN,
    );
    let atom = vm.intern_str("pid");
    vm.define(
        out.handle(),
        atom,
        Value::number(f64::from(process_id)),
        crate::object::PLAIN,
    );
    Ok(out)
}

fn next_number() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// `readFrom(number, whether it's the error stream)`: a promise for the next bytes it writes, or `null` once it's finished.
fn read_from(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = vm.arg(&args, 0);
    let number = vm.to_number(number)? as u32;
    let errors = vm.arg(&args, 1) == Value::TRUE;
    let stream = running().lock().ok().and_then(|all| {
        let one = all.get(&number)?;
        if errors {
            one.stderr.clone()
        } else {
            one.stdout.clone()
        }
    });
    Ok(match stream {
        Some(stream) => crate::net::read_stream(vm, &stream),
        None => vm.settled_promise(Outcome::Null),
    })
}

/// `writeTo(number, bytes or null)`: a promise for it having been given this as its input, or `null` to say that's all of it.
/// A program that isn't reading doesn't hold anything else up, as the write waits on the thread for sockets.
fn write_to(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = vm.arg(&args, 0);
    let number = vm.to_number(number)? as u32;
    let given = vm.arg(&args, 1);
    if given.is_nullish() {
        let stdin = running()
            .lock()
            .ok()
            .and_then(|mut all| all.get_mut(&number)?.stdin.take());
        if let Some(stdin) = stdin {
            stdin.close();
        }
        return Ok(vm.settled_promise(Outcome::Nothing));
    }
    let bytes = bytes_of(vm, given)?;
    let stdin = running()
        .lock()
        .ok()
        .and_then(|all| all.get(&number)?.stdin.clone());
    Ok(match stdin {
        Some(stdin) => crate::net::write_stream(vm, &stdin, bytes),
        None => vm.settled_promise(Outcome::Nothing),
    })
}

/// `waitFor(number)`: a promise for its exit code, once it has one.
fn wait_for(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = vm.arg(&args, 0);
    let number = vm.to_number(number)? as u32;
    // It's waited on through a copy of what names it, so that it can still be stopped, or forgotten, while it's waited on.
    let process = running().lock().ok().and_then(|all| {
        all.get(&number)?
            .child
            .as_handle()
            .try_clone_to_owned()
            .ok()
    });
    let Some(process) = process else {
        return Ok(vm.settled_promise(Outcome::Null));
    };
    Ok(
        vm.in_own_thread(move || match windows::wait_for_exit(&process) {
            Some(code) => Outcome::Number(f64::from(code)),
            None => Outcome::Null,
        }),
    )
}

/// `forget(number)`, for one that's finished and been read to the end.
fn forget(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = vm.arg(&args, 0);
    let number = vm.to_number(number)? as u32;
    if let Ok(mut all) = running().lock() {
        all.remove(&number);
    }
    Ok(Value::UNDEFINED)
}

/// `stop(number)`, which says whether there was anything to stop.
fn stop(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = vm.arg(&args, 0);
    let number = vm.to_number(number)? as u32;
    let stopped = running()
        .lock()
        .ok()
        .and_then(|mut all| all.get_mut(&number)?.child.kill().ok());
    Ok(Value::bool(stopped.is_some()))
}

/// `runNow(...)`, as [`read_program`] reads it, then what to give it as input, waiting until it's finished.
fn run_now(vm: &mut Vm, args: Args) -> Run<Value> {
    let program = read_program(vm, &args)?;
    let input = vm.arg(&args, 5);
    let input = if input.is_nullish() {
        Vec::new()
    } else {
        bytes_of(vm, input)?
    };
    let mut command = program.command();
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return Err(spawn_error(vm, &error, &program.file)),
    };
    // It could fill up one of these while this is writing to it, so writing goes on its own thread.
    let writing = child.stdin.take().map(|mut stdin| {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        })
    });
    let done = child.wait_with_output();
    if let Some(writing) = writing {
        let _ = writing.join();
    }
    let done = match done {
        Ok(done) => done,
        Err(error) => return Err(spawn_error(vm, &error, &program.file)),
    };
    let out = vm.new_plain();
    let stdout = new_bytes(vm, done.stdout)?;
    let stderr = new_bytes(vm, done.stderr)?;
    let atom = vm.intern_str("status");
    vm.define(
        out.handle(),
        atom,
        Value::number(f64::from(done.status.code().unwrap_or(1))),
        crate::object::PLAIN,
    );
    let atom = vm.intern_str("stdout");
    vm.define(out.handle(), atom, stdout, crate::object::PLAIN);
    let atom = vm.intern_str("stderr");
    vm.define(out.handle(), atom, stderr, crate::object::PLAIN);
    Ok(out)
}

/// `writeOut(bytes or text, whether it's for the error stream)`, for `process.stdout` and `process.stderr`.
fn write_out(vm: &mut Vm, args: Args) -> Run<Value> {
    let given = vm.arg(&args, 0);
    let bytes = if given.is_string() {
        vm.to_rust_string(given)?.into_bytes()
    } else {
        bytes_of(vm, given)?
    };
    if vm.arg(&args, 1) == Value::TRUE {
        let _ = std::io::stderr().write_all(&bytes);
    } else {
        let _ = std::io::stdout().write_all(&bytes);
    }
    Ok(Value::UNDEFINED)
}

/// `exitNow(code)`, which is what `process.exit` comes to.
fn exit_now(vm: &mut Vm, args: Args) -> Run<Value> {
    let code = vm.arg(&args, 0);
    let code = if code.is_undefined() {
        0.0
    } else {
        vm.to_number(code)?
    };
    let _ = std::io::stdout().flush();
    std::process::exit(code as i32);
}

// Files by number.

fn open_files() -> &'static Mutex<HashMap<u32, std::fs::File>> {
    static OPEN: OnceLock<Mutex<HashMap<u32, std::fs::File>>> = OnceLock::new();
    OPEN.get_or_init(|| Mutex::new(HashMap::new()))
}

fn file_error(vm: &mut Vm, error: &std::io::Error, doing: &'static str, path: String) -> Value {
    match vm.settle_now(super::host::failed(error, doing, path)) {
        Err(thrown) => thrown,
        Ok(value) => value,
    }
}

/// `open(path, read, write, append, create, truncate, exclusive)`, which gives its number.
fn open_file(vm: &mut Vm, args: Args) -> Run<Value> {
    let path = vm.arg(&args, 0);
    let path = vm.to_rust_string(path)?;
    let on = |vm: &mut Vm, index: usize| vm.arg(&args, index) == Value::TRUE;
    let (read, write, append) = (on(vm, 1), on(vm, 2), on(vm, 3));
    let (create, truncate, exclusive) = (on(vm, 4), on(vm, 5), on(vm, 6));
    let opened = std::fs::OpenOptions::new()
        .read(read)
        .write(write && !append)
        .append(append)
        .create(create && !exclusive)
        .truncate(truncate)
        .create_new(exclusive && create)
        .open(&path);
    match opened {
        Ok(file) => {
            let number = next_number() + 16;
            if let Ok(mut all) = open_files().lock() {
                all.insert(number, file);
            }
            Ok(Value::number(f64::from(number)))
        }
        Err(error) => Err(file_error(vm, &error, "open", path)),
    }
}

/// What's done with a file by its number, which is an error like Node's if there's no such file.
fn with_file<T>(
    vm: &mut Vm,
    args: &Args,
    doing: &'static str,
    run: impl FnOnce(&mut std::fs::File) -> std::io::Result<T>,
) -> Run<T> {
    let number = vm.arg(args, 0);
    let number = vm.to_number(number)? as u32;
    let found = match open_files().lock() {
        Ok(mut all) => all.get_mut(&number).map(run),
        Err(_) => None,
    };
    match found {
        Some(Ok(value)) => Ok(value),
        Some(Err(error)) => Err(file_error(vm, &error, doing, String::new())),
        None => {
            let error = std::io::Error::other("bad file descriptor");
            let thrown = file_error(vm, &error, doing, String::new());
            let code = vm.new_str("EBADF");
            let atom = vm.intern_str("code");
            vm.define(thrown.handle(), atom, code, crate::object::PLAIN);
            Err(thrown)
        }
    }
}

fn close_file(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = vm.arg(&args, 0);
    let number = vm.to_number(number)? as u32;
    if let Ok(mut all) = open_files().lock() {
        all.remove(&number);
    }
    Ok(Value::UNDEFINED)
}

/// `readAt(number, length, position)`, from where the file's at if `position` is negative.
fn read_at(vm: &mut Vm, args: Args) -> Run<Value> {
    use std::io::{Seek, SeekFrom};
    let length = vm.arg(&args, 1);
    let length = vm.to_number(length)?.max(0.0) as usize;
    let position = vm.arg(&args, 2);
    let position = vm.to_number(position)?;
    let bytes = with_file(vm, &args, "read", |file| {
        // Reading from somewhere in particular leaves where it's up to as it was.
        let was_at = file.stream_position()?;
        if position >= 0.0 {
            file.seek(SeekFrom::Start(position as u64))?;
        }
        let mut out = vec![0u8; length];
        let mut filled = 0;
        while filled < length {
            let read = file.read(&mut out[filled..])?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        out.truncate(filled);
        if position >= 0.0 {
            file.seek(SeekFrom::Start(was_at))?;
        }
        Ok(out)
    })?;
    new_bytes(vm, bytes)
}

/// `writeAt(number, bytes, position)`, at where the file's at if `position` is negative.
fn write_at(vm: &mut Vm, args: Args) -> Run<Value> {
    use std::io::{Seek, SeekFrom};
    let given = vm.arg(&args, 1);
    let bytes = bytes_of(vm, given)?;
    let position = vm.arg(&args, 2);
    let position = vm.to_number(position)?;
    with_file(vm, &args, "write", |file| {
        let was_at = file.stream_position()?;
        if position >= 0.0 {
            file.seek(SeekFrom::Start(position as u64))?;
        }
        file.write_all(&bytes)?;
        if position >= 0.0 {
            file.seek(SeekFrom::Start(was_at))?;
        }
        Ok(())
    })?;
    Ok(Value::number(bytes.len() as f64))
}

fn file_stat_by_number(vm: &mut Vm, args: Args) -> Run<Value> {
    let found = with_file(vm, &args, "fstat", |file| file.metadata())?;
    let modified = found
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0.0, |since| since.as_secs_f64() * 1000.0);
    vm.settle_now(Outcome::Stat {
        file: found.is_file(),
        folder: found.is_dir(),
        size: found.len() as f64,
        modified,
    })
}

fn truncate_file(vm: &mut Vm, args: Args) -> Run<Value> {
    let length = vm.arg(&args, 1);
    let length = vm.to_number(length)?.max(0.0) as u64;
    with_file(vm, &args, "ftruncate", |file| file.set_len(length))?;
    Ok(Value::UNDEFINED)
}

fn time_from_milliseconds(milliseconds: f64) -> std::time::SystemTime {
    // There's nothing sensible to do with a time that's nothing like one but to leave it at the start.
    let since =
        std::time::Duration::try_from_secs_f64(milliseconds.max(0.0) / 1000.0).unwrap_or_default();
    std::time::UNIX_EPOCH
        .checked_add(since)
        .unwrap_or(std::time::UNIX_EPOCH)
}

/// `setTime(path, milliseconds)`.
fn set_time(vm: &mut Vm, args: Args) -> Run<Value> {
    let path = vm.arg(&args, 0);
    let path = vm.to_rust_string(path)?;
    let when = vm.arg(&args, 1);
    let when = time_from_milliseconds(vm.to_number(when)?);
    let done = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .and_then(|file| file.set_modified(when));
    match done {
        Ok(()) => Ok(Value::UNDEFINED),
        Err(error) => Err(file_error(vm, &error, "utime", path)),
    }
}

fn set_time_of(vm: &mut Vm, args: Args) -> Run<Value> {
    let when = vm.arg(&args, 1);
    let when = time_from_milliseconds(vm.to_number(when)?);
    with_file(vm, &args, "futime", |file| file.set_modified(when))?;
    Ok(Value::UNDEFINED)
}

/// `locale()`, which is what the user's Windows says, like `en-GB`.
#[allow(unsafe_code)]
fn user_locale(vm: &mut Vm, _: Args) -> Run<Value> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultLocaleName(name: *mut u16, length: i32) -> i32;
    }
    let mut name = [0u16; 85];
    // SAFETY: it writes no more than the room it's told there is.
    let length = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
    if length <= 1 {
        return Ok(vm.new_str("en-US"));
    }
    let text = String::from_utf16_lossy(&name[..length as usize - 1]);
    Ok(vm.new_str(&text))
}

pub(super) fn add_natives(vm: &mut Vm, out: Value) {
    for (name, length, function) in [
        ("open", 7, open_file as crate::object::NativeFn),
        ("close", 1, close_file),
        ("readAt", 3, read_at),
        ("writeAt", 3, write_at),
        ("fstat", 1, file_stat_by_number),
        ("truncate", 2, truncate_file),
        ("setTime", 2, set_time),
        ("setTimeOf", 2, set_time_of),
        ("locale", 0, user_locale),
        ("hash", 2, hash),
        ("pbkdf2", 5, pbkdf2_native),
        ("machine", 0, machine),
        ("start", 5, start),
        ("readFrom", 2, read_from),
        ("writeTo", 2, write_to),
        ("waitFor", 1, wait_for),
        ("forget", 1, forget),
        ("stop", 1, stop),
        ("runNow", 6, run_now),
        ("writeOut", 2, write_out),
        ("exitNow", 1, exit_now),
    ] {
        vm.method(out.handle(), name, length, function);
    }
    for (name, length, function) in crate::net::NATIVES
        .into_iter()
        .chain(crate::http::NATIVES)
        .chain(crate::worker::NATIVES)
    {
        vm.method(out.handle(), name, length, function);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXED_VECTORS: &str = include_str!("../../tests/data/crypto-fixed-state-vectors.txt");

    fn fixture_bytes(length: usize) -> Vec<u8> {
        (0..length).map(|i| (i * 29 + 17) as u8).collect()
    }

    #[test]
    fn fixed_hmac_and_pbkdf2_match_node() {
        for line in FIXED_VECTORS.lines().filter(|s| !s.starts_with("hash ")) {
            let fields: Vec<_> = line.split_whitespace().collect();
            let algorithm = Pbkdf2Hash::from_name(fields[1]).unwrap();
            let key = fixture_bytes(fields[2].parse().unwrap());
            let input = fixture_bytes(fields[3].parse().unwrap());
            if fields[0] == "hmac" {
                let hmac = HmacState::new(algorithm, &key);
                for split in 0..=input.len() {
                    assert_eq!(
                        hex(&hmac.digest(&input[..split], &input[split..])[..algorithm.sizes().1]),
                        fields[4],
                        "{line}, split {split}"
                    );
                }
            } else {
                let rounds = fields[4].parse().unwrap();
                let length = fields[5].parse().unwrap();
                assert_eq!(
                    hex(&pbkdf2(&key, &input, rounds, length, algorithm).unwrap()),
                    fields[6],
                    "{line}"
                );
            }
        }
    }

    #[test]
    fn fixed_hash_states_match_node_at_every_split() {
        for line in FIXED_VECTORS.lines().filter(|s| s.starts_with("hash ")) {
            let fields: Vec<_> = line.split_whitespace().collect();
            let algorithm = Pbkdf2Hash::from_name(fields[1]).unwrap();
            let length: usize = fields[2].parse().unwrap();
            let expected = fields[3];
            let hlen = algorithm.sizes().1;
            if length == 1048576 {
                // Reuse 256 bytes: no whole-message fixture retained for this stream.
                let chunk = fixture_bytes(256);
                for size in [1, 64, 256] {
                    let mut state = HashState::new(algorithm);
                    for _ in 0..4096 {
                        for part in chunk.chunks(size) {
                            state.update(part);
                            state.update(&[]);
                        }
                    }
                    assert_eq!(
                        hex(&state.finish()[..hlen]),
                        expected,
                        "{line}, chunk {size}"
                    );
                }
                continue;
            }
            let input = fixture_bytes(length);
            assert_eq!(hex(&algorithm.digest(&input)), expected, "one-shot {line}");
            for split in 0..=length {
                let mut prefix = HashState::new(algorithm);
                prefix.update(&[]);
                prefix.update(&input[..split]);
                let mut fork = prefix.clone();
                prefix.update(&input[split..]);
                prefix.update(&[]);
                assert_eq!(
                    hex(&prefix.finish()[..hlen]),
                    expected,
                    "{line}, split {split}"
                );
                // A fork owns its tail and words, even when the original was finalized.
                for byte in &input[split..] {
                    fork.update(std::slice::from_ref(byte));
                }
                assert_eq!(
                    hex(&fork.finish()[..hlen]),
                    expected,
                    "fork {line}, split {split}"
                );
            }
        }
    }

    #[test]
    fn fixed_hash_length_accounting_wraps_explicitly() {
        for algorithm in [
            Pbkdf2Hash::Md5,
            Pbkdf2Hash::Sha1,
            Pbkdf2Hash::Sha256,
            Pbkdf2Hash::Sha384,
            Pbkdf2Hash::Sha512,
        ] {
            let mut state = HashState::new(algorithm);
            state.update(b"abc");
            let expected = state.clone().finish();
            // Adding 2^125 bytes leaves the low 128 length bits unchanged.
            state.bytes += 1u128 << 125;
            assert_eq!(state.finish(), expected);
            let mut state = HashState::new(algorithm);
            state.bytes = u128::MAX;
            state.update(&[]);
            assert_eq!(state.bytes, u128::MAX);
            state.update(b"a");
            assert_eq!(state.bytes, 0);
            if algorithm.sizes().0 == 64 {
                let mut state = HashState::new(algorithm);
                state.update(b"abc");
                state.bytes += 1u128 << 61;
                assert_eq!(state.finish(), expected);
            }
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn hashes_are_what_their_standards_say() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha512(b"abc")),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
    }

    #[test]
    fn a_message_right_at_the_edge_of_a_block_is_padded_properly() {
        // These lengths are where padding goes into another block.
        assert_eq!(
            hex(&sha256(&[b'a'; 55])),
            "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"
        );
        assert_eq!(
            hex(&sha256(&[b'a'; 56])),
            "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"
        );
    }

    #[test]
    fn pbkdf2_rfc6070_and_all_five_hashes() {
        for (rounds, expected) in [
            (1, "0c60c80f961f0e71f3a9b524af6012062fe037a6"),
            (2, "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957"),
            (4096, "4b007901b765489abead49d926f721d065a429c1"),
        ] {
            assert_eq!(
                hex(&pbkdf2(b"password", b"salt", rounds, 20, Pbkdf2Hash::Sha1).unwrap()),
                expected
            );
        }
        assert_eq!(
            hex(&pbkdf2(b"pass\0word", b"sa\0lt", 4096, 16, Pbkdf2Hash::Sha1).unwrap()),
            "56fa6aa75548099dcc37d7f03425e0c3"
        );
        // Independently verified with Node v24.16.0 / OpenSSL 3.5.6.
        for (algorithm, expected) in [
            (
                Pbkdf2Hash::Md5,
                "042407b552be345ad6eee2cf2f7ed01dd9662d8f0c6950eaec7124aa0c82279e",
            ),
            (
                Pbkdf2Hash::Sha1,
                "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957cae93136266537a8d7bf4b76",
            ),
            (
                Pbkdf2Hash::Sha256,
                "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43",
            ),
            (
                Pbkdf2Hash::Sha384,
                "54f775c6d790f21930459162fc535dbf04a939185127016a04176a0730c6f1f4",
            ),
            (
                Pbkdf2Hash::Sha512,
                "e1d9c16aa681708a45f5c7c4e215ceb66e011a2e9f0040713f18aefdb866d53c",
            ),
        ] {
            assert_eq!(
                hex(&pbkdf2(b"password", b"salt", 2, 32, algorithm).unwrap()),
                expected
            );
            assert!(pbkdf2(b"", b"", 1, 0, algorithm).unwrap().is_empty());
        }
    }

    #[test]
    fn pbkdf2_rejects_sizes_before_allocating() {
        assert!(pbkdf2(b"", b"", 0, 0, Pbkdf2Hash::Sha1).is_err());
        assert!(pbkdf2(b"", b"", 1, usize::MAX, Pbkdf2Hash::Sha1).is_err());
        assert!(pbkdf2(b"", b"", 1, i32::MAX as usize + 1, Pbkdf2Hash::Sha1).is_err());
    }

    #[test]
    fn pbkdf2_native_checks_arguments_without_coercion() {
        let mut engine = crate::Engine::new();
        let vm = engine.vm();
        vm.method(vm.global, "nativePbkdf2", 5, pbkdf2_native);
        engine
            .eval(
                r#"
            const bytes = new Uint8Array(0);
            let rejected = 0;
            for (const value of [0, -1, 1.5, NaN, Infinity, 2147483648, '2', null, undefined]) {
                try { nativePbkdf2(bytes, bytes, value, 0, 'sha256'); }
                catch (e) { rejected++; }
            }
            for (const value of [-1, 1.5, NaN, Infinity, 2147483648, '2', null, undefined]) {
                try { nativePbkdf2(bytes, bytes, 1, value, 'sha256'); }
                catch (e) { rejected++; }
            }
            for (const value of ['bogus', {}, null]) {
                try { nativePbkdf2(bytes, bytes, 1, 0, value); }
                catch (e) { rejected++; }
            }
            try { nativePbkdf2(null, bytes, 1, 0, 'sha256'); }
            catch (e) { rejected++; }
            if (rejected !== 21) throw new Error('native validation: ' + rejected);
            if (nativePbkdf2(bytes, bytes, 1, 0, 'SHA-256').length !== 0) throw new Error('zero');
        "#,
                "native-pbkdf2.js",
            )
            .unwrap();
    }
}
