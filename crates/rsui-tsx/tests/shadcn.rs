//! What shadcn/ui's components needed of rsui/web, each as small as it comes: CSS and DOM
//! that Tailwind 3 and Radix lean on, laid out as a browser lays it out.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn write(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-shadcn-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    for (file, text) in files {
        std::fs::write(folder.join(file), text).unwrap();
    }
    folder.join("app.tsx")
}

/// What's laid out, once an app with this CSS, these lines before it, and this JSX in a page has settled.
fn laid_out_with(name: &str, css: &str, before: &str, jsx: &str) -> String {
    laid_out_in(name, css, before, jsx, (500, 400))
}

/// The same, in a window this big. The page has no margin, so what's in it is placed from the window's corner.
fn laid_out_in(name: &str, css: &str, before: &str, jsx: &str, size: (u32, u32)) -> String {
    let css = format!("body {{ margin: 0; }}\n{css}");
    let app = format!(
        "import {{ PageWindow, dom }} from 'rsui/web';\nimport * as React from 'react';\nimport './app.css';\n{before}\nexport default function App() {{ return <PageWindow>{jsx}</PageWindow>; }}\n"
    );
    let mut hands = Hands::open(&write(name, &[("app.tsx", &app), ("app.css", &css)]), size);
    hands.settle();
    hands.app.rectangles()
}

fn laid_out(name: &str, css: &str, jsx: &str) -> String {
    laid_out_with(name, css, "", jsx)
}

/// The line about what says this.
fn line<'a>(laid_out: &'a str, text: &str) -> &'a str {
    let quoted = format!("{text:?}");
    laid_out
        .lines()
        .find(|line| line.contains(&quoted))
        .unwrap_or_else(|| panic!("nothing says {text}\n{laid_out}"))
}

/// Where what says this is, and how big: x, y, width and height.
fn rect(laid_out: &str, text: &str) -> [f32; 4] {
    let parts: Vec<f32> = line(laid_out, text)
        .split_whitespace()
        .nth(1)
        .unwrap()
        .split(',')
        .map(|one| one.parse().unwrap())
        .collect();
    [parts[0], parts[1], parts[2], parts[3]]
}

/// shadcn/ui's dialog footer is `flex flex-col-reverse sm:flex-row sm:justify-end`: the later `flex-direction: row` turns the
/// earlier one's reversing off, as each rule's props are put together after they're worked out.
#[test]
fn a_later_flex_direction_undoes_reverse() {
    let css = ".footer { display: flex; flex-direction: column-reverse; }\n@media (min-width: 10px) { .footer { flex-direction: row; justify-content: flex-end; } }\n.b { width: 50px; }\n";
    let laid_out = laid_out("reverse", css, r#"<dom.div className="footer"><dom.div className="b">Cancel</dom.div><dom.div className="b">Continue</dom.div></dom.div>"#);
    let cancel = rect(&laid_out, "Cancel");
    let go = rect(&laid_out, "Continue");
    assert_eq!(cancel[1], go[1], "on one line\n{laid_out}");
    assert!(cancel[0] < go[0], "Cancel first\n{laid_out}");
    assert_eq!(go[0] + go[2], 500.0, "at the end\n{laid_out}");
}

/// react-day-picker's weeks are `<tr>`s that are `display: flex` with `margin-top: 8px`: no longer rows, each is in a row and a
/// cell of its own, as CSS makes them, with its margin inside that.
#[test]
fn a_flex_table_row_keeps_its_margin() {
    let css = "table { border-collapse: collapse; }\n.week { display: flex; margin-top: 8px; }\ntd { height: 32px; width: 32px; padding: 0; }\n";
    let laid_out = laid_out(
        "flex-row",
        css,
        r#"<dom.table><dom.tbody><dom.tr className="week"><dom.td>1</dom.td><dom.td>2</dom.td></dom.tr><dom.tr className="week"><dom.td>8</dom.td><dom.td>9</dom.td></dom.tr></dom.tbody></dom.table>"#,
    );
    let first = rect(&laid_out, "1");
    let second = rect(&laid_out, "8");
    assert_eq!(second[1] - first[1], 40.0, "a week and its margin apart\n{laid_out}");
}

/// shadcn/ui's Command is `h-full` in a card that's a grid's cell, stretched to its row: it's as tall as the card, and what's after
/// it goes past the card's end, as nothing shrinks in a block.
#[test]
fn a_percentage_height_in_a_stretched_grid_cell_is_of_the_cell() {
    let css = ".grid { display: grid; grid-template-columns: 1fr 1fr; }\n.full { height: 100%; }\n.tall { min-height: 200px; }\n";
    let laid_out = laid_out(
        "grid-full",
        css,
        r#"<dom.div className="grid"><dom.section><dom.div className="full">full</dom.div><dom.div>after</dom.div></dom.section><dom.section className="tall">tall</dom.section></dom.div>"#,
    );
    assert_eq!(rect(&laid_out, "full")[3], 200.0, "{laid_out}");
    assert_eq!(rect(&laid_out, "after")[1], 200.0, "{laid_out}");
}

/// A carousel's slides past its `overflow: hidden` edge don't make the page any wider, so it gets no scrollbar for them.
#[test]
fn what_is_cut_off_does_not_widen_the_page() {
    let css = ".clip { overflow: hidden; width: 100px; }\n.strip { display: flex; }\n.slide { width: 300px; flex-shrink: 0; }\n";
    let laid_out = laid_out(
        "clip-wide",
        css,
        r#"<dom.div className="clip"><dom.div className="strip"><dom.div className="slide">one</dom.div><dom.div className="slide">two</dom.div><dom.div className="slide">three</dom.div></dom.div></dom.div>"#,
    );
    // The page's scrolling area, then what's in it, which is as wide as the window, with no scrollbar beside or under it.
    let inside = laid_out.lines().nth(2).unwrap();
    assert!(inside.trim_start().starts_with("group 0,0,500,400"), "{laid_out}");
}

/// A dialog in the middle of the window, `position: fixed; left: 50%; top: 50%`, is on top of the window's overlay before it, as
/// it comes after it: both are placed in the window, so drawn in the order they come in.
#[test]
fn a_fixed_box_placed_by_percentages_is_in_the_window() {
    let css = ".overlay { position: fixed; inset: 0; z-index: 50; background: rgba(0,0,0,0.8); }\n.dialog { position: fixed; left: 50%; top: 50%; z-index: 50; width: 100px; height: 40px; background: white; }\n";
    // In a window as big as the page is told it is before it's drawn, as rsui's own tests leave that.
    let laid_out = laid_out_in("fixed-share", css, "", r#"<dom.div className="overlay" /><dom.div className="dialog">sure?</dom.div>"#, (800, 600));
    let dialog = rect(&laid_out, "sure?");
    assert_eq!([dialog[0], dialog[1]], [400.0, 300.0], "{laid_out}");
}

/// Radix's focus guards go first and last in `body`, with `insertAdjacentElement('afterbegin')` before what the app drew, and
/// what code puts in `body` is drawn after the page, and is among what's in `body`.
#[test]
fn code_puts_things_in_body_around_what_the_app_drew() {
    let before = r#"function Probe() {
    const [said, setSaid] = React.useState('waiting');
    React.useEffect(() => {
        const start = document.createElement('span');
        const end = document.createElement('div');
        end.textContent = 'appended';
        try {
            document.body.insertAdjacentElement('afterbegin', start);
            document.body.insertAdjacentElement('beforeend', end);
            setSaid('last ' + (document.body.lastElementChild === end) + ' in ' + (start.parentNode === document.body));
        } catch (error) {
            setSaid('threw ' + error.message);
        }
    }, []);
    return <dom.div>{said}</dom.div>;
}"#;
    let laid_out = laid_out_with("body-kids", "", before, "<Probe />");
    line(&laid_out, "last true in true");
    line(&laid_out, "appended");
}

/// The page drawn, as red, green, blue and alpha for each pixel, in a window this big, with what's laid out.
fn drawn(name: &str, css: &str, before: &str, jsx: &str, size: (u32, u32)) -> (Vec<u8>, String) {
    let css = format!("body {{ margin: 0; }}\n{css}");
    let app = format!(
        "import {{ PageWindow, dom }} from 'rsui/web';\nimport * as React from 'react';\nimport './app.css';\n{before}\nexport default function App() {{ return <PageWindow>{jsx}</PageWindow>; }}\n"
    );
    let mut hands = Hands::open(&write(name, &[("app.tsx", &app), ("app.css", &css)]), size);
    hands.settle();
    let laid_out = hands.app.rectangles();
    let mut renderer = rsui::render::Renderer::offscreen(size).unwrap();
    let scene = hands.app.scene();
    renderer.render(scene, rsui::render::Color::WHITE).unwrap();
    (renderer.read_pixels().unwrap(), laid_out)
}

/// The color at a pixel, as red, green and blue.
fn at(pixels: &[u8], width: u32, (x, y): (u32, u32)) -> [u8; 3] {
    let i = ((y * width + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2]]
}

/// On a page that says where things are among the rest (`.a + .b`), each component is wrapped in a context's provider that tells
/// it, and a provider that was once around only text was learned to make text: every component it wrapped after that went on a
/// line, as shadcn's Breadcrumb, Pagination, Menubar and Form did, shrinking to fit. A provider only hands down what it's given.
#[test]
fn a_provider_once_around_text_does_not_put_blocks_on_a_line() {
    let css = ".a + .b { color: red; }\n";
    // The same context's provider around text in one place, as Radix Select's item text is, and around blocks in another.
    let before = r#"const Told = React.createContext(0);
function Block({ children }) { return <dom.nav>{children}</dom.nav>; }"#;
    let jsx = r#"<dom.div><dom.p><Told.Provider value={1}>only words</Told.Provider></dom.p><dom.section><dom.h2>title</dom.h2><Told.Provider value={2}><Block>first</Block></Told.Provider><Told.Provider value={3}><Block>second</Block></Told.Provider></dom.section></dom.div>"#;
    let laid_out = laid_out_with("provider-text", css, before, jsx);
    let first = rect(&laid_out, "first");
    let second = rect(&laid_out, "second");
    assert_eq!(first[2], 500.0, "a block as wide as what it's in\n{laid_out}");
    assert!(second[1] >= first[1] + first[3], "one under the other\n{laid_out}");
}

/// shadcn's Alert is `[&>svg~*]:pl-7 [&>svg]:absolute`: what comes after its lucide icon, a component found to make an `<svg>`
/// only once it's drawn, is moved clear of it.
#[test]
fn a_sibling_rule_sees_what_a_component_made() {
    let css = ".alert { position: relative; }\n.alert > svg { position: absolute; left: 0; top: 0; }\n.alert > svg ~ * { padding-left: 28px; }\n";
    let before = r#"function Icon() { return <dom.svg width={16} height={16} viewBox="0 0 16 16"><dom.path d="M0 0H16V16Z" /></dom.svg>; }
function Title({ children }) { return <dom.h5>{children}</dom.h5>; }"#;
    let jsx = r#"<dom.div className="alert"><Icon /><Title><dom.b>Heads up</dom.b></Title></dom.div>"#;
    let laid_out = laid_out_with("sibling-made", css, before, jsx);
    assert_eq!(rect(&laid_out, "Heads up")[0], 28.0, "{laid_out}");
}

/// Tailwind's preflight gives a textarea `font-size: 100%`, and shadcn's Textarea says `text-sm` (14px) after it: the class wins,
/// as it does on an `<input>`.
#[test]
fn a_textarea_takes_its_class_font_size_over_the_tag_rule() {
    let css = "textarea { font-size: 100%; }\n.sm { font-size: 14px; line-height: 20px; }\n";
    let laid_out = laid_out("textarea-size", css, r#"<dom.textarea className="sm" placeholder="Type here" />"#);
    let found = line(&laid_out, "Type here");
    assert!(found.contains("font_size=14"), "{found}\n{laid_out}");
}

/// Recharts fills shadcn's chart bars with `fill="var(--color-desktop)"`, a presentation attribute, from the variables around the
/// `<svg>`, and labels its axis with `<text>` and `<tspan>`, which are drawn.
#[test]
fn an_svg_fills_from_a_variable_and_draws_its_text() {
    let css = ".chart { --color-bar: #ff0000; font-size: 24px; }\n";
    // Made as recharts makes them, with React's `createElement` and the tags' names.
    let before = r##"const e = React.createElement;
function Chart() {
    return e('svg', { width: 200, height: 100, viewBox: '0 0 200 100' },
        e('rect', { x: 0, y: 0, width: 50, height: 50, fill: 'var(--color-bar)' }),
        e('text', { x: 150, y: 50, textAnchor: 'middle', fill: '#000000' }, e('tspan', { x: 150, dy: '0.71em' }, 'WWW')));
}"##;
    let jsx = r#"<dom.div className="chart"><Chart /></dom.div>"#;
    let (pixels, laid_out) = drawn("svg-var-text", css, before, jsx, (300, 200));
    assert_eq!(at(&pixels, 300, (25, 25)), [255, 0, 0], "the bar is filled\n{laid_out}");
    // The text is in the middle of x = 150, below y = 50, and dark.
    let mut dark = 0;
    for y in 50..80 {
        for x in 120..180 {
            if at(&pixels, 300, (x, y))[0] < 128 {
                dark += 1;
            }
        }
    }
    assert!(dark > 40, "{dark} dark pixels where the text is\n{laid_out}");
    for y in 0..100 {
        assert!(at(&pixels, 300, (110, y))[0] > 200, "nothing left of the middled text at y {y}");
    }
}

/// A disabled Button is `opacity: 0.5` on a dark background with light text: it's faded as a whole, so the text is as light as the
/// background is dark, rather than the background showing through the text.
#[test]
fn opacity_fades_what_overlaps_as_one() {
    let css = ".faded { opacity: 0.5; background: #000000; width: 100px; height: 60px; position: relative; }\n.over { position: absolute; left: 20px; top: 20px; width: 40px; height: 20px; background: #ffffff; }\n";
    let (pixels, laid_out) = drawn("group-opacity", css, "", r#"<dom.div className="faded"><dom.div className="over" /></dom.div>"#, (200, 100));
    // White over black, then half of that over the white page, is white; fading each by itself made it a light grey.
    let over = at(&pixels, 200, (40, 30));
    assert!(over.iter().all(|&part| part >= 250), "{over:?}\n{laid_out}");
    let back = at(&pixels, 200, (10, 10));
    assert!(back.iter().all(|&part| (125..=130).contains(&part)), "{back:?}\n{laid_out}");
}

/// shadcn's Button gives back `<button>` written with its tag's name, which is drawn by the component for that tag: it's found to
/// sit on a line as that one is, so a data table's Previous and Next are side by side in their block, rather than one under the
/// other.
#[test]
fn a_component_giving_back_a_tag_is_found_to_sit_on_a_line() {
    let css = ".a + .b { color: red; }\n.button { display: inline-flex; padding: 4px; }\n.gap > * ~ * { margin-left: 8px; }\n";
    let before = "function Button({ children }) { return React.createElement('button', { className: 'button' }, children); }";
    let laid_out = laid_out_with("tag-inline", css, before, r#"<dom.div className="gap"><Button>Previous</Button><Button>Next</Button></dom.div>"#);
    let previous = rect(&laid_out, "Previous");
    let next = rect(&laid_out, "Next");
    assert_eq!(previous[1], next[1], "on one line\n{laid_out}");
    assert!(next[0] > previous[0] + previous[2], "after it\n{laid_out}");
}

/// Radix's Select renders its closed list's items with `createPortal` into a `DocumentFragment`, to know what they say: they run
/// but aren't shown and take no room, so what's under a closed Select isn't pushed down by them.
#[test]
fn a_portal_into_a_fragment_takes_no_room() {
    let before = r#"import { createPortal } from 'react-dom';
function Hidden() {
    const fragment = React.useMemo(() => document.createDocumentFragment(), []);
    return createPortal(<dom.div style={{ height: 200 }}>hidden item</dom.div>, fragment);
}"#;
    let laid_out = laid_out_with("fragment-portal", "", before, r#"<dom.div><dom.div style={{ height: 30 }}>above</dom.div><Hidden /><dom.p style={{ margin: 0 }}>under</dom.p></dom.div>"#);
    assert_eq!(rect(&laid_out, "under")[1], 30.0, "{laid_out}");
}
