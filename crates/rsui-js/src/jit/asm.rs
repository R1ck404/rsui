//! A small x86-64 assembler: just the instructions the JIT's templates use, encoded by hand.
//!
//! Code goes into one of two buffers: the usual way through each instruction goes in `hot`, and what's only for when
//! something's unusual (a slow path, a bail-out) goes in `cold`, which is put after it. Jumps name labels, which can
//! be in either, and are worked out when the two are joined.
// Some encodings are here for templates still to come.
#![allow(dead_code)]

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct R(pub(crate) u8);

pub(crate) const RAX: R = R(0);
pub(crate) const RCX: R = R(1);
pub(crate) const RDX: R = R(2);
pub(crate) const RBX: R = R(3);
pub(crate) const RSP: R = R(4);
pub(crate) const RBP: R = R(5);
pub(crate) const RSI: R = R(6);
pub(crate) const RDI: R = R(7);
pub(crate) const R8: R = R(8);
pub(crate) const R9: R = R(9);
pub(crate) const R10: R = R(10);
pub(crate) const R11: R = R(11);
pub(crate) const R12: R = R(12);
pub(crate) const R13: R = R(13);
pub(crate) const R14: R = R(14);
pub(crate) const R15: R = R(15);

/// An SSE register.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct X(pub(crate) u8);

pub(crate) const XMM0: X = X(0);
pub(crate) const XMM1: X = X(1);
pub(crate) const XMM2: X = X(2);

/// A memory operand: `[base + index * 2^scale + disp]`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mem {
    base: R,
    index: Option<(R, u8)>,
    disp: i32,
}

/// `[base + disp]`.
pub(crate) fn m(base: R, disp: i32) -> Mem {
    Mem {
        base,
        index: None,
        disp,
    }
}

/// `[base + index * 2^scale + disp]`.
pub(crate) fn mi(base: R, index: R, scale: u8, disp: i32) -> Mem {
    debug_assert!(index != RSP && scale <= 3);
    Mem {
        base,
        index: Some((index, scale)),
        disp,
    }
}

/// The condition codes, as the low nibble of `jcc`, `setcc` and `cmovcc`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Cc(pub(crate) u8);

pub(crate) const O: Cc = Cc(0x0);
pub(crate) const NO: Cc = Cc(0x1);
pub(crate) const B: Cc = Cc(0x2);
pub(crate) const AE: Cc = Cc(0x3);
pub(crate) const E: Cc = Cc(0x4);
pub(crate) const NE: Cc = Cc(0x5);
pub(crate) const BE: Cc = Cc(0x6);
pub(crate) const A: Cc = Cc(0x7);
pub(crate) const S: Cc = Cc(0x8);
pub(crate) const NS: Cc = Cc(0x9);
pub(crate) const P: Cc = Cc(0xA);
pub(crate) const NP: Cc = Cc(0xB);
pub(crate) const L: Cc = Cc(0xC);
pub(crate) const GE: Cc = Cc(0xD);
pub(crate) const LE: Cc = Cc(0xE);
pub(crate) const G: Cc = Cc(0xF);

impl Cc {
    /// The opposite condition.
    pub(crate) fn not(self) -> Self {
        Self(self.0 ^ 1)
    }
}

/// The arithmetic group that shares one encoding, by its number in it.
#[derive(Clone, Copy)]
pub(crate) enum Alu {
    Add = 0,
    Or = 1,
    And = 4,
    Sub = 5,
    Xor = 6,
    Cmp = 7,
}

/// Shifts, by their number in the group.
#[derive(Clone, Copy)]
pub(crate) enum Shift {
    Shl = 4,
    Shr = 5,
    Sar = 7,
}

/// SSE2 scalar double operations (all `F2 0F xx`).
#[derive(Clone, Copy)]
pub(crate) enum Sd {
    Sqrt = 0x51,
    Add = 0x58,
    Mul = 0x59,
    Sub = 0x5C,
    Min = 0x5D,
    Div = 0x5E,
    Max = 0x5F,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Label(u32);

enum Rm {
    R(u8),
    M(Mem),
}

pub(crate) struct Asm {
    hot: Vec<u8>,
    cold: Vec<u8>,
    in_cold: bool,
    /// Where each label is: in the cold buffer or not, and how far into it.
    labels: Vec<Option<(bool, u32)>>,
    /// Each rel32 to fill in: which buffer, where the four bytes are, and the label they go to.
    fixups: Vec<(bool, u32, Label)>,
    /// Each rel32 of a call to an address outside this code (another function's): which buffer, where the four bytes
    /// are, and the address, filled in once where the code goes is known (see [`Asm::place`]).
    calls: Vec<(bool, u32, u64)>,
    /// How long the hot buffer was when the two were joined, which is where the cold one starts.
    joined_at: usize,
    /// The jump that's the last thing in each buffer (hot, cold), if it is: where it starts, how long it is, the
    /// label it goes to and its fixup. A jump to the label bound right after it is taken out again (see `bind`), as
    /// templates often end their usual way with a jump past their slow paths, which went to the other buffer.
    last_jump: [Option<(u32, u32, Label, usize)>; 2],
}

impl Asm {
    pub(crate) fn new() -> Self {
        Self {
            hot: Vec::with_capacity(4096),
            cold: Vec::with_capacity(2048),
            in_cold: false,
            labels: Vec::new(),
            fixups: Vec::new(),
            calls: Vec::new(),
            last_jump: [None, None],
            joined_at: 0,
        }
    }

    /// Whether what's emitted now goes to the cold buffer.
    pub(crate) fn set_cold(&mut self, cold: bool) {
        self.in_cold = cold;
    }

    pub(crate) fn is_cold(&self) -> bool {
        self.in_cold
    }

    fn buf(&mut self) -> &mut Vec<u8> {
        if self.in_cold {
            &mut self.cold
        } else {
            &mut self.hot
        }
    }

    /// How many bytes there are so far, in both.
    pub(crate) fn len(&self) -> usize {
        self.hot.len() + self.cold.len()
    }

    /// Where the hot buffer is up to.
    pub(crate) fn hot_pos(&self) -> usize {
        self.hot.len()
    }

    fn byte(&mut self, b: u8) {
        self.buf().push(b);
    }

    fn bytes(&mut self, b: &[u8]) {
        self.buf().extend_from_slice(b);
    }

    fn d32(&mut self, v: i32) {
        self.bytes(&v.to_le_bytes());
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
        if let Some((start, length, to, fixup)) = self.last_jump[which].take()
            && to == label
            && start + length == self.buf().len() as u32
        {
            self.buf().truncate(start as usize);
            self.fixups[fixup].1 = u32::MAX;
        }
        let at = self.buf().len() as u32;
        self.labels[label.0 as usize] = Some((self.in_cold, at));
    }

    pub(crate) fn is_bound(&self, label: Label) -> bool {
        self.labels[label.0 as usize].is_some()
    }

    /// Where a label ended up in the joined code, once it's been [`finish`](Self::finish)ed.
    pub(crate) fn offset_of(&self, label: Label) -> usize {
        match self.labels[label.0 as usize] {
            Some((false, at)) => at as usize,
            Some((true, at)) => self.joined_at + at as usize,
            None => panic!("an unbound label"),
        }
    }

    // Called from everywhere a template emits code, which it would make much bigger if it were in each.
    #[inline(never)]
    fn rel32(&mut self, label: Label) {
        let at = self.buf().len() as u32;
        let cold = self.in_cold;
        self.fixups.push((cold, at, label));
        self.d32(0);
    }

    /// Joins the buffers and fills in every jump.
    pub(crate) fn finish(&mut self) -> Vec<u8> {
        let mut code = std::mem::take(&mut self.hot);
        let hot_len = code.len();
        code.extend_from_slice(&self.cold);
        for &(cold, at, label) in &self.fixups {
            // A jump taken out by `bind`.
            if at == u32::MAX {
                continue;
            }
            let at = at as usize + if cold { hot_len } else { 0 };
            let target = match self.labels[label.0 as usize] {
                Some((false, to)) => to as usize,
                Some((true, to)) => hot_len + to as usize,
                None => panic!("a jump to an unbound label"),
            };
            let rel = target as i64 - (at as i64 + 4);
            code[at..at + 4].copy_from_slice(&(rel as i32).to_le_bytes());
        }
        self.joined_at = hot_len;
        code
    }

    /// Fills in the calls to other code in what [`finish`](Self::finish) joined, which is to go at `address`. Each has
    /// to be within 2 GB of it, as everything compiled is.
    pub(crate) fn place(&self, code: &mut [u8], address: u64) {
        for &(cold, at, target) in &self.calls {
            let at = at as usize + if cold { self.joined_at } else { 0 };
            let rel = target as i64 - (address as i64 + at as i64 + 4);
            debug_assert!(i32::try_from(rel).is_ok(), "a call too far");
            code[at..at + 4].copy_from_slice(&(rel as i32).to_le_bytes());
        }
    }

    // Encoding.

    fn rex(&mut self, w: bool, reg: u8, rm: &Rm, force: bool) {
        let (x, b) = match rm {
            Rm::R(r) => (0, *r >> 3),
            Rm::M(mem) => (mem.index.map_or(0, |(i, _)| i.0 >> 3), mem.base.0 >> 3),
        };
        let rex = 0x40 | (u8::from(w) << 3) | ((reg >> 3) << 2) | (x << 1) | b;
        if rex != 0x40 || force {
            self.byte(rex);
        }
    }

    fn modrm(&mut self, reg: u8, rm: &Rm) {
        let reg = reg & 7;
        match rm {
            Rm::R(r) => self.byte(0xC0 | (reg << 3) | (r & 7)),
            Rm::M(mem) => {
                let base = mem.base.0 & 7;
                let (mode, size) = if mem.disp == 0 && base != 5 {
                    (0u8, 0)
                } else if (-128..128).contains(&mem.disp) {
                    (1, 1)
                } else {
                    (2, 4)
                };
                match mem.index {
                    None if base != 4 => self.byte((mode << 6) | (reg << 3) | base),
                    None => {
                        self.byte((mode << 6) | (reg << 3) | 4);
                        self.byte(0x24);
                    }
                    Some((index, scale)) => {
                        self.byte((mode << 6) | (reg << 3) | 4);
                        self.byte((scale << 6) | ((index.0 & 7) << 3) | base);
                    }
                }
                match size {
                    1 => self.byte(mem.disp as i8 as u8),
                    4 => self.d32(mem.disp),
                    _ => {}
                }
            }
        }
    }

    /// One instruction: an optional mandatory prefix, REX, the opcode, then ModRM and what follows it.
    // Called from everywhere a template emits code, which it would make much bigger if it were in each.
    #[inline(never)]
    fn inst(&mut self, prefix: Option<u8>, w: bool, opcode: &[u8], reg: u8, rm: Rm, force_rex: bool) {
        if let Some(prefix) = prefix {
            self.byte(prefix);
        }
        self.rex(w, reg, &rm, force_rex);
        self.bytes(opcode);
        self.modrm(reg, &rm);
    }

    // Moves.

    pub(crate) fn mov_rr(&mut self, dst: R, src: R) {
        self.inst(None, true, &[0x89], src.0, Rm::R(dst.0), false);
    }

    pub(crate) fn mov_rr32(&mut self, dst: R, src: R) {
        self.inst(None, false, &[0x89], src.0, Rm::R(dst.0), false);
    }

    pub(crate) fn load(&mut self, dst: R, mem: Mem) {
        self.inst(None, true, &[0x8B], dst.0, Rm::M(mem), false);
    }

    pub(crate) fn load32(&mut self, dst: R, mem: Mem) {
        self.inst(None, false, &[0x8B], dst.0, Rm::M(mem), false);
    }

    /// `movzx r32, byte [mem]`.
    pub(crate) fn load8(&mut self, dst: R, mem: Mem) {
        self.inst(None, false, &[0x0F, 0xB6], dst.0, Rm::M(mem), false);
    }

    pub(crate) fn store(&mut self, mem: Mem, src: R) {
        self.inst(None, true, &[0x89], src.0, Rm::M(mem), false);
    }

    pub(crate) fn store32(&mut self, mem: Mem, src: R) {
        self.inst(None, false, &[0x89], src.0, Rm::M(mem), false);
    }

    pub(crate) fn store16(&mut self, mem: Mem, src: R) {
        self.inst(Some(0x66), false, &[0x89], src.0, Rm::M(mem), false);
    }

    /// `mov byte [mem], r8` (the low byte of `src`, which takes a REX for sil, dil, spl and bpl).
    pub(crate) fn store8(&mut self, mem: Mem, src: R) {
        self.inst(None, false, &[0x88], src.0, Rm::M(mem), src.0 >= 4);
    }

    /// `movsx r32, byte [mem]`.
    pub(crate) fn load8s(&mut self, dst: R, mem: Mem) {
        self.inst(None, false, &[0x0F, 0xBE], dst.0, Rm::M(mem), false);
    }

    /// `movzx r32, word [mem]`.
    pub(crate) fn load16(&mut self, dst: R, mem: Mem) {
        self.inst(None, false, &[0x0F, 0xB7], dst.0, Rm::M(mem), false);
    }

    /// `movsx r32, word [mem]`.
    pub(crate) fn load16s(&mut self, dst: R, mem: Mem) {
        self.inst(None, false, &[0x0F, 0xBF], dst.0, Rm::M(mem), false);
    }

    /// `inc byte [mem]`.
    pub(crate) fn inc_m8(&mut self, mem: Mem) {
        self.inst(None, false, &[0xFE], 0, Rm::M(mem), false);
    }

    // Called from everywhere a template emits code, which it would make much bigger if it were in each.
    #[inline(never)]
    pub(crate) fn mov_imm64(&mut self, dst: R, value: u64) {
        if value <= u32::MAX as u64 {
            // `mov r32, imm32` clears the top half.
            if dst.0 >= 8 {
                self.byte(0x41);
            }
            self.byte(0xB8 | (dst.0 & 7));
            self.d32(value as u32 as i32);
        } else if value as i64 >= i32::MIN as i64 && value as i64 <= i32::MAX as i64 {
            self.inst(None, true, &[0xC7], 0, Rm::R(dst.0), false);
            self.d32(value as i64 as i32);
        } else {
            self.byte(0x48 | (dst.0 >> 3));
            self.byte(0xB8 | (dst.0 & 7));
            self.bytes(&value.to_le_bytes());
        }
    }

    /// `mov qword [mem], imm32`, sign-extended.
    pub(crate) fn store_imm(&mut self, mem: Mem, value: i32) {
        self.inst(None, true, &[0xC7], 0, Rm::M(mem), false);
        self.d32(value);
    }

    pub(crate) fn store32_imm(&mut self, mem: Mem, value: u32) {
        self.inst(None, false, &[0xC7], 0, Rm::M(mem), false);
        self.d32(value as i32);
    }

    pub(crate) fn store8_imm(&mut self, mem: Mem, value: u8) {
        self.inst(None, false, &[0xC6], 0, Rm::M(mem), false);
        self.byte(value);
    }

    pub(crate) fn lea(&mut self, dst: R, mem: Mem) {
        self.inst(None, true, &[0x8D], dst.0, Rm::M(mem), false);
    }

    pub(crate) fn lea32(&mut self, dst: R, mem: Mem) {
        self.inst(None, false, &[0x8D], dst.0, Rm::M(mem), false);
    }

    pub(crate) fn cmov(&mut self, cc: Cc, dst: R, src: R) {
        self.inst(None, true, &[0x0F, 0x40 | cc.0], dst.0, Rm::R(src.0), false);
    }

    /// `cmovcc r64, [mem]`, which reads the memory whichever way it goes.
    pub(crate) fn cmov_rm(&mut self, cc: Cc, dst: R, mem: Mem) {
        self.inst(None, true, &[0x0F, 0x40 | cc.0], dst.0, Rm::M(mem), false);
    }

    /// Eight bytes of data, in the code.
    pub(crate) fn data64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    /// Fills in eight bytes already emitted in the hot buffer.
    pub(crate) fn patch64(code: &mut [u8], at: usize, value: u64) {
        code[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    // Arithmetic.

    /// `op dst, src`, 64 bits.
    pub(crate) fn alu_rr(&mut self, op: Alu, dst: R, src: R) {
        self.inst(None, true, &[(op as u8) * 8 + 1], src.0, Rm::R(dst.0), false);
    }

    pub(crate) fn alu_rr32(&mut self, op: Alu, dst: R, src: R) {
        self.inst(None, false, &[(op as u8) * 8 + 1], src.0, Rm::R(dst.0), false);
    }

    /// `op dst, [mem]`, 64 bits.
    pub(crate) fn alu_rm(&mut self, op: Alu, dst: R, mem: Mem) {
        self.inst(None, true, &[(op as u8) * 8 + 3], dst.0, Rm::M(mem), false);
    }

    pub(crate) fn alu_rm32(&mut self, op: Alu, dst: R, mem: Mem) {
        self.inst(None, false, &[(op as u8) * 8 + 3], dst.0, Rm::M(mem), false);
    }

    /// `op [mem], src`, 64 bits.
    pub(crate) fn alu_mr(&mut self, op: Alu, mem: Mem, src: R) {
        self.inst(None, true, &[(op as u8) * 8 + 1], src.0, Rm::M(mem), false);
    }

    fn alu_imm(&mut self, w: bool, op: Alu, rm: Rm, imm: i32) {
        if (-128..128).contains(&imm) {
            self.inst(None, w, &[0x83], op as u8, rm, false);
            self.byte(imm as i8 as u8);
        } else {
            self.inst(None, w, &[0x81], op as u8, rm, false);
            self.d32(imm);
        }
    }

    pub(crate) fn alu_ri(&mut self, op: Alu, dst: R, imm: i32) {
        self.alu_imm(true, op, Rm::R(dst.0), imm);
    }

    pub(crate) fn alu_ri32(&mut self, op: Alu, dst: R, imm: i32) {
        self.alu_imm(false, op, Rm::R(dst.0), imm);
    }

    pub(crate) fn alu_mi(&mut self, op: Alu, mem: Mem, imm: i32) {
        self.alu_imm(true, op, Rm::M(mem), imm);
    }

    pub(crate) fn alu_mi32(&mut self, op: Alu, mem: Mem, imm: i32) {
        self.alu_imm(false, op, Rm::M(mem), imm);
    }

    /// `cmp byte [mem], imm8`.
    pub(crate) fn cmp_m8(&mut self, mem: Mem, imm: u8) {
        self.inst(None, false, &[0x80], Alu::Cmp as u8, Rm::M(mem), false);
        self.byte(imm);
    }

    /// `test byte [mem], imm8`.
    pub(crate) fn test_m8(&mut self, mem: Mem, imm: u8) {
        self.inst(None, false, &[0xF6], 0, Rm::M(mem), false);
        self.byte(imm);
    }

    pub(crate) fn test_rr(&mut self, a: R, b: R) {
        self.inst(None, true, &[0x85], b.0, Rm::R(a.0), false);
    }

    pub(crate) fn test_rr32(&mut self, a: R, b: R) {
        self.inst(None, false, &[0x85], b.0, Rm::R(a.0), false);
    }

    /// `test r32, imm32`.
    pub(crate) fn test_ri32(&mut self, a: R, imm: u32) {
        self.inst(None, false, &[0xF7], 0, Rm::R(a.0), false);
        self.d32(imm as i32);
    }

    pub(crate) fn imul_rr32(&mut self, dst: R, src: R) {
        self.inst(None, false, &[0x0F, 0xAF], dst.0, Rm::R(src.0), false);
    }

    /// `imul dst, src, imm32`, 64 bits.
    pub(crate) fn imul_rri(&mut self, dst: R, src: R, imm: i32) {
        self.inst(None, true, &[0x69], dst.0, Rm::R(src.0), false);
        self.d32(imm);
    }

    pub(crate) fn imul_rri32(&mut self, dst: R, src: R, imm: i32) {
        self.inst(None, false, &[0x69], dst.0, Rm::R(src.0), false);
        self.d32(imm);
    }

    /// `cdq; idiv r32`: eax / r, quotient in eax, remainder in edx.
    pub(crate) fn idiv32(&mut self, r: R) {
        self.byte(0x99);
        self.inst(None, false, &[0xF7], 7, Rm::R(r.0), false);
    }

    pub(crate) fn neg32(&mut self, r: R) {
        self.inst(None, false, &[0xF7], 3, Rm::R(r.0), false);
    }

    pub(crate) fn not32(&mut self, r: R) {
        self.inst(None, false, &[0xF7], 2, Rm::R(r.0), false);
    }

    pub(crate) fn shift_ri(&mut self, op: Shift, r: R, by: u8) {
        self.inst(None, true, &[0xC1], op as u8, Rm::R(r.0), false);
        self.byte(by);
    }

    pub(crate) fn shift_ri32(&mut self, op: Shift, r: R, by: u8) {
        self.inst(None, false, &[0xC1], op as u8, Rm::R(r.0), false);
        self.byte(by);
    }

    /// Shifts a 32-bit register by `cl`.
    pub(crate) fn shift_cl32(&mut self, op: Shift, r: R) {
        self.inst(None, false, &[0xD3], op as u8, Rm::R(r.0), false);
    }

    pub(crate) fn inc_m(&mut self, mem: Mem) {
        self.inst(None, true, &[0xFF], 0, Rm::M(mem), false);
    }

    pub(crate) fn dec_m(&mut self, mem: Mem) {
        self.inst(None, true, &[0xFF], 1, Rm::M(mem), false);
    }

    // Control.

    #[inline(never)]
    pub(crate) fn jmp(&mut self, label: Label) {
        let start = self.buf().len() as u32;
        self.byte(0xE9);
        self.rel32(label);
        self.last_jump[usize::from(self.in_cold)] = Some((start, 5, label, self.fixups.len() - 1));
    }

    #[inline(never)]
    pub(crate) fn jcc(&mut self, cc: Cc, label: Label) {
        let start = self.buf().len() as u32;
        self.bytes(&[0x0F, 0x80 | cc.0]);
        self.rel32(label);
        self.last_jump[usize::from(self.in_cold)] = Some((start, 6, label, self.fixups.len() - 1));
    }

    pub(crate) fn call_label(&mut self, label: Label) {
        self.byte(0xE8);
        self.rel32(label);
    }

    /// A call to another compiled function's code, at `target` (see [`Asm::place`]).
    pub(crate) fn call_abs(&mut self, target: u64) {
        self.byte(0xE8);
        self.abs32(target);
    }

    /// A jump into another compiled function's code, at `target` (see [`Asm::place`]).
    pub(crate) fn jmp_abs(&mut self, target: u64) {
        self.byte(0xE9);
        self.abs32(target);
    }

    // Called from everywhere a template emits code, which it would make much bigger if it were in each.
    #[inline(never)]
    fn abs32(&mut self, target: u64) {
        let at = self.buf().len() as u32;
        let cold = self.in_cold;
        self.calls.push((cold, at, target));
        self.d32(0);
    }

    /// An instruction whose memory operand is `[rip + disp32]`, for the absolute address `target`, which has to be
    /// within 2 GB of the code (as its feedback is: see `memory::map_with_room`), with `tail` (an immediate) after the
    /// displacement, which the processor counts from the end of.
    fn rip(&mut self, w: bool, opcode: &[u8], reg: u8, target: u64, tail: &[u8], force_rex: bool) {
        let rex = 0x40 | (u8::from(w) << 3) | ((reg >> 3) << 2);
        if rex != 0x40 || force_rex {
            self.byte(rex);
        }
        self.bytes(opcode);
        self.byte(((reg & 7) << 3) | 5);
        self.abs32(target.wrapping_sub(tail.len() as u64));
        self.bytes(tail);
    }

    /// `mov byte [target], imm8`.
    pub(crate) fn rip_store8_imm(&mut self, target: u64, imm: u8) {
        self.rip(false, &[0xC6], 0, target, &[imm], false);
    }

    /// `mov byte [target], r8`.
    pub(crate) fn rip_store8(&mut self, target: u64, src: R) {
        self.rip(false, &[0x88], src.0, target, &[], src.0 >= 4);
    }

    /// `sub dword [target], imm8`.
    pub(crate) fn rip_sub32_imm8(&mut self, target: u64, imm: i8) {
        self.rip(false, &[0x83], Alu::Sub as u8, target, &[imm as u8], false);
    }

    /// `lea dst, [target]`.
    pub(crate) fn rip_lea(&mut self, dst: R, target: u64) {
        self.rip(true, &[0x8D], dst.0, target, &[], false);
    }

    /// `lea dst, [rip + label]`: where a label is.
    pub(crate) fn lea_label(&mut self, dst: R, label: Label) {
        self.byte(0x48 | ((dst.0 >> 3) << 2));
        self.byte(0x8D);
        self.byte(((dst.0 & 7) << 3) | 5);
        self.rel32(label);
    }

    pub(crate) fn jmp_r(&mut self, r: R) {
        self.inst(None, false, &[0xFF], 4, Rm::R(r.0), false);
    }

    pub(crate) fn call_r(&mut self, r: R) {
        self.inst(None, false, &[0xFF], 2, Rm::R(r.0), false);
    }

    pub(crate) fn ret(&mut self) {
        self.byte(0xC3);
    }

    pub(crate) fn push(&mut self, r: R) {
        if r.0 >= 8 {
            self.byte(0x41);
        }
        self.byte(0x50 | (r.0 & 7));
    }

    pub(crate) fn pop(&mut self, r: R) {
        if r.0 >= 8 {
            self.byte(0x41);
        }
        self.byte(0x58 | (r.0 & 7));
    }

    pub(crate) fn nop(&mut self) {
        self.byte(0x90);
    }

    pub(crate) fn int3(&mut self) {
        self.byte(0xCC);
    }

    // SSE2.

    /// `movq xmm, r64`.
    pub(crate) fn movq_xr(&mut self, dst: X, src: R) {
        self.inst(Some(0x66), true, &[0x0F, 0x6E], dst.0, Rm::R(src.0), false);
    }

    /// `movq r64, xmm`.
    pub(crate) fn movq_rx(&mut self, dst: R, src: X) {
        self.inst(Some(0x66), true, &[0x0F, 0x7E], src.0, Rm::R(dst.0), false);
    }

    /// `cvtsi2sd xmm, r32`.
    pub(crate) fn cvtsi2sd(&mut self, dst: X, src: R) {
        self.inst(Some(0xF2), false, &[0x0F, 0x2A], dst.0, Rm::R(src.0), false);
    }

    /// `cvtsi2sd xmm, r64`.
    pub(crate) fn cvtsi2sd64(&mut self, dst: X, src: R) {
        self.inst(Some(0xF2), true, &[0x0F, 0x2A], dst.0, Rm::R(src.0), false);
    }

    /// `cvttsd2si r32, xmm`.
    pub(crate) fn cvttsd2si(&mut self, dst: R, src: X) {
        self.inst(Some(0xF2), false, &[0x0F, 0x2C], dst.0, Rm::R(src.0), false);
    }

    /// `cvttsd2si r64, xmm`.
    pub(crate) fn cvttsd2si64(&mut self, dst: R, src: X) {
        self.inst(Some(0xF2), true, &[0x0F, 0x2C], dst.0, Rm::R(src.0), false);
    }

    pub(crate) fn sd(&mut self, op: Sd, dst: X, src: X) {
        self.inst(Some(0xF2), false, &[0x0F, op as u8], dst.0, Rm::R(src.0), false);
    }

    pub(crate) fn ucomisd(&mut self, a: X, b: X) {
        self.inst(Some(0x66), false, &[0x0F, 0x2E], a.0, Rm::R(b.0), false);
    }

    /// `movd xmm, r32`.
    pub(crate) fn movd_xr(&mut self, dst: X, src: R) {
        self.inst(Some(0x66), false, &[0x0F, 0x6E], dst.0, Rm::R(src.0), false);
    }

    /// `movsd xmm, qword [mem]`.
    pub(crate) fn movsd_load(&mut self, dst: X, mem: Mem) {
        self.inst(Some(0xF2), false, &[0x0F, 0x10], dst.0, Rm::M(mem), false);
    }

    /// `movsd qword [mem], xmm`.
    pub(crate) fn movsd_store(&mut self, mem: Mem, src: X) {
        self.inst(Some(0xF2), false, &[0x0F, 0x11], src.0, Rm::M(mem), false);
    }

    /// `movss xmm, dword [mem]`.
    pub(crate) fn movss_load(&mut self, dst: X, mem: Mem) {
        self.inst(Some(0xF3), false, &[0x0F, 0x10], dst.0, Rm::M(mem), false);
    }

    /// `movss dword [mem], xmm`.
    pub(crate) fn movss_store(&mut self, mem: Mem, src: X) {
        self.inst(Some(0xF3), false, &[0x0F, 0x11], src.0, Rm::M(mem), false);
    }

    /// `cvtss2sd xmm, xmm`.
    pub(crate) fn cvtss2sd(&mut self, dst: X, src: X) {
        self.inst(Some(0xF3), false, &[0x0F, 0x5A], dst.0, Rm::R(src.0), false);
    }

    /// `cvtsd2ss xmm, xmm`.
    pub(crate) fn cvtsd2ss(&mut self, dst: X, src: X) {
        self.inst(Some(0xF2), false, &[0x0F, 0x5A], dst.0, Rm::R(src.0), false);
    }

    /// `roundsd xmm, xmm, mode` (SSE4.1): 9 is down, 10 up, 11 towards zero, with the precision exception kept quiet.
    pub(crate) fn roundsd(&mut self, dst: X, src: X, mode: u8) {
        self.inst(Some(0x66), false, &[0x0F, 0x3A, 0x0B], dst.0, Rm::R(src.0), false);
        self.byte(mode);
    }

    /// `movapd xmm, xmm`.
    pub(crate) fn movapd(&mut self, dst: X, src: X) {
        self.inst(Some(0x66), false, &[0x0F, 0x28], dst.0, Rm::R(src.0), false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(f: impl FnOnce(&mut Asm)) -> Vec<u8> {
        let mut a = Asm::new();
        f(&mut a);
        a.finish()
    }

    #[test]
    fn instructions_are_encoded_as_the_manual_says() {
        // mov rax, [rbx+8]
        assert_eq!(code(|a| a.load(RAX, m(RBX, 8))), [0x48, 0x8B, 0x43, 0x08]);
        // mov rax, [r12] needs a SIB byte; [r13] needs a displacement.
        assert_eq!(code(|a| a.load(RAX, m(R12, 0))), [0x49, 0x8B, 0x04, 0x24]);
        assert_eq!(code(|a| a.load(RAX, m(R13, 0))), [0x49, 0x8B, 0x45, 0x00]);
        // mov [rbx+0x200], r9
        assert_eq!(
            code(|a| a.store(m(RBX, 0x200), R9)),
            [0x4C, 0x89, 0x8B, 0x00, 0x02, 0x00, 0x00]
        );
        // mov rax, [rbp + rax*8]
        assert_eq!(code(|a| a.load(RAX, mi(RBP, RAX, 3, 0))), [0x48, 0x8B, 0x44, 0xC5, 0x00]);
        // add eax, ecx; or rax, r14
        assert_eq!(code(|a| a.alu_rr32(Alu::Add, RAX, RCX)), [0x01, 0xC8]);
        assert_eq!(code(|a| a.alu_rr(Alu::Or, RAX, R14)), [0x4C, 0x09, 0xF0]);
        // cmp edx, 0xFFF90000
        assert_eq!(
            code(|a| a.alu_ri32(Alu::Cmp, RDX, 0xFFF9_0000u32 as i32)),
            [0x81, 0xFA, 0x00, 0x00, 0xF9, 0xFF]
        );
        // shr rdx, 32
        assert_eq!(code(|a| a.shift_ri(Shift::Shr, RDX, 32)), [0x48, 0xC1, 0xEA, 0x20]);
        // movq xmm0, rax; movq rax, xmm1; addsd xmm0, xmm1; ucomisd xmm1, xmm0
        assert_eq!(code(|a| a.movq_xr(XMM0, RAX)), [0x66, 0x48, 0x0F, 0x6E, 0xC0]);
        assert_eq!(code(|a| a.movq_rx(RAX, XMM1)), [0x66, 0x48, 0x0F, 0x7E, 0xC8]);
        assert_eq!(code(|a| a.sd(Sd::Add, XMM0, XMM1)), [0xF2, 0x0F, 0x58, 0xC1]);
        assert_eq!(code(|a| a.ucomisd(XMM1, XMM0)), [0x66, 0x0F, 0x2E, 0xC8]);
        // cvtsi2sd xmm1, ecx
        assert_eq!(code(|a| a.cvtsi2sd(XMM1, RCX)), [0xF2, 0x0F, 0x2A, 0xC9]);
        // mov r14, 0xFFF9000000000000
        assert_eq!(
            code(|a| a.mov_imm64(R14, 0xFFF9_0000_0000_0000)),
            [0x49, 0xBE, 0, 0, 0, 0, 0, 0, 0xF9, 0xFF]
        );
        // call r11; push r15; pop rbx
        assert_eq!(code(|a| a.call_r(R11)), [0x41, 0xFF, 0xD3]);
        assert_eq!(code(|a| a.push(R15)), [0x41, 0x57]);
        assert_eq!(code(|a| a.pop(RBX)), [0x5B]);
        // test byte [r12+0x2a], 0xff
        assert_eq!(code(|a| a.test_m8(m(R12, 0x2A), 0xFF)), [0x41, 0xF6, 0x44, 0x24, 0x2A, 0xFF]);
        // inc qword [r13+0x10]
        assert_eq!(code(|a| a.inc_m(m(R13, 0x10))), [0x49, 0xFF, 0x45, 0x10]);
        // mov word [r12+4], r9w
        assert_eq!(code(|a| a.store16(m(R12, 4), R9)), [0x66, 0x45, 0x89, 0x4C, 0x24, 0x04]);
        // mov byte [rax+rcx], dl; mov byte [rax], sil (which needs a REX)
        assert_eq!(code(|a| a.store8(mi(RAX, RCX, 0, 0), RDX)), [0x88, 0x14, 0x08]);
        assert_eq!(code(|a| a.store8(m(RAX, 0), RSI)), [0x40, 0x88, 0x30]);
        // movsx eax, byte [r11+rcx]; movzx eax, word [r11+rcx*2]; movsx eax, word [r11+rcx*2]
        assert_eq!(code(|a| a.load8s(RAX, mi(R11, RCX, 0, 0))), [0x41, 0x0F, 0xBE, 0x04, 0x0B]);
        assert_eq!(code(|a| a.load16(RAX, mi(R11, RCX, 1, 0))), [0x41, 0x0F, 0xB7, 0x04, 0x4B]);
        assert_eq!(code(|a| a.load16s(RAX, mi(R11, RCX, 1, 0))), [0x41, 0x0F, 0xBF, 0x04, 0x4B]);
        // inc byte [rax+0x2b]
        assert_eq!(code(|a| a.inc_m8(m(RAX, 0x2B))), [0xFE, 0x40, 0x2B]);
        // movsd xmm0, [r11+rcx*8]; movsd [r11+rcx*8], xmm1; movss xmm0, [r11+rcx*4]; movss [r11+rcx*4], xmm1
        assert_eq!(code(|a| a.movsd_load(XMM0, mi(R11, RCX, 3, 0))), [0xF2, 0x41, 0x0F, 0x10, 0x04, 0xCB]);
        assert_eq!(code(|a| a.movsd_store(mi(R11, RCX, 3, 0), XMM1)), [0xF2, 0x41, 0x0F, 0x11, 0x0C, 0xCB]);
        assert_eq!(code(|a| a.movss_load(XMM0, mi(R11, RCX, 2, 0))), [0xF3, 0x41, 0x0F, 0x10, 0x04, 0x8B]);
        assert_eq!(code(|a| a.movss_store(mi(R11, RCX, 2, 0), XMM1)), [0xF3, 0x41, 0x0F, 0x11, 0x0C, 0x8B]);
        // cvtss2sd xmm0, xmm0; cvtsd2ss xmm1, xmm0; movd xmm0, eax; roundsd xmm0, xmm1, 9; movapd xmm1, xmm0
        assert_eq!(code(|a| a.cvtss2sd(XMM0, XMM0)), [0xF3, 0x0F, 0x5A, 0xC0]);
        assert_eq!(code(|a| a.cvtsd2ss(XMM1, XMM0)), [0xF2, 0x0F, 0x5A, 0xC8]);
        assert_eq!(code(|a| a.movd_xr(XMM0, RAX)), [0x66, 0x0F, 0x6E, 0xC0]);
        assert_eq!(code(|a| a.roundsd(XMM0, XMM1, 9)), [0x66, 0x0F, 0x3A, 0x0B, 0xC1, 0x09]);
        assert_eq!(code(|a| a.movapd(XMM1, XMM0)), [0x66, 0x0F, 0x28, 0xC8]);
        // sqrtsd xmm0, xmm0; minsd xmm0, xmm1
        assert_eq!(code(|a| a.sd(Sd::Sqrt, XMM0, XMM0)), [0xF2, 0x0F, 0x51, 0xC0]);
        assert_eq!(code(|a| a.sd(Sd::Min, XMM0, XMM1)), [0xF2, 0x0F, 0x5D, 0xC1]);
    }

    #[test]
    fn jumps_reach_their_labels_across_buffers() {
        let mut a = Asm::new();
        let there = a.label();
        a.jcc(NE, there); // 6 bytes
        a.set_cold(true);
        a.bind(there);
        a.ret();
        a.set_cold(false);
        a.ret();
        let code = a.finish();
        // jne +1 (over the hot ret), then the hot ret, then the cold ret.
        assert_eq!(code, [0x0F, 0x85, 1, 0, 0, 0, 0xC3, 0xC3]);
    }
}
