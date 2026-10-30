//! A JSON parser for subgraph responses that knows which fields the fetch asked for.
//!
//! Subgraphs send fields in the order the query asks for them. So for every object we check the
//! next key we expect with one byte compare, instead of reading it as a string, and allocate
//! the object at its field count. An object of an abstract type that starts with `__typename`
//! (the planner puts it first) is read with the fields of its own type.
//!
//! The shape is only a hint. A key it doesn't expect gets looked up among the fields, and one it
//! doesn't know at all is parsed like any other JSON. Custom scalars are kept as raw JSON at the
//! fetch's `CustomScalarPaths`, the same as `PathAwareValueVisitor` does. Anything this parser isn't sure about
//! (invalid JSON, escaped keys, deep nesting) makes it give up, and the response goes through
//! sonic instead, so what we accept and the errors we return don't change.

use ahash::HashSet;
use std::ops::Range;

use crate::executor::response::{
    graphql_error::GraphQLError,
    value::{bytes_eq, Value},
};
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
    types: Vec<TypeFields>,
    data: Object,
}

#[derive(Debug, Clone)]
struct Node {
    /// The response key with its quotes, `"name"`, to compare against the input as is.
    quoted_key: Box<[u8]>,
    /// The object it holds. Empty for scalars.
    object: Object,
}

impl Node {
    fn key(&self) -> &[u8] {
        &self.quoted_key[1..self.quoted_key.len() - 1]
    }
}

#[derive(Debug, Clone, Default)]
struct Object {
    /// Fields under every type condition, in the order they first show up
    fields: Range<u32>,
    /// The fields of each type, sorted by name. Only for types whose type conditions select
    /// something else than `fields`.
    types: Range<u32>,
}

#[derive(Debug, Clone)]
struct TypeFields {
    name: Box<str>,
    fields: Range<u32>,
}

/// A field of an object and the selection sets of every place it's selected in.
struct Field<'d> {
    key: &'d str,
    sets: Vec<&'d SelectionSet>,
}

impl Field<'_> {
    fn same(a: &[Field], b: &[Field]) -> bool {
        a.len() == b.len()
            && a.iter().zip(b).all(|(a, b)| {
                a.key == b.key
                    && a.sets.len() == b.sets.len()
                    && a.sets
                        .iter()
                        .zip(&b.sets)
                        .all(|(a, b)| std::ptr::eq(*a, *b))
            })
    }
}

impl ResponseShape {
    /// `possible_types` gives the object types of an interface or a union, so an object knows
    /// which of its type conditions it matches.
    pub fn new<'s>(
        document: &Document,
        possible_types: impl Fn(&str) -> Option<&'s HashSet<String>>,
    ) -> Self {
        let mut builder = Builder {
            document,
            possible_types,
            shape: Self {
                nodes: Vec::new(),
                types: Vec::new(),
                data: Object::default(),
            },
        };
        builder.shape.data = builder.object(&[&document.operation.selection_set]);
        builder.shape
    }

    fn fields(&self, range: &Range<u32>) -> &[Node] {
        &self.nodes[range.start as usize..range.end as usize]
    }

    fn types(&self, range: &Range<u32>) -> &[TypeFields] {
        &self.types[range.start as usize..range.end as usize]
    }
}

struct Builder<'d, F> {
    document: &'d Document,
    possible_types: F,
    shape: ResponseShape,
}

impl<'d, 's, F: Fn(&str) -> Option<&'s HashSet<String>>> Builder<'d, F> {
    fn object(&mut self, sets: &[&'d SelectionSet]) -> Object {
        let merged = self.fields(sets, None);
        let fields = self.flatten(&merged);

        let mut names = Vec::new();
        for set in sets {
            self.conditions(set, &mut names);
        }
        // sorted, so the parser can binary search them
        names.sort_unstable();
        names.dedup();
        let mut lists: Vec<(Vec<Field>, Range<u32>)> = Vec::new();
        let mut types = Vec::new();
        for name in names {
            let own = self.fields(sets, Some(name));
            if Field::same(&own, &merged) {
                continue;
            }
            let fields = match lists.iter().find(|(list, _)| Field::same(list, &own)) {
                Some((_, fields)) => fields.clone(),
                None => {
                    let fields = self.flatten(&own);
                    lists.push((own, fields.clone()));
                    fields
                }
            };
            types.push(TypeFields {
                name: name.into(),
                fields,
            });
        }
        let start = self.shape.types.len() as u32;
        self.shape.types.extend(types);
        Object {
            fields,
            types: start..self.shape.types.len() as u32,
        }
    }

    /// The object types its type conditions match, interfaces and unions opened up.
    fn conditions<'x>(&self, set: &'d SelectionSet, out: &mut Vec<&'x str>)
    where
        'd: 'x,
        's: 'x,
    {
        for item in &set.items {
            let (condition, set) = match item {
                SelectionItem::Field(_) => continue,
                SelectionItem::InlineFragment(fragment) => {
                    (fragment.type_condition.as_str(), &fragment.selections)
                }
                SelectionItem::FragmentSpread(name) => {
                    match self.document.fragments.iter().find(|f| &f.name == name) {
                        Some(fragment) => {
                            (fragment.type_condition.as_str(), &fragment.selection_set)
                        }
                        None => continue,
                    }
                }
            };
            match (self.possible_types)(condition) {
                Some(types) => out.extend(types.iter().map(String::as_str)),
                None if !condition.is_empty() => out.push(condition),
                None => {}
            }
            self.conditions(set, out);
        }
    }

    fn applies(&self, condition: &str, typename: Option<&str>) -> bool {
        match typename {
            None => true,
            Some(typename) => {
                condition.is_empty()
                    || condition == typename
                    || (self.possible_types)(condition)
                        .is_some_and(|types| types.contains(typename))
            }
        }
    }

    /// The fields of an object of `typename`, or under every type condition when it's `None`.
    fn fields(&self, sets: &[&'d SelectionSet], typename: Option<&str>) -> Vec<Field<'d>> {
        let mut out = Vec::new();
        for set in sets {
            self.collect(set, typename, &mut out);
        }
        out
    }

    fn collect(&self, set: &'d SelectionSet, typename: Option<&str>, out: &mut Vec<Field<'d>>) {
        for item in &set.items {
            match item {
                SelectionItem::Field(field) => {
                    let key = field.selection_identifier();
                    let index = match out.iter().position(|f| f.key == key) {
                        Some(index) => index,
                        None => {
                            out.push(Field {
                                key,
                                sets: Vec::new(),
                            });
                            out.len() - 1
                        }
                    };
                    if !field.selections.items.is_empty() {
                        out[index].sets.push(&field.selections);
                    }
                }
                SelectionItem::InlineFragment(fragment) => {
                    if self.applies(&fragment.type_condition, typename) {
                        self.collect(&fragment.selections, typename, out)
                    }
                }
                SelectionItem::FragmentSpread(name) => {
                    if let Some(fragment) = self.document.fragments.iter().find(|f| &f.name == name)
                    {
                        if self.applies(&fragment.type_condition, typename) {
                            self.collect(&fragment.selection_set, typename, out)
                        }
                    }
                }
            }
        }
    }

    fn flatten(&mut self, fields: &[Field<'d>]) -> Range<u32> {
        let start = self.shape.nodes.len();
        self.shape.nodes.extend(fields.iter().map(|field| Node {
            quoted_key: format!("\"{}\"", field.key).into_bytes().into(),
            object: Object::default(),
        }));
        for (i, field) in fields.iter().enumerate() {
            if !field.sets.is_empty() {
                self.shape.nodes[start + i].object = self.object(&field.sets);
            }
        }
        start as u32..(start + fields.len()) as u32
    }
}

/// What we know about a value before reading it.
#[derive(Clone, Copy)]
struct Hint<'h> {
    shape: &'h ResponseShape,
    /// The fields its objects have
    fields: &'h [Node],
    types: &'h [TypeFields],
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
                        fields: shape.fields(&shape.data.fields),
                        types: shape.types(&shape.data.types),
                        scalars: custom_scalar_paths.filter(|paths| !paths.is_empty()),
                    })?;
                }
                "errors" if !std::mem::replace(&mut has_errors, true) => {
                    parser.whitespace();
                    let start = parser.pos;
                    parser.skip_value()?;
                    let errors = &input[start..parser.pos];
                    response.errors = Some(sonic_rs::from_slice(errors).ok()?);
                }
                "extensions" if !std::mem::replace(&mut has_extensions, true) => {
                    response.extensions = Some(parser.value()?);
                }
                "data" | "errors" | "extensions" => return None,
                _ => parser.skip_value()?,
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
        let mut nodes = hint.fields;
        // allocated after the first entry, which may say the object's type
        let mut entries = Vec::new();
        let mut next = 0;
        loop {
            self.whitespace();
            // usually the next field, or a later one when the ones between aren't there (another
            // type's fields, @skip)
            let ahead = nodes[next..].iter().position(|node| {
                self.input
                    .get(self.pos..self.pos + node.quoted_key.len())
                    .is_some_and(|key| bytes_eq(key, &node.quoted_key))
            });
            let (key, node) = match ahead {
                Some(ahead) => {
                    let node = &nodes[next + ahead];
                    next += ahead + 1;
                    let start = self.pos + 1;
                    self.pos += node.quoted_key.len();
                    // SAFETY: the input is valid UTF-8 and the key sits between two quotes
                    let key =
                        unsafe { std::str::from_utf8_unchecked(&self.input[start..self.pos - 1]) };
                    (key, Some(node))
                }
                None => {
                    let key = self.key()?;
                    let index = nodes[..next]
                        .iter()
                        .position(|node| bytes_eq(node.key(), key.as_bytes()));
                    // the fields after it most likely follow it
                    if let Some(index) = index {
                        next = index + 1;
                    }
                    (key, index.map(|index| &nodes[index]))
                }
            };
            self.expect(b':')?;
            let (fields, types) = node.map_or((&[][..], &[][..]), |node| {
                (
                    hint.shape.fields(&node.object.fields),
                    hint.shape.types(&node.object.types),
                )
            });
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
                    types,
                    scalars,
                })?
            };
            if entries.capacity() == 0 {
                // planned queries ask for __typename first, so an object of an abstract type can
                // read the rest with the fields of its own type
                if let ("__typename", Value::String(name)) = (key, &value) {
                    let types = hint.types;
                    if let Ok(index) = types.binary_search_by(|t| t.name.as_ref().cmp(name)) {
                        nodes = hint.shape.fields(&types[index].fields);
                        next = nodes
                            .iter()
                            .position(|node| node.key() == b"__typename")
                            .map_or(0, |index| index + 1);
                    }
                }
                entries.reserve_exact(nodes.len());
            }
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
        Some(Value::Object(self.entries.split_off(start)))
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
        Some(Value::Array(self.elements.split_off(start)))
    }

    /// Checks a value like `value` does, without building it.
    fn skip_value(&mut self) -> Option<()> {
        self.whitespace();
        let close = match self.peek()? {
            b'{' => b'}',
            b'[' => b']',
            _ => return self.value().map(drop),
        };
        self.pos += 1;
        if self.empty(close) {
            return Some(());
        }
        self.enter()?;
        loop {
            if close == b'}' {
                self.whitespace();
                self.key()?;
                self.expect(b':')?;
            }
            self.skip_value()?;
            if !self.more(close)? {
                break;
            }
        }
        self.depth -= 1;
        Some(())
    }

    /// A custom scalar, kept as the JSON it came as.
    fn raw(&mut self) -> Option<Value<'a>> {
        self.whitespace();
        let start = self.pos;
        self.skip_value()?;
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
        // read while scanning, it's only right up to 19 digits
        let mut magnitude = 0u64;
        // no leading zeros: `0` alone, or a number that starts with 1 to 9
        match self.peek()? {
            b'0' => self.pos += 1,
            b'1'..=b'9' => {
                while let Some(digit @ b'0'..=b'9') = self.peek() {
                    magnitude = magnitude
                        .wrapping_mul(10)
                        .wrapping_add((digit - b'0') as u64);
                    self.pos += 1;
                }
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
            let magnitude = if integer.len() <= 19 {
                Some(magnitude)
            } else {
                integer.iter().try_fold(0u64, |n, &digit| {
                    n.checked_mul(10)?.checked_add((digit - b'0') as u64)
                })
            };
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
    use ahash::{HashSet, HashSetExt};

    use bytes::Bytes;
    use graphql_tools::parser::query;

    use super::{parse_response, ResponseShape, MAX_DEPTH};
    use crate::executor::response::subgraph_response::SubgraphResponse;
    use crate::executor::response::value::Value;
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
        // `I` is an interface that A and C implement
        let mut interface = HashSet::new();
        interface.extend(["A".to_string(), "C".to_string()]);
        ResponseShape::new(
            &Document {
                operation: OperationDefinition {
                    name: None,
                    operation_kind: None,
                    selection_set: selection_set.expect("a query"),
                    variable_definitions: None,
                },
                fragments,
            },
            |name| (name == "I").then_some(&interface),
        )
    }

    const QUERY: &str = "{ a b { c d } list { x y }
        u { __typename ... on A { a1 n { x } } ... on B { b1 n { y } } ...F ... on I { i } } }
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
            // a field before one that came earlier in the shape
            r#"{"data":{"u":[{"b1":1,"__typename":"B","x":2,"a1":3}]}}"#,
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
            // the most digits read in one go, and ones checked on the way
            r#"{"data":{"a":[9999999999999999999,-9223372036854775807,12345678901234567890,99999999999999999999]}}"#,
            // skipped ones are still checked
            r#"{"data":{"a":1},"other":{"k":[1,{"x":"\n"}],"e":{}},"more":[]}"#,
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

    /// How many fields each object of `u` has room for.
    fn capacities(body: &str) -> Vec<usize> {
        same_as_sonic(body.as_bytes(), None);
        let shape = shape(QUERY);
        let response = parse_response(body.as_bytes(), &shape, None).expect("shaped");
        let Value::Object(data) = response.data else {
            panic!("data");
        };
        let Some((_, Value::Array(list))) = data.into_iter().find(|(key, _)| *key == "u") else {
            panic!("u");
        };
        list.into_iter()
            .map(|item| match item {
                Value::Object(fields) => fields.capacity(),
                _ => panic!("object"),
            })
            .collect()
    }

    #[test]
    fn reads_objects_with_the_fields_of_their_type() {
        // all of them: __typename a1 n b1 f i
        let all = 6;
        assert_eq!(
            capacities(
                r#"{"data":{"u":[{"__typename":"A","a1":1,"n":{"x":1},"f":2,"i":3},{"__typename":"B","b1":1,"n":{"y":2}},{"__typename":"C","i":1}]}}"#
            ),
            // A gets I's fields too, C only gets I's
            [5, 3, 2]
        );
        // a field of a type has the fields that type selected under it
        let body = r#"{"data":{"u":[{"__typename":"A","n":{"x":1}}]}}"#;
        let shape = shape(QUERY);
        let data = parse_response(body.as_bytes(), &shape, None)
            .expect("shaped")
            .data;
        let Value::Object(data) = data else { panic!() };
        let Value::Array(u) = &data[0].1 else {
            panic!()
        };
        let Value::Object(a) = &u[0] else { panic!() };
        let Value::Object(n) = &a[1].1 else { panic!() };
        assert_eq!(n.capacity(), 1);

        // __typename not first, a type the shape doesn't know, no __typename, not a string
        assert_eq!(
            capacities(
                r#"{"data":{"u":[{"a1":1,"__typename":"A"},{"__typename":"Z","z":1},{"b1":2},{"__typename":null,"i":1}]}}"#
            ),
            [all, all, all, all]
        );
        // a type that gets other fields than it asked for still reads them
        capacities(r#"{"data":{"u":[{"__typename":"B","a1":1,"z":{"k":[1]},"b1":2}]}}"#);
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
            r#"{"errors":[{"message":"\x"}]}"#,
            r#"{"data":{},"other":[01]}"#,
            r#"{"data":{},"other":{"k\n":1}}"#,
            r#"{"data":{},"other":{"k":1,}}"#,
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
