//! Reads a subgraph response into slots. The same loop as `response::shaped`, but nothing is
//! decoded or allocated: a scalar becomes the byte range the subgraph wrote, an object a block
//! of slots of its position's layout, a list a run of `items`.
//!
//! It checks the JSON like the shaped parser does (number grammar, string escapes, control
//! characters, UTF-8), so it never forwards something that isn't JSON. When it rejects a body,
//! the caller asks sonic for the error, so the message stays the one clients get today.

use bytes::Bytes;

use super::compile::{Layout, Shape, ShapeNode, NO_OBJECT, NO_SLOT};
use super::store::{Arena, Raw, Slot};
use crate::executor::response::graphql_error::GraphQLError;

/// Number of upcoming fields to check before searching all of them, see `shaped::LOOKAHEAD`
const LOOKAHEAD: usize = 4;
/// sonic stops at 256 levels. A body deeper than this is rejected, and sonic says why.
const MAX_DEPTH: u32 = 250;

pub struct ParsedBody {
    /// `data`, `Missing` when the body has none
    pub data: Slot,
    pub errors: Option<Vec<GraphQLError>>,
    /// Where the body's `extensions` value is, `start..end`
    pub extensions: Option<(u32, u32)>,
}

/// Reads `body` with `shape`. `None` when it isn't a valid response. Without `passthrough`,
/// every value goes into slots, also where the client would get a subtree as it came (demand
/// control counts the lists in it).
pub fn parse_body(
    arena: &mut Arena,
    layouts: &[Layout],
    shape: &Shape,
    body: &Bytes,
    passthrough: bool,
) -> Option<ParsedBody> {
    let index = arena.bodies.len() as u32;
    arena.bodies.push(body.clone());
    arena.failed.clear();
    arena.failed.resize(shape.verifies.len(), false);
    let scratch = arena.scratch.len();
    let mut parser = Parser {
        input: body,
        pos: 0,
        depth: 0,
        shape,
        layouts,
        arena,
        body: index,
        passthrough,
    };
    let parsed = parser.response();
    // a failed parse can leave items behind
    parser.arena.scratch.truncate(scratch);
    parsed
}

/// The body of an aliased context request (see `ContextAliasTemplate`): `data` has a key per
/// response key of the fetch and group, like `_entities_3`.
pub struct AliasedBody {
    /// Each value under an aliased key: the slot of its response key in `data`'s layout, its
    /// group, the value
    pub values: Vec<(u32, usize, Slot)>,
    /// `data` was null (or missing)
    pub no_data: bool,
    pub errors: Option<Vec<GraphQLError>>,
    /// See [`ParsedBody::extensions`]
    pub extensions: Option<(u32, u32)>,
}

/// Reads an aliased context request's body: `alias` maps a key of `data` to the index of its
/// response key's node among `data`'s nodes in `shape`, and its group. Other keys are skipped.
/// `None` when it isn't a valid response.
pub fn parse_aliased_body(
    arena: &mut Arena,
    layouts: &[Layout],
    shape: &Shape,
    body: &Bytes,
    passthrough: bool,
    alias: impl Fn(&str) -> Option<(u32, usize)>,
) -> Option<AliasedBody> {
    let index = arena.bodies.len() as u32;
    arena.bodies.push(body.clone());
    arena.failed.clear();
    arena.failed.resize(shape.verifies.len(), false);
    let scratch = arena.scratch.len();
    let mut parser = Parser {
        input: body,
        pos: 0,
        depth: 0,
        shape,
        layouts,
        arena,
        body: index,
        passthrough,
    };
    let parsed = parser.aliased_response(&alias);
    parser.arena.scratch.truncate(scratch);
    parsed
}

#[cfg(not(target_arch = "aarch64"))]
const ONES: u64 = 0x0101_0101_0101_0101;
#[cfg(not(target_arch = "aarch64"))]
const HIGH: u64 = 0x8080_8080_8080_8080;

#[inline(always)]
fn word(input: &[u8], pos: usize) -> u64 {
    match input.get(pos..pos + 8) {
        Some(bytes) => u64::from_le_bytes(bytes.try_into().expect("8 bytes")),
        None => {
            let mut first = [0; 8];
            let rest = input.get(pos..).unwrap_or_default();
            first[..rest.len()].copy_from_slice(rest);
            u64::from_le_bytes(first)
        }
    }
}

/// The index of the first quote, backslash or control byte in the chunk, 8 when none.
#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
fn special(chunk: u64) -> usize {
    let zero = |bytes: u64| bytes.wrapping_sub(ONES) & !bytes & HIGH;
    let quote = zero(chunk ^ (ONES * b'"' as u64));
    let backslash = zero(chunk ^ (ONES * b'\\' as u64));
    let control = chunk.wrapping_sub(ONES * 0x20) & !chunk & HIGH;
    ((quote | backslash | control).trailing_zeros() / 8) as usize
}

struct Parser<'a, 'p> {
    input: &'a [u8],
    pos: usize,
    depth: u32,
    shape: &'p Shape,
    layouts: &'p [Layout],
    arena: &'p mut Arena,
    body: u32,
    passthrough: bool,
}

impl Parser<'_, '_> {
    fn response(&mut self) -> Option<ParsedBody> {
        let mut parsed = ParsedBody {
            data: Slot::Missing,
            errors: None,
            extensions: None,
        };
        let (mut data, mut errors) = (false, false);
        self.whitespace();
        if self.peek() != Some(b'{') {
            return None;
        }
        self.pos += 1;
        if !self.empty(b'}') {
            loop {
                self.whitespace();
                let (start, end) = self.key()?;
                self.expect(b':')?;
                match &self.input[start..end] {
                    b"data" if !data => {
                        data = true;
                        parsed.data = self.shaped(0)?;
                    }
                    b"errors" if !errors => {
                        errors = true;
                        self.whitespace();
                        let start = self.pos;
                        self.skip_value()?;
                        // rare, and `GraphQLError` has its own serde rules: sonic reads them
                        parsed.errors =
                            Some(sonic_rs::from_slice(&self.input[start..self.pos]).ok()?);
                    }
                    b"extensions" if parsed.extensions.is_none() => {
                        self.whitespace();
                        let start = self.pos as u32;
                        self.skip_value()?;
                        parsed.extensions = Some((start, self.pos as u32));
                    }
                    b"data" | b"errors" | b"extensions" => return None,
                    _ => self.skip_value()?,
                }
                if !self.more(b'}')? {
                    break;
                }
            }
        }
        self.whitespace();
        (self.pos == self.input.len()).then_some(parsed)
    }

    /// [`Self::response`] for an aliased context request, see [`parse_aliased_body`]
    fn aliased_response(
        &mut self,
        alias: &impl Fn(&str) -> Option<(u32, usize)>,
    ) -> Option<AliasedBody> {
        let mut parsed = AliasedBody {
            values: Vec::new(),
            no_data: true,
            errors: None,
            extensions: None,
        };
        let (mut data, mut errors) = (false, false);
        self.whitespace();
        if self.peek() != Some(b'{') {
            return None;
        }
        self.pos += 1;
        if !self.empty(b'}') {
            loop {
                self.whitespace();
                let (start, end) = self.key()?;
                self.expect(b':')?;
                match &self.input[start..end] {
                    b"data" if !data => {
                        data = true;
                        self.whitespace();
                        if self.peek() == Some(b'n') {
                            self.literal(b"null", ())?;
                        } else {
                            parsed.no_data = false;
                            self.aliased_data(alias, &mut parsed.values)?;
                        }
                    }
                    b"errors" if !errors => {
                        errors = true;
                        self.whitespace();
                        let start = self.pos;
                        self.skip_value()?;
                        parsed.errors =
                            Some(sonic_rs::from_slice(&self.input[start..self.pos]).ok()?);
                    }
                    b"extensions" if parsed.extensions.is_none() => {
                        self.whitespace();
                        let start = self.pos as u32;
                        self.skip_value()?;
                        parsed.extensions = Some((start, self.pos as u32));
                    }
                    b"data" | b"errors" | b"extensions" => return None,
                    _ => self.skip_value()?,
                }
                if !self.more(b'}')? {
                    break;
                }
            }
        }
        self.whitespace();
        (self.pos == self.input.len()).then_some(parsed)
    }

    fn aliased_data(
        &mut self,
        alias: &impl Fn(&str) -> Option<(u32, usize)>,
        values: &mut Vec<(u32, usize, Slot)>,
    ) -> Option<()> {
        self.expect(b'{')?;
        if self.empty(b'}') {
            return Some(());
        }
        self.enter()?;
        let shape = self.shape;
        let (nodes_start, _) = shape.objects[0].nodes;
        loop {
            self.whitespace();
            let (start, end) = self.key()?;
            self.expect(b':')?;
            let key = std::str::from_utf8(&self.input[start..end]).ok()?;
            match alias(key) {
                Some((node, group)) => {
                    let node = shape.nodes[(nodes_start + node) as usize];
                    let value = if node.object != NO_OBJECT {
                        self.shaped(node.object)?
                    } else {
                        self.leaf()?
                    };
                    values.push((node.slot as u32, group, value));
                }
                None => self.skip_value()?,
            }
            if !self.more(b'}')? {
                self.depth -= 1;
                return Some(());
            }
        }
    }

    #[inline]
    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    #[inline]
    fn peek_at(&self, pos: usize) -> Option<u8> {
        self.input.get(pos).copied()
    }

    #[inline]
    fn whitespace(&mut self) {
        while let Some(b' ' | b'\n' | b'\r' | b'\t') = self.peek() {
            self.pos += 1;
        }
    }

    #[inline]
    fn expect(&mut self, byte: u8) -> Option<()> {
        if self.peek() != Some(byte) {
            self.whitespace();
            if self.peek() != Some(byte) {
                return None;
            }
        }
        self.pos += 1;
        Some(())
    }

    #[inline]
    fn more(&mut self, close: u8) -> Option<bool> {
        let mut byte = self.peek()?;
        if byte != b',' && byte != close {
            self.whitespace();
            byte = self.peek()?;
        }
        self.pos += 1;
        match byte {
            b',' => Some(true),
            _ if byte == close => Some(false),
            _ => None,
        }
    }

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

    #[inline]
    fn raw_slot(&self, start: usize) -> Slot {
        Slot::Raw(Raw {
            body: self.body,
            start: start as u32,
            len: (self.pos - start) as u32,
        })
    }

    /// A value where the query has no object: a scalar, or a list of them
    fn leaf(&mut self) -> Option<Slot> {
        self.whitespace();
        let start = self.pos;
        match self.peek()? {
            b'"' => {
                self.string()?;
                Some(self.raw_slot(start))
            }
            b'[' => self.list(NO_OBJECT),
            // an object where a scalar was asked for: kept as it came
            b'{' => {
                self.skip_value()?;
                Some(self.raw_slot(start))
            }
            b't' => self.literal(b"true", Slot::True),
            b'f' => self.literal(b"false", Slot::False),
            b'n' => self.literal(b"null", Slot::Null),
            b'-' | b'0'..=b'9' => {
                self.number()?;
                Some(self.raw_slot(start))
            }
            _ => None,
        }
    }

    /// A value we expect to be an object of `object` in the shape, or a list of them.
    fn shaped(&mut self, object: u32) -> Option<Slot> {
        self.whitespace();
        match self.peek()? {
            b'{' => self.shaped_object(object),
            b'[' => self.list(object),
            _ => self.leaf(),
        }
    }

    fn shaped_object(&mut self, object: u32) -> Option<Slot> {
        self.pos += 1;
        let shape = self.shape;
        let object = &shape.objects[object as usize];
        let layout = object.layout;
        let first = self.arena.object(self.layouts[layout as usize].width());
        let result = Slot::Object { layout, first };
        if self.empty(b'}') {
            return Some(result);
        }
        self.enter()?;
        let (start, end) = object.nodes;
        let nodes = &shape.nodes[start as usize..end as usize];
        let mut next = 0usize;
        loop {
            // compact JSON, fields in the order asked: the next key and its colon are here
            if let Some(node) = nodes.get(next).filter(|node| self.at_key_colon(node)) {
                self.pos += node.key_len as usize + 1;
                next += 1;
                self.field(node, first)?;
                if !self.more(b'}')? {
                    self.depth -= 1;
                    return Some(result);
                }
                continue;
            }
            self.whitespace();
            let word = word(self.input, self.pos);
            let found = if next < nodes.len() && self.starts_with_key(word, &nodes[next]) {
                Some(next)
            } else {
                let ahead = next + 1..(next + LOOKAHEAD).min(nodes.len());
                nodes
                    .get(ahead.clone())
                    .and_then(|run| run.iter().position(|n| self.starts_with_key(word, n)))
                    .map(|i| i + ahead.start)
                    .or_else(|| nodes.iter().position(|n| self.starts_with_key(word, n)))
            };
            match found {
                Some(index) => {
                    let node = nodes[index];
                    next = index + 1;
                    self.pos += node.key_len as usize;
                    self.expect(b':')?;
                    self.field(&node, first)?;
                }
                None => {
                    // a key we didn't ask for
                    self.key()?;
                    self.expect(b':')?;
                    self.skip_value()?;
                }
            }
            if !self.more(b'}')? {
                self.depth -= 1;
                return Some(result);
            }
        }
    }

    /// Reads a field's value, after its key, into its slot of the object at `first`
    #[inline]
    fn field(&mut self, node: &ShapeNode, first: u32) -> Option<()> {
        let value = if node.raw {
            self.whitespace();
            let start = self.pos;
            self.skip_value()?;
            self.raw_slot(start)
        } else if node.verify != NO_OBJECT && self.passthrough {
            self.passthrough(node.verify, node.object)?
        } else if node.object != NO_OBJECT {
            self.shaped(node.object)?
        } else {
            self.leaf()?
        };
        if node.slot != NO_SLOT {
            self.arena.slots[(first + node.slot as u32) as usize] = value;
        }
        Some(())
    }

    /// A value the client may get as the subgraph sent it (see `compile::Verify`): kept as
    /// one raw range when it checks out, read the normal way when it doesn't.
    fn passthrough(&mut self, verify: u32, object: u32) -> Option<Slot> {
        // a check that failed once in this body likely fails for the next item of the list
        // too (a subgraph that sends another key order, say): don't read every one twice
        if self.arena.failed[verify as usize] {
            return self.shaped(object);
        }
        self.whitespace();
        let (start, depth) = (self.pos, self.depth);
        let shape = self.shape;
        let index = verify;
        let verify = &shape.verifies[verify as usize];
        if self.peek() != Some(b'n')
            && self
                .check(
                    &verify.shape,
                    verify.object,
                    verify.enum_values.as_deref(),
                    verify.any,
                )
                .is_some()
        {
            return Some(self.raw_slot(start));
        }
        if self.peek_at(start) != Some(b'n') {
            self.arena.failed[index as usize] = true;
        }
        self.pos = start;
        self.depth = depth;
        self.shaped(object)
    }

    /// Checks a passthrough value: valid JSON, an object has exactly the expected keys in
    /// order, and no null where the type is non-null. Keeps nothing.
    fn check(
        &mut self,
        nullability: &[u8],
        object: u32,
        enum_values: Option<&[Box<str>]>,
        any: bool,
    ) -> Option<()> {
        // see `ShapeFlags`
        const LIST: u8 = 1;
        const NON_NULL: u8 = 2;
        self.whitespace();
        let flags = nullability.first().copied().unwrap_or(0);
        match self.peek()? {
            b'n' => {
                self.literal(b"null", ())?;
                (flags & NON_NULL == 0).then_some(())
            }
            // a custom scalar can be any JSON, a list or an object too
            _ if any => self.skip_value(),
            b'[' => {
                if flags & LIST == 0 {
                    return None;
                }
                self.pos += 1;
                if self.empty(b']') {
                    return Some(());
                }
                self.enter()?;
                loop {
                    self.check(&nullability[1..], object, enum_values, false)?;
                    if !self.more(b']')? {
                        break;
                    }
                }
                self.depth -= 1;
                Some(())
            }
            b'"' if enum_values.is_some() => {
                let start = self.pos + 1;
                let (end, escaped) = self.string_end()?;
                // an escaped enum value is rare: let the normal path decode and check it
                if escaped {
                    return None;
                }
                let value = &self.input[start..end];
                enum_values?
                    .iter()
                    .any(|allowed| allowed.as_bytes() == value)
                    .then_some(())
            }
            b'{' if object != NO_OBJECT => {
                self.pos += 1;
                self.enter()?;
                let shape = self.shape;
                let fields = &shape.verify_objects[object as usize].fields;
                for (index, (node, verify)) in fields.iter().enumerate() {
                    let node = &shape.nodes[*node as usize];
                    if self.at_key_colon(node) {
                        // compact JSON: the key and the colon in one compare
                        self.pos += node.key_len as usize + 1;
                    } else {
                        self.whitespace();
                        if !self.starts_with_key(word(self.input, self.pos), node) {
                            return None;
                        }
                        self.pos += node.key_len as usize;
                        self.expect(b':')?;
                    }
                    if verify.object == NO_OBJECT
                        && verify.enum_values.is_none()
                        && self.peek() == Some(b'"')
                    {
                        // most values are plain strings: straight to the scan
                        self.string()?;
                    } else {
                        self.check(
                            &verify.shape,
                            verify.object,
                            verify.enum_values.as_deref(),
                            verify.any,
                        )?;
                    }
                    // a key after the last one, or the end before it
                    if self.more(b'}')? == (index + 1 == fields.len()) {
                        return None;
                    }
                }
                self.depth -= 1;
                Some(())
            }
            _ => self.skip_value(),
        }
    }

    /// Whether the input at `pos` is the node's `"key":`
    #[inline(always)]
    fn at_key_colon(&self, node: &ShapeNode) -> bool {
        if node.key_colon == 0 {
            return false;
        }
        let Some(bytes) = self.input.get(self.pos..self.pos + 16) else {
            return false;
        };
        let word = u128::from_le_bytes(bytes.try_into().expect("16 bytes"));
        let len = node.key_len as usize + 1;
        word & (u128::MAX >> (128 - 8 * len)) == node.key_colon
    }

    /// Whether the input at `pos` starts with the node's quoted key, see
    /// `ResponseShape::starts_with_key`.
    #[inline(always)]
    fn starts_with_key(&self, word: u64, node: &ShapeNode) -> bool {
        let len = node.key_len as usize;
        if len <= 8 {
            return word & (u64::MAX >> (64 - 8 * len)) == node.prefix;
        }
        word == node.prefix
            && self
                .input
                .get(self.pos + 8..self.pos + len)
                .is_some_and(|rest| rest == &self.shape.key(node)[8..])
    }

    fn list(&mut self, object: u32) -> Option<Slot> {
        self.pos += 1;
        if self.empty(b']') {
            return Some(Slot::List { start: 0, len: 0 });
        }
        self.enter()?;
        let mark = self.arena.scratch.len();
        loop {
            let value = if object != NO_OBJECT {
                self.shaped(object)?
            } else {
                self.leaf()?
            };
            self.arena.scratch.push(value);
            if !self.more(b']')? {
                break;
            }
        }
        self.depth -= 1;
        let start = self.arena.items.len() as u32;
        let len = (self.arena.scratch.len() - mark) as u32;
        let arena = &mut *self.arena;
        arena.items.extend_from_slice(&arena.scratch[mark..]);
        arena.scratch.truncate(mark);
        Some(Slot::List { start, len })
    }

    /// Checks a value without keeping it.
    fn skip_value(&mut self) -> Option<()> {
        self.whitespace();
        let close = match self.peek()? {
            b'{' => b'}',
            b'[' => b']',
            b'"' => return self.string(),
            b't' => return self.literal(b"true", ()),
            b'f' => return self.literal(b"false", ()),
            b'n' => return self.literal(b"null", ()),
            b'-' | b'0'..=b'9' => return self.number(),
            _ => return None,
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

    /// A key, as the range between its quotes. A key with escapes is checked like any string;
    /// it never matches a field of the shape (those have none), so it's skipped or kept raw.
    /// (The shaped parser gives up on one and lets sonic read the whole body.)
    fn key(&mut self) -> Option<(usize, usize)> {
        if self.peek() != Some(b'"') {
            return None;
        }
        let start = self.pos + 1;
        let (end, escaped) = self.string_end()?;
        if escaped {
            check_escapes(&self.input[start..end])?;
        }
        Some((start, end))
    }

    /// Checks a string and moves past it.
    #[inline]
    fn string(&mut self) -> Option<()> {
        let start = self.pos + 1;
        let (end, escaped) = self.string_end()?;
        if escaped {
            check_escapes(&self.input[start..end])?;
        }
        Some(())
    }

    /// Finds the end of the string at `pos` and moves past its closing quote. Returns where
    /// the closing quote is, and whether there are escapes. Checks control characters and
    /// UTF-8, the same way `shaped::Parser::raw_string` does: two 8 byte steps for short
    /// strings, then memchr and a check pass the compiler vectorizes. (A single 8 byte SWAR
    /// pass for every length was 7–12% slower on the bench op and ct.)
    fn string_end(&mut self) -> Option<(usize, bool)> {
        let start = self.pos + 1;
        let mut end = start;
        let mut escaped = false;
        let mut non_ascii = false;
        let mut first = true;
        loop {
            if first {
                // most strings are short: one 16 byte step, then the long scan
                first = false;
                let (at, high) = chunk16(self.input, end);
                non_ascii |= high;
                end += at;
                if at == 16 {
                    continue;
                }
            } else {
                let mut high = 0;
                end = scan(self.input, end, &mut high)?;
                non_ascii |= high >= 0x80;
            }
            match *self.input.get(end)? {
                b'"' => break,
                b'\\' => {
                    escaped = true;
                    end += 2;
                }
                _ => return None,
            }
        }
        if (non_ascii || escaped) && std::str::from_utf8(&self.input[start..end]).is_err() {
            return None;
        }
        self.pos = end + 1;
        Some((end, escaped))
    }

    fn literal<T>(&mut self, word: &[u8], value: T) -> Option<T> {
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

    /// Checks a number and moves past it. Like the shaped parser, it rejects one that doesn't
    /// fit in an f64.
    fn number(&mut self) -> Option<()> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek()? {
            b'0' => self.pos += 1,
            b'1'..=b'9' => {
                self.digits();
            }
            _ => return None,
        }
        let integer_digits = self.pos - start;
        let mut exponent = false;
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if self.digits() == 0 {
                return None;
            }
        }
        if let Some(b'e' | b'E') = self.peek() {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return None;
            }
            exponent = true;
        }
        // only an exponent or a very long integer part can go past f64::MAX
        if exponent || integer_digits > 300 {
            // SAFETY: only ASCII digits, signs, a dot and an exponent
            let text = unsafe { std::str::from_utf8_unchecked(&self.input[start..self.pos]) };
            let float: f64 = text.parse().ok()?;
            if !float.is_finite() {
                return None;
            }
        }
        Some(())
    }
}

/// Where the first quote, backslash or control byte is in the 16 bytes at `pos` (16 when
/// none), and whether any byte before it is non-ASCII. Past the end of the input reads as a
/// control byte.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn chunk16(input: &[u8], pos: usize) -> (usize, bool) {
    use std::arch::aarch64::*;
    let Some(bytes) = input.get(pos..pos + 16) else {
        let mut padded = [0u8; 16];
        let rest = input.get(pos..).unwrap_or_default();
        padded[..rest.len()].copy_from_slice(rest);
        return chunk16(&padded, 0);
    };
    // SAFETY: 16 bytes in bounds
    unsafe {
        let v = vld1q_u8(bytes.as_ptr());
        let special = vorrq_u8(
            vorrq_u8(
                vceqq_u8(v, vdupq_n_u8(b'"')),
                vceqq_u8(v, vdupq_n_u8(b'\\')),
            ),
            vcltq_u8(v, vdupq_n_u8(0x20)),
        );
        let found = nibbles(special);
        let high = nibbles(vcgeq_u8(v, vdupq_n_u8(0x80)));
        if found == 0 {
            return (16, high != 0);
        }
        let zeros = found.trailing_zeros();
        // the high bits of the bytes before the special one
        (zeros as usize / 4, high & ((1u64 << zeros) - 1) != 0)
    }
}

/// 4 bits per byte of a mask, see the ARM blog post "Porting x86 vector bitmask optimizations
/// to Arm NEON"
#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn nibbles(mask: std::arch::aarch64::uint8x16_t) -> u64 {
    use std::arch::aarch64::*;
    vget_lane_u64::<0>(vreinterpret_u64_u8(vshrn_n_u16::<4>(vreinterpretq_u16_u8(
        mask,
    ))))
}

#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
fn chunk16(input: &[u8], pos: usize) -> (usize, bool) {
    let a = word(input, pos);
    let at = special(a);
    if at < 8 {
        return (at, a & HIGH != 0);
    }
    let b = word(input, pos + 8);
    (8 + special(b), (a | b) & HIGH != 0)
}

/// The first quote, backslash or control byte at or after `pos`, in one pass. ORs the bytes
/// it looked at into `high`, for the non-ASCII check; that can take in a few bytes past the
/// one it found, which only means the UTF-8 check runs when it didn't have to.
/// (`memchr2` and a separate check pass read every byte twice: 4% slower on ct.)
#[cfg(target_arch = "aarch64")]
#[inline]
fn scan(input: &[u8], mut pos: usize, high: &mut u8) -> Option<usize> {
    use std::arch::aarch64::*;
    // a lane per byte, 0xff where the byte is special
    #[inline(always)]
    unsafe fn specials(v: uint8x16_t) -> uint8x16_t {
        vorrq_u8(
            vorrq_u8(
                vceqq_u8(v, vdupq_n_u8(b'"')),
                vceqq_u8(v, vdupq_n_u8(b'\\')),
            ),
            vcltq_u8(v, vdupq_n_u8(0x20)),
        )
    }
    // SAFETY: every load reads 16 bytes that are in bounds
    unsafe {
        let mut acc = vdupq_n_u8(0);
        while pos + 32 <= input.len() {
            let a = vld1q_u8(input.as_ptr().add(pos));
            let b = vld1q_u8(input.as_ptr().add(pos + 16));
            acc = vorrq_u8(acc, vorrq_u8(a, b));
            let (ma, mb) = (specials(a), specials(b));
            // a narrowing shift makes the mask one u64: cheaper to test than a max across lanes
            if nibbles(vorrq_u8(ma, mb)) != 0 {
                *high |= vmaxvq_u8(acc);
                let found = nibbles(ma);
                return Some(if found != 0 {
                    pos + (found.trailing_zeros() / 4) as usize
                } else {
                    pos + 16 + (nibbles(mb).trailing_zeros() / 4) as usize
                });
            }
            pos += 32;
        }
        *high |= vmaxvq_u8(acc);
    }
    scan_tail(input, pos, high)
}

#[cfg(not(target_arch = "aarch64"))]
#[inline]
fn scan(input: &[u8], pos: usize, high: &mut u8) -> Option<usize> {
    let end = pos + memchr::memchr2(b'"', b'\\', input.get(pos..)?)?;
    let (low, max) = input[pos..end]
        .iter()
        .fold((u8::MAX, 0u8), |(low, high), &byte| {
            (low.min(byte), high.max(byte))
        });
    *high |= max;
    if low < 0x20 {
        // a control byte before the end: point at it, the caller rejects it
        return input[pos..end]
            .iter()
            .position(|&b| b < 0x20)
            .map(|at| pos + at);
    }
    Some(end)
}

#[cfg(target_arch = "aarch64")]
fn scan_tail(input: &[u8], mut pos: usize, high: &mut u8) -> Option<usize> {
    while let Some(&byte) = input.get(pos) {
        if byte == b'"' || byte == b'\\' || byte < 0x20 {
            return Some(pos);
        }
        *high |= byte;
        pos += 1;
    }
    None
}

/// Checks the escapes of a string, without the quotes: each is one of `"\/bfnrt` or `u` and
/// four hex digits.
fn check_escapes(mut bytes: &[u8]) -> Option<()> {
    while let Some(at) = memchr::memchr(b'\\', bytes) {
        match bytes.get(at + 1)? {
            b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => bytes = &bytes[at + 2..],
            b'u' => {
                let hex = bytes.get(at + 2..at + 6)?;
                if !hex.iter().all(u8::is_ascii_hexdigit) {
                    return None;
                }
                bytes = &bytes[at + 6..];
            }
            _ => return None,
        }
    }
    Some(())
}
