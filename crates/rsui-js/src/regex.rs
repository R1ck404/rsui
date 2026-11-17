//! Regular expressions: a pattern is compiled to a small program, which is run by trying things and going back when
//! they don't work out. Going back is kept on a list of its own, not on Rust's stack, so a long match can't overflow it.

use std::rc::Rc;

use crate::object::*;
use crate::text::{self, Text};
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

#[derive(Debug, Clone)]
enum Node {
    Empty,
    Char(u32),
    Any,
    Class(Box<ClassSet>),
    Start,
    End,
    WordBoundary,
    NotWordBoundary,
    Group(Box<Node>, Option<u32>),
    BackRef(u32),
    NamedBackRef(String),
    Look {
        node: Box<Node>,
        ahead: bool,
        negative: bool,
    },
    Repeat {
        node: Box<Node>,
        min: u32,
        max: u32,
        greedy: bool,
    },
    /// A group with its own flags, like `(?i:a)`.
    Flags {
        node: Box<Node>,
        ignore_case: Option<bool>,
        multiline: Option<bool>,
        dot_all: Option<bool>,
    },
    Concat(Vec<Node>),
    Alternate(Vec<Node>),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ClassSet {
    ranges: Vec<(u32, u32)>,
    properties: Vec<(Property, bool)>,
    negated: bool,
    /// Which ASCII characters are in this, without and with ignoring case, so those need no searching.
    ascii: [AsciiSet; 2],
}

type AsciiSet = [u64; 2];

#[inline(always)]
fn ascii_has(set: &AsciiSet, code: u32) -> bool {
    set[(code >> 6) as usize] & (1 << (code & 63)) != 0
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Property {
    Letter,
    Upper,
    Lower,
    Number,
    Whitespace,
    Punctuation,
    Any,
    IdentifierPart,
    Ranges(&'static [(u32, u32)]),
}

fn property_has(property: Property, code: u32) -> bool {
    if let Property::Ranges(ranges) = property {
        return ranges
            .iter()
            .any(|(low, high)| *low <= code && code <= *high);
    }
    let Some(character) = char::from_u32(code) else {
        return property == Property::Any;
    };
    match property {
        Property::IdentifierPart => crate::lexer::is_ident_part(character),
        Property::Ranges(_) => false,
        Property::Letter => character.is_alphabetic(),
        Property::Upper => character.is_uppercase(),
        Property::Lower => character.is_lowercase(),
        Property::Number => character.is_numeric(),
        Property::Whitespace => character.is_whitespace(),
        Property::Punctuation => {
            character.is_ascii_punctuation()
                || (!character.is_ascii()
                    && !character.is_alphanumeric()
                    && !character.is_whitespace()
                    && !character.is_control())
        }
        Property::Any => true,
    }
}

const NO_LIMIT: u32 = u32::MAX;
const MAX_INSTRUCTIONS: usize = 1 << 20;

fn is_word(code: u32) -> bool {
    code < 128 && ((code as u8).is_ascii_alphanumeric() || code == 95)
}

fn is_line_end(code: u32) -> bool {
    matches!(code, 10 | 13 | 0x2028 | 0x2029)
}

struct PatternParser<'a> {
    units: &'a [u16],
    at: usize,
    groups: u32,
    names: Vec<(String, u32)>,
    unicode: bool,
    /// Whether there's a named group anywhere, which changes what `\\k` means.
    has_named_groups: bool,
    total_groups: u32,
    /// What went wrong while reading an escape, which can't say so where it is.
    failure: Option<String>,
}

/// The names of what `\\p{...}` can ask about on its own, other than categories.
const BINARY_PROPERTIES: &[&str] = &[
    "ASCII",
    "ASCII_Hex_Digit",
    "AHex",
    "Alphabetic",
    "Alpha",
    "Any",
    "Assigned",
    "Bidi_Control",
    "Bidi_C",
    "Bidi_Mirrored",
    "Bidi_M",
    "Case_Ignorable",
    "CI",
    "Cased",
    "Changes_When_Casefolded",
    "CWCF",
    "Changes_When_Casemapped",
    "CWCM",
    "Changes_When_Lowercased",
    "CWL",
    "Changes_When_NFKC_Casefolded",
    "CWKCF",
    "Changes_When_Titlecased",
    "CWT",
    "Changes_When_Uppercased",
    "CWU",
    "Dash",
    "Default_Ignorable_Code_Point",
    "DI",
    "Deprecated",
    "Dep",
    "Diacritic",
    "Dia",
    "Emoji",
    "Emoji_Component",
    "EComp",
    "Emoji_Modifier",
    "EMod",
    "Emoji_Modifier_Base",
    "EBase",
    "Emoji_Presentation",
    "EPres",
    "Extended_Pictographic",
    "ExtPict",
    "Extender",
    "Ext",
    "Grapheme_Base",
    "Gr_Base",
    "Grapheme_Extend",
    "Gr_Ext",
    "Hex_Digit",
    "Hex",
    "IDS_Binary_Operator",
    "IDSB",
    "IDS_Trinary_Operator",
    "IDST",
    "ID_Continue",
    "IDC",
    "ID_Start",
    "IDS",
    "Ideographic",
    "Ideo",
    "Join_Control",
    "Join_C",
    "Logical_Order_Exception",
    "LOE",
    "Lowercase",
    "Lower",
    "Math",
    "Noncharacter_Code_Point",
    "NChar",
    "Pattern_Syntax",
    "Pat_Syn",
    "Pattern_White_Space",
    "Pat_WS",
    "Quotation_Mark",
    "QMark",
    "Radical",
    "Regional_Indicator",
    "RI",
    "Sentence_Terminal",
    "STerm",
    "Soft_Dotted",
    "SD",
    "Terminal_Punctuation",
    "Term",
    "Unified_Ideograph",
    "UIdeo",
    "Uppercase",
    "Upper",
    "Variation_Selector",
    "VS",
    "White_Space",
    "space",
    "XID_Continue",
    "XIDC",
    "XID_Start",
    "XIDS",
];

/// The names of general categories, short and long.
const CATEGORIES: &[&str] = &[
    "L",
    "Letter",
    "LC",
    "Cased_Letter",
    "Lu",
    "Uppercase_Letter",
    "Ll",
    "Lowercase_Letter",
    "Lt",
    "Titlecase_Letter",
    "Lm",
    "Modifier_Letter",
    "Lo",
    "Other_Letter",
    "M",
    "Mark",
    "Combining_Mark",
    "Mn",
    "Nonspacing_Mark",
    "Mc",
    "Spacing_Mark",
    "Me",
    "Enclosing_Mark",
    "N",
    "Number",
    "Nd",
    "Decimal_Number",
    "digit",
    "Nl",
    "Letter_Number",
    "No",
    "Other_Number",
    "P",
    "Punctuation",
    "punct",
    "Pc",
    "Connector_Punctuation",
    "Pd",
    "Dash_Punctuation",
    "Ps",
    "Open_Punctuation",
    "Pe",
    "Close_Punctuation",
    "Pi",
    "Initial_Punctuation",
    "Pf",
    "Final_Punctuation",
    "Po",
    "Other_Punctuation",
    "S",
    "Symbol",
    "Sm",
    "Math_Symbol",
    "Sc",
    "Currency_Symbol",
    "Sk",
    "Modifier_Symbol",
    "So",
    "Other_Symbol",
    "Z",
    "Separator",
    "Zs",
    "Space_Separator",
    "Zl",
    "Line_Separator",
    "Zp",
    "Paragraph_Separator",
    "C",
    "Other",
    "Cc",
    "Control",
    "cntrl",
    "Cf",
    "Format",
    "Cs",
    "Surrogate",
    "Co",
    "Private_Use",
    "Cn",
    "Unassigned",
];

/// Scripts this can actually tell apart, by their main blocks. Others are matched too loosely, but are accepted.
fn script_ranges(name: &str) -> Option<&'static [(u32, u32)]> {
    Some(match name {
        "Latin" | "Latn" => &[
            (0x41, 0x5A),
            (0x61, 0x7A),
            (0xAA, 0xAA),
            (0xBA, 0xBA),
            (0xC0, 0xD6),
            (0xD8, 0xF6),
            (0xF8, 0x24F),
            (0x1E00, 0x1EFF),
            (0xFF21, 0xFF3A),
            (0xFF41, 0xFF5A),
        ],
        "Greek" | "Grek" => &[
            (0x370, 0x373),
            (0x375, 0x377),
            (0x37A, 0x37D),
            (0x37F, 0x37F),
            (0x384, 0x384),
            (0x386, 0x386),
            (0x388, 0x3E1),
            (0x3F0, 0x3FF),
            (0x1F00, 0x1FFE),
        ],
        "Cyrillic" | "Cyrl" => &[
            (0x400, 0x484),
            (0x487, 0x52F),
            (0x1C80, 0x1C88),
            (0x2DE0, 0x2DFF),
            (0xA640, 0xA69F),
        ],
        "Armenian" | "Armn" => &[(0x531, 0x556), (0x559, 0x58A), (0x58D, 0x58F)],
        "Hebrew" | "Hebr" => &[
            (0x591, 0x5C7),
            (0x5D0, 0x5EA),
            (0x5EF, 0x5F4),
            (0xFB1D, 0xFB4F),
        ],
        "Arabic" | "Arab" => &[
            (0x600, 0x604),
            (0x606, 0x60B),
            (0x60D, 0x61A),
            (0x61C, 0x61E),
            (0x620, 0x63F),
            (0x641, 0x64A),
            (0x656, 0x66F),
            (0x671, 0x6DC),
            (0x6DE, 0x6FF),
            (0x750, 0x77F),
            (0x8A0, 0x8FF),
            (0xFB50, 0xFDFF),
            (0xFE70, 0xFEFC),
        ],
        "Devanagari" | "Deva" => &[
            (0x900, 0x950),
            (0x955, 0x963),
            (0x966, 0x97F),
            (0xA8E0, 0xA8FF),
        ],
        "Thai" => &[(0xE01, 0xE3A), (0xE40, 0xE5B)],
        "Hangul" | "Hang" => &[
            (0x1100, 0x11FF),
            (0x302E, 0x302F),
            (0x3131, 0x318E),
            (0x3200, 0x321E),
            (0x3260, 0x327E),
            (0xA960, 0xA97C),
            (0xAC00, 0xD7A3),
            (0xD7B0, 0xD7FB),
            (0xFFA0, 0xFFDC),
        ],
        "Hiragana" | "Hira" => &[
            (0x3041, 0x3096),
            (0x309D, 0x309F),
            (0x1B001, 0x1B11F),
            (0x1F200, 0x1F200),
        ],
        "Katakana" | "Kana" => &[
            (0x30A1, 0x30FA),
            (0x30FD, 0x30FF),
            (0x31F0, 0x31FF),
            (0x32D0, 0x32FE),
            (0x3300, 0x3357),
            (0xFF66, 0xFF6F),
            (0xFF71, 0xFF9D),
        ],
        "Han" | "Hani" => &[
            (0x2E80, 0x2E99),
            (0x2E9B, 0x2EF3),
            (0x2F00, 0x2FD5),
            (0x3005, 0x3005),
            (0x3007, 0x3007),
            (0x3021, 0x3029),
            (0x3038, 0x303B),
            (0x3400, 0x4DBF),
            (0x4E00, 0x9FFF),
            (0xF900, 0xFAD9),
            (0x20000, 0x323AF),
        ],
        _ => return None,
    })
}

/// Combining marks, by their main blocks, as Rust has no table of them.
const MARKS: &[(u32, u32)] = &[
    (0x300, 0x36F),
    (0x483, 0x489),
    (0x591, 0x5BD),
    (0x5BF, 0x5BF),
    (0x5C1, 0x5C2),
    (0x5C4, 0x5C5),
    (0x610, 0x61A),
    (0x64B, 0x65F),
    (0x670, 0x670),
    (0x6D6, 0x6DC),
    (0x6DF, 0x6E4),
    (0x6E7, 0x6E8),
    (0x6EA, 0x6ED),
    (0x900, 0x903),
    (0x93A, 0x93C),
    (0x93E, 0x94F),
    (0x951, 0x957),
    (0x1AB0, 0x1AFF),
    (0x1DC0, 0x1DFF),
    (0x20D0, 0x20F0),
    (0x302A, 0x302F),
    (0x3099, 0x309A),
    (0xFE00, 0xFE0F),
    (0xFE20, 0xFE2F),
    (0xE0100, 0xE01EF),
];

/// Emoji, roughly: the blocks that are nearly all emoji.
const EMOJI: &[(u32, u32)] = &[
    (0x23, 0x23),
    (0x2A, 0x2A),
    (0x30, 0x39),
    (0xA9, 0xA9),
    (0xAE, 0xAE),
    (0x203C, 0x203C),
    (0x2049, 0x2049),
    (0x2122, 0x2122),
    (0x2139, 0x2139),
    (0x2194, 0x21AA),
    (0x231A, 0x23FF),
    (0x25AA, 0x25FE),
    (0x2600, 0x27BF),
    (0x2934, 0x2935),
    (0x2B05, 0x2B55),
    (0x3030, 0x303D),
    (0x3297, 0x3299),
    (0x1F000, 0x1FAFF),
];

type Parsing<T> = Result<T, String>;

impl PatternParser<'_> {
    fn peek(&self) -> Option<u16> {
        self.units.get(self.at).copied()
    }

    fn peek_is(&self, character: char) -> bool {
        self.peek() == Some(character as u16)
    }

    fn eat(&mut self, character: char) -> bool {
        if self.peek_is(character) {
            self.at += 1;
            return true;
        }
        false
    }

    fn looking_at(&self, text: &str) -> bool {
        let wanted: Vec<u16> = text.encode_utf16().collect();
        self.units[self.at..].starts_with(&wanted)
    }

    fn alternatives(&mut self) -> Parsing<Node> {
        let mut options = vec![self.sequence()?];
        while self.eat('|') {
            options.push(self.sequence()?);
        }
        Ok(if options.len() == 1 {
            options.pop().unwrap_or(Node::Empty)
        } else {
            Node::Alternate(options)
        })
    }

    fn sequence(&mut self) -> Parsing<Node> {
        let mut items = Vec::new();
        while let Some(unit) = self.peek() {
            if unit == '|' as u16 || unit == ')' as u16 {
                break;
            }
            let atom = self.atom()?;
            items.push(self.quantified(atom)?);
        }
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.pop().unwrap_or(Node::Empty),
            _ => Node::Concat(items),
        })
    }

    fn number(&mut self) -> Option<u32> {
        let start = self.at;
        let mut value = 0u32;
        while let Some(unit) = self.peek() {
            let Some(digit) = char::from_u32(u32::from(unit)).and_then(|c| c.to_digit(10)) else {
                break;
            };
            value = value.saturating_mul(10).saturating_add(digit);
            self.at += 1;
        }
        (self.at > start).then_some(value)
    }

    fn quantified(&mut self, atom: Node) -> Parsing<Node> {
        let (min, max) = match self.peek().and_then(|unit| char::from_u32(u32::from(unit))) {
            Some('*') => {
                self.at += 1;
                (0, NO_LIMIT)
            }
            Some('+') => {
                self.at += 1;
                (1, NO_LIMIT)
            }
            Some('?') => {
                self.at += 1;
                (0, 1)
            }
            Some('{') => {
                let saved = self.at;
                self.at += 1;
                let parsed = (|| {
                    let min = self.number()?;
                    let max = if self.eat(',') {
                        if self.peek_is('}') {
                            NO_LIMIT
                        } else {
                            self.number()?
                        }
                    } else {
                        min
                    };
                    self.eat('}').then_some((min, max))
                })();
                match parsed {
                    Some((min, max)) if min > max => {
                        return Err("numbers out of order in a { } count".to_owned());
                    }
                    Some(limits) => limits,
                    None if self.unicode => {
                        return Err("a { has to be escaped in unicode mode".to_owned());
                    }
                    None => {
                        // Outside of unicode mode, a `{` that isn't a count is just itself.
                        self.at = saved;
                        return Ok(atom);
                    }
                }
            }
            _ => return Ok(atom),
        };
        if let Node::Look { ahead, .. } = &atom {
            if self.unicode || !*ahead {
                return Err("nothing to repeat".to_owned());
            }
        }
        if matches!(
            atom,
            Node::Start | Node::End | Node::WordBoundary | Node::NotWordBoundary
        ) {
            return Err("nothing to repeat".to_owned());
        }
        let greedy = !self.eat('?');
        Ok(Node::Repeat {
            node: Box::new(atom),
            min,
            max,
            greedy,
        })
    }

    fn next_code(&mut self) -> Parsing<u32> {
        let Some(unit) = self.peek() else {
            return Err("the pattern ends too soon".to_owned());
        };
        self.at += 1;
        if self.unicode && (0xD800..0xDC00).contains(&unit) {
            if let Some(low) = self.peek() {
                if (0xDC00..0xE000).contains(&low) {
                    self.at += 1;
                    return Ok(0x10000
                        + ((u32::from(unit) - 0xD800) << 10)
                        + (u32::from(low) - 0xDC00));
                }
            }
        }
        Ok(u32::from(unit))
    }

    fn atom(&mut self) -> Parsing<Node> {
        let code = self.next_code()?;
        let Some(character) = char::from_u32(code) else {
            return Ok(Node::Char(code));
        };
        Ok(match character {
            '.' => Node::Any,
            '^' => Node::Start,
            '$' => Node::End,
            '(' => return self.group(),
            '[' => Node::Class(Box::new(self.class()?)),
            '\\' => return self.escape(),
            '*' | '+' | '?' => return Err("nothing to repeat".to_owned()),
            ']' | '{' | '}' if self.unicode => {
                return Err(format!("a {character} has to be escaped in unicode mode"));
            }
            '{' => {
                // A count with nothing before it has nothing to repeat.
                let saved = self.at;
                let is_count = self.number().is_some()
                    && (self.eat('}')
                        || (self.eat(',') && {
                            let _ = self.number();
                            self.eat('}')
                        }));
                self.at = saved;
                if is_count {
                    return Err("nothing to repeat".to_owned());
                }
                Node::Char(code)
            }
            _ => Node::Char(code),
        })
    }

    fn group(&mut self) -> Parsing<Node> {
        let node = if self.looking_at("?:") {
            self.at += 2;
            let inner = self.alternatives()?;
            Node::Group(Box::new(inner), None)
        } else if self.looking_at("?=") || self.looking_at("?!") {
            let negative = self.looking_at("?!");
            self.at += 2;
            let inner = self.alternatives()?;
            Node::Look {
                node: Box::new(inner),
                ahead: true,
                negative,
            }
        } else if self.looking_at("?<=") || self.looking_at("?<!") {
            let negative = self.looking_at("?<!");
            self.at += 3;
            let inner = self.alternatives()?;
            Node::Look {
                node: Box::new(inner),
                ahead: false,
                negative,
            }
        } else if self.looking_at("?<") {
            self.at += 2;
            let name = self.group_name()?;
            self.groups += 1;
            let index = self.groups;
            if self.names.iter().any(|(found, _)| *found == name) {
                return Err(format!("the group name {name} is used twice"));
            }
            self.names.push((name, index));
            let inner = self.alternatives()?;
            Node::Group(Box::new(inner), Some(index))
        } else if self.looking_at("?") {
            // Flags for just this group, like `(?i:a)`.
            self.at += 1;
            return self.modifier_group();
        } else {
            self.groups += 1;
            let index = self.groups;
            let inner = self.alternatives()?;
            Node::Group(Box::new(inner), Some(index))
        };
        if !self.eat(')') {
            return Err("a group isn't closed".to_owned());
        }
        Ok(node)
    }

    fn modifier_group(&mut self) -> Parsing<Node> {
        let mut on = String::new();
        let mut off = String::new();
        let mut negative = false;
        loop {
            let Some(unit) = self.peek() else {
                return Err("a group isn't closed".to_owned());
            };
            self.at += 1;
            match char::from_u32(u32::from(unit)) {
                Some(':') => break,
                Some('-') if !negative => negative = true,
                Some(flag @ ('i' | 'm' | 's')) => {
                    if on.contains(flag) || off.contains(flag) {
                        return Err("a flag is there twice in a group".to_owned());
                    }
                    if negative {
                        off.push(flag);
                    } else {
                        on.push(flag);
                    }
                }
                _ => return Err("invalid group".to_owned()),
            }
        }
        if on.is_empty() && off.is_empty() {
            return Err("a group's flags can't be empty".to_owned());
        }
        let inner = self.alternatives()?;
        if !self.eat(')') {
            return Err("a group isn't closed".to_owned());
        }
        Ok(Node::Flags {
            node: Box::new(inner),
            ignore_case: if on.contains('i') {
                Some(true)
            } else if off.contains('i') {
                Some(false)
            } else {
                None
            },
            multiline: if on.contains('m') {
                Some(true)
            } else if off.contains('m') {
                Some(false)
            } else {
                None
            },
            dot_all: if on.contains('s') {
                Some(true)
            } else if off.contains('s') {
                Some(false)
            } else {
                None
            },
        })
    }

    fn group_name(&mut self) -> Parsing<String> {
        let mut name = String::new();
        loop {
            let Some(unit) = self.peek() else {
                return Err("a group's name isn't closed".to_owned());
            };
            if unit == '>' as u16 {
                self.at += 1;
                break;
            }
            self.at += 1;
            let code = if unit == '\\' as u16 {
                if !self.eat('u') {
                    return Err("a bad escape in a group's name".to_owned());
                }
                let saved_unicode = self.unicode;
                self.unicode = true;
                let code = self.char_escape('u');
                self.unicode = saved_unicode;
                code
            } else if (0xD800..0xDC00).contains(&unit)
                && self
                    .peek()
                    .is_some_and(|low| (0xDC00..0xE000).contains(&low))
            {
                let low = u32::from(self.peek().unwrap_or(0));
                self.at += 1;
                0x10000 + ((u32::from(unit) - 0xD800) << 10) + (low - 0xDC00)
            } else {
                u32::from(unit)
            };
            let fits = char::from_u32(code).is_some_and(|character| {
                if name.is_empty() {
                    crate::lexer::is_ident_start(character)
                } else {
                    crate::lexer::is_ident_part(character)
                }
            });
            let Some(character) = char::from_u32(code).filter(|_| fits) else {
                return Err("a group's name isn't a name".to_owned());
            };
            name.push(character);
        }
        if name.is_empty() {
            return Err("a group's name is empty".to_owned());
        }
        Ok(name)
    }

    fn hex(&mut self, count: usize) -> Option<u32> {
        let mut value = 0;
        for offset in 0..count {
            let unit = *self.units.get(self.at + offset)?;
            value = value * 16 + char::from_u32(u32::from(unit))?.to_digit(16)?;
        }
        self.at += count;
        Some(value)
    }

    /// An escape that means a single character.
    fn char_escape(&mut self, character: char) -> u32 {
        match character {
            'n' => 10,
            't' => 9,
            'r' => 13,
            'f' => 12,
            'v' => 11,
            '0' => 0,
            'c' => {
                if let Some(unit) = self.peek() {
                    self.at += 1;
                    u32::from(unit) & 31
                } else {
                    'c' as u32
                }
            }
            'x' => self.hex(2).unwrap_or('x' as u32),
            'u' => {
                if self.eat('{') {
                    let mut value = 0u32;
                    while let Some(unit) = self.peek() {
                        self.at += 1;
                        if unit == '}' as u16 {
                            break;
                        }
                        let digit = char::from_u32(u32::from(unit))
                            .and_then(|c| c.to_digit(16))
                            .unwrap_or(0);
                        value = value.saturating_mul(16).saturating_add(digit);
                    }
                    value
                } else if let Some(first) = self.hex(4) {
                    // A pair written as two escapes is one character, in unicode mode.
                    if self.unicode && (0xD800..0xDC00).contains(&first) && self.looking_at("\\u") {
                        let saved = self.at;
                        self.at += 2;
                        if let Some(low) = self.hex(4) {
                            if (0xDC00..0xE000).contains(&low) {
                                return 0x10000 + ((first - 0xD800) << 10) + (low - 0xDC00);
                            }
                        }
                        self.at = saved;
                    }
                    first
                } else {
                    'u' as u32
                }
            }
            other => other as u32,
        }
    }

    fn set_escape(&mut self, character: char, set: &mut ClassSet) -> bool {
        match character {
            'd' => set.ranges.push((48, 57)),
            'D' => {
                set.ranges.push((0, 47));
                set.ranges.push((58, 0x10FFFF));
            }
            'w' => {
                set.ranges.extend([(48, 57), (65, 90), (95, 95), (97, 122)]);
            }
            'W' => {
                set.ranges
                    .extend([(0, 47), (58, 64), (91, 94), (96, 96), (123, 0x10FFFF)]);
            }
            's' => {
                set.ranges.extend([
                    (9, 13),
                    (32, 32),
                    (0xA0, 0xA0),
                    (0x1680, 0x1680),
                    (0x2000, 0x200A),
                    (0x2028, 0x2029),
                    (0x202F, 0x202F),
                    (0x205F, 0x205F),
                    (0x3000, 0x3000),
                    (0xFEFF, 0xFEFF),
                ]);
            }
            'S' => {
                set.ranges.extend([
                    (0, 8),
                    (14, 31),
                    (33, 0x9F),
                    (0xA1, 0x167F),
                    (0x1681, 0x1FFF),
                    (0x200B, 0x2027),
                    (0x202A, 0x202E),
                    (0x2030, 0x205E),
                    (0x2060, 0x2FFF),
                    (0x3001, 0xFEFE),
                    (0xFF00, 0x10FFFF),
                ]);
            }
            'p' | 'P' if self.unicode => {
                let negated = character == 'P';
                if !self.eat('{') {
                    self.failure = Some("expected { after \\\\p".to_owned());
                    return true;
                }
                let start = self.at;
                while !self.peek_is('}') && self.peek().is_some() {
                    self.at += 1;
                }
                if self.peek().is_none() {
                    self.failure = Some("a property's name isn't closed".to_owned());
                    return true;
                }
                let name = String::from_utf16_lossy(&self.units[start..self.at]);
                self.at += 1;
                self.add_property_escape(&name, negated, set);
            }
            _ => return false,
        }
        true
    }

    fn add_property_escape(&mut self, written: &str, negated: bool, set: &mut ClassSet) {
        let (key, value) = match written.split_once('=') {
            Some((key, value)) => (Some(key), value),
            None => (None, written),
        };
        let valid = match key {
            Some("General_Category" | "gc") => CATEGORIES.contains(&value),
            Some("Script" | "sc" | "Script_Extensions" | "scx") => {
                value.len() >= 2
                    && value.as_bytes()[0].is_ascii_uppercase()
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            }
            Some(_) => false,
            None => CATEGORIES.contains(&value) || BINARY_PROPERTIES.contains(&value),
        };
        if !valid {
            self.failure = Some(format!("invalid property name {written}"));
            return;
        }
        if matches!(key, Some("Script" | "sc" | "Script_Extensions" | "scx")) {
            let property = match script_ranges(value) {
                Some(ranges) => Property::Ranges(ranges),
                None => Property::Any,
            };
            set.properties.push((property, negated));
            return;
        }
        let property = match value {
            "L" | "Letter" | "Alphabetic" | "Alpha" | "ID_Start" | "IDS" | "XID_Start" | "XIDS" => {
                Property::Letter
            }
            "Lu" | "Uppercase_Letter" | "Uppercase" | "Upper" => Property::Upper,
            "Ll" | "Lowercase_Letter" | "Lowercase" | "Lower" => Property::Lower,
            "N" | "Number" | "Nd" | "Decimal_Number" | "digit" => Property::Number,
            "Z" | "Zs" | "Separator" | "Space_Separator" | "White_Space" | "space" => {
                Property::Whitespace
            }
            "P" | "Punctuation" | "punct" | "S" | "Symbol" | "Po" | "Other_Punctuation" => {
                Property::Punctuation
            }
            "ASCII" => Property::Ranges(&[(0, 0x7F)]),
            "M" | "Mark" | "Combining_Mark" | "Mn" | "Nonspacing_Mark" => Property::Ranges(MARKS),
            "Emoji" | "Extended_Pictographic" | "ExtPict" | "Emoji_Presentation" | "EPres" => {
                Property::Ranges(EMOJI)
            }
            "Cc" | "Control" | "cntrl" => Property::Ranges(&[(0, 0x1F), (0x7F, 0x9F)]),
            "ASCII_Hex_Digit" | "AHex" | "Hex_Digit" | "Hex" => {
                Property::Ranges(&[(0x30, 0x39), (0x41, 0x46), (0x61, 0x66)])
            }
            "ID_Continue" | "IDC" | "XID_Continue" | "XIDC" => Property::IdentifierPart,
            _ => Property::Any,
        };
        set.properties.push((property, negated));
    }

    fn escape(&mut self) -> Parsing<Node> {
        let code = self.next_code()?;
        let Some(character) = char::from_u32(code) else {
            return Ok(Node::Char(code));
        };
        match character {
            'b' => return Ok(Node::WordBoundary),
            'B' => return Ok(Node::NotWordBoundary),
            '1'..='9' => {
                self.at -= 1;
                let saved = self.at;
                let number = self.number().unwrap_or(0);
                if number <= self.total_groups {
                    return Ok(Node::BackRef(number));
                }
                if self.unicode {
                    return Err("a back reference to a group that isn't there".to_owned());
                }
                // Outside of unicode mode it's an old-style octal, or just the digit.
                self.at = saved + 1;
                if character >= '8' {
                    return Ok(Node::Char(code));
                }
                let mut value = character.to_digit(8).unwrap_or(0);
                while let Some(digit) = self
                    .peek()
                    .and_then(|unit| char::from_u32(u32::from(unit)))
                    .and_then(|c| c.to_digit(8))
                {
                    if value * 8 + digit > 255 {
                        break;
                    }
                    value = value * 8 + digit;
                    self.at += 1;
                }
                return Ok(Node::Char(value));
            }
            'k' if self.unicode || self.has_named_groups => {
                if !self.eat('<') {
                    return Err("expected a group's name after \\\\k".to_owned());
                }
                let name = self.group_name()?;
                return Ok(Node::NamedBackRef(name));
            }
            '0' if self.unicode && self.peek().is_some_and(|unit| (48..58).contains(&unit)) => {
                return Err("a bad escape".to_owned());
            }
            _ => {}
        }
        let mut set = ClassSet::default();
        if self.set_escape(character, &mut set) {
            if let Some(failure) = self.failure.take() {
                return Err(failure);
            }
            return Ok(Node::Class(Box::new(set)));
        }
        self.check_identity_escape(character)?;
        Ok(Node::Char(self.char_escape(character)))
    }

    /// In unicode mode, only some things can follow a backslash.
    fn check_identity_escape(&self, character: char) -> Parsing<()> {
        if self.unicode && !"^$\\\\.*+?()[]{}|/ntrfvcxu0".contains(character) {
            return Err(format!("a bad escape: \\\\{character}"));
        }
        Ok(())
    }

    fn class_atom(&mut self, set: &mut ClassSet) -> Parsing<Option<u32>> {
        let code = self.next_code()?;
        if code != '\\' as u32 {
            return Ok(Some(code));
        }
        let code = self.next_code()?;
        let Some(character) = char::from_u32(code) else {
            return Ok(Some(code));
        };
        if character == 'b' {
            return Ok(Some(8));
        }
        if character == '-' {
            return Ok(Some('-' as u32));
        }
        if self.set_escape(character, set) {
            if let Some(failure) = self.failure.take() {
                return Err(failure);
            }
            return Ok(None);
        }
        self.check_identity_escape(character)?;
        Ok(Some(self.char_escape(character)))
    }

    fn class(&mut self) -> Parsing<ClassSet> {
        let mut set = ClassSet {
            negated: self.eat('^'),
            ..ClassSet::default()
        };
        loop {
            if self.peek().is_none() {
                return Err("a character class isn't closed".to_owned());
            }
            if self.eat(']') {
                return Ok(set);
            }
            let Some(low) = self.class_atom(&mut set)? else {
                if self.unicode
                    && self.peek_is('-')
                    && self.units.get(self.at + 1).copied() != Some(']' as u16)
                {
                    return Err("a range can't start with a class".to_owned());
                }
                continue;
            };
            if self.peek_is('-') && self.units.get(self.at + 1).copied() != Some(']' as u16) {
                self.at += 1;
                let high = self.class_atom(&mut set)?;
                match high {
                    Some(high) if high >= low => set.ranges.push((low, high)),
                    Some(_) => return Err("a range in a class is out of order".to_owned()),
                    None if self.unicode => {
                        return Err("a range can't end with a class".to_owned());
                    }
                    None => {
                        set.ranges.push((low, low));
                        set.ranges.push(('-' as u32, '-' as u32));
                    }
                }
            } else {
                set.ranges.push((low, low));
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Inst {
    Char(u32),
    Any,
    Class(u32),
    /// Try the first place, and if that doesn't work out the second.
    Split(u32, u32),
    Jump(u32),
    Save(u32),
    Start,
    End,
    WordBoundary,
    NotWordBoundary,
    BackRef(u32),
    /// Changes flags for what's next, as a group with its own flags says. Each flag is 0 for off, 1 for on or 2 to stay.
    SetFlags {
        ignore_case: u8,
        multiline: u8,
        dot_all: u8,
    },
    /// Runs what's next by itself, up to a `LookEnd`, and then carries on from `next`.
    Look {
        next: u32,
        ahead: bool,
        negative: bool,
    },
    LookEnd,
    /// Notes where a loop's turn began, so a turn that matches nothing can be stopped.
    LoopStart(u32),
    LoopCheck(u32),
    /// Empties the capture groups in a range, as a new turn of a loop starts.
    ClearGroups(u32, u32),
    /// A greedy repeat of one character, any character or a class, as `\d+` or `[a-z]{2,8}` are: it takes as many as it
    /// can in one go, and backing off is one choice for all of them rather than one for each. `kind` is one of the
    /// `RUN_` numbers and says what `value` is. `max` is [`RUN_NO_LIMIT`] for no limit. It's only used going forward
    /// outside unicode mode, where each character is one unit.
    Run {
        kind: u8,
        min: u16,
        max: u16,
        value: u32,
    },
    Match,
}

/// What a [`Inst::Run`] repeats: a character (`value`), any character, or a class (`value` is which).
const RUN_CHAR: u8 = 0;
const RUN_ANY: u8 = 1;
const RUN_CLASS: u8 = 2;
const RUN_NO_LIMIT: u16 = u16::MAX;

#[derive(Debug)]
pub(crate) struct Program {
    code: Vec<Inst>,
    classes: Vec<ClassSet>,
    pub(crate) groups: u32,
    pub(crate) names: Vec<(String, u32)>,
    loops: u32,
    ignore_case: bool,
    multiline: bool,
    dot_all: bool,
    unicode: bool,
    /// The ASCII characters a match could start with, if it has to start with a character.
    first: Option<AsciiSet>,
    /// Where the [`Inst::Run`] is that every match starts with, if there is one with no upper limit, that has to match at
    /// least once, and nothing refers back to a group. Then a try that fails tells us the tries from inside that run
    /// would too, as they could only end the run in places that were already tried.
    lead_run: Option<usize>,
}

/// Lists a search needs, which are kept for the next one: a pair for each level of looking around, and room for saving
/// captures while looking.
#[derive(Default)]
pub(crate) struct Spare {
    lists: Vec<(Vec<Choice>, Vec<Undo>)>,
    saved: Vec<i32>,
    /// The programs of the last few patterns made, with their text and flags.
    recent: Vec<(Text, Text, Rc<Program>)>,
    /// Which of `recent` goes next, once it's full.
    next_recent: usize,
}

const RECENT_PROGRAMS: usize = 8;

/// Which ASCII characters a match of this program could start with. There's no answer if it could start with nothing,
/// or if working that out would take more than following the instructions.
fn first_characters(code: &[Inst], classes: &[ClassSet], ignore_case: bool) -> Option<AsciiSet> {
    if ignore_case {
        return None;
    }
    let mut set = [0u64; 2];
    let mut seen = vec![false; code.len()];
    let mut to_visit = vec![0usize];
    while let Some(pc) = to_visit.pop() {
        if std::mem::replace(&mut seen[pc], true) {
            continue;
        }
        match code[pc] {
            Inst::Char(wanted) => {
                if wanted < 128 {
                    set[(wanted >> 6) as usize] |= 1 << (wanted & 63);
                }
            }
            Inst::Class(index) => {
                let class = classes[index as usize].ascii[0];
                set[0] |= class[0];
                set[1] |= class[1];
            }
            Inst::Run {
                kind, min, value, ..
            } => {
                match kind {
                    RUN_CHAR if value < 128 => set[(value >> 6) as usize] |= 1 << (value & 63),
                    RUN_CHAR => {}
                    RUN_CLASS => {
                        let class = classes[value as usize].ascii[0];
                        set[0] |= class[0];
                        set[1] |= class[1];
                    }
                    _ => return None,
                }
                if min == 0 {
                    to_visit.push(pc + 1);
                }
            }
            Inst::Save(_) | Inst::LoopStart(_) | Inst::LoopCheck(_) | Inst::ClearGroups(..) => {
                to_visit.push(pc + 1);
            }
            Inst::Jump(target) => to_visit.push(target as usize),
            Inst::Split(first, second) => {
                to_visit.push(first as usize);
                to_visit.push(second as usize);
            }
            _ => return None,
        }
    }
    Some(set)
}

struct Emitter {
    code: Vec<Inst>,
    classes: Vec<ClassSet>,
    /// Which of `classes` each class in the pattern is, by where it is, so that one that's repeated, as `\d{0,256}` repeats
    /// `\d`, is only there once, rather than once for each time it's repeated.
    class_at: std::collections::HashMap<usize, u32>,
    names: Vec<(String, u32)>,
    loops: u32,
    /// Ignoring case, multiline and dot-all, as they stand where this is up to.
    flag_state: (bool, bool, bool),
    unicode: bool,
}

fn group_range(node: &Node) -> Option<(u32, u32)> {
    fn walk(node: &Node, low: &mut u32, high: &mut u32) {
        match node {
            Node::Group(inner, index) => {
                if let Some(index) = index {
                    *low = (*low).min(*index);
                    *high = (*high).max(*index);
                }
                walk(inner, low, high);
            }
            Node::Look { node, .. } | Node::Repeat { node, .. } | Node::Flags { node, .. } => {
                walk(node, low, high);
            }
            Node::Concat(items) | Node::Alternate(items) => {
                for item in items {
                    walk(item, low, high);
                }
            }
            _ => {}
        }
    }
    let (mut low, mut high) = (u32::MAX, 0);
    walk(node, &mut low, &mut high);
    (low != u32::MAX).then_some((low, high))
}

impl Emitter {
    /// Which of `classes` a class in the pattern is, adding it if it isn't there yet.
    fn class_index(&mut self, set: &ClassSet) -> u32 {
        let place = std::ptr::from_ref::<ClassSet>(set) as usize;
        match self.class_at.get(&place) {
            Some(index) => *index,
            None => {
                self.classes.push(set.clone());
                let index = self.classes.len() as u32 - 1;
                self.class_at.insert(place, index);
                index
            }
        }
    }

    fn emit(&mut self, node: &Node, backward: bool) -> Parsing<()> {
        if self.code.len() > MAX_INSTRUCTIONS {
            return Err("this regular expression is too big".to_owned());
        }
        match node {
            Node::Empty => {}
            Node::Char(code) => self.code.push(Inst::Char(*code)),
            Node::Any => self.code.push(Inst::Any),
            Node::Class(set) => {
                let index = self.class_index(set);
                self.code.push(Inst::Class(index));
            }
            Node::Start => self.code.push(Inst::Start),
            Node::End => self.code.push(Inst::End),
            Node::WordBoundary => self.code.push(Inst::WordBoundary),
            Node::NotWordBoundary => self.code.push(Inst::NotWordBoundary),
            Node::Flags {
                node,
                ignore_case,
                multiline,
                dot_all,
            } => {
                let as_code = |flag: Option<bool>| flag.map_or(2, u8::from);
                self.code.push(Inst::SetFlags {
                    ignore_case: as_code(*ignore_case),
                    multiline: as_code(*multiline),
                    dot_all: as_code(*dot_all),
                });
                let (saved_i, saved_m, saved_s) = self.flag_state;
                self.flag_state = (
                    ignore_case.unwrap_or(saved_i),
                    multiline.unwrap_or(saved_m),
                    dot_all.unwrap_or(saved_s),
                );
                self.emit(node, backward)?;
                self.flag_state = (saved_i, saved_m, saved_s);
                self.code.push(Inst::SetFlags {
                    ignore_case: u8::from(saved_i),
                    multiline: u8::from(saved_m),
                    dot_all: u8::from(saved_s),
                });
            }
            Node::Group(inner, index) => match index {
                Some(index) => {
                    let (open, close) = if backward {
                        (index * 2 + 1, index * 2)
                    } else {
                        (index * 2, index * 2 + 1)
                    };
                    self.code.push(Inst::Save(open));
                    self.emit(inner, backward)?;
                    self.code.push(Inst::Save(close));
                }
                None => self.emit(inner, backward)?,
            },
            Node::BackRef(index) => self.code.push(Inst::BackRef(*index)),
            Node::NamedBackRef(name) => {
                let Some((_, index)) = self.names.iter().find(|(found, _)| found == name) else {
                    return Err(format!("no group is called {name}"));
                };
                self.code.push(Inst::BackRef(*index));
            }
            Node::Look {
                node,
                ahead,
                negative,
            } => {
                let at = self.code.len();
                self.code.push(Inst::Look {
                    next: 0,
                    ahead: *ahead,
                    negative: *negative,
                });
                self.emit(node, !*ahead)?;
                self.code.push(Inst::LookEnd);
                let next = self.code.len() as u32;
                self.code[at] = Inst::Look {
                    next,
                    ahead: *ahead,
                    negative: *negative,
                };
            }
            Node::Concat(items) => {
                if backward {
                    for item in items.iter().rev() {
                        self.emit(item, backward)?;
                    }
                } else {
                    for item in items {
                        self.emit(item, backward)?;
                    }
                }
            }
            Node::Alternate(options) => {
                let mut ends = Vec::new();
                for (index, option) in options.iter().enumerate() {
                    if index + 1 < options.len() {
                        let split = self.code.len();
                        self.code.push(Inst::Split(0, 0));
                        self.emit(option, backward)?;
                        ends.push(self.code.len());
                        self.code.push(Inst::Jump(0));
                        let next = self.code.len() as u32;
                        self.code[split] = Inst::Split(split as u32 + 1, next);
                    } else {
                        self.emit(option, backward)?;
                    }
                }
                let end = self.code.len() as u32;
                for at in ends {
                    self.code[at] = Inst::Jump(end);
                }
            }
            Node::Repeat {
                node,
                min,
                max,
                greedy,
            } => self.repeat(node, *min, *max, *greedy, backward)?,
        }
        Ok(())
    }

    fn turn(&mut self, node: &Node, backward: bool) -> Parsing<()> {
        if let Some((low, high)) = group_range(node) {
            self.code.push(Inst::ClearGroups(low, high));
        }
        self.emit(node, backward)
    }

    fn repeat(
        &mut self,
        node: &Node,
        min: u32,
        max: u32,
        greedy: bool,
        backward: bool,
    ) -> Parsing<()> {
        if min > 1000 || (max != NO_LIMIT && max > 1000) {
            return Err("a count in this regular expression is too big".to_owned());
        }
        if greedy && !backward && !self.unicode {
            let atom = match node {
                Node::Char(code) if *code <= 0xFFFF => Some((RUN_CHAR, *code)),
                Node::Any => Some((RUN_ANY, 0)),
                Node::Class(set) => Some((RUN_CLASS, self.class_index(set))),
                _ => None,
            };
            if let Some((kind, value)) = atom {
                self.code.push(Inst::Run {
                    kind,
                    value,
                    min: min as u16,
                    max: if max == NO_LIMIT {
                        RUN_NO_LIMIT
                    } else {
                        max as u16
                    },
                });
                return Ok(());
            }
        }
        for _ in 0..min {
            self.turn(node, backward)?;
        }
        if max == NO_LIMIT {
            let slot = self.loops;
            self.loops += 1;
            let top = self.code.len();
            self.code.push(Inst::Split(0, 0));
            self.code.push(Inst::LoopStart(slot));
            self.turn(node, backward)?;
            self.code.push(Inst::LoopCheck(slot));
            self.code.push(Inst::Jump(top as u32));
            let end = self.code.len() as u32;
            let body = top as u32 + 1;
            self.code[top] = if greedy {
                Inst::Split(body, end)
            } else {
                Inst::Split(end, body)
            };
        } else {
            let mut splits = Vec::new();
            for _ in min..max {
                splits.push(self.code.len());
                self.code.push(Inst::Split(0, 0));
                self.turn(node, backward)?;
            }
            let end = self.code.len() as u32;
            for at in splits {
                let body = at as u32 + 1;
                self.code[at] = if greedy {
                    Inst::Split(body, end)
                } else {
                    Inst::Split(end, body)
                };
            }
        }
        Ok(())
    }
}

/// How many capturing groups a pattern has, and whether any have names.
fn count_groups(pattern: &[u16]) -> (u32, bool) {
    let (mut count, mut named, mut in_class) = (0, false, false);
    let mut at = 0;
    while at < pattern.len() {
        match char::from_u32(u32::from(pattern[at])) {
            Some('\\') => at += 1,
            Some('[') => in_class = true,
            Some(']') => in_class = false,
            Some('(') if !in_class => {
                let next = pattern.get(at + 1).copied();
                if next != Some('?' as u16) {
                    count += 1;
                } else if pattern.get(at + 2).copied() == Some('<' as u16)
                    && !matches!(
                        pattern
                            .get(at + 3)
                            .copied()
                            .and_then(|unit| char::from_u32(u32::from(unit))),
                        Some('=' | '!')
                    )
                {
                    count += 1;
                    named = true;
                }
            }
            _ => {}
        }
        at += 1;
    }
    (count, named)
}

pub(crate) fn compile(pattern: &[u16], flags: &str) -> Parsing<Program> {
    let unicode = flags.contains('u') || flags.contains('v');
    // How many groups there are matters to what comes before them, so it has to be known first.
    let (total_groups, has_named_groups) = count_groups(pattern);
    let mut parser = PatternParser {
        units: pattern,
        at: 0,
        groups: 0,
        names: Vec::new(),
        unicode,
        has_named_groups,
        total_groups,
        failure: None,
    };
    let node = parser.alternatives()?;
    if parser.at < pattern.len() {
        return Err("a group isn't opened".to_owned());
    }
    let mut emitter = Emitter {
        code: vec![Inst::Save(0)],
        classes: Vec::new(),
        class_at: std::collections::HashMap::new(),
        names: std::mem::take(&mut parser.names),
        loops: 0,
        flag_state: (
            flags.contains('i'),
            flags.contains('m'),
            flags.contains('s'),
        ),
        unicode,
    };
    emitter.emit(&node, false)?;
    emitter.code.push(Inst::Save(1));
    emitter.code.push(Inst::Match);
    for class in &mut emitter.classes {
        class.ascii = ascii_sets(class);
    }
    let first = first_characters(&emitter.code, &emitter.classes, flags.contains('i'));
    let lead_run = emitter
        .code
        .iter()
        .position(|inst| !matches!(inst, Inst::Save(_)))
        .filter(|at| {
            matches!(emitter.code[*at], Inst::Run { min, max: RUN_NO_LIMIT, .. } if min > 0)
                && !emitter
                    .code
                    .iter()
                    .any(|inst| matches!(inst, Inst::BackRef(_)))
        });
    Ok(Program {
        first,
        lead_run,
        code: emitter.code,
        classes: emitter.classes,
        groups: parser.groups,
        names: emitter.names,
        loops: emitter.loops,
        ignore_case: flags.contains('i'),
        multiline: flags.contains('m'),
        dot_all: flags.contains('s'),
        unicode,
    })
}

fn fold(code: u32) -> u32 {
    if code < 128 {
        return u32::from((code as u8).to_ascii_lowercase());
    }
    match char::from_u32(code) {
        Some(character) => {
            let mut lower = character.to_lowercase();
            match (lower.next(), lower.next()) {
                (Some(single), None) => single as u32,
                _ => code,
            }
        }
        None => code,
    }
}

/// Which of the first 128 characters are in a class, as it is and ignoring case, as [`class_has`] says, worked out from its
/// ranges rather than by asking about each character, as a class can have hundreds of them.
fn ascii_sets(class: &ClassSet) -> [AsciiSet; 2] {
    let mut raw: AsciiSet = [0; 2];
    for (low, high) in &class.ranges {
        for code in *low..=(*high).min(127) {
            raw[(code >> 6) as usize] |= 1 << (code & 63);
        }
    }
    if !class.properties.is_empty() {
        for code in 0..128 {
            if class
                .properties
                .iter()
                .any(|(property, negated)| property_has(*property, code) != *negated)
            {
                raw[(code >> 6) as usize] |= 1 << (code & 63);
            }
        }
    }
    let mut folded = raw;
    for code in 0..128u32 {
        let byte = code as u8;
        if ascii_has(&raw, u32::from(byte.to_ascii_lowercase()))
            || ascii_has(&raw, u32::from(byte.to_ascii_uppercase()))
        {
            folded[(code >> 6) as usize] |= 1 << (code & 63);
        }
    }
    if class.negated {
        for set in [&mut raw, &mut folded] {
            set[0] = !set[0];
            set[1] = !set[1];
        }
    }
    [raw, folded]
}

fn class_has(set: &ClassSet, code: u32, ignore_case: bool) -> bool {
    let check = |code: u32| {
        set.ranges
            .iter()
            .any(|(low, high)| *low <= code && code <= *high)
            || set
                .properties
                .iter()
                .any(|(property, negated)| property_has(*property, code) != *negated)
    };
    let mut found = check(code);
    if !found && ignore_case {
        found = check(fold(code));
        if !found {
            if let Some(character) = char::from_u32(code) {
                let mut upper = character.to_uppercase();
                if let (Some(single), None) = (upper.next(), upper.next()) {
                    found = check(single as u32);
                }
            }
        }
    }
    found != set.negated
}

#[inline(always)]
fn class_matches(set: &ClassSet, code: u32, ignore_case: bool) -> bool {
    if code < 128 {
        ascii_has(&set.ascii[usize::from(ignore_case)], code)
    } else {
        class_has(set, code, ignore_case)
    }
}

/// How many steps a match can take, so a bad pattern can't hang the program.
const MAX_STEPS: u64 = 50_000_000;

struct Choice {
    pc: u32,
    at: u32,
    /// How many undo notes there were when this was chosen.
    undo: u32,
    /// For backing off an [`Inst::Run`], the fewest it can take, as a place: every place from here up to `at` is still
    /// to be tried, the furthest first. It's [`ONE_PLACE`] for any other choice, which is just `at`.
    low: u32,
}

const ONE_PLACE: u32 = u32::MAX;

/// A saved value that was changed, for going back.
enum Undo {
    Capture(u32, i32),
    Loop(u32, i32),
}

struct Matcher<'a> {
    ignore_case: bool,
    multiline: bool,
    dot_all: bool,
    program: &'a Program,
    input: &'a [u16],
    captures: Vec<i32>,
    loops: Vec<i32>,
    steps: u64,
    /// Lists kept between tries, so trying each position, or looking around, doesn't need any new memory.
    spare: Spare,
}

impl Matcher<'_> {
    fn next_code(&self, at: usize, backward: bool) -> Option<(u32, usize)> {
        if backward {
            if at == 0 {
                return None;
            }
            let unit = u32::from(self.input[at - 1]);
            if self.program.unicode && (0xDC00..0xE000).contains(&unit) && at >= 2 {
                let high = u32::from(self.input[at - 2]);
                if (0xD800..0xDC00).contains(&high) {
                    return Some((0x10000 + ((high - 0xD800) << 10) + (unit - 0xDC00), at - 2));
                }
            }
            return Some((unit, at - 1));
        }
        let unit = u32::from(*self.input.get(at)?);
        if self.program.unicode && (0xD800..0xDC00).contains(&unit) {
            if let Some(low) = self.input.get(at + 1) {
                let low = u32::from(*low);
                if (0xDC00..0xE000).contains(&low) {
                    return Some((0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00), at + 2));
                }
            }
        }
        Some((unit, at + 1))
    }

    /// Where a run of what an [`Inst::Run`] repeats, starting here, stops: at the first unit that isn't one, or `limit`.
    #[inline]
    fn run_end(&self, kind: u8, value: u32, at: usize, limit: usize) -> usize {
        let units = &self.input[at..limit];
        let taken = match kind {
            RUN_CHAR => {
                let ignore_case = self.ignore_case;
                units
                    .iter()
                    .position(|unit| {
                        let unit = u32::from(*unit);
                        unit != value && !(ignore_case && fold(unit) == fold(value))
                    })
            }
            RUN_ANY => {
                if self.dot_all {
                    None
                } else {
                    units.iter().position(|unit| is_line_end(u32::from(*unit)))
                }
            }
            _ => {
                let class = &self.program.classes[value as usize];
                let ignore_case = self.ignore_case;
                units
                    .iter()
                    .position(|unit| !class_matches(class, u32::from(*unit), ignore_case))
            }
        };
        at + taken.unwrap_or(units.len())
    }

    /// The furthest place from `low` to `high` that what comes at `pc` could match at, looking only at its first
    /// character if that's all it starts with, so a run backs off straight to where that character is.
    #[inline]
    fn furthest_place(&self, pc: usize, low: usize, high: usize) -> Option<usize> {
        if low > high {
            return None;
        }
        let code = &self.program.code;
        let mut next = pc;
        while let Inst::Save(_) = code[next] {
            next += 1;
        }
        match code[next] {
            Inst::Char(wanted) if !self.ignore_case => (low..=high)
                .rev()
                .find(|place| self.input.get(*place).is_some_and(|unit| u32::from(*unit) == wanted)),
            Inst::Class(index) => {
                let class = &self.program.classes[index as usize];
                (low..=high).rev().find(|place| {
                    self.input
                        .get(*place)
                        .is_some_and(|unit| class_matches(class, u32::from(*unit), self.ignore_case))
                })
            }
            _ => Some(high),
        }
    }

    fn is_word_at(&self, at: usize) -> bool {
        self.input
            .get(at)
            .is_some_and(|unit| is_word(u32::from(*unit)))
    }

    /// Runs from `start_pc`. Gives where it got to, if it matched. `backward` is for looking behind.
    fn run(&mut self, start_pc: u32, start_at: usize, backward: bool) -> Result<Option<usize>, ()> {
        let (mut choices, mut undo) = self.spare.lists.pop().unwrap_or_default();
        choices.clear();
        undo.clear();
        let result = self.run_with(start_pc, start_at, backward, &mut choices, &mut undo);
        self.spare.lists.push((choices, undo));
        result
    }

    fn run_with(
        &mut self,
        start_pc: u32,
        start_at: usize,
        backward: bool,
        choices: &mut Vec<Choice>,
        undo: &mut Vec<Undo>,
    ) -> Result<Option<usize>, ()> {
        let mut pc = start_pc as usize;
        let mut at = start_at;
        let code = &self.program.code;
        loop {
            self.steps += 1;
            if self.steps > MAX_STEPS {
                return Err(());
            }
            let matched = match code[pc] {
                Inst::Char(wanted) => match self.next_code(at, backward) {
                    Some((found, next))
                        if found == wanted || (self.ignore_case && fold(found) == fold(wanted)) =>
                    {
                        at = next;
                        pc += 1;
                        true
                    }
                    _ => false,
                },
                Inst::Any => match self.next_code(at, backward) {
                    Some((found, next)) if self.dot_all || !is_line_end(found) => {
                        at = next;
                        pc += 1;
                        true
                    }
                    _ => false,
                },
                Inst::Class(index) => match self.next_code(at, backward) {
                    Some((found, next))
                        if class_matches(
                            &self.program.classes[index as usize],
                            found,
                            self.ignore_case,
                        ) =>
                    {
                        at = next;
                        pc += 1;
                        true
                    }
                    _ => false,
                },
                Inst::Split(first, second) => {
                    choices.push(Choice {
                        pc: second,
                        at: at as u32,
                        undo: undo.len() as u32,
                        low: ONE_PLACE,
                    });
                    pc = first as usize;
                    true
                }
                Inst::Run {
                    kind,
                    min,
                    max,
                    value,
                } => {
                    let limit = if max == RUN_NO_LIMIT {
                        self.input.len()
                    } else {
                        (at + usize::from(max)).min(self.input.len())
                    };
                    let end = self.run_end(kind, value, at, limit);
                    self.steps += (end - at) as u64;
                    let low = at + usize::from(min);
                    pc += 1;
                    match self.furthest_place(pc, low, end) {
                        Some(place) => {
                            if place > low {
                                choices.push(Choice {
                                    pc: pc as u32,
                                    at: place as u32 - 1,
                                    undo: undo.len() as u32,
                                    low: low as u32,
                                });
                            }
                            at = place;
                            true
                        }
                        None => false,
                    }
                }
                Inst::Jump(target) => {
                    pc = target as usize;
                    true
                }
                Inst::Save(slot) => {
                    let slot = slot as usize;
                    undo.push(Undo::Capture(slot as u32, self.captures[slot]));
                    self.captures[slot] = at as i32;
                    pc += 1;
                    true
                }
                Inst::Start => {
                    let good =
                        at == 0 || (self.multiline && is_line_end(u32::from(self.input[at - 1])));
                    pc += 1;
                    good
                }
                Inst::End => {
                    let good = at == self.input.len()
                        || (self.multiline && is_line_end(u32::from(self.input[at])));
                    pc += 1;
                    good
                }
                Inst::WordBoundary | Inst::NotWordBoundary => {
                    let before = at > 0 && self.is_word_at(at - 1);
                    let boundary = before != self.is_word_at(at);
                    let wanted = matches!(code[pc], Inst::WordBoundary);
                    pc += 1;
                    boundary == wanted
                }
                Inst::BackRef(index) => {
                    let slot = index as usize * 2;
                    let (from, to) = (
                        self.captures.get(slot).copied().unwrap_or(-1),
                        self.captures.get(slot + 1).copied().unwrap_or(-1),
                    );
                    pc += 1;
                    if from < 0 || to < 0 {
                        true
                    } else {
                        let length = (to - from) as usize;
                        let piece = from as usize;
                        let fits = if backward {
                            at >= length
                        } else {
                            at + length <= self.input.len()
                        };
                        if fits {
                            let where_ = if backward { at - length } else { at };
                            let same = (0..length).all(|offset| {
                                let (x, y) = (
                                    u32::from(self.input[piece + offset]),
                                    u32::from(self.input[where_ + offset]),
                                );
                                x == y || (self.ignore_case && fold(x) == fold(y))
                            });
                            if same {
                                at = if backward { at - length } else { at + length };
                            }
                            same
                        } else {
                            false
                        }
                    }
                }
                Inst::Look {
                    next,
                    ahead,
                    negative,
                } => {
                    let mark = self.spare.saved.len();
                    self.spare.saved.extend_from_slice(&self.captures);
                    let result = self.run(pc as u32 + 1, at, !ahead)?;
                    pc = next as usize;
                    let saved = &self.spare.saved[mark..];
                    let good = if negative || result.is_none() {
                        self.captures.copy_from_slice(saved);
                        negative == result.is_none()
                    } else {
                        // What the look found stays, so it has to be possible to take it back.
                        for (slot, value) in saved.iter().enumerate() {
                            if self.captures[slot] != *value {
                                undo.push(Undo::Capture(slot as u32, *value));
                            }
                        }
                        true
                    };
                    self.spare.saved.truncate(mark);
                    good
                }
                Inst::SetFlags {
                    ignore_case,
                    multiline,
                    dot_all,
                } => {
                    if ignore_case != 2 {
                        self.ignore_case = ignore_case == 1;
                    }
                    if multiline != 2 {
                        self.multiline = multiline == 1;
                    }
                    if dot_all != 2 {
                        self.dot_all = dot_all == 1;
                    }
                    pc += 1;
                    true
                }
                Inst::LookEnd | Inst::Match => return Ok(Some(at)),
                Inst::LoopStart(slot) => {
                    undo.push(Undo::Loop(slot, self.loops[slot as usize]));
                    self.loops[slot as usize] = at as i32;
                    pc += 1;
                    true
                }
                Inst::LoopCheck(slot) => {
                    pc += 1;
                    // A turn of a loop that matched nothing would go on for ever.
                    self.loops[slot as usize] != at as i32
                }
                Inst::ClearGroups(low, high) => {
                    for index in low..=high {
                        for slot in [index * 2, index * 2 + 1] {
                            if self.captures[slot as usize] != -1 {
                                undo.push(Undo::Capture(slot, self.captures[slot as usize]));
                                self.captures[slot as usize] = -1;
                            }
                        }
                    }
                    pc += 1;
                    true
                }
            };
            if !matched {
                let (choice_pc, choice_undo, place) = loop {
                    let Some(choice) = choices.last_mut() else {
                        return Ok(None);
                    };
                    let (choice_pc, choice_undo) = (choice.pc, choice.undo);
                    if choice.low == ONE_PLACE {
                        let place = choice.at;
                        choices.pop();
                        break (choice_pc, choice_undo, place as usize);
                    }
                    // Backing off a run: one fewer, or as many fewer as it takes to get to a place worth trying.
                    let (low, high) = (choice.low as usize, choice.at as usize);
                    self.steps += 1;
                    match self.furthest_place(choice_pc as usize, low, high) {
                        Some(place) => {
                            if place > low {
                                choice.at = place as u32 - 1;
                            } else {
                                choices.pop();
                            }
                            break (choice_pc, choice_undo, place);
                        }
                        None => {
                            choices.pop();
                        }
                    }
                };
                while undo.len() > choice_undo as usize {
                    match undo.pop() {
                        Some(Undo::Capture(slot, value)) => self.captures[slot as usize] = value,
                        Some(Undo::Loop(slot, value)) => self.loops[slot as usize] = value,
                        None => break,
                    }
                }
                pc = choice_pc as usize;
                at = place;
            }
        }
    }
}

fn new_matcher<'a>(program: &'a Program, input: &'a [u16]) -> Matcher<'a> {
    Matcher {
        ignore_case: program.ignore_case,
        multiline: program.multiline,
        dot_all: program.dot_all,
        program,
        input,
        captures: vec![-1; (program.groups as usize + 1) * 2],
        loops: vec![-1; program.loops as usize],
        steps: 0,
        spare: Spare::default(),
    }
}

impl Matcher<'_> {
    /// Tries to match at exactly this position. The captures are pairs of positions, with -1 for no match.
    fn try_at(&mut self, at: usize) -> Result<bool, ()> {
        self.spare.saved.clear();
        self.captures.fill(-1);
        self.loops.fill(-1);
        self.reset_flags();
        Ok(self.run(0, at, false)?.is_some())
    }

    fn reset_flags(&mut self) {
        self.ignore_case = self.program.ignore_case;
        self.multiline = self.program.multiline;
        self.dot_all = self.program.dot_all;
    }

    /// Searches from a position on. If it finds a match, its captures are in `captures`. The limit on steps is for
    /// each search.
    fn search(&mut self, from: usize, sticky: bool) -> Result<bool, ()> {
        self.steps = 0;
        let length = self.input.len();
        let mut at = from;
        loop {
            if at > length {
                return Ok(false);
            }
            if let Some(first) = &self.program.first {
                // Most places can't be the start of a match, which only needs one character looking at.
                let skipped = self.input[at..]
                    .iter()
                    .position(|unit| *unit >= 128 || ascii_has(first, u32::from(*unit)));
                match skipped {
                    Some(0) => {}
                    Some(_) if sticky => return Ok(false),
                    Some(skipped) => at += skipped,
                    None => return Ok(false),
                }
            }
            if self.try_at(at)? {
                return Ok(true);
            }
            if sticky {
                return Ok(false);
            }
            at = self.after_failure(at);
        }
    }

    /// Where to try next after a try here failed. With a run that every match starts with, no place inside the run
    /// that starts here can do any better, so the next place worth trying is after it.
    fn after_failure(&mut self, at: usize) -> usize {
        let Some(pc) = self.program.lead_run else {
            return at + 1;
        };
        let Inst::Run { kind, value, .. } = self.program.code[pc] else {
            return at + 1;
        };
        self.reset_flags();
        self.run_end(kind, value, at, self.input.len()) + 1
    }
}

#[cfg(test)]
fn match_at(
    program: &Program,
    input: &[u16],
    at: usize,
    steps: &mut u64,
) -> Result<Option<Vec<i32>>, ()> {
    let mut matcher = new_matcher(program, input);
    matcher.steps = *steps;
    let found = matcher.try_at(at);
    *steps = matcher.steps;
    Ok(found?.then_some(matcher.captures))
}

/// The last match of a regular expression that doesn't go by `lastIndex` (neither global nor sticky), whose answer
/// only depends on its pattern and the string: the same string matched again gives the same captures, and the same
/// strings for them. Library code does that over and over (chart.js matches `'' + lineHeight` for every tick's font,
/// and a number turned into a string is the same string each time, see `Vm::number_string`). A collection forgets
/// it, as the strings it names may be freed.
#[derive(Default)]
pub(crate) struct LastMatches {
    /// A few, as code often matches two or three patterns in turn (chart.js's font style, then its line height).
    entries: [LastMatch; 4],
    /// Which one is replaced next.
    next: usize,
}

#[derive(Default)]
struct LastMatch {
    program: Option<Rc<Program>>,
    input: u32,
    found: Option<Vec<i32>>,
    /// The strings the array `exec` gives had for the captures, once one's been made.
    pieces: Vec<Value>,
}

pub(crate) struct RegExpData {
    pub(crate) source: Value,
    pub(crate) flags: Value,
    pub(crate) program: Rc<Program>,
    pub(crate) global: bool,
    pub(crate) sticky: bool,
    pub(crate) unicode: bool,
}

impl Vm {
    pub(crate) fn new_regexp(&mut self, pattern: Value, flags: Value, proto: u32) -> Run<Value> {
        let flags = self.to_js_string(flags)?;
        let (mut global, mut sticky, mut unicode, mut seen) = (false, false, false, 0u8);
        let flag_bytes = self.text_of(flags).ascii().unwrap_or(b"?");
        for flag in flag_bytes {
            let Some(bit) = b"dgimsuvy".iter().position(|known| known == flag) else {
                seen = u8::MAX;
                break;
            };
            if seen & 1 << bit != 0 {
                seen = u8::MAX;
                break;
            }
            seen |= 1 << bit;
            global |= *flag == b'g';
            sticky |= *flag == b'y';
            unicode |= matches!(*flag, b'u' | b'v');
        }
        if seen == u8::MAX {
            let flag_text = self.text_of(flags).to_rust();
            return Err(self.syntax_error(&format!(
                "Invalid flags supplied to RegExp constructor '{flag_text}'"
            )));
        }
        let source = self.to_js_string(pattern)?;
        // A pattern written in a function is made again each time it runs, so the last few are found by their text with
        // nothing made for looking.
        let recent = self.regexp_spare.recent.iter().find(|(known_source, known_flags, _)| {
            known_source == self.text_of(source) && known_flags == self.text_of(flags)
        });
        let program = match recent {
            Some((_, _, program)) => program.clone(),
            None => {
                let program = self.find_program(source, flags)?;
                let spare = &mut self.regexp_spare;
                let entry = (
                    self.strings.items[source.index()].text.clone(),
                    self.strings.items[flags.index()].text.clone(),
                    program.clone(),
                );
                if spare.recent.len() < RECENT_PROGRAMS {
                    spare.recent.push(entry);
                } else {
                    spare.recent[spare.next_recent] = entry;
                    spare.next_recent = (spare.next_recent + 1) % RECENT_PROGRAMS;
                }
                program
            }
        };
        let object = self.new_object_with_proto(
            proto,
            Kind::RegExp(Box::new(RegExpData {
                source,
                flags,
                program,
                global,
                sticky,
                unicode,
            })),
        );
        self.define(object.handle(), text::LAST_INDEX, Value::int(0), WRITABLE);
        Ok(object)
    }

    /// The program for a pattern and its flags, from those already worked out or worked out now.
    fn find_program(&mut self, source: Value, flags: Value) -> Run<Rc<Program>> {
        let units = self.text_of(source).units();
        let key = (units, self.text_of(flags).to_rust());
        if let Some(program) = self.regex_programs.get(&key) {
            return Ok(program.clone());
        }
        let (units, flag_text) = &key;
        let program = match compile(units, flag_text) {
            Ok(program) => Rc::new(program),
            Err(message) => {
                let pattern_text = String::from_utf16_lossy(units);
                return Err(self.syntax_error(&format!(
                    "Invalid regular expression: /{pattern_text}/{flag_text}: {message}"
                )));
            }
        };
        // Only so many are kept, for a program that makes patterns from what it's given.
        if self.regex_programs.len() >= 512 {
            self.regex_programs.clear();
        }
        self.regex_programs.insert(key, program.clone());
        Ok(program)
    }

    /// Searches from a position on, and gives the captures of the first match.
    pub(crate) fn regexp_search(
        &mut self,
        program: &Program,
        input: &[u16],
        from: usize,
        sticky: bool,
    ) -> Run<Option<Vec<i32>>> {
        let mut matcher = new_matcher(program, input);
        matcher.spare = std::mem::take(&mut self.regexp_spare);
        let found = matcher.search(from, sticky);
        self.regexp_spare = std::mem::take(&mut matcher.spare);
        match found {
            Ok(true) => Ok(Some(matcher.captures)),
            Ok(false) => Ok(None),
            Err(()) => Err(self.range_error("This regular expression took too long")),
        }
    }

    fn regexp_parts(&mut self, value: Value) -> Run<(Rc<Program>, bool, bool, bool)> {
        if value.is_object() {
            if let Kind::RegExp(data) = &self.objects[value.index()].kind {
                return Ok((data.program.clone(), data.global, data.sticky, data.unicode));
            }
        }
        Err(self.type_error("This isn't a regular expression"))
    }

    pub(crate) fn is_regexp(&self, value: Value) -> bool {
        value.is_object() && matches!(self.objects[value.index()].kind, Kind::RegExp(_))
    }

    /// Runs `exec`: it uses and updates `lastIndex`, as a global regular expression has to.
    pub(crate) fn regexp_exec_raw(
        &mut self,
        regexp: Value,
        string: Value,
    ) -> Run<Option<Vec<i32>>> {
        let plain = match regexp.is_object().then(|| &self.objects[regexp.index()].kind) {
            Some(Kind::RegExp(data)) if !data.global && !data.sticky => Some(data.program.clone()),
            _ => None,
        };
        if let Some(program) = &plain
            && let Some(last) = self.last_match.entries.iter().find(|last| {
                last.input == string.handle()
                    && last.program.as_ref().is_some_and(|known| Rc::ptr_eq(known, program))
            })
        {
            return Ok(last.found.clone());
        }
        let found = self.regexp_exec_searching(regexp, string)?;
        if let Some(program) = plain {
            let last = &mut self.last_match;
            last.entries[last.next] = LastMatch {
                program: Some(program),
                input: string.handle(),
                found: found.clone(),
                pieces: Vec::new(),
            };
            last.next = (last.next + 1) % last.entries.len();
        }
        Ok(found)
    }

    fn regexp_exec_searching(&mut self, regexp: Value, string: Value) -> Run<Option<Vec<i32>>> {
        // Going through a long string one match at a time shouldn't mean copying it each time.
        let input = if self.regexp_input.0 == string.handle() {
            self.regexp_input.0 = NONE;
            std::mem::take(&mut self.regexp_input.1)
        } else {
            let mut input = std::mem::take(&mut self.regexp_input.1);
            self.regexp_input.0 = NONE;
            input.clear();
            self.text_of(string).push_to(&mut input);
            input
        };
        let result = self.regexp_exec_on(regexp, &input);
        self.regexp_input = (string.handle(), input);
        result
    }

    fn regexp_exec_on(&mut self, regexp: Value, input: &[u16]) -> Run<Option<Vec<i32>>> {
        let (program, global, sticky, _) = self.regexp_parts(regexp)?;
        let uses_last = global || sticky;
        let from = if uses_last {
            let last = self.get(regexp, text::LAST_INDEX)?;
            let number = self.to_integer(last)?;
            if number < 0.0 { 0 } else { number as usize }
        } else {
            0
        };
        if from > input.len() {
            self.set(regexp, text::LAST_INDEX, Value::int(0))?;
            return Ok(None);
        }
        let found = self.regexp_search(&program, input, from, sticky)?;
        if uses_last {
            let last = match &found {
                Some(captures) => Value::int(captures[1]),
                None => Value::int(0),
            };
            self.set(regexp, text::LAST_INDEX, last)?;
        }
        Ok(found)
    }

    /// The array `exec` gives for a match.
    pub(crate) fn match_array(
        &mut self,
        regexp: Value,
        string: Value,
        captures: &[i32],
    ) -> Run<Value> {
        let (program, ..) = self.regexp_parts(regexp)?;
        let same = self.last_match.entries.iter().position(|last| {
            last.program.is_some() && last.input == string.handle() && last.found.as_deref() == Some(captures)
        });
        let items = if let Some(at) = same
            && !self.last_match.entries[at].pieces.is_empty()
        {
            // Strings don't change, and these haven't been freed: a collection would have forgotten them.
            self.last_match.entries[at].pieces.clone()
        } else {
            let mut items = Vec::with_capacity(captures.len() / 2);
            for pair in captures.chunks(2) {
                items.push(if pair[0] < 0 || pair[1] < 0 {
                    Value::UNDEFINED
                } else {
                    let piece = self
                        .text_of(string)
                        .slice(pair[0] as usize, pair[1] as usize);
                    self.new_text(piece)
                });
            }
            if let Some(at) = same {
                self.last_match.entries[at].pieces = items.clone();
            }
            items
        };
        let groups = if program.names.is_empty() {
            Value::UNDEFINED
        } else {
            let groups = self.new_plain();
            for (name, index) in &program.names {
                let atom = self.intern_str(name);
                self.define(groups.handle(), atom, items[*index as usize], PLAIN);
            }
            groups
        };
        let array = self.new_array(items);
        let index = Value::int(captures[0]);
        if self.match_shape != NONE {
            let made = &mut self.objects[array.index()];
            made.set_slots(&[index, string, groups]);
            made.shape = self.match_shape;
            return Ok(array);
        }
        self.define(array.handle(), text::INDEX, index, PLAIN);
        self.define(array.handle(), text::INPUT, string, PLAIN);
        self.define(array.handle(), text::GROUPS, groups, PLAIN);
        self.match_shape = self.objects[array.index()].shape;
        Ok(array)
    }

    pub(crate) fn regexp_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
        let pattern = vm.arg(&args, 0);
        let flags = vm.arg(&args, 1);
        let proto = vm.proto_for(&args, vm.realm.regexp_proto)?;
        let (pattern, flags) = if vm.is_regexp(pattern) {
            let Kind::RegExp(data) = &vm.objects[pattern.index()].kind else {
                unreachable!("just checked");
            };
            let source = data.source;
            let old_flags = data.flags;
            (
                source,
                if flags.is_undefined() {
                    old_flags
                } else {
                    flags
                },
            )
        } else {
            let pattern = if pattern.is_undefined() {
                vm.new_str("(?:)")
            } else {
                vm.to_js_string(pattern)?
            };
            let flags = if flags.is_undefined() {
                vm.new_str("")
            } else {
                flags
            };
            (pattern, flags)
        };
        vm.new_regexp(pattern, flags, proto)
    }

    pub(crate) fn regexp_exec(vm: &mut Vm, args: Args) -> Run<Value> {
        let regexp = args.this;
        let string = vm.arg(&args, 0);
        let string = vm.to_js_string(string)?;
        match vm.regexp_exec_raw(regexp, string)? {
            Some(captures) => vm.match_array(regexp, string, &captures),
            None => Ok(Value::NULL),
        }
    }

    pub(crate) fn regexp_test(vm: &mut Vm, args: Args) -> Run<Value> {
        let regexp = args.this;
        let string = vm.arg(&args, 0);
        let string = vm.to_js_string(string)?;
        Ok(Value::bool(vm.regexp_exec_raw(regexp, string)?.is_some()))
    }

    pub(crate) fn regexp_to_string(vm: &mut Vm, args: Args) -> Run<Value> {
        let this = args.this;
        let source = vm.get_named(this, "source")?;
        let flags = vm.get_named(this, "flags")?;
        let source = vm.to_rust_string(source)?;
        let flags = vm.to_rust_string(flags)?;
        Ok(vm.new_str(&format!("/{source}/{flags}")))
    }

    pub(crate) fn regexp_source(vm: &mut Vm, args: Args) -> Run<Value> {
        if !vm.is_regexp(args.this) {
            if args.this == Value::object(vm.realm.regexp_proto) {
                return Ok(vm.new_str("(?:)"));
            }
            return Err(vm.type_error("This isn't a regular expression"));
        }
        let Kind::RegExp(data) = &vm.objects[args.this.index()].kind else {
            unreachable!("just checked");
        };
        let source = data.source;
        if vm.text_of(source).len() == 0 {
            return Ok(vm.new_str("(?:)"));
        }
        Ok(source)
    }

    pub(crate) fn regexp_flags(vm: &mut Vm, args: Args) -> Run<Value> {
        if !vm.is_regexp(args.this) {
            return Ok(vm.new_str(""));
        }
        let Kind::RegExp(data) = &vm.objects[args.this.index()].kind else {
            unreachable!("just checked");
        };
        Ok(data.flags)
    }

    fn regexp_has_flag(vm: &mut Vm, args: Args, flag: char) -> Run<Value> {
        if !vm.is_regexp(args.this) {
            if args.this == Value::object(vm.realm.regexp_proto) {
                return Ok(Value::UNDEFINED);
            }
            return Err(vm.type_error("This isn't a regular expression"));
        }
        let Kind::RegExp(data) = &vm.objects[args.this.index()].kind else {
            unreachable!("just checked");
        };
        let flags = data.flags;
        Ok(Value::bool(vm.text_of(flags).to_rust().contains(flag)))
    }

    pub(crate) fn regexp_global(vm: &mut Vm, args: Args) -> Run<Value> {
        Self::regexp_has_flag(vm, args, 'g')
    }

    pub(crate) fn regexp_ignore_case(vm: &mut Vm, args: Args) -> Run<Value> {
        Self::regexp_has_flag(vm, args, 'i')
    }

    pub(crate) fn regexp_multiline(vm: &mut Vm, args: Args) -> Run<Value> {
        Self::regexp_has_flag(vm, args, 'm')
    }

    pub(crate) fn regexp_sticky(vm: &mut Vm, args: Args) -> Run<Value> {
        Self::regexp_has_flag(vm, args, 'y')
    }

    pub(crate) fn regexp_unicode(vm: &mut Vm, args: Args) -> Run<Value> {
        Self::regexp_has_flag(vm, args, 'u')
    }

    pub(crate) fn regexp_dot_all(vm: &mut Vm, args: Args) -> Run<Value> {
        Self::regexp_has_flag(vm, args, 's')
    }

    // What strings do with them.

    /// All the matches of a global regular expression, or the first of any other.
    pub(crate) fn regexp_all(&mut self, regexp: Value, string: Value) -> Run<Vec<Vec<i32>>> {
        self.regexp_all_from(regexp, string, 0)
    }

    /// The same, starting from a position for a global one.
    pub(crate) fn regexp_all_from(
        &mut self,
        regexp: Value,
        string: Value,
        start: usize,
    ) -> Run<Vec<Vec<i32>>> {
        let (program, global, sticky, unicode) = self.regexp_parts(regexp)?;
        let input = self.text_of(string).units();
        let mut out = Vec::new();
        if !global {
            if let Some(captures) = self.regexp_exec_raw(regexp, string)? {
                out.push(captures);
            }
            return Ok(out);
        }
        // One matcher does for every match.
        let mut matcher = new_matcher(&program, &input);
        matcher.spare = std::mem::take(&mut self.regexp_spare);
        let mut from = start;
        while from <= input.len() {
            match matcher.search(from, sticky) {
                Ok(true) => {}
                Ok(false) => break,
                Err(()) => {
                    self.regexp_spare = std::mem::take(&mut matcher.spare);
                    return Err(self.range_error("This regular expression took too long"));
                }
            }
            let captures = matcher.captures.clone();
            let (start, end) = (captures[0] as usize, captures[1] as usize);
            from = if end == start {
                let double = unicode
                    && input
                        .get(end)
                        .is_some_and(|unit| (0xD800..0xDC00).contains(unit))
                    && input
                        .get(end + 1)
                        .is_some_and(|unit| (0xDC00..0xE000).contains(unit));
                end + if double { 2 } else { 1 }
            } else {
                end
            };
            out.push(captures);
        }
        self.regexp_spare = std::mem::take(&mut matcher.spare);
        self.set(regexp, text::LAST_INDEX, Value::int(0))?;
        Ok(out)
    }

    pub(crate) fn regexp_names(&mut self, regexp: Value) -> Run<Vec<(String, u32)>> {
        let (program, ..) = self.regexp_parts(regexp)?;
        Ok(program.names.clone())
    }

    /// Splits around a regular expression's matches.
    pub(crate) fn regexp_split(&mut self, regexp: Value, string: Value, limit: u32) -> Run<Value> {
        let (program, ..) = self.regexp_parts(regexp)?;
        let units = self.text_of(string).units();
        if limit == 0 {
            return Ok(self.new_array(Vec::new()));
        }
        if units.is_empty() {
            let mut out = Vec::new();
            if self.regexp_search(&program, &units, 0, true)?.is_none() {
                out.push(string);
            }
            return Ok(self.new_array(out));
        }
        // Where the pieces are, as pairs of places, or -1 for a group that matched nothing. Finding them all first
        // needs only one matcher, and nothing runs while they're made into strings after.
        let mut pieces: Vec<(i32, i32)> = Vec::new();
        let mut matcher = new_matcher(&program, &units);
        matcher.spare = std::mem::take(&mut self.regexp_spare);
        let mut start = 0;
        let mut at = 0;
        // The language tries a sticky match at each place in turn. Searching from a place finds the first of those
        // that would match.
        let finished = loop {
            if at >= units.len() {
                break true;
            }
            match matcher.search(at, false) {
                Ok(true) => {}
                Ok(false) => break true,
                Err(()) => break false,
            }
            let captures = &matcher.captures;
            let (found, end) = (captures[0] as usize, captures[1] as usize);
            if found >= units.len() {
                break true;
            }
            if end == start {
                at = found + 1;
                continue;
            }
            pieces.push((start as i32, found as i32));
            if pieces.len() as u32 >= limit {
                break true;
            }
            let mut full = false;
            for pair in captures[2..].chunks(2) {
                pieces.push(if pair[0] < 0 || pair[1] < 0 {
                    (-1, -1)
                } else {
                    (pair[0], pair[1])
                });
                if pieces.len() as u32 >= limit {
                    full = true;
                    break;
                }
            }
            if full {
                break true;
            }
            start = end;
            at = if end == found { found + 1 } else { end };
        };
        self.regexp_spare = std::mem::take(&mut matcher.spare);
        if !finished {
            return Err(self.range_error("This regular expression took too long"));
        }
        if (pieces.len() as u32) < limit {
            pieces.push((start as i32, units.len() as i32));
        }
        let mut out = Vec::with_capacity(pieces.len());
        for (from, to) in pieces {
            out.push(if from < 0 {
                Value::UNDEFINED
            } else {
                let piece = self.text_of(string).slice(from as usize, to as usize);
                self.new_text(piece)
            });
        }
        Ok(self.new_array(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(pattern: &str, flags: &str, input: &str) -> Option<Vec<Option<String>>> {
        let units: Vec<u16> = pattern.encode_utf16().collect();
        let program = compile(&units, flags).unwrap();
        let input: Vec<u16> = input.encode_utf16().collect();
        let mut steps = 0;
        for at in 0..=input.len() {
            if let Some(captures) = match_at(&program, &input, at, &mut steps).unwrap() {
                return Some(
                    captures
                        .chunks(2)
                        .map(|pair| {
                            (pair[0] >= 0 && pair[1] >= 0).then(|| {
                                String::from_utf16_lossy(&input[pair[0] as usize..pair[1] as usize])
                            })
                        })
                        .collect(),
                );
            }
        }
        None
    }

    fn whole(pattern: &str, flags: &str, input: &str) -> Option<String> {
        find(pattern, flags, input).and_then(|mut captures| captures.swap_remove(0))
    }

    #[test]
    fn literal_text_and_classes_match() {
        assert_eq!(whole("abc", "", "xxabcxx").as_deref(), Some("abc"));
        assert_eq!(whole("[a-c]+", "", "xxabcabd").as_deref(), Some("abcab"));
        assert_eq!(whole("[^a-c]+", "", "abxyzc").as_deref(), Some("xyz"));
        assert_eq!(whole(r"\d+", "", "ab123c").as_deref(), Some("123"));
        assert_eq!(whole(r"\w+", "", "  foo_1 ").as_deref(), Some("foo_1"));
        assert_eq!(whole(r"\S+", "", "  foo ").as_deref(), Some("foo"));
        assert_eq!(whole("a.c", "", "a\nc"), None);
        assert_eq!(whole("a.c", "s", "a\nc").as_deref(), Some("a\nc"));
    }

    #[test]
    fn a_class_repeated_many_times_is_only_kept_once() {
        let units: Vec<u16> = r"^[a-zA-Z0-9-]{1,250}\.\d{0,256}$".encode_utf16().collect();
        let program = compile(&units, "").unwrap();
        // Each of the two classes is there once, however many times it's repeated.
        assert_eq!(program.classes.len(), 2);
        assert_eq!(
            whole(r"^[a-zA-Z0-9-]{1,250}\.\d{0,256}$", "", "build-7.123").as_deref(),
            Some("build-7.123")
        );
        assert_eq!(
            whole(r"^[a-zA-Z0-9-]{1,250}\.\d{0,256}$", "", "build_7.123"),
            None
        );
    }

    #[test]
    fn what_a_class_has_of_the_first_characters_is_what_it_says_it_has() {
        for (pattern, flags) in [
            (r"[a-fK-Z_0-9]", ""),
            (r"[a-fK-Z_0-9]", "i"),
            (r"[^a-fK-Z_0-9]", "i"),
            (r"[^\s\d]", ""),
            (r"[\w$]", "i"),
            (r"[\p{L}\P{Lu}]", "u"),
            (r"[\p{Lu}]", "iu"),
            (r"[^\p{Ll}x]", "iu"),
        ] {
            let units: Vec<u16> = pattern.encode_utf16().collect();
            let program = compile(&units, flags).unwrap();
            for class in &program.classes {
                for code in 0..128 {
                    for ignore_case in [false, true] {
                        assert_eq!(
                            ascii_has(&class.ascii[usize::from(ignore_case)], code),
                            class_has(class, code, ignore_case),
                            "{pattern} {flags} {code} {ignore_case}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn quantifiers_are_greedy_unless_told_not_to_be() {
        assert_eq!(whole("a+", "", "aaa").as_deref(), Some("aaa"));
        assert_eq!(whole("a+?", "", "aaa").as_deref(), Some("a"));
        assert_eq!(whole("a{2}", "", "aaa").as_deref(), Some("aa"));
        assert_eq!(whole("a{2,}", "", "aaaa").as_deref(), Some("aaaa"));
        assert_eq!(whole("a{1,2}?", "", "aaa").as_deref(), Some("a"));
        assert_eq!(whole("<.*>", "", "<a><b>").as_deref(), Some("<a><b>"));
        assert_eq!(whole("<.*?>", "", "<a><b>").as_deref(), Some("<a>"));
    }

    #[test]
    fn groups_capture_and_can_be_referred_back_to() {
        let captures = find(r"(a)(b)?(c)?", "", "ac").unwrap();
        assert_eq!(captures[1].as_deref(), Some("a"));
        assert_eq!(captures[2], None);
        assert_eq!(captures[3].as_deref(), Some("c"));
        assert_eq!(whole(r"(a+)b\1", "", "aabaa").as_deref(), Some("aabaa"));
        assert_eq!(whole(r"(?<x>a)\k<x>", "", "aa").as_deref(), Some("aa"));
        assert_eq!(whole("(?:ab)+", "", "ababx").as_deref(), Some("abab"));
    }

    #[test]
    fn a_new_turn_of_a_loop_forgets_its_groups() {
        let captures = find(r"(?:(a)|b)+", "", "ab").unwrap();
        assert_eq!(captures[1], None);
    }

    #[test]
    fn alternatives_are_tried_in_order() {
        assert_eq!(whole("a|ab", "", "ab").as_deref(), Some("a"));
        assert_eq!(whole("(?:a|ab)c", "", "abc").as_deref(), Some("abc"));
    }

    #[test]
    fn anchors_and_boundaries_hold_their_place() {
        assert_eq!(whole("^b", "", "ab"), None);
        assert_eq!(whole("^b", "m", "a\nb").as_deref(), Some("b"));
        assert_eq!(whole("a$", "", "ab"), None);
        assert_eq!(whole("a$", "m", "a\nb").as_deref(), Some("a"));
        assert_eq!(whole(r"\bfoo\b", "", "a foo b").as_deref(), Some("foo"));
        assert_eq!(whole(r"\bfoo\b", "", "afoo"), None);
        assert_eq!(whole(r"\Boo", "", "foo").as_deref(), Some("oo"));
    }

    #[test]
    fn looking_around_matches_nothing_itself() {
        assert_eq!(whole("a(?=b)", "", "ac ab").as_deref(), Some("a"));
        assert_eq!(whole("a(?!b)", "", "ab ac").as_deref(), Some("a"));
        assert_eq!(whole("(?<=x)a", "", "a xa").as_deref(), Some("a"));
        assert_eq!(find("(?<=x)a", "", "ya"), None);
        assert_eq!(find("(?<!x)a", "", "xa"), None);
        assert_eq!(whole("(?<!x)a", "", "xa a").as_deref(), Some("a"));
    }

    #[test]
    fn case_can_be_ignored() {
        assert_eq!(whole("AbC", "i", "aBc").as_deref(), Some("aBc"));
        assert_eq!(whole("[a-c]+", "i", "ABC").as_deref(), Some("ABC"));
        assert_eq!(whole("\u{e9}", "i", "\u{c9}").as_deref(), Some("\u{c9}"));
    }

    #[test]
    fn a_loop_that_matches_nothing_stops() {
        assert_eq!(whole("(a*)*", "", "b").as_deref(), Some(""));
        assert_eq!(whole("(?:)*x", "", "x").as_deref(), Some("x"));
    }

    #[test]
    fn unicode_mode_counts_whole_characters() {
        assert_eq!(whole("^.$", "u", "\u{1F600}").as_deref(), Some("\u{1F600}"));
        assert_eq!(whole("^.$", "", "\u{1F600}"), None);
        assert_eq!(
            whole(r"\u{1F600}", "u", "\u{1F600}").as_deref(),
            Some("\u{1F600}")
        );
        assert_eq!(
            whole(r"\p{L}+", "u", "12h\u{e9}llo3").as_deref(),
            Some("h\u{e9}llo")
        );
    }

    #[test]
    fn escapes_mean_what_they_say() {
        assert_eq!(whole(r"\.", "", "a.").as_deref(), Some("."));
        assert_eq!(whole(r"\x41", "", "A").as_deref(), Some("A"));
        assert_eq!(whole(r"A", "", "A").as_deref(), Some("A"));
        assert_eq!(whole(r"[\d-]+", "", "1-2").as_deref(), Some("1-2"));
        assert_eq!(whole(r"a{x", "", "a{x").as_deref(), Some("a{x"));
    }

    #[test]
    fn broken_patterns_are_refused() {
        for pattern in [
            "(",
            ")",
            "[a",
            "*",
            "a{5000}",
            "(?<n>a)\\k<m>",
            "[z-a]",
            "a{2,1}",
            "(?<a>)(?<a>)",
            "(?<1>)",
            "(?<=a)*",
            "(?i-i:a)",
            "(?ii:a)",
            "(?:",
            "^*",
        ] {
            let units: Vec<u16> = pattern.encode_utf16().collect();
            assert!(compile(&units, "").is_err(), "{pattern}");
        }
    }

    #[test]
    fn a_terrible_pattern_gives_up_rather_than_hangs() {
        let pattern: Vec<u16> = "(a*)*(a*)*(a*)*(a*)*b".encode_utf16().collect();
        let program = compile(&pattern, "").unwrap();
        let input: Vec<u16> = "a".repeat(200).encode_utf16().collect();
        let mut steps = 0;
        let result = match_at(&program, &input, 0, &mut steps);
        assert!(result.is_err() || matches!(result, Ok(None)));
    }
}
