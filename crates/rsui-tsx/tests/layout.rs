//! Margins, absolute positioning, grid and translate, as they're written in TSX, and that what's clicked is where it shows.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn open(name: &str, source: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-layout-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    Hands::open(&file, (400, 300))
}

fn rect_of(hands: &mut Hands, key: &str) -> (f32, f32, f32, f32) {
    hands.settle();
    let mut path = Vec::new();
    assert!(
        hands::path_of(hands.app.runtime().root(), key, &mut path),
        "no {key}\n{}",
        hands.app.outline()
    );
    let rect = hands.app.layout().rect(&path).expect("it's been laid out");
    (rect.x, rect.y, rect.width, rect.height)
}

#[test]
fn margins_are_written_the_way_padding_is() {
    let mut hands = open(
        "margins",
        r#"
        export default function App() {
            return (
                <window>
                    <group key="all" height={20} margin={10} />
                    <group key="sides" height={20} marginX={30} />
                    <group key="ends" height={20} marginY={5} />
                    <group key="one" height={20} marginTop={7} marginLeft={9} />
                    <group key="after" height={20} marginTop={-3} />
                </window>
            );
        }
        "#,
    );
    assert_eq!(rect_of(&mut hands, "all"), (10.0, 10.0, 380.0, 20.0));
    assert_eq!(rect_of(&mut hands, "sides"), (30.0, 40.0, 340.0, 20.0));
    assert_eq!(rect_of(&mut hands, "ends"), (0.0, 65.0, 400.0, 20.0));
    assert_eq!(rect_of(&mut hands, "one"), (9.0, 97.0, 391.0, 20.0));
    assert_eq!(rect_of(&mut hands, "after"), (0.0, 114.0, 400.0, 20.0));
}

#[test]
fn something_absolute_goes_where_it_says_and_can_be_clicked_there() {
    let mut hands = open(
        "absolute",
        r#"
        import { useState } from 'rsui';
        export default function App() {
            const [count, setCount] = useState(0);
            return (
                <window>
                    <group key="box" width={200} height={100} marginLeft={50} marginTop={50}>
                        <group key="first" height={50} />
                        <button key="corner" position="absolute" right={10} bottom={10} width={40} height={20} onClick={() => setCount(count + 1)}>Go</button>
                        <group key="middle" position="absolute" left="50%" top="50%" width={20} height={10} translateX="-50%" translateY="-50%" />
                        <group key="second" height={50} />
                    </group>
                    <text key="count">{'Count ' + count}</text>
                </window>
            );
        }
        "#,
    );
    let mut path = Vec::new();
    assert!(hands::path_of(
        hands.app.runtime().root(),
        "second",
        &mut path
    ));
    assert_eq!(rect_of(&mut hands, "first"), (50.0, 50.0, 200.0, 50.0));
    assert_eq!(
        rect_of(&mut hands, "second"),
        (50.0, 100.0, 200.0, 50.0),
        "they took up no room"
    );
    assert_eq!(rect_of(&mut hands, "corner"), (200.0, 120.0, 40.0, 20.0));
    assert_eq!(rect_of(&mut hands, "middle"), (140.0, 95.0, 20.0, 10.0));
    hands.click("corner");
    assert!(hands.says("Count 1"), "{}", hands.app.outline());
}

#[test]
fn a_grid_is_written_the_way_css_writes_it() {
    let mut hands = open(
        "grid",
        r#"
        export default function App() {
            return (
                <window display="grid" columns="100px 1fr repeat(2, 50px)" gap={10} padding={10}>
                    <group key="a" height={20} />
                    <group key="b" height={40} />
                    <group key="c" height={20} />
                    <group key="d" height={20} />
                    <group key="wide" height={20} gridColumn="1 / -1" />
                    <group key="two" height={20} gridColumn="span 2" />
                    <group key="there" height={20} gridColumn={4} />
                </window>
            );
        }
        "#,
    );
    // 400 less 20 of padding, 30 of gaps, and 200 of columns leaves 150.
    assert_eq!(rect_of(&mut hands, "a"), (10.0, 10.0, 100.0, 20.0));
    assert_eq!(rect_of(&mut hands, "b"), (120.0, 10.0, 150.0, 40.0));
    assert_eq!(rect_of(&mut hands, "c"), (280.0, 10.0, 50.0, 20.0));
    assert_eq!(rect_of(&mut hands, "d"), (340.0, 10.0, 50.0, 20.0));
    assert_eq!(rect_of(&mut hands, "wide"), (10.0, 60.0, 380.0, 20.0));
    assert_eq!(rect_of(&mut hands, "two"), (10.0, 90.0, 260.0, 20.0));
    assert_eq!(rect_of(&mut hands, "there"), (340.0, 90.0, 50.0, 20.0));
}

#[test]
fn translate_moves_where_something_is_clicked_as_well_as_where_it_shows() {
    let mut hands = open(
        "translate",
        r#"
        import { useState } from 'rsui';
        export default function App() {
            const [count, setCount] = useState(0);
            return (
                <window>
                    <button key="go" width={40} height={20} translateX={100} translateY={100} onClick={() => setCount(count + 1)}>Go</button>
                    <text key="count">{'Count ' + count}</text>
                </window>
            );
        }
        "#,
    );
    assert_eq!(rect_of(&mut hands, "go"), (100.0, 100.0, 40.0, 20.0));
    // Where it was before it was moved has nothing there now.
    hands.click_at(10.0, 10.0);
    assert!(hands.says("Count 0"), "{}", hands.app.outline());
    hands.click_at(110.0, 110.0);
    assert!(hands.says("Count 1"), "{}", hands.app.outline());
}

#[test]
fn none_of_this_clutters_what_an_agent_reads() {
    let hands = open(
        "outline",
        r#"
        export default function App() {
            return (
                <window display="grid" columns="1fr 1fr" columnGap={2} rowGap={3} justifyItems="center">
                    <group key="a" marginTop={1} position="absolute" top={1} left={2} right={3} bottom={4} translateX={5} translateY={6} />
                    <group key="b" gridColumn="1 / 2" gridRow={2} />
                </window>
            );
        }
        "#,
    );
    let outline = hands.app.outline();
    for name in [
        "margin",
        "position",
        "top=",
        "left=",
        "translate",
        "display",
        "columns",
        "grid_",
        "column_gap",
        "row_gap",
        "justify_items",
    ] {
        assert!(!outline.contains(name), "{name} is in\n{outline}");
    }
}
