//! What shadcn/ui's sonner, carousel, resizable, sidebar and chart needed of rsui/web to behave as they do in a browser,
//! each as small as it comes.

mod hands;

use std::path::PathBuf;

use hands::Hands;
use hands::{KeyCode, KeyPress};

fn open(name: &str, css: &str, page: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-shadcn-rich-{}-{name}", std::process::id()));
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

/// A page whose `<p>` says what `note` was told, for the test to read with `wait_for`.
const LOG: &str = r#"
const said = [];
function useLog() {
    const [, setCount] = React.useState(0);
    const note = React.useCallback((one) => { said.push(one); setCount(said.length); }, []);
    return [said, note];
}
"#;

#[test]
fn what_is_placed_by_itself_outside_a_box_with_nothing_in_its_flow_takes_the_pointer() {
    // sonner's toasts are `position: absolute; bottom: 0` in a fixed `<ol>` that's 0px tall, so they stand out of it, and
    // the front one is first but on top by its z-index.
    let mut hands = open(
        "outside-its-box",
        "",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                return (
                    <div>
                        <p>{{'log ' + log.join(', ')}}</p>
                        <ol style={{{{ position: 'fixed', top: 150, left: 100, width: 200, margin: 0, padding: 0, listStyle: 'none' }}}}>
                            <li onClick={{() => note('front')}} style={{{{ position: 'absolute', bottom: 0, right: 0, width: 200, height: 50, zIndex: 2, background: '#ddd' }}}}>front</li>
                            <li onClick={{() => note('back')}} style={{{{ position: 'absolute', bottom: 0, right: 0, width: 200, height: 50, zIndex: 1, background: '#aaa' }}}}>back</li>
                        </ol>
                    </div>
                );
            }}"#
        ),
    );
    // The list is 0px tall at 100, 150: the toasts stand above it, at 100..300, 100..150.
    hands.click_at(200.0, 125.0);
    hands.wait_for("log front");
}

#[test]
fn what_is_inside_is_styled_again_when_the_attributes_it_is_in_change() {
    // sonner's `[data-expanded='false'][data-front='false'] > * { opacity: 0 }` kept its toasts' contents hidden once the
    // `li` said `data-expanded="true"`.
    let mut hands = open(
        "parent-attributes",
        "li[data-x='false'] > * { opacity: 0; }\nli[data-y='false'] .c { opacity: 0; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [open, setOpen] = React.useState(false);
                const [log, note] = useLog();
                React.useEffect(() => {{
                    if (open) setTimeout(() => note(['a', 'b'].map((id) => getComputedStyle(document.querySelector('#' + id + ' .c')).opacity).join(' ')));
                }}, [open]);
                return (
                    <div>
                        <button id="go" onClick={{() => setOpen(true)}} style={{{{ width: 100, height: 30 }}}}>go</button>
                        <ol>
                            <li id="a" data-x={{open ? 'true' : 'false'}}><div className="c">a</div></li>
                            <li id="b" data-y={{open ? 'true' : 'false'}}><div className="c">b</div></li>
                        </ol>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(20.0, 15.0);
    hands.wait_for("log 1 1");
}

#[test]
fn what_is_said_after_a_variable_goes_over_it_found_by_an_attribute() {
    // sonner's `[data-sonner-toaster] { width: var(--width) }`, then `@media (max-width: 600px) { [data-sonner-toaster] {
    // width: 100% } }`: the later rule goes over the variable.
    let mut hands = open(
        "variable-then-later",
        "[data-t] { width: var(--width); }\n[data-t] { width: 50%; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                React.useEffect(() => {{ setTimeout(() => note(String(document.getElementById('t').getBoundingClientRect().width))); }}, []);
                return (
                    <div>
                        <div id="t" data-t="" style={{{{ '--width': '356px', height: 10 }}}}></div>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.wait_for("log 200");
}

#[test]
fn where_things_are_stays_known_after_a_render_that_changes_nothing() {
    // sonner's Alt+T sets its toaster expanded with no toasts, which renders nothing new; every element found since said it was
    // at 0, 0, 0 by 0.
    let mut hands = open(
        "same-render",
        "",
        &format!(
            r#"{LOG}
            function Quiet() {{
                const [expanded, setExpanded] = React.useState(false);
                React.useEffect(() => {{
                    const on = () => setExpanded(true);
                    document.addEventListener('keydown', on);
                    return () => document.removeEventListener('keydown', on);
                }}, []);
                return <section aria-live="polite">{{null}}</section>;
            }}
            function Page() {{
                const [log, note] = useLog();
                React.useEffect(() => {{
                    const on = () => setTimeout(() => {{ const b = document.getElementById('b').getBoundingClientRect(); note(b.width + 'x' + b.height); }}, 50);
                    document.addEventListener('keyup', on);
                    return () => document.removeEventListener('keyup', on);
                }}, []);
                return (
                    <div>
                        <button id="b" style={{{{ width: 80, height: 30 }}}}>b</button>
                        <Quiet />
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.press_key(KeyPress::new(KeyCode::Character('t')));
    hands.wait_for("log 80x30");
}

#[test]
fn escape_that_nothing_takes_leaves_a_pages_focus_where_it_is() {
    // sonner's toaster, focused by Alt+T, collapses on Escape and keeps the focus; rsui let go of it.
    let mut hands = open(
        "escape-focus",
        "",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                React.useEffect(() => {{
                    const on = (event) => {{ if (event.key === 'Escape') setTimeout(() => note('active ' + (document.activeElement?.id || document.activeElement?.tagName)), 50); }};
                    document.addEventListener('keydown', on);
                    return () => document.removeEventListener('keydown', on);
                }}, []);
                return (
                    <div>
                        <ol id="list" tabIndex={{-1}} style={{{{ width: 100, height: 30, margin: 0 }}}}>list</ol>
                        <button id="b" onClick={{() => document.getElementById('list').focus()}} style={{{{ width: 80, height: 30 }}}}>b</button>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(40.0, 45.0);
    hands.press(KeyCode::Escape);
    hands.wait_for("log active list");
}

#[test]
fn taking_the_pointer_says_so_and_letting_go_says_so() {
    // sonner's toast takes the pointer as a swipe starts: `gotpointercapture` before its next event and `lostpointercapture`
    // after it comes up, before the click.
    let mut hands = open(
        "pointer-capture",
        "",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                return (
                    <div>
                        <div id="d"
                            onPointerDown={{(event) => event.currentTarget.setPointerCapture(event.pointerId)}}
                            onGotPointerCapture={{() => note('got')}}
                            onLostPointerCapture={{() => note('lost')}}
                            onPointerUp={{() => note('up')}}
                            onClick={{() => note('click')}}
                            style={{{{ width: 100, height: 50, background: '#ddd' }}}}>drag</div>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.down_at(50.0, 25.0);
    hands.settle();
    hands.move_at(60.0, 25.0);
    hands.up_at(60.0, 25.0);
    hands.wait_for("log got, up, lost, click");
}

#[test]
fn what_overflows_a_moved_track_takes_the_pointer_where_it_is_drawn() {
    // Embla's track is as wide as one slide and moved along by `translate3d()`; the slides after the first overflow it, and a
    // click on the second, where it's drawn, went nowhere.
    let mut hands = open(
        "overflowing-track",
        "",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                const [moved, setMoved] = React.useState(false);
                return (
                    <div>
                        <button onClick={{() => setMoved(true)}} style={{{{ width: 60, height: 20 }}}}>go</button>
                        <div style={{{{ overflow: 'hidden', width: 100, marginLeft: 50 }}}}>
                            <div style={{{{ display: 'flex', transform: moved ? 'translate3d(-100px, 0px, 0px)' : undefined }}}}>
                                {{[1, 2, 3].map((i) => <div key={{i}} id={{'s' + i}} onClick={{() => note('s' + i + ' ' + document.elementFromPoint(100, 45)?.id)}} style={{{{ minWidth: 100, height: 50 }}}}>{{i}}</div>)}}
                            </div>
                        </div>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(30.0, 10.0);
    hands.click_at(100.0, 45.0);
    hands.wait_for("log s2 s2");
}

#[test]
fn a_flex_basis_worked_out_with_calc_is_a_share() {
    // Tailwind 4's `basis-1/2` is `flex-basis: calc(1/2 * 100%)` (shadcn's carousel-size).
    let mut hands = open(
        "calc-basis",
        ".half { flex: 0 0 calc(1/2 * 100%); min-width: 0; height: 10px; }",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                React.useEffect(() => {{ setTimeout(() => note(String(document.getElementById('h').getBoundingClientRect().width))); }}, []);
                return (
                    <div>
                        <div style={{{{ display: 'flex', width: 300 }}}}><div id="h" className="half"></div><div className="half"></div></div>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.wait_for("log 150");
}

#[test]
fn a_focused_control_that_is_disabled_is_no_longer_the_active_element() {
    // Embla's Next button is disabled at the last slide while it has the focus: a browser's `activeElement` is `body` then.
    let mut hands = open(
        "disabled-focus",
        "",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                const [off, setOff] = React.useState(false);
                React.useEffect(() => {{ if (off) setTimeout(() => note('active ' + document.activeElement.tagName)); }}, [off]);
                return (
                    <div>
                        <button id="b" disabled={{off}} onClick={{() => setOff(true)}} style={{{{ width: 80, height: 30 }}}}>b</button>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.click_at(40.0, 15.0);
    hands.wait_for("log active BODY");
}

#[test]
fn aria_attributes_are_properties_too() {
    // react-resizable-panels takes a separator whose `ariaDisabled` isn't `null` to be disabled; rsui's said `undefined`, so no
    // separator could be dragged.
    let mut hands = open(
        "aria-reflection",
        "",
        &format!(
            r#"{LOG}
            function Page() {{
                const [log, note] = useLog();
                React.useEffect(() => {{
                    setTimeout(() => {{
                        const a = document.getElementById('a');
                        const before = String(a.ariaDisabled) + ' ' + a.ariaValueNow + ' ' + a.ariaOrientation;
                        a.ariaLabel = 'named';
                        note(before + ' ' + a.getAttribute('aria-label'));
                    }});
                }}, []);
                return (
                    <div>
                        <div id="a" role="separator" aria-valuenow={{25}} aria-orientation="vertical" style={{{{ width: 10, height: 10 }}}}></div>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.wait_for("log null 25 vertical named");
}

#[test]
fn what_is_fixed_inside_something_transformed_is_placed_in_it_but_not_from_a_portal() {
    // shadcn's sidebar blocks are `position: fixed` in frames with `transform: translateZ(0)`, which CSS makes the box they're
    // placed in; a Sheet portaled into `body` from there is still placed in the window.
    let mut hands = open(
        "fixed-in-transform",
        "",
        &format!(
            r#"{LOG}
            import {{ createPortal }} from 'react-dom';
            function Page() {{
                const [log, note] = useLog();
                React.useEffect(() => {{
                    setTimeout(() => {{
                        const a = document.getElementById('in').getBoundingClientRect();
                        const b = document.getElementById('out').getBoundingClientRect();
                        note(a.left + ',' + a.top + ' ' + b.left + ',' + b.top);
                    }}, 50);
                }}, []);
                return (
                    <div>
                        <div style={{{{ position: 'relative', transform: 'translateZ(0)', marginLeft: 50, marginTop: 50, width: 200, height: 100 }}}}>
                            <div id="in" style={{{{ position: 'fixed', top: 10, left: 10, width: 20, height: 20 }}}}></div>
                            {{createPortal(<div id="out" style={{{{ position: 'fixed', top: 10, left: 10, width: 20, height: 20 }}}}></div>, document.body)}}
                        </div>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.wait_for("log 60,60 10,10");
}

#[test]
fn a_ref_on_a_part_of_a_picture_is_given_its_element_before_layout_effects() {
    // recharts registers its z-index layers from `<g ref>`s in a layout effect and draws no bar until they're there.
    let mut hands = open(
        "svg-part-ref",
        "",
        &format!(
            r#"{LOG}
            function Layer({{ note }}) {{
                const ref = React.useRef(null);
                React.useLayoutEffect(() => {{ note('layout ' + (ref.current ? ref.current.tagName : 'null')); }}, []);
                return <g ref={{ref}} className="layer" />;
            }}
            function Page() {{
                const [log, note] = useLog();
                const later = React.useCallback((one) => setTimeout(() => note(one)), [note]);
                return (
                    <div>
                        <svg width="40" height="40"><Layer note={{later}} /></svg>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.wait_for("log layout G");
}

#[test]
fn a_ref_on_a_part_of_a_picture_keeps_its_element_when_something_is_put_in_before_it() {
    // Mantine's `Tooltip.Floating` round each of RingProgress's circles puts its own box in before the circle once its portal's
    // mounted, so the circle is a new element of the picture, and the ref, let go of by the old one after the new one had it, was
    // null when its effect listened on it.
    let mut hands = open(
        "svg-part-ref-moved",
        "",
        &format!(
            r#"{LOG}
            function Shape({{ note }}) {{
                const ref = React.useRef(null);
                const [more, setMore] = React.useState(false);
                React.useEffect(() => {{ if (!more) {{ setMore(true); }} }}, [more]);
                React.useEffect(() => {{ if (more) {{ note('effect ' + (ref.current ? ref.current.tagName : 'null')); }} }}, [more]);
                return <>{{more ? <rect width="4" height="4" /> : null}}<circle ref={{ref}} cx="5" cy="5" r="4" /></>;
            }}
            function Page() {{
                const [log, note] = useLog();
                const later = React.useCallback((one) => setTimeout(() => note(one)), [note]);
                return (
                    <div>
                        <svg width="40" height="40"><Shape note={{later}} /></svg>
                        <p>{{'log ' + log.join(', ')}}</p>
                    </div>
                );
            }}"#
        ),
    );
    hands.wait_for("log effect CIRCLE");
}
