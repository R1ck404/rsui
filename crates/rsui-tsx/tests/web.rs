//! Code written for a browser keeps its styling: what's in `rsui/web`.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn write(name: &str, source: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-web-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    file
}

fn open(name: &str, source: &str) -> Hands {
    Hands::open(&write(name, source), (500, 400))
}

/// What some CSS comes to, as its props, with each one that's expected.
fn css_gives(name: &str, css: &str, expected: &[(&str, &str)]) {
    let source = format!(
        r#"
        import {{ __compile }} from 'rsui/web';
        const own = __compile({css:?}).own;
        export default function App() {{
            return <window>{{Object.keys(own).map((name) => <text key={{name}}>{{name + '=' + own[name]}}</text>)}}</window>;
        }}
        "#
    );
    let hands = open(name, &source);
    for (prop, value) in expected {
        let wanted = format!("{prop}={value}");
        assert!(
            hands.says(&wanted),
            "{wanted} from {css}\n{}",
            hands.app.outline()
        );
    }
}

#[test]
fn flexbox_comes_to_what_the_same_layout_is_here() {
    css_gives(
        "flex",
        "display: flex; align-items: center; justify-content: space-between; gap: 8px; flex: 1;",
        &[
            ("direction", "row"),
            ("align", "center"),
            ("justify", "space_between"),
            ("gap", "8"),
            ("grow", "1"),
            ("basis", "0%"),
        ],
    );
    css_gives(
        "column",
        "display: flex; flex-direction: column; flex-shrink: 0;",
        &[("direction", "column"), ("shrink", "0")],
    );
}

#[test]
fn a_grid_with_no_tracks_is_only_a_list() {
    css_gives(
        "grid-column",
        "display: grid; justify-items: center; align-content: center; row-gap: 5px;",
        &[
            ("direction", "column"),
            ("align", "center"),
            ("justify", "center"),
            ("gap", "5"),
        ],
    );
    // Down each column first is a grid's own, as it takes care of percentages and spare room as CSS does.
    css_gives(
        "grid-row",
        "display: grid; grid-auto-flow: column; align-items: center; column-gap: 8px; align-content: start; grid-template-columns: 2rem 1fr;",
        &[
            ("display", "grid"),
            ("gridFlow", "column"),
            ("align", "center"),
            ("columnGap", "8"),
            ("alignContent", "start"),
            ("columns", "32px 1fr"),
        ],
    );
    css_gives(
        "grid-real",
        "display: grid; grid-template-columns: 1fr 1fr; align-items: center;",
        &[
            ("display", "grid"),
            ("columns", "1fr 1fr"),
            ("align", "center"),
        ],
    );
}

#[test]
fn overflow_says_which_ways_something_scrolls() {
    for (css, axis) in [
        ("overflow: auto;", "both"),
        ("overflow: hidden scroll;", "vertical"),
        ("overflow: scroll hidden;", "horizontal"),
        ("overflow: scroll;", "both"),
        ("overflow-x: auto;", "horizontal"),
        ("overflow-y: auto;", "vertical"),
        ("overflow-x: auto; overflow-y: auto;", "both"),
        ("overflow: hidden; overflow-y: auto;", "vertical"),
    ] {
        css_gives(
            &format!("overflow-{}", css.len()),
            css,
            &[("__scroll", axis)],
        );
    }
}

#[test]
fn a_brace_that_closes_nothing_is_got_past_as_a_browser_would() {
    css_gives(
        "brace",
        "color: #111; }} padding: 3px; & > span { color: #222; }",
        &[("color", "#111"), ("paddingTop", "3")],
    );
}

#[test]
fn margins_and_padding_go_on_each_side_as_css_counts_them() {
    css_gives(
        "sides",
        "padding: 1px 2px 3px 4px; margin: 5px 6px;",
        &[
            ("paddingTop", "1"),
            ("paddingRight", "2"),
            ("paddingBottom", "3"),
            ("paddingLeft", "4"),
            ("marginTop", "5"),
            ("marginRight", "6"),
            ("marginBottom", "5"),
            ("marginLeft", "6"),
        ],
    );
}

#[test]
fn colors_of_every_kind_come_out_as_rsui_takes_them() {
    css_gives(
        "colors",
        "color: rgba(255, 0, 0, 0.5); background-color: hsl(120, 100%, 50%); border-color: tomato;",
        &[
            ("color", "#ff000080"),
            ("background", "#00ff00"),
            ("borderColor", "#ff6347"),
        ],
    );
    css_gives(
        "newer",
        "color: rgb(0 128 255 / 50%); background: #abc;",
        &[("color", "#0080ff80"), ("background", "#abc")],
    );
}

#[test]
fn borders_and_shadows_are_taken_apart() {
    css_gives(
        "border",
        "border: 2px solid #123456; border-radius: 6px; box-shadow: 1px 2px 3px #00000080;",
        &[
            ("borderWidth", "2"),
            ("borderColor", "#123456"),
            ("radius", "6"),
            ("shadowX", "1"),
            ("shadowY", "2"),
            ("shadowBlur", "3"),
            ("shadowColor", "#00000080"),
            ("shadows", "1 2 3 0 128"),
        ],
    );
    css_gives("no-border", "border: none;", &[("borderWidth", "0")]);
    css_gives("round", "border-radius: 50%;", &[("radius", "9999")]);
}

#[test]
fn sizes_in_every_unit_are_worked_out() {
    css_gives(
        "units",
        "width: 50%; height: 2rem; min-width: 3em; font-size: 12pt; top: calc(10px + 2 * 5px); left: calc(100% - 1.4rem);",
        &[
            ("width", "50%"),
            ("height", "32"),
            ("minWidth", "48"),
            ("fontSize", "16"),
            ("top", "20"),
            ("left", "100% + -22.4px"),
        ],
    );
}

#[test]
fn hovering_gives_the_background_it_says() {
    css_gives(
        "hover",
        "background: #111; &:hover { background: #222; } &:active { background-color: #333; }",
        &[
            ("background", "#111"),
            ("hover", "#222"),
            ("pressed", "#333"),
        ],
    );
}

#[test]
fn text_and_positions_come_across() {
    css_gives(
        "text",
        "font-weight: 700; text-align: center; text-decoration: underline; white-space: nowrap; position: absolute; left: 4px;",
        &[
            ("fontWeight", "700"),
            ("textAlign", "center"),
            ("underline", "true"),
            ("noWrap", "true"),
            ("position", "absolute"),
            ("left", "4"),
        ],
    );
}

#[test]
fn a_styled_component_works_with_props_and_what_it_is_given() {
    let mut hands = open(
        "styled",
        r#"
        import { useState } from 'react';
        import { __styled as styled, __css as css } from 'rsui/web';
        const mixin = css`padding: 4px; border-radius: 3px;`;
        const Box = styled.div`
            ${mixin}
            display: ${(p) => (p.hidden ? 'none' : 'flex')};
            background: ${(p) => (p.$on ? '#0000ff' : '#ff0000')};
        `;
        const Label = styled.p`color: #333;`;
        const Bigger = styled(Label)`font-size: 20px;`;
        const Press = styled.button.attrs({ type: 'button' })`padding: 2px;`;
        export default function App() {
            const [on, setOn] = useState(false);
            return (
                <window>
                    <Box $on={on}><Label>{'label ' + on}</Label><Bigger>bigger</Bigger></Box>
                    <Box hidden><Label>hidden one</Label></Box>
                    <Press key="press" onClick={() => setOn(!on)}>press me</Press>
                </window>
            );
        }
        "#,
    );
    assert!(hands.says("label false"), "{}", hands.app.outline());
    assert!(hands.says("bigger"));
    assert!(!hands.says("hidden one"), "display: none isn't there");
    hands.click("press");
    hands.settle();
    assert!(hands.says("label true"), "{}", hands.app.outline());
}

#[test]
fn what_is_said_about_things_inside_reaches_them() {
    let hands = open(
        "nested",
        r#"
        import { __styled as styled } from 'rsui/web';
        const Inner = styled.div`padding: 1px;`;
        const Card = styled.section`
            header { h3 { font-size: 30px; } }
            & > p { color: #010203; }
            ${Inner} { display: none; }
            .gone { display: none; }
        `;
        function show(element) { return JSON.stringify(element.props); }
        export default function App() {
            return (
                <window>
                    <Card>
                        <group webTag="header"><text webTag="h3" key="title">title</text></group>
                        <text webTag="p" key="para">para</text>
                        <Inner><text>inside the inner one</text></Inner>
                        <text className="gone">gone by class</text>
                    </Card>
                </window>
            );
        }
        "#,
    );
    assert!(hands.says("title"), "{}", hands.app.outline());
    assert!(hands.says("para"));
    assert!(
        !hands.says("inside the inner one"),
        "a selector can be a component\n{}",
        hands.app.outline()
    );
    assert!(!hands.says("gone by class"), "{}", hands.app.outline());
}

#[test]
fn variables_and_rems_come_from_the_whole_page_and_can_change() {
    let mut hands = open(
        "vars",
        r#"
        import { useEffect, useState } from 'react';
        import { __styled as styled, __createGlobalStyle as createGlobalStyle, __compile, __remSize } from 'rsui/web';
        const Global = createGlobalStyle`
            html { font-size: 62.5%; }
            :root { --main: #111111; --rgb: 10, 20, 30; }
            .dark { --main: #eeeeee; }
        `;
        const Box = styled.div`background: var(--main); color: rgba(var(--rgb), 0.5); width: var(--none, 3rem);`;
        export default function App() {
            const [turn, setTurn] = useState(0);
            useEffect(() => setTurn(1), []);
            const look = __compile('background: var(--main); color: rgba(var(--rgb), 0.5); width: var(--none, 3rem);').own;
            return (
                <window>
                    <Global />
                    <Box />
                    <button key="dark" onClick={() => { document.body.classList.toggle('dark'); setTurn(turn + 1); }}>dark</button>
                    <text>{'main ' + look.background}</text>
                    <text>{'faded ' + look.color}</text>
                    <text>{'wide ' + look.width}</text>
                    <text>{'rem ' + __remSize()}</text>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    assert!(hands.says("rem 10"), "{}", hands.app.outline());
    assert!(hands.says("main #111111"), "{}", hands.app.outline());
    assert!(hands.says("faded #0a141e80"), "{}", hands.app.outline());
    assert!(hands.says("wide 30"), "{}", hands.app.outline());
    hands.click("dark");
    hands.settle();
    assert!(hands.says("main #eeeeee"), "{}", hands.app.outline());
}

#[test]
fn what_is_said_about_things_inside_reaches_inside_components() {
    let hands = open(
        "through",
        r#"
        import { __styled as styled, useShapeLook } from 'rsui/web';
        const Inner = styled.p`color: #000000;`;
        const Other = styled.h3`color: #000000;`;
        function Wrapped({ children }) { return <Inner>{children}</Inner>; }
        function Deep() { return <Wrapped>deep one</Wrapped>; }
        function Shape() { const look = useShapeLook(); return <text>{'shape width ' + look.width}</text>; }
        const Card = styled.section`
            p { display: none; }
            svg { width: 21px; }
            h3 > svg { width: 22px; }
        `;
        const Gone = styled.div``;
        function Hidden() { return <Gone><Deep /></Gone>; }
        export default function App() {
            return (
                <window>
                    <Card><Deep /></Card>
                    <Card><Hidden /><Shape /></Card>
                    <Wrapped>outside it</Wrapped>
                </window>
            );
        }
        "#,
    );
    assert!(
        !hands.says("deep one"),
        "reached through two components\n{}",
        hands.app.outline()
    );
    assert!(hands.says("shape width 21"), "{}", hands.app.outline());
    assert!(hands.says("outside it"), "{}", hands.app.outline());
}

#[test]
fn fonts_go_down_to_what_is_inside_as_css_has_it() {
    let mut hands = open(
        "fonts",
        r#"
        import { __styled as styled, __compile } from 'rsui/web';
        const Big = styled.div`font-size: 40px;`;
        const Mine = styled.p`font-weight: 700;`;
        function Inside() { return <Mine>through a component</Mine>; }
        const Shows = styled.p`color: #111;`;
        function Size({ children }) { return <Shows>{children}</Shows>; }
        export default function App() {
            return <window><Big><text key="plain">plain words</text>bare words<Inside /><Size>sized</Size></Big></window>;
        }
        "#,
    );
    let outline = hands.app.outline();
    assert!(hands.says("plain words"), "{outline}");
    assert!(hands.says("bare words"), "{outline}");
    assert!(hands.says("through a component"), "{outline}");
    assert!(hands.says("sized"), "{outline}");
    // Text that big takes up more room than text that isn't.
    let (_, y1) = hands.middle_of("plain");
    assert!(y1 > 15.0, "it's as tall as 40 pixels of text: {y1}");
}

#[test]
fn the_font_shorthand_sets_every_part_of_the_font() {
    css_gives(
        "font-all",
        "font: italic 600 14px/2 Georgia, serif;",
        &[
            ("fontStyle", "italic"),
            ("fontWeight", "600"),
            ("fontSize", "14"),
            ("lineHeight", "28"),
            ("fontFamily", "Georgia"),
        ],
    );
    // What isn't said is normal again, and the line height can stand apart from the size.
    css_gives(
        "font-reset",
        "font-weight: 700; font-style: italic; font: 12px / 1.5 Arial;",
        &[("fontStyle", "normal"), ("fontWeight", "normal"), ("fontSize", "12"), ("lineHeight", "18")],
    );
}

#[test]
fn font_inherit_gives_a_button_the_font_around_it() {
    // A button has a font of its own (13.33px Arial, as in a browser), unless `font: inherit` says to take the page's.
    let hands = open(
        "font-inherit",
        r#"
        import { PageWindow, addCss } from 'rsui/web';
        addCss(`body { font-size: 20px; font-family: Georgia; } .same { font: inherit; }`);
        export default function App() {
            return <PageWindow><button className="same">Same</button><button>Own</button></PageWindow>;
        }
        "#,
    );
    let shown = hands.app.rectangles();
    let line = |label: &str| {
        shown
            .lines()
            .find(|line| line.trim_start().starts_with("text") && line.contains(label))
            .unwrap_or_else(|| panic!("no {label}: {shown}"))
            .to_string()
    };
    assert!(line("\"Same\"").contains("font_size=20") && line("\"Same\"").contains("Georgia"), "{shown}");
    assert!(line("\"Own\"").contains("font_size=13.3"), "{shown}");
}

#[test]
fn before_and_after_are_things_inside_it() {
    let hands = open(
        "parts",
        r#"
        import { __styled as styled } from 'rsui/web';
        const Quote = styled.div`
            &::before { content: "first"; color: #111; }
            &::after { content: "last"; }
        `;
        const Line = styled.div`&::before { content: ""; height: 1px; background: #123; }`;
        const Nothing = styled.div`&::before { color: #111; } &::after { content: "not shown"; display: none; }`;
        export default function App() {
            return <window><Quote><text>middle</text></Quote><Line><text>line</text></Line><Nothing><text>plain</text></Nothing></window>;
        }
        "#,
    );
    let outline = hands.app.outline();
    let (first, middle, last) = (
        outline.find("first").expect("before is there"),
        outline.find("middle").expect("what's inside is there"),
        outline.find("last").expect("after is there"),
    );
    assert!(first < middle && middle < last, "{outline}");
    assert!(hands.says("plain"));
    assert!(!hands.says("not shown"), "{outline}");
}

#[test]
fn a_part_of_a_shape_can_be_picked_by_its_id() {
    let hands = open(
        "shape-parts",
        r#"
        import { __styled as styled, useShapeParts } from 'rsui/web';
        function Ring() { const part = useShapeParts(); return <text>{'ring ' + part('ring').stroke + ' other ' + part('other').stroke}</text>; }
        const Holder = styled.div`#ring { stroke: #010203; }`;
        export default function App() {
            return <window><Holder><Ring /></Holder></window>;
        }
        "#,
    );
    assert!(
        hands.says("ring #010203 other undefined"),
        "{}",
        hands.app.outline()
    );
}

#[test]
fn something_with_a_layout_of_its_own_is_not_one_piece_of_text() {
    let hands = open(
        "arranged",
        r#"
        import { __styled as styled } from 'rsui/web';
        const Grid = styled.h3`display: grid; grid-template-columns: 1fr 1fr;`;
        const Part = styled.span``;
        export default function App() {
            return <window><Grid><Part>left</Part><Part>right</Part></Grid></window>;
        }
        "#,
    );
    let outline = hands.app.outline();
    assert!(outline.contains("group"), "{outline}");
    assert!(hands.says("left") && hands.says("right"), "{outline}");
}

#[test]
fn what_is_said_about_the_page_can_use_variables_that_come_later_and_change() {
    let mut hands = open(
        "later",
        r#"
        import { useState } from 'react';
        import { PageWindow, addCss, __cls as cls, __createGlobalStyle as createGlobalStyle } from 'rsui/web';
        addCss(`body { background: var(--back); } .card { color: var(--front); } #app { padding: 3px; }`);
        const Global = createGlobalStyle`
            :root { ${(p) => (p.dark ? '--front: #eeeeee; --back: #000000;' : '--front: #111111; --back: #ffffff;')} }
        `;
        export default function App() {
            const [dark, setDark] = useState(false);
            const [, again] = useState(0);
            // What this says has been worked out before what's in it has been drawn, so another go shows what it came to.
            return (
                <PageWindow>
                    <Global dark={dark} />
                    <button key="dark" onClick={() => setDark(!dark)}>turn</button>
                    <button key="again" onClick={() => again((n) => n + 1)}>again</button>
                    <text>{'card ' + cls('card').color}</text>
                </PageWindow>
            );
        }
        "#,
    );
    hands.click("again");
    hands.settle();
    assert!(hands.says("card #111111"), "{}", hands.app.outline());
    hands.click("dark");
    hands.settle();
    hands.click("again");
    hands.settle();
    assert!(hands.says("card #eeeeee"), "{}", hands.app.outline());
    hands.click("dark");
    hands.settle();
    hands.click("again");
    hands.settle();
    assert!(
        hands.says("card #111111"),
        "back again\n{}",
        hands.app.outline()
    );
}

#[test]
fn text_can_be_put_in_upper_case() {
    let hands = open(
        "case",
        r#"
        import { __styled as styled } from 'rsui/web';
        const Loud = styled.p`text-transform: uppercase;`;
        const Inside = styled.div`span { text-transform: capitalize; }`;
        export default function App() {
            return <window><Loud>quiet words</Loud><Inside><text webTag="span">some more words</text></Inside></window>;
        }
        "#,
    );
    assert!(hands.says("QUIET WORDS"), "{}", hands.app.outline());
    assert!(hands.says("Some More Words"), "{}", hands.app.outline());
}

#[test]
fn what_a_class_says_goes_on_with_cls() {
    let hands = open(
        "classes",
        r#"
        import { addCss, __cls as cls, __compile } from 'rsui/web';
        addCss(`.card { padding: 7px; color: #101010; } .gone { display: none; }`);
        const look = cls('card other');
        export default function App() {
            return <window>{Object.keys(look).map((name) => <text key={name}>{name + '=' + look[name]}</text>)}</window>;
        }
        "#,
    );
    assert!(hands.says("paddingTop=7"), "{}", hands.app.outline());
    assert!(hands.says("color=#101010"));
    assert!(hands.says("className=card other"));
}

#[test]
fn a_style_object_is_read_as_react_writes_it() {
    let hands = open(
        "style",
        r#"
        import { __webStyle as webStyle } from 'rsui/web';
        const look = webStyle({ marginTop: 4, opacity: 0.5, display: 'flex', backgroundColor: 'rgb(1, 2, 3)', flexGrow: 2, skipped: null });
        export default function App() {
            return <window>{Object.keys(look).map((name) => <text key={name}>{name + '=' + look[name]}</text>)}</window>;
        }
        "#,
    );
    assert!(hands.says("marginTop=4"), "{}", hands.app.outline());
    assert!(hands.says("opacity=0.5"));
    assert!(hands.says("direction=row"));
    assert!(hands.says("background=#010203"));
    assert!(hands.says("grow=2"));
}

#[test]
fn handlers_written_for_the_web_get_what_they_expect() {
    let mut hands = open(
        "events",
        r#"
        import { useState } from 'react';
        import { __styled as styled } from 'rsui/web';
        const Field = styled.input`padding: 2px;`;
        const Tick = styled.input`margin: 2px;`;
        const Pick = styled.select``;
        export default function App() {
            const [text, setText] = useState('');
            const [on, setOn] = useState(false);
            return (
                <window>
                    <Field key="field" value={text} onChange={(e) => setText(e.target.value)} />
                    <Tick key="tick" type="checkbox" checked={on} onChange={(e) => setOn(e.target.checked)} />
                    <text>{'typed ' + text + ' ' + on}</text>
                </window>
            );
        }
        "#,
    );
    hands.click("field");
    hands.type_text("hi");
    hands.settle();
    assert!(hands.says("typed hi false"), "{}", hands.app.outline());
    hands.click("tick");
    hands.settle();
    assert!(hands.says("typed hi true"), "{}", hands.app.outline());
}

#[test]
fn the_window_looks_as_the_page_says() {
    let hands = open(
        "page",
        r#"
        import { PageWindow, addCss } from 'rsui/web';
        addCss(`body { background: #123123; color: #fefefe; font-size: 20px; } .x { margin: 1px }`);
        export default function App() {
            return <PageWindow><text>in the page</text></PageWindow>;
        }
        "#,
    );
    assert!(hands.says("in the page"), "{}", hands.app.outline());
}

#[test]
fn a_gradient_on_the_page_is_the_windows() {
    let hands = open(
        "page-gradient",
        r#"
        import { PageWindow, addCss } from 'rsui/web';
        addCss(`body { background: linear-gradient(90deg, #ff0000, #0000ff); color: white; }`);
        export default function App() {
            return <PageWindow><text>on a gradient</text></PageWindow>;
        }
        "#,
    );
    let outline = hands.app.outline();
    let window = outline.lines().next().unwrap_or_default();
    assert!(window.contains("background_gradient=\"90|#ff0000ff|#0000ffff\""), "{outline}");
}

#[test]
fn code_that_touches_the_page_keeps_going() {
    let hands = open(
        "page-touch",
        r#"
        import 'rsui/web';
        document.title = 'A title';
        document.addEventListener('contextmenu', () => {});
        window.addEventListener('auxclick', () => {});
        const none = document.getElementById('app');
        const media = window.matchMedia('(prefers-color-scheme: dark)');
        const made = document.createElement('div');
        made.classList.add('a');
        export default function App() {
            return <window><text>{'still here ' + (none === null) + ' ' + typeof media.matches + ' ' + document.title + ' ' + made.classList.contains('a')}</text></window>;
        }
        "#,
    );
    assert!(
        hands.says("still here true boolean A title true"),
        "{}",
        hands.app.outline()
    );
}

#[test]
fn what_cannot_be_done_is_said_once_and_left_out() {
    let hands = open(
        "unsupported",
        r#"
        import { __compile, __unsupported } from 'rsui/web';
        __compile(`shape-outside: circle(); shape-outside: none; mask: none; clip-path: none; color: #123; box-sizing: border-box;`);
        const said = __unsupported();
        export default function App() {
            return <window><text>{said.join(' | ')}</text></window>;
        }
        "#,
    );
    assert!(
        hands.says("shape-outside | mask"),
        "{}",
        hands.app.outline()
    );
}

/// A page made of what React writes for a browser, with this CSS, as what's laid out in it, one thing to a line.
fn page_rectangles(name: &str, css: &str, body: &str) -> String {
    let source = format!(
        r#"
        import {{ PageWindow, addCss, dom, TextArea }} from 'rsui/web';
        addCss({css:?});
        export default function App() {{
            return <PageWindow>{body}</PageWindow>;
        }}
        "#
    );
    open(name, &source).app.rectangles()
}

#[test]
fn spaces_run_together_on_a_page_unless_css_keeps_them() {
    let laid_out = page_rectangles(
        "spaces",
        "",
        r#"<dom.div>{'one    two\n  three'}</dom.div><dom.div style={{ whiteSpace: 'pre' }}>{'kept  as\nit is'}</dom.div>"#,
    );
    assert!(laid_out.contains("\"one two three\""), "{laid_out}");
    assert!(laid_out.contains("\"kept  as\\nit is\""), "{laid_out}");
}

#[test]
fn text_kept_as_it_is_starts_a_new_line_at_each_line_break() {
    let laid_out = page_rectangles("pre-lines", "", r#"<dom.pre>{'one\ntwo\nthree'}</dom.pre>"#);
    let line = laid_out
        .lines()
        .find(|line| line.contains("\"one\\ntwo\\nthree\""))
        .expect("the text is there");
    let height: f32 = line
        .split(',')
        .nth(3)
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    // Three lines of 13 pixel monospace, as a browser has `pre`.
    assert!(height > 40.0 && height < 52.0, "{line}");
}

#[test]
fn a_font_that_is_only_monospace_is_smaller_unless_a_size_is_said() {
    let laid_out = page_rectangles(
        "monospace",
        "",
        r#"<dom.div><dom.code>small</dom.code></dom.div><dom.div style={{ fontSize: 16 }}><dom.code>said</dom.code></dom.div>"#,
    );
    let size = |text: &str| {
        laid_out
            .lines()
            .find(|line| line.contains(&format!("\"{text}\"")))
            .and_then(|line| line.split("font_size=").nth(1))
            .map(|rest| rest.split(' ').next().unwrap().to_owned())
            .unwrap_or_default()
    };
    assert_eq!(size("small"), "13", "{laid_out}");
    assert_eq!(size("said"), "16", "{laid_out}");
}

#[test]
fn css_about_what_comes_before_something_counts() {
    let laid_out = page_rectangles(
        "siblings",
        ".row + .row { font-size: 30px; }",
        r#"<dom.div><dom.div className="row">first</dom.div><dom.div className="row">second</dom.div></dom.div>"#,
    );
    let first = laid_out
        .lines()
        .find(|line| line.contains("\"first\""))
        .unwrap();
    let second = laid_out
        .lines()
        .find(|line| line.contains("\"second\""))
        .unwrap();
    assert!(!first.contains("font_size=30"), "{laid_out}");
    assert!(second.contains("font_size=30"), "{laid_out}");
}

#[test]
fn a_list_in_a_list_has_hollow_bullets_and_no_room_around_it() {
    let laid_out = page_rectangles(
        "lists",
        "",
        r#"<dom.ul><dom.li>outer<dom.ul><dom.li>inner</dom.li></dom.ul></dom.li></dom.ul>"#,
    );
    // The markers are drawn as a browser draws them: a filled disc outside, and a circle that isn't filled inside.
    let markers: Vec<&str> = laid_out
        .lines()
        .filter(|line| line.trim_start().starts_with("group") && line.contains(",5.33"))
        .collect();
    assert_eq!(markers.len(), 2, "{laid_out}");
    assert!(markers[0].contains("background=#000000ff"), "{laid_out}");
    assert!(!markers[1].contains("background="), "{laid_out}");
    let y = |text: &str| -> f32 {
        let line = laid_out
            .lines()
            .find(|line| line.contains(&format!("\"{text}\"")))
            .unwrap();
        line.split(',').nth(1).unwrap().parse().unwrap()
    };
    // The inner list starts right under the outer item's line, with no margin between.
    assert!(y("inner") - y("outer") < 22.0, "{laid_out}");
}

#[test]
fn a_label_around_a_checkbox_reads_across_with_its_text_on_the_same_baseline() {
    let laid_out = page_rectangles(
        "label",
        "",
        r#"<dom.label><dom.input type="checkbox" checked={false} onChange={() => {}} />Accept terms</dom.label>"#,
    );
    let checkbox = laid_out
        .lines()
        .find(|line| line.trim_start().starts_with("checkbox"))
        .expect("there's a checkbox");
    let text = laid_out
        .lines()
        .find(|line| line.contains("\"Accept terms\""))
        .unwrap();
    let numbers = |line: &str| -> Vec<f32> {
        line.trim_start()
            .split(' ')
            .nth(1)
            .unwrap()
            .split(',')
            .map(|part| part.parse().unwrap())
            .collect()
    };
    let (check, words) = (numbers(checkbox), numbers(text));
    assert!(
        words[0] > check[0] + check[2],
        "the text is after the box: {laid_out}"
    );
    assert!(
        (words[1] - check[1]).abs() < 6.0,
        "on the same line: {laid_out}"
    );
    assert_eq!((check[2], check[3]), (13.0, 13.0), "{laid_out}");
}

#[test]
fn an_outline_is_only_there_with_a_style() {
    css_gives(
        "outline",
        "outline: 2px solid #ff0000; outline-offset: 3px;",
        &[
            ("outlineWidth", "2"),
            ("outlineColor", "#ff0000"),
            ("outlineOffset", "3"),
        ],
    );
    css_gives("outline-none", "outline: none;", &[("outlineWidth", "0")]);
    css_gives(
        "outline-no-style",
        "outline: 2px #ff0000;",
        &[("outlineWidth", "0")],
    );
}

#[test]
fn a_border_can_be_dashes_or_dots() {
    css_gives(
        "dashed",
        "border: 3px dashed #cc0000;",
        &[("borderWidth", "3"), ("borderStyle", "dashed")],
    );
    css_gives(
        "dotted",
        "border-style: dotted;",
        &[("borderStyle", "dotted")],
    );
    css_gives(
        "solid",
        "border: 1px solid black;",
        &[("borderStyle", "solid")],
    );
}

#[test]
fn a_textarea_is_as_wide_as_a_browser_makes_it_and_sits_on_its_line() {
    let laid_out = page_rectangles(
        "textarea",
        "",
        r#"<dom.div style={{ fontFamily: 'Arial' }}><TextArea rows={3} value="" onChange={() => {}} /><dom.div>after</dom.div></dom.div>"#,
    );
    let field = laid_out
        .lines()
        .find(|line| line.trim_start().starts_with("text_input"))
        .expect("there's a field");
    let numbers: Vec<f32> = field
        .trim_start()
        .split(' ')
        .nth(1)
        .unwrap()
        .split(',')
        .map(|part| part.parse().unwrap())
        .collect();
    // 20 characters of 13 pixel monospace, to a whole pixel, and Edge's 15 pixel scrollbar, inside 2 pixels of padding and a
    // border of 1 on each side: 168, as Edge has it.
    assert!((numbers[2] - 168.0).abs() < 0.5, "{laid_out}");
    let after = laid_out
        .lines()
        .find(|line| line.contains("\"after\""))
        .unwrap();
    let after_y: f32 = after
        .trim_start()
        .split(' ')
        .nth(1)
        .unwrap()
        .split(',')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    // What's under it starts below the line it's on, which goes a little under its bottom, as text would.
    assert!(after_y > numbers[1] + numbers[3] + 2.0, "{laid_out}");
}

#[test]
fn a_range_is_drawn_as_a_browser_draws_one() {
    let laid_out = page_rectangles(
        "range",
        "",
        r#"<dom.input type="range" min="0" max="100" value="50" onChange={() => {}} />"#,
    );
    let sizes: Vec<Vec<f32>> = laid_out
        .lines()
        .filter(|line| line.trim_start().starts_with("group"))
        .map(|line| {
            line.trim_start()
                .split(' ')
                .nth(1)
                .unwrap()
                .split(',')
                .map(|part| part.parse().unwrap())
                .collect()
        })
        .collect();
    // 129 by 16, with a track 8 high in it and a thumb 16 across in the middle.
    assert!(
        sizes.iter().any(|rect| rect[2] == 129.0 && rect[3] == 16.0),
        "{laid_out}"
    );
    assert!(
        sizes.iter().any(|rect| rect[2] == 127.0 && rect[3] == 8.0),
        "{laid_out}"
    );
    let thumb = sizes
        .iter()
        .find(|rect| rect[2] == 16.0 && rect[3] == 16.0)
        .expect("there's a thumb");
    let track = sizes.iter().find(|rect| rect[2] == 129.0).unwrap();
    assert!(
        (thumb[0] + 8.0 - (track[0] + 64.5)).abs() < 0.5,
        "{laid_out}"
    );
}

#[test]
fn a_width_from_calc_is_a_share_and_some_pixels() {
    let laid_out = page_rectangles(
        "calc",
        "",
        r#"<dom.div style={{ width: 'calc(100% - 40px)', height: 10, background: '#ff0000' }} />"#,
    );
    // The page is 500 wide, less 8 on each side, and this is 40 less than that.
    assert!(laid_out.contains(",444,10"), "{laid_out}");
}

#[test]
fn a_tables_columns_share_what_is_left_by_what_is_in_them() {
    let laid_out = page_rectangles(
        "table-share",
        "",
        r#"<dom.table style={{ width: '100%', borderCollapse: 'collapse' }}><dom.tbody><dom.tr><dom.td>a</dom.td><dom.td>a much longer cell</dom.td></dom.tr></dom.tbody></dom.table>"#,
    );
    let widths: Vec<f32> = laid_out
        .lines()
        .filter(|line| line.contains("\"a\"") || line.contains("\"a much longer cell\""))
        .map(|line| {
            line.trim_start()
                .split(' ')
                .nth(1)
                .unwrap()
                .split(',')
                .nth(2)
                .unwrap()
                .parse()
                .unwrap()
        })
        .collect();
    assert_eq!(widths.len(), 2, "{laid_out}");
    assert!(widths[1] > widths[0] * 4.0, "{laid_out}");
}

#[test]
fn css_about_a_row_of_a_table_is_behind_its_cells() {
    let laid_out = page_rectangles(
        "table-rows",
        ".t tbody tr:nth-child(even) { background: #00ff00; }",
        r#"<dom.table className="t"><dom.tbody><dom.tr><dom.td>one</dom.td></dom.tr><dom.tr><dom.td>two</dom.td></dom.tr></dom.tbody></dom.table>"#,
    );
    assert_eq!(
        laid_out.matches("background=#00ff00ff").count(),
        1,
        "{laid_out}"
    );
}

#[test]
fn letters_can_be_further_apart() {
    let laid_out = page_rectangles(
        "letters",
        "",
        r#"<dom.span>abcd</dom.span><dom.div /><dom.span style={{ letterSpacing: 2 }}>abcd</dom.span>"#,
    );
    let widths: Vec<f32> = laid_out
        .lines()
        .filter(|line| line.contains("\"abcd\""))
        .map(|line| {
            line.trim_start()
                .split(' ')
                .nth(1)
                .unwrap()
                .split(',')
                .nth(2)
                .unwrap()
                .parse()
                .unwrap()
        })
        .collect();
    assert_eq!(widths.len(), 2, "{laid_out}");
    assert!((widths[1] - widths[0] - 8.0).abs() < 0.1, "{laid_out}");
}

#[test]
fn text_in_a_flex_box_has_no_spaces_at_its_ends() {
    let laid_out = page_rectangles(
        "flex-text",
        "",
        r#"<dom.div style={{ display: 'flex' }}><dom.span>a</dom.span>{' Gold '}</dom.div>"#,
    );
    assert!(laid_out.contains("\"Gold\""), "{laid_out}");
}

#[test]
fn a_field_focused_through_a_ref_gets_what_is_typed() {
    let mut hands = open(
        "ref-focus",
        r#"
        import { useRef, useState } from 'react';
        import { PageWindow, dom, Input } from 'rsui/web';
        export default function App() {
            const ref = useRef(null);
            const [text, setText] = useState('');
            return (
                <PageWindow>
                    <Input ref={ref} placeholder="name" value={text} onChange={(e) => setText(e.target.value)} />
                    <dom.button key="go" onClick={() => ref.current.focus()}>Focus it</dom.button>
                    <dom.div>{'typed ' + text}</dom.div>
                </PageWindow>
            );
        }
        "#,
    );
    hands.click("go");
    hands.settle();
    eprintln!("focused {:?}", hands.app.focused());
    hands.type_text("abc");
    hands.settle();
    assert!(hands.says("typed abc"), "{}", hands.app.outline());
}

#[test]
fn what_css_says_moves_is_given_to_rsui_to_move() {
    let hands = open(
        "motion",
        r#"
        import { dom, addCss, __keyframes as keyframes } from 'rsui/web';
        addCss(`
            .box { transition: opacity 200ms ease-out 50ms, background-color 1s; animation: grow 2s infinite alternate both; }
            @keyframes grow { from { opacity: 0; transform: scale(0.5) } 50% { background: #ff0000 } to { opacity: 1 } }
        `);
        const fade = keyframes`from { opacity: 0 } to { opacity: 1 }`;
        export default function App() {
            return (
                <dom.div>
                    <dom.div className="box">Hi</dom.div>
                    <dom.div style={{ animation: `${fade} 300ms linear` }}>There</dom.div>
                </dom.div>
            );
        }
        "#,
    );
    let outline = hands.app.outline();
    for wanted in [
        r#"transitions="opacity n 0.2 0.05 ease-out|opacity\nbackground c 1 0 ease|background-color""#,
        r#"animation="2 0 inf alternate both ease\n0 opacity n 0\n0 transform s scale(0.5)\n0.5 background c 4278190335\n1 opacity n 1""#,
        r#"animation="0.3 0 1 normal none linear\n0 opacity n 0\n1 opacity n 1""#,
    ] {
        assert!(outline.contains(wanted), "{wanted}\n{outline}");
    }
}

#[test]
fn a_flex_item_starts_from_what_its_basis_says() {
    // Only numbers start from a share of nothing, so each gets its share, unless how big what it's in depends on it.
    css_gives(
        "flex-share",
        "flex: 2",
        &[("grow", "2"), ("shrink", "1"), ("basis", "0%")],
    );
    // `auto` starts from how big it is, and what's left over is shared out.
    css_gives("flex-auto-basis", "flex: 1 1 auto", &[("grow", "1")]);
    css_gives(
        "flex-length",
        "flex: 3 1 120px",
        &[("grow", "3"), ("basis", "120")],
    );
    css_gives("flex-basis", "flex-basis: 50px", &[("basis", "50")]);
}

#[test]
fn a_checkbox_that_keeps_whether_its_checked_itself_is_checked_by_its_label() {
    let mut hands = open(
        "own-checkbox",
        r#"
        import { Input, dom } from 'rsui/web';
        export default function App() {
            return (
                <dom.label key="label">
                    <Input type="checkbox" defaultChecked={false} />
                    Remember me
                </dom.label>
            );
        }
        "#,
    );
    assert!(
        !hands.app.outline().contains("checked=true"),
        "{}",
        hands.app.outline()
    );
    hands.click("label");
    assert!(
        hands.app.outline().contains("checked=true"),
        "{}",
        hands.app.outline()
    );
    hands.click("label");
    assert!(
        !hands.app.outline().contains("checked=true"),
        "{}",
        hands.app.outline()
    );
}

#[test]
fn a_text_field_with_a_border_of_its_own_has_chromiums_padding_inside_it() {
    let hands = open(
        "field-border",
        r#"
        import { Input, dom, addCss } from 'rsui/web';
        addCss(`input.own { border: 2px solid #3b82f6; }`);
        export default function App() {
            return (
                <dom.div>
                    <Input key="plain" placeholder="plain" />
                    <Input key="own" className="own" placeholder="own" />
                </dom.div>
            );
        }
        "#,
    );
    let height = |key: &str| {
        let mut path = Vec::new();
        assert!(hands::path_of(hands.app.runtime().root(), key, &mut path));
        hands.app.layout().rect(&path).expect("laid out").height
    };
    // Chromium's: 15 of text, 1 of padding and 2 of border above and below, as its own field has 1 of border and 2 of padding.
    assert_eq!(height("plain"), 21.0);
    assert_eq!(height("own"), 21.0);
}

#[test]
fn something_that_scrolls_keeps_what_it_does_when_its_used() {
    let mut hands = open(
        "scroll-handlers",
        r#"
        import { useState } from 'react';
        import { __styled as styled, dom } from 'rsui/web';
        const Box = styled.div`
            height: 60px;
            overflow: auto;
            &:hover { color: #ff0000; }
        `;
        export default function App() {
            const [clicks, setClicks] = useState(0);
            return (
                <dom.div>
                    <Box key="box" onClick={() => setClicks(clicks + 1)}>
                        {Array.from({ length: 10 }, (_, i) => <dom.div key={i}>Row {i}</dom.div>)}
                    </Box>
                    <dom.p>{'Clicked ' + clicks}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    hands.click("box");
    assert!(hands.says("Clicked 1"), "{}", hands.app.outline());
}

#[test]
fn newer_colors_are_worked_out_as_a_browser_does() {
    css_gives(
        "newer-colors",
        "color: color-mix(in srgb, #ff0000 50%, #0000ff); background: oklch(62.8% 0.2577 29.23); \
         border-color: hwb(120 0% 50%); outline-color: color-mix(in oklab, white, black);",
        &[
            ("color", "#800080"),
            ("background", "#ff0000"),
            ("borderColor", "#008000"),
            ("outlineColor", "#636363"),
        ],
    );
    css_gives(
        "mix-shares",
        "color: color-mix(in srgb, #ffffff 25%, #000000); background: color-mix(in srgb, #ff0000 20%, #0000ff 20%);",
        &[("color", "#404040"), ("background", "#80008066")],
    );
}

#[test]
fn logical_properties_are_their_sides_in_text_that_goes_left_to_right() {
    css_gives(
        "logical",
        "margin-inline: 4px 8px; padding-block: 2px; inline-size: 120px; inset-inline-start: 7px; \
         padding-inline-end: 9px; max-block-size: 50px;",
        &[
            ("marginLeft", "4"),
            ("marginRight", "8"),
            ("paddingTop", "2"),
            ("paddingBottom", "2"),
            // Sizes are of what's inside the padding, as CSS has them unless it says otherwise.
            ("width", "129"),
            ("left", "7"),
            ("paddingRight", "9"),
            ("maxHeight", "54"),
        ],
    );
}

#[test]
fn layers_and_supports_keep_their_rules() {
    let hands = open(
        "layers",
        r#"
        import { addCss, dom } from 'rsui/web';
        addCss(`
            @layer base { .a { color: #112233; } }
            @layer base, theme;
            @supports (display: grid) { .b { color: #445566; } }
            @supports not (display: grid) { .b { color: #ff00ff; } }
        `);
        export default function App() {
            return <dom.div><dom.p className="a">a</dom.p><dom.p className="b">b</dom.p></dom.div>;
        }
        "#,
    );
    let outline = format!("{:?}", hands.app.runtime().root());
    assert!(outline.contains("287454207"), "@layer's rule is there: {outline}");
    assert!(outline.contains("1146447615"), "@supports's rule is there: {outline}");
    assert!(!outline.contains("4278255615"), "@supports not isn't: {outline}");
}

/// An app written as React is for a page, with no `rsui/web` in it: its tags are the page's, a click goes on to what's around
/// what was clicked, and a field keeps what's typed in it.
#[test]
fn react_written_for_a_page_works_as_it_does_there() {
    let mut hands = open(
        "react-page",
        r#"
        import React, { useRef, useState } from 'react';
        export default function App() {
            const [log, setLog] = useState([]);
            const add = (said) => setLog((was) => [...was, said]);
            const field = useRef(null);
            return (
                <div key="outer" style={{ padding: 20 }} onClick={(e) => add(e.target === e.currentTarget ? 'outer self' : 'outer inside')}>
                    <button key="bubbles" onClick={() => add('button')}>Bubbles</button>
                    <button key="stops" onClick={(e) => { e.stopPropagation(); add('stopped'); }}>Stops</button>
                    <div key="plain"><span>Plain</span></div>
                    <input ref={field} onKeyDown={(e) => add('key ' + e.key)} onChange={(e) => add('typed ' + e.target.value)} />
                    <button key="read" onClick={(e) => { e.stopPropagation(); add('read ' + field.current.value); }}>Read</button>
                    <p>{'Log: ' + log.join(', ')}</p>
                </div>
            );
        }
        "#,
    );
    hands.click("bubbles");
    hands.click("stops");
    hands.click("plain");
    let mut outer = Vec::new();
    assert!(hands::path_of(hands.app.runtime().root(), "outer", &mut outer));
    let rect = hands.app.layout().rect(&outer).expect("it's been laid out");
    hands.click_at(rect.x + 4.0, rect.y + 4.0);
    assert!(
        hands.says("Log: button, outer inside, stopped, outer inside, outer self"),
        "{}",
        hands.app.outline()
    );
    // A field with no value said keeps what's typed, however often what's around it renders, and its ref can read it.
    let input = hands.app.outline();
    assert!(input.contains("text_input"), "{input}");
    let (x, y) = {
        let root = hands.app.runtime().root();
        let mut path = Vec::new();
        fn find_input(node: &rsui::core::Node, path: &mut Vec<u32>) -> bool {
            if node.role == rsui::core::Role::TextInput {
                return true;
            }
            for (at, child) in node.children.iter().enumerate() {
                path.push(at as u32);
                if find_input(child, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
        assert!(find_input(root, &mut path));
        let rect = hands.app.layout().rect(&path).unwrap();
        (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
    };
    hands.click_at(x, y);
    hands.type_text("hi");
    assert!(hands.says("typed hi"), "{}", hands.app.outline());
    hands.press(hands::KeyCode::Down);
    assert!(hands.says("key ArrowDown"), "{}", hands.app.outline());
    hands.click("read");
    assert!(hands.says("read hi"), "{}", hands.app.outline());
}

#[test]
fn stylesheets_can_be_imported_and_css_modules_give_their_classes() {
    let file = write(
        "css-import",
        r#"
        import React from 'react';
        import './app.css';
        import styles from './card.module.css';
        export default function App() {
            return <div><p className="plain">plain</p><p className={styles.title}>{'class ' + styles.title}</p></div>;
        }
        "#,
    );
    let folder = file.parent().unwrap();
    std::fs::write(folder.join("app.css"), ".plain { color: #102030; }").unwrap();
    std::fs::write(folder.join("card.module.css"), ".title { color: #405060; }").unwrap();
    let hands = Hands::open(&file, (500, 400));
    assert!(hands.says("class title"), "{}", hands.app.outline());
    let tree = format!("{:?}", hands.app.runtime().root());
    assert!(tree.contains(&(0x102030_i64 * 256 + 255).to_string()), "app.css applies: {tree}");
    assert!(tree.contains(&(0x405060_i64 * 256 + 255).to_string()), "the module's class applies: {tree}");
}

// Events as a page has them.

use rsui::runtime::Modifiers;
use rsui::{KeyCode, KeyPress};

/// A slider of a page's own, made of boxes, that follows the pointer while it's held, even off the end of it.
#[test]
fn a_slider_made_of_boxes_follows_the_pointer_while_its_held() {
    let mut hands = open(
        "pointer-slider",
        r#"
        import React, { useRef, useState } from 'react';
        export default function App() {
            const [value, setValue] = useState(0);
            const [moves, setMoves] = useState(0);
            const held = useRef(false);
            const follow = (e) => {
                const box = e.currentTarget.getBoundingClientRect();
                setValue(Math.round(Math.max(0, Math.min(1, (e.clientX - box.left) / box.width)) * 100));
            };
            return (
                <div style={{ padding: 20 }}>
                    <div key="track" style={{ width: 200, height: 20, background: '#cccccc' }}
                        onPointerDown={(e) => { held.current = true; e.currentTarget.setPointerCapture(e.pointerId); follow(e); }}
                        onPointerMove={(e) => { setMoves((n) => n + 1); if (held.current) follow(e); }}
                        onPointerUp={(e) => { held.current = false; follow(e); }}>
                        <div key="thumb" style={{ width: 10, height: 20, background: '#0000ff' }} />
                    </div>
                    <p key="below" style={{ height: 40 }}>{'value ' + value}</p>
                    <p>{'moves ' + (moves > 0 ? 'heard' : 'none')}</p>
                </div>
            );
        }
        "#,
    );
    let [x, y, width, height] = hands.rect_of("track");
    let middle = y + height / 2.0;
    // On the thumb, which is inside what hears it, a quarter of the way along.
    hands.down_at(x + 5.0, middle);
    assert!(hands.says("value 3"), "{}", hands.app.outline());
    hands.move_at(x + width / 4.0, middle);
    assert!(hands.says("value 25"), "{}", hands.app.outline());
    // Off the end, and below it: it's still the track's, as it's held.
    let [_, below_y, _, _] = hands.rect_of("below");
    hands.move_at(x + width + 50.0, below_y + 10.0);
    assert!(hands.says("value 100"), "{}", hands.app.outline());
    hands.up_at(x + width / 2.0, below_y + 10.0);
    assert!(hands.says("value 50"), "{}", hands.app.outline());
    // Let go of, moving over it only says so.
    hands.move_at(x + 10.0, middle);
    assert!(hands.says("value 50"), "{}", hands.app.outline());
    assert!(hands.says("moves heard"), "{}", hands.app.outline());
}

#[test]
fn ctrl_and_shift_clicks_say_so_and_pick_several() {
    let mut hands = open(
        "multi-select",
        r#"
        import React, { useState } from 'react';
        const ITEMS = ['a', 'b', 'c', 'd'];
        export default function App() {
            const [picked, setPicked] = useState([]);
            const [last, setLast] = useState(null);
            const pick = (item, e) => {
                if (e.shiftKey && last !== null) {
                    const [from, to] = [ITEMS.indexOf(last), ITEMS.indexOf(item)].sort();
                    setPicked(ITEMS.slice(from, to + 1));
                } else if (e.ctrlKey || e.metaKey) {
                    setPicked((was) => (was.includes(item) ? was.filter((one) => one !== item) : [...was, item]));
                } else {
                    setPicked([item]);
                }
                setLast(item);
            };
            return (
                <ul>
                    {ITEMS.map((item) => <li key={item} onClick={(e) => pick(item, e)}>{'item ' + item}</li>)}
                    <p>{'picked ' + picked.join('')}</p>
                </ul>
            );
        }
        "#,
    );
    hands.click("a");
    assert!(hands.says("picked a"), "{}", hands.app.outline());
    let ctrl = Modifiers { ctrl: true, ..Modifiers::default() };
    hands.click_holding("c", ctrl);
    assert!(hands.says("picked ac"), "{}", hands.app.outline());
    hands.click_holding("a", ctrl);
    assert!(hands.says("picked c\""), "{}", hands.app.outline());
    // From the last one clicked, which was a.
    let shift = Modifiers { shift: true, ..Modifiers::default() };
    hands.click_holding("c", shift);
    assert!(hands.says("picked abc"), "{}", hands.app.outline());
}

#[test]
fn a_menu_closes_when_something_outside_it_is_pressed() {
    let mut hands = open(
        "click-outside",
        r#"
        import React, { useEffect, useRef, useState } from 'react';
        function Menu({ onClose }) {
            const box = useRef(null);
            useEffect(() => {
                const away = (e) => { if (box.current && !box.current.contains(e.target)) onClose(); };
                document.addEventListener('mousedown', away);
                return () => document.removeEventListener('mousedown', away);
            }, [onClose]);
            return <div ref={box} key="menu" style={{ padding: 8 }}><span key="inside">Inside the menu</span></div>;
        }
        export default function App() {
            const [open, setOpen] = useState(true);
            const [clicks, setClicks] = useState(0);
            useEffect(() => {
                const heard = () => setClicks((n) => n + 1);
                window.addEventListener('click', heard);
                return () => window.removeEventListener('click', heard);
            }, []);
            return (
                <div>
                    {open ? <Menu onClose={() => setOpen(false)} /> : <p>closed</p>}
                    <p key="away" style={{ height: 40 }}>Somewhere else</p>
                    <button key="open" onClick={() => setOpen(true)}>Open</button>
                    <p>{'window heard ' + clicks}</p>
                </div>
            );
        }
        "#,
    );
    hands.click("inside");
    assert!(hands.says("Inside the menu"), "a press inside leaves it open\n{}", hands.app.outline());
    hands.click("away");
    assert!(hands.says("closed"), "a press outside closes it\n{}", hands.app.outline());
    assert!(hands.says("window heard 2"), "window hears clicks on what doesn't take them\n{}", hands.app.outline());
    hands.click("open");
    assert!(hands.says("Inside the menu"), "{}", hands.app.outline());
    assert!(hands.says("window heard 3"), "and on what does\n{}", hands.app.outline());
}

#[test]
fn a_key_a_field_would_take_reaches_on_key_down_first() {
    let mut hands = open(
        "prevent-letter",
        r#"
        import React, { useState } from 'react';
        export default function App() {
            const [log, setLog] = useState([]);
            const [text, setText] = useState('');
            return (
                <div>
                    <input value={text} onChange={(e) => setText(e.target.value)}
                        onKeyDown={(e) => {
                            setLog((was) => [...was, e.key + '/' + e.code + (e.ctrlKey ? '/ctrl' : '')]);
                            if (e.key === 'x' || (e.ctrlKey && e.key === 'a')) e.preventDefault();
                        }} />
                    <p>{'text [' + text + ']'}</p>
                    <p>{'keys ' + log.join(' ')}</p>
                </div>
            );
        }
        "#,
    );
    let (x, y) = hands.middle_of_role(rsui::core::Role::TextInput);
    hands.click_at(x, y);
    for letter in ['a', 'x', 'b'] {
        hands.press_key(KeyPress::new(KeyCode::Character(letter)));
    }
    assert!(hands.says("text [ab]"), "x was stopped\n{}", hands.app.outline());
    // Select all is stopped, so typing goes on the end rather than over everything.
    hands.press_key(KeyPress::new(KeyCode::Character('a')).ctrl());
    hands.press_key(KeyPress::new(KeyCode::Character('c')).shift());
    assert!(hands.says("text [abC]"), "{}", hands.app.outline());
    assert!(
        hands.says("keys a/KeyA x/KeyX b/KeyB a/KeyA/ctrl C/KeyC"),
        "{}",
        hands.app.outline()
    );
}

#[test]
fn keys_say_what_the_web_calls_them_and_where_they_are() {
    let mut hands = open(
        "key-names",
        r#"
        import React, { useEffect, useState } from 'react';
        export default function App() {
            const [log, setLog] = useState([]);
            useEffect(() => {
                const down = (e) => setLog((was) => [...was, 'down ' + e.key + ' ' + e.code + ' ' + e.location + (e.repeat ? ' again' : '') + ' ' + e.keyCode]);
                const up = (e) => setLog((was) => [...was, 'up ' + e.key]);
                document.addEventListener('keydown', down);
                document.addEventListener('keyup', up);
                return () => {
                    document.removeEventListener('keydown', down);
                    document.removeEventListener('keyup', up);
                };
            }, []);
            return <div><p>{log.join(', ')}</p></div>;
        }
        "#,
    );
    hands.press_key(KeyPress::new(KeyCode::Function(5)));
    hands.press_key(KeyPress::new(KeyCode::Shift));
    hands.press_key(KeyPress::new(KeyCode::Character('q')).shift());
    let mut held = KeyPress::new(KeyCode::Left);
    held.repeat = true;
    hands.app.key(held).unwrap();
    hands.settle();
    for said in [
        "down F5 F5 0 116",
        "up F5",
        "down Shift ShiftLeft 1 16",
        "down Q KeyQ 0 81",
        "down ArrowLeft ArrowLeft 0 again 37",
    ] {
        assert!(hands.says(said), "{said}\n{}", hands.app.outline());
    }
}

#[test]
fn what_was_pressed_inside_is_the_element_it_was() {
    let mut hands = open(
        "inner-target",
        r#"
        import React, { useRef, useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('nothing');
            const special = useRef(null);
            return (
                <div>
                    <ul key="list" className="list" onClick={(e) => {
                        const item = e.target.closest('li');
                        setSaid([e.target.tagName, item?.dataset.id, e.target === special.current ? 'the ref' : 'not the ref', e.currentTarget.className, item?.matches('.row') ? 'row' : 'no'].join(' '));
                    }}>
                        <li key="one" className="row" data-id="1"><span key="label-one">One</span></li>
                        <li key="two" className="row" data-id="2" ref={special}>Two</li>
                    </ul>
                    <p>{'said ' + said}</p>
                </div>
            );
        }
        "#,
    );
    hands.click("label-one");
    assert!(hands.says("said SPAN 1 not the ref list row"), "{}", hands.app.outline());
    // It has a `ref`, so its key is the one that goes with that.
    let [x, y, width, height] = {
        let mut path = Vec::new();
        assert!(hands::path_of(hands.app.runtime().root(), "list", &mut path));
        path.push(1);
        let rect = hands.app.layout().rect(&path).expect("laid out");
        [rect.x, rect.y, rect.width, rect.height]
    };
    hands.click_at(x + width / 2.0, y + height / 2.0);
    assert!(hands.says("said LI 2 the ref list row"), "{}", hands.app.outline());
}

#[test]
fn tab_index_makes_things_focusable_and_orders_them() {
    let mut hands = open(
        "tab-index",
        r#"
        import React, { useState } from 'react';
        export default function App() {
            const [focused, setFocused] = useState('none');
            const box = (name, index) => (
                <div key={name} tabIndex={index} onFocus={() => setFocused(name)} onKeyDown={(e) => e.key === 'Enter' && setFocused(name + ' entered')}>
                    {name}
                </div>
            );
            return (
                <div>
                    {box('zero', 0)}
                    {box('never', -1)}
                    {box('second', 2)}
                    {box('first', 1)}
                    <p>{'focused ' + focused}</p>
                </div>
            );
        }
        "#,
    );
    // Past the last, the keyboard goes to the page, as a browser's does, and round from there.
    let order: Vec<String> = (0..5)
        .map(|_| {
            hands.press(KeyCode::Tab);
            let outline = hands.app.outline();
            outline
                .lines()
                .find_map(|line| {
                    let rest = line.split("\"focused ").nth(1)?;
                    Some(rest.split('"').next()?.to_owned())
                })
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(order, ["first", "second", "zero", "zero", "first"]);
    // One that tab doesn't reach can still be clicked to, and hears keys then.
    hands.click("never");
    assert!(hands.says("focused never"), "{}", hands.app.outline());
    hands.press(KeyCode::Enter);
    assert!(hands.says("focused never entered"), "{}", hands.app.outline());
}

#[test]
fn pasting_copying_and_cutting_can_be_heard_and_stopped() {
    let mut hands = open(
        "clipboard-events",
        r#"
        import React, { useState } from 'react';
        export default function App() {
            const [log, setLog] = useState([]);
            const [text, setText] = useState('start');
            return (
                <div>
                    <textarea value={text} onChange={(e) => setText(e.target.value)}
                        onPaste={(e) => { setLog((was) => [...was, 'pasted ' + e.clipboardData.getData('text')]); e.preventDefault(); }}
                        onCopy={(e) => { e.clipboardData.setData('text/plain', 'from the app'); e.preventDefault(); setLog((was) => [...was, 'copied']); }}
                        onCut={(e) => setLog((was) => [...was, 'cut'])} />
                    <p>{'text [' + text + ']'}</p>
                    <p>{'log ' + log.join(', ')}</p>
                </div>
            );
        }
        "#,
    );
    let (x, y) = hands.middle_of_role(rsui::core::Role::TextInput);
    hands.click_at(x, y);
    hands.press_with_ctrl('a');
    hands.press_with_ctrl('c');
    hands.press_with_ctrl('v');
    assert!(hands.says("text [start]"), "the paste was stopped\n{}", hands.app.outline());
    assert!(hands.says("log copied, pasted from the app"), "{}", hands.app.outline());
    hands.press_with_ctrl('a');
    hands.press_with_ctrl('x');
    assert!(hands.says("text []"), "a cut that isn't stopped cuts\n{}", hands.app.outline());
    assert!(hands.says("copied, pasted from the app, cut"), "{}", hands.app.outline());
}

#[test]
fn composing_with_a_way_of_typing_is_heard() {
    let mut hands = open(
        "composition",
        r#"
        import React, { useState } from 'react';
        export default function App() {
            const [log, setLog] = useState([]);
            const note = (e) => setLog((was) => [...was, e.type + ' ' + e.data]);
            return (
                <div>
                    <input onCompositionStart={note} onCompositionUpdate={note} onCompositionEnd={note}
                        onKeyDown={(e) => e.isComposing && setLog((was) => [...was, 'composing ' + e.key])} />
                    <p>{'log ' + log.join(', ')}</p>
                </div>
            );
        }
        "#,
    );
    let (x, y) = hands.middle_of_role(rsui::core::Role::TextInput);
    hands.click_at(x, y);
    hands.app.composition("start", "").unwrap();
    hands.settle();
    hands.app.key(KeyPress::new(KeyCode::Process)).unwrap();
    hands.settle();
    hands.app.composition("update", "ni").unwrap();
    hands.settle();
    hands.app.composition("end", "你").unwrap();
    hands.type_text("你");
    assert!(
        hands.says("log compositionstart , composing Process, compositionupdate ni, compositionend 你"),
        "{}",
        hands.app.outline()
    );
    assert!(hands.says("你"), "{}", hands.app.outline());
}

#[test]
fn the_wheel_and_keys_coming_up_are_heard_and_the_wheel_can_be_stopped() {
    let mut hands = open(
        "wheel-keyup",
        r#"
        import React, { useState } from 'react';
        export default function App() {
            const [log, setLog] = useState([]);
            return (
                <div>
                    <div key="zoom" style={{ height: 50, overflow: 'auto' }}
                        onWheel={(e) => { setLog((was) => [...was, 'wheel ' + e.deltaY]); e.preventDefault(); }}>
                        {Array.from({ length: 20 }, (_, i) => <p key={i}>{'row ' + i}</p>)}
                    </div>
                    <input onKeyUp={(e) => setLog((was) => [...was, 'up ' + e.key])} onInput={(e) => setLog((was) => [...was, 'input ' + e.target.value])} />
                    <p>{'log ' + log.join(', ')}</p>
                </div>
            );
        }
        "#,
    );
    let [x, y, width, height] = hands.rect_of("zoom");
    hands.move_at(x + width / 2.0, y + height / 2.0);
    hands.app.scroll(x + width / 2.0, y + height / 2.0, 3).unwrap();
    hands.settle();
    assert!(hands.says("wheel 100"), "{}", hands.app.outline());
    assert!(
        !hands.app.outline().contains("scroll_y=") || hands.app.outline().contains("scroll_y=0"),
        "stopping the wheel stops the scrolling\n{}",
        hands.app.outline()
    );
    let (x, y) = hands.middle_of_role(rsui::core::Role::TextInput);
    hands.click_at(x, y);
    hands.press_key(KeyPress::new(KeyCode::Character('k')));
    assert!(hands.says("input k, up k"), "{}", hands.app.outline());
}

#[test]
fn pictures_say_when_they_have_loaded_or_cant_be() {
    // A red picture two pixels wide and three high, where it's said to be, as this process can only have one folder that
    // pictures are from, and it's another test's.
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-web-{}-image-load-pictures", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let picture = folder.join("dot.png");
    std::fs::write(&picture, include_bytes!("fixtures/dot.png")).unwrap();
    let picture = picture.to_string_lossy().replace('\\', "/");
    let file = write(
        "image-load",
        &r#"
        import React, { useState } from 'react';
        export default function App() {
            const [log, setLog] = useState([]);
            return (
                <div>
                    <img src="PICTURE" onLoad={(e) => setLog((was) => [...was, 'loaded ' + e.target.naturalWidth + 'x' + e.target.naturalHeight])} />
                    <img src="PICTURE.missing" onError={() => setLog((was) => [...was, 'missing'])} />
                    <p>{'log ' + log.join(', ')}</p>
                </div>
            );
        }
        "#
        .replace("PICTURE", &picture),
    );
    let mut hands = Hands::open(&file, (500, 400));
    hands.wait_for("missing");
    assert!(hands.says("loaded 2x3"), "{}", hands.app.outline());
}

#[test]
fn transitions_and_animations_say_when_they_end_even_with_nothing_moving() {
    let mut hands = open(
        "motion-end",
        r#"
        import React, { useState } from 'react';
        import { addCss } from 'rsui/web';
        addCss(`
            .fade { opacity: 1; transition: opacity 0.2s; }
            .out { opacity: 0; transition: opacity 0.2s; }
            @keyframes pop { from { opacity: 0; } to { opacity: 1; } }
            .pop { animation: pop 0.1s; }
        `);
        export default function App() {
            const [out, setOut] = useState(false);
            const [log, setLog] = useState([]);
            return (
                <div>
                    <div key="fade" className={out ? 'out' : 'fade'} onClick={() => setOut(true)}
                        onTransitionEnd={(e) => setLog((was) => [...was, 'ended ' + e.propertyName])}>Fades</div>
                    <div className="pop" onAnimationEnd={(e) => setLog((was) => [...was, 'popped ' + e.animationName])}>Pops</div>
                    <p>{'log ' + log.join(', ')}</p>
                </div>
            );
        }
        "#,
    );
    hands.settle();
    assert!(hands.says("popped pop"), "{}", hands.app.outline());
    hands.click("fade");
    hands.settle();
    assert!(hands.says("ended opacity"), "{}", hands.app.outline());
}

#[test]
fn presses_go_down_through_what_listens_on_the_way_and_can_stop_there() {
    let mut hands = open(
        "capture",
        r#"
        import React, { useEffect, useState } from 'react';
        export default function App() {
            const [log, setLog] = useState([]);
            const [blocking, setBlocking] = useState(false);
            const note = (said) => setLog((was) => [...was, said]);
            useEffect(() => {
                const down = () => note('document down');
                document.addEventListener('click', down, true);
                return () => document.removeEventListener('click', down, true);
            }, []);
            return (
                <div key="outer" onClickCapture={(e) => { note('outer capture'); if (blocking) e.stopPropagation(); }} onClick={() => note('outer')}
                    onMouseDown={(e) => note('down ' + e.button + ' ' + e.target.tagName)} onAuxClick={(e) => note('aux ' + e.button)}>
                    <button key="inner" onClick={() => note('inner')}>Inner</button>
                    <button key="block" onClick={(e) => { e.stopPropagation(); setBlocking(true); }}>Block</button>
                    <p>{'log ' + log.join(', ')}</p>
                </div>
            );
        }
        "#,
    );
    hands.click("inner");
    assert!(
        hands.says("log down 0 BUTTON, document down, outer capture, inner, outer"),
        "{}",
        hands.app.outline()
    );
    hands.click("block");
    hands.click("inner");
    assert!(
        hands.says("outer capture, down 0 BUTTON, document down, outer capture\""),
        "a capturing handler that stops it stops the click\n{}",
        hands.app.outline()
    );
    // The middle button presses and clicks, as `auxclick`, without being a click.
    let (x, y) = hands.middle_of("inner");
    hands.app.button_down(x, y, rsui::PointerButton::Middle);
    hands.app.button_up(x, y, rsui::PointerButton::Middle).unwrap();
    hands.settle();
    assert!(hands.says("down 1 BUTTON, aux 1"), "{}", hands.app.outline());
}

#[test]
fn the_pointer_coming_and_going_says_where_it_is_and_hover_styles_still_follow_it() {
    let mut hands = open(
        "enter-leave",
        r#"
        import React, { useState } from 'react';
        import { addCss } from 'rsui/web';
        addCss(`.tip:hover { color: #ff0000; }`);
        export default function App() {
            const [said, setSaid] = useState('nothing');
            return (
                <div>
                    <div key="tip" className="tip" style={{ width: 100, height: 30 }}
                        onMouseEnter={(e) => setSaid('in ' + Math.round(e.offsetX))} onMouseLeave={(e) => setSaid('out ' + e.type)}>Tip</div>
                    <p key="away" style={{ height: 30 }}>{'said ' + said}</p>
                </div>
            );
        }
        "#,
    );
    let [x, y, _, height] = hands.rect_of("tip");
    hands.move_at(x + 20.0, y + height / 2.0);
    assert!(hands.says("said in 20"), "{}", hands.app.outline());
    let (away_x, away_y) = hands.middle_of("away");
    hands.move_at(away_x, away_y);
    assert!(hands.says("said out mouseleave"), "{}", hands.app.outline());
    hands.move_at(x + 20.0, y + height / 2.0);
    assert!(hands.says("said in 20"), "{}", hands.app.outline());
    hands.app.pointer_leave();
    hands.settle();
    assert!(hands.says("said out mouseleave"), "{}", hands.app.outline());
}

/// Drags something from the middle of one thing to the middle of another, in steps, as a hand does.
fn drag_between(hands: &mut Hands, from: &str, to: &str) {
    let (from_x, from_y) = hands.middle_of(from);
    let (to_x, to_y) = hands.middle_of(to);
    hands.down_at(from_x, from_y);
    for step in 1..=5 {
        let along = step as f32 / 5.0;
        hands.move_at(from_x + (to_x - from_x) * along, from_y + (to_y - from_y) * along);
    }
    hands.up_at(to_x, to_y);
}

#[test]
fn something_draggable_can_be_dragged_onto_what_takes_it_and_escape_stops_it() {
    let mut hands = open(
        "drag-drop",
        r#"
        import React, { useState } from 'react';
        export default function App() {
            const [log, setLog] = useState([]);
            const note = (said) => setLog((was) => [...was, said]);
            return (
                <div>
                    <div key="card" draggable style={{ width: 80, height: 30 }}
                        onDragStart={(e) => { e.dataTransfer.setData('text/plain', 'card-1'); note('start'); }}
                        onDragEnd={() => note('end')} onClick={() => note('clicked')}>Card</div>
                    <div key="spacer" style={{ height: 30 }} />
                    <div key="zone" style={{ width: 200, height: 60 }}
                        onDragEnter={() => note('enter')} onDragLeave={() => note('leave')}
                        onDragOver={(e) => e.preventDefault()}
                        onDrop={(e) => note('dropped ' + e.dataTransfer.getData('text'))}>Zone</div>
                    <div key="refuses" style={{ width: 200, height: 40 }} onDrop={() => note('never')}>Refuses</div>
                    <p>{'log ' + log.join(', ')}</p>
                </div>
            );
        }
        "#,
    );
    drag_between(&mut hands, "card", "zone");
    assert!(hands.says("log start, enter, dropped card-1, end\""), "and it wasn't a click\n{}", hands.app.outline());
    // Where it doesn't say it can be dropped, it isn't. It goes over the zone on the way, which hears it come and go.
    drag_between(&mut hands, "card", "refuses");
    assert!(hands.says("card-1, end, start, enter, leave, end\""), "{}", hands.app.outline());
    // Escape ends it with nothing dropped.
    let (x, y) = hands.middle_of("card");
    let (zone_x, zone_y) = hands.middle_of("zone");
    hands.down_at(x, y);
    hands.move_at(x + 10.0, y + 10.0);
    hands.move_at(zone_x, zone_y);
    hands.press(KeyCode::Escape);
    hands.up_at(zone_x, zone_y);
    assert!(hands.says("leave, end, start, enter, leave, end\""), "{}", hands.app.outline());
    assert!(!hands.app.is_dragging());
    // A press that doesn't go far is still a click.
    hands.click("card");
    assert!(hands.says("end, clicked\""), "{}", hands.app.outline());
}

#[test]
fn files_and_text_from_other_programs_can_be_dropped_where_its_said_they_can() {
    let mut hands = open(
        "drop-outside",
        r#"
        import React, { useState } from 'react';
        export default function App() {
            const [log, setLog] = useState([]);
            const note = (said) => setLog((was) => [...was, said]);
            return (
                <div>
                    <div key="zone" style={{ width: 200, height: 60 }}
                        onDragOver={(e) => { e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; }}
                        onDrop={(e) => {
                            e.preventDefault();
                            const files = [...e.dataTransfer.files].map((file) => file.name).join('+');
                            note('dropped [' + files + '] [' + e.dataTransfer.getData('text') + '] ' + e.dataTransfer.types.join('/'));
                        }}>Zone</div>
                    <div key="elsewhere" style={{ width: 200, height: 40 }}>Elsewhere</div>
                    <p>{'log ' + log.join(', ')}</p>
                </div>
            );
        }
        "#,
    );
    let files = ["C:/photos/cat.png".to_owned(), "C:/notes/todo.txt".to_owned()];
    let (x, y) = hands.middle_of("zone");
    assert!(hands.app.drag_from_outside(rsui::DropPhase::Enter, x, y, &files, None));
    assert!(hands.app.drag_from_outside(rsui::DropPhase::Over, x, y, &files, None));
    assert!(hands.app.drag_from_outside(rsui::DropPhase::Drop, x, y, &files, None));
    hands.settle();
    assert!(hands.says("dropped [cat.png+todo.txt] [] Files"), "{}", hands.app.outline());
    let (x, y) = hands.middle_of("elsewhere");
    assert!(!hands.app.drag_from_outside(rsui::DropPhase::Enter, x, y, &[], Some("https://example.com")));
    assert!(!hands.app.drag_from_outside(rsui::DropPhase::Drop, x, y, &[], Some("https://example.com")));
    let (x, y) = hands.middle_of("zone");
    hands.app.drag_from_outside(rsui::DropPhase::Enter, x, y, &[], Some("https://example.com"));
    hands.app.drag_from_outside(rsui::DropPhase::Drop, x, y, &[], Some("https://example.com"));
    hands.settle();
    assert!(
        hands.says("dropped [] [https://example.com] text/plain/text/uri-list"),
        "{}",
        hands.app.outline()
    );
    assert!(rsui::runtime::wants_drops(), "a window would be told to listen for this");
}

/// The left and width of each line that says one of these, in the order they're asked for.
fn columns_of(laid_out: &str, texts: &[&str]) -> Vec<(f32, f32)> {
    texts
        .iter()
        .map(|text| {
            let line = laid_out
                .lines()
                .find(|line| line.contains(&format!("\"{text}\"")))
                .unwrap_or_else(|| panic!("no {text} in\n{laid_out}"));
            let numbers: Vec<f32> = line
                .trim_start()
                .split(' ')
                .nth(1)
                .unwrap()
                .split(',')
                .map(|one| one.parse().unwrap())
                .collect();
            (numbers[0], numbers[2])
        })
        .collect()
}

#[test]
fn a_tables_columns_line_up_wherever_its_rows_come_from() {
    let laid_out = page_rectangles(
        "table-components",
        "td, th { border: 1px solid black; } table { border-collapse: collapse; }",
        r#"{(() => {
            const Row = ({ name, count }) => <dom.tr><dom.td>{name}</dom.td><dom.td>{count}</dom.td></dom.tr>;
            return <dom.table><dom.thead><dom.tr><dom.th>Name</dom.th><dom.th>Count</dom.th></dom.tr></dom.thead>
                <dom.tbody><Row name="a much longer name" count="1" /><Row name="b" count="22222" />
                <dom.tr><dom.td colSpan={2}>both</dom.td></dom.tr>{[].length ? null : <dom.div>nothing yet</dom.div>}</dom.tbody></dom.table>;
        })()}"#,
    );
    // Each column is as wide as what's widest in it, in rows that came from a component as much as in the head.
    let found = columns_of(&laid_out, &["Name", "a much longer name", "b", "Count", "22222"]);
    let (name, long, short, count, number) = (found[0], found[1], found[2], found[3], found[4]);
    assert!(name.0 == long.0 && long.0 == short.0 && long.1 == short.1, "{laid_out}");
    assert!(count.0 == number.0 && count.1 == number.1 && count.0 > long.0 + long.1, "{laid_out}");
    // Across both, and something that isn't a row, which is in a row of its own, in the first column.
    let found = columns_of(&laid_out, &["both", "nothing yet"]);
    assert!(found[0].1 > long.1 + count.1, "{laid_out}");
    // It has none of a cell's padding, which is all that's to the left of the text in one.
    assert_eq!(found[1].0, long.0 - 2.0, "{laid_out}");
}

#[test]
fn css_can_make_anything_a_table() {
    let laid_out = page_rectangles(
        "display-table",
        ".t { display: table; } .r { display: table-row; } .c { display: table-cell; }",
        r#"<dom.div className="t"><dom.div className="r"><dom.span className="c">one</dom.span><dom.span className="c">two</dom.span></dom.div>
            <dom.div className="r"><dom.span className="c">three three</dom.span><dom.span className="c">4</dom.span></dom.div></dom.div>"#,
    );
    let found = columns_of(&laid_out, &["one", "two", "three three", "4"]);
    let (one, two, three, four) = (found[0], found[1], found[2], found[3]);
    assert!(one.0 == three.0 && two.0 == four.0 && two.0 >= three.0 + three.1, "{laid_out}");
}

#[test]
fn a_dialog_is_shown_over_everything_and_escape_cancels_it() {
    let mut hands = open(
        "dialog",
        r#"
        import React, { useRef, useState } from 'react';
        export default function App() {
            const ref = useRef(null);
            const [log, setLog] = useState([]);
            const note = (said) => setLog((was) => [...was, said]);
            const [open, setOpen] = useState(false);
            return (
                <div>
                    <button key="modal" onClick={() => ref.current.showModal()}>Modal</button>
                    <button key="by-id" onClick={() => document.getElementById('found').showModal()}>By id</button>
                    <button key="attribute" onClick={() => setOpen(!open)}>Attribute</button>
                    <dialog ref={ref} className="box" onCancel={() => note('cancel')} onClose={() => note('close ' + ref.current.returnValue)}>
                        <p>Modal one</p>
                        <form method="dialog"><button key="yes" value="yes">Yes</button></form>
                    </dialog>
                    <dialog id="found"><button key="done" onClick={() => document.getElementById('found').close()}>Done</button></dialog>
                    <dialog open={open}><p>Inline one</p></dialog>
                    <p>{'log ' + log.join(', ')}</p>
                </div>
            );
        }
        "#,
    );
    // Closed, it's there to be found, but not drawn.
    assert!(!hands.says("Modal one"), "{}", hands.app.outline());
    hands.click("modal");
    assert!(hands.says("dialog #__topLayer overlay=\"fill\""), "{}", hands.app.outline());
    // Pressing what's behind it does nothing, and it has the keyboard to itself.
    hands.click_at(400.0, 380.0);
    assert!(hands.says("Modal one"), "{}", hands.app.outline());
    hands.press(KeyCode::Escape);
    hands.wait_for("log cancel, close \"");
    assert!(!hands.says("Modal one"), "{}", hands.app.outline());
    // Sending a form that says it's for the dialog closes it, with what sent it.
    hands.click("modal");
    hands.click("yes");
    hands.wait_for("close , close yes\"");
    hands.click("by-id");
    hands.click("done");
    assert!(!hands.has("done"), "{}", hands.app.outline());
    hands.click("attribute");
    assert!(hands.says("Inline one") && !hands.says("__topLayer"), "{}", hands.app.outline());
}

#[test]
fn where_the_page_is_and_where_its_been_are_kept_as_a_browser_keeps_them() {
    let mut hands = open(
        "history",
        r##"
        import React, { useEffect, useState } from 'react';
        export default function App() {
            const [log, setLog] = useState([]);
            const note = (said) => setLog((was) => [...was, said]);
            useEffect(() => {
                const popped = (e) => note('pop ' + location.pathname + location.hash + ' ' + JSON.stringify(e.state));
                const hashed = (e) => note('hash ' + new URL(e.newURL).hash);
                window.addEventListener('popstate', popped);
                window.addEventListener('hashchange', hashed);
                return () => { window.removeEventListener('popstate', popped); window.removeEventListener('hashchange', hashed); };
            }, []);
            return (
                <div>
                    <button key="push" onClick={() => { history.pushState({ n: 1 }, '', '/one?x=2'); note('at ' + location.pathname + location.search + ' ' + history.length); }}>Push</button>
                    <button key="back" onClick={() => history.back()}>Back</button>
                    <a key="hash" href="#top">Hash</a>
                    <a key="same" href="/two" onClick={(e) => { e.preventDefault(); note('kept'); }}>Kept</a>
                    <p>{'log ' + log.join(', ')}</p>
                </div>
            );
        }
        "##,
    );
    hands.click("push");
    assert!(hands.says("log at /one?x=2 2\""), "{}", hands.app.outline());
    hands.click("back");
    hands.wait_for("pop / null\"");
    hands.click("hash");
    hands.wait_for("pop /#top null, hash #top\"");
    hands.click("same");
    assert!(hands.says("hash #top, kept\""), "{}", hands.app.outline());
}

#[test]
fn elements_say_what_they_are_and_where_and_hear_what_they_listen_for() {
    let mut hands = open(
        "elements",
        r#"
        import React, { useEffect, useRef, useState } from 'react';
        export default function App() {
            const card = useRef(null);
            const [log, setLog] = useState([]);
            const note = (said) => setLog((was) => [...was, said]);
            useEffect(() => {
                const node = card.current;
                node.setAttribute('draggable', 'true');
                node.addEventListener('dragstart', () => note('dragstart on the element'));
                const found = document.querySelector('[data-kind="target"]');
                note((node instanceof HTMLElement) + ' ' + node.nodeType + ' ' + (node.ownerDocument === document) + ' ' + (found?.dataset.kind ?? 'none'));
                setTimeout(() => {
                    const box = node.getBoundingClientRect();
                    const at = document.elementFromPoint(box.x + 5, box.y + 5);
                    note('box ' + box.width + 'x' + box.height + ' ' + (at === node) + ' ' + getComputedStyle(node).marginTop);
                }, 30);
            }, []);
            return (
                <div>
                    <div key="card" ref={card} style={{ width: 80, height: 30, marginTop: 6 }}>Card</div>
                    <div key="zone" data-kind="target" style={{ width: 200, height: 60 }}>Zone</div>
                    <p>{'log ' + log.join(', ')}</p>
                </div>
            );
        }
        "#,
    );
    hands.wait_for("log true 1 true target, box 80x30 true 6px\"");
    // What has a `ref` is known by the key that gives it, so it's found from where it is, just above the zone.
    let [x, y, _, _] = hands.rect_of("zone");
    let (to_x, to_y) = hands.middle_of("zone");
    hands.down_at(x + 40.0, y - 15.0);
    for step in 1..=5 {
        let along = step as f32 / 5.0;
        hands.move_at(x + 40.0 + (to_x - x - 40.0) * along, y - 15.0 + (to_y - y + 15.0) * along);
    }
    hands.up_at(to_x, to_y);
    assert!(hands.says("6px, dragstart on the element\""), "{}", hands.app.outline());
}

/// Where the text that says this is, as `[x, y, width, height]`, in what `page_rectangles` gives.
fn box_of(laid_out: &str, text: &str) -> [f32; 4] {
    let line = laid_out
        .lines()
        .find(|line| line.contains(&format!("\"{text}\"")))
        .unwrap_or_else(|| panic!("no {text} in\n{laid_out}"));
    let numbers: Vec<f32> = line
        .trim_start()
        .split(' ')
        .nth(1)
        .unwrap()
        .split(',')
        .map(|one| one.parse().unwrap())
        .collect();
    [numbers[0], numbers[1], numbers[2], numbers[3]]
}

/// A page with this CSS and this in its body, by itself, to find things in by their keys.
fn open_page(name: &str, css: &str, body: &str) -> Hands {
    open(
        name,
        &format!(
            r#"
            import React from 'react';
            import {{ PageWindow, addCss, dom }} from 'rsui/web';
            addCss({css:?});
            export default function App() {{
                return <PageWindow>{body}</PageWindow>;
            }}
            "#
        ),
    )
}

#[test]
fn a_body_that_is_a_flex_box_puts_the_page_where_it_says() {
    // As electron-react-boilerplate's does: the page's root element is in the middle of the window, and a block itself, as on a page.
    let laid_out = page_rectangles(
        "body-flex",
        "body { margin: 0; overflow: hidden; display: flex; justify-content: center; align-items: center; }",
        r#"<dom.h1>A title in the middle</dom.h1><dom.p>Under it</dom.p>"#,
    );
    let [x, y, width, height] = box_of(&laid_out, "A title in the middle");
    assert!((x + width / 2.0 - 250.0).abs() < 1.0, "{laid_out}");
    let under = box_of(&laid_out, "Under it");
    assert!(under[1] > y + height && under[0] >= x, "{laid_out}");
    // Both together are in the middle, down, with their margins, which go through the root element as it has no padding: the
    // title's 21.44 above, and 16 under what's under it.
    let middle = y + (under[1] + under[3] - y) / 2.0;
    assert!((middle - (200.0 + (21.44 - 16.0) / 2.0)).abs() < 0.5, "{laid_out}");
}

#[test]
fn something_placed_by_itself_that_says_nowhere_is_where_its_flex_box_puts_things() {
    // As shadcn's theme toggle has it: two icons over each other in a button that puts what's in it in the middle.
    let mut hands = open_page(
        "static-position",
        "body { margin: 0; } .b { display: inline-flex; align-items: center; justify-content: center; width: 40px; height: 40px; position: relative; padding: 0; border: 0; } .i { width: 16px; height: 16px; } .a { position: absolute; }",
        r#"<dom.button key="b" className="b"><dom.span key="sun" className="i" /><dom.span key="moon" className="i a" /></dom.button>"#,
    );
    let (button, sun, moon) = (hands.rect_of("b"), hands.rect_of("sun"), hands.rect_of("moon"));
    assert_eq!(moon, sun, "{}", hands.app.outline());
    assert_eq!((sun[0] - button[0], sun[1] - button[1]), (12.0, 12.0), "{}", hands.app.outline());
}

#[test]
fn a_pictures_size_in_em_is_as_big_as_its_text() {
    // As react-icons draws its icons: `width="1em"`, which isn't what its `viewBox` says.
    let mut hands = open_page(
        "svg-em",
        "",
        r#"<dom.div style={{ fontSize: 20 }}><dom.svg key="icon" width="1em" height="1em" viewBox="0 0 448 512"><dom.path d="M0 0h448v512z" /></dom.svg></dom.div>"#,
    );
    let [_, _, width, height] = hands.rect_of("icon");
    assert_eq!((width, height), (20.0, 20.0), "{}", hands.app.outline());
}

#[test]
fn a_pictures_attributes_written_as_svg_writes_them_say_what_their_props_do() {
    // As a page's HTML has them, which React passes on as they are: `stroke-width` rather than `strokeWidth`. On the `<svg>`, for
    // all of it, and on each part, where CSS goes over them.
    let mut hands = open_page(
        "svg-dashed",
        ".thick { stroke-width: 3px; }",
        r#"<dom.svg key="icon" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.5" stroke-linecap="round">
            <dom.path d="M1 1h10" />
            <dom.path stroke-width="1.5" stroke-linejoin="bevel" fill-rule="evenodd" d="M2 2h10" />
            <dom.path className="thick" stroke-width="1" d="M3 3h10" />
            <dom.path stroke-width="4px" d="M4 4h10" />
        </dom.svg>"#,
    );
    hands.settle();
    let tree = format!("{:?}", hands.app.runtime().root());
    let layers: Vec<&str> = tree.split(r"\n").collect();
    let layer = |d: &str| layers.iter().find(|layer| layer.contains(d)).copied().unwrap_or_else(|| panic!("no {d} in {tree}"));
    assert!(layer("M1 1h10").contains(r"\t2.5\tround\t"), "{}", layer("M1 1h10"));
    assert!(layer("M2 2h10").contains(r"\t1.5\tround\t"), "{}", layer("M2 2h10"));
    assert!(layer("M2 2h10").contains(r"\tbevel\t") && layer("M2 2h10").contains("evenodd"), "{}", layer("M2 2h10"));
    assert!(layer("M3 3h10").contains(r"\t3\tround\t"), "{}", layer("M3 3h10"));
    // A width with its unit is that many of the picture's own units, as one without is.
    assert!(layer("M4 4h10").contains(r"\t4\tround\t"), "{}", layer("M4 4h10"));
}

#[test]
fn lh_is_a_line_of_the_text() {
    css_gives("lh", "min-height: 2lh;", &[("minHeight", "38.4")]);
    css_gives("rlh", "padding-top: 1rlh;", &[("paddingTop", "19.2")]);
}

#[test]
fn what_has_no_box_of_its_own_is_part_of_what_it_is_in() {
    let fragment = r#"{(() => {
        const Two = () => <><dom.span key="f1" className="w" /><dom.span key="f2" className="w" /></>;
        return <dom.div className="row"><Two /><dom.div className="c"><dom.span key="c1" className="w" /></dom.div></dom.div>;
    })()}"#;
    let mut hands = open_page(
        "contents",
        "body { margin: 0; } .row { display: flex; justify-content: center; gap: 10px; } .w { display: block; width: 50px; height: 10px; } .c { display: contents; background: red; padding: 20px; }",
        fragment,
    );
    // A fragment's children, and those of what CSS says is `display: contents`, are the flex box's own: three of them, in the middle.
    let (one, two, three) = (hands.rect_of("f1"), hands.rect_of("f2"), hands.rect_of("c1"));
    assert_eq!([one[0], two[0], three[0]], [165.0, 225.0, 285.0], "{}", hands.app.outline());
    assert_eq!([one[1], two[1], three[1]], [0.0, 0.0, 0.0], "{}", hands.app.outline());
    // Nothing of its box is drawn.
    let outline = hands.app.outline();
    let own = outline.lines().find(|line| line.contains("class=\"c\"")).expect("it's there");
    assert!(!own.contains("background") && !own.contains("padding"), "{outline}");
}

#[test]
fn a_cell_that_is_a_flex_box_is_still_in_its_column() {
    // As daisyUI's tables are written, with `<th class="flex">`: it's in a cell that's made up around it.
    let laid_out = page_rectangles(
        "flex-cell",
        "td, th { padding: 4px; text-align: left; } .f { display: flex; align-items: center; gap: 4px; }",
        r#"<dom.table><dom.thead><dom.tr><dom.th>Name</dom.th><dom.th className="f"><dom.span>+</dom.span><dom.span>Add</dom.span></dom.th></dom.tr></dom.thead>
            <dom.tbody><dom.tr><dom.td>a much longer name</dom.td><dom.td>ok</dom.td></dom.tr></dom.tbody></dom.table>"#,
    );
    let found = columns_of(&laid_out, &["Name", "a much longer name", "+", "ok"]);
    let (name, long, plus, ok) = (found[0], found[1], found[2], found[3]);
    assert_eq!(name.0, long.0, "{laid_out}");
    assert_eq!(plus.0, ok.0, "{laid_out}");
    assert!(plus.0 >= long.0 + long.1, "{laid_out}");
    // Both of what's in it are on one line.
    assert_eq!(box_of(&laid_out, "+")[1], box_of(&laid_out, "Add")[1], "{laid_out}");
}

#[test]
fn where_something_goes_in_a_grid_can_be_said_one_side_at_a_time() {
    let mut hands = open_page(
        "grid-sides",
        "body { margin: 0; } .g { display: grid; grid-template-columns: 50px 50px 50px 50px; } .s { grid-column: span 2 / span 2; } .start { grid-column-start: 2; } .end { grid-row-start: 2; grid-column-end: -1; } .area { grid-area: 3 / 1 / 4 / 3; }",
        r#"<dom.div className="g"><dom.div key="s" className="s start" style={{ height: 10 }} /><dom.div key="e" className="end" style={{ height: 10 }} /><dom.div key="a" className="area" style={{ height: 10 }} /></dom.div>"#,
    );
    assert_eq!(hands.rect_of("s"), [50.0, 0.0, 100.0, 10.0], "{}", hands.app.outline());
    assert_eq!(hands.rect_of("e"), [150.0, 10.0, 50.0, 10.0], "{}", hands.app.outline());
    assert_eq!(hands.rect_of("a"), [0.0, 20.0, 100.0, 10.0], "{}", hands.app.outline());
}

#[test]
fn a_rejection_nothing_catches_is_logged_and_heard_by_the_window_rather_than_shown_over_the_app() {
    let mut hands = open(
        "unhandled-rejection",
        r#"
        import { useEffect, useState } from 'react';
        export default function App() {
            const [heard, setHeard] = useState('nothing');
            const [count, setCount] = useState(0);
            useEffect(() => {
                const listen = (event) => setHeard(event.type + ' ' + event.reason.message + ' ' + (event.promise instanceof Promise));
                window.addEventListener('unhandledrejection', listen);
                // As an API that doesn't answer has it.
                fetchThatFails();
                return () => window.removeEventListener('unhandledrejection', listen);
            }, []);
            return (
                <div>
                    <p>{'heard ' + heard}</p>
                    <button key="count" className="count" onClick={() => setCount(count + 1)}>{'count ' + count}</button>
                    <button key="later" className="later" onClick={async () => { await null; throw new Error('after a click'); }}>later</button>
                </div>
            );
        }
        async function fetchThatFails() {
            throw new Error('offline');
        }
        "#,
    );
    hands.settle();
    hands.settle();
    let outline = hands.app.outline();
    assert!(outline.contains("heard unhandledrejection offline true"), "{outline}");
    // What's shown is the app, and it goes on.
    hands.click("count");
    hands.settle();
    assert!(hands.says("count 1"), "{}", hands.app.outline());
    hands.click("later");
    hands.settle();
    hands.settle();
    let outline = hands.app.outline();
    assert!(outline.contains("heard unhandledrejection after a click true"), "{outline}");
    assert!(outline.contains("count 1"), "{outline}");
}

#[test]
fn a_callback_ref_is_cleaned_up_by_what_it_gave_back_and_an_object_ref_is_emptied_when_its_element_goes() {
    let mut hands = open(
        "ref-cleanup",
        r#"
        import { useRef, useState } from 'react';
        const log = [];
        // As React 19 has it: what it gives back is called instead of it being given null.
        const watch = (node) => {
            log.push('set ' + node.tagName);
            return () => log.push('cleaned');
        };
        const plain = (node) => log.push(node === null ? 'plain null' : 'plain ' + node.tagName);
        export default function App() {
            const held = useRef(null);
            const [on, setOn] = useState(true);
            const [said, setSaid] = useState('');
            return (
                <div>
                    {on && <section ref={held}>box</section>}
                    {on && <div ref={watch}>watched</div>}
                    {on && <p ref={plain}>plain</p>}
                    <button key="toggle" onClick={() => setOn(!on)}>toggle</button>
                    <button key="read" onClick={() => setSaid(String(held.current?.tagName ?? null) + ' / ' + log.join(','))}>read</button>
                    <p>{'said ' + said}</p>
                </div>
            );
        }
        "#,
    );
    hands.settle();
    hands.click("read");
    hands.settle();
    assert!(hands.says("said SECTION / set DIV,plain P"), "{}", hands.app.outline());
    hands.click("toggle");
    hands.settle();
    hands.click("read");
    hands.settle();
    assert!(hands.says("said null / set DIV,plain P,cleaned,plain null"), "{}", hands.app.outline());
}

#[test]
fn a_form_with_an_action_sends_what_its_fields_say_and_is_pending_while_that_runs() {
    let mut hands = open(
        "form-action",
        r#"
        import { useRef, useState } from 'react';
        import { useFormStatus } from 'react-dom';
        let finish = null;
        function Status() {
            const { pending, data, method } = useFormStatus();
            return <p>{pending ? 'sending ' + data.get('title') + ' by ' + method : 'idle'}</p>;
        }
        export default function App() {
            const [saved, setSaved] = useState([]);
            const form = useRef(null);
            async function save(data) {
                await new Promise((done) => { finish = done; });
                setSaved((was) => [...was, [data.get('title'), data.get('kind'), data.getAll('tag').join('+'), data.get('via') ?? '-'].join('/')]);
            }
            return (
                <div>
                    <form action={save} ref={form}>
                        <input key="title" name="title" />
                        <input type="checkbox" name="tag" value="a" defaultChecked />
                        <input type="checkbox" name="tag" value="b" />
                        <input name="off" defaultValue="never" disabled />
                        <select name="kind" defaultValue="y"><option value="x">X</option><option value="y">Y</option></select>
                        <button key="send" name="via" value="button">Send</button>
                        <Status />
                    </form>
                    <button key="request" onClick={() => form.current.requestSubmit()}>request</button>
                    <button key="count" onClick={() => setSaved((was) => [...was, 'fields ' + form.current.elements.length + ' ' + new FormData(form.current).get('kind')])}>count</button>
                    <button key="finish" onClick={() => finish?.()}>finish</button>
                    <p>{'saved ' + saved.join(',')}</p>
                </div>
            );
        }
        "#,
    );
    hands.settle();
    assert!(hands.says("idle"), "{}", hands.app.outline());
    hands.click("title");
    hands.type_text("first");
    hands.press(KeyCode::Enter);
    hands.settle();
    // Enter sends it as its first button that sends it would, which says what it is.
    assert!(hands.says("sending first by get"), "{}", hands.app.outline());
    hands.click("finish");
    hands.settle();
    hands.settle();
    assert!(hands.says("saved first/y/a/button"), "{}", hands.app.outline());
    assert!(hands.says("idle"), "{}", hands.app.outline());
    // Its fields start again once it's been sent, as React has them.
    assert!(!hands.app.outline().contains("\"first\""), "{}", hands.app.outline());
    hands.click("title");
    hands.type_text("second");
    hands.click("request");
    hands.settle();
    assert!(hands.says("sending second by get"), "{}", hands.app.outline());
    hands.click("finish");
    hands.settle();
    hands.settle();
    assert!(hands.says("saved first/y/a/button,second/y/a/-"), "{}", hands.app.outline());
    hands.click("count");
    hands.settle();
    // What has a value is among its `elements`, as it is on a page, though its buttons aren't yet.
    assert!(hands.says(",fields 5 y"), "{}", hands.app.outline());
}

#[test]
fn style_put_anywhere_in_head_counts_in_the_order_it_is_there() {
    let mut hands = open(
        "head-order",
        r#"
        import 'rsui/web';
        import { useState } from 'react';
        const style = (css) => {
            const made = document.createElement('style');
            made.textContent = css;
            return made;
        };
        // What's later counts over what's earlier, whichever way it got there.
        const late = style('.box { color: #000011; }');
        document.head.appendChild(late);
        document.head.prepend(style('.box { color: #000022; } .other { color: #000044; }'));
        const meta = document.createElement('meta');
        meta.setAttribute('name', 'theme');
        document.head.append(meta);
        document.head.insertBefore(style('.box { color: #000033; }'), late);
        export default function App() {
            const [count, setCount] = useState(0);
            return (
                <div>
                    <p className="box">box</p>
                    <p className="other">other</p>
                    <button key="move" onClick={() => { document.head.insertBefore(late, document.head.firstChild); setCount(count + 1); }}>move</button>
                    <p>{'found ' + document.head.querySelectorAll('style').length + ' ' + document.querySelector('meta[name="theme"]')?.tagName + ' ' + count}</p>
                </div>
            );
        }
        "#,
    );
    hands.settle();
    let color = |hex: i64| (hex * 256 + 255).to_string();
    let tree = format!("{:?}", hands.app.runtime().root());
    assert!(hands.says("found 3 META 0"), "{}", hands.app.outline());
    assert!(tree.contains(&color(0x000011)), "the last in head counts: {tree}");
    assert!(tree.contains(&color(0x000044)), "{tree}");
    hands.click("move");
    hands.settle();
    let tree = format!("{:?}", hands.app.runtime().root());
    // Now it's first, so the one that was before it counts.
    assert!(tree.contains(&color(0x000033)), "moved to the start of head, it counts least: {tree}");
    assert!(!tree.contains(&color(0x000011)), "{tree}");
}

#[test]
fn group_and_peer_classes_follow_how_what_they_are_about_is() {
    let file = write(
        "group-peer",
        r#"
        import './app.css';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            return (
                <PageWindow><div>
                    <div key="group" className="group card">
                        <p className="label">label</p>
                    </div>
                    <label>
                        <input key="tick" type="checkbox" className="peer" />
                        <span className="note">note</span>
                        <span className="next">next</span>
                    </label>
                    <ul>
                        <li className="row">one</li>
                        <li className="row skip">two</li>
                        <li className="row">three</li>
                        <li className="row">four</li>
                    </ul>
                </div></PageWindow>
            );
        }
        "#,
    );
    std::fs::write(
        file.parent().unwrap().join("app.css"),
        r#"
        .card { padding: 20px; }
        .group:hover .group-hover\:x, .group:hover .label { color: #000011; }
        .peer:checked ~ .note { color: #000022; }
        .peer:checked + .note { background: #000033; }
        .peer:checked ~ .next { color: #000044; }
        .peer:checked + .next { color: #000055; }
        .row:nth-child(2 of .row:not(.skip)) { color: #000066; }
        li + li.skip { background: #000077; }
        "#,
    )
    .unwrap();
    let mut hands = Hands::open(&file, (500, 400));
    hands.settle();
    let color = |hex: i64| (hex * 256 + 255).to_string();
    let tree = format!("{:?}", hands.app.runtime().root());
    assert!(!tree.contains(&color(0x000011)), "{tree}");
    assert!(!tree.contains(&color(0x000022)), "{tree}");
    // The second of those that aren't skipped is the third, and what's right after the first is the skipped one.
    assert!(tree.contains(&color(0x000066)), "{tree}");
    assert!(tree.contains(&color(0x000077)), "{tree}");
    let (x, y) = hands.middle_of("group");
    hands.move_at(x, y);
    hands.settle();
    let tree = format!("{:?}", hands.app.runtime().root());
    assert!(tree.contains(&color(0x000011)), "what's in a group that's hovered: {tree}");
    hands.move_at(400.0, 390.0);
    hands.settle();
    let tree = format!("{:?}", hands.app.runtime().root());
    assert!(!tree.contains(&color(0x000011)), "{tree}");
    hands.click("tick");
    hands.settle();
    hands.settle();
    let tree = format!("{:?}", hands.app.runtime().root());
    assert!(tree.contains(&color(0x000022)), "what's after a box that's ticked: {tree}");
    assert!(tree.contains(&color(0x000033)), "{tree}");
    assert!(tree.contains(&color(0x000044)), "{tree}");
    assert!(!tree.contains(&color(0x000055)), "it isn't right after it: {tree}");
}

#[test]
fn a_peer_reaches_inside_what_comes_after_it() {
    // Mantine's Switch: `input:checked + * > .thumb { --switch-thumb-start: ... }` moves the thumb, inside the track that comes after
    // the input, so the thumb has to follow the input when it's unticked, not only the track.
    let file = write(
        "deep-peer",
        r#"
        import './app.css';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            return (
                <PageWindow><div>
                    <label>
                        <input key="tick" type="checkbox" className="peer" defaultChecked />
                        <span className="track"><span className="thumb">thumb</span></span>
                    </label>
                </div></PageWindow>
            );
        }
        "#,
    );
    std::fs::write(
        file.parent().unwrap().join("app.css"),
        r#"
        .thumb { color: var(--thumb, #000099); }
        .peer:checked + * > .thumb { --thumb: #000088; }
        .peer:checked + .track > .thumb { background: #000077; }
        "#,
    )
    .unwrap();
    let mut hands = Hands::open(&file, (500, 400));
    hands.settle();
    hands.settle();
    let color = |hex: i64| (hex * 256 + 255).to_string();
    let tree = format!("{:?}", hands.app.runtime().root());
    assert!(tree.contains(&color(0x000088)), "a ticked box's thumb: {tree}");
    assert!(tree.contains(&color(0x000077)), "{tree}");
    hands.click("tick");
    hands.settle();
    hands.settle();
    let tree = format!("{:?}", hands.app.runtime().root());
    assert!(tree.contains(&color(0x000099)), "unticked, the thumb goes back: {tree}");
    assert!(!tree.contains(&color(0x000088)), "{tree}");
    assert!(!tree.contains(&color(0x000077)), "{tree}");
    hands.click("tick");
    hands.settle();
    hands.settle();
    let tree = format!("{:?}", hands.app.runtime().root());
    assert!(tree.contains(&color(0x000088)), "ticked again: {tree}");
}

#[test]
fn transform_none_wins_over_an_earlier_transform() {
    // Mantine's checkbox icon is `translateY(5px) scale(0.5)` until `input:checked + .icon { transform: none }`, which has to take
    // that away rather than say nothing, or a ticked box's mark is small and low.
    let file = write(
        "transform-none",
        r#"
        import './app.css';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            return (
                <PageWindow><div>
                    <p className="on">on</p>
                    <span className="icon">checked</span>
                    <p>off</p>
                    <span className="icon">unchecked</span>
                    <span className="icon" style={{ transform: 'none' }}>written</span>
                </div></PageWindow>
            );
        }
        "#,
    );
    std::fs::write(
        file.parent().unwrap().join("app.css"),
        r#"
        .icon { display: block; transform: translateY(5px) scale(0.5); }
        .on + .icon { transform: none; }
        "#,
    )
    .unwrap();
    let mut hands = Hands::open(&file, (500, 400));
    hands.settle();
    let tree = format!("{:?}", hands.app.runtime().root());
    assert_eq!(tree.matches("scale(0.5)").count(), 1, "only the one after `off` is moved: {tree}");
    assert!(!tree.contains("\"none\""), "no transform is no transform at all: {tree}");
}

#[test]
fn query_selector_matches_as_css_does() {
    let mut hands = open(
        "query-selector",
        r#"
        import { useRef, useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('');
            const item = useRef(null);
            const ask = () => {
                const q = (selector) => document.querySelectorAll(selector).length;
                const found = [
                    q('li'),
                    q('ul > li:nth-child(odd)'),
                    q('li:nth-of-type(2)'),
                    q('li.a + li'),
                    q('li.a ~ li'),
                    q('section p:not(.quiet)'),
                    q('section:has(> p.quiet)'),
                    q(':is(h2, h3).title'),
                    q('li:nth-child(2 of .b)'),
                    q('input[type="checkbox"]:checked'),
                    document.querySelector('section h2')?.textContent,
                    item.current.matches('ul > li.b'),
                    item.current.closest('section')?.tagName,
                    String(item.current.closest('div.missing')),
                    document.getElementsByTagName('h3').length,
                    document.querySelector('ul').querySelectorAll(':scope > li.b').length,
                ];
                setSaid(found.join(' '));
            };
            return (
                <div>
                    <section>
                        <h2 className="title">Heading</h2>
                        <h3 className="title">Sub</h3>
                        <p>one</p>
                        <p className="quiet">two</p>
                        <ul>
                            <li className="a">a</li>
                            <li className="b">b</li>
                            <li ref={item} className="b">c</li>
                            <li>d</li>
                        </ul>
                        <input type="checkbox" checked readOnly />
                    </section>
                    <button key="ask" onClick={ask}>ask</button>
                    <p>{'said ' + said}</p>
                </div>
            );
        }
        "#,
    );
    hands.settle();
    hands.click("ask");
    hands.settle();
    assert!(hands.says("said 4 2 1 1 3 1 1 2 1 1 Heading true SECTION null 1 2"), "{}", hands.app.outline());
}

#[test]
fn closest_and_matches_say_the_same_once_the_tree_has_changed() {
    // Right after the tree changes, `closest()` and `matches()` with what only asks about an element and what it's in are
    // answered from what's along the way to it (`nodeUpOf` in web.js), and the rest from all of the page: each says just what
    // it says once all of the page has been made again.
    let mut hands = open(
        "closest-after-change",
        r#"
        import { useEffect, useRef, useState } from 'react';
        export default function App() {
            const [count, setCount] = useState(0);
            const [said, setSaid] = useState('');
            const item = useRef(null);
            const link = useRef(null);
            useEffect(() => {
                if (count === 0) {
                    return;
                }
                const ask = () => [
                    item.current.closest('section')?.tagName,
                    item.current.closest('.box > ul')?.tagName,
                    item.current.closest('[data-kind="list"]')?.getAttribute('data-kind'),
                    item.current.closest('div section li.b')?.textContent,
                    item.current.closest(':is(.box, .other) li')?.textContent,
                    item.current.closest('section:not(.box)')?.tagName,
                    String(item.current.closest('p')),
                    item.current.matches('ul > li.b'),
                    item.current.matches('section li:not(.a)'),
                    item.current.matches('li:nth-child(3)'),
                    item.current.matches('li.b + li'),
                    item.current.closest('li:last-child')?.textContent,
                    item.current.closest('ul:has(> li.a)')?.tagName,
                    link.current.closest('a')?.getAttribute('href'),
                    link.current.closest('a span')?.tagName,
                ].map(String).join(' ');
                const first = ask();
                // All of the page, made again.
                document.querySelectorAll('*');
                const again = ask();
                setSaid(first === again ? 'same ' + first : 'first ' + first + ' then ' + again);
            }, [count]);
            return (
                <div>
                    <section className="box">
                        <p>{'count ' + count}</p>
                        <ul data-kind="list">
                            <li className="a">a</li>
                            <li className="b">b</li>
                            <li ref={item} className="b">c</li>
                        </ul>
                        <a href="/there"><span ref={link}>{'go ' + count}</span></a>
                    </section>
                    <button key="more" onClick={() => setCount(count + 1)}>more</button>
                    <p>{'said ' + said}</p>
                </div>
            );
        }
        "#,
    );
    hands.settle();
    hands.click("more");
    hands.settle();
    hands.settle();
    assert!(
        hands.says("said same SECTION UL list undefined c undefined null true true true true c UL /there SPAN"),
        "{}",
        hands.app.outline()
    );
}

#[test]
fn scrolling_is_what_page_code_reads_and_sets_and_hears() {
    let mut hands = open(
        "scrolling",
        r#"
        import { useRef, useState } from 'react';
        export default function App() {
            const box = useRef(null);
            const [said, setSaid] = useState('');
            const [heard, setHeard] = useState(0);
            const read = () => {
                const at = box.current;
                setSaid([at.scrollTop, at.scrollHeight, at.clientHeight].join(' '));
            };
            const rows = [];
            for (let at = 0; at < 50; at += 1) {
                rows.push(<p key={at} id={'row' + at} style={{ height: 20, margin: 0 }}>{'row ' + at}</p>);
            }
            return (
                <div>
                    <div key="box" ref={box} style={{ height: 100, overflow: 'auto' }} onScroll={() => setHeard((n) => n + 1)}>{rows}</div>
                    <button key="down" onClick={() => { box.current.scrollTop = 300; read(); }}>down</button>
                    <button key="far" onClick={() => { box.current.scrollTo({ top: 5000 }); read(); }}>far</button>
                    <button key="into" onClick={() => { document.getElementById('row10').scrollIntoView(); }}>into</button>
                    <button key="read" onClick={read}>read</button>
                    <p>{'said ' + said + ' heard ' + heard}</p>
                </div>
            );
        }
        "#,
    );
    hands.settle();
    hands.click("read");
    hands.settle();
    assert!(hands.says("said 0 1000 100"), "{}", hands.app.outline());
    hands.click("down");
    hands.settle();
    hands.settle();
    // What's set is what it says straight away, and it's scrolled there.
    assert!(hands.says("said 300 1000 100 heard 1"), "{}", hands.app.outline());
    assert!(hands.app.outline().contains("scroll_y=300"), "{}", hands.app.outline());
    hands.click("far");
    hands.settle();
    hands.settle();
    assert!(hands.says("said 900 1000 100 heard 2"), "{}", hands.app.outline());
    hands.click("into");
    hands.settle();
    hands.settle();
    hands.click("read");
    hands.settle();
    assert!(hands.says("said 200 1000 100 heard 3"), "{}", hands.app.outline());
    // The wheel scrolls it as a person would, and that's heard too.
    let (x, y) = hands.middle_of("12");
    hands.move_at(x, y);
    hands.app.scroll(x, y, 1).unwrap();
    hands.settle();
    hands.settle();
    hands.click("read");
    hands.settle();
    let outline = hands.app.outline();
    assert!(!outline.contains("said 200 1000 100"), "{outline}");
    assert!(outline.contains("heard 4"), "{outline}");
}

#[test]
fn scrollbar_color_and_width_say_what_a_scrollbar_looks_like() {
    css_gives(
        "scrollbar-standard",
        "scrollbar-width: thin; scrollbar-color: #ff0000 #00ff00;",
        &[("__scrollbarWidth", "thin"), ("__scrollbarColor", "said"), ("__scrollbarThumb", "#ff0000"), ("__scrollbarTrack", "#00ff00")],
    );
    let hands = open(
        "scrollbar-drawn",
        r#"
        export default function App() {
            const rows = [];
            for (let at = 0; at < 30; at += 1) {
                rows.push(<p key={at}>{'row ' + at}</p>);
            }
            return (
                <div>
                    <div style={{ height: 100, overflow: 'auto', scrollbarWidth: 'thin', scrollbarColor: '#ff0000 #00ff00' }}>{rows}</div>
                    <div style={{ height: 100, overflow: 'auto', scrollbarWidth: 'none' }}>{rows}</div>
                </div>
            );
        }
        "#,
    );
    let tree = format!("{:?}", hands.app.runtime().root());
    assert!(tree.contains("scrollbar_size"), "{tree}");
    assert!(tree.contains(&(0xff0000_i64 * 256 + 255).to_string()), "{tree}");
    assert!(tree.contains(&(0x00ff00_i64 * 256 + 255).to_string()), "{tree}");
}

#[test]
fn what_a_styled_component_works_out_from_its_props_is_only_worked_out_with_them() {
    let mut hands = open(
        "styled-props-only",
        r#"
        import { useState } from 'react';
        import { __styled as styled } from 'rsui/web';
        const warned = [];
        console.warn = (...said) => warned.push(said.join(' '));
        const Ring = styled.div`
            width: ${(p) => `${p.size}px`};
            #progress { stroke-dashoffset: ${(p) => `${p.offset}px`}; transition: ${(p) => p.animate === 'true' && 'stroke-dashoffset 1s linear'}; }
        `;
        // What layout asks of a styled component before it's drawn, which is what it says with no props.
        const guessed = JSON.stringify(Ring.__webLook());
        export default function App() {
            const [shown, setShown] = useState('');
            return (
                <div>
                    <p>{'guessed ' + guessed}</p>
                    before <Ring size={37} offset={5}><svg><circle id="progress" r="4" /></svg></Ring> after
                    <button key="read" onClick={() => setShown(warned.join('|'))}>read</button>
                    <p>{'warned ' + shown + '.'}</p>
                </div>
            );
        }
        "#,
    );
    hands.settle();
    hands.click("read");
    hands.settle();
    let outline = hands.app.outline();
    assert!(outline.contains("warned "), "{outline}");
    assert!(!outline.contains("undefined"), "nothing's worked out from props that aren't there: {outline}");
    let tree = format!("{:?}", hands.app.runtime().root());
    assert!(tree.contains("Float(37.0)"), "what the props say applies: {tree}");
}

#[test]
fn a_page_element_takes_every_event_react_dom_knows_and_a_form_hears_its_reset() {
    let mut hands = open(
        "events-react-knows",
        r#"
        import { useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('nothing yet');
            const [focus, setFocus] = useState('none');
            return (
                <div onTouchStart={() => {}} onTouchMoveCapture={() => {}} onPointerCancel={() => {}} onLostPointerCapture={() => {}}>
                    <input key="field" placeholder="search" onAnimationStart={() => {}} onTransitionStart={() => {}} />
                    <div onFocusCapture={() => setFocus('in')} onBlurCapture={() => setFocus('out')}>
                        <input key="inner" placeholder="inner" />
                    </div>
                    <form onSubmit={(e) => e.preventDefault()} onReset={() => setSaid('reset heard')}>
                        <input key="name" name="name" defaultValue="kept" />
                        <button key="reset" type="reset">Cancel</button>
                    </form>
                    <p>{said + ' focus ' + focus}</p>
                </div>
            );
        }
        "#,
    );
    hands.settle();
    let outline = hands.app.outline();
    assert!(!outline.contains("isn't something that happens"), "{outline}");
    assert!(outline.contains("nothing yet focus none"), "{outline}");
    hands.click("inner");
    hands.settle();
    assert!(hands.says("focus in"), "{}", hands.app.outline());
    hands.click("field");
    hands.settle();
    assert!(hands.says("focus out"), "{}", hands.app.outline());
    hands.click("reset");
    hands.settle();
    assert!(hands.says("reset heard"), "{}", hands.app.outline());
}

#[test]
fn an_inline_callback_ref_that_keeps_its_element_in_state_does_not_render_for_ever() {
    let mut hands = open(
        "callback-ref-state",
        r#"
        import { useState, useLayoutEffect } from 'react';
        let renders = 0;
        function Measured() {
            renders++;
            const [el, setEl] = useState(null);
            const [seen, setSeen] = useState('nothing');
            useLayoutEffect(() => { if (el) setSeen(el.tagName); }, [el]);
            return <div ref={(node) => setEl(node)}>{'saw ' + seen}</div>;
        }
        export default function App() {
            const [, redraw] = useState(0);
            return <div><Measured /><button key="count" onClick={() => redraw((n) => n + 1)}>{'renders ' + renders}</button></div>;
        }
        "#,
    );
    hands.settle();
    assert!(hands.says("saw DIV"), "{}", hands.app.outline());
    hands.click("count");
    hands.settle();
    // As React has it: the first render, the one for the element, and one for what it saw, and no more.
    assert!(hands.says("renders 3"), "{}", hands.app.outline());
}

#[test]
fn selector_queries_preserve_structure_state_and_identity() {
    let mut hands = open(
        "selector-regressions",
        r#"
        import { useRef, useState } from 'react';
        import { addCss } from 'rsui/web';
        addCss('#ask:hover { color: red; }');
        export default function App() {
            const [changed, setChanged] = useState(false);
            const [said, setSaid] = useState('');
            const kept = useRef(null);
            const mounted = useRef(null);
            const ask = () => {
                const errors = [];
                const q = (s) => [...document.querySelectorAll(s)].map((e) => e.getAttribute('id')).join(',');
                const check = (s, want) => {
                    let got;
                    try { got = q(s); } catch (error) { errors.push(s + ': ' + error); return; }
                    if (got !== want) errors.push(s + '=' + got + ' wanted ' + want);
                };
                const box = document.getElementById('mixed');
                const a = document.getElementById('a');
                if (kept.current !== null && kept.current !== a) errors.push('identity');
                kept.current = a;
                if (!changed) {
                    check('#mixed > :first-child', 'a');
                    check('#mixed > :last-child', 'e');
                    check('#mixed > :only-child', '');
                    check('#mixed > :first-of-type', 'a,b,d');
                    check('#mixed > :last-of-type', 'b,d,e');
                    check('#mixed > :only-of-type', 'b,d');
                    check('#mixed > :nth-child(odd)', 'a,c,e');
                    check('#mixed > :nth-child(even)', 'b,d');
                    check('#mixed > :nth-child(3n+2)', 'b,e');
                    check('#mixed > :nth-last-child(2n)', 'b,d');
                    check('#mixed > :nth-of-type(2)', 'c');
                    check('#mixed > :nth-last-of-type(2)', 'c');
                    check('#mixed > :nth-child(2 of .chosen)', 'c');
                    check('#mixed > :nth-last-child(2 of .chosen)', 'c');
                    check('#mixed > :not(:is(.chosen, :where(span)))', 'd');
                } else {
                    check('#mixed > *', 'e,a,x,b');
                    check('#mixed > :first-child', 'e');
                    check('#mixed > :last-child', 'b');
                    check('#mixed > :nth-of-type(2)', 'a');
                    check('#mixed > :nth-last-child(1 of .chosen)', 'a');
                }
                const scoped = [...box.querySelectorAll(':scope > .chosen')].map((e) => e.getAttribute('id')).join(',');
                if (scoped !== (changed ? 'e,a' : 'a,c,e')) errors.push('scope=' + scoped);
                check('#single > :only-child', 'solo');
                check('#relations:has(.leaf)', 'relations');
                check('#relations:has(> .branch > .leaf)', 'relations');
                check('#lead:has(+ .branch .leaf)', 'lead');
                check('#lead:has(~ .later > .leaf)', 'lead');
                check('#lead:has(+ .branch + .later > .leaf)', 'lead');
                check('#lead:has(+ .missing .leaf)', '');
                check('#relations:has(.missing)', '');
                if (document.querySelector('#mixed > .chosen')?.getAttribute('id') !== (changed ? 'e' : 'a')) errors.push('first');
                if (a.closest('#mixed')?.getAttribute('id') !== 'mixed' || !a.matches('#mixed > p.chosen')) errors.push('matches/closest');
                check('#tick:checked', changed ? 'tick' : '');
                check('#ask:hover', 'ask');
                check('#controls > :nth-child(1 of :hover)', 'ask');
                check('#controls > :nth-last-child(1 of :checked)', changed ? 'tick' : '');
                check('#mixed > :nth-child(1 of [data-rank])', changed ? 'a' : '');
                check('#mixed > :nth-last-child(1 of [data-rank])', changed ? 'a' : '');
                setSaid(errors.length ? errors.join('; ') : 'PASS ' + changed);
            };
            const order = changed ? ['e', 'a', 'x', 'b'] : ['a', 'b', 'c', 'd', 'e'];
            return <div>
                <div id="controls">
                    <button key="ask" id="ask" onClick={ask}>ask</button>
                    <button key="change" onClick={() => setChanged(true)}>change</button>
                    <input id="tick" type="checkbox" checked={changed} readOnly />
                </div>
                <div id="mixed">{order.map((id) => {
                    const Tag = id === 'b' ? 'span' : id === 'd' ? 'div' : 'p';
                    return <Tag key={id} id={id} ref={id === 'a' ? mounted : null} data-rank={id === 'a' && changed ? 'yes' : undefined} className={['a', 'c', 'e'].includes(id) ? 'chosen' : ''}>{id}</Tag>;
                })}</div>
                <div id="single"><span id="solo">solo</span></div>
                <div id="relations"><div id="lead" /><div className="branch"><span className="leaf" /></div><div className="later"><span className="leaf" /></div></div>
                <p>{said}</p>
            </div>;
        }
        "#,
    );
    hands.click("ask");
    assert!(hands.says("PASS false"), "{}", hands.app.outline());
    hands.click("change");
    hands.click("ask");
    assert!(hands.says("PASS true"), "{}", hands.app.outline());
}
#[test]
fn what_clips_what_it_holds_says_how_tall_that_is_as_its_scroll_height() {
    // Mantine's Collapse opens to its `scrollHeight`, read while it's still `height: 0; overflow: hidden`: an Accordion's panel
    // that had been closed opened to nothing.
    let mut hands = open(
        "collapsed-scroll-height",
        r#"
        import React, { useRef, useState } from 'react';
        export default function App() {
            const box = useRef(null);
            const [said, setSaid] = useState('');
            return (
                <div>
                    <div ref={box} style={{ height: 0, overflow: 'hidden', paddingBottom: 4 }}>
                        <div style={{ height: 50, margin: '0 0 6px' }}>Held</div>
                    </div>
                    <button key="measure" onClick={() => setSaid('heights ' + box.current.scrollHeight + ' ' + box.current.clientHeight)}>Measure</button>
                    <p>{said}</p>
                </div>
            );
        }
        "#,
    );
    hands.click("measure");
    assert!(hands.says("heights 60 4"), "{}", hands.app.outline());
}

#[test]
fn a_translate_worked_out_with_calc_moves_what_it_moves() {
    // Mantine's slider marks are `translate(calc(-50% + var(--slider-size) / 2), ...)`, in rems, and stayed where they were.
    let mut hands = open(
        "calc-translate",
        r#"
        import React, { useRef, useState } from 'react';
        export default function App() {
            const moved = useRef(null);
            const [said, setSaid] = useState('');
            return (
                <div style={{ paddingLeft: 100 }}>
                    <div ref={moved} key="moved" style={{ width: 40, height: 10, transform: 'translate(calc(-50% + calc(0.5rem * 1) / 2), calc(calc(0.625rem * 1) / 2))' }} />
                    <button key="measure" onClick={() => { const box = moved.current.getBoundingClientRect(); const around = moved.current.parentElement.getBoundingClientRect(); setSaid('at ' + (box.left - around.left) + ' ' + (box.top - around.top)); }}>Measure</button>
                    <p>{said}</p>
                </div>
            );
        }
        "#,
    );
    hands.click("measure");
    assert!(hands.says("at 84 5"), "{}", hands.app.outline());
}

#[test]
fn what_is_placed_by_itself_is_no_narrower_than_what_it_holds_can_be() {
    // Mantine's slider label is `position: absolute; white-space: nowrap` in a thumb whose inside is 8 pixels wide, and was laid
    // out 8 pixels wide, with no room for its number.
    let mut hands = open(
        "absolute-shrink-to-fit",
        r#"
        import React from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            return (
                <PageWindow><div style={{ paddingLeft: 100, paddingTop: 50 }}>
                    <div style={{ position: 'relative', width: 8, height: 8 }}>
                        <div key="label" style={{ position: 'absolute', top: -36, whiteSpace: 'nowrap', padding: 5, fontSize: 12 }}>40</div>
                        <div key="said" style={{ position: 'absolute', top: 20, width: 4 }}>A sentence that wraps</div>
                    </div>
                </div></PageWindow>
            );
        }
        "#,
    );
    let label = hands.rect_of("label");
    assert!(label[2] > 18.0, "as wide as its number and its padding: {label:?}");
    let said = hands.rect_of("said");
    assert!((said[2] - 4.0).abs() < 0.01, "what says how wide it is is that wide: {said:?}");
}

#[test]
fn what_lets_the_pointer_through_passes_a_click_to_what_is_under_it() {
    // Mantine's Select and MultiSelect: the chevron is in an absolutely placed section with `pointer-events: none` and
    // `z-index: 1` over the field's end. With a `z-index` about, what's on top is tried first, and the chevron took the
    // click, so clicking it never opened the dropdown. A left section, before the field, took it without one.
    let file = write(
        "pointer-through",
        r#"
        import './app.css';
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const [clicks, setClicks] = useState(0);
            const [lefts, setLefts] = useState(0);
            return (
                <PageWindow><div>
                    <div className="wrap">
                        <input key="field" readOnly value="Banana" onClick={() => setClicks(clicks + 1)} />
                        <div key="chevron" className="section right"><svg width="12" height="12"><path d="M0 0 L12 12" /></svg></div>
                    </div>
                    <div className="wrap">
                        <div key="icon" className="section left plain">o</div>
                        <input key="other" readOnly value="Search" onClick={() => setLefts(lefts + 1)} />
                    </div>
                    <p>{'clicks: ' + clicks + ', lefts: ' + lefts}</p>
                </div></PageWindow>
            );
        }
        "#,
    );
    std::fs::write(
        file.parent().unwrap().join("app.css"),
        r#"
        .wrap { position: relative; width: 200px; margin: 10px; }
        .wrap input { width: 200px; height: 36px; padding: 0 34px; box-sizing: border-box; }
        .section { position: absolute; top: 0; bottom: 0; width: 34px; display: flex; align-items: center; justify-content: center;
            pointer-events: none; z-index: 1; }
        .section.plain { z-index: auto; }
        .right { right: 0; }
        .left { left: 0; }
        "#,
    )
    .unwrap();
    let mut hands = Hands::open(&file, (500, 400));
    hands.settle();
    let (x, y) = hands.middle_of("chevron");
    hands.click_at(x, y);
    hands.settle();
    hands.settle();
    assert!(hands.says("clicks: 1"), "the chevron's click reaches the field: {}", hands.app.outline());
    let (x, y) = hands.middle_of("icon");
    hands.click_at(x, y);
    hands.settle();
    hands.settle();
    assert!(hands.says("lefts: 1"), "and so does a left section's: {}", hands.app.outline());
}

#[test]
fn a_logical_inset_moves_to_a_share_of_what_it_is_in() {
    // Mantine's Switch: the thumb is `inset-inline-start: var(--switch-thumb-start)` with `transition: inset-inline-start
    // 150ms ease`, and checked it goes to `calc(100% - thumb - padding)`. It jumped there, as `inset-inline-start` wasn't
    // something that moved, and a length with a share in it couldn't be partway from a number of pixels.
    let file = write(
        "logical-inset",
        r#"
        import './app.css';
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const [on, setOn] = useState(false);
            return (
                <PageWindow><div>
                    <button key="go" onClick={() => setOn(!on)}>go</button>
                    <div key="track" className="track">
                        <div key="thumb" className={on ? 'thumb on' : 'thumb'} />
                    </div>
                </div></PageWindow>
            );
        }
        "#,
    );
    std::fs::write(
        file.parent().unwrap().join("app.css"),
        r#"
        .track { position: relative; width: 100px; height: 20px; }
        .thumb { position: absolute; top: 0; width: 20px; height: 20px; inset-inline-start: var(--start, 2px);
            transition: inset-inline-start 300ms linear; }
        .thumb.on { --start: calc(100% - 22px); }
        "#,
    )
    .unwrap();
    let mut hands = Hands::open(&file, (500, 400));
    hands.app.set_animations(true);
    let now = std::time::Instant::now();
    hands.app.tick(now);
    hands.settle();
    let start = hands.rect_of("track")[0];
    let left = |hands: &mut Hands| hands.rect_of("thumb")[0] - start;
    assert!((left(&mut hands) - 2.0).abs() < 0.01, "{}", left(&mut hands));
    // Time goes on from when it's clicked, as that's when a transition starts.
    let toggle = |hands: &mut Hands| {
        let from = std::time::Instant::now();
        hands.app.tick(from);
        hands.click("go");
        hands.settle();
        let mut lefts = Vec::new();
        for step in 1..=30 {
            hands.app.tick(from + std::time::Duration::from_millis(step * 16));
            hands.settle();
            lefts.push(left(hands));
        }
        lefts
    };
    let there = toggle(&mut hands);
    assert!(there.iter().any(|&x| x > 10.0 && x < 70.0), "it moves partway: {there:?}");
    assert!(there.windows(2).all(|pair| pair[1] >= pair[0] - 0.01), "and only one way: {there:?}");
    assert!((there.last().unwrap() - 78.0).abs() < 0.01, "to the end: {there:?}");
    let back = toggle(&mut hands);
    assert!(back.iter().any(|&x| x > 10.0 && x < 70.0), "and back partway: {back:?}");
    assert!((back.last().unwrap() - 2.0).abs() < 0.01, "to the start: {back:?}");
}

/// The rect of each text input, in order.
fn input_rects(hands: &mut Hands) -> Vec<[f32; 4]> {
    fn find(node: &rsui::core::Node, path: &mut Vec<u32>, out: &mut Vec<Vec<u32>>) {
        if node.role == rsui::core::Role::TextInput {
            out.push(path.clone());
        }
        for (index, child) in node.children.iter().enumerate() {
            path.push(index as u32);
            find(child, path, out);
            path.pop();
        }
    }
    hands.settle();
    let mut paths = Vec::new();
    find(hands.app.runtime().root(), &mut Vec::new(), &mut paths);
    paths
        .iter()
        .map(|path| {
            let rect = hands.app.layout().rect(path).expect("it's been laid out");
            [rect.x, rect.y, rect.width, rect.height]
        })
        .collect()
}

#[test]
fn a_read_only_field_has_the_keyboard_and_copies_but_nothing_changes_it() {
    // Mantine's Select that isn't searchable is an `<input readOnly>`: typing in it did, as `readOnly` never got to the field.
    let file = write(
        "read-only-field",
        r#"
        import './app.css';
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const [changes, setChanges] = useState(0);
            const [typed, setTyped] = useState('');
            const changed = () => setChanges((was) => was + 1);
            return (
                <PageWindow><div>
                    <input className="field" readOnly defaultValue="Banana" onChange={changed} />
                    <textarea className="field" readOnly defaultValue="Two lines" onChange={changed} />
                    <input className="field" value={typed} onChange={(event) => setTyped(event.target.value)} />
                    <input type="checkbox" readOnly checked onChange={changed} />
                    <p>{'changes: ' + changes + ', typed: [' + typed + ']'}</p>
                </div></PageWindow>
            );
        }
        "#,
    );
    std::fs::write(
        file.parent().unwrap().join("app.css"),
        ".field { display: block; box-sizing: border-box; width: 150px; height: 30px; } .field:read-only { width: 120px; } .field:read-write { margin-left: 7px; }",
    )
    .unwrap();
    let mut hands = Hands::open(&file, (500, 400));
    hands.settle();
    let outline = hands.app.outline();
    assert_eq!(outline.matches("read_only=true").count(), 2, "only the two text fields are: {outline}");
    // `:read-only` and `:read-write` still match as they did.
    let rects = input_rects(&mut hands);
    assert_eq!(rects.len(), 3, "{rects:?}");
    assert_eq!((rects[0][2], rects[1][2], rects[2][2]), (120.0, 120.0, 150.0), "{rects:?}");
    assert!((rects[2][0] - rects[0][0] - 7.0).abs() < 0.01, "{rects:?}");
    let middle = |rect: [f32; 4]| (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    // Something to paste: what's typed in the field that can be changed, copied.
    let (x, y) = middle(rects[2]);
    hands.click_at(x, y);
    hands.type_text("hey");
    hands.press_with_ctrl('a');
    hands.press_with_ctrl('c');
    assert!(hands.says("typed: [hey]"), "{}", hands.app.outline());
    for (which, value) in [(0, "Banana"), (1, "Two lines")] {
        let (x, y) = middle(rects[which]);
        hands.click_at(x, y);
        assert!(hands.app.focused().is_some(), "it has the keyboard");
        hands.type_text("x");
        hands.press(KeyCode::Backspace);
        hands.press(KeyCode::Delete);
        hands.press(KeyCode::Enter);
        hands.press_with_ctrl('v');
        hands.app.composition("start", "").unwrap();
        hands.app.composition("update", "ni").unwrap();
        hands.app.composition("end", "\u{4F60}").unwrap();
        hands.type_text("\u{4F60}");
        hands.press_with_ctrl('a');
        hands.press_with_ctrl('x');
        hands.press_with_ctrl('z');
        hands.settle();
        let outline = hands.app.outline();
        assert!(outline.contains(&format!("value=\"{value}\"")), "{value} is as it was: {outline}");
        assert!(hands.says("changes: 0"), "{outline}");
    }
    // What's selected in it can still be copied: the textarea's all is selected from the last round.
    hands.press_with_ctrl('c');
    let (x, y) = middle(rects[2]);
    hands.click_at(x, y);
    hands.press_with_ctrl('a');
    hands.press_with_ctrl('v');
    assert!(hands.says("typed: [Two lines]"), "copied from what can't be changed: {}", hands.app.outline());
}

#[test]
fn margins_that_trade_places_keep_what_they_are_in_the_same_width() {
    // Mantine's Switch with labels: the track's label has `margin-inline-start` until it's checked, then `margin-inline-end`,
    // with `transition: margin 150ms`. The one that came or went jumped, as a margin nothing said wasn't anything to move from
    // or to, so the track was wider for a moment turning on, and narrower turning off.
    let file = write(
        "trading-margins",
        r#"
        import './app.css';
        import { useState } from 'react';
        import { PageWindow } from 'rsui/web';
        export default function App() {
            const [on, setOn] = useState(false);
            return (
                <PageWindow><div>
                    <button key="go" onClick={() => setOn(!on)}>go</button>
                    <div key="track" className="track">
                        <div key="label" className={on ? 'label on' : 'label'} />
                    </div>
                </div></PageWindow>
            );
        }
        "#,
    );
    std::fs::write(
        file.parent().unwrap().join("app.css"),
        r#"
        .track { display: flex; width: fit-content; height: 20px; }
        .label { width: 34px; height: 20px; margin-inline-start: 25px; transition: margin 150ms ease; }
        .label.on { margin-inline-end: 25px; margin-inline-start: 0; }
        "#,
    )
    .unwrap();
    let mut hands = Hands::open(&file, (500, 400));
    hands.app.set_animations(true);
    hands.app.tick(std::time::Instant::now());
    hands.settle();
    assert_eq!(hands.rect_of("track")[2], 59.0);
    let toggle = |hands: &mut Hands| {
        let from = std::time::Instant::now();
        hands.app.tick(from);
        hands.click("go");
        hands.settle();
        let mut seen = Vec::new();
        for step in 1..=14 {
            hands.app.tick(from + std::time::Duration::from_millis(step * 16));
            hands.settle();
            seen.push((hands.rect_of("track")[2], hands.rect_of("label")[0] - hands.rect_of("track")[0]));
        }
        seen
    };
    for (way, seen) in [("on", toggle(&mut hands)), ("off", toggle(&mut hands))] {
        assert!(seen.iter().all(|&(width, _)| (width - 59.0).abs() < 0.01), "turning {way}, the track stays as wide: {seen:?}");
        assert!(seen.iter().any(|&(_, left)| left > 3.0 && left < 22.0), "turning {way}, the label moves partway: {seen:?}");
    }
}
