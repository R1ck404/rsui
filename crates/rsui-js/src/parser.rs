//! Turns tokens into a tree. TypeScript's types are read and thrown away here, and JSX becomes calls.

use crate::ast::*;
use crate::lexer::{Lexer, Parsed, SyntaxError, Tok, Token};

const MAX_CHAIN_LINKS: u32 = 2000;

/// Words that can never be names.
fn is_reserved(word: &str) -> bool {
    matches!(
        word,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
    )
}

/// The private names a class has, each with what kind it is, and the ones used in it, each with its line.
type PrivateScope<'a> = (Vec<(&'a str, u8)>, Vec<(&'a str, u32)>);

/// What's known about the function being read.
#[derive(Default)]
struct FunctionState {
    /// The names it mentions itself, as [`Parser::name_id`] numbers them. One can be here twice, but not often.
    names: Vec<u32>,
    /// The names that functions inside it mention, as they're to be seen from here. One can be here twice too.
    inner: Vec<u32>,
    /// Which function this is, so that what's marked as being in its lists can be told from what's in another's.
    serial: u32,
    uses_arguments: bool,
    is_async: bool,
    is_generator: bool,
    is_arrow: bool,
    strict: bool,
    allows_super_property: bool,
    allows_super_call: bool,
    allows_new_target: bool,
    /// Whether this is what's in a class's static block or field, where `await` can't be a name.
    in_class_init: bool,
}

/// What `this`, `arguments` and `new.target` are numbered, which are always the first names.
const THIS: u32 = 0;
const ARGUMENTS: u32 = 1;
const NEW_TARGET: u32 = 2;

pub(crate) struct Parser<'a> {
    /// Where the first choice after the nearest `?` starts.
    then_at: u32,
    /// Whether JSX here calls `__jsxPage`, for a file written for a page: see [`parse_file`].
    page_jsx: bool,
    /// What's been seen of how this file's JSX is written, for whatever's reading it.
    jsx_seen: std::rc::Rc<std::cell::Cell<JsxSeen>>,
    memory: &'a AstMemory,
    lexer: Lexer<'a>,
    tok: Token,
    /// Where the token before `tok` ends, which is where a function that ended with it ends.
    prev_end: u32,
    functions: Vec<FunctionState>,
    /// A number for each name that's been mentioned, so a function's names are only looked up once each time.
    name_ids: std::collections::HashMap<&'a str, u32, foldhash::fast::FixedState>,
    /// Each numbered name.
    name_texts: Vec<&'a str>,
    /// Each numbered name's [`name_hash`], with its number, as [`Names`] are sorted by them.
    name_keys: Vec<u64>,
    /// For each name, the last function that it was put in the `names` of, and the last it was put in the `inner` of,
    /// by their serials, so it's only put in once.
    noted_in: Vec<u32>,
    merged_into: Vec<u32>,
    /// For each name, the last function that declared it for all of itself, by its serial (see `note_declared`).
    declared_in: Vec<u32>,
    /// For each name, the last function it was put in the [`Function::outer_names`] of, by its serial.
    listed_in: Vec<u32>,
    /// Where a function's [`Function::outer_names`] are gathered, kept to be used again.
    outer_scratch: Vec<&'a str>,
    /// The serial the next function begun gets.
    next_serial: u32,
    /// Where Rust's stack was when parsing began, and how much further it can go.
    stack_start: usize,
    stack_room: usize,
    /// How many `with`s this is inside of.
    with_depth: usize,
    /// Whether `return` can be used outside of a function, which is how source that's going in a function comes.
    allow_top_return: bool,
    /// Where `{ a = 1 }` was read, which is only all right if it turns out to be a pattern.
    pending_cover: Option<u32>,
    /// For each class being read: the private names it has, and the ones used that it may not have.
    private_scopes: Vec<PrivateScope<'a>>,
    /// How many links of `a + b + c` or `a.b.c` have been read. They don't nest here, but they do later on.
    chain_links: u32,
    /// Where each element is in the source, for what wants to change it there. Nothing that runs code asks for this.
    marks: Option<Box<JsxMarks>>,
    /// Whether this is reading a file as a function's body, for [`parse_file`].
    reading_wrapped: bool,
    /// Whether the next statement is at the top of the file, outside any block or function, which is the only place an
    /// `import` or `export` can be. [`Parser::statement_into`] takes it for the statement it reads.
    at_top: bool,
    statement_at_top: bool,
    /// Whether an `import` or `export` statement has been read at the top. See [`Program::says_module`].
    said_module: bool,
    /// Whether what's been read so far could only be in a function, and not at the top of a module.
    only_wrapped: bool,
    /// Whether each statement has a [`Stmt::Line`] in front of it, for a debugger to stop at.
    debug_lines: bool,
}

/// What a derived class's constructor that isn't written is given, which it hands to its parent's.
pub(crate) const DEFAULT_ARGS: &str = "%args";

/// Where an attribute of an element is in the source, in bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JsxAttributeMark {
    /// Where its name starts and ends.
    pub name: (u32, u32),
    /// Where its value starts and ends, with the quotes or braces, if it has one.
    pub value: Option<(u32, u32)>,
}

/// Where an element is in the source, in bytes. A name that's empty is a fragment's.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JsxMark {
    /// Where its `<` is.
    pub start: u32,
    /// Where its name starts and ends. For `a.b`, that's only the `a`.
    pub name: (u32, u32),
    pub attributes: Vec<JsxAttributeMark>,
    /// Just past the `>` of its opening tag, or the `/>` if that's all it has.
    pub open_end: u32,
    /// Where the name in its closing tag starts and ends, if it has one.
    pub close_name: Option<(u32, u32)>,
    /// Just past the last `>`.
    pub end: u32,
}

#[derive(Debug, Default)]
struct JsxMarks {
    elements: Vec<JsxMark>,
    /// What the closing tag just read had as a name.
    closing: (u32, u32),
}

/// Where to go back to, if what's being tried turns out not to be that.
#[derive(Clone)]
struct Checkpoint<'a> {
    lexer: Lexer<'a>,
    tok: Token,
    prev_end: u32,
    marks: usize,
}

fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Where every element in this source is, in the order they start.
pub(crate) fn jsx_marks(source: &str, stack_room: usize) -> Parsed<Vec<JsxMark>> {
    let memory = AstMemory::new();
    let mut parser = Parser::new(source, &memory, stack_room, false);
    parser.marks = Some(Box::default());
    parser.advance()?;
    parser.directives()?;
    let mut body = Vec::new();
    while !matches!(parser.tok.kind, Tok::Eof) {
        parser.at_top = true;
        parser.statement_into(&mut body)?;
    }
    let mut marks = parser.marks.take().unwrap_or_default().elements;
    // What's read twice is noted twice, as functions' bodies can be looked at to see what they are.
    marks.sort_by_key(|mark| mark.start);
    marks.dedup_by_key(|mark| mark.start);
    Ok(marks)
}

pub(crate) fn parse<'a>(
    source: &'a str,
    memory: &'a AstMemory,
    stack_room: usize,
    allow_top_return: bool,
) -> Parsed<Program<'a>> {
    as_page_if_written_for_one(|page_jsx, seen| {
        let mut parser = Parser::new(source, memory, stack_room, allow_top_return);
        parser.page_jsx = page_jsx;
        parser.jsx_seen = seen;
        parser.read_program()
    })
}

/// Reads an ES module, where `await` can be used outside of any function, as it waits for the whole module.
pub(crate) fn parse_module<'a>(
    source: &'a str,
    memory: &'a AstMemory,
    stack_room: usize,
) -> Parsed<Program<'a>> {
    as_page_if_written_for_one(|page_jsx, seen| {
        let mut parser = Parser::new(source, memory, stack_room, false);
        parser.page_jsx = page_jsx;
        parser.jsx_seen = seen;
        if let Some(top) = parser.functions.first_mut() {
            top.is_async = true;
        }
        parser.read_program()
    })
}

/// Reads code, then again with `page_jsx` if it turns out to be written for a page and to have a tag rsui has too, such as
/// `<button>` beside a `<div>`. Its JSX then calls `__jsxPage`, which takes each tag as a page's. Nothing else is read twice.
fn as_page_if_written_for_one<T>(
    mut read: impl FnMut(bool, std::rc::Rc<std::cell::Cell<JsxSeen>>) -> Parsed<T>,
) -> Parsed<T> {
    let seen = std::rc::Rc::new(std::cell::Cell::new(JsxSeen::default()));
    let first = read(false, seen.clone());
    let said = seen.get();
    if first.is_ok() && said.for_page && said.shared_tag {
        return read(true, seen);
    }
    first
}

impl<'a> Parser<'a> {
    fn read_program(mut self) -> Parsed<Program<'a>> {
        self.advance()?;
        let mut body = Vec::new();
        self.directives()?;
        while !matches!(self.tok.kind, Tok::Eof) {
            self.at_top = true;
            self.statement_into(&mut body)?;
        }
        let mut state = self.functions.pop().unwrap_or_default();
        Ok(Program {
            function: Function {
                name: None,
                kind: FunctionKind::Normal,
                is_async: false,
                is_generator: false,
                params: &[],
                rest: None,
                body: self.memory.list(&body),
                inner_names: self.sorted_names(&mut state.inner),
                outer_names: &[],
                uses_arguments: false,
                uses_own_name: false,
                uses_this: false,
                line: 1,
                source: (0, 0),
            },
            says_module: self.said_module,
        })
    }
}

/// What a file turned out to be, from [`parse_file`].
pub(crate) enum FileProgram<'a> {
    /// A script, as the file's code wrapped in a function that takes the parameters asked for, which the program
    /// returns: as if the file had been written `return (function (exports, ...) {` and `})` around it.
    Wrapped(Program<'a>),
    /// An ES module.
    Module(Program<'a>),
    /// It wasn't clear in one reading, as when there's a mistake in it, so it has to be read the slower way.
    Unsure,
}

/// What's been seen of how a file's JSX is written.
#[derive(Clone, Copy, Default)]
struct JsxSeen {
    /// Something only a page has: `className`, `htmlFor`, or a tag like `div`.
    for_page: bool,
    /// A tag that's rsui's as well as a page's: `span`, `button` or `input`.
    shared_tag: bool,
}

/// Tags that only a page has, which say a file's JSX is written for one.
const PAGE_ONLY_TAGS: &[&str] = &[
    "a", "article", "aside", "div", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "header", "img", "label", "li",
    "main", "nav", "ol", "p", "section", "table", "td", "textarea", "tr", "ul",
];

/// Reads a file once, whether it's a script that's run in a function, as CommonJS is, or, if `allow_module`, an ES
/// module. The two are read the same way up to the first `import` or `export`, so reading starts as a function's body
/// and carries on as a module from there, if there is one. With `allow_module`, a mistake gives [`FileProgram::Unsure`],
/// so that what it says is just what it would have said otherwise.
/// Its JSX is as [`as_page_if_written_for_one`] says.
pub(crate) fn parse_file<'a>(
    source: &'a str,
    memory: &'a AstMemory,
    stack_room: usize,
    params: &[&'static str],
    allow_module: bool,
) -> Parsed<FileProgram<'a>> {
    as_page_if_written_for_one(|page_jsx, seen| {
        parse_file_as(source, memory, stack_room, params, allow_module, page_jsx, seen)
    })
}

fn parse_file_as<'a>(
    source: &'a str,
    memory: &'a AstMemory,
    stack_room: usize,
    params: &[&'static str],
    allow_module: bool,
    page_jsx: bool,
    seen: std::rc::Rc<std::cell::Cell<JsxSeen>>,
) -> Parsed<FileProgram<'a>> {
    let mut parser = Parser::new(source, memory, stack_room, false);
    parser.page_jsx = page_jsx;
    parser.jsx_seen = seen;
    parser.begin_function(false, false, false);
    parser.reading_wrapped = true;
    let mut body = Vec::new();
    let read = (|| -> Parsed<bool> {
        parser.advance()?;
        parser.directives()?;
        while !matches!(parser.tok.kind, Tok::Eof) {
            if allow_module && parser.starts_module_syntax()? {
                return Ok(true);
            }
            parser.at_top = true;
            parser.statement_into(&mut body)?;
        }
        Ok(false)
    })();
    match read {
        Ok(false) => {}
        Ok(true) if !parser.only_wrapped => return Ok(parser.rest_as_module(body)),
        Ok(true) => return Ok(FileProgram::Unsure),
        Err(_) if allow_module => return Ok(FileProgram::Unsure),
        Err(error) => return Err(error),
    }
    // The body's own `let`s and `const`s can't have the name of a parameter, as in any function.
    for statement in &body {
        let mut declared = Vec::new();
        match statement {
            Stmt::Var(kind, declarations) if *kind != VarKind::Var => {
                for (pattern, _) in declarations.iter() {
                    collect_binding_names(pattern, &mut declared);
                }
            }
            Stmt::Class(class) => declared.extend(class.name),
            _ => {}
        }
        if let Some(found) = declared.iter().find(|found| params.contains(found)) {
            if allow_module {
                return Ok(FileProgram::Unsure);
            }
            return Err(SyntaxError {
                message: format!("{found:?} has already been declared"),
                line: 1,
            });
        }
    }
    let uses_this = parser.mentions("this");
    let (inner_names, uses_arguments, outer_names) = parser.end_function();
    let params: Vec<PatternItem<'a>> = params
        .iter()
        .map(|name| PatternItem {
            target: Pattern::Ident(name, 1),
            default: None,
        })
        .collect();
    let function = Function {
        name: None,
        kind: FunctionKind::Normal,
        is_async: false,
        is_generator: false,
        params: memory.list(&params),
        rest: None,
        body: memory.list(&body),
        inner_names,
        outer_names,
        uses_arguments,
        uses_own_name: false,
        uses_this,
        line: 1,
        source: (0, 0),
    };
    let returned = Stmt::Return(Some(memory.node(Expr::Function(memory.node(function)))));
    let mut state = parser.functions.pop().unwrap_or_default();
    Ok(FileProgram::Wrapped(Program {
        function: Function {
            name: None,
            kind: FunctionKind::Normal,
            is_async: false,
            is_generator: false,
            params: &[],
            rest: None,
            body: memory.list(&[returned]),
            inner_names: parser.sorted_names(&mut state.inner),
            outer_names: &[],
            uses_arguments: false,
            uses_own_name: false,
            uses_this: false,
            line: 1,
            source: (0, 0),
        },
        says_module: false,
    }))
}

#[cfg(test)]
thread_local! {
    /// How many times source code has been read on this thread, for tests that it's only read once.
    pub(crate) static PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl<'a> Parser<'a> {
    /// Whether this is a name and then `=`, as in TypeScript's `import x = require("x")`, after `import` or `import type`.
    fn next_is_equals_after_name(&mut self) -> Parsed<bool> {
        let saved = self.save();
        if self.is_word("type") {
            self.advance()?;
        }
        let found = matches!(self.tok.kind, Tok::Ident) && {
            self.advance()?;
            self.is("=")
        };
        self.restore(saved);
        Ok(found)
    }

    /// Whether the statement here is an `import` or `export` declaration, which only a module has.
    fn starts_module_syntax(&mut self) -> Parsed<bool> {
        if self.is_word("export") {
            return Ok(true);
        }
        if !self.is_word("import") {
            return Ok(false);
        }
        let saved = self.save();
        self.advance()?;
        let call_or_meta = self.is("(") || self.is(".");
        self.restore(saved);
        Ok(!call_or_meta)
    }

    /// Goes on reading a file as a module, from its first `import` or `export`, as [`parse_file`] does. What's been
    /// read so far reads the same in a module, as long as it didn't `return` or use `new.target`.
    fn rest_as_module(mut self, mut body: Vec<Stmt<'a>>) -> FileProgram<'a> {
        self.reading_wrapped = false;
        // What was read as the function's body is the module's, with what's known about it.
        let mut state = self
            .functions
            .pop()
            .expect("there's the function the file was read as");
        state.allows_new_target = false;
        // A module can wait at its top, for whatever comes after.
        state.is_async = true;
        self.functions.clear();
        self.functions.push(state);
        while !matches!(self.tok.kind, Tok::Eof) {
            self.at_top = true;
            if self.statement_into(&mut body).is_err() {
                return FileProgram::Unsure;
            }
        }
        let mut state = self.functions.pop().unwrap_or_default();
        let inner_names = self.sorted_names(&mut state.inner);
        let program = Program {
            function: Function {
                name: None,
                kind: FunctionKind::Normal,
                is_async: false,
                is_generator: false,
                params: &[],
                rest: None,
                body: self.memory.list(&body),
                inner_names,
                outer_names: &[],
                uses_arguments: false,
                uses_own_name: false,
                uses_this: false,
                line: 1,
                source: (0, 0),
            },
            says_module: self.said_module,
        };
        // TypeScript's `import x = require()` and `export =` aren't what makes a module, though `export type` is.
        if !crate::modules::has_module_syntax(&program) {
            return FileProgram::Unsure;
        }
        FileProgram::Module(program)
    }

    fn new(
        source: &'a str,
        memory: &'a AstMemory,
        stack_room: usize,
        allow_top_return: bool,
    ) -> Self {
        #[cfg(test)]
        PARSES.with(|parses| parses.set(parses.get() + 1));
        let mut parser = Parser {
            memory,
            lexer: Lexer::new(source),
            tok: Token {
                kind: Tok::Eof,
                start: 0,
                end: 0,
                line: 1,
                newline_before: false,
                escaped_name: None,
                legacy_octal: false,
            },
            prev_end: 0,
            functions: vec![FunctionState::default()],
            name_ids: std::collections::HashMap::default(),
            name_texts: Vec::new(),
            name_keys: Vec::new(),
            noted_in: Vec::new(),
            merged_into: Vec::new(),
            declared_in: Vec::new(),
            listed_in: Vec::new(),
            outer_scratch: Vec::new(),
            next_serial: 1,
            stack_start: crate::stack_position(),
            stack_room,
            with_depth: 0,
            allow_top_return,
            pending_cover: None,
            private_scopes: Vec::new(),
            chain_links: 0,
            then_at: u32::MAX,
            marks: None,
            reading_wrapped: false,
            at_top: false,
            statement_at_top: false,
            said_module: false,
            only_wrapped: false,
            debug_lines: crate::inspect::parsing_for_debugger(),
            page_jsx: false,
            jsx_seen: std::rc::Rc::default(),
        };
        for name in ["this", "arguments", "new.target"] {
            parser.name_id(name);
        }
        parser
    }

    fn advance(&mut self) -> Parsed<()> {
        self.prev_end = self.tok.end;
        self.tok = self.lexer.next()?;
        Ok(())
    }

    fn error<T>(&self, message: impl Into<String>) -> Parsed<T> {
        Err(SyntaxError {
            message: message.into(),
            line: self.tok.line,
        })
    }

    fn save(&self) -> Checkpoint<'a> {
        Checkpoint {
            lexer: self.lexer.clone(),
            tok: self.tok.clone(),
            prev_end: self.prev_end,
            marks: self.marks.as_ref().map_or(0, |marks| marks.elements.len()),
        }
    }

    fn restore(&mut self, saved: Checkpoint<'a>) {
        self.lexer = saved.lexer;
        self.tok = saved.tok;
        self.prev_end = saved.prev_end;
        if let Some(marks) = &mut self.marks {
            marks.elements.truncate(saved.marks);
        }
    }

    fn text(&self) -> &'a str {
        match self.tok.escaped_name {
            Some(name) => name,
            None => &self.lexer.source[self.tok.start as usize..self.tok.end as usize],
        }
    }

    /// What the string token here says, kept with the tree.
    fn string_value(&self) -> &'a [u16] {
        let (start, end) = (self.tok.start as usize, self.tok.end as usize);
        let inside = &self.lexer.source.as_bytes()[start + 1..end - 1];
        if !inside.iter().any(|&byte| byte == b'\\' || byte >= 0x80) {
            return self.memory.widened(inside);
        }
        self.memory.list(&crate::lexer::string_units(
            self.lexer.source,
            self.tok.start,
            self.tok.end,
        ))
    }

    fn is_strict(&self) -> bool {
        self.functions.last().is_some_and(|state| state.strict)
    }

    /// Looks ahead at the strings that start a body, for `"use strict"`. It doesn't go past any of them.
    fn directives(&mut self) -> Parsed<bool> {
        let saved = self.save();
        let mut found = false;
        while let Tok::String = self.tok.kind {
            let written =
                &self.lexer.source[self.tok.start as usize + 1..self.tok.end as usize - 1];
            let is_strict = written == "use strict";
            let had_octal = self.tok.legacy_octal;
            self.advance()?;
            let ends_statement = self.is(";")
                || self.is("}")
                || matches!(self.tok.kind, Tok::Eof)
                || self.tok.newline_before;
            if !ends_statement {
                break;
            }
            let _ = self.eat(";")?;
            if had_octal && (found || is_strict) {
                return self.error("old-style octal escapes can't be in strict code");
            }
            if is_strict {
                found = true;
            }
        }
        self.restore(saved);
        if found {
            self.state().strict = true;
        }
        Ok(found)
    }

    /// A word that can't be a name here, in the code as it stands.
    fn check_name(&self, name: &str) -> Parsed<()> {
        let state = self.functions.last();
        if self.is_strict()
            && matches!(
                name,
                "implements"
                    | "interface"
                    | "let"
                    | "package"
                    | "private"
                    | "protected"
                    | "public"
                    | "static"
                    | "yield"
            )
        {
            return self.error(format!("{name:?} can't be a name in strict code"));
        }
        if name == "yield" && state.is_some_and(|state| state.is_generator) {
            return self.error("\"yield\" can't be a name in a generator");
        }
        if name == "await" && state.is_some_and(|state| state.is_async || state.in_class_init) {
            return self.error("\"await\" can't be a name here");
        }
        Ok(())
    }

    #[inline]
    fn is(&self, punct: &str) -> bool {
        matches!(&self.tok.kind, Tok::Punct(found) if *found == punct)
    }

    #[inline]
    fn is_word(&self, word: &str) -> bool {
        matches!(self.tok.kind, Tok::Ident) && self.text() == word
    }

    fn eat(&mut self, punct: &str) -> Parsed<bool> {
        if self.is(punct) {
            self.advance()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn eat_word(&mut self, word: &str) -> Parsed<bool> {
        if self.is_word(word) {
            self.advance()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn expect(&mut self, punct: &str) -> Parsed<()> {
        if self.is(punct) {
            self.advance()
        } else {
            self.error(format!("expected {punct} here, not {:?}", self.text()))
        }
    }

    fn semicolon(&mut self) -> Parsed<()> {
        if self.eat(";")?
            || self.is("}")
            || matches!(self.tok.kind, Tok::Eof)
            || self.tok.newline_before
        {
            return Ok(());
        }
        self.error(format!("expected ; here, not {:?}", self.text()))
    }

    fn state(&mut self) -> &mut FunctionState {
        self.functions
            .last_mut()
            .expect("there's always a function")
    }

    /// The number a name has, which it's given the first time it's asked about.
    fn name_id(&mut self, name: &'a str) -> u32 {
        let next = self.name_texts.len() as u32;
        let id = *self.name_ids.entry(name).or_insert(next);
        if id == next {
            self.name_texts.push(name);
            self.name_keys
                .push(u64::from(name_hash(name)) << 32 | u64::from(id));
            self.noted_in.push(u32::MAX);
            self.merged_into.push(u32::MAX);
            self.declared_in.push(u32::MAX);
            self.listed_in.push(u32::MAX);
        }
        id
    }

    /// Whether the function being read mentions a name, itself or in a function inside it.
    fn mentions(&self, name: &str) -> bool {
        let Some(&id) = self.name_ids.get(name) else {
            return false;
        };
        let state = self.functions.last().expect("there's always a function");
        state.names.contains(&id) || state.inner.contains(&id)
    }

    /// The names that these numbers are, sorted, without any twice, as a list that lasts and that can be searched.
    fn sorted_names(&self, ids: &mut [u32]) -> Names<'a> {
        let mut keys: Vec<u64> = ids.iter().map(|&id| self.name_keys[id as usize]).collect();
        keys.sort_unstable();
        keys.dedup();
        let sorted: Vec<(u32, &'a str)> = keys
            .iter()
            .map(|&key| ((key >> 32) as u32, self.name_texts[key as u32 as usize]))
            .collect();
        Names(self.memory.list(&sorted))
    }

    /// Notes that this function mentions a name.
    fn note(&mut self, name: &'a str) {
        let id = self.name_id(name);
        let with_depth = self.with_depth;
        let state = self
            .functions
            .last_mut()
            .expect("there's always a function");
        if self.noted_in[id as usize] != state.serial {
            self.noted_in[id as usize] = state.serial;
            state.names.push(id);
        }
        if id == ARGUMENTS && !state.is_arrow {
            state.uses_arguments = true;
        }
        // Inside a `with`, any name might come from its object.
        for with_name in &WITH_NAMES[..with_depth] {
            let id = self.name_id(with_name);
            let state = self
                .functions
                .last_mut()
                .expect("there's always a function");
            if self.noted_in[id as usize] != state.serial {
                self.noted_in[id as usize] = state.serial;
                state.names.push(id);
            }
        }
    }

    fn begin_function(&mut self, is_async: bool, is_generator: bool, is_arrow: bool) {
        let parent = self.functions.last();
        let strict = parent.is_some_and(|parent| parent.strict);
        // An arrow is part of what it's in, as far as all this goes.
        let (allows_super_property, allows_super_call, allows_new_target, in_class_init) =
            if is_arrow {
                (
                    parent.is_some_and(|parent| parent.allows_super_property),
                    parent.is_some_and(|parent| parent.allows_super_call),
                    parent.is_some_and(|parent| parent.allows_new_target),
                    parent.is_some_and(|parent| parent.in_class_init),
                )
            } else {
                (false, false, true, false)
            };
        let serial = self.next_serial;
        self.next_serial += 1;
        self.functions.push(FunctionState {
            serial,
            is_async,
            is_generator,
            is_arrow,
            strict,
            allows_super_property,
            allows_super_call,
            allows_new_target,
            in_class_init,
            ..FunctionState::default()
        });
    }

    /// Finishes a function, and says what's needed to know about it for its outside and for itself: the names
    /// functions inside it mention, whether it says `arguments`, and the names it may need from outside it.
    fn end_function(&mut self) -> (Names<'a>, bool, &'a [&'a str]) {
        let mut state = self.functions.pop().expect("a function was begun");
        let parent = self
            .functions
            .last_mut()
            .expect("there's always a function");
        let is_arrow = state.is_arrow;
        let mut outer = std::mem::take(&mut self.outer_scratch);
        for &id in state.names.iter().chain(state.inner.iter()) {
            // These mean something else in a function of its own, unless it's an arrow.
            if !is_arrow && matches!(id, THIS | ARGUMENTS | NEW_TARGET) {
                continue;
            }
            // A name it declares for all of itself is its own wherever it's said in it, never the one outside.
            if self.declared_in[id as usize] == state.serial {
                continue;
            }
            if self.listed_in[id as usize] != state.serial {
                self.listed_in[id as usize] = state.serial;
                outer.push(self.name_texts[id as usize]);
            }
            if self.merged_into[id as usize] != parent.serial {
                self.merged_into[id as usize] = parent.serial;
                parent.inner.push(id);
            }
        }
        let outer_names = self.memory.list(&outer);
        outer.clear();
        self.outer_scratch = outer;
        (
            self.sorted_names(&mut state.inner),
            state.uses_arguments,
            outer_names,
        )
    }

    /// Notes the names the function being read declares for all of itself: its parameters, and if they're all plain
    /// names, its `var`s and what its body declares at the top. A function inside it that says one of them means this
    /// function's, so the one outside of the same name isn't captured by it. (With a default or a pattern, what's in
    /// the parameters sees the outside, not the body's variables.)
    fn note_declared(&mut self, params: &[PatternItem<'a>], rest: Option<&Pattern<'a>>, body: &[Stmt<'a>]) {
        let mut names = Vec::new();
        for param in params {
            collect_binding_names(&param.target, &mut names);
        }
        if let Some(rest) = rest {
            collect_binding_names(rest, &mut names);
        }
        let is_simple = rest.is_none()
            && params
                .iter()
                .all(|param| param.default.is_none() && matches!(param.target, Pattern::Ident(..)));
        if is_simple {
            crate::compiler::collect_vars(body, &mut names);
            for statement in body {
                match statement {
                    Stmt::Var(_, declarations) => {
                        for (pattern, _) in declarations.iter() {
                            collect_binding_names(pattern, &mut names);
                        }
                    }
                    Stmt::Function(function) => names.extend(function.name),
                    Stmt::Class(class) => names.extend(class.name),
                    _ => {}
                }
            }
        }
        let serial = self.state().serial;
        for name in names {
            let id = self.name_id(name);
            self.declared_in[id as usize] = serial;
        }
    }

    fn name(&mut self) -> Parsed<&'a str> {
        if !matches!(self.tok.kind, Tok::Ident) {
            return self.error(format!("expected a name here, not {:?}", self.text()));
        }
        let name = self.text();
        self.advance()?;
        Ok(name)
    }

    fn binding_name(&mut self) -> Parsed<&'a str> {
        if matches!(self.tok.kind, Tok::Ident) && is_reserved(self.text()) {
            return self.error(format!("{:?} can't be a name", self.text()));
        }
        if matches!(self.tok.kind, Tok::Ident) {
            let name = self.text();
            self.check_name(name)?;
            if self.is_strict() && matches!(name, "eval" | "arguments") {
                return self.error(format!("{name:?} can't be declared in strict code"));
            }
        }
        self.name()
    }

    // TypeScript's types, which are only read so as to go past them.

    /// Goes past `>`, even if it's been read as part of a longer token, like the end of `A<B<C>>`.
    fn split_greater(&mut self) -> Parsed<()> {
        let Tok::Punct(punct) = self.tok.kind else {
            return self.error("expected >");
        };
        if !punct.starts_with('>') {
            return self.error("expected >");
        }
        if punct.len() > 1 {
            self.lexer.at = self.tok.start as usize + 1;
            self.tok.kind = Tok::Punct(">");
            self.tok.end = self.tok.start + 1;
        }
        Ok(())
    }

    fn skip_balanced(&mut self, open: &str, close: &str) -> Parsed<()> {
        let mut depth = 0;
        loop {
            if matches!(self.tok.kind, Tok::Eof) {
                return self.error(format!("{open} doesn't end"));
            }
            if let Tok::Template { tail: false, .. } = self.tok.kind {
                self.skip_template_rest()?;
                continue;
            }
            if self.is(open) {
                depth += 1;
            } else if self.is(close) {
                depth -= 1;
                if depth == 0 {
                    return self.advance();
                }
            }
            self.advance()?;
        }
    }

    /// Goes past a template, from its first part.
    fn skip_template_rest(&mut self) -> Parsed<()> {
        loop {
            self.advance()?;
            let mut depth = 0;
            loop {
                if matches!(self.tok.kind, Tok::Eof) {
                    return self.error("a template doesn't end");
                }
                if let Tok::Template { tail: false, .. } = self.tok.kind {
                    self.skip_template_rest()?;
                    continue;
                }
                if self.is("{") {
                    depth += 1;
                } else if self.is("}") {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                self.advance()?;
            }
            let part = self.lexer.read_template_part()?;
            let done = matches!(part, Tok::Template { tail: true, .. });
            self.tok.kind = part;
            if done {
                return self.advance();
            }
        }
    }

    fn skip_type_args(&mut self) -> Parsed<()> {
        self.expect("<")?;
        loop {
            if matches!(self.tok.kind, Tok::Eof) {
                return self.error("expected >");
            }
            if self.is(">") || self.is(">>") || self.is(">>>") || self.is(">=") || self.is(">>=") {
                self.split_greater()?;
                return self.advance();
            }
            if self.eat(",")? {
                continue;
            }
            // Type parameters can have all these around them.
            for word in ["const", "in", "out"] {
                if self.is_word(word) {
                    let saved = self.save();
                    self.advance()?;
                    if !matches!(self.tok.kind, Tok::Ident) {
                        self.restore(saved);
                    }
                    break;
                }
            }
            self.skip_type()?;
            if self.eat_word("extends")? {
                self.skip_type()?;
            }
            if self.eat("=")? {
                self.skip_type()?;
            }
        }
    }

    fn skip_type(&mut self) -> Parsed<()> {
        let _ = self.eat("|")? || self.eat("&")?;
        loop {
            self.skip_type_primary()?;
            loop {
                if self.is("[") && !self.tok.newline_before {
                    self.skip_balanced("[", "]")?;
                } else {
                    break;
                }
            }
            if self.eat("|")? || self.eat("&")? {
                continue;
            }
            break;
        }
        if self.is_word("is") && !self.tok.newline_before {
            self.advance()?;
            self.skip_type()?;
        }
        if self.is_word("extends") && !self.tok.newline_before {
            let saved = self.save();
            self.advance()?;
            self.skip_type()?;
            if self.eat("?")? {
                self.skip_type()?;
                self.expect(":")?;
                self.skip_type()?;
            } else {
                // That was a constraint, which belongs to what's reading the type parameter.
                self.restore(saved);
            }
        }
        Ok(())
    }

    fn skip_type_primary(&mut self) -> Parsed<()> {
        match &self.tok.kind {
            Tok::Punct("{") => return self.skip_balanced("{", "}"),
            Tok::Punct("[") => return self.skip_balanced("[", "]"),
            Tok::Punct("(") => {
                self.skip_balanced("(", ")")?;
                if self.eat("=>")? {
                    self.skip_type()?;
                }
                return Ok(());
            }
            Tok::Punct("<") => {
                self.skip_type_args()?;
                self.skip_balanced("(", ")")?;
                self.expect("=>")?;
                return self.skip_type();
            }
            Tok::Punct("-") => {
                self.advance()?;
                return self.advance();
            }
            Tok::Number(_) | Tok::BigInt(..) | Tok::String => return self.advance(),
            Tok::Template { tail: true, .. } => return self.advance(),
            Tok::Template { .. } => return self.skip_template_rest(),
            Tok::Ident => {}
            _ => return self.error(format!("expected a type here, not {:?}", self.text())),
        }
        let word = self.text();
        self.advance()?;
        match word {
            "keyof" | "typeof" | "readonly" | "infer" | "unique" | "asserts"
                if matches!(
                    self.tok.kind,
                    Tok::Ident | Tok::Punct("{") | Tok::Punct("[") | Tok::Punct("(") | Tok::String
                ) && !self.tok.newline_before =>
            {
                return self.skip_type_primary();
            }
            "new" | "abstract" => {
                if word == "abstract" {
                    self.advance()?;
                }
                if self.is("<") {
                    self.skip_type_args()?;
                }
                self.skip_balanced("(", ")")?;
                self.expect("=>")?;
                return self.skip_type();
            }
            _ => {}
        }
        while self.eat(".")? {
            self.advance()?;
        }
        if self.is("<") {
            self.skip_type_args()?;
        }
        Ok(())
    }

    /// Goes past `: Type`, if there is one.
    fn skip_annotation(&mut self) -> Parsed<()> {
        if self.eat(":")? {
            self.skip_type()?;
        }
        Ok(())
    }

    // Statements.

    fn block(&mut self) -> Parsed<Vec<Stmt<'a>>> {
        self.expect("{")?;
        let mut body = Vec::new();
        while !self.is("}") {
            if matches!(self.tok.kind, Tok::Eof) {
                return self.error("a block doesn't end");
            }
            self.statement_into(&mut body)?;
        }
        self.advance()?;
        Ok(body)
    }

    /// `{ a = 1 }` that never turned into a pattern is a mistake.
    fn check_cover(&mut self) -> Parsed<()> {
        if let Some(line) = self.pending_cover.take() {
            return Err(SyntaxError {
                message: "a property's value can only be left out like that in a pattern"
                    .to_owned(),
                line,
            });
        }
        Ok(())
    }

    /// The body of an `if` or a loop, or what's after a label, where only some things can go.
    fn body_statement(&mut self, is_if: bool, is_label: bool) -> Parsed<Stmt<'a>> {
        if matches!(self.tok.kind, Tok::Ident) && self.tok.escaped_name.is_none() {
            let word = self.text();
            let not_here = match word {
                "class" | "const" => true,
                "function" => {
                    // Old code can have a plain function here, and generators never.
                    let saved = self.save();
                    self.advance()?;
                    let is_generator = self.is("*");
                    self.restore(saved);
                    self.is_strict() || is_generator || !(is_if || is_label)
                }
                "async" => {
                    let saved = self.save();
                    self.advance()?;
                    let found = self.is_word("function") && !self.tok.newline_before;
                    self.restore(saved);
                    found
                }
                "let" => {
                    let saved = self.save();
                    self.advance()?;
                    let found = self.is("[")
                        || (!self.tok.newline_before
                            && (matches!(self.tok.kind, Tok::Ident) || self.is("{")));
                    self.restore(saved);
                    found
                }
                "using" => self.using_follows(false)?,
                "await" => self.await_using_follows()?,
                _ => false,
            };
            if not_here {
                return self.error(format!("a {word} declaration can't go here"));
            }
        }
        self.statement()
    }

    fn statement(&mut self) -> Parsed<Stmt<'a>> {
        let mut out = Vec::with_capacity(1);
        self.statement_into(&mut out)?;
        // A loop on its own keeps no place to stop in front of it, so that a label on it is still a loop's, and nor does
        // a declaration, which would otherwise be in a block of its own.
        if let [Stmt::Line(_), only] = out[..]
            && matches!(
                only,
                Stmt::For(_)
                    | Stmt::ForIn(_)
                    | Stmt::ForOf(_)
                    | Stmt::While(..)
                    | Stmt::DoWhile(..)
                    | Stmt::Labeled(..)
                    | Stmt::Function(_)
                    | Stmt::Class(_)
                    | Stmt::Var(..)
                    | Stmt::Empty
            )
        {
            out.remove(0);
        }
        Ok(match out.len() {
            0 => Stmt::Empty,
            1 => out.pop().unwrap_or(Stmt::Empty),
            _ => Stmt::Block(self.memory.list(&out)),
        })
    }

    /// Whether the next token starts a declaration with a name, for words that are only special before one.
    fn next_is_name(&mut self) -> Parsed<bool> {
        let saved = self.save();
        self.advance()?;
        let found = matches!(self.tok.kind, Tok::Ident) && !self.tok.newline_before;
        self.restore(saved);
        Ok(found)
    }

    /// Whether a `using` declaration starts here: `using`, and then on the same line the name it declares. In the head
    /// of a `for`, `using of` is the variable `using` being given each value, unless what's after it gives `of` one.
    fn using_follows(&mut self, in_for: bool) -> Parsed<bool> {
        if !self.is_word("using") || self.tok.escaped_name.is_some() {
            return Ok(false);
        }
        let saved = self.save();
        self.advance()?;
        let mut found = self.declared_name_follows();
        if found && in_for && self.is_word("of") {
            self.advance()?;
            found = self.is("=");
        }
        self.restore(saved);
        Ok(found)
    }

    /// Whether an `await using` declaration starts here, which it only can where `await` waits.
    fn await_using_follows(&mut self) -> Parsed<bool> {
        if !self.is_word("await")
            || self.tok.escaped_name.is_some()
            || !self.functions.last().is_some_and(|state| state.is_async)
        {
            return Ok(false);
        }
        let saved = self.save();
        self.advance()?;
        let mut found = self.is_word("using")
            && self.tok.escaped_name.is_none()
            && !self.tok.newline_before;
        if found {
            self.advance()?;
            found = self.declared_name_follows();
        }
        self.restore(saved);
        Ok(found)
    }

    /// Whether the token here could be the name a `using` declares, on the same line as what came before it.
    fn declared_name_follows(&self) -> bool {
        matches!(self.tok.kind, Tok::Ident) && !self.tok.newline_before && !is_reserved(self.text())
    }

    /// Reads `using a = b, c = d`, or the same after `await`, where each is a plain name and given a value.
    fn using_statement(&mut self, kind: VarKind) -> Parsed<Stmt<'a>> {
        // A script has nothing to dispose of what its top declares, so it can only be in a block or a function there.
        if self.statement_at_top
            && !self.reading_wrapped
            && !self.allow_top_return
            && !self.functions.last().is_some_and(|state| state.is_async)
        {
            return self.error("a using declaration can't be at the top of a script");
        }
        if kind == VarKind::AwaitUsing {
            self.advance()?;
        }
        self.advance()?;
        let declarations = self.declarations(false)?;
        self.check_using(&declarations, true)?;
        Ok(Stmt::Var(kind, self.memory.list(&declarations)))
    }

    /// What a `using` declares has to be a plain name other than `let`, and, but in a `for of`, be given a value.
    fn check_using(
        &self,
        declarations: &[(Pattern<'a>, Option<Expr<'a>>)],
        needs_value: bool,
    ) -> Parsed<()> {
        for (pattern, value) in declarations {
            match pattern {
                Pattern::Ident("let", _) => return self.error("\"let\" can't be a name here"),
                Pattern::Ident(..) => {}
                _ => return self.error("a using declaration can only declare a name"),
            }
            if needs_value && value.is_none() {
                return self.error("a using declaration needs a value");
            }
        }
        Ok(())
    }

    fn statement_into(&mut self, out: &mut Vec<Stmt<'a>>) -> Parsed<()> {
        self.check_depth()?;
        self.statement_at_top = std::mem::take(&mut self.at_top);
        if let Tok::Punct(punct) = self.tok.kind {
            match punct {
                "{" => {
                    let body = self.block()?;
                    out.push(Stmt::Block(self.memory.list(&body)));
                    return Ok(());
                }
                ";" => return self.advance(),
                _ => {}
            }
        }
        if self.debug_lines && !self.is_word("function") {
            out.push(Stmt::Line(self.tok.line));
        }
        if matches!(self.tok.kind, Tok::Ident) {
            if let Some(()) = self.keyword_statement(out)? {
                return Ok(());
            }
        }
        let expression = self.expression(false)?;
        // A label.
        if let Expr::Ident(name, _) = expression {
            if self.eat(":")? {
                let body = self.body_statement(false, true)?;
                out.push(Stmt::Labeled(name, self.memory.node(body)));
                return Ok(());
            }
        }
        self.check_cover()?;
        self.semicolon()?;
        out.push(Stmt::Expr(expression));
        Ok(())
    }

    fn keyword_statement(&mut self, out: &mut Vec<Stmt<'a>>) -> Parsed<Option<()>> {
        let word = self.text();
        match word {
            "var" | "const" => {
                // `const enum` is an enum.
                if word == "const" {
                    let saved = self.save();
                    self.advance()?;
                    if self.is_word("enum") {
                        self.enum_declaration(out)?;
                        return Ok(Some(()));
                    }
                    self.restore(saved);
                }
                let statement = self.var_statement()?;
                self.semicolon()?;
                out.push(statement);
            }
            "let" => {
                let saved = self.save();
                self.advance()?;
                let is_declaration =
                    matches!(self.tok.kind, Tok::Ident) || self.is("[") || self.is("{");
                self.restore(saved);
                if !is_declaration {
                    return Ok(None);
                }
                let statement = self.var_statement()?;
                self.semicolon()?;
                out.push(statement);
            }
            "using" | "await" => {
                let kind = if self.using_follows(false)? {
                    VarKind::Using
                } else if self.await_using_follows()? {
                    VarKind::AwaitUsing
                } else {
                    return Ok(None);
                };
                let statement = self.using_statement(kind)?;
                self.semicolon()?;
                out.push(statement);
            }
            "function" => {
                let start = self.tok.start;
                if let Some(function) = self.function_declaration(false, start)? {
                    out.push(Stmt::Function(self.memory.node(function)));
                }
            }
            "async" => {
                let start = self.tok.start;
                let saved = self.save();
                self.advance()?;
                if self.is_word("function") && !self.tok.newline_before {
                    if let Some(function) = self.function_declaration(true, start)? {
                        out.push(Stmt::Function(self.memory.node(function)));
                    }
                } else {
                    self.restore(saved);
                    return Ok(None);
                }
            }
            "class" => {
                let class = self.class(true)?;
                out.push(Stmt::Class(self.memory.node(class)));
            }
            "if" => {
                self.advance()?;
                self.expect("(")?;
                let test = self.expression(false)?;
                self.check_cover()?;
                self.expect(")")?;
                let then = self.body_statement(true, false)?;
                let otherwise = if self.eat_word("else")? {
                    Some(self.memory.node(self.body_statement(true, false)?))
                } else {
                    None
                };
                out.push(Stmt::If(
                    self.memory.node(test),
                    self.memory.node(then),
                    otherwise,
                ));
            }
            "for" => {
                let statement = self.for_statement()?;
                out.push(statement);
            }
            "while" => {
                self.advance()?;
                self.expect("(")?;
                let test = self.expression(false)?;
                self.check_cover()?;
                self.expect(")")?;
                let body = self.body_statement(false, false)?;
                out.push(Stmt::While(self.memory.node(test), self.memory.node(body)));
            }
            "do" => {
                self.advance()?;
                let body = self.body_statement(false, false)?;
                if !self.eat_word("while")? {
                    return self.error("expected while here");
                }
                self.expect("(")?;
                let test = self.expression(false)?;
                self.expect(")")?;
                let _ = self.eat(";")?;
                out.push(Stmt::DoWhile(
                    self.memory.node(body),
                    self.memory.node(test),
                ));
            }
            "return" => {
                if self.functions.len() == 1 && !self.allow_top_return {
                    return self.error("return can only be in a function");
                }
                if self.reading_wrapped && self.functions.len() == 2 {
                    self.only_wrapped = true;
                }
                self.advance()?;
                let value = if self.is(";")
                    || self.is("}")
                    || matches!(self.tok.kind, Tok::Eof)
                    || self.tok.newline_before
                {
                    None
                } else {
                    Some(self.expression(false)?)
                };
                self.semicolon()?;
                out.push(Stmt::Return(value.map(|value| self.memory.node(value))));
            }
            "break" | "continue" => {
                self.advance()?;
                let label = if matches!(self.tok.kind, Tok::Ident) && !self.tok.newline_before {
                    Some(self.name()?)
                } else {
                    None
                };
                self.semicolon()?;
                out.push(if word == "break" {
                    Stmt::Break(label)
                } else {
                    Stmt::Continue(label)
                });
            }
            "throw" => {
                self.advance()?;
                if self.tok.newline_before {
                    return self.error("what's thrown has to be on the same line as throw");
                }
                let value = self.expression(false)?;
                self.check_cover()?;
                self.semicolon()?;
                out.push(Stmt::Throw(self.memory.node(value)));
            }
            "try" => {
                let statement = self.try_statement()?;
                out.push(statement);
            }
            "switch" => {
                let statement = self.switch_statement()?;
                out.push(statement);
            }
            "debugger" => {
                self.advance()?;
                self.semicolon()?;
            }
            "with" => {
                if self.is_strict() {
                    return self.error("with can't be in strict code");
                }
                self.advance()?;
                self.expect("(")?;
                let object = self.expression(false)?;
                self.expect(")")?;
                if self.with_depth == WITH_NAMES.len() {
                    return self.error("too many withs inside each other");
                }
                self.with_depth += 1;
                let body = self.body_statement(false, false);
                self.with_depth -= 1;
                out.push(Stmt::With(
                    self.memory.node(object),
                    self.memory.node(body?),
                ));
            }
            "import" => {
                let saved = self.save();
                self.advance()?;
                if self.is("(") || self.is(".") {
                    self.restore(saved);
                    return Ok(None);
                }
                if self.statement_at_top && !self.next_is_equals_after_name()? {
                    self.said_module = true;
                }
                self.import_statement(out)?;
            }
            "export" => {
                if self.statement_at_top {
                    let saved = self.save();
                    self.advance()?;
                    if !self.is("=") {
                        self.said_module = true;
                    }
                    self.restore(saved);
                }
                if !self.statement_at_top {
                    // Only a type can be exported anywhere else, as it's gone.
                    let saved = self.save();
                    self.advance()?;
                    let only_a_type = ["type", "interface", "declare"]
                        .iter()
                        .any(|word| self.is_word(word));
                    self.restore(saved);
                    if !only_a_type {
                        return self.error("an export can only be at the top of a module");
                    }
                }
                self.export_statement(out)?;
            }
            // TypeScript's, which only count where they can't be anything else.
            "interface" if self.next_is_name()? => {
                self.advance()?;
                self.advance()?;
                while !self.is("{") {
                    if matches!(self.tok.kind, Tok::Eof) {
                        return self.error("an interface doesn't end");
                    }
                    self.advance()?;
                }
                self.skip_balanced("{", "}")?;
            }
            "type" if self.next_is_name()? => {
                self.advance()?;
                self.advance()?;
                if self.is("<") {
                    self.skip_type_args()?;
                }
                self.expect("=")?;
                self.skip_type()?;
                self.semicolon()?;
            }
            "declare" if self.next_is_name()? => {
                self.advance()?;
                // `declare global { ... }`, `declare module "x" { ... }` and `declare namespace X { ... }` are only ever types.
                if self.is_word("global") || self.is_word("module") || self.is_word("namespace") {
                    self.advance()?;
                    while !self.is("{")
                        && !self.is(";")
                        && !self.tok.newline_before
                        && !matches!(self.tok.kind, Tok::Eof)
                    {
                        self.advance()?;
                    }
                    if self.is("{") {
                        return self.skip_balanced("{", "}").map(Some);
                    }
                    self.semicolon()?;
                    return Ok(Some(()));
                }
                // Nothing that's declared runs.
                let mut ignored = Vec::new();
                self.statement_into(&mut ignored)?;
            }
            "abstract" if self.next_is_name()? => {
                self.advance()?;
                let class = self.class(true)?;
                out.push(Stmt::Class(self.memory.node(class)));
            }
            "enum" if self.next_is_name()? => self.enum_declaration(out)?,
            "namespace" | "module" if self.next_is_name()? => {
                self.advance()?;
                self.advance()?;
                while self.eat(".")? {
                    self.advance()?;
                }
                if self.is("{") {
                    return self.error("namespaces aren't handled yet");
                }
                self.semicolon()?;
            }
            _ => return Ok(None),
        }
        Ok(Some(()))
    }

    fn var_kind(&mut self) -> Parsed<VarKind> {
        let word = self.text();
        self.advance()?;
        Ok(match word {
            "var" => VarKind::Var,
            "let" => VarKind::Let,
            _ => VarKind::Const,
        })
    }

    fn var_statement(&mut self) -> Parsed<Stmt<'a>> {
        let kind = self.var_kind()?;
        Ok(Stmt::Var(
            kind,
            self.memory.list(&self.declarations(false)?),
        ))
    }

    fn declarations(&mut self, no_in: bool) -> Parsed<Vec<(Pattern<'a>, Option<Expr<'a>>)>> {
        let mut out = Vec::new();
        loop {
            let pattern = self.binding_pattern()?;
            // `let a!: number` says it'll be set later on.
            let _ = self.eat("!")?;
            self.skip_annotation()?;
            let value = if self.eat("=")? {
                let value = self.assignment(no_in)?;
                self.check_cover()?;
                Some(value)
            } else {
                None
            };
            out.push((pattern, value));
            if !self.eat(",")? {
                return Ok(out);
            }
        }
    }

    fn binding_pattern(&mut self) -> Parsed<Pattern<'a>> {
        if self.eat("[")? {
            let mut items = Vec::new();
            let mut rest = None;
            while !self.is("]") {
                if self.eat(",")? {
                    items.push(None);
                    continue;
                }
                if self.eat("...")? {
                    rest = Some(self.memory.node(self.binding_pattern()?));
                    let _ = self.eat(",")?;
                    break;
                }
                items.push(Some(self.binding_item()?));
                if !self.is("]") {
                    self.expect(",")?;
                }
            }
            self.expect("]")?;
            return Ok(Pattern::Array {
                items: self.memory.list(&items),
                rest,
            });
        }
        if self.eat("{")? {
            let mut items = Vec::new();
            let mut rest = None;
            while !self.is("}") {
                if self.eat("...")? {
                    rest = Some(self.memory.node(self.binding_pattern()?));
                    let _ = self.eat(",")?;
                    break;
                }
                let line = self.tok.line;
                let is_plain = matches!(self.tok.kind, Tok::Ident);
                let shorthand = is_plain.then(|| self.text());
                let key = self.property_key()?;
                let item = if self.eat(":")? {
                    self.binding_item()?
                } else {
                    let Some(name) = shorthand else {
                        return self.error("expected : here");
                    };
                    let default = if self.eat("=")? {
                        Some(self.assignment(false)?)
                    } else {
                        None
                    };
                    PatternItem {
                        target: Pattern::Ident(name, line),
                        default,
                    }
                };
                items.push((key, item));
                if !self.is("}") {
                    self.expect(",")?;
                }
            }
            self.expect("}")?;
            return Ok(Pattern::Object {
                items: self.memory.list(&items),
                rest,
            });
        }
        let line = self.tok.line;
        let name = self.binding_name()?;
        Ok(Pattern::Ident(name, line))
    }

    fn binding_item(&mut self) -> Parsed<PatternItem<'a>> {
        let target = self.binding_pattern()?;
        let default = if self.eat("=")? {
            Some(self.assignment(false)?)
        } else {
            None
        };
        Ok(PatternItem { target, default })
    }

    fn for_statement(&mut self) -> Parsed<Stmt<'a>> {
        self.advance()?;
        let is_await = self.eat_word("await")?;
        self.expect("(")?;
        let mut init = None;
        if !self.is(";") {
            let is_let = self.is_word("let") && {
                let saved = self.save();
                self.advance()?;
                let found = matches!(self.tok.kind, Tok::Ident) || self.is("[") || self.is("{");
                self.restore(saved);
                found
            };
            let using = if self.using_follows(true)? {
                Some(VarKind::Using)
            } else if self.await_using_follows()? {
                Some(VarKind::AwaitUsing)
            } else {
                None
            };
            if self.is_word("var") || self.is_word("const") || is_let || using.is_some() {
                let kind = match using {
                    Some(kind) => {
                        if kind == VarKind::AwaitUsing {
                            self.advance()?;
                        }
                        self.advance()?;
                        kind
                    }
                    None => self.var_kind()?,
                };
                let first = self.binding_pattern()?;
                self.skip_annotation()?;
                if self.is_word("of") || self.is_word("in") {
                    if kind.is_using() {
                        if self.is_word("in") {
                            return self.error("a using declaration can't be what a for in gives to");
                        }
                        self.check_using(&[(first, None)], false)?;
                    }
                    return self.for_each(ForTarget::Declare(kind, first), is_await);
                }
                let value = if self.eat("=")? {
                    Some(self.assignment(true)?)
                } else {
                    None
                };
                let mut declarations = vec![(first, value)];
                if self.eat(",")? {
                    declarations.extend(self.declarations(true)?);
                }
                if kind.is_using() {
                    self.check_using(&declarations, true)?;
                }
                init = Some(ForInit::Var(kind, self.memory.list(&declarations)));
            } else {
                let expression = self.expression(true)?;
                if self.is_word("of") || self.is_word("in") {
                    let target = self.to_pattern(expression)?;
                    self.pending_cover = None;
                    return self.for_each(ForTarget::Assign(target), is_await);
                }
                self.check_cover()?;
                init = Some(ForInit::Expr(expression));
            }
        }
        self.expect(";")?;
        let test = if self.is(";") {
            None
        } else {
            Some(self.expression(false)?)
        };
        self.expect(";")?;
        let update = if self.is(")") {
            None
        } else {
            Some(self.expression(false)?)
        };
        self.expect(")")?;
        let body = self.memory.node(self.body_statement(false, false)?);
        Ok(Stmt::For(self.memory.node(ForStmt {
            init,
            test,
            update,
            body,
        })))
    }

    fn for_each(&mut self, target: ForTarget<'a>, is_await: bool) -> Parsed<Stmt<'a>> {
        let is_of = self.is_word("of");
        self.advance()?;
        let source = if is_of {
            self.assignment(false)?
        } else {
            self.expression(false)?
        };
        self.check_cover()?;
        self.expect(")")?;
        let body = self.memory.node(self.body_statement(false, false)?);
        Ok(if is_of {
            Stmt::ForOf(self.memory.node(ForOfStmt {
                target,
                iterable: source,
                body,
                is_await,
            }))
        } else {
            Stmt::ForIn(self.memory.node(ForInStmt {
                target,
                object: source,
                body,
            }))
        })
    }

    fn try_statement(&mut self) -> Parsed<Stmt<'a>> {
        self.advance()?;
        let block = self.block()?;
        let mut param = None;
        let mut handler = None;
        if self.eat_word("catch")? {
            if self.eat("(")? {
                param = Some(self.binding_pattern()?);
                self.skip_annotation()?;
                self.expect(")")?;
            }
            let body = self.block()?;
            handler = Some(self.memory.list(&body));
        }
        let finalizer = if self.eat_word("finally")? {
            let body = self.block()?;
            Some(self.memory.list(&body))
        } else {
            None
        };
        if handler.is_none() && finalizer.is_none() {
            return self.error("a try needs a catch or a finally");
        }
        Ok(Stmt::Try(self.memory.node(TryStmt {
            block: self.memory.list(&block),
            param,
            handler,
            finalizer,
        })))
    }

    fn switch_statement(&mut self) -> Parsed<Stmt<'a>> {
        self.advance()?;
        self.expect("(")?;
        let value = self.expression(false)?;
        self.expect(")")?;
        self.expect("{")?;
        let mut cases = Vec::new();
        while !self.eat("}")? {
            let test = if self.eat_word("default")? {
                None
            } else if self.eat_word("case")? {
                Some(self.expression(false)?)
            } else {
                return self.error("expected case here");
            };
            self.expect(":")?;
            let mut body = Vec::new();
            while !self.is_word("case") && !self.is_word("default") && !self.is("}") {
                if matches!(self.tok.kind, Tok::Eof) {
                    return self.error("a switch doesn't end");
                }
                self.statement_into(&mut body)?;
            }
            // What a case declares belongs to the whole switch, and the language leaves `using` out of them.
            if body
                .iter()
                .any(|statement| matches!(statement, Stmt::Var(kind, _) if kind.is_using()))
            {
                return self.error("a using declaration can't be in a case of a switch");
            }
            cases.push(SwitchCase {
                test,
                body: self.memory.list(&body),
            });
        }
        Ok(Stmt::Switch(
            self.memory.node(value),
            self.memory.list(&cases),
        ))
    }

    /// The name of a module in an `import` or `export`, and what it says about itself after it, as `with { type: "json" }`,
    /// which says nothing a file's name doesn't here.
    fn module_source(&mut self) -> Parsed<&'a [u16]> {
        let source = self.string_token()?;
        if self.is_word("with") || (self.is_word("assert") && !self.tok.newline_before) {
            self.advance()?;
            self.expect("{")?;
            let mut keys: Vec<&'a str> = Vec::new();
            while !self.eat("}")? {
                let key = self.export_name()?;
                if keys.contains(&key) {
                    return self.error(format!("{key} is said twice about one import"));
                }
                keys.push(key);
                self.expect(":")?;
                self.string_token()?;
                if !self.is("}") {
                    self.expect(",")?;
                }
            }
        }
        Ok(source)
    }

    fn string_token(&mut self) -> Parsed<&'a [u16]> {
        let Tok::String = self.tok.kind else {
            return self.error("expected a string here");
        };
        let text = self.string_value();
        self.advance()?;
        Ok(text)
    }

    /// Reads `{ a, b as c, type D }`, leaving out what's only a type.
    /// A name in an `import { }` or `export { }`, which can be a string, as `export { a as "a-b" }`, for what has to
    /// have a name no variable can.
    fn export_name(&mut self) -> Parsed<&'a str> {
        if let Tok::String = self.tok.kind {
            let (start, end) = (self.tok.start as usize, self.tok.end as usize);
            let inside = &self.lexer.source[start + 1..end - 1];
            if inside.contains('\\') {
                return self.error("a name with escapes in it can't be exported here");
            }
            self.advance()?;
            return Ok(inside);
        }
        self.name()
    }

    fn name_list(&mut self) -> Parsed<Vec<(&'a str, &'a str)>> {
        self.expect("{")?;
        let mut list = Vec::new();
        while !self.eat("}")? {
            let mut only_type = false;
            if self.is_word("type") && self.next_is_name()? {
                self.advance()?;
                only_type = true;
            }
            let first = self.export_name()?;
            let second = if self.eat_word("as")? {
                self.export_name()?
            } else {
                first
            };
            if !only_type {
                list.push((first, second));
            }
            if !self.is("}") {
                self.expect(",")?;
            }
        }
        Ok(list)
    }

    fn import_statement(&mut self, out: &mut Vec<Stmt<'a>>) -> Parsed<()> {
        // `import type` is nothing at all, once types are gone.
        if self.is_word("type") {
            let saved = self.save();
            self.advance()?;
            if !self.is_word("from") && !self.is(",") && !self.is("=") {
                while !self.is(";")
                    && !self.tok.newline_before
                    && !matches!(self.tok.kind, Tok::Eof)
                {
                    if self.is("{") {
                        self.skip_balanced("{", "}")?;
                    } else {
                        self.advance()?;
                    }
                }
                return self.semicolon();
            }
            self.restore(saved);
        }
        // What's only a type is gone wherever it is, as when CommonJS is read inside a function.
        if !self.statement_at_top {
            return self.error("an import can only be at the top of a module");
        }
        if let Tok::String = self.tok.kind {
            let source = self.module_source()?;
            self.semicolon()?;
            out.push(Stmt::Import(self.memory.node(ImportStmt {
                source: self.memory.list(source),
                default: None,
                all: None,
                named: &[],
            })));
            return Ok(());
        }
        let mut default = None;
        let mut all = None;
        let mut named = Vec::new();
        if matches!(self.tok.kind, Tok::Ident) {
            default = Some(self.name()?);
            let _ = self.eat(",")?;
        }
        if self.eat("*")? {
            if !self.eat_word("as")? {
                return self.error("expected as here");
            }
            all = Some(self.name()?);
        } else if self.is("{") {
            named = self.name_list()?;
        }
        if !self.eat_word("from")? {
            return self.error("expected from here");
        }
        let source = self.module_source()?;
        self.semicolon()?;
        out.push(Stmt::Import(self.memory.node(ImportStmt {
            source: self.memory.list(source),
            default,
            all,
            named: self.memory.list(&named),
        })));
        Ok(())
    }

    fn collect_pattern_names(pattern: &Pattern<'a>, out: &mut Vec<(&'a str, &'a str)>) {
        match pattern {
            Pattern::Ident(name, _) => out.push((name, name)),
            Pattern::Expr(_) => {}
            Pattern::Array { items, rest } => {
                for item in items.iter().flatten() {
                    Self::collect_pattern_names(&item.target, out);
                }
                if let Some(rest) = rest {
                    Self::collect_pattern_names(rest, out);
                }
            }
            Pattern::Object { items, rest } => {
                for (_, item) in items.iter() {
                    Self::collect_pattern_names(&item.target, out);
                }
                if let Some(rest) = rest {
                    Self::collect_pattern_names(rest, out);
                }
            }
        }
    }

    fn export_statement(&mut self, out: &mut Vec<Stmt<'a>>) -> Parsed<()> {
        self.advance()?;
        if self.eat_word("default")? {
            // `abstract` only says the class can't be made with `new`, which is only checked as a type.
            if self.is_word("abstract") {
                let saved = self.save();
                self.advance()?;
                if !self.is_word("class") || self.tok.newline_before {
                    self.restore(saved);
                }
            }
            if self.is_word("function") || self.is_word("async") || self.is_word("class") {
                let saved = self.save();
                let mut declared = Vec::new();
                let is_named =
                    self.is_word("async") || self.is_word("function") || { self.next_is_name()? };
                if is_named {
                    if let Some(()) = self.keyword_statement(&mut declared).ok().flatten() {
                        let name = match declared.first() {
                            Some(Stmt::Function(function)) => function.name,
                            Some(Stmt::Class(class)) => class.name,
                            _ => None,
                        };
                        if let Some(name) = name {
                            out.extend(declared);
                            out.push(Stmt::Export(ExportName::List(
                                self.memory.list(&[(name, "default")]),
                            )));
                            return Ok(());
                        }
                    }
                }
                self.restore(saved);
            }
            // `export default class {}` and `export default function () {}` are declarations without names, which end
            // where their bodies do, with nothing after them that could make them part of something bigger.
            let mut is_declaration = self.is_word("function") || self.is_word("class");
            if self.is_word("async") {
                let saved = self.save();
                self.advance()?;
                is_declaration = self.is_word("function") && !self.tok.newline_before;
                self.restore(saved);
            }
            let value = if is_declaration {
                let line = self.tok.line;
                self.primary_word(line)?
            } else {
                let value = self.assignment(false)?;
                self.semicolon()?;
                value
            };
            out.push(Stmt::ExportDefault(self.memory.node(value)));
            return Ok(());
        }
        if self.eat("*")? {
            if self.eat_word("as")? {
                let name = self.name()?;
                if !self.eat_word("from")? {
                    return self.error("expected from here");
                }
                let source = self.module_source()?;
                self.semicolon()?;
                // A whole module under one name is an import with the name given out.
                out.push(Stmt::Import(self.memory.node(ImportStmt {
                    source: self.memory.list(source),
                    default: None,
                    all: Some(name),
                    named: &[],
                })));
                out.push(Stmt::Export(ExportName::List(
                    self.memory.list(&[(name, name)]),
                )));
                return Ok(());
            }
            if !self.eat_word("from")? {
                return self.error("expected from here");
            }
            let source = self.module_source()?;
            self.semicolon()?;
            out.push(Stmt::ExportFrom(self.memory.node(ExportFromStmt {
                source: self.memory.list(source),
                named: None,
            })));
            return Ok(());
        }
        if self.is("{") {
            let list = self.name_list()?;
            if self.eat_word("from")? {
                let source = self.module_source()?;
                self.semicolon()?;
                out.push(Stmt::ExportFrom(self.memory.node(ExportFromStmt {
                    source: self.memory.list(source),
                    named: Some(self.memory.list(&list)),
                })));
            } else {
                self.semicolon()?;
                out.push(Stmt::Export(ExportName::List(self.memory.list(&list))));
            }
            return Ok(());
        }
        // `export type { A }` and `export type * from "a"` are gone with the rest of the types.
        if self.is_word("type") {
            let saved = self.save();
            self.advance()?;
            if self.is("{") || self.is("*") {
                if self.is("{") {
                    self.skip_balanced("{", "}")?;
                } else {
                    self.advance()?;
                    if self.eat_word("as")? {
                        self.advance()?;
                    }
                }
                if self.eat_word("from")? {
                    self.advance()?;
                }
                return self.semicolon();
            }
            self.restore(saved);
        }
        // `export type` and `export interface` declarations go the same way as they would here.
        let start = out.len();
        self.statement_into(out)?;
        let mut names = Vec::new();
        for statement in &out[start..] {
            match statement {
                Stmt::Var(kind, _) if kind.is_using() => {
                    return self.error("a using declaration can't be exported");
                }
                Stmt::Var(_, declarations) => {
                    for (pattern, _) in declarations.iter() {
                        Self::collect_pattern_names(pattern, &mut names);
                    }
                }
                Stmt::Function(function) => {
                    if let Some(name) = function.name {
                        names.push((name, name));
                    }
                }
                Stmt::Class(class) => {
                    if let Some(name) = class.name {
                        names.push((name, name));
                    }
                }
                _ => {}
            }
        }
        if !names.is_empty() {
            out.push(Stmt::Export(ExportName::List(self.memory.list(&names))));
        }
        Ok(())
    }

    /// An enum becomes an object with both its names and its numbers in it.
    fn enum_declaration(&mut self, out: &mut Vec<Stmt<'a>>) -> Parsed<()> {
        self.advance()?;
        let line = self.tok.line;
        let name = self.name()?;
        self.expect("{")?;
        let mut properties = Vec::new();
        let mut next = 0f64;
        while !self.eat("}")? {
            let key = match &self.tok.kind {
                Tok::String => {
                    let text: Box<[u16]> = self.string_value().into();
                    self.advance()?;
                    text
                }
                _ => utf16(self.name()?).into_boxed_slice(),
            };
            let mut number = Some(next);
            let value = if self.eat("=")? {
                let value = self.assignment(false)?;
                number = match value {
                    Expr::Number(found) => Some(found),
                    Expr::Unary(UnaryOp::Neg, Expr::Number(found)) => Some(-found),
                    _ => None,
                };
                value
            } else {
                Expr::Number(next)
            };
            let key = self.memory.list(&key);
            properties.push(Property::Value(PropertyKey::String(key), value));
            if let Some(found) = number {
                properties.push(Property::Value(
                    PropertyKey::Number(found),
                    Expr::String(key),
                ));
                next = found + 1.0;
            }
            if !self.is("}") {
                self.expect(",")?;
            }
        }
        out.push(Stmt::Var(
            VarKind::Const,
            self.memory.list(&[(
                Pattern::Ident(name, line),
                Some(Expr::Object(self.memory.list(&properties))),
            )]),
        ));
        Ok(())
    }

    // Functions and classes.

    /// Reads a function, from its `function`. `None` is one with no body, which TypeScript has for overloads.
    fn function_declaration(&mut self, is_async: bool, start: u32) -> Parsed<Option<Function<'a>>> {
        self.function_after_keyword(is_async, true, start)
    }

    /// `start` is where its text starts, at `function` or the `async` before it.
    fn function_after_keyword(
        &mut self,
        is_async: bool,
        declaration: bool,
        start: u32,
    ) -> Parsed<Option<Function<'a>>> {
        let line = self.tok.line;
        self.advance()?;
        let is_generator = self.eat("*")?;
        let name = if matches!(self.tok.kind, Tok::Ident) && !self.is("(") {
            Some(self.binding_name()?)
        } else {
            None
        };
        if declaration && name.is_none() {
            return self.error("a function here needs a name");
        }
        if self.is("<") {
            self.skip_type_args()?;
        }
        self.begin_function(is_async, is_generator, false);
        let mut function = self.function_rest(name, FunctionKind::Normal, line, None, start)?;
        // A declaration's name is a variable outside it, which can be changed like any other.
        if let (true, Some(function)) = (declaration, &mut function) {
            function.uses_own_name = false;
        }
        Ok(function)
    }

    /// Reads a function's parameters and body. It's already been begun, and its text starts at `start`.
    fn function_rest(
        &mut self,
        name: Option<&'a str>,
        kind: FunctionKind,
        line: u32,
        properties: Option<&mut Vec<&'a str>>,
        start: u32,
    ) -> Parsed<Option<Function<'a>>> {
        let (is_async, is_generator) = {
            let state = self.state();
            (state.is_async, state.is_generator)
        };
        let (params, rest) = self.parameters(properties)?;
        self.skip_annotation()?;
        if !self.is("{") {
            // Only a type: an overload, or something abstract or declared.
            self.end_function();
            self.semicolon()?;
            return Ok(None);
        }
        let body = self.function_block(&params, rest.is_some(), name)?;
        let uses_own_name = name.is_some_and(|name| self.mentions(name));
        let uses_this = self.mentions("this");
        self.note_declared(&params, rest.as_ref(), &body);
        let (inner_names, uses_arguments, outer_names) = self.end_function();
        Ok(Some(Function {
            name,
            kind,
            is_async,
            is_generator,
            params: self.memory.list(&params),
            rest,
            body: self.memory.list(&body),
            inner_names,
            outer_names,
            uses_arguments,
            uses_own_name,
            uses_this,
            line,
            source: (start, self.prev_end),
        }))
    }

    /// Reads a function's body, which is where it can turn out to be strict.
    fn function_block(
        &mut self,
        params: &[PatternItem<'a>],
        has_rest: bool,
        name: Option<&'a str>,
    ) -> Parsed<Vec<Stmt<'a>>> {
        let is_simple = !has_rest
            && params
                .iter()
                .all(|param| param.default.is_none() && matches!(param.target, Pattern::Ident(..)));
        let line = self.tok.line;
        self.expect("{")?;
        let was_strict = self.is_strict();
        let says_strict = self.directives()?;
        if says_strict && !is_simple {
            return self.error("a function with fancy parameters can't say it's strict");
        }
        // What's before the body was read before it was known to be strict.
        if !was_strict && says_strict {
            if let Some(name) = name {
                if matches!(name, "eval" | "arguments") {
                    return self.error(format!("{name:?} can't be declared in strict code"));
                }
                self.check_name(name)?;
            }
        }
        if self.is_strict() || !is_simple || self.state().is_arrow {
            let mut names = Vec::new();
            for param in params {
                collect_binding_names(&param.target, &mut names);
            }
            for (index, found) in names.iter().enumerate() {
                if names[..index].contains(found) {
                    return Err(SyntaxError {
                        message: format!("{found:?} is a parameter twice"),
                        line,
                    });
                }
                if self.is_strict() && !was_strict {
                    if matches!(*found, "eval" | "arguments") {
                        return self.error(format!("{found:?} can't be declared in strict code"));
                    }
                    self.check_name(found)?;
                }
            }
        }
        let mut body = Vec::new();
        while !self.is("}") {
            if matches!(self.tok.kind, Tok::Eof) {
                return self.error("a block doesn't end");
            }
            self.statement_into(&mut body)?;
        }
        self.advance()?;
        // The body's own `let`s and `const`s can't have the name of a parameter.
        let mut names = Vec::new();
        for param in params {
            collect_binding_names(&param.target, &mut names);
        }
        for statement in &body {
            let mut declared = Vec::new();
            match statement {
                Stmt::Var(kind, declarations) if *kind != VarKind::Var => {
                    for (pattern, _) in declarations.iter() {
                        collect_binding_names(pattern, &mut declared);
                    }
                }
                Stmt::Class(class) => declared.extend(class.name),
                _ => {}
            }
            if let Some(found) = declared.iter().find(|found| names.contains(found)) {
                return Err(SyntaxError {
                    message: format!("{found:?} has already been declared"),
                    line,
                });
            }
        }
        Ok(body)
    }

    /// Reads `(a, b = 1, ...c)`. What's marked as a property in a constructor goes in `properties`.
    fn parameters(
        &mut self,
        mut properties: Option<&mut Vec<&'a str>>,
    ) -> Parsed<(Vec<PatternItem<'a>>, Option<Pattern<'a>>)> {
        self.expect("(")?;
        let mut params = Vec::new();
        let mut rest = None;
        while !self.is(")") {
            if self.eat("...")? {
                let pattern = self.binding_pattern()?;
                self.skip_annotation()?;
                rest = Some(pattern);
                if !self.is(")") {
                    return self.error("the rest parameter has to come last");
                }
                break;
            }
            // `this: Type` only says what `this` is.
            if self.is_word("this") {
                self.advance()?;
                self.skip_annotation()?;
                let _ = self.eat(",")?;
                continue;
            }
            let mut is_property = false;
            while matches!(
                self.text(),
                "public" | "private" | "protected" | "readonly" | "override"
            ) && matches!(self.tok.kind, Tok::Ident)
                && self.next_is_name_or_pattern()?
            {
                self.advance()?;
                is_property = true;
            }
            let target = self.binding_pattern()?;
            if is_property {
                if let (Some(properties), Pattern::Ident(name, _)) = (properties.as_mut(), &target)
                {
                    properties.push(name);
                }
            }
            let _ = self.eat("?")?;
            self.skip_annotation()?;
            let default = if self.eat("=")? {
                Some(self.assignment(false)?)
            } else {
                None
            };
            params.push(PatternItem { target, default });
            if !self.is(")") {
                self.expect(",")?;
            }
        }
        self.expect(")")?;
        Ok((params, rest))
    }

    fn next_is_name_or_pattern(&mut self) -> Parsed<bool> {
        let saved = self.save();
        self.advance()?;
        let found = matches!(self.tok.kind, Tok::Ident) || self.is("{") || self.is("[");
        self.restore(saved);
        Ok(found)
    }

    fn property_key(&mut self) -> Parsed<PropertyKey<'a>> {
        let key = match &self.tok.kind {
            Tok::Ident | Tok::Private => PropertyKey::Name(self.text()),
            Tok::String => PropertyKey::String(self.string_value()),
            Tok::Number(number) => PropertyKey::Number(*number),
            Tok::BigInt(digits, radix) => PropertyKey::BigInt(self.memory.text(digits), *radix),
            Tok::Punct("[") => {
                self.advance()?;
                let key = self.assignment(false)?;
                if !self.is("]") {
                    return self.error("expected ] here");
                }
                PropertyKey::Computed(key)
            }
            _ => {
                return self.error(format!(
                    "expected a property name here, not {:?}",
                    self.text()
                ));
            }
        };
        self.advance()?;
        Ok(key)
    }

    /// Reads a method, from its parameters. It can be `async` or a generator. Its text starts at `start`, which is
    /// before `get`, `set`, `async` or `*`, and its name, but after `static`.
    fn method(
        &mut self,
        kind: FunctionKind,
        is_async: bool,
        is_generator: bool,
        properties: Option<&mut Vec<&'a str>>,
        start: u32,
    ) -> Parsed<Option<Function<'a>>> {
        let line = self.tok.line;
        if self.is("<") {
            self.skip_type_args()?;
        }
        self.begin_function(is_async, is_generator, false);
        let state = self.state();
        state.allows_super_property = true;
        state.allows_super_call = kind == FunctionKind::DerivedConstructor;
        self.function_rest(None, kind, line, properties, start)
    }

    fn class(&mut self, needs_name: bool) -> Parsed<Class<'a>> {
        let line = self.tok.line;
        let start = self.tok.start;
        if !self.eat_word("class")? {
            return self.error("expected class here");
        }
        // A class is strict, name and all.
        let outer_strict = self.is_strict();
        self.state().strict = true;
        let result = self.class_after_keyword(needs_name, line, start);
        self.state().strict = outer_strict;
        result
    }

    fn class_after_keyword(&mut self, needs_name: bool, line: u32, start: u32) -> Parsed<Class<'a>> {
        let name = if matches!(self.tok.kind, Tok::Ident)
            && !self.is_word("extends")
            && !self.is_word("implements")
        {
            Some(self.binding_name()?)
        } else {
            None
        };
        if needs_name && name.is_none() {
            return self.error("a class here needs a name");
        }
        if self.is("<") {
            self.skip_type_args()?;
        }
        let parent = if self.eat_word("extends")? {
            let parent = self.left_hand_side()?;
            if self.is("<") {
                self.skip_type_args()?;
            }
            Some(parent)
        } else {
            None
        };
        if self.eat_word("implements")? {
            loop {
                self.skip_type()?;
                if !self.eat(",")? {
                    break;
                }
            }
        }
        self.expect("{")?;
        let mut constructor = None;
        let mut members = Vec::new();
        let mut field_statements = Vec::new();
        // What's said in every field's value goes together, as they all run in one function.
        self.begin_function(false, false, false);
        {
            let state = self.state();
            state.allows_super_property = true;
            state.in_class_init = true;
        }
        self.private_scopes.push((Vec::new(), Vec::new()));
        let is_derived = parent.is_some();
        while !self.is("}") {
            if self.eat(";")? {
                continue;
            }
            if matches!(self.tok.kind, Tok::Eof) {
                return self.error("a class doesn't end");
            }
            let mut is_static = false;
            let mut is_async = false;
            let mut is_generator = false;
            let mut accessor = "";
            let mut only_a_type = false;
            let mut is_auto_accessor = false;
            // A method's text starts after `static` and what only TypeScript has, as `public`.
            let mut member_start = self.tok.start;
            // These only count if a name comes after them. Otherwise they're the name.
            loop {
                let word = if matches!(self.tok.kind, Tok::Ident) {
                    self.text()
                } else {
                    ""
                };
                let is_modifier = matches!(
                    word,
                    "static"
                        | "public"
                        | "private"
                        | "protected"
                        | "readonly"
                        | "abstract"
                        | "override"
                        | "declare"
                        | "async"
                        | "get"
                        | "set"
                        | "accessor"
                );
                if !is_modifier {
                    break;
                }
                let saved = self.save();
                self.advance()?;
                if matches!(
                    self.tok.kind,
                    Tok::Punct("(" | "=" | ";" | "}" | ":" | "<" | "?" | "!")
                ) || (self.tok.newline_before && word != "static")
                {
                    self.restore(saved);
                    break;
                }
                match word {
                    "static" => is_static = true,
                    "async" => is_async = true,
                    "get" | "set" => accessor = word,
                    "declare" | "abstract" => only_a_type = true,
                    "accessor" => is_auto_accessor = true,
                    _ => {}
                }
                if !matches!(word, "async" | "get" | "set" | "accessor") {
                    member_start = self.tok.start;
                }
            }
            // A static block.
            if is_static && self.is("{") {
                self.begin_function(false, false, false);
                {
                    let state = self.state();
                    state.allows_super_property = true;
                    state.in_class_init = true;
                }
                let body = self.block()?;
                let (inner_names, _, outer_names) = self.end_function();
                members.push(ClassMember::StaticInit(self.memory.node(Function {
                    name: None,
                    kind: FunctionKind::Method,
                    is_async: false,
                    is_generator: false,
                    params: &[],
                    rest: None,
                    body: self.memory.list(&body),
                    inner_names,
                    outer_names,
                    uses_arguments: false,
                    uses_own_name: false,
                    uses_this: false,
                    line,
                    source: (0, 0),
                })));
                continue;
            }
            if self.eat("*")? {
                is_generator = true;
            }
            // An index signature is only a type.
            if self.is("[") {
                let saved = self.save();
                self.advance()?;
                let mut is_signature = false;
                if matches!(self.tok.kind, Tok::Ident) {
                    self.advance()?;
                    is_signature = self.is(":");
                }
                self.restore(saved);
                if is_signature {
                    self.skip_balanced("[", "]")?;
                    self.skip_annotation()?;
                    self.semicolon()?;
                    continue;
                }
            }
            let key_is_string = matches!(self.tok.kind, Tok::String);
            if matches!(self.tok.kind, Tok::Private) {
                let private = self.text();
                if private == "#constructor" {
                    return self.error("a private name can't be #constructor");
                }
                // A getter and a setter can share a name, if they're both static or neither is.
                let kind = match (accessor, is_static) {
                    ("get", false) => 1,
                    ("set", false) => 2,
                    ("get", true) => 5,
                    ("set", true) => 6,
                    _ => 3,
                };
                let scope = self.private_scopes.last_mut().expect("this is in a class");
                let clash = scope.0.iter().any(|(found, found_kind)| {
                    *found == private
                        && !(found_kind | kind == 3 && *found_kind != kind)
                        && !(found_kind | kind == 7 && *found_kind != kind && found_kind & 3 != 3)
                });
                scope.0.push((private, kind));
                if clash {
                    return self.error(format!("{private} is declared twice"));
                }
            }
            let key_name: Option<String> = match &self.tok.kind {
                Tok::Ident => Some(self.text().to_owned()),
                Tok::String => Some(String::from_utf16_lossy(self.string_value())),
                _ => None,
            };
            let key = self.property_key()?;
            let _ = self.eat("?")? || self.eat("!")?;
            if self.is("(") || self.is("<") {
                let is_constructor = !is_static
                    && (matches!(key, PropertyKey::Name("constructor"))
                        || (key_is_string && key_name.as_deref() == Some("constructor")));
                if is_constructor && (!accessor.is_empty() || is_async || is_generator) {
                    return self.error("a constructor has to be a plain method");
                }
                if is_constructor && constructor.is_some() {
                    return self.error("a class can only have one constructor");
                }
                if is_static && key_name.as_deref() == Some("prototype") {
                    return self.error("a static method can't be called prototype");
                }
                let kind = match accessor {
                    "get" => FunctionKind::Getter,
                    "set" => FunctionKind::Setter,
                    _ if is_constructor && is_derived => FunctionKind::DerivedConstructor,
                    _ if is_constructor => FunctionKind::Constructor,
                    _ => FunctionKind::Method,
                };
                let mut properties = Vec::new();
                let function =
                    self.method(kind, is_async, is_generator, Some(&mut properties), member_start)?;
                let Some(mut function) = function else {
                    continue;
                };
                if is_constructor {
                    self.add_parameter_properties(&mut function, &properties, is_derived);
                    constructor = Some(function);
                    continue;
                }
                let function = self.memory.node(function);
                members.push(match accessor {
                    "get" => ClassMember::Getter {
                        key,
                        function,
                        is_static,
                    },
                    "set" => ClassMember::Setter {
                        key,
                        function,
                        is_static,
                    },
                    _ => ClassMember::Method {
                        key,
                        function,
                        is_static,
                    },
                });
                continue;
            }
            // A field.
            if matches!(key_name.as_deref(), Some("constructor"))
                || (is_static && matches!(key_name.as_deref(), Some("prototype")))
            {
                return self.error("a field can't have that name");
            }
            self.skip_annotation()?;
            if only_a_type {
                self.semicolon()?;
                continue;
            }
            if is_static {
                self.begin_function(false, false, false);
                let state = self.state();
                state.allows_super_property = true;
                state.in_class_init = true;
            }
            let value = if self.eat("=")? {
                let value = self.assignment(false)?;
                self.check_cover()?;
                if self.mentions("arguments") {
                    return self.error("arguments can't be in a field's value");
                }
                Some(value)
            } else {
                None
            };
            self.semicolon()?;
            // `accessor x = 1` is a getter and a setter for `x`, over a private field of its own that holds the value.
            let mut key = key;
            let mut accessor_key = None;
            if is_auto_accessor {
                let scope = self.private_scopes.last_mut().expect("this is in a class");
                let storage = self
                    .memory
                    .text(&format!("#accessor storage {}", scope.0.len()));
                scope.0.push((storage, 3));
                accessor_key = Some((key, storage));
                key = PropertyKey::Name(storage);
            }
            let key = self.memory.node(key);
            let value = value.map(|value| self.memory.node(value));
            let statement = Stmt::Field(key, value);
            if is_static {
                let (inner_names, _, outer_names) = self.end_function();
                members.push(ClassMember::StaticInit(self.memory.node(Function {
                    name: None,
                    kind: FunctionKind::Method,
                    is_async: false,
                    is_generator: false,
                    params: &[],
                    rest: None,
                    body: self.memory.list(&[statement]),
                    inner_names,
                    outer_names,
                    uses_arguments: false,
                    uses_own_name: false,
                    uses_this: false,
                    line,
                    source: (0, 0),
                })));
            } else {
                field_statements.push(statement);
            }
            if let Some((key, storage)) = accessor_key {
                // A key that's worked out is worked out for each of them, which only matters if working it out does
                // something else too.
                let (getter, setter) = self.auto_accessor(storage, line);
                members.push(ClassMember::Getter {
                    key,
                    function: self.memory.node(getter),
                    is_static,
                });
                members.push(ClassMember::Setter {
                    key,
                    function: self.memory.node(setter),
                    is_static,
                });
            }
        }
        let (inner_names, _, outer_names) = self.end_function();
        self.advance()?;
        // Private names used here that it doesn't have might belong to a class around this one.
        let (declared, used) = self
            .private_scopes
            .pop()
            .expect("one was pushed for this class");
        for (private, private_line) in used {
            if !declared.iter().any(|(found, _)| *found == private) {
                match self.private_scopes.last_mut() {
                    Some(outer) => outer.1.push((private, private_line)),
                    None => {
                        return Err(SyntaxError {
                            message: format!("{private} isn't in a class that has it"),
                            line: private_line,
                        });
                    }
                }
            }
        }
        let private_names: Vec<&'a str> = declared.iter().map(|(name, _)| *name).collect();
        let private_names = self.memory.list(&private_names);
        let fields = (!field_statements.is_empty()).then(|| {
            self.memory.node(Function {
                name: None,
                kind: FunctionKind::Fields,
                is_async: false,
                is_generator: false,
                params: &[],
                rest: None,
                body: self.memory.list(&field_statements),
                inner_names,
                outer_names,
                uses_arguments: false,
                uses_own_name: false,
                uses_this: false,
                line,
                source: (0, 0),
            })
        });
        let constructor = constructor.unwrap_or_else(|| self.default_constructor(is_derived, line));
        Ok(Class {
            name,
            parent,
            constructor: self.memory.node(Function {
                name,
                source: (start, self.prev_end),
                ..constructor
            }),
            members: self.memory.list(&members),
            fields,
            private_names,
        })
    }

    /// `constructor(private x)` also means `this.x = x`, once there's a `this`.
    /// The getter and the setter that an `accessor` field is, which read and write the private field that holds it.
    fn auto_accessor(&mut self, storage: &'a str, line: u32) -> (Function<'a>, Function<'a>) {
        let field = Expr::Member {
            object: self.memory.node(Expr::This),
            name: storage,
            optional: false,
        };
        let mut made = [None, None];
        for (index, kind) in [FunctionKind::Getter, FunctionKind::Setter]
            .into_iter()
            .enumerate()
        {
            self.begin_function(false, false, false);
            self.note("this");
            let (params, body) = if kind == FunctionKind::Getter {
                (Vec::new(), vec![Stmt::Return(Some(self.memory.node(field)))])
            } else {
                self.note("value");
                let params = vec![PatternItem {
                    target: Pattern::Ident("value", line),
                    default: None,
                }];
                let body = vec![Stmt::Expr(Expr::Assign {
                    op: AssignOp::Plain,
                    target: self.memory.node(Pattern::Expr(field)),
                    value: self.memory.node(Expr::Ident("value", line)),
                })];
                (params, body)
            };
            self.note_declared(&params, None, &body);
            let (inner_names, uses_arguments, outer_names) = self.end_function();
            made[index] = Some(Function {
                name: None,
                kind,
                is_async: false,
                is_generator: false,
                params: self.memory.list(&params),
                rest: None,
                body: self.memory.list(&body),
                inner_names,
                outer_names,
                uses_arguments,
                uses_own_name: false,
                uses_this: true,
                line,
                source: (0, 0),
            });
        }
        let [Some(getter), Some(setter)] = made else {
            unreachable!("both were made");
        };
        (getter, setter)
    }

    fn add_parameter_properties(
        &self,
        function: &mut Function<'a>,
        properties: &[&'a str],
        derived: bool,
    ) {
        if properties.is_empty() {
            return;
        }
        let at = if derived {
            function
                .body
                .iter()
                .position(|statement| matches!(statement, Stmt::Expr(Expr::SuperCall(_))))
                .map_or(0, |at| at + 1)
        } else {
            0
        };
        let mut body = function.body.to_vec();
        for (index, name) in properties.iter().enumerate() {
            body.insert(
                at + index,
                Stmt::Expr(Expr::Assign {
                    op: AssignOp::Plain,
                    target: self.memory.node(Pattern::Expr(Expr::Member {
                        object: self.memory.node(Expr::This),
                        name,
                        optional: false,
                    })),
                    value: self.memory.node(Expr::Ident(name, function.line)),
                }),
            );
        }
        function.body = self.memory.list(&body);
    }

    /// A class's constructor when it doesn't say one. A derived class's hands what it's given to its parent's, which
    /// the compiler does without going through them one at a time, as the language says (see `Compiler::super_call`):
    /// its parameter's name, `%args`, is one code can't say.
    fn default_constructor(&self, derived: bool, line: u32) -> Function<'a> {
        let (kind, rest, body) = if derived {
            (
                FunctionKind::DerivedConstructor,
                Some(Pattern::Ident(DEFAULT_ARGS, line)),
                self.memory.list(&[Stmt::Expr(Expr::SuperCall(
                    self.memory
                        .list(&[Element::Spread(Expr::Ident(DEFAULT_ARGS, line))]),
                ))]),
            )
        } else {
            (FunctionKind::Constructor, None, &[][..])
        };
        Function {
            name: None,
            kind,
            is_async: false,
            is_generator: false,
            params: &[],
            rest,
            body,
            inner_names: Names::default(),
            outer_names: &[],
            uses_arguments: false,
            uses_own_name: false,
            uses_this: false,
            line,
            source: (0, 0),
        }
    }

    // Expressions.

    fn expression(&mut self, no_in: bool) -> Parsed<Expr<'a>> {
        let first = self.assignment(no_in)?;
        if !self.is(",") {
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat(",")? {
            items.push(self.assignment(no_in)?);
        }
        Ok(Expr::Sequence(self.memory.list(&items)))
    }

    /// Whether the `(` here starts an arrow function's parameters.
    fn parens_are_arrow(&mut self) -> bool {
        let saved = self.save();
        let at_then = self.tok.start == self.then_at;
        let found = (|| -> Parsed<bool> {
            // Most brackets can be told not to be parameters from what they start with, without going to their end,
            // which for `(function () { ... })` is the whole function. Parameters start with a name, a pattern, `...`,
            // a decorator, or there are none; and a name on its own is followed by one of these.
            self.advance()?;
            if matches!(self.tok.kind, Tok::Ident) {
                if is_reserved(self.text()) {
                    return Ok(false);
                }
                self.advance()?;
                if !(self.is(",") || self.is(")") || self.is("=") || self.is(":") || self.is("?")) {
                    return Ok(false);
                }
            } else if !(self.is("[")
                || self.is("{")
                || self.is("...")
                || self.is(")")
                || self.is("@"))
            {
                return Ok(false);
            }
            self.restore(saved.clone());
            self.skip_balanced("(", ")")?;
            if self.is("=>") {
                return Ok(!self.tok.newline_before);
            }
            if self.is(":") {
                self.advance()?;
                self.skip_type()?;
                if !self.is("=>") {
                    return Ok(false);
                }
                // Right after a `?`, this could also be what's chosen, and then the other choice: `a ? (b) : c => d`.
                // It's only a return type if there's still a `:` to come, for the `?`.
                if at_then {
                    self.advance()?;
                    self.begin_function(false, false, true);
                    let body = if self.is("{") {
                        self.skip_balanced("{", "}")
                    } else {
                        self.assignment(false).map(|_| ())
                    };
                    self.end_function();
                    return Ok(body.is_ok() && self.is(":"));
                }
                return Ok(true);
            }
            Ok(false)
        })();
        self.restore(saved);
        found.unwrap_or(false)
    }

    /// `start` is where its text starts: its parameters, or the `async` or type parameters before them.
    fn arrow_function(&mut self, is_async: bool, start: u32) -> Parsed<Expr<'a>> {
        let line = self.tok.line;
        let was_then = self.tok.start == self.then_at;
        self.begin_function(is_async, false, true);
        let (params, rest) = if self.is("(") {
            self.parameters(None)?
        } else {
            let line = self.tok.line;
            let name = self.binding_name()?;
            (
                vec![PatternItem {
                    target: Pattern::Ident(name, line),
                    default: None,
                }],
                None,
            )
        };
        self.skip_annotation()?;
        if !self.is("=>") || self.tok.newline_before {
            return self.error("expected => here");
        }
        self.advance()?;
        let body = if self.is("{") {
            self.function_block(&params, rest.is_some(), None)?
        } else {
            let mut names = Vec::new();
            for param in &params {
                collect_binding_names(&param.target, &mut names);
            }
            for (index, found) in names.iter().enumerate() {
                if names[..index].contains(found) {
                    return self.error(format!("{found:?} is a parameter twice"));
                }
            }
            // A `:` after the body is still for the `?` this is the first choice of, if it's that.
            if was_then {
                self.then_at = self.tok.start;
            }
            vec![Stmt::Return(Some(
                self.memory.node(self.assignment(false)?),
            ))]
        };
        self.note_declared(&params, rest.as_ref(), &body);
        let (inner_names, uses_arguments, outer_names) = self.end_function();
        Ok(Expr::Function(self.memory.node(Function {
            name: None,
            kind: FunctionKind::Arrow,
            is_async,
            is_generator: false,
            params: self.memory.list(&params),
            rest,
            body: self.memory.list(&body),
            inner_names,
            outer_names,
            uses_arguments,
            uses_own_name: false,
            uses_this: false,
            line,
            source: (start, self.prev_end),
        })))
    }

    /// Whether a name is followed by `=>`, which is asked of nearly every name in an expression.
    fn next_is_arrow_after_name(&mut self) -> Parsed<bool> {
        // The byte right after the name usually says, without reading a token.
        match self.lexer.source.as_bytes().get(self.lexer.at) {
            None => return Ok(false),
            Some(b'=') => {
                return Ok(self.lexer.source.as_bytes().get(self.lexer.at + 1) == Some(&b'>'));
            }
            Some(b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C | b'/') => {}
            Some(&byte) if byte >= 0x80 => {}
            Some(_) => return Ok(false),
        }
        let saved = self.save();
        self.advance()?;
        let found = self.is("=>") && !self.tok.newline_before;
        self.restore(saved);
        Ok(found)
    }

    /// Code nested deeper than this can't be read, which is better than running out of stack.
    fn check_depth(&self) -> Parsed<()> {
        if self.stack_start.abs_diff(crate::stack_position()) > self.stack_room {
            return self.error("this code is nested too deeply");
        }
        Ok(())
    }

    fn note_link(&mut self) -> Parsed<()> {
        self.chain_links += 1;
        if self.chain_links > MAX_CHAIN_LINKS {
            return self.error("this expression is too long");
        }
        Ok(())
    }

    fn assignment(&mut self, no_in: bool) -> Parsed<Expr<'a>> {
        self.check_depth()?;
        let saved_links = self.chain_links;
        self.chain_links = 0;
        let value = self.assignment_inner(no_in);
        self.chain_links = saved_links;
        value
    }

    fn assignment_inner(&mut self, no_in: bool) -> Parsed<Expr<'a>> {
        let start = self.tok.start;
        // Arrow functions.
        if self.is("(") && self.parens_are_arrow() {
            return self.arrow_function(false, start);
        }
        if matches!(self.tok.kind, Tok::Ident) {
            if self.is_word("async") {
                let saved = self.save();
                self.advance()?;
                if !self.tok.newline_before {
                    if self.is("(") && self.parens_are_arrow() {
                        return self.arrow_function(true, start);
                    }
                    if self.is("<") {
                        let generics = self.save();
                        if self.skip_type_args().is_ok() && self.is("(") && self.parens_are_arrow()
                        {
                            return self.arrow_function(true, start);
                        }
                        self.restore(generics);
                    }
                    if matches!(self.tok.kind, Tok::Ident)
                        && !self.is_word("function")
                        && self.next_is_arrow_after_name()?
                    {
                        return self.arrow_function(true, start);
                    }
                }
                self.restore(saved);
            }
            if self.next_is_arrow_after_name()? && !is_reserved(self.text()) {
                return self.arrow_function(false, start);
            }
            if self.is_word("yield") && self.state().is_generator {
                return self.yield_expression(no_in);
            }
        }
        // A generic arrow function: `<T>(x: T) => x`, or `<T,>` where it could be JSX.
        if self.is("<") {
            let saved = self.save();
            if self.skip_type_args().is_ok() && self.is("(") && self.parens_are_arrow() {
                return self.arrow_function(false, start);
            }
            self.restore(saved);
        }
        let left = self.conditional(no_in)?;
        let Tok::Punct(punct) = self.tok.kind else {
            return Ok(left);
        };
        let op = match punct {
            "=" => AssignOp::Plain,
            "+=" => AssignOp::Binary(BinaryOp::Add),
            "-=" => AssignOp::Binary(BinaryOp::Sub),
            "*=" => AssignOp::Binary(BinaryOp::Mul),
            "/=" => AssignOp::Binary(BinaryOp::Div),
            "%=" => AssignOp::Binary(BinaryOp::Mod),
            "**=" => AssignOp::Binary(BinaryOp::Pow),
            "<<=" => AssignOp::Binary(BinaryOp::Shl),
            ">>=" => AssignOp::Binary(BinaryOp::Shr),
            ">>>=" => AssignOp::Binary(BinaryOp::UShr),
            "&=" => AssignOp::Binary(BinaryOp::BitAnd),
            "|=" => AssignOp::Binary(BinaryOp::BitOr),
            "^=" => AssignOp::Binary(BinaryOp::BitXor),
            "&&=" => AssignOp::Logical(LogicalOp::And),
            "||=" => AssignOp::Logical(LogicalOp::Or),
            "??=" => AssignOp::Logical(LogicalOp::Nullish),
            _ => return Ok(left),
        };
        self.advance()?;
        let target = self.to_pattern(left)?;
        if op != AssignOp::Plain && !matches!(target, Pattern::Ident(..) | Pattern::Expr(_)) {
            return self.error("can't do that to a pattern");
        }
        if op == AssignOp::Plain {
            self.pending_cover = None;
        }
        let value = self.assignment(no_in)?;
        Ok(Expr::Assign {
            op,
            target: self.memory.node(target),
            value: self.memory.node(value),
        })
    }

    fn yield_expression(&mut self, no_in: bool) -> Parsed<Expr<'a>> {
        self.advance()?;
        if self.tok.newline_before
            || matches!(
                self.tok.kind,
                Tok::Eof | Tok::Punct(")" | "]" | "}" | "," | ";" | ":")
            )
        {
            return Ok(Expr::Yield {
                value: None,
                delegate: false,
            });
        }
        let delegate = self.eat("*")?;
        let value = self.assignment(no_in)?;
        Ok(Expr::Yield {
            value: Some(self.memory.node(value)),
            delegate,
        })
    }

    /// Takes what was read as a value to be something to assign to.
    fn to_pattern(&mut self, expression: Expr<'a>) -> Parsed<Pattern<'a>> {
        Ok(match expression {
            Expr::Ident(name, line) => {
                if self.is_strict() && matches!(name, "eval" | "arguments") {
                    return self.error(format!("{name:?} can't be assigned to in strict code"));
                }
                Pattern::Ident(name, line)
            }
            Expr::Member {
                optional: false, ..
            }
            | Expr::Index {
                optional: false, ..
            } => Pattern::Expr(expression),
            Expr::SuperMember(_) | Expr::SuperIndex(_) => Pattern::Expr(expression),
            Expr::Array(elements) => {
                let mut items = Vec::new();
                let mut rest = None;
                for element in elements {
                    match element {
                        None => items.push(None),
                        Some(Element::Plain(value)) => items.push(Some(self.to_item(*value)?)),
                        Some(Element::Spread(value)) => {
                            rest = Some(self.memory.node(self.to_pattern(*value)?));
                        }
                    }
                }
                Pattern::Array {
                    items: self.memory.list(&items),
                    rest,
                }
            }
            Expr::Object(properties) => {
                let mut items = Vec::new();
                let mut rest = None;
                for property in properties {
                    match property {
                        Property::Value(key, value) => items.push((*key, self.to_item(*value)?)),
                        Property::Short(name, line) => items.push((
                            PropertyKey::Name(name),
                            PatternItem {
                                target: Pattern::Ident(name, *line),
                                default: None,
                            },
                        )),
                        Property::Spread(value) => {
                            rest = Some(self.memory.node(self.to_pattern(*value)?));
                        }
                        _ => return self.error("that can't be assigned to"),
                    }
                }
                Pattern::Object {
                    items: self.memory.list(&items),
                    rest,
                }
            }
            _ => return self.error("that can't be assigned to"),
        })
    }

    fn to_item(&mut self, expression: Expr<'a>) -> Parsed<PatternItem<'a>> {
        Ok(match expression {
            Expr::Assign {
                op: AssignOp::Plain,
                target,
                value,
            } => PatternItem {
                target: *target,
                default: Some(*value),
            },
            other => PatternItem {
                target: self.to_pattern(other)?,
                default: None,
            },
        })
    }

    fn conditional(&mut self, no_in: bool) -> Parsed<Expr<'a>> {
        let test = self.binary(0, no_in)?;
        if !self.eat("?")? {
            return Ok(test);
        }
        let outer = std::mem::replace(&mut self.then_at, self.tok.start);
        let then = self.assignment(false);
        self.then_at = outer;
        let then = then?;
        self.expect(":")?;
        let otherwise = self.assignment(no_in)?;
        Ok(Expr::Conditional(
            self.memory.node(test),
            self.memory.node(then),
            self.memory.node(otherwise),
        ))
    }

    /// How tightly the operator here binds, and what it is.
    fn binary_operator(&self, no_in: bool) -> Option<(u8, Binary)> {
        Some(match &self.tok.kind {
            Tok::Punct(punct) => match *punct {
                "??" => (1, Binary::Logical(LogicalOp::Nullish)),
                "||" => (2, Binary::Logical(LogicalOp::Or)),
                "&&" => (3, Binary::Logical(LogicalOp::And)),
                "|" => (4, Binary::Plain(BinaryOp::BitOr)),
                "^" => (5, Binary::Plain(BinaryOp::BitXor)),
                "&" => (6, Binary::Plain(BinaryOp::BitAnd)),
                "==" => (7, Binary::Plain(BinaryOp::Eq)),
                "!=" => (7, Binary::Plain(BinaryOp::Ne)),
                "===" => (7, Binary::Plain(BinaryOp::StrictEq)),
                "!==" => (7, Binary::Plain(BinaryOp::StrictNe)),
                "<" => (8, Binary::Plain(BinaryOp::Lt)),
                "<=" => (8, Binary::Plain(BinaryOp::Le)),
                ">" => (8, Binary::Plain(BinaryOp::Gt)),
                ">=" => (8, Binary::Plain(BinaryOp::Ge)),
                "<<" => (9, Binary::Plain(BinaryOp::Shl)),
                ">>" => (9, Binary::Plain(BinaryOp::Shr)),
                ">>>" => (9, Binary::Plain(BinaryOp::UShr)),
                "+" => (10, Binary::Plain(BinaryOp::Add)),
                "-" => (10, Binary::Plain(BinaryOp::Sub)),
                "*" => (11, Binary::Plain(BinaryOp::Mul)),
                "/" => (11, Binary::Plain(BinaryOp::Div)),
                "%" => (11, Binary::Plain(BinaryOp::Mod)),
                "**" => (12, Binary::Plain(BinaryOp::Pow)),
                _ => return None,
            },
            Tok::Ident => match self.text() {
                "instanceof" => (8, Binary::Plain(BinaryOp::InstanceOf)),
                "in" if !no_in => (8, Binary::Plain(BinaryOp::In)),
                "as" | "satisfies" if !self.tok.newline_before => (8, Binary::TypeOnly),
                _ => return None,
            },
            _ => return None,
        })
    }

    fn binary(&mut self, least: u8, no_in: bool) -> Parsed<Expr<'a>> {
        let mut left = self.unary()?;
        while let Some((precedence, operator)) = self.binary_operator(no_in) {
            if precedence < least.max(1) {
                break;
            }
            self.note_link()?;
            self.advance()?;
            match operator {
                Binary::TypeOnly => {
                    // `x as const` has nothing to go past but that.
                    self.skip_type()?;
                }
                Binary::Plain(op) => {
                    // `**` goes from the right, and the rest from the left.
                    let next = if op == BinaryOp::Pow {
                        precedence
                    } else {
                        precedence + 1
                    };
                    let right = self.binary(next, no_in)?;
                    left = Expr::Binary(op, self.memory.node(left), self.memory.node(right));
                }
                Binary::Logical(op) => {
                    let right = self.binary(precedence + 1, no_in)?;
                    left = Expr::Logical(op, self.memory.node(left), self.memory.node(right));
                }
            }
        }
        Ok(left)
    }

    fn unary(&mut self) -> Parsed<Expr<'a>> {
        if let Tok::Punct(punct) = self.tok.kind {
            let op = match punct {
                "-" => Some(UnaryOp::Neg),
                "+" => Some(UnaryOp::Plus),
                "!" => Some(UnaryOp::Not),
                "~" => Some(UnaryOp::BitNot),
                _ => None,
            };
            if let Some(op) = op {
                self.advance()?;
                let value = self.unary()?;
                if self.is("**") {
                    return self.error("a unary operator can't come right before **");
                }
                return Ok(Expr::Unary(op, self.memory.node(value)));
            }
            if punct == "++" || punct == "--" {
                self.advance()?;
                let target = self.unary()?;
                self.check_update_target(&target)?;
                return Ok(Expr::Update {
                    increment: punct == "++",
                    prefix: true,
                    target: self.memory.node(target),
                });
            }
        }
        if matches!(self.tok.kind, Tok::Ident) {
            let op = match self.text() {
                "typeof" => Some(UnaryOp::TypeOf),
                "void" => Some(UnaryOp::Void),
                "delete" => Some(UnaryOp::Delete),
                _ => None,
            };
            if let Some(op) = op {
                self.advance()?;
                let value = self.unary()?;
                if op == UnaryOp::Delete {
                    self.check_delete(&value)?;
                }
                if self.is("**") {
                    return self.error("a unary operator can't come right before **");
                }
                return Ok(Expr::Unary(op, self.memory.node(value)));
            }
            if self.is_word("await") && self.state().is_async {
                self.advance()?;
                return Ok(Expr::Await(self.memory.node(self.unary()?)));
            }
        }
        let value = self.left_hand_side()?;
        if !self.tok.newline_before && (self.is("++") || self.is("--")) {
            self.check_update_target(&value)?;
            let increment = self.is("++");
            self.advance()?;
            return Ok(Expr::Update {
                increment,
                prefix: false,
                target: self.memory.node(value),
            });
        }
        Ok(value)
    }

    fn check_update_target(&self, target: &Expr<'a>) -> Parsed<()> {
        match target {
            Expr::Ident(name, _) => {
                if self.is_strict() && matches!(*name, "eval" | "arguments") {
                    return self.error(format!("{name:?} can't be changed in strict code"));
                }
                Ok(())
            }
            Expr::Member {
                optional: false, ..
            }
            | Expr::Index {
                optional: false, ..
            }
            | Expr::SuperMember(_)
            | Expr::SuperIndex(_) => Ok(()),
            _ => self.error("that can't be changed"),
        }
    }

    fn check_delete(&self, value: &Expr<'a>) -> Parsed<()> {
        match value {
            Expr::Ident(..) if self.is_strict() => {
                self.error("a name can't be deleted in strict code")
            }
            Expr::Member { name, .. } if name.starts_with('#') => {
                self.error("a private name can't be deleted")
            }
            Expr::Chain(inner) => self.check_delete(inner),
            _ => Ok(()),
        }
    }

    /// Notes a private name being used, which has to be one that a class around it has.
    fn use_private(&mut self, name: &'a str) -> Parsed<()> {
        let line = self.tok.line;
        match self.private_scopes.last_mut() {
            Some(scope) => {
                scope.1.push((name, line));
                Ok(())
            }
            None => self.error(format!("{name} isn't in a class that has it")),
        }
    }

    fn arguments(&mut self) -> Parsed<Vec<Element<'a>>> {
        self.expect("(")?;
        let mut args = Vec::new();
        while !self.is(")") {
            if self.eat("...")? {
                args.push(Element::Spread(self.assignment(false)?));
            } else {
                args.push(Element::Plain(self.assignment(false)?));
            }
            if !self.is(")") {
                self.expect(",")?;
            }
        }
        self.advance()?;
        Ok(args)
    }

    /// Tries to go past `<T>` before a call. If that isn't what it is, nothing's been moved past.
    fn type_args_before_call(&mut self) -> bool {
        let saved = self.save();
        let is_call = self.skip_type_args().is_ok()
            && (self.is("(") || matches!(self.tok.kind, Tok::Template { .. }));
        if !is_call {
            self.restore(saved);
        }
        is_call
    }

    fn left_hand_side(&mut self) -> Parsed<Expr<'a>> {
        let mut value = if self.is_word("new") {
            self.new_expression()?
        } else if self.is_word("super") {
            self.super_expression()?
        } else {
            self.primary()?
        };
        let mut in_chain = false;
        loop {
            let line = self.tok.line;
            if self.is("<") && self.type_args_before_call() {
                continue;
            }
            self.note_link()?;
            match &self.tok.kind {
                Tok::Punct(".") => {
                    self.advance()?;
                    let name = self.member_name()?;
                    value = Expr::Member {
                        object: self.memory.node(value),
                        name,
                        optional: false,
                    };
                }
                Tok::Punct("?.") => {
                    self.advance()?;
                    in_chain = true;
                    if self.is("(") {
                        let args = self.arguments()?;
                        value = Expr::Call {
                            callee: self.memory.node(value),
                            args: self.memory.list(&args),
                            optional: true,
                            line,
                        };
                    } else if self.eat("[")? {
                        let index = self.expression(false)?;
                        self.expect("]")?;
                        value = Expr::Index {
                            object: self.memory.node(value),
                            index: self.memory.node(index),
                            optional: true,
                        };
                    } else {
                        let name = self.member_name()?;
                        value = Expr::Member {
                            object: self.memory.node(value),
                            name,
                            optional: true,
                        };
                    }
                }
                Tok::Punct("[") => {
                    self.advance()?;
                    let index = self.expression(false)?;
                    self.expect("]")?;
                    value = Expr::Index {
                        object: self.memory.node(value),
                        index: self.memory.node(index),
                        optional: false,
                    };
                }
                Tok::Punct("(") => {
                    let args = self.arguments()?;
                    value = Expr::Call {
                        callee: self.memory.node(value),
                        args: self.memory.list(&args),
                        optional: false,
                        line,
                    };
                }
                Tok::Template { .. } => {
                    value = self.tagged_template(value)?;
                }
                // `value!` only says it isn't null.
                Tok::Punct("!") if !self.tok.newline_before => self.advance()?,
                _ => break,
            }
        }
        Ok(if in_chain {
            Expr::Chain(self.memory.node(value))
        } else {
            value
        })
    }

    fn member_name(&mut self) -> Parsed<&'a str> {
        if !matches!(self.tok.kind, Tok::Ident | Tok::Private) {
            return self.error(format!("expected a name here, not {:?}", self.text()));
        }
        let name = self.text();
        if matches!(self.tok.kind, Tok::Private) {
            self.use_private(name)?;
        }
        self.advance()?;
        Ok(name)
    }

    fn new_expression(&mut self) -> Parsed<Expr<'a>> {
        let line = self.tok.line;
        self.advance()?;
        if self.eat(".")? {
            let name = self.name()?;
            if name != "target" {
                return self.error("expected new.target");
            }
            if !self.state().allows_new_target {
                return self.error("new.target can only be in a function");
            }
            // It may only be allowed because of the function the file is being read as.
            if self.reading_wrapped {
                self.only_wrapped = true;
            }
            self.note("new.target");
            return Ok(Expr::NewTarget);
        }
        let mut callee = if self.is_word("new") {
            self.new_expression()?
        } else {
            self.primary()?
        };
        loop {
            if self.eat(".")? {
                let name = self.member_name()?;
                callee = Expr::Member {
                    object: self.memory.node(callee),
                    name,
                    optional: false,
                };
            } else if self.eat("[")? {
                let index = self.expression(false)?;
                self.expect("]")?;
                callee = Expr::Index {
                    object: self.memory.node(callee),
                    index: self.memory.node(index),
                    optional: false,
                };
            } else if self.is("<") && self.type_args_before_call() {
            } else {
                break;
            }
        }
        let args = if self.is("(") {
            self.arguments()?
        } else {
            Vec::new()
        };
        Ok(Expr::New {
            callee: self.memory.node(callee),
            args: self.memory.list(&args),
            line,
        })
    }

    fn super_expression(&mut self) -> Parsed<Expr<'a>> {
        self.advance()?;
        self.note("this");
        if self.is("(") {
            if !self.state().allows_super_call {
                return self.error("super() can only be in a derived class's constructor");
            }
            return Ok(Expr::SuperCall(self.memory.list(&self.arguments()?)));
        }
        if !self.state().allows_super_property {
            return self.error("super can only be in a method");
        }
        if self.eat(".")? {
            return Ok(Expr::SuperMember(self.member_name()?));
        }
        self.expect("[")?;
        let index = self.expression(false)?;
        self.expect("]")?;
        Ok(Expr::SuperIndex(self.memory.node(index)))
    }

    /// Reads a template's parts and what goes between them, starting from its first part.
    #[allow(clippy::type_complexity)]
    fn template_parts(&mut self) -> Parsed<(Vec<&'a [u16]>, Vec<&'a [u16]>, Vec<Expr<'a>>)> {
        let mut strings = Vec::new();
        let mut raws = Vec::new();
        let mut values = Vec::new();
        loop {
            let Tok::Template { cooked, raw, tail } = &self.tok.kind else {
                return self.error("expected the rest of a template here");
            };
            let tail = *tail;
            strings.push(self.memory.list(cooked));
            raws.push(self.memory.list(raw));
            if tail {
                self.advance()?;
                return Ok((strings, raws, values));
            }
            self.advance()?;
            values.push(self.expression(false)?);
            if !self.is("}") {
                return self.error("expected } in a template here");
            }
            // Right after the `}`, it's text again.
            self.tok.kind = self.lexer.read_template_part()?;
        }
    }

    fn tagged_template(&mut self, tag: Expr<'a>) -> Parsed<Expr<'a>> {
        let (strings, raw, values) = self.template_parts()?;
        Ok(Expr::TaggedTemplate(self.memory.node(TaggedParts {
            tag: self.memory.node(tag),
            strings: self.memory.list(&strings),
            raw: self.memory.list(&raw),
            values: self.memory.list(&values),
        })))
    }

    fn primary(&mut self) -> Parsed<Expr<'a>> {
        let line = self.tok.line;
        match &self.tok.kind {
            Tok::Number(number) => {
                let number = *number;
                if self.tok.legacy_octal && self.is_strict() {
                    return self.error("old-style octal numbers can't be in strict code");
                }
                self.advance()?;
                Ok(Expr::Number(number))
            }
            Tok::BigInt(digits, radix) => {
                let (digits, radix) = (self.memory.text(digits), *radix);
                self.advance()?;
                Ok(Expr::BigInt(digits, radix))
            }
            Tok::String => {
                let text = self.string_value();
                if self.tok.legacy_octal && self.is_strict() {
                    return self.error("old-style octal escapes can't be in strict code");
                }
                self.advance()?;
                Ok(Expr::String(text))
            }
            Tok::Template { .. } => {
                let (mut strings, _, values) = self.template_parts()?;
                if values.is_empty() {
                    return Ok(Expr::String(
                        self.memory.list(strings.pop().unwrap_or_default()),
                    ));
                }
                Ok(Expr::Template(self.memory.node(TemplateParts {
                    strings: self.memory.list(&strings),
                    values: self.memory.list(&values),
                })))
            }
            Tok::Private => {
                // `#x in object`
                let name = self.text();
                self.use_private(name)?;
                self.advance()?;
                if !self.is_word("in") {
                    return self.error("a private name can only be followed by in here");
                }
                Ok(Expr::PrivateName(name))
            }
            Tok::Ident => self.primary_word(line),
            Tok::Punct(punct) => match *punct {
                "(" => {
                    self.advance()?;
                    let value = self.expression(false)?;
                    self.expect(")")?;
                    Ok(value)
                }
                "[" => self.array_literal(),
                "{" => self.object_literal(),
                "/" | "/=" => {
                    let start = self.tok.start as usize;
                    let (pattern, flags) = self.lexer.read_regex(start)?;
                    let flag_text = String::from_utf16_lossy(&flags);
                    for (index, flag) in flag_text.char_indices() {
                        if !"dgimsuvy".contains(flag) || flag_text[..index].contains(flag) {
                            return self.error("a regular expression has flags that can't be");
                        }
                    }
                    // A pattern that can't work is found now, not when this code gets run.
                    if let Err(message) = crate::regex::compile(&pattern, &flag_text) {
                        return self.error(format!("Invalid regular expression: {message}"));
                    }
                    self.advance()?;
                    Ok(Expr::Regex(self.memory.node(RegexParts {
                        pattern: self.memory.list(&pattern),
                        flags: self.memory.list(&flags),
                    })))
                }
                "<" => {
                    let value = self.jsx_element()?;
                    self.advance()?;
                    Ok(value)
                }
                _ => self.error(format!("didn't expect {punct} here")),
            },
            Tok::Eof => self.error("the code ends too soon"),
        }
    }

    fn primary_word(&mut self, line: u32) -> Parsed<Expr<'a>> {
        match self.text() {
            "import" => {
                self.advance()?;
                if self.eat(".")? {
                    let name = self.name()?;
                    if name != "meta" {
                        return self.error("expected import.meta");
                    }
                    return Ok(Expr::ImportMeta);
                }
                self.expect("(")?;
                let specifier = self.assignment(false)?;
                // A second argument says how to load it, which doesn't matter here.
                if self.eat(",")? && !self.is(")") {
                    self.assignment(false)?;
                    let _ = self.eat(",")?;
                }
                self.expect(")")?;
                Ok(Expr::ImportCall(self.memory.node(specifier)))
            }
            "this" => {
                self.advance()?;
                self.note("this");
                Ok(Expr::This)
            }
            "null" => {
                self.advance()?;
                Ok(Expr::Null)
            }
            "true" => {
                self.advance()?;
                Ok(Expr::True)
            }
            "false" => {
                self.advance()?;
                Ok(Expr::False)
            }
            "function" => {
                let function = self.function_after_keyword(false, false, self.tok.start)?;
                let Some(function) = function else {
                    return self.error("a function here needs a body");
                };
                Ok(Expr::Function(self.memory.node(function)))
            }
            "class" => Ok(Expr::Class(self.memory.node(self.class(false)?))),
            "async" => {
                let start = self.tok.start;
                let saved = self.save();
                self.advance()?;
                if self.is_word("function") && !self.tok.newline_before {
                    let function = self.function_after_keyword(true, false, start)?;
                    let Some(function) = function else {
                        return self.error("a function here needs a body");
                    };
                    return Ok(Expr::Function(self.memory.node(function)));
                }
                self.restore(saved);
                let name = self.name()?;
                self.note(name);
                Ok(Expr::Ident(name, line))
            }
            word if is_reserved(word) => self.error(format!("didn't expect {word:?} here")),
            word => {
                self.check_name(word)?;
                let name = self.name()?;
                self.note(name);
                Ok(Expr::Ident(name, line))
            }
        }
    }

    fn array_literal(&mut self) -> Parsed<Expr<'a>> {
        self.advance()?;
        let mut elements = Vec::new();
        while !self.is("]") {
            if self.eat(",")? {
                elements.push(None);
                continue;
            }
            if self.eat("...")? {
                elements.push(Some(Element::Spread(self.assignment(false)?)));
            } else {
                elements.push(Some(Element::Plain(self.assignment(false)?)));
            }
            if !self.is("]") {
                self.expect(",")?;
            }
        }
        self.advance()?;
        Ok(Expr::Array(self.memory.list(&elements)))
    }

    fn object_literal(&mut self) -> Parsed<Expr<'a>> {
        self.advance()?;
        let mut properties = Vec::new();
        while !self.is("}") {
            properties.push(self.object_property()?);
            if !self.is("}") {
                self.expect(",")?;
            }
        }
        self.advance()?;
        Ok(Expr::Object(self.memory.list(&properties)))
    }

    fn object_property(&mut self) -> Parsed<Property<'a>> {
        if self.eat("...")? {
            return Ok(Property::Spread(self.assignment(false)?));
        }
        let mut is_async = false;
        let mut accessor = "";
        let start = self.tok.start;
        if self.is_word("async") || self.is_word("get") || self.is_word("set") {
            let word = self.text();
            let saved = self.save();
            self.advance()?;
            if matches!(self.tok.kind, Tok::Punct("," | ":" | "(" | "}" | "=" | "<")) {
                self.restore(saved);
            } else if word == "async" {
                is_async = true;
            } else {
                accessor = word;
            }
        }
        let is_generator = self.eat("*")?;
        let line = self.tok.line;
        let is_plain_name = matches!(self.tok.kind, Tok::Ident);
        let short = is_plain_name.then(|| self.text());
        if matches!(self.tok.kind, Tok::Private) {
            return self.error("a private name can only be in a class");
        }
        let key = self.property_key()?;
        if self.is("(") || self.is("<") {
            let kind = match accessor {
                "get" => FunctionKind::Getter,
                "set" => FunctionKind::Setter,
                _ => FunctionKind::Method,
            };
            let function = self.method(kind, is_async, is_generator, None, start)?;
            let Some(function) = function else {
                return self.error("a method here needs a body");
            };
            let function = self.memory.node(function);
            return Ok(match accessor {
                "get" => Property::Getter(key, function),
                "set" => Property::Setter(key, function),
                _ => Property::Method(key, function),
            });
        }
        if self.eat(":")? {
            return Ok(Property::Value(key, self.assignment(false)?));
        }
        let Some(name) = short else {
            return self.error("expected : here");
        };
        self.note(name);
        if is_reserved(name) {
            return self.error(format!("{name:?} can't be a shorthand property"));
        }
        self.check_name(name)?;
        // `{ a = 1 }` only means something when it turns out to be a pattern.
        if self.is("=") {
            self.pending_cover.get_or_insert(self.tok.line);
        }
        if self.eat("=")? {
            let value = self.assignment(false)?;
            return Ok(Property::Value(
                key,
                Expr::Assign {
                    op: AssignOp::Plain,
                    target: self.memory.node(Pattern::Ident(name, line)),
                    value: self.memory.node(value),
                },
            ));
        }
        Ok(Property::Short(name, line))
    }

    // JSX.

    /// The name of a tag, as a value: a string for one that's built in, and otherwise a variable.
    fn jsx_tag(&mut self) -> Parsed<Expr<'a>> {
        if !matches!(self.tok.kind, Tok::Ident) {
            return self.error("expected a tag's name here");
        }
        let line = self.tok.line;
        let name = self.lexer.read_jsx_name(self.tok.start as usize);
        self.advance()?;
        let is_built_in = !self.is(".")
            && (name.chars().next().is_some_and(char::is_lowercase) || name.contains('-'));
        if is_built_in {
            let mut seen = self.jsx_seen.get();
            if PAGE_ONLY_TAGS.binary_search(&name).is_ok() {
                seen.for_page = true;
            } else if matches!(name, "span" | "button" | "input" | "dialog") {
                seen.shared_tag = true;
            }
            self.jsx_seen.set(seen);
        }
        let mut value = if is_built_in {
            Expr::String(self.memory.list(&utf16(name)))
        } else {
            self.note(name);
            Expr::Ident(name, line)
        };
        while self.eat(".")? {
            let part = self.name()?;
            value = Expr::Member {
                object: self.memory.node(value),
                name: part,
                optional: false,
            };
        }
        Ok(value)
    }

    /// Reads an element. It stops on the closing `>` without going past it, as what's next depends on
    /// whether this is inside another.
    fn jsx_element(&mut self) -> Parsed<Expr<'a>> {
        let line = self.tok.line;
        self.note(self.jsx_name());
        let mark = self.begin_jsx_mark();
        self.advance()?;
        if self.is(">") {
            self.note("__Fragment");
            let mut kids = Vec::new();
            self.mark_jsx_part(mark, |own, at| own.open_end = at + 1);
            self.jsx_children(&mut kids)?;
            self.finish_jsx_mark(mark);
            let tag = Expr::Ident("__Fragment", line);
            return Ok(self.jsx_call(tag, Expr::Null, &kids, false, line));
        }
        if mark.is_some() && matches!(self.tok.kind, Tok::Ident) {
            let start = self.tok.start;
            let length = self.lexer.read_jsx_name(start as usize).len() as u32;
            self.mark_jsx_part(mark, |own, _| own.name = (start, start + length));
        }
        let tag = self.jsx_tag()?;
        if self.is("<") {
            let _ = self.skip_type_args();
        }
        let mut properties = Vec::new();
        let mut may_have_key = false;
        loop {
            if self.is("/") {
                self.advance()?;
                self.split_greater()?;
                self.mark_jsx_part(mark, |own, at| {
                    own.open_end = at + 1;
                    own.end = at + 1;
                });
                let props = self.jsx_props(properties);
                return Ok(self.jsx_call(tag, props, &[], may_have_key, line));
            }
            if self.is(">") || self.is(">=") || self.is(">>") {
                self.split_greater()?;
                self.mark_jsx_part(mark, |own, at| own.open_end = at + 1);
                break;
            }
            if self.eat("{")? {
                self.expect("...")?;
                may_have_key = true;
                properties.push(Property::Spread(self.assignment(false)?));
                if !self.is("}") {
                    return self.error("expected } here");
                }
                self.advance()?;
                continue;
            }
            if !matches!(self.tok.kind, Tok::Ident) {
                return self.error(format!("expected an attribute here, not {:?}", self.text()));
            }
            let name_start = self.tok.start;
            let name = self.lexer.read_jsx_name(self.tok.start as usize);
            may_have_key |= name == "key";
            if name == "className" || name == "htmlFor" {
                let mut seen = self.jsx_seen.get();
                seen.for_page = true;
                self.jsx_seen.set(seen);
            }
            self.advance()?;
            let mut value_at = None;
            let value = if self.is("=") {
                self.advance()?;
                let from = self.tok.start;
                let (value, to) = if self.eat("{")? {
                    let value = self.assignment(false)?;
                    if !self.is("}") {
                        return self.error("expected } here");
                    }
                    let to = self.tok.end;
                    self.advance()?;
                    (value, to)
                } else if self.is("<") {
                    let value = self.jsx_element()?;
                    let to = self.tok.end;
                    self.advance()?;
                    (value, to)
                } else {
                    let to = self.tok.end;
                    (Expr::String(self.memory.list(self.string_token()?)), to)
                };
                value_at = Some((from, to));
                value
            } else {
                Expr::True
            };
            self.mark_jsx_part(mark, |own, _| {
                own.attributes.push(JsxAttributeMark {
                    name: (name_start, name_start + name.len() as u32),
                    value: value_at,
                });
            });
            properties.push(Property::Value(
                PropertyKey::String(self.memory.list(&utf16(name))),
                value,
            ));
        }
        let props = self.jsx_props(properties);
        let mut kids = Vec::new();
        self.jsx_children(&mut kids)?;
        self.finish_jsx_mark(mark);
        Ok(self.jsx_call(tag, props, &kids, may_have_key, line))
    }

    /// Makes a note that an element starts here, if notes are being kept.
    fn begin_jsx_mark(&mut self) -> Option<usize> {
        let start = self.tok.start;
        let marks = self.marks.as_mut()?;
        marks.elements.push(JsxMark {
            start,
            ..JsxMark::default()
        });
        Some(marks.elements.len() - 1)
    }

    fn mark_jsx_part(&mut self, mark: Option<usize>, write: impl FnOnce(&mut JsxMark, u32)) {
        let at = self.tok.start;
        if let (Some(index), Some(marks)) = (mark, &mut self.marks) {
            write(&mut marks.elements[index], at);
        }
    }

    /// An element has just been read, as far as the last `>` of its closing tag.
    fn finish_jsx_mark(&mut self, mark: Option<usize>) {
        let at = self.tok.start;
        if let (Some(index), Some(marks)) = (mark, &mut self.marks) {
            let closing = marks.closing;
            let own = &mut marks.elements[index];
            own.close_name = Some(closing);
            own.end = at + 1;
        }
    }

    fn jsx_props(&self, properties: Vec<Property<'a>>) -> Expr<'a> {
        if properties.is_empty() {
            Expr::Null
        } else {
            Expr::Object(self.memory.list(&properties))
        }
    }

    /// `__jsx(type, props, children, mayHaveKey)`, leaving out what isn't needed from the end. Children as
    /// one array and knowing about keys both matter, as this runs for everything drawn: otherwise each
    /// call would have to gather its arguments and look for a property that's usually not there.
    fn jsx_call(
        &self,
        tag: Expr<'a>,
        props: Expr<'a>,
        kids: &[Element<'a>],
        may_have_key: bool,
        line: u32,
    ) -> Expr<'a> {
        let mut args = vec![Element::Plain(tag), Element::Plain(props)];
        if !kids.is_empty() {
            let kids: Vec<Option<Element<'a>>> = kids.iter().cloned().map(Some).collect();
            args.push(Element::Plain(Expr::Array(self.memory.list(&kids))));
        } else if may_have_key {
            args.push(Element::Plain(Expr::Null));
        }
        if may_have_key {
            args.push(Element::Plain(Expr::True));
        }
        Expr::Call {
            callee: self.memory.node(Expr::Ident(self.jsx_name(), line)),
            args: self.memory.list(&args),
            optional: false,
            line,
        }
    }

    /// What JSX calls in this file.
    fn jsx_name(&self) -> &'static str {
        if self.page_jsx { "__jsxPage" } else { "__jsx" }
    }

    /// Reads what's inside an element, and its closing tag, stopping on the final `>`.
    fn jsx_children(&mut self, args: &mut Vec<Element<'a>>) -> Parsed<()> {
        loop {
            let text = self.lexer.read_jsx_text();
            if let Some(cleaned) = clean_jsx_text(text) {
                args.push(Element::Plain(Expr::String(self.memory.list(&cleaned))));
            }
            self.advance()?;
            if self.is("{") {
                self.advance()?;
                if self.is("}") {
                    continue;
                }
                if self.eat("...")? {
                    args.push(Element::Spread(self.assignment(false)?));
                } else {
                    args.push(Element::Plain(self.expression(false)?));
                }
                if !self.is("}") {
                    return self.error("expected } here");
                }
                continue;
            }
            if !self.is("<") {
                return self.error("an element doesn't end");
            }
            let saved = self.save();
            self.advance()?;
            if self.is("/") {
                self.advance()?;
                // The tag's name has been checked by no one, as a type checker will.
                let (start, mut end) = (self.tok.start, self.tok.start);
                while !self.is(">") && !self.is(">=") && !self.is(">>") {
                    if matches!(self.tok.kind, Tok::Eof) {
                        return self.error("an element doesn't end");
                    }
                    end = self.tok.end;
                    self.advance()?;
                }
                if let Some(marks) = &mut self.marks {
                    marks.closing = (start, end);
                }
                return self.split_greater();
            }
            self.restore(saved);
            args.push(Element::Plain(self.jsx_element()?));
        }
    }
}

/// Every name a pattern declares.
fn collect_binding_names<'a>(pattern: &Pattern<'a>, out: &mut Vec<&'a str>) {
    match pattern {
        Pattern::Ident(name, _) => out.push(name),
        Pattern::Expr(_) => {}
        Pattern::Array { items, rest } => {
            for item in items.iter().flatten() {
                collect_binding_names(&item.target, out);
            }
            if let Some(rest) = rest {
                collect_binding_names(rest, out);
            }
        }
        Pattern::Object { items, rest } => {
            for (_, item) in items.iter() {
                collect_binding_names(&item.target, out);
            }
            if let Some(rest) = rest {
                collect_binding_names(rest, out);
            }
        }
    }
}

enum Binary {
    Plain(BinaryOp),
    Logical(LogicalOp),
    TypeOnly,
}

/// Text in JSX has its line breaks and the space around them taken out, and lines joined with a space.
fn clean_jsx_text(text: &str) -> Option<Vec<u16>> {
    let mut out = String::new();
    let lines: Vec<&str> = text.split('\n').collect();
    let last = lines.len() - 1;
    for (index, line) in lines.iter().enumerate() {
        let line = line.trim_end_matches('\r');
        let mut part = line;
        if index > 0 {
            part = part.trim_start();
        }
        if index < last {
            part = part.trim_end();
        }
        if part.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(part);
    }
    if out.is_empty() {
        return None;
    }
    let out = out
        .replace("&nbsp;", "\u{A0}")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    Some(utf16(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Parsed<Program<'_>> {
        // Only addresses, until something's parsed. A test can look at the tree for as long as it likes.
        let memory = Box::leak(Box::new(AstMemory::new()));
        super::parse(source, memory, 256 * 1024, false)
    }

    fn parses(source: &str) {
        if let Err(error) = parse(source) {
            panic!("{source:?} line {}: {}", error.line, error.message);
        }
    }

    fn fails(source: &str) {
        assert!(parse(source).is_err(), "{source:?} shouldn't parse");
    }

    #[test]
    fn plain_javascript_parses() {
        parses("let a = 1, b = [1, 2, ...c], { d, e: [f = 2] } = g;");
        parses("function f(a, b = 2, ...c) { return a + b * c ** 2 }");
        parses(
            "class A extends B { static x = 1; #y = 2; get z() { return this.#y } constructor() { super() } }",
        );
        parses("for (let i = 0; i < 3; i++) continue; for (const a of b); for (x in y);");
        parses("a?.b?.[c]?.(d); a ?? b; a ||= b; x = y ? z : w; `t${1}u${`v${2}`}`;");
        parses("label: while (1) { break label } do { } while (0)");
        parses("try { } catch { } finally { } switch (a) { case 1: b; default: c }");
        parses("async function f() { await g(); } function* h() { yield 1; yield* i(); }");
        parses("let r = /a[/]b/g.test(x); a / b / c;");
        parses("x = (a, b) => a + b; y = async c => c; z = async (d) => { await d };");
        parses(
            "import a, { b as c } from 'x'; import * as d from 'y'; export const e = 1; export { a as f }; export default a;",
        );
    }

    #[test]
    fn typescript_loses_its_types() {
        parses("let a: number = 1; const b: Array<Map<string, number>> = [];");
        parses("function f<T extends object>(a: T, b?: string): a is T { return true }");
        parses(
            "interface I { a: string } type T = { b: number } | string; declare const d: number;",
        );
        parses(
            "class A<T> extends B<T> implements C, D { private x: number = 1; readonly y!: string; abstract z(): void; constructor(public a: number, private b = 2) { super() } [k: string]: any; }",
        );
        parses("const c = a as unknown as string; let d = e! + f!.g; const h = i satisfies J;");
        parses("enum E { A, B = 5, C } const enum F { X = 'x' }");
        parses("const f = <T,>(a: T): T => a; g<number>(1); new H<string>();");
        parses(
            "import type { A } from 'a'; import { type B, c } from 'b'; export type { A }; export interface C {}",
        );
        parses(
            "function over(a: string): void; function over(a: number): void; function over(a: any) {}",
        );
        parses("type Cond<T> = T extends string ? 'a' : T extends number ? `b${T}` : never;");
        parses("let x: (a: number) => void = () => {}; let y: typeof x; let z: keyof T[];");
    }

    fn marked(source: &str, range: (u32, u32)) -> &str {
        &source[range.0 as usize..range.1 as usize]
    }

    #[test]
    fn where_elements_and_their_attributes_are_is_noted_exactly() {
        let source = r#"const a = <div className="x" onClick={() => go(1)} hidden><b>hi</b>{y && <i />}</div>;"#;
        let marks = jsx_marks(source, 16 * 1024 * 1024).unwrap();
        let names: Vec<&str> = marks.iter().map(|mark| marked(source, mark.name)).collect();
        assert_eq!(names, ["div", "b", "i"]);
        let div = &marks[0];
        assert_eq!(marked(source, (div.start, div.start + 4)), "<div");
        assert_eq!(marked(source, div.attributes[0].name), "className");
        assert_eq!(marked(source, div.attributes[0].value.unwrap()), r#""x""#);
        assert_eq!(
            marked(source, div.attributes[1].value.unwrap()),
            "{() => go(1)}"
        );
        assert_eq!(div.attributes[2].value, None);
        assert_eq!(source[..div.open_end as usize].chars().last(), Some('>'));
        assert_eq!(marked(source, div.close_name.unwrap()), "div");
        assert_eq!(
            marked(source, (div.start, div.end)).chars().last(),
            Some('>')
        );
        assert!(marked(source, (div.start, div.end)).ends_with("</div>"));
        assert_eq!(marked(source, (marks[2].start, marks[2].end)), "<i />");
        assert_eq!(marks[2].close_name, None);
    }

    #[test]
    fn what_is_not_an_element_in_the_end_is_not_noted() {
        // `<T>` here starts out looking like one.
        let source =
            "const a = <T,>(x: T) => x; const b = <>{1}<u></u></>; const c = 1 < 2 > (3 as any);";
        let marks = jsx_marks(source, 16 * 1024 * 1024).unwrap();
        let names: Vec<&str> = marks.iter().map(|mark| marked(source, mark.name)).collect();
        assert_eq!(names, ["", "u"]);
        assert!(marked(source, (marks[0].start, marks[0].end)).ends_with("</>"));
    }

    #[test]
    fn jsx_parses() {
        parses("const a = <div />;");
        parses("const a = <div id=\"x\" data-y={1} z {...p}>text {b} <span>c</span></div>;");
        parses("const a = <><A.B x={<C />} /></>;");
        parses("const a = b ? <c>d</c> : <e></e>;");
        parses("const a = <div>{/* nothing */}{[1].map(x => <i>{x}</i>)}</div>;");
    }

    #[test]
    fn jsx_text_is_tidied_up() {
        let cleaned = |text: &str| clean_jsx_text(text).map(|text| String::from_utf16_lossy(&text));
        assert_eq!(cleaned("  a b  "), Some("  a b  ".to_owned()));
        assert_eq!(cleaned("\n   a\n   b\n"), Some("a b".to_owned()));
        assert_eq!(cleaned("\n   \n"), None);
        assert_eq!(cleaned("&lt;a&amp;b&gt;"), Some("<a&b>".to_owned()));
    }

    #[test]
    fn broken_code_says_so() {
        fails("let = 1 2");
        fails("function ( {");
        fails("a +");
        fails("`a${b");
        fails("class { }");
        fails("'a");
        fails("/* a");
        fails("try { }");
        fails("<div>");
        fails("return 1");
        fails("1__0");
        fails("1_");
        fails("0_1");
    }

    #[test]
    fn strict_code_is_held_to_more() {
        fails("'use strict'; with (a) {}");
        fails("'use strict'; 010");
        fails("'use strict'; '\\01'");
        fails("'use strict'; delete a");
        fails("'use strict'; eval = 1");
        fails("'use strict'; let interface");
        fails("function f(a, a) { 'use strict' }");
        fails("function eval() { 'use strict' }");
        fails("function f(a = 1) { 'use strict' }");
        fails("class C { m() { with (a) {} } }");
        parses("with (a) {} 010; delete a; eval = 1; function f(a, a) {}");
    }

    #[test]
    fn mistakes_are_found_before_anything_runs() {
        fails("({ a = 1 })");
        parses("({ a = 1 } = {}); [{ a = 1 }] = []; for ({ a = 1 } of b); ({ a = 1 }) => a;");
        fails("function* g() { var yield }");
        fails("async function f() { var await }");
        fails("(a, a) => 1");
        fails("function f(a) { let a }");
        fails("function f(...a, b) {}");
        fails("function f(...a,) {}");
        fails("if (1) class C {}");
        fails("while (1) function f() {}");
        fails("if (1) function* g() {}");
        parses("if (1) function f() {} a: function g() {}");
        fails("-1 ** 2");
        parses("(-1) ** 2; 2 ** -1;");
        fails("1++");
        fails("/(/");
        fails("/a/gg");
        fails("new.target");
        fails("super.x");
        fails("function f() { super() }");
        fails("class A { constructor() { super() } }");
        parses("class A extends B { constructor() { super(); () => super.x } m() { super.m() } }");
        fails("class A { m() { super() } }");
        fails("class A { #a; #a }");
        fails("class A { m() { this.#b } }");
        fails("this.#a");
        fails("class A { #a; m() { delete this.#a } }");
        fails("class A { constructor() {} constructor() {} }");
        fails("class A { get constructor() {} }");
        fails("class A { static prototype() {} }");
        fails("class A { x = arguments }");
        fails("class A { #constructor }");
        parses(
            "class A { #a; get #b() { return 1 } set #b(v) {} static #c; m() { return #a in this && class { n() { return this.#a } } } }",
        );
        parses("class A { #a; m() { class B { n(o) { return o.#a } } } }");
    }

    #[test]
    fn names_can_have_escapes() {
        parses(
            "var \\u0061b = 1; \\u{62}; a\\u0062; x.\\u0061; class C { #\\u0061; m() { this.#a } }",
        );
        fails("var \\u0030a");
        fails("var a\\u0020b");
        fails("var \\x61");
    }

    #[test]
    fn what_inner_functions_mention_is_noted() {
        let program =
            parse("let a; function f() { return a + this.x } let g = () => this.y;").unwrap();
        let inner = &program.function.inner_names;
        assert!(inner.contains("a"));
        // An arrow's `this` is its parent's, and a function's own is its own.
        assert!(inner.contains("this"));
        let program = parse("function f() { this.x }").unwrap();
        assert!(!program.function.inner_names.contains("this"));
    }
}
