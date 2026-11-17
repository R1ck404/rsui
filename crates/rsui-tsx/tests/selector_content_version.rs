//! Selector content epochs remain separate from geometry and preserve provenance.
mod hands;

#[test]
fn serialized_epoch_survives_paint_scroll_and_rejects_an_old_content_anchor() {
    let folder = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "tsx-selector-content-version-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("app.css"), "body {margin:0}").unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, r#"
        import {PageWindow,dom} from 'rsui/web';import {useState} from 'react';
        import rsui from 'rsui';import './app.css';
        function check(ok,why){if(!ok)throw new Error(why)}
        let before,stamp;
        export default function App(){const [said,setSaid]=useState('waiting');return <PageWindow>
          <dom.div style={{height:1600}}><dom.div id="target" style={{backgroundColor:'black'}}>old</dom.div></dom.div>
          <dom.button key="capture" style={{position:'fixed',top:220,left:10}} onClick={()=>{
            const host=rsui.__host();before=host.pageTree(true);stamp=before[0][13];
            check(typeof stamp==='number'&&host.pageTreeContentVersion()===stamp,'serialized content epoch');
          }}>capture</dom.button>
          <dom.button key="paint" style={{position:'fixed',top:250,left:10}} onClick={()=>{
            const host=rsui.__host();check(window.scrollY>0,'native scroll applied');
            check(host.pageTreeContentVersion()===stamp,'scroll keeps metadata epoch');
            const fresh=host.pageTree(true);check(fresh[0][13]===stamp,'same content serialization keeps epoch');
            document.getElementById('target').style.backgroundColor='red';
          }}>paint</dom.button>
          <dom.button key="content" style={{position:'fixed',top:280,left:10}} onClick={()=>{
            const host=rsui.__host();check(host.pageTreeContentVersion()===stamp,'pure paint keeps epoch');
            check(host.pageTree(true)[0][13]===stamp,'paint reserialization keeps epoch');
            document.getElementById('target').textContent='new content';
          }}>content</dom.button>
          <dom.button key="verify" style={{position:'fixed',top:310,left:10}} onClick={()=>{
            const host=rsui.__host();check(host.pageTreeContentVersion()===undefined,'changed metadata not certified against old anchor');
            const fresh=host.pageTree(true),next=fresh[0][13];
            check(next>stamp&&host.pageTreeContentVersion()===next,'new content advances epoch');
            check(before[0][13]===stamp,'old serialization keeps its original provenance');
            check(host.pageTree(true)[0][13]===next,'intervening serialization cannot rewrite the old stamp');
            setSaid('content epochs passed');
          }}>verify</dom.button><dom.p style={{position:'fixed',top:190}}>{said}</dom.p>
        </PageWindow>}
    "#).unwrap();
    let mut ui = hands::Hands::open(&file, (500, 350));
    ui.click("capture");
    ui.app.scroll(450.0, 100.0, 3).unwrap();
    ui.settle();
    for button in ["paint", "content", "verify"] {
        ui.click(button);
        assert!(
            !ui.app.outline().contains("Something went wrong"),
            "{}",
            ui.app.outline()
        );
    }
    ui.wait_for("content epochs passed");
}
