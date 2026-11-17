//! Right-to-left layout (`dir="rtl"`, CSS's `direction: rtl`) as a browser has it, per CSS Writing Modes 3 and Logical Properties 1
//! for horizontal text: what each shadcn/ui `-rtl` docs example needed, each as small as it comes.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn open(name: &str, css: &str, page: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-rtl-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let css = format!("body {{ margin: 0; }}\n{css}");
    let app = format!(
        "import {{ PageWindow }} from 'rsui/web';\nimport * as React from 'react';\nimport './app.css';\n{page}\nexport default function App() {{ return <PageWindow><Page /></PageWindow>; }}\n"
    );
    std::fs::write(folder.join("app.tsx"), app).unwrap();
    std::fs::write(folder.join("app.css"), css).unwrap();
    let mut hands = Hands::open(&folder.join("app.tsx"), (400, 300));
    hands.settle();
    hands
}

/// A page that says, once it's clicked, the left of each element `ids` names, rounded, as `log 1 2 3`.
fn lefts(ids: &[&str], body: &str) -> String {
    let ids = ids.iter().map(|id| format!("'#{id}'")).collect::<Vec<_>>().join(", ");
    format!(
        r#"
        function Page() {{
            const [log, setLog] = React.useState('');
            const look = () => setLog([{ids}].map((one) => Math.round(document.querySelector(one).getBoundingClientRect().x)).join(' '));
            return <div onClick={{look}} style={{{{ height: 300 }}}}>{body}<p>{{'log ' + log}}</p></div>;
        }}"#
    )
}

#[test]
fn a_grids_first_column_is_at_the_right_and_its_start_is_a_cells_right() {
    // shadcn's FieldGroup `grid grid-cols-3` of month, year and CVV in `field-rtl`, and `justify-self: start` / `end` in a cell.
    let mut hands = open(
        "grid",
        ".g { display: grid; grid-template-columns: 100px 100px 100px; width: 300px; } .g > div { height: 10px; } \
         .s { width: 20px; justify-self: start; } .e { width: 20px; justify-self: end; }",
        &lefts(
            &["a", "b", "c", "s", "e", "l"],
            r#"<div className="g" dir="rtl"><div id="a" /><div id="b" /><div id="c" /><div id="s" className="s" /><div id="e" className="e" /></div>
               <div className="g"><div id="l" /></div>"#,
        ),
    );
    hands.click_at(350.0, 250.0);
    hands.wait_for("log 200 100 0 280 100 0");
}

#[test]
fn logical_sides_are_on_the_side_the_text_starts_from() {
    // `ms-*`, `ps-*` and `start-*` (Tailwind 4's `margin-inline-start`, `padding-inline-start`, `inset-inline-start`) at the
    // right where text goes from right to left, and at the left otherwise; `margin-left` stays at the left either way.
    let mut hands = open(
        "logical",
        ".box { position: relative; width: 300px; height: 20px; } .m { margin-inline-start: 30px; width: 50px; height: 5px; } \
         .p { padding-inline-start: 40px; width: 300px; box-sizing: border-box; } .p > div { width: 10px; height: 5px; } \
         .a { position: absolute; inset-inline-start: 10px; top: 0; width: 20px; height: 5px; } \
         .ml { margin-left: 30px; width: 50px; height: 5px; } \
         .both { position: absolute; inset: 0; inset-inline-start: 50px; }",
        &lefts(
            &["m", "p", "a", "ml", "bo", "m2", "p2", "a2"],
            r#"<div dir="rtl" style={{ width: 300 }}>
                 <div className="box"><div id="m" className="m" /></div>
                 <div className="p"><div id="p" /></div>
                 <div className="box"><div id="a" className="a" /></div>
                 <div className="box"><div id="ml" className="ml" /></div>
                 <div className="box"><div id="bo" className="both" /></div>
               </div>
               <div>
                 <div className="box"><div id="m2" className="m" /></div>
                 <div className="p"><div id="p2" /></div>
                 <div className="box"><div id="a2" className="a" /></div>
               </div>"#,
        ),
    );
    hands.click_at(350.0, 280.0);
    // A block's `margin-left` with a width that leaves room over is dropped where text goes from right to left (CSS 2.1
    // §10.3.3), so it sits at the right; `inset: 0; inset-inline-start: 50px` is left 0 and right 50 there.
    hands.wait_for("log 220 250 270 250 0 30 40 10");
}

#[test]
fn a_margin_left_auto_in_a_right_to_left_row_takes_the_room_on_its_own_left() {
    // shadcn's DropdownMenuShortcut is `ml-auto`: in a row that starts at the right, it stays next to what's before it, with the
    // room on its left, as margins are physical. `ms-auto` (inline start, the right there) puts the room on its right instead.
    let mut hands = open(
        "auto-margins",
        ".row { display: flex; width: 300px; } .row > span { width: 50px; height: 5px; } .ml { margin-left: auto; } .ms { margin-inline-start: auto; }",
        &lefts(
            &["a", "b", "c", "d"],
            r#"<div className="row" dir="rtl"><span id="a" /><span id="b" className="ml" /></div>
               <div className="row" dir="rtl"><span id="c" /><span id="d" className="ms" /></div>"#,
        ),
    );
    hands.click_at(350.0, 250.0);
    hands.wait_for("log 250 200 250 0");
}

#[test]
fn a_line_starts_at_the_right_where_text_goes_from_right_to_left() {
    // Text with a box on its line (an inline-block) where nothing says `text-align`: `start` is the right; `left` is the left.
    let mut hands = open(
        "line-start",
        ".l { width: 300px; } .l b { display: inline-block; width: 20px; height: 5px; }",
        &lefts(
            &["a", "b", "c"],
            r#"<div className="l" dir="rtl"><b id="a" /></div>
               <div className="l" dir="rtl" style={{ textAlign: 'left' }}><b id="b" /></div>
               <div className="l"><b id="c" /></div>"#,
        ),
    );
    hands.click_at(350.0, 250.0);
    hands.wait_for("log 280 0 0");
}

#[test]
fn what_is_placed_by_itself_without_sides_starts_at_the_right() {
    // CSS 2.1's static position, at the start of a line, which is its right where text goes from right to left.
    let mut hands = open(
        "static-position",
        ".box { position: relative; width: 300px; height: 20px; padding: 0 10px; } .abs { position: absolute; width: 40px; height: 5px; }",
        &lefts(
            &["a", "b"],
            r#"<div className="box" dir="rtl"><div id="a" className="abs" /></div>
               <div className="box"><div id="b" className="abs" /></div>"#,
        ),
    );
    hands.click_at(380.0, 250.0);
    hands.wait_for("log 270 10");
}

#[test]
fn a_right_to_left_scrollers_bar_is_at_its_left_and_what_is_in_it_starts_after_it() {
    let mut hands = open(
        "scrollbar-left",
        ".s { width: 200px; height: 60px; overflow-y: auto; } .s > div { height: 20px; }",
        &lefts(
            &["a", "b"],
            r#"<div className="s" dir="rtl"><div id="a" /><div /><div /><div /><div /></div>
               <div className="s"><div id="b" /><div /><div /><div /><div /></div>"#,
        ),
    );
    hands.click_at(350.0, 250.0);
    hands.wait_for("log 17 0");
}

#[test]
fn something_scrolling_across_from_the_right_starts_there_with_a_scroll_left_of_0_going_below_it() {
    // Chromium's `scrollLeft` in `dir="rtl"`: 0 at the start, which is the right, and less than 0 towards the left. What's past
    // the left edge is what there is to scroll to; scrollTo, scrollBy and scrollIntoView go by the same.
    let page = r#"
        function Page() {
            const [log, setLog] = React.useState('');
            const look = () => {
                const box = document.getElementById('box');
                const said = [Math.round(box.scrollLeft)];
                box.scrollLeft = -100;
                said.push(Math.round(box.scrollLeft));
                box.scrollBy({ left: -50 });
                said.push(Math.round(box.scrollLeft));
                box.scrollTo({ left: 0 });
                said.push(Math.round(box.scrollLeft));
                box.scrollLeft = 40;
                said.push(Math.round(box.scrollLeft));
                box.scrollLeft = -5000;
                said.push(Math.round(box.scrollLeft), box.scrollWidth, box.clientWidth);
                setLog(said.join(' '));
            };
            return <div onClick={look} style={{ height: 300 }}>
                <div id="box" dir="rtl" style={{ width: 200, height: 50, overflowX: 'auto', overflowY: 'hidden' }}>
                    <div style={{ width: 500, height: 40 }} />
                </div>
                <p>{'log ' + log}</p>
            </div>;
        }"#;
    let mut hands = open("scroll-left", "", page);
    hands.click_at(350.0, 250.0);
    hands.wait_for("log 0 -100 -150 0 0 -300 500 200");
}

#[test]
fn boxes_on_a_line_of_right_to_left_text_go_in_the_order_bidi_gives_them() {
    // An inline box is one object (U+FFFC) among the letters around it: in Arabic text the first is at the right, and in English
    // text in a right-to-left paragraph they stay in order, as a run from the left.
    let page = "
        function Page() {
            const [log, setLog] = React.useState('');
            const x = (id) => document.getElementById(id).getBoundingClientRect().x;
            const look = () => setLog(String(x('a') > x('b')) + ' ' + String(x('c') < x('d')));
            return <div onClick={look} style={{ height: 300 }}>
                <p dir=\"rtl\">\u{0627}\u{0636}\u{063A}\u{0637} <kbd id=\"a\" /> \u{062B}\u{0645} <kbd id=\"b\" /> \u{0644}\u{0644}\u{0628}\u{062D}\u{062B}</p>
                <p dir=\"rtl\">Press <kbd id=\"c\" /> then <kbd id=\"d\" /> now</p>
                <p>{'log ' + log}</p>
            </div>;
        }";
    let mut hands = open("inline-order", "kbd { display: inline-block; width: 20px; height: 10px; } p { width: 300px; margin: 0; }", page);
    hands.click_at(350.0, 250.0);
    hands.wait_for("log true true");
}

#[test]
fn float_inline_start_is_at_the_side_the_text_starts_from() {
    let mut hands = open(
        "float-start",
        ".f { float: inline-start; width: 30px; height: 10px; } .box { width: 300px; }",
        &lefts(
            &["r", "l"],
            r#"<div className="box" dir="rtl"><div id="r" className="f" />text</div><div className="box"><div id="l" className="f" />text</div>"#,
        ),
    );
    hands.click_at(350.0, 250.0);
    hands.wait_for("log 270 0");
}
