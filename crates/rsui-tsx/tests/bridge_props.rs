mod hands;
use hands::Hands;
use rsui::core::{Node, PropValue};
use std::path::PathBuf;
fn open() -> Hands {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("bridge-props-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("app.tsx");
    std::fs::write(&file,r#"
import {useState} from 'rsui';
let n=0, reads=0;
const style={height:8};
const mutable={type:'group', key:'mutable', props:{get style(){reads++;return style;}, get height(){reads++;return n%2?12:8;}}, kids:[]};
export default function App(){const [tick,set]=useState(0);return <window>
 <button key='go' onClick={()=>{if(reads!==2)throw Error('reads '+reads); reads=0;n++;style.height=50;set(n);}}>Go</button>
 <group key='rows' height={20} overflow='hidden'>{Array.from({length:200},(_,i)=><group key={i} height={8}/>)}</group>
 {mutable}
 <text key='value'>{tick}</text>
 </window>}
"#).unwrap();
    Hands::open(&file, (300, 160))
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
#[test]
fn shared_native_props_still_read_mutated_js_getters_every_update() {
    let mut ui = open();
    let rows = find(ui.app.runtime().root(), "rows");
    assert_eq!(rows.children.len(), 200);
    let id = rows.children[0].props.identity();
    assert!(rows.children.iter().all(|r| r.props.identity() == id));
    for i in 1..=8 {
        ui.click("go");
        assert_eq!(
            find(ui.app.runtime().root(), "value").name,
            i.to_string().as_str()
        );
        assert_eq!(
            find(ui.app.runtime().root(), "mutable").props.get("height"),
            Some(&PropValue::Float(if i % 2 == 1 { 12.0 } else { 8.0 }))
        );
        if i == 4 {
            rsui_tsx::census();
        }
    }
}
