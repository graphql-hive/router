use http::header::{InvalidHeaderName, InvalidHeaderValue};
use regex_automata::meta::BuildError;

#[derive(thiserror::Error, Debug)]
pub enum HeaderRuleCompileError {
    #[error("invalid header name '{0}': {1}")]
    BadHeaderName(String, InvalidHeaderName),
    #[error("invalid header value '{0}': {1}")]
    BadHeaderValue(String, InvalidHeaderValue),
    #[error("rename is only allowed with a single 'named' header")]
    InvalidRename,
    #[error("default is only allowed with a single 'named' header")]
    InvalidDefault,
    #[error("regex build failed: {0}")]
    RegexBuild(#[from] Box<BuildError>),
}
