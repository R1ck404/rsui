//! Where JavaScript and rsui's runtime meet.
//!
//! A JavaScript component is mounted as a [`js_component`] with the function and props in its own props, so the runtime
//! decides when it renders. When it does, this calls the function, then walks the description it returned, making
//! elements and mounting the components in it. There's only one engine to a thread, and it's kept here, as a component
//! gets nothing to reach it with but its props.
//!
//! The engine is borrowed only while JavaScript is running or a description is being read, never while something is
//! being mounted, so a component rendering inside another one finds it free.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use rsui::core::{CustomRole, Key, Props, Symbol, Text};
use rsui::layout::{Align, Justify, Placement};
use rsui::prelude::*;
use rsui::runtime::{PreviousNode, Remote};
use rsui_js::{Args, Atom, Engine, Root, Value, Vm};

use crate::props::{Gathered, Meanings, gather, plain_value};

/// `rsui`, with more of what a window's controls do after it, kept apart so each can be changed on its own.
const LAYER: &str = concat!(include_str!("layer.js"), include_str!("desktop/window.js"));
const REACT: &str = include_str!("react.js");
const REACT_DOM: &str = include_str!("react_dom.js");
const REACT_JSX: &str = include_str!("react_jsx.js");
const WEB: &str = include_str!("web.js");
/// A page's `<canvas>`, loaded the first time one's used: see `canvas.rs`.
const CANVAS: &str = include_str!("canvas.js");
/// Editing a page's elements with `contentEditable`, loaded the first time something can be edited: see `editing.js`.
const EDITING: &str = include_str!("editing.js");
/// What events as the web has them need beyond what every click does, which is only loaded when that's first wanted.
const EVENTS: &str = include_str!("events.js");
/// `rsui/desktop`, a part at a time, each loaded the first time it's used.
const DESKTOP: &str = include_str!("desktop/index.js");
const DESKTOP_CORE: &str = include_str!("desktop/core.js");
const DESKTOP_BASICS: &str = include_str!("desktop/basics.js");
const DESKTOP_SOUND: &str = include_str!("desktop/sound.js");
const DESKTOP_APP: &str = include_str!("desktop/app.js");
const DESKTOP_STORAGE: &str = include_str!("desktop/storage.js");
const DESKTOP_UPDATES: &str = include_str!("desktop/updates.js");
const DESKTOP_MORE: &str = include_str!("desktop/more.js");
const DESKTOP_PICTURES: &str = include_str!("desktop/pictures.js");
const DESKTOP_MEDIA: &str = include_str!("desktop/media.js");
/// The browser's names for what `rsui/desktop` does, for code written for a page.
const WEB_ROUTES: &str = include_str!("web_routes.js");
/// The globals a page has, `document` and `window` among them, only loaded once one's touched.
const DOM_GLOBALS: &str = include_str!("dom_globals.js");
const DOM_EVENTS: &str = include_str!("dom_events.js");
const DOM_OBSERVERS: &str = include_str!("dom_observers.js");
const DOM_STATE: &str = include_str!("dom_state.js");
/// A page's DOM as code makes and changes it, with its ranges and selection, loaded the first time code makes a node or asks
/// for one: see `dom.js`.
const DOM: &str = include_str!("dom.js");

/// What's needed from JavaScript, looked up once.
struct Internals {
    set_tracing: Root,
    describe_hooks: Root,
    restore_state: Root,
    new_instance: Root,
    new_click_event: Root,
    /// What a click handler gives back to have the click go on to what's around it, as on the web.
    pass_click: Root,
    /// What a page's element written in JSX comes to, from `rsui/web`.
    web_description: Root,
    /// Makes what a handler's given about where the pointer was and what it happened to, as the web has it.
    new_event: Root,
    /// Tells what's listening on `document` and `window`.
    document_event: Root,
    /// Makes rsui's own handlers of handlers for events as the web has them, on rsui's own elements.
    rsui_events: Root,
    desktop_event: Root,
    render: Root,
    flush_effects: Root,
    /// Runs the effects of everything `flush_effects` was given, once a round of rendering's effects have all been seen to.
    commit_effects: Root,
    unmount: Root,
    set_reloading: Root,
    pasted: Root,
    color_scheme_changed: Root,
    window_resized: Root,
    window_root: Root,
    fragment: Root,
    /// What `__jsx` puts in a component's element, as it wasn't made by the app's own JSX: see `JsProps::eq`.
    not_kept: Root,
    /// What a component gives back to be drawn in another order: see `__Reordered` in layer.js.
    reordered: Root,
    order_name: Atom,
    type_name: Atom,
    /// `className`, which says a tag rsui has too is a page's.
    class_name: Atom,
    key_name: Atom,
    props_name: Atom,
    kids_name: Atom,
    /// What `__jsxMemo` marks a component's element with (see layer.js).
    memo_name: Atom,
    function_name: Atom,
    count_name: Atom,
    visible_name: Atom,
    overscan_name: Atom,
    render_row_name: Atom,
    axis_name: Atom,
    reveal_name: Atom,
}

/// A mounted component's side of what JavaScript knows about it.
struct JsInstance {
    /// The object in `layer.js` holding its hooks.
    object: Root,
    /// Where it is in [`Host::instances`], which is how JavaScript says it has changed.
    slot: u32,
    /// Goes up whenever it has effects to run.
    effect_runs: Cell<u32>,
}

/// What an app has logged, a line each.
pub(crate) type Logs = std::rc::Rc<RefCell<Vec<String>>>;

#[derive(Default)]
struct Host {
    engine: RefCell<Option<Engine>>,
    internals: RefCell<Option<Internals>>,
    meanings: RefCell<Meanings>,
    /// What the window should be called, what to put on the clipboard, and whether something wants what's on it.
    title_wanted: RefCell<Option<String>>,
    /// What the main window was last called by the app, for asking about it where there's no window to ask.
    title_now: RefCell<Option<String>>,
    copy_wanted: RefCell<Option<String>>,
    paste_wanted: Cell<bool>,
    urls_wanted: RefCell<Vec<String>>,
    /// What dialogs get instead of coming up, when nobody's there for them, with nothing left meaning they're cancelled.
    dialog_answers: RefCell<Option<std::collections::VecDeque<Option<String>>>>,
    /// Where what's logged and what's said goes, when something's collecting it instead of the standard output.
    logs: RefCell<Option<Logs>>,
    dark_title_bar_wanted: Cell<Option<bool>>,
    /// What JavaScript has asked to have the keyboard.
    focus_wanted: RefCell<Option<(Key, Option<bool>, bool)>>,
    /// What part of a field's text JavaScript has asked to have selected, as `setSelectionRange()` does.
    select_wanted: RefCell<Option<(Key, u32, u32)>>,
    /// What JavaScript has asked to have the pointer, or `Some(None)` to let it go, as `setPointerCapture()` does.
    capture_wanted: RefCell<Option<Option<Key>>>,
    /// A list to read an element's props into, used again each time.
    read_props: RefCell<Vec<(Atom, Value)>>,
    /// Empty converted-prop scratch, capped independently of JS input size.
    native_entries: RefCell<Vec<(Symbol, rsui::core::PropValue)>>,
    /// The last few small sets of props, for elements with the same ones to share one: see [`native_props`]. It keeps no JS
    /// value or node, and no set with much text in it.
    recent_props: RefCell<RecentProps>,
    instances: RefCell<Vec<Option<State<JsInstance>>>>,
    /// Which runtime each of those belongs to, as a state only means anything to its own. There's one to a window.
    owners: RefCell<Vec<u32>>,
    /// The runtime of the main window, once it's rendered.
    main_runtime: Cell<u32>,
    /// How to reach each window's runtime.
    remotes: RefCell<Vec<(u32, Remote)>>,
    free_slots: RefCell<Vec<u32>>,
    dirty: RefCell<Vec<u32>>,
    /// What JavaScript asked for during the render that's going on.
    frame_wanted: Cell<bool>,
    /// Whether the page wants to hear when the window's been laid out: see `wantLaidOut`.
    laid_out_wanted: Cell<bool>,
    effects_wanted: Cell<bool>,
    /// Whether components have effects to run once the round of rendering that's going on has seen to all of them.
    commit_waiting: Cell<bool>,
    /// Whether a component left microtasks waiting as it rendered, which run once the round's over, rather than at whatever happens
    /// next: an `<input>` that's ticked tells what comes after it in one, for `input:checked + .icon` (see `tellPeer` in `web.js`).
    jobs_waiting: Cell<bool>,
    now_ms: Cell<f64>,
    animations_on: Cell<bool>,
    /// Whether the page's CSS has rules for when things don't move (`prefers-reduced-motion`), which held as nothing does: see
    /// [`take_motion_held`].
    motion_held: Cell<bool>,
    /// Whether what a component gives back that's more than one thing fills what it's in, for code written for a page.
    fragments_fill: Cell<bool>,
    /// Whether text has been asked to be drawn the way the system draws it, as it is on a page.
    system_glyphs_wanted: Cell<bool>,
    /// The JavaScript component whose output is being turned into elements, which is the parent of anything new in it.
    owner: RefCell<Option<Root>>,
    /// When the next JavaScript timer is due, if something's already waiting for it.
    timer_due: Cell<Option<Instant>>,
    /// Whether a timer that came due has waited a frame for what's changed to be drawn, which is as long as it waits.
    timer_waited: Cell<bool>,
    /// Whether something that happened is on its way down to what it happened to and back, from `document` hearing it on the way
    /// down until it hears it on the way back up. A timer that comes due meanwhile waits until that's over, as a browser's waits
    /// for the task that dispatches the event: Radix's DismissableLayer sets a `setTimeout(0)` as a click comes down, which
    /// must find what it noted then undone as the click goes back up.
    dispatching: Cell<bool>,
    /// Exact web-event phases which have listeners. The runtime's aggregate mask
    /// also covers compatibility mouse events, so it can request callbacks with
    /// no JavaScript work. None retains the older aggregate-only host contract.
    document_interests: RefCell<Option<std::collections::HashMap<String, u8>>>,
    /// What to show on top of the app, until the code is fixed.
    error: RefCell<Option<String>>,
    error_changed: Cell<bool>,
    root_state: Cell<Option<State<u32>>>,
    app: RefCell<Option<Root>>,
    /// Whether components are told apart by name, so a new version of one takes over from the old.
    hot: Cell<bool>,
    /// Whether a new version of the code is being swapped in, which is when a component is told apart by its name alone.
    swapping: Cell<bool>,
    remote: Arc<Mutex<Option<Remote>>>,
    /// The layer's object for each component, by its slot, and the props it was last given, for what a debugger asks
    /// about them while it's looking.
    traced: RefCell<std::collections::HashMap<u32, (Root, Root)>>,
    /// Whether the handler that's running has said nothing further along should hear about what it's handling, or that
    /// what that would do by itself shouldn't happen, as the web's `stopPropagation()` and `preventDefault()` say.
    stop_wanted: Cell<bool>,
    prevent_wanted: Cell<bool>,
    /// Whether a click on rsui's own element has been said to go on to what's around it, with `passOn()`.
    pass_wanted: Cell<bool>,
    /// What each component whose output has a long list in it was last given there, by its slot: see [`KeptKids`].
    kept_kids: RefCell<std::collections::HashMap<u32, KeptKids>>,
}

/// How long a list of children has to be for what's the same in it from one render to the next to be kept as it is.
const KEPT_FROM: usize = 64;
/// How far either side of where it's expected something kept is looked for, before every place is.
const NEAR_KEPT: isize = 4;

/// The children a component's output was last given, when that was a long list, as an editor's root is with thousands of
/// paragraphs. What's in the next list that's the same description as before, in the same place or with a key of its own, is a
/// child component that's given the same as it was, which is kept as it is, without describing it again: a list where a few have
/// been put in or taken out costs what those do. See `build_long`.
struct KeptKids {
    /// The list, held, so that what's in it is what it was.
    list: Root,
    /// Where the children are, as [`Place`] has it, which they're only the same children in if it's the same.
    scope: Place,
    /// For each of the list, the child of the output it came to and whether it had a key, if it was a child component's output
    /// by itself.
    outputs: Vec<Option<(u32, bool)>>,
}

/// Says that something wants to hear about what's dragged in from other programs, which a window then starts telling.
fn host_want_drops(_: &mut Vm, _: Args) -> Result<Value, Value> {
    rsui::runtime::set_wants_drops();
    Ok(Value::UNDEFINED)
}

fn host_pass_event(_: &mut Vm, _: Args) -> Result<Value, Value> {
    HOST.with(|host| host.pass_wanted.set(true));
    Ok(Value::UNDEFINED)
}

thread_local! {
    static HOST: Host = Host::default();
}

fn host_stop_event(_: &mut Vm, _: Args) -> Result<Value, Value> {
    HOST.with(|host| host.stop_wanted.set(true));
    Ok(Value::UNDEFINED)
}

fn host_prevent_event(_: &mut Vm, _: Args) -> Result<Value, Value> {
    HOST.with(|host| host.prevent_wanted.set(true));
    Ok(Value::UNDEFINED)
}

/// Says which kinds of thing `document` and `window` are listening for, as a number from [`rsui::runtime::listens`].
fn host_listen_document(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let kinds = args.get(vm, 0);
    let kinds = if kinds.is_number() {
        kinds.as_number() as u32
    } else {
        0
    };
    rsui::runtime::set_document_listens(kinds);
    let phases = args.get(vm, 1);
    let interests = vm.array_elements(phases).map(|phases| {
        phases
            .chunks_exact(2)
            .filter_map(|pair| {
                let name = vm.string_text(pair[0])?.into_owned();
                pair[1].is_number().then(|| (name, pair[1].as_number() as u8))
            })
            .collect()
    });
    HOST.with(|host| *host.document_interests.borrow_mut() = interests);
    Ok(Value::UNDEFINED)
}

fn document_phase_heard(
    interests: Option<&std::collections::HashMap<String, u8>>,
    kind: &str,
    phase: u8,
) -> bool {
    // Phase zero includes lifecycle notifications such as keyboardleft, rather
    // than an ordinary event propagating through the page.
    phase == 0
        || interests.is_none_or(|interests| {
            phase < 8 && interests.get(kind).is_some_and(|bits| bits & (1 << phase) != 0)
        })
}

/// Diagnostic same-executable control: preserve every legacy document phase
/// when explicitly disabled, while leaving listener registration unchanged.
fn exact_document_phases() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("RSUI_DOCUMENT_PHASES").as_deref() != Ok("0"))
}

/// How big a picture is, in its own pixels, as `[width, height]`, or undefined if it can't be read, for an `<img>`'s
/// `onLoad` and `onError`. Only the start of the file's read.
/// The change to a text field being offered to it right now, as `[inputType, data]`, for its `beforeinput` and `input`, or
/// nothing when it isn't typing that changed it.
fn host_editing(vm: &mut Vm, _args: Args) -> Result<Value, Value> {
    let Some(said) = rsui::edit_in_progress() else {
        return Ok(Value::UNDEFINED);
    };
    let kind = vm.new_string(said.input_type);
    let data = match said.data {
        Some(data) => vm.new_string(&data),
        None => Value::NULL,
    };
    Ok(vm.new_array(vec![kind, data]))
}

/// What's selected in the field that last had the keyboard, as `[domKey, anchor, caret, at]` with byte positions in its value,
/// `at` going up each time it changes, or nothing.
fn host_field_selection(vm: &mut Vm, _args: Args) -> Result<Value, Value> {
    let Some(said) = rsui::field_selection() else {
        return Ok(Value::UNDEFINED);
    };
    let key = vm.new_string(&said.key);
    let items = vec![
        key,
        Value::number(said.anchor as f64),
        Value::number(said.caret as f64),
        Value::number(said.at as f64),
    ];
    Ok(vm.new_array(items))
}

fn host_image_size(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let source = args.get(vm, 0);
    let Some(source) = vm.plain_text(source) else {
        return Ok(Value::UNDEFINED);
    };
    Ok(match rsui::render::image_size(&source) {
        Some((width, height)) => {
            let items = vec![
                Value::number(f64::from(width)),
                Value::number(f64::from(height)),
            ];
            vm.new_array(items)
        }
        None => Value::UNDEFINED,
    })
}

/// Says to the runtime what the handler that just ran said about what it was handling.
fn pass_on_what_was_said(updates: &mut Updates<'_>) {
    let (stop, prevent, pass) = HOST.with(|host| {
        (
            host.stop_wanted.take(),
            host.prevent_wanted.take(),
            host.pass_wanted.take(),
        )
    });
    if stop {
        updates.stop_propagation();
    }
    if pass {
        updates.pass_on();
    }
    if prevent {
        updates.prevent_default();
    }
}

/// What a handler written for the web is given about something that happened, from what came with it: where the pointer
/// was, which button and key, and what's along the way from the root to what it happened to.
/// The last event that was made into one for code written for the web, by its [`serial`](rsui::runtime::Event::serial): the same
/// one, for another handler, is the same object, which `newEvent` in `layer.js` keeps, so what's along the way isn't said again.
thread_local! {
    static LAST_EVENT_SERIAL: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn event_value(vm: &mut Vm, event: &rsui::runtime::Event) -> Value {
    let again = event.serial != 0 && LAST_EVENT_SERIAL.with(std::cell::Cell::get) == event.serial;
    let modifiers = &event.modifiers;
    let held = u32::from(modifiers.ctrl)
        | u32::from(modifiers.shift) << 1
        | u32::from(modifiers.alt) << 2
        | u32::from(modifiers.meta) << 3;
    let key = event.key.as_ref();
    let numbers = [
        f64::from(event.x),
        f64::from(event.y),
        f64::from(event.screen_x),
        f64::from(event.screen_y),
        f64::from(event.movement_x),
        f64::from(event.movement_y),
        f64::from(event.button),
        f64::from(event.buttons),
        f64::from(held),
        f64::from(event.detail),
        f64::from(event.phase),
        f64::from(event.delta_x),
        f64::from(event.delta_y),
        f64::from(key.map_or(0, |key| key.location)),
        f64::from(u8::from(key.is_some_and(|key| key.repeat))),
        f64::from(u8::from(key.is_some_and(|key| key.composing))),
        event.current.len() as f64,
        event.target.len() as f64,
        event.serial as f64,
    ];
    let numbers = numbers
        .iter()
        .map(|number| Value::number(*number))
        .collect();
    let numbers = vm.new_array(numbers);
    let text =
        |vm: &mut Vm, text: Option<&str>| text.map_or(Value::UNDEFINED, |text| vm.new_string(text));
    let key_text = text(vm, key.map(|key| key.key.as_str()));
    let code = text(vm, key.map(|key| key.code));
    let data = text(vm, event.data.as_deref());
    let chain = if again {
        Value::UNDEFINED
    } else {
        chain_value(vm, event)
    };
    let files = if event.files.is_empty() {
        Value::UNDEFINED
    } else {
        let paths = event.files.iter().map(|path| vm.new_string(path)).collect();
        vm.new_array(paths)
    };
    let make = with_internals(|internals| internals.new_event.clone());
    match vm.call_function(
        make.get(),
        Value::UNDEFINED,
        &[numbers, key_text, code, data, chain, files],
    ) {
        Ok(made) => {
            LAST_EVENT_SERIAL.with(|last| last.set(event.serial));
            made
        }
        Err(_) => {
            LAST_EVENT_SERIAL.with(|last| last.set(0));
            Value::UNDEFINED
        }
    }
}

/// What's along the way from the root to what something happened to, each as
/// `[domKey, role, class, id, value, name, x, y, width, height, data, key, bare]`, for what's written for the web to find the
/// elements it knows, and to say what the rest are.
fn chain_value(vm: &mut Vm, event: &rsui::runtime::Event) -> Value {
    let Some(root) = &event.root else {
        return Value::UNDEFINED;
    };
    let mut items = Vec::with_capacity(event.target.len() + 1);
    let mut node = root;
    for depth in 0..=event.target.len() {
        if depth > 0 {
            match node.children.get(event.target[depth - 1] as usize) {
                Some(child) => node = child,
                None => break,
            }
        }
        let rect = event.boxes.get(depth).copied().unwrap_or_default();
        items.push(node_value(vm, node, rect));
    }
    vm.new_array(items)
}

pub(crate) fn node_value(vm: &mut Vm, node: &rsui::core::Element, rect: [f32; 4]) -> Value {
    use rsui::core::{PropValue, symbol};
    let text = |vm: &mut Vm, value: Option<&PropValue>| match value {
        Some(PropValue::Str(text)) => vm.new_string(text),
        Some(PropValue::Int(number)) => Value::number(*number as f64),
        Some(PropValue::Float(number)) => Value::number(*number),
        Some(PropValue::Bool(on)) => Value::bool(*on),
        None => Value::UNDEFINED,
    };
    let props = &node.props;
    let dom_key = match (props.get_symbol(symbol!("dom_key")), &node.key) {
        (Some(value), _) => text(vm, Some(value)),
        // What has a `ref` has its own key for one.
        (None, Some(Key::Str(key))) if key.starts_with("__dom") => vm.new_string(key),
        _ => Value::UNDEFINED,
    };
    let role = vm.new_string(node.role.name());
    let class = text(vm, props.get_symbol(symbol!("class")));
    let id = text(vm, props.get_symbol(symbol!("__id")));
    let value = text(vm, props.get_symbol(symbol!("value")));
    let name = if node.name.is_empty() {
        Value::UNDEFINED
    } else {
        vm.new_string(&node.name)
    };
    let mut data = Vec::new();
    for (prop, value) in props.iter() {
        if let Some(name) = prop.as_str().strip_prefix("data-") {
            data.push(vm.new_string(name));
            data.push(text(vm, Some(value)));
        }
    }
    let data = if data.is_empty() {
        Value::UNDEFINED
    } else {
        vm.new_array(data)
    };
    // Just as long as it is, as the list it's made into keeps what room this has.
    let mut items = Vec::with_capacity(13);
    items.extend([dom_key, role, class, id, value, name]);
    items.extend(rect.iter().map(|number| Value::number(f64::from(*number))));
    items.push(data);
    // Its own key, and whether it has nothing to say for itself, which tell what's only there to lay out what's in it.
    items.push(match &node.key {
        Some(Key::Str(key)) => vm.new_string(key),
        Some(Key::Sym(key)) => vm.new_string(key.as_str()),
        Some(Key::Int(key)) => Value::number(*key as f64),
        None => Value::UNDEFINED,
    });
    items.push(Value::bool(props.is_empty()));
    vm.new_array(items)
}

/// Starts a part of what the engine counts (rsui-js's `counts`), named `label`, if the app is there.
#[cfg(feature = "counts")]
pub(crate) fn count_mark(label: &str) {
    HOST.with(|host| {
        if let Ok(mut engine) = host.engine.try_borrow_mut()
            && let Some(engine) = engine.as_mut()
        {
            rsui_js::counts::mark(engine.vm(), label);
        }
    });
}

/// Runs something with the engine.
fn with_vm<R>(action: impl FnOnce(&mut Vm) -> R) -> R {
    HOST.with(|host| {
        let mut engine = host.engine.borrow_mut();
        action(engine.as_mut().expect("the app has been loaded").vm())
    })
}

/// Runs something with the engine, then lets go of whatever it made, or was given back by JavaScript, that it didn't
/// root. That's otherwise kept until JavaScript next runs from the top, as `Engine::run_jobs` has it, which never happens
/// in an app that has nothing waiting: what a click made would be kept for good, and everything it refers to with it.
/// What `action` gives back can't be a bare [`Value`], then, unless it's a number or the like.
fn with_vm_letting_go<R>(action: impl FnOnce(&mut Vm) -> R) -> R {
    with_vm(|vm| {
        let mark = vm.new_values_mark();
        let out = action(vm);
        vm.release_new_values(mark);
        out
    })
}

/// Runs something with what's been looked up from JavaScript.
fn with_internals<R>(action: impl FnOnce(&Internals) -> R) -> R {
    HOST.with(|host| {
        let internals = host.internals.borrow();
        action(internals.as_ref().expect("the app has been loaded"))
    })
}

fn report(message: String) {
    eprintln!("{message}");
    HOST.with(|host| {
        *host.error.borrow_mut() = Some(message);
        host.error_changed.set(true);
    });
}

// What JavaScript calls.

fn host_dirty(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let slot = args.get(vm, 0).as_number() as u32;
    HOST.with(|host| host.dirty.borrow_mut().push(slot));
    Ok(Value::UNDEFINED)
}

/// Has every component render again, for when something they all depend on has changed, which it says.
fn host_dirty_all(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let why = args.get(vm, 0);
    let why = vm.plain_text(why);
    HOST.with(|host| {
        let count = host.instances.borrow().len() as u32;
        host.dirty.borrow_mut().extend(0..count);
        if let Some(why) = why {
            crate::why::note_all(count, &why);
        }
    });
    Ok(Value::UNDEFINED)
}

fn host_effects(_: &mut Vm, _: Args) -> Result<Value, Value> {
    HOST.with(|host| host.effects_wanted.set(true));
    Ok(Value::UNDEFINED)
}

fn host_set_title(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let title = args.get(vm, 0);
    let title = vm.plain_text(title).unwrap_or_default();
    HOST.with(|host| {
        *host.title_now.borrow_mut() = Some(title.clone());
        *host.title_wanted.borrow_mut() = Some(title);
    });
    Ok(Value::UNDEFINED)
}

fn host_copy(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let text = args.get(vm, 0);
    let text = vm.plain_text(text).unwrap_or_default();
    HOST.with(|host| *host.copy_wanted.borrow_mut() = Some(text));
    Ok(Value::UNDEFINED)
}

fn host_open_url(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let url = args.get(vm, 0);
    let url = vm.plain_text(url).unwrap_or_default();
    if !rsui::can_open_url(&url) {
        return Err(vm.type_error(&format!(
            "Only http, https and mailto can be opened like this, as anything else could run a program: {url}"
        )));
    }
    HOST.with(|host| host.urls_wanted.borrow_mut().push(url));
    Ok(Value::UNDEFINED)
}

/// Says that what a component gives back that's more than one thing should fill what it's in, for code written for a page.
fn host_fill_fragments(_: &mut Vm, _: Args) -> Result<Value, Value> {
    HOST.with(|host| host.fragments_fill.set(true));
    Ok(Value::UNDEFINED)
}

/// Has text drawn the way the system draws it, which is how a browser draws it, for what's written for a page.
fn host_system_text(_: &mut Vm, _: Args) -> Result<Value, Value> {
    HOST.with(|host| host.system_glyphs_wanted.set(true));
    Ok(Value::UNDEFINED)
}

fn host_is_dark(_: &mut Vm, _: Args) -> Result<Value, Value> {
    // `RSUI_COLOR_SCHEME=light` or `dark` says which instead of the system, as a test comparing pages with a browser that's set to
    // one of them does.
    if let Some(scheme) = std::env::var_os("RSUI_COLOR_SCHEME") {
        return Ok(Value::bool(scheme == "dark"));
    }
    Ok(Value::bool(rsui::system_is_dark()))
}

fn host_set_dark_title_bar(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let dark = args.get(vm, 0) == Value::TRUE;
    HOST.with(|host| host.dark_title_bar_wanted.set(Some(dark)));
    Ok(Value::UNDEFINED)
}

/// An app bar as a script says it, `{ edge, size, monitor }`, or none for anything else, `null` among them.
fn app_bar_of(vm: &mut Vm, value: Value) -> Option<rsui::AppBar> {
    if !value.is_object() {
        return None;
    }
    let (edge, size, monitor) = (vm.atom("edge"), vm.atom("size"), vm.atom("monitor"));
    let edge = vm.get_property(value, edge).ok()?;
    let edge = vm.string_text(edge)?.into_owned();
    let number = |vm: &mut Vm, name| {
        vm.get_property(value, name)
            .ok()
            .filter(|value| value.is_number())
            .map(Value::as_number)
    };
    let (size, monitor) = (number(vm, size), number(vm, monitor));
    rsui::AppBar::parse(&edge, size, monitor)
}

/// A layer as a script names it, or the usual one for anything else.
fn layer_of(vm: &Vm, value: Value) -> rsui::WindowLayer {
    vm.string_text(value)
        .and_then(|name| rsui::WindowLayer::parse(&name))
        .unwrap_or_default()
}

/// `openWindow(what's kept for it in the layer, title, width, height, least width, least height, modal, owned, frameless, on top,
/// fullscreen, layer, transparent, skip taskbar, all desktops, app bar)`.
fn host_open_window(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let entry = args.get(vm, 0);
    let entry = vm.root(entry);
    let title = args.get(vm, 1);
    let title = vm.plain_text(title).unwrap_or_default();
    let pair = |vm: &mut Vm, at: usize| {
        let (first, second) = (args.get(vm, at), args.get(vm, at + 1));
        (first.is_number() && second.is_number())
            .then(|| (first.as_number() as u32, second.as_number() as u32))
    };
    let size = pair(vm, 2);
    let smallest = pair(vm, 4);
    let modal = args.get(vm, 6) == Value::TRUE;
    let owned = args.get(vm, 7) != Value::FALSE;
    let frameless = args.get(vm, 8) == Value::TRUE;
    let on_top = args.get(vm, 9) == Value::TRUE;
    let fullscreen = args.get(vm, 10) == Value::TRUE;
    let layer = args.get(vm, 11);
    let layer = layer_of(vm, layer);
    let transparent = args.get(vm, 12) == Value::TRUE;
    let skip_taskbar = args.get(vm, 13) == Value::TRUE;
    let all_desktops = args.get(vm, 14) == Value::TRUE;
    let app_bar = args.get(vm, 15);
    let app_bar = app_bar_of(vm, app_bar);
    let id = rsui::open_window(
        rsui::WindowOptions {
            title,
            size,
            smallest,
            owned,
            modal,
            frameless,
            on_top,
            fullscreen,
            layer,
            transparent,
            skip_taskbar,
            all_desktops,
            app_bar,
        },
        window_root,
        WindowEntry { entry },
    );
    Ok(Value::number(f64::from(id.0)))
}

fn host_close_window(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = args.get(vm, 0);
    if id.is_number() {
        rsui::close_window(rsui::WindowId(id.as_number() as u32));
    }
    Ok(Value::UNDEFINED)
}

fn host_set_window_title(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = args.get(vm, 0);
    let title = args.get(vm, 1);
    let title = vm.plain_text(title).unwrap_or_default();
    if id.is_number() {
        rsui::set_window_title(rsui::WindowId(id.as_number() as u32), &title);
    }
    Ok(Value::UNDEFINED)
}

/// `changeWindow(id, what, first, second, third, fourth)`.
fn host_change_window(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    use rsui::WindowChange as Change;
    let id = args.get(vm, 0);
    let what = args.get(vm, 1);
    let (first, second) = (args.get(vm, 2), args.get(vm, 3));
    let (third, fourth) = (args.get(vm, 4), args.get(vm, 5));
    let on = first == Value::TRUE;
    let number = |value: Value| value.is_number().then(|| value.as_number());
    let what = vm.string_text(what).map(|what| what.into_owned());
    let text = |vm: &mut Vm, value: Value| vm.plain_text(value).unwrap_or_default();
    let bytes = |vm: &mut Vm, value: Value| {
        let hex = vm.plain_text(value)?;
        Some(
            (0..hex.len() / 2)
                .filter_map(|at| u8::from_str_radix(&hex[at * 2..at * 2 + 2], 16).ok())
                .collect::<Vec<u8>>(),
        )
    };
    let more = match what.as_deref() {
        Some("opacity") => number(first).map(|how| Change::Opacity(how as f32)),
        Some("resizable") => Some(Change::Resizable(on)),
        Some("sizeLimits") => {
            // Either side can be limited alone, which leaves the other at 0, for not limited.
            let limit = |width: Value, height: Value| {
                let side = |value: Value| {
                    if value.is_number() {
                        value.as_number().max(0.0) as u32
                    } else {
                        0
                    }
                };
                (width.is_number() || height.is_number()).then(|| (side(width), side(height)))
            };
            Some(Change::SizeLimits(
                limit(first, second),
                limit(third, fourth),
            ))
        }
        Some("skipTaskbar") => Some(Change::SkipTaskbar(on)),
        Some("clickThrough") => Some(Change::ClickThrough(on)),
        Some("contentProtection") => Some(Change::ContentProtection(on)),
        Some("showInactive") => Some(Change::ShowWithoutActivation),
        Some("startResize") => Some(Change::StartResize(text(vm, first))),
        Some("icon") => bytes(vm, first).map(Change::Icon),
        Some("progress") => Some(Change::Progress(
            number(first).unwrap_or(0.0),
            text(vm, second),
        )),
        Some("overlay") => {
            let picture = if first.is_null() || first.is_undefined() {
                None
            } else {
                bytes(vm, first)
            };
            Some(Change::Overlay(picture, text(vm, second)))
        }
        Some("flash") => Some(Change::Flash(text(vm, first))),
        Some("backdrop") => Some(Change::Backdrop(text(vm, first))),
        Some("layer") => vm
            .string_text(first)
            .and_then(|name| rsui::WindowLayer::parse(&name))
            .map(Change::Layer),
        Some("transparent") => Some(Change::Transparent(on)),
        Some("allDesktops") => Some(Change::AllDesktops(on)),
        Some("appBar") => Some(Change::AppBar(app_bar_of(vm, first))),
        _ => None,
    };
    if let Some(change) = more {
        if id.is_number() {
            rsui::change_window(rsui::WindowId(id.as_number() as u32), change);
        }
        return Ok(Value::UNDEFINED);
    }
    let change = match what.as_deref() {
        Some("frameless") => Some(Change::Frameless(on)),
        Some("onTop") => Some(Change::OnTop(on)),
        Some("fullscreen") => Some(Change::Fullscreen(on)),
        Some("minimize") => Some(Change::Minimize),
        Some("maximize") => Some(Change::Maximize),
        Some("restore") => Some(Change::Restore),
        Some("hide") => Some(Change::Hide),
        Some("show") => Some(Change::Show),
        Some("focus") => Some(Change::Focus),
        Some("center") => Some(Change::Center),
        Some("move") => number(first)
            .zip(number(second))
            .map(|(x, y)| Change::MoveTo(x as i32, y as i32)),
        Some("resize") => number(first)
            .zip(number(second))
            .map(|(width, height)| Change::Resize(width as u32, height as u32)),
        _ => None,
    };
    if let (true, Some(change)) = (id.is_number(), change) {
        rsui::change_window(rsui::WindowId(id.as_number() as u32), change);
    }
    Ok(Value::UNDEFINED)
}

fn host_paste(_: &mut Vm, _: Args) -> Result<Value, Value> {
    HOST.with(|host| host.paste_wanted.set(true));
    Ok(Value::UNDEFINED)
}

/// `dialog(kind, title, start, [[name, patterns]...])`, which gives a promise for the path, or `null` if none was chosen.
fn host_dialog(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    use rsui::dialogs::{DialogKind, DialogOptions, ask_for_path};
    let kind = args.get(vm, 0);
    let kind = match vm.string_text(kind).as_deref() {
        Some("save") => DialogKind::SaveFile,
        Some("folder") => DialogKind::PickFolder,
        _ => DialogKind::OpenFile,
    };
    let title = args.get(vm, 1);
    let title = vm.string_text(title).map(|text| text.into_owned());
    let start = args.get(vm, 2);
    let start = vm.string_text(start).map(|text| text.into_owned());
    let filters = args.get(vm, 3);
    let filters = vm
        .array_elements(filters)
        .map(<[Value]>::to_vec)
        .unwrap_or_default();
    let filters = filters
        .into_iter()
        .filter_map(|pair| {
            let pair = vm.array_elements(pair)?;
            let (name, pattern) = (*pair.first()?, *pair.get(1)?);
            Some((
                vm.string_text(name)?.into_owned(),
                vm.string_text(pattern)?.into_owned(),
            ))
        })
        .collect();
    let answer = HOST.with(|host| {
        let mut answers = host.dialog_answers.borrow_mut();
        answers
            .as_mut()
            .map(|answers| answers.pop_front().flatten())
    });
    if let Some(answer) = answer {
        say(match &answer {
            Some(path) => format!("a dialog was asked for, and got {path}"),
            None => "a dialog was asked for, and was cancelled".to_owned(),
        });
        return Ok(vm.later(move || Ok(answer)));
    }
    Ok(vm.later(move || {
        ask_for_path(
            kind,
            DialogOptions {
                title,
                start,
                filters,
            },
        )
    }))
}

fn host_focus(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let key = args.get(vm, 0);
    let key = key_from(vm, key);
    // `focus(key, visible, preventScroll)`: the DOM focus options, preserved through the native update.
    let visible = match args.get(vm, 1) {
        value if value.is_bool() => Some(value == Value::TRUE),
        _ => None,
    };
    let prevent_scroll = args.get(vm, 2) == Value::TRUE;
    HOST.with(|host| {
        *host.focus_wanted.borrow_mut() = key.map(|key| (key, visible, prevent_scroll))
    });
    Ok(Value::UNDEFINED)
}

/// `selectText(key, start, end)`: selects that much of the text of the field with this key, in UTF-16 units, if it has the
/// keyboard by the time it's applied.
fn host_select_text(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let key = args.get(vm, 0);
    let key = key_from(vm, key);
    let unit = |value: Value| {
        if value.is_number() {
            value.as_number().max(0.0).min(f64::from(u32::MAX)) as u32
        } else {
            0
        }
    };
    let (start, end) = (unit(args.get(vm, 1)), unit(args.get(vm, 2)));
    if let Some(key) = key {
        HOST.with(|host| *host.select_wanted.borrow_mut() = Some((key, start, end)));
    }
    Ok(Value::UNDEFINED)
}

/// `capturePointer(key)`: gives the pointer to the element with this key while the main button's down, or lets it go with
/// nothing, as a page's `setPointerCapture()` and `releasePointerCapture()` do.
fn host_capture_pointer(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let key = args.get(vm, 0);
    let key = if key.is_nullish() {
        None
    } else {
        key_from(vm, key)
    };
    HOST.with(|host| *host.capture_wanted.borrow_mut() = Some(key));
    Ok(Value::UNDEFINED)
}

/// `windowScale()`: how many physical pixels there are to a logical one in the window that's drawing, as a page's
/// `devicePixelRatio` says.
fn host_window_scale(_: &mut Vm, _: Args) -> Result<Value, Value> {
    Ok(Value::number(f64::from(rsui::window_scale())))
}

/// `wantLaidOut(on)`: whether `__rsuiLaidOut` is called each time the window's been laid out.
fn host_want_laid_out(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let on = args.get(vm, 0) == Value::TRUE;
    HOST.with(|host| host.laid_out_wanted.set(on));
    Ok(Value::UNDEFINED)
}

fn host_frame(_: &mut Vm, _: Args) -> Result<Value, Value> {
    HOST.with(|host| host.frame_wanted.set(true));
    Ok(Value::UNDEFINED)
}

fn host_now(_: &mut Vm, _: Args) -> Result<Value, Value> {
    Ok(Value::number(HOST.with(|host| host.now_ms.get())))
}

/// `appName()`: what the program's own files go by.
fn host_app_name(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    let name = program_name();
    Ok(vm.new_string(&name))
}

pub(crate) fn program_name() -> String {
    let program = rsui_js::running_packed()
        .then(|| std::env::current_exe().ok())
        .flatten();
    APP_FILE.with(|file| app_name(program.as_deref(), &file.borrow()))
}

/// The app's main file.
pub(crate) fn app_file() -> PathBuf {
    APP_FILE.with(|file| file.borrow().clone())
}

/// Tells JavaScript that something's happened on the desktop, which it was waiting to hear about.
pub(crate) fn fire_desktop_event(id: u32, value: Option<String>) {
    let remote = HOST.with(|host| host.remote.lock().expect("lock is not poisoned").clone());
    send_desktop_event(remote, id, value);
}

/// The same, for another thread to tell this one's JavaScript.
pub(crate) fn desktop_events_from_anywhere() -> impl Fn(u32, Option<String>) + Send + 'static {
    let remote = HOST.with(|host| host.remote.clone());
    move |id, value| {
        let remote = remote.lock().expect("lock is not poisoned").clone();
        send_desktop_event(remote, id, value);
    }
}

fn send_desktop_event(remote: Option<Remote>, id: u32, value: Option<String>) {
    let Some(remote) = remote else {
        return;
    };
    remote.send(move |updates| {
        let event = with_internals(|internals| internals.desktop_event.clone());
        let failed = with_vm_letting_go(|vm| {
            let value = match &value {
                Some(value) => vm.new_string(value),
                None => Value::UNDEFINED,
            };
            vm.call_function(
                event.get(),
                Value::UNDEFINED,
                &[Value::number(f64::from(id)), value],
            )
            .err()
            .map(|error| vm.error_text(error))
        });
        if let Some(message) = failed {
            report(message);
        }
        pump(updates);
    });
}

/// The answer that's been said to give the next dialog, if there's one.
pub(crate) fn next_dialog_answer() -> Option<String> {
    HOST.with(|host| {
        host.dialog_answers
            .borrow_mut()
            .as_mut()
            .and_then(|answers| answers.pop_front().flatten())
    })
}

/// A packed program goes by its own name, wherever it's moved to. Its main file isn't a real place, so the folder that seems to be in
/// says nothing about which program this is. Otherwise it's the folder that the main file's in, and the file.
fn app_name(packed: Option<&Path>, main: &Path) -> String {
    let stem = |path: &Path| {
        path.file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "app".to_owned())
    };
    if let Some(program) = packed {
        return stem(program);
    }
    let folder = main
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned());
    match folder {
        Some(folder) => format!("{folder}-{}", stem(main)),
        None => stem(main),
    }
}

/// The modules that come with this that have been started, for checking what an app pays for.
/// What the main window's called: what the app last called it, or its main file's name, as it starts with.
fn host_main_title(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    let said = HOST.with(|host| host.title_now.borrow().clone());
    let title = said.unwrap_or_else(|| {
        app_file()
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    Ok(vm.new_string(&title))
}

fn host_loaded_builtins(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    let loaded = vm.loaded_builtins();
    let names: Vec<Value> = loaded.iter().map(|name| vm.new_string(name)).collect();
    Ok(vm.new_array(names))
}

fn host_animations_on(_: &mut Vm, _: Args) -> Result<Value, Value> {
    Ok(Value::bool(HOST.with(|host| host.animations_on.get())))
}

/// `motionHeld()`: the page's CSS has a rule about `prefers-reduced-motion`, worked out while nothing moves.
fn host_motion_held(_: &mut Vm, _: Args) -> Result<Value, Value> {
    HOST.with(|host| host.motion_held.set(true));
    Ok(Value::UNDEFINED)
}

/// Whether the page's CSS has worked out a rule about `prefers-reduced-motion` while nothing moved, since this was last asked,
/// which is CSS that would have things move with animations on: for `--do` to say so.
pub(crate) fn take_motion_held() -> bool {
    HOST.with(|host| host.motion_held.replace(false))
}

/// The modules an app can import that come with this, by the names they're imported by.
pub(crate) const HOST_MODULES: &[(&str, &str)] = &[
    ("rsui", LAYER),
    ("rsui/desktop", DESKTOP),
    ("rsui/desktop/core", DESKTOP_CORE),
    ("rsui/desktop/basics", DESKTOP_BASICS),
    ("rsui/desktop/sound", DESKTOP_SOUND),
    ("rsui/desktop/app", DESKTOP_APP),
    ("rsui/desktop/storage", DESKTOP_STORAGE),
    ("rsui/desktop/updates", DESKTOP_UPDATES),
    ("rsui/desktop/more", DESKTOP_MORE),
    ("rsui/desktop/pictures", DESKTOP_PICTURES),
    ("rsui/desktop/media", DESKTOP_MEDIA),
    ("rsui/web-routes", WEB_ROUTES),
    ("rsui/dom-globals", DOM_GLOBALS),
    ("rsui/dom", DOM),
    ("rsui/dom-events", DOM_EVENTS),
    ("rsui/dom-observers", DOM_OBSERVERS),
    ("rsui/dom-state", DOM_STATE),
    ("rsui/web", WEB),
    ("rsui/canvas", CANVAS),
    ("rsui/editing", EDITING),
    ("rsui/events", EVENTS),
    ("react", REACT),
    ("react-dom", REACT_DOM),
    ("react-dom/client", REACT_DOM),
    // There's no HTML here to turn anything into, and it's usually only for something like a picture.
    (
        "react-dom/server",
        "const none = () => { console.warn(\"react-dom/server can't make HTML here, so it comes to nothing\"); return ''; }; module.exports = { renderToString: none, renderToStaticMarkup: none };",
    ),
    ("react/jsx-runtime", REACT_JSX),
    ("react/jsx-dev-runtime", REACT_JSX),
    (
        "react/compiler-runtime",
        include_str!("react_compiler_runtime.js"),
    ),
];

fn set_up_layer(engine: &mut Engine) -> Result<Internals, String> {
    // An app's libraries are the ones written for a page, so each package's build for browsers is what it gets, as a bundler
    // for one would give it.
    engine.vm().set_browser_packages(true);
    for (name, source) in HOST_MODULES {
        engine.add_module(name, source);
    }
    // Files and requests finish on other threads, and this brings what they found back to this one.
    let remote = HOST.with(|host| host.remote.clone());
    engine.set_waker(move || {
        if let Some(remote) = remote.lock().expect("lock is not poisoned").as_ref() {
            remote.send(pump);
        }
    });
    // Where a program keeps its files is found from these.
    for (name, value) in std::env::vars() {
        engine.set_env(&name, &value);
    }
    let failed = |engine: &mut Engine, error: Value| engine.error_message(error).message;
    let vm = engine.vm();
    let natives = vm.new_object();
    let functions: [(&str, rsui_js::HostFunction); 36] = [
        ("wantDrops", host_want_drops),
        ("stopEvent", host_stop_event),
        ("passEvent", host_pass_event),
        ("preventEvent", host_prevent_event),
        ("listenDocument", host_listen_document),
        ("imageSize", host_image_size),
        ("editing", host_editing),
        ("fieldSelection", host_field_selection),
        ("systemText", host_system_text),
        ("appName", host_app_name),
        ("changeWindow", host_change_window),
        ("openWindow", host_open_window),
        ("closeWindow", host_close_window),
        ("setWindowTitle", host_set_window_title),
        ("dirtyAll", host_dirty_all),
        ("focus", host_focus),
        ("capturePointer", host_capture_pointer),
        ("selectText", host_select_text),
        ("setTitle", host_set_title),
        ("openUrl", host_open_url),
        ("isDark", host_is_dark),
        ("fillFragments", host_fill_fragments),
        ("setDarkTitleBar", host_set_dark_title_bar),
        ("copy", host_copy),
        ("paste", host_paste),
        ("dialog", host_dialog),
        ("dirty", host_dirty),
        ("effects", host_effects),
        ("frame", host_frame),
        ("wantLaidOut", host_want_laid_out),
        ("windowScale", host_window_scale),
        ("now", host_now),
        ("animationsOn", host_animations_on),
        ("motionHeld", host_motion_held),
        ("loadedBuiltins", host_loaded_builtins),
        ("mainTitle", host_main_title),
    ];
    for (name, function) in functions
        .into_iter()
        .chain(crate::native::FUNCTIONS)
        .chain(crate::native_more::FUNCTIONS)
        .chain(crate::native_more::MORE_FUNCTIONS)
        .chain(crate::geometry::FUNCTIONS)
        .chain(crate::why::FUNCTIONS)
    {
        let function = vm.host_function(name, 0, function);
        let atom = vm.atom(name);
        vm.set_property(natives, atom, function)
            .map_err(|_| "the layer can't be set up".to_owned())?;
    }
    // What reads CSS for `rsui/web`, apart from the rest, as only that uses it.
    let css = vm.new_object();
    for (name, function) in crate::css::FUNCTIONS
        .into_iter()
        .chain(crate::declare::FUNCTIONS)
        .chain(crate::layout::FUNCTIONS)
        .chain(crate::css_build::FUNCTIONS)
    {
        let function = vm.host_function(name, 0, function);
        let atom = vm.atom(name);
        vm.set_property(css, atom, function)
            .map_err(|_| "the layer can't be set up".to_owned())?;
    }
    let css_name = vm.atom("css");
    vm.set_property(natives, css_name, css)
        .map_err(|_| "the layer can't be set up".to_owned())?;
    // What draws a page's canvases, apart from the rest, as only `rsui/canvas` uses it.
    let canvas = vm.new_object();
    for (name, function) in crate::canvas::FUNCTIONS {
        let function = vm.host_function(name, 0, function);
        let atom = vm.atom(name);
        vm.set_property(canvas, atom, function)
            .map_err(|_| "the layer can't be set up".to_owned())?;
    }
    let canvas_name = vm.atom("canvas");
    vm.set_property(natives, canvas_name, canvas)
        .map_err(|_| "the layer can't be set up".to_owned())?;
    vm.set_global("__rsuiHost", natives);
    // What a logpoint put into a line calls. It's only ever there for that.
    let logpoint = vm.host_function("__rsuiLogpoint", 2, crate::logpoint::host_logpoint);
    vm.set_global("__rsuiLogpoint", logpoint);
    if let Err(error) = vm.load_host_module("rsui") {
        return Err(failed(engine, error));
    }
    let read = |vm: &mut Vm, from: Value, name: &str| -> Result<Root, Value> {
        let atom = vm.atom(name);
        let value = vm.get_property(from, atom)?;
        Ok(vm.root(value))
    };
    let vm = engine.vm();
    let internals = (|| -> Result<Internals, Value> {
        let from = vm.get_global("__rsuiInternals")?;
        let fragment = vm.get_global("__Fragment")?;
        let reordered = vm.get_global("__Reordered")?;
        Ok(Internals {
            set_tracing: read(vm, from, "setTracing")?,
            describe_hooks: read(vm, from, "describeHooks")?,
            restore_state: read(vm, from, "restoreState")?,
            new_instance: read(vm, from, "newInstance")?,
            new_click_event: read(vm, from, "newClickEvent")?,
            pass_click: read(vm, from, "passClick")?,
            web_description: read(vm, from, "webDescription")?,
            new_event: read(vm, from, "newEvent")?,
            document_event: read(vm, from, "documentEvent")?,
            rsui_events: read(vm, from, "rsuiEvents")?,
            desktop_event: read(vm, from, "desktopEvent")?,
            render: read(vm, from, "render")?,
            flush_effects: read(vm, from, "flushEffects")?,
            commit_effects: read(vm, from, "commitEffects")?,
            unmount: read(vm, from, "unmount")?,
            set_reloading: read(vm, from, "setReloading")?,
            pasted: read(vm, from, "pasted")?,
            color_scheme_changed: read(vm, from, "colorSchemeChanged")?,
            window_resized: read(vm, from, "windowResized")?,
            window_root: read(vm, from, "WindowRoot")?,
            fragment: vm.root(fragment),
            not_kept: read(vm, from, "notKept")?,
            reordered: vm.root(reordered),
            order_name: vm.atom("order"),
            type_name: vm.atom("type"),
            class_name: vm.atom("className"),
            key_name: vm.atom("key"),
            props_name: vm.atom("props"),
            kids_name: vm.atom("kids"),
            memo_name: vm.atom("memo"),
            function_name: vm.atom("name"),
            count_name: vm.atom("count"),
            visible_name: vm.atom("visible"),
            overscan_name: vm.atom("overscan"),
            render_row_name: vm.atom("renderRow"),
            axis_name: vm.atom("axis"),
            reveal_name: vm.atom("reveal"),
        })
    })();
    let internals = match internals {
        Ok(internals) => internals,
        Err(error) => return Err(failed(engine, error)),
    };
    // A promise that's rejected with nothing to catch it is logged, and the window hears about it, as on a page, rather than
    // being shown over the app as an error is.
    let rejected = (|| -> Result<Value, Value> {
        let vm = engine.vm();
        let from = vm.get_global("__rsuiInternals")?;
        let atom = vm.atom("rejected");
        vm.get_property(from, atom)
    })();
    match rejected {
        Ok(rejected) => engine.report_rejections_to(rejected),
        Err(error) => return Err(failed(engine, error)),
    }
    // Nothing else should find this.
    engine
        .eval("delete globalThis.__rsuiInternals", "rsui")
        .map_err(|error| error.message)?;
    Ok(internals)
}

/// What [`root`] is given. It's made by [`load`], and only works on the thread that made it.
#[derive(Debug, Clone)]
pub struct Entry {
    title: Option<String>,
}

impl Entry {
    /// What the app's main file exports as `title`.
    pub fn title(&self) -> Option<String> {
        self.title.clone()
    }
}

/// Runs an app's main file on this thread, ready for [`root`] to show it. `stack_limit` is how much of this thread's stack
/// it can use. Loading another replaces the one before.
pub fn load(path: &Path, stack_limit: usize) -> Result<Entry, String> {
    // What a prop's name means is kept by its atom, and a new engine numbers its atoms its own way.
    HOST.with(|host| *host.meanings.borrow_mut() = Meanings::default());
    // A timer that an app that ran on this thread before was waiting for is that app's, whose runtime won't run it: this one's
    // timers are armed for themselves, rather than left waiting behind it.
    HOST.with(|host| host.timer_due.set(None));
    let mut engine = Engine::new();
    engine.set_stack_limit(stack_limit);
    // A window that's gone quiet can collect all of the JavaScript's garbage too (`RSUI_IDLE_GC`).
    rsui::on_quiet(collect_when_quiet);
    // Packing writes down what the app's files compile to, from rsui's own on, for the program to load (see `pack`).
    if KEEP_COMPILED_CODE.with(Cell::get) {
        engine.keep_compiled_code();
    }
    if let Some(logs) = HOST.with(|host| host.logs.borrow().clone()) {
        engine.set_printer(move |text| crate::agent::keep_log(&logs, text));
    } else if crate::live::listening() {
        // It's printed as it would be, and kept for a viewer too.
        engine.set_printer(|text| {
            use std::io::Write;
            let _ = std::io::stdout().write_all(text.as_bytes());
            crate::live::heard(text);
        });
    }
    rsui::mark("engine made");
    let internals = set_up_layer(&mut engine)?;
    rsui::mark("layer set up");
    // An app's code is a page's, which Vite gives `import.meta.env`, where Node gives modules none.
    let env = crate::vite_env::import_meta_env(path, rsui_js::running_packed());
    let entries: Vec<(&str, rsui_js::MetaEnvValue)> = env
        .iter()
        .map(|(key, value)| (key.as_str(), value.clone()))
        .collect();
    engine.set_import_meta_env(&entries);
    HOST.with(|host| {
        *host.engine.borrow_mut() = Some(engine);
        *host.internals.borrow_mut() = Some(internals);
    });
    if let Some(folder) = std::path::absolute(path)
        .ok()
        .and_then(|file| file.parent().map(Path::to_path_buf))
    {
        rsui::render::set_files_from(folder);
    }
    let (app, title) = run_main_file(path)?;
    rsui::mark("main file run");
    rsui_js::write_startup_times("main file run");
    // What the main file left waiting runs before the window opens, as an Electron app's `app.whenReady()` does before
    // its window's made, so that what that says about the window, as having no frame, is how it opens.
    HOST.with(|host| {
        if let Some(engine) = host.engine.borrow_mut().as_mut() {
            let _ = engine.run_jobs();
        }
    });
    HOST.with(|host| *host.app.borrow_mut() = Some(app));
    Ok(Entry { title })
}

fn run_main_file(path: &Path) -> Result<(Root, Option<String>), String> {
    APP_FILE.with(|file| *file.borrow_mut() = path.to_owned());
    HOST.with(|host| {
        let mut engine = host.engine.borrow_mut();
        let engine = engine.as_mut().expect("there's an engine");
        let exports = engine
            .run_file(path)
            .map_err(|error| newer_needed(path, error.message))?;
        if let Some(wanted) = version_wanted(path).filter(|wanted| is_newer(wanted, env!("CARGO_PKG_VERSION"))) {
            eprintln!(
                "this app needs a newer rsui-tsx ({wanted}), you have ({}), so some of it may not work",
                env!("CARGO_PKG_VERSION")
            );
        }
        let vm = engine.vm();
        let default_name = vm.atom("default");
        let title_name = vm.atom("title");
        let app = vm
            .get_property(exports, default_name)
            .and_then(|app| {
                let title = vm.get_property(exports, title_name)?;
                Ok((app, title))
            })
            .map_err(|error| vm.error_text(error))?;
        if !vm.is_function(app.0) {
            return Err(format!(
                "{} has to have a component as its default export",
                path.display()
            ));
        }
        let title = vm.string_text(app.1).map(|title| title.into_owned());
        // `export const size = [800, 600]`, for how much room there is inside the window.
        let size_name = vm.atom("size");
        let size = vm.get_property(exports, size_name).ok().and_then(|size| {
            let parts = vm.array_elements(size)?;
            let (width, height) = (parts.first()?, parts.get(1)?);
            (width.is_number() && height.is_number())
                .then(|| (width.as_number() as u32, height.as_number() as u32))
        });
        SIZE.with(|slot| slot.set(size));
        // What a page asks about how big the window is, as `innerWidth` does, is that from the start, before it's ever resized.
        // Run again, as it is when it's saved, the window's already the size it is, which may be maximized, and that stays: a
        // page that lays itself out from `innerWidth` isn't left laid out for the size it opened at.
        let told = vm
            .get_global("__rsuiPageSize")
            .is_ok_and(|known| !known.is_undefined());
        if let Some((width, height)) = size.filter(|_| !told) {
            let pair = vm.new_array(vec![
                Value::number(f64::from(width)),
                Value::number(f64::from(height)),
            ]);
            vm.set_global("__rsuiPageSize", pair);
        }
        // `export const frameless = true`, and the same for these others, for what kind of window it is.
        let mut said = |name: &str| {
            let name = vm.atom(name);
            vm.get_property(exports, name)
                .is_ok_and(|value| value == Value::TRUE)
        };
        let (frameless, on_top, fullscreen) =
            (said("frameless"), said("alwaysOnTop"), said("fullscreen"));
        rsui::set_main_window_kind(frameless, on_top, fullscreen);
        // `export const layer = 'desktop'`, `transparent`, `skipTaskbar` and `allDesktops = true`, and
        // `appBar = { edge: 'top', size: 32 }`, for the rest of how it opens.
        let mut get = |name: &str| {
            let name = vm.atom(name);
            vm.get_property(exports, name).unwrap_or(Value::UNDEFINED)
        };
        let (layer, transparent, skip_taskbar, all_desktops, app_bar) =
            (get("layer"), get("transparent"), get("skipTaskbar"), get("allDesktops"), get("appBar"));
        let app_bar = app_bar_of(vm, app_bar);
        rsui::set_main_window_options(&rsui::WindowOptions {
            layer: layer_of(vm, layer),
            transparent: transparent == Value::TRUE,
            skip_taskbar: skip_taskbar == Value::TRUE,
            all_desktops: all_desktops == Value::TRUE,
            app_bar,
            ..rsui::WindowOptions::default()
        });
        // `export const minSize = [400, 300]`, for how small the window can be made.
        let smallest_name = vm.atom("minSize");
        let smallest = vm
            .get_property(exports, smallest_name)
            .ok()
            .and_then(|size| {
                let parts = vm.array_elements(size)?;
                let (width, height) = (parts.first()?, parts.get(1)?);
                (width.is_number() && height.is_number())
                    .then(|| (width.as_number() as u32, height.as_number() as u32))
            });
        if let Some((width, height)) = smallest {
            rsui::set_smallest_window_size(width, height);
        }
        Ok((vm.root(app.0), title))
    })
}

/// The rsui-tsx an app says it needs, as its `package.json` says it: `"engines": { "rsui-tsx": ">=0.2.0" }`.
/// It's from the nearest `package.json` that's in the main file's folder or above it.
fn version_wanted(main: &Path) -> Option<String> {
    let folder = std::path::absolute(main).ok()?;
    let found = folder
        .ancestors()
        .skip(1)
        .map(|folder| folder.join("package.json"))
        .find(|file| file.is_file())?;
    let package: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(found).ok()?).ok()?;
    let wanted = package.get("engines")?.get("rsui-tsx")?.as_str()?;
    Some(
        wanted
            .trim_start_matches(['>', '=', '^', '~', 'v', ' '])
            .to_owned(),
    )
}

/// Whether version `wanted` comes after `have`, going by each of their numbers in turn.
fn is_newer(wanted: &str, have: &str) -> bool {
    let numbers = |text: &str| -> Vec<u64> {
        text.split(['.', '-', '+'])
            .map_while(|part| part.parse().ok())
            .collect()
    };
    numbers(wanted) > numbers(have)
}

/// What to say when the main file can't be run, which is plainer when that's because this rsui-tsx is older than what the app was made
/// for: either it asks for an `rsui/` module that this doesn't have, or its `package.json` says it needs a newer one.
fn newer_needed(main: &Path, message: String) -> String {
    let have = env!("CARGO_PKG_VERSION");
    let wanted = version_wanted(main).filter(|wanted| is_newer(wanted, have));
    let missing = message
        .split("Cannot find module '")
        .nth(1)
        .and_then(|rest| rest.split('\'').next())
        .filter(|name| {
            name.starts_with("rsui/") && !HOST_MODULES.iter().any(|(known, _)| known == name)
        });
    match (missing, wanted) {
        (Some(name), Some(wanted)) => format!(
            "this app needs a newer rsui-tsx ({wanted}), you have ({have}): it imports {name}, which this one doesn't have.\n\n{message}"
        ),
        (Some(name), None) => format!(
            "this app needs a newer rsui-tsx, you have ({have}): it imports {name}, which this one doesn't have.\n\n{message}"
        ),
        (None, Some(wanted)) => format!(
            "this app needs a newer rsui-tsx ({wanted}), you have ({have}), which may be why it can't start.\n\n{message}"
        ),
        (None, None) => message,
    }
}

thread_local! {
    static APP_FILE: RefCell<std::path::PathBuf> = RefCell::new(std::path::PathBuf::new());
    static SIZE: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
}

/// How much room the app asks for inside its window, if it does.
pub(crate) fn wanted_size() -> Option<(u32, u32)> {
    SIZE.with(Cell::get)
}

/// The modules that come with this, and with the engine, that the app on this thread has run so far.
pub(crate) fn loaded_builtins() -> Vec<String> {
    HOST.with(|host| {
        host.engine
            .borrow()
            .as_ref()
            .map_or_else(Vec::new, Engine::loaded_builtins)
    })
}

/// Loads another of the app's files, as `import()` would once it's wanted, for what it loads to be known. What goes wrong
/// in it is left for when the app runs it.
pub(crate) fn load_also(path: &std::path::Path) {
    HOST.with(|host| {
        if let Some(engine) = host.engine.borrow_mut().as_mut() {
            let _ = engine.run_file(path);
        }
    });
}

thread_local! {
    /// Whether the engine [`load`] makes on this thread writes down what it compiles, for packing.
    static KEEP_COMPILED_CODE: Cell<bool> = const { Cell::new(false) };
}

/// Has the engine the app on this thread is loaded with write down what its files compile to, for
/// [`take_compiled_code`]: packing puts that in the program, which then doesn't compile them again.
pub(crate) fn keep_compiled_code() {
    KEEP_COMPILED_CODE.with(|keep| keep.set(true));
}

/// What the app on this thread's files compiled to, if [`keep_compiled_code`] asked for it before it was loaded.
pub(crate) fn take_compiled_code() -> Option<rsui_js::CompiledCode> {
    HOST.with(|host| {
        host.engine
            .borrow_mut()
            .as_mut()
            .and_then(Engine::take_compiled_code)
    })
}

thread_local! {
    /// What the app needs that isn't code it loads, which packing it has to take too, as what its CSS tools made.
    static PACK_ALSO: RefCell<Vec<std::path::PathBuf>> = const { RefCell::new(Vec::new()) };
}

/// Has packing the app on this thread take this file too.
pub(crate) fn pack_also(path: &std::path::Path) {
    PACK_ALSO.with(|files| {
        let mut files = files.borrow_mut();
        if !files.iter().any(|one| one == path) {
            files.push(path.to_owned());
        }
    });
}

/// Has what the app's loaded do what it does before it's packed, as `rsui/web` making what the app's CSS tools make of its CSS,
/// which is otherwise done as it's first drawn.
pub(crate) fn before_packing() {
    HOST.with(|host| {
        if let Some(engine) = host.engine.borrow_mut().as_mut() {
            let _ = engine.eval("globalThis.__rsuiBeforePacking?.()", "rsui:pack");
        }
    });
}

/// What [`pack_also`] was given.
pub(crate) fn packed_also() -> Vec<std::path::PathBuf> {
    PACK_ALSO.with(|files| files.borrow().clone())
}

/// The files the app on this thread has loaded so far.
pub(crate) fn loaded_files() -> Vec<std::path::PathBuf> {
    HOST.with(|host| {
        host.engine
            .borrow()
            .as_ref()
            .map_or_else(Vec::new, Engine::loaded_files)
    })
}

pub(crate) fn resolve_for_pack(
    from: &std::path::Path,
    name: &str,
    is_import: bool,
) -> Result<Option<std::path::PathBuf>, String> {
    HOST.with(|host| {
        let mut engine = host.engine.borrow_mut();
        engine
            .as_mut()
            .ok_or_else(|| "pack engine is not loaded".to_owned())?
            .resolve_module_file(from, name, is_import)
            .map_err(|error| error.to_string())
    })
}

pub(crate) fn configuration_files_for_pack() -> Vec<std::path::PathBuf> {
    HOST.with(|host| host.engine.borrow().as_ref().map_or_else(Vec::new, Engine::module_configuration_files))
}

/// Has components keep their state when their code is swapped for a new version, which is what [`crate::run`] does when
/// it's watching files.
pub fn keep_state_across_reloads() {
    HOST.with(|host| host.hot.set(true));
}

/// Asks for a new version of an app's main file to be swapped in, and says whether anything is running to do that. This
/// is what happens when a file is saved while an app's being watched.
pub fn request_reload(path: &Path) -> bool {
    let path = path.to_owned();
    HOST.with(|host| {
        host.remote
            .lock()
            .expect("lock is not poisoned")
            .as_ref()
            .is_some_and(|remote| remote.send(move |updates| reload(updates, &path)))
    })
}

/// How a thread watching files gets a new version of the app running.
#[derive(Clone)]
pub(crate) struct ReloadSender {
    remote: Arc<Mutex<Option<Remote>>>,
}

pub(crate) fn reload_sender() -> ReloadSender {
    HOST.with(|host| {
        host.hot.set(true);
        ReloadSender {
            remote: host.remote.clone(),
        }
    })
}

impl ReloadSender {
    pub(crate) fn send(&self, path: PathBuf) {
        let remote = self.remote.lock().expect("lock is not poisoned");
        if let Some(remote) = remote.as_ref() {
            remote.send(move |updates| reload(updates, &path));
        }
    }
}

/// Swaps in a new version of the app's code. Every component renders again, and keeps its state if it can.
fn reload(updates: &mut Updates<'_>, path: &Path) {
    // What the last version asked of the desktop goes, for this one to ask for again.
    crate::native::let_go_of_everything();
    crate::native_more::let_go_of_everything();
    HOST.with(|host| {
        let mut engine = host.engine.borrow_mut();
        engine.as_mut().expect("there's an engine").forget_files();
    });
    match run_main_file(path) {
        Ok((app, _)) => {
            HOST.with(|host| {
                *host.app.borrow_mut() = Some(app);
                *host.error.borrow_mut() = None;
                host.error_changed.set(true);
            });
            let set_reloading = with_internals(|internals| internals.set_reloading.clone());
            with_vm_letting_go(|vm| {
                let _ = vm.call_function(set_reloading.get(), Value::UNDEFINED, &[Value::TRUE]);
            });
            HOST.with(|host| host.swapping.set(true));
            HOST.with(|host| {
                if let Some(generation) = host.root_state.get() {
                    updates.update(generation, |generation| *generation += 1);
                }
                // Each window's come to by way of the list of what's changed.
                let count = host.instances.borrow().len() as u32;
                host.dirty.borrow_mut().extend(0..count);
                let why = crate::why::take_reload_reason()
                    .unwrap_or("Its code was saved, so it ran again.");
                crate::why::note_all(count, why);
            });
        }
        // What's running carries on, and the error shows until the next save.
        Err(message) => report(message),
    }
    pump(updates);
}

/// Everything that JavaScript left waiting: finished promises, timers that are due, and components that have changed.
fn pump(updates: &mut Updates<'_>) {
    pump_with(updates, false);
}

/// What [`pump`] does, which a timer that's come due does too, as `timer` says. What's changed is drawn before a timer runs, but a
/// timer only waits for one frame of that: one that always leaves something more to draw mustn't keep timers and animation
/// frames from ever running, as a browser runs them between frames whatever's waiting to be drawn.
fn pump_with(updates: &mut Updates<'_>, timer: bool) {
    let effects_waiting = updates.effects_waiting();
    let wait = HOST.with(|host| {
        let mut engine = host.engine.borrow_mut();
        let engine = engine.as_mut().expect("there's an engine");
        // This runs after every effect and every click, and there's usually nothing to do.
        if !engine.has_pending() {
            // Even an empty microtask checkpoint ends the synchronous job. WeakRef
            // targets kept during this event must not survive until an unrelated timer.
            engine.vm().clear_kept_objects();
            return None;
        }
        if let Err(error) = engine.run_jobs() {
            report(error.message);
        }
        // What's changed is drawn before any timer runs, as a browser does: a timer that's been replaced by what that drawing does
        // mustn't run as it was. A timer that's due waits for one frame of that, and no more.
        if !host.dirty.borrow().is_empty() || effects_waiting {
            if !timer || !host.timer_waited.replace(true) {
                return engine.next_timer_in();
            }
        }
        if timer {
            host.timer_waited.set(false);
            // A timer's own turn comes once what was being dispatched is over, whether or not it said so.
            host.dispatching.set(false);
        } else if host.dispatching.get() {
            // In the middle of dispatching something, only promises are seen to: timers wait for it to be over (see
            // `dispatching`), which they're armed for.
            return engine.next_timer_in();
        }
        // A handler of an event that's still on its way, as React's `onKeyDown` before `document` hears the key, leaves its
        // timers until the event's been everywhere: a page never runs them in between. A timer is armed for them instead.
        // A replay has timers go off when they went off the first time, and no other time.
        if (!timer && rsui::runtime::dispatching()) || !crate::replay::may_run_timers() {
            return engine.next_timer_in();
        }
        let due = engine.next_timer_in().is_some_and(|wait| wait <= 0.0);
        let wait = match engine.run_timers() {
            Ok(wait) => wait,
            Err(error) => {
                report(error.message);
                None
            }
        };
        if due {
            crate::replay::timers_ran();
        }
        if let Err(error) = engine.run_jobs() {
            report(error.message);
        }
        wait
    });
    apply_changes(updates);
    if let Some(wait) = wait {
        arm_timer(updates, Duration::from_secs_f64(wait / 1000.0));
    }
}

fn arm_timer(updates: &mut Updates<'_>, wait: Duration) {
    // The main window's there for as long as the program is, which a timer can't count on from any other.
    let main = HOST.with(|host| host.main_runtime.get());
    if main != 0 && updates.runtime_id() != main {
        let remote = HOST.with(|host| {
            host.remotes
                .borrow()
                .iter()
                .find(|(id, _)| *id == main)
                .map(|(_, remote)| remote.clone())
        });
        if let Some(remote) = remote {
            remote.send(move |updates| arm_timer(updates, wait));
        }
        return;
    }
    let due = Instant::now() + wait;
    let needed = HOST.with(|host| reserve_timer_deadline(&host.timer_due, due));
    if needed {
        updates.after(wait, move |updates| {
            // Scheduling an earlier deadline leaves the older runtime callback
            // queued. It no longer owns the engine's timer: letting it clear and
            // rearm that newer deadline creates accumulating no-op wake chains.
            if HOST.with(|host| consume_timer_deadline(&host.timer_due, due)) {
                pump_with(updates, true);
            }
        });
    }
}

fn reserve_timer_deadline(timer: &Cell<Option<Instant>>, due: Instant) -> bool {
    match timer.get() {
        Some(already) if already <= due => false,
        _ => {
            timer.set(Some(due));
            true
        }
    }
}

fn consume_timer_deadline(timer: &Cell<Option<Instant>>, due: Instant) -> bool {
    if timer.get() != Some(due) {
        return false;
    }
    timer.set(None);
    true
}

macro_rules! apply_dirty {
    ($target:expr) => {
        HOST.with(|host| {
            let here = $target.runtime_id();
            let dirty = std::mem::take(&mut *host.dirty.borrow_mut());
            if !dirty.is_empty() {
                let instances = host.instances.borrow();
                let owners = host.owners.borrow();
                let mut elsewhere = Vec::new();
                for slot in dirty {
                    if let Some(Some(state)) = instances.get(slot as usize) {
                        if owners[slot as usize] == here {
                            $target.update(*state, |_| {});
                        } else {
                            // That's in another window, which is told to come and get it.
                            elsewhere.push(slot);
                        }
                    }
                }
                if !elsewhere.is_empty() {
                    wake_owners(host, &elsewhere, &owners);
                    host.dirty.borrow_mut().extend(elsewhere);
                }
            }
            if host.error_changed.get() {
                // What's wrong shows in the main window, whichever one it happened in.
                if here == host.main_runtime.get() {
                    host.error_changed.set(false);
                    if let Some(generation) = host.root_state.get() {
                        $target.update(generation, |_| {});
                    }
                } else {
                    wake_runtime(host, host.main_runtime.get());
                }
            }
            if let Some((key, visible, prevent_scroll)) = host.focus_wanted.borrow_mut().take() {
                $target.focus_with_options(key, visible, prevent_scroll);
            }
            if let Some((key, start, end)) = host.select_wanted.borrow_mut().take() {
                $target.select_text(key, start, end);
            }
            if let Some(key) = host.capture_wanted.borrow_mut().take() {
                $target.capture_pointer(key);
            }
            if let Some(title) = host.title_wanted.borrow_mut().take() {
                $target.set_title(title);
            }
            if let Some(text) = host.copy_wanted.borrow_mut().take() {
                $target.copy_text(text);
            }
            if host.paste_wanted.replace(false) {
                $target.ask_for_clipboard();
            }
            for url in host.urls_wanted.borrow_mut().drain(..) {
                $target.open_url(url);
            }
            if let Some(dark) = host.dark_title_bar_wanted.take() {
                $target.set_dark_title_bar(dark);
            }
            if host.system_glyphs_wanted.take() {
                $target.draw_text_as_the_system_does();
            }
        })
    };
}

fn drop_files() -> ActionKind {
    static NAME: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("drop_files"));
    ActionKind::Custom(*NAME)
}

fn wake_runtime(host: &Host, runtime: u32) {
    wake_runtime_with(host, runtime, apply_changes);
}

fn wake_runtime_with(host: &Host, runtime: u32, then: fn(&mut Updates<'_>)) {
    let remote = host
        .remotes
        .borrow()
        .iter()
        .find(|(id, _)| *id == runtime)
        .map(|(_, remote)| remote.clone());
    if let Some(remote) = remote {
        remote.send(then);
    }
}

fn wake_owners(host: &Host, slots: &[u32], owners: &[u32]) {
    let mut told = Vec::new();
    for slot in slots {
        let owner = owners[*slot as usize];
        if !told.contains(&owner) {
            told.push(owner);
            wake_runtime(host, owner);
        }
    }
}

fn apply_changes(updates: &mut Updates<'_>) {
    apply_dirty!(updates);
}

// The components.

/// What makes a JavaScript component.
struct JsProps {
    function: Root,
    props: Root,
    /// What kind of component this is. When it changes, what's there is unmounted and this starts again.
    kind: u64,
    /// Whether the parent gave it a key, which then goes on its element.
    keyed: bool,
    /// Whether its element was made by an app's own JSX where nothing said `'use no memo'`, which `__jsxMemo` marks. See `eq`.
    memo: bool,
}

impl PartialEq for JsProps {
    /// Whether it's given what it was given before, so it isn't rendered again, as React has it: the same props object (its
    /// element as it was), as React only compares props as `memo` does for a component that says so. Here that's also what
    /// draws an element of a page (`__rsuiShallow`, see web.js), and props that are the same and hold no objects or functions, which
    /// can't say anything else without being given anew. Props that hold an object can: TanStack Table's cells are given the same
    /// `row` each time, which says it's selected once it is, and a checkbox in one that wasn't rendered again kept its old state.
    ///
    /// An element an app's own JSX made, which `memo` says, doesn't render either when what it's given does the same as before,
    /// as React Compiler has it by keeping the element: a handler made again with the same code seeing the same values, or an
    /// object or element made again holding the same (see `Vm::props_equivalent`). Then the parent needn't keep its functions
    /// with `useCallback`, or wrap the child in `memo`. That's only for a component written in the app's own files, as one from
    /// a library, or rsui's own page elements, can read what changes without a render, as the page's CSS. Given the same ref,
    /// or the same object with methods, it renders whenever its parent does, as either can say something else with no render
    /// (unless it says `memo`), and so does a component that says `'use no memo'`, or a class. What reads a context or a store
    /// renders when that changes, as it would anyway, as its provider or the store tells it to.
    fn eq(&self, other: &Self) -> bool {
        let function = self.function.get();
        if function != other.function.get() {
            return false;
        }
        let (old, new) = (self.props.get(), other.props.get());
        if old == new {
            return true;
        }
        let not_kept = with_internals(|internals| internals.not_kept.get());
        with_vm(|vm| {
            let name = vm.atom("__rsuiShallow");
            let said_memo = vm
                .get_property(function, name)
                .is_ok_and(|said| said == Value::TRUE);
            if self.memo && other.memo && vm.made_by_app(function) && !vm.says_no_memo(function) {
                return vm.props_equivalent(old, new, !said_memo, not_kept);
            }
            vm.shallow_equal(old, new) && (said_memo || vm.holds_no_objects(new))
        })
    }
}

/// Whether a component in the same place is the same kind as before. Watching files, that's by its name, so a new version of
/// it keeps its state; but other than while a new version's being swapped in, it has to be the same function too, as React has
/// it: a component made anew each render, as shadcn's Calendar makes its `Root` and react-day-picker's nav inside its props, is
/// another kind each time, and what it drew is drawn anew, losing the focus (Edge leaves the month's Previous button and focus
/// goes to `<body>`).
fn same_kind(old: &JsProps, new: &JsProps) -> bool {
    old.kind == new.kind
        && (HOST.with(|host| !host.hot.get() || host.swapping.get())
            || old.function.get() == new.function.get())
}

fn new_instance() -> JsInstance {
    let slot = HOST.with(|host| {
        host.free_slots.borrow_mut().pop().unwrap_or_else(|| {
            let mut instances = host.instances.borrow_mut();
            instances.push(None);
            (instances.len() - 1) as u32
        })
    });
    let parent = HOST.with(|host| host.owner.borrow().clone());
    let make = with_internals(|internals| internals.new_instance.clone());
    let object = with_vm_letting_go(|vm| {
        let parent = parent.map_or(Value::NULL, |parent| parent.get());
        match vm.call_function(
            make.get(),
            Value::UNDEFINED,
            &[Value::int(slot as i32), parent],
        ) {
            Ok(object) => vm.root(object),
            Err(error) => {
                let message = vm.error_text(error);
                eprintln!("{message}");
                vm.root(Value::UNDEFINED)
            }
        }
    });
    JsInstance {
        object,
        slot,
        effect_runs: Cell::new(0),
    }
}

fn error_element(message: &str) -> Element {
    Element::new(Role::Text)
        .name(message)
        .props(Props::from(Paint::new().text(Color::hex(0xCF222E))))
}

/// Calls what JavaScript gives for something to do with a component, reporting anything that goes wrong.
fn call_for_instance(function: &Root, object: &Root) {
    let failed = with_vm_letting_go(|vm| {
        match vm.call_function(function.get(), Value::UNDEFINED, &[object.get()]) {
            Ok(_) => None,
            Err(error) => Some(vm.error_text(error)),
        }
    });
    if let Some(message) = failed {
        report(message);
    }
}

/// With `RSUI_SPLIT` set, counts a component rendering, as `n:render` in the split, for measuring what a frame rendered.
fn count_render() {
    if !rsui::split::on() {
        return;
    }
    thread_local! {
        static PART: u16 = rsui::split::named("n:render");
    }
    PART.with(|part| rsui::split::count(*part, 1));
}

fn js_component(cx: &mut Cx<'_>, props: &JsProps) -> Node {
    count_render();
    let instance = cx.state(new_instance);
    let (object, slot) = cx.with(instance, |instance| {
        (instance.object.clone(), instance.slot)
    });
    HOST.with(|host| {
        host.instances.borrow_mut()[slot as usize] = Some(instance);
        let mut owners = host.owners.borrow_mut();
        if owners.len() <= slot as usize {
            owners.resize(slot as usize + 1, 0);
        }
        owners[slot as usize] = cx.runtime_id();
    });
    let unmounting = object.clone();
    cx.effect_with_cleanup((), move |_| {
        move |updates: &mut Updates<'_>| {
            let unmount = with_internals(|internals| internals.unmount.clone());
            call_for_instance(&unmount, &unmounting);
            crate::why::forget_slot(slot);
            HOST.with(|host| {
                if crate::why::tracing() {
                    host.traced.borrow_mut().remove(&slot);
                }
                host.instances.borrow_mut()[slot as usize] = None;
                host.kept_kids.borrow_mut().remove(&slot);
                host.free_slots.borrow_mut().push(slot);
            });
            apply_changes(updates);
        }
    });
    set_clock(cx);
    // The layer notes where hooks are called from as they are, so it has to know before it renders. It stops when it's
    // told to, rather than when a window that isn't being looked at renders.
    let tracing = cx.tracing();
    if tracing && !crate::why::tracing() {
        follow_tracing(true);
    }
    let render = with_internals(|internals| internals.render.clone());
    let (output, mark) = with_vm(|vm| {
        let mark = vm.new_values_mark();
        let output = vm
            .call_function(
                render.get(),
                Value::UNDEFINED,
                &[object.get(), props.function.get(), props.props.get()],
            )
            .map_err(|error| vm.error_text(error));
        (output, mark)
    });
    if tracing {
        explain_render(cx, &object, slot, props);
    }
    // Children render while this builds its output, so these have to be read before they can take them.
    let frame_wanted = HOST.with(|host| host.frame_wanted.replace(false));
    let wants_effects = HOST.with(|host| host.effects_wanted.replace(false));
    let node = match output {
        Ok(output) => {
            // A provider whose value changed has marked what reads it. What of that is inside here renders now with
            // its new props, which counts for that too, rather than rendering again once this is over.
            if HOST.with(|host| !host.dirty.borrow().is_empty()) {
                apply_dirty!(cx);
            }
            let parent = HOST.with(|host| host.owner.replace(Some(object.clone())));
            let node = build_output(cx, output, props.keyed, Some(slot));
            HOST.with(|host| *host.owner.borrow_mut() = parent);
            node
        }
        Err(message) => {
            eprintln!("{message}");
            error_element(&message).into_node()
        }
    };
    with_vm(|vm| vm.release_new_values(mark));
    if with_vm(|vm| vm.has_jobs()) {
        HOST.with(|host| host.jobs_waiting.set(true));
    }
    if frame_wanted {
        cx.next_frame();
    }
    let runs = cx.with(instance, |instance| {
        if wants_effects {
            instance.effect_runs.set(instance.effect_runs.get() + 1);
        }
        instance.effect_runs.get()
    });
    let running = object;
    cx.effect(runs, move |_| {
        // Most components have no effects, and mounting a thousand of them shouldn't go into JavaScript
        // a thousand times to find that out. Those that do run once everything in this round has rendered, as React's commit
        // runs them: see `commit_effects`.
        if wants_effects {
            let flush = with_internals(|internals| internals.flush_effects.clone());
            call_for_instance(&flush, &running);
        }
        HOST.with(|host| host.commit_waiting.set(true));
    });
    // Setting state while rendering renders it again, once this is over.
    apply_dirty!(cx);
    node
}

/// Tells the layer whether to note what it does for a debugger, if that's changed.
fn follow_tracing(on: bool) {
    if crate::why::set_tracing(on) {
        if !on {
            HOST.with(|host| host.traced.borrow_mut().clear());
        }
        let set = with_internals(|internals| internals.set_tracing.clone());
        with_vm_letting_go(|vm| {
            let _ = vm.call_function(set.get(), Value::UNDEFINED, &[Value::bool(on)]);
        });
    }
}

/// The name of the component an instance in the layer is, from its function.
pub(crate) fn instance_name(vm: &mut Vm, instance: Value) -> String {
    let type_name = vm.atom("type");
    let name = vm.atom("name");
    vm.get_property(instance, type_name)
        .and_then(|function| vm.get_property(function, name))
        .ok()
        .and_then(|name| vm.plain_text(name))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "A component".to_owned())
}

/// Says who this component is and why it rendered, for what's tracing, and keeps what's needed to say it next time.
#[cold]
#[inline(never)]
fn explain_render(cx: &mut Cx<'_>, object: &Root, slot: u32, props: &JsProps) {
    crate::why::remember_slot(cx.runtime_id(), cx.component(), slot);
    let parent = HOST.with(|host| host.owner.borrow().clone());
    let last = HOST
        .with(|host| host.traced.borrow_mut().remove(&slot))
        .map(|(_, last)| last);
    let cause = cx.cause();
    let (name, why, kept) = with_vm_letting_go(|vm| {
        let name = instance_name(vm, object.get());
        let parent_name = parent.as_ref().map_or_else(
            || "What's around it".to_owned(),
            |parent| instance_name(vm, parent.get()),
        );
        let mut why = crate::why::take_notes(slot);
        match cause {
            rsui::runtime::Cause::Props => match &last {
                Some(last) => {
                    let around = parent.as_ref().map_or(Value::NULL, Root::get);
                    why.extend(crate::why::props_why(
                        vm,
                        last.get(),
                        props.props.get(),
                        &parent_name,
                        around,
                    ));
                }
                None => why.push(format!("{parent_name} rendered it with different props.")),
            },
            rsui::runtime::Cause::State if why.is_empty() => {
                why.push("Its state changed, or it's moving.".to_owned());
            }
            _ => {}
        }
        (name, why, vm.root(props.props.get()))
    });
    HOST.with(|host| {
        host.traced
            .borrow_mut()
            .insert(slot, (object.clone(), kept))
    });
    cx.trace_name(name);
    for line in why {
        cx.trace_why(line);
    }
}

/// What the hooks of the component that has this number in that runtime hold, a line each, if it's a JavaScript one.
pub(crate) fn hooks_of(runtime: u32, component: (u32, u32)) -> Option<Vec<String>> {
    let slot = crate::why::slot_of(runtime, component)?;
    let (found, _) = HOST.with(|host| host.traced.borrow().get(&slot).cloned())?;
    let describe = with_internals(|internals| internals.describe_hooks.clone());
    Some(with_vm_letting_go(|vm| {
        match vm.call_function(describe.get(), Value::UNDEFINED, &[found.get()]) {
            Ok(hooks) => crate::why::describe_hooks(vm, hooks),
            Err(error) => vec![vm.error_text(error)],
        }
    }))
}

/// Puts the state of every component back to how it was just after change `target`, or forward again, as `history` numbers
/// them, and has what that changes render. Says how many hooks it put back.
pub(crate) fn go_to_change(target: u32) -> Result<usize, String> {
    let plan = crate::why::plan_goto(target)?;
    let restore = with_internals(|internals| internals.restore_state.clone());
    let mut put_back = 0;
    for (instance, index, slot, value) in plan {
        let done = with_vm_letting_go(|vm| {
            vm.call_function(
                restore.get(),
                Value::UNDEFINED,
                &[instance.get(), Value::int(index as i32), value.get()],
            )
            .is_ok_and(|done| done == Value::TRUE)
        });
        if done {
            put_back += 1;
            crate::why::note_put_back(slot, target);
        }
    }
    // Each window comes and gets what's changed in it.
    HOST.with(|host| {
        let dirty = host.dirty.borrow().clone();
        let owners = host.owners.borrow();
        wake_owners(host, &dirty, &owners);
    });
    Ok(put_back)
}

/// Tells the layer whether to note what it does for a debugger, as the runtime says, when that's changed from outside.
pub(crate) fn set_tracing(on: bool) {
    follow_tracing(on);
}

/// Tells JavaScript what time it is for anything that's moving.
fn set_clock(cx: &Cx<'_>) {
    thread_local! {
        static STARTED: Instant = Instant::now();
    }
    let now = cx.frame_time();
    let elapsed = STARTED.with(|started| now.saturating_duration_since(*started));
    HOST.with(|host| {
        host.now_ms.set(elapsed.as_secs_f64() * 1000.0);
        host.animations_on.set(cx.animations_on());
    });
}

/// The kind of component a function is. It's the function itself, unless components are being swapped for new versions,
/// when it's what they're called.
fn kind_of(vm: &mut Vm, function: Value, name: Atom) -> u64 {
    if !HOST.with(|host| host.hot.get()) {
        return function.bits();
    }
    let named = vm.get_property(function, name).ok();
    let mut text = named.and_then(|named| vm.string_text(named));
    if text.as_ref().is_none_or(|text| text.is_empty()) {
        // What `forwardRef` gives back has no name, as React's has none, but it says what it's for.
        let display = vm.atom("displayName");
        let named = vm.get_property(function, display).ok();
        text = named.and_then(|named| vm.string_text(named));
    }
    match text {
        Some(text) if !text.is_empty() => {
            // FNV, which only has to tell a program's components apart.
            let mut hash = 0xCBF2_9CE4_8422_2325u64;
            for byte in text.bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3);
            }
            hash
        }
        _ => function.bits(),
    }
}

/// What a description is, with everything needed from JavaScript copied out.
enum Described {
    Component {
        function: Root,
        props: Root,
        kind: u64,
        key: Option<Key>,
        /// Which of the last render's components this is, with `key`, as [`Cx::child_at`] has it.
        place: u64,
        /// Whether `__jsxMemo` made it: see `JsProps::memo`.
        memo: bool,
    },
    Fragment(Vec<Kid>),
    Host {
        role: Role,
        is_row: bool,
        key: Option<Key>,
        gathered: Gathered,
        kids: Vec<Kid>,
        own_text: Option<Text>,
        /// A list of children long enough to be looked at for what's the same in it as last time, with where they are, in place of
        /// `kids`: see [`KeptKids`].
        long: Option<(Value, Place)>,
    },
    Scroll(Root, u64),
    List(Root, u64),
}

/// Where something is among what a component renders, as React tells children apart: by its place in the array of
/// children it's in, holes and all, or by its key in that array, and the same for each array and element around it,
/// with the type of each element. It's that path, hashed, as it only has to be told from the others in one component.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Place(u64);

impl Place {
    /// Where what a component returns is.
    const OUTPUT: Place = Place(0x243F_6A88_85A3_08D3);
    /// The type of a fragment, or an array inside an array, which React takes to be the same.
    const FRAGMENT: u64 = 0x1319_8A2E_0370_7344;

    /// What's at `index` in the array of children here.
    fn at(self, index: u32) -> Place {
        Place(mix(self.0, u64::from(index)))
    }

    /// What has `key` in the array of children here.
    fn keyed(self, key: &Key) -> Place {
        let key = match key {
            Key::Int(number) => *number,
            other => {
                use std::hash::{Hash, Hasher};
                let mut hasher = Fnv::default();
                other.hash(&mut hasher);
                hasher.finish()
            }
        };
        Place(mix(self.0 ^ 0xA409_3822_299F_31D0, key))
    }

    /// Where the children of something of this type, here, are.
    fn inside(self, kind: u64) -> Place {
        Place(mix(self.0 ^ 0x082E_FA98_EC4E_6C89, kind))
    }
}

/// Folds two numbers into one, as foldhash does, so that a different number on either side comes to a different one.
fn mix(a: u64, b: u64) -> u64 {
    let full = u128::from(a ^ 0x4528_21E6_38D0_1377) * u128::from(b ^ 0xBE54_66CF_34E9_0C6C);
    (full as u64) ^ ((full >> 64) as u64)
}

/// What an element's tag comes to as the type of what's in it.
const fn tag_kind(tag: &str) -> u64 {
    let bytes = tag.as_bytes();
    let mut hash = 0xCBF2_9CE4_8422_2325u64;
    let mut at = 0;
    while at < bytes.len() {
        hash = (hash ^ bytes[at] as u64).wrapping_mul(0x0100_0000_01B3);
        at += 1;
    }
    hash
}

/// FNV, which is quick for what's short, as tags and keys are.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xCBF2_9CE4_8422_2325)
    }
}

impl std::hash::Hasher for Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01B3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// One child of an element, a fragment or what a component returns, with where it is.
#[derive(Clone, Copy)]
struct Kid {
    value: Value,
    /// Its `type`, read once on the way here, or `undefined` for text.
    kind: Value,
    /// The array of children it's in.
    scope: Place,
    /// Where it is in that array.
    index: u32,
}

impl Kid {
    /// Its place, by its key if it has one.
    fn place(&self, key: Option<&Key>) -> Place {
        key.map_or_else(|| self.scope.at(self.index), |key| self.scope.keyed(key))
    }
}

fn key_from(vm: &Vm, value: Value) -> Option<Key> {
    if value.is_number() {
        let number = value.as_number();
        if number >= 0.0 && number.fract() == 0.0 {
            return Some(Key::Int(number as u64));
        }
    }
    if value.is_nullish() {
        return None;
    }
    if let Some(text) = vm.string_text(value) {
        return Some(Key::from(text.as_ref()));
    }
    plain_value(vm, value).and_then(|plain| match plain {
        rsui::core::PropValue::Float(number) => Some(Key::from(number.to_string())),
        _ => None,
    })
}

/// Fixed native tag facts. No JS value, descriptor or cache is retained.
#[derive(Clone, Copy)]
struct NativeTag {
    role: Role,
    row: bool,
    text: bool,
    page: bool,
    kind: u64,
}

fn native_tag(tag: &str) -> Option<NativeTag> {
    macro_rules! tag {
        ($name:literal, $role:ident, $row:literal, $text:literal, $page:literal) => {
            NativeTag {
                role: Role::$role,
                row: $row,
                text: $text,
                page: $page,
                kind: const { tag_kind($name) },
            }
        };
    }
    Some(match tag {
        "window" => tag!("window", Window, false, false, false),
        "group" => tag!("group", Group, false, false, false),
        "row" => tag!("row", Group, true, false, false),
        "column" => tag!("column", Group, false, false, false),
        "text" => tag!("text", Text, false, true, false),
        "span" => tag!("span", Text, false, true, true),
        "button" => tag!("button", Button, false, true, true),
        "link" => tag!("link", Link, false, true, false),
        "input" => tag!("input", TextInput, false, false, true),
        "checkbox" => tag!("checkbox", Checkbox, false, true, false),
        "item" => tag!("item", ListItem, false, true, false),
        "image" => tag!("image", Image, false, false, false),
        "dialog" => tag!("dialog", Dialog, false, false, true),
        "menu" => tag!("menu", Menu, false, false, false),
        "menuitem" => tag!("menuitem", MenuItem, false, true, false),
        _ => return None,
    })
}

/// What's needed to read children.
#[derive(Clone, Copy)]
struct Names {
    fragment: Value,
    reordered: Value,
    order_name: Atom,
    type_name: Atom,
    key_name: Atom,
    kids_name: Atom,
}

fn names() -> Names {
    with_internals(|internals| Names {
        fragment: internals.fragment.get(),
        reordered: internals.reordered.get(),
        order_name: internals.order_name,
        type_name: internals.type_name,
        key_name: internals.key_name,
        kids_name: internals.kids_name,
    })
}

/// The children JSX gave an element or a fragment, as one array, in the places React has them. One child alone is what
/// its children are, as `props.children` would be, so `<group>{rows}</group>` has the rows themselves as its children.
fn kids_in(vm: &mut Vm, names: Names, kids: Value, scope: Place, out: &mut Vec<Kid>) {
    match vm.array_elements(kids) {
        Some(&[only]) => children_in(vm, names, only, scope, out),
        _ => children_in(vm, names, kids, scope, out),
    }
}

/// Children as React has them: an array is each of what's in it in its own place, a fragment with no key is what's in
/// it, and anything else is the one child, in the first place.
fn children_in(vm: &mut Vm, names: Names, children: Value, scope: Place, out: &mut Vec<Kid>) {
    if let Some(count) = vm.array_elements(children).map(<[Value]>::len) {
        out.reserve(count);
        for at in 0..count {
            // Reading a fragment could run a getter, which could change the array.
            let Some(&child) = vm
                .array_elements(children)
                .and_then(|elements| elements.get(at))
            else {
                break;
            };
            child_in(vm, names, child, scope, at as u32, false, out);
        }
        return;
    }
    child_in(vm, names, children, scope, 0, true, out);
}

/// The child at `index` in an array of children. Something that shows nothing, as `null`, `false` or `undefined`, keeps
/// its place, so what comes after it stays where it was. An array or a fragment is one place, with places of its own
/// inside, apart from a fragment with no key that's `alone`, which is what's in it.
fn child_in(
    vm: &mut Vm,
    names: Names,
    child: Value,
    scope: Place,
    index: u32,
    alone: bool,
    out: &mut Vec<Kid>,
) {
    let mut kid = Kid {
        value: child,
        kind: Value::UNDEFINED,
        scope,
        index,
    };
    if let Some(count) = vm.array_elements(child).map(<[Value]>::len) {
        let inside = kid.place(None).inside(Place::FRAGMENT);
        out.reserve(count);
        for at in 0..count {
            let Some(&element) = vm
                .array_elements(child)
                .and_then(|elements| elements.get(at))
            else {
                break;
            };
            child_in(vm, names, element, inside, at as u32, false, out);
        }
    } else if child.is_string() || child.is_number() {
        out.push(kid);
    } else if child.is_object() {
        kid.kind = vm
            .get_property(child, names.type_name)
            .unwrap_or(Value::UNDEFINED);
        if kid.kind == names.fragment {
            let key = vm
                .get_property(child, names.key_name)
                .unwrap_or(Value::UNDEFINED);
            let inside = if alone && key.is_nullish() {
                scope
            } else {
                let key = key_from(vm, key);
                kid.place(key.as_ref()).inside(Place::FRAGMENT)
            };
            if let Ok(inner) = vm.get_property(child, names.kids_name) {
                kids_in(vm, names, inner, inside, out);
            }
        } else if kid.kind == names.reordered {
            reordered_in(vm, names, child, kid, alone, out);
        } else {
            out.push(kid);
        }
    }
}

/// A list a component gave back that code has moved some of, in the order it says, each in the place it has in the list, as
/// the list itself would be (see `__Reordered` in layer.js), so that nothing in it starts again for being moved.
#[cold]
fn reordered_in(
    vm: &mut Vm,
    names: Names,
    value: Value,
    kid: Kid,
    alone: bool,
    out: &mut Vec<Kid>,
) {
    let (Ok(list), Ok(order)) = (
        vm.get_property(value, names.kids_name),
        vm.get_property(value, names.order_name),
    ) else {
        return;
    };
    let inside = if alone {
        kid.scope
    } else {
        kid.place(None).inside(Place::FRAGMENT)
    };
    let count = vm.array_elements(order).map_or(0, <[Value]>::len);
    for at in 0..count {
        let Some(index) = vm
            .array_elements(order)
            .and_then(|elements| elements.get(at))
            .filter(|index| index.is_number())
            .map(|index| index.as_number())
        else {
            continue;
        };
        if !(index >= 0.0 && index.fract() == 0.0) {
            continue;
        }
        let Some(&element) = vm
            .array_elements(list)
            .and_then(|elements| elements.get(index as usize))
        else {
            continue;
        };
        child_in(vm, names, element, inside, index as u32, false, out);
    }
}

/// What `at` is, which is where it is among what its component renders.
fn describe(vm: &mut Vm, at: Kid) -> Result<Described, Value> {
    let (names, props_name, function_name, memo_name) = with_internals(|internals| {
        (
            names(),
            internals.props_name,
            internals.function_name,
            internals.memo_name,
        )
    });
    let (description, kind) = (at.value, at.kind);
    let key = vm.get_property(description, names.key_name)?;
    let key = key_from(vm, key);
    if kind == names.fragment {
        let kids = vm.get_property(description, names.kids_name)?;
        let mut out = Vec::new();
        let inside = at.place(key.as_ref()).inside(Place::FRAGMENT);
        kids_in(vm, names, kids, inside, &mut out);
        return Ok(Described::Fragment(out));
    }
    let props = vm.get_property(description, props_name)?;
    if vm.is_function(kind) {
        // A keyed one is that key in the array it's in.
        let place = if key.is_some() {
            at.scope
        } else {
            at.scope.at(at.index)
        };
        let memo = vm.get_property(description, memo_name)? == Value::TRUE;
        return Ok(Described::Component {
            kind: kind_of(vm, kind, function_name),
            function: vm.root(kind),
            props: vm.root(props),
            key,
            place: place.0,
            memo,
        });
    }
    let (role, is_row, names_with_text, inside) = {
        let (native, page) = {
            let tag = vm.string_text(kind);
            let native = native_tag(tag.as_deref().unwrap_or("group"));
            let page = native.map_or_else(
                || tag.as_deref().is_some_and(could_be_page_element),
                |tag| tag.page,
            );
            (native, page)
        };
        if page && is_page_element(vm, kind, props)? {
            let made = with_internals(|internals| internals.web_description.clone());
            let made = vm.call_function(made.get(), Value::UNDEFINED, &[description])?;
            if made.is_object() {
                let kind = vm.get_property(made, names.type_name)?;
                return describe(
                    vm,
                    Kid {
                        value: made,
                        kind,
                        ..at
                    },
                );
            }
        }
        if let Some(tag) = native {
            (
                tag.role,
                tag.row,
                tag.text,
                at.place(key.as_ref()).inside(tag.kind),
            )
        } else {
            let tag = vm.string_text(kind);
            let tag = tag.as_deref().unwrap_or("group");
            let inside = at.place(key.as_ref()).inside(tag_kind(tag));
            match tag {
                "scroll" | "list" => {
                    let is_scroll = tag == "scroll";
                    let description = vm.root(description);
                    return Ok(if is_scroll {
                        Described::Scroll(description, inside.0)
                    } else {
                        Described::List(description, inside.0)
                    });
                }
                _ => (Role::Custom(CustomRole::new(tag)), false, false, inside),
            }
        }
    };
    let gathered = gather_props(vm, props)?;
    let kids_value = vm.get_property(description, names.kids_name)?;
    // Ordinary labels have one string child. Read it straight into the name rather
    // than allocate a Kid vector, walk it, join it and immediately clear it. Keep
    // this after all the same property reads: getters may mutate the children.
    // Only inspect an actual array's current contents; nested arrays, fragments,
    // numbers, objects and proxies still go through the general child walker.
    if names_with_text {
        if let Some(&[only]) = vm.array_elements(kids_value) {
            if let Some(text) = vm.string_text(only) {
                return Ok(Described::Host {
                    role,
                    is_row,
                    key,
                    gathered,
                    kids: Vec::new(),
                    own_text: Some(Text::from(text.as_ref())),
                    long: None,
                });
            }
        }
    }
    // One list by itself is what's in it, as `kids_in` has it.
    let list = match vm.array_elements(kids_value) {
        Some(&[only]) if vm.array_elements(only).is_some() => only,
        _ => kids_value,
    };
    if !names_with_text
        && vm
            .array_elements(list)
            .is_some_and(|list| list.len() >= KEPT_FROM)
    {
        return Ok(Described::Host {
            role,
            is_row,
            key,
            gathered,
            kids: Vec::new(),
            own_text: None,
            long: Some((list, inside)),
        });
    }
    let mut kids = Vec::new();
    kids_in(vm, names, kids_value, inside, &mut kids);
    // Text is the name of what it's in, as long as there's nothing else in there.
    let mut own_text = None;
    if names_with_text && !kids.is_empty() && kids.iter().all(|kid| !kid.value.is_object()) {
        own_text = Some(joined_text(vm, &kids));
        kids.clear();
    }
    Ok(Described::Host {
        role,
        is_row,
        key,
        gathered,
        kids,
        own_text,
        long: None,
    })
}

/// Text in pieces as one, without anything in between that it doesn't need. Most are one string, and short.
fn joined_text(vm: &mut Vm, kids: &[Kid]) -> Text {
    if let [only] = kids {
        if let Some(text) = vm.string_text(only.value) {
            return Text::from(text.as_ref());
        }
    }
    let mut joined = Text::new();
    for kid in kids {
        if let Some(text) = vm.string_text(kid.value) {
            joined.push_str(&text);
        } else if let Some(text) = vm.plain_text(kid.value) {
            joined.push_str(&text);
        }
    }
    joined
}

pub(crate) fn gather_props(vm: &mut Vm, props: Value) -> Result<Gathered, Value> {
    let mut gathered = Gathered {
        entries: HOST.with(|host| host.native_entries.take()),
        ..Gathered::default()
    };
    if props.is_object() {
        HOST.with(|host| -> Result<(), Value> {
            // It's taken out while it's in use, as reading a prop can get back here.
            let mut list = host.read_props.take();
            let result = vm.properties_into(props, &mut list).and_then(|()| {
                gathered.entries.reserve(list.len());
                let mut meanings = host.meanings.borrow_mut();
                list.iter().try_for_each(|&(name, value)| {
                    gather(vm, &mut meanings, &mut gathered, name, value)
                })
            });
            host.read_props.replace(list);
            result
        })?;
    }
    if gathered.events {
        // Handlers for events as the web has them, which the layer makes rsui's own of. One for `onBlur` hears focus too.
        let make = with_internals(|internals| internals.rsui_events.clone());
        let made = vm.call_function(make.get(), Value::UNDEFINED, &[props])?;
        let focus_too = vm.properties(made)?;
        if focus_too
            .iter()
            .any(|(name, _)| vm.atom_name(*name) == "onFocus")
        {
            gathered
                .handlers
                .retain(|(action, _)| *action != ActionKind::Focus);
        }
        HOST.with(|host| -> Result<(), Value> {
            let mut meanings = host.meanings.borrow_mut();
            focus_too.iter().try_for_each(|&(name, value)| {
                gather(vm, &mut meanings, &mut gathered, name, value)
            })
        })?;
    }
    Ok(gathered)
}

/// The tags a page has that mean nothing else here, as `rsui/web` has them. `span`, `button` and `input` are rsui's own too.
const PAGE_TAGS: &[&str] = &[
    "a",
    "abbr",
    "address",
    "article",
    "aside",
    "b",
    "bdi",
    "bdo",
    "blockquote",
    "canvas",
    "caption",
    "circle",
    "cite",
    "code",
    "data",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "ellipse",
    "em",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "g",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "line",
    "main",
    "mark",
    "meta",
    "nav",
    "noscript",
    "ol",
    "optgroup",
    "option",
    "output",
    "p",
    "path",
    "polygon",
    "polyline",
    "pre",
    "q",
    "rect",
    "s",
    "samp",
    "script",
    "search",
    "section",
    "select",
    "small",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "svg",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "tr",
    "u",
    "ul",
    "var",
];

/// Whether JSX means a page's element by this tag, which is drawn as CSS says. A tag only a page has always does. Of the
/// ones rsui has too, `span`, `button` and `input` do when they say what a page's would, with a `className` or a `type`.
fn is_page_element(vm: &mut Vm, kind: Value, props: Value) -> Result<bool, Value> {
    let (listed, span) = vm.string_text(kind).map_or((false, false), |tag| {
        (
            PAGE_TAGS.binary_search(&tag.as_ref()).is_ok(),
            tag == "span",
        )
    });
    if listed {
        return Ok(true);
    }
    if !props.is_object() {
        return Ok(false);
    }
    let (class, kind) = with_internals(|internals| (internals.class_name, internals.type_name));
    if !vm.get_property(props, class)?.is_nullish() {
        return Ok(true);
    }
    Ok(!span && !vm.get_property(props, kind)?.is_nullish())
}

#[cfg(test)]
mod page_tag_tests {
    /// `binary_search` only finds what's in order, and a tag that isn't found is drawn as rsui's own, not a page's.
    #[test]
    fn the_page_tags_are_in_order() {
        assert!(
            super::PAGE_TAGS.windows(2).all(|pair| pair[0] < pair[1]),
            "{:?}",
            super::PAGE_TAGS
        );
        for tag in [
            "ul", "ol", "li", "hgroup", "search", "data", "output", "bdi", "bdo", "var", "div",
        ] {
            assert!(super::PAGE_TAGS.binary_search(&tag).is_ok(), "{tag}");
        }
    }
}

/// Whether [`is_page_element`] needs asking, which is quick, as it's asked about every element.
fn could_be_page_element(tag: &str) -> bool {
    matches!(tag, "span" | "button" | "input" | "dialog") || PAGE_TAGS.binary_search(&tag).is_ok()
}

/// Keys for elements that need one to have handlers, and weren't given one. They only have to be different from each
/// other within a component's output, so they count up from a number nobody would use.
struct AutoKeys(u64);

impl AutoKeys {
    fn new() -> Self {
        Self(rsui::core::GENERATED_KEYS_FROM)
    }

    fn next(&mut self) -> Key {
        self.0 += 1;
        Key::Int(self.0)
    }
}

/// What a component that renders nothing is in the tree. It has to be there for the component to stay mounted, and
/// this takes up no room and nothing can reach it.
fn nothing() -> Node {
    Element::new(Role::Group)
        .prop("overlay", "fill")
        .prop("inert", true)
        .into_node()
}

/// A component's output as one node. It has to come to one element, so anything else goes in a group. `slot` is the component's,
/// for what's kept of a long list in it.
fn build_output(cx: &mut Cx<'_>, output: Value, keyed: bool, slot: Option<u32>) -> Node {
    // What was kept is only for this render's root, if that's a host with a long list in it again, which keeps it once more.
    let root = slot.map(|slot| {
        (
            slot,
            HOST.with(|host| host.kept_kids.borrow_mut().remove(&slot)),
        )
    });
    let previous = cx.previous_nodes();
    let previous = previous.as_ref().and_then(|old| old.root());
    let mut auto = AutoKeys::new();
    let alone = output.is_object().then(|| {
        with_vm(|vm| {
            if vm.array_elements(output).is_some() {
                return None;
            }
            let names = names();
            let kind = vm
                .get_property(output, names.type_name)
                .unwrap_or(Value::UNDEFINED);
            let unkeyed_fragment = (kind == names.fragment
                && vm
                    .get_property(output, names.key_name)
                    .is_ok_and(Value::is_nullish))
                || kind == names.reordered;
            (!unkeyed_fragment).then_some(kind)
        })
    });
    if let Some(Some(kind)) = alone {
        let first = Kid {
            value: output,
            kind,
            scope: Place::OUTPUT,
            index: 0,
        };
        return build_node(cx, first, keyed, root, &mut auto, previous);
    }
    if !output.is_object() && !output.is_string() && !output.is_number() {
        return nothing();
    }
    // An array that's returned, or a fragment with no key, is the children themselves, as React has it.
    let kids = with_vm(|vm| {
        let mut kids = Vec::new();
        children_in(vm, names(), output, Place::OUTPUT, &mut kids);
        kids
    });
    let mut group = fragment_group();
    for (index, kid) in kids.into_iter().enumerate() {
        group = group.child(build_kid(
            cx,
            kid,
            &mut auto,
            previous.and_then(|old| old.child(index)),
        ));
    }
    finish_node(group, previous)
}

/// What holds what's more than one thing, as a fragment is.
fn fragment_group() -> Element {
    let group = Element::new(Role::Group);
    if HOST.with(|host| host.fragments_fill.get()) {
        // CSS doesn't have a box here at all, so what's in it is laid out as part of what's around it, as `display: contents` has it:
        // a flex box puts each of them where it goes, and `height: 100%` inside means what's around this.
        group.prop("display", "contents")
    } else {
        group
    }
}

fn build_kid(
    cx: &mut Cx<'_>,
    kid: Kid,
    auto: &mut AutoKeys,
    previous: Option<PreviousNode<'_>>,
) -> Node {
    if kid.value.is_object() {
        return build_node(cx, kid, false, None, auto, previous);
    }
    let text = with_vm(|vm| vm.plain_text(kid.value)).unwrap_or_default();
    finish_node(Element::new(Role::Text).name(text), previous)
}

fn finish_node(element: Element, previous: Option<PreviousNode<'_>>) -> Node {
    match previous {
        Some(old) => old.reuse(element),
        None => element.into_node(),
    }
}

/// `root` is the slot of the component whose output this is the root of, if it is, with what was kept of what was in it.
fn build_node(
    cx: &mut Cx<'_>,
    kid: Kid,
    keyed_root: bool,
    root: Option<(u32, Option<KeptKids>)>,
    auto: &mut AutoKeys,
    previous: Option<PreviousNode<'_>>,
) -> Node {
    match with_vm(|vm| describe(vm, kid).map_err(|error| vm.error_text(error))) {
        Err(message) => {
            eprintln!("{message}");
            error_element(&message).into_node()
        }
        Ok(Described::Component {
            function,
            props,
            kind,
            key,
            place,
            memo,
        }) => {
            let keyed = key.is_some();
            let props = JsProps {
                function,
                props,
                kind,
                keyed,
                memo,
            };
            cx.child_at(key, place, js_component, props, same_kind)
        }
        Ok(Described::Fragment(kids)) => {
            let mut group = fragment_group();
            for (index, kid) in kids.into_iter().enumerate() {
                group = group.child(build_kid(
                    cx,
                    kid,
                    auto,
                    previous.and_then(|old| old.child(index)),
                ));
            }
            finish_node(group, previous)
        }
        Ok(Described::Scroll(description, place)) => {
            let owner = HOST.with(|host| host.owner.borrow().clone());
            let props = Deferred { description, owner };
            cx.child_at(None, place, scroll_component, props, |_, _| true)
        }
        Ok(Described::List(description, place)) => {
            let owner = HOST.with(|host| host.owner.borrow().clone());
            let props = Deferred { description, owner };
            cx.child_at(None, place, list_component, props, |_, _| true)
        }
        Ok(Described::Host {
            role,
            is_row,
            key,
            gathered,
            kids,
            own_text,
            long,
        }) => finish_node(
            build_host(
                cx,
                role,
                is_row,
                key,
                gathered,
                Kids { kids, long, root },
                own_text,
                keyed_root,
                auto,
                previous,
            ),
            previous,
        ),
    }
}

/// What's in a host: its children, or a long list of them that's to be looked at as it's built, with the slot of the component
/// whose output the host is the root of, if it is, and what was kept of what was in it.
struct Kids {
    kids: Vec<Kid>,
    long: Option<(Value, Place)>,
    root: Option<(u32, Option<KeptKids>)>,
}

#[allow(clippy::too_many_arguments)]
fn build_host(
    cx: &mut Cx<'_>,
    role: Role,
    is_row: bool,
    key: Option<Key>,
    gathered: Gathered,
    kids: Kids,
    own_text: Option<Text>,
    keyed_root: bool,
    auto: &mut AutoKeys,
    previous: Option<PreviousNode<'_>>,
) -> Element {
    let Gathered {
        mut entries,
        name,
        handlers,
        events: _,
    } = gathered;
    if is_row {
        static DIRECTION: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("direction"));
        entries.push((*DIRECTION, "row".into()));
    }
    let mut element = Element::new(role);
    // What shows and what it's called can be different, as with a button that says "Remove" on a row, which
    // a screen reader should say more about. An element shows its name unless it has something inside it.
    let mut shown = None;
    if let Some(name) = name {
        if let Some(text) = with_vm(|vm| vm.plain_text(name)) {
            element = element.name(text);
        }
        if let Some(text) = own_text {
            // Everything about how text looks, which would otherwise be lost on the way to what shows it.
            static LOOKS: LazyLock<[Symbol; 9]> = LazyLock::new(|| {
                [
                    Symbol::new("text_color"),
                    Symbol::new("font_size"),
                    Symbol::new("font_weight"),
                    Symbol::new("font_family"),
                    Symbol::new("font_style"),
                    Symbol::new("underline"),
                    Symbol::new("ellipsis"),
                    Symbol::new("no_wrap"),
                    Symbol::new("text_align"),
                ]
            });
            let looks: Props = entries
                .iter()
                .filter(|(name, _)| LOOKS.contains(name))
                .cloned()
                .collect();
            shown = Some(Element::new(Role::Text).name(text).props(looks));
        }
    } else if let Some(text) = own_text {
        element = element.name(text);
    }
    element = element.props(native_props(entries));
    if let Some(shown) = shown {
        element = element.child(shown);
    }
    // What can be selected is a paragraph, so a text's words go in as a part.
    if role == Role::Text && kids.kids.is_empty() && kids.long.is_none() && !element.name.is_empty()
    {
        static SELECTABLE: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("selectable"));
        if element.props.get_symbol(*SELECTABLE) == Some(&rsui::core::PropValue::Bool(true)) {
            element = element.selectable();
        }
    }
    element.key = key;
    if element.key.is_none() && !handlers.is_empty() && !keyed_root {
        element.key = Some(auto.next());
    }
    match kids.long {
        Some((list, scope)) => {
            element = build_long(cx, element, list, scope, kids.root, auto, previous)
        }
        None => {
            for kid in kids.kids {
                let old = previous.and_then(|old| old.child(element.children.len()));
                element = element.child(build_kid(cx, kid, auto, old));
            }
        }
    }
    wire_handlers(cx, element, handlers)
}

/// `element` with each of `handlers` called when what it's for happens to it.
fn wire_handlers(cx: &mut Cx<'_>, mut element: Element, handlers: Vec<(ActionKind, Root)>) -> Element {
    for (action, function) in handlers {
        element = cx.on(element, action, move |updates, input| {
            if updates.tracing() {
                trace_handler(updates, &function);
            }
            let mut handled_it = false;
            let mut passes_click = false;
            let came_with = updates.event();
            let failed = with_vm_letting_go(|vm| {
                let mut arguments = Vec::with_capacity(2);
                match input {
                    // A click says how far inside this the press landed, which its event says, for what was written for the web.
                    Some(rsui::core::PropValue::Int(within)) if action == ActionKind::Click => {
                        let make = with_internals(|internals| internals.new_click_event.clone());
                        let within = Value::number(*within as f64);
                        let info =
                            came_with.map_or(Value::UNDEFINED, |event| event_value(vm, event));
                        if let Ok(event) =
                            vm.call_function(make.get(), Value::UNDEFINED, &[within, info])
                        {
                            arguments.push(event);
                        }
                    }
                    Some(rsui::core::PropValue::Bool(value)) => arguments.push(Value::bool(*value)),
                    Some(rsui::core::PropValue::Int(value)) => {
                        arguments.push(Value::number(*value as f64));
                    }
                    Some(rsui::core::PropValue::Float(value)) => {
                        arguments.push(Value::number(*value));
                    }
                    // Paths can have spaces in them, so they come a line each, as one list.
                    Some(rsui::core::PropValue::Str(text)) if action == drop_files() => {
                        let paths = text.split('\n').map(|path| vm.new_string(path)).collect();
                        arguments.push(vm.new_array(paths));
                    }
                    // What's said about the pointer comes in words, each of which is an argument.
                    Some(rsui::core::PropValue::Str(text))
                        if matches!(action, ActionKind::Custom(_)) =>
                    {
                        for word in text.split(' ') {
                            arguments.push(match word.parse::<f64>() {
                                Ok(number) => Value::number(number),
                                Err(_) => vm.new_string(word),
                            });
                        }
                    }
                    Some(rsui::core::PropValue::Str(text)) => arguments.push(vm.new_string(text)),
                    None if action == ActionKind::Click => {
                        let make = with_internals(|internals| internals.new_click_event.clone());
                        let info =
                            came_with.map_or(Value::UNDEFINED, |event| event_value(vm, event));
                        let within = Value::number(0.0);
                        if let Ok(event) =
                            vm.call_function(make.get(), Value::UNDEFINED, &[within, info])
                        {
                            arguments.push(event);
                        }
                    }
                    None => {}
                }
                // What came with it goes after what it's always been given, for what's written for the web.
                if action != ActionKind::Click {
                    if let Some(event) = came_with {
                        let event = event_value(vm, event);
                        arguments.push(event);
                    }
                }
                match vm.call_function(function.get(), Value::UNDEFINED, &arguments) {
                    Ok(result) => {
                        handled_it = result == Value::TRUE;
                        passes_click = action == ActionKind::Click
                            && result.is_object()
                            && result == with_internals(|internals| internals.pass_click.get());
                        None
                    }
                    Err(error) => Some(vm.error_text(error)),
                }
            });
            if let Some(message) = failed {
                report(message);
            }
            pass_on_what_was_said(updates);
            // Keys go on to what's around them, as they do on the web, unless this says that's the end of it.
            if (action == ActionKind::KeyPress && !handled_it) || passes_click {
                updates.pass_on();
            }
            pump(updates);
        });
    }
    element
}

/// A long list of children, as `children_in` would have them, built into `element`. When it's the root of a component's output,
/// what's in it that's the same description as the last time it was built, in the same place or with a key of its own, is a child
/// component given what it was, which is kept as it is ([`Cx::keep_child`]). The rest is described and built as usual. What's
/// built is then kept for the next time: see [`KeptKids`].
#[allow(clippy::too_many_arguments)]
fn build_long(
    cx: &mut Cx<'_>,
    mut element: Element,
    list: Value,
    scope: Place,
    root: Option<(u32, Option<KeptKids>)>,
    auto: &mut AutoKeys,
    previous: Option<PreviousNode<'_>>,
) -> Element {
    let names = names();
    let values: Vec<Value> = with_vm(|vm| {
        vm.array_elements(list)
            .map(<[Value]>::to_vec)
            .unwrap_or_default()
    });
    let (slot, kept) = root.map_or((None, None), |(slot, kept)| (Some(slot), kept));
    let kept = kept.filter(|kept| kept.scope == scope);
    let old: Vec<Value> = kept.as_ref().map_or_else(Vec::new, |kept| {
        with_vm(|vm| {
            vm.array_elements(kept.list.get())
                .map(<[Value]>::to_vec)
                .unwrap_or_default()
        })
    });
    // Where each of the old list was: as far along as the last one found was, as what's after something put in or taken out is
    // all that much further along, or near that, or, once something's further away, by what it is. What's there twice isn't kept.
    let mut shift = 0isize;
    let mut found: Option<std::collections::HashMap<u64, usize>> = None;
    let mut outputs = Vec::with_capacity(values.len());
    let mut kids = Vec::new();
    for (index, &value) in values.iter().enumerate() {
        if let Some(kept) = &kept {
            let near = (0..=NEAR_KEPT)
                .flat_map(|step| [step, -step])
                .find_map(|step| {
                    let at = usize::try_from(index as isize + shift + step).ok()?;
                    (old.get(at) == Some(&value)).then_some(at)
                });
            let at = if let Some(at) = near {
                shift = at as isize - index as isize;
                Some(at)
            } else {
                let found = found.get_or_insert_with(|| {
                    let mut found = std::collections::HashMap::with_capacity(old.len());
                    for (at, one) in old.iter().enumerate() {
                        if one.is_object() && found.insert(one.bits(), at).is_some() {
                            found.insert(one.bits(), usize::MAX);
                        }
                    }
                    found
                });
                let at = found
                    .get(&value.bits())
                    .copied()
                    .filter(|&at| at != usize::MAX);
                if let Some(at) = at {
                    shift = at as isize - index as isize;
                }
                at
            };
            if let Some(Some((child, keyed))) = at
                .map(|at| (at, kept.outputs[at]))
                .filter(|&(at, output)| output.is_some_and(|(_, keyed)| keyed || at == index))
                .map(|(_, output)| output)
            {
                if let Some(node) = cx.keep_child(child as usize) {
                    outputs.push(Some((element.children.len() as u32, keyed)));
                    element = element.child(node);
                    continue;
                }
            }
        }
        kids.clear();
        let component = with_vm(|vm| {
            child_in(vm, names, value, scope, index as u32, false, &mut kids);
            // Only an element that's itself a component, rather than one in an array or a fragment, whose place is that array's.
            match kids.as_slice() {
                [one] if one.value == value && vm.is_function(one.kind) => Some(
                    vm.get_property(one.value, names.key_name)
                        .is_ok_and(|key| key_from(vm, key).is_some()),
                ),
                _ => None,
            }
        });
        let first = element.children.len() as u32;
        for kid in kids.drain(..) {
            let old = previous.and_then(|old| old.child(element.children.len()));
            element = element.child(build_kid(cx, kid, auto, old));
        }
        outputs.push(component.map(|keyed| (first, keyed)));
    }
    if let Some(slot) = slot {
        let list = with_vm(|vm| vm.root(list));
        HOST.with(|host| {
            host.kept_kids.borrow_mut().insert(
                slot,
                KeptKids {
                    list,
                    scope,
                    outputs,
                },
            )
        });
    }
    element
}

/// How many places there are for sets of props to be kept in, for elements to share them, and how many sets each place keeps: see
/// [`native_props`].
const RECENT_PLACES: usize = 16;
const RECENT_WAYS: usize = 2;
/// The most props in a set that's kept, and the most text in all of its values.
const RECENT_PROPS_LONGEST: usize = 32;
const RECENT_PROPS_TEXT: usize = 256;

/// The last few small sets of props made, each in the place what's in it says, for an element given the same ones as another to share
/// that one's: a document's paragraphs of one kind have the same props, as a list's rows mostly do, and each would be a set of its own.
/// Each place keeps two, the one used last and the one before it, so that two kinds that come by turns, as a paragraph and the span
/// in it do, are both kept even when they're in the same place.
#[derive(Default)]
struct RecentProps {
    sets: [[Props; RECENT_WAYS]; RECENT_PLACES],
    /// Which of each place's two was used last.
    last: [u8; RECENT_PLACES],
}

impl RecentProps {
    fn clear(&mut self) {
        for place in &mut self.sets {
            for one in place {
                one.clear();
            }
        }
        self.last = [0; RECENT_PLACES];
    }

    /// How many sets are kept.
    fn len(&self) -> usize {
        self.sets
            .iter()
            .flatten()
            .filter(|one| !one.is_empty())
            .count()
    }

    fn iter(&self) -> impl Iterator<Item = &Props> {
        self.sets.iter().flatten()
    }
}

/// Where a set of props goes among those kept, from its names and a little of each value: a set's text is compared in full only when
/// one's found there.
fn recent_place(entries: &[(Symbol, rsui::core::PropValue)]) -> usize {
    use rsui::core::PropValue;
    let mut hash = 0xcbf2_9ce4_8422_2325u64 ^ entries.len() as u64;
    for (name, value) in entries {
        let said = match value {
            PropValue::Bool(on) => u64::from(*on),
            PropValue::Int(number) => *number as u64,
            PropValue::Float(number) => number.to_bits(),
            PropValue::Str(text) => {
                let bytes = text.as_bytes();
                let mut start = [0u8; 8];
                let first = bytes.len().min(8);
                start[..first].copy_from_slice(&bytes[..first]);
                u64::from_le_bytes(start) ^ ((bytes.len() as u64) << 56)
            }
        };
        hash = (hash ^ name.address() as u64).wrapping_mul(0x0100_0000_01b3);
        hash = (hash ^ said).wrapping_mul(0x0100_0000_01b3);
    }
    (hash ^ (hash >> 29) ^ (hash >> 47)) as usize % RECENT_PLACES
}

/// Whether a kept set is these, which are in order by name with one value each, as a set keeps them.
fn same_entries(kept: &Props, entries: &[(Symbol, rsui::core::PropValue)]) -> bool {
    kept.len() == entries.len()
        && kept
            .iter()
            .zip(entries)
            .all(|((name, value), (other, said))| name == *other && value == said)
}

/// All property reads/conversions have finished before this runs. Equal native values share immutable storage without trusting JS
/// object identity: a set the same as one of the last few kept is that one (see [`RecentProps`]). A big set, or one with much text,
/// isn't kept, so nothing large is held on to after what had it is gone.
fn native_props(mut entries: Vec<(Symbol, rsui::core::PropValue)>) -> Props {
    HOST.with(|host| {
        // In order by name, the last of each name winning, as a set has them: see `Props::replace_from`.
        entries.sort_by_key(|(name, _)| *name);
        entries.reverse();
        entries.dedup_by_key(|(name, _)| *name);
        entries.reverse();
        let mut recent = host.recent_props.borrow_mut();
        let place = recent_place(&entries);
        let found = (0..RECENT_WAYS).find(|&way| same_entries(&recent.sets[place][way], &entries));
        let props = if let Some(way) = found {
            entries.clear();
            recent.last[place] = way as u8;
            recent.sets[place][way].clone()
        } else {
            let mut made = Props::new();
            made.replace_from(&mut entries);
            let text: usize = made
                .iter()
                .map(|(_, value)| match value {
                    rsui::core::PropValue::Str(text) => text.len(),
                    _ => 0,
                })
                .sum();
            if !made.is_empty() && made.len() <= RECENT_PROPS_LONGEST && text <= RECENT_PROPS_TEXT {
                // In place of the one of the two that wasn't used last.
                let way = 1 - usize::from(recent.last[place]);
                recent.sets[place][way] = made.clone();
                recent.last[place] = way as u8;
            }
            made
        };
        drop(recent);
        if entries.capacity() <= 32 {
            host.native_entries.replace(entries);
        }
        props
    })
}

/// Says which function is handling what's been done, for what's tracing.
#[cold]
#[inline(never)]
fn trace_handler(updates: &mut Updates<'_>, function: &Root) {
    // What the handler sets says where, which the layer only does once it knows.
    follow_tracing(true);
    {
        let made = with_vm(|vm| vm.function_origin(function.get()));
        if let Some(made) = made {
            let name = if made.function.is_empty() {
                "the handler".to_owned()
            } else {
                format!("`{}`", made.function)
            };
            updates.trace_handler(format!("{name} at {}", crate::why::short_place(&made)));
        }
    }
}

/// A description of a scrolling area or a long list, which has state of its own in the runtime.
struct Deferred {
    description: Root,
    /// The JavaScript component this is part of, for anything new in here when it renders by itself.
    owner: Option<Root>,
}

impl PartialEq for Deferred {
    fn eq(&self, other: &Self) -> bool {
        self.description.get() == other.description.get()
    }
}

fn with_owner<R>(owner: &Option<Root>, action: impl FnOnce() -> R) -> R {
    let parent = HOST.with(|host| host.owner.replace(owner.clone()));
    let result = action();
    HOST.with(|host| *host.owner.borrow_mut() = parent);
    result
}

fn scroll_component(cx: &mut Cx<'_>, props: &Deferred) -> Element {
    cx.trace_name("scroll");
    with_owner(&props.owner, || {
        let description = props.description.get();
        let read = with_vm(|vm| -> Result<_, Value> {
            let (names, props_name) = with_internals(|internals| (names(), internals.props_name));
            let key = vm.get_property(description, names.key_name)?;
            let key = key_from(vm, key);
            let own = vm.get_property(description, props_name)?;
            let gathered = gather_props(vm, own)?;
            let axis = if own.is_object() {
                let name = with_internals(|internals| internals.axis_name);
                let axis = vm.get_property(own, name)?;
                vm.string_text(axis).map(|axis| axis.into_owned())
            } else {
                None
            };
            // What's told where it's scrolled to, each time it's scrolled, as a page's `scroll` event has it.
            let told = if own.is_object() {
                let name = vm.atom("scrollTold");
                let told = vm.get_property(own, name)?;
                vm.is_function(told).then(|| vm.root(told))
            } else {
                None
            };
            let kids_value = vm.get_property(description, names.kids_name)?;
            let mut kids = Vec::new();
            kids_in(vm, names, kids_value, Place::OUTPUT, &mut kids);
            Ok((key, gathered, axis, kids, told))
        });
        let (key, mut gathered, axis, kids, told) = match read {
            Ok(read) => read,
            Err(error) => {
                let message = with_vm(|vm| vm.error_text(error));
                return error_element(&message);
            }
        };
        static AXIS: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("axis"));
        gathered.entries.retain(|(name, _)| *name != *AXIS);
        // Where code wants it to go, each time `scrollAsk` is a new number, as setting `scrollTop` does.
        static ASK: LazyLock<[Symbol; 3]> = LazyLock::new(|| {
            [
                Symbol::new("scroll_ask"),
                Symbol::new("scroll_to_y"),
                Symbol::new("scroll_to_x"),
            ]
        });
        let mut control = rsui::runtime::ScrollControl::default();
        gathered.entries.retain(|(name, value)| {
            let number = match value {
                rsui::core::PropValue::Float(number) => Some(*number),
                rsui::core::PropValue::Int(number) => Some(*number as f64),
                _ => None,
            };
            if *name == ASK[0] {
                control.request = number.map_or(0, |number| number as u64);
            } else if *name == ASK[1] {
                control.y = number;
            } else if *name == ASK[2] {
                control.x = number;
            } else {
                return true;
            }
            false
        });
        if let Some(told) = told {
            control.moved = Some(std::rc::Rc::new(
                move |updates: &mut Updates<'_>, down: f64, across: f64| {
                    let failed = with_vm_letting_go(|vm| {
                        let args = [Value::number(down), Value::number(across)];
                        vm.call_function(told.get(), Value::UNDEFINED, &args)
                            .err()
                            .map(|error| vm.error_text(error))
                    });
                    if let Some(message) = failed {
                        report(message);
                    }
                    pass_on_what_was_said(updates);
                    pump(updates);
                },
            ));
        }
        let mut auto = AutoKeys::new();
        // The key it ends up with is the one its handlers go with, as what scrolls keeps them, hovered and clicked and all.
        let key = key.unwrap_or_else(|| Key::from("scroll"));
        let content = build_host(
            cx,
            Role::Group,
            false,
            Some(key.clone()),
            gathered,
            Kids {
                kids,
                long: None,
                root: None,
            },
            None,
            false,
            &mut auto,
            None,
        );
        let axis = match axis.as_deref() {
            Some("horizontal") => ScrollAxis::Horizontal,
            Some("both") => ScrollAxis::Both,
            _ => ScrollAxis::Vertical,
        };
        cx.scroll_area_with(key, axis, content, control)
    })
}

/// Has what's logged here collected from now on, rather than going to the standard output.
pub(crate) fn send_logs_to(logs: Logs) {
    HOST.with(|host| *host.logs.borrow_mut() = Some(logs));
}

/// Says something about what the app's doing, to whoever's watching.
pub(crate) fn say(text: String) {
    HOST.with(|host| match &*host.logs.borrow() {
        Some(logs) => crate::agent::keep_log(logs, &text),
        None => {
            println!("  {text}");
            crate::live::heard(&text);
        }
    });
}

/// Lets go of what's kept for an app that's gone, before another starts on this thread. Its windows have to be gone by now.
pub(crate) fn forget_what_ran() {
    HOST.with(|host| {
        host.native_entries.replace(Vec::new());
        host.recent_props.borrow_mut().clear();
        host.timer_due.set(None);
        host.main_runtime.set(0);
        host.remotes.borrow_mut().clear();
        host.root_state.set(None);
        host.dirty.borrow_mut().clear();
        host.document_interests.borrow_mut().take();
        *host.error.borrow_mut() = None;
        host.error_changed.set(false);
    });
}

/// Works out what's written, as code at the top of a script, the way a browser's console does, and says what it came to.
pub(crate) fn evaluate(code: &str) -> Result<String, String> {
    let said = HOST.with(|host| {
        let Ok(mut engine) = host.engine.try_borrow_mut() else {
            return Err("the app's in the middle of running its code".to_owned());
        };
        let engine = engine.as_mut().ok_or("the app has nothing running")?;
        // It comes to what its last statement does, as `eval` gives.
        let ran = engine.eval(code, "console");
        let shown = match ran {
            Ok(value) => Ok(engine.display(value)),
            Err(error) => Err(error.message),
        };
        let _ = engine.run_jobs();
        // What it asked for is done as what any of the app's own code asks for is, rather than once something else happens:
        // `button.focus()` gives it the keyboard before the next key, and state it set is drawn, with its timers waited for.
        wake_runtime_with(host, host.main_runtime.get(), after_outside_code);
        shown
    });
    // What it asked of the window, as `element.focus()` does, is done at the app's next update, as it would be after a handler
    // or a timer ran, rather than waiting for something else to happen: a key pressed right after would go to what had the
    // keyboard before.
    pump_soon();
    said
}

/// Tells the app's code that things have started or stopped moving, as `animations on` and `off` in `--do` say, which no
/// component's rendering would tell it: CSS that asks `prefers-reduced-motion` is worked out again, and `matchMedia` lists
/// watching it hear (`__rsuiMotionChanged` in web.js).
pub(crate) fn motion_changed(on: bool) {
    let was = HOST.with(|host| host.animations_on.replace(on));
    if was != on {
        let _ = evaluate("globalThis.__rsuiMotionChanged?.()");
    }
}

/// What code run from outside the app, as a console's, leaves to do: what it set going, and what it changed or asked for.
fn after_outside_code(updates: &mut Updates<'_>) {
    pump(updates);
    apply_dirty!(updates);
}

/// When the next JavaScript timer's due, by the app's clock, if one's waiting.
pub(crate) fn next_timer_at() -> Option<f64> {
    let wait = HOST.with(|host| {
        let engine = host.engine.try_borrow().ok()?;
        engine.as_ref()?.next_timer_in()
    })?;
    Some(rsui_js::now_ms() + wait)
}

/// Has what's waiting in JavaScript, and its timers that are due, run at the app's next update.
pub(crate) fn pump_soon() -> bool {
    HOST.with(|host| {
        host.remote
            .lock()
            .expect("lock is not poisoned")
            .as_ref()
            .is_some_and(|remote| remote.send(pump))
    })
}

/// Whether a JavaScript timer is waiting to go off.
pub(crate) fn timer_is_waiting() -> bool {
    HOST.with(|host| host.timer_due.get().is_some())
}

/// Whether a JavaScript timer is due within this long.
pub(crate) fn timer_due_within(wait: Duration) -> bool {
    HOST.with(|host| {
        host.timer_due
            .get()
            .is_some_and(|due| due <= Instant::now() + wait)
    })
}

/// Has dialogs on this thread not come up from now on, as there's nobody there for them.
pub(crate) fn answer_dialogs_here() {
    HOST.with(|host| *host.dialog_answers.borrow_mut() = Some(Default::default()));
}

/// Says what the next dialog gets, where they don't come up, with `None` being cancelled.
pub(crate) fn answer_next_dialog(answer: Option<String>) {
    HOST.with(|host| {
        if let Some(answers) = host.dialog_answers.borrow_mut().as_mut() {
            answers.push_back(answer);
        }
    });
}

/// What the next dialog that doesn't come up gets: nothing if dialogs come up here, and otherwise the answer the steps gave,
/// which is `None`, being cancelled, if they gave none.
pub(crate) fn take_dialog_answer() -> Option<Option<String>> {
    HOST.with(|host| {
        host.dialog_answers
            .borrow_mut()
            .as_mut()
            .map(|answers| answers.pop_front().flatten())
    })
}

/// Whether anything's happening on another thread that will come back to JavaScript by itself. A socket waiting to hear something
/// doesn't count, as it could wait for good, and nothing should wait for that.
pub(crate) fn is_working() -> bool {
    HOST.with(|host| host.engine.borrow().as_ref().is_some_and(Engine::is_busy))
}

/// Collects all of the engine's garbage on this thread, then counts what it's still keeping. It's for finding out why
/// an app's memory grows; see `census.rs` in rsui-js.
pub fn census() -> rsui_js::Census {
    HOST.with(|host| {
        let mut engine = host.engine.borrow_mut();
        engine.as_mut().expect("the app has been loaded").census()
    })
}

/// What the engine on this thread uses, and what its lists of places take from the system: for `memory` in steps.
pub fn engine_memory() -> (rsui_js::MemoryUse, rsui_js::Footprint, u32) {
    HOST.with(|host| {
        let engine = host.engine.borrow();
        let engine = engine.as_ref().expect("the app has been loaded");
        (engine.memory(), engine.footprint(), engine.collections())
    })
}

/// Captures mappings while the thread's engine still owns its JIT allocations.
#[cfg(all(windows, feature = "sampling-profile", feature = "jit"))]
pub(crate) fn sampling_jit_code() -> Vec<rsui_js::JitCode> {
    HOST.with(|host| host.engine.borrow().as_ref().map(Engine::jit_code).unwrap_or_default())
}

/// When a quiet window collects all of the JavaScript's garbage, as `RSUI_IDLE_GC` says. By default it's
/// [`IdleCollect::Moment`]: in a real window that's 6-11% less private memory for an editor with 10,000 paragraphs, and the
/// first key after costs 1-2 ms more than the window's giving memory back already does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum IdleCollect {
    /// `0`: never.
    Off,
    /// `1`, or nothing said: each time things have gone quiet for a few seconds ([`rsui::Quiet::Moment`]).
    Moment,
    /// `long`: when a window lets go of the GPU ([`rsui::Quiet::Long`]).
    Long,
    /// `plain`: as `1`, but only collecting, without giving pages back or shrinking storage, to tell their costs apart.
    Plain,
}

fn idle_collect() -> IdleCollect {
    static MODE: std::sync::OnceLock<IdleCollect> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("RSUI_IDLE_GC").as_deref() {
        Ok("0") => IdleCollect::Off,
        Ok("1") => IdleCollect::Moment,
        Ok("long") => IdleCollect::Long,
        Ok("plain") => IdleCollect::Plain,
        _ => IdleCollect::Moment,
    })
}

/// What a quiet window does as well, as [`idle_collect`] says: collects all of the engine's garbage, gives back empty pages
/// and shrinks storage (`Engine::collect_garbage_when_quiet`), unless JavaScript is running. Says whether it did.
fn collect_when_quiet(quiet: rsui::Quiet) -> bool {
    let mode = idle_collect();
    let now = match (mode, quiet) {
        (IdleCollect::Moment | IdleCollect::Plain, rsui::Quiet::Moment) => true,
        (IdleCollect::Long, rsui::Quiet::Long) => true,
        _ => false,
    };
    if !now {
        return false;
    }
    HOST.with(|host| {
        if let Ok(mut engine) = host.engine.try_borrow_mut()
            && let Some(engine) = engine.as_mut()
        {
            if mode == IdleCollect::Plain {
                engine.collect_garbage();
            } else {
                engine.collect_garbage_when_quiet();
            }
            return true;
        }
        false
    })
}

/// Collects all of the engine's garbage on this thread, as `collect` in steps asks.
pub fn collect_all() {
    HOST.with(|host| {
        let mut engine = host.engine.borrow_mut();
        engine
            .as_mut()
            .expect("the app has been loaded")
            .collect_garbage();
    })
}

/// The timers the engine on this thread has waiting, grouped by what they run, most first: for the `timers` step.
pub fn timer_summary() -> Vec<(usize, String)> {
    HOST.with(|host| {
        host.engine
            .borrow()
            .as_ref()
            .map(rsui_js::Engine::timer_summary)
            .unwrap_or_default()
    })
}

/// The same, with plain objects counted by their properties' names: see `census_by_keys` in rsui-js.
#[cfg(feature = "alloc-owners")]
pub fn census_by_keys() -> rsui_js::Census {
    HOST.with(|host| {
        let mut engine = host.engine.borrow_mut();
        engine
            .as_mut()
            .expect("the app has been loaded")
            .census_by_keys()
    })
}

/// The shortest way from a root to each of up to `limit` objects the engine on this thread is keeping that are counted
/// as something with `label` in it, as [`census`] counts them.
pub fn retainers(label: &str, limit: usize) -> Vec<String> {
    HOST.with(|host| {
        let mut engine = host.engine.borrow_mut();
        engine
            .as_mut()
            .expect("the app has been loaded")
            .retainers(label, limit)
    })
}

/// How big each of the lists this thread keeps for its app is, by name, for seeing that none of them grows for good.
pub fn tables() -> Vec<(&'static str, usize)> {
    let mut out = HOST.with(|host| {
        let instances = host.instances.borrow();
        vec![
            ("instances", instances.len()),
            (
                "live instances",
                instances.iter().filter(|one| one.is_some()).count(),
            ),
            ("free instance slots", host.free_slots.borrow().len()),
            ("instance owners", host.owners.borrow().len()),
            ("remotes", host.remotes.borrow().len()),
            ("dirty", host.dirty.borrow().len()),
            ("read props", host.read_props.borrow().capacity()),
            (
                "native prop scratch",
                host.native_entries.borrow().capacity(),
            ),
            ("recent props", host.recent_props.borrow().len()),
            ("meanings", host.meanings.borrow().len()),
        ]
    });
    out.extend(crate::layout::table_sizes());
    out
}

/// What makes one row of a long list.
struct RowProps {
    render: Root,
    index: usize,
    owner: Option<Root>,
}

impl PartialEq for RowProps {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.render.get() == other.render.get()
    }
}

fn row_component(cx: &mut Cx<'_>, props: &RowProps) -> Node {
    cx.trace_name("a row of a list");
    with_owner(&props.owner, || {
        let (output, mark) = with_vm(|vm| {
            let mark = vm.new_values_mark();
            let output = vm
                .call_function(
                    props.render.get(),
                    Value::UNDEFINED,
                    &[Value::int_or_double(props.index)],
                )
                .map_err(|error| vm.error_text(error));
            (output, mark)
        });
        let node = match output {
            Ok(output) => build_output(cx, output, true, None),
            Err(message) => {
                eprintln!("{message}");
                error_element(&message).into_node()
            }
        };
        with_vm(|vm| vm.release_new_values(mark));
        node
    })
}

fn list_component(cx: &mut Cx<'_>, props: &Deferred) -> Element {
    cx.trace_name("list");
    with_owner(&props.owner, || {
        let description = props.description.get();
        let read = with_vm(|vm| -> Result<_, Value> {
            let (key_name, props_name) =
                with_internals(|internals| (internals.key_name, internals.props_name));
            let key = vm.get_property(description, key_name)?;
            let key = key_from(vm, key);
            let own = vm.get_property(description, props_name)?;
            let number = |vm: &mut Vm, name: Atom, fallback: f64| -> Result<f64, Value> {
                let value = vm.get_property(own, name)?;
                Ok(if value.is_number() {
                    value.as_number()
                } else {
                    fallback
                })
            };
            let (count_name, visible_name, overscan_name, render_name, reveal_name) =
                with_internals(|internals| {
                    (
                        internals.count_name,
                        internals.visible_name,
                        internals.overscan_name,
                        internals.render_row_name,
                        internals.reveal_name,
                    )
                });
            let len = number(vm, count_name, 0.0)? as usize;
            let visible = number(vm, visible_name, 24.0)? as usize;
            let overscan = number(vm, overscan_name, 2.0)? as usize;
            let reveal = vm.get_property(own, reveal_name)?;
            let reveal = (reveal.is_number() && reveal.as_number() >= 0.0)
                .then(|| reveal.as_number() as usize);
            let render = vm.get_property(own, render_name)?;
            let render = vm.root(render);
            let mut gathered = gather_props(vm, own)?;
            static UNWANTED: LazyLock<[Symbol; 5]> = LazyLock::new(|| {
                [
                    Symbol::new("count"),
                    Symbol::new("visible"),
                    Symbol::new("overscan"),
                    Symbol::new("render_row"),
                    Symbol::new("reveal"),
                ]
            });
            gathered
                .entries
                .retain(|(name, _)| !UNWANTED.contains(name));
            Ok((key, len, visible, overscan, render, gathered, reveal))
        });
        let (key, len, visible, overscan, render, gathered, reveal) = match read {
            Ok(read) => read,
            Err(error) => {
                let message = with_vm(|vm| vm.error_text(error));
                return error_element(&message);
            }
        };
        let owner = props.owner.clone();
        let mut gathered = gathered;
        // A list with an `itemWidth` is a grid of items that wraps itself, as many across as fit.
        static ITEM_WIDTH: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("item_width"));
        let item_width = gathered.entries.iter().find(|(name, _)| *name == *ITEM_WIDTH).and_then(|(_, value)| match value {
            rsui::core::PropValue::Float(width) => Some(*width),
            rsui::core::PropValue::Int(width) => Some(*width as f64),
            _ => None,
        });
        gathered.entries.retain(|(name, _)| *name != *ITEM_WIDTH);
        // `onResize` is told how many rows fit and how many items across, as the window measures them, which is the list's own
        // `resize`. Its `scroll` is its own too, so an `onScroll` given it isn't called.
        let mut fits = None;
        gathered.handlers.retain(|(action, function)| match action {
            ActionKind::Resize => {
                fits = Some(function.clone());
                false
            }
            ActionKind::Scroll => false,
            _ => true,
        });
        let fits = fits.map(|told| {
            std::rc::Rc::new(move |updates: &mut Updates<'_>, rows: f64, across: usize| {
                let failed = with_vm_letting_go(|vm| {
                    let args = [Value::number(rows), Value::number(across as f64)];
                    vm.call_function(told.get(), Value::UNDEFINED, &args)
                        .err()
                        .map(|error| vm.error_text(error))
                });
                if let Some(message) = failed {
                    report(message);
                }
                pass_on_what_was_said(updates);
                pump(updates);
            }) as std::rc::Rc<dyn Fn(&mut Updates<'_>, f64, usize)>
        });
        let list = cx.virtual_list_with(
            key.unwrap_or_else(|| Key::from("list")),
            ListWindow {
                len,
                visible,
                overscan,
                reveal,
            },
            rsui::runtime::ListOptions { item_width, fits },
            |cx, index| {
                cx.keyed(
                    index,
                    row_component,
                    RowProps {
                        render: render.clone(),
                        index,
                        owner: owner.clone(),
                    },
                )
            },
        );
        let style: Props = gathered.entries.into_iter().collect();
        let list = Element {
            props: style.merged(&list.props),
            ..list
        };
        // What else it's given to hear, as a click on it below its rows, which anything else would hear.
        wire_handlers(cx, list, gathered.handlers)
    })
}

/// What every window's runtime is told about what happens to it.
fn set_up_runtime(cx: &mut Cx<'_>) {
    cx.on_clipboard(|updates, text| {
        let pasted = with_internals(|internals| internals.pasted.clone());
        let failed = with_vm_letting_go(|vm| {
            let text = vm.new_string(text.unwrap_or(""));
            vm.call_function(pasted.get(), Value::UNDEFINED, &[text])
                .err()
                .map(|error| vm.error_text(error))
        });
        if let Some(message) = failed {
            report(message);
        }
        pump(updates);
    });
    cx.on_color_scheme(|updates, dark| {
        let changed = with_internals(|internals| internals.color_scheme_changed.clone());
        let failed = with_vm_letting_go(|vm| {
            vm.call_function(changed.get(), Value::UNDEFINED, &[Value::bool(dark)])
                .err()
                .map(|error| vm.error_text(error))
        });
        if let Some(message) = failed {
            report(message);
        }
        pump(updates);
    });
    cx.on_resize(|updates, width, height| {
        let resized = with_internals(|internals| internals.window_resized.clone());
        let failed = with_vm_letting_go(|vm| {
            vm.call_function(
                resized.get(),
                Value::UNDEFINED,
                &[
                    Value::number(f64::from(width)),
                    Value::number(f64::from(height)),
                ],
            )
            .err()
            .map(|error| vm.error_text(error))
        });
        if let Some(message) = failed {
            report(message);
        }
        pump(updates);
    });
    cx.on_laid_out(|updates| {
        // Only a page that's watching sizes, with `ResizeObserver`, wants this.
        if !HOST.with(|host| host.laid_out_wanted.get()) {
            return;
        }
        let failed = with_vm_letting_go(|vm| {
            let told = vm
                .get_global("__rsuiLaidOut")
                .ok()
                .filter(|told| vm.is_function(*told))?;
            vm.call_function(told, Value::UNDEFINED, &[])
                .err()
                .map(|error| vm.error_text(error))
        });
        if let Some(message) = failed {
            report(message);
        }
        pump(updates);
    });
    cx.on_document_event(|updates, kind| {
        let came_with = updates.event();
        // On the way down, and between the handlers on the way down and those on the way up (phase 2), it's being dispatched,
        // and on the way back up (or after everything, when it was stopped) it's over: see `dispatching`.
        let phase = came_with.map_or(0, |event| event.phase);
        let heard = HOST.with(|host| {
            host.dispatching.set(phase == 1 || phase == 2);
            !exact_document_phases()
                || document_phase_heard(host.document_interests.borrow().as_ref(), kind, phase)
        });
        if !heard {
            // Keep the same microtask checkpoints and dispatch/timer boundaries,
            // including flags a previous phase's microtasks left for this one,
            // but do not allocate a web Event merely to find no listeners.
            pass_on_what_was_said(updates);
            pump(updates);
            return;
        }
        let told = with_internals(|internals| internals.document_event.clone());
        // What heard it before, on the way down or on what it went through, may have said not to do what it would by itself.
        let prevented = if updates.default_prevented() {
            Value::TRUE
        } else {
            Value::FALSE
        };
        let failed = with_vm_letting_go(|vm| {
            let kind = vm.new_string(kind);
            let event = came_with.map_or(Value::UNDEFINED, |event| event_value(vm, event));
            vm.call_function(told.get(), Value::UNDEFINED, &[kind, event, prevented])
                .err()
                .map(|error| vm.error_text(error))
        });
        if let Some(message) = failed {
            report(message);
        }
        pass_on_what_was_said(updates);
        pump(updates);
    });
    cx.on_effects_done(commit_effects);
    // The runtime has said so already, and stopped them. Only that part of the app has stopped, which shouldn't be covered up
    // along with the rest of it, as a page's console says what went wrong and leaves the page as it is.
    cx.on_runaway(apply_changes);
}

/// Runs the effects that a round of rendering left, all together and in the order React runs them, before what changed is drawn.
/// What it's given is only seen to when something's mounted or has effects, as what it does then can leave work for JavaScript.
fn commit_effects(updates: &mut Updates<'_>) {
    let jobs = HOST.with(|host| host.jobs_waiting.replace(false));
    if !HOST.with(|host| host.commit_waiting.replace(false)) {
        if jobs {
            pump(updates);
        }
        return;
    }
    let commit = with_internals(|internals| internals.commit_effects.clone());
    let failed = with_vm_letting_go(|vm| {
        vm.call_function(commit.get(), Value::UNDEFINED, &[])
            .err()
            .map(|error| vm.error_text(error))
    });
    if let Some(message) = failed {
        report(message);
    }
    pump(updates);
}

/// How to reach this runtime, until it's gone.
fn remember_remote(cx: &mut Cx<'_>) {
    cx.effect_with_cleanup((), |updates| {
        let id = updates.runtime_id();
        let remote = updates.remote();
        HOST.with(|host| host.remotes.borrow_mut().push((id, remote)));
        move |_: &mut Updates<'_>| {
            HOST.with(|host| host.remotes.borrow_mut().retain(|(known, _)| *known != id));
        }
    });
}

/// What a window other than the main one is given.
struct WindowEntry {
    /// What's kept for it in the layer, which has what it shows.
    entry: Root,
}

/// The root of any window other than the main one, which shows what the `Window` it came from says to, whenever that says so.
fn window_root(cx: &mut Cx<'_>, window: &WindowEntry) -> Node {
    cx.trace_name("the window");
    set_up_runtime(cx);
    remember_remote(cx);
    cx.effect((), pump);
    let function = with_internals(|internals| internals.window_root.clone());
    let kind = with_vm(|vm| {
        let name = vm.atom("name");
        kind_of(vm, function.get(), name)
    });
    cx.child_of_kind(
        js_component,
        JsProps {
            function,
            props: window.entry.clone(),
            kind,
            keyed: false,
            memo: false,
        },
        same_kind,
    )
}

/// The root of an app: whatever the main file exports as its default, and any error on top of it.
pub fn root(cx: &mut Cx<'_>, _: &Entry) -> Node {
    cx.trace_name("the app");
    set_up_runtime(cx);
    HOST.with(|host| host.main_runtime.set(cx.runtime_id()));
    let generation = cx.state(|| 0u32);
    HOST.with(|host| host.root_state.set(Some(generation)));
    remember_remote(cx);
    cx.effect((), |updates| {
        HOST.with(|host| {
            *host.remote.lock().expect("lock is not poisoned") = Some(updates.remote());
        });
        pump(updates);
    });
    let version = cx.get(generation);
    cx.effect(version, |_| {
        // Whatever renders from here on is the new code, and has to line up.
        let set_reloading = with_internals(|internals| internals.set_reloading.clone());
        with_vm_letting_go(|vm| {
            let _ = vm.call_function(set_reloading.get(), Value::UNDEFINED, &[Value::FALSE]);
        });
        HOST.with(|host| host.swapping.set(false));
    });
    let app = HOST
        .with(|host| host.app.borrow().clone())
        .expect("the app has been loaded");
    let (kind, props) = with_vm_letting_go(|vm| {
        let name = vm.atom("name");
        let kind = kind_of(vm, app.get(), name);
        let props = vm.new_object();
        (kind, vm.root(props))
    });
    let node = cx.child_of_kind(
        js_component,
        JsProps {
            function: app,
            props,
            kind,
            keyed: false,
            memo: false,
        },
        same_kind,
    );
    let error = HOST.with(|host| host.error.borrow().clone());
    let Some(error) = error else {
        return node;
    };
    // It can be put away, with Escape, by clicking away or with its button, and the app's there behind it as it was.
    let dismiss = |updates: &mut Updates<'_>, _: Option<&rsui::core::PropValue>| {
        HOST.with(|host| *host.error.borrow_mut() = None);
        if let Some(generation) = HOST.with(|host| host.root_state.get()) {
            updates.update(generation, |_| {});
        }
    };
    let button = Element::new(Role::Button)
        .key("error-dismiss")
        .name("Dismiss")
        .props(Props::from(rsui::layout::Style::new().padding(6.0)));
    let button = cx.on(button, ActionKind::Click, dismiss);
    let notice = Element::new(Role::Dialog)
        .key("error")
        .name("Something went wrong")
        .props(
            Props::from(
                rsui::layout::Style::new()
                    .overlay(Placement::Fill)
                    .align(Align::Start)
                    .justify(Justify::Start)
                    .padding(24.0),
            )
            .merged(&Props::from(
                Paint::new().background(Color::rgba(255, 255, 255, 0xF0)),
            )),
        )
        .child(error_element(&error))
        .child(button);
    let notice = cx.on(notice, ActionKind::Dismiss, dismiss);
    Element::new(Role::Window)
        .child(
            Element::new(Role::Group)
                .props(Props::from(rsui::layout::Style::new().flex(1.0)))
                .child(node),
        )
        .child(notice)
        .into_node()
}

#[cfg(test)]
mod tests {
    use super::app_name;
    use std::path::Path;

    #[test]
    fn exact_document_interests_distinguish_event_types_and_phases_with_legacy_fallback() {
        use super::document_phase_heard;
        let interests = std::collections::HashMap::from([
            ("pointermove".to_owned(), (1 << 1) | (1 << 3)),
            ("mousemove".to_owned(), 1 << 2),
        ]);
        assert!(document_phase_heard(Some(&interests), "pointermove", 1));
        assert!(!document_phase_heard(Some(&interests), "pointermove", 2));
        assert!(document_phase_heard(Some(&interests), "pointermove", 3));
        assert!(!document_phase_heard(Some(&interests), "mousemove", 1));
        assert!(document_phase_heard(Some(&interests), "mousemove", 2));
        assert!(!document_phase_heard(Some(&interests), "wheel", 3));
        assert!(document_phase_heard(Some(&interests), "keyboardleft", 0));
        assert!(document_phase_heard(None, "wheel", 3));
    }

    #[test]
    fn superseded_timer_callbacks_cannot_clear_or_rearm_the_current_deadline() {
        use super::{consume_timer_deadline, reserve_timer_deadline};
        use std::cell::Cell;
        use std::time::{Duration, Instant};

        // All ordering is an explicit clock; this regression needs no sleeps.
        let now = Instant::now();
        let delayed = now + Duration::from_millis(20);
        let immediate = now + Duration::from_millis(1);
        let following = now + Duration::from_millis(40);
        let timer = Cell::new(None);
        assert!(reserve_timer_deadline(&timer, delayed));
        assert!(!reserve_timer_deadline(&timer, delayed));
        assert!(!reserve_timer_deadline(&timer, following));
        assert!(reserve_timer_deadline(&timer, immediate));
        assert!(consume_timer_deadline(&timer, immediate));

        // The immediate turn arms another JS timer before the original runtime
        // callback wakes. That old callback must leave the new timer intact.
        assert!(reserve_timer_deadline(&timer, following));
        assert!(!consume_timer_deadline(&timer, delayed));
        assert_eq!(timer.get(), Some(following));
        assert!(!consume_timer_deadline(&timer, immediate));
        assert_eq!(timer.get(), Some(following));
        assert!(consume_timer_deadline(&timer, following));
        assert_eq!(timer.get(), None);
        assert!(!consume_timer_deadline(&timer, following));
    }

    #[test]
    fn native_tag_hashes_keep_the_existing_placement_identity() {
        use std::hash::Hasher;
        for tag in [
            "window",
            "group",
            "row",
            "column",
            "text",
            "span",
            "button",
            "link",
            "input",
            "checkbox",
            "item",
            "image",
            "dialog",
            "menu",
            "menuitem",
            "",
            "custom-λ",
            "scroll",
            "list",
        ] {
            let mut old = super::Fnv::default();
            old.write(tag.as_bytes());
            assert_eq!(super::tag_kind(tag), old.finish(), "{tag}");
            if let Some(native) = super::native_tag(tag) {
                assert_eq!(native.kind, old.finish(), "{tag}");
                assert_eq!(native.page, super::could_be_page_element(tag), "{tag}");
            }
        }
    }

    #[test]
    fn two_packed_programs_in_one_folder_have_different_names() {
        let main = Path::new("C:/Downloads/app.tsx");
        let first = app_name(Some(Path::new("C:/Downloads/first.exe")), main);
        let second = app_name(Some(Path::new("C:/Downloads/second.exe")), main);
        assert_eq!(first, "first");
        assert_eq!(second, "second");
    }

    #[test]
    fn a_packed_program_keeps_its_name_when_it_moves() {
        let here = app_name(
            Some(Path::new("C:/here/notes.exe")),
            Path::new("C:/here/app.tsx"),
        );
        let there = app_name(
            Some(Path::new("D:/there/notes.exe")),
            Path::new("D:/there/app.tsx"),
        );
        assert_eq!(here, there);
    }

    #[test]
    fn a_program_that_is_not_packed_goes_by_its_folder_and_file() {
        assert_eq!(
            app_name(None, Path::new("C:/code/notes/app.tsx")),
            "notes-app"
        );
        assert_eq!(app_name(None, Path::new("app.tsx")), "app");
    }

    #[test]
    fn versions_are_compared_by_each_number() {
        use super::is_newer;
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("0.10", "0.9.3"));
        assert!(is_newer("1.0.0", "0.99.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1", "0.1.0"));
    }

    #[test]
    fn an_app_made_for_a_newer_rsui_tsx_says_so_when_it_cannot_start() {
        let folder = std::env::temp_dir().join(format!("rsui-newer-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("package.json"),
            r#"{ "engines": { "rsui-tsx": ">=99.1.0" } }"#,
        )
        .unwrap();
        let main = folder.join("app.tsx");
        let said = super::newer_needed(&main, "Cannot find module 'rsui/charts'".to_owned());
        assert!(
            said.starts_with("this app needs a newer rsui-tsx (99.1.0), you have ("),
            "{said}"
        );
        assert!(said.contains("it imports rsui/charts"), "{said}");
        // What's there isn't what's missing, and a package that isn't rsui's has nothing to do with it.
        std::fs::remove_file(folder.join("package.json")).unwrap();
        let said = super::newer_needed(&main, "Cannot find module 'left-pad'".to_owned());
        assert_eq!(said, "Cannot find module 'left-pad'");
        let said = super::newer_needed(&main, "Cannot find module 'rsui/charts'".to_owned());
        assert!(
            said.starts_with("this app needs a newer rsui-tsx, you have ("),
            "{said}"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }
}

#[cfg(test)]
mod bundled {
    /// The bundled modules' functions are compiled when they're first called, and aren't checked for mistakes before
    /// then, so here's where they're all checked.
    #[test]
    fn every_bundled_module_compiles() {
        for (name, source) in super::HOST_MODULES {
            if let Err(error) = rsui_js::check_compiles(source, &rsui_js::MODULE_PARAMS) {
                panic!("{name}: {error}");
            }
        }
    }
}

#[cfg(test)]
mod native_prop_reuse_tests {
    use super::*;

    /// Nothing big is kept: no set of many props, and none with much text.
    fn nothing_big_kept() -> bool {
        HOST.with(|host| {
            host.recent_props.borrow().iter().all(|one| {
                one.len() <= RECENT_PROPS_LONGEST
                    && one.iter().all(|(_, value)| match value {
                        rsui::core::PropValue::Str(text) => text.len() <= RECENT_PROPS_TEXT,
                        _ => true,
                    })
            })
        })
    }

    #[test]
    fn reuse_is_bounded_and_does_not_retain_large_payloads() {
        forget_what_ran();
        let make = || vec![(Symbol::new("height"), rsui::core::PropValue::Float(8.0))];
        let a = native_props(make());
        let b = native_props(make());
        assert_eq!(a.identity(), b.identity());
        let large: Vec<_> = (0..100)
            .map(|i| (Symbol::new(&format!("p{i}")), rsui::core::PropValue::Int(i)))
            .collect();
        native_props(large);
        assert!(nothing_big_kept());
        HOST.with(|host| assert!(host.native_entries.borrow().capacity() <= 32));
        native_props(vec![(Symbol::new("image"), "x".repeat(100_000).into())]);
        assert!(nothing_big_kept());
        native_props(make());
        forget_what_ran();
        HOST.with(|host| {
            assert_eq!(host.recent_props.borrow().len(), 0);
            assert_eq!(host.native_entries.borrow().capacity(), 0);
        });
        assert_eq!(a.get("height"), Some(&rsui::core::PropValue::Float(8.0)));
    }

    #[test]
    fn sets_made_by_turns_are_shared_with_their_text() {
        forget_what_ran();
        // A paragraph's props and its span's, one after the other, as a document of them has them.
        let paragraph = || {
            vec![
                (
                    Symbol::new("line_height"),
                    rsui::core::PropValue::Float(20.0),
                ),
                (Symbol::new("min_auto"), rsui::core::PropValue::Bool(true)),
            ]
        };
        let span = || {
            vec![
                (Symbol::new("data-lexical-text"), "true".into()),
                (Symbol::new("fit_width"), rsui::core::PropValue::Bool(true)),
            ]
        };
        let first = (native_props(paragraph()), native_props(span()));
        let second = (native_props(paragraph()), native_props(span()));
        assert_eq!(first.0.identity(), second.0.identity());
        assert_eq!(first.1.identity(), second.1.identity());
        // The same props in another order are the same set.
        let mut turned = span();
        turned.reverse();
        assert_eq!(native_props(turned), first.1);
        // A different value is a set of its own.
        let other = native_props(vec![(
            Symbol::new("line_height"),
            rsui::core::PropValue::Float(21.0),
        )]);
        assert_ne!(other, first.0);
        forget_what_ran();
    }
}
