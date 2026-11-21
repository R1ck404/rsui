//! The interpreter: one loop over instructions, on one stack of registers. A call from JavaScript to JavaScript
//! doesn't use Rust's stack, so deep recursion costs little and a call is cheap.

use std::collections::{HashMap, VecDeque};

use foldhash::fast::FixedState;

use crate::arena::Arena;
use crate::object::*;
use crate::op::*;
use crate::text::{self, Atom, AtomKind, Atoms, Strings, Text};
use crate::value::Value;

pub(crate) type Run<T> = Result<T, Value>;

pub(crate) const MAX_FRAMES: usize = 12000;
/// How much of Rust's stack the engine assumes it can have. A thread starts with a megabyte on Windows.
pub(crate) const DEFAULT_STACK_LIMIT: usize = 850 * 1024;

pub(crate) const CONSTRUCTING: u8 = 1;
/// The frame's function keeps its arguments, and they're on top of the list of those that are.
pub(crate) const KEPT_ARGS: u8 = 2;
/// The frame is a generator that `for of` carried on with in the interpreter it runs in (`Vm::step_in_place`), rather
/// than from Rust: its `yield` hands the value straight to that loop, and its end leaves it.
pub(crate) const RESUMED_HERE: u8 = 4;
/// With the JIT: the frame's function has compiled code, but the interpreter carries the frame on to its end, as it
/// goes through a generator in place better than compiled code can (see `jit::helpers::iter_next`).
#[cfg(feature = "jit")]
pub(crate) const INTERPRETED: u8 = 8;
/// There's no use for a stack of more values than this, which is two gigabytes of them.
const MAX_REGISTERS: usize = 1 << 28;

/// What a bitwise operator makes of a number, for any that isn't huge: a double whose whole part fits in 64 bits has
/// its low 32 bits as the answer, which is the wrapping the operators do without having to divide. Nothing for NaN,
/// infinities, numbers past 2^63 and what isn't a number.
#[inline(always)]
fn quick_int32(value: Value) -> Option<i32> {
    if value.is_int() {
        Some(value.as_int())
    } else if value.is_double() && value.as_double().abs() < 9_223_372_036_854_775_808.0 {
        Some(value.as_double() as i64 as i32)
    } else {
        None
    }
}

/// Copies a call's arguments from Rust onto the stack. The caller has almost always just written them one at a time,
/// and reading two of those at once (as LLVM likes to, for a copy) can't be answered from the writes still on their
/// way to memory: the read waits for them to land, which was about half of what a call from Rust cost. Copying them
/// one by one, each as one volatile write, can't be merged like that. Long lists are copied the usual way.
///
/// # Safety
///
/// `place` has room for all of `args`.
#[inline(always)]
#[allow(unsafe_code)]
unsafe fn write_args(place: *mut Value, args: &[Value]) {
    if args.len() > 8 {
        // SAFETY: the caller says there's room.
        unsafe { std::ptr::copy_nonoverlapping(args.as_ptr(), place, args.len()) };
        return;
    }
    for (at, arg) in args.iter().enumerate() {
        // SAFETY: as above.
        unsafe { place.add(at).write_volatile(*arg) };
    }
}

/// What a native function is called with. The arguments are on the register stack.
#[derive(Clone, Copy)]
pub struct Args {
    pub(crate) this: Value,
    pub(crate) start: usize,
    pub(crate) count: usize,
    pub(crate) new_target: Value,
    pub(crate) callee: u32,
}

/// A function Rust is going to call, with what its frame needs worked out by [`Vm::prepare_call`], which Rust that calls
/// it many times only has to do once. Anything but an everyday function (`proto` is `NONE`) goes the long way each time.
/// What's here stays right for as long as the function is alive, which whoever has its value makes sure of: a compiled
/// function's proto doesn't change, and its captured variables don't move.
#[derive(Clone, Copy)]
pub(crate) struct Prepared {
    pub(crate) function: Value,
    proto: u32,
    registers: u32,
    params: u32,
    clear: u32,
    start: u32,
    cache_base: u32,
    upvalues: *const Value,
}

#[derive(Clone, Copy)]
pub(crate) struct Frame {
    pub(crate) pc: u32,
    pub(crate) base: u32,
    pub(crate) top: u32,
    pub(crate) proto: u32,
    pub(crate) closure: u32,
    pub(crate) cache_base: u32,
    pub(crate) ret_dst: u16,
    pub(crate) flags: u8,
    pub(crate) new_target: Value,
    /// The closure's captured variables, which stay where they are for as long as the closure does.
    pub(crate) upvalues: *const Value,
}

/// Where a property was found on objects of a shape: on the object if `holder` is `NONE`, or else on that object.
#[derive(Clone, Copy)]
pub(crate) struct Lookup {
    pub(crate) shape: u32,
    pub(crate) atom: u32,
    pub(crate) holder: u32,
    pub(crate) slot: u32,
    pub(crate) epoch: u32,
}

/// How to make `this` for a `new`, as it was last time, which only needs checking.
#[derive(Clone, Copy)]
pub(crate) struct ThisCache {
    function: u32,
    shape: u32,
    slot: u32,
    proto: u32,
    root: u32,
}

/// What an `instanceof` found on functions of one shape: that their `Symbol.hasInstance` is the usual one, in the slot
/// `has_slot` of `holder` (`NONE` for the function itself), and where their own `prototype` is. It holds while no
/// prototype changes (the epoch), and the slots are read again each time, as a write to one doesn't change the epoch.
#[derive(Clone, Copy)]
pub(crate) struct InstanceOfCache {
    pub(crate) shape: u32,
    pub(crate) epoch: u32,
    pub(crate) holder: u32,
    pub(crate) has_slot: u32,
    pub(crate) prototype_slot: u32,
}

impl InstanceOfCache {
    pub(crate) const EMPTY: Self = Self {
        shape: NONE,
        epoch: 0,
        holder: NONE,
        has_slot: 0,
        prototype_slot: 0,
    };
}

/// The objects built-in code has to be able to find.
#[derive(Default, Clone, Copy)]
pub(crate) struct Realm {
    pub(crate) object_proto: u32,
    pub(crate) function_proto: u32,
    pub(crate) array_proto: u32,
    pub(crate) array_constructor: u32,
    pub(crate) string_proto: u32,
    pub(crate) number_proto: u32,
    pub(crate) boolean_proto: u32,
    pub(crate) symbol_proto: u32,
    pub(crate) error_proto: u32,
    pub(crate) type_error_proto: u32,
    pub(crate) range_error_proto: u32,
    pub(crate) reference_error_proto: u32,
    pub(crate) syntax_error_proto: u32,
    pub(crate) iterator_proto: u32,
    pub(crate) native_iterator_proto: u32,
    pub(crate) generator_proto: u32,
    pub(crate) promise_proto: u32,
    pub(crate) promise_constructor: u32,
    pub(crate) map_proto: u32,
    pub(crate) set_proto: u32,
    pub(crate) weak_map_proto: u32,
    pub(crate) weak_set_proto: u32,
    pub(crate) date_proto: u32,
    pub(crate) regexp_proto: u32,
    pub(crate) array_buffer_proto: u32,
    pub(crate) data_view_proto: u32,
    pub(crate) text_encoder_proto: u32,
    pub(crate) text_decoder_proto: u32,
    pub(crate) async_generator_proto: u32,
    pub(crate) bigint_proto: u32,
    pub(crate) weak_ref_proto: u32,
    pub(crate) finalization_registry_proto: u32,
    pub(crate) generator_function_proto: u32,
    pub(crate) async_function_proto: u32,
    pub(crate) async_generator_function_proto: u32,
    pub(crate) generator_function_shape: u32,
    pub(crate) async_function_shape: u32,
    pub(crate) async_generator_function_shape: u32,
    pub(crate) process_env: u32,
    pub(crate) object_shape: u32,
    pub(crate) array_shape: u32,
    pub(crate) function_shape: u32,
    pub(crate) array_values: Value0,
    pub(crate) suppressed_error_proto: u32,
    pub(crate) disposable_stack_proto: u32,
    pub(crate) async_disposable_stack_proto: u32,
    /// What compiled `using` code calls, once something's needed it. See `builtins/disposal.rs`.
    pub(crate) disposal: Value0,
    /// What compiled code calls to name a function from a key that's only known when it runs. See
    /// `builtins/object.rs`.
    pub(crate) name_function: Value0,
    /// What `setTimeout` and `setImmediate` give inherit from these, once there's been one.
    pub(crate) timeout_proto: Value0,
    pub(crate) immediate_proto: Value0,
}

/// A value with a default, for the realm to hold.
#[derive(Clone, Copy)]
pub(crate) struct Value0(pub(crate) Value);

impl Default for Value0 {
    fn default() -> Self {
        Self(Value::UNDEFINED)
    }
}

pub(crate) enum Job {
    /// A promise settled and it has someone waiting.
    Reaction {
        handler: Value,
        argument: Value,
        derived: Value,
        is_reject: bool,
    },
    /// A promise was resolved with something with a `then`.
    Thenable {
        promise: Value,
        thenable: Value,
        then: Value,
    },
    /// Something asked for with `queueMicrotask`.
    Call(Value),
}

pub(crate) struct Timer {
    pub(crate) id: u32,
    pub(crate) at: f64,
    pub(crate) callback: Value,
    pub(crate) args: Vec<Value>,
    pub(crate) repeat: Option<f64>,
    /// How long it waits, which `refresh` waits again.
    pub(crate) delay: f64,
    /// Whether it keeps the program going while it waits, which `unref` says it doesn't.
    pub(crate) refed: bool,
    /// The `Timeout` or `Immediate` that `setTimeout` or the like gave for it.
    pub(crate) object: Value,
}

/// That objects of this shape inherit their `constructor` from this slot of their prototype, for as long as no prototype
/// has changed shape. Looking it up is too slow for how often an array method wants to know.
#[derive(Clone, Copy)]
pub(crate) struct ConstructorMemo {
    pub(crate) shape: u32,
    pub(crate) epoch: u32,
    pub(crate) slot: u32,
}

/// How many numbers' strings [`Vm::number_strings`] keeps, in one place each by the number's bits.
pub(crate) const NUMBER_STRINGS: usize = 256;

/// The strings [`Vm::number_strings`] keeps, which it reads and changes as the array they're in, and what Rust that reads, makes
/// and fills objects many properties at a time remembers (see bulk.rs) beside them, in the same box. That's not about numbers;
/// it's there so that `Vm` is laid out as it was. In a field of its own (136 bytes, or a box of 8 at its end) it moved what
/// the interpreter reads of `Vm`, and js-bench's interpreter was 2% slower for it (prop_mono and prop_poly 7%, int_loop 6%),
/// or with the box 14% (fib 40%), without running any of it.
pub(crate) struct NumberStrings {
    strings: [(u64, u32); NUMBER_STRINGS],
    pub(crate) bulk: crate::bulk::Bulk,
}

impl std::ops::Deref for NumberStrings {
    type Target = [(u64, u32); NUMBER_STRINGS];

    fn deref(&self) -> &Self::Target {
        &self.strings
    }
}

impl std::ops::DerefMut for NumberStrings {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.strings
    }
}

pub struct Vm {
    pub(crate) objects: crate::gc::Objects,
    pub(crate) free_objects: crate::gc::FreeSlots,
    pub(crate) object_marks: Vec<u64>,
    pub(crate) strings: Strings,
    pub(crate) cells: crate::gc::Cells,
    pub(crate) free_cells: crate::gc::FreeSlots,
    pub(crate) generations: crate::gc::Generations,
    pub(crate) shapes: Shapes,
    pub(crate) atoms: Atoms,
    pub(crate) private_names: HashMap<String, Atom, FixedState>,
    pub(crate) code: Arena<Op>,
    pub(crate) protos: Arena<Proto>,
    pub(crate) caches: Arena<Cache>,
    pub(crate) consts: Arena<Value>,
    pub(crate) const_map: HashMap<u64, u32, FixedState>,
    /// What's free in `code`, `caches` and `protos`, and how much of the first two each function has. See
    /// [`crate::compiled`].
    pub(crate) code_room: crate::compiled::CodeRoom,
    /// The functions that are compiled the first time they're called, by proto. See [`crate::compiler::Uncompiled`].
    pub(crate) uncompiled: HashMap<u32, crate::compiler::Uncompiled, FixedState>,
    /// Where the variables are in each function a debugger can stop in. See [`crate::op::DebugVar`].
    pub(crate) debug_vars: HashMap<u32, Box<[crate::op::DebugVar]>, FixedState>,
    /// Which files' functions wait to be compiled until they're called. See [`crate::LazyCompile`].
    pub(crate) lazy_compile: crate::LazyCompile,
    /// What the code cache keeps: what's compiled, when packing, and the names it's loaded, when starting. See
    /// codecache.rs.
    pub(crate) code_cache: Option<Box<crate::codecache::CodeCache>>,
    /// The sources functions were compiled from, for their text. See `source_text.rs`.
    pub(crate) sources: crate::source_text::Sources,
    /// What files whose functions wait to be compiled are parsed into. See [`crate::compiler::KeptAst`].
    pub(crate) kept_memory: Option<std::rc::Rc<crate::ast::AstMemory>>,
    pub(crate) stack: Arena<Value>,
    pub(crate) frames: Arena<Frame>,
    /// The arguments of each frame whose function keeps them. Hardly any do.
    pub(crate) kept_args: Vec<Vec<Value>>,
    /// Lists from `kept_args` to use again, as most calls to such a function would otherwise make a new one.
    pub(crate) spare_args: Vec<Vec<Value>>,
    /// Patterns already worked out, by what they say and their flags, as a regular expression written in a function is made
    /// again each time it runs, and working one out is far more than looking it up.
    pub(crate) regex_programs:
        HashMap<(Vec<u16>, String), std::rc::Rc<crate::regex::Program>, FixedState>,
    this_cache: [ThisCache; 8],
    /// What `instanceof` found on the last few shapes of function it was given, by shape.
    pub(crate) instance_of_cache: [InstanceOfCache; 8],
    /// The strings of single ASCII characters, once they've been wanted.
    pub(crate) char_strings: Vec<Value>,
    /// The last string a regular expression looked at, as a list of units, and whose it was.
    pub(crate) regexp_input: (u32, Vec<u16>),
    /// The strings numbers were last turned into, by the number's bits, so that text made of the same numbers over and
    /// over (a chart's labels and sizes, `'' + 1.2`) makes and formats each once between collections, which empty it,
    /// as what it names may be freed by then. See [`Vm::number_string`].
    pub(crate) number_strings: Box<NumberStrings>,
    /// Where the last few proxy handlers keep their traps, by the handler's shape and the trap's name: (shape, name,
    /// slot). A collection empties it, as a shape it names may be freed and its number given to another.
    pub(crate) trap_cache: [(u32, u32, u32); 4],
    /// The last few parts of strings that were sliced off, as (string, start, end, part): see [`Vm::piece_of`]. A
    /// collection empties it.
    pub(crate) piece_cache: [(u32, u32, u32, u32); 8],
    /// What `exec` adds to the array it makes, so it needn't look it up each time.
    pub(crate) match_shape: u32,
    /// The shape of the `{ value, done }` that iterators hand back, once one's been made.
    pub(crate) iter_result_shape: u32,
    /// The shape of an `arguments` in sloppy code, once one's been made (`Vm::create_arguments`).
    pub(crate) arguments_shape: u32,
    pub(crate) regexp_spare: crate::regex::Spare,
    pub(crate) last_match: crate::regex::LastMatches,
    /// Where properties were last found, for reads that one place in the code doesn't see the same way each time.
    pub(crate) lookups: Vec<Lookup>,
    /// What's known about where arrays and promises get their `constructor`. See [`Vm::constructor_of`].
    pub(crate) constructor_memos: [ConstructorMemo; 2],
    /// The names `for in` and `Object.keys` find on the last few shapes they looked at.
    pub(crate) shape_keys: [(u32, Box<[Value]>); 8],
    /// A list `Object.assign` keeps for what's in each object it copies from (see `object_assign`).
    pub(crate) assign_entries: Vec<(Atom, u32, u8)>,
    /// Where `Array.prototype[Symbol.iterator]` is, which `for of` looks at each time it starts.
    pub(crate) array_iterator_slot: (u32, u32),
    pub(crate) top: usize,
    pub(crate) realm: Realm,
    pub(crate) global: u32,
    /// Goes up whenever something changes that a saved read through a prototype relied on.
    pub(crate) epoch: u32,
    /// Goes up whenever an `ArrayBuffer`'s memory moves or goes (it's resized or transferred), which is what the
    /// quick way to a typed array's elements relies on not having happened (see `TypedData::quick`). It never comes
    /// back round to a number a typed array could still have, as it would take 2^64 resizes.
    pub(crate) buffer_epoch: u64,
    /// What's been made since it was last safe to forget, which natives may be holding on to.
    pub(crate) nursery: Vec<Value>,
    pub(crate) run_mark: usize,
    /// How many times the interpreter has been started and hasn't finished: one while JavaScript runs that Rust started
    /// (an async function that a job carries on with, too, as the job's native holds nothing), and one more for each
    /// time a native, or the engine's own Rust, calls back into JavaScript. Those can be holding atoms that nothing else
    /// has, so a collection only frees atoms when this is one or less.
    pub(crate) nested_runs: u32,
    pub(crate) roots: Vec<Value>,
    /// Fixed bootstrap-only intrinsic descriptors for non-executing selector
    /// certificates. Holds no application data; at most 72 slots (1728 bytes).
    pub(crate) selector_intrinsics: Vec<crate::embed::SelectorIntrinsic>,
    /// What Rust is holding on to, through `Root`s.
    pub(crate) held: std::rc::Rc<crate::embed::Held>,
    pub(crate) jobs: VecDeque<Job>,
    /// What `WeakRef`s have handed out (or been made with) since the last time nothing was running, which stays alive
    /// until then, as the language says ([[KeptAlive]]). See [`Vm::keep_during_job`].
    pub(crate) kept: Vec<Value>,
    /// The `FinalizationRegistry`s with targets that have gone, whose callbacks run once the jobs have.
    pub(crate) cleanups: VecDeque<Value>,
    pub(crate) timers: Vec<Timer>,
    pub(crate) next_timer: u32,
    pub(crate) debt: usize,
    /// Whether a component is rendering, so that what it reads is noted (see `reads.rs`), which compiled code tests too.
    pub(crate) tracking: bool,
    /// What renders read: see `reads.rs`.
    pub(crate) reads: crate::reads::Reads,
    /// What renders worked out, kept for the next: see `keeping.rs`.
    pub(crate) keeping: crate::keeping::Keeping,
    /// Whether the natives being made are the language's own, which are pure (see `NativeData::pure`).
    pub(crate) making_pure: bool,
    pub(crate) threshold: usize,
    pub(crate) stack_base: usize,
    pub(crate) stack_limit: usize,
    /// An error from something that had no way to give it, like a proxy's handler while listing keys.
    pub(crate) deferred_error: Option<Value>,
    /// A write or a delete was just refused, rather than something going wrong in what it called.
    pub(crate) write_refused: bool,
    /// The parts of the standard library written in JavaScript, once something's asked for one.
    pub(crate) helpers: Value,
    /// An object only the engine has, whose one value is undefined and stays so. A place in the code that reads a
    /// property objects of some shape don't have, nor anything on the way up, remembers it as if it were on this one
    /// (`Vm::learn_absent`), so the interpreter finds undefined without asking. It's kept in `roots`.
    pub(crate) absent: u32,
    /// Whether anything has given a prototype elements of its own, which is almost never.
    pub(crate) prototypes_have_elements: bool,
    pub(crate) printer: Box<dyn FnMut(&str)>,
    pub(crate) gc_count: u32,
    pub(crate) yielded: crate::generator::Suspended,
    pub(crate) current_generator: u32,
    /// What `current_generator` was before each frame with `RESUMED_HERE` that's running, the innermost last.
    resumed_here: Vec<u32>,
    pub(crate) function_has_instance: Value,
    /// The function that `arguments.callee` is, in strict code: all it does is throw.
    pub(crate) throw_type_error: Value,
    pub(crate) unhandled_rejections: Vec<Value>,
    /// What's happening on other threads.
    pub(crate) background: crate::background::Background,
    pub(crate) symbol_registry: HashMap<String, Atom, FixedState>,
    pub(crate) joining: Vec<Value>,
    pub(crate) random_state: u64,
    pub(crate) root_cache: [(u32, u32); 16],
    /// Where each module that's been loaded is, while `require` is at work.
    pub(crate) modules: crate::modules::Modules,
    /// What the JIT has compiled, and what its code needs to find here.
    #[cfg(feature = "jit")]
    pub(crate) jit: crate::jit::Jit,
}

impl Vm {
    /// Whether promise jobs or microtasks are waiting to run, as one that a component queued while it rendered is.
    pub fn has_jobs(&self) -> bool {
        !self.jobs.is_empty()
    }

    pub(crate) fn new() -> Self {
        let mut strings = Strings::default();
        let atoms = Atoms::new(&mut strings);
        let mut stack = Arena::with_limit_at(MAX_REGISTERS, 2624);
        stack.grow_to(1024);
        let mut vm = Self {
            objects: crate::gc::Objects::new(Arena::default()),
            free_objects: Default::default(),
            object_marks: Vec::new(),
            strings,
            cells: crate::gc::Cells::new(Arena::with_limit_at(1 << 29, 3264)),
            free_cells: Default::default(),
            generations: Default::default(),
            shapes: Shapes::default(),
            atoms,
            private_names: HashMap::default(),
            code: Arena::default(),
            protos: Arena::with_limit(1 << 24),
            caches: Arena::with_limit_at(1 << 28, 640),
            consts: Arena::with_limit_at(1 << 28, 1984),
            const_map: HashMap::default(),
            code_room: Default::default(),
            uncompiled: HashMap::default(),
            debug_vars: HashMap::default(),
            lazy_compile: match std::env::var("RSUI_LAZY_COMPILE").as_deref() {
                Ok("all") => crate::LazyCompile::Always,
                Ok("packages") => crate::LazyCompile::NodeModules,
                _ => crate::LazyCompile::Never,
            },
            code_cache: None,
            sources: crate::source_text::Sources::default(),
            kept_memory: None,
            stack,
            frames: Arena::with_limit_at(MAX_FRAMES + 1, 1344),
            kept_args: Vec::new(),
            spare_args: Vec::new(),
            regex_programs: HashMap::default(),
            this_cache: [ThisCache {
                function: NONE,
                shape: 0,
                slot: 0,
                proto: 0,
                root: 0,
            }; 8],
            instance_of_cache: [InstanceOfCache::EMPTY; 8],
            char_strings: Vec::new(),
            regexp_input: (NONE, Vec::new()),
            number_strings: Box::new(NumberStrings { strings: [(0, NONE); NUMBER_STRINGS], bulk: crate::bulk::Bulk::default() }),
            trap_cache: [(NONE, 0, 0); 4],
            piece_cache: [(NONE, 0, 0, NONE); 8],
            match_shape: NONE,
            iter_result_shape: NONE,
            arguments_shape: NONE,
            regexp_spare: crate::regex::Spare::default(),
            last_match: crate::regex::LastMatches::default(),
            lookups: Vec::new(),
            constructor_memos: [ConstructorMemo {
                shape: u32::MAX,
                epoch: 0,
                slot: 0,
            }; 2],
            shape_keys: std::array::from_fn(|_| (NONE, Box::default())),
            assign_entries: Vec::new(),
            array_iterator_slot: (NONE, 0),
            top: 0,
            realm: Realm::default(),
            global: 0,
            epoch: 0,
            buffer_epoch: 1,
            nursery: Vec::with_capacity(64),
            run_mark: 0,
            nested_runs: 0,
            roots: Vec::new(),
            selector_intrinsics: Vec::new(),
            held: std::rc::Rc::default(),
            jobs: VecDeque::new(),
            kept: Vec::new(),
            cleanups: VecDeque::new(),
            timers: Vec::new(),
            next_timer: 0,
            debt: 0,
            tracking: false,
            reads: crate::reads::Reads::default(),
            keeping: crate::keeping::Keeping::default(),
            making_pure: false,
            threshold: crate::gc::first_threshold(),
            stack_base: crate::stack_position(),
            stack_limit: DEFAULT_STACK_LIMIT,
            deferred_error: None,
            write_refused: false,
            helpers: Value::UNDEFINED,
            absent: NONE,
            prototypes_have_elements: false,
            printer: Box::new(|text| {
                use std::io::Write;
                let _ = std::io::stdout().write_all(text.as_bytes());
            }),
            gc_count: 0,
            yielded: crate::generator::Suspended::None,
            current_generator: NONE,
            resumed_here: Vec::new(),
            function_has_instance: Value::UNDEFINED,
            throw_type_error: Value::UNDEFINED,
            unhandled_rejections: Vec::new(),
            background: crate::background::Background::new(),
            symbol_registry: HashMap::default(),
            joining: Vec::new(),
            random_state: 0,
            root_cache: [(NONE - 1, 0); 16],
            modules: crate::modules::Modules::default(),
            #[cfg(feature = "jit")]
            jit: crate::jit::Jit::new(),
        };
        vm.install_builtins();
        vm.capture_selector_intrinsics();
        #[cfg(feature = "counts")]
        crate::counts::install(&mut vm);
        vm.nursery.clear();
        vm.debt = 0;
        vm
    }

    /// How much more of Rust's stack there is to use.
    pub(crate) fn stack_room(&self) -> usize {
        let used = self.stack_base.abs_diff(crate::stack_position());
        self.stack_limit.saturating_sub(used)
    }

    pub(crate) fn memory(&self) -> crate::MemoryUse {
        let mut objects = self.objects.len() * size_of::<Object>();
        objects += self.selector_intrinsics.capacity() * size_of::<crate::embed::SelectorIntrinsic>();
        for index in 0..self.objects.len() {
            // A place on a page that's been given back has no memory to read (see `giveback.rs`).
            if self.free_objects.holes().has(index) {
                continue;
            }
            let object = &self.objects[index];
            objects += object.slots_heap_bytes();
            match &object.kind {
                Kind::Array(items) | Kind::Arguments(items) => objects += items.capacity() * 8,
                Kind::Function { upvalues, .. } => objects += upvalues.len() * 8,
                Kind::Native(_) => objects += size_of::<NativeData>(),
                Kind::ArrayBuffer(data) => objects += data.bytes.capacity(),
                _ => {}
            }
        }
        objects += self.cells.len() * 8;
        let mut strings = self.strings.items.len() * size_of::<crate::text::StringData>();
        let tally = crate::text::Tally::new();
        for index in 0..self.strings.items.len() {
            if self.strings.free.holes().has(index) {
                continue;
            }
            let string = &self.strings.items[index];
            strings += string.text.heap_bytes_in(tally);
        }
        crate::MemoryUse {
            objects,
            strings,
            shapes: self.shapes.memory() + self.number_strings.bulk.memory(),
            code: self.code_bytes(),
            stack: self.stack.capacity() * 8 + self.frames.capacity() * size_of::<Frame>(),
        }
    }

    // Making things.

    pub(crate) fn alloc_object(&mut self, shape: u32, kind: Kind, capacity: usize) -> Value {
        let value = self.alloc_unrooted(shape, kind, capacity);
        self.nursery.push(value);
        value
    }

    /// An object that goes straight into a register, so nothing has to remember it in the meantime.
    #[inline]
    #[allow(unsafe_code)]
    pub(crate) fn alloc_unrooted(&mut self, shape: u32, kind: Kind, capacity: usize) -> Value {
        #[cfg(feature = "counts")]
        crate::counts::made(&kind);
        let object = Object::new(shape, kind, capacity);
        self.debt += size_of::<Object>() + object.slots_heap_bytes();
        let handle = if let Some(handle) = self.free_objects.pop() {
            debug_assert!(matches!(self.objects[handle as usize].kind, Kind::Free));
            // SAFETY: what's on the free list is in the list of objects, and a free place has nothing to drop.
            unsafe { self.objects.as_mut_ptr().add(handle as usize).write(object) };
            handle
        } else if self.free_objects.holes().pages() != 0 {
            // Pages given back after a full collection are used again before the list grows (see `giveback.rs`).
            match self.place_object_in_hole(object) {
                Some(handle) => handle,
                None => unreachable!("a page given back has free places once it's taken back"),
            }
        } else {
            self.objects.push(object);
            (self.objects.len() - 1) as u32
        };
        Value::object(handle)
    }

    /// [`Vm::alloc_unrooted`] for the JIT's helpers, as a function of its own: if they called that one too, LLVM would
    /// weigh its inlining into the interpreter's loop differently from a build without the JIT, and did, which made
    /// the interpreter's allocations calls.
    #[cfg(feature = "jit")]
    #[inline(never)]
    #[allow(unsafe_code)]
    pub(crate) fn alloc_for_jit(&mut self, shape: u32, kind: Kind, capacity: usize) -> Value {
        let object = Object::new(shape, kind, capacity);
        self.debt += size_of::<Object>() + object.slots_heap_bytes();
        let handle = if let Some(handle) = self.free_objects.pop() {
            // SAFETY: as in `alloc_unrooted`.
            unsafe { self.objects.as_mut_ptr().add(handle as usize).write(object) };
            handle
        } else if self.free_objects.holes().pages() != 0 {
            match self.place_object_in_hole(object) {
                Some(handle) => handle,
                None => unreachable!("a page given back has free places once it's taken back"),
            }
        } else {
            self.objects.push(object);
            (self.objects.len() - 1) as u32
        };
        Value::object(handle)
    }

    pub(crate) fn new_plain(&mut self) -> Value {
        self.alloc_object(self.realm.object_shape, Kind::Plain, 0)
    }

    pub(crate) fn new_object_with_proto(&mut self, proto: u32, kind: Kind) -> Value {
        let shape = self.root_shape(proto);
        self.alloc_object(shape, kind, 0)
    }

    pub(crate) fn root_shape(&mut self, proto: u32) -> u32 {
        // Almost every `new` asks for one of the same few, so there's no need to look them up each time. There are
        // sixteen, as with four, an async function's promise and the generator it runs in kept pushing each other out.
        let way = proto as usize % self.root_cache.len();
        if self.root_cache[way].0 == proto {
            return self.root_cache[way].1;
        }
        if proto != NONE {
            self.objects[proto as usize].flags |= IS_PROTO;
        }
        let shape = self.shapes.root(proto);
        self.root_cache[way] = (proto, shape);
        shape
    }

    /// Forgets the few shapes and handles kept here to save looking them up, which a collection could have freed: a
    /// shape's place, or an object's, can be used again for something else after one. They're found again quickly.
    pub(crate) fn forget_shape_caches(&mut self) {
        for cached in &mut self.this_cache {
            cached.function = NONE;
        }
        self.instance_of_cache = [InstanceOfCache::EMPTY; 8];
        self.root_cache = [(NONE - 1, 0); 16];
        for memo in &mut self.constructor_memos {
            memo.shape = u32::MAX;
        }
        for keys in &mut self.shape_keys {
            if keys.0 != NONE {
                *keys = (NONE, Box::default());
            }
        }
        self.array_iterator_slot = (NONE, 0);
    }

    pub fn new_array(&mut self, items: Vec<Value>) -> Value {
        if items.len() <= 4 {
            let value = self.small_array(&items);
            self.nursery.push(value);
            return value;
        }
        self.debt += items.capacity() * 8;
        self.alloc_object(self.realm.array_shape, Kind::Array(items.into()), 0)
    }

    /// An array with its few elements (four at most) inside its own object.
    #[allow(unsafe_code)]
    pub(crate) fn small_array(&mut self, values: &[Value]) -> Value {
        let array = self.alloc_unrooted(self.realm.array_shape, Kind::Array(Items::empty()), 0);
        // SAFETY: the object was just put there, and nothing else is looking at it. It's new, so it's young, and needs
        // no write barrier.
        let object = unsafe { &mut *self.objects.as_mut_ptr().add(array.index()) };
        object.keep_items_inside_with(values);
        array
    }

    pub(crate) fn new_text(&mut self, text: Text) -> Value {
        self.debt += text.heap_bytes() + 32;
        let value = Value::string(self.strings.add(text));
        self.nursery.push(value);
        value
    }

    pub(crate) fn new_str(&mut self, text: &str) -> Value {
        self.new_text(Text::from_str(text))
    }

    pub(crate) fn new_cell(&mut self, value: Value) -> Value {
        self.debt += 8;
        let handle = if let Some(handle) = self.free_cells.pop() {
            self.cells[handle as usize] = value;
            handle
        } else if let Some(handle) = (self.free_cells.holes().pages() != 0)
            .then(|| self.place_cell_in_hole(value))
            .flatten()
        {
            handle
        } else {
            self.cells.push(value);
            (self.cells.len() - 1) as u32
        };
        Value::cell(handle)
    }

    pub(crate) fn new_native(
        &mut self,
        name: Atom,
        length: u8,
        func: NativeFn,
        data: Value,
        is_constructor: bool,
    ) -> Value {
        // What natives are called stays, as these are made by the engine and by Rust, which name only so many things.
        self.atoms.pin(name);
        let value = self.alloc_object(
            self.realm.function_shape,
            Kind::Native(Box::new(NativeData {
                func,
                data,
                name,
                length,
                is_constructor,
                forwards: 0,
                pure: self.making_pure,
            })),
            0,
        );
        self.objects[value.index()].flags |= LAZY_FUNCTION;
        value
    }

    // Names.

    pub(crate) fn intern_str(&mut self, name: &str) -> Atom {
        self.atoms.intern(&mut self.strings, Text::from_str(name))
    }

    pub(crate) fn intern_units(&mut self, units: &[u16]) -> Atom {
        self.atoms
            .intern(&mut self.strings, Text::from_units(units))
    }

    /// A name made up while running, such as a key that was worked out or read from JSON, which goes once nothing has
    /// it. [`intern_str`](Self::intern_str)'s stay.
    pub(crate) fn intern_key_str(&mut self, name: &str) -> Atom {
        self.atoms.intern_key(&mut self.strings, Text::from_str(name))
    }

    /// A private name that's different from any other, even one called the same. It's for the code, so it stays.
    pub(crate) fn new_private_atom(&mut self, name: &str) -> Atom {
        let description = self.strings.add(Text::from_str(name));
        let atom = self.atoms.new_symbol(Some(description), AtomKind::Hidden);
        self.atoms.pin(atom);
        atom
    }

    pub(crate) fn private_atom(&mut self, name: &str) -> Atom {
        if let Some(atom) = self.private_names.get(name) {
            return *atom;
        }
        let atom = self.new_private_atom(name);
        self.private_names.insert(name.to_owned(), atom);
        atom
    }

    /// The string that names an atom, or the symbol that it is.
    pub(crate) fn atom_value(&mut self, atom: Atom) -> Value {
        if let Some(index) = atom.held_index() {
            return self.new_str(&index.to_string());
        }
        match self.atoms.kind_of(atom) {
            AtomKind::Name => Value::string(self.atoms.string_of(atom).unwrap_or(0)),
            _ => Value::symbol(atom.0),
        }
    }

    pub(crate) fn add_const(&mut self, value: Value) -> u32 {
        if let Some(index) = self.const_map.get(&value.0) {
            return *index;
        }
        let index = self.consts.len() as u32;
        self.consts.push(value);
        self.const_map.insert(value.0, index);
        index
    }

    pub(crate) fn bigint_literal(&mut self, digits: &str, radix: u32) -> Value {
        let data = crate::builtins::BigData::parse_digits(digits, radix).unwrap_or_default();
        let value = self.new_bigint(data);
        if let Some(cache) = self.code_cache.as_mut().filter(|cache| cache.recording) {
            cache.bigints.push((value.0, digits.to_owned(), radix));
        }
        value
    }

    pub(crate) fn template_object(&mut self, strings: &[&[u16]], raw: &[&[u16]]) -> Value {
        let cooked: Vec<Value> = strings
            .iter()
            .map(|units| self.new_text(Text::from_units(units)))
            .collect();
        let raw_items: Vec<Value> = raw
            .iter()
            .map(|units| self.new_text(Text::from_units(units)))
            .collect();
        let object = self.new_array(cooked);
        let raw_array = self.new_array(raw_items);
        self.objects[raw_array.index()].flags |= FROZEN_ELEMENTS | NOT_EXTENSIBLE;
        self.define(object.handle(), text::RAW, raw_array, 0);
        self.objects[object.index()].flags |= FROZEN_ELEMENTS | NOT_EXTENSIBLE;
        if let Some(cache) = self.code_cache.as_mut().filter(|cache| cache.recording) {
            let strings = strings.iter().map(|units| units.to_vec()).collect();
            let raw = raw.iter().map(|units| units.to_vec()).collect();
            cache.templates.push((object.0, strings, raw));
        }
        // Each place in the code has one for good, and constants all stay.
        object
    }

    // Errors.

    #[cold]
    #[inline(never)]
    pub(crate) fn make_error(&mut self, proto: u32, message: &str) -> Value {
        let error = self.new_object_with_proto(proto, Kind::Error);
        let message = self.new_str(message);
        self.define(error.handle(), text::MESSAGE, message, HIDDEN);
        // Reading a built-in error's name and message can't go wrong, as this one was only just made.
        let _ = crate::builtins::attach_full_stack(self, error);
        error
    }

    pub(crate) fn stack_trace(&mut self) -> String {
        let mut out = String::new();
        let count = self.frames.len();
        for frame in self.frames[count.saturating_sub(16)..].iter().rev() {
            let proto = &self.protos[frame.proto as usize];
            let at = frame.pc.saturating_sub(proto.start).saturating_sub(1);
            let line = match proto.lines.partition_point(|entry| entry.0 <= at) {
                0 => 0,
                after => proto.lines[after - 1].1,
            };
            let name = self.atom_text(proto.name);
            let file = self.atom_text(proto.file);
            if name.is_empty() {
                out.push_str(&format!("    at {file}:{line}\n"));
            } else {
                out.push_str(&format!("    at {name} ({file}:{line})\n"));
            }
        }
        out.pop();
        out
    }

    pub(crate) fn atom_text(&self, atom: Atom) -> String {
        if let Some(index) = atom.held_index() {
            return index.to_string();
        }
        match self.atoms.string_of(atom) {
            Some(handle) => self.strings.items[handle as usize].text.to_rust(),
            None => String::new(),
        }
    }

    #[cold]
    #[inline(never)]
    pub fn type_error(&mut self, message: &str) -> Value {
        self.make_error(self.realm.type_error_proto, message)
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn range_error(&mut self, message: &str) -> Value {
        self.make_error(self.realm.range_error_proto, message)
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn reference_error(&mut self, message: &str) -> Value {
        self.make_error(self.realm.reference_error_proto, message)
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn syntax_error(&mut self, message: &str) -> Value {
        self.make_error(self.realm.syntax_error_proto, message)
    }

    // Calling.

    #[inline]
    pub(crate) fn arg(&self, args: &Args, index: usize) -> Value {
        if index < args.count {
            self.stack[args.start + index]
        } else {
            Value::UNDEFINED
        }
    }

    /// Calls a function from Rust.
    pub(crate) fn call(&mut self, function: Value, this: Value, args: &[Value]) -> Run<Value> {
        #[cfg(feature = "jsprof")]
        if self.frames.is_empty() {
            crate::jsprof::discard_idle_tick();
        }
        // What's made counts as JavaScript's, when that's measured: see `owners.rs` in rsui-alloc.
        #[cfg(feature = "owners")]
        let _owner = rsui_alloc::owners::scope(rsui_alloc::owners::JS);
        // Both are inlined, so what's worked out stays in registers rather than being written down and read back.
        let prepared = self.prepare_call(function);
        self.enter_prepared(&prepared, this, args)
    }

    /// Works out what calling `function` takes, for [`Vm::call_prepared`], which Rust that calls the same function over
    /// and over (an array method's callback, a sort's comparison) only has to do once. See [`Prepared`].
    #[inline(always)]
    pub(crate) fn prepare_call(&mut self, function: Value) -> Prepared {
        let mut prepared = Prepared {
            function,
            proto: NONE,
            registers: 0,
            params: 0,
            clear: 0,
            start: 0,
            cache_base: 0,
            upvalues: std::ptr::null(),
        };
        if !function.is_object() {
            return prepared;
        }
        let Kind::Function {
            proto, upvalues, ..
        } = &self.objects[function.index()].kind
        else {
            return prepared;
        };
        let proto_id = *proto;
        let upvalues = upvalues.as_ptr();
        let proto = &self.protos[proto_id as usize];
        // A function that's yet to be compiled keeps its arguments until it is, so it goes the long way the first time.
        // Calling from this deep in Rust's stack goes the long way too, which says when it's too deep. Rust's stack is
        // as deep for every call made with this, so it only has to be looked at once.
        if proto.flags & (IS_CLASS | KEEPS_ARGS) != 0
            || self.stack_base.abs_diff(crate::stack_position()) > self.stack_limit / 10 * 7
        {
            return prepared;
        }
        prepared.proto = proto_id;
        prepared.registers = u32::from(proto.registers);
        prepared.params = u32::from(proto.params);
        prepared.clear = proto.clear() as u32;
        prepared.start = proto.start;
        prepared.cache_base = proto.cache_base;
        prepared.upvalues = upvalues;
        prepared
    }

    /// Calls what [`Vm::prepare_call`] worked out. For an everyday function this only writes the arguments, makes the
    /// frame and runs it.
    #[inline(never)]
    pub(crate) fn call_prepared(
        &mut self,
        prepared: &Prepared,
        this: Value,
        args: &[Value],
    ) -> Run<Value> {
        self.enter_prepared(prepared, this, args)
    }

    /// [`Vm::call_prepared`], inlined into [`Vm::call`].
    #[inline(always)]
    #[allow(unsafe_code)]
    fn enter_prepared(&mut self, prepared: &Prepared, this: Value, args: &[Value]) -> Run<Value> {
        let start = self.top;
        let new_base = start + 1;
        let top = new_base + prepared.registers as usize;
        if prepared.proto == NONE
            || top.max(new_base + 1 + args.len()) + 16 > self.stack.len()
            || self.frames.len() >= self.frames.capacity()
        {
            return self.call_slowly(prepared.function, this, args);
        }
        // SAFETY: the stack reaches past the frame's registers and the arguments, as was just checked.
        unsafe {
            let place = self.stack.as_mut_ptr().add(start);
            place.write(prepared.function);
            place.add(1).write(this);
            write_args(place.add(2), args);
            // Only the parameters with no argument and the function's own variables: the rest are written before
            // they're read.
            let registers_at = self.stack.as_mut_ptr().add(new_base);
            for at in 1 + args.len().min(prepared.params as usize)..prepared.clear as usize {
                registers_at.add(at).write(Value::UNDEFINED);
            }
        }
        self.top = new_base + 1 + args.len();
        let stop = self.frames.len();
        #[cfg(feature = "jsprof")]
        crate::jsprof::maybe_sample(self);
        // SAFETY: the list of frames has room for one more, as was checked.
        unsafe {
            self.frames.as_mut_ptr().add(stop).write(Frame {
                pc: prepared.start,
                base: new_base as u32,
                top: top as u32,
                proto: prepared.proto,
                closure: prepared.function.handle(),
                cache_base: prepared.cache_base,
                ret_dst: 0,
                flags: 0,
                new_target: Value::UNDEFINED,
                upvalues: prepared.upvalues,
            });
            self.frames.set_len(stop + 1);
        }
        #[cfg(feature = "counts")]
        crate::counts::call(self, prepared.proto);
        let saved_mark = self.run_mark;
        self.run_mark = self.nursery.len();
        self.nested_runs += 1;
        // With the JIT, in the function's compiled code if it has some.
        #[cfg(feature = "jit")]
        let result = self.run_frame(stop);
        #[cfg(not(feature = "jit"))]
        let result = self.run_loop(stop);
        self.nested_runs -= 1;
        self.run_mark = saved_mark;
        self.top = start;
        if let Ok(value) = result {
            // Whoever called this might hold on to it through another call.
            if value.is_object() || value.is_string() || value.is_bigint() {
                self.nursery.push(value);
            }
        }
        result
    }

    /// [`Vm::call`] for anything but an everyday function, or when the stack or the list of frames has to grow.
    #[inline(never)]
    #[allow(unsafe_code)]
    fn call_slowly(&mut self, function: Value, this: Value, args: &[Value]) -> Run<Value> {
        let start = self.top;
        let needed = start + args.len() + 2;
        self.reserve_stack(needed + 16)?;
        // SAFETY: there's room for all of this now, as the last line made sure.
        unsafe {
            let place = self.stack.as_mut_ptr().add(start);
            place.write(function);
            place.add(1).write(this);
            write_args(place.add(2), args);
        }
        self.top = needed;
        let result = self.call_at(start, args.len(), None);
        self.top = start;
        if let Ok(value) = result {
            // Whoever called this might hold on to it through another call.
            if value.is_object() || value.is_string() || value.is_bigint() {
                self.nursery.push(value);
            }
        }
        result
    }

    /// Makes sure the stack reaches this far.
    #[inline]
    pub(crate) fn reserve_stack(&mut self, needed: usize) -> Run<()> {
        if needed > self.stack.len() && !self.stack.grow_to(needed) {
            return Err(self.range_error("Maximum call stack size exceeded"));
        }
        Ok(())
    }

    pub(crate) fn construct(&mut self, function: Value, args: &[Value]) -> Run<Value> {
        let start = self.top;
        let needed = start + args.len() + 2;
        self.reserve_stack(needed + 16)?;
        self.stack[start] = function;
        self.stack[start + 1] = Value::UNDEFINED;
        self.stack[start + 2..needed].copy_from_slice(args);
        self.top = needed;
        let result = self.call_at(start, args.len(), Some(function));
        self.top = start;
        if let Ok(value) = result {
            self.nursery.push(value);
        }
        result
    }

    /// Calls what's at `start` on the stack, with `this` after it and then the arguments.
    /// With a `new_target` it's a `new`. The top of the stack has to be past the arguments.
    #[inline(never)]
    pub(crate) fn call_at(
        &mut self,
        start: usize,
        argc: usize,
        new_target: Option<Value>,
    ) -> Run<Value> {
        let mut function = self.stack[start];
        if !function.is_object() {
            return Err(self.not_callable(function, new_target.is_some()));
        }
        let mut argc = argc;
        // A bound function, `call` or `apply` is the call it stands for made in its place, rather than a call from
        // here again (see `forward_in_place`).
        if new_target.is_none()
            && match &self.objects[function.index()].kind {
                Kind::Bound(_) => true,
                Kind::Native(native) => native.forwards != 0,
                _ => false,
            }
        {
            argc = self.forward_in_place(start, argc)?;
            function = self.stack[start];
            if !function.is_object() {
                return Err(self.not_callable(function, false));
            }
        }
        match &self.objects[function.index()].kind {
            Kind::Function { proto, .. } => {
                let proto = *proto;
                let flags = self.protos[proto as usize].flags;
                let this_value = match new_target {
                    Some(target) => {
                        if flags & IS_CONSTRUCTOR == 0 {
                            return Err(self.not_callable(function, true));
                        }
                        if flags & IS_DERIVED != 0 {
                            Value::EMPTY
                        } else {
                            self.create_this(target)?
                        }
                    }
                    None => {
                        if flags & IS_CLASS != 0 {
                            let name = self.atom_text(self.protos[proto as usize].name);
                            return Err(self.type_error(&format!(
                                "Class constructor {name} cannot be invoked without 'new'"
                            )));
                        }
                        self.stack[start + 1]
                    }
                };
                self.stack[start + 1] = this_value;
                let stop = self.frames.len();
                self.enter_function(
                    function.handle(),
                    proto,
                    start + 1,
                    argc,
                    0,
                    if new_target.is_some() {
                        CONSTRUCTING
                    } else {
                        0
                    },
                    new_target.unwrap_or(Value::UNDEFINED),
                )?;
                self.run(stop)
            }
            Kind::Native(native) => {
                let func = native.func;
                let is_constructor = native.is_constructor;
                if self.keeping.working != 0 && !native.pure {
                    self.keeping.spoiled = true;
                }
                #[cfg(feature = "counts")]
                crate::counts::native(self, func as usize, native.name);
                if new_target.is_some() && !is_constructor {
                    return Err(self.not_callable(function, true));
                }
                let args = Args {
                    this: self.stack[start + 1],
                    start: start + 2,
                    count: argc,
                    new_target: new_target.unwrap_or(Value::UNDEFINED),
                    callee: function.handle(),
                };
                func(self, args)
            }
            Kind::Bound(bound) => {
                let target = bound.target;
                let this = bound.this;
                let extra = bound.args.clone();
                let mut args = Vec::with_capacity(extra.len() + argc);
                args.extend_from_slice(&extra);
                args.extend_from_slice(&self.stack[start + 2..start + 2 + argc]);
                match new_target {
                    Some(_) => self.construct(target, &args),
                    None => self.call(target, this, &args),
                }
            }
            Kind::Proxy(_) => {
                let handle = function.handle();
                let this = self.stack[start + 1];
                let args = self.stack[start + 2..start + 2 + argc].to_vec();
                match new_target {
                    Some(target) => self.proxy_construct(handle, &args, target),
                    None => self.proxy_call(handle, this, &args),
                }
            }
            _ => Err(self.not_callable(function, new_target.is_some())),
        }
    }

    /// A call from the interpreter that isn't to an everyday function, at `start` on the stack as for
    /// [`Vm::call_at`]. A bound function, `call` and `apply` are only ways of calling another function, so when that
    /// one is a compiled function this makes the call it stands for in their place on the stack and makes a frame for
    /// it, with its answer going to `dst`, and fails with [`Value::ENTERED`]: the interpreter carries on in the new
    /// frame, rather than Rust starting the interpreter again to run it. Anything else is called with [`Vm::call_at`].
    #[inline(never)]
    pub(crate) fn call_indirect(&mut self, start: usize, argc: usize, dst: u16) -> Run<Value> {
        let argc = self.forward_in_place(start, argc)?;
        let function = self.stack[start];
        if function.is_object()
            && let Kind::Function { proto, .. } = &self.objects[function.index()].kind
        {
            let proto = *proto;
            if self.protos[proto as usize].flags & IS_CLASS == 0 {
                self.enter_function(
                    function.handle(),
                    proto,
                    start + 1,
                    argc,
                    dst,
                    0,
                    Value::UNDEFINED,
                )?;
                return Err(Value::ENTERED);
            }
        }
        self.call_at(start, argc, None)
    }

    /// Makes a call to a bound function, `call` or `apply` at `start` on the stack the call it stands for, as many
    /// times over as that's one of them again, and gives how many arguments it has then. Each step is the same call
    /// written another way, so it can stop at any of them: at `apply` of something that isn't a plain list, say.
    /// `self.top` is left past the arguments.
    fn forward_in_place(&mut self, start: usize, mut argc: usize) -> Run<usize> {
        loop {
            let function = self.stack[start];
            if !function.is_object() {
                break;
            }
            match &self.objects[function.index()].kind {
                Kind::Native(native) if native.forwards == FORWARDS_CALL => {
                    // f.call(this, ...args): f moves down to where `call` was, and its arguments down one.
                    let at = start + 1;
                    self.stack[start] = self.stack[at];
                    if argc == 0 {
                        self.stack[at] = Value::UNDEFINED;
                    } else {
                        self.stack.copy_within(at + 1..at + 1 + argc, at);
                        argc -= 1;
                    }
                }
                Kind::Native(native) if native.forwards == FORWARDS_APPLY => {
                    let this = if argc > 0 {
                        self.stack[start + 2]
                    } else {
                        Value::UNDEFINED
                    };
                    let list = if argc > 1 {
                        self.stack[start + 3]
                    } else {
                        Value::UNDEFINED
                    };
                    let count = if list.is_nullish() {
                        0
                    } else if list.is_object()
                        && let Kind::Array(items) | Kind::Arguments(items) =
                            &self.objects[list.index()].kind
                    {
                        // A hole is read as a property (which could be an accessor, or on a prototype), so a list
                        // with any goes the long way.
                        if items.iter().any(|item| item.is_empty()) {
                            break;
                        }
                        items.len()
                    } else {
                        break;
                    };
                    self.reserve_stack(start + 2 + count + 16)?;
                    self.stack[start] = self.stack[start + 1];
                    self.stack[start + 1] = this;
                    if count > 0 {
                        let (Kind::Array(items) | Kind::Arguments(items)) =
                            &self.objects[list.index()].kind
                        else {
                            unreachable!("it was a list just now");
                        };
                        self.stack[start + 2..start + 2 + count].copy_from_slice(items);
                    }
                    argc = count;
                }
                Kind::Bound(bound) => {
                    // The bound arguments go in front of the ones given.
                    let (target, this, extra) = (bound.target, bound.this, bound.args.len());
                    self.reserve_stack(start + 2 + argc + extra + 16)?;
                    let Kind::Bound(bound) = &self.objects[function.index()].kind else {
                        unreachable!("it was bound just now");
                    };
                    let first = start + 2;
                    self.stack.copy_within(first..first + argc, first + extra);
                    self.stack[first..first + extra].copy_from_slice(&bound.args);
                    self.stack[start] = target;
                    self.stack[start + 1] = this;
                    argc += extra;
                }
                _ => break,
            }
        }
        self.top = self.top.max(start + 2 + argc);
        Ok(argc)
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn not_callable(&mut self, function: Value, is_new: bool) -> Value {
        let shown = self.brief(function);
        if is_new {
            self.type_error(&format!("{shown} is not a constructor"))
        } else {
            self.type_error(&format!("{shown} is not a function"))
        }
    }

    /// A few words saying what a value is, for error messages.
    pub(crate) fn brief(&mut self, value: Value) -> String {
        if value.is_string() {
            return format!("\"{}\"", self.strings.items[value.index()].text.to_rust());
        }
        if value.is_object() {
            return if self.is_callable(value) {
                "function".to_owned()
            } else if matches!(self.objects[value.index()].kind, Kind::Array(_)) {
                "array".to_owned()
            } else {
                "object".to_owned()
            };
        }
        if value.is_symbol() {
            return "Symbol()".to_owned();
        }
        match self.to_string_lossy(value) {
            Ok(text) => text,
            Err(_) => "value".to_owned(),
        }
    }

    #[inline]
    pub(crate) fn is_callable(&self, value: Value) -> bool {
        value.is_object()
            && match &self.objects[value.index()].kind {
                Kind::Function { .. } | Kind::Native(_) | Kind::Bound(_) => true,
                Kind::Proxy(data) => data.callable,
                _ => false,
            }
    }

    pub(crate) fn is_constructor(&self, value: Value) -> bool {
        if !value.is_object() {
            return false;
        }
        match &self.objects[value.index()].kind {
            Kind::Function { proto, .. } => {
                self.protos[*proto as usize].flags & IS_CONSTRUCTOR != 0
            }
            Kind::Native(native) => native.is_constructor,
            Kind::Bound(bound) => self.is_constructor(bound.target),
            Kind::Proxy(data) => self.is_constructor(data.target),
            _ => false,
        }
    }

    /// The object for a `new` to fill in, with the prototype that `new_target` says.
    pub(crate) fn create_this(&mut self, new_target: Value) -> Run<Value> {
        let handle = new_target.handle();
        let cached = self.this_cache[handle as usize % self.this_cache.len()];
        if cached.function == handle && new_target.is_object() {
            let function = &self.objects[handle as usize];
            if cached.shape == function.shape
                && function.slots().get(cached.slot as usize) == Some(&Value::object(cached.proto))
            {
                return Ok(self.alloc_unrooted(cached.root, Kind::Plain, 0));
            }
        }
        self.create_this_slowly(new_target)
    }

    fn create_this_slowly(&mut self, new_target: Value) -> Run<Value> {
        let proto = self.get(new_target, text::PROTOTYPE)?;
        let handle = new_target.handle();
        let proto = if proto.is_object() {
            proto.handle()
        } else {
            self.realm.object_proto
        };
        let root = self.root_shape(proto);
        // Asking again can skip all that as long as it's the same property, holding the same thing.
        if new_target.is_object()
            && matches!(self.objects[handle as usize].kind, Kind::Function { .. })
        {
            let shape = self.objects[handle as usize].shape;
            if let Some((slot, flags)) = self.shapes.lookup(shape, text::PROTOTYPE) {
                let holds = self.objects[handle as usize].slots()[slot as usize];
                if flags & ACCESSOR == 0 && holds == Value::object(proto) {
                    let way = handle as usize % self.this_cache.len();
                    self.this_cache[way] = ThisCache {
                        function: handle,
                        shape,
                        slot,
                        proto,
                        root,
                    };
                }
            }
        }
        Ok(self.alloc_unrooted(root, Kind::Plain, 0))
    }

    /// The prototype a built-in constructor should give what it makes, which a subclass can change.
    pub(crate) fn proto_for(&mut self, args: &Args, default: u32) -> Run<u32> {
        if args.new_target.is_object() && args.new_target.handle() != args.callee {
            let proto = self.get(args.new_target, text::PROTOTYPE)?;
            if proto.is_object() {
                return Ok(proto.handle());
            }
        }
        Ok(default)
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    pub(crate) fn enter_function(
        &mut self,
        closure: u32,
        proto_id: u32,
        new_base: usize,
        argc: usize,
        ret_dst: u16,
        flags: u8,
        new_target: Value,
    ) -> Run<()> {
        if self.protos[proto_id as usize].start == NOT_COMPILED {
            self.compile_on_first_call(proto_id)?;
        }
        let proto = &self.protos[proto_id as usize];
        let registers = proto.registers as usize;
        let params = proto.params as usize;
        let start = proto.start;
        let cache_base = proto.cache_base;
        let keeps_args = proto.flags & KEEPS_ARGS != 0;
        let proto_flags = proto.flags;
        let proto_clear = proto.clear();
        let top = new_base + registers;
        if self.frames.len() >= MAX_FRAMES {
            return Err(self.range_error("Maximum call stack size exceeded"));
        }
        self.reserve_stack(top + 16)?;
        let mut flags = flags;
        if keeps_args {
            let mut args = self.spare_args.pop().unwrap_or_default();
            args.extend_from_slice(&self.stack[new_base + 1..new_base + 1 + argc]);
            self.kept_args.push(args);
            flags |= KEPT_ARGS;
        }
        let filled = new_base + 1 + argc.min(params);
        let clear = new_base + proto_clear;
        if filled < clear {
            self.stack[filled..clear].fill(Value::UNDEFINED);
        }
        let entry = if keeps_args || proto_flags & IS_CLASS != 0 {
            NONE
        } else {
            start
        };
        let upvalues = match &mut self.objects.for_the_engine(closure as usize).kind {
            Kind::Function {
                upvalues,
                entry: known,
                ..
            } => {
                // A closure made before its function was compiled learns where calls start, so the next call doesn't
                // come this way.
                *known = entry;
                upvalues.as_ptr()
            }
            _ => std::ptr::null(),
        };
        #[cfg(feature = "jsprof")]
        crate::jsprof::maybe_sample(self);
        self.frames.push(Frame {
            pc: start,
            base: new_base as u32,
            top: top as u32,
            proto: proto_id,
            closure,
            cache_base,
            ret_dst,
            flags,
            new_target,
            upvalues,
        });
        #[cfg(feature = "counts")]
        crate::counts::call(self, proto_id);
        Ok(())
    }

    /// Forgets what calls that have returned left in the registers above the top, before a collection that doesn't
    /// look at them frees what they name. A call only makes its parameters and variables undefined (`Proto::clear`),
    /// and the collector looks at all of a frame's registers, so without this one of its temporaries could still hold
    /// a handle from a call long gone, to something freed and maybe given back to the system. Everything in a register
    /// is then something that hasn't been freed: it was either looked at by the last collection or written since.
    pub(crate) fn wipe_above_top(&mut self) {
        let top = self.top.min(self.stack.len());
        // All zeros is the number 0, which names nothing.
        self.stack[top..].fill(Value(0));
    }

    /// `===` on two numbers, strings or bigints, kept out of the interpreter loop.
    #[inline(never)]
    fn strict_equals_slowly(&self, a: Value, b: Value) -> bool {
        self.strict_equals(a, b)
    }

    pub(crate) fn pop_frame(&mut self) {
        if let Some(frame) = self.frames.pop() {
            if frame.flags & KEPT_ARGS != 0 {
                self.finish_kept_args();
            }
            if frame.flags & RESUMED_HERE != 0 {
                // An error left the generator, which ends it.
                self.leave_resumed_generator(GeneratorState::Done);
            }
        }
    }

    /// The generator running in a frame with `RESUMED_HERE` has stopped, its frame has gone or is about to, and the one
    /// that was running before carries on.
    fn leave_resumed_generator(&mut self, state: GeneratorState) {
        if let Kind::Generator(data) = &mut self.objects[self.current_generator as usize].kind {
            data.state = state;
        }
        self.current_generator = self.resumed_here.pop().unwrap_or(NONE);
    }

    /// A frame's kept arguments are finished with. The list can be used again, if it isn't a huge one.
    fn finish_kept_args(&mut self) {
        if let Some(mut args) = self.kept_args.pop() {
            if self.spare_args.len() < 8 && args.capacity() <= 64 {
                args.clear();
                self.spare_args.push(args);
            }
        }
    }

    /// Runs until the frame count is back to `stop`.
    #[inline]
    pub(crate) fn run(&mut self, stop: usize) -> Run<Value> {
        #[cfg(feature = "owners")]
        let _owner = rsui_alloc::owners::scope(rsui_alloc::owners::JS);
        // Natives calling back into JavaScript stop before compiling something would run out.
        if self.stack_base.abs_diff(crate::stack_position()) > self.stack_limit / 10 * 7 {
            while self.frames.len() > stop {
                self.pop_frame();
            }
            return Err(self.range_error("Maximum call stack size exceeded"));
        }
        let saved_mark = self.run_mark;
        let saved_top = self.top;
        self.run_mark = self.nursery.len();
        self.nested_runs += 1;
        // With the JIT, in the function's compiled code if it has some.
        #[cfg(feature = "jit")]
        let result = self.run_frame(stop);
        #[cfg(not(feature = "jit"))]
        let result = self.run_loop(stop);
        self.nested_runs -= 1;
        self.run_mark = saved_mark;
        self.top = saved_top;
        result
    }

    /// Finds somewhere to carry on from after an error. If there's nowhere before `stop`, the frames are gone.
    pub(crate) fn unwind(&mut self, error: Value, stop: usize) -> bool {
        let is_generator_return = error == Value::GENERATOR_RETURN;
        while self.frames.len() > stop {
            let frame = self.frames.last_mut().expect("frames are above stop");
            let proto = &self.protos[frame.proto as usize];
            let at = frame.pc.wrapping_sub(proto.start).wrapping_sub(1);
            for handler in &proto.handlers {
                if handler.start <= at
                    && at < handler.end
                    && !(is_generator_return && handler.is_catch)
                {
                    frame.pc = proto.start + handler.target;
                    let reg = frame.base as usize + handler.reg as usize;
                    self.stack[reg] = error;
                    return true;
                }
            }
            self.pop_frame();
        }
        false
    }

    // The interpreter reads instructions, registers, caches and object headers without checking they're in bounds, and
    // keeps pointers to the lists they're in, which never move. The compiler only makes instructions that name
    // registers below the function's count and jump to places in the function, which ends in a return. Every frame has
    // made sure the stack is big enough for all its registers. A value only ever holds the handle of a live object or
    // cell, as the collector keeps whatever a register holds, and a cache entry that matches an object's shape (a
    // place's first, or one of its others) names a slot that shape has, as does a remembered lookup that matches it
    // (`Vm::remembered_value`, `Vm::remembered_store`). A place's instruction is only ever made polymorphic once it has
    // its other entries (`Vm::learn`).
    //
    // How fast this loop is depends on how it's compiled as much as on what it does, so before changing it:
    //
    // - Each handler ends with its own jump to the next one, because .cargo/config.toml lets LLVM copy that jump
    //   (`-tail-dup-pred-size`, `-tail-dup-succ-size`). Without them the whole loop shares one jump, how well the CPU
    //   predicts it depends on where every handler happens to land, and unrelated edits anywhere in the crate made
    //   benches 10-50% slower.
    // - Only `ip`, `regs`, `frame` and `ics` are kept from one instruction to the next; the lists everything else is
    //   in are read from `self` where a handler needs them (`objects!()`, ...), which is a load. LLVM gives what's kept
    //   a register across the whole loop, and when there's more than fits, it spills some of it inside whichever
    //   handlers are short of registers and reloads it on the way out of them, so that adding to one handler made
    //   others slower: polymorphic property caches added to GetProp cost 4-8% on code with one shape when the lists
    //   were kept too.
    // - Keep what a handler does when things are as usual here, and everything else in a function of its own marked
    //   `#[inline(never)]` (`get_cached`, `set_by_value`, `call_at`, ...), passing what it needs as arguments. Code
    //   inlined here, even if it never runs, takes registers from the rest of the loop. Something only some places
    //   need can be an instruction of its own that the slow way rewrites those places to (GetPropPoly, see
    //   `Vm::learn`), so the usual one carries none of it.
    // - Something new that every handler would pay for (a check per instruction, a counter) costs about 1-3% per
    //   load or test. Put it where it's needed: calls, loops' back jumps (`Loop*`) or `safe_point!`.
    // - Measure before and after, and for a change that should only be
    //   faster in one place, check the benches it can't affect didn't move.
    #[cfg(not(feature = "jit"))]
    pub(crate) fn run_loop(&mut self, stop: usize) -> Run<Value> {
        self.run_loop_body(stop, false)
    }

    /// With the JIT built in, there are two of the loop: one with the JIT's hooks (where calls and loops count
    /// towards compiling, and go into compiled code), and, when `RSUI_JIT=0` says there's to be no compiling, one
    /// without, which is the same as the loop built without the JIT, so that interpreting pays nothing for it.
    #[cfg(feature = "jit")]
    #[inline(always)]
    pub(crate) fn run_loop(&mut self, stop: usize) -> Run<Value> {
        if self.jit.mode == crate::jit::Mode::Off {
            self.run_loop_plain(stop)
        } else {
            self.run_loop_jit(stop)
        }
    }

    #[cfg(feature = "jit")]
    #[inline(never)]
    fn run_loop_plain(&mut self, stop: usize) -> Run<Value> {
        self.run_loop_body(stop, false)
    }

    #[cfg(feature = "jit")]
    #[inline(never)]
    fn run_loop_jit(&mut self, stop: usize) -> Run<Value> {
        self.run_loop_body(stop, true)
    }

    /// The loop itself (see above), with the JIT's hooks in it if `jit` says so, which is known where it's called.
    #[allow(clippy::cognitive_complexity, unsafe_code, unused_unsafe, unused_variables)]
    #[inline(always)]
    fn run_loop_body(&mut self, stop: usize, jit: bool) -> Run<Value> {
        // Only these are kept from one instruction to the next: where the next instruction is, the registers, the
        // frame and its caches. The lists everything else is in are read from `self` when a handler needs them,
        // which costs a load, where keeping each in a register of its own left too few for the handlers, and LLVM
        // then kept `ip`, `regs` and the rest on the stack in more and more of them as handlers grew. The function's
        // captured variables are read from the frame for the same reason.
        let mut frame: *mut Frame;
        let mut ip: *const Op;
        let mut regs: *mut Value;
        let mut ics: *mut Cache;
        macro_rules! code {
            () => {
                self.code.as_ptr()
            };
        }
        macro_rules! stack {
            () => {
                self.stack.as_mut_ptr()
            };
        }
        macro_rules! objects {
            () => {
                self.objects.as_mut_ptr()
            };
        }
        macro_rules! cells {
            () => {
                self.cells.as_mut_ptr()
            };
        }
        macro_rules! consts {
            () => {
                self.consts.as_ptr()
            };
        }
        macro_rules! protos {
            () => {
                self.protos.as_ptr()
            };
        }
        macro_rules! ups {
            () => {
                unsafe { (*frame).upvalues }
            };
        }

        macro_rules! load_frame {
            () => {{
                let current = unsafe { &*frame };
                ip = unsafe { code!().add(current.pc as usize) };
                regs = unsafe { stack!().add(current.base as usize) };
                ics = unsafe { self.caches.as_mut_ptr().add(current.cache_base as usize) };
            }};
        }
        macro_rules! reload {
            () => {{
                frame = unsafe { self.frames.as_mut_ptr().add(self.frames.len() - 1) };
                load_frame!();
            }};
        }
        macro_rules! save_pc {
            () => {
                unsafe { (*frame).pc = ip.offset_from(code!()) as u32 };
            };
        }
        macro_rules! base {
            () => {
                unsafe { (*frame).base as usize }
            };
        }
        macro_rules! cache_index {
            ($cache:expr) => {
                unsafe { (*frame).cache_base as usize + $cache as usize }
            };
        }
        reload!();
        // (With the JIT, a frame Rust made has already been counted, and gone into compiled code if it has some,
        // in `Vm::run_frame`, which is the only way in here for a frame at its function's start.)

        loop {
            let thrown: Value = 'run: loop {
                macro_rules! reg {
                    ($index:expr) => {
                        *unsafe { &mut *regs.add($index as usize) }
                    };
                }
                macro_rules! throw {
                    ($error:expr) => {{
                        break 'run $error;
                    }};
                }
                macro_rules! attempt {
                    ($result:expr) => {{
                        save_pc!();
                        self.top = unsafe { (*frame).top as usize };
                        match $result {
                            Ok(value) => value,
                            Err(error) => throw!(error),
                        }
                    }};
                }
                macro_rules! jump {
                    ($offset:expr) => {
                        ip = unsafe { ip.offset($offset as isize) }
                    };
                }
                // A new frame on top of this one, for a function that's nothing unusual: one that doesn't keep its
                // arguments (`KEEPS_ARGS`), which the caller has looked at already, as it's in the same byte as the
                // other flags it needs.
                macro_rules! enter {
                    ($closure:expr, $proto:expr, $start:expr, $ups:expr, $f:expr, $argc:expr, $dst:expr, $flags:expr, $target:expr) => {{
                        let proto = unsafe { &*protos!().add($proto as usize) };
                        let start: u32 = $start;
                        let new_base = base!() + $f as usize + 1;
                        let registers = proto.registers as usize;
                        if new_base + registers + 16 > self.stack.len()
                            || self.frames.len() >= self.frames.capacity()
                        {
                            enter_slowly!($closure, $proto, $f, $argc, $dst, $flags, $target);
                        }
                        // Only the parameters with no argument and the function's own variables: the rest are written
                        // before they're read. That's done last, once the frame's written, so what the frame needs
                        // isn't kept in registers through it, which made LLVM keep some of it on the stack.
                        let mut at = 1 + ($argc as usize).min(proto.params as usize);
                        let clear = proto.clear();
                        save_pc!();
                        unsafe {
                            frame = frame.add(1);
                            frame.write(Frame {
                                pc: start,
                                base: new_base as u32,
                                top: (new_base + registers) as u32,
                                proto: $proto,
                                closure: $closure,
                                cache_base: proto.cache_base,
                                ret_dst: $dst,
                                flags: $flags,
                                new_target: $target,
                                upvalues: $ups,
                            });
                            let depth = self.frames.len() + 1;
                            self.frames.set_len(depth);
                            ip = code!().add(start as usize);
                            regs = regs.add($f as usize + 1);
                            ics = self.caches.as_mut_ptr().add(proto.cache_base as usize);
                            // One at a time (a volatile write can't be merged), as there are hardly ever more than a
                            // few: LLVM otherwise unrolls this into a loop of wide writes with more code around it
                            // than the writes it saves, and takes registers for it.
                            while at < clear {
                                regs.add(at).write_volatile(Value::UNDEFINED);
                                at += 1;
                            }
                        }
                        #[cfg(feature = "counts")]
                        crate::counts::call(self, $proto);
                        // Counts the call for the JIT, and runs the function's code if it has some.
                        #[cfg(feature = "jit")]
                        if jit {
                            jit_count!($proto, jit_next!(self.jit_enter(stop)));
                        }
                        continue;
                    }};
                }
                // What to do after the JIT's code has run, or decided not to.
                #[cfg(feature = "jit")]
                macro_rules! jit_next {
                    ($next:expr) => {
                        match $next {
                            crate::jit::Next::Interpret => reload!(),
                            crate::jit::Next::Return(value) => return Ok(value),
                            crate::jit::Next::Throw(error) => {
                                // Where the frame on top is, for `save_pc!` on the way to `unwind`.
                                frame = unsafe { self.frames.as_mut_ptr().add(self.frames.len() - 1) };
                                ip = unsafe { code!().add((*frame).pc as usize) };
                                throw!(error);
                            }
                        }
                    };
                }
                // After a jump back to the top of a loop: counts it for the JIT, and carries on in the function's
                // code if it has some.
                macro_rules! jit_loop {
                    () => {
                        #[cfg(feature = "counts")]
                        crate::counts::turn(self, unsafe { (*frame).proto });
                        #[cfg(feature = "jit")]
                        if jit {
                            jit_count!(unsafe { (*frame).proto }, {
                                save_pc!();
                                jit_next!(self.jit_loop(stop));
                            });
                        }
                    };
                }
                // Counts a call or a loop's iteration of function `$proto` towards compiling it (see `Jit::slots`),
                // here while it isn't due yet, which is what most functions are most of the time; and does `$more`
                // when it's due, or has code worth going into. Most functions are never called often enough to be
                // compiled, and calling out for each call made the interpreter 3-5% slower on an app's code.
                #[cfg(feature = "jit")]
                macro_rules! jit_count {
                    ($proto:expr, $more:expr) => {{
                        let place = unsafe { self.jit.slots.as_mut_ptr().add($proto as usize) };
                        let slot = unsafe { *place };
                        if slot & 1 != 0 {
                            if slot.wrapping_sub(3) < crate::jit::COUNTING {
                                unsafe { *place = slot - 2 };
                            } else {
                                $more;
                            }
                        }
                    }};
                }
                // The same for any function, by way of a function of its own: one that keeps its arguments, or when the
                // stack or the list of frames has to grow.
                macro_rules! enter_slowly {
                    ($closure:expr, $proto:expr, $f:expr, $argc:expr, $dst:expr, $flags:expr, $target:expr) => {{
                        save_pc!();
                        let entered = self.enter_function(
                            $closure,
                            $proto,
                            base!() + $f as usize + 1,
                            $argc as usize,
                            $dst,
                            $flags,
                            $target,
                        );
                        if let Err(error) = entered {
                            throw!(error);
                        }
                        reload!();
                        continue;
                    }};
                }
                macro_rules! arithmetic {
            ($dst:expr, $x:expr, $y:expr, $checked:ident, $float:tt, $slow:ident) => {{
                let (x, y): (Value, Value) = ($x, $y);
                reg!($dst) = if Value::both_ints(x, y) {
                    match x.as_int().$checked(y.as_int()) {
                        Some(value) => Value::int(value),
                        None => Value::computed(f64::from(x.as_int()) $float f64::from(y.as_int())),
                    }
                } else if Value::both_doubles(x, y) {
                    // What float code has most, with nothing to work out about either.
                    Value::computed(x.as_double() $float y.as_double())
                } else if x.is_number() && y.is_number() {
                    Value::computed(x.as_number() $float y.as_number())
                } else {
                    attempt!(self.$slow(x, y))
                };
            }};
        }
                macro_rules! multiply {
                    ($dst:expr, $x:expr, $y:expr) => {{
                        let (x, y): (Value, Value) = ($x, $y);
                        reg!($dst) = if Value::both_ints(x, y) {
                            match x.as_int().checked_mul(y.as_int()) {
                                Some(value) if value != 0 => Value::int(value),
                                // Zero times something negative is negative zero.
                                _ => Value::computed(f64::from(x.as_int()) * f64::from(y.as_int())),
                            }
                        } else if Value::both_doubles(x, y) {
                            Value::computed(x.as_double() * y.as_double())
                        } else if x.is_number() && y.is_number() {
                            Value::computed(x.as_number() * y.as_number())
                        } else {
                            attempt!(self.mul_slow(x, y))
                        };
                    }};
                }
                macro_rules! divide {
                    ($dst:expr, $x:expr, $y:expr) => {{
                        let (x, y): (Value, Value) = ($x, $y);
                        reg!($dst) = if x.is_number() && y.is_number() {
                            Value::number(x.as_number() / y.as_number())
                        } else {
                            attempt!(self.div_slow(x, y))
                        };
                    }};
                }
                macro_rules! remainder {
                    ($dst:expr, $x:expr, $y:expr) => {{
                        let (x, y): (Value, Value) = ($x, $y);
                        reg!($dst) = if Value::both_ints(x, y) && x.as_int() >= 0 && y.as_int() > 0
                        {
                            Value::int(x.as_int() % y.as_int())
                        } else if x.is_number() && y.is_number() {
                            Value::number(x.as_number() % y.as_number())
                        } else {
                            attempt!(self.mod_slow(x, y))
                        };
                    }};
                }
                macro_rules! bitwise {
                    ($dst:expr, $x:expr, $y:expr, $op:ident) => {{
                        let (x, y): (Value, Value) = ($x, $y);
                        reg!($dst) = if Value::both_ints(x, y) {
                            bit_op!($op, x.as_int(), y.as_int())
                        } else if let (Some(x), Some(y)) = (quick_int32(x), quick_int32(y)) {
                            bit_op!($op, x, y)
                        } else {
                            attempt!(self.bitwise_slow(big_op!($op), x, y))
                        };
                    }};
                }
                macro_rules! big_op {
                    (and) => {
                        crate::builtins::BigOp::And
                    };
                    (or) => {
                        crate::builtins::BigOp::Or
                    };
                    (xor) => {
                        crate::builtins::BigOp::Xor
                    };
                    (shl) => {
                        crate::builtins::BigOp::Shl
                    };
                    (shr) => {
                        crate::builtins::BigOp::Shr
                    };
                    (ushr) => {
                        crate::builtins::BigOp::UShr
                    };
                }
                macro_rules! bit_op {
                    (and, $x:expr, $y:expr) => {
                        Value::int($x & $y)
                    };
                    (or, $x:expr, $y:expr) => {
                        Value::int($x | $y)
                    };
                    (xor, $x:expr, $y:expr) => {
                        Value::int($x ^ $y)
                    };
                    (shl, $x:expr, $y:expr) => {
                        Value::int($x.wrapping_shl($y as u32 & 31))
                    };
                    (shr, $x:expr, $y:expr) => {
                        Value::int($x.wrapping_shr($y as u32 & 31))
                    };
                    (ushr, $x:expr, $y:expr) => {
                        Value::int_or_double(($x as u32).wrapping_shr($y as u32 & 31) as usize)
                    };
                }
                macro_rules! compare {
            ($a:expr, $b:expr, $op:tt, $slow:expr) => {{
                let (x, y) = (reg!($a), reg!($b));
                if Value::both_ints(x, y) {
                    x.as_int() $op y.as_int()
                } else if x.is_number() && y.is_number() {
                    x.as_number() $op y.as_number()
                } else {
                    attempt!($slow(self, x, y))
                }
            }};
        }
                macro_rules! lt { ($a:expr, $b:expr) => { compare!($a, $b, <, Vm::less_than) } }
                macro_rules! le { ($a:expr, $b:expr) => { compare!($a, $b, <=, Vm::less_equal) } }
                macro_rules! gt { ($a:expr, $b:expr) => { compare!($a, $b, >, Vm::greater_than) } }
                macro_rules! ge { ($a:expr, $b:expr) => { compare!($a, $b, >=, Vm::greater_equal) } }
                macro_rules! compare_int {
            ($a:expr, $value:expr, $op:tt, $slow:expr) => {{
                let x = reg!($a);
                let value = i32::from($value);
                if x.is_int() {
                    x.as_int() $op value
                } else if x.is_double() {
                    x.as_double() $op f64::from(value)
                } else {
                    attempt!($slow(self, x, Value::int(value)))
                }
            }};
        }
                macro_rules! lt_int { ($a:expr, $v:expr) => { compare_int!($a, $v, <, Vm::less_than) } }
                macro_rules! le_int { ($a:expr, $v:expr) => { compare_int!($a, $v, <=, Vm::less_equal) } }
                macro_rules! gt_int { ($a:expr, $v:expr) => { compare_int!($a, $v, >, Vm::greater_than) } }
                macro_rules! ge_int { ($a:expr, $v:expr) => { compare_int!($a, $v, >=, Vm::greater_equal) } }
                // `===`, with the answer here for the same thing twice, two ints, or anything that's only equal to
                // itself (objects, `null`, `undefined`, booleans, symbols), and `strict_equals` for the rest.
                macro_rules! strict_equal {
                    ($a:expr, $b:expr) => {{
                        let (x, y) = (reg!($a), reg!($b));
                        if x == y {
                            // NaN is the one thing that isn't equal to itself.
                            !x.is_double() || !x.as_double().is_nan()
                        } else if Value::both_ints(x, y) || !x.has_other_forms() || !y.has_other_forms() {
                            false
                        } else {
                            self.strict_equals_slowly(x, y)
                        }
                    }};
                }
                macro_rules! loose_equal {
                    ($a:expr, $b:expr) => {{
                        let (x, y) = (reg!($a), reg!($b));
                        if Value::both_ints(x, y) {
                            x == y
                        } else {
                            attempt!(self.loose_equals(x, y))
                        }
                    }};
                }
                macro_rules! call_function {
                    ($dst:expr, $f:expr, $argc:expr) => {{
                        safe_point!();
                        let callee = reg!($f);
                        if callee.is_object() {
                            match unsafe { &(*objects!().add(callee.index())).kind } {
                                Kind::Function {
                                    proto,
                                    upvalues,
                                    entry,
                                    ..
                                } => {
                                    let proto = *proto;
                                    // One test for the usual case, on what the closure says, so where the code
                                    // starts doesn't wait on reading the proto.
                                    if *entry != NONE {
                                        enter!(
                                            callee.handle(),
                                            proto,
                                            *entry,
                                            upvalues.as_ptr(),
                                            $f,
                                            $argc,
                                            $dst,
                                            0,
                                            Value::UNDEFINED
                                        );
                                    }
                                    // A class can't be called, which `call_at` says.
                                    let flags = unsafe { (*protos!().add(proto as usize)).flags };
                                    if flags & IS_CLASS == 0 {
                                        enter_slowly!(
                                            callee.handle(),
                                            proto,
                                            $f,
                                            $argc,
                                            $dst,
                                            0,
                                            Value::UNDEFINED
                                        );
                                    }
                                }
                                // `call` and `apply` go to `call_indirect`, which calls what they would here.
                                Kind::Native(native) if native.forwards == 0 => {
                                    let func = native.func;
                                    if self.keeping.working != 0 && !native.pure {
                                        self.keeping.spoiled = true;
                                    }
                                    #[cfg(feature = "counts")]
                                    crate::counts::native(self, func as usize, native.name);
                                    let args = Args {
                                        this: reg!($f + 1),
                                        start: base!() + $f as usize + 2,
                                        count: $argc as usize,
                                        new_target: Value::UNDEFINED,
                                        callee: callee.handle(),
                                    };
                                    reg!($dst) = attempt!(func(self, args));
                                    continue;
                                }
                                _ => {}
                            }
                        }
                        reg!($dst) = attempt!(self.call_indirect(
                            base!() + $f as usize,
                            $argc as usize,
                            $dst
                        ));
                    }};
                }

                // Reads a property for GetProp, and for CallMethod before the call.
                // `$here` says whether the read's answer is all the instruction wants (GetProp), so that a proxy's
                // trap can be called in place, answering to `$dst` (see `Vm::get_cached_here`).
                macro_rules! get_prop {
                    ($dst:expr, $target:expr, $cache:expr, $here:expr) => {{
                        'read: {
                            let target = $target;
                            let entry = unsafe { &*ics.add($cache as usize) };
                            if target.is_object() {
                                let found = unsafe { &*objects!().add(target.index()) };
                                if found.shape == entry.shape {
                                    if entry.other == NONE {
                                        reg!($dst) = unsafe { found.read_slot(entry.slot as usize) };
                                        break 'read;
                                    }
                                    if entry.epoch == self.epoch {
                                        reg!($dst) = unsafe {
                                            (*objects!().add(entry.other as usize))
                                                .read_slot(entry.slot as usize)
                                        };
                                        break 'read;
                                    }
                                }
                            }
                            // Another shape: `get_cached` remembers it, and makes this place GetPropPoly.
                            reg!($dst) = attempt!(if $here {
                                self.get_cached_here(target, cache_index!($cache), $dst)
                            } else {
                                self.get_cached(target, cache_index!($cache))
                            });
                        }
                    }};
                }
                // Reads a property for GetPropPoly and CallMethodPoly: the place's first entry, then its others, and
                // once it has seen more shapes than it has entries for, what reads remember. A hit changes nothing, as
                // `get_cached` is what learns (`Vm::learn`). (`length` is GetLength's, so never comes here.)
                macro_rules! get_prop_poly {
                    ($dst:expr, $target:expr, $cache:expr, $here:expr) => {{
                        'read: {
                            let target = $target;
                            'slow: {
                                if !target.is_object() {
                                    break 'slow;
                                }
                                let found = unsafe { &*objects!().add(target.index()) };
                                let first = unsafe { &*ics.add($cache as usize) };
                                // Only a place with more entries is ever made polymorphic.
                                debug_assert!(first.more != NONE);
                                let known = 'find: {
                                    if first.shape == found.shape {
                                        break 'find first;
                                    }
                                    if first.more & MEGAMORPHIC == 0 {
                                        let more = unsafe { (first as *const Cache).add(first.more as usize) };
                                        let mut at = 0;
                                        while at < POLY {
                                            let next = unsafe { &*more.add(at) };
                                            if next.shape == found.shape {
                                                break 'find next;
                                            }
                                            at += 1;
                                        }
                                        break 'slow;
                                    }
                                    // Too many shapes to keep here: where the last search on this shape found it,
                                    // remembered here too, as `get_cached` would, so a run of objects of one shape
                                    // finds it in the first entry.
                                    if found.flags & LAZY_FUNCTION == 0 {
                                        if let Some((holder, slot)) =
                                            self.remembered_value(found.shape, first.atom)
                                        {
                                            let entry = unsafe { &mut *ics.add($cache as usize) };
                                            entry.shape = found.shape;
                                            entry.other = holder;
                                            entry.slot = slot;
                                            entry.epoch = self.epoch;
                                            let holder = if holder == NONE {
                                                found
                                            } else {
                                                unsafe { &*objects!().add(holder as usize) }
                                            };
                                            reg!($dst) = unsafe { holder.read_slot(slot as usize) };
                                            break 'read;
                                        }
                                    }
                                    break 'slow;
                                };
                                if known.other == NONE {
                                    reg!($dst) = unsafe { found.read_slot(known.slot as usize) };
                                    break 'read;
                                }
                                if known.epoch == self.epoch {
                                    reg!($dst) = unsafe {
                                        (*objects!().add(known.other as usize)).read_slot(known.slot as usize)
                                    };
                                    break 'read;
                                }
                            }
                            reg!($dst) = attempt!(if $here {
                                self.get_cached_here(target, cache_index!($cache), $dst)
                            } else {
                                self.get_cached(target, cache_index!($cache))
                            });
                        }
                    }};
                }

                macro_rules! finish_return {
                    ($value:expr) => {{
                        let value: Value = $value;
                        let value = if unsafe { (*frame).flags } == 0 {
                            value
                        } else {
                            attempt!(self.finish_special_frame(value))
                        };
                        let ret_dst = unsafe { (*frame).ret_dst };
                        let depth = self.frames.len() - 1;
                        unsafe { self.frames.set_len(depth) };
                        if depth == stop {
                            return Ok(value);
                        }
                        frame = unsafe { frame.sub(1) };
                        load_frame!();
                        unsafe { *regs.add(ret_dst as usize) = value };
                    }};
                }
                macro_rules! safe_point {
                    () => {
                        if self.debt > self.threshold {
                            save_pc!();
                            self.top = unsafe { (*frame).top as usize };
                            self.collect_garbage();
                        }
                    };
                }

                // The handlers read their own fields from here. Reading them all up front holds on to registers the
                // handlers could use, and made calls and arithmetic about 20% slower.
                let op = unsafe { &*ip };
                #[cfg(feature = "counts")]
                crate::counts::op(op, unsafe { (*frame).proto }, unsafe { ip.offset_from(code!()) } as u32);
                ip = unsafe { ip.add(1) };
                match *op {
                    Op::LoadConst { dst, index } => {
                        reg!(dst) = unsafe { *consts!().add(index as usize) }
                    }
                    Op::LoadInt { dst, value } => reg!(dst) = Value::int(value),
                    Op::LoadUndefined { dst } => reg!(dst) = Value::UNDEFINED,
                    Op::LoadNull { dst } => reg!(dst) = Value::NULL,
                    Op::LoadTrue { dst } => reg!(dst) = Value::TRUE,
                    Op::LoadFalse { dst } => reg!(dst) = Value::FALSE,
                    Op::LoadEmpty { dst } => reg!(dst) = Value::EMPTY,
                    Op::Move { dst, src } => reg!(dst) = reg!(src),

                    Op::Add { dst, a, b } => {
                        arithmetic!(dst, reg!(a), reg!(b), checked_add, +, add_slow)
                    }
                    Op::AddInt { dst, a, value } => {
                        let x = reg!(a);
                        reg!(dst) = if x.is_int() {
                            match x.as_int().checked_add(i32::from(value)) {
                                Some(sum) => Value::int(sum),
                                None => Value::computed(f64::from(x.as_int()) + f64::from(value)),
                            }
                        } else if x.is_double() {
                            Value::computed(x.as_double() + f64::from(value))
                        } else {
                            attempt!(self.add_slow(x, Value::int(i32::from(value))))
                        };
                    }
                    Op::AddInt32 { dst, a, b } => {
                        let (x, y) = (reg!(a), reg!(b));
                        reg!(dst) = if Value::both_ints(x, y) {
                            Value::int(x.as_int().wrapping_add(y.as_int()))
                        } else {
                            attempt!(self.add_int32_slow(x, y))
                        };
                    }
                    Op::SubInt { dst, a, value } => {
                        let x = reg!(a);
                        reg!(dst) = if x.is_int() {
                            match x.as_int().checked_sub(i32::from(value)) {
                                Some(sum) => Value::int(sum),
                                None => Value::computed(f64::from(x.as_int()) - f64::from(value)),
                            }
                        } else if x.is_double() {
                            Value::computed(x.as_double() - f64::from(value))
                        } else {
                            attempt!(self.sub_slow(x, Value::int(i32::from(value))))
                        };
                    }
                    Op::MulInt { dst, a, value } => {
                        multiply!(dst, reg!(a), Value::int(i32::from(value)))
                    }
                    Op::ModInt { dst, a, value } => {
                        remainder!(dst, reg!(a), Value::int(i32::from(value)))
                    }
                    Op::DivInt { dst, a, value } => {
                        let (x, by) = (reg!(a), i32::from(value));
                        // A power of two above zero: what it divides exactly stays an int, of the same sign (0 too).
                        reg!(dst) = if x.is_int() && x.as_int() & (by - 1) == 0 {
                            Value::int(x.as_int() >> by.trailing_zeros())
                        } else if x.is_number() {
                            Value::number(x.as_number() / f64::from(by))
                        } else {
                            attempt!(self.div_slow(x, Value::int(by)))
                        };
                    }
                    Op::BitAndInt { dst, a, value } => {
                        bitwise!(dst, reg!(a), Value::int(i32::from(value)), and)
                    }
                    Op::BitOrInt { dst, a, value } => {
                        bitwise!(dst, reg!(a), Value::int(i32::from(value)), or)
                    }
                    Op::ShlInt { dst, a, value } => {
                        bitwise!(dst, reg!(a), Value::int(i32::from(value)), shl)
                    }
                    Op::ShrInt { dst, a, value } => {
                        bitwise!(dst, reg!(a), Value::int(i32::from(value)), shr)
                    }
                    Op::UShrInt { dst, a, value } => {
                        bitwise!(dst, reg!(a), Value::int(i32::from(value)), ushr)
                    }
                    Op::AddK { dst, a, k } => {
                        arithmetic!(dst, reg!(a), unsafe { *consts!().add(k as usize) }, checked_add, +, add_slow)
                    }
                    Op::SubK { dst, a, k } => {
                        arithmetic!(dst, reg!(a), unsafe { *consts!().add(k as usize) }, checked_sub, -, sub_slow)
                    }
                    Op::MulK { dst, a, k } => {
                        multiply!(dst, reg!(a), unsafe { *consts!().add(k as usize) })
                    }
                    Op::DivK { dst, a, k } => {
                        divide!(dst, reg!(a), unsafe { *consts!().add(k as usize) })
                    }
                    Op::ModK { dst, a, k } => {
                        remainder!(dst, reg!(a), unsafe { *consts!().add(k as usize) })
                    }
                    Op::Sub { dst, a, b } => {
                        arithmetic!(dst, reg!(a), reg!(b), checked_sub, -, sub_slow)
                    }
                    Op::Mul { dst, a, b } => multiply!(dst, reg!(a), reg!(b)),
                    Op::Div { dst, a, b } => divide!(dst, reg!(a), reg!(b)),
                    Op::Mod { dst, a, b } => remainder!(dst, reg!(a), reg!(b)),
                    Op::Pow { dst, a, b } => {
                        let (x, y) = (reg!(a), reg!(b));
                        reg!(dst) = attempt!(self.pow_slow(x, y));
                    }
                    Op::BitAnd { dst, a, b } => bitwise!(dst, reg!(a), reg!(b), and),
                    Op::BitOr { dst, a, b } => bitwise!(dst, reg!(a), reg!(b), or),
                    Op::BitXor { dst, a, b } => bitwise!(dst, reg!(a), reg!(b), xor),
                    Op::Shl { dst, a, b } => bitwise!(dst, reg!(a), reg!(b), shl),
                    Op::Shr { dst, a, b } => bitwise!(dst, reg!(a), reg!(b), shr),
                    Op::UShr { dst, a, b } => bitwise!(dst, reg!(a), reg!(b), ushr),
                    Op::Lt { dst, a, b } => reg!(dst) = Value::bool(lt!(a, b)),
                    Op::Le { dst, a, b } => reg!(dst) = Value::bool(le!(a, b)),
                    Op::Gt { dst, a, b } => reg!(dst) = Value::bool(gt!(a, b)),
                    Op::Ge { dst, a, b } => reg!(dst) = Value::bool(ge!(a, b)),
                    Op::Eq { dst, a, b } => {
                        let equal = loose_equal!(a, b);
                        reg!(dst) = Value::bool(equal);
                    }
                    Op::Ne { dst, a, b } => {
                        let equal = loose_equal!(a, b);
                        reg!(dst) = Value::bool(!equal);
                    }
                    Op::StrictEq { dst, a, b } => {
                        let equal = strict_equal!(a, b);
                        reg!(dst) = Value::bool(equal);
                    }
                    Op::StrictNe { dst, a, b } => {
                        let equal = strict_equal!(a, b);
                        reg!(dst) = Value::bool(!equal);
                    }
                    Op::In { dst, a, b } => {
                        let (key, object) = (reg!(a), reg!(b));
                        let found = attempt!(self.has_in(key, object));
                        reg!(dst) = Value::bool(found);
                    }
                    Op::InstanceOf { dst, a, b } => {
                        let (value, target) = (reg!(a), reg!(b));
                        let found = attempt!(self.instance_of(value, target));
                        reg!(dst) = Value::bool(found);
                    }

                    Op::Neg { dst, src } => {
                        let x = reg!(src);
                        reg!(dst) = if x.is_int() && x.as_int() != 0 && x.as_int() != i32::MIN {
                            Value::int(-x.as_int())
                        } else {
                            attempt!(self.negate_slow(x))
                        };
                    }
                    Op::ToNumber { dst, src } => {
                        let x = reg!(src);
                        reg!(dst) = if x.is_number() {
                            x
                        } else {
                            let number = attempt!(self.to_number(x));
                            Value::number(number)
                        };
                    }
                    Op::ToNumeric { dst, src } => {
                        let x = reg!(src);
                        reg!(dst) = if x.is_number() {
                            x
                        } else {
                            attempt!(self.to_numeric(x))
                        };
                    }
                    Op::Not { dst, src } => {
                        let x = reg!(src);
                        let truth = self.to_boolean(x);
                        reg!(dst) = Value::bool(!truth);
                    }
                    Op::BitNot { dst, src } => {
                        let x = reg!(src);
                        if let Some(value) = quick_int32(x) {
                            reg!(dst) = Value::int(!value);
                            continue;
                        }
                        let numeric = attempt!(self.to_numeric(x));
                        reg!(dst) = if numeric.is_bigint() {
                            self.bigint_not(numeric)
                        } else {
                            let value = attempt!(self.to_int32(numeric));
                            Value::int(!value)
                        };
                    }
                    Op::TypeOf { dst, src } => {
                        let x = reg!(src);
                        let kind = self.type_of(x);
                        reg!(dst) = kind;
                    }
                    Op::Inc { dst, src } => {
                        let x = reg!(src);
                        reg!(dst) = if x.is_int() && x.as_int() != i32::MAX {
                            Value::int(x.as_int() + 1)
                        } else {
                            attempt!(self.step_slow(x, 1))
                        };
                    }
                    Op::Dec { dst, src } => {
                        let x = reg!(src);
                        reg!(dst) = if x.is_int() && x.as_int() != i32::MIN {
                            Value::int(x.as_int() - 1)
                        } else {
                            attempt!(self.step_slow(x, -1))
                        };
                    }
                    Op::ToKey { dst, src } => {
                        let key = reg!(src);
                        if !key.is_string() && !key.is_symbol() && !key.is_int() {
                            let atom = attempt!(self.to_atom(key));
                            reg!(dst) = self.atom_value(atom);
                        } else {
                            reg!(dst) = key;
                        }
                    }
                    Op::Concat { dst, start, count } => {
                        let joined = attempt!(
                            self.concat_registers(base!() + start as usize, count as usize)
                        );
                        reg!(dst) = joined;
                    }

                    Op::Jump { offset } => jump!(offset),
                    Op::Loop { offset } => {
                        safe_point!();
                        jump!(offset);
                        jit_loop!();
                    }
                    Op::JumpIfTrue { condition, offset } => {
                        let value = reg!(condition);
                        if value == Value::TRUE || (value != Value::FALSE && self.to_boolean(value))
                        {
                            jump!(offset);
                        }
                    }
                    Op::JumpIfFalse { condition, offset } => {
                        let value = reg!(condition);
                        if value == Value::FALSE
                            || (value != Value::TRUE && !self.to_boolean(value))
                        {
                            jump!(offset);
                        }
                    }
                    Op::JumpIfNullish { condition, offset } => {
                        if reg!(condition).is_nullish() {
                            jump!(offset);
                        }
                    }
                    Op::JumpIfNotNullish { condition, offset } => {
                        if !reg!(condition).is_nullish() {
                            jump!(offset);
                        }
                    }
                    Op::JumpIfNotUndefined { condition, offset } => {
                        if !reg!(condition).is_undefined() {
                            jump!(offset);
                        }
                    }
                    Op::JumpIfUndefined { condition, offset } => {
                        if reg!(condition).is_undefined() {
                            jump!(offset);
                        }
                    }
                    Op::JumpIfNull { condition, offset } => {
                        if reg!(condition).is_null() {
                            jump!(offset);
                        }
                    }
                    Op::JumpIfNotNull { condition, offset } => {
                        if !reg!(condition).is_null() {
                            jump!(offset);
                        }
                    }
                    Op::JumpLt { a, b, offset } => {
                        if lt!(a, b) {
                            jump!(offset);
                        }
                    }
                    Op::JumpLe { a, b, offset } => {
                        if le!(a, b) {
                            jump!(offset);
                        }
                    }
                    Op::JumpGt { a, b, offset } => {
                        if gt!(a, b) {
                            jump!(offset);
                        }
                    }
                    Op::JumpGe { a, b, offset } => {
                        if ge!(a, b) {
                            jump!(offset);
                        }
                    }
                    Op::JumpNotLt { a, b, offset } => {
                        if !lt!(a, b) {
                            jump!(offset);
                        }
                    }
                    Op::JumpNotLe { a, b, offset } => {
                        if !le!(a, b) {
                            jump!(offset);
                        }
                    }
                    Op::JumpNotGt { a, b, offset } => {
                        if !gt!(a, b) {
                            jump!(offset);
                        }
                    }
                    Op::JumpNotGe { a, b, offset } => {
                        if !ge!(a, b) {
                            jump!(offset);
                        }
                    }
                    Op::JumpStrictEq { a, b, offset } => {
                        if strict_equal!(a, b) {
                            jump!(offset);
                        }
                    }
                    Op::JumpStrictNe { a, b, offset } => {
                        if !strict_equal!(a, b) {
                            jump!(offset);
                        }
                    }

                    Op::JumpEq { a, b, offset } => {
                        if loose_equal!(a, b) {
                            jump!(offset);
                        }
                    }
                    Op::JumpNe { a, b, offset } => {
                        if !loose_equal!(a, b) {
                            jump!(offset);
                        }
                    }
                    Op::JumpLtInt { a, value, offset } => {
                        if lt_int!(a, value) {
                            jump!(offset);
                        }
                    }
                    Op::JumpLeInt { a, value, offset } => {
                        if le_int!(a, value) {
                            jump!(offset);
                        }
                    }
                    Op::JumpGtInt { a, value, offset } => {
                        if gt_int!(a, value) {
                            jump!(offset);
                        }
                    }
                    Op::JumpGeInt { a, value, offset } => {
                        if ge_int!(a, value) {
                            jump!(offset);
                        }
                    }
                    Op::JumpNotLtInt { a, value, offset } => {
                        if !lt_int!(a, value) {
                            jump!(offset);
                        }
                    }
                    Op::JumpNotLeInt { a, value, offset } => {
                        if !le_int!(a, value) {
                            jump!(offset);
                        }
                    }
                    Op::JumpNotGtInt { a, value, offset } => {
                        if !gt_int!(a, value) {
                            jump!(offset);
                        }
                    }
                    Op::JumpNotGeInt { a, value, offset } => {
                        if !ge_int!(a, value) {
                            jump!(offset);
                        }
                    }
                    Op::JumpStrictEqInt { a, value, offset } => {
                        let x = reg!(a);
                        if x == Value::int(i32::from(value))
                            || (x.is_double() && x.as_double() == f64::from(value))
                        {
                            jump!(offset);
                        }
                    }
                    Op::JumpStrictNeInt { a, value, offset } => {
                        let x = reg!(a);
                        if !(x == Value::int(i32::from(value))
                            || (x.is_double() && x.as_double() == f64::from(value)))
                        {
                            jump!(offset);
                        }
                    }
                    Op::JumpTypeIs { a, kind, offset } => {
                        if self.type_kind(reg!(a)) == kind {
                            jump!(offset);
                        }
                    }
                    Op::JumpTypeIsNot { a, kind, offset } => {
                        if self.type_kind(reg!(a)) != kind {
                            jump!(offset);
                        }
                    }
                    Op::IncLoopLt { i, limit, offset } => {
                        let x = reg!(i);
                        reg!(i) = if x.is_int() && x.as_int() != i32::MAX {
                            Value::int(x.as_int() + 1)
                        } else {
                            attempt!(self.step_slow(x, 1))
                        };
                        if lt!(i, limit) {
                            safe_point!();
                            jump!(offset);
                            jit_loop!();
                        }
                    }
                    Op::IncLoopLtInt { i, limit, offset } => {
                        let x = reg!(i);
                        reg!(i) = if x.is_int() && x.as_int() != i32::MAX {
                            Value::int(x.as_int() + 1)
                        } else {
                            attempt!(self.step_slow(x, 1))
                        };
                        if lt_int!(i, limit) {
                            safe_point!();
                            jump!(offset);
                            jit_loop!();
                        }
                    }
                    Op::IncLoopLtLength { i, object, offset } => {
                        let x = reg!(i);
                        reg!(i) = if x.is_int() && x.as_int() != i32::MAX {
                            Value::int(x.as_int() + 1)
                        } else {
                            attempt!(self.step_slow(x, 1))
                        };
                        let target = reg!(object);
                        let length = if target.is_object() {
                            match unsafe { &(*objects!().add(target.index())).kind } {
                                Kind::Array(items) => Value::int_or_double(items.len()),
                                _ => attempt!(self.loop_length(target)),
                            }
                        } else {
                            attempt!(self.loop_length(target))
                        };
                        let x = reg!(i);
                        let goes_on = if Value::both_ints(x, length) {
                            x.as_int() < length.as_int()
                        } else if x.is_number() && length.is_number() {
                            x.as_number() < length.as_number()
                        } else {
                            attempt!(self.less_than(x, length))
                        };
                        if goes_on {
                            safe_point!();
                            jump!(offset);
                            jit_loop!();
                        }
                    }
                    Op::LoopLt { a, b, offset } => {
                        if lt!(a, b) {
                            safe_point!();
                            jump!(offset);
                            jit_loop!();
                        }
                    }
                    Op::LoopLe { a, b, offset } => {
                        if le!(a, b) {
                            safe_point!();
                            jump!(offset);
                            jit_loop!();
                        }
                    }
                    Op::LoopGt { a, b, offset } => {
                        if gt!(a, b) {
                            safe_point!();
                            jump!(offset);
                            jit_loop!();
                        }
                    }
                    Op::LoopGe { a, b, offset } => {
                        if ge!(a, b) {
                            safe_point!();
                            jump!(offset);
                            jit_loop!();
                        }
                    }
                    Op::LoopLtInt { a, value, offset } => {
                        if lt_int!(a, value) {
                            safe_point!();
                            jump!(offset);
                            jit_loop!();
                        }
                    }
                    Op::LoopLeInt { a, value, offset } => {
                        if le_int!(a, value) {
                            safe_point!();
                            jump!(offset);
                            jit_loop!();
                        }
                    }
                    Op::LoopGtInt { a, value, offset } => {
                        if gt_int!(a, value) {
                            safe_point!();
                            jump!(offset);
                            jit_loop!();
                        }
                    }
                    Op::LoopGeInt { a, value, offset } => {
                        if ge_int!(a, value) {
                            safe_point!();
                            jump!(offset);
                            jit_loop!();
                        }
                    }
                    Op::LoopStrictNe { a, b, offset } => {
                        if !strict_equal!(a, b) {
                            safe_point!();
                            jump!(offset);
                            jit_loop!();
                        }
                    }
                    Op::LoopStrictNeInt { a, value, offset } => {
                        let x = reg!(a);
                        if !(x == Value::int(i32::from(value))
                            || (x.is_double() && x.as_double() == f64::from(value)))
                        {
                            safe_point!();
                            jump!(offset);
                            jit_loop!();
                        }
                    }

                    Op::GetGlobal { dst, cache } => {
                        let entry = unsafe { &*ics.add(cache as usize) };
                        let global = unsafe { &*objects!().add(self.global as usize) };
                        reg!(dst) = if global.shape == entry.shape {
                            unsafe { global.read_slot(entry.slot as usize) }
                        } else {
                            attempt!(self.get_global_slow(cache_index!(cache)))
                        };
                    }
                    Op::SetGlobal { src, cache } => {
                        let value = reg!(src);
                        attempt!(self.set_global_slow(cache_index!(cache), value));
                    }
                    Op::SetGlobalSloppy { src, cache } => {
                        let value = reg!(src);
                        let atom = unsafe { (*ics.add(cache as usize)).atom };
                        let global = self.global;
                        let written = {
                            save_pc!();
                            self.top = unsafe { &*frame }.top as usize;
                            self.set_on(global, atom, value)
                        };
                        if let Err(error) = written {
                            if !self.forgive_refusal() {
                                throw!(error);
                            }
                        }
                    }
                    Op::DeclareGlobalVar { name } => {
                        let atom = Atom(name);
                        let global = self.global;
                        if !self.has_own(global, atom) {
                            self.add_property(
                                global,
                                atom,
                                Value::UNDEFINED,
                                WRITABLE | ENUMERABLE,
                            );
                        }
                    }
                    Op::DeclareGlobalFunction { src, name } => {
                        let value = reg!(src);
                        let atom = Atom(name);
                        let global = self.global;
                        self.define(global, atom, value, WRITABLE | ENUMERABLE);
                    }
                    Op::TypeOfGlobal { dst, cache } => {
                        let atom = unsafe { (*ics.add(cache as usize)).atom };
                        let global = Value::object(self.global);
                        let value = attempt!(self.get(global, atom));
                        let kind = self.type_of(value);
                        reg!(dst) = kind;
                    }

                    Op::NewCell { dst, src } => {
                        let value = reg!(src);
                        let cell = self.new_cell(value);
                        reg!(dst) = cell;
                    }
                    Op::CopyCell { reg } => {
                        let old = reg!(reg);
                        let value = unsafe { *cells!().add(old.index()) };
                        let cell = self.new_cell(value);
                        reg!(reg) = cell;
                    }
                    Op::GetCell { dst, cell } => {
                        let value = unsafe { *cells!().add(reg!(cell).index()) };
                        if value.is_empty() {
                            let error = self
                                .reference_error("Cannot access a variable before initialization");
                            throw!(error);
                        }
                        reg!(dst) = value;
                    }
                    Op::SetCell { cell, src } => {
                        let index = reg!(cell).index();
                        let place = unsafe { &mut *cells!().add(index) };
                        if place.is_empty() {
                            let error = self
                                .reference_error("Cannot access a variable before initialization");
                            throw!(error);
                        }
                        *place = reg!(src);
                        self.cells.write_barrier(index, reg!(src));
                    }
                    Op::InitCell { cell, src } => {
                        let index = reg!(cell).index();
                        unsafe { *cells!().add(index) = reg!(src) };
                        self.cells.write_barrier(index, reg!(src));
                    }
                    Op::GetUp { dst, index } => {
                        let value = unsafe { *cells!().add((*ups!().add(index as usize)).index()) };
                        if value.is_empty() {
                            let error = self.tdz_error(index);
                            throw!(error);
                        }
                        reg!(dst) = value;
                    }
                    Op::GetUpValue { dst, index } => {
                        reg!(dst) = unsafe { *ups!().add(index as usize) };
                    }
                    Op::SetUp { index, src } => {
                        let cell = unsafe { (*ups!().add(index as usize)).index() };
                        let place = unsafe { &mut *cells!().add(cell) };
                        if place.is_empty() {
                            let error = self.tdz_error(index);
                            throw!(error);
                        }
                        *place = reg!(src);
                        self.cells.write_barrier(cell, reg!(src));
                    }
                    Op::Closure { dst, proto } => {
                        let closure = self.make_closure(proto, base!());
                        reg!(dst) = closure;
                    }
                    Op::Note { count, kind, .. } => {
                        if self.tracking {
                            let at = unsafe { ip.offset_from(code!()) } as usize - 1;
                            if kind == crate::op::NOTE_KEEP || kind == crate::op::NOTE_KEPT {
                                // A call that's been made here, or given what it gave last time, is stepped over, with
                                // the Note after it (see `keeping.rs`).
                                if attempt!(self.noted(at)) {
                                    jump!(count as usize + 2);
                                    continue;
                                }
                            } else {
                                self.note(at);
                            }
                        }
                        jump!(count);
                    }
                    Op::NoteKey { .. } | Op::NoteName { .. } => {}
                    Op::LoadCallee { dst } => {
                        let closure = unsafe { &*frame }.closure;
                        reg!(dst) = Value::object(closure);
                    }

                    Op::NewObject { dst, capacity } => {
                        let shape = self.realm.object_shape;
                        let object = self.alloc_unrooted(shape, Kind::Plain, capacity as usize);
                        reg!(dst) = object;
                    }
                    Op::NewArray { dst, capacity } => {
                        let array = if capacity <= 4 {
                            self.small_array(&[])
                        } else {
                            self.debt += capacity as usize * 8;
                            let shape = self.realm.array_shape;
                            let items = Vec::with_capacity(capacity as usize);
                            self.alloc_unrooted(shape, Kind::Array(items.into()), 0)
                        };
                        reg!(dst) = array;
                    }
                    Op::ArrayPush { array, value } => {
                        let value = reg!(value);
                        let handle = reg!(array).index();
                        let found = unsafe { &mut *objects!().add(handle) };
                        // The array can have lived through a collection while its elements were worked out.
                        self.objects.write_barrier(found, handle);
                        if let Kind::Array(items) = &mut found.kind {
                            items.push(value);
                        }
                        self.debt += 8;
                    }
                    Op::ArrayHole { array } => {
                        let handle = reg!(array).index();
                        if let Kind::Array(items) = &mut self.objects[handle].kind {
                            items.push(Value::EMPTY);
                        }
                    }
                    Op::ArraySpread { array, source } => {
                        let (array, source) = (reg!(array), reg!(source));
                        attempt!(self.spread_into(array, source));
                    }

                    Op::GetProp { dst, object, cache } => get_prop!(dst, reg!(object), cache, true),
                    Op::SetProp {
                        object,
                        value,
                        cache,
                    } => {
                        let target = reg!(object);
                        let value = reg!(value);
                        let entry = unsafe { &*ics.add(cache as usize) };
                        if target.is_object() {
                            let found = unsafe { &mut *objects!().add(target.index()) };
                            // An old object goes the slow way once, which remembers it (see gc.rs).
                            if found.shape == entry.shape && found.flags & crate::gc::OLD == 0 {
                                if entry.other == NONE {
                                    unsafe { found.write_slot(entry.slot as usize, value) };
                                    continue;
                                }
                                if entry.epoch == self.epoch
                                    && found.flags & (NOT_EXTENSIBLE | IS_PROTO) == 0
                                {
                                    found.shape = entry.other;
                                    found.push_slot(value);
                                    self.debt += 8;
                                    continue;
                                }
                            }
                        }
                        // Another shape: `set_cached` remembers it, and makes this place SetPropPoly.
                        let written = {
                            save_pc!();
                            self.top = unsafe { (*frame).top as usize };
                            self.set_cached(target, value, cache_index!(cache))
                        };
                        if let Err(error) = written {
                            if !self.forgive_refusal() {
                                throw!(error);
                            }
                        }
                    }
                    Op::GetPropPoly { dst, object, cache } => {
                        get_prop_poly!(dst, reg!(object), cache, true)
                    }
                    // As SetProp, with the place's other entries too. Past those, `set_cached` finds the rest in what
                    // writes remember.
                    Op::SetPropPoly {
                        object,
                        value,
                        cache,
                    } => {
                        let target = reg!(object);
                        let value = reg!(value);
                        if target.is_object() {
                            let found = unsafe { &mut *objects!().add(target.index()) };
                            let first = unsafe { &*ics.add(cache as usize) };
                            debug_assert!(first.more != NONE);
                            // An old object goes the slow way once, which remembers it (see gc.rs).
                            let known = 'find: {
                                if found.flags & crate::gc::OLD != 0 {
                                    break 'find None;
                                }
                                if first.shape == found.shape {
                                    break 'find Some(first);
                                }
                                // Too many shapes to keep here: where a write to this shape went before, as
                                // `set_cached` would find it.
                                if first.more & MEGAMORPHIC != 0 {
                                    if found.flags & LAZY_FUNCTION == 0 {
                                        if let Some((to, slot)) =
                                            self.remembered_store(found.shape, first.atom)
                                        {
                                            if to == NONE {
                                                unsafe { found.write_slot(slot as usize, value) };
                                                continue 'run;
                                            }
                                            if found.flags & (NOT_EXTENSIBLE | IS_PROTO) == 0 {
                                                found.shape = to;
                                                found.push_slot(value);
                                                self.debt += 8;
                                                continue 'run;
                                            }
                                        }
                                    }
                                    break 'find None;
                                }
                                let more = unsafe { (first as *const Cache).add(first.more as usize) };
                                let mut at = 0;
                                while at < POLY {
                                    let next = unsafe { &*more.add(at) };
                                    if next.shape == found.shape {
                                        break 'find Some(next);
                                    }
                                    at += 1;
                                }
                                None
                            };
                            if let Some(known) = known {
                                if known.other == NONE {
                                    unsafe { found.write_slot(known.slot as usize, value) };
                                    continue;
                                }
                                if known.epoch == self.epoch
                                    && found.flags & (NOT_EXTENSIBLE | IS_PROTO) == 0
                                {
                                    found.shape = known.other;
                                    found.push_slot(value);
                                    self.debt += 8;
                                    continue;
                                }
                            }
                        }
                        let written = {
                            save_pc!();
                            self.top = unsafe { (*frame).top as usize };
                            self.set_cached(target, value, cache_index!(cache))
                        };
                        if let Err(error) = written {
                            if !self.forgive_refusal() {
                                throw!(error);
                            }
                        }
                    }
                    Op::DefineField {
                        object,
                        value,
                        cache,
                    } => {
                        let target = reg!(object);
                        let value = reg!(value);
                        let entry = unsafe { &*ics.add(cache as usize) };
                        let found = unsafe { &mut *objects!().add(target.index()) };
                        if found.shape == entry.shape && found.flags & crate::gc::OLD == 0 {
                            found.shape = entry.other;
                            found.push_slot(value);
                            continue;
                        }
                        attempt!(self.define_cached(target.handle(), value, cache_index!(cache)));
                    }
                    Op::GetLength { dst, object, cache } => {
                        let target = reg!(object);
                        reg!(dst) = if target.is_object() {
                            // `arguments.length` is always how many there are, as for an array (`own_special`).
                            if let Kind::Array(items) | Kind::Arguments(items) =
                                unsafe { &(*objects!().add(target.index())).kind }
                            {
                                Value::int_or_double(items.len())
                            } else {
                                attempt!(self.get_cached(target, cache_index!(cache)))
                            }
                        } else if target.is_string() {
                            Value::int(self.strings.items[target.index()].text.len() as i32)
                        } else {
                            attempt!(self.get_cached(target, cache_index!(cache)))
                        };
                    }
                    Op::GetElem { dst, object, key } => {
                        let (target, key) = (reg!(object), reg!(key));
                        if target.is_object() && key.is_int() {
                            // A negative index turns into a huge one, which is too big as well.
                            let index = key.as_int() as u32 as usize;
                            match unsafe { &(*objects!().add(target.index())).kind } {
                                Kind::Array(items) | Kind::Arguments(items) => {
                                    if let Some(value) = items.get(index) {
                                        if !value.is_empty() {
                                            reg!(dst) = *value;
                                            continue;
                                        }
                                    }
                                }
                                Kind::TypedArray(data) => {
                                    if data.epoch == self.buffer_epoch && index < data.quick_length {
                                        reg!(dst) = unsafe { data.read_quick(index) };
                                        continue;
                                    }
                                    if let Some(value) = self.typed_get_quick(target.handle(), index) {
                                        reg!(dst) = value;
                                        continue;
                                    }
                                }
                                _ => {}
                            }
                        }
                        reg!(dst) = attempt!(self.get_by_value_here(target, key, dst));
                    }
                    Op::SetElem { object, key, value } => {
                        let (target, key, value) = (reg!(object), reg!(key), reg!(value));
                        if target.is_object() && key.is_int() {
                            let found = unsafe { &mut *objects!().add(target.index()) };
                            // An old array goes the slow way once, which remembers it (see gc.rs).
                            if found.flags & (FROZEN_ELEMENTS | crate::gc::OLD) == 0 {
                                if let Kind::Array(items) = &mut found.kind {
                                    if let Some(slot) = items.get_mut(key.as_int() as u32 as usize)
                                    {
                                        if !slot.is_empty() {
                                            *slot = value;
                                            continue;
                                        }
                                    }
                                }
                            }
                            if key.as_int() >= 0
                                && self.typed_set_quick(
                                    target.handle(),
                                    key.as_int() as usize,
                                    value,
                                )
                            {
                                save_pc!();
                                self.go_typed_set();
                                continue;
                            }
                        }
                        let written = {
                            save_pc!();
                            self.top = unsafe { &*frame }.top as usize;
                            self.set_by_value(target, key, value)
                        };
                        if let Err(error) = written {
                            if !self.forgive_refusal() {
                                throw!(error);
                            }
                        }
                    }
                    Op::SetElemTyped { object, key, value } => {
                        let (target, key, item) = (reg!(object), reg!(key), reg!(value));
                        if target.is_object() && key.is_int() {
                            if let Kind::TypedArray(data) =
                                unsafe { &(*objects!().add(target.index())).kind }
                            {
                                let index = key.as_int() as u32 as usize;
                                if data.epoch == self.buffer_epoch
                                    && index < data.quick_length
                                    && item.is_number()
                                {
                                    unsafe { data.write_quick(index, item) };
                                    continue;
                                }
                            }
                        }
                        // Anything else makes it SetElem again, which then runs.
                        save_pc!();
                        self.go_untyped_set();
                        ip = unsafe { ip.sub(1) };
                    }
                    Op::DefineElem { object, key, value } => {
                        let (target, key, value) = (reg!(object), reg!(key), reg!(value));
                        let atom = attempt!(self.to_atom(key));
                        if self.atoms.kind_of(atom) == AtomKind::Hidden {
                            attempt!(self.add_private(target.handle(), atom, value, WRITABLE));
                        } else {
                            self.define(target.handle(), atom, value, PLAIN);
                        }
                    }
                    Op::Delete { dst, object, key } => {
                        let (target, key) = (reg!(object), reg!(key));
                        let deleted = {
                            save_pc!();
                            self.top = unsafe { &*frame }.top as usize;
                            self.delete_by_value(target, key)
                        };
                        reg!(dst) = match deleted {
                            Ok(done) => Value::bool(done),
                            Err(error) => {
                                if !self.forgive_refusal() {
                                    throw!(error);
                                }
                                Value::FALSE
                            }
                        };
                    }
                    Op::ObjectSpread { object, source } => {
                        let (target, source) = (reg!(object), reg!(source));
                        attempt!(self.copy_properties(target, source, &[]));
                    }
                    Op::ObjectRest { dst, source, keys } => {
                        let (source, keys) = (reg!(source), reg!(keys));
                        let excluded = attempt!(self.keys_to_atoms(keys));
                        let target = self.new_plain();
                        reg!(dst) = target;
                        attempt!(self.copy_properties(target, source, &excluded));
                    }
                    Op::DefineGetter {
                        object,
                        key,
                        function,
                    } => {
                        let (target, key, function) = (reg!(object), reg!(key), reg!(function));
                        let atom = attempt!(self.to_atom(key));
                        self.define_accessor(
                            target.handle(),
                            atom,
                            Some(function),
                            None,
                            ENUMERABLE | CONFIGURABLE,
                        );
                    }
                    Op::DefineSetter {
                        object,
                        key,
                        function,
                    } => {
                        let (target, key, function) = (reg!(object), reg!(key), reg!(function));
                        let atom = attempt!(self.to_atom(key));
                        self.define_accessor(
                            target.handle(),
                            atom,
                            None,
                            Some(function),
                            ENUMERABLE | CONFIGURABLE,
                        );
                    }
                    Op::DefineMethod {
                        object,
                        key,
                        function,
                    } => {
                        let (target, key, function) = (reg!(object), reg!(key), reg!(function));
                        let atom = attempt!(self.to_atom(key));
                        // A private method can't be written to, where any other can.
                        let flags = if self.atoms.kind_of(atom) == AtomKind::Hidden {
                            HIDDEN & !WRITABLE
                        } else {
                            HIDDEN
                        };
                        self.define(target.handle(), atom, function, flags);
                    }
                    Op::DefineClassGetter {
                        object,
                        key,
                        function,
                    } => {
                        let (target, key, function) = (reg!(object), reg!(key), reg!(function));
                        let atom = attempt!(self.to_atom(key));
                        self.define_accessor(
                            target.handle(),
                            atom,
                            Some(function),
                            None,
                            CONFIGURABLE,
                        );
                    }
                    Op::DefineClassSetter {
                        object,
                        key,
                        function,
                    } => {
                        let (target, key, function) = (reg!(object), reg!(key), reg!(function));
                        let atom = attempt!(self.to_atom(key));
                        self.define_accessor(
                            target.handle(),
                            atom,
                            None,
                            Some(function),
                            CONFIGURABLE,
                        );
                    }
                    Op::SetHome { function, home } => {
                        let new_home = reg!(home).handle();
                        let function_handle = reg!(function).index();
                        if let Kind::Function { home, .. } = &mut self.objects[function_handle].kind
                        {
                            *home = new_home;
                        }
                    }

                    Op::Call { dst, f, argc } => call_function!(dst, f, argc),
                    Op::CallMath { dst, f, which } => {
                        let callee = reg!(f);
                        // The compiler gave it as many arguments as the function takes.
                        if let Some(value) =
                            unsafe { self.math_quick(which, callee, regs.add(f as usize + 2)) }
                        {
                            reg!(dst) = value;
                            continue;
                        }
                        save_pc!();
                        self.go_plain_call();
                        ip = unsafe { ip.sub(1) };
                    }
                    Op::CallPlain { dst, f, argc } => {
                        reg!(f + 1) = Value::UNDEFINED;
                        call_function!(dst, f, argc);
                    }
                    Op::CallMethod { dst, f, cache } => {
                        get_prop!(f, reg!(f + 1), cache, false);
                        call_function!(dst, f, 0);
                    }
                    Op::CallMethodPoly { dst, f, cache } => {
                        get_prop_poly!(f, reg!(f + 1), cache, false);
                        call_function!(dst, f, 0);
                    }
                    Op::New { dst, f, argc } => {
                        safe_point!();
                        let callee = reg!(f);
                        if callee.is_object() {
                            if let Kind::Function {
                                proto, upvalues, ..
                            } = unsafe { &(*objects!().add(callee.index())).kind }
                            {
                                let proto = *proto;
                                let flags = unsafe { (*protos!().add(proto as usize)).flags };
                                if flags & IS_CONSTRUCTOR != 0 {
                                    let this_value = if flags & IS_DERIVED != 0 {
                                        Value::EMPTY
                                    } else {
                                        attempt!(self.create_this(callee))
                                    };
                                    reg!(f + 1) = this_value;
                                    if flags & KEEPS_ARGS != 0 {
                                        enter_slowly!(
                                            callee.handle(),
                                            proto,
                                            f,
                                            argc,
                                            dst,
                                            CONSTRUCTING,
                                            callee
                                        );
                                    }
                                    enter!(
                                        callee.handle(),
                                        proto,
                                        unsafe { (*protos!().add(proto as usize)).start },
                                        upvalues.as_ptr(),
                                        f,
                                        argc,
                                        dst,
                                        CONSTRUCTING,
                                        callee
                                    );
                                }
                            }
                        }
                        reg!(dst) = attempt!(self.call_at(
                            base!() + f as usize,
                            argc as usize,
                            Some(callee)
                        ));
                    }
                    Op::CallSpread { dst, f, array } => {
                        let (callee, this, array) = (reg!(f), reg!(f + 1), reg!(array));
                        if callee.is_object() {
                            if let (
                                Kind::Function {
                                    proto, upvalues, ..
                                },
                                Kind::Array(items),
                            ) = unsafe {
                                (
                                    &(*objects!().add(callee.index())).kind,
                                    &(*objects!().add(array.index())).kind,
                                )
                            } {
                                let proto = *proto;
                                let flags = unsafe { (*protos!().add(proto as usize)).flags };
                                let count = items.len();
                                let room = base!() + f as usize + 2 + count + 16;
                                if flags & (IS_CLASS | KEEPS_ARGS) == 0 && room <= self.stack.len() {
                                    let first = f as usize + 2;
                                    for (at, item) in items.iter().enumerate() {
                                        reg!(first + at) = if item.is_empty() {
                                            Value::UNDEFINED
                                        } else {
                                            *item
                                        };
                                    }
                                    enter!(
                                        callee.handle(),
                                        proto,
                                        unsafe { (*protos!().add(proto as usize)).start },
                                        upvalues.as_ptr(),
                                        f,
                                        count,
                                        dst,
                                        0,
                                        Value::UNDEFINED
                                    );
                                }
                            }
                        }
                        let args = self.array_items(array);
                        reg!(dst) = attempt!(self.call(callee, this, &args));
                    }
                    Op::NewSpread { dst, f, array } => {
                        let (callee, array) = (reg!(f), reg!(array));
                        let args = self.array_items(array);
                        reg!(dst) = attempt!(self.construct(callee, &args));
                    }
                    Op::SuperCall { dst, f, argc } => {
                        let closure = unsafe { (*frame).closure };
                        let new_target = unsafe { (*frame).new_target };
                        let (parent, new_target) =
                            attempt!(self.super_constructor(closure, new_target));
                        reg!(f) = parent;
                        if let Kind::Function {
                            proto, upvalues, ..
                        } = unsafe { &(*objects!().add(parent.index())).kind }
                        {
                            let proto = *proto;
                            let flags = unsafe { (*protos!().add(proto as usize)).flags };
                            if flags & IS_CONSTRUCTOR != 0 {
                                let this_value = if flags & IS_DERIVED != 0 {
                                    Value::EMPTY
                                } else {
                                    attempt!(self.create_this(new_target))
                                };
                                reg!(f + 1) = this_value;
                                if flags & KEEPS_ARGS != 0 {
                                    enter_slowly!(
                                        parent.handle(),
                                        proto,
                                        f,
                                        argc,
                                        dst,
                                        CONSTRUCTING,
                                        new_target
                                    );
                                }
                                enter!(
                                    parent.handle(),
                                    proto,
                                    unsafe { (*protos!().add(proto as usize)).start },
                                    upvalues.as_ptr(),
                                    f,
                                    argc,
                                    dst,
                                    CONSTRUCTING,
                                    new_target
                                );
                            }
                        }
                        reg!(dst) = attempt!(self.call_at(
                            base!() + f as usize,
                            argc as usize,
                            Some(new_target)
                        ));
                    }
                    Op::SuperCallSpread { dst, f, array } => {
                        let array = reg!(array);
                        reg!(dst) = attempt!(self.super_call_spread_here(f, array, dst));
                    }
                    Op::Return { src } => finish_return!(reg!(src)),
                    Op::ReturnUndefined => finish_return!(Value::UNDEFINED),
                    Op::Throw { src } => throw!(reg!(src)),
                    Op::CheckObjectCoercible { src } => {
                        if reg!(src).is_nullish() {
                            let shown = reg!(src);
                            let shown = self.brief(shown);
                            let error = self.type_error(&format!(
                                "Cannot destructure '{shown}' as it is {shown}."
                            ));
                            throw!(error);
                        }
                    }

                    Op::GetIterator { iter, source } => {
                        let source = reg!(source);
                        let (first, second) = attempt!(self.get_iterator(source));
                        reg!(iter) = first;
                        reg!(iter + 1) = second;
                    }
                    Op::IterNext {
                        value,
                        iter,
                        offset,
                    } => {
                        let (first, second) = (reg!(iter), reg!(iter + 1));
                        if first.is_empty() {
                            reg!(value) = Value::UNDEFINED;
                            jump!(offset);
                            continue;
                        }
                        if second.is_int() {
                            if let Kind::Array(items) =
                                unsafe { &(*objects!().add(first.index())).kind }
                            {
                                let at = second.as_int() as usize;
                                if at < items.len() {
                                    let item = unsafe { *items.get_unchecked(at) };
                                    reg!(iter + 1) = Value::int(at as i32 + 1);
                                    reg!(value) = if item.is_empty() {
                                        Value::UNDEFINED
                                    } else {
                                        item
                                    };
                                    continue;
                                }
                            }
                            reg!(iter) = Value::EMPTY;
                            reg!(value) = Value::UNDEFINED;
                            jump!(offset);
                            continue;
                        }
                        let item = attempt!(self.step_in_place(first, second, value));
                        if item.is_empty() {
                            reg!(iter) = Value::EMPTY;
                            reg!(value) = Value::UNDEFINED;
                            jump!(offset);
                        } else {
                            reg!(value) = item;
                        }
                    }
                    Op::IterClose { iter } => {
                        let (first, second) = (reg!(iter), reg!(iter + 1));
                        if !first.is_empty() && !second.is_int() {
                            attempt!(self.iterator_close(first));
                        }
                    }
                    Op::IterCloseQuietly { iter } => {
                        let (first, second) = (reg!(iter), reg!(iter + 1));
                        if !first.is_empty() && !second.is_int() {
                            save_pc!();
                            self.top = unsafe { &*frame }.top as usize;
                            let _ = self.iterator_close(first);
                        }
                    }
                    Op::GetAsyncIterator { iter, source } => {
                        let source = reg!(source);
                        let (first, second) = attempt!(self.get_async_iterator(source));
                        reg!(iter) = first;
                        reg!(iter + 1) = second;
                    }
                    Op::AsyncIterCall { dst, iter } => {
                        let (first, second) = (reg!(iter), reg!(iter + 1));
                        reg!(dst) = attempt!(self.call(second, first, &[]));
                    }
                    Op::IterResult { value, offset } => {
                        let result = reg!(value);
                        if !result.is_object() {
                            let error = self.type_error("Iterator result is not an object");
                            throw!(error);
                        }
                        let done = attempt!(self.get(result, text::DONE));
                        reg!(value) = attempt!(self.get(result, text::VALUE));
                        if self.to_boolean(done) {
                            jump!(offset);
                        }
                    }
                    Op::DelegateNext {
                        dst,
                        iter,
                        received,
                    } => {
                        let (first, second, sent) = (reg!(iter), reg!(iter + 1), reg!(received));
                        if second.is_int() {
                            // An array, with its place kept in the register after it.
                            let at = second.as_int() as usize;
                            let item = match &self.objects[first.index()].kind {
                                Kind::Array(items) => items.get(at).copied(),
                                _ => None,
                            };
                            if item.is_some() {
                                reg!(iter + 1) = Value::int(at as i32 + 1);
                            }
                            let result = match item {
                                Some(found) if found.is_empty() => {
                                    self.iter_result(Value::UNDEFINED, false)
                                }
                                Some(found) => self.iter_result(found, false),
                                None => self.iter_result(Value::UNDEFINED, true),
                            };
                            reg!(dst) = result;
                        } else {
                            reg!(dst) = attempt!(self.delegate_next(first, second, sent));
                        }
                    }
                    Op::ForInKeys { dst, source } => {
                        let source = reg!(source);
                        reg!(dst) = attempt!(self.for_in_keys(source));
                    }

                    Op::CreateArguments { dst } => reg!(dst) = self.create_arguments(),
                    Op::Rest { dst, start } => {
                        // Nothing else needs the arguments by now, so the array can just have them.
                        let args = if unsafe { (*frame).flags } & KEPT_ARGS != 0 {
                            self.kept_args.last().map_or(&[][..], Vec::as_slice)
                        } else {
                            &[]
                        };
                        let rest = args.get(start as usize..).unwrap_or(&[]);
                        let array = if rest.len() <= 4 {
                            let mut few = [Value::UNDEFINED; 4];
                            few[..rest.len()].copy_from_slice(rest);
                            let count = rest.len();
                            self.small_array(&few[..count])
                        } else {
                            let items = rest.to_vec();
                            self.new_array(items)
                        };
                        reg!(dst) = array;
                    }
                    Op::NewTarget { dst } => {
                        reg!(dst) = self
                            .frames
                            .last()
                            .expect("there's a frame running")
                            .new_target;
                    }
                    Op::NewClass {
                        dst,
                        constructor,
                        parent,
                    } => {
                        let (constructor, parent) = (reg!(constructor), reg!(parent));
                        reg!(dst) = attempt!(self.new_class(constructor, parent));
                    }
                    Op::SetFields {
                        constructor,
                        function,
                    } => {
                        let (constructor, function) = (reg!(constructor), reg!(function));
                        self.define(constructor.handle(), text::FIELDS, function, 0);
                    }
                    Op::SetPrivateMethods {
                        constructor,
                        methods,
                    } => {
                        let (constructor, methods) = (reg!(constructor), reg!(methods));
                        self.define(constructor.handle(), text::PRIVATE_METHODS, methods, 0);
                    }
                    Op::RunFields { this } => {
                        let this_reg = this;
                        let this = reg!(this);
                        let closure = unsafe { &*frame }.closure;
                        attempt!(self.run_fields_here(closure, this, this_reg));
                    }
                    Op::SuperGet { dst, key, this } => {
                        let (key, this) = (reg!(key), reg!(this));
                        let closure = unsafe { &*frame }.closure;
                        reg!(dst) = attempt!(self.super_get(closure, key, this));
                    }
                    Op::SuperSet { key, value, this } => {
                        let (key, value, this) = (reg!(key), reg!(value), reg!(this));
                        let atom = attempt!(self.to_atom(key));
                        attempt!(self.set(this, atom, value));
                    }
                    Op::SloppyThis => {
                        let this = reg!(0);
                        if this.is_nullish() {
                            reg!(0) = Value::object(self.global);
                        } else if !this.is_object() {
                            reg!(0) = attempt!(self.to_object(this));
                        }
                    }
                    Op::ToObject { dst, src } => {
                        let value = reg!(src);
                        reg!(dst) = attempt!(self.to_object(value));
                    }
                    Op::CheckThis { reg } => {
                        if reg!(reg).is_empty() {
                            let error = self.reference_error(
                            "Must call super constructor in derived class before accessing 'this' or returning from derived constructor",
                        );
                            throw!(error);
                        }
                    }
                    Op::ThrowConstAssign { name } => {
                        let _ = name;
                        let error = self.type_error("Assignment to constant variable.");
                        throw!(error);
                    }
                    Op::ThrowTdz { name } => {
                        let name = self.atom_text(Atom(name));
                        let error = self.reference_error(&format!(
                            "Cannot access '{name}' before initialization"
                        ));
                        throw!(error);
                    }
                    // Only in code being debugged, where it's only more than a load while something's to be stopped for.
                    Op::Step { line } => {
                        if crate::inspect::DEBUG_WANTED.load(std::sync::atomic::Ordering::Relaxed) {
                            save_pc!();
                            self.top = unsafe { (*frame).top as usize };
                            if let Err(error) = self.at_step(line) {
                                throw!(error);
                            }
                            reload!();
                        }
                    }

                    Op::InitialYield => {
                        save_pc!();
                        let generator = self.suspend_new_generator();
                        let closure = unsafe { &*frame }.closure;
                        let proto = unsafe { &*frame }.proto;
                        let ret_dst = unsafe { &*frame }.ret_dst;
                        self.pop_frame();
                        let flags = self.protos[proto as usize].flags;
                        let result = if flags & IS_GENERATOR != 0 {
                            let default = if flags & IS_ASYNC != 0 {
                                self.realm.async_generator_proto
                            } else {
                                self.realm.generator_proto
                            };
                            let chosen = match self.get(Value::object(closure), text::PROTOTYPE) {
                                Ok(found) if found.is_object() => found.handle(),
                                _ => default,
                            };
                            if chosen != self.realm.generator_proto {
                                self.set_proto(generator.handle(), chosen);
                            }
                            generator
                        } else if flags & IS_ASYNC != 0 {
                            // Async functions start right away, on a stack of their own.
                            self.top = match self.frames.last() {
                                Some(frame) if self.frames.len() > stop => frame.top as usize,
                                _ => self.top,
                            };
                            self.start_async(generator)
                        } else {
                            generator
                        };
                        if self.frames.len() == stop {
                            return Ok(result);
                        }
                        reload!();
                        unsafe { *regs.add(ret_dst as usize) = result };
                    }
                    Op::Yield { dst, src } => {
                        let value = reg!(src);
                        save_pc!();
                        if unsafe { (*frame).flags } & RESUMED_HERE != 0 {
                            self.yield_here(dst, value);
                            throw!(Value::ENTERED);
                        }
                        self.suspend_generator(dst);
                        self.pop_frame();
                        self.yielded = crate::generator::Suspended::Yield;
                        return Ok(value);
                    }
                    Op::Await { dst, src } => {
                        let value = reg!(src);
                        save_pc!();
                        self.suspend_generator(dst);
                        self.pop_frame();
                        self.yielded = crate::generator::Suspended::Await;
                        return Ok(value);
                    }
                }
            };
            // Not an error: a handler made a frame, or ended one, by way of a function of its own (`call_indirect`,
            // `step_in_place`, `finish_special_frame`, `yield_here`), and the loop goes on with the frame on top. Coming
            // this way keeps the reload out of each of those handlers.
            if thrown == Value::ENTERED {
                // A generator that compiled code carried on in place (`jit::helpers::iter_next`) has yielded or
                // ended, leaving the compiled frame under this loop's to carry on.
                #[cfg(feature = "jit")]
                if jit && self.frames.len() <= stop {
                    return Ok(Value::UNDEFINED);
                }
                reload!();
                continue;
            }
            save_pc!();
            if self.unwind(thrown, stop) {
                reload!();
                continue;
            }
            return Err(thrown);
        }
    }

    /// The `arguments` of the frame on top. In sloppy code it's made with the shape the last one had, as they all have
    /// the same two properties, which adding one at a time looked up each time.
    #[inline(never)]
    pub(crate) fn create_arguments(&mut self) -> Value {
        let frame = *self.frames.last().expect("there's a frame running");
        let items = if frame.flags & KEPT_ARGS != 0 {
            self.kept_args.last().cloned().unwrap_or_default()
        } else {
            Vec::new()
        };
        let strict = self.protos[frame.proto as usize].strict();
        let shape = if strict || self.arguments_shape == NONE {
            self.realm.object_shape
        } else {
            self.arguments_shape
        };
        let arguments = self.alloc_object(shape, Kind::Arguments(items.into()), 2);
        let values = self.realm.array_values.0;
        let callee = Value::object(frame.closure);
        if shape == self.arguments_shape {
            self.objects[arguments.index()].set_slots(&[values, callee]);
            return arguments;
        }
        self.add_property(
            arguments.handle(),
            text::SYMBOL_ITERATOR,
            values,
            HIDDEN,
        );
        if strict {
            let thrower = self.throw_type_error;
            self.define_accessor(
                arguments.handle(),
                text::CALLEE,
                Some(thrower),
                Some(thrower),
                0,
            );
        } else {
            self.add_property(arguments.handle(), text::CALLEE, callee, HIDDEN);
            self.arguments_shape = self.objects[arguments.index()].shape;
        }
        arguments
    }

    /// Finishing a frame that needs more than the value handing back.
    #[inline(never)]
    pub(crate) fn finish_special_frame(&mut self, value: Value) -> Run<Value> {
        let frame = *self.frames.last().expect("there's a frame running");
        if frame.flags & RESUMED_HERE != 0 {
            // The loop that carried it on has ended too.
            self.end_resumed_generator();
            return Err(Value::ENTERED);
        }
        let mut value = value;
        if frame.flags & CONSTRUCTING != 0 && !value.is_object() {
            value = self.stack[frame.base as usize];
            if value.is_empty() {
                return Err(self.reference_error(
                    "Must call super constructor in derived class before accessing 'this' or returning from derived constructor",
                ));
            }
        }
        if frame.flags & KEPT_ARGS != 0 {
            self.finish_kept_args();
        }
        Ok(value)
    }

    /// The next result of what a `yield*` is going through, as the `{ value, done }` that it would have made.
    pub(crate) fn delegate_next(&mut self, iterator: Value, next: Value, sent: Value) -> Run<Value> {
        if matches!(self.objects[iterator.index()].kind, Kind::Iter(_)) {
            return Ok(match self.native_iter_step(iterator.handle())? {
                Some(item) => self.iter_result(item, false),
                None => self.iter_result(Value::UNDEFINED, true),
            });
        }
        self.call(next, iterator, &[sent])
    }

    /// Code that isn't strict carries on when a write is refused. It says if that's what's happening here.
    pub(crate) fn forgive_refusal(&mut self) -> bool {
        let refused = std::mem::replace(&mut self.write_refused, false);
        let frame = self.frames.last().expect("there's a frame running");
        refused && !self.protos[frame.proto as usize].strict()
    }

    pub(crate) fn tdz_error(&mut self, index: u16) -> Value {
        let frame = self.frames.last().expect("there's a frame running");
        let name = self.protos[frame.proto as usize].capture_names[index as usize];
        let name = self.atom_text(name);
        self.reference_error(&format!("Cannot access '{name}' before initialization"))
    }

    /// What a closure of this proto keeps as its `entry` (see [`Kind::Function`]): where its code starts, if a call can
    /// go straight there, which it can't for a class, or a function that keeps its arguments or is yet to be compiled
    /// (that has `KEEPS_ARGS` until it is).
    pub(crate) fn entry_of(&self, proto_id: u32) -> u32 {
        let proto = &self.protos[proto_id as usize];
        if proto.flags & (IS_CLASS | KEEPS_ARGS) == 0 {
            proto.start
        } else {
            NONE
        }
    }

    #[inline(never)]
    pub(crate) fn make_closure(&mut self, proto_id: u32, base: usize) -> Value {
        let parent = self.frames.last().expect("there's a frame running").closure;
        let proto = &self.protos[proto_id as usize];
        let flags = proto.flags;
        let upvalues = if proto.captures.is_empty() {
            Upvalues::NONE
        } else {
            let parent_upvalues: &[Value] = match &self.objects[parent as usize].kind {
                Kind::Function { upvalues, .. } => upvalues,
                _ => &[],
            };
            Upvalues::new(
                proto.captures.len(),
                proto.captures.iter().map(|capture| {
                    if capture.from_register {
                        self.stack[base + capture.index as usize]
                    } else {
                        parent_upvalues[capture.index as usize]
                    }
                }),
            )
        };
        self.debt += upvalues.len() * 8;
        let home = if flags & IS_ARROW != 0 {
            match &self.objects[parent as usize].kind {
                Kind::Function { home, .. } => *home,
                _ => NONE,
            }
        } else {
            NONE
        };
        let shape = match (flags & IS_GENERATOR != 0, flags & IS_ASYNC != 0) {
            (false, false) => self.realm.function_shape,
            (true, false) => self.realm.generator_function_shape,
            (false, true) => self.realm.async_function_shape,
            (true, true) => self.realm.async_generator_function_shape,
        };
        let entry = self.entry_of(proto_id);
        let closure = self.alloc_unrooted(
            shape,
            Kind::Function {
                proto: proto_id,
                home,
                entry,
                upvalues,
            },
            0,
        );
        self.objects[closure.index()].flags |= LAZY_FUNCTION;
        if self.tracking {
            self.note_closure(closure);
        }
        closure
    }

    /// Runs a script, whose `this` is the global object.
    pub(crate) fn run_script(&mut self, proto_id: u32) -> Run<Value> {
        self.run_proto_with_this(proto_id, Value::object(self.global), &[])
    }

    /// Runs a compiled program, with `args` as its parameters if it's a module.
    pub(crate) fn run_proto(&mut self, proto_id: u32, args: &[Value]) -> Run<Value> {
        self.run_proto_with_this(proto_id, Value::UNDEFINED, args)
    }

    fn run_proto_with_this(&mut self, proto_id: u32, this: Value, args: &[Value]) -> Run<Value> {
        let saved_frames = self.frames.len();
        let start = self.top;
        let closure = self.alloc_object(
            self.realm.function_shape,
            Kind::Function {
                proto: proto_id,
                home: NONE,
                entry: NONE,
                upvalues: Upvalues::NONE,
            },
            0,
        );
        self.objects[closure.index()].flags |= LAZY_FUNCTION;
        let needed = start + args.len() + 2;
        self.reserve_stack(needed + 16)?;
        self.stack[start] = closure;
        self.stack[start + 1] = this;
        self.stack[start + 2..needed].copy_from_slice(args);
        self.top = needed;
        self.enter_function(
            closure.handle(),
            proto_id,
            start + 1,
            args.len(),
            0,
            0,
            Value::UNDEFINED,
        )?;
        let result = self.run(saved_frames);
        self.top = start;
        result
    }

    pub(crate) fn enter_generator_frame(&mut self, generator: u32) -> Run<()> {
        let Kind::Generator(data) = &mut self.objects[generator as usize].kind else {
            unreachable!("only generators are resumed");
        };
        let count = data.registers.len();
        let extra_args = data.extra_args.take();
        let (proto_id, closure, pc, new_target) =
            (data.proto, data.closure, data.pc, data.new_target);
        let new_base = self.top + 1;
        let top = new_base + count;
        if self.frames.len() >= MAX_FRAMES {
            return Err(self.range_error("Maximum call stack size exceeded"));
        }
        self.reserve_stack(top + 16)?;
        // The list stays with the generator, to be filled in again when it next stops.
        if let Kind::Generator(data) = &self.objects[generator as usize].kind {
            self.stack[new_base..top].copy_from_slice(&data.registers);
        }
        let cache_base = self.protos[proto_id as usize].cache_base;
        let mut flags = 0;
        if let Some(args) = extra_args {
            self.kept_args.push(args);
            flags = KEPT_ARGS;
        }
        let upvalues = match &self.objects[closure as usize].kind {
            Kind::Function { upvalues, .. } => upvalues.as_ptr(),
            _ => std::ptr::null(),
        };
        self.frames.push(Frame {
            pc,
            base: new_base as u32,
            top: top as u32,
            proto: proto_id,
            closure,
            cache_base,
            ret_dst: 0,
            flags,
            new_target,
            upvalues,
        });
        Ok(())
    }

    /// The next value for `for of` (`IterNext`, its value going to register `dst`), or `EMPTY` once there are none.
    /// A generator with the built-in `next` is carried on with in the interpreter that's running, rather than started
    /// again from Rust: its frame goes on top, with `RESUMED_HERE`, and this fails with [`Value::ENTERED`]. Its `yield`
    /// then puts the value in `dst` and carries on after the `IterNext` (`yield_here`), and its end leaves the loop
    /// (`end_resumed_generator`), as `IterNext` would have.
    #[inline(never)]
    fn step_in_place(&mut self, iterator: Value, next: Value, dst: u16) -> Run<Value> {
        if self.resume_in_place(iterator, next, dst)? {
            return Err(Value::ENTERED);
        }
        Ok(self
            .iterator_step(iterator, next)?
            .unwrap_or(Value::EMPTY))
    }

    /// Whether what `for of` goes through is a generator with the built-in `next` that can be carried on in place.
    #[inline]
    pub(crate) fn resumes_in_place(&self, iterator: Value, next: Value) -> bool {
        iterator.is_object()
            && next.is_object()
            && matches!(&self.objects[next.index()].kind,
                Kind::Native(native) if native.func as usize == Self::generator_next as NativeFn as usize)
            && matches!(&self.objects[iterator.index()].kind,
                Kind::Generator(data) if data.state == GeneratorState::Suspended
                    && self.protos[data.proto as usize].flags & IS_ASYNC == 0
                    && !self.has_code(data.proto))
    }

    /// Whether a function has been compiled to machine code, which carries on a generator better than doing it in place:
    /// `for of` then resumes it in its code (see `jit::helpers::iter_next`).
    #[inline(always)]
    fn has_code(&self, proto: u32) -> bool {
        #[cfg(feature = "jit")]
        return crate::jit::has_code(self.jit.slots[proto as usize]);
        #[cfg(not(feature = "jit"))]
        {
            let _ = proto;
            false
        }
    }

    /// For [`Vm::step_in_place`]: if what `for of` goes through is a generator with the built-in `next`, puts its frame
    /// on top, with `RESUMED_HERE`, so that its `yield` goes to register `dst` of the frame under it, and says so.
    #[inline]
    pub(crate) fn resume_in_place(&mut self, iterator: Value, next: Value, dst: u16) -> Run<bool> {
        if self.resumes_in_place(iterator, next) {
            self.enter_in_place(iterator, dst)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// For [`Vm::resume_in_place`]: puts the frame of `iterator`, a suspended generator, on top, with `RESUMED_HERE`,
    /// so that its `yield` goes to register `dst` of the frame under it.
    #[inline]
    pub(crate) fn enter_in_place(&mut self, iterator: Value, dst: u16) -> Run<()> {
        if let Kind::Generator(data) = &self.objects[iterator.index()].kind {
            let handle = iterator.handle();
            let resume_register = data.resume_register;
            self.enter_generator_frame(handle)?;
            if let Kind::Generator(data) = &mut self.objects[handle as usize].kind {
                data.state = GeneratorState::Running;
            }
            let frame = self.frames.last_mut().expect("a frame was just made");
            frame.flags |= RESUMED_HERE;
            frame.ret_dst = dst;
            let base = frame.base as usize;
            if resume_register != crate::generator::NOT_STARTED {
                self.stack[base + resume_register as usize] = Value::UNDEFINED;
            }
            self.resumed_here.push(self.current_generator);
            self.current_generator = handle;
        }
        Ok(())
    }

    /// A `yield` in a frame with `RESUMED_HERE`: the generator stops, and the loop that carried it on gets the value.
    #[inline(never)]
    pub(crate) fn yield_here(&mut self, resume_register: u16, value: Value) {
        self.suspend_generator(resume_register);
        self.leave_resumed_generator(GeneratorState::Suspended);
        let frame = self.frames.pop().expect("the generator's frame is running");
        let caller = self.frames.last().expect("the loop's frame is under it");
        self.stack[caller.base as usize + frame.ret_dst as usize] = value;
    }

    /// A frame with `RESUMED_HERE` has come to its end, so the generator is done, and the `for of` that carried it on
    /// leaves its loop, as its `IterNext` would have.
    fn end_resumed_generator(&mut self) {
        let frame = self.frames.pop().expect("the generator's frame is running");
        if frame.flags & KEPT_ARGS != 0 {
            self.finish_kept_args();
        }
        self.leave_resumed_generator(GeneratorState::Done);
        let caller = self.frames.last_mut().expect("the loop's frame is under it");
        let Op::IterNext {
            value,
            iter,
            offset,
        } = self.code[caller.pc as usize - 1]
        else {
            unreachable!("only IterNext carries a generator on in place");
        };
        let base = caller.base as usize;
        caller.pc = caller.pc.wrapping_add_signed(i32::from(offset));
        self.stack[base + iter as usize] = Value::EMPTY;
        self.stack[base + value as usize] = Value::UNDEFINED;
    }

    /// Makes the generator object for the frame that's running, with that frame copied into it.
    pub(crate) fn suspend_new_generator(&mut self) -> Value {
        let frame = self.frames.last_mut().expect("there's a frame running");
        let base = frame.base as usize;
        let top = frame.top as usize;
        let keeps_args = frame.flags & KEPT_ARGS != 0;
        frame.flags &= !KEPT_ARGS;
        let extra_args = if keeps_args {
            self.kept_args.pop()
        } else {
            None
        };
        let frame = self.frames.last().expect("there's a frame running");
        let data = GeneratorData {
            state: GeneratorState::Suspended,
            proto: frame.proto,
            closure: frame.closure,
            pc: frame.pc,
            resume_register: crate::generator::NOT_STARTED,
            registers: Box::default(),
            extra_args,
            new_target: frame.new_target,
            promise: Value::UNDEFINED,
            return_value: Value::UNDEFINED,
            queue: Vec::new(),
        };
        let registers: Box<[Value]> = self.stack[base..top].into();
        self.debt += registers.len() * 8;
        let proto = self.realm.generator_proto;
        let generator = self.new_object_with_proto(proto, Kind::Generator(Box::new(data)));
        if let Kind::Generator(data) = &mut self.objects[generator.index()].kind {
            data.registers = registers;
        }
        generator
    }

    pub(crate) fn suspend_generator(&mut self, resume_register: u16) {
        let frame = self.frames.last_mut().expect("there's a frame running");
        let base = frame.base as usize;
        let top = frame.top as usize;
        let pc = frame.pc;
        let keeps_args = frame.flags & KEPT_ARGS != 0;
        frame.flags &= !KEPT_ARGS;
        let extra_args = if keeps_args {
            self.kept_args.pop()
        } else {
            None
        };
        let generator = self.current_generator;
        if let Kind::Generator(data) = &mut self.objects[generator as usize].kind {
            data.pc = pc;
            data.resume_register = resume_register;
            data.registers.copy_from_slice(&self.stack[base..top]);
            data.extra_args = extra_args;
        }
    }

    /// Carries on after an error that's been thrown into the frame on top.
    pub(crate) fn throw_into_frame(&mut self, error: Value, stop: usize) -> Run<Value> {
        if self.unwind(error, stop) {
            self.run(stop)
        } else {
            Err(error)
        }
    }

    pub(crate) fn set_register(&mut self, offset: u16, value: Value) {
        let base = self.frames.last().expect("a frame was just made").base as usize;
        self.stack[base + offset as usize] = value;
    }
}
