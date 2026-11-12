//! Prints a flat `Document` minified, byte for byte like `parser::minify_query_document` prints
//! the tree, quirks included.

use super::{
    Definition, Document, Kind, ListId, OperationKind, Span, TypeId, TypeRef, Value, ValueId,
};

/// The document minified, the same as `minify_query_document` of the tree it was parsed into.
pub fn minify(doc: &Document) -> String {
    let mut m = Minifier {
        doc,
        buffer: String::with_capacity(1024),
        block_indent: 2,
        last_was_non_punctuator: false,
        int_buffer: itoa::Buffer::new(),
        floats_buffer: ryu::Buffer::new(),
    };
    for definition in &doc.definitions {
        match definition {
            Definition::Operation(op) => {
                let keyword = match op.kind {
                    OperationKind::Shorthand => None,
                    OperationKind::Query => Some("query"),
                    OperationKind::Mutation => Some("mutation"),
                    OperationKind::Subscription => Some("subscription"),
                };
                if let Some(keyword) = keyword {
                    m.non_punct(keyword);
                    if let Some(name) = doc.opt_str(op.name) {
                        m.non_punct(name);
                    }
                    m.variables(op.variables);
                    m.directives(op.directives);
                }
                m.selection_set(op.selection_set);
            }
            Definition::Fragment(f) => {
                m.non_punct("fragment");
                m.non_punct(doc.str(f.name));
                m.non_punct("on");
                m.non_punct(doc.str(f.type_condition));
                m.directives(f.directives);
                m.selection_set(f.selection_set);
            }
        }
    }
    m.buffer
}

struct Minifier<'d> {
    doc: &'d Document,
    buffer: String,
    block_indent: u16,
    last_was_non_punctuator: bool,
    int_buffer: itoa::Buffer,
    floats_buffer: ryu::Buffer,
}

impl Minifier<'_> {
    #[inline(always)]
    fn non_punct(&mut self, s: &str) {
        if self.last_was_non_punctuator {
            self.buffer.push(' ');
        }
        self.buffer.push_str(s);
        self.last_was_non_punctuator = true;
    }

    #[inline(always)]
    fn punct(&mut self, s: &str) {
        self.buffer.push_str(s);
        self.last_was_non_punctuator = false;
    }

    #[inline(always)]
    fn punct_char(&mut self, c: char) {
        self.buffer.push(c);
        self.last_was_non_punctuator = false;
    }

    fn selection_set(&mut self, list: ListId) {
        self.punct_char('{');
        for &id in self.doc.items(list) {
            let s = *self.doc.selection(id);
            match s.kind {
                Kind::Field => {
                    if let Some(alias) = self.doc.opt_str(s.alias) {
                        self.non_punct(alias);
                        self.punct_char(':');
                    }
                    self.non_punct(self.doc.str(s.name));
                    self.arguments(s.args);
                    self.directives(s.directives);
                    if !self.doc.is_empty(s.children) {
                        self.selection_set(s.children);
                    }
                }
                Kind::FragmentSpread => {
                    self.punct("...");
                    self.non_punct(self.doc.str(s.name));
                    self.directives(s.directives);
                }
                Kind::InlineFragment => {
                    self.punct("...");
                    if let Some(type_condition) = self.doc.opt_str(s.type_condition) {
                        self.non_punct("on");
                        self.non_punct(type_condition);
                    }
                    self.directives(s.directives);
                    self.selection_set(s.children);
                }
            }
        }
        self.punct_char('}');
    }

    fn variables(&mut self, span: Span) {
        if span.is_empty() {
            return;
        }
        self.punct_char('(');
        for variable in &self.doc.variables[span.range()] {
            self.punct_char('$');
            self.non_punct(self.doc.str(variable.name));
            self.punct_char(':');
            self.type_ref(variable.ty);
            if let Some(value) = variable.default_value {
                self.punct_char('=');
                self.value(value);
            }
        }
        self.punct_char(')');
    }

    fn type_ref(&mut self, ty: TypeId) {
        match self.doc.types[ty as usize] {
            TypeRef::Named(name) => self.non_punct(self.doc.str(name)),
            TypeRef::List(inner) => {
                self.punct_char('[');
                self.type_ref(inner);
                self.punct_char(']');
            }
            TypeRef::NonNull(inner) => {
                self.type_ref(inner);
                self.punct_char('!');
            }
        }
    }

    fn directives(&mut self, span: Span) {
        for directive in self.doc.directives_of(span) {
            self.punct_char('@');
            self.non_punct(self.doc.str(directive.name));
            self.arguments(directive.args);
        }
    }

    fn arguments(&mut self, span: Span) {
        if span.is_empty() {
            return;
        }
        self.punct_char('(');
        for argument in self.doc.args_of(span) {
            self.non_punct(self.doc.str(argument.name));
            self.punct_char(':');
            self.value(argument.value);
        }
        self.punct_char(')');
    }

    fn value(&mut self, id: ValueId) {
        match *self.doc.value(id) {
            Value::Variable(name) => {
                self.punct_char('$');
                self.non_punct(self.doc.str(name));
            }
            Value::Int(int) => {
                let s = self.int_buffer.format(int);
                if self.last_was_non_punctuator {
                    self.buffer.push(' ');
                }
                self.buffer.push_str(s);
                self.last_was_non_punctuator = true;
            }
            Value::Float(float) => {
                let s = self.floats_buffer.format(float);
                if self.last_was_non_punctuator {
                    self.buffer.push(' ');
                }
                self.buffer.push_str(s);
                self.last_was_non_punctuator = true;
            }
            Value::String(s) => self.quoted(s),
            Value::Boolean(b) => self.non_punct(if b { "true" } else { "false" }),
            Value::Null => self.non_punct("null"),
            Value::Enum(name) => self.non_punct(self.doc.str(name)),
            Value::List(items) => {
                self.punct_char('[');
                for &item in &self.doc.value_items[items.range()] {
                    self.value(item);
                }
                self.punct_char(']');
            }
            Value::Object(fields) => {
                self.punct_char('{');
                for &(name, value) in &self.doc.object_fields[fields.range()] {
                    self.non_punct(self.doc.str(name));
                    self.punct_char(':');
                    self.value(value);
                }
                self.punct_char('}');
            }
        }
    }

    /// `Minifier::write_quoted` of the tree minifier.
    fn quoted(&mut self, sym: u32) {
        let s = self.doc.str(sym);
        if self.last_was_non_punctuator {
            self.buffer.push(' ');
        }

        let mut has_newline = false;
        let mut needs_escaping = false;
        for &byte in s.as_bytes() {
            if byte == b'\n' {
                has_newline = true;
            } else if byte == b'"' || byte == b'\\' || byte < 0x20 || byte == 0x7F {
                needs_escaping = true;
            }
            if has_newline && needs_escaping {
                break;
            }
        }

        if !needs_escaping && !has_newline {
            self.buffer.push('"');
            self.buffer.push_str(s);
            self.buffer.push('"');
        } else if !has_newline {
            use std::fmt::Write;
            self.buffer.push('"');
            for c in s.chars() {
                match c {
                    '\r' => self.buffer.push_str(r"\r"),
                    '\n' => self.buffer.push_str(r"\n"),
                    '\t' => self.buffer.push_str(r"\t"),
                    '"' => self.buffer.push_str("\\\""),
                    '\\' => self.buffer.push_str(r"\\"),
                    '\u{0020}'..='\u{FFFF}' => self.buffer.push(c),
                    // the tree minifier prints the code point in decimal here, kept as is
                    _ => write!(&mut self.buffer, "\\u{:04}", c as u32).unwrap(),
                }
            }
            self.buffer.push('"');
        } else {
            self.buffer.push_str(r#"""""#);
            self.buffer.push('\n');
            self.block_indent += 2;
            for line in s.lines() {
                if !line.trim().is_empty() {
                    for _ in 0..self.block_indent {
                        self.buffer.push(' ');
                    }
                    let mut last_pos = 0;
                    for (pos, _) in line.match_indices(r#"""""#) {
                        self.buffer.push_str(&line[last_pos..pos]);
                        self.buffer.push_str(r#"\"""#);
                        last_pos = pos + 3;
                    }
                    self.buffer.push_str(&line[last_pos..]);
                }
                self.buffer.push('\n');
            }
            self.block_indent -= 2;
            for _ in 0..self.block_indent {
                self.buffer.push(' ');
            }
            self.buffer.push_str(r#"""""#);
        }
        self.last_was_non_punctuator = true;
    }
}
