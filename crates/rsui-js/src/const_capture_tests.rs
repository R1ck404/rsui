use crate::Engine;
use std::{cell::RefCell, rc::Rc};

fn run(source: &str) -> (String, Engine) {
    let output = Rc::new(RefCell::new(String::new()));
    let sink = output.clone();
    let mut engine = Engine::new();
    #[cfg(all(feature = "jit", windows, target_arch = "x86_64"))]
    {
        engine.vm.jit.threshold = 2;
    }
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    engine.eval(source, "const-captures.js").unwrap();
    engine.run_until_idle().unwrap();
    let text = output.borrow().trim_end().to_owned();
    (text, engine)
}

fn captures(engine: &mut Engine, name: &str) -> Vec<bool> {
    let name = engine.vm.intern_str(name);
    engine
        .vm
        .protos
        .iter()
        .flat_map(|proto| {
            proto
                .capture_names
                .iter()
                .zip(&proto.captures)
                .filter_map(|(found, capture)| (*found == name).then_some(capture.by_value))
        })
        .collect()
}

#[test]
fn initialized_const_values_escape_without_cells_and_keep_object_identity() {
    let (text, mut engine) = run(r#"
        function make(n) { const value = {n}; return [() => value.n, v => value.n=v, () => { try { value=3 } catch(e) { return e.name } }]; }
        const a=make(4); a[1](9); console.log(a[0](), a[2]());
        function nested(n) { const held=n; return () => () => held; }
        console.log(nested(8)()());
    "#);
    assert_eq!(text, "9 TypeError\n8");
    assert!(
        captures(&mut engine, "value")
            .iter()
            .all(|by_value| *by_value)
    );
    assert!(!captures(&mut engine, "value").is_empty());
    assert!(
        captures(&mut engine, "held")
            .iter()
            .all(|by_value| *by_value)
    );
}

#[test]
fn earlier_hoisted_self_recursive_and_pattern_captures_keep_live_tdz_cells() {
    let (text, mut engine) = run(r#"
        function early() { const read=()=>late; let before; try { read() } catch(e) { before=e.name; } const late=7; return before+' '+read(); }
        function hoisted() { let before; try { read() } catch(e) { before=e.name; } const late=8; function read(){return late} return before+' '+read(); }
        function recursive() { const self=()=>self; return self()===self; }
        function pattern() { const {read=()=>late}={}; const late=9; return read(); }
        function classes() { const C=class { read(){ return late; } }; const late=10; return new C().read(); }
        function arrayPattern() { const [read=()=>late]=[]; const late=11; return read(); }
        function computedPattern() { try { const {[(()=>late)()]: value}={}; const late=12; } catch(e) { return e.name; } }
        console.log(early(), hoisted(), recursive(), pattern(), classes(), arrayPattern(), computedPattern());
    "#);
    assert_eq!(
        text,
        "ReferenceError 7 ReferenceError 8 true 9 10 11 ReferenceError"
    );
    assert!(
        captures(&mut engine, "late")
            .iter()
            .all(|by_value| !*by_value)
    );
    assert!(
        captures(&mut engine, "self")
            .iter()
            .all(|by_value| !*by_value)
    );
}

#[test]
fn block_shadowing_loop_instances_and_dynamic_scope_preserve_bindings() {
    let (text, mut engine) = run(r#"
        function loops() { const list=[]; for(let i=0;i<4;i++){const local=i;list.push(()=>local)} for(const item of [4,5])list.push(()=>item); return list.map(f=>f()).join(','); }
        function shadow(){const x=1; const a=()=>x; {const x=2; const b=()=>x; return a()+' '+b();}}
        function dynamic(){const dynamicValue=3; return o=>{with(o){return dynamicValue}}}
        function evaluated(){const evaluatedValue=4; return ()=>{eval('1');return evaluatedValue}}
        console.log(loops(), shadow(), dynamic()({dynamicValue:12}), dynamic()({}), evaluated()());
    "#);
    assert_eq!(text, "0,1,2,3,4,5 1 2 12 3 4");
    assert!(
        captures(&mut engine, "local")
            .iter()
            .all(|by_value| *by_value)
    );
    assert!(
        captures(&mut engine, "dynamicValue")
            .iter()
            .all(|by_value| !*by_value)
    );
    assert!(
        captures(&mut engine, "evaluatedValue")
            .iter()
            .all(|by_value| !*by_value)
    );
}

#[test]
fn value_captures_survive_gc_and_repeated_jit_calls() {
    let (_, mut engine) = run(r#"
        function make(n){const kept={n};return ()=>kept.n;}
        globalThis.saved=[];for(let i=0;i<5000;i++)saved.push(make(i));
        if(typeof gc==='function')gc();
    "#);
    engine.collect_garbage();
    engine.eval("if(saved[0]()!==0||saved[4999]()!==4999)throw new Error('lost capture'); for(let i=0;i<5000;i++)if(saved[i]()!==i)throw new Error('changed capture');", "after-gc.js").unwrap();
    assert!(
        captures(&mut engine, "kept")
            .iter()
            .all(|by_value| *by_value)
    );
}

#[test]
fn deep_capture_analysis_falls_back_to_cells() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let source = format!("function make(){{const deepValue=7;const unused={}true;return()=>deepValue}}if(make()()!==7)throw new Error('changed capture');", "!".repeat(80));
            let mut engine = Engine::new();
            engine.set_stack_limit(12 * 1024 * 1024);
            engine.eval(&source, "deep-captures.js").unwrap();
            let found = captures(&mut engine, "deepValue");
            assert!(!found.is_empty());
            assert!(found.iter().all(|by_value| !*by_value));
        })
        .unwrap()
        .join()
        .unwrap();
}
