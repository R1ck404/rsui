//! Text with parts that look different, from TSX, used as a person would.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn open(name: &str, source: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("paragraphs-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    Hands::open(&file, (400, 300))
}

#[test]
fn words_and_spans_inside_text_are_one_paragraph() {
    let mut hands = open(
        "one",
        r#"
        export default function App() {
            return (
                <window>
                    <text key="words">Some <span fontWeight="bold">bold</span> words</text>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    let outline = hands.app.outline();
    assert!(outline.contains(r#"text "Some ""#), "{outline}");
    assert!(outline.contains(r#"text "bold""#), "{outline}");
    assert!(outline.contains(r#"text " words""#), "{outline}");
    let (_, first_y) = hands.middle_of("words");
    assert!(first_y < 30.0, "it's one line of text\n{outline}");
}

#[test]
fn a_paragraph_wraps_in_what_it_is_given() {
    let mut hands = open(
        "wrap",
        r#"
        export default function App() {
            return (
                <window>
                    <column width={100}>
                        <text key="words">Some words <span key="part" fontWeight="bold">and more of them</span> to go over several lines</text>
                    </column>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    let mut path = Vec::new();
    assert!(hands::path_of(
        hands.app.runtime().root(),
        "words",
        &mut path
    ));
    let rect = hands.app.layout().rect(&path).unwrap();
    assert!(rect.width <= 100.0, "{rect:?}");
    assert!(rect.height > 50.0, "it's on several lines: {rect:?}");
}

#[test]
fn state_in_a_span_changes_what_shows_and_clicking_a_link_works() {
    let mut hands = open(
        "state",
        r#"
        import { useState } from 'rsui';
        export default function App() {
            const [count, setCount] = useState(0);
            return (
                <window>
                    <text>
                        Clicked <span fontWeight="bold">{count} times</span>, so <link key="more" onClick={() => setCount(count + 1)}>click</link> again.
                    </text>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    assert!(hands.says(r#"text "0 times""#), "{}", hands.app.outline());
    hands.click("more");
    assert!(hands.says(r#"text "1 times""#), "{}", hands.app.outline());
    hands.click("more");
    assert!(hands.says(r#"text "2 times""#), "{}", hands.app.outline());
}

#[test]
fn components_and_lists_can_be_parts_of_a_paragraph() {
    let mut hands = open(
        "parts",
        r#"
        function Strong({ children }) {
            return <span fontWeight="bold">{children}</span>;
        }
        export default function App() {
            const names = ['one', 'two', 'three'];
            return (
                <window>
                    <text key="words">
                        A <Strong>component</Strong> and {names.map((name) => <span key={name}>{name} </span>)}
                    </text>
                    {/* Without anything to show, it's still something that can be there. */}
                    <text key="nothing">{false}{null}</text>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    let outline = hands.app.outline();
    assert!(outline.contains(r#"text "component""#), "{outline}");
    assert!(outline.contains(r#"text "three ""#), "{outline}");
}

#[test]
fn text_that_asks_for_it_can_be_selected_and_plain_text_cannot() {
    let mut hands = open(
        "selecting",
        r#"
        export default function App() {
            return (
                <window>
                    <text key="yes" selectable>Words to select</text>
                    <text key="no">Words to leave</text>
                </window>
            );
        }
        "#,
    );
    hands.settle();
    assert!(
        hands.says(r#"text "Words to select""#),
        "{}",
        hands.app.outline()
    );
    let (x, y) = hands.middle_of("yes");
    hands.app.pointer_down(x, y, 3);
    hands.settle();
    assert_eq!(hands.app.selected_text(), Some("Words to select"));
    let _ = hands.app.pointer_up(x, y);
    let (x, y) = hands.middle_of("no");
    hands.app.pointer_down(x, y, 3);
    hands.settle();
    assert_eq!(hands.app.selected_text(), None);
}
