//! Build-tool integration. Run explicitly after installing the pinned compiler:
//! cargo test -p rsui-tsx --test react_compiler_pack -- --ignored --test-threads=1

use std::path::{Path, PathBuf};
use std::process::Command;

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn folder(name: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("react-compiler-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

fn write(root: &Path, name: &str, source: &str) {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, source).unwrap();
}

fn tool() -> Command {
    let mut command = Command::new(TOOL);
    // The repository's earlier pilot installed these exact pinned versions.
    // This is a test-only convenience; production defaults to crates/rsui-tsx/react-compiler.
    let pilot = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/performance-research/compiler-toolchain");
    if std::env::var_os("RSUI_REACT_COMPILER_TOOLCHAIN").is_none()
        && pilot.join("node_modules").is_dir()
    {
        command.env("RSUI_REACT_COMPILER_TOOLCHAIN", pilot);
    }
    command
}

fn packed_sources(program: &Path) -> Vec<(String, Vec<u8>)> {
    let bytes = std::fs::read(program).unwrap();
    let footer = &bytes[bytes.len() - 24..];
    assert_eq!(&footer[16..], b"RSUIPACK");
    let mut at = u64::from_le_bytes(footer[..8].try_into().unwrap()) as usize;
    fn u32_at(bytes: &[u8], at: &mut usize) -> u32 {
        let n = u32::from_le_bytes(bytes[*at..*at + 4].try_into().unwrap());
        *at += 4;
        n
    }
    fn text(bytes: &[u8], at: &mut usize) -> String {
        let n = u32_at(bytes, at) as usize;
        let text = String::from_utf8(bytes[*at..*at + n].to_vec()).unwrap();
        *at += n;
        text
    }
    let _entry = text(&bytes, &mut at);
    let count = u32_at(&bytes, &mut at);
    (0..count)
        .map(|_| {
            let name = text(&bytes, &mut at);
            let offset = u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
            at += 8;
            let length = u32_at(&bytes, &mut at) as usize;
            (name, bytes[offset..offset + length].to_vec())
        })
        .collect()
}

#[test]
fn compiler_flags_require_the_build_workflow() {
    let output = Command::new(TOOL)
        .args(["--react-compiler", "--once", "absent.tsx"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("build-only"));
}

#[test]
#[ignore = "requires Node and npm ci --prefix crates/rsui-tsx/react-compiler on the build machine"]
fn compiled_modules_keep_original_resolution_and_run_standalone_with_and_without_bytecode() {
    let root = folder("graph");
    let source = root.join("project");
    let place = root.join("standalone");
    std::fs::create_dir_all(&place).unwrap();
    write(&source, "package.json", r#"{"type":"module"}"#);
    write(
        &source,
        "tsconfig.json",
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
    );
    write(
        &source,
        "src/caption.ts",
        "export const caption: string = 'relative';",
    );
    write(
        &source,
        "src/barrel.ts",
        "export { caption } from './caption';",
    );
    write(&source, "src/alias.ts", "export const alias = 'alias';");
    write(
        &source,
        "src/location.ts",
        "export const location = import.meta.url.endsWith('/src/location.ts');",
    );
    write(
        &source,
        "src/lazy.tsx",
        "import { extra } from 'delayed-package'; console.log('lazy-effect'); export const later = 'dynamic-' + extra; export function Later() { return <text>later</text>; }",
    );
    let main = r#"
        import React, { useState } from 'react';
        import { caption } from './barrel';
        import { alias } from '@/alias';
        import { location } from './location';
        if (process.env.RUN_LAZY === '1') import('@/lazy').then(module => console.log('dynamic:' + module.later));
        console.log('identity:' + import.meta.url.endsWith('/src/App.tsx') + ',' + location);
        export default function App() {
            const [value, setValue] = useState(7);
            return <window><text>{caption + ',' + alias + ',' + value}</text><button onClick={() => setValue(value + 1)}>add</button></window>;
        }
        console.log('compiled:' + (App.toString().includes('memo_cache_sentinel') || App.toString().includes('_c(')));
        console.log('relocated:' + import.meta.url.includes('/standalone/'));
    "#;
    let literal = source.join("src/App.tsx").to_string_lossy().into_owned();
    let main = format!(
        "{main}\nconsole.log('user-literal:' + {});",
        serde_json::to_string(&literal).unwrap()
    );
    write(&source, "src/App.tsx", &main);
    // Compiler dependencies outside the app graph must not be bundled merely
    // because the build used them.
    write(
        &source,
        "node_modules/unused-tool/package.json",
        r#"{"name":"unused-tool","main":"index.js"}"#,
    );
    write(
        &source,
        "node_modules/unused-tool/index.js",
        "throw new Error('unused tool');",
    );
    write(
        &source,
        "node_modules/delayed-package/package.json",
        r#"{"name":"delayed-package","main":"index.js"}"#,
    );
    write(
        &source,
        "node_modules/delayed-package/index.js",
        "exports.extra = require('delayed-sub').word;",
    );
    write(
        &source,
        "node_modules/delayed-package/src/__tests__/unused.tsx",
        "class Unused { @NotRuntime() value; }",
    );
    write(
        &source,
        "node_modules/delayed-package/types.d.cts",
        "export const value: MissingDeclarationOnlyType;",
    );
    write(
        &source,
        "node_modules/delayed-sub/package.json",
        r#"{"name":"delayed-sub","main":"index.js"}"#,
    );
    write(
        &source,
        "node_modules/delayed-sub/index.js",
        "exports.word = 'nested';",
    );
    let mut programs = Vec::new();
    for cache in [true, false] {
        let program = place.join(format!("compiled-{cache}.exe"));
        let mut command = tool();
        command.arg("--react-compiler").arg("--pack").arg(&program);
        command.env_remove("RUN_LAZY");
        if !cache {
            command.arg("--no-code-cache");
        }
        let built = command.arg(source.join("src/App.tsx")).output().unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        assert!(
            !String::from_utf8_lossy(&built.stdout).contains("lazy-effect"),
            "delayed module was executed during graph collection"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("src/App.tsx")).unwrap(),
            main
        );
        let files = packed_sources(&program);
        assert!(files.iter().any(|(name, bytes)| name.ends_with("App.tsx")
            && String::from_utf8_lossy(bytes).contains("react/compiler-runtime")));
        assert!(
            files.iter().all(|(name, _)| !name.contains(".rsui")
                && !name.contains("unused-tool")
                && !name.contains("@babel")
                && !name.contains("babel-plugin-react-compiler")),
            "{:?}",
            files.iter().map(|(name, _)| name).collect::<Vec<_>>()
        );
        programs.push((program, cache));
    }
    // Neither project files nor the helper/cache can be found at their old paths.
    std::fs::rename(&source, root.join("project-away")).unwrap();
    let mut pictures = Vec::new();
    for (program, cache) in programs {
        let picture = place.join(format!("frame-{cache}.png"));
        let times = place.join(format!("times-{cache}.log"));
        let ran = Command::new(&program)
            .arg("--png")
            .arg(&picture)
            .current_dir(&place)
            .env("PATH", "")
            .env("RUN_LAZY", "1")
            .env("RSUI_REACT_COMPILER_HELPER", "compiler-does-not-exist")
            .env("RSUI_REACT_COMPILER_TOOLCHAIN", "toolchain-does-not-exist")
            .env("RSUI_STARTUP_TIMES", &times)
            .output()
            .unwrap();
        assert!(
            ran.status.success(),
            "{}",
            String::from_utf8_lossy(&ran.stderr)
        );
        let stdout = String::from_utf8_lossy(&ran.stdout);
        assert!(stdout.contains("identity:true,true"), "{stdout}");
        assert!(stdout.contains("compiled:true"), "{stdout}");
        assert!(stdout.contains("relocated:true"), "{stdout}");
        assert!(
            stdout.contains(&format!("user-literal:{literal}")),
            "{stdout}"
        );
        assert!(stdout.contains("dynamic:dynamic-nested"), "{stdout}");
        let log = std::fs::read_to_string(times).unwrap();
        let line = log
            .lines()
            .filter(|line| line.starts_with("cache "))
            .next_back()
            .unwrap();
        if cache {
            let counts: Vec<u32> = line
                .split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect();
            assert!(counts[0] >= 5, "{line}");
            assert_eq!(counts[1], 0, "{line}");
        } else {
            assert_eq!(line, "cache 0 loaded 0 refused 0 resolved");
        }
        pictures.push(std::fs::read(picture).unwrap());
    }
    assert_eq!(pictures[0], pictures[1]);
}

#[test]
fn only_explicit_diagnostic_artifacts_accept_environment_test_steps() {
    let root = folder("diagnostics");
    let source = root.join("App.tsx");
    std::fs::write(&source, r#"
        import { useState } from 'rsui';
        export default function App() {
            const [value, setValue] = useState(0);
            return <window><text>{'Value ' + value}</text><button key="change" onClick={() => setValue(value + 1)}>change</button></window>;
        }
    "#).unwrap();
    for diagnostic in [false, true] {
        let program = root.join(format!("diagnostic-{diagnostic}.exe"));
        let mut command = Command::new(TOOL);
        command.arg("--pack").arg(&program);
        if diagnostic {
            command.arg("--pack-diagnostics");
        }
        let built = command.arg(&source).output().unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let picture = root.join(format!("diagnostic-{diagnostic}.png"));
        let steps = root.join("steps.txt");
        std::fs::write(&steps, "click #change; expect \"Value 1\"; look").unwrap();
        let ran = Command::new(&program)
            .arg("--png")
            .arg(&picture)
            .args(["--do", "this-remains-an-app-argument"])
            .env("RSUI_PACK_STEPS", format!("@{}", steps.display()))
            .output()
            .unwrap();
        assert!(
            ran.status.success(),
            "{}",
            String::from_utf8_lossy(&ran.stderr)
        );
        let stdout = String::from_utf8_lossy(&ran.stdout);
        assert!(
            stdout.contains(if diagnostic { "Value 1" } else { "Value 0" }),
            "{stdout}"
        );
        if !diagnostic {
            assert!(picture.is_file());
        }
    }
}

#[test]
#[ignore = "requires Node and npm ci --prefix crates/rsui-tsx/react-compiler on the build machine"]
fn compiler_failures_preserve_original_sources_and_existing_output() {
    let root = folder("failure");
    let source = root.join("project");
    write(&source, "package.json", "{}");
    let broken = "export default function App( { invalid syntax";
    write(&source, "App.tsx", broken);
    let output = root.join("previous.exe");
    std::fs::write(&output, "previous completed program").unwrap();
    let result = tool()
        .arg("--react-compiler")
        .arg("--pack")
        .arg(&output)
        .arg(source.join("App.tsx"))
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("React Compiler failed"));
    assert_eq!(
        std::fs::read_to_string(&output).unwrap(),
        "previous completed program"
    );
    assert_eq!(
        std::fs::read_to_string(source.join("App.tsx")).unwrap(),
        broken
    );
}

#[test]
#[ignore = "requires Node and npm ci --prefix crates/rsui-tsx/react-compiler on the build machine"]
fn explicit_dependency_roots_compile_wrapped_callbacks_under_original_package_paths() {
    let root = folder("external-root");
    let source = root.join("project");
    let dist = source.join("node_modules/pure-components/dist");
    write(&source, "package.json", "{}");
    write(
        &source,
        "node_modules/pure-components/package.json",
        r#"{"name":"pure-components","exports":{".":"./dist/index.mjs"}}"#,
    );
    write(
        &source,
        "node_modules/pure-components/dist/index.mjs",
        r#"
        import * as React from 'react';
        const define = Object.defineProperty;
        const label = (fn, name) => define(fn, 'name', { value: name, configurable: true });
        export const Child = React.forwardRef(label(function Child(props, ref) {
            const [value] = React.useState('state');
            return <text>{props.label + value}</text>;
        }, 'FriendlyChild'));
    "#,
    );
    write(
        &source,
        "App.tsx",
        "import { Child } from 'pure-components'; export default function App() { return <window><Child label=\"external \" /></window>; }",
    );
    let mut images = Vec::new();
    for external in [false, true] {
        let program = root.join(format!("external-{external}.exe"));
        let mut command = tool();
        command.args(["--react-compiler", "--pack"]).arg(&program);
        if external {
            command
                .arg("--react-compiler-root")
                .arg(&source)
                .arg("--react-compiler-root")
                .arg(&dist);
        }
        let built = command.arg(source.join("App.tsx")).output().unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let package = packed_sources(&program)
            .into_iter()
            .find(|(name, _)| name.ends_with("dist\\index.mjs"))
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&package.1).contains("react/compiler-runtime"),
            external
        );
        let picture = root.join(format!("external-{external}.png"));
        let ran = Command::new(&program)
            .arg("--png")
            .arg(&picture)
            .env("PATH", "")
            .output()
            .unwrap();
        assert!(
            ran.status.success(),
            "{}",
            String::from_utf8_lossy(&ran.stderr)
        );
        assert!(String::from_utf8_lossy(&ran.stdout).contains("external state"));
        images.push(std::fs::read(picture).unwrap());
    }
    assert_eq!(images[0], images[1]);
}
