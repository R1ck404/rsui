//! What the shadcn/ui Accordion, Collapsible, Tabs, Select and Scroll Area
//! needed of rsui/web, each as small as it comes.

mod hands;

use std::path::PathBuf;

use hands::Hands;
use rsui::KeyCode;

fn open(name: &str, css: &str, before: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-shadcn-disclosure-{}-{name}", std::process::id()));
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

#[test]
fn a_data_attribute_given_as_undefined_isnt_there() {
    // Radix gives every Accordion item `data-disabled={disabled ? '' : undefined}`; React leaves the attribute out.
    let hands = open(
        "data-undefined",
        "",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                const item = document.getElementById('item');
                setSaid('disabled ' + item.getAttribute('data-disabled') + ' ' + item.hasAttribute('data-disabled') + ' ' + ('disabled' in item.dataset) + ', state ' + item.getAttribute('data-state'));
            }, []);
            return (
                <dom.div>
                    <dom.div id="item" data-disabled={undefined} data-state="open">Item</dom.div>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    assert!(hands.says("disabled null false false, state open"), "{}", hands.app.outline());
}

#[test]
fn a_line_height_worked_out_with_no_units_is_times_the_font_size() {
    // Tailwind 4's `text-2xl` is `line-height: var(--text-2xl--line-height)`, which is `calc(2 / 1.5)`: 32px at 24px.
    let mut hands = open(
        "unitless-calc-line-height",
        ":root { --lh: calc(2 / 1.5); } .big { font-size: 24px; line-height: var(--lh); }",
        r#"
        function Page() {
            return <dom.div><dom.div className="big" key="big">One line</dom.div></dom.div>;
        }
        "#,
    );
    let [_, _, _, height] = hands.rect_of("big");
    assert!((height - 32.0).abs() < 0.5, "a line 32px tall, not {height}: {}", hands.app.outline());
}

#[test]
fn a_heading_beside_a_block_keeps_its_own_font() {
    // The docs' page: `<main><h1>…</h1><section>…</section></main>`. The `<h1>`'s text was given the font of what it's in,
    // 16px, before its own `Styled` said 32px.
    let mut hands = open(
        "heading-font",
        "",
        r#"
        function Page() {
            return <dom.div><dom.h1 key="heading">Accordion</dom.h1><dom.div>Demo</dom.div></dom.div>;
        }
        "#,
    );
    let [_, _, _, height] = hands.rect_of("heading");
    assert!(height > 30.0, "an h1 as tall as 32px text, not {height}: {}", hands.app.outline());
}

#[test]
fn tab_skips_what_scrolls_when_something_in_it_takes_the_keyboard() {
    // A page that scrolls starts Tab at its first button, not at the page, as a browser's keyboard-focusable scrollers are.
    let mut hands = open(
        "tab-scroller",
        "",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('nothing');
            return (
                <dom.div>
                    <dom.div style={{ height: 120, overflowY: 'auto' }}>
                        <dom.button onFocus={() => setSaid('first')}>First</dom.button>
                        <dom.div style={{ height: 600 }} />
                    </dom.div>
                    <dom.div style={{ height: 120, overflowY: 'auto' }} onFocus={() => setSaid('scroller')}>
                        <dom.div style={{ height: 600 }}>Only text</dom.div>
                    </dom.div>
                    <dom.p>{'focused ' + said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    hands.press(KeyCode::Tab);
    hands.settle();
    assert!(hands.says("focused first"), "{}", hands.app.outline());
    hands.press(KeyCode::Tab);
    hands.settle();
    assert!(hands.says("focused scroller"), "{}", hands.app.outline());
}

#[test]
fn code_that_stops_an_animation_measures_without_it_and_reads_its_name_as_it_is_now() {
    // Radix Collapsible, in a layout effect: `animationName = 'none'`, measure, put it back; then Presence reads
    // `getComputedStyle().animationName`. The box opens with `height: 0` from its keyframes, and is 80px without them.
    let mut hands = open(
        "stilled-animation",
        "@keyframes grow { from { height: 0; } to { height: 20px; } } .box { animation: grow 1s; overflow: hidden; }",
        r#"
        function Box({ onSaid }) {
            const ref = React.useRef(null);
            React.useLayoutEffect(() => {
                const box = ref.current;
                box.style.animationName = 'none';
                const height = box.getBoundingClientRect().height;
                const during = getComputedStyle(box).animationName;
                box.style.animationName = '';
                onSaid('height ' + height + ' during ' + during + ' after ' + getComputedStyle(box).animationName);
            }, []);
            return <dom.div ref={ref} className="box"><dom.div style={{ height: 80 }} /></dom.div>;
        }
        function Page() {
            const [open, setOpen] = React.useState(false);
            const [said, setSaid] = React.useState('');
            React.useEffect(() => { setTimeout(() => setOpen(true), 30); }, []);
            return <dom.div>{open ? <Box onSaid={setSaid} /> : null}<dom.p>{said}</dom.p></dom.div>;
        }
        "#,
    );
    hands.wait_for("height ");
    assert!(hands.says("height 80 during none after grow"), "{}", hands.app.outline());
}

#[test]
fn an_animation_name_written_on_an_element_and_taken_off_says_what_css_gives() {
    // Radix leaves `animation-name: none` on content that started open, and takes it off as it closes; Presence then reads
    // the closing animation's name, before the element's been drawn again.
    let mut hands = open(
        "restored-animation-name",
        "@keyframes shut { from { opacity: 1; } to { opacity: 0; } } .box { animation: shut 1s; }",
        r#"
        function Page() {
            const ref = React.useRef(null);
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                setTimeout(() => {
                    const box = ref.current;
                    const before = getComputedStyle(box).animationName;
                    box.style.animationName = '';
                    setSaid('before ' + before + ' after ' + getComputedStyle(box).animationName);
                }, 30);
            }, []);
            return <dom.div><dom.div ref={ref} className="box" style={{ animationName: 'none' }}>Box</dom.div><dom.p>{said}</dom.p></dom.div>;
        }
        "#,
    );
    hands.wait_for("before ");
    assert!(hands.says("before none after shut"), "{}", hands.app.outline());
}

#[test]
fn element_style_reads_what_react_wrote() {
    // Radix sets `--radix-collapsible-content-height` through React's `style`; code reads it back from `element.style`.
    let mut hands = open(
        "react-style",
        "",
        r#"
        function Page() {
            const ref = React.useRef(null);
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                const box = ref.current;
                setSaid('h ' + box.style.getPropertyValue('--h') + ' w ' + box.style.width);
            }, []);
            return <dom.div><dom.div ref={ref} style={{ '--h': '12px', width: 30 }}>Box</dom.div><dom.p>{said}</dom.p></dom.div>;
        }
        "#,
    );
    hands.wait_for("h ");
    assert!(hands.says("h 12px w 30px"), "{}", hands.app.outline());
}

#[test]
fn a_shape_in_an_svg_is_where_its_geometry_is() {
    // lucide's chevron, `m6 9 6 6 6-6` in a 24-unit viewBox drawn at 16px: 8 by 4, at 4,6 in the svg.
    let mut hands = open(
        "svg-path-rect",
        "",
        r#"
        function Page() {
            const ref = React.useRef(null);
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                setTimeout(() => {
                    const svg = ref.current;
                    const outer = svg.getBoundingClientRect();
                    const path = svg.querySelector('path');
                    const box = path.getBoundingClientRect();
                    const round = (value) => Math.round(value * 100) / 100;
                    setSaid('path ' + round(box.x - outer.x) + ',' + round(box.y - outer.y) + ' ' + round(box.width) + 'x' + round(box.height) + ' ' + getComputedStyle(path).display);
                }, 30);
            }, []);
            return (
                <dom.div>
                    <dom.svg ref={ref} width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor"><dom.path d="m6 9 6 6 6-6" /></dom.svg>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    hands.wait_for("path ");
    assert!(hands.says("path 4,6 8x4 inline"), "{}", hands.app.outline());
}

#[test]
fn what_a_clip_path_cuts_all_of_away_takes_no_pointer() {
    // Tailwind's `sr-only` text in an icon button: a press on the button is the button's, not the text's.
    let mut hands = open(
        "sr-only-pointer",
        ".sr { position: absolute; left: 10px; top: 10px; width: 20px; height: 20px; overflow: hidden; clip-path: inset(50%); }\
         .button { position: relative; display: block; width: 40px; height: 40px; padding: 0; }",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('nothing');
            return (
                <dom.div>
                    <dom.button key="button" className="button" onPointerDown={(event) => setSaid('pressed ' + event.target.tagName)}>
                        <dom.span className="sr">Toggle</dom.span>
                    </dom.button>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    let [x, y, width, height] = hands.rect_of("button");
    hands.click_at(x + width / 2.0, y + height / 2.0);
    hands.settle();
    assert!(hands.says("pressed BUTTON"), "{}", hands.app.outline());
}

#[test]
fn what_is_not_shown_keeps_its_place_among_what_is_around_it() {
    // Radix Tabs' hidden panels are between the shown one and the rest in the DOM, and count for `:nth-child`.
    let mut hands = open(
        "hidden-order",
        "",
        r#"
        function Page() {
            const ref = React.useRef(null);
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                setTimeout(() => {
                    const ids = [...ref.current.children].map((one) => one.id).join(' ');
                    const third = ref.current.querySelector('p:nth-child(3)')?.id;
                    const hidden = document.getElementById('b').getBoundingClientRect();
                    setSaid('order ' + ids + ' third ' + third + ' size ' + hidden.width + 'x' + hidden.height);
                }, 30);
            }, []);
            return (
                <dom.div>
                    <dom.div ref={ref}><dom.p id="a">A</dom.p><dom.p id="b" hidden>B</dom.p><dom.p id="c">C</dom.p></dom.div>
                    <dom.span>{said}</dom.span>
                </dom.div>
            );
        }
        "#,
    );
    hands.wait_for("order ");
    assert!(hands.says("order a b c third c size 0x0"), "{}", hands.app.outline());
}

#[test]
fn a_portal_into_an_element_of_the_page_is_in_that_element() {
    // Radix Select's chosen item's text goes into its trigger's `SelectValue` through `createPortal`.
    let mut hands = open(
        "element-portal",
        "",
        r#"
        import { createPortal } from 'react-dom';
        function Page() {
            const [target, setTarget] = React.useState(null);
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                if (target !== null) {
                    setTimeout(() => setSaid('value ' + target.textContent + ' nodes ' + target.childNodes.length), 30);
                }
            }, [target]);
            return (
                <dom.div>
                    <dom.span ref={setTarget} id="value" />
                    <dom.div>{target !== null ? createPortal('Banana', target) : null}</dom.div>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    hands.wait_for("value ");
    assert!(hands.says("value Banana nodes 1"), "{}", hands.app.outline());
}

#[test]
fn an_event_that_does_not_bubble_is_heard_where_it_was_sent_on_html() {
    // Radix's DismissableLayer sends `dismissableLayer.pointerDownOutside` to what a press outside landed on, which is `<html>`
    // while `body` is `pointer-events: none`, and listens for it there.
    let mut hands = open(
        "html-custom-event",
        "",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                const html = document.documentElement;
                let heard = 'nothing heard';
                html.addEventListener('outside', (event) => { heard = 'heard ' + event.detail.at; }, { once: true });
                html.dispatchEvent(new CustomEvent('outside', { bubbles: false, cancelable: true, detail: { at: 'html' } }));
                setSaid(heard);
            }, []);
            return <dom.p>{said}</dom.p>;
        }
        "#,
    );
    hands.wait_for("heard");
    assert!(hands.says("heard html"), "{}", hands.app.outline());
}

#[test]
fn a_table_box_is_as_wide_as_its_min_width_says() {
    // Radix ScrollArea's content: `display: table; min-width: 100%` in its viewport, as wide as the viewport.
    let mut hands = open(
        "table-min-width",
        ".outer { width: 200px; } .table { display: table; min-width: 100%; }",
        r#"
        function Page() {
            return <dom.div className="outer"><dom.div className="table" key="table"><dom.div>Short</dom.div></dom.div></dom.div>;
        }
        "#,
    );
    let [_, _, width, _] = hands.rect_of("table");
    assert!((width - 200.0).abs() < 0.5, "as wide as what it's in, not {width}: {}", hands.app.outline());
}
