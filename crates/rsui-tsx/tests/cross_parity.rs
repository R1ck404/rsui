//! Engine causes found under many shadcn/ui components at once: what's fixed in the
//! window placed and sized by its sides as CSS has it, and transitions moving and telling CSS's own longhands.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn open(name: &str, css: &str, before: &str, jsx: &str, size: (u32, u32)) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-cross-parity-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let css = format!("body {{ margin: 0; }}\n{css}");
    let app = format!(
        "import {{ PageWindow, dom }} from 'rsui/web';\nimport * as React from 'react';\nimport './app.css';\n{before}\nexport default function App() {{ return <PageWindow>{jsx}</PageWindow>; }}\n"
    );
    std::fs::write(folder.join("app.tsx"), app).unwrap();
    std::fs::write(folder.join("app.css"), css).unwrap();
    let mut hands = Hands::open(&folder.join("app.tsx"), size);
    hands.settle();
    hands
}

/// A component that shows what `measure` says, every 50 ms once the page has been laid out.
const SAYS: &str = r#"function Says({ measure }) {
    const [said, setSaid] = React.useState('waiting');
    React.useEffect(() => {
        const timer = setInterval(() => {
            try {
                setSaid('said ' + measure());
            } catch (error) {
                setSaid('threw ' + error.message);
            }
        }, 50);
        return () => clearInterval(timer);
    }, []);
    return <dom.div id="says">{said}</dom.div>;
}"#;

/// CSS 2.1 §10.3.7 and §10.6.4 for `position: fixed`, in the window: both sides and no size stretch it between them (Tailwind's
/// `inset-x-0` on a drawer or a bottom sheet), shares are of the window, `max-width` holds it back from the far side, `auto`
/// margins on both sides put a box with a size in the middle, and one side with no size is as big as what's in it.
#[test]
fn fixed_boxes_are_sized_and_placed_by_their_sides() {
    let before = format!(
        "{SAYS}\nconst measure = () => ['bar', 'mid', 'centred', 'capped', 'corner'].map((id) => {{ const r = document.getElementById(id).getBoundingClientRect(); \
         return [r.x, r.y, r.width, r.height].map(Math.round).join(','); }}).join(' ');"
    );
    let css = "div.f { position: fixed; }
.bar { left: 0; right: 0; bottom: 0; height: 50px; }
.mid { left: 10%; right: 10%; top: 20px; bottom: 20px; }
.centred { left: 0; right: 0; width: 100px; margin: 0 auto; top: 0; height: 10px; }
.capped { left: 0; right: 0; max-width: 200px; top: 30px; height: 10px; }
.corner { right: 8px; bottom: 8px; width: 40px; height: 20px; }";
    let jsx = r#"<dom.div>
        <dom.div id="bar" className="f bar" />
        <dom.div id="mid" className="f mid" />
        <dom.div id="centred" className="f centred" />
        <dom.div id="capped" className="f capped" />
        <dom.div id="corner" className="f corner" />
        <Says measure={measure} />
    </dom.div>"#;
    let mut hands = open("fixed-sides", css, &before, jsx, (400, 300));
    hands.wait_for("said 0,250,400,50 40,20,320,260 150,0,100,10 0,30,200,10 352,272,40,20");
}

/// A transition of `border-color` and `box-shadow` tells each of CSS's longhands that changed, as a browser does: the four
/// `border-*-color`s, and `box-shadow`, which moves as a list (a ring and a shadow from one clear shadow). They start in the order
/// Chromium tells them and end in CSS's composite order, by name.
#[test]
fn transitions_tell_the_longhands_that_move() {
    let before = r#"function Toggle() {
    const [on, setOn] = React.useState(false);
    const [log, setLog] = React.useState([]);
    React.useEffect(() => {
        const hear = (event) => setLog((was) => [...was, event.type.slice('transition'.length) + ':' + event.propertyName]);
        for (const type of ['transitionrun', 'transitionend', 'transitioncancel']) {
            window.addEventListener(type, hear, true);
        }
    }, []);
    return <dom.div>
        <dom.div id="b" className={on ? 'b on' : 'b'} onClick={() => setOn(true)}>go</dom.div>
        <dom.div id="log">{'log ' + log.join(' ')}</dom.div>
    </dom.div>;
}"#;
    let css = ".b { width: 80px; height: 30px; border: 1px solid red; box-shadow: 0 0 0 0 transparent; transition: border-color 60ms linear, box-shadow 60ms linear; }
.b.on { border-color: blue; box-shadow: 0 0 0 3px blue, 0 1px 2px black; }";
    let mut hands = open("transition-longhands", css, before, "<Toggle />", (400, 300));
    hands.app.set_animations(true);
    hands.settle();
    hands.click_at(20.0, 15.0);
    hands.wait_for(
        "log run:border-top-color run:border-bottom-color run:border-left-color run:box-shadow run:border-right-color \
         end:border-bottom-color end:border-left-color end:border-right-color end:border-top-color end:box-shadow",
    );
}
