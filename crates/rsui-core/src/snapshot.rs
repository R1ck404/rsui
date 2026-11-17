use crate::element::{ActionKind, Element, Key, PropValue, Role};
use crate::symbol::Symbol;

/// The tree as a JSON value.
pub fn snapshot(root: &Element) -> serde_json::Value {
    serde_json::to_value(root).expect("an element always serializes to JSON")
}

/// The tree as a JSON string. Equal trees always produce identical bytes.
pub fn snapshot_json(root: &Element) -> String {
    let mut out = String::new();
    snapshot_into(root, &mut out);
    out
}

/// Appends [`snapshot_json`]'s output to `out`. Reusing a buffer avoids growing
/// a new one for each snapshot.
///
/// This writes JSON directly because snapshots are taken often and are large.
/// A test keeps the output identical to the `Serialize` implementations.
pub fn snapshot_into(root: &Element, out: &mut String) {
    out.push('{');
    if let Some(key) = &root.key {
        out.push_str("\"key\":");
        match key {
            Key::Int(key) => out.push_str(itoa::Buffer::new().format(*key)),
            Key::Sym(key) => write_string(key.as_str(), out),
            Key::Str(key) => write_string(key, out),
        }
        out.push(',');
    }
    out.push_str("\"role\":");
    write_role(root.role, out);
    if !root.name.is_empty() {
        out.push_str(",\"name\":");
        write_string(&root.name, out);
    }
    if !root.props.is_empty() {
        out.push_str(",\"props\":{");
        for (index, (name, value)) in root.props.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            write_string(name.as_str(), out);
            out.push(':');
            write_value(value, out);
        }
        out.push('}');
    }
    if !root.actions.is_empty() {
        out.push_str(",\"actions\":[");
        for (index, action) in root.actions.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            write_action(action, out);
        }
        out.push(']');
    }
    if !root.children.is_empty() {
        out.push_str(",\"children\":[");
        for (index, child) in root.children.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            snapshot_into(child, out);
        }
        out.push(']');
    }
    out.push('}');
}

fn write_role(role: Role, out: &mut String) {
    out.push_str(match role {
        Role::Window => "\"window\"",
        Role::Group => "\"group\"",
        Role::Text => "\"text\"",
        Role::Button => "\"button\"",
        Role::Link => "\"link\"",
        Role::TextInput => "\"text_input\"",
        Role::Checkbox => "\"checkbox\"",
        Role::List => "\"list\"",
        Role::ListItem => "\"list_item\"",
        Role::Image => "\"image\"",
        Role::Dialog => "\"dialog\"",
        Role::Menu => "\"menu\"",
        Role::MenuItem => "\"menu_item\"",
        Role::Custom(role) => return write_custom(role.name(), out),
    });
}

fn write_action(action: ActionKind, out: &mut String) {
    out.push_str(match action {
        ActionKind::Click => "\"click\"",
        ActionKind::Focus => "\"focus\"",
        ActionKind::Toggle => "\"toggle\"",
        ActionKind::SetValue => "\"set_value\"",
        ActionKind::Scroll => "\"scroll\"",
        ActionKind::KeyPress => "\"key_press\"",
        ActionKind::Resize => "\"resize\"",
        ActionKind::Dismiss => "\"dismiss\"",
        ActionKind::Custom(name) => return write_custom(name, out),
    });
}

fn write_custom(name: Symbol, out: &mut String) {
    out.push_str("{\"custom\":");
    write_string(name.as_str(), out);
    out.push('}');
}

fn write_value(value: &PropValue, out: &mut String) {
    match value {
        PropValue::Bool(true) => out.push_str("true"),
        PropValue::Bool(false) => out.push_str("false"),
        PropValue::Int(value) => out.push_str(itoa::Buffer::new().format(*value)),
        PropValue::Float(value) => write_float(*value, out),
        PropValue::Str(value) => write_string(value, out),
    }
}

fn write_float(value: f64, out: &mut String) {
    // Longer than any JSON number serde_json writes for an f64.
    let mut buffer = [0; 40];
    let mut unused = &mut buffer[..];
    serde_json::to_writer(&mut unused, &value).expect("a float fits in the buffer");
    let written = 40 - unused.len();
    out.push_str(std::str::from_utf8(&buffer[..written]).expect("JSON numbers are ASCII"));
}

fn write_string(text: &str, out: &mut String) {
    if text
        .bytes()
        .any(|byte| byte < 0x20 || byte == b'"' || byte == b'\\')
    {
        out.push_str(&serde_json::to_string(text).expect("a string always serializes to JSON"));
    } else {
        out.push('"');
        out.push_str(text);
        out.push('"');
    }
}
