use std::cmp::Ordering;
use std::collections::HashMap;

use foldhash::fast::RandomState;
use triomphe::Arc;

use crate::element::{Key, Node, Props};
use crate::patch::{Patch, Path};

/// Marks "no index" in the index tables below.
const NONE: u32 = u32::MAX;

/// Beyond this many moves in one list, a single [`Patch::Reorder`] is cheaper to
/// apply: each move shifts part of the list, while a reorder is one pass over it.
const MAX_MOVES: usize = 64;

/// Computes the patches that turn `old` into `new`.
///
/// Children are matched by key, and unkeyed children by their order among the
/// unkeyed. If a key repeats, its occurrences match in order. A child whose role
/// or key changed is replaced. Reordered children produce the fewest moves
/// possible, or a single [`Patch::Reorder`] if that would be too many moves.
/// Subtrees shared by `old` and `new` are skipped without being visited. Equal
/// trees cost one small allocation at most, whatever their size.
pub fn diff(old: &Node, new: &Node) -> Vec<Patch> {
    let mut patches = Vec::new();
    diff_at(&[], old, new, &mut patches);
    patches
}

/// Like [`diff`] for two versions of the node at `path` in a larger tree: appends
/// patches that apply to that tree. This lets an update cost what changed, however
/// large the tree around it is.
pub fn diff_at(path: &[u32], old: &Node, new: &Node, patches: &mut Vec<Patch>) {
    let mut differ = Differ {
        patches: std::mem::take(patches),
        path: path.to_vec(),
    };
    differ.node(old, new);
    *patches = differ.patches;
}

struct Differ {
    patches: Vec<Patch>,
    /// Path of the node being visited. Only copied when a patch needs it.
    path: Vec<u32>,
}

impl Differ {
    fn here(&self) -> Path {
        Path::from(self.path.as_slice())
    }

    fn node(&mut self, old: &Node, new: &Node) {
        if Arc::ptr_eq(old, new) {
            return;
        }
        if old.key != new.key || old.role != new.role {
            self.patches.push(Patch::Replace {
                path: self.here(),
                node: new.clone(),
            });
            return;
        }
        if old.name != new.name {
            self.patches.push(Patch::SetName {
                path: self.here(),
                name: new.name.clone(),
            });
        }
        self.props(&old.props, &new.props);
        if old.actions != new.actions {
            self.patches.push(Patch::SetActions {
                path: self.here(),
                actions: new.actions,
            });
        }
        self.children(&old.children, &new.children);
    }

    /// Both sets are sorted by name, so one pass over them finds every change.
    fn props(&mut self, old: &Props, new: &Props) {
        // A shared set is the same as itself, however long it is, and a `dom_key` beside it is all else there is.
        let own = old.dom_key() == new.dom_key();
        if own && old.identity() == new.identity() {
            return;
        }
        if own && old.entries() == new.entries() {
            return;
        }
        let (mut old, mut new) = (old.iter().peekable(), new.iter().peekable());
        loop {
            let order = match (old.peek(), new.peek()) {
                (Some((old_name, _)), Some((new_name, _))) => old_name.cmp(new_name),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => break,
            };
            if order == Ordering::Less {
                let (name, _) = old.next().expect("there's one, as it was looked at");
                self.patches.push(Patch::RemoveProp {
                    path: self.here(),
                    name,
                });
                continue;
            }
            let (name, value) = new.next().expect("there's one, as it was looked at");
            if order == Ordering::Greater || old.peek().is_some_and(|(_, was)| *was != value) {
                self.patches.push(Patch::SetProp {
                    path: self.here(),
                    name,
                    value: value.clone(),
                });
            }
            if order == Ordering::Equal {
                old.next();
            }
        }
    }

    fn children(&mut self, old: &[Node], new: &[Node]) {
        // Most updates leave the ends of a list alone, so trim those first and
        // only pay for matching in the part that differs.
        let shared = old.len().min(new.len());
        let mut start = 0;
        while start < shared && old[start].key == new[start].key {
            start += 1;
        }
        // Only keys can match from the end. Unkeyed children match by order
        // from the front, so they would pair up differently from this side.
        let (mut old_end, mut new_end) = (old.len(), new.len());
        while old_end > start
            && new_end > start
            && old[old_end - 1].key.is_some()
            && old[old_end - 1].key == new[new_end - 1].key
        {
            old_end -= 1;
            new_end -= 1;
        }

        let source = self.reconcile(start, &old[start..old_end], &new[start..new_end]);

        // Every structural patch for this parent comes first, so the indices
        // below are final positions.
        for index in 0..start {
            self.child(index, &old[index], &new[index]);
        }
        for (offset, &from) in source.iter().enumerate() {
            if from != NONE {
                let index = start + offset;
                self.child(index, &old[start + from as usize], &new[index]);
            }
        }
        for offset in 0..new.len() - new_end {
            let index = new_end + offset;
            self.child(index, &old[old_end + offset], &new[index]);
        }
    }

    fn child(&mut self, index: usize, old: &Node, new: &Node) {
        if Arc::ptr_eq(old, new) {
            return;
        }
        self.path.push(index as u32);
        self.node(old, new);
        self.path.pop();
    }

    /// Emits the structural patches for the range of children starting at
    /// `start`. For each new child, returns the old child it continues, or `NONE`.
    fn reconcile(&mut self, start: usize, old: &[Node], new: &[Node]) -> Vec<u32> {
        if old.is_empty() && new.is_empty() {
            return Vec::new();
        }
        let parent = self.here();

        // For each identity, chain together the old children that have it, in
        // order. Each new child then takes the first one left in its chain.
        let mut first = KeyIndex::new(old);
        let mut next = vec![NONE; old.len()];
        for (index, child) in old.iter().enumerate().rev() {
            let head = first
                .slot(child.key.as_ref(), true)
                .expect("every old key has a slot");
            next[index] = *head;
            *head = index as u32;
        }

        let mut matched = vec![false; old.len()];
        let mut source = Vec::with_capacity(new.len());
        let mut in_order = true;
        let mut last = None;
        for child in new {
            let from = match first.slot(child.key.as_ref(), false) {
                Some(head) if *head != NONE => {
                    let from = *head;
                    *head = next[from as usize];
                    matched[from as usize] = true;
                    in_order &= last.is_none_or(|last| from > last);
                    last = Some(from);
                    from
                }
                _ => NONE,
            };
            source.push(from);
        }

        // From the back, so that a removal never shifts one still to come.
        for index in (0..old.len()).rev().filter(|&index| !matched[index]) {
            self.patches.push(Patch::Remove {
                parent: parent.clone(),
                index: (start + index) as u32,
            });
        }

        if in_order {
            self.insert_new(&parent, start, new, &source);
        } else {
            self.reorder(&parent, start, new, &source, &matched);
        }
        source
    }

    /// Emits moves and inserts for children that changed order, leaving the
    /// longest possible run of children in place.
    ///
    /// Working from the back, each child goes just before the one placed last.
    /// Its index depends on what is in front of it at that moment, so this lays
    /// out every position a child can pass through on one line, with a separate
    /// slot for where a moved child starts and where it ends. A Fenwick tree
    /// over that line counts the occupied slots in front of any position in
    /// O(log n), which keeps a full reversal of a long list O(n log n).
    fn reorder(
        &mut self,
        parent: &Path,
        start: usize,
        new: &[Node],
        source: &[u32],
        matched: &[bool],
    ) {
        let stays = longest_increasing_run(source);
        let kept = stays.iter().filter(|&&stays| stays).count();
        if matched.iter().filter(|&&matched| matched).count() - kept > MAX_MOVES {
            self.rearrange(parent, start, new, source, matched);
            return;
        }

        let mut occupied = Fenwick::new(matched.len() + new.len());
        let mut start_slot = vec![NONE; matched.len()];
        let mut end_slot = vec![NONE; new.len()];

        let (mut slot, mut next_old, mut next_new) = (0, 0, 0);
        for index in (0..=new.len()).filter(|&i| i == new.len() || stays[i]) {
            let old_limit = if index == new.len() {
                matched.len()
            } else {
                source[index] as usize
            };
            // Children still waiting to move sit in front of those that have
            // already moved into this gap.
            for from in (next_old..old_limit).filter(|&from| matched[from]) {
                start_slot[from] = slot;
                occupied.occupy(slot as usize);
                slot += 1;
            }
            for placed in &mut end_slot[next_new..index] {
                *placed = slot;
                slot += 1;
            }
            if index < new.len() {
                end_slot[index] = slot;
                occupied.occupy(slot as usize);
                slot += 1;
                next_old = old_limit + 1;
                next_new = index + 1;
            }
        }

        occupied.build();

        for index in (0..new.len()).rev() {
            if stays[index] {
                continue;
            }
            let end = end_slot[index] as usize;
            if source[index] == NONE {
                self.patches.push(Patch::Insert {
                    parent: parent.clone(),
                    index: start as u32 + occupied.before(end),
                    node: new[index].clone(),
                });
            } else {
                let begin = start_slot[source[index] as usize] as usize;
                let from = occupied.before(begin);
                occupied.add(begin, -1);
                let to = occupied.before(end);
                self.patches.push(Patch::Move {
                    parent: parent.clone(),
                    from: start as u32 + from,
                    to: start as u32 + to,
                });
            }
            occupied.add(end, 1);
        }
    }

    /// Emits one patch that puts the surviving children in their new order,
    /// followed by the inserts.
    fn rearrange(
        &mut self,
        parent: &Path,
        start: usize,
        new: &[Node],
        source: &[u32],
        matched: &[bool],
    ) {
        // Removals have already closed the gaps, so survivors are numbered
        // by how many survivors come before them.
        let mut survivors = 0;
        let position: Vec<u32> = matched
            .iter()
            .map(|&matched| {
                let current = survivors;
                survivors += matched as u32;
                current
            })
            .collect();
        self.patches.push(Patch::Reorder {
            parent: parent.clone(),
            start: start as u32,
            order: source
                .iter()
                .filter(|&&from| from != NONE)
                .map(|&from| position[from as usize])
                .collect(),
        });
        self.insert_new(parent, start, new, source);
    }

    /// Inserts the new children, assuming all the others are already in order.
    fn insert_new(&mut self, parent: &Path, start: usize, new: &[Node], source: &[u32]) {
        for (index, &from) in source.iter().enumerate() {
            if from == NONE {
                self.patches.push(Patch::Insert {
                    parent: parent.clone(),
                    index: (start + index) as u32,
                    node: new[index].clone(),
                });
            }
        }
    }
}

/// Finds the first remaining old child for a key.
enum KeyIndex<'a> {
    /// Integer keys in a compact range index a table directly, with no hashing.
    Dense {
        min: u64,
        heads: Vec<u32>,
    },
    Hashed(HashMap<Option<&'a Key>, u32, RandomState>),
}

impl<'a> KeyIndex<'a> {
    fn new(old: &'a [Node]) -> Self {
        let (mut min, mut max) = (u64::MAX, 0);
        let all_int = old.iter().all(|child| match child.key {
            Some(Key::Int(key)) => {
                min = min.min(key);
                max = max.max(key);
                true
            }
            _ => false,
        });
        if !old.is_empty() && all_int && (max - min) / 4 < old.len() as u64 {
            Self::Dense {
                min,
                heads: vec![NONE; (max - min) as usize + 1],
            }
        } else {
            Self::Hashed(HashMap::with_capacity_and_hasher(
                old.len(),
                RandomState::default(),
            ))
        }
    }

    /// The head of the chain for `key`. With `create`, one is made if needed.
    fn slot(&mut self, key: Option<&'a Key>, create: bool) -> Option<&mut u32> {
        match self {
            Self::Dense { min, heads } => match key {
                Some(Key::Int(key)) => heads.get_mut(key.checked_sub(*min)? as usize),
                _ => None,
            },
            Self::Hashed(heads) => {
                if create {
                    Some(heads.entry(key).or_insert(NONE))
                } else {
                    heads.get_mut(&key)
                }
            }
        }
    }
}

/// Flags the entries forming a longest increasing subsequence, ignoring `NONE`.
fn longest_increasing_run(values: &[u32]) -> Vec<bool> {
    let mut tails: Vec<u32> = Vec::new();
    let mut previous = vec![NONE; values.len()];
    for (index, &value) in values.iter().enumerate() {
        if value == NONE {
            continue;
        }
        // Lists are usually mostly in order, so try extending the run first.
        let extends_run = tails
            .last()
            .is_none_or(|&tail| values[tail as usize] < value);
        let length = if extends_run {
            tails.len()
        } else {
            tails.partition_point(|&tail| values[tail as usize] < value)
        };
        if length > 0 {
            previous[index] = tails[length - 1];
        }
        if length == tails.len() {
            tails.push(index as u32);
        } else {
            tails[length] = index as u32;
        }
    }

    let mut flags = vec![false; values.len()];
    let mut current = tails.last().copied().unwrap_or(NONE);
    while current != NONE {
        flags[current as usize] = true;
        current = previous[current as usize];
    }
    flags
}

/// A binary indexed tree over slot occupancy.
struct Fenwick(Vec<i32>);

impl Fenwick {
    fn new(len: usize) -> Self {
        Self(vec![0; len + 1])
    }

    /// Marks a slot as occupied. Only valid before [`build`](Self::build).
    fn occupy(&mut self, slot: usize) {
        self.0[slot + 1] = 1;
    }

    /// Turns the marks into the tree's totals in one pass.
    fn build(&mut self) {
        for position in 1..self.0.len() {
            let parent = position + (position & position.wrapping_neg());
            if parent < self.0.len() {
                let total = self.0[position];
                self.0[parent] += total;
            }
        }
    }

    fn add(&mut self, slot: usize, delta: i32) {
        let mut position = slot + 1;
        while position < self.0.len() {
            self.0[position] += delta;
            position += position & position.wrapping_neg();
        }
    }

    /// How many occupied slots come before `slot`.
    fn before(&self, slot: usize) -> u32 {
        let (mut position, mut total) = (slot, 0);
        while position > 0 {
            total += self.0[position];
            position &= position - 1;
        }
        total as u32
    }
}
