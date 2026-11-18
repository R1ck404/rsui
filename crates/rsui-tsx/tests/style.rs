//! rsui/web's style engine: which rules reach which elements, in what order, and what's drawn again when they change.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn write(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-style-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    for (file, text) in files {
        std::fs::write(folder.join(file), text).unwrap();
    }
    folder.join("app.tsx")
}

fn open(name: &str, files: &[(&str, &str)]) -> Hands {
    Hands::open(&write(name, files), (500, 400))
}

/// What's laid out, once an app with this CSS and this JSX in a page has settled.
fn laid_out(name: &str, css: &str, jsx: &str) -> String {
    let app = format!(
        "import {{ PageWindow, dom }} from 'rsui/web';\nimport './app.css';\nexport default function App() {{ return <PageWindow>{jsx}</PageWindow>; }}\n"
    );
    let mut hands = open(name, &[("app.tsx", &app), ("app.css", css)]);
    hands.settle();
    hands.app.rectangles()
}

/// The line about what says this.
fn line<'a>(laid_out: &'a str, text: &str) -> &'a str {
    let quoted = format!("{text:?}");
    laid_out
        .lines()
        .find(|line| line.contains(&quoted))
        .unwrap_or_else(|| panic!("nothing says {text}\n{laid_out}"))
}

/// How wide and tall what says this is.
fn size(laid_out: &str, text: &str) -> (f32, f32) {
    let rect: Vec<f32> = line(laid_out, text)
        .split_whitespace()
        .nth(1)
        .unwrap()
        .split(',')
        .map(|one| one.parse().unwrap())
        .collect();
    (rect[2], rect[3])
}

/// A component in a list that makes more than one element, as one around a dialog's children or a fragment is, said a different
/// element each time it was drawn, which drew the whole page again, forever, so that no timer ever ran.
#[test]
fn what_a_component_makes_is_learned_once_and_the_page_settles() {
    let app = r#"
        import { dom } from 'rsui/web';
        import * as React from 'react';
        import './app.css';
        let drawn = 0;
        function Header() {
            drawn += 1;
            return <dom.div className="label">Add local repository</dom.div>;
        }
        function Both() {
            drawn += 1;
            return <><dom.span className="label">one</dom.span><dom.b>two</dom.b></>;
        }
        export default function App() {
            const ref = React.useRef<any>(null);
            const [fired, setFired] = React.useState(false);
            React.useEffect(() => {
                ref.current?.showModal?.();
                setTimeout(() => setFired(true), 50);
            }, []);
            return (
                <dom.div>
                    <dom.p>{fired ? 'timer fired after ' + drawn : 'timer waiting'}</dom.p>
                    <Both />
                    <dom.dialog ref={ref}>
                        <Header />
                        <dom.form onSubmit={(e: any) => e.preventDefault()}>
                            <dom.input placeholder="repository path" />
                        </dom.form>
                    </dom.dialog>
                </dom.div>
            );
        }
    "#;
    let css = ".label { color: #333; }\n.label:first-of-type { font-weight: bold; }\n";
    let mut hands = open("learn-once", &[("app.tsx", app), ("app.css", css)]);
    hands.wait_for("timer fired");
    let outline = hands.app.outline();
    let drawn: u32 = outline
        .split("timer fired after ")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .and_then(|count| count.parse().ok())
        .expect("how many times they were drawn");
    assert!(drawn < 12, "drawn {drawn} times\n{outline}");
}

/// A custom property's name is as it's written, as `--Grid-columns` isn't `--grid-columns`.
#[test]
fn custom_property_names_keep_their_case() {
    let css = ":root { --Box-Width: 70px; --boxwidth: 10px; --boxWidth: 30px; }\n.a { width: var(--Box-Width); }\n.b { width: var(--boxWidth); }\n";
    let laid_out = laid_out("var-case", css, r#"<dom.div className="a">a</dom.div><dom.div className="b">b</dom.div>"#);
    assert_eq!(size(&laid_out, "a").0, 70.0, "{laid_out}");
    assert_eq!(size(&laid_out, "b").0, 30.0, "{laid_out}");
}

/// A variable set in an element's `style` is what the rules on it, and on what's inside it, use.
#[test]
fn a_variable_in_a_style_reaches_the_rules_on_it_and_inside_it() {
    let css = ".child { width: var(--w, 10px); }\n";
    let laid_out = laid_out(
        "inline-var",
        css,
        r#"<dom.div style={{ '--w': '80px' } as any}><dom.div className="child">inside</dom.div></dom.div><dom.div className="child" style={{ '--w': '90px' } as any}>own</dom.div><dom.div className="child">neither</dom.div>"#,
    );
    assert_eq!(size(&laid_out, "inside").0, 80.0, "{laid_out}");
    assert_eq!(size(&laid_out, "own").0, 90.0, "{laid_out}");
    assert_eq!(size(&laid_out, "neither").0, 10.0, "{laid_out}");
}

/// A later rule's `display` undoes an earlier one's `display: none`, as one value of a property goes over another, and as
/// `!important` and specificity say.
#[test]
fn a_display_from_another_rule_undoes_display_none() {
    let css = ".hidden { display: none; }\n.md-block { @media (width >= 10px) { display: block; } }\n.flat-block { display: block; }\n\
        .j { display: none; }\n.k.j { display: block; }\n.gone { display: none !important; }\n.a.gone { display: block !important; }\n\
        .never { display: none !important; }\n.over { display: block; }\n";
    let laid_out = laid_out(
        "display-undoes-none",
        css,
        r#"<dom.div className="hidden md-block">one</dom.div><dom.div className="hidden flat-block">two</dom.div><dom.div className="k j">three</dom.div><dom.div className="a gone">four</dom.div><dom.div className="never over">five</dom.div>"#,
    );
    for shown in ["one", "two", "three", "four"] {
        line(&laid_out, shown);
    }
    assert!(!laid_out.contains("\"five\""), "!important stays: {laid_out}");
}

/// `hidden` is the browser's own `[hidden] { display: none }`, which a page's `display` goes over.
#[test]
fn the_hidden_attribute_is_what_the_browser_says_and_css_goes_over_it() {
    let css = ".shown { display: flex; }\n";
    let laid_out = laid_out(
        "hidden-attribute",
        css,
        r#"<dom.div hidden>plain hidden</dom.div><dom.div hidden className="shown">shown anyway</dom.div><dom.div>after</dom.div>"#,
    );
    assert!(!laid_out.contains("plain hidden"), "{laid_out}");
    line(&laid_out, "shown anyway");
    line(&laid_out, "after");
}

/// `!important` on a rule about an id goes over a rule that says more without it.
#[test]
fn important_on_an_id_goes_over_a_more_particular_rule() {
    let css = "#root > .base > div { width: 20px; }\n#c { width: 70px !important; }\n";
    let laid_out = laid_out(
        "important-id",
        css,
        r#"<dom.div id="root"><dom.div className="base"><dom.div id="c">c</dom.div><dom.div>d</dom.div></dom.div></dom.div>"#,
    );
    assert_eq!(size(&laid_out, "c").0, 70.0, "{laid_out}");
    assert_eq!(size(&laid_out, "d").0, 20.0, "{laid_out}");
}

/// A character written by its number in a selector, as `\2c ` is, ends at its space, which isn't between two steps.
#[test]
fn a_character_by_its_number_in_a_selector_ends_at_its_space() {
    let css = r".shadow-\[0_1px_rgba\(0\2c 0\2c 0\2c 0\.05\)\] { width: 40px; }";
    let laid_out = laid_out("hex-escape", css, r#"<dom.div className="shadow-[0_1px_rgba(0,0,0,0.05)]">x</dom.div>"#);
    assert_eq!(size(&laid_out, "x").0, 40.0, "{laid_out}");
}

/// An empty `transition-duration`, as a `var()` that's nothing gives, says nothing, rather than stopping the page.
#[test]
fn an_empty_transition_list_says_nothing() {
    let css = ".t { transition-property: opacity; transition-duration: var(--none); transition-delay: var(--none); }\n";
    let laid_out = laid_out("empty-transition", css, r#"<dom.div className="t">still here</dom.div>"#);
    line(&laid_out, "still here");
}

/// A `<style>` in what's drawn is a sheet of the page's, and isn't drawn itself.
#[test]
fn a_style_in_the_tree_is_a_sheet_and_not_text() {
    let app = r#"
        import 'rsui/web';
        export default function App() {
            return <div>
                <style dangerouslySetInnerHTML={{ __html: '.a { width: 70px; }' }} />
                <style>{'.b { width: 60px; }'}</style>
                <div className="a">a</div><div className="b">b</div>
            </div>;
        }
    "#;
    let mut hands = open("style-in-tree", &[("app.tsx", app)]);
    hands.settle();
    let laid_out = hands.app.rectangles();
    assert!(!laid_out.contains("width:"), "{laid_out}");
    assert_eq!(size(&laid_out, "a").0, 70.0, "{laid_out}");
    assert_eq!(size(&laid_out, "b").0, 60.0, "{laid_out}");
}

/// `@font-face` takes the first file in `src` that can be read here, without what a bundler puts after `?`, from where the sheet is.
#[test]
fn a_font_face_file_is_found_from_its_sheet() {
    let font = std::path::Path::new(r"C:\Windows\Fonts\arial.ttf");
    if !font.exists() {
        return;
    }
    let css = "@font-face { font-family: 'Own'; src: url('../fonts/own.eot'); src: url('../fonts/own.woff2?v=2') format('woff2'), \
        url(\"../fonts/own.ttf?v=2#x\") format('truetype'); }\n.f { font-family: 'Own'; }\n";
    let app = "import 'rsui/web';\nimport './styles/app.css';\nexport default function App() { return <div className=\"f\">own font</div>; }\n";
    let file = write("font-face", &[("app.tsx", app)]);
    let folder = file.parent().unwrap();
    std::fs::create_dir_all(folder.join("styles")).unwrap();
    std::fs::create_dir_all(folder.join("fonts")).unwrap();
    std::fs::write(folder.join("styles/app.css"), css).unwrap();
    std::fs::copy(font, folder.join("fonts/own.ttf")).unwrap();
    let mut hands = Hands::open(&file, (500, 400));
    hands.settle();
    let laid_out = hands.app.rectangles();
    assert!(line(&laid_out, "own font").contains("font_family=own.ttf"), "{laid_out}");
}

/// A range's track is drawn as `::-webkit-slider-runnable-track` says, across its middle.
#[test]
fn a_range_track_is_drawn_as_its_part_says() {
    let css = ".r { -webkit-appearance: none; width: 200px; }\n.r::-webkit-slider-runnable-track { height: 4px; background: #00ff00; }\n\
        .r::-webkit-slider-thumb { width: 10px; height: 10px; background: #0000ff; }\n";
    let laid_out = laid_out("range-track", css, r#"<dom.input type="range" className="r" value={50} onChange={() => {}} />"#);
    let track = laid_out
        .lines()
        .find(|line| line.contains("background=#00ff00ff"))
        .unwrap_or_else(|| panic!("no track\n{laid_out}"));
    assert!(track.contains(",200,4 "), "{laid_out}");
    assert!(laid_out.contains("background=#0000ffff"), "{laid_out}");
}

/// `::before` that's a box of its own, as `display: inline-block` makes it, is an element with that look, with its text in it.
#[test]
fn a_part_that_is_a_box_is_laid_out_as_an_element() {
    let css = ".box::before { content: \"x\"; display: inline-block; width: 14px; height: 14px; background: #0000ff; }\n";
    let laid_out = laid_out("part-box", css, r#"<dom.label className="box">after</dom.label>"#);
    assert!(line(&laid_out, "x").contains("background=#0000ffff"), "{laid_out}");
    line(&laid_out, "after");
}

/// Something that isn't a plain object, as a color a library makes is, is what it says as text, in a styled component's CSS.
#[test]
fn a_class_instance_in_a_styled_template_is_its_text() {
    let app = r#"
        import { __styled as styled } from 'rsui/web';
        class Size { constructor(px) { this.px = px; } toString() { return this.px + 'px'; } }
        const Box = styled.div`width: ${new Size(45)};`;
        export default function App() { return <div><Box>box</Box></div>; }
    "#;
    let mut hands = open("class-instance", &[("app.tsx", app)]);
    hands.settle();
    let laid_out = hands.app.rectangles();
    assert_eq!(size(&laid_out, "box").0, 45.0, "{laid_out}");
}

/// `inherit` on what isn't passed down by itself takes the parent's own value, as Bootstrap's `.navbar > .container { flex-wrap:
/// inherit }` does, and a value said after it in the same rule goes over it.
#[test]
fn inherit_takes_the_parents_value_of_what_isnt_passed_down() {
    let css = ".outer { display: flex; flex-wrap: wrap; width: 100px; background: #123456; }\n\
               .inner { display: flex; flex-wrap: inherit; width: 100px; }\n\
               .again { display: flex; flex-wrap: inherit; flex-wrap: nowrap; width: 100px; }\n\
               .cell { width: 60px; height: 10px; flex-shrink: 0; }\n\
               .bg { background: inherit; height: 5px; }\n";
    let laid_out = laid_out(
        "inherit-flex-wrap",
        css,
        r#"<dom.div className="outer"><dom.div className="inner"><dom.div className="cell">a</dom.div><dom.div className="cell">b</dom.div></dom.div><dom.div className="again"><dom.div className="cell">c</dom.div><dom.div className="cell">d</dom.div></dom.div><dom.div className="bg">e</dom.div></dom.div>"#,
    );
    let y = |text: &str| -> f32 { line(&laid_out, text).split_whitespace().nth(1).unwrap().split(',').nth(1).unwrap().parse().unwrap() };
    assert!(y("b") > y("a"), "flex-wrap: inherit wraps as its parent does: {laid_out}");
    assert_eq!(y("d"), y("c"), "nowrap said after inherit goes over it: {laid_out}");
    assert!(line(&laid_out, "e").contains("background=#123456ff"), "background: inherit is the parent's: {laid_out}");
}

/// A percentage `flex-basis` is that share of the flex box, so `flex-basis: 100%` in a box that wraps starts a line of its own,
/// as Bootstrap's navbar does with its menu, and `flex: 1` (a share of 0%) still splits what's left.
#[test]
fn a_percentage_flex_basis_is_a_share_of_the_flex_box() {
    let css = ".row { display: flex; flex-wrap: wrap; width: 200px; }\n\
               .brand { width: 50px; }\n\
               .menu { flex-basis: 100%; flex-grow: 1; }\n\
               .half { flex-basis: 50%; }\n\
               .split { display: flex; width: 200px; }\n\
               .one { flex: 1; }\n";
    let laid_out = laid_out(
        "flex-basis-share",
        css,
        r#"<dom.div className="row"><dom.div className="brand">brand</dom.div><dom.div className="menu">menu</dom.div><dom.div className="half">half</dom.div></dom.div><dom.div className="split"><dom.div className="one">x</dom.div><dom.div className="one">y</dom.div></dom.div>"#,
    );
    let rect = |text: &str| -> Vec<f32> {
        line(&laid_out, text).split_whitespace().nth(1).unwrap().split(',').map(|one| one.parse().unwrap()).collect()
    };
    assert!(rect("menu")[1] > rect("brand")[1], "flex-basis: 100% starts a line of its own: {laid_out}");
    assert_eq!(rect("menu")[2], 200.0, "{laid_out}");
    assert_eq!(rect("half")[2], 100.0, "flex-basis: 50% is half the box: {laid_out}");
    assert_eq!(rect("x")[2], 100.0, "flex: 1 shares what's left: {laid_out}");
    assert_eq!(rect("y")[2], 100.0, "{laid_out}");
}

/// A link that `display: block` makes a block takes room for its padding, as any block does, as Bootstrap's `.nav-link` is.
#[test]
fn a_block_link_takes_room_for_its_padding() {
    let css = ".block { display: block; padding: 8px 0; }\n";
    let laid_out = laid_out(
        "block-link-padding",
        css,
        r##"<dom.div><dom.a className="block" href="#">one</dom.a><dom.a className="block" href="#">two</dom.a></dom.div><dom.div>after</dom.div>"##,
    );
    let y = |text: &str| -> f32 { line(&laid_out, text).split_whitespace().nth(1).unwrap().split(',').nth(1).unwrap().parse().unwrap() };
    let tall = size(&laid_out, "one").1;
    assert_eq!(y("two") - y("one"), tall + 16.0, "two block links are a line and their padding apart: {laid_out}");
    assert_eq!(y("after") - y("two"), tall + 8.0, "what's after is below the second link's padding: {laid_out}");
}

/// One `display` takes the place of another: Bootstrap's `d-grid d-md-flex` is a flex row once the window is wide enough, though
/// `display: grid` came first, and both are `!important`.
#[test]
fn a_later_display_takes_the_place_of_an_earlier_one() {
    // A rule about a state, as Bootstrap has many, puts every rule in the cascade together, which is where `!important` counts.
    let css = ".d-grid { display: grid !important; }\n.x:hover { color: red; }\n@media (min-width: 100px) { .d-md-flex { display: flex !important; } }\n.later { display: grid; }\n.later { display: block; }\n";
    let laid_out = laid_out(
        "display-replaced",
        css,
        r#"<dom.div className="d-grid d-md-flex"><dom.button>one</dom.button><dom.button>two</dom.button></dom.div><dom.div className="later"><dom.span>three</dom.span><dom.span>four</dom.span></dom.div>"#,
    );
    let at = |text: &str| -> Vec<f32> {
        line(&laid_out, text).split_whitespace().nth(1).unwrap().split(',').map(|one| one.parse().unwrap()).collect()
    };
    assert_eq!(at("one")[1], at("two")[1], "flex after grid lays them out in a row: {laid_out}");
    assert!(at("two")[0] > at("one")[0], "{laid_out}");
}

/// Only a list item has a marker: an `<li>` that's `display: flex`, as Bootstrap's `list-group-item d-flex` is, has none, and
/// what's in it is laid out as a flex box's items, with a badge's padding.
#[test]
fn an_li_that_isnt_a_list_item_has_no_marker() {
    let css = ".row { display: flex; justify-content: space-between; }\n.badge { display: inline-block; padding: 4px 8px; }\n";
    let laid_out = laid_out(
        "li-flex-marker",
        css,
        r#"<dom.ul><dom.li className="row">Active<dom.span className="badge">14</dom.span></dom.li><dom.li>plain</dom.li></dom.ul>"#,
    );
    let markers = laid_out.lines().filter(|one| one.contains("background=#000000ff") && one.trim_start().starts_with("group")).count();
    assert_eq!(markers, 1, "only the plain item has a marker: {laid_out}");
    let badge = size(&laid_out, "14");
    assert!(badge.0 >= 16.0 + 8.0, "the badge has its padding: {laid_out}");
}

/// A select the page draws itself (`appearance: none`) has text as tall as its line height says, as a browser's has: Bootstrap's
/// `.form-select` is a 24px line with 6px of padding and a 1px border either side, 38px in all, the same as its `.form-control`
/// input. One the browser draws has its font's own line height, whatever the page says, as Chromium has it.
#[test]
fn a_select_is_as_tall_as_its_line_height_says() {
    let css = ".s { display: block; width: 300px; padding: 6px 12px; font-size: 16px; line-height: 1.5; border: 1px solid #ccc; box-sizing: border-box; }\n.own { appearance: none; }\n.normal { line-height: normal; }\n";
    let option = r#"<dom.option>Starter</dom.option>"#;
    let laid_out = laid_out(
        "select-line-height",
        css,
        &format!(
            r#"<dom.select className="s own">{option}</dom.select><dom.input className="s" defaultValue="same" /><dom.select className="s">{option}</dom.select><dom.select className="s normal">{option}</dom.select>"#
        ),
    );
    let heights = |kind: &str| -> Vec<f32> {
        laid_out
            .lines()
            .filter(|one| one.trim_start().starts_with(kind))
            .map(|one| one.split_whitespace().nth(1).unwrap().split(',').nth(3).unwrap().parse().unwrap())
            .collect()
    };
    let [own, drawn, normal] = heights("button")[..] else { panic!("three selects: {laid_out}") };
    assert_eq!(own, 38.0, "{laid_out}");
    assert_eq!(heights("text_input"), [38.0], "{laid_out}");
    assert!(drawn == normal && drawn < 38.0, "{laid_out}");
}

/// What a rule about more than one element says floats does, as Bootstrap's `.form-check .form-check-input { float: left;
/// margin-left: -1.5em }` does: the checkbox is in the padding, and its label starts where the block's content does.
#[test]
fn a_float_from_a_descendant_rule_floats_into_the_padding() {
    let css = ".fc { padding-left: 24px; }\n.fc .box { float: left; margin-left: -24px; width: 16px; height: 16px; background: #123456; }\n";
    let laid_out = laid_out(
        "descendant-float",
        css,
        r#"<dom.div className="fc"><dom.span className="box" /><dom.label>Remember me</dom.label></dom.div>"#,
    );
    let x = |text: &str| -> f32 { line(&laid_out, text).split_whitespace().nth(1).unwrap().split(',').next().unwrap().parse().unwrap() };
    let page_left = 8.0;
    assert_eq!(x("Remember me"), page_left + 24.0, "the label starts at the content: {laid_out}");
    let box_line = laid_out.lines().find(|one| one.contains("background=#123456ff")).unwrap_or_else(|| panic!("{laid_out}"));
    let box_x: f32 = box_line.split_whitespace().nth(1).unwrap().split(',').next().unwrap().parse().unwrap();
    assert_eq!(box_x, page_left, "the box is in the padding: {laid_out}");
}

/// Rules about what comes before something reach a button and an input too, as Tailwind's `space-y-4` does in a form, though
/// they're put on lines with what's around them.
#[test]
fn sibling_rules_reach_buttons_and_inputs() {
    let css = ".sp > :not([hidden]) ~ :not([hidden]) { margin-top: 16px; }\n.blk { display: block; }\n";
    let laid_out = laid_out(
        "sibling-fields",
        css,
        r#"<dom.form className="sp"><dom.div>first</dom.div><dom.button type="button" className="blk">press</dom.button><dom.input className="blk" defaultValue="typed" /></dom.form>"#,
    );
    let y = |kind: &str| -> f32 {
        let line = laid_out.lines().find(|one| one.contains(kind)).unwrap_or_else(|| panic!("no {kind}: {laid_out}"));
        line.split_whitespace().nth(1).unwrap().split(',').nth(1).unwrap().parse().unwrap()
    };
    let first = y("\"first\"");
    let button = laid_out.lines().find(|one| one.trim_start().starts_with("button")).unwrap_or_else(|| panic!("{laid_out}"));
    let button_y: f32 = button.split_whitespace().nth(1).unwrap().split(',').nth(1).unwrap().parse().unwrap();
    assert!(button_y >= first + 18.0 + 16.0 - 0.5, "the button is 16px below the first line: {laid_out}");
    let input = laid_out.lines().find(|one| one.trim_start().starts_with("text_input")).unwrap_or_else(|| panic!("{laid_out}"));
    let input_y: f32 = input.split_whitespace().nth(1).unwrap().split(',').nth(1).unwrap().parse().unwrap();
    let button_h: f32 = button.split_whitespace().nth(1).unwrap().split(',').nth(3).unwrap().parse().unwrap();
    assert!(input_y >= button_y + button_h + 16.0 - 0.5, "the input is 16px below the button: {laid_out}");
}

/// A textarea is as wide as its `cols` of its font's characters, to the next whole pixel, and a scrollbar 15 wide unless it's
/// hidden, inside its 2px of padding and its border, as Edge has them: 147 + 15 + 6 for 20 of 13.3333px monospace.
#[test]
fn a_textarea_is_as_wide_as_its_columns_and_a_scrollbar() {
    let css = "body { margin: 0; }\ntextarea { display: block; }\n.hidden { overflow: hidden; }\n";
    let laid_out = laid_out(
        "textarea-widths",
        css,
        r#"<dom.div><dom.textarea /><dom.textarea cols={10} /><dom.textarea className="hidden" /></dom.div>"#,
    );
    let widths: Vec<f32> = laid_out
        .lines()
        .filter(|one| one.trim_start().starts_with("text_input"))
        .map(|one| one.split_whitespace().nth(1).unwrap().split(',').nth(2).unwrap().parse().unwrap())
        .collect();
    assert_eq!(widths, [168.0, 95.0, 153.0], "{laid_out}");
}

/// What a browser draws inside a `<select>` takes room on top of the page's padding, which is none unless it says: 4 before its
/// text and 16 after, for its arrow, as Chromium has it, which `appearance: none` takes away, with the arrow.
#[test]
fn a_select_has_room_for_its_arrow_beside_its_padding() {
    let css = "body { margin: 0; }\nselect { display: block; }\n.none { appearance: none; }\n.bare { padding: 0; }\n.wide { padding: 0 10px; }\n";
    let option = r#"<dom.option>One option</dom.option>"#;
    let laid_out = laid_out(
        "select-widths",
        css,
        &format!(
            r#"<dom.div><dom.select>{option}</dom.select><dom.select className="none">{option}</dom.select><dom.select className="bare">{option}</dom.select><dom.select className="wide">{option}</dom.select></dom.div>"#
        ),
    );
    let widths: Vec<f32> = laid_out
        .lines()
        .filter(|one| one.trim_start().starts_with("button"))
        .map(|one| one.split_whitespace().nth(1).unwrap().split(',').nth(2).unwrap().parse().unwrap())
        .collect();
    let [own, none, bare, wide] = widths[..] else { panic!("four selects: {laid_out}") };
    assert!((own - none - 20.0).abs() < 0.01, "{laid_out}");
    assert!((bare - own).abs() < 0.01 && (wide - own - 20.0).abs() < 0.01, "{laid_out}");
}

/// A form's controls that are blocks keep their own width, as a browser has them, rather than stretching across what they're in:
/// an `<input>` with `display: block` is as wide as one on a line, and a button is as wide as its text.
#[test]
fn form_controls_that_are_blocks_keep_their_own_width() {
    let css = "body { margin: 0; }\ninput, button { display: block; }\n";
    let laid_out = laid_out(
        "block-controls",
        css,
        r#"<dom.div><dom.input defaultValue="typed" /><dom.button type="button">press</dom.button></dom.div>"#,
    );
    let width = |kind: &str| -> f32 {
        let line = laid_out.lines().find(|one| one.trim_start().starts_with(kind)).unwrap_or_else(|| panic!("no {kind}: {laid_out}"));
        line.split_whitespace().nth(1).unwrap().split(',').nth(2).unwrap().parse().unwrap()
    };
    assert!(width("text_input") < 300.0, "the input's own width, not the window's 500: {laid_out}");
    assert!(width("button") < 100.0, "the button as wide as its text: {laid_out}");
}

/// What's right in the page that all goes along a line, as two inline-block `<div>`s do, is side by side on that line, as it would
/// be in a block, rather than one under the other.
#[test]
fn inline_blocks_right_in_the_page_are_side_by_side() {
    let css = "body { margin: 0; }\n.d { display: inline-block; width: 32px; height: 32px; margin: 12px; }\n";
    let laid_out = laid_out("page-inline-blocks", css, r#"<dom.div className="d" /><dom.div className="d" />"#);
    let places: Vec<(f32, f32)> = laid_out
        .lines()
        .filter(|one| one.contains(",32,32"))
        .map(|one| {
            let rect: Vec<f32> = one.split_whitespace().nth(1).unwrap().split(',').map(|n| n.parse().unwrap()).collect();
            (rect[0], rect[1])
        })
        .collect();
    assert_eq!(places.len(), 2, "{laid_out}");
    assert_eq!(places[0].1, places[1].1, "on one line: {laid_out}");
    assert_eq!(places[1].0 - places[0].0, 56.0, "the second after the first and both their margins: {laid_out}");
}

/// `rows="5"` on a textarea is as many rows as `rows={5}`, as on a page.
#[test]
fn a_textarea_has_the_rows_it_says_as_text() {
    let laid_out = laid_out(
        "textarea-rows",
        "textarea { font-size: 16px; line-height: 20px; padding: 0; border: 0; }\n",
        r#"<dom.textarea rows="5" defaultValue="five" /><dom.textarea rows={2} defaultValue="two" />"#,
    );
    let heights: Vec<f32> = laid_out
        .lines()
        .filter(|one| one.trim_start().starts_with("text_input"))
        .map(|one| one.split_whitespace().nth(1).unwrap().split(',').nth(3).unwrap().parse().unwrap())
        .collect();
    assert_eq!(heights.len(), 2, "{laid_out}");
    assert!(heights[0] > heights[1] * 2.0, "five rows are more than twice as tall as two: {laid_out}");
}

/// What only an old browser read, and what's left of what Tailwind reads, is left out without a word, as a browser does.
#[test]
fn css_for_old_browsers_and_tools_is_left_out_quietly() {
    let app = r#"
        import { useState } from 'react';
        import { PageWindow, addCss, dom } from 'rsui/web';
        const warned = [];
        console.warn = (...said) => warned.push(said.join(' '));
        addCss('.a { *zoom: 1; _height: 1px; width: 30px; }\n@reference "x.css";\n@source "../src";\n@-moz-keyframes spin { from { opacity: 0 } to { opacity: 1 } }\n.b { animation: spin 1s; }');
        export default function App() {
            const [shown, setShown] = useState('');
            return <PageWindow><dom.div className="a b">a</dom.div><button key="read" onClick={() => setShown(warned.join('|'))}>read</button><dom.p>{'warned [' + shown + ']'}</dom.p></PageWindow>;
        }
    "#;
    let mut hands = open("quiet-css", &[("app.tsx", app)]);
    hands.settle();
    hands.click("read");
    hands.settle();
    assert!(hands.says("warned []"), "{}", hands.app.outline());
}

/// What CSS-in-JS puts in a `<style>` a rule or a text node at a time is used as it comes, and `cssRules` are the rules as they
/// were written, with what's under `@media` and `!important`.
#[test]
fn rules_added_to_a_style_one_at_a_time_are_used() {
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        const style = document.createElement('style');
        document.head.appendChild(style);
        style.sheet.insertRule('.a { width: 30px; }', 0);
        style.sheet.insertRule('@media (min-width: 1px) { .b { width: 40px !important; } }', 1);
        style.sheet.insertRule('.b { width: 10px; }', 2);
        style.appendChild(document.createTextNode('.c { width: 50px; }'));
        const other = document.createElement('style');
        other.textContent = '.d { width: 70px; } .e { width: 80px; }';
        document.head.appendChild(other);
        other.sheet.deleteRule(0);
        const said = Array.from(style.sheet.cssRules).map((rule) => rule.cssText).join(' | ');
        export default function App() {
            return <PageWindow>
                <dom.div className="a">a</dom.div><dom.div className="b">b</dom.div><dom.div className="c">c</dom.div>
                <dom.div className="d">d</dom.div><dom.div className="e">e</dom.div>
                <dom.p>{'rules ' + style.sheet.cssRules.length + ': ' + said}</dom.p>
            </PageWindow>;
        }
    "#;
    let mut hands = open("style-one-at-a-time", &[("app.tsx", app)]);
    hands.settle();
    let laid_out = hands.app.rectangles();
    assert_eq!(size(&laid_out, "a").0, 30.0, "{laid_out}");
    assert_eq!(size(&laid_out, "b").0, 40.0, "{laid_out}");
    assert_eq!(size(&laid_out, "c").0, 50.0, "{laid_out}");
    assert_ne!(size(&laid_out, "d").0, 70.0, "{laid_out}");
    assert_eq!(size(&laid_out, "e").0, 80.0, "{laid_out}");
    assert!(
        hands.says("rules 4: .a { width: 30px; } | @media (min-width: 1px) { .b { width: 40px !important; } } | .b { width: 10px; } | .c { width: 50px; }"),
        "{}",
        hands.app.outline()
    );
}

/// Rules about ids, attributes, more than one class and what's inside something reach a page's elements whether or not they're
/// in a `PageWindow`, and `body` and `html` are what everything is in, with the classes they're given.
#[test]
fn rules_reach_elements_outside_a_page_window_and_start_from_body() {
    let css = "#one { width: 21px; }\ndiv#two { width: 22px; }\n[id=\"three\"] { width: 23px; }\n.wrap #four { width: 24px; }\n\
        .a.b { width: 25px; }\nbody.dark .themed { width: 26px; }\nhtml.dark > body .deep { width: 27px; }\n";
    let app = r#"
        import 'rsui/web';
        import './app.css';
        document.body.classList.add('dark');
        export default function App() {
            return <div className="wrap">
                <div id="one">one</div><div id="two">two</div><div id="three">three</div><div id="four">four</div>
                <div className="a b">ab</div><div className="themed">themed</div><div><div className="deep">deep</div></div>
            </div>;
        }
    "#;
    let mut hands = open("outside-page-window", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    let laid_out = hands.app.rectangles();
    for (text, width) in [("one", 21.0), ("two", 22.0), ("three", 23.0), ("four", 24.0), ("ab", 25.0), ("themed", 26.0), ("deep", 27.0)] {
        assert_eq!(size(&laid_out, text).0, width, "{text}\n{laid_out}");
    }
}

/// Which of two classes that say as much goes over the other is up to the order of their rules, sheet by sheet, and `!important`,
/// never the order they're written in `className`.
#[test]
fn classes_go_over_each_other_in_the_order_of_their_rules() {
    let css = ".p10 { width: 10px; }\n.p0 { width: 30px; }\n.imp0 { width: 30px !important; }\n.later10 { width: 10px; }\n";
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        import './first.css';
        import './second.css';
        export default function App() {
            return <PageWindow>
                <dom.div className="imp0 p10">a</dom.div><dom.div className="p10 imp0">b</dom.div>
                <dom.div className="later10 p0">c</dom.div><dom.div className="p0 p10">d</dom.div>
                <dom.div className="lib app">e</dom.div><dom.div className="app lib">f</dom.div>
            </PageWindow>;
        }
    "#;
    let mut hands = open(
        "class-order",
        &[("app.tsx", app), ("first.css", ".app { width: 50px; }\n"), ("second.css", &format!(".lib {{ width: 60px; }}\n{css}"))],
    );
    hands.settle();
    let laid_out = hands.app.rectangles();
    for (text, width) in [("a", 30.0), ("b", 30.0), ("c", 10.0), ("d", 30.0), ("e", 60.0), ("f", 60.0)] {
        assert_eq!(size(&laid_out, text).0, width, "{text}\n{laid_out}");
    }
}

/// `:has()` with more than one step is about what's inside something as that selector says, starting from the thing itself.
#[test]
fn has_with_several_steps_looks_inside() {
    let css = ".box:not(:has(.inner img.big)) { width: 31px; }
.box:has(.inner img.big) { width: 32px; }
        .row:has(> .a + .b) { width: 33px; }
.row:has(> .b .a) { width: 34px; }
";
    let laid_out = laid_out(
        "has-steps",
        css,
        r#"<dom.div className="box">without</dom.div><dom.div className="box"><dom.div className="inner"><dom.img className="big" width={4} height={4} /></dom.div>with</dom.div>
           <dom.div className="row"><dom.span className="a">x</dom.span><dom.span className="b">y</dom.span>sibling</dom.div>"#,
    );
    assert_eq!(size(&laid_out, "without").0, 31.0, "{laid_out}");
    // The box after the one without, which has the image in it.
    let without = laid_out.lines().position(|line| line.contains("\"without\"")).expect("without");
    assert!(laid_out.lines().nth(without + 1).is_some_and(|line| line.contains(",32,")), "{laid_out}");
    // The row, just before what's in it, which `> .a + .b` is about, and `> .b .a`, which comes later, isn't.
    let x = laid_out.lines().position(|line| line.contains("\"x\"")).expect("x");
    let row: Vec<&str> = laid_out.lines().nth(x - 1).unwrap().split_whitespace().nth(1).unwrap().split(',').collect();
    assert_eq!(row[2], "33", "{laid_out}");
}

/// `html` and `body` keep the attributes they're given, and the page's rules about them are about what's inside them.
#[test]
fn rules_about_attributes_on_html_and_body_apply() {
    let css = "html[data-theme=dark] .themed { width: 41px; }
body[data-size=\"big\"] .sized { width: 42px; }
";
    let app = r#"
        import { useState } from 'react';
        import { PageWindow, dom } from 'rsui/web';
        import './app.css';
        document.documentElement.setAttribute('data-theme', 'dark');
        export default function App() {
            const [big, setBig] = useState(false);
            return <PageWindow>
                <dom.div className="themed">themed</dom.div><dom.div className="sized">sized</dom.div>
                <button key="big" onClick={() => { document.body.dataset.size = 'big'; setBig(true); }}>{big ? 'made big' : 'make big'}</button>
            </PageWindow>;
        }
    "#;
    let mut hands = open("page-attributes", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    let laid_out = hands.app.rectangles();
    assert_eq!(size(&laid_out, "themed").0, 41.0, "{laid_out}");
    assert_ne!(size(&laid_out, "sized").0, 42.0, "{laid_out}");
    hands.click("big");
    hands.wait_for("made big");
    hands.settle();
    let laid_out = hands.app.rectangles();
    assert_eq!(size(&laid_out, "sized").0, 42.0, "{laid_out}");
}

/// Only what a `:hover` rule could be about keeps track of the pointer, and hovering it draws it again, not what's inside it, which
/// no rule about its being hovered is about.
#[test]
fn hovering_draws_again_only_what_a_hover_rule_is_about() {
    let css = ".tile { width: 100px; height: 40px; }
.tile:hover { width: 120px; }
.note { height: 10px; }
";
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        import './app.css';
        let drawn = 0;
        function Inside() { drawn += 1; return <dom.span className="note">{'inside drawn ' + drawn}</dom.span>; }
        export default function App() {
            return <PageWindow><dom.div className="tile" id="tile"><Inside /></dom.div><dom.div className="other" id="other">other</dom.div></PageWindow>;
        }
    "#;
    let mut hands = open("hover-cheap", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    let outline = hands.app.outline();
    let other = outline.lines().find(|line| line.contains("\"other\"")).expect("other");
    assert!(!other.contains("hover_change"), "{outline}");
    let tile = outline.lines().find(|line| line.contains("class=\"tile\"")).expect("tile");
    assert!(tile.contains("hover_change"), "{outline}");
    let rect: Vec<f32> = line(&hands.app.rectangles(), "inside drawn 1")
        .split_whitespace()
        .nth(1)
        .unwrap()
        .split(',')
        .map(|one| one.parse().unwrap())
        .collect();
    hands.move_at(rect[0] + 2.0, rect[1] + 2.0);
    hands.settle();
    let laid_out = hands.app.rectangles();
    assert!(laid_out.contains(",120,40"), "{laid_out}");
    assert!(hands.says("inside drawn 1"), "{}", hands.app.outline());
}

/// A page with rules about where things are among their kind, and components in lists whose elements are only known once they're
/// drawn, is drawn once when it starts, not once more for each of those things being found out.
#[test]
fn a_page_is_drawn_once_when_it_starts() {
    let css = ".row:first-of-type { width: 30px; }
.row:first-child .cell { width: 20px; }
.dark .cell { height: 5px; }
";
    let app = r#"
        import { useEffect, useState } from 'react';
        import { __styled as styled, PageWindow, dom } from 'rsui/web';
        import './app.css';
        let drawn = 0;
        const Counted = styled.span`width: ${() => { drawn += 1; return 10; }}px;`;
        function Row({ n }: { n: number }) {
            return <dom.div className="row"><Counted className="cell">{'cell ' + n}</Counted></dom.div>;
        }
        function Report() {
            const [said, setSaid] = useState(-1);
            useEffect(() => { setTimeout(() => setSaid(drawn), 100); }, []);
            return <dom.p>{said < 0 ? 'counting' : 'drawn ' + said + '.'}</dom.p>;
        }
        document.documentElement.classList.add('dark');
        export default function App() {
            return <PageWindow><dom.div className="list">{Array.from({ length: 20 }, (_, n) => <Row key={n} n={n} />)}</dom.div><Report /></PageWindow>;
        }
    "#;
    let mut hands = open("drawn-once", &[("app.tsx", app), ("app.css", css)]);
    hands.wait_for("drawn ");
    let outline = hands.app.outline();
    assert!(outline.contains("drawn 20."), "{outline}");
}

/// Rules about more than one class, and what's inside what, go over rules about one class or under them as the cascade says: by how
/// much they say about what they're for, and then by which comes later, whatever kind of rule each is.
#[test]
fn every_kind_of_rule_goes_in_the_cascade_order() {
    let css = ".x { width: 20px; }
div > .x { width: 10px; }
.y { width: 40px; }
[data-y] { width: 30px; }
        [data-z] { width: 60px; }
.z { width: 50px; }
.parent .styled { height: 7px; }
.styled { width: 70px; height: 9px; }
";
    let app = r#"
        import { __styled as styled, PageWindow, dom } from 'rsui/web';
        import './app.css';
        const Box = styled.div`width: 80px; height: 8px;`;
        export default function App() {
            return <PageWindow>
                <dom.span className="x">x</dom.span>
                <dom.div className="y" data-y="1">y</dom.div>
                <dom.div className="z" data-z="1">z</dom.div>
                <dom.div className="parent"><Box className="styled">styled</Box></dom.div>
            </PageWindow>;
        }
    "#;
    let mut hands = open("cascade-order", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    let laid_out = hands.app.rectangles();
    assert_eq!(size(&laid_out, "x").0, 20.0, "a class goes over what says less, written after it
{laid_out}");
    assert_eq!(size(&laid_out, "y").0, 30.0, "an attribute written later goes over a class
{laid_out}");
    assert_eq!(size(&laid_out, "z").0, 50.0, "a class written later goes over an attribute
{laid_out}");
    assert_eq!(size(&laid_out, "styled"), (80.0, 7.0), "a styled component's own CSS goes over a page's class, under what says more
{laid_out}");
}

/// Cached plans with many candidates keep their ordering when attribute rules are merged in, when the same classes are
/// written in another order, and when a new sheet invalidates the cached plan.
#[test]
fn indexed_plans_preserve_cascade_after_a_sheet_changes() {
    let filler = (0..20).map(|at| format!(".unused{at} .missing {{ width: 1px; }}\n")).collect::<String>();
    let css = format!("{filler}\n.parent .x {{ width: 20px; }}\n.parent [data-wide] {{ width: 30px; }}\n.parent .later {{ width: 40px; }}\n.parent .x {{ height: 12px !important; }}\n.parent .later {{ height: 25px; }}\n");
    let app = r#"
        import { PageWindow } from 'rsui/web';
        import './app.css';
        export default function App() {
            return <PageWindow><div className="parent">
                <div className="x" data-wide="">attribute</div>
                <div className="x later" data-wide="">forward</div>
                <div className="later x" data-wide="">reverse</div>
                <button key="add-sheet" onClick={() => {
                    const style = document.createElement('style');
                    style.textContent = '.parent .x { width: 55px; }';
                    document.head.appendChild(style);
                }}>Add sheet</button>
            </div></PageWindow>;
        }
    "#;
    let mut hands = open("indexed-plan-sheet", &[("app.tsx", app), ("app.css", &css)]);
    hands.settle();
    let before = hands.app.rectangles();
    for (text, width) in [("attribute", 30.0), ("forward", 40.0), ("reverse", 40.0)] {
        assert_eq!(size(&before, text), (width, 12.0), "{before}");
    }
    hands.click("add-sheet");
    hands.settle();
    let after = hands.app.rectangles();
    for text in ["attribute", "forward", "reverse"] {
        assert_eq!(size(&after, text), (55.0, 12.0), "{after}");
    }
}

#[test]
fn a_block_with_an_intrinsic_maximum_does_not_center_its_contents_across_the_page() {
    let css = ".nav { position: relative; display: flex; flex: 1 1 0%; max-width: max-content; align-items: center; justify-content: center; gap: 8px; }\n.left { width: 100px; height: 20px; } .right { width: 120px; height: 20px; }\n.panel { position: absolute; left: 0; top: 100%; width: 200px; height: 40px; }";
    let laid_out = laid_out("block-intrinsic-max", css, r#"
        <dom.div>origin</dom.div>
        <dom.nav className="nav"><dom.div className="left">left</dom.div><dom.div className="right">right</dom.div><dom.div className="panel">panel</dom.div></dom.nav>
    "#);
    let origin = rect(&laid_out, "origin");
    let left = rect(&laid_out, "left");
    let right = rect(&laid_out, "right");
    let panel = rect(&laid_out, "panel");
    assert_eq!(left[0], origin[0], "the intrinsic nav starts at its block's left edge\n{laid_out}");
    assert_eq!(right[0] - left[0], 108.0, "{laid_out}");
    assert_eq!(panel[0], left[0], "the absolute viewport shares the nav's left edge\n{laid_out}");
    assert_eq!(panel[1], left[1] + 20.0, "{laid_out}");
}

#[test]
fn pending_variables_follow_later_inline_declarations_and_important_overrides() {
    let css = ".box { --w: 30px; width: var(--w); height: var(--missing, 8px); }\n.box { height: 12px; }\n.box[data-on] { --w: 55px; width: var(--w) !important; }\n";
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        import { useState } from 'react';
        import './app.css';
        export default function App() {
            const [on, setOn] = useState(false);
            return <PageWindow>
                <dom.div className="box" data-on={on ? '' : undefined}
                    style={{ '--local': on ? '60px' : '40px', height: 'var(--local)' }}>variable box</dom.div>
                <button key="change-vars" onClick={() => setOn(!on)}>Change variables</button>
            </PageWindow>;
        }
    "#;
    let mut hands = open("pending-variables-cascade", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    assert_eq!(size(&hands.app.rectangles(), "variable box"), (30.0, 40.0));
    hands.click("change-vars");
    hands.settle();
    assert_eq!(size(&hands.app.rectangles(), "variable box"), (55.0, 60.0));
    hands.click("change-vars");
    hands.settle();
    assert_eq!(size(&hands.app.rectangles(), "variable box"), (30.0, 40.0));
}

#[test]
fn a_local_variable_equal_to_its_parent_still_shadows_later_parent_changes() {
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        import './app.css';
        export default function App() {
            return <PageWindow>
                <dom.div id="variable-parent" style={{ '--w': '20px' }}>
                    <dom.div className="read-width">inherited</dom.div>
                    <dom.div style={{ '--w': '20px' }}><dom.div className="read-width">shadowed</dom.div></dom.div>
                </dom.div>
                <button key="change-parent" onClick={() => document.getElementById('variable-parent').style.setProperty('--w', '40px')}>Change parent variable</button>
            </PageWindow>;
        }
    "#;
    let mut hands = open("equal-variable-shadow", &[("app.tsx", app), ("app.css", ".read-width { width: var(--w); height: 10px; }")]);
    hands.settle();
    assert_eq!(size(&hands.app.rectangles(), "inherited").0, 20.0);
    assert_eq!(size(&hands.app.rectangles(), "shadowed").0, 20.0);
    hands.click("change-parent");
    hands.settle();
    let after = hands.app.rectangles();
    assert_eq!(size(&after, "inherited").0, 40.0, "{after}");
    assert_eq!(size(&after, "shadowed").0, 20.0, "{after}");
}

#[test]
fn learning_a_new_inline_component_preserves_unchanged_sheet_declarations() {
    let css = ".sentinel-large-sheet .sheet-target { width: 35px; height: 20px; }\n";
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        import * as rsui from 'rsui';
        import { useState } from 'react';
        import './app.css';
        const natives = rsui.__host().css;
        const read = natives.cssRead;
        let reads = 0, atOpen = 0;
        natives.cssRead = (...args) => {
            if (String(args[0]).includes('sentinel-large-sheet')) reads++;
            return read(...args);
        };
        function LateInline() { return 'new inline'; }
        export default function App() {
            const [shown, setShown] = useState(false);
            const [said, setSaid] = useState('');
            return <PageWindow>
                <dom.div><dom.span>before</dom.span>{shown ? <LateInline /> : null}<dom.span>after</dom.span></dom.div>
                <dom.div className="sentinel-large-sheet"><dom.div className="sheet-target">sheet target</dom.div></dom.div>
                <button key="show-new" onClick={() => { atOpen = reads; setShown(true); }}>Show new component</button>
                <button key="read-count" onClick={() => setSaid('sheet reparses ' + (reads - atOpen))}>Read parse count</button>
                <dom.p>{said}</dom.p>
            </PageWindow>;
        }
    "#;
    let mut hands = open("flow-keeps-sheet", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    hands.click("show-new");
    hands.settle();
    hands.click("read-count");
    hands.settle();
    assert!(hands.says("sheet reparses 0"), "{}", hands.app.outline());
    assert_eq!(size(&hands.app.rectangles(), "sheet target"), (35.0, 20.0));
    assert!(hands.says("new inline"), "{}", hands.app.outline());
}

#[test]
fn resets_on_ungenerated_pseudo_elements_do_not_resolve_variables() {
    let css = r#"
        ::before, ::after { --pseudo-marker: 1; }
        .absent::before { width: 99px; height: 99px; }
        .none::before { content: none; }
        .normal::before { content: normal; }
        .empty::before { content: ''; display: block; width: 18px; height: 7px; }
        .deferred { --generated: 'made'; }
        .deferred::before { content: var(--generated); }
        .fallback::before { content: var(--missing, 'fallback'); }
        .quoted::before { content: 'none'; }
    "#;
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        import * as rsui from 'rsui';
        import { useState } from 'react';
        import './app.css';
        const natives = rsui.__host().css;
        const keys = natives.cssVariableKeys;
        let reads = 0;
        natives.cssVariableKeys = (token, look) => {
            if (look['--pseudo-marker'] !== undefined) reads++;
            return keys(token, look);
        };
        export default function App() {
            const [show, setShow] = useState(false);
            const [said, setSaid] = useState('');
            return <PageWindow>
                <dom.div className="absent">absent content</dom.div>
                <dom.div className="none">suppressed content</dom.div>
                <dom.div className="normal">normal content</dom.div>
                {show && <><dom.div className="empty">empty content</dom.div><dom.div className="deferred">deferred content</dom.div>
                    <dom.div className="fallback">fallback content</dom.div><dom.div className="quoted">quoted content</dom.div></>}
                <button key="read-pseudo" onClick={() => setSaid('pseudo reads ' + reads)}>Read pseudo work</button>
                <button key="show-pseudo" onClick={() => setShow(true)}>Generate pseudo elements</button>
                <dom.p>{said}</dom.p>
            </PageWindow>;
        }
    "#;
    let mut hands = open("unused-pseudo-vars", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    hands.click("read-pseudo");
    hands.settle();
    assert!(hands.says("pseudo reads 0"), "{}", hands.app.outline());
    hands.click("show-pseudo");
    hands.settle();
    let outline = hands.app.outline();
    assert!(outline.contains("rsui-before"), "empty content generates a box: {outline}");
    for text in ["made", "fallback", "none"] {
        assert!(outline.contains(&format!("text {text:?}")), "generated {text}: {outline}");
    }
}

#[test]
fn learning_a_lazily_mounted_kind_does_not_restyle_an_unrelated_section() {
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        import * as rsui from 'rsui';
        import { useState } from 'react';
        import './app.css';
        const natives = rsui.__host().css;
        const font = natives.cssOwnFont;
        let unrelated = 0, atOpen = 0;
        natives.cssOwnFont = (token, look, ...rest) => {
            if (look.width === 137) unrelated++;
            return font(token, look, ...rest);
        };
        function LateWords() { return 'late words'; }
        function Opener() {
            const [shown, setShown] = useState(false);
            return <dom.div><button key="mount-kind" onClick={() => { atOpen = unrelated; setShown(true); }}>Mount kind</button>
                <dom.div>{shown ? <LateWords /> : null}</dom.div></dom.div>;
        }
        export default function App() {
            const [said, setSaid] = useState('');
            return <PageWindow><dom.div className="unrelated">unrelated section</dom.div><Opener />
                <button key="read-unrelated" onClick={() => setSaid('unrelated restyles ' + (unrelated - atOpen))}>Read unrelated work</button>
                <dom.p>{said}</dom.p></PageWindow>;
        }
    "#;
    let mut hands = open("targeted-flow", &[("app.tsx", app), ("app.css", ".unrelated { width: 137px; height: 20px; }")]);
    hands.settle();
    hands.click("mount-kind");
    hands.settle();
    hands.click("read-unrelated");
    hands.settle();
    assert!(hands.says("unrelated restyles 0"), "{}", hands.app.outline());
    assert!(hands.says("late words"), "{}", hands.app.outline());
}

#[test]
fn ancestor_property_pseudos_change_the_owner_chain() {
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        import { useState } from 'react';
        import './app.css';
        export default function App() {
            const [disabled, setDisabled] = useState(false);
            return <PageWindow><dom.button key="toggle" onClick={() => setDisabled(!disabled)}>Toggle</dom.button>
                <dom.fieldset disabled={disabled}><dom.div className="child" key="child">Child</dom.div></dom.fieldset>
            </PageWindow>;
        }
    "#;
    let mut hands = open("cascade-ancestor-property", &[("app.tsx", app), ("app.css", ".child { width: 30px; height: 20px; } fieldset:disabled .child { width: 70px; }")]);
    hands.settle();
    for expected in [70.0, 30.0, 70.0, 30.0] {
        hands.click("toggle");
        assert_eq!(hands.rect_of("child")[2], expected, "{}", hands.app.rectangles());
    }
}

#[test]
fn an_absent_ancestor_does_not_hide_a_later_dynamic_sibling_rule() {
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        import { useState } from 'react';
        import './app.css';
        export default function App() {
            const [n, setN] = useState(0);
            return <PageWindow><dom.button key="next" onClick={() => setN(n + 1)}>Next</dom.button>
                <dom.div className={n >= 2 ? 'space' : 'outside'}>
                    <dom.div hidden={n === 3}>previous</dom.div><dom.div key="child" className="child">{'Child ' + n}</dom.div>
                </dom.div></PageWindow>;
        }
    "#;
    let mut hands = open("cascade-absent-ancestor", &[("app.tsx", app), ("app.css", ".child { width: 30px; height: 20px; } .space > :not([hidden]) ~ :not([hidden]) { width: 70px; }")]);
    hands.settle();
    for expected in [30.0, 70.0, 30.0, 70.0] {
        hands.click("next");
        assert_eq!(hands.rect_of("child")[2], expected, "{}", hands.app.rectangles());
    }
}

/// Where what says this is, and how big it is: `[x, y, width, height]`.
fn rect(laid_out: &str, text: &str) -> Vec<f32> {
    line(laid_out, text)
        .split_whitespace()
        .nth(1)
        .unwrap()
        .split(',')
        .map(|one| one.parse().unwrap())
        .collect()
}

/// A rem is the root's font size, which Pico sets with a variable on `:where(:root)`, a percentage that a media query changes.
#[test]
fn a_rem_is_the_root_font_size_a_variable_says() {
    let css = ":root { --size: 100%; }\n@media (min-width: 1px) { :root { --size: 125%; } }\n:where(:root) { font-size: var(--size); }\n\
        .r { width: 10rem; }\n";
    let laid_out = laid_out("rem-from-variable", css, r#"<dom.div className="r">rem</dom.div>"#);
    assert_eq!(size(&laid_out, "rem").0, 200.0, "{laid_out}");
}

/// `body`'s font size in em is a share of the root's, and a rem stays the root's.
#[test]
fn the_body_font_size_is_a_share_of_the_roots() {
    let css = "html { font-size: 20px; }\nbody { font-size: 0.5em; }\n.e { width: 10em; }\n.r { width: 10rem; }\n";
    let laid_out = laid_out("body-font-share", css, r#"<dom.div className="e">em</dom.div><dom.div className="r">rem</dom.div>"#);
    assert_eq!(size(&laid_out, "em").0, 100.0, "{laid_out}");
    assert_eq!(size(&laid_out, "rem").0, 200.0, "{laid_out}");
}

/// `<hgroup>` is an element of its own, which rules about what's in it reach.
#[test]
fn an_hgroup_is_an_element_rules_reach_inside() {
    let css = "hgroup > p { width: 50px; }\n";
    let laid_out = laid_out("hgroup", css, r#"<dom.hgroup><dom.h1>title</dom.h1><dom.p>under</dom.p></dom.hgroup><dom.p>outside</dom.p>"#);
    assert_eq!(size(&laid_out, "under").0, 50.0, "{laid_out}");
    assert_ne!(size(&laid_out, "outside").0, 50.0, "{laid_out}");
}

/// A grid row is as tall as what's in it is at the width it's stretched to, where a label's field of `width: 100%` goes on a
/// line below its text, as Pico's form in a `.grid` has it.
#[test]
fn a_grid_row_is_as_tall_as_its_label_with_a_wide_field() {
    let css = "body { margin: 0; font-size: 16px; line-height: 20px; }\n.grid { display: grid; grid-template-columns: 1fr 1fr; }\n\
        label { display: block; }\ninput { box-sizing: border-box; width: 100%; height: 50px; margin: 0; }\n";
    let laid_out = laid_out(
        "grid-label-rows",
        css,
        r#"<dom.div className="grid"><dom.label>first <dom.input /></dom.label><dom.label>second <dom.input /></dom.label></dom.div><dom.p>after</dom.p>"#,
    );
    // A line of text, then the 50px field under it, then the paragraph's 16px margin.
    assert!(rect(&laid_out, "after")[1] >= 86.0, "{laid_out}");
}
/// A closed `<details>` shows only its summary, which has a triangle before what it says unless CSS takes the marker away, as
/// Pico's `list-style-type: none` does.
#[test]
fn a_summary_has_a_marker_that_css_can_take_away() {
    let css = "body { margin: 0; }\n.plain summary { list-style-type: none; }\n";
    let laid_out = laid_out(
        "summary-marker",
        css,
        r#"<dom.details><dom.summary>marked</dom.summary><dom.p>hidden</dom.p></dom.details><dom.details className="plain"><dom.summary>unmarked</dom.summary></dom.details>"#,
    );
    assert!(!laid_out.contains("\"hidden\""), "what's in a closed details isn't shown: {laid_out}");
    assert!(rect(&laid_out, "marked")[0] > 10.0, "the triangle is before the text: {laid_out}");
    assert_eq!(rect(&laid_out, "unmarked")[0], 0.0, "{laid_out}");
}

/// A `::after` with nothing in it that floats, as Pico's chevron on a summary does, is beside the text rather than under it.
#[test]
fn an_empty_after_that_floats_is_beside_the_text() {
    let css = "body { margin: 0; line-height: 20px; }\n.x::after { content: ''; display: block; float: right; width: 16px; height: 16px; }\n";
    let laid_out = laid_out("empty-after-float", css, r#"<dom.div className="x">text</dom.div><dom.p>after</dom.p>"#);
    assert_eq!(rect(&laid_out, "after")[1], 36.0, "one line, then the paragraph's margin: {laid_out}");
}

/// `<progress>` is an element CSS about `progress` reaches, as wide as it says.
#[test]
fn a_progress_is_as_big_as_css_says() {
    let css = "progress { display: block; width: 300px; height: 9px; }\n";
    let app = "import { PageWindow, dom } from 'rsui/web';\nimport './app.css';\nexport default function App() { return <PageWindow><dom.progress value={60} max={100} /><dom.p>after</dom.p></PageWindow>; }\n";
    let mut hands = open("progress-css", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    let laid_out = hands.app.rectangles();
    let bar = laid_out.lines().find(|line| line.contains(",300,9")).unwrap_or_else(|| panic!("a 300 by 9 bar: {laid_out}"));
    assert!(bar.contains("dom="), "{laid_out}");
}
/// A `<fieldset>` and a `display: flow-root` box keep the margins of what's first and last in them inside them, as boxes that
/// lay out what's in them by themselves do in a browser, where a plain block's go out of it.
#[test]
fn a_fieldset_keeps_the_margins_of_whats_in_it() {
    let css = "body { margin: 0; line-height: 20px; }\nfieldset { margin: 0; padding: 0; border: 0; }\n.root { display: flow-root; }\n\
        .in { margin: 10px 0; }\n";
    let laid_out = laid_out(
        "fieldset-margins",
        css,
        r#"<dom.fieldset><dom.div className="in">one</dom.div></dom.fieldset><dom.div className="root"><dom.div className="in">two</dom.div></dom.div><dom.div><dom.div className="in">three</dom.div></dom.div>"#,
    );
    assert_eq!(rect(&laid_out, "one")[1], 10.0, "{laid_out}");
    // Under the fieldset's 40 and inside the flow-root's own 10.
    assert_eq!(rect(&laid_out, "two")[1], 50.0, "{laid_out}");
    // The flow-root ends at 80, and a plain block's margin goes out of it, to go together with nothing above.
    assert_eq!(rect(&laid_out, "three")[1], 90.0, "{laid_out}");
}
/// `:where()` says nothing about how much a rule says, and `:is()` says as much as the most in it does: Pico's
/// `:where(ol, ul) li { margin-bottom }` gives way to a later `nav li { margin: 0 }`.
#[test]
fn where_says_nothing_and_is_says_its_most() {
    let css = ":where(ul) li { width: 40px; }\nli { width: 20px; }\n:is(#n, .m) li { width: 60px; }\n.k li { width: 30px; }\n";
    let laid_out = laid_out(
        "where-is-specificity",
        css,
        r#"<dom.ul><dom.li>plain</dom.li></dom.ul><dom.ul id="n" className="k"><dom.li>by id</dom.li></dom.ul>"#,
    );
    // The item itself, as its one word can go past its end.
    let items: Vec<&str> = laid_out.lines().filter(|line| line.trim_start().starts_with("list_item")).collect();
    assert!(items.len() == 2 && items[0].contains(",20,") && items[1].contains(",60,"), "{laid_out}");
}
/// `font-weight: bolder` is a step bolder than what's around it, as Pico's `strong` is, and `lighter` a step lighter.
#[test]
fn bolder_is_a_step_from_the_weight_around_it() {
    let css = ".b { font-weight: bolder; }\n.heavy { font-weight: 600; }\n.l { font-weight: lighter; }\n";
    let laid_out = laid_out(
        "bolder-weight",
        css,
        r#"<dom.div><dom.span>Same</dom.span></dom.div><dom.div><dom.span className="b">Same</dom.span></dom.div><dom.div className="heavy"><dom.span className="l">Same</dom.span></dom.div>"#,
    );
    // Each "Same", in the order they're in: as it is, bold, and 400, a step lighter than 600.
    let widths: Vec<f32> = laid_out
        .lines()
        .filter(|line| line.contains("\"Same\""))
        .map(|line| line.split_whitespace().nth(1).unwrap().split(',').nth(2).unwrap().parse().unwrap())
        .collect();
    assert_eq!(widths.len(), 3, "{laid_out}");
    assert!(widths[1] > widths[0], "bolder than normal is bold: {laid_out}");
    assert_eq!(widths[2], widths[0], "lighter than 600 is normal: {laid_out}");
}
/// A `<progress>` after a block is on a line of its own, which goes below the bottom of its margin to leave room for what goes
/// under a line of text, as an inline-block does, even when a rule about where things are among each other, as Pico's
/// `li:not(:last-child)` is, has everything told where it is.
#[test]
fn a_progress_after_a_block_is_on_a_line() {
    let css = "body { margin: 0; line-height: 20px; }\np { margin: 0; }\n.x:last-child { color: red; }\n\
        progress { display: inline-block; width: 100px; height: 9px; margin-bottom: 9px; }\n";
    let app = "import { PageWindow, dom } from 'rsui/web';\nimport './app.css';\nexport default function App() { return <PageWindow><dom.div><dom.p>above</dom.p><dom.progress value={60} max={100} /></dom.div><dom.p>after</dom.p></PageWindow>; }\n";
    let mut hands = open("progress-line", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    let laid_out = hands.app.rectangles();
    // Its margin, 18 in all, sits on the line's text, which has a little more below it, rather than going out of what it's in.
    assert!(rect(&laid_out, "after")[1] >= 40.0, "{laid_out}");
}
/// A media query list holds if any query in it does, as Bulma's `screen and (min-width: 1px), print` does; `print` alone, `not
/// screen` and a feature that isn't so don't.
#[test]
fn a_media_query_list_holds_if_any_of_it_does() {
    let css = "@media screen and (min-width: 1px), print { .a { width: 10px; } }\n@media print { .b { width: 20px; } }\n\
        @media not screen { .c { width: 30px; } }\n@media (prefers-reduced-transparency: no-preference) and (hover: hover) { .d { width: 40px; } }\n";
    let laid_out = laid_out(
        "media-list",
        css,
        r#"<dom.div className="a">a</dom.div><dom.div className="b">b</dom.div><dom.div className="c">c</dom.div><dom.div className="d">d</dom.div>"#,
    );
    assert_eq!(size(&laid_out, "a").0, 10.0, "{laid_out}");
    assert_ne!(size(&laid_out, "b").0, 20.0, "{laid_out}");
    assert_ne!(size(&laid_out, "c").0, 30.0, "{laid_out}");
    assert_eq!(size(&laid_out, "d").0, 40.0, "{laid_out}");
}

/// `* { box-sizing: inherit }` takes `html`'s `border-box`, as Bulma's does, on what's right inside the page and on fields.
#[test]
fn box_sizing_inherit_takes_what_html_says() {
    let css = "html { box-sizing: border-box; }\n*, ::before, ::after { box-sizing: inherit; }\n\
        .w { width: 100px; padding: 0 10px; }\ninput { width: 100px; padding: 0 10px; border: 0; }\n";
    let laid_out = laid_out("box-sizing-inherit", css, r#"<dom.div className="w">top</dom.div><dom.div><dom.input placeholder="field" /></dom.div>"#);
    assert_eq!(size(&laid_out, "top").0, 100.0, "padding and all, rather than 120: {laid_out}");
    let field = laid_out.lines().find(|line| line.contains("text_input")).unwrap_or_else(|| panic!("{laid_out}"));
    assert!(field.contains(",100,"), "as wide as it says, padding and all: {laid_out}");
}

/// `flex: none` after a rule that says `flex-basis: 0` starts from how big it is, as Bulma's `.column.is-two-thirds` does.
#[test]
fn flex_none_undoes_a_basis_before_it() {
    let css = "body { margin: 0; }\n.row { display: flex; width: 300px; }\n.c { flex-basis: 0; flex-grow: 1; }\n.t { flex: none; width: 200px; }\n";
    let laid_out = laid_out("flex-none-basis", css, r#"<dom.div className="row"><dom.div className="c">grows</dom.div><dom.div className="c t">fixed</dom.div></dom.div>"#);
    assert_eq!(size(&laid_out, "fixed").0, 200.0, "{laid_out}");
    assert_eq!(size(&laid_out, "grows").0, 100.0, "{laid_out}");
}

/// `:has(+ .x)` is about what comes right after, as Bulma's `.title:has(+ .subtitle)` is, and sibling rules reach what's right
/// inside the page too.
#[test]
fn has_reaches_what_comes_after() {
    let css = "body { margin: 0; }\np { margin: 0; }\n.t { margin-bottom: 24px; }\n.t:has(+ .s) { margin-bottom: 0; }\n";
    let laid_out = laid_out(
        "has-after",
        css,
        r#"<dom.p className="t">Title</dom.p><dom.p className="s">Sub</dom.p><dom.p className="t">Alone</dom.p><dom.p>other</dom.p>"#,
    );
    let title = rect(&laid_out, "Title");
    assert_eq!(rect(&laid_out, "Sub")[1], title[1] + title[3], "{laid_out}");
    let alone = rect(&laid_out, "Alone");
    assert_eq!(rect(&laid_out, "other")[1], alone[1] + alone[3] + 24.0, "{laid_out}");
}
/// `min-width: 100%` is all of what it's in, on a block and on a field, as Bulma's textarea has it.
#[test]
fn a_min_width_can_be_a_share_of_what_its_in() {
    let css = "body { margin: 0; }\n.w { width: 300px; }\n.b { width: 100px; min-width: 100%; }\n\
        textarea { display: block; width: 100px; min-width: 100%; box-sizing: border-box; }\n";
    let laid_out = laid_out("min-width-share", css, r#"<dom.div className="w"><dom.div className="b">block</dom.div><dom.textarea placeholder="area" /></dom.div>"#);
    assert_eq!(size(&laid_out, "block").0, 300.0, "{laid_out}");
    let area = laid_out.lines().find(|line| line.contains("\"area\"")).unwrap_or_else(|| panic!("{laid_out}"));
    assert!(area.contains(" 0,") && area.contains(",300,"), "{laid_out}");
}

/// A textarea is two rows unless it says, as a browser's is, and a rule about what it's in that gives it a `min-height` makes
/// that the height inside its padding and border, as `box-sizing: content-box` says, as a rule about its class does.
#[test]
fn a_textarea_is_two_rows_and_takes_a_min_height_from_any_rule() {
    let css = "body { margin: 0; }\ntextarea { display: block; font: 10px monospace; line-height: 10px; padding: 0; border: 0; }\n\
        .w .tall { min-height: 50px; padding: 5px; }\n";
    let laid_out = laid_out("textarea-two-rows", css, r#"<dom.textarea placeholder="plain" /><dom.div className="w"><dom.textarea className="tall" placeholder="tall" /></dom.div>"#);
    let height = |text: &str| {
        let line = laid_out.lines().find(|line| line.contains(&format!("{text:?}"))).unwrap_or_else(|| panic!("{text}: {laid_out}"));
        line.split_whitespace().nth(1).unwrap().split(',').nth(3).unwrap().parse::<f32>().unwrap()
    };
    assert_eq!(height("plain"), 20.0, "{laid_out}");
    assert_eq!(height("tall"), 60.0, "{laid_out}");
}
/// A font family is the first of its list that's installed, as MUI's `"Roboto", "Helvetica", "Arial"` is Arial where Roboto
/// isn't, as in a browser.
#[test]
fn a_font_list_uses_the_first_font_there_is() {
    let css = ".list { font-family: \"No Such Font At All\", Helvetica-Not-Here, Arial, sans-serif; }\n.arial { font-family: Arial; }\n";
    let laid_out = laid_out("font-list", css, r#"<dom.div><dom.span className="list">Wide words</dom.span></dom.div><dom.div><dom.span className="arial">Wide words.</dom.span></dom.div>"#);
    let listed = size(&laid_out, "Wide words").0;
    let arial = size(&laid_out, "Wide words.").0;
    assert!(arial > listed && arial - listed < 6.0, "the same font, with a full stop more: {listed} {arial}\n{laid_out}");
}

/// A class's `box-sizing` goes over `* { box-sizing: inherit }`, as MUI's input's `content-box` does under CssBaseline.
#[test]
fn a_value_goes_over_inherit_from_a_rule_before_it() {
    let css = "html { box-sizing: border-box; }\n*, ::before, ::after { box-sizing: inherit; }\n\
        .c { box-sizing: content-box; width: 100px; padding: 0 10px; }\n.b { width: 100px; padding: 0 10px; }\n";
    let laid_out = laid_out("inherit-overridden", css, r#"<dom.div className="c">content</dom.div><dom.div className="b">border</dom.div>"#);
    assert_eq!(size(&laid_out, "content").0, 120.0, "{laid_out}");
    assert_eq!(size(&laid_out, "border").0, 100.0, "{laid_out}");
}

/// A component that makes something that goes along a line, as MUI's `Button` makes an inline-flex `<button>`, is found to, and
/// what it's in puts two of them on one line from then on.
#[test]
fn a_component_found_to_make_something_inline_goes_on_a_line() {
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        import './app.css';
        function Pill({ children }: { children: string }) {
            return <dom.span className="pill">{children}</dom.span>;
        }
        export default function App() {
            return <PageWindow><dom.div><Pill>first</Pill><Pill>second</Pill></dom.div></PageWindow>;
        }
    "#;
    let css = "body { margin: 0; }\n.pill { display: inline-flex; padding: 0 8px; }\n";
    let mut hands = open("learned-inline", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    let laid_out = hands.app.rectangles();
    let (first, second) = (rect(&laid_out, "first"), rect(&laid_out, "second"));
    assert_eq!(first[1], second[1], "side by side: {laid_out}");
    assert!(second[0] > first[0], "{laid_out}");
}

/// `thin`, `medium` and `thick` are 1, 3 and 5 pixels of border, as MUI's Divider's `border-bottom-width: thin` is.
#[test]
fn border_width_words_are_pixels() {
    let css = "body { margin: 0; }\n.t { border: 0 solid black; border-bottom-width: thin; }\n.m { border: medium solid black; }\n";
    let laid_out = laid_out("border-words", css, r#"<dom.div className="t">thin</dom.div><dom.div className="m">medium</dom.div>"#);
    let (thin, medium) = (rect(&laid_out, "thin"), rect(&laid_out, "medium"));
    assert_eq!(medium[1] - thin[1] - thin[3], 0.0, "{laid_out}");
    let text_height = thin[3] - 1.0;
    assert_eq!(medium[3], text_height + 6.0, "{laid_out}");
}

/// A fieldset's legend is as wide as what's in it, even as a block, and a legend anywhere else is as wide as a block is.
#[test]
fn a_fieldsets_legend_is_as_wide_as_whats_in_it() {
    let css = "body { margin: 0; }\nfieldset { margin: 0; padding: 0; border: 0; }\nlegend { display: block; width: auto; padding: 0; }\n";
    let laid_out = laid_out("legend-fit", css, r#"<dom.fieldset><dom.legend>inside</dom.legend></dom.fieldset><dom.legend>outside</dom.legend>"#);
    assert!(size(&laid_out, "inside").0 < 100.0, "{laid_out}");
    assert_eq!(size(&laid_out, "outside").0, 500.0, "{laid_out}");
}
/// An `<svg>` that says only how wide it is is as tall as its `viewBox` says for that, even placed between two insets, as
/// Mantine's checkbox tick is.
#[test]
fn an_svg_is_as_tall_as_its_view_box_says() {
    let css = "body { margin: 0; }\n.box { position: relative; width: 100px; height: 100px; }\n\
        .tick { position: absolute; top: 0; bottom: 0; left: 0; right: 0; margin: auto; width: 50%; }\n.flow { width: 40px; }\n";
    let app = r#"import { PageWindow, dom } from 'rsui/web';
import './app.css';
export default function App() {
    return <PageWindow>
        <dom.div className="box"><dom.svg className="tick" viewBox="0 0 10 7"><dom.path d="M0 0L10 7" /></dom.svg></dom.div>
        <dom.svg className="flow" viewBox="0 0 4 1"><dom.path d="M0 0L4 1" /></dom.svg>
    </PageWindow>;
}
"#;
    let mut hands = open("svg-view-box", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    let laid_out = hands.app.rectangles();
    assert!(laid_out.contains(" 25,32.5,50,35"), "50 wide, 35 tall, in the middle: {laid_out}");
    assert!(laid_out.contains(",40,10"), "40 wide and 10 tall: {laid_out}");
}

/// What's said on `:where(html, .theme)`, as Chakra's tokens are, is the page's, and the class's too.
#[test]
fn where_with_a_list_is_each_of_it() {
    let css = ":where(html, .theme) { --gap: 30px; }\n.a { width: var(--gap); }\n:is(.b, .c) { width: 50px; }\n";
    let laid_out = laid_out("where-list", css, r#"<dom.div className="a">a</dom.div><dom.div className="c">c</dom.div>"#);
    assert_eq!(size(&laid_out, "a").0, 30.0, "{laid_out}");
    assert_eq!(size(&laid_out, "c").0, 50.0, "{laid_out}");
}

/// A custom property's name can have an escape in it, as Chakra's `--chakra-spacing-1\.5` does.
#[test]
fn a_variable_name_can_have_an_escape_in_it() {
    let css = ":root { --space-1\\.5: 24px; }\n.a { width: var(--space-1\\.5); }\n";
    let laid_out = laid_out("escaped-variable", css, r#"<dom.div className="a">a</dom.div>"#);
    assert_eq!(size(&laid_out, "a").0, 24.0, "{laid_out}");
}
/// An inline-block `<span>` in text that's a block, as a `<p>` or a fieldset's `<legend>` is, is a box with its padding, even
/// when a rule about more than one element makes it one, as MUI's `.css-x > span` does.
#[test]
fn an_inline_block_in_a_paragraph_keeps_its_padding() {
    let css = "body { margin: 0; font: 12px/20px Arial; }\np { margin: 0; }\n.ib { display: inline-block; padding: 0 5px; }\n\
        fieldset { margin: 0; padding: 0; border: 0; }\nlegend { padding: 0; }\n.l > span { display: inline-block; padding: 0 7px; }\n";
    let laid_out = laid_out(
        "inline-block-in-text",
        css,
        r#"<dom.p><dom.span className="ib">padded</dom.span></dom.p><dom.p><dom.span>plain</dom.span></dom.p><dom.fieldset><dom.legend className="l"><dom.span>legend</dom.span></dom.legend></dom.fieldset>"#,
    );
    let plain = size(&laid_out, "plain").0;
    let padded = size(&laid_out, "padded");
    assert!(padded.0 > plain + 10.0 && padded.1 == 20.0, "its text and 10 more, a line tall: {laid_out}");
    assert!(size(&laid_out, "legend").0 > 40.0, "with 14 of padding: {laid_out}");
}

/// An inline-block too wide for its line of text goes past its end, rather than getting narrower than what's in it.
#[test]
fn an_inline_block_wider_than_its_line_goes_past_it() {
    let css = "body { margin: 0; }\n.w { width: 10px; }\n.ib { display: inline-block; white-space: nowrap; padding: 0 5px; }\n";
    let laid_out = laid_out("inline-block-overflows", css, r#"<dom.div className="w">a <dom.span className="ib">much too wide</dom.span></dom.div>"#);
    assert!(size(&laid_out, "much too wide").0 > 50.0, "{laid_out}");
}

/// A legend made through a component is the fieldset's legend all the same, as MUI's Emotion-made one is.
#[test]
fn a_legend_made_by_a_component_is_the_fieldsets() {
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        import './app.css';
        function Legend({ children }: { children: string }) {
            return <dom.legend className="l">{children}</dom.legend>;
        }
        export default function App() {
            return <PageWindow><dom.fieldset><Legend>made</Legend></dom.fieldset></PageWindow>;
        }
    "#;
    let css = "body { margin: 0; }\nfieldset { margin: 0; padding: 0; border: 0; }\n.l { display: block; width: auto; padding: 0; }\n";
    let mut hands = open("legend-component", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    let laid_out = hands.app.rectangles();
    assert!(size(&laid_out, "made").0 < 100.0, "{laid_out}");
}
/// An `<svg>` in a flex box gets narrower than it says to fit, as what clips what's in it can, as Edge has shadcn's checkbox
/// tick, and an inline-block by itself in a block that's too narrow for it goes past its end.
#[test]
fn an_svg_shrinks_in_a_flex_box_and_a_lone_inline_block_goes_past_its_block() {
    let css = "body { margin: 0; }\n.f { display: flex; width: 14px; }\n.s { width: 16px; height: 16px; display: block; }\n\
        .w { width: 10px; }\n.ib { display: inline-block; white-space: nowrap; }\n";
    let app = r#"import { PageWindow, dom } from 'rsui/web';
import './app.css';
export default function App() {
    return <PageWindow>
        <dom.div className="f"><dom.svg className="s" viewBox="0 0 24 24"><dom.path d="M0 0L24 24" /></dom.svg></dom.div>
        <dom.div className="w"><dom.span className="ib">much too wide</dom.span></dom.div>
    </PageWindow>;
}
"#;
    let mut hands = open("svg-shrink-lone-block", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    let laid_out = hands.app.rectangles();
    assert!(laid_out.contains(" 0,0,14,16"), "the svg as wide as its box: {laid_out}");
    assert!(size(&laid_out, "much too wide").0 > 50.0, "{laid_out}");
}

/// An inline element's box is as tall as its font makes it, as CSS's inline box is, wherever its line puts its text, and a
/// block's lone inline element is on a line as tall as the block's own text would be.
#[test]
fn an_inline_box_is_as_tall_as_its_font() {
    let css = "body { margin: 0; font: 16px Arial; line-height: 40px; }\nsmall { font-size: 10px; line-height: 12px; }\n";
    let laid_out = laid_out("inline-area", css, r#"<dom.div>in <dom.span>span</dom.span> text</dom.div><dom.div><dom.small>small</dom.small></dom.div><dom.p>after</dom.p>"#);
    let span = rect(&laid_out, "span");
    assert!(span[3] < 30.0 && span[1] > 5.0, "the font's height, not the line's 40, in the middle of it: {laid_out}");
    let small = rect(&laid_out, "small");
    assert!(rect(&laid_out, "after")[1] - small[1] >= 40.0 - 12.0, "the small's line is 40 tall: {laid_out}");
}

/// `vertical-align: middle` puts something's middle half the height of an `x` above its line's baseline, as CSS has it, which
/// is lower than the middle of a line whose text sits low in it.
#[test]
fn vertical_align_middle_goes_by_the_x_height() {
    let css = "body { margin: 0; font: 20px Arial; line-height: 40px; }\n.box { display: inline-block; width: 10px; height: 10px; vertical-align: middle; }\n";
    let laid_out = laid_out("valign-middle", css, r#"<dom.div>xx <dom.span className="box"></dom.span> yy</dom.div>"#);
    let boxed = laid_out.lines().find(|line| line.contains(",10,10")).unwrap_or_else(|| panic!("{laid_out}"));
    let top: f32 = boxed.split_whitespace().nth(1).unwrap().split(',').nth(1).unwrap().parse().unwrap();
    assert!(top > 15.0 && top < 20.0, "below the line's middle (15), by the text's baseline less half an x: {laid_out}");
}
/// What a component gives back as only text, as Ant Design's Typography does, is written as the element it's in says, in its
/// font and on its lines, through a component that gives back only that component, and once the page has been told where things
/// are among the rest, as a rule about what comes after something has it.
#[test]
fn text_a_component_gives_back_is_written_as_its_element_says() {
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        import './app.css';
        function Words({ children }: { children: string }) { return <>{[children]}{[false, null, [undefined]]}</>; }
        function Shell({ children }: { children: any }) { return children; }
        export default function App() {
            return <PageWindow>
                <dom.h4 className="h"><Words>Acme</Words></dom.h4>
                <dom.div className="p"><Shell><Words>para</Words></Shell></dom.div>
                <dom.p className="after">after</dom.p>
            </PageWindow>;
        }
    "#;
    let css = "body { margin: 0; font: 16px Arial; }\n.h { margin: 0; font-size: 30px; line-height: 40px; }\n\
        .p { line-height: 30px; }\n.after { margin: 0; }\n.a + .b { color: red; }\n";
    let mut hands = open("component-text-font", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    let laid_out = hands.app.rectangles();
    assert!(size(&laid_out, "Acme").0 > 55.0, "30px, not 16: {laid_out}");
    assert!(rect(&laid_out, "after")[1] >= 69.0, "the heading's 40, and a line of 30: {laid_out}");
}

/// A button's text is as far apart as its font's own lines, as a browser's `font` for it says, rather than what it's in says,
/// unless the page says it's that; and an inline-flex button sits on a tall line by its text, in the middle of the button.
#[test]
fn a_button_has_its_own_line_height_and_sits_on_its_line_by_its_text() {
    let css = "body { margin: 0; font: 14px Arial; line-height: 64px; }\n\
        .b { display: inline-flex; align-items: center; height: 32px; padding: 0 4px; border: 1px solid; box-sizing: border-box; font: 14px Arial; }\n\
        .i { line-height: inherit; }\n";
    let jsx = r#"<dom.div><dom.button className="b"><dom.span>go</dom.span></dom.button></dom.div><dom.div><dom.button className="b i">x</dom.button></dom.div>"#;
    let laid_out = laid_out("button-line-height", css, jsx);
    let go = rect(&laid_out, "go");
    assert!(go[3] < 25.0, "the font's own height, not 64: {laid_out}");
    // The line's baseline is about 37 down, and the text's is about 21 down in the button, which is 16.
    let button = laid_out.lines().find(|line| line.trim_start().starts_with("button")).unwrap_or_else(|| panic!("{laid_out}"));
    let top: f32 = button.split_whitespace().nth(1).unwrap().split(',').nth(1).unwrap().parse().unwrap();
    assert!((14.0..19.0).contains(&top), "by its text, which is in its middle: {laid_out}");
    assert!(size(&laid_out, "x").1 > 25.0, "`inherit` is what it's in says, not the font's own 16: {laid_out}");
}

/// What's in a block isn't a flex item, so `flex: auto` doesn't make it grow, as Ant Design's `Layout` says it in the page.
#[test]
fn flex_auto_in_a_block_does_not_grow() {
    let css = "body { margin: 0; font: 16px Arial; }\n.l { flex: auto; display: flex; flex-direction: column; }\np { margin: 0; }\n";
    let laid_out = laid_out("flex-auto-block", css, r#"<dom.div className="l">short</dom.div><dom.p>after</dom.p>"#);
    assert!(rect(&laid_out, "after")[1] < 40.0, "{laid_out}");
}

/// `a + b` asks about the classes of what a component made before it, as Ant Design's `.ant-checkbox + span` does of
/// rc-checkbox's `<span class="ant-checkbox">`.
#[test]
fn what_comes_after_an_element_a_component_made_knows_its_classes() {
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        import './app.css';
        function Mark() { return <dom.span className="mark">x</dom.span>; }
        export default function App() {
            return <PageWindow><dom.label><Mark /><dom.span>label</dom.span></dom.label></PageWindow>;
        }
    "#;
    let css = "body { margin: 0; font: 16px Arial; }\n.mark + span { font-size: 30px; }\n";
    let mut hands = open("sibling-of-component", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    let laid_out = hands.app.rectangles();
    let (x, label) = (rect(&laid_out, "x"), rect(&laid_out, "label"));
    assert!(label[2] > 50.0, "30px, not 16: {laid_out}");
    assert!(label[0] >= x[0] + x[2] - 0.5, "{laid_out}");
}

/// A component that's all there is in a block, and turns out to make something inline, is on a line as tall as the block's
/// text, and isn't drawn again for being found out.
#[test]
fn a_lone_component_that_makes_something_inline_is_on_a_line() {
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        import './app.css';
        let drawn = 0;
        function Go() { drawn += 1; return <dom.button className="b">{'go ' + drawn}</dom.button>; }
        export default function App() {
            return <PageWindow><dom.div><Go /></dom.div><dom.p>after</dom.p></PageWindow>;
        }
    "#;
    let css = "body { margin: 0; font: 16px Arial; line-height: 50px; }\n.b { display: inline-flex; height: 20px; line-height: 20px; }\np { margin: 0; }\n";
    let mut hands = open("lone-inline-component", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    let laid_out = hands.app.rectangles();
    assert!(rect(&laid_out, "after")[1] >= 49.0, "a line 50 tall: {laid_out}");
    assert!(hands.says("go 1"), "{}", hands.app.outline());
}

/// A grid that says nothing of its columns has one that's only as wide as what's in it when `justify-content` says where it
/// goes, as Radix's Callout does, and that fills it otherwise.
#[test]
fn a_grid_with_no_columns_is_as_wide_as_what_is_in_it_when_justify_content_says() {
    let css = "body { margin: 0; font: 16px Arial; }\np { margin: 0; }\n.g { display: grid; justify-content: flex-start; }\n\
        .n { display: grid; justify-content: normal; }\n";
    let laid_out = laid_out("grid-justify-content", css, r#"<dom.div className="g"><dom.p>short</dom.p></dom.div><dom.div className="n"><dom.p>filled</dom.p></dom.div>"#);
    assert!(size(&laid_out, "short").0 < 100.0, "{laid_out}");
    assert!(size(&laid_out, "filled").0 > 400.0, "{laid_out}");
}

/// A gradient whose colors all come to nothing is none, rather than stopping the page, and there's a `ShadowRoot`, which
/// Ant Design asks whether something is.
#[test]
fn an_empty_gradient_is_none_and_there_is_a_shadow_root() {
    let css = "body { margin: 0; font: 16px Arial; }\n.e { height: 10px; background-image: linear-gradient(var(--nope), var(--nope)); }\n";
    let jsx = r#"<dom.div className="e"></dom.div><dom.p>{'shadow ' + typeof ShadowRoot}</dom.p>"#;
    let laid_out = laid_out("empty-gradient", css, jsx);
    line(&laid_out, "shadow function");
}

/// `all: unset` takes away what the browser says a button looks like, as Radix's `.rt-reset` does, and what comes after it in
/// the same rule still says what it says.
#[test]
fn all_unset_takes_away_a_buttons_own_look() {
    let css = "body { margin: 0; font: 16px Arial; }\n.r { all: unset; display: inline-block; }\n.k { all: unset; color: #ff0000; }\n";
    let laid_out = laid_out("all-unset", css, r#"<dom.div><dom.button className="r">go</dom.button></dom.div><dom.div><dom.button className="k">red</dom.button></dom.div>"#);
    let go = laid_out.lines().find(|line| line.trim_start().starts_with("button")).unwrap_or_else(|| panic!("{laid_out}"));
    assert!(!go.contains("efefef"), "no gray of its own: {laid_out}");
    assert!(rect(&laid_out, "go")[0] < 0.5, "no padding or border of its own: {laid_out}");
    assert!(line(&laid_out, "red").contains("text_color=#ff0000"), "{laid_out}");
}

/// A font `@font-face` makes of one the system has, with `local()`, is that font, in the weight its name says, as big as
/// `size-adjust` makes it, as Radix's "Segoe UI (Custom)" is; a family that names Mac fonts first goes past them to it.
#[test]
fn a_local_font_face_is_the_systems_font_sized_as_it_says() {
    let css = "@font-face { font-family: 'Mine'; src: local('Arial Bold'); size-adjust: 200%; }\n\
        body { margin: 0; }\np { margin: 0; }\n.m { font-family: -apple-system, BlinkMacSystemFont, 'Mine', serif; font-size: 10px; }\n\
        .a { font: bold 20px Arial; }\n";
    let laid_out = laid_out("local-font-face", css, r#"<dom.p className="m">abc</dom.p><dom.p className="a">bca</dom.p>"#);
    let (made, real) = (size(&laid_out, "abc").0, size(&laid_out, "bca").0);
    assert!((made - real).abs() < 1.0, "bold Arial at 20px either way, {made} and {real}: {laid_out}");
}

/// `::before` uses the variables set on its own element, as Radix's checkbox's box does.
#[test]
fn before_uses_its_elements_variables() {
    let css = "body { margin: 0; font: 16px Arial; }\n.v { --s: 16px; display: inline-flex; }\n\
        .v::before { content: ''; display: block; width: var(--s); height: var(--s); }\n";
    let laid_out = laid_out("before-variables", css, r#"<dom.div><dom.span className="v"></dom.span><dom.span>after</dom.span></dom.div>"#);
    assert!(rect(&laid_out, "after")[0] >= 15.5, "after the 16 wide box: {laid_out}");
}

/// A span in text takes room for its padding and border at its sides, and its box takes them in up and down too, without its
/// line being any taller, as CSS's inline boxes have them.
#[test]
fn a_span_in_text_takes_room_for_its_padding() {
    let css = "body { margin: 0; font: 16px Arial; }\np { margin: 0; }\n.q { padding: 0 10px; border: 2px solid #888888; }\n";
    let laid_out = laid_out("inline-padding", css, r#"<dom.p>x <dom.span>same</dom.span> y</dom.p><dom.p>x <dom.span className="q">same</dom.span> y</dom.p>"#);
    let boxes: Vec<Vec<f32>> = laid_out
        .lines()
        .filter(|line| line.contains("\"same\""))
        .map(|line| line.split_whitespace().nth(1).unwrap().split(',').map(|one| one.parse().unwrap()).collect())
        .collect();
    assert_eq!(boxes.len(), 2, "{laid_out}");
    let (plain, padded) = (&boxes[0], &boxes[1]);
    assert!((padded[2] - plain[2] - 24.0).abs() < 0.5, "10 of padding and 2 of border each side: {laid_out}");
    assert!((padded[3] - plain[3] - 4.0).abs() < 0.5, "its border up and down: {laid_out}");
    // The second line starts a line's height down, and the padded box 2 above its text.
    assert!((padded[1] - plain[1] - (plain[3] - 2.0)).abs() < 1.5, "lines as tall as without it: {laid_out}");
}

/// Text in an inline-flex box is as tall as its line height, past the box, which sits on its line by that text, without the
/// line being any taller.
#[test]
fn text_in_an_inline_flex_box_is_as_tall_as_its_line() {
    let css = "body { margin: 0; font: 14px Arial; line-height: 64px; }\np { margin: 0; }\n\
        .b { display: inline-flex; align-items: center; height: 32px; padding: 4px 15px; border: 1px solid; }\n";
    let laid_out = laid_out("inline-flex-tall-line", css, r#"<dom.div><dom.span className="b">tall</dom.span></dom.div><dom.p>after</dom.p>"#);
    assert!(size(&laid_out, "tall").1 >= 63.0, "{laid_out}");
    assert!((rect(&laid_out, "after")[1] - 64.0).abs() < 1.0, "the line stays 64: {laid_out}");
}

/// `border-width` says each side, over what was said of one side before, as MUI's Divider's `border-width: 0` is over the
/// browser's `<hr>` borders, before its own `border-bottom-width`.
#[test]
fn border_width_says_each_side_over_what_came_before() {
    let css = "body { margin: 0; font: 16px Arial; }\np { margin: 0; }\n\
        .d { margin: 0; border-width: 0; border-style: solid; border-bottom-width: 1px; }\n";
    let laid_out = laid_out("border-width-sides", css, r#"<dom.hr className="d" /><dom.p>after</dom.p>"#);
    assert!((rect(&laid_out, "after")[1] - 1.0).abs() < 0.01, "a line 1 tall: {laid_out}");
}

/// A part of a paragraph has its own `letter-spacing`, as MUI's Typography in a label does.
#[test]
fn a_part_of_a_paragraph_has_its_own_letter_spacing() {
    let css = "body { margin: 0; font: 16px Arial; }\np { margin: 0; }\n.w { letter-spacing: 2px; }\n";
    let laid_out = laid_out("part-letter-spacing", css, r#"<dom.p>x <dom.span>same</dom.span> y</dom.p><dom.p>x <dom.span className="w">same</dom.span> y</dom.p>"#);
    let widths: Vec<f32> = laid_out
        .lines()
        .filter(|line| line.contains("\"same\""))
        .map(|line| line.split_whitespace().nth(1).unwrap().split(',').nth(2).unwrap().parse().unwrap())
        .collect();
    assert_eq!(widths.len(), 2, "{laid_out}");
    assert!((widths[1] - widths[0] - 8.0).abs() < 0.5, "2 more after each of its 4 letters: {laid_out}");
}

/// `word-spacing` is that much more after each space, and what `getComputedStyle` says, as emotion's `Global` sets it.
#[test]
fn words_can_be_further_apart() {
    let css = "body { margin: 0; font: 16px Arial; }\n.w { word-spacing: 5px; }\n";
    let laid_out = laid_out(
        "word-spacing",
        css,
        r#"<dom.div><dom.span>a b c</dom.span></dom.div><dom.div><dom.span className="w">a b c</dom.span></dom.div>"#,
    );
    let widths: Vec<f32> = laid_out
        .lines()
        .filter(|line| line.contains("\"a b c\""))
        .map(|line| line.split_whitespace().nth(1).unwrap().split(',').nth(2).unwrap().parse().unwrap())
        .collect();
    assert_eq!(widths.len(), 2, "{laid_out}");
    assert!((widths[1] - widths[0] - 10.0).abs() < 0.5, "5 more after each of its 2 spaces: {laid_out}");
}

/// A page's text is as wide as it is, to a 64th of a pixel, whether or not its words can be broken, and letters closer together
/// than they'd be don't make it wrap in the room it was measured at, as Radix's headings would.
#[test]
fn page_text_is_as_wide_as_it_is_whether_or_not_its_words_break() {
    let css = "body { margin: 0; font: 14px Arial; }\nbutton { padding: 0; border: 0; font: 14px Arial; }\n\
        .a { overflow-wrap: anywhere; }\n.h { display: inline-block; margin: 0; font: 20px Arial; letter-spacing: -0.5px; overflow-wrap: anywhere; }\n";
    let jsx = r#"<dom.div><dom.button>Docs</dom.button></dom.div><dom.div><dom.button className="a">Docs</dom.button></dom.div>
        <dom.div><dom.h1 className="h">Two words</dom.h1></dom.div>"#;
    let laid_out = laid_out("page-text-width", css, jsx);
    let widths: Vec<f32> = laid_out
        .lines()
        .filter(|line| line.contains("\"Docs\""))
        .map(|line| line.split_whitespace().nth(1).unwrap().split(',').nth(2).unwrap().parse().unwrap())
        .collect();
    assert_eq!(widths.len(), 2, "{laid_out}");
    assert!((widths[0] - widths[1]).abs() < 0.02, "the same, not a whole pixel more: {laid_out}");
    assert!((widths[0] - widths[0].round()).abs() > 0.01, "a 64th of a pixel at a time: {laid_out}");
    assert!(size(&laid_out, "Two words").1 < 30.0, "one line: {laid_out}");
}

/// What's beside a float, and goes along a line, is on a line as tall as its text's are, as Pico's nav link beside its item's
/// floated `::before` is.
#[test]
fn what_is_beside_a_float_is_on_a_line() {
    let css = "body { margin: 0; font: 18px Arial; line-height: 27px; }\nul { display: flex; margin: 0; padding: 0; list-style: none; }\n\
        li { padding: 18px 9px; }\nli::before { float: left; content: '\\200B'; }\n\
        a { display: inline-block; padding: 6px 9px; margin: -9px 0 0; border: 1px solid; }\np { margin: 0; }\n";
    let laid_out = laid_out("beside-float-line", css, r#"<dom.ul><dom.li><dom.a href="/x">link</dom.a></dom.li></dom.ul><dom.p>after</dom.p>"#);
    // The line's baseline is about 21 down, and the link's text's about 28 down from its border, less its margin of 9: the
    // line's own text is higher, so the line starts there, and is as tall as the link below that.
    let after = rect(&laid_out, "after")[1];
    assert!(after > 18.0 + 27.0 + 18.0 && after < 18.0 + 40.0 + 18.0, "{laid_out}");
    let link = laid_out.lines().find(|line| line.trim_start().starts_with("link")).unwrap_or_else(|| panic!("{laid_out}"));
    let top: f32 = link.split_whitespace().nth(1).unwrap().split(',').nth(1).unwrap().parse().unwrap();
    assert!(top > 18.0 - 9.0 + 0.5, "below the line's top less its margin, as the line's own text is higher: {laid_out}");
}

/// `font-variant-numeric: tabular-nums` has every digit as wide as each other, with the font's own `tnum` glyphs, in what it's
/// said on and in what's in that, as Chakra's badges do, and `font-feature-settings: "tnum"` says the same.
#[test]
fn tabular_nums_has_every_digit_as_wide() {
    // Segoe UI's own digits are only all as wide as each other in its regular weight.
    let css = "body { margin: 0; font: 500 16px 'Segoe UI'; }\np { margin: 0; }\n.t { font-variant-numeric: tabular-nums; }\n\
        .f { font-feature-settings: \"tnum\"; }\n";
    let jsx = r#"<dom.p><dom.span>1111</dom.span> <dom.span>0000</dom.span></dom.p>
        <dom.div className="t"><dom.p><dom.span>2111</dom.span> <dom.span>2000</dom.span></dom.p></dom.div>
        <dom.p className="f">3311</dom.p><dom.p className="f">3300</dom.p>"#;
    let laid_out = laid_out("tabular-nums", css, jsx);
    let width = |text: &str| size(&laid_out, text).0;
    assert!(width("0000") - width("1111") > 4.0, "a 1 is narrower than a 0 by itself: {laid_out}");
    let (two, eight) = (width("2111"), width("2000"));
    assert!((two - eight).abs() < 0.05, "every digit as wide, in what's in it: {two} and {eight}: {laid_out}");
    assert!((width("3311") - width("3300")).abs() < 0.05, "{laid_out}");
}

/// `min()`, `max()` and `clamp()` of a share of what something's in are worked out against how big that is, wherever a length goes,
/// as a browser has them: `min(320px, calc(100% - 48px))` is 320 pixels in something 400 wide, and 152 in something 200 wide.
#[test]
fn min_max_and_clamp_are_worked_out_against_what_they_are_in() {
    let css = "body { margin: 0; }\n.wide, .narrow { position: relative; }\n.wide { width: 400px; }\n.narrow { width: 200px; }\n\
        .row { display: flex; }\n.tall { height: 200px; }\n.short { height: 40px; }\n\
        .a { width: min(320px, calc(100% - 48px)); }\n.b { width: max(60px, 20%); }\n.c { width: clamp(50px, 50%, 150px); }\n\
        .d { max-width: min(50%, 150px); }\n.e { padding-left: max(10px, 5%); }\n\
        .f { width: 50px; margin-left: clamp(5px, 5%, 15px); }\n.g { flex: 0 0 min(50%, 150px); }\n.h { height: min(50%, 30px); }\n\
        .k { box-sizing: border-box; width: min(50%, 150px); padding: 0 10px; }\n\
        .l { position: absolute; top: 0; left: max(10px, 10%); width: 10px; }\n";
    let side = |side: &str, tall: &str| {
        format!(
            r#"<dom.div className="{side}">
                <dom.div className="a">a {side}</dom.div><dom.div className="b">b {side}</dom.div><dom.div className="c">c {side}</dom.div>
                <dom.div className="d">d {side}</dom.div><dom.div className="e"><dom.span>e {side}</dom.span></dom.div><dom.div className="f">f {side}</dom.div>
                <dom.div className="row"><dom.div className="g">g {side}</dom.div></dom.div>
                <dom.div className="{tall}"><dom.div className="h">h {side}</dom.div></dom.div>
                <dom.div className="k">k {side}</dom.div><dom.div className="l">l {side}</dom.div>
            </dom.div>"#
        )
    };
    let laid_out = laid_out("min-max-clamp", css, &(side("wide", "tall") + &side("narrow", "short")));
    for (side, [a, b, c, d, e, f, g, h, k, l]) in [
        ("wide", [320.0, 80.0, 150.0, 150.0, 20.0, 15.0, 150.0, 30.0, 150.0, 40.0]),
        ("narrow", [152.0, 60.0, 100.0, 100.0, 10.0, 10.0, 100.0, 20.0, 100.0, 20.0]),
    ] {
        let at = |name: &str| format!("{name} {side}");
        let width = |name: &str| size(&laid_out, &at(name)).0;
        // How far from the left of what it's in, where `a` starts.
        let from_left = |name: &str| rect(&laid_out, &at(name))[0] - rect(&laid_out, &at("a"))[0];
        assert_eq!(width("a"), a, "width: min() {side}: {laid_out}");
        assert_eq!(width("b"), b, "width: max() {side}: {laid_out}");
        assert_eq!(width("c"), c, "width: clamp() {side}: {laid_out}");
        assert_eq!(width("d"), d, "max-width {side}: {laid_out}");
        assert_eq!(from_left("e"), e, "padding {side}: {laid_out}");
        assert_eq!(from_left("f"), f, "margin {side}: {laid_out}");
        assert_eq!(width("g"), g, "flex-basis {side}: {laid_out}");
        assert_eq!(size(&laid_out, &at("h")).1, h, "height {side}: {laid_out}");
        assert_eq!(width("k"), k, "border-box {side}: {laid_out}");
        assert_eq!(from_left("l"), l, "left {side}: {laid_out}");
    }
}

/// `currentcolor` in a `color-mix()` is the color of the text, its own or what it inherits, as Tailwind 4's `::placeholder` has
/// it, and in `color` itself, the color it inherits.
#[test]
fn color_mix_with_currentcolor_is_mixed_with_the_color_of_the_text() {
    let css = ".own { color: #ee0000; background: color-mix(in oklab, currentcolor 50%, transparent); }\n.blue { color: #0000ff; }\n\
        .mixed { background: color-mix(in srgb, currentcolor 50%, #ffffff); }\n\
        .paler { color: color-mix(in srgb, currentcolor 50%, #ffffff); border: 1px solid color-mix(in srgb, currentcolor 50%, #000000); }\n\
        input::placeholder { color: color-mix(in oklab, currentcolor 50%, transparent); }\n";
    let jsx = r#"<dom.div className="own">own</dom.div>
        <dom.div className="blue"><dom.div className="mixed">inherited</dom.div><dom.div className="paler">paler</dom.div></dom.div>
        <dom.input placeholder="hint" />"#;
    let app = format!(
        "import {{ PageWindow, dom }} from 'rsui/web';\nimport './app.css';\nexport default function App() {{ return <PageWindow>{jsx}</PageWindow>; }}\n"
    );
    let mut hands = open("color-mix-current", &[("app.tsx", &app), ("app.css", css)]);
    hands.settle();
    let color = |text: &str, prop: &str| color_prop(hands.app.runtime().root(), text, prop);
    assert_eq!(color("own", "background").as_deref(), Some("#ee000080"));
    assert_eq!(color("inherited", "background").as_deref(), Some("#8080ffff"));
    assert_eq!(color("paler", "text_color").as_deref(), Some("#8080ffff"));
    // A border's `currentcolor` is the color of its own text, which is the paler one.
    assert_eq!(color("paler", "border_color").as_deref(), Some("#404080ff"));
    // A placeholder's is the color of the field's text, which is black, as it is around it.
    assert_eq!(color("hint", "placeholder_color").as_deref(), Some("#00000080"));
}

/// A color prop of what says this text, as `#rrggbbaa`.
fn color_prop(node: &rsui::core::Node, text: &str, prop: &str) -> Option<String> {
    if node.name.as_str() == text {
        return match node.props.get(prop) {
            Some(rsui::core::PropValue::Int(color)) => Some(format!("#{:08x}", *color as u32)),
            other => Some(format!("{other:?}")),
        };
    }
    node.children.iter().find_map(|child| color_prop(child, text, prop))
}

/// A color mixed with the color of the text is mixed again when that changes, there or in what it's in.
#[test]
fn color_mix_with_currentcolor_follows_the_color_of_the_text() {
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        import * as React from 'react';
        import './app.css';
        export default function App() {
            const [red, setRed] = React.useState(false);
            return (
                <PageWindow>
                    <dom.div className={red ? 'red' : 'blue'} onClick={() => setRed(true)}>
                        <dom.div className="mixed">swatch</dom.div>
                    </dom.div>
                </PageWindow>
            );
        }
    "#;
    let css = ".blue { color: #0000ff; }\n.red { color: #ff0000; }\n.mixed { background: color-mix(in srgb, currentcolor 50%, #ffffff); }\n";
    let mut hands = open("color-mix-follows", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    let laid_out = hands.app.rectangles();
    assert!(line(&laid_out, "swatch").contains("background=#8080ffff"), "{laid_out}");
    let [x, y, width, height] = rect(&laid_out, "swatch")[..] else { panic!("{laid_out}") };
    hands.click_at(x + width / 2.0, y + height / 2.0);
    hands.settle();
    let laid_out = hands.app.rectangles();
    assert!(line(&laid_out, "swatch").contains("background=#ff8080ff"), "{laid_out}");
}

/// A shorthand on an element's `style` says all of what it stands for, over what a class says of any of it, as on a page:
/// `background: #f00` takes away a class's gradient, which `background-color` doesn't, and `border` or `border-color` gives
/// every side its color, over a class's `border-left`. The same when React changes the `style` later.
#[test]
fn a_shorthand_in_a_style_goes_over_what_a_class_says_of_its_parts() {
    let app = r#"
        import { PageWindow, dom } from 'rsui/web';
        import * as React from 'react';
        import './app.css';
        export default function App() {
            const [later, setLater] = React.useState(false);
            const red = { background: '#f00' };
            return (
                <PageWindow>
                    <dom.div onClick={() => setLater(!later)}>
                        <dom.div className="g" style={red}>one</dom.div>
                        <dom.div className="g" style={{ backgroundColor: '#f00' }}>two</dom.div>
                        <dom.div className="i" style={red}>three</dom.div>
                        <dom.div className="s" style={{ border: '2px solid #f00' }}>four</dom.div>
                        <dom.div className="s" style={{ borderColor: '#f00' }}>five</dom.div>
                        <dom.div className="s">plain</dom.div>
                        <dom.div className="g" style={later ? red : undefined}>six</dom.div>
                        <dom.div className="g" style={later ? undefined : red}>seven</dom.div>
                    </dom.div>
                </PageWindow>
            );
        }
    "#;
    let css = ".g { background: linear-gradient(90deg, #00f, #0f0); }\n.i { background-image: linear-gradient(90deg, #00f, #0f0); }\n\
               .s { border-left: 6px solid #00f; border-top-color: #00f; }\n";
    let mut hands = open("shorthand-over-class", &[("app.tsx", app), ("app.css", css)]);
    hands.settle();
    let prop = |hands: &Hands, text: &str, prop: &str| color_prop(hands.app.runtime().root(), text, prop).unwrap_or_default();
    let gradient = |hands: &Hands, text: &str| prop(hands, text, "background_gradient") != "None";
    assert!(!gradient(&hands, "one") && prop(&hands, "one", "background") == "#ff0000ff", "background goes over a gradient");
    assert!(gradient(&hands, "two"), "background-color leaves the gradient");
    assert!(!gradient(&hands, "three"), "background goes over background-image");
    assert_eq!(prop(&hands, "plain", "border_left_color"), "#0000ffff", "the class's own");
    for text in ["four", "five"] {
        for side in ["border_top_color", "border_left_color"] {
            assert_ne!(prop(&hands, text, side), "#0000ffff", "every side of {text} is red");
        }
        assert_eq!(prop(&hands, text, "border_color"), "#ff0000ff", "{text}");
    }
    assert!(gradient(&hands, "six") && !gradient(&hands, "seven"));
    let laid_out = hands.app.rectangles();
    let [x, y, width, height] = rect(&laid_out, "one")[..] else { panic!("{laid_out}") };
    hands.click_at(x + width / 2.0, y + height / 2.0);
    hands.settle();
    assert!(!gradient(&hands, "six") && gradient(&hands, "seven"), "a style put on later, and taken off");
}

/// Padding that's a share of what something's in is a share of how wide that is, and outside a width and height said with
/// `box-sizing: content-box`, as `width: 50px; padding-left: 5%` is 70 wide in 400, in a block or along a flex box, where what's
/// too much is taken from each in proportion to how wide it is inside its padding.
#[test]
fn padding_that_is_a_share_is_outside_a_content_box_size() {
    let css = ".w, .f { width: 400px; }\n.f { display: flex; }\n.p { width: 50px; padding-left: 5%; }\n\
               .t { width: 50px; height: 20px; padding-top: 5%; padding-bottom: 2.5%; }\n\
               .bb { box-sizing: border-box; width: 100px; padding-left: 10%; }\n.mx { width: 50%; padding: 0 5%; }\n";
    let laid_out = laid_out(
        "share-padding",
        css,
        r#"<dom.div><dom.div className="w"><dom.div className="p">a</dom.div><dom.div className="t">b</dom.div><dom.div className="bb">c</dom.div><dom.div className="mx">d</dom.div></dom.div><dom.div className="f"><dom.div className="p">e</dom.div><dom.div className="bb">f</dom.div><dom.div className="mx">g</dom.div></dom.div></dom.div>"#,
    );
    assert_eq!(size(&laid_out, "a").0, 70.0, "{laid_out}");
    assert_eq!(size(&laid_out, "b"), (50.0, 50.0), "{laid_out}");
    assert_eq!(size(&laid_out, "c").0, 100.0, "{laid_out}");
    assert_eq!(size(&laid_out, "d").0, 240.0, "{laid_out}");
    // 410 in 400: 10 taken from 50, 60 and 200 inside their padding, as Edge has it.
    for (text, wide) in [("e", 70.0 - 50.0 / 31.0), ("f", 100.0 - 60.0 / 31.0), ("g", 240.0 - 200.0 / 31.0)] {
        assert!((size(&laid_out, text).0 - wide).abs() < 0.01, "{text}: {laid_out}");
    }
}
