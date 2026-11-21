//! A small AArch64 assembler: just the instructions arm64.rs's templates use, each a 32-bit word encoded by hand (the
//! Arm Architecture Reference Manual's A64 encodings).
//!
//! As in asm.rs, code goes into two buffers, `hot` for the usual way through each instruction and `cold` for what's
//! only for when something's unusual, which is put after it, and branches name labels in either, worked out when the
//! two are joined. A conditional branch reaches 1 MB either way, and an unconditional one 128 MB: a function whose
//! cold code is further than that from its hot code is assembled again with every conditional branch to a label
//! written as the opposite condition over an unconditional branch (`long`), which [`Asm::finish`] says it needs.
//!
//! 64-bit constants are made with `movz`/`movn` and `movk`, so code needs no literal pool, and a call to an address
//! outside the code (a helper, another function's code) goes through `x16`, so needs nothing filled in once the code's
//! place is known. `x16` and `x17` are the assembler's own: `x17` holds an offset too big for an instruction, and
//! templates must keep nothing in either.
// Some encodings are here for templates still to come.
#![allow(dead_code)]

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct R(pub(crate) u8);

pub(crate) const X0: R = R(0);
pub(crate) const X1: R = R(1);
pub(crate) const X2: R = R(2);
pub(crate) const X3: R = R(3);
pub(crate) const X4: R = R(4);
pub(crate) const X5: R = R(5);
pub(crate) const X6: R = R(6);
pub(crate) const X7: R = R(7);
pub(crate) const X8: R = R(8);
pub(crate) const X9: R = R(9);
pub(crate) const X10: R = R(10);
pub(crate) const X11: R = R(11);
pub(crate) const X12: R = R(12);
pub(crate) const X13: R = R(13);
pub(crate) const X14: R = R(14);
pub(crate) const X15: R = R(15);
/// The scratch registers the procedure call standard gives a linker's veneers, which here are the assembler's.
pub(crate) const X16: R = R(16);
pub(crate) const X17: R = R(17);
pub(crate) const X19: R = R(19);
pub(crate) const X20: R = R(20);
pub(crate) const X21: R = R(21);
pub(crate) const X22: R = R(22);
pub(crate) const X23: R = R(23);
pub(crate) const X24: R = R(24);
pub(crate) const X25: R = R(25);
pub(crate) const X26: R = R(26);
pub(crate) const X27: R = R(27);
pub(crate) const X28: R = R(28);
pub(crate) const FP: R = R(29);
pub(crate) const LR: R = R(30);
/// Register 31 is the zero register in most instructions and the stack pointer in the rest; which one each takes is
/// said where it matters.
pub(crate) const XZR: R = R(31);
pub(crate) const SP: R = R(31);

/// A floating-point register, used as a double (or, for `fcvt`, a single).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct D(pub(crate) u8);

pub(crate) const D0: D = D(0);
pub(crate) const D1: D = D(1);
pub(crate) const D2: D = D(2);
pub(crate) const D3: D = D(3);

/// A condition, as A64 numbers them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Cond(pub(crate) u8);

pub(crate) const EQ: Cond = Cond(0);
pub(crate) const NE: Cond = Cond(1);
/// Carry set: unsigned higher or same.
pub(crate) const HS: Cond = Cond(2);
/// Carry clear: unsigned lower.
pub(crate) const LO: Cond = Cond(3);
pub(crate) const MI: Cond = Cond(4);
pub(crate) const PL: Cond = Cond(5);
pub(crate) const VS: Cond = Cond(6);
pub(crate) const VC: Cond = Cond(7);
/// Unsigned higher.
pub(crate) const HI: Cond = Cond(8);
/// Unsigned lower or same.
pub(crate) const LS: Cond = Cond(9);
pub(crate) const GE: Cond = Cond(10);
pub(crate) const LT: Cond = Cond(11);
pub(crate) const GT: Cond = Cond(12);
pub(crate) const LE: Cond = Cond(13);

impl Cond {
    /// The opposite condition.
    pub(crate) fn not(self) -> Self {
        Self(self.0 ^ 1)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Label(u32);

/// What a fix-up fills in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fix {
    /// `b`/`bl`: 26 bits of words at bit 0.
    B26,
    /// `b.cond`, `cbz`, `cbnz`: 19 bits of words at bit 5.
    B19,
    /// `tbz`, `tbnz`: 14 bits of words at bit 5.
    B14,
    /// `adr`: 21 bits of bytes, split.
    Adr,
}

/// The size of a load or store, as the top two bits of its encoding have it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Size {
    B8 = 0,
    B16 = 1,
    B32 = 2,
    B64 = 3,
}

/// The double operations with two operands (`fadd` and the like), by their opcode field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Fop {
    Mul = 0,
    Div = 1,
    Add = 2,
    Sub = 3,
    Max = 4,
    Min = 5,
}

/// The double operations with one operand, by their encoding.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Fop1 {
    Abs = 0x1E60_C000,
    Neg = 0x1E61_4000,
    Sqrt = 0x1E61_C000,
    /// Towards minus infinity (`Math.floor`).
    RoundDown = 0x1E65_4000,
    /// Towards plus infinity (`Math.ceil`).
    RoundUp = 0x1E64_C000,
    /// Towards zero (`Math.trunc`).
    RoundZero = 0x1E65_C000,
}

pub(crate) struct Asm {
    hot: Vec<u32>,
    cold: Vec<u32>,
    in_cold: bool,
    /// Every conditional branch to a label as the opposite condition over a `b` (see the module's comment).
    long: bool,
    /// Where each label is: in the cold buffer or not, and how many words into it.
    labels: Vec<Option<(bool, u32)>>,
    /// Each branch to fill in: which buffer, which word, the label and the kind.
    fixups: Vec<(bool, u32, Label, Fix)>,
    /// How long the hot buffer was when the two were joined, in words, which is where the cold one starts.
    joined_at: usize,
    /// The `b` that's the last thing in each buffer (hot, cold), if it is: which word, its label and its fixup. One to
    /// the label bound right after it is taken out again (see `bind`).
    last_jump: [Option<(u32, Label, usize)>; 2],
}

/// The `N:immr:imms` of a logical instruction's immediate, if `value` is one it can have (a run of ones, rotated, and
/// repeated across the register in elements of 2 to 64 bits).
pub(crate) fn logical_imm(value: u64, wide: bool) -> Option<u32> {
    let value = if wide {
        value
    } else {
        let low = value & 0xFFFF_FFFF;
        low | (low << 32)
    };
    if value == 0 || value == u64::MAX {
        return None;
    }
    let mut size = 64u32;
    while size > 2 {
        let half = size / 2;
        let mask = (1u64 << half) - 1;
        if value & mask != (value >> half) & mask {
            break;
        }
        size = half;
    }
    let mask = if size == 64 { u64::MAX } else { (1u64 << size) - 1 };
    let element = value & mask;
    let ones = element.count_ones();
    let run = if ones == 64 { u64::MAX } else { (1u64 << ones) - 1 };
    let rotate_right = |x: u64, by: u32| {
        if by == 0 { x } else { ((x >> by) | (x << (size - by))) & mask }
    };
    let by = (0..size).find(|&by| rotate_right(element, by) == run)?;
    let immr = (size - by) % size;
    let imms = (!(2 * size - 1) & 0x3F) | (ones - 1);
    let n = u32::from(size == 64);
    Some((n << 12) | (immr << 6) | imms)
}

impl Asm {
    pub(crate) fn new(long: bool) -> Self {
        Self {
            hot: Vec::with_capacity(1024),
            cold: Vec::with_capacity(512),
            in_cold: false,
            long,
            labels: Vec::new(),
            fixups: Vec::new(),
            joined_at: 0,
            last_jump: [None, None],
        }
    }

    pub(crate) fn set_cold(&mut self, cold: bool) {
        self.in_cold = cold;
    }

    pub(crate) fn is_cold(&self) -> bool {
        self.in_cold
    }

    fn buf(&mut self) -> &mut Vec<u32> {
        if self.in_cold { &mut self.cold } else { &mut self.hot }
    }

    /// How many bytes there are so far, in both.
    pub(crate) fn len(&self) -> usize {
        4 * (self.hot.len() + self.cold.len())
    }

    // Called from everywhere a template emits code, which it would make much bigger if it were in each.
    #[inline(never)]
    pub(crate) fn emit(&mut self, word: u32) {
        self.buf().push(word);
    }

    #[inline(never)]
    pub(crate) fn label(&mut self) -> Label {
        self.labels.push(None);
        Label(self.labels.len() as u32 - 1)
    }

    /// Puts the label here.
    #[inline(never)]
    pub(crate) fn bind(&mut self, label: Label) {
        debug_assert!(self.labels[label.0 as usize].is_none(), "a label is bound once");
        let which = usize::from(self.in_cold);
        if let Some((at, to, fixup)) = self.last_jump[which].take()
            && to == label
            && at + 1 == self.buf().len() as u32
        {
            self.buf().pop();
            self.fixups[fixup].1 = u32::MAX;
        }
        let at = self.buf().len() as u32;
        self.labels[label.0 as usize] = Some((self.in_cold, at));
    }

    pub(crate) fn is_bound(&self, label: Label) -> bool {
        self.labels[label.0 as usize].is_some()
    }

    /// Where a label ended up in the joined code, in bytes, once it's been [`finish`](Self::finish)ed.
    pub(crate) fn offset_of(&self, label: Label) -> usize {
        4 * match self.labels[label.0 as usize] {
            Some((false, at)) => at as usize,
            Some((true, at)) => self.joined_at + at as usize,
            None => panic!("an unbound label"),
        }
    }

    #[inline(never)]
    fn fixup(&mut self, word: u32, label: Label, fix: Fix) {
        let at = self.buf().len() as u32;
        let cold = self.in_cold;
        self.fixups.push((cold, at, label, fix));
        self.emit(word);
    }

    /// Joins the buffers and fills in every branch: the code, or `None` if a branch couldn't reach its label (which
    /// assembling it again `long` mends, unless it's an unconditional one 128 MB away).
    pub(crate) fn finish(&mut self) -> Option<Vec<u8>> {
        let hot_len = self.hot.len();
        let mut words = std::mem::take(&mut self.hot);
        words.extend_from_slice(&self.cold);
        self.joined_at = hot_len;
        for &(cold, at, label, fix) in &self.fixups {
            if at == u32::MAX {
                continue;
            }
            let at = at as usize + if cold { hot_len } else { 0 };
            let target = match self.labels[label.0 as usize] {
                Some((false, to)) => to as usize,
                Some((true, to)) => hot_len + to as usize,
                None => panic!("a branch to an unbound label"),
            };
            let delta = target as i64 - at as i64;
            let fits = |bits: u32| delta >= -(1 << (bits - 1)) && delta < (1 << (bits - 1));
            words[at] |= match fix {
                Fix::B26 if fits(26) => (delta as u32) & 0x03FF_FFFF,
                Fix::B19 if fits(19) => ((delta as u32) & 0x7_FFFF) << 5,
                Fix::B14 if fits(14) => ((delta as u32) & 0x3FFF) << 5,
                Fix::Adr if fits(19) => {
                    let bytes = (delta * 4) as u32;
                    ((bytes & 3) << 29) | (((bytes >> 2) & 0x7_FFFF) << 5)
                }
                _ => return None,
            };
        }
        let mut code = Vec::with_capacity(words.len() * 4);
        for word in words {
            code.extend_from_slice(&word.to_le_bytes());
        }
        Some(code)
    }

    /// Fills in eight bytes already emitted.
    pub(crate) fn patch64(code: &mut [u8], at: usize, value: u64) {
        code[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    /// Eight bytes of data, in the code.
    pub(crate) fn data64(&mut self, value: u64) {
        self.emit(value as u32);
        self.emit((value >> 32) as u32);
    }

    // Moves.

    /// `mov dst, src`, 64 bits (neither the stack pointer).
    pub(crate) fn mov(&mut self, dst: R, src: R) {
        if dst != src {
            self.emit(0xAA00_03E0 | u32::from(src.0) << 16 | u32::from(dst.0));
        }
    }

    /// `mov wdst, wsrc`: the low 32 bits, the top ones cleared.
    pub(crate) fn mov32(&mut self, dst: R, src: R) {
        self.emit(0x2A00_03E0 | u32::from(src.0) << 16 | u32::from(dst.0));
    }

    /// `mov dst, sp`.
    pub(crate) fn mov_from_sp(&mut self, dst: R) {
        self.emit(0x9100_03E0 | u32::from(dst.0));
    }

    // Called from everywhere a template emits code, which it would make much bigger if it were in each.
    /// Any 64-bit constant, in as few `movz`/`movn` and `movk` as it takes.
    #[inline(never)]
    pub(crate) fn mov_imm64(&mut self, dst: R, value: u64) {
        let d = u32::from(dst.0);
        let chunks: [u32; 4] = std::array::from_fn(|at| ((value >> (16 * at)) & 0xFFFF) as u32);
        let zeros = chunks.iter().filter(|&&chunk| chunk == 0).count();
        let ones = chunks.iter().filter(|&&chunk| chunk == 0xFFFF).count();
        // As a 32-bit `orr` of a logical immediate, when that's one instruction and the rest would be more.
        if value <= u32::MAX as u64 && zeros < 3 && value != 0 && let Some(imm) = logical_imm(value, false) {
            self.emit(0x3200_03E0 | imm << 10 | d);
            return;
        }
        if ones > zeros {
            // `movn` with the first chunk that isn't all ones, then `movk` the others that aren't.
            let first = chunks.iter().position(|&chunk| chunk != 0xFFFF).unwrap_or(0);
            self.emit(0x9280_0000 | (first as u32) << 21 | ((!chunks[first]) & 0xFFFF) << 5 | d);
            for (at, &chunk) in chunks.iter().enumerate() {
                if at != first && chunk != 0xFFFF {
                    self.emit(0xF280_0000 | (at as u32) << 21 | chunk << 5 | d);
                }
            }
        } else {
            let first = chunks.iter().position(|&chunk| chunk != 0).unwrap_or(0);
            self.emit(0xD280_0000 | (first as u32) << 21 | chunks[first] << 5 | d);
            for (at, &chunk) in chunks.iter().enumerate() {
                if at != first && chunk != 0 {
                    self.emit(0xF280_0000 | (at as u32) << 21 | chunk << 5 | d);
                }
            }
        }
    }

    // Loads and stores. `base` may be the stack pointer.

    /// A load or store of `size` with `rt` at `[base + disp]`: one instruction with the offset in it if it fits,
    /// else with the offset in x17. `op` is the unsigned-offset encoding of the instruction.
    #[inline(never)]
    fn mem(&mut self, op: u32, size: u32, rt: u8, base: R, disp: i32) {
        let scale = 1i32 << size;
        let (rt, rn) = (u32::from(rt), u32::from(base.0));
        if disp >= 0 && disp % scale == 0 && disp / scale < 4096 {
            self.emit(op | ((disp / scale) as u32) << 10 | rn << 5 | rt);
        } else if (-256..256).contains(&disp) {
            // The unscaled form.
            self.emit((op & !0x0100_0000) | ((disp as u32) & 0x1FF) << 12 | rn << 5 | rt);
        } else {
            self.mov_imm64(X17, disp as i64 as u64);
            // The register-offset form, with the offset as it is.
            self.emit((op & !0x0100_0000) | 0x0020_0800 | 17 << 16 | 0b011 << 13 | rn << 5 | rt);
        }
    }

    /// A load or store of `size` with `rt` at `[base + index << size]` (`index` 64 bits), or `[base + index]` unless
    /// `scaled`.
    fn mem_index(&mut self, op: u32, rt: u8, base: R, index: R, scaled: bool) {
        self.emit(
            (op & !0x0100_0000)
                | 0x0020_0800
                | u32::from(index.0) << 16
                | 0b011 << 13
                | u32::from(scaled) << 12
                | u32::from(base.0) << 5
                | u32::from(rt),
        );
    }

    fn load_op(size: Size, signed: bool) -> u32 {
        match (size, signed) {
            (Size::B64, _) => 0xF940_0000,
            (Size::B32, _) => 0xB940_0000,
            (Size::B16, false) => 0x7940_0000,
            (Size::B16, true) => 0x79C0_0000,
            (Size::B8, false) => 0x3940_0000,
            (Size::B8, true) => 0x39C0_0000,
        }
    }

    fn store_op(size: Size) -> u32 {
        match size {
            Size::B64 => 0xF900_0000,
            Size::B32 => 0xB900_0000,
            Size::B16 => 0x7900_0000,
            Size::B8 => 0x3900_0000,
        }
    }

    /// `ldr dst, [base + disp]`, 64 bits.
    pub(crate) fn load(&mut self, dst: R, base: R, disp: i32) {
        self.mem(Self::load_op(Size::B64, false), 3, dst.0, base, disp);
    }

    /// A load of `size`, zero-extended (or, with `signed`, sign-extended to 32 bits).
    pub(crate) fn load_sized(&mut self, size: Size, signed: bool, dst: R, base: R, disp: i32) {
        self.mem(Self::load_op(size, signed), size as u32, dst.0, base, disp);
    }

    pub(crate) fn load32(&mut self, dst: R, base: R, disp: i32) {
        self.load_sized(Size::B32, false, dst, base, disp);
    }

    pub(crate) fn load16(&mut self, dst: R, base: R, disp: i32) {
        self.load_sized(Size::B16, false, dst, base, disp);
    }

    pub(crate) fn load8(&mut self, dst: R, base: R, disp: i32) {
        self.load_sized(Size::B8, false, dst, base, disp);
    }

    /// `str src, [base + disp]`, 64 bits (`src` may be the zero register).
    pub(crate) fn store(&mut self, src: R, base: R, disp: i32) {
        self.mem(Self::store_op(Size::B64), 3, src.0, base, disp);
    }

    pub(crate) fn store_sized(&mut self, size: Size, src: R, base: R, disp: i32) {
        self.mem(Self::store_op(size), size as u32, src.0, base, disp);
    }

    pub(crate) fn store32(&mut self, src: R, base: R, disp: i32) {
        self.store_sized(Size::B32, src, base, disp);
    }

    pub(crate) fn store16(&mut self, src: R, base: R, disp: i32) {
        self.store_sized(Size::B16, src, base, disp);
    }

    pub(crate) fn store8(&mut self, src: R, base: R, disp: i32) {
        self.store_sized(Size::B8, src, base, disp);
    }

    /// `ldr dst, [base, index, lsl #log2(size)]` (or unscaled), zero-extended.
    pub(crate) fn load_index(&mut self, size: Size, dst: R, base: R, index: R, scaled: bool) {
        self.mem_index(Self::load_op(size, false), dst.0, base, index, scaled);
    }

    /// The same, sign-extended to 32 bits.
    pub(crate) fn load_index_signed(&mut self, size: Size, dst: R, base: R, index: R, scaled: bool) {
        self.mem_index(Self::load_op(size, true), dst.0, base, index, scaled);
    }

    pub(crate) fn store_index(&mut self, size: Size, src: R, base: R, index: R, scaled: bool) {
        self.mem_index(Self::store_op(size), src.0, base, index, scaled);
    }

    /// `ldr ddst, [base + disp]`.
    pub(crate) fn load_d(&mut self, dst: D, base: R, disp: i32) {
        self.mem(0xFD40_0000, 3, dst.0, base, disp);
    }

    pub(crate) fn store_d(&mut self, src: D, base: R, disp: i32) {
        self.mem(0xFD00_0000, 3, src.0, base, disp);
    }

    /// `ldr ddst, [base, index, lsl #3]`; with `single`, `ldr sdst, [base, index, lsl #2]`.
    pub(crate) fn load_fp_index(&mut self, single: bool, dst: D, base: R, index: R) {
        self.mem_index(if single { 0xBD40_0000 } else { 0xFD40_0000 }, dst.0, base, index, true);
    }

    pub(crate) fn store_fp_index(&mut self, single: bool, src: D, base: R, index: R) {
        self.mem_index(if single { 0xBD00_0000 } else { 0xFD00_0000 }, src.0, base, index, true);
    }

    /// `stp a, b, [sp, #-16]!`.
    pub(crate) fn push_pair(&mut self, a: R, b: R) {
        self.emit(0xA9BF_0000 | u32::from(b.0) << 10 | 31 << 5 | u32::from(a.0));
    }

    /// `ldp a, b, [sp], #16`.
    pub(crate) fn pop_pair(&mut self, a: R, b: R) {
        self.emit(0xA8C1_0000 | u32::from(b.0) << 10 | 31 << 5 | u32::from(a.0));
    }

    // Arithmetic. In the register forms register 31 is the zero register; in the immediate ones, the stack pointer.

    #[inline(never)]
    fn add_sub_imm(&mut self, op: u32, wide: bool, dst: R, src: R, imm: u32) -> bool {
        let sf = if wide { 0x8000_0000 } else { 0 };
        let (rd, rn) = (u32::from(dst.0), u32::from(src.0));
        if imm < 4096 {
            self.emit(sf | op | imm << 10 | rn << 5 | rd);
            true
        } else if imm & 0xFFF == 0 && imm < 1 << 24 {
            self.emit(sf | op | 1 << 22 | (imm >> 12) << 10 | rn << 5 | rd);
            true
        } else {
            false
        }
    }

    /// `add dst, src, #imm` (or `sub` for a negative one), 64 bits, through x17 if it doesn't fit.
    pub(crate) fn add_imm(&mut self, dst: R, src: R, imm: i64) {
        self.add_imm_w(true, dst, src, imm);
    }

    pub(crate) fn add_imm32(&mut self, dst: R, src: R, imm: i64) {
        self.add_imm_w(false, dst, src, imm);
    }

    fn add_imm_w(&mut self, wide: bool, dst: R, src: R, imm: i64) {
        let (op, magnitude) = if imm >= 0 { (0x1100_0000, imm) } else { (0x5100_0000, -imm) };
        if magnitude <= u32::MAX as i64 && self.add_sub_imm(op, wide, dst, src, magnitude as u32) {
            return;
        }
        self.mov_imm64(X17, imm as u64);
        // The extended-register form, which takes the stack pointer.
        let sf = if wide { 0x8000_0000 } else { 0 };
        let option = if wide { 0b011 } else { 0b010 };
        self.emit(sf | 0x0B20_0000 | 17 << 16 | option << 13 | u32::from(src.0) << 5 | u32::from(dst.0));
    }

    /// `adds dst, src, #imm` (or `subs` for a negative one), setting the flags; 32 or 64 bits. `imm` has to fit.
    pub(crate) fn adds_imm(&mut self, wide: bool, dst: R, src: R, imm: i32) {
        let (op, magnitude) = if imm >= 0 { (0x3100_0000, imm as u32) } else { (0x7100_0000, imm.unsigned_abs()) };
        let fitted = self.add_sub_imm(op, wide, dst, src, magnitude);
        assert!(fitted, "an adds immediate that fits");
    }

    /// `cmp a, #imm`, 32 or 64 bits, with any immediate (through x17 if it has to be).
    #[inline(never)]
    pub(crate) fn cmp_imm(&mut self, wide: bool, a: R, imm: i64) {
        let fits = |value: i64| (0..4096).contains(&value) || (value & 0xFFF == 0 && (0..1 << 24).contains(&value));
        if fits(imm) {
            self.add_sub_imm(0x7100_0000, wide, XZR, a, imm as u32);
        } else if fits(-imm) {
            self.add_sub_imm(0x3100_0000, wide, XZR, a, (-imm) as u32);
        } else {
            self.mov_imm64(X17, imm as u64);
            self.cmp(wide, a, X17);
        }
    }

    #[inline(never)]
    fn shifted(&mut self, op: u32, wide: bool, dst: R, a: R, b: R, shift: u8, amount: u8) {
        let sf = if wide { 0x8000_0000 } else { 0 };
        self.emit(
            sf | op
                | u32::from(shift) << 22
                | u32::from(b.0) << 16
                | u32::from(amount) << 10
                | u32::from(a.0) << 5
                | u32::from(dst.0),
        );
    }

    pub(crate) fn add(&mut self, wide: bool, dst: R, a: R, b: R) {
        self.shifted(0x0B00_0000, wide, dst, a, b, 0, 0);
    }

    /// `add dst, a, b, lsl #amount`, 64 bits.
    pub(crate) fn add_lsl(&mut self, dst: R, a: R, b: R, amount: u8) {
        self.shifted(0x0B00_0000, true, dst, a, b, 0, amount);
    }

    /// `add dst, a, wb, uxtw #amount`: a 64-bit register and a 32-bit one zero-extended and shifted (0 to 4).
    pub(crate) fn add_uxtw(&mut self, dst: R, a: R, b: R, amount: u8) {
        self.emit(
            0x8B20_0000
                | u32::from(b.0) << 16
                | 0b010 << 13
                | u32::from(amount) << 10
                | u32::from(a.0) << 5
                | u32::from(dst.0),
        );
    }

    pub(crate) fn adds(&mut self, wide: bool, dst: R, a: R, b: R) {
        self.shifted(0x2B00_0000, wide, dst, a, b, 0, 0);
    }

    pub(crate) fn sub(&mut self, wide: bool, dst: R, a: R, b: R) {
        self.shifted(0x4B00_0000, wide, dst, a, b, 0, 0);
    }

    pub(crate) fn subs(&mut self, wide: bool, dst: R, a: R, b: R) {
        self.shifted(0x6B00_0000, wide, dst, a, b, 0, 0);
    }

    pub(crate) fn cmp(&mut self, wide: bool, a: R, b: R) {
        self.subs(wide, XZR, a, b);
    }

    /// `neg dst, src`, setting the flags (`negs`).
    pub(crate) fn negs(&mut self, wide: bool, dst: R, src: R) {
        self.subs(wide, dst, XZR, src);
    }

    pub(crate) fn and(&mut self, wide: bool, dst: R, a: R, b: R) {
        self.shifted(0x0A00_0000, wide, dst, a, b, 0, 0);
    }

    pub(crate) fn orr(&mut self, wide: bool, dst: R, a: R, b: R) {
        self.shifted(0x2A00_0000, wide, dst, a, b, 0, 0);
    }

    pub(crate) fn eor(&mut self, wide: bool, dst: R, a: R, b: R) {
        self.shifted(0x4A00_0000, wide, dst, a, b, 0, 0);
    }

    /// `orr dst, a, b, lsr #amount`, 64 bits.
    pub(crate) fn orr_lsr(&mut self, dst: R, a: R, b: R, amount: u8) {
        self.shifted(0x2A00_0000, true, dst, a, b, 1, amount);
    }

    pub(crate) fn tst(&mut self, wide: bool, a: R, b: R) {
        self.shifted(0x6A00_0000, wide, XZR, a, b, 0, 0);
    }

    /// `mvn dst, src`.
    pub(crate) fn mvn(&mut self, wide: bool, dst: R, src: R) {
        self.shifted(0x2A20_0000, wide, dst, XZR, src, 0, 0);
    }

    /// A logical operation with an immediate (`op` its 32-bit encoding without the immediate), through x17 if the
    /// immediate can't be encoded.
    #[inline(never)]
    fn logical_imm_op(&mut self, op: u32, reg_op: u32, wide: bool, dst: R, src: R, imm: u64) {
        let sf = if wide { 0x8000_0000 } else { 0 };
        match logical_imm(imm, wide) {
            Some(fields) => self.emit(sf | op | fields << 10 | u32::from(src.0) << 5 | u32::from(dst.0)),
            None => {
                self.mov_imm64(X17, imm);
                self.shifted(reg_op, wide, dst, src, X17, 0, 0);
            }
        }
    }

    pub(crate) fn and_imm(&mut self, wide: bool, dst: R, src: R, imm: u64) {
        self.logical_imm_op(0x1200_0000, 0x0A00_0000, wide, dst, src, imm);
    }

    pub(crate) fn orr_imm(&mut self, wide: bool, dst: R, src: R, imm: u64) {
        self.logical_imm_op(0x3200_0000, 0x2A00_0000, wide, dst, src, imm);
    }

    pub(crate) fn eor_imm(&mut self, wide: bool, dst: R, src: R, imm: u64) {
        self.logical_imm_op(0x5200_0000, 0x4A00_0000, wide, dst, src, imm);
    }

    pub(crate) fn tst_imm(&mut self, wide: bool, src: R, imm: u64) {
        self.logical_imm_op(0x7200_0000, 0x6A00_0000, wide, XZR, src, imm);
    }

    /// `lsl dst, src, #by`.
    pub(crate) fn lsl_imm(&mut self, wide: bool, dst: R, src: R, by: u8) {
        let bits = if wide { 64 } else { 32 };
        let by = u32::from(by) % bits;
        let op = if wide { 0xD340_0000 } else { 0x5300_0000 };
        let immr = (bits - by) % bits;
        let imms = bits - 1 - by;
        self.emit(op | immr << 16 | imms << 10 | u32::from(src.0) << 5 | u32::from(dst.0));
    }

    /// `lsr dst, src, #by`.
    pub(crate) fn lsr_imm(&mut self, wide: bool, dst: R, src: R, by: u8) {
        let (op, top) = if wide { (0xD340_0000, 63) } else { (0x5300_0000, 31) };
        self.emit(op | u32::from(by) << 16 | top << 10 | u32::from(src.0) << 5 | u32::from(dst.0));
    }

    /// `asr dst, src, #by`.
    pub(crate) fn asr_imm(&mut self, wide: bool, dst: R, src: R, by: u8) {
        let (op, top) = if wide { (0x9340_0000, 63) } else { (0x1300_0000, 31) };
        self.emit(op | u32::from(by) << 16 | top << 10 | u32::from(src.0) << 5 | u32::from(dst.0));
    }

    /// `sxtw dst, wsrc`.
    pub(crate) fn sxtw(&mut self, dst: R, src: R) {
        self.emit(0x9340_7C00 | u32::from(src.0) << 5 | u32::from(dst.0));
    }

    /// `lslv`, `lsrv`, `asrv`: a shift by a register, which takes its low 5 (or 6) bits.
    fn shift_reg(&mut self, op: u32, wide: bool, dst: R, src: R, by: R) {
        let sf = if wide { 0x8000_0000 } else { 0 };
        self.emit(sf | op | u32::from(by.0) << 16 | u32::from(src.0) << 5 | u32::from(dst.0));
    }

    pub(crate) fn lslv(&mut self, wide: bool, dst: R, src: R, by: R) {
        self.shift_reg(0x1AC0_2000, wide, dst, src, by);
    }

    pub(crate) fn lsrv(&mut self, wide: bool, dst: R, src: R, by: R) {
        self.shift_reg(0x1AC0_2400, wide, dst, src, by);
    }

    pub(crate) fn asrv(&mut self, wide: bool, dst: R, src: R, by: R) {
        self.shift_reg(0x1AC0_2800, wide, dst, src, by);
    }

    pub(crate) fn sdiv(&mut self, wide: bool, dst: R, a: R, b: R) {
        self.shift_reg(0x1AC0_0C00, wide, dst, a, b);
    }

    pub(crate) fn mul(&mut self, wide: bool, dst: R, a: R, b: R) {
        let sf = if wide { 0x8000_0000 } else { 0 };
        self.emit(sf | 0x1B00_7C00 | u32::from(b.0) << 16 | u32::from(a.0) << 5 | u32::from(dst.0));
    }

    /// `msub dst, a, b, c`: `c - a * b`.
    pub(crate) fn msub(&mut self, wide: bool, dst: R, a: R, b: R, c: R) {
        let sf = if wide { 0x8000_0000 } else { 0 };
        self.emit(
            sf | 0x1B00_8000 | u32::from(b.0) << 16 | u32::from(c.0) << 10 | u32::from(a.0) << 5 | u32::from(dst.0),
        );
    }

    /// `smull dst, wa, wb`: the 64-bit product of two 32-bit ones.
    pub(crate) fn smull(&mut self, dst: R, a: R, b: R) {
        self.emit(0x9B20_7C00 | u32::from(b.0) << 16 | u32::from(a.0) << 5 | u32::from(dst.0));
    }

    /// `csel dst, a, b, cond`: `a` if `cond`, else `b`.
    pub(crate) fn csel(&mut self, wide: bool, dst: R, a: R, b: R, cond: Cond) {
        let sf = if wide { 0x8000_0000 } else { 0 };
        self.emit(
            sf | 0x1A80_0000 | u32::from(b.0) << 16 | u32::from(cond.0) << 12 | u32::from(a.0) << 5 | u32::from(dst.0),
        );
    }

    /// `cset dst, cond`: 1 if `cond`, else 0.
    pub(crate) fn cset(&mut self, dst: R, cond: Cond) {
        self.emit(0x9A9F_07E0 | u32::from(cond.not().0) << 12 | u32::from(dst.0));
    }

    /// `csneg dst, a, b, cond`: `a` if `cond`, else `-b`.
    pub(crate) fn csneg(&mut self, wide: bool, dst: R, a: R, b: R, cond: Cond) {
        let sf = if wide { 0x8000_0000 } else { 0 };
        self.emit(
            sf | 0x5A80_0400 | u32::from(b.0) << 16 | u32::from(cond.0) << 12 | u32::from(a.0) << 5 | u32::from(dst.0),
        );
    }

    // Branches.

    /// `b label`.
    #[inline(never)]
    pub(crate) fn b(&mut self, label: Label) {
        let at = self.buf().len() as u32;
        self.fixup(0x1400_0000, label, Fix::B26);
        self.last_jump[usize::from(self.in_cold)] = Some((at, label, self.fixups.len() - 1));
    }

    /// `bl label`.
    pub(crate) fn bl(&mut self, label: Label) {
        self.fixup(0x9400_0000, label, Fix::B26);
    }

    /// A short conditional branch (`word` with no offset) to a label, or with `long`, the opposite one over a `b`.
    #[inline(never)]
    fn branch_short(&mut self, word: u32, inverted: u32, fix: Fix, label: Label) {
        if self.long {
            // Over the `b` that follows: two words on.
            let skip = match fix {
                Fix::B14 => 2 << 5,
                _ => 2 << 5,
            };
            self.emit(inverted | skip);
            self.fixup(0x1400_0000, label, Fix::B26);
        } else {
            self.fixup(word, label, fix);
        }
    }

    /// `b.cond label`.
    pub(crate) fn bcond(&mut self, cond: Cond, label: Label) {
        self.branch_short(0x5400_0000 | u32::from(cond.0), 0x5400_0000 | u32::from(cond.not().0), Fix::B19, label);
    }

    /// `cbz` (or with `nonzero`, `cbnz`) `r`, `label`; 32 or 64 bits.
    pub(crate) fn cbz(&mut self, wide: bool, nonzero: bool, r: R, label: Label) {
        let sf = if wide { 0x8000_0000 } else { 0 };
        let word = |nonzero: bool| sf | if nonzero { 0x3500_0000 } else { 0x3400_0000 } | u32::from(r.0);
        self.branch_short(word(nonzero), word(!nonzero), Fix::B19, label);
    }

    /// `tbz` (or with `set`, `tbnz`) `r`, `#bit`, `label`.
    pub(crate) fn tbz(&mut self, set: bool, r: R, bit: u8, label: Label) {
        let word = |set: bool| {
            (if set { 0x3700_0000 } else { 0x3600_0000 })
                | u32::from(bit >> 5) << 31
                | u32::from(bit & 31) << 19
                | u32::from(r.0)
        };
        self.branch_short(word(set), word(!set), Fix::B14, label);
    }

    /// `adr dst, label`: where a label is.
    pub(crate) fn adr(&mut self, dst: R, label: Label) {
        self.fixup(0x1000_0000 | u32::from(dst.0), label, Fix::Adr);
    }

    pub(crate) fn br(&mut self, r: R) {
        self.emit(0xD61F_0000 | u32::from(r.0) << 5);
    }

    pub(crate) fn blr(&mut self, r: R) {
        self.emit(0xD63F_0000 | u32::from(r.0) << 5);
    }

    pub(crate) fn ret(&mut self) {
        self.emit(0xD65F_03C0);
    }

    pub(crate) fn nop(&mut self) {
        self.emit(0xD503_201F);
    }

    pub(crate) fn brk(&mut self) {
        self.emit(0xD420_0000);
    }

    /// A call to the code at `target`, through x16.
    pub(crate) fn call_abs(&mut self, target: u64) {
        self.mov_imm64(X16, target);
        self.blr(X16);
    }

    // Floating point.

    /// `fmov ddst, xsrc`.
    pub(crate) fn fmov_to_d(&mut self, dst: D, src: R) {
        self.emit(0x9E67_0000 | u32::from(src.0) << 5 | u32::from(dst.0));
    }

    /// `fmov xdst, dsrc`.
    pub(crate) fn fmov_from_d(&mut self, dst: R, src: D) {
        self.emit(0x9E66_0000 | u32::from(src.0) << 5 | u32::from(dst.0));
    }

    /// `fmov ddst, dsrc`.
    pub(crate) fn fmov_dd(&mut self, dst: D, src: D) {
        if dst != src {
            self.emit(0x1E60_4000 | u32::from(src.0) << 5 | u32::from(dst.0));
        }
    }

    pub(crate) fn fop(&mut self, op: Fop, dst: D, a: D, b: D) {
        self.emit(0x1E60_0800 | (op as u32) << 12 | u32::from(b.0) << 16 | u32::from(a.0) << 5 | u32::from(dst.0));
    }

    pub(crate) fn fop1(&mut self, op: Fop1, dst: D, src: D) {
        self.emit(op as u32 | u32::from(src.0) << 5 | u32::from(dst.0));
    }

    /// `fcmp a, b`: equal sets Z and C, less N, greater C, and unordered C and V.
    pub(crate) fn fcmp(&mut self, a: D, b: D) {
        self.emit(0x1E60_2000 | u32::from(b.0) << 16 | u32::from(a.0) << 5);
    }

    /// `scvtf ddst, wsrc` (or `xsrc` with `wide`).
    pub(crate) fn scvtf(&mut self, wide: bool, dst: D, src: R) {
        let sf = if wide { 0x8000_0000 } else { 0 };
        self.emit(sf | 0x1E62_0000 | u32::from(src.0) << 5 | u32::from(dst.0));
    }

    /// `ucvtf ddst, wsrc` (or `xsrc` with `wide`).
    pub(crate) fn ucvtf(&mut self, wide: bool, dst: D, src: R) {
        let sf = if wide { 0x8000_0000 } else { 0 };
        self.emit(sf | 0x1E63_0000 | u32::from(src.0) << 5 | u32::from(dst.0));
    }

    /// `fcvtzs wdst, dsrc` (or `xdst` with `wide`): towards zero, saturating, and NaN as 0.
    pub(crate) fn fcvtzs(&mut self, wide: bool, dst: R, src: D) {
        let sf = if wide { 0x8000_0000 } else { 0 };
        self.emit(sf | 0x1E78_0000 | u32::from(src.0) << 5 | u32::from(dst.0));
    }

    /// `fcvt sdst, dsrc`.
    pub(crate) fn fcvt_to_single(&mut self, dst: D, src: D) {
        self.emit(0x1E62_4000 | u32::from(src.0) << 5 | u32::from(dst.0));
    }

    /// `fcvt ddst, ssrc`.
    pub(crate) fn fcvt_to_double(&mut self, dst: D, src: D) {
        self.emit(0x1E22_C000 | u32::from(src.0) << 5 | u32::from(dst.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(f: impl FnOnce(&mut Asm)) -> Vec<u32> {
        let mut a = Asm::new(false);
        f(&mut a);
        let code = a.finish().expect("the branches reach");
        code.chunks(4).map(|word| u32::from_le_bytes(word.try_into().unwrap())).collect()
    }

    #[test]
    fn instructions_are_encoded_as_the_manual_says() {
        // What an assembler makes of each (checked against GNU as).
        assert_eq!(words(|a| a.load(X0, X19, 8)), [0xF940_0660]); // ldr x0, [x19, #8]
        assert_eq!(words(|a| a.load(X0, X19, -8)), [0xF85F_8260]); // ldur x0, [x19, #-8]
        assert_eq!(words(|a| a.load32(X1, X20, 4)), [0xB940_0681]); // ldr w1, [x20, #4]
        assert_eq!(words(|a| a.load8(X2, X21, 3)), [0x3940_0EA2]); // ldrb w2, [x21, #3]
        assert_eq!(words(|a| a.store(X24, X19, 16)), [0xF900_0A78]); // str x24, [x19, #16]
        assert_eq!(words(|a| a.store16(X4, X20, 2)), [0x7900_0684]); // strh w4, [x20, #2]
        assert_eq!(words(|a| a.load_index(Size::B64, X0, X26, X1, true)), [0xF861_7B40]); // ldr x0, [x26, x1, lsl #3]
        assert_eq!(words(|a| a.mov(X0, X21)), [0xAA15_03E0]); // mov x0, x21
        assert_eq!(words(|a| a.mov32(X1, X1)), [0x2A01_03E1]); // mov w1, w1
        assert_eq!(words(|a| a.mov_imm64(X22, 0xFFF9_0000_0000_0000)), [0xD2FF_FF36]); // movz x22, #0xfff9, lsl #48
        assert_eq!(words(|a| a.mov_imm64(X0, 7)), [0xD280_00E0]); // movz x0, #7
        assert_eq!(words(|a| a.mov_imm64(X0, 0x00FF_00FF)), [0x3200_9FE0]); // orr w0, wzr, #0xff00ff
        assert_eq!(words(|a| a.mov_imm64(X0, u64::MAX - 1)), [0x9280_0020]); // movn x0, #1
        assert_eq!(words(|a| a.mov_imm64(X1, 0x1234_5678)), [0xD28A_CF01, 0xF2A2_4681]); // movz, movk lsl 16
        assert_eq!(words(|a| a.add_imm(X0, X0, 1)), [0x9100_0400]); // add x0, x0, #1
        assert_eq!(words(|a| a.add_imm(SP, SP, -16)), [0xD100_43FF]); // sub sp, sp, #16
        assert_eq!(words(|a| a.adds_imm(false, X0, X0, 1)), [0x3100_0400]); // adds w0, w0, #1
        assert_eq!(words(|a| a.cmp_imm(false, X1, 5)), [0x7100_143F]); // cmp w1, #5
        assert_eq!(words(|a| a.cmp_imm(true, X1, -5)), [0xB100_143F]); // cmn x1, #5
        assert_eq!(words(|a| a.adds(false, X2, X0, X1)), [0x2B01_0002]); // adds w2, w0, w1
        assert_eq!(words(|a| a.subs(false, X2, X0, X1)), [0x6B01_0002]); // subs w2, w0, w1
        assert_eq!(words(|a| a.cmp(true, X0, X1)), [0xEB01_001F]); // cmp x0, x1
        assert_eq!(words(|a| a.orr(true, X0, X0, X22)), [0xAA16_0000]); // orr x0, x0, x22
        assert_eq!(words(|a| a.eor(true, X9, X0, X22)), [0xCA16_0009]); // eor x9, x0, x22
        assert_eq!(words(|a| a.add_uxtw(X0, X19, X1, 3)), [0x8B21_4E60]); // add x0, x19, w1, uxtw #3
        assert_eq!(words(|a| a.add_lsl(X0, X25, X1, 6)), [0x8B01_1B20]); // add x0, x25, x1, lsl #6
        assert_eq!(words(|a| a.and_imm(true, X0, X0, !1)), [0x927F_F800]); // and x0, x0, #0xfffffffffffffffe
        assert_eq!(words(|a| a.orr_imm(true, X0, X0, 1)), [0xB240_0000]); // orr x0, x0, #1
        assert_eq!(words(|a| a.tst_imm(false, X0, 0xFF)), [0x7200_1C1F]); // tst w0, #0xff
        assert_eq!(words(|a| a.lsr_imm(true, X9, X0, 48)), [0xD370_FC09]); // lsr x9, x0, #48
        assert_eq!(words(|a| a.lsl_imm(true, X0, X0, 6)), [0xD37A_E400]); // lsl x0, x0, #6
        assert_eq!(words(|a| a.asr_imm(false, X0, X0, 2)), [0x1302_7C00]); // asr w0, w0, #2
        assert_eq!(words(|a| a.lslv(false, X0, X0, X1)), [0x1AC1_2000]); // lsl w0, w0, w1
        assert_eq!(words(|a| a.smull(X2, X0, X1)), [0x9B21_7C02]); // smull x2, w0, w1
        assert_eq!(words(|a| a.sdiv(false, X2, X0, X1)), [0x1AC1_0C02]); // sdiv w2, w0, w1
        assert_eq!(words(|a| a.msub(false, X0, X2, X1, X0)), [0x1B01_8040]); // msub w0, w2, w1, w0
        assert_eq!(words(|a| a.csel(true, X0, X1, X2, EQ)), [0x9A82_0020]); // csel x0, x1, x2, eq
        assert_eq!(words(|a| a.cset(X0, LT)), [0x9A9F_A7E0]); // cset x0, lt
        assert_eq!(words(|a| a.push_pair(FP, LR)), [0xA9BF_7BFD]); // stp x29, x30, [sp, #-16]!
        assert_eq!(words(|a| a.pop_pair(FP, LR)), [0xA8C1_7BFD]); // ldp x29, x30, [sp], #16
        assert_eq!(words(|a| a.blr(X16)), [0xD63F_0200]); // blr x16
        assert_eq!(words(|a| a.ret()), [0xD65F_03C0]);
        assert_eq!(words(|a| a.mov_from_sp(X16)), [0x9100_03F0]); // mov x16, sp
        assert_eq!(words(|a| a.fmov_to_d(D0, X0)), [0x9E67_0000]); // fmov d0, x0
        assert_eq!(words(|a| a.fmov_from_d(X0, D1)), [0x9E66_0020]); // fmov x0, d1
        assert_eq!(words(|a| a.fop(Fop::Add, D0, D0, D1)), [0x1E61_2800]); // fadd d0, d0, d1
        assert_eq!(words(|a| a.fop(Fop::Div, D0, D0, D1)), [0x1E61_1800]); // fdiv d0, d0, d1
        assert_eq!(words(|a| a.fop(Fop::Min, D0, D0, D1)), [0x1E61_5800]); // fmin d0, d0, d1
        assert_eq!(words(|a| a.fop1(Fop1::Sqrt, D0, D0)), [0x1E61_C000]); // fsqrt d0, d0
        assert_eq!(words(|a| a.fop1(Fop1::RoundDown, D0, D1)), [0x1E65_4020]); // frintm d0, d1
        assert_eq!(words(|a| a.fcmp(D0, D1)), [0x1E61_2000]); // fcmp d0, d1
        assert_eq!(words(|a| a.scvtf(false, D1, X1)), [0x1E62_0021]); // scvtf d1, w1
        assert_eq!(words(|a| a.fcvtzs(false, X0, D0)), [0x1E78_0000]); // fcvtzs w0, d0
        assert_eq!(words(|a| a.fcvtzs(true, X0, D0)), [0x9E78_0000]); // fcvtzs x0, d0
        assert_eq!(words(|a| a.fcvt_to_single(D0, D0)), [0x1E62_4000]); // fcvt s0, d0
        assert_eq!(words(|a| a.fcvt_to_double(D0, D0)), [0x1E22_C000]); // fcvt d0, s0
        assert_eq!(words(|a| a.load_fp_index(true, D0, X11, X1)), [0xBC61_7960]); // ldr s0, [x11, x1, lsl #2]
    }

    #[test]
    fn logical_immediates_are_found_for_the_patterns_that_have_them() {
        assert_eq!(logical_imm(1, true), Some(0x1000)); // N=1, immr=0, imms=0
        assert_eq!(logical_imm(0xFFFF_FFFF, true), Some(0x101F));
        assert_eq!(logical_imm(0x5555_5555_5555_5555, true), Some(0x03C));
        assert_eq!(logical_imm(0, true), None);
        assert_eq!(logical_imm(0x1234, true), None);
        assert!(logical_imm(1 << 63, true).is_some());
        assert!(logical_imm(0xFFF0_0000_0000_0000, true).is_some());
    }

    #[test]
    fn branches_reach_their_labels_across_buffers_and_far_ones_go_long() {
        let mut a = Asm::new(false);
        let there = a.label();
        a.bcond(NE, there);
        a.set_cold(true);
        a.bind(there);
        a.ret();
        a.set_cold(false);
        a.ret();
        let code = a.finish().unwrap();
        // b.ne +2 (over the hot ret), the hot ret, then the cold ret.
        assert_eq!(&code[0..4], &(0x5400_0041u32).to_le_bytes());
        // A conditional branch more than 1 MB away doesn't reach, and does when it's long.
        for long in [false, true] {
            let mut a = Asm::new(long);
            let far = a.label();
            a.cbz(true, false, X0, far);
            for _ in 0..(1 << 18) + 4 {
                a.nop();
            }
            a.bind(far);
            a.ret();
            assert_eq!(a.finish().is_some(), long);
        }
        // A `b` to the label right after it goes.
        assert_eq!(
            words(|a| {
                let next = a.label();
                a.b(next);
                a.bind(next);
                a.ret();
            }),
            [0xD65F_03C0]
        );
    }
}
