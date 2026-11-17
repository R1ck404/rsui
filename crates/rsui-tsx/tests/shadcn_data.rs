//! What shadcn/ui's Typography, Table, Data Table, Pagination, Breadcrumb, Separator, Skeleton, Progress and Aspect Ratio pages
//! needed of rsui/web to look and act as they do in a browser, each as small as it comes.

mod hands;

use std::path::PathBuf;

use hands::Hands;
use rsui::KeyCode;

fn open(name: &str, css: &str, before: &str, jsx: &str, size: (u32, u32)) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-shadcn-data-{}-{name}", std::process::id()));
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

/// A link in a paragraph goes along its line, in a file whose JSX has no tag rsui has too (no `<span>`, `<button>` or `<input>`),
/// so that its `<a>` is a plain tag until it's drawn, as shadcn's typography examples are written: it was put on a line of its
/// own, and the paragraph was three lines tall.
#[test]
fn a_link_in_a_paragraph_stays_on_its_line() {
    let before = format!(
        "{SAYS}\nconst measure = () => {{ const p = document.getElementById('p'); const a = p.querySelector('a'); \
         return [Math.round(p.getBoundingClientRect().height), a.getBoundingClientRect().x > 20].join(' '); }};"
    );
    let jsx = r#"<div><p id="p">Bare p then <a href="/x">a link</a>: and more.</p><Says measure={measure} /></div>"#;
    let mut hands = open("link-in-p", "p { line-height: 20px; }", &before, jsx, (500, 300));
    hands.wait_for("said 20 true");
}

/// A part of `text-decoration` said after an `inherit` of all of it goes over that: Tailwind's `.underline` (which says
/// `text-decoration-line`) over its preflight's `a { text-decoration: inherit }` underlines the link.
#[test]
fn a_decoration_line_goes_over_an_inherited_decoration() {
    let before = format!("{SAYS}\nconst measure = () => getComputedStyle(document.getElementById('a')).textDecorationLine;");
    let css = "a { color: inherit; text-decoration: inherit; }\n.underline { text-decoration-line: underline; }";
    let jsx = r#"<dom.div><dom.p>Go <dom.a id="a" href="/x" className="underline">there</dom.a>.</dom.p><Says measure={measure} /></dom.div>"#;
    let mut hands = open("underline-over-inherit", css, &before, jsx, (400, 200));
    hands.wait_for("said underline");
}

/// The page's scrolling is `<html>`'s: `getBoundingClientRect()` takes off how far it's scrolled once (it was taken off again for
/// each element it's in, so `#root` was twice as far up as it is and `main` three times), `body.scrollTop` is 0 as in a page
/// with a doctype, and `document.scrollingElement` is `<html>`.
#[test]
fn rects_under_a_scrolled_page_take_off_the_scroll_once() {
    let before = format!(
        "{SAYS}\nfunction Scroll() {{ React.useEffect(() => {{ setTimeout(() => window.scrollTo(0, 300), 10); }}, []); return null; }}\n\
         const measure = () => {{ const t = document.getElementById('t'); return [Math.round(document.getElementById('outer').getBoundingClientRect().y), \
         Math.round(t.getBoundingClientRect().y), t.offsetTop, document.body.scrollTop, document.documentElement.scrollTop, document.scrollingElement.tagName].join(','); }};"
    );
    let css = ".pad { height: 400px; }\n.tall { height: 2000px; }";
    let jsx = r#"<dom.div id="outer"><Scroll /><dom.div><dom.div className="pad" /><dom.div id="t">t</dom.div></dom.div><dom.div className="tall" /><Says measure={measure} /></dom.div>"#;
    let mut hands = open("scrolled-rects-once", css, &before, jsx, (400, 300));
    hands.wait_for("said -300,100,400,0,300,HTML");
}

/// `scroll-width: none` on `<html>` takes the window's scrollbar away, and the room it took.
#[test]
fn no_scrollbar_on_html_takes_no_room() {
    let before = format!("{SAYS}\nconst measure = () => Math.round(document.getElementById('w').getBoundingClientRect().width);");
    let css = "html { scrollbar-width: none; }\n.tall { height: 2000px; }";
    let jsx = r#"<dom.div><dom.div id="w">w</dom.div><dom.div className="tall" /><Says measure={measure} /></dom.div>"#;
    let mut hands = open("no-page-scrollbar", css, &before, jsx, (400, 300));
    hands.wait_for("said 400");
}

/// Something that's stopped scrolling says `scrollend`, after its `scroll`s, as a browser does.
#[test]
fn scrolling_ends_with_scrollend() {
    let before = format!(
        "{SAYS}\nlet heard = [];\nwindow.addEventListener('scroll', () => {{ if (heard[heard.length - 1] !== 'scroll') heard.push('scroll'); }});\n\
         document.addEventListener('scrollend', () => heard.push('end'));\n\
         function Scroll() {{ React.useEffect(() => {{ setTimeout(() => window.scrollTo(0, 200), 10); }}, []); return null; }}\n\
         const measure = () => heard.join(' ') + ' ' + Math.round(window.scrollY);"
    );
    let jsx = r#"<dom.div><Scroll /><dom.div className="tall" /><Says measure={measure} /></dom.div>"#;
    let mut hands = open("scrollend", ".tall { height: 2000px; }", &before, jsx, (400, 300));
    hands.wait_for("said scroll end 200");
}

/// `blur()` takes the keyboard away in Rust too, so the next key goes to the page, not to what had it; and with nothing on the
/// keyboard and the pointer nowhere, a page's keys scroll the page: PageDown by 87.5% of what shows, as Chromium's do, Space as
/// PageDown, and Space on a focused link scrolls rather than following it.
#[test]
fn keys_scroll_the_page_as_a_browser_does() {
    let before = format!(
        "{SAYS}\nlet keys = [];\nlet clicks = 0;\nwindow.addEventListener('keydown', (event) => keys.push(event.target.tagName));\n\
         const measure = () => [keys.join('+'), Math.round(window.scrollY), clicks].join('|');\n\
         function Blur() {{ React.useEffect(() => {{ setTimeout(() => {{ const a = document.getElementById('a'); a.focus(); a.blur(); }}, 10); }}, []); return null; }}"
    );
    let jsx = r#"<dom.div><Blur /><dom.a id="a" href="/x" onClick={(event) => { event.preventDefault(); clicks += 1; }}>link</dom.a><dom.div className="tall" /><Says measure={measure} /></dom.div>"#;
    let mut hands = open("keys-scroll", ".tall { height: 4000px; }", &before, jsx, (400, 400));
    hands.wait_for("said |0|0");
    hands.press(KeyCode::PageDown);
    hands.wait_for("said BODY|350|0");
    hands.press(KeyCode::Space);
    hands.wait_for("said BODY+BODY|700|0");
    hands.press(KeyCode::Home);
    hands.wait_for("said BODY+BODY+BODY|0|0");
}

/// A link to `#` goes to the top of the page and says `hashchange`, as a browser does with a fragment that's empty, and a link
/// to `#name` goes to what has that id.
#[test]
fn a_link_to_a_fragment_scrolls_there() {
    let before = format!(
        "{SAYS}\nlet changes = 0;\nwindow.addEventListener('hashchange', () => {{ changes += 1; }});\n\
         function Scroll() {{ React.useEffect(() => {{ setTimeout(() => window.scrollTo(0, 500), 10); }}, []); return null; }}\n\
         const measure = () => [Math.round(window.scrollY), changes].join(' ');"
    );
    let jsx = r##"<dom.div><Scroll /><dom.div className="tall" /><dom.a id="top" href="#">top</dom.a><dom.a id="down" href="#far">far</dom.a><dom.div className="tall" /><dom.div id="far">far</dom.div><dom.div className="tall" /><Says measure={measure} /></dom.div>"##;
    let mut hands = open("fragment-links", ".tall { height: 600px; }", &before, jsx, (400, 300));
    hands.wait_for("said 500 0");
    // The link is 600px down the page, so 100px down the window.
    hands.click_at(5.0, 108.0);
    hands.wait_for("said 0 1");
}

/// A table as shadcn's is (`border-collapse: collapse`, a `border-b` on each `<tr>`, a caption first with `caption-side: bottom` and
/// `mt-4`, a head with `w-[100px]`): the column that says its width keeps it while the others share what's left by how wide they'd
/// like to be, each row's line under it takes a pixel of room (37px rows of 36px cells, a head said to be 40px stays 40), and the
/// caption's margin stays inside the table rather than pushing it down.
#[test]
fn a_collapsed_table_is_laid_out_as_a_browser_does() {
    let before = format!(
        "{SAYS}
const measure = () => {{ const r = (id) => document.getElementById(id).getBoundingClientRect();          return [Math.round(r('t').y), Math.round(r('h1').width), Math.round(r('h1').height), Math.round(r('r1').height), Math.round(r('c').y - r('t').y)].join(','); }};"
    );
    let css = "table { border-collapse: collapse; width: 400px; caption-side: bottom; }
caption { margin-top: 16px; height: 20px; }
               tr { border-bottom: 1px solid black; }
th { height: 40px; padding: 0 8px; text-align: left; }
td { padding: 8px; line-height: 20px; }
               .w100 { width: 100px; }";
    let jsx = r#"<dom.div><dom.table id="t"><dom.caption id="c">cap</dom.caption><dom.thead><dom.tr><dom.th id="h1" className="w100">A</dom.th><dom.th>B</dom.th></dom.tr></dom.thead><dom.tbody><dom.tr id="r1"><dom.td>one</dom.td><dom.td>two</dom.td></dom.tr></dom.tbody></dom.table><Says measure={measure} /></dom.div>"#;
    let mut hands = open("collapsed-table", css, &before, jsx, (500, 300));
    // The table at the top, the head's first column 100px and its padding (content-box) and 40px tall, a 37px row, the caption
    // 16px under 77px of rows.
    hands.wait_for("said 0,116,40,37,93");
}

/// A percentage height in a flex row whose height isn't known is as if it said nothing (shadcn's vertical `<Separator
/// className="h-full">` in an `items-center` row, centred in its frame, is 0px tall, not as tall as the row), and a `display: flex` from a later rule
/// keeps the `flex-direction` an earlier one said (`hidden flex-col md:flex` is a column at md).
#[test]
fn flex_percent_heights_and_directions_are_as_in_a_browser() {
    let before = format!(
        "{SAYS}
const measure = () => {{ const r = (id) => document.getElementById(id).getBoundingClientRect();          return [Math.round(r('sep').height), Math.round(r('b').y - r('a').y)].join(','); }};"
    );
    let css = ".row {{ display: flex; align-items: center; }}
.sep {{ width: 1px; height: 100%; background: black; }}
               .col {{ flex-direction: column; }}
.hide {{ display: none; }}
.show {{ display: flex; }}
.tall {{ height: 40px; }}
               .frame {{ display: flex; align-items: center; justify-content: center; height: 100px; }}"
        .replace("{{", "{").replace("}}", "}");
    let jsx = r#"<dom.div><dom.div className="frame"><dom.div className="row"><dom.div className="tall">x</dom.div><dom.div id="sep" className="sep" /></dom.div></dom.div><dom.div className="hide col show"><dom.span id="a">A</dom.span><dom.span id="b">B</dom.span></dom.div><Says measure={measure} /></dom.div>"#;
    let mut hands = open("flex-percent-direction", &css, &before, jsx, (400, 300));
    hands.wait_for("said 0,18");
}

/// An `aria-label` names what it's on and isn't drawn: a Radix checkbox with `aria-label="Select row"` and nothing in it while it's
/// unchecked drew "S" in its 16px box. A bare button with one is as wide as nothing.
#[test]
fn an_aria_label_is_not_drawn() {
    let before = format!("{SAYS}
const measure = () => Math.round(document.getElementById('b').getBoundingClientRect().width);");
    let css = "button { padding: 0; border: 0; }";
    let jsx = r#"<dom.div><dom.button id="b" aria-label="Select all" /><Says measure={measure} /></dom.div>"#;
    let mut hands = open("aria-label-quiet", css, &before, jsx, (400, 200));
    hands.wait_for("said 0");
}

/// Text a component gives back is a Text node of the element it's all that's in, as TanStack's `flexRender` gives a string
/// header: the `<th>` had no `childNodes`.
#[test]
fn text_a_component_gives_back_is_a_text_node() {
    let before = format!(
        "{SAYS}
function Header() {{ return 'Status'; }}
         const measure = () => {{ const th = document.getElementById('h'); return [th.childNodes.length, th.firstChild && th.firstChild.nodeType, th.firstChild && th.firstChild.data].join(','); }};"
    );
    let jsx = r#"<dom.div><dom.table><dom.thead><dom.tr><dom.th id="h"><Header /></dom.th></dom.tr></dom.thead></dom.table><Says measure={measure} /></dom.div>"#;
    let mut hands = open("component-text-node", "", &before, jsx, (400, 200));
    hands.wait_for("said 1,3,Status");
}
