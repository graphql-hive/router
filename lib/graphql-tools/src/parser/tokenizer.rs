use std::fmt::{self};

use combine::easy::{Error, Errors, Info};
use combine::error::StreamError;
use combine::stream::ResetStream;
use combine::{Positioned, StreamOnce};

use super::position::Pos;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Kind {
    Punctuator,
    Name,
    IntValue,
    FloatValue,
    StringValue,
    BlockString,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Token<'a> {
    pub kind: Kind,
    pub value: &'a str,
}

#[derive(Debug, PartialEq)]
pub struct TokenStream<'a> {
    buf: &'a str,
    state: StreamState,
    cached_token: Option<CachedToken<'a>>,
    token_limit: Option<usize>,
}

impl<'a> TokenStream<'a> {
    #[inline]
    pub(crate) fn offset(&self) -> usize {
        self.state.offset
    }

    #[inline(always)]
    pub(crate) fn peek_token(&mut self) -> Result<Option<Token<'a>>, Error<Token<'a>, Token<'a>>> {
        Ok(self.cached_token()?.map(|cached| cached.token))
    }

    #[inline(always)]
    pub(crate) fn next_token_without_positions(
        &mut self,
    ) -> Result<Token<'a>, Error<Token<'a>, Token<'a>>> {
        let Some((token, after)) = self.scan_token_impl::<false>()? else {
            return Err(Error::end_of_input());
        };
        self.state = after;
        Ok(token)
    }
}

/// Byte offsets index the source; columns count characters according to the
/// parser's established conventions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StreamState {
    offset: usize,
    position: Pos,
    recursion_limit: usize,
    token_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CacheKey {
    // Position is determined by the immutable source prefix at `offset`; the
    // cached after-state still carries it for fast commit.
    offset: usize,
    recursion_limit: usize,
    token_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CachedToken<'a> {
    before: CacheKey,
    token: Token<'a>,
    after: StreamState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    state: StreamState,
}

impl<'a> StreamOnce for TokenStream<'a> {
    type Token = Token<'a>;
    type Range = Token<'a>;
    type Position = Pos;
    type Error = Errors<Token<'a>, Token<'a>, Pos>;

    #[inline(always)]
    fn uncons(&mut self) -> Result<Self::Token, Error<Token<'a>, Token<'a>>> {
        if let Some(cached) = self.cached_token {
            if cached.before == self.cache_key() {
                self.cached_token = None;
                self.state = cached.after;
                return Ok(cached.token);
            }
        }

        let Some((token, after)) = self.scan_token_direct()? else {
            self.cached_token = None;
            return Err(Error::end_of_input());
        };
        self.state = after;
        Ok(token)
    }
}

impl<'a> TokenStream<'a> {
    #[inline(always)]
    fn cache_key(&self) -> CacheKey {
        CacheKey {
            offset: self.state.offset,
            recursion_limit: self.state.recursion_limit,
            token_count: self.state.token_count,
        }
    }

    #[inline(always)]
    fn cached_token(&mut self) -> Result<Option<CachedToken<'a>>, Error<Token<'a>, Token<'a>>> {
        if let Some(cached) = self.cached_token {
            if cached.before == self.cache_key() {
                return Ok(Some(cached));
            }
        }

        let Some((token, after)) = self.scan_token()? else {
            self.cached_token = None;
            return Ok(None);
        };
        let cached = CachedToken {
            before: self.cache_key(),
            token,
            after,
        };
        self.cached_token = Some(cached);
        Ok(Some(cached))
    }

    #[inline(never)]
    fn scan_token_direct(
        &self,
    ) -> Result<Option<(Token<'a>, StreamState)>, Error<Token<'a>, Token<'a>>> {
        self.scan_token()
    }
}

impl<'a> Positioned for TokenStream<'a> {
    fn position(&self) -> Self::Position {
        self.state.position
    }
}

impl<'a> ResetStream for TokenStream<'a> {
    type Checkpoint = Checkpoint;
    fn checkpoint(&self) -> Self::Checkpoint {
        Checkpoint { state: self.state }
    }
    fn reset(&mut self, checkpoint: Checkpoint) -> Result<(), Self::Error> {
        self.state = checkpoint.state;
        if self
            .cached_token
            .is_some_and(|cached| cached.before != self.cache_key())
        {
            self.cached_token = None;
        }
        Ok(())
    }
}

#[inline(always)]
fn is_name_byte(b: u8) -> bool {
    matches!(b, b'_' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9')
}

#[inline(always)]
fn is_name_start_byte(b: u8) -> bool {
    matches!(b, b'_' | b'a'..=b'z' | b'A'..=b'Z')
}

#[inline(always)]
fn digit_end(bytes: &[u8], mut index: usize) -> usize {
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    index
}

#[inline]
fn line_break_len(bytes: &[u8], index: usize) -> Option<usize> {
    match bytes.get(index) {
        Some(b'\r') => Some(if bytes.get(index + 1) == Some(&b'\n') {
            2
        } else {
            1
        }),
        Some(b'\n') => Some(1),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NumberToken {
    length: usize,
    kind: Kind,
}

fn scan_number(bytes: &[u8], start: usize) -> Option<NumberToken> {
    let mut index = start;
    if bytes.get(index) == Some(&b'-') {
        index += 1;
    }

    match *bytes.get(index)? {
        b'0' => {
            index += 1;
            if bytes.get(index).is_some_and(u8::is_ascii_digit) {
                return None;
            }
        }
        byte if (b'1'..=b'9').contains(&byte) => {
            index += 1;
            index = digit_end(bytes, index);
        }
        _ => return None,
    }

    let mut is_float = false;
    if bytes.get(index) == Some(&b'.') {
        if !bytes.get(index + 1).is_some_and(u8::is_ascii_digit) {
            return None;
        }
        is_float = true;
        index += 2;
        index = digit_end(bytes, index);
    }

    if bytes
        .get(index)
        .is_some_and(|byte| *byte == b'e' || *byte == b'E')
    {
        let mut exponent = index + 1;
        if matches!(bytes.get(exponent), Some(b'+' | b'-')) {
            exponent += 1;
        }
        if !bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
            return None;
        }
        is_float = true;
        index = exponent + 1;
        index = digit_end(bytes, index);
    }

    if bytes
        .get(index)
        .copied()
        .is_some_and(|byte| is_name_start_byte(byte) || byte == b'.')
    {
        return None;
    }

    Some(NumberToken {
        length: index - start,
        kind: if is_float {
            Kind::FloatValue
        } else {
            Kind::IntValue
        },
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EscapeError {
    Unknown(u8),
    IncompleteUnicode(usize),
    InvalidUnicode(u32),
}

/// Decode the escape beginning at `index`, returning the character and byte
/// width. The existing four-digit Unicode form is intentional; surrogate code
/// units remain rejected individually.
pub(crate) fn decode_escape(bytes: &[u8], index: usize) -> Result<(char, usize), EscapeError> {
    debug_assert_eq!(bytes.get(index), Some(&b'\\'));
    let Some(&escaped) = bytes.get(index + 1) else {
        return Err(EscapeError::Unknown(b'\\'));
    };
    let simple = match escaped {
        b'"' | b'\\' | b'/' => Some(escaped as char),
        b'b' => Some('\u{0008}'),
        b'f' => Some('\u{000c}'),
        b'n' => Some('\n'),
        b'r' => Some('\r'),
        b't' => Some('\t'),
        _ => None,
    };
    if let Some(value) = simple {
        return Ok((value, 2));
    }
    if escaped != b'u' {
        return Err(EscapeError::Unknown(escaped));
    }

    let available = bytes.len().saturating_sub(index + 2);
    if available < 4 {
        return Err(EscapeError::IncompleteUnicode(available));
    }
    let digits = &bytes[index + 2..index + 6];
    let mut code_point = 0u32;
    for &digit in digits {
        let value = match digit {
            b'0'..=b'9' => digit - b'0',
            b'a'..=b'f' => digit - b'a' + 10,
            b'A'..=b'F' => digit - b'A' + 10,
            _ => return Err(EscapeError::Unknown(digit)),
        };
        code_point = (code_point << 4) | u32::from(value);
    }
    char::from_u32(code_point)
        .map(|value| (value, 6))
        .ok_or(EscapeError::InvalidUnicode(code_point))
}

const DEFAULT_RECURSION_LIMIT: usize = 50;
const TAB_COLUMN_WIDTH: usize = 8;
const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";
const BLOCK_STRING_DELIMITER: &str = "\"\"\"";

impl<'a> TokenStream<'a> {
    pub fn new(s: &'a str) -> TokenStream<'a> {
        Self::with_recursion_limit(s, DEFAULT_RECURSION_LIMIT, None)
    }

    pub fn new_with_token_limit(s: &'a str, token_limit: usize) -> TokenStream<'a> {
        Self::with_recursion_limit(s, DEFAULT_RECURSION_LIMIT, Some(token_limit))
    }

    pub(crate) fn new_without_positions(s: &'a str) -> TokenStream<'a> {
        let mut me = TokenStream {
            buf: s,
            state: StreamState {
                offset: 0,
                position: Pos { line: 1, column: 1 },
                recursion_limit: DEFAULT_RECURSION_LIMIT,
                token_count: 0,
            },
            cached_token: None,
            token_limit: None,
        };
        skip_whitespace::<false>(me.buf.as_bytes(), &mut me.state);
        me
    }

    /// Increasing this limit may let malicious input exhaust the call stack.
    pub(crate) fn with_recursion_limit(
        s: &'a str,
        recursion_limit: usize,
        token_limit: Option<usize>,
    ) -> TokenStream<'a> {
        let mut me = TokenStream {
            buf: s,
            state: StreamState {
                offset: 0,
                position: Pos { line: 1, column: 1 },
                recursion_limit,
                token_count: 0,
            },
            cached_token: None,
            token_limit,
        };
        skip_whitespace::<true>(me.buf.as_bytes(), &mut me.state);
        me
    }

    #[inline(always)]
    fn scan_token(&self) -> Result<Option<(Token<'a>, StreamState)>, Error<Token<'a>, Token<'a>>> {
        self.scan_token_impl::<true>()
    }

    #[inline(always)]
    fn scan_token_impl<const TRACK_POSITION: bool>(
        &self,
    ) -> Result<Option<(Token<'a>, StreamState)>, Error<Token<'a>, Token<'a>>> {
        let before = self.state;
        let bytes = self.buf.as_bytes();
        let Some(&first) = bytes.get(before.offset) else {
            // EOF is checked before the token budget so an exact-limit input
            // can finish successfully.
            return Ok(None);
        };
        if self
            .token_limit
            .is_some_and(|limit| before.token_count >= limit)
        {
            return Err(Error::message_static_message("Token limit exceeded"));
        }

        use Kind::*;
        let (kind, length, mut after) = match first {
            b'(' | b'[' | b'{' => {
                let recursion_limit = before
                    .recursion_limit
                    .checked_sub(1)
                    .ok_or_else(|| Error::message_static_message("Recursion limit exceeded"))?;
                let mut after = before;
                after.recursion_limit = recursion_limit;
                (Punctuator, 1, after)
            }
            b')' | b']' | b'}' => {
                let mut after = before;
                after.recursion_limit = after.recursion_limit.saturating_add(1);
                (Punctuator, 1, after)
            }
            b'!' | b'$' | b':' | b'=' | b'@' | b'|' | b'&' => (Punctuator, 1, before),
            b'.' => {
                if self.buf[before.offset..].starts_with("...") {
                    (Punctuator, 3, before)
                } else {
                    let c = self.buf[before.offset..].chars().next().unwrap();
                    return Err(Error::Unexpected(Info::Owned(
                        format_args!("bare dot {:?} is not supported, only \"...\"", c).to_string(),
                    )));
                }
            }
            b'_' | b'a'..=b'z' | b'A'..=b'Z' => {
                let mut end = before.offset + 1;
                while end < bytes.len() && is_name_byte(bytes[end]) {
                    end += 1;
                }
                (Name, end - before.offset, before)
            }
            b'-' | b'0'..=b'9' => {
                let number = scan_number(bytes, before.offset).ok_or_else(|| {
                    Error::Unexpected(Info::Owned(
                        format_args!("unsupported number {:?}", &self.buf[before.offset..])
                            .to_string(),
                    ))
                })?;
                (number.kind, number.length, before)
            }
            b'"' => return self.scan_string_impl::<TRACK_POSITION>(before),
            _ => {
                let c = self.buf[before.offset..].chars().next().unwrap();
                return Err(Error::Unexpected(Info::Owned(
                    format_args!("unexpected character {:?}", c).to_string(),
                )));
            }
        };

        after.offset += length;
        if TRACK_POSITION {
            after.position.column += length;
        }
        if self.token_limit.is_some() {
            after.token_count += 1;
        }
        skip_whitespace::<TRACK_POSITION>(bytes, &mut after);
        let token = Token {
            kind,
            value: &self.buf[before.offset..before.offset + length],
        };
        Ok(Some((token, after)))
    }

    fn scan_string_impl<const TRACK_POSITION: bool>(
        &self,
        before: StreamState,
    ) -> Result<Option<(Token<'a>, StreamState)>, Error<Token<'a>, Token<'a>>> {
        let bytes = self.buf.as_bytes();
        let start = before.offset;
        if self.buf[start..].starts_with(BLOCK_STRING_DELIMITER) {
            let tail = &self.buf[start + BLOCK_STRING_DELIMITER.len()..];
            for (end, _) in tail.match_indices(BLOCK_STRING_DELIMITER) {
                let mut slashes = 0;
                for &byte in tail.as_bytes()[..end].iter().rev() {
                    if byte == b'\\' {
                        slashes += 1;
                    } else {
                        break;
                    }
                }
                if slashes % 2 == 0 {
                    let length = BLOCK_STRING_DELIMITER.len() + end + BLOCK_STRING_DELIMITER.len();
                    let mut after = before;
                    if TRACK_POSITION {
                        self.update_position(&mut after, length);
                    } else {
                        after.offset += length;
                    }
                    if self.token_limit.is_some() {
                        after.token_count += 1;
                    }
                    skip_whitespace::<TRACK_POSITION>(bytes, &mut after);
                    return Ok(Some((
                        Token {
                            kind: Kind::BlockString,
                            value: &self.buf[start..start + length],
                        },
                        after,
                    )));
                }
            }
            return Err(Error::Unexpected(Info::Owned(
                "unterminated block string value".to_string(),
            )));
        }

        let mut index = start + 1;
        let mut columns = 1;
        while index < bytes.len() {
            match bytes[index] {
                b'\\' => {
                    let (_, consumed) = decode_escape(bytes, index).map_err(string_escape_error)?;
                    index += consumed;
                    if TRACK_POSITION {
                        columns += consumed;
                    }
                }
                b'"' => {
                    let length = index + 1 - start;
                    let mut after = before;
                    after.offset += length;
                    if TRACK_POSITION {
                        after.position.column += columns + 1;
                    }
                    if self.token_limit.is_some() {
                        after.token_count += 1;
                    }
                    skip_whitespace::<TRACK_POSITION>(bytes, &mut after);
                    return Ok(Some((
                        Token {
                            kind: Kind::StringValue,
                            value: &self.buf[start..start + length],
                        },
                        after,
                    )));
                }
                b'\n' | b'\r' => {
                    return Err(Error::Unexpected(Info::Owned(
                        "unterminated string value".to_string(),
                    )));
                }
                byte if !byte.is_ascii() => {
                    index += utf8_char_len(byte);
                    if TRACK_POSITION {
                        columns += 1;
                    }
                }
                _ => {
                    index += 1;
                    if TRACK_POSITION {
                        columns += 1;
                    }
                }
            }
        }
        Err(Error::Unexpected(Info::Owned(
            "unterminated string value".to_string(),
        )))
    }

    fn update_position(&self, state: &mut StreamState, length: usize) {
        let value = &self.buf[state.offset..state.offset + length];
        state.offset += length;
        let bytes = value.as_bytes();
        let mut line_offset = 0;
        let mut index = 0;
        while index < bytes.len() {
            if let Some(line_length) = line_break_len(bytes, index) {
                state.position.line += 1;
                index += line_length;
                line_offset = index;
            } else {
                index += 1;
            }
        }
        if line_offset > 0 {
            state.position.column = value[line_offset..].chars().count() + 1;
        } else {
            state.position.column += value.chars().count();
        }
    }
}

fn string_escape_error(error: EscapeError) -> Error<Token<'static>, Token<'static>> {
    let message = match error {
        EscapeError::Unknown(byte) => {
            format_args!("bad escaped char {:?}", byte as char).to_string()
        }
        EscapeError::IncompleteUnicode(found) => {
            format_args!("\\u must have 4 characters after it, only found {found}").to_string()
        }
        EscapeError::InvalidUnicode(value) => {
            format_args!("{value:04X} is not a valid unicode code point").to_string()
        }
    };
    Error::Unexpected(Info::Owned(message))
}

#[inline(always)]
fn skip_whitespace<const TRACK_POSITION: bool>(bytes: &[u8], state: &mut StreamState) {
    let end = bytes.len();
    let mut index = state.offset;

    if !TRACK_POSITION {
        loop {
            if index >= end {
                state.offset = index;
                return;
            }
            match bytes[index] {
                b' ' | b',' | b'\t' => index += 1,
                0xef if bytes[index..].starts_with(UTF8_BOM) => index += UTF8_BOM.len(),
                b'#' => {
                    index += 1;
                    while index < end && !matches!(bytes[index], b'\r' | b'\n') {
                        index += 1;
                    }
                }
                b'\r' | b'\n' => {
                    index += line_break_len(bytes, index).unwrap();
                }
                _ => {
                    state.offset = index;
                    return;
                }
            }
        }
    }

    loop {
        if index >= end {
            state.offset = index;
            return;
        }
        if let Some(length) = line_break_len(bytes, index) {
            state.position.column = 1;
            state.position.line += 1;
            index += length;
            continue;
        }
        match bytes[index] {
            b' ' | b',' => {
                state.position.column += 1;
                index += 1;
            }
            b'\t' => {
                state.position.column += TAB_COLUMN_WIDTH;
                index += 1;
            }
            0xef if bytes[index..].starts_with(UTF8_BOM) => {
                index += UTF8_BOM.len();
            }
            b'#' => {
                index += 1;
                while index < end {
                    if let Some(length) = line_break_len(bytes, index) {
                        state.position.column = 1;
                        state.position.line += 1;
                        index += length;
                        break;
                    }
                    index += 1;
                }
            }
            _ => break,
        }
    }
    state.offset = index;
}

fn utf8_char_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first < 0xe0 {
        2
    } else if first < 0xf0 {
        3
    } else {
        4
    }
}

impl<'a> fmt::Display for Token<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}[{:?}]", self.value, self.kind)
    }
}

#[cfg(test)]
mod test {
    use super::Kind::*;
    use super::{Kind, TokenStream};
    use crate::parser::Pos;
    use combine::easy::Error;

    use combine::stream::ResetStream;
    use combine::{Positioned, StreamOnce};

    fn tok_str(s: &str) -> Vec<&str> {
        let mut r = Vec::new();
        let mut s = TokenStream::new(s);
        loop {
            match s.uncons() {
                Ok(x) => r.push(x.value),
                Err(ref e) if e == &Error::end_of_input() => break,
                Err(e) => panic!("Parse error at {}: {}", s.position(), e),
            }
        }
        r
    }
    fn tok_typ(s: &str) -> Vec<Kind> {
        let mut r = Vec::new();
        let mut s = TokenStream::new(s);
        loop {
            match s.uncons() {
                Ok(x) => r.push(x.kind),
                Err(ref e) if e == &Error::end_of_input() => break,
                Err(e) => panic!("Parse error at {}: {}", s.position(), e),
            }
        }
        r
    }

    #[test]
    fn comments_and_commas() {
        assert_eq!(tok_str("# hello { world }"), &[] as &[&str]);
        assert_eq!(tok_str("# x\n,,,"), &[] as &[&str]);
        assert_eq!(tok_str(", ,,  ,,,  # x"), &[] as &[&str]);
    }

    #[test]
    fn simple() {
        assert_eq!(tok_str("a { b }"), ["a", "{", "b", "}"]);
        assert_eq!(tok_typ("a { b }"), [Name, Punctuator, Name, Punctuator]);
    }

    #[test]
    fn query() {
        assert_eq!(
            tok_str(
                "query Query {
            object { field }
        }"
            ),
            ["query", "Query", "{", "object", "{", "field", "}", "}"]
        );
    }

    #[test]
    fn fragment() {
        assert_eq!(tok_str("a { ...b }"), ["a", "{", "...", "b", "}"]);
    }

    #[test]
    fn int() {
        assert_eq!(tok_str("0"), ["0"]);
        assert_eq!(tok_str("0,"), ["0"]);
        assert_eq!(tok_str("0# x"), ["0"]);
        assert_eq!(tok_typ("0"), [IntValue]);
        assert_eq!(tok_str("-0"), ["-0"]);
        assert_eq!(tok_typ("-0"), [IntValue]);
        assert_eq!(tok_str("-1"), ["-1"]);
        assert_eq!(tok_typ("-1"), [IntValue]);
        assert_eq!(tok_str("-132"), ["-132"]);
        assert_eq!(tok_typ("-132"), [IntValue]);
        assert_eq!(tok_str("132"), ["132"]);
        assert_eq!(tok_typ("132"), [IntValue]);
        assert_eq!(
            tok_str("a(x: 10) { b }"),
            ["a", "(", "x", ":", "10", ")", "{", "b", "}"]
        );
        assert_eq!(
            tok_typ("a(x: 10) { b }"),
            [
                Name, Punctuator, Name, Punctuator, IntValue, Punctuator, Punctuator, Name,
                Punctuator
            ]
        );
    }

    // TODO(tailhook) fix errors in parser and check error message
    #[test]
    #[should_panic]
    fn zero_int() {
        tok_str("01");
    }
    #[test]
    #[should_panic]
    fn zero_int4() {
        tok_str("00001");
    }
    #[test]
    #[should_panic]
    fn minus_int() {
        tok_str("-");
    }
    #[test]
    #[should_panic]
    fn minus_zero_int() {
        tok_str("-01");
    }
    #[test]
    #[should_panic]
    fn minus_zero_int4() {
        tok_str("-00001");
    }
    #[test]
    #[should_panic]
    fn letters_int() {
        tok_str("0bbc");
    }

    #[test]
    fn float() {
        assert_eq!(tok_str("0.0"), ["0.0"]);
        assert_eq!(tok_typ("0.0"), [FloatValue]);
        assert_eq!(tok_str("-0.0"), ["-0.0"]);
        assert_eq!(tok_typ("-0.0"), [FloatValue]);
        assert_eq!(tok_str("-1.0"), ["-1.0"]);
        assert_eq!(tok_typ("-1.0"), [FloatValue]);
        assert_eq!(tok_str("-1.023"), ["-1.023"]);
        assert_eq!(tok_typ("-1.023"), [FloatValue]);
        assert_eq!(tok_str("-132.0"), ["-132.0"]);
        assert_eq!(tok_typ("-132.0"), [FloatValue]);
        assert_eq!(tok_str("132.0"), ["132.0"]);
        assert_eq!(tok_typ("132.0"), [FloatValue]);
        assert_eq!(tok_str("0e+0"), ["0e+0"]);
        assert_eq!(tok_typ("0e+0"), [FloatValue]);
        assert_eq!(tok_str("0.0e+0"), ["0.0e+0"]);
        assert_eq!(tok_typ("0.0e+0"), [FloatValue]);
        assert_eq!(tok_str("-0e+0"), ["-0e+0"]);
        assert_eq!(tok_typ("-0e+0"), [FloatValue]);
        assert_eq!(tok_str("-1e+0"), ["-1e+0"]);
        assert_eq!(tok_typ("-1e+0"), [FloatValue]);
        assert_eq!(tok_str("-132e+0"), ["-132e+0"]);
        assert_eq!(tok_typ("-132e+0"), [FloatValue]);
        assert_eq!(tok_str("132e+0"), ["132e+0"]);
        assert_eq!(tok_typ("132e+0"), [FloatValue]);
        assert_eq!(
            tok_str("a(x: 10.0) { b }"),
            ["a", "(", "x", ":", "10.0", ")", "{", "b", "}"]
        );
        assert_eq!(
            tok_typ("a(x: 10.0) { b }"),
            [
                Name, Punctuator, Name, Punctuator, FloatValue, Punctuator, Punctuator, Name,
                Punctuator
            ]
        );
        assert_eq!(tok_str("1.23e4"), ["1.23e4"]);
        assert_eq!(tok_typ("1.23e4"), [FloatValue]);
        assert_eq!(tok_str("1e9"), ["1e9"]);
        assert_eq!(tok_typ("1e9"), [FloatValue]);
    }

    // TODO(tailhook) fix errors in parser and check error message
    #[test]
    #[should_panic]
    fn no_int_float() {
        tok_str(".0");
    }
    #[test]
    #[should_panic]
    fn no_int_float1() {
        tok_str(".1");
    }
    #[test]
    #[should_panic]
    fn zero_float() {
        tok_str("01.0");
    }
    #[test]
    #[should_panic]
    fn zero_float4() {
        tok_str("00001.0");
    }
    #[test]
    #[should_panic]
    fn minus_float() {
        tok_str("-.0");
    }
    #[test]
    #[should_panic]
    fn minus_zero_float() {
        tok_str("-01.0");
    }
    #[test]
    #[should_panic]
    fn minus_zero_float4() {
        tok_str("-00001.0");
    }
    #[test]
    #[should_panic]
    fn letters_float() {
        tok_str("0bbc.0");
    }
    #[test]
    #[should_panic]
    fn letters_float2() {
        tok_str("0.bbc");
    }
    #[test]
    #[should_panic]
    fn letters_float3() {
        tok_str("0.bbce0");
    }
    #[test]
    fn exp_without_sign_float() {
        assert_eq!(tok_str("0e0"), ["0e0"]);
        assert_eq!(tok_typ("0e0"), [FloatValue]);
    }
    #[test]
    #[should_panic]
    fn unterminated_string() {
        tok_str(r#""hello\""#);
    }
    #[test]
    #[should_panic]
    fn extra_unterminated_string() {
        tok_str(r#""hello\\\""#);
    }

    #[test]
    fn string() {
        assert_eq!(tok_str(r#""""#), [r#""""#]);
        assert_eq!(tok_typ(r#""""#), [StringValue]);
        assert_eq!(tok_str(r#""hello""#), [r#""hello""#]);
        assert_eq!(tok_str(r#""hello\\""#), [r#""hello\\""#]);
        assert_eq!(tok_str(r#""hello\\\\""#), [r#""hello\\\\""#]);
        assert_eq!(tok_str(r#""he\\llo""#), [r#""he\\llo""#]);
        assert_eq!(tok_typ(r#""hello""#), [StringValue]);
        assert_eq!(tok_str(r#""my\"quote""#), [r#""my\"quote""#]);
        assert_eq!(tok_typ(r#""my\"quote""#), [StringValue]);
    }

    #[test]
    fn escaped_string_characters() {
        assert_eq!(tok_str(r#""line\n\b""#), [r#""line\n\b""#]);
        assert!(TokenStream::new(r#""bad\q""#).uncons().is_err());
        assert!(TokenStream::new(r#""bad\u12zz""#).uncons().is_err());

        let mut stream = TokenStream::new("\"line\r\"");
        assert!(stream.uncons().is_err());
    }

    #[test]
    fn line_terminators_preserve_positions() {
        let stream = TokenStream::new("# comment\r\nfield");
        assert_eq!(stream.position(), Pos { line: 2, column: 1 });

        let mut stream = TokenStream::new("\"\"\"one\r\n two\"\"\"");
        assert_eq!(stream.uncons().unwrap().kind, BlockString);
        assert_eq!(stream.position(), Pos { line: 2, column: 8 });
    }

    #[test]
    fn peek_and_checkpoint_replay_are_logically_transparent() {
        let mut stream = TokenStream::new("{ field } [value]");
        let checkpoint = stream.checkpoint();
        let peeked = stream.peek_token().unwrap().unwrap();
        assert_eq!(peeked.value, "{");
        assert_eq!(stream.offset(), 0);
        assert_eq!(stream.position(), Pos { line: 1, column: 1 });

        let first = stream.uncons().unwrap();
        let second = stream.uncons().unwrap();
        let after_two = (stream.offset(), stream.position());
        stream.reset(checkpoint).unwrap();
        assert_eq!(stream.uncons().unwrap(), first);
        assert_eq!(stream.uncons().unwrap(), second);
        assert_eq!((stream.offset(), stream.position()), after_two);
    }

    #[test]
    fn block_string() {
        assert_eq!(tok_str(r#""""""""#), [r#""""""""#]);
        assert_eq!(tok_typ(r#""""""""#), [BlockString]);
        assert_eq!(tok_str(r#""""hello""""#), [r#""""hello""""#]);
        assert_eq!(tok_typ(r#""""hello""""#), [BlockString]);
        assert_eq!(tok_str(r#""""my "quote" """"#), [r#""""my "quote" """"#]);
        assert_eq!(tok_typ(r#""""my "quote" """"#), [BlockString]);
        assert_eq!(tok_str(r#""""\"""quote" """"#), [r#""""\"""quote" """"#]);
        assert_eq!(tok_typ(r#""""\"""quote" """"#), [BlockString]);
    }
}
