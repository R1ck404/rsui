//! Pointer and mouse coordinates use document space even when the page has scrolled.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn open(name: &str, before: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "tsx-pointer-coordinates-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("app.css"), "body { margin: 0; }").unwrap();
    let source = format!(
        "import {{ PageWindow, dom }} from 'rsui/web';\nimport * as React from 'react';\nimport './app.css';\n{before}\nexport default function App() {{ return <PageWindow><Page /></PageWindow>; }}"
    );
    std::fs::write(folder.join("app.tsx"), source).unwrap();
    Hands::open(&folder.join("app.tsx"), (500, 400))
}

#[test]
fn pointer_and_compatibility_mouse_events_include_document_scroll() {
    let mut hands = open(
        "document-scroll",
        r#"
        function Page() {
            const [pointer, setPointer] = React.useState('waiting');
            const [mouse, setMouse] = React.useState('waiting');
            const point = e => [e.clientX,e.clientY,e.pageX,e.pageY].join(',');
            return <dom.div style={{width:2000,height:2000}}>
                <dom.button key="go" style={{position:'fixed',top:10,left:10}}
                    onClick={() => window.scrollTo(300,300)}>Scroll</dom.button>
                <dom.div key="hit" style={{position:'absolute',left:350,top:350,width:100,height:100}}
                    onPointerMove={e=>setPointer(point(e))} onMouseMove={e=>setMouse(point(e))}>Target</dom.div>
                <dom.p style={{position:'fixed',top:200,left:10}}>{'pointer '+pointer+' mouse '+mouse}</dom.p>
            </dom.div>;
        }
        "#,
    );
    hands.click("go");
    hands.move_at(75.0, 75.0);
    assert!(
        hands.says("pointer 75,75,375,375 mouse 75,75,375,375"),
        "{}",
        hands.app.outline()
    );
}

#[test]
fn a_native_events_document_coordinates_are_stable_through_propagation() {
    let mut hands = open(
        "stable-during-dispatch",
        r#"
        function Page() {
            const [said,setSaid] = React.useState('waiting');
            React.useEffect(()=>{
                const move = e => {
                    if (e.target.id === 'hit') window.scrollTo(0,0);
                };
                document.addEventListener('pointermove',move,true);
                return ()=>document.removeEventListener('pointermove',move,true);
            },[]);
            return <dom.div style={{height:2000}}>
                <dom.button key="go" style={{position:'fixed',top:10,left:10}}
                    onClick={() => window.scrollTo(0,300)}>Scroll</dom.button>
                <dom.div id="hit" key="hit" style={{position:'absolute',left:50,top:350,width:100,height:100}}
                    onPointerMove={e=>setSaid([e.clientY,e.pageY].join(','))}>Target</dom.div>
                <dom.p style={{position:'fixed',top:200,left:10}}>{'point '+said}</dom.p>
            </dom.div>;
        }
        "#,
    );
    hands.click("go");
    hands.move_at(75.0, 75.0);
    assert!(hands.says("point 75,375"), "{}", hands.app.outline());
}

#[test]
fn pointer_listeners_added_during_dispatch_hear_the_remaining_phases() {
    let mut hands = open(
        "listeners-during-dispatch",
        r#"
        function Page() {
            const [said,setSaid] = React.useState('waiting');
            React.useEffect(()=>{
                const seen = [];
                let first;
                const late = e => {
                    seen.push(e.type+':'+e.eventPhase+':'+(e === first));
                    setSaid(seen.join(' '));
                };
                const capture = e => {
                    if (e.target.id !== 'hit') return;
                    first = e;
                    seen.push(e.type+':'+e.eventPhase);
                    e.target.addEventListener('pointermove',late,{once:true});
                    document.addEventListener('pointermove',late,{once:true});
                    window.addEventListener('mousemove',late,{once:true});
                };
                document.addEventListener('pointermove',capture,{capture:true,once:true});
            },[]);
            return <dom.div><dom.div id="hit" key="hit" style={{width:200,height:100}}>Target</dom.div>
                <dom.p>{'events '+said}</dom.p></dom.div>;
        }
        "#,
    );
    hands.move_at(50.0, 50.0);
    assert!(hands.says("events pointermove:1 pointermove:2:true pointermove:3:true mousemove:3:false"), "{}", hands.app.outline());
}

#[test]
fn a_bubble_only_document_listener_still_hears_a_fresh_native_move() {
    let mut hands = open(
        "bubble-only",
        r#"
        function Page() {
            const [said,setSaid] = React.useState('waiting');
            React.useEffect(()=>{
                const heard = e => setSaid([e.type,e.eventPhase,e.target.id,e.clientX,e.clientY].join(':'));
                document.addEventListener('pointermove',heard);
                return ()=>document.removeEventListener('pointermove',heard);
            },[]);
            return <dom.div><dom.div id="hit" key="hit" style={{width:200,height:100}}>Target</dom.div>
                <dom.p>{'event '+said}</dom.p></dom.div>;
        }
        "#,
    );
    hands.move_at(50.0, 50.0);
    assert!(hands.says("event pointermove:3:hit:50:50"), "{}", hands.app.outline());
}

#[test]
fn capture_microtask_prevention_reaches_an_unobserved_phase_before_the_mouse_event() {
    let mut hands = open(
        "capture-microtask-prevention",
        r#"
        function Page() {
            const [said,setSaid] = React.useState('waiting');
            React.useEffect(()=>{
                const down = e => {
                    if (e.target.id !== 'hit') return;
                    queueMicrotask(()=>{e.preventDefault();setSaid('prevented');});
                };
                const mouse = e => {
                    if (e.target.id === 'hit') setSaid('unexpected compatibility mouse');
                };
                document.addEventListener('pointerdown',down,true);
                document.addEventListener('mousedown',mouse,true);
                return ()=>{
                    document.removeEventListener('pointerdown',down,true);
                    document.removeEventListener('mousedown',mouse,true);
                };
            },[]);
            // There are no element or bubbling pointerdown listeners: the
            // following phases must still consume the capture checkpoint's flags.
            return <dom.div><dom.button id="hit" key="hit" style={{width:160,height:50}}>Target</dom.button>
                <dom.p>{'status '+said}</dom.p></dom.div>;
        }
        "#,
    );
    let (x, y) = hands.middle_of("hit");
    hands.down_at(x, y);
    assert!(hands.says("status prevented"), "{}", hands.app.outline());
    assert!(hands.app.focused().is_none(), "prevented press must not move focus");
    hands.up_at(x, y);
}

fn captured_moves(name: &str) -> Hands {
    open(
        name,
        r#"
        function Page() {
            const [moves,setMoves]=React.useState('none');
            const [up,setUp]=React.useState('none');
            const [lost,setLost]=React.useState('none');
            React.useEffect(()=>{
                const gone=e=>setLost(e.target.id);
                window.addEventListener('lostpointercapture',gone);
                return ()=>window.removeEventListener('lostpointercapture',gone);
            },[]);
            return <dom.div>
                <dom.div id="grab" key="grab" style={{width:100,height:40}}
                    onPointerDown={e=>{e.currentTarget.setPointerCapture(e.pointerId);e.preventDefault();}}
                    onPointerMove={e=>{if(e.buttons)setMoves(e.target.id+':'+e.clientX+':'+e.buttons+':'+e.target.hasPointerCapture(e.pointerId));}}
                    onPointerUp={e=>setUp(e.target.id+':'+e.buttons+':'+e.target.hasPointerCapture(e.pointerId))}>Grab</dom.div>
                <dom.div key="other" style={{width:300,height:100}}>Other</dom.div>
                <dom.p>{'moves '+moves+' up '+up+' lost '+lost}</dom.p>
            </dom.div>;
        }
    "#,
    )
}

#[test]
fn explicit_capture_delivers_intermediate_moves_for_each_mouse_button() {
    for (name, button, mask) in [
        ("primary", rsui::PointerButton::Main, 1),
        ("secondary", rsui::PointerButton::Secondary, 2),
        ("middle", rsui::PointerButton::Middle, 4),
    ] {
        let mut hands = captured_moves(name);
        hands.app.pointer_move(20.0, 20.0);
        hands.app.button_down(20.0, 20.0, button);
        hands.settle();
        hands.move_at(200.0, 80.0);
        assert!(
            hands.says(&format!("moves grab:200:{mask}:true")),
            "{name}: {}",
            hands.app.outline()
        );
        hands.move_at(250.0, 90.0);
        assert!(
            hands.says(&format!("moves grab:250:{mask}:true")),
            "{name}: {}",
            hands.app.outline()
        );
        hands.app.button_up(250.0, 90.0, button).unwrap();
        hands.settle();
        assert!(
            hands.says("up grab:0:true lost grab"),
            "{name}: {}",
            hands.app.outline()
        );
    }
}

#[test]
fn paced_capture_publishes_a_waiting_move_before_pointer_release() {
    use std::time::Duration;
    let mut hands = captured_moves("paced");
    hands.app.set_move_pace(Duration::from_millis(5));
    hands.down_at(20.0, 20.0);
    hands.move_at(200.0, 80.0);
    std::thread::sleep(Duration::from_millis(8));
    hands.app.wake();
    hands.settle();
    assert!(
        hands.says("moves grab:200:1:true"),
        "{}",
        hands.app.outline()
    );
    hands.move_at(250.0, 90.0);
    std::thread::sleep(Duration::from_millis(8));
    hands.app.wake();
    hands.settle();
    assert!(
        hands.says("moves grab:250:1:true"),
        "{}",
        hands.app.outline()
    );
    hands.up_at(250.0, 90.0);
}

#[test]
fn animation_frames_publish_paced_moves_without_waiting_for_a_timer() {
    use std::time::{Duration, Instant};
    let mut hands = captured_moves("animation-paced");
    // The first move is delivered immediately. The next move must be delivered by the animation frame itself: Windows
    // processes continuously invalidated WM_PAINT before WM_TIMER, so an animating window may never get a timer wake.
    hands.app.set_move_pace(Duration::from_millis(5));
    hands.down_at(20.0, 20.0);
    hands.move_at(200.0, 80.0);
    std::thread::sleep(Duration::from_millis(8));
    hands.app.tick(Instant::now());
    hands.settle();
    assert!(
        hands.says("moves grab:200:1:true"),
        "{}",
        hands.app.outline()
    );
    hands.move_at(250.0, 90.0);
    std::thread::sleep(Duration::from_millis(8));
    hands.app.tick(Instant::now());
    hands.settle();
    assert!(
        hands.says("moves grab:250:1:true"),
        "{}",
        hands.app.outline()
    );
    hands.up_at(250.0, 90.0);
}
