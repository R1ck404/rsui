//! Focus reveal follows the rendered containing blocks, including viewport-fixed content.

mod hands;

use hands::Hands;
use std::path::PathBuf;

fn open(name: &str, content: &str) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tsx-focus-scrolling-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let source = format!(
        "import {{ PageWindow, dom }} from 'rsui/web';\nimport * as React from 'react';\nimport './app.css';\nfunction Page() {{ return ({content}); }}\nexport default function App() {{ return <PageWindow><Page /></PageWindow>; }}"
    );
    std::fs::write(folder.join("app.tsx"), source).unwrap();
    std::fs::write(folder.join("app.css"), "body { margin: 0; }").unwrap();
    Hands::open(&folder.join("app.tsx"), (500, 400))
}

#[test]
fn focusing_a_viewport_fixed_box_does_not_move_the_document() {
    let mut hands = open(
        "fixed-focus",
        r#"
        <dom.div>
            <dom.div key="marker" style={{height: 1600}}>Document</dom.div>
            <dom.div style={{position: 'fixed', top: 20, left: 20}}>
                <dom.button key="scroll-page-trigger" onClick={() => window.scrollTo(0, 300)}>Scroll document</dom.button>
                <dom.button key="focus-panel-trigger" onClick={() => document.getElementById('panel').focus()}>Focus panel</dom.button>
            </dom.div>
            <dom.div id="panel" key="panel" tabIndex={-1}
                style={{position: 'fixed', top: 350, left: 20, width: 150, height: 260}}>Floating panel</dom.div>
        </dom.div>
    "#,
    );
    hands.click("scroll-page-trigger");
    let before = hands.rect_of("marker")[1];
    assert!(
        before < -200.0,
        "marker {before}: {}",
        hands.app.rectangles()
    );
    hands.click("focus-panel-trigger");
    assert!(
        (hands.rect_of("marker")[1] - before).abs() < 0.5,
        "focusing a viewport-fixed box must not try to move it by scrolling the document"
    );
}

#[test]
fn opening_a_fixed_popup_with_autofocus_preserves_document_scroll() {
    let mut hands = open(
        "fixed-open-autofocus",
        r#"
        (() => {
            const [opened, setOpened] = React.useState(false);
            React.useLayoutEffect(() => {
                if (opened) document.getElementById('popup').focus();
            }, [opened]);
            return <dom.div>
                <dom.div key="marker" style={{height: 1600}}>Document</dom.div>
                <dom.div style={{position: 'fixed', top: 20, left: 20}}>
                    <dom.button key="scroll-page-trigger" onClick={() => window.scrollTo(0, 300)}>Scroll document</dom.button>
                    <dom.button key="open-popup-trigger" onClick={() => setOpened(true)}>Open popup</dom.button>
                </dom.div>
                {opened ? <dom.div id="popup" key="popup" tabIndex={-1}
                    style={{position: 'fixed', top: 350, left: 20, width: 150, height: 260}}>Popup</dom.div> : null}
            </dom.div>;
        })()
    "#,
    );
    hands.click("scroll-page-trigger");
    let before = hands.rect_of("marker")[1];
    assert!(before < -200.0);
    hands.click("open-popup-trigger");
    let mut popup = Vec::new();
    assert!(hands::path_of(
        hands.app.runtime().root(),
        "popup",
        &mut popup
    ));
    assert_eq!(hands.app.focused(), Some(popup.as_slice()));
    assert!(
        (hands.rect_of("marker")[1] - before).abs() < 0.5,
        "opening and autofocusing a fixed popup must leave the document alone"
    );
}

#[test]
fn focus_still_scrolls_inside_a_fixed_box() {
    let mut hands = open(
        "fixed-inner-scroll",
        r#"
        <dom.div>
            <dom.div key="marker" style={{height: 1600}}>Document</dom.div>
            <dom.div style={{position: 'fixed', top: 20, left: 20}}>
                <dom.button key="scroll-page-trigger" onClick={() => window.scrollTo(0, 300)}>Scroll document</dom.button>
                <dom.button key="focus-panel-trigger" onClick={() => document.getElementById('target').focus()}>Focus target</dom.button>
            </dom.div>
            <dom.div style={{position: 'fixed', top: 150, left: 20, width: 150, height: 100, overflow: 'auto'}}>
                <dom.div key="inside-spacer" style={{height: 400}}>Panel content</dom.div>
                <dom.button id="target" key="inside-target">Target</dom.button>
            </dom.div>
        </dom.div>
    "#,
    );
    hands.click("scroll-page-trigger");
    let before = hands.rect_of("marker")[1];
    assert!(before < -200.0);
    assert!(hands.rect_of("inside-target")[1] > 400.0);
    hands.click("focus-panel-trigger");
    let target = hands.rect_of("inside-target");
    assert!(
        target[1] >= 150.0 && target[1] + target[3] <= 250.5,
        "focus must reveal the item within its fixed scroller: {target:?}"
    );
    assert!((hands.rect_of("marker")[1] - before).abs() < 0.5);
}

#[test]
fn fixed_content_inside_a_transformed_containing_block_can_reveal_that_block() {
    let mut hands = open(
        "transformed-fixed",
        r#"
        <dom.div>
            <dom.button key="focus-panel-trigger" onClick={() => document.getElementById('target').focus()}>Focus target</dom.button>
            <dom.div style={{height: 900}}>Document content</dom.div>
            <dom.div style={{transform: 'translateX(0px)', height: 100}}>
                <dom.button id="target" key="contained-target"
                    style={{position: 'fixed', top: 0, left: 0}}>Contained target</dom.button>
            </dom.div>
        </dom.div>
    "#,
    );
    assert!(hands.rect_of("contained-target")[1] > 800.0);
    hands.click("focus-panel-trigger");
    let target = hands.rect_of("contained-target");
    assert!(
        target[1] >= 0.0 && target[1] + target[3] <= 400.5,
        "a transform-contained fixed box moves with its document: {target:?}"
    );
}
