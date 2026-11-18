//! Turns the tree the parser made into instructions. Variables that no inner function can see live in registers,
//! and the ones that can be seen live in cells, which is what makes most variables free to use.

use crate::ast::*;
use crate::builtins::disposal;
use crate::lexer::SyntaxError;
use crate::op::*;
use crate::text::{self, Atom};
use crate::value::Value;
use crate::vm::Vm;

type Compiled<T> = Result<T, SyntaxError>;

/// Diagnostic control for comparing the initialized-const pass against the original
/// cell representation. Read once per process, including by the packed-code cache.
pub(crate) fn const_value_captures_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED
        .get_or_init(|| std::env::var_os("RSUI_CONST_CAPTURES").is_none_or(|value| value != "0"))
}

/// Where a file's source code is, for [`KeptAst`].
pub(crate) enum KeptSource {
    /// Source that's in the program, as the bundled files are.
    Static(&'static str),
    Owned(String),
}

/// A file's source and what the parser made of it, kept for as long as any of its functions is waiting to be compiled,
/// which is until it's first called. If nothing is waiting once the file has been compiled, it's gone then, as before.
pub(crate) struct KeptAst {
    /// The file's own, or the one all files whose functions wait are parsed into ([`Vm::kept_memory`]), as each tree is
    /// usually small, and memory of its own costs more than the tree.
    memory: std::rc::Rc<AstMemory>,
    source: KeptSource,
}

impl KeptAst {
    pub(crate) fn new(source: KeptSource) -> std::rc::Rc<Self> {
        std::rc::Rc::new(Self {
            memory: std::rc::Rc::new(AstMemory::new()),
            source,
        })
    }

    /// For a file whose functions are to wait, parsed into `memory`, which other files' trees are in too.
    pub(crate) fn sharing(source: KeptSource, memory: std::rc::Rc<AstMemory>) -> std::rc::Rc<Self> {
        std::rc::Rc::new(Self { memory, source })
    }

    /// For what's already been parsed into `memory` from `source`.
    pub(crate) fn of(memory: AstMemory, source: String) -> std::rc::Rc<Self> {
        std::rc::Rc::new(Self {
            memory: std::rc::Rc::new(memory),
            source: KeptSource::Owned(source),
        })
    }

    pub(crate) fn text(&self) -> &str {
        match &self.source {
            KeptSource::Static(text) => text,
            KeptSource::Owned(text) => text,
        }
    }

    pub(crate) fn memory(&self) -> &AstMemory {
        &self.memory
    }

    /// Once a file has been compiled: if some of its functions are waiting, the memory the parser took in advance
    /// isn't needed.
    pub(crate) fn finished(self: std::rc::Rc<Self>) {
        // Memory that's shared goes on being parsed into.
        if std::rc::Rc::strong_count(&self) > 1 && std::rc::Rc::strong_count(&self.memory) == 1 {
            self.memory.shrink();
        }
    }
}

impl Vm {
    /// Where the files whose functions wait to be compiled are parsed, all into one.
    pub(crate) fn kept_memory(&mut self) -> std::rc::Rc<AstMemory> {
        self.kept_memory
            .get_or_insert_with(|| std::rc::Rc::new(AstMemory::new()))
            .clone()
    }
}

#[cfg(test)]
#[path = "const_capture_tests.rs"]
mod const_capture_tests;

/// A function that's compiled the first time it's called, rather than with the code it's in. Most of the functions in
/// the bundled JavaScript never run in a given program. What it needs from outside was worked out when the code
/// around it was compiled: its proto already has its captures, so the code around it never needs compiling again.
pub(crate) struct Uncompiled {
    /// In `kept`, which lasts as long as this does. Nothing borrowed from here outlives the compiling of it.
    function: &'static Function<'static>,
    kept: std::rc::Rc<KeptAst>,
    /// The name and kind of each of its captures, in their order, and whether each is from the top of a module or a
    /// script.
    outside: Box<[(&'static str, VarKind, bool)]>,
    /// The private names of the classes around it.
    private_scopes: Box<[Vec<(&'static str, Atom)>]>,
    /// Whether it's strict because of where it is.
    outer_strict: bool,
    has_fields: bool,
}

impl Uncompiled {
    /// The line it starts on.
    pub(crate) fn line(&self) -> u32 {
        self.function.line
    }
}

const MAX_REGISTERS: u32 = 60000;

struct Var<'a> {
    name: &'a str,
    reg: u16,
    cell: bool,
    kind: VarKind,
    /// Whether code that comes after this point can count on it having started.
    started: bool,
}

#[derive(Default)]
struct Block<'a> {
    vars: Vec<Var<'a>>,
    /// Simple immutable bindings whose initialization precedes every capture.
    const_values: Vec<&'a str>,
    /// Where its code starts, for what a debugger's told about its variables.
    start: u32,
}

#[derive(Clone, Copy, PartialEq)]
enum Place {
    Register {
        reg: u16,
        kind: VarKind,
        started: bool,
    },
    Cell {
        reg: u16,
        kind: VarKind,
    },
    Up {
        index: u16,
        kind: VarKind,
    },
    /// A capture that's the value of a parameter that's never given another, rather than a cell.
    UpValue {
        index: u16,
        kind: VarKind,
    },
    Global,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Exit {
    Return,
    Break(u32),
    Continue(u32),
}

#[derive(Clone, Copy, PartialEq)]
enum TargetKind {
    Loop,
    Switch,
    Block,
}

enum Control<'a> {
    Target {
        id: u32,
        labels: Vec<&'a str>,
        kind: TargetKind,
        breaks: Vec<usize>,
        continues: Vec<usize>,
        /// The iterator to close when leaving, for a `for of`.
        iter: Option<u16>,
        /// Whether it's a `for await`, whose iterator is closed by waiting for what its `return` gives.
        is_await: bool,
    },
    Finally {
        kind_reg: u16,
        value_reg: u16,
        entries: Vec<usize>,
        exits: Vec<Exit>,
    },
}

struct FnState<'a> {
    kind: FunctionKind,
    inner: Names<'a>,
    blocks: Vec<Block<'a>>,
    code: Vec<Op>,
    caches: Vec<Cache>,
    handlers: Vec<Handler>,
    lines: Vec<(u32, u32)>,
    next_reg: u32,
    max_reg: u32,
    /// The registers below this are `this`, the parameters and the function's own variables, which a call makes
    /// undefined. Everything above is written before it's read.
    clear: u32,
    /// What it captures: each name, how, its kind, and whether it's from the top of a module or a script, whose variables
    /// what's read as a component renders doesn't note.
    upvalues: Vec<(&'a str, Capture, VarKind, bool)>,
    control: Vec<Control<'a>>,
    chains: Vec<Vec<usize>>,
    next_target: u32,
    pending_labels: Vec<&'a str>,
    /// Jumps are as long as they can be, as a short one was too short.
    wide: bool,
    overflowed: bool,
    has_fields: bool,
    /// The cell `this` is in, when inner arrows can see it.
    this_cell: Option<u16>,
    /// The parameters that are never given another value (see [`Rebinds`]), which a closure captures the value of
    /// rather than a cell, so they live in their registers as if nothing captured them.
    by_value: Vec<&'a str>,
    const_value_captures: bool,
    strict: bool,
    is_async_generator: bool,
    /// The register of the list a `using` declaration adds what it's given to, for each block around what's being
    /// compiled that has one, with the innermost last.
    disposals: Vec<u16>,
    /// Whether it has somewhere a debugger can stop ([`Stmt::Line`]), and if so, where each of its variables is and
    /// for which stretch of its code.
    steps: bool,
    debug_vars: Vec<crate::op::DebugVar>,
    /// Whether what it reads of what it captures, and what it's given if it's a component, is noted while something's
    /// rendering (see `reads.rs`): for an app's own code.
    noting: bool,
    /// Whether it's the top of a module or a script.
    is_top: bool,
    /// Whether it's named as a component is, with a capital, and given its props as its first parameter.
    component: bool,
    /// Whether it's a component or a hook (`useSomething`) of the app's own, whose plain calls can give what they gave
    /// at the last render (see `NOTE_KEEP` and `keeping.rs`).
    keeps: bool,
    /// The registers of a component's props and of the names destructured from them, which are what a Note says is read,
    /// with the name each was destructured as.
    roots: Vec<(u16, Option<&'a str>)>,
    /// The registers of the parameters of a function that isn't a component whose reads are noted where what it was given
    /// was (see `NOTE_PARAMS`), which are only ever read here.
    param_roots: Vec<u16>,
    /// Whether the next Note is of something given to a call, as an argument (see `NOTE_ARG`).
    arg_next: bool,
    /// How many Notes of what's given to the call whose arguments are being worked out there have been.
    arg_notes: u32,
    /// Whether the next object literal is the props of an element the app's JSX makes (see `NOTE_ARG`).
    jsx_props_next: bool,
    /// The prop the next Note's read is given as, to an element the app's JSX makes, as `look` in `<Icon look={look} />`.
    prop_next: Option<Atom>,
}

/// What a read that's noted starts from.
#[derive(Clone, Copy)]
enum NoteBase<'a> {
    /// A component's props, or a name destructured from them, in this register.
    Own { reg: u16, key: Option<&'a str> },
    /// This captured variable.
    Up { index: u16 },
    /// A parameter of a function that isn't a component, in this register: see `NOTE_PARAMS`.
    Param { reg: u16 },
}

/// Reads that have been emitted, with the Note to emit after them: see `Compiler::noted_reads`.
struct NotedReads {
    value: u16,
    root: u16,
    kind: u8,
    keys: Vec<Op>,
    /// Where in `keys` what's compared with goes, after the name the props were destructured as.
    first: usize,
    mark: u32,
}

/// One step of a chain of reads.
#[derive(Clone, Copy)]
enum ChainStep<'a> {
    Name(&'a str),
    Index(&'a Expr<'a>),
}

pub(crate) struct Compiler<'a, 'v> {
    vm: &'v mut Vm,
    fns: Vec<FnState<'a>>,
    file: Atom,
    stack_start: usize,
    stack_room: usize,
    /// How many `with`s the code being compiled is inside of, over all the functions around it.
    with_depth: usize,
    /// How many classes it's inside of, as everything in one is strict.
    class_depth: usize,
    /// The private names of each class around what's being compiled, with the innermost last. Each class has its own, so
    /// that a `#value` in a class and another in the class it extends are two different things.
    private_scopes: Vec<Vec<(&'a str, Atom)>>,
    /// Whether the program's last statement gives the result, as it does for `eval`.
    keeps_last_value: bool,
    /// Whether this is a script, whose `var`s and functions go on the global object.
    is_script: bool,
    /// What's being compiled, when what's in it can be compiled later. See [`Uncompiled`].
    kept: Option<std::rc::Rc<KeptAst>>,
    /// Whether everything being compiled has been checked for mistakes already, as what was waiting to be compiled
    /// has.
    checked: bool,
    /// For a function that was waiting to be compiled: what it sees from outside, the proto it's to go in, and whether
    /// it's strict because of where it is.
    outside: Vec<(&'a str, Capture, VarKind, bool)>,
    into: Option<u32>,
    outer_strict: bool,
    /// What a module being compiled imports and exports, for what loads it. See [`crate::modules::ModuleInfo`].
    module: Option<crate::modules::ModuleInfo>,
    /// The source what's being compiled is in, by its number, for the text of its functions. See `source_text.rs`.
    source: u32,
    /// Whether the file is one of the app's own, once that's been asked: see [`Compiler::jsx_factory`].
    app_file: Option<bool>,
    /// Whether the next read that's noted has to be the same value as itself, not one that does the same: see
    /// `NOTE_SAME_ITSELF`.
    itself_next: bool,
    /// Whether the elements of the next array literal are read as themselves, as a hook's dependencies are.
    itself_items: bool,
}

/// Whether a name is a hook's, as React has them: `use`, and a capital or nothing after it.
fn is_hook_name(name: &str) -> bool {
    name.strip_prefix("use").is_some_and(|rest| rest.chars().next().is_none_or(|first| first.is_ascii_uppercase()))
}

/// What a module's `export default` of something that isn't a declaration is kept in. No name that can be written is this.
const DEFAULT_LOCAL: &str = "*default*";

/// How a name that a module exports is declared at its top: as what kind of variable, and whether it has a value before
/// the code gets to it.
fn top_declaration(body: &[Stmt<'_>], wanted: &str) -> Option<(VarKind, bool)> {
    for statement in body {
        match statement {
            Stmt::Var(kind, declarations) => {
                for (pattern, _) in declarations.iter() {
                    let mut names = Vec::new();
                    pattern_names(pattern, &mut names);
                    if names.contains(&wanted) {
                        return Some((*kind, *kind == VarKind::Var));
                    }
                }
            }
            Stmt::Function(function) if function.name == Some(wanted) => {
                return Some((VarKind::Let, true));
            }
            Stmt::Class(class) if class.name == Some(wanted) => return Some((VarKind::Let, false)),
            _ => {}
        }
    }
    None
}

/// Whether a function's body starts by saying it's strict.
/// Whether this makes a function with no name of its own, which takes its name from where it's put, as
/// [`Compiler::expr_named`] gives it.
fn is_anonymous_function(expression: &Expr<'_>) -> bool {
    match expression {
        Expr::Function(function) => function.name.is_none(),
        Expr::Class(class) => class.name.is_none(),
        _ => false,
    }
}

fn says_strict(body: &[Stmt<'_>]) -> bool {
    let wanted: Vec<u16> = "use strict".encode_utf16().collect();
    for statement in body {
        match statement {
            Stmt::Line(_) => {}
            Stmt::Expr(Expr::String(units)) => {
                if **units == *wanted {
                    return true;
                }
            }
            _ => return false,
        }
    }
    false
}

fn pattern_names<'a>(pattern: &Pattern<'a>, out: &mut Vec<&'a str>) {
    match pattern {
        Pattern::Ident(name, _) => out.push(name),
        Pattern::Expr(_) => {}
        Pattern::Array { items, rest } => {
            for item in items.iter().flatten() {
                pattern_names(&item.target, out);
            }
            if let Some(rest) = rest {
                pattern_names(rest, out);
            }
        }
        Pattern::Object { items, rest } => {
            for (_, item) in items.iter() {
                pattern_names(&item.target, out);
            }
            if let Some(rest) = rest {
                pattern_names(rest, out);
            }
        }
    }
}

/// Finds every `var` in these statements, which all belong to the function, however deep they are.
fn state_kind_is_fields(kind: FunctionKind) -> bool {
    kind == FunctionKind::Fields
}

/// Which of some names a function's body may give a new value to, anywhere in it or in a function inside it: by
/// assigning to it, `++` or `--`, a `for in` or `for of` that assigns to it, or declaring something of that name, which
/// for a `var` or a function is the same variable as a parameter, and for anything else is taken to be as well. What
/// isn't looked through here (a `with`, whose names could be anything) counts as giving every one a new value. A
/// parameter none of this happens to keeps the value it was called with, so a closure can be given that value rather
/// than a cell.
struct Rebinds<'n> {
    names: &'n [&'n str],
    hit: Vec<bool>,
    captures: bool,
    opaque: bool,
    depth: u32,
}

impl<'n> Rebinds<'n> {
    fn new(names: &'n [&'n str]) -> Self {
        Self {
            names,
            hit: vec![false; names.len()],
            captures: false,
            opaque: false,
            depth: 0,
        }
    }

    fn captures(names: &'n [&'n str]) -> Self {
        Self {
            captures: true,
            ..Self::new(names)
        }
    }

    fn name(&mut self, name: &str) {
        if self.captures {
            return;
        }
        self.capture_name(name);
    }

    fn capture_name(&mut self, name: &str) {
        for (at, candidate) in self.names.iter().enumerate() {
            if *candidate == name {
                self.hit[at] = true;
            }
        }
    }

    fn all(&mut self) {
        self.hit.fill(true);
        self.opaque = true;
    }

    fn enter(&mut self) -> bool {
        if self.depth >= 64 {
            self.all();
            return false;
        }
        self.depth += 1;
        true
    }

    fn statements(&mut self, statements: &[Stmt<'_>]) {
        for statement in statements {
            self.statement(statement);
        }
    }

    fn statement(&mut self, statement: &Stmt<'_>) {
        if !self.enter() {
            return;
        }
        match statement {
            Stmt::Empty | Stmt::Line(_) | Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::Expr(expression) => self.expr(expression),
            Stmt::Throw(expression) | Stmt::ExportDefault(expression) => self.expr(expression),
            Stmt::Var(_, declarations) => self.declarations(declarations),
            Stmt::Function(function) => {
                if let Some(name) = function.name {
                    self.name(name);
                }
                self.function(function);
            }
            Stmt::Class(class) => {
                if let Some(name) = class.name {
                    self.name(name);
                }
                self.class(class);
            }
            Stmt::Block(body) => self.statements(body),
            Stmt::If(test, then, otherwise) => {
                self.expr(test);
                self.statement(then);
                if let Some(otherwise) = otherwise {
                    self.statement(otherwise);
                }
            }
            Stmt::For(parts) => {
                match &parts.init {
                    Some(ForInit::Var(_, declarations)) => self.declarations(declarations),
                    Some(ForInit::Expr(expression)) => self.expr(expression),
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
            Stmt::Switch(test, cases) => {
                self.expr(test);
                for case in cases.iter() {
                    if let Some(test) = &case.test {
                        self.expr(test);
                    }
                    self.statements(case.body);
                }
            }
            Stmt::Labeled(_, body) => self.statement(body),
            Stmt::Field(key, value) => {
                self.key(key);
                if let Some(value) = value {
                    self.expr(value);
                }
            }
            Stmt::Import(import) => {
                for name in import.default.iter().chain(&import.all) {
                    self.name(name);
                }
                for (_, local) in import.named.iter() {
                    self.name(local);
                }
            }
            Stmt::Export(_) | Stmt::ExportFrom(_) => {}
            Stmt::With(..) => self.all(),
        }
        self.depth -= 1;
    }

    fn declarations(&mut self, declarations: &[(Pattern<'_>, Option<Expr<'_>>)]) {
        for (pattern, value) in declarations.iter() {
            self.pattern(pattern);
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

    /// Every name in a pattern is given a value, and what's in its defaults and keys is looked through.
    fn pattern(&mut self, pattern: &Pattern<'_>) {
        if !self.enter() {
            return;
        }
        match pattern {
            Pattern::Ident(name, _) => self.name(name),
            Pattern::Expr(expression) => self.expr(expression),
            Pattern::Array { items, rest } => {
                for item in items.iter().flatten() {
                    self.item(item);
                }
                if let Some(rest) = rest {
                    self.pattern(rest);
                }
            }
            Pattern::Object { items, rest } => {
                for (key, item) in items.iter() {
                    self.key(key);
                    self.item(item);
                }
                if let Some(rest) = rest {
                    self.pattern(rest);
                }
            }
        }
        self.depth -= 1;
    }

    fn item(&mut self, item: &PatternItem<'_>) {
        self.pattern(&item.target);
        if let Some(default) = &item.default {
            self.expr(default);
        }
    }

    fn key(&mut self, key: &PropertyKey<'_>) {
        if let PropertyKey::Computed(expression) = key {
            self.expr(expression);
        }
    }

    fn function(&mut self, function: &Function<'_>) {
        if self.captures {
            for name in function.outer_names {
                self.capture_name(name);
                if *name == "eval" {
                    self.all();
                }
            }
            return;
        }
        // A function's own parameters are its own, and only what's assigned in their defaults matters out here.
        for param in function.params.iter() {
            self.param(&param.target);
            if let Some(default) = &param.default {
                self.expr(default);
            }
        }
        if let Some(rest) = &function.rest {
            self.param(rest);
        }
        self.statements(function.body);
    }

    fn param(&mut self, pattern: &Pattern<'_>) {
        match pattern {
            Pattern::Ident(..) => {}
            other => self.pattern(other),
        }
    }

    fn class(&mut self, class: &Class<'_>) {
        if let Some(parent) = &class.parent {
            self.expr(parent);
        }
        self.function(class.constructor);
        for member in class.members.iter() {
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

    fn elements(&mut self, elements: &[Element<'_>]) {
        for element in elements {
            match element {
                Element::Plain(value) | Element::Spread(value) => self.expr(value),
            }
        }
    }

    fn expr(&mut self, expression: &Expr<'_>) {
        if !self.enter() {
            return;
        }
        if matches!(expression, Expr::Ident("eval", _)) {
            self.all();
        }
        match expression {
            Expr::Number(_)
            | Expr::BigInt(..)
            | Expr::String(_)
            | Expr::Regex(_)
            | Expr::Ident(..)
            | Expr::PrivateName(_)
            | Expr::ImportMeta
            | Expr::This
            | Expr::Null
            | Expr::True
            | Expr::False
            | Expr::NewTarget
            | Expr::SuperMember(_) => {}
            Expr::Template(parts) => {
                for value in parts.values.iter() {
                    self.expr(value);
                }
            }
            Expr::TaggedTemplate(parts) => {
                self.expr(parts.tag);
                for value in parts.values.iter() {
                    self.expr(value);
                }
            }
            Expr::ImportCall(value)
            | Expr::Unary(_, value)
            | Expr::Chain(value)
            | Expr::SuperIndex(value)
            | Expr::Await(value) => self.expr(value),
            Expr::Array(elements) => {
                for element in elements.iter().flatten() {
                    match element {
                        Element::Plain(value) | Element::Spread(value) => self.expr(value),
                    }
                }
            }
            Expr::Object(properties) => {
                for property in properties.iter() {
                    match property {
                        Property::Value(key, value) => {
                            self.key(key);
                            self.expr(value);
                        }
                        Property::Short(..) => {}
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
            Expr::Function(function) => {
                // A function expression's own name is its own, inside it.
                self.function(function);
            }
            Expr::Class(class) => self.class(class),
            Expr::Update { target, .. } => match target {
                Expr::Ident(name, _) => self.name(name),
                other => self.expr(other),
            },
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
            Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
                self.expr(callee);
                self.elements(args);
            }
            Expr::Member { object, .. } => self.expr(object),
            Expr::Index { object, index, .. } => {
                self.expr(object);
                self.expr(index);
            }
            Expr::SuperCall(args) => self.elements(args),
            Expr::Sequence(values) => {
                for value in values.iter() {
                    self.expr(value);
                }
            }
            Expr::Yield { value, .. } => {
                if let Some(value) = value {
                    self.expr(value);
                }
            }
        }
        self.depth -= 1;
    }
}

pub(crate) fn collect_vars<'a>(statements: &[Stmt<'a>], out: &mut Vec<&'a str>) {
    for statement in statements {
        collect_vars_one(statement, out);
    }
}

fn collect_vars_one<'a>(statement: &Stmt<'a>, out: &mut Vec<&'a str>) {
    match statement {
        Stmt::Var(VarKind::Var, declarations) => {
            for (pattern, _) in declarations.iter() {
                pattern_names(pattern, out);
            }
        }
        Stmt::Block(body) => collect_vars(body, out),
        Stmt::If(_, then, otherwise) => {
            collect_vars_one(then, out);
            if let Some(otherwise) = otherwise {
                collect_vars_one(otherwise, out);
            }
        }
        Stmt::For(parts) => {
            if let Some(ForInit::Var(VarKind::Var, declarations)) = &parts.init {
                for (pattern, _) in declarations.iter() {
                    pattern_names(pattern, out);
                }
            }
            collect_vars_one(parts.body, out);
        }
        Stmt::ForIn(parts) => {
            if let ForTarget::Declare(VarKind::Var, pattern) = &parts.target {
                pattern_names(pattern, out);
            }
            collect_vars_one(parts.body, out);
        }
        Stmt::ForOf(parts) => {
            if let ForTarget::Declare(VarKind::Var, pattern) = &parts.target {
                pattern_names(pattern, out);
            }
            collect_vars_one(parts.body, out);
        }
        Stmt::While(_, body)
        | Stmt::DoWhile(body, _)
        | Stmt::Labeled(_, body)
        | Stmt::With(_, body) => {
            collect_vars_one(body, out);
        }
        Stmt::Try(parts) => {
            collect_vars(parts.block, out);
            if let Some(handler) = parts.handler {
                collect_vars(handler, out);
            }
            if let Some(finalizer) = parts.finalizer {
                collect_vars(finalizer, out);
            }
        }
        Stmt::Switch(_, cases) => {
            for case in cases.iter() {
                collect_vars(case.body, out);
            }
        }
        _ => {}
    }
}

/// Whether working out this expression could change a local variable, as far as can be quickly told.
fn may_assign(expression: &Expr<'_>, depth: u32) -> bool {
    if depth == 0 {
        return true;
    }
    let next = depth - 1;
    let elements = |args: &[Element<'_>]| {
        args.iter().any(|arg| match arg {
            Element::Plain(value) | Element::Spread(value) => may_assign(value, next),
        })
    };
    match expression {
        Expr::Number(_)
        | Expr::BigInt(..)
        | Expr::String(_)
        | Expr::Ident(..)
        | Expr::This
        | Expr::Null
        | Expr::True
        | Expr::False
        | Expr::Function(_) => false,
        Expr::Unary(_, value) => may_assign(value, next),
        Expr::Binary(_, a, b) | Expr::Logical(_, a, b) => {
            may_assign(a, next) || may_assign(b, next)
        }
        Expr::Conditional(a, b, c) => {
            may_assign(a, next) || may_assign(b, next) || may_assign(c, next)
        }
        Expr::Member { object, .. } => may_assign(object, next),
        Expr::Index { object, index, .. } => may_assign(object, next) || may_assign(index, next),
        Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
            may_assign(callee, next) || elements(args)
        }
        _ => true,
    }
}

/// Whether this can be put straight into a variable that it may also read.
/// Everything here only writes to where it's going as its very last step.
fn writes_last(expression: &Expr<'_>) -> bool {
    matches!(
        expression,
        Expr::Number(_)
            | Expr::String(_)
            | Expr::Ident(..)
            | Expr::This
            | Expr::Null
            | Expr::True
            | Expr::False
            | Expr::Function(_)
            | Expr::Binary(..)
            | Expr::Member {
                optional: false,
                ..
            }
            | Expr::Index {
                optional: false,
                ..
            }
            | Expr::Call {
                optional: false,
                ..
            }
            | Expr::New { .. }
    ) || matches!(expression, Expr::Conditional(_, then, otherwise) if writes_last(then) && writes_last(otherwise))
}

/// Where a result goes when nothing wants it, so there's no need to put it anywhere.
const DISCARD: u16 = u16::MAX;

/// The instruction for an operator with a small number on the right, if it has one.
fn small_number_op(op: BinaryOp, dst: u16, a: u16, number: f64) -> Option<Op> {
    let value = number as i16;
    if f64::from(value) != number || (value == 0 && number.is_sign_negative()) {
        return None;
    }
    Some(match op {
        BinaryOp::Add => Op::AddInt { dst, a, value },
        BinaryOp::Sub => Op::SubInt { dst, a, value },
        BinaryOp::Mul if value != 0 => Op::MulInt { dst, a, value },
        BinaryOp::Mod if value > 0 => Op::ModInt { dst, a, value },
        BinaryOp::Div if value > 0 && value & (value - 1) == 0 => Op::DivInt { dst, a, value },
        BinaryOp::BitAnd => Op::BitAndInt { dst, a, value },
        BinaryOp::BitOr => Op::BitOrInt { dst, a, value },
        BinaryOp::Shl => Op::ShlInt { dst, a, value },
        BinaryOp::Shr => Op::ShrInt { dst, a, value },
        BinaryOp::UShr => Op::UShrInt { dst, a, value },
        _ => return None,
    })
}

/// The number `typeof` gives for a string it can give.
fn type_kind(units: &[u16]) -> Option<u16> {
    let name = String::from_utf16_lossy(units);
    Some(match name.as_str() {
        "number" => TYPE_NUMBER,
        "string" => TYPE_STRING,
        "boolean" => TYPE_BOOLEAN,
        "undefined" => TYPE_UNDEFINED,
        "symbol" => TYPE_SYMBOL,
        "bigint" => TYPE_BIGINT,
        "function" => TYPE_FUNCTION,
        "object" => TYPE_OBJECT,
        _ => return None,
    })
}

impl<'a, 'v> Compiler<'a, 'v> {
    pub(crate) fn new(vm: &'v mut Vm, file: Atom) -> Self {
        let room = vm.stack_room();
        Self {
            stack_room: room,
            vm,
            fns: Vec::new(),
            file,
            stack_start: crate::stack_position(),
            with_depth: 0,
            class_depth: 0,
            private_scopes: Vec::new(),
            keeps_last_value: false,
            is_script: false,
            kept: None,
            checked: false,
            outside: Vec::new(),
            into: None,
            outer_strict: false,
            module: None,
            source: crate::source_text::NO_SOURCE,
            app_file: None,
            itself_next: false,
            itself_items: false,
        }
    }

    /// What JSX calls here. The parser has an element call `__jsxMemo` or `__jsxPageMemo` where nothing says `'use no memo'`
    /// (see `Parser::jsx_name`), which lets a component it makes skip rendering when it's given what does the same as last
    /// time. That's only for an app's own files: one in `node_modules`, or code that isn't in a file, calls `__jsx` or
    /// `__jsxPage`, as what's there was written to be rendered whenever what it's in is, as React has it. That's any mention of
    /// the name there, so what defines `__jsxMemo` calls it by another.
    fn jsx_factory(&mut self, name: &'a str) -> &'a str {
        let plain = match name {
            "__jsxMemo" => "__jsx",
            "__jsxPageMemo" => "__jsxPage",
            _ => return name,
        };
        if self.app_file() { name } else { plain }
    }

    /// Whether what's compiled is in one of the app's own files: a real path, outside `node_modules`.
    /// Whether a function is a component whose reads of its props are noted (see `reads.rs`): one named with a capital, as
    /// React has a component, that isn't a method, a class, async or a generator, given its props first as a name or an object
    /// pattern with no default, and that doesn't see its arguments any other way.
    fn is_component(&mut self, function: &'a Function<'a>, hint: Option<Atom>) -> bool {
        let named = match function.name {
            Some(name) => name.chars().next().is_some_and(|first| first.is_ascii_uppercase()),
            None => hint.is_some_and(|hint| {
                self.vm.atom_text(hint).chars().next().is_some_and(|first| first.is_ascii_uppercase())
            }),
        };
        let Some(first) = function.params.first() else {
            return false;
        };
        let plain = match &first.target {
            Pattern::Ident(name, _) => {
                let names = [*name];
                let mut rebinds = Rebinds::new(&names);
                rebinds.statements(function.body);
                !rebinds.hit[0]
            }
            Pattern::Object { .. } => true,
            _ => false,
        };
        named
            && plain
            && first.default.is_none()
            && matches!(function.kind, FunctionKind::Normal | FunctionKind::Arrow)
            && !function.is_async
            && !function.is_generator
            && !function.uses_arguments
            && !function.inner_names.contains("arguments")
            && self.with_depth == 0
    }

    /// Whether a function is a hook, as React names one, `use` and a capital, that gives its answer as it's called.
    fn is_hook(&mut self, function: &'a Function<'a>, hint: Option<Atom>) -> bool {
        let named = match function.name {
            Some(name) => is_hook_name(name),
            None => hint.is_some_and(|hint| is_hook_name(&self.vm.atom_text(hint))),
        };
        named
            && matches!(function.kind, FunctionKind::Normal | FunctionKind::Arrow)
            && !function.is_async
            && !function.is_generator
            && self.with_depth == 0
    }

    fn app_file(&mut self) -> bool {
        *self.app_file.get_or_insert_with(|| {
            let path = std::path::PathBuf::from(self.vm.atom_text(self.file));
            path.is_absolute()
                && !path
                    .components()
                    .any(|part| part.as_os_str().eq_ignore_ascii_case("node_modules"))
        })
    }

    /// Says which source what's compiled is in, so each function can say what its text is.
    pub(crate) fn with_source(mut self, source: u32) -> Self {
        self.source = source;
        self
    }

    /// Notes which source a function is from, and where its text is in it, which is nowhere for one that was made up,
    /// as a file's own function is. What's inside one that waits to be compiled is from the same source.
    fn note_text(&mut self, id: u32, function: &Function<'_>) {
        if self.source != crate::source_text::NO_SOURCE {
            self.vm.set_text(id, self.source, function.source);
        }
    }

    /// Lets the functions in what's compiled wait until they're called to be compiled. `kept` has to be what the code
    /// being compiled is in. What's in the program itself isn't checked for mistakes before it's called, as reading
    /// through it all takes a good part of what's saved: the bundled files are checked by tests instead
    /// ([`crate::check_compiles`]).
    pub(crate) fn keeping(mut self, kept: &std::rc::Rc<KeptAst>, lazily: bool) -> Self {
        if lazily {
            self.checked = matches!(kept.source, KeptSource::Static(_));
            self.kept = Some(kept.clone());
        }
        self
    }

    fn f(&mut self) -> &mut FnState<'a> {
        self.fns.last_mut().expect("compiling a function")
    }

    fn error<T>(&self, message: impl Into<String>, line: u32) -> Compiled<T> {
        Err(SyntaxError {
            message: message.into(),
            line,
        })
    }

    fn check_depth(&self) -> Compiled<()> {
        if self.stack_start.abs_diff(crate::stack_position()) > self.stack_room {
            return self.error("this code is nested too deeply", 0);
        }
        Ok(())
    }

    #[inline]
    fn emit(&mut self, op: Op) -> usize {
        let state = self.f();
        state.code.push(op);
        state.code.len() - 1
    }

    fn here(&mut self) -> usize {
        self.f().code.len()
    }

    fn mark_line(&mut self, line: u32) {
        let at = self.here() as u32;
        let state = self.f();
        if state.lines.last().is_none_or(|last| last.1 != line) {
            state.lines.push((at, line));
        }
    }

    fn alloc(&mut self) -> Compiled<u16> {
        let state = self.f();
        let reg = state.next_reg;
        state.next_reg += 1;
        if state.next_reg > state.max_reg {
            state.max_reg = state.next_reg;
        }
        if reg >= MAX_REGISTERS {
            return self.error("this function has too many variables", 0);
        }
        Ok(reg as u16)
    }

    fn mark(&mut self) -> u32 {
        self.f().next_reg
    }

    fn release(&mut self, mark: u32) {
        self.f().next_reg = mark;
    }

    fn patch_to(&mut self, at: usize, target: usize) {
        let distance = target as i64 - (at as i64 + 1);
        let state = self.f();
        match &mut state.code[at] {
            Op::Jump { offset }
            | Op::Loop { offset }
            | Op::JumpIfTrue { offset, .. }
            | Op::JumpIfFalse { offset, .. }
            | Op::JumpIfNullish { offset, .. }
            | Op::JumpIfNotNullish { offset, .. }
            | Op::JumpIfNotUndefined { offset, .. }
            | Op::JumpIfUndefined { offset, .. }
            | Op::JumpIfNull { offset, .. }
            | Op::JumpIfNotNull { offset, .. } => *offset = distance as i32,
            Op::JumpLt { offset, .. }
            | Op::JumpLe { offset, .. }
            | Op::JumpGt { offset, .. }
            | Op::JumpGe { offset, .. }
            | Op::JumpNotLt { offset, .. }
            | Op::JumpNotLe { offset, .. }
            | Op::JumpNotGt { offset, .. }
            | Op::JumpNotGe { offset, .. }
            | Op::JumpStrictEq { offset, .. }
            | Op::JumpStrictNe { offset, .. }
            | Op::JumpEq { offset, .. }
            | Op::JumpNe { offset, .. }
            | Op::JumpLtInt { offset, .. }
            | Op::JumpLeInt { offset, .. }
            | Op::JumpGtInt { offset, .. }
            | Op::JumpGeInt { offset, .. }
            | Op::JumpNotLtInt { offset, .. }
            | Op::JumpNotLeInt { offset, .. }
            | Op::JumpNotGtInt { offset, .. }
            | Op::JumpNotGeInt { offset, .. }
            | Op::JumpStrictEqInt { offset, .. }
            | Op::JumpStrictNeInt { offset, .. }
            | Op::JumpTypeIs { offset, .. }
            | Op::JumpTypeIsNot { offset, .. }
            | Op::LoopLt { offset, .. }
            | Op::LoopLe { offset, .. }
            | Op::LoopGt { offset, .. }
            | Op::LoopGe { offset, .. }
            | Op::LoopLtInt { offset, .. }
            | Op::LoopLeInt { offset, .. }
            | Op::LoopGtInt { offset, .. }
            | Op::LoopGeInt { offset, .. }
            | Op::LoopStrictNe { offset, .. }
            | Op::LoopStrictNeInt { offset, .. }
            | Op::IncLoopLt { offset, .. }
            | Op::IncLoopLtInt { offset, .. }
            | Op::IncLoopLtLength { offset, .. }
            | Op::IterResult { offset, .. }
            | Op::IterNext { offset, .. } => match i16::try_from(distance) {
                Ok(short) => *offset = short,
                Err(_) => state.overflowed = true,
            },
            other => unreachable!("{other:?} isn't a jump"),
        }
    }

    fn patch(&mut self, at: usize) {
        let target = self.here();
        self.patch_to(at, target);
    }

    fn patch_all(&mut self, list: &[usize]) {
        for at in list {
            self.patch(*at);
        }
    }

    fn jump_back(&mut self, target: usize) {
        let at = self.emit(Op::Loop { offset: 0 });
        self.patch_to(at, target);
    }

    fn jump(&mut self) -> usize {
        self.emit(Op::Jump { offset: 0 })
    }

    fn intern(&mut self, name: &str) -> Atom {
        self.vm.intern_str(name)
    }

    fn atom_of_key(&mut self, name: &str) -> Atom {
        if name.starts_with('#') {
            let found = self
                .private_scopes
                .iter()
                .rev()
                .flatten()
                .find(|(known, _)| *known == name);
            match found {
                Some((_, atom)) => *atom,
                None => self.vm.private_atom(name),
            }
        } else {
            self.intern(name)
        }
    }

    fn cache(&mut self, atom: Atom) -> Compiled<u16> {
        let state = self.f();
        if state.caches.len() >= u16::MAX as usize {
            return self.error("this function has too many property accesses", 0);
        }
        state.caches.push(Cache::new(atom));
        Ok((state.caches.len() - 1) as u16)
    }

    fn load_const(&mut self, dst: u16, value: Value) {
        let index = self.vm.add_const(value);
        self.emit(Op::LoadConst { dst, index });
    }

    fn load_string(&mut self, dst: u16, units: &[u16]) {
        let atom = self.vm.intern_units(units);
        let value = self.vm.atom_value(atom);
        self.load_const(dst, value);
    }

    fn load_number(&mut self, dst: u16, number: f64) {
        let value = Value::number(number);
        if value.is_int() {
            self.emit(Op::LoadInt {
                dst,
                value: value.as_int(),
            });
        } else {
            self.load_const(dst, value);
        }
    }

    // Variables.

    fn find_local(state: &FnState<'a>, name: &str) -> Option<Place> {
        for block in state.blocks.iter().rev() {
            for var in block.vars.iter().rev() {
                if var.name == name {
                    return Some(if var.cell {
                        Place::Cell {
                            reg: var.reg,
                            kind: var.kind,
                        }
                    } else {
                        Place::Register {
                            reg: var.reg,
                            kind: var.kind,
                            started: var.started,
                        }
                    });
                }
            }
        }
        None
    }

    /// What a function inside the one at `depth` would capture of a variable of that one's: its cell, or for a
    /// parameter that's never given another value, the value itself.
    fn local_capture(state: &FnState<'a>, name: &str) -> Option<Result<(Capture, VarKind), ()>> {
        Some(match Self::find_local(state, name)? {
            Place::Cell { reg, kind } => Ok((
                Capture {
                    from_register: true,
                    by_value: false,
                    index: reg,
                },
                kind,
            )),
            Place::Register { reg, kind, started }
                if state.by_value.contains(&name)
                    || (kind == VarKind::Const
                        && started
                        && state.blocks.iter().any(|block| {
                            block.const_values.contains(&name)
                                && block.vars.iter().any(|var| var.reg == reg)
                        })) =>
            {
                Ok((
                    Capture {
                        from_register: true,
                        by_value: true,
                        index: reg,
                    },
                    kind,
                ))
            }
            _ => Err(()),
        })
    }

    fn upvalue(&mut self, depth: usize, name: &'a str) -> Compiled<Option<(u16, VarKind, bool)>> {
        // What the outermost function sees from outside is all there already, if it was waiting to be compiled.
        for (index, (found, capture, kind, _)) in self.fns[depth].upvalues.iter().enumerate() {
            if *found == name {
                return Ok(Some((index as u16, *kind, capture.by_value)));
            }
        }
        if depth == 0 {
            return Ok(None);
        }
        let (capture, kind, top) = match Self::local_capture(&self.fns[depth - 1], name) {
            Some(Ok((capture, kind))) => (capture, kind, self.fns[depth - 1].is_top),
            Some(Err(())) => {
                return self.error(format!("{name} should have been seen as captured"), 0);
            }
            None => match self.upvalue(depth - 1, name)? {
                Some((index, kind, by_value)) => (
                    Capture {
                        from_register: false,
                        by_value,
                        index,
                    },
                    kind,
                    self.fns[depth - 1].upvalues[index as usize].3,
                ),
                None => return Ok(None),
            },
        };
        let list = &mut self.fns[depth].upvalues;
        list.push((name, capture, kind, top));
        Ok(Some(((list.len() - 1) as u16, kind, capture.by_value)))
    }

    fn resolve(&mut self, name: &'a str) -> Compiled<Place> {
        if let Some(place) = Self::find_local(self.f(), name) {
            return Ok(place);
        }
        let depth = self.fns.len() - 1;
        Ok(match self.upvalue(depth, name)? {
            Some((index, kind, false)) => Place::Up { index, kind },
            Some((index, kind, true)) => Place::UpValue { index, kind },
            None => Place::Global,
        })
    }

    fn set_started(&mut self, name: &str) {
        for block in self.f().blocks.iter_mut().rev() {
            for var in block.vars.iter_mut().rev() {
                if var.name == name {
                    var.started = true;
                    return;
                }
            }
        }
    }

    /// Makes a variable in the current block, unless it's already there.
    fn declare(&mut self, name: &'a str, kind: VarKind, started: bool) -> Compiled<()> {
        let state = self.f();
        let block = state.blocks.last().expect("there's always a block");
        if block.vars.iter().any(|var| var.name == name) {
            return Ok(());
        }
        let cell = state.inner.contains(name) && !block.const_values.contains(&name);
        let reg = self.alloc()?;
        if cell {
            if kind == VarKind::Var || started {
                self.emit(Op::LoadUndefined { dst: reg });
            } else {
                self.emit(Op::LoadEmpty { dst: reg });
            }
            self.emit(Op::NewCell { dst: reg, src: reg });
        }
        self.f()
            .blocks
            .last_mut()
            .expect("there's always a block")
            .vars
            .push(Var {
                name,
                reg,
                cell,
                kind,
                started,
            });
        Ok(())
    }

    fn throw_tdz(&mut self, name: &str) {
        let atom = self.intern(name);
        self.emit(Op::ThrowTdz { name: atom.0 });
    }

    /// The `with`s between here and where a name is declared, from the closest out.
    fn with_chain(&mut self, name: &str) -> Vec<&'a str> {
        let mut chain = Vec::new();
        if self.with_depth == 0
            || name.starts_with('%')
            || matches!(name, "this" | "arguments" | "new.target")
        {
            return chain;
        }
        for state in self.fns.iter().rev() {
            for block in state.blocks.iter().rev() {
                for var in block.vars.iter().rev() {
                    if var.name == name {
                        return chain;
                    }
                    if var.name.starts_with("%with") {
                        chain.push(var.name);
                    }
                }
            }
        }
        chain
    }

    fn read_var(&mut self, name: &'a str, dst: u16) -> Compiled<()> {
        let chain = self.with_chain(name);
        if chain.is_empty() {
            self.read_plain(name, dst)?;
            return self.note_bare(name, dst);
        }
        let mark = self.mark();
        let object = self.alloc()?;
        let key = self.alloc()?;
        let found = self.alloc()?;
        let units: Vec<u16> = name.encode_utf16().collect();
        self.load_string(key, &units);
        let mut ends = Vec::new();
        for with_name in chain {
            self.read_plain(with_name, object)?;
            self.emit(Op::In {
                dst: found,
                a: key,
                b: object,
            });
            let skip = self.emit(Op::JumpIfFalse {
                condition: found,
                offset: 0,
            });
            self.emit(Op::GetElem { dst, object, key });
            ends.push(self.jump());
            self.patch(skip);
        }
        self.read_plain(name, dst)?;
        self.patch_all(&ends);
        self.release(mark);
        Ok(())
    }

    fn read_plain(&mut self, name: &'a str, dst: u16) -> Compiled<()> {
        match self.resolve(name)? {
            Place::Register { reg, started, .. } => {
                if !started {
                    self.throw_tdz(name);
                } else if reg != dst {
                    self.emit(Op::Move { dst, src: reg });
                }
            }
            Place::Cell { reg, .. } => {
                self.emit(Op::GetCell { dst, cell: reg });
            }
            Place::Up { index, .. } => {
                self.emit(Op::GetUp { dst, index });
            }
            Place::UpValue { index, .. } => {
                self.emit(Op::GetUpValue { dst, index });
            }
            Place::Global => {
                if name == "undefined" {
                    self.emit(Op::LoadUndefined { dst });
                } else {
                    let atom = self.intern(name);
                    let cache = self.cache(atom)?;
                    self.emit(Op::GetGlobal { dst, cache });
                }
            }
        }
        Ok(())
    }

    /// Gives a variable its first value.
    fn init_var(&mut self, name: &'a str, src: u16) -> Compiled<()> {
        match self.resolve(name)? {
            Place::Register { reg, .. } => {
                if reg != src {
                    self.emit(Op::Move { dst: reg, src });
                }
            }
            Place::Cell { reg, .. } => {
                self.emit(Op::InitCell { cell: reg, src });
            }
            Place::Up { .. } | Place::UpValue { .. } | Place::Global => {
                return self.error(format!("{name} isn't declared here"), 0);
            }
        }
        self.set_started(name);
        Ok(())
    }

    fn write_var(&mut self, name: &'a str, src: u16) -> Compiled<()> {
        let chain = self.with_chain(name);
        if !chain.is_empty() {
            let mark = self.mark();
            let object = self.alloc()?;
            let key = self.alloc()?;
            let found = self.alloc()?;
            let units: Vec<u16> = name.encode_utf16().collect();
            self.load_string(key, &units);
            let mut ends = Vec::new();
            for with_name in chain {
                self.read_plain(with_name, object)?;
                self.emit(Op::In {
                    dst: found,
                    a: key,
                    b: object,
                });
                let skip = self.emit(Op::JumpIfFalse {
                    condition: found,
                    offset: 0,
                });
                self.emit(Op::SetElem {
                    object,
                    key,
                    value: src,
                });
                ends.push(self.jump());
                self.patch(skip);
            }
            self.write_plain(name, src)?;
            self.patch_all(&ends);
            self.release(mark);
            return Ok(());
        }
        self.write_plain(name, src)
    }

    fn write_plain(&mut self, name: &'a str, src: u16) -> Compiled<()> {
        let place = self.resolve(name)?;
        let kind = match place {
            Place::Register { kind, .. }
            | Place::Cell { kind, .. }
            | Place::Up { kind, .. }
            | Place::UpValue { kind, .. } => Some(kind),
            Place::Global => None,
        };
        if kind == Some(VarKind::FunctionName) && !self.f().strict {
            return Ok(());
        }
        if matches!(
            kind,
            Some(VarKind::Const | VarKind::Using | VarKind::AwaitUsing | VarKind::FunctionName)
        ) {
            let atom = self.intern(name);
            self.emit(Op::ThrowConstAssign { name: atom.0 });
            return Ok(());
        }
        match place {
            Place::Register { reg, started, .. } => {
                if !started {
                    self.throw_tdz(name);
                } else if reg != src {
                    self.emit(Op::Move { dst: reg, src });
                }
            }
            Place::Cell { reg, .. } => {
                self.emit(Op::SetCell { cell: reg, src });
            }
            Place::Up { index, .. } => {
                self.emit(Op::SetUp { index, src });
            }
            // Only a parameter that nothing gives another value is captured as a value.
            Place::UpValue { .. } => {
                return self.error(format!("{name} was taken to be given no other value"), 0);
            }
            Place::Global => {
                let atom = self.intern(name);
                let cache = self.cache(atom)?;
                if self.f().strict {
                    self.emit(Op::SetGlobal { src, cache });
                } else {
                    self.emit(Op::SetGlobalSloppy { src, cache });
                }
            }
        }
        Ok(())
    }

    /// The register a variable lives in, if it's that simple and can be written there.
    fn writable_register(&mut self, name: &'a str) -> Compiled<Option<u16>> {
        if !self.with_chain(name).is_empty() {
            return Ok(None);
        }
        Ok(match self.resolve(name)? {
            Place::Register {
                reg,
                started: true,
                kind: VarKind::Var | VarKind::Let,
            } if !self.is_root(reg) => Some(reg),
            _ => None,
        })
    }

    /// A register that holds this variable already, if it's that simple.
    fn local_register(&mut self, name: &'a str) -> Compiled<Option<u16>> {
        if !self.with_chain(name).is_empty() {
            return Ok(None);
        }
        Ok(match self.resolve(name)? {
            // A component's props, or what's destructured from them, are read through `read_var`, which notes the read.
            Place::Register {
                reg, started: true, ..
            } if !self.is_root(reg) => Some(reg),
            _ => None,
        })
    }

    /// Whether a register is a component's props, or a name destructured from them, whose reads are noted.
    fn is_root(&self, reg: u16) -> bool {
        self.fns
            .last()
            .is_some_and(|state| state.roots.iter().any(|(root, _)| *root == reg) || state.param_roots.contains(&reg))
    }

    /// Where a read of `name` that's noted starts from, if it's one: a component's props or what's destructured from them,
    /// or a variable captured from a function around this one, in an app's own code.
    fn note_base(&mut self, name: &'a str) -> Compiled<Option<NoteBase<'a>>> {
        if !self.f().noting || !self.with_chain(name).is_empty() {
            return Ok(None);
        }
        Ok(match self.resolve(name)? {
            Place::Register { reg, .. } if self.f().param_roots.contains(&reg) => Some(NoteBase::Param { reg }),
            Place::Register { reg, .. } | Place::Cell { reg, .. } => self
                .f()
                .roots
                .iter()
                .find(|(root, _)| *root == reg)
                .map(|&(reg, key)| NoteBase::Own { reg, key }),
            Place::Up { index, .. } | Place::UpValue { index, .. }
                if !self.f().upvalues[index as usize].3 =>
            {
                Some(NoteBase::Up { index })
            }
            _ => None,
        })
    }

    /// Whether an expression is a read that's noted, or a chain of property reads starting with one.
    fn may_note(&mut self, expression: &'a Expr<'a>) -> Compiled<bool> {
        if !self.f().noting {
            return Ok(false);
        }
        let mut at = expression;
        loop {
            match at {
                Expr::Member {
                    object,
                    optional: false,
                    ..
                }
                | Expr::Index {
                    object,
                    optional: false,
                    ..
                } => at = object,
                Expr::Ident(name, _) => return Ok(self.note_base(name)?.is_some()),
                _ => return Ok(false),
            }
        }
    }

    /// Emits a Note of `kind`, with the keys after it.
    fn emit_note(&mut self, value: u16, root: u16, mut kind: u8, keys: &[Op]) {
        let mut prop = None;
        if std::mem::take(&mut self.itself_next) {
            kind |= NOTE_SAME_ITSELF;
            if std::mem::take(&mut self.f().arg_next) {
                kind |= NOTE_ARG;
                self.f().arg_notes += 1;
            }
        } else if let Some(name) = self.f().prop_next.take() {
            kind |= NOTE_ARG;
            prop = Some(Op::NoteName { atom: name.0 });
        }
        self.emit(Op::Note {
            value,
            root,
            count: (keys.len() + usize::from(prop.is_some())) as u8,
            kind,
        });
        for key in keys {
            self.emit(*key);
        }
        if let Some(prop) = prop {
            self.emit(prop);
        }
    }

    /// Notes that `name`, just read into `dst`, was read whole, if it's what's noted.
    fn note_bare(&mut self, name: &'a str, dst: u16) -> Compiled<()> {
        if dst == DISCARD {
            return Ok(());
        }
        match self.note_base(name)? {
            Some(NoteBase::Own { reg, key: Some(key) }) => {
                let atom = self.intern(key);
                self.emit_note(dst, reg, NOTE_OWN | NOTE_ALIASED, &[Op::NoteName { atom: atom.0 }]);
            }
            Some(NoteBase::Own { reg, key: None }) => self.emit_note(dst, reg, NOTE_OWN, &[]),
            Some(NoteBase::Up { index }) => self.emit_note(dst, index, NOTE_UP, &[]),
            Some(NoteBase::Param { reg }) => self.emit_note(dst, reg, NOTE_PARAM, &[]),
            None => {}
        }
        Ok(())
    }

    /// Compiles a chain of property reads that starts with what's noted, as `props.session.views[key]`, noting it whole,
    /// with what it gave in `dst`. With `this`, it's what's about to be called, and what it's read from goes there. Gives
    /// back whether it was such a chain: nothing's been emitted if it wasn't.
    fn noted_chain(&mut self, expression: &'a Expr<'a>, dst: u16, this: Option<u16>) -> Compiled<bool> {
        if !matches!(expression, Expr::Member { .. } | Expr::Index { .. }) {
            return Ok(false);
        }
        match self.noted_reads(expression, dst, this)? {
            Some(reads) => {
                self.finish_note(reads, None);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Emits the reads of what's noted, a name or a chain of property reads starting with one, into `dst`, and gives the
    /// Note to emit after them (see [`finish_note`](Self::finish_note)), or nothing if it isn't noted, with nothing
    /// emitted. With `this`, it's what's about to be called, and what it's read from goes there.
    fn noted_reads(&mut self, expression: &'a Expr<'a>, dst: u16, this: Option<u16>) -> Compiled<Option<NotedReads>> {
        if dst == DISCARD || !self.f().noting {
            return Ok(None);
        }
        let mut steps = Vec::new();
        let mut at = expression;
        let name = loop {
            match at {
                Expr::Member {
                    object,
                    name,
                    optional: false,
                } => {
                    steps.push(ChainStep::Name(name));
                    at = object;
                }
                Expr::Index {
                    object,
                    index,
                    optional: false,
                } => {
                    steps.push(ChainStep::Index(index));
                    at = object;
                }
                Expr::Ident(name, _) => break *name,
                _ => return Ok(None),
            }
        };
        if steps.len() > 30 || (this.is_some() && steps.is_empty()) {
            return Ok(None);
        }
        let Some(base) = self.note_base(name)? else {
            return Ok(None);
        };
        steps.reverse();
        let itself = std::mem::take(&mut self.itself_next);
        let mark = self.mark();
        let mut keys = Vec::with_capacity(steps.len() + 2);
        let (root, mut kind) = match base {
            NoteBase::Own { reg, key: Some(key) } => {
                let atom = self.intern(key);
                keys.push(Op::NoteName { atom: atom.0 });
                (reg, NOTE_OWN | NOTE_ALIASED)
            }
            NoteBase::Own { reg, key: None } => (reg, NOTE_OWN),
            NoteBase::Up { index } => (index, NOTE_UP),
            NoteBase::Param { reg } => (reg, NOTE_PARAM),
        };
        let mut prop = None;
        if itself {
            kind |= NOTE_SAME_ITSELF;
            if std::mem::take(&mut self.f().arg_next) {
                kind |= NOTE_ARG;
                self.f().arg_notes += 1;
            }
        } else if let Some(name) = self.f().prop_next.take() {
            kind |= NOTE_ARG;
            prop = Some(name);
        }
        let first = keys.len();
        if let Some(this) = this {
            kind |= NOTE_CALLED;
            keys.push(Op::NoteKey { reg: this });
        }
        let count = steps.len();
        if count == 0 {
            self.read_plain(name, dst)?;
        } else {
            let mut object = match this {
                Some(this) if count == 1 => this,
                _ => self.alloc()?,
            };
            self.read_plain(name, object)?;
            for (at, step) in steps.iter().enumerate() {
                let target = if at == count - 1 {
                    dst
                } else if at == count - 2
                    && let Some(this) = this
                {
                    this
                } else {
                    self.alloc()?
                };
                match *step {
                    ChainStep::Name(key) => {
                        self.get_named(target, object, key)?;
                        let atom = self.intern(key);
                        keys.push(Op::NoteName { atom: atom.0 });
                    }
                    ChainStep::Index(index) => {
                        let key = self.alloc()?;
                        self.expr(index, key)?;
                        self.emit(Op::GetElem {
                            dst: target,
                            object,
                            key,
                        });
                        keys.push(Op::NoteKey { reg: key });
                    }
                }
                object = target;
            }
        }
        if let Some(name) = prop {
            keys.push(Op::NoteName { atom: name.0 });
        }
        Ok(Some(NotedReads {
            value: dst,
            root,
            kind,
            keys,
            first,
            mark,
        }))
    }

    /// Emits the Note for reads [`noted_reads`](Self::noted_reads) emitted, which, with `compared`, says what was read
    /// was only compared with `===` or `!==` to what that register has.
    fn finish_note(&mut self, mut reads: NotedReads, compared: Option<u16>) {
        if let Some(other) = compared {
            reads.kind |= NOTE_COMPARED;
            reads.kind &= !NOTE_SAME_ITSELF;
            reads.keys.insert(reads.first, Op::NoteKey { reg: other });
        }
        self.emit(Op::Note {
            value: reads.value,
            root: reads.root,
            count: reads.keys.len() as u8,
            kind: reads.kind,
        });
        for key in &reads.keys {
            self.emit(*key);
        }
        self.release(reads.mark);
    }

    /// Works out a value that's read as itself, as what's compared with `===` or given to a call is.
    fn expr_itself(&mut self, expression: &'a Expr<'a>, dst: u16) -> Compiled<()> {
        if self.may_note(expression)? {
            self.itself_next = true;
            let done = self.expr_fresh(expression, dst);
            self.itself_next = false;
            return done;
        }
        self.expr_fresh(expression, dst)
    }

    /// The operands of `===` and the rest: what's noted is read as itself. Compared with `===` or `!==`, what's noted on
    /// one side only says whether it's the same as what the other side has, as in `active={id === focus}`, which is the
    /// same for each pane but the two the focus moves between, however it moved. Only one side is noted that way, as
    /// two would each take the other to be what it was.
    fn equality_operands(&mut self, strict: bool, a: &'a Expr<'a>, b: &'a Expr<'a>) -> Compiled<(u16, u16)> {
        let (left_noted, right_noted) = (self.may_note(a)?, self.may_note(b)?);
        if !(left_noted || right_noted) {
            return self.binary_operands(a, b);
        }
        let left = self.alloc()?;
        let right = self.alloc()?;
        if !strict {
            self.expr_itself(a, left)?;
            self.expr_itself(b, right)?;
        } else if right_noted {
            self.expr_itself(a, left)?;
            let reads = self.noted_reads(b, right, None)?.expect("may_note said so");
            self.finish_note(reads, Some(left));
        } else {
            // What's on the left is noted once what it's compared with is known.
            let reads = self.noted_reads(a, left, None)?.expect("may_note said so");
            self.expr_fresh(b, right)?;
            self.finish_note(reads, Some(right));
        }
        Ok((left, right))
    }

    fn read_this(&mut self, dst: u16) -> Compiled<()> {
        let state = self.f();
        let is_arrow = state.kind == FunctionKind::Arrow;
        let is_derived = state.kind == FunctionKind::DerivedConstructor;
        if let Some(cell) = state.this_cell {
            self.emit(Op::GetCell { dst, cell });
        } else if is_arrow {
            self.read_var("this", dst)?;
        } else {
            if is_derived {
                self.emit(Op::CheckThis { reg: 0 });
            }
            if dst != 0 {
                self.emit(Op::Move { dst, src: 0 });
            }
        }
        Ok(())
    }

    // Functions.

    /// Compiles a script, or code for `eval`, whose result is the value of its last statement.
    pub(crate) fn eval_program(mut self, program: &'a Program<'a>) -> Compiled<u32> {
        self.keeps_last_value = true;
        self.is_script = true;
        self.function(&program.function, None, false, false, false)
    }

    /// Compiles a whole file. A module is compiled as a generator that stops twice before running its code, as it's
    /// loaded in steps with the modules it imports: see `Vm::load_es_module`. What it imports and exports goes in
    /// `Modules::infos`, by its proto.
    pub(crate) fn program(mut self, program: &'a Program<'a>, is_module: bool) -> Compiled<u32> {
        let id = self.function(&program.function, None, false, is_module, false)?;
        if let Some(info) = self.module.take() {
            self.vm.modules.infos.insert(id, info);
        }
        Ok(id)
    }

    fn function(
        &mut self,
        function: &'a Function<'a>,
        hint: Option<Atom>,
        has_fields: bool,
        is_module: bool,
        wide: bool,
    ) -> Compiled<u32> {
        if let Some(id) = self.function_once(function, hint, has_fields, is_module, wide)? {
            return Ok(id);
        }
        match self.function_once(function, hint, has_fields, is_module, true)? {
            Some(id) => Ok(id),
            None => self.error("a jump in this function is too long", function.line),
        }
    }

    fn function_once(
        &mut self,
        function: &'a Function<'a>,
        hint: Option<Atom>,
        has_fields: bool,
        is_module: bool,
        wide: bool,
    ) -> Compiled<Option<u32>> {
        let wide = wide || self.fns.last().is_some_and(|parent| parent.wide);
        let params = function.params.len() as u32 + if is_module { 2 } else { 0 };
        let strict = is_module
            || self.class_depth > 0
            || self.fns.last().is_some_and(|parent| parent.strict)
            || (self.fns.is_empty() && self.outer_strict)
            || says_strict(function.body);
        let upvalues = if self.fns.is_empty() {
            self.outside.clone()
        } else {
            Vec::new()
        };
        let is_top = is_module || (self.fns.is_empty() && self.into.is_none());
        let noting = !is_top && crate::reads::reads_enabled() && self.app_file();
        let component = noting && self.is_component(function, hint);
        let keeps = component || (noting && self.is_hook(function, hint));
        self.fns.push(FnState {
            kind: function.kind,
            inner: function.inner_names,
            blocks: vec![Block::default()],
            code: Vec::new(),
            caches: Vec::new(),
            handlers: Vec::new(),
            lines: Vec::new(),
            next_reg: 1 + params,
            max_reg: 1 + params,
            clear: u32::MAX,
            upvalues,
            control: Vec::new(),
            chains: Vec::new(),
            next_target: 0,
            pending_labels: Vec::new(),
            wide,
            overflowed: false,
            has_fields,
            this_cell: None,
            by_value: Vec::new(),
            const_value_captures: false,
            strict,
            is_async_generator: function.is_async && function.is_generator,
            disposals: Vec::new(),
            steps: false,
            debug_vars: Vec::new(),
            noting,
            is_top,
            component,
            keeps,
            roots: Vec::new(),
            param_roots: Vec::new(),
            arg_next: false,
            arg_notes: 0,
            jsx_props_next: false,
            prop_next: None,
        });
        self.mark_line(function.line);
        if function.is_async && !function.is_generator {
            self.emit(Op::InitialYield);
        }
        if function.uses_this
            && !strict
            && function.kind != FunctionKind::Arrow
            && function.kind != FunctionKind::Constructor
            && function.kind != FunctionKind::DerivedConstructor
        {
            self.emit(Op::SloppyThis);
        }
        let result = self.function_body(function, is_module);
        if let Err(error) = result {
            self.fns.pop();
            return Err(error);
        }
        if self.f().steps {
            let end = self.here() as u32;
            let blocks = std::mem::take(&mut self.f().blocks);
            for block in &blocks {
                self.note_debug_vars(block, end);
            }
        }
        let mut state = self.fns.pop().expect("just pushed");
        if state.overflowed {
            return Ok(None);
        }
        let debug_vars = std::mem::take(&mut state.debug_vars);
        let (name, length, mut flags) = self.header(function, hint);
        if is_module {
            flags = IS_GENERATOR;
            // Whether it waits at its top, which makes what imports it wait for it.
            let waits = state.code.iter().any(|op| matches!(op, Op::Await { .. }));
            if let Some(info) = &mut self.module {
                info.is_async = waits;
            }
        }
        let start = self.vm.place_code(&state.code);
        let cache_base = self.vm.place_caches(&state.caches);
        let spans = (state.code.len() as u32, state.caches.len() as u32);
        let mut captures = Vec::with_capacity(state.upvalues.len());
        let mut capture_names = Vec::with_capacity(state.upvalues.len());
        for (name, capture, _, _) in &state.upvalues {
            captures.push(*capture);
            capture_names.push(self.vm.intern_str(name));
        }
        let app_file = self.app_file();
        let proto = Proto {
            start,
            cache_base,
            registers: state.max_reg as u16,
            params: params as u16,
            locals: (state.clear.min(state.max_reg) - 1 - params).min(u32::from(ALL_LOCALS)) as u8,
            length,
            flags,
            says: Proto::says_of(state.strict, function.no_memo, app_file),
            name,
            file: self.file,
            captures: captures.into_boxed_slice(),
            capture_names: capture_names.into_boxed_slice(),
            handlers: state.handlers.into_boxed_slice(),
            lines: state.lines.into_boxed_slice(),
        };
        // A function that was waiting goes where it was said it would be, which already knows where its text is.
        let id = if let (true, Some(id)) = (self.fns.is_empty(), self.into) {
            self.vm.replace_proto(id, proto, spans.0, spans.1);
            id
        } else {
            let id = self.vm.add_proto(proto, spans.0, spans.1);
            self.note_text(id, function);
            id
        };
        if !debug_vars.is_empty() {
            self.vm.debug_vars.insert(id, debug_vars.into_boxed_slice());
        }
        Ok(Some(id))
    }

    /// Keeps where a block's variables are, from where it starts to `end`, for a debugger, in a function it can stop in.
    fn note_debug_vars(&mut self, block: &Block<'a>, end: u32) {
        for var in &block.vars {
            if var.name.starts_with('%') || var.name.starts_with('*') {
                continue;
            }
            let name = self.vm.intern_str(var.name);
            self.f().debug_vars.push(crate::op::DebugVar {
                name,
                reg: var.reg,
                cell: var.cell,
                start: block.start,
                end,
            });
        }
    }

    /// A function's name, its `length`, and its flags, which are known without compiling it.
    fn header(&mut self, function: &'a Function<'a>, hint: Option<Atom>) -> (Atom, u8, u8) {
        let name = match function.name {
            Some(name) => self.intern(name),
            None => hint.unwrap_or(text::EMPTY_NAME),
        };
        let mut length = 0;
        for param in function.params.iter() {
            if param.default.is_some() {
                break;
            }
            length += 1;
        }
        let mut flags = 0;
        match function.kind {
            FunctionKind::Normal => {
                if !function.is_async && !function.is_generator {
                    flags |= IS_CONSTRUCTOR;
                }
            }
            FunctionKind::Arrow => flags |= IS_ARROW | IS_METHOD,
            FunctionKind::Method
            | FunctionKind::Getter
            | FunctionKind::Setter
            | FunctionKind::Fields => flags |= IS_METHOD,
            FunctionKind::Constructor => flags |= IS_CONSTRUCTOR | IS_CLASS,
            FunctionKind::DerivedConstructor => flags |= IS_CONSTRUCTOR | IS_CLASS | IS_DERIVED,
        }
        if function.is_async {
            flags |= IS_ASYNC;
        }
        if function.is_generator {
            flags |= IS_GENERATOR;
        }
        if function.rest.is_some()
            || function.uses_arguments
            || (function.kind != FunctionKind::Arrow && function.inner_names.contains("arguments"))
        {
            flags |= KEEPS_ARGS;
        }
        (name, length, flags)
    }

    /// Whether a function can wait until it's called to be compiled. Inside a `with`, what a name means depends on
    /// what's around it in a way that isn't kept.
    /// A field's computed key is read where the class is, but compiled in what sets up the field, so what it names
    /// isn't in that function's [`Function::outer_names`]: such a function is compiled with the class.
    fn can_wait(&self, function: &Function<'a>) -> bool {
        self.kept.is_some()
            && self.with_depth == 0
            && !function
                .body
                .iter()
                .any(|statement| matches!(statement, Stmt::Field(PropertyKey::Computed(_), _)))
    }

    /// Makes the proto for a function that's compiled when it's first called. What it captures is worked out from the
    /// names it may need from outside, as compiling it would: each that's in a cell here, or that this function sees
    /// from outside, is captured. Its mistakes are found now, as the language says they're found before anything runs.
    fn function_later(
        &mut self,
        function: &'a Function<'a>,
        hint: Option<Atom>,
        has_fields: bool,
    ) -> Compiled<u32> {
        if !self.checked {
            self.check_function(function)?;
        }
        let kept = self.kept.clone().expect("only kept code waits");
        let depth = self.fns.len() - 1;
        let mut captures = Vec::new();
        let mut capture_names = Vec::new();
        let mut outside = Vec::new();
        for &name in function.outer_names {
            let found = match Self::local_capture(&self.fns[depth], name) {
                Some(Ok((capture, kind))) => Some((capture, kind, self.fns[depth].is_top)),
                // Only what's captured can be seen from inside, so the function has something of its own by that name.
                Some(Err(())) => None,
                None => self.upvalue(depth, name)?.map(|(index, kind, by_value)| {
                    (
                        Capture {
                            from_register: false,
                            by_value,
                            index,
                        },
                        kind,
                        self.fns[depth].upvalues[index as usize].3,
                    )
                }),
            };
            if let Some((capture, kind, top)) = found {
                captures.push(capture);
                capture_names.push(self.vm.intern_str(name));
                outside.push((name, kind, top));
            }
        }
        let (name, length, flags) = self.header(function, hint);
        let outer_strict = self.class_depth > 0 || self.f().strict;
        let strict = outer_strict || says_strict(function.body);
        // SAFETY: what's borrowed from `kept` is only seen while the `Uncompiled` lasts, which holds on to `kept`, and
        // the source and the memory in that don't move.
        #[allow(unsafe_code)]
        let (function, outside, private_scopes) = unsafe {
            let outside: Vec<(&'static str, VarKind, bool)> = outside
                .into_iter()
                .map(|(name, kind, top)| (&*std::ptr::from_ref::<str>(name), kind, top))
                .collect();
            let private_scopes: Vec<Vec<(&'static str, Atom)>> = self
                .private_scopes
                .iter()
                .map(|scope| {
                    scope
                        .iter()
                        .map(|(name, atom)| (&*std::ptr::from_ref::<str>(name), *atom))
                        .collect()
                })
                .collect();
            (
                &*std::ptr::from_ref(function).cast::<Function<'static>>(),
                outside,
                private_scopes,
            )
        };
        let app_file = self.app_file();
        let waiting = Proto {
            start: NOT_COMPILED,
            cache_base: 0,
            registers: 0,
            params: function.params.len() as u16,
            locals: 0,
            length,
            flags: flags | KEEPS_ARGS,
            says: Proto::says_of(strict, function.no_memo, app_file),
            name,
            file: self.file,
            captures: captures.into_boxed_slice(),
            capture_names: capture_names.into_boxed_slice(),
            handlers: Box::new([]),
            lines: Box::new([]),
        };
        let id = self.vm.add_proto(waiting, 0, 0);
        self.note_text(id, function);
        self.vm.uncompiled.insert(
            id,
            Uncompiled {
                function,
                kept,
                outside: outside.into_boxed_slice(),
                private_scopes: private_scopes.into_boxed_slice(),
                outer_strict,
                has_fields,
            },
        );
        Ok(id)
    }

    /// Compiles a function that was waiting, into the proto it was given.
    fn compile_waiting(mut self, waiting: &'a Uncompiled, id: u32) -> Compiled<()> {
        let proto = &self.vm.protos[id as usize];
        let hint = proto.name;
        self.outside = waiting
            .outside
            .iter()
            .zip(proto.captures.iter())
            .map(|(&(name, kind, top), &capture)| (name, capture, kind, top))
            .collect();
        self.into = Some(id);
        self.outer_strict = waiting.outer_strict;
        self.private_scopes = waiting.private_scopes.to_vec();
        self.kept = Some(waiting.kept.clone());
        self.checked = true;
        self.source = self
            .vm
            .text_of_proto(id)
            .map_or(crate::source_text::NO_SOURCE, |(source, ..)| source);
        self.function(
            waiting.function,
            Some(hint),
            waiting.has_fields,
            false,
            false,
        )?;
        Ok(())
    }

    /// For a function of the app's that isn't a component, which of its parameters have what's read of them noted where what
    /// it was given was read, when that was noted, and which are read whole there, as a function inside sees them: see
    /// `NOTE_PARAMS`. Only a plain name that nothing gives another value, of a plain function that isn't async or a generator
    /// and doesn't look at its `arguments`, and only the first 16.
    fn note_params(&mut self, function: &'a Function<'a>, is_module: bool) -> Compiled<()> {
        if !self.f().noting
            || self.f().component
            || is_module
            || !matches!(function.kind, FunctionKind::Normal | FunctionKind::Arrow | FunctionKind::Method)
            || function.is_async
            || function.is_generator
            || function.uses_arguments
            || function.inner_names.contains("arguments")
            || self.with_depth > 0
        {
            return Ok(());
        }
        let names: Vec<&'a str> = function
            .params
            .iter()
            .take(16)
            .map(|param| match (&param.target, &param.default) {
                (Pattern::Ident(name, _), None) => *name,
                _ => "",
            })
            .collect();
        let mut rebinds = Rebinds::new(&names);
        rebinds.statements(function.body);
        let (mut noted, mut whole) = (0u16, 0u16);
        for (at, name) in names.iter().enumerate() {
            // Said twice, the last is the one that's read.
            if name.is_empty() || rebinds.hit[at] || names[at + 1..].contains(name) {
                continue;
            }
            let reg = 1 + at as u16;
            if function.inner_names.contains(name) {
                whole |= 1 << at;
            } else if matches!(Self::find_local(self.f(), name), Some(Place::Register { reg: found, .. }) if found == reg) {
                noted |= 1 << at;
                self.f().param_roots.push(reg);
            }
        }
        if noted != 0 || whole != 0 {
            self.emit_note(noted, whole, NOTE_PARAMS, &[]);
        }
        Ok(())
    }

    fn function_body(&mut self, function: &'a Function<'a>, is_module: bool) -> Compiled<()> {
        // A component says its frame is the one being rendered, before its props are put in a cell.
        if self.f().component {
            self.emit_note(1, 0, NOTE_ENTER, &[]);
        }
        if const_value_captures_enabled() {
            let mut dynamic = Rebinds::new(&[]);
            dynamic.statements(function.body);
            self.f().const_value_captures = !is_module && self.with_depth == 0 && !dynamic.opaque;
        }
        let is_arrow = function.kind == FunctionKind::Arrow;
        if !is_arrow {
            if function.inner_names.contains("this") {
                let reg = self.alloc()?;
                self.emit(Op::NewCell { dst: reg, src: 0 });
                let state = self.f();
                state.this_cell = Some(reg);
                state.blocks[0].vars.push(Var {
                    name: "this",
                    reg,
                    cell: true,
                    kind: VarKind::Var,
                    started: true,
                });
            }
            if function.inner_names.contains("new.target") {
                let reg = self.alloc()?;
                self.emit(Op::NewTarget { dst: reg });
                self.emit(Op::NewCell { dst: reg, src: reg });
                self.f().blocks[0].vars.push(Var {
                    name: "new.target",
                    reg,
                    cell: true,
                    kind: VarKind::Const,
                    started: true,
                });
            }
            if function.uses_arguments || function.inner_names.contains("arguments") {
                let reg = self.alloc()?;
                self.emit(Op::CreateArguments { dst: reg });
                let cell = function.inner_names.contains("arguments");
                if cell {
                    self.emit(Op::NewCell { dst: reg, src: reg });
                }
                self.f().blocks[0].vars.push(Var {
                    name: "arguments",
                    reg,
                    cell,
                    kind: VarKind::Var,
                    started: true,
                });
            }
        }
        // A function can use its own name, unless something closer has that name.
        if let (FunctionKind::Normal, true, Some(name)) =
            (function.kind, function.uses_own_name, function.name)
        {
            let reg = self.alloc()?;
            self.emit(Op::LoadCallee { dst: reg });
            let cell = function.inner_names.contains(name);
            if cell {
                self.emit(Op::NewCell { dst: reg, src: reg });
            }
            self.f().blocks[0].vars.push(Var {
                name,
                reg,
                cell,
                kind: VarKind::FunctionName,
                started: true,
            });
        }
        if function.kind == FunctionKind::Constructor && self.f().has_fields {
            self.emit(Op::RunFields { this: 0 });
        }
        self.f().blocks.push(Block::default());
        let mut next_param = 1u16;
        if is_module {
            for name in ["%bind", "%import"] {
                let cell = function.inner_names.contains(name);
                if cell {
                    self.emit(Op::NewCell {
                        dst: next_param,
                        src: next_param,
                    });
                }
                self.current_block().vars.push(Var {
                    name,
                    reg: next_param,
                    cell,
                    kind: VarKind::Const,
                    started: true,
                });
                next_param += 1;
            }
        }
        // A parameter that a function inside this one sees, and that nothing gives another value, can be captured as
        // its value. Only when every parameter is a plain name, with no default, so that each has its value before
        // anything that could capture it runs, and none is said twice.
        let plain_params = function.rest.is_none()
            && function
                .params
                .iter()
                .all(|param| param.default.is_none() && matches!(param.target, Pattern::Ident(..)));
        if plain_params {
            let captured: Vec<&'a str> = function
                .params
                .iter()
                .filter_map(|param| match param.target {
                    Pattern::Ident(name, _) if function.inner_names.contains(name) => Some(name),
                    _ => None,
                })
                .collect();
            if !captured.is_empty() {
                let mut rebinds = Rebinds::new(&captured);
                rebinds.statements(function.body);
                let said = |name: &str| {
                    function
                        .params
                        .iter()
                        .filter(|param| matches!(param.target, Pattern::Ident(other, _) if other == name))
                        .count()
                };
                // A component's props are in a cell when something sees them, which is how what reads them is told.
                let props = match function.params.first().map(|param| &param.target) {
                    Some(Pattern::Ident(name, _)) if self.f().component => Some(*name),
                    _ => None,
                };
                let by_value: Vec<&'a str> = captured
                    .iter()
                    .zip(&rebinds.hit)
                    .filter(|&(name, hit)| !hit && said(name) == 1 && Some(*name) != props)
                    .map(|(name, _)| *name)
                    .collect();
                self.f().by_value = by_value;
            }
        }
        // Parameters are named first, so that their defaults can see the ones before them.
        for (number, param) in function.params.iter().enumerate() {
            let reg = next_param;
            next_param += 1;
            let props = number == 0 && self.f().component;
            if let Some(default) = &param.default {
                let skip = self.emit(Op::JumpIfNotUndefined {
                    condition: reg,
                    offset: 0,
                });
                self.expr_named(default, reg, Self::pattern_hint(&param.target))?;
                self.patch(skip);
            }
            match &param.target {
                Pattern::Ident(name, _) => {
                    let cell =
                        function.inner_names.contains(name) && !self.f().by_value.contains(name);
                    if cell {
                        self.emit(Op::NewCell { dst: reg, src: reg });
                    }
                    self.current_block().vars.push(Var {
                        name,
                        reg,
                        cell,
                        kind: VarKind::Let,
                        started: true,
                    });
                    if props {
                        self.f().roots.push((reg, None));
                        if cell {
                            self.emit_note(reg, 0, NOTE_CELL, &[]);
                        }
                    }
                }
                pattern => {
                    let mut names = Vec::new();
                    pattern_names(pattern, &mut names);
                    for name in &names {
                        self.declare(name, VarKind::Let, false)?;
                    }
                    match pattern {
                        Pattern::Object { items, rest } if props => {
                            // What's given another value somewhere isn't the props' any more.
                            let mut rebinds = Rebinds::new(&names);
                            rebinds.statements(function.body);
                            let given: Vec<&'a str> = names
                                .iter()
                                .zip(&rebinds.hit)
                                .filter(|(_, hit)| **hit)
                                .map(|(name, _)| *name)
                                .collect();
                            self.bind_props(pattern, items, rest.is_some(), reg, &given)?;
                        }
                        _ => self.bind(pattern, reg, true)?,
                    }
                }
            }
        }
        self.note_params(function, is_module)?;
        if let Some(rest) = &function.rest {
            let mark_names = {
                let mut names = Vec::new();
                pattern_names(rest, &mut names);
                names
            };
            for name in mark_names {
                self.declare(name, VarKind::Let, false)?;
            }
            let value = self.alloc()?;
            self.emit(Op::Rest {
                dst: value,
                start: function.params.len() as u16,
            });
            self.bind(rest, value, true)?;
        }
        // A generator stops here: what's wrong with its arguments comes out when it's called, not when it's first run.
        if function.is_generator || is_module {
            self.emit(Op::InitialYield);
        }
        // A module's functions are declared as `let`s are, so two with one name, or one and a `var`, is a mistake there.
        self.check_redeclarations(function.body, !is_module)?;
        let module_names = if is_module {
            Some(self.module_cells(function.body)?)
        } else {
            None
        };
        let mut vars = Vec::new();
        collect_vars(function.body, &mut vars);
        let is_script_top = self.is_script && self.fns.len() == 1;
        for name in vars {
            if is_script_top {
                let atom = self.intern(name);
                self.emit(Op::DeclareGlobalVar { name: atom.0 });
            } else {
                self.declare(name, VarKind::Var, true)?;
            }
        }
        if is_script_top {
            self.declare_lexicals_except_functions(function.body)?;
            self.hoist_global_functions(function.body)?;
        } else if let Some((locals, imported)) = &module_names {
            self.declare_lexicals(function.body)?;
            self.module_steps(function.body, locals, imported)?;
        } else {
            self.declare_block(function.body)?;
        }
        // Everything from here on is declared in a block of its own or is a temporary, and is written first.
        let state = self.f();
        state.clear = state.next_reg;
        if self.keeps_last_value && self.fns.len() == 1 {
            if let Some((Stmt::Expr(last), rest)) = function.body.split_last() {
                self.statements(rest)?;
                let value = self.expr_reg(last)?;
                return self.emit_return(Some(value));
            }
        }
        self.statements(function.body)?;
        self.emit_return(None)
    }

    fn current_block(&mut self) -> &mut Block<'a> {
        self.f().blocks.last_mut().expect("there's always a block")
    }

    fn make_closure(
        &mut self,
        function: &'a Function<'a>,
        dst: u16,
        hint: Option<Atom>,
        has_fields: bool,
    ) -> Compiled<()> {
        self.check_depth()?;
        let proto = if self.can_wait(function) {
            self.function_later(function, hint, has_fields)?
        } else {
            self.function(function, hint, has_fields, false, false)?
        };
        self.emit(Op::Closure { dst, proto });
        Ok(())
    }

    /// Makes the variables that this block's statements declare, and its functions, which can be called from the start.
    fn declare_block(&mut self, statements: &'a [Stmt<'a>]) -> Compiled<()> {
        self.const_value_candidates(statements);
        self.declare_lexicals(statements)?;
        self.hoist_functions(statements)
    }

    /// A lexical block is entered once before its statements execute. Before a
    /// simple const initializer completes, any closure mentioning it needs a
    /// live cell (TDZ/self recursion/hoisted declarations). Afterwards its value
    /// cannot change, so copying it at closure creation is exact. Capture scans
    /// deliberately over-approximate branches, classes and nested functions.
    fn const_value_candidates(&mut self, statements: &'a [Stmt<'a>]) {
        if !self.f().const_value_captures {
            return;
        }
        let names: Vec<&'a str> = statements
            .iter()
            .flat_map(|statement| match statement {
                Stmt::Var(VarKind::Const, declarations) => declarations
                    .iter()
                    .filter_map(|(pattern, init)| match pattern {
                        Pattern::Ident(name, _) if init.is_some() => Some(*name),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .filter(|name| self.f().inner.contains(name))
            .collect();
        if names.is_empty() {
            return;
        }
        let mut before = Rebinds::captures(&names);
        for statement in statements {
            if let Stmt::Function(function) = statement {
                before.function(function);
            }
        }
        let mut values = Vec::new();
        for statement in statements {
            if let Stmt::Var(VarKind::Const, declarations) = statement {
                for (pattern, init) in declarations.iter() {
                    if let Some(init) = init {
                        before.expr(init);
                    }
                    before.pattern(pattern);
                    if let Pattern::Ident(name, _) = pattern {
                        if names
                            .iter()
                            .position(|wanted| wanted == name)
                            .is_some_and(|at| !before.hit[at])
                        {
                            values.push(*name);
                        }
                    }
                }
            } else {
                before.statement(statement);
            }
        }
        self.current_block().const_values = values;
    }

    fn declare_lexicals_except_functions(&mut self, statements: &'a [Stmt<'a>]) -> Compiled<()> {
        for statement in statements {
            if !matches!(statement, Stmt::Function(_)) {
                self.declare_lexicals(std::slice::from_ref(statement))?;
            }
        }
        Ok(())
    }

    fn hoist_global_functions(&mut self, statements: &'a [Stmt<'a>]) -> Compiled<()> {
        for statement in statements {
            if let Stmt::Function(function) = statement {
                let Some(name) = function.name else {
                    continue;
                };
                let mark = self.mark();
                let value = self.alloc()?;
                self.make_closure(function, value, None, false)?;
                let atom = self.intern(name);
                self.emit(Op::DeclareGlobalFunction {
                    src: value,
                    name: atom.0,
                });
                self.release(mark);
            }
        }
        Ok(())
    }

    /// Finds names declared twice in a way that isn't allowed. `is_function_top` is for a function's own body, where a
    /// function declaration acts like a `var`.
    /// What the head of a loop declares with `let`, `const` or `using` can't be a `var` in its body as well.
    fn check_loop_vars(&self, declared: &[&'a str], body: &Stmt<'a>) -> Compiled<()> {
        let mut vars = Vec::new();
        collect_vars_one(body, &mut vars);
        match declared.iter().find(|name| vars.contains(name)) {
            Some(found) => self.error(format!("{found:?} has already been declared"), 0),
            None => Ok(()),
        }
    }

    fn check_redeclarations(&self, statements: &[Stmt<'a>], is_function_top: bool) -> Compiled<()> {
        let mut lexical: Vec<&'a str> = Vec::new();
        let mut functions: Vec<&'a str> = Vec::new();
        let duplicate = |name: &str| -> Compiled<()> {
            self.error(format!("{name:?} has already been declared"), 0)
        };
        for statement in statements {
            match statement {
                Stmt::Var(kind, declarations) if *kind != VarKind::Var => {
                    for (pattern, _) in declarations.iter() {
                        let mut names = Vec::new();
                        pattern_names(pattern, &mut names);
                        for name in names {
                            if lexical.contains(&name) || functions.contains(&name) {
                                return duplicate(name);
                            }
                            lexical.push(name);
                        }
                    }
                }
                Stmt::Class(class) => {
                    if let Some(name) = class.name {
                        if lexical.contains(&name) || functions.contains(&name) {
                            return duplicate(name);
                        }
                        lexical.push(name);
                    }
                }
                Stmt::Function(function) => {
                    if let Some(name) = function.name {
                        if lexical.contains(&name) {
                            return duplicate(name);
                        }
                        // Two plain functions of the same name are all right in a block, for old code.
                        if !is_function_top
                            && functions.contains(&name)
                            && (function.is_async || function.is_generator)
                        {
                            return duplicate(name);
                        }
                        functions.push(name);
                    }
                }
                _ => {}
            }
        }
        if lexical.is_empty() && (is_function_top || functions.is_empty()) {
            return Ok(());
        }
        let mut vars = Vec::new();
        collect_vars(statements, &mut vars);
        for name in vars {
            if lexical.contains(&name) || (!is_function_top && functions.contains(&name)) {
                return duplicate(name);
            }
        }
        Ok(())
    }

    fn declare_lexicals(&mut self, statements: &'a [Stmt<'a>]) -> Compiled<()> {
        for statement in statements {
            match statement {
                Stmt::Var(kind, declarations) if *kind != VarKind::Var => {
                    for (pattern, _) in declarations.iter() {
                        let mut names = Vec::new();
                        pattern_names(pattern, &mut names);
                        for name in names {
                            self.declare(name, *kind, false)?;
                        }
                    }
                }
                Stmt::Class(class) => {
                    if let Some(name) = class.name {
                        self.declare(name, VarKind::Let, false)?;
                    }
                }
                Stmt::Function(function) => {
                    if let Some(name) = function.name {
                        self.declare(name, VarKind::Let, true)?;
                    }
                }
                Stmt::Import(parts) => {
                    if let Some(name) = parts.default {
                        self.declare(name, VarKind::Const, false)?;
                    }
                    if let Some(name) = parts.all {
                        self.declare(name, VarKind::Const, false)?;
                    }
                    for (_, local) in parts.named {
                        self.declare(local, VarKind::Const, false)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn hoist_functions(&mut self, statements: &'a [Stmt<'a>]) -> Compiled<()> {
        for statement in statements {
            if let Stmt::Function(function) = statement {
                let Some(name) = function.name else {
                    continue;
                };
                let mark = self.mark();
                let value = self.alloc()?;
                self.make_closure(function, value, None, false)?;
                self.init_var(name, value)?;
                self.release(mark);
            }
        }
        Ok(())
    }

    fn open_block(&mut self) -> u32 {
        let mark = self.mark();
        let start = self.here() as u32;
        self.f().blocks.push(Block {
            vars: Vec::new(),
            const_values: Vec::new(),
            start,
        });
        mark
    }

    fn close_block(&mut self, mark: u32) {
        if let Some(block) = self.f().blocks.pop()
            && self.f().steps
        {
            let end = self.here() as u32;
            self.note_debug_vars(&block, end);
        }
        self.release(mark);
    }

    // Statements.

    fn statements(&mut self, statements: &'a [Stmt<'a>]) -> Compiled<()> {
        // What's from the first `using` on is disposed of however the block's left, and what's before it is as usual.
        let first_using = statements
            .iter()
            .position(|statement| matches!(statement, Stmt::Var(kind, _) if kind.is_using()));
        let (plain, disposed) = statements.split_at(first_using.unwrap_or(statements.len()));
        for statement in plain {
            self.statement(statement)?;
        }
        if !disposed.is_empty() {
            let is_await = disposed
                .iter()
                .any(|statement| matches!(statement, Stmt::Var(VarKind::AwaitUsing, _)));
            self.disposing(is_await, &mut |this| {
                for statement in disposed {
                    this.statement(statement)?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    fn scoped_statements(&mut self, statements: &'a [Stmt<'a>]) -> Compiled<()> {
        self.check_redeclarations(statements, false)?;
        let mark = self.open_block();
        self.declare_block(statements)?;
        self.statements(statements)?;
        self.close_block(mark);
        Ok(())
    }

    fn take_labels(&mut self) -> Vec<&'a str> {
        std::mem::take(&mut self.f().pending_labels)
    }

    fn push_target(&mut self, kind: TargetKind, iter: Option<u16>) -> u32 {
        let labels = self.take_labels();
        let state = self.f();
        state.next_target += 1;
        let id = state.next_target;
        state.control.push(Control::Target {
            id,
            labels,
            kind,
            breaks: Vec::new(),
            continues: Vec::new(),
            iter,
            is_await: false,
        });
        id
    }

    /// Finishes a target, and gives back its `break`s and `continue`s, to be aimed.
    fn pop_target(&mut self) -> (Vec<usize>, Vec<usize>) {
        match self.f().control.pop() {
            Some(Control::Target {
                breaks, continues, ..
            }) => (breaks, continues),
            _ => unreachable!("targets and finallys are popped in the order they were pushed"),
        }
    }

    fn statement(&mut self, statement: &'a Stmt<'a>) -> Compiled<()> {
        self.check_depth()?;
        let mark = self.mark();
        match statement {
            Stmt::Empty | Stmt::Function(_) => {}
            Stmt::Line(line) => {
                self.mark_line(*line);
                self.f().steps = true;
                self.emit(Op::Step { line: *line });
            }
            Stmt::Expr(expression) => self.expr_discard(expression)?,
            Stmt::Var(kind, declarations) => self.var_statement(*kind, declarations)?,
            Stmt::Class(class) => {
                let value = self.alloc()?;
                self.class(class, value, None)?;
                if let Some(name) = class.name {
                    self.init_var(name, value)?;
                }
            }
            Stmt::Block(body) => self.scoped_statements(body)?,
            Stmt::If(test, then, otherwise) => {
                let mut to_else = Vec::new();
                self.test(test, false, &mut to_else)?;
                self.statement(then)?;
                if let Some(otherwise) = otherwise {
                    let end = self.jump();
                    self.patch_all(&to_else);
                    self.statement(otherwise)?;
                    self.patch(end);
                } else {
                    self.patch_all(&to_else);
                }
            }
            Stmt::For(parts) => self.for_statement(
                parts.init.as_ref(),
                parts.test.as_ref(),
                parts.update.as_ref(),
                parts.body,
            )?,
            Stmt::ForOf(parts) => self.for_each(
                &parts.target,
                &parts.iterable,
                parts.body,
                false,
                parts.is_await,
            )?,
            Stmt::ForIn(parts) => {
                self.for_each(&parts.target, &parts.object, parts.body, true, false)?;
            }
            Stmt::With(object, body) => {
                let scope = self.open_block();
                let name = WITH_NAMES[self.with_depth];
                self.declare(name, VarKind::Let, false)?;
                let held = self.alloc()?;
                self.expr(object, held)?;
                self.emit(Op::ToObject {
                    dst: held,
                    src: held,
                });
                self.init_var(name, held)?;
                self.with_depth += 1;
                let result = self.statement(body);
                self.with_depth -= 1;
                result?;
                self.close_block(scope);
            }
            Stmt::While(test, body) => {
                let labels_id = self.push_target(TargetKind::Loop, None);
                let _ = labels_id;
                if self.is_simple_loop_test(test) {
                    let entry = self.jump();
                    let top = self.here();
                    self.statement(body)?;
                    let (breaks, continues) = self.pop_target();
                    self.patch_all(&continues);
                    self.patch(entry);
                    self.loop_back_if(test, top)?;
                    self.patch_all(&breaks);
                } else {
                    let top = self.here();
                    let mut exits = Vec::new();
                    self.test(test, false, &mut exits)?;
                    self.statement(body)?;
                    let (breaks, continues) = self.pop_target();
                    self.patch_all(&continues);
                    self.jump_back(top);
                    self.patch_all(&exits);
                    self.patch_all(&breaks);
                }
            }
            Stmt::DoWhile(body, test) => {
                self.push_target(TargetKind::Loop, None);
                let top = self.here();
                self.statement(body)?;
                let (breaks, continues) = self.pop_target();
                self.patch_all(&continues);
                if self.is_simple_loop_test(test) {
                    self.loop_back_if(test, top)?;
                } else {
                    let mut exits = Vec::new();
                    self.test(test, false, &mut exits)?;
                    self.jump_back(top);
                    self.patch_all(&exits);
                }
                self.patch_all(&breaks);
            }
            Stmt::Return(Some(Expr::Conditional(test, then, otherwise))) => {
                let mark = self.mark();
                let mut other = Vec::new();
                self.test(test, false, &mut other)?;
                let src = self.expr_reg(then)?;
                self.do_exit(Exit::Return, Some(src))?;
                self.release(mark);
                self.patch_all(&other);
                let src = self.expr_reg(otherwise)?;
                self.do_exit(Exit::Return, Some(src))?;
                self.release(mark);
            }
            Stmt::Return(value) => {
                let src = match value {
                    Some(value) => Some(self.expr_reg(value)?),
                    None => None,
                };
                self.do_exit(Exit::Return, src)?;
            }
            Stmt::Break(label) => {
                let id = self.find_target(*label, false)?;
                self.do_exit(Exit::Break(id), None)?;
            }
            Stmt::Continue(label) => {
                let id = self.find_target(*label, true)?;
                self.do_exit(Exit::Continue(id), None)?;
            }
            Stmt::Throw(value) => {
                let src = self.expr_reg(value)?;
                self.emit(Op::Throw { src });
            }
            Stmt::Try(parts) => self.try_statement(
                parts.block,
                parts.param.as_ref(),
                parts.handler,
                parts.finalizer,
            )?,
            Stmt::Switch(value, cases) => self.switch_statement(value, cases)?,
            Stmt::Field(key, value) => {
                let this = self.alloc()?;
                self.read_this(this)?;
                let held = self.alloc()?;
                let hint = self.key_hint(key);
                match value {
                    Some(value) => self.expr_named(value, held, hint)?,
                    None => {
                        self.emit(Op::LoadUndefined { dst: held });
                    }
                }
                // What it's kept under isn't what a function in it is called, for a private name.
                let plain_atom = match key {
                    PropertyKey::Name(name) => Some(self.atom_of_key(name)),
                    PropertyKey::String(_) => hint,
                    _ => None,
                };
                let plain_atom = plain_atom.filter(|atom| self.vm.atoms.index_of(*atom).is_none());
                if let Some(atom) = plain_atom {
                    let cache = self.cache(atom)?;
                    self.emit(Op::DefineField {
                        object: this,
                        value: held,
                        cache,
                    });
                } else {
                    let key_reg = self.alloc()?;
                    self.property_key(key, key_reg)?;
                    self.emit(Op::DefineElem {
                        object: this,
                        key: key_reg,
                        value: held,
                    });
                }
            }
            Stmt::Labeled(name, body) => {
                self.f().pending_labels.push(name);
                match **body {
                    Stmt::For { .. }
                    | Stmt::ForIn { .. }
                    | Stmt::ForOf { .. }
                    | Stmt::While(..)
                    | Stmt::DoWhile(..)
                    | Stmt::Labeled(..) => self.statement(body)?,
                    _ => {
                        self.push_target(TargetKind::Block, None);
                        self.statement(body)?;
                        let (breaks, _) = self.pop_target();
                        self.patch_all(&breaks);
                    }
                }
            }
            // What these say was done in the module's first steps.
            Stmt::Import(_) | Stmt::Export(_) | Stmt::ExportFrom(_) => {}
            Stmt::ExportDefault(value) => {
                let held = self.alloc()?;
                self.expr_named(value, held, Some(text::DEFAULT))?;
                self.init_var(DEFAULT_LOCAL, held)?;
            }
        }
        self.release(mark);
        Ok(())
    }

    /// Gives each name a module imports, and each of its own that it exports, a cell before anything else is declared,
    /// and works out what it imports and exports, for what loads it. A name that's imported is the very cell the module
    /// it's from has for it, which goes in its register in the module's second step (see [`Self::module_steps`]), so it's
    /// always what that module has in it now, and reading it costs what reading any variable a function holds on to does.
    /// Gives back the module's own exported names, by the numbers it tells `%bind` about them by, and each imported name
    /// with the number of its import.
    #[allow(clippy::type_complexity)]
    fn module_cells(
        &mut self,
        body: &'a [Stmt<'a>],
    ) -> Compiled<(Vec<&'a str>, Vec<(&'a str, usize)>)> {
        use crate::modules::{ExportFrom, ModuleInfo};
        let mut info = ModuleInfo::default();
        let mut imported: Vec<(&'a str, usize)> = Vec::new();
        let mut locals: Vec<&'a str> = Vec::new();
        for statement in body {
            match statement {
                Stmt::Import(parts) => {
                    let from = info.request(parts.source);
                    if let Some(local) = parts.default {
                        imported.push((local, info.imports.len()));
                        info.imports.push((from, Some("default".to_owned())));
                    }
                    if let Some(local) = parts.all {
                        imported.push((local, info.imports.len()));
                        info.imports.push((from, None));
                    }
                    for (name, local) in parts.named {
                        imported.push((local, info.imports.len()));
                        info.imports.push((from, Some((*name).to_owned())));
                    }
                }
                Stmt::ExportFrom(parts) => {
                    let from = info.request(parts.source);
                    match parts.named {
                        None => info.stars.push(from),
                        Some(list) => {
                            for (name, exported) in list {
                                let source = ExportFrom::Import(from, Some((*name).to_owned()));
                                info.exports.push(((*exported).to_owned(), source));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for statement in body {
            let list: &[(&'a str, &'a str)] = match statement {
                Stmt::Export(ExportName::List(list)) => list,
                Stmt::ExportDefault(_) => &[(DEFAULT_LOCAL, "default")],
                _ => continue,
            };
            for (local, exported) in list {
                // What's imported and exported again is the other module's, as if it said `export { x } from`.
                let source = match imported.iter().find(|(name, _)| name == local) {
                    Some((_, entry)) => {
                        let (from, name) = info.imports[*entry].clone();
                        ExportFrom::Import(from, name)
                    }
                    None => {
                        let at = match locals.iter().position(|name| name == local) {
                            Some(at) => at,
                            None => {
                                locals.push(local);
                                locals.len() - 1
                            }
                        };
                        ExportFrom::Local(at)
                    }
                };
                info.exports.push(((*exported).to_owned(), source));
            }
        }
        for (at, (name, _)) in info.exports.iter().enumerate() {
            if info.exports[..at].iter().any(|(other, _)| other == name) {
                return self.error(format!("Duplicate export of '{name}'"), 0);
            }
        }
        info.locals = locals.iter().map(|name| (*name).to_owned()).collect();
        // A module that only passes on what others export runs no code of its own: what it imports and exports again
        // is all there is to it.
        info.only_reexports = locals.is_empty()
            && body.iter().all(|statement| {
                matches!(
                    statement,
                    Stmt::Import(_) | Stmt::ExportFrom(_) | Stmt::Export(ExportName::List(_)) | Stmt::Line(_)
                ) || matches!(statement, Stmt::Block(inner) if inner.is_empty())
            });
        let mut vars = Vec::new();
        collect_vars(body, &mut vars);
        for (local, _) in &imported {
            let declared_here = top_declaration(body, local).is_some() || vars.contains(local);
            if declared_here
                || self
                    .current_block()
                    .vars
                    .iter()
                    .any(|var| var.name == *local)
            {
                return self.error(format!("Identifier '{local}' has already been declared"), 0);
            }
            let reg = self.alloc()?;
            self.emit(Op::LoadUndefined { dst: reg });
            self.current_block().vars.push(Var {
                name: local,
                reg,
                cell: true,
                kind: VarKind::Const,
                started: true,
            });
        }
        for name in &locals {
            let (kind, started) = if *name == DEFAULT_LOCAL {
                (VarKind::Let, false)
            } else if let Some(found) = top_declaration(body, name) {
                found
            } else if vars.contains(name) {
                (VarKind::Var, true)
            } else {
                return self.error(format!("Export '{name}' is not defined in module"), 0);
            };
            let reg = self.alloc()?;
            if started {
                self.emit(Op::LoadUndefined { dst: reg });
            } else {
                self.emit(Op::LoadEmpty { dst: reg });
            }
            self.emit(Op::NewCell { dst: reg, src: reg });
            self.current_block().vars.push(Var {
                name,
                reg,
                cell: true,
                kind,
                started,
            });
        }
        self.module = Some(info);
        Ok((locals, imported))
    }

    /// A module's first two steps, which what loads it runs for every module it loads at once before running any of
    /// their code, as modules can import each other. The first hands over the cells of what it exports. The second
    /// takes the cells of what it imports, which every module has handed over by then, and makes its functions, which
    /// can be called from then on.
    fn module_steps(
        &mut self,
        body: &'a [Stmt<'a>],
        locals: &[&'a str],
        imported: &[(&'a str, usize)],
    ) -> Compiled<()> {
        let mark = self.mark();
        let f = self.alloc()?;
        let this = self.alloc()?;
        let first = self.alloc()?;
        let second = self.alloc()?;
        for (at, name) in locals.iter().enumerate() {
            let Place::Cell { reg, .. } = self.resolve(name)? else {
                return self.error(format!("{name} should be in a cell"), 0);
            };
            self.read_var("%bind", f)?;
            self.emit(Op::LoadUndefined { dst: this });
            self.emit(Op::Move {
                dst: first,
                src: reg,
            });
            self.load_number(second, at as f64);
            self.emit(Op::Call { dst: f, f, argc: 2 });
        }
        self.emit(Op::LoadUndefined { dst: first });
        self.emit(Op::Yield {
            dst: first,
            src: first,
        });
        for (name, entry) in imported {
            let Place::Cell { reg, .. } = self.resolve(name)? else {
                return self.error(format!("{name} should be in a cell"), 0);
            };
            self.read_var("%import", f)?;
            self.emit(Op::LoadUndefined { dst: this });
            self.load_number(first, *entry as f64);
            self.emit(Op::Call { dst: f, f, argc: 1 });
            self.emit(Op::Move { dst: reg, src: f });
        }
        self.release(mark);
        self.hoist_functions(body)?;
        let mark = self.mark();
        let held = self.alloc()?;
        self.emit(Op::LoadUndefined { dst: held });
        self.emit(Op::Yield {
            dst: held,
            src: held,
        });
        self.release(mark);
        Ok(())
    }

    fn var_statement(
        &mut self,
        kind: VarKind,
        declarations: &'a [(Pattern<'a>, Option<Expr<'a>>)],
    ) -> Compiled<()> {
        if kind.is_using() {
            return self.using_declarations(kind, declarations);
        }
        for (pattern, value) in declarations {
            let mark = self.mark();
            match (pattern, value) {
                (Pattern::Ident(name, line), Some(value)) => {
                    self.mark_line(*line);
                    let atom = self.atom_of_key(name);
                    let local = self.resolve(name)?;
                    let direct = match local {
                        Place::Register { reg, .. }
                            if kind != VarKind::Var || writes_last(value) =>
                        {
                            Some(reg)
                        }
                        _ => None,
                    };
                    if let Some(reg) = direct {
                        // A `let` or a `const` can't be read until after this, so nothing can see it being built.
                        if kind == VarKind::Var {
                            self.expr_named(value, reg, Some(atom))?;
                        } else {
                            match value {
                                Expr::Array(_) | Expr::Object(_) => self.expr_fresh(value, reg)?,
                                _ => self.expr_named(value, reg, Some(atom))?,
                            }
                        }
                        self.set_started(name);
                    } else {
                        let temp = self.alloc()?;
                        self.expr_named(value, temp, Some(atom))?;
                        if kind == VarKind::Var {
                            self.write_var(name, temp)?;
                        } else {
                            self.init_var(name, temp)?;
                        }
                    }
                }
                (Pattern::Ident(name, _), None) => {
                    if kind != VarKind::Var {
                        let temp = self.alloc()?;
                        self.emit(Op::LoadUndefined { dst: temp });
                        self.init_var(name, temp)?;
                    }
                }
                (pattern @ Pattern::Object { items, rest }, Some(Expr::Ident(from, _)))
                    if kind == VarKind::Const && self.props_named(from)?.is_some() =>
                {
                    let source = self.props_named(from)?.expect("just found");
                    self.bind_props(pattern, items, rest.is_some(), source, &[])?;
                }
                (pattern, Some(value)) => {
                    let source = self.expr_reg(value)?;
                    self.bind(pattern, source, kind != VarKind::Var)?;
                }
                (_, None) => return self.error("a pattern needs a value", 0),
            }
            self.release(mark);
        }
        Ok(())
    }

    fn for_statement(
        &mut self,
        init: Option<&'a ForInit<'a>>,
        test: Option<&'a Expr<'a>>,
        update: Option<&'a Expr<'a>>,
        body: &'a Stmt<'a>,
    ) -> Compiled<()> {
        // What a `using` declares in the head lasts for the whole loop.
        match init {
            Some(ForInit::Var(kind, _)) if kind.is_using() => self
                .disposing(*kind == VarKind::AwaitUsing, &mut |this| {
                    this.for_loop(init, test, update, body)
                }),
            _ => self.for_loop(init, test, update, body),
        }
    }

    fn for_loop(
        &mut self,
        init: Option<&'a ForInit<'a>>,
        test: Option<&'a Expr<'a>>,
        update: Option<&'a Expr<'a>>,
        body: &'a Stmt<'a>,
    ) -> Compiled<()> {
        let block_mark = self.open_block();
        let labels = self.take_labels();
        let mut per_turn = Vec::new();
        match init {
            Some(ForInit::Var(kind, declarations)) => {
                if *kind != VarKind::Var {
                    let mut names = Vec::new();
                    for (pattern, _) in declarations.iter() {
                        pattern_names(pattern, &mut names);
                    }
                    self.check_loop_vars(&names, body)?;
                    for name in &names {
                        self.declare(name, *kind, false)?;
                    }
                    for name in names {
                        if let Place::Cell { reg, .. } = self.resolve(name)? {
                            per_turn.push(reg);
                        }
                    }
                }
                self.var_statement(*kind, declarations)?;
            }
            Some(ForInit::Expr(expression)) => self.expr_discard(expression)?,
            None => {}
        }
        self.f().pending_labels = labels;
        self.push_target(TargetKind::Loop, None);
        // With a plain comparison, the test goes at the bottom, as one instruction that also jumps back.
        let counted = self.counted_loop(test, update)?;
        let at_bottom = test.filter(|test| counted.is_none() && self.is_simple_loop_test(test));
        let entry = at_bottom.map(|_| self.jump());
        let mut exits = Vec::new();
        // A counting loop's last instruction does the step as well, so the first test has to be separate.
        if let (Some(test), Some(_)) = (test, counted) {
            self.test(test, false, &mut exits)?;
        }
        let top = self.here();
        if let (Some(test), None, None) = (test, at_bottom, counted) {
            self.test(test, false, &mut exits)?;
        }
        self.statement(body)?;
        let (breaks, continues) = self.pop_target();
        self.patch_all(&continues);
        for reg in per_turn {
            self.emit(Op::CopyCell { reg });
        }
        if let Some(make) = counted {
            let at = self.emit(make);
            self.patch_to(at, top);
        } else {
            if let Some(update) = update {
                self.expr_discard(update)?;
            }
            if let (Some(test), Some(entry)) = (at_bottom, entry) {
                self.patch(entry);
                self.loop_back_if(test, top)?;
            } else {
                self.jump_back(top);
            }
        }
        self.patch_all(&exits);
        self.patch_all(&breaks);
        self.close_block(block_mark);
        Ok(())
    }

    /// The one instruction that ends a loop like `for (...; i < n; i++)`, if this is one, where the variable and the
    /// limit are in registers so nothing else has to be worked out.
    fn counted_loop(
        &mut self,
        test: Option<&'a Expr<'a>>,
        update: Option<&'a Expr<'a>>,
    ) -> Compiled<Option<Op>> {
        let (Some(Expr::Binary(BinaryOp::Lt, left, right)), Some(update)) = (test, update) else {
            return Ok(None);
        };
        let (
            Expr::Update {
                increment: true,
                target,
                ..
            },
            Expr::Ident(counter, _),
        ) = (update, *left)
        else {
            return Ok(None);
        };
        let Expr::Ident(updated, _) = *target else {
            return Ok(None);
        };
        if counter != updated || self.f().wide || !self.with_chain(counter).is_empty() {
            return Ok(None);
        }
        let i = match self.resolve(counter)? {
            Place::Register {
                reg,
                kind: VarKind::Var | VarKind::Let,
                started: true,
            } => reg,
            _ => return Ok(None),
        };
        let offset = 0;
        Ok(match *right {
            Expr::Number(number) => {
                let limit = *number as i16;
                (f64::from(limit) == *number && (limit != 0 || number.is_sign_positive()))
                    .then_some(Op::IncLoopLtInt { i, limit, offset })
            }
            Expr::Ident(name, _) => {
                self.local_register(name)?
                    .map(|limit| Op::IncLoopLt { i, limit, offset })
            }
            Expr::Member {
                object: Expr::Ident(name, _),
                name: "length",
                optional: false,
            } => self
                .local_register(name)?
                .map(|object| Op::IncLoopLtLength { i, object, offset }),
            _ => None,
        })
    }

    fn is_simple_loop_test(&mut self, test: &Expr<'a>) -> bool {
        !self.f().wide
            && match test {
                Expr::Binary(BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge, ..) => true,
                // `undefined` and `null` have tests of their own, which a jump back after them is as quick as.
                Expr::Binary(BinaryOp::StrictNe, _, b) => match &**b {
                    Expr::Number(_) => true,
                    Expr::Ident(name, _) => *name != "undefined",
                    _ => false,
                },
                _ => false,
            }
    }

    /// Goes back to the start of a loop if a plain comparison holds.
    fn loop_back_if(&mut self, test: &'a Expr<'a>, target: usize) -> Compiled<()> {
        let Expr::Binary(op, a, b) = test else {
            unreachable!("only a comparison comes here");
        };
        let mark = self.mark();
        let small = match &**b {
            Expr::Number(number) => {
                let value = *number as i16;
                (f64::from(value) == *number && (value != 0 || number.is_sign_positive()))
                    .then_some(value)
            }
            _ => None,
        };
        let at = if let Some(value) = small {
            let a = self.expr_reg(a)?;
            self.emit(match op {
                BinaryOp::StrictNe => Op::LoopStrictNeInt {
                    a,
                    value,
                    offset: 0,
                },
                BinaryOp::Lt => Op::LoopLtInt {
                    a,
                    value,
                    offset: 0,
                },
                BinaryOp::Le => Op::LoopLeInt {
                    a,
                    value,
                    offset: 0,
                },
                BinaryOp::Gt => Op::LoopGtInt {
                    a,
                    value,
                    offset: 0,
                },
                _ => Op::LoopGeInt {
                    a,
                    value,
                    offset: 0,
                },
            })
        } else {
            let (a, b) = self.binary_operands(a, b)?;
            self.emit(match op {
                BinaryOp::StrictNe => Op::LoopStrictNe { a, b, offset: 0 },
                BinaryOp::Lt => Op::LoopLt { a, b, offset: 0 },
                BinaryOp::Le => Op::LoopLe { a, b, offset: 0 },
                BinaryOp::Gt => Op::LoopGt { a, b, offset: 0 },
                _ => Op::LoopGe { a, b, offset: 0 },
            })
        };
        self.patch_to(at, target);
        self.release(mark);
        Ok(())
    }

    fn for_each(
        &mut self,
        target: &'a ForTarget<'a>,
        source: &'a Expr<'a>,
        body: &'a Stmt<'a>,
        over_keys: bool,
        is_await: bool,
    ) -> Compiled<()> {
        let mark = self.mark();
        let labels = self.take_labels();
        let iter = self.alloc()?;
        let _position = self.alloc()?;
        let held = self.alloc()?;
        if over_keys {
            let object = self.expr_reg(source)?;
            self.emit(Op::ForInKeys {
                dst: held,
                source: object,
            });
        } else {
            self.expr(source, held)?;
        }
        if is_await {
            self.emit(Op::GetAsyncIterator { iter, source: held });
        } else {
            self.emit(Op::GetIterator { iter, source: held });
        }
        self.f().pending_labels = labels;
        self.push_target(TargetKind::Loop, Some(iter));
        if let Some(Control::Target {
            is_await: waits, ..
        }) = self.f().control.last_mut()
        {
            *waits = is_await;
        }
        let top = self.here();
        let value = self.alloc()?;
        let exit = if is_await {
            self.emit(Op::AsyncIterCall { dst: value, iter });
            self.emit(Op::Await {
                dst: value,
                src: value,
            });
            let exit = self.iter_result(value);
            // What a plain iterator gives may be a promise, which `for await` waits for too.
            self.emit(Op::Await {
                dst: value,
                src: value,
            });
            exit
        } else {
            self.iter_next(value, iter)?
        };
        let start = self.here();
        let block_mark = self.open_block();
        match target {
            ForTarget::Declare(kind, pattern) => {
                if *kind != VarKind::Var {
                    let mut names = Vec::new();
                    pattern_names(pattern, &mut names);
                    self.check_loop_vars(&names, body)?;
                    for name in names {
                        self.declare(name, *kind, false)?;
                    }
                }
                // What a `using` declares is given its value once that's on the list of what to dispose of, below.
                let mut bound = kind.is_using();
                if let (true, Pattern::Ident(name, _)) =
                    (*kind != VarKind::Var && !kind.is_using(), pattern)
                {
                    // The next value can go straight in the variable, if it's just in a register.
                    if let Place::Register { reg, .. } = self.resolve(name)? {
                        if let Op::IterNext { value, .. } = &mut self.f().code[exit] {
                            *value = reg;
                            bound = true;
                        }
                        if bound {
                            self.set_started(name);
                        }
                    }
                }
                if !bound {
                    self.bind(pattern, value, *kind != VarKind::Var)?;
                }
            }
            ForTarget::Assign(pattern) => self.bind(pattern, value, false)?,
        }
        match target {
            // Each value is disposed of when its turn of the loop ends.
            ForTarget::Declare(kind, Pattern::Ident(name, _)) if kind.is_using() => {
                let kind = *kind;
                self.disposing(kind == VarKind::AwaitUsing, &mut |this| {
                    let added = this.add_disposable(kind, value)?;
                    this.init_var(name, added)?;
                    this.statement(body)
                })?;
            }
            _ => self.statement(body)?,
        }
        self.close_block(block_mark);
        let (breaks, continues) = self.pop_target();
        let end = self.here();
        self.patch_all(&continues);
        self.jump_back(top);
        // An error in the body closes the iterator too, and then carries on.
        let handler_reg = self.alloc()?;
        let target_pc = self.here();
        self.f().handlers.push(Handler {
            start: start as u32,
            end: end as u32,
            target: target_pc as u32,
            reg: handler_reg,
            is_catch: false,
        });
        self.emit(Op::IterCloseQuietly { iter });
        self.emit(Op::Throw { src: handler_reg });
        self.patch(exit);
        self.patch_all(&breaks);
        self.release(mark);
        Ok(())
    }

    /// Reads what `next` gave, and gives what to aim at where to go when there are no more.
    fn iter_result(&mut self, value: u16) -> usize {
        if self.f().wide {
            self.emit(Op::IterResult { value, offset: 1 });
            self.emit(Op::Jump { offset: 1 });
            return self.jump();
        }
        self.emit(Op::IterResult { value, offset: 0 })
    }

    /// Gets the next value of an iterator, and gives what to aim at where to go when there are no more.
    fn iter_next(&mut self, value: u16, iter: u16) -> Compiled<usize> {
        if self.f().wide {
            self.emit(Op::IterNext {
                value,
                iter,
                offset: 1,
            });
            self.emit(Op::Jump { offset: 1 });
            return Ok(self.jump());
        }
        Ok(self.emit(Op::IterNext {
            value,
            iter,
            offset: 0,
        }))
    }

    fn find_target(&mut self, label: Option<&str>, is_continue: bool) -> Compiled<u32> {
        for control in self.f().control.iter().rev() {
            if let Control::Target {
                id, labels, kind, ..
            } = control
            {
                let matches = match label {
                    Some(label) => labels.contains(&label),
                    None => match kind {
                        TargetKind::Loop => true,
                        TargetKind::Switch => !is_continue,
                        TargetKind::Block => false,
                    },
                };
                if matches {
                    return Ok(*id);
                }
            }
        }
        self.error(
            if is_continue {
                "continue isn't inside a loop"
            } else {
                "break isn't inside a loop"
            },
            0,
        )
    }

    fn emit_return(&mut self, src: Option<u16>) -> Compiled<()> {
        let state = self.f();
        if state.kind == FunctionKind::DerivedConstructor {
            if let Some(cell) = state.this_cell {
                self.emit(Op::GetCell { dst: 0, cell });
            }
        }
        match src {
            Some(src) => self.emit(Op::Return { src }),
            // What sets up a class's fields gives back the object it set them up on, so that the interpreter can
            // call it in place and have its answer go where that object already is (see `Vm::run_fields_here`).
            None if state_kind_is_fields(self.f().kind) => self.emit(Op::Return { src: 0 }),
            None => self.emit(Op::ReturnUndefined),
        };
        Ok(())
    }

    /// Closes the iterator of a `for of` that's being left early. A `for await` calls its `return` and waits for what
    /// that gives, as the language says, so what it does to finish has happened before the code after the loop runs.
    fn close_iterator(&mut self, iter: u16, is_await: bool) -> Compiled<()> {
        if !is_await {
            self.emit(Op::IterClose { iter });
            return Ok(());
        }
        let mark = self.mark();
        let f = self.alloc()?;
        let this = self.alloc()?;
        self.get_named(f, iter, "return")?;
        let skip = self.emit(Op::JumpIfNullish {
            condition: f,
            offset: 0,
        });
        self.emit(Op::Move {
            dst: this,
            src: iter,
        });
        self.emit(Op::Call { dst: f, f, argc: 0 });
        self.emit(Op::Await { dst: f, src: f });
        self.patch(skip);
        self.release(mark);
        Ok(())
    }

    /// Leaves by a `return`, `break` or `continue`, going through what's in the way.
    fn do_exit(&mut self, exit: Exit, value: Option<u16>) -> Compiled<()> {
        let count = self.f().control.len();
        for at in (0..count).rev() {
            enum Step {
                Finally(u16, u16),
                Done(bool),
                Crossing(Option<u16>, bool),
            }
            let step = match &self.f().control[at] {
                Control::Finally {
                    kind_reg,
                    value_reg,
                    ..
                } => Step::Finally(*kind_reg, *value_reg),
                Control::Target {
                    id, iter, is_await, ..
                } => match exit {
                    Exit::Break(target) if target == *id => {
                        let (iter, is_await) = (*iter, *is_await);
                        if let Some(iter) = iter {
                            self.close_iterator(iter, is_await)?;
                        }
                        Step::Done(true)
                    }
                    Exit::Continue(target) if target == *id => Step::Done(false),
                    _ => Step::Crossing(*iter, *is_await),
                },
            };
            match step {
                Step::Finally(kind_reg, value_reg) => {
                    if exit == Exit::Return {
                        match value {
                            Some(value) if value != value_reg => {
                                self.emit(Op::Move {
                                    dst: value_reg,
                                    src: value,
                                });
                            }
                            Some(_) => {}
                            None => {
                                self.emit(Op::LoadUndefined { dst: value_reg });
                            }
                        }
                    }
                    let kind = {
                        let Control::Finally { exits, .. } = &mut self.f().control[at] else {
                            unreachable!("it was just seen to be a finally");
                        };
                        let index = match exits.iter().position(|found| *found == exit) {
                            Some(index) => index,
                            None => {
                                exits.push(exit);
                                exits.len() - 1
                            }
                        };
                        index as i32 + 2
                    };
                    self.emit(Op::LoadInt {
                        dst: kind_reg,
                        value: kind,
                    });
                    let entry = self.jump();
                    let Control::Finally { entries, .. } = &mut self.f().control[at] else {
                        unreachable!("it was just seen to be a finally");
                    };
                    entries.push(entry);
                    return Ok(());
                }
                Step::Done(is_break) => {
                    let jump = self.jump();
                    let Control::Target {
                        breaks, continues, ..
                    } = &mut self.f().control[at]
                    else {
                        unreachable!("it was just seen to be a target");
                    };
                    if is_break {
                        breaks.push(jump);
                    } else {
                        continues.push(jump);
                    }
                    return Ok(());
                }
                Step::Crossing(iter, is_await) => {
                    if let Some(iter) = iter {
                        self.close_iterator(iter, is_await)?;
                    }
                }
            }
        }
        self.emit_return(value)
    }

    fn try_statement(
        &mut self,
        block: &'a [Stmt<'a>],
        param: Option<&'a Pattern<'a>>,
        handler: Option<&'a [Stmt<'a>]>,
        finalizer: Option<&'a [Stmt<'a>]>,
    ) -> Compiled<()> {
        let registers = if finalizer.is_some() {
            Some(self.begin_finally()?)
        } else {
            None
        };
        let start = self.here();
        self.scoped_statements(block)?;
        if let Some(handler) = handler {
            let block_end = self.here();
            let skip = self.jump();
            let target = self.here();
            let mark = self.open_block();
            let caught = self.alloc()?;
            self.f().handlers.push(Handler {
                start: start as u32,
                end: block_end as u32,
                target: target as u32,
                reg: caught,
                is_catch: true,
            });
            if let Some(param) = param {
                let mut names = Vec::new();
                pattern_names(param, &mut names);
                for statement in handler {
                    let mut declared = Vec::new();
                    match statement {
                        Stmt::Var(kind, declarations) if *kind != VarKind::Var => {
                            for (pattern, _) in declarations.iter() {
                                pattern_names(pattern, &mut declared);
                            }
                        }
                        Stmt::Class(class) => declared.extend(class.name),
                        Stmt::Function(function) => declared.extend(function.name),
                        _ => {}
                    }
                    if let Some(found) = declared.iter().find(|found| names.contains(found)) {
                        return self.error(format!("{found:?} has already been declared"), 0);
                    }
                }
                for name in names {
                    self.declare(name, VarKind::Let, false)?;
                }
                self.bind(param, caught, true)?;
            }
            self.scoped_statements(handler)?;
            self.close_block(mark);
            self.patch(skip);
        }
        let Some(finalizer) = finalizer else {
            return Ok(());
        };
        let (kind_reg, value_reg) = registers.expect("there's a finalizer");
        self.finish_finally(start, kind_reg, value_reg, &mut |this| {
            this.scoped_statements(finalizer)
        })
    }

    /// Begins what a `finally` runs after, however it's left. See [`Self::finish_finally`].
    fn begin_finally(&mut self) -> Compiled<(u16, u16)> {
        let kind_reg = self.alloc()?;
        let value_reg = self.alloc()?;
        self.f().control.push(Control::Finally {
            kind_reg,
            value_reg,
            entries: Vec::new(),
            exits: Vec::new(),
        });
        Ok((kind_reg, value_reg))
    }

    /// Ends what [`Self::begin_finally`] began at `start`, with `finalizer` to run first whichever way it was left. That
    /// can read how in `kind_reg`: 0 for getting to the end, 1 for a throw, of what's in `value_reg`, and from 2 on for a
    /// `return`, `break` or `continue`, with what's returned in `value_reg`.
    fn finish_finally(
        &mut self,
        start: usize,
        kind_reg: u16,
        value_reg: u16,
        finalizer: &mut dyn FnMut(&mut Self) -> Compiled<()>,
    ) -> Compiled<()> {
        let Some(Control::Finally { entries, exits, .. }) = self.f().control.pop() else {
            unreachable!("targets and finallys are popped in the order they were pushed");
        };
        let protected_end = self.here();
        self.emit(Op::LoadInt {
            dst: kind_reg,
            value: 0,
        });
        let to_body = self.jump();
        let target = self.here();
        self.f().handlers.push(Handler {
            start: start as u32,
            end: protected_end as u32,
            target: target as u32,
            reg: value_reg,
            is_catch: false,
        });
        self.emit(Op::LoadInt {
            dst: kind_reg,
            value: 1,
        });
        self.patch(to_body);
        self.patch_all(&entries);
        finalizer(self)?;
        // Now carry on with whatever got this here.
        let mark = self.mark();
        let number = self.alloc()?;
        self.emit(Op::LoadInt {
            dst: number,
            value: 1,
        });
        let compare = self.alloc()?;
        self.emit(Op::StrictEq {
            dst: compare,
            a: kind_reg,
            b: number,
        });
        let skip = self.emit(Op::JumpIfFalse {
            condition: compare,
            offset: 0,
        });
        self.emit(Op::Throw { src: value_reg });
        self.patch(skip);
        for (index, exit) in exits.iter().enumerate() {
            self.emit(Op::LoadInt {
                dst: number,
                value: index as i32 + 2,
            });
            self.emit(Op::StrictEq {
                dst: compare,
                a: kind_reg,
                b: number,
            });
            let skip = self.emit(Op::JumpIfFalse {
                condition: compare,
                offset: 0,
            });
            let value = (*exit == Exit::Return).then_some(value_reg);
            self.do_exit(*exit, value)?;
            self.patch(skip);
        }
        self.release(mark);
        Ok(())
    }

    /// Compiles `body` so that what its `using` declarations add is disposed of, the last first, however it's left, as a
    /// `finally` would. With `is_await`, each is waited for, and the language says when there's still a wait at the end.
    fn disposing(
        &mut self,
        is_await: bool,
        body: &mut dyn FnMut(&mut Self) -> Compiled<()>,
    ) -> Compiled<()> {
        let mark = self.mark();
        let list = self.alloc()?;
        self.emit(Op::NewArray {
            dst: list,
            capacity: 0,
        });
        let (kind_reg, value_reg) = self.begin_finally()?;
        self.f().disposals.push(list);
        let start = self.here();
        body(self)?;
        self.f().disposals.pop();
        self.finish_finally(start, kind_reg, value_reg, &mut |this| {
            let mark = this.mark();
            if is_await {
                this.dispose_awaiting(list, kind_reg, value_reg)?;
            } else {
                this.call_disposal(disposal::DISPOSE, &[list, kind_reg, value_reg])?;
            }
            this.release(mark);
            Ok(())
        })?;
        self.release(mark);
        Ok(())
    }

    /// Disposes of what's on `list` a step at a time, waiting for what each step gives, and noting it if that fails.
    fn dispose_awaiting(&mut self, list: u16, kind_reg: u16, value_reg: u16) -> Compiled<()> {
        let top = self.here();
        let given = self.call_disposal(disposal::STEP, &[list, kind_reg, value_reg])?;
        let done = self.alloc()?;
        self.emit(Op::StrictEq {
            dst: done,
            a: given,
            b: list,
        });
        let to_end = self.emit(Op::JumpIfTrue {
            condition: done,
            offset: 0,
        });
        let waits = self.here();
        self.emit(Op::Await {
            dst: given,
            src: given,
        });
        let waited = self.here();
        self.jump_back(top);
        let caught = self.alloc()?;
        let target = self.here();
        self.f().handlers.push(Handler {
            start: waits as u32,
            end: waited as u32,
            target: target as u32,
            reg: caught,
            is_catch: true,
        });
        self.call_disposal(disposal::FAILED, &[list, caught])?;
        self.jump_back(top);
        self.patch(to_end);
        Ok(())
    }

    /// `using a = b`: `b` goes on the list of what the block disposes of, and then in `a`, which is a `const`.
    fn using_declarations(
        &mut self,
        kind: VarKind,
        declarations: &'a [(Pattern<'a>, Option<Expr<'a>>)],
    ) -> Compiled<()> {
        for (pattern, value) in declarations {
            let (Pattern::Ident(name, line), Some(value)) = (pattern, value) else {
                return self.error("a using declaration needs a name and a value", 0);
            };
            let mark = self.mark();
            self.mark_line(*line);
            let atom = self.atom_of_key(name);
            let given = self.alloc()?;
            self.expr_named(value, given, Some(atom))?;
            let added = self.add_disposable(kind, given)?;
            self.init_var(name, added)?;
            self.release(mark);
        }
        Ok(())
    }

    /// Puts what's in `value` on the innermost list of what to dispose of, and gives the register it's then in.
    fn add_disposable(&mut self, kind: VarKind, value: u16) -> Compiled<u16> {
        let Some(&list) = self.f().disposals.last() else {
            return self.error("a using declaration has to be in a block", 0);
        };
        let mode = if kind == VarKind::AwaitUsing {
            disposal::ADD_AWAITED
        } else {
            disposal::ADD
        };
        self.call_disposal(mode, &[list, value])
    }

    /// Calls the native that does the work of `using` (see [`disposal`]), with `mode` and then `args`, and gives the
    /// register what it gives back is in.
    fn call_disposal(&mut self, mode: i32, args: &[u16]) -> Compiled<u16> {
        let function = self.vm.disposal_function();
        let f = self.alloc()?;
        let this = self.alloc()?;
        let first = self.alloc()?;
        self.load_const(f, function);
        self.emit(Op::LoadUndefined { dst: this });
        self.emit(Op::LoadInt {
            dst: first,
            value: mode,
        });
        for &arg in args {
            let reg = self.alloc()?;
            self.emit(Op::Move { dst: reg, src: arg });
        }
        self.emit(Op::Call {
            dst: f,
            f,
            argc: 1 + args.len() as u16,
        });
        Ok(f)
    }

    fn switch_statement(
        &mut self,
        value: &'a Expr<'a>,
        cases: &'a [SwitchCase<'a>],
    ) -> Compiled<()> {
        let held = self.alloc()?;
        self.expr(value, held)?;
        let labels = self.take_labels();
        // Every case is in one block, so what's declared in one can't be declared again in another.
        let all: Vec<Stmt<'a>> = cases
            .iter()
            .flat_map(|case| case.body.iter().copied())
            .collect();
        self.check_redeclarations(&all, false)?;
        let mark = self.open_block();
        for case in cases {
            self.declare_lexicals(case.body)?;
        }
        for case in cases {
            self.hoist_functions(case.body)?;
        }
        self.f().pending_labels = labels;
        self.push_target(TargetKind::Switch, None);
        let mut to_body = Vec::with_capacity(cases.len());
        let compare = self.alloc()?;
        for case in cases {
            if let Some(test) = &case.test {
                let inner = self.mark();
                let tested = self.expr_reg(test)?;
                self.emit(Op::StrictEq {
                    dst: compare,
                    a: held,
                    b: tested,
                });
                to_body.push(Some(self.emit(Op::JumpIfTrue {
                    condition: compare,
                    offset: 0,
                })));
                self.release(inner);
            } else {
                to_body.push(None);
            }
        }
        let otherwise = self.jump();
        let mut default_at = None;
        for (index, case) in cases.iter().enumerate() {
            match to_body[index] {
                Some(jump) => self.patch(jump),
                None => default_at = Some(self.here()),
            }
            self.statements(case.body)?;
        }
        match default_at {
            Some(target) => self.patch_to(otherwise, target),
            None => self.patch(otherwise),
        }
        let (breaks, _) = self.pop_target();
        self.patch_all(&breaks);
        self.close_block(mark);
        Ok(())
    }

    // Patterns.

    fn pattern_hint(pattern: &Pattern<'_>) -> Option<Atom> {
        let _ = pattern;
        None
    }

    /// Puts the value that's in `source` where a pattern says. `declaring` means it's the variables' first value.
    /// Whether a component is being compiled, at the top of its body, and `name` is its props, which are in a register: if so,
    /// that register, with the props in it (read out of their cell if they're in one).
    fn props_named(&mut self, name: &'a str) -> Compiled<Option<u16>> {
        if !self.f().component || self.f().blocks.len() != 2 || !self.with_chain(name).is_empty() {
            return Ok(None);
        }
        Ok(match Self::find_local(self.f(), name) {
            Some(Place::Register { reg, started: true, .. }) if self.f().roots.contains(&(reg, None)) => Some(reg),
            Some(Place::Cell { reg, .. }) if self.f().roots.contains(&(reg, None)) => {
                let value = self.alloc()?;
                self.emit(Op::GetCell { dst: value, cell: reg });
                Some(value)
            }
            _ => None,
        })
    }

    /// Destructures a component's props, in `source`: each plain name, as `session` in `{ session }`, is the props'
    /// `session` whenever it's read, which is what's noted (see `reads.rs`), unless it's one in `given`, which something
    /// gives another value. What has a default, or is destructured further, is read whole here, and a rest reads all of them.
    fn bind_props(
        &mut self,
        pattern: &'a Pattern<'a>,
        items: &'a [(PropertyKey<'a>, PatternItem<'a>)],
        has_rest: bool,
        source: u16,
        given: &[&'a str],
    ) -> Compiled<()> {
        if has_rest {
            // What's left is all of them, as far as this can tell.
            let whole = self.alloc()?;
            self.emit(Op::Move { dst: whole, src: source });
            self.emit_note(whole, source, NOTE_OWN, &[]);
            return self.bind(pattern, source, true);
        }
        self.emit(Op::CheckObjectCoercible { src: source });
        for (key, item) in items {
            let inner = self.mark();
            let value = self.alloc()?;
            match (key, &item.target, &item.default) {
                (PropertyKey::Name(key), Pattern::Ident(name, _), None) if !given.contains(name) => {
                    self.get_named(value, source, key)?;
                    self.init_var(name, value)?;
                    let atom = self.intern(key);
                    match Self::find_local(self.f(), name) {
                        Some(Place::Cell { reg, .. }) => {
                            self.f().roots.push((reg, Some(*key)));
                            self.emit_note(reg, 0, NOTE_CELL | NOTE_ALIASED, &[Op::NoteName { atom: atom.0 }]);
                        }
                        Some(Place::Register { reg, .. }) => self.f().roots.push((reg, Some(*key))),
                        _ => {}
                    }
                }
                (PropertyKey::Name(key), ..) => {
                    self.get_named(value, source, key)?;
                    let atom = self.intern(key);
                    self.emit_note(value, source, NOTE_OWN, &[Op::NoteName { atom: atom.0 }]);
                    self.bind_item(item, value, true)?;
                }
                _ => {
                    let key_reg = self.alloc()?;
                    self.property_key(key, key_reg)?;
                    self.emit(Op::GetElem {
                        dst: value,
                        object: source,
                        key: key_reg,
                    });
                    self.emit_note(value, source, NOTE_OWN, &[Op::NoteKey { reg: key_reg }]);
                    self.bind_item(item, value, true)?;
                }
            }
            self.release(inner);
        }
        Ok(())
    }

    fn bind(&mut self, pattern: &'a Pattern<'a>, source: u16, declaring: bool) -> Compiled<()> {
        self.check_depth()?;
        match pattern {
            Pattern::Ident(name, _) => {
                if declaring {
                    self.init_var(name, source)
                } else {
                    self.write_var(name, source)
                }
            }
            Pattern::Expr(target) => self.assign_to_expr(target, source),
            Pattern::Array { items, rest } => {
                let mark = self.mark();
                let iter = self.alloc()?;
                let _position = self.alloc()?;
                self.emit(Op::GetIterator { iter, source });
                for item in items.iter() {
                    let inner = self.mark();
                    let value = self.alloc()?;
                    // Whether or not it's finished, this carries on to the next instruction.
                    self.emit(Op::IterNext {
                        value,
                        iter,
                        offset: 0,
                    });
                    if let Some(item) = item {
                        self.bind_item(item, value, declaring)?;
                    }
                    self.release(inner);
                }
                if let Some(rest) = rest {
                    let array = self.alloc()?;
                    self.emit(Op::NewArray {
                        dst: array,
                        capacity: 0,
                    });
                    let value = self.alloc()?;
                    let top = self.here();
                    let done = self.emit(Op::IterNext {
                        value,
                        iter,
                        offset: 0,
                    });
                    self.emit(Op::ArrayPush { array, value });
                    self.jump_back(top);
                    self.patch(done);
                    self.bind(rest, array, declaring)?;
                }
                self.emit(Op::IterClose { iter });
                self.release(mark);
                Ok(())
            }
            Pattern::Object { items, rest } => {
                let mark = self.mark();
                self.emit(Op::CheckObjectCoercible { src: source });
                let keys = if rest.is_some() {
                    let keys = self.alloc()?;
                    self.emit(Op::NewArray {
                        dst: keys,
                        capacity: items.len() as u32,
                    });
                    Some(keys)
                } else {
                    None
                };
                for (key, item) in items.iter() {
                    let inner = self.mark();
                    let value = self.alloc()?;
                    match key {
                        PropertyKey::Name(name) => {
                            // With a rest, the names it's not to have are listed too, but each value is still read
                            // through a place that remembers where it was, rather than by a name worked out each
                            // time.
                            if let Some(keys) = keys {
                                let key_reg = self.alloc()?;
                                self.property_key(key, key_reg)?;
                                self.emit(Op::ArrayPush {
                                    array: keys,
                                    value: key_reg,
                                });
                            }
                            self.get_named(value, source, name)?;
                        }
                        _ => {
                            let key_reg = self.alloc()?;
                            self.property_key(key, key_reg)?;
                            if let Some(keys) = keys {
                                self.emit(Op::ArrayPush {
                                    array: keys,
                                    value: key_reg,
                                });
                            }
                            self.emit(Op::GetElem {
                                dst: value,
                                object: source,
                                key: key_reg,
                            });
                        }
                    }
                    self.bind_item(item, value, declaring)?;
                    self.release(inner);
                }
                if let (Some(rest), Some(keys)) = (rest, keys) {
                    let value = self.alloc()?;
                    self.emit(Op::ObjectRest {
                        dst: value,
                        source,
                        keys,
                    });
                    self.bind(rest, value, declaring)?;
                }
                self.release(mark);
                Ok(())
            }
        }
    }

    fn bind_item(
        &mut self,
        item: &'a PatternItem<'a>,
        value: u16,
        declaring: bool,
    ) -> Compiled<()> {
        if let Some(default) = &item.default {
            let skip = self.emit(Op::JumpIfNotUndefined {
                condition: value,
                offset: 0,
            });
            let hint = match &item.target {
                Pattern::Ident(name, _) => Some(self.atom_of_key(name)),
                _ => None,
            };
            self.expr_named(default, value, hint)?;
            self.patch(skip);
        }
        self.bind(&item.target, value, declaring)
    }

    fn get_named(&mut self, dst: u16, object: u16, name: &str) -> Compiled<()> {
        let atom = self.atom_of_key(name);
        let cache = self.cache(atom)?;
        if atom == text::LENGTH {
            self.emit(Op::GetLength { dst, object, cache });
        } else {
            self.emit(Op::GetProp { dst, object, cache });
        }
        Ok(())
    }

    fn set_named(&mut self, object: u16, value: u16, name: &str) -> Compiled<()> {
        let atom = self.atom_of_key(name);
        let cache = self.cache(atom)?;
        self.emit(Op::SetProp {
            object,
            value,
            cache,
        });
        Ok(())
    }

    /// Writes to a property, working out which one from the expression.
    fn assign_to_expr(&mut self, target: &'a Expr<'a>, value: u16) -> Compiled<()> {
        let mark = self.mark();
        match target {
            Expr::Member { object, name, .. } => {
                let object = self.expr_reg(object)?;
                self.set_named(object, value, name)?;
            }
            Expr::Index { object, index, .. } => {
                let object = self.expr_reg(object)?;
                let key = self.expr_reg(index)?;
                self.emit(Op::SetElem { object, key, value });
            }
            Expr::SuperMember(name) => {
                let this = self.alloc()?;
                self.read_this(this)?;
                let key = self.alloc()?;
                let atom = self.intern(name);
                let string = self.vm.atom_value(atom);
                self.load_const(key, string);
                self.emit(Op::SuperSet { key, value, this });
            }
            Expr::SuperIndex(index) => {
                let this = self.alloc()?;
                self.read_this(this)?;
                let key = self.expr_reg(index)?;
                self.emit(Op::SuperSet { key, value, this });
            }
            _ => return self.error("that can't be assigned to", 0),
        }
        self.release(mark);
        Ok(())
    }

    // Conditions.

    /// Jumps if the expression's truth is the same as `sense`, and carries straight on otherwise.
    fn test(
        &mut self,
        expression: &'a Expr<'a>,
        sense: bool,
        out: &mut Vec<usize>,
    ) -> Compiled<()> {
        self.check_depth()?;
        match expression {
            Expr::True | Expr::False => {
                if matches!(expression, Expr::True) == sense {
                    out.push(self.jump());
                }
                return Ok(());
            }
            Expr::Unary(UnaryOp::Not, inner) => return self.test(inner, !sense, out),
            Expr::Logical(LogicalOp::And, a, b) => {
                if sense {
                    let mut skip = Vec::new();
                    self.test(a, false, &mut skip)?;
                    self.test(b, true, out)?;
                    self.patch_all(&skip);
                } else {
                    self.test(a, false, out)?;
                    self.test(b, false, out)?;
                }
                return Ok(());
            }
            Expr::Logical(LogicalOp::Or, a, b) => {
                if sense {
                    self.test(a, true, out)?;
                    self.test(b, true, out)?;
                } else {
                    let mut skip = Vec::new();
                    self.test(a, true, &mut skip)?;
                    self.test(b, false, out)?;
                    self.patch_all(&skip);
                }
                return Ok(());
            }
            Expr::Binary(op, a, b) if !self.f().wide => {
                if self.test_special(*op, a, b, sense, out)? {
                    return Ok(());
                }
                let make: Option<fn(u16, u16) -> Op> = match (op, sense) {
                    (BinaryOp::Lt, true) => Some(|a, b| Op::JumpLt { a, b, offset: 0 }),
                    (BinaryOp::Lt, false) => Some(|a, b| Op::JumpNotLt { a, b, offset: 0 }),
                    (BinaryOp::Le, true) => Some(|a, b| Op::JumpLe { a, b, offset: 0 }),
                    (BinaryOp::Le, false) => Some(|a, b| Op::JumpNotLe { a, b, offset: 0 }),
                    (BinaryOp::Gt, true) => Some(|a, b| Op::JumpGt { a, b, offset: 0 }),
                    (BinaryOp::Gt, false) => Some(|a, b| Op::JumpNotGt { a, b, offset: 0 }),
                    (BinaryOp::Ge, true) => Some(|a, b| Op::JumpGe { a, b, offset: 0 }),
                    (BinaryOp::Ge, false) => Some(|a, b| Op::JumpNotGe { a, b, offset: 0 }),
                    (BinaryOp::StrictEq, true) | (BinaryOp::StrictNe, false) => {
                        Some(|a, b| Op::JumpStrictEq { a, b, offset: 0 })
                    }
                    (BinaryOp::StrictEq, false) | (BinaryOp::StrictNe, true) => {
                        Some(|a, b| Op::JumpStrictNe { a, b, offset: 0 })
                    }
                    (BinaryOp::Eq, true) | (BinaryOp::Ne, false) => {
                        Some(|a, b| Op::JumpEq { a, b, offset: 0 })
                    }
                    (BinaryOp::Eq, false) | (BinaryOp::Ne, true) => {
                        Some(|a, b| Op::JumpNe { a, b, offset: 0 })
                    }
                    _ => None,
                };
                if let Some(make) = make {
                    let mark = self.mark();
                    let (a, b) = if matches!(op, BinaryOp::Eq | BinaryOp::Ne | BinaryOp::StrictEq | BinaryOp::StrictNe) {
                        self.equality_operands(matches!(op, BinaryOp::StrictEq | BinaryOp::StrictNe), a, b)?
                    } else {
                        self.binary_operands(a, b)?
                    };
                    out.push(self.emit(make(a, b)));
                    self.release(mark);
                    return Ok(());
                }
            }
            _ => {}
        }
        let mark = self.mark();
        let condition = self.expr_reg(expression)?;
        out.push(self.emit(if sense {
            Op::JumpIfTrue {
                condition,
                offset: 0,
            }
        } else {
            Op::JumpIfFalse {
                condition,
                offset: 0,
            }
        }));
        self.release(mark);
        Ok(())
    }

    /// A comparison that has an instruction just for its kind of operand. It says whether this was one.
    fn test_special(
        &mut self,
        op: BinaryOp,
        a: &'a Expr<'a>,
        b: &'a Expr<'a>,
        sense: bool,
        out: &mut Vec<usize>,
    ) -> Compiled<bool> {
        let mark = self.mark();
        if matches!(
            op,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::StrictEq | BinaryOp::StrictNe
        ) {
            let if_equal = sense == matches!(op, BinaryOp::Eq | BinaryOp::StrictEq);
            let strict = matches!(op, BinaryOp::StrictEq | BinaryOp::StrictNe);
            let typed = match (a, b) {
                (Expr::Unary(UnaryOp::TypeOf, inner), Expr::String(name))
                | (Expr::String(name), Expr::Unary(UnaryOp::TypeOf, inner)) => {
                    type_kind(name).map(|kind| (inner, kind))
                }
                _ => None,
            };
            if let Some((inner, kind)) = typed {
                // `typeof` of a name that isn't declared is fine, which only `typeof` itself deals with.
                let safe = match &**inner {
                    Expr::Ident(name, _) => self.local_register(name)?.is_some(),
                    _ => true,
                };
                if safe {
                    let a = self.expr_reg(inner)?;
                    out.push(self.emit(if if_equal {
                        Op::JumpTypeIs { a, kind, offset: 0 }
                    } else {
                        Op::JumpTypeIsNot { a, kind, offset: 0 }
                    }));
                    self.release(mark);
                    return Ok(true);
                }
            }
            let right_nothing = self.nothing_literal(b)?;
            let (value, nothing) = match right_nothing {
                Some(nothing) => (Some(a), Some(nothing)),
                None => match self.nothing_literal(a)? {
                    Some(nothing) => (Some(b), Some(nothing)),
                    None => (None, None),
                },
            };
            if let (Some(value), Some(is_null)) = (value, nothing) {
                let condition = self.expr_reg(value)?;
                let offset = 0;
                out.push(self.emit(match (strict, is_null, if_equal) {
                    (false, _, true) => Op::JumpIfNullish { condition, offset },
                    (false, _, false) => Op::JumpIfNotNullish { condition, offset },
                    (true, true, true) => Op::JumpIfNull { condition, offset },
                    (true, true, false) => Op::JumpIfNotNull { condition, offset },
                    (true, false, true) => Op::JumpIfUndefined { condition, offset },
                    (true, false, false) => Op::JumpIfNotUndefined { condition, offset },
                }));
                self.release(mark);
                return Ok(true);
            }
        }
        let Expr::Number(number) = b else {
            return Ok(false);
        };
        let value = *number as i16;
        if f64::from(value) != *number || (value == 0 && number.is_sign_negative()) {
            return Ok(false);
        }
        let make: fn(u16, i16) -> Op = match (op, sense) {
            (BinaryOp::Lt, true) => |a, value| Op::JumpLtInt {
                a,
                value,
                offset: 0,
            },
            (BinaryOp::Lt, false) => |a, value| Op::JumpNotLtInt {
                a,
                value,
                offset: 0,
            },
            (BinaryOp::Le, true) => |a, value| Op::JumpLeInt {
                a,
                value,
                offset: 0,
            },
            (BinaryOp::Le, false) => |a, value| Op::JumpNotLeInt {
                a,
                value,
                offset: 0,
            },
            (BinaryOp::Gt, true) => |a, value| Op::JumpGtInt {
                a,
                value,
                offset: 0,
            },
            (BinaryOp::Gt, false) => |a, value| Op::JumpNotGtInt {
                a,
                value,
                offset: 0,
            },
            (BinaryOp::Ge, true) => |a, value| Op::JumpGeInt {
                a,
                value,
                offset: 0,
            },
            (BinaryOp::Ge, false) => |a, value| Op::JumpNotGeInt {
                a,
                value,
                offset: 0,
            },
            (BinaryOp::StrictEq, true) | (BinaryOp::StrictNe, false) => {
                |a, value| Op::JumpStrictEqInt {
                    a,
                    value,
                    offset: 0,
                }
            }
            (BinaryOp::StrictEq, false) | (BinaryOp::StrictNe, true) => {
                |a, value| Op::JumpStrictNeInt {
                    a,
                    value,
                    offset: 0,
                }
            }
            _ => return Ok(false),
        };
        let a = self.expr_reg(a)?;
        out.push(self.emit(make(a, value)));
        self.release(mark);
        Ok(true)
    }

    /// Whether this is written as `null` (true) or `undefined` (false), and is known to be nothing else.
    fn nothing_literal(&mut self, expression: &'a Expr<'a>) -> Compiled<Option<bool>> {
        Ok(match expression {
            Expr::Null => Some(true),
            Expr::Unary(UnaryOp::Void, inner) if matches!(&**inner, Expr::Number(_)) => Some(false),
            Expr::Ident("undefined", _)
                if self.with_chain("undefined").is_empty()
                    && self.resolve("undefined")? == Place::Global =>
            {
                Some(false)
            }
            _ => None,
        })
    }

    // Expressions.

    /// Works out both sides of an operator, in the right order, even if the right side changes a variable on the left.
    fn binary_operands(&mut self, a: &'a Expr<'a>, b: &'a Expr<'a>) -> Compiled<(u16, u16)> {
        let left = if matches!(a, Expr::Ident(..)) && may_assign(b, 8) {
            let temp = self.alloc()?;
            self.expr(a, temp)?;
            temp
        } else {
            self.expr_reg(a)?
        };
        let right = self.expr_reg(b)?;
        Ok((left, right))
    }

    /// Works out an expression into some register, which is the variable's own if it's a variable.
    fn expr_reg(&mut self, expression: &'a Expr<'a>) -> Compiled<u16> {
        match expression {
            Expr::Ident(name, _) => {
                if let Some(reg) = self.local_register(name)? {
                    return Ok(reg);
                }
            }
            Expr::This => {
                let state = self.f();
                if state.kind != FunctionKind::Arrow
                    && state.kind != FunctionKind::DerivedConstructor
                    && state.this_cell.is_none()
                {
                    return Ok(0);
                }
            }
            _ => {}
        }
        let temp = self.alloc()?;
        self.expr_fresh(expression, temp)?;
        Ok(temp)
    }

    fn expr_discard(&mut self, expression: &'a Expr<'a>) -> Compiled<()> {
        let mark = self.mark();
        match expression {
            Expr::Update {
                increment,
                prefix,
                target,
            } => {
                if let Expr::Ident(name, _) = &**target {
                    if let Some(reg) = self.local_register(name)? {
                        if !matches!(
                            self.resolve(name)?,
                            Place::Register {
                                kind: VarKind::Const,
                                ..
                            }
                        ) {
                            self.emit(if *increment {
                                Op::Inc { dst: reg, src: reg }
                            } else {
                                Op::Dec { dst: reg, src: reg }
                            });
                            return Ok(());
                        }
                    }
                }
                self.update(*increment, *prefix, target, DISCARD)?;
            }
            Expr::Assign {
                op: AssignOp::Plain,
                target,
                value,
            } => {
                if let Pattern::Ident(name, _) = &**target {
                    if let Place::Register {
                        reg,
                        started: true,
                        kind: VarKind::Var | VarKind::Let,
                    } = self.resolve(name)?
                    {
                        if writes_last(value) && self.with_chain(name).is_empty() {
                            let atom = self.atom_of_key(name);
                            self.expr_named(value, reg, Some(atom))?;
                            return Ok(());
                        }
                    }
                }
                self.assign(AssignOp::Plain, target, value, DISCARD)?;
            }
            Expr::Assign { op, target, value } => self.assign(*op, target, value, DISCARD)?,
            _ => {
                let temp = self.alloc()?;
                self.expr(expression, temp)?;
            }
        }
        self.release(mark);
        Ok(())
    }

    /// Works out a value that may be a function or a class with no name, which takes the one it's being given.
    fn expr_named(
        &mut self,
        expression: &'a Expr<'a>,
        dst: u16,
        hint: Option<Atom>,
    ) -> Compiled<()> {
        match expression {
            Expr::Function(function) if function.name.is_none() => {
                self.make_closure(function, dst, hint, false)
            }
            Expr::Class(class) if class.name.is_none() => self.class(class, dst, hint),
            _ => self.expr(expression, dst),
        }
    }

    fn property_key(&mut self, key: &'a PropertyKey<'a>, dst: u16) -> Compiled<()> {
        match key {
            PropertyKey::Name(name) => {
                let atom = self.atom_of_key(name);
                let value = if name.starts_with('#') {
                    Value::symbol(atom.0)
                } else {
                    self.vm.atom_value(atom)
                };
                self.load_const(dst, value);
            }
            PropertyKey::String(units) => self.load_string(dst, units),
            PropertyKey::Number(number) => self.load_number(dst, *number),
            PropertyKey::BigInt(digits, radix) => {
                let value = self.vm.bigint_literal(digits, *radix);
                self.load_const(dst, value);
                self.emit(Op::ToKey { dst, src: dst });
            }
            PropertyKey::Computed(expression) => {
                self.expr(expression, dst)?;
                self.emit(Op::ToKey { dst, src: dst });
            }
        }
        Ok(())
    }

    /// Whether a key is `__proto__`, which as a value says what an object literal's prototype is, or a private name.
    fn is_proto_key(&self, key: &PropertyKey<'a>) -> bool {
        match key {
            PropertyKey::Name(name) => *name == "__proto__" || name.starts_with('#'),
            PropertyKey::String(units) => String::from_utf16_lossy(units) == "__proto__",
            _ => false,
        }
    }

    fn key_hint(&mut self, key: &PropertyKey<'a>) -> Option<Atom> {
        match key {
            // A function gets its name as text, which for a private name is what it's written as.
            PropertyKey::Name(name) => Some(self.intern(name)),
            PropertyKey::String(units) => Some(self.vm.intern_units(units)),
            PropertyKey::Number(number) => {
                Some(self.intern(&crate::convert::number_to_string(*number)))
            }
            _ => None,
        }
    }

    /// The name of a getter or a setter, as `get x` or `set x`, if its key is known before the code runs.
    fn accessor_hint(&mut self, key: &PropertyKey<'a>, prefix: &str) -> Option<Atom> {
        let mut units: Vec<u16> = prefix.encode_utf16().collect();
        units.push(u16::from(b' '));
        match key {
            PropertyKey::Name(name) => units.extend(name.encode_utf16()),
            PropertyKey::String(found) => units.extend_from_slice(found),
            PropertyKey::Number(number) => {
                units.extend(crate::convert::number_to_string(*number).encode_utf16());
            }
            _ => return None,
        }
        Some(self.vm.intern_units(&units))
    }

    /// Names the function in `function` from the key in `key`, which is only known when the code runs, as a computed
    /// key's is: `[description]` for a symbol, and `prefix` before it for a getter (1) or a setter (2).
    fn name_from_key(&mut self, function: u16, key: u16, prefix: i32) -> Compiled<()> {
        let mark = self.mark();
        let native = self.vm.name_function_native();
        let f = self.alloc()?;
        let this = self.alloc()?;
        let first = self.alloc()?;
        let second = self.alloc()?;
        let third = self.alloc()?;
        self.load_const(f, native);
        self.emit(Op::LoadUndefined { dst: this });
        self.emit(Op::Move {
            dst: first,
            src: function,
        });
        self.emit(Op::Move {
            dst: second,
            src: key,
        });
        self.emit(Op::LoadInt {
            dst: third,
            value: prefix,
        });
        self.emit(Op::Call { dst: f, f, argc: 3 });
        self.release(mark);
        Ok(())
    }

    fn expr(&mut self, expression: &'a Expr<'a>, dst: u16) -> Compiled<()> {
        self.check_depth()?;
        match expression {
            Expr::Number(number) => self.load_number(dst, *number),
            Expr::BigInt(digits, radix) => {
                let value = self.vm.bigint_literal(digits, *radix);
                self.load_const(dst, value);
            }
            Expr::String(units) => self.load_string(dst, units),
            Expr::Template(parts) => self.template(parts.strings, parts.values, dst)?,
            Expr::TaggedTemplate(parts) => {
                self.tagged_template(parts.tag, parts.strings, parts.raw, parts.values, dst)?;
            }
            Expr::Regex(parts) => {
                let (pattern, flags) = (parts.pattern, parts.flags);
                let mark = self.mark();
                let f = self.alloc()?;
                let _this = self.alloc()?;
                let pattern_reg = self.alloc()?;
                let flags_reg = self.alloc()?;
                let atom = self.intern("RegExp");
                let cache = self.cache(atom)?;
                self.emit(Op::GetGlobal { dst: f, cache });
                self.load_string(pattern_reg, pattern);
                self.load_string(flags_reg, flags);
                self.emit(Op::New { dst, f, argc: 2 });
                self.release(mark);
            }
            Expr::Ident(name, line) => {
                self.mark_line(*line);
                let name = self.jsx_factory(name);
                self.read_var(name, dst)?;
            }
            Expr::PrivateName(name) => {
                let atom = self.atom_of_key(name);
                self.load_const(dst, Value::symbol(atom.0));
            }
            Expr::ImportCall(specifier) => {
                let mark = self.mark();
                let f = self.alloc()?;
                let this = self.alloc()?;
                let argument = self.alloc()?;
                let file = self.alloc()?;
                let atom = self.vm.private_atom("%import");
                let cache = self.cache(atom)?;
                self.emit(Op::GetGlobal { dst: f, cache });
                self.emit(Op::LoadUndefined { dst: this });
                self.expr(specifier, argument)?;
                // An internal filename, distinct from a user's equal string,
                // so packed bytecode can relocate it without changing literals.
                let file_value = Value::symbol(self.file.0);
                self.load_const(file, file_value);
                self.emit(Op::Call { dst, f, argc: 2 });
                self.release(mark);
            }
            // The same object each time, for each module: see `Vm::import_meta`.
            Expr::ImportMeta => {
                let mark = self.mark();
                let f = self.alloc()?;
                let this = self.alloc()?;
                let file = self.alloc()?;
                let atom = self.vm.private_atom("%meta");
                let cache = self.cache(atom)?;
                self.emit(Op::GetGlobal { dst: f, cache });
                self.emit(Op::LoadUndefined { dst: this });
                let file_value = Value::symbol(self.file.0);
                self.load_const(file, file_value);
                self.emit(Op::Call { dst, f, argc: 1 });
                self.release(mark);
            }
            Expr::This => self.read_this(dst)?,
            Expr::Null => {
                self.emit(Op::LoadNull { dst });
            }
            Expr::True => {
                self.emit(Op::LoadTrue { dst });
            }
            Expr::False => {
                self.emit(Op::LoadFalse { dst });
            }
            Expr::NewTarget => {
                if self.f().kind == FunctionKind::Arrow || self.f().inner.contains("new.target") {
                    self.read_var("new.target", dst)?;
                } else {
                    self.emit(Op::NewTarget { dst });
                }
            }
            Expr::Array(elements) => self.array_literal(elements, dst)?,
            Expr::Object(properties) => self.object_literal(properties, dst)?,
            Expr::Function(function) => self.make_closure(function, dst, None, false)?,
            Expr::Class(class) => self.class(class, dst, None)?,
            Expr::Unary(op, value) => self.unary(*op, value, dst)?,
            Expr::Update {
                increment,
                prefix,
                target,
            } => self.update(*increment, *prefix, target, dst)?,
            Expr::Binary(op, a, b) => self.binary(*op, a, b, dst)?,
            Expr::Logical(op, a, b) => {
                self.expr(a, dst)?;
                let skip = self.logical_jump(*op, dst);
                self.expr(b, dst)?;
                self.patch(skip);
            }
            Expr::Assign { op, target, value } => self.assign(*op, target, value, dst)?,
            Expr::Conditional(test, then, otherwise) => {
                let mut to_else = Vec::new();
                self.test(test, false, &mut to_else)?;
                self.expr(then, dst)?;
                let end = self.jump();
                self.patch_all(&to_else);
                self.expr(otherwise, dst)?;
                self.patch(end);
            }
            Expr::Call {
                callee,
                args,
                optional,
                line,
            } => self.call(callee, args, *optional, *line, dst)?,
            Expr::New { callee, args, line } => {
                let mark = self.mark();
                let f = self.alloc()?;
                let _this = self.alloc()?;
                self.expr(callee, f)?;
                self.mark_line(*line);
                if args.iter().any(|arg| matches!(arg, Element::Spread(_))) {
                    let array = self.args_array(args)?;
                    self.emit(Op::NewSpread { dst, f, array });
                } else {
                    let argc = self.plain_args(args)?;
                    self.emit(Op::New { dst, f, argc });
                }
                self.release(mark);
            }
            Expr::Member { .. } | Expr::Index { .. } if self.noted_chain(expression, dst, None)? => {}
            Expr::Member {
                object,
                name,
                optional,
            } => {
                let mark = self.mark();
                let object = self.expr_reg(object)?;
                if *optional {
                    self.chain_jump(object);
                }
                self.get_named(dst, object, name)?;
                self.release(mark);
            }
            Expr::Index {
                object,
                index,
                optional,
            } => {
                let mark = self.mark();
                let (object, key) = if *optional {
                    let object = self.expr_reg(object)?;
                    self.chain_jump(object);
                    (object, self.expr_reg(index)?)
                } else {
                    self.binary_operands(object, index)?
                };
                self.emit(Op::GetElem { dst, object, key });
                self.release(mark);
            }
            Expr::Chain(inner) => {
                self.f().chains.push(Vec::new());
                self.expr(inner, dst)?;
                let jumps = self.f().chains.pop().unwrap_or_default();
                let end = self.jump();
                self.patch_all(&jumps);
                self.emit(Op::LoadUndefined { dst });
                self.patch(end);
            }
            Expr::SuperCall(args) => self.super_call(args, dst)?,
            Expr::SuperMember(name) => {
                let mark = self.mark();
                let this = self.alloc()?;
                self.read_this(this)?;
                let key = self.alloc()?;
                let atom = self.intern(name);
                let string = self.vm.atom_value(atom);
                self.load_const(key, string);
                self.emit(Op::SuperGet { dst, key, this });
                self.release(mark);
            }
            Expr::SuperIndex(index) => {
                let mark = self.mark();
                let this = self.alloc()?;
                self.read_this(this)?;
                let key = self.expr_reg(index)?;
                self.emit(Op::SuperGet { dst, key, this });
                self.release(mark);
            }
            Expr::Sequence(items) => {
                let Some((last, rest)) = items.split_last() else {
                    return Ok(());
                };
                for item in rest {
                    self.expr_discard(item)?;
                }
                self.expr(last, dst)?;
            }
            Expr::Await(value) => {
                let mark = self.mark();
                let src = self.expr_reg(value)?;
                self.emit(Op::Await { dst, src });
                self.release(mark);
            }
            Expr::Yield { value, delegate } => {
                let mark = self.mark();
                if *delegate {
                    let Some(value) = value else {
                        return self.error("yield* needs something to yield from", 0);
                    };
                    let is_async = self.f().is_async_generator;
                    let iter = self.alloc()?;
                    let _position = self.alloc()?;
                    let source = self.expr_reg(value)?;
                    if is_async {
                        self.emit(Op::GetAsyncIterator { iter, source });
                    } else {
                        self.emit(Op::GetIterator { iter, source });
                    }
                    let item = self.alloc()?;
                    let received = self.alloc()?;
                    self.emit(Op::LoadUndefined { dst: received });
                    let top = self.here();
                    self.emit(Op::DelegateNext {
                        dst: item,
                        iter,
                        received,
                    });
                    if is_async {
                        self.emit(Op::Await {
                            dst: item,
                            src: item,
                        });
                    }
                    let exit = self.iter_result(item);
                    let start = self.here();
                    self.emit(Op::Yield {
                        dst: received,
                        src: item,
                    });
                    let end = self.here();
                    self.jump_back(top);
                    // Stopping this generator partway stops the one it's going through too.
                    let caught = self.alloc()?;
                    let target = self.here();
                    self.f().handlers.push(Handler {
                        start: start as u32,
                        end: end as u32,
                        target: target as u32,
                        reg: caught,
                        is_catch: false,
                    });
                    self.emit(Op::IterCloseQuietly { iter });
                    self.emit(Op::Throw { src: caught });
                    self.patch(exit);
                    // What the inner one returned is the value of the whole `yield*`.
                    self.emit(Op::Move { dst, src: item });
                } else {
                    let src = match value {
                        Some(value) => self.expr_reg(value)?,
                        None => {
                            let temp = self.alloc()?;
                            self.emit(Op::LoadUndefined { dst: temp });
                            temp
                        }
                    };
                    // An async generator waits for what it's about to yield.
                    let src = if self.f().is_async_generator {
                        let temp = self.alloc()?;
                        self.emit(Op::Await { dst: temp, src });
                        temp
                    } else {
                        src
                    };
                    self.emit(Op::Yield { dst, src });
                }
                self.release(mark);
            }
        }
        Ok(())
    }

    fn logical_jump(&mut self, op: LogicalOp, reg: u16) -> usize {
        self.emit(match op {
            LogicalOp::And => Op::JumpIfFalse {
                condition: reg,
                offset: 0,
            },
            LogicalOp::Or => Op::JumpIfTrue {
                condition: reg,
                offset: 0,
            },
            LogicalOp::Nullish => Op::JumpIfNotNullish {
                condition: reg,
                offset: 0,
            },
        })
    }

    fn chain_jump(&mut self, object: u16) {
        let jump = self.emit(Op::JumpIfNullish {
            condition: object,
            offset: 0,
        });
        if let Some(list) = self.f().chains.last_mut() {
            list.push(jump);
        }
    }

    fn template(
        &mut self,
        strings: &'a [&'a [u16]],
        values: &'a [Expr<'a>],
        dst: u16,
    ) -> Compiled<()> {
        let mark = self.mark();
        let start = self.f().next_reg as u16;
        let mut count = 0u16;
        for (index, string) in strings.iter().enumerate() {
            if !string.is_empty() {
                let reg = self.alloc()?;
                self.load_string(reg, string);
                count += 1;
            }
            if let Some(value) = values.get(index) {
                let reg = self.alloc()?;
                self.expr(value, reg)?;
                count += 1;
            }
        }
        self.emit(Op::Concat { dst, start, count });
        self.release(mark);
        Ok(())
    }

    fn tagged_template(
        &mut self,
        tag: &'a Expr<'a>,
        strings: &'a [&'a [u16]],
        raw: &'a [&'a [u16]],
        values: &'a [Expr<'a>],
        dst: u16,
    ) -> Compiled<()> {
        let mark = self.mark();
        let f = self.alloc()?;
        let this = self.alloc()?;
        self.callee(tag, f, this)?;
        let template = self.alloc()?;
        let object = self.vm.template_object(strings, raw);
        self.load_const(template, object);
        for value in values {
            let reg = self.alloc()?;
            self.expr(value, reg)?;
        }
        self.emit(Op::Call {
            dst,
            f,
            argc: values.len() as u16 + 1,
        });
        self.release(mark);
        Ok(())
    }

    /// Works out an expression into a register nothing else is using yet. A new object or array can be built right
    /// there, as nothing in the expression could be looking at it.
    fn expr_fresh(&mut self, expression: &'a Expr<'a>, dst: u16) -> Compiled<()> {
        match expression {
            Expr::Array(elements) => self.array_literal_in(elements, dst),
            Expr::Object(properties) => self.object_literal_in(properties, dst),
            _ => self.expr(expression, dst),
        }
    }

    fn array_literal(&mut self, elements: &'a [Option<Element<'a>>], dst: u16) -> Compiled<()> {
        let mark = self.mark();
        let array = self.alloc()?;
        self.array_literal_in(elements, array)?;
        self.emit(Op::Move { dst, src: array });
        self.release(mark);
        Ok(())
    }

    fn array_literal_in(
        &mut self,
        elements: &'a [Option<Element<'a>>],
        array: u16,
    ) -> Compiled<()> {
        let itself = std::mem::take(&mut self.itself_items);
        self.emit(Op::NewArray {
            dst: array,
            capacity: elements.len() as u32,
        });
        for element in elements {
            let inner = self.mark();
            match element {
                None => {
                    self.emit(Op::ArrayHole { array });
                }
                Some(Element::Plain(value)) if itself && self.may_note(value)? => {
                    let temp = self.alloc()?;
                    self.expr_itself(value, temp)?;
                    self.emit(Op::ArrayPush { array, value: temp });
                }
                Some(Element::Plain(value)) => {
                    let value = self.expr_reg(value)?;
                    self.emit(Op::ArrayPush { array, value });
                }
                Some(Element::Spread(value)) => {
                    let source = self.expr_reg(value)?;
                    self.emit(Op::ArraySpread { array, source });
                }
            }
            self.release(inner);
        }
        Ok(())
    }

    fn object_literal(&mut self, properties: &'a [Property<'a>], dst: u16) -> Compiled<()> {
        let mark = self.mark();
        let object = self.alloc()?;
        self.object_literal_in(properties, object)?;
        self.emit(Op::Move { dst, src: object });
        self.release(mark);
        Ok(())
    }

    fn object_literal_in(&mut self, properties: &'a [Property<'a>], object: u16) -> Compiled<()> {
        let jsx = std::mem::take(&mut self.f().jsx_props_next);
        self.emit(Op::NewObject {
            dst: object,
            capacity: properties.len().min(1000) as u16,
        });
        for property in properties {
            let inner = self.mark();
            // A prop of an element that's a noted read as it is says which prop it's given as.
            let given = match property {
                Property::Value(key @ (PropertyKey::Name(_) | PropertyKey::String(_)), value) if jsx => {
                    let atom = self.key_hint(key).filter(|atom| !matches!(self.vm.atom_text(*atom).as_str(), "children" | "key" | "ref"));
                    if atom.is_some() && self.may_note(value)? { atom } else { None }
                }
                Property::Short(name, _) if jsx && !matches!(*name, "children" | "key" | "ref") => {
                    if self.note_base(name)?.is_some() { Some(self.intern(name)) } else { None }
                }
                _ => None,
            };
            self.f().prop_next = given;
            match property {
                Property::Value(key, value) => {
                    let hint = self.key_hint(key);
                    let value_reg = self.alloc()?;
                    let is_proto = match key {
                        PropertyKey::Name(name) => *name == "__proto__",
                        PropertyKey::String(units) => {
                            String::from_utf16_lossy(units) == "__proto__"
                        }
                        _ => false,
                    };
                    let plain_atom = match key {
                        PropertyKey::Name(_) | PropertyKey::String(_) if !is_proto => {
                            hint.filter(|atom| self.vm.atoms.index_of(*atom).is_none())
                        }
                        _ => None,
                    };
                    if let Some(atom) = plain_atom {
                        let value_reg = match value {
                            Expr::Ident(name, _) => self.expr_short(name)?,
                            Expr::Array(_) | Expr::Object(_) => {
                                self.expr_fresh(value, value_reg)?;
                                value_reg
                            }
                            _ => {
                                self.expr_named(value, value_reg, hint)?;
                                value_reg
                            }
                        };
                        let cache = self.cache(atom)?;
                        self.emit(Op::DefineField {
                            object,
                            value: value_reg,
                            cache,
                        });
                    } else if is_proto {
                        self.expr(value, value_reg)?;
                        self.set_named(object, value_reg, "__proto__")?;
                    } else {
                        let key_reg = self.alloc()?;
                        self.property_key(key, key_reg)?;
                        self.expr_named(value, value_reg, hint)?;
                        if hint.is_none() && is_anonymous_function(value) {
                            self.name_from_key(value_reg, key_reg, 0)?;
                        }
                        self.emit(Op::DefineElem {
                            object,
                            key: key_reg,
                            value: value_reg,
                        });
                    }
                }
                Property::Short(name, line) => {
                    self.mark_line(*line);
                    let value = self.expr_short(name)?;
                    let atom = self.atom_of_key(name);
                    let cache = self.cache(atom)?;
                    self.emit(Op::DefineField {
                        object,
                        value,
                        cache,
                    });
                }
                // A method with a name of its own (`get(target, key) {}`) is defined as a field is, which a place in
                // the code remembers how to do: the same property, with the same flags, as by its key.
                Property::Method(
                    key @ (PropertyKey::Name(_) | PropertyKey::String(_)),
                    function,
                ) if self.key_hint(key).is_some_and(|atom| {
                    self.vm.atoms.index_of(atom).is_none() && !self.is_proto_key(key)
                }) =>
                {
                    let hint = self.key_hint(key);
                    let atom = hint.expect("it has a name");
                    let closure = self.alloc()?;
                    self.make_closure(function, closure, hint, false)?;
                    self.emit(Op::SetHome {
                        function: closure,
                        home: object,
                    });
                    let cache = self.cache(atom)?;
                    self.emit(Op::DefineField {
                        object,
                        value: closure,
                        cache,
                    });
                }
                Property::Method(key, function)
                | Property::Getter(key, function)
                | Property::Setter(key, function) => {
                    let key_reg = self.alloc()?;
                    self.property_key(key, key_reg)?;
                    let closure = self.alloc()?;
                    let prefix = match property {
                        Property::Getter(..) => 1,
                        Property::Setter(..) => 2,
                        _ => 0,
                    };
                    let hint = match prefix {
                        1 => self.accessor_hint(key, "get"),
                        2 => self.accessor_hint(key, "set"),
                        _ => self.key_hint(key),
                    };
                    self.make_closure(function, closure, hint, false)?;
                    if hint.is_none() {
                        self.name_from_key(closure, key_reg, prefix)?;
                    }
                    self.emit(Op::SetHome {
                        function: closure,
                        home: object,
                    });
                    self.emit(match property {
                        Property::Getter(..) => Op::DefineGetter {
                            object,
                            key: key_reg,
                            function: closure,
                        },
                        Property::Setter(..) => Op::DefineSetter {
                            object,
                            key: key_reg,
                            function: closure,
                        },
                        _ => Op::DefineElem {
                            object,
                            key: key_reg,
                            value: closure,
                        },
                    });
                }
                Property::Spread(value) => {
                    let source = self.expr_reg(value)?;
                    self.emit(Op::ObjectSpread { object, source });
                }
            }
            self.f().prop_next = None;
            self.release(inner);
        }
        Ok(())
    }

    fn expr_short(&mut self, name: &'a str) -> Compiled<u16> {
        if let Some(reg) = self.local_register(name)? {
            return Ok(reg);
        }
        let temp = self.alloc()?;
        self.read_var(name, temp)?;
        Ok(temp)
    }

    fn unary(&mut self, op: UnaryOp, value: &'a Expr<'a>, dst: u16) -> Compiled<()> {
        let mark = self.mark();
        match op {
            UnaryOp::Neg => {
                if let Expr::Number(number) = value {
                    self.load_number(dst, -*number);
                } else {
                    let src = self.expr_reg(value)?;
                    self.emit(Op::Neg { dst, src });
                }
            }
            UnaryOp::Plus => {
                let src = self.expr_reg(value)?;
                self.emit(Op::ToNumber { dst, src });
            }
            UnaryOp::Not => {
                let src = self.expr_reg(value)?;
                self.emit(Op::Not { dst, src });
            }
            UnaryOp::BitNot => {
                let src = self.expr_reg(value)?;
                self.emit(Op::BitNot { dst, src });
            }
            UnaryOp::TypeOf => {
                if let Expr::Ident(name, _) = value {
                    let chain = self.with_chain(name);
                    if !chain.is_empty() && self.resolve(name)? == Place::Global {
                        let object = self.alloc()?;
                        let key = self.alloc()?;
                        let found = self.alloc()?;
                        let item = self.alloc()?;
                        let units: Vec<u16> = name.encode_utf16().collect();
                        self.load_string(key, &units);
                        let mut ends = Vec::new();
                        for with_name in chain {
                            self.read_plain(with_name, object)?;
                            self.emit(Op::In {
                                dst: found,
                                a: key,
                                b: object,
                            });
                            let skip = self.emit(Op::JumpIfFalse {
                                condition: found,
                                offset: 0,
                            });
                            self.emit(Op::GetElem {
                                dst: item,
                                object,
                                key,
                            });
                            self.emit(Op::TypeOf { dst, src: item });
                            ends.push(self.jump());
                            self.patch(skip);
                        }
                        let atom = self.intern(name);
                        let cache = self.cache(atom)?;
                        self.emit(Op::TypeOfGlobal { dst, cache });
                        self.patch_all(&ends);
                        self.release(mark);
                        return Ok(());
                    }
                    if self.resolve(name)? == Place::Global && self.with_chain(name).is_empty() {
                        let atom = self.intern(name);
                        let cache = self.cache(atom)?;
                        self.emit(Op::TypeOfGlobal { dst, cache });
                        self.release(mark);
                        return Ok(());
                    }
                }
                let src = self.expr_reg(value)?;
                self.emit(Op::TypeOf { dst, src });
            }
            UnaryOp::Void => {
                self.expr_discard(value)?;
                self.emit(Op::LoadUndefined { dst });
            }
            UnaryOp::Delete => match value {
                Expr::Member { object, name, .. } => {
                    let object = self.expr_reg(object)?;
                    let key = self.alloc()?;
                    let atom = self.atom_of_key(name);
                    let string = if name.starts_with('#') {
                        Value::symbol(atom.0)
                    } else {
                        self.vm.atom_value(atom)
                    };
                    self.load_const(key, string);
                    self.emit(Op::Delete { dst, object, key });
                }
                Expr::Index { object, index, .. } => {
                    let (object, key) = self.binary_operands(object, index)?;
                    self.emit(Op::Delete { dst, object, key });
                }
                Expr::Chain(inner) => {
                    self.f().chains.push(Vec::new());
                    self.unary(UnaryOp::Delete, inner, dst)?;
                    let jumps = self.f().chains.pop().unwrap_or_default();
                    let end = self.jump();
                    self.patch_all(&jumps);
                    self.emit(Op::LoadTrue { dst });
                    self.patch(end);
                }
                other => {
                    self.expr_discard(other)?;
                    self.emit(Op::LoadTrue { dst });
                }
            },
        }
        self.release(mark);
        Ok(())
    }

    fn update(
        &mut self,
        increment: bool,
        prefix: bool,
        target: &'a Expr<'a>,
        dst: u16,
    ) -> Compiled<()> {
        let step = |dst, src| {
            if increment {
                Op::Inc { dst, src }
            } else {
                Op::Dec { dst, src }
            }
        };
        // When nothing wants the result, the old value doesn't matter, which is the simpler way round.
        let prefix = prefix || dst == DISCARD;
        let mark = self.mark();
        // A variable in a register can be stepped where it is, with no copies: `i++` is one instruction.
        let in_place = match target {
            Expr::Ident(name, _) => self.writable_register(name)?,
            _ => None,
        };
        match target {
            Expr::Ident(..) if let Some(reg) = in_place => {
                if prefix {
                    self.emit(step(reg, reg));
                    self.give_result(dst, reg);
                } else {
                    let old = self.alloc()?;
                    self.emit(Op::ToNumeric { dst: old, src: reg });
                    self.emit(step(reg, old));
                    self.give_result(dst, old);
                }
            }
            Expr::Ident(name, _) => {
                let old = self.alloc()?;
                self.read_var(name, old)?;
                if prefix {
                    let new = self.alloc()?;
                    self.emit(step(new, old));
                    self.write_var(name, new)?;
                    self.give_result(dst, new);
                } else {
                    self.emit(Op::ToNumeric { dst: old, src: old });
                    let new = self.alloc()?;
                    self.emit(step(new, old));
                    self.write_var(name, new)?;
                    self.give_result(dst, old);
                }
            }
            Expr::Member { object, name, .. } => {
                let object = self.expr_reg(object)?;
                let old = self.alloc()?;
                self.get_named(old, object, name)?;
                if !prefix {
                    self.emit(Op::ToNumeric { dst: old, src: old });
                }
                let new = self.alloc()?;
                self.emit(step(new, old));
                self.set_named(object, new, name)?;
                self.give_result(dst, if prefix { new } else { old });
            }
            Expr::Index { object, index, .. } => {
                let (object, key) = self.binary_operands(object, index)?;
                let old = self.alloc()?;
                self.emit(Op::GetElem {
                    dst: old,
                    object,
                    key,
                });
                if !prefix {
                    self.emit(Op::ToNumeric { dst: old, src: old });
                }
                let new = self.alloc()?;
                self.emit(step(new, old));
                self.emit(Op::SetElem {
                    object,
                    key,
                    value: new,
                });
                self.give_result(dst, if prefix { new } else { old });
            }
            _ => return self.error("that can't be changed", 0),
        }
        self.release(mark);
        Ok(())
    }

    fn binary(&mut self, op: BinaryOp, a: &'a Expr<'a>, b: &'a Expr<'a>, dst: u16) -> Compiled<()> {
        let mark = self.mark();
        // `(x + y) | 0` is one instruction. (With a number for `y`, AddInt and BitOrInt need no registers loaded.)
        if let (BinaryOp::BitOr, Expr::Binary(BinaryOp::Add, x, y), Expr::Number(zero)) = (op, a, b)
            && *zero == 0.0
            && !matches!(**y, Expr::Number(_))
        {
            let (x, y) = self.binary_operands(x, y)?;
            self.emit(Op::AddInt32 { dst, a: x, b: y });
            self.release(mark);
            return Ok(());
        }
        // `3 * n` is `n * 3`, which has an instruction of its own: a number turns into itself with nothing to see.
        let (a, b) = match (a, b) {
            (Expr::Number(_), other)
                if matches!(op, BinaryOp::Mul | BinaryOp::BitAnd | BinaryOp::BitOr)
                    && !matches!(other, Expr::Number(_)) =>
            {
                (b, a)
            }
            _ => (a, b),
        };
        let equality = matches!(op, BinaryOp::Eq | BinaryOp::Ne | BinaryOp::StrictEq | BinaryOp::StrictNe);
        if self.right_side_is_constant(op, b) {
            let left = self.expr_reg(a)?;
            self.binary_with_left(op, left, b, dst)?;
        } else if equality {
            let strict = matches!(op, BinaryOp::StrictEq | BinaryOp::StrictNe);
            let (a, b) = self.equality_operands(strict, a, b)?;
            self.emit(Self::binary_op(op, dst, a, b));
        } else {
            let (a, b) = self.binary_operands(a, b)?;
            self.emit(Self::binary_op(op, dst, a, b));
        }
        self.release(mark);
        Ok(())
    }

    /// Whether an operator's right side has an instruction that holds it, so the left side can't be changed by it.
    fn right_side_is_constant(&self, op: BinaryOp, b: &Expr<'_>) -> bool {
        match b {
            Expr::Number(number) => {
                small_number_op(op, 0, 0, *number).is_some()
                    || matches!(
                        op,
                        BinaryOp::Add
                            | BinaryOp::Sub
                            | BinaryOp::Mul
                            | BinaryOp::Div
                            | BinaryOp::Mod
                    )
            }
            Expr::String(_) => op == BinaryOp::Add,
            _ => false,
        }
    }

    /// An operator whose left side is already in a register.
    fn binary_with_left(
        &mut self,
        op: BinaryOp,
        left: u16,
        b: &'a Expr<'a>,
        dst: u16,
    ) -> Compiled<()> {
        if let Expr::Number(number) = b {
            if let Some(small) = small_number_op(op, dst, left, *number) {
                self.emit(small);
                return Ok(());
            }
        }
        let constant = match b {
            Expr::Number(number)
                if matches!(
                    op,
                    BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod
                ) =>
            {
                Some(Value::number(*number))
            }
            Expr::String(units) if op == BinaryOp::Add => {
                let atom = self.vm.intern_units(units);
                Some(self.vm.atom_value(atom))
            }
            _ => None,
        };
        if let Some(value) = constant {
            let index = self.vm.add_const(value);
            if let Ok(k) = u16::try_from(index) {
                let a = left;
                self.emit(match op {
                    BinaryOp::Add => Op::AddK { dst, a, k },
                    BinaryOp::Sub => Op::SubK { dst, a, k },
                    BinaryOp::Mul => Op::MulK { dst, a, k },
                    BinaryOp::Div => Op::DivK { dst, a, k },
                    _ => Op::ModK { dst, a, k },
                });
                return Ok(());
            }
        }
        let right = self.expr_reg(b)?;
        self.emit(Self::binary_op(op, dst, left, right));
        Ok(())
    }

    fn give_result(&mut self, dst: u16, src: u16) {
        if dst != DISCARD && dst != src {
            self.emit(Op::Move { dst, src });
        }
    }

    fn binary_op(op: BinaryOp, dst: u16, a: u16, b: u16) -> Op {
        match op {
            BinaryOp::Add => Op::Add { dst, a, b },
            BinaryOp::Sub => Op::Sub { dst, a, b },
            BinaryOp::Mul => Op::Mul { dst, a, b },
            BinaryOp::Div => Op::Div { dst, a, b },
            BinaryOp::Mod => Op::Mod { dst, a, b },
            BinaryOp::Pow => Op::Pow { dst, a, b },
            BinaryOp::BitAnd => Op::BitAnd { dst, a, b },
            BinaryOp::BitOr => Op::BitOr { dst, a, b },
            BinaryOp::BitXor => Op::BitXor { dst, a, b },
            BinaryOp::Shl => Op::Shl { dst, a, b },
            BinaryOp::Shr => Op::Shr { dst, a, b },
            BinaryOp::UShr => Op::UShr { dst, a, b },
            BinaryOp::Lt => Op::Lt { dst, a, b },
            BinaryOp::Le => Op::Le { dst, a, b },
            BinaryOp::Gt => Op::Gt { dst, a, b },
            BinaryOp::Ge => Op::Ge { dst, a, b },
            BinaryOp::Eq => Op::Eq { dst, a, b },
            BinaryOp::Ne => Op::Ne { dst, a, b },
            BinaryOp::StrictEq => Op::StrictEq { dst, a, b },
            BinaryOp::StrictNe => Op::StrictNe { dst, a, b },
            BinaryOp::In => Op::In { dst, a, b },
            BinaryOp::InstanceOf => Op::InstanceOf { dst, a, b },
        }
    }

    fn assign(
        &mut self,
        op: AssignOp,
        target: &'a Pattern<'a>,
        value: &'a Expr<'a>,
        dst: u16,
    ) -> Compiled<()> {
        let mark = self.mark();
        match op {
            AssignOp::Plain => match target {
                Pattern::Ident(name, _) => {
                    let atom = self.atom_of_key(name);
                    let temp = self.alloc()?;
                    self.expr_named(value, temp, Some(atom))?;
                    self.write_var(name, temp)?;
                    self.give_result(dst, temp);
                }
                Pattern::Expr(Expr::Member { object, name, .. }) => {
                    let object_reg = if may_assign(value, 8) {
                        let temp = self.alloc()?;
                        self.expr(object, temp)?;
                        temp
                    } else {
                        self.expr_reg(object)?
                    };
                    let temp = if matches!(value, Expr::Ident(..)) {
                        self.expr_reg(value)?
                    } else {
                        let atom = self.atom_of_key(name);
                        let temp = self.alloc()?;
                        self.expr_named(value, temp, Some(atom))?;
                        temp
                    };
                    self.set_named(object_reg, temp, name)?;
                    self.give_result(dst, temp);
                }
                Pattern::Expr(Expr::Index { object, index, .. }) => {
                    let later_may_assign = may_assign(value, 8);
                    let object_reg = if later_may_assign || may_assign(index, 8) {
                        let temp = self.alloc()?;
                        self.expr(object, temp)?;
                        temp
                    } else {
                        self.expr_reg(object)?
                    };
                    let key = if later_may_assign {
                        let temp = self.alloc()?;
                        self.expr(index, temp)?;
                        temp
                    } else {
                        self.expr_reg(index)?
                    };
                    let temp = self.expr_reg(value)?;
                    self.emit(Op::SetElem {
                        object: object_reg,
                        key,
                        value: temp,
                    });
                    self.give_result(dst, temp);
                }
                pattern => {
                    let temp = self.alloc()?;
                    self.expr(value, temp)?;
                    self.bind(pattern, temp, false)?;
                    self.give_result(dst, temp);
                }
            },
            AssignOp::Binary(binary) => {
                if let Pattern::Ident(name, _) = target {
                    if let Place::Register {
                        reg,
                        started: true,
                        kind: VarKind::Var | VarKind::Let,
                    } = self.resolve(name)?
                    {
                        if self.with_chain(name).is_empty() && !may_assign(value, 8) {
                            self.binary_with_left(binary, reg, value, reg)?;
                            self.give_result(dst, reg);
                            self.release(mark);
                            return Ok(());
                        }
                    }
                }
                let result = self.alloc()?;
                match target {
                    Pattern::Ident(name, _) => {
                        let old = self.alloc()?;
                        self.read_var(name, old)?;
                        self.binary_with_left(binary, old, value, result)?;
                        self.write_var(name, result)?;
                    }
                    Pattern::Expr(Expr::Member { object, name, .. }) => {
                        let object = self.expr_reg(object)?;
                        let old = self.alloc()?;
                        self.get_named(old, object, name)?;
                        self.binary_with_left(binary, old, value, result)?;
                        self.set_named(object, result, name)?;
                    }
                    Pattern::Expr(Expr::Index { object, index, .. }) => {
                        let (object, key) = self.binary_operands(object, index)?;
                        let old = self.alloc()?;
                        self.emit(Op::GetElem {
                            dst: old,
                            object,
                            key,
                        });
                        self.binary_with_left(binary, old, value, result)?;
                        self.emit(Op::SetElem {
                            object,
                            key,
                            value: result,
                        });
                    }
                    _ => return self.error("that can't be assigned to", 0),
                }
                self.give_result(dst, result);
            }
            AssignOp::Logical(logical) => {
                let result = self.alloc()?;
                match target {
                    Pattern::Ident(name, _) => {
                        self.read_var(name, result)?;
                        let skip = self.logical_jump(logical, result);
                        let atom = self.atom_of_key(name);
                        self.expr_named(value, result, Some(atom))?;
                        self.write_var(name, result)?;
                        self.patch(skip);
                    }
                    Pattern::Expr(Expr::Member { object, name, .. }) => {
                        let object = self.expr_reg(object)?;
                        self.get_named(result, object, name)?;
                        let skip = self.logical_jump(logical, result);
                        self.expr(value, result)?;
                        self.set_named(object, result, name)?;
                        self.patch(skip);
                    }
                    Pattern::Expr(Expr::Index { object, index, .. }) => {
                        let (object, key) = self.binary_operands(object, index)?;
                        self.emit(Op::GetElem {
                            dst: result,
                            object,
                            key,
                        });
                        let skip = self.logical_jump(logical, result);
                        self.expr(value, result)?;
                        self.emit(Op::SetElem {
                            object,
                            key,
                            value: result,
                        });
                        self.patch(skip);
                    }
                    _ => return self.error("that can't be assigned to", 0),
                }
                self.give_result(dst, result);
            }
        }
        self.release(mark);
        Ok(())
    }

    /// Works out what's being called, and what `this` will be.
    fn callee(&mut self, callee: &'a Expr<'a>, f: u16, this: u16) -> Compiled<()> {
        if matches!(callee, Expr::Member { optional: false, .. } | Expr::Index { optional: false, .. })
            && self.noted_chain(callee, f, Some(this))?
        {
            return Ok(());
        }
        match callee {
            Expr::Member {
                object,
                name,
                optional,
            } => {
                self.expr(object, this)?;
                if *optional {
                    self.chain_jump(this);
                }
                self.get_named(f, this, name)?;
            }
            Expr::Index {
                object,
                index,
                optional,
            } => {
                self.expr(object, this)?;
                if *optional {
                    self.chain_jump(this);
                }
                let mark = self.mark();
                let key = self.expr_reg(index)?;
                self.emit(Op::GetElem {
                    dst: f,
                    object: this,
                    key,
                });
                self.release(mark);
            }
            Expr::SuperMember(_) | Expr::SuperIndex(_) => {
                self.read_this(this)?;
                self.expr(callee, f)?;
            }
            other => {
                self.expr(other, f)?;
                self.emit(Op::LoadUndefined { dst: this });
            }
        }
        Ok(())
    }

    /// Puts each argument in the next register, and says how many there were.
    fn plain_args(&mut self, args: &'a [Element<'a>]) -> Compiled<u16> {
        for arg in args {
            let Element::Plain(value) = arg else {
                return self.error("a spread came where it shouldn't", 0);
            };
            let reg = self.alloc()?;
            self.expr_fresh(value, reg)?;
        }
        Ok(args.len() as u16)
    }

    /// [`plain_args`](Self::plain_args) for a call: what's noted is read as itself, as a call can keep it or compare it, and
    /// the arrays given to a hook (`useEffect` and the rest) are its dependencies, which are compared as themselves.
    fn call_args(&mut self, callee: &'a Expr<'a>, args: &'a [Element<'a>]) -> Compiled<u16> {
        if !self.f().noting {
            return self.plain_args(args);
        }
        let named = match callee {
            Expr::Ident(name, _) => Some(*name),
            Expr::Member { name, .. } => Some(*name),
            _ => None,
        };
        let hook = named.is_some_and(|name| name.starts_with("use"));
        // What's given that's noted waits to be noted until it's known what it's given to: see `NOTE_ARG`.
        let notes_before = std::mem::take(&mut self.f().arg_notes);
        let mut first = None;
        // Only a component's element: a tag's props are drawn, and nothing reads them to say what of them matters.
        let jsx = matches!(callee, Expr::Ident("__jsxMemo" | "__jsxPageMemo", _))
            && !matches!(args.first(), Some(Element::Plain(Expr::String(_))));
        for (at, arg) in args.iter().enumerate() {
            let Element::Plain(value) = arg else {
                return self.error("a spread came where it shouldn't", 0);
            };
            let reg = self.alloc()?;
            first.get_or_insert(reg);
            if hook && matches!(value, Expr::Array(_)) {
                self.itself_items = true;
                self.expr_fresh(value, reg)?;
                self.itself_items = false;
            } else if jsx && at == 1 && matches!(value, Expr::Object(_)) {
                // An element's props: what's given as one is read as what it's given to reads it, where that's noted.
                self.f().jsx_props_next = true;
                let done = self.expr_itself(value, reg);
                self.f().jsx_props_next = false;
                done?;
            } else {
                let noted = self.may_note(value)?;
                self.f().arg_next = noted;
                let done = self.expr_itself(value, reg);
                self.f().arg_next = false;
                done?;
            }
        }
        let noted = std::mem::replace(&mut self.f().arg_notes, notes_before);
        if let (true, Some(first)) = (noted > 0, first) {
            // Last before the call, which is what's given to it.
            self.emit_note(first, args.len() as u16, NOTE_CALLING, &[]);
        }
        Ok(args.len() as u16)
    }

    fn args_array(&mut self, args: &'a [Element<'a>]) -> Compiled<u16> {
        let array = self.alloc()?;
        self.emit(Op::NewArray {
            dst: array,
            capacity: args.len() as u32,
        });
        for arg in args {
            let mark = self.mark();
            match arg {
                Element::Plain(value) => {
                    let value = self.expr_reg(value)?;
                    self.emit(Op::ArrayPush { array, value });
                }
                Element::Spread(value) => {
                    let source = self.expr_reg(value)?;
                    self.emit(Op::ArraySpread { array, source });
                }
            }
            self.release(mark);
        }
        Ok(array)
    }

    fn call(
        &mut self,
        callee: &'a Expr<'a>,
        args: &'a [Element<'a>],
        optional: bool,
        line: u32,
        dst: u16,
    ) -> Compiled<()> {
        let mark = self.mark();
        let f = self.alloc()?;
        let this = self.alloc()?;
        let has_spread = args.iter().any(|arg| matches!(arg, Element::Spread(_)));
        if !has_spread
            && !matches!(
                callee,
                Expr::Member { .. }
                    | Expr::Index { .. }
                    | Expr::SuperMember(_)
                    | Expr::SuperIndex(_)
            )
        {
            self.expr(callee, f)?;
            if optional {
                self.chain_jump(f);
            }
            let argc = self.call_args(callee, args)?;
            self.mark_line(line);
            // In a component's or hook's own body, a call of a function by name whose answer is used can give what it gave
            // at the last render (see `keeping.rs`). A hook is called every time, as React counts them.
            let keep = !optional
                && dst != DISCARD
                && self.f().keeps
                && matches!(callee, Expr::Ident(name, _) if !is_hook_name(name) && !name.starts_with("__"));
            if keep {
                self.emit(Op::Note {
                    value: dst,
                    root: f,
                    count: 1,
                    kind: NOTE_KEEP,
                });
                self.emit(Op::NoteKey { reg: argc });
            }
            self.emit(Op::CallPlain { dst, f, argc });
            if keep {
                self.emit(Op::Note {
                    value: dst,
                    root: f,
                    count: 0,
                    kind: NOTE_KEPT,
                });
            }
            self.release(mark);
            return Ok(());
        }
        // `object.name()` reads the method and calls it in one instruction. With arguments, they come between the two.
        if let Expr::Member {
            object,
            name,
            optional: false,
        } = callee
            && args.is_empty()
            && !optional
            && !self.may_note(object)?
        {
            let atom = self.atom_of_key(name);
            if atom != text::LENGTH {
                self.expr(object, this)?;
                let cache = self.cache(atom)?;
                self.mark_line(line);
                self.emit(Op::CallMethod { dst, f, cache });
                self.release(mark);
                return Ok(());
            }
        }
        // What a lookup of what's noted gives is noted after it: see `NOTE_RESULT`.
        let result_noted = !has_spread
            && dst != DISCARD
            && matches!(callee, Expr::Member { optional: false, .. } | Expr::Index { optional: false, .. })
            && self.may_note(callee)?;
        self.callee(callee, f, this)?;
        if optional {
            self.chain_jump(f);
        }
        if has_spread {
            let array = self.args_array(args)?;
            self.mark_line(line);
            self.emit(Op::CallSpread { dst, f, array });
        } else {
            let argc = self.call_args(callee, args)?;
            self.mark_line(line);
            // Whatever `Math` is when it runs, as CallMath looks at what it calls.
            let which = match callee {
                Expr::Member {
                    object: Expr::Ident("Math", _),
                    name,
                    optional: false,
                } if !optional => crate::builtins::math_quick_index(name, argc),
                _ => None,
            };
            self.emit(match which {
                Some(which) => Op::CallMath { dst, f, which },
                None => Op::Call { dst, f, argc },
            });
            if result_noted && which.is_none() {
                self.emit(Op::Note {
                    value: dst,
                    root: f,
                    count: 1,
                    kind: NOTE_RESULT,
                });
                // How many arguments there were, as a key's register.
                self.emit(Op::NoteKey { reg: argc });
            }
        }
        self.release(mark);
        Ok(())
    }

    fn super_call(&mut self, args: &'a [Element<'a>], dst: u16) -> Compiled<()> {
        let mark = self.mark();
        let f = self.alloc()?;
        let this = self.alloc()?;
        // A constructor that isn't written hands its parent the arguments it was given, as they are, without going
        // through them as a spread would (which would ask the array for its iterator).
        if let [Element::Spread(Expr::Ident(crate::parser::DEFAULT_ARGS, _))] = args
            && self.f().kind == FunctionKind::DerivedConstructor
        {
            let array = self.alloc()?;
            self.read_var(crate::parser::DEFAULT_ARGS, array)?;
            self.emit(Op::SuperCallSpread {
                dst: this,
                f,
                array,
            });
        } else if args.iter().any(|arg| matches!(arg, Element::Spread(_))) {
            let array = self.args_array(args)?;
            self.emit(Op::SuperCallSpread {
                dst: this,
                f,
                array,
            });
        } else {
            let argc = self.plain_args(args)?;
            self.emit(Op::SuperCall { dst: this, f, argc });
        }
        // `this` starts now, wherever it's kept.
        let state = self.f();
        if state.kind == FunctionKind::Arrow {
            self.write_var("this", this)?;
        } else {
            if let Some(cell) = state.this_cell {
                self.emit(Op::InitCell { cell, src: this });
            }
            self.emit(Op::Move { dst: 0, src: this });
            if self.f().has_fields {
                self.emit(Op::RunFields { this });
            }
        }
        self.emit(Op::Move { dst, src: this });
        self.release(mark);
        Ok(())
    }

    fn class(&mut self, class: &'a Class<'a>, dst: u16, hint: Option<Atom>) -> Compiled<()> {
        let mark = self.mark();
        self.class_depth += 1;
        // A class can always use its own name, whatever happens to the variable outside.
        let scope = self.open_block();
        if let Some(name) = class.name {
            self.declare(name, VarKind::Const, false)?;
        }
        let parent = self.alloc()?;
        match &class.parent {
            Some(expression) => self.expr(expression, parent)?,
            None => {
                self.emit(Op::LoadEmpty { dst: parent });
            }
        }
        // These are only for what's inside it, and what it extends is outside.
        let mut names: Vec<(&'a str, Atom)> = Vec::new();
        for name in class.private_names {
            if !names.iter().any(|(known, _)| known == name) {
                names.push((*name, self.vm.new_private_atom(name)));
            }
        }
        self.private_scopes.push(names);
        let is_private =
            |key: &PropertyKey<'a>| matches!(key, PropertyKey::Name(name) if name.starts_with('#'));
        // These go on each instance as it's made, as what inherits from one doesn't get to use them.
        let has_private_methods = class.members.iter().any(|member| match member {
            ClassMember::Method { key, is_static, .. }
            | ClassMember::Getter { key, is_static, .. }
            | ClassMember::Setter { key, is_static, .. } => !is_static && is_private(key),
            ClassMember::StaticInit(_) => false,
        });
        let constructor = self.alloc()?;
        let name_atom = match class.name {
            Some(name) => Some(self.intern(name)),
            None => hint,
        };
        let has_fields = class.fields.is_some() || has_private_methods;
        self.make_closure(class.constructor, constructor, name_atom, has_fields)?;
        let prototype = self.alloc()?;
        self.emit(Op::NewClass {
            dst: prototype,
            constructor,
            parent,
        });
        if let Some(name) = class.name {
            self.init_var(name, constructor)?;
        }
        let private_methods = self.alloc()?;
        if has_private_methods {
            self.emit(Op::NewObject {
                dst: private_methods,
                capacity: 0,
            });
            self.emit(Op::SetPrivateMethods {
                constructor,
                methods: private_methods,
            });
        }
        for member in class.members.iter() {
            let inner = self.mark();
            match member {
                ClassMember::Method {
                    key,
                    function,
                    is_static,
                }
                | ClassMember::Getter {
                    key,
                    function,
                    is_static,
                }
                | ClassMember::Setter {
                    key,
                    function,
                    is_static,
                } => {
                    let home = if *is_static { constructor } else { prototype };
                    let object = if !*is_static && is_private(key) {
                        private_methods
                    } else {
                        home
                    };
                    let key_reg = self.alloc()?;
                    self.property_key(key, key_reg)?;
                    let closure = self.alloc()?;
                    let prefix = match member {
                        ClassMember::Getter { .. } => 1,
                        ClassMember::Setter { .. } => 2,
                        _ => 0,
                    };
                    let name_hint = match prefix {
                        1 => self.accessor_hint(key, "get"),
                        2 => self.accessor_hint(key, "set"),
                        _ => self.key_hint(key),
                    };
                    self.make_closure(function, closure, name_hint, false)?;
                    if name_hint.is_none() {
                        self.name_from_key(closure, key_reg, prefix)?;
                    }
                    self.emit(Op::SetHome {
                        function: closure,
                        home,
                    });
                    self.emit(match member {
                        ClassMember::Getter { .. } => Op::DefineClassGetter {
                            object,
                            key: key_reg,
                            function: closure,
                        },
                        ClassMember::Setter { .. } => Op::DefineClassSetter {
                            object,
                            key: key_reg,
                            function: closure,
                        },
                        _ => Op::DefineMethod {
                            object,
                            key: key_reg,
                            function: closure,
                        },
                    });
                }
                ClassMember::StaticInit(function) => {
                    let f = self.alloc()?;
                    let this = self.alloc()?;
                    self.make_closure(function, f, None, false)?;
                    self.emit(Op::SetHome {
                        function: f,
                        home: constructor,
                    });
                    self.emit(Op::Move {
                        dst: this,
                        src: constructor,
                    });
                    self.emit(Op::Call { dst: f, f, argc: 0 });
                }
            }
            self.release(inner);
        }
        if let Some(fields) = &class.fields {
            let closure = self.alloc()?;
            self.make_closure(fields, closure, None, false)?;
            self.emit(Op::SetHome {
                function: closure,
                home: prototype,
            });
            self.emit(Op::SetFields {
                constructor,
                function: closure,
            });
        }
        self.emit(Op::Move {
            dst,
            src: constructor,
        });
        self.private_scopes.pop();
        self.close_block(scope);
        self.release(mark);
        self.class_depth -= 1;
        Ok(())
    }
}

/// The statements that `break` and `continue` can go to, as compiling keeps them, for [`Compiler::check_function`].
#[derive(Default)]
struct Targets<'a> {
    open: Vec<(Vec<&'a str>, TargetKind)>,
    pending_labels: Vec<&'a str>,
}

impl<'a> Targets<'a> {
    fn push(&mut self, kind: TargetKind) {
        let labels = std::mem::take(&mut self.pending_labels);
        self.open.push((labels, kind));
    }
}

/// Finding the mistakes in a function that waits to be compiled, which compiling it would have found. Each of these
/// follows what compiling does with the same part of the tree, and says what it would have said.
impl<'a> Compiler<'a, '_> {
    fn check_function(&self, function: &'a Function<'a>) -> Compiled<()> {
        self.check_depth()?;
        for param in function.params {
            if let Some(default) = &param.default {
                self.check_expr(default)?;
            }
            self.check_binding(&param.target)?;
        }
        if let Some(rest) = &function.rest {
            self.check_binding(rest)?;
        }
        self.check_redeclarations(function.body, true)?;
        let mut targets = Targets::default();
        self.check_statements(function.body, &mut targets)
    }

    fn check_statements(
        &self,
        statements: &'a [Stmt<'a>],
        targets: &mut Targets<'a>,
    ) -> Compiled<()> {
        for statement in statements {
            self.check_statement(statement, targets)?;
        }
        Ok(())
    }

    fn check_scoped(&self, statements: &'a [Stmt<'a>], targets: &mut Targets<'a>) -> Compiled<()> {
        self.check_redeclarations(statements, false)?;
        self.check_statements(statements, targets)
    }

    fn check_target(
        &self,
        label: Option<&str>,
        is_continue: bool,
        targets: &Targets<'a>,
    ) -> Compiled<()> {
        for (labels, kind) in targets.open.iter().rev() {
            let matches = match label {
                Some(label) => labels.contains(&label),
                None => match kind {
                    TargetKind::Loop => true,
                    TargetKind::Switch => !is_continue,
                    TargetKind::Block => false,
                },
            };
            if matches {
                return Ok(());
            }
        }
        self.error(
            if is_continue {
                "continue isn't inside a loop"
            } else {
                "break isn't inside a loop"
            },
            0,
        )
    }

    fn check_statement(&self, statement: &'a Stmt<'a>, targets: &mut Targets<'a>) -> Compiled<()> {
        self.check_depth()?;
        match statement {
            Stmt::Empty
            | Stmt::Line(_)
            | Stmt::Import(_)
            | Stmt::Export(_)
            | Stmt::ExportFrom(_) => {}
            Stmt::Function(function) => self.check_function(function)?,
            Stmt::Expr(expression) => self.check_expr(expression)?,
            Stmt::Throw(expression) | Stmt::ExportDefault(expression) => {
                self.check_expr(expression)?;
            }
            Stmt::Return(value) => {
                if let Some(value) = value {
                    self.check_expr(value)?;
                }
            }
            Stmt::Var(_, declarations) => self.check_declarations(declarations)?,
            Stmt::Class(class) => self.check_class(class)?,
            Stmt::Block(body) => self.check_scoped(body, targets)?,
            Stmt::If(test, then, otherwise) => {
                self.check_expr(test)?;
                self.check_statement(then, targets)?;
                if let Some(otherwise) = otherwise {
                    self.check_statement(otherwise, targets)?;
                }
            }
            Stmt::For(parts) => {
                let labels = std::mem::take(&mut targets.pending_labels);
                match &parts.init {
                    Some(ForInit::Var(_, declarations)) => self.check_declarations(declarations)?,
                    Some(ForInit::Expr(expression)) => self.check_expr(expression)?,
                    None => {}
                }
                targets.pending_labels = labels;
                targets.push(TargetKind::Loop);
                if let Some(test) = &parts.test {
                    self.check_expr(test)?;
                }
                self.check_statement(parts.body, targets)?;
                targets.open.pop();
                if let Some(update) = &parts.update {
                    self.check_expr(update)?;
                }
            }
            Stmt::ForIn(parts) => {
                self.check_for_each(&parts.target, &parts.object, parts.body, targets)?
            }
            Stmt::ForOf(parts) => {
                self.check_for_each(&parts.target, &parts.iterable, parts.body, targets)?;
            }
            Stmt::With(object, body) => {
                self.check_expr(object)?;
                self.check_statement(body, targets)?;
            }
            Stmt::While(test, body) => {
                targets.push(TargetKind::Loop);
                self.check_expr(test)?;
                self.check_statement(body, targets)?;
                targets.open.pop();
            }
            Stmt::DoWhile(body, test) => {
                targets.push(TargetKind::Loop);
                self.check_statement(body, targets)?;
                targets.open.pop();
                self.check_expr(test)?;
            }
            Stmt::Break(label) => self.check_target(*label, false, targets)?,
            Stmt::Continue(label) => self.check_target(*label, true, targets)?,
            Stmt::Try(parts) => {
                self.check_scoped(parts.block, targets)?;
                if let Some(handler) = parts.handler {
                    if let Some(param) = &parts.param {
                        let mut names = Vec::new();
                        pattern_names(param, &mut names);
                        for statement in handler {
                            let mut declared = Vec::new();
                            match statement {
                                Stmt::Var(kind, declarations) if *kind != VarKind::Var => {
                                    for (pattern, _) in declarations.iter() {
                                        pattern_names(pattern, &mut declared);
                                    }
                                }
                                Stmt::Class(class) => declared.extend(class.name),
                                Stmt::Function(function) => declared.extend(function.name),
                                _ => {}
                            }
                            if let Some(found) = declared.iter().find(|found| names.contains(found))
                            {
                                return self
                                    .error(format!("{found:?} has already been declared"), 0);
                            }
                        }
                        self.check_binding(param)?;
                    }
                    self.check_scoped(handler, targets)?;
                }
                if let Some(finalizer) = parts.finalizer {
                    self.check_scoped(finalizer, targets)?;
                }
            }
            Stmt::Switch(value, cases) => {
                self.check_expr(value)?;
                targets.push(TargetKind::Switch);
                for case in cases.iter() {
                    if let Some(test) = &case.test {
                        self.check_expr(test)?;
                    }
                    self.check_statements(case.body, targets)?;
                }
                targets.open.pop();
            }
            Stmt::Labeled(name, body) => {
                targets.pending_labels.push(name);
                match **body {
                    Stmt::For { .. }
                    | Stmt::ForIn { .. }
                    | Stmt::ForOf { .. }
                    | Stmt::While(..)
                    | Stmt::DoWhile(..)
                    | Stmt::Labeled(..) => self.check_statement(body, targets)?,
                    _ => {
                        targets.push(TargetKind::Block);
                        self.check_statement(body, targets)?;
                        targets.open.pop();
                    }
                }
            }
            Stmt::Field(key, value) => {
                if let Some(value) = value {
                    self.check_expr(value)?;
                }
                self.check_key(key)?;
            }
        }
        Ok(())
    }

    fn check_for_each(
        &self,
        target: &'a ForTarget<'a>,
        source: &'a Expr<'a>,
        body: &'a Stmt<'a>,
        targets: &mut Targets<'a>,
    ) -> Compiled<()> {
        let labels = std::mem::take(&mut targets.pending_labels);
        self.check_expr(source)?;
        targets.pending_labels = labels;
        targets.push(TargetKind::Loop);
        match target {
            ForTarget::Declare(_, pattern) => self.check_binding(pattern)?,
            ForTarget::Assign(pattern) => self.check_assign_target(pattern)?,
        }
        self.check_statement(body, targets)?;
        targets.open.pop();
        Ok(())
    }

    fn check_declarations(
        &self,
        declarations: &'a [(Pattern<'a>, Option<Expr<'a>>)],
    ) -> Compiled<()> {
        for (pattern, value) in declarations {
            match (pattern, value) {
                (Pattern::Ident(..), None) => {}
                (_, Some(value)) => {
                    self.check_expr(value)?;
                    self.check_binding(pattern)?;
                }
                (_, None) => return self.error("a pattern needs a value", 0),
            }
        }
        Ok(())
    }

    fn check_key(&self, key: &'a PropertyKey<'a>) -> Compiled<()> {
        if let PropertyKey::Computed(expression) = key {
            self.check_expr(expression)?;
        }
        Ok(())
    }

    /// A pattern that declares names, whose defaults and computed keys are code.
    fn check_binding(&self, pattern: &'a Pattern<'a>) -> Compiled<()> {
        match pattern {
            Pattern::Ident(..) => Ok(()),
            Pattern::Expr(_) => self.check_assign_target(pattern),
            Pattern::Array { items, rest } => {
                for item in items.iter().flatten() {
                    self.check_binding(&item.target)?;
                    if let Some(default) = &item.default {
                        self.check_expr(default)?;
                    }
                }
                if let Some(rest) = rest {
                    self.check_binding(rest)?;
                }
                Ok(())
            }
            Pattern::Object { items, rest } => {
                for (key, item) in items.iter() {
                    self.check_key(key)?;
                    self.check_binding(&item.target)?;
                    if let Some(default) = &item.default {
                        self.check_expr(default)?;
                    }
                }
                if let Some(rest) = rest {
                    self.check_binding(rest)?;
                }
                Ok(())
            }
        }
    }

    /// Where an `=` puts something, as `assign_to_expr` takes it.
    fn check_assign_target(&self, pattern: &'a Pattern<'a>) -> Compiled<()> {
        match pattern {
            Pattern::Expr(target) => {
                if !matches!(
                    target,
                    Expr::Member { .. }
                        | Expr::Index { .. }
                        | Expr::SuperMember(_)
                        | Expr::SuperIndex(_)
                ) {
                    return self.error("that can't be assigned to", 0);
                }
                self.check_expr(target)
            }
            _ => self.check_binding(pattern),
        }
    }

    fn check_class(&self, class: &'a Class<'a>) -> Compiled<()> {
        if let Some(parent) = &class.parent {
            self.check_expr(parent)?;
        }
        self.check_function(class.constructor)?;
        for member in class.members.iter() {
            match member {
                ClassMember::Method { key, function, .. }
                | ClassMember::Getter { key, function, .. }
                | ClassMember::Setter { key, function, .. } => {
                    self.check_key(key)?;
                    self.check_function(function)?;
                }
                ClassMember::StaticInit(function) => self.check_function(function)?,
            }
        }
        if let Some(fields) = class.fields {
            self.check_function(fields)?;
        }
        Ok(())
    }

    fn check_elements(&self, elements: &'a [Element<'a>]) -> Compiled<()> {
        for element in elements {
            match element {
                Element::Plain(value) | Element::Spread(value) => self.check_expr(value)?,
            }
        }
        Ok(())
    }

    fn check_expr(&self, expression: &'a Expr<'a>) -> Compiled<()> {
        self.check_depth()?;
        match expression {
            Expr::Number(_)
            | Expr::BigInt(..)
            | Expr::String(_)
            | Expr::Regex(_)
            | Expr::Ident(..)
            | Expr::PrivateName(_)
            | Expr::ImportMeta
            | Expr::This
            | Expr::Null
            | Expr::True
            | Expr::False
            | Expr::NewTarget
            | Expr::SuperMember(_) => {}
            Expr::Template(parts) => {
                for value in parts.values {
                    self.check_expr(value)?;
                }
            }
            Expr::TaggedTemplate(parts) => {
                self.check_expr(parts.tag)?;
                for value in parts.values {
                    self.check_expr(value)?;
                }
            }
            Expr::ImportCall(value)
            | Expr::Unary(_, value)
            | Expr::Chain(value)
            | Expr::SuperIndex(value)
            | Expr::Await(value)
            | Expr::Member { object: value, .. } => self.check_expr(value)?,
            Expr::Array(items) => {
                for item in items.iter().flatten() {
                    match item {
                        Element::Plain(value) | Element::Spread(value) => self.check_expr(value)?,
                    }
                }
            }
            Expr::Object(properties) => {
                for property in properties.iter() {
                    match property {
                        Property::Value(key, value) => {
                            self.check_key(key)?;
                            self.check_expr(value)?;
                        }
                        Property::Short(..) => {}
                        Property::Method(key, function)
                        | Property::Getter(key, function)
                        | Property::Setter(key, function) => {
                            self.check_key(key)?;
                            self.check_function(function)?;
                        }
                        Property::Spread(value) => self.check_expr(value)?,
                    }
                }
            }
            Expr::Function(function) => self.check_function(function)?,
            Expr::Class(class) => self.check_class(class)?,
            Expr::Update { target, .. } => {
                if !matches!(
                    target,
                    Expr::Ident(..) | Expr::Member { .. } | Expr::Index { .. }
                ) {
                    return self.error("that can't be changed", 0);
                }
                self.check_expr(target)?;
            }
            Expr::Binary(_, a, b) | Expr::Logical(_, a, b) => {
                self.check_expr(a)?;
                self.check_expr(b)?;
            }
            Expr::Assign { op, target, value } => {
                match op {
                    AssignOp::Plain => self.check_assign_target(target)?,
                    AssignOp::Binary(_) | AssignOp::Logical(_) => {
                        if !matches!(
                            target,
                            Pattern::Ident(..)
                                | Pattern::Expr(Expr::Member { .. })
                                | Pattern::Expr(Expr::Index { .. })
                        ) {
                            return self.error("that can't be assigned to", 0);
                        }
                        self.check_binding(target)?;
                    }
                }
                self.check_expr(value)?;
            }
            Expr::Conditional(a, b, c) => {
                self.check_expr(a)?;
                self.check_expr(b)?;
                self.check_expr(c)?;
            }
            Expr::Call { callee, args, .. } | Expr::New { callee, args, .. } => {
                self.check_expr(callee)?;
                self.check_elements(args)?;
            }
            Expr::Index { object, index, .. } => {
                self.check_expr(object)?;
                self.check_expr(index)?;
            }
            Expr::SuperCall(args) => self.check_elements(args)?,
            Expr::Sequence(values) => {
                for value in values.iter() {
                    self.check_expr(value)?;
                }
            }
            Expr::Yield { value, delegate } => {
                if *delegate && value.is_none() {
                    return self.error("yield* needs something to yield from", 0);
                }
                if let Some(value) = value {
                    self.check_expr(value)?;
                }
            }
        }
        Ok(())
    }
}

impl Vm {
    /// Compiles a function the first time it's called. See [`Uncompiled`].
    #[cold]
    #[inline(never)]
    pub(crate) fn compile_on_first_call(&mut self, proto: u32) -> crate::vm::Run<()> {
        let Some(waiting) = self.uncompiled.remove(&proto) else {
            return Ok(());
        };
        let file = self.protos[proto as usize].file;
        match Compiler::new(self, file).compile_waiting(&waiting, proto) {
            Ok(()) => Ok(()),
            Err(error) => {
                // It's still waiting, and says so again if it's called again.
                self.uncompiled.insert(proto, waiting);
                Err(self.report_syntax_error(&error, file))
            }
        }
    }
}
