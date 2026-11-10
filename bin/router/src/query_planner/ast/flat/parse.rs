//! A parser for executable documents straight into a flat `Document`.
//!
//! Strings come out the same as `graphql_tools`' parser: same escapes (its `\b` quirk too) and
//! the same block string trimming. Directives on operations, fragment definitions and variables
//! are read and dropped, normalization never looks at them.

use super::{
    Argument, Definition, Directive, Document, Fragment, Kind, Operation, OperationKind, Selection,
    Span, Sym, TypeRef, Value, ValueId, VariableDefinition, NONE,
};

#[derive(Debug)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

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
}

struct Parser<'s> {
    src: &'s str,
    bytes: &'s [u8],
    pos: usize,
    token: Token,
    doc: Document,
    /// Stacks for lists being built, nested lists go on top and are taken off when done.
    sel_stack: Vec<u32>,
    value_stack: Vec<ValueId>,
    field_stack: Vec<(Sym, ValueId)>,
}

pub fn parse(src: &str) -> Result<Document, ParseError> {
    let mut parser = Parser {
        src,
        bytes: src.as_bytes(),
        pos: 0,
        token: Token {
            kind: Tok::Eof,
            start: 0,
            end: 0,
        },
        doc: Document::default(),
        sel_stack: Vec::new(),
        value_stack: Vec::new(),
        field_stack: Vec::new(),
    };
    parser.advance()?;
    parser.document()?;
    Ok(parser.doc)
}

impl<'s> Parser<'s> {
    fn error<T>(&self, message: &str) -> Result<T, ParseError> {
        Err(ParseError(format!(
            "{message} at {} ({:?})",
            self.token.start,
            &self.src[self.token.start..self.token.end]
        )))
    }

    fn text(&self) -> &'s str {
        &self.src[self.token.start..self.token.end]
    }

    fn advance(&mut self) -> Result<(), ParseError> {
        let bytes = self.bytes;
        // whitespace, commas, comments and a byte order mark
        loop {
            match bytes.get(self.pos) {
                Some(b' ' | b'\t' | b'\n' | b'\r' | b',') => self.pos += 1,
                Some(b'#') => {
                    while !matches!(bytes.get(self.pos), None | Some(b'\n' | b'\r')) {
                        self.pos += 1;
                    }
                }
                Some(0xEF) if bytes[self.pos..].starts_with("\u{feff}".as_bytes()) => self.pos += 3,
                _ => break,
            }
        }
        let start = self.pos;
        let Some(&first) = bytes.get(start) else {
            self.token = Token {
                kind: Tok::Eof,
                start,
                end: start,
            };
            return Ok(());
        };
        let kind = match first {
            b'!' | b'$' | b'&' | b'(' | b')' | b':' | b'=' | b'@' | b'[' | b']' | b'{' | b'|'
            | b'}' => {
                self.pos += 1;
                Tok::Punct(first)
            }
            b'.' => {
                if !bytes[start..].starts_with(b"...") {
                    return Err(ParseError(format!("unexpected '.' at {start}")));
                }
                self.pos += 3;
                Tok::Spread
            }
            b'_' | b'a'..=b'z' | b'A'..=b'Z' => {
                self.pos += 1;
                while matches!(
                    bytes.get(self.pos),
                    Some(b'_' | b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z')
                ) {
                    self.pos += 1;
                }
                Tok::Name
            }
            b'-' | b'0'..=b'9' => {
                self.pos += 1;
                let mut float = false;
                self.digits();
                if bytes.get(self.pos) == Some(&b'.') {
                    float = true;
                    self.pos += 1;
                    self.digits();
                }
                if matches!(bytes.get(self.pos), Some(b'e' | b'E')) {
                    float = true;
                    self.pos += 1;
                    if matches!(bytes.get(self.pos), Some(b'+' | b'-')) {
                        self.pos += 1;
                    }
                    self.digits();
                }
                if float {
                    Tok::Float
                } else {
                    Tok::Int
                }
            }
            b'"' => {
                if bytes[start..].starts_with(b"\"\"\"") {
                    self.pos += 3;
                    loop {
                        match bytes.get(self.pos) {
                            None => return Err(ParseError("unterminated block string".into())),
                            Some(b'\\') if bytes[self.pos + 1..].starts_with(b"\"\"\"") => {
                                self.pos += 4
                            }
                            Some(b'"') if bytes[self.pos..].starts_with(b"\"\"\"") => {
                                self.pos += 3;
                                break;
                            }
                            _ => self.pos += 1,
                        }
                    }
                    Tok::BlockString
                } else {
                    self.pos += 1;
                    loop {
                        match bytes.get(self.pos) {
                            None | Some(b'\n' | b'\r') => {
                                return Err(ParseError("unterminated string".into()))
                            }
                            Some(b'\\') => self.pos += 2,
                            Some(b'"') => {
                                self.pos += 1;
                                break;
                            }
                            _ => self.pos += 1,
                        }
                    }
                    Tok::String
                }
            }
            _ => return Err(ParseError(format!("unexpected character at {start}"))),
        };
        self.token = Token {
            kind,
            start,
            end: self.pos,
        };
        Ok(())
    }

    fn digits(&mut self) {
        while matches!(self.bytes.get(self.pos), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
    }

    fn is_punct(&self, c: u8) -> bool {
        self.token.kind == Tok::Punct(c)
    }

    fn expect_punct(&mut self, c: u8) -> Result<(), ParseError> {
        if !self.is_punct(c) {
            return self.error(&format!("expected '{}'", c as char));
        }
        self.advance()
    }

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
            let definition = if self.is_punct(b'{') {
                Definition::Operation(Operation {
                    kind: OperationKind::Shorthand,
                    name: NONE,
                    variables: Span::EMPTY,
                    selection_set: self.selection_set()?,
                })
            } else if self.is_keyword("fragment") {
                self.advance()?;
                let name = self.name()?;
                if !self.is_keyword("on") {
                    return self.error("expected 'on'");
                }
                self.advance()?;
                let type_condition = self.name()?;
                self.skip_directives()?;
                Definition::Fragment(Fragment {
                    name,
                    type_condition,
                    selection_set: self.selection_set()?,
                })
            } else {
                let kind = match self.token.kind == Tok::Name {
                    true => match self.text() {
                        "query" => OperationKind::Query,
                        "mutation" => OperationKind::Mutation,
                        "subscription" => OperationKind::Subscription,
                        _ => return self.error("expected a definition"),
                    },
                    false => return self.error("expected a definition"),
                };
                self.advance()?;
                let name = match self.token.kind == Tok::Name {
                    true => self.name()?,
                    false => NONE,
                };
                let variables = self.variable_definitions()?;
                self.skip_directives()?;
                Definition::Operation(Operation {
                    kind,
                    name,
                    variables,
                    selection_set: self.selection_set()?,
                })
            };
            self.doc.definitions.push(definition);
        }
        Ok(())
    }

    fn variable_definitions(&mut self) -> Result<Span, ParseError> {
        let start = self.doc.variables.len() as u32;
        if !self.is_punct(b'(') {
            return Ok(Span { start, len: 0 });
        }
        self.advance()?;
        // values and types don't add variables, so these stay together
        let mut definitions = Vec::new();
        loop {
            // a description, it means nothing for execution
            if matches!(self.token.kind, Tok::String | Tok::BlockString) {
                self.advance()?;
            }
            self.expect_punct(b'$')?;
            let name = self.name()?;
            self.expect_punct(b':')?;
            let ty = self.type_ref()?;
            let default_value = match self.is_punct(b'=') {
                true => {
                    self.advance()?;
                    Some(self.value()?)
                }
                false => None,
            };
            self.skip_directives()?;
            definitions.push(VariableDefinition {
                name,
                ty,
                default_value,
            });
            if self.is_punct(b')') {
                self.advance()?;
                break;
            }
        }
        let start = self.doc.variables.len() as u32;
        let len = definitions.len() as u32;
        self.doc.variables.extend(definitions);
        Ok(Span { start, len })
    }

    fn type_ref(&mut self) -> Result<u32, ParseError> {
        let ty = if self.is_punct(b'[') {
            self.advance()?;
            let inner = self.type_ref()?;
            self.expect_punct(b']')?;
            TypeRef::List(inner)
        } else {
            TypeRef::Named(self.name()?)
        };
        self.doc.types.push(ty);
        let mut id = self.doc.types.len() as u32 - 1;
        if self.is_punct(b'!') {
            self.advance()?;
            self.doc.types.push(TypeRef::NonNull(id));
            id += 1;
        }
        Ok(id)
    }

    fn selection_set(&mut self) -> Result<Span, ParseError> {
        self.expect_punct(b'{')?;
        let mark = self.sel_stack.len();
        while !self.is_punct(b'}') {
            if self.token.kind == Tok::Eof {
                return self.error("expected '}'");
            }
            let selection = self.selection()?;
            self.doc.selections.push(selection);
            self.sel_stack.push(self.doc.selections.len() as u32 - 1);
        }
        self.advance()?;
        if self.sel_stack.len() == mark {
            return self.error("empty selection set");
        }
        let start = self.doc.children.len() as u32;
        self.doc.children.extend(self.sel_stack.drain(mark..));
        Ok(Span {
            start,
            len: self.doc.children.len() as u32 - start,
        })
    }

    fn selection(&mut self) -> Result<Selection, ParseError> {
        if self.token.kind == Tok::Spread {
            self.advance()?;
            if self.is_keyword("on") || self.is_punct(b'@') || self.is_punct(b'{') {
                let type_condition = match self.is_keyword("on") {
                    true => {
                        self.advance()?;
                        self.name()?
                    }
                    false => NONE,
                };
                let directives = self.directives()?;
                return Ok(Selection {
                    kind: Kind::InlineFragment,
                    name: NONE,
                    alias: NONE,
                    type_condition,
                    args: Span::EMPTY,
                    directives,
                    children: self.selection_set()?,
                });
            }
            let name = self.name()?;
            return Ok(Selection {
                kind: Kind::FragmentSpread,
                name,
                alias: NONE,
                type_condition: NONE,
                args: Span::EMPTY,
                directives: self.directives()?,
                children: Span::EMPTY,
            });
        }

        let first = self.name()?;
        let (alias, name) = match self.is_punct(b':') {
            true => {
                self.advance()?;
                (first, self.name()?)
            }
            false => (NONE, first),
        };
        let args = self.arguments()?;
        let directives = self.directives()?;
        let children = match self.is_punct(b'{') {
            true => self.selection_set()?,
            false => Span::EMPTY,
        };
        Ok(Selection {
            kind: Kind::Field,
            name,
            alias,
            type_condition: NONE,
            args,
            directives,
            children,
        })
    }

    fn arguments(&mut self) -> Result<Span, ParseError> {
        if !self.is_punct(b'(') {
            return Ok(Span::EMPTY);
        }
        self.advance()?;
        // values don't add arguments, so one list's arguments stay together
        let start = self.doc.args.len() as u32;
        loop {
            let name = self.name()?;
            self.expect_punct(b':')?;
            let value = self.value()?;
            self.doc.args.push(Argument { name, value });
            if self.is_punct(b')') {
                self.advance()?;
                break;
            }
        }
        Ok(Span {
            start,
            len: self.doc.args.len() as u32 - start,
        })
    }

    fn directives(&mut self) -> Result<Span, ParseError> {
        if !self.is_punct(b'@') {
            return Ok(Span::EMPTY);
        }
        // a directive's arguments go in `args`, so the directives stay together
        let start = self.doc.directives.len() as u32;
        while self.is_punct(b'@') {
            self.advance()?;
            let name = self.name()?;
            let args = self.arguments()?;
            self.doc.directives.push(Directive { name, args });
        }
        Ok(Span {
            start,
            len: self.doc.directives.len() as u32 - start,
        })
    }

    fn skip_directives(&mut self) -> Result<(), ParseError> {
        let len = self.doc.directives.len();
        self.directives()?;
        self.doc.directives.truncate(len);
        Ok(())
    }

    fn value(&mut self) -> Result<ValueId, ParseError> {
        let value = match self.token.kind {
            Tok::Punct(b'$') => {
                self.advance()?;
                Value::Variable(self.name()?)
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
                let text = self.text();
                // most strings have no escapes, those are interned as they are
                let sym = match text.contains('\\') {
                    false => self.doc.strings.intern(&text[1..text.len() - 1]),
                    true => {
                        let unquoted = unquote_string(text)?;
                        self.doc.strings.intern(&unquoted)
                    }
                };
                self.advance()?;
                Value::String(sym)
            }
            Tok::BlockString => {
                let unquoted = unquote_block_string(self.text());
                let sym = self.doc.strings.intern(&unquoted);
                self.advance()?;
                Value::String(sym)
            }
            Tok::Name => {
                let value = match self.text() {
                    "true" => Value::Boolean(true),
                    "false" => Value::Boolean(false),
                    "null" => Value::Null,
                    text => Value::Enum(self.doc.strings.intern(text)),
                };
                self.advance()?;
                value
            }
            Tok::Punct(b'[') => {
                self.advance()?;
                let mark = self.value_stack.len();
                while !self.is_punct(b']') {
                    let item = self.value()?;
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
                    let value = self.value()?;
                    self.field_stack.push((name, value));
                }
                self.advance()?;
                // as written, repeats too, like the parser's tree
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
        Ok(self.doc.values.len() as u32 - 1)
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
            // the same as graphql_tools, which pushes U+0010 for `\b`
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

/// The same trimming as graphql_tools' `unquote_block_string`.
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
