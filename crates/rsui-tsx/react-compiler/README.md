# Build-time React Compiler

This tool compiles project React modules before making a standalone rsui app. Node and Babel are required on the build machine. The packed app uses rsui's existing `react/compiler-runtime`; it does not launch Node or load the compiler.

Install the exact locked build dependencies:

```powershell
npm ci --prefix crates/rsui-tsx/react-compiler
```

Build with the original entry path:

```powershell
rsui-tsx --react-compiler --pack MyApp.exe path/to/App.tsx
```

The nearest `package.json`, `tsconfig.json`, or `jsconfig.json` folder supplies the default source root. Repeat `--react-compiler-root DIR` for explicit roots, including workspace packages. The default helper scope compiles `.tsx`, `.jsx`, `.ts`, and `.js`; it skips nested `node_modules`, dot folders, `target`, symlinks, and declaration files. Compiler inference chooses components and hooks to optimize. Original files and import specifiers remain unchanged. rsui still resolves relative imports, package imports, re-exports, tsconfig aliases, dynamic imports, and `import.meta` from each original module path.

Dependency compilation is an explicit build choice. Supply both the app root and a dependency distribution root; specifying a broad app root does not include its nested dependencies:

```powershell
rsui-tsx --react-compiler --react-compiler-root path/to/app --react-compiler-root path/to/app/node_modules/some-react-package/dist --pack MyApp.exe path/to/app/App.tsx
```

Explicit roots additionally support `.mjs`, `.cjs`, `.mts`, and `.cts`, retaining each module's source goal. `.d.ts`, `.d.mts`, and `.d.cts` are excluded case-insensitively. A root inside `node_modules` must be explicitly supplied; canonicalization preserves that restriction across links. Choose a distribution folder rather than unrelated package source/tests. The compiler's source predicate permits only the validated input currently being transformed, overriding its normal dependency exclusion only inside this controlled build scope.

Some distributions wrap `forwardRef`/`memo` callbacks in function-name helpers, obscuring compiler inference. The build recognizes React import bindings and constant-bound wrappers whose sole operation returns the callback through the standard `Object.defineProperty(callback, 'name', plainDataDescriptor)` operation. It annotates only those module-scope callbacks, preserving wrapper calls, descriptors, existing opt-outs, incompatible-API bailouts, and critical compiler safety checks. Spoofed React objects, shadowed/reassigned built-ins, arbitrary wrappers, and dynamic component factories are excluded. Applied annotations are recorded in per-module diagnostics. Explicit dependency compilation can still fail legitimate compiler checks or provide no performance gain; validate the resulting artifact before adopting it.

In the shadcn demo experiment, adding the Radix Popper distribution to project compilation passed functional checks but produced no consistent native scrolling benefit over compiling the project alone. The delivered demo therefore compiles only project modules. Explicit dependency roots are an opt-in build capability, not a general performance recommendation.

The option is restricted to `--pack`. Ordinary development and HMR retain their existing behavior. `--no-code-cache` remains supported with compiled source; the default also packs bytecode recorded from that transformed source.

`react-hook-form:useForm` and `@tanstack/react-table:useReactTable` callers receive explicit `use no memo` bailouts because those APIs return mutable data. Add other known incompatible APIs with repeatable options:

```powershell
rsui-tsx --react-compiler --react-compiler-incompatible-api some-library:useMutable --pack MyApp.exe App.tsx
```

Compiler-inferred bailouts remain visible in the build summary and per-module diagnostic files. A compiler process or syntax failure aborts the build; it does not silently retry the original source. A failed build preserves a previously completed output.

The build follows literal imports, re-exports, dynamic imports, and unshadowed `require()` calls using rsui's own resolver, preserving import-versus-require package conditions. Delayed modules are collected without executing their top levels. Runtime packages outside explicitly selected dependency roots remain uncompiled and use the existing whole-package packing policy. A reached project module outside the configured source roots, or a nonliteral import/require whose target cannot be determined, fails explicitly. Use explicit source roots and statically named module edges for standalone builds. Arbitrary application loaders and runtime-generated module names are not a supported module graph.

Compiler compatibility depends on application semantics. Mutable observable APIs that the compiler cannot recognize, including some MobX patterns, may need the standard `"use no memo"` directive. This workflow does not prove every React library safe to memoize.

The `.rsui/react-compiler` folder contains build-only cached input snapshots, transformed modules, diagnostics, and manifests. Cache keys cover the original path and source, helper/compiler versions, and compiler options. Output hashes detect damaged cache entries. Builds check that original files did not change during compilation. This folder and external compiler dependencies are excluded from the app's loaded module graph and packed assets.

For an installed SDK, place this tool folder beside `rsui-tsx.exe` as `react-compiler`, or set `RSUI_REACT_COMPILER_HELPER` to its `compile.cjs`. Its adjacent `package.json` and `node_modules` normally provide the pinned dependencies. `RSUI_REACT_COMPILER_TOOLCHAIN` may select a separate directory containing that same pinned toolchain. The workspace source path is a development fallback, not a required path on the user's computer.

For verification of the actual packed payload, explicitly build a diagnostic artifact:

```powershell
rsui-tsx --react-compiler --pack-diagnostics --pack MyApp-test.exe App.tsx
$env:RSUI_PACK_STEPS = '@steps.txt'
.\MyApp-test.exe
Remove-Item Env:RSUI_PACK_STEPS
```

`RSUI_PACK_STEPS` uses the existing test-runner format: inline steps, `@FILE`, or `-` for standard input. Diagnostic metadata also enables the existing live inspector for native interaction tests. Ordinary packed artifacts ignore this environment variable and keep their inspector disabled. Arbitrary packed-app command-line arguments retain their existing semantics; this does not introduce a packed `--do` switch. Build paired plain/compiled diagnostic artifacts from the same executable when comparing behavior or performance. Node, Babel, source-cache paths, and helper execution remain absent from the running artifact.

Validate the build helper and standalone packaging:

```powershell
node --test crates/rsui-tsx/react-compiler/test.cjs
cargo test -p rsui-tsx --test react_compiler_pack -- --ignored --test-threads=1
```

Memoization can reduce repeated React work. It does not eliminate first-mount work, native layout, scrolling costs, or the memory retained by memoization caches.
