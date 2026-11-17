//! Turns source code into tokens, one at a time, as the parser asks. It can't always tell by itself what
//! something is: `/` can start a regular expression, and text in JSX isn't code. The parser says which it wants.

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    Eof,
    Ident,
    /// A private name, like `#count`.
    Private,
    Number(f64),
    /// A BigInt's digits, and the base they're in.
    BigInt(Box<str>, u32),
    /// A string, which is read from the source when it's wanted. See [`string_units`].
    String,
    /// A part of a template, with whether it's the last.
    Template {
        cooked: Box<[u16]>,
        raw: Box<[u16]>,
        tail: bool,
    },
    Punct(&'static str),
}

#[derive(Debug, Clone)]
pub(crate) struct Token {
    pub(crate) kind: Tok,
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) line: u32,
    /// Whether a line ends between the last token and this.
    pub(crate) newline_before: bool,
    /// A name with an escape in it, as it reads once the escapes are worked out.
    pub(crate) escaped_name: Option<&'static str>,
    /// Whether this has an old-style octal in it, which strict code can't.
    pub(crate) legacy_octal: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct SyntaxError {
    pub(crate) message: String,
    pub(crate) line: u32,
}

pub(crate) type Parsed<T> = Result<T, SyntaxError>;

/// A regular expression's pattern and its flags.
pub(crate) type RegexParts = (Box<[u16]>, Box<[u16]>);

#[derive(Clone)]
pub(crate) struct Lexer<'a> {
    pub(crate) source: &'a str,
    bytes: &'a [u8],
    pub(crate) at: usize,
    pub(crate) line: u32,
    escaped_name: Option<&'static str>,
    legacy_octal: bool,
}

const NAME_START: u8 = 1;
const NAME_PART: u8 = 2;

/// For each ASCII byte, whether a name can start with it ([`NAME_START`]) or only go on with it ([`NAME_PART`]).
/// Bytes past ASCII are 0 here, and are looked at as characters.
static ASCII_NAME: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut byte = 0;
    while byte < 128 {
        let character = byte as u8;
        table[byte] = if character.is_ascii_alphabetic() || character == b'$' || character == b'_' {
            NAME_START
        } else if character.is_ascii_digit() {
            NAME_PART
        } else {
            0
        };
        byte += 1;
    }
    table
};

pub(crate) fn is_ident_start(character: char) -> bool {
    character == '$'
        || character == '_'
        || character.is_ascii_alphabetic()
        || (!character.is_ascii()
            && (character.is_alphabetic()
                || matches!(
                    character,
                    '\u{2118}' | '\u{212E}' | '\u{309B}' | '\u{309C}' | '\u{1885}' | '\u{1886}'
                )))
}

pub(crate) fn is_ident_part(character: char) -> bool {
    is_ident_start(character)
        || character.is_ascii_digit()
        || matches!(character, '\u{200C}' | '\u{200D}')
        // Marks and connectors can go in a name too, but Rust has no table of them. Anything that's not a space or
        // punctuation is taken to be one, as code with a mistake in it here would fail soon after anyway.
        || (!character.is_ascii()
            && !character.is_whitespace()
            && !character.is_control()
            && !matches!(character, '\u{2000}'..='\u{206F}' | '\u{3000}'..='\u{3004}' | '\u{FEFF}'))
}

fn push_code(out: &mut Vec<u16>, code: u32) {
    match char::from_u32(code) {
        Some(character) => {
            let mut units = [0u16; 2];
            out.extend_from_slice(character.encode_utf16(&mut units));
        }
        // A surrogate on its own is allowed in a JavaScript string.
        None => out.push(code as u16),
    }
}

impl<'a> Lexer<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            at: 0,
            line: 1,
            escaped_name: None,
            legacy_octal: false,
        }
    }

    pub(crate) fn error<T>(&self, message: impl Into<String>) -> Parsed<T> {
        Err(SyntaxError {
            message: message.into(),
            line: self.line,
        })
    }

    #[inline]
    fn peek_byte(&self, ahead: usize) -> u8 {
        self.bytes.get(self.at + ahead).copied().unwrap_or(0)
    }

    fn peek_char(&self) -> Option<char> {
        self.source[self.at..].chars().next()
    }

    /// Goes past spaces and comments, and says if a line ended among them.
    fn skip_space(&mut self) -> Parsed<bool> {
        let mut newline = false;
        loop {
            match self.peek_byte(0) {
                b' ' | b'\t' | 0x0B | 0x0C => self.at += 1,
                b'\r' => {
                    self.at += 1;
                    if self.peek_byte(0) != b'\n' {
                        self.line += 1;
                    }
                    newline = true;
                }
                b'\n' => {
                    self.at += 1;
                    self.line += 1;
                    newline = true;
                }
                b'/' if self.peek_byte(1) == b'/' => {
                    while self.at < self.bytes.len()
                        && !matches!(self.bytes[self.at], b'\n' | b'\r')
                    {
                        self.at += 1;
                    }
                }
                b'/' if self.peek_byte(1) == b'*' => {
                    self.at += 2;
                    loop {
                        if self.at >= self.bytes.len() {
                            return self.error("a comment doesn't end");
                        }
                        if self.bytes[self.at] == b'*' && self.peek_byte(1) == b'/' {
                            self.at += 2;
                            break;
                        }
                        if self.bytes[self.at] == b'\n' {
                            self.line += 1;
                            newline = true;
                        }
                        self.at += 1;
                    }
                }
                byte if byte >= 0x80 => match self.peek_char() {
                    Some('\u{2028}' | '\u{2029}') => {
                        self.at += 3;
                        self.line += 1;
                        newline = true;
                    }
                    Some('\u{FEFF}') => self.at += 3,
                    Some(character) if character.is_whitespace() => {
                        self.at += character.len_utf8();
                    }
                    _ => return Ok(newline),
                },
                _ => return Ok(newline),
            }
        }
    }

    pub(crate) fn next(&mut self) -> Parsed<Token> {
        let newline_before = self.skip_space()?;
        let start = self.at;
        let line = self.line;
        self.escaped_name = None;
        self.legacy_octal = false;
        let kind = self.read_token()?;
        Ok(Token {
            kind,
            start: start as u32,
            end: self.at as u32,
            line,
            newline_before,
            escaped_name: self.escaped_name,
            legacy_octal: self.legacy_octal,
        })
    }

    fn read_token(&mut self) -> Parsed<Tok> {
        let Some(&byte) = self.bytes.get(self.at) else {
            return Ok(Tok::Eof);
        };
        // Nearly all code is ASCII, and what a token is can be told from its first byte.
        if byte < 0x80 {
            return match byte {
                b'a'..=b'z' | b'A'..=b'Z' | b'$' | b'_' | b'\\' => {
                    self.read_ident()?;
                    Ok(Tok::Ident)
                }
                b'0'..=b'9' => self.read_number(),
                b'.' if self.peek_byte(1).is_ascii_digit() => self.read_number(),
                b'"' | b'\'' => self.read_string(byte as char),
                b'`' => {
                    self.at += 1;
                    self.read_template_part()
                }
                b'#' => self.read_private(),
                _ => self.read_punct(byte),
            };
        }
        let Some(character) = self.peek_char() else {
            return Ok(Tok::Eof);
        };
        if is_ident_start(character) {
            self.read_ident()?;
            return Ok(Tok::Ident);
        }
        self.error(format!("didn't expect {character:?} here"))
    }

    fn read_private(&mut self) -> Parsed<Tok> {
        self.at += 1;
        self.read_ident()?;
        if let Some(name) = self.escaped_name {
            // It's kept with its `#`, as every private name is.
            let with_hash: &'static str = Box::leak(format!("#{name}").into_boxed_str());
            self.escaped_name = Some(with_hash);
        }
        Ok(Tok::Private)
    }

    /// Reads an operator or a bit of punctuation, the longest one that's there, from its first byte.
    fn read_punct(&mut self, first: u8) -> Parsed<Tok> {
        let second = self.peek_byte(1);
        let third = self.peek_byte(2);
        let punct: &'static str = match first {
            b'{' => "{",
            b'}' => "}",
            b'(' => "(",
            b')' => ")",
            b'[' => "[",
            b']' => "]",
            b';' => ";",
            b',' => ",",
            b':' => ":",
            b'~' => "~",
            b'@' => "@",
            b'.' => {
                if second == b'.' && third == b'.' {
                    "..."
                } else {
                    "."
                }
            }
            b'=' => match second {
                b'=' if third == b'=' => "===",
                b'=' => "==",
                b'>' => "=>",
                _ => "=",
            },
            b'!' => match second {
                b'=' if third == b'=' => "!==",
                b'=' => "!=",
                _ => "!",
            },
            b'<' => match second {
                b'<' if third == b'=' => "<<=",
                b'<' => "<<",
                b'=' => "<=",
                _ => "<",
            },
            b'>' => match second {
                b'>' if third == b'>' => {
                    if self.peek_byte(3) == b'=' {
                        ">>>="
                    } else {
                        ">>>"
                    }
                }
                b'>' if third == b'=' => ">>=",
                b'>' => ">>",
                b'=' => ">=",
                _ => ">",
            },
            b'+' => match second {
                b'+' => "++",
                b'=' => "+=",
                _ => "+",
            },
            b'-' => match second {
                b'-' => "--",
                b'=' => "-=",
                _ => "-",
            },
            b'*' => match second {
                b'*' if third == b'=' => "**=",
                b'*' => "**",
                b'=' => "*=",
                _ => "*",
            },
            b'/' => {
                if second == b'=' {
                    "/="
                } else {
                    "/"
                }
            }
            b'%' => {
                if second == b'=' {
                    "%="
                } else {
                    "%"
                }
            }
            b'^' => {
                if second == b'=' {
                    "^="
                } else {
                    "^"
                }
            }
            b'&' => match second {
                b'&' if third == b'=' => "&&=",
                b'&' => "&&",
                b'=' => "&=",
                _ => "&",
            },
            b'|' => match second {
                b'|' if third == b'=' => "||=",
                b'|' => "||",
                b'=' => "|=",
                _ => "|",
            },
            b'?' => match second {
                b'?' if third == b'=' => "??=",
                b'?' => "??",
                // A `?.` before a digit is a `?` and then a number.
                b'.' if !third.is_ascii_digit() => "?.",
                _ => "?",
            },
            _ => return self.error(format!("didn't expect {:?} here", first as char)),
        };
        self.at += punct.len();
        Ok(Tok::Punct(punct))
    }

    fn read_ident(&mut self) -> Parsed<()> {
        let start = self.at;
        // Most names are only ASCII letters and digits, which can be gone past a byte at a time.
        if self
            .bytes
            .get(start)
            .is_some_and(|&byte| ASCII_NAME[byte as usize] == NAME_START)
        {
            let mut at = start + 1;
            while at < self.bytes.len() && ASCII_NAME[self.bytes[at] as usize] != 0 {
                at += 1;
            }
            self.at = at;
            if self
                .bytes
                .get(at)
                .is_none_or(|&byte| byte < 0x80 && byte != b'\\')
            {
                return Ok(());
            }
        }
        let mut decoded: Option<String> = None;
        while let Some(character) = self.peek_char() {
            let is_first = self.at == start;
            if is_ident_part(character) && (!is_first || is_ident_start(character)) {
                self.at += character.len_utf8();
                if let Some(name) = &mut decoded {
                    name.push(character);
                }
            } else if character == '\\' {
                let before = decoded.get_or_insert_with(|| self.source[start..self.at].to_owned());
                let mut name = std::mem::take(before);
                self.at += 1;
                if self.peek_byte(0) != b'u' {
                    return self.error("a bad escape in a name");
                }
                self.at += 1;
                let mut units = Vec::new();
                self.read_escape_code(&mut units)?;
                let Some(Ok(found)) = char::decode_utf16(units).next() else {
                    return self.error("a bad escape in a name");
                };
                let fits = if name.is_empty() {
                    is_ident_start(found)
                } else {
                    is_ident_part(found)
                };
                if !fits {
                    return self.error("a bad escape in a name");
                }
                name.push(found);
                decoded = Some(name);
            } else {
                break;
            }
        }
        if self.at == start {
            return self.error("expected a name");
        }
        // Names are normally pieces of the source. One this rare can just stay.
        if let Some(name) = decoded {
            self.escaped_name = Some(Box::leak(name.into_boxed_str()));
        }
        Ok(())
    }

    fn read_number(&mut self) -> Parsed<Tok> {
        let start = self.at;
        if self.peek_byte(0) == b'0' {
            let radix = match self.peek_byte(1) | 0x20 {
                b'x' => 16,
                b'o' => 8,
                b'b' => 2,
                _ => 0,
            };
            if radix != 0 {
                self.at += 2;
                let mut value = 0f64;
                let mut any = false;
                loop {
                    let byte = self.peek_byte(0);
                    if byte == b'_' {
                        let next_is_digit = (self.peek_byte(1) as char).is_digit(radix);
                        if !any || !next_is_digit {
                            return self.error("a separator has to be between digits");
                        }
                        self.at += 1;
                        continue;
                    }
                    let Some(digit) = (byte as char).to_digit(radix) else {
                        break;
                    };
                    value = value * f64::from(radix) + f64::from(digit);
                    any = true;
                    self.at += 1;
                }
                if !any {
                    return self.error("a number has no digits");
                }
                if self.peek_byte(0) == b'n' {
                    let digits: String = self.source[start + 2..self.at]
                        .chars()
                        .filter(|character| *character != '_')
                        .collect();
                    self.at += 1;
                    return Ok(Tok::BigInt(digits.into_boxed_str(), radix));
                }
                return self.after_number(value);
            }
            // A number that starts with a zero is in octal, in old code.
            let mut ahead = 1;
            while self.peek_byte(ahead).is_ascii_digit() {
                ahead += 1;
            }
            if ahead > 1 {
                self.legacy_octal = true;
                if self.peek_byte(ahead) == b'_' {
                    return self.error("a number that starts with 0 can't have a separator");
                }
            }
            if ahead > 1 && (1..ahead).all(|at| self.peek_byte(at) < b'8') {
                let value = (1..ahead).fold(0f64, |value, at| {
                    value * 8.0 + f64::from(self.peek_byte(at) - b'0')
                });
                self.at += ahead;
                return self.after_number(value);
            }
        }
        let digits = |lexer: &mut Self| {
            while lexer.peek_byte(0).is_ascii_digit() || lexer.peek_byte(0) == b'_' {
                lexer.at += 1;
            }
        };
        if self.peek_byte(0) == b'0' && self.peek_byte(1) == b'_' {
            return self.error("a number that starts with 0 can't have a separator");
        }
        digits(self);
        if self.peek_byte(0) == b'.' {
            self.at += 1;
            digits(self);
        }
        if self.peek_byte(0) | 0x20 == b'e' {
            let sign = usize::from(matches!(self.peek_byte(1), b'+' | b'-'));
            if self.peek_byte(1 + sign).is_ascii_digit() {
                self.at += 1 + sign;
                digits(self);
            }
        }
        let written = &self.source[start..self.at];
        // Separators are rare, and only need looking at when there are some.
        let separated = written.contains('_');
        if separated
            && (written.contains("__")
                || written.ends_with('_')
                || written.contains("_.")
                || written.contains("._")
                || written.contains("_e")
                || written.contains("_E")
                || written.contains("e_")
                || written.contains("E_"))
        {
            return self.error("a separator has to be between digits");
        }
        let text: std::borrow::Cow<str> = if separated {
            written
                .chars()
                .filter(|character| *character != '_')
                .collect::<String>()
                .into()
        } else {
            written.into()
        };
        if self.peek_byte(0) == b'n' {
            self.at += 1;
            return Ok(Tok::BigInt(text.into_owned().into_boxed_str(), 10));
        }
        let Ok(value) = text.parse::<f64>() else {
            return self.error("that isn't a number");
        };
        self.after_number(value)
    }

    fn after_number(&mut self, value: f64) -> Parsed<Tok> {
        if let Some(character) = self.peek_char() {
            if is_ident_start(character) {
                return self.error("a name can't start right after a number");
            }
        }
        Ok(Tok::Number(value))
    }

    /// Goes past a string, checking it. What's in it is only worked out when it's wanted, by [`string_units`], so a
    /// string costs nothing more than this however many times it's looked at.
    fn read_string(&mut self, quote: char) -> Parsed<Tok> {
        self.at += 1;
        // Up to the first escape, line end or character past ASCII, each byte is a unit of the string.
        let plain = self.bytes[self.at..]
            .iter()
            .position(|&byte| {
                byte == quote as u8 || matches!(byte, b'\\' | b'\n' | b'\r') || byte >= 0x80
            })
            .unwrap_or(self.bytes.len() - self.at);
        self.at += plain;
        if self.peek_byte(0) == quote as u8 {
            self.at += 1;
            return Ok(Tok::String);
        }
        let mut out = Vec::new();
        self.read_string_rest(quote, &mut out)?;
        Ok(Tok::String)
    }

    /// Reads the rest of a string, from somewhere inside it, to just past its quote.
    fn read_string_rest(&mut self, quote: char, out: &mut Vec<u16>) -> Parsed<()> {
        loop {
            let Some(character) = self.peek_char() else {
                return self.error("a string doesn't end");
            };
            self.at += character.len_utf8();
            if character == quote {
                return Ok(());
            }
            match character {
                '\\' => self.read_escape(out)?,
                '\n' | '\r' => return self.error("a string doesn't end on its line"),
                _ => push_code(out, character as u32),
            }
        }
    }

    /// Reads what's after a backslash.
    fn read_escape(&mut self, out: &mut Vec<u16>) -> Parsed<()> {
        let Some(character) = self.peek_char() else {
            return self.error("a string doesn't end");
        };
        self.at += character.len_utf8();
        match character {
            'n' => out.push(10),
            't' => out.push(9),
            'r' => out.push(13),
            'b' => out.push(8),
            'f' => out.push(12),
            'v' => out.push(11),
            '0' if !self.peek_byte(0).is_ascii_digit() => out.push(0),
            'x' => {
                let code = self.read_hex(2)?;
                out.push(code as u16);
            }
            'u' => self.read_escape_code(out)?,
            '1'..='7' | '0' => {
                // An old-style octal escape.
                self.legacy_octal = true;
                let mut code = character.to_digit(8).unwrap_or(0);
                let mut digits = 1;
                while digits < 3 {
                    let Some(digit) = (self.peek_byte(0) as char).to_digit(8) else {
                        break;
                    };
                    if code * 8 + digit > 255 {
                        break;
                    }
                    code = code * 8 + digit;
                    self.at += 1;
                    digits += 1;
                }
                out.push(code as u16);
            }
            '8' | '9' => {
                self.legacy_octal = true;
                push_code(out, character as u32);
            }
            '\r' => {
                if self.peek_byte(0) == b'\n' {
                    self.at += 1;
                }
                self.line += 1;
            }
            '\n' | '\u{2028}' | '\u{2029}' => self.line += 1,
            _ => push_code(out, character as u32),
        }
        Ok(())
    }

    /// Reads what's after a `\\u`.
    fn read_escape_code(&mut self, out: &mut Vec<u16>) -> Parsed<()> {
        let code = if self.peek_byte(0) == b'{' {
            self.at += 1;
            let mut code = 0u32;
            let mut any = false;
            while self.peek_byte(0) != b'}' {
                let Some(digit) = (self.peek_byte(0) as char).to_digit(16) else {
                    return self.error("a bad escape in a string");
                };
                code = code.saturating_mul(16).saturating_add(digit);
                self.at += 1;
                any = true;
            }
            if !any {
                return self.error("a bad escape in a string");
            }
            self.at += 1;
            code
        } else {
            self.read_hex(4)?
        };
        if code > 0x10FFFF {
            return self.error("a bad escape in a string");
        }
        push_code(out, code);
        Ok(())
    }

    fn read_hex(&mut self, count: usize) -> Parsed<u32> {
        let mut code = 0;
        for _ in 0..count {
            let Some(digit) = (self.peek_byte(0) as char).to_digit(16) else {
                return self.error("a bad escape in a string");
            };
            code = code * 16 + digit;
            self.at += 1;
        }
        Ok(code)
    }

    /// Reads a template from where its text starts, up to `${` or where it ends.
    pub(crate) fn read_template_part(&mut self) -> Parsed<Tok> {
        let mut cooked = Vec::new();
        let mut raw = Vec::new();
        loop {
            let Some(character) = self.peek_char() else {
                return self.error("a template doesn't end");
            };
            self.at += character.len_utf8();
            match character {
                '`' => {
                    return Ok(Tok::Template {
                        cooked: cooked.into_boxed_slice(),
                        raw: raw.into_boxed_slice(),
                        tail: true,
                    });
                }
                '$' if self.peek_byte(0) == b'{' => {
                    self.at += 1;
                    return Ok(Tok::Template {
                        cooked: cooked.into_boxed_slice(),
                        raw: raw.into_boxed_slice(),
                        tail: false,
                    });
                }
                '\\' => {
                    let start = self.at;
                    self.read_escape(&mut cooked)?;
                    raw.push(u16::from(b'\\'));
                    raw.extend(self.source[start..self.at].encode_utf16());
                }
                _ => {
                    if character == '\n' {
                        self.line += 1;
                    }
                    push_code(&mut cooked, character as u32);
                    push_code(&mut raw, character as u32);
                }
            }
        }
    }

    /// Reads a regular expression, from the `/` that's at `start`, and its flags.
    pub(crate) fn read_regex(&mut self, start: usize) -> Parsed<RegexParts> {
        self.at = start + 1;
        let mut in_class = false;
        loop {
            let Some(character) = self.peek_char() else {
                return self.error("a regular expression doesn't end");
            };
            self.at += character.len_utf8();
            match character {
                '\\' => {
                    if let Some(next) = self.peek_char() {
                        self.at += next.len_utf8();
                    }
                }
                '[' => in_class = true,
                ']' => in_class = false,
                '/' if !in_class => break,
                '\n' | '\r' => return self.error("a regular expression doesn't end on its line"),
                _ => {}
            }
        }
        let pattern: Vec<u16> = self.source[start + 1..self.at - 1].encode_utf16().collect();
        let flags_start = self.at;
        while let Some(character) = self.peek_char() {
            if !is_ident_part(character) {
                break;
            }
            self.at += character.len_utf8();
        }
        let flags: Vec<u16> = self.source[flags_start..self.at].encode_utf16().collect();
        Ok((pattern.into_boxed_slice(), flags.into_boxed_slice()))
    }

    /// Reads text between JSX tags, up to the next `<` or `{`.
    pub(crate) fn read_jsx_text(&mut self) -> &'a str {
        let start = self.at;
        while self.at < self.bytes.len() && !matches!(self.bytes[self.at], b'<' | b'{') {
            if self.bytes[self.at] == b'\n' {
                self.line += 1;
            }
            self.at += 1;
        }
        &self.source[start..self.at]
    }

    /// Reads a name in JSX, which can have dashes in it. It starts at `start`, where a name's already been read.
    pub(crate) fn read_jsx_name(&mut self, start: usize) -> &'a str {
        self.at = start;
        while let Some(character) = self.peek_char() {
            if is_ident_part(character) || character == '-' {
                self.at += character.len_utf8();
            } else {
                break;
            }
        }
        &self.source[start..self.at]
    }
}

/// What the string token from `start` to `end` in `source` says, which the lexer has already checked.
pub(crate) fn string_units(source: &str, start: u32, end: u32) -> Vec<u16> {
    let inside = &source.as_bytes()[start as usize + 1..end as usize - 1];
    if !inside.iter().any(|&byte| byte == b'\\' || byte >= 0x80) {
        return inside.iter().map(|&byte| u16::from(byte)).collect();
    }
    let mut lexer = Lexer::new(source);
    lexer.at = start as usize + 1;
    let quote = char::from(source.as_bytes()[start as usize]);
    let mut out = Vec::with_capacity(inside.len());
    // It was read without a mistake when it was a token, so it is again.
    let _ = lexer.read_string_rest(quote, &mut out);
    out
}
