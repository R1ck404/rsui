//! A small todo app that an agent can operate over the Model Context Protocol.
//!
//! Run it with `cargo run -p rsui-agent --example todo`, or point an MCP client at the
//! built program. It reads requests from stdin and writes responses to stdout.

use std::time::Duration;

use rsui_agent::Session;
use rsui_core::{ActionKind, Element, Node, PropValue, Role, Text, text};
use rsui_headless::Headless;
use rsui_runtime::{Cx, State};

#[derive(Clone, PartialEq)]
struct Todo {
    id: u64,
    label: Text,
}

#[derive(PartialEq)]
struct RowProps {
    todo: Todo,
    todos: State<Vec<Todo>>,
}

fn row(cx: &mut Cx, props: &RowProps) -> Element {
    let done = cx.state(|| false);
    let id = props.todo.id;
    let todos = props.todos;
    let checkbox = cx.on(
        Element::new(Role::Checkbox)
            .key("done")
            .name(props.todo.label.clone())
            .prop("checked", cx.get(done)),
        ActionKind::Toggle,
        move |cx, _| cx.update(done, |done| *done = !*done),
    );
    let delete = cx.on(
        Element::new(Role::Button).key("delete").name("Delete"),
        ActionKind::Click,
        move |cx, _| cx.update(todos, |todos| todos.retain(|todo| todo.id != id)),
    );
    Element::new(Role::ListItem).child(checkbox).child(delete)
}

fn app(cx: &mut Cx, _: &()) -> Element {
    let todos = cx.state(Vec::<Todo>::new);
    let next = cx.state(|| 1u64);
    let draft = cx.state(Text::new);
    let loading = cx.state(|| false);

    let input = cx.on(
        Element::new(Role::TextInput)
            .key("draft")
            .name("New todo")
            .prop("value", cx.get(draft)),
        ActionKind::SetValue,
        move |cx, input| match input {
            Some(PropValue::Str(text)) => cx.set(draft, (**text).clone()),
            _ => cx.reject("set_value needs the text of the todo"),
        },
    );
    let add = cx.on(
        Element::new(Role::Button).key("add").name("Add"),
        ActionKind::Click,
        move |cx, _| {
            let label = cx.get(draft);
            if label.trim().is_empty() {
                cx.reject("type something first: use set_value on the text input");
                return;
            }
            let id = cx.get(next);
            cx.set(next, id + 1);
            cx.update(todos, |todos| todos.push(Todo { id, label }));
            cx.set(draft, Text::new());
        },
    );
    let load = cx.on(
        Element::new(Role::Button)
            .key("load")
            .name(if cx.get(loading) {
                "Loading..."
            } else {
                "Load samples"
            }),
        ActionKind::Click,
        move |cx, _| {
            cx.set(loading, true);
            cx.after(Duration::from_millis(50), move |cx| {
                cx.update(todos, |todos| {
                    let start = todos.iter().map(|todo| todo.id).max().unwrap_or(0) + 1000;
                    for (offset, label) in ["Water the plants", "Call the bank"].iter().enumerate()
                    {
                        todos.push(Todo {
                            id: start + offset as u64,
                            label: (*label).into(),
                        });
                    }
                });
                cx.set(loading, false);
            });
        },
    );

    let current = cx.get(todos);
    let remaining = current.len();
    let rows: Vec<Node> = current
        .into_iter()
        .map(|todo| cx.keyed(todo.id, row, RowProps { todo, todos }))
        .collect();
    Element::new(Role::Window)
        .key("app")
        .name(text!("Todos ({remaining})"))
        .child(input)
        .child(add)
        .child(load)
        .child(Element::new(Role::List).key("todos").children(rows))
}

fn main() -> std::io::Result<()> {
    Session::new(Headless::new(app, ()), "rsui-todo").serve_stdio()
}
