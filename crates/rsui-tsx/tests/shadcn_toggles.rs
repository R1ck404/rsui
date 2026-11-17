//! What shadcn/ui's checkbox, radio group, switch, toggles and slider needed of rsui/web to behave as they do in a browser,
//! each as small as it comes.

mod hands;

use std::path::PathBuf;

use hands::Hands;
use hands::{KeyCode, KeyPress};
use rsui::PointerButton;

fn open(name: &str, css: &str, page: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-shadcn-toggles-{}-{name}", std::process::id()));
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

/// A page whose `<p id="log">` says what `note` was told, for the test to read with `says`.
const LOG: &str = r#"
const said = [];
function useLog() {
    const [, setCount] = React.useState(0);
    const note = React.useCallback((one) => { said.push(one); setCount(said.length); }, []);
    return [said, note];
}
"#;

#[test]
fn an_undefined_data_prop_is_no_attribute_and_an_svgs_shapes_go_with_it() {
    // Radix gives `data-disabled={disabled ? '' : undefined}`; its checkbox indicator's check icon is taken away when it's
    // unticked, and its path stayed in the page as a child of `<body>`.
    let mut hands = open(
        "undefined-data",
        "",
        &format!(
            r#"{LOG}
            function Page() {{
                const [on, setOn] = React.useState(true);
                const [log, note] = useLog();
                return (
                    <div>
                        <button id="b" data-disabled={{undefined}} data-off={{false}} style={{{{ width: 40, height: 40 }}}} onClick={{() => {{ setOn(!on); setTimeout(() => note('attr ' + document.getElementById('b').getAttribute('data-disabled') + ' ' + document.getElementById('b').getAttribute('data-off') + ' body ' + document.body.children.length)); }}}}>
                            {{on ? <svg width="24" height="24" viewBox="0 0 24 24"><path d="M20 6 9 17l-5-5" stroke="black" fill="none" /></svg> : null}}
                        </button>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(20.0, 20.0);
    hands.wait_for("log attr null false body 1");
}

#[test]
fn what_is_in_something_the_pointer_goes_through_is_gone_through_too() {
    // `pointer-events` is inherited: a click on the check icon in Radix's indicator (`pointer-events: none`) is on the box.
    let mut hands = open(
        "inherited-pointer-events",
        "",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                return (
                    <div ref={{(node) => {{ if (node && !window.__heard) {{ window.__heard = true; window.addEventListener('click', (event) => note(event.target.tagName), true); }} }}}}>
                        <button style={{{{ width: 40, height: 40, padding: 0 }}}}>
                            <span style={{{{ pointerEvents: 'none', display: 'block' }}}}><svg width="40" height="40" viewBox="0 0 24 24"><path d="M0 0h24v24H0z" /></svg></span>
                        </button>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(20.0, 20.0);
    hands.wait_for("log BUTTON");
}

#[test]
fn a_double_click_is_heard_on_the_way_down_and_a_labels_click_says_how_many() {
    let mut hands = open(
        "double-click",
        "label { display: block; width: 200px; height: 30px; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                React.useEffect(() => {{
                    const heard = (event) => note(event.type + ' ' + event.target.tagName + ' ' + event.detail);
                    window.addEventListener('dblclick', heard, true);
                    document.getElementById('box').addEventListener('click', heard);
                }}, []);
                return (
                    <div>
                        <label htmlFor="box">Accept</label>
                        <button id="box">Box</button>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.app.pointer_move(50.0, 15.0);
    hands.app.pointer_down(50.0, 15.0, 1);
    let _ = hands.app.pointer_up(50.0, 15.0);
    hands.app.pointer_down(50.0, 15.0, 2);
    let _ = hands.app.pointer_up(50.0, 15.0);
    hands.settle();
    hands.wait_for("log click BUTTON 1, click BUTTON 2, dblclick LABEL 2");
}

#[test]
fn a_line_height_of_a_unitless_calc_is_times_the_font_and_is_what_a_heading_has() {
    // Tailwind's `text-2xl` is `line-height: var(--tw-leading, var(--text-2xl--line-height))` with `calc(2 / 1.5)`, in a page
    // whose `html` says `line-height: 1.5`, which the heading's text had instead.
    let mut hands = open(
        "line-height-calc",
        r#"html { line-height: 1.5; }
        :root { --text-2xl--line-height: calc(2 / 1.5); }
        h1, h2 { margin: 0; }
        .a { font-size: 24px; line-height: var(--tw-leading, var(--text-2xl--line-height)); }
        .b { font-size: 18px; line-height: 28px; }"#,
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                const look = () => note([...document.querySelectorAll('h1, h2')].map((one) => Math.round(one.getBoundingClientRect().height)).join(' '));
                return <main onClick={{look}}><h1 className="a">Checkbox</h1><section><h2 className="b">Demo</h2><p>{{'log ' + log.join(', ')}}</p></section></main>;
            }}"#
        ),
    );
    hands.click_at(20.0, 10.0);
    hands.wait_for("log 32 28");
}

#[test]
fn tab_goes_through_buttons_the_page_and_back_but_not_labels() {
    // A `<button>` with nothing listening can have the keyboard, and `document.activeElement` says so; a label that clicks its
    // box can't; past the last, the keyboard goes to the page (`body`), then round; `relatedTarget` says where it went.
    let mut hands = open(
        "tab-order",
        "",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                React.useEffect(() => {{
                    window.addEventListener('focus', (event) => note('focus ' + event.target.id + ' from ' + (event.relatedTarget?.id ?? '-')), true);
                    window.addEventListener('keyup', () => note('on ' + (document.activeElement.id || document.activeElement.tagName)), true);
                }}, []);
                return (
                    <div>
                        <button id="a">A</button>
                        <label htmlFor="b" onClick={{() => {{}}}}>Label</label>
                        <button id="b">B</button>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    for _ in 0..4 {
        hands.press_key(KeyPress::new(KeyCode::Tab));
    }
    // Coming back in from past the last, the window hears `focus` first, as in Edge.
    hands.wait_for("log focus a from -, on a, focus b from a, on b, on BODY, focus undefined from -, focus a from -, on a");
}

#[test]
fn space_clicks_a_button_as_it_comes_up_and_enter_as_it_goes_down() {
    let mut hands = open(
        "space-on-keyup",
        "",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                return (
                    <div>
                        <button id="a" onKeyDown={{(event) => note('down ' + event.key)}} onKeyUp={{(event) => note('up ' + event.key)}} onClick={{() => note('click')}}>A</button>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.press_key(KeyPress::new(KeyCode::Tab));
    hands.press_key(KeyPress::new(KeyCode::Space));
    hands.press_key(KeyPress::new(KeyCode::Enter));
    hands.wait_for("log up Tab, down , up , click, down Enter, click, up Enter");
}

#[test]
fn has_about_what_a_component_makes_is_known_once_its_drawn() {
    // shadcn's `Field` is `has-[>[data-slot=field-content]]:items-start`, and the `<div data-slot="field-content">` is what
    // its `FieldContent` component makes; a card label is `has-data-[state=checked]:...`, which changes as the box in it does.
    let mut hands = open(
        "has-component",
        r#".row { display: flex; align-items: center; height: 60px; }
        .row:has(> [data-slot=content]) { align-items: flex-start; }
        .card:has([data-state=checked]) { color: red; }"#,
        &format!(
            r#"{LOG}
            function Content(props) {{ return <span data-slot="content" {{...props}} />; }}
            function Box() {{
                const [on, setOn] = React.useState(false);
                return <button id="box" style={{{{ display: 'block', width: 60, height: 30 }}}} data-state={{on ? 'checked' : 'unchecked'}} onClick={{() => setOn(!on)}}>box</button>;
            }}
            function Page() {{
                const [log, note] = useLog();
                const look = () => note(Math.round(document.getElementById('c').getBoundingClientRect().y - document.getElementById('row').getBoundingClientRect().y) + ' ' + getComputedStyle(document.getElementById('card')).color);
                return (
                    <div>
                        <div id="card" className="card"><Box /></div>
                        <div id="row" className="row"><Content id="c">c</Content></div>
                        <button id="read" style={{{{ display: 'block', width: 60, height: 30 }}}} onClick={{look}}>read</button>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    // The row's content is at its top, and once the box in the card is ticked, the card is red, and then not. It's read once the
    // page has been drawn again, with a click of its own: what a component makes is only known to `:has()` once it's been laid
    // out (see `hasDrawn` in web.js), so a timer that happened to go off before that, as a 50 ms one did on a busy machine,
    // read the colour from before.
    hands.click_at(30.0, 15.0);
    hands.settle();
    hands.click_at(30.0, 105.0);
    hands.wait_for("log 0 rgb(255, 0, 0)");
    hands.click_at(30.0, 15.0);
    hands.settle();
    hands.click_at(30.0, 105.0);
    hands.wait_for("log 0 rgb(255, 0, 0), 0 rgb(0, 0, 0)");
}

#[test]
fn a_flex_row_goes_from_the_right_where_dir_says_rtl() {
    let mut hands = open(
        "rtl-row",
        ".row { display: flex; width: 300px; } .row > span { width: 50px; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                const look = () => note([...document.querySelectorAll('.row > span')].map((one) => Math.round(one.getBoundingClientRect().x)).join(' '));
                return <div onClick={{look}}><div className="row" dir="rtl"><span>a</span><span>b</span></div><p>{{'log ' + log.join(', ')}}</p></div>;
            }}"#
        ),
    );
    hands.click_at(280.0, 10.0);
    hands.wait_for("log 250 200");
}

#[test]
fn a_right_click_is_an_auxclick_then_a_contextmenu_and_a_disabled_button_hears_no_mouse_clicks() {
    let mut hands = open(
        "aux-and-disabled",
        "button { width: 60px; height: 30px; display: block; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                React.useEffect(() => {{
                    for (const type of ['pointerdown', 'mousedown', 'mouseup', 'click', 'auxclick', 'contextmenu']) {{
                        window.addEventListener(type, (event) => note(type + ' ' + event.target.id), true);
                    }}
                }}, []);
                return <div><button id="on">On</button><button id="off" disabled>Off</button><p>{{'log ' + log.join(', ')}}</p></div>;
            }}"#
        ),
    );
    hands.app.pointer_move(30.0, 15.0);
    hands.app.button_down(30.0, 15.0, PointerButton::Secondary);
    let _ = hands.app.button_up(30.0, 15.0, PointerButton::Secondary);
    hands.settle();
    hands.click_at(30.0, 45.0);
    hands.wait_for("log pointerdown on, mousedown on, mouseup on, auxclick on, contextmenu on, pointerdown off");
    assert!(!hands.says("click off"), "{}", hands.app.outline());
}

#[test]
fn a_timer_a_key_handler_sets_waits_until_the_key_has_been_everywhere() {
    // Radix's roving focus moves the keyboard in a `setTimeout` from `onKeyDown`, and its radio group checks what it moved to
    // only while `document` has heard the arrow go down: the timer ran before `document` heard the key.
    let mut hands = open(
        "timer-after-dispatch",
        "button { width: 60px; height: 30px; display: block; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                React.useEffect(() => {{
                    document.addEventListener('keydown', (event) => note('document ' + event.key));
                }}, []);
                return (
                    <div>
                        <button id="a" onKeyDown={{(event) => {{ setTimeout(() => note('timer')); Promise.resolve().then(() => note('microtask')); }}}}>A</button>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(30.0, 15.0);
    hands.press_key(KeyPress::new(KeyCode::Down));
    hands.wait_for("log microtask, document ArrowDown, timer");
}

#[test]
fn a_press_on_a_disabled_control_gives_the_keyboard_to_what_its_in_and_isnt_heard_as_the_mouses() {
    // Radix's radio item calls `preventDefault()` in `onMouseDown` when it can't have the keyboard, which kept the keyboard
    // from going to its group: a browser sends a disabled control no `mousedown` at all, to React's props either.
    let mut hands = open(
        "disabled-press",
        "button { width: 60px; height: 30px; display: block; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                return (
                    <div>
                        <div id="group" tabIndex={{0}} onFocus={{(event) => note('focus ' + event.target.id)}} onMouseDown={{() => note('group mousedown')}} onPointerDown={{() => note('group pointerdown')}}>
                            <button id="off" disabled onMouseDown={{(event) => {{ note('off mousedown'); event.preventDefault(); }}}}>Off</button>
                        </div>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(30.0, 15.0);
    hands.wait_for("log group pointerdown, focus group");
    assert!(!hands.says("mousedown"), "{}", hands.app.outline());
}

#[test]
fn text_content_is_what_react_gave_whatever_text_transform_shows() {
    let mut hands = open(
        "text-content-case",
        ".cap { text-transform: capitalize; } .up { text-transform: uppercase; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                const look = () => {{
                    document.createTextNode('');
                    note(document.getElementById('h').textContent + ' / ' + document.getElementById('u').textContent + ' / ' + document.body.textContent.slice(0, 11));
                }};
                return (
                    <div onClick={{look}} style={{{{ height: 200 }}}}>
                        <h1 id="h" className="cap">radio group</h1>
                        <p className="up"><span id="u">quiet</span></p>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    assert!(hands.says("Radio Group"), "{}", hands.app.outline());
    hands.click_at(200.0, 150.0);
    hands.wait_for("log radio group / quiet / radio group");
}

#[test]
fn what_isnt_stretched_across_a_column_sits_at_the_right_where_dir_says_rtl() {
    // A `w-fit` label in a `flex-col` (shadcn's `FieldContent`) of a right-to-left radio group, and a block's narrower child.
    let mut hands = open(
        "rtl-column",
        ".col { display: flex; flex-direction: column; width: 300px; } .col > span { width: 50px; } .blk { width: 300px; } .blk > div { width: 60px; height: 10px; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                const look = () => note(['#c', '#k', '#l'].map((one) => Math.round(document.querySelector(one).getBoundingClientRect().x)).join(' '));
                return (
                    <div onClick={{look}} style={{{{ height: 200 }}}}>
                        <div className="col" dir="rtl"><span id="c">a</span></div>
                        <div className="blk" dir="rtl"><div id="k" /></div>
                        <div className="col"><span id="l">a</span></div>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(350.0, 150.0);
    hands.wait_for("log 250 240 0");
}

#[test]
fn what_a_keys_handlers_change_is_there_before_the_key_does_what_it_does() {
    // Radix's radio group takes itself out of the tab order as Shift+Tab goes down on one of its items, so the keyboard goes to
    // what's before it rather than to the group (which gave it straight back).
    let mut hands = open(
        "shift-tab-state",
        "button { width: 60px; height: 30px; display: block; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                const [out, setOut] = React.useState(false);
                return (
                    <div>
                        <button id="a" onFocus={{() => note('focus a')}}>A</button>
                        <div id="g" tabIndex={{out ? -1 : 0}} onFocus={{(event) => event.target.id === 'g' && note('focus g')}}>
                            <button id="b" onKeyDown={{(event) => {{ if (event.key === 'Tab' && event.shiftKey) setOut(true); }}}}>B</button>
                        </div>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(30.0, 45.0);
    hands.press_key(KeyPress::new(KeyCode::Tab).shift());
    hands.wait_for("log focus a");
    assert!(!hands.says("focus g"), "{}", hands.app.outline());
}

#[test]
fn a_fieldsets_gap_isnt_between_its_legend_and_the_rest() {
    // Chrome lays out a fieldset's rendered legend by itself, and what else is in it in a box of its own that its `display: flex`
    // and `gap` are about.
    let mut hands = open(
        "legend-gap",
        "fieldset { display: flex; flex-direction: column; gap: 20px; border: 0; padding: 0; margin: 0; } legend { padding: 0; } p { margin: 0; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                const look = () => {{
                    const at = (id) => document.getElementById(id).getBoundingClientRect();
                    note(Math.round(at('one').y - at('legend').bottom) + ' ' + Math.round(at('two').y - at('one').bottom));
                }};
                return (
                    <div onClick={{look}} style={{{{ height: 200 }}}}>
                        <fieldset><legend id="legend">Plan</legend><p id="one">One</p><p id="two">Two</p></fieldset>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(200.0, 150.0);
    hands.wait_for("log 0 20");
}

#[test]
fn a_label_doesnt_click_its_control_again_when_the_click_was_on_it() {
    // shadcn's switch card is `<label for="s">` with the switch inside it: a click on the switch toggled it twice.
    let mut hands = open(
        "label-own-control",
        "button { width: 60px; height: 30px; display: block; } label { display: block; padding: 10px; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                return (
                    <div>
                        <label htmlFor="s"><span>Card</span><button id="s" onClick={{() => note('click')}}>S</button></label>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    // The button's at 10..70 across and 29..59 down, under the label's text.
    let (x, y) = hands.middle_of_role(rsui::core::Role::Button);
    hands.click_at(x, y);
    hands.click_at(30.0, 15.0);
    hands.wait_for("log click, click");
    assert!(!hands.says("click, click, click"), "{}", hands.app.outline());
}

#[test]
fn words_beside_an_icon_are_no_element_an_event_happens_to() {
    // shadcn's Toggle is `<button><svg/> Bookmark</button>`: `Children.toArray` gives the words a key, and a click on them said
    // it was on a `<span>` that isn't in the page.
    let mut hands = open(
        "keyed-text-target",
        "button { display: flex; gap: 8px; padding: 0; width: 200px; height: 40px; align-items: center; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                React.useEffect(() => {{ window.addEventListener('pointerdown', (event) => note(event.target.tagName), true); }}, []);
                return (
                    <div>
                        <button><svg width="16" height="16" viewBox="0 0 16 16"><path d="M0 0h16v16H0z" /></svg>Bookmark</button>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(100.0, 20.0);
    hands.wait_for("log BUTTON");
}

#[test]
fn whats_in_a_provider_is_among_the_children_of_what_its_in() {
    // shadcn's ToggleGroup gives its items inside a context provider of its own: each was `:first-child` and `:last-child`.
    let mut hands = open(
        "nth-through-provider",
        ".row { display: flex; } .row > i { display: block; width: 20px; height: 20px; } .row > i:first-child { margin-top: 3px; } .row > i:last-child { margin-top: 7px; }",
        &format!(
            r#"{LOG}
            const Ctx = React.createContext(null);
            function Item(props) {{ return <i {{...props}} />; }}
            function Page() {{
                const [log, note] = useLog();
                const look = () => note([...document.querySelectorAll('.row > i')].map((one) => getComputedStyle(one).marginTop).join(' '));
                return (
                    <div onClick={{look}} style={{{{ height: 200 }}}}>
                        <div className="row"><Ctx.Provider value={{1}}><Item /><Item /><><Item /></></Ctx.Provider></div>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(200.0, 150.0);
    hands.wait_for("log 3px 0px 7px");
}

#[test]
fn tab_from_what_isnt_a_stop_goes_on_from_where_it_is() {
    // A Radix group whose items are all disabled has the keyboard after a click (`tabindex="-1"`), and Tab from it went nowhere.
    let mut hands = open(
        "tab-from-not-a-stop",
        "button, div[tabindex] { width: 60px; height: 30px; display: block; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                React.useEffect(() => {{
                    window.addEventListener('keyup', () => note('on ' + (document.activeElement.id || document.activeElement.tagName)), true);
                }}, []);
                return (
                    <div>
                        <button id="a">A</button>
                        <div id="g" tabIndex={{-1}}>G</div>
                        <button id="b">B</button>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(30.0, 45.0);
    hands.press_key(KeyPress::new(KeyCode::Tab));
    hands.click_at(30.0, 45.0);
    hands.press_key(KeyPress::new(KeyCode::Tab).shift());
    hands.press_key(KeyPress::new(KeyCode::Tab).shift());
    hands.wait_for("log on b, on a, on BODY");
}

#[test]
fn an_element_with_nothing_in_it_has_no_text_whatever_its_aria_label() {
    // Radix's slider thumb is `<span role="slider" aria-label="Minimum">` with nothing in it.
    let mut hands = open(
        "aria-label-text",
        "span { display: block; width: 16px; height: 16px; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                const look = () => note('[' + document.getElementById('t').textContent + '] [' + document.getElementById('w').textContent + ']');
                return (
                    <div onClick={{look}} style={{{{ height: 200 }}}}>
                        <div id="w"><span id="t" role="slider" aria-label="Minimum" tabIndex={{0}} /><span aria-label="Max">x</span></div>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(200.0, 150.0);
    hands.wait_for("log [] [x]");
}

#[test]
fn a_word_wider_than_its_centred_column_is_as_wide_as_it_is() {
    // shadcn's font weight toggles: "Medium" in a 38px column with `items-center` is wider than the column, centred over it.
    let mut hands = open(
        "word-past-column",
        ".col { display: flex; flex-direction: column; align-items: center; width: 20px; margin-left: 100px; font-size: 16px; } .col > span { white-space: nowrap; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                const look = () => {{
                    const one = document.getElementById('w').getBoundingClientRect();
                    const col = document.querySelector('.col').getBoundingClientRect();
                    note((one.width > 30 ? 'wide' : 'narrow ' + one.width) + ' ' + Math.round(one.x + one.width / 2 - (col.x + col.width / 2)));
                }};
                return (
                    <div onClick={{look}} style={{{{ height: 200 }}}}>
                        <div className="col"><span id="w">Medium</span></div>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(300.0, 150.0);
    hands.wait_for("log wide 0");
}

#[test]
fn an_svgs_shapes_are_shown_with_it() {
    let mut hands = open(
        "svg-shapes-shown",
        "",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                const shown = (selector) => (getComputedStyle(document.querySelector(selector)).display === 'none' ? 'hidden' : 'shown');
                const look = () => note(shown('circle') + ' ' + shown('#gone'));
                return (
                    <div onClick={{look}} style={{{{ height: 200 }}}}>
                        <svg width="24" height="24" viewBox="0 0 24 24"><circle cx="12" cy="12" r="6" /></svg>
                        <div id="gone" style={{{{ display: 'none' }}}}>x</div>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(300.0, 150.0);
    hands.wait_for("log shown hidden");
}
