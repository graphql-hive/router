use crate::parser::{
    query::{
        Definition, Directive, Document, FragmentDefinition, OperationDefinition, Selection,
        SelectionSet, Text, Type, TypeCondition, Value, VariableDefinition,
    },
    tokenizer::{Kind, Token, TokenStream},
};
use combine::StreamOnce;
use thiserror::Error;

/// Error minifying query
#[derive(Error, Debug)]
#[error("query minify error: {}", _0)]
pub struct MinifyError(String);

pub fn minify_query(source: &str) -> Result<String, MinifyError> {
    let mut bits: Vec<&str> = Vec::new();
    let mut stream = TokenStream::new(source);
    let mut prev_was_punctuator = false;

    loop {
        match stream.uncons() {
            Ok(x) => {
                let token: Token = x;
                let is_non_punctuator = token.kind != Kind::Punctuator;

                if prev_was_punctuator && is_non_punctuator {
                    bits.push(" ");
                }

                bits.push(token.value);
                prev_was_punctuator = is_non_punctuator;
            }
            Err(ref e) if e == &combine::easy::Error::end_of_input() => break,
            Err(e) => return Err(MinifyError(e.to_string())),
        }
    }

    Ok(bits.join(""))
}

/// Minify a document according to the same rules as `minify_query`
pub fn minify_query_document<'a, T: Text<'a>>(doc: &Document<'a, T>) -> String {
    let mut minifier = Minifier::new();
    minifier.write_document(doc);
    minifier.buffer
}

/// Minifier builds a minified GraphQL query string from a parsed AST.
///
/// This struct uses a single-pass traversal of the document AST, writing directly to a buffer
/// instead of creating intermediate string representations. This approach is significantly
/// faster than the `minify_query` approach which required converting the AST to a Display string first.
///
/// Key optimizations:
/// - Direct buffer writing avoids intermediate allocations
/// - Reusable buffers for number formatting (itoa, ryu) reduce allocation overhead
/// - Tracking `last_was_non_punctuator` allows spacing without post-processing
struct Minifier {
    /// Accumulates the minified output as we traverse the AST
    buffer: String,
    /// Current indentation level (in spaces). Used only for block string formatting.
    block_indent: u16,
    /// Tracks whether the last character written were non-punctuator tokens (identifiers, keywords).
    /// This is essential for maintaining valid GraphQL syntax: identifiers must be separated by spaces.
    /// For example: "query Foo" requires a space between "query" and "Foo", but "query{" doesn't.
    last_was_non_punctuator: bool,
    /// Reusable buffer for converting integers to strings using the `itoa` crate,
    /// that's optimized for fast, allocation-free integer formatting.
    int_buffer: itoa::Buffer,
    /// Reusable buffer for converting floats to strings using the `ryu` crate.
    floats_buffer: ryu::Buffer,
}

impl Minifier {
    fn new() -> Self {
        Self {
            // preallocate a buffer size to minimize reallocations,
            // most queries will fit within 1KB
            buffer: String::with_capacity(1024),
            last_was_non_punctuator: false,
            block_indent: 2,
            int_buffer: itoa::Buffer::new(),
            floats_buffer: ryu::Buffer::new(),
        }
    }

    /// Writes a non-punctuator token (identifier, keyword) to the output.
    ///
    /// We add a space before this token if the last thing written was also a non-punctuator.
    /// We mark ourselves as the last token written, so the next non-punctuator knows to add a space.
    #[inline(always)]
    fn write_non_punctuator(&mut self, s: &str) {
        if self.last_was_non_punctuator {
            self.buffer.push(' ');
        }
        self.buffer.push_str(s);
        self.last_was_non_punctuator = true;
    }

    /// Writes a punctuator token (operator) to the output without spacing logic.
    ///
    /// Punctuators are: {([:!$=@ and so on.
    /// These never need spaces around them in minified GraphQL (e.g., "a:b", not "a : b").
    ///
    /// We reset `last_was_non_punctuator` to false because punctuators don't require spacing before the next token.
    #[inline(always)]
    fn write_punctuator(&mut self, s: &str) {
        self.buffer.push_str(s);
        self.last_was_non_punctuator = false;
    }

    /// Writes a single-character punctuator to the output.
    ///
    /// This is a single-character variant of `write_punctuator` for better performance.
    /// Using `push(char)` is faster than `push_str()` for single characters.
    #[inline(always)]
    fn write_punctuator_char(&mut self, c: char) {
        self.buffer.push(c);
        self.last_was_non_punctuator = false;
    }

    /// Writes the top-level document by iterating through all definitions
    #[inline]
    fn write_document<'a, T: Text<'a>>(&mut self, doc: &Document<'a, T>) {
        for def in &doc.definitions {
            self.write_definition(def);
        }
    }

    #[inline]
    fn write_definition<'a, T: Text<'a>>(&mut self, def: &Definition<'a, T>) {
        match def {
            Definition::Operation(op) => self.write_operation(op),
            Definition::Fragment(frag) => self.write_fragment(frag),
        }
    }

    #[inline]
    fn write_operation<'a, T: Text<'a>>(&mut self, op: &OperationDefinition<'a, T>) {
        match op {
            OperationDefinition::SelectionSet(set) => self.write_selection_set(set),
            OperationDefinition::Query(q) => {
                self.write_non_punctuator("query");
                if let Some(ref name) = q.name {
                    self.write_non_punctuator(name.as_ref());
                }
                self.write_variable_definitions(&q.variable_definitions);
                self.write_directives(&q.directives);
                self.write_selection_set(&q.selection_set);
            }
            OperationDefinition::Mutation(m) => {
                self.write_non_punctuator("mutation");
                if let Some(ref name) = m.name {
                    self.write_non_punctuator(name.as_ref());
                }
                self.write_variable_definitions(&m.variable_definitions);
                self.write_directives(&m.directives);
                self.write_selection_set(&m.selection_set);
            }
            OperationDefinition::Subscription(s) => {
                self.write_non_punctuator("subscription");
                if let Some(ref name) = s.name {
                    self.write_non_punctuator(name.as_ref());
                }
                self.write_variable_definitions(&s.variable_definitions);
                self.write_directives(&s.directives);
                self.write_selection_set(&s.selection_set);
            }
        }
    }

    #[inline]
    fn write_fragment<'a, T: Text<'a>>(&mut self, frag: &FragmentDefinition<'a, T>) {
        self.write_non_punctuator("fragment");
        self.write_non_punctuator(frag.name.as_ref());
        self.write_type_condition(&frag.type_condition);
        self.write_directives(&frag.directives);
        self.write_selection_set(&frag.selection_set);
    }

    /// No separators between items,
    /// each item's trailing punctuation determines spacing.
    #[inline]
    fn write_selection_set<'a, T: Text<'a>>(&mut self, set: &SelectionSet<'a, T>) {
        self.write_punctuator_char('{');
        for item in &set.items {
            self.write_selection(item);
        }
        self.write_punctuator_char('}');
    }

    #[inline]
    fn write_selection<'a, T: Text<'a>>(&mut self, selection: &Selection<'a, T>) {
        match selection {
            Selection::Field(f) => {
                if let Some(ref alias) = f.alias {
                    self.write_non_punctuator(alias.as_ref());
                    self.write_punctuator_char(':');
                }
                self.write_non_punctuator(f.name.as_ref());
                self.write_arguments(&f.arguments);
                self.write_directives(&f.directives);
                if !f.selection_set.items.is_empty() {
                    self.write_selection_set(&f.selection_set);
                }
            }
            Selection::FragmentSpread(fs) => {
                self.write_punctuator("...");
                self.write_non_punctuator(fs.fragment_name.as_ref());
                self.write_directives(&fs.directives);
            }
            Selection::InlineFragment(ifrag) => {
                self.write_punctuator("...");
                if let Some(ref tc) = ifrag.type_condition {
                    self.write_type_condition(tc);
                }
                self.write_directives(&ifrag.directives);
                self.write_selection_set(&ifrag.selection_set);
            }
        }
    }

    #[inline]
    fn write_type_condition<'a, T: Text<'a>>(&mut self, tc: &TypeCondition<'a, T>) {
        match tc {
            TypeCondition::On(name) => {
                self.write_non_punctuator("on");
                self.write_non_punctuator(name.as_ref());
            }
        }
    }

    #[inline]
    fn write_variable_definitions<'a, T: Text<'a>>(&mut self, vars: &[VariableDefinition<'a, T>]) {
        if vars.is_empty() {
            return;
        }
        self.write_punctuator_char('(');
        for var in vars {
            self.write_punctuator_char('$');
            self.write_non_punctuator(var.name.as_ref());
            self.write_punctuator_char(':');
            self.write_type(&var.var_type);
            if let Some(ref def) = var.default_value {
                self.write_punctuator_char('=');
                self.write_value(def);
            }
        }
        self.write_punctuator_char(')');
    }

    #[inline]
    fn write_type<'a, T: Text<'a>>(&mut self, ty: &Type<'a, T>) {
        match ty {
            Type::NamedType(name) => self.write_non_punctuator(name.as_ref()),
            Type::ListType(inner) => {
                self.write_punctuator_char('[');
                self.write_type(inner);
                self.write_punctuator_char(']');
            }
            Type::NonNullType(inner) => {
                self.write_type(inner);
                self.write_punctuator_char('!');
            }
        }
    }

    #[inline]
    fn write_directives<'a, T: Text<'a>>(&mut self, dirs: &[Directive<'a, T>]) {
        for dir in dirs {
            self.write_punctuator_char('@');
            self.write_non_punctuator(dir.name.as_ref());
            self.write_arguments(&dir.arguments);
        }
    }

    #[inline]
    fn write_arguments<'a, T: Text<'a>>(&mut self, args: &[(T::Value, Value<'a, T>)]) {
        if args.is_empty() {
            return;
        }
        self.write_punctuator_char('(');
        for (name, val) in args {
            self.write_non_punctuator(name.as_ref());
            self.write_punctuator_char(':');
            self.write_value(val);
        }
        self.write_punctuator_char(')');
    }

    /// Writes a string value with proper escaping and formatting.
    ///
    /// This method handles three different cases to optimize for common scenarios:
    ///
    /// Case 1: Simple strings (the fast path)
    /// If the string contains no escaping-needed characters and no newlines, we write it
    /// directly with quotes. This is the most common case and avoids character-by-character scanning.
    ///
    /// Case 2: Strings needing escaping but no newlines
    /// We iterate through each character and escape special characters:
    /// - Control characters get special escapes
    /// - Quotes and backslashes: escaped with backslash
    /// - Regular characters: copied as-is
    ///
    /// Case 3: Block strings (multi-line with newlines)
    /// GraphQL supports block strings (triple quotes) for multi-line content.
    /// Block strings preserve line breaks, escape triple-quote sequences, and use indentation.
    /// We use this format if the string contains newlines.
    ///
    /// Spacing: Strings are non-punctuators that require spacing when preceded by another non-punctuator.
    /// We add the space at the beginning if needed, maintaining `last_was_non_punctuator = true`.
    #[inline(always)]
    pub fn write_quoted(&mut self, s: &str) {
        if self.last_was_non_punctuator {
            self.buffer.push(' ');
        }

        let bytes = s.as_bytes();
        let mut has_newline = false;
        let mut needs_escaping = false;

        for &byte in bytes {
            if byte == b'\n' {
                has_newline = true;
            // Check for control characters that need escaping. The parser accepts raw control characters in strings,
            // so we must escape them here to produce valid minified output.
            } else if byte == b'"' || byte == b'\\' || byte < 0x20 || byte == 0x7F {
                needs_escaping = true;
            }

            if has_newline && needs_escaping {
                break;
            }
        }

        if !needs_escaping && !has_newline {
            self.buffer.reserve(s.len() + 2);
            self.buffer.push('"');
            self.buffer.push_str(s);
            self.buffer.push('"');
            self.last_was_non_punctuator = true;
            return;
        }

        if !has_newline {
            use std::fmt::Write;
            /// Reserve extra space for escape sequences. Most strings need 2 bytes for quotes,
            /// and ~16 bytes accounts for typical escape sequences (e.g., \", \\, \u00XX).
            /// The buffer will grow dynamically if this estimate is too small.
            self.buffer.reserve(s.len() + 16);
            self.buffer.push('"');
            for c in s.chars() {
                match c {
                    '\r' => self.buffer.push_str(r"\r"),
                    '\n' => self.buffer.push_str(r"\n"),
                    '\t' => self.buffer.push_str(r"\t"),
                    '"' => self.buffer.push_str("\\\""),
                    '\\' => self.buffer.push_str(r"\\"),
                    // Regular text characters. These are safe to write directly without escaping.
                    '\u{0020}'..='\u{FFFF}' => self.buffer.push(c),
                    // Characters outside the printable range are escaped as \uXXXX.
                    _ => write!(&mut self.buffer, "\\u{:04}", c as u32).unwrap(),
                }
            }
            self.buffer.push('"');
        } else {
            // Block strings can expand significantly with indentation and escape sequences.
            // Reserve space upfront to avoid repeated reallocations.
            self.buffer.reserve(s.len() + 32);
            self.buffer.push_str(r#"""""#);
            self.buffer.push('\n');

            self.block_indent += 2;

            for line in s.lines() {
                if !line.trim().is_empty() {
                    self.indent();
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
            self.indent();

            self.buffer.push_str(r#"""""#);
        }
        self.last_was_non_punctuator = true;
    }

    /// Writes the current indentation level (in spaces).
    #[inline]
    pub fn indent(&mut self) {
        for _ in 0..self.block_indent {
            self.buffer.push(' ');
        }
    }

    #[inline]
    fn write_value<'a, T: Text<'a>>(&mut self, val: &Value<'a, T>) {
        match val {
            Value::Variable(name) => {
                self.write_punctuator_char('$');
                self.write_non_punctuator(name.as_ref());
            }
            Value::Int(n) => {
                // Use itoa's format method for fast, accurate integer-to-string conversion.
                // itoa is much faster than standard Rust integer formatting and produces
                // minimal output (no unnecessary padding or precision).
                let s = self.int_buffer.format(n.0);
                if self.last_was_non_punctuator {
                    self.buffer.push(' ');
                }
                self.buffer.push_str(s);
                self.last_was_non_punctuator = true;
            }
            Value::Float(f) => {
                // Use ryu's format method for fast, accurate float-to-string conversion.
                // ryu produces the shortest decimal representation that correctly round-trips,
                // which is more efficient than using Display or other methods.
                let s = self.floats_buffer.format(*f);
                if self.last_was_non_punctuator {
                    self.buffer.push(' ');
                }
                self.buffer.push_str(s);
                self.last_was_non_punctuator = true;
            }
            Value::String(s) => self.write_quoted(s),
            Value::Boolean(b) => self.write_non_punctuator(if *b { "true" } else { "false" }),
            Value::Null => self.write_non_punctuator("null"),
            Value::Enum(name) => self.write_non_punctuator(name.as_ref()),
            Value::List(items) => {
                self.write_punctuator_char('[');
                for item in items {
                    self.write_value(item);
                }
                self.write_punctuator_char(']');
            }
            Value::Object(fields) => {
                self.write_punctuator_char('{');
                for (name, val) in fields {
                    self.write_non_punctuator(name.as_ref());
                    self.write_punctuator_char(':');
                    self.write_value(val);
                }
                self.write_punctuator_char('}');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn strip_ignored_characters() {
        let source = "
        query SomeQuery($foo: String!, $bar: String) {
            someField(foo: $foo, bar: $bar) {
                a
                b {
                    ... on B {
                        c
                        d
                    }
                }
            }
        }
        ";

        let minified =
            super::minify_query(source.to_string().as_str()).expect("minification failed");

        assert_eq!(
            &minified,
            "query SomeQuery($foo:String!$bar:String){someField(foo:$foo bar:$bar){a b{...on B{c d}}}}"
        );
    }

    #[test]
    fn unexpected_token() {
        let source = "
        query foo {
            bar;
        }
        ";

        let minified = super::minify_query(source.to_string().as_str());

        assert!(minified.is_err());

        assert_eq!(
            minified.unwrap_err().to_string(),
            "query minify error: Unexpected unexpected character ';'"
        );
    }

    #[test]
    fn minify_document_test() {
        let source = "
        query SomeQuery($foo: String!, $bar: String) {
            someField(foo: $foo, bar: $bar) {
                a
                b {
                    ... on B {
                        c
                        d
                    }
                }
            }
        }
        ";

        let doc =
            crate::parser::query::grammar::parse_query::<String>(source).expect("parse failed");
        let minified_doc = super::minify_query_document(&doc);
        let minified_query = super::minify_query(source).expect("minification failed");

        assert_eq!(minified_doc, minified_query);
    }

    #[test]
    fn minify_document_complex() {
        let source = r#"
        mutation DoSomething($input: UpdateInput! = { a: 1, b: "foo" }) @opt(level: 1) {
            updateItem(id: "123", data: $input) {
                id
                ... on Item {
                    name
                    tags
                }
                ...FragmentName
            }
        }
        fragment FragmentName on Item {
            owner {
                id
                email
            }
        }
        "#;

        let doc =
            crate::parser::query::grammar::parse_query::<String>(source).expect("parse failed");
        let minified_doc = super::minify_query_document(&doc);
        let minified_query = super::minify_query(source).expect("minification failed");

        assert_eq!(minified_doc, minified_query);
    }

    #[test]
    fn test_minify_directive_args() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/directive_args.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @r#"query{node@dir(a:1 b:"2" c:true d:false e:null)}"#);

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_directive_args_multiline() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/directive_args_multiline.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @r#"query{node@dir(a:1 b:"2" c:true d:false e:null)}"#);

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_fragment() {
        let source = std::fs::read_to_string("src/parser/tests/queries/fragment.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"fragment frag on Friend{node}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_fragment_spread() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/fragment_spread.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node{id...something}}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_inline_fragment() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/inline_fragment.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node{id...on User{name}}}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_inline_fragment_dir() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/inline_fragment_dir.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node{id...on User@defer{name}}}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_kitchen_sink() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/kitchen-sink.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @r#"
        query queryName($foo:ComplexType$site:Site=MOBILE){whoever123is:node(id:[123 456]){id...on User@defer{field2{id alias:field1(first:10 after:$foo)@include(if:$foo){id...frag}}}...@skip(unless:$foo){id}...{id}}}mutation likeStory{like(story:123)@defer{story{id}}}subscription StoryLikeSubscription($input:StoryLikeSubscribeInput){storyLikeSubscribe(input:$input){story{likers{count}likeSentence{text}}}}fragment frag on Friend{foo(size:$size bar:$b obj:{key:"value" block:"""

              block string uses \"""

          """})}{unnamed(truthy:true falsey:false nullish:null)query}
        "#);

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );

        insta::assert_snapshot!(minified_document, @r#"query queryName($foo:ComplexType$site:Site=MOBILE){whoever123is:node(id:[123 456]){id...on User@defer{field2{id alias:field1(first:10 after:$foo)@include(if:$foo){id...frag}}}...@skip(unless:$foo){id}...{id}}}mutation likeStory{like(story:123)@defer{story{id}}}subscription StoryLikeSubscription($input:StoryLikeSubscribeInput){storyLikeSubscribe(input:$input){story{likers{count}likeSentence{text}}}}fragment frag on Friend{foo(size:$size bar:$b obj:{block:"block string uses \"\"\"" key:"value"})}{unnamed(truthy:true falsey:false nullish:null)query}"#);

        // The output is different because the parsed AST normalizes multiline strings,
        // while minify_query preserves the original formatting.
        // The minify_document has no idea about the original formatting.
    }

    #[test]
    fn test_minify_kitchen_sink_canonical() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/kitchen-sink_canonical.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @r#"query queryName($foo:ComplexType$site:Site=MOBILE){whoever123is:node(id:[123 456]){id...on User@defer{field2{id alias:field1(first:10 after:$foo)@include(if:$foo){id...frag}}}...@skip(unless:$foo){id}...{id}}}mutation likeStory{like(story:123)@defer{story{id}}}subscription StoryLikeSubscription($input:StoryLikeSubscribeInput){storyLikeSubscribe(input:$input){story{likers{count}likeSentence{text}}}}fragment frag on Friend{foo(size:$size bar:$b obj:{block:"block string uses \"\"\"" key:"value"})}{unnamed(truthy:true falsey:false nullish:null)query}"#);

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_minimal() {
        let source = std::fs::read_to_string("src/parser/tests/queries/minimal.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"{a}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_minimal_mutation() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/minimal_mutation.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"mutation{notify}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_minimal_query() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/minimal_query.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_mutation_directive() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/mutation_directive.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"mutation@directive{node}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_mutation_nameless_vars() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/mutation_nameless_vars.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"mutation($first:Int$second:Int){field1(first:$first)field2(second:$second)}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_named_query() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/named_query.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query Foo{field}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_nested_selection() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/nested_selection.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node{id}}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_query_aliases() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_aliases.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{an_alias:node}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_query_arguments() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_arguments.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node(id:1)}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_query_arguments_multiline() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_arguments_multiline.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node(id:1)node(id:1 one:3)}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_query_array_argument_multiline() {
        let source = std::fs::read_to_string(
            "src/parser/tests/queries/query_array_argument_multiline.graphql",
        )
        .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node(id:[5 6 7])}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_query_directive() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_directive.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query@directive{node}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_query_list_argument() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_list_argument.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node(id:1 list:[123 456])}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_query_nameless_vars() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_nameless_vars.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query($first:Int$second:Int){field1(first:$first)field2(second:$second)}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_query_nameless_vars_multiple_fields() {
        let source = std::fs::read_to_string(
            "src/parser/tests/queries/query_nameless_vars_multiple_fields.graphql",
        )
        .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query($houseId:String!$streetNumber:Int!){house(id:$houseId){id name lat lng}street(number:$streetNumber){id}houseStreet(id:$houseId number:$streetNumber){id}}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_query_nameless_vars_multiple_fields_canonical() {
        let source = std::fs::read_to_string(
            "src/parser/tests/queries/query_nameless_vars_multiple_fields_canonical.graphql",
        )
        .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query($houseId:String!$streetNumber:Int!){house(id:$houseId){id name lat lng}street(number:$streetNumber){id}houseStreet(id:$houseId number:$streetNumber){id}}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );

        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_query_object_argument() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_object_argument.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node(id:1 obj:{key1:123 key2:456})}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_query_object_argument_multiline() {
        let source = std::fs::read_to_string(
            "src/parser/tests/queries/query_object_argument_multiline.graphql",
        )
        .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query{node(id:1 obj:{key1:123 key2:456})}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_query_var_default_float() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_var_default_float.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query Foo($site:Float=0.5){field}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_query_var_default_list() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_var_default_list.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query Foo($site:[Int]=[123 456]){field}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_query_var_default_object() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_var_default_object.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query Foo($site:Site={url:null}){field}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_query_var_default_string() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_var_default_string.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @r#"query Foo($site:String="string"){field}"#);

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_query_var_defaults() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_var_defaults.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query Foo($site:Site=MOBILE){field}");
    }

    #[test]
    fn test_minify_query_vars() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/query_vars.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"query Foo($arg:SomeType){field}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_string_literal() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/string_literal.graphql").unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @r#"query{node(id:"hello")}"#);

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_subscription_directive() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/subscription_directive.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @"subscription@directive{node}");

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }

    #[test]
    fn test_minify_triple_quoted_literal() {
        let source =
            std::fs::read_to_string("src/parser/tests/queries/triple_quoted_literal.graphql")
                .unwrap();
        let minified_query = super::minify_query(&source).unwrap();
        insta::assert_snapshot!(minified_query, @r#"
        query{node(id:"""
            Hello,
              world!
          """)}
        "#);

        let minified_document = super::minify_query_document(
            &crate::parser::query::grammar::parse_query::<String>(&source).unwrap(),
        );
        assert_eq!(
            minified_query, minified_document,
            "minify_query and minify_document outputs differ"
        );
    }
}
