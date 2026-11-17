//! A bounded, data-only certificate of the inputs read by simple :has recipes.
//! Completed receipts contain only primitives and WeakRefs, never page nodes,
//! lives, property bags, or maps. All ordinary matching remains the fallback.
use rsui_js::{Args, Atom, DataSurfaceError, Value, Vm};

const CAP: usize = 1024 * 1024;
const MAX_ROWS: usize = 8192;
const MAX_FIELDS: usize = 64;
const MAX_CELLS: usize = CAP / 8;
const ARRAY_OVERHEAD: usize = 128;
const WEAK_OVERHEAD: usize = 128;
const MAGIC: f64 = 731_026_051.0;

#[cfg(debug_assertions)]
thread_local! {
    static DEBUG_PHASE: std::cell::Cell<(&'static str, usize)> = const { std::cell::Cell::new(("entry", 0)) };
    static DEBUG_PRINTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

macro_rules! phase {
    ($name:expr, $row:expr) => {
        #[cfg(debug_assertions)]
        DEBUG_PHASE.with(|phase| phase.set(($name, $row)));
    };
}

#[cfg(debug_assertions)]
fn debug_failure(error: DataSurfaceError) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var("RSUI_RECEIPT_DEBUG").as_deref() == Ok("1")) {
        return;
    }
    DEBUG_PRINTS.with(|printed| {
        if printed.get() >= 16 {
            return;
        }
        printed.set(printed.get() + 1);
        DEBUG_PHASE.with(|phase| {
            let (phase, row) = phase.get();
            eprintln!("selector-receipt decline phase={phase} row={row} error={error:?}");
        });
    });
}

enum Cell {
    Primitive(Value),
    Owner { live: Value, weak: Option<Value> },
}

fn unsupported<T>() -> Result<T, DataSurfaceError> {
    Err(DataSurfaceError::Unsupported)
}

fn read(vm: &mut Vm, object: Value, name: &str) -> Result<Value, DataSurfaceError> {
    let name = vm.atom(name);
    Ok(vm
        .data_surface_lookup(object, name)?
        .map_or(Value::UNDEFINED, |part| part.value))
}

fn number(value: Value) -> Result<usize, DataSurfaceError> {
    if value.is_number() {
        let value = value.as_number();
        if value.is_finite() && value >= 0.0 && value.fract() == 0.0 && value <= MAX_ROWS as f64 {
            return Ok(value as usize);
        }
    }
    unsupported()
}

fn flag(value: Value) -> Result<Value, DataSurfaceError> {
    if value.is_undefined() || value.is_bool() {
        Ok(value)
    } else {
        unsupported()
    }
}

struct Field {
    value: Value,
    key: Atom,
    special: u8,
}

fn fields(vm: &mut Vm, value: Value) -> Result<Vec<Field>, DataSurfaceError> {
    let values = vm.data_surface_array(value, MAX_FIELDS)?;
    let mut fields: Vec<Field> = Vec::with_capacity(values.len());
    for value in values {
        vm.data_surface_primitive_cost(value)?;
        let Some(name) = vm.plain_text(value) else {
            return unsupported();
        };
        if !name.is_ascii() || name.is_empty() || name.len() > 128 {
            return unsupported();
        }
        let special = match name.as_str() {
            "@tag" => 1,
            "@state" => 2,
            _ if name.starts_with('@') => return unsupported(),
            _ => 0,
        };
        let key = vm.data_surface_key(value)?;
        if fields.iter().any(|field| field.key == key) {
            return unsupported();
        }
        fields.push(Field {
            value,
            key,
            special,
        });
    }
    Ok(fields)
}

/// Mirrors ownOf: without attributes it reads the given bag directly; with
/// attributes it reads a fresh ordinary object's own-enumerable spread result.
fn effective(
    vm: &mut Vm,
    given: Value,
    attributes: Value,
    key: Atom,
) -> Result<Value, DataSurfaceError> {
    if attributes.is_undefined() {
        return Ok(vm
            .data_surface_lookup(given, key)?
            .map_or(Value::UNDEFINED, |part| part.value));
    }
    for source in [attributes, given] {
        if let Some(part) = vm.data_surface_own(source, key)?
            && part.enumerable
        {
            return Ok(part.value);
        }
    }
    let prototype = vm.data_surface_object_prototype();
    Ok(vm
        .data_surface_lookup(prototype, key)?
        .map_or(Value::UNDEFINED, |part| part.value))
}

fn live_fields(
    vm: &mut Vm,
    live: Value,
    fields: &[Field],
    out: &mut Vec<Cell>,
) -> Result<(), DataSurfaceError> {
    if !vm.data_surface_plain(live) {
        return unsupported();
    }
    let given = read(vm, live, "given")?;
    let attributes = read(vm, live, "attributes")?;
    if !vm.data_surface_plain(given)
        || (!attributes.is_undefined() && !vm.data_surface_plain(attributes))
    {
        return unsupported();
    }
    for name in ["tag", "state"] {
        let name = vm.atom(name);
        vm.data_surface_lookup(live, name)?;
    }
    for name in ["id", "className"] {
        let name = vm.atom(name);
        effective(vm, given, attributes, name)?;
    }
    for field in fields {
        let value = match field.special {
            1 => {
                let name = vm.atom("tag");
                vm.data_surface_lookup(live, name)?
                    .map_or(Value::UNDEFINED, |part| part.value)
            }
            2 => {
                let name = vm.atom("state");
                vm.data_surface_lookup(live, name)?
                    .map_or(Value::UNDEFINED, |part| part.value)
            }
            _ => effective(vm, given, attributes, field.key)?,
        };
        vm.data_surface_primitive_cost(value)?;
        out.push(Cell::Primitive(value));
    }
    Ok(())
}

fn authoritative_owner(
    vm: &mut Vm,
    lives: Value,
    key: Value,
    live: Value,
) -> Result<Value, DataSurfaceError> {
    let Some(weak) = vm.data_surface_map_get(lives, key)? else {
        return unsupported();
    };
    if vm.data_surface_weak_target(weak)? != Some(live) {
        return unsupported();
    }
    Ok(weak)
}

fn collect(
    vm: &mut Vm,
    by_key: Value,
    fields: &[Field],
    recipe: Value,
    lives: Value,
    hidden: Value,
) -> Result<Vec<Cell>, DataSurfaceError> {
    if !recipe.is_number()
        || !recipe.as_number().is_finite()
        || recipe.as_number() < 0.0
        || recipe.as_number().fract() != 0.0
        || recipe.as_number() >= (1u64 << 53) as f64
    {
        return unsupported();
    }
    phase!("maps", 0);
    let entries = vm.data_surface_map(by_key, MAX_ROWS)?;
    let roster = vm.data_surface_map(lives, MAX_ROWS)?;
    let not_shown = vm.data_surface_map(hidden, MAX_ROWS)?;
    let cells = 4usize
        .checked_add(fields.len())
        .and_then(|n| n.checked_add(entries.len().checked_mul(9 + fields.len())?))
        .and_then(|n| n.checked_add(2 + roster.len().checked_mul(2 + fields.len())?))
        .and_then(|n| n.checked_add(2 + not_shown.len().checked_mul(7)?))
        .ok_or(DataSurfaceError::LimitExceeded)?;
    if cells > MAX_CELLS {
        return Err(DataSurfaceError::LimitExceeded);
    }
    let mut out = Vec::with_capacity(cells);
    for value in [
        Value::number(MAGIC),
        recipe,
        Value::number(fields.len() as f64),
        Value::number(entries.len() as f64),
    ] {
        out.push(Cell::Primitive(value));
    }
    for field in fields {
        out.push(Cell::Primitive(field.value));
    }
    // Anonymous fragment/root parents get a stable first-seen partition number.
    // This temporary list is discarded before returning; no parent is retained.
    let mut anonymous = Vec::new();
    for (_row, &(key, node)) in entries.iter().enumerate() {
        phase!("node plain/key", _row);
        if !key.is_string() || !vm.data_surface_plain(node) {
            return unsupported();
        }
        let node_key = read(vm, node, "key")?;
        if !vm.data_surface_same_primitive(key, node_key) {
            return unsupported();
        }
        phase!("entry array", _row);
        let entry = read(vm, node, "entry")?;
        let entry = vm.data_surface_array(entry, 14)?;
        if entry.len() < 12 || !vm.data_surface_same_primitive(key, entry[0]) {
            return unsupported();
        }
        phase!("authoritative live", _row);
        let live = read(vm, node, "live")?;
        let authoritative = authoritative_owner(vm, lives, key, live)?;
        if entry.len() == 12
            && (vm.data_surface_map_get(hidden, key)? != Some(live)
                || !vm.data_surface_same_primitive(key, entry[11]))
        {
            return unsupported();
        }
        phase!("live plain", _row);
        if !vm.data_surface_plain(live) {
            return unsupported();
        }
        let given = read(vm, live, "given")?;
        let attributes = read(vm, live, "attributes")?;
        phase!("given bag", _row);
        if !vm.data_surface_plain(given) {
            return unsupported();
        }
        phase!("attributes bag", _row);
        if !attributes.is_undefined() && !vm.data_surface_plain(attributes) {
            return unsupported();
        }
        // These slots are read by ownOf even when a recipe never consumes them.
        // Certify data-only lookup, but do not retain irrelevant input values.
        phase!("always-read surface", _row);
        for name in ["tag", "state"] {
            let name = vm.atom(name);
            vm.data_surface_lookup(live, name)?;
        }
        for name in ["id", "className"] {
            let name = vm.atom(name);
            effective(vm, given, attributes, name)?;
        }
        phase!("parent plain", _row);
        let parent = read(vm, node, "parent")?;
        if !vm.data_surface_plain(parent) {
            return unsupported();
        }
        let parent_key = read(vm, parent, "key")?;
        phase!("parent key/partition", _row);
        let parent_token = if parent_key.is_undefined() {
            if !read(vm, parent, "parent")?.is_null() {
                return unsupported();
            }
            let at = anonymous
                .iter()
                .position(|&one| one == parent)
                .unwrap_or_else(|| {
                    anonymous.push(parent);
                    anonymous.len() - 1
                });
            Value::number(at as f64)
        } else {
            if !parent_key.is_string()
                || vm.data_surface_map_get(by_key, parent_key)? != Some(parent)
            {
                return unsupported();
            }
            parent_key
        };
        phase!("parent child/index", _row);
        let index = number(read(vm, node, "index")?)?;
        let parent_children = read(vm, parent, "children")?;
        let parent_children = vm.data_surface_array(parent_children, MAX_ROWS)?;
        if parent_children.get(index).copied() != Some(node) {
            return unsupported();
        }
        phase!("child membership", _row);
        let children = read(vm, node, "children")?;
        let children = vm.data_surface_array(children, MAX_ROWS)?;
        for (index, &child) in children.iter().enumerate() {
            if !vm.data_surface_plain(child) {
                return unsupported();
            }
            let child_key = read(vm, child, "key")?;
            if !child_key.is_string()
                || vm.data_surface_map_get(by_key, child_key)? != Some(child)
                || read(vm, child, "parent")? != node
                || number(read(vm, child, "index")?)? != index
            {
                return unsupported();
            }
        }
        phase!("live weak", _row);
        let weak = read(vm, live, "weak")?;
        let weak = if weak.is_undefined() {
            None
        } else {
            if weak != authoritative || vm.data_surface_weak_target(weak)? != Some(live) {
                return unsupported();
            }
            Some(weak)
        };
        out.push(Cell::Primitive(key));
        out.push(Cell::Owner { live, weak });
        for value in [
            Value::bool(weak.is_some()),
            parent_token,
            Value::number(index as f64),
            Value::number(children.len() as f64),
            Value::number(parent_children.len() as f64),
            flag(read(vm, parent, "detached")?)?,
            flag(read(vm, node, "detached")?)?,
        ] {
            out.push(Cell::Primitive(value));
        }
        phase!("primitive fields", _row);
        for field in fields {
            let value = match field.special {
                1 => {
                    let name = vm.atom("tag");
                    vm.data_surface_lookup(live, name)?
                        .map_or(Value::UNDEFINED, |part| part.value)
                }
                2 => {
                    let name = vm.atom("state");
                    vm.data_surface_lookup(live, name)?
                        .map_or(Value::UNDEFINED, |part| part.value)
                }
                _ => effective(vm, given, attributes, field.key)?,
            };
            vm.data_surface_primitive_cost(value)?;
            out.push(Cell::Primitive(value));
        }
    }
    // The current authoritative roster is distinct from the old snapshot graph.
    // Its entries/WeakRef association are validated even for lives not in byKey.
    out.push(Cell::Primitive(Value::number(1.0)));
    out.push(Cell::Primitive(Value::number(roster.len() as f64)));
    for (_row, (key, weak)) in roster.into_iter().enumerate() {
        phase!("authoritative roster fields", _row);
        if !key.is_string() {
            return unsupported();
        }
        let Some(live) = vm.data_surface_weak_target(weak)? else {
            return unsupported();
        };
        out.push(Cell::Primitive(key));
        out.push(Cell::Owner {
            live,
            weak: Some(weak),
        });
        live_fields(vm, live, fields, &mut out)?;
    }
    out.push(Cell::Primitive(Value::number(2.0)));
    out.push(Cell::Primitive(Value::number(not_shown.len() as f64)));
    for (_row, (key, live)) in not_shown.into_iter().enumerate() {
        phase!("not-shown roster", _row);
        if !key.is_string() || !vm.data_surface_plain(live) {
            return unsupported();
        }
        let weak = authoritative_owner(vm, lives, key, live)?;
        out.push(Cell::Primitive(key));
        out.push(Cell::Owner {
            live,
            weak: Some(weak),
        });
        let parent = read(vm, live, "parentKey")?;
        if !parent.is_undefined() && !parent.is_string() {
            return unsupported();
        }
        let text = read(vm, live, "text")?;
        if !text.is_undefined() && !text.is_string() {
            return unsupported();
        }
        for value in [
            parent,
            flag(read(vm, live, "listPart")?)?,
            text,
            flag(read(vm, live, "notShown")?)?,
            flag(read(vm, live, "detached")?)?,
        ] {
            out.push(Cell::Primitive(value));
        }
    }
    Ok(out)
}

fn capture_inner(
    vm: &mut Vm,
    by_key: Value,
    names: Value,
    recipe: Value,
    lives: Value,
    hidden: Value,
) -> Result<Value, DataSurfaceError> {
    phase!("intrinsics", 0);
    if !vm.data_surface_intrinsics_kept() {
        return unsupported();
    }
    phase!("fields", 0);
    let fields = fields(vm, names)?;
    let inputs = collect(vm, by_key, &fields, recipe, lives, hidden)?;
    // Complete preflight happens before any primitive backing is copied or the
    // receipt array is allocated. Charge array and WeakRef overhead generously.
    phase!("budget", 0);
    let mut cost = ARRAY_OVERHEAD;
    for cell in &inputs {
        cost = cost
            .checked_add(match cell {
                Cell::Primitive(value) => vm.data_surface_primitive_cost(*value)?,
                Cell::Owner { .. } => 8 + WEAK_OVERHEAD,
            })
            .ok_or(DataSurfaceError::LimitExceeded)?;
        if cost > CAP {
            return Err(DataSurfaceError::LimitExceeded);
        }
    }
    let mut remaining = CAP - ARRAY_OVERHEAD;
    let mut values = Vec::with_capacity(inputs.len());
    for cell in inputs {
        values.push(match cell {
            Cell::Primitive(value) => vm.data_surface_copy_primitive(value, &mut remaining)?,
            Cell::Owner { live, weak } => {
                remaining -= 8 + WEAK_OVERHEAD;
                match weak {
                    Some(weak) => weak,
                    None => vm.data_surface_weak_ref(live)?,
                }
            }
        });
    }
    if !vm.data_surface_intrinsics_kept() {
        return unsupported();
    }
    Ok(vm.new_array(values))
}

pub(crate) fn capture(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (by_key, names, recipe) = (args.get(vm, 0), args.get(vm, 1), args.get(vm, 2));
    let (lives, hidden) = (args.get(vm, 3), args.get(vm, 4));
    Ok(
        match capture_inner(vm, by_key, names, recipe, lives, hidden) {
            Ok(receipt) => receipt,
            Err(_error) => {
                #[cfg(debug_assertions)]
                debug_failure(_error);
                Value::UNDEFINED
            }
        },
    )
}

pub(crate) fn kept(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (receipt, by_key, names, recipe) = (
        args.get(vm, 0),
        args.get(vm, 1),
        args.get(vm, 2),
        args.get(vm, 3),
    );
    let (lives, hidden) = (args.get(vm, 4), args.get(vm, 5));
    let matched = (|| -> Result<bool, DataSurfaceError> {
        if !vm.data_surface_intrinsics_kept() {
            return Ok(false);
        }
        let expected = vm.data_surface_array(receipt, MAX_CELLS)?;
        let fields = fields(vm, names)?;
        let inputs = collect(vm, by_key, &fields, recipe, lives, hidden)?;
        if inputs.len() != expected.len() {
            return Ok(false);
        }
        for (input, expected) in inputs.into_iter().zip(expected) {
            match input {
                Cell::Primitive(value) if !vm.data_surface_same_primitive(value, expected) => {
                    return Ok(false);
                }
                Cell::Primitive(_) => {}
                Cell::Owner { live, weak } => {
                    if weak.is_some_and(|weak| weak != expected)
                        || vm.data_surface_weak_target(expected)? != Some(live)
                    {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(vm.data_surface_intrinsics_kept())
    })()
    .unwrap_or(false);
    Ok(Value::bool(matched))
}

fn program_string(vm: &Vm, value: Value, optional: bool) -> Result<(), DataSurfaceError> {
    if optional && value.is_nullish() {
        return Ok(());
    }
    if !value.is_string() {
        return unsupported();
    }
    vm.data_surface_primitive_cost(value).map(|_| ())
}

fn programs(
    vm: &mut Vm,
    selectors: Value,
    depth: usize,
    budget: &mut usize,
) -> Result<(), DataSurfaceError> {
    if depth > 8 {
        return Err(DataSurfaceError::LimitExceeded);
    }
    for selector in vm.data_surface_array(selectors, 256)? {
        if selector.is_null() {
            continue;
        }
        for step in vm.data_surface_array(selector, 64)? {
            *budget = budget
                .checked_sub(1)
                .ok_or(DataSurfaceError::LimitExceeded)?;
            if !vm.data_surface_plain(step) {
                return unsupported();
            }
            for name in ["tag", "id", "part", "link"] {
                let value = read(vm, step, name)?;
                program_string(vm, value, true)?;
            }
            for name in ["unknown", "self"] {
                flag(read(vm, step, name)?)?;
            }
            let classes = read(vm, step, "classes")?;
            for class in vm.data_surface_array(classes, 256)? {
                program_string(vm, class, false)?;
            }
            let attrs = read(vm, step, "attrs")?;
            for attr in vm.data_surface_array(attrs, 128)? {
                *budget = budget
                    .checked_sub(1)
                    .ok_or(DataSurfaceError::LimitExceeded)?;
                if !vm.data_surface_plain(attr) {
                    return unsupported();
                }
                for name in ["name", "op", "value"] {
                    let value = read(vm, attr, name)?;
                    program_string(vm, value, name != "name")?;
                }
            }
            let pseudos = read(vm, step, "pseudo")?;
            for pseudo in vm.data_surface_array(pseudos, 128)? {
                *budget = budget
                    .checked_sub(1)
                    .ok_or(DataSurfaceError::LimitExceeded)?;
                if !vm.data_surface_plain(pseudo) {
                    return unsupported();
                }
                for name in ["name", "arg"] {
                    let value = read(vm, pseudo, name)?;
                    program_string(vm, value, name == "arg")?;
                }
                for name in ["list", "of"] {
                    let nested = read(vm, pseudo, name)?;
                    if !nested.is_nullish() {
                        programs(vm, nested, depth + 1, budget)?;
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn safe_programs(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let watched = args.get(vm, 0);
    let safe = (|| -> Result<(), DataSurfaceError> {
        if !vm.data_surface_intrinsics_kept() {
            return unsupported();
        }
        let mut budget = 8192usize;
        for live in vm.data_surface_set(watched, MAX_ROWS)? {
            if !vm.data_surface_plain(live) {
                return unsupported();
            }
            let drawn = read(vm, live, "hasDrawn")?;
            for (key, one) in vm.data_surface_map(drawn, 1024)? {
                program_string(vm, key, false)?;
                if !vm.data_surface_plain(one) {
                    return unsupported();
                }
                let selectors = read(vm, one, "selectors")?;
                programs(vm, selectors, 0, &mut budget)?;
            }
        }
        Ok(())
    })()
    .is_ok();
    Ok(Value::bool(safe))
}

pub(crate) fn safe_intrinsics(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    Ok(Value::bool(vm.data_surface_intrinsics_kept()))
}

#[cfg(test)]
#[path = "selector_receipt_tests.rs"]
mod tests;
