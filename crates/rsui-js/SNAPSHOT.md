# Compiling the bundled JavaScript ahead of time: a design note

Not built yet. The bytecode format is changing (op.rs, compiler.rs), so this waits until that has settled. It's written
down now so the next round can start from it.

## What it would save

Every start, the engine reads and compiles the JavaScript that ships inside it: `js/helpers.js` when an engine is made,
then whichever bundled modules the app loads (`rsui` = rsui-tsx's `layer.js`, `fs`, `path`, `internal/web`, ...). After
this round's parser work, for the invoices app that's roughly 1 ms of parsing and 1.5 ms of compiling out of about 5 ms of
loading (layer.js 0.36 + 0.45 ms, node/web.js 0.34 + 0.32 ms). Compiling is now the bigger half, and only
a snapshot (or compiling lazily, below) takes it away. `rsui/web` (web.js, 276 KB) is 2.4 ms to parse and 4.6 ms to
parse and compile, for an app that uses it.

## What goes in it

For each bundled file, what `Compiler::program` makes, in a form that doesn't depend on the run:

- A header: a magic number, a format version, and a hash of the engine's own build (the compiler's source and
  `op.rs`), so a snapshot from another build is never used. If it doesn't match, the file is compiled as it is now.
- The atoms it uses, as strings. Atoms are numbered as they're interned, so in the snapshot every atom is an index into
  this table, and loading interns each once and maps the indexes.
- The constants (`Vm::add_const`): numbers as bits, strings as UTF-16, BigInts as their digits, and anything that's an
  object at run time (template objects, regular expressions) as what makes it, so it's made when first used, as now.
- Each function's `Proto` (`op.rs`): registers, params, length, flags, strictness, name and file atoms, captures and
  their names, exception handlers, the line table, and its code as the instructions are written, with constant, atom
  and proto references as indexes into the snapshot's own tables. `start` and `cache_base` are given out again on
  load, as the code and inline caches are appended to the VM's.
- Where each function's source starts and ends, for `Function.prototype.toString`: the source is already in the binary
  (`include_str!`), so the text itself isn't copied.

## Where it's made

`build.rs` can't run the engine it's building. Two ways round that:

1. **A generator, with its output checked in.** `cargo run -p rsui-js --example snapshot` compiles the bundled files and
   writes `src/snapshot/*.bin`, which `include_bytes!` puts in the binary. A test compiles them again and fails if the
   bytes differ, so a change to the JS or the compiler that isn't followed by regenerating is caught. Simple, but every
   compiler change means regenerating, and the files show up in diffs.
2. **The parser and compiler in their own crate**, used by both `build.rs` and the engine. Nothing to check in, but the
   compiler takes `&mut Vm` today (for atoms and constants), so it would need a small interface for those first. More
   work, and probably right in the end.

Start with 1. Packed apps could carry their own files' snapshots the same way, made by `rsui-tsx pack`.

## Loading

`load_builtin_module` (modules.rs) and the helpers loading in `builtins/mod.rs` look for a snapshot of the file before
`compile_wrapped`. Loading is: check the header, intern the atoms, append code and constants, make the protos. That
should be a small fraction of compiling, as it's copying and one hash lookup per atom.

Better still, load each function only when it's first called: most of layer.js's and web.js's functions never run in a
given app. That needs a proto that says "not loaded yet, here's where it is in the snapshot", which the call path
loads the first time. That's the same hook lazy compiling needs:

## Compiling functions lazily (built)

A function in a package (a file with a `node_modules` folder in its path, on disk or in a packed program) is compiled
the first time it's called (`compiler.rs`: `Compiler::function_later`, `Uncompiled`, `Vm::compile_on_first_call`).
rsui's own runtime and an app's own files are compiled whole when they load, as most of them runs by the first frame
anyway: for them, waiting only moved the work from loading to mounting and kept their trees in memory.
`Engine::set_lazy_compile` (`LazyCompile::Never`, `NodeModules`, `Always`) changes which files wait.

- The parser notes, for each function, every name it or a function inside it may need from outside
  (`Function::outer_names`). When the code around a function is compiled, each of those that's a cell there, or that the
  code around it sees from outside, becomes a capture, as compiling the function would have found. So the function's
  proto (name, `length`, flags, strictness, captures) is made then, and the code around it never needs compiling again.
- Its code is made on its first call. Until then its `start` is `NOT_COMPILED` and it has `KEEPS_ARGS`, so every way
  into it goes through `Vm::enter_function`, which compiles it into the same proto. The fast way into a function pays
  nothing for this.
- The file's tree and source are kept (`KeptAst`) while any of its functions is waiting. That's 0.1-0.5 MB for the
  example apps and 1.3-1.5 MB for the ones that use `rsui/web` (web.js). Parsing a function again from its source range
  would save that memory, but most of the bundled code does run by the first frame, so it would mean reading most of it
  twice.
- What compiling would have found wrong with a function (redeclarations, `break` with nowhere to go, ...) is still
  found when the file loads, by a walk that follows the compiler (`Compiler::check_function`). The bundled modules, when
  they're made to wait, skip it, as it costs a good part of what's saved; tests compile each of them whole instead
  (`check_compiles`).
- Not lazily: what's inside a `with`, a field initializer with a computed key (its key is read where the class is),
  and code given to `eval` and `new Function`.

Measured, with every file waiting: loading drops 15-20% for every example app, though about half of that comes back
when mounting, as components are compiled when they first render, and to first frame moves 0-5%, for 0.1-0.5 MB more
memory (1.3-1.5 MB with `rsui/web`). With only packages waiting, for an app using lodash, date-fns and zod (about 300
package files, 1 MB of them read): loading and first frame about 1% sooner (lodash alone loads 6.0 -> 5.0 ms, but most
of loading such an app is finding and reading the files), for 3.3 MB more memory, most of it lodash's tree and source.
A package that's big and mostly unused gains most; many small files whose functions all run gain nothing.

The next step is a pre-parse (V8's lazy parsing): read a function's body only to find where it ends, the names it
mentions and its early errors, and parse it fully on its first call. That needs the compiler's early errors
(`check_function`) to move into the parser, and the parser to read a function on its own from a source range with
what it needs to know about where it is (strictness, `await`/`yield`, `super`, `new.target`, private names). Snapshots
(above) would take compiling out of startup altogether, and lazy loading from a snapshot uses the same hook.

## What to measure

Parse and compile time per file, before and after; how much of load is left
in `Compiler::function`, from a startup sample of an app; `Engine::new` for helpers.js; and binary size, as the snapshot adds roughly the size of the
bytecode (an instruction is 8 bytes) plus constants for every bundled file.
