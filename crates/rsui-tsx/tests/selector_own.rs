//! Shared structural accessors preserve selectors across native and DOM updates.
mod hands;

fn open(name: &str, body: &str) -> hands::Hands {
    let folder=std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-selector-own-{}-{name}",std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("app.css"),"body {margin:0}").unwrap();
    let file=folder.join("app.tsx");
    std::fs::write(&file,format!(r#"
        import {{PageWindow,dom}} from 'rsui/web'; import {{useState}} from 'react';
        import {{createPortal}} from 'react-dom'; import './app.css';
        function check(ok,why){{if(!ok)throw new Error(why)}}
        function ids(selector){{return Array.from(document.querySelectorAll(selector),x=>x.id).join(',')}}
        {body}
    "#)).unwrap();
    hands::Hands::open(&file,(500,350))
}

#[test]
fn mixed_type_nth_previous_siblings_and_relational_queries() {
    let mut ui=open("structural",r#"
        export default function App(){const [said,setSaid]=useState('waiting');return <PageWindow>
        <dom.div id="rows"><dom.div id="a" className="row marked">a</dom.div><dom.span id="b" className="row">b</dom.span>
        <dom.div id="c" className="row marked">c</dom.div><dom.div id="d" className="row">d</dom.div></dom.div>
        <dom.button key="verify" onClick={()=>{
          check(ids('#rows > :nth-child(2)')==='b','nth child');
          check(ids('#rows > div:nth-of-type(2)')==='c','nth of type');
          check(ids('#rows > :nth-last-child(2)')==='c','nth last child');
          check(ids('#rows > :nth-child(2 of .marked)')==='c','filtered nth child');
          check(ids('#a + .row')==='b','adjacent');check(ids('#a ~ .marked')==='c','previous siblings');
          check(ids('#rows:has(> .marked + span)')==='rows','relative has');
          check(ids('#rows > :first-child')==='a' && ids('#rows > :last-child')==='d','ends');
          setSaid('structural passed');
        }}>verify</dom.button><dom.p>{said}</dom.p></PageWindow>}
    "#);
    ui.click("verify");ui.wait_for("structural passed");
}

#[test]
fn retained_handles_read_new_structural_positions_after_a_keyed_native_patch() {
    let mut ui=open("native-patch",r#"
        export default function App(){const [on,setOn]=useState(false),[said,setSaid]=useState('waiting');return <PageWindow>
        <dom.div id="rows">{(on?['c','a','b']:['a','b','c']).map(id=><dom.div key={id} id={id} className="row">{id}</dom.div>)}</dom.div>
        <dom.button key="change" onClick={()=>{window.kept=document.querySelector('#a');
          check(window.kept.matches(':first-child'),'first read');check(ids('#a + .row')==='b','old next');setOn(true)}}>change</dom.button>
        <dom.button key="verify" onClick={()=>{
          check(window.kept===document.querySelector('#a'),'same element after reorder');
          check(window.kept.matches(':nth-child(2)'),'new read uses current index');
          check(!window.kept.matches(':first-child'),'old index gone');
          check(ids('#c + #a + #b')==='b','new previous sibling chain');
          check(ids('#rows > :nth-of-type(3)')==='b','new type index');setSaid('patch passed');
        }}>verify</dom.button><dom.p>{said}</dom.p></PageWindow>}
    "#);
    ui.click("change");ui.click("verify");ui.wait_for("patch passed");
}

#[test]
fn imperative_attributes_text_and_dom_order_refresh_structural_descriptors() {
    let mut ui=open("imperative",r#"
        export default function App(){const [said,setSaid]=useState('waiting');return <PageWindow>
        <dom.div id="rows"><dom.div id="a" className="row">old</dom.div><dom.div id="b" className="row"/><dom.div id="c" className="row"/></dom.div>
        <dom.button key="change" onClick={()=>{const a=document.querySelector('#a'),rows=document.querySelector('#rows');
          check(a.matches(':first-child'),'prime old descriptor');a.setAttribute('data-hot','yes');
          a.classList.add('marked');a.textContent='changed';rows.appendChild(a);
        }}>change</dom.button>
        <dom.button key="verify" onClick={()=>{
          check(ids('#rows > :nth-child(3)')==='a','imperative reorder');
          check(ids('#c + .marked[data-hot=yes]')==='a','new attrs and previous sibling');
          check(document.querySelector('#a').textContent==='changed','new text');
          check(ids('#rows > :empty')==='b,c','empty metadata');
          check(ids('#rows:has(> #c + #a:not(:empty))')==='rows','relative new text and order');setSaid('imperative passed');
        }}>verify</dom.button><dom.p>{said}</dom.p></PageWindow>}
    "#);
    ui.click("change");ui.click("verify");ui.wait_for("imperative passed");
}

#[test]
fn portal_order_and_reparenting_refresh_nth_and_previous_sibling_reads() {
    let mut ui=open("portal",r#"
        export default function App(){const [on,setOn]=useState(false),[said,setSaid]=useState('waiting');
        const moving=<dom.span key="moving" id="moving">moving</dom.span>;
        return <PageWindow><dom.div id="left">{on?null:moving}</dom.div><dom.div id="right"><dom.span id="fixed">fixed</dom.span>{on?moving:null}</dom.div>
        {createPortal(<dom.div id="portal">{(on?['q','p']:['p','q']).map(id=><dom.span key={id} id={id}>{id}</dom.span>)}</dom.div>,document.body)}
        <dom.button key="change" onClick={()=>{check(ids('#left > :only-child')==='moving','initial parent');
          check(ids('#portal > :first-child')==='p','initial portal order');setOn(true)}}>change</dom.button>
        <dom.button key="verify" onClick={()=>{check(ids('#left > #moving')==='','old parent empty');
          check(ids('#right > #fixed + #moving:nth-child(2)')==='moving','new parent and sibling');
          check(ids('#portal > #q + #p:nth-of-type(2)')==='p','new portal sibling and type order');
          check(document.querySelector('#portal').parentElement===document.body,'portal remains in body');setSaid('portal passed');
        }}>verify</dom.button><dom.p>{said}</dom.p></PageWindow>}
    "#);
    ui.click("change");ui.click("verify");ui.wait_for("portal passed");
}
