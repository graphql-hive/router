//! A JSON parser for subgraph responses that knows which fields the fetch asked for.
//!
//! Subgraphs send fields in the order the query asks for them. So for every object we check the
//! next key we expect with one byte compare, instead of reading it as a string, and allocate
//! the object at its field count.
//!
//! The shape is only a hint. A key it doesn't expect gets looked up among the fields, and one it
//! doesn't know at all is parsed like any other JSON. Custom scalars are kept as raw JSON at the
//! fetch's `CustomScalarPaths`, the same as `PathAwareValueVisitor` does. Anything this parser isn't sure about
//! (invalid JSON, escaped keys, deep nesting) makes it give up, and the response goes through
//! sonic instead, so what we accept and the errors we return don't change.

use std::ops::Range;

use crate::executor::response::{graphql_error::GraphQLError, value::Value};
use crate::query_planner::ast::{
    document::Document, selection_item::SelectionItem, selection_set::SelectionSet,
};
use crate::query_planner::planner::plan_nodes::CustomScalarPaths;

/// The fields of every object in a fetch's response, built once with the plan.
#[derive(Debug, Clone)]
pub struct ResponseShape {
    /// Every object's fields sit next to each other, so a field only needs to know where its
    /// own fields are.
    nodes: Vec<Node>,
    data: Range<u32>,
}

#[derive(Debug, Clone)]
struct Node {
    /// The response key with its quotes, `"name"`, to compare against the input as is.
    quoted_key: Box<[u8]>,
    /// The fields of the object it holds. Empty for scalars.
    fields: Range<u32>,
}

impl Node {
    fn key(&self) -> &[u8] {
        &self.quoted_key[1..self.quoted_key.len() - 1]
    }
}

struct Field<'d> {
    key: &'d str,
    fields: Vec<Field<'d>>,
}

impl ResponseShape {
    pub fn new(document: &Document) -> Self {
        let mut data = Vec::new();
        collect(document, &document.operation.selection_set, &mut data);
        let mut nodes = Vec::new();
        let data = flatten(&data, &mut nodes);
        Self { nodes, data }
    }

    fn fields(&self, range: &Range<u32>) -> &[Node] {
        &self.nodes[range.start as usize..range.end as usize]
    }
}

/// Fields under every type condition go in one list, in the order they first show up. An object
/// of one type then has its fields in the same order with gaps, which the parser skips over.
// ponytail: objects of an abstract type get room for the fields of all its types, split the
// shape per __typename if that memory shows up
fn collect<'d>(document: &'d Document, set: &'d SelectionSet, out: &mut Vec<Field<'d>>) {
    for item in &set.items {
        match item {
            SelectionItem::Field(field) => {
                let key = field.selection_identifier();
                let index = match out.iter().position(|f| f.key == key) {
                    Some(index) => index,
                    None => {
                        out.push(Field {
                            key,
                            fields: Vec::new(),
                        });
                        out.len() - 1
                    }
                };
                collect(document, &field.selections, &mut out[index].fields);
            }
            SelectionItem::InlineFragment(fragment) => collect(document, &fragment.selections, out),
            SelectionItem::FragmentSpread(name) => {
                if let Some(fragment) = document.fragments.iter().find(|f| &f.name == name) {
                    collect(document, &fragment.selection_set, out)
                }
            }
        }
    }
}

fn flatten(fields: &[Field], nodes: &mut Vec<Node>) -> Range<u32> {
    let start = nodes.len();
    nodes.extend(fields.iter().map(|field| Node {
        quoted_key: format!("\"{}\"", field.key).into_bytes().into(),
        fields: 0..0,
    }));
    for (i, field) in fields.iter().enumerate() {
        nodes[start + i].fields = flatten(&field.fields, nodes);
    }
    start as u32..(start + fields.len()) as u32
}

/// What we know about a value before reading it.
#[derive(Clone, Copy)]
struct Hint<'h> {
    shape: &'h ResponseShape,
    /// The fields its objects have
    fields: &'h [Node],
    /// Where the custom scalars under it are, `None` when there aren't any
    scalars: Option<&'h CustomScalarPaths>,
}

pub struct ParsedResponse<'a> {
    pub data: Value<'a>,
    pub errors: Option<Vec<GraphQLError>>,
    pub extensions: Option<Value<'a>>,
}

/// `None` when the response is one for sonic to parse.
pub fn parse_response<'a>(
    input: &'a [u8],
    shape: &ResponseShape,
    custom_scalar_paths: Option<&CustomScalarPaths>,
) -> Option<ParsedResponse<'a>> {
    std::str::from_utf8(input).ok()?;
    let mut parser = Parser {
        input,
        pos: 0,
        depth: 0,
        elements: Vec::new(),
        entries: Vec::new(),
    };
    let mut response = ParsedResponse {
        data: Value::Null,
        errors: None,
        extensions: None,
    };
    let (mut has_data, mut has_errors, mut has_extensions) = (false, false, false);

    parser.whitespace();
    if parser.peek() != Some(b'{') {
        return None;
    }
    parser.pos += 1;
    if !parser.empty(b'}') {
        loop {
            parser.whitespace();
            let key = parser.key()?;
            parser.expect(b':')?;
            match key {
                // a second one is an error, sonic says which
                "data" if !std::mem::replace(&mut has_data, true) => {
                    response.data = parser.shaped(Hint {
                        shape,
                        fields: shape.fields(&shape.data),
                        scalars: custom_scalar_paths.filter(|paths| !paths.is_empty()),
                    })?;
                }
                "errors" if !std::mem::replace(&mut has_errors, true) => {
                    parser.whitespace();
                    let start = parser.pos;
                    parser.value()?;
                    let errors = &input[start..parser.pos];
                    response.errors = Some(sonic_rs::from_slice(errors).ok()?);
                }
                "extensions" if !std::mem::replace(&mut has_extensions, true) => {
                    response.extensions = Some(parser.value()?);
                }
                "data" | "errors" | "extensions" => return None,
                _ => {
                    parser.value()?;
                }
            }
            if !parser.more(b'}')? {
                break;
            }
        }
    }
    parser.whitespace();
    (parser.pos == input.len()).then_some(response)
}

/// sonic allows 254, anything deeper than this goes to sonic, so we never run out of stack.
const MAX_DEPTH: u32 = 128;

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
    depth: u32,
    // children of the objects and lists still being read, so each one gets one exact
    // allocation, like DeserializeScratch
    elements: Vec<Value<'a>>,
    entries: Vec<(&'a str, Value<'a>)>,
}

impl<'a> Parser<'a> {
    #[inline]
    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    #[inline]
    fn whitespace(&mut self) {
        while let Some(b' ' | b'\n' | b'\r' | b'\t') = self.peek() {
            self.pos += 1;
        }
    }

    #[inline]
    fn expect(&mut self, byte: u8) -> Option<()> {
        self.whitespace();
        if self.peek() != Some(byte) {
            return None;
        }
        self.pos += 1;
        Some(())
    }

    /// After a value in an object or a list: true after a comma, false at its end.
    #[inline]
    fn more(&mut self, close: u8) -> Option<bool> {
        self.whitespace();
        let byte = self.peek()?;
        self.pos += 1;
        match byte {
            b',' => Some(true),
            _ if byte == close => Some(false),
            _ => None,
        }
    }

    /// Skips `close` and says true if the object or list is empty.
    #[inline]
    fn empty(&mut self, close: u8) -> bool {
        self.whitespace();
        let empty = self.peek() == Some(close);
        self.pos += empty as usize;
        empty
    }

    #[inline]
    fn enter(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= MAX_DEPTH).then_some(())
    }

    fn value(&mut self) -> Option<Value<'a>> {
        self.whitespace();
        match self.peek()? {
            b'"' => self.string(),
            b'{' => self.object(),
            b'[' => self.list(None),
            b't' => self.literal(b"true", Value::Bool(true)),
            b'f' => self.literal(b"false", Value::Bool(false)),
            b'n' => self.literal(b"null", Value::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    /// A value we expect to be an object with the hint's fields, or a list of them.
    fn shaped(&mut self, hint: Hint) -> Option<Value<'a>> {
        if hint.scalars.is_some_and(|paths| paths.terminal) {
            return self.raw();
        }
        self.whitespace();
        match self.peek()? {
            b'{' => self.shaped_object(hint),
            b'[' => self.list(Some(hint)),
            _ => self.value(),
        }
    }

    fn shaped_object(&mut self, hint: Hint) -> Option<Value<'a>> {
        self.pos += 1;
        if self.empty(b'}') {
            return Some(Value::Object(Vec::new()));
        }
        self.enter()?;
        let nodes = hint.fields;
        let mut entries = Vec::with_capacity(nodes.len());
        let mut next = 0;
        loop {
            self.whitespace();
            let (key, node) = match nodes.get(next) {
                Some(node) if self.input[self.pos..].starts_with(&node.quoted_key) => {
                    next += 1;
                    let start = self.pos + 1;
                    self.pos += node.quoted_key.len();
                    // SAFETY: the input is valid UTF-8 and the key sits between two quotes
                    let key =
                        unsafe { std::str::from_utf8_unchecked(&self.input[start..self.pos - 1]) };
                    (key, Some(node))
                }
                _ => {
                    let key = self.key()?;
                    let index = nodes.iter().position(|node| node.key() == key.as_bytes());
                    // the fields after it most likely follow it
                    if let Some(index) = index {
                        next = index + 1;
                    }
                    (key, index.map(|index| &nodes[index]))
                }
            };
            self.expect(b':')?;
            let fields = node.map_or(&[][..], |node| hint.shape.fields(&node.fields));
            let scalars = hint
                .scalars
                .and_then(|paths| paths.child(key))
                .filter(|paths| !paths.is_empty());
            let value = if fields.is_empty() && scalars.is_none() {
                self.value()?
            } else {
                self.shaped(Hint {
                    shape: hint.shape,
                    fields,
                    scalars,
                })?
            };
            entries.push((key, value));
            if !self.more(b'}')? {
                self.depth -= 1;
                return Some(Value::Object(entries));
            }
        }
    }

    fn object(&mut self) -> Option<Value<'a>> {
        self.pos += 1;
        if self.empty(b'}') {
            return Some(Value::Object(Vec::new()));
        }
        self.enter()?;
        let start = self.entries.len();
        loop {
            self.whitespace();
            let key = self.key()?;
            self.expect(b':')?;
            let value = self.value()?;
            self.entries.push((key, value));
            if !self.more(b'}')? {
                break;
            }
        }
        self.depth -= 1;
        Some(Value::Object(self.entries.drain(start..).collect()))
    }

    fn list(&mut self, hint: Option<Hint>) -> Option<Value<'a>> {
        self.pos += 1;
        if self.empty(b']') {
            return Some(Value::Array(Vec::new()));
        }
        self.enter()?;
        let start = self.elements.len();
        loop {
            let value = match hint {
                Some(hint) => self.shaped(hint)?,
                None => self.value()?,
            };
            self.elements.push(value);
            if !self.more(b']')? {
                break;
            }
        }
        self.depth -= 1;
        Some(Value::Array(self.elements.drain(start..).collect()))
    }

    /// A custom scalar, kept as the JSON it came as.
    // ponytail: builds the value to check it's valid and drops it, so objects and lists as custom
    // scalars allocate for nothing. A skip-only reader if those show up.
    fn raw(&mut self) -> Option<Value<'a>> {
        self.whitespace();
        let start = self.pos;
        self.value()?;
        // SAFETY: the input is valid UTF-8, and a JSON value starts and ends on ASCII
        let raw = unsafe { std::str::from_utf8_unchecked(&self.input[start..self.pos]) };
        Some(Value::RawJson(raw.into()))
    }

    /// Keys are borrowed from the input, so one with escapes is sonic's to reject.
    fn key(&mut self) -> Option<&'a str> {
        if self.peek() != Some(b'"') {
            return None;
        }
        match self.raw_string()? {
            (key, false) => Some(key),
            (_, true) => None,
        }
    }

    fn string(&mut self) -> Option<Value<'a>> {
        let start = self.pos;
        let (raw, escaped) = self.raw_string()?;
        if !escaped {
            return Some(Value::String(raw.into()));
        }
        let decoded: String = sonic_rs::from_slice(&self.input[start..self.pos]).ok()?;
        Some(Value::String(decoded.into()))
    }

    /// What's between the quotes, and whether it has escapes to decode.
    fn raw_string(&mut self) -> Option<(&'a str, bool)> {
        let start = self.pos + 1;
        let mut end = start;
        let mut escaped = false;
        loop {
            // a backslash at the very end skips past it, so `get`
            end += memchr::memchr2(b'"', b'\\', self.input.get(end..)?)?;
            if self.input[end] == b'"' {
                break;
            }
            escaped = true;
            end += 2;
        }
        let raw = &self.input[start..end];
        // JSON strings can't hold these unescaped. Or-ing all of them lets it run in SIMD.
        if raw.iter().fold(false, |found, &byte| found | (byte < 0x20)) {
            return None;
        }
        self.pos = end + 1;
        // SAFETY: the input is valid UTF-8, and a quote is never part of a longer character
        Some((unsafe { std::str::from_utf8_unchecked(raw) }, escaped))
    }

    fn literal(&mut self, word: &[u8], value: Value<'a>) -> Option<Value<'a>> {
        if !self.input[self.pos..].starts_with(word) {
            return None;
        }
        self.pos += word.len();
        Some(value)
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while let Some(b'0'..=b'9') = self.peek() {
            self.pos += 1;
        }
        self.pos - start
    }

    fn number(&mut self) -> Option<Value<'a>> {
        let start = self.pos;
        let negative = self.peek() == Some(b'-');
        self.pos += negative as usize;
        let integer = self.pos;
        // no leading zeros: `0` alone, or a number that starts with 1 to 9
        match self.peek()? {
            b'0' => self.pos += 1,
            b'1'..=b'9' => {
                self.digits();
            }
            _ => return None,
        }
        let integer = &self.input[integer..self.pos];
        let mut float = false;
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if self.digits() == 0 {
                return None;
            }
            float = true;
        }
        if let Some(b'e' | b'E') = self.peek() {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return None;
            }
            float = true;
        }
        if !float {
            let magnitude = integer.iter().try_fold(0u64, |n, &digit| {
                n.checked_mul(10)?.checked_add((digit - b'0') as u64)
            });
            match magnitude {
                Some(n) if !negative => return Some(Value::U64(n)),
                // -0 is a float to sonic
                Some(n) if n != 0 && n <= i64::MIN.unsigned_abs() => {
                    return Some(Value::I64((n as i64).wrapping_neg()))
                }
                _ => {}
            }
        }
        // sonic reads floats, so they come out the same to the last bit
        sonic_rs::from_slice::<f64>(&self.input[start..self.pos])
            .ok()
            .map(Value::F64)
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use graphql_tools::parser::query;

    use super::{parse_response, ResponseShape, MAX_DEPTH};
    use crate::executor::response::subgraph_response::SubgraphResponse;
    use crate::query_planner::ast::{
        document::Document, fragment::FragmentDefinition, operation::OperationDefinition,
    };
    use crate::query_planner::planner::plan_nodes::CustomScalarPaths;
    use crate::query_planner::utils::parsing::parse_operation;

    fn shape(source: &str) -> ResponseShape {
        let mut selection_set = None;
        let mut fragments = Vec::new();
        for definition in parse_operation(source).definitions {
            match definition {
                query::Definition::Operation(query::OperationDefinition::SelectionSet(set)) => {
                    selection_set = Some(set.into())
                }
                query::Definition::Operation(query::OperationDefinition::Query(q)) => {
                    selection_set = Some(q.selection_set.into())
                }
                query::Definition::Fragment(fragment) => {
                    let query::TypeCondition::On(type_condition) = fragment.type_condition;
                    fragments.push(FragmentDefinition {
                        name: fragment.name,
                        selection_set: fragment.selection_set.into(),
                        type_condition,
                    });
                }
                query::Definition::Operation(_) => {}
            }
        }
        ResponseShape::new(&Document {
            operation: OperationDefinition {
                name: None,
                operation_kind: None,
                selection_set: selection_set.expect("a query"),
                variable_definitions: None,
            },
            fragments,
        })
    }

    const QUERY: &str =
        "{ a b { c d } list { x y } u { __typename ... on A { a1 } ... on B { b1 } ...F } }
        fragment F on A { f }";

    /// Whatever the shaped parser does with a body, the response or the error, must be what
    /// sonic does with it.
    fn same_as_sonic(body: &[u8], paths: Option<&CustomScalarPaths>) {
        let shape = shape(QUERY);
        let bytes = Bytes::copy_from_slice(body);
        let shaped = SubgraphResponse::deserialize_with_shape(bytes.clone(), &shape, paths);
        let sonic = SubgraphResponse::deserialize_from_bytes(bytes, paths);
        assert_eq!(
            format!("{shaped:?}"),
            format!("{sonic:?}"),
            "{}",
            String::from_utf8_lossy(body)
        );
    }

    #[test]
    fn reads_what_sonic_reads() {
        let bodies: &[&str] = &[
            r#"{"data":{"a":1,"b":{"c":"x","d":null},"list":[{"x":1,"y":2},{"x":3,"y":4}]}}"#,
            // other order, missing and extra fields, nulls and nested lists
            r#"{"data":{"list":[{"y":3,"x":4,"z":{"k":[1]}},null,[{"x":5}]],"b":{"d":[]},"a":{"x":1}}}"#,
            // a field of one type, then of another, then a fragment's
            r#"{"data":{"u":[{"__typename":"B","b1":1},{"__typename":"A","a1":2,"f":3},{}]}}"#,
            r#"{"data":null,"errors":[{"message":"boom","path":["a",0],"extensions":{"code":"X"}}]}"#,
            r#"{"errors":[{"message":"only errors"}]}"#,
            r#"{"data":{},"extensions":{"x":[1,{"y":true}],"n":null}}"#,
            r#"{"hasNext":false,"data":{"a":1},"other":{"deep":[[[]]]}}"#,
            r#"{"data":{"a":{},"b":[],"list":[]}}"#,
            " \n\t{ \"data\" : { \"a\" : [ 1 , 2 ] , \"b\" : { } } } \r\n",
            r#"{"data":{"a":"\" \\ \/ \b \f \n \r \t \u00e9 \ud83d\ude00 end"}}"#,
            r#"{"data":{"a":"żółć 😀","b":"a\u0000b"}}"#,
            r#"{"data":{"a":[0,-1,1.5,-0,-0.0,1e10,1E-3,2.5e+3,0.1,123456789.123456789e-5]}}"#,
            r#"{"data":{"a":[18446744073709551615,18446744073709551616,-9223372036854775808,-9223372036854775809]}}"#,
            r#"{"data":{"a":[true,false,null]}}"#,
            // malformed, without data or errors
            r#"{"data":null}"#,
            r#"{}"#,
        ];
        let shape = shape(QUERY);
        for body in bodies {
            assert!(
                parse_response(body.as_bytes(), &shape, None).is_some(),
                "the shaped parser reads {body}"
            );
            same_as_sonic(body.as_bytes(), None);
        }
    }

    #[test]
    fn leaves_invalid_and_odd_bodies_to_sonic() {
        let bodies: Vec<Vec<u8>> = [
            r#"{"data":{"a":01}}"#,
            r#"{"data":{"a":-01}}"#,
            r#"{"data":{"a":1.}}"#,
            r#"{"data":{"a":.5}}"#,
            r#"{"data":{"a":-}}"#,
            r#"{"data":{"a":1e}}"#,
            r#"{"data":{"a":1e+}}"#,
            r#"{"data":{"a":+1}}"#,
            r#"{"data":{"a":[1,]}}"#,
            r#"{"data":{"a":1,}}"#,
            r#"{"data":{"a":tru}}"#,
            r#"{"data":{"a":nul}}"#,
            r#"{"data":{"a":"\x"}}"#,
            r#"{"data":{"a":"\ud800"}}"#,
            r#"{"data":{"a\n":1}}"#,
            r#"{"data":{"a":1}} x"#,
            r#"{"data":{"a":1}"#,
            r#"{"data":{"a" 1}}"#,
            r#"{"data":{a:1}}"#,
            r#"{"data":{"a":"unterminated}}"#,
            r#"{"data":{"a":"\"#,
            r#"{"data":{"a":1},"data":{}}"#,
            r#"{"errors":null}"#,
            r#"{"errors":[{"nope":1}]}"#,
            r#"[1]"#,
            r#""#,
        ]
        .iter()
        .map(|body| body.as_bytes().to_vec())
        .chain([
            b"{\"data\":{\"a\":\"raw \x01 control\"}}".to_vec(),
            b"{\"data\":{\"a\":\"bad \xff utf8\"}}".to_vec(),
        ])
        .collect();
        let shape = shape(QUERY);
        for body in &bodies {
            assert!(
                parse_response(body, &shape, None).is_none(),
                "{} goes to sonic",
                String::from_utf8_lossy(body)
            );
            same_as_sonic(body, None);
        }
    }

    #[test]
    fn leaves_deep_nesting_to_sonic() {
        let shape = shape(QUERY);
        let deep = |depth| {
            format!(
                "{{\"data\":{{\"a\":{}{}}}}}",
                "[".repeat(depth),
                "]".repeat(depth)
            )
        };
        assert!(parse_response(deep(MAX_DEPTH as usize).as_bytes(), &shape, None).is_some());
        assert!(parse_response(deep(200).as_bytes(), &shape, None).is_none());
    }

    #[test]
    fn keeps_custom_scalars_as_raw_json_like_sonic() {
        let mut paths = CustomScalarPaths::default();
        // one the shape knows, one inside list items, a whole list, and one it doesn't know
        paths.insert_path(["b", "c"]);
        paths.insert_path(["list", "y"]);
        paths.insert_path(["a"]);
        paths.insert_path(["meta"]);
        let shape = shape(QUERY);

        let read: &[&str] = &[
            r#"{"data":{"meta":{ "k" : [1, "x\n", null] },"b":{"c":"2024-01-01T00:00:00Z","d":1}}}"#,
            r#"{"data":{"list":[{"x":1,"y":{"any":true}},{"y":[1,2]},{"y":null},{"y":"s\"q"},null]}}"#,
            r#"{"data":{"a":[ 1 , {"z":2} ],"meta" :   -0.5e3  ,"b":{"c":true}}}"#,
            r#"{"data":{"a":null,"meta":"","b":null,"list":[]}}"#,
            r#"{"data":{"a":"żółć 😀","meta":{}}}"#,
        ];
        for body in read {
            assert!(
                parse_response(body.as_bytes(), &shape, Some(&paths)).is_some(),
                "the shaped parser reads {body}"
            );
            same_as_sonic(body.as_bytes(), Some(&paths));
        }

        let left_to_sonic: &[&str] = &[
            r#"{"data":{"meta":{"k":01}}}"#,
            r#"{"data":{"meta":[1,]}}"#,
            r#"{"data":{"meta":{"k\n":1}}}"#,
        ];
        for body in left_to_sonic {
            assert!(
                parse_response(body.as_bytes(), &shape, Some(&paths)).is_none(),
                "{body} goes to sonic"
            );
            same_as_sonic(body.as_bytes(), Some(&paths));
        }
    }
}
