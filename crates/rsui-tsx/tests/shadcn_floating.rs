//! What shadcn/ui's floating overlays (popover, tooltip, hover card, dropdown menu) needed of
//! rsui/web, each as small as it comes: Floating UI measures from the window only when the wrapper says it's fixed, Tailwind 4's
//! line heights are numbers worked out by `calc()`, and an attribute React is given as `undefined` isn't there.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn open(name: &str, css: &str, before: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-shadcn-floating-{}-{name}", std::process::id()));
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
fn a_fixed_box_says_its_fixed_and_where_it_is() {
    // Radix's popper wrapper is `position: fixed; left: 0; top: 0; transform: translate(x, y)`, inside a `position: relative` box.
    // Floating UI's `getOffsetParent` asks its computed `position`: anything but `fixed` has it measure the trigger from that
    // box, and the popover is put that far up and to the left of where it should be.
    let hands = open(
        "fixed",
        "",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            const wrapper = React.useRef(null);
            const filled = React.useRef(null);
            React.useLayoutEffect(() => {
                const style = getComputedStyle(wrapper.current);
                const fill = getComputedStyle(filled.current);
                setSaid(['wrapper', style.position, style.left, style.top, 'fill', fill.position, fill.left, fill.top, fill.right, fill.bottom].join(' '));
            }, []);
            return (
                <dom.div style={{ position: 'relative', marginLeft: 40, marginTop: 30, height: 200 }}>
                    <dom.div ref={wrapper} style={{ position: 'fixed', left: 0, top: 0, transform: 'translate(100px, 50px)', minWidth: 'max-content' }}>
                        <dom.div style={{ width: 80, height: 20 }}>Popover</dom.div>
                    </dom.div>
                    <dom.div ref={filled} style={{ position: 'fixed', inset: 0, pointerEvents: 'none' }} />
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    assert!(
        hands.says("wrapper fixed 0px 0px fill fixed 0px 0px 0px 0px"),
        "{}",
        hands.app.outline()
    );
}

#[test]
fn a_line_height_calc_that_comes_to_a_number_is_times_the_font_size() {
    // Tailwind 4's `text-sm` is `font-size: var(--text-sm); line-height: var(--tw-leading, var(--text-sm--line-height))`, with
    // `--text-sm--line-height: calc(1.25 / 0.875)`: 20px for 14px text, not 1.43px.
    let hands = open(
        "line-height-calc",
        r#"
        :root { --text-sm: 0.875rem; --text-sm--line-height: calc(1.25 / 0.875); --text-2xl: 1.5rem; --text-2xl--line-height: calc(2 / 1.5); }
        .text-sm { font-size: var(--text-sm); line-height: var(--tw-leading, var(--text-sm--line-height)); }
        .text-2xl { font-size: var(--text-2xl); line-height: var(--tw-leading, var(--text-2xl--line-height)); }
        .px { line-height: calc(10px + 6px); }
        "#,
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            const small = React.useRef(null);
            const big = React.useRef(null);
            const px = React.useRef(null);
            React.useLayoutEffect(() => {
                const line = (element) => Math.round(parseFloat(getComputedStyle(element).lineHeight));
                setSaid(['lines', line(small.current), line(big.current), line(px.current)].join(' '));
            }, []);
            return (
                <dom.div>
                    <dom.div ref={small} className="text-sm">Small</dom.div>
                    <dom.div ref={big} className="text-2xl">Big</dom.div>
                    <dom.div ref={px} className="px">Pixels</dom.div>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    assert!(hands.says("lines 20 32 16"), "{}", hands.app.outline());
}

#[test]
fn a_data_attribute_given_as_undefined_isnt_there() {
    // Radix's PopperAnchor gives its trigger `data-radix-popper-side={placedSide}`, undefined until the popover's been placed:
    // React writes no attribute for it, so `getAttribute` is null and `dataset` doesn't have it.
    let hands = open(
        "undefined-attribute",
        "",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            const node = React.useRef(null);
            React.useLayoutEffect(() => {
                const element = node.current;
                setSaid(['side', String(element.getAttribute('data-side')), element.hasAttribute('data-side'), 'side' in element.dataset,
                    'align', String(element.getAttribute('data-align')), 'state', element.getAttribute('data-state')].join(' '));
            }, []);
            return (
                <dom.div>
                    <dom.button ref={node} data-side={undefined} data-align={null} data-state="closed">Trigger</dom.button>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    assert!(
        hands.says("side null false false align null state closed"),
        "{}",
        hands.app.outline()
    );
}

#[test]
fn a_timer_set_as_an_event_comes_down_waits_until_its_gone_back_up() {
    // Radix's DismissableLayer notes each outside click on `document` on the way down, sets a `setTimeout(0)`, and undoes the
    // note as the click comes back up to `document`. The timer goes off after the click's been dispatched, as a browser runs
    // it in a task of its own, and finds the note undone: so a popover closes when the page outside it is clicked.
    let mut hands = open(
        "timer-after-dispatch",
        "",
        r#"
        const log = [];
        function Page() {
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                const down = () => {
                    log.push('down');
                    setTimeout(() => {
                        log.push('timer');
                        setSaid(log.join(' '));
                    }, 0);
                };
                const up = () => log.push('up');
                document.addEventListener('click', down, true);
                document.addEventListener('click', up);
                return () => {
                    document.removeEventListener('click', down, true);
                    document.removeEventListener('click', up);
                };
            }, []);
            return (
                <dom.div>
                    <dom.button style={{ display: 'block', width: 100, height: 40 }} onClick={() => log.push('element')}>Outside</dom.button>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    hands.click_at(50.0, 20.0);
    hands.wait_for("down element up timer");
}

#[test]
fn a_flex_row_in_right_to_left_text_starts_on_the_right() {
    // Shadcn's RTL examples are in `<div dir="rtl">`, which says `direction: rtl` as a browser's own sheet does: a row of buttons
    // starts at the right, the first rightmost, and a menu item's shortcut (`ml-auto`) goes to its left end. `row-reverse` there
    // goes from the left again.
    let hands = open(
        "rtl-row",
        r#"
        .row { display: flex; width: 300px; gap: 10px; }
        .back { flex-direction: row-reverse; }
        .wrap { flex-wrap: wrap; }
        .one, .two { width: 50px; height: 20px; }
        "#,
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            const one = React.useRef(null);
            const two = React.useRef(null);
            const three = React.useRef(null);
            const four = React.useRef(null);
            React.useLayoutEffect(() => {
                const x = (element) => Math.round(element.getBoundingClientRect().x);
                setSaid(['rtl', x(one.current), x(two.current), 'back', x(three.current), 'wrap', x(four.current)].join(' '));
            }, []);
            return (
                <dom.div dir="rtl" style={{ width: 300 }}>
                    <dom.div className="row">
                        <dom.div ref={one} className="one" />
                        <dom.div ref={two} className="two" />
                    </dom.div>
                    <dom.div className="row back">
                        <dom.div ref={three} className="one" />
                        <dom.div className="two" />
                    </dom.div>
                    <dom.div className="row wrap">
                        <dom.div ref={four} className="one" />
                        <dom.div className="two" />
                    </dom.div>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    assert!(hands.says("rtl 250 190 back 0 wrap 250"), "{}", hands.app.outline());
}

#[test]
fn var_written_right_against_var_stays_apart() {
    // tw-animate-css's `animate-in` is `animation: enter var(--tw-animation-duration,var(--tw-duration,.15s))var(--tw-ease,ease)...`
    // with nothing between the `var()`s: it's `.15s ease`, as CSS keeps tokens apart, and the popover fades and zooms in, from
    // `fade-in-0`'s `--tw-enter-opacity: 0` on the same element.
    let hands = open(
        "var-against-var",
        r#"
        @property --tw-enter-opacity { syntax: "*"; inherits: false; initial-value: 1; }
        @keyframes enter { from { opacity: var(--tw-enter-opacity, 1); } }
        .in { --tw-enter-opacity: 0; animation: enter var(--tw-animation-duration,var(--tw-duration,.15s))var(--tw-ease,ease)var(--tw-animation-delay,0s)var(--tw-animation-iteration-count,1)var(--tw-animation-direction,normal)var(--tw-animation-fill-mode,none); }
        "#,
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            const node = React.useRef(null);
            React.useLayoutEffect(() => {
                const style = getComputedStyle(node.current);
                setSaid(['plays', style.animationName, style.animationDuration].join(' '));
            }, []);
            return (
                <dom.div>
                    <dom.div ref={node} className="in">Popover</dom.div>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    assert!(hands.says("plays enter 0.15s"), "{}", hands.app.outline());
    // `--tw-enter-opacity` isn't passed down (`inherits: false`), but it's the element's own, so its keyframe starts from 0.
    assert!(hands.app.outline().contains("0 opacity n 0"), "{}", hands.app.outline());
}

#[test]
fn what_is_in_a_fixed_box_stays_put_in_the_window_as_the_page_scrolls() {
    // A popover's content is in Radix's `position: fixed` wrapper, which rsui draws as an overlay of the window: scrolling the
    // page doesn't move it, so its `getBoundingClientRect()` takes nothing away for that scrolling.
    let mut hands = open(
        "fixed-under-scroll",
        ".tall { height: 2000px; }",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            const inside = React.useRef(null);
            React.useEffect(() => {
                setTimeout(() => {
                    window.scrollTo(0, 100);
                    // Once the page has got there, which it may glide to.
                    const looking = setInterval(() => {
                        if (Math.round(window.scrollY) >= 50) {
                            clearInterval(looking);
                            setSaid(['inside', Math.round(inside.current.getBoundingClientRect().y), 'scrolled'].join(' '));
                        }
                    }, 20);
                }, 10);
            }, []);
            return (
                <dom.div>
                    <dom.div style={{ position: 'fixed', left: 0, top: 0, transform: 'translate(20px, 50px)' }}>
                        <dom.div ref={inside} style={{ width: 80, height: 20 }}>Popover</dom.div>
                    </dom.div>
                    <dom.p>{said}</dom.p>
                    <dom.div className="tall" />
                </dom.div>
            );
        }
        "#,
    );
    hands.wait_for("inside 50 scrolled");
}
