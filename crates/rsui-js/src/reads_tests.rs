//! What a component read as it rendered, and whether new props hold what it read: see `reads.rs`.

use crate::Engine;
use crate::value::Value;

fn engine_with(source: &str) -> Engine {
    let file = std::env::temp_dir().join("app").join("reads.jsx");
    let mut engine = Engine::new();
    engine.eval(source, file.to_str().unwrap()).unwrap();
    engine
}

fn global(engine: &mut Engine, name: &str) -> Value {
    engine.vm().get_global(name).unwrap()
}

/// Renders the component named `component` with the props `props` names, as rsui-tsx does, and gives what it read.
fn render(engine: &mut Engine, owner: u32, component: &str, props: &str) -> u32 {
    render_with(engine, owner, component, props, false)
}

/// [`render`], cautiously or not, as rsui-tsx renders a component that didn't say `memo` cautiously.
fn render_with(engine: &mut Engine, owner: u32, component: &str, props: &str, cautious: bool) -> u32 {
    let function = global(engine, component);
    let props = global(engine, props);
    let vm = engine.vm();
    let id = vm.start_reads(owner, function, props, true, cautious);
    vm.call_function(function, Value::UNDEFINED, &[props]).unwrap();
    vm.end_reads();
    id
}

fn same(engine: &mut Engine, id: u32, props: &str) -> bool {
    let props = global(engine, props);
    let vm = engine.vm();
    vm.reads_same(id, props, Value::UNDEFINED)
}

fn call(engine: &mut Engine, name: &str) -> String {
    let function = global(engine, name);
    let vm = engine.vm();
    let said = vm.call_function(function, Value::UNDEFINED, &[]).unwrap();
    vm.plain_text(said).unwrap_or_else(|| "?".to_owned())
}

const PANE: &str = r#"
    function Pane({ session, id }) {
        const own = session.views[id];
        return own ? own.view : 'list';
    }
    const views = { a: { view: 'grid' }, b: { view: 'details' } };
    globalThis.Pane = Pane;
    globalThis.first = { session: { tabs: 1, views }, id: 'a' };
    globalThis.tabsMoved = { session: { tabs: 2, views }, id: 'a' };
    globalThis.otherViewChanged = { session: { tabs: 3, views: { ...views, b: { view: 'grid' } } }, id: 'a' };
    globalThis.ownViewChanged = { session: { tabs: 1, views: { ...views, a: { view: 'list' } } }, id: 'a' };
    globalThis.otherPane = { session: { tabs: 1, views }, id: 'b' };
"#;

#[test]
fn a_component_given_a_session_that_changed_only_where_it_didnt_read_is_the_same() {
    let mut engine = engine_with(PANE);
    let id = render(&mut engine, 1, "Pane", "first");
    assert!(same(&mut engine, id, "tabsMoved"));
    // Another folder's view is in what it was given, but it only read its own.
    assert!(same(&mut engine, id, "otherViewChanged"));
    assert!(!same(&mut engine, id, "ownViewChanged"));
    assert!(!same(&mut engine, id, "otherPane"));
}

const CONDITIONAL: &str = r#"
    function Shown(props) { return props.open ? props.a : props.b; }
    globalThis.Shown = Shown;
    globalThis.closed = { open: false, a: 1, b: 2 };
    globalThis.closedOtherA = { open: false, a: 5, b: 2 };
    globalThis.closedOtherB = { open: false, a: 1, b: 5 };
    globalThis.opened = { open: true, a: 1, b: 2 };
    globalThis.openedOtherB = { open: true, a: 1, b: 7 };
"#;

#[test]
fn what_wasnt_read_this_time_doesnt_count_and_each_render_notes_its_own() {
    let mut engine = engine_with(CONDITIONAL);
    let id = render(&mut engine, 1, "Shown", "closed");
    assert!(same(&mut engine, id, "closedOtherA"));
    assert!(!same(&mut engine, id, "closedOtherB"));
    assert!(!same(&mut engine, id, "opened"));
    engine.vm().drop_reads(id);
    // Rendered again, open, it reads `a` and not `b`.
    let id = render(&mut engine, 1, "Shown", "opened");
    assert!(same(&mut engine, id, "openedOtherB"));
}

const WHOLE: &str = r#"
    function Passing({ session }) { return { type: 'Child', props: { session } }; }
    function Spreading(props) { return { ...props.session }; }
    function Keys(props) { return Object.keys(props.session).length; }
    function Reading({ session }) { return session.views; }
    const views = {};
    Object.assign(globalThis, { Passing, Spreading, Keys, Reading });
    globalThis.first = { session: { tabs: 1, views } };
    globalThis.tabsMoved = { session: { tabs: 2, views } };
    globalThis.sameAgain = { session: { tabs: 1, views } };
"#;

#[test]
fn what_is_given_on_spread_or_listed_is_read_whole() {
    let mut engine = engine_with(WHOLE);
    for component in ["Passing", "Spreading", "Keys"] {
        let id = render(&mut engine, 1, component, "first");
        assert!(!same(&mut engine, id, "tabsMoved"), "{component}");
    }
    // Given whole to an element, the same again is still the same, as it would be for the element's own props.
    let id = render(&mut engine, 1, "Passing", "first");
    assert!(same(&mut engine, id, "sameAgain"));
    let id = render(&mut engine, 1, "Reading", "first");
    assert!(same(&mut engine, id, "tabsMoved"));
}

const CAPTURED: &str = r#"
    function Menu({ session }) {
        globalThis.later = () => 'tabs ' + session.tabs;
        return session.views;
    }
    function Label({ session }) {
        const label = () => 'tabs ' + session.tabs;
        return label();
    }
    const views = {};
    Object.assign(globalThis, { Menu, Label });
    globalThis.first = { session: { tabs: 1, views } };
    globalThis.tabsMoved = { session: { tabs: 2, views } };
"#;

#[test]
fn a_closure_made_as_it_rendered_sees_the_new_props_once_skipped_and_what_it_read_counts() {
    let mut engine = engine_with(CAPTURED);
    let id = render(&mut engine, 1, "Menu", "first");
    assert_eq!(call(&mut engine, "later"), "tabs 1");
    assert!(same(&mut engine, id, "tabsMoved"));
    // What it made as it rendered, as a menu's handler, does what one the new props made would.
    assert_eq!(call(&mut engine, "later"), "tabs 2");
    // A closure called as it renders reads what it reads for it.
    let id = render(&mut engine, 1, "Label", "first");
    assert!(!same(&mut engine, id, "tabsMoved"));
}

const DEPENDENCIES: &str = r#"
    function useEffect(run, deps) {}
    function Watching({ session }) { useEffect(() => {}, [session]); return 1; }
    function WatchingViews({ session }) { useEffect(() => {}, [session.views]); return 1; }
    const views = { a: 1 };
    Object.assign(globalThis, { Watching, WatchingViews });
    globalThis.first = { session: { tabs: 1, views } };
    globalThis.tabsMoved = { session: { tabs: 2, views } };
    globalThis.viewsMadeAgain = { session: { tabs: 1, views: { a: 1 } } };
"#;

#[test]
fn a_hooks_dependencies_are_read_as_themselves() {
    let mut engine = engine_with(DEPENDENCIES);
    let id = render(&mut engine, 1, "Watching", "first");
    assert!(!same(&mut engine, id, "tabsMoved"));
    let id = render(&mut engine, 1, "WatchingViews", "first");
    assert!(same(&mut engine, id, "tabsMoved"));
    // Made again with the same in them, an effect that depends on it runs again, as it would in React.
    assert!(!same(&mut engine, id, "viewsMadeAgain"));
}

const ARRAYS: &str = r#"
    function First(props) { return props.list[0]; }
    function All(props) { return props.list.map((one) => one * 2); }
    function Count(props) { return props.list.length; }
    Object.assign(globalThis, { First, All, Count });
    globalThis.first = { list: [1, 2, 3] };
    globalThis.secondChanged = { list: [1, 5, 3] };
    globalThis.longer = { list: [1, 2, 3, 4] };
"#;

#[test]
fn an_element_read_by_its_index_is_read_alone_and_a_map_reads_them_all() {
    let mut engine = engine_with(ARRAYS);
    let id = render(&mut engine, 1, "First", "first");
    assert!(same(&mut engine, id, "secondChanged"));
    let id = render(&mut engine, 1, "All", "first");
    assert!(!same(&mut engine, id, "secondChanged"));
    let id = render(&mut engine, 1, "Count", "first");
    assert!(same(&mut engine, id, "secondChanged"));
    assert!(!same(&mut engine, id, "longer"));
}

const RENDERING_FUNCTIONS: &str = r#"
    function app(focus, other) {
        const view = { focus, other };
        return { pane: (id) => (id === view.focus ? 'active ' + id : id), extra: other };
    }
    function Slot({ pane, id }) { return pane(id); }
    function Around(props) { return { type: 'Slot', props: { ...props } }; }
    Object.assign(globalThis, { Slot, Around, app });
    const one = app('a', 1);
    globalThis.first = { pane: one.pane, id: 'b' };
    globalThis.otherChanged = { pane: app('a', 2).pane, id: 'b' };
    globalThis.focusMoved = { pane: app('c', 1).pane, id: 'b' };
    globalThis.focusHere = { pane: app('b', 1).pane, id: 'b' };
    globalThis.aroundFirst = { pane: one.pane, id: 'b' };
    globalThis.aroundMoved = { pane: app('b', 1).pane, id: 'b' };
    globalThis.callFirst = () => globalThis.aroundFirst.pane('b');
    globalThis.aroundAgain = { pane: app('c', 1).pane, id: 'b' };
    globalThis.aroundSame = { pane: app('a', 1).pane, id: 'b' };
    globalThis.aroundOnceMore = { pane: app('b', 3).pane, id: 'b' };
"#;

#[test]
fn a_function_called_as_it_renders_is_the_same_while_what_it_read_is() {
    let mut engine = engine_with(RENDERING_FUNCTIONS);
    let id = render(&mut engine, 1, "Slot", "first");
    assert!(same(&mut engine, id, "otherChanged"));
    // What `focus` is was only compared with this slot's own, as `id === view.focus`: moving it between two others is the
    // same for this one, and moving it here isn't.
    assert!(same(&mut engine, id, "focusMoved"));
    assert!(!same(&mut engine, id, "focusHere"));
}

#[test]
fn what_called_a_function_handed_on_is_looked_at_again() {
    let mut engine = engine_with(RENDERING_FUNCTIONS);
    // `Slot` read the function `Around` handed on, as a split's pane does below the split.
    let slot = render(&mut engine, 7, "Slot", "aroundFirst");
    let around = render(&mut engine, 8, "Around", "aroundFirst");
    assert!(same(&mut engine, around, "aroundMoved"));
    assert_eq!(engine.vm().take_unheld_readers(), vec![7]);
    // The function the slot has does what the new one does.
    assert_eq!(call(&mut engine, "callFirst"), "active b");
    let _ = slot;
}

#[test]
fn a_function_called_by_something_whose_reads_arent_noted_isnt_handed_on() {
    let mut engine = engine_with(RENDERING_FUNCTIONS);
    let function = global(&mut engine, "Slot");
    let props = global(&mut engine, "aroundFirst");
    let vm = engine.vm();
    vm.start_reads(9, function, props, false, false);
    vm.call_function(function, Value::UNDEFINED, &[props]).unwrap();
    vm.end_reads();
    let around = render(&mut engine, 8, "Around", "aroundFirst");
    assert!(!same(&mut engine, around, "aroundMoved"));
    assert_eq!(call(&mut engine, "callFirst"), "b");
}

const COMPARED: &str = r#"
    function Row(props) { return props.id === props.selected ? 'on' : 'off'; }
    function Both(props) { return props.a === props.b; }
    Object.assign(globalThis, { Row, Both });
    globalThis.c = { id: 'c', selected: 'a' };
    globalThis.cMoved = { id: 'c', selected: 'b' };
    globalThis.cHere = { id: 'c', selected: 'c' };
    globalThis.apart = { a: 1, b: 2 };
    globalThis.together = { a: 3, b: 3 };
"#;

#[test]
fn what_is_only_compared_is_the_same_while_the_comparison_comes_out_the_same() {
    let mut engine = engine_with(COMPARED);
    // The selection moved between two other rows: this one's `id === selected` is false all the same.
    let id = render(&mut engine, 1, "Row", "c");
    assert!(same(&mut engine, id, "cMoved"));
    assert!(!same(&mut engine, id, "cHere"));
    // Only one side of a comparison is noted that way, as both changing could make them equal.
    let id = render(&mut engine, 1, "Both", "apart");
    assert!(!same(&mut engine, id, "together"));
}

#[test]
fn what_renders_next_holds_the_function_what_called_it_has() {
    let mut engine = engine_with(RENDERING_FUNCTIONS);
    let _slot = render(&mut engine, 7, "Slot", "aroundFirst");
    let around = render(&mut engine, 8, "Around", "aroundFirst");
    assert!(same(&mut engine, around, "aroundAgain"));
    assert!(engine.vm().take_unheld_readers().is_empty());
    // What rsui-tsx keeps as the props now is what it was given, and compared whole against what comes after it, as it is
    // first, what's handed on has to reach the function the slot has.
    let kept = global(&mut engine, "aroundAgain");
    let newer = global(&mut engine, "aroundOnceMore");
    let vm = engine.vm();
    assert!(vm.props_equivalent_handing_on(kept, newer, true, Value::UNDEFINED));
    assert_eq!(vm.take_unheld_readers(), vec![7]);
    assert_eq!(call(&mut engine, "callFirst"), "active b");
}

#[test]
fn a_function_old_to_the_collector_is_handed_on_all_the_same() {
    let mut engine = engine_with(RENDERING_FUNCTIONS);
    let _slot = render(&mut engine, 7, "Slot", "aroundFirst");
    let around = render(&mut engine, 8, "Around", "aroundFirst");
    // Kept through two collections, what's in it is old, which the collector marks on each object.
    engine.vm().collect_garbage_now();
    engine.vm().collect_garbage_now();
    assert!(same(&mut engine, around, "aroundMoved"));
    assert_eq!(engine.vm().take_unheld_readers(), vec![7]);
}

#[test]
fn a_function_kept_below_follows_one_that_did_the_same_kept_above() {
    let mut engine = engine_with(RENDERING_FUNCTIONS);
    let _slot = render(&mut engine, 7, "Slot", "aroundFirst");
    let first = global(&mut engine, "aroundFirst");
    let again = global(&mut engine, "aroundSame");
    let moved = global(&mut engine, "aroundMoved");
    let vm = engine.vm();
    // What's above is given a function that does the same, and keeps it, as a page's element does once it renders, while
    // the slot below keeps the one it read.
    assert!(vm.props_equivalent_handing_on(first, again, true, Value::UNDEFINED));
    assert!(vm.take_unheld_readers().is_empty());
    // Then what's above is handed one that does something else: the slot's has to follow it.
    assert!(vm.props_equivalent_handing_on(again, moved, true, Value::UNDEFINED));
    assert_eq!(vm.take_unheld_readers(), vec![7]);
    assert_eq!(call(&mut engine, "callFirst"), "active b");
}

const MODEL: &str = r#"
    function model(n) {
        return { n, add: () => n + 1 };
    }
    function Bar(props) {
        return { type: 'button', props: { onClick: props.model.add } };
    }
    globalThis.Bar = Bar;
    globalThis.first = { model: model(1) };
    globalThis.second = { model: model(2) };
    globalThis.drawn = { onClick: first.model.add };
    globalThis.drawnNow = { onClick: second.model.add };
    globalThis.clickDrawn = () => String(drawn.onClick());
"#;

#[test]
fn what_read_a_handler_from_an_object_with_methods_is_looked_at_again_once_and_left_as_it_is() {
    let mut engine = engine_with(MODEL);
    // Given a new model, which it read a function from and so is only the same as itself, it renders again.
    let first = render_with(&mut engine, 7, "Bar", "first", true);
    assert!(!same(&mut engine, first, "second"));
    engine.vm().drop_reads(first);
    let _again = render_with(&mut engine, 7, "Bar", "second", true);
    // The button it drew keeps the handler it was first given, which is handed the new one. What read that, with the props
    // it has now, reads the same: handing on changes nothing else of the model.
    let (drawn, now) = (global(&mut engine, "drawn"), global(&mut engine, "drawnNow"));
    let vm = engine.vm();
    assert!(vm.page_props_equivalent_handing_on(drawn, now, Value::UNDEFINED));
    assert!(vm.take_unheld_readers().is_empty());
    // The button's handed the same one each time what drew it renders, which it already does what that does.
    let key = vm.atom("onClick");
    let (old, new) = (vm.get_property(drawn, key).unwrap(), vm.get_property(now, key).unwrap());
    assert!(!vm.join_handed(old, new));
    assert!(vm.page_props_equivalent_handing_on(drawn, now, Value::UNDEFINED));
    assert!(vm.take_unheld_readers().is_empty());
    assert_eq!(call(&mut engine, "clickDrawn"), "3");
}

const SHARING: &str = r#"
    function make(n) {
        return () => n;
    }
    globalThis.made = [];
    globalThis.makePair = (i) => {
        globalThis.drawn = { onClick: make(i) };
        globalThis.drawnNow = { onClick: make(i + 1) };
        made.push(drawn, drawnNow);
    };
    globalThis.kept = [];
    globalThis.keepSome = () => {
        for (let i = 0; i < 4096; i++) kept.push({ i });
    };
    globalThis.letGo = () => {
        globalThis.drawn = null;
        globalThis.drawnNow = null;
        globalThis.made = [];
    };
    globalThis.clickDrawn = () => String(drawn.onClick());
"#;

/// Makes `count` handlers from `from` on, each handed on to one that sees the next number.
fn hand_on_pairs(engine: &mut Engine, from: i32, count: i32) {
    let make = global(engine, "makePair");
    for i in from..from + count {
        let vm = engine.vm();
        vm.call_function(make, Value::UNDEFINED, &[Value::int(i)]).unwrap();
        let (drawn, now) = (global(engine, "drawn"), global(engine, "drawnNow"));
        assert!(engine.vm().props_equivalent_handing_on(drawn, now, true, Value::UNDEFINED));
    }
}

#[test]
fn closures_that_shared_what_they_captured_are_forgotten_once_they_have_gone() {
    let mut engine = engine_with(SHARING);
    hand_on_pairs(&mut engine, 0, 20_000);
    // What's kept after them stops the list of objects being cut short, so that their places are given back.
    call(&mut engine, "keepSome");
    call(&mut engine, "letGo");
    engine.collect_garbage_when_quiet();
    assert!(engine.footprint().objects.given_back_pages > 100, "{}", engine.footprint());
    // Closures made now are given the blocks of captured variables those had, and handing on looks for what shares
    // theirs, which mustn't be the closures that have gone.
    hand_on_pairs(&mut engine, 20_000, 20_000);
    assert_eq!(call(&mut engine, "clickDrawn"), "40000");
}

const PAINTED: &str = r#"
    function paint(look, selected) {
        const c = look.colors;
        if (selected) {
            return look.selection === 'filled' ? c.select : c.soft;
        }
        return look.zebra ? c.zebra : c.text;
    }
    function shade(look) { return look; }
    function Row({ look, selected }) { return paint(look, selected); }
    function Kept({ look }) { return shade(look); }
    function Native({ look }) { return Object.keys(look).length; }
    Object.assign(globalThis, { Row, Kept, Native });
    const colors = { select: 'blue', soft: 'grey', zebra: 'white', text: 'black' };
    globalThis.plain = { look: { colors, selection: 'filled', zebra: false }, selected: false };
    globalThis.subtle = { look: { colors, selection: 'subtle', zebra: false }, selected: false };
    globalThis.striped = { look: { colors, selection: 'filled', zebra: true }, selected: false };
    globalThis.picked = { look: { colors, selection: 'filled', zebra: false }, selected: true };
    globalThis.pickedSubtle = { look: { colors, selection: 'subtle', zebra: false }, selected: true };
"#;

#[test]
fn what_a_function_given_part_of_the_props_reads_of_it_is_noted_where_that_was_read() {
    let mut engine = engine_with(PAINTED);
    let id = render(&mut engine, 1, "Row", "plain");
    // How a selected row is drawn changed, which this one, not selected, never read.
    assert!(same(&mut engine, id, "subtle"));
    assert!(!same(&mut engine, id, "striped"));
    let id = render(&mut engine, 1, "Row", "picked");
    assert!(!same(&mut engine, id, "pickedSubtle"));
    // What a function gives back as it is, or a native goes through, is read as itself, as before.
    for component in ["Kept", "Native"] {
        let id = render(&mut engine, 1, component, "plain");
        assert!(!same(&mut engine, id, "subtle"), "{component}");
    }
}

const GIVEN_ON: &str = r#"
    globalThis.__jsxMemo = (type, props) => ({ type, props });
    function Icon({ look }) { return look.stroke; }
    function Row({ look, name }) { return <Icon look={look} label={name} />; }
    function Twice({ look }) { return [<Icon look={look} />, <Icon look={look} />]; }
    Object.assign(globalThis, { Icon, Row, Twice });
    const colors = { text: 'black' };
    globalThis.first = { look: { stroke: 1, colors }, name: 'a' };
    globalThis.recolored = { look: { stroke: 1, colors: { text: 'white' } }, name: 'a' };
    globalThis.thicker = { look: { stroke: 2, colors }, name: 'a' };
"#;

/// Renders `parent` given `props`, and each component element it gives back inside it, as rsui-tsx does, linking what each
/// read to what the parent gave it. Gives what the parent read, and what each child read with the props it was given.
fn render_tree(engine: &mut Engine, parent: &str, props: &str) -> (u32, Vec<(u32, Value)>) {
    let function = global(engine, parent);
    let props = global(engine, props);
    let vm = engine.vm();
    let id = vm.start_reads(1, function, props, true, false);
    let output = vm.call_function(function, Value::UNDEFINED, &[props]).unwrap();
    let elements = vm.array_elements(output).map_or_else(|| vec![output], <[Value]>::to_vec);
    let mut children = Vec::new();
    for (at, element) in elements.into_iter().enumerate() {
        let type_name = vm.atom("type");
        let props_name = vm.atom("props");
        let child = vm.get_property(element, type_name).unwrap();
        let given = vm.get_property(element, props_name).unwrap();
        let child_id = vm.start_reads(2 + at as u32, child, given, true, false);
        vm.call_function(child, Value::UNDEFINED, &[given]).unwrap();
        vm.end_reads();
        let parent = vm.current_reads();
        vm.link_child(parent, child_id, given);
        children.push((child_id, given));
    }
    vm.end_reads();
    (id, children)
}

#[test]
fn what_is_only_given_on_to_a_child_is_the_same_while_what_the_child_read_of_it_is() {
    let mut engine = engine_with(GIVEN_ON);
    let (id, children) = render_tree(&mut engine, "Row", "first");
    // The row only gave its look to the icon, which only read its stroke.
    assert!(same(&mut engine, id, "recolored"));
    // And the icon was given the new one in its place, for when it renders next.
    let vm = engine.vm();
    let look = vm.atom("look");
    let colors = vm.atom("colors");
    let given = vm.get_property(children[0].1, look).unwrap();
    let now = vm.get_property(given, colors).unwrap();
    let text = vm.atom("text");
    let said = vm.get_property(now, text).unwrap();
    assert_eq!(vm.plain_text(said).as_deref(), Some("white"));
    let (id, _) = render_tree(&mut engine, "Row", "first");
    assert!(!same(&mut engine, id, "thicker"));
    // Given to two, both are looked at.
    let (id, _) = render_tree(&mut engine, "Twice", "first");
    assert!(same(&mut engine, id, "recolored"));
    let (id, _) = render_tree(&mut engine, "Twice", "first");
    assert!(!same(&mut engine, id, "thicker"));
}

#[test]
fn what_is_given_to_a_child_that_isnt_linked_is_read_whole() {
    let mut engine = engine_with(GIVEN_ON);
    // Rendered alone, nothing says what the icon read of it.
    let id = render(&mut engine, 1, "Row", "first");
    assert!(!same(&mut engine, id, "recolored"));
}

const LOOKED_UP: &str = r#"
    function Row({ selected, path }) { return selected.has(path) ? 'picked' : 'plain'; }
    function Named({ names, path }) { return names.get(path); }
    Object.assign(globalThis, { Row, Named });
    globalThis.first = { selected: new Set(['a']), path: 'b' };
    globalThis.otherPicked = { selected: new Set(['c']), path: 'b' };
    globalThis.picked = { selected: new Set(['b']), path: 'b' };
    globalThis.names = { names: new Map([['b', 'Bee'], ['c', 'Sea']]), path: 'b' };
    globalThis.otherNamed = { names: new Map([['b', 'Bee'], ['c', 'See']]), path: 'b' };
    globalThis.renamed = { names: new Map([['b', 'Be']]), path: 'b' };
"#;

#[test]
fn what_a_set_or_map_says_when_looked_in_is_all_that_is_read_of_it() {
    let mut engine = engine_with(LOOKED_UP);
    let id = render(&mut engine, 1, "Row", "first");
    // Another one picked, which this row isn't, isn't anything to it.
    assert!(same(&mut engine, id, "otherPicked"));
    assert!(!same(&mut engine, id, "picked"));
    let id = render(&mut engine, 1, "Named", "names");
    assert!(same(&mut engine, id, "otherNamed"));
    assert!(!same(&mut engine, id, "renamed"));
}
