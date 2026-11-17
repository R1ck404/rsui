//! What rsui/web did differently from a browser on shadcn's display components (button, badge,
//! alert, card, avatar, kbd, spinner), each as small as it comes.

mod hands;

use std::path::PathBuf;

use hands::Hands;
use rsui::{KeyCode, KeyPress};

fn open(name: &str, css: &str, before: &str, jsx: &str, size: (u32, u32)) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-display-parity-{}-{name}", std::process::id()));
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

/// A heading in a `<div>` is written in its own font, not the div's: every shadcn docs page's headings were body text.
#[test]
fn a_heading_in_a_div_is_written_in_its_own_font() {
    let before = format!(
        "{SAYS}\nconst measure = () => [document.getElementById('h').getBoundingClientRect().height, document.getElementById('p').getBoundingClientRect().height].join(' ');"
    );
    let css = "h1 { font-size: 32px; line-height: 40px; margin: 0; } p { font-size: 24px; line-height: 1.5; margin: 0; }";
    let jsx = r#"<div><div><h1 id="h">Heading</h1><p id="p">Paragraph</p></div><Says measure={measure} /></div>"#;
    let mut hands = open("heading", css, &before, jsx, (400, 300));
    hands.wait_for("said 40 36");
}

/// `textContent` is the DOM's text, as it was written, and only `innerText` is what `text-transform` shows.
#[test]
fn text_content_is_what_was_written_and_inner_text_what_is_shown() {
    let before = format!(
        "{SAYS}\nconst measure = () => {{ const h = document.getElementById('h'); return [h.textContent, h.innerText].join(' '); }};"
    );
    let jsx = r#"<div><h1 id="h" className="cap">button</h1><Says measure={measure} /></div>"#;
    let mut hands = open("case", ".cap { text-transform: capitalize; }", &before, jsx, (400, 300));
    hands.wait_for("said button Button");
}

/// A `data-` prop that's undefined or null isn't an attribute, as react-dom has it, however the element is found.
#[test]
fn an_undefined_data_prop_is_no_attribute() {
    let before = format!(
        "{SAYS}\nconst measure = () => {{ const found = []; const walk = (e) => {{ if (e.id === 'g') found.push(e); for (const c of e.children || []) walk(c); }}; walk(document.body); \
         const g = found[0]; return [g.getAttribute('data-orientation'), g.hasAttribute('data-orientation'), g.getAttribute('data-off'), g.dataset.on].map(String).join('|'); }};"
    );
    let jsx = r#"<div><div id="g" data-orientation={undefined} data-off={null} data-on={false}>g</div><Says measure={measure} /></div>"#;
    let mut hands = open("data", "", &before, jsx, (400, 300));
    hands.wait_for("said null|false|null|false");
}

/// `body` is as tall as what's in it, not the window.
#[test]
fn body_is_as_tall_as_what_is_in_it() {
    let before = format!("{SAYS}\nconst measure = () => document.body.getBoundingClientRect().height;");
    let jsx = r#"<div style={{ height: 120 }}><Says measure={measure} /></div>"#;
    let mut hands = open("body", "", &before, jsx, (400, 300));
    hands.wait_for("said 120");
}

/// A double click is two clicks, the second with `detail` 2, then `dblclick`, which listeners on the way down hear too, on
/// something with no click handler of its own; the other button clicks as `auxclick` before its `contextmenu` (which says `detail` 0, as Edge's does: event-order/seq); and Space
/// doesn't click a link, which only Enter does.
#[test]
fn double_clicks_aux_clicks_and_space_on_a_link_go_as_in_a_browser() {
    let before = format!(
        "{SAYS}\nconst log = [];\nfor (const type of ['click', 'dblclick', 'auxclick', 'contextmenu']) window.addEventListener(type, (event) => log.push(type + event.detail), true);\nconst measure = () => log.join(',');"
    );
    let css = "#b { position: absolute; left: 10px; top: 10px; width: 100px; height: 30px; } #l { position: absolute; left: 10px; top: 60px; }";
    let jsx = r#"<div><button id="b">Button</button><a id="l" href="/login" onClick={(event) => event.preventDefault()}>Link</a><Says measure={measure} /></div>"#;
    let mut hands = open("clicks", css, &before, jsx, (400, 300));
    hands.move_at(50.0, 20.0);
    for clicks in 1..=2 {
        hands.app.pointer_down(50.0, 20.0, clicks);
        hands.settle();
        hands.app.pointer_up(50.0, 20.0).unwrap();
        hands.settle();
    }
    hands.app.button_down(50.0, 20.0, rsui::PointerButton::Secondary);
    hands.settle();
    hands.app.button_up(50.0, 20.0, rsui::PointerButton::Secondary).unwrap();
    hands.settle();
    hands.wait_for("said click1,click2,dblclick2,auxclick1,contextmenu0");
    hands.click_at(20.0, 65.0);
    hands.press_key(KeyPress::new(KeyCode::Space));
    hands.press_key(KeyPress::new(KeyCode::Enter));
    hands.wait_for("said click1,click2,dblclick2,auxclick1,contextmenu0,click1,click0");
}

/// A disabled button's `pointer-events: none` goes to what's in it too, so the pointer over its text reaches what's behind it,
/// and its `:hover` look doesn't come on.
#[test]
fn pointer_events_none_lets_the_pointer_through_what_is_in_it() {
    let before = format!(
        "{SAYS}\nconst log = [];\nwindow.addEventListener('pointerover', (event) => log.push(event.target.id || event.target.tagName), true);\nconst measure = () => log.join(',');"
    );
    let css = "#b { pointer-events: none; position: absolute; left: 10px; top: 10px; width: 100px; height: 30px; }";
    let jsx = r#"<div id="behind" style={{ height: 200 }}><button id="b"><span id="t">Button</span></button><Says measure={measure} /></div>"#;
    let mut hands = open("pointer-through", css, &before, jsx, (400, 300));
    hands.move_at(300.0, 150.0);
    hands.move_at(50.0, 25.0);
    hands.wait_for("said behind");
}

/// `:has(> svg)` sees an icon component as the `<svg>` it makes, as Button's `has-[>svg]:px-3` asks of lucide's icons.
#[test]
fn has_sees_a_component_as_what_it_makes() {
    let before = format!(
        "{SAYS}\nfunction Icon() {{ return <svg width=\"16\" height=\"16\" viewBox=\"0 0 24 24\"><path d=\"M2 2L20 20\" /></svg>; }}\nconst measure = () => document.getElementById('b').getBoundingClientRect().width;"
    );
    let css = ".b { display: inline-flex; padding: 0 16px; border: 0; } .b:has(> svg) { padding: 0 12px; }";
    let jsx = r#"<div><button id="b" className="b"><Icon /></button><Says measure={measure} /></div>"#;
    let mut hands = open("has-component", css, &before, jsx, (400, 300));
    hands.wait_for("said 40");
}

/// `[aria-pressed]` finds a toggle whose `aria-pressed` is `false`, which React writes as the word, as on a page.
#[test]
fn an_aria_attribute_that_is_false_is_still_there() {
    let before = format!("{SAYS}
const measure = () => document.querySelectorAll('[aria-pressed]').length + ' ' + document.querySelectorAll('[aria-pressed=false]').length;");
    let jsx = r#"<div><button aria-pressed={false}>off</button><button aria-pressed={true}>on</button><Says measure={measure} /></div>"#;
    let mut hands = open("aria-false", "", &before, jsx, (400, 300));
    hands.wait_for("said 2 1");
}
