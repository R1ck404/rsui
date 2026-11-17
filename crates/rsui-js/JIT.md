# rsui-js's JIT

A baseline JIT for x86-64 on Windows, in `src/jit/`, behind the `jit` cargo feature, which rsui-tsx turns on by
default (see "Built in by default"). A function that has run often
enough has each of its instructions turned into a fixed piece of machine code that works on the interpreter's own
frames, registers and NaN-boxed values. Every value is in the frame's registers in memory at every instruction
boundary (an instruction may also find the one before's result still in a machine register, below), so compiled code
can be entered and left at any instruction boundary, and anything it doesn't do itself it hands to the same slow-path
functions the interpreter uses, or to the interpreter itself. There's no deoptimizer and no second representation of
a frame.

It was built on the interpreter at `20feba6` (branch `t3/baseline-x64-jit`, `d2f300a`) and merged in round 4 onto
the interpreter with that round's four lanes (numbers, property access, allocation and GC, the call path); what the
lanes changed is mirrored here, as "Where it touches the interpreter" says.

This replaces the feasibility study of November 2026 (commit `9dccfec`, branch `t3/jit-feasibility-study`), whose
profiles and prototype it was built from. The study's main finding stands: most of the interpreter's cost on calls
and loops is dispatch and bookkeeping between instructions, which is what a template JIT removes.

## Turning it on

| | |
|---|---|
| `--features jit` (rsui-js; `rsui-js/jit` from another crate) | builds it in. Without it, nothing in this document exists in the binary, and the build is the interpreter as it is without the JIT. rsui-js on its own (its tests, `run.exe`, `test262.exe`) is built without it unless asked. |
| rsui-tsx's `jit` feature, on by default | is `rsui-js/jit`, so `rsui-tsx.exe`, its examples and its tests have the JIT, as does anything else built in the same cargo command as rsui-tsx (cargo gives rsui-js one set of features per build). `--no-default-features` builds rsui-tsx without it. |
| `RSUI_JIT=0` (or `off`) | interprets everything, with the feature built in, in a copy of the interpreter's loop with none of the JIT's hooks (`run_loop_plain`). This is how to turn it off in a shipped `rsui-tsx.exe`. |
| `RSUI_JIT=1` (the default with the feature) | compiles a function once it has been called or gone round a loop [`THRESHOLD`](src/jit/mod.rs) (3000) times in all, on a thread of its own (see "Compiling"). At 1000, an app's first interactions compiled twice as many functions, whose code cost more than it saved there, and took 1 MB more memory; the benchmarks measure the same at either. |
| `RSUI_JIT=eager` | compiles every function it can the first time it's called, for testing. |
| `RSUI_JIT_LOG=1` | a line for each function compiled or refused (and why), and when the engine goes: how many were compiled, bytes of code, entries from outside compiled code, bail-outs (split into instructions the generic helper doesn't do, which shouldn't happen, and calls deep in Rust's stack), frames left to the interpreter at a generator, which instructions went to the generic helper most, and which calls and property reads were made the long way, by what they called or read. |
| `RSUI_JIT_LOG=2` | the same, and each function's instructions as its job is taken, with where its frame was then, and which calls were compiled in line (see "Calls and properties the code knows"). |
| `RSUI_JIT_STRICT=1` | ends the process if the generic helper meets an instruction it doesn't do (`helpers::bail_at`), so that a Test262 run shows any as a crash. |
| `--features jit-opt` (rsui-js or rsui-tsx) | builds in the feedback for an optimizing tier (see "Feedback"), which turns on `jit`. Without it, compiled code is exactly as described here. |
| `RSUI_OPT=0` (or `off`) | with `jit-opt` built in, compiled code records nothing, and is the code it is without the feature. |
| `RSUI_OPT_LOG=1` | with `jit-opt`, a line for each function that's due for the optimizing tier, and what all the feedback came to when the engine goes. |

Other targets build with the feature and never compile anything (`mode()` is always `Off` there).

Binary size (`run.exe`, shipped settings, round 4): 2,739,712 bytes without the feature, 3,047,424 with it (+301 KB,
of which about 60 KB is the second copy of the interpreter's loop); the app benchmark's executable 5,377,024 and 5,683,200. The code
it generates is about 170 bytes per instruction compiled (invoices: 2,488 instructions, 0.41 MB); on the benchmarks
that's 0.01-0.10 MB. An engine reserves 64 MB of addresses for code (`CODE_LIMIT`) and commits it 64 KB at a time;
past the limit nothing more is compiled.

## What's compiled

Every function compiled to bytecode except async functions and async generators (which wait, which compiled code
doesn't know how to do yet). Generators are (see "Generators"). Functions with `try`, `catch` and `finally` are
compiled, and so is everything with `for of`, which has a handler to close the iterator. A function longer than 30,000
instructions isn't (`MAX_OPS`).

Each instruction is one of three kinds (see `Gen::op` in `src/jit/x64.rs`):

- **In machine code, with its unusual cases elsewhere.** Loads and moves; `+ - * / %` and the bitwise operators on
  ints, then two doubles in line, with mixed numbers, an int that overflows, `-0` and NaN out of the way; `(a + b) |
  0` (AddInt32) and dividing by a power of two (DivInt) on ints and doubles; `-x`, `++` and `--` on ints and doubles;
  comparisons and their jumps; `===`, and `while (a !== b)` (LoopStrictNe); truthiness of booleans, ints, objects,
  null and undefined; `typeof x == "..."` for every kind but telling a function from an object; loops (`Loop*`,
  `IncLoopLt*`, with a typed array's and `arguments`' length too); globals, cells and captured variables (a cell's
  write barrier by a helper, for what isn't a number); property reads, writes and adds at a place that knows the
  shape, in its first entry here and its other three out of the way, whether the instruction is GetProp or GetPropPoly
  (a place may have turned polymorphic after the function was compiled), and a string's or a number's, from its
  prototype; adding a property or an object literal's field inside a plain object; array, `arguments` and typed array
  elements (every kind, at `TypedData::quick`) and lengths; `for of` over an array, and `GetIterator`, `IterClose`;
  `Math.*` while it's the built-in function (`sqrt`, `abs`, `fround`, `min`, `max`, `imul` and, with SSE4.1, `floor`,
  `ceil`, `trunc`; the rest by `math_quick_apply`); `SloppyThis`, `CheckThis`, `CheckObjectCoercible`; calls and `new`
  of compiled functions, and `super(...)` of a compiled parent, with short callees compiled in line (see "Calls and
  properties the code knows"); `instanceof` of a function with the usual `Symbol.hasInstance`; returns.
- **A small helper that does only that**, called with what it needs in registers: making objects, arrays,
  closures and cells; pushing onto an array literal; a megamorphic read or write from what reads and writes remember;
  a string's length; `+` of what isn't two numbers (joining strings), Concat; comparing what isn't two numbers; `===`
  of strings; truthiness and `typeof` of what's not quick to tell; `%` of doubles (`fmod`); calling a native, whose
  function compiled code finds; any other InstanceOf, SuperGet, CreateArguments, ToNumber, SetElem by a name; a typed
  array's length; `for of` over a hand-written iterator; ArraySpread, Rest, and GetIterator of what isn't an array.
- **The generic helper**, `helpers::op`, which does the whole instruction again the way `run_loop` does. Anything
  without a template comes here, as does what the templates and small helpers leave (a getter, a miss in every
  entry of a cache), and calls that compiled code doesn't make directly go to `helpers::call`.

Nothing bails out to the interpreter in normal running. The only bail-outs are a call made when Rust's stack is
already deep (below), and an instruction the generic helper can't do, which would be a bug in the list above:
`Stats::unknown` counts those, the JIT's tests check there are none, and `RSUI_JIT_STRICT` makes any end the process.

One thing compiled code leaves to the interpreter on purpose: a `for of` over a generator with the built-in `next`
that isn't compiled yet. The interpreter carries such a generator on in place (`Vm::step_in_place`, `RESUMED_HERE`),
with nothing to start or stop for each value, and running it from Rust for each value cost more than interpreting the
whole loop. So `helpers::iter_next` has the interpreter carry that frame on from there to its end, marked
`INTERPRETED` (which keeps `jit_loop` and `run_frame` from going back into its code), and compiled code returns what
it returned. Not a generator's frame that a `for of` is carrying on in place itself, though: its `yield` would hand
its value to that loop and end the frame, which isn't its returning, so there the generator is carried on from Rust.
A generator that is compiled is carried on in its code (see "Generators").

## Machine state

Compiled code keeps eight machine registers, all ones Windows' calling convention has a function keep, so the Rust
helpers leave them alone:

| register | holds |
|---|---|
| `rbx` | the running frame's registers (`regs` in `run_loop`) |
| `r12` | the running frame (`frame`) |
| `r13` | the `Vm` |
| `r14` | the int tag, `0xFFF9 << 48`, to `or` into a 32-bit result |
| `r15` | the running function's caches (`ics`) |
| `rsi` | `undefined` |
| `rdi` | the list of objects (an object's address is its handle shifted by 6) |
| `rbp` | the list of cells |

Everything else is scratch. Values live in the frame's registers in memory at every instruction boundary, so the
collector sees all of them, and so does the interpreter if it takes over.

**The last result in a register.** A template may say it leaves its result in a scratch register too, on every way
it ends (`Gen::leaves`: arithmetic, bitwise operators, divides, loads, moves, `++`/`--`/`-x`, GetProp, GetElem); its
cold paths load it there before they rejoin. The next instruction's template then takes that operand from the register
rather than loading it again (`Gen::operand`), for its first loads only, and only if nothing jumps to it
(`Gen::targets`: jumps, handlers, and a loop's top, where the interpreter can come in). The value is still stored, so
nothing else changes; on this machine, which forwards a store to a load almost for free, it made little difference.

A compiled function's code starts with a four-word header, then its entry:

```
entry - 32   where a call that has made the frame itself comes in (the pre-body: `sub rsp, 40`, its caches, the body)
entry - 24   whether it's worth going into from the interpreter or Rust (see below)
entry - 16   where its body is: the first instruction, for a frame that's already made
entry - 8    where a `new` from compiled code comes in, or 0 if it isn't a constructor
entry        nop; the prologue for a call from compiled code, which makes the frame; then the body
```

Each function's slot (`Jit::slots`, beside the list of protos, as long as it and never moving) says what the JIT
knows: 0 is never to be compiled; `2n + 1` below `CODE_MIN` is `n` more calls and loop iterations before it is; at or
above `CODE_MIN` it's the address of the entry, plus one if it's worth entering from outside compiled code (so the
interpreter only has to look further when the low bit is set, and a call to `entry + 1` just skips the `nop`).

On the machine stack, a compiled function keeps 40 bytes (the 32 Windows has a caller leave, and 8 for alignment) and
its return address: 48 bytes per frame. The 8 at `rsp + 32` (`SPARE`) are nothing a helper or callee touches; a
callee compiled in line keeps where its captured variables are there.

## Getting in and out

**From the interpreter.** With the feature, the interpreter's loop is in two copies, both `run_loop_body` (which is
all of `run_loop` without the feature): `run_loop_plain`, with no hooks at all, for `RSUI_JIT=0`, and `run_loop_jit`,
with two `#[cfg(feature = "jit")]` hooks (`jit_count!`), which count a call or a loop's turn by taking 2 off the
function's slot in line while it's counting and not due, and only call out when the slot's low bit is set otherwise
(the function is due, or has code worth going into). Most of an app's functions are never called often enough to be
compiled, and calling out to count each call made an app's interactions 3-5% slower with nothing compiled at all. They
call out:

- after `enter!` makes a frame (`jit_enter`): compiles if it's due, and runs the function's body;
- after a jump back to the top of a loop (`jit_loop`): compiles if it's due, and enters the code at that
  instruction, which is how a long loop in code called once (a script's top level) gets compiled. Code can be gone
  into where nothing is known of the machine's registers (`Gen::targets`: a loop's top, a handler, where a generator
  carries on), which `Compiled::entries` says for each instruction.

**From Rust.** Every way Rust runs a frame at its function's start (`Vm::call`, `Vm::run`, a native's callback, a call
compiled code leaves to `helpers::call`) goes through `run_frame`, which counts the call, compiles the function when
it's due, and goes straight into its code, without starting the interpreter at all. (This used to be a hook at the
start of `run_loop`, which left an interpreter frame under the compiled code; in a debug build, where that frame is
large, eager compiling ran Rust's stack out in rsui-tsx's tests.) A frame partway through a function with code (a
generator carrying on, an error thrown to a handler) goes into it there (`jit_resume`), unless it's `INTERPRETED`.

The trampoline is given the engine, the frame and where to start, in registers, and works out the rest itself from
the frame and the engine's lists (which never move): the frame's registers and caches, and the stack's floor. So
going in is one call with nothing written down first, which costs about what starting the interpreter's loop does
(that pushes as many registers, and sets up as much). A function that's short (under 10 instructions), with no loop and
no call, isn't worth going into from the interpreter, which is already running, so the interpreter keeps running it
when it's called from the interpreter or Rust, and compiled code still calls it directly. That's what array callbacks
like `(x) => x * 2` are; going into every compiled function from Rust made react_render 1% faster and
array_functional 3% slower.

A call compiled code makes the long way (`helpers::call`) to a function that isn't compiled yet starts the
interpreter for it afresh, which costs about twenty interpreted instructions, so such a call counts for twenty
(`CALLED_WEIGHT`, `Jit::count_called`): a function compiled code calls is likely called as often as its caller is. In
invoices, compiled functions did that 2,700 times in the first rounds of use, and this cut it to 1,000.

**Calls between compiled functions** are native `call`s: the call site checks the callee is an everyday function
(not a class, not keeping its `arguments`) with code, moves `rbx` and `r12` to the new frame, and calls the entry
with the new base, the argument count, the closure, its captured variables and where the result goes in
registers. The prologue checks there's room for the frame and its registers (a helper makes room, or throws the
`RangeError` with the caller's frame on top), writes the frame as `enter!` does, and makes the parameters with no
argument and the function's own variables undefined. A `new` of a constructor with code makes its `this` with a
helper (`create_this`) and comes in at the constructor's own entry, which sets the frame's flags and `new.target`;
a derived class's `this` is left empty for its `super(...)`, which calls the parent's construct entry the same way
(the parent found through the closure's shape, with the frame's `new.target`). A call whose callee is known is made
differently, below.

**Leaving.** Compiled code returns a status in `edx` and a value in `rax`:

- `OK`: the function returned. Its frame is gone and the value is for the caller.
- `THROW`: an error, in `rax`. A function with a handler around where it happened (its frame's `pc`) takes it
  (`helpers::catch`, which does what `Vm::unwind` does for that one frame) and carries on in machine code at the
  handler; else it hands the same status up. The frames of the compiled callees it went through are still on the
  frame list, and whoever takes the error first pops them, as `unwind` does.
- `BAIL`: the frame on top is to be carried on with by the interpreter from its `pc`. Every compiled caller hands
  it up unchanged, so the interpreter gets every frame back, and when the one on top returns, `finish_return` writes
  its value where its `ret_dst` says and the caller carries on in the interpreter (until its next loop iteration
  enters its code again).

Every compiled function stores its frame's `pc` before anything that can throw, call or collect, so `Vm::unwind`,
stack traces and the collector's view of where each frame is are the same as the interpreter's.

**Rust's stack.** Compiled code uses Rust's stack for calls (48 bytes a frame) where the interpreter doesn't. Each
time compiled code is entered, `Jit::floor` is set to half of the engine's stack limit, and a call from compiled code
when the stack is below it leaves the call to the interpreter (a `BAIL` at the call), which makes it without using
Rust's stack. Code isn't entered below the floor either. So deep recursion runs in compiled code for a few thousand
frames, then in the interpreter, and hits `MAX_FRAMES` exactly where the interpreter would. The helpers that call
back into JavaScript use Rust's stack as the interpreter's natives do.

## Compiling

With `RSUI_JIT=1`, a function that's due is compiled on a thread of its own (`jit::Worker`), started the first time
an engine has something to compile. The engine takes a copy of everything compiling needs (`x64::Job`, below) and
carries on interpreting the function, whose slot stays due; the next time it's called (or loops), the engine looks for
what the thread has finished (`jit_collect`), puts it in place, and goes into its code. So compiling (10-100 us a
function, 0.6 ms in all over invoices' first interactions) doesn't hold up what the engine is doing, which in an app is
what someone's waiting for. The thread writes the code (it has the `x64::Writer`: the code memory, its layout, and
what it has written); the engine only sets the slot, after a CPUID, as Intel's manual asks before running code another
processor wrote, and new code starts on a cache line of its own.

**What a job has.** The compiler is written for a copy of the engine taken when the function is due, never for the
engine itself, and compiled code is written for what the engine knows at that moment (see "Calls and properties the
code knows"). `Job::new` takes, on the engine's thread:

- the function's instructions, the proto's fields, and the constants its instructions name (`constant_of`);
- the cache entries of its property places (`read_place`: GetProp, SetProp, GetGlobal, CallMethod and their
  polymorphic forms, with a polymorphic place's other entries), and the shapes they name that the code may compare
  with as constants (`Job::plain`, none a dictionary's), with the engine's epoch and global object;
- what each call is likely to call (`x64::Look`, which does the guessing on the engine's thread, from the caches, the
  globals, the captured variables and the frame on top), and for each such callee its proto's fields, its slot, and its
  instructions if it's short enough to be compiled in line, with their caches and constants too;
- the registers of the frame it's being compiled for, if that's the frame on top, and which loop that frame is in
  (for `looks_double`);
- where the engine's lists are, which the code points into.

The shapes in `Job::plain` are pinned as the job is taken (`Shapes::pin`). A collection can run before the code is in
place and free one of them; pinned, its number isn't given to another shape, which the code would take for it, and as
no object has it any more, the code's compare with it just fails. Pinning when the code is put in place would be too
late: the number could have been given out again by then. This pins every plain shape the job's places name, a few
more than the code ends up comparing with (an add's place, a holder of another epoch), which costs a freed shape's
empty place each.

The writer keeps what it knows of each function it has written code for (`Known`: where the code is, where each
instruction starts, and what each takes in a register), so code it writes later can call those directly and go on in
them from a call compiled in line. The engine keeps its own list (`Compiled`), with what it needs to go into the code
and to compile it again.

**Callees ahead of their caller.** A function's jobs start with those of the callees worth compiling just before it
(`Look::inline_candidates`, `Vm::jit_jobs`): the short ones its calls could compile in line, which needs their code
(see "Compiled in line"), and those more than halfway to being due, which its calls then call directly. The worker
compiles them in that order. Their slots go on counting as before, and such a function is given its code only when
it's due (`jit_install`, or `jit_compile` finding it in the list).

**Compiled again.** A function compiled before the calls in it knew what they'd call is compiled again once they
would, with what its caches know by then, three times more at most (see "Compiled again", below). What decides that
runs on the engine's thread, at the call (`helpers::recompile`, `Vm::jit_recompile`). With functions compiled there and
then (`RSUI_JIT=eager`, the tests), the new code is written at once and the running code goes on in it at that call.
With the worker, the job goes to it and the running code carries on in the old code, those calls made the way that
works for any callee, which is only slower; the count is set to look again after 64 more of them (`RECOMPILE_POLL`),
and once the new code is there, the frame running the old code goes on in the new at that call. Anything that enters
the function from then on enters the new code. Waiting for the worker instead would hold the engine up for a compile
behind whatever else the worker has queued.

Nothing ever waits for that thread. A job has all it needs, nothing of the engine is read while compiling, and the
code memory is kept by whoever still holds it (`memory::Views`), so when the engine goes, the thread finishes what it's
doing and ends. Waiting for it there hung: an app's engine goes as its thread ends, in a thread-local's destructor,
where Windows won't let a thread start or end until it's done. `RSUI_JIT=eager` compiles on the engine's thread, as
do the tests' runs with `Jit::background` off, so that they run the same way each time.

## Generators

A generator's code is compiled like any function's (async ones aren't). `InitialYield` and `Yield` are helpers
(`initial_yield`, `yield_value`) that do what the interpreter does (make the generator; save the frame into it, and
take the frame away) and leave what's to be handed back in `vm.jit.value`, which the template returns as `ret` does.
The instruction after each is somewhere the code can be gone into, and a generator is carried on there:

- **from Rust** (`next()`, spreading, destructuring, `Array.from`, a `for of` the interpreter runs over a compiled
  generator, which no longer carries it on in place): `resume_generator` makes the frame and `run_frame` goes into the
  code at its `pc` (`jit_resume`); the `yield` returns to the trampoline;
- **from a compiled `for of`**, directly: `helpers::iter_next` puts the generator's frame on top as the interpreter does
  to carry it on in place (`RESUMED_HERE`, its `yield` going to the loop's register; `resume_in_code`) and says
  `RESUME` with where its code goes on; the loop's code sets up the frame's registers and caches and calls there, as a
  compiled call does. Its `yield` returns what it yields; its end (`finish_return` saying `LEFT`) has put the loop's end
  in place, and the loop jumps out; anything else is handed up as a call's. So a pipeline of generators runs in
  compiled code with nothing started or stopped for each value but the generator's frame, whose registers are copied in
  and out each time, as the interpreter does.

A generator the interpreter is carrying on in place when it's compiled goes on in its code at the top of its next
loop (`jit_loop`); its `yield` then does what `yield_here` does, and its end what `end_resumed_generator` does, which
leaves the interpreter to carry on with the loop's frame (`LEFT`).

## Guards and caches

Every type check is done each time: a value's tag before treating it as an int, a double or an object, `jo` after
int arithmetic, a zero result of `*` (which could be `-0`) going the double way, `-0` and NaN when a double becomes
an int again (`Value::number`), the `OLD` bit before writing to an object or array in place (which goes through the
write barrier's slow way otherwise), `FROZEN_ELEMENTS`, holes.

Caches are read as data, and what an entry knew when the code was compiled is also written into it (below, "Fixed
shapes"): a property read compares the object's shape with the cache entry's shape in memory, then reads the slot the
entry says, from the object or (if the entry's epoch is the engine's)
from the object it names: a prototype, or `Vm::absent` for a property that isn't there, whose one value is undefined.
When the interpreter's slow path learns another shape, compiled code sees it the next time. `Vm::learn` rewriting an
instruction to its polymorphic form doesn't change the compiled template, so a monomorphic one (GetProp, SetProp,
CallMethod) looks at the place's other entries out of the way, and once it's megamorphic, at what reads and writes
remember. A string's or a number's read is remembered for its prototype's shape (on that prototype, further up, or
nowhere), among the place's entries like any shape, as `get_from_primitive_proto` learns it. Shapes and handles the
collector frees are cleared from the caches by the collector, as before.

A typed array's elements are read and written where `TypedData::quick` says, while its `epoch` is the engine's
`buffer_epoch` (a buffer that's resized or transferred moves it); anything else goes to the helpers that work them out
again. `Math.*` is worked out in compiled code while the callee is the native `math_quick_native` says (one compare of
its function), as `Vm::math_quick` checks; else it's the call, which turns the place into a Call for the interpreter.

Where Rust lays something out as it likes, the layout is found at run time from real values
(`object::object_layout`): which byte of an object says what kind it is (with the feature, `Kind` is `repr(u8)`, which
keeps it 24 bytes), where a function's proto and captured variables are, an array's elements and length, where a
heap of property values keeps its pointer. Frame, proto, cache and `Vm` fields are found with `offset_of!`. The lists
compiled code points into (objects, cells, stack, frames, caches, protos, slots) are `Arena`s, which never move, so
their addresses go in the code as constants; anything that can change (lengths, the debt, the epoch) is read from
the `Vm` each time.

## Collecting garbage

Compiled code reaches the same safe points as the interpreter (calls, `new`, loops' back jumps), with the same test
of the debt against the threshold, and calls `helpers::collect` with the frame's `pc` saved and the top of the stack
set, as `safe_point!` does. Values are always in the registers in memory then (a template's result in a machine
register is a copy), and objects don't move, so there's nothing else to tell the collector. Helpers that allocate do
so as `run_loop` does (`alloc_for_jit`, a twin of `alloc_unrooted`, for what goes straight into a register; with the
helpers calling `alloc_unrooted` itself, LLVM stopped inlining it into the interpreter's loop), and only collect at
safe points.

## Executable memory

`memory.rs` maps one section of the page file twice: a read-write view, which code is written through, and a
read-execute view, which it runs from, so no address is ever writable and executable at once, and adding a function
changes no page's protection. (Making pages writable and then executable again took two `VirtualProtect` calls a
function, each of which has every core forget what it knew of those pages: a third of what compiling a function cost.)
The section's addresses are reserved once (`CODE_LIMIT`) and its memory committed 64 KB at a time, then
`FlushInstructionCache`. A function's code is written at an address known before it's generated, on a cache line of
its own. Nothing in the code is changed after it's written; a function compiled again gets new code, and its old code
stays for the frames running it.

Compiled code has no unwind information registered, so the native sampler (`RtlVirtualUnwind`) can't walk through
it: samples land on `?!?` and the frames above them are lost. A panic can't unwind through it either, which is why
every helper is `extern "C"` (a panic in one aborts). A profiler built with the feature says which compiled function
and instruction each sample was in, or under (in a helper it called), from `Engine::jit_code`, which knows where each
instruction's code and cold paths start; `RSUI_PROF_US=50` samples often enough for one bench.

## Calls and properties the code knows

Round 5 made compiled code use what the caches and the running frame know when a function is due, rather than
only reading them each time. All of it is taken into the function's job as it's due (see "Compiling"). Every guess is
checked each time it's used, and the way that works for anything is kept behind it, out of the way, so a wrong guess
costs time, never correctness.

**Known callees** (`Look::likely_callees`, `known_callee`, `guess`; `Gen::known_callees`). A call's callee is guessed
from the instruction that put it in its register: a method a read finds on a prototype (its cache's holder and slot),
a global, a captured variable or a register of the frame being compiled for (a function is due while it runs, so the
frame on top is its own), or the function itself. A polymorphic method read gives each method its entries found (up
to four). The guessing is done on the engine's thread as the job is taken; the writer keeps those of the guesses that
have code by then (or are the function itself). The call compares the callee's proto with each, and for one that's an
everyday function makes its frame at the call with what's known of it (`call_known`: no prologue, no argument count
compared) and calls its pre-body, at an address known as it's compiled. Anything else is `call_any`, the call that
works for any callee.

**Compiled in line** (`inline_call`). A known callee of at most 40 instructions, all ones whose templates need
nothing of a frame but its registers (`inlinable`: arithmetic, comparisons and forward jumps, property reads and
writes, globals and captured variables, `Math`), with no loop, call or handler, is compiled in line: its
instructions run on its registers where its frame would have them (from `f + 1`), its caches reached from the
caller's by a constant distance (`ics_delta`), with no frame made, no stack check and no safe point. Wherever one of
them would go to a helper that needs the frame, or the generic way (`deopt`), the code makes the frame then, as the
call would have, and goes on in the callee's own compiled code at that instruction, which does it from its start
(every template only changes anything once it has taken its usual way); `Known::incoming` says what that code
takes in a register there. So the callee has to have been compiled itself: a function's short callees are compiled
just before it (see "Callees ahead of their caller"). `RSUI_JIT_LOG` counts the calls compiled in line and how often
they went on in the callee.

**Compiled again** (`Vm::jit_recompile`). A call compiled with nothing to go on (a script's second loop when its first
is compiled, say) counts down its function's count (`Compiled::counter`, whose address is in the code); at 0 (after
1000 such calls) the function is compiled once more if one of those calls would know what it calls now, with what its
caches and frame know by then, and the code goes on in the new code at that instruction (a call's template takes
nothing from a register, so any call can be gone into). Else the count starts again, twice as long each time. A
function is compiled four times at most, and one longer than 400 instructions only once (`MOST_COMPILES`,
`RECOMPILE_MOST`). On the worker this is asynchronous (see "Compiling").

**Fixed shapes** (`fixed_read`, `fixed_write`, `fixed_poly`). A read, or a write of a property the object has,
whose place's entries know a shape (not a dictionary's) compares the object's shape with that shape as a constant,
and reads or writes the slot it says, also a constant: for a read on a prototype, the holder's address too, while the
engine's epoch is what it was then. Polymorphic places compare with each shape they knew; globals with the global
object's. Anything else goes the cache's way, as before (an add does too, so that code never gives an object a shape
that's been freed). A freed shape's number could be given to another, which the code would take for it, so the shapes
compiled code may name are pinned as its job is taken (`Job::plain`, `Shapes::pin`): the collector frees them as ever,
with what only they kept, but doesn't give their numbers out again, and then no object has them. What it costs is a
freed shape's empty place each, for as long as the engine runs, and no more: the end of the list of shapes is given
back past pinned places too, and a pinned number comes back as an empty place if the list grows that far again
(`Shapes::place`). Holding the list at the highest pinned number instead kept 1,380 places for 636 shapes in
`prototypes_and_shapes_made_by_each_call_go_with_what_used_them` with `RSUI_JIT=1`, which makes a class and a
prototype each call.

**Doubles first** (`looks_double`). When a function is due while its frame is going round a loop, the registers of
that loop's instructions hold what they worked on the last time round (the job has a copy, `Job::live`, and the loop,
`Job::looping`): arithmetic whose operands were both doubles there tests for two doubles first, in line, with ints out
of the way, and comparisons of doubles compare as doubles in line. Only that loop's: the registers of a loop that hasn't
run yet in that frame hold whatever another frame left there, and trusting those made typed_arrays' int loops 3.5
times as slow. A function with a loop that's due as it's called, whose frame says nothing yet, is compiled at its
loop's next turn instead (`Vm::jit_compile`, with `RSUI_JIT=1`; `RSUI_JIT_LOG=2` says where each job was taken): with
the threshold at 3000, float_math's `advance` was due as it was called, and its code took 15% longer without this.

**Constructors**. A write whose place adds a property (`this.x = x`) does that in line, and a constructor (a class's,
or a function that adds to its `this`) returns from a `new` in line. `instanceof` reads `Vm::instance_of_cache` and
walks the prototypes through the shapes' list (whose address a `Vec` keeps where `vec_pointer` finds it).

## Adding an instruction

1. If it's a new instruction for the interpreter, add it to `helpers::op` first, the way `run_loop` does it, calling
   the same `Vm` functions. That alone makes compiled code correct for it (the template is the generic call).
2. For speed, give it a template in `Gen::op`: do the usual case in machine code and jump to `cold_generic` (or
   `generic_jump` for a jump) for everything else. The generic way does the whole instruction from the start, so a
   template must not change anything before it decides to take it. Use `r10`/`r11` as scratch after the guard
   helpers, which use them. An unusual case that's common (a string, a native) is better as a small helper of its own
   (`direct`, `cold_direct`), which skips the generic helper's work of finding which instruction it is.
3. Anything that can throw, call JavaScript or allocate: call a helper, with the frame's `pc` saved (helpers given
   the `pc` save it themselves, with `here`). After it, `test eax, eax; jnz exit`.
4. A jump's offset has to be in `jump_offset`, so that `extent` and the checks in `compilable` see it.
5. If its template's first loads are of operands, take them with `operand` (see "Machine state"); if it leaves its
   result in a register on every way it ends, say so with `leaves`, and rejoin from cold paths with
   `cold_generic_leaving`.
6. If the interpreter rewrites the instruction in place (as SetElem and SetElemTyped, CallMath and Call, GetProp and
   GetPropPoly), the helpers read whatever is there when they run, so they have to do each form, and the template has
   to stay right for all of them.
7. Test it in `src/jit/tests.rs`, which runs each program interpreted, compiled at once and compiled once hot, needs
   all three to print the same, and checks nothing went to `bail_at`; run those with `RSUI_GC_STRESS=1
   RSUI_GC_VERIFY=1` too.

What an instruction means is written twice now: in `run_loop` and in `helpers::op` (plus the templates' fast paths).
Changing one means changing the other; the JIT tests and Test262 with `RSUI_JIT=eager` and `RSUI_JIT_STRICT=1` are
what catch a difference.

## Feedback

With the `jit-opt` feature, compiled code records what it sees, for an optimizing tier to compile
from: `src/jit/feedback.rs`. Nothing reads it yet but `RSUI_OPT_LOG`. The interpreter records nothing, so a function
that's never compiled pays nothing, and with `RSUI_OPT=0` compiled code is byte for byte what it is without the feature.

- **Where.** Each compiled function has a block, laid out from its instructions alone (`feedback::layout`): an 8-byte
  header, its count to being due, then a record for each instruction `feedback::kind_of` names. Code compiled again
  uses the same block. Blocks are in 8 MB of addresses set aside just after the view code runs from, committed 64 KB
  at a time, so that code writes a byte of one with a single instruction that names it by its distance (`asm::rip_*`,
  7 bytes): `memory::map_with_room` reserves room for both, gives it back and maps the view there (the room next to a
  view the system placed was often taken, as views of code are packed side by side), and `reserve_near` looks within
  a gigabyte if that fails. If there's no room near enough, nothing is recorded.
- **Plain stores.** What the code records in hot paths is a flag in a byte of its own that it stores 1 in, never an
  `or` into bits: an `or` reads the byte first, so each run of a loop waits for the last one's store.
  `feedback::logical` reads a record as if its bits were in one place.
- **Numbers** (arithmetic and comparisons): the code's byte is `DOUBLE`, stored where a template leaves its ints' way
  (the label its int guards and its overflow check go to), so it means "not two int32s with an int32 result", as V8's
  "number" feedback does; the next byte is the helper's, `STRING` and `OTHER` (and `DOUBLE`) from the operands.
- **Branches** that jump forward: two flags, one stored on the way on and one on a stub the jump goes through out of
  the way (`Gen::branch_target`, `branch_went`).
- **Calls**: a callee the call knew and found stores its own flag (`Gen::note_known_callee`; which callee each flag is
  the writer keeps for the function, so it's the same in all its code, and `Compiled::feedback_knowns` says); a
  compiled function `call_any` calls is compared with the record's word in line, and anything new goes to a routine
  that makes it that callee or `MANY` (`note_word`); `helpers::call` and `call_native` record the rest (a native by its
  handle, with `NATIVE`; a bound function, a proxy or a class as `OTHER_CALLEE`). `feedback::plain_call` puts the two
  together.
- **`new`**: what was called and the shape of the `this` made (`helpers::this_for`).
- **Elements**: holes, past the end, typed arrays and their kind (in machine code, where `typed_place` finds the kind),
  keys that are names and which, other objects, primitives, odd keys (by the helpers).
- **How hot.** Each function counts down from 10,000 at its entry and each loop's jump back; at 0, a routine calls
  `helpers::opt_due`, which has it wait until its records haven't changed over 1,000 more, three times at most, then
  counts it as due.
- **How a helper finds its record**: from the frame on top and the instruction (`feedback::site_of`), with the last
  few functions' blocks kept by their protos' low bits. `helpers::typed_set`, which isn't told its instruction, is
  given the record by the code first (`Gen::set_site`, `Jit::site`). Once a function is due, its helpers record no
  more: what it's to be compiled from is there.
- **What it costs**: the JS benchmarks 3.2% slower than the baseline without the feature, most
  on the benchmarks whose time is in helpers (natives, keys that are names, calls of functions without code); in the
  apps, invoices' editing a field 2–4% and typing in search 1–2.5%, the rest within the app benchmark's noise.
- **Code size.** The two routines (count, call word) are written once, with the trampoline (`feedback_routines`).
  Over all of the JS benchmarks compiled eagerly, feedback makes the code 2.41% bigger (invoices' and web_dashboard's 2.6%); a stub written while a template is in its
  cold code already is jumped over (`Gen::out_of_line`).
- **What's left out.** A callee compiled in line records nothing; where it can't go on in line, its own code does.

## Built in by default

Since branch `jit-default` (on `3d09d955`), rsui-tsx's `jit` feature is on by default and turns on
`rsui-js/jit`, so `rsui-tsx.exe` ships with the JIT at `RSUI_JIT=1`. rsui-js on its own is as it was: its tests,
`run.exe` and `test262.exe` are the interpreter unless built with `--features jit`. `RSUI_JIT=0` turns it off in a
shipped `rsui-tsx.exe`, and `--no-default-features` builds rsui-tsx without it. In a cargo command that builds rsui-tsx
with other crates (`cargo test -p rsui-tsx -p rsui-js ...`), cargo gives rsui-js one set of features, so they all have
it.

One change to the JIT came with it: SetHome has a helper of its own (`helpers::set_home`) instead of the generic one.
Every named method of an object literal has one, and chart.js ran 275,000 of them the generic way in
the canvas benchmark.

Before is `3d09d955` without the JIT, and after is this branch. Both are release builds with the shipped settings, in
their own target dirs, and were run alternately.

**Size**

| | Before | After |
|---|---:|---:|
| `rsui-tsx.exe` | 8,379,392 B | 8,710,656 B (+323.5 KiB) |
| App benchmark executable | 7,327,232 B | 7,659,008 B |
| `run.exe` (`--features jit`) | 3,226,624 B | 3,559,936 B |

**Startup and peak memory** (15 runs: `--png` of each app, medians from the repo and from
`%TEMP%`, the builds run in the order before, after, after, before):

| | Before | After |
|---|---|---|
| invoices | 79.0–93.4 ms, 28.7–28.9 MB | 78.1–94.4 ms, 28.9–29.0 MB |
| web_dashboard | 124.6–126.6 ms, 36.1–36.2 MB | 124.9–125.7 ms, 36.4–36.6 MB |
| notes | 93.1–93.8 ms, 30.0–31.1 MB | 79.7–94.0 ms, 28.4–31.1 MB |

Both builds land at about 78 or 93 ms on the native apps, as before. An earlier round, with the build before SetHome's
helper, gave invoices 77.9–78.6 against 78.2–79.1 ms and web_dashboard 124.1–126.1 against 124.6–126.2 ms; with
`RSUI_JIT=0`, web_dashboard was 125.3–126.7 ms. invoices and notes compile nothing before their first frame. Of
web_dashboard's functions, 10 are compiled before its first frame (0.12 MB of code committed), which is its extra
0.3–0.5 MB.

**Engine** (ms, best of 20 runs; `run.exe` without and with `--features jit`):

| bench | Before | After | After/Before | | bench | Before | After | After/Before |
|---|---:|---:|---:|---|---|---:|---:|---:|
| method_calls | 99.6 | 36.4 | **0.365** | | spread_destructure | 120.1 | 101.1 | 0.841 |
| richards | 149.6 | 57.1 | **0.382** | | call_apply | 117.8 | 100.2 | 0.851 |
| float_math | 186.2 | 93.3 | **0.501** | | map_set | 76.6 | 70.7 | 0.923 |
| closures | 85.3 | 44.2 | **0.518** | | buffer | 26.1 | 25.4 | 0.973 |
| class_proto | 71.5 | 37.8 | **0.529** | | array_sort | 92.9 | 91.3 | 0.983 |
| prop_mono | 72.4 | 39.2 | **0.541** | | json | 80.2 | 78.9 | 0.984 |
| fib | 80.0 | 44.3 | **0.554** | | string_build | 97.2 | 95.7 | 0.984 |
| raytrace | 82.1 | 48.1 | 0.586 | | builtin_misc | 95.2 | 94.0 | 0.987 |
| int_loop | 134.2 | 80.1 | 0.597 | | empty | 13.2 | 13.1 | 0.988 |
| typed_arrays | 88.4 | 56.4 | 0.638 | | crypto-pbkdf2 | 32.6 | 32.5 | 0.996 |
| alloc_churn | 121.6 | 80.9 | 0.666 | | zlib | 31.7 | 31.7 | 0.999 |
| try_catch | 70.7 | 50.6 | 0.715 | | react_render | 110.2 | 110.1 | 1.000 |
| array_ops | 87.8 | 63.0 | 0.717 | | object_keys | 109.6 | 110.2 | 1.005 |
| prop_poly | 79.8 | 59.5 | 0.746 | | regex | 46.0 | 46.3 | 1.005 |
| | | | | | async_await | 74.5 | 75.0 | 1.006 |
| | | | | | crypto-hash | 16.3 | 16.5 | 1.013 |
| | | | | | string_ops | 81.0 | 82.4 | 1.016 |
| | | | | | generators | 121.4 | 124.1 | 1.022 |
| | | | | | array_functional | 91.1 | 95.4 | 1.047 |

Geomean **0.768**. Without startup, the 33 benches took 2,406 ms before and 1,752 ms after. The peak working set's
geomean is 1.059, about 0.3–0.5 MB a bench: the bigger program, the code, and the thread that compiles.

array_functional and generators are the two slower by more than 2%. Eight of the flat benches run alternately four
times more (20 rounds each), twice with `RSUI_JIT=1` and twice with `RSUI_JIT=0`, gave:

- array_functional: 1.023 and 1.033 with `RSUI_JIT=1`, and 1.025 and 1.041 with `RSUI_JIT=0`;
- generators: 1.000 and 1.027 with `RSUI_JIT=1`, and 1.043 and 1.023 with `RSUI_JIT=0`;
- the other six (async_await, builtin_misc, json, regex, string_ops, zlib): within 2% with the JIT on (geomeans 1.007
  and 1.006 for all eight).

So array_functional's cost comes from what the feature builds, not from compiling. It's the same with the JIT off,
and its callbacks are too short to be gone into from Rust (see "Getting in and out"). It's the shared interpreter and
call path compiled differently in the build with the feature, which "How it measures" found in rounds 4 and 5 too.

**Apps** (the app benchmark before and after, 9 runs; best / median, after/before):

| app, what | After/Before |
|---|---:|
| invoices, editing a field | 0.985 / 1.023 |
| invoices, selecting an invoice | 1.022 / 1.032 |
| invoices, typing in search | 1.012 / 1.004 |
| invoices, hover | 1.004 / 1.000 |
| gallery, adding / hover / scrolling the list | 0.979 / 1.023, 1.000 / 1.000, 1.000 / 0.995 |
| tasks, notes, files, hover | 1.000 / 1.000, 1.000 / 1.000, 1.02 / 1.02 |
| expenses, hover | 1.068 / 1.067 |
| invoices, peak working set | 11.33 to 12.05 MB |

Each app's load, mount and first frame are within 4% either way, apart from tasks' mount (0.46 against 0.51 ms).
Earlier runs gave expenses' hover 0.953 / 0.980, and tasks' 1.064 / 1.062 then 0.974 / 0.974 in 15 runs, so those rows
are the app benchmark's noise. Selecting an invoice was 2–4% slower in every run, and also with `RSUI_JIT=0` (1.024 / 1.017 in
15 runs). That's the same build effect as array_functional, and the same as round 5's 1.028 / 1.019.

**Editing** (the editing benchmark with 2000 paragraphs, 3 rounds, ms, layout after the key, best):

| | Before | After | Edge |
|---|---:|---:|---:|
| TipTap Enter | 6.22 | 5.22 | 3.76 |
| TipTap key typed | 3.19 | 2.89 | 2.92 |
| TipTap Shift+Down | 2.31 | 1.80 | 1.58 |
| TipTap Ctrl+A | 2.14 | 1.91 | 4.12 |
| TipTap peak working set, private bytes | 65.5, 57.3 MB | 73.1, 58.7 MB | |
| Lexical Enter | 6.50 | 4.90 | 3.57–4.23 |
| Lexical key typed | 3.75 | 2.84 | 2.95–3.04 |
| Lexical Shift+Down | 1.94 | 1.66 | 1.61 |
| Lexical Ctrl+A | 8.98 | 8.27 | 4.24–4.38 |
| Lexical peak working set, private bytes | 163.9, 162.4 MB | 172.5, 161.8 MB | |

Ready (page start to editor there) is 158–172 ms either way. TipTap compiles 265 functions there (14,132
instructions, 4.37 MB of code) and Lexical 270 (11,613 instructions, 3.51 MB): about 310 bytes an instruction. The
peak working set counts that code twice, as it's mapped twice (see "Executable memory"), and private bytes don't
count it, so the 8 MB more working set is about 4 MB of memory. Making the code smaller is the first thing under
"What's next".

**Canvas** (the canvas benchmark, 5 rounds, ms, best, rsui only):

| | Before | After |
|---|---:|---:|
| 5000 filled circles | 9.63 | 8.99 |
| 5000 stroked lines | 10.58 | 10.05 |
| 10000 fillRect | 4.35 | 3.48 |
| 2000 fillText | 1.87 | 1.65 |
| 2000 drawImage sprites, scaled | 2.16, 11.72 | 1.88, 11.40 |
| getImageData+putImageData | 12,035 MB/s | 11,929 MB/s |
| chart.js line, 1000 points: one frame, `update('none')` | 3.30, 12.60 | 3.05, 12.15 |
| chart.js bar, 200 bars: one frame, `update('none')` | 1.34, 6.43 | 1.22, 6.54 |

chart.js's bar update is the one row slower. It was 6.62–6.65 ms against 6.28–6.37 before SetHome's helper, and
6.40 with `RSUI_JIT=0`. With `RSUI_JIT_LOG=1`, 406,000 of its reads are of proxies (chart.js's options), which
compiled code reads through the generic helper. That calls the trap from Rust, where the interpreter has called it in
place (see "Proxy traps and getters called in place" under "What's next"). Before SetHome's helper the
line update was 12.65–12.69 ms.

**Checks**, all with the JIT built in:

| Check | Result |
|---|---|
| `cargo test -p rsui-tsx -p rsui-js -p rsui-text -p rsui-render -p rsui-layout --no-fail-fast` | 1,471 passed, none failed, 8 ignored, against 1,441 without the feature: the other 30 are the tests built only with it, one of them new (`methods_of_object_literals_find_super_through_their_home`). rsui-js has the feature here (`cargo test -p rsui-js --features jit` afterwards built nothing) |
| `cargo test -p rsui-js --features jit`, `RSUI_JIT=eager RSUI_JIT_STRICT=1` | 405 passed, 1 failed, 1 ignored. The failure is `compiled::tests::a_new_version_of_the_same_code_uses_the_room_the_last_one_left`, and `3d09d955` fails it in the same way (see below) |
| `cargo check -p rsui --tests` | clean, no warnings |
| Test262 (2 shards), default and `RSUI_JIT=eager RSUI_JIT_STRICT=1` | 38,414 of 48,794 pass in both, and every test's outcome is the same as before (the interpreter's, and its eager run's), apart from `built-ins/Array/S15.4_A1.1_T10.js`. That one failed before and crashes here, as the runs were in a job that caps a process at 1.5 GB, and the test makes the engine ask for 1 GiB before its RangeError. The interpreter's `test262.exe` from `3d09d955` crashes the same way in that job |
| Web libraries | 37 of 37 in Edge and in rsui |
| Editors | TipTap, Lexical and Slate 19/19 in `basic` and 12/12 in `more` |
| Canvas | `tests` 19/19, chart.js 6/6, signature_pad 1/1, with a table of differences identical to the interpreter's |
| CSS pages | bootstrap 116/116 within 1px, bulma 73, landing 62, pico 52 and 23 within 4px, tailwind 72, antd 100, chakra 60, mantine 78, mui 77 and 1 within 4px, radix-themes 54, shadcn 57: the same as before |
| `rsui-tsx --png` of each of the 10 `.tsx` examples | byte for byte the same PNG as before, with `RSUI_JIT=1`, `eager` (and `RSUI_JIT_STRICT=1`) and `0` |

The failing test saves a new version of a script 40 times and checks the lists of compiled code don't grow. With
`eager`, every function is compiled, and a function the JIT has compiled keeps its number (`CodeRoom::named`), as
machine code may compare a callee with it, and keeps its machine code too (`Jit::forget`). So the list of protos grows
by the script's four functions a version (to 160, against 40), while the code and caches go back as they should. With
`RSUI_JIT=1` nothing in the test gets hot, and it passes. The same is true of an app being edited while it runs: each
function that was hot enough to compile keeps its number and its code when its file is saved again. That costs a
proto's place and its code for each such function each time, up to `CODE_LIMIT`, past which nothing more is compiled.
Giving those back needs knowing that no code still running names the function, which the JIT doesn't keep track of.

## How it measures

**Round 5** (`r5/jit-merge`: agent 1's way in and out, generators and compiling on a thread of its own, merged with
agent 2's calls and property access; see "Compiling" and "Calls and properties the code knows").
Measured in ms, best of 20, process wall time. A is the base `run.exe` (`d16b715`, round 4's engine with the JIT built in)
with `RSUI_JIT=0`, interpreting; B this build with `RSUI_JIT=1`:

| bench | A | B | B/A | | bench | A | B | B/A |
|---|---:|---:|---:|---|---|---:|---:|---:|
| method_calls | 97.1 | 36.1 | **0.371** | | call_apply | 115.2 | 98.0 | **0.850** |
| richards | 143.1 | 56.6 | **0.396** | | map_set | 75.1 | 70.1 | 0.933 |
| float_math | 180.3 | 93.5 | **0.519** | | array_sort | 93.9 | 90.8 | 0.966 |
| closures | 86.8 | 45.2 | **0.521** | | string_build | 93.7 | 91.6 | 0.978 |
| class_proto | 70.2 | 37.3 | **0.531** | | json | 78.8 | 77.2 | 0.980 |
| fib | 80.3 | 44.4 | **0.553** | | buffer | 25.5 | 25.4 | 0.995 |
| prop_mono | 71.5 | 39.8 | **0.557** | | builtin_misc | 92.8 | 92.8 | 0.999 |
| raytrace | 81.6 | 46.9 | 0.575 | | array_functional | 91.5 | 91.9 | 1.004 |
| int_loop | 132.6 | 79.6 | **0.601** | | async_await | 73.1 | 73.5 | 1.005 |
| typed_arrays | 88.6 | 55.6 | **0.627** | | regex | 45.5 | 46.0 | 1.010 |
| alloc_churn | 122.4 | 81.8 | 0.668 | | zlib | 18.4 | 18.8 | 1.020 |
| try_catch | 68.1 | 45.9 | 0.674 | | object_keys | 110.0 | 112.3 | 1.021 |
| prop_poly | 79.8 | 58.1 | **0.728** | | react_render | 107.9 | 110.1 | 1.021 |
| array_ops | 84.6 | 62.0 | 0.733 | | string_ops | 74.7 | 76.3 | 1.023 |
| spread_destructure | 122.8 | 101.3 | 0.825 | | generators | 122.2 | 126.0 | 1.031 |

Geomean **0.757**; without startup, the 30 benches went from 2,346 to 1,703 ms. The same A/B, both sides with
`RSUI_JIT=1`, against:

- the same build (round 4's JIT): geomean **0.943**; class_proto 0.635, method_calls 0.667, prop_poly 0.790,
  richards 0.799, float_math 0.880, fib and raytrace 0.913, closures 0.914, try_catch 0.942, prop_mono 0.943, and
  nothing slower by more than 3% (generators, string_build and typed_arrays 1.03);
- agent 1's branch alone (`t3/jit-default-performance`, `d46a850`, built the same way): geomean **0.943**, with the
  same gains, and nothing slower by more than 3% (array_ops 1.028, generators 1.013, react_render 1.000);
- agent 2's branch alone (`t3/jit-call-property-performance`, `6d21061`): geomean **0.997**; its targets the same or
  faster (class_proto 0.981, closures 0.988, float_math 0.992, prop_poly 0.998, method_calls and fib 1.004, richards
  0.965), prop_mono 0.937 (agent 1's), and array_functional 1.035 and buffer 1.034, which a second run (best of 15,
  base, agent 1, agent 2 and this alternating) didn't repeat: array_functional 92.7, 89.8, 93.3 and 91.9 ms, buffer
  26.4, 26.1, 26.2 and 25.5, and generators 126.4, 125.3, 123.3 and 121.9.

Alone, agent 1's branch had measured 0.798 against `RSUI_JIT=0`, and agent 2's 0.946 against `RSUI_JIT=1`
(class_proto 0.65, method_calls 0.67, prop_poly 0.80, richards 0.84, closures and float_math 0.88, fib 0.92).

What getting both took, measured: with agent 1's threshold of 3000, float_math's `advance` became due as it was called
rather than in a loop, so its job knew nothing for "doubles first", and its code took 15% longer than with agent 2's
alone (1.2 million steps: 710 ms against 620); compiled at its loop's next turn, 601 against 601. The registers of
every loop rather than the frame's one would have done that too, but made typed_arrays 3.5 times as slow (see
"Doubles first"). float_math's n-body half also showed how far a session can drift with other work on the machine:
agent 2's build measured 74-76 ms in one and 82 in the next, against itself; only the alternating 20-round A/Bs, and
runs ten times as long, were worth deciding on.

The interpreter is agent 1's (`vm.rs` is the same). Without the feature the only change from `d16b715` is agent 2's
`Shapes::sweep` looking at pins (+512 bytes); the dispatch check gives 14,580 instructions, 328 indirect jumps, 48
reloading, as before. Against `d16b715`'s `run.exe` it measures a geomean of 0.997 (20 rounds), with string_ops
1.041, the one bench outside 3%: 1.043 again in 21 runs, and 1.016 in another, where agent 2's build without the
feature, the same size, gave 0.999. Nothing it runs is changed (its time is in the string builtins), so it's most
likely where code lands in the binary.

The example apps (the app benchmark at 40 rounds, best and median of 9 runs each, alternating; base `RSUI_JIT=0` as 1):

| app, what | base `RSUI_JIT=1` | agent 1 | this |
|---|---:|---:|---:|
| invoices, selecting an invoice | 1.043 / 1.037 | 0.998 / 1.018 | 1.028 / 1.019 |
| invoices, editing a field | 1.016 / 1.017 | 1.011 / 0.997 | 1.012 / 1.014 |
| invoices, typing in search | 1.047 / 1.013 | 1.014 / 1.004 | 1.013 / 1.008 |
| web_dashboard, mount | 1.158 / 1.144 | 0.998 / 0.971 | 0.977 / 0.984 |
| web_dashboard, hover | 1.030 / 1.009 | 1.056 / 1.017 | 1.011 / 1.017 |
| gallery, scrolling the list | 0.964 / 0.990 | 0.964 / 0.995 | 0.974 / 1.000 |

Every other interaction of gallery, tasks, notes, expenses and files is within about 3% of the base with
`RSUI_JIT=0`, as agent 1's is. In a second run of invoices alone (15 runs, agent 1 and this alternating), selecting an
invoice was 0.995 / 1.010 for agent 1 and 1.010 / 1.025 for this, and its other interactions also about 1.5% slower,
which is under what the app benchmark can resolve but goes the same way each time: the same functions are compiled, and their
code is 8% bigger (0.09 MB against 0.08), as agent 2's calls and property reads keep a way for what's known and one for
any, and an app's code comes back from memory at each interaction. web_dashboard's mount, which agent 2 measured 6%
slower from 22 functions compiled again and 40-55% more code, is as fast as interpreting here: with the threshold at
3000 only 5 of its functions are compiled in 40 rounds (0.06 MB, 1 compiled again, on the worker), where round 4's JIT
compiles 16 (0.32 MB).

Memory: engine memory over a 3000-step soak of the app benchmark is flat with the JIT on in all seven apps (objects, strings,
cells, shapes, atoms and rooted all +0 from the first tenth to the end; web_dashboard's object bytes +1 KB, where
agent 1's build gives +0.3 KB and the base interpreting +1.4 KB). Pinned shapes are counted out of `shapes` once
they're freed, so a pin that kept something would show there.

Binary size (shipped settings): without the feature 2,740,224 bytes (+512 over `d16b715`); with it, `run.exe`
3,101,184 (+53,760 over `d16b715`'s 3,047,424; agent 1's alone 3,085,312, agent 2's 3,046,912) and the app benchmark's executable
5,735,936 (+52,736). The snapshot (`Job`, `Look`, `Known`) and the rest of the merge are about 16 KB of that.

The numbers above are for `171be2f`; the commit after it only changes how the list of shapes gives back its end, and
measured 0.941 against round 4's JIT again (20 rounds), with the same Test262 results, soaks and dispatch counts.

Test262 (48,794 tests) gives exactly the baseline's outcome for every test without the feature, with
`RSUI_JIT=0`, and with `RSUI_JIT=1` and `RSUI_JIT_STRICT=1` (37,219 pass), and with `RSUI_JIT=eager` and
`RSUI_JIT_STRICT=1` exactly what `d16b715`'s eager build gives, test for test: 18 tests that fail either way have an
error's line one later in compiled code (`S11.13.1_A5_T3.js`, the twelve `S11.13.2_A5.*_T3.js`, `S11.3.1_A5_T3.js`,
`S11.3.2_A5_T3.js`, `S11.4.4_A5_T3.js`, `S11.4.5_A5_T3.js`, `S12.10_A1.6_T1.js`, `S12.10_A3.6_T1.js`: `Cannot access
'%with1' before initialization`). The engine's and rsui-tsx's tests pass without the feature (553), and with
`RSUI_JIT_STRICT=1` and `RSUI_JIT=eager` or `RSUI_JIT=1` (581 each); `jit/tests.rs` with `RSUI_GC_STRESS=1
RSUI_GC_VERIFY=1` (28, in 45 minutes), and with those and `RSUI_JIT=eager` the groups that collect shapes and other
things, one filter each: `shape` (10), `collect` (8 of 9), `weak` (5), `census` (2), `atom` (5), `class` (8). The
ninth of `collect`, `async_functions_survive_collections_while_they_wait`, which makes 150,000 objects with a collection
and a check of the heap at each safe point, hadn't finished after 70 minutes, and with `RSUI_JIT=0` it went just as
slowly.

**Round 4**, for comparison: A the interpreter at `r4/integration` (`0f4195f`, no JIT), B with the feature and
`RSUI_JIT=1`: geomean 0.812; against Node 24.16 and Bun 1.3.14 in the same session (7 runs), all 30 benches' wall time was 2,204 ms for us, 2,489 for Node and 2,302 for Bun; without startup (13 ms
against 39 and 37), 1,811, 1,308 and 1,184, furthest behind on call_apply, class_proto, method_calls, richards and
float_math (0.1-0.3 of Node). With the feature and `RSUI_JIT=0` the geomean against the build without it was 1.014, with
call- and generator-heavy code up to 8% slower: what the interpreter shares with the JIT's helpers is compiled
differently when they call it too.

## What's next

In about the order they'd pay:

- **Code size**, which is what apps pay for most: about 190 bytes an instruction before round 5's calls and fixed
  shapes, more now that calls have a way for the callee they know and one for any, and property reads one for the
  shape they knew and the cache's (575 bytes for a GetProp in round 4, 448 of them its out-of-the-way paths: the
  place's other entries, a string's or a number's prototype, what reads remember), written again at every place. An
  app runs most of its code about once per interaction, between layout and text that push it out of the processor's
  caches, so it comes back from memory each time: invoices' JavaScript takes a quarter less time compiled once
  everything's warm, but its first forty interactions gain little, and are about 1.5% slower with round 5's calls and
  property reads than with agent 1's alone, for 8% more code. One copy of those paths per function (or one in all,
  given the place), and short jumps, would roughly halve the code, and the time compiling takes.
- **Feedback from compiled code, and a second tier on it.** The guesses come from the caches and the frame at the
  moment a function is due; what compiled code then sees (which callee each call found, what types each instruction
  made, how often a call compiled in line went on in the callee) isn't kept. class_proto's `this.area()` in `describe`
  finds another callee than the one guessed 130,000 times. Counting it per place, and compiling again with it (the
  jump into new code at a call is there), would let calls of callbacks and arithmetic of mixed types be compiled for
  what they do, and "doubles first" wouldn't depend on where a function happened to become due.
- **Proxy traps and getters called in place.** The interpreter calls a proxy's `get` trap and a
  remembered getter written in JavaScript in place, as a call is (`get_cached_here`, `proxy_get_here`). Compiled code
  still sends a proxy read to the generic helper, which calls the trap from Rust (`get_cached`, then `Vm::call` and the
  trampoline into the trap's code), so with the JIT chart.js's bar update is 2-4% slower than interpreted (see "Built
  in by default"): 406,000 of its reads in the canvas benchmark are of proxies. A read that knows the trap, and
  calls it as compiled code calls a function it knows, would do in compiled code what the interpreter does.
- **Fewer checks on the same object.** Every property read and write checks its object's tag and shape again, even
  `this` in a short method read three times in a row. Knowing, within a stretch of instructions that can't change
  shapes, that a register holds an object of a shape already checked (fixed shapes make that something the code can
  know) would take most of them out; the address could be kept in `SPARE` or a register freed for it.
- **`call`, `apply` and bound functions in compiled code.** They go through `call_native` and the native, which makes
  the call they stand for from Rust (and into the target's code through the trampoline); the interpreter makes it in
  place. `forward_in_place` rewriting the stack, then a direct call with the argument count in a register (the
  prologue takes it from `ecx` already), would make them direct calls. call_apply is still 9x Node's time.
- **A generator's registers kept where they are.** Each value a generator yields copies its frame in and out of the
  generator object (`enter_generator_frame`, `suspend_generator`), in compiled code as in the interpreter; that copying,
  not the generator's own instructions, is most of what a pipeline of them costs now.
- **Async functions compiled.** An `await` is a `yield` to the job queue: a helper and a return, as `Yield` is, and
  `resume_generator` already goes into a compiled frame at its `pc`; what's left is `InitialYield` starting an async
  function (`start_async`, with the stack top the interpreter uses there).
- **Unboxed doubles.** Float code still boxes every result to store it; keeping doubles in `xmm` registers across a
  run of instructions, with the type checks done once, would be the next step after the last result in a register.
  float_math's advance spends most of its time on property reads and boxed arithmetic now (and `sqrt` and `/`).
- **Allocation in machine code.** `new` makes its `this` with a helper (`create_this`, `Vm::this_cache`), and object
  and array literals and closures are made by helpers too: allocating from the collector's free places in compiled
  code needs a way in to them that the collector's work this round can keep.
- **Unwind information** (`RtlAddFunctionTable`), so the native sampler, crash dumps and a debugger can walk through
  compiled frames: profiles of an app are mostly `?!?` now, with the frames above lost.
- ARM64 would be a second `x64.rs`, with the same helpers and layout.

## Where it touches the interpreter

For merging with other work on `run_loop` and the call path, everything outside `src/jit/` is behind
`#[cfg(feature = "jit")]`, only widens visibility, or is a refactor that's the same without the feature:

- `vm.rs`: the `jit` field of `Vm`; `run_loop` is `run_loop_body(stop, jit)`, with `run_loop_plain` and
  `run_loop_jit` its two copies with the feature; in the body, under `jit`, the hook after `enter!` makes a frame,
  `jit_loop!` after each loop instruction's jump back (LoopStrictNe and LoopStrictNeInt too), both through
  `jit_count!`, which counts in line, and in the `ENTERED` path a return once the frame on top is below `stop`;
  `Vm::call` and `Vm::run` go through `run_frame`; `INTERPRETED`, a frame flag; `alloc_for_jit`; `resume_in_place`
  and `resumes_in_place`, split out of `step_in_place`, and `enter_in_place`, split out of `resume_in_place`;
  `resumes_in_place` says no for a generator whose function has code (`has_code`, false without the feature);
  `pub(crate)` on `MAX_FRAMES`, `CONSTRUCTING`, `KEPT_ARGS`, `RESUMED_HERE`, `spare_args`, `alloc_unrooted`,
  `small_array`, `enter_function`, `finish_special_frame`, `delegate_next`, `forgive_refusal`, `tdz_error`,
  `make_closure`, `create_arguments`, `unwind`, `run_loop`, `pop_frame`, `suspend_new_generator`,
  `suspend_generator` and `yield_here`, which the helpers call.
- `props.rs`: `quick_loop_length`, split out of `loop_length`; `get_from_primitive_proto` (was
  `get_from_string_proto`), which numbers use too and which remembers what's further up or nowhere, through `learn`.
- `builtins/math.rs`: `quick_argc`, and for the JIT `quick_native`, `quick_name` and `quick_apply`.
- `compiler.rs`: a slot for each proto made, and a new count when a waiting function is compiled.
- `object.rs`: `Kind` is `repr(u8)` with the feature, and `object_layout` finds where things are in an object
  (a closure's `entry`, a native's `forwards`, `arguments`, a typed array's data, the element kinds' bytes).
- `arena.rs`: where an arena keeps its length and how much it has committed, for compiled code to read.
- `lib.rs`, `Cargo.toml`: the module and the feature; `Engine::jit_code` and `JitCode`, for the profiler.
- `object.rs`: `Shapes::pin`, and a pinned shape that's freed isn't given out again (`sweep`, `place`, `live`).

A change to what an instruction does in `run_loop` has to be made in `helpers::op` too, and in its template if it has
one (see "Adding an instruction"). What round 4's lanes changed and how it's mirrored here: AddInt32, DivInt,
LoopStrictNe, LoopStrictNeInt, SetElemTyped and CallMath are new in `helpers::op`, `jump_offset` and the templates;
GetLength and GetElem read `arguments` as a list and typed arrays at `TypedData::quick`; IncLoopLtLength's length is
`loop_length`'s; a closure's `entry` is in the layout but compiled calls go by the proto's slot; generators carried on
in place (`RESUMED_HERE`, `Value::ENTERED`) are left to the interpreter; `call` and `apply` and bound functions go
through `call_at`, which makes them the call they stand for (`forward_in_place`); an absent property's cache entry
names `Vm::absent`, which compiled code reads like any holder; reads that remember two entries per shape and name
(`lookup_at`) are what `mega_get` and `mega_set` use.
