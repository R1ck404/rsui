//! What's rendered is only what what changed reaches: a long list's rows by what `renderRow` read for each, what a component only
//! gave on to its children by what they read of it, a lookup in a `Set` by what it said, and what a page's element puts around
//! what's in it by whether that does the same (see "What a component read" in DESIGN.md).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use rsui_headless::{Headless, Query};

/// A new folder for each test, as tests run side by side.
fn folder() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "given-on-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

fn start(source: &str) -> Headless {
    let path = folder().join("app.tsx");
    std::fs::write(&path, source).unwrap();
    let entry = rsui_tsx::load(&path, 512 * 1024).unwrap();
    Headless::new(rsui_tsx::root, entry)
}

fn key(name: &str) -> Query {
    Query::new().key(name)
}

fn line_with(ui: &Headless, part: &str) -> String {
    ui.outline()
        .lines()
        .find(|line| line.contains(part))
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("nothing says {part:?} in\n{}", ui.outline()))
}

/// How many components render for a click.
fn renders_for(ui: &mut Headless, name: &str) -> u64 {
    let before = ui.runtime().renders();
    ui.click(&key(name)).unwrap();
    ui.runtime().renders() - before
}

/// A long list whose rows are given what's picked as a lookup in a `Set`, and a look made again each time the app renders, which
/// each row only gives on to its mark, and the mark only reads `mark` of. The look's shade changes with nothing reading it, and a
/// plain function the row calls reads what it's given.
const LIST: &str = r#"
    import { useState } from 'rsui';
    function label(look, index) {
        return look.prefix + index;
    }
    function Mark(props) {
        return <text>{props.look.mark}</text>;
    }
    function Row(props) {
        return (
            <button key={'row' + props.index} onClick={props.onPick}>
                <Mark look={props.look} />
                <text>{label(props.look, props.index) + (props.picked ? ' (picked)' : '')}</text>
            </button>
        );
    }
    export default function App() {
        const [picked, setPicked] = useState(() => new Set());
        const [shade, setShade] = useState(0);
        const look = { mark: '*', prefix: 'item ', shade };
        const renderRow = (index) => (
            <Row key={index} index={index} look={look} picked={picked.has(index)} onPick={() => setPicked(new Set([index]))} />
        );
        return (
            <window>
                <column>
                    <button key="shade" onClick={() => setShade(shade + 1)}>shade {shade}</button>
                    <list count={50} rowHeight={24} height={480} renderRow={renderRow} />
                </column>
            </window>
        );
    }
"#;

#[test]
fn picking_a_row_renders_the_rows_whose_lookup_changed() {
    let mut ui = start(LIST);
    let first = renders_for(&mut ui, "row3");
    assert!(line_with(&ui, "item 3").contains("(picked)"));
    // The app, the list, and the row that's picked now, with what's in it: not the twenty others that show.
    assert!(first <= 6, "{first} rendered");
    let second = renders_for(&mut ui, "row7");
    assert!(line_with(&ui, "item 7").contains("(picked)"));
    assert!(!line_with(&ui, "item 3").contains("(picked)"));
    assert!(second <= 9, "{second} rendered");
}

#[test]
fn what_a_row_only_gives_on_is_the_same_while_what_reads_it_reads_the_same() {
    let mut ui = start(LIST);
    // The look is made again with another shade, which nothing in a row reads: no row renders.
    let shaded = renders_for(&mut ui, "shade");
    assert!(line_with(&ui, "shade 1").contains("shade 1"));
    assert!(shaded <= 3, "{shaded} rendered");
    // And a row that renders afterwards for itself still says what it should.
    ui.click(&key("row5")).unwrap();
    assert!(line_with(&ui, "item 5").contains("(picked)"));
    assert!(line_with(&ui, "*").contains('*'));
}

/// A page's list, whose items are each given a place among the rest by what the `<ul>` puts around them.
const PAGE: &str = r#"
    import { useState } from 'rsui';
    function Item(props) {
        return <li className="item">{props.label}</li>;
    }
    export default function App() {
        const [count, setCount] = useState(0);
        return (
            <window>
            <div className="page">
                <button key="more" className="more" onClick={() => setCount(count + 1)}>{'count ' + count}</button>
                <ul className="items">
                    {Array.from({ length: 30 }, (_, index) => <Item key={index} label={'item ' + index} />)}
                    <li key="count" className="count">{'clicked ' + count}</li>
                </ul>
            </div>
            </window>
        );
    }
"#;

#[test]
fn what_a_page_element_puts_around_what_is_in_it_doesnt_render_for_being_made_again() {
    let mut ui = start(PAGE);
    // The first time, the list finds out what kind of element each item makes, which tells each where it is anew.
    ui.click(&key("more")).unwrap();
    let before = ui.runtime().renders();
    ui.click(&key("more")).unwrap();
    let rendered = ui.runtime().renders() - before;
    assert!(line_with(&ui, "clicked 2").contains("clicked 2"));
    // The app, what draws the page's elements it wrote again, the list, whose last item says something else, and the button's
    // text: not the thirty items before it, or what's around each of them to say where it is among the rest, which is the same.
    assert!(rendered < 15, "{rendered} rendered");
}
