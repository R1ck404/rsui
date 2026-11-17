//! Compiler-shaped programs exercise the runtime without depending on Babel or npm.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use rsui_headless::{Headless, Query};

fn file(source: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "compiler-runtime-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("app.tsx");
    std::fs::write(&path, source).unwrap();
    path
}

fn open(source: &str) -> Headless {
    let entry = rsui_tsx::load(&file(source), 512 * 1024).unwrap();
    Headless::new(rsui_tsx::root, entry)
}

fn click(ui: &mut Headless, key: &str) {
    ui.click(&Query::new().key(key)).unwrap();
}

#[test]
fn caches_keep_separate_cursors_and_recompute_only_changed_inputs() {
    let mut ui = open(
        r#"
        import { useState } from 'react';
        import { c } from 'react/compiler-runtime';
        const EMPTY=Symbol.for('react.memo_cache_sentinel');
        let calculations=0;
        function useLabel(n) {
            const $=c(2);
            if ($[0]!==n) { $[1]='value '+n; $[0]=n; calculations++; }
            return $[1];
        }
        export default function App() {
            const [n,setN]=useState(0), [tick,setTick]=useState(0);
            const a=useLabel(n), b=useLabel(n+10), $=c(1);
            if ($[0]===EMPTY) $[0]='constant';
            return <window><text>{a+' '+b+' '+$[0]+' calculations '+calculations+' tick '+tick}</text>
                <button key='tick' onClick={()=>setTick(tick+1)}>Tick</button>
                <button key='next' onClick={()=>setN(n+1)}>Next</button></window>;
        }
    "#,
    );
    assert!(
        ui.outline()
            .contains("value 0 value 10 constant calculations 2 tick 0")
    );
    click(&mut ui, "tick");
    assert!(ui.outline().contains("calculations 2 tick 1"));
    click(&mut ui, "next");
    assert!(
        ui.outline()
            .contains("value 1 value 11 constant calculations 4 tick 1")
    );
}

#[test]
fn reload_invalidates_same_size_constants_and_can_add_or_remove_caches_without_moving_state() {
    let source = r#"
        import { useState } from 'react';
        import { c } from 'react/compiler-runtime';
        const EMPTY=Symbol.for('react.memo_cache_sentinel');
        function useLabel() {
            /*CACHE*/
            const [label]=useState('kept');
            return label;
        }
        export default function App() {
            const [n,setN]=useState(0);
            const label=useLabel(), $=c(1);
            if ($[0]===EMPTY) $[0]='old';
            return <window><button key='next' onClick={()=>setN(n+1)}>{$[0]+' '+label+' '+n}</button></window>;
        }
    "#;
    let path = file(source);
    let entry = rsui_tsx::load(&path, 512 * 1024).unwrap();
    rsui_tsx::keep_state_across_reloads();
    let mut ui = Headless::new(rsui_tsx::root, entry);
    click(&mut ui, "next");
    click(&mut ui, "next");
    assert!(ui.outline().contains("old kept 2"));
    for (constant, extra) in [
        ("new", "const extra=c(3);"),
        ("third", ""),
        ("last", "const extra=c(5);"),
    ] {
        let next = source
            .replace("'old'", &format!("'{constant}'"))
            .replace("/*CACHE*/", extra);
        std::fs::write(&path, next).unwrap();
        assert!(rsui_tsx::request_reload(&path));
        ui.settle(Duration::from_secs(5));
        assert!(
            ui.outline().contains(&format!("{constant} kept 2")),
            "{}",
            ui.outline()
        );
    }
    click(&mut ui, "next");
    assert!(ui.outline().contains("last kept 3"));
}

#[test]
fn suspended_attempts_discard_partial_cache_writes() {
    let mut ui = open(
        r#"
        import { Suspense } from 'react';
        import { c } from 'react/compiler-runtime';
        const EMPTY=Symbol.for('react.memo_cache_sentinel');
        let ready=false, finish;
        const waiting=new Promise(done=>{finish=()=>{ready=true;done()}});
        function Reads() {
            const $=c(1);
            if (!ready) { $[0]='abandoned'; throw waiting; }
            return <text>{$[0]===EMPTY ? 'clean retry' : 'stale '+$[0]}</text>;
        }
        export default function App() {
            return <window><Suspense fallback={<text>waiting</text>}><Reads /></Suspense>
                <button key='finish' onClick={()=>finish()}>Finish</button></window>;
        }
    "#,
    );
    assert!(ui.outline().contains("waiting"));
    click(&mut ui, "finish");
    ui.settle(Duration::from_secs(1));
    assert!(ui.outline().contains("clean retry"), "{}", ui.outline());
    assert!(!ui.outline().contains("stale"));
}

#[test]
fn a_hook_shape_restart_discards_the_first_attempts_cache() {
    let source = r#"
        import { useState, useRef } from 'react';
        import { c } from 'react/compiler-runtime';
        const EMPTY=Symbol.for('react.memo_cache_sentinel');
        let computes=0;
        export default function App() {
            const $=c(1);
            if ($[0]===EMPTY) $[0]=++computes;
            const [label]=useState('old');
            return <window><text>{label+' attempt '+$[0]}</text></window>;
        }
    "#;
    let path = file(source);
    let entry = rsui_tsx::load(&path, 512 * 1024).unwrap();
    rsui_tsx::keep_state_across_reloads();
    let mut ui = Headless::new(rsui_tsx::root, entry);
    assert!(ui.outline().contains("old attempt 1"));
    let changed = source.replace(
        "const [label]=useState('old');",
        "const label=useRef('new').current;",
    );
    std::fs::write(&path, changed).unwrap();
    assert!(rsui_tsx::request_reload(&path));
    ui.settle(Duration::from_secs(5));
    assert!(ui.outline().contains("new attempt 2"), "{}", ui.outline());
}

#[test]
fn replacing_a_component_starts_with_a_fresh_cache() {
    let mut ui = open(
        r#"
        import { useState } from 'react';
        import { c } from 'react/compiler-runtime';
        const EMPTY=Symbol.for('react.memo_cache_sentinel');
        function First() { const $=c(1); if($[0]===EMPTY)$[0]='first'; return <text>{$[0]}</text>; }
        function Second() { const $=c(1); if($[0]===EMPTY)$[0]='second'; return <text>{$[0]}</text>; }
        export default function App() {
            const [other,setOther]=useState(false);
            return <window><button key='swap' onClick={()=>setOther(!other)}>Swap</button>{other?<Second/>:<First/>}</window>;
        }
    "#,
    );
    assert!(ui.outline().contains("first"));
    click(&mut ui, "swap");
    assert!(ui.outline().contains("second"), "{}", ui.outline());
    click(&mut ui, "swap");
    assert!(ui.outline().contains("first"), "{}", ui.outline());
}

#[test]
fn calling_a_compiler_cache_outside_render_is_rejected() {
    let ui = open(
        r#"
        import { c } from 'react/compiler-runtime';
        let message='missing error';
        try { c(1); } catch(error) { message=error.message; }
        export default function App() { return <window><text>{message}</text></window>; }
    "#,
    );
    assert!(
        ui.outline()
            .contains("only be used while a component is rendering")
    );
}
