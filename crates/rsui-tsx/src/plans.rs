//! What `planOf` in web.js finds once for each list of classes, kind of element and id: the page's rules that could be about such
//! an element, put in the cascade's order, and whether what those about it alone say can be put on all together. web.js finds
//! the rules and keeps what they say; this keeps which props each of them says, by a number web.js keeps on the rule, so that
//! putting them in order and looking at what goes on apart reads nothing from JavaScript but numbers. A plan for a Mantine
//! modal's elements has about a hundred items, and finding which of them can go on together read every rule's props again,
//! for each of the 95 plans made each time it opened.

use std::cell::RefCell;
use std::collections::HashMap as StdHashMap;

use rsui_js::{Args, HostFunction, Value, Vm};

type Run<T> = Result<T, Value>;
/// A map with a quick hash, as what's looked up here is looked up for each rule of each element.
type HashMap<K, V> = StdHashMap<K, V, foldhash::fast::FixedState>;

/// The columns of a plan's item, as web.js's `PLAN_COLUMNS` has them: where it is in the page's rules (-1 for a rule about one
/// class or kind), how much it says, its sheet's place, its place in its sheet, and 1 if it's about the element alone.
const COLUMNS: usize = 5;
/// What `cssPlanMake` is given for each item: the same, with what it says (`cssPlanSaid`) after where it is.
const GIVEN: usize = 6;

/// What a rule says, as `itemsApart` in web.js looks at it: the props it sets (`::before` for a rule about a part), and those of
/// them that are `!important`.
struct Said {
    keys: Box<[u32]>,
    important: Box<[u32]>,
}

#[derive(Clone, Copy)]
struct Item {
    at: f64,
    said: u32,
    weight: f64,
    rank: f64,
    order: f64,
    alone: bool,
    /// Its place among what was given.
    given: u32,
}

#[derive(Default)]
struct Store {
    names: HashMap<Box<str>, u32>,
    said: Vec<Said>,
    said_ids: HashMap<(Box<[u32]>, Box<[u32]>), u32>,
    items: Vec<Item>,
    /// Which props an item after this one says, in `apart`, as the number of the time they were put in.
    later: Vec<u32>,
    later_important: Vec<u32>,
    later_now: u32,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store::default());
}

/// Forgets everything, for an engine that starts on a thread another has been on.
pub(crate) fn reset() {
    STORE.with(|store| *store.borrow_mut() = Store::default());
}

fn texts(vm: &Vm, value: Value) -> Vec<String> {
    vm.array_elements(value)
        .map(|items| items.iter().map(|&item| vm.string_text(item).map(|text| text.into_owned()).unwrap_or_default()).collect())
        .unwrap_or_default()
}

impl Store {
    fn name(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.names.get(name) {
            return id;
        }
        let id = self.names.len() as u32;
        self.names.insert(name.into(), id);
        id
    }

    /// Whether what the items about the element alone say can be put on all together before the rest, as `itemsApart` has it:
    /// nothing an item about more than that says is said again by one about it alone after it, where `!important` is only gone
    /// over by what's `!important` too.
    fn apart(&mut self, from: usize, to: usize) -> bool {
        let size = self.names.len();
        if self.later.len() < size {
            self.later.resize(size, 0);
            self.later_important.resize(size, 0);
        }
        self.later_now = self.later_now.wrapping_add(1);
        if self.later_now == 0 {
            self.later.fill(0);
            self.later_important.fill(0);
            self.later_now = 1;
        }
        let now = self.later_now;
        for item in self.items[from..to].iter().rev() {
            let said = &self.said[item.said as usize];
            if item.alone {
                for &key in &said.keys {
                    self.later[key as usize] = now;
                }
                for &key in &said.important {
                    self.later_important[key as usize] = now;
                }
            } else if said.keys.iter().any(|key| {
                let later = if said.important.contains(key) { &self.later_important } else { &self.later };
                later[*key as usize] == now
            }) {
                return false;
            }
        }
        true
    }
}

/// `cssPlanSaid(keys, important)`: a number for what a rule says, the same for the same props, for `cssPlanMake` to be told.
fn host_said(vm: &mut Vm, args: Args) -> Run<Value> {
    let (keys, important) = (texts(vm, args.get(vm, 0)), texts(vm, args.get(vm, 1)));
    let id = STORE.with(|store| {
        let mut store = store.borrow_mut();
        let keys: Box<[u32]> = keys.iter().map(|key| store.name(key)).collect();
        let important: Box<[u32]> = important.iter().map(|key| store.name(key)).collect();
        if let Some(&id) = store.said_ids.get(&(keys.clone(), important.clone())) {
            return id;
        }
        let id = store.said.len() as u32;
        store.said.push(Said { keys: keys.clone(), important: important.clone() });
        store.said_ids.insert((keys, important), id);
        id
    });
    Ok(Value::number(id as f64))
}

/// `cssPlanMake(count, given, columns, sources)`: a plan's items in the cascade's order, from the first `count` of what `given`
/// (a `Float64Array`) holds, six numbers for each: where it is in the page's rules (-1 for a rule about one class or kind), what
/// it says (`cssPlanSaid`), how much, its sheet's place, its place in its sheet, and 1 if it's about the element alone. Where
/// nothing tells two apart, they stay in the order they were given, as web.js's sort keeps them. The plan's columns go into
/// `columns` (a `Float64Array`, five numbers for each, as web.js's plan has them), and where each was among what was given
/// into `sources` (a `Float64Array`). Gives back where what says more than one class does starts, times 8, with 4 if any is
/// about more than the element alone, and 2 and 1 for whether each side of that can be put on apart.
fn host_make(vm: &mut Vm, args: Args) -> Run<Value> {
    let (count, given, columns, sources) = (args.get(vm, 0), args.get(vm, 1), args.get(vm, 2), args.get(vm, 3));
    let count = if count.is_number() { count.as_number().max(0.0) as usize } else { 0 };
    let shape = STORE.with(|store| {
        let mut store = store.borrow_mut();
        let store = &mut *store;
        if store.said.is_empty() {
            store.said.push(Said { keys: Box::new([]), important: Box::new([]) });
        }
        store.items.clear();
        let bytes = vm.bytes(given).unwrap_or(&[]);
        let number = |at: usize| bytes.get(at * 8..at * 8 + 8).map_or(f64::NAN, |chunk| f64::from_le_bytes(chunk.try_into().unwrap()));
        for place in 0..count.min(bytes.len() / (8 * GIVEN)) {
            let at = place * GIVEN;
            let said = number(at + 1);
            store.items.push(Item {
                at: number(at),
                said: if said >= 0.0 && (said as usize) < store.said.len() { said as u32 } else { 0 },
                weight: number(at + 2),
                rank: number(at + 3),
                order: number(at + 4),
                alone: number(at + 5) != 0.0,
                given: place as u32,
            });
        }
        // A sort that keeps the order of those it can't tell apart, as web.js's does: `x.weight - y.weight || x.rank - y.rank
        // || x.order - y.order`.
        let sign = |difference: f64| difference.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal);
        store.items.sort_by(|x, y| sign(x.weight - y.weight).then_with(|| sign(x.rank - y.rank)).then_with(|| sign(x.order - y.order)));
        let size = store.items.len();
        let split = store.items.iter().position(|item| item.weight > 100.0).unwrap_or(size);
        let others = store.items.iter().any(|item| !item.alone);
        let apart = [store.apart(0, split), store.apart(split, size)];
        split * 8 + usize::from(others) * 4 + usize::from(apart[0]) * 2 + usize::from(apart[1])
    });
    STORE.with(|store| {
        let store = store.borrow();
        if let Some(bytes) = vm.bytes_mut(columns) {
            for (item, out) in store.items.iter().zip(bytes.chunks_exact_mut(8 * COLUMNS)) {
                let values = [item.at, item.weight, item.rank, item.order, if item.alone { 1.0 } else { 0.0 }];
                for (value, place) in values.iter().zip(out.chunks_exact_mut(8)) {
                    place.copy_from_slice(&value.to_le_bytes());
                }
            }
        }
        if let Some(bytes) = vm.bytes_mut(sources) {
            for (item, place) in store.items.iter().zip(bytes.chunks_exact_mut(8)) {
                place.copy_from_slice(&f64::from(item.given).to_le_bytes());
            }
        }
    });
    Ok(Value::number(shape as f64))
}

pub(crate) const FUNCTIONS: [(&str, HostFunction); 2] = [("cssPlanSaid", host_said), ("cssPlanMake", host_make)];
