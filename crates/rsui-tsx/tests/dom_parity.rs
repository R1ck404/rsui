//! What rsui/web did differently from a browser in the DOM, events, focus and geometry, each as small
//! as it comes.

mod hands;

use std::path::PathBuf;

use hands::Hands;
use rsui::KeyCode;

fn open(name: &str, css: &str, before: &str, jsx: &str, size: (u32, u32)) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-dom-parity-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let css = format!("body {{ margin: 0; }}\n{css}");
    let app = format!(
        "import {{ PageWindow, dom }} from 'rsui/web';\nimport * as React from 'react';\nimport {{ createPortal }} from 'react-dom';\nimport './app.css';\n{before}\nexport default function App() {{ return <PageWindow>{jsx}</PageWindow>; }}\n"
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
    const tell = () => {
        try {
            setSaid('said ' + measure());
        } catch (error) {
            setSaid('threw ' + error.message);
        }
    };
    React.useEffect(() => {
        const timer = setInterval(tell, 50);
        return () => clearInterval(timer);
    }, []);
    return <dom.div id="says">{said}</dom.div>;
}"#;

/// A portal into `body` is at the end of `body` for what the DOM says, not in what it's written in, however React has it.
#[test]
fn a_portal_into_body_is_at_the_end_of_body() {
    let before = format!(
        "{SAYS}\nconst measure = () => {{ const p = document.getElementById('p'); const form = document.getElementById('f'); \
         return [p.parentElement === document.body, document.body.lastElementChild === p, form.contains(p), document.querySelector('form #p') === null, \
         document.querySelector('body > #p') === p].join(' '); }};"
    );
    let jsx = r#"<dom.div id="root"><dom.form id="f">{createPortal(<dom.div id="p">in portal</dom.div>, document.body)}</dom.form><Says measure={measure} /></dom.div>"#;
    let mut hands = open("portal", "", &before, jsx, (400, 300));
    hands.wait_for("said true true false true true");
}

/// What a later portal into `body` holds is drawn over, and gets the pointer before, what an earlier one does, as it comes after
/// it in `body`, though it's written before it: a dialog opened while a hover card is up is on top of it.
#[test]
fn a_portal_opened_later_is_on_top() {
    let before = r#"function Later() {
    const [open, setOpen] = React.useState(false);
    const [hit, setHit] = React.useState('none');
    return <dom.div>
        <dom.button id="open" onClick={() => setOpen(true)}>open</dom.button>
        <dom.div id="hit">{'hit ' + hit}</dom.div>
        {open && createPortal(<dom.div className="cover" onClick={() => setHit('later')}>later</dom.div>, document.body)}
        <Earlier onHit={() => setHit('earlier')} />
    </dom.div>;
}
function Earlier({ onHit }) {
    return createPortal(<dom.div className="cover" onClick={onHit}>earlier</dom.div>, document.body);
}"#;
    let css = ".cover { position: fixed; left: 100px; top: 100px; width: 200px; height: 100px; z-index: 50; background: white; }";
    let mut hands = open("portal-order", css, before, "<Later />", (400, 300));
    hands.click_at(150.0, 150.0);
    hands.wait_for("hit earlier");
    hands.click_at(10.0, 10.0);
    hands.settle();
    hands.click_at(150.0, 150.0);
    hands.wait_for("hit later");
}

/// A dialog's `translate(-50%, -50%)` moves what's in it too, and a `position: fixed` box at `left: 50%` that's wider than the
/// half of the window that's left is still there, rather than pulled back into the window.
#[test]
fn a_centred_fixed_box_and_what_is_in_it_are_where_a_browser_puts_them() {
    let before = format!(
        "{SAYS}\nconst measure = () => ['box', 'kid'].map((id) => {{ const r = document.getElementById(id).getBoundingClientRect(); return Math.round(r.x) + ',' + Math.round(r.y); }}).join(' ');"
    );
    let css = ".box { position: fixed; left: 50%; top: 50%; width: 560px; height: 100px; padding: 10px; transform: translate(-50%, -50%); }";
    let jsx = r#"<dom.div><dom.div id="box" className="box"><dom.div id="kid">kid</dom.div></dom.div><Says measure={measure} /></dom.div>"#;
    // The page is told it's 800 by 600 before it's drawn, as rsui's own tests leave the window, so that's what `50%` is of.
    let mut hands = open("centred", css, &before, jsx, (800, 600));
    // 580 wide from 400 goes past 800 and stays there: 400 - 290 = 110 across, 300 - 60 = 240 down, and what's in it 10 further
    // in each way. Pulled back into the window, it would be at -70.
    hands.wait_for("said 110,240 120,250");
}

/// Tailwind's `text-sm` says `line-height: calc(1.25 / .875)`, a number with no unit, which is times the text's size.
#[test]
fn a_unitless_calc_line_height_is_times_the_font_size() {
    let before = format!("{SAYS}\nconst measure = () => Math.round(document.getElementById('t').getBoundingClientRect().height);");
    let css = ".t { font-size: 14px; line-height: calc(1.25 / .875); width: 60px; }";
    let jsx = r#"<dom.div><dom.p id="t" className="t">one two three four</dom.p><Says measure={measure} /></dom.div>"#;
    let mut hands = open("calc-line", css, &before, jsx, (400, 300));
    // Two lines or more of 20px each, never a few pixels.
    hands.wait_for("said ");
    assert!(!hands.says("said 3") && !hands.says("said 5") && !hands.says("said 10"), "{}", hands.app.rectangles());
}

/// `var(--a)var(--b)` with nothing between, as Tailwind writes `animate-in`, is two words once they're put in, not one.
#[test]
fn vars_side_by_side_stay_apart() {
    let before = format!(
        "{SAYS}\nconst measure = () => {{ const s = getComputedStyle(document.getElementById('a')); return s.getPropertyValue('animation-name') + ' ' + s.getPropertyValue('animation-duration'); }};"
    );
    let css = "@keyframes enter { from { opacity: 0; } }\n.a { animation: enter var(--d,var(--tw-duration,.5s))var(--e,ease)var(--delay,0s); }";
    let jsx = r#"<dom.div><dom.div id="a" className="a">a</dom.div><Says measure={measure} /></dom.div>"#;
    let mut hands = open("vars-apart", css, &before, jsx, (400, 300));
    hands.wait_for("said enter 0.5s");
}

/// An element React drew lists its attributes, as `getAttributeNames()` and `attributes` do, and its `classList` is its `class`.
#[test]
fn an_element_lists_its_attributes() {
    let before = format!(
        "{SAYS}\nconst measure = () => {{ const b = document.getElementById('b'); b.classList.add('three'); \
         return [b.getAttributeNames().join(','), Array.from(b.attributes, (one) => one.name + '=' + one.value).length, b.attributes.item(0).name, \
         b.attributes.getNamedItem('data-kind').value, [...b.classList].join('+'), b.classList.contains('two'), b.getAttribute('class')].join(' '); }};"
    );
    let jsx = r#"<dom.div><dom.div id="b" className="one two" data-kind="box" role="group" tabIndex={-1}>b</dom.div><Says measure={measure} /></dom.div>"#;
    let mut hands = open("attributes", "", &before, jsx, (400, 300));
    hands.wait_for("said id,class,data-kind,role,tabindex 5 id box one+two+three true one two three");
}

/// A field that's all selected by `select()` says it's selected forward, as Edge and Chrome do on Windows, and says `select`.
#[test]
fn select_says_forward_and_select() {
    let before = format!(
        "{SAYS}\nlet selects = 0;\nwindow.addEventListener('select', () => {{ selects += 1; }}, true);\nconst measure = () => {{ const f = document.getElementById('f'); f.select(); \
         return [f.selectionStart, f.selectionEnd, f.selectionDirection].join(','); }};"
    );
    let jsx = r#"<dom.div><dom.input id="f" defaultValue="abcdef" /><Says measure={measure} /></dom.div>"#;
    let mut hands = open("select", "", &before, jsx, (400, 300));
    hands.wait_for("said 0,6,forward");
}

/// `getBoundingClientRect()` is from the window, as the page and what's around it are scrolled, and `offsetTop` is from the page.
#[test]
fn bounding_rects_count_scrolling() {
    let before = format!(
        "{SAYS}\nconst measure = () => {{ const t = document.getElementById('t'); const r = t.getBoundingClientRect(); return [Math.round(r.y), t.offsetTop, Math.round(window.scrollY)].join(','); }};\n\
         function Scroll() {{ React.useEffect(() => {{ setTimeout(() => {{ window.scrollTo(0, 100); document.getElementById('sc').scrollTop = 40; }}, 10); }}, []); return null; }}"
    );
    // 50px above it, as something that's there rather than a margin, which goes out through the top of the page's scrolling.
    let css = ".gap { height: 50px; }\n.sc { overflow: auto; height: 100px; width: 200px; }\n.in { height: 500px; }\n.tall { height: 2000px; }";
    let jsx = r#"<dom.div><Scroll /><dom.div className="gap" /><dom.div id="sc" className="sc"><dom.div className="in"><dom.div id="t" style={{ marginTop: 150 }}>t</dom.div></dom.div></dom.div><dom.div className="tall" /><Says measure={measure} /></dom.div>"#;
    let mut hands = open("scrolled-rects", css, &before, jsx, (400, 300));
    // 50 + 150 down the page, less the 40 it's scrolled in its box and the 100 the page is.
    hands.wait_for("said 60,200,100");
}

/// Tab goes from field to field past a `<label>` (which can be clicked but can't have the keyboard) and the page's own scrolling,
/// and what loses the keyboard is told what gets it, as `relatedTarget`.
#[test]
fn tab_goes_past_labels_and_says_where_it_went() {
    let before = format!(
        "{SAYS}\nlet went = '';\nwindow.addEventListener('focusout', (event) => {{ went = event.target.id + '>' + (event.relatedTarget?.id ?? 'none'); }}, true);\n\
         const measure = () => document.activeElement.id + ' ' + went;"
    );
    let jsx = r#"<dom.div><dom.label htmlFor="a">A</dom.label><dom.input id="a" /><dom.label htmlFor="b">B</dom.label><dom.input id="b" /><dom.div className="tall" /><Says measure={measure} /></dom.div>"#;
    let mut hands = open("tab-labels", ".tall { height: 2000px; }", &before, jsx, (400, 300));
    hands.press(KeyCode::Tab);
    hands.press(KeyCode::Tab);
    hands.wait_for("said b a>b");
}

/// What `html` is given is what selectors about it look at, and an element listing its attributes doesn't change that: a second
/// `function pageAttributes` (for `getAttributeNames`) once went over the first, as the later of two goes, and Mantine's
/// `:root[data-mantine-color-scheme]` variables were lost.
#[test]
fn root_attributes_still_match_selectors() {
    let before = format!(
        "{SAYS}\ndocument.documentElement.setAttribute('data-scheme', 'light');\nconst measure = () => Math.round(document.getElementById('t').getBoundingClientRect().width) + ' ' + document.getElementById('t').getAttributeNames().join(',');"
    );
    let css = ".t { width: 50px; }\n:root[data-scheme='light'] .t { width: 77px; }";
    let jsx = r#"<dom.div><dom.div id="t" className="t">t</dom.div><Says measure={measure} /></dom.div>"#;
    let mut hands = open("root-attributes", css, &before, jsx, (400, 300));
    hands.wait_for("said 77 id,class");
}

/// The runtime's scripts are one scope each, where a name that's declared twice at the top is the later one everywhere, silently:
/// each is declared once.
#[test]
fn no_name_is_declared_twice_in_the_runtime() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in ["web.js", "layer.js", "dom.js", "dom_events.js", "dom_globals.js", "editing.js", "react.js"] {
        let text = std::fs::read_to_string(folder.join(file)).unwrap();
        let mut seen = std::collections::HashSet::new();
        for line in text.lines() {
            let rest = line.strip_prefix("async ").unwrap_or(line);
            let name = ["function* ", "function ", "const ", "let ", "class "]
                .iter()
                .find_map(|start| rest.strip_prefix(start))
                .map(|after| after.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')).next().unwrap_or(""));
            if let Some(name) = name.filter(|name| !name.is_empty()) {
                assert!(seen.insert(name.to_owned()), "{file} declares {name} twice at the top");
            }
        }
    }
}

/// A `<style>` put in `<head>` has the page drawn again, which, when nothing looks different, its window has no reason to lay out:
/// what's found in the page then still says where it is, rather than that it has no size (the harness's `click` after its first
/// `eval`, which hides the caret).
#[test]
fn rects_stay_after_a_style_that_changes_nothing() {
    let before = format!(
        "{SAYS}
let first = null;
let added = false;
setTimeout(() => {{ const s = document.createElement('style'); s.textContent = '.zz {{ color: red }}'; document.head.appendChild(s); added = true; }}, 20);
         const measure = () => {{ if (!added) return 'not yet'; first ??= Math.round(document.querySelector('#t').getBoundingClientRect().width); return first; }};"
    );
    let jsx = r#"<dom.div><dom.div id="t" style={{ width: 120, height: 30 }}>t</dom.div><Says measure={measure} /></dom.div>"#;
    let mut hands = open("style-rects", "", &before, jsx, (400, 300));
    hands.wait_for("said 120");
}
