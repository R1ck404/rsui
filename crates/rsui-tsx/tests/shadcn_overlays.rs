//! What shadcn/ui's Dialog, Alert Dialog, Sheet and Drawer pages needed of rsui/web to look and act as
//! they do in a browser, each as small as it comes.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn open(name: &str, css: &str, before: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-shadcn-overlays-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let css = format!("body {{ margin: 0; }}\n{css}");
    let app = format!(
        "import {{ PageWindow, dom }} from 'rsui/web';\nimport * as React from 'react';\nimport './app.css';\n{before}\nexport default function App() {{ return <PageWindow><Page /></PageWindow>; }}\n"
    );
    std::fs::write(folder.join("app.tsx"), app).unwrap();
    std::fs::write(folder.join("app.css"), css).unwrap();
    let mut hands = Hands::open(&folder.join("app.tsx"), (500, 400));
    hands.settle();
    hands
}

/// The line of `rectangles_with_lines` that draws this text.
fn drawn(hands: &mut Hands, text: &str) -> String {
    let all = hands.app.rectangles_with_lines();
    all.lines()
        .find(|line| line.contains(&format!("\"{text}\"")))
        .unwrap_or_else(|| panic!("no {text:?} in\n{all}"))
        .to_string()
}

#[test]
fn a_line_height_thats_a_calc_of_numbers_is_times_the_font_size() {
    // Tailwind 4's `text-2xl` is `line-height: var(--tw-leading, var(--text-2xl--line-height))` with
    // `--text-2xl--line-height: calc(2 / 1.5)`: 1.333 times the text's size, 32px at 24px, not 1.333px.
    let mut hands = open(
        "calc-line-height",
        r#"
        :root { --leading: calc(2 / 1.5); }
        .big { font-size: 24px; line-height: var(--tw-leading, var(--leading)); }
        .small { font-size: 14px; line-height: calc(1.25 / 0.875); }
        "#,
        r#"
        function Page() {
            return (
                <dom.div>
                    <dom.h1 className="big">Title</dom.h1>
                    <dom.p className="small">Words</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    let height = |line: String| -> f64 {
        let rect = line.split_whitespace().nth(1).unwrap();
        rect.split(',').nth(3).unwrap().parse().unwrap()
    };
    let title = drawn(&mut hands, "Title");
    assert!((height(title.clone()) - 32.0).abs() < 0.01, "{title}");
    let words = drawn(&mut hands, "Words");
    assert!((height(words.clone()) - 20.0).abs() < 0.01, "{words}");
}

#[test]
fn portals_that_open_together_stack_in_the_order_theyre_written() {
    // Radix's Dialog opens its overlay's portal and its content's at once, each drawing nothing until a layout effect says it's
    // mounted, and rsui could draw the content's before the overlay's. In a browser both go at the end of `body` in the order
    // they're written, as they're committed together, so the content is over the overlay, and takes the click.
    let mut hands = open(
        "portal-order",
        r#"
        .cover { position: fixed; inset: 0; z-index: 50; background: rgba(0, 0, 0, 0.5); }
        .box { position: fixed; left: 100px; top: 100px; width: 200px; height: 100px; z-index: 50; background: white; }
        "#,
        r#"
        import { createPortal } from 'react-dom';
        function Later({ children }) {
            const [ready, setReady] = React.useState(false);
            React.useLayoutEffect(() => setReady(true), []);
            return ready ? createPortal(children, document.body) : null;
        }
        function Page() {
            const [open, setOpen] = React.useState(false);
            const [said, setSaid] = React.useState('nothing');
            return (
                <dom.div>
                    <dom.button id="open" onClick={() => setOpen(true)}>Open</dom.button>
                    <dom.p>{said}</dom.p>
                    {open && <Later><dom.div className="cover" onClick={() => setSaid('the cover')} /></Later>}
                    {open && <Later><dom.button className="box" onClick={() => setSaid('the box')}>Box</dom.button></Later>}
                </dom.div>
            );
        }
        "#,
    );
    hands.click_at(20.0, 10.0);
    hands.settle();
    assert!(hands.says("nothing"), "{}", hands.app.outline());
    hands.click_at(200.0, 150.0);
    hands.settle();
    assert!(hands.says("the box"), "{}", hands.app.outline());
}

#[test]
fn a_page_elements_own_font_isnt_lost_to_what_its_in() {
    // shadcn's page headings: `<h1 className="text-2xl font-bold">` in a flex column `<main>`, which pushed its own 16px font onto
    // the heading's text, so the heading was as big as the body's text.
    let mut hands = open(
        "own-font",
        r#"
        .column { display: flex; flex-direction: column; }
        .big { font-size: 24px; font-weight: 700; }
        .caps { text-transform: capitalize; }
        .row { font-size: 20px; }
        "#,
        r#"
        function Page() {
            return (
                <dom.main className="column">
                    <dom.h1 className="big caps">big title</dom.h1>
                    <dom.div className="row"><dom.p>inherited words</dom.p></dom.div>
                </dom.main>
            );
        }
        "#,
    );
    let title = drawn(&mut hands, "Big Title");
    assert!(title.contains("font_size=24"), "{title}");
    let inherited = drawn(&mut hands, "inherited words");
    assert!(inherited.contains("font_size=20"), "{inherited}");
}
