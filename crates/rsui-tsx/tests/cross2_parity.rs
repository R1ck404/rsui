//! Engine causes under many shadcn/ui components: CSS `order` laid out from each item's own style, container
//! queries, constraint validation as CSS sees it, and `:has()` read from code as the page is now.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn open(name: &str, css: &str, before: &str, jsx: &str, size: (u32, u32)) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-cross2-parity-{}-{name}", std::process::id()));
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

/// CSS Flexbox §5.4: `order` places a flex item among the others from its own style, whatever made it, so a component's
/// `order-first` (shadcn's InputGroupAddon after its input) goes first, while the DOM keeps the order it's written in.
#[test]
fn order_lays_out_a_components_element_from_its_own_style() {
    let before = format!(
        "{SAYS}\nfunction Addon() {{ return <dom.span id=\"a\" className=\"first\" />; }}\n\
         const measure = () => {{ const x = (id) => Math.round(document.getElementById(id).getBoundingClientRect().x); \
         return [x('a'), x('b'), x('g1'), x('g2'), document.getElementById('row').firstElementChild.id].join(','); }};"
    );
    let css = ".row { display: flex; } .grid { display: grid; grid-template-columns: 40px 40px; }
.first { order: -1; width: 20px; height: 5px; } .b { width: 30px; height: 5px; } .last { order: 2; }";
    let jsx = r#"<dom.div>
        <dom.div id="row" className="row"><dom.span id="b" className="b" /><Addon /></dom.div>
        <dom.div className="grid"><dom.span id="g1" className="last" /><dom.span id="g2" /></dom.div>
        <Says measure={measure} />
    </dom.div>"#;
    let mut hands = open("order", css, &before, jsx, (300, 200));
    hands.wait_for("said 0,20,40,0,b");
}

/// CSS Containment 3 §7: `@container name (width >= ...)` holds by the size its nearest container of that name is laid out at,
/// which is looked at again after layout; a container that's too narrow, or not of that name, doesn't make it hold.
#[test]
fn container_queries_follow_the_containers_laid_out_size() {
    let before = format!(
        "{SAYS}\nconst measure = () => ['k1', 'k2', 'k3'].map((id) => Math.round(document.getElementById(id).getBoundingClientRect().width)).join(',');"
    );
    let css = ".c { container-type: inline-size; container-name: box; } .other { container: other / inline-size; width: 400px; }
.k { width: 11px; height: 5px; }
@container box (width >= 300px) { .k { width: 33px; } }";
    let jsx = r#"<dom.div>
        <dom.div className="c" style={{ width: 400 }}><dom.div id="k1" className="k" /></dom.div>
        <dom.div className="c" style={{ width: 200 }}><dom.div id="k2" className="k" /></dom.div>
        <dom.div className="other"><dom.div id="k3" className="k" /></dom.div>
        <Says measure={measure} />
    </dom.div>"#;
    let mut hands = open("container", css, &before, jsx, (500, 200));
    hands.wait_for("said 33,11,11");
}

/// HTML §4.10.21 as CSS sees it: `:invalid` and `:valid` follow what a field holds and `setCustomValidity()`, `:required` and
/// `:optional` are only about fields, and `:user-invalid` only holds once the form has been tried and not sent.
#[test]
fn validity_pseudo_classes_follow_the_field() {
    let before = format!(
        "{SAYS}\nconst measure = () => {{ const e = document.getElementById('e'); const t = document.getElementById('t'); \
         return [getComputedStyle(e).color, getComputedStyle(t).color, e.matches(':user-invalid'), document.getElementById('d').matches(':optional'), \
         document.getElementById('f').matches(':invalid')].join('|'); }};\n\
         setTimeout(() => {{ document.getElementById('t').setCustomValidity('no'); document.getElementById('f').requestSubmit(); }}, 300);"
    );
    let css = "input:invalid { color: rgb(255, 0, 0); } input:valid { color: rgb(0, 128, 0); }";
    let jsx = r#"<dom.form id="f">
        <dom.input id="e" required defaultValue="" />
        <dom.input id="t" defaultValue="filled" />
        <dom.div id="d" />
        <Says measure={measure} />
    </dom.form>"#;
    let mut hands = open("validity", css, &before, jsx, (300, 200));
    hands.wait_for("said rgb(255, 0, 0)|rgb(255, 0, 0)|true|false|true");
}

/// A `:has()` about what a component renders, read with `getComputedStyle()` straight after the change is committed (in a layout
/// effect), says what the page is now, as a browser does, both for the element and for a rule about what's inside it.
#[test]
fn has_about_a_components_output_is_current_when_read() {
    let before = r#"function Kid({ on }) { return <dom.span className="kid" data-on={on ? 'yes' : undefined}>x</dom.span>; }
function Box() {
    const [on, setOn] = React.useState(false);
    const [said, setSaid] = React.useState('');
    React.useLayoutEffect(() => {
        if (said !== '' || !on) {
            return;
        }
        const box = document.getElementById('box');
        setSaid('read ' + getComputedStyle(box).color + '|' + getComputedStyle(document.getElementById('tail')).backgroundColor);
    }, [on]);
    React.useEffect(() => {
        const timer = setTimeout(() => setOn(true), 300);
        return () => clearTimeout(timer);
    }, []);
    return <dom.div><dom.div id="box" className="box"><Kid on={on} /><dom.div id="tail" className="tail" /></dom.div><dom.div>{said}</dom.div></dom.div>;
}"#;
    let css = ".box { color: rgb(0, 0, 255); } .box:has(> .kid[data-on=yes]) { color: rgb(255, 0, 0); }
.box:has(> .kid[data-on=yes]) .tail { background-color: rgb(0, 128, 0); } .tail { height: 5px; }";
    let mut hands = open("has-now", css, before, "<Box />", (300, 200));
    hands.wait_for("read rgb(255, 0, 0)|rgb(0, 128, 0)");
}
