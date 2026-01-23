use std::fmt::Display;

use combine::easy::Errors;
use thiserror::Error;

use crate::parser::position::Pos;
use crate::parser::tokenizer::Token;

pub type InternalError<'a> = Errors<Token<'a>, Token<'a>, Pos>;

/// Error parsing query
///
/// This structure is opaque for forward compatibility. We are exploring a
/// way to improve both error message and API.
#[derive(Error, Debug)]
pub struct ParseError(pub InternalError<'static>);

impl<'a> From<InternalError<'a>> for ParseError {
    fn from(e: InternalError<'a>) -> ParseError {
        let e = unsafe { std::mem::transmute::<InternalError<'a>, InternalError<'static>>(e) };
        ParseError(e)
    }
}

impl Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
