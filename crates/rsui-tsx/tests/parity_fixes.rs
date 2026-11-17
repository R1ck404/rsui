//! Engine causes found under shadcn/ui's resizable, data-table and chart.

mod hands;

use std::path::PathBuf;

use hands::Hands;

fn open(name: &str, css: &str, before: &str, jsx: &str, size: (u32, u32)) -> Hands {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-parity-fixes-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let css = format!("body {{ margin: 0; }}\n{css}");
    let app = format!(
        "import {{ PageWindow, dom }} from 'rsui/web';\nimport * as React from 'react';\nimport './app.css';\n{before}\nexport default function App() {{ return <PageWindow>{jsx}</PageWindow>; }}\n"
    );
    std::fs::write(folder.join("app.tsx"), app).unwrap();
    std::fs::write(folder.join("app.css"), css).unwrap();
    let mut hands = Hands::open(&folder.join("app.tsx"), size);
    hands.settle();
    hands
}

/// CSS 2.1 §10.5 and CSS Flexbox §9.8: a percentage height is of its containing block's height when that's definite, and a
/// scroller's is however much of it shows, not its content. react-resizable-panels puts a panel's content in a flex item that
/// scrolls (`overflow: auto`, stretched, so definite), and shadcn's resizable-demo has an `h-full` group in it, which was 1px.
#[test]
fn a_share_of_a_scrollers_height_is_of_what_shows() {
    let css = ".plain { height: 200px; width: 100px; overflow: auto; }
.full { height: 100%; }
.half { height: 50%; }
.row { display: flex; height: 200px; width: 300px; }
.panel { display: flex; flex: 1 1 0; }
.scroller { flex-grow: 1; max-height: 100%; max-width: 100%; overflow: auto; }
.column { height: 100%; display: flex; flex-direction: column; }
.top { flex: 25 1 0; } .bottom { flex: 75 1 0; }
.tall { height: 500px; }
.capped { max-height: 200px; overflow: auto; }";
    let jsx = r#"<dom.div>
        <Toggle />
        <dom.div key="plain2" className="plain"><dom.div key="half" className="half" /><dom.div key="tall" className="tall" /></dom.div>
        <dom.div key="capped" className="capped"><dom.div key="none" className="full" /><dom.div key="tall2" className="tall" /></dom.div>
        <dom.div key="row" className="row"><dom.div key="panel" className="panel"><dom.div key="scroller" className="scroller">
            <dom.div key="column" className="column"><dom.div key="top" className="top" /><dom.div key="bottom" className="bottom" /></dom.div>
        </dom.div></dom.div></dom.div>
    </dom.div>"#;
    let before = r#"function Toggle() {
        const [taller, setTaller] = React.useState(false);
        return <dom.div>
            <dom.button key="taller" onClick={() => setTaller(true)}>taller</dom.button>
            <dom.div key="plain" className="plain" style={taller ? { height: 300 } : undefined}><dom.div key="full" className="full" /></dom.div>
        </dom.div>;
    }"#;
    let mut hands = open("scroller-share", css, before, jsx, (400, 900));
    assert_eq!(hands.rect_of("full")[3], 200.0, "height: 100% in a 200px scroller");
    assert_eq!(hands.rect_of("half")[3], 100.0, "height: 50% beside something taller than the scroller");
    assert_eq!(hands.rect_of("none")[3], 0.0, "a scroller as tall as what's in it (up to its max-height) has no height to be a share of");
    assert_eq!(hands.rect_of("column")[3], 200.0, "height: 100% in a stretched flex item that scrolls");
    assert_eq!(hands.rect_of("top")[3], 50.0);
    assert_eq!(hands.rect_of("bottom")[3], 150.0);
    // And of how tall it is now, when that changes and nothing in it does.
    hands.click("taller");
    assert_eq!(hands.rect_of("full")[3], 300.0, "height: 100% in a scroller made 300px");
}


/// React renders a component again when what it's in does, unless it's given the very same props object (its element as it was)
/// or it's `memo`'s: TanStack Table v9 gives a cell's component the same memoized context each time (`flexRender(def.cell,
/// cell.getContext())`, whose `row` says it's selected once it is), and the row's checkbox kept its old state.
#[test]
fn a_component_given_the_same_objects_is_rendered_again() {
    let before = r#"const store = { selected: false };
const context = { row: { isSelected: () => store.selected } };
let memoRenders = 0;
function Cell({ row }) {
    return <dom.span key="cell">{row.isSelected() ? 'cell on' : 'cell off'}</dom.span>;
}
function Holder({ row }) {
    return <dom.span key="holder">{row.isSelected() ? 'holder on' : 'holder off'}</dom.span>;
}
const Memo = React.memo(function Memo({ row }) {
    memoRenders += 1;
    return <dom.span key="memo">{'memo ' + memoRenders}</dom.span>;
});
function Table() {
    const [, setVersion] = React.useState(0);
    return <dom.div>
        <dom.button key="toggle" onClick={() => { store.selected = !store.selected; setVersion((version) => version + 1); }}>toggle</dom.button>
        <dom.div>{React.createElement(Cell, context)}</dom.div>
        <dom.div><Holder row={context.row} /></dom.div>
        <Memo row={context.row} />
    </dom.div>;
}"#;
    let mut hands = open("same-objects", "", before, "<Table />", (400, 300));
    assert!(hands.says("cell off") && hands.says("holder off"), "{}", hands.app.outline());
    let memo = |hands: &Hands| hands.app.outline().lines().find(|line| line.contains("#memo")).unwrap_or_default().to_owned();
    let memo_before = memo(&hands);
    hands.click("toggle");
    assert!(hands.says("cell on"), "createElement with the same object: {}", hands.app.outline());
    assert!(hands.says("holder on"), "a prop that's the same object: {}", hands.app.outline());
    assert_eq!(memo(&hands), memo_before, "memo compares its props");
}
