//! `Reflect`.

use crate::object::*;
use crate::text::AtomKind;
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

fn target_of(vm: &mut Vm, args: &Args, name: &str) -> Run<Value> {
    let target = vm.arg(args, 0);
    if !target.is_object() {
        return Err(vm.type_error(&format!("Reflect.{name} called on non-object")));
    }
    Ok(target)
}

fn apply(vm: &mut Vm, args: Args) -> Run<Value> {
    let (function, this, list) = (vm.arg(&args, 0), vm.arg(&args, 1), vm.arg(&args, 2));
    if !vm.is_callable(function) {
        return Err(vm.not_callable(function, false));
    }
    if !list.is_object() {
        return Err(vm.type_error("CreateListFromArrayLike called on non-object"));
    }
    let length = vm.length_of(list)?;
    let mut items = Vec::with_capacity(length);
    for index in 0..length {
        items.push(vm.get_index(list, index)?);
    }
    vm.call(function, this, &items)
}

fn construct(vm: &mut Vm, args: Args) -> Run<Value> {
    let (function, list) = (vm.arg(&args, 0), vm.arg(&args, 1));
    let target = if args.count > 2 {
        vm.arg(&args, 2)
    } else {
        function
    };
    if !vm.is_constructor(function) {
        return Err(vm.not_callable(function, true));
    }
    if !vm.is_constructor(target) {
        return Err(vm.not_callable(target, true));
    }
    if !list.is_object() {
        return Err(vm.type_error("CreateListFromArrayLike called on non-object"));
    }
    let length = vm.length_of(list)?;
    let mut items = Vec::with_capacity(length);
    for index in 0..length {
        items.push(vm.get_index(list, index)?);
    }
    vm.construct_with_target(function, &items, target)
}

fn get(vm: &mut Vm, args: Args) -> Run<Value> {
    let target = target_of(vm, &args, "get")?;
    let key = vm.arg(&args, 1);
    let atom = vm.to_atom(key)?;
    let receiver = if args.count > 2 {
        vm.arg(&args, 2)
    } else {
        target
    };
    vm.get_from(target.handle(), atom, receiver)
}

fn set(vm: &mut Vm, args: Args) -> Run<Value> {
    let target = target_of(vm, &args, "set")?;
    let key = vm.arg(&args, 1);
    let atom = vm.to_atom(key)?;
    let value = vm.arg(&args, 2);
    Ok(Value::bool(vm.set(target, atom, value).is_ok()))
}

fn has(vm: &mut Vm, args: Args) -> Run<Value> {
    let target = target_of(vm, &args, "has")?;
    let key = vm.arg(&args, 1);
    let atom = vm.to_atom(key)?;
    Ok(Value::bool(vm.has_property_checked(target.handle(), atom)?))
}

fn delete_property(vm: &mut Vm, args: Args) -> Run<Value> {
    let target = target_of(vm, &args, "deleteProperty")?;
    let key = vm.arg(&args, 1);
    let atom = vm.to_atom(key)?;
    Ok(Value::bool(vm.delete(target.handle(), atom).is_ok()))
}

fn own_keys(vm: &mut Vm, args: Args) -> Run<Value> {
    let target = target_of(vm, &args, "ownKeys")?;
    let keys = vm.own_keys_checked(target.handle())?;
    let mut items = Vec::with_capacity(keys.len());
    for (key, _) in keys {
        if vm.atoms.kind_of(key) != AtomKind::Hidden {
            items.push(vm.atom_value(key));
        }
    }
    Ok(vm.new_array(items))
}

fn get_prototype_of(vm: &mut Vm, args: Args) -> Run<Value> {
    let target = target_of(vm, &args, "getPrototypeOf")?;
    vm.prototype_checked(target)
}

fn set_prototype_of(vm: &mut Vm, args: Args) -> Run<Value> {
    let target = target_of(vm, &args, "setPrototypeOf")?;
    let proto = vm.arg(&args, 1);
    if !proto.is_object() && !proto.is_null() {
        return Err(vm.type_error("Object prototype may only be an Object or null"));
    }
    let handle = if proto.is_null() {
        NONE
    } else {
        proto.handle()
    };
    Ok(Value::bool(vm.change_proto(target.handle(), handle)))
}

fn define_property(vm: &mut Vm, args: Args) -> Run<Value> {
    let target = target_of(vm, &args, "defineProperty")?;
    let key = vm.arg(&args, 1);
    let atom = vm.to_atom(key)?;
    let descriptor = vm.arg(&args, 2);
    Ok(Value::bool(
        vm.define_from_descriptor(target.handle(), atom, descriptor)
            .is_ok(),
    ))
}

fn get_own_property_descriptor(vm: &mut Vm, args: Args) -> Run<Value> {
    target_of(vm, &args, "getOwnPropertyDescriptor")?;
    let object = vm.get_named(Value::object(vm.global), "Object")?;
    let method = vm.get_named(object, "getOwnPropertyDescriptor")?;
    let (target, key) = (vm.arg(&args, 0), vm.arg(&args, 1));
    vm.call(method, object, &[target, key])
}

fn is_extensible(vm: &mut Vm, args: Args) -> Run<Value> {
    let target = target_of(vm, &args, "isExtensible")?;
    Ok(Value::bool(
        vm.objects[target.index()].flags & NOT_EXTENSIBLE == 0,
    ))
}

fn prevent_extensions(vm: &mut Vm, args: Args) -> Run<Value> {
    let target = target_of(vm, &args, "preventExtensions")?;
    vm.objects[target.index()].flags |= NOT_EXTENSIBLE;
    Ok(Value::TRUE)
}

pub(super) fn install(vm: &mut Vm) {
    let reflect = vm.plain_proto();
    let atom = vm.intern_str("Reflect");
    vm.add_property(vm.global, atom, Value::object(reflect), HIDDEN);
    for (name, length, func) in [
        ("apply", 3, apply as NativeFn),
        ("construct", 2, construct),
        ("get", 2, get),
        ("set", 3, set),
        ("has", 2, has),
        ("deleteProperty", 2, delete_property),
        ("ownKeys", 1, own_keys),
        ("getPrototypeOf", 1, get_prototype_of),
        ("setPrototypeOf", 2, set_prototype_of),
        ("defineProperty", 3, define_property),
        ("getOwnPropertyDescriptor", 2, get_own_property_descriptor),
        ("isExtensible", 1, is_extensible),
        ("preventExtensions", 1, prevent_extensions),
    ] {
        vm.method(reflect, name, length, func);
    }
}
