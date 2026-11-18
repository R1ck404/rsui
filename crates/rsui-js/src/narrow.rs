//! Which names a module reads from a namespace it imports, when that's all it does with it.
//!
//! `import * as icons from "icons"` asks for everything the package exports, and a package whose main file passes on
//! thousands of modules then loads every one of them, though the code may only ever say `icons.Star`. When every mention
//! of the namespace in the module is reading one name from it, nothing can tell a namespace that has only those names
//! from one that has them all: the namespace itself never gets anywhere else, so nothing can look at what else it has.
//! That's what's worked out here, from the module's whole tree (functions in it are compiled later, but their trees are
//! already there).
//!
//! Anything else gives up, so a module is only ever treated so when that can't be seen. These do:
//! - the namespace as a value anywhere (passed, returned, stored, spread, exported, `typeof`, `in`, compared),
//! - a name read with anything but a string written there (`icons[name]`),
//! - a call or tagged template through it (`icons.make()`, which hands the namespace over as `this`),
//! - writing, deleting or counting up a name of it, which is an error that's left to say so as it would,
//! - a declaration anywhere of something else by the same name, so no mention has to be told apart from the import,
//! - any mention of `eval`, which can read the module's variables by a name written as text,
//! - `with`, which modules can't have.

use crate::ast::{
    Class, ClassMember, Element, Expr, ExportName, ForInit, ForTarget, Function, Pattern, PatternItem, Property,
    PropertyKey, Stmt, UnaryOp,
};

/// The names a module with this body reads from the namespace it imports as `local`, each once, if reading names is
/// all it does with it. `None` if it does anything else.
pub(crate) fn namespace_reads(body: &[Stmt<'_>], local: &str) -> Option<Vec<String>> {
    let mut walk = Walk {
        local,
        names: Vec::new(),
        escaped: false,
    };
    walk.statements(body);
    (!walk.escaped).then_some(walk.names)
}

struct Walk<'n> {
    local: &'n str,
    names: Vec<String>,
    escaped: bool,
}

impl Walk<'_> {
    fn read(&mut self, name: String) {
        if !self.names.contains(&name) {
            self.names.push(name);
        }
    }

    /// Whether this is the namespace itself.
    fn is_local(&self, expr: &Expr<'_>) -> bool {
        matches!(expr, Expr::Ident(name, _) if *name == self.local)
    }

    /// Whether this is a name of the namespace, as `icons.Star` or `icons["Star"]`, however it's wrapped in a chain.
    fn is_name_of_local(&self, expr: &Expr<'_>) -> bool {
        match expr {
            Expr::Chain(inner) => self.is_name_of_local(inner),
            Expr::Member { object, .. } | Expr::Index { object, .. } => self.is_local(object),
            _ => false,
        }
    }

    /// A name that's declared or assigned, or mentioned: if it's the namespace's, or `eval`, that's the end of it.
    fn name(&mut self, name: &str) {
        if name == self.local || name == "eval" {
            self.escaped = true;
        }
    }

    fn statements(&mut self, list: &[Stmt<'_>]) {
        for statement in list {
            if self.escaped {
                return;
            }
            self.statement(statement);
        }
    }

    fn statement(&mut self, statement: &Stmt<'_>) {
        if self.escaped {
            return;
        }
        match statement {
            Stmt::Empty | Stmt::Line(_) | Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::Expr(value) => self.expr(value),
            Stmt::Throw(value) | Stmt::ExportDefault(value) => self.expr(value),
            Stmt::Var(_, list) => self.declarations(list),
            Stmt::Function(function) => self.function(function),
            Stmt::Class(class) => self.class(class),
            Stmt::Block(list) => self.statements(list),
            Stmt::If(test, then, otherwise) => {
                self.expr(test);
                self.statement(then);
                if let Some(otherwise) = otherwise {
                    self.statement(otherwise);
                }
            }
            Stmt::For(parts) => {
                match &parts.init {
                    Some(ForInit::Var(_, list)) => self.declarations(list),
                    Some(ForInit::Expr(value)) => self.expr(value),
                    None => {}
                }
                if let Some(test) = &parts.test {
                    self.expr(test);
                }
                if let Some(update) = &parts.update {
                    self.expr(update);
                }
                self.statement(parts.body);
            }
            Stmt::ForIn(parts) => {
                self.for_target(&parts.target);
                self.expr(&parts.object);
                self.statement(parts.body);
            }
            Stmt::ForOf(parts) => {
                self.for_target(&parts.target);
                self.expr(&parts.iterable);
                self.statement(parts.body);
            }
            // A module can't have one, but nothing here is sure what a name in one means.
            Stmt::With(..) => self.escaped = true,
            Stmt::While(test, body) | Stmt::DoWhile(body, test) => {
                self.expr(test);
                self.statement(body);
            }
            Stmt::Return(value) => {
                if let Some(value) = value {
                    self.expr(value);
                }
            }
            Stmt::Try(parts) => {
                self.statements(parts.block);
                if let Some(param) = &parts.param {
                    self.pattern(param);
                }
                if let Some(handler) = parts.handler {
                    self.statements(handler);
                }
                if let Some(finalizer) = parts.finalizer {
                    self.statements(finalizer);
                }
            }
            Stmt::Switch(value, cases) => {
                self.expr(value);
                for case in *cases {
                    if let Some(test) = &case.test {
                        self.expr(test);
                    }
                    self.statements(case.body);
                }
            }
            // A label is a name of another kind, which no variable is.
            Stmt::Labeled(_, body) => self.statement(body),
            Stmt::Field(key, value) => {
                self.key(key);
                if let Some(value) = value {
                    self.expr(value);
                }
            }
            // Another import by the same name couldn't have compiled, and the names these give are the other
            // module's.
            Stmt::Import(_) | Stmt::ExportFrom(_) => {}
            Stmt::Export(ExportName::List(list)) => {
                for (local, _) in *list {
                    self.name(local);
                }
            }
        }
    }

    fn declarations(&mut self, list: &[(Pattern<'_>, Option<Expr<'_>>)]) {
        for (target, value) in list {
            self.pattern(target);
            if let Some(value) = value {
                self.expr(value);
            }
        }
    }

    fn for_target(&mut self, target: &ForTarget<'_>) {
        match target {
            ForTarget::Declare(_, pattern) | ForTarget::Assign(pattern) => self.pattern(pattern),
        }
    }

    fn function(&mut self, function: &Function<'_>) {
        if let Some(name) = function.name {
            self.name(name);
        }
        for param in function.params {
            self.pattern_item(param);
        }
        if let Some(rest) = &function.rest {
            self.pattern(rest);
        }
        self.statements(function.body);
    }

    fn class(&mut self, class: &Class<'_>) {
        if let Some(name) = class.name {
            self.name(name);
        }
        if let Some(parent) = &class.parent {
            self.expr(parent);
        }
        self.function(class.constructor);
        for member in class.members {
            match member {
                ClassMember::Method { key, function, .. }
                | ClassMember::Getter { key, function, .. }
                | ClassMember::Setter { key, function, .. } => {
                    self.key(key);
                    self.function(function);
                }
                ClassMember::StaticInit(function) => self.function(function),
            }
        }
        if let Some(fields) = class.fields {
            self.function(fields);
        }
    }

    fn key(&mut self, key: &PropertyKey<'_>) {
        if let PropertyKey::Computed(value) = key {
            self.expr(value);
        }
    }

    fn pattern_item(&mut self, item: &PatternItem<'_>) {
        self.pattern(&item.target);
        if let Some(default) = &item.default {
            self.expr(default);
        }
    }

    /// Somewhere a value goes, declared or assigned.
    fn pattern(&mut self, pattern: &Pattern<'_>) {
        match pattern {
            Pattern::Ident(name, _) => self.name(name),
            Pattern::Expr(target) => {
                if self.is_name_of_local(target) {
                    self.escaped = true;
                } else {
                    self.expr(target);
                }
            }
            Pattern::Array { items, rest } => {
                for item in items.iter().flatten() {
                    self.pattern_item(item);
                }
                if let Some(rest) = rest {
                    self.pattern(rest);
                }
            }
            Pattern::Object { items, rest } => {
                for (key, item) in *items {
                    self.key(key);
                    self.pattern_item(item);
                }
                if let Some(rest) = rest {
                    self.pattern(rest);
                }
            }
        }
    }

    fn elements(&mut self, list: &[Element<'_>]) {
        for element in list {
            match element {
                Element::Plain(value) | Element::Spread(value) => self.expr(value),
            }
        }
    }

    fn expr(&mut self, expr: &Expr<'_>) {
        if self.escaped {
            return;
        }
        match expr {
            Expr::Number(_)
            | Expr::BigInt(..)
            | Expr::String(_)
            | Expr::Regex(_)
            | Expr::PrivateName(_)
            | Expr::ImportMeta
            | Expr::This
            | Expr::Null
            | Expr::True
            | Expr::False
            | Expr::NewTarget
            | Expr::SuperMember(_) => {}
            Expr::Ident(name, _) => self.name(name),
            Expr::Template(parts) => {
                for value in parts.values {
                    self.expr(value);
                }
            }
            Expr::TaggedTemplate(parts) => {
                if self.is_name_of_local(parts.tag) {
                    self.escaped = true;
                    return;
                }
                self.expr(parts.tag);
                for value in parts.values {
                    self.expr(value);
                }
            }
            Expr::ImportCall(value)
            | Expr::Chain(value)
            | Expr::SuperIndex(value)
            | Expr::Await(value) => self.expr(value),
            Expr::Array(items) => {
                for item in items.iter().flatten() {
                    match item {
                        Element::Plain(value) | Element::Spread(value) => self.expr(value),
                    }
                }
            }
            Expr::Object(properties) => {
                for property in *properties {
                    match property {
                        Property::Value(key, value) => {
                            self.key(key);
                            self.expr(value);
                        }
                        Property::Short(name, _) => self.name(name),
                        Property::Method(key, function)
                        | Property::Getter(key, function)
                        | Property::Setter(key, function) => {
                            self.key(key);
                            self.function(function);
                        }
                        Property::Spread(value) => self.expr(value),
                    }
                }
            }
            Expr::Function(function) => self.function(function),
            Expr::Class(class) => self.class(class),
            Expr::Unary(op, value) => {
                if *op == UnaryOp::Delete && self.is_name_of_local(value) {
                    self.escaped = true;
                } else {
                    self.expr(value);
                }
            }
            Expr::Update { target, .. } => {
                if self.is_name_of_local(target) {
                    self.escaped = true;
                } else {
                    self.expr(target);
                }
            }
            Expr::Binary(_, left, right) | Expr::Logical(_, left, right) => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Assign { target, value, .. } => {
                self.pattern(target);
                self.expr(value);
            }
            Expr::Conditional(test, then, otherwise) => {
                self.expr(test);
                self.expr(then);
                self.expr(otherwise);
            }
            Expr::Call { callee, args, .. } => {
                // `icons.make()` calls it with the namespace as `this`.
                if self.is_name_of_local(callee) {
                    self.escaped = true;
                    return;
                }
                self.expr(callee);
                self.elements(args);
            }
            Expr::New { callee, args, .. } => {
                self.expr(callee);
                self.elements(args);
            }
            Expr::Member { object, name, .. } => {
                if self.is_local(object) {
                    self.read((*name).to_owned());
                } else {
                    self.expr(object);
                }
            }
            Expr::Index { object, index, .. } => {
                if !self.is_local(object) {
                    self.expr(object);
                    self.expr(index);
                    return;
                }
                match index {
                    Expr::String(text) => match String::from_utf16(text) {
                        Ok(name) => self.read(name),
                        Err(_) => self.escaped = true,
                    },
                    _ => self.escaped = true,
                }
            }
            Expr::SuperCall(args) => self.elements(args),
            Expr::Sequence(list) => {
                for value in *list {
                    self.expr(value);
                }
            }
            Expr::Yield { value, .. } => {
                if let Some(value) = value {
                    self.expr(value);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::namespace_reads;

    /// What [`namespace_reads`] says of `icons` in this module.
    fn reads(source: &str) -> Option<Vec<String>> {
        let memory = crate::ast::AstMemory::new();
        let program = crate::parser::parse_module(source, &memory, 1 << 20).expect("it parses");
        namespace_reads(program.function.body, "icons")
    }

    fn names(list: &[&str]) -> Option<Vec<String>> {
        Some(list.iter().map(|name| (*name).to_owned()).collect())
    }

    #[test]
    fn reading_names_is_all_some_modules_do() {
        assert_eq!(reads("import * as icons from 'i';\nconsole.log(icons.Star);"), names(&["Star"]));
        assert_eq!(
            reads("import * as icons from 'i';\nconst a = icons.Star, b = icons['Moon'];\nexport function f() { return [icons.Star, icons?.Sun, new icons.Thing()]; }"),
            names(&["Star", "Moon", "Sun", "Thing"])
        );
        // Calling what's read hands that over, not the namespace.
        assert_eq!(reads("import * as icons from 'i';\n(0, icons.make)();\nicons.Star.call(null);"), names(&["make", "Star"]));
        // JSX is calls with what's read in them.
        assert_eq!(reads("import * as icons from 'i';\nexport const A = () => <icons.Star size={2} />;"), names(&["Star"]));
        assert_eq!(reads("import * as icons from 'i';\nexport const x = 1;"), names(&[]));
        // Something else that has a name of the same spelling is a property, not the namespace.
        assert_eq!(reads("import * as icons from 'i';\nconst o = { icons: 1 };\nconsole.log(o.icons, icons.A);"), names(&["A"]));
    }

    #[test]
    fn anything_else_gives_up() {
        for source in [
            "console.log(icons);",
            "console.log(Object.keys(icons));",
            "f({ ...icons });",
            "const o = { icons };",
            "export { icons };",
            "export default icons;",
            "console.log(typeof icons);",
            "console.log('Star' in icons);",
            "console.log(icons[name]);",
            "console.log(icons[0]);",
            "icons.make();",
            "icons?.make();",
            "icons.make?.();",
            "icons['make']();",
            "icons.tag`x`;",
            "icons.Star = 1;",
            "delete icons.Star;",
            "icons.count++;",
            "[icons.Star] = [1];",
            "function f(icons) { return icons.Star; }",
            "function f() { let icons = 1; return icons; }",
            "try {} catch (icons) {}",
            "const f = function icons() {};",
            "{ class icons {} }",
            "function f() { const { a, ...icons } = {}; }",
            "const f = (...icons) => 1;",
            "for (const icons of []) {}",
            "console.log(eval('icons'));",
            "const e = eval;",
        ] {
            let module = format!("import * as icons from 'i';\nconsole.log(icons.A);\n{source}");
            assert_eq!(reads(&module), None, "{source}");
        }
    }
}
