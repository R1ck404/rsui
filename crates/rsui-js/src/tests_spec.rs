//! Tests of what the language and Node say some things are exactly: the names functions are given, the source text
//! they give back, normalizing text, Windows paths and `import.meta`.

use std::cell::RefCell;
use std::rc::Rc;

use super::*;

/// Runs a script to its end, and gives what it logged, one line to each call.
fn run(source: &str) -> String {
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let result = engine
        .eval(source, "test.js")
        .and_then(|_| engine.run_until_idle());
    if let Err(error) = result {
        panic!("{source}\nthrew: {error}");
    }
    output.borrow().trim_end().to_owned()
}

/// Writes these files into a folder of their own, runs the first as the main module, and gives what it logged.
/// `{dir}` in what's logged is the folder.
fn run_files(name: &str, files: &[(&str, &str)]) -> String {
    let folder = std::env::temp_dir().join(format!("rsui-js-spec-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    for (file, text) in files {
        let path = folder.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let ran = engine
        .run_file(&folder.join(files[0].0))
        .map(|_| ())
        .and_then(|()| engine.run_until_idle())
        .map_err(|error| error.message);
    let _ = std::fs::remove_dir_all(&folder);
    assert_eq!(ran, Ok(()));
    let shown = output.borrow().replace("\r\n", "\n");
    shown.trim_end().replace(&folder.display().to_string(), "{dir}")
}

/// Runs a script as a CommonJS file, so it has `require`.
fn run_cjs(name: &str, source: &str) -> String {
    run_files(name, &[("main.js", source)])
}

#[test]
fn getters_and_setters_are_named_get_and_set_their_key() {
    assert_eq!(
        run(r#"
            const s = Symbol('desc'), blank = Symbol(), key = 'k';
            const o = { get x() { return 1 }, set x(v) {}, get [s]() {}, set [blank](v) {}, get [key]() {}, get 1.5() {}, get 'a b'() {} };
            const d = (k) => Object.getOwnPropertyDescriptor(o, k);
            console.log(d('x').get.name, '|', d('x').set.name, '|', d(s).get.name, '|', d(blank).set.name, '|', d('k').get.name, '|', d('1.5').get.name, '|', d('a b').get.name);
        "#),
        "get x | set x | get [desc] | set  | get k | get 1.5 | get a b"
    );
    assert_eq!(
        run(r#"
            const s = Symbol('t');
            class A { get x() {} static set y(v) {} get [s]() {} get #p() { return 1 } static name2() {} p() { return Object.getOwnPropertyDescriptor(A.prototype, 'x').get.name } }
            const d = Object.getOwnPropertyDescriptor;
            console.log(d(A.prototype, 'x').get.name, '|', d(A, 'y').set.name, '|', d(A.prototype, s).get.name);
        "#),
        "get x | set y | get [t]"
    );
    // Built-in ones are named the same way.
    assert_eq!(
        run(r#"
            const d = Object.getOwnPropertyDescriptor;
            console.log(d(Map.prototype, 'size').get.name, d(Object.prototype, '__proto__').get.name, d(Object.prototype, '__proto__').set.name, d(RegExp.prototype, 'flags').get.name);
        "#),
        "get size get __proto__ set __proto__ get flags"
    );
}

#[test]
fn functions_under_computed_keys_are_named_from_them() {
    assert_eq!(
        run(r#"
            const s = Symbol('sym'), blank = Symbol(), k = 'kk';
            const o = { [s]() {}, [blank]: function () {}, [k]: () => {}, ['c']: class {}, *[Symbol.iterator]() {}, 7() {}, [1n]() {} };
            console.log(o[s].name, '|', o[blank].name, '|', o.kk.name, '|', o.c.name, '|', o[Symbol.iterator].name, '|', o[7].name, '|', o[1].name);
            const named = { [k]: function own() {}, [s]: class { static name() { return 'mine' } } };
            console.log(named.kk.name, typeof named[s].name);
            class C { [s]() {} static [k]() {} }
            console.log(C.prototype[s].name, C.kk.name);
        "#),
        "[sym] |  | kk | c | [Symbol.iterator] | 7 | 1\nown function\n[sym] kk"
    );
}

/// `path.win32` as Node's documentation describes it, with drive-relative paths like `C:foo`, which are in the folder
/// that drive was last in, and so not absolute.
#[test]
fn windows_paths_with_a_drive_and_no_root_are_relative_to_that_drive() {
    assert_eq!(
        run_cjs("drive_relative", r#"
            const { win32 } = require('path');
            const B = String.fromCharCode(92), w = (s) => s.replaceAll('~', B);
            console.log(['//server', w('~~server'), 'C:/foo/..', w('C:~foo~..'), w('bar~baz'), 'bar/baz', '.', 'C:foo', 'C:', ''].map((p) => win32.isAbsolute(p)).join());
            console.log([w('C:~temp~~foo~bar~..~'), w('C:////temp~~/~/~/foo/bar'), w('C:foo~..~..'), 'C:', 'c:foo/./bar//', w('C:..~x'), w('C:~..~x')].map((p) => win32.normalize(p)).join(' '));
            console.log([['C:', 'foo'], ['C:foo', '..', 'bar'], ['C:', w('~x')], ['//server', 'share'], ['C:x', 'D:y']].map((a) => win32.join(...a)).join(' '));
            console.log([win32.dirname('C:foo'), win32.dirname('C:'), win32.basename('C:foo.txt', '.txt'), win32.basename('C:'), win32.extname('C:.x'), win32.extname('C:a.x')].join(' '));
            console.log(JSON.stringify(win32.parse(w('C:~path~dir~file.txt'))), JSON.stringify(win32.parse('C:foo')), JSON.stringify(win32.parse('C:')));
            console.log(win32.relative(w('C:~orandea~test~aaa'), w('C:~orandea~impl~bbb')), win32.relative(w('C:~a'), w('D:~a')), win32.relative(w('C:~A~b'), w('c:~a~B~c')));
        "#),
        [
            "true,true,true,true,false,false,false,false,false,false",
            r"C:\temp\foo\ C:\temp\foo\bar C:.. C:. c:foo\bar\ C:..\x C:\x",
            r"C:\foo C:bar C:\x \\server\share\ C:x\D:y",
            "C: C: foo   .x",
            r#"{"root":"C:\\","dir":"C:\\path\\dir","base":"file.txt","ext":".txt","name":"file"} {"root":"C:","dir":"C:","base":"foo","ext":"","name":"foo"} {"root":"C:","dir":"C:","base":"","ext":"","name":""}"#,
            r"..\..\impl\bbb D:\a c",
        ]
        .join("\n")
    );
}

/// What `resolve` does with a drive and no root: it's in the folder Windows says that drive was last in, which it keeps
/// in a variable called `=Q:`, or the working folder if that's on the drive, or else the drive's root. Node can't set
/// one of those variables through `process.env`, so this only checks a drive that has none, and the working folder's.
#[test]
#[cfg(windows)]
fn resolving_a_drive_relative_path_uses_the_folder_that_drive_was_in() {
    assert_eq!(
        run_cjs("drive_resolve", r#"
            const { win32 } = require('path');
            const B = String.fromCharCode(92), w = (s) => s.replaceAll('~', B);
            const cwd = process.cwd(), drive = cwd.slice(0, 2);
            const kept = process.env['=' + drive.toUpperCase()];
            const here = win32.resolve(drive + 'x') === (kept && kept.slice(0, 2).toLowerCase() === drive.toLowerCase() ? kept : cwd) + B + 'x';
            console.log(win32.resolve('Q:foo'), win32.resolve('Q:foo', '..', 'bar'), win32.resolve('q:'), win32.resolve('Q:', w('~abs')), win32.resolve(w('~~s~h~x'), 'Q:y'), here);
        "#),
        r"Q:\foo Q:\bar q:\ Q:\abs Q:\y true"
    );
}

/// `path.posix` gives what Node does too, where the steps it used to follow didn't, for a last part that's `..`.
#[test]
fn posix_paths_are_taken_apart_as_node_does() {
    assert_eq!(
        run_cjs("posix", r#"
            const { posix } = require('path');
            console.log([posix.extname('..'), posix.extname('/..'), posix.basename('/', 'o'), posix.dirname('//a'), posix.normalize('./'), posix.relative('/a/b', '/a')].join('|'));
            console.log(JSON.stringify(posix.parse('.git')), JSON.stringify(posix.parse('/home/user/dir/file.txt')));
        "#),
        "||/|//|./|..\n{\"root\":\"\",\"dir\":\"\",\"base\":\".git\",\"ext\":\"\",\"name\":\".git\"} {\"root\":\"/\",\"dir\":\"/home/user/dir\",\"base\":\"file.txt\",\"ext\":\".txt\",\"name\":\"file\"}"
    );
}

/// `import.meta` as Node 24 has it: `url` a `file:` URL written as `url.pathToFileURL` writes it, which
/// `fileURLToPath` turns back into `filename`, `dirname`, `main`, and `resolve`, which treats what starts with `./` or
/// `/` as a URL from the module's own, whether there's a file there or not.
#[test]
fn import_meta_has_the_module_as_a_file_url_and_as_a_path() {
    let shown = run_files(
        "meta",
        &[
            (
                "a b#[x]~\u{e9}/main.mjs",
                r#"
                import { fileURLToPath, pathToFileURL } from 'url';
                import { other } from './other.mjs';
                const r = (s) => { try { return import.meta.resolve(s); } catch (e) { return 'threw ' + e.code; } };
                const up = new URL('..', import.meta.url).href;
                console.log(import.meta.url.startsWith('file:///'), import.meta.url.slice(up.length - 1));
                console.log(fileURLToPath(import.meta.url) === import.meta.filename, pathToFileURL(import.meta.filename).href === import.meta.url, import.meta.dirname + String.fromCharCode(92) + 'main.mjs' === import.meta.filename);
                console.log(Object.keys(import.meta).join(), Object.getPrototypeOf(import.meta), import.meta.main, other.main, other === import.meta);
                console.log([r('./other.mjs'), r('./missing.js'), r('./other.mjs?q=1#h'), r('../x/y.js')].map((u) => u.startsWith(up) ? u.slice(up.length) : u).join(' '));
                console.log(r('fs'), r('node:path'), r('nope-pkg'), r('https://a/b.js'), import.meta.resolve.name, import.meta.resolve.length);
                "#,
            ),
            ("a b#[x]~\u{e9}/other.mjs", "export const other = import.meta;"),
        ],
    );
    assert_eq!(
        shown,
        [
            "true /a%20b%23%5Bx%5D%7E%C3%A9/main.mjs",
            "true true true",
            "dirname,filename,main,resolve,url null true false false",
            "a%20b%23%5Bx%5D%7E%C3%A9/other.mjs a%20b%23%5Bx%5D%7E%C3%A9/missing.js a%20b%23%5Bx%5D%7E%C3%A9/other.mjs?q=1#h x/y.js",
            "node:fs node:path threw ERR_MODULE_NOT_FOUND https://a/b.js resolve 1",
        ]
        .join("\n")
    );
}

/// `normalize` takes text apart and puts it back together in each of Unicode's four forms, with NFC when it's not
/// told which, and a `RangeError` for a form that isn't one.
#[test]
fn normalize_gives_each_of_the_four_forms() {
    assert_eq!(
        run(r#"
            const show = (s) => [...s].map((c) => c.codePointAt(0).toString(16)).join(' ');
            const u = (...codes) => String.fromCharCode(...codes);
            const s = u(0x1E9B, 0x323);
            console.log(['NFC', 'NFD', 'NFKC', 'NFKD'].map((f) => show(s.normalize(f))).join(' | '));
            console.log(show(u(0x65, 0x301).normalize()), '|', show(u(0xD55C).normalize('NFD')), '|', show(u(0x1112, 0x1161, 0x11AB).normalize()), '|', show(u(0xFB01).normalize('NFKD')));
            console.log('ascii'.normalize('NFKD') === 'ascii', show(u(0xD800, 0x301).normalize()), show(u(0x41, 0x30A).normalize(['NFC'])));
            try { 'a'.normalize('nfc'); } catch (e) { console.log(e.constructor.name, e.message); }
            try { String.prototype.normalize.call(null); } catch (e) { console.log(e.constructor.name); }
        "#),
        [
            "1e9b 323 | 17f 323 307 | 1e69 | 73 323 307",
            "e9 | 1112 1161 11ab | d55c | 66 69",
            "true d800 301 c5",
            "RangeError The normalization form should be one of NFC, NFD, NFKC, NFKD.",
            "TypeError",
        ]
        .join("\n")
    );
}

/// What every kind of function written in a file gives back from `toString`: exactly its text, from its first token to
/// its last, as Node gives it. A method's starts after `static`. What's built in, or bound, says so.
const EVERY_KIND: &str = r#"function /* a */ plain /* b */ (x, y) /* c */ { return x + y; }
async function asyncFn() { await 1; }
function* gen() { yield 1; }
const arrow = (a) => a * 2;
const asyncArrow = async a => { return a; };
class A extends Object {
  #p = 1;
  constructor() { super(); }
  static   /* s */ m(a) { return a; }
  get /* g */ x() { return 1; }
  *g() {}
  ['comp' + 'uted']() {}
  static async *sag() {}
}
const o = { method() {}, set v(x) {}, async *ag() {}, [Symbol.iterator]() {}, f: function () {}, a: () => 1 };
const d = Object.getOwnPropertyDescriptor;
const fns = [plain, asyncFn, gen, arrow, asyncArrow, A, A.m, d(A.prototype, 'x').get, A.prototype.g, A.prototype.computed, A.sag,
  o.method, d(o, 'v').set, o.ag, o[Symbol.iterator], o.f, o.a, ((x) => (y) => x + y)(1),
  new Function('a', 'b', 'return a + b'), eval('(function e() { /* evaled */ })'),
  Math.max, Array.prototype.map, plain.bind(null), d(Map.prototype, 'size').get, [].values().map, Intl.NumberFormat];
for (const f of fns) console.log(Function.prototype.toString.call(f));
"#;

const EVERY_KIND_SHOWN: &[&str] = &[
    "function /* a */ plain /* b */ (x, y) /* c */ { return x + y; }",
    "async function asyncFn() { await 1; }",
    "function* gen() { yield 1; }",
    "(a) => a * 2",
    "async a => { return a; }",
    "class A extends Object {\n  #p = 1;\n  constructor() { super(); }\n  static   /* s */ m(a) { return a; }\n  get /* g */ x() { return 1; }\n  *g() {}\n  ['comp' + 'uted']() {}\n  static async *sag() {}\n}",
    "m(a) { return a; }",
    "get /* g */ x() { return 1; }",
    "*g() {}",
    "['comp' + 'uted']() {}",
    "async *sag() {}",
    "method() {}",
    "set v(x) {}",
    "async *ag() {}",
    "[Symbol.iterator]() {}",
    "function () {}",
    "() => 1",
    "(y) => x + y",
    "function anonymous(a,b\n) {\nreturn a + b\n}",
    "function e() { /* evaled */ }",
    "function max() { [native code] }",
    "function map() { [native code] }",
    "function () { [native code] }",
    "function get size() { [native code] }",
    "function map() { [native code] }",
    "function NumberFormat() { [native code] }",
];

#[test]
fn a_function_gives_back_the_text_it_was_written_as() {
    assert_eq!(run_cjs("to_string", EVERY_KIND), EVERY_KIND_SHOWN.join("\n"));
}

#[test]
fn function_source_preserves_unicode_after_full_collection() {
    let mut engine = Engine::new();
    let written = "function /* caf\u{e9} \u{1f600} */ unicode(\u{3c0}) { return '\u{4f60}\u{597d}\u{1f600}' + \u{3c0}; }";
    let script = format!("globalThis.written = {written}; globalThis.before = written.toString(); if (before !== {written:?}) throw new Error('exact Unicode source');");
    engine.eval(&script, "unicode-source.js").unwrap();
    engine.collect_garbage();
    engine.eval("if (written.toString() !== before) throw new Error('source survives full GC');", "verify-source.js").unwrap();
}

#[test]
fn function_source_file_is_revalidated_after_full_collection() {
    let folder = std::env::temp_dir().join(format!("rsui-js-spec-{}-source-revalidation", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("main.js");
    let source = "globalThis.written = function unicode() { return '\u{3c0}\u{1f600}'; }; globalThis.before = written.toString();";
    std::fs::write(&file, source).unwrap();
    let mut engine = Engine::new();
    engine.run_file(&file).unwrap();
    // Same byte length, different contents: both the existing last-file cache and hash validation matter.
    let changed = source.replace('\u{1f600}', "\u{1f601}");
    assert_eq!(source.len(), changed.len());
    std::fs::write(&file, changed).unwrap();
    engine.eval("if (written.toString() !== before) throw new Error('last-file snapshot changed');", "before-gc.js").unwrap();
    engine.collect_garbage();
    let result = engine.eval("if (!written.toString().includes('[native code]')) throw new Error('changed source accepted');", "after-gc.js");
    let _ = std::fs::remove_dir_all(&folder);
    result.unwrap();
}

/// The same, with every function compiled only when it's first called, which is when what's inside it is.
#[test]
fn a_function_compiled_when_first_called_gives_back_its_text_too() {
    let folder = std::env::temp_dir().join(format!("rsui-js-spec-{}-to_string_lazily", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("main.js"), EVERY_KIND).unwrap();
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    engine.set_lazy_compile(crate::LazyCompile::Always);
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    let ran = engine.run_file(&folder.join("main.js")).map(|_| ());
    let _ = std::fs::remove_dir_all(&folder);
    ran.unwrap();
    assert_eq!(output.borrow().trim_end().replace("\r\n", "\n"), EVERY_KIND_SHOWN.join("\n"));
}

/// A file's text is read again when it's asked for. If the file has changed since, what was compiled isn't there to
/// give, so it says what a built-in function says rather than give what it wasn't.
#[test]
fn a_function_from_a_file_that_has_changed_since_says_it_is_native() {
    assert_eq!(
        run_cjs(
            "to_string_changed",
            "function kept() { return 1; }\n\
             const fs = require('fs');\n\
             fs.writeFileSync(__filename, fs.readFileSync(__filename, 'utf8').replace('return 1', 'return 22'));\n\
             console.log(kept.toString());",
        ),
        "function kept() { [native code] }"
    );
}

/// The text of code given as a string is kept only as long as something compiled from it can run.
#[test]
fn the_text_of_code_given_as_a_string_goes_with_its_functions() {
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    engine
        .eval("globalThis.kept = new Function('return 1');", "test.js")
        .unwrap();
    engine.collect_garbage();
    let before = engine.vm.source_count();
    engine
        .eval(
            "for (let i = 0; i < 200; i++) new Function('return ' + i)();",
            "test.js",
        )
        .unwrap();
    engine.collect_garbage();
    // A few may be held on to by what ran last, but not two hundred.
    assert!(engine.vm.source_count() < before + 10, "{} after {before}", engine.vm.source_count());
    engine.eval("console.log(kept.toString())", "test.js").unwrap();
    assert_eq!(output.borrow().trim_end(), "function anonymous(\n) {\nreturn 1\n}");
}

/// A TypeScript file whose only exports are types is a module, as TypeScript has it, though nothing's left of its
/// exports once the types are gone. Read as CommonJS, it gave `export *` a `default` that made names ambiguous.
#[test]
fn a_typescript_file_that_only_exports_types_is_a_module() {
    let shown = run_files(
        "types_only",
        &[
            (
                "main.ts",
                "import * as all from './index';\n\
                 import * as types from './types';\n\
                 import { value } from './index';\n\
                 console.log(Object.keys(all).join(), value, Object.keys(types).length, 'default' in types);",
            ),
            ("a.ts", "export const value: number = 1;\n"),
            ("types.ts", "export type Shape = { size: number };\nexport interface Named { name: string }\n"),
            ("mid.ts", "export * from './types';\n"),
            ("index.ts", "export * from './a';\nexport * from './mid';\n"),
        ],
    );
    assert_eq!(shown, "value 1 0 false");
}

/// The same for a file that only imports a type, which makes it a module, so it has no `require` or `module`.
#[test]
fn a_typescript_file_that_only_imports_a_type_is_a_module() {
    let shown = run_files(
        "type_import_only",
        &[
            ("main.ts", "import './uses';\n"),
            ("uses.ts", "import type { Shape } from './types';\nconst s: Shape = { size: 2 };\nconsole.log(typeof module, typeof require, s.size);\n"),
            ("types.ts", "export type Shape = { size: number };\n"),
        ],
    );
    assert_eq!(shown, "undefined undefined 2");
}

/// `setTimeout` and `setInterval` give a `Timeout`, and `setImmediate` an `Immediate`, as in Node, which can be
/// unref'd so they don't keep the program going, and a `Timeout` turns into its number, which clears it too.
#[test]
fn timers_are_objects_that_can_be_unrefd() {
    let started = std::time::Instant::now();
    assert_eq!(
        run(r#"
            const t = setTimeout(() => console.log('never'), 5);
            console.log(typeof t, t.constructor.name, t.hasRef(), t.unref() === t, t.hasRef(), t.ref() === t, t.hasRef());
            const i = setImmediate(() => console.log('immediate'));
            console.log(i.constructor.name, typeof i.unref, typeof i[Symbol.toPrimitive], typeof i.refresh);
            const o = {}; o[t] = 1;
            console.log(typeof +t, Object.keys(o)[0] === String(+t));
            clearTimeout(+t);
            const s = setTimeout(() => console.log('never'), 5); clearTimeout(String(+s));
            const c = setTimeout(() => console.log('never'), 5); console.log(c.close() === c);
            const i2 = setImmediate(() => console.log('never')); clearImmediate(i2);
            let n = 0;
            const r = setTimeout(() => { n++; if (n < 3) r.refresh(); else console.log('refreshed', n, r.hasRef()); }, 1);
            // Nothing else is waiting, so this never runs, and the program ends.
            setInterval(() => console.log('never'), 60000).unref();
            setTimeout(() => console.log('unref, but runs while the other waits'), 2).unref();
            setTimeout(() => {}, 20);
        "#),
        [
            "object Timeout true true false true true",
            "Immediate function undefined undefined",
            "number true",
            "true",
            "immediate",
            "unref, but runs while the other waits",
            "refreshed 3 true",
        ]
        .join("\n")
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(30));
}

/// TypeScript's own way of naming a file by what it'll be compiled to: `./App.js` is `App.tsx` when there's no
/// `App.js`, as TypeScript and Vite find it, and a `.js` file that's really there is still what's found.
#[test]
fn a_typescript_file_is_found_by_the_name_of_the_javascript_it_becomes() {
    let shown = run_files(
        "ts_extensions",
        &[
            (
                "main.ts",
                "import { app } from './App.js';\n\
                 import { util } from './util.js';\n\
                 import { m } from './m.mjs';\n\
                 import { view } from './view.jsx';\n\
                 import { c } from './c.cjs';\n\
                 console.log(app, util, m, view, c);",
            ),
            ("App.tsx", "export const app: string = 'App.tsx';\n"),
            ("util.js", "export const util = 'util.js';\n"),
            ("util.ts", "export const util: string = 'util.ts';\n"),
            ("m.mts", "export const m: string = 'm.mts';\n"),
            ("view.tsx", "export const view: string = 'view.tsx';\n"),
            ("c.cts", "exports.c = 'c.cts';\n"),
        ],
    );
    assert_eq!(shown, "App.tsx util.js m.mts view.tsx c.cts");
}

/// `import.meta.env` is only there when what's running the program gives it, as rsui-tsx does for an app, the way Vite
/// does, and Node gives a module none.
#[test]
fn import_meta_env_is_what_the_embedder_says() {
    let folder = std::env::temp_dir().join(format!("rsui-js-spec-{}-meta-env", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("main.mjs"), "import { env } from './other.mjs';\nconsole.log(import.meta.env.MODE, import.meta.env.DEV, import.meta.env.VITE_NAME, env === import.meta.env, Object.keys(import.meta).includes('env'));").unwrap();
    std::fs::write(folder.join("other.mjs"), "export const env = import.meta.env;").unwrap();
    std::fs::write(folder.join("plain.mjs"), "console.log(import.meta.env, 'env' in import.meta);").unwrap();
    let output = Rc::new(RefCell::new(String::new()));
    let mut engine = Engine::new();
    let sink = output.clone();
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    engine.set_import_meta_env(&[
        ("VITE_NAME", MetaEnvValue::Text("app".to_owned())),
        ("MODE", MetaEnvValue::Text("development".to_owned())),
        ("DEV", MetaEnvValue::Bool(true)),
    ]);
    let ran = engine.run_file(&folder.join("main.mjs")).map(|_| ());
    let mut plain = Engine::new();
    let sink = output.clone();
    plain.set_printer(move |text| sink.borrow_mut().push_str(text));
    let plain_ran = plain.run_file(&folder.join("plain.mjs")).map(|_| ());
    let _ = std::fs::remove_dir_all(&folder);
    ran.unwrap();
    plain_ran.unwrap();
    assert_eq!(output.borrow().trim_end(), "development true app true true\nundefined false");
}
