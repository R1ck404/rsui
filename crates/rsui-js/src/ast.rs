//! What source code means, as the parser finds it. It all lives in one block of memory that's handed back as soon as
//! the code has been compiled, so nothing in it needs dropping, and parts of it can be copied around freely.

use std::cell::Cell;

/// Names without repeats, each with its [`name_hash`], in the order of those. Sorting numbers is much quicker than
/// sorting text, and the parser makes one of these for every function.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Names<'a>(pub(crate) &'a [(u32, &'a str)]);

impl Names<'_> {
    pub(crate) fn contains(&self, name: &str) -> bool {
        let hash = name_hash(name);
        let first = self.0.partition_point(|(found, _)| *found < hash);
        self.0[first..]
            .iter()
            .take_while(|(found, _)| *found == hash)
            .any(|(_, found)| *found == name)
    }
}

/// A number made from a name, the same each time, for [`Names`] (FNV-1a).
pub(crate) fn name_hash(name: &str) -> u32 {
    name.bytes().fold(0x811c_9dc5u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

/// Where the parser puts everything it makes. It reserves a big range of addresses and takes memory as it needs it, and
/// another range if that one fills up.
pub(crate) struct AstMemory {
    base: Cell<*mut u8>,
    used: Cell<usize>,
    committed: Cell<usize>,
    /// How big the range at `base` is.
    reserved: Cell<usize>,
    /// The ranges that filled up before the one at `base`, which are still in use.
    full: std::cell::RefCell<Vec<*mut u8>>,
}

#[allow(unsafe_code)]
mod system {
    use std::ffi::c_void;

    pub(super) const MEM_COMMIT: u32 = 0x1000;
    pub(super) const MEM_RESERVE: u32 = 0x2000;
    pub(super) const MEM_RELEASE: u32 = 0x8000;
    pub(super) const PAGE_NOACCESS: u32 = 1;
    pub(super) const PAGE_READWRITE: u32 = 4;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub(super) fn VirtualAlloc(
            address: *mut c_void,
            size: usize,
            kind: u32,
            protect: u32,
        ) -> *mut c_void;
        pub(super) fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
    }

    pub(super) const MEM_DECOMMIT: u32 = 0x4000;
}

/// More than nearly any source code needs, as this is only addresses. Asking for more costs more, in time and in what
/// Windows keeps to describe the range, which counts when a program keeps many files' trees (see
/// `compiler::KeptAst`), so a file that needs more gets another range.
#[cfg(not(test))]
const AST_RESERVED: usize = 1 << 28;
/// Small in tests, so that going on to another range is something every test that parses much does.
#[cfg(test)]
const AST_RESERVED: usize = 1 << 20;
const AST_STEP: usize = 256 * 1024;

/// How much of the memory a parse took is kept for the next one. A program's files are parsed one after another as it
/// starts, and memory that's already there is much quicker to use than memory that has to be found and cleared first.
const AST_KEPT: usize = 4 * 1024 * 1024;

/// A range of addresses, with what's at its start still there.
struct Spare {
    base: *mut u8,
    committed: usize,
}

/// Ranges that were used for parsing and can be again, on this thread.
struct Spares(Vec<Spare>);

impl Drop for Spares {
    fn drop(&mut self) {
        release_spares(&mut self.0);
    }
}

thread_local! {
    static SPARES: std::cell::RefCell<Spares> = const { std::cell::RefCell::new(Spares(Vec::new())) };
}

#[allow(unsafe_code)]
fn release_spares(spares: &mut Vec<Spare>) {
    for spare in spares.drain(..) {
        // SAFETY: these are addresses `AstMemory::new` reserved, whole, that nothing is using.
        unsafe {
            system::VirtualFree(spare.base.cast(), 0, system::MEM_RELEASE);
        }
    }
}

/// Gives back the memory that's kept on this thread for parsing, once there's not going to be much more of it for a
/// while, as when a program has started.
pub(crate) fn release_spare_memory() {
    let _ = SPARES.try_with(|spares| release_spares(&mut spares.borrow_mut().0));
}

#[allow(unsafe_code)]
impl AstMemory {
    pub(crate) fn new() -> Self {
        if let Some(spare) = SPARES
            .try_with(|spares| spares.borrow_mut().0.pop())
            .ok()
            .flatten()
        {
            return Self {
                base: Cell::new(spare.base),
                used: Cell::new(0),
                committed: Cell::new(spare.committed),
                reserved: Cell::new(AST_RESERVED),
                full: std::cell::RefCell::new(Vec::new()),
            };
        }
        Self {
            base: Cell::new(reserve(AST_RESERVED)),
            used: Cell::new(0),
            committed: Cell::new(0),
            reserved: Cell::new(AST_RESERVED),
            full: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// Room for `bytes` at `start`, or at the start of a new range if this one hasn't enough.
    #[cold]
    #[inline(never)]
    fn room_slowly(&self, start: usize, bytes: usize) -> *mut u8 {
        let (mut start, mut end) = (start, start + bytes);
        if end > self.reserved.get() {
            let size = bytes.next_multiple_of(AST_STEP).max(AST_RESERVED);
            self.full.borrow_mut().push(self.base.get());
            self.base.set(reserve(size));
            self.reserved.set(size);
            self.committed.set(0);
            (start, end) = (0, bytes);
        }
        let have = self.committed.get();
        let wanted = end
            .next_multiple_of(AST_STEP)
            .max(have + AST_STEP)
            .min(self.reserved.get());
        // SAFETY: the range is inside what was reserved at `base`.
        let done = unsafe {
            system::VirtualAlloc(
                self.base.get().add(have).cast(),
                wanted - have,
                system::MEM_COMMIT,
                system::PAGE_READWRITE,
            )
        };
        assert!(!done.is_null(), "out of memory");
        self.committed.set(wanted);
        self.used.set(end);
        // SAFETY: that's inside the range with memory behind it.
        unsafe { self.base.get().add(start) }
    }

    /// Gives back what was taken in advance and not used, for memory that's going to be kept for a while.
    pub(crate) fn shrink(&self) {
        let keep = self.used.get().next_multiple_of(4096);
        let committed = self.committed.get();
        if committed > keep {
            // SAFETY: that's inside what was reserved at `base`, past everything that's been given out.
            unsafe {
                system::VirtualFree(
                    self.base.get().add(keep).cast(),
                    committed - keep,
                    system::MEM_DECOMMIT,
                );
            }
            self.committed.set(keep);
        }
    }

    #[inline]
    fn room_for<T>(&self, count: usize) -> *mut T {
        const { assert!(align_of::<T>() <= 8) };
        let start = self.used.get().next_multiple_of(align_of::<T>());
        let bytes = count * size_of::<T>();
        let end = start + bytes;
        if end > self.committed.get() {
            // Ranges start on a page boundary, which is aligned enough.
            return self.room_slowly(start, bytes).cast();
        }
        self.used.set(end);
        // SAFETY: that's inside the range with memory behind it, and aligned because `base` is on a page boundary.
        unsafe { self.base.get().add(start).cast() }
    }

    /// Keeps a value for as long as this memory lasts. Only things that don't need dropping can go here.
    #[inline]
    pub(crate) fn node<T: Copy>(&self, value: T) -> &T {
        let place = self.room_for::<T>(1);
        // SAFETY: `room_for` gave a place of the right size and alignment that nothing else has.
        unsafe {
            place.write(value);
            &*place
        }
    }

    /// Keeps a list for as long as this memory lasts.
    #[inline]
    pub(crate) fn list<T: Copy>(&self, items: &[T]) -> &[T] {
        if items.is_empty() {
            return &[];
        }
        let place = self.room_for::<T>(items.len());
        // SAFETY: `room_for` gave a place of the right size and alignment that nothing else has.
        unsafe {
            std::ptr::copy_nonoverlapping(items.as_ptr(), place, items.len());
            std::slice::from_raw_parts(place, items.len())
        }
    }
}

#[allow(unsafe_code)]
impl AstMemory {
    /// Keeps ASCII bytes as UTF-16, for as long as this memory lasts.
    pub(crate) fn widened(&self, bytes: &[u8]) -> &[u16] {
        if bytes.is_empty() {
            return &[];
        }
        let place = self.room_for::<u16>(bytes.len());
        // SAFETY: `room_for` gave a place of the right size and alignment that nothing else has, and every unit of it
        // is written before it's seen.
        unsafe {
            for (at, &byte) in bytes.iter().enumerate() {
                place.add(at).write(u16::from(byte));
            }
            std::slice::from_raw_parts(place, bytes.len())
        }
    }

    /// Keeps some text for as long as this memory lasts.
    pub(crate) fn text(&self, text: &str) -> &str {
        let bytes = self.list(text.as_bytes());
        // SAFETY: these bytes were just copied from a `str`.
        unsafe { std::str::from_utf8_unchecked(bytes) }
    }
}

#[allow(unsafe_code)]
impl Drop for AstMemory {
    fn drop(&mut self) {
        for base in self.full.get_mut().drain(..) {
            // SAFETY: these are whole ranges `reserve` got, and everything borrowed from here has gone.
            unsafe {
                system::VirtualFree(base.cast(), 0, system::MEM_RELEASE);
            }
        }
        let base = self.base.get();
        if self.reserved.get() != AST_RESERVED {
            // SAFETY: as above.
            unsafe {
                system::VirtualFree(base.cast(), 0, system::MEM_RELEASE);
            }
            return;
        }
        let committed = self.committed.get();
        let kept = committed.min(AST_KEPT);
        if committed > kept {
            // SAFETY: that's inside what `new` reserved, and everything borrowed from here has gone.
            unsafe {
                system::VirtualFree(
                    base.add(kept).cast(),
                    committed - kept,
                    system::MEM_DECOMMIT,
                );
            }
        }
        let spare = Spare {
            base,
            committed: kept,
        };
        let kept = SPARES
            .try_with(|spares| {
                let mut spares = spares.borrow_mut();
                // One for each parse that's going on at once, which is hardly ever more than this.
                (spares.0.len() < 2).then(|| spares.0.push(spare)).is_some()
            })
            .unwrap_or(false);
        if !kept {
            // SAFETY: these are the addresses `new` got, whole, and everything borrowed from here has gone.
            unsafe {
                system::VirtualFree(base.cast(), 0, system::MEM_RELEASE);
            }
        }
    }
}

/// A new range of addresses for parsing into, which starts on a page boundary.
#[allow(unsafe_code)]
fn reserve(size: usize) -> *mut u8 {
    // SAFETY: asking for a new range of addresses has no preconditions.
    let base = unsafe {
        system::VirtualAlloc(
            std::ptr::null_mut(),
            size,
            system::MEM_RESERVE,
            system::PAGE_NOACCESS,
        )
    };
    assert!(!base.is_null(), "there are no addresses left to reserve");
    base.cast()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VarKind {
    Var,
    Let,
    Const,
    /// `using`, a `const` whose value is disposed of when the block it's in is left.
    Using,
    /// `await using`, which is disposed of in the same way and waited for.
    AwaitUsing,
    /// A function's own name, inside it: writing to it is ignored, unless the code is strict.
    FunctionName,
}

impl VarKind {
    pub(crate) fn is_using(self) -> bool {
        matches!(self, Self::Using | Self::AwaitUsing)
    }
}

/// What a `with`'s object is called inside the compiler, by how many `with`s it's inside of.
pub(crate) const WITH_NAMES: [&str; 8] = [
    "%with0", "%with1", "%with2", "%with3", "%with4", "%with5", "%with6", "%with7",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    UShr,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    In,
    InstanceOf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogicalOp {
    And,
    Or,
    Nullish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnaryOp {
    Neg,
    Plus,
    Not,
    BitNot,
    TypeOf,
    Void,
    Delete,
}

/// What an assignment does to what's there: `=`, or an operator as in `+=`, or one that may not assign at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssignOp {
    Plain,
    Binary(BinaryOp),
    Logical(LogicalOp),
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Expr<'a> {
    Number(f64),
    BigInt(&'a str, u32),
    String(&'a [u16]),
    Template(&'a TemplateParts<'a>),
    TaggedTemplate(&'a TaggedParts<'a>),
    Regex(&'a RegexParts<'a>),
    Ident(&'a str, u32),
    /// `#x`, which only comes before `in`.
    PrivateName(&'a str),
    /// `import(x)`.
    ImportCall(&'a Expr<'a>),
    /// `import.meta`.
    ImportMeta,
    This,
    Null,
    True,
    False,
    NewTarget,
    Array(&'a [Option<Element<'a>>]),
    Object(&'a [Property<'a>]),
    Function(&'a Function<'a>),
    Class(&'a Class<'a>),
    Unary(UnaryOp, &'a Expr<'a>),
    Update {
        increment: bool,
        prefix: bool,
        target: &'a Expr<'a>,
    },
    Binary(BinaryOp, &'a Expr<'a>, &'a Expr<'a>),
    Logical(LogicalOp, &'a Expr<'a>, &'a Expr<'a>),
    Assign {
        op: AssignOp,
        target: &'a Pattern<'a>,
        value: &'a Expr<'a>,
    },
    Conditional(&'a Expr<'a>, &'a Expr<'a>, &'a Expr<'a>),
    Call {
        callee: &'a Expr<'a>,
        args: &'a [Element<'a>],
        optional: bool,
        line: u32,
    },
    New {
        callee: &'a Expr<'a>,
        args: &'a [Element<'a>],
        line: u32,
    },
    Member {
        object: &'a Expr<'a>,
        name: &'a str,
        optional: bool,
    },
    Index {
        object: &'a Expr<'a>,
        index: &'a Expr<'a>,
        optional: bool,
    },
    /// Where an optional chain ends, which is as far as `?.` skips.
    Chain(&'a Expr<'a>),
    SuperCall(&'a [Element<'a>]),
    SuperMember(&'a str),
    SuperIndex(&'a Expr<'a>),
    Sequence(&'a [Expr<'a>]),
    Await(&'a Expr<'a>),
    Yield {
        value: Option<&'a Expr<'a>>,
        delegate: bool,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TemplateParts<'a> {
    pub(crate) strings: &'a [&'a [u16]],
    pub(crate) values: &'a [Expr<'a>],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TaggedParts<'a> {
    pub(crate) tag: &'a Expr<'a>,
    pub(crate) strings: &'a [&'a [u16]],
    pub(crate) raw: &'a [&'a [u16]],
    pub(crate) values: &'a [Expr<'a>],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RegexParts<'a> {
    pub(crate) pattern: &'a [u16],
    pub(crate) flags: &'a [u16],
}

/// Something in an array literal or a call's arguments.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Element<'a> {
    Plain(Expr<'a>),
    Spread(Expr<'a>),
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PropertyKey<'a> {
    Name(&'a str),
    String(&'a [u16]),
    Number(f64),
    BigInt(&'a str, u32),
    Computed(Expr<'a>),
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Property<'a> {
    Value(PropertyKey<'a>, Expr<'a>),
    /// `{ a }`, which is a value that's the variable of the same name.
    Short(&'a str, u32),
    Method(PropertyKey<'a>, &'a Function<'a>),
    Getter(PropertyKey<'a>, &'a Function<'a>),
    Setter(PropertyKey<'a>, &'a Function<'a>),
    Spread(Expr<'a>),
}

/// Somewhere to put a value, or the parts of one.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Pattern<'a> {
    Ident(&'a str, u32),
    /// A property or element of something, which only assignments can have.
    Expr(Expr<'a>),
    Array {
        items: &'a [Option<PatternItem<'a>>],
        rest: Option<&'a Pattern<'a>>,
    },
    Object {
        items: &'a [(PropertyKey<'a>, PatternItem<'a>)],
        rest: Option<&'a Pattern<'a>>,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PatternItem<'a> {
    pub(crate) target: Pattern<'a>,
    pub(crate) default: Option<Expr<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FunctionKind {
    Normal,
    Arrow,
    Method,
    Getter,
    Setter,
    Constructor,
    DerivedConstructor,
    /// What sets up a class's fields on a new instance.
    Fields,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Function<'a> {
    pub(crate) name: Option<&'a str>,
    pub(crate) kind: FunctionKind,
    pub(crate) is_async: bool,
    pub(crate) is_generator: bool,
    pub(crate) params: &'a [PatternItem<'a>],
    pub(crate) rest: Option<Pattern<'a>>,
    pub(crate) body: &'a [Stmt<'a>],
    /// Every name mentioned in functions inside this one, which may need to outlive its call.
    pub(crate) inner_names: Names<'a>,
    /// Every name this function or one inside it mentions that may be from outside it, each once, which is what it
    /// has to be able to see when it's made, if it's compiled later.
    pub(crate) outer_names: &'a [&'a str],
    /// Whether this function itself says `arguments`.
    pub(crate) uses_arguments: bool,
    /// Whether this function or one inside it says this function's name.
    pub(crate) uses_own_name: bool,
    /// Whether this function or an arrow inside it says `this`.
    pub(crate) uses_this: bool,
    pub(crate) line: u32,
    /// Where its text is in the source, from and to, in bytes, which `Function.prototype.toString` gives back: from
    /// `function` or `async`, a method's name or what's before it but `static`, an arrow's parameters, or a class's
    /// `class`, for its constructor. `(0, 0)` for one that's made up, as a class's fields are.
    pub(crate) source: (u32, u32),
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ClassMember<'a> {
    Method {
        key: PropertyKey<'a>,
        function: &'a Function<'a>,
        is_static: bool,
    },
    Getter {
        key: PropertyKey<'a>,
        function: &'a Function<'a>,
        is_static: bool,
    },
    Setter {
        key: PropertyKey<'a>,
        function: &'a Function<'a>,
        is_static: bool,
    },
    /// A field, or a static block, which is all in functions of their own.
    StaticInit(&'a Function<'a>),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Class<'a> {
    pub(crate) name: Option<&'a str>,
    pub(crate) parent: Option<Expr<'a>>,
    pub(crate) constructor: &'a Function<'a>,
    pub(crate) members: &'a [ClassMember<'a>],
    /// What runs on each new instance, for fields, if there are any.
    pub(crate) fields: Option<&'a Function<'a>>,
    /// The private names it declares, which mean something different in any other class.
    pub(crate) private_names: &'a [&'a str],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SwitchCase<'a> {
    pub(crate) test: Option<Expr<'a>>,
    pub(crate) body: &'a [Stmt<'a>],
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ForInit<'a> {
    Var(VarKind, &'a [(Pattern<'a>, Option<Expr<'a>>)]),
    Expr(Expr<'a>),
}

/// What's assigned on each turn of a `for of` or a `for in`.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ForTarget<'a> {
    Declare(VarKind, Pattern<'a>),
    Assign(Pattern<'a>),
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ExportName<'a> {
    /// `export { local as exported }`
    List(&'a [(&'a str, &'a str)]),
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Stmt<'a> {
    Empty,
    /// Where a statement starts, in a file that's being debugged, which is where the program can be paused. Only the
    /// parser puts these in, and only when [`crate::inspect`] says the file's being debugged.
    Line(u32),
    Expr(Expr<'a>),
    Var(VarKind, &'a [(Pattern<'a>, Option<Expr<'a>>)]),
    Function(&'a Function<'a>),
    Class(&'a Class<'a>),
    Block(&'a [Stmt<'a>]),
    If(&'a Expr<'a>, &'a Stmt<'a>, Option<&'a Stmt<'a>>),
    For(&'a ForStmt<'a>),
    ForIn(&'a ForInStmt<'a>),
    ForOf(&'a ForOfStmt<'a>),
    With(&'a Expr<'a>, &'a Stmt<'a>),
    While(&'a Expr<'a>, &'a Stmt<'a>),
    DoWhile(&'a Stmt<'a>, &'a Expr<'a>),
    Return(Option<&'a Expr<'a>>),
    Break(Option<&'a str>),
    Continue(Option<&'a str>),
    Throw(&'a Expr<'a>),
    Try(&'a TryStmt<'a>),
    Switch(&'a Expr<'a>, &'a [SwitchCase<'a>]),
    Labeled(&'a str, &'a Stmt<'a>),
    /// A class's field, which goes on `this` whatever its prototype has.
    Field(&'a PropertyKey<'a>, Option<&'a Expr<'a>>),
    /// `import a, { b as c } from "x"`, `import * as d from "x"`.
    Import(&'a ImportStmt<'a>),
    /// Names this module gives to others.
    Export(ExportName<'a>),
    /// `export { a as b } from "x"`, or everything from it.
    ExportFrom(&'a ExportFromStmt<'a>),
    ExportDefault(&'a Expr<'a>),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ForStmt<'a> {
    pub(crate) init: Option<ForInit<'a>>,
    pub(crate) test: Option<Expr<'a>>,
    pub(crate) update: Option<Expr<'a>>,
    pub(crate) body: &'a Stmt<'a>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ForInStmt<'a> {
    pub(crate) target: ForTarget<'a>,
    pub(crate) object: Expr<'a>,
    pub(crate) body: &'a Stmt<'a>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ForOfStmt<'a> {
    pub(crate) target: ForTarget<'a>,
    pub(crate) iterable: Expr<'a>,
    pub(crate) body: &'a Stmt<'a>,
    pub(crate) is_await: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TryStmt<'a> {
    pub(crate) block: &'a [Stmt<'a>],
    pub(crate) param: Option<Pattern<'a>>,
    pub(crate) handler: Option<&'a [Stmt<'a>]>,
    pub(crate) finalizer: Option<&'a [Stmt<'a>]>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ImportStmt<'a> {
    pub(crate) source: &'a [u16],
    pub(crate) default: Option<&'a str>,
    pub(crate) all: Option<&'a str>,
    pub(crate) named: &'a [(&'a str, &'a str)],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExportFromStmt<'a> {
    pub(crate) source: &'a [u16],
    pub(crate) named: Option<&'a [(&'a str, &'a str)]>,
}

/// A whole program: its body, as a function that's called once.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Program<'a> {
    pub(crate) function: Function<'a>,
    /// Whether an `import` or `export` statement was written at its top, even one that's only of types and so is
    /// gone, which TypeScript takes to make a file a module, as the language does any other. TypeScript's own ways to
    /// write CommonJS, `import x = require()` and `export =`, don't count.
    pub(crate) says_module: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_doesnt_fit_goes_in_another_range_and_stays_put() {
        let memory = AstMemory::new();
        let lists: Vec<&[u64]> = (0..5u64)
            .map(|at| memory.list(&vec![at; AST_RESERVED / 8 / 2 + 1]))
            .collect();
        let big = memory.list(&vec![9u64; AST_RESERVED / 8 * 3]);
        let small = memory.node(7u32);
        for (at, list) in lists.iter().enumerate() {
            assert!(list.iter().all(|&value| value == at as u64));
        }
        assert!(big.iter().all(|&value| value == 9));
        assert_eq!(*small, 7);
        assert!(memory.full.borrow().len() >= 3);
    }

    #[test]
    fn the_parts_of_the_tree_stay_small() {
        assert!(size_of::<Expr<'_>>() <= 32, "{}", size_of::<Expr<'_>>());
        assert!(size_of::<Stmt<'_>>() <= 32, "{}", size_of::<Stmt<'_>>());
        assert!(
            size_of::<Element<'_>>() <= 40,
            "{}",
            size_of::<Element<'_>>()
        );
    }
}
