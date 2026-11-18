//! Where a page's elements are, for what `rsui/web` is asked about them while it runs: `getBoundingClientRect()` and what an
//! element is in, `document.elementFromPoint()`, and `querySelector()` and `getElementById()`. Each answers from the tree as
//! it was last laid out, and with where everything was then, which `rsui::geometry` keeps, as a browser answers from the
//! page as it was last laid out, unless something's changed since. An element is said as what's along the way to it from the
//! root, each as an event says it (see `chain_value` in `bridge.rs`), which is what `web.js` makes elements from. What's
//! looked for is found in the tree as it is now, which an effect that's just been rendered may be looking through before it's
//! been laid out, as a browser would lay it out then: what's in it that's new is somewhere, and that's nowhere yet.

use std::cell::RefCell;
use std::collections::HashMap;

use rsui::core::{Element, Key, Node, PropValue, Symbol, symbol};
use rsui::layout::Layout;
use rsui_js::{Args, HostFunction, Value, Vm};

use crate::bridge::node_value;

pub(crate) const FUNCTIONS: [(&str, HostFunction); 24] = [
    ("treeNow", host_tree_now),
    ("stillNow", host_still_now),
    ("chainsNow", host_chains_now),
    ("hasKey", host_has_key),
    ("textCaret", host_text_caret),
    ("textIndexAt", host_text_index_at),
    ("scrollOf", host_scroll_of),
    ("scrollOffsetsOf", host_scroll_offsets_of),
    ("contentSizeOf", host_content_size_of),
    ("keepLaidOut", host_keep_laid_out),
    ("chainOf", host_chain_of),
    ("sizeOf", host_size_of),
    ("chainAt", host_chain_at),
    ("chainsOf", host_chains_of),
    ("pageTree", host_page_tree),
    ("pageTreeKept", host_page_tree_kept),
    ("pageTreeContentKept", host_page_tree_content_kept),
    ("pageTreeContentVersion", host_page_tree_content_version),
    ("captureSelectorInputs", crate::selector_receipt::capture),
    ("selectorInputsKept", crate::selector_receipt::kept),
    ("selectorReceiptSafeIntrinsics", crate::selector_receipt::safe_intrinsics),
    ("selectorReceiptSafePrograms", crate::selector_receipt::safe_programs),
    ("pageChainOf", host_page_chain_of),
    ("nearView", host_near_view),
];

/// `nearView(margin)`: the keys of the page's elements whose contents are being skipped (`content_skipped`, which `content-visibility:
/// auto` gives them in `web.js`) that are within `margin` of being seen, in the window or what scrolls around them, as the window last
/// laid them out. What's in a long list that scrolls is only looked at as far as its rows: see `Layout::near_view`.
fn host_near_view(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let margin = args.get(vm, 0).as_number();
    let margin = if margin.is_finite() { margin.max(0.0) as f32 } else { 0.0 };
    let skipped = symbol!("content_skipped");
    let keys: Vec<String> = rsui::geometry::ask(|root, layout| {
        let mut keys = Vec::new();
        layout.near_view(
            root,
            margin,
            &mut |node| node.props.get_symbol(skipped) == Some(&PropValue::Bool(true)),
            &mut |_, node| {
                if let Some(key) = dom_key(node) {
                    keys.push(key.to_owned());
                }
            },
        );
        Some(keys)
    })
    .unwrap_or_default();
    let items = keys.iter().map(|key| vm.new_string(key)).collect();
    Ok(vm.new_array(items))
}

/// `treeNow()`: a number for the tree as it is now, which is another once anything in it has changed, for what keeps something it
/// worked out from the tree for as long as it's the same.
fn host_tree_now(_: &mut Vm, _: Args) -> Result<Value, Value> {
    Ok(tree_now().map_or(Value::UNDEFINED, |tree| {
        Value::number(std::ptr::from_ref::<Element>(&*tree).addr() as f64)
    }))
}

thread_local! {
    /// The tree and the geometry's generation `chainsNow` last saw, and the number it gave for them. The tree's kept, so that
    /// another can't be made in the same place and be taken for it.
    static CHAINS: RefCell<(Option<Node>, u64, f64)> = const { RefCell::new((None, 0, 0.0)) };
}

/// `chainsNow()`: a number that's the same for as long as `chainOf` would say the same about every key, which is for as long as
/// the tree and where everything is in it are, so that an element can keep where it is until then. A page asks where each
/// element above one is, again and again, and floating-ui asks that for each popover on every scroll.
fn host_chains_now(_: &mut Vm, _: Args) -> Result<Value, Value> {
    let tree = tree_now();
    let generation = rsui::geometry::generation();
    let number = CHAINS.with(|chains| {
        let mut chains = chains.borrow_mut();
        let same_tree = match (&chains.0, &tree) {
            (Some(kept), Some(tree)) => same(kept, tree),
            (None, None) => true,
            _ => false,
        };
        if !same_tree || chains.1 != generation {
            *chains = (tree, generation, chains.2 + 1.0);
        }
        chains.2
    });
    Ok(Value::number(number))
}

/// What's along the way from the root to `path`, with where each was, if `layout` is of this tree.
fn chain_to(vm: &mut Vm, root: &Node, layout: Option<&Layout>, path: &[u32]) -> Value {
    let mut items = Vec::with_capacity(path.len() + 1);
    let mut node: &Element = root;
    let mut rectangles = layout.map(|layout| layout.rects_along(path));
    for depth in 0..=path.len() {
        if depth > 0 {
            match node.children.get(path[depth - 1] as usize) {
                Some(child) => node = child,
                None => break,
            }
        }
        let rect = rectangles.as_mut()
            .and_then(Iterator::next)
            .map_or([0.0; 4], |rect| [rect.x, rect.y, rect.width, rect.height]);
        items.push(node_value(vm, node, rect));
    }
    vm.new_array(items)
}

/// The key an element of a page has for its handle, if it has one: see `useDomRef` in `web.js`.
fn dom_key(node: &Element) -> Option<&str> {
    match (node.props.get_symbol(symbol!("dom_key")), &node.key) {
        (Some(PropValue::Str(key)), _) => Some(key.as_str()),
        (None, Some(Key::Str(key))) if key.starts_with("__dom") => Some(key.as_str()),
        _ => None,
    }
}

/// Whether these are the same tree.
fn same(one: &Node, other: &Node) -> bool {
    std::ptr::eq::<Element>(&**one, &**other)
}

/// What's along the way to `path` in `tree`, with where each was if that tree's been laid out.
fn chain_in(vm: &mut Vm, tree: &Node, path: &[u32]) -> Value {
    let laid_out = rsui::geometry::ask(|root, layout| {
        same(root, tree).then(|| chain_to(vm, root, Some(layout), path))
    });
    match laid_out {
        Some(chain) => chain,
        None => chain_to(vm, tree, None, path),
    }
}

/// The tree as it is now, or as it was last laid out, if nothing's said, with what code has stopped moving since (see `STILL`).
fn tree_now() -> Option<Node> {
    let tree = rsui::runtime::current_tree()
        .or_else(|| rsui::geometry::ask(|root, _| Some(root.clone())))?;
    Some(STILL.with(|still| still.borrow_mut().over(tree)))
}

/// An element whose animation code has stopped, by its key: the props the animation moves, and what they are without it.
type Stilled = (String, Vec<Symbol>, Vec<(Symbol, PropValue)>);

/// Elements whose `style` code has given `animation-name: none` since the tree was last made, as Radix Collapsible does for a
/// moment to measure its content at its full height. A page works out its style again before it says where anything is, so
/// that's measured with no animation, where the tree here still has what the animation moved until the element's drawn again.
/// Only for the tree they were said about: in the next one the element is drawn as its style says.
#[derive(Default)]
struct Still {
    tree: Option<Node>,
    stilled: Vec<Stilled>,
    made: Option<Node>,
}

thread_local! {
    static STILL: RefCell<Still> = RefCell::new(Still::default());
}

impl Still {
    /// `tree` with what's stilled in it as it is with no animation.
    fn over(&mut self, tree: Node) -> Node {
        if self.stilled.is_empty() {
            return tree;
        }
        if !self.tree.as_ref().is_some_and(|kept| same(kept, &tree)) {
            *self = Self::default();
            return tree;
        }
        if let Some(made) = &self.made {
            return made.clone();
        }
        let mut made = tree.clone();
        for (key, moved, base) in &self.stilled {
            let Some(path) = path_of_key(&tree, key) else {
                continue;
            };
            let mut slot = &mut made;
            for &index in &path {
                slot = &mut Node::make_mut(slot).children[index as usize];
            }
            let element = Node::make_mut(slot);
            element.props.remove("animation");
            for name in moved {
                element.props.remove(name.as_str());
            }
            for (name, value) in base {
                element.props.insert(*name, value.clone());
            }
        }
        self.made = Some(made.clone());
        made
    }
}

/// `stillNow(key, moved, base)`: the element with this key is to be asked about as if its animation had stopped, with the props
/// named in `moved` (as its `animation` names them) as `base` has them, or not there. `stillNow(key)` takes that back.
fn host_still_now(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let key = args.get(vm, 0);
    let Some(key) = vm.plain_text(key) else {
        return Ok(Value::UNDEFINED);
    };
    let moved = args.get(vm, 1);
    let stilled = if moved.is_object() {
        let mut names = Vec::new();
        for (_, name) in vm.properties(moved)? {
            if let Some(name) = vm.plain_text(name) {
                names.push(Symbol::new(&name));
            }
        }
        let base = args.get(vm, 2);
        let base = if base.is_object() {
            crate::bridge::gather_props(vm, base)?.entries
        } else {
            Vec::new()
        };
        Some((key.clone(), names, base))
    } else {
        None
    };
    let tree =
        rsui::runtime::current_tree().or_else(|| rsui::geometry::ask(|root, _| Some(root.clone())));
    STILL.with(|still| {
        let mut still = still.borrow_mut();
        let same_tree =
            matches!((&still.tree, &tree), (Some(kept), Some(tree)) if same(kept, tree));
        if !same_tree {
            *still = Still {
                tree,
                ..Still::default()
            };
        }
        still.made = None;
        still.stilled.retain(|(was, _, _)| *was != key);
        if let Some(stilled) = stilled {
            still.stilled.push(stilled);
        }
    });
    Ok(Value::UNDEFINED)
}

/// A tree, and where each element with a key is in it.
type Keyed = (Node, HashMap<String, Vec<u32>>);

thread_local! {
    /// Where each element with a key is in the trees last asked about, found all at once the first time one's asked for. Each
    /// tree's kept with them, so that it can't be another one in the same place.
    static KEYED: RefCell<Vec<Keyed>> = const { RefCell::new(Vec::new()) };
}

fn find_keyed(node: &Element, path: &mut Vec<u32>, found: &mut HashMap<String, Vec<u32>>) {
    if let Some(key) = dom_key(node) {
        found.insert(key.to_owned(), path.clone());
    }
    for (index, child) in node.children.iter().enumerate() {
        path.push(index as u32);
        find_keyed(child, path, found);
        path.pop();
    }
}

fn path_of_key(root: &Node, key: &str) -> Option<Vec<u32>> {
    KEYED.with(|keyed| {
        let mut keyed = keyed.borrow_mut();
        // Where it was in the last tree looked through, if it's still there in this one: most of a tree stays where it was from
        // one to the next, as typing changes one paragraph, so a document of thousands needn't be looked through after each key.
        if !keyed.iter().any(|(tree, _)| same(tree, root))
            && let Some(path) = keyed.last().and_then(|(_, found)| found.get(key))
            && node_at(root, path).and_then(dom_key) == Some(key)
        {
            return Some(path.clone());
        }
        // The one laid out and the one that's now are asked about by turns, so both are kept.
        let at = match keyed.iter().position(|(tree, _)| same(tree, root)) {
            Some(at) => at,
            None => {
                // What's changed since a tree that's kept, which is what a frame changed, rather than all of it again: a page
                // asks about something that isn't shown at every frame, as an opened Popover's dropdown is, and finding all
                // of a kitchen sink's 2,500 elements again for that was 0.4 ms of each frame of a drag.
                let (tree, found) = match keyed.len() {
                    0 => {
                        let mut found = HashMap::new();
                        find_keyed(root, &mut Vec::new(), &mut found);
                        (root.clone(), found)
                    }
                    1 => {
                        let (tree, found) = &keyed[0];
                        let mut found = found.clone();
                        keyed_changes(tree, root, &mut Vec::new(), &mut found);
                        (root.clone(), found)
                    }
                    _ => {
                        let (tree, mut found) = keyed.remove(0);
                        keyed_changes(&tree, root, &mut Vec::new(), &mut found);
                        (root.clone(), found)
                    }
                };
                keyed.push((tree, found));
                keyed.len() - 1
            }
        };
        keyed[at].1.get(key).cloned()
    })
}

/// Brings where each element with a key is, `found` for `old`, up to date for `new`, at the same `path` in both, looking only at
/// what isn't the same in the two: a tree that's changed shares what didn't change with the one before it.
fn keyed_changes(
    old: &Element,
    new: &Element,
    path: &mut Vec<u32>,
    found: &mut HashMap<String, Vec<u32>>,
) {
    if std::ptr::eq(old, new) {
        return;
    }
    let (was, now) = (dom_key(old), dom_key(new));
    if was != now {
        if let Some(was) = was {
            forget_key(was, path, found);
        }
        if let Some(now) = now {
            found.insert(now.to_owned(), path.clone());
        }
    }
    for index in 0..old.children.len().max(new.children.len()) {
        path.push(index as u32);
        match (old.children.get(index), new.children.get(index)) {
            (Some(old), Some(new)) => keyed_changes(old, new, path, found),
            (Some(old), None) => forget_keyed(old, path, found),
            (None, Some(new)) => find_keyed(new, path, found),
            (None, None) => {}
        }
        path.pop();
    }
}

/// Forgets a key that was at `path`, unless it's been found somewhere else already, having moved there.
fn forget_key(key: &str, path: &[u32], found: &mut HashMap<String, Vec<u32>>) {
    if found.get(key).is_some_and(|at| at.as_slice() == path) {
        found.remove(key);
    }
}

/// Forgets each key in what's gone from `path`.
fn forget_keyed(node: &Element, path: &mut Vec<u32>, found: &mut HashMap<String, Vec<u32>>) {
    if let Some(key) = dom_key(node) {
        forget_key(key, path, found);
    }
    for (index, child) in node.children.iter().enumerate() {
        path.push(index as u32);
        forget_keyed(child, path, found);
        path.pop();
    }
}

/// `keepLaidOut()`: says that something holds an element it may ask about, which keeps where everything was while the tree
/// changes, for an effect to ask: see `rsui::geometry`.
fn host_keep_laid_out(_: &mut Vm, _: Args) -> Result<Value, Value> {
    rsui::geometry::keep();
    Ok(Value::UNDEFINED)
}

/// `hasKey(key)`: whether the element with this key is in the tree as it is now, without saying where, which would lay it out.
fn host_has_key(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let key = args.get(vm, 0);
    let Some(key) = vm.plain_text(key) else {
        return Ok(Value::FALSE);
    };
    Ok(Value::bool(
        tree_now().is_some_and(|tree| path_of_key(&tree, &key).is_some()),
    ))
}

/// `chainOf(key)`: what's along the way to the element with this key, or undefined if it isn't shown.
fn host_chain_of(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let key = args.get(vm, 0);
    let Some(key) = vm.plain_text(key) else {
        return Ok(Value::UNDEFINED);
    };
    // What's in the tree as it is now is where its window put it, if that's the tree it laid out, and otherwise where laying out
    // the tree as it is now puts it, as a browser lays out a page that's changed since when it's asked where something is.
    if let Some(tree) = tree_now()
        && let Some(path) = path_of_key(&tree, &key)
    {
        let shown = rsui::geometry::ask(|root, layout| {
            same(root, &tree).then(|| chain_to(vm, root, Some(layout), &path))
        });
        if let Some(chain) = shown {
            return Ok(chain);
        }
        return Ok(rsui::geometry::ask_now(&tree, |root, layout| {
            Some(chain_to(vm, root, Some(layout), &path))
        })
        .unwrap_or_else(|| chain_to(vm, &tree, None, &path)));
    }
    // Something in another window's tree, as it was last laid out.
    Ok(rsui::geometry::ask(|root, layout| {
        let path = path_of_key(root, &key)?;
        Some(chain_to(vm, root, Some(layout), &path))
    })
    .unwrap_or(Value::UNDEFINED))
}

/// `sizeOf(key)`: how wide and tall the element with this key is, as `[width, height]`, where `chainOf` would say, without
/// what's along the way to it: what a page's `ResizeObserver`s watch is looked at after every layout, and making each one's
/// whole chain for that was most of what that cost. Nothing if it isn't there.
fn host_size_of(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let key = args.get(vm, 0);
    let Some(key) = vm.plain_text(key) else {
        return Ok(Value::UNDEFINED);
    };
    let size =
        |layout: &Layout, path: &[u32]| layout.rect(path).map(|rect| [rect.width, rect.height]);
    let found = if let Some(tree) = tree_now()
        && let Some(path) = path_of_key(&tree, &key)
    {
        rsui::geometry::ask(|root, layout| same(root, &tree).then(|| size(layout, &path)))
            .or_else(|| rsui::geometry::ask_now(&tree, |_, layout| Some(size(layout, &path))))
            .flatten()
    } else {
        rsui::geometry::ask(|root, layout| {
            let path = path_of_key(root, &key)?;
            Some(size(layout, &path))
        })
        .flatten()
    };
    Ok(match found {
        Some([width, height]) => {
            let items = vec![
                Value::number(f64::from(width)),
                Value::number(f64::from(height)),
            ];
            vm.new_array(items)
        }
        None => Value::UNDEFINED,
    })
}

/// The paragraphs in an element, in order, each with its path and its text, as a page's block that's text, or what's broken
/// into lines, has them.
fn paragraphs_in(node: &Element, path: &mut Vec<u32>, out: &mut Vec<(Vec<u32>, String)>) {
    if node.role == rsui::core::Role::Text {
        out.push((path.clone(), rsui::text::TextEngine::element_text(node)));
        return;
    }
    for (index, child) in node.children.iter().enumerate() {
        path.push(index as u32);
        paragraphs_in(child, path, out);
        path.pop();
    }
}

/// The byte of `text` that is its `units`th UTF-16 unit, as JavaScript counts, or its end.
fn byte_of_units(text: &str, units: usize) -> usize {
    let mut seen = 0;
    for (byte, character) in text.char_indices() {
        if seen >= units {
            return byte;
        }
        seen += character.len_utf16();
    }
    text.len()
}

fn units_of_byte(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].encode_utf16().count()
}

/// The element with a key in the tree as it is now, laid out, with something to measure its text with.
fn with_element<R>(
    key: &str,
    what: impl FnOnce(&Element, &[u32], &Layout, &mut rsui::text::TextEngine) -> Option<R>,
) -> Option<R> {
    let tree = tree_now()?;
    let path = path_of_key(&tree, key)?;
    rsui::geometry::measure(&tree, |root, layout, text| {
        let node = node_at(root, &path)?;
        what(node, &path, layout, text)
    })
}

/// `textCaret(key, at)`: where a caret just before the `at`th UTF-16 unit of the text in the element with this key goes, as
/// `[x, y, height]` from the window's top left: in the paragraph that has that place, after what's before it in the others.
fn host_text_caret(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let key = args.get(vm, 0);
    let Some(key) = vm.plain_text(key) else {
        return Ok(Value::UNDEFINED);
    };
    let at = args.get(vm, 1).as_number().max(0.0) as usize;
    let found = with_element(&key, |node, path, layout, text| {
        let mut found = Vec::new();
        paragraphs_in(node, &mut Vec::new(), &mut found);
        let mut left = at;
        for (index, (inner, said)) in found.iter().enumerate() {
            let units = said.encode_utf16().count();
            if left <= units || index == found.len() - 1 {
                let mut full = path.to_vec();
                full.extend_from_slice(inner);
                let paragraph = node_at(node, inner)?;
                let rect = layout.rect(&full)?;
                let content = rsui::render::content_box(paragraph, rect, 1.0);
                let (x, y, height) = text.element_caret(
                    paragraph,
                    content.width,
                    byte_of_units(said, left.min(units)),
                )?;
                return Some([content.left + x, content.top + y, height]);
            }
            left -= units;
        }
        // Nothing in it that's text: its first line.
        let rect = layout.rect(path)?;
        let content = rsui::render::content_box(node, rect, 1.0);
        let style = rsui::text::TextStyle::from_element(node);
        Some([content.left, content.top, text.line_height(style)])
    });
    Ok(match found {
        Some(found) => {
            let items = found
                .iter()
                .map(|value| Value::number(f64::from(*value)))
                .collect();
            vm.new_array(items)
        }
        None => Value::UNDEFINED,
    })
}

/// `textIndexAt(key, x, y)`: which UTF-16 unit of the text in the element with this key a place in the window is nearest the
/// start of, in the paragraph that's nearest it down the window.
fn host_text_index_at(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let key = args.get(vm, 0);
    let Some(key) = vm.plain_text(key) else {
        return Ok(Value::UNDEFINED);
    };
    let (x, y) = (
        args.get(vm, 1).as_number() as f32,
        args.get(vm, 2).as_number() as f32,
    );
    let found = with_element(&key, |node, path, layout, text| {
        let mut found = Vec::new();
        paragraphs_in(node, &mut Vec::new(), &mut found);
        let mut before = 0;
        // The nearest down the window, and of what's on the same line, the nearest across: pieces of text side by side on a line,
        // as a bold word and what's after it, needn't be as tall as each other.
        const SAME_LINE: f32 = 4.0;
        let mut best: Option<(f32, f32, usize)> = None;
        for (inner, said) in &found {
            let mut full = path.to_vec();
            full.extend_from_slice(inner);
            let paragraph = node_at(node, inner)?;
            let rect = layout.rect(&full)?;
            let content = rsui::render::content_box(paragraph, rect, 1.0);
            let away = if y < content.top {
                content.top - y
            } else if y > content.top + content.height {
                y - content.top - content.height
            } else {
                0.0
            };
            let away = if away <= SAME_LINE { 0.0 } else { away };
            let across = if x < content.left {
                content.left - x
            } else if x > content.left + content.width {
                x - content.left - content.width
            } else {
                0.0
            };
            if best.is_none_or(|(down, side, _)| away < down || (away == down && across < side)) {
                let byte = text.element_byte_at(
                    paragraph,
                    content.width,
                    (
                        x - content.left,
                        (y - content.top).clamp(0.0, (content.height - 1.0).max(0.0)),
                    ),
                );
                best = Some((away, across, before + units_of_byte(said, byte)));
            }
            before += said.encode_utf16().count();
        }
        let best = best.map(|(away, _, at)| (away, at));
        Some(best.map_or(0, |(_, at)| at))
    });
    Ok(found.map_or(Value::UNDEFINED, |at| Value::number(at as f64)))
}

/// `chainAt(x, y)`: what's along the way to what's at this place in the window, from its top left, or undefined if that's
/// nothing.
fn host_chain_at(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (x, y) = (args.get(vm, 0), args.get(vm, 1));
    if !x.is_number() || !y.is_number() {
        return Ok(Value::UNDEFINED);
    }
    let (x, y) = (x.as_number() as f32, y.as_number() as f32);
    Ok(rsui::geometry::ask(|root, layout| {
        let path = rsui::geometry::pointer_path(root, layout, x, y)?;
        Some(chain_to(vm, root, Some(layout), &path))
    })
    .unwrap_or(Value::UNDEFINED))
}

/// The node at `path` in `root`.
fn node_at<'a>(root: &'a Element, path: &[u32]) -> Option<&'a Element> {
    let mut node = root;
    for index in path {
        node = node.children.get(*index as usize)?;
    }
    Some(node)
}

/// `scrollOf(key)`: how far what has this key is scrolled, across and down, how big what's in it is, across and down, and how
/// much of that shows, and then how big it is itself, as it was last laid out, or undefined if it doesn't scroll or hasn't been
/// laid out. Its own size says whether that was at the window's size now: see `pageView` in `web.js`.
fn host_scroll_of(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let key = args.get(vm, 0);
    let Some(key) = vm.plain_text(key) else {
        return Ok(Value::UNDEFINED);
    };
    let scroll = |root: &Node, layout: &Layout, path: &[u32]| {
        let node = node_at(root, path)?;
        let info = layout.scroll_info(path, node)?;
        let own = layout
            .rect(path)
            .map_or([info.view.width, info.view.height], |rect| {
                [rect.width, rect.height]
            });
        // Whether it starts at its right, as one where text goes from right to left does, whose `scrollLeft` is from there.
        let backwards = matches!(node.props.get("text_direction"), Some(rsui::core::PropValue::Str(way)) if way.as_str() == "rtl");
        Some([
            info.x,
            info.y,
            info.content.width,
            info.content.height,
            info.view.width,
            info.view.height,
            own[0],
            own[1],
            if backwards { 1.0 } else { 0.0 },
        ])
    };
    // A node found in a current layout can legitimately have no scrolling box.
    // Preserve that answer instead of measuring the whole tree again for None.
    // Query the latest tree, as chainOf does, rather than an older shown tree
    // which may still contain this key with obsolete overflow properties.
    let info = if let Some(tree) = tree_now()
        && let Some(path) = path_of_key(&tree, &key)
    {
        rsui::geometry::ask(|root, layout| same(root, &tree).then(|| scroll(root, layout, &path)))
            .or_else(|| {
                rsui::geometry::ask_now(&tree, |root, layout| Some(scroll(root, layout, &path)))
            })
            .flatten()
    } else {
        rsui::geometry::ask(|root, layout| {
            let path = path_of_key(root, &key)?;
            Some(scroll(root, layout, &path))
        })
        .flatten()
    };
    Ok(match info {
        Some(numbers) => {
            let items = numbers
                .iter()
                .map(|number| Value::number(f64::from(*number)))
                .collect();
            vm.new_array(items)
        }
        None => Value::UNDEFINED,
    })
}

/// `scrollOffsetsOf(key)`: the same current offsets as `scrollOf`, without
/// measuring descendant content dimensions that input coordinates do not read.
fn host_scroll_offsets_of(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let key = args.get(vm, 0);
    let Some(key) = vm.plain_text(key) else {
        return Ok(Value::UNDEFINED);
    };
    let scroll = |root: &Node, layout: &Layout, path: &[u32]| {
        let node = node_at(root, path)?;
        layout.scroll_offsets(path, node)
    };
    // Preserve a known non-scroller's None rather than measuring again, and use
    // the latest tree/layout with the same resolution rules as full scrollOf.
    let offsets = if let Some(tree) = tree_now()
        && let Some(path) = path_of_key(&tree, &key)
    {
        rsui::geometry::ask(|root, layout| same(root, &tree).then(|| scroll(root, layout, &path)))
            .or_else(|| rsui::geometry::ask_now(&tree, |root, layout| Some(scroll(root, layout, &path))))
            .flatten()
    } else {
        rsui::geometry::ask(|root, layout| {
            let path = path_of_key(root, &key)?;
            Some(scroll(root, layout, &path))
        })
        .flatten()
    };
    Ok(match offsets {
        Some([x, y]) => vm.new_array(vec![Value::number(f64::from(x)), Value::number(f64::from(y))]),
        None => Value::UNDEFINED,
    })
}

/// `contentSizeOf(key)`: how big what's in an element that doesn't scroll is, as `[width, height]`, as its `scrollWidth` and
/// `scrollHeight` say, which is as far as what's in it reaches when that's past its own size, as when it clips it: see
/// `Layout::content_size`. Undefined for one that scrolls, which `scrollOf` answers, or that isn't laid out.
fn host_content_size_of(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let key = args.get(vm, 0);
    let Some(key) = vm.plain_text(key) else {
        return Ok(Value::UNDEFINED);
    };
    let size = |root: &Node, layout: &Layout, path: &[u32]| {
        let node = node_at(root, path)?;
        layout.content_size(path, node)
    };
    let size = if let Some(tree) = tree_now()
        && let Some(path) = path_of_key(&tree, &key)
    {
        rsui::geometry::ask(|root, layout| same(root, &tree).then(|| size(root, layout, &path)))
            .or_else(|| {
                rsui::geometry::ask_now(&tree, |root, layout| Some(size(root, layout, &path)))
            })
            .flatten()
    } else {
        rsui::geometry::ask(|root, layout| {
            let path = path_of_key(root, &key)?;
            Some(size(root, layout, &path))
        })
        .flatten()
    };
    Ok(match size {
        Some(size) => {
            let items = vec![
                Value::number(f64::from(size.width)),
                Value::number(f64::from(size.height)),
            ];
            vm.new_array(items)
        }
        None => Value::UNDEFINED,
    })
}

thread_local! {
    /// The tree `pageTree` last gave, and whether it said where everything in it is (from its window's layout or one made for it),
    /// so that it isn't made again while it's the same.
    static GIVEN: RefCell<Option<(Node, bool)>> = const { RefCell::new(None) };
    // No extra tree: the serialization anchor above owns the only reference.
    // Saturation disables certification rather than aliasing f64 generations.
    static CONTENT_VERSION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn add_elements(
    vm: &mut Vm,
    node: &Element,
    layout: Option<&Layout>,
    path: &mut Vec<u32>,
    out: &mut Vec<Value>,
) {
    let rect = layout
        .and_then(|layout| layout.rect(path))
        .map_or([0.0; 4], |rect| [rect.x, rect.y, rect.width, rect.height]);
    out.push(node_value(vm, node, rect));
    out.push(Value::number(node.children.len() as f64));
    for (index, child) in node.children.iter().enumerate() {
        path.push(index as u32);
        add_elements(vm, child, layout, path, out);
        path.pop();
    }
}

/// `pageTree(always)`: every element in the tree as it is now, one after another as they're written, each as an event says
/// it (see `chain_value` in `bridge.rs`) and then how many it has in it, with where each was if this tree's been laid out.
/// It's undefined if that's what it gave last time, unless `always` says otherwise, which is what `querySelector()` and the
/// rest of what `web.js` matches selectors against asks for, as many of them can be asked while nothing changes.
fn host_page_tree(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let always = args.get(vm, 0) == Value::TRUE;
    let Some(tree) = tree_now() else {
        return Ok(Value::UNDEFINED);
    };
    let laid_out = laid_out(&tree);
    let unchanged = GIVEN.with(|given| {
        given
            .borrow()
            .as_ref()
            .is_some_and(|(before, was_laid_out)| {
                same(before, &tree) && (*was_laid_out || !laid_out)
            })
    });
    if unchanged && !always {
        return Ok(Value::UNDEFINED);
    }
    let split = rsui::split::enter(rsui::split::PAGE_TREE_BUILD);
    let mut out = Vec::new();
    let made = rsui::geometry::ask(|root, layout| {
        same(root, &tree).then(|| add_elements(vm, root, Some(layout), &mut Vec::new(), &mut out))
    });
    // A tree its window hasn't laid out, as one a `<style>` put in `<head>` has drawn again with nothing that looks different, so
    // that the window has no reason to, is laid out for this, as `chainOf` does: given with no boxes, every element found from it
    // said it had no size, and went on saying so as long as it was the same tree (a harness `click` after its first `eval`).
    let made = made.or_else(|| {
        rsui::geometry::ask_now(&tree, |root, layout| {
            add_elements(vm, root, Some(layout), &mut Vec::new(), &mut out);
            Some(())
        })
    });
    if made.is_none() {
        add_elements(vm, &tree, None, &mut Vec::new(), &mut out);
    }
    let changed_content = GIVEN.with(|given| given.borrow().as_ref()
        .is_none_or(|(before, _)| !same_page_content(before, &tree)));
    let content_version = CONTENT_VERSION.with(|version| {
        if changed_content { version.set(version.get().saturating_add(1)); }
        version.get()
    });
    GIVEN.with(|given| *given.borrow_mut() = Some((tree.clone(), made.is_some())));
    // Only the root serialization entry carries this additional metadata. Build
    // its array directly, so inherited index setters cannot execute user code.
    if content_version < (1u64 << 53)
        && let Some(root) = out.first_mut()
        && let Ok(mut fields) = vm.data_surface_array(*root, 13)
    {
        fields.push(Value::number(content_version as f64));
        *root = vm.new_array(fields);
    }
    if rsui::split::on() {
        rsui::split::count(
            rsui::split::named("pageTree-elements"),
            out.len() as u64 / 2,
        );
    }
    let array = vm.new_array(out);
    drop(split);
    Ok(array)
}

/// Whether the page-tree serialization is identical apart from its rectangles.
/// Keep the original GIVEN anchor: certifying selector topology must never make
/// ordinary geometry queries believe old rectangles describe the current tree.
fn same_page_content(one: &Node, other: &Node) -> bool {
    if same(one, other) {
        return true;
    }
    if one.role != other.role
        || one.name != other.name
        || one.key != other.key
        || one.props.is_empty() != other.props.is_empty()
        || one.children.len() != other.children.len()
    {
        return false;
    }
    for name in [
        symbol!("dom_key"),
        symbol!("class"),
        symbol!("__id"),
        symbol!("value"),
    ] {
        if one.props.get_symbol(name) != other.props.get_symbol(name) {
            return false;
        }
    }
    if !one
        .props
        .iter()
        .filter(|(name, _)| name.as_str().starts_with("data-"))
        .eq(other
            .props
            .iter()
            .filter(|(name, _)| name.as_str().starts_with("data-")))
    {
        return false;
    }
    one.children
        .iter()
        .zip(&other.children)
        .all(|(one, other)| same_page_content(one, other))
}

fn host_page_tree_content_kept(_: &mut Vm, _: Args) -> Result<Value, Value> {
    let Some(tree) = tree_now() else {
        return Ok(Value::FALSE);
    };
    Ok(Value::bool(GIVEN.with(|given| {
        given
            .borrow()
            .as_ref()
            .is_some_and(|(before, _)| same_page_content(before, &tree))
    })))
}

fn host_page_tree_content_version(_: &mut Vm, _: Args) -> Result<Value, Value> {
    let Some(tree) = tree_now() else { return Ok(Value::UNDEFINED); };
    let kept = GIVEN.with(|given| given.borrow().as_ref()
        .is_some_and(|(before, _)| same_page_content(before, &tree)));
    let version = CONTENT_VERSION.with(std::cell::Cell::get);
    Ok(if kept && version < (1u64 << 53) { Value::number(version as f64) } else { Value::UNDEFINED })
}

/// Whether this tree is the one that's been laid out, which is when what `pageTree` gives says where each element is.
fn laid_out(tree: &Node) -> bool {
    rsui::geometry::ask(|root, _| same(root, tree).then_some(())).is_some()
}

/// `pageTreeKept()`: whether what `pageTree` gave last is still the tree as it is now, as it would say by giving nothing, without
/// its making anything or noting that it's been asked.
fn host_page_tree_kept(_: &mut Vm, _: Args) -> Result<Value, Value> {
    let Some(tree) = tree_now() else {
        // `pageTree` gives nothing then too, which keeps what it gave.
        return Ok(Value::TRUE);
    };
    let laid_out = laid_out(&tree);
    let kept = GIVEN.with(|given| {
        given
            .borrow()
            .as_ref()
            .is_some_and(|(before, was_laid_out)| {
                same(before, &tree) && (*was_laid_out || !laid_out)
            })
    });
    Ok(Value::bool(kept))
}

/// `pageChainOf(key)`: what's along the way from the root to the element with this key in the tree as it is now, each as
/// `pageTree` gives it, with where each was just as it says that, for what only asks about what an element's in, as
/// `closest()` mostly does, without all of the page being made for it. It's undefined if it isn't there, or there's no tree.
fn host_page_chain_of(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let key = args.get(vm, 0);
    let Some(key) = vm.plain_text(key) else {
        return Ok(Value::UNDEFINED);
    };
    let Some(tree) = tree_now() else {
        return Ok(Value::UNDEFINED);
    };
    let Some(path) = path_of_key(&tree, &key) else {
        return Ok(Value::UNDEFINED);
    };
    Ok(chain_in(vm, &tree, &path))
}

/// What one part of a selector says, as `.item[data-id="3"]` does: classes, an id, and attributes, with what each is if
/// that's said. What kind of element it is isn't known here, which is left to what asks.
#[derive(Default)]
struct Wanted {
    classes: Vec<String>,
    id: Option<String>,
    attributes: Vec<(String, Option<String>)>,
}

impl Wanted {
    fn read(text: &str) -> Option<Self> {
        let mut wanted = Wanted::default();
        let mut rest = text.trim();
        // What kind it is comes first, if it's said.
        let kind = rest.find(['.', '#', '[']).unwrap_or(rest.len());
        rest = &rest[kind..];
        while !rest.is_empty() {
            let name_end = |from: &str| from.find(['.', '#', '[']).unwrap_or(from.len());
            if let Some(after) = rest.strip_prefix('.') {
                let end = name_end(after);
                wanted.classes.push(after[..end].to_owned());
                rest = &after[end..];
            } else if let Some(after) = rest.strip_prefix('#') {
                let end = name_end(after);
                wanted.id = Some(after[..end].to_owned());
                rest = &after[end..];
            } else if let Some(after) = rest.strip_prefix('[') {
                let end = after.find(']')?;
                let inside = &after[..end];
                rest = &after[end + 1..];
                match inside.split_once('=') {
                    Some((name, value)) => {
                        let value = value.trim().trim_matches(['"', '\'']);
                        wanted
                            .attributes
                            .push((name.trim().to_owned(), Some(value.to_owned())));
                    }
                    None => wanted.attributes.push((inside.trim().to_owned(), None)),
                }
            } else {
                return None;
            }
        }
        Some(wanted)
    }

    fn matches(&self, node: &Element) -> bool {
        let text = |value: Option<&PropValue>| match value {
            Some(PropValue::Str(text)) => Some(text.as_str().to_owned()),
            Some(PropValue::Int(number)) => Some(number.to_string()),
            Some(PropValue::Float(number)) => Some(number.to_string()),
            Some(PropValue::Bool(on)) => on.then(String::new),
            None => None,
        };
        let class = text(node.props.get_symbol(symbol!("class")));
        let id = text(node.props.get_symbol(symbol!("__id")));
        if self.classes.is_empty() && self.id.is_none() && self.attributes.is_empty() {
            return false;
        }
        if !self.classes.iter().all(|wanted| {
            class
                .as_deref()
                .is_some_and(|class| class.split_whitespace().any(|one| one == wanted))
        }) {
            return false;
        }
        if self.id.is_some() && self.id != id {
            return false;
        }
        self.attributes.iter().all(|(name, value)| {
            let has = match name.as_str() {
                "class" => class.clone(),
                "id" => id.clone(),
                "aria-label" => text(node.props.get_symbol(symbol!("__label"))),
                other => text(node.props.get(other)),
            };
            match (has, value) {
                (Some(has), Some(value)) => &has == value,
                (has, None) => has.is_some(),
                (None, Some(_)) => false,
            }
        })
    }
}

fn find_wanted(
    node: &Element,
    wanted: &Wanted,
    first: bool,
    path: &mut Vec<u32>,
    found: &mut Vec<Vec<u32>>,
) {
    if first && !found.is_empty() {
        return;
    }
    if wanted.matches(node) {
        found.push(path.clone());
    }
    for (index, child) in node.children.iter().enumerate() {
        path.push(index as u32);
        find_wanted(child, wanted, first, path, found);
        path.pop();
    }
}

/// `chainsOf(selector, first)`: what's along the way to each element that one part of a selector says, as far as what's
/// shown says, in the order they're in, or only the first.
fn host_chains_of(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (selector, first) = (args.get(vm, 0), args.get(vm, 1));
    let first = first == Value::TRUE;
    let Some(wanted) = vm.plain_text(selector).as_deref().and_then(Wanted::read) else {
        return Ok(vm.new_array(Vec::new()));
    };
    let Some(tree) = tree_now() else {
        return Ok(vm.new_array(Vec::new()));
    };
    let mut found = Vec::new();
    find_wanted(&tree, &wanted, first, &mut Vec::new(), &mut found);
    let chains = found.iter().map(|path| chain_in(vm, &tree, path)).collect();
    Ok(vm.new_array(chains))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsui::core::Role;

    #[test]
    fn linear_chain_rectangles_keep_missing_layout_prefixes_zero() {
        let root = Element::new(Role::Group)
            .child(Element::new(Role::Group).child(Element::new(Role::Group)))
            .into_node();
        let shorter = Element::new(Role::Group)
            .child(Element::new(Role::Group).prop("height", 25.0))
            .into_node();
        let mut laid_out = Layout::new();
        laid_out.compute(&shorter, rsui::layout::Size::new(100.0, 80.0), &mut rsui::layout::FixedMetrics::default());
        let unlaid = Layout::new();
        let mut engine = rsui_js::Engine::new();
        let vm = engine.vm();
        for layout in [None, Some(&unlaid), Some(&laid_out)] {
            for path in [&[][..], &[0][..], &[0, 0][..], &[0, 9, 0][..]] {
                let chain = chain_to(vm, &root, layout, path);
                let entries = vm.array_elements(chain).unwrap();
                let mut node: &Element = &root;
                let mut expected = Vec::new();
                for depth in 0..=path.len() {
                    if depth > 0 {
                        let Some(child) = node.children.get(path[depth - 1] as usize) else { break };
                        node = child;
                    }
                    expected.push(layout.and_then(|layout| layout.rect(&path[..depth]))
                        .map_or([0.0; 4], |rect| [f64::from(rect.x), f64::from(rect.y), f64::from(rect.width), f64::from(rect.height)]));
                }
                let actual: Vec<[f64; 4]> = entries.iter().map(|&entry| {
                    let entry = vm.array_elements(entry).unwrap();
                    std::array::from_fn(|index| entry[index + 6].as_number())
                }).collect();
                assert_eq!(actual, expected, "{path:?}");
            }
        }
    }

    /// A number from a seed, the same each run.
    fn next(seed: &mut u64) -> u64 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *seed >> 33
    }

    fn made(seed: &mut u64, counter: &mut u32, depth: u32) -> Node {
        *counter += 1;
        let mut element = Element::new(Role::Group);
        // Some have no key, some a `__dom` key, and some a `dom_key` prop, as a page's do.
        match next(seed) % 3 {
            0 => {}
            1 => element = element.key(format!("__dom{counter}")),
            _ => element = element.prop("dom_key", format!("__dom{counter}")),
        }
        if depth > 0 {
            for _ in 0..next(seed) % 4 {
                element = element.child(made(seed, counter, depth - 1));
            }
        }
        Node::new(element)
    }

    /// The tree with one thing changed somewhere: a child added, taken away, moved, or made again, sharing the rest.
    fn changed(node: &Node, seed: &mut u64, counter: &mut u32) -> Node {
        let mut element = (**node).clone();
        let count = element.children.len();
        let choice = next(seed) % 6;
        if count > 0 && choice < 2 {
            let at = (next(seed) % count as u64) as usize;
            element.children[at] = changed(&element.children[at], seed, counter);
        } else if choice == 2 {
            let at = (next(seed) % (count as u64 + 1)) as usize;
            element.children.insert(at, made(seed, counter, 2));
        } else if count > 0 && choice == 3 {
            element
                .children
                .remove((next(seed) % count as u64) as usize);
        } else if count > 1 && choice == 4 {
            let child = element.children.remove(0);
            element.children.push(child);
        } else {
            element = element.prop("changed", next(seed) as f64);
        }
        Node::new(element)
    }

    #[test]
    fn keys_found_from_what_changed_are_where_finding_them_all_puts_them() {
        let mut seed = 7;
        let mut counter = 0;
        for _ in 0..40 {
            let mut tree = made(&mut seed, &mut counter, 5);
            let mut found = HashMap::new();
            find_keyed(&tree, &mut Vec::new(), &mut found);
            for _ in 0..30 {
                let next_tree = changed(&tree, &mut seed, &mut counter);
                keyed_changes(&tree, &next_tree, &mut Vec::new(), &mut found);
                let mut all = HashMap::new();
                find_keyed(&next_tree, &mut Vec::new(), &mut all);
                assert_eq!(found, all);
                tree = next_tree;
            }
        }
    }
}

#[cfg(test)]
mod selector_content_tests {
    use super::*;
    use rsui::core::Role;
    fn base() -> Node {
        Node::new(
            Element::new(Role::Group)
                .key("root")
                .prop("dom_key", "__dom0")
                .child(Node::new(
                    Element::new(Role::Text)
                        .key("child")
                        .name("text")
                        .prop("dom_key", "__dom1"),
                )),
        )
    }
    #[test]
    fn page_content_ignores_geometry_and_paint_but_preserves_serialized_inputs() {
        let root = base();
        let paint = Node::new(
            (*root)
                .clone()
                .prop("scroll_y", 17.25)
                .prop("transform", "translateX(5px)")
                .prop("opacity", 0.4),
        );
        assert!(same_page_content(&root, &paint));
        for name in ["dom_key", "class", "__id", "value", "data-state"] {
            let changed = Node::new((*root).clone().prop(name, "changed"));
            assert!(!same_page_content(&root, &changed), "{name}");
        }
        assert!(!same_page_content(
            &root,
            &Node::new((*root).clone().key("different"))
        ));
        assert!(!same_page_content(
            &root,
            &Node::new((*root).clone().name("different"))
        ));
        let mut changed = (*root).clone();
        changed.role = Role::Text;
        assert!(!same_page_content(&root, &Node::new(changed)));
        let mut changed = (*root).clone();
        changed.children[0] = Node::new((*root.children[0]).clone().name(""));
        assert!(!same_page_content(&root, &Node::new(changed)));
        let empty = Node::new(Element::new(Role::Group));
        assert!(!same_page_content(
            &empty,
            &Node::new((*empty).clone().prop("opacity", 1.0))
        ));
        let mut changed = (*root).clone();
        changed
            .children
            .push(Node::new(Element::new(Role::Group).key("second")));
        let before = Node::new(changed.clone());
        changed.children.swap(0, 1);
        assert!(!same_page_content(&before, &Node::new(changed)));
    }
}
