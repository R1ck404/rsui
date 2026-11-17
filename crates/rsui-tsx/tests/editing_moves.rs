//! Editing observes document movement only while a selection gesture owns it.
mod hands;

use hands::Hands;

fn open(name: &str, source: &str) -> Hands {
    let folder = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-editing-moves-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("app.css"), "body {margin:0;font:16px monospace} p {margin:0;line-height:24px;height:24px}").unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, format!(r#"
        import {{PageWindow,dom}} from 'rsui/web';
        import {{useState,useEffect}} from 'react';
        import './app.css';
        function Status(){{
            const [said,setSaid]=useState('waiting');
            useEffect(()=>{{const timer=setInterval(()=>setSaid('selection:'+getSelection().toString()),10);return()=>clearInterval(timer)}},[]);
            return <dom.p style={{{{position:'fixed',top:160}}}}>{{said}}</dom.p>;
        }}
        {source}
    "#)).unwrap();
    Hands::open(&file, (500, 300))
}

fn document_hears_moves() -> bool {
    rsui::runtime::document_listens() & rsui::runtime::listens::MOVE != 0
}

#[test]
fn an_idle_editing_module_does_not_request_document_moves() {
    let mut ui = open("idle", r#"
        export default function App(){return <PageWindow>
            <dom.div contentEditable style={{height:24,lineHeight:'24px'}}>hello there world</dom.div>
            <Status/>
        </PageWindow>}
    "#);
    ui.wait_for("selection:");
    assert!(!document_hears_moves(), "mounting an editor and reading getSelection must not subscribe to every move");
    for x in [5.0, 40.0, 90.0] {
        ui.move_at(x, 80.0);
        assert!(!document_hears_moves());
    }
}

#[test]
fn editable_selection_receives_held_moves_then_disarms_and_rearms() {
    let mut ui = open("editable", r#"
        export default function App(){return <PageWindow>
            <dom.div contentEditable style={{height:24,lineHeight:'24px'}}>hello there world</dom.div>
            <Status/>
        </PageWindow>}
    "#);
    ui.wait_for("selection:");
    assert!(!document_hears_moves());
    ui.down_at(1.0, 10.0);
    assert!(document_hears_moves(), "accepted editable press arms before its first held move");
    ui.move_at(95.0, 10.0);
    ui.wait_for("selection:hello");
    ui.move_at(155.0, 10.0);
    ui.wait_for("selection:hello there");
    ui.up_at(155.0, 10.0);
    assert!(!document_hears_moves(), "release stops the global movement subscription");
    ui.down_at(1.0, 10.0);
    assert!(document_hears_moves(), "the next accepted press re-arms movement");
    ui.up_at(1.0, 10.0);
    assert!(!document_hears_moves());
}

#[test]
fn prevented_and_nonprimary_presses_do_not_arm_selection_moves() {
    let mut ui = open("rejected", r#"
        export default function App(){return <PageWindow>
            <dom.div contentEditable onMouseDown={event=>event.preventDefault()}
                style={{height:24,lineHeight:'24px'}}>hello there world</dom.div>
            <Status/>
        </PageWindow>}
    "#);
    ui.wait_for("selection:");
    ui.down_at(1.0, 10.0);
    assert!(!document_hears_moves(), "canceled editing default must not arm movement");
    ui.up_at(1.0, 10.0);
    for button in [rsui::PointerButton::Secondary, rsui::PointerButton::Middle] {
        ui.app.button_down(1.0, 10.0, button);
        ui.settle();
        assert!(!document_hears_moves(), "a non-primary press does not select page text");
        ui.app.button_up(1.0, 10.0, button).unwrap();
        ui.settle();
        assert!(!document_hears_moves());
    }
}

#[test]
fn a_zero_button_move_cancels_tracking_without_collapsing_selected_text() {
    let mut ui = open("lost-up", r#"
        function SelectAll(){useEffect(()=>{
            const range=document.createRange();range.selectNodeContents(document.getElementById('page'));
            const selection=getSelection();selection.removeAllRanges();selection.addRange(range);
        },[]);return null}
        export default function App(){return <PageWindow>
            <dom.p id="page">hello there world</dom.p>
            <dom.div contentEditable style={{position:'fixed',top:80}}>editor loads editing</dom.div>
            <SelectAll/><Status/>
        </PageWindow>}
    "#);
    ui.wait_for("selection:hello there world");
    ui.down_at(5.0, 10.0);
    assert!(document_hears_moves());
    // Native capture loss can be observed without the page receiving mouseup.
    // Its next real move has buttons=0 and must not imitate a selected-text click.
    ui.app.cancel_press();
    ui.move_at(20.0, 10.0);
    assert!(!document_hears_moves());
    assert!(ui.says("selection:hello there world"), "{}", ui.app.outline());
    ui.down_at(5.0, 10.0);
    assert!(document_hears_moves());
    ui.up_at(5.0, 10.0);
    assert!(!document_hears_moves());
}

#[test]
fn lazily_upgrading_a_held_page_press_keeps_the_first_selection_move() {
    let mut ui = open("lazy-upgrade", r#"
        export default function App(){const [upgraded,setUpgraded]=useState(false);return <PageWindow>
            <dom.p onMouseMove={event=>{if((event.buttons&1)!==0){getSelection();setUpgraded(true)}}}>hello there world</dom.p>
            {upgraded?<Status/>:<dom.p>not upgraded</dom.p>}
        </PageWindow>}
    "#);
    assert!(ui.says("not upgraded"));
    assert!(!document_hears_moves());
    ui.down_at(1.0, 10.0);
    assert!(document_hears_moves(), "the lightweight deferred press initially owns movement");
    ui.move_at(95.0, 10.0);
    ui.wait_for("selection:hello");
    assert!(document_hears_moves(), "public pagePress must transfer movement ownership to editing");
    ui.up_at(95.0, 10.0);
    assert!(!document_hears_moves(), "up must remove both deferred and editing movement listeners");
}
