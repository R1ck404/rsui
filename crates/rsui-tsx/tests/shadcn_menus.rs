//! What shadcn/ui's menus (context menu, menubar, navigation menu, command) needed of rsui/web
//! to match Edge, each as small as it comes: Tailwind 4's unitless line heights, attributes React leaves out, `[data-disabled]`
//! on one of two alike items, a `position: fixed` popper's offset parent, `pointer-events: none` in the flow and on `body`, and
//! `mousemove` after what `pointermove` did.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn open(name: &str, css: &str, before: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-shadcn-menus-{}-{name}", std::process::id()));
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
fn a_line_height_thats_a_calc_of_numbers_is_times_the_text_size() {
    // Tailwind 4's `text-sm` is `line-height: var(--tw-leading, var(--text-sm--line-height))` with
    // `--text-sm--line-height: calc(1.25 / 0.875)`, a number: 20px at 14px, not 1.43px.
    let hands = open(
        "unitless-calc",
        ":root { --lh: calc(1.25 / 0.875); } .t { font-size: 14px; line-height: var(--tw-leading, var(--lh)); }",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                setSaid('lines ' + Math.round(parseFloat(getComputedStyle(document.getElementById('t')).lineHeight)) + 'px apart');
            }, []);
            return (
                <dom.div>
                    <dom.div id="t" className="t">One line</dom.div>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    assert!(hands.says("lines 20px apart"), "{}", hands.app.outline());
}

#[test]
fn an_attribute_react_leaves_out_isnt_there() {
    // Radix's `data-disabled={disabled ? '' : undefined}`: an enabled item has no `data-disabled`, not "undefined".
    let hands = open(
        "left-out",
        "",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                const item = document.getElementById('item');
                setSaid('attribute ' + item.getAttribute('data-disabled') + ', matches ' + item.matches('[data-disabled]'));
            }, []);
            return (
                <dom.div>
                    <dom.div id="item" data-disabled={undefined} data-gone={null}>Back</dom.div>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    assert!(hands.says("attribute null, matches false"), "{}", hands.app.outline());
}

#[test]
fn an_empty_attribute_on_one_of_two_alike_elements_is_matched() {
    // A Radix menu's items have the same classes; only the disabled one has `data-disabled=""`, which
    // `.data-\[disabled\]\:opacity-50[data-disabled]` is about. What the first item came to was kept for the second as well.
    let hands = open(
        "empty-attribute-kept",
        ".item[data-disabled] { opacity: 50%; pointer-events: none; }",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                const [back, forward] = document.querySelectorAll('.item');
                setSaid('back ' + getComputedStyle(back).opacity + ', forward ' + getComputedStyle(forward).opacity + ' ' + getComputedStyle(forward).pointerEvents);
            }, []);
            return (
                <dom.div>
                    <dom.div className="item">Back</dom.div>
                    <dom.div className="item" data-disabled="">Forward</dom.div>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    assert!(hands.says("back 1, forward 0.5 none"), "{}", hands.app.outline());
}

#[test]
fn a_fixed_box_says_its_fixed_and_has_no_offset_parent() {
    // Floating UI asks a Radix popper's wrapper (`position: fixed; left: 0; top: 0; transform: ...`) for its computed position
    // and offset parent, and placed a context menu from the positioned box it was in when it heard `static` and that box.
    let hands = open(
        "fixed-offset-parent",
        "",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                const fixed = document.getElementById('fixed');
                setSaid('position ' + getComputedStyle(fixed).position + ', offset parent ' + fixed.offsetParent);
            }, []);
            return (
                <dom.div style={{ position: 'relative', marginLeft: 40, marginTop: 40 }}>
                    <dom.div id="fixed" style={{ position: 'fixed', left: 0, top: 0 }}>Menu</dom.div>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    assert!(hands.says("position fixed, offset parent null"), "{}", hands.app.outline());
}

#[test]
fn what_doesnt_take_the_pointer_in_the_flow_passes_it_to_what_its_in() {
    // A disabled Radix menu item is `pointer-events: none`: a click on it is its group's, as on a page.
    let mut hands = open(
        "pointer-events-flow",
        ".off { pointer-events: none; }",
        r#"
        function Page() {
            const [log, setLog] = React.useState('nothing');
            return (
                <dom.div>
                    <dom.div id="group" style={{ padding: 10 }} onClick={(event) => setLog('group, target ' + event.target.id)}>
                        <dom.div id="forward" key="forward" className="off" style={{ height: 30 }} onClick={() => setLog('forward')}>Forward</dom.div>
                    </dom.div>
                    <dom.p>{'clicked ' + log}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    let (x, y) = hands.middle_of("forward");
    hands.click_at(x, y);
    hands.settle();
    assert!(hands.says("clicked group, target group"), "{}", hands.app.outline());
}

#[test]
fn body_that_doesnt_take_the_pointer_leaves_it_to_what_says_it_does() {
    // A modal Radix layer sets `body.style.pointerEvents = 'none'` and its own content `pointer-events: auto`: a click outside
    // it is the page's `<html>`'s, and one in it is the content's; Tab still goes through the content.
    let mut hands = open(
        "pointer-events-body",
        "",
        r#"
        function Page() {
            const [log, setLog] = React.useState('nothing');
            React.useEffect(() => {
                document.body.style.pointerEvents = 'none';
                const heard = (event) => setLog((was) => was + ', ' + event.target.tagName.toLowerCase() + (event.target.id ? '#' + event.target.id : ''));
                window.addEventListener('pointerdown', heard, true);
                return () => window.removeEventListener('pointerdown', heard, true);
            }, []);
            return (
                <dom.div>
                    <dom.div id="outside" key="outside" style={{ height: 40 }}>Outside</dom.div>
                    <dom.div id="layer" style={{ pointerEvents: 'auto', height: 40 }}>
                        <dom.button id="inside" key="inside">Inside</dom.button>
                    </dom.div>
                    <dom.p>{'heard ' + log}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    let (x, y) = hands.middle_of("outside");
    hands.click_at(x, y);
    hands.settle();
    let (x, y) = hands.middle_of("inside");
    hands.click_at(x, y);
    hands.settle();
    assert!(hands.says("heard nothing, html, button#inside"), "{}", hands.app.outline());
}

#[test]
fn mousemove_comes_after_what_pointermove_did() {
    // Radix's menu item focuses itself on `pointermove`: on a page, `mousemove` is only sent once `pointermove` has been
    // everywhere, so a listener on `window` hears pointermove, the focus, then mousemove.
    let mut hands = open(
        "pointer-then-mouse",
        "",
        r#"
        function Page() {
            const [log, setLog] = React.useState([]);
            React.useEffect(() => {
                const heard = (event) => setLog((was) => (was.length < 6 && !was.includes(event.type) ? [...was, event.type] : was));
                for (const type of ['pointermove', 'mousemove', 'focusin']) window.addEventListener(type, heard, true);
            }, []);
            return (
                <dom.div>
                    <dom.div id="item" key="item" tabIndex={-1} style={{ height: 40 }} onPointerMove={(event) => event.currentTarget.focus()}>Item</dom.div>
                    <dom.p>{'order ' + log.join(' ')}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    let (x, y) = hands.middle_of("item");
    hands.move_at(x, y);
    hands.settle();
    hands.move_at(x + 2.0, y);
    hands.settle();
    assert!(hands.says("order pointermove focusin mousemove"), "{}", hands.app.outline());
}

#[test]
fn what_was_just_put_in_has_the_keyboard_the_moment_its_given_it() {
    // Radix's FocusScope focuses a menu it's just put in, in an effect, and RovingFocusGroup then focuses the first item and asks
    // `document.activeElement` whether that worked: it's there the moment React's put it in, before it's drawn, and it hears
    // `focusin` once, not again when it's drawn.
    let mut hands = open(
        "focus-before-drawn",
        "",
        r#"
        function Content({ note }) {
            const ref = React.useRef(null);
            React.useEffect(() => {
                ref.current.focus();
                note('active ' + document.activeElement.id);
            }, []);
            return <dom.div id="content" ref={ref} tabIndex={-1} onFocus={() => note('onFocus')}>Content</dom.div>;
        }
        function Page() {
            const [open, setOpen] = React.useState(false);
            const [said, setSaid] = React.useState([]);
            const note = (x) => setSaid((was) => [...was, x]);
            return (
                <dom.div>
                    <dom.button key="open" onClick={() => setOpen(true)}>Open</dom.button>
                    {open ? <Content note={note} /> : null}
                    <dom.p>{'said ' + said.join(', ')}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    hands.click("open");
    hands.settle();
    hands.settle();
    assert!(hands.says("said onFocus, active content"), "{}", hands.app.outline());
}

#[test]
fn effects_inside_run_first_when_both_render_again_in_one_round() {
    // Radix's FocusScope sets its state from its ref, and the menu items' RovingFocusGroup sets its own in a layout effect: both
    // render again in the same round, and the items' effects (which put them back in their collection) run before FocusScope's
    // (which focuses the menu, and RovingFocusGroup then its first item), as React's do, whatever rendered first.
    let mut hands = open(
        "effects-inside-first",
        "",
        r#"
        function Item({ items, name }) {
            React.useEffect(() => {
                items.add(name);
                return () => { items.delete(name); };
            });
            return <dom.div>{name}</dom.div>;
        }
        function Group({ items }) {
            const [count, setCount] = React.useState(0);
            React.useLayoutEffect(() => {
                if (count === 0) setCount(1);
            }, [count]);
            return <dom.div>{['a', 'b'].map((name) => <Item key={name} items={items} name={name} />)}</dom.div>;
        }
        function Scope({ items, children }) {
            const [container, setContainer] = React.useState(null);
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                if (container !== null) setSaid('items ' + items.size);
            }, [container]);
            return <dom.div ref={setContainer}>{children}<dom.p>{said}</dom.p></dom.div>;
        }
        function Page() {
            const items = React.useRef(new Set()).current;
            const [open, setOpen] = React.useState(false);
            return (
                <dom.div>
                    <dom.button key="open" onClick={() => setOpen(true)}>Open</dom.button>
                    {open ? <Scope items={items}><Group items={items} /></Scope> : null}
                </dom.div>
            );
        }
        "#,
    );
    hands.click("open");
    hands.settle();
    hands.settle();
    assert!(hands.says("items 2"), "{}", hands.app.outline());
}

#[test]
fn text_beside_an_element_in_a_flex_box_keeps_its_space() {
    // `New Tab <MenubarShortcut>T</MenubarShortcut>` in a flex item: the space isn't drawn, but it's in `textContent`.
    let hands = open(
        "flex-text-space",
        "",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                document.createElement('div');
                setSaid('[' + document.getElementById('item').textContent + ']');
            }, []);
            return (
                <dom.div>
                    <dom.div id="item" style={{ display: 'flex', gap: 8 }}>New Tab <dom.span>T</dom.span></dom.div>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    assert!(hands.says("[New Tab T]"), "{}", hands.app.outline());
}

#[test]
fn a_path_in_an_svg_is_shown_where_its_shape_is() {
    // lucide's ChevronRight (`m9 18 6-6-6-6` in a 24 unit viewBox) drawn 16px wide: the path is 4 by 8, at 6, 4 in the svg.
    let mut hands = open(
        "svg-path-box",
        "",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            const measure = () => {
                const svg = document.getElementById('icon').getBoundingClientRect();
                const path = document.querySelector('#icon path');
                const box = path.getBoundingClientRect();
                setSaid('path ' + getComputedStyle(path).display + ' ' + Math.round(box.x - svg.x) + ',' + Math.round(box.y - svg.y) + ' ' + Math.round(box.width) + 'x' + Math.round(box.height));
            };
            return (
                <dom.div>
                    <dom.button key="measure" onClick={measure}>Measure</dom.button>
                    <dom.svg id="icon" width={24} height={24} viewBox="0 0 24 24" style={{ width: 16, height: 16 }}><dom.path d="m9 18 6-6-6-6" /></dom.svg>
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    hands.click("measure");
    hands.settle();
    assert!(hands.says("path inline 6,4 4x8"), "{}", hands.app.outline());
}

#[test]
fn element_from_point_finds_what_a_transform_moved_there() {
    // A Radix popper is `position: fixed` at 0, 0 and moved where it's shown by `transform: translate()`: what's under the
    // pointer there is it, as the window's own hit test has it.
    let mut hands = open(
        "from-point-transform",
        "",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            const measure = () => {
                const found = document.elementFromPoint(130, 80);
                setSaid('found ' + (found && found.id));
            };
            return (
                <dom.div>
                    <dom.button key="measure" onClick={measure} style={{ marginTop: 300 }}>Measure</dom.button>
                    <dom.div id="moved" style={{ position: 'fixed', left: 0, top: 0, width: 60, height: 60, transform: 'translate(100px, 50px)', background: 'red' }} />
                    <dom.p style={{ marginTop: 200 }}>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    hands.click("measure");
    hands.settle();
    assert!(hands.says("found moved"), "{}", hands.app.outline());
}

#[test]
fn a_press_told_not_to_has_no_mouse_events_but_still_clicks() {
    // Radix's menu triggers prevent `pointerdown`'s default to open without taking the keyboard: no `mousedown` or `mouseup`
    // follows, and `click` still does.
    let mut hands = open(
        "held-back-mouse",
        "",
        r#"
        function Page() {
            const [log, setLog] = React.useState([]);
            React.useEffect(() => {
                const heard = (event) => setLog((was) => [...was, event.type]);
                for (const type of ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']) document.addEventListener(type, heard);
            }, []);
            return (
                <dom.div>
                    <dom.button key="trigger" onPointerDown={(event) => event.preventDefault()}>File</dom.button>
                    <dom.p>{'heard ' + log.join(' ')}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    hands.click("trigger");
    hands.settle();
    assert!(hands.says("heard pointerdown pointerup click"), "{}", hands.app.outline());
}

#[test]
fn what_has_the_keyboard_says_blur_when_its_taken_out() {
    // A menu item that has the keyboard goes when its menu closes: Chromium says `blur` and `focusout` to nothing then, and the
    // page's `activeElement` is `body`, before Radix gives the trigger the keyboard back.
    let mut hands = open(
        "blur-on-removal",
        "",
        r#"
        function Page() {
            const [shown, setShown] = React.useState(true);
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                document.getElementById('item').focus();
                document.addEventListener('focusout', (event) => setSaid('focusout to ' + event.relatedTarget));
                setTimeout(() => setShown(false), 0);
            }, []);
            React.useEffect(() => {
                if (!shown) setTimeout(() => setSaid((was) => was + ', active ' + document.activeElement.tagName), 0);
            }, [shown]);
            return (
                <dom.div>
                    {shown ? <dom.div id="item" tabIndex={-1}>Item</dom.div> : null}
                    <dom.p>{said}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    for _ in 0..6 {
        hands.settle();
    }
    assert!(hands.says("focusout to null, active BODY"), "{}", hands.app.outline());
}

#[test]
fn an_event_that_doesnt_go_up_reaches_what_listens_on_html() {
    // Radix's DismissableLayer sends `pointerDownOutside` (no bubbling) to what a press outside landed on, which is `<html>`
    // while a modal menu has `body` at `pointer-events: none`, and listens for it there.
    let hands = open(
        "html-own-listener",
        "",
        r#"
        function Page() {
            const [said, setSaid] = React.useState('');
            React.useEffect(() => {
                const html = document.documentElement;
                html.addEventListener('outside', () => setSaid('heard on html'), { once: true });
                html.dispatchEvent(new CustomEvent('outside', { bubbles: false, cancelable: true }));
            }, []);
            return <dom.p>{said}</dom.p>;
        }
        "#,
    );
    assert!(hands.says("heard on html"), "{}", hands.app.outline());
}

#[test]
fn a_growing_item_thats_no_wider_than_its_content_stays_that_wide() {
    // NavigationMenu's nav is `flex-1 max-w-max` in a centred row: it grows no wider than its list, and sits in the middle.
    let mut hands = open(
        "max-width-max-content",
        ".row { display: flex; justify-content: center; width: 400px; } .grow { flex: 1 1 0%; max-width: max-content; }",
        r#"
        function Page() {
            return (
                <dom.div className="row">
                    <dom.div className="grow"><dom.div key="inside" style={{ width: 100, height: 20 }} /></dom.div>
                </dom.div>
            );
        }
        "#,
    );
    let [x, _, width, _] = hands.rect_of("inside");
    assert!((width - 100.0).abs() < 0.5 && (x - 150.0).abs() < 0.5, "at {x}, {width} wide: {}", hands.app.outline());
}

#[test]
fn a_right_press_gives_the_keyboard_and_auxclick_comes_before_contextmenu() {
    // Chromium focuses what a right press is on (Edge's menubar trigger), and says `auxclick` before `contextmenu`.
    let mut hands = open(
        "right-press",
        "",
        r#"
        function Page() {
            const [log, setLog] = React.useState([]);
            React.useEffect(() => {
                const heard = (event) => setLog((was) => [...was, event.type + (event.type === 'focusin' ? ':' + event.target.id : '')]);
                for (const type of ['focusin', 'auxclick', 'contextmenu']) document.addEventListener(type, heard);
            }, []);
            return (
                <dom.div>
                    <dom.button id="edit" key="edit">Edit</dom.button>
                    <dom.p>{'heard ' + log.join(' ')}</dom.p>
                </dom.div>
            );
        }
        "#,
    );
    let (x, y) = hands.middle_of("edit");
    hands.app.button_down(x, y, rsui::PointerButton::Secondary);
    hands.settle();
    hands.app.button_up(x, y, rsui::PointerButton::Secondary).unwrap();
    hands.settle();
    assert!(hands.says("heard focusin:edit auxclick contextmenu"), "{}", hands.app.outline());
}

#[test]
fn hovering_a_menu_item_can_focus_without_scrolling_the_page() {
    let mut hands = open(
        "hover-prevent-scroll",
        "",
        r#"
        function Page() {
            return (
                <dom.div>
                    <dom.div key="marker" style={{ height: 1000 }}>Page content</dom.div>
                    <dom.div style={{ position: 'fixed', left: 20, top: 20 }}>
                        <dom.button key="start" onClick={() => window.scrollTo(0, 300)}>Scroll page</dom.button>
                        <dom.div id="item" key="item" tabIndex={-1} style={{ height: 30 }}
                            onPointerMove={(event) => event.currentTarget.focus({ preventScroll: true })}>Menu item</dom.div>
                        <dom.div id="nested" key="nested" tabIndex={-1} style={{ height: 30 }}
                            onPointerMove={(event) => event.currentTarget.focus({ preventScroll: true, focusVisible: false })}>Nested menu item</dom.div>
                    </dom.div>
                </dom.div>
            );
        }
        "#,
    );
    hands.click("start");
    let before = hands.rect_of("marker");
    assert!(before[1] < -200.0, "page should be scrolled: {before:?}");
    for key in ["item", "nested", "item", "nested"] {
        let (x, y) = hands.middle_of(key);
        hands.move_at(x, y);
        let after = hands.rect_of("marker");
        assert!(
            (after[1] - before[1]).abs() < 0.5,
            "hovering {key} moved page: {before:?} -> {after:?}"
        );
    }
}

#[test]
fn prevent_scroll_focus_does_not_disable_a_later_normal_focus_reveal() {
    let mut hands = open(
        "focus-reveal-after-prevent",
        "",
        r#"
        function Page() {
            return (
                <dom.div>
                    <dom.button key="quiet" onClick={() => document.getElementById('target').focus({ preventScroll: true })}>Focus quietly</dom.button>
                    <dom.button key="normal" onClick={() => document.getElementById('other').focus()}>Focus normally</dom.button>
                    <dom.div style={{ height: 1000 }}>Space</dom.div>
                    <dom.button id="target" key="target">Target</dom.button>
                    <dom.button id="other" key="other">Other target</dom.button>
                </dom.div>
            );
        }
        "#,
    );
    let before = hands.rect_of("target");
    hands.click("quiet");
    assert!((hands.rect_of("target")[1] - before[1]).abs() < 0.5);
    let mut focused = Vec::new();
    assert!(hands::path_of(
        hands.app.runtime().root(),
        "target",
        &mut focused
    ));
    assert_eq!(hands.app.focused(), Some(focused.as_slice()));
    hands.click("normal");
    let after = hands.rect_of("other");
    assert!(
        after[1] >= 0.0 && after[1] < 400.0,
        "normal focus should reveal its target: {after:?}"
    );
}

#[test]
fn tab_after_prevent_scroll_focus_still_reveals_the_next_item() {
    let mut hands = open(
        "tab-after-prevent-scroll",
        "",
        r#"
        function Page() {
            return (
                <dom.div>
                    <dom.button key="quiet" onClick={() => document.getElementById('target').focus({ preventScroll: true })}>Focus quietly</dom.button>
                    <dom.div style={{ height: 1000 }}>Space</dom.div>
                    <dom.button id="target" key="target">Target</dom.button>
                    <dom.button id="next" key="next">Next target</dom.button>
                </dom.div>
            );
        }
        "#,
    );
    hands.click("quiet");
    assert!(hands.rect_of("target")[1] > 400.0);
    hands.press(hands::KeyCode::Tab);
    let after = hands.rect_of("next");
    assert!(
        after[1] >= 0.0 && after[1] < 400.0,
        "Tab should reveal its target: {after:?}"
    );
}

#[test]
fn a_body_portal_does_not_reteach_its_origins_sibling_classes() {
    let mut hands = open(
        "portal-sibling-classes",
        ".trigger + .after { width: 77px; } .after { height: 20px; }",
        r#"
        import { createPortal } from 'react-dom';
        function Floating() {
            const [open, setOpen] = React.useState(false);
            const trigger = React.useMemo(() => <dom.button className="trigger">Trigger</dom.button>, []);
            React.useLayoutEffect(() => setOpen(true), []);
            return <>
                {trigger}
                {open ? createPortal(<dom.div>Popup</dom.div>, document.body) : null}
            </>;
        }
        function Page() {
            return <dom.div><Floating /><dom.div key="after" className="after">After</dom.div></dom.div>;
        }
        "#,
    );
    assert_eq!(hands.app.runtime().runaways(), 0, "portal sibling metadata must settle");
    assert!((hands.rect_of("after")[2] - 77.0).abs() < 0.5);
}

#[test]
fn a_body_portal_does_not_use_siblings_from_its_react_origin() {
    let mut hands = open(
        "portal-origin-siblings",
        ".popup { width: 30px; height: 20px; } .before + .popup { width: 77px; }",
        r#"
        import { createPortal } from 'react-dom';
        function Floating() {
            return createPortal(<dom.div key="popup" className="popup">Popup</dom.div>, document.body);
        }
        function Page() {
            return <dom.div><dom.div className="before">Before</dom.div><Floating /></dom.div>;
        }
        "#,
    );
    assert!((hands.rect_of("popup")[2] - 30.0).abs() < 0.5, "a portal has body siblings, not its origin's siblings");
}

#[test]
fn locking_root_overflow_preserves_the_existing_scroll_position() {
    let mut hands = open(
        "root-overflow-lock",
        "",
        r#"
        function Page() {
            return <dom.div>
                <dom.div key="marker" style={{height: 1500}}>Content</dom.div>
                <dom.div style={{position:'fixed', top:20, left:20}}>
                    <dom.button key="go" onClick={() => window.scrollTo(0, 300)}>Scroll</dom.button>
                    <dom.button key="lock" onClick={() => (document.body.style.overflow = 'hidden')}>Lock</dom.button>
                    <dom.button key="unlock" onClick={() => (document.body.style.overflow = '')}>Unlock</dom.button>
                </dom.div>
            </dom.div>;
        }
        "#,
    );
    hands.click("go");
    let before = hands.rect_of("marker");
    assert!(before[1] < -200.0, "page not scrolled: {before:?}, {}", hands.app.rectangles());
    hands.click("lock");
    let locked = hands.rect_of("marker");
    assert!((locked[1] - before[1]).abs() < 0.5, "locking moved page: {before:?} -> {locked:?}");
    hands.click("unlock");
    let unlocked = hands.rect_of("marker");
    assert!((unlocked[1] - before[1]).abs() < 0.5, "unlocking moved page: {before:?} -> {unlocked:?}");
}

#[test]
fn scroll_into_view_of_a_viewport_fixed_box_does_not_scroll_the_document() {
    let mut hands = open(
        "fixed-scroll-into-view",
        "",
        r#"
        function Page() {
            return <dom.div>
                <dom.div key="marker" style={{height: 1500}}>Content</dom.div>
                <dom.div style={{position:'fixed', top:20, left:20}}>
                    <dom.button key="go" onClick={() => window.scrollTo(0,300)}>Scroll</dom.button>
                    <dom.button key="reveal" onClick={() => document.getElementById('fixed').scrollIntoView()}>Reveal fixed box</dom.button>
                </dom.div>
                <dom.div id="fixed" key="fixed" style={{position:'fixed', top:600, left:20, height:30}}>Fixed box</dom.div>
            </dom.div>;
        }
        "#,
    );
    hands.click("go");
    let before = hands.rect_of("marker");
    hands.click("reveal");
    let after = hands.rect_of("marker");
    assert!((after[1] - before[1]).abs() < 0.5, "fixed reveal moved page: {before:?} -> {after:?}");
}

#[test]
fn scroll_into_view_inside_a_fixed_scroller_only_moves_that_scroller() {
    let mut hands = open(
        "fixed-inner-scroll-into-view",
        "",
        r#"
        function Page() {
            return <dom.div>
                <dom.div key="marker" style={{height:1500}}>Content</dom.div>
                <dom.div style={{position:'fixed', top:20, left:20}}>
                    <dom.button key="go" onClick={() => window.scrollTo(0,300)}>Scroll</dom.button>
                    <dom.button key="reveal" onClick={() => document.getElementById('target').scrollIntoView({block:'nearest'})}>Reveal inside</dom.button>
                </dom.div>
                <dom.div style={{position:'fixed', top:350, left:20, width:200, height:100, overflow:'auto'}}>
                    <dom.div style={{height:600}}>Space</dom.div>
                    <dom.button id="target" key="target">Inner target</dom.button>
                </dom.div>
            </dom.div>;
        }
        "#,
    );
    hands.click("go");
    let before = hands.rect_of("marker");
    assert!(hands.rect_of("target")[1] > 400.0);
    hands.click("reveal");
    let after = hands.rect_of("marker");
    assert!((after[1] - before[1]).abs() < 0.5, "inner fixed reveal moved page: {before:?} -> {after:?}");
    let target = hands.rect_of("target");
    assert!(target[1] >= 349.5 && target[1] + target[3] <= 450.5, "inner target not revealed: {target:?}");
}

#[test]
fn scroll_into_view_of_fixed_content_in_a_transformed_box_can_scroll_the_document() {
    let mut hands = open(
        "contained-fixed-scroll-into-view",
        "",
        r#"
        function Page() {
            return <dom.div>
                <dom.button key="reveal" onClick={() => document.getElementById('target').scrollIntoView({block:'nearest'})}>Reveal contained fixed</dom.button>
                <dom.div style={{height:1000}}>Space</dom.div>
                <dom.div style={{transform:'translateZ(0)', height:400}}>
                    <dom.div id="target" key="target" style={{position:'fixed', top:20, left:20, width:100, height:30}}>Contained fixed</dom.div>
                </dom.div>
            </dom.div>;
        }
        "#,
    );
    assert!(hands.rect_of("target")[1] > 400.0);
    hands.click("reveal");
    let target = hands.rect_of("target");
    assert!(target[1] >= -0.5 && target[1] + target[3] <= 400.5, "contained target not revealed: {target:?}");
}
