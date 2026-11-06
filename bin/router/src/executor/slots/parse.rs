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
    /// The body has `extensions`
    pub extensions: bool,
}

/// Reads `body` with `shape`. `None` when it isn't a valid response.
pub fn parse_body(
    arena: &mut Arena,
    layouts: &[Layout],
    shape: &Shape,
    body: &Bytes,
) -> Option<ParsedBody> {
    let index = arena.bodies.len() as u32;
    arena.bodies.push(body.clone());
    let scratch = arena.scratch.len();
    let mut parser = Parser {
        input: body,
        pos: 0,
        depth: 0,
        shape,
        layouts,
        arena,
        body: index,
    };
    let parsed = parser.response();
    // a failed parse can leave items behind
    parser.arena.scratch.truncate(scratch);
    parsed
}

const ONES: u64 = 0x0101_0101_0101_0101;
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
}

impl Parser<'_, '_> {
    fn response(&mut self) -> Option<ParsedBody> {
        let mut parsed = ParsedBody {
            data: Slot::Missing,
            errors: None,
            extensions: false,
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
                    b"extensions" if !parsed.extensions => {
                        parsed.extensions = true;
                        self.skip_value()?;
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
                    let value = if node.raw {
                        self.whitespace();
                        let start = self.pos;
                        self.skip_value()?;
                        self.raw_slot(start)
                    } else if node.verify != NO_OBJECT {
                        self.passthrough(node.verify, node.object)?
                    } else if node.object != NO_OBJECT {
                        self.shaped(node.object)?
                    } else {
                        self.leaf()?
                    };
                    if node.slot != NO_SLOT {
                        self.arena.slots[(first + node.slot as u32) as usize] = value;
                    }
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

    /// A value the client may get as the subgraph sent it (see `compile::Verify`): kept as
    /// one raw range when it checks out, read the normal way when it doesn't.
    fn passthrough(&mut self, verify: u32, object: u32) -> Option<Slot> {
        self.whitespace();
        let (start, depth) = (self.pos, self.depth);
        let shape = self.shape;
        let verify = &shape.verifies[verify as usize];
        if self.peek() != Some(b'n')
            && self
                .check(&verify.shape, verify.object, verify.enum_values.as_deref())
                .is_some()
        {
            return Some(self.raw_slot(start));
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
                    self.check(&nullability[1..], object, enum_values)?;
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
                    self.whitespace();
                    let node = &shape.nodes[*node as usize];
                    if !self.starts_with_key(word(self.input, self.pos), node) {
                        return None;
                    }
                    self.pos += node.key_len as usize;
                    self.expect(b':')?;
                    self.check(&verify.shape, verify.object, verify.enum_values.as_deref())?;
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
        let mut steps = 2;
        loop {
            if steps > 0 {
                steps -= 1;
                let chunk = word(self.input, end);
                non_ascii |= chunk & HIGH != 0;
                let at = special(chunk);
                end += at;
                if at == 8 {
                    continue;
                }
            } else {
                let from = end;
                end += memchr::memchr2(b'"', b'\\', self.input.get(end..)?)?;
                // a min/max reduction, which the compiler vectorizes (a loop of `|=` on bools
                // it didn't)
                let (low, high) = self.input[from..end]
                    .iter()
                    .fold((u8::MAX, 0u8), |(low, high), &byte| {
                        (low.min(byte), high.max(byte))
                    });
                if low < 0x20 {
                    return None;
                }
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
