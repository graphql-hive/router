//! Parses an executable document straight into a flat `Document`.
//!
//! It accepts what `parser::query::parse_query` accepts and rejects what it rejects, and counts
//! positions its way: a tab is eight columns, `\r` is nothing, a comment ends a line on `\r` or
//! `\n`. Error messages are its own; to show the familiar ones, parse again with the tree parser.

use super::{
    Argument, Definition, Directive, Document, Fragment, Kind, ListId, Operation, OperationKind,
    Pos, SelId, Selection, Span, Sym, TypeId, TypeRef, Value, ValueId, VariableDefinition, NONE,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tok {
    Punct(u8),
    Spread,
    Name,
    Int,
    Float,
    String,
    BlockString,
    Eof,
}

#[derive(Clone, Copy)]
struct Token {
    kind: Tok,
    start: usize,
    end: usize,
    pos: Pos,
}

/// The tree parser's default nesting limit for `(`, `[` and `{`.
const RECURSION_LIMIT: usize = 50;

struct Parser<'s> {
    src: &'s str,
    bytes: &'s [u8],
    /// Byte offset and position of the next token, after whitespace.
    off: usize,
    pos: Pos,
    token: Token,
    depth_left: usize,
    token_limit: Option<usize>,
    token_count: usize,
    doc: Document,
    /// Items of lists being built. Nested lists go on top and are taken off when done.
    sel_stack: Vec<SelId>,
    value_stack: Vec<ValueId>,
    field_stack: Vec<(Sym, ValueId)>,
    arg_stack: Vec<Argument>,
    directive_stack: Vec<Directive>,
}

pub fn parse(src: &str) -> Result<Document, ParseError> {
    parse_with_limits(src, None)
}

/// Like `parser::query::parse_query_with_token_limit` when `token_limit` is set.
pub fn parse_with_limits(src: &str, token_limit: Option<usize>) -> Result<Document, ParseError> {
    let mut parser = Parser {
        src,
        bytes: src.as_bytes(),
        off: 0,
        pos: Pos { line: 1, column: 1 },
        token: Token {
            kind: Tok::Eof,
            start: 0,
            end: 0,
            pos: Pos::default(),
        },
        depth_left: RECURSION_LIMIT,
        token_limit,
        token_count: 0,
        doc: Document::default(),
        sel_stack: Vec::new(),
        value_stack: Vec::new(),
        field_stack: Vec::new(),
        arg_stack: Vec::new(),
        directive_stack: Vec::new(),
    };
    parser.skip_whitespace();
    parser.advance()?;
    parser.document()?;
    Ok(parser.doc)
}

impl<'s> Parser<'s> {
    fn error<T>(&self, message: &str) -> Result<T, ParseError> {
        Err(ParseError(format!(
            "{message} at {}:{} ({:?})",
            self.token.pos.line,
            self.token.pos.column,
            &self.src[self.token.start..self.token.end]
        )))
    }

    #[inline]
    fn text(&self) -> &'s str {
        &self.src[self.token.start..self.token.end]
    }

    /// Whitespace, commas, comments, `\r` and byte order marks, counted like the tree parser.
    fn skip_whitespace(&mut self) {
        let bytes = self.bytes;
        loop {
            match bytes.get(self.off) {
                Some(b'\r') => self.off += 1,
                Some(b'\t') => {
                    self.pos.column += 8;
                    self.off += 1;
                }
                Some(b'\n') => {
                    self.pos.line += 1;
                    self.pos.column = 1;
                    self.off += 1;
                }
                Some(b' ' | b',') => {
                    self.pos.column += 1;
                    self.off += 1;
                }
                Some(b'#') => {
                    self.off += 1;
                    while let Some(&b) = bytes.get(self.off) {
                        self.off += 1;
                        if b == b'\r' || b == b'\n' {
                            self.pos.line += 1;
                            self.pos.column = 1;
                            break;
                        }
                    }
                }
                Some(0xEF) if bytes[self.off..].starts_with("\u{feff}".as_bytes()) => self.off += 3,
                _ => break,
            }
        }
    }

    /// Reads the next token into `self.token`, then the whitespace after it.
    fn advance(&mut self) -> Result<(), ParseError> {
        let bytes = self.bytes;
        let start = self.off;
        let pos = self.pos;
        // checked before the end of the input too, like the tree tokenizer
        if let Some(limit) = self.token_limit {
            if self.token_count >= limit {
                return Err(ParseError("Token limit exceeded".into()));
            }
        }
        let Some(&first) = bytes.get(start) else {
            self.token = Token {
                kind: Tok::Eof,
                start,
                end: start,
                pos,
            };
            return Ok(());
        };
        let (kind, counted) = match first {
            b'(' | b'[' | b'{' => {
                self.depth_left = self
                    .depth_left
                    .checked_sub(1)
                    .ok_or_else(|| ParseError("Recursion limit exceeded".into()))?;
                self.off += 1;
                (Tok::Punct(first), true)
            }
            b')' | b']' | b'}' => {
                self.depth_left = self.depth_left.saturating_add(1);
                self.off += 1;
                (Tok::Punct(first), true)
            }
            b'!' | b'$' | b':' | b'=' | b'@' | b'|' | b'&' => {
                self.off += 1;
                (Tok::Punct(first), true)
            }
            b'.' => {
                if !bytes[start..].starts_with(b"...") {
                    return Err(ParseError(format!(
                        "bare dot at {}:{}, only \"...\"",
                        pos.line, pos.column
                    )));
                }
                self.off += 3;
                (Tok::Spread, true)
            }
            b'_' | b'a'..=b'z' | b'A'..=b'Z' => {
                self.off += 1;
                while matches!(
                    bytes.get(self.off),
                    Some(b'_' | b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z')
                ) {
                    self.off += 1;
                }
                // the tree tokenizer doesn't count a name that ends the input
                (Tok::Name, self.off < bytes.len())
            }
            b'-' | b'0'..=b'9' => {
                let mut exponent = None;
                let mut real = None;
                let mut end = start + 1;
                // scan to a delimiter and check it after, like the tree tokenizer
                while let Some(&b) = bytes.get(end) {
                    match b {
                        b' ' | b'\n' | b'\r' | b'\t' | b',' | b'#' | b'!' | b'$' | b':' | b'='
                        | b'@' | b'|' | b'&' | b'(' | b')' | b'[' | b']' | b'{' | b'}' => break,
                        b'.' => real = Some(end - start),
                        b'e' | b'E' => exponent = Some(end - start),
                        _ => {}
                    }
                    end += 1;
                }
                let value = &self.src[start..end];
                self.off = end;
                if exponent.is_some() || real.is_some() {
                    if !check_float(value, exponent, real) {
                        return Err(ParseError(format!("unsupported float {value:?}")));
                    }
                    (Tok::Float, false)
                } else {
                    if !check_int(value) {
                        return Err(ParseError(format!("unsupported integer {value:?}")));
                    }
                    (Tok::Int, true)
                }
            }
            b'"' => {
                if bytes[start..].starts_with(b"\"\"\"") {
                    let tail = &self.src[start + 3..];
                    let mut end = None;
                    for (idx, _) in tail.match_indices("\"\"\"") {
                        if !tail[..idx].ends_with('\\') {
                            end = Some(start + 3 + idx + 3);
                            break;
                        }
                    }
                    let Some(end) = end else {
                        return Err(ParseError("unterminated block string value".into()));
                    };
                    self.off = end;
                    (Tok::BlockString, false)
                } else {
                    let mut escaped = false;
                    let mut end = None;
                    for (idx, c) in self.src[start + 1..].char_indices() {
                        match c {
                            '"' if !escaped => {
                                end = Some(start + 1 + idx + 1);
                                break;
                            }
                            '\n' => {
                                return Err(ParseError("unterminated string value".into()));
                            }
                            _ => {}
                        }
                        escaped = !escaped && c == '\\';
                    }
                    let Some(end) = end else {
                        return Err(ParseError("unterminated string value".into()));
                    };
                    self.off = end;
                    (Tok::String, false)
                }
            }
            _ => {
                let c = self.src[start..].chars().next().unwrap_or_default();
                return Err(ParseError(format!("unexpected character {c:?}")));
            }
        };
        if counted && self.token_limit.is_some() {
            self.token_count += 1;
        }
        // the position after the token: columns by bytes, by chars for strings, and lines for
        // block strings, as the tree tokenizer counts them
        let value = &self.src[start..self.off];
        match kind {
            Tok::String => self.pos.column += value.chars().count() as u32,
            Tok::BlockString => {
                let lines = value.bytes().filter(|&b| b == b'\n').count() as u32;
                if lines > 0 {
                    let last = value.rfind('\n').unwrap() + 1;
                    self.pos.line += lines;
                    self.pos.column = value[last..].chars().count() as u32 + 1;
                } else {
                    self.pos.column += value.chars().count() as u32;
                }
            }
            _ => self.pos.column += value.len() as u32,
        }
        self.token = Token {
            kind,
            start,
            end: self.off,
            pos,
        };
        self.skip_whitespace();
        Ok(())
    }

    #[inline]
    fn is_punct(&self, c: u8) -> bool {
        self.token.kind == Tok::Punct(c)
    }

    fn expect_punct(&mut self, c: u8) -> Result<(), ParseError> {
        if !self.is_punct(c) {
            return self.error(&format!("expected '{}'", c as char));
        }
        self.advance()
    }

    #[inline]
    fn is_keyword(&self, keyword: &str) -> bool {
        self.token.kind == Tok::Name && self.text() == keyword
    }

    fn name(&mut self) -> Result<Sym, ParseError> {
        if self.token.kind != Tok::Name {
            return self.error("expected a name");
        }
        let sym = self.doc.strings.intern(self.text());
        self.advance()?;
        Ok(sym)
    }

    fn document(&mut self) -> Result<(), ParseError> {
        if self.token.kind == Tok::Eof {
            return self.error("expected a definition");
        }
        while self.token.kind != Tok::Eof {
            let pos = self.token.pos;
            let definition = if self.is_punct(b'{') {
                Definition::Operation(Operation {
                    kind: OperationKind::Shorthand,
                    name: NONE,
                    variables: Span::EMPTY,
                    directives: Span::EMPTY,
                    selection_set: self.selection_set()?,
                    pos,
                })
            } else if self.is_keyword("fragment") {
                self.advance()?;
                let name = self.name()?;
                if !self.is_keyword("on") {
                    return self.error("expected 'on'");
                }
                self.advance()?;
                let type_condition = self.name()?;
                let directives = self.directives()?;
                Definition::Fragment(Fragment {
                    name,
                    type_condition,
                    directives,
                    selection_set: self.selection_set()?,
                    pos,
                })
            } else {
                let kind = if self.is_keyword("query") {
                    OperationKind::Query
                } else if self.is_keyword("mutation") {
                    OperationKind::Mutation
                } else if self.is_keyword("subscription") {
                    OperationKind::Subscription
                } else {
                    return self.error("expected a definition");
                };
                self.advance()?;
                let name = if self.token.kind == Tok::Name {
                    self.name()?
                } else {
                    NONE
                };
                let variables = self.variable_definitions()?;
                let directives = self.directives()?;
                Definition::Operation(Operation {
                    kind,
                    name,
                    variables,
                    directives,
                    selection_set: self.selection_set()?,
                    pos,
                })
            };
            self.doc.definitions.push(definition);
        }
        Ok(())
    }

    fn variable_definitions(&mut self) -> Result<Span, ParseError> {
        if !self.is_punct(b'(') {
            return Ok(Span::EMPTY);
        }
        self.advance()?;
        let mut defs = Vec::new();
        loop {
            let pos = self.token.pos;
            // a description means nothing for execution, it's dropped
            if matches!(self.token.kind, Tok::String | Tok::BlockString) {
                self.advance()?;
            }
            self.expect_punct(b'$')?;
            let name = self.name()?;
            self.expect_punct(b':')?;
            let ty = self.type_ref()?;
            let default_value = if self.is_punct(b'=') {
                self.advance()?;
                Some(self.value(true)?)
            } else {
                None
            };
            defs.push(VariableDefinition {
                name,
                ty,
                default_value,
                pos,
            });
            if self.is_punct(b')') {
                self.advance()?;
                break;
            }
        }
        let start = self.doc.variables.len() as u32;
        self.doc.variables.extend(defs);
        Ok(Span {
            start,
            len: self.doc.variables.len() as u32 - start,
        })
    }

    fn type_ref(&mut self) -> Result<TypeId, ParseError> {
        let inner = if self.is_punct(b'[') {
            self.advance()?;
            let inner = self.type_ref()?;
            self.expect_punct(b']')?;
            TypeRef::List(inner)
        } else {
            TypeRef::Named(self.name()?)
        };
        self.doc.types.push(inner);
        let mut id = self.doc.types.len() as TypeId - 1;
        if self.is_punct(b'!') {
            self.advance()?;
            self.doc.types.push(TypeRef::NonNull(id));
            id = self.doc.types.len() as TypeId - 1;
        }
        Ok(id)
    }

    fn selection_set(&mut self) -> Result<ListId, ParseError> {
        self.expect_punct(b'{')?;
        let mark = self.sel_stack.len();
        loop {
            let selection = self.selection()?;
            self.sel_stack.push(selection);
            if self.is_punct(b'}') {
                self.advance()?;
                break;
            }
        }
        let list = self.doc.list_of(&self.sel_stack[mark..]);
        self.sel_stack.truncate(mark);
        Ok(list)
    }

    fn selection(&mut self) -> Result<SelId, ParseError> {
        let selection = if self.token.kind == Tok::Spread {
            self.advance()?;
            let pos = self.token.pos;
            if self.is_keyword("on") {
                self.advance()?;
                let type_condition = self.name()?;
                let directives = self.directives()?;
                Selection {
                    kind: Kind::InlineFragment,
                    name: NONE,
                    alias: NONE,
                    type_condition,
                    args: Span::EMPTY,
                    directives,
                    children: self.selection_set()?,
                    pos,
                }
            } else if self.token.kind == Tok::Name {
                let name = self.name()?;
                let directives = self.directives()?;
                Selection {
                    kind: Kind::FragmentSpread,
                    name,
                    alias: NONE,
                    type_condition: NONE,
                    args: Span::EMPTY,
                    directives,
                    children: self.doc.new_list(),
                    pos,
                }
            } else {
                let directives = self.directives()?;
                Selection {
                    kind: Kind::InlineFragment,
                    name: NONE,
                    alias: NONE,
                    type_condition: NONE,
                    args: Span::EMPTY,
                    directives,
                    children: self.selection_set()?,
                    pos,
                }
            }
        } else {
            let pos = self.token.pos;
            let first = self.name()?;
            let (alias, name) = if self.is_punct(b':') {
                self.advance()?;
                (first, self.name()?)
            } else {
                (NONE, first)
            };
            let args = self.arguments()?;
            let directives = self.directives()?;
            let children = if self.is_punct(b'{') {
                self.selection_set()?
            } else {
                self.doc.new_list()
            };
            Selection {
                kind: Kind::Field,
                name,
                alias,
                type_condition: NONE,
                args,
                directives,
                children,
                pos,
            }
        };
        self.doc.selections.push(selection);
        Ok(self.doc.selections.len() as SelId - 1)
    }

    fn arguments(&mut self) -> Result<Span, ParseError> {
        if !self.is_punct(b'(') {
            return Ok(Span::EMPTY);
        }
        self.advance()?;
        let mark = self.arg_stack.len();
        loop {
            let name = self.name()?;
            self.expect_punct(b':')?;
            let value = self.value(false)?;
            self.arg_stack.push(Argument { name, value });
            if self.is_punct(b')') {
                self.advance()?;
                break;
            }
        }
        let start = self.doc.args.len() as u32;
        self.doc.args.extend(self.arg_stack.drain(mark..));
        Ok(Span {
            start,
            len: self.doc.args.len() as u32 - start,
        })
    }

    fn directives(&mut self) -> Result<Span, ParseError> {
        let mark = self.directive_stack.len();
        while self.is_punct(b'@') {
            let pos = self.token.pos;
            self.advance()?;
            let name = self.name()?;
            let args = self.arguments()?;
            self.directive_stack.push(Directive { name, args, pos });
        }
        if self.directive_stack.len() == mark {
            return Ok(Span::EMPTY);
        }
        let start = self.doc.directives.len() as u32;
        self.doc
            .directives
            .extend(self.directive_stack.drain(mark..));
        Ok(Span {
            start,
            len: self.doc.directives.len() as u32 - start,
        })
    }

    /// A value. `constant` is a default value's: no variables.
    fn value(&mut self, constant: bool) -> Result<ValueId, ParseError> {
        let value = match self.token.kind {
            Tok::Name => {
                let value = match self.text() {
                    "true" => Value::Boolean(true),
                    "false" => Value::Boolean(false),
                    "null" => Value::Null,
                    name => Value::Enum(self.doc.strings.intern(name)),
                };
                self.advance()?;
                value
            }
            Tok::Int => {
                let Ok(int) = self.text().parse::<i64>() else {
                    return self.error("integer out of range");
                };
                self.advance()?;
                Value::Int(int)
            }
            Tok::Float => {
                let Ok(float) = self.text().parse::<f64>() else {
                    return self.error("bad float");
                };
                self.advance()?;
                Value::Float(float)
            }
            Tok::String => {
                let unquoted = unquote_string(self.text())?;
                self.advance()?;
                Value::String(self.doc.strings.intern(&unquoted))
            }
            Tok::BlockString => {
                let unquoted = unquote_block_string(self.text());
                self.advance()?;
                Value::String(self.doc.strings.intern(&unquoted))
            }
            Tok::Punct(b'$') if !constant => {
                self.advance()?;
                Value::Variable(self.name()?)
            }
            Tok::Punct(b'[') => {
                self.advance()?;
                let mark = self.value_stack.len();
                while !self.is_punct(b']') {
                    let item = self.value(constant)?;
                    self.value_stack.push(item);
                }
                self.advance()?;
                let start = self.doc.value_items.len() as u32;
                self.doc.value_items.extend(self.value_stack.drain(mark..));
                Value::List(Span {
                    start,
                    len: self.doc.value_items.len() as u32 - start,
                })
            }
            Tok::Punct(b'{') => {
                self.advance()?;
                let mark = self.field_stack.len();
                while !self.is_punct(b'}') {
                    let name = self.name()?;
                    self.expect_punct(b':')?;
                    let value = self.value(constant)?;
                    self.field_stack.push((name, value));
                }
                self.advance()?;
                let start = self.doc.object_fields.len() as u32;
                self.doc
                    .object_fields
                    .extend(self.field_stack.drain(mark..));
                Value::Object(Span {
                    start,
                    len: self.doc.object_fields.len() as u32 - start,
                })
            }
            _ => return self.error("expected a value"),
        };
        self.doc.values.push(value);
        Ok(self.doc.values.len() as ValueId - 1)
    }
}

fn check_int(value: &str) -> bool {
    value == "0"
        || value == "-0"
        || (!value.starts_with('0')
            && value != "-"
            && !value.starts_with("-0")
            && value[1..].chars().all(|x| x.is_ascii_digit()))
}

fn check_dec(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|x| x.is_ascii_digit())
}

fn check_exp(value: &str) -> bool {
    let Some(first) = value.chars().next() else {
        return false;
    };
    if first != '-' && first != '+' && (first <= '0' || first >= '9') {
        return false;
    }
    value[1..].chars().all(|x| x.is_ascii_digit())
}

fn check_float(value: &str, exponent: Option<usize>, real: Option<usize>) -> bool {
    match (exponent, real) {
        (Some(e), Some(r)) if e < r => false,
        (Some(e), Some(r)) => {
            check_int(&value[..r]) && check_dec(&value[r + 1..e]) && check_exp(&value[e + 1..])
        }
        (Some(e), None) => check_int(&value[..e]) && check_exp(&value[e + 1..]),
        (None, Some(r)) => check_int(&value[..r]) && check_dec(&value[r + 1..]),
        (None, None) => unreachable!(),
    }
}

fn unquote_string(s: &str) -> Result<String, ParseError> {
    let mut res = String::with_capacity(s.len());
    let mut chars = s[1..s.len() - 1].chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            res.push(c);
            continue;
        }
        match chars.next() {
            Some(c @ ('"' | '\\' | '/')) => res.push(c),
            // the same as the tree parser, which pushes U+0010 for `\b`
            Some('b') => res.push('\u{0010}'),
            Some('f') => res.push('\u{000C}'),
            Some('n') => res.push('\n'),
            Some('r') => res.push('\r'),
            Some('t') => res.push('\t'),
            Some('u') => {
                let code: String = chars.by_ref().take(4).collect();
                match u32::from_str_radix(&code, 16).ok().and_then(char::from_u32) {
                    Some(c) if code.len() == 4 => res.push(c),
                    _ => return Err(ParseError(format!("bad unicode escape {code}"))),
                }
            }
            c => return Err(ParseError(format!("bad escaped char {c:?}"))),
        }
    }
    Ok(res)
}

/// The same trimming as the tree parser's `unquote_block_string`.
fn unquote_block_string(src: &str) -> String {
    let lines = src[3..src.len() - 3].lines();

    let mut common_indent = usize::MAX;
    let mut first_non_empty_line: Option<usize> = None;
    let mut last_non_empty_line = 0;
    for (idx, line) in lines.clone().enumerate() {
        let indent = line.len() - line.trim_start().len();
        if indent == line.len() {
            continue;
        }
        first_non_empty_line.get_or_insert(idx);
        last_non_empty_line = idx;
        if idx != 0 {
            common_indent = std::cmp::min(common_indent, indent);
        }
    }

    let Some(first_non_empty_line) = first_non_empty_line else {
        return String::new();
    };

    let mut result = String::with_capacity(src.len() - 6);
    let mut lines = lines
        .enumerate()
        .skip(first_non_empty_line)
        .take(last_non_empty_line - first_non_empty_line + 1)
        .map(|(idx, line)| {
            if idx != 0 && line.len() >= common_indent {
                &line[common_indent..]
            } else {
                line
            }
        })
        .map(|x| x.replace(r#"\""""#, r#"""""#));

    if let Some(line) = lines.next() {
        result.push_str(&line);
        for line in lines {
            result.push('\n');
            result.push_str(&line);
        }
    }
    result
}
