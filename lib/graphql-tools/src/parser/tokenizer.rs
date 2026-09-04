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
    position: Pos,
    off: usize,
    next_state: Option<(usize, Token<'a>, usize, Pos)>,
    recursion_limit: usize,
    token_limit: Option<usize>,
    token_count: usize,
}

impl TokenStream<'_> {
    pub(crate) fn offset(&self) -> usize {
        self.off
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Checkpoint {
    position: Pos,
    off: usize,
}

impl<'a> StreamOnce for TokenStream<'a> {
    type Token = Token<'a>;
    type Range = Token<'a>;
    type Position = Pos;
    type Error = Errors<Token<'a>, Token<'a>, Pos>;

    fn uncons(&mut self) -> Result<Self::Token, Error<Token<'a>, Token<'a>>> {
        if let Some((at, tok, off, pos)) = self.next_state {
            if at == self.off {
                self.off = off;
                self.position = pos;
                return Ok(tok);
            }
        }
        let old_pos = self.off;
        let (kind, len) = self.take_token()?;
        let value = &self.buf[self.off - len..self.off];
        self.skip_whitespace();
        let token = Token { kind, value };
        self.next_state = Some((old_pos, token, self.off, self.position));
        Ok(token)
    }
}

impl<'a> Positioned for TokenStream<'a> {
    fn position(&self) -> Self::Position {
        self.position
    }
}

impl<'a> ResetStream for TokenStream<'a> {
    type Checkpoint = Checkpoint;
    fn checkpoint(&self) -> Self::Checkpoint {
        Checkpoint {
            position: self.position,
            off: self.off,
        }
    }
    fn reset(&mut self, checkpoint: Checkpoint) -> Result<(), Self::Error> {
        self.position = checkpoint.position;
        self.off = checkpoint.off;
        Ok(())
    }
}

// NOTE: we expect that first character is always digit or minus, as returned
// by tokenizer
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
    if value.is_empty() {
        return false;
    }
    let first = value.chars().next().unwrap();
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

#[inline(always)]
fn is_name_byte(b: u8) -> bool {
    matches!(b, b'_' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9')
}

#[inline(always)]
fn is_delimiter_byte(b: u8) -> bool {
    matches!(
        b,
        b' ' | b'\n'
            | b'\r'
            | b'\t'
            | b','
            | b'#'
            | b'!'
            | b'$'
            | b':'
            | b'='
            | b'@'
            | b'|'
            | b'&'
            | b'('
            | b')'
            | b'['
            | b']'
            | b'{'
            | b'}'
    )
}

impl<'a> TokenStream<'a> {
    pub fn new(s: &'a str) -> TokenStream<'a> {
        Self::with_recursion_limit(s, 50, None)
    }

    pub fn new_with_token_limit(s: &'a str, token_limit: usize) -> TokenStream<'a> {
        Self::with_recursion_limit(s, 50, Some(token_limit))
    }

    /// Specify a limit to recursive parsing. Note that increasing the limit
    /// from the default may represent a security issue since a maliciously
    /// crafted input may cause a stack overflow, crashing the process.
    pub(crate) fn with_recursion_limit(
        s: &'a str,
        recursion_limit: usize,
        token_limit: Option<usize>,
    ) -> TokenStream<'a> {
        let mut me = TokenStream {
            buf: s,
            position: Pos { line: 1, column: 1 },
            off: 0,
            next_state: None,
            recursion_limit,
            token_limit,
            token_count: 0,
        };
        me.skip_whitespace();
        me
    }

    /// Convenience for the common case where a token does
    /// not span multiple lines. Infallible.
    #[inline]
    fn advance_token<T>(&mut self, kind: Kind, size: usize) -> Result<(Kind, usize), T> {
        self.position.column += size;
        self.off += size;
        // Should be counted?
        if self.token_limit.is_some() {
            self.token_count += 1;
        }
        Ok((kind, size))
    }

    #[inline]
    fn bytes(&self) -> &'a [u8] {
        self.buf.as_bytes()
    }

    fn take_token(&mut self) -> Result<(Kind, usize), Error<Token<'a>, Token<'a>>> {
        if let Some(limit) = self.token_limit {
            if self.token_count >= limit {
                return Err(Error::message_static_message("Token limit exceeded"));
            }
        }
        use self::Kind::*;
        let bytes = self.bytes();
        let end = bytes.len();
        let first = match bytes.get(self.off).copied() {
            Some(b) => b,
            None => return Err(Error::end_of_input()),
        };

        match first {
            b'(' | b'[' | b'{' => {
                self.recursion_limit = self
                    .recursion_limit
                    .checked_sub(1)
                    .ok_or_else(|| Error::message_static_message("Recursion limit exceeded"))?;
                self.advance_token(Punctuator, 1)
            }
            b')' | b']' | b'}' => {
                self.recursion_limit = self.recursion_limit.saturating_add(1);
                self.advance_token(Punctuator, 1)
            }
            b'!' | b'$' | b':' | b'=' | b'@' | b'|' | b'&' => self.advance_token(Punctuator, 1),
            b'.' => {
                if self.buf[self.off..].starts_with("...") {
                    self.advance_token(Punctuator, 3)
                } else {
                    let c = self.buf[self.off..].chars().next().unwrap();
                    Err(Error::Unexpected(Info::Owned(
                        format_args!(
                            "bare dot {:?} is not supported, \
                            only \"...\"",
                            c
                        )
                        .to_string(),
                    )))
                }
            }
            b'_' | b'a'..=b'z' | b'A'..=b'Z' => {
                let mut i = self.off + 1;
                while i < end && is_name_byte(bytes[i]) {
                    i += 1;
                }
                let len = i - self.off;
                if i < end {
                    return self.advance_token(Name, len);
                }
                self.position.column += len;
                self.off += len;
                Ok((Name, len))
            }
            b'-' | b'0'..=b'9' => {
                let mut exponent: Option<usize> = None;
                let mut real: Option<usize> = None;
                let mut i = self.off + 1;
                let len = loop {
                    if i >= end {
                        break i - self.off;
                    }
                    let b = bytes[i];
                    if is_delimiter_byte(b) {
                        break i - self.off;
                    }
                    if b == b'.' {
                        real = Some(i - self.off);
                    } else if b == b'e' || b == b'E' {
                        exponent = Some(i - self.off);
                    }
                    i += 1;
                };
                if exponent.is_some() || real.is_some() {
                    let value = &self.buf[self.off..][..len];
                    if !check_float(value, exponent, real) {
                        return Err(Error::Unexpected(Info::Owned(
                            format_args!("unsupported float {:?}", value).to_string(),
                        )));
                    }
                    self.position.column += len;
                    self.off += len;
                    Ok((FloatValue, len))
                } else {
                    let value = &self.buf[self.off..][..len];
                    if !check_int(value) {
                        return Err(Error::Unexpected(Info::Owned(
                            format_args!("unsupported integer {:?}", value).to_string(),
                        )));
                    }
                    self.advance_token(IntValue, len)
                }
            }
            b'"' => {
                let remaining = &self.buf[self.off..];
                if let Some(tail) = remaining.strip_prefix("\"\"\"") {
                    for (end_idx, _) in tail.match_indices("\"\"\"") {
                        if !tail[..end_idx].ends_with('\\') {
                            self.update_position(end_idx + 6);
                            return Ok((BlockString, end_idx + 6));
                        }
                    }
                    Err(Error::Unexpected(Info::Owned(
                        "unterminated block string value".to_string(),
                    )))
                } else {
                    let mut nchars = 1;
                    let mut escaped = false;
                    let mut i = self.off + 1;
                    while i < end {
                        let b = bytes[i];
                        if b == b'\\' {
                            escaped = !escaped;
                            i += 1;
                            nchars += 1;
                            continue;
                        }
                        if b > 0x7f {
                            i += utf8_char_len(bytes[i]);
                            nchars += 1;
                            continue;
                        }
                        if b == b'"' {
                            nchars += 1;
                            if escaped {
                                escaped = false;
                            } else {
                                let len = i + 1 - self.off;
                                self.position.column += nchars;
                                self.off += len;
                                return Ok((StringValue, len));
                            }
                        } else if b == b'\n' {
                            return Err(Error::Unexpected(Info::Owned(
                                "unterminated string value".to_string(),
                            )));
                        } else {
                            nchars += 1;
                        }
                        i += 1;
                    }
                    Err(Error::Unexpected(Info::Owned(
                        "unterminated string value".to_string(),
                    )))
                }
            }
            _ => {
                let c = self.buf[self.off..].chars().next().unwrap();
                Err(Error::Unexpected(Info::Owned(
                    format_args!("unexpected character {:?}", c).to_string(),
                )))
            }
        }
    }

    fn skip_whitespace(&mut self) {
        let bytes = self.bytes();
        let end = bytes.len();
        let mut i = self.off;
        loop {
            if i >= end {
                self.off = i;
                return;
            }
            match bytes[i] {
                b' ' | b',' => {
                    self.position.column += 1;
                    i += 1;
                }
                b'\t' => {
                    self.position.column += 8;
                    i += 1;
                }
                b'\n' => {
                    self.position.column = 1;
                    self.position.line += 1;
                    i += 1;
                }
                b'\r' => {
                    i += 1;
                }
                0xef if i + 2 < end && bytes[i + 1] == 0xbb && bytes[i + 2] == 0xbf => {
                    i += 3;
                }
                b'#' => {
                    i += 1;
                    while i < end {
                        let b = bytes[i];
                        if b == b'\r' || b == b'\n' {
                            self.position.column = 1;
                            self.position.line += 1;
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                }
                _ => break,
            }
        }
        self.off = i;
    }

    fn update_position(&mut self, len: usize) {
        let val = &self.buf[self.off..][..len];
        self.off += len;
        let lines = val.as_bytes().iter().filter(|&&x| x == b'\n').count();
        self.position.line += lines;
        if lines > 0 {
            let line_offset = val.rfind('\n').unwrap() + 1;
            let num = val[line_offset..].chars().count();
            self.position.column = num + 1;
        } else {
            let num = val.chars().count();
            self.position.column += num;
        }
    }
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
    use combine::easy::Error;

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
    #[should_panic]
    fn no_exp_sign_float() {
        tok_str("0e0");
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
