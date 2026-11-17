mod hands;
use hands::Hands;
use rsui::core::{Node, Role};
use std::path::PathBuf;

fn open(label: &str, source: &str) -> Hands {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("bridge-tags-{}-{label}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    Hands::open(&file, (400, 400))
}
fn find<'a>(node: &'a Node, key: &str) -> &'a Node {
    let mut path = Vec::new();
    assert!(hands::path_of(node, key, &mut path), "missing {key}");
    let mut node = node;
    for i in path {
        node = &node.children[i as usize];
    }
    node
}

#[test]
fn native_tags_preserve_roles_and_text_ownership() {
    let ui = open(
        "roles",
        r#"
const tags=['window','group','row','column','text','span','button','link','input','checkbox','item','image','dialog','menu','menuitem'];
export default function App(){return <window>{tags.map(type=>({type,key:type,props:{},kids:['label']}))}{ {type:null,key:'default',props:{},kids:[]} }{ {type:'custom-\u03bb',key:'custom',props:{},kids:['child']} }</window>}
"#,
    );
    let root = ui.app.runtime().root();
    for (tag, role) in [
        ("window", Role::Window),
        ("group", Role::Group),
        ("row", Role::Group),
        ("column", Role::Group),
        ("text", Role::Text),
        ("span", Role::Text),
        ("button", Role::Button),
        ("link", Role::Link),
        ("input", Role::TextInput),
        ("checkbox", Role::Checkbox),
        ("item", Role::ListItem),
        ("image", Role::Image),
        ("dialog", Role::Dialog),
        ("menu", Role::Menu),
        ("menuitem", Role::MenuItem),
        ("default", Role::Group),
    ] {
        assert_eq!(find(root, tag).role, role, "{tag}");
    }
    for tag in [
        "text", "span", "button", "link", "checkbox", "item", "menuitem",
    ] {
        let node = find(root, tag);
        assert_eq!(node.name, "label", "{tag}");
        assert!(node.children.is_empty(), "{tag}");
    }
    assert!(matches!(find(root, "custom").role, Role::Custom(_)));
    assert_eq!(find(root, "custom").children[0].name, "child");
    assert_eq!(find(root, "group").children[0].name, "label");
}

#[test]
fn ambiguous_tags_keep_disambiguation_getter_order_on_every_update() {
    let mut ui = open(
        "getters",
        r#"
import {useState} from 'react'; let reads=[];
const tags=['group','span','button','input','dialog','image','custom-tag'];
const nodes=tags.map(type=>{const props={height:8};Object.defineProperty(props,'className',{get(){reads.push(type+':c');return undefined;}});Object.defineProperty(props,'type',{get(){reads.push(type+':t');return undefined;}});return {type,key:type,props,kids:[]};});
export default function App(){const [n,set]=useState(0);return <window><button key='go' onClick={()=>{const want='span:c,button:c,button:t,input:c,input:t,dialog:c,dialog:t';if(reads.join(',')!==want)throw Error(reads.join(','));reads=[];set(n+1);}}>Next</button>{nodes}<text key='value'>{n}</text></window>}
"#,
    );
    for n in 1..=5 {
        ui.click("go");
        assert_eq!(
            find(ui.app.runtime().root(), "value").name,
            n.to_string().as_str()
        );
        if n == 3 {
            rsui_tsx::census();
        }
    }
}

#[test]
fn tag_changes_reset_child_state_but_same_tag_updates_preserve_it() {
    let mut ui = open(
        "scope",
        r#"
import {useState} from 'react';
function Child(){const [n,set]=useState(0);return <button key='child' onClick={()=>set(n+1)}>{'child '+n}</button>}
export default function App(){const [tag,setTag]=useState(0);const [tick,setTick]=useState(0);const tags=['group','row','column','custom-\u03bb','custom-\u03c9'];return <window><button key='swap' onClick={()=>setTag((tag+1)%tags.length)}>Swap</button><button key='tick' onClick={()=>setTick(tick+1)}>Tick</button>{ {type:tags[tag],key:'host',props:{},kids:[<Child key='counter'/>]} }<text>{tick}</text></window>}
"#,
    );
    for _ in 0..6 {
        ui.click("child");
        assert_eq!(find(ui.app.runtime().root(), "child").name, "child 1");
        ui.click("tick");
        assert_eq!(find(ui.app.runtime().root(), "child").name, "child 1");
        ui.click("swap");
        assert_eq!(find(ui.app.runtime().root(), "child").name, "child 0");
    }
}
