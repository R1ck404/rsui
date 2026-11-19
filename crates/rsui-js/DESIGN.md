# rsui-js

rsui's own JavaScript engine, for writing apps in TypeScript and JSX with no webview. JavaScript only runs
components and event handlers. Diffing, layout, text and painting stay in Rust, so a slow moment in JS is
short, and an app that's doing nothing runs no JS at all.

## Budgets

Anything over budget is a bug. Those marked "held" have a test that fails if they're broken.

| What | Budget | Where it is now |
|---|---|---|
| Added to a program's size | under 500 KB | **over: 1.36 MB** |
| A value | 8 bytes | 8 bytes, held |
| An instruction | 8 bytes | 8 bytes, held |
| An object | one cache line | 64 bytes, held |
| An empty engine | under 300 KB of RAM | 151 KB of data, held, and made in about 0.3 ms |
| CPU and memory work while idle | none: no timers, no background collector | true by design |
| Property reads in UI code | faster than QuickJS | yes: 161 ms against quickjs-ng's 586 ms |

Heavy loops won't match V8's JIT, and aren't meant to. That work belongs in Rust.

## How it works

- **Values** are 8 bytes: a double, or a tag and a 32-bit number in the space of NaNs.
- **Objects** are 64 bytes and share their layout through shapes, each saying where a property is. A place in the code
  that reads a property remembers the last shape it saw, which skips looking it up, and a table shared by the whole
  engine catches the places that see many shapes. The first four properties live inside the object, and so do the
  first four elements of an array that has no properties, so most need no second allocation.
- **Strings** of up to 22 ASCII characters live inside their entry, with no allocation. Long text built a piece at a
  time shares one growing buffer, so `out += piece` is linear. Property names are atoms, which compare as numbers. A
  whole number used as a key is an atom with no string behind it.
- **Code** is register bytecode with 8 bytes to an instruction. Calls run on one stack with no copying, as a
  caller's temporaries become its callee's first registers, and calling JS from JS doesn't use Rust's stack. Small
  numbers and constants go inside the instruction that uses them, a comparison and its jump are one instruction, and
  `for (...; i < n; i++)` ends in one instruction that does all of the loop's own work.
- **Memory** is a mark and sweep collector that runs only at safe points, between instructions, and only after
  enough has been allocated: 128 KB, or half of what's alive if that's more. It never runs on a timer. Objects don't
  move, and are named by 32-bit numbers. A native holding on to what it's made is safe, as everything made since the
  last safe point is kept. The big lists reserve addresses up front and take memory as they grow, so they never need
  two copies of themselves at once, and give memory back after a collection. `WeakMap`, `WeakSet`, `WeakRef` and
  `FinalizationRegistry` hold on weakly, as the language says: an entry of a `WeakMap` is kept for as long as its key
  is reachable some other way, and goes with it. Full collections also free shapes, atoms and compiled code that nothing
  uses any more, so saving a file again and again while an app runs keeps one version of its code (`compiled.rs`).
- **Parsing** puts the whole syntax tree in one block, with an expression taking 32 bytes. It goes back to the system
  as soon as the code has been compiled, and a module's source text is gone before the module runs.
- **TypeScript** has its types removed by the parser, so running never waits on checking them. An `import` or `export`
  written at a file's top makes it a module, as TypeScript has it, even one of only types, which is gone once they are. `enum` and
  parameter properties become real code, and so do `accessor` fields, as a getter and a setter over a private field.
  `abstract`, `declare` and `override` are only types. A named import that the other module doesn't export is a type, in TypeScript, as the
  app wouldn't have compiled otherwise, so it's nothing, as TypeScript leaves that import out. Checking types is left to the official tools.
- **JSX** becomes calls to `__jsx(type, props, children, mayHaveKey)`, where `children` is an array, and each of the last two is left out if there's nothing to say.
- **Modules** are found the way Node finds them, through `node_modules`, `package.json`, `exports` and all. Node's
  `events`, `path`, `util` and `assert` come with it, and are only compiled if something asks for them. As TypeScript and
  Vite have it, `./App.js` is `App.ts` or `App.tsx` when there's no `App.js` (`.jsx` is `.tsx`, `.mjs` `.mts`, `.cjs`
  `.cts`), from any file, as the name would find nothing otherwise. `path` follows Node's own steps for both kinds of path
  (`builtins/host/path_win32.rs`), so `C:foo`, which is in the folder that drive was last in, `CON:` and `\\?\` all come
  out as they do in Node; checked against it on 19,532 calls.
- **`import.meta`** has what Node 24 gives, in its order: `dirname`, `filename`, `main`, `resolve` and `url`, which is
  written as `url.pathToFileURL` writes it, so `fileURLToPath` turns it back. `resolve` takes what starts with `./` or
  `/`, and URLs, as URLs from the module's own, without looking for the file, as Node does. `import.meta.env` is only
  there if what runs the program gives it (`Engine::set_import_meta_env`), as rsui-tsx does for an app, the way Vite
  does, from its `.env` files: Node gives a module none.
- **A function's text**, which `Function.prototype.toString` gives back as it was written, isn't kept. Each function has
  where its text starts and ends, and which source that's in, in a table beside its proto (`compiled.rs`), 12 bytes a
  function. What's bundled is in the program already; a file is read again when it's asked for, through the same steps
  as when it was loaded, and only used if it's as long and hashes the same, and the last one read is kept until the next
  full collection. Only code given as a string, to `eval` or `new Function`, is copied, and each source goes once nothing
  compiled from it can run (`source_text.rs`). The engine's own standard library, `helpers.js`, says it's built in, as
  the language's functions do.
- **ES modules** are loaded as the language says, with everything they import that hasn't been: each is compiled as a
  generator that stops twice before its code runs. Every module takes its first step, which hands over the cells of the
  variables it exports, then every module its second, which takes the cells of what it imports and makes its functions,
  and then each runs, after what it imports. So an import is the variable it names, not a copy, at the cost of reading
  any variable a function holds on to, and modules that import each other see each other's functions straight away and
  the rest once it's there. A module can `await` at its top, and what imports it waits for it, and so does
  `Engine::run_file`, so an app's window has its component. A namespace (`import * as`) is a proxy that answers for
  itself, reading each name's cell, so it behaves as the language's own kind of object does, and `require` of an ES
  module gives what Node gives. A file is an ES module if it's `.mjs`, if its package says `"type": "module"`, or if what's in it
  says so. CommonJS is run when its turn comes, and what's imported from it is what it had then, as in Node.
- **What a render works out** is kept for the next one: a plain call in a component's or a hook's body gives what it gave
  last time while it's given the same and nothing it was worked out from has changed, which is everything reached from the
  function and its arguments, watched through the collector's write barrier, so any change to it, by anything, is seen. A
  call that changes what it didn't make, calls something that isn't pure, or throws isn't kept (`keeping.rs`, and "What a
  render works out" in rsui-tsx's DESIGN.md).
- **`using` and `await using`** need no instructions of their own. A block with one in it is compiled as a `try` with a
  `finally`, from the first `using` on, and each declaration calls a native that puts what it's given on a list the block
  keeps in a register, which the `finally` has the same native dispose of, last first. For `await using`, it does that a
  step at a time, and the compiled code waits for what each step gives, so it waits at the same moments as Node.
  `DisposableStack` and `AsyncDisposableStack` keep the same kind of list. Code with no `using` in it compiles as it did.

Parts of the standard library that are simplest in JavaScript, like iterator helpers and `Set` methods, are written in it,
and only compiled the first time one is called.

`unsafe` code is kept to the interpreter's loop, which reads instructions, registers and object headers without bounds
checks, the lists that never move (`arena.rs`), the compact layouts in `object.rs`, the shared text buffer in `text.rs`, the
block the syntax tree lives in (`ast.rs`), the loops in `builtins/host/zlib` that write what's undone into room made for
it ahead, and the processor's carry-less multiply for CRC-32, and what Windows is asked through: sockets and pipes (`iocp.rs` and `net.rs`) and
watching for changes (`builtins/host/watch.rs`). Each says what makes it sound.

### Many properties at once

Rust that makes, reads and fills objects for every element, as rsui-tsx's styling does, has its own way in (`bulk.rs`), as
going through `get_property` and `set_property` a property at a time finds where each is again: a list of names found once
(`Keys`), objects made with just those names with the shape found once (`new_object_with`), reads of them with where each is
remembered for the last few shapes (`read_keys`) or found from the shape alone for names no prototype has (`read_own_keys`),
keys taken out keeping a shape other objects share rather than a dictionary (`delete_keys`), and `assign`, which remembers for
each pair of shapes what goes where and the shape the target ends with, so copying is reading and writing slots. What's
remembered is forgotten when a full collection frees shapes (`Shapes::generation`), and an addition is only trusted for the
`epoch` it was found in. Measured with the JIT in `bulk_speed`: 12 numbers made 97 ns (109 as an object literal in JS), 6 reads
17 ns (11 in JS), a rule's props put on a look 28 ns (`Object.assign` in JS 52). `Object.assign` itself uses `assign` for a
plain source and target, which on Mantine's page was 4% of its steps (1114.6 against 1161.2 ms) and on shadcn's 1%; Test262's
`built-ins/Object`, object literals and assignment pass as before (4,834 of 5,066).

What `bulk.rs` remembers is kept in the box `number_strings` already was (`NumberStrings`, which derefs to the array it was),
not in a field of `Vm` of its own: there, its 136 bytes moved the fields after it, and js-bench's interpreter, which never runs
any of it, was 2% slower (prop_mono and prop_poly 7%, int_loop 6%, the same in two runs of ten rounds); as a box at the end of
`Vm`, 14% (fib 40%). With `Vm`'s fields as they were, those benches are where they were. A field added to `Vm` is measured
with js-bench for that reason.

## What made it fast

Two of the biggest wins aren't what you'd guess, and are easy to lose by accident. They were found by measuring on a
Ryzen 7800X3D, and other processors may differ.

- **Lists start at different places in their pages** (`Arena::with_limit_at`). A processor can take a read for a recent
  write when both addresses have the same position in their pages, and then has to start over. A small program's
  instructions, registers and variables would all be near the start of their lists. Fixing this took one loop from 280 ms
  to 154 ms. A few code positions still hit it, which is why a benchmark can change by 30% between builds for no other
  reason. Compare several positions before believing a small change made something worse.
- **Instruction fields are read inside each handler**, by matching on a reference to the instruction. Copying the whole
  instruction first has the compiler read every field before dispatching, which holds on to registers the handlers could
  use. That was about 20% on calls and arithmetic.

## How it's tested

Every test here runs real code. Beyond those, these were last measured on 2026-11-06:

- **npm packages:** 36 popular packages, each used by a small program, print exactly what they print in Node 24.
- **Test262**, the official conformance suite, on 2026-11-10: 38,336 of 42,966 pass (89.2%), using `examples/test262.rs`,
  including 618 of the 704 module tests, and 5,828 are for proposals that Node 24 doesn't have either.
- **Speed**, on 22 small benchmarks of what programs do, best of five runs. In all, 1,994 ms, against 7,285 ms for
  quickjs-ng 0.18 and 2,471 ms for V8 without its JIT. It beat QuickJS on all 22. Against V8 without its JIT it was faster
  on 10 and slower on 12, mostly by up to 1.3 times, with JSON at 1.6 and regular expressions at 1.3, and string building
  at 4 ms against 1 ms. On 19 more, of features like generators and `Map`, it took 1,898 ms against 7,556 ms and 2,585 ms.
  QuickJS and V8 without its JIT both do generators in half the time. V8 with its JIT is far ahead on long loops, and
  that's expected.
- **Peak memory**, as working set and then private bytes. Keeping 300,000 small objects alive: 55.5 MB and 55.9 MB,
  against 110.7 MB and 107.2 MB for QuickJS and 123.4 MB and 131.3 MB for Node. Loading lodash: 8.0 MB and 5.3 MB, against
  49.3 MB and 24.6 MB for Node. QuickJS has no `require` to compare. An engine that's doing nothing adds about 600 KB of
  private memory and 700 KB of code pages to a process. Most of a process's working set is Windows' own shared libraries,
  at about 4.8 MB with no engine at all.
- **The web's streams**, on 2026-11-10: of the web platform's own tests for them (WPT's `streams/`, run with a small stand-in
  for its harness, leaving out what needs collecting garbage on demand), 1,118 of 1,137 pass. Node 24 fails 15 of the same
  ones, which are for a proposed kind of stream and for something WPT's harness does differently, and the 4 only this fails
  need WebAssembly. Programs for `fetch` and `http` reading from a server that streams print what they print in Node 24.

The biggest groups of what still fails in Test262: `Atomics` and `SharedArrayBuffer`, regular expressions' `v` flag and
full Unicode property tables, direct `eval`, typed arrays' subclass (`Symbol.species`) rules, realms,
and early errors that nothing valid would ever hit.

## What's not here yet

- Direct `eval` seeing local variables. All `eval` is the global kind.
- `arguments` following changes to parameters.
- `Intl`, and real `localeCompare`. `normalize` is real: its tables are made from Unicode 18's database
  (`builtins/normalize_tables.rs`, about 55 KB of the program), and it passes all of Unicode's
  `NormalizationTest.txt`. `Date` has local time from Windows, which doesn't know about the times
  before standard time that Node's own database does. `toLocaleString` and its kind use the user's format from Windows too,
  and with options they understand the common ones, and put things in American order.
- `SharedArrayBuffer`, `Atomics`, the `v` flag on regular expressions, and `Symbol.species` on typed arrays.
- Private names belonging to each time a class is evaluated. They belong to the class in the code, so two classes made by running the
  same code, as a function that makes a class would, can read each other's.
- A subclass of `Array` seeing each element go in as a method works it out. It's made first, and it's given them all at the end.
- `\p{...}` only knows the common properties and scripts. The rest are accepted, but match too much, as a full table
  would cost more binary size than it's worth for now.
- `Object.keys` of a module's namespace doesn't throw for a name that isn't there yet, as the
  language says it should. A named import from CommonJS is whatever it has under that name, where Node would first check
  that it has one, and `"type": "commonjs"` doesn't stop a file that looks like a module being one.
- TLS of its own, public keys and ciphers. `tls`, `cluster`, `v8` and `vm` are there to load, as a lot of packages load them without using
  them, and what would need them says so when it's called. `fetch` and `WebSocket` do TLS, through what comes with Windows.
  What's really there is `net` (TCP and the system's named pipes), `dns` (looking names up, through the system), `dgram`, `worker_threads` and `Worker`, `fs` (including files by number, and `watch`, `watchFile` and `promises.watch`, with one thread for all the watchers that waits without using any CPU), `path`, `os`, `crypto` (hashes, HMAC,
  PBKDF2 and random numbers), `stream`, `child_process`, `http` and `https` (asking for things, with what comes back arriving as it
  comes, though what's sent goes all at once, and a simple server, with one request to a connection and no upgrades, which sends
  what's written a chunk at a time when it hasn't said how long it'll be), `http2` (sessions that ask for things the same way,
  with HTTP/2's headers, and no server), `zlib` (DEFLATE that's as small as Node's at each level, written and read at about its speed or more; Brotli, which reads all of what the format has, dictionary and all, and writes within a few percent of Node's size up to quality 9 but about an eighth bigger at 11, as it doesn't use the dictionary or search as hard; and Zstandard, which reads all of RFC 8878, with dictionaries of either kind, and writes no more than 4% bigger than Node does at levels 1 to 19, using zstd's fast and double-fast strategies at levels 1 to 4 and its binary tree and the cheapest way through each stretch at the slowest levels, at about two thirds of Node's speed at levels 1 and 3 and about its speed at 9 and 19, and reads it faster than Node. All of them work a piece at a time for streams. As in Node, Zstandard's frame is the end when more comes after it in the same piece, and the web's compression streams don't take it), `events`, `util`, `assert`,
  `url` (all of it: file URLs and paths both ways, with drive letters and network shares, `\\?\UNC\` ones too, and the older `parse`, `format` and
  `resolve`; names that aren't ASCII go into Punycode without UTS #46's tables), `buffer`, `querystring`, `string_decoder`, `timers`, `readline`, `punycode`, `async_hooks`, `perf_hooks`, `module` and `tty`.
  `AsyncLocalStorage` doesn't follow a function past an `await`. File handles from `fs.promises`, `net` and `http` servers,
  `dgram` sockets, `readline`, child processes and workers can be disposed of as in Node 24, but timers and streams can't.
  `setTimeout` and `setInterval` give a `Timeout` and `setImmediate` an `Immediate`, as in Node, which can be `unref`'d so
  they don't keep the program going, and a `Timeout` turns into its number, which `clearTimeout` takes too, as code written
  for a page expects. Code written for a page gets these as well, as it's the same `setTimeout`. `Intl` goes by Windows for numbers and dates in any language, has
  English's rules for plurals, lists and times, and only knows the local time zone and UTC. There's `Blob`, `File`, `MessageChannel`,
  `navigator` and `DOMException`, and `WebSocket`. The web's streams are all there, as the standard says, from byte streams and
  their own readers to the text and compression streams, with `stream/web` and Node's ways between its streams and these. The compression
  streams squeeze only as well as `zlib` does. `fetch` is kept once the headers are in, and then reads each part of the body only when
  something asks for it, so it can be read as it arrives and stopped partway. What's sent with a request is read whole first, even
  from a stream. The Anthropic and OpenAI SDKs stream a reply through it from a server on the same machine, an event at a time, as
  they do in Node. All of this is compiled the first time something looks for it, so it
  costs nothing until then. 135 popular packages have been run against Node.
- A worker has an engine of its own, and what goes between them is copied. Nothing can stop code that's running, so `terminate()` only takes effect when it's next
  between things. There's no `SharedArrayBuffer` between them, and a port can't be passed along. `process.exit()` inside one ends the whole program.
- One thread looks after every socket, named pipe, and other program's input and output, through a completion port, and it's only there while something's waiting on one of them. A connection that's idle holds no buffer. A server has one connection at a time waiting to be accepted, with the rest waiting their turn in the system's queue.
  Each request from `fetch` or `http`, and each `WebSocket`, has a thread of its own while it's going on, and none for each read.
- Sparse arrays: `a[100000000] = 1` really makes an array that long, up to a limit of 2^28 elements.
- Text that's added to at the front is copied each time, as only adding to the end shares a buffer.
