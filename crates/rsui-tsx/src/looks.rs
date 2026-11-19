//! Putting a rule's props on an element's look, as `mergeLook` in web.js does: what the props say goes over what the look has,
//! and what in the look waits for one of the element's own variables (`__var name`) is taken out where the props say everything
//! it would set, as `.a { background-color: var(--v) }` then `.a:checked { background-color: green }` is green. That last part
//! was most of what `mergeLook` took in web.js, as nearly every look on shadcn's page waits for a variable and each rule looked
//! through all of them, and Rust finds them once for each shape of look.
//!
//! What a look has of the longhands of `transition` and `animation` (`__transitionParts`) has what the props say of them put over
//! it, as `longhandsOver` does. What's left to web.js, which is told so and does all of it: `!important` on either side, a
//! `display: flex` over a direction said before it, a declaration whose props aren't known yet (`said.keys`, which web.js finds
//! and keeps), and one the props say only some of again, which gets a new object.

use std::cell::RefCell;
use std::collections::HashMap as StdHashMap;

use rsui_js::{Args, Atom, HostFunction, Keys, Value, Vm};

type Run<T> = Result<T, Value>;
type HashMap<K, V> = StdHashMap<K, V, foldhash::fast::FixedState>;

/// How many shapes of look are remembered, before they're all forgotten and found again.
const KEPT_SHAPES: usize = 8192;

struct Names {
    /// What's read of the look: `__important`, `__vars`, `__transitionParts`, `__animationParts`, `direction`,
    /// `__directionByDisplay`.
    look: Keys,
    /// What's read of the props: `__important`, `__directionByDisplay`, `__transition`, `__transitionParts`, `__animation`,
    /// `__animationParts`.
    props: Keys,
    transition_parts: Atom,
    animation_parts: Atom,
    keys: Atom,
    over: Atom,
}

#[derive(Default)]
struct Store {
    names: Option<Names>,
    /// For each shape of look, which of its props wait for a variable, and where each is.
    waiting: HashMap<u64, Box<[(Atom, u32)]>>,
    /// Whether each name is one of those, by its atom, while atoms are the same.
    is_waiting: HashMap<Atom, bool>,
    atom_generation: u32,
    slots: Vec<(Atom, u32)>,
    values: Vec<Value>,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store::default());
}

/// Forgets everything, for an engine that starts on a thread another has been on.
pub(crate) fn reset() {
    STORE.with(|store| *store.borrow_mut() = Store::default());
}

/// `cssMergeLook(look, props, weight, inOrder)`: what `mergeLook` does, with `mergeWeight` and `mergeInOrder`, or `false`, with
/// nothing done, where it's left to web.js.
fn host_merge_look(vm: &mut Vm, args: Args) -> Run<Value> {
    let (look, props, weight, in_order) = (args.get(vm, 0), args.get(vm, 1), args.get(vm, 2), args.get(vm, 3) == Value::TRUE);
    let weight = if weight.is_number() { weight.as_number() } else { f64::INFINITY };
    let mut store = STORE.with(|store| std::mem::take(&mut *store.borrow_mut()));
    let done = merge_look(&mut store, vm, look, props, weight, in_order);
    STORE.with(|kept| *kept.borrow_mut() = store);
    Ok(Value::bool(done?))
}

fn merge_look(store: &mut Store, vm: &mut Vm, look: Value, props: Value, weight: f64, in_order: bool) -> Run<bool> {
    if !vm.is_plain_object(look) || !vm.is_plain_object(props) {
        return Ok(false);
    }
    let names = match &store.names {
        Some(names) => names,
        None => store.names.insert(Names {
            look: vm.keys(&["__important", "__vars", "__transitionParts", "__animationParts", "direction", "__directionByDisplay"]),
            props: vm.keys(&["__important", "__directionByDisplay", "__transition", "__transitionParts", "__animation", "__animationParts"]),
            transition_parts: vm.atom("__transitionParts"),
            animation_parts: vm.atom("__animationParts"),
            keys: vm.atom("keys"),
            over: vm.atom("over"),
        }),
    };
    let (keys_atom, over_atom) = (names.keys, names.over);
    let parts_atoms = [names.transition_parts, names.animation_parts];
    let mut had = [Value::UNDEFINED; 6];
    vm.read_own_keys(look, names.look, &mut had)?;
    let mut says = [Value::UNDEFINED; 6];
    vm.read_own_keys(props, names.props, &mut says)?;
    let [kept, vars, transition_parts, animation_parts, direction, by_display] = had;
    if !kept.is_undefined() || !says[0].is_undefined() {
        return Ok(false);
    }
    if says[1] == Value::TRUE && !direction.is_undefined() && by_display != Value::TRUE {
        return Ok(false);
    }
    // `replacedVars`: what the props say all of again goes, before they're put on.
    let mut gone: Vec<Atom> = Vec::new();
    if vars == Value::TRUE && (weight > 100.0 || in_order) {
        if store.atom_generation != vm.atom_generation() {
            store.atom_generation = vm.atom_generation();
            store.is_waiting.clear();
            store.waiting.clear();
        }
        // A look that's had something taken out has a shape of its own, and what waits is found in it each time.
        let stamp = vm.shape_stamp(look);
        let mut alone = Vec::new();
        if stamp.is_none_or(|stamp| !store.waiting.contains_key(&stamp)) {
            vm.own_slots_into(look, &mut store.slots);
            for &(key, slot) in &store.slots {
                let waits = *store.is_waiting.entry(key).or_insert_with(|| vm.atom_name(key).starts_with("__var "));
                if waits {
                    alone.push((key, slot));
                }
            }
            if let Some(stamp) = stamp {
                if store.waiting.len() >= KEPT_SHAPES {
                    store.waiting.clear();
                }
                store.waiting.insert(stamp, std::mem::take(&mut alone).into());
            }
        }
        let waiting = match stamp {
            Some(stamp) => &store.waiting[&stamp],
            None => &alone[..],
        };
        let mut over: Vec<Atom> = Vec::new();
        for &(key, slot) in waiting.iter() {
            if vm.has_own_name(props, key) {
                continue;
            }
            let said = vm.slot_value(look, slot);
            if said.is_nullish() {
                continue;
            }
            // What it sets, which web.js finds the first time and keeps on it.
            let Some(sets) = vm.own_value(said, keys_atom).filter(|sets| !sets.is_undefined()) else {
                return Ok(false);
            };
            let Some(found) = vm.array_elements(sets) else {
                return Ok(false);
            };
            let sets = &mut store.values;
            sets.clear();
            sets.extend_from_slice(found);
            over.clear();
            for one in sets.iter().copied() {
                if !one.is_string() {
                    return Ok(false);
                }
                let atom = vm.property_atom(one)?;
                if vm.has_own_name(props, atom) {
                    over.push(atom);
                }
            }
            if !sets.is_empty() && over.len() == sets.len() {
                gone.push(key);
            } else if !over.is_empty() {
                // Some of it is said again: kept, noting which, unless that's noted already.
                let Some(noted) = vm.own_value(said, over_atom).filter(|noted| !noted.is_undefined()) else {
                    return Ok(false);
                };
                let Some(noted) = vm.array_elements(noted).map(<[Value]>::to_vec) else {
                    return Ok(false);
                };
                for one in &over {
                    let mut found = false;
                    for name in noted.iter().copied() {
                        if name.is_string() && vm.property_atom(name)? == *one {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        return Ok(false);
                    }
                }
            }
        }
    }
    // Taken out keeping a shape shared with other looks, as most are (see `Vm::delete_keys`).
    vm.delete_keys(look, &gone)?;
    vm.assign(look, props)?;
    // `longhandsOver`: what the look had of `transition`'s and `animation`'s longhands, with what the props say over it, unless
    // they say the shorthand, which says all of it again.
    for (at, before) in [transition_parts, animation_parts].into_iter().enumerate() {
        if before.is_undefined() {
            continue;
        }
        let (shorthand, parts) = (says[2 + at * 2], says[3 + at * 2]);
        if !shorthand.is_undefined() {
            if parts.is_undefined() {
                vm.delete_property(look, parts_atoms[at])?;
            }
        } else if !parts.is_undefined() {
            let both = vm.new_object();
            vm.assign(both, before)?;
            vm.assign(both, parts)?;
            vm.set_property(look, parts_atoms[at], both)?;
        } else if vm.has_own_name(props, parts_atoms[at]) {
            // Props that say `undefined` for them leave what was there.
            vm.set_property(look, parts_atoms[at], before)?;
        }
    }
    Ok(true)
}

pub(crate) const FUNCTIONS: [(&str, HostFunction); 1] = [("cssMergeLook", host_merge_look)];
