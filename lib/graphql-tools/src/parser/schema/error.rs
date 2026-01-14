use combine::easy::Errors;
use thiserror::Error;

use crate::parser::position::Pos;
use crate::parser::tokenizer::Token;

pub type InternalError<'a> = Errors<Token<'a>, Token<'a>, Pos>;

/// Error parsing schema
///
/// This structure is opaque for forward compatibility. We are exploring a
/// way to improve both error message and API.
#[derive(Error, Debug)]
#[error("schema parse error: {}", _0)]
pub struct ParseError(String);

impl<'a> From<InternalError<'a>> for ParseError {
    fn from(e: InternalError<'a>) -> ParseError {
        ParseError(format!("{}", e))
    }
}
