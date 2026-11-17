use std::fmt::Write;

use rsui_core::{Element, GENERATED_KEYS_FROM, Key, Node, Patch, PropValue, Role};

/// Props that only affect how something looks. An outline is for reading what a UI says
/// and does, and these would drown that out. A full snapshot still has them.
const STYLING_PROPS: [&str; 72] = [
    "scale",
    "scale_x",
    "scale_y",
    "rotate",
    "transform",
    "transform_origin",
    "justify_items",
    "grid_flow",
    "align_content",
    "on_line",
    "margin_top",
    "margin_right",
    "margin_bottom",
    "margin_left",
    "position",
    "top",
    "right",
    "bottom",
    "left",
    "translate_x",
    "translate_y",
    "display",
    "columns",
    "rows",
    "grid_column",
    "grid_row",
    "column_gap",
    "row_gap",
    "ellipsis",
    "font_family",
    "font_style",
    "no_wrap",
    "opacity",
    "shadow_blur",
    "shadow_color",
    "shadow_x",
    "shadow_y",
    "stroke_width",
    "text_align",
    "underline",
    "align",
    "align_self",
    "background",
    "basis",
    "border_color",
    "border_width",
    "corner_radius",
    "direction",
    "font_size",
    "font_weight",
    "gap",
    "grow",
    "height",
    "height_percent",
    "hover_background",
    "justify",
    "max_height",
    "max_width",
    "min_height",
    "min_width",
    "padding_bottom",
    "padding_left",
    "padding_right",
    "padding_top",
    "pressed_background",
    "row_height",
    "shrink",
    "text_color",
    "first_built",
    "view_rows",
    "width",
    "width_percent",
];

/// The tree as compact text, far smaller than its JSON snapshot, with one line per
/// node. Styling is left out, as it says nothing about what the UI does:
///
/// ```text
/// window "Todos" #app @/
///   button "Add" #add [click] @0
///   list #rows count=2 @1
///     list_item "Buy milk" #7 [click,toggle] @1/0
/// ```
///
/// A line holds the role, `"name"`, `#key`, `prop=value`s, `[actions]` and `@path`, and
/// leaves out whatever is empty. Text from the app is always quoted and escaped, so it
/// can't pose as extra lines, and a reader can tell it apart from the outline itself.
pub fn outline(root: &Node) -> String {
    outline_at(root, &[], None)
}

/// [`outline`] for the subtree at `node`, which is at `path` in its tree. With `depth`,
/// only that many levels below `node` are written, and a note says what's left out.
pub fn outline_at(node: &Node, path: &[u32], depth: Option<usize>) -> String {
    let mut out = String::new();
    let mut path = path.to_vec();
    write_tree(node, &mut path, 0, depth, &mut out);
    out
}

/// Whether this is what a component that renders nothing leaves in the tree, which nothing can reach, and says nothing.
fn is_nothing(node: &Element) -> bool {
    node.role == Role::Group
        && node.name.is_empty()
        && node.key.is_none()
        && node.children.is_empty()
        && node.actions.is_empty()
        && node.props.get("inert") == Some(&PropValue::Bool(true))
        && node.props.get("overlay").is_some()
}

fn write_tree(
    node: &Node,
    path: &mut Vec<u32>,
    level: usize,
    depth: Option<usize>,
    out: &mut String,
) {
    if is_nothing(node) {
        return;
    }
    write_indent(level, out);
    write_line(node, path, out);
    out.push('\n');
    if node.children.is_empty() {
        return;
    }
    if depth == Some(0) {
        write_indent(level + 1, out);
        let _ = writeln!(out, "(+{} children)", node.children.len());
        return;
    }
    for (index, child) in node.children.iter().enumerate() {
        path.push(index as u32);
        write_tree(child, path, level + 1, depth.map(|left| left - 1), out);
        path.pop();
    }
}

fn write_indent(level: usize, out: &mut String) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

/// One node's line, without its children.
pub(crate) fn write_line(node: &Element, path: &[u32], out: &mut String) {
    out.push_str(node.role.name());
    if !node.name.is_empty() {
        out.push(' ');
        write_quoted(&node.name, out);
    }
    if let Some(key) = &node.key {
        if !matches!(key, Key::Int(number) if *number > GENERATED_KEYS_FROM) {
            out.push_str(" #");
            write_key(key, out);
        }
    }
    for (name, value) in node.props.iter() {
        if STYLING_PROPS.contains(&name.as_str()) {
            continue;
        }
        out.push(' ');
        out.push_str(name.as_str());
        out.push('=');
        write_value(value, out);
    }
    if !node.actions.is_empty() {
        out.push_str(" [");
        for (index, action) in node.actions.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(action.name());
        }
        out.push(']');
    }
    out.push_str(" @");
    write_path(path, out);
}

fn write_quoted(text: &str, out: &mut String) {
    out.push_str(&serde_json::to_string(text).expect("a string always serializes to JSON"));
}

fn write_key(key: &Key, out: &mut String) {
    let text = key.to_string();
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if plain {
        out.push_str(&text);
    } else {
        write_quoted(&text, out);
    }
}

fn write_value(value: &PropValue, out: &mut String) {
    let _ = match value {
        PropValue::Bool(value) => write!(out, "{value}"),
        PropValue::Int(value) => write!(out, "{value}"),
        PropValue::Float(value) => write!(out, "{value}"),
        PropValue::Str(value) => {
            write_quoted(value, out);
            Ok(())
        }
    };
}

fn write_path(path: &[u32], out: &mut String) {
    if path.is_empty() {
        out.push('/');
    }
    for (index, part) in path.iter().enumerate() {
        if index > 0 {
            out.push('/');
        }
        let _ = write!(out, "{part}");
    }
}

/// A path as written in an outline: `/` for the root, or indices like `1/0`.
pub fn format_path(path: &[u32]) -> String {
    let mut out = String::new();
    write_path(path, &mut out);
    out
}

/// Reads a path written by [`format_path`], with or without its `@`.
pub fn parse_path(text: &str) -> Option<Vec<u32>> {
    let text = text.strip_prefix('@').unwrap_or(text);
    if text == "/" || text.is_empty() {
        return Some(Vec::new());
    }
    text.split('/').map(|part| part.parse().ok()).collect()
}

/// Whether a change is only to how something looks, which an [`outline`] leaves out. That's worth leaving out of a list of
/// what's changed for the same reason, especially as a color only means anything as a picture.
pub fn only_changes_styling(patch: &Patch) -> bool {
    match patch {
        Patch::SetProp { name, .. } | Patch::RemoveProp { name, .. } => {
            STYLING_PROPS.contains(&name.as_str())
        }
        Patch::Insert { node, .. } | Patch::Replace { node, .. } => is_nothing(node),
        _ => false,
    }
}

/// One change as a line of text, in the same terms as an [`outline`].
pub fn describe_patch(patch: &Patch) -> String {
    let mut out = String::new();
    let new_node = |node: &Node, path: &[u32], out: &mut String| {
        write_line(node, path, out);
        if !node.children.is_empty() {
            let _ = write!(out, " (+{} children)", node.children.len());
        }
    };
    let child_path = |parent: &[u32], index: u32| {
        let mut path = parent.to_vec();
        path.push(index);
        path
    };
    match patch {
        Patch::Replace { path, node } => {
            out.push_str("replace ");
            new_node(node, path, &mut out);
        }
        Patch::SetName { path, name } => {
            out.push_str("set_name @");
            write_path(path, &mut out);
            out.push(' ');
            write_quoted(name, &mut out);
        }
        Patch::SetProp { path, name, value } => {
            out.push_str("set_prop @");
            write_path(path, &mut out);
            let _ = write!(out, " {}=", name.as_str());
            write_value(value, &mut out);
        }
        Patch::RemoveProp { path, name } => {
            out.push_str("remove_prop @");
            write_path(path, &mut out);
            let _ = write!(out, " {}", name.as_str());
        }
        Patch::SetActions { path, actions } => {
            out.push_str("set_actions @");
            write_path(path, &mut out);
            let names: Vec<&str> = actions.iter().map(|action| action.name()).collect();
            let _ = write!(out, " [{}]", names.join(","));
        }
        Patch::Insert {
            parent,
            index,
            node,
        } => {
            out.push_str("insert ");
            new_node(node, &child_path(parent, *index), &mut out);
        }
        Patch::Remove { parent, index } => {
            out.push_str("remove @");
            write_path(&child_path(parent, *index), &mut out);
        }
        Patch::Move { parent, from, to } => {
            out.push_str("move @");
            write_path(&child_path(parent, *from), &mut out);
            out.push_str(" to @");
            write_path(&child_path(parent, *to), &mut out);
        }
        Patch::Reorder {
            parent,
            start,
            order,
        } => {
            out.push_str("reorder @");
            write_path(parent, &mut out);
            let _ = write!(out, " {} children from {start}", order.len());
        }
    }
    out
}
