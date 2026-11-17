//! Where a thread's time goes, part by part, for measuring: `RSUI_SPLIT=1` has [`enter`] start charging time to a part
//! until what it gives back is dropped, when the part that was being charged before is again. Each part's time is its own,
//! without the parts entered inside it, so JavaScript that calls Rust that calls JavaScript is told apart. Unset, [`enter`]
//! reads one flag and does nothing else. [`take`] gives what each part has had since it was last asked.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

/// Nothing entered.
pub const OUTSIDE: u16 = 0;
/// Running JavaScript, from Rust calling into the engine.
pub const JS: u16 = 1;
/// Collecting the engine's garbage.
pub const GC: u16 = 2;
/// `Runtime::flush`: Rust working out what's changed in the tree.
pub const FLUSH: u16 = 3;
/// Working out what needs laying out from what changed.
pub const LAYOUT_APPLY: u16 = 4;
/// Laying out.
pub const LAYOUT: u16 = 5;
/// Working out what to draw.
pub const PAINT: u16 = 6;
/// Telling `rsui::geometry` where things are after a layout.
pub const GEOMETRY: u16 = 7;
/// Handling an input before it reaches JavaScript.
pub const INPUT: u16 = 8;
/// `pageTree` building its list of the page.
pub const PAGE_TREE_BUILD: u16 = 9;
/// Where named parts, such as each host function, start.
pub const NAMED: u16 = 32;

const NAMES: [&str; 10] =
    ["outside", "js", "gc", "flush", "layout-apply", "layout", "paint", "geometry", "input", "pageTree-build"];

struct State {
    current: u16,
    since: Instant,
    /// Nanoseconds and times entered, by part.
    totals: Vec<(u64, u64)>,
    names: Vec<String>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        current: OUTSIDE,
        since: Instant::now(),
        totals: Vec::new(),
        names: NAMES.iter().map(|name| (*name).to_owned()).collect(),
    });
}

/// Whether `RSUI_SPLIT` asks for this.
#[inline]
pub fn on() -> bool {
    static ON: AtomicU8 = AtomicU8::new(0);
    match ON.load(Ordering::Relaxed) {
        0 => {
            let on = std::env::var_os("RSUI_SPLIT").is_some_and(|value| value != "0");
            ON.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        known => known == 2,
    }
}

/// A part with a name of its own, numbered from [`NAMED`], the same number each time it's asked for by that name.
pub fn named(name: &str) -> u16 {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        if let Some(at) = state.names.iter().skip(NAMED as usize).position(|one| one == name) {
            return NAMED + at as u16;
        }
        while state.names.len() < NAMED as usize {
            let at = state.names.len();
            state.names.push(format!("part{at}"));
        }
        state.names.push(name.to_owned());
        (state.names.len() - 1) as u16
    })
}

fn charge(state: &mut State, now: Instant) {
    let part = state.current as usize;
    if state.totals.len() <= part {
        state.totals.resize(part + 1, (0, 0));
    }
    state.totals[part].0 += now.duration_since(state.since).as_nanos() as u64;
    state.since = now;
}

/// Charges time to `part` until this is dropped.
pub struct Scope {
    before: u16,
}

impl Drop for Scope {
    fn drop(&mut self) {
        let now = Instant::now();
        let _ = STATE.try_with(|state| {
            let mut state = state.borrow_mut();
            charge(&mut state, now);
            state.current = self.before;
        });
    }
}

/// Starts charging time to `part`, if `RSUI_SPLIT` is set.
#[inline]
pub fn enter(part: u16) -> Option<Scope> {
    if !on() {
        return None;
    }
    enter_now(part)
}

#[inline(never)]
fn enter_now(part: u16) -> Option<Scope> {
    let now = Instant::now();
    STATE
        .try_with(|state| {
            let mut state = state.borrow_mut();
            charge(&mut state, now);
            let before = state.current;
            state.current = part;
            if state.totals.len() <= part as usize {
                state.totals.resize(part as usize + 1, (0, 0));
            }
            state.totals[part as usize].1 += 1;
            Scope { before }
        })
        .ok()
}

/// Counts `count` more of something under `part`'s name, with no time, as how many elements a list had.
pub fn count(part: u16, count: u64) {
    if !on() {
        return;
    }
    let _ = STATE.try_with(|state| {
        let mut state = state.borrow_mut();
        let at = part as usize;
        if state.totals.len() <= at {
            state.totals.resize(at + 1, (0, 0));
        }
        state.totals[at].1 += count;
    });
}

/// Each part that's had time or been entered since this was last asked, with its name, nanoseconds and times entered,
/// and starts again from nothing. What's being charged now goes on being charged.
pub fn take() -> Vec<(String, u64, u64)> {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        charge(&mut state, Instant::now());
        let totals = std::mem::take(&mut state.totals);
        totals
            .into_iter()
            .enumerate()
            .filter(|(_, (nanos, times))| *nanos > 0 || *times > 0)
            .map(|(at, (nanos, times))| {
                let name = state.names.get(at).cloned().unwrap_or_else(|| format!("part{at}"));
                (name, nanos, times)
            })
            .collect()
    })
}
