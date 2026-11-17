mod hands;
use hands::Hands;
use rsui::core::Node;
use std::path::PathBuf;
fn open(label: &str, source: &str) -> Hands {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("bridge-nodes-{}-{label}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    Hands::open(&file, (400, 400))
}
fn find<'a>(node: &'a Node, key: &str) -> &'a Node {
    let mut path = Vec::new();
    assert!(hands::path_of(node, key, &mut path));
    let mut node = node;
    for i in path {
        node = &node.children[i as usize];
    }
    node
}
fn same(a: &Node, b: &Node) -> bool {
    std::ptr::eq(&**a, &**b)
}
#[test]
fn reused_leaf_registers_the_new_handler_and_reads_current_props() {
    let mut ui = open(
        "handlers",
        r#"import {useState} from 'react';
 export default function App(){const [n,set]=useState(0);return <window><button key='go' onClick={()=>set(n+1)}>Next</button><group key='stable' height={8}/><group key='changed' height={8+n}/><text key='value'>{n}</text></window>}"#,
    );
    let old = ui.app.runtime().root().clone();
    for i in 1..=6 {
        ui.click("go");
        let root = ui.app.runtime().root();
        assert!(same(find(&old, "stable"), find(root, "stable")));
        assert!(same(find(&old, "go"), find(root, "go")));
        assert!(!same(find(&old, "changed"), find(root, "changed")));
        assert_eq!(find(root, "value").name, i.to_string().as_str());
        if i == 3 {
            rsui_tsx::census();
        }
    }
    assert_eq!(find(&old, "value").name, "0");
}
#[test]
fn host_and_component_transitions_keep_state_and_event_ownership() {
    let mut ui = open(
        "owners",
        r#"import {useState} from 'react';
 function Child(){const [v,set]=useState(0);return <button key='child-output' onClick={()=>set(v+1)}>{'child '+v}</button>}
 export default function App(){const [n,set]=useState(0);return <window><button key='go' onClick={()=>set(n+1)}>Next</button>{n%2?<button key='new-host'>child 0</button>:null}<Child key='instance'/><text>{n}</text></window>}"#,
    );
    for _ in 0..3 {
        ui.click("child-output");
        ui.click("go");
    }
    assert!(ui.says("child 3"));
    ui.click("child-output");
    assert!(ui.says("child 4"));
}
#[test]
fn getter_mutation_and_reordered_fragments_do_not_reuse_stale_leaves() {
    let mut ui = open(
        "mutable",
        r#"import {useState} from 'react';let n=0,reads=0;const kids=['a'];const leaf={type:'text',key:'mutable',props:{get color(){reads++;kids[0]=n%2?'b':'a';return '#123456';}},kids};
 export default function App(){const [tick,set]=useState(0);return <window><button key='go' onClick={()=>{if(reads!==1)throw Error('reads '+reads);reads=0;n++;set(n);}}>Next</button>{tick%2?<><text key='other'>other</text>{leaf}</>:<>{leaf}<text key='other'>other</text></>}</window>}"#,
    );
    for i in 1..=6 {
        ui.click("go");
        assert_eq!(
            find(ui.app.runtime().root(), "mutable").name,
            if i % 2 == 1 { "b" } else { "a" }
        );
    }
}
