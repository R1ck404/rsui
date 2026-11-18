//! The instructions compiled code is made of. Each is 8 bytes, and names its registers by number.

use crate::text::Atom;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Op {
    LoadConst {
        dst: u16,
        index: u32,
    },
    LoadInt {
        dst: u16,
        value: i32,
    },
    LoadUndefined {
        dst: u16,
    },
    LoadNull {
        dst: u16,
    },
    LoadTrue {
        dst: u16,
    },
    LoadFalse {
        dst: u16,
    },
    LoadEmpty {
        dst: u16,
    },
    Move {
        dst: u16,
        src: u16,
    },

    Add {
        dst: u16,
        a: u16,
        b: u16,
    },
    AddInt {
        dst: u16,
        a: u16,
        value: i16,
    },
    /// `(a + b) | 0`, which code that keeps to 32-bit ints is full of: two ints add round, as `| 0` makes them.
    AddInt32 {
        dst: u16,
        a: u16,
        b: u16,
    },
    SubInt {
        dst: u16,
        a: u16,
        value: i16,
    },
    MulInt {
        dst: u16,
        a: u16,
        value: i16,
    },
    /// The number is always above zero.
    ModInt {
        dst: u16,
        a: u16,
        value: i16,
    },
    /// The number is always a power of two, so an int it divides exactly is shifted, with no division at all.
    DivInt {
        dst: u16,
        a: u16,
        value: i16,
    },
    BitAndInt {
        dst: u16,
        a: u16,
        value: i16,
    },
    BitOrInt {
        dst: u16,
        a: u16,
        value: i16,
    },
    ShlInt {
        dst: u16,
        a: u16,
        value: i16,
    },
    ShrInt {
        dst: u16,
        a: u16,
        value: i16,
    },
    UShrInt {
        dst: u16,
        a: u16,
        value: i16,
    },
    // An operator with a constant on the right, which saves an instruction to load it.
    AddK {
        dst: u16,
        a: u16,
        k: u16,
    },
    SubK {
        dst: u16,
        a: u16,
        k: u16,
    },
    MulK {
        dst: u16,
        a: u16,
        k: u16,
    },
    DivK {
        dst: u16,
        a: u16,
        k: u16,
    },
    ModK {
        dst: u16,
        a: u16,
        k: u16,
    },
    Sub {
        dst: u16,
        a: u16,
        b: u16,
    },
    Mul {
        dst: u16,
        a: u16,
        b: u16,
    },
    Div {
        dst: u16,
        a: u16,
        b: u16,
    },
    Mod {
        dst: u16,
        a: u16,
        b: u16,
    },
    Pow {
        dst: u16,
        a: u16,
        b: u16,
    },
    BitAnd {
        dst: u16,
        a: u16,
        b: u16,
    },
    BitOr {
        dst: u16,
        a: u16,
        b: u16,
    },
    BitXor {
        dst: u16,
        a: u16,
        b: u16,
    },
    Shl {
        dst: u16,
        a: u16,
        b: u16,
    },
    Shr {
        dst: u16,
        a: u16,
        b: u16,
    },
    UShr {
        dst: u16,
        a: u16,
        b: u16,
    },
    Lt {
        dst: u16,
        a: u16,
        b: u16,
    },
    Le {
        dst: u16,
        a: u16,
        b: u16,
    },
    Gt {
        dst: u16,
        a: u16,
        b: u16,
    },
    Ge {
        dst: u16,
        a: u16,
        b: u16,
    },
    Eq {
        dst: u16,
        a: u16,
        b: u16,
    },
    Ne {
        dst: u16,
        a: u16,
        b: u16,
    },
    StrictEq {
        dst: u16,
        a: u16,
        b: u16,
    },
    StrictNe {
        dst: u16,
        a: u16,
        b: u16,
    },
    In {
        dst: u16,
        a: u16,
        b: u16,
    },
    InstanceOf {
        dst: u16,
        a: u16,
        b: u16,
    },

    Neg {
        dst: u16,
        src: u16,
    },
    ToNumber {
        dst: u16,
        src: u16,
    },
    ToNumeric {
        dst: u16,
        src: u16,
    },
    Not {
        dst: u16,
        src: u16,
    },
    BitNot {
        dst: u16,
        src: u16,
    },
    TypeOf {
        dst: u16,
        src: u16,
    },
    Inc {
        dst: u16,
        src: u16,
    },
    Dec {
        dst: u16,
        src: u16,
    },
    ToKey {
        dst: u16,
        src: u16,
    },
    /// Joins registers from `start` on into a string, as a template does.
    Concat {
        dst: u16,
        start: u16,
        count: u16,
    },

    Jump {
        offset: i32,
    },
    /// A jump backwards, where it's safe to collect garbage.
    Loop {
        offset: i32,
    },
    JumpIfTrue {
        condition: u16,
        offset: i32,
    },
    JumpIfFalse {
        condition: u16,
        offset: i32,
    },
    JumpIfNullish {
        condition: u16,
        offset: i32,
    },
    JumpIfNotNullish {
        condition: u16,
        offset: i32,
    },
    JumpIfNotUndefined {
        condition: u16,
        offset: i32,
    },
    JumpIfUndefined {
        condition: u16,
        offset: i32,
    },
    JumpIfNull {
        condition: u16,
        offset: i32,
    },
    JumpIfNotNull {
        condition: u16,
        offset: i32,
    },

    // A comparison and a jump in one, as nearly every comparison goes straight to a jump.
    JumpLt {
        a: u16,
        b: u16,
        offset: i16,
    },
    JumpLe {
        a: u16,
        b: u16,
        offset: i16,
    },
    JumpGt {
        a: u16,
        b: u16,
        offset: i16,
    },
    JumpGe {
        a: u16,
        b: u16,
        offset: i16,
    },
    JumpNotLt {
        a: u16,
        b: u16,
        offset: i16,
    },
    JumpNotLe {
        a: u16,
        b: u16,
        offset: i16,
    },
    JumpNotGt {
        a: u16,
        b: u16,
        offset: i16,
    },
    JumpNotGe {
        a: u16,
        b: u16,
        offset: i16,
    },
    JumpStrictEq {
        a: u16,
        b: u16,
        offset: i16,
    },
    JumpStrictNe {
        a: u16,
        b: u16,
        offset: i16,
    },
    JumpEq {
        a: u16,
        b: u16,
        offset: i16,
    },
    JumpNe {
        a: u16,
        b: u16,
        offset: i16,
    },
    // The same, with a small number on the right.
    JumpLtInt {
        a: u16,
        value: i16,
        offset: i16,
    },
    JumpLeInt {
        a: u16,
        value: i16,
        offset: i16,
    },
    JumpGtInt {
        a: u16,
        value: i16,
        offset: i16,
    },
    JumpGeInt {
        a: u16,
        value: i16,
        offset: i16,
    },
    JumpNotLtInt {
        a: u16,
        value: i16,
        offset: i16,
    },
    JumpNotLeInt {
        a: u16,
        value: i16,
        offset: i16,
    },
    JumpNotGtInt {
        a: u16,
        value: i16,
        offset: i16,
    },
    JumpNotGeInt {
        a: u16,
        value: i16,
        offset: i16,
    },
    JumpStrictEqInt {
        a: u16,
        value: i16,
        offset: i16,
    },
    JumpStrictNeInt {
        a: u16,
        value: i16,
        offset: i16,
    },
    /// `typeof a == "..."`, which doesn't need the string. The kind is one of the `TYPE_` numbers.
    JumpTypeIs {
        a: u16,
        kind: u16,
        offset: i16,
    },
    JumpTypeIsNot {
        a: u16,
        kind: u16,
        offset: i16,
    },
    // The end of a loop with its test at the bottom: a comparison, a jump back, and a safe place to collect garbage.
    LoopLt {
        a: u16,
        b: u16,
        offset: i16,
    },
    LoopLe {
        a: u16,
        b: u16,
        offset: i16,
    },
    LoopGt {
        a: u16,
        b: u16,
        offset: i16,
    },
    LoopGe {
        a: u16,
        b: u16,
        offset: i16,
    },
    LoopLtInt {
        a: u16,
        value: i16,
        offset: i16,
    },
    LoopLeInt {
        a: u16,
        value: i16,
        offset: i16,
    },
    LoopGtInt {
        a: u16,
        value: i16,
        offset: i16,
    },
    LoopGeInt {
        a: u16,
        value: i16,
        offset: i16,
    },
    /// `while (a !== b)`, with its test at the bottom too.
    LoopStrictNe {
        a: u16,
        b: u16,
        offset: i16,
    },
    LoopStrictNeInt {
        a: u16,
        value: i16,
        offset: i16,
    },
    // The end of `for (...; i < n; i++)`, which is most loops: all of the loop's own work in one instruction.
    IncLoopLt {
        i: u16,
        limit: u16,
        offset: i16,
    },
    IncLoopLtInt {
        i: u16,
        limit: i16,
        offset: i16,
    },
    /// With `i < list.length`.
    IncLoopLtLength {
        i: u16,
        object: u16,
        offset: i16,
    },

    GetGlobal {
        dst: u16,
        cache: u16,
    },
    SetGlobal {
        src: u16,
        cache: u16,
    },
    /// Writing to a variable nobody declared makes it a global, in code that isn't strict.
    SetGlobalSloppy {
        src: u16,
        cache: u16,
    },
    TypeOfGlobal {
        dst: u16,
        cache: u16,
    },
    /// A script's `var`: a property of the global object, which stays as it is if it's already there.
    DeclareGlobalVar {
        name: u32,
    },
    /// A script's function declaration, which goes on the global object.
    DeclareGlobalFunction {
        src: u16,
        name: u32,
    },

    /// Puts a value in a new cell, for a variable that a function inside this one can see.
    NewCell {
        dst: u16,
        src: u16,
    },
    /// A new cell with what's in the old one, so each turn of a loop has its own variable.
    CopyCell {
        reg: u16,
    },
    GetCell {
        dst: u16,
        cell: u16,
    },
    /// Sets a variable that has to have started.
    SetCell {
        cell: u16,
        src: u16,
    },
    InitCell {
        cell: u16,
        src: u16,
    },
    GetUp {
        dst: u16,
        index: u16,
    },
    /// A capture that's the value itself rather than a cell (see `Capture::by_value`).
    GetUpValue {
        dst: u16,
        index: u16,
    },
    SetUp {
        index: u16,
        src: u16,
    },
    Closure {
        dst: u16,
        proto: u32,
    },
    LoadCallee {
        dst: u16,
    },

    NewObject {
        dst: u16,
        capacity: u16,
    },
    NewArray {
        dst: u16,
        capacity: u32,
    },
    ArrayPush {
        array: u16,
        value: u16,
    },
    ArrayHole {
        array: u16,
    },
    ArraySpread {
        array: u16,
        source: u16,
    },

    GetProp {
        dst: u16,
        object: u16,
        cache: u16,
    },
    SetProp {
        object: u16,
        value: u16,
        cache: u16,
    },
    /// GetProp at a place that has seen objects of more than one shape, which `Vm::learn` turns it into. It looks at
    /// the [`POLY`] more entries the place has as well as its first, and once it has seen more shapes than that, at
    /// what reads remember. It's an instruction of its own so that GetProp has no more code than one shape needs.
    GetPropPoly {
        dst: u16,
        object: u16,
        cache: u16,
    },
    /// SetProp at a place that has seen objects of more than one shape, as for GetPropPoly.
    SetPropPoly {
        object: u16,
        value: u16,
        cache: u16,
    },
    /// Makes a property on an object being built, whatever its prototype has.
    DefineField {
        object: u16,
        value: u16,
        cache: u16,
    },
    GetLength {
        dst: u16,
        object: u16,
        cache: u16,
    },
    GetElem {
        dst: u16,
        object: u16,
        key: u16,
    },
    SetElem {
        object: u16,
        key: u16,
        value: u16,
    },
    /// SetElem at a place that has just written to a typed array, which the interpreter turns it into
    /// (`Vm::go_typed_set`), so that SetElem has no code for writing one but the call it makes. Anything else turns
    /// it back.
    SetElemTyped {
        object: u16,
        key: u16,
        value: u16,
    },
    DefineElem {
        object: u16,
        key: u16,
        value: u16,
    },
    Delete {
        dst: u16,
        object: u16,
        key: u16,
    },
    ObjectSpread {
        object: u16,
        source: u16,
    },
    /// The properties of `source`, other than those whose keys are in the array `keys`.
    ObjectRest {
        dst: u16,
        source: u16,
        keys: u16,
    },
    DefineGetter {
        object: u16,
        key: u16,
        function: u16,
    },
    DefineSetter {
        object: u16,
        key: u16,
        function: u16,
    },
    DefineMethod {
        object: u16,
        key: u16,
        function: u16,
    },
    DefineClassGetter {
        object: u16,
        key: u16,
        function: u16,
    },
    DefineClassSetter {
        object: u16,
        key: u16,
        function: u16,
    },
    SetHome {
        function: u16,
        home: u16,
    },

    /// The function is in register `f`, `this` is in the one after, and the arguments follow.
    Call {
        dst: u16,
        f: u16,
        argc: u16,
    },
    /// `object.name()`: reads the method from the object in register `f + 1` into `f`, as GetProp does, and calls
    /// it with no arguments. One instruction rather than two, as calls like this are common.
    CallMethod {
        dst: u16,
        f: u16,
        cache: u16,
    },
    /// CallMethod at a place that has seen objects of more than one shape, as for GetPropPoly.
    CallMethodPoly {
        dst: u16,
        f: u16,
        cache: u16,
    },
    /// `Math.name(x)` or `Math.name(x, y)`, which is Call with `which` saying what it should be (its place in
    /// `builtins::math::QUICK`), and which works it out itself while it's still that function and given numbers.
    /// Anything else makes it Call, for good.
    CallMath {
        dst: u16,
        f: u16,
        which: u16,
    },
    /// A call with no object before it, whose `this` is undefined. That needs no instruction of its own to set up.
    CallPlain {
        dst: u16,
        f: u16,
        argc: u16,
    },
    New {
        dst: u16,
        f: u16,
        argc: u16,
    },
    CallSpread {
        dst: u16,
        f: u16,
        array: u16,
    },
    NewSpread {
        dst: u16,
        f: u16,
        array: u16,
    },
    SuperCall {
        dst: u16,
        f: u16,
        argc: u16,
    },
    SuperCallSpread {
        dst: u16,
        f: u16,
        array: u16,
    },
    Return {
        src: u16,
    },
    ReturnUndefined,
    Throw {
        src: u16,
    },
    CheckObjectCoercible {
        src: u16,
    },

    /// Takes two registers: an array and a position in it, or an iterator and its `next`.
    GetIterator {
        iter: u16,
        source: u16,
    },
    /// Jumps once there's nothing more.
    IterNext {
        value: u16,
        iter: u16,
        offset: i16,
    },
    IterClose {
        iter: u16,
    },
    /// Closes an iterator because of an error, so nothing it does matters.
    IterCloseQuietly {
        iter: u16,
    },
    /// Finds what `for await` goes through: an iterator and its `next`.
    GetAsyncIterator {
        iter: u16,
        source: u16,
    },
    /// Calls `next`, giving what to wait for.
    AsyncIterCall {
        dst: u16,
        iter: u16,
    },
    /// Looks at what `next` gave once it's been waited for. Its value goes in the register, and it jumps at the end.
    IterResult {
        value: u16,
        offset: i16,
    },
    /// Asks the iterator a `yield*` is going through for its next result, passing on what this generator was sent.
    DelegateNext {
        dst: u16,
        iter: u16,
        received: u16,
    },
    /// An array of what `for in` should go through.
    ForInKeys {
        dst: u16,
        source: u16,
    },

    CreateArguments {
        dst: u16,
    },
    Rest {
        dst: u16,
        start: u16,
    },
    NewTarget {
        dst: u16,
    },
    /// Sets up a class: the constructor is in `constructor`, the parent is in `parent` (or nothing), and the
    /// prototype goes in `dst`.
    NewClass {
        dst: u16,
        constructor: u16,
        parent: u16,
    },
    SetFields {
        constructor: u16,
        function: u16,
    },
    /// Says what private methods and accessors each new instance gets, which are on this object.
    SetPrivateMethods {
        constructor: u16,
        methods: u16,
    },
    RunFields {
        this: u16,
    },
    SuperGet {
        dst: u16,
        key: u16,
        this: u16,
    },
    SuperSet {
        key: u16,
        value: u16,
        this: u16,
    },
    CheckThis {
        reg: u16,
    },
    /// A function that isn't strict never sees `this` as nothing, or as something that's not an object.
    SloppyThis,
    ToObject {
        dst: u16,
        src: u16,
    },
    ThrowConstAssign {
        name: u32,
    },
    ThrowTdz {
        name: u32,
    },
    /// Where a statement starts, in code being debugged, where it can be paused. See [`crate::inspect`].
    Step {
        line: u32,
    },

    /// Starts a generator or an async function, by handing back an object with this frame saved in it.
    InitialYield,
    Yield {
        dst: u16,
        src: u16,
    },
    Await {
        dst: u16,
        src: u16,
    },
}

pub(crate) const TYPE_NUMBER: u16 = 0;
pub(crate) const TYPE_STRING: u16 = 1;
pub(crate) const TYPE_BOOLEAN: u16 = 2;
pub(crate) const TYPE_UNDEFINED: u16 = 3;
pub(crate) const TYPE_SYMBOL: u16 = 4;
pub(crate) const TYPE_BIGINT: u16 = 5;
pub(crate) const TYPE_FUNCTION: u16 = 6;
pub(crate) const TYPE_OBJECT: u16 = 7;

/// One place in the code that reads or writes a property, and what it last found.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cache {
    pub(crate) atom: Atom,
    pub(crate) shape: u32,
    /// For a read, the object it was found on, if not the object itself.
    /// For a write, the shape to change to, if it's adding a property.
    pub(crate) other: u32,
    pub(crate) slot: u32,
    pub(crate) epoch: u32,
    /// Once the place has seen a second shape, how far after this entry the [`POLY`] more it has are, which say the
    /// same for other shapes, with [`MEGAMORPHIC`] set once they're all in use too. `NONE` until then.
    pub(crate) more: u32,
}

/// How many entries a place in the code has for other shapes than its first's. A place that sees more shapes than
/// that is megamorphic: it then finds the rest in what reads remember (`Vm::remembered_value`).
pub(crate) const POLY: usize = 3;
/// Set in [`Cache::more`] when a place has seen more shapes than it has entries for.
pub(crate) const MEGAMORPHIC: u32 = 1 << 31;

impl Cache {
    pub(crate) fn new(atom: Atom) -> Self {
        Self {
            atom,
            shape: crate::object::NONE,
            other: crate::object::NONE,
            slot: 0,
            epoch: 0,
            more: crate::object::NONE,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Handler {
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) target: u32,
    pub(crate) reg: u16,
    /// A `catch` doesn't see a generator being told to return, though a `finally` has to run.
    pub(crate) is_catch: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Capture {
    /// Whether it's from a register of the function that makes this closure, or else from one of its own captures.
    pub(crate) from_register: bool,
    /// Whether what's captured is the value itself, of a parameter that's never given another (see `Rebinds` in
    /// compiler.rs), which GetUpValue reads, rather than a cell, which GetUp reads.
    pub(crate) by_value: bool,
    pub(crate) index: u16,
}

pub(crate) const IS_ARROW: u8 = 1;
pub(crate) const IS_CONSTRUCTOR: u8 = 2;
pub(crate) const IS_CLASS: u8 = 4;
pub(crate) const IS_DERIVED: u8 = 8;
pub(crate) const KEEPS_ARGS: u8 = 16;
pub(crate) const IS_GENERATOR: u8 = 32;
pub(crate) const IS_ASYNC: u8 = 64;
/// A method, which has no `prototype` of its own and can't be used with `new`.
pub(crate) const IS_METHOD: u8 = 128;

/// In a [`Proto`]'s `says`: whether it's strict code.
pub(crate) const SAYS_STRICT: u8 = 1;
/// In a [`Proto`]'s `says`: whether its code says `'use no memo'`, so that as a component it renders whenever what it's in
/// does, unless its props are the same plain values. See [`crate::Vm::says_no_memo`].
pub(crate) const SAYS_NO_MEMO: u8 = 2;
/// In a [`Proto`]'s `says`: whether it's in one of an app's own files, rather than in `node_modules` or what comes with the
/// engine, so that as a component it can skip rendering when it's given what does the same. See [`crate::Vm::made_by_app`].
pub(crate) const SAYS_APP_FILE: u8 = 4;

/// What a [`Proto`]'s `start` is until it's compiled, which is the first time it's called. It has `KEEPS_ARGS` until
/// then too, so that every way into it goes through `Vm::enter_function`, which compiles it.
pub(crate) const NOT_COMPILED: u32 = u32::MAX;

/// A compiled function.
#[derive(Debug)]
pub(crate) struct Proto {
    pub(crate) start: u32,
    pub(crate) cache_base: u32,
    pub(crate) registers: u16,
    pub(crate) params: u16,
    pub(crate) length: u8,
    pub(crate) flags: u8,
    /// What its code says of itself, as [`SAYS_STRICT`] and [`SAYS_NO_MEMO`], in one byte so that a proto stays 88 bytes.
    pub(crate) says: u8,
    /// How many registers after the parameters are the function's own variables, which a call makes undefined as
    /// it does the parameters it has no argument for, or [`ALL_LOCALS`]. See [`Proto::clear`].
    pub(crate) locals: u8,
    pub(crate) name: Atom,
    pub(crate) file: Atom,
    pub(crate) captures: Box<[Capture]>,
    pub(crate) capture_names: Box<[Atom]>,
    pub(crate) handlers: Box<[Handler]>,
    /// Which line each stretch of code came from, as pairs of where it starts and the line.
    pub(crate) lines: Box<[(u32, u32)]>,
}

/// Where one of a function's variables is, for a debugger, while its code is between `start` and `end`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DebugVar {
    pub(crate) name: Atom,
    pub(crate) reg: u16,
    /// Whether the register holds a cell the variable's in, as it does when an inner function can see it.
    pub(crate) cell: bool,
    pub(crate) start: u32,
    pub(crate) end: u32,
}

/// A function with this many variables or more has all its registers made undefined by a call.
pub(crate) const ALL_LOCALS: u8 = u8::MAX;

impl Proto {
    /// What `says` is for code that's strict or not, says `'use no memo'` or not, and is in an app's own file or not.
    pub(crate) fn says_of(strict: bool, no_memo: bool, app_file: bool) -> u8 {
        (if strict { SAYS_STRICT } else { 0 })
            | (if no_memo { SAYS_NO_MEMO } else { 0 })
            | (if app_file { SAYS_APP_FILE } else { 0 })
    }

    /// Whether it's strict code.
    #[inline(always)]
    pub(crate) fn strict(&self) -> bool {
        self.says & SAYS_STRICT != 0
    }

    /// Whether its code says `'use no memo'`.
    pub(crate) fn no_memo(&self) -> bool {
        self.says & SAYS_NO_MEMO != 0
    }

    /// How many registers from the bottom a call has to make undefined where no argument was given: `this`, the
    /// parameters and the function's own variables. The rest are temporaries and block variables, which are always
    /// written before they're read, so they can keep what an earlier call left there (see `Vm::wipe_above_top`).
    #[inline(always)]
    pub(crate) fn clear(&self) -> usize {
        if self.locals == ALL_LOCALS {
            self.registers as usize
        } else {
            1 + self.params as usize + self.locals as usize
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_instruction_is_eight_bytes() {
        assert_eq!(size_of::<Op>(), 8);
    }

    /// How big this is moves where the hot ones land in their pages, which can make calls 15% slower when a read of one
    /// lands on a register just written (richards, at 96 bytes). Measure before making it bigger.
    #[test]
    fn a_compiled_function_is_88_bytes() {
        assert_eq!(size_of::<Proto>(), 88);
    }
}
